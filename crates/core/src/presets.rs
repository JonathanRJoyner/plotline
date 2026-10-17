//! Presets: the style, canvas, pens and settings of a session as a small, versioned JSON value.
//!
//! A preset is an object with the format name `plotline-preset`, an integer `version`, the `style`
//! name, the `canvas`, the `pens`, the `machine` (an object, or null for none), the `paper` (a
//! colour written as `#rrggbb`) and the `settings` of each stage (an object of objects). The
//! `paper` key is optional: a preset without it, such as one written before it existed, has
//! white paper. It is the wire format a front end sends and the file format a preset is saved in.
//!
//! [`parse`] checks the format, brings a value of an older version up to [`PRESET_VERSION`]
//! through the `MIGRATIONS` chain, checks that each section has the right shape and builds the
//! canvas and pens through their validating constructors. Whoever raises [`PRESET_VERSION`] adds
//! the function for the version it leaves behind, so `MIGRATIONS[v]` upgrades a value of version
//! `v` to `v + 1`, and a value of version `v` goes through `MIGRATIONS[v]` up to
//! `MIGRATIONS[PRESET_VERSION - 1]`. Entry 0 stays `None`: there was no version 0. The settings are not checked here: the session validates
//! them stage by stage, drops a stage or key the style does not have with a warning, and lets a
//! key the preset leaves out keep its default.
//!
//! The SVG and JSON exports embed the preset of the run that made the drawing, and [`from_text`]
//! reads a preset back out of a preset file, an exported SVG or an exported JSON drawing.

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::machine::Machine;
use crate::types::{check_color, Canvas, Fit, Pen, DEFAULT_PAPER};

/// The `format` of every preset.
pub const PRESET_FORMAT: &str = "plotline-preset";

/// The version this build writes and the newest it reads.
pub const PRESET_VERSION: u64 = 3;

/// A step of the chain: a preset of one version as the next.
type Migration = fn(Value) -> Value;

/// `MIGRATIONS[v]` upgrades a preset of version `v` to version `v + 1`.
const MIGRATIONS: &[Option<Migration>] = &[None, Some(v1_to_v2), Some(v2_to_v3)];

/// Version 2 added the machine; a version 1 preset has none.
fn v1_to_v2(mut v: Value) -> Value {
    v["machine"] = Value::Null;
    v
}

/// Version 3 gave the machine's area a corner and a size and dropped the bed. An area without
/// the four old bounds is left as it is, so `parse` names what is wrong with it.
fn v2_to_v3(mut v: Value) -> Value {
    if let Some(m) = v.get_mut("machine").and_then(Value::as_object_mut) {
        m.remove("bed_mm");
        let bound = |k: &str| m.get("area").and_then(|a| a.get(k)).and_then(Value::as_f64);
        if let (Some(x0), Some(x1), Some(y0), Some(y1)) = (
            bound("x_min"),
            bound("x_max"),
            bound("y_min"),
            bound("y_max"),
        ) {
            m.insert(
                "area".to_string(),
                json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0}),
            );
        }
    }
    v
}

/// A value that is not a preset this build can read.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PresetError {
    /// The `format` is missing or is not `plotline-preset`.
    #[error("not a preset: its format must be '{PRESET_FORMAT}'")]
    NotAPreset,
    /// The `version` is not a whole number up to the current one, so a newer build wrote the value
    /// or it is no version at all. The value is printed as it was given.
    #[error("unsupported preset version {0}: this build reads up to {PRESET_VERSION}")]
    UnsupportedVersion(Value),
    /// The chain has no function that upgrades from version `from`.
    #[error("cannot read preset version {version}: no migration from {from}")]
    NoMigration { version: u64, from: u64 },
    /// A section is missing or has the wrong shape: its name, then what it must be.
    #[error("preset field '{0}' must be {1}")]
    BadField(&'static str, &'static str),
    /// The canvas or a pen is not well formed or is not accepted by its constructor, or the preset
    /// element of an SVG does not hold JSON.
    #[error("{0}")]
    Invalid(String),
    /// The text is neither JSON nor an SVG.
    #[error("not a preset: it is neither JSON nor an SVG")]
    NeitherJsonNorSvg,
    /// An SVG without the preset element.
    #[error("this SVG carries no plotline preset")]
    NoPresetInSvg,
    /// A `plotline-paths` drawing without a `preset` key.
    #[error("this drawing carries no preset")]
    NoPresetInDrawing,
}

