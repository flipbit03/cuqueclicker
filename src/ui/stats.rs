use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::achievement;
use crate::game::state::{GameState, TICK_HZ};
use crate::i18n::t;

// Per-variant tints for the powerup-breakdown rows. Match the border
// channel hues in `src/ui/border.rs` so the stats panel and the in-game
// HUD pulse colors are visually coherent — a player who saw red for
// Frenzy on the title border should see red for "Frenzy caught" here too.
const LUCKY_FG: Color = Color::Rgb(255, 215, 0); // border LUCKY_TINT
const FRENZY_FG: Color = Color::Rgb(255, 80, 80); // border FRENZY_TINT-ish (legible)
const BUFF_FG: Color = Color::Rgb(220, 140, 255); // border BUFF_TINT
const GREEN_FG: Color = Color::Rgb(120, 230, 140); // border GREEN_COIN_TINT

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let lang = t();
    let session_secs = state.session_ticks / TICK_HZ as u64;
    let total_secs = state.total_play_ticks / TICK_HZ as u64;
    let unlocked = state.achievements_earned.len();
    let total = achievement::count();

    // Default value tint for non-variant rows.
    let neutral = Color::Rgb(240, 220, 180);

    // (label, value, value_color). Variant-specific rows tint the value to
    // match the in-game border channel for that powerup.
    let rows: [(&str, String, Color); 12] = [
        (
            lang.stat_session_time,
            format::duration(session_secs),
            neutral,
        ),
        (lang.stat_total_time, format::duration(total_secs), neutral),
        (
            lang.stat_total_clicks,
            format::big(state.total_clicks as f64),
            neutral,
        ),
        (
            lang.stat_lifetime_cuques,
            format::big_mag(state.lifetime_cuques),
            neutral,
        ),
        (lang.stat_best_fps, format::big_mag(state.best_fps), neutral),
        (
            lang.stat_fingerers_owned,
            format::big(state.fingerers_owned_total() as f64),
            neutral,
        ),
        (
            lang.stat_golden_caught,
            format::big(state.golden_caught as f64),
            neutral,
        ),
        (
            lang.stat_lucky_caught,
            format::big(state.lucky_caught as f64),
            LUCKY_FG,
        ),
        (
            lang.stat_frenzy_caught,
            format::big(state.frenzy_caught as f64),
            FRENZY_FG,
        ),
        (
            lang.stat_buff_caught,
            format::big(state.buff_caught as f64),
            BUFF_FG,
        ),
        (
            lang.stat_green_coin_caught,
            format::big(state.green_coin_caught as f64),
            GREEN_FG,
        ),
        (
            lang.stat_achievements,
            format!("{} / {}", unlocked, total),
            neutral,
        ),
    ];

    let label_w = rows
        .iter()
        .map(|(label, _, _)| label.chars().count())
        .max()
        .unwrap_or(0);

    let lines: Vec<Line> = rows
        .iter()
        .map(|(label, value, value_fg)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<width$}  ", label, width = label_w),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    value.clone(),
                    Style::default().fg(*value_fg).add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .collect();

    let p = Paragraph::new(lines).block(Block::bordered().title(lang.stats_title));
    frame.render_widget(p, area);
}
