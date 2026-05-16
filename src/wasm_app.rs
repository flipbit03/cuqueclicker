//! Browser entry point. Single-threaded; the sim ticks at 20Hz inside
//! the requestAnimationFrame callback via wall-clock catch-up against
//! `performance.now()`.
//!
//! Inputs from ratzilla's `KeyEvent` / `MouseEvent` / `WheelEvent` are
//! translated to platform-neutral [`crate::input::InputEvent`]s and fed
//! to the **same** [`crate::input::process_input_event`] the native
//! runner uses. Produced [`crate::sim::Action`]s are applied directly
//! via [`crate::sim::apply_action`] (no mpsc — there's only one stack).
//!
//! Persistence is the [`Persistence`] impl from `platform::web`
//! (localStorage). Save scheduling mirrors native's `SAVE_INTERVAL_TICKS`.
//! `InstanceLock` is a no-op; see `platform/web.rs` module docs.
//!
//! ## Mouse coords gotcha
//!
//! Ratzilla 0.3.0's mouse events ship `x` / `y` in *page pixels* (the
//! `clientX/Y` from the underlying `web_sys::MouseEvent`), not terminal
//! cells. We translate by reading the canvas `getBoundingClientRect()`
//! plus the cell pitch derived per-frame from `frame.area()` (cells)
//! vs the canvas client size (pixels). The pitch is recomputed every
//! frame so window resize stays in sync with hit-testing.
//!
//! ## Wheel events gotcha
//!
//! `ratzilla::WebRenderer::on_mouse_event` only listens for
//! `mousedown` / `mouseup` / `mousemove`; **wheel is not covered**.
//! We register `window.onwheel` directly and translate to
//! [`InputEvent::Wheel`] alongside the ratzilla-provided events.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::Terminal;
use ratzilla::backend::webgl2::{FontAtlasData, WebGl2BackendOptions};
use ratzilla::event::{
    KeyCode as RzKeyCode, KeyEvent as RzKeyEvent, MouseButton as RzMouseButton,
    MouseEvent as RzMouseEvent, MouseEventKind as RzMouseEventKind,
};
use ratzilla::{WebGl2Backend, WebRenderer};
use wasm_bindgen::prelude::*;

use crate::game::state::TICK_HZ;
use crate::input::{
    self, InputContext, InputEvent, KeyCode, Modifiers, MouseButton, UiState, WheelDelta,
};
use crate::platform::Persistence;
use crate::sim::{self, Action, SimGeometry};
use crate::ui;

/// 20Hz sim tick → 50ms per tick.
const SIM_TICK_MS: f64 = 1000.0 / TICK_HZ as f64;
/// Cap on how many ticks we'll catch up in one rAF callback after a
/// long pause (tab throttled / page hidden / OS sleep). Mirrors the
/// `MAX_TICK_CATCHUP` on native.
const MAX_TICK_CATCHUP: u32 = 20;
/// Save every `SAVE_INTERVAL_TICKS` sim ticks (1s at 20Hz). Tighter than
/// native's 10s interval because the browser has no graceful-shutdown
/// equivalent: the user can F5 / close-tab at any moment, and our
/// `beforeunload` flush is best-effort (some browsers throttle long
/// handlers). One save per second keeps the worst-case loss subjectively
/// small while keeping localStorage churn modest (a single stringify +
/// `setItem` per second).
const SAVE_INTERVAL_TICKS: u64 = TICK_HZ as u64;

/// Per-frame state that lives on the input/render side. Held in
/// `Rc<RefCell<…>>` so the rAF closure, key-event closure, mouse-event
/// closure, and wheel-event closure can all share it. Borrows never
/// overlap because JS callbacks aren't reentrant within a single task.
struct WebUi {
    ui: UiState,
    /// Single per-frame layout snapshot — same role as the native
    /// `App::layout`. `InputContext::from_layout` projects this into the
    /// per-event input ctx, so adding a new clickable region is one struct
    /// edit (in `DrawOutput`) and one projection edit (in `InputContext`)
    /// — no platform-side mirror to keep in sync.
    layout: crate::ui::DrawOutput,
    /// Wall-clock anchor for sim catch-up. Set on first rAF, advanced
    /// by `SIM_TICK_MS` per tick we actually run.
    last_tick_ms: f64,
    /// Tick counter for save scheduling.
    ticks_since_save: u64,
    /// Pixel pitch per cell, learned at draw-time from canvas size vs
    /// `frame.area()` and consumed by mouse-coord translation. Zero
    /// until the first frame renders.
    cell_pixel_w: f64,
    cell_pixel_h: f64,
}

