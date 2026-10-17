//! Learned line models: the files a session holds and the trait that runs them.
//!
//! A model is an ONNX file that takes a picture and answers where the lines are. The core never
//! reads the file: a host supplies a [`ModelRuntime`] (the native one is a tract build, the browser
//! one runs in its own WebAssembly module), the session hands it the bytes, and the `model` stage
//! asks it to run one tile at a time. This module holds what the core does itself: the size limit,
//! the checksum a catalogue model must match, the names a session gives its models, and the store
//! the runs read them from.
//!
//! Every model takes `1 x 3 x TILE x TILE` floats, RGB in 0 to 1, and returns `1 x 1 x TILE x
//! TILE` floats in 0 to 1 where 1 is a line. The overlap-tile strategy that cuts a picture into
//! such tiles, with mirrored borders, is that of O. Ronneberger, P. Fischer and T. Brox, "U-Net:
//! convolutional networks for biomedical image segmentation", MICCAI 2015, pp. 234-241.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod catalogue;
pub mod standin;

pub use catalogue::{catalogue, CatalogueEntry};
pub use standin::{StandIn, STANDIN_BYTES, STANDIN_FLOOR_BYTES, STANDIN_HALF_BYTES};

/// The side of a tile in pixels, margins included; the kept centre is `TILE - 2 * overlap`.
pub const TILE: usize = 512;

/// The largest model file a session takes, in bytes (200 MB).
pub const MAX_BYTES: usize = 209_715_200;

/// The overlap, in pixels, of a model file of the user's own, which carries no catalogue entry.
pub const FILE_OVERLAP: usize = 64;

/// Why a model could not be added or run; the message is shown to users.
#[derive(Debug, Error)]
pub enum ModelError {
    /// The file is over [`MAX_BYTES`]; the number is its size.
    #[error("the model file is {0} bytes, over the 200 MB limit")]
    TooLarge(usize),
    /// A catalogue model's bytes do not match the size and SHA-256 the catalogue lists.
    #[error("model {id} does not match its published checksum; the download may be damaged")]
    Hash { id: String },
    /// The catalogue has no model of this id.
    #[error("no model called {0} is in the catalogue")]
    Unknown(String),
    /// The session has no runtime to load the model into.
    #[error("this build cannot run models")]
    NoRuntime,
    /// The runtime could not use the file; the text says why.
    #[error("not a usable model: {0}")]
    Invalid(String),
    /// The runtime failed while running a model; the text says why.
    #[error("the model failed to run: {0}")]
    Run(String),
}

/// What a host supplies to run models: tract natively, a worker's module in the browser.
///
/// The ids are the store's ids. A runtime is told the tile side with each model so that it can
/// refuse a graph that does not fit.
pub trait ModelRuntime: Send + Sync {
    /// Prepare `bytes`, an ONNX file, to run under `id`, replacing any model of that id. Refuse a
    /// model that does not take `1 x 3 x side x side` and return `1 x 1 x side x side`.
    fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError>;
    /// Run the model `id` on `rgb`, `3 * side * side` floats in channel-major order, and give back
    /// its `side * side` answer.
    fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError>;
    /// Drop the model `id`; unknown ids are ignored.
    fn unload(&self, id: &str);
}

/// A model a session holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    /// The name the `model` setting uses.
    pub id: String,
    /// The name people see: the catalogue's, or the id for a file of the user's own.
    pub label: String,
    /// Pixels of context each side of a tile that the tiler throws away.
    pub overlap: usize,
    /// The catalogue entry the model was checked against, if it is a hosted model.
    pub entry: Option<CatalogueEntry>,
}

/// The models of a session and the runtime that runs them.
///
/// The map sits behind a lock so that the session can add and remove models while runs hold the
/// store by reference count.
#[derive(Default)]
pub struct ModelStore {
    runtime: Option<Arc<dyn ModelRuntime>>,
    models: Mutex<BTreeMap<String, ModelInfo>>,
}

impl ModelStore {
    /// An empty store that loads into `runtime`, if there is one.
    pub fn new(runtime: Option<Arc<dyn ModelRuntime>>) -> ModelStore {
        ModelStore {
            runtime,
            models: Mutex::default(),
        }
    }

    /// The model called `id`, if the store holds it.
    pub fn get(&self, id: &str) -> Option<ModelInfo> {
        self.models.lock().unwrap().get(id).cloned()
    }

    /// The runtime, if there is one.
    pub fn runtime(&self) -> Option<Arc<dyn ModelRuntime>> {
        self.runtime.clone()
    }

    /// Every model, in id order.
    pub fn list(&self) -> Vec<ModelInfo> {
        self.models.lock().unwrap().values().cloned().collect()
    }

