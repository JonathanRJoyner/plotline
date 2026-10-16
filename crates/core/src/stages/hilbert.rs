//! The Hilbert curve stage: a space-filling curve that folds tighter where the image is darker.
//!
//! This is an adaptive Hilbert curve after L. Velho and J. de M. Gomes, "Digital halftoning with
//! space filling curves", *Proc. SIGGRAPH*, 1991, pp. 81-90. Their patent is listed as expired in
//! the patent record (US5602943). The tone map is cut into cells of `cell_mm`, anchored at its
//! top-left corner: `ceil(width / cell_mm)` columns by `ceil(height / cell_mm)` rows, the last
//! column and row overhanging the map by less than a cell. The curve through the grid is the
//! generalized Hilbert curve of J. Červený, "gilbert: generalized Hilbert space-filling curve for
//! rectangular domains of arbitrary sizes", <https://github.com/jakubcerveny/gilbert>
//! (BSD-2-Clause), written here from the description in its README. It cuts a block of cells into
//! two or three smaller blocks in curve order until each is one cell wide or high, which it walks
//! cell by cell; a power of two on both sides gives the classic curve. A block splits while it is
//! wider than `max_cell_mm`, or while it is wider than both `cell_mm` and `cell_mm / max(ink,
//! 1/64)`, where its width is its longer side and `ink` its mean over the map; a block of one cell
//! never splits, and a block that does not split is a leaf. The line joins the centres of the
//! leaves in curve order. A curve through blocks of side `s` lays down about `s` of line per `s^2` of
//! area, so the line density follows the ink.
//!
//! The block is the vector form of H. Sagan, *Space-Filling Curves*, Springer, 1994: a corner
//! cell and two axis vectors, in whole cells, so there is no rounding and the curve is the same on
//! every platform. Here the recursion runs on an explicit stack. The mean ink of a block comes
//! from a summed-area table in `f64` over the pixels whose centres lie in the block, each counted
//! whole, and along an axis on which a block thinner than a pixel holds no pixel centre, the pixel
//! under its middle counts. The curve visits every cell once in unit steps. When the
//! longer side has an odd number of cells and the shorter an even number, one diagonal step may be
//! unavoidable. A leaf centre off the map, in the overhanging column or row or the strip a lane's
//! move uncovers, is clamped onto the map's edge, so the curve is one path with one pen. When the
//! drawing is split between pens, a leaf over almost no ink in its lane ends the path.
//!
//! Split between pens, pen `i` of `n` also moves the grid's corner up and left by `cell_mm * i / n`
//! on both axes, and grows the grid until it covers the map from there. The leaves over full ink
//! are `cell_mm` wide, so two pens' finest folds interleave, as the grid placer's dots do, and the
//! coarser folds are offset by the same distance. Two lattices of one period and angle superpose
//! with no moiré whatever their offset (I. Amidror, *The Theory of the Moiré Phenomenon*, 2nd ed.,
//! Springer, 2009).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, ChunkRun, LANE_MIN_INK};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};
use crate::types::{Paths, ToneMap};

