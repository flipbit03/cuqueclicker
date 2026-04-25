//! Main application. Runs on two threads:
//!
//! - **main thread**: owns the terminal, renders snapshots of the game state,
//!   captures crossterm events, and translates them into [`Action`] messages
//!   for the sim. Never blocks the sim — a slow render (SSH lag, terminal
//!   resize, stuck flush) is invisible to game logic.
//! - **sim thread**: owns the canonical [`GameState`], runs the 20Hz tick
//!   loop, drains [`Action`]s, saves to disk, and publishes snapshots via
//!   [`ArcSwap`]. Tick cadence is driven by `mpsc::recv_timeout(until_next_tick)`,
//!   so it wakes exactly on tick deadlines or incoming actions — no busy spin,
//!   no lost ticks under arbitrary render delay.
//!
//! State flow:
//!
//! ```text
//!   main                                    sim
//!   ─────                                   ─────
//!   terminal.draw(&snapshot)           ┌──  sim_loop:
//!     ├─ computes biscuit/golden rects │     mut state
//!     └─ returns visible upgrade/      │     loop:
//!        fingerer slot ordering        │       recv_timeout(next_tick)
//!                                      │         │
//!   event::poll + translate            │         ├─ Action → apply
//!     ├─ Click{col,row}      ──────────┼─> mpsc  ├─ Timeout → tick N times
//!     ├─ BuyFingerer{idx,qty}          │         └─ snapshot.store(state.clone())
//!     ├─ UpdateGeometry{rects}         │     on shutdown: save, exit
//!     ├─ ...                           │
//!     └─ DevForceGolden/AddCuques  ────┘
//!
//!   snapshot.load_full() ◄─── ArcSwap ◄────── sim publishes every iteration
//!   demo_rx.try_iter()   ◄─── mpsc    ◄────── demo driver panel swaps / quit
//! ```

