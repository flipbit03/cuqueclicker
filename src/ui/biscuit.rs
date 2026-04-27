use ratatui::{prelude::*, widgets::*};

use crate::game::golden::{GOLDEN_LIFE_TICKS, GoldenCuque, GoldenVariant};
use crate::game::state::{Buff, CLENCH_SQUASH_TICKS, CLENCH_TICKS, GameState};

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

/// Draw the biscuit. Reads:
///
/// - `state.clench_ticks` — counts down a click flash. While >0, the eye
///   becomes `*` and the body shifts pink. The first `CLENCH_SQUASH_TICKS`
///   of that countdown also drop the top blank row, giving a one-frame
///   vertical squash before the spring back.
/// - active `ClickFrenzy` buff — biscuit is tinted toward red and shakes
///   ±1 col on clench frames. Pure visual chaos, no behavior change.
/// - `state.session_ticks` — drives a slow ambient breathing color cycle
///   so the biscuit isn't completely static at idle.
pub fn draw(frame: &mut Frame, area: Rect, state: &GameState, zoom_idx: usize) -> Rect {
    let art = BISCUIT_LEVELS[zoom_idx.min(BISCUIT_LEVELS.len() - 1)];
    let clenched = state.clench_ticks > 0;
    // First CLENCH_SQUASH_TICKS frames of the clench: render a vertically
    // squashed variant of the art so the cuque visibly contracts, then
    // springs back. clench_ticks counts down from CLENCH_TICKS so "early in
    // the clench" means clench_ticks is large.
    let squash = clenched && state.clench_ticks + CLENCH_SQUASH_TICKS > CLENCH_TICKS;

    // CRITICAL: the squash transformation MUST preserve total row count.
    // `hands::draw` reads `biscuit.height` and `biscuit.y` to compute the
    // orbital center + radii — if either changes per-frame, every hand
    // around the cuque jitters on each click. The squash is built by
    // dropping the rows immediately above + below the eye and padding with
    // a blank row at top and bottom. Net: same height, eye stays at the
    // same screen y, outer outline contracts inward toward the eye, hands
    // around the biscuit don't move.
    let render_art_owned: Vec<String> = if squash {
        squashed_art(art)
    } else {
        art.iter().map(|s| s.to_string()).collect()
    };
    let render_art: Vec<&str> = render_art_owned.iter().map(|s| s.as_str()).collect();

    let w = render_art
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let h = render_art.len() as u16;
    // Anchor placement to the EYE column rather than the art's bounding box.
    // Each zoom level has a different art width and a different in-art eye
    // column, so centering by `(area.width - w) / 2` (integer truncation)
    // makes the eye drift left/right across zoom changes. Anchoring the eye
    // to a fixed screen column instead keeps the asshole stationary on every
    // zoom level — the surrounding art shifts; the focus point doesn't.
    let target_eye_col = area.x + area.width / 2;
    let eye_col_in_art = render_art
        .iter()
        .find_map(|s| s.chars().position(|c| c == 'O' || c == '*'))
        .unwrap_or(w as usize / 2) as u16;
    let x_base = target_eye_col
        .saturating_sub(eye_col_in_art)
        .max(area.x)
        .min((area.x + area.width).saturating_sub(w));
    let y_base = area.y + area.height.saturating_sub(h) / 2;

    // The stable rect is what we RETURN to callers (hands, particles,
    // golden). It must NOT depend on per-frame transients like the Frenzy
    // shake — otherwise the orbital hands and floating particles jitter on
    // every clench. Frenzy shake is applied only to the render position
    // below.
    let stable_rect = Rect {
        x: x_base,
        y: y_base,
        width: w.min(area.width),
        height: h.min(area.height.saturating_sub(y_base - area.y)),
    };

    // Frenzy shake: ±1 col jitter while clenched and frenzied. Drives off
    // session_ticks so successive frames pick different offsets without
    // needing per-render RNG state.
    let frenzy_active = state
        .buffs
        .iter()
        .any(|b| matches!(b, Buff::ClickFrenzy { .. }));
    let shake = if frenzy_active && clenched {
        (state.session_ticks % 3) as i32 - 1
    } else {
        0
    };
    let render_x = ((x_base as i32 + shake)
        .max(area.x as i32)
        .min((area.x + area.width).saturating_sub(stable_rect.width) as i32))
        as u16;
    let render_rect = Rect {
        x: render_x,
        y: stable_rect.y,
        width: stable_rect.width,
        height: stable_rect.height,
    };

    let lines: Vec<Line> = render_art
        .iter()
        .map(|s| {
            if clenched {
                Line::from(s.replace('O', "*"))
            } else {
                Line::from(s.to_string())
            }
        })
        .collect();

    // Color blend:
    //   - resting tan (220, 170, 150) when calm.
    //   - clenched pink (255, 120, 140); during Frenzy bias the pink redder.
    //   - idle: slow ±~5% sinusoidal breath on brightness, so the biscuit
    //     never freezes between events.
    let base = if clenched {
        if frenzy_active {
            (255.0_f32, 80.0, 110.0)
        } else {
            (255.0_f32, 120.0, 140.0)
        }
    } else {
        let t = (state.session_ticks as f32) / 25.0; // ~8s period at 20Hz
        let breath = 1.0 + 0.05 * t.sin();
        let r = 220.0 * breath;
        let g = 170.0 * breath;
        let b = 150.0 * breath;
        (
            r.clamp(0.0, 255.0),
            g.clamp(0.0, 255.0),
            b.clamp(0.0, 255.0),
        )
    };

    let color = Color::Rgb(base.0 as u8, base.1 as u8, base.2 as u8);
    let p = Paragraph::new(lines).style(Style::default().fg(color));
    frame.render_widget(p, render_rect);
    // Return the STABLE rect so hands / particles / golden see a steady
    // biscuit position even when render_rect was shifted by the Frenzy
    // shake or vertically squeezed by the squash padding.
    stable_rect
}

