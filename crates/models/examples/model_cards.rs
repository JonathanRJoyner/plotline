//! The pictures on the model cards: the code-rendered still life drawn with the `learned` style and
//! each model file given, one PNG per model.
//!
//! Run it by hand when a catalogue entry is added or its model changes, from the repository root:
//! `cargo run --release -p plotline-models --example model_cards -- web/public/models PATH...`
//! where each PATH is a model file named `<catalogue id>.onnx`, such as the one `plotline models
//! get teed-1` keeps in the cache folder (point `PLOTLINE_CACHE_DIR` at a scratch folder to fetch
//! it without touching your own). The file must match the catalogue's size and SHA-256. The
//! pictures, drawn on A4 with a 0.3 mm black pen at the style's defaults, are 300 px wide, and
//! overwrite `OUT_DIR/<id>.png`.

use std::path::Path;
use std::sync::Arc;

use plotline_core::synthetic::still_life;
use plotline_core::{Pen, Session, DEFAULT_CACHE_BYTES};
use plotline_models::TractRuntime;

/// The card picture's width in pixels.
const WIDTH_PX: u32 = 300;

/// The image size, the shape of the default A4 drawing area, so the drawing fills its page.
const W: u32 = 600;
const H: u32 = 875;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, files @ ..] = &args[..] else {
        return Err("usage: model_cards OUT_DIR MODEL_FILE...".into());
    };
    if files.is_empty() {
        return Err("usage: model_cards OUT_DIR MODEL_FILE...".into());
    }
    let out = Path::new(out);
    std::fs::create_dir_all(out)?;

    let mut session = Session::new(DEFAULT_CACHE_BYTES);
    session.set_model_runtime(Arc::new(TractRuntime::default()));
    session.set_pens(vec![Pen::new("Black 0.3", 0.3, "#000000")?])?;
    session.set_image(still_life(W, H));
    session.set_style("learned")?;

    for file in files {
        let path = Path::new(file);
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("the model file has no name")?;
        let info = (session.add_model(Some(id), &std::fs::read(path)?))
            .map_err(|e| format!("{id}: {e}"))?;
        session.update("model", &serde_json::json!({ "model": info.id }))?;
        let mut run = session.start()?;
        while run.next_frame().is_some() {}
        if let Some(error) = session.error() {
            return Err(format!("{id}: {error}").into());
        }
        let png = session
            .preview_png(WIDTH_PX, false, false, None)
            .ok_or("the run left no drawing")??;
        // The pen is black on white paper, so grey loses nothing and the file is far smaller.
        let card = out.join(format!("{id}.png"));
        image::load_from_memory(&png)?.to_luma8().save(&card)?;
        println!("{}", card.display());
    }
    Ok(())
}
