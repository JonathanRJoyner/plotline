//! G-code export: one pen layer of a drawing as a program for a [`Machine`].
//!
//! The dialect is strict so a downstream checker never has to guess: upper case, one command per
//! line, `;` comments only, `G21` and `G90` before any move, no `G28`, no arcs, no line that moves
//! X/Y and Z together, and numbers with at most three decimals and no trailing zeros. Each layer
//! is its own file with no pause, because a printer that pauses by reheating makes each pen its own
//! job. The page is centred in the machine's drawable area by [`Machine::mapping`], which refuses
//! a page that does not fit, and every vertex is checked to lie on the page, so no move can leave
//! the area.

use thiserror::Error;

use crate::machine::{Machine, MachineError};
use crate::types::Drawing;

/// Why a layer could not be written.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GcodeError {
    /// The page does not fit the machine's area.
    #[error(transparent)]
    Machine(#[from] MachineError),
    /// The drawing has no layer with this index; the count follows.
    #[error("no layer {0}; the drawing has {1}")]
    NoLayer(usize, usize),
    /// A vertex lies off the page, so it could land outside the machine's area.
    #[error("path {path} leaves the page at ({x}, {y})")]
    OffPage { path: usize, x: String, y: String },
}

/// `v` with at most three decimals and no trailing zeros; `-0` is written as `0`.
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "" | "-0" => "0".to_string(),
        s => s.to_string(),
    }
}

