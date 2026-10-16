//! A single-stroke font drawn as paths, for labels on a plot.
//!
//! The glyphs were drawn for plotline: a few straight strokes each on a grid 4 units wide and 6
//! high, with y down and the baseline at 6. No font file or third-party glyph data is used. The
//! font has `A`-`Z`, `0`-`9`, `.`, `-`, `:`, `/` and the space; lowercase letters draw as their
//! capitals and any other character draws as a space.

/// The grid is 4 units wide and 6 high; a glyph advances by its width plus a gap of one unit.
const ROWS: f64 = 6.0;
const ADVANCE: f64 = 5.0;

type Glyph = &'static [&'static [[u8; 2]]];

/// The strokes of `c` on the grid, or `None` when the font does not draw it.
#[rustfmt::skip]
fn glyph(c: char) -> Option<Glyph> {
    let g: Glyph = match c.to_ascii_uppercase() {
        'A' => &[&[[0, 6], [0, 2], [2, 0], [4, 2], [4, 6]], &[[0, 4], [4, 4]]],
        'B' => &[&[[0, 0], [0, 6], [3, 6], [4, 5], [4, 4], [3, 3], [0, 3]], &[[0, 0], [3, 0], [4, 1], [4, 2], [3, 3]]],
        'C' => &[&[[4, 1], [3, 0], [1, 0], [0, 1], [0, 5], [1, 6], [3, 6], [4, 5]]],
        'D' => &[&[[0, 0], [0, 6], [2, 6], [4, 4], [4, 2], [2, 0], [0, 0]]],
        'E' => &[&[[4, 0], [0, 0], [0, 6], [4, 6]], &[[0, 3], [3, 3]]],
        'F' => &[&[[4, 0], [0, 0], [0, 6]], &[[0, 3], [3, 3]]],
        'G' => &[&[[4, 1], [3, 0], [1, 0], [0, 1], [0, 5], [1, 6], [3, 6], [4, 5], [4, 3], [2, 3]]],
        'H' => &[&[[0, 0], [0, 6]], &[[4, 0], [4, 6]], &[[0, 3], [4, 3]]],
        'I' => &[&[[0, 0], [4, 0]], &[[2, 0], [2, 6]], &[[0, 6], [4, 6]]],
        'J' => &[&[[4, 0], [4, 5], [3, 6], [1, 6], [0, 5]]],
        'K' => &[&[[0, 0], [0, 6]], &[[4, 0], [0, 3], [4, 6]]],
        'L' => &[&[[0, 0], [0, 6], [4, 6]]],
        'M' => &[&[[0, 6], [0, 0], [2, 3], [4, 0], [4, 6]]],
        'N' => &[&[[0, 6], [0, 0], [4, 6], [4, 0]]],
        'O' => &[&[[1, 0], [3, 0], [4, 1], [4, 5], [3, 6], [1, 6], [0, 5], [0, 1], [1, 0]]],
        'P' => &[&[[0, 6], [0, 0], [3, 0], [4, 1], [4, 2], [3, 3], [0, 3]]],
        'Q' => &[&[[1, 0], [3, 0], [4, 1], [4, 5], [3, 6], [1, 6], [0, 5], [0, 1], [1, 0]], &[[2, 4], [4, 6]]],
        'R' => &[&[[0, 6], [0, 0], [3, 0], [4, 1], [4, 2], [3, 3], [0, 3]], &[[2, 3], [4, 6]]],
        'S' => &[&[[4, 1], [3, 0], [1, 0], [0, 1], [0, 2], [1, 3], [3, 3], [4, 4], [4, 5], [3, 6], [1, 6], [0, 5]]],
        'T' => &[&[[0, 0], [4, 0]], &[[2, 0], [2, 6]]],
        'U' => &[&[[0, 0], [0, 5], [1, 6], [3, 6], [4, 5], [4, 0]]],
        'V' => &[&[[0, 0], [2, 6], [4, 0]]],
        'W' => &[&[[0, 0], [1, 6], [2, 3], [3, 6], [4, 0]]],
        'X' => &[&[[0, 0], [4, 6]], &[[4, 0], [0, 6]]],
        'Y' => &[&[[0, 0], [2, 3], [4, 0]], &[[2, 3], [2, 6]]],
        'Z' => &[&[[0, 0], [4, 0], [0, 6], [4, 6]]],
        // The zero's slash tells it from the O.
        '0' => &[&[[1, 0], [3, 0], [4, 1], [4, 5], [3, 6], [1, 6], [0, 5], [0, 1], [1, 0]], &[[1, 4], [3, 2]]],
        '1' => &[&[[1, 1], [2, 0], [2, 6]], &[[1, 6], [3, 6]]],
        '2' => &[&[[0, 1], [1, 0], [3, 0], [4, 1], [4, 2], [0, 6], [4, 6]]],
        '3' => &[&[[0, 1], [1, 0], [3, 0], [4, 1], [4, 2], [3, 3], [1, 3]], &[[3, 3], [4, 4], [4, 5], [3, 6], [1, 6], [0, 5]]],
        '4' => &[&[[3, 6], [3, 0], [0, 4], [4, 4]]],
        '5' => &[&[[4, 0], [0, 0], [0, 3], [3, 3], [4, 4], [4, 5], [3, 6], [1, 6], [0, 5]]],
        '6' => &[&[[4, 1], [3, 0], [1, 0], [0, 1], [0, 5], [1, 6], [3, 6], [4, 5], [4, 4], [3, 3], [0, 3]]],
        '7' => &[&[[0, 0], [4, 0], [1, 6]]],
        '8' => &[&[[1, 0], [3, 0], [4, 1], [4, 2], [3, 3], [1, 3], [0, 2], [0, 1], [1, 0]], &[[1, 3], [0, 4], [0, 5], [1, 6], [3, 6], [4, 5], [4, 4], [3, 3]]],
        '9' => &[&[[0, 5], [1, 6], [3, 6], [4, 5], [4, 1], [3, 0], [1, 0], [0, 1], [0, 2], [1, 3], [4, 3]]],
        '.' => &[&[[2, 5], [2, 6]]],
        '-' => &[&[[1, 3], [3, 3]]],
        ':' => &[&[[2, 1], [2, 2]], &[[2, 4], [2, 5]]],
        '/' => &[&[[0, 6], [4, 0]]],
        ' ' => &[],
        _ => return None,
    };
    Some(g)
}

