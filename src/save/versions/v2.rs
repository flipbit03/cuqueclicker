//! Save schema V2 — adds the `version` field, per-fingerer modifiers, and
//! Green-Coin spawn pity counter.
//!
//! WORK IN PROGRESS until the Green Coin PR (#21) merges. Once that PR
//! lands on `main` this file is FROZEN: subsequent schema changes go in
//! `v3.rs` together with a `From<GameStateV2> for GameStateV3` conversion
//! and a unit test.
//!
//! Each persisted enum/struct has a frozen V2 copy here so future changes
//! to the live types in `crate::game::modifier` and `crate::game::state`
//! can't retroactively reshape what V2 means on disk.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::v1::{BuffV1, GameStateV1};
use crate::game::modifier::{
    FingererAggregate, Modifier, ModifierDuration, ModifierEffect, ModifierSource,
};
use crate::game::state::{Buff, FingererState, GameState};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ModifierSourceV2 {
    GreenCoin,
    PurpleCoin,
}

impl From<ModifierSourceV2> for ModifierSource {
    fn from(s: ModifierSourceV2) -> Self {
        match s {
            ModifierSourceV2::GreenCoin => ModifierSource::GreenCoin,
            ModifierSourceV2::PurpleCoin => ModifierSource::PurpleCoin,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum ModifierEffectV2 {
    FlatFps(f64),
    AddPercent(f64),
    MulFactor(f64),
}

impl From<ModifierEffectV2> for ModifierEffect {
    fn from(e: ModifierEffectV2) -> Self {
        match e {
            ModifierEffectV2::FlatFps(v) => ModifierEffect::FlatFps(v),
            ModifierEffectV2::AddPercent(v) => ModifierEffect::AddPercent(v),
            ModifierEffectV2::MulFactor(v) => ModifierEffect::MulFactor(v),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModifierDurationV2 {
    Permanent,
    Ticks(u32),
}

impl From<ModifierDurationV2> for ModifierDuration {
    fn from(d: ModifierDurationV2) -> Self {
        match d {
            ModifierDurationV2::Permanent => ModifierDuration::Permanent,
            ModifierDurationV2::Ticks(n) => ModifierDuration::Ticks(n),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModifierV2 {
    pub source: ModifierSourceV2,
    pub effects: Vec<ModifierEffectV2>,
    pub duration: ModifierDurationV2,
    #[serde(default)]
    pub created_at_tick: u64,
}

impl From<ModifierV2> for Modifier {
    fn from(m: ModifierV2) -> Self {
        Modifier {
            source: m.source.into(),
            effects: m.effects.into_iter().map(Into::into).collect(),
            duration: m.duration.into(),
            created_at_tick: m.created_at_tick,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FingererStateV2 {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub modifiers: Vec<ModifierV2>,
}

impl From<FingererStateV2> for FingererState {
    fn from(v: FingererStateV2) -> Self {
        let modifiers: Vec<Modifier> = v.modifiers.into_iter().map(Into::into).collect();
        let aggregate = FingererAggregate::rebuild(&modifiers);
        FingererState {
            count: v.count,
            modifiers,
            aggregate,
        }
    }
}

/// V2 mirror of `Buff`. At V2-as-shipped-by-this-PR-Phase-2 the live `Buff`
/// still carries `FingererBoost`; Phase 3 will collapse that variant into
/// the modifier system, at which point the live `Buff` shrinks but `BuffV2`
/// stays as-is (frozen) and the V2→Current path absorbs `FingererBoost`
/// into modifiers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BuffV2 {
    ClickFrenzy {
        ticks_remaining: u32,
        initial_ticks: u32,
        mult: f64,
    },
    FingererBoost {
        ticks_remaining: u32,
        initial_ticks: u32,
        fingerer_id: String,
        mult: f64,
    },
}

impl From<BuffV2> for Buff {
    fn from(b: BuffV2) -> Self {
        match b {
            BuffV2::ClickFrenzy {
                ticks_remaining,
                initial_ticks,
                mult,
            } => Buff::ClickFrenzy {
                ticks_remaining,
                initial_ticks,
                mult,
            },
            BuffV2::FingererBoost {
                ticks_remaining,
                initial_ticks,
                fingerer_id,
                mult,
            } => Buff::FingererBoost {
                ticks_remaining,
                initial_ticks,
                fingerer_id,
                mult,
            },
        }
    }
}

fn default_v2_version() -> u32 {
    2
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GameStateV2 {
    #[serde(default = "default_v2_version")]
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

impl GameStateV2 {
    /// Convert a V2 snapshot into the live `GameState`. Every persisted
    /// field is copied verbatim; ephemeral state (`#[serde(skip)]` fields)
    /// stays at its `Default` and gets seeded by `migrate_runtime` after
    /// the chain finishes.
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

/// V1 → V2 conversion. The shape change is:
///   - `fingerers_owned: HashMap<String, u32>` →
///     `fingerers_state: HashMap<String, FingererStateV2 { count, modifiers: vec![] }>`
///   - `buffs: Vec<BuffV1>` → `buffs: Vec<BuffV2>` (verbatim re-tag).
///   - Adds the `version: 2` field.
///
/// Phase 3 of the Green Coin PR will extend this to also absorb V1
/// `Buff::FingererBoost` entries into per-fingerer modifiers; until then,
/// they pass through as V2 buffs unchanged.
impl From<GameStateV1> for GameStateV2 {
    fn from(v1: GameStateV1) -> Self {
        let fingerers_state = v1
            .fingerers_owned
            .into_iter()
            .map(|(id, count)| {
                (
                    id,
                    FingererStateV2 {
                        count,
                        modifiers: vec![],
                    },
                )
            })
            .collect();
        let buffs = v1
            .buffs
            .into_iter()
            .map(|b| match b {
                BuffV1::ClickFrenzy {
                    ticks_remaining,
                    initial_ticks,
                    mult,
                } => BuffV2::ClickFrenzy {
                    ticks_remaining,
                    initial_ticks,
                    mult,
                },
                BuffV1::FingererBoost {
                    ticks_remaining,
                    initial_ticks,
                    fingerer_id,
                    mult,
                } => BuffV2::FingererBoost {
                    ticks_remaining,
                    initial_ticks,
                    fingerer_id,
                    mult,
                },
            })
            .collect();
        GameStateV2 {
            version: 2,
            cuques: v1.cuques,
            total_clicks: v1.total_clicks,
            lifetime_cuques: v1.lifetime_cuques,
            best_fps: v1.best_fps,
            golden_caught: v1.golden_caught,
            fingerers_state,
            achievements_earned: v1.achievements_earned,
            upgrades_earned: v1.upgrades_earned,
            prestige: v1.prestige,
            total_play_ticks: v1.total_play_ticks,
            buffs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_to_v2_preserves_fingerer_counts() {
        let v1 = GameStateV1 {
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_owned: [("index_finger".into(), 9), ("latex_glove".into(), 4)]
                .into_iter()
                .collect(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
        };

        let v2: GameStateV2 = v1.into();

        assert_eq!(v2.version, 2);
        assert_eq!(v2.fingerers_state.get("index_finger").unwrap().count, 9);
        assert_eq!(v2.fingerers_state.get("latex_glove").unwrap().count, 4);
        assert!(
            v2.fingerers_state
                .values()
                .all(|st| st.modifiers.is_empty())
        );
    }

    #[test]
    fn v1_to_v2_passes_through_click_frenzy() {
        let v1 = GameStateV1 {
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_owned: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![BuffV1::ClickFrenzy {
                ticks_remaining: 100,
                initial_ticks: 260,
                mult: 777.0,
            }],
        };

        let v2: GameStateV2 = v1.into();

        assert_eq!(v2.buffs.len(), 1);
        assert!(matches!(
            v2.buffs[0],
            BuffV2::ClickFrenzy {
                ticks_remaining: 100,
                ..
            }
        ));
    }

    #[test]
    fn v2_into_current_rebuilds_aggregate_from_modifiers() {
        // A V2 save with one Green Coin (+10%) and one Purple Coin (x2) on
        // a fingerer must arrive in live state with the aggregate already
        // populated — the FPS hot path reads it without rebuilding.
        let v2 = GameStateV2 {
            version: 2,
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_state: [(
                "latex_glove".to_string(),
                FingererStateV2 {
                    count: 5,
                    modifiers: vec![
                        ModifierV2 {
                            source: ModifierSourceV2::GreenCoin,
                            effects: vec![ModifierEffectV2::AddPercent(0.10)],
                            duration: ModifierDurationV2::Permanent,
                            created_at_tick: 0,
                        },
                        ModifierV2 {
                            source: ModifierSourceV2::PurpleCoin,
                            effects: vec![ModifierEffectV2::MulFactor(2.0)],
                            duration: ModifierDurationV2::Ticks(600),
                            created_at_tick: 0,
                        },
                    ],
                },
            )]
            .into_iter()
            .collect(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
        };

        let live = v2.into_current();
        let st = live.fingerers_state.get("latex_glove").unwrap();
        assert_eq!(st.count, 5);
        assert_eq!(st.modifiers.len(), 2);
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
        assert!((st.aggregate.mul_factor - 2.0).abs() < 1e-9);
    }
}
