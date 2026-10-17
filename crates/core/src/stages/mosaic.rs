//! The mosaic stage: the tone map cut into patches that follow its shapes, each filled with
//! parallel lines along its long axis.
//!
//! The patches are SLIC superpixels, R. Achanta, A. Shaji, K. Smith, A. Lucchi, P. Fua and S.
//! Susstrunk, "SLIC superpixels compared to state-of-the-art superpixel methods", *IEEE Trans.
//! PAMI* 34(11), 2012, pp. 2274-2282. Centres start on a grid, move to the lowest gradient in their
//! 3 by 3 neighbourhood, and ten passes of local k-means follow: each pixel joins the nearest
//! centre among those whose window of twice the grid interval holds it, by the distance
//! `D = sqrt(dc^2 + (ds / S)^2 m^2)`, and each centre moves to the mean of its pixels. The last
//! step of the paper relabels disjoint segments so that every region is connected. The paper works
//! in CIELAB; plotline has one channel, the ink, and measures `dc` as `100 |ink_a - ink_b|`, the
//! range of CIELAB lightness, so the compactness `m` keeps the paper's meaning.
//!
//! A patch's lines run along the principal axis of its pixels' second central moments,
//! `theta = atan2(2 mu11, mu20 - mu02) / 2`, the moments of M.-K. Hu, "Visual pattern recognition
//! by moment invariants", *IRE Trans. Information Theory* 8(2), 1962, pp. 179-187. Tone is shown by
//! the spacing of the lines, as stroke textures do in G. Winkenbach and D. H. Salesin,
//! "Computer-generated pen-and-ink illustration", *Proc. SIGGRAPH*, 1994, pp. 91-100: lines of
//! width `w` at spacing `d` cover `w / d` of the paper, so a spacing of `spacing_mm / t` over ink
//! `t` covers in proportion to the ink.
//!
//! The paper does not say how small a connected piece may be before it must join a neighbour; the
//! floor of a quarter of `S^2` pixels is this project's choice. A pixel that no window holds in the
//! first pass keeps the label of its grid cell. A line that crosses a patch for less than two
//! pixels, as at a corner, draws nothing, since the dash would be shorter than the pen is wide.
//!
//! The lines of a patch are drawn back and forth and joined into one path only where the gap from
//! one line's end to the next one's start is within twice `spacing_mm`, the spacing over full ink,
//! and the middle of the join lies in the patch. The limit is absolute rather than a multiple of
//! the patch's own spacing: a light patch has few lines far apart, and joining those draws a U or
//! a frame much darker than the tone, so they stay loose strokes, while the connectors of a dark
//! patch vanish among its lines.
//!
//! When the drawing is split between pens, each pen's lines turn by its share of the half turn, so
//! that the pens do not draw over each other.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, clip, direction, sample_line, ChunkRun};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, Points, ToneMap};

/// Passes of local k-means; Achanta et al. find ten enough.
const PASSES: usize = 10;

/// How many full-ink spacings apart two neighbouring lines of a patch may end and still be joined
/// by a connector. The limit is absolute, not a multiple of the patch's own spacing: a light patch
/// has few, widely spaced lines, and joining them would draw it as a U or a frame, far darker than
/// its tone, so its lines stay loose strokes, while in a dark patch the connectors vanish among
/// the lines.
const JOIN: f64 = 2.0;

/// The shortest run of a line inside a region that is drawn, in pixels. A line that only grazes a
/// region's corner would draw a dash shorter than the pen is wide, which plot lint calls a speck.
const MIN_RUN_PX: f64 = 2.0;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the mosaic stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MosaicSettings {
    /// About how wide one patch is.
    #[schemars(range(min = 2.0, max = 50.0), extend("x-step" = 0.5, "x-unit" = "mm"))]
    pub region_mm: f64,
    /// Higher keeps the patches round; lower lets them follow the image's shapes.
    #[schemars(range(min = 1.0, max = 40.0), extend("x-step" = 1.0))]
    pub compactness: f64,
    /// Distance between lines over full ink; lighter patches space theirs wider.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-density" = "inverse", "x-density-of" = "lines", "x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
}

impl Default for MosaicSettings {
    fn default() -> Self {
        MosaicSettings {
            region_mm: 8.0,
            compactness: 3.0,
            spacing_mm: 0.6,
        }
    }
}

/// Patches of parallel lines, each at its own angle and spacing.
pub struct MosaicStage;

