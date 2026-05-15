//! Parallel-Aggregate cache for tree contributions.
//!
//! `bought: HashSet<TreeCoord>` on `UpgradeTreeState` is the source of truth.
//! This `TreeAggregate` is the **derived cache** read by the FPS / click /
//! powerup hot paths. Rebuilt on load and incrementally updated on buy /
//! refund. Reads are O(1) regardless of how many nodes the player owns —
//! the per-tick FPS calc never iterates the bought set.

use std::collections::HashSet;

use crate::game::fingerer::FINGERERS;
use crate::game::powerup::{N_KINDS, PowerupKind};
use crate::game::tree::coord::TreeCoord;
use crate::game::tree::node::{NodeSpec, node_at};
use crate::game::tree::primitive::{Op, Primitive, Target};

/// Per-fingerer contribution from the tree. Mirrors `FingererAggregate`
/// (additive percent sums, mul factor multiplies, flat sums) so the FPS
/// formula combines tree + modifier contributions symmetrically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FingererTreeContrib {
    pub flat_fps: f64,
    pub add_percent: f64,
    pub mul_factor: f64,
    /// Multiplicative on the buy cost of this fingerer (`< 1.0` discount,
    /// `> 1.0` inflation). Used by `GameState::cost`.
    pub cost_mul: f64,
}

impl Default for FingererTreeContrib {
    fn default() -> Self {
        Self {
            flat_fps: 0.0,
            add_percent: 0.0,
            mul_factor: 1.0,
            cost_mul: 1.0,
        }
    }
}

/// All of the tree's contributions, pre-folded into a single struct for
/// O(1) reads on the hot paths.
#[derive(Clone, Debug)]
pub struct TreeAggregate {
    /// Per-fingerer contributions, indexed by `FINGERERS` catalog position.
    pub per_fingerer: Vec<FingererTreeContrib>,
    /// Global "all fingerers" contributions — distribute across every
    /// fingerer's per-tier output. Stack on top of per-fingerer.
    pub all_fingerers_flat: f64,
    pub all_fingerers_add: f64,
    pub all_fingerers_mul: f64,
    /// Click contributions.
    pub click_add: f64,
    pub click_mul: f64,
    pub click_flat: f64,
    /// Prestige multiplier extensions (applied on top of the base
    /// prestige formula).
    pub prestige_add: f64,
    pub prestige_mul: f64,
    /// Per-powerup-kind multipliers, indexed by `PowerupKind as usize`.
    pub powerup_spawn_mul: [f64; N_KINDS],
    pub powerup_reward_mul: [f64; N_KINDS],
    pub powerup_duration_mul: [f64; N_KINDS],
    /// Green Coin AddPercent strength multiplier (the base +10% becomes
    /// +10% * green_coin_strength_mul on catch).
    pub green_coin_strength_mul: f64,
}

impl Default for TreeAggregate {
    fn default() -> Self {
        Self {
            per_fingerer: vec![FingererTreeContrib::default(); FINGERERS.len()],
            all_fingerers_flat: 0.0,
            all_fingerers_add: 0.0,
            all_fingerers_mul: 1.0,
            click_add: 0.0,
            click_mul: 1.0,
            click_flat: 0.0,
            prestige_add: 0.0,
            prestige_mul: 1.0,
            powerup_spawn_mul: [1.0; N_KINDS],
            powerup_reward_mul: [1.0; N_KINDS],
            powerup_duration_mul: [1.0; N_KINDS],
            green_coin_strength_mul: 1.0,
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
        self.all_fingerers_mul = 1.0;
        self.click_add = 0.0;
        self.click_mul = 1.0;
        self.click_flat = 0.0;
        self.prestige_add = 0.0;
        self.prestige_mul = 1.0;
        self.powerup_spawn_mul = [1.0; N_KINDS];
        self.powerup_reward_mul = [1.0; N_KINDS];
        self.powerup_duration_mul = [1.0; N_KINDS];
        self.green_coin_strength_mul = 1.0;
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
            mul_factor: base.mul_factor * self.all_fingerers_mul,
            cost_mul: base.cost_mul,
        }
    }
}

