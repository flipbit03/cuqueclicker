//! Per-lot node generator.
//!
//! `node_at(x, y)` returns either `Some(NodeSpec)` (this lot carries a node)
//! or `None` (empty lot — procedural gap). The result is a pure function of
//! `(TREE_SEED, x, y)`: the same input always produces byte-identical output.
//!
//! Lot geometry: the canvas is an infinite grid of lots, each
//! `LOT_W × LOT_H` cells. A lot's node (if any) is a procedurally-sized
//! rectangular box placed at a procedurally-jittered offset inside the lot.
//! Boxes never overlap because each lives strictly inside its lot interior.
//!
//! Edges: any two lots whose `(x, y)` differ by at most 1 on each axis
//! ("8-king" adjacency) may be connected. Existence is rolled with
//! `edge_exists(a, b)` from a position-seeded RNG, so the connectivity
//! pattern is also stable forever.

use std::cell::RefCell;
use std::collections::HashMap;

use super::coord::TreeCoord;
use super::naming::make_title;
use super::noise::{fbm_2d, value_noise_2d};
use super::primitive::{Op, Primitive, Target};
use super::rng::SplitMix64;
use super::seed::TREE_SEED;

/// Size of one "lot" cell in the infinite tree grid, in terminal cells.
/// 36 wide × 10 tall comfortably holds a Notable box (16×5) with breathing
/// room for edge routing.
pub const LOT_W: i32 = 36;
pub const LOT_H: i32 = 10;

/// Probability that a given lot is *empty* (no node). Creates organic gaps
/// in the procedural tree, mimicking PoE's sparse regions. Bumped from
/// 0.20 to 0.35 after the orthogonal-edge-always-connects rule: with
/// every adjacent pair guaranteed to be linked, a lower gap rate made
/// the tree feel like a dense lattice instead of a branching graph.
/// More gaps + always-connected adjacency reads as a sparser, more
/// tree-like topology.
const GAP_PROBABILITY: f64 = 0.35;

/// Origin lot bias: lots near (0, 0) are more likely to be populated AND
/// more likely to carry small/notable nodes (vs. keystones), so the player's
/// starting neighborhood is dense and approachable. Within `BIAS_RADIUS`
/// lots of origin, gappiness is reduced and rarity is capped.
const BIAS_RADIUS: i32 = 4;

/// Visual classification, computed from the rolled primitive stack. Drives
/// box dimensions, edge weight, and biome tinting in the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rarity {
    Small,
    Notable,
    Keystone,
}

impl Rarity {
    pub fn box_dims(self) -> (u16, u16) {
        match self {
            Rarity::Small => (14, 3),
            Rarity::Notable => (18, 5),
            Rarity::Keystone => (24, 7),
        }
    }
}

/// A single tree node, regenerated from `(TREE_SEED, lot)` on demand.
#[derive(Clone, Debug)]
pub struct NodeSpec {
    pub lot: TreeCoord,
    /// Absolute position+size of the rendered box, in terminal cells, in
    /// "canvas" coordinates. The renderer translates these into screen
    /// coords by subtracting the pan offset.
    pub box_x: i32,
    pub box_y: i32,
    pub box_w: u16,
    pub box_h: u16,
    pub rarity: Rarity,
    pub primitives: Vec<Primitive>,
    pub cost: crate::bignum::Mag,
    pub title: String,
    /// Dominant target hint — used by the renderer for biome-color tinting.
    /// Picked as the target of the first primitive in the rolled stack.
    pub dominant_target: Target,
    /// `true` only for the (0, 0) lot — the "anchor" cuque-sprite node.
    /// Anchors carry no primitives, no cost, are auto-owned at startup,
    /// and the renderer paints `BISCUIT_TINY` instead of a regular box.
    /// Every other node in the tree branches outward from this anchor.
    pub is_anchor: bool,
}

/// Salt constants — keep all of generation's `SplitMix64::from_coords` calls
/// distinct so they don't share an RNG stream.
const SALT_NODE: u64 = 0xA1;
const SALT_EDGE: u64 = 0xED;
const SALT_GAP: u64 = 0x6A;

/// Pre-suppression predicate: does the local gap roll for this lot say
/// "populated"? Origin neighborhood (|x|<=1 AND |y|<=1) is forced
/// populated regardless of the roll. This is the *raw* "could this lot
/// hold a node" check — orphan suppression on top of it lives in
/// `reaches_origin`, which is what `node_at` actually gates on.
fn passes_gap_roll(x: i32, y: i32) -> bool {
    let is_origin_neighborhood = x.abs() <= 1 && y.abs() <= 1;
    if is_origin_neighborhood {
        return true;
    }
    let mut gap_rng = SplitMix64::from_coords(TREE_SEED, x, y, SALT_GAP);
    let near_origin = x.abs() <= BIAS_RADIUS && y.abs() <= BIAS_RADIUS;
    let gap_p = if near_origin {
        GAP_PROBABILITY * 0.5
    } else {
        GAP_PROBABILITY
    };
    !gap_rng.bool_with_prob(gap_p)
}

/// Build the anchor (origin) `NodeSpec`. Anchors are special:
/// - no primitives (no effect on the FPS / click / cost calc)
/// - cost 0
/// - rendered as `BISCUIT_TINY` instead of a normal box
/// - auto-owned in `GameState::default` / `migrate_runtime` so the
///   player's starting frontier is the anchor's king-neighbors
fn anchor_spec() -> NodeSpec {
    // Center the 16×8 cuque inside the 36×10 lot at canvas origin.
    let box_w: u16 = 16;
    let box_h: u16 = 8;
    let off_x = ((LOT_W as u16).saturating_sub(box_w) / 2) as i32;
    let off_y = ((LOT_H as u16).saturating_sub(box_h) / 2) as i32;
    NodeSpec {
        lot: TreeCoord::ORIGIN,
        box_x: off_x,
        box_y: off_y,
        box_w,
        box_h,
        rarity: Rarity::Small,
        primitives: Vec::new(),
        cost: crate::bignum::Mag::ZERO,
        title: "The Cuque".to_string(),
        dominant_target: Target::Fingerer(0),
        is_anchor: true,
    }
}

