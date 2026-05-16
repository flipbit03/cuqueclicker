//! Per-fingerer modifier system.
//!
//! A [`Modifier`] is a composable buff or debuff attached to a single
//! fingerer's [`crate::game::state::FingererState`]. Each modifier carries
//! a stable [`ModifierSource`] id, zero or more [`ModifierEffect`]s, and
//! an optional duration in ticks. Modifiers stack freely — multiple of
//! the same source on the same fingerer is fine and additive.
//!
//! ## Stacking semantics
//!
//! - [`ModifierEffect::FlatFps`] values across modifiers **sum**, applied
//!   before any percent.
//! - [`ModifierEffect::AddPercent`] values across modifiers **sum**
//!   (two +10% Green Coins = +20%, not +21%).
//! - [`ModifierEffect::MulFactor`] values across modifiers **multiply**
//!   (two x2 buffs = x4).
//!
//! Final fingerer output:
//!
//! ```text
//! ((base * count + flat_fps) * (1 + add_percent) * mul_factor) * upgrades_mult
//! ```
//!
//! ## Aggregate cache
//!
//! Hot-path reads (FPS calc, sidebar render) MUST go through
//! [`FingererAggregate`], never iterate the `Vec<Modifier>` directly. The
//! aggregate is rebuilt in three situations only:
//!   1. A modifier is added or removed via the public API.
//!   2. The per-tick walk drops an expired [`ModifierDuration::Ticks`]
//!      entry whose count just hit zero.
//!   3. The save loader reconstructs it (the field is `#[serde(skip)]`).
//!
//! ## Adding a new buff/debuff source
//!
//! Add a variant to [`ModifierSource`] and map it in [`ModifierSource::id`].
//! No tick-loop changes needed — the existing per-tick walk already
//! decrements timed modifiers and rebuilds aggregates on expiry. The id
//! string is load-bearing forever; treat it like a fingerer or upgrade id.

use serde::{Deserialize, Serialize};

use crate::bignum::Mag;

/// Stable identifier for the *kind* of buff or debuff a modifier represents.
/// Used for de-duping in the UI ("3× Green Coin" instead of three identical
/// chips), for save serialization, and for future per-source rules.
///
/// **The string returned by [`Self::id`] is a load-bearing primary key** —
/// renaming it silently invalidates every player's saved progress on that
/// source. Treat it like a fingerer or upgrade id: cosmetic names live in
/// `i18n.rs`; the id stays stable forever.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ModifierSource {
    /// Green Coin — rare powerup that attaches a permanent +10% AddPercent
    /// to a random owned fingerer.
    GreenCoin,
    /// Purple Coin — the existing Buff Golden, recast as a per-fingerer
    /// modifier with a finite duration. Phase 3 of the Green Coin PR
    /// absorbs `Buff::FingererBoost` into this source.
    PurpleCoin,
}

impl ModifierSource {
    pub fn id(self) -> &'static str {
        match self {
            Self::GreenCoin => "green_coin",
            Self::PurpleCoin => "purple_coin",
        }
    }
}

/// One contribution from a modifier. A single [`Modifier`] may carry
/// multiple effects — e.g. a future debuff could combine
/// `[FlatFps(-10.0), MulFactor(0.5)]`.
///
/// `MulFactor` carries a [`Mag`] (log-magnitude) so the catch-time
/// product `7.0 × tree.powerup_reward_mul[Buff]` can grow truly
/// unboundedly across late-game stacks without overflowing `f64`. The
/// `Mag` (de)serializer accepts both raw JSON numbers (legacy V4 saves
/// stored `"MulFactor": 7.0`) and the explicit `{"log10": …}` form (new
/// huge values), so the change is wire-compatible — no save version
/// bump.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum ModifierEffect {
    /// Flat additive contribution to the fingerer's per-tier output.
    /// Pre-multiplier: added to `base * count` before percent or mul.
    FlatFps(f64),
    /// Additive percent. `0.10` = +10%. Sums across all modifiers on the
    /// fingerer (two +10% Green Coins = +20%).
    AddPercent(f64),
    /// Multiplicative factor. Multiplies across all modifiers on the
    /// fingerer (two x2 = x4). Applied after [`Self::AddPercent`].
    MulFactor(Mag),
}

/// Lifetime of a modifier. Permanent modifiers stick for the rest of the
/// run (cleared only by `prestige_reset`). Timed modifiers decrement once
/// per `state.tick()` and drop when they hit zero.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModifierDuration {
    Permanent,
    /// Remaining ticks. Decremented in the per-tick modifier walk; the
    /// modifier is removed (and the aggregate rebuilt) on the tick it
    /// would step from `Ticks(0)`.
    Ticks(u32),
}

/// A single buff or debuff attached to a fingerer. Composable: a fingerer
/// may carry an unbounded number of these. See module docs for the
/// stacking rules and aggregate-cache contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Modifier {
    pub source: ModifierSource,
    /// Zero or more contributions. An empty Vec is valid (a sourced marker
    /// with no numeric effect — e.g. a flag-style modifier read by future
    /// UI without changing FPS).
    pub effects: Vec<ModifierEffect>,
    pub duration: ModifierDuration,
    /// `total_play_ticks` value at the time this modifier was attached.
    /// Drives "X ago" labels and tie-breaking. Recorded against the
    /// monotonic play counter (not wall-clock) so it survives quit/restart
    /// without becoming wrong.
    #[serde(default)]
    pub created_at_tick: u64,
}

