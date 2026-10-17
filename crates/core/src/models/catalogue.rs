//! The catalogue of hosted models, built into the core.
//!
//! `catalogue.json` lists each model a release knows how to fetch: where the file lives, its
//! SHA-256, its licence and who to credit. The core only reads the list. A front end downloads the
//! file itself, then hands the bytes to [`Session::add_model`](crate::Session::add_model), which
//! checks them against the entry here.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// One hosted model: where to get it and under what terms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueEntry {
    /// The name settings and commands use, ending in `-<n>` for the revision of the file.
    pub id: String,
    /// The name people see.
    pub label: String,
    /// One or two sentences on what the model draws.
    pub description: String,
    /// Where to download the file, pinned to a commit.
    pub url: String,
    /// The SHA-256 of the file, 64 lowercase hex digits.
    pub sha256: String,
    /// The SPDX identifier of the licence the weights are under.
    pub licence: String,
    /// The credit line the licence asks for.
    pub attribution: String,
    /// Where the original weights come from.
    pub source: String,
    /// The paper the model is described in.
    pub paper: String,
    /// What the weights were trained on.
    pub training: String,
    /// The size of the file in bytes.
    pub bytes: u64,
    /// Pixels of context each side of a tile that the tiler throws away.
    pub overlap: usize,
}

/// Every hosted model this release lists, parsed from `catalogue.json` on first use.
///
/// Panics if the embedded file is not valid JSON of the right shape; a unit test parses it, so
/// that cannot reach a user.
pub fn catalogue() -> &'static [CatalogueEntry] {
    #[cfg(test)]
    if let Some(entries) = test_hook::TEST_ENTRIES.with(std::cell::Cell::get) {
        return entries;
    }
    static CATALOGUE: OnceLock<Vec<CatalogueEntry>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("catalogue.json")).expect("catalogue.json is valid")
    })
}

/// A test-only way to list entries of the test's own in place of the real ones.
#[cfg(test)]
pub(crate) mod test_hook {
    use std::cell::Cell;

    use super::CatalogueEntry;

    thread_local! {
        pub(super) static TEST_ENTRIES: Cell<Option<&'static [CatalogueEntry]>> =
            const { Cell::new(None) };
    }

    /// Makes [`catalogue`](super::catalogue) list `entries` on this thread until dropped.
    pub(crate) struct Entries;

    impl Entries {
        pub(crate) fn set(entries: Vec<CatalogueEntry>) -> Entries {
            // Leaked so the slice is `'static`; a test process is short.
            TEST_ENTRIES.with(|c| c.set(Some(Box::leak(entries.into_boxed_slice()))));
            Entries
        }
    }

    impl Drop for Entries {
        fn drop(&mut self) {
            TEST_ENTRIES.with(|c| c.set(None));
        }
    }
}