/// Procgen entry point: returns the node at lot `(x, y)`, or `None` if the
/// lot is empty.
pub fn node_at(x: i32, y: i32) -> Option<NodeSpec> {
    // Origin lot is the anchor — special spec, always present.
    if x == 0 && y == 0 {
        return Some(anchor_spec());
    }
    // Orphan suppression: a lot is a real node only if there is a
    // *strictly-decreasing-manhattan* chain of populated king-neighbors
    // back to origin. The plain "passes_gap_roll AND has a populated
    // neighbor" check used previously allowed isolated pairs at the
    // tree's outskirts to become each other's parent — visible as nodes
    // disconnected from the rest of the tree that the player can never
    // reach. See `reaches_origin` for the predicate; `anchor_of` uses
    // the same reachability check so anchor edges never point at a
    // suppressed lot.
    if !reaches_origin(TreeCoord::new(x, y)) {
        return None;
    }

    let mut rng = SplitMix64::from_coords(TREE_SEED, x, y, SALT_NODE);

    // Rarity is driven by a noise field. Two noise samples: one for
    // "rarity probability" (low frequency, broad zones) and one for "rarity
    // hot spot" (high frequency, isolated peaks).
    let rarity = pick_rarity(x, y, &mut rng);

    // Box dimensions follow rarity. Jitter the box's offset inside its lot
    // so a region of nodes doesn't look gridded.
    let (box_w, box_h) = rarity.box_dims();
    let max_off_x = (LOT_W as u16).saturating_sub(box_w + 2).max(1);
    let max_off_y = (LOT_H as u16).saturating_sub(box_h + 1).max(1);
    let off_x = rng.range_usize(max_off_x as usize) as i32 + 1;
    let off_y = rng.range_usize(max_off_y as usize) as i32 + 1;

    // saturating_mul guards against overflow at extreme lots (|x|
    // larger than `i32::MAX / LOT_W`, ~6e7). Out-of-range box coords
    // get clamped to i32 edges — the renderer's canvas_to_screen
    // bounds-checks them off-screen, so nothing visible breaks.
    let box_x = x.saturating_mul(LOT_W).saturating_add(off_x);
    let box_y = y.saturating_mul(LOT_H).saturating_add(off_y);

    // Primitive count: small=1, notable=2-3, keystone=2-4. Keystones MUST
    // have at least one bane (the procgen biases sign accordingly).
    let count = match rarity {
        Rarity::Small => 1,
        Rarity::Notable => 2 + rng.range_usize(2), // 2..=3
        Rarity::Keystone => 2 + rng.range_usize(3), // 2..=4
    };

    let primitives = roll_primitives(x, y, rarity, count, &mut rng);
    let dominant_target = primitives[0].target;
    let cost = roll_cost(x, y, rarity);

    // Name uses its own RNG stream so naming changes don't shift other
    // procgen properties at a lot.
    let mut name_rng = SplitMix64::from_coords(TREE_SEED, x, y, 0xDA);
    let title = make_title(&mut name_rng, &primitives);

    Some(NodeSpec {
        lot: TreeCoord::new(x, y),
        box_x,
        box_y,
        box_w,
        box_h,
        rarity,
        primitives,
        cost,
        title,
        dominant_target,
        is_anchor: false,
    })
}

fn pick_rarity(x: i32, y: i32, rng: &mut SplitMix64) -> Rarity {
    // Origin neighborhood: always Small. Gives the player a gentle ramp.
    if x.abs() <= 1 && y.abs() <= 1 {
        return Rarity::Small;
    }
    // Biome-density noise: low frequency. High values = "this region has
    // dense, rare goodies"; low = mostly small nodes.
    let density = fbm_2d(TREE_SEED ^ 0x7E, x as f64 * 0.18, y as f64 * 0.18);
    // Rarity hot spot: high frequency. A small fraction of lots roll a
    // keystone here regardless of density.
    let hotspot = value_noise_2d(TREE_SEED ^ 0xBD, x as f64 * 0.81, y as f64 * 0.81);

    // Decision: keystones are scarce. Notables are common in dense biomes.
    if hotspot > 0.96 {
        return Rarity::Keystone;
    }
    if density > 0.65 && rng.next_f64() < 0.35 {
        return Rarity::Notable;
    }
    if hotspot > 0.85 && rng.next_f64() < 0.25 {
        return Rarity::Notable;
    }
    Rarity::Small
}

fn roll_primitives(
    x: i32,
    y: i32,
    rarity: Rarity,
    count: usize,
    rng: &mut SplitMix64,
) -> Vec<Primitive> {
    let mut out = Vec::with_capacity(count);
    let bane_force = matches!(rarity, Rarity::Keystone);
    for i in 0..count {
        // Bias target toward the lot's "biome" — sample a noise field over
        // target indices to give regions a flavor without hard-coding a
        // biome→target table.
        let target = pick_target_biased(x, y, rng);
        let op = pick_op(target, rng);
        // Sign policy (mirrors PoE's small/notable/keystone convention):
        //   - Small / Notable: ALWAYS boon. Players expect "buying a node
        //     makes me stronger" — a pure-debuff Small was a bug, not a
        //     design feature. Tradeoffs only appear at the Keystone tier.
        //   - Keystone: first primitive is a STRONG boon, then enforce
        //     at least one bane so every keystone is a real tradeoff
        //     (its whole identity is "I gave something up for this").
        let force_sign = match rarity {
            Rarity::Small | Rarity::Notable => Some(true), // pure boons
            Rarity::Keystone => {
                if i == 0 {
                    Some(true) // strong opening boon
                } else if i == 1 || (i == count - 1 && !out.iter().any(|p: &Primitive| p.is_bane()))
                {
                    Some(false) // bane (or last-resort bane if none yet)
                } else {
                    None
                }
            }
        };
        let magnitude = roll_magnitude(op, rarity, force_sign, bane_force, x, y, rng);
        out.push(Primitive {
            op,
            target,
            magnitude,
        });
    }
    out
}

