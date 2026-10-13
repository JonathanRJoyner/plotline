//! The Pen use stage: share the tone map between the pens.
//!
//! In `one_pen` mode the first pen draws everything, so the one map is the tone map itself. In
//! `light_to_dark` mode the ink range is cut into one band per pen, in the pens' list order: with
//! `n` pens the first pen's ink rises over the lightest `1/n` of the range and then stays full, and
//! each later pen starts where the one before it is full, so it adds ink only to darker tones.
//! Adding strokes tone by tone, so that a darker tone keeps the marks of the lighter ones, is how
//! tonal art maps build hatching (Praun, Hoppe, Webb and Finkelstein, "Real-time hatching",
//! SIGGRAPH 2001).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::ToneMap;

/// How the pens share the drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitMode {
    /// The first pen draws everything.
    OnePen,
    /// Pens share the darkness in list order: the first draws from the lightest tones, each later pen adds only to darker ones.
    LightToDark,
    /// Each pen draws the part of the image its colour makes up.
    ByColour,
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the pen use stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SplitSettings {
    /// How the pens share the drawing: the first pen alone, light to dark in list order, or each pen by its colour; list light inks first, since pens plot in list order.
    pub mode: SplitMode,
}

impl Default for SplitSettings {
    fn default() -> Self {
        SplitSettings {
            mode: SplitMode::OnePen,
        }
    }
}

/// Share the tone map between the pens: one map per pen that draws, the lane's own tone for every
/// stage after this one.
pub struct SplitStage;

impl Stage for SplitStage {
    fn name(&self) -> &'static str {
        "split"
    }
    fn label(&self) -> &'static str {
        "Pen use"
    }
    fn version(&self) -> u32 {
        1
    }
    // The image and the canvas are for the modes that read colour; the others leave them alone.
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Image, Kind::Canvas, Kind::Pens, Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Tones
    }
    fn schema(&self) -> Value {
        schema_for::<SplitSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SplitSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let settings = validate::<SplitSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(SplitRun {
            input: Some((inputs.tone().clone(), inputs.pens().len(), settings.mode)),
            maps: None,
        }))
    }
}

/// The work happens in the first step, which reports one frame; the second gives the maps.
struct SplitRun {
    /// What the first step reads: the tone map, the number of pens and the mode; `None` once it
    /// has run.
    input: Option<(Arc<ToneMap>, usize, SplitMode)>,
    /// The finished maps, between the first step and the second.
    maps: Option<Vec<Arc<ToneMap>>>,
}

impl StageRun for SplitRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some((tone, pens, mode)) = self.input.take() {
            self.maps = Some(match mode {
                SplitMode::OnePen => vec![tone],
                SplitMode::LightToDark => bands(&tone, pens).into_iter().map(Arc::new).collect(),
                SplitMode::ByColour => {
                    return Err(StageError::Failed("by colour is not built yet".into()))
                }
            });
            return Ok(Step::Frame(Frame {
                stage: String::new(),
                progress: 1.0,
                label: "pen use".into(),
                points: None,
                paths: None,
                pen: 0,
            }));
        }
        let maps = self
            .maps
            .take()
            .expect("the split run has already finished");
        Ok(Step::Done(Artifact::Tones(Arc::new(maps))))
    }
}