/// Render the golden cuque marker. Position is resolved against the CURRENT
/// `biscuit` rect every frame from the golden's stored fractional anchor —
/// so the marker travels with the biscuit on zoom and resize, instead of
/// stranding in the old screen position. Returned `Rect` is the actual
/// drawn rect, used by the click router for hit-testing.
///
/// Build the "squashed" frame of a biscuit ASCII level by removing the rows
/// immediately above and below the eye row (the one containing 'O') and
/// padding with a blank row at top and bottom.
///
/// Why this shape: a real squash needs the centerline (eye) to stay anchored
/// while the upper and lower halves contract toward it — that's what reads
/// as a flattened ellipsoid. Just shrinking from the top makes the cuque
/// look like the topmost row is flickering, not pulsing.
///
/// Why the blank padding: total row count MUST be preserved. The biscuit
/// rect that this function feeds is read by `hands::draw` to place the
/// orbital fingerers — any change to rect.height (or rect.y, via
/// recentering) would shift every hand around the cuque on every click.
/// Padding keeps the rect identical between calm and squashed states.
///
/// Falls back to a plain copy if the art is too short to safely drop two
/// rows or doesn't contain an eye 'O' — neither case exists in the
/// shipped catalog but defensive against future zoom levels.
fn squashed_art(art: &[&str]) -> Vec<String> {
    let n = art.len();
    if n < 5 {
        return art.iter().map(|s| s.to_string()).collect();
    }
    let Some(eye_row) = art.iter().position(|s| s.contains('O')) else {
        return art.iter().map(|s| s.to_string()).collect();
    };
    if eye_row == 0 || eye_row + 1 >= n {
        return art.iter().map(|s| s.to_string()).collect();
    }
    // Build a blank line the same width as the widest art row so the rect
    // dimensions don't shift.
    let width = art.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let blank: String = " ".repeat(width);

    let mut out: Vec<String> = Vec::with_capacity(n);
    // Top blank pad — replaces the row we'd otherwise lose by dropping
    // (eye_row - 1).
    out.push(blank.clone());
    // Original rows 0..=eye_row-2 (skipping eye_row-1).
    for s in art.iter().take(eye_row - 1) {
        out.push((*s).to_string());
    }
    // The eye row itself.
    out.push(art[eye_row].to_string());
    // Original rows eye_row+2..n (skipping eye_row+1).
    for s in art.iter().skip(eye_row + 2) {
        out.push((*s).to_string());
    }
    // Bottom blank pad to match the top pad.
    out.push(blank);
    debug_assert_eq!(out.len(), n);
    out
}

