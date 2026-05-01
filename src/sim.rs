//! Platform-agnostic simulation core.
//!
//! Owns the [`Action`] / [`BuyQty`] types (the input router produces them;
//! [`apply_action`] is the only thing that interprets them) and the per-tick
//! `state.tick()` + ambient spawn helpers.
//!
//! What lives **outside** this module:
//! - the threaded sim loop on native (`app.rs::sim_loop`), which wraps
//!   [`sim_tick`] + [`apply_action`] with `mpsc::recv_timeout`, save
//!   scheduling via the [`Persistence`](crate::platform::Persistence) impl,
//!   and the demo-recorder driver.
//! - the requestAnimationFrame-driven loop on web (added when the wasm
//!   port lands), which calls the same [`sim_tick`] + [`apply_action`]
//!   single-threaded.
//!
//! The split is: this module is cross-platform; threading + I/O scheduling
//! around it isn't. See tracking issue #13 for rationale.

use rand::RngExt;
use ratatui::layout::Rect;

use crate::game::golden::{self, GoldenVariant};
use crate::game::green_coin;
use crate::game::state::{GameState, TICK_DT};

/// Buy quantity for a fingerer purchase action. Modifier-key meaning is
/// translated to this in the input router; sim only consumes the resolved
/// value so the modifier mapping can change without touching tick logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyQty {
    One,
    Ten,
    Max,
}

/// Commands the input router produces and the sim consumes. The sim is
/// the sole authority on [`GameState`] mutation — input handling translates
/// raw events (key/mouse/wheel) into these and feeds them through.
#[derive(Clone, Debug)]
pub enum Action {
    Click {
        col: u16,
        row: u16,
    },
    ClickCenter,
    CatchGolden,
    BuyFingerer {
        idx: usize,
        qty: BuyQty,
    },
    BuyUpgrade(usize),
    PrestigeReset,
    /// Latest render-computed biscuit geometry, so the sim can place goldens
    /// and auto-particles inside the current layout. The golden rect lives
    /// on the input/render side (only the click handler reads it).
    UpdateGeometry {
        biscuit: Rect,
    },
    /// Dev-only cheats (F-keys). Gated at the input router by `debug`;
    /// the sim trusts whatever arrives.
    DevAddCuques(f64),
    DevForceGolden(GoldenVariant),
    /// Force-spawn a Green Coin (F5 in dev). Bypasses the spawn pity
    /// counter and the golden-spawn-event tie-in entirely.
    DevSpawnGreenCoin,
    /// J10: a click that didn't hit anything actionable. Sim spawns a
    /// short-lived "·" misclick particle at the screen point so dead-zone
    /// clicks visibly register.
    Misclick {
        col: u16,
        row: u16,
    },
}

/// Geometry the sim needs to interpret screen-space events. Updated on
/// every render via [`Action::UpdateGeometry`].
#[derive(Clone, Copy, Default)]
pub struct SimGeometry {
    pub biscuit: Rect,
}

/// Apply one [`Action`] to the canonical [`GameState`]. Pure data: no I/O,
/// no time, no threading. Called from both the native sim thread (on
/// `mpsc::recv_timeout` returning Ok) and the web rAF loop.
pub fn apply_action(state: &mut GameState, action: Action, geom: &mut SimGeometry) {
    match action {
        Action::Click { col, row } => {
            let r = geom.biscuit;
            if r.width > 0
                && col >= r.x
                && col < r.x + r.width
                && row >= r.y
                && row < r.y + r.height
            {
                state.click((col, row), r);
            }
        }
        Action::ClickCenter => {
            let r = geom.biscuit;
            if r.width > 0 && r.height > 0 {
                state.click((r.x + r.width / 2, r.y + r.height / 2), r);
            }
            // Mark this tick as "saw a spacebar press." `tick()` reads the
            // flag, advances the held-streak counter, and clears it. A
            // single tap → 1 tick of streak → resets immediately. A held
            // key (terminal repeat) → streak climbs over time.
            state.space_pressed_this_tick = true;
        }
        Action::CatchGolden => {
            // Catch button is unified across the on-screen powerups — try
            // both and let whichever's present consume the press. Order
            // doesn't matter (independent slots) but Golden goes first to
            // match the legacy code path.
            state.catch_golden();
            state.catch_green_coin();
        }
        Action::BuyFingerer { idx, qty } => match qty {
            BuyQty::One => {
                state.buy(idx);
            }
            BuyQty::Ten => {
                state.buy_n(idx, 10);
            }
            BuyQty::Max => {
                state.buy_max(idx);
            }
        },
        Action::BuyUpgrade(idx) => {
            state.buy_upgrade(idx);
        }
        Action::PrestigeReset => {
            state.prestige_reset();
        }
        Action::UpdateGeometry { biscuit } => {
            *geom = SimGeometry { biscuit };
        }
        Action::DevAddCuques(n) => {
            state.dev_add_cuques(n);
        }
        Action::DevForceGolden(variant) => {
            force_spawn_golden(state, geom, variant);
        }
        Action::DevSpawnGreenCoin => {
            force_spawn_green_coin(state, geom);
        }
        Action::Misclick { col, row } => {
            state.spawn_misclick(col, row);
        }
    }
}

