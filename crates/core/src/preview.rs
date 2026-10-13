//! Preview rasterizer: a drawing as a picture, for the front ends that show one.
//!
//! The page is white and the drawing rect is outlined in light grey, one pixel wide. Each layer is
//! drawn in its pen's colour at the pen's true width, at least one pixel, with round caps and
//! joins, and a path of one vertex is a dot as wide as the pen. Two overlays are optional:
//! `source` lays the tone map under the paths at 30 percent opacity, as the grey the pen would
//! ink, and `travel` adds the pen-up moves as thin light red lines. The picture shows the whole
//! page, or a `region` of it as `[x, y, width, height]` in mm. One mm is `width_px / width`
//! pixels in both directions, where `width` is the region's or the page's, and the region's
//! top-left corner is the picture's. A region may reach past the page, which is white. The page
//! origin is the top-left corner and y grows down, as it does in the engine, so nothing is
//! flipped.
//!
//! Rasterization and anti-aliasing are tiny-skia's job, so this module cites no algorithm.

use thiserror::Error;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap,
    PixmapPaint, Rect, Stroke, Transform,
};

use crate::types::{Drawing, ToneMap};

const OUTLINE: [u8; 3] = [0xdd, 0xdd, 0xdd];
const TRAVEL: [u8; 3] = [0xff, 0x80, 0x80];
const SOURCE_OPACITY: f32 = 0.3;

/// Why a preview could not be made.
#[derive(Debug, Error)]
pub enum PreviewError {
    /// The PNG encoder failed; the message is the encoder's.
    #[error("could not encode the preview: {0}")]
    Encode(String),
    /// The picture has no pixels, as when `width_px` is 0, or is too large for a pixmap.
    #[error("the preview has no pixels")]
    Empty,
    /// The region has a value that is not finite, or a width or height that is not positive.
    #[error(
        "the preview region must have finite values and a positive width and height, got {0:?}"
    )]
    Region([f64; 4]),
}

/// `d` as PNG bytes of the picture [`render_rgba`] gives.
pub fn render_png(
    d: &Drawing,
    width_px: u32,
    travel: bool,
    source: bool,
    region: Option<[f64; 4]>,
) -> Result<Vec<u8>, PreviewError> {
    let v = view(d, width_px, region)?;
    render(d, width_px, &v, travel, source)
        .ok_or(PreviewError::Empty)?
        .encode_png()
        .map_err(|e| PreviewError::Encode(e.to_string()))
}

/// `d` as `(width, height, pixels)`: `width_px` wide and `round(height * width_px / width)` high,
/// where `width` and `height` are the region's in mm, or the page's when there is none. `travel`
/// and `source` turn on the overlays the module doc describes.
///
/// The pixels are RGBA bytes, row by row from the top, and every one is opaque, so the colours are
/// plain rather than premultiplied. Pixel `(x, y)` starts at `4 * (y * width + x)`. The pixels are
/// empty when the picture has none, is too large for a pixmap, or the region is refused.
pub fn render_rgba(
    d: &Drawing,
    width_px: u32,
    travel: bool,
    source: bool,
    region: Option<[f64; 4]>,
) -> (u32, u32, Vec<u8>) {
    let Ok(v) = view(d, width_px, region) else {
        return (width_px, 0, Vec::new());
    };
    match render(d, width_px, &v, travel, source) {
        Some(pixmap) => (pixmap.width(), pixmap.height(), pixmap.take()),
        None => (width_px, v.height_px, Vec::new()),
    }
}

/// The part of the page a picture shows: the page point at its top-left corner, its scale and its
/// height in pixels.
struct View {
    x: f64,
    y: f64,
    px_per_mm: f64,
    height_px: u32,
}

/// The view of `region`, or of the whole page when there is none, for a picture `width_px` wide.
fn view(d: &Drawing, width_px: u32, region: Option<[f64; 4]>) -> Result<View, PreviewError> {
    let region = region.unwrap_or([0.0, 0.0, d.canvas.width_mm, d.canvas.height_mm]);
    let [x, y, w, h] = region;
    if !region.iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Err(PreviewError::Region(region));
    }
    let px_per_mm = f64::from(width_px) / w;
    Ok(View {
        x,
        y,
        px_per_mm,
        height_px: (h * px_per_mm).round() as u32,
    })
}