/// The least ink a block is taken to hold when deciding its size, which caps the size over white
/// at 64 times `cell_mm`.
const LEAST_INK: f64 = 1.0 / 64.0;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the Hilbert curve stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HilbertSettings {
    /// Smallest fold, used over full ink.
    #[schemars(range(min = 0.3, max = 5.0), extend("x-density" = "inverse", "x-step" = 0.1, "x-unit" = "mm"))]
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
/// A largest fold below the smallest is raised to it. The curve runs through a grid of `cell_mm`
/// cells that covers the map and is one path with one pen. The root block is split by the same
/// rule as inside a block, level by level, until there are about 30 blocks; a step draws the next
/// share of them in curve order and reports a frame holding the paths so far.
pub struct HilbertStage;

impl Stage for HilbertStage {
    fn name(&self) -> &'static str {
        "hilbert"
    }
    fn label(&self) -> &'static str {
        "Hilbert curve"
    }
    fn version(&self) -> u32 {
        3
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
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<HilbertSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let lane = ctx.lane();
        let lift = lane.count > 1;
        let rect = tone.rect_mm();
        let (cell, max) = (s.cell_mm, s.max_cell_mm.max(s.cell_mm));
        // Each pen's grid moves up and left by its share of a cell; the first pen's stays put.
        let d = cell * lane.index as f64 / lane.count as f64;
        // The tolerance keeps float noise, as in 6.0 / 0.6 = 10.000000000000002, from adding a
        // column wholly off the map.
        let count = |len: f64| ((len + d) / cell - 1e-9).ceil().max(1.0) as i64;
        let (w, h) = (count(rect[2]), count(rect[3]));
        let root = Block::root(w, h);
        let grid = Grid {
            origin: [rect[0] - d, rect[1] - d],
            cell,
            max,
            sums: Sums::new(&tone),
            tone,
        };
        // Split level by level, in curve order, for the frames; the rule is the same inside a job,
        // so the drawing does not depend on the chunking.
        let mut jobs = vec![root];
        while jobs.len() < 30 {
            let next: Vec<Block> = jobs
                .iter()
                .flat_map(|b| {
                    if grid.splits(b) {
                        b.children()
                    } else {
                        vec![*b]
                    }
                })
                .collect();
            if next.len() == jobs.len() {
                break;
            }
            jobs = next;
        }
        Ok(Box::new(ChunkRun::new(
            jobs.len(),
            "blocks",
            Box::new(move |n, paths, open| {
                let end = |paths: &mut Paths, open: &mut Vec<[f64; 2]>| {
                    if open.len() >= 2 {
                        paths.push(open);
                    }
                    open.clear();
                };
                let mut stack = vec![jobs[n]];
                while let Some(b) = stack.pop() {
                    if grid.splits(&b) {
                        stack.extend(b.children().into_iter().rev());
                    } else {
                        let centre = clamp_to(rect, grid.centre(&b));
                        // Split between pens, a lane lifts the pen over no ink.
                        if lift && grid.tone.sample(centre) < LANE_MIN_INK {
                            end(paths, open);
                        } else {
                            open.push(centre);
                        }
                    }
                }
            }),
        )))
    }
}

/// A block of grid cells: the corner cell `(x, y)` and the vectors `a`, along the curve's travel,
/// and `b`, across it, each along an axis and signed, in cells. With `da` and `db` the unit
/// vectors of `a` and `b`, the block is the cells `(x, y) + i * da + j * db` for `i` below `|a|`
/// and `j` below `|b|`.
#[derive(Clone, Copy)]
struct Block {
    x: i64,
    y: i64,
    a: [i64; 2],
    b: [i64; 2],
}

/// The unit vector along `v`.
fn unit(v: [i64; 2]) -> [i64; 2] {
    [v[0].signum(), v[1].signum()]
}

impl Block {
    /// The whole `w` by `h` grid, run along its longer side.
    fn root(w: i64, h: i64) -> Block {
        let (a, b) = if w >= h {
            ([w, 0], [0, h])
        } else {
            ([0, h], [w, 0])
        };
        Block { x: 0, y: 0, a, b }
    }

    /// The lengths of `a` and `b` in cells.
    fn size(&self) -> (i64, i64) {
        ((self.a[0] + self.a[1]).abs(), (self.b[0] + self.b[1]).abs())
    }

    /// The cells' extent: `[x0, x1, y0, y1]` with the ends exclusive.
    fn span(&self) -> [i64; 4] {
        let ((w, h), (da, db)) = (self.size(), (unit(self.a), unit(self.b)));
        let ex = self.x + (w - 1) * da[0] + (h - 1) * db[0];
        let ey = self.y + (w - 1) * da[1] + (h - 1) * db[1];
        [
            self.x.min(ex),
            self.x.max(ex) + 1,
            self.y.min(ey),
            self.y.max(ey) + 1,
        ]
    }