    /// Check `bytes` and load them; see [`Session::add_model`](crate::Session::add_model).
    pub(crate) fn add(&self, id: Option<&str>, bytes: &[u8]) -> Result<ModelInfo, ModelError> {
        if bytes.len() > MAX_BYTES {
            return Err(ModelError::TooLarge(bytes.len()));
        }
        let info = match id {
            Some(id) => {
                let entry = catalogue()
                    .iter()
                    .find(|e| e.id == id)
                    .ok_or_else(|| ModelError::Unknown(id.to_string()))?;
                if bytes.len() as u64 != entry.bytes || sha256_hex(bytes) != entry.sha256 {
                    return Err(ModelError::Hash { id: id.to_string() });
                }
                ModelInfo {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    overlap: entry.overlap,
                    entry: Some(entry.clone()),
                }
            }
            None => {
                let id = file_id(bytes);
                ModelInfo {
                    label: id.clone(),
                    id,
                    overlap: FILE_OVERLAP,
                    entry: None,
                }
            }
        };
        let runtime = self.runtime.as_ref().ok_or(ModelError::NoRuntime)?;
        runtime.load(&info.id, bytes, TILE)?;
        self.models
            .lock()
            .unwrap()
            .insert(info.id.clone(), info.clone());
        Ok(info)
    }

    /// Forget the model `id` and unload it; an id the store does not hold changes nothing.
    pub(crate) fn remove(&self, id: &str) {
        if self.models.lock().unwrap().remove(id).is_some() {
            if let Some(runtime) = &self.runtime {
                runtime.unload(id);
            }
        }
    }
}

/// The id of a model file of the user's own: `file-` and the first 12 hex digits of its SHA-256.
pub fn file_id(bytes: &[u8]) -> String {
    format!("file-{}", &sha256_hex(bytes)[..12])
}

