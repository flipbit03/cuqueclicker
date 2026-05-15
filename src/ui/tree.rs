//! Full-screen upgrade-tree modal renderer.
//!
//! Layout (top to bottom):
//! - Header (3 rows): title, cuques, cursor coord, owned count.
//! - Canvas (variable): the pannable infinite tree, centered on the cursor.
//! - Info pane (8 rows): focused node's title + primitives + cost + action hints.
//!
//! Bottom `help_height` rows (the help bar `ui::mod.rs` draws first) are
//! intentionally left untouched.
//!
//! Coordinate spaces:
//! - `lot (lx, ly)`: integer coords on the infinite tree grid.
//! - `canvas (cx, cy)`: cell coords on the same infinite grid where boxes
//!   live (each lot occupies `LOT_W × LOT_H` cells).
//! - `screen (sx, sy)`: actual terminal cell coords.
//!
//! Conversion: `screen = canvas - pan + canvas_origin`.

use ratatui::{prelude::*, style::Modifier as StyleMod, widgets::*};

use crate::format;
use crate::game::powerup::PowerupKind;
use crate::game::state::{
    GameState, PURCHASE_FLASH_TICKS, TREE_REFUND_FRACTION, UNLOCK_FLASH_TICKS,
};
use crate::game::tree::aggregate::TreeAggregate;
use crate::game::tree::coord::TreeCoord;
use crate::game::tree::naming::primitive_blurb;
use crate::game::tree::node::{self, LOT_H, LOT_W, NodeSpec, Rarity};
use crate::game::tree::primitive::{Op, Primitive, Target};
use crate::i18n::t;
use crate::input::TreeRenderState;
use crate::ui::{TreeButtonAction, border, hud_title};

/// Tree-modal renderer output. Carries the same per-node click rects the
/// modal has always exposed, plus an optional clickable action button
/// (Buy or Refund) for the focused node — that's the left-click target
/// for touch / single-button players.
pub struct TreeDrawOutput {
    pub node_rects: Vec<(TreeCoord, Rect)>,
    pub action_button: Option<(TreeButtonAction, Rect)>,
}

/// Per-frame ease factor. The displayed pan moves `PAN_TWEEN_FACTOR` of the
/// remaining distance to the target per render frame — small enough to
/// feel smooth, big enough that a multi-lot jump arrives in well under a
/// second at 60Hz.
const PAN_TWEEN_FACTOR: f32 = 0.20;
/// Snap threshold: once the pan is within this many cells of the target,
/// jump exactly to the target so subpixel residuals don't blur the
/// rendering forever.
const PAN_SNAP_EPSILON: f32 = 0.5;

/// How many lots (in each direction from the cursor) to consider for
/// rendering. Conservative — anything outside this radius is fog regardless
/// of edges.
const VISIBLE_RADIUS_LOTS: i32 = 6;

/// Help bar height reserved at the bottom — `ui::mod.rs` rendered the help
/// text there before we got the frame, so we leave those rows alone.
const HELP_BAR_HEIGHT: u16 = 2;

const INFO_PANE_HEIGHT: u16 = 8;
const HEADER_HEIGHT: u16 = 3;

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
    mouse_pos: Option<(u16, u16)>,
    tree_render: &mut TreeRenderState,
) -> TreeDrawOutput {
    let modal_h = area.height.saturating_sub(HELP_BAR_HEIGHT);
    if modal_h < HEADER_HEIGHT + INFO_PANE_HEIGHT + 4 {
        // Terminal too small; bail out gracefully.
        return TreeDrawOutput {
            node_rects: Vec::new(),
            action_button: None,
        };
    }
    let modal = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: modal_h,
    };

    // Clear the modal area so the biscuit / sidebar / HUD beneath don't
    // bleed through. We paint every cell with a space at the resting bg.
    fill_area(frame, modal);

    let rows = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(1),
        Constraint::Length(INFO_PANE_HEIGHT),
    ])
    .split(modal);
    let header_area = rows[0];
    let canvas_area = rows[1];
    let info_area = rows[2];

    draw_header(frame, header_area, state);
    let node_rects = draw_canvas(frame, canvas_area, state, mouse_pos, tree_render);
    let action_button = draw_info_pane(frame, info_area, state);
    // Hover-fill the info-pane action button (buy / refund) so the player
    // reads it as a clickable target. Matches the help-bar token hover:
    // white fg + dark bg tint + bold.
    if let (Some((_, r)), Some((mx, my))) = (action_button, mouse_pos)
        && mx >= r.x
        && mx < r.x + r.width
        && my >= r.y
        && my < r.y + r.height
    {
        let buf = frame.buffer_mut();
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(
                        cell.style()
                            .fg(Color::Rgb(255, 255, 255))
                            .bg(Color::Rgb(40, 40, 50))
                            .add_modifier(StyleMod::BOLD),
                    );
                }
            }
        }
    }
    TreeDrawOutput {
        node_rects,
        action_button,
    }
}

fn fill_area(frame: &mut Frame, area: Rect) {
    // Reset bg to terminal default so the player's $BG_COLOR shows through —
    // the rest of the app (HUD title, sidebar, biscuit area) lets the
    // terminal bg bleed through, and the tree modal should match. We still
    // need to overpaint every cell with a space so the biscuit / sidebar /
    // HUD text drawn before us is wiped.
    let buf = frame.buffer_mut();
    let clear = Style::default().fg(Color::Reset).bg(Color::Reset);
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_style(clear);
            }
        }
    }
}

// ---------- Header ----------------------------------------------------------