/// `n` maps that cut `tone`'s ink range into equal bands: map `k` has ink
/// `clamp(n * t - k, 0, 1)` for the tone's ink `t`, and the size and transform of `tone`. Together
/// they hold `n * t` of ink, and map `k + 1` has ink only where map `k` is full.
pub(crate) fn bands(tone: &ToneMap, n: usize) -> Vec<ToneMap> {
    (0..n)
        .map(|k| ToneMap {
            width: tone.width,
            height: tone.height,
            ink: tone
                .ink
                .iter()
                .map(|&t| (n as f32 * t - k as f32).clamp(0.0, 1.0))
                .collect(),
            mm_per_px: tone.mm_per_px,
            origin_mm: tone.origin_mm,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::synthetic::flat;
    use crate::test_util::{black, tone_map};
    use crate::types::{Canvas, Fit, Image, Pen};
    use serde_json::json;
    use std::collections::HashMap;

    /// Run the split stage to the end with `patch` over the defaults.
    fn run_split(
        image: Image,
        canvas: Canvas,
        pens: Vec<Pen>,
        tone: ToneMap,
        patch: Value,
    ) -> Result<(Vec<Frame>, Vec<Arc<ToneMap>>), StageError> {
        let settings = merge(&SplitStage.defaults(), &patch);
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas))),
            (Kind::Pens, Artifact::Pens(Arc::new(pens))),
            (Kind::Tone, Artifact::Tone(Arc::new(tone))),
        ]));
        let mut run = SplitStage.start(inputs, &settings, RunContext::new(Arc::default()))?;
        let mut frames = Vec::new();
        loop {
            match run.step()? {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Tones(maps)) => return Ok((frames, maps.to_vec())),
                Step::Done(a) => panic!("expected tone maps, got {}", a.kind().name()),
            }
        }
    }

    /// [`run_split`] on a flat image and a 100 mm square page, which these modes do not read.
    fn split(pens: usize, tone: ToneMap, patch: Value) -> Vec<Arc<ToneMap>> {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let pens = vec![black(); pens];
        run_split(flat(4, 4, 0.5), canvas, pens, tone, patch)
            .unwrap()
            .1
    }

    #[test]
    fn settings_schema() {
        assert_eq!(
            (SplitStage.name(), SplitStage.label()),
            ("split", "Pen use")
        );
        assert_eq!(SplitStage.defaults(), json!({"mode": "one_pen"}));
        let schema = SplitStage.schema();
        let variants = schema["$defs"]["SplitMode"]["oneOf"].as_array().unwrap();
        let names: Vec<_> = variants
            .iter()
            .map(|v| v["const"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["one_pen", "light_to_dark", "by_colour"]);
        assert!(variants.iter().all(|v| v["description"].is_string()));
        assert!(schema["properties"]["mode"]["description"].is_string());
    }

    #[test]
    fn one_pen_passes_the_map_through() {
        let tone = tone_map(16, 4, |x, y| (x + y) as f32 / 19.0);
        let maps = split(2, tone.clone(), json!({}));
        assert_eq!(maps.len(), 1);
        assert_eq!(*maps[0], tone);
    }

    #[test]
    fn light_to_dark_splits_the_tone_in_list_order() {
        let tone = tone_map(16, 4, |x, _| x as f32 / 15.0);
        let maps = split(2, tone.clone(), json!({"mode": "light_to_dark"}));
        assert_eq!(maps.len(), 2);
        let want = |k: f32| -> Vec<f32> {
            tone.ink
                .iter()
                .map(|&t| (2.0 * t - k).clamp(0.0, 1.0))
                .collect()
        };
        assert_eq!(maps[0].ink, want(0.0));
        assert_eq!(maps[1].ink, want(1.0));
        for m in &maps {
            assert_eq!((m.width, m.height), (tone.width, tone.height));
            assert_eq!((m.mm_per_px, m.origin_mm), (tone.mm_per_px, tone.origin_mm));
        }
        let one = split(1, tone.clone(), json!({"mode": "light_to_dark"}));
        assert_eq!(one.len(), 1);
        assert_eq!(*one[0], tone);
    }

    #[test]
    fn bands_add_up_and_nest() {
        let tone = tone_map(11, 1, |x, _| x as f32 / 10.0);
        let maps = bands(&tone, 3);
        assert_eq!(maps.len(), 3);
        for i in 0..11 {
            let t = tone.ink[i];
            // Each later pen starts only once the one before is full, and together they give 3t.
            assert!(maps[1].ink[i] == 0.0 || maps[0].ink[i] == 1.0);
            assert!(maps[2].ink[i] == 0.0 || maps[1].ink[i] == 1.0);
            let total: f32 = maps.iter().map(|m| m.ink[i]).sum();
            assert!((total - 3.0 * t).abs() < 1e-5, "pixel {i}: {total}");
        }
    }

    #[test]
    fn by_colour_is_not_built_yet() {
        let tone = tone_map(4, 4, |_, _| 0.5);
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let err = run_split(
            flat(4, 4, 0.5),
            canvas,
            vec![black()],
            tone,
            json!({"mode": "by_colour"}),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "by colour is not built yet");
    }

    #[test]
    fn the_stage_reports_one_frame_then_the_maps() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let tone = tone_map(4, 4, |_, _| 0.5);
        let (frames, maps) =
            run_split(flat(4, 4, 0.5), canvas, vec![black()], tone, json!({})).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            (frames[0].progress, frames[0].label.as_str()),
            (1.0, "pen use")
        );
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        let tone = tone_map(4, 4, |_, _| 0.5);
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let err = run_split(
            flat(4, 4, 0.5),
            canvas,
            vec![black()],
            tone,
            json!({"mode": "darkest_first"}),
        )
        .unwrap_err();
        assert!(matches!(err, StageError::Settings(_)), "{err:?}");
    }
}
