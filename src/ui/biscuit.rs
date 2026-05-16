use ratatui::{prelude::*, widgets::*};

use crate::game::powerup::{Powerup, PowerupKind};
use crate::game::state::{Buff, CLENCH_SQUASH_TICKS, CLENCH_TICKS, GameState};

/// Asshole-spin animation frames. Cycled by `total_clicks % N` while a
/// spacebar hold has been detected (≥1s of continuous repeat). `*` lands
/// every 5th frame as a "flash" sparkle in the rotation so the cycle reads
/// as an actually-spinning asshole occasionally bursting into a star,
/// rather than four indistinguishable line strokes.
const SPIN_FRAMES: [char; 5] = ['\\', '|', '/', '-', '*'];

/// Resolve a prestige count to (tint_color, mix_strength) for the cuque
/// body. The tint is a noble hue the body bleeds toward; the mix is how
/// strongly the tint dominates the resting tan. Steps are coarse on
/// purpose — a player needs to PRESTIGE several times in a tier to start
/// glimpsing the next one, so a tier transition reads as earned rather
/// than continuous drift. Caps at "divine white" past tier 5.
fn prestige_body_tint(prestige: u64) -> ((f32, f32, f32), f32) {
    // Pure tan when at 0; each tier biases the cuque toward a distinct
    // hue. Mix grows with tier so endgame is dramatic, but capped < 0.7
    // so the body never goes monochrome — you can still tell it's a cuque.
    match prestige {
        0 => ((220.0, 170.0, 150.0), 0.0),
        1..=2 => ((255.0, 200.0, 110.0), 0.18),   // warm gold
        3..=5 => ((255.0, 215.0, 80.0), 0.32),    // saturated gold
        6..=9 => ((230.0, 220.0, 235.0), 0.40),   // silver-pink
        10..=14 => ((180.0, 230.0, 255.0), 0.50), // ethereal cyan
        15..=24 => ((220.0, 200.0, 255.0), 0.55), // celestial violet
        _ => ((255.0, 250.0, 240.0), 0.65),       // divine white
    }
}

// IMPORTANT: the focal cell (asshole) is intentionally a SPACE in each
// art slice below. The renderer overpaints that cell with the live glyph
// (`O` / `*` / spin frame `\ | / - *`) at draw time, using the
// `asshole_col` / `asshole_row` declared on each `BiscuitArt`. We do NOT
// substitute glyphs into the strings — that ran into the obvious trap of
// `replace('O', '|')` collateral-damaging the `|` walls on rows 9-21,
// and the burning-pulse overpaint also got fooled into picking the
// wrong row when searching for a stand-in glyph.
// Body walls at cols 1 and 59 → visual center column 30. The right-side
// curve rows (2-8 top, 21-27 bottom) used to extend 1 column further from
// center than their left-side mirrors, so the right wall `|` sat at the
// same column as the curve `\` above it (no rounding offset) while the
// left side had `/` 1 col inside the wall — visibly asymmetric. Shifted
// the right cluster on each curve row 1 col left so the right contour now
// rounds into the wall the same way the left does.
//
// Rows 0, 1, 28, 29 (the underscore row + adjacent `__,-~~` row) sit at
// half-column offsets from the body center; their |L|/|R| asymmetry is
// inherent to the even-width middle gap, and a 1-col shift just flips
// which side is shorter. Left as-is.
const BISCUIT_FULL: &[&str] = &[
    r"                    ____________________                    ",
    r"              __,-~~                    ~~-,__              ",
    r"           ,-~'                               `~-,          ",
    r"        ,-'                                       `-,       ",
    r"      ,'                                             `.     ",
    r"     /         -~-~-~-              -~-~-~-            \    ",
    r"    /                                                   \   ",
    r"   /             -~~-~-~~-                               \  ",
    r"  /                                                       \ ",
    r" |          -~-~-~-~-             -~-~-~-~-                |",
    r" |                                                         |",
    r" |                                                         |",
    r" |                  \\\\\\\\   |   ////////                |",
    r" |                   \\\\\\\\  |  ////////                 |",
    r" |                    \\\\\\\\\|/////////                  |",
    r" |         ~ - - - - -                   - - - - - ~       |",
    r" |                    /////////|\\\\\\\\\                  |",
    r" |                   ////////  |  \\\\\\\\                 |",
    r" |                  ////////   |   \\\\\\\\                |",
    r" |                                                         |",
    r" |          -~-~-~-~-             -~-~-~-~-                |",
    r"  \                                                       / ",
    r"   \             -~~-~-~~-                               /  ",
    r"    \                                                   /   ",
    r"     \         -~-~-~-              -~-~-~-            /    ",
    r"      `.                                             ,'     ",
    r"        `-,                                       ,-'       ",
    r"           `~-,                               ,-~'          ",
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
    r" |    ~ - - -               - - - ~     |",
    r" |            //////|\\\\\\             |",
    r" |           /////  |  \\\\\            |",
    r" |          /////   |   \\\\\           |",
    r"  \                                    /",
    r"   \         -~-~-      -~-~-         / ",
    r"    `.                              ,'  ",
    r"      `-,                        ,-'    ",
    r"         `~-,,_______________,,-~'      ",
];

