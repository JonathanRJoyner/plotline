//! Golden files: the grid style's output on small synthetic images, compared to files in
//! `crates/core/golden`.
//!
//! Only the coordinates are compared with a tolerance (1e-3 mm). Everything else must match
//! exactly: every top-level key except `layers`, the number of layers, and the pen and the path
//! count of each layer. A rounding difference that flips one grid cell therefore shows up as a
//! count mismatch. If a platform ever does that, move `threshold` in `produce` farther from every
//! sampled ink value instead of loosening the comparison. After a deliberate change to the
//! geometry, rewrite the files with `PLOTLINE_UPDATE_GOLDEN=1 cargo test -p plotline-core --test
//! golden` and review the diff.

use std::sync::{Arc, Mutex};

use plotline_core::cache::StageCache;
use plotline_core::export::json::to_value;
use plotline_core::registry::get_style;
use plotline_core::runner::{Run, RunState, Shared};
use plotline_core::stage::Artifact;
use plotline_core::synthetic::{checkerboard, gradient};
use plotline_core::types::{Canvas, Fit, Image, Pen};
use serde_json::{json, Value};

/// The grid style's drawing of `image` as `plotline-paths` JSON.
fn produce(image: Image) -> Value {
    let style = get_style("grid").expect("the grid style is registered");
    let mut settings = style.defaults();
    settings["tone"]["resolution"] = json!(64);
    settings["points"]["threshold"] = json!(0.6);
    // The spacing the files were checked against the Python engine at, before its default fell to 1.
    settings["points"]["spacing_mm"] = json!(2.0);
    let canvas = Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap();
    let pen = Pen::new("Black 0.3", 0.3, "#000000").unwrap();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut run = Run::new(
        style,
        settings,
        [
            Artifact::Image(Arc::new(image)),
            Artifact::Canvas(Arc::new(canvas)),
            Artifact::Pens(Arc::new(vec![pen])),
        ],
        Arc::new(Mutex::new(StageCache::new(64 << 20))),
        shared.clone(),
        Arc::default(),
        0,
    );
    while run.next_frame().is_some() {}
    assert_eq!(run.state(), RunState::Done, "run failed: {:?}", run.error());
    let drawing = shared.lock().unwrap().drawing.clone();
    to_value(&drawing.expect("a finished run leaves a drawing"))
}

/// Every top-level entry of `v` except the layers.
fn header(v: &Value) -> Vec<(&String, &Value)> {
    v.as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| *k != "layers")
        .collect()
}

#[test]
fn grid_matches_golden_files() {
    let cases = [
        ("grid_gradient", gradient(64, 48)),
        ("grid_checkerboard", checkerboard(64, 48, 8)),
    ];
    for (name, image) in cases {
        let actual = produce(image);
        let path = format!("{}/golden/{name}.json", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("PLOTLINE_UPDATE_GOLDEN").is_some() {
            let text = serde_json::to_string_pretty(&actual).unwrap() + "\n";
            std::fs::write(&path, text).unwrap();
            continue;
        }
        let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(header(&actual), header(&expected), "{name}: header");
        let (a, e) = (
            actual["layers"].as_array().unwrap(),
            expected["layers"].as_array().unwrap(),
        );
        assert_eq!(a.len(), e.len(), "{name}: layer count");
        for (la, le) in a.iter().zip(e) {
            assert_eq!(la["pen"], le["pen"], "{name}: pen");
            let (pa, pe) = (
                la["paths"].as_array().unwrap(),
                le["paths"].as_array().unwrap(),
            );
            assert_eq!(pa.len(), pe.len(), "{name}: path count");
            assert!(!pa.is_empty(), "{name}: no paths");
            for (i, (p, q)) in pa.iter().zip(pe).enumerate() {
                let (p, q) = (p.as_array().unwrap(), q.as_array().unwrap());
                assert_eq!(p.len(), q.len(), "{name}: path {i} length");
                for (u, v) in p.iter().zip(q) {
                    for k in 0..2 {
                        let (x, y) = (u[k].as_f64().unwrap(), v[k].as_f64().unwrap());
                        assert!((x - y).abs() <= 1e-3, "{name}: path {i}: {u} vs {v}");
                    }
                }
            }
        }
    }
}
