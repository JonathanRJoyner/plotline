//! Plot lint: physical problems with a finished drawing, each with a setting patch that fixes it.
//!
//! [`measure`] looks at a drawing once, when its run ends: the pen passes over each spot from the
//! [`coverage`] raster, the short paths and segments, and the vertices off the page. [`check`]
//! turns that into findings for the session as it stands, so a patch starts from the current
//! settings. A finding's patch has the shape of an image analysis suggestion's
//! ([`crate::analysis::Suggestion`]), so the front ends apply both the same way. The thresholds
//! are hand-set taste defaults, recorded in the brief.

use serde::Serialize;
use serde_json::{json, Value};

use crate::coverage::{self, Coverage};
use crate::machine::Machine;
use crate::stats::{self, Speeds};
use crate::style::Style;
use crate::types::{Drawing, Paths, Pen};

/// A pen passing a spot this often risks the paper; over `OVERDRAW_SHARE` of the inked cells
/// passed that often is overdraw. Two passes are normal where hatching crosses and fill rings meet.
const OVERDRAW_PASSES: u8 = 3;
const OVERDRAW_SHARE: f64 = 0.05;
/// Short paths are specks when there are at least this many and this share of the paths.
const SPECKS: usize = 10;
const SPECK_SHARE: f64 = 0.01;
/// Below this share of the drawing area inked, a drawing is nearly empty.
const NEARLY_EMPTY: f64 = 0.001;
/// Tone at least this dark counts as the image's dark parts; under this share of the map they are
/// too small to judge.
const DARK_TONE: f32 = 0.75;
const DARK_AREA: f64 = 0.01;
/// Dark parts covered under this share cannot reach dark; the spacing patch aims for the target.
const DARK_COVERAGE: f64 = 0.35;
const DARK_TARGET: f64 = 0.5;
/// Pen lifts over this share of the plot time are lift-heavy, travel over this one travel-heavy.
const LIFT_SHARE: f64 = 0.5;
const TRAVEL_SHARE: f64 = 0.3;
/// A segment shorter than this fraction of the pen is finer than the pen can show; a layer with
/// at least `FINE_MIN` segments, over half of them that fine, holds needless vertices.
const FINE: f64 = 1.0 / 8.0;
const FINE_MIN: usize = 1000;

/// What one layer's pen lays down.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerMeasure {
    /// Cells the pen passes at least once, and at least [`OVERDRAW_PASSES`] times.
    pub inked: usize,
    pub overdrawn: usize,
    /// The most passes over one cell.
    pub max_passes: u8,
    pub paths: usize,
    /// One-vertex paths.
    pub dots: usize,
    /// Paths shorter than the pen width, dots included.
    pub short: usize,
    pub segments: usize,
    /// Segments shorter than [`FINE`] of the pen width.
    pub fine_segments: usize,
}

/// What [`measure`] finds in a drawing; it depends on the drawing alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
    /// The raster's cell size in mm.
    pub cell_mm: f64,
    /// One entry per layer, in drawing order.
    pub layers: Vec<LayerMeasure>,
    /// The share of the drawing area any layer inks.
    pub inked_share: f64,
    /// The share any layer inks of the cells whose nearest tone-map pixel holds [`DARK_TONE`] or
    /// more; `None` without a tone map or when such cells are under [`DARK_AREA`] of its area.
    pub dark_coverage: Option<f64>,
    /// Vertices outside the page, and vertices past the drawing area by more than half their pen.
    pub off_page: usize,
    pub off_area: usize,
}