fn fold_primitive(agg: &mut TreeAggregate, p: Primitive, add: bool) {
    let sign = if add { 1.0 } else { -1.0 };
    match (p.op, p.target) {
        // --- Per-fingerer ---
        (Op::AddPercent, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                c.add_percent += sign * p.magnitude;
            }
        }
        (Op::MulFactor, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                if add {
                    c.mul_factor *= p.magnitude;
                } else if p.magnitude != 0.0 {
                    c.mul_factor /= p.magnitude;
                }
            }
        }
        (Op::FlatAdd, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                c.flat_fps += sign * p.magnitude;
            }
        }
        (Op::CostMul, Target::Fingerer(i)) => {
            if let Some(c) = agg.per_fingerer.get_mut(i as usize) {
                if add {
                    c.cost_mul *= p.magnitude;
                } else if p.magnitude != 0.0 {
                    c.cost_mul /= p.magnitude;
                }
            }
        }
        // --- All fingerers ---
        (Op::AddPercent, Target::AllFingerers) => agg.all_fingerers_add += sign * p.magnitude,
        (Op::MulFactor, Target::AllFingerers) => {
            if add {
                agg.all_fingerers_mul *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.all_fingerers_mul /= p.magnitude;
            }
        }
        (Op::FlatAdd, Target::AllFingerers) => agg.all_fingerers_flat += sign * p.magnitude,
        // --- Click ---
        (Op::AddPercent, Target::Click) => agg.click_add += sign * p.magnitude,
        (Op::MulFactor, Target::Click) => {
            if add {
                agg.click_mul *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.click_mul /= p.magnitude;
            }
        }
        (Op::FlatAdd, Target::Click) => agg.click_flat += sign * p.magnitude,
        // --- Prestige ---
        (Op::AddPercent, Target::Prestige) => agg.prestige_add += sign * p.magnitude,
        (Op::MulFactor, Target::Prestige) => {
            if add {
                agg.prestige_mul *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.prestige_mul /= p.magnitude;
            }
        }
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
            let i = k as usize;
            if add {
                agg.powerup_reward_mul[i] *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.powerup_reward_mul[i] /= p.magnitude;
            }
        }
        (Op::EffectMul, Target::PowerupDuration(k)) => {
            let i = k as usize;
            if add {
                agg.powerup_duration_mul[i] *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.powerup_duration_mul[i] /= p.magnitude;
            }
        }
        // --- Green Coin strength ---
        (Op::EffectMul, Target::GreenCoinStrength) => {
            if add {
                agg.green_coin_strength_mul *= p.magnitude;
            } else if p.magnitude != 0.0 {
                agg.green_coin_strength_mul /= p.magnitude;
            }
        }
        // Op/Target combinations the procgen never produces — silently
        // ignore. Belt-and-suspenders: any future generation bug just
        // means the primitive has no effect, not a panic.
        _ => {}
    }
    let _ = (PowerupKind::ALL,);
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
            assert_eq!(c.mul_factor, 1.0);
            assert_eq!(c.cost_mul, 1.0);
        }
        assert_eq!(a.all_fingerers_mul, 1.0);
        assert_eq!(a.click_mul, 1.0);
        assert_eq!(a.prestige_mul, 1.0);
        for v in a.powerup_spawn_mul {
            assert_eq!(v, 1.0);
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
        // Folding in then out should produce a state equal to default
        // within float error.
        assert!((a.per_fingerer[0].add_percent).abs() < 1e-12);
        assert!((a.click_mul - 1.0).abs() < 1e-12);
        assert!((a.green_coin_strength_mul - 1.0).abs() < 1e-12);
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
        assert!((a.click_mul - 6.0).abs() < 1e-12);
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
        assert_eq!(a.click_mul, 1.0);
    }

    #[test]
    fn rebuild_with_only_anchor_is_identity() {
        // The origin (anchor) carries NO primitives — it's the cuque
        // sprite, always-active, with zero gameplay effect. So an
        // aggregate rebuilt with just the anchor owned should equal the
        // identity aggregate. Confirms anchor + non-anchor rebuilds
        // compose correctly.
        let mut bought = HashSet::new();
        bought.insert(TreeCoord::ORIGIN);
        let mut a = TreeAggregate::default();
        a.rebuild_from_bought(&bought);
        for c in &a.per_fingerer {
            assert_eq!(c.add_percent, 0.0);
            assert_eq!(c.flat_fps, 0.0);
            assert!((c.mul_factor - 1.0).abs() < 1e-12);
            assert!((c.cost_mul - 1.0).abs() < 1e-12);
        }
        assert!((a.click_mul - 1.0).abs() < 1e-12);
        assert!((a.all_fingerers_mul - 1.0).abs() < 1e-12);
        assert!((a.prestige_mul - 1.0).abs() < 1e-12);
    }
}
