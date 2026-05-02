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

use crate::game::powerup::{self, Powerup, PowerupKind};
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
    /// Catch the on-screen powerup with the given `spawn_id`. The id is
    /// minted at spawn time on `GameState::next_spawn_id`; click hit-test
    /// and the `g` hotkey both reference instances by id, never by Vec
    /// index, so `swap_remove` on catch is safe even with multiple
    /// in-flight events between frames.
    CatchPowerup(u64),
    BuyFingerer {
        idx: usize,
        qty: BuyQty,
    },
    BuyUpgrade(usize),
    PrestigeReset,
    /// Latest render-computed biscuit geometry, so the sim can place
    /// powerups and auto-particles inside the current layout. Powerup
    /// rects live on the input/render side (only the click handler reads
    /// them).
    UpdateGeometry {
        biscuit: Rect,
    },
    /// Dev-only cheats (F-keys). Gated at the input router by `debug`;
    /// the sim trusts whatever arrives.
    DevAddCuques(f64),
    /// Force-spawn a powerup of the given kind. Pushes a fresh entry onto
    /// `state.powerups` — pressing the same F-key twice now produces two
    /// of the same kind on screen.
    DevForcePowerup(PowerupKind),
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
        Action::CatchPowerup(id) => {
            state.catch_powerup(id);
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
        Action::DevForcePowerup(kind) => {
            force_spawn_powerup(state, geom, kind);
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
    state.tick_powerups();
    maybe_spawn_powerups(state, geom);
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

/// Insets pull the spawn lottery away from the biscuit edges so the 5×3
/// marker has room to render without clipping into the border. Match the
/// pre-refactor inset values exactly — they were tuned against the same
/// marker geometry.
const SPAWN_INSET_X: f32 = 0.08;
const SPAWN_INSET_Y: f32 = 0.10;
/// Minimum biscuit-fractional distance between two on-screen powerups.
/// Slight overlap is acceptable visually; *exact* overlap loses the
/// parallelism feature (two markers in one cell read as one). 5% of the
/// biscuit dimension is empirically enough that the 5×3 markers stay
/// distinguishable.
const POWERUP_MIN_DIST: f32 = 0.05;
/// Best-effort retry budget for dispersion. Eight tries is plenty when the
/// Vec is short (the expected ~0.2 concurrent per kind average); on a
/// pile-up the fall-through to plain-random keeps the spawn happening
/// rather than skipping it.
const POWERUP_DISPERSION_TRIES: u32 = 8;

/// Pick a fractional position inside the biscuit, dispersed away from any
/// existing powerup in `existing`. Best-effort: up to
/// `POWERUP_DISPERSION_TRIES` retries, then accept a plain-random position
/// (acceptable to the issue spec — exact overlap is rare in practice).
fn pick_dispersed_frac(existing: &[Powerup]) -> (f32, f32) {
    let mut rng = rand::rng();
    for _ in 0..POWERUP_DISPERSION_TRIES {
        let fx = rng.random_range(SPAWN_INSET_X..=(1.0 - SPAWN_INSET_X));
        let fy = rng.random_range(SPAWN_INSET_Y..=(1.0 - SPAWN_INSET_Y));
        let too_close = existing.iter().any(|p| {
            let dx = p.frac_x - fx;
            let dy = p.frac_y - fy;
            (dx * dx + dy * dy).sqrt() < POWERUP_MIN_DIST
        });
        if !too_close {
            return (fx, fy);
        }
    }
    let fx = rng.random_range(SPAWN_INSET_X..=(1.0 - SPAWN_INSET_X));
    let fy = rng.random_range(SPAWN_INSET_Y..=(1.0 - SPAWN_INSET_Y));
    (fx, fy)
}

fn maybe_spawn_powerups(state: &mut GameState, geom: &SimGeometry) {
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    // Each kind runs on its own clock. Cooldown is reset to a fresh
    // exponential sample on every spawn (regardless of how many of the
    // same kind are already on screen — the parallelism is the whole
    // point of this refactor). `tick_powerups` already decremented the
    // cooldown this tick, so a `> 0` test here is correct.
    for kind in PowerupKind::ALL {
        let i = kind as usize;
        if state.powerup_cooldowns[i] > 0 {
            continue;
        }
        spawn_powerup(state, kind);
        state.powerup_cooldowns[i] = powerup::next_cooldown(kind);
    }
}

/// Push a fresh powerup of `kind` onto `state.powerups`. Position is
/// picked with the dispersion helper so back-to-back spawns don't land in
/// the exact same cell. Cooldown management is the caller's responsibility
/// (`maybe_spawn_powerups` resets the kind's clock; the dev cheats don't,
/// so pressing F8 twice in quick succession really does push two Lucky's).
fn spawn_powerup(state: &mut GameState, kind: PowerupKind) {
    let (frac_x, frac_y) = pick_dispersed_frac(&state.powerups);
    let spawn_id = state.mint_spawn_id();
    state.powerups.push(Powerup {
        kind,
        spawn_id,
        frac_x,
        frac_y,
        life_ticks: kind.lifetime_ticks(),
    });
}

/// Dev cheat: force-spawn a powerup of `kind`. Unlike `maybe_spawn_powerups`
/// this does NOT reset the cooldown, so it doesn't disturb the natural
/// rhythm — and it does NOT gate on slot occupancy (that's the whole
/// point: pressing F8 twice produces two Lucky's). The biscuit-size
/// guard mirrors the natural-spawn path so a tiny terminal can't drop a
/// marker into a 0-width rect.
fn force_spawn_powerup(state: &mut GameState, geom: &SimGeometry, kind: PowerupKind) {
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    spawn_powerup(state, kind);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::state::GameState;
    use ratatui::layout::Rect;

    fn geom_with_biscuit() -> SimGeometry {
        SimGeometry {
            biscuit: Rect::new(0, 0, 40, 20),
        }
    }

    #[test]
    fn force_spawn_pushes_to_vec_uncapped() {
        // Pressing the same F-key twice in a row produces two on-screen
        // powerups of that kind — no per-kind cap, no slot-occupancy
        // displacement. This is the headline feature of the refactor.
        let mut state = GameState::default();
        let geom = geom_with_biscuit();
        force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
        force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
        let lucky_count = state
            .powerups
            .iter()
            .filter(|p| p.kind == PowerupKind::Lucky)
            .count();
        assert_eq!(lucky_count, 2);
        // Distinct spawn ids — id reuse would defeat the per-instance
        // hit-test.
        let ids: Vec<u64> = state.powerups.iter().map(|p| p.spawn_id).collect();
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn force_spawn_mixes_kinds_freely() {
        // All four kinds can coexist; no slot ever forces a one-per-kind cap.
        let mut state = GameState::default();
        let geom = geom_with_biscuit();
        for kind in PowerupKind::ALL {
            force_spawn_powerup(&mut state, &geom, kind);
        }
        assert_eq!(state.powerups.len(), 4);
        for kind in PowerupKind::ALL {
            assert!(state.powerups.iter().any(|p| p.kind == kind));
        }
    }

    #[test]
    fn spawn_dispersion_avoids_exact_overlap() {
        // Two consecutive force-spawns on a fresh state must produce two
        // distinct positions. Dispersion is best-effort; we assert the
        // weaker but tractable property "distance between them is at
        // least the dispersion threshold" most of the time. With only one
        // existing entry the retry loop almost always finds a clean spot.
        let mut state = GameState::default();
        let geom = geom_with_biscuit();
        force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
        force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
        let a = &state.powerups[0];
        let b = &state.powerups[1];
        let dx = a.frac_x - b.frac_x;
        let dy = a.frac_y - b.frac_y;
        let dist = (dx * dx + dy * dy).sqrt();
        // Allow a generous floor: dispersion fall-through can produce a
        // single near-overlap, but not zero.
        assert!(dist > 0.0, "two spawns landed at the exact same point");
    }
}
