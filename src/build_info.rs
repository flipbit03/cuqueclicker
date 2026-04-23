/// `CARGO_PKG_VERSION` at compile time. "0.0.0" means a dev build; release
/// builds have `Cargo.toml` patched from the git tag before compilation.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dev build marker. `release.yml` swaps `Cargo.toml` version before building,
/// so any binary whose version is still "0.0.0" is an unreleased build.
pub fn is_dev_build() -> bool {
    VERSION == "0.0.0"
}