/// J9 juice: the marker shimmers. Each character of the 5-wide marker
/// samples its own foreground color from a horizontally-traveling wave
/// between a `bright` peak, a `dim` trough, and an `accent` highlight on
/// the off-phase. The bg stays a constant low-key tint, so what the player
/// sees is the TEXT itself sliding through colors — not a flashing box.
/// In the final 20% of life the wave speeds up and the trough darkens so
/// a soon-to-expire golden visibly accelerates without losing legibility.
pub fn draw_golden(frame: &mut Frame, golden: &GoldenCuque, biscuit: Rect) -> Rect {
    let buf = frame.buffer_mut();
    // `bright` and `dim` define the gradient endpoints the wave swings
    // between. `accent` is a third color woven in on the off-phase so the
    // shimmer reads chromatic, not just bright/dark.
    let (center, bright, dim, accent, bg) = match golden.variant {
        GoldenVariant::Lucky => (
            '$',
            (255.0_f32, 230.0, 80.0),
            (140.0_f32, 90.0, 0.0),
            (255.0_f32, 170.0, 30.0),
            Color::Rgb(40, 25, 0),
        ),
        GoldenVariant::Frenzy => (
            '!',
            (255.0, 110.0, 110.0),
            (120.0, 0.0, 0.0),
            (255.0, 200.0, 60.0),
            Color::Rgb(50, 0, 0),
        ),
        GoldenVariant::Buff => (
            '+',
            (230.0, 160.0, 255.0),
            (80.0, 20.0, 110.0),
            (140.0, 220.0, 255.0),
            Color::Rgb(35, 0, 45),
        ),
    };

    // Wave speed (rad/tick) and trough depth both bump in alarm mode.
    let life_frac = (golden.life_ticks as f32 / GOLDEN_LIFE_TICKS as f32).clamp(0.0, 1.0);
    let alarm = life_frac < 0.20;
    let speed = if alarm { 0.55 } else { 0.22 };
    let dim_pull = if alarm { 1.0 } else { 0.6 };
    // Phase advances every tick; per-cell offset shifts the wave across the
    // 5-cell width so neighbors land at different points of the gradient.
    let phase = (GOLDEN_LIFE_TICKS - golden.life_ticks) as f32 * speed;
    let cell_offset = std::f32::consts::TAU / 5.0; // one full cycle across width

    let lines: [String; 3] = [
        ".---.".to_string(),
        format!("( {} )", center),
        "`---'".to_string(),
    ];
    let w: u16 = 5;
    let h: u16 = 3;

    let area = buf.area;
    if area.width == 0 || area.height == 0 || biscuit.width < w || biscuit.height < h {
        return Rect::default();
    }

    let (anchor_col, anchor_row) =
        crate::game::state::biscuit_frac_to_screen(golden.frac_x, golden.frac_y, biscuit);
    let mut col = anchor_col;
    let mut row = anchor_row;
    // Keep the 5x3 marker fully inside the biscuit so it never overlaps the
    // sidebar / HUD chrome, then clamp once more to the screen for safety.
    if col + w > biscuit.x + biscuit.width {
        col = (biscuit.x + biscuit.width).saturating_sub(w);
    }
    if row + h > biscuit.y + biscuit.height {
        row = (biscuit.y + biscuit.height).saturating_sub(h);
    }
    if col < biscuit.x {
        col = biscuit.x;
    }
    if row < biscuit.y {
        row = biscuit.y;
    }
    if col + w > area.x + area.width {
        col = (area.x + area.width).saturating_sub(w);
    }
    if row + h > area.y + area.height {
        row = (area.y + area.height).saturating_sub(h);
    }

    // Per-character horizontal gradient: walk every cell, sample the wave
    // for that cell's column offset, and write a 1-char styled span. Cheap
    // (15 cells max) and gives "shimmering text" instead of "blinking box".
    for (dy, line) in lines.iter().enumerate() {
        let y = row + dy as u16;
        if y >= area.y + area.height {
            break;
        }
        for (i, ch) in line.chars().enumerate() {
            let x = col + i as u16;
            if x >= area.x + area.width {
                break;
            }
            let arg = phase + i as f32 * cell_offset;
            let wave_main = (arg.sin() + 1.0) * 0.5; // 0..1
            // Accent rides on a quarter-phase shift so it brightens in
            // between the main bright peaks rather than reinforcing them.
            let wave_accent = ((arg + std::f32::consts::FRAC_PI_2).sin() + 1.0) * 0.5;
            // Pull the trough darker by `dim_pull` so alarm mode visibly
            // crushes the dim end without affecting peak readability.
            let dim_dim = (
                dim.0 * (1.0 - 0.4 * dim_pull),
                dim.1 * (1.0 - 0.4 * dim_pull),
                dim.2 * (1.0 - 0.4 * dim_pull),
            );
            let main_r = dim_dim.0 + (bright.0 - dim_dim.0) * wave_main;
            let main_g = dim_dim.1 + (bright.1 - dim_dim.1) * wave_main;
            let main_b = dim_dim.2 + (bright.2 - dim_dim.2) * wave_main;
            // Cap accent contribution at 35% so it tints without washing
            // out the bright peak.
            let accent_w = wave_accent * 0.35;
            let r = main_r + (accent.0 - main_r) * accent_w;
            let g = main_g + (accent.1 - main_g) * accent_w;
            let b = main_b + (accent.2 - main_b) * accent_w;
            let style = Style::default()
                .fg(Color::Rgb(
                    r.clamp(0.0, 255.0) as u8,
                    g.clamp(0.0, 255.0) as u8,
                    b.clamp(0.0, 255.0) as u8,
                ))
                .bg(bg)
                .add_modifier(Modifier::BOLD);
            buf.set_string(x, y, ch.to_string(), style);
        }
    }

    Rect {
        x: col,
        y: row,
        width: w,
        height: h,
    }
}