impl WebUi {
    fn new() -> Self {
        Self {
            ui: UiState::new(),
            layout: Default::default(),
            last_tick_ms: now_ms(),
            ticks_since_save: 0,
            cell_pixel_w: 0.0,
            cell_pixel_h: 0.0,
        }
    }
}

#[wasm_bindgen(start)]
pub fn run() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    crate::i18n::init();

    // Override beamterm's default bitmap-font atlas with a JetBrains Mono
    // atlas baked from the regular TTF (15pt, 11×22 cells). Coverage:
    // ASCII (always), Latin-1 supplement (® ± · × ÷ + accented letters),
    // dashes / ellipsis / superscripts, arrows, math operators, box
    // drawing + block elements (ratatui's default border style needs
    // U+2500..U+257F). See `assets/jetbrains-mono.atlas`; the
    // regeneration command lives in `README.md`.
    let atlas = FontAtlasData::from_binary(include_bytes!("../assets/jetbrains-mono.atlas"))
        .map_err(|e| JsValue::from_str(&format!("FontAtlasData::from_binary failed: {e:?}")))?;
    let backend = WebGl2Backend::new_with_options(WebGl2BackendOptions::new().font_atlas(atlas))
        .map_err(|e| JsValue::from_str(&format!("WebGl2Backend::new failed: {e}")))?;
    let terminal = Terminal::new(backend)
        .map_err(|e| JsValue::from_str(&format!("Terminal::new failed: {e}")))?;

    let persistence = Persistence::new();
    let state = Rc::new(RefCell::new(persistence.load()));
    let web = Rc::new(RefCell::new(WebUi::new()));
    let geom = Rc::new(RefCell::new(SimGeometry::default()));
    // Reused per-event scratch buffer — mirrors the native pattern.
    // `process_input_event` appends actions; we drain into `apply_action`.
    let actions = Rc::new(RefCell::new(Vec::<Action>::with_capacity(4)));

    // --- Key listener -------------------------------------------------
    {
        let state = state.clone();
        let web = web.clone();
        let geom = geom.clone();
        let actions = actions.clone();
        terminal.on_key_event(move |k| {
            if let Some(ev) = translate_key(k) {
                dispatch(ev, &state, &web, &geom, &actions);
            }
        });
    }

    // --- Mouse listener (mousedown / mouseup / mousemove) -------------
    {
        let state = state.clone();
        let web = web.clone();
        let geom = geom.clone();
        let actions = actions.clone();
        terminal.on_mouse_event(move |m| {
            // CRITICAL: drop the `Ref` from `web.borrow()` BEFORE calling
            // `dispatch` (which `borrow_mut()`s `web`). If we inline the
            // `web.borrow()` into the `for` head, Rust's temporary
            // lifetime extension keeps the `Ref` alive for the whole
            // loop body — then `dispatch` panics on the existing borrow,
            // which aborts wasm and locks every subsequent input out.
            let evs = {
                let w = web.borrow();
                translate_mouse(m, &w)
            };
            for ev in evs {
                dispatch(ev, &state, &web, &geom, &actions);
            }
        });
    }

    // --- Wheel listener (ratzilla 0.3.0 doesn't surface this) ---------
    {
        let state = state.clone();
        let web = web.clone();
        let geom = geom.clone();
        let actions = actions.clone();
        let closure = Closure::<dyn FnMut(_)>::new(move |e: web_sys::WheelEvent| {
            // Block the default browser scroll so wheel-zoom doesn't also
            // pan the page when the canvas is in a scrollable container.
            e.prevent_default();
            let (col, row) = pixel_to_cell(e.client_x() as u32, e.client_y() as u32, &web.borrow())
                .unwrap_or((0, 0));
            let delta = if e.delta_y() < 0.0 {
                WheelDelta::Up
            } else if e.delta_y() > 0.0 {
                WheelDelta::Down
            } else {
                return;
            };
            dispatch(
                InputEvent::Wheel { col, row, delta },
                &state,
                &web,
                &geom,
                &actions,
            );
        });
        web_sys::window()
            .ok_or_else(|| JsValue::from_str("no window"))?
            .set_onwheel(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
    }

    // --- Unload save (F5 / close-tab / nav-away) ----------------------
    // Browsers fire `beforeunload` when the user refreshes, closes, or
    // navigates. localStorage `setItem` is synchronous so it'll complete
    // within the unload window — meaning F5 loses at most the deltas
    // accumulated since the last periodic 1Hz save (and usually nothing,
    // because the periodic save runs every 20 ticks anyway). Some
    // browsers skip `beforeunload` when the page is bfcache-eligible;
    // `pagehide` covers that case (Safari especially), so we register
    // both to maximize the window during which we get a final flush.
    {
        let state = state.clone();
        let flush = Closure::<dyn FnMut(_)>::new(move |_e: web_sys::Event| {
            let p = Persistence::new();
            // `try_borrow` keeps us safe if a tick is mid-flight on the
            // sim loop (single-threaded JS makes this unlikely, but
            // unload may fire from inside a microtask boundary).
            if let Ok(s) = state.try_borrow() {
                let _ = p.save(&s);
            }
        });
        let win = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        win.add_event_listener_with_callback("beforeunload", flush.as_ref().unchecked_ref())
            .map_err(|e| {
                JsValue::from_str(&format!("addEventListener beforeunload failed: {e:?}"))
            })?;
        win.add_event_listener_with_callback("pagehide", flush.as_ref().unchecked_ref())
            .map_err(|e| JsValue::from_str(&format!("addEventListener pagehide failed: {e:?}")))?;
        flush.forget();
    }

    // --- Render + tick loop -------------------------------------------
    let s = state.clone();
    let w = web.clone();
    let g = geom.clone();
    terminal.draw_web(move |f| {
        let mut state = s.borrow_mut();
        let mut web = w.borrow_mut();
        let mut geom = g.borrow_mut();

        // Catch-up sim ticks against wall-clock. After tab throttling /
        // page sleep we may be many ticks behind; cap so we don't grind.
        let now = now_ms();
        let mut catchup = 0u32;
        while now - web.last_tick_ms >= SIM_TICK_MS {
            sim::sim_tick(&mut state, &geom);
            web.last_tick_ms += SIM_TICK_MS;
            web.ticks_since_save += 1;
            if web.ticks_since_save >= SAVE_INTERVAL_TICKS {
                web.ticks_since_save = 0;
                let _ = persistence.save(&state);
            }
            catchup += 1;
            if catchup >= MAX_TICK_CATCHUP {
                web.last_tick_ms = now;
                break;
            }
        }

        // Render. The geometry the input router hit-tests against is
        // sourced from this draw's `DrawOutput`.
        let area = f.area();
        // Mirror native's gate: `is_dev_build()` returns true only when
        // `Cargo.toml` is still at the placeholder `0.0.0`. The Pages
        // deploy workflow sed-patches that to the real version on
        // `release: published`, so a tagged release build flips this
        // off and the debug pane / F-key cheats vanish exactly like a
        // shipped native binary.
        let debug = crate::build_info::is_dev_build();
        web.layout = ui::draw(
            f,
            &state,
            web.ui.mode,
            web.ui.zoom_idx,
            debug,
            web.ui.last_mouse_pos,
            &mut web.ui.tree_render,
            web.ui.prestige_confirm_pending,
        );
        // Hand the latest biscuit rect to the sim so goldens and auto-
        // particles spawn inside the current layout. Powerup engine pauses
        // while a full-screen modal (tree) is open.
        geom.biscuit = web.layout.biscuit_rect;
        geom.powerups_paused = web.ui.mode == crate::ui::Mode::Tree;

        // Re-derive the cell pitch from canvas size vs grid cells.
        //
        // beamterm's atlas computes `area = buffer_px / atlas_cell_px`
        // with INTEGER FLOOR division, so the cells render in the
        // top-left `area * atlas_cell_px` region of the buffer with up
        // to `atlas_cell_px - 1` unused pixels on the right and bottom.
        // Naively dividing `rect / area` (the obvious cell-size formula)
        // smears that gutter back into each cell — the per-row error is
        // small but cumulative, so by the bottom of the screen the click
        // hit-test lands a full cell below where it should. Inverting
        // the floor-divide via `(buffer_px / area).floor()` recovers the
        // true atlas cell size, then we scale back to CSS pixels in case
        // the browser sub-pixel-stretches the buffer to the display rect
        // (fractional DPR + non-integer body content height).
        if area.width > 0
            && area.height > 0
            && let Some(canvas) = canvas_element()
        {
            let rect = canvas.get_bounding_client_rect();
            let buf_w = canvas.width() as f64;
            let buf_h = canvas.height() as f64;
            let atlas_w = (buf_w / area.width as f64).floor();
            let atlas_h = (buf_h / area.height as f64).floor();
            // Convert atlas cell size from buffer-px to CSS-px so it
            // pairs cleanly with `clientX`/`clientY` from mouse events.
            web.cell_pixel_w = if buf_w > 0.0 {
                atlas_w * rect.width() / buf_w
            } else {
                atlas_w
            };
            web.cell_pixel_h = if buf_h > 0.0 {
                atlas_h * rect.height() / buf_h
            } else {
                atlas_h
            };
        }
    });

    Ok(())
}

