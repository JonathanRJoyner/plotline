//! The test chart: a page of small labelled patches for one pen, to plot and compare by eye.
//!
//! The engine's physical numbers are guesses until someone checks them on paper. The chart shows
//! where lines of a given spacing merge, how big dots of a given size come out, how many passes
//! the paper takes, and, from the stats' plot time, whether the speeds are right. Every size
//! follows the pen width `w`, so the patches mean the same on any pen; see
//! `docs/superpowers/specs/2026-10-16-testchart-design.md` and the brief's decision log.

use crate::stages::stipple::{dot_path, DotShape};
use crate::text;
use crate::types::{Canvas, Drawing, Layer, Paths, Pen};

/// Line spacings of the first section, in pen widths.
const SPACINGS: [f64; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
/// Dot diameters of the second section, in pen widths.
const DOTS: [f64; 6] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0];
/// Times the third section's solid fill is drawn over.
const PASSES: [u32; 6] = [1, 2, 3, 4, 6, 8];
/// Pens thinner than this would make a dot grid of millions of dots.
const MIN_PEN_MM: f64 = 0.05;

/// The chart for `pen` on `canvas`: one layer, no tone map.
///
/// Fails when the pen is thinner than 0.05 mm, or when the content does not fit the drawing area.
pub fn chart(canvas: &Canvas, pen: &Pen) -> Result<Drawing, String> {
    let w = pen.width_mm;
    if w < MIN_PEN_MM {
        return Err(format!(
            "the pen is too thin for the test chart: {w} mm; it needs at least {MIN_PEN_MM} mm"
        ));
    }
    let [x0, y0, aw, ah] = canvas.drawing_rect();
    let side = (8.0 * w).max(10.0);
    let mut page = Page {
        paths: Paths::new(),
        x0,
        right: x0 + aw,
        side,
        gap: side / 2.0,
        em: (5.0 * w).max(2.5),
        y: y0,
        reach: x0,
    };
    let title = format!("TEST CHART PEN {w:.2} MM");
    page.text(&title, [x0, y0]);
    page.y += page.em + page.gap;

    let mm = |f: f64| format!("{:.2}", f * w);
    let labels: Vec<String> = SPACINGS.iter().map(|&f| mm(f)).collect();
    page.section("SPACING MM", &labels, |i, out, at| {
        lines(out, at, side, SPACINGS[i] * w, 1);
    });
    let labels: Vec<String> = DOTS.iter().map(|&f| mm(f)).collect();
    page.section("DOT MM", &labels, |i, out, at| {
        dots(out, at, side, DOTS[i] * w, w);
    });
    let labels: Vec<String> = PASSES.iter().map(|n| format!("X{n}")).collect();
    page.section("PASSES", &labels, |i, out, at| {
        lines(out, at, side, w, PASSES[i]);
    });

    let height = page.y - page.gap - y0;
    if height > ah {
        return Err(format!(
            "the page is too small for the test chart: it needs {height:.0} mm of the drawing \
             area's height and has {ah:.0} mm"
        ));
    }
    let width = page.reach - x0;
    if width > aw {
        return Err(format!(
            "the page is too small for the test chart: it needs {width:.0} mm of the drawing \
             area's width and has {aw:.0} mm"
        ));
    }
    Ok(Drawing {
        canvas: canvas.clone(),
        layers: vec![Layer {
            pen: pen.clone(),
            paths: page.paths,
        }],
        tone: None,
    })
}

/// The chart as it is laid out: the paths so far and the row the next thing starts on.
struct Page {
    paths: Paths,
    /// The left and right edges of the drawing area.
    x0: f64,
    right: f64,
    /// A patch's side `P`, the gap `G` between patches and rows, and the text height `T`.
    side: f64,
    gap: f64,
    em: f64,
    /// The top of the next row.
    y: f64,
    /// The furthest right anything reaches.
    reach: f64,
}

