//! Platform-agnostic input router.
//!
//! Defines a small [`InputEvent`] vocabulary that's a strict superset of
//! what we need to translate from any input source (crossterm on native,
//! ratzilla on web). The router consumes one [`InputEvent`] and produces
//! zero or more [`Action`]s into a caller-owned `Vec<Action>` buffer; it
//! also mutates the [`UiState`] that doesn't belong on the sim side
//! (`mode`, `zoom_idx`, `running`, `last_mouse_pos`).
//!
//! Adapters live next to their event source — `app.rs::translate_crossterm`
//! produces `InputEvent` from `crossterm::event::Event`, and (when the
//! wasm port lands) a sibling adapter does the same for `ratzilla::event`.
//! Both feed the same router so behavior parity is enforced by sharing
//! code, not by duplicating it.
//!
//! Geometry (`fingerer_rows`, `upgrade_rows`, `help_hits`, etc.) is passed
//! in via [`InputContext`] — the renderer recomputes these every frame
//! and the click handler hit-tests against the latest set.

use ratatui::layout::Rect;

use crate::game::state::GameState;
use crate::sim::{Action, BuyQty};
use crate::ui::{HelpAction, Mode};

/// Platform-neutral input vocabulary. Crossterm's `Event::{Key,Mouse,Resize,…}`
/// and ratzilla's `KeyEvent`/`MouseEvent`/`WheelEvent` both narrow into this
/// — anything we don't need (focus, paste, resize) is dropped at the adapter.
#[derive(Clone, Debug)]
pub enum InputEvent {
    /// A key was pressed. `code` is the resolved key (with shifted symbols
    /// already mapped to their character form, e.g. Shift+1 → `!`).
    KeyPress { code: KeyCode, mods: Modifiers },
    /// A mouse button went down at terminal cell `(col, row)`.
    MouseDown {
        col: u16,
        row: u16,
        button: MouseButton,
        mods: Modifiers,
    },
    /// The mouse moved over `(col, row)` — used for hover highlighting.
    /// Drag events normalize to this too; the router doesn't care which.
    MouseMoved { col: u16, row: u16 },
    /// Scroll wheel scrolled. `(col, row)` is the cursor cell at the time
    /// of the wheel tick — used to gate zoom to the play-area.
    Wheel {
        col: u16,
        row: u16,
        delta: WheelDelta,
    },
}

/// Subset of key codes the game actually consumes. Anything else from the
/// underlying terminal event is dropped at the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Esc,
    F(u8),
}

/// Subset of mouse buttons the game cares about. Middle-click is dropped
/// at every adapter (it had no game effect on native and we don't intend
/// one on web either); add a variant here if a future input source wants
/// it routed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelDelta {
    Up,
    Down,
}

/// State that lives on the input/render side of the boundary, not on the
/// sim. Persistence-wise: not serialized, recreated fresh on each launch.
pub struct UiState {
    pub mode: Mode,
    pub zoom_idx: usize,
    pub running: bool,
    pub last_mouse_pos: Option<(u16, u16)>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            mode: Mode::Game,
            zoom_idx: 0,
            running: true,
            last_mouse_pos: None,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-frame geometry the click router hit-tests against. All `Rect`s come
/// from the latest `ui::draw` output; `current` is the latest published
/// snapshot. Borrowed for the duration of one event dispatch.
pub struct InputContext<'a> {
    pub fingerer_rows: &'a [(usize, Rect)],
    pub upgrade_rows: &'a [(usize, Rect)],
    pub help_hits: &'a [(HelpAction, Rect)],
    pub biscuit_rect: Rect,
    pub golden_rect: Rect,
    /// Hit-test rect for the on-screen Green Coin marker. Zero-rect when
    /// no coin is visible; click-routes through `Action::CatchGolden` (the
    /// sim resolves both Golden and Green Coin from that single action).
    pub green_coin_rect: Rect,
    pub play_area: Rect,
    pub prestige_reset_rect: Rect,
    pub debug: bool,
    pub current: &'a GameState,
}

