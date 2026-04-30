//! Platform abstraction layer.
//!
//! Two impls are kept in lockstep: `native` (filesystem + std::fs locking)
//! and `web` (localStorage + no-op locking, added when the wasm port lands).
//! Both expose **the same set of types and methods** — code outside this
//! module doesn't know which one it's compiled against. Drift is caught at
//! compile time because both impls have to satisfy the same callers.
//!
//! See `tracking issue #13` for the discovery rationale and gotchas.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::{CAPABILITIES, InstanceLock, Persistence};

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::{CAPABILITIES, InstanceLock, Persistence};

/// Static feature-flag struct describing what the host environment
/// supports. Each platform impl exposes a `CAPABILITIES` constant of
/// this type. UI / input code reads it to gate affordances that have
/// no meaning on a given platform — e.g. the `[q] quit` help-bar hint
/// is hidden in the browser, where the wasm bundle has no authority
/// to close the tab.
///
/// Add new fields here when a feature genuinely diverges between
/// surfaces (and add the matching value to *both* `native::CAPABILITIES`
/// and `web::CAPABILITIES`); don't reach for `cfg(target_arch)` deeper
/// in the tree if a capability flag fits the seam.
#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    /// Whether the player can quit the running instance from inside
    /// the game. True on native (terminal close = process exit).
    /// False on web — the wasm has no authority over the tab; the
    /// player closes the page or navigates away themselves.
    pub can_quit: bool,
}