/// Measure `d` on one raster over the page, with the cell size of its thinnest pen, cleared
/// between layers. See [`Measure`].
pub fn measure(d: &Drawing) -> Measure {
    let page = [0.0, 0.0, d.canvas.width_mm, d.canvas.height_mm];
    let [ax, ay, aw, ah] = d.canvas.drawing_rect();
    let thinnest = d.layers.iter().map(|l| l.pen.width_mm).reduce(f64::min);
    let cell = coverage::cell_size(thinnest.unwrap_or(0.3), page);
    let mut grid = Coverage::new(page, cell);
    let mut union = vec![false; grid.count.len()];
    let (mut off_page, mut off_area) = (0, 0);
    let mut layers = Vec::new();
    for layer in &d.layers {
        let w = layer.pen.width_mm;
        grid.count.fill(0);
        grid.stamp(&layer.paths, w);
        let (mut inked, mut overdrawn, mut max_passes) = (0, 0, 0);
        for (&n, any) in grid.count.iter().zip(union.iter_mut()) {
            inked += usize::from(n > 0);
            overdrawn += usize::from(n >= OVERDRAW_PASSES);
            max_passes = max_passes.max(n);
            *any |= n > 0;
        }
        let half = w / 2.0;
        for &[x, y] in layer.paths.coords() {
            off_page += usize::from(!(0.0..=page[2]).contains(&x) || !(0.0..=page[3]).contains(&y));
            let past = (ax - half..=ax + aw + half).contains(&x)
                && (ay - half..=ay + ah + half).contains(&y);
            off_area += usize::from(!past);
        }
        let (mut dots, mut short, mut segments, mut fine_segments) = (0, 0, 0, 0);
        for p in layer.paths.iter() {
            dots += usize::from(p.len() == 1);
            short += usize::from(Paths::path_length(p) < w);
            for s in p.windows(2) {
                segments += 1;
                fine_segments += usize::from(Paths::path_length(s) < w * FINE);
            }
        }
        layers.push(LayerMeasure {
            inked,
            overdrawn,
            max_passes,
            paths: layer.paths.len(),
            dots,
            short,
            segments,
            fine_segments,
        });
    }
    let inked_area = union.iter().filter(|&&b| b).count() as f64 * cell * cell;
    Measure {
        cell_mm: cell,
        layers,
        inked_share: inked_area / (aw * ah),
        dark_coverage: d.tone.as_deref().and_then(|t| {
            // The tone-map column or row under each cell centre's, if any.
            let under = |cells: usize, origin: f64, pixels: u32| -> Vec<Option<usize>> {
                (0..cells)
                    .map(|i| {
                        let p = ((i as f64 + 0.5) * cell - origin) / t.mm_per_px;
                        (0.0..f64::from(pixels)).contains(&p).then_some(p as usize)
                    })
                    .collect()
            };
            let xs = under(grid.width, t.origin_mm[0], t.width);
            let ys = under(grid.height, t.origin_mm[1], t.height);
            let (mut tone_cells, mut dark, mut hit) = (0usize, 0usize, 0usize);
            for (row, y) in ys.iter().enumerate() {
                let (Some(y), cells) = (y, &union[row * grid.width..(row + 1) * grid.width]) else {
                    continue;
                };
                let ink = &t.ink[y * t.width as usize..];
                for (x, &any) in xs.iter().zip(cells) {
                    let Some(x) = x else { continue };
                    tone_cells += 1;
                    if ink[*x] >= DARK_TONE {
                        dark += 1;
                        hit += usize::from(any);
                    }
                }
            }
            (dark > 0 && dark as f64 >= DARK_AREA * tone_cells as f64)
                .then(|| hit as f64 / dark as f64)
        }),
        off_page,
        off_area,
    }
}

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The plot would go wrong: off the paper or the machine.
    Error,
    /// The plot would waste ink, paper or time.
    Warning,
    /// Worth knowing.
    Info,
}

/// A physical problem with the drawing, the number the rule tested, and a fix if a setting has
/// one. `patch` holds any of `style`, `settings` (`{stage: {key: value}}`) and `page`
/// (`{width_mm, height_mm}`), or is `None` for a note.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub value: f64,
    pub patch: Option<Value>,
}

/// `v` rounded up, or down, to the 0.1 step of the length settings.
fn step_up(v: f64) -> f64 {
    (v * 10.0 - 1e-9).ceil() / 10.0
}
fn step_down(v: f64) -> f64 {
    (v * 10.0 + 1e-9).floor() / 10.0
}

/// The settings and schemas of the session the findings are for.
struct Context<'a> {
    style: &'a Style,
    settings: &'a Value,
}

