//! Core engine for plotline.

pub mod analysis;
pub mod cache;
pub mod export;
pub mod fit;
pub mod image;
pub mod machine;
pub mod math;
pub mod pens;
pub mod presets;
pub mod preview;
pub mod raster;
pub mod registry;
pub mod runner;
pub mod session;
pub mod settings;
pub mod spatial;
pub mod stage;
pub mod stages;
pub mod stats;
pub mod style;
pub mod synthetic;
pub mod types;

pub use cache::{CacheStats, DEFAULT_CACHE_BYTES};
pub use fit::{FitResult, FitRun, Job, Metric};
pub use image::ImageError;
pub use preview::PreviewError;
pub use runner::{Run, RunState};
pub use session::{Session, SessionError};
pub use stage::Frame;
pub use stats::{Speeds, Stats};
pub use types::{Canvas, Drawing, Fit, Image, Layer, Paths, Pen, Points, ToneMap, TypeError};

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
