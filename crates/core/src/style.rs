//! Styles: ordered stages whose inputs are resolved by artifact kind.
//!
//! Each kind a stage requires comes from the nearest earlier stage that provides it, or else from
//! the root kind of that name, which the run is given. A stage that provides a root kind, such as
//! the image, replaces the root as the source for every later stage. Resolution happens once, in
//! [`Style::new`], so the runner only looks the sources up. This is why a stage may not be named
//! like a root kind.
//!
//! A stage that provides [`Kind::Tones`] splits the drawing between the pens: the stages after it
//! run once per pen, and read the tone map and the pen of their lane in place of the earlier
//! `Tone` and `Pens`. A style may split once, and a split style gives paths, not layers, since the
//! runner makes the layers from the lanes.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::fit::Density;
use crate::settings::{merge, validate};
use crate::stage::{Kind, Stage};

/// Where a stage reads one of its inputs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The root artifact the run is given.
    Root(Kind),
    /// The output of the stage at this index in [`Style::stages`].
    Stage(usize),
    /// The run's lane after a split: for [`Kind::Tone`] the lane's tone map, for [`Kind::Pens`] a
    /// list holding the lane's pen.
    Lane(Kind),
}

/// Why a style was rejected.
#[derive(Debug, Error)]
pub enum StyleError {
    /// Two stages share a name.
    #[error("style {style:?}: stage name {name:?} is used more than once")]
    DuplicateName { style: String, name: String },
    /// A stage is named `image`, `canvas` or `pens`.
    #[error("style {style:?}: stage {name:?} is named like a root kind")]
    RootName { style: String, name: String },
    /// A stage requires a kind that nothing before it provides.
    #[error(
        "style {style:?}: stage {name:?} requires {kind:?}, which no root kind or earlier stage provides"
    )]
    NoProvider {
        style: String,
        name: String,
        kind: &'static str,
    },
    /// The style has no stages.
    #[error("style {style:?} has no stages")]
    NoStages { style: String },
    /// The last stage provides something that cannot be drawn.
    #[error("style {style:?}: last stage {name:?} provides {kind:?}, not 'paths' or 'layers'")]
    BadLastStage {
        style: String,
        name: String,
        kind: &'static str,
    },
    /// A second stage provides 'tones'; a style splits into pens once.
    #[error("style {style:?}: stage {name:?} splits into pens again, after {first:?}")]
    SecondSplit {
        style: String,
        name: String,
        first: String,
    },
    /// A split style ends in 'layers', which the runner makes from the lanes.
    #[error(
        "style {style:?}: last stage {name:?} provides 'layers', but the stages after a split give paths, one set per pen"
    )]
    LayersAfterSplit { style: String, name: String },
}

/// No look draws its marks more than this many times sparser than the style's default along its
/// density setting: coarser marks swamp the picture's features whatever their tone error says
/// (see docs/decisions/2026-10-17-looks-on-more-pictures.md).
pub const SPARSEST_LOOK: f64 = 2.0;

/// The looks a style may have, in the order they are listed: name, label, description.
pub const LOOKS: [(&str, &str, &str); 4] = [
    (
        "balanced",
        "Balanced",
        "The style's defaults: a good start on most images.",
    ),
    (
        "faithful",
        "Faithful",
        "Follows the image's tones most closely, as measured on test images.",
    ),
    (
        "quick",
        "Quick",
        "Sparser marks that plot in about half an hour on A4 at the default speeds.",
    ),
    (
        "fine",
        "Fine",
        "The densest marks that still plot cleanly with a 0.3 mm pen and keep the tones.",
    ),
];

/// A named starting point within a style: values for the settings its looks set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Look {
    /// The stable identifier, one of the names in [`LOOKS`].
    pub name: String,
    /// The name shown to users.
    pub label: String,
    /// What the look is for, the same for every style.
    pub description: String,
    /// `{stage: {key: value}}` for every key any look of the style sets: this look's value, or the
    /// style's default where this look leaves the key alone.
    pub settings: Value,
}

