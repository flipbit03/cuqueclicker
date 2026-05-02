//! Unified powerup model — replaces the old per-variant slots
//! (`GoldenCuque[3]` + `GreenCoin`) with a single `Vec<Powerup>` keyed by
//! `PowerupKind`.
//!
//! Adding a new kind is a single-file change: extend `PowerupKind`, give it
//! `lifetime_ticks` / `mean_cooldown_ticks`, add a render branch, and add a
//! catch-effect arm in `GameState::catch_powerup`. Every other system —
//! input routing, tick loop, persistence, achievements — inherits the new
//! kind without further plumbing.
//!
//! Spawn timing is **truncated exponential per kind**, sampled inter-arrival
//! (one RNG draw per spawn, not a per-tick roll). Per-kind cooldown clocks
//! tick independently and don't freeze when the kind already has on-screen
//! instances — the Vec is unbounded and pile-ups self-resolve via the
//! short lifetime.

use rand::RngExt;

use crate::game::state::TICK_HZ;

/// Stable discriminator for every kind of powerup. Order also defines the
/// indexing into `GameState::powerup_cooldowns: [u32; N_KINDS]`, so don't
/// reorder once shipped — a save written under one ordering would index a
/// different kind's cooldown after a swap. (Cooldowns are `#[serde(skip)]`
/// today, so the actual blast radius is "fresh-load reseeds them anyway,"
/// but treat the order as stable to keep test fixtures honest.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerupKind {
    /// Instant flat reward: `max(10, fps * 60s)` cuques. No buff, no
    /// duration. Frequent, small.
    Lucky,
    /// 13-second `ClickFrenzy` buff: each manual click during the buff
    /// adds an FPS-scaled bonus on top of normal click power. Less
    /// common, disruptive while it lasts; the FPS scaling means a
    /// late-game Frenzy is much more rewarding than an early-game one
    /// (and the early-game version stays bounded against the cost
    /// ladder).
    Frenzy,
    /// 60-second per-fingerer modifier on a random *owned* fingerer:
    /// that fingerer's passive FPS is multiplied x7. Implemented as a
    /// `ModifierSource::PurpleCoin` `MulFactor(7.0)` with `Ticks(1200)`.
    Buff,
    /// Permanent +10% AddPercent modifier on a random *visible* fingerer
    /// (sidebar-visible — including unowned tiers above the visibility
    /// threshold; Index Finger is always eligible). Rare; sourced as
    /// `ModifierSource::GreenCoin`.
    GreenCoin,
}

/// Number of distinct powerup kinds — must match `PowerupKind::ALL.len()`.
/// Used to size `GameState::powerup_cooldowns`.
pub const N_KINDS: usize = 4;

/// Compile-time guard against `N_KINDS` drifting out of sync with the
/// `PowerupKind::ALL` array. If a new variant is added to `PowerupKind`
/// and `ALL` is updated but `N_KINDS` isn't (or vice versa), the build
/// fails here instead of producing silent index-out-of-bounds at runtime
/// against `GameState::powerup_cooldowns: [u32; N_KINDS]`.
const _: () = assert!(PowerupKind::ALL.len() == N_KINDS);

impl PowerupKind {
    /// Stable iteration order. Mirrors discriminant order so
    /// `kind as usize` indexes into anything sized to `N_KINDS`.
    pub const ALL: [Self; N_KINDS] = [Self::Lucky, Self::Frenzy, Self::Buff, Self::GreenCoin];

    /// On-screen lifetime for this kind. ~11s at 20Hz across all kinds for
    /// now — long enough to see and react, short enough to be a meaningful
    /// catch. Keep on `PowerupKind` so tuning is one number per kind.
    pub fn lifetime_ticks(self) -> u32 {
        match self {
            Self::Lucky | Self::Frenzy | Self::Buff | Self::GreenCoin => 220,
        }
    }

