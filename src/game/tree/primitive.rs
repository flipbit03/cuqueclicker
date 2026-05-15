//! Engine primitives — the fixed vocabulary of effects the tree can produce.
//!
//! Each tree node is a `Vec<Primitive>` where each primitive is a triple of
//! `(Op, Target, magnitude)`. Op + Target are code-defined enums (the engine
//! needs to dispatch on them); the **composition, count, target, magnitude,
//! and sign** of each primitive at a given lot are 100% procgen.
//!
//! Adding a new primitive variant: append to `Op` or `Target`, add a fold
//! case in `aggregate.rs::TreeAggregate::fold_in`, and add a render case in
//! `naming.rs`. There's no other table to update.

use crate::game::fingerer::FINGERERS;
use crate::game::powerup::PowerupKind;

/// What math the primitive performs. Sign of `magnitude` decides boon vs
/// bane: a `MulFactor` with magnitude `< 1.0` is a debuff, `> 1.0` a buff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Add to the additive percent on the target (target's `add_percent`
    /// gains `magnitude`). `0.10` = +10%.
    AddPercent,
    /// Multiply the multiplicative factor on the target. `2.0` = ×2,
    /// `0.5` = ÷2.
    MulFactor,
    /// Add to the flat FPS contribution on the target.
    FlatAdd,
    /// Multiply the COST of buying a fingerer (`< 1.0` is a discount,
    /// `> 1.0` is inflation). Only meaningful when target is a fingerer.
    CostMul,
    /// Scale the inter-arrival cooldown of a powerup kind. `< 1.0` =
    /// spawns more often, `> 1.0` = rarer. Only meaningful when target is
    /// a PowerupSpawn(kind).
    SpawnRateMul,
    /// Scale the reward / duration of a powerup-related effect. Used with
    /// `Target::PowerupReward(kind)` and `Target::PowerupDuration(kind)`.
    EffectMul,
}

/// What the primitive operates on. The full set of game-touchable axes —
/// any effect we want the tree to express has to map to one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A specific fingerer by catalog index. `idx` is `0..FINGERERS.len()`.
    /// Stable: `idx` is the position in the `FINGERERS` array. The catalog
    /// id at that index is the load-bearing string; `idx` is just a
    /// compact encoding for the procgen-derived target.
    Fingerer(u8),
    /// All fingerers — the effect distributes across every fingerer's
    /// per-tier output.
    AllFingerers,
    /// Manual click power (`click_power()`).
    Click,
    /// Powerup spawn rate for the given kind (used with `Op::SpawnRateMul`).
    PowerupSpawn(PowerupKind),
    /// Powerup reward for the given kind (used with `Op::EffectMul`).
    /// Currently meaningful for Lucky (flat cuques) and Buff (mul factor).
    PowerupReward(PowerupKind),
    /// Powerup buff duration for the given kind (used with `Op::EffectMul`).
    /// Currently meaningful for Frenzy and Buff.
    PowerupDuration(PowerupKind),
    /// Prestige multiplier (current formula is 1 + 0.01 * prestige_count).
    Prestige,
    /// Green Coin AddPercent strength (the +10% becomes +10% * magnitude).
    GreenCoinStrength,
}

/// Number of categorical targets the procgen can roll from (fingerers + globals).
/// FINGERERS.len() varies via the catalog; this is just a generation hint.
pub const N_PRIMITIVE_TARGETS: usize = 24;