/// Apply one platform-neutral event: route through the shared input
/// router, drain produced actions through `apply_action`, leave UiState
/// updates in place. Borrows are nested inside the function so each
/// callback closure can call us without holding overlapping locks.
fn dispatch(
    ev: InputEvent,
    state: &Rc<RefCell<crate::game::state::GameState>>,
    web: &Rc<RefCell<WebUi>>,
    geom: &Rc<RefCell<SimGeometry>>,
    actions: &Rc<RefCell<Vec<Action>>>,
) {
    let mut state = state.borrow_mut();
    let mut web_ref = web.borrow_mut();
    let mut geom = geom.borrow_mut();
    let mut actions = actions.borrow_mut();

    // Split-borrow `web` so `process_input_event` gets `&mut ui` while
    // `InputContext::from_layout` reads `&layout` from the same struct.
    // Rust's disjoint-field rule allows this only via explicit
    // destructuring.
    let WebUi { ui, layout, .. } = &mut *web_ref;
    let ctx = InputContext::from_layout(layout, &state, crate::build_info::is_dev_build());
    actions.clear();
    input::process_input_event(ev, ui, &ctx, &mut actions);
    for a in actions.drain(..) {
        sim::apply_action(&mut state, a, &mut geom);
    }
}

// --- ratzilla → InputEvent translation ----------------------------------

