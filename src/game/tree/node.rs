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
    pub cost: f64,
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

/// Pure-local "is this lot allowed to hold a node?" predicate. Doesn't
/// recurse, doesn't check neighbors — just the gap roll and the
/// always-populated origin neighborhood. Composing this with the
/// has-at-least-one-populated-neighbor check (in `node_at`) is how the
/// procgen suppresses truly-isolated lots.
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
        cost: 0.0,
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
    if !passes_gap_roll(x, y) {
        return None;
    }
    // Suppress truly-isolated lots — populated lots that have ZERO
    // populated king-neighbors. Such nodes would be unreachable forever
    // (no anchor edge, no procgen edge possible) and are confusing
    // visual noise. With the 80% population rate this only ever fires
    // on lots with 8 consecutive gap rolls (P ≈ 2.6 × 10⁻⁶), but those
    // do appear deep in the canvas — and they look like bugs.
    let has_pop_neighbor = neighbors_of(TreeCoord::new(x, y))
        .iter()
        .any(|n| passes_gap_roll(n.x, n.y));
    if !has_pop_neighbor {
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

    let box_x = x * LOT_W + off_x;
    let box_y = y * LOT_H + off_y;

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
    let radius = if x.abs() + y.abs() < 4 { 1.5 } else { 4.0 };

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
    let dist = (x.abs() + y.abs()) as f64;
    let depth_scale = 1.0 + (dist / 12.0).min(20.0); // capped so keystones don't run away
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

    match op {
        Op::AddPercent => {
            // Small node: 1-5% additive; Keystone boon: up to 40%+.
            let mag = rng.range_f64(0.01, 0.05) * s;
            if bane { -mag * 0.7 } else { mag }
        }
        Op::MulFactor => {
            // Multiplicative boons: 1.05× small, 2-5× keystone.
            let boost = rng.range_f64(0.02, 0.10) * s; // additive 'extra'
            if bane {
                // Bane mul: 0.5x ... 0.85x (rarer); deeper = more punishing
                let nerf = rng.range_f64(0.05, 0.40).min(0.50);
                (1.0 - nerf).max(0.01)
            } else {
                1.0 + boost
            }
        }
        Op::FlatAdd => {
            let mag = rng.range_f64(0.5, 4.0) * s;
            if bane { -mag * 0.5 } else { mag }
        }
        Op::CostMul => {
            // < 1 is boon (discount), > 1 is bane (inflation).
            let scale = rng.range_f64(0.02, 0.10) * (1.0 + (dist / 30.0));
            if bane {
                (1.0 + scale).min(2.0)
            } else {
                (1.0 - scale).max(0.50)
            }
        }
        Op::SpawnRateMul => {
            // > 1 is boon (more frequent), < 1 is bane (rarer).
            let scale = rng.range_f64(0.02, 0.12) * (1.0 + (dist / 30.0));
            if bane {
                (1.0 - scale).max(0.30)
            } else {
                (1.0 + scale).min(3.0)
            }
        }
        Op::EffectMul => {
            // > 1 is boon, < 1 is bane.
            let scale = rng.range_f64(0.02, 0.15) * (1.0 + (dist / 24.0));
            if bane {
                (1.0 - scale).max(0.10)
            } else {
                (1.0 + scale).min(5.0)
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
pub const NODE_COST_GROWTH: f64 = 1.75;

/// Base cost at distance 0 (before rarity and jitter). The origin lot
/// is auto-owned so this never quotes a real purchase, but it anchors
/// the ramp: cost(d) = NODE_COST_MULT * NODE_BASE_COST * NODE_COST_GROWTH^d
///                    * rarity_factor * jitter.
pub const NODE_BASE_COST: f64 = 50.0;

pub fn roll_cost(x: i32, y: i32, rarity: Rarity) -> f64 {
    let dist = (x.abs() + y.abs()) as f64;
    let depth_factor = NODE_COST_GROWTH.powf(dist);
    let rarity_factor = match rarity {
        Rarity::Small => 1.0,
        Rarity::Notable => 5.0,
        Rarity::Keystone => 40.0,
    };
    // Tiny per-lot jitter so adjacent same-rarity nodes don't all cost
    // exactly the same.
    let mut rng = SplitMix64::from_coords(TREE_SEED, x, y, 0xC0);
    let jitter = rng.range_f64(0.85, 1.25);
    // `NODE_COST_GROWTH.powf(dist)` overflows to `f64::INFINITY` around
    // dist ≈ 1232 (at growth=1.75). Clamp to a safe ceiling so the
    // info-pane "Cost: ?" path is replaced by a real finite quote — the
    // player won't ever afford it anyway, but the rendering and the
    // `affordable_cuques() >= cost` check both behave better with a
    // concrete number.
    let raw = NODE_COST_MULT * NODE_BASE_COST * depth_factor * rarity_factor * jitter;
    raw.min(1e300).floor().max(10.0)
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
///      distance from origin. This is what controls the cobweb at
///      origin: only origin's 4 orthogonal neighbors will anchor to
///      origin directly, while diagonal neighbors anchor via one of the
///      orthogonal cousins. Cuts origin's incoming anchor-edge count
///      from 8 to 4, halving the visual density.
///   2. Fall back to a DIAGONAL strictly-smaller-manhattan neighbor if
///      no orthogonal candidate exists (happens when all 4 orth
///      neighbors are gaps — rare but possible at gap_p=0.35).
///   3. Last resort: any populated king-neighbor (same/greater
///      manhattan). Belt-and-suspenders; almost never triggers.
///
/// Returns `None` for origin (no parent) and for unpopulated lots.
pub fn anchor_of(lot: TreeCoord) -> Option<TreeCoord> {
    if lot == TreeCoord::ORIGIN {
        return None;
    }
    if !passes_gap_roll(lot.x, lot.y) {
        return None;
    }
    let dist = lot.x.abs() + lot.y.abs();
    let mut candidates: Vec<TreeCoord> = neighbors_of(lot).to_vec();
    // Sort by (manhattan, x, y) for stable tie-breaking.
    candidates.sort_by_key(|n| (n.x.abs() + n.y.abs(), n.x, n.y));
    let is_orthogonal = |n: &TreeCoord| {
        let dx = (n.x - lot.x).abs();
        let dy = (n.y - lot.y).abs();
        (dx == 0 || dy == 0) && (dx + dy == 1)
    };
    // Pass 1: orthogonal + strictly-smaller-manhattan + populated.
    for n in &candidates {
        if (n.x.abs() + n.y.abs()) >= dist {
            continue;
        }
        if !is_orthogonal(n) {
            continue;
        }
        if passes_gap_roll(n.x, n.y) {
            return Some(*n);
        }
    }
    // Pass 2: diagonal + strictly-smaller-manhattan + populated. Lets
    // a lot whose 4 orthogonal neighbors are all gaps still find a
    // parent toward origin.
    for n in &candidates {
        if (n.x.abs() + n.y.abs()) >= dist {
            continue;
        }
        if passes_gap_roll(n.x, n.y) {
            return Some(*n);
        }
    }
    // Pass 3: any populated neighbor. Last resort.
    for n in &candidates {
        if passes_gap_roll(n.x, n.y) {
            return Some(*n);
        }
    }
    None
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
        let mid_h = TreeCoord::new(hi.x, lo.y);
        let mid_v = TreeCoord::new(lo.x, hi.y);
        // Visual-crossing suppression: both L-bend corner lots occupied
        // → the diagonal line would slice across an unrelated box.
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
    if !rng.bool_with_prob(p) {
        return false;
    }
    // Anchor-redundancy suppression: if a 2-hop ANCHOR path through a
    // common king-neighbor already connects A and B, the direct edge
    // is just closing a triangle on top of the spine. Suppress so the
    // tree reads as branches, not triangles.
    //
    // Anchor edges are permanent (the `anchor_of()` early-return above
    // guarantees this), so a 2-hop made entirely of anchor edges is
    // itself permanent — collapsing the third edge can't disconnect A
    // from B. Using the anchor-only criterion also dodges the failure
    // mode where all three sides of an all-procgen triangle would
    // suppress each other if we did a recursive `edge_exists` check.
    let is_anchor_edge =
        |x: TreeCoord, y: TreeCoord| -> bool { anchor_of(x) == Some(y) || anchor_of(y) == Some(x) };
    for c in neighbors_of(a) {
        if c == b || c == a {
            continue;
        }
        if !is_king_neighbor(c, b) {
            continue;
        }
        if node_at(c.x, c.y).is_none() {
            continue;
        }
        if is_anchor_edge(a, c) && is_anchor_edge(c, b) {
            return false;
        }
    }
    true
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
                if let Some(n) = node_at(x, y) {
                    if n.rarity == Rarity::Keystone {
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
    }

    #[test]
    fn non_anchor_edges_dont_close_anchor_triangles() {
        // Walk every king-pair in a sizable region. For each non-anchor
        // edge that exists, prove no common king-neighbor has anchor
        // edges to both endpoints — i.e. the edge isn't closing a
        // triangle on top of the spine.
        for x in -30..=30 {
            for y in -30..=30 {
                let a = TreeCoord::new(x, y);
                for &(dx, dy) in &[
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                    (1, 1),
                    (1, -1),
                    (-1, 1),
                    (-1, -1),
                ] {
                    let b = TreeCoord::new(x + dx, y + dy);
                    if (a.x, a.y) > (b.x, b.y) {
                        continue;
                    }
                    if !edge_exists(a, b) {
                        continue;
                    }
                    // Anchor edges get a pass — they're load-bearing for
                    // connectivity and the suppression rule explicitly
                    // preserves them.
                    if anchor_of(a) == Some(b) || anchor_of(b) == Some(a) {
                        continue;
                    }
                    for c in neighbors_of(a) {
                        if c == a || c == b || !is_king_neighbor(c, b) {
                            continue;
                        }
                        if node_at(c.x, c.y).is_none() {
                            continue;
                        }
                        let ac_anchor = anchor_of(a) == Some(c) || anchor_of(c) == Some(a);
                        let cb_anchor = anchor_of(c) == Some(b) || anchor_of(b) == Some(c);
                        assert!(
                            !(ac_anchor && cb_anchor),
                            "non-anchor edge {a:?} <-> {b:?} closes an anchor \
                             triangle through {c:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cost_grows_with_distance() {
        let near = roll_cost(0, 0, Rarity::Small);
        let far = roll_cost(20, 20, Rarity::Small);
        assert!(far > near * 100.0, "far ({far}) should be >> near ({near})");
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
            // Use (d, 0) so manhattan == d. Average over a small sample to
            // smooth out jitter, since jitter is seeded from (x, y).
            let small = roll_cost(d, 0, Rarity::Small);
            let notable = roll_cost(d, 0, Rarity::Notable);
            let keystone = roll_cost(d, 0, Rarity::Keystone);
            eprintln!(
                "{:>4}  {:>14}  {:>14}  {:>14}",
                d,
                format_cost(small),
                format_cost(notable),
                format_cost(keystone)
            );
        }
    }

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
