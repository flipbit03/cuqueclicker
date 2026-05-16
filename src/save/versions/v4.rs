//! Save schema V4 — drops the old hardcoded `upgrades_earned` set entirely
//! and introduces the infinite procedural upgrade tree.
//!
//! V3 → V4 is a **breaking change**. The old `UPGRADES` catalog is gone;
//! every entry the player had earned under V3 is silently dropped. In
//! exchange the player's V4 save carries an `UpgradeTreeState` whose
//! `bought` set is empty — they start fresh on the new tree at lot
//! `(0, 0)`.
//!
//! Once V4 is on `main` this file is FROZEN. Subsequent schema changes go
//! in `v5.rs` together with a `From<GameStateV4> for GameStateV5`
//! conversion.
//!
//! Each persisted enum/struct re-uses V2's frozen copies (modifier source,
//! effect, duration, modifier, fingerer state, buff). Only the top-level
//! `GameState` shape changes between V3 and V4.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::v2::{BuffV2, FingererStateV2};
use super::v3::GameStateV3;
use crate::game::tree::UpgradeTreeState;

fn default_v4_version() -> u32 {
    4
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GameStateV4 {
    #[serde(default = "default_v4_version")]
    pub version: u32,
    #[serde(default)]
    pub cuques: f64,
    #[serde(default)]
    pub total_clicks: u64,
    #[serde(default)]
    pub lifetime_cuques: f64,
    #[serde(default)]
    pub best_fps: f64,
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
    /// V4 addition: the infinite upgrade tree state. Replaces the old
    /// hardcoded `upgrades_earned: HashSet<String>` entirely.
    #[serde(default)]
    pub tree: UpgradeTreeState,
}

// V4's old `into_current` lived here when V4 was the current schema. The
// V5 bump moved the live-state conversion to `v5::GameStateV5::into_current`
// (the live `GameState` shape now requires `Mag` counters, which the V4
// frozen schema doesn't carry). V4 saves on disk continue to load — the
// chain now walks `v4 → v5 → GameState` — but the V4 module itself only
// owns the persisted shape and the `From<GameStateV3>` step.

/// V3 → V4 conversion. Drops `upgrades_earned` (old hardcoded UPGRADES
/// catalog is retired); inits a fresh empty `UpgradeTreeState`. Every
/// other field passes through verbatim; `version` is stamped to 4.
///
/// The breaking change is intentional — coordinated with the game's
/// 1.0.0 release. Players who upgrade lose their old hand-curated
/// upgrade purchases but keep cuques, fingerers, achievements, prestige,
/// and active buffs.
impl From<GameStateV3> for GameStateV4 {
    fn from(v3: GameStateV3) -> Self {
        GameStateV4 {
            version: 4,
            cuques: v3.cuques,
            total_clicks: v3.total_clicks,
            lifetime_cuques: v3.lifetime_cuques,
            best_fps: v3.best_fps,
            golden_caught: v3.golden_caught,
            lucky_caught: v3.lucky_caught,
            frenzy_caught: v3.frenzy_caught,
            buff_caught: v3.buff_caught,
            green_coin_caught: v3.green_coin_caught,
            fingerers_state: v3.fingerers_state,
            achievements_earned: v3.achievements_earned,
            // upgrades_earned silently dropped — old catalog is gone.
            prestige: v3.prestige,
            total_play_ticks: v3.total_play_ticks,
            buffs: v3.buffs,
            tree: UpgradeTreeState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::tree::coord::TreeCoord;
    use crate::save::versions::v2::{
        FingererStateV2, ModifierDurationV2, ModifierEffectV2, ModifierSourceV2, ModifierV2,
    };

    fn empty_v3() -> GameStateV3 {
        GameStateV3 {
            version: 3,
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
        }
    }

    #[test]
    fn v3_to_v4_drops_upgrades_earned_silently() {
        let mut v3 = empty_v3();
        v3.upgrades_earned.insert("click_mult_1".into());
        v3.upgrades_earned.insert("hand_of_god_mult_3".into());
        let v4: GameStateV4 = v3.into();
        assert_eq!(v4.version, 4);
        // V4 has no upgrades_earned field at all — the JSON round-trip
        // proves it doesn't sneak through.
        let json = serde_json::to_string(&v4).expect("serialize");
        assert!(
            !json.contains("upgrades_earned"),
            "old upgrades field must not appear in V4 JSON: {json}"
        );
    }

    #[test]
    fn v3_to_v4_inits_empty_tree() {
        let v4: GameStateV4 = empty_v3().into();
        assert!(v4.tree.bought.is_empty());
        assert_eq!(v4.tree.cursor, TreeCoord::ORIGIN);
        assert_eq!(v4.tree.last_bought, None);
    }

    #[test]
    fn v3_to_v4_preserves_all_other_fields() {
        let v3 = GameStateV3 {
            version: 3,
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
        };

        let v4: GameStateV4 = v3.into();

        assert_eq!(v4.cuques, 1234.5);
        assert_eq!(v4.total_clicks, 99);
        assert_eq!(v4.lifetime_cuques, 5_000.0);
        assert_eq!(v4.best_fps, 12.0);
        assert_eq!(v4.golden_caught, 17);
        assert_eq!(v4.lucky_caught, 10);
        assert_eq!(v4.frenzy_caught, 4);
        assert_eq!(v4.buff_caught, 2);
        assert_eq!(v4.green_coin_caught, 1);
        assert_eq!(v4.prestige, 3);
        assert_eq!(v4.total_play_ticks, 9_000);
        assert_eq!(v4.buffs.len(), 1);
        let st = v4.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.count, 9);
        assert_eq!(st.modifiers.len(), 1);
        assert!(v4.achievements_earned.contains("first_finger"));
    }

    #[test]
    fn v4_through_v5_preserves_tree_state() {
        // V4-on-disk → live state goes through V5 now. Round the V4
        // fixture through the chain and confirm the bought set survives.
        let mut v4 = GameStateV4 {
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
        };
        v4.tree.bought.insert(TreeCoord::ORIGIN);

        let live = crate::save::versions::v5::GameStateV5::from(v4).into_current();
        assert!(live.tree.bought.contains(&TreeCoord::ORIGIN));
    }
}