// Body walls sit at cols 1 and 25 → visual center column 13. The rounded
// contour rows (0-3, 9-11) used to terminate one column short of the wall
// on the right side, leaving a 2-column gap between e.g. `\` (row 3) and
// `|` (row 4). Symmetrized around col 13 so each row's left margin and
// right margin are equal — same shape now reads as a closed oval.
const BISCUIT_SMALL: &[&str] = &[
    r"        ___________       ",
    r"     ,-~           ~-,    ",
    r"   ,'                 `.  ",
    r"  /    -~-~-  -~-~-     \ ",
    r" |       \\\ | ///       | ",
    r" |        \\\|///        | ",
    r" | ~ - -           - - ~ | ",
    r" |        ///|\\\        | ",
    r" |       /// | \\\       | ",
    r"  \    -~-~-  -~-~-     / ",
    r"   `.                 ,'  ",
    r"     `-,,_________,,-'    ",
];

/// 16 cols × 8 rows. Borrowed by the tree-modal anchor (the (0, 0) lot)
/// to render the cuque-as-anchor instead of a regular upgrade box.
pub(crate) const BISCUIT_TINY: &[&str] = &[
    r"     ______     ",
    r"   ,~      ~,   ",
    r"  /          \  ",
    r" |    \|/     | ",
    r" | -       -  | ",
    r" |    /|\     | ",
    r"  \          /  ",
    r"   `-,____,-'   ",
];
/// Focal cell ("the asshole") of `BISCUIT_TINY`, expressed in art-local
/// (col, row) coords. Anchor render in the tree modal paints an `O` at
/// this cell so it reads as a cuque, not just an outline.
pub(crate) const BISCUIT_TINY_FOCAL: (u16, u16) = (7, 4);

/// One zoom level. `(asshole_col, asshole_row)` are the exact in-art
/// coordinates of the focal cell — declared at author time, never searched
/// for at draw time. The renderer prints the static `rows` verbatim (the
/// focal cell is a SPACE in the source) and then paints exactly one cell
/// at that coordinate with the live glyph (`O` / `*` / spin frame). Pros:
///   - centering doesn't depend on the glyph (spin frames are ASCII chars
///     that also appear elsewhere in the cuque outline; substitution
///     would either miss or hit-too-many cells);
///   - the burning-pulse overpaint targets exactly one cell, every frame,
///     with no string search to mis-fire on outline `|` walls;
///   - reorganizing or extending the art is a localized edit — bump the
///     coords here and the renderer follows.
///
/// Authors of new levels MUST update both coords when adding art.
struct BiscuitArt {
    rows: &'static [&'static str],
    asshole_col: u16,
    asshole_row: u16,
    label: Option<&'static str>,
}

const BISCUIT_LEVELS: &[BiscuitArt] = &[
    BiscuitArt {
        rows: BISCUIT_FULL,
        asshole_col: 31,
        asshole_row: 15,
        label: None,
    },
    BiscuitArt {
        rows: BISCUIT_MEDIUM,
        asshole_col: 20,
        asshole_row: 9,
        label: Some("70%"),
    },
    BiscuitArt {
        rows: BISCUIT_SMALL,
        asshole_col: 13,
        asshole_row: 6,
        label: Some("45%"),
    },
    BiscuitArt {
        rows: BISCUIT_TINY,
        asshole_col: 7,
        asshole_row: 4,
        label: Some("25%"),
    },
];

pub fn level_count() -> usize {
    BISCUIT_LEVELS.len()
}

/// Screen coordinates of the focal cell ("the asshole") for the given
/// zoom level + biscuit rect. Used by `hands::draw` to orbit the ring
/// around the visual cuque center rather than the bounding-box center
/// (which differs by up to 1 column in TINY/FULL because each art's
/// `asshole_col` isn't exactly `width / 2`).
pub fn focal_point(zoom_idx: usize, biscuit: Rect) -> (u16, u16) {
    let level = &BISCUIT_LEVELS[zoom_idx.min(BISCUIT_LEVELS.len() - 1)];
    (biscuit.x + level.asshole_col, biscuit.y + level.asshole_row)
}