/// Pick a target weighted by a low-frequency noise field. Each (x, y)
/// region "prefers" a band of target indices, naturally clustering similar
/// effects into neighborhoods.
fn pick_target_biased(x: i32, y: i32, rng: &mut SplitMix64) -> Target {
    let n_targets = Target::target_count();
    // Sample a biome noise to bias toward a center index. This gives
    // contiguous regions of (e.g.) Fingerer(5) effects without an
    // explicit biome enum.
    let bias = value_noise_2d(TREE_SEED ^ 0xB10, x as f64 * 0.13, y as f64 * 0.13);
    let center = (bias * n_targets as f64) as usize % n_targets;
    // Radius of bias: tighter when in the origin neighborhood (player
    // starts focused on Index Finger), wider elsewhere.
    let radius = if x.unsigned_abs().saturating_add(y.unsigned_abs()) < 4 {
        1.5
    } else {
        4.0
    };

    // 60% of the time pick from a Gaussian-ish window around `center`; 40%
    // of the time pick uniformly to keep regions from being monolithic.
    let idx = if rng.bool_with_prob(0.60) {
        let mag = (rng.range_f64(-1.0, 1.0)) * radius;
        let signed = center as f64 + mag;
        let wrapped = signed.rem_euclid(n_targets as f64) as usize;
        wrapped.min(n_targets - 1)
    } else {
        rng.range_usize(n_targets)
    };
    let mut t = Target::from_index(idx);
    // Force `Fingerer(0)` (Index Finger) for origin so first node is
    // useful to a brand-new player.
    if x == 0 && y == 0 {
        t = Target::Fingerer(0);
    }
    t
}

fn pick_op(target: Target, rng: &mut SplitMix64) -> Op {
    // Op availability depends on target. Each target advertises its valid
    // ops; we pick uniformly from those.
    let ops: &[Op] = match target {
        Target::Fingerer(_) => &[Op::AddPercent, Op::MulFactor, Op::FlatAdd, Op::CostMul],
        Target::AllFingerers => &[Op::AddPercent, Op::MulFactor, Op::FlatAdd],
        Target::Click => &[Op::AddPercent, Op::MulFactor, Op::FlatAdd],
        Target::PowerupSpawn(_) => &[Op::SpawnRateMul],
        Target::PowerupReward(_) => &[Op::EffectMul],
        Target::PowerupDuration(_) => &[Op::EffectMul],
        Target::Prestige => &[Op::AddPercent, Op::MulFactor],
        Target::GreenCoinStrength => &[Op::EffectMul],
    };
    ops[rng.range_usize(ops.len())]
}

fn roll_magnitude(
    op: Op,
    rarity: Rarity,
    force_sign: Option<bool>, // Some(true)=force boon, Some(false)=force bane
    bane_force: bool,         // overall composition: should at least 1 bane appear?
    x: i32,
    y: i32,
    rng: &mut SplitMix64,
) -> f64 {
    // Base "strength multiplier" scales with rarity AND with manhattan
    // distance from origin: deeper lots roll harder magnitudes.
    let dist = x.unsigned_abs().saturating_add(y.unsigned_abs()) as f64;
    // depth_scale cap was 20.0; tightened to 10.0 because every per-node
    // boon stacks across hundreds of bought nodes in the late game, and
    // a 20× depth multiplier on top of the per-roll range produced
    // late-game players whose fps overflowed `f64` (cuques shown as `?`
    // in the HUD, all next-buy costs `?`). The combination of a smaller
    // cap here plus the 1000× shrink on the per-roll ranges keeps even
    // a 1000+ node late-game stack bounded.
    let depth_scale = 1.0 + (dist / 12.0).min(10.0);
    let rarity_scale = match rarity {
        Rarity::Small => 1.0,
        Rarity::Notable => 2.5,
        Rarity::Keystone => 5.0,
    };
    let s = depth_scale * rarity_scale;

    let bane = match force_sign {
        Some(true) => false,
        Some(false) => true,
        None => {
            if bane_force {
                rng.bool_with_prob(0.25)
            } else {
                rng.bool_with_prob(0.05)
            }
        }
    };

    // Per-node magnitudes are all ~1000× smaller than the first cut of
    // the tree (which gave keystone-class buffs like ×1.17 Green Coin
    // spawn rate; two or three stacked turned spawns into a continuous
    // rain and FPS into `Infinity` after a single long session). The
    // shape — boons biased positive, banes biased negative, depth and
    // rarity scale through `s` — is identical; only the absolute scale
    // has shrunk. Keystones still feel distinct from Smalls because the
    // 5× rarity_scale stays, and deep regions still hit harder than
    // origin via depth_scale; both effects just compound much more
    // gently when 500+ nodes are owned.
    match op {
        Op::AddPercent => {
            // Per-node boon at d=0 Small: +0.001% to +0.005% additive.
            // At d=100 Keystone: ~+0.5% per node. Stack of 1000 deep
            // keystones ≈ +5 total (additive), reasonable late-game.
            let mag = rng.range_f64(0.00001, 0.00005) * s;
            if bane { -mag * 0.7 } else { mag }
        }
        Op::MulFactor => {
            // Multiplicative — compounds across the player's bought set.
            // Knocked down ANOTHER 100× past the 1000× rebalance because
            // even ×1.005 per node stacked 10k nodes deep was reaching
            // ×5e21 total. At ×1.00005 per node, 10k nodes is ×1.65,
            // 100k is ×148 — game stays long.
            let boost = rng.range_f64(0.0000002, 0.000001) * s;
            if bane {
                let nerf = rng.range_f64(0.0000005, 0.000004).min(0.00001);
                (1.0 - nerf).max(0.9999)
            } else {
                (1.0 + boost).min(1.00005)
            }
        }
        Op::FlatAdd => {
            // Flat FPS additions are linear (no compounding), so the
            // 1000× shrink stays — these don't suffer from the runaway
            // problem MulFactor/EffectMul do. Adding more nodes adds
            // FPS additively, exactly the linear-grind path we want.
            let mag = rng.range_f64(0.0005, 0.004) * s;
            if bane { -mag * 0.5 } else { mag }
        }
        Op::CostMul => {
            // Multiplicative on fingerer buy cost. Another 100× tighter
            // so even a deep stack of discounts can't trivialize the
            // cost ladder we're explicitly stretching.
            let scale = rng.range_f64(0.0000002, 0.000001) * (1.0 + (dist / 30.0));
            if bane {
                (1.0 + scale).min(1.00005)
            } else {
                (1.0 - scale).max(0.99995)
            }
        }
        Op::SpawnRateMul => {
            // Multiplicative on powerup spawn frequency. The headline
            // regression that started this thread was a single ×1.17
            // GreenCoin SpawnRate. After the 1000× shrink the cap was
            // ×1.005 — still produced ×148 from a 1000-node stack
            // (constant-spawn territory). Another 100× knocks the cap
            // to ×1.00005, so 1000 nodes → ×1.05 spawn rate, barely
            // noticeable. Game wants to be longer; cooldowns stay long.
            let scale = rng.range_f64(0.0000002, 0.0000012) * (1.0 + (dist / 30.0));
            if bane {
                (1.0 - scale).max(0.99995)
            } else {
                (1.0 + scale).min(1.00005)
            }
        }
        Op::EffectMul => {
            // Multiplicative AND two-stage compounding — amplifies the
            // catch-time mult that itself gets persisted on a fingerer.
            // This is the path that produced the 311-trillion-x
            // PurpleCoin in the wild save. Another 100× tightening to
            // ×1.0001 cap means 10k deep nodes → ~×2.7 catch amplifier,
            // not ×1e43. Keeps catch-time mults inside small-integer
            // territory across any realistic session.
            let scale = rng.range_f64(0.0000002, 0.0000015) * (1.0 + (dist / 24.0));
            if bane {
                (1.0 - scale).max(0.9999)
            } else {
                (1.0 + scale).min(1.0001)
            }
        }
    }
}

