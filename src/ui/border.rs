use ratatui::{prelude::*, widgets::*};

use crate::game::state::{
    ACHIEVEMENT_FLASH_TICKS, Buff, GameState, LUCKY_FLASH_TICKS, PURCHASE_FLASH_TICKS,
};

// Resting baseline — what the border looks like when nothing is happening.
const BASELINE: (f32, f32, f32) = (200.0, 200.0, 210.0);
// Active carrier — what the "non-tint" cells are during an active event.
// Pure white gives high contrast against every tint so pulses pop.
const ACTIVE_CARRIER: (f32, f32, f32) = (255.0, 255.0, 255.0);

// Each event owns a (tint, cycle_length_in_cells) pair. When active, that
// event adds a sine wave of "direction from baseline toward tint" modulated
// by its strength. Cycles are kept coprime so simultaneous events drift out
// of sync and produce moiré-like plaids of color instead of averaging to
// mush.
const FRENZY_TINT: (f32, f32, f32) = (255.0, 60.0, 60.0);
const FRENZY_CYCLE: f32 = 13.0;
const BUFF_TINT: (f32, f32, f32) = (200.0, 100.0, 255.0);
const BUFF_CYCLE: f32 = 17.0;
const LUCKY_TINT: (f32, f32, f32) = (255.0, 215.0, 0.0);
const LUCKY_CYCLE: f32 = 23.0;
const PURCHASE_TINT: (f32, f32, f32) = (40.0, 230.0, 80.0);
const PURCHASE_CYCLE: f32 = 11.0;
// Achievement-unlock channel: warm gold like Lucky but on its own coprime
// cycle so an unlock during a Lucky/Frenzy doesn't average out into the
// existing tint — it lays a distinct moiré on top.
const ACHIEVEMENT_TINT: (f32, f32, f32) = (255.0, 200.0, 100.0);
const ACHIEVEMENT_CYCLE: f32 = 19.0;

// Permanent subtle undertone once any prestige has been earned.
const PRESTIGE_TINT: (f32, f32, f32) = (200.0, 170.0, 80.0);
const PRESTIGE_WEIGHT: f32 = 0.08;

pub fn draw_animated(frame: &mut Frame, area: Rect, state: &GameState, title: &str) {
    let block = Block::bordered().title(title);
    frame.render_widget(block, area);

    if area.width < 2 || area.height < 2 {
        return;
    }

    let buf = frame.buffer_mut();
    let mut i: usize = 0;
    let last_x = area.x + area.width - 1;
    let last_y = area.y + area.height - 1;

    for x in area.x..=last_x {
        recolor(buf, x, area.y, cell_color(i, state));
        i += 1;
    }
    for y in (area.y + 1)..=last_y {
        recolor(buf, last_x, y, cell_color(i, state));
        i += 1;
    }
    if area.height > 1 {
        for x in (area.x..last_x).rev() {
            recolor(buf, x, last_y, cell_color(i, state));
            i += 1;
        }
    }
    if area.width > 1 && area.height > 2 {
        for y in ((area.y + 1)..last_y).rev() {
            recolor(buf, area.x, y, cell_color(i, state));
            i += 1;
        }
    }
}

fn recolor(buf: &mut Buffer, x: u16, y: u16, color: Color) {
    if x >= buf.area.x + buf.area.width || y >= buf.area.y + buf.area.height {
        return;
    }
    let cell = &mut buf[(x, y)];
    cell.set_fg(color);
    cell.modifier.insert(Modifier::BOLD);
}

