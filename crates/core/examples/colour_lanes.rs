//! Pen use by colour, measured: a flat-colour poster drawn in code and `synthetic::scene`, each
//! style that shares its drawing between pens, and where each pen's line lands.
//!
//! `cargo run --release -p plotline-core --example colour_lanes -- <out-dir> [label]`
//!
//! Writes `<out-dir>/<label>.json` and a preview of every run under `<out-dir>/<label>/`. For each
//! run: every layer's pen-down length, and on the poster how much of it lies outside the areas of
//! its own colour (on white paper, or on another pen's colour), the points placed, the tone error
//! and the colour measure of the audit (`common/colour.rs`). Run it before and after a change to
//! the split or the lanes and compare the two files.

use std::fs;
use std::path::PathBuf;

use plotline_core::synthetic::scene;
use plotline_core::types::Drawing;
use plotline_core::{Image, Pen, RunState, Session};
use serde_json::{json, Value};

#[path = "common/colour.rs"]
mod colour;
use colour::colour_match;

const RED: [f32; 3] = [208.0 / 255.0, 32.0 / 255.0, 46.0 / 255.0];
const YELLOW: [f32; 3] = [245.0 / 255.0, 208.0 / 255.0, 0.0];

/// The poster's areas, by what should draw them.
#[derive(Clone, Copy, PartialEq)]
enum Area {
    White,
    Red,
    Yellow,
    Black,
}

/// The poster at a share `(fx, fy)` of its width and height: a red disc, a yellow square, a black
/// bar, and half tints of the red and the yellow, on white. Returns the area and its colour.
fn poster_at(fx: f64, fy: f64, aspect: f64) -> (Area, [f32; 3]) {
    let tint = |c: [f32; 3]| c.map(|v| 0.5 + 0.5 * v);
    let sq = |v: f64| v * v;
    if sq(fx - 0.3) + sq((fy - 0.22) * aspect) < sq(0.2) {
        (Area::Red, RED)
    } else if (0.55..0.9).contains(&fx) && (0.08..0.33).contains(&fy) {
        (Area::Yellow, YELLOW)
    } else if (0.08..0.92).contains(&fx) && (0.46..0.52).contains(&fy) {
        (Area::Black, [0.0; 3])
    } else if (0.08..0.42).contains(&fx) && (0.62..0.9).contains(&fy) {
        (Area::Red, tint(RED))
    } else if (0.58..0.92).contains(&fx) && (0.62..0.9).contains(&fy) {
        (Area::Yellow, tint(YELLOW))
    } else {
        (Area::White, [1.0; 3])
    }
}

fn poster(w: u32, h: u32) -> Image {
    let aspect = f64::from(h) / f64::from(w);
    let rgb = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let (fx, fy) = (
                (f64::from(x) + 0.5) / f64::from(w),
                (f64::from(y) + 0.5) / f64::from(h),
            );
            poster_at(fx, fy, aspect).1
        })
        .collect();
    Image {
        width: w,
        height: h,
        rgb,
    }
}

fn pen(name: &str, color: &str) -> Pen {
    Pen::new(name, 0.3, color).unwrap()
}

