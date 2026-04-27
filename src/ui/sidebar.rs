use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::fingerer::{self, FINGERERS};
use crate::game::state::{GameState, PURCHASE_FLASH_TICKS};
use crate::i18n::t;
use crate::ui::border;

const ROWS_PER_FINGERER: u16 = 4;
const FLASH_TINT: (f32, f32, f32) = (40.0, 230.0, 80.0);
const UNAFFORDABLE_TINT: (f32, f32, f32) = (255.0, 60.0, 60.0);
// Resting color the flash starts from / returns to (neutral gray similar to
// default terminal text, so the transition in and out is gentle).
const FLASH_REST: (f32, f32, f32) = (200.0, 200.0, 210.0);
// Active carrier: pure white so the wave pulses white<->green with maximum
// contrast during the flash.
const FLASH_CARRIER: (f32, f32, f32) = (255.0, 255.0, 255.0);
const FLASH_CYCLE: f32 = 11.0;

/// Returns one entry per visible fingerer row: the live `FINGERERS` index
/// and the click-target rect on screen. Aligned 1:1 with the rendered rows
/// so the click router can map a click coordinate to an
/// `Action::BuyFingerer` without re-parsing the panel layout.
pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) -> Vec<(usize, Rect)> {
    let lang = t();
    let mut lines: Vec<Line> = Vec::new();
    let visible: Vec<usize> = (0..FINGERERS.len())
        .filter(|&i| fingerer::visible(i, state.fingerer_count_idx(i), state.lifetime_cuques))
        .collect();

    for (slot, &i) in visible.iter().enumerate() {
        if slot >= 10 {
            break;
        }
        let hotkey = if slot == 9 {
            '0'
        } else {
            (b'1' + slot as u8) as char
        };
        let k = &FINGERERS[i];
        let owned = state.fingerer_count_idx(i);
        let cost = state.cost(i);
        let affordable = state.can_buy(i);
        let cost_style = if affordable {
            Style::default()
                .fg(Color::Rgb(0, 255, 80))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(220, 70, 70))
        };
        let name = lang.fingerer_names.get(i).copied().unwrap_or("?");
        lines.push(Line::from(vec![
            Span::styled(format!("[{}] ", hotkey), Style::default().fg(Color::Yellow)),
            Span::raw(k.icon),
            Span::raw(" "),
            Span::styled(
                name.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::raw(format!("    {}: {}  ", lang.owned, owned)),
            Span::styled(format!("{} {}", lang.cost, format::big(cost)), cost_style),
        ]));
        let mult = state.fingerer_mult(i);
        let effective = k.fps_per_unit * mult;
        let mult_tag = if mult > 1.0001 {
            format!(" (x{:.1})", mult)
        } else {
            String::new()
        };
        lines.push(Line::from(format!(
            "    +{} {}{}",
            format::rate(effective),
            lang.fps_each,
            mult_tag,
        )));
        lines.push(Line::raw(""));
    }
    let p = Paragraph::new(lines).block(Block::bordered().title(lang.fingerers_title));
    frame.render_widget(p, area);

    paint_flashes(frame, area, state, &visible);

    // Panel-border flash on purchase: green if any fingerer's row flash is
    // burning, red if any unaffordable click was just rejected. Mirrors the
    // HUD title's behavior so the whole panel pulses, not just the row.
    let any_purchase = visible
        .iter()
        .filter_map(|&i| state.fingerer_flash_ticks.get(i).copied())
        .max()
        .unwrap_or(0);
    let any_unaff = visible
        .iter()
        .filter_map(|&i| state.fingerer_unaffordable_flash.get(i).copied())
        .max()
        .unwrap_or(0);
    // Carrier-strength is timing-only — no bulk-buy scaling — so the
    // panel border reaches full white on every flash. The wave amplitude
    // booster (bulk-buy intensity) is handled inside `paint_border_flash`.
    let purchase_strength = border::plateau_fade(any_purchase, PURCHASE_FLASH_TICKS);
    let unaff_strength = border::plateau_fade(any_unaff, PURCHASE_FLASH_TICKS / 2);
    if purchase_strength > 0.001 {
        border::paint_border_flash(
            frame,
            area,
            state,
            border::PANEL_PURCHASE_TINT,
            border::PANEL_PURCHASE_CYCLE,
            purchase_strength,
        );
    } else if unaff_strength > 0.001 {
        border::paint_border_flash(
            frame,
            area,
            state,
            border::PANEL_UNAFFORDABLE_TINT,
            border::PANEL_UNAFFORDABLE_CYCLE,
            unaff_strength,
        );
    }

    if area.width < 3 || area.height < 3 {
        return Vec::new();
    }
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    let mut rows: Vec<(usize, Rect)> = Vec::new();
    for (slot, &i) in visible.iter().enumerate() {
        if slot >= 10 {
            break;
        }
        let row_top = slot as u16 * ROWS_PER_FINGERER;
        if row_top >= inner_h {
            break;
        }
        // Skip the trailing blank separator (3 useful rows out of 4).
        let height = (ROWS_PER_FINGERER - 1).min(inner_h - row_top);
        rows.push((
            i,
            Rect {
                x: inner_x,
                y: inner_y + row_top,
                width: inner_w,
                height,
            },
        ));
    }
    rows
}

