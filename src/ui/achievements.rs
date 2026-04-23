use ratatui::{prelude::*, widgets::*};

use crate::game::state::GameState;
use crate::i18n::t;

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let lang = t();
    let mut lines: Vec<Line> = Vec::new();
    let unlocked = state.achievements_earned.len();
    let total = lang.achievement_names.len();

    lines.push(Line::from(vec![
        Span::styled(
            format!("{} / {} ", unlocked, total),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(lang.ach_summary, Style::default().fg(Color::DarkGray)),
    ]));
    lines.push(Line::raw(""));

    for (i, name) in lang.achievement_names.iter().enumerate() {
        let unlocked = state.has_achievement_idx(i);
        let (marker, name_style) = if unlocked {
            (
                lang.ach_unlocked,
                Style::default()
                    .fg(Color::Rgb(255, 215, 0))
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (lang.ach_locked, Style::default().fg(Color::DarkGray))
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", marker), name_style),
            Span::styled(name.to_string(), name_style),
        ]));
        let desc = lang.achievement_descs.get(i).copied().unwrap_or("");
        let desc_style = if unlocked {
            Style::default().fg(Color::Rgb(180, 180, 180))
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(desc.to_string(), desc_style),
        ]));
        lines.push(Line::raw(""));
    }

    let p = Paragraph::new(lines)
        .block(Block::bordered().title(lang.achievements_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}
