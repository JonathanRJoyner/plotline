//! The engine's shared vocabulary: pen, canvas, image, tone map, paths, layer and drawing.
//!
//! Coordinates are page coordinates in millimetres: the origin is the top-left corner of the page,
//! x grows to the right and y grows down. The only pixel coordinates are the tone map's own.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A value that failed validation; the message is shown to users.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TypeError {
    /// The message says which value was wrong and what it was.
    #[error("{0}")]
    Invalid(String),
}

/// A physical pen: a name, a stroke width in mm and a colour written as `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pen {
    /// The pen's name.
    pub name: String,
    /// The stroke width in mm.
    pub width_mm: f64,
    /// The colour, `#` and six hex digits in either case.
    pub color: String,
}

impl Pen {
    /// Build a pen, rejecting a width that is not positive or a colour not written as `#rrggbb`.
    pub fn new(name: &str, width_mm: f64, color: &str) -> Result<Pen, TypeError> {
        // Written as a bool so that NaN fails the check.
        let width_ok = width_mm > 0.0;
        if !width_ok {
            return Err(TypeError::Invalid(format!(
                "width_mm must be positive, got {width_mm:?}"
            )));
        }
        let hex = color.strip_prefix('#').unwrap_or("");
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TypeError::Invalid(format!(
                "color must be written as #rrggbb, got {color:?}"
            )));
        }
        Ok(Pen {
            name: name.to_string(),
            width_mm,
            color: color.to_string(),
        })
    }
}

/// How an image fits the drawing area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Scale the whole image to sit inside the drawing area, centred.
    Fit,
    /// Make the image rectangle the drawing area itself; the caller crops the image to its aspect.
    Fill,
}

/// The page in mm, a margin around the drawing area, and how an image fits that area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    /// The page width in mm.
    pub width_mm: f64,
    /// The page height in mm.
    pub height_mm: f64,
    /// The margin on every side in mm.
    pub margin_mm: f64,
    /// How an image fits the drawing area.
    pub fit: Fit,
}

impl Canvas {
    /// Build a canvas, rejecting a page or margin that leaves no positive drawing area.
    pub fn new(
        width_mm: f64,
        height_mm: f64,
        margin_mm: f64,
        fit: Fit,
    ) -> Result<Canvas, TypeError> {
        // Each check is written as a bool so that NaN fails it.
        let width_ok = width_mm > 0.0;
        let height_ok = height_mm > 0.0;
        let margin_ok = margin_mm >= 0.0;
        let area_ok = 2.0 * margin_mm < width_mm.min(height_mm);
        if !width_ok {
            return Err(TypeError::Invalid(format!(
                "width_mm must be positive, got {width_mm:?}"
            )));
        }
        if !height_ok {
            return Err(TypeError::Invalid(format!(
                "height_mm must be positive, got {height_mm:?}"
            )));
        }
        if !margin_ok {
            return Err(TypeError::Invalid(format!(
                "margin_mm must not be negative, got {margin_mm:?}"
            )));
        }
        if !area_ok {
            return Err(TypeError::Invalid(format!(
                "margin_mm must leave a positive drawing area, got {margin_mm:?} \
                 for a {width_mm:?} x {height_mm:?} mm page"
            )));
        }
        Ok(Canvas {
            width_mm,
            height_mm,
            margin_mm,
            fit,
        })
    }

    /// The page minus the margin, as `[x, y, w, h]` in mm.
    pub fn drawing_rect(&self) -> [f64; 4] {
        let m = self.margin_mm;
        [m, m, self.width_mm - 2.0 * m, self.height_mm - 2.0 * m]
    }

    /// Where an image of this pixel size sits on the page, as `[x, y, w, h]` in mm.
    pub fn image_rect(&self, width_px: u32, height_px: u32) -> [f64; 4] {
        let [x, y, w, h] = self.drawing_rect();
        if self.fit == Fit::Fill {
            return [x, y, w, h];
        }
        let (wp, hp) = (f64::from(width_px), f64::from(height_px));
        let scale = (w / wp).min(h / hp);
        let (iw, ih) = (wp * scale, hp * scale);
        [x + (w - iw) / 2.0, y + (h - ih) / 2.0, iw, ih]
    }

    /// The tone map placement for an image of this pixel size: `(mm_per_px, origin_mm)`.
    pub fn image_transform(&self, width_px: u32, height_px: u32) -> (f64, [f64; 2]) {
        let [x, y, w, _] = self.image_rect(width_px, height_px);
        (w / f64::from(width_px), [x, y])
    }
}

