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
/// Minimum cell-space distance between two on-screen powerup centers,
/// measured in biscuit-cell units (NOT fractional units). The 5×3 marker
/// is 5 cells wide and 3 tall, so a 4-cell minimum keeps two markers
/// from sharing any of their interior cells while still allowing tight
/// neighbors that read as distinct.
const POWERUP_MIN_CELL_DIST: f32 = 4.0;
/// Approximate biscuit cell aspect ratio (width / height of a terminal
/// cell). Most monospace fonts render cells ~2× taller than wide; the
/// FULL biscuit's bounding box is ~60×30 (cell ratio 2:1), MEDIUM is
/// 40×18 (~2.2:1), TINY is 16×8 (2:1). Using 2.0 here keeps the
/// dispersion check working in cell space, so the same fractional gap
/// in `frac_y` covers more visual cells than in `frac_x` — without this
/// correction, two markers separated only vertically would read as
/// overlapping while passing the dispersion filter.
const BISCUIT_CELL_ASPECT: f32 = 2.0;
/// Best-effort retry budget for dispersion. Eight tries is plenty when the
/// Vec is short (the expected ~0.2 concurrent per kind average); on a
/// pile-up the fall-through to plain-random keeps the spawn happening
/// rather than skipping it.
const POWERUP_DISPERSION_TRIES: u32 = 8;

/// Pick a fractional position inside the biscuit, dispersed away from any
/// existing powerup in `existing`. Best-effort: up to
/// `POWERUP_DISPERSION_TRIES` retries, then accept a plain-random position
/// (acceptable to the issue spec — exact overlap is rare in practice).
///
/// `biscuit_cells` is `(width, height)` of the live biscuit rect. The
/// dispersion check works in CELL SPACE — `dx_cells² + dy_cells² ≥
/// POWERUP_MIN_CELL_DIST²` — because the biscuit is roughly 2:1 in cell
/// aspect (terminal cells are ~2× tall as wide), and a pure-fractional
/// distance would over-allow vertical overlap.
fn pick_dispersed_frac(existing: &[Powerup], biscuit_cells: (u16, u16)) -> (f32, f32) {
    let (bw, bh) = biscuit_cells;
    let bw = bw.max(1) as f32;
    let bh = bh.max(1) as f32;
    let min_sq = POWERUP_MIN_CELL_DIST * POWERUP_MIN_CELL_DIST;
    let mut rng = rand::rng();
    for _ in 0..POWERUP_DISPERSION_TRIES {
        let fx = rng.random_range(SPAWN_INSET_X..=(1.0 - SPAWN_INSET_X));
        let fy = rng.random_range(SPAWN_INSET_Y..=(1.0 - SPAWN_INSET_Y));
        let too_close = existing.iter().any(|p| {
            // Convert fractional deltas to cell-space deltas. Y is
            // multiplied by BISCUIT_CELL_ASPECT to compensate for the
            // tall terminal cell — one row visually equals ~2 cols.
            let dx_cells = (p.frac_x - fx) * bw;
            let dy_cells = (p.frac_y - fy) * bh * BISCUIT_CELL_ASPECT;
            dx_cells * dx_cells + dy_cells * dy_cells < min_sq
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
    let cells = (geom.biscuit.width, geom.biscuit.height);
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
        spawn_powerup(state, kind, cells);
        state.powerup_cooldowns[i] = powerup::next_cooldown(kind);
    }
}

/// Push a fresh powerup of `kind` onto `state.powerups`. Position is
/// picked with the dispersion helper so back-to-back spawns don't land in
/// the exact same cell. Cooldown management is the caller's responsibility
/// (`maybe_spawn_powerups` resets the kind's clock; the dev cheats don't —
/// pressing F8 twice in quick succession really does push two Lucky's, AND
/// the natural Lucky cooldown keeps ticking down independently, so a dev
/// spawn followed shortly by a natural spawn is expected and intentional).
fn spawn_powerup(state: &mut GameState, kind: PowerupKind, biscuit_cells: (u16, u16)) {
    // Defensive: every spawn site uses the kind's full lifetime. If a
    // future caller passes a Powerup with `life_ticks: 0` directly,
    // `tick_powerups` would still drop it on the next tick — but the
    // marker would briefly render at near-zero life, hitting the
    // alarm-mode shimmer immediately. Catch that misuse here.
    let life_ticks = kind.lifetime_ticks();
    debug_assert!(life_ticks > 0, "PowerupKind::lifetime_ticks must be > 0");
    let (frac_x, frac_y) = pick_dispersed_frac(&state.powerups, biscuit_cells);
    let spawn_id = state.mint_spawn_id();
    state.powerups.push(Powerup {
        kind,
        spawn_id,
        frac_x,
        frac_y,
        life_ticks,
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
    spawn_powerup(state, kind, (geom.biscuit.width, geom.biscuit.height));
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

    #[test]
    fn spawn_dispersion_keeps_cell_distance_in_typical_layout() {
        // Statistical: across 1000 fresh-state pair spawns on a normal
        // 60×30 biscuit, the cell-space distance between the two
        // markers should clear `POWERUP_MIN_CELL_DIST` the vast
        // majority of the time (only the fall-through path violates,
        // and that fires once per ~8 retries × dense neighborhood,
        // which is rare for a single existing point on a 50-cell-wide
        // free area). Asserting a 90%+ pass rate is generous.
        let mut clear = 0;
        let trials = 1000;
        let geom = SimGeometry {
            biscuit: Rect::new(0, 0, 60, 30),
        };
        for _ in 0..trials {
            let mut state = GameState::default();
            force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
            force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
            let a = &state.powerups[0];
            let b = &state.powerups[1];
            let dx_cells = (a.frac_x - b.frac_x) * geom.biscuit.width as f32;
            let dy_cells = (a.frac_y - b.frac_y) * geom.biscuit.height as f32 * BISCUIT_CELL_ASPECT;
            let cell_dist = (dx_cells * dx_cells + dy_cells * dy_cells).sqrt();
            if cell_dist >= POWERUP_MIN_CELL_DIST {
                clear += 1;
            }
        }
        let ratio = clear as f32 / trials as f32;
        assert!(
            ratio > 0.90,
            "expected ≥90% of pair spawns to clear cell distance; got {clear}/{trials} = {ratio}"
        );
    }

    #[test]
    fn spawn_dispersion_handles_tiny_biscuit_without_panic() {
        // Edge case: at TINY zoom (16×8) the biscuit is barely large
        // enough for the marker. The dispersion helper must not divide
        // by zero or panic, even when the size guard in
        // `maybe_spawn_powerups` would normally reject.
        let mut state = GameState::default();
        // Just above the size guard so force_spawn_powerup goes through.
        let geom = SimGeometry {
            biscuit: Rect::new(0, 0, 16, 8),
        };
        force_spawn_powerup(&mut state, &geom, PowerupKind::Lucky);
        force_spawn_powerup(&mut state, &geom, PowerupKind::Frenzy);
        assert_eq!(state.powerups.len(), 2);
    }
}