impl Page {
    /// Draw `s` with its top-left corner at `at`.
    fn text(&mut self, s: &str, at: [f64; 2]) {
        for stroke in text::paths(s, at, self.em) {
            self.paths.push(&stroke);
        }
        self.reach = self.reach.max(at[0] + self.ink_width(s));
    }

    /// The width of `s` without the gap its last glyph advances by.
    fn ink_width(&self, s: &str) -> f64 {
        text::width(s, self.em) - self.em / 6.0
    }

    /// A title, then the patches left to right with `labels[i]` under patch `i`, drawn by
    /// `draw(i, paths, top_left)`. The row wraps when the next patch or label would pass the
    /// drawing area's right edge, and the patches move apart when a label is wider than the
    /// space between them.
    fn section(
        &mut self,
        title: &str,
        labels: &[String],
        draw: impl Fn(usize, &mut Paths, [f64; 2]),
    ) {
        let (side, em) = (self.side, self.em);
        let row = side + 1.5 * em;
        self.text(title, [self.x0, self.y]);
        self.y += 1.5 * em;
        let widest = labels.iter().map(|l| self.ink_width(l)).fold(0.0, f64::max);
        let step = (side + self.gap).max(widest + em);
        let mut x = self.x0;
        for (i, label) in labels.iter().enumerate() {
            let wide = side.max(self.ink_width(label));
            if x > self.x0 && x + wide > self.right {
                x = self.x0;
                self.y += row + self.gap;
            }
            draw(i, &mut self.paths, [x, self.y]);
            self.text(label, [x, self.y + side + em / 2.0]);
            self.reach = self.reach.max(x + side);
            x += step;
        }
        self.y += row + self.gap;
    }
}

/// Horizontal lines `s` apart across a patch of side `side` whose top-left corner is `at`, the
/// first `s / 2` below the top, alternating direction, all drawn `passes` times.
fn lines(out: &mut Paths, at: [f64; 2], side: f64, s: f64, passes: u32) {
    for _ in 0..passes {
        let mut k = 0;
        while s * (f64::from(k) + 0.5) <= side {
            let y = at[1] + s * (f64::from(k) + 0.5);
            let (a, b) = ([at[0], y], [at[0] + side, y]);
            out.push(&if k % 2 == 0 { [a, b] } else { [b, a] });
            k += 1;
        }
    }
}

