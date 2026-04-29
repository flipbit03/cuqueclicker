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
pub use native::{InstanceLock, Persistence};