fn draw_header(frame: &mut Frame, area: Rect, state: &GameState) {
    // Casino-style animated border + the SAME title the main HUD shows.
    // The "— Upgrade Tree" suffix lets the player tell at a glance which
    // mode they're in while the chrome stays continuous with the rest of
    // the app.
    let lang = t();
    let title = format!("{}—{}", hud_title(), lang.tree_title);
    border::draw_animated(frame, area, state, &title);
    if area.width < 3 || area.height < 3 {
        return;
    }
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let cursor = state.tree.cursor;
    let line = Line::from(vec![
        Span::raw(format!("{}: ", lang.hud_cuques)),
        Span::styled(
            format::big(state.displayed_cuques),
            Style::default()
                .fg(Color::Rgb(180, 255, 180))
                .add_modifier(StyleMod::BOLD),
        ),
        Span::raw(format!("   {}: ", lang.hud_fps)),
        Span::styled(
            format::rate(state.displayed_fps),
            Style::default().fg(Color::Rgb(200, 200, 240)),
        ),
        Span::styled(
            format!("    cursor ({:+}, {:+})", cursor.x, cursor.y),
            Style::default().fg(Color::Rgb(180, 200, 255)),
        ),
        Span::styled(
            // Owned count excludes the anchor — it's an always-on cuque,
            // not something the player "bought". Subtracting it makes the
            // count read "buys made", which matches the player's mental
            // model.
            format!(
                "    owned: {}",
                state
                    .tree
                    .bought
                    .iter()
                    .filter(|c| **c != TreeCoord::ORIGIN)
                    .count()
            ),
            Style::default().fg(Color::Rgb(220, 220, 180)),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

// ---------- Canvas ----------------------------------------------------------

#[derive(Clone)]
struct VisibleNode {
    spec_box_x: i32,
    spec_box_y: i32,
    box_w: u16,
    box_h: u16,
    rarity: Rarity,
    lot: TreeCoord,
    dominant_target: Target,
    cost: f64,
    owned: bool,
    reachable: bool,
    affordable: bool,
    is_anchor: bool,
    title_short: String,
    /// Active flash strengths in [0.0, 1.0]. Green = just bought; yellow =
    /// just unlocked by another buy; red = just refunded.
    buy_flash: f32,
    unlock_flash: f32,
    refund_flash: f32,
}

fn draw_canvas(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
    mouse_pos: Option<(u16, u16)>,
    tree_render: &mut TreeRenderState,
) -> Vec<(TreeCoord, Rect)> {
    if area.width < 10 || area.height < 5 {
        return Vec::new();
    }

    let cursor = state.tree.cursor;
    let canvas_center_x = (area.width / 2) as i32;
    let canvas_center_y = (area.height / 2) as i32;

    // Target pan: keep cursor centered. Updated whenever the cursor moves
    // — keyboard nav (TreeFocus), bookmark jumps, and node clicks all flow
    // through `state.tree.cursor`. Drag DOESN'T re-center; it directly
    // modified `pan_*` AND `target_pan_*` in the input router, so when
    // dragging ends the camera stays where the player dragged it.
    let cursor_centered_x = (cursor.x * LOT_W + LOT_W / 2 - canvas_center_x) as f32;
    let cursor_centered_y = (cursor.y * LOT_H + LOT_H / 2 - canvas_center_y) as f32;

    if !tree_render.initialized {
        // First frame of this modal session: snap to target, don't tween.
        tree_render.pan_x = cursor_centered_x;
        tree_render.pan_y = cursor_centered_y;
        tree_render.target_pan_x = cursor_centered_x;
        tree_render.target_pan_y = cursor_centered_y;
        tree_render.prev_cursor = cursor;
        tree_render.initialized = true;
    } else {
        if cursor != tree_render.prev_cursor {
            // Cursor moved (keyboard nav, bookmark jump, click-to-focus).
            // Recenter the target; the displayed pan eases toward it.
            tree_render.target_pan_x = cursor_centered_x;
            tree_render.target_pan_y = cursor_centered_y;
            tree_render.prev_cursor = cursor;
        }
        // Tween only when not actively dragging — drag updates pan_* and
        // target_pan_* together so tweening would be a no-op anyway, but
        // explicitly skipping clarifies intent.
        if tree_render.drag_last.is_none() {
            let dx = tree_render.target_pan_x - tree_render.pan_x;
            let dy = tree_render.target_pan_y - tree_render.pan_y;
            if dx.abs() < PAN_SNAP_EPSILON && dy.abs() < PAN_SNAP_EPSILON {
                tree_render.pan_x = tree_render.target_pan_x;
                tree_render.pan_y = tree_render.target_pan_y;
            } else {
                tree_render.pan_x += dx * PAN_TWEEN_FACTOR;
                tree_render.pan_y += dy * PAN_TWEEN_FACTOR;
            }
        }
    }

    // Round to integer cells for actual rendering — subpixel positions
    // would render fractionally and look blurry.
    let pan_x = tree_render.pan_x.round() as i32;
    let pan_y = tree_render.pan_y.round() as i32;

    // Harvest is centered on the lot currently under the VIEWPORT center,
    // not the cursor lot. Drag-panning moves the camera without moving the
    // cursor; anchoring the harvest to the cursor would leave the
    // dragged-into edge unpopulated until the player clicked something to
    // re-focus. With viewport-anchored harvest the tree grows naturally
    // ahead of the camera, matching the "infinite" expectation.
    let view_center_canvas_x = pan_x + canvas_center_x;
    let view_center_canvas_y = pan_y + canvas_center_y;
    let view_lot_x = view_center_canvas_x.div_euclid(LOT_W);
    let view_lot_y = view_center_canvas_y.div_euclid(LOT_H);
    let visible_radius = VISIBLE_RADIUS_LOTS + 2;
    let mut visible: Vec<VisibleNode> = Vec::new();
    for dy in -visible_radius..=visible_radius {
        for dx in -visible_radius..=visible_radius {
            let lot = TreeCoord::new(view_lot_x + dx, view_lot_y + dy);
            let Some(spec) = node::node_at(lot.x, lot.y) else {
                continue;
            };
            let owned = state.tree.bought.contains(&lot);
            // While a wavefront is still walking toward this lot, hold it
            // in its "unreachable" look — the box only flips reachable
            // once the path finishes energizing.
            let reachable = !owned && state.tree_reachable(lot) && !state.tree_unlock_pending(lot);
            let affordable = state.affordable_cuques() >= spec.cost;
            let title_short = truncate(&spec.title, (spec.box_w as usize).saturating_sub(2));
            let buy_flash = state
                .tree_buy_flash
                .get(&lot)
                .copied()
                .map(|t| t as f32 / PURCHASE_FLASH_TICKS as f32)
                .unwrap_or(0.0);
            let unlock_flash = state
                .tree_unlock_flash
                .get(&lot)
                .copied()
                .map(|t| t as f32 / UNLOCK_FLASH_TICKS as f32)
                .unwrap_or(0.0);
            let refund_flash = state
                .tree_refund_flash
                .get(&lot)
                .copied()
                .map(|t| t as f32 / PURCHASE_FLASH_TICKS as f32)
                .unwrap_or(0.0);
            visible.push(VisibleNode {
                spec_box_x: spec.box_x,
                spec_box_y: spec.box_y,
                box_w: spec.box_w,
                box_h: spec.box_h,
                rarity: spec.rarity,
                lot,
                dominant_target: spec.dominant_target,
                cost: spec.cost,
                owned,
                reachable,
                affordable,
                is_anchor: spec.is_anchor,
                title_short,
                buy_flash,
                unlock_flash,
                refund_flash,
            });
        }
    }

    // Edges: pairs of visible lots that have a procedural edge.
    let edges: Vec<(TreeCoord, TreeCoord)> = {
        let mut out = Vec::new();
        for i in 0..visible.len() {
            for j in (i + 1)..visible.len() {
                let a = visible[i].lot;
                let b = visible[j].lot;
                if node::edge_exists(a, b) {
                    out.push((a, b));
                }
            }
        }
        out
    };

    // Draw edges first so boxes overpaint their junctions. The path
    // glyphs at line termini use HALF-HEAVY box-drawing chars
    // (`╿ ╽ ╼ ╾`) — heavier on the side facing the endpoint box, which
    // most monospace fonts render slightly thicker, closing the visual
    // half-cell gap between a `│` line and the box's `─` border
    // without needing T-junctions or compromising the box outline.
    for (a, b) in &edges {
        let av = visible.iter().find(|v| v.lot == *a).cloned();
        let bv = visible.iter().find(|v| v.lot == *b).cloned();
        if let (Some(av), Some(bv)) = (av, bv) {
            draw_edge(frame, area, pan_x, pan_y, &av, &bv, state);
        }
    }

    let mut click_rects: Vec<(TreeCoord, Rect)> = Vec::new();
    for v in &visible {
        if let Some(r) = draw_box(
            frame,
            area,
            pan_x,
            pan_y,
            v,
            cursor,
            &state.tree_aggregate,
            state,
        ) {
            click_rects.push((v.lot, r));
        }
    }

    // Mouse hover: brighten the box under the cursor.
    if let Some((mx, my)) = mouse_pos {
        for &(_, r) in &click_rects {
            if mx >= r.x && mx < r.x + r.width && my >= r.y && my < r.y + r.height {
                hover_lift(frame, r);
                break;
            }
        }
    }

    click_rects
}

/// Map a canvas-grid cell to a screen cell. Returns None if outside the
/// drawable rect.
fn canvas_to_screen(area: Rect, pan_x: i32, pan_y: i32, cx: i32, cy: i32) -> Option<(u16, u16)> {
    let sx = cx - pan_x + area.x as i32;
    let sy = cy - pan_y + area.y as i32;
    if sx < area.x as i32
        || sx >= (area.x as i32 + area.width as i32)
        || sy < area.y as i32
        || sy >= (area.y as i32 + area.height as i32)
    {
        return None;
    }
    Some((sx as u16, sy as u16))
}

#[allow(clippy::too_many_arguments)]
fn draw_box(
    frame: &mut Frame,
    area: Rect,
    pan_x: i32,
    pan_y: i32,
    v: &VisibleNode,
    cursor: TreeCoord,
    tree_agg: &TreeAggregate,
    state: &GameState,
) -> Option<Rect> {
    // Anchor (the cuque-sprite at origin) renders as a small ass instead
    // of a regular box. It's auto-owned, has no primitives, and is the
    // seed point for the rest of the tree.
    if v.is_anchor {
        return draw_anchor(frame, area, pan_x, pan_y, v, cursor, state);
    }
    let buf = frame.buffer_mut();
    let bx = v.spec_box_x;
    let by = v.spec_box_y;
    let bw = v.box_w as i32;
    let bh = v.box_h as i32;

    // Box drawing chars: pick line set + style by node state.
    let (corners, h_char, v_char, base_style) = box_chars_for(v);

    // Cursor-on-this-lot: bold + bright tint over the regular palette.
    let is_focus = v.lot == cursor;
    let box_style = if is_focus {
        base_style
            .add_modifier(StyleMod::BOLD)
            .add_modifier(StyleMod::REVERSED)
    } else {
        base_style
    };

    // Top, bottom borders + sides.
    let mut painted_any = false;
    let mut min_sx = u16::MAX;
    let mut min_sy = u16::MAX;
    let mut max_sx = 0u16;
    let mut max_sy = 0u16;

    for row in 0..bh {
        for col in 0..bw {
            let cx = bx + col;
            let cy = by + row;
            let Some((sx, sy)) = canvas_to_screen(area, pan_x, pan_y, cx, cy) else {
                continue;
            };
            let ch = if row == 0 && col == 0 {
                corners.0
            } else if row == 0 && col == bw - 1 {
                corners.1
            } else if row == bh - 1 && col == 0 {
                corners.2
            } else if row == bh - 1 && col == bw - 1 {
                corners.3
            } else if row == 0 || row == bh - 1 {
                h_char
            } else if col == 0 || col == bw - 1 {
                v_char
            } else {
                // Interior: render title + cost on appropriate rows. Leave
                // anything we don't have content for as a space (overwriting
                // edges or fill).
                let interior_w = (bw - 2) as usize;
                let r_in = row - 1;
                if r_in == 0 && interior_w > 0 {
                    // Title row.
                    v.title_short.chars().nth((col - 1) as usize).unwrap_or(' ')
                } else if r_in == (bh - 2) - 1 && interior_w > 4 {
                    // Cost row near bottom-interior. Render "Cost: X" or
                    // glyphs as needed.
                    let cost_str = if v.owned {
                        "[ owned ]".to_string()
                    } else {
                        format::big(v.cost).to_string()
                    };
                    cost_str.chars().nth((col - 1) as usize).unwrap_or(' ')
                } else if r_in > 0
                    && r_in < (bh - 2)
                    && v.rarity != Rarity::Small
                    && r_in == 1
                    && interior_w > 4
                {
                    // Mid row for Notable/Keystone: a primitive summary count.
                    let summary = primitive_summary(v);
                    summary.chars().nth((col - 1) as usize).unwrap_or(' ')
                } else {
                    ' '
                }
            };

            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_char(ch);
                cell.set_style(box_style);
            }
            painted_any = true;
            min_sx = min_sx.min(sx);
            min_sy = min_sy.min(sy);
            max_sx = max_sx.max(sx);
            max_sy = max_sy.max(sy);
        }
    }

    let _ = tree_agg;

    if !painted_any {
        return None;
    }
    Some(Rect {
        x: min_sx,
        y: min_sy,
        width: max_sx.saturating_sub(min_sx).saturating_add(1),
        height: max_sy.saturating_sub(min_sy).saturating_add(1),
    })
}

/// Paint the cuque-sprite anchor at the origin lot — borrows
/// `BISCUIT_TINY` for visual continuity with the main game. The asshole
/// cell gets an `O` overlay so the ass reads as a cuque, not a generic
/// outline.
///
/// Unfocused: warm white-gold, no fanciness — the ass is the seed point,
/// not a button to click. Focused: per-cell **plasma shimmer** instead
/// of the usual REVERSED inversion (which made the gold cuque look
/// black-on-yellow and read as broken). The shimmer cycles through the
/// game's already-warm tints (gold → red → purple → back), driven by
/// `state.steady_phase` so the speed matches the rest of the app.
fn draw_anchor(
    frame: &mut Frame,
    area: Rect,
    pan_x: i32,
    pan_y: i32,
    v: &VisibleNode,
    cursor: TreeCoord,
    state: &GameState,
) -> Option<Rect> {
    let rows = crate::ui::biscuit::BISCUIT_TINY;
    let focal = crate::ui::biscuit::BISCUIT_TINY_FOCAL;
    let is_focus = v.lot == cursor;
    let phase = state.steady_phase;

    // Warm white-gold base. Used directly when unfocused; replaced per-cell
    // with a plasma shimmer when focused.
    let base_style = Style::default()
        .fg(Color::Rgb(255, 230, 180))
        .add_modifier(StyleMod::BOLD);

    let buf = frame.buffer_mut();
    let mut min_sx = u16::MAX;
    let mut min_sy = u16::MAX;
    let mut max_sx = 0u16;
    let mut max_sy = 0u16;
    let mut painted_any = false;

    for (row_idx, line) in rows.iter().enumerate() {
        for (col_idx, ch) in line.chars().enumerate() {
            // Skip pure-space cells outside the art outline — leaving
            // them as the cleared modal bg makes the anchor blend
            // naturally with whatever terminal bg the user runs.
            let is_focal = col_idx as u16 == focal.0 && row_idx as u16 == focal.1;
            let glyph = if is_focal { 'O' } else { ch };
            if glyph == ' ' {
                continue;
            }
            let cx = v.spec_box_x + col_idx as i32;
            let cy = v.spec_box_y + row_idx as i32;
            let Some((sx, sy)) = canvas_to_screen(area, pan_x, pan_y, cx, cy) else {
                continue;
            };
            let style = if is_focus {
                Style::default()
                    .fg(plasma_color(phase, col_idx as i32, row_idx as i32))
                    .add_modifier(StyleMod::BOLD)
            } else {
                base_style
            };
            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_char(glyph);
                cell.set_style(style);
            }
            painted_any = true;
            min_sx = min_sx.min(sx);
            min_sy = min_sy.min(sy);
            max_sx = max_sx.max(sx);
            max_sy = max_sy.max(sy);
        }
    }

    if !painted_any {
        return None;
    }
    Some(Rect {
        x: min_sx,
        y: min_sy,
        width: max_sx.saturating_sub(min_sx).saturating_add(1),
        height: max_sy.saturating_sub(min_sy).saturating_add(1),
    })
}

/// Per-cell plasma shimmer for the focused-anchor effect. Sums a few sine
/// waves into a `[0, 1]` parameter `n`, then interpolates that around a
/// warm 3-keyframe color loop (gold → red-amber → purple → gold). Each
/// cell offsets in space (col, row) so the pattern drifts diagonally
/// across the cuque, and the `phase` time-shifts the whole field every
/// tick.
fn plasma_color(phase: u32, col: i32, row: i32) -> Color {
    let t = phase as f32 * 0.06;
    let cx = col as f32 * 0.45;
    let cy = row as f32 * 0.85;
    let v = (cx + t).sin() + (cy + t * 1.27).sin() + ((cx + cy) * 0.6 + t * 0.83).sin();
    let n = ((v / 3.0) + 1.0) * 0.5; // map [-1, 1] → [0, 1]

    // Three warm keyframes from the game's existing palette.
    let keys: [(f32, f32, f32); 3] = [
        (255.0, 215.0, 110.0), // Lucky-ish gold
        (255.0, 110.0, 90.0),  // warm Frenzy red
        (210.0, 130.0, 255.0), // Buff purple
    ];
    let pos = n * 3.0;
    let idx = (pos.floor() as usize) % 3;
    let frac = pos - pos.floor();
    let a = keys[idx];
    let b = keys[(idx + 1) % 3];
    let r = a.0 + (b.0 - a.0) * frac;
    let g = a.1 + (b.1 - a.1) * frac;
    let bb = a.2 + (b.2 - a.2) * frac;
    Color::Rgb(
        r.clamp(60.0, 255.0) as u8,
        g.clamp(60.0, 255.0) as u8,
        bb.clamp(60.0, 255.0) as u8,
    )
}

fn primitive_summary(v: &VisibleNode) -> String {
    // Very short tag describing rarity + dominant target. Fits inside a
    // Notable's middle row (~16 chars).
    let prefix = match v.rarity {
        Rarity::Small => "small",
        Rarity::Notable => "notable",
        Rarity::Keystone => "KEYSTONE",
    };
    let target = match v.dominant_target {
        Target::Fingerer(i) => crate::i18n::t()
            .fingerer_names
            .get(i as usize)
            .copied()
            .unwrap_or("?")
            .to_string(),
        Target::AllFingerers => "all fingerers".to_string(),
        Target::Click => "click".to_string(),
        Target::PowerupSpawn(k) => format!("{} spawn", powerup_short(k)),
        Target::PowerupReward(k) => format!("{} reward", powerup_short(k)),
        Target::PowerupDuration(k) => format!("{} time", powerup_short(k)),
        Target::Prestige => "prestige".to_string(),
        Target::GreenCoinStrength => "GC pow".to_string(),
    };
    format!("{}: {}", prefix, target)
}

fn powerup_short(k: PowerupKind) -> &'static str {
    match k {
        PowerupKind::Lucky => "Lucky",
        PowerupKind::Frenzy => "Frenzy",
        PowerupKind::Buff => "Buff",
        PowerupKind::GreenCoin => "GCoin",
    }
}

/// Pick box-drawing characters and a color style for a node's render state.
/// Visual hierarchy (only OWNED uses double-line; everything else is
/// dotted, with brightness distinguishing the four states):
///
/// - **Owned**: double-line `╔═╗`, bright biome color, BOLD.
/// - **Buyable** (reachable + affordable): dotted line `┌╌┐`, near-WHITE.
/// - **Reachable but unaffordable**: dotted line, dim biome color.
/// - **Unreachable**: dotted line, very dark grey.
///
/// Active flashes (just-bought / just-unlocked / just-refunded) blend the
/// fg color toward a tint by the flash strength so a buy "pulses green",
/// a newly-reachable neighbor "pulses yellow", and a refunded lot
/// "pulses red".
fn box_chars_for(v: &VisibleNode) -> ((char, char, char, char), char, char, Style) {
    // (top-left, top-right, bottom-left, bottom-right)
    let owned_corners: (char, char, char, char) = ('╔', '╗', '╚', '╝');
    let dotted_corners: (char, char, char, char) = ('┌', '┐', '└', '┘');

    let biome = biome_color(v.dominant_target, v.rarity);
    // Slightly darker than before so the "ignored" state reads as
    // background chrome against the user's terminal bg, not as a
    // candidate to click on.
    let unreachable_fg = Color::Rgb(60, 60, 70);
    // Near-white but tinted very slightly toward the biome's hue so the
    // user can still tell biomes apart at a glance among buyable nodes.
    // Blend ~85% white + 15% biome.bright.
    let buyable_fg = blend_to_white(biome.bright, 0.15);

    let mut result = if v.owned {
        (
            owned_corners,
            '═',
            '║',
            Style::default()
                .fg(biome.bright)
                .add_modifier(StyleMod::BOLD),
        )
    } else if v.reachable && v.affordable {
        (
            dotted_corners,
            '╌',
            '╎',
            Style::default().fg(buyable_fg).add_modifier(StyleMod::BOLD),
        )
    } else if v.reachable {
        // Reachable but unaffordable: dim biome — clearly a tree node
        // (biome-colored) but not currently actionable.
        (dotted_corners, '╌', '╎', Style::default().fg(biome.dim))
    } else {
        // Unreachable: very dark grey, dotted. Reads as background.
        (
            dotted_corners,
            '╌',
            '╎',
            Style::default().fg(unreachable_fg),
        )
    };

    // Flash overlays. Strength is in [0, 1]; blend base fg toward tint.
    if v.buy_flash > 0.001 {
        let tint = (40.0, 230.0, 80.0); // green
        result.3 = Style::default()
            .fg(blend_to(biome.bright, tint, v.buy_flash))
            .add_modifier(StyleMod::BOLD);
    } else if v.refund_flash > 0.001 {
        let tint = (255.0, 80.0, 80.0); // red
        let from = if v.owned {
            biome.bright
        } else {
            unreachable_fg
        };
        result.3 = Style::default()
            .fg(blend_to(from, tint, v.refund_flash))
            .add_modifier(StyleMod::BOLD);
    } else if v.unlock_flash > 0.001 {
        let tint = (255.0, 230.0, 120.0); // gold
        let from = if v.reachable { buyable_fg } else { biome.dim };
        result.3 = Style::default()
            .fg(blend_to(from, tint, v.unlock_flash))
            .add_modifier(StyleMod::BOLD);
    }
    result
}

/// Blend a base color toward pure white by `t` in [0, 1]. Used to pick
/// the "buyable" fg — near-white with a faint biome tint so different
/// biomes are still distinguishable among the buyable set.
fn blend_to_white(base: Color, biome_weight: f32) -> Color {
    let (br, bg, bb) = color_rgb(base);
    let mix = biome_weight.clamp(0.0, 1.0);
    let r = 240.0 * (1.0 - mix) + br * mix;
    let g = 240.0 * (1.0 - mix) + bg * mix;
    let b = 240.0 * (1.0 - mix) + bb * mix;
    Color::Rgb(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
    )
}

fn blend_to(base: Color, tint: (f32, f32, f32), t: f32) -> Color {
    let (br, bg, bb) = color_rgb(base);
    let mix = t.clamp(0.0, 1.0);
    Color::Rgb(
        (br + (tint.0 - br) * mix).clamp(0.0, 255.0) as u8,
        (bg + (tint.1 - bg) * mix).clamp(0.0, 255.0) as u8,
        (bb + (tint.2 - bb) * mix).clamp(0.0, 255.0) as u8,
    )
}

fn color_rgb(c: Color) -> (f32, f32, f32) {
    match c {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        _ => (200.0, 200.0, 200.0),
    }
}

struct BiomeColors {
    bright: Color,
    dim: Color,
}

/// Map a target (the node's "biome") to a color pair. Each fingerer index
/// gets a hand-picked tier color; globals get neutral/special tints.
fn biome_color(target: Target, rarity: Rarity) -> BiomeColors {
    // Keystones override biome with a hot red/purple. Spotting them is
    // the whole point.
    if matches!(rarity, Rarity::Keystone) {
        return BiomeColors {
            bright: Color::Rgb(255, 80, 200),
            dim: Color::Rgb(150, 50, 120),
        };
    }
    match target {
        Target::Fingerer(idx) => fingerer_biome(idx as usize),
        Target::AllFingerers => BiomeColors {
            bright: Color::Rgb(255, 215, 0),
            dim: Color::Rgb(150, 130, 0),
        },
        Target::Click => BiomeColors {
            bright: Color::Rgb(255, 130, 130),
            dim: Color::Rgb(150, 80, 80),
        },
        Target::PowerupSpawn(_) | Target::PowerupReward(_) | Target::PowerupDuration(_) => {
            BiomeColors {
                bright: Color::Rgb(220, 140, 255),
                dim: Color::Rgb(130, 90, 150),
            }
        }
        Target::Prestige => BiomeColors {
            bright: Color::Rgb(255, 200, 220),
            dim: Color::Rgb(150, 110, 130),
        },
        Target::GreenCoinStrength => BiomeColors {
            bright: Color::Rgb(120, 230, 140),
            dim: Color::Rgb(60, 130, 80),
        },
    }
}

fn fingerer_biome(idx: usize) -> BiomeColors {
    match idx {
        0 => BiomeColors {
            bright: Color::Rgb(255, 220, 120),
            dim: Color::Rgb(160, 130, 70),
        },
        1 => BiomeColors {
            bright: Color::Rgb(255, 180, 100),
            dim: Color::Rgb(160, 100, 60),
        },
        2 => BiomeColors {
            bright: Color::Rgb(180, 255, 230),
            dim: Color::Rgb(90, 150, 130),
        },
        3 => BiomeColors {
            bright: Color::Rgb(255, 150, 180),
            dim: Color::Rgb(160, 80, 100),
        },
        4 => BiomeColors {
            bright: Color::Rgb(180, 220, 255),
            dim: Color::Rgb(90, 130, 160),
        },
        5 => BiomeColors {
            bright: Color::Rgb(140, 230, 200),
            dim: Color::Rgb(70, 130, 100),
        },
        6 => BiomeColors {
            bright: Color::Rgb(200, 160, 255),
            dim: Color::Rgb(110, 80, 160),
        },
        7 => BiomeColors {
            bright: Color::Rgb(160, 200, 255),
            dim: Color::Rgb(80, 120, 160),
        },
        8 => BiomeColors {
            bright: Color::Rgb(255, 255, 200),
            dim: Color::Rgb(160, 160, 110),
        },
        _ => BiomeColors {
            bright: Color::Rgb(255, 255, 255),
            dim: Color::Rgb(180, 180, 180),
        },
    }
}

/// Orthogonal L-routing between two boxes — one bend with a rounded
/// corner glyph (`╭ ╮ ╰ ╯`). Looks far cleaner than diagonal Bresenham
/// stair-step, which produced walls of `///\\\`. Path leaves the source
/// box from the side facing the destination, runs a straight segment,
/// turns once with a rounded corner, runs the second segment, and
/// terminates at the destination box's edge. Cells inside either endpoint
/// box are skipped — the box's own border owns those.
fn draw_edge(
    frame: &mut Frame,
    area: Rect,
    pan_x: i32,
    pan_y: i32,
    a: &VisibleNode,
    b: &VisibleNode,
    state: &GameState,
) {
    let a_owned = state.tree.bought.contains(&a.lot);
    let b_owned = state.tree.bought.contains(&b.lot);
    // Pre-energize dim style — what the line looked like when neither
    // endpoint was owned. While the wave is in flight, cells AHEAD of
    // the wavefront keep this dim grey so the snake's head reads as
    // physically painting the line from grey to lit.
    let dim_style = Style::default().fg(Color::Rgb(80, 80, 100));
    // Lit / resting style — what the line settles into once the wave
    // has passed (and what cells behind the wave hold). Matches the
    // one-owned and both-owned resting palettes.
    let lit_style = if a_owned && b_owned {
        Style::default()
            .fg(Color::Rgb(255, 220, 120))
            .add_modifier(StyleMod::BOLD)
    } else if a_owned || b_owned {
        Style::default().fg(Color::Rgb(180, 180, 200))
    } else {
        dim_style
    };

    // `edge_path_cells` is canonical (lo→hi lot order), so the path
    // shape is the same regardless of which is `a` vs `b` here. That
    // guarantees the wave overlays the same staircase the grey base
    // line was drawn on — without canonicalization the wave could
    // bend along a Bresenham mirror image of the resting line.
    let anim = state
        .tree_edge_anims
        .iter()
        .find(|an| (an.from == a.lot && an.to == b.lot) || (an.from == b.lot && an.to == a.lot));
    let path = node::edge_path_cells(a.lot, b.lot);
    if path.is_empty() {
        return;
    }
    // Wave anchor: which end of the canonical path is the anim's source
    // (anim.from). `canonical_lo` is the lot at path[0].
    let canonical_lo = if (a.lot.x, a.lot.y) <= (b.lot.x, b.lot.y) {
        a.lot
    } else {
        b.lot
    };
    let wave_at_start = anim.map(|an| an.from == canonical_lo).unwrap_or(true);
    // Source-side leading_inside (cells inside the source box at the
    // wave's start end of the canonical path). Seeds the head past the
    // in-box prefix so the visible wave starts at the FIRST visible
    // cell on tick 0 — regardless of how much of the path lives inside
    // the source's bounding rect.
    let source_node: &VisibleNode = if let Some(an) = anim {
        if an.from == a.lot { a } else { b }
    } else {
        a
    };
    let leading_inside = if wave_at_start {
        node::count_leading_in_rect(
            &path,
            source_node.spec_box_x,
            source_node.spec_box_y,
            source_node.box_w,
            source_node.box_h,
        )
    } else {
        node::count_trailing_in_rect(
            &path,
            source_node.spec_box_x,
            source_node.spec_box_y,
            source_node.box_w,
            source_node.box_h,
        )
    };
    let head_path_index = anim
        .map(|an| (leading_inside + an.visible_advance()).min(path.len().saturating_sub(1)))
        .unwrap_or(0);

    // Same opacity rule as before: regular boxes are opaque across their
    // whole bounding rect, the anchor is only opaque on actually-painted
    // sprite cells (so edges can meet its rounded outline).
    let in_a = |x: i32, y: i32| opaque_for(a, x, y);
    let in_b = |x: i32, y: i32| opaque_for(b, x, y);

    let buf = frame.buffer_mut();
    let path_len = path.len();
    for i in 0..path_len {
        let (cx, cy) = path[i];
        if in_a(cx, cy) || in_b(cx, cy) {
            continue;
        }
        // Animation styling. `dist_from_head` counts how many cells we
        // are behind the wavefront in the anim's direction-of-travel:
        // 0 is the head cell, 1+ is the trailing tail, negative means
        // we're ahead of the wavefront. The path is canonical (lo→hi);
        // the wave runs from path[0] outward when the anim's source is
        // at canonical_lo, otherwise from path[end] inward.
        let dist_from_head: i32 = if anim.is_some() {
            if wave_at_start {
                (head_path_index as i32) - (i as i32)
            } else {
                (head_path_index as i32) - ((path_len - 1 - i) as i32)
            }
        } else {
            0
        };

        let prev_raw = if i > 0 { Some(path[i - 1]) } else { None };
        let next_raw = path.get(i + 1).copied();
        let prev_in_box = prev_raw.is_some_and(|(px, py)| in_a(px, py) || in_b(px, py));
        let next_in_box = next_raw.is_some_and(|(nx, ny)| in_a(nx, ny) || in_b(nx, ny));
        let prev = prev_raw.filter(|_| !prev_in_box);
        let next = next_raw.filter(|_| !next_in_box);
        // dir_to_box: direction FROM this cell TOWARD the in-box
        // neighbor. Tells the glyph picker where the endpoint box is,
        // which can differ from the line's motion direction at this
        // cell (a Bresenham staircase can end with a horizontal step
        // into a box that's vertically above/below).
        let dir_to_box = if prev_in_box {
            prev_raw.map(|p| dir_between((cx, cy), p))
        } else if next_in_box {
            next_raw.map(|n| dir_between((cx, cy), n))
        } else {
            None
        };
        let glyph = path_glyph(
            prev.map(|p| dir_between(p, (cx, cy))),
            next.map(|n| dir_between((cx, cy), n)),
            dir_to_box,
        );

        // Snake-game styling. The head cell is pure white; the few
        // cells just behind it pulse through cyan; further-behind
        // cells settle into the lit resting style (so the trail
        // STAYS lit — the snake grows). Cells AHEAD of the head are
        // still grey (pre-energize), so the wave reads as the snake
        // eating its way along the wire.
        let style = if anim.is_some() {
            if dist_from_head < 0 {
                dim_style
            } else {
                match dist_from_head {
                    0 => Style::default()
                        .fg(Color::Rgb(255, 255, 255))
                        .add_modifier(StyleMod::BOLD),
                    1 => Style::default()
                        .fg(Color::Rgb(120, 220, 255))
                        .add_modifier(StyleMod::BOLD),
                    2 => Style::default()
                        .fg(Color::Rgb(80, 170, 230))
                        .add_modifier(StyleMod::BOLD),
                    _ => lit_style,
                }
            }
        } else {
            lit_style
        };

        if let Some((sx, sy)) = canvas_to_screen(area, pan_x, pan_y, cx, cy)
            && let Some(cell) = buf.cell_mut((sx, sy))
        {
            cell.set_char(glyph);
            cell.set_style(style);
        }
    }
    // Diagonal anchor metadata is no longer consulted at render time —
    // the staircase doesn't care which "L-corner" the procgen reserved.
    let _ = node::diagonal_route_via;
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Returns `true` if the cell at canvas coord `(x, y)` is "opaque" for
/// edge-routing purposes — i.e. the renderer will paint a visible
/// glyph there, so an edge line should stop short and not intrude.
///
/// For regular boxes: every cell in the bounding rect is opaque (the
/// box paints title text, primitives, and border in every cell).
/// For the cuque anchor: every cell inside the SILHOUETTE is opaque —
/// the silhouette per row spans the leftmost to rightmost non-space
/// characters in `BISCUIT_TINY`. Cells inside the silhouette are part
/// of the visible cuque body (even where the source has interior
/// spaces, they're still inside the rounded outline). Cells in the
/// bounding rect but OUTSIDE the silhouette (the four corners) are
/// transparent so edges glide past the rounded shape cleanly.
fn opaque_for(v: &VisibleNode, x: i32, y: i32) -> bool {
    let local_col = x - v.spec_box_x;
    let local_row = y - v.spec_box_y;
    if local_col < 0 || local_row < 0 || local_col >= v.box_w as i32 || local_row >= v.box_h as i32
    {
        return false;
    }
    if !v.is_anchor {
        return true;
    }
    // Anchor: opaque inside the per-row silhouette of BISCUIT_TINY.
    let rows = crate::ui::biscuit::BISCUIT_TINY;
    let Some(line) = rows.get(local_row as usize) else {
        return false;
    };
    let chars: Vec<char> = line.chars().collect();
    let first = chars.iter().position(|c| *c != ' ');
    let last = chars.iter().rposition(|c| *c != ' ');
    match (first, last) {
        (Some(f), Some(l)) => local_col >= f as i32 && local_col <= l as i32,
        _ => false,
    }
}

fn dir_between(from: (i32, i32), to: (i32, i32)) -> Dir {
    if to.0 > from.0 {
        Dir::Right
    } else if to.0 < from.0 {
        Dir::Left
    } else if to.1 > from.1 {
        Dir::Down
    } else {
        Dir::Up
    }
}

/// Glyph for a single cell in a path given the direction of motion when
/// arriving (`dir_in`) and departing (`dir_out`), plus the direction
/// from this cell TOWARD an endpoint box if either neighbor is filtered
/// out by the box's opaque region (`dir_to_box`).
///
/// Terminus handling — the load-bearing case. When the path approaches
/// an endpoint box, the last visible cell's LINE motion (e.g. a
/// rightward staircase step) can differ from the BOX direction
/// (e.g. the box sits directly above). A plain `─` would dead-end
/// facing empty space; the correct render is a rounded corner `╯` that
/// bends from the line's prev direction into the box edge. When line
/// motion and box direction lie on the same axis, the cell is just an
/// ordinary straight `─`/`│`.
fn path_glyph(dir_in: Option<Dir>, dir_out: Option<Dir>, dir_to_box: Option<Dir>) -> char {
    use Dir::*;
    // Helper: pick the corner glyph for a cell that has paint reaching
    // TWO of its edges (the two directions passed). Order doesn't matter.
    fn corner(a: Dir, b: Dir) -> char {
        let mut ds = [a, b];
        ds.sort_by_key(|d| match d {
            Up => 0,
            Down => 1,
            Left => 2,
            Right => 3,
        });
        match (ds[0], ds[1]) {
            (Up, Left) => '╯',
            (Up, Right) => '╰',
            (Down, Left) => '╮',
            (Down, Right) => '╭',
            // Two-edge combos that are actually straight runs.
            (Up, Down) => '│',
            (Left, Right) => '─',
            _ => '·',
        }
    }

    // Terminus case: this cell has exactly one of dir_in / dir_out set,
    // and we know which direction the box sits.
    if let Some(box_dir) = dir_to_box {
        let line_dir = dir_in.or(dir_out);
        match line_dir {
            // Same-axis case — line motion is colinear with box
            // direction (same OR opposite, e.g. line emerges from a
            // box to the left going right). Ordinary straight glyph;
            // no terminator decoration.
            Some(d) if d == box_dir || d == opposite(box_dir) => {
                return match box_dir {
                    Up | Down => '│',
                    Left | Right => '─',
                };
            }
            // Perpendicular: the cell bends from the line's incoming
            // edge into the box's edge. Corner glyph.
            //
            // LAST visible cell (`dir_in = Some`, `dir_out = None`):
            // line came IN from `-dir_in` (the opposite edge of the
            // dir_in motion). Cell's painted edges are `-dir_in` and
            // `box_dir`.
            //
            // FIRST visible cell (`dir_in = None`, `dir_out = Some`):
            // line goes OUT toward `dir_out`. Cell's painted edges are
            // `dir_out` and `box_dir`.
            Some(d) => {
                let line_edge = if dir_in.is_some() { opposite(d) } else { d };
                return corner(line_edge, box_dir);
            }
            None => {
                return match box_dir {
                    Up | Down => '│',
                    Left | Right => '─',
                };
            }
        }
    }

    match (dir_in, dir_out) {
        // Straight runs through the middle of the line.
        (Some(Right), Some(Right)) | (Some(Left), Some(Left)) => '─',
        (Some(Down), Some(Down)) | (Some(Up), Some(Up)) => '│',
        // Rounded turns. The corner glyph connects the cell's two
        // OUTGOING edges (the directions the line leaves through):
        //   ╭ connects DOWN and RIGHT edges
        //   ╮ connects DOWN and LEFT
        //   ╰ connects UP and RIGHT
        //   ╯ connects UP and LEFT
        (Some(Right), Some(Down)) | (Some(Up), Some(Left)) => '╮',
        (Some(Right), Some(Up)) | (Some(Down), Some(Left)) => '╯',
        (Some(Left), Some(Down)) | (Some(Up), Some(Right)) => '╭',
        (Some(Left), Some(Up)) | (Some(Down), Some(Right)) => '╰',
        // U-turns (shouldn't happen in Bresenham staircase) and the
        // both-None case fall back to a neutral marker.
        _ => '·',
    }
}

fn opposite(d: Dir) -> Dir {
    match d {
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
    }
}

fn hover_lift(frame: &mut Frame, r: Rect) {
    let buf = frame.buffer_mut();
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(StyleMod::BOLD));
            }
        }
    }
}

