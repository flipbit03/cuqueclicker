//! Captures the current git branch (and re-runs when it changes) and
//! exposes it to the crate as the compile-time env var `CUQUE_GIT_BRANCH`.
//!
//! Used by the HUD title to mark dev builds with their branch name so
//! side-by-side instances built from different branches are visually
//! distinguishable. When the crate is built outside a git tree — e.g.
//! from a crates.io tarball, `cargo install`, or a vendored source drop —
//! no env var is set and `option_env!` at the call site yields `None`.
//! Shipped release binaries are built from a patched-version tree inside
//! CI's checkout, so they DO see a branch here, but their HUD already
//! shows the real version so the branch suffix is suppressed downstream.

use std::process::Command;

fn main() {
    // Rebuild if HEAD moves (new commit, branch checkout, etc.).
    println!("cargo:rerun-if-changed=.git/HEAD");

    // If HEAD points at a branch ref, also rebuild when THAT ref moves —
    // otherwise `cargo build` after a simple `git commit` on the current
    // branch wouldn't notice.
    if let Ok(out) = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        && out.status.success()
        && let Ok(s) = std::str::from_utf8(&out.stdout)
    {
        let refpath = s.trim();
        if !refpath.is_empty() {
            println!("cargo:rerun-if-changed=.git/{refpath}");
        }
    }

    // Branch name. On detached HEAD `--abbrev-ref HEAD` returns "HEAD";
    // fall back to a short SHA so the HUD still has something useful.
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let label = match branch {
        Some(b) if b != "HEAD" => Some(b),
        _ => Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    };
    if let Some(label) = label {
        println!("cargo:rustc-env=CUQUE_GIT_BRANCH={label}");
    }
    // If both git commands failed (no git tree, no git installed) we simply
    // don't set the env var — the HUD renders without a branch suffix.
}
