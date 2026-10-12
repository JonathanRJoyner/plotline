//! Core engine for plotline.

/// The crate version, in Cargo's semver form.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_dev_prerelease() {
        assert_eq!(super::VERSION, "0.1.0-dev0");
    }
}
