//! JSON paths export: a drawing as a JSON value that other programs can read.
//!
//! The format is named `plotline-paths` and has an integer `version`. Coordinates are page
//! coordinates in mm (origin at the top-left corner, y down) rounded to three decimals, so the same
//! drawing always gives the same text and golden files stay stable. A path of one vertex is a dot.

use serde_json::{json, Value};

use crate::types::Drawing;

/// `d` as `{"format", "version", "units", "canvas", "layers"}`, where each layer is the pen and its
/// paths as lists of `[x, y]` pairs. With a `preset` the object also has a `preset` key; the version
/// stays 1, because a reader that ignores unknown keys reads the file as before.
pub fn to_value(d: &Drawing, preset: Option<&Value>) -> Value {
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    let layers: Vec<Value> = d
        .layers
        .iter()
        .map(|layer| {
            let paths: Vec<Vec<[f64; 2]>> = layer
                .paths
                .iter()
                .map(|p| p.iter().map(|&[x, y]| [round(x), round(y)]).collect())
                .collect();
            json!({"pen": layer.pen, "paths": paths})
        })
        .collect();
    let mut value = json!({
        "format": "plotline-paths",
        "version": 1,
        "units": "mm",
        "canvas": d.canvas,
        "layers": layers,
    });
    if let Some(preset) = preset {
        value["preset"] = preset.clone();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::drawing_with;

    #[test]
    fn json_round_trips_and_rounds() {
        let v = to_value(&drawing_with(vec![vec![[1.23456, 2.0]]]), None);
        assert_eq!(v["format"], "plotline-paths");
        assert_eq!(v["version"], 1);
        assert_eq!(v["units"], "mm");
        assert_eq!(v["canvas"]["fit"], "fit");
        assert_eq!(v["layers"][0]["paths"][0][0][0], 1.235);
        assert!(v.get("preset").is_none());
    }

    #[test]
    fn json_carries_the_preset_only_when_given_one() {
        let d = drawing_with(vec![]);
        let preset = json!({"style": "grid"});
        let v = to_value(&d, Some(&preset));
        assert_eq!(v["preset"], preset);
        assert_eq!(v["version"], 1);
        let mut without = v.clone();
        without.as_object_mut().unwrap().remove("preset");
        assert_eq!(without, to_value(&d, None));
    }
}