/// The picture, or `None` when it has no pixels or is too large for a pixmap.
fn render(d: &Drawing, width_px: u32, view: &View, travel: bool, source: bool) -> Option<Pixmap> {
    let View {
        x: view_x,
        y: view_y,
        px_per_mm,
        height_px,
    } = *view;
    let mut pixmap = Pixmap::new(width_px, height_px)?;
    pixmap.fill(Color::WHITE);
    let to_px = |[x, y]: [f64; 2]| {
        (
            ((x - view_x) * px_per_mm) as f32,
            ((y - view_y) * px_per_mm) as f32,
        )
    };

    let [x, y, w, h] = d.canvas.drawing_rect();
    let (left, top) = to_px([x, y]);
    let (right, bottom) = to_px([x + w, y + h]);
    // A one pixel stroke centred half a pixel in, so the outline lies just inside the rect.
    if let Some(rect) = Rect::from_ltrb(left + 0.5, top + 0.5, right - 0.5, bottom - 0.5) {
        let outline = PathBuilder::from_rect(rect);
        let stroke = Stroke::default();
        pixmap.stroke_path(
            &outline,
            &paint(OUTLINE),
            &stroke,
            Transform::identity(),
            None,
        );
    }

    if let Some(tone) = d.tone.as_deref().filter(|_| source) {
        draw_tone(&mut pixmap, tone, view);
    }

    for layer in &d.layers {
        let color = paint(rgb(&layer.pen.color));
        let width = (layer.pen.width_mm * px_per_mm).max(1.0) as f32;
        let (mut lines, mut dots) = (PathBuilder::new(), PathBuilder::new());
        for path in layer.paths.iter() {
            match path {
                [] => {}
                &[dot] => {
                    let (x, y) = to_px(dot);
                    dots.push_circle(x, y, width / 2.0);
                }
                [first, rest @ ..] => {
                    let (x, y) = to_px(*first);
                    lines.move_to(x, y);
                    for &p in rest {
                        let (x, y) = to_px(p);
                        lines.line_to(x, y);
                    }
                }
            }
        }
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        if let Some(lines) = lines.finish() {
            pixmap.stroke_path(&lines, &color, &stroke, Transform::identity(), None);
        }
        if let Some(dots) = dots.finish() {
            pixmap.fill_path(
                &dots,
                &color,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    if travel {
        // The pen moves from the last vertex of each path to the first of the next, across layers.
        let mut moves = PathBuilder::new();
        let mut pen = None;
        for path in d.layers.iter().flat_map(|layer| layer.paths.iter()) {
            if let (Some(&first), Some(&last)) = (path.first(), path.last()) {
                if let Some((x, y)) = pen {
                    moves.move_to(x, y);
                    let (x, y) = to_px(first);
                    moves.line_to(x, y);
                }
                pen = Some(to_px(last));
            }
        }
        if let Some(moves) = moves.finish() {
            // The default stroke is one pixel wide with butt caps.
            let stroke = Stroke::default();
            pixmap.stroke_path(&moves, &paint(TRAVEL), &stroke, Transform::identity(), None);
        }
    }
    Some(pixmap)
}

/// Blend `tone` over the page area it covers as the grey the pen would ink, white where the ink is
/// 0 and black where it is 1, scaled up with bilinear filtering.
fn draw_tone(pixmap: &mut Pixmap, tone: &ToneMap, view: &View) {
    let Some(mut grey) = Pixmap::new(tone.width, tone.height) else {
        return;
    };
    let (pixels, _) = grey.data_mut().as_chunks_mut::<4>();
    for (px, &ink) in pixels.iter_mut().zip(&tone.ink) {
        // `as` saturates, so the grey is clipped to 0..=255.
        let g = (255.0 * (1.0 - ink)) as u8;
        *px = [g, g, g, 255];
    }
    let [x, y, _, _] = tone.rect_mm();
    let scale = (tone.mm_per_px * view.px_per_mm) as f32;
    let (tx, ty) = (
        ((x - view.x) * view.px_per_mm) as f32,
        ((y - view.y) * view.px_per_mm) as f32,
    );
    let paint = PixmapPaint {
        opacity: SOURCE_OPACITY,
        quality: FilterQuality::Bilinear,
        blend_mode: BlendMode::SourceOver,
    };
    let transform = Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    pixmap.draw_pixmap(0, 0, grey.as_ref(), &paint, transform, None);
}

/// An opaque anti-aliased paint of colour `[r, g, b]`.
fn paint([r, g, b]: [u8; 3]) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    paint.set_color_rgba8(r, g, b, 255);
    paint
}

/// The bytes of a `#rrggbb` colour. Black when `color` is not written that way, which
/// [`crate::types::Pen::new`] rejects but a pen built field by field can still hold.
fn rgb(color: &str) -> [u8; 3] {
    let hex = color
        .strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .unwrap_or(0);
    let [_, r, g, b] = hex.to_be_bytes();
    [r, g, b]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{drawing_100, drawing_100_with_tone, drawing_with};

    /// A 100 mm page with a line and a dot, drawn with a 4 mm red pen.
    fn strokes() -> Drawing {
        drawing_100(
            vec![vec![[20.0, 50.0], [80.0, 50.0]], vec![[50.0, 20.0]]],
            4.0,
            "#ff0000",
        )
    }

    #[test]
    fn size_follows_aspect_and_page_is_white() {
        let (w, h, px) = render_rgba(&drawing_with(vec![]), 200, false, false, None); // 210 x 297 page
        assert_eq!((w, h), (200, 283));
        assert_eq!(&px[0..3], &[255, 255, 255]);
    }

    #[test]
    fn strokes_land_where_the_path_is_and_dots_are_pen_wide() {
        // 100 x 100 mm page, no margin, 100 px: 1 px per mm; pen 4 mm wide, colour #ff0000
        let d = strokes();
        let (_, _, px) = render_rgba(&d, 100, false, false, None);
        let at = |x: usize, y: usize| &px[4 * (y * 100 + x)..4 * (y * 100 + x) + 3];
        assert_eq!(at(50, 50), &[255, 0, 0]);
        assert_eq!(at(50, 40), &[255, 255, 255]);
        assert_eq!(at(50, 20), &[255, 0, 0]);
        assert_eq!(at(50, 24), &[255, 255, 255]); // dot radius 2
    }

    #[test]
    fn travel_and_source_overlays_appear_only_when_asked() {
        let d = drawing_100_with_tone(vec![vec![[10.0, 10.0]], vec![[90.0, 10.0]]]); // tone: full ink everywhere
        let (_, _, plain) = render_rgba(&d, 100, false, false, None);
        let (_, _, travel) = render_rgba(&d, 100, true, false, None);
        let (_, _, source) = render_rgba(&d, 100, false, true, None);
        let at =
            |px: &[u8], x: usize, y: usize| px[4 * (y * 100 + x)..4 * (y * 100 + x) + 3].to_vec();
        assert_eq!(at(&plain, 50, 10), vec![255, 255, 255]);
        assert!(at(&travel, 50, 10)[1] < 200); // a red line crosses the midpoint
        assert!(at(&source, 50, 50)[0] < 200 && at(&source, 50, 50)[0] > 150); // 30 percent black over white
        assert!(render_png(&d, 100, true, true, None)
            .unwrap()
            .starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn a_region_zooms_and_offsets() {
        // The dot at (50, 20) mm in a 20 mm square window at (40, 10), 100 px wide: 5 px per mm,
        // so the dot sits at pixel (50, 50) and the 4 mm pen has a 10 px radius.
        let d = drawing_100(vec![vec![[50.0, 20.0]]], 4.0, "#ff0000");
        let (w, h, px) = render_rgba(&d, 100, false, false, Some([40.0, 10.0, 20.0, 20.0]));
        assert_eq!((w, h), (100, 100));
        let at = |x: usize, y: usize| &px[4 * (y * 100 + x)..4 * (y * 100 + x) + 3];
        assert_eq!(at(50, 50), &[255, 0, 0]);
        assert_eq!(at(50, 58), &[255, 0, 0]);
        assert_eq!(at(50, 62), &[255, 255, 255]);
    }

    #[test]
    fn a_region_beside_the_paths_is_white() {
        let (_, _, px) = render_rgba(
            &strokes(),
            100,
            false,
            false,
            Some([60.0, 60.0, 10.0, 10.0]),
        );
        assert_eq!(px.len(), 4 * 100 * 100);
        assert!(px.chunks(4).all(|p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn the_whole_page_as_a_region_matches_none() {
        let d = strokes();
        assert_eq!(
            render_png(&d, 100, true, false, Some([0.0, 0.0, 100.0, 100.0])).unwrap(),
            render_png(&d, 100, true, false, None).unwrap()
        );
    }

    #[test]
    fn a_bad_region_is_refused() {
        let d = strokes();
        for region in [
            [0.0, 0.0, 0.0, 10.0],
            [0.0, 0.0, 10.0, -1.0],
            [f64::NAN, 0.0, 10.0, 10.0],
        ] {
            assert!(
                matches!(
                    render_png(&d, 100, false, false, Some(region)),
                    Err(PreviewError::Region(_))
                ),
                "{region:?}"
            );
            assert!(render_rgba(&d, 100, false, false, Some(region))
                .2
                .is_empty());
        }
    }
}
