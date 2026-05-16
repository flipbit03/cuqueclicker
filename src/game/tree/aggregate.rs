//! Parallel-Aggregate cache for tree contributions.
//!
//! `bought: HashSet<TreeCoord>` on `UpgradeTreeState` is the source of truth.
//! This `TreeAggregate` is the **derived cache** read by the FPS / click /
//! powerup hot paths. Rebuilt on load and incrementally updated on buy /
//! refund. Reads are O(1) regardless of how many nodes the player owns —
//! the per-tick FPS calc never iterates the bought set.

use std::collections::HashSet;

use crate::bignum::Mag;
use crate::game::fingerer::FINGERERS;
use crate::game::powerup::N_KINDS;
use crate::game::tree::coord::TreeCoord;
use crate::game::tree::node::{NodeSpec, node_at};
use crate::game::tree::primitive::{Op, Primitive, Target};

/// Per-fingerer contribution from the tree. Mirrors `FingererAggregate`
/// (additive percent sums, mul factor multiplies, flat sums) so the FPS
/// formula combines tree + modifier contributions symmetrically.
///
/// Multiplicative fields live in `Mag` (log-magnitude) because they
/// compound across hundreds-to-thousands of bought nodes; the original
/// `f64` storage produced `Infinity` in long-haul play. Additive fields
/// (`flat_fps`, `add_percent`) stay `f64` — they don't compound and
/// per-node values stay tiny.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FingererTreeContrib {
    pub flat_fps: f64,
    pub add_percent: f64,
    pub mul_factor: Mag,
    /// Multiplicative on the buy cost of this fingerer (`< 1.0` discount,
    /// `> 1.0` inflation). Used by `GameState::cost`.
    pub cost_mul: Mag,
}

impl Default for FingererTreeContrib {
    fn default() -> Self {
        Self {
            flat_fps: 0.0,
            add_percent: 0.0,
            mul_factor: Mag::ONE,
            cost_mul: Mag::ONE,
        }
    }
}

/// All of the tree's contributions, pre-folded into a single struct for
/// O(1) reads on the hot paths.
///
/// All multiplicative fields are `Mag` so multi-thousand-node stacks
/// can't overflow. The few small-magnitude additive fields stay `f64`
/// because they don't compound. `powerup_spawn_mul` also stays `f64`
/// because its consumer (`sim::next_cooldown`) computes
/// `base / mul_f64` directly and we'd just round-trip through `f64`
/// anyway.
#[derive(Clone, Debug)]
pub struct TreeAggregate {
    /// Per-fingerer contributions, indexed by `FINGERERS` catalog position.
    pub per_fingerer: Vec<FingererTreeContrib>,
    /// Global "all fingerers" contributions — distribute across every
    /// fingerer's per-tier output. Stack on top of per-fingerer.
    pub all_fingerers_flat: f64,
    pub all_fingerers_add: f64,
    pub all_fingerers_mul: Mag,
    /// Click contributions.
    pub click_add: f64,
    pub click_mul: Mag,
    pub click_flat: f64,
    /// Prestige multiplier extensions (applied on top of the base
    /// prestige formula).
    pub prestige_add: f64,
    pub prestige_mul: Mag,
    /// Per-powerup-kind spawn-rate multiplier. `f64` because the only
    /// consumer (`sim::next_cooldown`) needs the raw value as a divisor
    /// and the per-node cap (×1.005) keeps any realistic stack inside
    /// `f64` range.
    pub powerup_spawn_mul: [f64; N_KINDS],
    /// Per-powerup-kind reward multiplier. `Mag` because this stage was
    /// the headline overflow path: `powerup_reward_mul[Buff]` scales the
    /// 7× MulFactor that gets persisted on a fingerer every Buff catch,
    /// and 10k+ deep tree stacks would otherwise blow past `f64`.
    pub powerup_reward_mul: [Mag; N_KINDS],
    pub powerup_duration_mul: [Mag; N_KINDS],
    /// Green Coin AddPercent strength multiplier (the base +10% becomes
    /// +10% * green_coin_strength_mul on catch).
    pub green_coin_strength_mul: Mag,
}

