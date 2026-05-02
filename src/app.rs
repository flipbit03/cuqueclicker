//! Native runner. Two threads:
//!
//! - **main thread**: owns the terminal, renders snapshots of the game state,
//!   captures crossterm events, normalizes them into [`crate::input::InputEvent`]s,
//!   and feeds them to the platform-agnostic input router (which produces
//!   [`Action`]s + mutates [`UiState`]). Never blocks the sim — a slow render
//!   (SSH lag, terminal resize, stuck flush) is invisible to game logic.
//! - **sim thread**: owns the canonical [`GameState`], runs the 20Hz tick
//!   loop via [`crate::sim::sim_tick`], drains [`Action`]s, saves to disk
//!   through the [`Persistence`] impl, and publishes snapshots via
//!   [`ArcSwap`]. Tick cadence is driven by `mpsc::recv_timeout(until_next_tick)`,
//!   so it wakes exactly on tick deadlines or incoming actions — no busy spin,
//!   no lost ticks under arbitrary render delay.
//!
//! The cross-platform half (input router, `apply_action`, `sim_tick`) is in
//! `src/input.rs` and `src/sim.rs`. This file owns native-specific glue:
//! crossterm event translation, threading, save scheduling, and the
//! demo-recorder autopilot.

use anyhow::Result;
use arc_swap::ArcSwap;
use crossterm::event::{
    self, Event, KeyCode as CtKeyCode, KeyEventKind, KeyModifiers, MouseButton as CtMouseButton,
    MouseEvent as CtMouseEvent, MouseEventKind,
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

use crate::game::achievement::ACHIEVEMENTS;
use crate::game::fingerer;
use crate::game::fingerer::FINGERERS;
use crate::game::golden::{self, GoldenVariant};
use crate::game::state::{GameState, TICK_HZ};
use crate::game::upgrade::UPGRADES;
use crate::input::{
    self, InputContext, InputEvent, KeyCode as InKeyCode, Modifiers, MouseButton as InMouseButton,
    UiState, WheelDelta,
};
use crate::platform::Persistence;
use crate::sim::{self, Action, SimGeometry};
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
    persistence: Persistence,
}

