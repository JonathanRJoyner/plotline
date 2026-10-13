//! The ink stage: draw an image's own marks rather than its tone. Every ink shape is outlined and
//! filled with rings that follow its edge inwards, and ink too thin to hold a ring is drawn once
//! down its middle, so line art, lettering and brush work keep their shapes and weights.
//!
//! The tone map is resampled bilinearly onto a finer working grid, so edges fall between pixels
//! and the rings come out smooth, softened by half a tone pixel of Gaussian blur against pixel
//! steps and compression noise, and thresholded into a mask. The rings are iso-lines of the
//! mask's distance transform; the ink no ring covers is thinned to a skeleton and walked into
//! centrelines. The algorithms are cited in [`crate::raster`].

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::clamp_to;
use crate::raster;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, ToneMap};

/// How the ink stage draws a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InkMode {
    /// Outline every shape and fill it with rings, so solid areas plot solid.
    Fill,
    /// Outline every shape once and leave it hollow.
    Outline,
    /// One line down the middle of every stroke, for clean line art drawn with single strokes.
    Centreline,
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the ink stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct InkSettings {
    /// How ink shapes are drawn: filled with rings, outlined once, or one line down the middle of each stroke.
    pub mode: InkMode,
    /// Least ink density that counts as part of a shape; lower it to keep faint lines, raise it to drop a grey background.
    #[schemars(range(min = 0.05, max = 0.95), extend("x-step" = 0.01))]
    pub threshold: f64,
    /// Distance between neighbouring fill rings as a share of the pen width; below 1 they overlap so filled shapes plot solid.
    #[schemars(range(min = 0.3, max = 1.5), extend("x-step" = 0.05))]
    pub spacing: f64,
    /// Ink shapes smaller than this area are dropped as specks.
    #[schemars(
        range(min = 0.0, max = 4.0),
        extend("x-step" = 0.01, "x-unit" = "mm²", "x-advanced" = true)
    )]
    pub min_area_mm2: f64,
}

impl Default for InkSettings {
    fn default() -> Self {
        InkSettings {
            mode: InkMode::Fill,
            threshold: 0.5,
            spacing: 0.8,
            min_area_mm2: 0.02,
        }
    }
}

/// The working grid stays under this many cells, which bounds memory in a browser tab.
const MAX_CELLS: f64 = 12e6;
/// The pen should span this many working cells, so rings a fraction of a pen apart resolve.
const CELLS_PER_PEN: f64 = 6.0;
/// The outer ring's ink reaches this share of the pen past the shape's edge, as ink spreads.
const OVERHANG: f64 = 0.25;
/// The blur before thresholding, in tone pixels.
const SOFTEN: f64 = 0.5;
/// Rings are smoothed along a Gaussian of this many pen widths: neighbouring rings then follow
/// each other instead of wobbling apart on the grid's steps and leaving gaps in a fill.
const RING_BLUR: f64 = 1.0;
/// Centrelines are smoothed less, so the turns of a stroke stay sharp.
const LINE_BLUR: f64 = 0.6;
/// Missed ink inside a square this many pens wide is left to the rings around it.
const CRUMB: f64 = 1.2;
/// Ink further than this share of the pen from every ring is drawn as a centreline.
const REACH: f64 = 0.45;

/// Outlines and fills, or centrelines, of the ink shapes in the tone map.
pub struct InkStage;

impl Stage for InkStage {
    fn name(&self) -> &'static str {
        "ink"
    }
    fn label(&self) -> &'static str {
        "Ink"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone, Kind::Pens]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<InkSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<InkSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<InkSettings>(self.name(), &self.schema(), settings)?;
        let pen = inputs
            .pens()
            .first()
            .ok_or_else(|| StageError::Failed("the ink style needs a pen".into()))?
            .width_mm;
        Ok(Box::new(InkRun {
            tone: inputs.tone().clone(),
            s,
            pen,
            phase: 0,
            grid: None,
            rings: Vec::new(),
            paths: Paths::new(),
        }))
    }
}

/// The working grid: the mask, its size, how many cells a tone pixel spans and a cell's size.
struct Grid {
    mask: Vec<bool>,
    w: usize,
    h: usize,
    k: usize,
    cell_mm: f64,
}

/// Three working steps, each ending in a frame, then the paths.
struct InkRun {
    tone: Arc<ToneMap>,
    s: InkSettings,
    pen: f64,
    phase: u8,
    grid: Option<Grid>,
    /// The rings in cell coordinates, before smoothing.
    rings: Vec<Vec<[f64; 2]>>,
    paths: Paths,
}

