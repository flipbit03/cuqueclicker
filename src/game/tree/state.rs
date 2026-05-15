//! Persistent state for the upgrade tree.
//!
//! Saved between sessions: `bought`, `cursor`, `last_bought`.
//! Reconstructed on load: the `TreeAggregate` cache (lives on `GameState`,
//! `#[serde(skip)]`, rebuilt by `migrate_runtime`).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::coord::TreeCoord;

/// Player-owned tree state. Small on disk: a set of `i64`-sized coords,
/// a cursor coord, and the lot of the most recently bought node.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpgradeTreeState {
    /// Lot coordinates of every bought node.
    #[serde(default)]
    pub bought: HashSet<TreeCoord>,
    /// Cursor (focus) position on the canvas, in lot coordinates. The
    /// renderer pans the viewport so the cursor's lot sits near the
    /// center. Saved so reopening the tree lands where the player left it.
    #[serde(default)]
    pub cursor: TreeCoord,
    /// Lot of the most recently bought node. Drives the `[1] last bought`
    /// shortcut. `None` on fresh game / after prestige / after the only
    /// bought node was refunded.
    #[serde(default)]
    pub last_bought: Option<TreeCoord>,
}

impl Default for UpgradeTreeState {
    fn default() -> Self {
        Self {
            bought: HashSet::new(),
            cursor: TreeCoord::ORIGIN,
            last_bought: None,
        }
    }
}

impl UpgradeTreeState {
    pub fn is_owned(&self, c: TreeCoord) -> bool {
        self.bought.contains(&c)
    }

    pub fn owned_count(&self) -> usize {
        self.bought.len()
    }
}