impl App {
    pub fn new(
        state: GameState,
        debug: bool,
        demo_seconds: Option<u32>,
        persistence: Persistence,
    ) -> Self {
        Self {
            state,
            debug,
            demo_seconds,
            persistence,
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
            persistence,
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
                        persistence,
                    );
                })
                .expect("spawn sim thread")
        };

        let mut ui = UiState::new();
        // Single per-frame layout snapshot — the source of truth for every
        // clickable region. The input router consumes it via
        // `InputContext::from_layout`. Adding a new rect/list goes through
        // `DrawOutput` + the projection, never here.
        let mut layout: ui::DrawOutput = Default::default();
        // Reused per-event scratch buffer — `process_input_event` appends
        // produced actions here, then we drain it into the mpsc channel.
        let mut actions: Vec<Action> = Vec::with_capacity(4);

        while ui.running && !shutdown.load(Ordering::Relaxed) {
            // Drain any panel/quit requests from the demo driver before we draw,
            // so the frame we render reflects them.
            for msg in sim_msg_rx.try_iter() {
                match msg {
                    SimMsg::DemoSetMode(m) => ui.mode = m,
                    SimMsg::DemoQuit => ui.running = false,
                }
            }

            let current = snapshot.load_full();
            terminal.draw(|f| {
                layout = ui::draw(f, &current, ui.mode, ui.zoom_idx, debug, ui.last_mouse_pos);
            })?;

            // Hand fresh geometry to the sim. Ordering is preserved by mpsc,
            // so the sim always uses the most recently drawn layout.
            let _ = action_tx.send(Action::UpdateGeometry {
                biscuit: layout.biscuit_rect,
            });

            if event::poll(Duration::from_millis(INPUT_POLL_MS))? {
                let ctx = InputContext::from_layout(&layout, &current, debug);
                loop {
                    let ev = event::read()?;
                    if let Some(input_ev) = translate_crossterm(ev) {
                        actions.clear();
                        input::process_input_event(input_ev, &mut ui, &ctx, &mut actions);
                        for a in actions.drain(..) {
                            let _ = action_tx.send(a);
                        }
                    }
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
    persistence: Persistence,
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
            Ok(action) => sim::apply_action(&mut state, action, &mut geom),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Run every tick we're behind on. If we've fallen absurdly far
        // behind (laptop sleep), snap forward rather than grind through
        // thousands of catch-up ticks.
        let mut catchup = 0u32;
        while Instant::now() >= next_tick {
            sim::sim_tick(&mut state, &geom);
            // Native-only post-tick concerns: demo-recorder autopilot and
            // periodic save scheduling. Both are wrapped around the
            // platform-agnostic `sim::sim_tick` call above.
            if demo_seconds.is_some() {
                demo_driver_tick(
                    &mut state,
                    &geom,
                    demo_seconds,
                    &mut demo_ticks,
                    &mut demo_golden_spawns,
                    &sim_msg_tx,
                );
            } else {
                ticks_since_save += 1;
                if ticks_since_save >= SAVE_INTERVAL_TICKS {
                    ticks_since_save = 0;
                    let _ = persistence.save(&state);
                }
            }
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
        let _ = persistence.save(&state);
    }
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
            state.click((r.x + r.width / 2, r.y + r.height / 2), r);
        }
    }

    // Keep the screen busy with goldens: tighter cooldown than normal.
    // Force the next-eligible variant slot's cooldown to a short value
    // so the demo doesn't have to wait the natural per-variant rng.
    let any_golden = state.goldens.iter().any(|g| g.is_some());
    if !any_golden {
        for cd in state.golden_cooldowns.iter_mut() {
            if *cd == 0 {
                *cd = DEMO_GOLDEN_COOLDOWN;
            }
        }
    }

    // Force the variant on freshly-spawned goldens so the clip deterministically
    // cycles Buff → Frenzy → Lucky. Buff comes first so a viewer definitely
    // sees the purple powerup. After spawn we MOVE the slot to the
    // chosen variant's index so the rest of the game's per-variant
    // routing keeps working.
    for slot_idx in 0..state.goldens.len() {
        if let Some(g) = state.goldens[slot_idx].as_ref()
            && g.life_ticks == golden::GOLDEN_LIFE_TICKS
        {
            let target = match *demo_golden_spawns % 3 {
                0 => GoldenVariant::Buff,
                1 => GoldenVariant::Frenzy,
                _ => GoldenVariant::Lucky,
            };
            *demo_golden_spawns += 1;
            if slot_idx != target as usize {
                let mut g = state.goldens[slot_idx].take().unwrap();
                g.variant = target;
                state.goldens[target as usize] = Some(g);
            } else if let Some(g) = state.goldens[slot_idx].as_mut() {
                g.variant = target;
            }
            break;
        }
    }

    // Auto-catch whatever golden has been on screen long enough so the
    // marker is actually visible before disappearing. Iterate every
    // variant slot independently.
    for variant in GoldenVariant::ALL {
        let alive = state.goldens[variant as usize]
            .as_ref()
            .map(|g| g.life_ticks + 20 < golden::GOLDEN_LIFE_TICKS)
            .unwrap_or(false);
        if alive {
            state.catch_golden(variant);
        }
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

// --- crossterm → InputEvent translation --------------------------------

/// Normalize one crossterm event into our platform-neutral [`InputEvent`].
/// Returns `None` for events we drop entirely (focus, paste, resize, key
/// release/repeat, and unsupported mouse kinds).
fn translate_crossterm(ev: Event) -> Option<InputEvent> {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            let code = translate_key_code(k.code)?;
            Some(InputEvent::KeyPress {
                code,
                mods: translate_mods(k.modifiers),
            })
        }
        Event::Mouse(m) => translate_mouse(m),
        _ => None,
    }
}

