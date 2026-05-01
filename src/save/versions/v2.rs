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

/// V2 mirror of `Buff`. After Phase 3 of #21 the live `Buff` carries only
/// click-side variants — per-fingerer multipliers live on the modifier
/// system instead. V2's frozen shape matches that: the V1→V2 conversion
/// absorbs `BuffV1::FingererBoost` into per-fingerer modifiers before
/// `BuffV2` ever sees them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum BuffV2 {
    ClickFrenzy {
        ticks_remaining: u32,
        initial_ticks: u32,
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
    /// Lifetime grand total across every powerup variant. Strict rollup;
    /// existing achievements (Golden Touch, Golden Hoarder) gate on this,
    /// so it stays accurate even when migrating from a V1 that only had
    /// the rollup.
    #[serde(default)]
    pub golden_caught: u64,
    /// Per-variant catch counters introduced with V2. V1 saves had no
    /// breakdown to recover, so these zero-init on V1→V2 — the *total*
    /// stays accurate via `golden_caught`; only the *breakdown* is
    /// post-V2.
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
    /// Pity counter for the Green Coin spawn roll. Persisted so the timer
    /// survives quit/restart. Pre-V2 saves default this to 0 — they had
    /// no Green Coin mechanic, so starting fresh is correct.
    #[serde(default)]
    pub goldens_since_green_coin: u32,
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

/// V1 → V2 conversion. Shape changes:
///   - `fingerers_owned: HashMap<String, u32>` →
///     `fingerers_state: HashMap<String, FingererStateV2 { count, modifiers }>`
///   - `BuffV1::FingererBoost` is **absorbed** into per-fingerer modifiers
///     with `source: PurpleCoin`, `effects: [MulFactor(mult)]`, and
///     `duration: Ticks(ticks_remaining)`. The `created_at_tick` field is
///     reconstructed as `total_play_ticks - elapsed`, where elapsed is
///     `initial_ticks - ticks_remaining` (saturating).
///   - `BuffV1::ClickFrenzy` passes through as `BuffV2::ClickFrenzy`.
///   - Adds `version: 2`.
impl From<GameStateV1> for GameStateV2 {
    fn from(v1: GameStateV1) -> Self {
        let mut fingerers_state: HashMap<String, FingererStateV2> = v1
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
        let mut buffs: Vec<BuffV2> = Vec::new();
        for b in v1.buffs {
            match b {
                BuffV1::ClickFrenzy {
                    ticks_remaining,
                    initial_ticks,
                    mult,
                } => buffs.push(BuffV2::ClickFrenzy {
                    ticks_remaining,
                    initial_ticks,
                    mult,
                }),
                BuffV1::FingererBoost {
                    ticks_remaining,
                    initial_ticks,
                    fingerer_id,
                    mult,
                } => {
                    // Reconstruct the buff's start tick from current
                    // play-tick minus elapsed. saturating_sub guards against
                    // weird/inconsistent saves where the elapsed math
                    // doesn't fit (we lose age accuracy in that case but
                    // never crash).
                    let elapsed = initial_ticks.saturating_sub(ticks_remaining) as u64;
                    let created_at_tick = v1.total_play_ticks.saturating_sub(elapsed);
                    let st = fingerers_state.entry(fingerer_id).or_default();
                    st.modifiers.push(ModifierV2 {
                        source: ModifierSourceV2::PurpleCoin,
                        effects: vec![ModifierEffectV2::MulFactor(mult)],
                        duration: ModifierDurationV2::Ticks(ticks_remaining),
                        created_at_tick,
                    });
                }
            }
        }
        GameStateV2 {
            version: 2,
            cuques: v1.cuques,
            total_clicks: v1.total_clicks,
            lifetime_cuques: v1.lifetime_cuques,
            best_fps: v1.best_fps,
            golden_caught: v1.golden_caught,
            // V1 had no per-variant breakdown — the four counters
            // zero-init. The rollup `golden_caught` stays accurate.
            lucky_caught: 0,
            frenzy_caught: 0,
            buff_caught: 0,
            green_coin_caught: 0,
            fingerers_state,
            achievements_earned: v1.achievements_earned,
            upgrades_earned: v1.upgrades_earned,
            prestige: v1.prestige,
            total_play_ticks: v1.total_play_ticks,
            buffs,
            // V1 saves predate the Green Coin mechanic; the pity counter
            // simply starts fresh on first load into V2.
            goldens_since_green_coin: 0,
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
    fn v1_to_v2_absorbs_in_flight_fingerer_boost_into_modifier() {
        // A V1 save mid-Buff-golden has `BuffV1::FingererBoost` running.
        // Phase 3 routes that into the modifier system — Buff golden's live
        // semantics now live there. Remaining time and mult must survive;
        // the modifier's `created_at_tick` reconstructs from
        // `total_play_ticks - elapsed`.
        let v1 = GameStateV1 {
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_owned: [("latex_glove".into(), 4)].into_iter().collect(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 1500,
            buffs: vec![BuffV1::FingererBoost {
                ticks_remaining: 600,
                initial_ticks: 1200,
                fingerer_id: "latex_glove".into(),
                mult: 7.0,
            }],
        };

        let v2: GameStateV2 = v1.into();

        // The V2 buffs Vec no longer carries this — it's been moved.
        assert!(v2.buffs.is_empty());

        let st = v2
            .fingerers_state
            .get("latex_glove")
            .expect("fingerer entry preserved");
        assert_eq!(st.count, 4);
        assert_eq!(st.modifiers.len(), 1);
        let m = &st.modifiers[0];
        assert!(matches!(m.source, ModifierSourceV2::PurpleCoin));
        assert!(matches!(m.duration, ModifierDurationV2::Ticks(600)));
        assert!(matches!(
            m.effects[0],
            ModifierEffectV2::MulFactor(v) if (v - 7.0).abs() < 1e-9
        ));
        // 1500 - (1200 - 600) = 900
        assert_eq!(m.created_at_tick, 900);
    }

    #[test]
    fn v1_to_v2_absorbed_modifier_attaches_to_unowned_fingerer_safely() {
        // Edge case: the V1 save has a FingererBoost on a fingerer that
        // wasn't in `fingerers_owned`. The migration must still preserve
        // the modifier rather than silently dropping it. We create the
        // entry with count=0 — same defensive behavior as live
        // `attach_modifier`.
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
            buffs: vec![BuffV1::FingererBoost {
                ticks_remaining: 100,
                initial_ticks: 100,
                fingerer_id: "hand_of_god".into(),
                mult: 7.0,
            }],
        };

        let v2: GameStateV2 = v1.into();

        let st = v2
            .fingerers_state
            .get("hand_of_god")
            .expect("entry created");
        assert_eq!(st.count, 0);
        assert_eq!(st.modifiers.len(), 1);
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
            lucky_caught: 0,
            frenzy_caught: 0,
            buff_caught: 0,
            green_coin_caught: 0,
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
            goldens_since_green_coin: 0,
        };

        let live = v2.into_current();
        let st = live.fingerers_state.get("latex_glove").unwrap();
        assert_eq!(st.count, 5);
        assert_eq!(st.modifiers.len(), 2);
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
        assert!((st.aggregate.mul_factor - 2.0).abs() < 1e-9);
    }

    #[test]
    fn v1_to_v2_zero_inits_per_variant_counters() {
        // V1 had only the rollup `golden_caught`. V2 adds four per-variant
        // counters; on V1→V2 they zero-init while the rollup is preserved.
        // No data is lost — the *total* stays accurate; only the
        // *breakdown* is post-V2.
        let v1 = GameStateV1 {
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 17,
            fingerers_owned: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: vec![],
        };

        let v2: GameStateV2 = v1.into();

        assert_eq!(v2.golden_caught, 17, "rollup carried forward");
        assert_eq!(v2.lucky_caught, 0);
        assert_eq!(v2.frenzy_caught, 0);
        assert_eq!(v2.buff_caught, 0);
        assert_eq!(v2.green_coin_caught, 0);
    }

    #[test]
    fn v2_into_current_preserves_per_variant_counters() {
        let v2 = GameStateV2 {
            version: 2,
            cuques: 0.0,
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

        let live = v2.into_current();

        assert_eq!(live.golden_caught, 100);
        assert_eq!(live.lucky_caught, 60);
        assert_eq!(live.frenzy_caught, 20);
        assert_eq!(live.buff_caught, 15);
        assert_eq!(live.green_coin_caught, 5);
    }
}