/// Cost scales with Manhattan distance from origin: deeper = pricier.
/// Flat multiplier on every node's rolled cost — shifts the whole
/// pricing curve up or down without bending it. Use this when "the
/// tree feels too cheap" but the SHAPE of the ramp is correct.
pub const NODE_COST_MULT: f64 = 5.0;

/// Per-lot exponential growth rate. Each step further from the origin
/// multiplies the node's cost by this factor (manhattan distance). The
/// shape of the late-game wall lives here — bump it to make deep
/// nodes balloon faster, lower it for a gentler ramp.
///
/// Reference: fingerers grow at 1.15^count (classic Cookie Clicker
/// 15%-per-buy). The tree grows much steeper because each "lot step"
/// gives only ONE upgrade, vs many buys per fingerer.
///
/// Bumped 1.75 → 2.5 along with the 1000× shrink in `roll_magnitude`.
/// Per-node power is now tiny, so the player wants to buy *many*
/// nodes; a steeper cost ramp keeps the player from sweeping the
/// entire frontier in a single session and lets the alphabetic-suffix
/// number range (10^36+ and beyond) become a real late-game milestone
/// instead of a transient overflow.
pub const NODE_COST_GROWTH: f64 = 2.5;

/// Base cost at distance 0 (before rarity and jitter). The origin lot
/// is auto-owned so this never quotes a real purchase, but it anchors
/// the ramp: cost(d) = NODE_COST_MULT * NODE_BASE_COST * NODE_COST_GROWTH^d
///                    * rarity_factor * jitter.
pub const NODE_BASE_COST: f64 = 50.0;

pub fn roll_cost(x: i32, y: i32, rarity: Rarity) -> crate::bignum::Mag {
    use crate::bignum::Mag;
    let dist = x.unsigned_abs().saturating_add(y.unsigned_abs()) as f64;
    let rarity_factor: f64 = match rarity {
        Rarity::Small => 1.0,
        Rarity::Notable => 5.0,
        Rarity::Keystone => 40.0,
    };
    let mut rng = SplitMix64::from_coords(TREE_SEED, x, y, 0xC0);
    let jitter = rng.range_f64(0.85, 1.25);
    // Compute directly in log10 space so very-deep nodes (`dist` in
    // the thousands) produce a faithful Mag instead of the old
    // `.min(1e300)`-clamped placeholder. Each multiplicative factor is
    // log10'd and summed; the result is `10^(sum)`.
    let log10 = NODE_COST_MULT.log10()
        + NODE_BASE_COST.log10()
        + dist * NODE_COST_GROWTH.log10()
        + rarity_factor.log10()
        + jitter.log10();
    // `.max` a small floor so even the cheapest origin-neighborhood node
    // isn't free: 10 cuques minimum.
    if log10 < 1.0 {
        Mag::from_f64(10.0)
    } else {
        Mag { log10 }
    }
}

/// True when the lots at `a` and `b` are 8-king neighbors (chebyshev
/// distance 1). Returns false for the same lot (a node isn't its own
/// neighbor) and for any pair further away.
pub fn is_king_neighbor(a: TreeCoord, b: TreeCoord) -> bool {
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    dx <= 1 && dy <= 1 && !(dx == 0 && dy == 0)
}

/// For a populated lot, return its "anchor parent" — the deterministic
/// king-neighbor that is GUARANTEED to have an edge to this lot.
///
/// Selection rules, in order:
///   1. Prefer an ORTHOGONAL king-neighbor at strictly smaller manhattan
///      distance from origin that itself reaches origin. This is what
///      controls the cobweb at origin: only origin's 4 orthogonal
///      neighbors will anchor to origin directly, while diagonal
///      neighbors anchor via one of the orthogonal cousins. Cuts
///      origin's incoming anchor-edge count from 8 to 4.
///   2. Fall back to a DIAGONAL strictly-smaller-manhattan neighbor
///      that reaches origin.
///
/// Returns `None` for origin (no parent) and for lots that fail
/// `reaches_origin`. There is deliberately no "any populated neighbor"
/// fallback — that used to exist and was the source of orphan islands:
/// two adjacent populated lots whose only smaller-manhattan neighbors
/// were all gaps would each point at the other, forming a closed loop
/// disconnected from the rest of the tree.
pub fn anchor_of(lot: TreeCoord) -> Option<TreeCoord> {
    if lot == TreeCoord::ORIGIN {
        return None;
    }
    if !reaches_origin(lot) {
        return None;
    }
    let dist = lot.manhattan();
    let mut candidates: Vec<TreeCoord> = neighbors_of(lot).to_vec();
    // Sort by (manhattan, x, y) for stable tie-breaking. Uses the
    // saturating manhattan helper so extreme coords (near i32::MIN /
    // i32::MAX) don't overflow the abs sum.
    candidates.sort_by_key(|n| (n.manhattan(), n.x, n.y));
    let is_orthogonal = |n: &TreeCoord| {
        let dx = n.x.wrapping_sub(lot.x).unsigned_abs();
        let dy = n.y.wrapping_sub(lot.y).unsigned_abs();
        (dx == 0 || dy == 0) && (dx + dy == 1)
    };
    // Pass 1: orthogonal + strictly-smaller-manhattan + reaches-origin.
    for n in &candidates {
        if n.manhattan() >= dist {
            continue;
        }
        if !is_orthogonal(n) {
            continue;
        }
        if reaches_origin(*n) {
            return Some(*n);
        }
    }
    // Pass 2: diagonal + strictly-smaller-manhattan + reaches-origin.
    // Lets a lot whose 4 orthogonal neighbors all fail (gap or orphaned)
    // still find a parent toward origin.
    for n in &candidates {
        if n.manhattan() >= dist {
            continue;
        }
        if reaches_origin(*n) {
            return Some(*n);
        }
    }
    None
}