fn translate_key_code(code: CtKeyCode) -> Option<InKeyCode> {
    match code {
        CtKeyCode::Char(c) => Some(InKeyCode::Char(c)),
        CtKeyCode::Esc => Some(InKeyCode::Esc),
        CtKeyCode::F(n) => Some(InKeyCode::F(n)),
        _ => None,
    }
}

fn translate_mods(mods: KeyModifiers) -> Modifiers {
    Modifiers {
        shift: mods.contains(KeyModifiers::SHIFT),
        alt: mods.contains(KeyModifiers::ALT),
        ctrl: mods.contains(KeyModifiers::CONTROL),
    }
}

/// Narrow crossterm's mouse button to the subset the game cares about.
/// Middle-click is intentionally dropped at the adapter boundary so it
/// stays a no-op (matching pre-refactor behavior, where `handle_event`
/// only matched `Down(Left)` / `Down(Right)`).
fn translate_mouse_button(button: CtMouseButton) -> Option<InMouseButton> {
    match button {
        CtMouseButton::Left => Some(InMouseButton::Left),
        CtMouseButton::Right => Some(InMouseButton::Right),
        CtMouseButton::Middle => None,
    }
}

fn translate_mouse(m: CtMouseEvent) -> Option<InputEvent> {
    let mods = translate_mods(m.modifiers);
    match m.kind {
        MouseEventKind::Down(button) => Some(InputEvent::MouseDown {
            col: m.column,
            row: m.row,
            button: translate_mouse_button(button)?,
            mods,
        }),
        MouseEventKind::ScrollUp => Some(InputEvent::Wheel {
            col: m.column,
            row: m.row,
            delta: WheelDelta::Up,
        }),
        MouseEventKind::ScrollDown => Some(InputEvent::Wheel {
            col: m.column,
            row: m.row,
            delta: WheelDelta::Down,
        }),
        // K5: track mouse position for hover highlighting. Crossterm only
        // emits Moved/Drag events when AnyMotion mouse mode is enabled
        // (it is). Drag-with-left collapses to a plain Moved on the
        // platform-neutral side; the renderer doesn't care.
        MouseEventKind::Moved | MouseEventKind::Drag(CtMouseButton::Left) => {
            Some(InputEvent::MouseMoved {
                col: m.column,
                row: m.row,
            })
        }
        _ => None,
    }
}

// --- Demo state --------------------------------------------------------

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
        // Default is a random 20-80s wait per slot; force all to 0 so the
        // first demo golden (a Buff, per the cycle in demo_driver_tick)
        // spawns on tick 1 — the purple powerup lands well within the
        // first few seconds of the clip.
        golden_cooldowns: [0; 3],
        best_fps: 50_000.0,
        ..GameState::default()
    };
    // Seed counts/flags BY CATALOG INDEX rather than by hardcoded id strings,
    // so a future rename/reorder/removal of a fingerer or upgrade can never
    // silently degrade the demo (the live id at that slot is always used).
    //
    // Per-tier owned counts ramp down 40→10 across the first 8 fingerers.
    // The per-type cap in `ui/hands.rs` is 40, so anything beyond that is
    // visually identical — 8 types owned = thick crust of hands.
    const DEMO_FINGERER_COUNTS: &[u32] = &[40, 40, 35, 30, 25, 20, 15, 10];
    for (idx, &count) in DEMO_FINGERER_COUNTS.iter().enumerate() {
        if let Some(f) = FINGERERS.get(idx)
            && count > 0
        {
            s.fingerers_state.entry(f.id.to_string()).or_default().count = count;
        }
    }
    // Take the first 10 upgrades from the catalog (deterministic regardless
    // of how UPGRADES is reordered) — gives a spread of click + per-tier
    // multipliers so the sidebar shows (xN) on several tiers.
    for u in UPGRADES.iter().take(10) {
        s.upgrades_earned.insert(u.id.to_string());
    }
    // First 6 achievements for visual variety in that panel.
    for a in ACHIEVEMENTS.iter().take(6) {
        s.achievements_earned.insert(a.id.to_string());
    }
    s
}