/// The SHA-256 of `bytes` as 64 lowercase hex digits.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::catalogue::test_hook::Entries;
    use super::*;
    use crate::session::Session;

    /// A session whose runtime is a stand-in, and a handle on that stand-in.
    fn session() -> (Session, Arc<StandIn>) {
        let rt = Arc::new(StandIn::new());
        let mut s = Session::new(1 << 20);
        s.set_model_runtime(rt.clone());
        (s, rt)
    }

    /// A catalogue entry whose checksum and size are those of `bytes`.
    fn entry(id: &str, bytes: &[u8]) -> CatalogueEntry {
        CatalogueEntry {
            id: id.into(),
            label: "Test model".into(),
            description: "A model for tests.".into(),
            url: "https://example.invalid/0123456789abcdef0123456789abcdef01234567/m.onnx".into(),
            sha256: sha256_hex(bytes),
            licence: "MIT".into(),
            attribution: "Made for tests.".into(),
            source: "https://example.invalid/source".into(),
            paper: "A test paper.".into(),
            training: "Nothing.".into(),
            bytes: bytes.len() as u64,
            overlap: 32,
        }
    }

    /// What is wrong with `e`, if anything.
    ///
    /// The overlap must be a multiple of 8, so that `TILE - 2 * overlap`, the centre a tile keeps,
    /// is a multiple of 16 and every tile's window starts on a multiple of 8. Nothing in the tiler
    /// depends on that alignment; it works for any overlap under half a tile. The rule is for
    /// models whose layers halve the image: it keeps their windows aligned. The overlap must be
    /// under half a tile because the centre would be empty.
    fn problem(e: &CatalogueEntry) -> Option<String> {
        const LICENCES: [&str; 7] = [
            "MIT",
            "BSD-2-Clause",
            "BSD-3-Clause",
            "Apache-2.0",
            "CC0-1.0",
            "CC-BY-3.0",
            "CC-BY-4.0",
        ];
        let id = &e.id;
        if !LICENCES.contains(&e.licence.as_str()) {
            return Some(format!("{id}: licence {}", e.licence));
        }
        let hex = |s: &str| s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if e.sha256.len() != 64 || !hex(&e.sha256) {
            return Some(format!("{id}: sha256 is not 64 lowercase hex digits"));
        }
        let commit = e
            .url
            .split(|c: char| !c.is_ascii_hexdigit())
            .any(|run| run.len() == 40 && hex(run));
        if !commit {
            return Some(format!("{id}: url holds no 40-digit commit"));
        }
        if e.training.is_empty() || e.attribution.is_empty() {
            return Some(format!("{id}: training or attribution is empty"));
        }
        let numbered =
            e.id.rsplit_once('-')
                .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !numbered {
            return Some(format!("{id}: id does not end in -<n>"));
        }
        // The tiler keeps TILE - 2 * overlap pixels of each tile; see the doc comment above.
        if 2 * e.overlap >= TILE || !e.overlap.is_multiple_of(8) {
            return Some(format!(
                "{id}: overlap {} must be a multiple of 8 under half a tile",
                e.overlap
            ));
        }
        None
    }

    #[test]
    fn add_model_names_a_file_by_its_hash() {
        let (mut s, _) = session();
        let info = s.add_model(None, STANDIN_BYTES).unwrap();
        assert_eq!(info.id, file_id(STANDIN_BYTES));
        assert_eq!(
            info.id,
            format!("file-{}", &sha256_hex(STANDIN_BYTES)[..12])
        );
        assert_eq!(info.label, info.id);
        assert_eq!(info.overlap, 64);
        assert!(info.entry.is_none());
        let listed = s.models();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, info.id);
        assert_eq!(s.model_store().get(&info.id).unwrap().id, info.id);
    }

    #[test]
    fn add_model_refuses_files_over_200_mb() {
        let (mut s, _) = session();
        let big = vec![0u8; 209_715_201];
        let err = s.add_model(None, &big).unwrap_err();
        assert!(matches!(err, ModelError::TooLarge(209_715_201)), "{err:?}");
        assert!(s.models().is_empty());
    }

    #[test]
    fn add_model_takes_a_catalogue_id_with_the_right_bytes() {
        let _entries = Entries::set(vec![entry("test-1", STANDIN_BYTES)]);
        let (mut s, _) = session();
        let info = s.add_model(Some("test-1"), STANDIN_BYTES).unwrap();
        assert_eq!(info.id, "test-1");
        assert_eq!(info.label, "Test model");
        assert_eq!(info.overlap, 32);
        assert_eq!(info.entry.unwrap().bytes, STANDIN_BYTES.len() as u64);
        assert_eq!(s.models().len(), 1);
    }

    #[test]
    fn add_model_refuses_a_catalogue_id_with_the_wrong_hash() {
        let _entries = Entries::set(vec![entry("test-1", STANDIN_BYTES)]);
        let (mut s, _) = session();
        // The right length with other bytes, then another length.
        for bytes in [&b"STANDIM"[..], STANDIN_HALF_BYTES] {
            let err = s.add_model(Some("test-1"), bytes).unwrap_err();
            assert!(
                matches!(&err, ModelError::Hash { id } if id == "test-1"),
                "{err:?}"
            );
        }
        assert!(s.models().is_empty());
    }

    #[test]
    fn add_model_refuses_an_unknown_id() {
        let (mut s, _) = session();
        let err = s.add_model(Some("nope-1"), STANDIN_BYTES).unwrap_err();
        assert!(
            matches!(&err, ModelError::Unknown(id) if id == "nope-1"),
            "{err:?}"
        );
        assert!(s.models().is_empty());
    }

    #[test]
    fn add_model_without_a_runtime_says_so() {
        let mut s = Session::new(1 << 20);
        let err = s.add_model(None, STANDIN_BYTES).unwrap_err();
        assert!(matches!(err, ModelError::NoRuntime), "{err:?}");
        assert!(s.models().is_empty());
    }

    #[test]
    fn add_model_passes_on_what_the_runtime_refuses() {
        let (mut s, _) = session();
        let err = s.add_model(None, b"not a model").unwrap_err();
        assert!(matches!(err, ModelError::Invalid(_)), "{err:?}");
        assert!(s.models().is_empty());
    }

    #[test]
    fn adding_an_id_again_replaces_it() {
        let _entries = Entries::set(vec![entry("test-1", STANDIN_BYTES)]);
        let (mut s, _) = session();
        s.add_model(Some("test-1"), STANDIN_BYTES).unwrap();
        s.add_model(Some("test-1"), STANDIN_BYTES).unwrap();
        s.add_model(None, STANDIN_BYTES).unwrap();
        s.add_model(None, STANDIN_BYTES).unwrap();
        assert_eq!(s.models().len(), 2);
    }

    #[test]
    fn remove_model_unloads_it() {
        let (mut s, rt) = session();
        let id = s.add_model(None, STANDIN_BYTES).unwrap().id;
        s.remove_model(&id);
        assert!(s.models().is_empty());
        assert_eq!(rt.unloaded(), std::slice::from_ref(&id));
        // Removing what is not there changes nothing.
        s.remove_model(&id);
        assert_eq!(rt.unloaded().len(), 1);
    }

    #[test]
    fn a_new_runtime_starts_with_no_models() {
        let (mut s, _) = session();
        s.add_model(None, STANDIN_BYTES).unwrap();
        s.set_model_runtime(Arc::new(StandIn::new()));
        assert!(s.models().is_empty());
    }

    #[test]
    fn catalogue_entries_are_complete() {
        let mut ids = std::collections::BTreeSet::new();
        for e in catalogue() {
            assert_eq!(problem(e), None);
            assert!(ids.insert(&e.id), "{} is listed twice", e.id);
        }
        // The check itself: a good entry passes and each fault is found.
        let good = entry("test-1", STANDIN_BYTES);
        assert_eq!(problem(&good), None);
        let faults: [fn(&mut CatalogueEntry); 8] = [
            |e| e.licence = "GPL-3.0-only".into(),
            |e| e.sha256 = "ABC".into(),
            |e| e.url = "https://example.invalid/main/m.onnx".into(),
            |e| e.training.clear(),
            |e| e.attribution.clear(),
            |e| e.id = "test".into(),
            // 256 leaves no centre, and over it the tiler would hang or panic.
            |e| e.overlap = TILE / 2,
            |e| e.overlap = 12,
        ];
        for fault in faults {
            let mut bad = good.clone();
            fault(&mut bad);
            assert!(problem(&bad).is_some(), "{bad:?}");
        }
    }
}
