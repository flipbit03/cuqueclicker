use ratatui::prelude::*;

use crate::game::state::{Particle, biscuit_frac_to_screen};

const PARTICLE_LIFE_F: f32 = 20.0;

/// Render auto/click particles. Positions are resolved against the CURRENT
/// `biscuit` rect from each particle's fractional anchor, so they travel
/// with the biscuit on zoom/resize. `area` (the biscuit rect, same as the
/// resolution target) is the visual clip.
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
        let dim = (t * 255.0) as u8;
        let style = Style::default()
            .fg(Color::Rgb(255, dim, dim))
            .add_modifier(Modifier::BOLD);
        buf.set_string(col, row, &p.text, style);
    }
}
