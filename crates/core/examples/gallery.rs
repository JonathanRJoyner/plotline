//! Regression gallery: every listed style drawn on a few test images with a few pens, as one page
//! of previews, so a change can be judged by eye before anything is plotted.
//!
//! Run it with
//! `cargo run --release -p plotline-core --example gallery -- <out-dir> [--baseline <dir>]`.
//! It writes one preview PNG per cell, `stats.json` and `index.html` into `<out-dir>`. With
//! `--baseline`, each PNG is compared byte for byte with the same cell of an earlier run in `<dir>`.
//! The engine is deterministic, so any difference is a real change. A changed cell shows how its
//! stats moved, and the page switches every changed cell between the new picture, the baseline's
//! and their difference. A usual round is a run on `main` into `target/gallery-main`, then a run on
//! the branch with `--baseline target/gallery-main`. `target/` is ignored by git, so no picture
//! lands in the repository.

use std::fs;
use std::path::PathBuf;

use plotline_core::registry::list_styles;
use plotline_core::synthetic::{gradient, scene, still_life};
use plotline_core::{Pen, Session, DEFAULT_CACHE_BYTES};
use serde_json::{json, Map, Value};

/// The preview width in pixels, about 5 px per mm on the default A4 page.
const WIDTH_PX: u32 = 1000;

/// How a cell compares with the baseline.
#[derive(PartialEq)]
enum Change {
    NoBaseline,
    New,
    Same,
    Changed,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, baseline) = match args.as_slice() {
        [out] => (PathBuf::from(out), None),
        [out, flag, base] if flag == "--baseline" => {
            (PathBuf::from(out), Some(PathBuf::from(base)))
        }
        _ => return Err("usage: gallery <out-dir> [--baseline <dir>]".into()),
    };
    let old_stats: Map<String, Value> = match &baseline {
        Some(base) => {
            let path = base.join("stats.json");
            let bytes =
                fs::read(&path).map_err(|e| format!("{} is not a gallery: {e}", path.display()))?;
            serde_json::from_slice(&bytes)?
        }
        None => Map::new(),
    };
    fs::create_dir_all(&out)?;
    if let Some(base) = &baseline {
        // Writing into the baseline would overwrite the pictures it is compared with.
        if fs::canonicalize(base)? == fs::canonicalize(&out)? {
            return Err("the baseline must be another directory than the output".into());
        }
    }

    // The images are the shape of the default A4 drawing area, so each fills its page.
    let images = [
        ("still-life", still_life(600, 875)),
        ("scene", scene(600, 875)),
        ("gradient", gradient(600, 875)),
    ];
    let pens = [
        Pen::new("Black 0.1", 0.1, "#000000")?,
        Pen::new("Black 0.5", 0.5, "#000000")?,
    ];
    let mut session = Session::new(DEFAULT_CACHE_BYTES);
    let (mut stats, mut cells) = (Map::new(), String::new());
    let (mut total, mut changed, mut new) = (0, 0, 0);
    for style in list_styles() {
        session.set_style(&style.name)?;
        for (image_name, image) in &images {
            session.set_image(image.clone());
            for pen in &pens {
                session.set_pens(vec![pen.clone()])?;
                let name = format!("{}-{image_name}-{}", style.name, pen.width_mm);
                let file = format!("{name}.png");
                let mut run = session.start()?;
                while run.next_frame().is_some() {}
                let (png, cell_stats) =
                    match (session.error(), session.preview_png(WIDTH_PX, false, false)) {
                        (None, Some(png)) => (png?, serde_json::to_value(session.stats())?),
                        (error, _) => {
                            let error = error.unwrap_or_else(|| "the run left no drawing".into());
                            (Vec::new(), json!({ "error": error }))
                        }
                    };
                fs::write(out.join(&file), &png)?;
                let change = match &baseline {
                    None => Change::NoBaseline,
                    Some(base) => match fs::read(base.join(&file)) {
                        Err(_) => Change::New,
                        Ok(old) if old == png => Change::Same,
                        Ok(old) => {
                            fs::write(out.join(format!("{name}.base.png")), old)?;
                            Change::Changed
                        }
                    },
                };
                total += 1;
                changed += usize::from(change == Change::Changed);
                new += usize::from(change == Change::New);
                let label = format!("{} · {image_name} · {} mm", style.name, pen.width_mm);
                cells += &cell(&name, &label, &cell_stats, old_stats.get(&name), &change);
                stats.insert(name, cell_stats);
            }
        }
    }
    fs::write(
        out.join("stats.json"),
        serde_json::to_string_pretty(&stats)?,
    )?;

    let removed: Vec<&str> = old_stats
        .keys()
        .filter(|k| !stats.contains_key(*k))
        .map(String::as_str)
        .collect();
    let summary = match baseline {
        None => format!("{total} cells, no baseline"),
        Some(_) => format!(
            "{changed} of {total} cells changed, {new} new, {} removed{}",
            removed.len(),
            if removed.is_empty() {
                String::new()
            } else {
                format!(": {}", removed.join(", "))
            }
        ),
    };
    let columns = images.len() * pens.len();
    fs::write(out.join("index.html"), page(&summary, columns, &cells))?;
    println!("{summary}\n{}", out.join("index.html").display());
    Ok(())
}

