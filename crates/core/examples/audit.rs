//! Audit harness: every listed style on a folder of images, in several pen setups, with the
//! trial (Try all styles), the colour suggestion, Optimize and the looks, measured the way a user
//! would judge them.
//!
//! `cargo run --release -p plotline-core --example audit -- <images-dir> <out-dir> [workers]`
//!
//! Writes `<out-dir>/<image>.json` per image and preview PNGs under `<out-dir>/png/<image>/`.
//! The images are read from the folder and never copied into the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use plotline_core::preview::render_png_shrunk;
use plotline_core::registry::list_styles;
use plotline_core::synthetic::{scene, still_life};
use plotline_core::types::Drawing;
use plotline_core::{Image, Job, Pen, RunState, Session};
use serde_json::{json, Value};

#[path = "common/colour.rs"]
mod colour;
use colour::colour_match;

const PREVIEW_PX: u32 = 1000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: audit <images-dir> <out-dir> [workers]")?,
    );
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: audit <images-dir> <out-dir> [workers]")?,
    );
    let workers: usize = args.next().map(|w| w.parse()).transpose()?.unwrap_or(6);
    let only: Option<String> = std::env::var("AUDIT_ONLY").ok();
    fs::create_dir_all(out.join("png"))?;
    let mut images: Vec<(String, Image)> = vec![
        ("synthetic-still-life".into(), still_life(600, 875)),
        ("synthetic-scene".into(), scene(600, 875)),
    ];
    let mut files: Vec<_> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    files.sort();
    for path in files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let image = plotline_core::image::decode(&fs::read(&path)?)?;
        images.push((name, image));
    }
    if let Some(only) = &only {
        images.retain(|(n, _)| only.split(',').any(|o| n.contains(o)));
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((name, image)) = images.get(i) else {
                    break;
                };
                if out.join(format!("{name}.json")).exists() {
                    continue;
                }
                let t = Instant::now();
                match audit_image(name, image, &out) {
                    Ok(v) => {
                        let file = out.join(format!("{name}.json"));
                        fs::write(file, serde_json::to_string_pretty(&v).unwrap()).unwrap();
                        eprintln!("{name}: {:.0} s", t.elapsed().as_secs_f64());
                    }
                    Err(e) => eprintln!("{name}: FAILED {e}"),
                }
            });
        }
    });
    Ok(())
}

fn pen(name: &str, color: &str) -> Pen {
    Pen::new(name, 0.3, color).unwrap()
}

fn drive(job: &mut dyn Job) -> Result<(), String> {
    while job.next_frame().is_some() {}
    match job.state() {
        RunState::Done => Ok(()),
        _ => Err(job.error().unwrap_or_else(|| "cancelled".into())),
    }
}