/// Run the platform-agnostic body of one sim tick: state updates + ambient
/// spawn helpers. Save scheduling and demo-driver autopilot are the
/// **caller's** concern (they live in `app.rs::sim_loop` on native).
pub fn sim_tick(state: &mut GameState, geom: &SimGeometry) {
    state.tick();
    state.tick_golden();
    state.tick_green_coin();
    maybe_spawn_golden(state, geom);
    maybe_spawn_auto_particle(state, geom);
    maybe_idle_clench(state);
}

fn maybe_idle_clench(state: &mut GameState) {
    if state.clench_ticks > 0 {
        return;
    }
    // ~1 per 45s average at 20Hz
    if rand::rng().random::<f64>() < 1.0 / 900.0 {
        state.trigger_clench();
    }
}

fn maybe_spawn_auto_particle(state: &mut GameState, geom: &SimGeometry) {
    let fps = state.fps();
    if fps <= 0.0 || geom.biscuit.width < 4 || geom.biscuit.height < 4 {
        return;
    }
    let target_rate = fps.sqrt().clamp(0.5, 8.0);
    let prob = target_rate * TICK_DT;
    let mut rng = rand::rng();
    if rng.random::<f64>() >= prob {
        return;
    }
    // Random anchor within the biscuit, with a small inset so the "+N" text
    // doesn't clip into the border.
    let frac_x = rng.random_range(0.05_f32..=0.95);
    let frac_y = rng.random_range(0.10_f32..=0.95);
    state.spawn_auto_particle(frac_x, frac_y);
}

fn maybe_spawn_golden(state: &mut GameState, geom: &SimGeometry) {
    if state.golden.is_some() || state.golden_cooldown > 0 {
        return;
    }
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    state.golden = Some(golden::spawn_in(geom.biscuit));

    // Green Coin spawn pity: each regular Golden spawn bumps the chance
    // by 1%. The roll fires whether or not a Green Coin slot is currently
    // free; if one's already on screen, the counter still increments but
    // the roll is *not* re-cast (the existing coin keeps its turn). The
    // counter resets the moment a Green Coin appears, regardless of
    // whether the player catches it or it expires.
    state.goldens_since_green_coin = state.goldens_since_green_coin.saturating_add(1);
    if state.green_coin.is_none() {
        let p = state.goldens_since_green_coin as f64 * 0.01;
        if rand::rng().random::<f64>() < p {
            state.green_coin = Some(green_coin::spawn_in(geom.biscuit));
            state.goldens_since_green_coin = 0;
        }
    }
}

fn force_spawn_golden(state: &mut GameState, geom: &SimGeometry, variant: GoldenVariant) {
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    let mut g = golden::spawn_in(geom.biscuit);
    g.variant = variant;
    state.golden = Some(g);
}

fn force_spawn_green_coin(state: &mut GameState, geom: &SimGeometry) {
    if state.green_coin.is_some() {
        return;
    }
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    state.green_coin = Some(green_coin::spawn_in(geom.biscuit));
}
