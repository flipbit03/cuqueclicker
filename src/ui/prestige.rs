use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::GameState;
use crate::i18n::t;

/// Output of the Prestige panel render — three optional click rects.
/// Exactly which ones are populated depends on whether prestige is
/// available AND whether the player is mid-confirmation:
///   - **Resting (available, no pending confirm)**: `reset` populated;
///     `yes` / `no` default. Clicking `reset` flips into pending state.
///   - **Confirming (pending)**: `yes` / `no` populated; `reset`
///     default. Clicking `yes` fires `Action::PrestigeReset`, `no`
///     cancels and returns to resting.
///   - **Unavailable**: all three default (panel just shows lifetime-
///     needed hint).
#[derive(Clone, Copy, Default)]
pub struct PrestigeRects {
    pub reset: Rect,
    pub yes: Rect,
    pub no: Rect,
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    state: &GameState,
    mouse_pos: Option<(u16, u16)>,
    confirm_pending: bool,
) -> PrestigeRects {
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

    // Track which lines carry click targets so we can rebuild their
    // screen rects after rendering. Different layouts depending on
    // whether the player is mid-confirmation.
    let mut reset_line_idx: Option<usize> = None;
    let mut yes_line_idx: Option<usize> = None;
    let mut no_line_idx: Option<usize> = None;
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
        if confirm_pending {
            // Confirmation block: question + warning + Y/N buttons.
            // The question line is loud red so the player can't miss it.
            lines.push(Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_question),
                Style::default()
                    .fg(Color::Rgb(255, 90, 90))
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_warning),
                Style::default().fg(Color::Rgb(220, 180, 120)),
            )));
            lines.push(Line::raw(""));
            yes_line_idx = Some(lines.len());
            lines.push(Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_yes),
                Style::default()
                    .fg(Color::Rgb(255, 100, 100))
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            )));
            no_line_idx = Some(lines.len());
            lines.push(Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_no),
                Style::default()
                    .fg(Color::Rgb(120, 220, 120))
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            )));
        } else {
            reset_line_idx = Some(lines.len());
            lines.push(Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_hint),
                Style::default().fg(Color::Rgb(220, 140, 255)),
            )));
        }
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

    // Project each line-index into a screen rect within the bordered
    // block's interior (`area.x + 1, area.y + 1 + idx`, inner width).
    // Then apply a hover lift on whichever rect the mouse is over so
    // the mouse-first player sees it as a button.
    let project = |idx: Option<usize>| -> Rect {
        let Some(idx) = idx else {
            return Rect::default();
        };
        if area.width < 2 || area.height < 2 {
            return Rect::default();
        }
        let inner_w = area.width - 2;
        let y = area.y + 1 + idx as u16;
        if y >= area.y + area.height - 1 {
            return Rect::default();
        }
        Rect {
            x: area.x + 1,
            y,
            width: inner_w,
            height: 1,
        }
    };
    let rects = PrestigeRects {
        reset: project(reset_line_idx),
        yes: project(yes_line_idx),
        no: project(no_line_idx),
    };
    if let Some((mx, my)) = mouse_pos {
        let buf = frame.buffer_mut();
        for r in [rects.reset, rects.yes, rects.no] {
            if r.width == 0 {
                continue;
            }
            if !(mx >= r.x && mx < r.x + r.width && my == r.y) {
                continue;
            }
            for dx in 0..r.width {
                let cx = r.x + dx;
                if cx >= buf.area.x + buf.area.width {
                    break;
                }
                let cell = &mut buf[(cx, r.y)];
                cell.set_fg(Color::Rgb(255, 255, 255));
                cell.set_bg(Color::Rgb(40, 30, 50));
                cell.modifier.insert(Modifier::BOLD);
            }
        }
    }
    rects
}