fn paint_flashes(frame: &mut Frame, area: Rect, state: &GameState, visible: &[usize]) {
    if area.width < 3 || area.height < 3 {
        return;
    }
    let phase = state.border_phase as f32;
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_right = area.x + area.width - 1;
    let inner_bottom = area.y + area.height - 1;
    // Bulk-buy intensifier: 1.0..3.0 multiplier on the WAVE AMPLITUDE only
    // (not the carrier brightness). A single buy already pulses fully white
    // ↔ tint with maximum contrast; bulk-buy just pushes the tint peaks
    // harder so a max-buy reads louder without dimming the white carrier.
    let bulk_amp = state.purchase_flash_strength.clamp(1.0, 3.0);
    let buf = frame.buffer_mut();

    for (slot, &fingerer_idx) in visible.iter().enumerate() {
        if slot >= 10 {
            break;
        }
        let purchase_ticks = state
            .fingerer_flash_ticks
            .get(fingerer_idx)
            .copied()
            .unwrap_or(0);
        let unaff_ticks = state
            .fingerer_unaffordable_flash
            .get(fingerer_idx)
            .copied()
            .unwrap_or(0);
        // Per-row tint priority: green purchase wins over red unaffordable
        // when both happen to be running (e.g. user clicked unaffordable,
        // then bought 1 of an affordable adjacent fingerer that aliased on
        // index — defensive only). `strength` here is the carrier blend
        // factor (timing only, 0..1); `amp` boosts the wave's tint
        // contribution for bulk buys.
        let (strength, tint, amp) = if purchase_ticks > 0 {
            (
                smoothstep(purchase_ticks as f32 / PURCHASE_FLASH_TICKS as f32),
                FLASH_TINT,
                bulk_amp,
            )
        } else if unaff_ticks > 0 {
            (
                smoothstep(unaff_ticks as f32 / (PURCHASE_FLASH_TICKS as f32 / 2.0)),
                UNAFFORDABLE_TINT,
                1.0,
            )
        } else {
            continue;
        };
        if strength <= 0.001 {
            continue;
        }
        let row_start = inner_y + slot as u16 * ROWS_PER_FINGERER;
        // Carrier eases from resting gray to pure WHITE on full strength —
        // never washed out by bulk-buy. The wave below paints tint on top.
        let carrier_r = FLASH_REST.0 + (FLASH_CARRIER.0 - FLASH_REST.0) * strength;
        let carrier_g = FLASH_REST.1 + (FLASH_CARRIER.1 - FLASH_REST.1) * strength;
        let carrier_b = FLASH_REST.2 + (FLASH_CARRIER.2 - FLASH_REST.2) * strength;
        // First 3 of the 4 rows hold the fingerer's text; the 4th is the
        // blank separator.
        for dy in 0..3u16 {
            let row = row_start + dy;
            if row >= inner_bottom {
                break;
            }
            for col in inner_x..inner_right {
                let rel = (col - area.x) as f32;
                let wave01 =
                    (((rel + phase) * std::f32::consts::TAU / FLASH_CYCLE).sin() + 1.0) * 0.5;
                let contribution = (wave01 * strength * amp).min(1.0);
                let r = carrier_r + (tint.0 - carrier_r) * contribution;
                let g = carrier_g + (tint.1 - carrier_g) * contribution;
                let b = carrier_b + (tint.2 - carrier_b) * contribution;
                let cell = &mut buf[(col, row)];
                cell.set_fg(Color::Rgb(r as u8, g as u8, b as u8));
                cell.modifier.insert(Modifier::BOLD);
            }
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
