//! Library crate for `cuqueclicker`.
//!
//! Both targets (`cuqueclicker` binary on native, the wasm cdylib for the
//! browser build via trunk) share these modules. Native-only modules
//! (`app`, `self_cmd`) and web-only modules (`wasm_app`) are gated by
//! `cfg(target_arch = "wasm32")`.
//!
//! `platform/` is the cross-target abstraction: `Persistence` and
//! `InstanceLock` are re-exported with the same struct/method shape on
//! both sides, so caller code outside `platform::` is portable.

pub mod build_info;
pub mod format;
pub mod game;
pub mod i18n;
pub mod input;
pub mod platform;
pub mod sim;
pub mod ui;

#[cfg(not(target_arch = "wasm32"))]
pub mod app;
#[cfg(not(target_arch = "wasm32"))]
pub mod self_cmd;

#[cfg(target_arch = "wasm32")]
mod wasm_app;