/// A grid of dots of diameter `d` drawn with a pen `w` wide, centred in a patch of side `side`
/// whose top-left corner is `at`. A dot wider than the pen is a spiral, which fills it solid, so
/// the pitch leaves a pen width between neighbours at least.
fn dots(out: &mut Paths, at: [f64; 2], side: f64, d: f64, w: f64) {
    let pitch = (2.0 * d).max(d + w);
    let n = ((side / pitch).floor() as usize).max(1);
    let first = (side - (n - 1) as f64 * pitch) / 2.0;
    for row in 0..n {
        for col in 0..n {
            let centre = [
                at[0] + first + col as f64 * pitch,
                at[1] + first + row as f64 * pitch,
            ];
            out.push(&dot_path(centre, d, w, DotShape::Auto));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Fit;
    use std::collections::HashMap;

    fn canvas(w: f64, h: f64) -> Canvas {
        Canvas::new(w, h, 10.0, Fit::Fit).unwrap()
    }
    fn pen(w: f64) -> Pen {
        Pen::new("test", w, "#000000").unwrap()
    }
    fn paths_of(d: &Drawing) -> &Paths {
        assert_eq!(d.layers.len(), 1);
        assert!(d.tone.is_none());
        &d.layers[0].paths
    }

    #[test]
    fn it_fits_the_pages_it_is_meant_for() {
        for (page, w) in [
            (canvas(210.0, 297.0), 0.3),
            (canvas(148.0, 210.0), 0.3),
            (canvas(210.0, 297.0), 1.0),
        ] {
            let d = chart(&page, &pen(w)).unwrap();
            let [x, y, aw, ah] = page.drawing_rect();
            let tol = 1e-9;
            for &[vx, vy] in paths_of(&d).coords() {
                assert!(vx >= x - tol && vx <= x + aw + tol, "x {vx} on {page:?}");
                assert!(vy >= y - tol && vy <= y + ah + tol, "y {vy} on {page:?}");
            }
        }
    }
    #[test]
    fn a_page_too_small_is_refused_with_both_heights() {
        // An A7 page with a 10 mm margin leaves 54 x 85 mm.
        let e = chart(&canvas(74.0, 105.0), &pen(2.0)).unwrap_err();
        assert!(
            e.starts_with("the page is too small for the test chart: it needs ")
                && e.ends_with(" mm of the drawing area's height and has 85 mm"),
            "{e}"
        );
        // A page tall enough but too narrow says so about the width.
        let e = chart(&canvas(30.0, 1000.0), &pen(0.3)).unwrap_err();
        assert!(e.contains("width and has 10 mm"), "{e}");
        let e = chart(&canvas(210.0, 297.0), &pen(0.01)).unwrap_err();
        assert!(e.contains("too thin"), "{e}");
    }
    #[test]
    fn the_same_inputs_give_the_same_paths() {
        let a = chart(&canvas(210.0, 297.0), &pen(0.3)).unwrap();
        let b = chart(&canvas(210.0, 297.0), &pen(0.3)).unwrap();
        assert_eq!(paths_of(&a), paths_of(&b));
    }
    #[test]
    fn a_pass_patch_is_drawn_its_number_of_times() {
        let d = chart(&canvas(210.0, 297.0), &pen(0.3)).unwrap();
        // The spacing section draws each full-width line once, so a line drawn n times belongs to
        // a patch of n passes. Lines are keyed by their left end, as the bits of the coordinates.
        let mut seen: HashMap<[u64; 2], usize> = HashMap::new();
        for p in paths_of(&d).iter().filter(|p| p.len() == 2) {
            let ([ax, ay], [bx, by]) = (p[0], p[1]);
            if ay == by && ((ax - bx).abs() - 10.0).abs() < 1e-9 {
                *seen
                    .entry([ax.min(bx).to_bits(), ay.to_bits()])
                    .or_default() += 1;
            }
        }
        let mut patches: HashMap<usize, usize> = HashMap::new();
        for &n in seen.values() {
            *patches.entry(n).or_default() += 1;
        }
        // 0.3 mm lines fit 33 times in 10 mm.
        for n in [2, 3, 4, 6, 8] {
            assert_eq!(patches[&n], 33, "{n} passes");
        }
        assert!(patches.keys().all(|n| [1, 2, 3, 4, 6, 8].contains(n)));
    }
    #[test]
    fn wide_labels_push_the_patches_apart() {
        let narrow = chart(&canvas(210.0, 297.0), &pen(0.3)).unwrap();
        let wide = chart(&canvas(210.0, 297.0), &pen(1.0)).unwrap();
        // The patch is 10 mm either way, but at 1 mm the label "0.50" is 16 mm wide. The lefts of
        // the first section's patches are the lines of its top row.
        let lefts = |d: &Drawing| -> Vec<f64> {
            let lines: Vec<&[[f64; 2]]> = paths_of(d)
                .iter()
                .filter(|p| p.len() == 2 && p[0][1] == p[1][1] && (p[1][0] - p[0][0]).abs() > 9.9)
                .collect();
            let top = lines.iter().map(|p| p[0][1]).fold(f64::INFINITY, f64::min);
            let mut xs: Vec<f64> = (lines.iter())
                .filter(|p| p[0][1] < top + 10.0)
                .map(|p| p[0][0].min(p[1][0]))
                .collect();
            xs.sort_by(f64::total_cmp);
            xs.dedup();
            xs
        };
        let (narrow, wide) = (lefts(&narrow), lefts(&wide));
        assert_eq!(narrow.len(), 8);
        assert!((narrow[1] - narrow[0] - 15.0).abs() < 1e-9, "{narrow:?}");
        assert!(wide[1] - wide[0] > 20.0, "{wide:?}");
    }
}
