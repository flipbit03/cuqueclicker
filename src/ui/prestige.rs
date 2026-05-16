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
    let owned = state.prestige;
    let available = state.prestige_available();
    let bonus_pct = state.prestige as f64;
    // Saturating arithmetic — with the prestige cap removed,
    // `(owned + 1).pow(2) * 1_000_000` would overflow u64 around
    // owned > 1.36e7 (debug-panic, release-wrap). Saturating keeps
    // the displayed-only number sane at extreme values.
    let next_threshold = owned
        .saturating_add(1)
        .saturating_mul(owned.saturating_add(1))
        .saturating_mul(1_000_000);

    // Render the bordered panel chrome first; we paint into its inner
    // area below using independent sub-rects so the click rects for
    // the action buttons don't depend on a single Paragraph's wrap
    // behavior — earlier off-by-one bugs all stemmed from line-index
    // math drifting under soft-wrap.
    let block = Block::bordered().title(lang.prestige_title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut rects = PrestigeRects::default();
    if inner.width == 0 || inner.height == 0 {
        return rects;
    }

    // ---- Info section (top): cuques owned, fps bonus, available ----
    let mut info_lines: Vec<Line> = Vec::new();
    info_lines.push(Line::from(vec![
        Span::raw(format!("  {}: ", lang.prestige_owned_label)),
        Span::styled(
            format::big(owned as f64),
            Style::default()
                .fg(Color::Rgb(255, 215, 0))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  ({})", lang.prestige_currency)),
    ]));
    info_lines.push(Line::raw(""));
    info_lines.push(Line::from(vec![
        Span::raw(format!("  {}: ", lang.prestige_bonus_label)),
        Span::styled(
            format!("+{:.0}% {}", bonus_pct, lang.fps_unit),
            Style::default().fg(Color::Rgb(120, 230, 120)),
        ),
    ]));
    if available > 0 {
        info_lines.push(Line::raw(""));
        info_lines.push(Line::from(vec![
            Span::raw(format!("  {}: ", lang.prestige_available_label)),
            Span::styled(
                format!("+{}", format::big(available as f64)),
                Style::default()
                    .fg(Color::Rgb(255, 215, 0))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        info_lines.push(Line::raw(""));
        for l in lang.prestige_not_enough.lines() {
            info_lines.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
        info_lines.push(Line::raw(""));
        info_lines.push(Line::from(vec![
            Span::raw(format!("  {}: ", lang.prestige_lifetime_needed)),
            Span::styled(
                format::big(next_threshold as f64),
                Style::default().fg(Color::Rgb(200, 180, 140)),
            ),
        ]));
    }

    // ---- Action section (bottom): reset hint OR yes/no buttons ----
    // Reserve a fixed-height bottom strip whose lines we render WITHOUT
    // wrap so each Vec line maps 1:1 to a visual row inside the strip.
    // Click rects are computed from the strip's `(y, height)`, never
    // from the Paragraph above it. That decoupling was the structural
    // fix for the off-by-one bug where wrap on any earlier line shifted
    // the buttons one row away from their click rects.
    let action_lines: Vec<(Line, Option<ActionTarget>)> = if available == 0 {
        Vec::new()
    } else if confirm_pending {
        let mut v: Vec<(Line, Option<ActionTarget>)> = Vec::new();
        v.push((
            Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_question),
                Style::default()
                    .fg(Color::Rgb(255, 90, 90))
                    .add_modifier(Modifier::BOLD),
            )),
            None,
        ));
        for chunk in lang.prestige_confirm_warning.lines() {
            v.push((
                Line::from(Span::styled(
                    format!("  {chunk}"),
                    Style::default().fg(Color::Rgb(220, 180, 120)),
                )),
                None,
            ));
        }
        v.push((Line::raw(""), None));
        v.push((
            Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_yes),
                Style::default()
                    .fg(Color::Rgb(255, 100, 100))
                    .add_modifier(Modifier::BOLD),
            )),
            Some(ActionTarget::Yes),
        ));
        // Blank row between Yes / No so a touch / mouse player has a
        // forgiving gap between the two click targets.
        v.push((Line::raw(""), None));
        v.push((
            Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_no),
                Style::default()
                    .fg(Color::Rgb(120, 220, 120))
                    .add_modifier(Modifier::BOLD),
            )),
            Some(ActionTarget::No),
        ));
        v
    } else {
        vec![(
            Line::from(Span::styled(
                format!("  {}", lang.prestige_confirm_hint),
                Style::default().fg(Color::Rgb(220, 140, 255)),
            )),
            Some(ActionTarget::Reset),
        )]
    };

    let action_h = action_lines.len() as u16;
    let chunks = if action_h > 0 && action_h < inner.height {
        Layout::vertical([Constraint::Min(1), Constraint::Length(action_h)]).split(inner)
    } else {
        // Action strip would overflow the panel — fall back to drawing
        // info only. The action buttons can't render so click rects
        // stay default.
        Layout::vertical([Constraint::Min(1), Constraint::Length(0)]).split(inner)
    };
    let info_area = chunks[0];
    let action_area = chunks[1];

    // Info section uses Wrap { trim: false } so long lines (e.g. pt_BR
    // currency name) wrap inside the info strip without affecting the
    // action strip below.
    let info_para = Paragraph::new(info_lines).wrap(Wrap { trim: false });
    frame.render_widget(info_para, info_area);

    if action_h == 0 || action_area.height == 0 {
        return rects;
    }
    // Action section: NO wrap — each Line is exactly one visual row.
    // Click rects derive from `action_area.y + offset` where `offset`
    // is the Vec index (which equals the visual row because wrap is
    // off).
    let mut action_para_lines: Vec<Line> = Vec::with_capacity(action_lines.len());
    for (line, target) in action_lines.iter() {
        let row_y = action_area.y + action_para_lines.len() as u16;
        let rect = Rect {
            x: action_area.x,
            y: row_y,
            width: action_area.width,
            height: 1,
        };
        match target {
            Some(ActionTarget::Reset) => rects.reset = rect,
            Some(ActionTarget::Yes) => rects.yes = rect,
            Some(ActionTarget::No) => rects.no = rect,
            None => {}
        }
        action_para_lines.push(line.clone());
    }
    let action_para = Paragraph::new(action_para_lines);
    frame.render_widget(action_para, action_area);

    // Hover lift on whichever click rect the mouse is over.
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

#[derive(Copy, Clone)]
enum ActionTarget {
    Reset,
    Yes,
    No,
}
