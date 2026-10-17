//! Measures the rippling styles and `mosaic` at several tone gammas, the evidence for their default
//! of 1.9 (see `docs/decisions/2026-10-17-ripple-gamma-kept.md`).
//!
//! Run it with `cargo run --release -p plotline-core --example ripple_gamma`. For `waves`, `spiral`
//! and `mosaic` on the regression gallery's three pictures at A4, with one black 0.3 mm pen and by
//! colour with yellow, red and black, it prints each gamma's tone error, contrast, colour gain
//! against the source image (the share of blank paper's CIE 1976 ΔE the drawing takes away, see
//! [`tone_match::colour`]), mean darkness of the page and plot time.

use plotline_core::preview::render_rgba;
use plotline_core::synthetic::{gradient, scene, still_life};
use plotline_core::{tone_match, Pen, Session};
use serde_json::json;

const GAMMAS: [f64; 4] = [1.0, 1.3, 1.6, 1.9];

fn main() {
    let images = [
        ("still life", still_life(600, 875)),
        ("scene", scene(600, 875)),
        ("gradient", gradient(600, 875)),
    ];
    let black = vec![Pen::new("Black", 0.3, "#000000").unwrap()];
    let colours = ["#f5d000", "#d0202e", "#000000"]
        .map(|c| Pen::new(c, 0.3, c).unwrap())
        .to_vec();
    for style in ["waves", "spiral", "mosaic"] {
        for (pens, mode) in [(&black, "one_pen"), (&colours, "by_colour")] {
            for (name, image) in &images {
                for gamma in GAMMAS {
                    let mut s = Session::new(1 << 30);
                    s.set_image(image.clone());
                    s.set_pens(pens.clone()).unwrap();
                    s.set_style(style).unwrap();
                    s.update("split", &json!({"mode": mode})).unwrap();
                    s.update("tone", &json!({"gamma": gamma})).unwrap();
                    let mut r = s.start().unwrap();
                    while r.next_frame().is_some() {}
                    let d = s.drawing().unwrap();
                    let t = s.tone_match().unwrap();
                    let c = tone_match::colour(&d, image, s.paper()).unwrap();
                    let (_, _, rgba) = render_rgba(&d, 400, s.paper(), false, false, None);
                    let dark = 1.0
                        - rgba
                            .chunks(4)
                            .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
                            .sum::<f64>()
                            / (765.0 * (rgba.len() / 4) as f64);
                    let minutes = s.stats().unwrap().estimated_seconds / 60.0;
                    println!(
                        "{style:6} {mode:9} {name:10} g{gamma}: error {:.3} contrast {:.2} \
                         colour gain {:.3} dark {:.4} {minutes:.0} min",
                        t.error, t.contrast, c.gain, dark
                    );
                }
            }
        }
    }
}
