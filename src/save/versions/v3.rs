//! Save schema V3 — drops `goldens_since_green_coin`.
//!
//! V3's only persisted-shape change vs V2 is the removal of the Green Coin
//! pity counter. The Vec-based powerup refactor (#25) replaced the pity
//! mechanic with per-kind cooldown clocks (one for each `PowerupKind`),
//! all of which are `#[serde(skip)]`. The pity counter became dead weight
//! on disk — a field present in every V2 save that the live state doesn't
//! consult anymore. V3 drops it.
//!
//! Once V3 is on `main` this file is FROZEN: subsequent schema changes
//! go in `v4.rs` together with a `From<GameStateV3>` conversion.
//!
//! Each persisted enum/struct re-uses V2's frozen copies (modifier source,
//! effect, duration, modifier, fingerer state, buff). The struct-level
//! shape change is small enough that re-vendoring the inner types would
//! be pure noise.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::v2::{BuffV2, FingererStateV2, GameStateV2};
use crate::game::state::GameState;

fn default_v3_version() -> u32 {
    3
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GameStateV3 {
    #[serde(default = "default_v3_version")]
    pub version: u32,
    #[serde(default)]
    pub cuques: f64,
    #[serde(default)]
    pub total_clicks: u64,
    #[serde(default)]
    pub lifetime_cuques: f64,
    #[serde(default)]
    pub best_fps: f64,
    /// Lifetime grand total across every powerup kind. Strict rollup;
    /// existing achievements (Golden Touch, Golden Hoarder) gate on this.
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
    pub upgrades_earned: HashSet<String>,
    #[serde(default)]
    pub prestige: u64,
    #[serde(default)]
    pub total_play_ticks: u64,
    #[serde(default)]
    pub buffs: Vec<BuffV2>,
}

impl GameStateV3 {
    /// Convert a V3 snapshot into the live `GameState`. Every persisted
    /// field is copied verbatim; ephemeral state (`#[serde(skip)]` fields,
    /// including `powerups`/`next_spawn_id`/`powerup_cooldowns`) stays at
    /// its `Default` and gets seeded by `migrate_runtime` after the chain
    /// finishes.
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
            upgrades_earned: self.upgrades_earned,
            prestige: self.prestige,
            total_play_ticks: self.total_play_ticks,
            buffs,
            ..GameState::default()
        }
    }
}

/// V2 → V3 conversion. Drops `goldens_since_green_coin`. Every other field
/// passes through verbatim; `version` is stamped to 3.
impl From<GameStateV2> for GameStateV3 {
    fn from(v2: GameStateV2) -> Self {
        GameStateV3 {
            version: 3,
            cuques: v2.cuques,
            total_clicks: v2.total_clicks,
            lifetime_cuques: v2.lifetime_cuques,
            best_fps: v2.best_fps,
            golden_caught: v2.golden_caught,
            lucky_caught: v2.lucky_caught,
            frenzy_caught: v2.frenzy_caught,
            buff_caught: v2.buff_caught,
            green_coin_caught: v2.green_coin_caught,
            fingerers_state: v2.fingerers_state,
            achievements_earned: v2.achievements_earned,
            upgrades_earned: v2.upgrades_earned,
            prestige: v2.prestige,
            total_play_ticks: v2.total_play_ticks,
            buffs: v2.buffs,
            // Pity counter dropped — Vec-based powerups use per-kind
            // cooldown clocks instead, all of which are `#[serde(skip)]`.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::versions::v2::{
        BuffV2, FingererStateV2, ModifierDurationV2, ModifierEffectV2, ModifierSourceV2, ModifierV2,
    };

    fn empty_v2_with_pity(pity: u32) -> GameStateV2 {
        GameStateV2 {
            version: 2,
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
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
            goldens_since_green_coin: pity,
        }
    }

    #[test]
    fn v2_to_v3_drops_pity_counter() {
        // V2 has the field; V3's struct shape doesn't, so any non-zero
        // pity value just disappears. The rest of the persisted state
        // must round-trip unchanged.
        let v2 = empty_v2_with_pity(7);
        let v3: GameStateV3 = v2.into();
        assert_eq!(v3.version, 3);
    }

    #[test]
    fn v2_to_v3_preserves_all_other_fields() {
        let v2 = GameStateV2 {
            version: 2,
            cuques: 1234.5,
            total_clicks: 99,
            lifetime_cuques: 5_000.0,
            best_fps: 12.0,
            golden_caught: 17,
            lucky_caught: 10,
            frenzy_caught: 4,
            buff_caught: 2,
            green_coin_caught: 1,
            fingerers_state: [(
                "index_finger".to_string(),
                FingererStateV2 {
                    count: 9,
                    modifiers: vec![ModifierV2 {
                        source: ModifierSourceV2::GreenCoin,
                        effects: vec![ModifierEffectV2::AddPercent(0.10)],
                        duration: ModifierDurationV2::Permanent,
                        created_at_tick: 0,
                    }],
                },
            )]
            .into_iter()
            .collect(),
            achievements_earned: ["first_finger".into()].into_iter().collect(),
            upgrades_earned: ["click_mult_1".into()].into_iter().collect(),
            prestige: 3,
            total_play_ticks: 9_000,
            buffs: vec![BuffV2::ClickFrenzy {
                ticks_remaining: 100,
                initial_ticks: 260,
                mult: 777.0,
            }],
            goldens_since_green_coin: 5,
        };

        let v3: GameStateV3 = v2.into();

        assert_eq!(v3.cuques, 1234.5);
        assert_eq!(v3.total_clicks, 99);
        assert_eq!(v3.lifetime_cuques, 5_000.0);
        assert_eq!(v3.best_fps, 12.0);
        assert_eq!(v3.golden_caught, 17);
        assert_eq!(v3.lucky_caught, 10);
        assert_eq!(v3.frenzy_caught, 4);
        assert_eq!(v3.buff_caught, 2);
        assert_eq!(v3.green_coin_caught, 1);
        assert_eq!(v3.prestige, 3);
        assert_eq!(v3.total_play_ticks, 9_000);
        assert_eq!(v3.buffs.len(), 1);
        let st = v3.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.count, 9);
        assert_eq!(st.modifiers.len(), 1);
        assert!(v3.achievements_earned.contains("first_finger"));
        assert!(v3.upgrades_earned.contains("click_mult_1"));
    }

    #[test]
    fn v3_into_current_zero_inits_powerup_runtime_fields() {
        // V3 doesn't persist any powerup-on-screen state; `into_current`
        // must produce a live state whose `powerups` Vec is empty and
        // whose `next_spawn_id` is 0.
        let v3 = empty_v2_with_pity(0).into();
        let live = GameStateV3::into_current(v3);
        assert!(live.powerups.is_empty());
        assert_eq!(live.next_spawn_id, 0);
    }
}
