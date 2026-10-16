//! The stages of the hidden `_slow` style, which exists for host tests and is hidden from
//! `list_styles`: a run that takes a known time, reports a known number of frames, and can fail at
//! a chosen step.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, Points};

/// Settings for [`SlowPoint`], which takes none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SlowPointSettings {}

/// Stage `a`: one point at the origin, after one frame.
pub struct SlowPoint;

impl Stage for SlowPoint {
    fn name(&self) -> &'static str {
        "a"
    }
    fn label(&self) -> &'static str {
        "One point"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Image]
    }
    fn provides(&self) -> Kind {
        Kind::Points
    }
    fn schema(&self) -> Value {
        schema_for::<SlowPointSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SlowPointSettings>()
    }
    fn start(
        &self,
        _inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        validate::<SlowPointSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(SlowPointRun { framed: false }))
    }
}

struct SlowPointRun {
    /// Whether the one frame has been given.
    framed: bool,
}

impl StageRun for SlowPointRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let points = Points(vec![[0.0, 0.0]]);
        if self.framed {
            return Ok(Step::Done(Artifact::Points(Arc::new(points))));
        }
        self.framed = true;
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: 1.0,
            label: "1/1".into(),
            points: Some(points),
            paths: None,
            pen: 0,
        }))
    }
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for [`SlowPaths`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SlowSettings {
    /// Number of frames the stage reports before it finishes.
    #[schemars(range(min = 0, max = 100000))]
    pub frames: u32,
    /// Pause before each frame in milliseconds; WebAssembly builds do not pause.
    #[schemars(range(min = 0, max = 100000))]
    pub delay_ms: u32,
    /// Step, counted from 1, at which the stage fails; 0 never fails.
    #[schemars(range(min = 0, max = 100000))]
    pub fail_after: u32,
    /// Step, counted from 1, at which the stage panics, for hosts to test a crash; 0 never panics.
    #[schemars(range(min = 0, max = 100000))]
    pub panic_after: u32,
}

impl Default for SlowSettings {
    fn default() -> Self {
        SlowSettings {
            frames: 100,
            delay_ms: 10,
            fail_after: 0,
            panic_after: 0,
        }
    }
}

/// Stage `b`: `frames` frames, each after a pause of `delay_ms`, then one path from (1, 1) to
/// (2, 2). With `fail_after = k > 0` step `k` fails instead, and with `panic_after = k > 0` it
/// panics.
pub struct SlowPaths;

impl Stage for SlowPaths {
    fn name(&self) -> &'static str {
        "b"
    }
    fn label(&self) -> &'static str {
        "Slow path"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<SlowSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SlowSettings>()
    }
    fn start(
        &self,
        _inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let settings = validate::<SlowSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(SlowPathsRun { settings, step: 0 }))
    }
}

struct SlowPathsRun {
    settings: SlowSettings,
    /// Steps taken so far.
    step: u32,
}

impl StageRun for SlowPathsRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let s = &self.settings;
        self.step += 1;
        if self.step == s.fail_after {
            return Err(StageError::Failed("slow failed".into()));
        }
        assert!(self.step != s.panic_after, "slow panicked");
        if self.step > s.frames {
            let mut paths = Paths::new();
            paths.push(&[[1.0, 1.0], [2.0, 2.0]]);
            return Ok(Step::Done(Artifact::Paths(Arc::new(paths))));
        }
        // A WebAssembly worker cannot sleep.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::sleep(std::time::Duration::from_millis(u64::from(s.delay_ms)));
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: self.step as f32 / s.frames as f32,
            label: format!("{}/{}", self.step, s.frames),
            points: None,
            paths: None,
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn slow_paths_reports_its_frames_and_fails_at_the_chosen_step() {
        let start = |settings| {
            let ctx = RunContext::new(Arc::default());
            SlowPaths
                .start(Inputs::new(HashMap::new()), &settings, ctx)
                .unwrap()
        };
        let mut run = start(json!({"frames": 2, "delay_ms": 0, "fail_after": 0}));
        for label in ["1/2", "2/2"] {
            assert!(matches!(run.step(), Ok(Step::Frame(f)) if f.label == label));
        }
        assert!(matches!(run.step(), Ok(Step::Done(Artifact::Paths(p))) if p.len() == 1));
        let mut run = start(json!({"frames": 2, "delay_ms": 0, "fail_after": 2}));
        assert!(matches!(run.step(), Ok(Step::Frame(_))));
        assert!(matches!(run.step(), Err(StageError::Failed(m)) if m == "slow failed"));
    }
}
