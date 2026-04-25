use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::fingerer::{self, FINGERERS};
use crate::game::state::{GameState, PURCHASE_FLASH_TICKS};
use crate::i18n::t;

const ROWS_PER_FINGERER: u16 = 4;
const FLASH_TINT: (f32, f32, f32) = (40.0, 230.0, 80.0);
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
    let buf = frame.buffer_mut();

    for (slot, &fingerer_idx) in visible.iter().enumerate() {
        if slot >= 10 {
            break;
        }
        let flash_ticks = state
            .fingerer_flash_ticks
            .get(fingerer_idx)
            .copied()
            .unwrap_or(0);
        if flash_ticks == 0 {
            continue;
        }
        let strength = smoothstep(flash_ticks as f32 / PURCHASE_FLASH_TICKS as f32);
        let row_start = inner_y + slot as u16 * ROWS_PER_FINGERER;
        // Carrier eases from the resting gray up to pure white as the flash
        // strength rises, then back to gray as it fades — no jarring cut.
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
                let contribution = wave01 * strength;
                let r = carrier_r + (FLASH_TINT.0 - carrier_r) * contribution;
                let g = carrier_g + (FLASH_TINT.1 - carrier_g) * contribution;
                let b = carrier_b + (FLASH_TINT.2 - carrier_b) * contribution;
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
