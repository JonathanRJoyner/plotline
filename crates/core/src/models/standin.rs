//! A runtime for tests: it runs no neural network, only a Sobel edge filter.
//!
//! The engine's tests need a [`ModelRuntime`] whose answer they can compute another way. The
//! stand-in "loads" two fixed byte strings and answers with the 3 x 3 Sobel gradient magnitude of
//! the picture's luminance (I. Sobel and G. Feldman, "A 3x3 isotropic gradient operator for image
//! processing", 1968), clamped to 0 to 1, or half of that for the second string. A third string
//! answers with a floor: 0.5 on blank paper, rising to 1 at a full edge.

use std::collections::HashMap;
use std::sync::Mutex;

use super::{ModelError, ModelRuntime};

/// The bytes of a stand-in model that answers with the Sobel magnitude.
pub const STANDIN_BYTES: &[u8] = b"STANDIN";
/// The bytes of a stand-in model that answers with half the Sobel magnitude.
pub const STANDIN_HALF_BYTES: &[u8] = b"STANDIN-HALF";
/// The bytes of a stand-in model whose answer never falls below 0.5, as a model does that does not
/// put blank paper at 0.
pub const STANDIN_FLOOR_BYTES: &[u8] = b"STANDIN-FLOOR";

/// The stand-in runtime; it also remembers which models were unloaded.
#[derive(Default)]
pub struct StandIn {
    /// Tile side, answer scale and answer floor of each loaded model.
    loaded: Mutex<HashMap<String, (usize, f32, f32)>>,
    unloaded: Mutex<Vec<String>>,
}

impl StandIn {
    pub fn new() -> StandIn {
        StandIn::default()
    }

    /// The ids passed to `unload`, oldest first.
    pub fn unloaded(&self) -> Vec<String> {
        self.unloaded.lock().unwrap().clone()
    }
}

impl ModelRuntime for StandIn {
    fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError> {
        let (scale, floor) = if bytes == STANDIN_BYTES {
            (1.0, 0.0)
        } else if bytes == STANDIN_HALF_BYTES {
            (0.5, 0.0)
        } else if bytes == STANDIN_FLOOR_BYTES {
            (1.0, 0.5)
        } else {
            return Err(ModelError::Invalid("not a stand-in model".into()));
        };
        self.loaded
            .lock()
            .unwrap()
            .insert(id.to_string(), (side, scale, floor));
        Ok(())
    }

    fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError> {
        let (side, scale, floor) = *self
            .loaded
            .lock()
            .unwrap()
            .get(id)
            .ok_or_else(|| ModelError::Run(format!("model {id} is not loaded")))?;
        let n = side * side;
        if rgb.len() != 3 * n {
            return Err(ModelError::Run(format!(
                "expected {} floats, got {}",
                3 * n,
                rgb.len()
            )));
        }
        let luma: Vec<f32> = (0..n)
            .map(|i| 0.299 * rgb[i] + 0.587 * rgb[n + i] + 0.114 * rgb[2 * n + i])
            .collect();
        // Edge pixels repeat, so a flat picture stays flat out to its border.
        let at = |x: isize, y: isize| {
            let (x, y) = (x.clamp(0, side as isize - 1), y.clamp(0, side as isize - 1));
            luma[y as usize * side + x as usize]
        };
        let mut out = Vec::with_capacity(n);
        for y in 0..side as isize {
            for x in 0..side as isize {
                let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                    - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                    - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                let sobel = (gx * gx + gy * gy).sqrt().clamp(0.0, 1.0);
                out.push(floor + (1.0 - floor) * sobel * scale);
            }
        }
        Ok(out)
    }

    fn unload(&self, id: &str) {
        self.loaded.lock().unwrap().remove(id);
        self.unloaded.lock().unwrap().push(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `side` x `side` picture, dark on the left half and white on the right, channel-major.
    fn step(side: usize) -> Vec<f32> {
        let plane: Vec<f32> = (0..side * side)
            .map(|i| if i % side < side / 2 { 0.0 } else { 1.0 })
            .collect();
        plane.repeat(3)
    }

    #[test]
    fn the_stand_in_finds_a_step_edge_and_leaves_flat_ground_alone() {
        let rt = StandIn::new();
        rt.load("a", STANDIN_BYTES, 8).unwrap();
        let out = rt.infer("a", &step(8)).unwrap();
        assert_eq!(out.len(), 64);
        for y in 0..8 {
            for x in 0..8 {
                let want = if x == 3 || x == 4 { 1.0 } else { 0.0 };
                assert_eq!(out[y * 8 + x], want, "({x}, {y})");
            }
        }
    }

    #[test]
    fn the_half_model_answers_with_half() {
        let rt = StandIn::new();
        rt.load("a", STANDIN_BYTES, 8).unwrap();
        rt.load("b", STANDIN_HALF_BYTES, 8).unwrap();
        let (a, b) = (
            rt.infer("a", &step(8)).unwrap(),
            rt.infer("b", &step(8)).unwrap(),
        );
        assert!(a.iter().zip(&b).all(|(a, b)| *b == 0.5 * *a));
    }

    #[test]
    fn the_stand_in_refuses_what_it_cannot_run() {
        let rt = StandIn::new();
        assert!(matches!(
            rt.load("a", b"other", 8),
            Err(ModelError::Invalid(_))
        ));
        assert!(matches!(
            rt.infer("a", &step(8)),
            Err(ModelError::Run(m)) if m == "model a is not loaded"
        ));
        rt.load("a", STANDIN_BYTES, 8).unwrap();
        assert!(matches!(rt.infer("a", &[0.0; 3]), Err(ModelError::Run(_))));
        rt.unload("a");
        assert_eq!(rt.unloaded(), ["a"]);
        assert!(rt.infer("a", &step(8)).is_err());
    }
}