impl InkRun {
    /// The thresholded working grid with a border of blank cells, specks removed.
    fn grid(&self) -> Grid {
        let t = &self.tone;
        let (tw, th) = (t.width as usize, t.height as usize);
        let want = (CELLS_PER_PEN * t.mm_per_px / self.pen)
            .ceil()
            .clamp(1.0, 4.0);
        let room = (MAX_CELLS / (tw * th) as f64).sqrt().floor().max(1.0);
        let k = want.min(room) as usize;
        let fine = crate::math::resample(&t.ink, tw, th, tw * k, th * k);
        // Half a tone pixel of blur rounds off the source's pixel steps and compression noise.
        let fine = crate::math::blur(&fine, tw * k, th * k, SOFTEN * k as f64);
        let (w, h) = (tw * k + 2, th * k + 2);
        let level = self.s.threshold as f32;
        let mut mask = vec![false; w * h];
        for y in 0..th * k {
            for x in 0..tw * k {
                mask[(y + 1) * w + x + 1] = fine[y * tw * k + x] >= level;
            }
        }
        let cell_mm = t.mm_per_px / k as f64;
        let min_cells = (self.s.min_area_mm2 / (cell_mm * cell_mm)).round() as usize;
        raster::drop_small(&mut mask, w, min_cells);
        Grid {
            mask,
            w,
            h,
            k,
            cell_mm,
        }
    }

    /// Cell-coordinate polylines smoothed along a Gaussian of `blur` pens and simplified.
    fn polish(&self, lines: Vec<(Vec<[f64; 2]>, bool)>, blur: f64) -> Vec<Vec<[f64; 2]>> {
        let g = self.grid.as_ref().expect("the grid is built first");
        let pen_cells = self.pen / g.cell_mm;
        let sigma = (blur * pen_cells).max(1.0);
        lines
            .into_iter()
            .map(|(line, closed)| {
                // A Gaussian shrinks a loop by about sigma^2 / (2 r); a twelfth of the loop's
                // length keeps that under a seventh of its radius, so small dots keep their size.
                let sigma = if closed {
                    sigma.min(line.len() as f64 / 12.0)
                } else {
                    sigma
                };
                raster::simplify(&raster::smooth(&line, sigma, closed), pen_cells / 20.0)
            })
            .collect()
    }

    /// Cell-coordinate polylines added to the paths in page mm.
    fn add(&mut self, lines: &[Vec<[f64; 2]>]) {
        let g = self.grid.as_ref().expect("the grid is built first");
        let rect = self.tone.rect_mm();
        for line in lines {
            // Cell x is working pixel x - 1, whose centre lies (x - 0.5) / k tone pixels in.
            let mm: Vec<[f64; 2]> = line
                .iter()
                .map(|&[x, y]| {
                    let px = [(x - 0.5) / g.k as f64, (y - 0.5) / g.k as f64];
                    clamp_to(rect, self.tone.to_mm(px))
                })
                .collect();
            self.paths.push(&mm);
        }
    }

    fn frame(&self, progress: f32, label: String) -> Step {
        Step::Frame(Frame {
            stage: String::new(),
            progress,
            label,
            points: None,
            paths: Some(self.paths.clone()),
            pen: 0,
        })
    }
}

