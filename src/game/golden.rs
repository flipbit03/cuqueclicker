use rand::RngExt;
use ratatui::layout::Rect;

/// A Golden Cuque variant. The on-screen marker color + label differ per
/// variant, but all are caught through the same path (mouse click on the
/// marker, or the `g` hotkey). See `GameState::catch_golden` for effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenVariant {
    /// Instant flat reward: `max(10, fps * 60s)` cuques. No buff, no duration.
    Lucky,
    /// 13-second `ClickFrenzy` buff: manual clicks produce x777 cuques.
    Frenzy,
    /// 60-second per-fingerer modifier on a random owned fingerer: that
    /// fingerer's passive FPS is multiplied x7. Implemented as a
    /// `ModifierSource::PurpleCoin` `MulFactor(7.0)` with `Ticks(1200)`.
    Buff,
}

impl GoldenVariant {
    pub fn random() -> Self {
        let r: f64 = rand::rng().random();
        if r < 0.6 {
            Self::Lucky
        } else if r < 0.8 {
            Self::Frenzy
        } else {
            Self::Buff
        }
    }
}

/// Position is stored as a fraction of the biscuit rect ([0.0, 1.0] on each
/// axis) so the marker stays anchored to its spot on the biscuit when the
/// terminal resizes or the user zooms (`+`/`-`). The renderer resolves these
/// fractions against the *current* biscuit rect every frame.
#[derive(Clone)]
pub struct GoldenCuque {
    pub frac_x: f32,
    pub frac_y: f32,
    pub life_ticks: u32,
    pub variant: GoldenVariant,
}

pub const GOLDEN_LIFE_TICKS: u32 = 220; // ~11s at 20Hz
pub const GOLDEN_COOLDOWN_MIN: u32 = 400; // 20s
pub const GOLDEN_COOLDOWN_MAX: u32 = 1600; // 80s
// Inset (in fractional units) the spawn lottery away from the biscuit edges
// so the 5x3 marker has room to render without bumping into the border.
const SPAWN_INSET_X: f32 = 0.08;
const SPAWN_INSET_Y: f32 = 0.10;

pub fn next_cooldown() -> u32 {
    rand::rng().random_range(GOLDEN_COOLDOWN_MIN..=GOLDEN_COOLDOWN_MAX)
}

/// Pick a random fractional position inside the biscuit, away from the edges.
/// `_biscuit` is taken so the signature documents intent (the spawn area is
/// "inside this rect"); the actual fractions are rect-independent.
pub fn spawn_in(_biscuit: Rect) -> GoldenCuque {
    let mut r = rand::rng();
    let frac_x = r.random_range(SPAWN_INSET_X..=(1.0 - SPAWN_INSET_X));
    let frac_y = r.random_range(SPAWN_INSET_Y..=(1.0 - SPAWN_INSET_Y));
    GoldenCuque {
        frac_x,
        frac_y,
        life_ticks: GOLDEN_LIFE_TICKS,
        variant: GoldenVariant::random(),
    }
}
