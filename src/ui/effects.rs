use ratatui::prelude::*;

use crate::game::state::{MisclickParticle, Particle, ParticleKind, biscuit_frac_to_screen};

const PARTICLE_LIFE_F: f32 = 20.0;
const MISCLICK_LIFE_F: f32 = 8.0;

/// Render auto/click/golden/confetti particles. Positions are resolved
/// against the CURRENT `biscuit` rect from each particle's fractional
/// anchor, so they travel with the biscuit on zoom/resize. `area` is the
/// visual clip (same as the resolution target).
pub fn draw_particles(frame: &mut Frame, biscuit: Rect, particles: &[Particle]) {
    if biscuit.width == 0 || biscuit.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    for p in particles {
        let (col, row) = biscuit_frac_to_screen(p.frac_x, p.frac_y, biscuit);
        if row < biscuit.y || row >= biscuit.y + biscuit.height {
            continue;
        }
        if col < biscuit.x || col >= biscuit.x + biscuit.width {
            continue;
        }
        let t = (p.life as f32 / PARTICLE_LIFE_F).clamp(0.0, 1.0);
        let style = particle_style(p.kind, t);
        buf.set_string(col, row, &p.text, style);
    }
}

/// Style picker per kind. `t` ∈ `[0,1]` is "remaining life fraction" so
/// 1.0 = freshly spawned, 0.0 = about to despawn. All kinds fade out as t
/// drops; the difference is base color and BOLD weight.
fn particle_style(kind: ParticleKind, t: f32) -> Style {
    let dim = (t * 255.0) as u8;
    match kind {
        // Default click / auto: white that fades to red — same as before.
        ParticleKind::Click | ParticleKind::Auto => Style::default()
            .fg(Color::Rgb(255, dim, dim))
            .add_modifier(Modifier::BOLD),
        // High-power click (Frenzy, big mults): warm yellow → red, BOLD,
        // so a +777 reads distinctly from a stream of +1s.
        ParticleKind::ClickBig => Style::default()
            .fg(Color::Rgb(255, 255 - (255 - dim) / 2, dim))
            .add_modifier(Modifier::BOLD),
        // Golden catch: gold for Lucky-style labels, fades to white.
        ParticleKind::Golden => Style::default()
            .fg(Color::Rgb(255, 230, (dim / 2).max(80)))
            .add_modifier(Modifier::BOLD),
        // Confetti glyphs: cyan/magenta-ish so they read as celebratory
        // detail rather than another `+N` floating up.
        ParticleKind::Confetti => Style::default()
            .fg(Color::Rgb(((1.0 - t) * 200.0) as u8 + 55, dim, 255))
            .add_modifier(Modifier::BOLD),
    }
}

/// Render screen-anchored "misclick" tap particles. These ignore the biscuit
/// rect — they live at the exact column/row the click missed at, so dead-zone
/// clicks visibly *register* even if no game state changed.
pub fn draw_misclicks(frame: &mut Frame, particles: &[MisclickParticle]) {
    let buf = frame.buffer_mut();
    let area = buf.area;
    for m in particles {
        if m.col < area.x
            || m.col >= area.x + area.width
            || m.row < area.y
            || m.row >= area.y + area.height
        {
            continue;
        }
        let t = (m.life as f32 / MISCLICK_LIFE_F).clamp(0.0, 1.0);
        let v = (t * 180.0) as u8 + 60;
        let style = Style::default().fg(Color::Rgb(v, v, v));
        buf.set_string(m.col, m.row, "·", style);
    }
}