    /// Mean inter-arrival time (in ticks) for this kind. Single tuning
    /// knob — raise to make the kind rarer, lower to see it more often.
    pub fn mean_cooldown_ticks(self) -> u32 {
        match self {
            Self::Lucky => 60 * TICK_HZ,      // ~60s avg — frequent, small
            Self::Frenzy => 120 * TICK_HZ,    // ~120s avg — fps-scaled click bonus
            Self::Buff => 120 * TICK_HZ,      // ~120s avg — x7 fingerer mult
            Self::GreenCoin => 240 * TICK_HZ, // ~240s avg — rarest, permanent +10%
        }
    }

    /// Safety floor on the truncated exponential. Prevents pathologically
    /// rapid spawns from the left tail (two of the same kind appearing in
    /// the same second by chance and visually overlapping).
    pub fn min_cooldown_ticks(self) -> u32 {
        5 * TICK_HZ
    }

    /// Safety ceiling on the truncated exponential. Catches the bottom ~2%
    /// of the right tail; everything below 4× the mean passes through
    /// unchanged. Prevents the player from feeling like a kind has been
    /// "removed" during a particularly long drought.
    pub fn max_cooldown_ticks(self) -> u32 {
        4 * self.mean_cooldown_ticks()
    }
}

/// One on-screen powerup. Position is biscuit-fractional ([0, 1] on each
/// axis), same convention as `Particle` and the old `GoldenCuque` —
/// the marker stays anchored to its spot when the terminal resizes or the
/// user zooms.
///
/// `spawn_id` is a stable, monotonic identifier minted from
/// `GameState::next_spawn_id` at spawn time. Click hit-testing and the `g`
/// hotkey reference instances by id, never by Vec index — Vec indices shift
/// on `swap_remove`, and the input router holds layout rects across multiple
/// events, so a stable id is the only safe way to disambiguate "this
/// specific Frenzy among the three on screen."
#[derive(Clone, Debug)]
pub struct Powerup {
    pub kind: PowerupKind,
    pub spawn_id: u64,
    pub frac_x: f32,
    pub frac_y: f32,
    pub life_ticks: u32,
}

/// Sample the next inter-arrival cooldown for this kind. Truncated
/// exponential: `Exp(1/mean)` clamped to `[min, max]` safety rails.
///
/// Memoryless / bursty / "truly random" arrivals — the genre's natural
/// distribution. Per-spawn cost: one RNG draw + one `ln` + one `round` +
/// one `clamp` (~25ns). Called once every couple of seconds across all
/// kinds combined; effectively free.
pub fn next_cooldown(kind: PowerupKind) -> u32 {
    let mean = kind.mean_cooldown_ticks() as f64;
    let min = kind.min_cooldown_ticks();
    let max = kind.max_cooldown_ticks();
    // Inverse-CDF of `Exp(1/mean)`: `-ln(1 - u) * mean` for `u ∈ [0, 1)`.
    // Sampling `1.0 - u` keeps the argument in `(0, 1]` so `ln()` never
    // hits `-∞` when `random()` returns 0.0.
    let u: f64 = rand::rng().random();
    let raw = -(1.0 - u).ln() * mean;
    raw.round().clamp(min as f64, max as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_cooldown_in_truncated_exponential_range() {
        // Statistical sanity — every sample lands in [min, max], and the
        // empirical mean is in the right ballpark. Not a tight test (we
        // don't fix the RNG), just enough to catch a "forgot to multiply
        // by mean" or "swapped min/max" regression.
        for kind in PowerupKind::ALL {
            let min = kind.min_cooldown_ticks();
            let max = kind.max_cooldown_ticks();
            let mean = kind.mean_cooldown_ticks() as f64;
            let n = 10_000;
            let mut sum: f64 = 0.0;
            for _ in 0..n {
                let v = next_cooldown(kind);
                assert!(
                    v >= min && v <= max,
                    "next_cooldown({kind:?}) = {v} not in [{min}, {max}]"
                );
                sum += v as f64;
            }
            let empirical_mean = sum / n as f64;
            // Truncation pulls the empirical mean a bit below the
            // exponential's nominal mean (right-tail clipped to 4×mean).
            // ±25% is generous enough that flakes are extremely rare.
            assert!(
                empirical_mean > mean * 0.55 && empirical_mean < mean * 1.25,
                "empirical mean {empirical_mean} for {kind:?} too far from {mean}"
            );
        }
    }
}
