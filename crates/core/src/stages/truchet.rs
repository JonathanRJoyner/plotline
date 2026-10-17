//! The truchet stage: quarter-circle arcs on square tiles in random turns, nested more deeply
//! where the image is darker, and joined from tile to tile into long curves.
//!
//! S. Truchet, "Mémoire sur les combinaisons", *Mémoires de l'Académie Royale des Sciences*, 1704,
//! pp. 363-372, laid square tiles split by a diagonal in different turns. C. S. Smith and
//! P. Boucher, "The tiling patterns of Sebastien Truchet and the topology of structural
//! hierarchy", *Leonardo* 20(4), 1987, pp. 373-385, end with the variant drawn here: each tile
//! holds two quarter-circle arcs of radius `s / 2`, centred on two opposite corners, that join
//! the midpoints of adjacent sides, so that tiles in random turns join their arcs into continuous
//! curves.
//!
//! Nesting several arcs about each corner, and choosing their number from the tone, is this
//! project's own rule. Arcs of radius `s / 2 + j g` about one corner, for `j` from `-m` to `m`,
//! meet each side `j g` either side of its midpoint, whatever the tile, so two neighbouring
//! tiles with the same count join all their arcs and, where the counts differ, the extra arcs
//! end at the side. The arcs are the edges of a graph on the points where they meet the sides;
//! its trails ([`trails`]) are the curves, each drawn as one path, so the pen lifts once per curve
//! rather than once per arc.
//!
//! A tile can hold only an odd number of arcs a corner, or none, so a tone between two counts is
//! rounded, and the rounding error is passed on to the tiles after it in row-major order with the
//! weights of R. W. Floyd and L. Steinberg, "An adaptive algorithm for spatial greyscale",
//! *Proc. SID* 17(2), 1976, pp. 75-77: 7/16 to the next tile in the row, and 3/16, 5/16 and 1/16
//! to the tiles below to the left, below and below to the right. The counts then average to the
//! tone over a stretch of tiles, as the dots of an error-diffused picture do, where plain
//! rounding would give a tone near a rounding point one count over a whole area. Every count
//! stays odd, or none, so the joins stay whole.
//!
//! When the drawing is split between pens, lane `i` of `n` turns its tile grid by `90 i / n`
//! degrees about the tone map's centre, so that the lanes' grids are spread evenly over the
//! quarter turn after which a square grid repeats, and draws its turns from its own random stream.

use std::f64::consts::{FRAC_PI_2, SQRT_2};
use std::sync::Arc;

use rand::Rng;
use rand_pcg::Pcg64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::graph::{clip_polyline, trails};
use super::lines::inside;
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, ToneMap};

/// Segments in a quarter arc, a 64th of a turn each: the cap on the stipple dots' arcs.
const ARC_SEGMENTS: usize = 16;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the truchet stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TruchetSettings {
    /// Side of one square tile.
    #[schemars(range(min = 2.0, max = 40.0), extend("x-step" = 0.5, "x-unit" = "mm"))]
    pub tile_mm: f64,
    /// Distance between nested arcs; closer arcs fit more into a tile and draw darker.
    #[schemars(range(min = 0.1, max = 5.0), extend("x-density" = "inverse", "x-step" = 0.05, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Turns of the tiles.
    #[schemars(range(min = 0, max = 9999))]
    pub seed: u32,
}

impl Default for TruchetSettings {
    fn default() -> Self {
        TruchetSettings {
            tile_mm: 6.0,
            spacing_mm: 0.4,
            seed: 0,
        }
    }
}

/// The most arcs either side of the middle one that a corner of a tile of side `side` can nest,
/// `spacing` apart, without meeting the arcs of the opposite corner: the outermost arcs of the
/// two corners then stay at least `spacing` apart along the tile's diagonal.
pub(crate) fn nest_limit(side: f64, spacing: f64) -> usize {
    (((SQRT_2 - 1.0) * side / spacing - 1.0) / 2.0)
        .floor()
        .max(0.0) as usize
}

