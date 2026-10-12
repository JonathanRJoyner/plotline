//! Styles: ordered stages whose inputs are resolved by artifact kind.
//!
//! Each kind a stage requires comes from the nearest earlier stage that provides it, or else from
//! the root kind of that name, which the run is given. A stage that provides a root kind, such as
//! the image, replaces the root as the source for every later stage. Resolution happens once, in
//! [`Style::new`], so the runner only looks the sources up. This is why a stage may not be named
//! like a root kind.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use thiserror::Error;

use crate::stage::{Kind, Stage};

/// Where a stage reads one of its inputs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The root artifact the run is given.
    Root(Kind),
    /// The output of the stage at this index in [`Style::stages`].
    Stage(usize),
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
}

impl Style {
    /// Resolve every stage's inputs, or say why the style cannot work. The checks run in this
    /// order: repeated names, root names, unmet requirements, no stages, an undrawable last stage.
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
        Ok(Style {
            name: style(),
            label: label.to_string(),
            description: description.to_string(),
            stages,
            sources,
        })
    }

    /// Where stage `stage_index` reads each of its required kinds from, in the order of its
    /// `requires`. Panics if the index is out of range.
    pub fn sources(&self, stage_index: usize) -> &[(Kind, Source)] {
        &self.sources[stage_index]
    }

    /// The style and each stage's settings as JSON Schema, for UIs and tool descriptions:
    /// `{"name", "label", "description", "stages": [{"name", "label", "schema"}]}`.
    pub fn schema(&self) -> Value {
        let stages: Vec<Value> = self
            .stages
            .iter()
            .map(|s| json!({"name": s.name(), "label": s.label(), "schema": s.schema()}))
            .collect();
        json!({
            "name": self.name,
            "label": self.label,
            "description": self.description,
            "stages": stages,
        })
    }

    /// Each stage's default settings, keyed by stage name.
    pub fn defaults(&self) -> Value {
        Value::Object(
            self.stages
                .iter()
                .map(|s| (s.name().to_string(), s.defaults()))
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
    use crate::test_util::{paths, points, tone, Spy};

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
    }
}
