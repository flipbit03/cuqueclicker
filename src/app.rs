use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use rand::RngExt;
use ratatui::{Terminal, prelude::*};
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

pub struct App {
    state: GameState,
    running: bool,
    mode: Mode,
    biscuit_rect: Rect,
    golden_rect: Rect,
    visible_upgrades: Vec<usize>,
    visible_fingerers: Vec<usize>,
    ticks_since_save: u64,
    zoom_idx: usize,
    debug: bool,
    /// Some(n) means we're in demo-for-recording mode and should self-quit
    /// after `n` seconds. None for normal gameplay.
    demo_seconds: Option<u32>,
    demo_ticks: u64,
    demo_golden_spawns: u32,
}

impl App {
    pub fn new(state: GameState, debug: bool, demo_seconds: Option<u32>) -> Self {
        Self {
            state,
            running: true,
            mode: Mode::Game,
            biscuit_rect: Rect::default(),
            golden_rect: Rect::default(),
            visible_upgrades: Vec::new(),
            visible_fingerers: Vec::new(),
            ticks_since_save: 0,
            zoom_idx: 0,
            debug,
            demo_seconds,
            demo_ticks: 0,
            demo_golden_spawns: 0,
        }
    }

    pub fn run<B: Backend>(mut self, terminal: &mut Terminal<B>) -> Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        let tick_dt = Duration::from_micros(1_000_000 / TICK_HZ as u64);
        let mut next_tick = Instant::now() + tick_dt;

