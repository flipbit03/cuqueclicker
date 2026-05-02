use ratatui::prelude::*;

use crate::game::fingerer::FINGERERS;
use crate::game::state::GameState;

const PER_TYPE_CAP: usize = 40;
const PER_RING: usize = 48;

/// True when `(col, row)` falls on a cell currently occupied by an orbital
/// hand glyph. Used by the click router so a click landing on a `[]` or
/// `:*` decoration outside the biscuit is treated as a no-op rather than a
/// misclick (otherwise the misclick "·" briefly replaces the glyph and
/// reads as flicker).
///
/// Mirrors the placement math in `draw()` exactly — keep in sync if either
/// changes. Cheap: at most `PER_TYPE_CAP * FINGERERS.len()` hand
/// candidates, and we early-return on the first match.
pub fn occupied_at(
    col: u16,
    row: u16,
    biscuit: Rect,
    focal: (u16, u16),
    state: &GameState,
) -> bool {
    if biscuit.width == 0 || biscuit.height == 0 {
        return false;
    }
    // Orbit around the focal cell ("the asshole"), NOT the bbox center.
    // Each zoom art's `asshole_col` differs from `width / 2` by up to 1
    // column (TINY: 7 vs 8, FULL: 31 vs 30, etc.), and using the bbox
    // center makes the ring visibly lopsided around the cuque. Radii
    // still derive from bbox dimensions — that's the right scale.
    let cx = focal.0 as f32;
    let cy = focal.1 as f32;
    let base_rx = (biscuit.width as f32 / 2.0 + 3.0).max(6.0);
    let base_ry = (biscuit.height as f32 / 2.0 + 2.0).max(3.0);

    let mut glyphs: Vec<(usize, &str)> = Vec::new();
    for (idx, f) in FINGERERS.iter().enumerate() {
        let n = (state.fingerer_count(f.id) as usize).min(PER_TYPE_CAP);
        for _ in 0..n {
            glyphs.push((idx, f.icon));
        }
    }
    if glyphs.is_empty() {
        return false;
    }

    let total = glyphs.len();
    let bx = biscuit.x as i32;
    let br = bx + biscuit.width as i32;
    let by = biscuit.y as i32;
    let bb = by + biscuit.height as i32;

    for (i, (type_idx, icon)) in glyphs.iter().enumerate() {
        let ring = i / PER_RING;
        let slot = i % PER_RING;
        let slot_count = ((total - ring * PER_RING).min(PER_RING)) as f32;
        let angle = (slot as f32 / slot_count) * std::f32::consts::TAU
            + (ring as f32 * 0.15)
            + (*type_idx as f32 * 0.07);
        // Mirror the per-tier poke math from `draw()` exactly so a click
        // landing on a hand-glyph cell at this frame is detected
        // identically (otherwise the hit-test could drop just as the
        // pulse pops the glyph inward).
        let speed = FINGERERS
            .get(*type_idx)
            .map(|f| f.poke_speed.max(0.1))
            .unwrap_or(1.0);
        let tier_divisor = (5.0 / speed).max(1.0) as u64;
        let tier_phase = state.session_ticks / tier_divisor;
        let poke = if ((i * 7) as u64 + tier_phase).is_multiple_of(23) {
            1.2
        } else {
            0.0
        };
        let rx = base_rx + ring as f32 * 4.0 - poke;
        let ry = base_ry + ring as f32 * 2.0 - poke * 0.5;
        let px = cx + rx * angle.cos();
        let py = cy + ry * angle.sin();
        let g_col = px.round() as i32;
        let g_row = py.round() as i32;

        let icon_w = icon.chars().count() as i32;
        // Hands inside the biscuit footprint aren't drawn (they get
        // suppressed in `draw`) — don't count them as occupied.
        if g_row >= by && g_row < bb && g_col < br && g_col + icon_w > bx {
            continue;
        }
        // The glyph occupies cells [g_col, g_col + icon_w) on row g_row.
        if (row as i32) == g_row && (col as i32) >= g_col && (col as i32) < g_col + icon_w {
            return true;
        }
    }
    false
}

