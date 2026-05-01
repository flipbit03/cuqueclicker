//! Frozen save schemas — one module per shipped version.
//!
//! Each `vN` module is a verbatim copy of the persisted fields as they
//! existed at the time that version shipped. Once a vN module is on `main`
//! it MUST NOT be edited (except to fix a migration bug). Add `vN+1.rs`
//! with a `From<vN>`-style conversion instead.

pub mod v1;
pub mod v2;
pub mod v3;
