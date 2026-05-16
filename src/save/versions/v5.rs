//! Save schema V5 — moves cuque-side counters and multiplier aggregates to
//! [`crate::bignum::Mag`] (log-magnitude) so end-game stacks no longer
//! overflow `f64` to `Infinity`.
//!
//! The persisted *wire* shape is intentionally backward-compatible: `Mag`
//! ships an untagged serde shim that accepts both legacy raw JSON numbers
//! (everything written before this bump used `"cuques": 1234.5`) and the
//! explicit `{"log10": x}` form (new huge values past `f64::MAX`). So a
//! V5 reader still parses every V4-written file; a V5 writer only emits
//! the struct form when a value's log10 exceeds the `f64` range, keeping
//! "small" saves indistinguishable from V4 on disk.
//!
//! V4 → V5 is mechanical: every `f64` counter / aggregate / cost becomes
//! `Mag::from_f64(old)`, which collapses any `Inf` / `NaN` left behind by
//! the runaway-overflow bug class V5 exists to kill into `Mag::ZERO`
//! rather than reproducing it.
//!
//! Once V5 is on `main` this file is FROZEN. Subsequent schema changes
//! go in `v6.rs` with a `From<GameStateV5>` conversion.
//!
//! Note: V5's persisted shape *is* the live `GameState` shape — moving
//! to a Mag-typed counter set is a live-state change as much as a save
//! change, so V5 has no new fields relative to V4. The whole point of
//! V5 is the field *type* swap from `f64` → `Mag`.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::v2::{BuffV2, FingererStateV2};
use super::v4::GameStateV4;
use crate::bignum::Mag;
use crate::game::state::GameState;
use crate::game::tree::UpgradeTreeState;

