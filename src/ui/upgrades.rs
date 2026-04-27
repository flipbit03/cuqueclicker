use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::{GameState, PURCHASE_FLASH_TICKS};
use crate::game::upgrade::{self, UPGRADES};
use crate::i18n::t;
use crate::ui::border;

/// Per-row block height: hotkey+name, indented description, indented cost,
/// blank separator. Used to compute click hit-test rects below.
const ROWS_PER_UPGRADE: u16 = 4;
/// Indent applied to wrapped continuation lines so a long description hangs
/// under the title indent instead of falling back to col 0.
const HANGING_INDENT: &str = "    ";
const FLASH_TINT: (f32, f32, f32) = (40.0, 230.0, 80.0);
const UNAFFORDABLE_TINT: (f32, f32, f32) = (255.0, 60.0, 60.0);
const FLASH_REST: (f32, f32, f32) = (200.0, 200.0, 210.0);
const FLASH_CARRIER: (f32, f32, f32) = (255.0, 255.0, 255.0);
const FLASH_CYCLE: f32 = 11.0;

/// Returns one entry per visible upgrade row: the live `UPGRADES` index
/// and the click-target rect on screen. Aligned 1:1 with the rendered
/// rows, so the click router can map a click coordinate to an
/// `Action::BuyUpgrade(idx)` without re-parsing the panel layout.
pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) -> Vec<(usize, Rect)> {
    let lang = t();
    let available = upgrade::available_ids(state);
    let visible: Vec<usize> = available.iter().take(10).copied().collect();
    // Inner content width (panel width minus the bordered block's chrome) —
    // used to pre-wrap descriptions with a hanging indent so continuation
    // lines line up under the title text instead of falling back to col 0.
    let desc_width = area.width.saturating_sub(2 + HANGING_INDENT.len() as u16) as usize;

    let mut lines: Vec<Line> = Vec::new();

    if visible.is_empty() {
        for l in lang.upgrades_none.lines() {
            lines.push(Line::from(l.to_string()).style(Style::default().fg(Color::DarkGray)));
        }
    } else {
        for (slot, &u_idx) in visible.iter().enumerate() {
            let hotkey = if slot == 9 {
                '0'
            } else {
                (b'1' + slot as u8) as char
            };
            let u = &UPGRADES[u_idx];
            let affordable = state.cuques >= u.cost;
            let name = lang.upgrade_names.get(u_idx).copied().unwrap_or("?");
            let desc = lang.upgrade_descs.get(u_idx).copied().unwrap_or("");
            let cost_style = if affordable {
                Style::default()
                    .fg(Color::Rgb(0, 255, 80))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(220, 70, 70))
            };
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", hotkey), Style::default().fg(Color::Yellow)),
                Span::styled(
                    name.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]));
            // J13: pre-wrap with a hanging indent so wrapped lines align
            // under the title text. The panel renders with `Wrap { trim }`
            // disabled below, so each entry here is a finished line.
            for desc_line in wrap_hanging(desc, desc_width) {
                lines.push(Line::from(vec![Span::styled(
                    desc_line,
                    Style::default().fg(Color::DarkGray),
                )]));
            }
            lines.push(Line::from(vec![
                Span::raw(HANGING_INDENT),
                Span::styled(format!("{} {}", lang.cost, format::big(u.cost)), cost_style),
            ]));
            lines.push(Line::raw(""));
        }
    }

    let p = Paragraph::new(lines).block(Block::bordered().title(lang.upgrades_title));
    frame.render_widget(p, area);

    paint_flashes(frame, area, state, &visible);

    // Panel-border flash mirrors sidebar.rs: green on any active purchase
    // flash for visible upgrades, red on any active unaffordable flash.
    let any_purchase = visible
        .iter()
        .filter_map(|&i| state.upgrade_flash_ticks.get(i).copied())
        .max()
        .unwrap_or(0);
    let any_unaff = visible
        .iter()
        .filter_map(|&i| state.upgrade_unaffordable_flash.get(i).copied())
        .max()
        .unwrap_or(0);
    // Carrier-strength is timing-only — bulk-buy scaling is applied as a
    // wave-amplitude bonus inside `paint_border_flash`, not as a carrier
    // dampener — so the panel border reaches full white on every flash.
    let purchase_strength = border::plateau_fade(any_purchase, PURCHASE_FLASH_TICKS);
    let unaff_strength = border::plateau_fade(any_unaff, PURCHASE_FLASH_TICKS / 2);
    // Unaffordable wins when active — see the matching comment in
    // sidebar.rs. Ensures a click on an unaffordable upgrade always lights
    // the panel border red, even if a recent successful purchase's green
    // hasn't fully decayed yet.
    if unaff_strength > 0.001 {
        border::paint_border_flash(
            frame,
            area,
            state,
            border::PANEL_UNAFFORDABLE_TINT,
            border::PANEL_UNAFFORDABLE_CYCLE,
            unaff_strength,
        );
    } else if purchase_strength > 0.001 {
        border::paint_border_flash(
            frame,
            area,
            state,
            border::PANEL_PURCHASE_TINT,
            border::PANEL_PURCHASE_CYCLE,
            purchase_strength,
        );
    }

    // Build per-row click rects. Inside the bordered block: x+1 / y+1 origin,
    // -2 width / -2 height. Each row block is 3 used lines + 1 blank; clicks
    // anywhere on those 3 used lines trigger the buy. NB: long descriptions
    // can wrap, but the headline (hotkey + name + cost block) is always
    // ROWS_PER_UPGRADE rows in the source `lines`. Using ROWS_PER_UPGRADE-1
    // (skip the trailing blank) keeps us conservative — clicks on the blank
    // separator do nothing.
    if area.width < 3 || area.height < 3 {
        return Vec::new();
    }
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    let mut rows: Vec<(usize, Rect)> = Vec::new();
    for (slot, &u_idx) in visible.iter().enumerate() {
        let row_top = slot as u16 * ROWS_PER_UPGRADE;
        if row_top >= inner_h {
            break;
        }
        let height = (ROWS_PER_UPGRADE - 1).min(inner_h - row_top);
        rows.push((
            u_idx,
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

/// Word-wrap `text` at `width` columns, prepending `HANGING_INDENT` to every
/// produced line. Returns at least one line (the indent + first word) so a
/// caller never has to special-case empty input.
fn wrap_hanging(text: &str, width: usize) -> Vec<String> {
    let indent = HANGING_INDENT;
    if width <= indent.len() {
        return vec![format!("{indent}{text}")];
    }
    let avail = width - indent.len();
    let mut out: Vec<String> = Vec::new();
    let mut row = String::new();
    for word in text.split_whitespace() {
        if row.is_empty() {
            row.push_str(word);
        } else if row.len() + 1 + word.len() <= avail {
            row.push(' ');
            row.push_str(word);
        } else {
            out.push(format!("{indent}{row}"));
            row.clear();
            row.push_str(word);
        }
    }
    if !row.is_empty() || out.is_empty() {
        out.push(format!("{indent}{row}"));
    }
    out
}

fn paint_flashes(frame: &mut Frame, area: Rect, state: &GameState, visible: &[usize]) {
    if area.width < 3 || area.height < 3 {
        return;
    }
    // Steady phase clock — see sidebar.rs for the rationale. Decoupled
    // from the HUD-border's speed so concurrent shimmers don't entrain.
    let phase = state.steady_phase as f32;
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_right = area.x + area.width - 1;
    let inner_bottom = area.y + area.height - 1;
    // Bulk-buy boosts wave amplitude only; carrier-strength is timing-only
    // so the row reaches full white-↔-tint contrast on every flash.
    let bulk_amp = state.purchase_flash_strength.clamp(1.0, 3.0);
    let buf = frame.buffer_mut();

    for (slot, &u_idx) in visible.iter().enumerate() {
        if slot >= 10 {
            break;
        }
        let purchase_ticks = state.upgrade_flash_ticks.get(u_idx).copied().unwrap_or(0);
        let unaff_ticks = state
            .upgrade_unaffordable_flash
            .get(u_idx)
            .copied()
            .unwrap_or(0);
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
        let row_start = inner_y + slot as u16 * ROWS_PER_UPGRADE;
        let carrier_r = FLASH_REST.0 + (FLASH_CARRIER.0 - FLASH_REST.0) * strength;
        let carrier_g = FLASH_REST.1 + (FLASH_CARRIER.1 - FLASH_REST.1) * strength;
        let carrier_b = FLASH_REST.2 + (FLASH_CARRIER.2 - FLASH_REST.2) * strength;
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
