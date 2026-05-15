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
use crate::game::tree::coord::TreeCoord;
use crate::sim::{Action, BuyQty};
use crate::ui::{HelpAction, Mode, TreeButtonAction};

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
    /// A mouse button was released. Used to end drag tracking in the tree
    /// modal; click effects fire on `MouseDown`, not on `MouseUp`.
    MouseUp {
        col: u16,
        row: u16,
        button: MouseButton,
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
    /// Cursor / pan navigation. Mapped from crossterm `Up/Down/Left/Right`.
    Up,
    Down,
    Left,
    Right,
    /// Confirm — used by the tree modal to buy the focused node.
    Enter,
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
    pub tree_render: TreeRenderState,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            mode: Mode::Game,
            zoom_idx: 0,
            running: true,
            last_mouse_pos: None,
            tree_render: TreeRenderState::default(),
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render-side state for the upgrade tree modal. Holds the smoothed camera
/// pan (so panning eases instead of snapping), the active drag tracker,
/// and a snapshot of the last-seen cursor for cursor-change detection.
///
/// All `f32` because the tween needs sub-cell precision — the final
/// `pan_x` / `pan_y` are rounded back to integer cells in the renderer.
#[derive(Clone, Copy, Debug)]
pub struct TreeRenderState {
    /// Current rendered pan (canvas-cell coords). Eased toward `target_*`
    /// each frame; pan reads use `.round() as i32`.
    pub pan_x: f32,
    pub pan_y: f32,
    /// Where the camera is heading. Set to the cursor's centered position
    /// whenever the cursor changes; modified directly by drag.
    pub target_pan_x: f32,
    pub target_pan_y: f32,
    /// Previously-rendered cursor — drives cursor-change detection
    /// (cursor change resets `target_pan_*` to its centered position).
    pub prev_cursor: TreeCoord,
    /// `false` before the first frame in the current modal session; the
    /// renderer snaps `pan_*` to `target_*` on that frame instead of
    /// tweening. Reset to `false` on every modal close so the camera
    /// doesn't tween from a stale position on reopen.
    pub initialized: bool,
    /// Last mouse cell observed while the left button was held. While
    /// `Some`, MouseMoved events apply the (cell-delta) directly to
    /// `pan_*` AND `target_pan_*` so the dragged position sticks after
    /// release (the tween doesn't pull back to cursor).
    pub drag_last: Option<(u16, u16)>,
}

impl Default for TreeRenderState {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            target_pan_x: 0.0,
            target_pan_y: 0.0,
            prev_cursor: TreeCoord::ORIGIN,
            initialized: false,
            drag_last: None,
        }
    }
}

/// Per-frame geometry the click router hit-tests against. All `Rect`s come
/// from the latest `ui::draw` output; `current` is the latest published
/// snapshot. Borrowed for the duration of one event dispatch.
pub struct InputContext<'a> {
    pub fingerer_rows: &'a [(usize, Rect)],
    pub tree_node_rects: &'a [(crate::game::tree::coord::TreeCoord, Rect)],
    /// Optional left-clickable action button in the tree-modal info pane.
    /// `Some` when the focused node is currently actionable (buyable +
    /// affordable, or owned + refundable). Lets a touch / single-button
    /// player trigger buy/refund without needing right-click.
    pub tree_action_button: Option<(TreeButtonAction, Rect)>,
    pub help_hits: &'a [(HelpAction, Rect)],
    pub biscuit_rect: Rect,
    /// Screen position of the biscuit's focal cell. See
    /// [`crate::ui::DrawOutput::biscuit_focal`]. Used by `hands::occupied_at`
    /// to keep its hit-test math in sync with the visual orbit.
    pub biscuit_focal: (u16, u16),
    /// `(spawn_id, rect)` for every on-screen powerup. Click hit-test
    /// walks this list and routes the first match to
    /// `Action::CatchPowerup(spawn_id)`. The `g` hotkey min_by_key's it
    /// for the most-urgent. Both reference instances by id, never by
    /// Vec index — `swap_remove` on catch is safe.
    pub powerup_rects: &'a [(u64, Rect)],
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
            tree_node_rects: &layout.tree_node_rects,
            tree_action_button: layout.tree_action_button,
            help_hits: &layout.help_hits,
            biscuit_rect: layout.biscuit_rect,
            biscuit_focal: layout.biscuit_focal,
            powerup_rects: &layout.powerup_rects,
            play_area: layout.play_area,
            prestige_reset_rect: layout.prestige_reset_rect,
            debug,
            current,
        }
    }
}