/// The content of a preset, migrated, shape-checked and with the canvas and pens validated.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetData {
    /// The name of the style.
    pub style: String,
    /// The page.
    pub canvas: Canvas,
    /// The pens, in order.
    pub pens: Vec<Pen>,
    /// The machine, if the preset names one.
    pub machine: Option<Machine>,
    /// The colour of the paper, written as `#rrggbb`; [`DEFAULT_PAPER`] when the preset has none.
    pub paper: String,
    /// The settings of each stage by stage name, an object of objects; not yet validated.
    pub settings: Value,
}

/// The preset of a session with this `style`, `canvas`, `pens`, `machine`, `paper` and `settings`.
pub fn to_value(
    style: &str,
    canvas: &Canvas,
    pens: &[Pen],
    machine: Option<&Machine>,
    paper: &str,
    settings: &Value,
) -> Value {
    json!({
        "format": PRESET_FORMAT,
        "version": PRESET_VERSION,
        "style": style,
        "canvas": canvas,
        "pens": pens,
        "machine": machine,
        "paper": paper,
        "settings": settings,
    })
}

/// The preset a file holds, ready for [`parse`]: a preset itself, an SVG exported with its preset
/// embedded, or a `plotline-paths` JSON drawing exported with one. Text that starts with `<` is an
/// SVG; anything else must be JSON. A JSON value that is not a drawing is returned as it is, so
/// `parse` names what is wrong with it.
pub fn from_text(text: &str) -> Result<Value, PresetError> {
    // A byte order mark is what some Windows tools put at the start of a UTF-8 file.
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with('<') {
        return from_svg(text);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| PresetError::NeitherJsonNorSvg)?;
    if value.get("format") != Some(&json!("plotline-paths")) {
        return Ok(value);
    }
    value
        .get("preset")
        .cloned()
        .ok_or(PresetError::NoPresetInDrawing)
}

/// The JSON in the `<metadata id="plotline-preset">` element of `svg`, with an editor's CDATA
/// wrapper removed and the entities unescaped.
fn from_svg(svg: &str) -> Result<Value, PresetError> {
    let none = || PresetError::NoPresetInSvg;
    let (_, tag) = svg.split_once(r#"id="plotline-preset""#).ok_or_else(none)?;
    let (_, content) = tag.split_once('>').ok_or_else(none)?;
    let (content, _) = content.split_once("</metadata>").ok_or_else(none)?;
    let content = content.trim();
    let cdata = content
        .strip_prefix("<![CDATA[")
        .and_then(|c| c.strip_suffix("]]>"));
    serde_json::from_str(&unescape(cdata.unwrap_or(content)))
        .map_err(|e| PresetError::Invalid(format!("the preset in this SVG is not JSON: {e}")))
}

/// `s` with the predefined XML entities and the decimal and hex character references replaced. One
/// left-to-right pass, so `&amp;lt;` gives `&lt;` and not `<`; a reference it does not know stays.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        // The longest reference, `&#x10FFFF;`, is ten bytes; looking further would make a run of
        // bare ampersands quadratic.
        let end = rest.bytes().take(12).position(|b| b == b';');
        match end.and_then(|end| Some((reference(&rest[1..end])?, end + 1))) {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

/// The character a reference names, without its `&` and `;`: `lt`, `gt`, `quot`, `apos`, `amp`,
/// `#65` or `#x41`.
fn reference(name: &str) -> Option<char> {
    let code = match name {
        "lt" => '<' as u32,
        "gt" => '>' as u32,
        "quot" => '"' as u32,
        "apos" => '\'' as u32,
        "amp" => '&' as u32,
        _ => {
            let digits = name.strip_prefix('#')?;
            match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            }
        }
    };
    char::from_u32(code)
}

/// Apply `chain[from]` for each `from` from `version` up to `target`.
fn upgrade(
    mut value: Value,
    version: u64,
    target: u64,
    chain: &[Option<Migration>],
) -> Result<Value, PresetError> {
    for from in version..target {
        let migrate = chain
            .get(from as usize)
            .copied()
            .flatten()
            .ok_or(PresetError::NoMigration { version, from })?;
        value = migrate(value);
    }
    Ok(value)
}

/// The canvas as written in a preset; the margin and fit may be left out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCanvas {
    width_mm: f64,
    height_mm: f64,
    #[serde(default)]
    margin_mm: f64,
    fit: Option<Fit>,
    #[serde(default)]
    crop: [f64; 4],
}