/// Layer `layer` of `d` as G-code for `m`.
///
/// The program is a header comment naming the pen and counting the paths, a reminder that the
/// file has no `G28`, `G21`, `G90`, the machine's start lines, a lift to the safe height, then for
/// each path a travel to its first vertex (left out when the pen is already there), the pen down
/// (Z, dwell, draw feed), one `G1` per further vertex, the pen up (Z, dwell, draw feed); then a
/// lift to the safe height and the machine's end lines. A one-vertex path is a dot: down and up
/// at one point.
pub fn to_gcode(d: &Drawing, layer: usize, m: &Machine) -> Result<String, GcodeError> {
    let count = d.layers.len();
    let l = d
        .layers
        .get(layer)
        .ok_or(GcodeError::NoLayer(layer, count))?;
    let map = m.mapping(d.canvas.width_mm, d.canvas.height_mm)?;
    let name = l.pen.name.replace(['\n', '\r'], " ");
    let z_move = |z: f64| format!("G1 Z{} F{}", num(z), num(m.z_feed));
    let pen_to = |z: f64| {
        [
            z_move(z),
            format!("G4 P{}", m.settle_ms),
            format!("G1 F{}", num(m.draw_feed)),
        ]
    };
    let mut out = vec![
        format!(
            "; plotline: {name}, {} paths, pen {} of {count}",
            l.paths.len(),
            layer + 1
        ),
        "; Home the machine before mounting the pen: this file has no G28.".to_string(),
        "G21".to_string(),
        "G90".to_string(),
    ];
    out.extend(m.start_gcode.lines().map(str::to_string));
    out.push(z_move(m.z_safe_mm));
    let mut pen: Option<[f64; 2]> = None;
    let (w, h) = (d.canvas.width_mm, d.canvas.height_mm);
    for (i, path) in l.paths.iter().enumerate() {
        // The mapping keeps the page inside the area, so a vertex on the page is inside it too.
        if let Some(&[x, y]) = path
            .iter()
            .find(|[x, y]| !(-1e-6..=w + 1e-6).contains(x) || !(-1e-6..=h + 1e-6).contains(y))
        {
            return Err(GcodeError::OffPage {
                path: i,
                x: num(x),
                y: num(y),
            });
        }
        let Some(&first) = path.first() else {
            continue;
        };
        let first = map.to_machine(first);
        let there =
            pen.is_some_and(|p| (p[0] - first[0]).abs() < 1e-9 && (p[1] - first[1]).abs() < 1e-9);
        if !there {
            out.push(format!(
                "G0 X{} Y{} F{}",
                num(first[0]),
                num(first[1]),
                num(m.travel_feed)
            ));
        }
        out.extend(pen_to(m.z_down_mm));
        for &p in &path[1..] {
            let [x, y] = map.to_machine(p);
            out.push(format!("G1 X{} Y{}", num(x), num(y)));
        }
        out.extend(pen_to(m.z_up_mm));
        pen = Some(map.to_machine(*path.last().unwrap()));
    }
    out.push(z_move(m.z_safe_mm));
    out.extend(m.end_gcode.lines().map(str::to_string));
    Ok(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::builtin;
    use crate::test_util::{drawing_100, drawing_with};

    fn a1() -> Machine {
        builtin().remove(0)
    }

    /// Walk `gcode` the way a strict downstream checker does and panic at the first unsafe line:
    /// only the allowed commands, G21 and G90 before any move, no line moving X/Y and Z together,
    /// the first X/Y move only once Z is at the safe height, every X/Y below the safe height
    /// inside the area, Z never below the pen-down height, and X/Y inside the bed.
    fn check(gcode: &str, m: &Machine) {
        const ALLOWED: [&str; 10] = [
            "G0", "G1", "G4", "G21", "G90", "M104", "M140", "M400", "M106", "M107",
        ];
        let (mut g21, mut g90, mut z, mut moved) = (false, false, None::<f64>, false);
        for (n, raw) in gcode.lines().enumerate() {
            let line = raw.split(';').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let code = words.next().unwrap();
            assert!(
                ALLOWED.contains(&code),
                "line {n}: {code} is not allowed: {raw}"
            );
            let word = |letter: char| {
                line.split_whitespace()
                    .find(|w| w.starts_with(letter))
                    .map(|w| w[1..].parse::<f64>().unwrap())
            };
            match code {
                "G21" => g21 = true,
                "G90" => g90 = true,
                "G0" | "G1" => {
                    assert!(g21 && g90, "line {n}: a move before G21 and G90: {raw}");
                    let (x, y, dz) = (word('X'), word('Y'), word('Z'));
                    assert!(
                        !((x.is_some() || y.is_some()) && dz.is_some()),
                        "line {n}: X/Y and Z in one move: {raw}"
                    );
                    if let Some(dz) = dz {
                        assert!(
                            dz >= m.z_down_mm - 1e-9,
                            "line {n}: Z below pen down: {raw}"
                        );
                        z = Some(dz);
                    }
                    if let (Some(x), Some(y)) = (x, y) {
                        let safe = z.is_some_and(|z| z >= m.z_safe_mm - 1e-9);
                        assert!(
                            moved || safe,
                            "line {n}: first X/Y move before the lift: {raw}"
                        );
                        moved = true;
                        assert!(
                            0.0 <= x && x <= m.bed_mm[0] && 0.0 <= y && y <= m.bed_mm[1],
                            "line {n}: outside the bed: {raw}"
                        );
                        assert!(
                            safe || m.area.contains([x, y], 1e-6),
                            "line {n}: outside the area with the pen low: {raw}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(28.0), "28");
        assert_eq!(num(1.23456), "1.235");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(3600.0), "3600");
    }

    #[test]
    fn a_dot_is_down_and_up_at_one_point() {
        // A 100 mm page centred in the 200 by 174 mm area starts at (78, 179).
        let g = to_gcode(
            &drawing_100(vec![vec![[10.0, 20.0]]], 0.3, "#000000"),
            0,
            &a1(),
        )
        .unwrap();
        assert!(
            g.contains(
                "G0 X88 Y159 F6000\nG1 Z4 F1200\nG4 P50\nG1 F3600\nG1 Z9 F1200\nG4 P50\nG1 F3600\n"
            ),
            "{g}"
        );
        assert!(
            g.starts_with("; plotline: Pen, 1 paths, pen 1 of 1\n"),
            "{g}"
        );
        check(&g, &a1());
    }

    #[test]
    fn a_line_draws_its_further_vertices_and_skips_a_travel_of_nothing() {
        let paths = vec![
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            vec![[10.0, 10.0], [20.0, 10.0]],
        ];
        let g = to_gcode(&drawing_100(paths, 0.3, "#000000"), 0, &a1()).unwrap();
        assert!(
            g.contains("G1 F3600\nG1 X88 Y179\nG1 X88 Y169\nG1 Z9 F1200\n"),
            "{g}"
        );
        assert_eq!(g.matches("G0 X").count(), 1, "{g}");
        check(&g, &a1());
    }

    #[test]
    fn the_program_lifts_first_and_ends_with_the_machine_lines() {
        let paths = vec![
            vec![[0.0, 0.0], [100.0, 100.0]],
            vec![[50.0, 50.0]],
            vec![[100.0, 0.0], [0.0, 100.0]],
        ];
        let g = to_gcode(&drawing_100(paths, 0.3, "#000000"), 0, &a1()).unwrap();
        let lift = g.find("G1 Z20 F1200\n").unwrap();
        assert!(g.find("G0 X").unwrap() > lift, "{g}");
        assert!(
            g.contains("G21\nG90\nM104 S0\nM140 S0\nG1 Z20 F1200\n"),
            "{g}"
        );
        assert!(
            g.ends_with("G1 Z20 F1200\nG1 Z20 F600\nG1 Y250 F3600\n"),
            "{g}"
        );
        check(&g, &a1());
    }

    #[test]
    fn a_page_larger_than_the_area_is_refused() {
        let err = to_gcode(&drawing_with(vec![vec![[1.0, 1.0]]]), 0, &a1()).unwrap_err();
        assert!(err.to_string().contains("larger"), "{err}");
    }

    #[test]
    fn a_missing_layer_is_named() {
        let err = to_gcode(&drawing_100(vec![], 0.3, "#000000"), 1, &a1()).unwrap_err();
        assert_eq!(err.to_string(), "no layer 1; the drawing has 1");
    }

    #[test]
    fn a_vertex_off_the_page_is_refused() {
        let paths = vec![vec![[5.0, 5.0]], vec![[5.0, 5.0], [-1.0, 5.0]]];
        let err = to_gcode(&drawing_100(paths, 0.3, "#000000"), 0, &a1()).unwrap_err();
        assert_eq!(err.to_string(), "path 1 leaves the page at (-1, 5)");
    }
}
