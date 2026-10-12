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
