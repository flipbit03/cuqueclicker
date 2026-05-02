pub mod achievements;
pub mod biscuit;
pub mod border;
pub mod debug_pane;
pub mod effects;
pub mod hands;
pub mod prestige;
pub mod sidebar;
pub mod stats;
pub mod toast;
pub mod upgrades;

use ratatui::{prelude::*, widgets::*};

use crate::format;
use crate::game::state::{Buff, GameState, HUD_FLASH_TICKS, TICK_HZ};
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

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    Game,
    Stats,
    Achievements,
    Upgrades,
    Prestige,
}

/// Click target for a help-bar hint or for the prestige-reset confirm
/// line. Mirrors the keyboard shortcuts so the mouse-first player has
/// equivalent reach to every action a key would fire.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelpAction {
    /// Open the named mode (or close it back to Game if already there).
    OpenMode(Mode),
    /// Catch whatever golden is on screen.
    GrabGolden,
    /// Confirm-and-claim prestige reset (only when prestige_available > 0).
    PrestigeReset,
    /// Quit the program.
    Quit,
}

/// Per-frame layout snapshot produced by [`draw`]. Single source of truth
/// for every clickable region on screen + the play-area envelope.
///
/// The platform shells (`app.rs`, `wasm_app.rs`) **store this verbatim**
/// and call [`crate::input::InputContext::from_layout`] to project it
/// into the per-event input context. Adding a new clickable region only
/// touches this struct + `InputContext` + the projection — never the
/// platform code. (We were burned by the alternative once: a missing
/// `green_coin_rect` field on the wasm-side `InputContext` constructor
/// crashed the wasm build after the native code shipped just fine.)
#[derive(Default)]
pub struct DrawOutput {
    pub biscuit_rect: Rect,
    /// Screen position of the biscuit's focal cell ("the asshole"). Each
    /// zoom level's art has the focal at a slightly different offset
    /// inside its bounding box (TINY: col 7 of width 16, FULL: col 31 of
    /// width 60, etc.), so the bbox center isn't the visual center. This
    /// drives `hands::draw`'s orbit center.
    pub biscuit_focal: (u16, u16),
    pub golden_rect: Rect,
    /// On-screen Green Coin marker rect, or zero-rect when no coin is
    /// visible. Hit-tested by the click router exactly like `golden_rect`
    /// — clicking either one routes through `Action::CatchGolden`, which
    /// the sim resolves by trying both catch paths.
    pub green_coin_rect: Rect,
    /// The whole left column where the biscuit + hands + particles live —
    /// i.e. "the box that displays the ass." Used by the input router so
    /// the scroll-wheel zoom fires anywhere in this region (including the
    /// vast empty space around a small biscuit at low zoom), and only the
    /// right-hand sidebar opts out of zoom.
    pub play_area: Rect,
    /// `(upgrade_idx, screen_row_rect)` pairs for the Upgrades panel —
    /// populated only when the active mode renders that panel; empty
    /// otherwise. The click router hit-tests these for `BuyUpgrade`.
    /// First element of each tuple is also the digit-shortcut target,
    /// kept aligned with `visible_upgrades`.
    pub upgrade_rows: Vec<(usize, Rect)>,
    /// `(fingerer_idx, screen_row_rect)` for the Game-mode sidebar.
    pub fingerer_rows: Vec<(usize, Rect)>,
    /// (action, rect) for every clickable help-bar hint at the bottom of
    /// the play column. Mouse-first players use these to switch panels,
    /// catch goldens, prestige-reset, and quit — all of which used to be
    /// keyboard-only. Empty rects when the hint is non-actionable
    /// (e.g. `[Space/Click] finger` is informational, not a click target).
    pub help_hits: Vec<(HelpAction, Rect)>,
    /// Click rect for the `Press [r] to reset and claim` confirm line in
    /// the Prestige panel. Default rect when not in Prestige mode or no
    /// prestige is available.
    pub prestige_reset_rect: Rect,
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
    let text = format!("zoom {}", label);
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
    mouse_pos: Option<(u16, u16)>,
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

