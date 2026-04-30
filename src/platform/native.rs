//! Native platform impl: filesystem persistence + OS file lock.
//!
//! Save lives at `$XDG_CONFIG_HOME/cuqueclicker/save.json` (or the platform
//! equivalent under `$HOME`/`$APPDATA`/`$USERPROFILE`). The single-instance
//! lock is `std::fs::File::try_lock` on a sibling `.lock` file. The lock is
//! tied to the file descriptor — process exit, clean or crash, releases it.
//! The `.lock` file on disk carries no state of its own.

use anyhow::Result;
use std::env;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;

use super::Capabilities;
use crate::game::state::GameState;

/// Native is the canonical surface — every game-side affordance maps to
/// something the OS can actually do.
pub const CAPABILITIES: Capabilities = Capabilities { can_quit: true };

fn save_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("cuqueclicker").join("save.json"))
}

fn lock_path() -> Option<PathBuf> {
    save_path().map(|p| p.with_extension("json.lock"))
}

/// Filesystem-backed persistence. Stateless — the save path is derived
/// from environment variables on every call.
pub struct Persistence;

impl Default for Persistence {
    fn default() -> Self {
        Self::new()
    }
}

impl Persistence {
    pub fn new() -> Self {
        Self
    }

    /// Best-effort load. Returns `GameState::default()` if no save exists,
    /// the file is unreadable, or it fails to deserialize. Always runs the
    /// loaded state through `migrate()` to bring older save schemas current.
    pub fn load(&self) -> GameState {
        if let Some(path) = save_path()
            && let Ok(data) = fs::read_to_string(&path)
            && let Ok(state) = serde_json::from_str::<GameState>(&data)
        {
            return state.migrate();
        }
        GameState::default()
    }

    /// Atomic write via tmp-rename. On failure (no save dir, disk full)
    /// returns the IO error — callers typically `let _ =` this since save
    /// failures shouldn't crash the game.
    pub fn save(&self, state: &GameState) -> Result<()> {
        if let Some(path) = save_path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.tmp");
            let data = serde_json::to_string_pretty(state)?;
            fs::write(&tmp, data)?;
            fs::rename(&tmp, &path)?;
        }
        Ok(())
    }
}

/// Holds an exclusive OS-level lock on the save directory for the lifetime
/// of the process. `std::fs::File::try_lock` is stdlib (since Rust 1.89):
/// flock on Unix, LockFileEx on Windows.
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Acquire the lock or fail with `io::ErrorKind::WouldBlock` if another
    /// instance holds it. Other IO failures (no save dir, permission denied)
    /// surface their underlying error verbatim.
    pub fn try_acquire() -> io::Result<Self> {
        let Some(path) = lock_path() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no XDG_CONFIG_HOME / HOME / APPDATA / USERPROFILE set; cannot locate save dir",
            ));
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "save lock held by another process",
            ),
            TryLockError::Error(io_err) => io_err,
        })?;
        Ok(InstanceLock { _file: file })
    }
}
