//! The tract runtime for learned line models.
//!
//! [`TractRuntime`] implements the core's [`ModelRuntime`] with tract, a pure-Rust ONNX engine.
//! The Python extension links it natively; the browser build is the same crate as its own
//! WebAssembly module (the `web` feature). [`tiny_conv_onnx`] builds a small model in code for
//! tests of both.

mod tiny;

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
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

/// `f`'s result, or `kind` holding the panic's message when tract panics on a graph it does not
/// expect, so a hostile or broken file fails as an error instead of unwinding into the host. A
/// WebAssembly build aborts on a panic instead, so there it stays a trap.
fn caught<T>(
    f: impl FnOnce() -> Result<T, ModelError>,
    kind: fn(String) -> ModelError,
) -> Result<T, ModelError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let why = (panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_string());
        Err(kind(format!("the model runtime failed: {why}")))
    })
}

impl ModelRuntime for TractRuntime {
    fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError> {
        caught(|| self.load_unguarded(id, bytes, side), ModelError::Invalid)
    }

    fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError> {
        caught(|| self.infer_unguarded(id, rgb), ModelError::Run)
    }

    fn unload(&self, id: &str) {
        self.models.lock().unwrap().remove(id);
    }
}

impl TractRuntime {
    fn load_unguarded(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError> {
        let invalid = |e: TractError| ModelError::Invalid(format!("{e:#}"));
        // A file that does not decode, or decodes to no graph (an empty file does), is not ONNX at
        // all. The decoder's own words say only where it gave up, so they are left out; what tract
        // says about a graph it cannot translate does help, and `invalid` keeps it.
        let onnx = onnx();
        let proto = (onnx.proto_model_for_read(&mut &bytes[..]).ok())
            .filter(|proto| proto.graph.is_some())
            .ok_or_else(|| ModelError::Invalid("the file is not an ONNX model".into()))?;
        let parsed = onnx.model_for_proto_model(&proto).map_err(invalid)?;
        // Checked before anything indexes input 0 or output 0, which panics on a graph without one.
        if parsed.inputs.len() != 1 || parsed.outputs.len() != 1 {
            return Err(ModelError::Invalid(format!(
                "the model must have one input and one output, not {} and {}",
                parsed.inputs.len(),
                parsed.outputs.len()
            )));
        }
        let plan = parsed
            .with_input_fact(0, f32::fact([1, 3, side, side]).into())
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(invalid)?;
        let fact = plan.model().output_fact(0).map_err(invalid)?;
        if fact.datum_type != f32::datum_type()
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

    fn infer_unguarded(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError> {
        let run = |e: TractError| ModelError::Run(format!("{e:#}"));
        // The lock covers the lookup only, so runs of different models do not wait for each other.
        let (plan, side) = {
            let models = self.models.lock().unwrap();
            let m = models
                .get(id)
                .ok_or_else(|| ModelError::Run(format!("model {id} is not loaded")))?;
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
}

/// The model module's entry points, which the engine module's bridge
/// (`crates/wasm/js/models_bridge.js`) calls with a tile copied out of the engine's memory.
#[cfg(feature = "web")]
mod web {
    use std::sync::LazyLock;

    use plotline_core::models::{ModelError, ModelRuntime};
    use wasm_bindgen::prelude::*;

    use crate::TractRuntime;

    /// The one runtime of the worker's model module.
    static RUNTIME: LazyLock<TractRuntime> = LazyLock::new(TractRuntime::default);

    /// The error's own text: the bridge wraps it in its kind again, which would say it twice.
    fn thrown(e: ModelError) -> JsError {
        JsError::new(&match e {
            ModelError::Invalid(text) | ModelError::Run(text) => text,
            e => e.to_string(),
        })
    }

    /// Parse `bytes` as a model for square tiles of `side` pixels and keep it under `id`.
    #[wasm_bindgen]
    pub fn load(id: &str, bytes: &[u8], side: usize) -> Result<(), JsError> {
        RUNTIME.load(id, bytes, side).map_err(thrown)
    }

    /// Run the model `id` on one tile of `3 * side * side` channel-major floats.
    #[wasm_bindgen]
    pub fn infer(id: &str, rgb: &[f32]) -> Result<Vec<f32>, JsError> {
        RUNTIME.infer(id, rgb).map_err(thrown)
    }

    /// Drop the model `id`.
    #[wasm_bindgen]
    pub fn unload(id: &str) {
        RUNTIME.unload(id);
    }
}

#[cfg(test)]
mod tests {
    use plotline_core::models::{ModelError, ModelRuntime, TILE};
    use prost::Message;
    use tract_onnx::pb::{GraphProto, ModelProto, NodeProto};

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

    /// What a person is told when the file does not parse as ONNX, with none of tract's wording.
    const NOT_ONNX: &str = "the file is not an ONNX model";

    #[test]
    fn a_file_that_is_not_onnx_is_refused_in_plain_words() {
        let tiny = tiny_conv_onnx();
        let cases: [(&str, &[u8]); 4] = [
            ("text", b"not a model"),
            ("empty", b""),
            // The start of a zip file, which is what a PyTorch `.pth` is.
            ("pth", b"PK\x03\x04\x00\x00\x08\x08\x08\x00archive/data.pkl"),
            ("truncated", &tiny[..tiny.len() / 2]),
        ];
        for (name, bytes) in cases {
            let r = TractRuntime::default().load("x", bytes, TILE);
            assert!(
                matches!(&r, Err(ModelError::Invalid(m)) if m == NOT_ONNX),
                "{name}: {r:?}"
            );
        }
    }

    #[test]
    fn a_graph_tract_cannot_translate_keeps_its_own_reason() {
        // Valid ONNX that uses an operator no runtime knows: the reason is worth showing.
        let bytes = tiny_edited(|g| {
            g.node[0].op_type = "NoSuchOperator".into();
        });
        let r = TractRuntime::default().load("x", &bytes, TILE);
        assert!(
            matches!(&r, Err(ModelError::Invalid(m)) if m != NOT_ONNX),
            "{r:?}"
        );
    }

    /// The tiny model with its graph changed by `edit`, encoded again.
    fn tiny_edited(edit: impl FnOnce(&mut GraphProto)) -> Vec<u8> {
        let mut model = ModelProto::decode(&tiny_conv_onnx()[..]).unwrap();
        edit(model.graph.as_mut().unwrap());
        model.encode_to_vec()
    }

    #[test]
    fn a_model_with_no_inputs_is_refused() {
        // An Identity over a stored tensor: valid ONNX, but there is no input to give a picture to.
        let bytes = tiny_edited(|g| {
            g.input.clear();
            g.node = vec![NodeProto {
                op_type: "Identity".into(),
                input: vec!["b".into()],
                output: vec!["y".into()],
                ..Default::default()
            }];
        });
        let r = TractRuntime::default().load("x", &bytes, TILE);
        assert!(matches!(r, Err(ModelError::Invalid(_))), "{r:?}");
    }

    #[test]
    fn a_model_with_no_outputs_is_refused() {
        let bytes = tiny_edited(|g| g.output.clear());
        let r = TractRuntime::default().load("x", &bytes, TILE);
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
        let r = rt.infer("tiny", &picture(16));
        assert!(
            matches!(&r, Err(ModelError::Run(m)) if m == "model tiny is not loaded"),
            "{r:?}"
        );
    }

    #[test]
    fn a_panic_in_the_runtime_is_an_error_of_its_kind() {
        let load = crate::caught::<()>(|| panic!("a broken graph"), ModelError::Invalid);
        let text = "the model runtime failed: a broken graph";
        assert!(
            matches!(&load, Err(ModelError::Invalid(m)) if m == text),
            "{load:?}"
        );
        let id = "tiny";
        let infer = crate::caught::<()>(|| panic!("tile {id} overflowed"), ModelError::Run);
        let text = "the model runtime failed: tile tiny overflowed";
        assert!(
            matches!(&infer, Err(ModelError::Run(m)) if m == text),
            "{infer:?}"
        );
    }

    #[test]
    fn a_wrong_sized_input_is_refused() {
        let rt = TractRuntime::default();
        rt.load("tiny", &tiny_conv_onnx(), 16).unwrap();
        let r = rt.infer("tiny", &picture(8));
        assert!(matches!(r, Err(ModelError::Run(_))), "{r:?}");
    }
}