/// An RGB image, row-major with 3 floats per pixel, each in 0..1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The pixels, `width * height * 3` floats.
    pub rgb: Vec<f32>,
}

impl Image {
    /// The luminance of each pixel, `0.299 r + 0.587 g + 0.114 b`, evaluated in f32 in that order.
    pub fn luminance(&self) -> Vec<f32> {
        self.rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[r, g, b]| 0.299 * r + 0.587 * g + 0.114 * b)
            .collect()
    }
}

/// Ink density per pixel (0 is white, 1 is full ink) and where the pixels sit on the page.
///
/// Pixel `(row, col)` covers `[col, col + 1) x [row, row + 1)` in pixel coordinates, so its centre
/// is at `(col + 0.5, row + 0.5)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToneMap {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The ink density, row-major, `width * height` floats.
    pub ink: Vec<f32>,
    /// The size of one pixel in mm.
    pub mm_per_px: f64,
    /// The page position of the map's top-left corner in mm.
    pub origin_mm: [f64; 2],
}

impl ToneMap {
    /// The page area the whole map covers, as `[x, y, w, h]` in mm.
    pub fn rect_mm(&self) -> [f64; 4] {
        let [x, y] = self.origin_mm;
        [
            x,
            y,
            f64::from(self.width) * self.mm_per_px,
            f64::from(self.height) * self.mm_per_px,
        ]
    }

    /// Convert pixel coordinates `[x, y]` to page mm.
    pub fn to_mm(&self, px: [f64; 2]) -> [f64; 2] {
        [
            px[0] * self.mm_per_px + self.origin_mm[0],
            px[1] * self.mm_per_px + self.origin_mm[1],
        ]
    }

    /// Convert page mm `[x, y]` to pixel coordinates.
    pub fn to_px(&self, mm: [f64; 2]) -> [f64; 2] {
        [
            (mm[0] - self.origin_mm[0]) / self.mm_per_px,
            (mm[1] - self.origin_mm[1]) / self.mm_per_px,
        ]
    }

    /// Bilinear ink at a page point; points off the map clamp to the edge pixels.
    pub fn sample(&self, mm: [f64; 2]) -> f32 {
        let [px, py] = self.to_px(mm);
        // Centre space has pixel centres at integers.
        let fx = (px - 0.5).clamp(0.0, f64::from(self.width - 1));
        let fy = (py - 0.5).clamp(0.0, f64::from(self.height - 1));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = (
            (x0 + 1).min(self.width as usize - 1),
            (y0 + 1).min(self.height as usize - 1),
        );
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let at = |x: usize, y: usize| f64::from(self.ink[y * self.width as usize + x]);
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        (top * (1.0 - ty) + bottom * ty) as f32
    }
}

/// A list of points in mm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Points(pub Vec<[f64; 2]>);

/// Many polylines in mm, stored flat: `offsets` has one entry more than there are paths, starts
/// at 0, and path `i` is `coords[offsets[i]..offsets[i + 1]]`. A path of one vertex is a dot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawPaths")]
pub struct Paths {
    coords: Vec<[f64; 2]>,
    offsets: Vec<u32>,
}

/// The unchecked shape of `Paths` on the wire; `Paths` is only built from it through `try_from`.
#[derive(Deserialize)]
struct RawPaths {
    coords: Vec<[f64; 2]>,
    offsets: Vec<u32>,
}

impl TryFrom<RawPaths> for Paths {
    type Error = TypeError;

    fn try_from(raw: RawPaths) -> Result<Paths, TypeError> {
        let starts_at_zero = raw.offsets.first() == Some(&0);
        let never_decreases = raw.offsets.windows(2).all(|w| w[0] <= w[1]);
        let covers_coords = raw.offsets.last().map(|&n| n as usize) == Some(raw.coords.len());
        if !(starts_at_zero && never_decreases && covers_coords) {
            return Err(TypeError::Invalid(
                "offsets must start at 0, never decrease and end at the number of coords"
                    .to_string(),
            ));
        }
        Ok(Paths {
            coords: raw.coords,
            offsets: raw.offsets,
        })
    }
}

impl Default for Paths {
    fn default() -> Self {
        Paths {
            coords: Vec::new(),
            offsets: vec![0],
        }
    }
}

impl Paths {
    /// No paths.
    pub fn new() -> Paths {
        Paths::default()
    }