        while self.running {
            terminal.draw(|f| {
                let out = ui::draw(f, &self.state, self.mode, self.zoom_idx, self.debug);
                self.biscuit_rect = out.biscuit_rect;
                self.golden_rect = out.golden_rect;
                self.visible_upgrades = out.visible_upgrades;
                self.visible_fingerers = out.visible_fingerers;
            })?;

            let now = Instant::now();
            let timeout = next_tick.saturating_duration_since(now);
            if event::poll(timeout)? {
                loop {
                    self.dispatch(event::read()?);
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            if Instant::now() >= next_tick {
                self.game_tick();
                next_tick += tick_dt;
                if Instant::now() > next_tick + tick_dt * 10 {
                    next_tick = Instant::now() + tick_dt;
                }
            }
        }

        if self.demo_seconds.is_none() {
            self.state.tick_achievements();
            let _ = save::save(&self.state);
        }
        Ok(())
    }

    fn game_tick(&mut self) {
        self.state.tick();
        self.state.tick_golden();
        self.maybe_spawn_golden();
        self.maybe_spawn_auto_particle();
        self.maybe_idle_clench();
        if self.demo_seconds.is_some() {
            self.demo_driver_tick();
            // Demo mode runs on ephemeral state — never write to disk.
            return;
        }
        self.ticks_since_save += 1;
        if self.ticks_since_save >= SAVE_INTERVAL_TICKS {
            self.ticks_since_save = 0;
            let _ = save::save(&self.state);
        }
    }

    fn maybe_idle_clench(&mut self) {
        if self.state.clench_ticks > 0 {
            return;
        }
        // ~1 per 45s average at 20Hz
        if rand::rng().random::<f64>() < 1.0 / 900.0 {
            self.state.trigger_clench();
        }
    }

    fn maybe_spawn_auto_particle(&mut self) {
        let fps = self.state.fps();
        if fps <= 0.0 || self.biscuit_rect.width < 4 || self.biscuit_rect.height < 4 {
            return;
        }
        let target_rate = fps.sqrt().clamp(0.5, 8.0);
        let prob = target_rate * TICK_DT;
        let mut rng = rand::rng();
        if rng.random::<f64>() >= prob {
            return;
        }
        let col = rng.random_range(
            self.biscuit_rect.x + 1
                ..self.biscuit_rect.x + self.biscuit_rect.width.saturating_sub(2),
        );
        let row = rng.random_range(
            self.biscuit_rect.y + 1
                ..self.biscuit_rect.y + self.biscuit_rect.height.saturating_sub(1),
        );
        self.state.spawn_auto_particle(col, row);
    }

    fn maybe_spawn_golden(&mut self) {
        if self.state.golden.is_some() || self.state.golden_cooldown > 0 {
            return;
        }
        if self.biscuit_rect.width < 8 || self.biscuit_rect.height < 5 {
            return;
        }
        let col_range = (
            self.biscuit_rect.x + 2,
            self.biscuit_rect.x + self.biscuit_rect.width.saturating_sub(8),
        );
        let row_range = (
            self.biscuit_rect.y + 1,
            self.biscuit_rect.y + self.biscuit_rect.height.saturating_sub(4),
        );
        self.state.golden = Some(golden::spawn_in(col_range, row_range));
    }

    fn dispatch(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind == KeyEventKind::Press => self.handle_key(k),
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                self.handle_click(m.column, m.row);
            }
            Event::Mouse(m) if m.kind == MouseEventKind::ScrollUp => {
                let _ = m;
                self.zoom_idx = self.zoom_idx.saturating_sub(1);
            }
            Event::Mouse(m) if m.kind == MouseEventKind::ScrollDown => {
                let _ = m;
                self.zoom_idx = (self.zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, k: KeyEvent) {
        let code = k.code;
        let mods = k.modifiers;
        match code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Esc => match self.mode {
                Mode::Game => self.running = false,
                _ => self.mode = Mode::Game,
            },
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.mode = match self.mode {
                    Mode::Stats => Mode::Game,
                    _ => Mode::Stats,
                };
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.mode = match self.mode {
                    Mode::Achievements => Mode::Game,
                    _ => Mode::Achievements,
                };
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                self.mode = match self.mode {
                    Mode::Upgrades => Mode::Game,
                    _ => Mode::Upgrades,
                };
            }
            // [g] catches any Golden Cuque variant (Lucky reward, Frenzy
            // click buff, or Buff fingerer boost) — keyboard equivalent of
            // clicking the on-screen marker.
            KeyCode::Char('g') | KeyCode::Char('G') if self.state.golden.is_some() => {
                self.state.catch_golden();
            }
            // Debug/testing: gated behind dev mode. See src/ui/debug_pane.rs
            // for the advertised key list.
            KeyCode::F(1) if self.debug => self.force_spawn_golden(GoldenVariant::Lucky),
            KeyCode::F(2) if self.debug => self.force_spawn_golden(GoldenVariant::Frenzy),
            KeyCode::F(3) if self.debug => self.force_spawn_golden(GoldenVariant::Buff),
            KeyCode::F(4) if self.debug => self.state.dev_add_cuques(1_000_000.0),
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.mode = match self.mode {
                    Mode::Prestige => Mode::Game,
                    _ => Mode::Prestige,
                };
            }
            KeyCode::Char('r') | KeyCode::Char('R')
                if self.mode == Mode::Prestige && self.state.prestige_reset() =>
            {
                self.mode = Mode::Game;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.zoom_idx = self.zoom_idx.saturating_sub(1);
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.zoom_idx = (self.zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if self.mode == Mode::Prestige {
                    if self.state.prestige_reset() {
                        self.mode = Mode::Game;
                    }
                } else if self.mode == Mode::Game {
                    self.click_center();
                }
            }
            KeyCode::Char(c) => {
                if let Some((slot, shifted_sym)) = digit_slot(c) {
                    let buy_10 = shifted_sym || mods.contains(KeyModifiers::SHIFT);
                    let buy_max =
                        mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::CONTROL);
                    match self.mode {
                        Mode::Game => {
                            if let Some(&fid) = self.visible_fingerers.get(slot) {
                                if buy_max {
                                    self.state.buy_max(fid);
                                } else if buy_10 {
                                    self.state.buy_n(fid, 10);
                                } else {
                                    self.state.buy(fid);
                                }
                            }
                        }
                        Mode::Upgrades => {
                            if let Some(&u_idx) = self.visible_upgrades.get(slot) {
                                self.state.buy_upgrade(u_idx);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_click(&mut self, col: u16, row: u16) {
        if self.mode != Mode::Game {
            return;
        }
        let g = self.golden_rect;
        if g.width > 0 && col >= g.x && col < g.x + g.width && row >= g.y && row < g.y + g.height {
            self.state.catch_golden();
            return;
        }
        let r = self.biscuit_rect;
        if col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height {
            self.state.click((col, row));
        }
    }

    fn click_center(&mut self) {
        let r = self.biscuit_rect;
        if r.width == 0 || r.height == 0 {
            return;
        }
        self.state.click((r.x + r.width / 2, r.y + r.height / 2));
    }

    fn force_spawn_golden(&mut self, variant: GoldenVariant) {
        if self.biscuit_rect.width < 8 || self.biscuit_rect.height < 5 {
            return;
        }
        let col_range = (
            self.biscuit_rect.x + 2,
            self.biscuit_rect.x + self.biscuit_rect.width.saturating_sub(8),
        );
        let row_range = (
            self.biscuit_rect.y + 1,
            self.biscuit_rect.y + self.biscuit_rect.height.saturating_sub(4),
        );
        let mut g = golden::spawn_in(col_range, row_range);
        g.variant = variant;
        self.state.golden = Some(g);
    }

    /// Demo-mode autopilot: injects synthetic inputs on a loose schedule so
    /// the whole game surface (clicks, buffs, buys, panel swaps) gets
    /// exercised on camera, and quits the app when the requested recording
    /// duration elapses.
    ///
    /// Pacing is intentionally *slower* than real gameplay — a recorded clip
    /// needs the viewer to visually track what's happening. Too many events
    /// per second just read as noise.
    fn demo_driver_tick(&mut self) {
        self.demo_ticks += 1;
        let t = self.demo_ticks;
        let mut rng = rand::rng();

        // ~1.5 clicks/s. Real play is faster; on camera it'd smear.
        if t.is_multiple_of(13) {
            self.click_center();
        }

        // Keep the screen busy with goldens: tighter cooldown than normal.
        if self.state.golden.is_none() && self.state.golden_cooldown == 0 {
            self.state.golden_cooldown = DEMO_GOLDEN_COOLDOWN;
        }

        // Force the variant on freshly-spawned goldens so the clip deterministically
        // cycles Buff → Frenzy → Lucky instead of rolling random weights. Buff comes
        // first so a viewer definitely sees the purple powerup.
        if let Some(g) = &mut self.state.golden
            && g.life_ticks == golden::GOLDEN_LIFE_TICKS
        {
            g.variant = match self.demo_golden_spawns % 3 {
                0 => GoldenVariant::Buff,
                1 => GoldenVariant::Frenzy,
                _ => GoldenVariant::Lucky,
            };
            self.demo_golden_spawns += 1;
        }

        // Auto-catch whatever golden is on screen after a brief "reaction
        // time" so the marker is actually visible before disappearing.
        if let Some(g) = &self.state.golden
            && g.life_ticks + 20 < golden::GOLDEN_LIFE_TICKS
        {
            self.state.catch_golden();
        }

        // Every ~4s, buy 1-2 of a random affordable fingerer — slow enough
        // that a viewer sees each purchase flash as a distinct event.
        if t.is_multiple_of(80) {
            let candidates: Vec<usize> = (0..fingerer::count())
                .filter(|&i| self.state.can_buy(i))
                .collect();
            if !candidates.is_empty() {
                let idx = candidates[rng.random_range(0..candidates.len())];
                self.state.buy_n(idx, rng.random_range(1..=2));
            }
        }

        // Every ~8s, buy the cheapest available upgrade — drives the HUD
        // border carrier up to white with a mint flash.
        if t.is_multiple_of(160) {
            let available = crate::game::upgrade::available_ids(&self.state);
            if let Some(&u_idx) = available
                .iter()
                .min_by(|&&a, &&b| UPGRADES[a].cost.partial_cmp(&UPGRADES[b].cost).unwrap())
            {
                self.state.buy_upgrade(u_idx);
            }
        }

        // Every ~15s, show a non-game panel for ~2s.
        let phase = t % 300;
        if phase == 100 {
            self.mode = Mode::Stats;
        } else if phase == 140 {
            self.mode = Mode::Achievements;
        } else if phase == 180 {
            self.mode = Mode::Upgrades;
        } else if phase == 220 {
            self.mode = Mode::Game;
        }

        // Deadline: auto-quit when the user's requested duration elapses so
        // the asciinema recording sees a clean exit.
        if let Some(secs) = self.demo_seconds
            && t >= (secs as u64) * (TICK_HZ as u64)
        {
            self.running = false;
        }
    }
}

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
