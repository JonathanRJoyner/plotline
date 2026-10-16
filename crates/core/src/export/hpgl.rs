//! HPGL export: a whole drawing as one plotter program.
//!
//! The instruction set is the one described in Hewlett-Packard's *The HP-GL/2 and HP RTL
//! Reference Guide* (Addison-Wesley, 1997). Only four instructions are used, `IN`, `SP`, `PU` and
//! `PD`, all from the original HP-GL set that HP-GL/2 kept, so both kinds of plotter accept the
//! file. Every layer that has paths selects its own pen with `SP`, so a multi-pen plotter draws
//! the whole drawing from one file. One instruction goes on each line, every line ends with `;`,
//! and the text is ASCII. No velocity, force or plotter-specific instruction is written: plotters
//! differ, so a pen number above a plotter's carousel and coordinates past its hard-clip limits
//! are the user's to check.
//!
//! Coordinates are whole plotter units. HPGL's y axis points up from the page's lower-left
//! corner, while the drawing's points down from the top-left, so y is flipped.

use thiserror::Error;

use crate::types::Drawing;

/// Why a drawing could not be written.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum HpglError {
    /// The resolution was not a finite number above 0.
    #[error("plotter units per mm must be a number above 0, not {0}")]
    UnitsPerMm(f64),
}

/// `d` as an HPGL program with `units_per_mm` plotter units to the millimetre; 40, HPGL's
/// plotter unit of 0.025 mm, suits most plotters.
///
/// The program is `IN;`, then for each layer that has paths `SP{n};` (pen `layer + 1`) and, for
/// each path, `PU x,y;` to its first vertex and `PD` with the rest as one comma-separated list;
/// then `PU;` and `SP0;`, which lift the pen and put it away. A one-vertex path is a dot: `PD;`
/// with no coordinates lowers the pen where it stands. A resolution that is not finite, or not
/// above 0, is refused.
pub fn to_hpgl(d: &Drawing, units_per_mm: f64) -> Result<String, HpglError> {
    if !(units_per_mm.is_finite() && units_per_mm > 0.0) {
        return Err(HpglError::UnitsPerMm(units_per_mm));
    }
    let h = d.canvas.height_mm;
    // Whole units; `as i64` cannot make "-0".
    let pt = |[x, y]: [f64; 2]| {
        format!(
            "{},{}",
            (x * units_per_mm).round() as i64,
            ((h - y) * units_per_mm).round() as i64
        )
    };
    let mut out = vec!["IN;".to_string()];
    for (i, layer) in d.layers.iter().enumerate() {
        if layer.paths.is_empty() {
            continue;
        }
        out.push(format!("SP{};", i + 1));
        for path in layer.paths.iter() {
            let Some((&first, rest)) = path.split_first() else {
                continue;
            };
            out.push(format!("PU{};", pt(first)));
            out.push(if rest.is_empty() {
                "PD;".to_string()
            } else {
                let rest: Vec<String> = rest.iter().map(|&p| pt(p)).collect();
                format!("PD{};", rest.join(","))
            });
        }
    }
    out.extend(["PU;".to_string(), "SP0;".to_string()]);
    Ok(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{drawing_100, drawing_with};

    #[test]
    fn the_program_has_one_instruction_to_a_line() {
        let paths = vec![
            vec![[2.0, 3.0], [4.0, 3.0], [4.0, 5.0]],
            vec![[10.0, 10.0], [11.0, 10.0]],
        ];
        let h = to_hpgl(&drawing_100(paths, 0.3, "#000000"), 40.0).unwrap();
        assert_eq!(
            h,
            "IN;\nSP1;\nPU80,3880;\nPD160,3880,160,3800;\nPU400,3600;\nPD440,3600;\nPU;\nSP0;\n"
        );
    }

    #[test]
    fn y_is_flipped_about_the_page_height() {
        // The A4 page is 297 mm high, so y = 20 mm is 277 mm above the bottom edge.
        let h = to_hpgl(&drawing_with(vec![vec![[20.0, 20.0]]]), 40.0).unwrap();
        assert!(h.contains("PU800,11080;\n"), "{h}");
        let h = to_hpgl(&drawing_with(vec![vec![[0.0, 297.0], [0.0, 0.0]]]), 40.0).unwrap();
        assert!(h.contains("PU0,0;\nPD0,11880;\n"), "{h}");
    }

    #[test]
    fn coordinates_are_rounded_to_whole_units() {
        // 0.0124 mm is under half a unit, 0.0126 mm over; y = 99.9876 flips to 0.0124 mm.
        let h = to_hpgl(
            &drawing_100(
                vec![vec![[0.0124, 99.9874], [0.0126, 99.9876]]],
                0.3,
                "#000000",
            ),
            40.0,
        )
        .unwrap();
        assert!(h.contains("PU0,1;\nPD1,0;\n"), "{h}");
        let h = to_hpgl(
            &drawing_100(vec![vec![[1.0, 1.0], [2.0, 1.0]]], 0.3, "#000000"),
            10.5,
        )
        .unwrap();
        assert!(h.contains("PU11,1040;\nPD21,1040;\n"), "{h}");
    }

    #[test]
    fn a_dot_lowers_the_pen_where_it_stands() {
        let h = to_hpgl(&drawing_100(vec![vec![[5.0, 5.0]]], 0.3, "#000000"), 40.0).unwrap();
        assert!(h.contains("PU200,3800;\nPD;\nPU;\n"), "{h}");
    }

    #[test]
    fn each_layer_selects_its_own_pen_and_an_empty_one_is_skipped() {
        let mut d = drawing_100(vec![vec![[1.0, 1.0]]], 0.3, "#000000");
        d.layers.extend(drawing_100(vec![], 0.5, "#ff0000").layers);
        d.layers
            .extend(drawing_100(vec![vec![[2.0, 2.0]]], 0.5, "#00ff00").layers);
        let h = to_hpgl(&d, 40.0).unwrap();
        assert_eq!(
            h,
            "IN;\nSP1;\nPU40,3960;\nPD;\nSP3;\nPU80,3920;\nPD;\nPU;\nSP0;\n"
        );
    }

    #[test]
    fn a_drawing_with_no_paths_is_only_the_frame() {
        let h = to_hpgl(&drawing_100(vec![], 0.3, "#000000"), 40.0).unwrap();
        assert_eq!(h, "IN;\nPU;\nSP0;\n");
    }

    #[test]
    fn a_resolution_that_is_not_above_zero_is_refused() {
        let d = drawing_100(vec![vec![[1.0, 1.0]]], 0.3, "#000000");
        for bad in [0.0, -40.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = to_hpgl(&d, bad).unwrap_err();
            assert!(err.to_string().contains("above 0"), "{bad}: {err}");
        }
    }
}
