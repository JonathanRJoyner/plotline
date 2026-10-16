//! The tract runtime for learned line models.
//!
//! [`TractRuntime`] implements the core's [`ModelRuntime`] with tract, a pure-Rust ONNX engine.
//! The Python extension links it natively; the browser build is the same crate as its own
//! WebAssembly module (the `web` feature). [`tiny_conv_onnx`] builds a small model in code for
//! tests of both.

mod tiny;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use plotline_core::models::{ModelError, ModelRuntime};
use tract_onnx::prelude::*;

pub use tiny::{tiny_conv_onnx, tiny_conv_onnx_with};

/// A model made ready to run, and the tile side it was made for.
struct Loaded {
    plan: Arc<TypedRunnableModel>,
    side: usize,
}

/// Runs ONNX models with tract; the models sit behind a lock so runs on several threads share it.
#[derive(Default)]
pub struct TractRuntime {
    models: Mutex<HashMap<String, Loaded>>,
}

impl ModelRuntime for TractRuntime {
    fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError> {
        let invalid = |e: TractError| ModelError::Invalid(format!("{e:#}"));
        let plan = onnx()
            .model_for_read(&mut &bytes[..])
            .and_then(|m| m.with_input_fact(0, f32::fact([1, 3, side, side]).into()))
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(invalid)?;
        let model = plan.model();
        let fact = model.output_fact(0).map_err(invalid)?;
        if model.inputs.len() != 1
            || model.outputs.len() != 1
            || fact.datum_type != f32::datum_type()
            || fact.shape.as_concrete() != Some(&[1, 1, side, side][..])
        {
            return Err(ModelError::Invalid(format!(
                "the model must take 1 x 3 x {side} x {side} floats and give 1 x 1 x {side} x {side}"
            )));
        }
        self.models
            .lock()
            .unwrap()
            .insert(id.to_string(), Loaded { plan, side });
        Ok(())
    }

    fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError> {
        let run = |e: TractError| ModelError::Run(format!("{e:#}"));
        // The lock covers the lookup only, so runs of different models do not wait for each other.
        let (plan, side) = {
            let models = self.models.lock().unwrap();
            let m = models
                .get(id)
                .ok_or_else(|| ModelError::Unknown(id.to_string()))?;
            (m.plan.clone(), m.side)
        };
        if rgb.len() != 3 * side * side {
            return Err(ModelError::Run(format!(
                "expected {} floats, got {}",
                3 * side * side,
                rgb.len()
            )));
        }
        let input = Tensor::from_shape(&[1, 3, side, side], rgb).map_err(run)?;
        let out = plan.run(tvec!(input.into_tvalue())).map_err(run)?;
        Ok(out[0]
            .try_as_plain_ram()
            .map_err(run)?
            .as_slice::<f32>()
            .map_err(run)?
            .to_vec())
    }

    fn unload(&self, id: &str) {
        self.models.lock().unwrap().remove(id);
    }
}

#[cfg(test)]
mod tests {
    use plotline_core::models::{ModelError, ModelRuntime, TILE};

    use crate::{tiny_conv_onnx, tiny_conv_onnx_with, TractRuntime};

    /// A seeded picture in 0 to 1, channel-major, from a 32-bit linear congruential generator.
    fn picture(side: usize) -> Vec<f32> {
        let mut state = 12345u32;
        (0..3 * side * side)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 24) as f32
            })
            .collect()
    }

    #[test]
    fn the_tiny_model_matches_a_direct_convolution() {
        let rt = TractRuntime::default();
        rt.load("tiny", &tiny_conv_onnx(), TILE).unwrap();
        let rgb = picture(TILE);
        let got = rt.infer("tiny", &rgb).unwrap();
        assert_eq!(got.len(), TILE * TILE);
        let weight = |c: usize, y: usize, x: usize| {
            (c + 1) as f64 * f64::from(0.01f32) * ((y * 3 + x) as f64 - 4.0)
        };
        for py in 0..TILE {
            for px in 0..TILE {
                let mut sum = 0.5f64;
                for c in 0..3 {
                    for y in 0..3 {
                        for x in 0..3 {
                            let (sy, sx) = (py + y, px + x);
                            // Padding 1 reads zeros outside the picture.
                            if (1..=TILE).contains(&sy) && (1..=TILE).contains(&sx) {
                                let v = rgb[c * TILE * TILE + (sy - 1) * TILE + (sx - 1)];
                                sum += weight(c, y, x) * f64::from(v);
                            }
                        }
                    }
                }
                let want = 1.0 / (1.0 + (-sum).exp());
                let diff = (f64::from(got[py * TILE + px]) - want).abs();
                assert!(diff < 1e-5, "pixel ({px}, {py}) differs by {diff}");
            }
        }
    }

    #[test]
    fn a_one_channel_input_is_refused() {
        let r = TractRuntime::default().load("x", &tiny_conv_onnx_with(1, 1), TILE);
        assert!(matches!(r, Err(ModelError::Invalid(_))), "{r:?}");
    }

    #[test]
    fn a_three_channel_output_is_refused() {
        let r = TractRuntime::default().load("x", &tiny_conv_onnx_with(3, 3), TILE);
        assert!(matches!(r, Err(ModelError::Invalid(_))), "{r:?}");
    }

    #[test]
    fn a_file_that_is_not_onnx_is_refused() {
        let r = TractRuntime::default().load("x", b"not a model", TILE);
        assert!(matches!(r, Err(ModelError::Invalid(_))), "{r:?}");
    }

    #[test]
    fn a_truncated_file_is_refused() {
        let bytes = tiny_conv_onnx();
        let r = TractRuntime::default().load("x", &bytes[..bytes.len() / 2], TILE);
        assert!(matches!(r, Err(ModelError::Invalid(_))), "{r:?}");
    }

    #[test]
    fn the_e2e_fixture_is_the_tiny_model() {
        let file = std::fs::read("../../web/e2e/fixtures/tiny-conv.onnx").unwrap();
        assert_eq!(file, tiny_conv_onnx());
    }

    #[test]
    fn an_unloaded_model_cannot_run() {
        let rt = TractRuntime::default();
        rt.load("tiny", &tiny_conv_onnx(), 16).unwrap();
        assert!(rt.infer("tiny", &picture(16)).is_ok());
        rt.unload("tiny");
        assert!(rt.infer("tiny", &picture(16)).is_err());
    }

    #[test]
    fn a_wrong_sized_input_is_refused() {
        let rt = TractRuntime::default();
        rt.load("tiny", &tiny_conv_onnx(), 16).unwrap();
        let r = rt.infer("tiny", &picture(8));
        assert!(matches!(r, Err(ModelError::Run(_))), "{r:?}");
    }
}