/// A pen as written in a preset.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPen {
    name: String,
    width_mm: f64,
    color: String,
}

/// Read a preset, bringing an older version up to date.
///
/// Fails when the format is not `plotline-preset`, when the version is not a whole number up to
/// [`PRESET_VERSION`] or has no migration, when `style`, `canvas`, `pens` or `settings` is missing
/// or the wrong shape (the message names the field), when `paper` is there but is not a string
/// written as `#rrggbb`, and when the canvas, a pen or the machine is refused by [`Canvas::new`],
/// [`Pen::new`] or [`Machine::validate`]. A missing or null `machine` is none, and a missing
/// `paper` is [`DEFAULT_PAPER`].
pub fn parse(value: &Value) -> Result<PresetData, PresetError> {
    if value.get("format") != Some(&json!(PRESET_FORMAT)) {
        return Err(PresetError::NotAPreset);
    }
    let given = value.get("version").unwrap_or(&Value::Null);
    let version = match given.as_u64() {
        Some(v) if v <= PRESET_VERSION => v,
        _ => return Err(PresetError::UnsupportedVersion(given.clone())),
    };
    let value = upgrade(value.clone(), version, PRESET_VERSION, MIGRATIONS)?;
    let style = value["style"]
        .as_str()
        .ok_or(PresetError::BadField("style", "a string"))?;
    let canvas = value
        .get("canvas")
        .filter(|canvas| canvas.is_object())
        .ok_or(PresetError::BadField("canvas", "an object"))?;
    let pens = value["pens"]
        .as_array()
        .filter(|pens| pens.iter().all(Value::is_object))
        .ok_or(PresetError::BadField("pens", "a list of objects"))?;
    let settings = value["settings"]
        .as_object()
        .filter(|stages| stages.values().all(Value::is_object))
        .ok_or(PresetError::BadField(
            "settings",
            "an object with an object for each stage",
        ))?;

    let paper = match value.get("paper") {
        None => DEFAULT_PAPER,
        Some(paper) => paper
            .as_str()
            .filter(|paper| check_color("paper", paper).is_ok())
            .ok_or(PresetError::BadField(
                "paper",
                "a colour written as #rrggbb",
            ))?,
    };

    let c: RawCanvas = serde_json::from_value(canvas.clone())
        .map_err(|e| PresetError::Invalid(format!("canvas: {e}")))?;
    let canvas = Canvas::new(
        c.width_mm,
        c.height_mm,
        c.margin_mm,
        c.fit.unwrap_or(Fit::Fit),
    )
    .and_then(|canvas| canvas.with_crop(c.crop))
    .map_err(|e| PresetError::Invalid(e.to_string()))?;
    let pens = pens
        .iter()
        .enumerate()
        .map(|(i, pen)| {
            let p: RawPen = serde_json::from_value(pen.clone())
                .map_err(|e| PresetError::Invalid(format!("pen {i}: {e}")))?;
            Pen::new(&p.name, p.width_mm, &p.color).map_err(|e| PresetError::Invalid(e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let machine = match value.get("machine") {
        None | Some(Value::Null) => None,
        Some(m) => {
            let m: Machine = serde_json::from_value(m.clone())
                .map_err(|e| PresetError::Invalid(format!("machine: {e}")))?;
            m.validate()
                .map_err(|e| PresetError::Invalid(format!("machine: {e}")))?;
            Some(m)
        }
    };
    Ok(PresetData {
        style: style.to_string(),
        canvas,
        pens,
        machine,
        paper: paper.to_string(),
        settings: Value::Object(settings.clone()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Machine;
    use crate::test_util::{a4, black, drawing_with, machine};
    use serde_json::json;

    #[test]
    fn round_trip() {
        let v = to_value(
            "grid",
            &a4(),
            &[black()],
            None,
            DEFAULT_PAPER,
            &json!({"tone": {"resolution": 500}}),
        );
        assert_eq!(v["format"], "plotline-preset");
        assert_eq!(v["version"], 3);
        let p = parse(&v).unwrap();
        assert_eq!(p.style, "grid");
        assert_eq!(p.settings["tone"]["resolution"], 500);
    }

    #[test]
    fn rejects_bad_shapes_with_named_fields() {
        let good = to_value("grid", &a4(), &[black()], None, DEFAULT_PAPER, &json!({}));
        let mut v = good.clone();
        v["format"] = json!("x");
        assert!(parse(&v)
            .unwrap_err()
            .to_string()
            .starts_with("not a preset"));
        let mut v = good.clone();
        v["version"] = json!(4);
        assert!(parse(&v).unwrap_err().to_string().contains("reads up to 3"));
        let mut v = good.clone();
        v["version"] = json!(0);
        assert!(parse(&v)
            .unwrap_err()
            .to_string()
            .contains("no migration from 0"));
        let mut v = good.clone();
        v["pens"] = json!([1]);
        assert!(parse(&v).unwrap_err().to_string().contains("'pens'"));
        let mut v = good.clone();
        v["canvas"]["width_mm"] = json!(-1);
        assert!(parse(&v).is_err());
    }

    /// The message that `parse` gives for a good preset with `key` set to `value`.
    fn error_with(key: &str, value: Value) -> String {
        let mut v = to_value("grid", &a4(), &[black()], None, DEFAULT_PAPER, &json!({}));
        v[key] = value;
        parse(&v).unwrap_err().to_string()
    }

    #[test]
    fn messages_are_the_documented_ones() {
        assert_eq!(
            parse(&json!([])).unwrap_err().to_string(),
            "not a preset: its format must be 'plotline-preset'"
        );
        assert_eq!(
            error_with("style", json!(3)),
            "preset field 'style' must be a string"
        );
        assert_eq!(
            error_with("canvas", json!([])),
            "preset field 'canvas' must be an object"
        );
        assert_eq!(
            error_with("pens", json!({})),
            "preset field 'pens' must be a list of objects"
        );
        assert_eq!(
            error_with("settings", json!({"tone": 1})),
            "preset field 'settings' must be an object with an object for each stage"
        );
        assert_eq!(
            error_with("version", json!(0)),
            "cannot read preset version 0: no migration from 0"
        );
    }

    #[test]
    fn a_version_that_is_not_a_whole_number_up_to_the_current_one_is_unsupported() {
        for (given, shown) in [
            (json!(4), "4"),
            (json!(1.0), "1.0"),
            (json!("1"), "\"1\""),
            (json!(-1), "-1"),
            (json!(null), "null"),
        ] {
            assert_eq!(
                error_with("version", given),
                format!("unsupported preset version {shown}: this build reads up to 3")
            );
        }
        let mut v = to_value("grid", &a4(), &[black()], None, DEFAULT_PAPER, &json!({}));
        v.as_object_mut().unwrap().remove("version");
        assert!(parse(&v).unwrap_err().to_string().contains("version null"));
    }

    #[test]
    fn canvas_and_pens_come_back_as_they_went_in() {
        let pens = [black(), Pen::new("Red", 0.5, "#FF0000").unwrap()];
        let p = parse(&to_value(
            "grid",
            &a4(),
            &pens,
            None,
            DEFAULT_PAPER,
            &json!({}),
        ))
        .unwrap();
        assert_eq!((p.canvas, p.pens), (a4(), pens.to_vec()));
        let v = to_value("grid", &a4(), &[], None, DEFAULT_PAPER, &json!({}));
        assert_eq!(v["canvas"]["fit"], "fit");
    }

    #[test]
    fn a_crop_comes_back_a_bad_one_is_named_and_no_crop_is_left_out() {
        let canvas = a4().with_crop([0.1, 0.0, 0.25, 0.3]).unwrap();
        let v = to_value("grid", &canvas, &[black()], None, DEFAULT_PAPER, &json!({}));
        assert_eq!(v["canvas"]["crop"], json!([0.1, 0.0, 0.25, 0.3]));
        assert_eq!(parse(&v).unwrap().canvas, canvas);
        let plain = to_value("grid", &a4(), &[black()], None, DEFAULT_PAPER, &json!({}));
        assert!(plain["canvas"].get("crop").is_none());
        let page = |crop: Value| json!({"width_mm": 5, "height_mm": 5, "crop": crop});
        assert_eq!(
            error_with("canvas", page(json!([0.5, 0, 0, 0]))),
            "crop left must be from 0 to 0.45, got 0.5"
        );
        assert!(error_with("canvas", page(json!([0.1]))).contains("canvas: invalid length 1"));
    }

    #[test]
    fn the_paper_round_trips_and_is_white_when_missing() {
        let v = to_value("grid", &a4(), &[black()], None, "#f5f0e1", &json!({}));
        assert_eq!(v["paper"], "#f5f0e1");
        assert_eq!(parse(&v).unwrap().paper, "#f5f0e1");
        // A preset written before the key existed.
        let mut old = v.clone();
        old.as_object_mut().unwrap().remove("paper");
        assert_eq!(parse(&old).unwrap().paper, DEFAULT_PAPER);
    }

    #[test]
    fn a_bad_paper_is_named() {
        for bad in [
            json!("cream"),
            json!("#fff"),
            json!("f5f0e1"),
            json!(5),
            json!(null),
        ] {
            assert_eq!(
                error_with("paper", bad),
                "preset field 'paper' must be a colour written as #rrggbb"
            );
        }
    }

    #[test]
    fn a_canvas_may_leave_out_margin_and_fit() {
        let p = parse(&json!({
            "format": "plotline-preset", "version": 1, "style": "s", "pens": [], "settings": {},
            "canvas": {"width_mm": 100, "height_mm": 50},
        }))
        .unwrap();
        assert_eq!(p.canvas, Canvas::new(100.0, 50.0, 0.0, Fit::Fit).unwrap());
    }

    #[test]
    fn a_bad_canvas_or_pen_is_named() {
        let unknown = json!({"width_mm": 5, "height_mm": 5, "margin": 1});
        assert!(error_with("canvas", unknown).contains("canvas: unknown field"));
        assert!(error_with("canvas", json!({"height_mm": 5})).contains("missing field `width_mm`"));
        assert!(
            error_with("canvas", json!({"width_mm": 5, "height_mm": 5, "fit": "x"}))
                .contains("unknown variant")
        );
        assert_eq!(
            error_with(
                "canvas",
                json!({"width_mm": 5, "height_mm": 5, "margin_mm": 3})
            ),
            "margin_mm must leave a positive drawing area, got 3.0 for a 5.0 x 5.0 mm page"
        );
        assert!(
            error_with("pens", json!([{"name": "a", "width_mm": 1}])).contains("pen 0: missing")
        );
        assert!(error_with(
            "pens",
            json!([{"name": "a", "width_mm": 0, "color": "#000000"}])
        )
        .contains("width_mm must be positive"));
        assert!(error_with(
            "pens",
            json!([{"name": "a", "width_mm": 1, "color": "red"}])
        )
        .contains("#rrggbb"));
    }

    #[test]
    fn the_chain_applies_each_step_from_the_version_on() {
        let chain: &[Option<Migration>] = &[
            Some(|mut v| {
                v["steps"] = json!(format!("{}a", v["steps"].as_str().unwrap_or("")));
                v
            }),
            Some(|mut v| {
                v["steps"] = json!(format!("{}b", v["steps"].as_str().unwrap_or("")));
                v
            }),
        ];
        assert_eq!(upgrade(json!({}), 0, 2, chain).unwrap()["steps"], "ab");
        assert_eq!(upgrade(json!({}), 1, 2, chain).unwrap()["steps"], "b");
        assert_eq!(upgrade(json!({}), 2, 2, chain).unwrap(), json!({}));
        assert_eq!(
            upgrade(json!({}), 0, 3, chain),
            Err(PresetError::NoMigration {
                version: 0,
                from: 2
            })
        );
    }

    #[test]
    fn a_version_1_preset_parses_with_no_machine() {
        let p = parse(&json!({
            "format": "plotline-preset", "version": 1, "style": "grid", "pens": [], "settings": {},
            "canvas": {"width_mm": 100, "height_mm": 50},
        }))
        .unwrap();
        assert_eq!(p.machine, None);
        let mut v = to_value("grid", &a4(), &[black()], None, DEFAULT_PAPER, &json!({}));
        assert_eq!(v["machine"], Value::Null);
        v.as_object_mut().unwrap().remove("machine");
        assert_eq!(parse(&v).unwrap().machine, None);
    }

    #[test]
    fn the_machine_round_trips() {
        let m: Machine = machine();
        let v = to_value(
            "grid",
            &a4(),
            &[black()],
            Some(&m),
            DEFAULT_PAPER,
            &json!({}),
        );
        assert_eq!(v["machine"]["area"]["x"], 28.0);
        assert_eq!(parse(&v).unwrap().machine, Some(m));
        let mut m = crate::test_util::servo_machine();
        m.max_segment_mm = Some(1.0);
        let v = to_value(
            "grid",
            &a4(),
            &[black()],
            Some(&m),
            DEFAULT_PAPER,
            &json!({}),
        );
        assert_eq!(parse(&v).unwrap().machine, Some(m));
    }

    #[test]
    fn a_version_2_preset_migrates_the_machine() {
        let p = parse(&json!({
            "format": "plotline-preset", "version": 2, "style": "grid", "pens": [], "settings": {},
            "canvas": {"width_mm": 100, "height_mm": 50},
            "machine": {
                "name": "Test printer",
                "bed_mm": [256, 256],
                "area": {"x_min": 28, "x_max": 228, "y_min": 42, "y_max": 216},
                "z_down_mm": 4, "z_up_mm": 9, "z_safe_mm": 20,
                "draw_feed": 3600, "travel_feed": 6000, "z_feed": 1200,
                "settle_ms": 50,
                "start_gcode": "M104 S0\nM140 S0",
                "end_gcode": "G1 Z20 F600\nG1 Y250 F3600",
            },
        }))
        .unwrap();
        assert_eq!(p.machine, Some(machine()));
    }

    #[test]
    fn a_version_2_area_missing_a_bound_is_refused() {
        let mut v = to_value(
            "grid",
            &a4(),
            &[black()],
            Some(&machine()),
            DEFAULT_PAPER,
            &json!({}),
        );
        v["version"] = json!(2);
        v["machine"]["area"] = json!({"x_min": 28, "x_max": 228, "y_min": 42});
        let err = parse(&v).unwrap_err().to_string();
        assert!(err.starts_with("machine: unknown field"), "{err}");
    }

    /// A preset whose pen name needs every escape.
    fn awkward_preset() -> Value {
        let pen = Pen::new("A & <B> \"c\"", 0.3, "#000000").unwrap();
        to_value(
            "grid",
            &a4(),
            &[pen],
            None,
            DEFAULT_PAPER,
            &json!({"tone": {"resolution": 500}}),
        )
    }

    #[test]
    fn from_text_reads_a_preset_an_svg_and_a_paths_drawing() {
        let (preset, drawing) = (awkward_preset(), drawing_with(vec![]));
        assert_eq!(from_text(&preset.to_string()).unwrap(), preset);
        let svg = crate::export::svg::to_svg(&drawing, Some(&preset));
        assert_eq!(from_text(&svg).unwrap(), preset);
        let paths = crate::export::json::to_value(&drawing, Some(&preset));
        assert_eq!(from_text(&paths.to_string()).unwrap(), preset);
        // A byte order mark and blank space before the text do not matter.
        assert_eq!(from_text(&format!("\u{feff}\n {svg}")).unwrap(), preset);
        assert_eq!(from_text(&format!("\u{feff}{preset}")).unwrap(), preset);
    }

    #[test]
    fn from_text_strips_a_cdata_wrapper() {
        let preset = awkward_preset();
        let svg = format!(
            "<svg>\n<metadata id=\"plotline-preset\">\n<![CDATA[{preset}]]>\n</metadata>\n</svg>"
        );
        assert_eq!(from_text(&svg).unwrap(), preset);
    }

    #[test]
    fn from_text_refuses_files_without_a_preset() {
        let drawing = drawing_with(vec![]);
        let refusal = |text: &str| from_text(text).unwrap_err().to_string();
        assert_eq!(
            refusal("<svg></svg>"),
            "this SVG carries no plotline preset"
        );
        assert_eq!(
            refusal(r#"<svg><metadata id="plotline-preset">{}"#),
            "this SVG carries no plotline preset"
        );
        assert_eq!(
            refusal(&crate::export::json::to_value(&drawing, None).to_string()),
            "this drawing carries no preset"
        );
        assert_eq!(
            refusal("plain words"),
            "not a preset: it is neither JSON nor an SVG"
        );
        assert!(
            refusal(r#"<svg><metadata id="plotline-preset">{</metadata>"#)
                .starts_with("the preset in this SVG is not JSON: ")
        );
        // Other JSON is returned as it is; `parse` refuses it.
        let other = from_text("[1]").unwrap();
        assert_eq!(parse(&other), Err(PresetError::NotAPreset));
    }

    #[test]
    fn unescape_reads_one_pass_and_leaves_unknown_references() {
        assert_eq!(
            unescape("&lt;&gt;&quot;&apos;&#65;&#x42;&#X43;&amp;lt;"),
            "<>\"'ABC&lt;"
        );
        assert_eq!(
            unescape("a & b &bogus; &#; &#xZ; &#1114112; &"),
            "a & b &bogus; &#; &#xZ; &#1114112; &"
        );
        assert_eq!(unescape(&"&".repeat(1000)), "&".repeat(1000));
    }

    #[test]
    fn a_bad_machine_names_the_field() {
        let mut m = machine();
        m.z_up_mm = 1.0;
        let v = to_value(
            "grid",
            &a4(),
            &[black()],
            Some(&m),
            DEFAULT_PAPER,
            &json!({}),
        );
        let err = parse(&v).unwrap_err().to_string();
        assert!(
            err.starts_with("machine: ") && err.contains("z_up_mm"),
            "{err}"
        );
        assert!(error_with("machine", json!(5)).contains("machine"));
    }
}
