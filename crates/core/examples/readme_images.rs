//! The pictures in the README: the code-rendered still life and the drawing of it in each style the
//! README's picture table shows.
//!
//! Run it with `cargo run --release -p plotline-core --example readme_images` from the repository
//! root. It overwrites the pictures in `docs/images/`, so a change that moves a drawing can refresh
//! the README.

use std::fs;
use std::path::Path;

use plotline_core::synthetic::still_life;
use plotline_core::{Pen, Session, DEFAULT_CACHE_BYTES};

/// The preview width in pixels: twice the width of a README column, so the pictures stay sharp on
/// a dense screen.
const WIDTH_PX: u32 = 560;

/// The image size, the shape of the default A4 drawing area, so each drawing fills its page.
const W: u32 = 600;
const H: u32 = 875;

/// The styles in the README's picture table, in its order.
const STYLES: [&str; 6] = ["grid", "hatch", "waves", "spiral", "ink", "scribble"];

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
    session.set_image(photo);
    for name in STYLES {
        session.set_style(name)?;
        // At ink's default threshold of 0.5 only the deepest shadows become shapes; 0.3 keeps the cube
        // and the balls' shaded sides.
        if name == "ink" {
            session.update("ink", &serde_json::json!({"threshold": 0.3}))?;
        }
        let mut run = session.start()?;
        while run.next_frame().is_some() {}
        if let Some(error) = session.error() {
            return Err(format!("{name}: {error}").into());
        }
        let png = session
            .preview_png(WIDTH_PX, false, false, None)
            .ok_or("the run left no drawing")??;
        // The pen is black on white paper, so grey loses nothing and the file is far smaller.
        image::load_from_memory(&png)?
            .to_luma8()
            .save(out.join(format!("{name}.png")))?;
    }
    println!("{}", out.display());
    Ok(())
}