fn default_v5_version() -> u32 {
    5
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GameStateV5 {
    #[serde(default = "default_v5_version")]
    pub version: u32,
    #[serde(default)]
    pub cuques: Mag,
    #[serde(default)]
    pub total_clicks: u64,
    #[serde(default)]
    pub lifetime_cuques: Mag,
    #[serde(default)]
    pub best_fps: Mag,
    #[serde(default)]
    pub golden_caught: u64,
    #[serde(default)]
    pub lucky_caught: u64,
    #[serde(default)]
    pub frenzy_caught: u64,
    #[serde(default)]
    pub buff_caught: u64,
    #[serde(default)]
    pub green_coin_caught: u64,
    #[serde(default)]
    pub fingerers_state: HashMap<String, FingererStateV2>,
    #[serde(default)]
    pub achievements_earned: HashSet<String>,
    #[serde(default)]
    pub prestige: u64,
    #[serde(default)]
    pub total_play_ticks: u64,
    #[serde(default)]
    pub buffs: Vec<BuffV2>,
    #[serde(default)]
    pub tree: UpgradeTreeState,
}

impl GameStateV5 {
    /// Convert a V5 snapshot into the live `GameState`. The persisted
    /// fields copy verbatim; ephemeral state (`#[serde(skip)]` fields)
    /// stays at `Default` and is seeded by `migrate_runtime`.
    pub fn into_current(self) -> GameState {
        let fingerers_state = self
            .fingerers_state
            .into_iter()
            .map(|(id, st)| (id, st.into()))
            .collect();
        let buffs = self.buffs.into_iter().map(Into::into).collect();
        GameState {
            version: crate::save::CURRENT_VERSION,
            cuques: self.cuques,
            total_clicks: self.total_clicks,
            lifetime_cuques: self.lifetime_cuques,
            best_fps: self.best_fps,
            golden_caught: self.golden_caught,
            lucky_caught: self.lucky_caught,
            frenzy_caught: self.frenzy_caught,
            buff_caught: self.buff_caught,
            green_coin_caught: self.green_coin_caught,
            fingerers_state,
            achievements_earned: self.achievements_earned,
            prestige: self.prestige,
            total_play_ticks: self.total_play_ticks,
            buffs,
            tree: self.tree,
            ..GameState::default()
        }
    }
}

/// V4 → V5 conversion. Each f64 counter gets `Mag::from_f64`'d at the
/// boundary; finite values copy faithfully, and `Inf` / `NaN` left over
/// from corrupted V4 saves (the original bug we triaged had
/// `cuques = 0.0` because the V4 saver had refused-to-serialize an
/// `Infinity` and silently fell through to a zero) collapse to
/// `Mag::ZERO`. Everything else passes through verbatim.
impl From<GameStateV4> for GameStateV5 {
    fn from(v4: GameStateV4) -> Self {
        GameStateV5 {
            version: 5,
            cuques: Mag::from_f64(v4.cuques),
            total_clicks: v4.total_clicks,
            lifetime_cuques: Mag::from_f64(v4.lifetime_cuques),
            best_fps: Mag::from_f64(v4.best_fps),
            golden_caught: v4.golden_caught,
            lucky_caught: v4.lucky_caught,
            frenzy_caught: v4.frenzy_caught,
            buff_caught: v4.buff_caught,
            green_coin_caught: v4.green_coin_caught,
            fingerers_state: v4.fingerers_state,
            achievements_earned: v4.achievements_earned,
            prestige: v4.prestige,
            total_play_ticks: v4.total_play_ticks,
            buffs: v4.buffs,
            tree: v4.tree,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tree::coord::TreeCoord;

    fn empty_v4() -> GameStateV4 {
        GameStateV4 {
            version: 4,
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            lucky_caught: 0,
            frenzy_caught: 0,
            buff_caught: 0,
            green_coin_caught: 0,
            fingerers_state: HashMap::new(),
            achievements_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
            tree: UpgradeTreeState::default(),
        }
    }

    #[test]
    fn v4_to_v5_promotes_f64_counters_to_mag() {
        let v4 = GameStateV4 {
            cuques: 1.5e30,
            lifetime_cuques: 1.5e30,
            best_fps: 5_000.0,
            ..empty_v4()
        };
        let v5: GameStateV5 = v4.into();
        assert_eq!(v5.version, 5);
        // Round-trip through Mag: log10(1.5e30) ≈ 30.176.
        assert!((v5.cuques.to_f64() - 1.5e30).abs() / 1.5e30 < 1e-12);
        assert!((v5.best_fps.to_f64() - 5_000.0).abs() < 1e-9);
    }

    #[test]
    fn v4_to_v5_collapses_nan_and_inf_to_zero() {
        // The exact failure mode of the wild "broken save": cuques was
        // serialized as 0.0 because serde refused the underlying f64::Inf.
        // After this migration `Mag::from_f64` on any non-finite value
        // produces `Mag::ZERO` — we never re-introduce the Inf to live
        // state.
        let v4 = GameStateV4 {
            cuques: f64::INFINITY,
            lifetime_cuques: f64::NAN,
            best_fps: f64::NEG_INFINITY,
            ..empty_v4()
        };
        let v5: GameStateV5 = v4.into();
        assert_eq!(v5.cuques, Mag::ZERO);
        assert_eq!(v5.lifetime_cuques, Mag::ZERO);
        assert_eq!(v5.best_fps, Mag::ZERO);
    }

    #[test]
    fn v5_into_current_preserves_tree_state() {
        let mut v5: GameStateV5 = empty_v4().into();
        v5.tree.bought.insert(TreeCoord::ORIGIN);
        v5.tree.bought.insert(TreeCoord::new(3, -2));
        let live = v5.into_current();
        assert!(live.tree.bought.contains(&TreeCoord::ORIGIN));
        assert!(live.tree.bought.contains(&TreeCoord::new(3, -2)));
    }

    #[test]
    fn v5_serialize_huge_value_round_trips() {
        // The blocker that motivated V5: a `Mag` with log10 past the
        // f64-finite range needs to serialize and deserialize through
        // the V5 frozen schema without falling back to `default()`.
        // The `Mag` shim emits `{"log10": x}` for big values; the V5
        // schema declares `Mag`-typed fields so the struct form is
        // accepted.
        let mut v5: GameStateV5 = empty_v4().into();
        v5.cuques = Mag { log10: 600.0 };
        v5.lifetime_cuques = Mag { log10: 1200.0 };
        let json = serde_json::to_string(&v5).expect("serialize");
        let parsed: GameStateV5 = serde_json::from_str(&json).expect("deserialize");
        assert!((parsed.cuques.log10 - 600.0).abs() < 1e-9);
        assert!((parsed.lifetime_cuques.log10 - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn v5_serialize_small_value_emits_plain_number() {
        // For values comfortably inside `f64`, the wire format stays a
        // plain JSON number — same shape every previous version
        // produced. Confirms V4 readers (and humans) reading a V5 save
        // see familiar text for typical play.
        let mut v5: GameStateV5 = empty_v4().into();
        v5.cuques = Mag::from_f64(12345.6);
        let json = serde_json::to_string(&v5.cuques).expect("serialize");
        assert!(
            !json.contains("log10"),
            "small Mag should serialize as a bare number, got {json}"
        );
    }
}