// Memoized "is this lot connected to origin via a strictly-decreasing-
// manhattan chain of populated king-neighbors". Pure function of
// (lot.x, lot.y) — the procgen seed and gap-roll function are both
// deterministic — so caching results forever is sound. The map only
// grows as the player explores; in practice it stays in the low
// thousands of entries (one per lot the renderer ever asks about).
thread_local! {
    static REACHES_ORIGIN_MEMO: RefCell<HashMap<TreeCoord, bool>> = RefCell::new(HashMap::new());
}

/// True iff `lot` admits a path of populated king-neighbors back to
/// origin where every step strictly *decreases* manhattan distance.
/// Origin itself returns true; unpopulated lots (gap roll) return false.
///
/// The strict-decrease constraint is what makes the procgen orphan-free:
/// it's impossible to form a cycle among lots if every edge in the
/// chain reduces a non-negative integer (manhattan distance), so any
/// successful chain terminates at origin in at most `lot.manhattan()`
/// steps. Recursion depth is therefore bounded by manhattan distance,
/// and the thread-local memo collapses overlapping sub-chains across
/// the many lots the renderer asks about per frame.
pub fn reaches_origin(lot: TreeCoord) -> bool {
    if lot == TreeCoord::ORIGIN {
        return true;
    }
    if let Some(v) = REACHES_ORIGIN_MEMO.with(|m| m.borrow().get(&lot).copied()) {
        return v;
    }
    if !passes_gap_roll(lot.x, lot.y) {
        REACHES_ORIGIN_MEMO.with(|m| m.borrow_mut().insert(lot, false));
        return false;
    }
    let dist = lot.manhattan();
    let mut result = false;
    for n in neighbors_of(lot) {
        if n.manhattan() < dist && reaches_origin(n) {
            result = true;
            break;
        }
    }
    REACHES_ORIGIN_MEMO.with(|m| m.borrow_mut().insert(lot, result));
    result
}

/// Sample a low-frequency density noise field at the midpoint between
/// two lots. Returns `[0.0, 1.0]`. Drives the per-edge connection
/// probability so the tree alternates between dense pockets (most
/// adjacent pairs connect) and sinuous corridors (most don't). Biome
/// width ≈ 8 lots so the variation reads clearly when panning.
fn edge_density(a: TreeCoord, b: TreeCoord) -> f64 {
    let mx = (a.x as f64 + b.x as f64) * 0.5;
    let my = (a.y as f64 + b.y as f64) * 0.5;
    value_noise_2d(TREE_SEED ^ 0xDE, mx * 0.12, my * 0.12)
}

/// Procgen decision: does an edge exist between two neighboring lots?
/// Returns false for non-neighbors. Both lots must actually contain a
/// node — call `node_at` first.
///
/// Three sources of edges, OR'd together:
///   1. **Anchor edges** — every non-origin populated lot has a
///      guaranteed edge to its `anchor_of` neighbor. Keeps every node
///      reachable from origin transitively even when the random rolls
///      come up empty.
///   2. **Orthogonal procgen edges** — random roll with probability
///      driven by a low-frequency density-noise field. Probability
///      ranges from ~50% in "sinuous" regions to ~95% in "dense"
///      pockets. Adjacent boxes might NOT be connected in sinuous
///      zones, which is the explorational variety the player wants.
///   3. **Diagonal procgen edges** — same noise-driven probability
///      scaled to a lower band (~8% sparse, ~28% dense), plus the
///      hard suppression when both L-bend corner lots are occupied
///      (the visual line would otherwise cross an unrelated box).
pub fn edge_exists(a: TreeCoord, b: TreeCoord) -> bool {
    if !is_king_neighbor(a, b) {
        return false;
    }
    // Anchor edges always exist — preserve "no orphans" regardless of
    // the noise-driven probability dropping to zero.
    if anchor_of(a) == Some(b) || anchor_of(b) == Some(a) {
        return true;
    }
    let dx = (a.x - b.x).abs();
    let dy = (a.y - b.y).abs();
    let diagonal = dx == 1 && dy == 1;
    // Canonicalize: smaller (x, y) first.
    let (lo, hi) = if (a.x, a.y) < (b.x, b.y) {
        (a, b)
    } else {
        (b, a)
    };
    if diagonal {
        // Both L-bend corner lots occupied → suppress to avoid visually
        // crossing an unrelated box.
        let mid_h = TreeCoord::new(hi.x, lo.y);
        let mid_v = TreeCoord::new(lo.x, hi.y);
        if node_at(mid_h.x, mid_h.y).is_some() && node_at(mid_v.x, mid_v.y).is_some() {
            return false;
        }
    }
    let density = edge_density(a, b);
    let p = if diagonal {
        // Diagonals are rare bonus shortcuts. 2-10% range — most
        // diagonals only exist as anchor edges (i.e. when a node's
        // parent happens to be a diagonal neighbor).
        0.02 + 0.08 * density
    } else {
        // Orthogonal: 12-35%. Combined with the anchor-edge
        // guarantee (every non-origin node has exactly one anchor
        // edge to its parent toward origin), this gives a clear
        // "spine of the tree" plus occasional extra branches. Origin's
        // 8 king-neighbors still all get an anchor edge to the cuque,
        // but EXTRA edges between siblings are now uncommon — no more
        // cobweb at origin.
        0.12 + 0.23 * density
    };
    let mut rng = SplitMix64::from_coords(
        TREE_SEED,
        lo.x.wrapping_add(hi.x.wrapping_mul(7919)),
        lo.y.wrapping_add(hi.y.wrapping_mul(6151)),
        SALT_EDGE,
    );
    rng.bool_with_prob(p)
}