/// Per layer of a poster drawing: the pen-down metres, those off the pen's own colour on white
/// paper, and those on another pen's colour, judged at each segment's midpoint.
fn off_colour(d: &Drawing, image: &Image) -> Vec<Value> {
    let [ix, iy, iw, ih] = d.canvas.image_rect(image.width, image.height);
    let aspect = f64::from(image.height) / f64::from(image.width);
    let own = |name: &str| match name {
        "Red" => Area::Red,
        "Yellow" => Area::Yellow,
        _ => Area::Black,
    };
    d.layers
        .iter()
        .map(|l| {
            let mine = own(&l.pen.name);
            let (mut all, mut white, mut other) = (0.0, 0.0, 0.0);
            for p in l.paths.iter() {
                for s in p.windows(2) {
                    let len = (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]);
                    let mid = [(s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0];
                    let (fx, fy) = ((mid[0] - ix) / iw, (mid[1] - iy) / ih);
                    all += len;
                    match poster_at(fx, fy, aspect).0 {
                        a if a == mine => {}
                        Area::White => white += len,
                        _ => other += len,
                    }
                }
            }
            let m = |mm: f64| (mm / 100.0).round() / 10.0;
            json!({"pen": l.pen.name, "down_m": m(all), "on_white_m": m(white), "on_other_m": m(other)})
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().ok_or("usage: colour_lanes <out-dir> [label]")?);
    let label = args.next().unwrap_or_else(|| "run".into());
    let pngs = out.join(&label);
    fs::create_dir_all(&pngs)?;
    let ryb = vec![
        pen("Yellow", "#f5d000"),
        pen("Red", "#d0202e"),
        pen("Black", "#000000"),
    ];
    let greys = vec![pen("Grey", "#9a9a9a"), pen("Black", "#000000")];
    let setups: Vec<(&str, Image, Vec<Pen>, &str)> = vec![
        ("poster-ryb", poster(600, 875), ryb.clone(), "by_colour"),
        ("scene-ryb", scene(600, 875), ryb, "by_colour"),
        ("scene-greys", scene(600, 875), greys, "light_to_dark"),
    ];
    // Each run's name, style and settings over the style's defaults. The second stipple places as
    // many points in all as three pens did each before they shared `count`.
    let runs: [(&str, &str, Value); 11] = [
        ("waves", "waves", json!({})),
        ("waves-gamma-1", "waves", json!({"tone": {"gamma": 1.0}})),
        ("spiral", "spiral", json!({})),
        ("mosaic", "mosaic", json!({})),
        ("flow", "flow", json!({})),
        ("hilbert", "hilbert", json!({})),
        ("stipple", "stipple", json!({})),
        (
            "stipple-15000",
            "stipple",
            json!({"voronoi": {"count": 15000}}),
        ),
        ("line", "line", json!({})),
        ("scribble", "scribble", json!({})),
        ("hatch", "hatch", json!({})),
    ];
    let mut results = Vec::new();
    for (setup, image, pens, mode) in &setups {
        let mut s = Session::new(1 << 30);
        s.set_image(image.clone());
        s.set_pens(pens.clone())?;
        for (name, style, patch) in &runs {
            s.set_style(style)?;
            s.reset_settings();
            s.update("split", &json!({ "mode": mode }))?;
            for (stage, values) in patch.as_object().into_iter().flatten() {
                s.update(stage, values)?;
            }
            let mut run = s.start()?;
            while run.next_frame().is_some() {}
            if run.state() != RunState::Done {
                return Err(run.error().unwrap_or_default().into());
            }
            let d = s.drawing().unwrap();
            fs::write(
                pngs.join(format!("{setup}-{name}.png")),
                s.preview_png(1000, false, false, None).unwrap()?,
            )?;
            let layers = if setup.starts_with("poster") {
                off_colour(&d, image)
            } else {
                let m = |mm: f64| (mm / 100.0).round() / 10.0;
                (d.layers.iter())
                    .map(|l| json!({"pen": l.pen.name, "down_m": m(l.paths.total_length()), "paths": l.paths.len()}))
                    .collect()
            };
            let c = colour_match(&d, s.paper(), image);
            let gain = 1.0 - c["de"].as_f64().unwrap() / c["de_paper"].as_f64().unwrap();
            let stats = s.stats().unwrap();
            let r = |v: f64, k: f64| (v * k).round() / k;
            let result = json!({
                "setup": setup,
                "run": name,
                "layers": layers,
                "points": run.point_count(),
                "minutes": r(stats.estimated_seconds / 60.0, 10.0),
                "tone_error": s.tone_match().map(|t| r(t.error, 1000.0)),
                "de": r(c["de"].as_f64().unwrap(), 100.0),
                "gain": r(gain, 1000.0),
            });
            println!("{result}");
            results.push(result);
        }
    }
    fs::write(
        out.join(format!("{label}.json")),
        serde_json::to_string_pretty(&results)?,
    )?;
    Ok(())
}
