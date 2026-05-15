//! Lot coordinates on the infinite tree canvas.

use serde::{Deserialize, Serialize};

/// Identifies a lot in the infinite procedural tree. The pair `(x, y)` is
/// the **stable id** of a node — saves store only owned lot coords, and the
/// generator regenerates every other property from `(TREE_SEED, x, y)`.
///
/// Origin `(0, 0)` is the player's starting lot — the Index Finger homeland.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TreeCoord {
    pub x: i32,
    pub y: i32,
}

impl TreeCoord {
    pub const ORIGIN: TreeCoord = TreeCoord { x: 0, y: 0 };

    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    pub fn manhattan(self) -> u32 {
        self.x.unsigned_abs() + self.y.unsigned_abs()
    }
}