impl<'a> InputContext<'a> {
    /// Build an input context by borrowing from a render
    /// [`DrawOutput`](crate::ui::DrawOutput).
    ///
    /// The platform shells (`app.rs`, `wasm_app.rs`) call this so adding
    /// a new clickable region only touches `DrawOutput` + `InputContext` +
    /// this projection — never the platform code. Without this single
    /// projection point, native and wasm each kept their own field-by-field
    /// copy of the layout snapshot, and a new field meant updating both;
    /// the wasm build broke once when only the native copy was updated.
    pub fn from_layout(
        layout: &'a crate::ui::DrawOutput,
        current: &'a GameState,
        debug: bool,
    ) -> Self {
        InputContext {
            fingerer_rows: &layout.fingerer_rows,
            upgrade_rows: &layout.upgrade_rows,
            help_hits: &layout.help_hits,
            biscuit_rect: layout.biscuit_rect,
            golden_rect: layout.golden_rect,
            green_coin_rect: layout.green_coin_rect,
            play_area: layout.play_area,
            prestige_reset_rect: layout.prestige_reset_rect,
            debug,
            current,
        }
    }
}

/// Process one [`InputEvent`]. Mutates [`UiState`]; appends produced actions
/// to `out`. Pure data — does no I/O. The router *reads* `GameState` (via
/// `ctx.current` for `prestige_available()` / `golden.is_some()` and via
/// `ui::hands::occupied_at` for misclick gating) but never mutates it; all
/// mutation flows through the produced [`Action`]s and `apply_action`.
pub fn process_input_event(
    ev: InputEvent,
    ui: &mut UiState,
    ctx: &InputContext,
    out: &mut Vec<Action>,
) {
    match ev {
        InputEvent::KeyPress { code, mods } => handle_key(code, mods, ui, ctx, out),
        InputEvent::MouseDown {
            col,
            row,
            button,
            mods,
        } => {
            ui.last_mouse_pos = Some((col, row));
            // M1+M2: try help-bar / prestige-reset hits first. These give
            // the mouse-only player parity with `[u]/[p]/[s]/[a]/[g]/[q]/[r]`
            // shortcuts. Consumed hits short-circuit the rest of the click
            // pipeline so we don't also fire a misclick particle.
            if try_help_click(col, row, ui, ctx, out) {
                return;
            }
            handle_click(col, row, button, mods, ui, ctx, out);
        }
        InputEvent::MouseMoved { col, row } => {
            // K5: hover highlighting; renderer reads `last_mouse_pos`.
            // Drag events from the underlying terminal collapse to this.
            ui.last_mouse_pos = Some((col, row));
        }
        InputEvent::Wheel { col, row, delta } => {
            // Scroll only zooms inside the play area (the whole left column
            // where the biscuit lives, including the void around a small
            // biscuit at low zoom). Cold frames (no rect yet) conservatively
            // allow zoom so the very first scroll after launch isn't dropped.
            if !in_play_area(col, row, ctx.play_area) {
                return;
            }
            match delta {
                WheelDelta::Up => ui.zoom_idx = ui.zoom_idx.saturating_sub(1),
                WheelDelta::Down => {
                    ui.zoom_idx = (ui.zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
                }
            }
        }
    }
}

/// True when the scroll happened anywhere inside the play-area rect — the
/// whole left column the biscuit lives in (HUD-and-help-excluded). Cold
/// frames (no rect yet) conservatively allow zoom so the very first scroll
/// after launch isn't dropped.
fn in_play_area(col: u16, row: u16, play_area: Rect) -> bool {
    if play_area.width == 0 || play_area.height == 0 {
        return true;
    }
    col >= play_area.x
        && col < play_area.x + play_area.width
        && row >= play_area.y
        && row < play_area.y + play_area.height
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

fn click_buy_qty(mods: Modifiers) -> BuyQty {
    if mods.alt || mods.ctrl {
        BuyQty::Max
    } else if mods.shift {
        BuyQty::Ten
    } else {
        BuyQty::One
    }
}

/// Try to consume a click on a help-bar hint or the prestige-reset confirm
/// line. Returns true when the click was handled — caller short-circuits
/// the rest of the pipeline (no biscuit/row/misclick path).
fn try_help_click(
    col: u16,
    row: u16,
    ui: &mut UiState,
    ctx: &InputContext,
    out: &mut Vec<Action>,
) -> bool {
    // Prestige-reset confirm: in-panel button. Match BEFORE help-bar so
    // the confirm "wins" if the help bar happens to overlap it (it
    // shouldn't, but defensive).
    if rect_contains(ctx.prestige_reset_rect, col, row) && ctx.current.prestige_available() > 0 {
        out.push(Action::PrestigeReset);
        ui.mode = Mode::Game;
        return true;
    }
    for &(action, rect) in ctx.help_hits {
        if !rect_contains(rect, col, row) {
            continue;
        }
        match action {
            HelpAction::OpenMode(target) => {
                // Same toggle semantics the keyboard uses: tapping the
                // hint for the active mode returns to Game.
                ui.mode = if ui.mode == target {
                    Mode::Game
                } else {
                    target
                };
            }
            HelpAction::GrabGolden => {
                if ctx.current.golden.is_some() {
                    out.push(Action::CatchGolden);
                }
            }
            HelpAction::PrestigeReset => {
                if ctx.current.prestige_available() > 0 {
                    out.push(Action::PrestigeReset);
                    ui.mode = Mode::Game;
                }
            }
            HelpAction::Quit => {
                ui.running = false;
            }
        }
        return true;
    }
    false
}

fn handle_click(
    col: u16,
    row: u16,
    button: MouseButton,
    mods: Modifiers,
    ui: &UiState,
    ctx: &InputContext,
    out: &mut Vec<Action>,
) {
    // Golden cuques are catchable from ANY panel — match the keyboard 'g'
    // behavior, which has no mode guard. The marker still renders on the
    // biscuit while a non-Game panel is open. Right-click on a golden
    // also catches.
    if rect_contains(ctx.golden_rect, col, row) || rect_contains(ctx.green_coin_rect, col, row) {
        out.push(Action::CatchGolden);
        return;
    }
    // Clicking the biscuit itself is also mode-agnostic. Right-click on
    // the biscuit is a no-op so a player can't accidentally finger the
    // cuque with the wrong button.
    if rect_contains(ctx.biscuit_rect, col, row) {
        if button == MouseButton::Left {
            out.push(Action::Click { col, row });
        }
        return;
    }
    // Mouse-buy fingerers from the sidebar in Game mode. Modifiers control
    // quantity (plain = 1, Shift = 10, Alt/Ctrl = max), matching the
    // digit-key shortcuts. Right-click is the always-Max affordance
    // regardless of modifiers.
    if ui.mode == Mode::Game {
        for &(idx, r) in ctx.fingerer_rows {
            if rect_contains(r, col, row) {
                let qty = if button == MouseButton::Right {
                    BuyQty::Max
                } else {
                    click_buy_qty(mods)
                };
                out.push(Action::BuyFingerer { idx, qty });
                return;
            }
        }
    }
    // Mouse-buy upgrades from the Upgrades panel. Modifiers ignored — each
    // upgrade is a one-shot purchase. Right-click also buys.
    if ui.mode == Mode::Upgrades {
        for &(idx, r) in ctx.upgrade_rows {
            if rect_contains(r, col, row) {
                out.push(Action::BuyUpgrade(idx));
                return;
            }
        }
    }
    // J10: nothing actionable under the click. Acknowledge it visually with
    // a brief "·" so the dead-zone (e.g. the air around a 25%-zoom biscuit)
    // doesn't feel inert. Skip when:
    //   - the click was right-button (right-click without a target is a
    //     true no-op);
    //   - the click landed on an orbital hand glyph — those are decoration,
    //     not click targets, but they're visually present, so a misclick
    //     "·" replacing part of `[]` / `:*` / `>>` reads as flicker.
    //   - M3: the click landed OUTSIDE the play area (HUD title, sidebar,
    //     debug pane, help bar). Inert UI chrome shouldn't get a "·"
    //     overpainted into it.
    if button != MouseButton::Left {
        return;
    }
    if !rect_contains(ctx.play_area, col, row) {
        return;
    }
    if crate::ui::hands::occupied_at(col, row, ctx.biscuit_rect, ctx.current) {
        return;
    }
    out.push(Action::Misclick { col, row });
}

fn handle_key(
    code: KeyCode,
    mods: Modifiers,
    ui: &mut UiState,
    ctx: &InputContext,
    out: &mut Vec<Action>,
) {
    match code {
        // Gated on the platform's `can_quit` capability so a stray `q`
        // press in the browser doesn't silently flip `ui.running` (which
        // a future feature might key off of even though the rAF loop
        // doesn't read it today).
        KeyCode::Char('q') if crate::platform::CAPABILITIES.can_quit => ui.running = false,
        // J12: Esc dismisses panels back to Game mode but is a NO-OP from
        // Game itself. Quit is `q` only — Esc-to-quit was an aggressive
        // default that surprised playtesters who reflex-pressed it to
        // "deselect" with no panel open.
        KeyCode::Esc => match ui.mode {
            Mode::Game => {}
            _ => ui.mode = Mode::Game,
        },
        KeyCode::Char('s') | KeyCode::Char('S') => {
            ui.mode = if matches!(ui.mode, Mode::Stats) {
                Mode::Game
            } else {
                Mode::Stats
            };
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            ui.mode = if matches!(ui.mode, Mode::Achievements) {
                Mode::Game
            } else {
                Mode::Achievements
            };
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            ui.mode = if matches!(ui.mode, Mode::Upgrades) {
                Mode::Game
            } else {
                Mode::Upgrades
            };
        }
        // [g] catches any Golden Cuque variant. Guard on the latest snapshot
        // to avoid sending a noop CatchGolden when nothing is on screen.
        KeyCode::Char('g') | KeyCode::Char('G')
            if ctx.current.golden.is_some() || ctx.current.green_coin.is_some() =>
        {
            out.push(Action::CatchGolden);
        }
        // Debug/testing: gated by `debug`. See src/ui/debug_pane.rs for the
        // advertised key list. F8 (not F1) is Lucky because Chrome / Edge /
        // Safari hijack F1 for browser Help and never forward the keydown
        // to the wasm page; F8 has no default browser binding outside of
        // an open DevTools instance.
        KeyCode::F(8) if ctx.debug => {
            out.push(Action::DevForceGolden(
                crate::game::golden::GoldenVariant::Lucky,
            ));
        }
        KeyCode::F(2) if ctx.debug => {
            out.push(Action::DevForceGolden(
                crate::game::golden::GoldenVariant::Frenzy,
            ));
        }
        KeyCode::F(3) if ctx.debug => {
            out.push(Action::DevForceGolden(
                crate::game::golden::GoldenVariant::Buff,
            ));
        }
        KeyCode::F(4) if ctx.debug => {
            out.push(Action::DevAddCuques(1_000_000.0));
        }
        KeyCode::F(5) if ctx.debug => {
            out.push(Action::DevSpawnGreenCoin);
        }
        KeyCode::Char('p') | KeyCode::Char('P') => {
            ui.mode = if matches!(ui.mode, Mode::Prestige) {
                Mode::Game
            } else {
                Mode::Prestige
            };
        }
        // Prestige confirm: check the snapshot for available prestige before
        // firing. Optimistically close the panel — if the sim rejects the
        // reset (raced against a simultaneous lifetime-cuque drop) nothing
        // bad happens.
        KeyCode::Char('r') | KeyCode::Char('R')
            if ui.mode == Mode::Prestige && ctx.current.prestige_available() > 0 =>
        {
            out.push(Action::PrestigeReset);
            ui.mode = Mode::Game;
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            ui.zoom_idx = ui.zoom_idx.saturating_sub(1);
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            ui.zoom_idx = (ui.zoom_idx + 1).min(crate::ui::biscuit::level_count() - 1);
        }
        // Space ALWAYS fingers the cuque, regardless of which panel is open
        // — same contract as left-click on the biscuit.
        KeyCode::Char(' ') => {
            out.push(Action::ClickCenter);
        }
        KeyCode::Char(c) => {
            if let Some((slot, shifted_sym)) = digit_slot(c) {
                let buy_10 = shifted_sym || mods.shift;
                let buy_max = mods.alt || mods.ctrl;
                match ui.mode {
                    Mode::Game => {
                        if let Some(&(fid, _)) = ctx.fingerer_rows.get(slot) {
                            let qty = if buy_max {
                                BuyQty::Max
                            } else if buy_10 {
                                BuyQty::Ten
                            } else {
                                BuyQty::One
                            };
                            out.push(Action::BuyFingerer { idx: fid, qty });
                        }
                    }
                    Mode::Upgrades => {
                        if let Some(&(u_idx, _)) = ctx.upgrade_rows.get(slot) {
                            out.push(Action::BuyUpgrade(u_idx));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    //! Router tests. The whole point of pulling `process_input_event` into a
    //! platform-neutral module is that we can exercise it with no terminal,
    //! no event loop, and no threading. Each test feeds a synthetic event
    //! and asserts on the produced `Vec<Action>` + `UiState` deltas.
    //!
    //! Constructing an `InputContext` requires stubs for the per-frame rects
    //! and a borrowed `GameState`; helpers below collapse the boilerplate.
    //! `state_with_golden()` and `state_with_prestige()` mutate just enough
    //! of the default to exercise the gates the router cares about.
    use super::*;
    use crate::game::golden::{self, GoldenVariant};
    use crate::sim::{Action, BuyQty};
    use ratatui::layout::Rect;
    use std::mem::discriminant;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    /// Builds an `InputContext` over caller-provided rect/row/state slices.
    /// The lifetimes line up because everything is borrowed from locals
    /// the test itself owns.
    #[allow(clippy::too_many_arguments)]
    fn ctx<'a>(
        biscuit: Rect,
        golden_rect: Rect,
        play_area: Rect,
        prestige_reset_rect: Rect,
        fingerer_rows: &'a [(usize, Rect)],
        upgrade_rows: &'a [(usize, Rect)],
        help_hits: &'a [(HelpAction, Rect)],
        debug: bool,
        current: &'a GameState,
    ) -> InputContext<'a> {
        InputContext {
            fingerer_rows,
            upgrade_rows,
            help_hits,
            biscuit_rect: biscuit,
            golden_rect,
            green_coin_rect: Rect::default(),
            play_area,
            prestige_reset_rect,
            debug,
            current,
        }
    }

    fn empty_ctx<'a>(state: &'a GameState) -> InputContext<'a> {
        ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            state,
        )
    }

    /// State with a golden cuque on screen, so [g] / golden-rect clicks have
    /// something to catch.
    fn state_with_golden() -> GameState {
        let mut g = golden::spawn_in(rect(10, 10, 20, 10));
        g.variant = GoldenVariant::Lucky;
        GameState {
            golden: Some(g),
            ..GameState::default()
        }
    }

    /// State with enough lifetime cuques to make `prestige_available()` > 0,
    /// so the prestige-reset confirm rect is "live".
    fn state_with_prestige() -> GameState {
        // prestige_available() square-roots `lifetime_cuques / 1e9` and
        // floors. 4e9 → 2 prestige tokens.
        GameState {
            lifetime_cuques: 4_000_000_000.0,
            ..GameState::default()
        }
    }

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::KeyPress {
            code,
            mods: Modifiers::default(),
        }
    }

    fn key_with(code: KeyCode, shift: bool, alt: bool, ctrl: bool) -> InputEvent {
        InputEvent::KeyPress {
            code,
            mods: Modifiers { shift, alt, ctrl },
        }
    }

    fn mouse_down(col: u16, row: u16, button: MouseButton, mods: Modifiers) -> InputEvent {
        InputEvent::MouseDown {
            col,
            row,
            button,
            mods,
        }
    }

    // -- Key handling ------------------------------------------------------

    #[test]
    fn q_key_flips_running_off() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('q')), &mut ui, &empty_ctx(&s), &mut out);
        assert!(!ui.running);
        assert!(out.is_empty());
    }

    #[test]
    fn esc_from_game_is_noop() {
        // J12: Esc from Game must not quit and must not change mode.
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Esc), &mut ui, &empty_ctx(&s), &mut out);
        assert!(ui.running);
        assert_eq!(ui.mode, Mode::Game);
        assert!(out.is_empty());
    }

    #[test]
    fn esc_from_stats_returns_to_game() {
        let s = GameState::default();
        let mut ui = UiState::new();
        ui.mode = Mode::Stats;
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Esc), &mut ui, &empty_ctx(&s), &mut out);
        assert_eq!(ui.mode, Mode::Game);
        assert!(out.is_empty());
    }

    #[test]
    fn s_key_toggles_stats() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('s')), &mut ui, &empty_ctx(&s), &mut out);
        assert_eq!(ui.mode, Mode::Stats);
        process_input_event(key(KeyCode::Char('s')), &mut ui, &empty_ctx(&s), &mut out);
        assert_eq!(ui.mode, Mode::Game);
    }

    #[test]
    fn space_emits_click_center() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char(' ')), &mut ui, &empty_ctx(&s), &mut out);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Action::ClickCenter));
    }

    #[test]
    fn g_with_no_golden_is_silent() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &empty_ctx(&s), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn g_with_golden_emits_catch() {
        let s = state_with_golden();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &empty_ctx(&s), &mut out);
        assert!(matches!(out.as_slice(), [Action::CatchGolden]));
    }

    #[test]
    fn fkeys_gated_by_debug() {
        let s = GameState::default();
        let mut ui = UiState::new();
        // debug=false → all F-keys silent.
        let mut out = Vec::new();
        let c = empty_ctx(&s);
        process_input_event(key(KeyCode::F(8)), &mut ui, &c, &mut out);
        process_input_event(key(KeyCode::F(2)), &mut ui, &c, &mut out);
        process_input_event(key(KeyCode::F(3)), &mut ui, &c, &mut out);
        process_input_event(key(KeyCode::F(4)), &mut ui, &c, &mut out);
        assert!(out.is_empty(), "F-keys must be silent when debug=false");
    }

    #[test]
    fn fkeys_active_when_debug() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            &[],
            &[],
            &[],
            true, // debug ON
            &s,
        );
        let mut out = Vec::new();
        process_input_event(key(KeyCode::F(8)), &mut ui, &c, &mut out);
        process_input_event(key(KeyCode::F(4)), &mut ui, &c, &mut out);
        assert!(matches!(
            out[0],
            Action::DevForceGolden(GoldenVariant::Lucky)
        ));
        assert!(matches!(out[1], Action::DevAddCuques(_)));
    }

    // -- Digit shortcuts: modifier→BuyQty ----------------------------------

    fn fingerer_row_ctx<'a>(state: &'a GameState, rows: &'a [(usize, Rect)]) -> InputContext<'a> {
        ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            rows,
            &[],
            &[],
            false,
            state,
        )
    }

    #[test]
    fn digit_1_buys_one() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(0_usize, rect(0, 0, 1, 1))];
        let mut out = Vec::new();
        process_input_event(
            key(KeyCode::Char('1')),
            &mut ui,
            &fingerer_row_ctx(&s, &rows),
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                idx: 0,
                qty: BuyQty::One,
            }]
        ));
    }

    #[test]
    fn shifted_digit_symbol_buys_ten() {
        // Terminals emit '!' for Shift+1 without a SHIFT modifier on macOS.
        // The router must recognize '!' as the Shift-1 alias.
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(0_usize, rect(0, 0, 1, 1))];
        let mut out = Vec::new();
        process_input_event(
            key(KeyCode::Char('!')),
            &mut ui,
            &fingerer_row_ctx(&s, &rows),
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                idx: 0,
                qty: BuyQty::Ten,
            }]
        ));
    }

    #[test]
    fn shift_modifier_on_digit_buys_ten() {
        // Some keymaps emit '1' WITH the SHIFT modifier — also valid for ×10.
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(0_usize, rect(0, 0, 1, 1))];
        let mut out = Vec::new();
        process_input_event(
            key_with(KeyCode::Char('1'), true, false, false),
            &mut ui,
            &fingerer_row_ctx(&s, &rows),
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                qty: BuyQty::Ten,
                ..
            }]
        ));
    }

    #[test]
    fn alt_or_ctrl_modifier_on_digit_buys_max() {
        let s = GameState::default();
        let rows = [(0_usize, rect(0, 0, 1, 1))];
        for (alt, ctrl) in [(true, false), (false, true)] {
            let mut ui = UiState::new();
            let mut out = Vec::new();
            process_input_event(
                key_with(KeyCode::Char('1'), false, alt, ctrl),
                &mut ui,
                &fingerer_row_ctx(&s, &rows),
                &mut out,
            );
            assert!(
                matches!(
                    out.as_slice(),
                    [Action::BuyFingerer {
                        qty: BuyQty::Max,
                        ..
                    }]
                ),
                "alt={alt} ctrl={ctrl} should buy max",
            );
        }
    }

    #[test]
    fn digit_with_no_visible_row_is_silent() {
        // Game mode but no fingerer_rows yet (cold frame): pressing 1 must
        // not panic or emit an action.
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(
            key(KeyCode::Char('1')),
            &mut ui,
            &fingerer_row_ctx(&s, &[]),
            &mut out,
        );
        assert!(out.is_empty());
    }

    // -- Mouse button semantics --------------------------------------------

    #[test]
    fn left_click_on_biscuit_emits_click() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(10, 5, 30, 20),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(20, 10, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(
            matches!(out.as_slice(), [Action::Click { col: 20, row: 10 }]),
            "got {:?}",
            out
        );
    }

    #[test]
    fn right_click_on_biscuit_is_noop() {
        // Right-click on the biscuit must not finger the cuque (avoids
        // accidental clicks). Specifically: no Click action, no misclick.
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(10, 5, 30, 20),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(20, 10, MouseButton::Right, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(out.is_empty(), "got {:?}", out);
    }

    #[test]
    fn left_click_on_golden_emits_catch() {
        let s = state_with_golden();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            rect(50, 12, 4, 2),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(51, 13, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(matches!(out.as_slice(), [Action::CatchGolden]));
    }

    #[test]
    fn right_click_on_golden_also_catches() {
        // The marker is small and reflex right-clicks shouldn't waste it.
        let s = state_with_golden();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            rect(50, 12, 4, 2),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(51, 13, MouseButton::Right, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(matches!(out.as_slice(), [Action::CatchGolden]));
    }

    #[test]
    fn left_click_on_fingerer_row_buys_one() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(2_usize, rect(100, 5, 38, 3))];
        let c = ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            &rows,
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(110, 6, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                idx: 2,
                qty: BuyQty::One,
            }]
        ));
    }

    #[test]
    fn right_click_on_fingerer_row_buys_max() {
        // J15: right-click is the always-Max affordance (modifiers ignored).
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(2_usize, rect(100, 5, 38, 3))];
        let c = ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            &rows,
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(110, 6, MouseButton::Right, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                qty: BuyQty::Max,
                ..
            }]
        ));
    }

    #[test]
    fn shift_left_click_on_fingerer_row_buys_ten() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(2_usize, rect(100, 5, 38, 3))];
        let c = ctx(
            Rect::default(),
            Rect::default(),
            Rect::default(),
            Rect::default(),
            &rows,
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        let mods = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        process_input_event(
            mouse_down(110, 6, MouseButton::Left, mods),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(matches!(
            out.as_slice(),
            [Action::BuyFingerer {
                qty: BuyQty::Ten,
                ..
            }]
        ));
    }

    // -- Misclick gating ---------------------------------------------------

    #[test]
    fn dead_zone_left_click_inside_play_area_emits_misclick() {
        // Click in the empty space of the play area (not biscuit, not row).
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(40, 10, 20, 10), // biscuit far from click
            Rect::default(),
            rect(0, 0, 100, 30), // play_area covers the click
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(5, 5, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(
            matches!(out.as_slice(), [Action::Misclick { col: 5, row: 5 }]),
            "got {:?}",
            out
        );
    }

    #[test]
    fn click_outside_play_area_does_not_misclick() {
        // M3: clicks on inert UI chrome (HUD title, sidebar, debug pane)
        // must NOT spawn a misclick particle.
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(40, 10, 20, 10),
            Rect::default(),
            rect(0, 0, 100, 30), // play area capped at col 100
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(120, 5, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(out.is_empty(), "got {:?}", out);
    }

    #[test]
    fn right_click_in_dead_zone_is_silent() {
        // Right-click on nothing actionable is a true no-op (no misclick ack).
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(40, 10, 20, 10),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(5, 5, MouseButton::Right, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(out.is_empty());
    }

    // -- Help-bar clickable hints ------------------------------------------

    #[test]
    fn click_quit_hint_flips_running() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let hits = [(HelpAction::Quit, rect(50, 29, 8, 1))];
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &hits,
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(53, 29, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(!ui.running);
        assert!(out.is_empty(), "Quit is UI-only, no Action emitted");
    }

    #[test]
    fn click_open_mode_hint_toggles_mode() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let hits = [(HelpAction::OpenMode(Mode::Stats), rect(50, 29, 8, 1))];
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &hits,
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(53, 29, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.mode, Mode::Stats);
    }

    // -- Prestige reset rect -----------------------------------------------

    #[test]
    fn prestige_reset_rect_unavailable_does_not_reset() {
        // With prestige_available() == 0, clicking the confirm rect must
        // NOT produce a PrestigeReset action. The click can still fall
        // through to a Misclick if it's in the play area (that's by
        // design — it's a dead-zone click) but never to a reset.
        let s = GameState::default(); // prestige_available() = 0
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            rect(40, 15, 30, 1), // prestige_reset_rect at this position
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(50, 15, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(
            !out.iter().any(|a| matches!(a, Action::PrestigeReset)),
            "no PrestigeReset when unavailable; got {:?}",
            out
        );
        assert_eq!(ui.mode, Mode::Game, "mode unchanged from default Game");
    }

    #[test]
    fn prestige_reset_rect_available_emits_action() {
        let s = state_with_prestige();
        let mut ui = UiState::new();
        ui.mode = Mode::Prestige;
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            rect(40, 15, 30, 1),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(50, 15, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(discriminant(&out[0]), discriminant(&Action::PrestigeReset),);
        assert_eq!(
            ui.mode,
            Mode::Game,
            "panel auto-closes after prestige confirm",
        );
    }

    // -- Wheel zoom --------------------------------------------------------

    #[test]
    fn wheel_down_inside_play_area_increases_zoom_idx() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            InputEvent::Wheel {
                col: 50,
                row: 15,
                delta: WheelDelta::Down,
            },
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.zoom_idx, 1);
    }

    #[test]
    fn wheel_outside_play_area_does_not_zoom() {
        // Wheel events on the right-hand sidebar must not zoom the biscuit.
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            InputEvent::Wheel {
                col: 120,
                row: 10,
                delta: WheelDelta::Down,
            },
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.zoom_idx, 0);
    }

    #[test]
    fn wheel_up_saturates_at_zero() {
        let s = GameState::default();
        let mut ui = UiState::new();
        ui.zoom_idx = 0;
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            InputEvent::Wheel {
                col: 50,
                row: 15,
                delta: WheelDelta::Up,
            },
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.zoom_idx, 0, "saturating_sub at 0 stays 0");
    }

    #[test]
    fn wheel_down_caps_at_last_level() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let last = crate::ui::biscuit::level_count() - 1;
        ui.zoom_idx = last;
        let c = ctx(
            Rect::default(),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            InputEvent::Wheel {
                col: 50,
                row: 15,
                delta: WheelDelta::Down,
            },
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.zoom_idx, last, "min() cap at last level");
    }

    // -- Mouse position tracking -------------------------------------------

    #[test]
    fn mouse_moved_updates_last_position() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(
            InputEvent::MouseMoved { col: 42, row: 7 },
            &mut ui,
            &empty_ctx(&s),
            &mut out,
        );
        assert_eq!(ui.last_mouse_pos, Some((42, 7)));
        assert!(out.is_empty());
    }

    #[test]
    fn mouse_down_updates_last_position_before_dispatch() {
        // The hover renderer reads `last_mouse_pos` next frame; a click
        // should leave the cursor "where it landed" so the row beneath the
        // click is highlighted.
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            rect(40, 10, 20, 10),
            Rect::default(),
            rect(0, 0, 100, 30),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            &s,
        );
        let mut out = Vec::new();
        process_input_event(
            mouse_down(7, 7, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert_eq!(ui.last_mouse_pos, Some((7, 7)));
    }
}