impl Default for TreeAggregate {
    fn default() -> Self {
        Self {
            per_fingerer: vec![FingererTreeContrib::default(); FINGERERS.len()],
            all_fingerers_flat: 0.0,
            all_fingerers_add: 0.0,
            all_fingerers_mul: Mag::ONE,
            click_add: 0.0,
            click_mul: Mag::ONE,
            click_flat: 0.0,
            prestige_add: 0.0,
            prestige_mul: Mag::ONE,
            powerup_spawn_mul: [1.0; N_KINDS],
            powerup_reward_mul: [Mag::ONE; N_KINDS],
            powerup_duration_mul: [Mag::ONE; N_KINDS],
            green_coin_strength_mul: Mag::ONE,
        }
    }
}

impl TreeAggregate {
    /// Resize `per_fingerer` to match the live `FINGERERS` length and
    /// reset every field to identity. Cheap; do this in `migrate_runtime`
    /// before walking `bought`.
    pub fn reset(&mut self) {
        if self.per_fingerer.len() != FINGERERS.len() {
            self.per_fingerer = vec![FingererTreeContrib::default(); FINGERERS.len()];
        } else {
            for c in self.per_fingerer.iter_mut() {
                *c = FingererTreeContrib::default();
            }
        }
        self.all_fingerers_flat = 0.0;
        self.all_fingerers_add = 0.0;
        self.all_fingerers_mul = Mag::ONE;
        self.click_add = 0.0;
        self.click_mul = Mag::ONE;
        self.click_flat = 0.0;
        self.prestige_add = 0.0;
        self.prestige_mul = Mag::ONE;
        self.powerup_spawn_mul = [1.0; N_KINDS];
        self.powerup_reward_mul = [Mag::ONE; N_KINDS];
        self.powerup_duration_mul = [Mag::ONE; N_KINDS];
        self.green_coin_strength_mul = Mag::ONE;
    }

    /// Rebuild the aggregate from scratch by regenerating each owned node
    /// from its lot coord and folding its primitives in. Called by
    /// `migrate_runtime` on load and by `prestige_reset` after clearing
    /// `bought`.
    pub fn rebuild_from_bought(&mut self, bought: &HashSet<TreeCoord>) {
        self.reset();
        for &lot in bought {
            if let Some(node) = node_at(lot.x, lot.y) {
                for &p in &node.primitives {
                    fold_primitive(self, p, true);
                }
            }
        }
    }

    /// Fold a single node's primitive stack into the aggregate. Called
    /// when the player buys a node — incremental update, O(primitives in
    /// node) ≈ 1-4.
    pub fn fold_in_node(&mut self, node: &NodeSpec) {
        for &p in &node.primitives {
            fold_primitive(self, p, true);
        }
    }

    /// Inverse of `fold_in_node`: subtract a node's contribution. Called
    /// on refund.
    pub fn fold_out_node(&mut self, node: &NodeSpec) {
        for &p in &node.primitives {
            fold_primitive(self, p, false);
        }
    }

    /// Convenience: get the per-fingerer contrib for a catalog index,
    /// folded with the global `all_fingerers_*` contributions.
    pub fn effective_for_fingerer(&self, idx: usize) -> FingererTreeContrib {
        let base = self.per_fingerer.get(idx).copied().unwrap_or_default();
        FingererTreeContrib {
            flat_fps: base.flat_fps + self.all_fingerers_flat,
            add_percent: base.add_percent + self.all_fingerers_add,
            mul_factor: base.mul_factor.mul(self.all_fingerers_mul),
            cost_mul: base.cost_mul,
        }
    }
}

