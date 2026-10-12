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
    s.update("points", r#"{"threshold":0.6}"#).unwrap();
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
fn the_pen_bank_is_listed() {
    let pens: serde_json::Value = serde_json::from_str(&plotline_wasm::list_pens()).unwrap();
    assert!(pens.as_array().unwrap().len() >= 30);
    assert_eq!(pens[1]["name"], "Sakura Pigma Micron 005");
    assert_eq!(pens[1]["width_mm"], 0.2);
}
#[wasm_bindgen_test]
fn machines_and_gcode() {
    let machines: serde_json::Value =
        serde_json::from_str(&plotline_wasm::list_machines()).unwrap();
    assert!(machines[0]["name"].as_str().unwrap().contains("A1"));
    let mut s = plotline_wasm::Session::new();
    s.set_image(64, 48, &plotline_wasm::synthetic_gradient(64, 48))
        .unwrap();
    s.set_canvas(r#"{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}"#)
        .unwrap();
    s.update("tone", r#"{"resolution":64}"#).unwrap();
    assert!(s.gcode(0).is_err());
    s.set_machine(&machines[0].to_string()).unwrap();
    assert!(s.machine().unwrap().contains("A1"));
    let speeds: serde_json::Value = serde_json::from_str(&s.speeds()).unwrap();
    assert!((speeds["pen_lift_s"].as_f64().unwrap() - 0.6).abs() < 1e-9);
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let g = s.gcode(0).unwrap();
    assert!(g.contains("G1 Z4 F1200") && g.contains("G1 Z20 F1200"));
    s.set_machine("null").unwrap();
    assert!(s.machine().is_none());
}
