use ratatui::{prelude::*, widgets::*};

use crate::game::golden::{GoldenCuque, GoldenVariant};

const BISCUIT_FULL: &[&str] = &[
    r"                    ____________________                    ",
    r"              __,-~~                    ~~-,__              ",
    r"           ,-~'                                `~-,         ",
    r"        ,-'                                        `-,      ",
    r"      ,'                                              `.    ",
    r"     /         -~-~-~-              -~-~-~-             \   ",
    r"    /                                                    \  ",
    r"   /             -~~-~-~~-                                \ ",
    r"  /                                                        \",
    r" |          -~-~-~-~-             -~-~-~-~-                |",
    r" |                                                         |",
    r" |                                                         |",
    r" |                  \\\\\\\\   |   ////////                |",
    r" |                   \\\\\\\\  |  ////////                 |",
    r" |                    \\\\\\\\\|/////////                  |",
    r" |         ~ - - - - -         O         - - - - - ~       |",
    r" |                    /////////|\\\\\\\\\                  |",
    r" |                   ////////  |  \\\\\\\\                 |",
    r" |                  ////////   |   \\\\\\\\                |",
    r" |                                                         |",
    r" |          -~-~-~-~-             -~-~-~-~-                |",
    r"  \                                                        /",
    r"   \             -~~-~-~~-                                / ",
    r"    \                                                    /  ",
    r"     \         -~-~-~-              -~-~-~-             /   ",
    r"      `.                                              ,'    ",
    r"        `-,                                        ,-'      ",
    r"           `~-,                                ,-~'         ",
    r"              `~-,,_                      _,,-~'            ",
    r"                   `~-,,______________,,-~'                 ",
];

const BISCUIT_MEDIUM: &[&str] = &[
    r"            ________________            ",
    r"         ,-~                 ~-,        ",
    r"      ,-'                        `-,    ",
    r"    ,'                              `.  ",
    r"   /         -~-~-      -~-~-         \ ",
    r"  /                                    \",
    r" |          \\\\\   |   /////           |",
    r" |           \\\\\  |  /////            |",
    r" |            \\\\\\|//////             |",
    r" |    ~ - - -       O       - - - ~     |",
    r" |            //////|\\\\\\             |",
    r" |           /////  |  \\\\\            |",
    r" |          /////   |   \\\\\           |",
    r"  \                                    /",
    r"   \         -~-~-      -~-~-         / ",
    r"    `.                              ,'  ",
    r"      `-,                        ,-'    ",
    r"         `~-,,_______________,,-~'      ",
];

const BISCUIT_SMALL: &[&str] = &[
    r"        __________        ",
    r"     ,-~          ~-,     ",
    r"   ,'                `.   ",
    r"  /    -~-~-  -~-~-    \  ",
    r" |       \\\ | ///       | ",
    r" |        \\\|///        | ",
    r" | ~ - -     O     - - ~ | ",
    r" |        ///|\\\        | ",
    r" |       /// | \\\       | ",
    r"  \    -~-~-  -~-~-    /  ",
    r"   `.                ,'   ",
    r"     `-,,________,,-'     ",
];

const BISCUIT_TINY: &[&str] = &[
    r"     ______     ",
    r"   ,~      ~,   ",
    r"  /          \  ",
    r" |    \|/     | ",
    r" | -   O   -  | ",
    r" |    /|\     | ",
    r"  \          /  ",
    r"   `-,____,-'   ",
];

const BISCUIT_LEVELS: &[&[&str]] = &[BISCUIT_FULL, BISCUIT_MEDIUM, BISCUIT_SMALL, BISCUIT_TINY];

pub fn level_count() -> usize {
    BISCUIT_LEVELS.len()
}

pub fn level_label(idx: usize) -> Option<&'static str> {
    match idx {
        0 => None,
        1 => Some("70%"),
        2 => Some("45%"),
        3 => Some("25%"),
        _ => None,
    }
}

pub fn draw(frame: &mut Frame, area: Rect, clenched: bool, zoom_idx: usize) -> Rect {
    let art = BISCUIT_LEVELS[zoom_idx.min(BISCUIT_LEVELS.len() - 1)];
    let w = art.iter().map(|s| s.chars().count()).max().unwrap_or(0) as u16;
    let h = art.len() as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect {
        x,
        y,
        width: w.min(area.width),
        height: h.min(area.height),
    };

    let lines: Vec<Line> = art
        .iter()
        .map(|s| {
            if clenched {
                Line::from(s.replace('O', "*"))
            } else {
                Line::from(s.to_string())
            }
        })
        .collect();

    let color = if clenched {
        Color::Rgb(255, 120, 140)
    } else {
        Color::Rgb(220, 170, 150)
    };
    let p = Paragraph::new(lines).style(Style::default().fg(color));
    frame.render_widget(p, rect);
    rect
}

pub fn draw_golden(frame: &mut Frame, golden: &GoldenCuque) -> Rect {
    let buf = frame.buffer_mut();
    let (center, style) = match golden.variant {
        GoldenVariant::Lucky => (
            '$',
            Style::default()
                .fg(Color::Rgb(255, 215, 0))
                .bg(Color::Rgb(60, 40, 0))
                .add_modifier(Modifier::BOLD),
        ),
        GoldenVariant::Frenzy => (
            '!',
            Style::default()
                .fg(Color::Rgb(255, 80, 80))
                .bg(Color::Rgb(80, 0, 0))
                .add_modifier(Modifier::BOLD),
        ),
        GoldenVariant::Buff => (
            '+',
            Style::default()
                .fg(Color::Rgb(220, 140, 255))
                .bg(Color::Rgb(50, 0, 60))
                .add_modifier(Modifier::BOLD),
        ),
    };

    let lines: [String; 3] = [
        ".---.".to_string(),
        format!("( {} )", center),
        "`---'".to_string(),
    ];
    let w: u16 = 5;
    let h: u16 = 3;

    let area = buf.area;
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }

    let mut col = golden.col;
    let mut row = golden.row;
    if col + w > area.x + area.width {
        col = (area.x + area.width).saturating_sub(w);
    }
    if row + h > area.y + area.height {
        row = (area.y + area.height).saturating_sub(h);
    }
    if col < area.x {
        col = area.x;
    }
    if row < area.y {
        row = area.y;
    }

    for (dy, line) in lines.iter().enumerate() {
        let y = row + dy as u16;
        if y >= area.y + area.height {
            break;
        }
        buf.set_string(col, y, line.as_str(), style);
    }

    Rect {
        x: col,
        y: row,
        width: w,
        height: h,
    }
}
