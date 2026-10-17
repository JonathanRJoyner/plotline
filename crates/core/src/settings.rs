//! Stage settings: the JSON Schema every client sees, and the one gate values pass through.
//!
//! Each stage declares one settings struct deriving `JsonSchema`. Its schema, with help text,
//! bounds, defaults and `x-` UI hints, is the only description of settings that clients see, and
//! [`validate`] checks a JSON object against it before the stage ever receives the values.
//!
//! The schema lists `properties` alphabetically. A stage that wants its form in another order adds
//! a root-level `"x-order": [key, ...]` to its schema; clients show those settings first, in that
//! order, then any left over in the schema's order.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// A settings value the stage did not agree to; the message names the stage and the setting.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SettingsError {
    /// The key is not one of the stage's settings.
    #[error("stage '{stage}', setting '{key}': unknown setting")]
    UnknownKey { stage: String, key: String },
    /// The value has the wrong JSON type.
    #[error("stage '{stage}', setting '{key}': expected {expected}, got {got}")]
    WrongType {
        stage: String,
        key: String,
        expected: String,
        got: String,
    },
    /// The number lies outside the setting's inclusive bounds; `None` is an open side.
    #[error("stage '{stage}', setting '{key}': {}, got {value}", bounds(.min, .max))]
    OutOfRange {
        stage: String,
        key: String,
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// The value is not one of the setting's choices.
    #[error("stage '{stage}', setting '{key}': {value} is not one of {choices}")]
    NotAChoice {
        stage: String,
        key: String,
        value: Value,
        choices: Value,
    },
    /// The settings failed to deserialize; `key` is `None` when no single setting is to blame.
    #[error("{}{message}", prefix(.stage, .key))]
    Invalid {
        stage: String,
        key: Option<String>,
        message: String,
    },
}

fn bounds(min: &Option<f64>, max: &Option<f64>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("must be between {lo} and {hi}"),
        (Some(lo), None) => format!("must be at least {lo}"),
        (None, Some(hi)) => format!("must be at most {hi}"),
        (None, None) => "is out of range".to_string(),
    }
}

fn prefix(stage: &str, key: &Option<String>) -> String {
    match key {
        Some(key) => format!("stage '{stage}', setting '{key}': "),
        None => format!("stage '{stage}': "),
    }
}

/// The JSON Schema for `S`, without the `$schema` key.
pub fn schema_for<S: JsonSchema>() -> Value {
    let mut schema = schemars::schema_for!(S).to_value();
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
    }
    schema
}

/// The default settings for `S` as a JSON object.
pub fn defaults_for<S: Serialize + Default>() -> Value {
    serde_json::to_value(S::default()).expect("settings structs serialize to JSON")
}

/// Shallow object merge where `patch` wins; anything other than two objects yields `patch`.
pub fn merge(current: &Value, patch: &Value) -> Value {
    match (current, patch) {
        (Value::Object(current), Value::Object(patch)) => {
            let mut merged = current.clone();
            merged.extend(patch.clone());
            Value::Object(merged)
        }
        _ => patch.clone(),
    }
}

/// Check `value` against `schema` key by key, then deserialize it into the stage's settings.
pub fn validate<S: DeserializeOwned>(
    stage: &str,
    schema: &Value,
    value: &Value,
) -> Result<S, SettingsError> {
    let invalid = |message: String| SettingsError::Invalid {
        stage: stage.to_string(),
        key: None,
        message,
    };
    let Some(object) = value.as_object() else {
        let message = format!("settings must be a JSON object, got {}", json_type(value));
        return Err(invalid(message));
    };
    for (key, v) in object {
        let (stage, key) = (stage.to_string(), key.clone());
        let Some(prop) = schema.get("properties").and_then(|p| p.get(&key)) else {
            return Err(SettingsError::UnknownKey { stage, key });
        };
        // schemars puts an enum or a nested struct under `$defs` and points at it with a `$ref`.
        let def = prop
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| schema.get("$defs")?.get(r.strip_prefix("#/$defs/")?))
            .unwrap_or(prop);
        // Variants with doc comments come out as `oneOf` of `const` instead of `enum`.
        let choices: Option<Vec<Value>> = def
            .get("enum")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| {
                let variants = def.get("oneOf")?.as_array()?;
                variants.iter().map(|c| c.get("const").cloned()).collect()
            });
        let expected: Vec<&str> = if choices.is_some() {
            vec!["string"]
        } else {
            match def.get("type") {
                Some(Value::String(t)) => vec![t.as_str()],
                Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            }
        };
        if !expected.is_empty() && !expected.iter().any(|t| type_matches(t, v)) {
            let (expected, got) = (expected.join(" or "), v.to_string());
            return Err(SettingsError::WrongType {
                stage,
                key,
                expected,
                got,
            });
        }
        if let Some(x) = v.as_f64() {
            let (min, max) = (prop.get("minimum"), prop.get("maximum"));
            let (min, max) = (min.and_then(Value::as_f64), max.and_then(Value::as_f64));
            if min.is_some_and(|lo| x < lo) || max.is_some_and(|hi| x > hi) {
                return Err(SettingsError::OutOfRange {
                    stage,
                    key,
                    value: x,
                    min,
                    max,
                });
            }
        }
        if let Some(choices) = choices {
            if !choices.contains(v) {
                let (value, choices) = (v.clone(), Value::Array(choices));
                return Err(SettingsError::NotAChoice {
                    stage,
                    key,
                    value,
                    choices,
                });
            }
        }
    }
    serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))
}

