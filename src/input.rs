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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
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
    pub play_area: Rect,
    pub prestige_reset_rect: Rect,
    pub debug: bool,
    pub current: &'a GameState,
}

/// Process one [`InputEvent`]. Mutates [`UiState`]; appends produced actions
/// to `out`. Pure data — does no I/O, doesn't touch [`GameState`].
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
    if rect_contains(ctx.golden_rect, col, row) {
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
        KeyCode::Char('q') => ui.running = false,
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
        KeyCode::Char('g') | KeyCode::Char('G') if ctx.current.golden.is_some() => {
            out.push(Action::CatchGolden);
        }
        // Debug/testing: gated by `debug`. See src/ui/debug_pane.rs for the
        // advertised key list.
        KeyCode::F(1) if ctx.debug => {
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