/// `p` turned about `centre` by the angle whose cosine and sine are `turn`.
fn rotate(centre: [f64; 2], turn: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (p[0] - centre[0], p[1] - centre[1]);
    [
        centre[0] + dx * turn[0] - dy * turn[1],
        centre[1] + dx * turn[1] + dy * turn[0],
    ]
}

/// Quarter-circle arcs on square tiles, joined into curves.
///
/// The tiles cover the tone map's rect from its top-left corner, so the last column and row may
/// overhang it. Each tile takes its turn from the seeded generator, ink or not, so a change of
/// tone never reshuffles the turns; its ink is the mean of the tone at the centres of a 4 by 4
/// grid of sub-squares. With `t` that ink and `m_max` from [`nest_limit`], the tile wants
/// `n = t (2 m_max + 1)` arcs about each corner, plus the rounding error the tiles before it
/// passed on. It holds none when `n` is under a half, else the odd number nearest `n`, at most
/// `2 m_max + 1`, and passes `n` less that number on. A tile whose 16 samples all fall off the
/// map holds none, and neither takes nor passes on error. The curves are cut to the rect.
///
/// A step makes a chunk of rows of tiles, for about 30 frames that show the arcs so far, one path
/// each. The last step joins the arcs, and its frame and the artifact hold the curves.
pub struct TruchetStage;

impl Stage for TruchetStage {
    fn name(&self) -> &'static str {
        "truchet"
    }
    fn label(&self) -> &'static str {
        "Truchet tiles"
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
        schema_for::<TruchetSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<TruchetSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<TruchetSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let [x, y, w, h] = rect;
        let centre = [x + w / 2.0, y + h / 2.0];
        let lane = ctx.lane();
        // Lane 0 keeps the upright grid, so one pen draws as it would alone.
        let turn = (lane.index > 0).then(|| {
            let angle = FRAC_PI_2 * lane.index as f64 / lane.count as f64;
            [math::cos(angle), math::sin(angle)]
        });
        // The rect in the grid's own frame, where the tiles are upright, bounds the covering grid.
        let corners = [[x, y], [x + w, y], [x, y + h], [x + w, y + h]].map(|p| match turn {
            Some([c, s]) => rotate(centre, [c, -s], p),
            None => p,
        });
        // A hair of slack keeps a map a whole number of tiles across from gaining a column.
        let span = |axis: usize| {
            let (lo, hi) = (corners.iter())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p[axis]), hi.max(p[axis]))
                });
            let first = ((lo - rect[axis]) / s.tile_mm + 1e-9).floor();
            let end = ((hi - rect[axis]) / s.tile_mm - 1e-9).ceil();
            (first as i64, (end - first) as usize)
        };
        let ((col0, cols), (row0, rows)) = (span(0), span(1));
        let mut unit: [[f64; 2]; ARC_SEGMENTS + 1] = std::array::from_fn(|k| {
            let angle = FRAC_PI_2 * k as f64 / ARC_SEGMENTS as f64;
            [math::cos(angle), math::sin(angle)]
        });
        // The ends sit exactly on the tile's sides.
        unit[0] = [1.0, 0.0];
        unit[ARC_SEGMENTS] = [0.0, 1.0];
        Ok(Box::new(TruchetRun {
            tone,
            rect,
            centre,
            rng: ctx.rng(u64::from(s.seed)),
            side: s.tile_mm,
            spacing: s.spacing_mm,
            reach: nest_limit(s.tile_mm, s.spacing_mm),
            turn,
            first: [col0, row0],
            cols,
            rows,
            per_frame: rows.div_ceil(30).max(1),
            next_row: 0,
            unit,
            arcs: Paths::new(),
            ends: Vec::new(),
            joined: None,
            error: vec![0.0; (rows + 1) * (cols + 2)],
        }))
    }
}

