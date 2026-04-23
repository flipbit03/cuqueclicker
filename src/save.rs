use anyhow::Result;
use std::env;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::PathBuf;

use crate::game::state::GameState;

pub fn save_path() -> Option<PathBuf> {
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

/// Holds an exclusive OS-level lock on the save directory for the lifetime
/// of the process. Uses `std::fs::File::try_lock` (stable since Rust 1.89):
/// flock on Unix, LockFileEx on Windows. The lock is tied to the file
/// descriptor, so any process exit — clean or crash — releases it; the
/// `.lock` file on disk is just a handle target and carries no lock state.
pub struct SaveLock {
    _file: File,
}

pub fn acquire_lock() -> io::Result<SaveLock> {
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
    Ok(SaveLock { _file: file })
}

pub fn load() -> GameState {
    if let Some(path) = save_path()
        && let Ok(data) = fs::read_to_string(&path)
        && let Ok(state) = serde_json::from_str::<GameState>(&data)
    {
        return state.migrate();
    }
    GameState::default()
}

pub fn save(state: &GameState) -> Result<()> {
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