pub fn level_label(idx: usize) -> Option<&'static str> {
    BISCUIT_LEVELS.get(idx).and_then(|a| a.label)
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
    let level = &BISCUIT_LEVELS[zoom_idx.min(BISCUIT_LEVELS.len() - 1)];
    let art = level.rows;
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
        squashed_art(art, level.asshole_row as usize)
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
    // Anchor placement to the ASSHOLE column declared on the art level.
    // Centering the bounding box by `(area.width - w) / 2` integer-truncates
    // and combines with each art's different in-art asshole column, so the
    // asshole drifts left/right as the player zooms. Anchoring the asshole
    // itself to a fixed screen column keeps the focal point stationary on
    // every zoom — the surrounding cuque shifts; the asshole doesn't. The
    // declared column is independent of which glyph happens to live in
    // that cell, so spin animation frames (`\ | / -`) work without breaking
    // alignment.
    let target_asshole_col = area.x + area.width / 2;
    let x_base = target_asshole_col
        .saturating_sub(level.asshole_col)
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

    // Render the static art lines as-is — the focal cell is a literal SPACE
    // in the source. The asshole glyph is painted directly into its
    // declared (asshole_col, asshole_row) cell after the body draw.
    let lines: Vec<Line> = render_art
        .iter()
        .map(|s| Line::from(s.to_string()))
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
        // Resting tan, then re-tinted toward the prestige tier color.
        // Higher tiers earn nobler hues so endgame feels visibly rewarded —
        // tan → warm gold → silver-pink → ethereal cyan → divine white.
        let (tint, mix) = prestige_body_tint(state.prestige);
        let base_r = 220.0 * breath;
        let base_g = 170.0 * breath;
        let base_b = 150.0 * breath;
        let r = base_r + (tint.0 - base_r) * mix;
        let g = base_g + (tint.1 - base_g) * mix;
        let b = base_b + (tint.2 - base_b) * mix;
        (
            r.clamp(0.0, 255.0),
            g.clamp(0.0, 255.0),
            b.clamp(0.0, 255.0),
        )
    };

    let color = Color::Rgb(base.0 as u8, base.1 as u8, base.2 as u8);
    let p = Paragraph::new(lines).style(Style::default().fg(color));
    frame.render_widget(p, render_rect);

    // Asshole glyph picker:
    //  - not clenched              → resting `O` (cuque body color)
    //  - clenched, space NOT held  → burning `*` with a hot pulsing red
    //  - clenched, space HELD ≥ 1s → spin frame `\ | / - *`, advanced by
    //                                `total_clicks % 5` so each repeat tick
    //                                rotates one step (`*` is the flash
    //                                frame in the cycle).
    //
    // Painted directly into the declared (asshole_col, asshole_row) cell —
    // no string substitution, no row search. The squash transform
    // preserves the asshole row index, so this works in both calm and
    // squashed states.
    let space_held = state.space_held();
    let asshole_glyph: char = if !clenched {
        'O'
    } else if space_held {
        SPIN_FRAMES[(state.total_clicks as usize) % SPIN_FRAMES.len()]
    } else {
        '*'
    };
    let buf = frame.buffer_mut();
    let cx = render_rect.x + level.asshole_col;
    let cy = render_rect.y + level.asshole_row;
    if cx < buf.area.x + buf.area.width && cy < buf.area.y + buf.area.height {
        let style = if clenched {
            // Pulse the focal cell at ~5Hz (period ~4 ticks at 20Hz) so the
            // asshole reads as actively burning, distinct from the merely
            // pink cuque body.
            let phase = (state.session_ticks as f32) * 0.8;
            let pulse = (phase.sin() + 1.0) * 0.5;
            let r = (200.0 + 55.0 * pulse) as u8;
            let g = (30.0 + 60.0 * pulse) as u8;
            Style::default()
                .fg(Color::Rgb(r, g, 0))
                .add_modifier(Modifier::BOLD)
        } else {
            // Resting `O` matches the cuque body color so it doesn't pop.
            Style::default().fg(color)
        };
        let cell = &mut buf[(cx, cy)];
        cell.set_char(asshole_glyph);
        cell.set_style(style);
    }
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
/// immediately above and below the asshole row and padding with a blank
/// row at top and bottom.
///
/// Why this shape: a real squash needs the centerline (asshole) to stay
/// anchored while the upper and lower halves contract toward it — that's
/// what reads as a flattened ellipsoid. Just shrinking from the top makes
/// the cuque look like the topmost row is flickering, not pulsing.
///
/// Why the blank padding: total row count MUST be preserved. The biscuit
/// rect that this function feeds is read by `hands::draw` to place the
/// orbital fingerers — any change to rect.height (or rect.y, via
/// recentering) would shift every hand around the cuque on every click.
/// Padding keeps the rect identical between calm and squashed states.
///
/// Critical invariant: the asshole row's index in the output is the same
/// as `asshole_row` in the input, so the renderer can use the level's
/// declared `asshole_row` regardless of squash state.
///
/// Falls back to a plain copy if the art is too short to safely drop two
/// rows around the asshole row.
fn squashed_art(art: &[&str], asshole_row: usize) -> Vec<String> {
    let n = art.len();
    if n < 5 || asshole_row == 0 || asshole_row + 1 >= n {
        return art.iter().map(|s| s.to_string()).collect();
    }
    let width = art.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let blank: String = " ".repeat(width);

    let mut out: Vec<String> = Vec::with_capacity(n);
    out.push(blank.clone()); // top pad replaces the dropped (asshole_row - 1)
    for s in art.iter().take(asshole_row - 1) {
        out.push((*s).to_string());
    }
    out.push(art[asshole_row].to_string());
    for s in art.iter().skip(asshole_row + 2) {
        out.push((*s).to_string());
    }
    out.push(blank); // bottom pad replaces the dropped (asshole_row + 1)
    debug_assert_eq!(out.len(), n);
    out
}