/// For a diagonal edge between `a` and `b`, return the "L-bend corner
/// lot" the renderer should route through — the corner lot that's
/// empty. `None` for non-diagonal pairs (renderer chooses by longer-axis
/// heuristic instead).
///
/// Invariant: if `edge_exists(a, b)` is true for a diagonal pair, at
/// least one of the two corner lots is empty (the other diagonal-suppress
/// rule above guarantees this), so this returns `Some` for every
/// existing diagonal edge.
pub fn diagonal_route_via(a: TreeCoord, b: TreeCoord) -> Option<TreeCoord> {
    let (lo, hi) = if (a.x, a.y) < (b.x, b.y) {
        (a, b)
    } else {
        (b, a)
    };
    let dx = (hi.x - lo.x).abs();
    let dy = (hi.y - lo.y).abs();
    if dx != 1 || dy != 1 {
        return None;
    }
    let mid_h = TreeCoord::new(hi.x, lo.y);
    let mid_v = TreeCoord::new(lo.x, hi.y);
    if node_at(mid_h.x, mid_h.y).is_none() {
        return Some(mid_h);
    }
    if node_at(mid_v.x, mid_v.y).is_none() {
        return Some(mid_v);
    }
    None
}

/// The 8 neighboring lots of `c`.
pub fn neighbors_of(c: TreeCoord) -> [TreeCoord; 8] {
    [
        TreeCoord::new(c.x - 1, c.y - 1),
        TreeCoord::new(c.x, c.y - 1),
        TreeCoord::new(c.x + 1, c.y - 1),
        TreeCoord::new(c.x - 1, c.y),
        TreeCoord::new(c.x + 1, c.y),
        TreeCoord::new(c.x - 1, c.y + 1),
        TreeCoord::new(c.x, c.y + 1),
        TreeCoord::new(c.x + 1, c.y + 1),
    ]
}

/// Helper: all `TreeCoord`s that are king-neighbors of `c` AND have a node.
pub fn neighbors_with_nodes(c: TreeCoord) -> Vec<TreeCoord> {
    neighbors_of(c)
        .into_iter()
        .filter(|n| node_at(n.x, n.y).is_some())
        .collect()
}

/// Cells along the rendered edge between two nodes, in canonical
/// lo→hi lot order (lex on `(x, y)`). Returns an empty Vec if either
/// endpoint has no node.
///
/// Canonical direction is load-bearing: `bresenham_path(A, B)` and
/// `bresenham_path(B, A)` produce DIFFERENT cell sequences for the same
/// endpoints, so we sort the inputs here. Pre-buy and post-buy
/// rendering, plus the unlock-anim's completion check, all call this
/// function with the same pair of lots — without canonicalization the
/// wave would energize a different staircase shape from the grey base
/// line, and you'd see the path geometry flip the moment the wave
/// started.
///
/// The case-split mirrors `ui::tree::draw_edge`:
///   - boxes vertically aligned within ±2 columns → straight vertical run
///     at the midpoint x.
///   - boxes horizontally aligned within ±2 rows → straight horizontal
///     run at the midpoint y.
///   - otherwise → Bresenham staircase between the two centers.
pub fn edge_path_cells(a: TreeCoord, b: TreeCoord) -> Vec<(i32, i32)> {
    let (a, b) = if (a.x, a.y) <= (b.x, b.y) {
        (a, b)
    } else {
        (b, a)
    };
    let Some(an) = node_at(a.x, a.y) else {
        return Vec::new();
    };
    let Some(bn) = node_at(b.x, b.y) else {
        return Vec::new();
    };
    let acx = an.box_x + (an.box_w as i32) / 2;
    let acy = an.box_y + (an.box_h as i32) / 2;
    let bcx = bn.box_x + (bn.box_w as i32) / 2;
    let bcy = bn.box_y + (bn.box_h as i32) / 2;

    if (acx - bcx).abs() <= 2 {
        let mid_x = (acx + bcx) / 2;
        let step: i32 = if acy <= bcy { 1 } else { -1 };
        let mut path = Vec::new();
        let mut y = acy;
        loop {
            path.push((mid_x, y));
            if y == bcy {
                break;
            }
            y += step;
        }
        return path;
    }
    if (acy - bcy).abs() <= 2 {
        let mid_y = (acy + bcy) / 2;
        let step: i32 = if acx <= bcx { 1 } else { -1 };
        let mut path = Vec::new();
        let mut x = acx;
        loop {
            path.push((x, mid_y));
            if x == bcx {
                break;
            }
            x += step;
        }
        return path;
    }
    bresenham_path(acx, acy, bcx, bcy)
}

