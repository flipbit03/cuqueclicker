//! Green Coin — rare powerup that attaches a permanent +10% AddPercent
//! modifier to a random *owned* fingerer when caught.
//!
//! Spawn semantics (see `crate::sim::maybe_spawn_golden`):
//!   - Each time a regular Golden Cuque spawns, a `goldens_since_green_coin`
//!     counter on `GameState` is incremented and a `rng < N * 0.01` roll
//!     decides whether to *also* spawn a Green Coin alongside.
//!   - The counter resets to 0 the moment a Green Coin appears, regardless
//!     of whether the player catches it or it expires.
//!   - Green Coin and a regular Golden can coexist on screen — they're
//!     independent entities with their own lifetimes.
//!
//! Catch effect (see `GameState::catch_green_coin`): pick a random fingerer
//! with `count > 0` and push a `Modifier {
//!   source: GreenCoin,
//!   effects: [AddPercent(0.10)],
//!   duration: Permanent,
//! }`. Multiple Green Coins on the same fingerer stack additively (+10%
//! per coin) — see `crate::game::modifier` stacking rules.
//!
//! Persistence: this struct is `#[serde(skip)]` on `GameState` (mirrors
//! `GoldenCuque`); the *counter* IS persisted so the pity timer survives
//! quit/restart.

use ratatui::layout::Rect;

use rand::RngExt;

/// Position is stored as a fraction of the biscuit rect ([0.0, 1.0] on each
/// axis), same convention as `GoldenCuque`. The renderer resolves these
/// fractions against the *current* biscuit rect every frame, so the marker
/// stays anchored on resize / zoom.
#[derive(Clone, Debug)]
pub struct GreenCoin {
    pub frac_x: f32,
    pub frac_y: f32,
    pub life_ticks: u32,
}

/// Lifetime parity with `GoldenCuque` — ~11s at 20Hz. Long enough to
/// notice and react, short enough that it's a meaningful catch.
pub const GREEN_COIN_LIFE_TICKS: u32 = 220;

const SPAWN_INSET_X: f32 = 0.08;
const SPAWN_INSET_Y: f32 = 0.10;

/// Permanent AddPercent the Green Coin attaches on catch. Tunable; bumping
/// it changes the long-term power curve significantly so treat with care.
pub const GREEN_COIN_ADD_PERCENT: f64 = 0.10;

/// Pick a random fractional position inside the biscuit, away from the
/// edges. `_biscuit` is taken so the signature documents intent (the spawn
/// area is "inside this rect"); the actual fractions are rect-independent.
pub fn spawn_in(_biscuit: Rect) -> GreenCoin {
    let mut r = rand::rng();
    GreenCoin {
        frac_x: r.random_range(SPAWN_INSET_X..=(1.0 - SPAWN_INSET_X)),
        frac_y: r.random_range(SPAWN_INSET_Y..=(1.0 - SPAWN_INSET_Y)),
        life_ticks: GREEN_COIN_LIFE_TICKS,
    }
}