impl Context<'_> {
    fn number(&self, stage: &str, key: &str) -> Option<f64> {
        self.settings.get(stage)?.get(key)?.as_f64()
    }

    /// The first stage, in run order, with a number setting `key` among `keys`.
    fn find(&self, keys: &[&'static str]) -> Option<(&'static str, &'static str, f64)> {
        self.style.stages.iter().find_map(|s| {
            let stage = s.name();
            keys.iter()
                .find_map(|&key| Some((stage, key, self.number(stage, key)?)))
        })
    }

    /// A patch setting `stage.key` to `v` clamped into the schema's range, as an integer when the
    /// schema asks for one, or `None` when that is the current value.
    fn set(&self, stage: &str, key: &str, v: f64) -> Option<Value> {
        let index = self.style.stage_index(stage)?;
        let schema = &self.style.stages[index].schema()["properties"][key];
        let bound = |k: &str, or: f64| schema[k].as_f64().unwrap_or(or);
        let v = v.clamp(bound("minimum", f64::MIN), bound("maximum", f64::MAX));
        if Some(v) == self.number(stage, key) {
            return None;
        }
        let v = if schema["type"] == "integer" {
            json!(v as i64)
        } else {
            json!(v)
        };
        Some(json!({"settings": {stage: {key: v}}}))
    }
}

const SPACING: &[&str] = &["spacing_mm", "min_spacing_mm"];

