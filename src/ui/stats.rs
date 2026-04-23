use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::achievement;
use crate::game::state::{GameState, TICK_HZ};
use crate::i18n::t;

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let lang = t();
    let session_secs = state.session_ticks / TICK_HZ as u64;
    let total_secs = state.total_play_ticks / TICK_HZ as u64;
    let unlocked = state.achievements_earned.len();
    let total = achievement::count();

    let rows = [
        (lang.stat_session_time, format::duration(session_secs)),
        (lang.stat_total_time, format::duration(total_secs)),
        (
            lang.stat_total_clicks,
            format::big(state.total_clicks as f64),
        ),
        (
            lang.stat_lifetime_cuques,
            format::big(state.lifetime_cuques),
        ),
        (lang.stat_best_fps, format::rate(state.best_fps)),
        (
            lang.stat_fingerers_owned,
            format::big(state.fingerers_owned_total() as f64),
        ),
        (
            lang.stat_golden_caught,
            format::big(state.golden_caught as f64),
        ),
        (lang.stat_achievements, format!("{} / {}", unlocked, total)),
    ];

    let label_w = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);

    let lines: Vec<Line> = rows
        .iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<width$}  ", label, width = label_w),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    value.clone(),
                    Style::default()
                        .fg(Color::Rgb(240, 220, 180))
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .collect();

    let p = Paragraph::new(lines).block(Block::bordered().title(lang.stats_title));
    frame.render_widget(p, area);
}
