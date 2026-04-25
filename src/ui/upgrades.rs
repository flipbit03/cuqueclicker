use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::GameState;
use crate::game::upgrade::{self, UPGRADES};
use crate::i18n::t;

/// Per-row block height: hotkey+name, indented description, indented cost,
/// blank separator. Used to compute click hit-test rects below.
const ROWS_PER_UPGRADE: u16 = 4;

/// Returns one entry per visible upgrade row: the live `UPGRADES` index
/// and the click-target rect on screen. Aligned 1:1 with the rendered
/// rows, so the click router can map a click coordinate to an
/// `Action::BuyUpgrade(idx)` without re-parsing the panel layout.
pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) -> Vec<(usize, Rect)> {
    let lang = t();
    let available = upgrade::available_ids(state);
    let visible: Vec<usize> = available.iter().take(10).copied().collect();

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
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{} {}", lang.cost, format::big(u.cost)), cost_style),
            ]));
            lines.push(Line::raw(""));
        }
    }

    let p = Paragraph::new(lines)
        .block(Block::bordered().title(lang.upgrades_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);

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
