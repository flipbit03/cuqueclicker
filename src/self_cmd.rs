//! `cuqueclicker self update` — re-install the latest released version in
//! place. How it does that depends on how this binary was built:
//!
//! - **Shipped-binary build** (the `binary-release` cargo feature is enabled,
//!   i.e. it came from a GitHub Release via curl+sh or PowerShell): re-runs
//!   the same one-liner installer that initially fetched it. The installer
//!   is idempotent — it overwrites the binary at the same install path.
//! - **Source build** (the default, e.g. `cargo install` or a local
//!   `cargo build`): runs `cargo install cuqueclicker --force`, which will
//!   pick up the newest version from crates.io.
//!
//! Both paths inherit stdio, so the user sees installer / cargo output live.

use anyhow::{Context, Result, bail};
use std::process::Command;

#[cfg(all(feature = "binary-release", unix))]
const UNIX_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/flipbit03/cuqueclicker/main/install.sh";
#[cfg(all(feature = "binary-release", windows))]
const WINDOWS_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/flipbit03/cuqueclicker/main/install.ps1";

#[cfg(feature = "binary-release")]
pub fn update() -> Result<()> {
    println!("Updating via installer script...");

    #[cfg(unix)]
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("curl -fsSL {UNIX_INSTALL_URL} | sh"))
        .status()
        .context("failed to spawn sh")?;

    #[cfg(windows)]
    let status = Command::new("powershell")
        .args(["-Command", &format!("irm {WINDOWS_INSTALL_URL} | iex")])
        .status()
        .context("failed to spawn powershell")?;

    if !status.success() {
        bail!("update installer exited with status {:?}", status.code());
    }
    Ok(())
}

#[cfg(not(feature = "binary-release"))]
pub fn update() -> Result<()> {
    println!("Updating via `cargo install cuqueclicker --force`...");
    let status = Command::new("cargo")
        .args(["install", "cuqueclicker", "--force"])
        .status()
        .context("failed to spawn cargo (is it on your PATH?)")?;
    if !status.success() {
        bail!("cargo install exited with status {:?}", status.code());
    }
    Ok(())
}