impl Target {
    /// Stable index used by the procgen to pick a target uniformly from the
    /// available axes. Distinct values are returned for distinct targets.
    pub fn from_index(idx: usize) -> Self {
        let nf = FINGERERS.len();
        // Layout:
        //   [0..nf)            -> Fingerer(i)
        //   [nf]               -> AllFingerers
        //   [nf+1]             -> Click
        //   [nf+2 ..nf+6)      -> PowerupSpawn(L/F/B/G)
        //   [nf+6 ..nf+10)     -> PowerupReward(L/F/B/G)
        //   [nf+10..nf+14)     -> PowerupDuration(L/F/B/G)
        //   [nf+14]            -> Prestige
        //   [nf+15]            -> GreenCoinStrength
        let kinds = PowerupKind::ALL;
        if idx < nf {
            return Target::Fingerer(idx as u8);
        }
        let i = idx - nf;
        if i == 0 {
            return Target::AllFingerers;
        }
        if i == 1 {
            return Target::Click;
        }
        if (2..6).contains(&i) {
            return Target::PowerupSpawn(kinds[i - 2]);
        }
        if (6..10).contains(&i) {
            return Target::PowerupReward(kinds[i - 6]);
        }
        if (10..14).contains(&i) {
            return Target::PowerupDuration(kinds[i - 10]);
        }
        if i == 14 {
            return Target::Prestige;
        }
        // Last bucket, anything beyond falls into GreenCoinStrength.
        Target::GreenCoinStrength
    }

    /// Total number of distinct targets the procgen can produce, given the
    /// current FINGERERS catalog length. Procgen uses
    /// `rng.range_usize(target_count())` to pick.
    pub fn target_count() -> usize {
        FINGERERS.len() + 16
    }
}

/// A single tree-node effect contribution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primitive {
    pub op: Op,
    pub target: Target,
    pub magnitude: f64,
}

impl Primitive {
    /// True when the primitive's sign (relative to its op's "neutral" value)
    /// represents a debuff/bane. Used to classify nodes as keystones.
    pub fn is_bane(self) -> bool {
        match self.op {
            Op::AddPercent | Op::FlatAdd => self.magnitude < 0.0,
            // For multiplicative ops a magnitude < 1.0 is a nerf (incl 0).
            Op::MulFactor | Op::EffectMul | Op::SpawnRateMul => self.magnitude < 1.0,
            // CostMul: < 1.0 is a *boon* (cheaper); > 1.0 is a bane.
            Op::CostMul => self.magnitude > 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_from_index_covers_full_range() {
        let n = Target::target_count();
        let mut seen_fingerer = 0;
        let mut seen_all = 0;
        let mut seen_click = 0;
        let mut seen_prestige = 0;
        for i in 0..n {
            match Target::from_index(i) {
                Target::Fingerer(_) => seen_fingerer += 1,
                Target::AllFingerers => seen_all += 1,
                Target::Click => seen_click += 1,
                Target::Prestige => seen_prestige += 1,
                _ => {}
            }
        }
        assert_eq!(seen_fingerer, FINGERERS.len());
        assert_eq!(seen_all, 1);
        assert_eq!(seen_click, 1);
        assert_eq!(seen_prestige, 1);
    }

    #[test]
    fn is_bane_sign_semantics() {
        // AddPercent: negative is bane.
        assert!(
            !Primitive {
                op: Op::AddPercent,
                target: Target::Click,
                magnitude: 0.10
            }
            .is_bane()
        );
        assert!(
            Primitive {
                op: Op::AddPercent,
                target: Target::Click,
                magnitude: -0.05
            }
            .is_bane()
        );

        // MulFactor: <1 is bane.
        assert!(
            !Primitive {
                op: Op::MulFactor,
                target: Target::Click,
                magnitude: 2.0
            }
            .is_bane()
        );
        assert!(
            Primitive {
                op: Op::MulFactor,
                target: Target::Click,
                magnitude: 0.5
            }
            .is_bane()
        );

        // CostMul: >1 is bane (more expensive).
        assert!(
            !Primitive {
                op: Op::CostMul,
                target: Target::Fingerer(0),
                magnitude: 0.8
            }
            .is_bane()
        );
        assert!(
            Primitive {
                op: Op::CostMul,
                target: Target::Fingerer(0),
                magnitude: 1.5
            }
            .is_bane()
        );

        // SpawnRateMul: <1 is bane (slower spawns); >1 is boon (faster).
        assert!(
            !Primitive {
                op: Op::SpawnRateMul,
                target: Target::PowerupSpawn(PowerupKind::Lucky),
                magnitude: 1.10
            }
            .is_bane()
        );
        assert!(
            Primitive {
                op: Op::SpawnRateMul,
                target: Target::PowerupSpawn(PowerupKind::Lucky),
                magnitude: 0.91
            }
            .is_bane()
        );
    }
}