struct TruchetRun {
    tone: Arc<ToneMap>,
    rect: [f64; 4],
    centre: [f64; 2],
    rng: Pcg64,
    /// The tile side and the distance between nested arcs, in mm.
    side: f64,
    spacing: f64,
    /// The most arcs either side of the middle one that a corner nests; see [`nest_limit`].
    reach: usize,
    /// Cosine and sine of the lane's turn about the rect's centre; none for the first lane.
    turn: Option<[f64; 2]>,
    /// The covering grid: its first column and row counted from the rect's top-left corner, and
    /// how many columns and rows it has.
    first: [i64; 2],
    cols: usize,
    rows: usize,
    per_frame: usize,
    next_row: usize,
    /// Cosine and sine of the angles of a quarter arc's vertices.
    unit: [[f64; 2]; ARC_SEGMENTS + 1],
    /// Every arc made so far, from its end on a horizontal side to its end on a vertical one.
    arcs: Paths,
    /// The ids of each arc's two ends, in the order of `arcs`.
    ends: Vec<(usize, usize)>,
    /// The finished curves, once joined.
    joined: Option<Arc<Paths>>,
    /// The rounding error passed on to each tile, row by row, with a spare column either side of
    /// each row and one spare row.
    error: Vec<f64>,
}

impl TruchetRun {
    /// `p`, a position in the grid's own frame, on the page.
    fn place(&self, p: [f64; 2]) -> [f64; 2] {
        match self.turn {
            Some(turn) => rotate(self.centre, turn, p),
            None => p,
        }
    }

