/// `CARGO_PKG_VERSION` at compile time. "0.0.0" means a dev build; release
/// builds have `Cargo.toml` patched from the git tag before compilation.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dev build marker. `release.yml` swaps `Cargo.toml` version before building,
/// so any binary whose version is still "0.0.0" is an unreleased build.
pub fn is_dev_build() -> bool {
    VERSION == "0.0.0"
}

/// Git branch (or short SHA on detached HEAD) captured by `build.rs` at
/// compile time. `None` when built outside a git tree — e.g. a crates.io
/// tarball or a vendored source drop — so release paths don't need special
/// handling. Used by the HUD to mark dev builds with their source branch.
pub const GIT_BRANCH: Option<&str> = option_env!("CUQUE_GIT_BRANCH");
