//! The pictures in the README: the code-rendered still life, every listed style's drawing of it,
//! and the `ink` style's drawing of the logo, since `ink` is for line art rather than photographs.
//!
//! Run it with `cargo run --release -p plotline-core --example readme_images` from the repository
//! root. It overwrites the pictures in `docs/images/`, so a change that moves a drawing can refresh
//! the README.

use std::fs;
use std::path::Path;

use plotline_core::registry::list_styles;
use plotline_core::synthetic::still_life;
use plotline_core::{Image, Pen, Session, DEFAULT_CACHE_BYTES};
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

/// The preview width in pixels: twice the width of a README column, so the pictures stay sharp on
/// a dense screen.
const WIDTH_PX: u32 = 560;

/// The image size, the shape of the default A4 drawing area, so each drawing fills its page.
const W: u32 = 600;
const H: u32 = 875;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Path::new("docs/images");
    fs::create_dir_all(out)?;

    let photo = still_life(W, H);
    let rgb: Vec<u8> = photo
        .rgb
        .iter()
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    image::RgbImage::from_raw(W, H, rgb)
        .ok_or("the still life has the wrong number of pixels")?
        .save(out.join("still-life.jpg"))?;

    let mut session = Session::new(DEFAULT_CACHE_BYTES);
    session.set_pens(vec![Pen::new("Black 0.3", 0.3, "#000000")?])?;
    for style in list_styles() {
        let image = if style.name == "ink" {
            logo()?
        } else {
            photo.clone()
        };
        session.set_image(image);
        session.set_style(&style.name)?;
        let mut run = session.start()?;
        while run.next_frame().is_some() {}
        if let Some(error) = session.error() {
            return Err(format!("{}: {error}", style.name).into());
        }
        let png = session
            .preview_png(WIDTH_PX, false, false, None)
            .ok_or("the run left no drawing")??;
        // The pen is black on white paper, so grey loses nothing and the file is far smaller.
        image::load_from_memory(&png)?
            .to_luma8()
            .save(out.join(format!("{}.png", style.name)))?;
    }
    println!("{}", out.display());
    Ok(())
}

/// The logo's mark in black on white, as `docs/images/logo-light.svg` draws it on its 24-unit
/// grid: a curve between two ring handles.
fn logo() -> Result<Image, Box<dyn std::error::Error>> {
    let mut pixmap = Pixmap::new(W, H).ok_or("no pixmap")?;
    pixmap.fill(tiny_skia::Color::WHITE);
    let scale = W as f32 * 0.8 / 24.0;
    let at = Transform::from_scale(scale, scale)
        .post_translate(W as f32 * 0.1, (H as f32 - 24.0 * scale) / 2.0);
    let mut black = Paint::default();
    black.set_color_rgba8(0, 0, 0, 255);

    let mut curve = PathBuilder::new();
    curve.move_to(5.0, 17.0);
    curve.cubic_to(5.0, 10.0, 10.0, 5.0, 17.0, 5.0);
    let stroke = Stroke {
        width: 2.0,
        line_cap: tiny_skia::LineCap::Round,
        ..Stroke::default()
    };
    let rings = [(5.0, 19.0), (19.0, 5.0)].map(|(x, y)| PathBuilder::from_circle(x, y, 2.0));
    for path in [curve.finish()].into_iter().chain(rings) {
        pixmap.stroke_path(&path.ok_or("no path")?, &black, &stroke, at, None);
    }

    let rgb = pixmap
        .pixels()
        .iter()
        .flat_map(|p| [p.red(), p.green(), p.blue()]);
    Ok(plotline_core::image::from_rgb_f32(
        W,
        H,
        rgb.map(|v| f32::from(v) / 255.0).collect(),
    )?)
}