/// Apply a suggestion's patch as the front ends do: style, then pens, then settings.
fn apply(s: &mut Session, patch: &Value) -> Result<(), String> {
    if let Some(style) = patch["style"].as_str() {
        s.set_style(style).map_err(|e| e.to_string())?;
    }
    if let Some(pens) = patch["pens"].as_array() {
        let pens: Result<Vec<Pen>, _> = pens
            .iter()
            .map(|p| {
                Pen::new(
                    p["name"].as_str().unwrap_or(""),
                    p["width_mm"].as_f64().unwrap_or(0.3),
                    p["color"].as_str().unwrap_or(""),
                )
            })
            .collect();
        s.set_pens(pens.map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    for (stage, values) in patch["settings"].as_object().into_iter().flatten() {
        s.update(stage, values).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn audit_image(name: &str, image: &Image, out: &Path) -> Result<Value, String> {
    let pngs = out.join("png").join(name);
    fs::create_dir_all(&pngs).map_err(|e| e.to_string())?;
    let mut s = Session::new(1 << 30);
    s.set_image(image.clone());
    let analysis = s.analysis().unwrap();
    let stats = serde_json::to_value(&analysis.stats).unwrap();
    let suggestions = serde_json::to_value(&analysis.suggestions).unwrap();
    let colour_patch = analysis
        .suggestions
        .iter()
        .find(|g| g.kind == "colour")
        .and_then(|g| g.patch.clone());

    // The same palette fitted to a small library, as "palette from my pens" does.
    let library = vec![
        pen("Black", "#000000"),
        pen("Red", "#d0202e"),
        pen("Blue", "#1f4fb4"),
        pen("Yellow", "#f5d000"),
        pen("Green", "#1e8a3c"),
    ];
    let mut lib_session = Session::new(1 << 20);
    lib_session.set_pen_library(library);
    lib_session.set_image(image.clone());
    let lib_analysis = lib_session.analysis().unwrap();
    let lib_palette = serde_json::to_value(&lib_analysis.stats.palette).unwrap();
    let lib_suggestions = serde_json::to_value(&lib_analysis.suggestions).unwrap();

    let mut setups: Vec<(&str, Vec<Pen>, &str)> = vec![
        ("mono", vec![pen("Black 0.3", "#000000")], "one_pen"),
        (
            "ryb",
            vec![
                pen("Yellow", "#f5d000"),
                pen("Red", "#d0202e"),
                pen("Black", "#000000"),
            ],
            "by_colour",
        ),
        (
            "greys",
            vec![pen("Grey", "#9a9a9a"), pen("Black", "#000000")],
            "light_to_dark",
        ),
    ];
    let palette_pens: Option<Vec<Pen>> = colour_patch.as_ref().and_then(|p| {
        p["pens"].as_array().map(|ps| {
            ps.iter()
                .map(|p| {
                    Pen::new(
                        p["name"].as_str().unwrap(),
                        p["width_mm"].as_f64().unwrap(),
                        p["color"].as_str().unwrap(),
                    )
                    .unwrap()
                })
                .collect()
        })
    });
    if let Some(pp) = &palette_pens {
        setups.push(("palette", pp.clone(), "by_colour"));
    }

    let mut results = Vec::new();
    for (setup, pens, mode) in &setups {
        s.set_pens(pens.clone()).map_err(|e| e.to_string())?;
        for style in list_styles() {
            s.set_style(&style.name).map_err(|e| e.to_string())?;
            s.reset_settings();
            s.update("split", &json!({ "mode": mode }))
                .map_err(|e| e.to_string())?;
            let t = Instant::now();
            let mut run = s.start().map_err(|e| e.to_string())?;
            let ran = drive(&mut run);
            let secs = t.elapsed().as_secs_f64();
            let mut cell = json!({"setup": setup, "style": style.name, "secs": secs});
            match ran {
                Err(e) => cell["error"] = json!(e),
                Ok(()) => {
                    let d = s.drawing().unwrap();
                    cell["m"] = measure(&s, &d, image);
                    let png = s
                        .preview_png(PREVIEW_PX, false, false, None)
                        .unwrap()
                        .unwrap();
                    fs::write(pngs.join(format!("{setup}-{}.png", style.name)), png).ok();
                    let thumb = render_png_shrunk(&d, 600, s.paper(), 2.0).unwrap();
                    fs::write(
                        pngs.join(format!("{setup}-{}.thumb.png", style.name)),
                        thumb,
                    )
                    .ok();
                }
            }
            results.push(cell);
        }
    }

    // Try all styles: trials started from the GUI's first style, as the dialog does, with the
    // three-pen setup the owner reported; each must match a real switch's drawing.
    let mut trials = Vec::new();
    for (setup, pens, mode) in &setups {
        s.set_pens(pens.clone()).map_err(|e| e.to_string())?;
        s.set_style("waves").map_err(|e| e.to_string())?;
        s.reset_settings();
        s.update("split", &json!({ "mode": mode }))
            .map_err(|e| e.to_string())?;
        for style in list_styles() {
            let (mut run, slot) = s.trial(&style.name).map_err(|e| e.to_string())?;
            let ran = drive(&mut run);
            let slot = slot.lock().unwrap();
            let mut cell = json!({"setup": setup, "style": style.name});
            match (ran, &slot.drawing) {
                (Ok(()), Some(d)) => {
                    let paths: Vec<usize> = d.layers.iter().map(|l| l.paths.len()).collect();
                    let thumb = render_png_shrunk(d, 600, s.paper(), 2.0).unwrap();
                    cell["paths"] = json!(paths);
                    cell["thumb_marked"] = json!(marked(&thumb));
                    cell["settings"] = d
                        .preset
                        .as_ref()
                        .map(|p| p["settings"].clone())
                        .unwrap_or(Value::Null);
                }
                (Ok(()), None) => cell["error"] = json!("no drawing"),
                (Err(e), _) => cell["error"] = json!(e),
            }
            trials.push(cell);
        }
    }

    // Optimize, Fit and the looks, one pen, every style with a density setting.
    s.set_pens(vec![pen("Black 0.3", "#000000")]).unwrap();
    let mut optimize = Vec::new();
    let mut looks = Vec::new();
    for style in list_styles() {
        s.set_style(&style.name).unwrap();
        s.reset_settings();
        let t = Instant::now();
        let mut cell = json!({"style": style.name});
        match s.optimize(None) {
            Err(e) => cell["error"] = json!(e.to_string()),
            Ok(mut job) => match drive(&mut job) {
                Err(e) => cell["error"] = json!(e),
                // A finished search with no trial scored within its limits has no result.
                Ok(()) if job.result().is_none() => cell["no_result"] = json!(true),
                Ok(()) => {
                    let r = job.result().unwrap();
                    cell["result"] = serde_json::to_value(&r).unwrap();
                    cell["secs"] = json!(t.elapsed().as_secs_f64());
                    if let Some(patch) = &r.patch {
                        let before = s.settings().clone();
                        apply(&mut s, patch)?;
                        let mut run = s.start().unwrap();
                        let ran = drive(&mut run);
                        cell["applied"] = match ran {
                            Ok(()) => {
                                let d = s.drawing().unwrap();
                                let png = s
                                    .preview_png(PREVIEW_PX, false, false, None)
                                    .unwrap()
                                    .unwrap();
                                fs::write(pngs.join(format!("optimized-{}.png", style.name)), png)
                                    .ok();
                                measure(&s, &d, image)
                            }
                            Err(e) => json!({"error": e}),
                        };
                        for (stage, v) in before.as_object().unwrap() {
                            s.update(stage, v).unwrap();
                        }
                    }
                }
            },
        }
        // Optimize with the budget the GUI offers by default: a 60 minute plot.
        s.reset_settings();
        if let Ok(mut job) = s.optimize(Some(("minutes", 60.0))) {
            if let (Ok(()), Some(r)) = (drive(&mut job), job.result()) {
                cell["budget60"] = json!({"value": r.value, "error": r.error, "minutes": r.minutes,
                    "current_error": r.current_error, "reason": r.reason});
            }
        }
        optimize.push(cell);

        for look in style.looks() {
            s.reset_settings();
            s.apply_look(&look.name).unwrap();
            let mut run = s.start().unwrap();
            let mut cell =
                json!({"style": style.name, "look": look.name, "settings": look.settings});
            match drive(&mut run) {
                Ok(()) => {
                    let d = s.drawing().unwrap();
                    cell["m"] = measure(&s, &d, image);
                }
                Err(e) => cell["error"] = json!(e),
            }
            looks.push(cell);
        }
    }

    Ok(json!({
        "image": name,
        "size": [image.width, image.height],
        "stats": stats,
        "suggestions": suggestions,
        "library_palette": lib_palette,
        "library_suggestions": lib_suggestions,
        "runs": results,
        "trials": trials,
        "optimize": optimize,
        "looks": looks,
    }))
}

/// Share of a 150 px cell's pixels at least 10 % darker than white, as the style-grid law counts.
fn marked(png: &[u8]) -> f64 {
    let picture = image::load_from_memory(png).unwrap().to_luma8();
    let height = 150 * picture.height() / picture.width();
    let cell =
        image::imageops::resize(&picture, 150, height, image::imageops::FilterType::Triangle);
    cell.pixels().filter(|p| p.0[0] <= 230).count() as f64 / f64::from(150 * height)
}

fn measure(s: &Session, d: &Drawing, image: &Image) -> Value {
    let stats = s.stats().unwrap();
    let lint: Vec<Value> = s
        .lint()
        .unwrap_or_default()
        .iter()
        .map(|f| json!({"code": f.code, "severity": format!("{:?}", f.severity), "message": f.message}))
        .collect();
    let tone = s
        .tone_match()
        .map(|t| json!({"error": t.error, "ssim": t.ssim, "floor": t.floor, "reach": t.reach}));
    let layers: Vec<Value> = d
        .layers
        .iter()
        .map(|l| json!({"pen": l.pen.name, "color": l.pen.color, "paths": l.paths.len(), "down_mm": l.paths.total_length()}))
        .collect();
    let thumb = render_png_shrunk(d, 600, s.paper(), 2.0).unwrap();
    json!({
        "paths": stats.path_count,
        "minutes": stats.estimated_seconds / 60.0,
        "down_m": stats.pen_down_mm / 1000.0,
        "layers": layers,
        "tone": tone,
        "lint": lint,
        "thumb_marked": marked(&thumb),
        "colour": colour_match(d, s.paper(), image),
    })
}