/// The findings for drawing `d` and its measurement `m`, given the session's current style,
/// settings, pens, speeds and machine, in a fixed order. See the plot lint spec for the rules.
pub fn check(
    d: &Drawing,
    m: &Measure,
    style: &Style,
    settings: &Value,
    pens: &[Pen],
    speeds: Speeds,
    machine: Option<&Machine>,
) -> Vec<Finding> {
    let cx = Context { style, settings };
    let mut out = Vec::new();
    let mut add = |code, severity, message: String, value: f64, patch: Option<Value>| {
        out.push(Finding {
            code,
            severity,
            message,
            value,
            patch,
        })
    };
    let widest = d.layers.iter().map(|l| l.pen.width_mm).fold(0.0, f64::max);
    let paths: usize = m.layers.iter().map(|l| l.paths).sum();
    let percent = |share: f64| (share * 100.0).round();

    let empty = paths == 0 || m.inked_share < NEARLY_EMPTY;
    if empty {
        let threshold = cx.find(&["threshold"]);
        let patch = threshold
            .and_then(|(stage, key, t)| cx.set(stage, key, (t * 50.0).floor() / 100.0))
            .or_else(|| {
                (cx.settings["tone"]["levels"] == "manual")
                    .then(|| json!({"settings": {"tone": {"levels": "auto"}}}))
            });
        let what = if paths == 0 {
            "No marks at these settings.".to_string()
        } else {
            format!(
                "The drawing inks under {} % of the drawing area.",
                NEARLY_EMPTY * 100.0
            )
        };
        let message =
            format!("{what} Try a lower threshold, or the tone's black, white and gamma settings.");
        add("empty", Severity::Warning, message, m.inked_share, patch);
    }
    if m.off_page > 0 {
        let message = format!(
            "{} vertices lie off the page, where the pen would leave the paper.",
            m.off_page
        );
        add(
            "off_page",
            Severity::Error,
            message,
            m.off_page as f64,
            None,
        );
    }
    if let Some(machine) = machine {
        let (w, h) = (d.canvas.width_mm, d.canvas.height_mm);
        if let Err(e) = machine.mapping(w, h) {
            let over = (w - machine.area.width).max(h - machine.area.height);
            // The page turned may fit.
            let turned = machine
                .mapping(h, w)
                .is_ok()
                .then(|| json!({"page": {"width_mm": h, "height_mm": w}}));
            add(
                "off_machine",
                Severity::Error,
                format!("{e}."),
                over,
                turned,
            );
        }
    }
    if m.off_page == 0 && m.off_area > 0 {
        let message = format!(
            "{} vertices lie in the margin, past the drawing area by more than half the pen width.",
            m.off_area
        );
        add(
            "off_area",
            Severity::Warning,
            message,
            m.off_area as f64,
            None,
        );
    }
    let spacing = cx.find(SPACING);
    let close = spacing.filter(|&(_, _, s)| s < widest - 1e-9);
    if let Some((stage, key, s)) = close {
        let message =
            format!("Marks {s} mm apart are closer than the {widest} mm pen, so they overlap.");
        add(
            "close_lines",
            Severity::Warning,
            message,
            s,
            cx.set(stage, key, step_up(widest)),
        );
    }
    let worst = (d.layers.iter().zip(&m.layers))
        .filter(|(_, l)| l.inked > 0)
        .map(|(layer, l)| (layer, l, l.overdrawn as f64 / l.inked as f64))
        .max_by(|a, b| a.2.total_cmp(&b.2));
    if let Some((layer, l, share)) = worst.filter(|w| w.2 > OVERDRAW_SHARE && close.is_none()) {
        let levels = cx.number("hatch", "levels").filter(|&n| n > 1.0);
        let patch = match (levels, spacing) {
            (Some(n), _) => cx.set("hatch", "levels", n - 1.0),
            (None, Some((stage, key, s))) => cx.set(stage, key, step_up(s * 1.5)),
            _ => None,
        };
        let message = format!(
            "{} % of what {} inks is drawn over {OVERDRAW_PASSES} or more times, up to {}, which wastes ink and can wear through the paper.",
            percent(share),
            layer.pen.name,
            l.max_passes
        );
        add("overdraw", Severity::Warning, message, share, patch);
    }
    let dots: usize = m.layers.iter().map(|l| l.dots).sum();
    let short: usize = m.layers.iter().map(|l| l.short).sum();
    if dots * 2 < paths && short >= SPECKS && short as f64 >= SPECK_SHARE * paths as f64 {
        let min = cx.number("order", "min_length_mm").unwrap_or(0.0);
        let patch = cx.set("order", "min_length_mm", step_up(widest).max(min));
        let message = format!(
            "{short} paths are shorter than the pen is wide; each costs a pen lift for a speck."
        );
        add("specks", Severity::Warning, message, short as f64, patch);
    }
    if let Some(c) = m.dark_coverage.filter(|&c| c < DARK_COVERAGE && !empty) {
        // Coverage grows about in proportion to the marks: closer lines, or more points or strokes.
        let closer = spacing.and_then(|(stage, key, s)| {
            let s2 = step_down(s * c / DARK_TARGET).max(step_up(widest));
            (s2 < s).then(|| cx.set(stage, key, s2)).flatten()
        });
        let patch = closer.or_else(|| {
            let (stage, key, n) = cx.find(&["count", "max_lines"])?;
            cx.set(stage, key, (n * DARK_TARGET / c).round())
        });
        let message = format!(
            "Where the image is darkest, the drawing covers only {} % of the paper.",
            percent(c)
        );
        add("dark", Severity::Info, message, c, patch);
    }
    let st = stats::compute(d, speeds);
    if st.estimated_seconds > 0.0 {
        let lifts = paths as f64 * speeds.pen_lift_s / st.estimated_seconds;
        if lifts > LIFT_SHARE {
            let message = format!(
                "Pen lifts take {} % of the plot time ({paths} lifts). Styles that draw long strokes, such as waves, spiral and single line, lift far less.",
                percent(lifts)
            );
            add("lifts", Severity::Info, message, lifts, None);
        }
        let travel = st.pen_up_mm / speeds.travel_mm_s / st.estimated_seconds;
        if travel > TRAVEL_SHARE {
            let optimized = cx.settings["order"]["optimize"] != false;
            let mut message = format!(
                "Pen-up travel takes {} % of the plot time.",
                percent(travel)
            );
            let (severity, patch) = if optimized {
                (Severity::Info, None)
            } else {
                message += " Path order's optimize setting cuts it.";
                let patch = json!({"settings": {"order": {"optimize": true}}});
                (Severity::Warning, Some(patch))
            };
            add("travel", severity, message, travel, patch);
        }
    }
    let fine = (m.layers.iter())
        .filter(|l| l.segments >= FINE_MIN)
        .map(|l| l.fine_segments as f64 / l.segments as f64)
        .fold(0.0, f64::max);
    if fine > 0.5 {
        let message = format!(
            "{} % of the segments are shorter than an eighth of the pen width, so the paths hold more vertices than the pen can show.",
            percent(fine)
        );
        add("vertices", Severity::Info, message, fine, None);
    }
    let mode = cx.settings["split"]["mode"].as_str().unwrap_or("one_pen");
    if mode == "one_pen" && pens.len() > 1 {
        let patch = json!({"settings": {"split": {"mode": "light_to_dark"}}});
        let message = "Pen use is set to one pen, so only the first pen draws.".to_string();
        add(
            "unused_pens",
            Severity::Info,
            message,
            (pens.len() - 1) as f64,
            Some(patch),
        );
    } else if mode != "one_pen" {
        let idle: Vec<&str> = (d.layers.iter())
            .filter(|l| l.paths.is_empty())
            .map(|l| l.pen.name.as_str())
            .collect();
        if !idle.is_empty() {
            let message = format!(
                "These pens draw nothing at these settings: {}.",
                idle.join(", ")
            );
            add(
                "unused_pens",
                Severity::Info,
                message,
                idle.len() as f64,
                None,
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::registry::get_style;
    use crate::session::Session;
    use crate::synthetic::still_life;
    use crate::test_util::{drawing_100, drawing_100_with_tone, drawing_with, machine};
    use crate::types::{Canvas, Fit, Layer, ToneMap};

    /// The findings for `d` under `style` at its defaults changed by `change`, the drawing's pens,
    /// the default speeds and no machine.
    fn lint(d: &Drawing, style: &str, change: impl Fn(&mut Value)) -> Vec<Finding> {
        let style = get_style(style).unwrap();
        let mut settings = style.defaults();
        change(&mut settings);
        let pens: Vec<Pen> = d.layers.iter().map(|l| l.pen.clone()).collect();
        check(
            d,
            &measure(d),
            &style,
            &settings,
            &pens,
            Speeds::default(),
            None,
        )
    }

    fn find<'a>(all: &'a [Finding], code: &str) -> Option<&'a Finding> {
        all.iter().find(|f| f.code == code)
    }

    /// Whether `style` at its defaults finds `code` in `d`.
    fn finds(d: &Drawing, style: &str, code: &str) -> bool {
        find(&lint(d, style, |_| {}), code).is_some()
    }

    /// One pen 0.3 mm wide on a 100 mm page with no margin.
    fn page(paths: Vec<Vec<[f64; 2]>>) -> Drawing {
        drawing_100(paths, 0.3, "#000000")
    }

    /// Lines across a 100 mm page `gap` mm apart.
    fn lines(gap: f64) -> Vec<Vec<[f64; 2]>> {
        (1..)
            .map(|i| i as f64 * gap)
            .take_while(|&y| y < 100.0)
            .map(|y| vec![[1.0, y], [99.0, y]])
            .collect()
    }

    /// `n` dots in a row 0.9 mm apart.
    fn dots(n: usize) -> Vec<Vec<[f64; 2]>> {
        (0..n).map(|i| vec![[1.0 + i as f64 * 0.9, 50.0]]).collect()
    }

    #[test]
    fn an_empty_drawing_suggests_a_lower_threshold_then_auto_levels() {
        let d = page(vec![]);
        let all = lint(&d, "grid", |_| {});
        let empty = find(&all, "empty").unwrap();
        assert_eq!(empty.severity, Severity::Warning);
        assert!(empty.message.starts_with("No marks at these settings."));
        let patch = json!({"settings": {"points": {"threshold": 0.25}}});
        assert_eq!(empty.patch, Some(patch));
        let all = lint(&d, "grid", |s| s["points"]["threshold"] = json!(0.0));
        let patch = json!({"settings": {"tone": {"levels": "auto"}}});
        assert_eq!(find(&all, "empty").unwrap().patch, Some(patch));
        let all = lint(&d, "grid", |s| {
            s["points"]["threshold"] = json!(0.0);
            s["tone"]["levels"] = json!("auto");
        });
        assert_eq!(find(&all, "empty").unwrap().patch, None);
        // The ink threshold stops at its schema's least value.
        let all = lint(&d, "ink", |s| s["ink"]["threshold"] = json!(0.06));
        let patch = json!({"settings": {"ink": {"threshold": 0.05}}});
        assert_eq!(find(&all, "empty").unwrap().patch, Some(patch));
    }

    #[test]
    fn a_speck_of_ink_is_nearly_empty() {
        let speck = drawing_with(vec![vec![[50.0, 50.0], [51.0, 50.0]]]);
        let all = lint(&speck, "hatch", |_| {});
        assert!(find(&all, "empty")
            .unwrap()
            .message
            .starts_with("The drawing inks under 0.1 % of"));
        assert!(!finds(&page(lines(1.0)), "hatch", "empty"));
    }

    #[test]
    fn vertices_off_the_page_and_in_the_margin() {
        let all = lint(&page(vec![vec![[-1.0, 5.0], [5.0, 5.0]]]), "hatch", |_| {});
        let off = find(&all, "off_page").unwrap();
        assert_eq!((off.severity, off.value), (Severity::Error, 1.0));
        assert!(find(&all, "off_area").is_none());
        // A4 has a 10 mm margin; a vertex within half the pen of the drawing area is not past it.
        let d = drawing_with(vec![vec![[5.0, 50.0], [9.9, 50.0], [100.0, 50.0]]]);
        let all = lint(&d, "hatch", |_| {});
        assert_eq!(find(&all, "off_area").unwrap().value, 1.0);
        assert!(find(&all, "off_page").is_none());
    }

    #[test]
    fn a_page_larger_than_the_machine_is_an_error() {
        let mut d = page(lines(1.0));
        let style = get_style("hatch").unwrap();
        let settings = style.defaults();
        let pens = [d.layers[0].pen.clone()];
        let m = machine();
        let all = |d: &Drawing| {
            check(
                d,
                &measure(d),
                &style,
                &settings,
                &pens,
                Speeds::default(),
                Some(&m),
            )
        };
        assert!(find(&all(&d), "off_machine").is_none());
        // The area is 200 by 174 mm: 150 by 190 does not fit, and turned it does.
        d.canvas = Canvas::new(150.0, 190.0, 0.0, Fit::Fit).unwrap();
        let found = all(&d);
        let off = find(&found, "off_machine").unwrap();
        assert_eq!((off.severity, off.value), (Severity::Error, 16.0));
        assert!(off.message.contains("200 by 174 mm"), "{}", off.message);
        let turned = json!({"page": {"width_mm": 190.0, "height_mm": 150.0}});
        assert_eq!(off.patch, Some(turned));
        d.canvas = Canvas::new(180.0, 190.0, 0.0, Fit::Fit).unwrap();
        assert_eq!(find(&all(&d), "off_machine").unwrap().patch, None);
    }

    #[test]
    fn marks_closer_than_the_pen_overlap() {
        let d = page(lines(1.0));
        let all = lint(&d, "hatch", |s| s["hatch"]["spacing_mm"] = json!(0.2));
        let close = find(&all, "close_lines").unwrap();
        assert_eq!(close.value, 0.2);
        let patch = json!({"settings": {"hatch": {"spacing_mm": 0.3}}});
        assert_eq!(close.patch, Some(patch));
        assert!(!finds(&d, "hatch", "close_lines"));
        // Flow's spacing key is its least spacing.
        let wide = drawing_100(lines(1.0), 0.8, "#000000");
        let all = lint(&wide, "flow", |s| s["flow"]["min_spacing_mm"] = json!(0.5));
        let patch = json!({"settings": {"flow": {"min_spacing_mm": 0.8}}});
        assert_eq!(find(&all, "close_lines").unwrap().patch, Some(patch));
    }

    #[test]
    fn three_passes_over_a_twentieth_of_the_ink_is_overdraw() {
        let d = page((0..4).flat_map(|_| lines(2.0)).collect());
        let all = lint(&d, "hatch", |_| {});
        let over = find(&all, "overdraw").unwrap();
        assert_eq!(over.value, 1.0);
        assert!(over.message.contains("up to 4"), "{}", over.message);
        let patch = json!({"settings": {"hatch": {"levels": 2}}});
        assert_eq!(over.patch, Some(patch));
        let all = lint(&d, "hatch", |s| s["hatch"]["levels"] = json!(1));
        let patch = json!({"settings": {"hatch": {"spacing_mm": 1.5}}});
        assert_eq!(find(&all, "overdraw").unwrap().patch, Some(patch));
        assert_eq!(
            find(&lint(&d, "ink", |_| {}), "overdraw").unwrap().patch,
            None
        );
        // Two passes, as where hatching crosses, are not overdraw.
        let twice = page((0..2).flat_map(|_| lines(2.0)).collect());
        assert!(!finds(&twice, "hatch", "overdraw"));
        // Lines closer than the pen explain the overdraw, so only that is said.
        let all = lint(&d, "hatch", |s| s["hatch"]["spacing_mm"] = json!(0.2));
        assert!(find(&all, "close_lines").is_some() && find(&all, "overdraw").is_none());
    }

    #[test]
    fn short_paths_are_specks_unless_the_style_draws_dots() {
        let mut paths = lines(5.0);
        let x = |i: usize| 10.0 + i as f64;
        paths.extend((0..20).map(|i| vec![[x(i), 50.5], [x(i) + 0.1, 50.5]]));
        let all = lint(&page(paths), "hatch", |_| {});
        let specks = find(&all, "specks").unwrap();
        assert_eq!(specks.value, 20.0);
        let patch = json!({"settings": {"order": {"min_length_mm": 0.3}}});
        assert_eq!(specks.patch, Some(patch));
        assert!(!finds(&page(dots(100)), "grid", "specks"));
    }

    #[test]
    fn a_light_drawing_of_a_dark_image_cannot_reach_dark() {
        // Lines 2 mm apart with a 0.3 mm pen cover about 15 % of a page of full ink.
        let mut d = drawing_100_with_tone(lines(2.0));
        let all = lint(&d, "hatch", |_| {});
        let dark = find(&all, "dark").unwrap();
        assert!((0.1..0.2).contains(&dark.value), "{}", dark.value);
        assert_eq!(dark.severity, Severity::Info);
        let patch = json!({"settings": {"hatch": {"spacing_mm": 0.3}}});
        assert_eq!(dark.patch, Some(patch));
        let all = lint(&d, "stipple", |_| {});
        let count = &find(&all, "dark").unwrap().patch.as_ref().unwrap()["settings"];
        assert!(
            count["voronoi"]["count"].as_u64().unwrap() > 15000,
            "{count}"
        );
        // Dense lines reach it, and a light image has nothing dark to reach.
        assert!(!finds(&drawing_100_with_tone(lines(0.3)), "hatch", "dark"));
        let grey = ToneMap {
            width: 1,
            height: 1,
            ink: vec![0.5],
            mm_per_px: 100.0,
            origin_mm: [0.0; 2],
        };
        d.tone = Some(Arc::new(grey));
        assert!(!finds(&d, "hatch", "dark"));
    }

    #[test]
    fn lifts_and_travel_take_a_share_of_the_time() {
        let all = lint(&page(dots(100)), "grid", |_| {});
        let lifts = find(&all, "lifts").unwrap();
        assert!(lifts.value > 0.9, "{}", lifts.value);
        assert!(lifts.message.contains("(100 lifts)"), "{}", lifts.message);
        // Two short strokes at opposite corners: most of the time is travel.
        let far = page(vec![
            vec![[0.5, 0.5], [3.0, 0.5]],
            vec![[99.0, 99.0], [96.5, 99.0]],
        ]);
        let all = lint(&far, "hatch", |s| s["order"]["optimize"] = json!(false));
        let travel = find(&all, "travel").unwrap();
        assert_eq!(travel.severity, Severity::Warning);
        let patch = json!({"settings": {"order": {"optimize": true}}});
        assert_eq!(travel.patch, Some(patch));
        let all = lint(&far, "hatch", |_| {});
        let travel = find(&all, "travel").unwrap();
        assert_eq!((travel.severity, &travel.patch), (Severity::Info, &None));
    }

    #[test]
    fn segments_finer_than_the_pen_are_needless() {
        let line = |step: f64| vec![(0..2000).map(|i| [10.0 + i as f64 * step, 50.0]).collect()];
        let all = lint(&page(line(0.02)), "hatch", |_| {});
        assert_eq!(find(&all, "vertices").unwrap().value, 1.0);
        assert!(!finds(&page(line(0.04)), "hatch", "vertices"));
    }

    #[test]
    fn pens_that_draw_nothing() {
        let mut d = page(lines(5.0));
        let red = Pen::new("Red", 0.5, "#ff0000").unwrap();
        let style = get_style("hatch").unwrap();
        let mut settings = style.defaults();
        let pens = [d.layers[0].pen.clone(), red.clone()];
        let all = |d: &Drawing, settings: &Value| {
            check(
                d,
                &measure(d),
                &style,
                settings,
                &pens,
                Speeds::default(),
                None,
            )
        };
        let found = all(&d, &settings);
        let unused = find(&found, "unused_pens").unwrap();
        let message = "Pen use is set to one pen, so only the first pen draws.";
        assert_eq!(unused.message, message);
        let patch = json!({"settings": {"split": {"mode": "light_to_dark"}}});
        assert_eq!(unused.patch, Some(patch));
        settings["split"]["mode"] = json!("light_to_dark");
        assert!(find(&all(&d, &settings), "unused_pens").is_none());
        d.layers.push(Layer {
            pen: red,
            paths: Paths::new(),
        });
        let found = all(&d, &settings);
        let unused = find(&found, "unused_pens").unwrap();
        let message = "These pens draw nothing at these settings: Red.";
        assert_eq!((unused.message.as_str(), &unused.patch), (message, &None));
    }

    #[test]
    fn findings_serialise_as_the_spec_shows() {
        let all = lint(&page(lines(1.0)), "hatch", |s| {
            s["hatch"]["spacing_mm"] = json!(0.2)
        });
        assert_eq!(
            serde_json::to_value(find(&all, "close_lines").unwrap()).unwrap(),
            json!({
                "code": "close_lines",
                "severity": "warning",
                "message": "Marks 0.2 mm apart are closer than the 0.3 mm pen, so they overlap.",
                "value": 0.2,
                "patch": {"settings": {"hatch": {"spacing_mm": 0.3}}}
            })
        );
    }

    #[test]
    fn the_session_lints_each_drawing_and_its_patches_apply() {
        let mut s = Session::new(256 << 20);
        assert!(s.lint().is_none());
        s.set_image(still_life(300, 437));
        for (style, expected) in [("grid", vec!["dark", "lifts"]), ("hatch", vec![])] {
            s.set_style(style).unwrap();
            let mut run = s.start().unwrap();
            while run.next_frame().is_some() {}
            let all = s.lint().unwrap();
            let codes: Vec<_> = all.iter().map(|f| f.code).collect();
            assert_eq!(codes, expected, "{style}");
            for patch in all.iter().filter_map(|f| f.patch.as_ref()) {
                for (stage, values) in patch["settings"].as_object().unwrap() {
                    s.update(stage, values).unwrap();
                }
            }
        }
        // The findings follow the session's speeds without a new run.
        s.set_speeds(Speeds {
            pen_lift_s: 100.0,
            ..Speeds::default()
        });
        assert!(find(&s.lint().unwrap(), "lifts").is_some());
    }

    #[test]
    fn the_golden_grid_lints_the_same_on_every_target() {
        // The same setup and literal are in crates/wasm/tests/smoke.rs, so CI proves both
        // targets agree on the raster.
        let mut s = Session::new(1 << 24);
        s.set_image(crate::synthetic::gradient(64, 48));
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
        s.update("tone", &json!({"resolution": 64})).unwrap();
        s.update("points", &json!({"threshold": 0.6, "spacing_mm": 2}))
            .unwrap();
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        let all = s.lint().unwrap();
        assert_eq!(find(&all, "dark").unwrap().value, 0.015042735042735043);
    }
}
