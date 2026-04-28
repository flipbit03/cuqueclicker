use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::GameState;
use crate::i18n::t;

/// Draws the Prestige panel and returns the click-target rect for the
/// "Press [r] to reset and claim" confirm line. The rect is `Rect::default()`
/// when no prestige is currently available — i.e. nothing to confirm.
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
    mouse_pos: Option<(u16, u16)>,
) -> Rect {
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
            format!("+{:.0}% {}", bonus_pct, lang.fps_unit),
            Style::default().fg(Color::Rgb(120, 230, 120)),
        ),
    ]));
    lines.push(Line::raw(""));

    // Track the line index the confirm hint lands on so we can compute its
    // screen rect after rendering. Layout below is fixed-order so the
    // confirm always sits at index 6 when `available > 0`.
    let mut confirm_line_idx: Option<usize> = None;
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
        confirm_line_idx = Some(lines.len());
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

    // Compute the confirm-line rect from the bordered block's interior:
    // x = area.x + 1, y = area.y + 1 + line_idx, full inner width.
    let mut confirm_rect = Rect::default();
    if let Some(idx) = confirm_line_idx
        && area.width >= 2
        && area.height >= 2
    {
        let inner_w = area.width - 2;
        let y = area.y + 1 + idx as u16;
        if y < area.y + area.height - 1 {
            confirm_rect = Rect {
                x: area.x + 1,
                y,
                width: inner_w,
                height: 1,
            };
            // Subtle hover lift on the clickable confirm line so the
            // mouse-first player can SEE it's a button. Mouse-only path:
            // hover → bright + bg fill; click is wired in app.rs.
            if let Some((mx, my)) = mouse_pos
                && mx >= confirm_rect.x
                && mx < confirm_rect.x + confirm_rect.width
                && my == confirm_rect.y
            {
                let buf = frame.buffer_mut();
                for dx in 0..confirm_rect.width {
                    let cx = confirm_rect.x + dx;
                    if cx >= buf.area.x + buf.area.width {
                        break;
                    }
                    let cell = &mut buf[(cx, confirm_rect.y)];
                    cell.set_fg(Color::Rgb(255, 230, 255));
                    cell.set_bg(Color::Rgb(40, 30, 50));
                    cell.modifier.insert(Modifier::BOLD);
                }
            }
        }
    }
    confirm_rect
}