pub fn draw(
    frame: &mut Frame,
    play_area: Rect,
    biscuit: Rect,
    focal: (u16, u16),
    state: &GameState,
) {
    if play_area.width == 0 || play_area.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    // Orbit around the visual focal cell — see comment in `occupied_at`.
    let cx = focal.0 as f32;
    let cy = focal.1 as f32;

    let base_rx = (biscuit.width as f32 / 2.0 + 3.0).max(6.0);
    let base_ry = (biscuit.height as f32 / 2.0 + 2.0).max(3.0);

    let mut glyphs: Vec<(usize, &str)> = Vec::new();
    for (idx, f) in FINGERERS.iter().enumerate() {
        let n = (state.fingerer_count(f.id) as usize).min(PER_TYPE_CAP);
        for _ in 0..n {
            glyphs.push((idx, f.icon));
        }
    }
    if glyphs.is_empty() {
        return;
    }

    let total = glyphs.len();

    for (i, (type_idx, icon)) in glyphs.iter().enumerate() {
        let ring = i / PER_RING;
        let slot = i % PER_RING;
        let slot_count = ((total - ring * PER_RING).min(PER_RING)) as f32;
        let angle = (slot as f32 / slot_count) * std::f32::consts::TAU
            + (ring as f32 * 0.15)
            + (*type_idx as f32 * 0.07);
        // Per-tier poke timing: each fingerer tier carries its own
        // `poke_speed` (1.0 = baseline). Tier-aware tick_phase = ticks
        // divided by a per-tier divisor — high speed = small divisor =
        // fast pulse; low speed = big divisor = slow majestic pulse.
        // Hand of God ends up ~10× slower than Index Finger, so a heavy
        // crust of HoG pulses with statelier authority while finger-tier
        // hands twitch fast.
        let speed = FINGERERS
            .get(*type_idx)
            .map(|f| f.poke_speed.max(0.1))
            .unwrap_or(1.0);
        let tier_divisor = (5.0 / speed).max(1.0) as u64;
        let tier_phase = state.session_ticks / tier_divisor;
        let poke = if ((i * 7) as u64 + tier_phase).is_multiple_of(23) {
            1.2
        } else {
            0.0
        };
        let rx = base_rx + ring as f32 * 4.0 - poke;
        let ry = base_ry + ring as f32 * 2.0 - poke * 0.5;
        let px = cx + rx * angle.cos();
        let py = cy + ry * angle.sin();
        let col = px.round() as i32;
        let row = py.round() as i32;

        let icon_w = icon.chars().count() as i32;
        if col < play_area.x as i32
            || col + icon_w > (play_area.x + play_area.width) as i32
            || row < play_area.y as i32
            || row >= (play_area.y + play_area.height) as i32
        {
            continue;
        }
        let bx = biscuit.x as i32;
        let br = bx + biscuit.width as i32;
        let by = biscuit.y as i32;
        let bb = by + biscuit.height as i32;
        if row >= by && row < bb && col < br && col + icon_w > bx {
            continue;
        }
        let color = hand_color(*type_idx, poke > 0.0);
        buf.set_string(col as u16, row as u16, *icon, Style::default().fg(color));
    }
}

fn hand_color(type_idx: usize, poking: bool) -> Color {
    let palette = [
        Color::Rgb(220, 170, 130), // Dedo: warm tan
        Color::Rgb(230, 180, 160), // Mao: pinker
        Color::Rgb(200, 230, 220), // Luva: mint
        Color::Rgb(255, 130, 170), // Beijo Grego: rose pink
        Color::Rgb(180, 200, 230), // Robotico: steel blue
        Color::Rgb(200, 160, 220), // Tentaculo: purple
        Color::Rgb(255, 200, 120), // Vortice: amber
        Color::Rgb(140, 180, 255), // Buraco: cold blue
        Color::Rgb(255, 180, 255), // Cosmic: magenta
        Color::Rgb(255, 230, 150), // Deus: divine gold
    ];
    let c = palette[type_idx.min(palette.len() - 1)];
    if poking { brighten(c) } else { c }
}

fn brighten(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as u16 + 40).min(255) as u8,
            (g as u16 + 40).min(255) as u8,
            (b as u16 + 40).min(255) as u8,
        ),
        other => other,
    }
}
