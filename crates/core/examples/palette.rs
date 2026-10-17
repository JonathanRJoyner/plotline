//! The palette the image analysis fits for an image, with a given pen library.
//!
//! `palette <image> [name:#rrggbb,...]` prints the palette, its mean ΔE*ab and the inks it
//! suggests adding, as JSON. With no library the palette is fitted to the built-in inks.

use plotline_core::analysis::measure_with;
use plotline_core::Pen;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let image = plotline_core::image::decode(&std::fs::read(&a[0])?)?;
    let library: Vec<Pen> = a
        .get(1)
        .map(|l| {
            l.split(',')
                .map(|p| {
                    let (name, color) = p.split_once(':').unwrap();
                    Pen::new(name, 0.3, color).unwrap()
                })
                .collect()
        })
        .unwrap_or_default();
    let s = measure_with(&image, &library);
    println!(
        "{}",
        json!({"palette": s.palette, "error": s.palette_error, "add": s.palette_additions})
    );
    Ok(())
}
