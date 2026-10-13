//! The Hilbert curve stage: a space-filling curve that folds tighter where the image is darker.
//!
//! This is an adaptive Hilbert curve after L. Velho and J. de M. Gomes, "Digital halftoning with
//! space filling curves", *Proc. SIGGRAPH*, 1991, pp. 81-90. Their patent is listed as expired in
//! the patent record (US5602943). The curve is drawn over the smallest square of side
//! `cell_mm * 2^k` that covers the tone map, anchored at its top-left corner. A cell splits into
//! four, in Hilbert order and orientation, while it is wider than `max_cell_mm`, or while it is
//! wider than both `cell_mm` and `cell_mm / max(ink, 1/64)`, where `ink` is the mean over the cell.
//! The line joins the centres of the leaf cells in curve order. A curve through cells of side `s`
//! lays down about `s` of line per `s^2` of area, so the line density follows the ink.
//!
//! The recursion is the classic vector form, in which a cell is an origin and two side vectors and
//! its four children are the origin and the vectors halved, transposed or reflected: H. Sagan,
//! *Space-Filling Curves*, Springer, 1994. Here it runs on an explicit stack. Cell sizes are
//! `cell_mm` times a power of two, so halving is exact. The mean ink of a cell comes from a
//! summed-area table in `f64` over the pixels whose centres lie in the cell, each counted whole;
//! a cell with none, or entirely off the map, has ink 0. A leaf whose centre lies outside the map
//! ends the current path, and a cell entirely outside the map ends it without being split, since
//! all its leaves would.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{inside, ChunkRun};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};
use crate::types::{Paths, ToneMap};

/// The least ink a cell is taken to hold when deciding its size, which caps the size over white
/// at 64 times `cell_mm`.
const LEAST_INK: f64 = 1.0 / 64.0;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the Hilbert curve stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HilbertSettings {
    /// Smallest fold, used over full ink.
    #[schemars(range(min = 0.3, max = 5.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub cell_mm: f64,
    /// Largest fold, used over white.
    #[schemars(range(min = 1.0, max = 40.0), extend("x-step" = 0.5, "x-unit" = "mm"))]
    pub max_cell_mm: f64,
}

impl Default for HilbertSettings {
    fn default() -> Self {
        HilbertSettings {
            cell_mm: 0.6,
            max_cell_mm: 8.0,
        }
    }
}

/// Draw one adaptive Hilbert curve over the tone map.
///
/// A largest fold below the smallest is raised to it. The line is one path wherever the map
/// covers the square, and breaks where the curve leaves the map. A step draws one of the square's
/// four quadrants in curve order, or the whole curve when the square is one cell, and reports a
/// frame holding the paths so far.
pub struct HilbertStage;

impl Stage for HilbertStage {
    fn name(&self) -> &'static str {
        "hilbert"
    }
    fn label(&self) -> &'static str {
        "Hilbert curve"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<HilbertSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<HilbertSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<HilbertSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let sums = Sums::new(&tone);
        let (cell, max) = (s.cell_mm, s.max_cell_mm.max(s.cell_mm));
        // Doubling a float is exact, so the side is the same on every platform.
        let (mut side, mut k) = (cell, 0);
        while side < rect[2].max(rect[3]) {
            side *= 2.0;
            k += 1;
        }
        let root = Cell {
            o: [rect[0], rect[1]],
            i: [side, 0.0],
            j: [0.0, side],
            side,
        };
        let jobs: Vec<Cell> = if k == 0 {
            vec![root]
        } else {
            root.children().to_vec()
        };
        Ok(Box::new(ChunkRun::new(
            jobs.len(),
            "quadrants",
            Box::new(move |n, paths, open| {
                let end = |paths: &mut Paths, open: &mut Vec<[f64; 2]>| {
                    if open.len() >= 2 {
                        paths.push(open);
                    }
                    open.clear();
                };
                let mut stack = vec![jobs[n]];
                while let Some(c) = stack.pop() {
                    let [x0, x1, y0, y1] = c.bounds();
                    if x1 <= rect[0]
                        || x0 >= rect[0] + rect[2]
                        || y1 <= rect[1]
                        || y0 >= rect[1] + rect[3]
                    {
                        end(paths, open);
                    } else if c.side > max
                        || (c.side > cell
                            && c.side > cell / sums.mean(&tone, [x0, x1, y0, y1]).max(LEAST_INK))
                    {
                        stack.extend(c.children().into_iter().rev());
                    } else {
                        let centre = c.centre();
                        if inside(rect, centre) {
                            open.push(centre);
                        } else {
                            end(paths, open);
                        }
                    }
                }
            }),
        )))
    }
}

/// A square cell: the corner `o` and the side vectors `i` and `j`, each of length `side` and along
/// an axis, so the cell is `o + a * i + b * j` for `a` and `b` from 0 to 1.
#[derive(Clone, Copy)]
struct Cell {
    o: [f64; 2],
    i: [f64; 2],
    j: [f64; 2],
    side: f64,
}