impl StageRun for InkRun {
    fn step(&mut self) -> Result<Step, StageError> {
        self.phase += 1;
        match self.phase {
            1 => {
                self.grid = Some(self.grid());
                Ok(self.frame(0.2, "ink shapes".into()))
            }
            2 => {
                if self.s.mode != InkMode::Centreline {
                    let g = self.grid.as_ref().unwrap();
                    let pen_cells = self.pen / g.cell_mm;
                    let depth = raster::distance(&g.mask, g.w, g.h);
                    // The distance is 1 at an edge cell, whose centre is half a cell inside.
                    let first = 0.5 + (0.5 - OVERHANG) * pen_cells;
                    let last = if self.s.mode == InkMode::Outline {
                        first
                    } else {
                        f64::INFINITY
                    };
                    let step = self.s.spacing * pen_cells;
                    let rings = raster::iso_lines(&depth, g.w, g.h, first, step, last);
                    // Smoothed before the coverage test, so it judges the rings that get drawn.
                    self.rings =
                        self.polish(rings.into_iter().map(|r| (r, true)).collect(), RING_BLUR);
                    let rings = std::mem::take(&mut self.rings);
                    self.add(&rings);
                    self.rings = rings;
                }
                let n = self.paths.len();
                Ok(self.frame(0.7, format!("{n} rings")))
            }
            3 => {
                let g = self.grid.as_ref().unwrap();
                let pen_cells = self.pen / g.cell_mm;
                let mut missed = g.mask.clone();
                if !self.rings.is_empty() {
                    let near = raster::cover(&self.rings, g.w, g.h, REACH * pen_cells);
                    for (m, n) in missed.iter_mut().zip(near) {
                        *m = *m && !n;
                    }
                    // Missed ink smaller than a square of CRUMB pens sits between rings whose ink
                    // spreads over it, and drawing it costs a pen lift per crumb.
                    let crumbs = (CRUMB * CRUMB * pen_cells * pen_cells).round() as usize;
                    raster::drop_small(&mut missed, g.w, crumbs.max(2));
                }
                let specks = raster::groups(&missed, g.w);
                raster::thin(&mut missed, g.w);
                let mut lines = raster::skeleton_lines(&missed, g.w, pen_cells);
                // Thinning can erase a group of a few cells outright; each becomes a dot.
                for group in specks.iter().filter(|s| s.iter().all(|&i| !missed[i])) {
                    let n = group.len() as f64;
                    let sum = group.iter().fold([0.0, 0.0], |a, &i| {
                        [a[0] + (i % g.w) as f64, a[1] + (i / g.w) as f64]
                    });
                    lines.push((vec![[sum[0] / n, sum[1] / n]], false));
                }
                let before = self.paths.len();
                let lines = self.polish(lines, LINE_BLUR);
                self.add(&lines);
                let n = self.paths.len() - before;
                Ok(self.frame(1.0, format!("{n} centrelines")))
            }
            _ => Ok(Step::Done(Artifact::Paths(Arc::new(std::mem::take(
                &mut self.paths,
            ))))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_inside, bounds};
    use crate::types::Pen;
    use serde_json::json;
    use std::collections::HashMap;

    /// A `w` by `h` tone map, `mm` per pixel, corner at (10, 10) mm, ink where `f` holds.
    fn map(w: u32, h: u32, mm: f64, f: impl Fn(f64, f64) -> bool) -> ToneMap {
        let ink = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let c = [(f64::from(x) + 0.5) * mm, (f64::from(y) + 0.5) * mm];
                if f(c[0], c[1]) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        ToneMap {
            width: w,
            height: h,
            ink,
            mm_per_px: mm,
            origin_mm: [10.0, 10.0],
        }
    }

    /// Run the stage with pen width `pen` and `patch` over the defaults.
    fn run(tone: ToneMap, pen: f64, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        let settings = crate::settings::merge(&InkStage.defaults(), &patch);
        let pens = vec![Pen::new("p", pen, "#000000").unwrap()];
        let inputs = Inputs::new(HashMap::from([
            (Kind::Tone, Artifact::Tone(Arc::new(tone))),
            (Kind::Pens, Artifact::Pens(Arc::new(pens))),
        ]));
        let mut r = InkStage
            .start(inputs, &settings, RunContext::new(Arc::default()))
            .unwrap();
        let mut frames = Vec::new();
        loop {
            match r.step().unwrap() {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Paths(p)) => return (frames, p),
                Step::Done(a) => panic!("expected paths, got {}", a.kind().name()),
            }
        }
    }

    fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        reach(&[[a[0] + t * dx, a[1] + t * dy]], p)
    }

    fn closed(p: &[[f64; 2]]) -> bool {
        p.len() > 2 && p[0] == p[p.len() - 1]
    }

    /// The largest distance of a path's vertices from `c`.
    fn reach(p: &[[f64; 2]], c: [f64; 2]) -> f64 {
        p.iter()
            .map(|v| ((v[0] - c[0]) * (v[0] - c[0]) + (v[1] - c[1]) * (v[1] - c[1])).sqrt())
            .fold(0.0, f64::max)
    }

    #[test]
    fn settings_schema() {
        let s = InkStage.schema();
        assert_eq!(bounds(&s, "threshold"), (0.05, 0.95, 0.5));
        assert_eq!(bounds(&s, "spacing"), (0.3, 1.5, 0.8));
        assert_eq!(bounds(&s, "min_area_mm2"), (0.0, 4.0, 0.02));
        assert_eq!(InkStage.defaults()["mode"], "fill");
        assert_eq!((InkStage.name(), InkStage.label()), ("ink", "Ink"));
    }