/// The strokes of `s` as polylines in page mm, y down. `origin` is the top-left corner of the
/// first glyph and `height` the height of the 6-unit grid in mm.
pub fn paths(s: &str, origin: [f64; 2], height: f64) -> Vec<Vec<[f64; 2]>> {
    let unit = height / ROWS;
    let mut out = Vec::new();
    for (i, c) in s.chars().enumerate() {
        let left = origin[0] + i as f64 * ADVANCE * unit;
        for stroke in glyph(c).unwrap_or(&[]) {
            let at =
                |&[x, y]: &[u8; 2]| [left + f64::from(x) * unit, origin[1] + f64::from(y) * unit];
            out.push(stroke.iter().map(at).collect());
        }
    }
    out
}

/// The advance of `s` in mm: each character, drawn or not, takes 5 units, so the last one's gap
/// is counted.
pub fn width(s: &str, height: f64) -> f64 {
    s.chars().count() as f64 * ADVANCE * height / ROWS
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/";

    #[test]
    fn every_character_the_chart_draws_has_a_glyph() {
        for c in DRAWN.chars() {
            let p = paths(&c.to_string(), [0.0, 0.0], 6.0);
            assert!(!p.is_empty(), "{c}");
            for v in p.iter().flatten() {
                assert!(
                    (0.0..=4.0).contains(&v[0]) && (0.0..=6.0).contains(&v[1]),
                    "{c}"
                );
            }
        }
        assert_eq!(paths("a", [0.0, 0.0], 6.0), paths("A", [0.0, 0.0], 6.0));
        let glyphs: std::collections::HashSet<String> = DRAWN
            .chars()
            .map(|c| format!("{:?}", paths(&c.to_string(), [0.0, 0.0], 6.0)))
            .collect();
        assert_eq!(glyphs.len(), DRAWN.len(), "two characters share a glyph");
    }
    #[test]
    fn width_is_the_advance() {
        assert_eq!(width("", 6.0), 0.0);
        assert_eq!(width("AB-", 6.0), 15.0);
        assert_eq!(width("AB", 3.0), 5.0);
        // The second glyph starts one advance after the first.
        let ab = paths("AB", [10.0, 20.0], 12.0);
        assert_eq!(ab[0][0], [10.0, 32.0]);
        assert_eq!(ab[2][0], [10.0 + 10.0, 20.0 + 0.0]);
    }
    #[test]
    fn unknown_characters_and_spaces_draw_nothing() {
        assert!(paths("  \u{e9}~#", [0.0, 0.0], 6.0).is_empty());
        assert_eq!(width("  \u{e9}~#", 6.0), 25.0);
    }
}
