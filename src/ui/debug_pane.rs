use ratatui::{prelude::*, widgets::*};

/// Keys advertised in the debug pane. One source of truth for both the
/// rendered rows and the hotkey dispatch in `input.rs`.
///
/// `F8` is the Lucky-spawn cheat instead of `F1` because Chrome (and most
/// browsers) hijack `F1` for "Help" and never deliver the keydown to the
/// page. `F8` has no default browser binding outside of an open DevTools
/// instance, so it's safe on both web and native.
pub const DEBUG_KEYS: &[(&str, &str)] = &[
    ("F8", "Lucky  [$]"),
    ("F2", "Frenzy [!]"),
    ("F3", "Buff   [+]"),
    ("F4", "+1M cuques"),
];

pub fn draw(frame: &mut Frame, play_area: Rect) {
    if play_area.width < 24 || play_area.height < (DEBUG_KEYS.len() as u16 + 3) {
        return;
    }

    let w: u16 = 22;
    let h: u16 = DEBUG_KEYS.len() as u16 + 3; // border × 2 + hint line

    let area = Rect {
        x: play_area.x,
        y: play_area.y,
        width: w,
        height: h,
    };

    let mut lines: Vec<Line> = DEBUG_KEYS
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<3}", key),
                    Style::default()
                        .fg(Color::Rgb(255, 200, 90))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(*desc, Style::default().fg(Color::Rgb(200, 200, 210))),
            ])
        })
        .collect();

    lines.push(Line::from(Span::styled(
        "--no-debug: disable",
        Style::default().fg(Color::Rgb(110, 110, 120)),
    )));

    let block = Block::bordered()
        .title(" DEBUG ")
        .border_style(Style::default().fg(Color::Rgb(180, 140, 60)));
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(Clear, area);
    frame.render_widget(p, area);
}