/// Whether two setting values are equal, numbers by value: JavaScript writes `1.0` as `1`, which
/// JSON reads as an integer. Objects compare key by key.
pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        },
    }
}

/// A named recipe: stages that run in order, each reading artifacts by kind.
pub struct Style {
    /// The stable identifier.
    pub name: String,
    /// The name shown to users.
    pub label: String,
    /// What the style is for.
    pub description: String,
    /// The stages, in run order.
    pub stages: Vec<Arc<dyn Stage>>,
    /// Per stage, in the order of its `requires`, where each input comes from.
    sources: Vec<Vec<(Kind, Source)>>,
    /// The index of the stage that provides `Tones`, if the style splits into pens.
    split: Option<usize>,
    /// Settings this style starts from in place of its stages' own defaults, by stage name.
    overrides: Value,
    /// The looks the style declares besides Balanced, in order, each with its patch
    /// `{stage: {key: value}}` over the defaults.
    looks: Vec<(String, Value)>,
}

impl Style {
    /// Resolve every stage's inputs, or say why the style cannot work. The checks run in this
    /// order: repeated names, root names, then stage by stage unmet requirements and a second
    /// split, no stages, an undrawable last stage, 'layers' after a split.
    pub fn new(
        name: &str,
        label: &str,
        description: &str,
        stages: Vec<Arc<dyn Stage>>,
    ) -> Result<Style, StyleError> {
        let style = || name.to_string();
        for stage in &stages {
            if stages.iter().filter(|s| s.name() == stage.name()).count() > 1 {
                return Err(StyleError::DuplicateName {
                    style: style(),
                    name: stage.name().to_string(),
                });
            }
        }
        for stage in &stages {
            if Kind::ROOTS.iter().any(|k| k.name() == stage.name()) {
                return Err(StyleError::RootName {
                    style: style(),
                    name: stage.name().to_string(),
                });
            }
        }
        let mut providers: HashMap<Kind, Source> =
            Kind::ROOTS.iter().map(|&k| (k, Source::Root(k))).collect();
        let mut sources = Vec::with_capacity(stages.len());
        let mut split: Option<usize> = None;
        for (i, stage) in stages.iter().enumerate() {
            let found = stage
                .requires()
                .iter()
                .map(|&kind| {
                    let source = providers.get(&kind).copied();
                    source
                        .map(|s| (kind, s))
                        .ok_or_else(|| StyleError::NoProvider {
                            style: style(),
                            name: stage.name().to_string(),
                            kind: kind.name(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            sources.push(found);
            providers.insert(stage.provides(), Source::Stage(i));
            if stage.provides() == Kind::Tones {
                if let Some(first) = split {
                    return Err(StyleError::SecondSplit {
                        style: style(),
                        name: stage.name().to_string(),
                        first: stages[first].name().to_string(),
                    });
                }
                split = Some(i);
                providers.insert(Kind::Tone, Source::Lane(Kind::Tone));
                providers.insert(Kind::Pens, Source::Lane(Kind::Pens));
            }
        }
        let Some(last) = stages.last() else {
            return Err(StyleError::NoStages { style: style() });
        };
        if !matches!(last.provides(), Kind::Paths | Kind::Layers) {
            return Err(StyleError::BadLastStage {
                style: style(),
                name: last.name().to_string(),
                kind: last.provides().name(),
            });
        }
        if split.is_some() && last.provides() == Kind::Layers {
            return Err(StyleError::LayersAfterSplit {
                style: style(),
                name: last.name().to_string(),
            });
        }
        Ok(Style {
            name: style(),
            label: label.to_string(),
            description: description.to_string(),
            stages,
            sources,
            split,
            overrides: json!({}),
            looks: Vec::new(),
        })
    }

    /// The style with `patch` merged over the defaults of stage `stage`, for a style that needs a
    /// shared stage set differently, such as a finer tone map. Panics for a stage the style does
    /// not have or a value its schema refuses, since built-in styles are fixed at compile time.
    pub fn with_defaults(mut self, stage: &str, patch: Value) -> Style {
        let i = self
            .stage_index(stage)
            .unwrap_or_else(|| panic!("style {:?} has no stage {stage:?}", self.name));
        let merged = merge(&self.stages[i].defaults(), &patch);
        validate::<Value>(stage, &self.stages[i].schema(), &merged)
            .unwrap_or_else(|e| panic!("style {:?}: {e}", self.name));
        let base = self.overrides.get(stage).cloned().unwrap_or(json!({}));
        self.overrides[stage] = merge(&base, &patch);
        self
    }

    /// The style with the look `name` added: `patch` (`{stage: {key: value}}`) merged over the
    /// style's defaults, so declare looks after any [`Style::with_defaults`]. Panics for a name
    /// that is not in [`LOOKS`] or is `"balanced"`, which every style has; a look declared twice
    /// or out of the order of [`LOOKS`]; a stage the style does not have or a value its schema
    /// refuses; and a look with the same settings as another, which includes a patch that only
    /// repeats the defaults. Built-in styles are fixed at compile time, so a panic shows up in
    /// every test run.
    pub fn with_look(mut self, name: &str, patch: Value) -> Style {
        let style = self.name.clone();
        let rank = |look: &str| LOOKS.iter().position(|(n, _, _)| *n == look);
        let Some(index) = rank(name).filter(|&i| i > 0) else {
            panic!("style {style:?}: {name:?} is not a look a style declares");
        };
        if self.looks.iter().any(|(n, _)| n == name) {
            panic!("style {style:?}: look {name:?} is declared twice");
        }
        if let Some((last, _)) = self.looks.last().filter(|(l, _)| rank(l) > Some(index)) {
            panic!("style {style:?}: look {name:?} is declared after {last:?}");
        }
        let Some(stages) = patch.as_object() else {
            panic!("style {style:?}: look {name:?}: the patch must be an object of stages");
        };
        let defaults = self.defaults();
        for (stage, values) in stages {
            let i = self
                .stage_index(stage)
                .unwrap_or_else(|| panic!("style {style:?} has no stage {stage:?}"));
            let merged = merge(&defaults[stage], values);
            validate::<Value>(stage, &self.stages[i].schema(), &merged)
                .unwrap_or_else(|e| panic!("style {style:?}: look {name:?}: {e}"));
        }
        if let Some(d) = Density::of(&self) {
            let base = defaults[&d.stage][&d.key].as_f64();
            if let (Some(v), Some(base)) = (patch[&d.stage][&d.key].as_f64(), base) {
                let sparser = if d.direct { base / v } else { v / base };
                if sparser > SPARSEST_LOOK * (1.0 + 1e-9) {
                    panic!("style {style:?}: look {name:?} is {sparser:.2} times sparser than the default, over {SPARSEST_LOOK}");
                }
            }
        }
        self.looks.push((name.to_string(), patch));
        let looks = self.looks();
        let (last, earlier) = looks.split_last().expect("a look was just added");
        if let Some(other) = earlier.iter().find(|l| same(&l.settings, &last.settings)) {
            let other = &other.name;
            panic!("style {style:?}: look {name:?} has the same settings as look {other:?}");
        }
        self
    }

    /// Every look of the style: Balanced, which is the defaults, then the declared looks in the
    /// order of [`LOOKS`]. Each look's `settings` cover every key any declared look sets, and a
    /// look that leaves a key alone has the style's default there. A style with no declared look
    /// has only Balanced, with no settings.
    pub fn looks(&self) -> Vec<Look> {
        let defaults = self.defaults();
        let mut keys = BTreeSet::new();
        for (_, patch) in &self.looks {
            for (stage, values) in patch.as_object().into_iter().flatten() {
                for key in values.as_object().into_iter().flatten().map(|(k, _)| k) {
                    keys.insert((stage, key));
                }
            }
        }
        let make = |name: &str, patch: Option<&Value>| {
            let mut settings = json!({});
            for &(stage, key) in &keys {
                let set = patch.and_then(|p| p.get(stage)?.get(key));
                settings[stage][key] = set.unwrap_or(&defaults[stage][key]).clone();
            }
            let (_, label, description) = LOOKS
                .iter()
                .find(|(n, _, _)| *n == name)
                .expect("every look is in LOOKS");
            Look {
                name: name.to_string(),
                label: label.to_string(),
                description: description.to_string(),
                settings,
            }
        };
        let declared = self.looks.iter().map(|(n, patch)| make(n, Some(patch)));
        std::iter::once(make("balanced", None))
            .chain(declared)
            .collect()
    }

    /// Where stage `stage_index` reads each of its required kinds from, in the order of its
    /// `requires`. Panics if the index is out of range.
    pub fn sources(&self, stage_index: usize) -> &[(Kind, Source)] {
        &self.sources[stage_index]
    }

    /// The index of the stage that provides [`Kind::Tones`], which the stages after it run once
    /// per pen for; `None` if the style does not split.
    pub fn split(&self) -> Option<usize> {
        self.split
    }

    /// The style and each stage's settings as JSON Schema, for UIs and tool descriptions:
    /// `{"name", "label", "description", "stages": [{"name", "label", "schema"}], "looks":
    /// [{"name", "label", "description", "settings"}]}`. A stage's schema shows the style's own
    /// defaults where the style overrides them.
    pub fn schema(&self) -> Value {
        let stages: Vec<Value> = self
            .stages
            .iter()
            .map(|s| {
                let mut schema = s.schema();
                if let Some(Value::Object(o)) = self.overrides.get(s.name()) {
                    for (key, value) in o {
                        schema["properties"][key]["default"] = value.clone();
                    }
                }
                json!({"name": s.name(), "label": s.label(), "schema": schema})
            })
            .collect();
        json!({
            "name": self.name,
            "label": self.label,
            "description": self.description,
            "stages": stages,
            "looks": self.looks(),
        })
    }

    /// Each stage's default settings, with the style's overrides, keyed by stage name.
    pub fn defaults(&self) -> Value {
        Value::Object(
            self.stages
                .iter()
                .map(|s| {
                    let own = self.overrides.get(s.name()).cloned().unwrap_or(json!({}));
                    (s.name().to_string(), merge(&s.defaults(), &own))
                })
                .collect(),
        )
    }

    /// The index of the stage called `name`.
    pub fn stage_index(&self, name: &str) -> Option<usize> {
        self.stages.iter().position(|s| s.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{paths, points, tone, tones, Spy};

    #[test]
    fn resolves_nearest_provider_and_roots() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Points, points(1)));
        let b = Arc::new(Spy::new(
            "b",
            &[Kind::Points, Kind::Tone],
            Kind::Paths,
            paths(1),
        ));
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let c = Arc::new(Spy::new("c", &[Kind::Paths], Kind::Paths, paths(1)));
        let s = Style::new("s", "S", "d", vec![t, a, b, c]).unwrap();
        assert_eq!(
            s.sources(2),
            &[
                (Kind::Points, Source::Stage(1)),
                (Kind::Tone, Source::Stage(0))
            ]
        );
        assert_eq!(s.sources(3), &[(Kind::Paths, Source::Stage(2))]);
        assert_eq!(s.sources(0), &[(Kind::Image, Source::Root(Kind::Image))]);
    }
    #[test]
    fn rejects_bad_styles() {
        let p = || Arc::new(Spy::new("p", &[Kind::Image], Kind::Points, points(1)));
        let r = Arc::new(Spy::new("r", &[Kind::Points], Kind::Paths, paths(1)));
        assert!(Style::new("s", "S", "d", vec![]).is_err());
        assert!(Style::new("s", "S", "d", vec![p(), p()]).is_err());
        assert!(Style::new(
            "s",
            "S",
            "d",
            vec![Arc::new(Spy::new("image", &[], Kind::Points, points(1)))]
        )
        .is_err());
        assert!(Style::new("s", "S", "d", vec![r.clone()]).is_err()); // requires points, none
        assert!(Style::new("s", "S", "d", vec![p()]).is_err()); // last provides points
        assert!(Style::new("s", "S", "d", vec![p(), r]).is_ok());
    }
    #[test]
    fn errors_say_what_is_wrong() {
        let spy = |name, requires, provides| -> Arc<dyn Stage> {
            Arc::new(Spy::new(name, requires, provides, points(1)))
        };
        let msg = |stages| Style::new("s", "S", "d", stages).err().unwrap().to_string();
        let p = || spy("p", &[Kind::Image], Kind::Points);
        assert_eq!(
            msg(vec![p(), p()]),
            r#"style "s": stage name "p" is used more than once"#
        );
        assert_eq!(
            msg(vec![spy("pens", &[], Kind::Points)]),
            r#"style "s": stage "pens" is named like a root kind"#
        );
        assert_eq!(
            msg(vec![spy("r", &[Kind::Points], Kind::Paths)]),
            r#"style "s": stage "r" requires "points", which no root kind or earlier stage provides"#
        );
        assert_eq!(msg(vec![]), r#"style "s" has no stages"#);
        assert_eq!(
            msg(vec![p()]),
            r#"style "s": last stage "p" provides "points", not 'paths' or 'layers'"#
        );
    }
    #[test]
    fn stages_after_a_split_read_their_lane() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let s = Arc::new(Spy::new(
            "s",
            &[Kind::Tone, Kind::Pens],
            Kind::Tones,
            tones(&[0.5]),
        ));
        let a = Arc::new(Spy::new(
            "a",
            &[Kind::Tone, Kind::Pens, Kind::Canvas],
            Kind::Points,
            points(1),
        ));
        let b = Arc::new(Spy::new(
            "b",
            &[Kind::Points, Kind::Tone],
            Kind::Paths,
            paths(1),
        ));
        let style = Style::new("x", "X", "d", vec![t, s, a, b]).unwrap();
        assert_eq!(style.split(), Some(1));
        assert_eq!(
            style.sources(1),
            &[
                (Kind::Tone, Source::Stage(0)),
                (Kind::Pens, Source::Root(Kind::Pens))
            ]
        );
        assert_eq!(
            style.sources(2),
            &[
                (Kind::Tone, Source::Lane(Kind::Tone)),
                (Kind::Pens, Source::Lane(Kind::Pens)),
                (Kind::Canvas, Source::Root(Kind::Canvas)),
            ]
        );
        assert_eq!(
            style.sources(3),
            &[
                (Kind::Points, Source::Stage(2)),
                (Kind::Tone, Source::Lane(Kind::Tone))
            ]
        );
    }
    #[test]
    fn a_style_without_a_split_has_none() {
        let p: Arc<dyn Stage> = Arc::new(Spy::new("p", &[Kind::Image], Kind::Paths, paths(1)));
        assert_eq!(Style::new("s", "S", "d", vec![p]).unwrap().split(), None);
    }
    #[test]
    fn a_second_split_or_layers_after_a_split_is_refused() {
        let split = |name| -> Arc<dyn Stage> {
            Arc::new(Spy::new(name, &[Kind::Image], Kind::Tones, tones(&[0.5])))
        };
        let msg = |stages| Style::new("x", "X", "d", stages).err().unwrap().to_string();
        let p: Arc<dyn Stage> = Arc::new(Spy::new("p", &[Kind::Tone], Kind::Paths, paths(1)));
        assert_eq!(
            msg(vec![split("s"), split("u"), p]),
            r#"style "x": stage "u" splits into pens again, after "s""#
        );
        let l: Arc<dyn Stage> = Arc::new(Spy::new("l", &[Kind::Tone], Kind::Layers, paths(1)));
        assert_eq!(
            msg(vec![split("s"), l]),
            r#"style "x": last stage "l" provides 'layers', but the stages after a split give paths, one set per pen"#
        );
    }
    #[test]
    fn stage_index_finds_stages_by_name() {
        let p: Arc<dyn Stage> = Arc::new(Spy::new("p", &[Kind::Image], Kind::Paths, paths(1)));
        let s = Style::new("s", "S", "d", vec![p]).unwrap();
        assert_eq!(s.stage_index("p"), Some(0));
        assert_eq!(s.stage_index("q"), None);
    }
    #[test]
    fn schema_and_defaults_are_keyed_by_stage() {
        let s = Style::new(
            "s",
            "S",
            "d",
            vec![Arc::new(Spy::new(
                "p",
                &[Kind::Image],
                Kind::Paths,
                paths(1),
            ))],
        )
        .unwrap();
        assert_eq!(s.schema()["stages"][0]["name"], "p");
        assert_eq!(s.defaults()["p"]["knob"], 0);
        let s = s.with_defaults("p", json!({"knob": 3}));
        assert_eq!(s.defaults()["p"]["knob"], 3);
        assert_eq!(
            s.schema()["stages"][0]["schema"]["properties"]["knob"]["default"],
            3
        );
    }

    /// Two stages, `p` then `q`, each with the one setting `knob`, `p` defaulting to 3.
    fn two_knobs() -> Style {
        let p = Arc::new(Spy::new("p", &[Kind::Image], Kind::Points, points(1)));
        let q = Arc::new(Spy::new("q", &[Kind::Points], Kind::Paths, paths(1)));
        Style::new("s", "S", "d", vec![p, q])
            .unwrap()
            .with_defaults("p", json!({"knob": 3}))
    }
    fn names(looks: &[Look]) -> Vec<&str> {
        looks.iter().map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn a_style_without_looks_has_only_balanced() {
        let looks = two_knobs().looks();
        assert_eq!(names(&looks), ["balanced"]);
        assert_eq!(looks[0].label, "Balanced");
        assert_eq!(looks[0].description, LOOKS[0].2);
        assert_eq!(looks[0].settings, json!({}));
    }
    #[test]
    fn looks_merge_over_the_defaults_and_fill_the_other_keys() {
        let s = two_knobs()
            .with_look("faithful", json!({"p": {"knob": 5}}))
            .with_look("fine", json!({"q": {"knob": 7}}));
        let looks = s.looks();
        assert_eq!(names(&looks), ["balanced", "faithful", "fine"]);
        // Every look has every key a look sets, and the style's default where it sets none.
        let settings = |i: usize| looks[i].settings.clone();
        assert_eq!(settings(0), json!({"p": {"knob": 3}, "q": {"knob": 0}}));
        assert_eq!(settings(1), json!({"p": {"knob": 5}, "q": {"knob": 0}}));
        assert_eq!(settings(2), json!({"p": {"knob": 3}, "q": {"knob": 7}}));
        assert_eq!(looks[1].label, "Faithful");
        assert_eq!(looks[2].description, LOOKS[3].2);
    }
    #[test]
    fn a_look_differs_from_the_styles_defaults_not_the_stages() {
        // The stage alone defaults to 0, but the style's default for p.knob is 3.
        let s = two_knobs().with_look("quick", json!({"p": {"knob": 0}}));
        assert_eq!(s.looks()[1].settings, json!({"p": {"knob": 0}}));
    }
    #[test]
    fn looks_follow_the_order_of_the_list_not_the_order_of_keys() {
        let s = two_knobs()
            .with_look("faithful", json!({"q": {"knob": 1}}))
            .with_look("quick", json!({"p": {"knob": 1}}))
            .with_look("fine", json!({"p": {"knob": 2}, "q": {"knob": 2}}));
        assert_eq!(names(&s.looks()), ["balanced", "faithful", "quick", "fine"]);
    }
    #[test]
    fn the_schema_lists_the_looks() {
        let s = two_knobs().with_look("fine", json!({"q": {"knob": 7}}));
        let looks = &s.schema()["looks"];
        assert_eq!(looks.as_array().unwrap().len(), 2);
        assert_eq!(looks[0]["name"], "balanced");
        assert_eq!(looks[0]["label"], "Balanced");
        assert_eq!(looks[0]["settings"], json!({"q": {"knob": 0}}));
        assert_eq!(looks[1]["name"], "fine");
        assert_eq!(looks[1]["description"], LOOKS[3].2);
        assert_eq!(looks[1]["settings"], json!({"q": {"knob": 7}}));
        assert_eq!(two_knobs().schema()["looks"][0]["settings"], json!({}));
    }
    #[test]
    fn same_compares_numbers_by_value_and_objects_by_key() {
        assert!(same(&json!(1), &json!(1.0)));
        assert!(!same(&json!(1), &json!(2)));
        assert!(same(&json!("a"), &json!("a")));
        assert!(same(&json!({"p": {"k": 1}}), &json!({"p": {"k": 1.0}})));
        assert!(!same(&json!({"p": {"k": 1}}), &json!({"p": {"k": 2}})));
        assert!(!same(&json!({"p": 1}), &json!({"q": 1})));
    }

    #[test]
    #[should_panic(expected = "2.25 times sparser")]
    fn a_look_is_at_most_twice_as_sparse_as_the_default() {
        use crate::registry::halftone_style;
        // Halftone's spacing defaults to 2 mm: 4 mm passes, 4.5 mm is too coarse.
        halftone_style().with_look("quick", json!({"points": {"spacing_mm": 4.0}}));
        halftone_style().with_look("quick", json!({"points": {"spacing_mm": 4.5}}));
    }
    #[test]
    #[should_panic(expected = "\"sparkly\" is not a look")]
    fn a_look_name_must_be_listed() {
        two_knobs().with_look("sparkly", json!({"p": {"knob": 5}}));
    }
    #[test]
    #[should_panic(expected = "\"balanced\" is not a look")]
    fn balanced_is_never_declared() {
        two_knobs().with_look("balanced", json!({"p": {"knob": 5}}));
    }
    #[test]
    #[should_panic(expected = "declared twice")]
    fn a_look_is_declared_once() {
        two_knobs()
            .with_look("fine", json!({"p": {"knob": 5}}))
            .with_look("fine", json!({"p": {"knob": 6}}));
    }
    #[test]
    #[should_panic(expected = "\"faithful\" is declared after \"fine\"")]
    fn looks_are_declared_in_the_order_of_the_list() {
        two_knobs()
            .with_look("fine", json!({"p": {"knob": 5}}))
            .with_look("faithful", json!({"p": {"knob": 6}}));
    }
    #[test]
    #[should_panic(expected = "has no stage \"nope\"")]
    fn a_look_names_a_stage_of_the_style() {
        two_knobs().with_look("fine", json!({"nope": {"knob": 5}}));
    }
    #[test]
    #[should_panic(expected = "knob")]
    fn a_look_value_passes_the_schema() {
        two_knobs().with_look("fine", json!({"p": {"knob": "high"}}));
    }
    #[test]
    #[should_panic(expected = "'nob': unknown setting")]
    fn a_look_key_is_one_the_stage_has() {
        two_knobs().with_look("fine", json!({"p": {"nob": 5}}));
    }
    #[test]
    #[should_panic(expected = "must be an object of stages")]
    fn a_look_patch_is_an_object() {
        two_knobs().with_look("fine", json!(5));
    }
    #[test]
    #[should_panic(expected = "same settings as look \"balanced\"")]
    fn a_patch_that_repeats_the_defaults_is_refused() {
        // 3 is the style's default for p.knob.
        two_knobs().with_look("fine", json!({"p": {"knob": 3}}));
    }
    #[test]
    #[should_panic(expected = "same settings as look \"balanced\"")]
    fn an_empty_patch_is_refused() {
        two_knobs().with_look("fine", json!({}));
    }
    #[test]
    #[should_panic(expected = "same settings as look \"faithful\"")]
    fn two_looks_with_the_same_settings_are_refused() {
        two_knobs()
            .with_look("faithful", json!({"p": {"knob": 5}}))
            .with_look("fine", json!({"p": {"knob": 5}}));
    }
}
