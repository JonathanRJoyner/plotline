//! Core engine for plotline.

pub mod cache;
pub mod export;
pub mod math;
pub mod preview;
pub mod registry;
pub mod runner;
pub mod settings;
pub mod spatial;
pub mod stage;
pub mod stages;
pub mod stats;
pub mod style;
pub mod synthetic;
pub mod types;

#[cfg(test)]
mod test_util;

/// The crate version, in Cargo's semver form.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_dev_prerelease() {
        assert_eq!(super::VERSION, "0.1.0-dev0");
    }
}
