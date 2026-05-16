//! Infinite procedural upgrade tree.
//!
//! Every node in the tree is a pure function of `(TREE_SEED, lot_x, lot_y)`.
//! Save state stores only the set of bought lot coordinates plus a pan
//! position; everything else (geometry, effects, names, costs, edges) is
//! regenerated on demand from the seed.
//!
//! The seed is a FNV-1a 64-bit hash of the literal string `"DEADBEEF_BANANA"`
//! — same value for every player, every install, forever. The tree itself is
//! the meta-game.

pub mod aggregate;
pub mod coord;
pub mod naming;
pub mod node;
pub mod noise;
pub mod primitive;
pub mod rng;
pub mod seed;
pub mod state;

pub use aggregate::{FingererTreeContrib, TreeAggregate};
pub use coord::TreeCoord;
pub use node::{
    LOT_H, LOT_W, NodeSpec, Rarity, anchor_of, diagonal_route_via, edge_exists, neighbors_of,
    node_at,
};
pub use primitive::{Op, Primitive, Target};
pub use seed::{TREE_SEED, TREE_SEED_LITERAL};
pub use state::UpgradeTreeState;
