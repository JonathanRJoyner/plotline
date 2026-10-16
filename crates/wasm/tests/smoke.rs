use wasm_bindgen_test::*;
#[wasm_bindgen_test]
fn grid_matches_the_golden_file() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("../../core/golden/grid_gradient.json")).unwrap();
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    // The golden file pins the 2 mm spacing it was checked against the Python engine at.
    s.update("points", r#"{"threshold":0.6,"spacing_mm":2}"#)
        .unwrap();
    let mut run = s.start().unwrap();
    let mut n = 0;
    while run.next_frame().is_some() {
        n += 1;
    }
    assert!(n >= 4);
    s.set_speeds(r#"{"draw_mm_s":10,"travel_mm_s":20,"pen_lift_s":0.5}"#)
        .unwrap();
    let speeds: serde_json::Value = serde_json::from_str(&s.speeds()).unwrap();
    assert_eq!(speeds["draw_mm_s"], 10.0);
    let stats: serde_json::Value = serde_json::from_str(&s.stats().unwrap()).unwrap();
    assert!(stats["estimated_seconds"].as_f64().unwrap() > 0.0);
    // The exact value is pinned through the core below, since serde_json's parser can move a
    // float's last bit.
    let lint: serde_json::Value = serde_json::from_str(&s.lint().unwrap()).unwrap();
    assert!(lint.as_array().unwrap().iter().any(|f| f["code"] == "dark"));
    let d = s.drawing().unwrap();
    let canvas: serde_json::Value = serde_json::from_str(&d.canvas()).unwrap();
    assert_eq!(canvas["width_mm"], 100.0);
    let rect: serde_json::Value = serde_json::from_str(&d.tone_rect().unwrap()).unwrap();
    assert_eq!(
        (
            rect["width_px"].as_u64().unwrap(),
            rect["height_px"].as_u64().unwrap()
        ),
        (64, 48)
    );
    // The 4:3 picture fits the 90 by 65 mm drawing rect at 86.67 mm wide, centred.
    assert!((rect["x"].as_f64().unwrap() - 20.0 / 3.0).abs() < 1e-9);
    assert_eq!(rect["y"], 5.0);
    let ink = d.tone_ink().unwrap();
    assert_eq!(ink.length(), 64 * 48);
    assert!(ink.get_index(0) < ink.get_index(63)); // the gradient is black at the left, white at the right
    let actual: serde_json::Value = serde_json::from_str(&s.json().unwrap()).unwrap();
    let (a, e) = (&actual["layers"][0]["paths"], &golden["layers"][0]["paths"]);
    assert_eq!(a.as_array().unwrap().len(), e.as_array().unwrap().len());
    for (pa, pe) in a.as_array().unwrap().iter().zip(e.as_array().unwrap()) {
        for (va, ve) in pa[0]
            .as_array()
            .unwrap()
            .iter()
            .zip(pe[0].as_array().unwrap())
        {
            assert!((va.as_f64().unwrap() - ve.as_f64().unwrap()).abs() <= 1e-3);
        }
    }
}
#[wasm_bindgen_test]
fn stipple_matches_the_golden_file() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("../../core/golden/stipple_gradient.json")).unwrap();
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.set_style("stipple").unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    s.update("voronoi", r#"{"count":300,"iterations":10}"#)
        .unwrap();
    // Points, so the file pins the relaxation and stays small.
    s.update("stipple", r#"{"shape":"point"}"#).unwrap();
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let actual: serde_json::Value = serde_json::from_str(&s.json().unwrap()).unwrap();
    let (a, e) = (&actual["layers"][0]["paths"], &golden["layers"][0]["paths"]);
    let (a, e) = (a.as_array().unwrap(), e.as_array().unwrap());
    assert_eq!(a.len(), e.len());
    for (i, (pa, pe)) in a.iter().zip(e).enumerate() {
        let (pa, pe) = (pa.as_array().unwrap(), pe.as_array().unwrap());
        assert_eq!(pa.len(), pe.len(), "path {i} length");
        for (va, ve) in pa.iter().zip(pe) {
            for k in 0..2 {
                let (x, y) = (va[k].as_f64().unwrap(), ve[k].as_f64().unwrap());
                assert!((x - y).abs() <= 1e-3, "path {i}: {va} vs {ve}");
            }
        }
    }
}
#[wasm_bindgen_test]
fn the_pen_bank_is_listed() {
    let pens: serde_json::Value = serde_json::from_str(&plotline_wasm::list_pens()).unwrap();
    assert!(pens.as_array().unwrap().len() >= 30);
    assert_eq!(pens[1]["name"], "Sakura Pigma Micron 005");
    assert_eq!(pens[1]["width_mm"], 0.2);
    assert_eq!(pens[1]["kind"], "fineliner");
    assert_eq!(pens[1]["model"], "Sakura Pigma Micron");
}
#[wasm_bindgen_test]
fn the_inks_are_listed() {
    let inks: serde_json::Value = serde_json::from_str(&plotline_wasm::list_inks()).unwrap();
    assert_eq!(
        inks[0],
        serde_json::json!({"name": "black", "color": "#000000"})
    );
    assert_eq!(inks.as_array().unwrap().len(), 10);
}
#[wasm_bindgen_test]
fn frames_carry_their_pen() {
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_pens(
        r##"[{"name":"A","width_mm":0.3,"color":"#000000"},
            {"name":"B","width_mm":0.3,"color":"#ff0000"}]"##,
    )
    .unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    s.update("split", r#"{"mode":"light_to_dark"}"#).unwrap();
    let mut run = s.start().unwrap();
    let mut pens = std::collections::BTreeSet::new();
    while let Some(frame) = run.next_frame() {
        pens.insert(frame.pen());
    }
    assert!(
        pens.contains(&1),
        "no frame came from the second pen: {pens:?}"
    );
}
#[wasm_bindgen_test]
fn machines_and_gcode() {
    let machine = r#"{"name": "Test printer",
        "area": {"x": 28, "y": 42, "width": 200, "height": 174},
        "z_down_mm": 4, "z_up_mm": 9, "z_safe_mm": 20,
        "draw_feed": 3600, "travel_feed": 6000, "z_feed": 1200, "settle_ms": 50,
        "start_gcode": "", "end_gcode": ""}"#;
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    assert!(s.gcode(0).is_err());
    s.set_machine(machine).unwrap();
    assert!(s.machine().unwrap().contains("Test printer"));
    let speeds: serde_json::Value = serde_json::from_str(&s.speeds()).unwrap();
    assert!((speeds["pen_lift_s"].as_f64().unwrap() - 0.6).abs() < 1e-9);
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let g = s.gcode(0).unwrap();
    assert!(g.contains("G1 Z4 F1200") && g.contains("G1 Z20 F1200"));
    s.set_machine("null").unwrap();
    assert!(s.machine().is_none());
}
#[wasm_bindgen_test]
fn still_life_analysis_matches_native() {
    // The same literals are pinned in crates/core/src/session.rs on the native target.
    let still_life = plotline_core::synthetic::still_life(600, 875);
    let stats = plotline_core::analysis::measure(&still_life);
    assert_eq!(stats.two_level, 0.8165753611527483);
    assert_eq!(stats.p_low, 0.1220703125);
    let mut s = plotline_wasm::Session::new();
    assert!(s.analysis().is_none());
    s.set_image(600, 875, &still_life.rgb).unwrap();
    let a: serde_json::Value = serde_json::from_str(&s.analysis().unwrap()).unwrap();
    assert_eq!(a["stats"]["height_px"], 875);
    assert!(a["suggestions"].is_array());
}
#[wasm_bindgen_test]
fn a_fit_takes_the_native_steps() {
    // The same literals as fit::tests::the_samples_are_pinned in the core crate.
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.set_style("waves").unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    let mut run = s.fit("metres", 10.0).unwrap();
    while run.next_frame().is_some() {}
    assert_eq!(run.state(), "done");
    let r: serde_json::Value = serde_json::from_str(&run.result().unwrap()).unwrap();
    let values: Vec<f64> = (r["samples"].as_array().unwrap().iter())
        .map(|x| x["value"].as_f64().unwrap())
        .collect();
    assert_eq!(values, [2.0, 1.472, 0.655, 0.883, 0.843]);
    // serde_json's parser can move a float's last bit, so the metric is checked within 1e-12.
    let achieved = r["achieved"].as_f64().unwrap();
    assert!((achieved - 9.934812216679127).abs() < 1e-12, "{achieved}");
    assert!(s.start().unwrap().result().is_none());
}
#[wasm_bindgen_test]
fn golden_grid_lint_matches_native() {
    // The same setup and literal are pinned in crates/core/src/lint.rs on the native target.
    use serde_json::json;
    let mut s = plotline_core::Session::new(1 << 24);
    s.set_image(plotline_core::synthetic::gradient(64, 48));
    let canvas = plotline_core::Canvas::new(100.0, 75.0, 5.0, plotline_core::Fit::Fit).unwrap();
    s.set_canvas(canvas);
    s.update("tone", &json!({"resolution": 64})).unwrap();
    s.update("points", &json!({"threshold": 0.6, "spacing_mm": 2}))
        .unwrap();
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let all = s.lint().unwrap();
    let dark = all.iter().find(|f| f.code == "dark").unwrap();
    assert_eq!(dark.value, 0.015042735042735043);
}
#[wasm_bindgen_test]
fn iso_lines_keep_their_levels_apart_past_u32() {
    // 256 x 256 cells have 2^17 edges, so on wasm32 `level * edges` wrapped to 0 at level 2^15
    // and that level's ring took level 0's keys. One cell at 40,000 gives one ring per level.
    let n = 256;
    let mut field = vec![0.0f32; n * n];
    field[128 * n + 128] = 40_000.0;
    let lines = plotline_core::raster::iso_lines(&field, n, n, 0.5, 1.0, 39_999.5);
    assert_eq!(lines.len(), 40_000);
    assert!(
        lines.iter().all(|l| l.len() == 5),
        "every ring is a closed diamond"
    );
}
