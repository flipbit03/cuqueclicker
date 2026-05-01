//! Version-peeking helper for the save migration chain.
//!
//! Splitting this out of `mod.rs` keeps the dispatch in [`super::load_from_str`]
//! readable and gives version detection its own focused test surface.

use serde::Deserialize;

/// Read the top-level `"version"` field from a save JSON without
/// deserializing the rest of the document. Returns `1` if the field is
/// absent, non-numeric, or the JSON itself is malformed — pre-versioned
/// saves (everything written by `main` before this branch) are V1 by
/// definition.
pub fn peek_version(json: &str) -> u32 {
    #[derive(Deserialize)]
    struct VersionPeek {
        #[serde(default = "default_version")]
        version: u32,
    }
    fn default_version() -> u32 {
        1
    }
    serde_json::from_str::<VersionPeek>(json)
        .map(|v| v.version)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_1_when_field_absent() {
        assert_eq!(peek_version(r#"{"cuques": 100}"#), 1);
    }

    #[test]
    fn returns_1_on_invalid_json() {
        assert_eq!(peek_version("not json"), 1);
    }

    #[test]
    fn reads_the_field_when_present() {
        assert_eq!(peek_version(r#"{"version": 7, "cuques": 100}"#), 7);
    }

    #[test]
    fn returns_1_when_version_is_non_numeric() {
        // serde rejects "1" as u32 → fall back to 1 rather than crash.
        assert_eq!(peek_version(r#"{"version": "1"}"#), 1);
    }
}