use anyhow::Result;
use arc_swap::ArcSwap;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use rand::RngExt;
use ratatui::{Terminal, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::game::fingerer;
use crate::game::golden::{self, GoldenVariant};
use crate::game::state::{GameState, TICK_DT, TICK_HZ};
use crate::game::upgrade::UPGRADES;
use crate::save;
use crate::ui::{self, Mode};

const SAVE_INTERVAL_TICKS: u64 = TICK_HZ as u64 * 10;
// Golden cooldown override used only during demo recording so the viewer
// sees buffs/flashes frequently within the short clip.
const DEMO_GOLDEN_COOLDOWN: u32 = 40;
// Input poll timeout on the main thread — sets render responsiveness
// when there's no input. At 16ms we redraw ~60Hz; snapshots advance
// at 20Hz, so visual updates land within one frame of their tick.
const INPUT_POLL_MS: u64 = 16;
// How far behind we let the sim fall before we give up on catching up
// (post-sleep, suspended SSH, etc.) and resync to wall clock. 20 ticks
// = 1s at 20Hz.
const MAX_TICK_CATCHUP: u32 = 20;

/// Buy quantity for a fingerer purchase action.
#[derive(Clone, Copy)]
enum BuyQty {
    One,
    Ten,
    Max,
}

/// Commands the main (render) thread sends to the sim thread. The sim is
/// the sole authority on [`GameState`] mutation — main's event handler only
/// translates input into these messages.
enum Action {
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
    /// and auto-particles inside the current layout. The golden rect stays
    /// on the main thread (only the click handler reads it).
    UpdateGeometry {
        biscuit: Rect,
    },
    /// Dev-only cheats (F-keys). Gated at the main-thread handler by
    /// `debug`; the sim trusts whatever arrives.
    DevAddCuques(f64),
    DevForceGolden(GoldenVariant),
}

/// Messages the sim thread sends back to main. Used exclusively by the
/// demo driver to steer the on-camera panel cycle and to request a clean
/// shutdown when the recording duration elapses.
enum SimMsg {
    DemoSetMode(Mode),
    DemoQuit,
}

pub struct App {
    state: GameState,
    debug: bool,
    demo_seconds: Option<u32>,
}

impl App {
    pub fn new(state: GameState, debug: bool, demo_seconds: Option<u32>) -> Self {
        Self {
            state,
            debug,
            demo_seconds,
        }
    }

    pub fn run<B: Backend>(self, terminal: &mut Terminal<B>) -> Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        let App {
            state,
            debug,
            demo_seconds,
        } = self;

        let snapshot = Arc::new(ArcSwap::from_pointee(state.clone()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (action_tx, action_rx) = mpsc::channel::<Action>();
        let (sim_msg_tx, sim_msg_rx) = mpsc::channel::<SimMsg>();

        let sim_handle = {
            let snapshot = snapshot.clone();
            let shutdown = shutdown.clone();
            thread::Builder::new()
                .name("cuque-sim".into())
                .spawn(move || {
                    sim_loop(
                        state,
                        snapshot,
                        action_rx,
                        sim_msg_tx,
                        shutdown,
                        demo_seconds,
                    );
                })
                .expect("spawn sim thread")
        };

        // UI-only state that lives on the main thread. None of this is
        // persisted or reachable from sim.
        let mut mode = Mode::Game;
        let mut zoom_idx: usize = 0;
        let mut running = true;
        let mut visible_upgrades: Vec<usize> = Vec::new();
        let mut visible_fingerers: Vec<usize> = Vec::new();
        let mut biscuit_rect = Rect::default();
        let mut golden_rect = Rect::default();

        while running && !shutdown.load(Ordering::Relaxed) {
            // Drain any panel/quit requests from the demo driver before we draw,
            // so the frame we render reflects them.
            for msg in sim_msg_rx.try_iter() {
                match msg {
                    SimMsg::DemoSetMode(m) => mode = m,
                    SimMsg::DemoQuit => running = false,
                }
            }

            let current = snapshot.load_full();
            terminal.draw(|f| {
                let out = ui::draw(f, &current, mode, zoom_idx, debug);
                biscuit_rect = out.biscuit_rect;
                golden_rect = out.golden_rect;
                visible_upgrades = out.visible_upgrades;
                visible_fingerers = out.visible_fingerers;
            })?;

            // Hand fresh geometry to the sim. Ordering is preserved by mpsc,
            // so the sim always uses the most recently drawn layout.
            let _ = action_tx.send(Action::UpdateGeometry {
                biscuit: biscuit_rect,
            });

            if event::poll(Duration::from_millis(INPUT_POLL_MS))? {
                loop {
                    let ev = event::read()?;
                    handle_event(
                        ev,
                        &action_tx,
                        &mut mode,
                        &mut zoom_idx,
                        &mut running,
                        &visible_fingerers,
                        &visible_upgrades,
                        debug,
                        &current,
                        biscuit_rect,
                        golden_rect,
                    );
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }

        // Tell sim to wind down, wait for it to flush state to disk.
        shutdown.store(true, Ordering::Relaxed);
        drop(action_tx);
        sim_handle.join().expect("sim thread panicked");
        Ok(())
    }
}

// --- Sim thread ---------------------------------------------------------

fn sim_loop(
    mut state: GameState,
    snapshot: Arc<ArcSwap<GameState>>,
    actions: mpsc::Receiver<Action>,
    sim_msg_tx: mpsc::Sender<SimMsg>,
    shutdown: Arc<AtomicBool>,
    demo_seconds: Option<u32>,
) {
    let tick_dt = Duration::from_micros(1_000_000 / TICK_HZ as u64);
    let mut next_tick = Instant::now() + tick_dt;
    let mut ticks_since_save: u64 = 0;
    let mut demo_ticks: u64 = 0;
    let mut demo_golden_spawns: u32 = 0;
    let mut geom = SimGeometry::default();

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Block until the next tick deadline OR an action arrives — whichever
        // comes first. No busy spin, no tick drift.
        let timeout = next_tick.saturating_duration_since(Instant::now());
        match actions.recv_timeout(timeout) {
            Ok(action) => apply_action(&mut state, action, &mut geom),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Run every tick we're behind on. If we've fallen absurdly far
        // behind (laptop sleep), snap forward rather than grind through
        // thousands of catch-up ticks.
        let mut catchup = 0u32;
        while Instant::now() >= next_tick {
            sim_tick(
                &mut state,
                &geom,
                demo_seconds,
                &mut demo_ticks,
                &mut demo_golden_spawns,
                &mut ticks_since_save,
                &sim_msg_tx,
            );
            next_tick += tick_dt;
            catchup += 1;
            if catchup >= MAX_TICK_CATCHUP && Instant::now() > next_tick {
                next_tick = Instant::now() + tick_dt;
                break;
            }
        }

        // Publish the new snapshot. Cheap clone (few small HashMaps + a
        // short Vec of particles); Arc swap is lock-free.
        snapshot.store(Arc::new(state.clone()));
    }

    // Graceful shutdown: one last achievement sweep and a final save. Demo
    // mode runs on ephemeral state and never touches disk.
    if demo_seconds.is_none() {
        state.tick_achievements();
        let _ = save::save(&state);
    }
}

#[derive(Clone, Copy, Default)]
struct SimGeometry {
    biscuit: Rect,
}

fn apply_action(state: &mut GameState, action: Action, geom: &mut SimGeometry) {
    match action {
        Action::Click { col, row } => {
            let r = geom.biscuit;
            if r.width > 0
                && col >= r.x
                && col < r.x + r.width
                && row >= r.y
                && row < r.y + r.height
            {
                state.click((col, row));
            }
        }
        Action::ClickCenter => {
            let r = geom.biscuit;
            if r.width > 0 && r.height > 0 {
                state.click((r.x + r.width / 2, r.y + r.height / 2));
            }
        }
        Action::CatchGolden => {
            state.catch_golden();
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
    }
}

fn sim_tick(
    state: &mut GameState,
    geom: &SimGeometry,
    demo_seconds: Option<u32>,
    demo_ticks: &mut u64,
    demo_golden_spawns: &mut u32,
    ticks_since_save: &mut u64,
    sim_msg_tx: &mpsc::Sender<SimMsg>,
) {
    state.tick();
    state.tick_golden();
    maybe_spawn_golden(state, geom);
    maybe_spawn_auto_particle(state, geom);
    maybe_idle_clench(state);
    if demo_seconds.is_some() {
        demo_driver_tick(
            state,
            geom,
            demo_seconds,
            demo_ticks,
            demo_golden_spawns,
            sim_msg_tx,
        );
        return;
    }
    *ticks_since_save += 1;
    if *ticks_since_save >= SAVE_INTERVAL_TICKS {
        *ticks_since_save = 0;
        let _ = save::save(state);
    }
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
    let col =
        rng.random_range(geom.biscuit.x + 1..geom.biscuit.x + geom.biscuit.width.saturating_sub(2));
    let row = rng
        .random_range(geom.biscuit.y + 1..geom.biscuit.y + geom.biscuit.height.saturating_sub(1));
    state.spawn_auto_particle(col, row);
}

fn maybe_spawn_golden(state: &mut GameState, geom: &SimGeometry) {
    if state.golden.is_some() || state.golden_cooldown > 0 {
        return;
    }
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    let col_range = (
        geom.biscuit.x + 2,
        geom.biscuit.x + geom.biscuit.width.saturating_sub(8),
    );
    let row_range = (
        geom.biscuit.y + 1,
        geom.biscuit.y + geom.biscuit.height.saturating_sub(4),
    );
    state.golden = Some(golden::spawn_in(col_range, row_range));
}

fn force_spawn_golden(state: &mut GameState, geom: &SimGeometry, variant: GoldenVariant) {
    if geom.biscuit.width < 8 || geom.biscuit.height < 5 {
        return;
    }
    let col_range = (
        geom.biscuit.x + 2,
        geom.biscuit.x + geom.biscuit.width.saturating_sub(8),
    );
    let row_range = (
        geom.biscuit.y + 1,
        geom.biscuit.y + geom.biscuit.height.saturating_sub(4),
    );
    let mut g = golden::spawn_in(col_range, row_range);
    g.variant = variant;
    state.golden = Some(g);
}

/// Demo-mode autopilot, running on the sim thread. Mutates state directly
/// for clicks/buys; sends `SimMsg` back to main for panel swaps and the
/// final quit signal since `mode` lives on the render thread.
fn demo_driver_tick(
    state: &mut GameState,
    geom: &SimGeometry,
    demo_seconds: Option<u32>,
    demo_ticks: &mut u64,
    demo_golden_spawns: &mut u32,
    sim_msg_tx: &mpsc::Sender<SimMsg>,
) {
    *demo_ticks += 1;
    let t = *demo_ticks;
    let mut rng = rand::rng();

    // ~1.5 clicks/s. Real play is faster; on camera it'd smear.
    if t.is_multiple_of(13) {
        let r = geom.biscuit;
        if r.width > 0 && r.height > 0 {
            state.click((r.x + r.width / 2, r.y + r.height / 2));
        }
    }

    // Keep the screen busy with goldens: tighter cooldown than normal.
    if state.golden.is_none() && state.golden_cooldown == 0 {
        state.golden_cooldown = DEMO_GOLDEN_COOLDOWN;
    }

    // Force the variant on freshly-spawned goldens so the clip deterministically
    // cycles Buff → Frenzy → Lucky. Buff comes first so a viewer definitely
    // sees the purple powerup.
    if let Some(g) = &mut state.golden
        && g.life_ticks == golden::GOLDEN_LIFE_TICKS
    {
        g.variant = match *demo_golden_spawns % 3 {
            0 => GoldenVariant::Buff,
            1 => GoldenVariant::Frenzy,
            _ => GoldenVariant::Lucky,
        };
        *demo_golden_spawns += 1;
    }

    // Auto-catch whatever golden is on screen after a brief "reaction
    // time" so the marker is actually visible before disappearing.
    if let Some(g) = &state.golden
        && g.life_ticks + 20 < golden::GOLDEN_LIFE_TICKS
    {
        state.catch_golden();
    }

    // Every ~4s, buy 1-2 of a random affordable fingerer.
    if t.is_multiple_of(80) {
        let candidates: Vec<usize> = (0..fingerer::count())
            .filter(|&i| state.can_buy(i))
            .collect();
        if !candidates.is_empty() {
            let idx = candidates[rng.random_range(0..candidates.len())];
            state.buy_n(idx, rng.random_range(1..=2));
        }
    }

    // Every ~8s, buy the cheapest available upgrade.
    if t.is_multiple_of(160) {
        let available = crate::game::upgrade::available_ids(state);
        if let Some(&u_idx) = available
            .iter()
            .min_by(|&&a, &&b| UPGRADES[a].cost.partial_cmp(&UPGRADES[b].cost).unwrap())
        {
            state.buy_upgrade(u_idx);
        }
    }

    // Every ~15s, show a non-game panel for ~2s.
    let phase = t % 300;
    let panel_swap = if phase == 100 {
        Some(Mode::Stats)
    } else if phase == 140 {
        Some(Mode::Achievements)
    } else if phase == 180 {
        Some(Mode::Upgrades)
    } else if phase == 220 {
        Some(Mode::Game)
    } else {
        None
    };
    if let Some(m) = panel_swap {
        let _ = sim_msg_tx.send(SimMsg::DemoSetMode(m));
    }

    // Deadline: auto-quit when the user's requested duration elapses so the
    // asciinema recording sees a clean exit.
    if let Some(secs) = demo_seconds
        && t >= (secs as u64) * (TICK_HZ as u64)
    {
        let _ = sim_msg_tx.send(SimMsg::DemoQuit);
    }
}

// --- Main thread: event → action translation ---------------------------

#[allow(clippy::too_many_arguments)]
fn handle_event(
    ev: Event,
    tx: &mpsc::Sender<Action>,
    mode: &mut Mode,
    zoom_idx: &mut usize,
    running: &mut bool,
    visible_fingerers: &[usize],
    visible_upgrades: &[usize],
    debug: bool,
    current: &GameState,
    biscuit_rect: Rect,
    golden_rect: Rect,
) {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => handle_key(
            k,
            tx,
            mode,
            zoom_idx,
            running,
            visible_fingerers,
            visible_upgrades,
            debug,
            current,
        ),
        Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
            handle_click(m.column, m.row, tx, *mode, biscuit_rect, golden_rect);
        }
        Event::Mouse(m) if m.kind == MouseEventKind::ScrollUp => {
            let _ = m;
            *zoom_idx = zoom_idx.saturating_sub(1);
        }
        Event::Mouse(m) if m.kind == MouseEventKind::ScrollDown => {
            let _ = m;
            *zoom_idx = (*zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
        }
        _ => {}
    }
}

fn handle_click(
    col: u16,
    row: u16,
    tx: &mpsc::Sender<Action>,
    mode: Mode,
    biscuit: Rect,
    golden: Rect,
) {
    if mode != Mode::Game {
        return;
    }
    if golden.width > 0
        && col >= golden.x
        && col < golden.x + golden.width
        && row >= golden.y
        && row < golden.y + golden.height
    {
        let _ = tx.send(Action::CatchGolden);
        return;
    }
    if col >= biscuit.x
        && col < biscuit.x + biscuit.width
        && row >= biscuit.y
        && row < biscuit.y + biscuit.height
    {
        let _ = tx.send(Action::Click { col, row });
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    k: KeyEvent,
    tx: &mpsc::Sender<Action>,
    mode: &mut Mode,
    zoom_idx: &mut usize,
    running: &mut bool,
    visible_fingerers: &[usize],
    visible_upgrades: &[usize],
    debug: bool,
    current: &GameState,
) {
    let code = k.code;
    let mods = k.modifiers;
    match code {
        KeyCode::Char('q') => *running = false,
        KeyCode::Esc => match *mode {
            Mode::Game => *running = false,
            _ => *mode = Mode::Game,
        },
        KeyCode::Char('s') | KeyCode::Char('S') => {
            *mode = if matches!(*mode, Mode::Stats) {
                Mode::Game
            } else {
                Mode::Stats
            };
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            *mode = if matches!(*mode, Mode::Achievements) {
                Mode::Game
            } else {
                Mode::Achievements
            };
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            *mode = if matches!(*mode, Mode::Upgrades) {
                Mode::Game
            } else {
                Mode::Upgrades
            };
        }
        // [g] catches any Golden Cuque variant. Guard on the latest snapshot
        // to avoid sending a noop CatchGolden when nothing is on screen —
        // race with sim is harmless (worst case: catch fires against empty
        // state, no-ops inside state.catch_golden()).
        KeyCode::Char('g') | KeyCode::Char('G') if current.golden.is_some() => {
            let _ = tx.send(Action::CatchGolden);
        }
        // Debug/testing: gated on the main thread by `debug`. See
        // src/ui/debug_pane.rs for the advertised key list.
        KeyCode::F(1) if debug => {
            let _ = tx.send(Action::DevForceGolden(GoldenVariant::Lucky));
        }
        KeyCode::F(2) if debug => {
            let _ = tx.send(Action::DevForceGolden(GoldenVariant::Frenzy));
        }
        KeyCode::F(3) if debug => {
            let _ = tx.send(Action::DevForceGolden(GoldenVariant::Buff));
        }
        KeyCode::F(4) if debug => {
            let _ = tx.send(Action::DevAddCuques(1_000_000.0));
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            *mode = if matches!(*mode, Mode::Prestige) {
                Mode::Game
            } else {
                Mode::Prestige
            };
        }
        // Prestige confirm: check the snapshot for available prestige before
        // firing. Optimistically close the panel — if the sim rejects the
        // reset (raced against a simultaneous lifetime-cuque drop, which
        // only happens during prestige itself) nothing bad happens.
        KeyCode::Char('r') | KeyCode::Char('R')
            if *mode == Mode::Prestige && current.prestige_available() > 0 =>
        {
            let _ = tx.send(Action::PrestigeReset);
            *mode = Mode::Game;
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            *zoom_idx = zoom_idx.saturating_sub(1);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            *zoom_idx = (*zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            if *mode == Mode::Prestige {
                if current.prestige_available() > 0 {
                    let _ = tx.send(Action::PrestigeReset);
                    *mode = Mode::Game;
                }
            } else if *mode == Mode::Game {
                let _ = tx.send(Action::ClickCenter);
            }
        }
        KeyCode::Char(c) => {
            if let Some((slot, shifted_sym)) = digit_slot(c) {
                let buy_10 = shifted_sym || mods.contains(KeyModifiers::SHIFT);
                let buy_max =
                    mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::CONTROL);
                match *mode {
                    Mode::Game => {
                        if let Some(&fid) = visible_fingerers.get(slot) {
                            let qty = if buy_max {
                                BuyQty::Max
                            } else if buy_10 {
                                BuyQty::Ten
                            } else {
                                BuyQty::One
                            };
                            let _ = tx.send(Action::BuyFingerer { idx: fid, qty });
                        }
                    }
                    Mode::Upgrades => {
                        if let Some(&u_idx) = visible_upgrades.get(slot) {
                            let _ = tx.send(Action::BuyUpgrade(u_idx));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

// --- Demo state + digit-slot map ---------------------------------------

/// Rich starting state for `--demo-for-recording`. Tuned so a viewer
/// sees **numbers moving fast** (high FPS → counter spins visibly) and
/// **many rings of hands** around the biscuit (heavy owned counts).
/// Starting cuques is intentionally modest relative to FPS so the HUD
/// counter grows by a visible fraction every frame instead of looking
/// frozen.
pub fn build_demo_state() -> GameState {
    let mut s = GameState {
        // Low relative to FPS so the counter clearly grows throughout
        // the clip, and cheap enough to buy early tiers often.
        cuques: 500_000.0,
        lifetime_cuques: 500_000_000.0, // unlocks all tiers via the visibility gate
        total_clicks: 500,
        total_play_ticks: 3600 * TICK_HZ as u64, // pretend we've been at this an hour
        prestige: 3,
        golden_caught: 7,
        // Default is a random 20-80s wait; force 0 so the first demo golden
        // (a Buff, per the cycle in demo_driver_tick) spawns on tick 1 —
        // the purple powerup lands well within the first few seconds of the clip.
        golden_cooldown: 0,
        best_fps: 50_000.0,
        ..GameState::default()
    };
    // 3+ rings of hands around the biscuit. Per-type cap in `ui/hands.rs`
    // is 40, so anything over 40 is visually identical. 8 types owned =
    // thick crust of hands around the ass.
    for (id, count) in [
        ("index_finger", 40),
        ("whole_hand", 40),
        ("latex_glove", 35),
        ("greek_kiss", 30),
        ("robotic_finger", 25),
        ("tentacle", 20),
        ("finger_vortex", 15),
        ("dimensional_hole", 10),
    ] {
        s.fingerers_owned.insert(id.into(), count);
    }
    // A spread of upgrades so the sidebar shows (xN) multipliers on several tiers.
    for id in [
        "click_mult_1",
        "click_mult_2",
        "index_finger_mult_1",
        "index_finger_mult_2",
        "whole_hand_mult_1",
        "whole_hand_mult_2",
        "latex_glove_mult_1",
        "latex_glove_mult_2",
        "robotic_finger_mult_1",
        "all_fingerers_boost",
    ] {
        s.upgrades_earned.insert(id.into());
    }
    // A handful of achievements for visual variety in that panel.
    for id in [
        "first_finger",
        "warming_up",
        "seasoned_fingerer",
        "automation",
        "factory_of_fingers",
        "golden_touch",
    ] {
        s.achievements_earned.insert(id.into());
    }
    s
}

fn digit_slot(c: char) -> Option<(usize, bool)> {
    match c {
        '1' => Some((0, false)),
        '2' => Some((1, false)),
        '3' => Some((2, false)),
        '4' => Some((3, false)),
        '5' => Some((4, false)),
        '6' => Some((5, false)),
        '7' => Some((6, false)),
        '8' => Some((7, false)),
        '9' => Some((8, false)),
        '0' => Some((9, false)),
        '!' => Some((0, true)),
        '@' => Some((1, true)),
        '#' => Some((2, true)),
        '$' => Some((3, true)),
        '%' => Some((4, true)),
        '^' => Some((5, true)),
        '&' => Some((6, true)),
        '*' => Some((7, true)),
        '(' => Some((8, true)),
        ')' => Some((9, true)),
        _ => None,
    }
}