impl Stage for MosaicStage {
    fn name(&self) -> &'static str {
        "mosaic"
    }
    fn label(&self) -> &'static str {
        "Mosaic"
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
        schema_for::<MosaicSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<MosaicSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<MosaicSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let lane = ctx.lane();
        let window = (s.region_mm / tone.mm_per_px).max(2.0);
        Ok(Box::new(MosaicRun {
            slic: Slic::new(tone, window, s.compactness),
            window,
            spacing: s.spacing_mm,
            // Each pen's lines turn by its share of the half turn.
            turn: 180.0 * lane.index as f64 / lane.count as f64,
            pass: 0,
            phase: Phase::Seeds,
            lines: None,
        }))
    }
}

/// The local k-means of SLIC over a tone map.
struct Slic {
    tone: Arc<ToneMap>,
    /// The grid interval `S` in pixels, also the half width of a centre's window.
    s: f64,
    /// `m^2 / S^2`, the weight of the squared pixel distance.
    spatial: f64,
    /// Each centre's `x` and `y` in pixels and its ink.
    centres: Vec<[f64; 3]>,
    /// The centre each pixel belongs to, row-major.
    labels: Vec<u32>,
    /// The distance of each pixel to its label in the pass in progress.
    dist: Vec<f64>,
}

impl Slic {
    /// The seeds on a grid of interval `s` pixels, each moved to the lowest gradient of its 3 by 3
    /// neighbourhood, and every pixel labelled with the seed of its grid cell.
    fn new(tone: Arc<ToneMap>, s: f64, m: f64) -> Slic {
        let (w, h) = (tone.width as usize, tone.height as usize);
        let nx = ((w as f64 / s).round() as usize).max(1);
        let ny = ((h as f64 / s).round() as usize).max(1);
        // The ink at a pixel, read clamped at the map's edge.
        let ink = |x: i64, y: i64| {
            let (x, y) = (x.clamp(0, w as i64 - 1), y.clamp(0, h as i64 - 1));
            f64::from(tone.ink[y as usize * w + x as usize])
        };
        let mut centres = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                let cx = ((i as f64 + 0.5) * w as f64 / nx as f64) as i64;
                let cy = ((j as f64 + 0.5) * h as f64 / ny as f64) as i64;
                let (mut best, mut at) = (f64::INFINITY, (cx, cy));
                for y in cy - 1..=cy + 1 {
                    for x in cx - 1..=cx + 1 {
                        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                            continue;
                        }
                        let (dx, dy) =
                            (ink(x + 1, y) - ink(x - 1, y), ink(x, y + 1) - ink(x, y - 1));
                        let g = dx * dx + dy * dy;
                        if g < best {
                            (best, at) = (g, (x, y));
                        }
                    }
                }
                centres.push([at.0 as f64, at.1 as f64, ink(at.0, at.1)]);
            }
        }
        let labels = (0..w * h)
            .map(|p| ((p / w * ny / h) * nx + p % w * nx / w) as u32)
            .collect();
        Slic {
            s,
            spatial: m * m / (s * s),
            dist: vec![f64::INFINITY; w * h],
            centres,
            labels,
            tone,
        }
    }

    /// One pass: every pixel takes the nearest centre whose window holds it (a pixel in no window
    /// keeps its label), then each centre moves to the mean of its pixels.
    fn pass(&mut self) {
        let (w, h) = (self.tone.width as usize, self.tone.height as usize);
        self.dist.fill(f64::INFINITY);
        for (c, &[cx, cy, ink]) in self.centres.iter().enumerate() {
            let x0 = (cx - self.s).ceil().max(0.0) as usize;
            let x1 = ((cx + self.s).floor() as usize).min(w - 1);
            let y0 = (cy - self.s).ceil().max(0.0) as usize;
            let y1 = ((cy + self.s).floor() as usize).min(h - 1);
            for y in y0..=y1 {
                let dy = y as f64 - cy;
                for x in x0..=x1 {
                    let dx = x as f64 - cx;
                    let p = y * w + x;
                    let dc = 100.0 * (f64::from(self.tone.ink[p]) - ink);
                    let d = dc * dc + self.spatial * (dx * dx + dy * dy);
                    // Strictly nearer, so a tie goes to the lower centre.
                    if d < self.dist[p] {
                        self.dist[p] = d;
                        self.labels[p] = c as u32;
                    }
                }
            }
        }
        // Per centre: the sums of x, y and ink over its pixels, and their number.
        let mut sums = vec![[0.0; 4]; self.centres.len()];
        for (p, &c) in self.labels.iter().enumerate() {
            let sum = &mut sums[c as usize];
            sum[0] += (p % w) as f64;
            sum[1] += (p / w) as f64;
            sum[2] += f64::from(self.tone.ink[p]);
            sum[3] += 1.0;
        }
        for (c, sum) in self.centres.iter_mut().zip(&sums) {
            if sum[3] > 0.0 {
                *c = [sum[0] / sum[3], sum[1] / sum[3], sum[2] / sum[3]];
            }
        }
    }

    /// The centres on the page, in mm.
    fn points(&self) -> Points {
        let at = |c: &[f64; 3]| self.tone.to_mm([c[0] + 0.5, c[1] + 0.5]);
        Points(self.centres.iter().map(at).collect(), None)
    }
}

