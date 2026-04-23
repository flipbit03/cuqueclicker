use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::GameState;
use crate::i18n::t;

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let lang = t();
    let mut lines: Vec<Line> = Vec::new();

    let owned = state.prestige;
    let available = state.prestige_available();
    let bonus_pct = state.prestige as f64 * 1.0;
    let next_threshold = (owned + 1).pow(2) * 1_000_000;

    lines.push(Line::from(vec![
        Span::raw(format!("  {}: ", lang.prestige_owned_label)),
        Span::styled(
            format::big(owned as f64),
            Style::default()
                .fg(Color::Rgb(255, 215, 0))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  ({})", lang.prestige_currency)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw(format!("  {}: ", lang.prestige_bonus_label)),
        Span::styled(
            format!("+{:.0}% FPS", bonus_pct),
            Style::default().fg(Color::Rgb(120, 230, 120)),
        ),
    ]));
    lines.push(Line::raw(""));

    if available > 0 {
        lines.push(Line::from(vec![
            Span::raw(format!("  {}: ", lang.prestige_available_label)),
            Span::styled(
                format!("+{}", format::big(available as f64)),
                Style::default()
                    .fg(Color::Rgb(255, 215, 0))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", lang.prestige_confirm_hint),
            Style::default().fg(Color::Rgb(220, 140, 255)),
        )));
    } else {
        for l in lang.prestige_not_enough.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {}", l),
                Style::default().fg(Color::DarkGray),
            )));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::raw(format!("  {}: ", lang.prestige_lifetime_needed)),
            Span::styled(
                format::big(next_threshold as f64),
                Style::default().fg(Color::Rgb(200, 180, 140)),
            ),
        ]));
    }

    let p = Paragraph::new(lines)
        .block(Block::bordered().title(lang.prestige_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}