    /// The id of the top side of the tile in column `c` and row `r` of the covering grid; row
    /// `rows` holds the bottom sides of the last row.
    fn horizontal(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    /// The id of the left side of the tile in column `c` and row `r`; column `cols` holds the
    /// right sides of the last column. Vertical sides are numbered after the horizontal ones.
    fn vertical(&self, c: usize, r: usize) -> usize {
        (self.rows + 1) * self.cols + r * (self.cols + 1) + c
    }

    /// The id of the point `offset` spacings from the middle of `side`, counted along it from its
    /// left or top end. Both tiles that share the side name the point alike.
    fn point(&self, side: usize, offset: i64) -> usize {
        side * (2 * self.reach + 1) + (offset + self.reach as i64) as usize
    }

    /// Makes the tile in column `c` and row `r`.
    fn tile(&mut self, c: usize, r: usize) {
        // Every tile draws its turn, so a change of tone never reshuffles them.
        let flip = self.rng.random::<bool>();
        let side = self.side;
        let x = self.rect[0] + (self.first[0] + c as i64) as f64 * side;
        let y = self.rect[1] + (self.first[1] + r as i64) as f64 * side;
        let (mut ink, mut on_map) = (0.0, false);
        for b in 0..4 {
            for a in 0..4 {
                let p = self.place([
                    x + (f64::from(a) + 0.5) * side / 4.0,
                    y + (f64::from(b) + 0.5) * side / 4.0,
                ]);
                on_map |= inside(self.rect, p);
                ink += f64::from(self.tone.sample(p));
            }
        }
        if !on_map {
            return;
        }
        let stride = self.cols + 2;
        let here = r * stride + c + 1;
        let n = ink / 16.0 * (2 * self.reach + 1) as f64 + self.error[here];
        // The odd number of arcs nearest n is 2 m + 1, and none is nearer when n is under a half.
        let nests = (n >= 0.5).then(|| ((n / 2.0).floor() as usize).min(self.reach));
        let rounding = n - nests.map_or(0.0, |m| (2 * m + 1) as f64);
        self.error[here + 1] += rounding * 7.0 / 16.0;
        self.error[here + stride - 1] += rounding * 3.0 / 16.0;
        self.error[here + stride] += rounding * 5.0 / 16.0;
        self.error[here + stride + 1] += rounding / 16.0;
        let Some(m) = nests else {
            return;
        };
        let m = m as i64;
        let corners: [[usize; 2]; 2] = if flip {
            [[1, 0], [0, 1]]
        } else {
            [[0, 0], [1, 1]]
        };
        for [kx, ky] in corners {
            // The steps from the corner along the tile's horizontal and vertical sides.
            let (sx, sy) = (1 - 2 * kx as i64, 1 - 2 * ky as i64);
            let corner = [x + kx as f64 * side, y + ky as f64 * side];
            let top = self.horizontal(c, r + ky);
            let left = self.vertical(c + kx, r);
            for j in -m..=m {
                let radius = side / 2.0 + j as f64 * self.spacing;
                let ends = (self.point(top, sx * j), self.point(left, sy * j));
                let arc: Vec<[f64; 2]> = (self.unit.iter())
                    .map(|&[cos, sin]| {
                        self.place([
                            corner[0] + radius * cos * sx as f64,
                            corner[1] + radius * sin * sy as f64,
                        ])
                    })
                    .collect();
                self.ends.push(ends);
                self.arcs.push(&arc);
            }
        }
    }

    /// The arcs joined into curves along their trails and cut to the rect.
    fn join(&self) -> Paths {
        let sides = (self.rows + 1) * self.cols + self.rows * (self.cols + 1);
        let curves = trails(sides * (2 * self.reach + 1), &self.ends);
        // No two arcs have the same ends, so the ends find the arc.
        let mut by_ends: Vec<((usize, usize), usize)> = (self.ends.iter().enumerate())
            .map(|(i, &(a, b))| ((a.min(b), a.max(b)), i))
            .collect();
        by_ends.sort_unstable();
        let mut joined = Paths::new();
        for curve in curves {
            let mut line: Vec<[f64; 2]> = Vec::new();
            for w in curve.windows(2) {
                let key = (w[0].min(w[1]), w[0].max(w[1]));
                let i = by_ends[by_ends.binary_search_by_key(&key, |e| e.0).unwrap()].1;
                let arc = self.arcs.get(i);
                // A shared end is written once.
                let skip = usize::from(!line.is_empty());
                if self.ends[i].0 == w[0] {
                    line.extend_from_slice(&arc[skip..]);
                } else {
                    line.extend(arc.iter().rev().skip(skip));
                }
            }
            for piece in clip_polyline(self.rect, &line) {
                joined.push(&piece);
            }
        }
        joined
    }
}

/// A frame of `paths` at `progress`, for the runner to give its stage and pen.
fn frame(progress: f32, label: String, paths: Paths) -> Step {
    Step::Frame(Frame {
        stage: String::new(),
        progress,
        label,
        points: None,
        paths: Some(paths),
        pen: 0,
    })
}

impl StageRun for TruchetRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some(joined) = self.joined.take() {
            return Ok(Step::Done(Artifact::Paths(joined)));
        }
        if self.next_row < self.rows {
            let end = (self.next_row + self.per_frame).min(self.rows);
            for r in self.next_row..end {
                for c in 0..self.cols {
                    self.tile(c, r);
                }
            }
            self.next_row = end;
            let label = format!("{end}/{} rows", self.rows);
            return Ok(frame(
                end as f32 / self.rows as f32,
                label,
                self.arcs.clone(),
            ));
        }
        let joined = Arc::new(self.join());
        let label = format!("{} curves", joined.len());
        let shown = (*joined).clone();
        self.joined = Some(joined);
        Ok(frame(1.0, label, shown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Lane;
    use crate::test_util::{assert_inside, bounds, run_tone_stage_in_lane, tone_map};
    use serde_json::json;
    use std::collections::{BTreeMap, HashSet};

    /// The rect of `tone_map(16, 8, ..)`: four by two tiles of 8 mm.
    const RECT: [f64; 4] = [10.0, 10.0, 32.0, 16.0];

    /// The stage's run over `tone` with `patch` over tiles of 8 mm, where the default is 6, so
    /// that the tests' tile counts and arc numbers stay round.
    fn run(tone: ToneMap, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        run_in_lane(tone, patch, Lane::SINGLE)
    }

    fn run_in_lane(tone: ToneMap, patch: Value, lane: Lane) -> (Vec<Frame>, Arc<Paths>) {
        let patch = crate::settings::merge(&json!({"tile_mm": 8.0}), &patch);
        run_tone_stage_in_lane(&TruchetStage, tone, patch, lane)
    }

    /// How many arcs each 8 mm tile of `tone_map`'s rect holds, by column and row, among the tiles
    /// that hold any, taking an arc for the tile its middle vertex lies in.
    fn per_tile(frames: &[Frame]) -> BTreeMap<(i64, i64), usize> {
        let mut counts = BTreeMap::new();
        for a in arcs(frames).iter() {
            let tile = (
                (a[8][0] - 10.0).div_euclid(8.0),
                (a[8][1] - 10.0).div_euclid(8.0),
            );
            *counts.entry((tile.0 as i64, tile.1 as i64)).or_insert(0) += 1;
        }
        counts
    }

    /// The arcs before joining: the paths of the last frame of the tile pass, which comes just
    /// before the frame of the joined curves.
    fn arcs(frames: &[Frame]) -> &Paths {
        frames[frames.len() - 2].paths.as_ref().unwrap()
    }

    /// The centre of a quarter arc of 17 vertices: opposite the middle vertex across the chord,
    /// half the chord's length from it.
    fn centre(arc: &[[f64; 2]]) -> [f64; 2] {
        assert_eq!(arc.len(), 17);
        let (a, b, mid) = (arc[0], arc[16], arc[8]);
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let out = [mid[0] - m[0], mid[1] - m[1]];
        let k = math::hypot(b[0] - a[0], b[1] - a[1]) / 2.0 / math::hypot(out[0], out[1]);
        [m[0] - out[0] * k, m[1] - out[1] * k]
    }

    fn near(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    /// Whether `p` lies on the border of `rect`.
    fn on_border(rect: [f64; 4], p: [f64; 2]) -> bool {
        let [x, y, w, h] = rect;
        [x, x + w].iter().any(|&e| (p[0] - e).abs() < 1e-9)
            || [y, y + h].iter().any(|&e| (p[1] - e).abs() < 1e-9)
    }

    #[test]
    fn settings_schema() {
        let s = TruchetStage.schema();
        assert_eq!(bounds(&s, "tile_mm"), (2.0, 40.0, 6.0));
        assert_eq!(bounds(&s, "spacing_mm"), (0.1, 5.0, 0.4));
        assert_eq!(bounds(&s, "seed"), (0.0, 9999.0, 0.0));
        assert_eq!(s["properties"]["spacing_mm"]["x-density"], "inverse");
        assert_eq!(s["properties"]["tile_mm"]["x-step"], 0.5);
        assert_eq!(s["properties"]["spacing_mm"]["x-step"], 0.05);
        assert_eq!(
            (TruchetStage.name(), TruchetStage.label()),
            ("truchet", "Truchet tiles")
        );
    }

    #[test]
    fn corners_nest_as_far_as_the_diagonal_allows() {
        assert_eq!(nest_limit(8.0, 0.4), 3);
        assert_eq!(nest_limit(8.0, 1.0), 1);
        assert_eq!(nest_limit(2.0, 5.0), 0);
        assert_eq!(nest_limit(40.0, 0.1), 82);
        // One tile of full ink: its two corners each nest seven arcs, and the outermost arcs of
        // the two corners stay at least a spacing apart.
        let (frames, _) = run(tone_map(4, 4, |_, _| 1.0), json!({}));
        let all = arcs(&frames);
        assert_eq!(all.len(), 14);
        let corner = centre(all.get(0));
        let (one, other): (Vec<_>, Vec<_>) = all.iter().partition(|a| near(centre(a), corner));
        assert_eq!((one.len(), other.len()), (7, 7));
        let gap = (one.iter().flat_map(|a| a.iter()))
            .flat_map(|p| {
                (other.iter().flat_map(|a| a.iter()))
                    .map(move |q| math::hypot(p[0] - q[0], p[1] - q[1]))
            })
            .fold(f64::INFINITY, f64::min);
        assert!(gap >= 0.4 - 1e-9, "{gap}");
    }

    #[test]
    fn white_gives_no_paths_and_full_ink_fills_every_tile() {
        let (frames, paths) = run(tone_map(16, 8, |_, _| 0.0), json!({}));
        assert!(paths.is_empty() && arcs(&frames).is_empty());
        assert!(frames.last().unwrap().label.ends_with("curves"));
        // Eight tiles, two corners each, seven arcs about a corner.
        let (frames, _) = run(tone_map(16, 8, |_, _| 1.0), json!({}));
        assert_eq!(arcs(&frames).len(), 8 * 2 * 7);
        assert!(arcs(&frames).iter().all(|a| a.len() == 17));
    }

    #[test]
    fn ink_picks_the_odd_number_of_arcs_nearest() {
        // A single tile: (ink, arcs about a corner). Seven nests at most, so n = 7 ink.
        for (ink, per_corner) in [
            (0.05, 0),
            (0.1, 1),
            (0.3, 3),
            (0.5, 3),
            (0.6, 5),
            (1.0, 7),
            (1.2, 7),
        ] {
            let (frames, _) = run(tone_map(4, 4, |_, _| ink), json!({}));
            assert_eq!(arcs(&frames).len(), 2 * per_corner, "ink {ink}");
        }
    }

    #[test]
    fn a_tile_passes_its_rounding_error_on_to_the_tiles_after_it() {
        // Ink 0.3 wants 2.1 arcs a corner. Alone each tile would hold 3, the odd number nearest,
        // and 3 is 0.9 too many, which takes 7/16 of that from the next tile in the row and 5/16
        // from the one below. Rounding goes 3, 1, then the tile below wants 1.95 and holds 1, and
        // the last, owed what the three before it passed on, wants 2.68 and holds 3.
        let (frames, _) = run(tone_map(8, 8, |_, _| 0.3), json!({}));
        let want = [((0, 0), 6), ((1, 0), 2), ((0, 1), 2), ((1, 1), 6)];
        assert_eq!(per_tile(&frames), BTreeMap::from(want));
    }

    #[test]
    fn rounding_error_keeps_the_mean_and_every_count_odd() {
        // Ink 0.5 wants 3.5 arcs a corner, where plain rounding gives every tile 3.
        let (frames, _) = run(tone_map(160, 80, |_, _| 0.5), json!({}));
        let counts = per_tile(&frames);
        // Two corners hold an odd number of arcs each, so a tile holds 2 mod 4.
        assert!(counts.values().all(|n| n % 4 == 2), "{counts:?}");
        assert!(counts.values().any(|&n| n == 6) && counts.values().any(|&n| n == 10));
        // 40 by 20 tiles, with none left empty.
        let mean = counts.values().sum::<usize>() as f64 / 2.0 / (40 * 20) as f64;
        assert!((mean - 3.5).abs() < 0.02, "{mean}");
    }

    #[test]
    fn over_full_ink_every_curve_is_a_loop_or_ends_on_the_edge() {
        let rect = [10.0, 10.0, 48.0, 24.0];
        let (frames, paths) = run(tone_map(24, 12, |_, _| 1.0), json!({}));
        assert!(!paths.is_empty());
        for p in paths.iter() {
            let (a, b) = (p[0], p[p.len() - 1]);
            assert!(
                near(a, b) || (on_border(rect, a) && on_border(rect, b)),
                "{a:?} to {b:?}"
            );
        }
        // Joined: far fewer paths than arcs, and every arc's vertices are in them.
        let all = arcs(&frames);
        assert_eq!(all.len(), 18 * 14);
        assert!(paths.len() * 3 < all.len(), "{} curves", paths.len());
        let vertices: usize = paths.iter().map(<[_]>::len).sum();
        assert_eq!(vertices, all.len() * 16 + paths.len());
    }

    #[test]
    fn a_tile_with_fewer_arcs_ends_the_extra_ones_at_the_shared_side() {
        // Two tiles: the left full of ink (seven arcs a corner), the right with three.
        let tone = tone_map(8, 4, |x, _| if x < 4 { 1.0 } else { 0.4 });
        let (_, paths) = run(tone, json!({}));
        // The shared side is at x = 18 and its middle at y = 14.
        let ends: Vec<f64> = {
            let mut ys: Vec<f64> = paths
                .iter()
                .flat_map(|p| [p[0], p[p.len() - 1]])
                .filter(|e| (e[0] - 18.0).abs() < 1e-9)
                .map(|e| e[1])
                .collect();
            ys.sort_by(f64::total_cmp);
            ys
        };
        assert_eq!(ends.len(), 4, "{ends:?}");
        for (got, want) in ends.iter().zip([12.8, 13.2, 14.8, 15.2]) {
            assert!((got - want).abs() < 1e-9, "{ends:?}");
        }
        // The three arcs both tiles hold join through the side.
        for y in [13.6, 14.0, 14.4] {
            let through = paths
                .iter()
                .any(|p| p[1..p.len() - 1].iter().any(|&v| near(v, [18.0, y])));
            assert!(through, "no curve crosses at y = {y}");
        }
    }

    #[test]
    fn a_change_of_tone_does_not_change_the_turns() {
        // The right half goes blank, and the arcs that stay are the very same arcs.
        let (full, _) = run(tone_map(16, 8, |_, _| 1.0), json!({"seed": 5}));
        let half = |x, _| if x < 8 { 1.0 } else { 0.0 };
        let (left, _) = run(tone_map(16, 8, half), json!({"seed": 5}));
        let bits = |paths: &Paths| -> HashSet<Vec<[u64; 2]>> {
            paths
                .iter()
                .map(|a| a.iter().map(|p| p.map(f64::to_bits)).collect())
                .collect()
        };
        let (all, kept) = (bits(arcs(&full)), bits(arcs(&left)));
        assert_eq!((all.len(), kept.len()), (8 * 14, 4 * 14));
        assert!(kept.is_subset(&all));
    }

    #[test]
    fn a_seed_turns_the_tiles_and_a_run_repeats() {
        let tone = || tone_map(16, 8, |_, _| 1.0);
        let (_, a) = run(tone(), json!({"seed": 3}));
        assert_eq!(a.coords(), run(tone(), json!({"seed": 3})).1.coords());
        assert_ne!(a.coords(), run(tone(), json!({"seed": 4})).1.coords());
    }

    #[test]
    fn a_second_lane_turns_the_grid() {
        let tone = || tone_map(16, 8, |_, _| 1.0);
        // The corners of the upright grid: 8 mm apart from the rect's top-left corner.
        let on_grid = |k: [f64; 2]| {
            let (u, v) = ((k[0] - 10.0) / 8.0, (k[1] - 10.0) / 8.0);
            (u - u.round()).abs() < 1e-6 && (v - v.round()).abs() < 1e-6
        };
        let (frames, _) = run(tone(), json!({}));
        assert!(arcs(&frames).iter().all(|a| on_grid(centre(a))));
        let lane = Lane { index: 1, count: 2 };
        let (frames, paths) = run_in_lane(tone(), json!({}), lane);
        let all = arcs(&frames);
        // Only the pivot of the turn, the map's centre, is a corner of both grids.
        let pivot = [26.0, 18.0];
        let shared = all
            .iter()
            .map(centre)
            .filter(|&k| on_grid(k) && !near(k, pivot));
        assert_eq!(shared.count(), 0);
        // Tiles turned off the map hold no arcs, and the ones that cover it hold them all.
        assert!(all.len() > 8 * 14 && all.len() < 36 * 14, "{}", all.len());
        let [x, y, w, h] = RECT;
        let reach = |axis: usize, lo: f64, hi: f64| {
            let (a, b) = (paths.coords().iter().map(|p| p[axis]))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                    (a.min(v), b.max(v))
                });
            a < lo + 1.0 && b > hi - 1.0
        };
        assert!(reach(0, x, x + w) && reach(1, y, y + h));
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        // Tiles of 6 mm overhang this 20 by 10 mm map on two sides, and the ink varies.
        let ink = |x: u32, y: u32| (x + 2 * y) as f32 / 18.0;
        for lane in [
            Lane::SINGLE,
            Lane { index: 1, count: 2 },
            Lane { index: 2, count: 3 },
        ] {
            let (_, paths) = run_in_lane(
                tone_map(10, 5, ink),
                json!({"tile_mm": 6.0, "spacing_mm": 0.5}),
                lane,
            );
            assert!(!paths.is_empty(), "{lane:?}");
            assert_inside(&paths, [10.0, 10.0, 20.0, 10.0]);
        }
    }
}
