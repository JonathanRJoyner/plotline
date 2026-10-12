//! JSON paths export: a drawing as a JSON value that other programs can read.
//!
//! The format is named `plotline-paths` and has an integer `version`. Coordinates are page
//! coordinates in mm (origin at the top-left corner, y down) rounded to three decimals, so the same
//! drawing always gives the same text and golden files stay stable. A path of one vertex is a dot.

use serde_json::{json, Value};

use crate::types::Drawing;

/// `d` as `{"format", "version", "units", "canvas", "layers"}`, where each layer is the pen and its
/// paths as lists of `[x, y]` pairs.
pub fn to_value(d: &Drawing) -> Value {
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
    json!({
        "format": "plotline-paths",
        "version": 1,
        "units": "mm",
        "canvas": d.canvas,
        "layers": layers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::drawing_with;

    #[test]
    fn json_round_trips_and_rounds() {
        let v = to_value(&drawing_with(vec![vec![[1.23456, 2.0]]]));
        assert_eq!(v["format"], "plotline-paths");
        assert_eq!(v["version"], 1);
        assert_eq!(v["units"], "mm");
        assert_eq!(v["canvas"]["fit"], "fit");
        assert_eq!(v["layers"][0]["paths"][0][0][0], 1.235);
    }
}
