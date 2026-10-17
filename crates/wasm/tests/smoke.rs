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
    // Speeds saved before acceleration counted take the default.
    assert_eq!(speeds["accel_mm_s2"], 1000.0);
    let stats: serde_json::Value = serde_json::from_str(&s.stats().unwrap()).unwrap();
    assert!(stats["estimated_seconds"].as_f64().unwrap() > 0.0);
    // The exact value is pinned through the core below, since serde_json's parser can move a
    // float's last bit.
    let lint: serde_json::Value = serde_json::from_str(&s.lint().unwrap()).unwrap();
    assert!(lint.as_array().unwrap().iter().any(|f| f["code"] == "dark"));
    let d = s.drawing().unwrap();
    let canvas: serde_json::Value = serde_json::from_str(&d.canvas()).unwrap();
    assert_eq!(canvas["width_mm"], 100.0);
    // The playback's times: one per vertex, a move per path, ending at the plot time.
    let t = s.timeline().unwrap();
    assert_eq!((t.layer_count(), t.start()), (1, vec![5.0, 5.0]));
    let vertices = t.layer_vertices(0).unwrap();
    assert_eq!(2 * vertices.length(), d.layer_coords(0).unwrap().length());
    assert_eq!(
        t.layer_travel(0).unwrap().length(),
        2 * (d.layer_offsets(0).unwrap().length() - 1)
    );
    let end = vertices.get_index(vertices.length() - 1);
    let total = stats["estimated_seconds"].as_f64().unwrap();
    assert!((end - total).abs() < 1e-6 * total, "{end} {total}");
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
    // The exported SVG holds the run's preset.
    let svg = s.svg().unwrap();
    let preset: serde_json::Value =
        serde_json::from_str(&plotline_wasm::preset_from_text(&svg).unwrap()).unwrap();
    assert_eq!(preset["style"], "grid");
    assert_eq!(preset["settings"]["points"]["spacing_mm"], 2.0);
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
fn hpgl_is_one_program_without_a_machine() {
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    assert!(s.hpgl(40.0).is_err());
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let h = s.hpgl(40.0).unwrap();
    assert!(h.starts_with("IN;\nSP1;\nPU") && h.ends_with("PU;\nSP0;\n"));
    assert!(s.hpgl(0.0).is_err());
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
fn framing_matches_native() {
    // The same literals as analysis::tests::the_framing_crop_is_pinned in the core crate: a dark
    // disc of radius 0.05 at (0.7, 0.3) on a light 256 x 256 field, on the default A4 page.
    let rgb: Vec<f32> = (0..256 * 256)
        .flat_map(|i| {
            let (u, v) = (
                (i % 256) as f32 / 256.0 - 0.7,
                (i / 256) as f32 / 256.0 - 0.3,
            );
            [if u * u + v * v <= 0.05 * 0.05 {
                0.1
            } else {
                0.95
            }; 3]
        })
        .collect();
    let mut s = plotline_wasm::Session::new();
    s.set_image(256, 256, &rgb).unwrap();
    let a: serde_json::Value = serde_json::from_str(&s.analysis().unwrap()).unwrap();
    let framing = (a["suggestions"].as_array().unwrap().iter())
        .find(|g| g["kind"] == "framing")
        .expect("a framing suggestion");
    let crop: Vec<f64> = (framing["patch"]["page"]["crop"].as_array().unwrap().iter())
        .map(|v| v.as_f64().unwrap())
        .collect();
    for (got, want) in crop.iter().zip([0.43, 0.0, 0.01, 0.2]) {
        assert!((got - want).abs() < 1e-12, "{crop:?}");
    }
    // The page takes the crop, the preset carries it, and a bad one is refused.
    let page =
        r#"{"width_mm":210,"height_mm":297,"margin_mm":10,"fit":"fit","crop":[0.45,0,0.04,0.29]}"#;
    s.set_canvas(page).unwrap();
    assert!(s.preset().contains(r#""crop""#));
    assert!(s.set_canvas(&page.replace("0.45,0", "0.5,0")).is_err());
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
    assert_eq!(dark.value, 0.017834757834757835);
}
#[wasm_bindgen_test]
fn the_tone_match_and_the_response_match_native() {
    // The same setup and literals are pinned in crates/core/src/session.rs on the native target.
    use serde_json::json;
    let drain = |s: &mut plotline_core::Session| {
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
    };
    let mut s = plotline_core::Session::new(1 << 24);
    s.set_image(plotline_core::synthetic::gradient(64, 48));
    let canvas = plotline_core::Canvas::new(100.0, 75.0, 5.0, plotline_core::Fit::Fit).unwrap();
    s.set_canvas(canvas);
    s.set_style("waves").unwrap();
    s.update("tone", &json!({"resolution": 64})).unwrap();
    drain(&mut s);
    let off = s.tone_match().unwrap();
    assert_eq!(
        (off.error, off.ssim),
        (0.08197368769144689, 0.7488977156057808)
    );
    s.update("tone", &json!({"response": "auto"})).unwrap();
    drain(&mut s);
    assert_eq!(s.tone_match().unwrap().error, 0.08880655753709105);
}
#[wasm_bindgen_test]
fn the_focus_map_matches_native() {
    // The same setup and literal are pinned in crates/core/src/session.rs on the native target.
    use serde_json::json;
    let mut s = plotline_core::Session::new(1 << 24);
    s.set_image(plotline_core::synthetic::still_life(120, 120));
    let canvas = plotline_core::Canvas::new(100.0, 100.0, 0.0, plotline_core::Fit::Fit).unwrap();
    s.set_canvas(canvas);
    let focus = json!([[0.3, 0.4], [0.8, 0.7]]);
    s.update("tone", &json!({"resolution": 64, "focus": focus}))
        .unwrap();
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let drawing = s.drawing().unwrap();
    let ink = &drawing.tone.as_ref().unwrap().ink;
    let sum = (ink.iter()).fold(0u32, |h, v| h.wrapping_mul(31).wrapping_add(v.to_bits()));
    assert_eq!(sum, 686_476_666);
    // The wasm session passes the two placements through.
    let mut w = plotline_wasm::Session::new();
    assert!(w.image_point(50.0, 50.0).is_none());
    w.set_image(
        120,
        120,
        &plotline_core::synthetic::still_life(120, 120).rgb,
    )
    .unwrap();
    w.set_canvas(r#"{"width_mm":100,"height_mm":100,"margin_mm":0,"fit":"fit"}"#)
        .unwrap();
    assert_eq!(w.image_point(50.0, 50.0), Some(vec![0.5, 0.5]));
    assert_eq!(w.page_point(0.25, 0.5), Some(vec![25.0, 50.0]));
}
#[wasm_bindgen_test]
fn a_look_is_applied_and_read_back() {
    let mut s = plotline_wasm::Session::new();
    s.set_style("waves").unwrap();
    assert_eq!(s.look().as_deref(), Some("balanced"));
    s.apply_look("fine").unwrap();
    assert_eq!(s.look().as_deref(), Some("fine"));
    assert!(s.apply_look("no such look").is_err());
    assert_eq!(s.look().as_deref(), Some("fine"));
    s.update("waves", r#"{"spacing_mm":1.7}"#).unwrap();
    assert_eq!(s.look(), None);
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