impl Cell {
    /// The four quadrants in Hilbert order: the first is transposed, the last transposed and
    /// reflected.
    fn children(&self) -> [Cell; 4] {
        let half = |v: [f64; 2]| [v[0] / 2.0, v[1] / 2.0];
        let add = |a: [f64; 2], b: [f64; 2]| [a[0] + b[0], a[1] + b[1]];
        let neg = |v: [f64; 2]| [-v[0], -v[1]];
        let (hi, hj) = (half(self.i), half(self.j));
        let (o, side) = (self.o, self.side / 2.0);
        [
            Cell {
                o,
                i: hj,
                j: hi,
                side,
            },
            Cell {
                o: add(o, hi),
                i: hi,
                j: hj,
                side,
            },
            Cell {
                o: add(add(o, hi), hj),
                i: hi,
                j: hj,
                side,
            },
            Cell {
                o: add(add(o, hi), self.j),
                i: neg(hj),
                j: neg(hi),
                side,
            },
        ]
    }

    /// The cell's middle, in mm.
    fn centre(&self) -> [f64; 2] {
        [
            self.o[0] + (self.i[0] + self.j[0]) / 2.0,
            self.o[1] + (self.i[1] + self.j[1]) / 2.0,
        ]
    }

    /// The cell's extent in mm: `[x0, x1, y0, y1]`.
    fn bounds(&self) -> [f64; 4] {
        let e = [
            self.o[0] + self.i[0] + self.j[0],
            self.o[1] + self.i[1] + self.j[1],
        ];
        [
            self.o[0].min(e[0]),
            self.o[0].max(e[0]),
            self.o[1].min(e[1]),
            self.o[1].max(e[1]),
        ]
    }
}

/// A summed-area table of the tone map's ink in `f64`: entry `(x, y)` is the sum over the pixels
/// left of column `x` and above row `y`. Each row is summed along itself first, in order, so a
/// flat map gives exact sums.
struct Sums {
    width: usize,
    height: usize,
    at: Vec<f64>,
}

impl Sums {
    fn new(tone: &ToneMap) -> Sums {
        let (w, h) = (tone.width as usize, tone.height as usize);
        let mut at = vec![0.0; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0.0;
            for x in 0..w {
                row += f64::from(tone.ink[y * w + x]);
                at[(y + 1) * (w + 1) + x + 1] = at[y * (w + 1) + x + 1] + row;
            }
        }
        Sums {
            width: w,
            height: h,
            at,
        }
    }