impl Modifier {
    /// Plateau-at-1.0 until the last `FADE_TICKS` of the duration, then
    /// smoothstep-decay to 0. Mirrors `Buff::strength` so border / HUD
    /// pulse code can blend timed modifiers into the same activity sum
    /// without a special case. Permanent modifiers always read 1.0
    /// (they don't fade).
    pub fn strength(&self) -> f32 {
        const FADE_TICKS: f32 = 30.0; // ~1.5s at 20Hz
        match self.duration {
            ModifierDuration::Permanent => 1.0,
            ModifierDuration::Ticks(n) => {
                let remaining = n as f32;
                if remaining >= FADE_TICKS {
                    1.0
                } else {
                    let t = (remaining / FADE_TICKS).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                }
            }
        }
    }
}

/// Pre-computed sum/product of every effect across every modifier on a
/// fingerer. Read on every FPS calc — the tick path rebuilds this when
/// modifiers are added, removed, or expire, so reads are O(1).
///
/// `Default` is the **identity**: zero flat, zero add-percent, x1 multiplier.
/// This is the value reads see when no modifiers are attached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FingererAggregate {
    pub flat_fps: f64,
    pub add_percent: f64,
    pub mul_factor: Mag,
}

impl Default for FingererAggregate {
    fn default() -> Self {
        Self {
            flat_fps: 0.0,
            add_percent: 0.0,
            mul_factor: Mag::ONE,
        }
    }
}

impl FingererAggregate {
    /// Walk every effect on every modifier and fold them into a single
    /// aggregate. Linear in (modifiers × effects); call on add/remove/expire.
    pub fn rebuild(modifiers: &[Modifier]) -> Self {
        let mut a = Self::default();
        for m in modifiers {
            for e in &m.effects {
                match *e {
                    ModifierEffect::FlatFps(v) => a.flat_fps += v,
                    ModifierEffect::AddPercent(v) => a.add_percent += v,
                    ModifierEffect::MulFactor(v) => a.mul_factor = a.mul_factor.mul(v),
                }
            }
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm(effects: Vec<ModifierEffect>) -> Modifier {
        Modifier {
            source: ModifierSource::GreenCoin,
            effects,
            duration: ModifierDuration::Permanent,
            created_at_tick: 0,
        }
    }

    fn mul(v: f64) -> ModifierEffect {
        ModifierEffect::MulFactor(Mag::from_f64(v))
    }

    #[test]
    fn empty_aggregate_is_identity() {
        let a = FingererAggregate::rebuild(&[]);
        assert_eq!(a.flat_fps, 0.0);
        assert_eq!(a.add_percent, 0.0);
        assert_eq!(a.mul_factor, Mag::ONE);
    }

    #[test]
    fn add_percent_sums_across_modifiers() {
        let mods = vec![
            perm(vec![ModifierEffect::AddPercent(0.10)]),
            perm(vec![ModifierEffect::AddPercent(0.10)]),
        ];
        let a = FingererAggregate::rebuild(&mods);
        assert!((a.add_percent - 0.20).abs() < 1e-9);
    }

    #[test]
    fn mul_factor_multiplies_across_modifiers() {
        let mods = vec![perm(vec![mul(2.0)]), perm(vec![mul(2.0)])];
        let a = FingererAggregate::rebuild(&mods);
        assert!((a.mul_factor.to_f64() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn flat_fps_sums_across_modifiers() {
        let mods = vec![
            perm(vec![ModifierEffect::FlatFps(50.0)]),
            perm(vec![ModifierEffect::FlatFps(75.0)]),
        ];
        let a = FingererAggregate::rebuild(&mods);
        assert!((a.flat_fps - 125.0).abs() < 1e-9);
    }

    #[test]
    fn mixed_effects_on_same_modifier_all_apply() {
        let mods = vec![perm(vec![
            ModifierEffect::FlatFps(50.0),
            ModifierEffect::AddPercent(0.10),
            mul(2.0),
        ])];
        let a = FingererAggregate::rebuild(&mods);
        assert!((a.flat_fps - 50.0).abs() < 1e-9);
        assert!((a.add_percent - 0.10).abs() < 1e-9);
        assert!((a.mul_factor.to_f64() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn mul_factor_stack_survives_billions() {
        // Stack 10k MulFactor(1.5) modifiers — under the old f64 path
        // this overflows to Infinity around the 1740th. With Mag it
        // produces a precise log10 ≈ 1761 (= 10000 * log10(1.5)).
        let mods: Vec<_> = (0..10_000).map(|_| perm(vec![mul(1.5)])).collect();
        let a = FingererAggregate::rebuild(&mods);
        let expected = (1.5_f64).log10() * 10_000.0;
        assert!((a.mul_factor.log10 - expected).abs() < 1e-6);
    }

    #[test]
    fn source_ids_are_stable() {
        // These strings are load-bearing — they appear in serialized saves.
        // Renaming would orphan player progress.
        assert_eq!(ModifierSource::GreenCoin.id(), "green_coin");
        assert_eq!(ModifierSource::PurpleCoin.id(), "purple_coin");
    }
}