/// Per-kind palette/glyph table. `bright`/`dim`/`accent` are the three
/// colors the shimmer wave samples; `bg` stays low-key so the eye reads
/// the *text* sliding through hues, not a blinking box. Centralized here
/// so a new `PowerupKind` is one extra arm.
struct PowerupPalette {
    center: char,
    bright: (f32, f32, f32),
    dim: (f32, f32, f32),
    accent: (f32, f32, f32),
    bg: Color,
}

fn powerup_palette(kind: PowerupKind) -> PowerupPalette {
    match kind {
        PowerupKind::Lucky => PowerupPalette {
            center: '$',
            bright: (255.0, 230.0, 80.0),
            dim: (140.0, 90.0, 0.0),
            accent: (255.0, 170.0, 30.0),
            bg: Color::Rgb(40, 25, 0),
        },
        PowerupKind::Frenzy => PowerupPalette {
            center: '!',
            bright: (255.0, 110.0, 110.0),
            dim: (120.0, 0.0, 0.0),
            accent: (255.0, 200.0, 60.0),
            bg: Color::Rgb(50, 0, 0),
        },
        PowerupKind::Buff => PowerupPalette {
            center: '+',
            bright: (230.0, 160.0, 255.0),
            dim: (80.0, 20.0, 110.0),
            accent: (140.0, 220.0, 255.0),
            bg: Color::Rgb(35, 0, 45),
        },
        PowerupKind::GreenCoin => PowerupPalette {
            center: '$',
            bright: (140.0, 255.0, 160.0),
            dim: (10.0, 80.0, 30.0),
            accent: (200.0, 255.0, 110.0),
            bg: Color::Rgb(0, 30, 10),
        },
    }
}

/// J9 juice: the marker shimmers. Each character of the 5-wide marker
/// samples its own foreground color from a horizontally-traveling wave
/// between a `bright` peak, a `dim` trough, and an `accent` highlight on
/// the off-phase. The bg stays a constant low-key tint, so what the player
/// sees is the TEXT itself sliding through colors — not a flashing box.
/// In the final 20% of life the wave speeds up and the trough darkens so
/// a soon-to-expire powerup visibly accelerates without losing legibility.
///
/// Unified across kinds: `Lucky`/`Frenzy`/`Buff`/`GreenCoin` differ only
/// in their `powerup_palette` entry. Adding a new kind = add a palette
/// arm; this function inherits it.
pub fn draw_powerup(frame: &mut Frame, powerup: &Powerup, biscuit: Rect) -> Rect {
    let buf = frame.buffer_mut();
    let PowerupPalette {
        center,
        bright,
        dim,
        accent,
        bg,
    } = powerup_palette(powerup.kind);

    // Wave speed (rad/tick) and trough depth both bump in alarm mode. The
    // normal phase pulses at ~1.9 Hz (0.6 rad/tick × 20 ticks/s ÷ TAU);
    // the alarm phase doubles to ~4.8 Hz so a soon-to-expire powerup
    // visibly speeds up. Earlier values (0.22 / 0.55) read as too sleepy.
    let life_total = powerup.kind.lifetime_ticks();
    let life_frac = (powerup.life_ticks as f32 / life_total as f32).clamp(0.0, 1.0);
    let alarm = life_frac < 0.20;
    let speed = if alarm { 1.5 } else { 0.6 };
    let dim_pull = if alarm { 1.0 } else { 0.6 };
    // Phase advances every tick; per-cell offset shifts the wave across the
    // 5-cell width so neighbors land at different points of the gradient.
    let phase = (life_total - powerup.life_ticks) as f32 * speed;
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
        crate::game::state::biscuit_frac_to_screen(powerup.frac_x, powerup.frac_y, biscuit);
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