/// Apply (`add == true`) or undo (`add == false`) a single primitive's
/// contribution. Multiplicative primitives compose via `Mag` so the
/// running product stays representable no matter how many nodes the
/// player buys.
fn fold_primitive(agg: &mut TreeAggregate, p: Primitive, add: bool) {
    let sign = if add { 1.0 } else { -1.0 };
    let mag = Mag::from_f64(p.magnitude);
    let apply_mul = |slot: &mut Mag, m: Mag| {
        if add {
            *slot = slot.mul(m);
        } else if !m.is_zero() {
            *slot = slot.div(m);
        }
    };
    match (p.op, p.target) {
        // --- Per-fingerer ---
        (Op::AddPercent, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                c.add_percent += sign * p.magnitude;
            }
        }
        (Op::MulFactor, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                apply_mul(&mut c.mul_factor, mag);
            }
        }
        (Op::FlatAdd, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                c.flat_fps += sign * p.magnitude;
            }
        }
        (Op::CostMul, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                apply_mul(&mut c.cost_mul, mag);
            }
        }
        // --- All fingerers ---
        (Op::AddPercent, Target::AllFingerers) => agg.all_fingerers_add += sign * p.magnitude,
        (Op::MulFactor, Target::AllFingerers) => apply_mul(&mut agg.all_fingerers_mul, mag),
        (Op::FlatAdd, Target::AllFingerers) => agg.all_fingerers_flat += sign * p.magnitude,
        // --- Click ---
        (Op::AddPercent, Target::Click) => agg.click_add += sign * p.magnitude,
        (Op::MulFactor, Target::Click) => apply_mul(&mut agg.click_mul, mag),
        (Op::FlatAdd, Target::Click) => agg.click_flat += sign * p.magnitude,
        // --- Prestige ---
        (Op::AddPercent, Target::Prestige) => agg.prestige_add += sign * p.magnitude,
        (Op::MulFactor, Target::Prestige) => apply_mul(&mut agg.prestige_mul, mag),
        // --- Powerup spawn / reward / duration ---
        (Op::SpawnRateMul, Target::PowerupSpawn(k)) => {
            let i = k as usize;
            if add {
                agg.powerup_spawn_mul[i] *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.powerup_spawn_mul[i] /= p.magnitude;
            }
        }
        (Op::EffectMul, Target::PowerupReward(k)) => {
            apply_mul(&mut agg.powerup_reward_mul[k as usize], mag);
        }
        (Op::EffectMul, Target::PowerupDuration(k)) => {
            apply_mul(&mut agg.powerup_duration_mul[k as usize], mag);
        }
        // --- Green Coin strength ---
        (Op::EffectMul, Target::GreenCoinStrength) => {
            apply_mul(&mut agg.green_coin_strength_mul, mag);
        }
        // Op/Target combinations the procgen never produces — fail loud
        // in dev so a future generation bug or new enum variant can't
        // silently charge the player cuques for an effect that doesn't
        // fold into the aggregate. Release builds still no-op (the
        // primitive has no effect) instead of panicking the run.
        (op, target) => {
            debug_assert!(
                false,
                "unhandled tree primitive: op={op:?} target={target:?} — \
                 add a fold arm in aggregate.rs::fold_primitive or remove \
                 this (op, target) pairing from procgen pick_op"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(op: Op, target: Target, mag: f64) -> Primitive {
        Primitive {
            op,
            target,
            magnitude: mag,
        }
    }

    #[test]
    fn default_is_identity() {
        let a = TreeAggregate::default();
        for c in &a.per_fingerer {
            assert_eq!(c.flat_fps, 0.0);
            assert_eq!(c.add_percent, 0.0);
            assert_eq!(c.mul_factor, Mag::ONE);
            assert_eq!(c.cost_mul, Mag::ONE);
        }
        assert_eq!(a.all_fingerers_mul, Mag::ONE);
        assert_eq!(a.click_mul, Mag::ONE);
        assert_eq!(a.prestige_mul, Mag::ONE);
        for v in a.powerup_spawn_mul {
            assert_eq!(v, 1.0);
        }
        for v in a.powerup_reward_mul {
            assert_eq!(v, Mag::ONE);
        }
    }

    #[test]
    fn fold_in_then_out_returns_to_default() {
        let mut a = TreeAggregate::default();
        let prims = vec![
            p(Op::AddPercent, Target::Fingerer(0), 0.10),
            p(Op::MulFactor, Target::Click, 2.0),
            p(Op::EffectMul, Target::GreenCoinStrength, 1.5),
        ];
        for &pp in &prims {
            fold_primitive(&mut a, pp, true);
        }
        for &pp in &prims {
            fold_primitive(&mut a, pp, false);
        }
        assert!((a.per_fingerer[0].add_percent).abs() < 1e-12);
        assert!((a.click_mul.to_f64() - 1.0).abs() < 1e-12);
        assert!((a.green_coin_strength_mul.to_f64() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn add_percent_stacks_additively() {
        let mut a = TreeAggregate::default();
        fold_primitive(&mut a, p(Op::AddPercent, Target::Fingerer(0), 0.10), true);
        fold_primitive(&mut a, p(Op::AddPercent, Target::Fingerer(0), 0.15), true);
        assert!((a.per_fingerer[0].add_percent - 0.25).abs() < 1e-12);
    }

    #[test]
    fn mul_factor_stacks_multiplicatively() {
        let mut a = TreeAggregate::default();
        fold_primitive(&mut a, p(Op::MulFactor, Target::Click, 2.0), true);
        fold_primitive(&mut a, p(Op::MulFactor, Target::Click, 3.0), true);
        assert!((a.click_mul.to_f64() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn mul_factor_stack_is_truly_unbounded() {
        // The whole point of switching to `Mag`: a stack of 100k nodes
        // each contributing ×1.005 used to overflow `f64` to `Infinity`.
        // Now it stays representable and the log10 is exact.
        let mut a = TreeAggregate::default();
        for _ in 0..100_000 {
            fold_primitive(&mut a, p(Op::MulFactor, Target::Click, 1.005), true);
        }
        let expected_log10 = (1.005_f64).log10() * 100_000.0;
        assert!((a.click_mul.log10 - expected_log10).abs() < 1e-6);
    }

    #[test]
    fn effective_for_fingerer_folds_global() {
        let mut a = TreeAggregate::default();
        fold_primitive(&mut a, p(Op::AddPercent, Target::Fingerer(0), 0.10), true);
        fold_primitive(&mut a, p(Op::AddPercent, Target::AllFingerers, 0.05), true);
        let eff = a.effective_for_fingerer(0);
        assert!((eff.add_percent - 0.15).abs() < 1e-12);
    }

    #[test]
    fn rebuild_from_empty_bought_is_identity() {
        let mut a = TreeAggregate::default();
        // Pre-pollute then rebuild from empty.
        fold_primitive(&mut a, p(Op::MulFactor, Target::Click, 5.0), true);
        a.rebuild_from_bought(&HashSet::new());
        assert_eq!(a.click_mul, Mag::ONE);
    }

    #[test]
    fn rebuild_with_only_anchor_is_identity() {
        let mut bought = HashSet::new();
        bought.insert(TreeCoord::ORIGIN);
        let mut a = TreeAggregate::default();
        a.rebuild_from_bought(&bought);
        for c in &a.per_fingerer {
            assert_eq!(c.add_percent, 0.0);
            assert_eq!(c.flat_fps, 0.0);
            assert_eq!(c.mul_factor, Mag::ONE);
            assert_eq!(c.cost_mul, Mag::ONE);
        }
        assert_eq!(a.click_mul, Mag::ONE);
        assert_eq!(a.all_fingerers_mul, Mag::ONE);
        assert_eq!(a.prestige_mul, Mag::ONE);
    }
}