    /// The smaller blocks in curve order: the cells of a block one cell wide or high, two blocks
    /// across `a` for a long block, and otherwise three, the first and last transposed.
    fn children(&self) -> Vec<Block> {
        let ((w, h), (da, db)) = (self.size(), (unit(self.a), unit(self.b)));
        let (o, a, b) = ((self.x, self.y), self.a, self.b);
        let cell = |i: i64, v: [i64; 2]| Block {
            x: o.0 + i * v[0],
            y: o.1 + i * v[1],
            a: da,
            b: db,
        };
        if h == 1 {
            return (0..w).map(|i| cell(i, da)).collect();
        }
        if w == 1 {
            return (0..h).map(|i| cell(i, db)).collect();
        }
        let half = |v: [i64; 2]| [v[0] / 2, v[1] / 2];
        let add = |p: [i64; 2], q: [i64; 2]| [p[0] + q[0], p[1] + q[1]];
        let diff = |p: [i64; 2], q: [i64; 2]| [p[0] - q[0], p[1] - q[1]];
        let (mut a2, mut b2) = (half(a), half(b));
        let (w2, h2) = ((a2[0] + a2[1]).abs(), (b2[0] + b2[1]).abs());
        if 2 * w > 3 * h {
            // Cut at an even length where there is one to give the curve the right ends.
            if w2 % 2 == 1 && w > 2 {
                a2 = add(a2, da);
            }
            vec![
                Block {
                    x: o.0,
                    y: o.1,
                    a: a2,
                    b,
                },
                Block {
                    x: o.0 + a2[0],
                    y: o.1 + a2[1],
                    a: diff(a, a2),
                    b,
                },
            ]
        } else {
            if h2 % 2 == 1 && h > 2 {
                b2 = add(b2, db);
            }
            let back = diff(add(a, b2), add(da, db));
            vec![
                Block {
                    x: o.0,
                    y: o.1,
                    a: b2,
                    b: a2,
                },
                Block {
                    x: o.0 + b2[0],
                    y: o.1 + b2[1],
                    a,
                    b: diff(b, b2),
                },
                Block {
                    x: o.0 + back[0],
                    y: o.1 + back[1],
                    a: [-b2[0], -b2[1]],
                    b: [a2[0] - a[0], a2[1] - a[1]],
                },
            ]
        }
    }
}

/// The grid over the map: where it starts in mm, the cell size and the largest fold, and the tone
/// map with its summed-area table, which decide whether a block splits.
struct Grid {
    origin: [f64; 2],
    cell: f64,
    max: f64,
    tone: Arc<ToneMap>,
    sums: Sums,
}

impl Grid {
    /// The block's extent in mm: `[x0, x1, y0, y1]`.
    fn bounds(&self, b: &Block) -> [f64; 4] {
        let [x0, x1, y0, y1] = b.span();
        let at = |c: i64, axis: usize| self.origin[axis] + c as f64 * self.cell;
        [at(x0, 0), at(x1, 0), at(y0, 1), at(y1, 1)]
    }

    /// The block's middle, in mm.
    fn centre(&self, b: &Block) -> [f64; 2] {
        let [x0, x1, y0, y1] = b.span();
        [
            self.origin[0] + (x0 + x1) as f64 * self.cell / 2.0,
            self.origin[1] + (y0 + y1) as f64 * self.cell / 2.0,
        ]
    }

