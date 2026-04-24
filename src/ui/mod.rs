pub mod achievements;
pub mod biscuit;
pub mod border;
pub mod debug_pane;
pub mod effects;
pub mod hands;
pub mod prestige;
pub mod sidebar;
pub mod stats;
pub mod upgrades;

use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::{Buff, GameState, TICK_HZ};
use crate::i18n::t;

// Hardcoded as "0.0.0" in source; release.yml patches Cargo.toml before
// building so CARGO_PKG_VERSION reflects the real version in shipped
// binaries. A 0.0.0 build advertises itself as "(dev)" in the HUD.
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn hud_title() -> String {
    if VERSION == "0.0.0" {
        // Dev builds include the git branch (or short SHA on detached HEAD)
        // so two instances built from different branches can be told apart
        // at a glance — useful for side-by-side comparison.
        match crate::build_info::GIT_BRANCH {
            Some(branch) => format!(" CuqueClicker v0.0.0 (dev, {branch}) "),
            None => " CuqueClicker v0.0.0 (dev) ".into(),
        }
    } else {
        format!(" CuqueClicker v{VERSION} ")
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Mode {
    Game,
    Stats,
    Achievements,
    Upgrades,
    Prestige,
}

pub struct DrawOutput {
    pub biscuit_rect: Rect,
    pub golden_rect: Rect,
    pub visible_upgrades: Vec<usize>,
    pub visible_fingerers: Vec<usize>,
}

fn wrapped_height(text: &str, width: u16) -> u16 {
    if width == 0 {
        return text.lines().count().max(1) as u16;
    }
    let mut total: u16 = 0;
    for line in text.split('\n') {
        let mut row_len: u16 = 0;
        let mut rows: u16 = 1;
        for word in line.split_whitespace() {
            let wlen = word.chars().count() as u16;
            if row_len == 0 {
                row_len = wlen.min(width);
            } else if row_len + 1 + wlen <= width {
                row_len += 1 + wlen;
            } else {
                rows += 1;
                row_len = wlen.min(width);
            }
        }
        total = total.saturating_add(rows);
    }
    total.max(1)
}

fn draw_zoom_indicator(frame: &mut Frame, area: Rect, label: &str) {
    let text = format!("[-/+] zoom {}", label);
    let w = text.chars().count() as u16;
    if area.width < w || area.height == 0 {
        return;
    }
    let col = area.x + area.width - w;
    let row = area.y + area.height - 1;
    let buf = frame.buffer_mut();
    buf.set_string(
        col,
        row,
        &text,
        Style::default().fg(Color::Rgb(120, 120, 120)),
    );
}

pub fn draw(
    frame: &mut Frame,
    state: &GameState,
    mode: Mode,
    zoom_idx: usize,
    debug: bool,
) -> DrawOutput {
    let lang = t();
    let area = frame.area();
    let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(38)]).split(area);

    let help_text = match mode {
        Mode::Game => lang.help_game,
        Mode::Stats => lang.help_stats,
        Mode::Achievements => lang.help_ach,
        Mode::Upgrades => lang.help_upgrades,
        Mode::Prestige => lang.help_prestige,
    };
    let help_height = wrapped_height(help_text, cols[0].width).max(1);
    let left = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(help_height),
    ])
    .split(cols[0]);

    let mut hud_spans: Vec<Span> = vec![Span::raw(format!(
        "{}: {}   {}: {}",
        lang.hud_cuques,
        format::big(state.cuques),
        lang.hud_fps,
        format::rate(state.fps())
    ))];
    if state.prestige > 0 {
        hud_spans.push(Span::styled(
            format!(
                "   {}: {} (+{:.0}%)",
                lang.prestige_title.trim(),
                state.prestige,
                state.prestige as f64
            ),
            Style::default()
                .fg(Color::Rgb(255, 215, 0))
                .add_modifier(Modifier::BOLD),
        ));
    }
    for b in &state.buffs {
        let secs = b.ticks_remaining().div_ceil(TICK_HZ);
        let (label, color) = match b {
            Buff::ClickFrenzy { mult, .. } => (
                format!("  [!! FRENZY x{} {}s]", *mult as u64, secs),
                Color::Rgb(255, 80, 80),
            ),
            Buff::FingererBoost {
                fingerer_id, mult, ..
            } => {
                let idx = crate::game::fingerer::FINGERERS
                    .iter()
                    .position(|f| f.id == fingerer_id);
                let name = idx
                    .and_then(|i| lang.fingerer_names.get(i).copied())
                    .unwrap_or("?");
                (
                    format!("  [++ {} x{} {}s]", name, *mult as u64, secs),
                    Color::Rgb(220, 140, 255),
                )
            }
        };
        hud_spans.push(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    let title = hud_title();
    border::draw_animated(frame, left[0], state, &title);
    let hud_inner = Rect {
        x: left[0].x + 1,
        y: left[0].y + 1,
        width: left[0].width.saturating_sub(2),
        height: left[0].height.saturating_sub(2),
    };
    let hud = Paragraph::new(Line::from(hud_spans));
    frame.render_widget(hud, hud_inner);

    let biscuit_rect = biscuit::draw(frame, left[1], state.clench_ticks > 0, zoom_idx);
    hands::draw(frame, left[1], biscuit_rect, state);
    effects::draw_particles(frame, left[1], &state.particles);
    draw_zoom_indicator(
        frame,
        left[1],
        biscuit::level_label(zoom_idx).unwrap_or("100%"),
    );

    if debug {
        debug_pane::draw(frame, left[1]);
    }
    let golden_rect = match &state.golden {
        Some(g) => biscuit::draw_golden(frame, g),
        None => Rect::default(),
    };

    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray))
        .wrap(Wrap { trim: false });
    frame.render_widget(help, left[2]);

    let mut visible_upgrades = Vec::new();
    let mut visible_fingerers = Vec::new();
    match mode {
        Mode::Game => visible_fingerers = sidebar::draw(frame, cols[1], state),
        Mode::Stats => stats::draw(frame, cols[1], state),
        Mode::Achievements => achievements::draw(frame, cols[1], state),
        Mode::Upgrades => visible_upgrades = upgrades::draw(frame, cols[1], state),
        Mode::Prestige => prestige::draw(frame, cols[1], state),
    }

    DrawOutput {
        biscuit_rect,
        golden_rect,
        visible_upgrades,
        visible_fingerers,
    }
}
