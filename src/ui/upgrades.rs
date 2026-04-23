use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::GameState;
use crate::game::upgrade::{self, UPGRADES};
use crate::i18n::t;

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) -> Vec<usize> {
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

    visible
}