    /// Whether the block is cut into smaller ones: it is more than one cell and wider than the
    /// largest fold or than the fold its ink asks for.
    fn splits(&self, b: &Block) -> bool {
        let [x0, x1, y0, y1] = b.span();
        let long = (x1 - x0).max(y1 - y0) as f64 * self.cell;
        (x1 - x0) * (y1 - y0) > 1
            && (long > self.max
                || (long > self.cell
                    && long
                        > self.cell / self.sums.mean(&self.tone, self.bounds(b)).max(LEAST_INK)))
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
    /// counted whole. Along an axis on which the box holds no pixel centre, the pixel under the
    /// box's middle counts instead.
    fn mean(&self, tone: &ToneMap, [x0, x1, y0, y1]: [f64; 4]) -> f64 {
        let [px0, py0] = tone.to_px([x0, y0]);
        let [px1, py1] = tone.to_px([x1, y1]);
        // Pixel `c` has its centre at `c + 0.5`, so the pixels in `[a, b)` are the columns from
        // `ceil(a - 0.5)` up to `ceil(b - 0.5)`, and neighbouring cells share none.
        let span = |a: f64, b: f64, n: usize| {
            let at = |v: f64| ((v - 0.5).ceil().max(0.0) as usize).min(n);
            if at(a) < at(b) {
                return (at(a), at(b));
            }
            // A box thinner than a pixel holds no centre on this axis: take the pixel under its middle.
            let c = (((a + b) / 2.0).floor().max(0.0) as usize).min(n - 1);
            (c, c + 1)
        };
        let ((lx, hx), (ly, hy)) = (span(px0, px1, self.width), span(py0, py1, self.height));
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
    use crate::stage::{Kind, Lane};
    use crate::test_util::{
        assert_inside, bounds, run_tone_stage, run_tone_stage_in_lane, tone_map,
    };
    use crate::types::ToneMap;
    use serde_json::json;

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
        assert_eq!(HilbertStage.version(), 3);
        assert_eq!(
            (HilbertStage.requires(), HilbertStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn white_folds_only_to_the_largest_cell() {
        // 64 mm square: 8 by 8 leaves of 8 mm. The root runs along x, so the curve starts in the
        // top-left leaf and ends in the top-right one.
        let (_, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(32, 32, |_, _| 0.0),
            json!({"cell_mm": 1.0, "max_cell_mm": 8.0}),
        );
        assert_eq!(paths.len(), 1);
        let path = paths.get(0);
        assert_eq!(path.len(), 64);
        assert_eq!((path[0], path[63]), ([14.0, 14.0], [70.0, 14.0]));
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
    fn a_second_pen_folds_between_the_first_pens_cells() {
        let run = |lane| {
            let tone = tone_map(32, 32, |_, _| 1.0);
            run_tone_stage_in_lane(&HilbertStage, tone, json!({"cell_mm": 2.0}), lane).1
        };
        let single = run(Lane::SINGLE);
        assert_eq!(run(Lane { index: 0, count: 2 }).coords(), single.coords());
        // Pen 0's leaves are centred on odd mm, 11 to 73. Pen 1's grid starts 1 mm up and left,
        // so its leaves are centred on even mm, 10 to 74, the map's edges included.
        let second = run(Lane { index: 1, count: 2 });
        assert!(single.coords().iter().flatten().all(|v| v % 2.0 == 1.0));
        assert_eq!(second.coords().len(), 33 * 33);
        assert!(second.coords().iter().flatten().all(|v| v % 2.0 == 0.0));
        assert_inside(&second, [10.0, 10.0, 64.0, 64.0]);
    }

    #[test]
    fn a_pen_over_half_the_map_draws_one_path_over_that_half() {
        // 128 by 64 mm of 2 mm pixels, black on the left half, which is the root's first block.
        // Cells of 1 mm make blocks thinner than a pixel, which still read the ink under them.
        let tone = tone_map(64, 32, |x, _| if x < 32 { 1.0 } else { 0.0 });
        let patch = json!({"cell_mm": 1.0});
        let lane = Lane { index: 0, count: 2 };
        let (_, paths) = run_tone_stage_in_lane(&HilbertStage, tone, patch, lane);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths.coords().len(), 64 * 64);
        assert!(paths.coords().iter().all(|p| p[0] < 74.0));
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
        // 60 by 40 mm: the grid overhangs the map by under a cell on two sides, and the curve is
        // still one path, its edge centres clamped onto the map.
        let tone = tone_map(30, 20, |x, y| (x + y) as f32 / 48.0);
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&HilbertStage, tone, json!({}));
        assert_eq!(paths.len(), 1);
        assert_inside(&paths, rect);
    }

    #[test]
    fn frames_grow_to_the_drawing() {
        let (frames, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(32, 32, |_, _| 0.5),
            json!({"cell_mm": 1.0}),
        );
        let counts: Vec<usize> = frames
            .iter()
            .map(|f| f.paths.as_ref().unwrap().coords().len())
            .collect();
        assert!((2..=30).contains(&counts.len()), "{counts:?}");
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert_eq!(counts[counts.len() - 1], paths.coords().len());
    }

    #[test]
    fn a_root_that_is_one_leaf_gives_one_frame() {
        // A 2 mm square under a 5 mm cell: one cell, one vertex, which is not a path.
        let (frames, paths) = run_tone_stage(
            &HilbertStage,
            tone_map(1, 1, |_, _| 1.0),
            json!({"cell_mm": 5.0}),
        );
        assert_eq!(frames.len(), 1);
        assert!(paths.is_empty());
    }

    #[test]
    fn a_black_a4_page_gives_one_path_of_every_cell() {
        // The drawing area of A4 with a 10 mm margin, tone-mapped at 1000 px on its long side:
        // 190.02 by 277 mm, so 317 by 462 cells of 0.6 mm.
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
        assert_eq!(paths.len(), 1);
        assert_eq!(paths.coords().len(), 317 * 462);
        for s in steps(paths.get(0)) {
            assert!((s - 0.6).abs() < 1e-9, "{s}");
        }
    }

    /// The cells of a `w` by `h` grid in curve order, walked down to single cells.
    fn walk(w: i64, h: i64) -> Vec<[i64; 2]> {
        let root = Block::root(w, h);
        let (mut stack, mut cells) = (vec![root], Vec::new());
        while let Some(b) = stack.pop() {
            let [x0, x1, y0, y1] = b.span();
            if (x1 - x0) * (y1 - y0) == 1 {
                cells.push([x0, y0]);
            } else {
                stack.extend(b.children().into_iter().rev());
            }
        }
        cells
    }

    #[test]
    fn the_curve_visits_every_cell_once_in_unit_steps() {
        for w in 1..=40i64 {
            for h in 1..=40i64 {
                let cells = walk(w, h);
                let mut seen = vec![false; (w * h) as usize];
                for c in &cells {
                    assert!(
                        (0..w).contains(&c[0]) && (0..h).contains(&c[1]),
                        "{w}x{h} {c:?}"
                    );
                    let slot = &mut seen[(c[1] * w + c[0]) as usize];
                    assert!(!*slot, "{w}x{h}: {c:?} twice");
                    *slot = true;
                }
                assert_eq!(cells.len(), (w * h) as usize, "{w}x{h}");
                let diagonals = cells
                    .windows(2)
                    .filter(|p| {
                        let (dx, dy) = ((p[1][0] - p[0][0]).abs(), (p[1][1] - p[0][1]).abs());
                        assert!(dx <= 1 && dy <= 1 && dx + dy >= 1, "{w}x{h}: {p:?}");
                        dx + dy == 2
                    })
                    .count();
                let (long, short) = (w.max(h), w.min(h));
                let odd_even = long % 2 == 1 && short % 2 == 0;
                assert!(diagonals <= 1, "{w}x{h}: {diagonals} diagonals");
                assert!(diagonals == 0 || odd_even, "{w}x{h}: a diagonal");
                assert_eq!(cells[0], [0, 0], "{w}x{h}");
                if !odd_even {
                    let end = if w >= h { [w - 1, 0] } else { [0, h - 1] };
                    assert_eq!(cells[cells.len() - 1], end, "{w}x{h}");
                }
            }
        }
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