    /// The mean ink over the pixels whose centres lie in the box `[x0, x1) x [y0, y1)` (mm), each
    /// counted whole; 0 when there are none.
    fn mean(&self, tone: &ToneMap, [x0, x1, y0, y1]: [f64; 4]) -> f64 {
        let [px0, py0] = tone.to_px([x0, y0]);
        let [px1, py1] = tone.to_px([x1, y1]);
        // Pixel `c` has its centre at `c + 0.5`, so the pixels in `[a, b)` are the columns from
        // `ceil(a - 0.5)` up to `ceil(b - 0.5)`, and neighbouring cells share none.
        let span = |a: f64, b: f64, n: usize| {
            let at = |v: f64| ((v - 0.5).ceil().max(0.0) as usize).min(n);
            (at(a), at(b))
        };
        let ((lx, hx), (ly, hy)) = (span(px0, px1, self.width), span(py0, py1, self.height));
        if lx >= hx || ly >= hy {
            return 0.0;
        }
        let w = self.width + 1;
        let sum = self.at[hy * w + hx] - self.at[ly * w + hx] - self.at[hy * w + lx]
            + self.at[ly * w + lx];
        sum / ((hx - lx) * (hy - ly)) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math;
    use crate::stage::Kind;
    use crate::test_util::{assert_inside, bounds, run_tone_stage, tone_map};
    use crate::types::ToneMap;
    use serde_json::json;
    use std::sync::Arc;

    /// The lengths of a path's segments.
    fn steps(path: &[[f64; 2]]) -> Vec<f64> {
        path.windows(2)
            .map(|w| math::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .collect()
    }

    #[test]
    fn settings_schema() {
        let s = HilbertStage.schema();
        assert_eq!(bounds(&s, "cell_mm"), (0.3, 5.0, 0.6));
        assert_eq!(bounds(&s, "max_cell_mm"), (1.0, 40.0, 8.0));
        assert_eq!(
            (HilbertStage.name(), HilbertStage.label()),
            ("hilbert", "Hilbert curve")
        );
        assert_eq!(HilbertStage.version(), 1);
        assert_eq!(
            (HilbertStage.requires(), HilbertStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn white_folds_only_to_the_largest_cell() {
        // 64 mm square: 8 by 8 leaves of 8 mm. The curve starts in the top-left leaf and ends in
        // the bottom-left one.
        let (_, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(32, 32, |_, _| 0.0),
            json!({"cell_mm": 1.0, "max_cell_mm": 8.0}),
        );
        assert_eq!(paths.len(), 1);
        let path = paths.get(0);
        assert_eq!(path.len(), 64);
        assert_eq!((path[0], path[63]), ([14.0, 14.0], [14.0, 70.0]));
        for s in steps(path) {
            assert!((s - 8.0).abs() < 1e-9, "{s}");
        }
    }

    #[test]
    fn the_largest_fold_sets_the_cells_over_white() {
        let white = |patch| {
            let (_, paths) = run_tone_stage(&HilbertStage, tone_map(32, 32, |_, _| 0.0), patch);
            paths.coords().len()
        };
        assert_eq!(white(json!({"cell_mm": 1.0, "max_cell_mm": 16.0})), 16);
        assert_eq!(white(json!({"cell_mm": 1.0, "max_cell_mm": 8.0})), 64);
        // A largest fold below the smallest is raised to it, so no cell is smaller than that.
        assert_eq!(white(json!({"cell_mm": 2.0, "max_cell_mm": 1.0})), 1024);
    }

    #[test]
    fn black_folds_to_the_smallest_cell() {
        let (_, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(32, 32, |_, _| 1.0),
            json!({"cell_mm": 2.0}),
        );
        assert_eq!(paths.len(), 1);
        let path = paths.get(0);
        assert_eq!(path.len(), 1024);
        for s in steps(path) {
            assert!((s - 2.0).abs() < 1e-9, "{s}");
        }
    }

    #[test]
    fn the_curve_is_one_path_on_a_covered_square() {
        let ink = |x: u32, y: u32| ((x * 7 + y * 13) % 10) as f32 / 9.0;
        let tone = tone_map(32, 32, ink);
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&HilbertStage, tone, json!({"cell_mm": 1.0}));
        assert_eq!(paths.len(), 1);
        assert_inside(&paths, rect);
        // Dark and light cells fold to different sizes, so the segments differ in length.
        let lengths = steps(paths.get(0));
        let (lo, hi) = lengths
            .iter()
            .fold((f64::MAX, 0.0f64), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        assert!(hi > 1.5 * lo, "{lo} {hi}");
    }

    #[test]
    fn dark_folds_tighter_than_light() {
        // A 128 by 64 mm map, black on the left half and white on the right.
        let (_, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(64, 32, |x, _| if x < 32 { 1.0 } else { 0.0 }),
            json!({"cell_mm": 1.0}),
        );
        let (dark, light) =
            paths.coords().iter().fold(
                (0, 0),
                |(d, l), p| if p[0] < 74.0 { (d + 1, l) } else { (d, l + 1) },
            );
        assert!(dark > 10 * light, "{dark} {light}");
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        // 60 by 40 mm: the square that covers it is 76.8 mm, so the curve leaves the map and
        // comes back, and each visit is its own path.
        let tone = tone_map(30, 20, |x, y| (x + y) as f32 / 48.0);
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&HilbertStage, tone, json!({}));
        assert!(paths.len() > 1, "{}", paths.len());
        assert!(paths.iter().all(|p| p.len() >= 2));
        assert_inside(&paths, rect);
    }

    #[test]
    fn one_frame_per_top_level_quadrant() {
        let (frames, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(32, 32, |_, _| 0.5),
            json!({"cell_mm": 1.0}),
        );
        let counts: Vec<usize> = frames
            .iter()
            .map(|f| f.paths.as_ref().unwrap().coords().len())
            .collect();
        assert_eq!(counts.len(), 4);
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert_eq!(counts[3], paths.coords().len());
    }

    #[test]
    fn a_root_that_is_one_leaf_gives_one_frame() {
        // A 2 mm square under a 5 mm cell: the root is one leaf, centred outside the map.
        let (frames, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(1, 1, |_, _| 1.0),
            json!({"cell_mm": 5.0}),
        );
        assert_eq!(frames.len(), 1);
        assert!(paths.is_empty());
    }

    #[test]
    fn a_black_a4_page_gives_about_150000_vertices() {
        // The drawing area of A4 with a 10 mm margin, tone-mapped at 1000 px on its long side.
        let (w, h, mm) = (686u32, 1000u32, 0.277);
        let tone = ToneMap {
            width: w,
            height: h,
            ink: vec![1.0; (w * h) as usize],
            mm_per_px: mm,
            origin_mm: [10.0, 10.0],
        };
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&HilbertStage, tone, json!({}));
        assert_inside(&paths, rect);
        let n = paths.coords().len();
        assert!((140_000..155_000).contains(&n), "{n}");
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"cell_mm": 0.2}),
            json!({"max_cell_mm": 41}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&HilbertStage.defaults(), &patch);
            let tone = crate::stage::Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
            let ctx = RunContext::new(Arc::default());
            let err = HilbertStage.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