/// The 4-connected components of `labels`, a map `width` pixels wide, numbered in row-major order of
/// their first pixel. In that order each component under `min` pixels joins the neighbouring
/// component with the most pixels at that moment (a tie goes to the lower number), and that one
/// grows by its size. Returns the region of each pixel, numbered in order of its first pixel, and
/// the number of regions.
fn connect(labels: &[u32], width: usize, min: f64) -> (Vec<u32>, usize) {
    const NONE: u32 = u32::MAX;
    let n = labels.len();
    let mut comp = vec![NONE; n];
    let mut size: Vec<usize> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..n {
        if comp[start] != NONE {
            continue;
        }
        let id = size.len() as u32;
        comp[start] = id;
        stack.push(start);
        let mut count = 0;
        while let Some(p) = stack.pop() {
            count += 1;
            let x = p % width;
            let sides = [
                (x > 0).then(|| p - 1),
                (x + 1 < width).then(|| p + 1),
                (p >= width).then(|| p - width),
                (p + width < n).then(|| p + width),
            ];
            for q in sides.into_iter().flatten() {
                if comp[q] == NONE && labels[q] == labels[start] {
                    comp[q] = id;
                    stack.push(q);
                }
            }
        }
        size.push(count);
    }
    // Only a small component ever looks for a neighbour, so only its edges are kept.
    let small = |size: &[usize], c: u32| (size[c as usize] as f64) < min;
    let mut edges: Vec<(u32, u32)> = Vec::new();
    for p in 0..n {
        let right = (p % width + 1 < width).then(|| p + 1);
        for q in right.into_iter().chain((p + width < n).then(|| p + width)) {
            let (a, b) = (comp[p], comp[q]);
            if a == b {
                continue;
            }
            for e in [(a, b), (b, a)] {
                if small(&size, e.0) && edges.last() != Some(&e) {
                    edges.push(e);
                }
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let mut near: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (a, b) in edges {
        near.entry(a).or_default().push(b);
    }
    let mut parent: Vec<u32> = (0..size.len() as u32).collect();
    let find = |parent: &mut [u32], mut c: u32| {
        while parent[c as usize] != c {
            parent[c as usize] = parent[parent[c as usize] as usize];
            c = parent[c as usize];
        }
        c
    };
    for c in 0..size.len() as u32 {
        if parent[c as usize] != c || !small(&size, c) {
            continue;
        }
        // A component without neighbours is the whole map.
        let Some(around) = near.remove(&c) else {
            continue;
        };
        let mut best: Option<u32> = None;
        for &a in &around {
            let r = find(&mut parent, a);
            let better = best.is_none_or(|b| {
                size[r as usize] > size[b as usize]
                    || (size[r as usize] == size[b as usize] && r < b)
            });
            if r != c && better {
                best = Some(r);
            }
        }
        let Some(best) = best else { continue };
        parent[c as usize] = best;
        size[best as usize] += size[c as usize];
        if let Some(list) = near.get_mut(&best) {
            list.extend(around);
        }
    }
    let mut number = vec![NONE; size.len()];
    let mut count = 0;
    for c in &mut comp {
        let root = find(&mut parent, *c) as usize;
        if number[root] == NONE {
            number[root] = count;
            count += 1;
        }
        *c = number[root];
    }
    (comp, count as usize)
}

/// What the lines of a region need to know about it.
struct Region {
    /// The mean ink.
    ink: f64,
    /// The mean of the pixel centres in mm.
    centre: [f64; 2],
    /// The long axis, counter-clockwise from horizontal as seen on the page.
    angle_deg: f64,
    /// The first and last column and row of the region's pixels.
    bbox: [usize; 4],
}

/// The [`Region`]s of the `count` regions whose region ids per pixel are `ids`.
fn describe(tone: &ToneMap, ids: &[u32], count: usize) -> Vec<Region> {
    let w = tone.width as usize;
    // Per region: the pixel count and the sums of x, y and ink.
    let mut sums = vec![[0.0; 4]; count];
    let mut boxes = vec![[usize::MAX, usize::MAX, 0, 0]; count];
    for (p, &r) in ids.iter().enumerate() {
        let (x, y) = (p % w, p / w);
        let sum = &mut sums[r as usize];
        sum[0] += 1.0;
        sum[1] += x as f64;
        sum[2] += y as f64;
        sum[3] += f64::from(tone.ink[p]);
        let b = &mut boxes[r as usize];
        *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    // Per region: the sums of dx^2, dy^2 and dx dy about the mean. The angle needs only their
    // ratios, so they stay unscaled.
    let mut moments = vec![[0.0; 3]; count];
    for (p, &r) in ids.iter().enumerate() {
        let sum = &sums[r as usize];
        let dx = (p % w) as f64 - sum[1] / sum[0];
        let dy = (p / w) as f64 - sum[2] / sum[0];
        let m = &mut moments[r as usize];
        *m = [m[0] + dx * dx, m[1] + dy * dy, m[2] + dx * dy];
    }
    (0..count)
        .map(|r| {
            let (n, [mu20, mu02, mu11]) = (sums[r][0], moments[r]);
            // The image's y axis points down, so the angle turns the other way on the page.
            let theta = 0.5 * math::atan2(2.0 * mu11, mu20 - mu02);
            Region {
                ink: sums[r][3] / n,
                centre: tone.to_mm([sums[r][1] / n + 0.5, sums[r][2] / n + 0.5]),
                angle_deg: -theta * 180.0 / PI,
                bbox: boxes[r],
            }
        })
        .collect()
}

/// The regions of a tone map, ready to be filled with lines.
struct Mosaic {
    tone: Arc<ToneMap>,
    /// The region of each pixel, row-major.
    ids: Vec<u32>,
    regions: Vec<Region>,
}

impl Mosaic {
    /// The region of the pixel under `p`, which lies in the bounding box `bbox`.
    fn region_at(&self, p: [f64; 2], bbox: [usize; 4]) -> u32 {
        let [px, py] = self.tone.to_px(p);
        let x = (px.floor().max(bbox[0] as f64) as usize).min(bbox[2]);
        let y = (py.floor().max(bbox[1] as f64) as usize).min(bbox[3]);
        self.ids[y * self.tone.width as usize + x]
    }

    /// Adds the paths of region `i`: lines `spacing / ink` apart at its angle turned by `turn`
    /// degrees, drawn back and forth, joined into one path where they stay in the region.
    fn draw(&self, i: usize, spacing: f64, turn: f64, paths: &mut Paths) {
        let r = &self.regions[i];
        if r.ink <= 0.0 {
            return;
        }
        let d = spacing / r.ink;
        let (dir, normal) = direction(r.angle_deg + turn);
        let [x0, y0, x1, y1] = r.bbox;
        let lo = self.tone.to_mm([x0 as f64, y0 as f64]);
        let hi = self.tone.to_mm([(x1 + 1) as f64, (y1 + 1) as f64]);
        let rect = [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]];
        // The lines lie at (k + 1/2) d from the centre along the normal, for every k that meets
        // the box.
        let c = r.centre;
        let across = |p: [f64; 2]| (p[0] - c[0]) * normal[0] + (p[1] - c[1]) * normal[1];
        let corners = [lo, [hi[0], lo[1]], [lo[0], hi[1]], hi].map(across);
        let first = corners.iter().copied().fold(f64::INFINITY, f64::min);
        let last = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let base = c[0] * normal[0] + c[1] * normal[1];
        let step = self.tone.mm_per_px / 2.0;
        let long = |&(a, b): &([f64; 2], [f64; 2])| {
            math::hypot(b[0] - a[0], b[1] - a[1]) >= MIN_RUN_PX * self.tone.mm_per_px
        };
        let mut segments: Vec<([f64; 2], [f64; 2])> = Vec::new();
        // Lines that met the region so far; every second one is drawn the other way.
        let mut met = 0;
        for k in (first / d - 0.5).ceil() as i64..=(last / d - 0.5).floor() as i64 {
            let o = base + (k as f64 + 0.5) * d;
            let Some((t0, t1)) = clip(rect, o, dir, normal) else {
                continue;
            };
            let at = |t: f64| {
                clamp_to(
                    rect,
                    [o * normal[0] + t * dir[0], o * normal[1] + t * dir[1]],
                )
            };
            // Each run of samples inside the region is one segment, from its first sample to its
            // last, unless it is a speck.
            let mut line = Vec::new();
            let mut run: Option<([f64; 2], [f64; 2])> = None;
            for p in sample_line(at(t0), at(t1), step) {
                if self.region_at(p, r.bbox) == i as u32 {
                    run = Some((run.map_or(p, |(first, _)| first), p));
                } else if let Some(seg) = run.take().filter(long) {
                    line.push(seg);
                }
            }
            line.extend(run.filter(long));
            if line.is_empty() {
                continue;
            }
            if met % 2 == 1 {
                line.reverse();
                line.iter_mut().for_each(|s| *s = (s.1, s.0));
            }
            met += 1;
            segments.extend(line);
        }
        let mut path: Vec<[f64; 2]> = Vec::new();
        for (a, b) in segments {
            if let Some(&end) = path.last() {
                let mid = [(end[0] + a[0]) / 2.0, (end[1] + a[1]) / 2.0];
                if math::hypot(a[0] - end[0], a[1] - end[1]) <= JOIN * spacing
                    && self.region_at(mid, r.bbox) == i as u32
                {
                    path.extend([a, b]);
                    continue;
                }
                paths.push(&path);
                path.clear();
            }
            path.extend([a, b]);
        }
        if !path.is_empty() {
            paths.push(&path);
        }
    }
}

/// Where a run is.
enum Phase {
    /// The first step reports the seeds.
    Seeds,
    /// The next step is pass `pass + 1`.
    Passes,
    /// The next step joins small pieces and finds the regions.
    Connect,
    /// The regions are being filled with lines.
    Lines,
}

struct MosaicRun {
    slic: Slic,
    /// The grid interval `S` in pixels.
    window: f64,
    spacing: f64,
    /// The turn of this pen's lines in degrees.
    turn: f64,
    /// Passes done.
    pass: usize,
    phase: Phase,
    /// The fill of the regions, from the connect step on.
    lines: Option<ChunkRun>,
}

impl MosaicRun {
    /// A frame holding the centres; the placement steps fill the first half of the progress.
    fn frame(&self, done: usize, label: String) -> Step {
        Step::Frame(Frame {
            stage: String::new(),
            progress: 0.5 * done as f32 / (PASSES + 2) as f32,
            label,
            points: Some(self.slic.points()),
            paths: None,
            pen: 0,
        })
    }
}

impl StageRun for MosaicRun {
    fn step(&mut self) -> Result<Step, StageError> {
        match self.phase {
            Phase::Seeds => {
                self.phase = Phase::Passes;
                Ok(self.frame(1, "seeds".into()))
            }
            Phase::Passes => {
                self.slic.pass();
                self.pass += 1;
                if self.pass == PASSES {
                    self.phase = Phase::Connect;
                }
                Ok(self.frame(1 + self.pass, format!("pass {} of {PASSES}", self.pass)))
            }
            Phase::Connect => {
                let tone = self.slic.tone.clone();
                let min = self.window * self.window / 4.0;
                let (ids, count) = connect(&self.slic.labels, tone.width as usize, min);
                let regions = describe(&tone, &ids, count);
                let mosaic = Mosaic { tone, ids, regions };
                let (spacing, turn) = (self.spacing, self.turn);
                self.lines = Some(ChunkRun::new(
                    count,
                    "regions",
                    Box::new(move |i, paths, _| mosaic.draw(i, spacing, turn, paths)),
                ));
                self.phase = Phase::Lines;
                Ok(self.frame(PASSES + 2, format!("{count} regions")))
            }
            Phase::Lines => match self
                .lines
                .as_mut()
                .expect("lines start after connect")
                .step()?
            {
                Step::Frame(f) => Ok(Step::Frame(Frame {
                    progress: 0.5 + 0.5 * f.progress,
                    ..f
                })),
                done @ Step::Done(Artifact::Paths(_)) => Ok(done),
                Step::Done(_) => unreachable!("a chunk run gives paths"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Lane;
    use crate::test_util::{
        assert_inside, bounds, run_tone_stage, run_tone_stage_in_lane, tone_map,
    };
    use serde_json::json;

    /// The 4-connected components of the pixel labels `ids` on a map `width` pixels wide, found
    /// by a flood fill that shares nothing with `connect`.
    fn components(ids: &[u32], width: usize) -> usize {
        let mut seen = vec![false; ids.len()];
        let mut count = 0;
        for start in 0..ids.len() {
            if seen[start] {
                continue;
            }
            count += 1;
            seen[start] = true;
            let mut todo = vec![start];
            while let Some(p) = todo.pop() {
                let (x, y) = (p % width, p / width);
                let mut next = Vec::new();
                if x > 0 {
                    next.push(p - 1);
                }
                if x + 1 < width {
                    next.push(p + 1);
                }
                if y > 0 {
                    next.push(p - width);
                }
                if p + width < ids.len() {
                    next.push(p + width);
                }
                for q in next {
                    if !seen[q] && ids[q] == ids[start] {
                        seen[q] = true;
                        todo.push(q);
                    }
                }
            }
        }
        count
    }

    /// The labels after ten passes, the regions of `tone`, how many there are and `S`.
    fn regions(tone: ToneMap, region_mm: f64, m: f64) -> (Vec<u32>, Vec<u32>, usize, f64) {
        let s = (region_mm / tone.mm_per_px).max(2.0);
        let width = tone.width as usize;
        let mut slic = Slic::new(Arc::new(tone), s, m);
        for _ in 0..PASSES {
            slic.pass();
        }
        let (ids, count) = connect(&slic.labels, width, s * s / 4.0);
        (slic.labels, ids, count, s)
    }

    /// The segments of `paths`: every path is a run of two-vertex segments.
    fn segments(paths: &Paths) -> Vec<([f64; 2], [f64; 2])> {
        let pair = |s: &[[f64; 2]]| (s[0], s[1]);
        paths.iter().flat_map(|p| p.chunks(2).map(pair)).collect()
    }

    /// A map `w` by `h` pixels with full ink where `band` holds and none elsewhere.
    fn band_map(w: u32, h: u32, band: impl Fn(u32, u32) -> bool) -> ToneMap {
        tone_map(w, h, |x, y| if band(x, y) { 1.0 } else { 0.0 })
    }

    #[test]
    fn settings_schema() {
        let s = MosaicStage.schema();
        assert_eq!(bounds(&s, "region_mm"), (2.0, 50.0, 8.0));
        assert_eq!(bounds(&s, "compactness"), (1.0, 40.0, 3.0));
        assert_eq!(bounds(&s, "spacing_mm"), (0.2, 10.0, 0.6));
        let p = &s["properties"];
        assert_eq!(p["spacing_mm"]["x-density"], "inverse");
        assert_eq!(p["spacing_mm"]["x-density-of"], "lines");
        assert!(
            p["region_mm"].get("x-density").is_none()
                && p["compactness"].get("x-density").is_none()
        );
        assert_eq!(p["region_mm"]["x-step"], 0.5);
        assert_eq!(p["compactness"]["x-step"], 1.0);
        assert_eq!(p["spacing_mm"]["x-step"], 0.1);
        for (key, help) in [
            ("region_mm", "About how wide one patch is."),
            (
                "compactness",
                "Higher keeps the patches round; lower lets them follow the image's shapes.",
            ),
            (
                "spacing_mm",
                "Distance between lines over full ink; lighter patches space theirs wider.",
            ),
        ] {
            assert_eq!(p[key]["description"], help, "{key}");
        }
        let m = MosaicStage;
        assert_eq!((m.name(), m.label()), ("mosaic", "Mosaic"));
        assert_eq!(
            (m.requires(), m.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn a_seed_moves_to_the_first_lowest_gradient() {
        // 12 by 12 pixels with S = 6 give seeds at 3 and 9. Ink steps up at x = 3, so the gradient is
        // 1 at x = 2 and 3 and 0 at x = 4; the 3 by 3 neighbourhood of the first seed reads its top
        // row first.
        let tone = tone_map(12, 12, |x, _| if x < 3 { 0.0 } else { 1.0 });
        let slic = Slic::new(Arc::new(tone), 6.0, 10.0);
        assert_eq!(slic.centres.len(), 4);
        assert_eq!(slic.centres[0], [4.0, 2.0, 1.0]);
        // The others sit on flat ink: the first pixel of their neighbourhood is the winner.
        assert_eq!(slic.centres[1], [8.0, 2.0, 1.0]);
    }

    #[test]
    fn no_region_crosses_a_sharp_edge() {
        // Black up to x = 18: the edge falls inside a grid cell.
        let tone = tone_map(40, 30, |x, _| if x < 18 { 1.0 } else { 0.0 });
        let (_, ids, count, _) = regions(tone, 8.0, 10.0);
        assert!(count > 20, "{count} regions");
        let mut black = vec![None; count];
        for (p, &r) in ids.iter().enumerate() {
            let is_black = p % 40 < 18;
            assert_eq!(
                *black[r as usize].get_or_insert(is_black),
                is_black,
                "region {r}"
            );
        }
    }

    #[test]
    fn regions_are_connected_and_not_tiny() {
        // Ragged ink with a low compactness leaves SLIC with disjoint pieces to join.
        let ink = |x: u32, y: u32| ((x * 7 + y * 13 + x * y % 5) % 11) as f32 / 10.0;
        let (labels, ids, count, s) = regions(tone_map(60, 45, ink), 8.0, 1.0);
        assert!(components(&labels, 60) > count, "nothing to join");
        assert_eq!(components(&ids, 60), count);
        let mut size = vec![0; count];
        for &r in &ids {
            size[r as usize] += 1;
        }
        assert!(
            count > 1 && size.iter().all(|&n| n as f64 >= s * s / 4.0),
            "{size:?}"
        );
        // Numbered in order of the first pixel.
        let mut firsts: Vec<usize> = (0..count)
            .map(|r| ids.iter().position(|&i| i == r as u32).unwrap())
            .collect();
        let sorted = firsts.clone();
        firsts.sort_unstable();
        assert_eq!(firsts, sorted);
    }

    #[test]
    fn a_small_piece_joins_its_biggest_neighbour() {
        // Two rows of labels 0 0 1 2 2 2 2: the one column of label 1 is under the floor of 3
        // pixels and sits between 4 pixels and 8, so it joins the 8.
        let labels = [0, 0, 1, 2, 2, 2, 2, 0, 0, 1, 2, 2, 2, 2];
        let (ids, count) = connect(&labels, 7, 3.0);
        assert_eq!((count, ids[..7].to_vec()), (2, vec![0, 0, 1, 1, 1, 1, 1]));
        // Between two of 4 pixels it joins the lower numbered.
        let labels = [0, 0, 1, 2, 2, 0, 0, 1, 2, 2];
        let (ids, count) = connect(&labels, 5, 3.0);
        assert_eq!((count, ids[..5].to_vec()), (2, vec![0, 0, 0, 1, 1]));
    }

    #[test]
    fn a_flat_map_gives_a_grid_of_regions() {
        // 80 by 60 pixels at 2 mm; patches of 8 mm are 4 pixels across.
        let (_, _, count, _) = regions(tone_map(80, 60, |_, _| 0.5), 8.0, 10.0);
        assert!(
            (270..=330).contains(&count),
            "{count} regions, expected about 300"
        );
    }

    #[test]
    fn white_gives_no_paths_and_the_frames_show_the_work() {
        let (frames, paths) = run_tone_stage(&MosaicStage, tone_map(40, 30, |_, _| 0.0), json!({}));
        assert!(paths.is_empty());
        let labels: Vec<&str> = frames.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels[0], "seeds");
        assert_eq!(labels[1], "pass 1 of 10");
        assert_eq!(labels[10], "pass 10 of 10");
        assert!(
            labels[11].ends_with(" regions") && labels[12].ends_with(" regions"),
            "{labels:?}"
        );
        assert!(frames.windows(2).all(|w| w[0].progress <= w[1].progress));
        // The centres are points in mm on the map.
        let rect = [10.0, 10.0, 80.0, 60.0];
        for f in &frames[..12] {
            let points = f.points.as_ref().unwrap();
            assert!(
                points.0.len() > 10
                    && points
                        .0
                        .iter()
                        .all(|&p| crate::stages::lines::inside(rect, p))
            );
        }
    }

    #[test]
    fn full_ink_spaces_the_lines_by_the_setting() {
        // Patches of 20 mm on 80 mm: sixteen, each drawn back and forth as one path.
        let tone = tone_map(40, 40, |_, _| 1.0);
        let (_, paths) = run_tone_stage(
            &MosaicStage,
            tone,
            json!({"region_mm": 20.0, "spacing_mm": 1.0}),
        );
        assert_eq!(paths.len(), 16);
        for path in paths.iter() {
            let lines: Vec<&[[f64; 2]]> = path.chunks(2).collect();
            assert!(lines.len() > 10);
            for pair in lines.windows(2) {
                let (a, b, c) = (pair[0][0], pair[0][1], pair[1][0]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let off = ((c[0] - a[0]) * dy - (c[1] - a[1]) * dx).abs() / math::hypot(dx, dy);
                assert!((off - 1.0).abs() < 1e-6, "{off} mm between lines");
            }
        }
    }

    #[test]
    fn a_light_patch_keeps_its_lines_loose() {
        // Spacing 1 mm joins lines up to 2 mm apart. At ink 0.8 they are 1.25 mm apart, so each
        // patch is one back and forth path; at 0.4 they are 2.5 mm apart, so each is a stroke.
        let paths = |ink: f32| {
            let tone = tone_map(40, 40, move |_, _| ink);
            let patch = json!({"region_mm": 20.0, "spacing_mm": 1.0});
            run_tone_stage(&MosaicStage, tone, patch).1
        };
        assert_eq!(paths(0.8).len(), 16);
        let light = paths(0.4);
        assert!(light.len() > 64 && light.iter().all(|p| p.len() == 2));
    }

    #[test]
    fn a_light_patch_is_drawn_light_not_with_one_line() {
        // A patch of 8 mm over ink 0.1 would space its lines 6 mm from the centre out, so it has none.
        let length = |ink: f32| {
            let tone = tone_map(40, 40, move |_, _| ink);
            run_tone_stage(&MosaicStage, tone, json!({"spacing_mm": 0.6}))
                .1
                .total_length()
        };
        assert_eq!(length(0.05), 0.0);
        let (mid, dark) = (length(0.3), length(0.9));
        assert!(
            mid > 0.0 && dark > 2.0 * mid,
            "{mid} mm at 0.3 ink, {dark} mm at 0.9"
        );
    }

    #[test]
    fn a_region_elongated_along_x_draws_its_lines_along_x() {
        let band = band_map(60, 30, |_, y| (12..18).contains(&y));
        let patch = json!({"region_mm": 20.0, "compactness": 1.0, "spacing_mm": 1.0});
        let (_, paths) = run_tone_stage(&MosaicStage, band, patch);
        let lines = segments(&paths);
        assert!(lines.len() > 30);
        for (a, b) in lines {
            assert!(
                (b[1] - a[1]).abs() < 0.25 * (b[0] - a[0]).abs(),
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn angles_turn_counter_clockwise_on_the_page() {
        // A band from the top left down to the bottom right: y grows with x, so the lines run
        // down and to the right on the page, at about 45 degrees.
        let band = band_map(40, 40, |x, y| x.abs_diff(y) <= 2);
        let patch = json!({"region_mm": 16.0, "compactness": 1.0, "spacing_mm": 1.0});
        let (_, paths) = run_tone_stage(&MosaicStage, band, patch);
        let lines = segments(&paths);
        assert!(lines.len() > 10);
        for (a, b) in lines {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            assert!(
                dx * dy > 0.0 && (0.5..2.0).contains(&(dy / dx)),
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn a_second_pen_turns_the_lines_a_quarter() {
        let band = || band_map(60, 30, |_, y| (12..18).contains(&y));
        let patch = json!({"region_mm": 20.0, "compactness": 1.0, "spacing_mm": 1.0});
        let (_, alone) = run_tone_stage(&MosaicStage, band(), patch.clone());
        let first = Lane { index: 0, count: 2 };
        let (_, one) = run_tone_stage_in_lane(&MosaicStage, band(), patch.clone(), first);
        assert_eq!(one.coords(), alone.coords());
        let second = Lane { index: 1, count: 2 };
        let (_, two) = run_tone_stage_in_lane(&MosaicStage, band(), patch, second);
        let lines = segments(&two);
        assert!(lines.len() > 30);
        for (a, b) in lines {
            assert!(
                (b[0] - a[0]).abs() < 1e-6 && (b[1] - a[1]).abs() > 0.5,
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn a_line_that_only_grazes_a_region_draws_nothing() {
        // The map is 2 mm a pixel, so no dash may be under 4 mm; ragged ink makes many corners.
        let ink = |x: u32, y: u32| ((x * 7 + y * 13 + x * y % 5) % 11) as f32 / 10.0;
        let patch = json!({"region_mm": 8.0, "compactness": 1.0, "spacing_mm": 0.5});
        let (_, paths) = run_tone_stage(&MosaicStage, tone_map(40, 30, ink), patch);
        let lines = segments(&paths);
        assert!(lines.len() > 100);
        for (a, b) in lines {
            assert!(
                math::hypot(b[0] - a[0], b[1] - a[1]) >= 4.0 - 1e-9,
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        let ink = |x: u32, y: u32| ((x * 7 + y * 13 + x * y % 5) % 11) as f32 / 10.0;
        let rect = [10.0, 10.0, 80.0, 60.0];
        for patch in [
            json!({}),
            json!({"region_mm": 2.0, "compactness": 1.0, "spacing_mm": 0.2}),
        ] {
            let (_, paths) = run_tone_stage(&MosaicStage, tone_map(40, 30, ink), patch);
            assert!(!paths.is_empty());
            assert_inside(&paths, rect);
        }
    }

    #[test]
    fn the_same_inputs_give_the_same_paths() {
        let run = || {
            let ink = |x: u32, y: u32| ((x * 5 + y * 3) % 17) as f32 / 16.0;
            run_tone_stage(&MosaicStage, tone_map(40, 30, ink), json!({})).1
        };
        assert_eq!(run(), run());
    }
}
