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
pub const CURRENT_VERSION: u32 = 5;

/// Best-effort load from a JSON string. Falls back to a default state if
/// the input is malformed at any layer of the chain. The result is always
/// passed through [`GameState::migrate_runtime`] so ephemeral
/// `#[serde(skip)]` fields (flash vecs, count-up tweens, etc.) are seeded.
///
/// Versions outside the known set deserialize as default — this is
/// pessimistic on purpose (a future version is more likely to have new
/// fields than the current code can interpret).
pub fn load_from_str(json: &str) -> GameState {
    use versions::{v1, v2, v3, v4, v5};
    match migrate::peek_version(json) {
        1 => match serde_json::from_str::<v1::GameStateV1>(json) {
            Ok(v) => v5::GameStateV5::from(v4::GameStateV4::from(v3::GameStateV3::from(
                v2::GameStateV2::from(v),
            )))
            .into_current()
            .migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        2 => match serde_json::from_str::<v2::GameStateV2>(json) {
            Ok(v) => v5::GameStateV5::from(v4::GameStateV4::from(v3::GameStateV3::from(v)))
                .into_current()
                .migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        3 => match serde_json::from_str::<v3::GameStateV3>(json) {
            Ok(v) => v5::GameStateV5::from(v4::GameStateV4::from(v))
                .into_current()
                .migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        4 => match serde_json::from_str::<v4::GameStateV4>(json) {
            Ok(v) => v5::GameStateV5::from(v).into_current().migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        5 => match serde_json::from_str::<v5::GameStateV5>(json) {
            Ok(v) => v.into_current().migrate_runtime(),
            Err(_) => GameState::default().migrate_runtime(),
        },
        _ => GameState::default().migrate_runtime(),
    }
}

/// Serialize the live state to its on-disk JSON form. The `version` field
/// is whatever the caller has on `state` — `GameState::default()` and the
/// migration chain both stamp [`CURRENT_VERSION`], so the only way to write
/// a wrong version is to mutate `state.version` by hand, which nothing does.
///
/// Sanitizes non-finite f64 fields (NaN / INFINITY) to 0.0 before
/// serializing, since `serde_json` refuses to serialize non-finite f64
/// and historically callers `let _ =`-swallowed the resulting Err — the
/// player's progress would silently stop being written. Reaching a
/// non-finite value normally is impossible, but a corrupted save loaded
/// once can poison `cuques` / `lifetime_cuques` and we'd rather lose
/// the corruption than lose subsequent saves.
pub fn save_to_string(state: &GameState) -> serde_json::Result<String> {
    // With Mag-typed counters, "non-finite" is no longer reachable —
    // `Mag` stores log10 and saturates on impossible inputs. The clone+
    // sanitize dance is preserved as belt-and-suspenders against any
    // future buggy assignment that puts a NaN into the log10 field; if
    // we ever see one, fall the affected counter back to zero.
    let mut sanitized = state.clone();
    if sanitized.cuques.log10.is_nan() {
        sanitized.cuques = crate::bignum::Mag::ZERO;
    }
    if sanitized.lifetime_cuques.log10.is_nan() {
        sanitized.lifetime_cuques = crate::bignum::Mag::ZERO;
    }
    if sanitized.best_fps.log10.is_nan() {
        sanitized.best_fps = crate::bignum::Mag::ZERO;
    }
    serde_json::to_string_pretty(&sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_versioned_json_loads_through_v1() {
        // A save written by main (no `version` field, V1 shape) must load
        // without losing data. V4 silently drops `upgrades_earned` (the
        // 1.0.0 breaking change), so we only assert the surviving fields.
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
        assert_eq!(s.cuques, crate::bignum::Mag::from_f64(1234.5));
        assert_eq!(s.total_clicks, 99);
        assert_eq!(s.fingerer_count("index_finger"), 7);
        assert!(s.has_achievement("first_finger"));
        // V3→V4 dropped the old upgrades_earned. The tree starts with
        // the anchor (origin) auto-owned — `migrate_runtime` inserts it
        // for any save that didn't have it.
        assert_eq!(s.tree.bought.len(), 1);
        assert!(
            s.tree
                .bought
                .contains(&crate::game::tree::coord::TreeCoord::ORIGIN)
        );
    }

    #[test]
    fn malformed_json_falls_back_to_default() {
        let s = load_from_str("{ not valid json");
        assert_eq!(s.cuques, crate::bignum::Mag::ZERO);
        assert_eq!(s.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip_through_save_to_string_preserves_state() {
        let original = GameState {
            cuques: crate::bignum::Mag::from_f64(4242.0),
            total_clicks: 17,
            ..GameState::default()
        };
        let json = save_to_string(&original).expect("serialize");
        let loaded = load_from_str(&json);
        assert_eq!(loaded.cuques, crate::bignum::Mag::from_f64(4242.0));
        assert_eq!(loaded.total_clicks, 17);
        assert_eq!(loaded.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip_preserves_huge_mag_values_past_f64_range() {
        // Regression for the V5-blocker: before the bump, `Mag` serialized
        // as `{"log10": x}` once it cleared `f64`-finite range, but the
        // V4 frozen schema's `cuques: f64` rejected the struct shape and
        // `load_from_str` silently fell back to `default()` — wiping the
        // entire save. With V5 declaring Mag-typed counters, the struct
        // form is now part of the schema and the round-trip survives.
        let original = GameState {
            cuques: crate::bignum::Mag { log10: 600.0 },
            lifetime_cuques: crate::bignum::Mag { log10: 1200.0 },
            best_fps: crate::bignum::Mag { log10: 750.0 },
            ..GameState::default()
        };
        let json = save_to_string(&original).expect("serialize");
        let loaded = load_from_str(&json);
        assert!((loaded.cuques.log10 - 600.0).abs() < 1e-9);
        assert!((loaded.lifetime_cuques.log10 - 1200.0).abs() < 1e-9);
        assert!((loaded.best_fps.log10 - 750.0).abs() < 1e-9);
    }

    #[test]
    fn v4_json_with_plain_number_fields_still_loads_under_v5() {
        // A V4-shaped save (`"cuques": 1234.5`) must still parse cleanly
        // through the V5 reader. The `Mag` untagged-serde shim accepts
        // bare JSON numbers exactly because of this requirement.
        let v4_json = r#"{
            "version": 4,
            "cuques": 1234.5,
            "total_clicks": 88,
            "lifetime_cuques": 6789.0,
            "best_fps": 12.0,
            "golden_caught": 0,
            "lucky_caught": 0,
            "frenzy_caught": 0,
            "buff_caught": 0,
            "green_coin_caught": 0,
            "fingerers_state": {},
            "achievements_earned": [],
            "prestige": 2,
            "total_play_ticks": 4242,
            "buffs": [],
            "tree": { "bought": [], "cursor": {"x": 0, "y": 0}, "last_bought": null }
        }"#;
        let loaded = load_from_str(v4_json);
        assert_eq!(loaded.version, CURRENT_VERSION);
        assert!((loaded.cuques.to_f64() - 1234.5).abs() < 1e-9);
        assert!((loaded.lifetime_cuques.to_f64() - 6789.0).abs() < 1e-9);
        assert_eq!(loaded.total_clicks, 88);
        assert_eq!(loaded.prestige, 2);
    }
}