/// Whether `v` has JSON Schema type `t`; types this check does not know pass.
fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "string" => v.is_string(),
        "null" => v.is_null(),
        "object" => v.is_object(),
        _ => true,
    }
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(default, deny_unknown_fields)]
    struct Demo {
        /// A knob.
        #[schemars(range(min = 0.0, max = 10.0), extend("x-unit" = "mm", "x-step" = 0.5, "x-advanced" = true))]
        knob: f64,
        /// A flag.
        flag: bool,
        /// A choice.
        mode: Mode,
    }
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        Point,
        Circle,
    }
    impl Default for Demo {
        fn default() -> Self {
            Demo {
                knob: 2.0,
                flag: true,
                mode: Mode::Point,
            }
        }
    }

    #[test]
    fn schema_carries_help_bounds_defaults_and_extensions() {
        let s = schema_for::<Demo>();
        let knob = &s["properties"]["knob"];
        assert_eq!(knob["description"], "A knob.");
        assert_eq!(knob["minimum"], 0.0);
        assert_eq!(knob["maximum"], 10.0);
        assert_eq!(knob["default"], 2.0);
        assert_eq!(knob["x-unit"], "mm");
        assert_eq!(knob["x-advanced"], true);
        assert!(s.get("$schema").is_none());
        assert_eq!(defaults_for::<Demo>()["flag"], true);
    }
    #[test]
    fn validate_accepts_partial_objects_merged_over_defaults() {
        let merged = merge(&defaults_for::<Demo>(), &json!({"knob": 7.5}));
        let d: Demo = validate("demo", &schema_for::<Demo>(), &merged).unwrap();
        assert_eq!(
            d,
            Demo {
                knob: 7.5,
                ..Demo::default()
            }
        );
    }
    #[test]
    fn validate_rejects_unknown_out_of_range_wrong_type_and_bad_choice() {
        let s = schema_for::<Demo>();
        let e = validate::<Demo>("demo", &s, &json!({"knobb": 1}))
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("stage 'demo', setting 'knobb': "), "{e}");
        assert!(validate::<Demo>("demo", &s, &json!({"knob": 11}))
            .unwrap_err()
            .to_string()
            .contains("10"));
        let e = validate::<Demo>("demo", &s, &json!({"knob": "500"}))
            .unwrap_err()
            .to_string();
        assert!(e.contains("knob"), "{e}");
        let err = validate::<Demo>("demo", &s, &json!({"mode": "square"})).unwrap_err();
        assert!(matches!(err, SettingsError::NotAChoice { .. }), "{err:?}");
        let e = err.to_string();
        assert!(
            e.starts_with("stage 'demo', setting 'mode': ") && e.contains("circle"),
            "{e}"
        );
    }

    #[test]
    fn validate_reports_wrong_types_by_key() {
        let s = schema_for::<Demo>();
        for (key, bad, expected) in [
            ("knob", json!("500"), "number"),
            ("flag", json!(1), "boolean"),
            ("mode", json!(3), "string"),
        ] {
            let err = validate::<Demo>("demo", &s, &json!({ key: bad })).unwrap_err();
            assert!(matches!(err, SettingsError::WrongType { .. }), "{err:?}");
            let msg = err.to_string();
            assert!(
                msg.starts_with(&format!("stage 'demo', setting '{key}': ")),
                "{msg}"
            );
            assert!(msg.contains(expected), "{msg}");
        }
        let int_schema = json!({"properties": {"n": {"type": "integer"}}});
        assert!(matches!(
            validate::<Value>("demo", &int_schema, &json!({"n": 1.5})),
            Err(SettingsError::WrongType { .. })
        ));
        assert!(validate::<Value>("demo", &int_schema, &json!({"n": 3})).is_ok());
        let nested =
            json!({"properties": {"i": {"$ref": "#/$defs/I"}}, "$defs": {"I": {"type": "object"}}});
        assert!(validate::<Value>("demo", &nested, &json!({"i": {}})).is_ok());
        assert!(matches!(
            validate::<Value>("demo", &nested, &json!({"i": 1})),
            Err(SettingsError::WrongType { .. })
        ));
    }

    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum Shape {
        /// A point.
        Point,
        /// A circle.
        Circle,
    }
    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Documented {
        mode: Shape,
    }

    #[test]
    fn validate_reads_choices_from_documented_variants() {
        let s = schema_for::<Documented>();
        let err = validate::<Documented>("demo", &s, &json!({"mode": "square"})).unwrap_err();
        assert!(matches!(err, SettingsError::NotAChoice { .. }), "{err:?}");
        let e = err.to_string();
        assert!(e.starts_with("stage 'demo', setting 'mode': "), "{e}");
        let d: Documented = validate("demo", &s, &json!({"mode": "circle"})).unwrap();
        assert!(matches!(d.mode, Shape::Circle));
    }
}
