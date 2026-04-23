use ratatui::prelude::*;

use crate::game::state::Particle;

const PARTICLE_LIFE_F: f32 = 20.0;

pub fn draw_particles(frame: &mut Frame, area: Rect, particles: &[Particle]) {
    let buf = frame.buffer_mut();
    for p in particles {
        let r = p.row.round();
        if r < area.y as f32 || r >= (area.y + area.height) as f32 {
            continue;
        }
        let row = r as u16;
        if p.col < area.x || p.col >= area.x + area.width {
            continue;
        }
        let t = (p.life as f32 / PARTICLE_LIFE_F).clamp(0.0, 1.0);
        let dim = (t * 255.0) as u8;
        let style = Style::default()
            .fg(Color::Rgb(255, dim, dim))
            .add_modifier(Modifier::BOLD);
        buf.set_string(p.col, row, &p.text, style);
    }
}
