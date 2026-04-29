//! Achievement-unlock toast overlay.
//!
//! Rendered *over* the biscuit area so it's prominent without disturbing
//! the right-side panel. The sim populates `state.active_unlock_id` /
//! `active_unlock_ticks` from the queue in `state.newly_unlocked`; this
//! module just translates that into a brief gold-bordered popup.
//!
//! Lives ~4s on screen (TOAST_TICKS). The whole box (bg, border, header,
//! body) shares a single `strength` curve so it fades in and out as one
//! object — never the empty box first or the text alone after.
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

    // Single ease curve drives EVERYTHING: bg darkness, border color, header
    // color, body color. They all rise and fall together so the toast reads
    // as one fading object.
    let life = state.active_unlock_ticks as f32 / TOAST_TICKS as f32;
    let strength = ease_in_out(life);
    if strength <= 0.01 {
        return;
    }

    let header_plain = header_plain();
    let body_text = format!("  {name}  ");
    let inner_w = (header_plain.chars().count().max(body_text.chars().count()) + 2) as u16;
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

    // Solid bg fill is the foundation: paint every cell in the rect with the
    // toast bg before anything else, so the box is opaque (no biscuit /
    // particles bleeding through the blank rows). Without this, ratatui's
    // `Block::bordered().style(bg)` only colors the border cells; the
    // interior keeps whatever the underlying widget drew, which is why the
    // earlier toast looked hollow.
    let bg = Color::Rgb((30.0 * strength) as u8, (20.0 * strength) as u8, 0);
    let bg_style = Style::default().bg(bg);
    {
        let buf = frame.buffer_mut();
        for dy in 0..rect.height {
            for dx in 0..rect.width {
                let cx = rect.x + dx;
                let cy = rect.y + dy;
                if cx >= buf.area.x + buf.area.width || cy >= buf.area.y + buf.area.height {
                    continue;
                }
                let cell = &mut buf[(cx, cy)];
                cell.set_char(' ');
                cell.set_style(bg_style);
            }
        }
    }

    // Border + text now ride the same `strength`. We fade the foreground
    // colors via the same multiplier as the bg so the whole popup feels
    // like one object.
    let border_fg = Color::Rgb(
        (255.0 * strength) as u8,
        (180.0 * strength) as u8,
        (40.0 * strength) as u8,
    );
    let header_fg = border_fg;
    let body_fg = Color::Rgb(
        (255.0 * strength) as u8,
        (230.0 * strength) as u8,
        ((180.0 * strength) as u8).max((60.0 * strength) as u8),
    );
    let header_style = Style::default()
        .fg(header_fg)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let body_style = Style::default()
        .fg(body_fg)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let block = Block::bordered()
        .border_style(Style::default().fg(border_fg).bg(bg))
        .style(bg_style);
    let lines = vec![
        // The leading/trailing blanks are styled with the bg so they paint
        // their cells (instead of leaving the underlying buffer through).
        Line::from(Span::styled(" ".repeat(w as usize), bg_style)),
        Line::from(Span::styled(header_plain.to_string(), header_style)),
        Line::from(Span::styled(body_text, body_style)),
        Line::from(Span::styled(" ".repeat(w as usize), bg_style)),
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
///
/// `life` decays from 1.0 → 0.0 over the toast's lifetime. The entry ramp
/// is shorter than the exit ramp so the popup feels snappy on arrival and
/// gentle on departure.
fn ease_in_out(life: f32) -> f32 {
    let life = life.clamp(0.0, 1.0);
    // Ramp up over the first 15% (entry), hold at full, ramp down over the
    // last 25% (exit). Both bg and fg use this single curve.
    let entry = smoothstep((1.0 - life) / 0.15);
    let exit = smoothstep(life / 0.25);
    entry.min(exit)
}
