//! Save persistence with versioned migration chain.
//!
//! Wire format on disk: a single JSON object with a `"version": u32` field.
//! Pre-versioned saves (no field) are treated as V1 by [`migrate::peek_version`].
//! Each version has a frozen struct in `versions/vN.rs` and a conversion into
//! the next version's struct, walked end-to-end on load.
//!
//! Once shipped, a `versions/vN.rs` is FROZEN. New schema changes go in
//! `vN+1.rs` together with a `From<vN> for vN+1` impl and a unit test. See
//! the "Save versioning" section in `CLAUDE.md` for the full policy.
//!
//! Callers (the `Persistence` impls under `platform/`) should never touch
//! `serde_json` on `GameState` directly — go through [`load_from_str`] /
//! [`save_to_string`] so the version dispatch stays in one place.

pub mod migrate;
pub mod versions;

use crate::game::state::GameState;

/// The version number every fresh save is written as. Bump in lockstep
/// with adding a new `versions/vN.rs` and routing it in [`load_from_str`].
pub const CURRENT_VERSION: u32 = 1;

/// Best-effort load from a JSON string. Falls back to a default state if
/// the input is malformed at any layer of the chain. The result is always
/// passed through [`GameState::migrate_runtime`] so ephemeral
/// `#[serde(skip)]` fields (flash vecs, count-up tweens, etc.) are seeded.
///
/// Versions outside the known set deserialize as default — this is
/// pessimistic on purpose (a future version is more likely to have new
/// fields than the current code can interpret).
pub fn load_from_str(json: &str) -> GameState {
    match migrate::peek_version(json) {
        1 => match serde_json::from_str::<versions::v1::GameStateV1>(json) {
            Ok(v1) => v1.into_current().migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        _ => GameState::default().migrate_runtime(),
    }
}

/// Serialize the live state to its on-disk JSON form. The `version` field
/// is whatever the caller has on `state` — `GameState::default()` and the
/// migration chain both stamp [`CURRENT_VERSION`], so the only way to write
/// a wrong version is to mutate `state.version` by hand, which nothing does.
pub fn save_to_string(state: &GameState) -> serde_json::Result<String> {
    serde_json::to_string_pretty(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_versioned_json_loads_through_v1() {
        // A save written by main (no `version` field, today's shape) must
        // load without losing data.
        let legacy = r#"{
            "cuques": 1234.5,
            "total_clicks": 99,
            "lifetime_cuques": 1234.5,
            "best_fps": 0.0,
            "golden_caught": 0,
            "fingerers_owned": {"index_finger": 7},
            "achievements_earned": ["first_finger"],
            "upgrades_earned": ["click_mult_1"],
            "prestige": 0,
            "total_play_ticks": 0,
            "buffs": []
        }"#;
        let s = load_from_str(legacy);
        assert_eq!(s.version, CURRENT_VERSION);
        assert_eq!(s.cuques, 1234.5);
        assert_eq!(s.total_clicks, 99);
        assert_eq!(s.fingerer_count("index_finger"), 7);
        assert!(s.has_upgrade("click_mult_1"));
        assert!(s.has_achievement("first_finger"));
    }

    #[test]
    fn malformed_json_falls_back_to_default() {
        let s = load_from_str("{ not valid json");
        assert_eq!(s.cuques, 0.0);
        assert_eq!(s.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip_through_save_to_string_preserves_state() {
        let original = GameState {
            cuques: 4242.0,
            total_clicks: 17,
            ..GameState::default()
        };
        let json = save_to_string(&original).expect("serialize");
        let loaded = load_from_str(&json);
        assert_eq!(loaded.cuques, 4242.0);
        assert_eq!(loaded.total_clicks, 17);
        assert_eq!(loaded.version, CURRENT_VERSION);
    }
}