fn cell_color(i: usize, state: &GameState) -> Color {
    let phase = state.border_phase as f32;

    // Flashes plateau at full strength for most of their duration, then
    // smoothstep-fade over the last fraction. Keeps the "white + tint"
    // contrast visible for most of the flash, then eases back to baseline.
    // Purchase strength is multiplied by the bulk-buy multiplier (1.0..3.0)
    // so a max-buy lands harder than a single click, but capped at 1.0 in
    // the channel composition so we don't blow out the carrier blend.
    let purchase_s = plateau_fade(state.purchase_flash_ticks, PURCHASE_FLASH_TICKS)
        * (state.purchase_flash_strength.max(1.0) / 3.0).clamp(0.34, 1.0);
    let lucky_s = plateau_fade(state.lucky_flash_ticks, LUCKY_FLASH_TICKS);
    let achievement_s = plateau_fade(state.achievement_flash_ticks, ACHIEVEMENT_FLASH_TICKS);
    let frenzy_s = state
        .buffs
        .iter()
        .filter_map(|b| match b {
            Buff::ClickFrenzy { .. } => Some(b.strength()),
            _ => None,
        })
        .fold(0.0_f32, f32::max);
    let buff_s = state
        .buffs
        .iter()
        .filter_map(|b| match b {
            Buff::FingererBoost { .. } => Some(b.strength()),
            _ => None,
        })
        .fold(0.0_f32, f32::max);

    // Carrier smoothly blends from resting gray to pure white as total
    // activity rises, so pulses swing between WHITE and tint (high contrast)
    // instead of GRAY and tint (dull). When activity decays back to 0 the
    // carrier eases back to gray, avoiding a jarring cut.
    let activity = purchase_s
        .max(lucky_s)
        .max(achievement_s)
        .max(frenzy_s)
        .max(buff_s);
    let carrier_r = BASELINE.0 + (ACTIVE_CARRIER.0 - BASELINE.0) * activity;
    let carrier_g = BASELINE.1 + (ACTIVE_CARRIER.1 - BASELINE.1) * activity;
    let carrier_b = BASELINE.2 + (ACTIVE_CARRIER.2 - BASELINE.2) * activity;

    let mut r = carrier_r;
    let mut g = carrier_g;
    let mut b = carrier_b;

    // Each event adds a wave-modulated deviation from the (white) carrier
    // toward its tint. Channels are summed independently and clamped, so
    // simultaneous events produce chromatic combinations (red+blue → hot
    // magenta, red+gold → orange, etc.) rather than a muddy average.
    for (tint, cycle, strength) in [
        (PURCHASE_TINT, PURCHASE_CYCLE, purchase_s),
        (LUCKY_TINT, LUCKY_CYCLE, lucky_s),
        (ACHIEVEMENT_TINT, ACHIEVEMENT_CYCLE, achievement_s),
        (BUFF_TINT, BUFF_CYCLE, buff_s),
        (FRENZY_TINT, FRENZY_CYCLE, frenzy_s),
    ] {
        if strength > 0.001 {
            let wave01 = (((i as f32 + phase) * std::f32::consts::TAU / cycle).sin() + 1.0) * 0.5;
            let contribution = wave01 * strength;
            r += (tint.0 - carrier_r) * contribution;
            g += (tint.1 - carrier_g) * contribution;
            b += (tint.2 - carrier_b) * contribution;
        }
    }

    let mut r = r.clamp(0.0, 255.0);
    let mut g = g.clamp(0.0, 255.0);
    let mut b = b.clamp(0.0, 255.0);

    if state.prestige > 0 {
        r += (PRESTIGE_TINT.0 - r) * PRESTIGE_WEIGHT;
        g += (PRESTIGE_TINT.1 - g) * PRESTIGE_WEIGHT;
        b += (PRESTIGE_TINT.2 - b) * PRESTIGE_WEIGHT;
    }

    Color::Rgb(r as u8, g as u8, b as u8)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1.0 for the bulk of the flash, smoothstep-decays only in the last ~40%.
/// Shapes like: ---[=============\___].
pub fn plateau_fade(remaining: u32, total: u32) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let fade_ticks = (total as f32 * 0.4).max(1.0);
    let r = remaining as f32;
    if r >= fade_ticks {
        1.0
    } else {
        smoothstep(r / fade_ticks)
    }
}

/// Recolor the rectangular border of `area` by walking it cell-by-cell with
/// the same wave-tint logic as `draw_animated`, but driven by a single
/// `(tint, strength)` pair. Used to flash secondary panel borders (Fingerers
/// / Upgrades sidebar) on purchase events so the whole panel reads as
/// "something just happened here", not just the row that changed.
///
/// Caller is expected to have already rendered a `Block::bordered()` over
/// `area`; this only mutates color/style of the existing border cells.
pub fn paint_border_flash(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
    tint: (f32, f32, f32),
    cycle: f32,
    strength: f32,
) {
    if strength <= 0.001 || area.width < 2 || area.height < 2 {
        return;
    }
    let buf = frame.buffer_mut();
    let phase = state.border_phase as f32;
    let last_x = area.x + area.width - 1;
    let last_y = area.y + area.height - 1;

    let mut paint = |x: u16, y: u16, i: usize| {
        if x >= buf.area.x + buf.area.width || y >= buf.area.y + buf.area.height {
            return;
        }
        let wave01 = (((i as f32 + phase) * std::f32::consts::TAU / cycle).sin() + 1.0) * 0.5;
        // Carrier: resting gray → white as strength rises, then tint sits on
        // top via the same wave logic as the HUD border. Identical look feel.
        let carrier_r = BASELINE.0 + (ACTIVE_CARRIER.0 - BASELINE.0) * strength;
        let carrier_g = BASELINE.1 + (ACTIVE_CARRIER.1 - BASELINE.1) * strength;
        let carrier_b = BASELINE.2 + (ACTIVE_CARRIER.2 - BASELINE.2) * strength;
        let contribution = wave01 * strength;
        let r = carrier_r + (tint.0 - carrier_r) * contribution;
        let g = carrier_g + (tint.1 - carrier_g) * contribution;
        let b = carrier_b + (tint.2 - carrier_b) * contribution;
        let cell = &mut buf[(x, y)];
        cell.set_fg(Color::Rgb(
            r.clamp(0.0, 255.0) as u8,
            g.clamp(0.0, 255.0) as u8,
            b.clamp(0.0, 255.0) as u8,
        ));
        cell.modifier.insert(Modifier::BOLD);
    };

    let mut i = 0usize;
    for x in area.x..=last_x {
        paint(x, area.y, i);
        i += 1;
    }
    for y in (area.y + 1)..=last_y {
        paint(last_x, y, i);
        i += 1;
    }
    if area.height > 1 {
        for x in (area.x..last_x).rev() {
            paint(x, last_y, i);
            i += 1;
        }
    }
    if area.width > 1 && area.height > 2 {
        for y in ((area.y + 1)..last_y).rev() {
            paint(area.x, y, i);
            i += 1;
        }
    }
}

/// Tint constants exported so callers (sidebar, upgrades) can request a
/// matching panel-border flash without redefining the palette.
pub const PANEL_PURCHASE_TINT: (f32, f32, f32) = PURCHASE_TINT;
pub const PANEL_PURCHASE_CYCLE: f32 = PURCHASE_CYCLE;
pub const PANEL_UNAFFORDABLE_TINT: (f32, f32, f32) = (255.0, 70.0, 70.0);
pub const PANEL_UNAFFORDABLE_CYCLE: f32 = 7.0;