/// Process one [`InputEvent`]. Mutates [`UiState`]; appends produced actions
/// to `out`. Pure data — does no I/O. The router *reads* `GameState` (via
/// `ctx.current` for `prestige_available()` / `powerups.iter()` and via
/// `ui::hands::occupied_at` for misclick gating) but never mutates it; all
/// mutation flows through the produced [`Action`]s and `apply_action`.
pub fn process_input_event(
    ev: InputEvent,
    ui: &mut UiState,
    ctx: &InputContext,
    out: &mut Vec<Action>,
) {
    match ev {
        InputEvent::KeyPress { code, mods } => {
            // Keyboard nav inside the tree modal cancels any in-progress
            // drag so a stray-held button doesn't keep panning after the
            // player switches to keyboard.
            if ui.mode == Mode::Tree {
                ui.tree_render.drag_last = None;
            }
            handle_key(code, mods, ui, ctx, out);
        }
        InputEvent::MouseDown {
            col,
            row,
            button,
            mods,
        } => {
            ui.last_mouse_pos = Some((col, row));
            // In tree mode, left-mouse-down starts a potential drag — the
            // drag actually begins on the next MouseMoved event with the
            // anchor still held. Click effects (focus, buy) still fire
            // immediately below.
            if ui.mode == Mode::Tree && button == MouseButton::Left {
                ui.tree_render.drag_last = Some((col, row));
            }
            // M1+M2: try help-bar / prestige-reset hits first. These give
            // the mouse-only player parity with `[u]/[p]/[s]/[a]/[g]/[q]/[r]`
            // shortcuts. Consumed hits short-circuit the rest of the click
            // pipeline so we don't also fire a misclick particle.
            if try_help_click(col, row, ui, ctx, out) {
                return;
            }
            handle_click(col, row, button, mods, ui, ctx, out);
        }
        InputEvent::MouseUp { col, row, button } => {
            ui.last_mouse_pos = Some((col, row));
            // Left release ends an in-progress tree drag. Other buttons
            // are not used for drag.
            if button == MouseButton::Left {
                ui.tree_render.drag_last = None;
            }
        }
        InputEvent::MouseMoved { col, row } => {
            // K5: hover highlighting; renderer reads `last_mouse_pos`.
            // Drag events from the underlying terminal collapse to this.
            // While the tree modal is open with a held-left, apply the
            // cell delta to the pan target so the camera follows the
            // mouse instantly (no tween — that would fight the drag).
            if ui.mode == Mode::Tree
                && let Some((lc, lr)) = ui.tree_render.drag_last
            {
                let dx = col as i32 - lc as i32;
                let dy = row as i32 - lr as i32;
                if dx != 0 || dy != 0 {
                    let r = &mut ui.tree_render;
                    r.pan_x -= dx as f32;
                    r.pan_y -= dy as f32;
                    r.target_pan_x -= dx as f32;
                    r.target_pan_y -= dy as f32;
                    r.drag_last = Some((col, row));
                }
            }
            ui.last_mouse_pos = Some((col, row));
        }
        InputEvent::Wheel { col, row, delta } => {
            // Biscuit zoom is meaningless in tree mode — the biscuit isn't
            // visible, and accidentally zooming-out behind the modal then
            // closing it is a confusing UX trap. Drop wheel events entirely
            // while the tree is open. (A future feature could repurpose
            // wheel for tree-canvas zoom; for now plain drop.)
            if ui.mode == Mode::Tree {
                return;
            }
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

/// Push the catch action for whichever on-screen powerup has the fewest
/// life ticks remaining (most-urgent across every kind). No-op if nothing
/// is on screen. Shared by the keyboard `g` handler and the help-bar
/// `[g]` click.
fn push_grab_most_urgent(ctx: &InputContext, out: &mut Vec<Action>) {
    if let Some(p) = ctx.current.powerups.iter().min_by_key(|p| p.life_ticks) {
        out.push(Action::CatchPowerup(p.spawn_id));
    }
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
                // Help-bar `[g]` click — grab the most-urgent powerup
                // currently on screen, identical to the keyboard 'g'.
                push_grab_most_urgent(ctx, out);
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
            HelpAction::TreeFocusOrigin => {
                out.push(Action::TreeFocus(TreeCoord::ORIGIN));
            }
            HelpAction::TreeFocusLastBought => {
                let target = ctx.current.tree.last_bought.unwrap_or(TreeCoord::ORIGIN);
                out.push(Action::TreeFocus(target));
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
    // Tree mode is a FULL-SCREEN modal that paints over the biscuit /
    // sidebar / HUD. The rects from those layers are still live in
    // `InputContext` (the renderer drew them before the tree overpainted),
    // and falling through to the normal click pipeline would let those
    // ghost rects swallow tree clicks. Concretely: the cuque-anchor
    // renders at screen-center, which is also where `biscuit_rect` sits,
    // so clicking the anchor used to fire `Action::Click` (a biscuit
    // finger) instead of `Action::TreeFocus`. Isolate tree-mode clicks
    // so only `tree_node_rects` apply.
    if ui.mode == Mode::Tree {
        // Left-click on the info-pane action button = buy or refund
        // the focused lot. Lets touch / single-button players trigger
        // the action without a right-click. Right-click here also
        // fires the action — same intent either way.
        if let Some((action, r)) = ctx.tree_action_button
            && rect_contains(r, col, row)
        {
            let cursor = ctx.current.tree.cursor;
            match action {
                TreeButtonAction::Buy => out.push(Action::TreeBuy(cursor)),
                TreeButtonAction::Refund => out.push(Action::TreeRefund(cursor)),
            }
            return;
        }
        for &(lot, r) in ctx.tree_node_rects {
            if rect_contains(r, col, row) {
                out.push(Action::TreeFocus(lot));
                if button == MouseButton::Right {
                    let owned = ctx.current.tree.bought.contains(&lot);
                    if owned {
                        // Try refund — silently rejected by the sim if
                        // it would orphan another owned node, so safe
                        // to emit unconditionally.
                        out.push(Action::TreeRefund(lot));
                    } else if ctx.current.can_buy_tree_node(lot) {
                        out.push(Action::TreeBuy(lot));
                    }
                }
                return;
            }
        }
        // Click on empty tree canvas: no-op. The MouseDown handler
        // above already started drag tracking, so a hold + move from
        // here will pan; a release without motion does nothing visible.
        let _ = mods;
        return;
    }

    // Powerups are catchable from ANY panel — match the keyboard 'g'
    // behavior, which has no mode guard. The marker still renders on the
    // biscuit while a non-Game panel is open. Right-click on a powerup
    // also catches.
    //
    // Each powerup is its own click target — clicking one does NOT vacuum
    // up an adjacent or overlapping powerup. Multiple of any kind coexist
    // freely; each one catches only itself, by spawn_id.
    for &(id, rect) in ctx.powerup_rects {
        if rect_contains(rect, col, row) {
            out.push(Action::CatchPowerup(id));
            return;
        }
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
    let _ = mods;
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
    if crate::ui::hands::occupied_at(col, row, ctx.biscuit_rect, ctx.biscuit_focal, ctx.current) {
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
        KeyCode::Char('t') | KeyCode::Char('T') => {
            ui.mode = if matches!(ui.mode, Mode::Tree) {
                Mode::Game
            } else {
                Mode::Tree
            };
        }
        // [g] catches the most-urgent powerup (lowest remaining life
        // ticks) across all four slots — Lucky, Frenzy, Buff, and
        // Green Coin. A second [g] press grabs the next-most-urgent.
        // Lets the player race against expiry without the keyboard
        // accidentally vacuuming up siblings.
        KeyCode::Char('g') | KeyCode::Char('G') => {
            push_grab_most_urgent(ctx, out);
        }
        // Debug/testing: gated by `debug`. See src/ui/debug_pane.rs for the
        // advertised key list. F8 (not F1) is Lucky because Chrome / Edge /
        // Safari hijack F1 for browser Help and never forward the keydown
        // to the wasm page; F8 has no default browser binding outside of
        // an open DevTools instance.
        KeyCode::F(8) if ctx.debug => {
            out.push(Action::DevForcePowerup(
                crate::game::powerup::PowerupKind::Lucky,
            ));
        }
        KeyCode::F(2) if ctx.debug => {
            out.push(Action::DevForcePowerup(
                crate::game::powerup::PowerupKind::Frenzy,
            ));
        }
        KeyCode::F(3) if ctx.debug => {
            out.push(Action::DevForcePowerup(
                crate::game::powerup::PowerupKind::Buff,
            ));
        }
        KeyCode::F(4) if ctx.debug => {
            out.push(Action::DevAddCuques(1_000_000.0));
        }
        KeyCode::F(5) if ctx.debug => {
            out.push(Action::DevForcePowerup(
                crate::game::powerup::PowerupKind::GreenCoin,
            ));
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
        // Tree-mode controls. Pan via hjkl or arrow keys; Enter buys the
        // focused node; R refunds the focused node; `0` jumps to the
        // root anchor, `1` jumps to the last bought node.
        KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Left if ui.mode == Mode::Tree => {
            let c = ctx.current.tree.cursor;
            out.push(Action::TreeFocus(TreeCoord::new(c.x - 1, c.y)));
        }
        KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Right if ui.mode == Mode::Tree => {
            let c = ctx.current.tree.cursor;
            out.push(Action::TreeFocus(TreeCoord::new(c.x + 1, c.y)));
        }
        KeyCode::Char('k') | KeyCode::Char('K') | KeyCode::Up if ui.mode == Mode::Tree => {
            let c = ctx.current.tree.cursor;
            out.push(Action::TreeFocus(TreeCoord::new(c.x, c.y - 1)));
        }
        KeyCode::Char('j') | KeyCode::Char('J') | KeyCode::Down if ui.mode == Mode::Tree => {
            let c = ctx.current.tree.cursor;
            out.push(Action::TreeFocus(TreeCoord::new(c.x, c.y + 1)));
        }
        KeyCode::Enter if ui.mode == Mode::Tree => {
            out.push(Action::TreeBuy(ctx.current.tree.cursor));
        }
        KeyCode::Char('r') | KeyCode::Char('R') if ui.mode == Mode::Tree => {
            out.push(Action::TreeRefund(ctx.current.tree.cursor));
        }
        KeyCode::Char('0') if ui.mode == Mode::Tree => {
            out.push(Action::TreeFocus(TreeCoord::ORIGIN));
        }
        KeyCode::Char('1') if ui.mode == Mode::Tree => {
            let target = ctx.current.tree.last_bought.unwrap_or(TreeCoord::ORIGIN);
            out.push(Action::TreeFocus(target));
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
                    // Tree mode handles its own digits (`0` and `1`) above
                    // — anything else is ignored.
                    _ => {
                        let _ = (slot, buy_10, buy_max);
                    }
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
    //! `state_with_lucky()` and `state_with_prestige()` mutate just enough
    //! of the default to exercise the gates the router cares about.
    use super::*;
    use crate::game::powerup::{Powerup, PowerupKind};
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
        powerup_rects: &'a [(u64, Rect)],
        play_area: Rect,
        prestige_reset_rect: Rect,
        fingerer_rows: &'a [(usize, Rect)],
        tree_node_rects: &'a [(crate::game::tree::coord::TreeCoord, Rect)],
        help_hits: &'a [(HelpAction, Rect)],
        debug: bool,
        current: &'a GameState,
    ) -> InputContext<'a> {
        InputContext {
            fingerer_rows,
            tree_node_rects,
            tree_action_button: None,
            help_hits,
            biscuit_rect: biscuit,
            biscuit_focal: (0, 0),
            powerup_rects,
            play_area,
            prestige_reset_rect,
            debug,
            current,
        }
    }

    fn empty_ctx<'a>(state: &'a GameState) -> InputContext<'a> {
        ctx(
            Rect::default(),
            &[],
            Rect::default(),
            Rect::default(),
            &[],
            &[],
            &[],
            false,
            state,
        )
    }

    /// State with a Lucky powerup queued so `[g]` / hit-test clicks have
    /// something to catch. Returns the `(state, spawn_id)` pair so tests
    /// that need to override `life_ticks` or hit-test can do so by id.
    fn state_with_lucky() -> (GameState, u64) {
        let mut s = GameState::default();
        let id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::Lucky,
            spawn_id: id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: PowerupKind::Lucky.lifetime_ticks(),
        });
        (s, id)
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
    fn g_with_no_powerup_is_silent() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &empty_ctx(&s), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn g_with_powerup_emits_catch() {
        let (s, id) = state_with_lucky();
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &empty_ctx(&s), &mut out);
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(spawn_id)] if *spawn_id == id),
            "expected CatchPowerup({id}), got {out:?}"
        );
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
        process_input_event(key(KeyCode::F(5)), &mut ui, &c, &mut out);
        assert!(out.is_empty(), "F-keys must be silent when debug=false");
    }

    #[test]
    fn fkeys_active_when_debug() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let c = ctx(
            Rect::default(),
            &[],
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
        process_input_event(key(KeyCode::F(5)), &mut ui, &c, &mut out);
        assert!(matches!(
            out[0],
            Action::DevForcePowerup(PowerupKind::Lucky)
        ));
        assert!(matches!(out[1], Action::DevAddCuques(_)));
        assert!(matches!(
            out[2],
            Action::DevForcePowerup(PowerupKind::GreenCoin)
        ));
    }

    // -- Digit shortcuts: modifier→BuyQty ----------------------------------

    fn fingerer_row_ctx<'a>(state: &'a GameState, rows: &'a [(usize, Rect)]) -> InputContext<'a> {
        ctx(
            Rect::default(),
            &[],
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
            &[],
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
            &[],
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
    fn left_click_on_powerup_emits_catch() {
        let (s, id) = state_with_lucky();
        let mut ui = UiState::new();
        let powerup_rects = [(id, rect(50, 12, 4, 2))];
        let c = ctx(
            Rect::default(),
            &powerup_rects,
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
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(spawn_id)] if *spawn_id == id),
            "got {out:?}"
        );
    }

    #[test]
    fn left_click_on_powerup_only_catches_one_under_cursor() {
        // Two powerups on screen; clicking the second's rect catches only
        // it. Multiple powerups coexist — clicking one doesn't vacuum up
        // any others, regardless of kind.
        let mut s = GameState::default();
        let lucky_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::Lucky,
            spawn_id: lucky_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 100,
        });
        let green_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::GreenCoin,
            spawn_id: green_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 100,
        });
        let mut ui = UiState::new();
        let powerup_rects = [
            (lucky_id, rect(50, 12, 5, 3)),
            (green_id, rect(70, 12, 5, 3)),
        ];
        let c = ctx(
            Rect::default(),
            &powerup_rects,
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
            mouse_down(72, 13, MouseButton::Left, Modifiers::default()),
            &mut ui,
            &c,
            &mut out,
        );
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(id)] if *id == green_id),
            "got {out:?}"
        );
    }

    #[test]
    fn g_with_two_kinds_picks_lower_life_ticks() {
        // `g` should grab the most-urgent powerup first when multiples are
        // on screen, regardless of kind. Min by `life_ticks`.
        let mut s = GameState::default();
        let lucky_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::Lucky,
            spawn_id: lucky_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 50,
        });
        let green_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::GreenCoin,
            spawn_id: green_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 200,
        });
        let c = empty_ctx(&s);
        let mut ui = UiState::new();
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &c, &mut out);
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(id)] if *id == lucky_id),
            "lower-life Lucky should win, got {out:?}"
        );

        // Flip the lifetimes — green is now the urgent one.
        let mut s = GameState::default();
        let lucky_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::Lucky,
            spawn_id: lucky_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 200,
        });
        let green_id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::GreenCoin,
            spawn_id: green_id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 50,
        });
        let c = empty_ctx(&s);
        let mut out = Vec::new();
        process_input_event(key(KeyCode::Char('g')), &mut ui, &c, &mut out);
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(id)] if *id == green_id),
            "lower-life GreenCoin should win, got {out:?}"
        );
    }

    #[test]
    fn right_click_on_powerup_also_catches() {
        // The marker is small and reflex right-clicks shouldn't waste it.
        let (s, id) = state_with_lucky();
        let mut ui = UiState::new();
        let powerup_rects = [(id, rect(50, 12, 4, 2))];
        let c = ctx(
            Rect::default(),
            &powerup_rects,
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
        assert!(
            matches!(out.as_slice(), [Action::CatchPowerup(spawn_id)] if *spawn_id == id),
            "got {out:?}"
        );
    }

    #[test]
    fn left_click_on_fingerer_row_buys_one() {
        let s = GameState::default();
        let mut ui = UiState::new();
        let rows = [(2_usize, rect(100, 5, 38, 3))];
        let c = ctx(
            Rect::default(),
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
            &[],
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