    /// The number of paths.
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Whether there are no paths.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The vertices of path `i`.
    pub fn get(&self, i: usize) -> &[[f64; 2]] {
        &self.coords[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    /// Every vertex of every path, in order.
    pub fn coords(&self) -> &[[f64; 2]] {
        &self.coords
    }

    /// The start of each path in `coords`, then the total vertex count.
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    /// Append a path. Panics if `path` is empty, since a path has at least one vertex.
    pub fn push(&mut self, path: &[[f64; 2]]) {
        assert!(!path.is_empty(), "a path has at least one vertex");
        self.coords.extend_from_slice(path);
        self.offsets.push(self.coords.len() as u32);
    }

    /// Append a path with its vertices in reverse order. Panics if `path` is empty.
    pub fn push_reversed(&mut self, path: &[[f64; 2]]) {
        assert!(!path.is_empty(), "a path has at least one vertex");
        self.coords.extend(path.iter().rev());
        self.offsets.push(self.coords.len() as u32);
    }

    /// The paths in order.
    pub fn iter(&self) -> impl Iterator<Item = &[[f64; 2]]> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }

    /// The length of the polyline through the vertices of `path` in mm; 0 for one vertex.
    pub fn path_length(path: &[[f64; 2]]) -> f64 {
        path.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    /// The sum of `path_length` over all paths in mm, which is the pen-down distance.
    pub fn total_length(&self) -> f64 {
        self.iter().map(Paths::path_length).sum()
    }

    /// The pen-up distance in mm when the paths are drawn in order with the pen starting at
    /// `start`: the move from `start` to the first vertex of the first path, plus the gap from
    /// each path's last vertex to the next path's first vertex. 0 for no paths.
    pub fn travel_length(&self, start: [f64; 2]) -> f64 {
        let mut pen = start;
        let mut total = 0.0;
        for path in self.iter() {
            if let (Some(first), Some(last)) = (path.first(), path.last()) {
                let (dx, dy) = (first[0] - pen[0], first[1] - pen[1]);
                total += (dx * dx + dy * dy).sqrt();
                pen = *last;
            }
        }
        total
    }
}

/// The paths drawn with one pen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// The pen the paths are drawn with.
    pub pen: Pen,
    /// The paths, in drawing order.
    pub paths: Paths,
}

/// A finished drawing: the canvas, the layers in drawing order, and the tone map if any.
#[derive(Debug, Clone)]
pub struct Drawing {
    /// The page.
    pub canvas: Canvas,
    /// The layers, in drawing order.
    pub layers: Vec<Layer>,
    /// The tone map the drawing was made from, if any.
    pub tone: Option<Arc<ToneMap>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_rect_and_fit() {
        let c = Canvas::new(210.0, 297.0, 10.0, Fit::Fit).unwrap();
        assert_eq!(c.drawing_rect(), [10.0, 10.0, 190.0, 277.0]);
        let r = c.image_rect(1000, 500);
        assert!(
            (r[0] - 10.0).abs() < 1e-9
                && (r[1] - 101.0).abs() < 1e-9
                && (r[2] - 190.0).abs() < 1e-9
                && (r[3] - 95.0).abs() < 1e-9
        );
        let (s, o) = c.image_transform(1000, 500);
        assert!((s - 0.19).abs() < 1e-12 && (o[1] - 101.0).abs() < 1e-9);
    }
    #[test]
    fn fill_covers_drawing_rect() {
        let c = Canvas::new(100.0, 50.0, 0.0, Fit::Fill).unwrap();
        assert_eq!(c.image_rect(64, 64), [0.0, 0.0, 100.0, 50.0]);
        assert_eq!(c.image_transform(64, 32).0, 1.5625);
    }
    #[test]
    fn canvas_rejects_bad_dimensions() {
        for (w, h, m) in [
            (0.0, 10.0, 0.0),
            (100.0, -1.0, 0.0),
            (100.0, 100.0, 50.0),
            (100.0, 100.0, -1.0),
        ] {
            assert!(Canvas::new(w, h, m, Fit::Fit).is_err());
        }
    }
    #[test]
    fn pen_validation() {
        assert!(Pen::new("x", 0.3, "000000").is_err());
        assert!(Pen::new("x", 0.0, "#000000").is_err());
        assert!(Pen::new("x", 0.3, "#AbCdEf").is_ok());
    }
    #[test]
    fn tone_map_geometry_and_sampling() {
        let t = ToneMap {
            width: 2,
            height: 2,
            ink: vec![0.0, 1.0, 0.0, 1.0],
            mm_per_px: 10.0,
            origin_mm: [5.0, 5.0],
        };
        assert_eq!(t.rect_mm(), [5.0, 5.0, 20.0, 20.0]);
        assert_eq!(t.to_px([10.0, 10.0]), [0.5, 0.5]);
        let p = [12.0, 17.0];
        let back = t.to_mm(t.to_px(p));
        assert!((back[0] - p[0]).abs() < 1e-12 && (back[1] - p[1]).abs() < 1e-12);
        for (mm, want) in [
            ([10.0, 10.0], 0.0),
            ([20.0, 10.0], 1.0),
            ([15.0, 10.0], 0.5),
            ([100.0, 100.0], 1.0),
            ([-50.0, -50.0], 0.0),
        ] {
            assert!((t.sample(mm) - want).abs() < 1e-6, "{mm:?}");
        }
    }
    #[test]
    fn paths_layout_and_lengths() {
        let mut p = Paths::new();
        p.push(&[[0.0, 0.0], [3.0, 4.0]]);
        p.push(&[[7.0, 7.0]]);
        p.push_reversed(&[[1.0, 0.0], [2.0, 0.0]]);
        assert_eq!(p.len(), 3);
        assert_eq!(p.offsets(), &[0, 2, 3, 5]);
        assert_eq!(p.get(2), &[[2.0, 0.0], [1.0, 0.0]]);
        assert_eq!(Paths::path_length(p.get(0)), 5.0);
        assert_eq!(Paths::path_length(p.get(1)), 0.0);
        assert_eq!(p.total_length(), 6.0);
        // start -> (0,0): 0; (3,4) -> (7,7): 5; (7,7) -> (2,0): sqrt(74)
        assert!((p.travel_length([0.0, 0.0]) - (5.0 + 74f64.sqrt())).abs() < 1e-12);
        assert_eq!(Paths::new().travel_length([1.0, 1.0]), 0.0);
    }
    #[test]
    fn luminance_matches_numpy_float32_bits() {
        // Bit patterns generated with numpy float32: 0.299 * r + 0.587 * g + 0.114 * b.
        let img = Image {
            width: 4,
            height: 1,
            rgb: vec![
                0.5, 0.25, 0.125, //
                1.0, 1.0, 1.0, //
                0.1, 0.7, 0.3, //
                0.0, 0.0, 1.0,
            ],
        };
        let want = [0x3e9ef9db_u32, 0x3f800000, 0x3ef33334, 0x3de978d5];
        let got: Vec<u32> = img.luminance().iter().map(|v| v.to_bits()).collect();
        assert_eq!(got, want);
    }
    #[test]
    fn validation_messages() {
        let canvas = |w, h, m| Canvas::new(w, h, m, Fit::Fit).unwrap_err().to_string();
        assert_eq!(canvas(0.0, 10.0, 0.0), "width_mm must be positive, got 0.0");
        assert_eq!(
            canvas(100.0, -1.0, 0.0),
            "height_mm must be positive, got -1.0"
        );
        assert_eq!(
            canvas(100.0, 100.0, -1.0),
            "margin_mm must not be negative, got -1.0"
        );
        assert_eq!(
            canvas(100.0, 100.0, 50.0),
            "margin_mm must leave a positive drawing area, got 50.0 for a 100.0 x 100.0 mm page"
        );
        assert_eq!(
            Pen::new("x", 0.3, "000000").unwrap_err().to_string(),
            "color must be written as #rrggbb, got \"000000\""
        );
        assert_eq!(
            Pen::new("x", 0.0, "#000000").unwrap_err().to_string(),
            "width_mm must be positive, got 0.0"
        );
    }
    #[test]
    fn paths_deserialize_checks_the_offsets() {
        for bad in [
            r#"{"coords":[],"offsets":[]}"#,
            r#"{"coords":[[0,0]],"offsets":[0,5]}"#,
            r#"{"coords":[[0,0],[1,1]],"offsets":[1,2]}"#,
            r#"{"coords":[[0,0],[1,1]],"offsets":[0,2,1,2]}"#,
            r#"{"coords":[[0,0],[1,1]],"offsets":[0,1]}"#,
        ] {
            assert!(serde_json::from_str::<Paths>(bad).is_err(), "{bad}");
        }
        let mut p = Paths::new();
        p.push(&[[0.0, 0.0], [3.0, 4.0]]);
        p.push_reversed(&[[1.0, 0.0], [2.0, 0.0]]);
        let back: Paths = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
        let empty: Paths = serde_json::from_str(r#"{"coords":[],"offsets":[0]}"#).unwrap();
        assert!(empty.is_empty());
    }
    #[test]
    #[should_panic(expected = "a path has at least one vertex")]
    fn push_rejects_an_empty_path() {
        Paths::new().push(&[]);
    }
    #[test]
    #[should_panic(expected = "a path has at least one vertex")]
    fn push_reversed_rejects_an_empty_path() {
        Paths::new().push_reversed(&[]);
    }
}
