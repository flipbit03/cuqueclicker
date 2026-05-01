//! Save schema V3 — splits the rolled-up `golden_caught` stat into four
//! per-variant counters (`lucky_caught`, `frenzy_caught`, `buff_caught`,
//! `green_coin_caught`) for the stats panel breakdown.
//!
//! WORK IN PROGRESS until the next PR merges. Once on `main` this file is
//! FROZEN; subsequent schema changes go in `v4.rs`.
//!
//! The `golden_caught` field stays — it's the lifetime grand total across
//! every variant, kept stable for back-compat with existing achievements
//! that gate on it. Pre-V3 saves have no per-variant breakdown to recover,
//! so the four new counters default to 0 and only track post-V3 catches.
//! This is honest data: total accurate, breakdown only counts what the
//! game actually saw.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::v2::{BuffV2, FingererStateV2, GameStateV2};
use crate::game::state::{Buff, FingererState, GameState};

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
    /// Lifetime grand total across every powerup variant. Strict rollup;
    /// achievements key off this.
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
    #[serde(default)]
    pub goldens_since_green_coin: u32,
}

impl GameStateV3 {
    pub fn into_current(self) -> GameState {
        let fingerers_state: HashMap<String, FingererState> = self
            .fingerers_state
            .into_iter()
            .map(|(id, st)| (id, st.into()))
            .collect();
        let buffs: Vec<Buff> = self.buffs.into_iter().map(Into::into).collect();
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
            goldens_since_green_coin: self.goldens_since_green_coin,
            ..GameState::default()
        }
    }
}

/// V2 → V3: zero-init the four new per-variant counters. The pre-V3
/// rollup `golden_caught` carries forward as the grand total, so no
/// player progress is lost — the *breakdown* is what's missing for
/// catches that happened before V3 shipped, not the *count*.
impl From<GameStateV2> for GameStateV3 {
    fn from(v2: GameStateV2) -> Self {
        GameStateV3 {
            version: 3,
            cuques: v2.cuques,
            total_clicks: v2.total_clicks,
            lifetime_cuques: v2.lifetime_cuques,
            best_fps: v2.best_fps,
            golden_caught: v2.golden_caught,
            lucky_caught: 0,
            frenzy_caught: 0,
            buff_caught: 0,
            green_coin_caught: 0,
            fingerers_state: v2.fingerers_state,
            achievements_earned: v2.achievements_earned,
            upgrades_earned: v2.upgrades_earned,
            prestige: v2.prestige,
            total_play_ticks: v2.total_play_ticks,
            buffs: v2.buffs,
            goldens_since_green_coin: v2.goldens_since_green_coin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_v2() -> GameStateV2 {
        GameStateV2 {
            version: 2,
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_state: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
            goldens_since_green_coin: 0,
        }
    }

    #[test]
    fn v2_to_v3_zero_inits_per_variant_counters() {
        let v2 = GameStateV2 {
            golden_caught: 17,
            ..empty_v2()
        };

        let v3: GameStateV3 = v2.into();

        // Rollup carried forward verbatim.
        assert_eq!(v3.golden_caught, 17);
        // Breakdown starts fresh — pre-V3 catches weren't recorded that way.
        assert_eq!(v3.lucky_caught, 0);
        assert_eq!(v3.frenzy_caught, 0);
        assert_eq!(v3.buff_caught, 0);
        assert_eq!(v3.green_coin_caught, 0);
        assert_eq!(v3.version, 3);
    }

    #[test]
    fn v3_into_current_preserves_all_counters() {
        let v3 = GameStateV3 {
            version: 3,
            cuques: 1.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 100,
            lucky_caught: 60,
            frenzy_caught: 20,
            buff_caught: 15,
            green_coin_caught: 5,
            fingerers_state: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
            goldens_since_green_coin: 0,
        };

        let live = v3.into_current();

        assert_eq!(live.golden_caught, 100);
        assert_eq!(live.lucky_caught, 60);
        assert_eq!(live.frenzy_caught, 20);
        assert_eq!(live.buff_caught, 15);
        assert_eq!(live.green_coin_caught, 5);
    }
}
