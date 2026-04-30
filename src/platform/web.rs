//! Web platform impl: localStorage persistence + no-op instance lock.
//!
//! Save key is `cuqueclicker:save`. The browser already scopes one
//! localStorage origin per tab/origin pair, and there's no `flock`
//! analogue — `InstanceLock::try_acquire` always succeeds. If a player
//! opens two tabs of the same origin simultaneously, both write to the
//! same key; last writer wins. We accept the data race because the only
//! alternative (BroadcastChannel + leader election) is far more code
//! than the failure mode warrants for a single-player idle game.

use super::Capabilities;
use crate::game::state::GameState;

/// In a browser tab the wasm bundle has no way to exit — there's no
/// process to signal, and `window.close()` only works for windows the
/// script opened. The player closes the tab themselves; we hide the
/// `[q] quit` affordance accordingly.
pub const CAPABILITIES: Capabilities = Capabilities { can_quit: false };

const SAVE_KEY: &str = "cuqueclicker:save";

/// localStorage-backed persistence. Stateless — looks up
/// `window.localStorage` on every call. Returns `GameState::default()`
/// when the browser blocks access (private mode without storage,
/// disabled cookies, etc.).
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

    /// Best-effort load. Returns `GameState::default()` if storage isn't
    /// available, the key is missing, or the value fails to deserialize.
    /// Always runs the loaded state through `migrate()` so older saves
    /// from other browsers / older versions reach the current schema.
    pub fn load(&self) -> GameState {
        if let Some(storage) = local_storage()
            && let Ok(Some(data)) = storage.get_item(SAVE_KEY)
            && let Ok(state) = serde_json::from_str::<GameState>(&data)
        {
            return state.migrate();
        }
        GameState::default()
    }

    /// Best-effort save. localStorage `set_item` can fail when the quota
    /// is exhausted (typically ~5 MB per origin); the resulting JsValue
    /// is silently dropped — the contract is "saves shouldn't crash the
    /// game", same as native.
    pub fn save(&self, state: &GameState) -> anyhow::Result<()> {
        let Some(storage) = local_storage() else {
            return Ok(());
        };
        let data = serde_json::to_string(state)?;
        let _ = storage.set_item(SAVE_KEY, &data);
        Ok(())
    }
}

/// No-op single-instance lock. Browsers already isolate origins per tab
/// and we don't want to coordinate across tabs (see module docs); the
/// lock exists solely so callers don't have to `cfg`-gate the ownership
/// pattern from native.
pub struct InstanceLock;

impl InstanceLock {
    /// Always succeeds on web. The return type matches native's
    /// `io::Result<Self>` shape so callers can pattern-match identically.
    pub fn try_acquire() -> std::io::Result<Self> {
        Ok(InstanceLock)
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}
