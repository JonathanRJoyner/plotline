//! Presets: the style, canvas, pens and settings of a session as a small, versioned JSON value.
//!
//! A preset is an object with the format name `plotline-preset`, an integer `version`, the `style`
//! name, the `canvas`, the `pens` and the `settings` of each stage (an object of objects). It is
//! the wire format a front end sends and the file format a preset is saved in.
//!
//! [`parse`] checks the format, brings a value of an older version up to [`PRESET_VERSION`]
//! through the `MIGRATIONS` chain, checks that each section has the right shape and builds the
//! canvas and pens through their validating constructors. Whoever raises [`PRESET_VERSION`] adds
//! the function for the version it leaves behind, so `MIGRATIONS[v]` upgrades a value of version
//! `v` to `v + 1`, and a value of version `v` goes through `MIGRATIONS[v]` up to
//! `MIGRATIONS[PRESET_VERSION - 1]`. The settings are not checked here: the session validates
//! them stage by stage, drops a stage or key the style does not have with a warning, and lets a
//! key the preset leaves out keep its default.

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::types::{Canvas, Fit, Pen};

/// The `format` of every preset.
pub const PRESET_FORMAT: &str = "plotline-preset";

/// The version this build writes and the newest it reads.
pub const PRESET_VERSION: u64 = 1;

/// `MIGRATIONS[v]` upgrades a preset of version `v` to version `v + 1`.
const MIGRATIONS: &[fn(Value) -> Value] = &[];

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
    /// The canvas or a pen is not well formed or is not accepted by its constructor.
    #[error("{0}")]
    Invalid(String),
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
    /// The settings of each stage by stage name, an object of objects; not yet validated.
    pub settings: Value,
}

/// The preset of a session with this `style`, `canvas`, `pens` and `settings`.
pub fn to_value(style: &str, canvas: &Canvas, pens: &[Pen], settings: &Value) -> Value {
    json!({
        "format": PRESET_FORMAT,
        "version": PRESET_VERSION,
        "style": style,
        "canvas": canvas,
        "pens": pens,
        "settings": settings,
    })
}

/// Apply `chain[from]` for each `from` from `version` up to `target`.
fn upgrade(
    mut value: Value,
    version: u64,
    target: u64,
    chain: &[fn(Value) -> Value],
) -> Result<Value, PresetError> {
    for from in version..target {
        let migrate = chain
            .get(from as usize)
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
/// or the wrong shape (the message names the field), and when the canvas or a pen is refused by
/// [`Canvas::new`] or [`Pen::new`].
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

    let c: RawCanvas = serde_json::from_value(canvas.clone())
        .map_err(|e| PresetError::Invalid(format!("canvas: {e}")))?;
    let canvas = Canvas::new(
        c.width_mm,
        c.height_mm,
        c.margin_mm,
        c.fit.unwrap_or(Fit::Fit),
    )
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
    Ok(PresetData {
        style: style.to_string(),
        canvas,
        pens,
        settings: Value::Object(settings.clone()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{a4, black};
    use serde_json::json;

    #[test]
    fn round_trip() {
        let v = to_value(
            "grid",
            &a4(),
            &[black()],
            &json!({"tone": {"resolution": 500}}),
        );
        assert_eq!(v["format"], "plotline-preset");
        assert_eq!(v["version"], 1);
        let p = parse(&v).unwrap();
        assert_eq!(p.style, "grid");
        assert_eq!(p.settings["tone"]["resolution"], 500);
    }

    #[test]
    fn rejects_bad_shapes_with_named_fields() {
        let good = to_value("grid", &a4(), &[black()], &json!({}));
        let mut v = good.clone();
        v["format"] = json!("x");
        assert!(parse(&v)
            .unwrap_err()
            .to_string()
            .starts_with("not a preset"));
        let mut v = good.clone();
        v["version"] = json!(2);
        assert!(parse(&v).unwrap_err().to_string().contains("reads up to 1"));
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
        let mut v = to_value("grid", &a4(), &[black()], &json!({}));
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
            (json!(2), "2"),
            (json!(1.0), "1.0"),
            (json!("1"), "\"1\""),
            (json!(-1), "-1"),
            (json!(null), "null"),
        ] {
            assert_eq!(
                error_with("version", given),
                format!("unsupported preset version {shown}: this build reads up to 1")
            );
        }
        let mut v = to_value("grid", &a4(), &[black()], &json!({}));
        v.as_object_mut().unwrap().remove("version");
        assert!(parse(&v).unwrap_err().to_string().contains("version null"));
    }

    #[test]
    fn canvas_and_pens_come_back_as_they_went_in() {
        let pens = [black(), Pen::new("Red", 0.5, "#FF0000").unwrap()];
        let p = parse(&to_value("grid", &a4(), &pens, &json!({}))).unwrap();
        assert_eq!((p.canvas, p.pens), (a4(), pens.to_vec()));
        let v = to_value("grid", &a4(), &[], &json!({}));
        assert_eq!(v["canvas"]["fit"], "fit");
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
        let chain: &[fn(Value) -> Value] = &[
            |mut v| {
                v["steps"] = json!(format!("{}a", v["steps"].as_str().unwrap_or("")));
                v
            },
            |mut v| {
                v["steps"] = json!(format!("{}b", v["steps"].as_str().unwrap_or("")));
                v
            },
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
}