/// Count cells at the START of `path` whose canvas-grid coords lie
/// inside the rect `(box_x, box_y, box_w, box_h)`. Used by the edge
/// renderer + the unlock-anim's completion check to skip the "inside
/// the source box" prefix when seeding the wavefront. Iteration stops
/// at the first cell that's outside the rect — interior gaps don't
/// inflate the count.
pub fn count_leading_in_rect(
    path: &[(i32, i32)],
    box_x: i32,
    box_y: i32,
    box_w: u16,
    box_h: u16,
) -> usize {
    let mut count = 0;
    for &(cx, cy) in path {
        if cx >= box_x && cx < box_x + box_w as i32 && cy >= box_y && cy < box_y + box_h as i32 {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// Mirror of `count_leading_in_rect` but counting from the END of
/// the path inward.
pub fn count_trailing_in_rect(
    path: &[(i32, i32)],
    box_x: i32,
    box_y: i32,
    box_w: u16,
    box_h: u16,
) -> usize {
    let mut count = 0;
    for &(cx, cy) in path.iter().rev() {
        if cx >= box_x && cx < box_x + box_w as i32 && cy >= box_y && cy < box_y + box_h as i32 {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// Bresenham-style staircase between (ax, ay) and (bx, by) in
/// canvas-grid coords. Single-axis steps only, interleaved by
/// cross-multiplied normalized progress so the staircase doesn't
/// degenerate into a one-big-L jolt. Returns cells in start→end order.
pub fn bresenham_path(ax: i32, ay: i32, bx: i32, by: i32) -> Vec<(i32, i32)> {
    let mut path = Vec::with_capacity(((bx - ax).abs() + (by - ay).abs() + 1) as usize);
    let mut x = ax;
    let mut y = ay;
    let dx = (bx - ax).abs();
    let dy = (by - ay).abs();
    let sx: i32 = (bx - ax).signum();
    let sy: i32 = (by - ay).signum();
    path.push((x, y));
    let mut xs: i64 = 0;
    let mut ys: i64 = 0;
    while x != bx || y != by {
        let step_x_now = if x == bx {
            false
        } else if y == by {
            true
        } else {
            (xs + 1) * (dy as i64) <= (ys + 1) * (dx as i64)
        };
        if step_x_now {
            x += sx;
            xs += 1;
        } else {
            y += sy;
            ys += 1;
        }
        path.push((x, y));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fingerer::FINGERERS;

    #[test]
    fn origin_always_has_a_node() {
        assert!(
            node_at(0, 0).is_some(),
            "origin lot must always be populated"
        );
    }

    #[test]
    fn origin_is_index_finger_small() {
        let n = node_at(0, 0).unwrap();
        assert_eq!(n.rarity, Rarity::Small);
        assert!(
            matches!(n.dominant_target, Target::Fingerer(0)),
            "origin lot should target Index Finger so the first buy is useful"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        // Use a lot guaranteed to be populated regardless of GAP_PROBABILITY
        // tuning — origin neighborhood (|x|<=1 AND |y|<=1) is always
        // populated. (3, -2) is now a gap with GAP_PROBABILITY = 0.35.)
        let a = node_at(1, 1).unwrap();
        let b = node_at(1, 1).unwrap();
        assert_eq!(a.lot, b.lot);
        assert_eq!(a.rarity, b.rarity);
        assert_eq!(a.box_x, b.box_x);
        assert_eq!(a.box_y, b.box_y);
        assert_eq!(a.cost, b.cost);
        assert_eq!(a.title, b.title);
        assert_eq!(a.primitives.len(), b.primitives.len());
        for (pa, pb) in a.primitives.iter().zip(b.primitives.iter()) {
            assert_eq!(pa.op, pb.op);
            assert_eq!(pa.target, pb.target);
            assert_eq!(pa.magnitude, pb.magnitude);
        }
    }

    #[test]
    fn box_fits_within_lot_bounds() {
        // Sample a broad area; every generated box must stay inside its
        // lot (no overlapping into neighbor lots, no crossing the canvas
        // grid lines we'd later use for edge routing).
        for x in -10..=10 {
            for y in -10..=10 {
                if let Some(n) = node_at(x, y) {
                    let lot_min_x = x * LOT_W;
                    let lot_max_x = (x + 1) * LOT_W;
                    let lot_min_y = y * LOT_H;
                    let lot_max_y = (y + 1) * LOT_H;
                    assert!(
                        n.box_x >= lot_min_x,
                        "box {} below lot min {}",
                        n.box_x,
                        lot_min_x
                    );
                    assert!(
                        (n.box_x + n.box_w as i32) <= lot_max_x,
                        "box at ({x},{y}) overflows lot right edge"
                    );
                    assert!(n.box_y >= lot_min_y);
                    assert!((n.box_y + n.box_h as i32) <= lot_max_y);
                }
            }
        }
    }

    #[test]
    fn gaps_exist_outside_origin_neighborhood() {
        // Count populated vs empty lots in a far-out region.
        let mut pop = 0;
        let mut empty = 0;
        for x in 10..=20 {
            for y in 10..=20 {
                if node_at(x, y).is_some() {
                    pop += 1;
                } else {
                    empty += 1;
                }
            }
        }
        assert!(empty > 0, "tree must have gaps");
        assert!(pop > 0, "tree must also have nodes");
    }

    #[test]
    fn edges_are_deterministic_and_symmetric() {
        let a = TreeCoord::new(3, 4);
        let b = TreeCoord::new(4, 5);
        assert_eq!(edge_exists(a, b), edge_exists(b, a));
        assert_eq!(edge_exists(a, b), edge_exists(a, b));
    }

    #[test]
    fn non_neighbors_never_have_edges() {
        let a = TreeCoord::new(0, 0);
        let b = TreeCoord::new(5, 5);
        assert!(!edge_exists(a, b));
    }

    #[test]
    fn keystones_have_at_least_one_bane() {
        // Walk a large region; for every keystone, assert composition.
        for x in -30..=30 {
            for y in -30..=30 {
                if let Some(n) = node_at(x, y)
                    && n.rarity == Rarity::Keystone
                {
                    assert!(
                        n.primitives.iter().any(|p| p.is_bane()),
                        "keystone at ({x},{y}) has no bane primitive: {:?}",
                        n.primitives
                    );
                    assert!(
                        n.primitives.iter().any(|p| !p.is_bane()),
                        "keystone at ({x},{y}) has no boon primitive"
                    );
                }
            }
        }
    }

    #[test]
    fn cost_grows_with_distance() {
        let near = roll_cost(0, 0, Rarity::Small);
        let far = roll_cost(20, 20, Rarity::Small);
        // log10(far) - log10(near) >= 2 means far >= 100x near.
        assert!(
            far.log10 - near.log10 > 2.0,
            "far ({}) should be >> near ({})",
            crate::format::big_mag(far),
            crate::format::big_mag(near)
        );
    }

    /// Diagnostic table — not a real assertion, prints the live ramp at
    /// the current `NODE_COST_MULT` / `NODE_COST_GROWTH` values. Gated
    /// behind `#[ignore]` so it doesn't run in normal CI; invoke
    /// explicitly when tuning balance:
    ///
    ///   `cargo test --release dump_cost_table -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_cost_table() {
        eprintln!(
            "NODE_COST_MULT={NODE_COST_MULT}  NODE_COST_GROWTH={NODE_COST_GROWTH}  base={NODE_BASE_COST}"
        );
        eprintln!(
            "{:>4}  {:>14}  {:>14}  {:>14}",
            "dist", "Small", "Notable", "Keystone"
        );
        for &d in &[1, 2, 3, 5, 7, 10, 12, 15, 18, 20, 25, 30, 35, 40, 50i32] {
            // Use (d, 0) so manhattan == d.
            let small = roll_cost(d, 0, Rarity::Small);
            let notable = roll_cost(d, 0, Rarity::Notable);
            let keystone = roll_cost(d, 0, Rarity::Keystone);
            eprintln!(
                "{:>4}  {:>14}  {:>14}  {:>14}",
                d,
                crate::format::big_mag(small),
                crate::format::big_mag(notable),
                crate::format::big_mag(keystone)
            );
        }
    }

    #[allow(dead_code)]
    fn format_cost(c: f64) -> String {
        if c >= 1e15 {
            format!("{:.2} quad", c / 1e15)
        } else if c >= 1e12 {
            format!("{:.2} T", c / 1e12)
        } else if c >= 1e9 {
            format!("{:.2} B", c / 1e9)
        } else if c >= 1e6 {
            format!("{:.2} M", c / 1e6)
        } else if c >= 1e3 {
            format!("{:.2} k", c / 1e3)
        } else {
            format!("{c:.0}")
        }
    }

    #[test]
    fn box_dims_match_rarity() {
        assert_eq!(Rarity::Small.box_dims(), (14, 3));
        assert_eq!(Rarity::Notable.box_dims(), (18, 5));
        assert_eq!(Rarity::Keystone.box_dims(), (24, 7));
    }

    #[test]
    fn fingerer_target_index_in_range() {
        // Any generated Fingerer(idx) must be a valid catalog index.
        for x in -10..=10 {
            for y in -10..=10 {
                if let Some(n) = node_at(x, y) {
                    for p in &n.primitives {
                        if let Target::Fingerer(i) = p.target {
                            assert!(
                                (i as usize) < FINGERERS.len(),
                                "fingerer target idx {i} out of range"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_orphan_components_within_visible_radius() {
        use std::collections::{HashSet, VecDeque};
        // 40 covers a generous "explore for an hour" radius. Before the
        // `reaches_origin` orphan check this would surface ~50 orphans;
        // the player saw them as isolated 2-node islands in the panned
        // canvas with no path back to The Cuque.
        let radius: i32 = 40;
        let mut populated: HashSet<TreeCoord> = HashSet::new();
        for x in -radius..=radius {
            for y in -radius..=radius {
                if node_at(x, y).is_some() {
                    populated.insert(TreeCoord::new(x, y));
                }
            }
        }
        let mut reached: HashSet<TreeCoord> = HashSet::new();
        let mut q: VecDeque<TreeCoord> = VecDeque::new();
        q.push_back(TreeCoord::ORIGIN);
        reached.insert(TreeCoord::ORIGIN);
        while let Some(c) = q.pop_front() {
            for n in neighbors_of(c) {
                if !populated.contains(&n) {
                    continue;
                }
                if reached.contains(&n) {
                    continue;
                }
                if edge_exists(c, n) {
                    reached.insert(n);
                    q.push_back(n);
                }
            }
        }
        let orphans: Vec<_> = populated.difference(&reached).copied().collect();
        if !orphans.is_empty() {
            for o in orphans.iter().take(20) {
                eprintln!("orphan at ({}, {}) manhattan={}", o.x, o.y, o.manhattan());
            }
        }
        assert!(
            orphans.is_empty(),
            "{} orphan(s) detected in radius {}",
            orphans.len(),
            radius
        );
    }

    #[test]
    fn multiplicative_primitives_stay_under_per_node_caps() {
        // Per-node caps are the load-bearing balance knob for keeping the
        // game long: every multiplicative op compounds across the bought
        // set, so a generous cap turns into runaway numbers. Caps were
        // tightened ANOTHER 100× past the initial rebalance after the
        // "make the game longer" pass:
        //   MulFactor / CostMul / SpawnRateMul: ×1.00005
        //   EffectMul:                          ×1.0001
        // EffectMul gets a slightly looser ceiling because it's
        // applied at catch time (once per powerup), not on every fps
        // tick like the others.
        const MUL_CAP: f64 = 1.00005;
        const EFFECT_CAP: f64 = 1.0001;
        for x in -60..=60 {
            for y in -60..=60 {
                if let Some(n) = node_at(x, y) {
                    for p in &n.primitives {
                        match p.op {
                            Op::MulFactor | Op::CostMul | Op::SpawnRateMul => {
                                assert!(
                                    p.magnitude <= MUL_CAP + 1e-12,
                                    "{:?} at ({x},{y}) is {} (cap {MUL_CAP})",
                                    p.op,
                                    p.magnitude
                                );
                            }
                            Op::EffectMul => {
                                assert!(
                                    p.magnitude <= EFFECT_CAP + 1e-12,
                                    "EffectMul at ({x},{y}) is {} (cap {EFFECT_CAP})",
                                    p.magnitude
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn no_zero_magnitude_primitives() {
        // Procgen should never produce a useless 0% / ×1.0 primitive.
        for x in -10..=10 {
            for y in -10..=10 {
                if let Some(n) = node_at(x, y) {
                    for p in &n.primitives {
                        assert!(p.magnitude != 0.0, "zero-magnitude primitive at ({x},{y})");
                        if matches!(
                            p.op,
                            Op::MulFactor | Op::CostMul | Op::SpawnRateMul | Op::EffectMul
                        ) {
                            assert!(
                                (p.magnitude - 1.0).abs() > 1e-9,
                                "identity-magnitude mul-style primitive at ({x},{y})"
                            );
                        }
                    }
                }
            }
        }
    }
}
