//! Persistent state for the upgrade tree.
//!
//! Saved between sessions: `bought`, `pan`, `bookmarks`.
//! Reconstructed on load: the `TreeAggregate` cache (lives on `GameState`,
//! `#[serde(skip)]`, rebuilt by `migrate_runtime`).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::coord::TreeCoord;

/// Number of bookmarks the player can keep — one per digit key 1..0.
pub const N_BOOKMARKS: usize = 10;

/// Player-owned tree state. Small on disk: a set of `i64`-sized coords,
/// a single pan coord, and a fixed array of bookmark coords.
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
    /// User-defined bookmark targets. Default `(0, 0)` means "unset"; the
    /// UI may overwrite default slots with biome-of-fingerer heuristics
    /// at draw time. Each entry corresponds to digit key (`1`..`9`, `0`).
    #[serde(default = "default_bookmarks")]
    pub bookmarks: [TreeCoord; N_BOOKMARKS],
}

fn default_bookmarks() -> [TreeCoord; N_BOOKMARKS] {
    [TreeCoord::ORIGIN; N_BOOKMARKS]
}

impl Default for UpgradeTreeState {
    fn default() -> Self {
        Self {
            bought: HashSet::new(),
            cursor: TreeCoord::ORIGIN,
            bookmarks: default_bookmarks(),
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
