//! One run with explicit pens and settings, for checking a hypothesis from the audit.
//!
//! `probe <image> <style> <pens> <settings-json> <out.png>` where `<pens>` is
//! `name:width:#rrggbb,...` and the settings are `{stage: {key: value}}`. Prints the stats, the
//! tone match, the lint codes and each layer's path count and pen-down length as JSON.

use plotline_core::{Pen, RunState, Session};
use serde_json::{json, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [image, style, pens, settings, out] = &a[..] else {
        return Err("usage: probe <image> <style> <pens> <settings-json> <out.png>".into());
    };
    let mut s = Session::new(1 << 30);
    s.set_image_bytes(&std::fs::read(image)?)?;
    let pens: Vec<Pen> = pens
        .split(',')
        .map(|p| {
            let f: Vec<&str> = p.split(':').collect();
            Pen::new(f[0], f[1].parse().unwrap(), f[2]).unwrap()
        })
        .collect();
    s.set_pens(pens)?;
    s.set_style(style)?;
    let settings: Value = serde_json::from_str(settings)?;
    for (stage, v) in settings.as_object().into_iter().flatten() {
        s.update(stage, v)?;
    }
    let t = std::time::Instant::now();
    let mut run = s.start()?;
    while run.next_frame().is_some() {}
    if run.state() != RunState::Done {
        return Err(run.error().unwrap_or_default().into());
    }
    let secs = t.elapsed().as_secs_f64();
    let d = s.drawing().unwrap();
    std::fs::write(out, s.preview_png(1000, false, false, None).unwrap()?)?;
    let st = s.stats().unwrap();
    let layers: Vec<Value> = d
        .layers
        .iter()
        .map(|l| {
            json!([
                l.pen.name,
                l.paths.len(),
                (l.paths.total_length() / 1000.0 * 10.0).round() / 10.0
            ])
        })
        .collect();
    let lint: Vec<String> = s
        .lint()
        .unwrap_or_default()
        .iter()
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    let tone = s
        .tone_match()
        .map(|t| json!({"error": t.error, "ssim": t.ssim, "floor": t.floor, "reach": t.reach}));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "secs": secs, "paths": st.path_count, "minutes": st.estimated_seconds / 60.0,
            "layers": layers, "tone": tone, "lint": lint,
        }))?
    );
    Ok(())
}