// ---------- Info pane ------------------------------------------------------

/// Button-label strings. Kept as constants so the click-rect math (which
/// has to know exactly how many characters wide the label is) stays in
/// sync with the rendered text. Touching one site touches both.
const BUY_BUTTON_LABEL: &str = "[Click/Enter] to buy";
const REFUND_BUTTON_LABEL: &str = "[Click/R] refund";

fn draw_info_pane(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
) -> Option<(TreeButtonAction, Rect)> {
    let cursor = state.tree.cursor;
    let mut lines: Vec<Line> = Vec::new();
    // The rect of the currently-rendered action button (if any). We
    // compute it from string-prefix lengths during the line build and
    // return it for the input router to hit-test.
    let mut action_button: Option<(TreeButtonAction, Rect)> = None;
    // Inner content lives one row below the top border that the
    // surrounding Block draws.
    let inner_y = area.y + 1;
    let inner_x = area.x;

    match node::node_at(cursor.x, cursor.y) {
        None => {
            lines.push(Line::styled(
                format!("(empty lot at {:+}, {:+})", cursor.x, cursor.y),
                Style::default().fg(Color::Rgb(120, 120, 130)),
            ));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Pan with arrows / hjkl to find populated lots.",
                Style::default().fg(Color::Rgb(160, 160, 170)),
            ));
        }
        Some(spec) => {
            // The anchor gets its own info-pane copy — no rarity tag, no
            // primitives, no buy/refund options. It's the cuque itself,
            // always active.
            if spec.is_anchor {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}  ", spec.title),
                        Style::default()
                            .fg(Color::Rgb(255, 220, 130))
                            .add_modifier(StyleMod::BOLD),
                    ),
                    Span::styled(
                        "[Root Node]",
                        Style::default()
                            .fg(Color::Rgb(255, 200, 100))
                            .add_modifier(StyleMod::BOLD),
                    ),
                ]));
                lines.push(Line::styled(
                    "  - the twitching ass itself",
                    Style::default().fg(Color::Rgb(200, 200, 210)),
                ));
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    "  every path of the tree branches outward from here.",
                    Style::default().fg(Color::Rgb(160, 160, 170)),
                ));
                let p = Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::TOP)
                        .border_style(Style::default().fg(Color::Rgb(60, 60, 80))),
                );
                frame.render_widget(p, area);
                return action_button;
            }

            let owned = state.tree.bought.contains(&cursor);
            let reachable = state.tree_reachable(cursor);
            let affordable = state.affordable_cuques() >= spec.cost;
            let rarity_label = match spec.rarity {
                Rarity::Small => "Small",
                Rarity::Notable => "Notable",
                Rarity::Keystone => "KEYSTONE",
            };
            let rarity_color = match spec.rarity {
                Rarity::Small => Color::Rgb(200, 200, 220),
                Rarity::Notable => Color::Rgb(255, 220, 120),
                Rarity::Keystone => Color::Rgb(255, 80, 200),
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  ", spec.title),
                    Style::default()
                        .fg(Color::Rgb(255, 255, 255))
                        .add_modifier(StyleMod::BOLD),
                ),
                Span::styled(
                    format!("[{}]", rarity_label),
                    Style::default()
                        .fg(rarity_color)
                        .add_modifier(StyleMod::BOLD),
                ),
            ]));
            for p in &spec.primitives {
                lines.push(Line::from(vec![
                    Span::raw("  • "),
                    Span::styled(primitive_blurb(*p), Style::default().fg(prim_color(*p))),
                ]));
            }
            lines.push(Line::raw(""));
            // The action hint is the LAST line we push to `lines`. Its
            // row in screen coords is `inner_y + lines.len()` *just
            // before* we push it. Capture that now and use it below
            // when computing the button rect.
            let action_row = inner_y + lines.len() as u16;
            let action_hint = if owned {
                let can_refund = state.can_refund_tree_node(cursor);
                if !can_refund {
                    let reason = if cursor == TreeCoord::ORIGIN {
                        "anchor — origin cannot be refunded"
                    } else {
                        "would orphan another owned node — refund a leaf first"
                    };
                    Line::from(vec![
                        Span::styled(
                            "  [owned]  ",
                            Style::default().fg(Color::Rgb(180, 220, 180)),
                        ),
                        Span::styled(
                            format!("(no refund: {reason})"),
                            Style::default().fg(Color::Rgb(170, 130, 130)),
                        ),
                    ])
                } else {
                    let refund = (spec.cost * TREE_REFUND_FRACTION).floor();
                    let loss = spec.cost - refund;
                    let prefix = "  [owned]  "; // 11 cols
                    let label_len = REFUND_BUTTON_LABEL.chars().count() as u16;
                    action_button = Some((
                        TreeButtonAction::Refund,
                        Rect {
                            x: inner_x + prefix.chars().count() as u16,
                            y: action_row,
                            width: label_len,
                            height: 1,
                        },
                    ));
                    Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Color::Rgb(180, 220, 180))),
                        Span::styled(
                            REFUND_BUTTON_LABEL,
                            Style::default()
                                .fg(Color::Rgb(220, 220, 120))
                                .add_modifier(StyleMod::BOLD)
                                .add_modifier(StyleMod::UNDERLINED),
                        ),
                        Span::styled(
                            format!(
                                "  (returns {}, loses {})",
                                format::big(refund),
                                format::big(loss)
                            ),
                            Style::default().fg(Color::Rgb(180, 150, 150)),
                        ),
                    ])
                }
            } else if !reachable {
                Line::styled(
                    "  unreachable — buy a connected neighbor first",
                    Style::default().fg(Color::Rgb(180, 100, 100)),
                )
            } else if !affordable {
                Line::styled(
                    format!(
                        "  Cost: {}  (need {} more)",
                        format::big(spec.cost),
                        format::big(spec.cost - state.affordable_cuques())
                    ),
                    Style::default().fg(Color::Rgb(220, 100, 100)),
                )
            } else {
                let cost_text = format::big(spec.cost);
                // Line layout: "  Cost: {cost}   {BUY_LABEL}"
                let prefix_cols =
                    "  Cost: ".chars().count() + cost_text.chars().count() + "   ".chars().count();
                let label_len = BUY_BUTTON_LABEL.chars().count() as u16;
                action_button = Some((
                    TreeButtonAction::Buy,
                    Rect {
                        x: inner_x + prefix_cols as u16,
                        y: action_row,
                        width: label_len,
                        height: 1,
                    },
                ));
                Line::from(vec![
                    Span::raw("  Cost: "),
                    Span::styled(
                        cost_text,
                        Style::default()
                            .fg(Color::Rgb(120, 255, 120))
                            .add_modifier(StyleMod::BOLD),
                    ),
                    Span::raw("   "),
                    Span::styled(
                        BUY_BUTTON_LABEL,
                        Style::default()
                            .fg(Color::Rgb(220, 220, 120))
                            .add_modifier(StyleMod::BOLD)
                            .add_modifier(StyleMod::UNDERLINED),
                    ),
                ])
            };
            lines.push(action_hint);
        }
    }

    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::Rgb(60, 60, 80))),
    );
    frame.render_widget(p, area);
    action_button
}

fn prim_color(p: Primitive) -> Color {
    if p.is_bane() {
        Color::Rgb(255, 130, 130)
    } else {
        match p.op {
            Op::AddPercent => Color::Rgb(180, 230, 255),
            Op::MulFactor => Color::Rgb(255, 220, 120),
            Op::FlatAdd => Color::Rgb(180, 255, 200),
            Op::CostMul => Color::Rgb(220, 180, 255),
            Op::SpawnRateMul | Op::EffectMul => Color::Rgb(255, 200, 240),
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1).max(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

#[allow(dead_code)]
fn _used(spec: &NodeSpec) {
    let _ = spec.box_x;
}