fn translate_key(k: RzKeyEvent) -> Option<InputEvent> {
    let code = match k.code {
        RzKeyCode::Char(c) => KeyCode::Char(c),
        RzKeyCode::Esc => KeyCode::Esc,
        RzKeyCode::F(n) => KeyCode::F(n),
        RzKeyCode::Up => KeyCode::Up,
        RzKeyCode::Down => KeyCode::Down,
        RzKeyCode::Left => KeyCode::Left,
        RzKeyCode::Right => KeyCode::Right,
        RzKeyCode::Enter => KeyCode::Enter,
        _ => return None,
    };
    Some(InputEvent::KeyPress {
        code,
        mods: Modifiers {
            shift: k.shift,
            alt: k.alt,
            ctrl: k.ctrl,
        },
    })
}

/// Translate one ratzilla `MouseEvent`. Returns at most two events when
/// a button-down also implies a position update (we want both the click
/// dispatched AND `last_mouse_pos` set so hover highlighting tracks).
fn translate_mouse(m: RzMouseEvent, web: &WebUi) -> Vec<InputEvent> {
    let Some((col, row)) = pixel_to_cell(m.x, m.y, web) else {
        return Vec::new();
    };
    let mods = Modifiers {
        shift: m.shift,
        alt: m.alt,
        ctrl: m.ctrl,
    };
    match m.event {
        // We treat `Pressed` as the canonical "click" — same as native's
        // `Down`. Released/Unidentified are dropped.
        RzMouseEventKind::Pressed => {
            let button = match m.button {
                RzMouseButton::Left => MouseButton::Left,
                RzMouseButton::Right => MouseButton::Right,
                // Drop middle/back/forward/unidentified to match native.
                _ => return Vec::new(),
            };
            vec![InputEvent::MouseDown {
                col,
                row,
                button,
                mods,
            }]
        }
        RzMouseEventKind::Released => {
            let button = match m.button {
                RzMouseButton::Left => MouseButton::Left,
                RzMouseButton::Right => MouseButton::Right,
                _ => return Vec::new(),
            };
            vec![InputEvent::MouseUp { col, row, button }]
        }
        RzMouseEventKind::Moved => vec![InputEvent::MouseMoved { col, row }],
        _ => Vec::new(),
    }
}

// --- Helpers ----------------------------------------------------------

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// First `<canvas>` in the document. `WebGl2Backend` appends one canvas
/// to `<body>` on construction; tag-name lookup beats id juggling.
fn canvas_element() -> Option<web_sys::HtmlCanvasElement> {
    let doc = web_sys::window()?.document()?;
    let elem = doc.query_selector("canvas").ok().flatten()?;
    elem.dyn_into::<web_sys::HtmlCanvasElement>().ok()
}

/// Translate page-pixel coords (clientX/Y from a `web_sys::MouseEvent`
/// or `WheelEvent`) to terminal grid `(col, row)`. Returns `None` if the
/// canvas isn't mounted yet or the cell pitch hasn't been measured.
fn pixel_to_cell(x: u32, y: u32, web: &WebUi) -> Option<(u16, u16)> {
    if web.cell_pixel_w <= 0.0 || web.cell_pixel_h <= 0.0 {
        return None;
    }
    let canvas = canvas_element()?;
    let rect = canvas.get_bounding_client_rect();
    let col_f = (x as f64 - rect.left()) / web.cell_pixel_w;
    let row_f = (y as f64 - rect.top()) / web.cell_pixel_h;
    if col_f < 0.0 || row_f < 0.0 {
        return None;
    }
    Some((col_f.floor() as u16, row_f.floor() as u16))
}
