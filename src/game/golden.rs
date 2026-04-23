use rand::RngExt;

/// A Golden Cuque variant. The on-screen marker color + label differ per
/// variant, but all are caught through the same path (mouse click on the
/// marker, or the `g` hotkey). See `GameState::catch_golden` for effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenVariant {
    /// Instant flat reward: `max(10, fps * 60s)` cuques. No buff, no duration.
    Lucky,
    /// 13-second `ClickFrenzy` buff: manual clicks produce x777 cuques.
    Frenzy,
    /// 60-second `FingererBoost` buff on a random owned fingerer: that
    /// fingerer's passive FPS is multiplied x7.
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

#[derive(Clone)]
pub struct GoldenCuque {
    pub col: u16,
    pub row: u16,
    pub life_ticks: u32,
    pub variant: GoldenVariant,
}

pub const GOLDEN_LIFE_TICKS: u32 = 220; // ~11s at 20Hz
pub const GOLDEN_COOLDOWN_MIN: u32 = 400; // 20s
pub const GOLDEN_COOLDOWN_MAX: u32 = 1600; // 80s

pub fn next_cooldown() -> u32 {
    rand::rng().random_range(GOLDEN_COOLDOWN_MIN..=GOLDEN_COOLDOWN_MAX)
}

pub fn spawn_in(area_col_range: (u16, u16), area_row_range: (u16, u16)) -> GoldenCuque {
    let mut r = rand::rng();
    let col = r.random_range(area_col_range.0..=area_col_range.1.max(area_col_range.0));
    let row = r.random_range(area_row_range.0..=area_row_range.1.max(area_row_range.0));
    GoldenCuque {
        col,
        row,
        life_ticks: GOLDEN_LIFE_TICKS,
        variant: GoldenVariant::random(),
    }
}