    #[test]
    fn a_disc_fills_with_nested_rings() {
        // A disc of radius 3 mm on a 0.05 mm grid, centred at (14, 14) mm on the page.
        let disc = map(160, 160, 0.05, |x, y| {
            (x - 4.0) * (x - 4.0) + (y - 4.0) * (y - 4.0) < 9.0
        });
        let (frames, paths) = run(disc, 0.5, json!({}));
        let c = [14.0, 14.0];
        let rings: Vec<&[[f64; 2]]> = paths.iter().filter(|p| closed(p)).collect();
        // Insets 0.125, 0.525, ... 2.925 mm: eight rings inside a 3 mm radius, the last so small
        // that smoothing may shrink it to a dot.
        assert!((7..=8).contains(&rings.len()), "{}", rings.len());
        let outer = rings.iter().map(|r| reach(r, c)).fold(0.0, f64::max);
        assert!((outer - 2.875).abs() < 0.06, "{outer}");
        // The fill is solid: every point of the disc lies under the 0.5 mm pen.
        let segs: Vec<([f64; 2], [f64; 2])> = paths
            .iter()
            .flat_map(|p| p.windows(2).map(|s| (s[0], s[1])).collect::<Vec<_>>())
            .collect();
        for i in 0..=24 {
            for j in 0..=24 {
                let q = [11.0 + f64::from(i) * 0.25, 11.0 + f64::from(j) * 0.25];
                if reach(&[q], c) < 2.95 {
                    let near = segs
                        .iter()
                        .map(|&(a, b)| seg_dist(q, a, b))
                        .fold(9.0, f64::min);
                    assert!(near <= 0.25, "{q:?} is {near} mm from the nearest line");
                }
            }
        }
        assert_eq!(frames.len(), 3);
        assert_inside(&paths, [10.0, 10.0, 8.0, 8.0]);
    }

    #[test]
    fn outline_mode_keeps_one_ring() {
        let disc = map(160, 160, 0.05, |x, y| {
            (x - 4.0) * (x - 4.0) + (y - 4.0) * (y - 4.0) < 9.0
        });
        let (_, paths) = run(disc, 0.5, json!({"mode": "outline"}));
        let rings: Vec<&[[f64; 2]]> = paths.iter().filter(|p| closed(p)).collect();
        assert_eq!(rings.len(), 1);
    }

    #[test]
    fn a_hairline_becomes_one_centreline() {
        // A line 0.1 mm wide, thinner than half of a 0.5 mm pen, from x 1 to 7 mm at y 2 mm.
        let hair = map(160, 80, 0.05, |x, y| {
            (1.0..7.0).contains(&x) && (y - 2.0).abs() < 0.05
        });
        let (_, paths) = run(hair, 0.5, json!({}));
        assert_eq!(
            paths.len(),
            1,
            "{:?}",
            paths.iter().map(|p| p.len()).collect::<Vec<_>>()
        );
        let p = paths.get(0);
        assert!(!closed(p));
        let xs: Vec<f64> = p.iter().map(|v| v[0]).collect();
        let span = xs.iter().copied().fold(0.0, f64::max) - xs.iter().copied().fold(99.0, f64::min);
        assert!(span > 5.5, "{span}");
        assert!(p.iter().all(|v| (v[1] - 12.0).abs() < 0.1));
    }

    #[test]
    fn centreline_mode_draws_a_bar_once() {
        let bar = map(160, 80, 0.05, |x, y| {
            (1.0..7.0).contains(&x) && (y - 2.0).abs() < 0.4
        });
        let (_, paths) = run(bar, 0.3, json!({"mode": "centreline"}));
        assert!(paths.iter().all(|p| !closed(p)));
        let longest = paths.iter().map(Paths::path_length).fold(0.0, f64::max);
        assert!(longest > 4.5, "{longest}");
    }

    #[test]
    fn specks_are_dropped() {
        // A 0.1 mm square dot: 0.01 mm², under the default 0.02.
        let dot = map(80, 80, 0.05, |x, y| {
            (2.0..2.1).contains(&x) && (2.0..2.1).contains(&y)
        });
        let (_, paths) = run(dot, 0.3, json!({}));
        assert!(paths.is_empty());
        let (_, kept) = run(
            map(80, 80, 0.05, |x, y| {
                (2.0..2.1).contains(&x) && (2.0..2.1).contains(&y)
            }),
            0.3,
            json!({"min_area_mm2": 0.0}),
        );
        assert!(!kept.is_empty());
    }

    #[test]
    fn output_is_deterministic_and_coarse_maps_are_refined() {
        // On a coarse 0.5 mm map the stage works on a finer grid, so the ring is still round.
        let coarse = || {
            map(20, 20, 0.5, |x, y| {
                (x - 5.0) * (x - 5.0) + (y - 5.0) * (y - 5.0) < 9.0
            })
        };
        let (_, a) = run(coarse(), 0.3, json!({"mode": "outline"}));
        let (_, b) = run(coarse(), 0.3, json!({"mode": "outline"}));
        assert_eq!(a.coords(), b.coords());
        let ring = a.iter().find(|p| closed(p)).unwrap();
        let c = [15.0, 15.0];
        let lo = ring.iter().map(|v| reach(&[*v], c)).fold(99.0, f64::min);
        let hi = reach(ring, c);
        assert!(hi - lo < 0.2, "{lo} {hi}");
    }
}
