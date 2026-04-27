//! Achievement-unlock toast overlay.
//!
//! Rendered *over* the biscuit area so it's prominent without disturbing
//! the right-side panel. The sim populates `state.active_unlock_id` /
//! `active_unlock_ticks` from the queue in `state.newly_unlocked`; this
//! module just translates that into a brief gold-bordered popup.
//!
//! Lives ~4s on screen (TOAST_TICKS), with smoothstep ease-in/out on the
//! border + text so it never snaps in or out.
use ratatui::{prelude::*, widgets::*};

use crate::game::achievement::ACHIEVEMENTS;
use crate::game::state::{GameState, TOAST_TICKS};
use crate::i18n::t;

/// Draw the toast over `area` if one is active. No-op when nothing's queued.
pub fn draw(frame: &mut Frame, area: Rect, state: &GameState) {
    let Some(id) = state.active_unlock_id.as_deref() else {
        return;
    };
    if state.active_unlock_ticks == 0 {
        return;
    }
    // Resolve catalog index → localized name. Unknown ids are skipped
    // silently — never show a broken "?" toast.
    let Some(idx) = ACHIEVEMENTS.iter().position(|a| a.id == id) else {
        return;
    };
    let lang = t();
    let Some(name) = lang.achievement_names.get(idx).copied() else {
        return;
    };

    // Ease in/out via two smoothsteps — full strength in the middle, fade
    // at both ends. Multiplied through to the border + text alpha.
    let life = state.active_unlock_ticks as f32 / TOAST_TICKS as f32;
    let strength = ease_in_out(life);
    if strength <= 0.01 {
        return;
    }

    // Header stays ASCII so it renders cleanly in every terminal — emojis
    // don't widen reliably across the variety of fonts CuqueClicker runs in.
    let header_plain = header_plain();
    let body = format!("  {name}  ");
    let inner_w = (header_plain.chars().count().max(body.chars().count()) + 2) as u16;
    let w = (inner_w + 2).min(area.width.saturating_sub(2));
    let h: u16 = 5;
    if area.width < w + 2 || area.height < h + 2 {
        return;
    }

    // Centered horizontally, ~1/4 of the way down so it sits near the top
    // without overlapping the HUD or biscuit eye.
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    let gold = (255.0_f32 * strength) as u8;
    let g = (180.0_f32 * strength) as u8;
    let style = Style::default()
        .fg(Color::Rgb(gold, g, 40))
        .bg(Color::Rgb(30, 20, 0))
        .add_modifier(Modifier::BOLD);
    let dim = (180.0_f32 * strength) as u8;
    let body_style = Style::default()
        .fg(Color::Rgb(255, 230, dim.max(80)))
        .bg(Color::Rgb(30, 20, 0))
        .add_modifier(Modifier::BOLD);
    let block = Block::bordered()
        .border_style(style)
        .style(Style::default().bg(Color::Rgb(30, 20, 0)));
    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(header_plain.to_string(), style)),
        Line::from(Span::styled(body, body_style)),
        Line::raw(""),
    ];
    let p = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .block(block);
    frame.render_widget(p, rect);
}

fn header_plain() -> &'static str {
    " *** ACHIEVEMENT UNLOCKED *** "
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `1.0` for the bulk of the toast; smooth ramps at the entry and exit so
/// it feels like a popup, not a hard cut. `life` is remaining/total.
fn ease_in_out(life: f32) -> f32 {
    let life = life.clamp(0.0, 1.0);
    // Ramp up over the first 15%, hold, ramp down over the last 25%.
    let entry = smoothstep((1.0 - life) / 0.15);
    let exit = smoothstep(life / 0.25);
    entry.min(exit)
}