/// One cell of the grid: the preview, with the baseline's stacked on it when it changed, and a
/// caption with the stats and, against a baseline, how they moved.
fn cell(name: &str, label: &str, stats: &Value, old: Option<&Value>, change: &Change) -> String {
    let class = match change {
        Change::Changed => "changed",
        Change::New => "new",
        _ => "",
    };
    let base = if *change == Change::Changed {
        format!(r#"<img class="base" src="{name}.base.png" alt="">"#)
    } else {
        String::new()
    };
    let body = if let Some(error) = stats["error"].as_str() {
        format!(r#"<p class="error">{}</p>"#, escape(error))
    } else {
        let field = |key: &str, unit: &str| {
            let v = stats[key].as_f64().unwrap_or(0.0);
            let moved = old
                .and_then(|o| o[key].as_f64())
                .filter(|o| o.round() != v.round())
                .map(|o| format!(" ({:+.0})", v - o))
                .unwrap_or_default();
            format!("{v:.0}{unit}{moved}")
        };
        format!(
            r#"<a class="pic" href="{name}.png" target="_blank"><img src="{name}.png" alt="{label}" loading="lazy">{base}</a>
<figcaption>{label}<br>{} paths · {} down · {} up · {}</figcaption>"#,
            field("path_count", ""),
            field("pen_down_mm", " mm"),
            field("pen_up_mm", " mm"),
            field("estimated_seconds", " s"),
        )
    };
    format!("<figure class=\"{class}\">{body}</figure>\n")
}

/// `text` safe to place between HTML tags.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The whole page: the summary, the view switches and the grid, one style per row of `columns`.
fn page(summary: &str, columns: usize, cells: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>plotline gallery</title>
<style>
body {{ margin: 16px; font: 13px system-ui, sans-serif; background: #eee; color: #222; }}
header {{ position: sticky; top: 0; background: #eee; padding: 8px 0; display: flex; gap: 16px; flex-wrap: wrap; }}
main {{ display: grid; grid-template-columns: repeat({columns}, minmax(0, 1fr)); gap: 12px; }}
figure {{ margin: 0; padding: 6px; background: #fff; border: 3px solid transparent; }}
figure.changed {{ border-color: #d33; }}
figure.new {{ border-color: #36c; }}
.pic {{ display: block; position: relative; isolation: isolate; }}
.pic img {{ display: block; width: 100%; }}
.pic .base {{ position: absolute; inset: 0; display: none; }}
body:has(#view-base:checked) .base {{ display: block; }}
body:has(#view-diff:checked) .base {{ display: block; mix-blend-mode: difference; }}
body:has(#view-diff:checked) .changed .pic {{ filter: invert(1); }}
body:has(#only:checked) figure:not(.changed, .new) {{ display: none; }}
.error {{ color: #d33; }}
</style>
</head>
<body>
<header>
<strong>{summary}</strong>
<span>Changed cells show:
<label><input type="radio" name="view" checked> new</label>
<label><input type="radio" name="view" id="view-base"> baseline</label>
<label><input type="radio" name="view" id="view-diff"> difference</label></span>
<label><input type="checkbox" id="only"> only changed and new</label>
</header>
<main>
{cells}</main>
</body>
</html>
"#
    )
}
