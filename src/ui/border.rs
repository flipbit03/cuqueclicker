use ratatui::{prelude::*, widgets::*};

use crate::game::state::{Buff, GameState, LUCKY_FLASH_TICKS, PURCHASE_FLASH_TICKS};

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
    let purchase_s = plateau_fade(state.purchase_flash_ticks, PURCHASE_FLASH_TICKS);
    let lucky_s = plateau_fade(state.lucky_flash_ticks, LUCKY_FLASH_TICKS);
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
    let activity = purchase_s.max(lucky_s).max(frenzy_s).max(buff_s);
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
fn plateau_fade(remaining: u32, total: u32) -> f32 {
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
