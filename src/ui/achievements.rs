use ratatui::{prelude::*, widgets::*};

use crate::game::state::{ACHIEVEMENT_FLASH_TICKS, GameState};
use crate::i18n::t;
use crate::ui::border;

const HANGING_INDENT: &str = "    ";

pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let lang = t();
    let mut lines: Vec<Line> = Vec::new();
    let unlocked = state.achievements_earned.len();
    let total = lang.achievement_names.len();
    let desc_width = area.width.saturating_sub(2 + HANGING_INDENT.len() as u16) as usize;

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
        for desc_line in wrap_hanging(desc, desc_width) {
            lines.push(Line::from(vec![Span::styled(desc_line, desc_style)]));
        }
        lines.push(Line::raw(""));
    }

    let p = Paragraph::new(lines).block(Block::bordered().title(lang.achievements_title));
    frame.render_widget(p, area);

    // When the player is looking at this panel AND an achievement just
    // unlocked, mirror the HUD title border's gold pulse on this panel's
    // border so the celebration lands on whatever they're staring at —
    // not just the bar at the top. Uses `steady_phase` (timing-only) so
    // it never entrains with other concurrent shimmers, and the same
    // gold tint/cycle the HUD title uses so the visual reads as "the
    // same event."
    let strength = border::plateau_fade(state.achievement_flash_ticks, ACHIEVEMENT_FLASH_TICKS);
    if strength > 0.001 {
        border::paint_border_flash(
            frame,
            area,
            state,
            border::PANEL_ACHIEVEMENT_TINT,
            border::PANEL_ACHIEVEMENT_CYCLE,
            strength,
        );
    }
}

/// Word-wrap with a leading hanging indent applied to every line, so wrapped
/// continuations align under the title indent instead of falling back to
/// column 0. Mirrors `ui::upgrades::wrap_hanging`.
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
