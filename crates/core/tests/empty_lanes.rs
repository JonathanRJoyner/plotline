//! A pen with no ink draws nothing. When the pens share the drawing, a lane whose map is empty must
//! give a layer with no paths in every style, and a later pen in light to dark mode adds only to
//! tones darker than its band.

use std::sync::Arc;

use plotline_core::models::{file_id, StandIn, STANDIN_BYTES};
use plotline_core::registry::list_styles;
use plotline_core::synthetic::flat;
use plotline_core::{Canvas, Fit, Pen, Session};
use serde_json::json;

/// The number of paths in each layer of `style`'s drawing of a flat image of `value` with a black
/// and a red pen shared in `mode`.
fn layer_paths(style: &str, value: f32, mode: &str) -> Vec<usize> {
    let mut s = Session::new(64 << 20);
    s.set_image(flat(64, 48, value));
    s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
    s.set_pens(vec![
        Pen::new("Black", 0.3, "#000000").unwrap(),
        Pen::new("Red", 0.3, "#ff0000").unwrap(),
    ])
    .unwrap();
    s.set_style(style).unwrap();
    if style == "learned" {
        s.set_model_runtime(Arc::new(StandIn::default()));
        s.add_model(None, STANDIN_BYTES).unwrap();
        let model = json!({"model": file_id(STANDIN_BYTES), "resolution": 256});
        s.update("model", &model).unwrap();
    } else {
        // Gamma 1 for every style, so the ink is 1 - value even where a style starts darker.
        s.update("tone", &json!({"resolution": 64, "gamma": 1.0}))
            .unwrap();
    }
    s.update("split", &json!({"mode": mode})).unwrap();
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    assert!(run.error().is_none(), "{style}: {:?}", run.error());
    let drawing = s.drawing().expect("the run leaves a drawing");
    drawing.layers.iter().map(|l| l.paths.len()).collect()
}

#[test]
fn a_white_image_by_colour_draws_nothing() {
    for style in list_styles() {
        let counts = layer_paths(&style.name, 1.0, "by_colour");
        assert!(counts.iter().all(|&n| n == 0), "{}: {counts:?}", style.name);
    }
}

#[test]
fn a_light_grey_leaves_the_second_pen_light_to_dark_empty() {
    // Ink 0.4 fills the first band to 0.8, past the mid-grey thresholds of grid and ink, and leaves
    // the second band empty.
    // A model finds no lines in a flat grey, so `learned` has no first band to fill.
    for style in list_styles().iter().filter(|s| s.name != "learned") {
        let counts = layer_paths(&style.name, 0.6, "light_to_dark");
        assert_eq!(counts.len(), 2, "{}: {counts:?}", style.name);
        assert!(
            counts[0] > 0 && counts[1] == 0,
            "{}: {counts:?}",
            style.name
        );
    }
}