    // J5 count-up: render the smoothed `displayed_*` values rather than the
    // raw current values. Big jumps (golden, max-buy, F4) ease in instead of
    // snapping. Tween itself runs in `state.tick()`.
    //
    // Color sweep: TWO competing channels — green for cuques going UP
    // (income, golden, F4), red for cuques going DOWN (purchase,
    // prestige reset). Whichever channel is stronger this frame drives
    // the lerp toward white. So a buy that lands during a still-decaying
    // gain flash correctly flips the digits red as the spend channel
    // overtakes the fading gain. Both lerp toward bright white at t=0,
    // which matches the resting (no-flash) style — no hard cut.
    let gain_t = (state.cuques_flash_ticks as f32 / HUD_FLASH_TICKS as f32).clamp(0.0, 1.0);
    let spend_t = (state.cuques_spend_flash_ticks as f32 / HUD_FLASH_TICKS as f32).clamp(0.0, 1.0);
    const FLASH_GAIN: (f32, f32, f32) = (80.0, 255.0, 80.0); // bright green
    const FLASH_SPEND: (f32, f32, f32) = (255.0, 90.0, 90.0); // urgent red
    const FLASH_REST: (f32, f32, f32) = (255.0, 255.0, 255.0);
    let (peak, t) = if spend_t > gain_t {
        (FLASH_SPEND, spend_t)
    } else {
        (FLASH_GAIN, gain_t)
    };
    let mix = 1.0 - t;
    let r = peak.0 + (FLASH_REST.0 - peak.0) * mix;
    let g = peak.1 + (FLASH_REST.1 - peak.1) * mix;
    let b = peak.2 + (FLASH_REST.2 - peak.2) * mix;
    let cuques_style = Style::default()
        .fg(Color::Rgb(
            r.clamp(0.0, 255.0) as u8,
            g.clamp(0.0, 255.0) as u8,
            b.clamp(0.0, 255.0) as u8,
        ))
        .add_modifier(Modifier::BOLD);
    let mut hud_spans: Vec<Span> = vec![
        Span::raw(format!("{}: ", lang.hud_cuques)),
        Span::styled(format::big(state.displayed_cuques), cuques_style),
        Span::raw(format!(
            "   {}: {}",
            lang.hud_fps,
            format::rate(state.displayed_fps)
        )),
    ];
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
        };
        hud_spans.push(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    // Active timed per-fingerer modifiers — Purple Coin Buff golden today,
    // anything else timed in the future. Phase 5 of #21 will replace this
    // with a dedicated HUD strip; for now we mirror the legacy chip layout
    // so UX continuity holds across phases.
    for (id, st) in &state.fingerers_state {
        for m in &st.modifiers {
            let crate::game::modifier::ModifierDuration::Ticks(remaining) = m.duration else {
                continue;
            };
            let secs = remaining.div_ceil(TICK_HZ);
            let idx = crate::game::fingerer::FINGERERS
                .iter()
                .position(|f| f.id == id);
            let name = idx
                .and_then(|i| lang.fingerer_names.get(i).copied())
                .unwrap_or("?");
            // Pick a number to show: prefer the strongest single MulFactor
            // effect (matches the old "x7" presentation); fall back to a
            // count-of-effects marker for purely additive sources.
            let mul = m.effects.iter().find_map(|e| match e {
                crate::game::modifier::ModifierEffect::MulFactor(v) => Some(*v),
                _ => None,
            });
            let label = match mul {
                Some(v) => format!("  [++ {} x{} {}s]", name, v as u64, secs),
                None => format!("  [++ {} {}s]", name, secs),
            };
            let color = match m.source {
                crate::game::modifier::ModifierSource::PurpleCoin => Color::Rgb(220, 140, 255),
                crate::game::modifier::ModifierSource::GreenCoin => Color::Rgb(120, 230, 140),
            };
            hud_spans.push(Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
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

    let biscuit_rect = biscuit::draw(frame, left[1], state, zoom_idx);
    let biscuit_focal = biscuit::focal_point(zoom_idx, biscuit_rect);
    hands::draw(frame, left[1], biscuit_rect, biscuit_focal, state);
    effects::draw_particles(frame, biscuit_rect, &state.particles);
    effects::draw_misclicks(frame, &state.misclick_particles);
    draw_zoom_indicator(
        frame,
        left[1],
        biscuit::level_label(zoom_idx).unwrap_or("100%"),
    );

    if debug {
        debug_pane::draw(frame, left[1]);
    }
    let golden_rect = match &state.golden {
        Some(g) => biscuit::draw_golden(frame, g, biscuit_rect),
        None => Rect::default(),
    };
    let green_coin_rect = match &state.green_coin {
        Some(c) => biscuit::draw_green_coin(frame, c, biscuit_rect),
        None => Rect::default(),
    };

    // J1: achievement toast overlay. Lives in `left[1]` (biscuit/main area)
    // so it covers nothing important on the right; auto-dismisses after
    // TOAST_TICKS via the sim. We render *after* biscuit/golden so it
    // always sits on top.
    toast::draw(frame, left[1], state);

    // Custom help-bar render: lay out `[X] label` tokens left-to-right,
    // wrapping at the rect width, paint each with mode-aware styling
    // (active mode bolded), and return per-token click rects so the
    // mouse-first player can drive the game without ever touching a key.
    let help_hits = draw_help(frame, left[2], help_text, mode, mouse_pos);

    let mut upgrade_rows: Vec<(usize, Rect)> = Vec::new();
    let mut fingerer_rows: Vec<(usize, Rect)> = Vec::new();
    let mut prestige_reset_rect = Rect::default();
    match mode {
        Mode::Game => fingerer_rows = sidebar::draw(frame, cols[1], state, mouse_pos),
        Mode::Stats => stats::draw(frame, cols[1], state),
        Mode::Achievements => achievements::draw(frame, cols[1], state),
        Mode::Upgrades => upgrade_rows = upgrades::draw(frame, cols[1], state, mouse_pos),
        Mode::Prestige => prestige_reset_rect = prestige::draw(frame, cols[1], state, mouse_pos),
    }

    DrawOutput {
        biscuit_rect,
        biscuit_focal,
        golden_rect,
        green_coin_rect,
        play_area: left[1],
        upgrade_rows,
        fingerer_rows,
        help_hits,
        prestige_reset_rect,
    }
}

/// Custom help-bar renderer.
///
/// Splits the help string into "tokens" (each token is a contiguous
/// non-whitespace run of `[X] label words`, separated from the next by
/// a double space or a newline — the convention used in `i18n::Lang`'s
/// help strings). Each token is laid out left-to-right with wrap at
/// the rect's width, painted at the resolved screen position, and
/// matched against a (mode, key) → action table. Clickable tokens get
/// a slightly brighter color and BOLD; the token under the mouse
/// cursor gets an additional brightness lift + bg fill so the player
/// reads it as a button.
fn draw_help(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    mode: Mode,
    mouse_pos: Option<(u16, u16)>,
) -> Vec<(HelpAction, Rect)> {
    let mut hits: Vec<(HelpAction, Rect)> = Vec::new();
    if area.width == 0 || area.height == 0 {
        return hits;
    }
    let buf = frame.buffer_mut();
    let mut cursor_x: u16 = 0;
    let mut cursor_y: u16 = 0;
    for line in text.split('\n') {
        // Tokens are separated by a literal `  ` (two spaces). Single
        // spaces inside a token are content (e.g. "back to game").
        for token in line.split("  ") {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let w = token.chars().count() as u16;
            // Wrap if the token wouldn't fit on the current line.
            if cursor_x + w > area.width && cursor_x > 0 {
                cursor_y += 1;
                cursor_x = 0;
            }
            if cursor_y >= area.height {
                break;
            }
            let action = map_help_token(token, mode);
            // Hide `[q] quit` (or its localized equivalent) on platforms
            // where the wasm/native runner has no authority to exit —
            // see `platform::Capabilities::can_quit`. Skipping renders
            // AND skips appending to `help_hits`, so the next token
            // slides into the position cursor without leaving a gap.
            if matches!(action, Some(HelpAction::Quit)) && !crate::platform::CAPABILITIES.can_quit {
                continue;
            }
            let active = matches!(action, Some(HelpAction::OpenMode(m)) if m == mode);
            let token_rect = Rect {
                x: area.x + cursor_x,
                y: area.y + cursor_y,
                width: w.min(area.width.saturating_sub(cursor_x)),
                height: 1,
            };
            // Style picker:
            //  - active mode hint                : bright yellow, BOLD
            //  - actionable (clickable)          : light gray, BOLD
            //  - informational                   : dark gray
            //  - hovered                         : color lifted, bg tint
            // Hover lift fires ONLY on actionable hints — informational
            // tokens like `[Space/Click] finger` and `[Shift] x10` are
            // descriptive labels with no click handler, so brightening
            // them on hover would advertise a button that doesn't exist.
            let hovered = action.is_some()
                && mouse_pos
                    .map(|(mx, my)| {
                        mx >= token_rect.x
                            && mx < token_rect.x + token_rect.width
                            && my == token_rect.y
                    })
                    .unwrap_or(false);
            let mut style = if active {
                Style::default()
                    .fg(Color::Rgb(255, 220, 120))
                    .add_modifier(Modifier::BOLD)
            } else if action.is_some() {
                Style::default()
                    .fg(Color::Rgb(180, 180, 180))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            if hovered {
                style = style
                    .fg(Color::Rgb(255, 255, 255))
                    .bg(Color::Rgb(40, 40, 50))
                    .add_modifier(Modifier::BOLD);
            }
            buf.set_string(token_rect.x, token_rect.y, token, style);
            if let Some(a) = action {
                hits.push((a, token_rect));
            }
            cursor_x += w + 2; // double-space separator
        }
        cursor_y += 1;
        cursor_x = 0;
        if cursor_y >= area.height {
            break;
        }
    }
    hits
}

/// Match a help-bar token like `"[u] upgrades"` to a `HelpAction`,
/// disambiguated by the current mode (so `[s] stats` opens Stats from
/// Game but `[s/Esc] back to game` from Stats returns to Game).
fn map_help_token(token: &str, mode: Mode) -> Option<HelpAction> {
    // Extract the bracketed key. We accept the first `[...]` group;
    // everything after the first `]` is descriptive label.
    let open = token.find('[')?;
    let close = token[open + 1..].find(']')? + open + 1;
    let key = &token[open + 1..close];
    // Universal hints first.
    if key.eq_ignore_ascii_case("q") {
        return Some(HelpAction::Quit);
    }
    // Back-to-game from any non-Game mode (the `[X/Esc] back ...` pattern
    // covers stats / achievements / upgrades / prestige).
    if mode != Mode::Game && (key.contains("Esc") || key.contains("esc")) {
        return Some(HelpAction::OpenMode(Mode::Game));
    }
    // Single-letter mode openers, only meaningful from Game.
    match (mode, key) {
        (Mode::Game, "u") | (Mode::Game, "U") => Some(HelpAction::OpenMode(Mode::Upgrades)),
        (Mode::Game, "p") | (Mode::Game, "P") => Some(HelpAction::OpenMode(Mode::Prestige)),
        (Mode::Game, "s") | (Mode::Game, "S") => Some(HelpAction::OpenMode(Mode::Stats)),
        (Mode::Game, "a") | (Mode::Game, "A") => Some(HelpAction::OpenMode(Mode::Achievements)),
        (Mode::Game, "g") | (Mode::Game, "G") => Some(HelpAction::GrabGolden),
        (Mode::Prestige, "r") | (Mode::Prestige, "R") => Some(HelpAction::PrestigeReset),
        _ => None,
    }
}
