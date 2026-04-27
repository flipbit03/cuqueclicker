use std::collections::{HashMap, HashSet};

use rand::RngExt;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::game::achievement::ACHIEVEMENTS;
use crate::game::fingerer::{self, FINGERERS};
use crate::game::golden::GoldenCuque;
use crate::game::upgrade::{UPGRADES, UpgradeEffect};

pub const TICK_HZ: u32 = 20;
pub const TICK_DT: f64 = 1.0 / TICK_HZ as f64;
/// How long the biscuit stays "clenched" (eye→`*`, color shifts pink, art
/// vertically squashes by one row). Bumped from 3 to 6 so a single click is
/// actually visible — at 20Hz, 3 ticks (~150ms) was hard to perceive.
pub const CLENCH_TICKS: u32 = 6;
/// First `CLENCH_SQUASH_TICKS` of a clench draw the biscuit one row shorter
/// (top blank dropped, art shifted) so each finger reads as a real squish
/// before springing back. Strict subset of CLENCH_TICKS.
pub const CLENCH_SQUASH_TICKS: u32 = 2;
const PARTICLE_LIFE: u32 = 20;
/// Misclick "·" lifetime — short, just enough to acknowledge the attempt.
pub const MISCLICK_LIFE: u32 = 8;
/// Achievement-unlock toast: how long the popup stays on screen.
pub const TOAST_TICKS: u32 = TICK_HZ * 4;
/// HUD digit "I just got bigger" green flash duration.
pub const HUD_FLASH_TICKS: u32 = TICK_HZ; // 1s
/// Achievement-unlock border channel duration (gold pulse like Lucky but
/// shorter — celebratory, not lingering).
pub const ACHIEVEMENT_FLASH_TICKS: u32 = TICK_HZ * 2;
/// Per-tick upward drift for a particle, expressed as a fraction of the
/// biscuit's height. Calibrated to match the original feel before the
/// switch to fractional anchors: the old code rose 0.18 cells/tick on
/// any biscuit size; on a typical ~30-row biscuit that's 0.006 of height
/// per tick — slow enough that a "+1" only travels ~10-12% of the biscuit
/// across its 1-second life, instead of streaking across half of it.
const PARTICLE_FRAC_RISE: f32 = 0.006;
const GOLDEN_REWARD_SECONDS: f64 = 60.0;
const GOLDEN_REWARD_FLAT: f64 = 10.0;

/// Visual flavor for a particle. Drives color/weight in the renderer; the
/// motion model (rise + horizontal drift) is identical across kinds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    /// Default `+1` from a normal click — white→red fade.
    Click,
    /// High-power click (Frenzy x777, big mults). Bold + warm-yellow accent
    /// so it stands out from a swarm of `+1`s.
    ClickBig,
    /// Auto-fingerer income particle.
    Auto,
    /// Golden-catch label ("FRENZY x777!", "+1.2k", etc). Longer life,
    /// brighter palette.
    Golden,
    /// Bulk-buy confetti pop. Coloured glyphs, shorter than a click.
    Confetti,
}

/// Position is stored as a fraction of the biscuit rect ([0.0, 1.0] on each
/// axis), matching `GoldenCuque`. The renderer resolves these fractions
/// against the *current* biscuit rect every frame, so particles travel with
/// the biscuit when the terminal resizes or the user zooms.
#[derive(Clone)]
pub struct Particle {
    pub frac_x: f32,
    pub frac_y: f32,
    pub life: u32,
    pub text: String,
    pub kind: ParticleKind,
    /// Per-tick horizontal drift in fraction-of-biscuit units. Set at spawn
    /// from a small uniform so co-spawned particles separate as they rise
    /// instead of stacking into garbage like `++1++++1`.
    pub drift_x: f32,
}

/// Screen-anchored particle (raw col/row, not biscuit-fractional). Used for
/// misclick acknowledgement: a small grey "·" at the exact dead-zone click
/// point so the player knows the click registered but missed every target.
#[derive(Clone)]
pub struct MisclickParticle {
    pub col: u16,
    pub row: u16,
    pub life: u32,
}

/// Convert an absolute `(col, row)` screen point into biscuit-fractional
/// coordinates, clamped to [0.0, 1.0]. Used at click/spawn sites that come
/// from screen-space input (mouse clicks, RNG within the biscuit rect).
pub fn screen_to_biscuit_frac(col: u16, row: u16, biscuit: Rect) -> (f32, f32) {
    if biscuit.width == 0 || biscuit.height == 0 {
        return (0.5, 0.5);
    }
    let fx = ((col as i32 - biscuit.x as i32) as f32) / biscuit.width as f32;
    let fy = ((row as i32 - biscuit.y as i32) as f32) / biscuit.height as f32;
    (fx.clamp(0.0, 1.0), fy.clamp(0.0, 1.0))
}

/// Convert biscuit-fractional coordinates back to an absolute screen point.
pub fn biscuit_frac_to_screen(frac_x: f32, frac_y: f32, biscuit: Rect) -> (u16, u16) {
    let col = biscuit.x as f32 + frac_x.clamp(0.0, 1.0) * biscuit.width as f32;
    let row = biscuit.y as f32 + frac_y.clamp(0.0, 1.0) * biscuit.height as f32;
    (
        col.round().clamp(0.0, u16::MAX as f32) as u16,
        row.round().clamp(0.0, u16::MAX as f32) as u16,
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Buff {
    ClickFrenzy {
        ticks_remaining: u32,
        initial_ticks: u32,
        mult: f64,
    },
    /// Fingerer mult buff — applied when a "Buff" golden variant is caught.
    /// Targets a fingerer by stable id so the buff stays on the right tier
    /// across catalog changes.
    FingererBoost {
        ticks_remaining: u32,
        initial_ticks: u32,
        fingerer_id: String,
        mult: f64,
    },
}

impl Buff {
    pub fn ticks_remaining(&self) -> u32 {
        match self {
            Buff::ClickFrenzy {
                ticks_remaining, ..
            } => *ticks_remaining,
            Buff::FingererBoost {
                ticks_remaining, ..
            } => *ticks_remaining,
        }
    }

    /// Plateau-at-1.0 until the last `BUFF_FADE_TICKS` of the buff, then
    /// smoothstep-decay to 0. Gives a "stays on, then swift but smooth fade"
    /// feel rather than a constantly-shrinking linear ramp.
    pub fn strength(&self) -> f32 {
        const FADE_TICKS: f32 = 30.0; // ~1.5s at 20Hz
        let remaining = self.ticks_remaining() as f32;
        if remaining >= FADE_TICKS {
            1.0
        } else {
            let t = (remaining / FADE_TICKS).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
    }

    fn tick(&mut self) {
        match self {
            Buff::ClickFrenzy {
                ticks_remaining, ..
            } => {
                *ticks_remaining = ticks_remaining.saturating_sub(1);
            }
            Buff::FingererBoost {
                ticks_remaining, ..
            } => {
                *ticks_remaining = ticks_remaining.saturating_sub(1);
            }
        }
    }
}

/// Persistent game state. Catalog-addressed state (`fingerers_owned`,
/// `upgrades_earned`, `achievements_earned`) is keyed by STABLE STRING IDS,
/// not positional indices, so reordering / inserting / removing entries in
/// `FINGERERS`, `UPGRADES`, or `ACHIEVEMENTS` never corrupts an old save.
/// Unknown ids in a save are ignored (forward-compat); missing ids default
/// to zero / absent (backward-compat).
#[derive(Clone, Serialize, Deserialize)]
pub struct GameState {
    #[serde(default)]
    pub cuques: f64,
    #[serde(default)]
    pub total_clicks: u64,
    #[serde(default)]
    pub lifetime_cuques: f64,
    #[serde(default)]
    pub best_fps: f64,
    #[serde(default)]
    pub golden_caught: u64,

    /// Fingerer id → owned count.
    #[serde(default)]
    pub fingerers_owned: HashMap<String, u32>,
    /// Set of earned achievement ids.
    #[serde(default)]
    pub achievements_earned: HashSet<String>,
    /// Set of earned upgrade ids.
    #[serde(default)]
    pub upgrades_earned: HashSet<String>,

    #[serde(default)]
    pub prestige: u64,
    #[serde(default)]
    pub total_play_ticks: u64,
    #[serde(default)]
    pub buffs: Vec<Buff>,

    #[serde(skip)]
    pub clench_ticks: u32,
    #[serde(skip)]
    pub particles: Vec<Particle>,
    /// Screen-anchored "misclick" tap particles — independent buffer because
    /// they don't follow the biscuit (they're feedback for clicks that
    /// MISSED the biscuit, including the dead zone at low zoom).
    #[serde(skip)]
    pub misclick_particles: Vec<MisclickParticle>,
    #[serde(skip)]
    pub golden: Option<GoldenCuque>,
    #[serde(skip)]
    pub golden_cooldown: u32,
    #[serde(skip)]
    pub session_ticks: u64,
    /// Queue of achievement ids that unlocked but haven't yet been shown as a
    /// toast. Drained one-at-a-time by `tick()` into `active_unlock_id`.
    #[serde(skip)]
    pub newly_unlocked: Vec<String>,
    /// Currently-on-screen achievement toast (id) and its remaining life in
    /// ticks. `None` means no toast right now; `tick()` pops the next pending
    /// id off `newly_unlocked` when this clears.
    #[serde(skip)]
    pub active_unlock_id: Option<String>,
    #[serde(skip)]
    pub active_unlock_ticks: u32,
    #[serde(skip)]
    pub visual_debt: f64,
    #[serde(skip)]
    pub lucky_flash_ticks: u32,
    #[serde(skip)]
    pub achievement_flash_ticks: u32,
    #[serde(skip)]
    pub border_phase: u32,
    #[serde(skip)]
    pub purchase_flash_ticks: u32,
    /// Strength multiplier (1.0..=3.0) for the most recent purchase flash,
    /// scaled by bulk-buy quantity. The border + panel borders read this so
    /// a max-buy lands harder than a single click.
    #[serde(skip)]
    pub purchase_flash_strength: f32,
    /// One slot per visible sidebar row; indexed by catalog position because
    /// it's purely a render-time flash and doesn't need to survive reorders.
    #[serde(skip)]
    pub fingerer_flash_ticks: Vec<u32>,
    /// Mirror of `fingerer_flash_ticks` for the Upgrades panel. Sized to
    /// UPGRADES.len() lazily by `migrate()`.
    #[serde(skip)]
    pub upgrade_flash_ticks: Vec<u32>,
    /// Negative-feedback flash: red row pulse when a click hit a row but
    /// `cuques < cost`. One slot per fingerer / upgrade index.
    #[serde(skip)]
    pub fingerer_unaffordable_flash: Vec<u32>,
    #[serde(skip)]
    pub upgrade_unaffordable_flash: Vec<u32>,
    /// HUD count-up tween: rendered numbers smoothly chase the real ones.
    /// Initialized to the live values on load so the first frame doesn't
    /// look like a count-up from zero.
    #[serde(skip)]
    pub displayed_cuques: f64,
    #[serde(skip)]
    pub displayed_fps: f64,
    /// Brief green flash on the HUD digits when a big jump lands (golden,
    /// purchase, dev cheat).
    #[serde(skip)]
    pub cuques_flash_ticks: u32,
}

pub const LUCKY_FLASH_TICKS: u32 = 70; // 3.5s at 20Hz
pub const PURCHASE_FLASH_TICKS: u32 = 20; // 1s at 20Hz

impl Default for GameState {
    fn default() -> Self {
        Self {
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            fingerers_owned: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: Vec::new(),
            clench_ticks: 0,
            particles: Vec::new(),
            misclick_particles: Vec::new(),
            golden: None,
            golden_cooldown: crate::game::golden::next_cooldown(),
            session_ticks: 0,
            newly_unlocked: Vec::new(),
            active_unlock_id: None,
            active_unlock_ticks: 0,
            visual_debt: 0.0,
            lucky_flash_ticks: 0,
            achievement_flash_ticks: 0,
            border_phase: 0,
            purchase_flash_ticks: 0,
            purchase_flash_strength: 1.0,
            fingerer_flash_ticks: vec![0; fingerer::count()],
            upgrade_flash_ticks: vec![0; UPGRADES.len()],
            fingerer_unaffordable_flash: vec![0; fingerer::count()],
            upgrade_unaffordable_flash: vec![0; UPGRADES.len()],
            displayed_cuques: 0.0,
            displayed_fps: 0.0,
            cuques_flash_ticks: 0,
        }
    }
}

impl GameState {
    /// Initialize ephemeral runtime state that `#[serde(skip)]` left empty
    /// after deserialization, and normalize any fields that need live values
    /// rather than the serde default. Hook point for future shape
    /// transforms — see CLAUDE.md on the stable-id save policy.
    pub fn migrate(mut self) -> Self {
        // Per-catalog flash slots are runtime-only — re-size if the catalog
        // grew/shrank since this save was written.
        if self.fingerer_flash_ticks.len() != fingerer::count() {
            self.fingerer_flash_ticks = vec![0; fingerer::count()];
        }
        if self.upgrade_flash_ticks.len() != UPGRADES.len() {
            self.upgrade_flash_ticks = vec![0; UPGRADES.len()];
        }
        if self.fingerer_unaffordable_flash.len() != fingerer::count() {
            self.fingerer_unaffordable_flash = vec![0; fingerer::count()];
        }
        if self.upgrade_unaffordable_flash.len() != UPGRADES.len() {
            self.upgrade_unaffordable_flash = vec![0; UPGRADES.len()];
        }
        if self.golden_cooldown == 0 {
            self.golden_cooldown = crate::game::golden::next_cooldown();
        }
        // Seed the count-up tween at the live values so a freshly-loaded save
        // doesn't animate the HUD "from 0" up to whatever the player had.
        self.displayed_cuques = self.cuques;
        self.displayed_fps = 0.0; // recomputed on first tick
        if self.purchase_flash_strength <= 0.0 {
            self.purchase_flash_strength = 1.0;
        }
        self
    }

    // -- Catalog lookups (stable-id keyed) ---------------------------------

    pub fn fingerer_count(&self, id: &str) -> u32 {
        self.fingerers_owned.get(id).copied().unwrap_or(0)
    }

    pub fn fingerer_count_idx(&self, idx: usize) -> u32 {
        FINGERERS
            .get(idx)
            .map(|f| self.fingerer_count(f.id))
            .unwrap_or(0)
    }

    pub fn fingerers_owned_total(&self) -> u32 {
        self.fingerers_owned.values().sum()
    }

    pub fn has_upgrade(&self, id: &str) -> bool {
        self.upgrades_earned.contains(id)
    }

    pub fn has_achievement(&self, id: &str) -> bool {
        self.achievements_earned.contains(id)
    }

    pub fn has_achievement_idx(&self, idx: usize) -> bool {
        ACHIEVEMENTS
            .get(idx)
            .is_some_and(|a| self.has_achievement(a.id))
    }

    // -- Click / tick -------------------------------------------------------

    pub fn click(&mut self, origin: (u16, u16), biscuit: Rect) {
        let power = self.click_power();
        self.add_cuques(power);
        self.total_clicks += 1;
        self.clench_ticks = CLENCH_TICKS;
        // Click that meaningfully grows the counter also flashes the HUD
        // digits — a single +1 doesn't deserve the green tint, but a
        // Frenzy +777 (or any bulk jump) does.
        if power >= 50.0 {
            self.cuques_flash_ticks = HUD_FLASH_TICKS;
        }
        let mut rng = rand::rng();
        // Wider random horizontal jitter (proportional to biscuit width) plus
        // a small Y jitter so co-spawned particles don't overlap into "+1+1+1"
        // mush at the same row. Per-particle drift_x continues the spread
        // over the particle's life.
        let jitter_x_range = (biscuit.width as i32 / 8).max(3);
        let jitter_x = rng.random_range(-jitter_x_range..=jitter_x_range);
        let jitter_y = rng.random_range(-1..=1);
        let col = (origin.0 as i32 + jitter_x).max(0) as u16;
        let row = origin
            .1
            .saturating_sub(1)
            .saturating_add_signed(jitter_y as i16);
        let (frac_x, frac_y) = screen_to_biscuit_frac(col, row, biscuit);
        let drift_x = rng.random_range(-0.012_f32..=0.012);
        let frenzy_active = self
            .buffs
            .iter()
            .any(|b| matches!(b, Buff::ClickFrenzy { .. }));
        // Small numbers stay subtle; big ones (Frenzy, Cosmic mults) get a
        // bold ClickBig style so they read as "this matters" against the
        // chatter of auto-particles.
        let kind = if power >= 50.0 || frenzy_active {
            ParticleKind::ClickBig
        } else {
            ParticleKind::Click
        };
        self.particles.push(Particle {
            frac_x,
            frac_y,
            life: PARTICLE_LIFE,
            text: format!("+{}", crate::format::big(power)),
            kind,
            drift_x,
        });
        // Frenzy clicks also spawn a halo of `*` confetti to make every tap
        // feel chaotic without altering game behavior.
        if frenzy_active {
            for _ in 0..2 {
                let halo_x = rng.random_range(-0.05_f32..=0.05);
                let halo_y = rng.random_range(-0.04_f32..=0.04);
                let (hfx, hfy) =
                    screen_to_biscuit_frac(origin.0, origin.1.saturating_sub(1), biscuit);
                self.particles.push(Particle {
                    frac_x: (hfx + halo_x).clamp(0.0, 1.0),
                    frac_y: (hfy + halo_y).clamp(0.0, 1.0),
                    life: PARTICLE_LIFE / 2,
                    text: "*".into(),
                    kind: ParticleKind::Confetti,
                    drift_x: rng.random_range(-0.02_f32..=0.02),
                });
            }
        }
    }

    /// Spawn a screen-anchored "·" particle at a click point that hit nothing
    /// (biscuit dead zone, blank panel area, etc). Acknowledges that the
    /// click registered without altering any game state.
    pub fn spawn_misclick(&mut self, col: u16, row: u16) {
        // Cap to avoid unbounded buildup if a player rage-clicks empty space.
        if self.misclick_particles.len() >= 16 {
            self.misclick_particles.remove(0);
        }
        self.misclick_particles.push(MisclickParticle {
            col,
            row,
            life: MISCLICK_LIFE,
        });
    }

    /// Spawn `n` confetti particles scattered over the biscuit. Used for
    /// bulk-buy juice — a max-buy of a fingerer pops a small burst.
    pub fn spawn_confetti(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let mut rng = rand::rng();
        let glyphs = ['*', '+', '~', '.', 'o'];
        for _ in 0..n.min(8) {
            let glyph = glyphs[rng.random_range(0..glyphs.len())];
            self.particles.push(Particle {
                frac_x: rng.random_range(0.10_f32..=0.90),
                frac_y: rng.random_range(0.20_f32..=0.85),
                life: PARTICLE_LIFE,
                text: glyph.to_string(),
                kind: ParticleKind::Confetti,
                drift_x: rng.random_range(-0.02_f32..=0.02),
            });
        }
    }

    pub fn click_power(&self) -> f64 {
        let mut m = 1.0;
        for u in UPGRADES.iter() {
            if self.has_upgrade(u.id)
                && let UpgradeEffect::ClickMult(f) = u.effect
            {
                m *= f;
            }
        }
        for b in &self.buffs {
            if let Buff::ClickFrenzy { mult, .. } = b {
                m *= *mult;
            }
        }
        m
    }

    pub fn fingerer_mult(&self, idx: usize) -> f64 {
        let Some(target) = FINGERERS.get(idx) else {
            return 1.0;
        };
        let mut m = 1.0;
        for u in UPGRADES.iter() {
            if !self.has_upgrade(u.id) {
                continue;
            }
            match u.effect {
                UpgradeEffect::FingererMult(id, f) if id == target.id => m *= f,
                UpgradeEffect::AllFingerersMult(f) => m *= f,
                _ => {}
            }
        }
        for b in &self.buffs {
            if let Buff::FingererBoost {
                fingerer_id, mult, ..
            } = b
                && fingerer_id == target.id
            {
                m *= *mult;
            }
        }
        m
    }

    fn add_cuques(&mut self, amount: f64) {
        self.cuques += amount;
        self.lifetime_cuques += amount;
    }

    /// Dev-build cheat. Bypasses normal flow; not reachable in release builds
    /// because the F-key that triggers it is gated behind `App::debug`.
    pub fn dev_add_cuques(&mut self, amount: f64) {
        self.add_cuques(amount);
        self.cuques_flash_ticks = HUD_FLASH_TICKS;
    }

    /// Catch whatever Golden Cuque is currently on screen (any variant:
    /// Lucky, Frenzy, or Buff). Applies the variant-specific effect,
    /// increments `golden_caught`, re-rolls the next spawn cooldown, and
    /// returns the flat reward (0.0 for buff variants).
    pub fn catch_golden(&mut self) -> f64 {
        use crate::game::golden::GoldenVariant;
        let Some(golden) = self.golden.take() else {
            return 0.0;
        };
        self.golden_caught += 1;
        self.golden_cooldown = crate::game::golden::next_cooldown();
        let (reward, label) = match golden.variant {
            GoldenVariant::Lucky => {
                let fps = self.fps();
                let r = (fps * GOLDEN_REWARD_SECONDS).max(GOLDEN_REWARD_FLAT);
                self.add_cuques(r);
                self.lucky_flash_ticks = LUCKY_FLASH_TICKS;
                self.cuques_flash_ticks = HUD_FLASH_TICKS;
                (r, format!("+{}", crate::format::big(r)))
            }
            GoldenVariant::Frenzy => {
                let dur = TICK_HZ * 13;
                self.buffs.push(Buff::ClickFrenzy {
                    ticks_remaining: dur,
                    initial_ticks: dur,
                    mult: 777.0,
                });
                (0.0, "FRENZY x777!".into())
            }
            GoldenVariant::Buff => {
                let active_ids: Vec<&'static str> = FINGERERS
                    .iter()
                    .filter(|f| self.fingerer_count(f.id) > 0)
                    .map(|f| f.id)
                    .collect();
                let pick = if active_ids.is_empty() {
                    FINGERERS[0].id
                } else {
                    use rand::RngExt;
                    active_ids[rand::rng().random_range(0..active_ids.len())]
                };
                let dur = TICK_HZ * 60;
                self.buffs.push(Buff::FingererBoost {
                    ticks_remaining: dur,
                    initial_ticks: dur,
                    fingerer_id: pick.to_string(),
                    mult: 7.0,
                });
                (0.0, "BOOSTED x7!".into())
            }
        };
        self.particles.push(Particle {
            frac_x: golden.frac_x,
            frac_y: golden.frac_y,
            life: PARTICLE_LIFE * 2,
            text: label,
            kind: ParticleKind::Golden,
            drift_x: 0.0,
        });
        reward
    }

    pub fn fps(&self) -> f64 {
        let base: f64 = FINGERERS
            .iter()
            .enumerate()
            .map(|(i, k)| k.fps_per_unit * self.fingerer_count(k.id) as f64 * self.fingerer_mult(i))
            .sum();
        base * self.prestige_mult()
    }

    pub fn border_speed(&self) -> u32 {
        let mut s: u32 = 1;
        for b in &self.buffs {
            match b {
                Buff::ClickFrenzy { .. } => s = s.max(3),
                Buff::FingererBoost { .. } => s = s.max(2),
            }
        }
        if self.lucky_flash_ticks > 0 {
            s = s.max(4);
        }
        if self.achievement_flash_ticks > 0 {
            s = s.max(3);
        }
        if self.purchase_flash_ticks > 0 {
            s += 2;
        }
        s
    }

    /// Trigger the green purchase flash on the global border + the panel
    /// border. `strength` scales how loud the flash is (1.0 = single buy,
    /// up to 3.0 = bulk max-buy) so a max-buy lands harder than a +1.
    pub fn trigger_purchase_flash(&mut self, strength: f32) {
        self.purchase_flash_ticks = PURCHASE_FLASH_TICKS;
        // Take the louder of the in-flight strength and the new event so
        // back-to-back small buys don't squash a still-decaying loud one.
        self.purchase_flash_strength = self.purchase_flash_strength.max(strength).clamp(1.0, 3.0);
    }

    pub fn prestige_mult(&self) -> f64 {
        1.0 + 0.01 * self.prestige as f64
    }

    pub fn prestige_earned_total(&self) -> u64 {
        (self.lifetime_cuques / 1_000_000.0).sqrt().floor() as u64
    }

    pub fn prestige_available(&self) -> u64 {
        self.prestige_earned_total().saturating_sub(self.prestige)
    }

    pub fn prestige_reset(&mut self) -> bool {
        let available = self.prestige_available();
        if available == 0 {
            return false;
        }
        self.prestige = self.prestige_earned_total();
        self.cuques = 0.0;
        self.displayed_cuques = 0.0;
        self.displayed_fps = 0.0;
        self.fingerers_owned.clear();
        self.upgrades_earned.clear();
        self.buffs.clear();
        self.visual_debt = 0.0;
        self.particles.clear();
        self.misclick_particles.clear();
        self.golden = None;
        self.clench_ticks = 0;
        self.golden_cooldown = crate::game::golden::next_cooldown();
        true
    }

    pub fn tick(&mut self) {
        for b in self.buffs.iter_mut() {
            b.tick();
        }
        self.buffs.retain(|b| b.ticks_remaining() > 0);

        self.lucky_flash_ticks = self.lucky_flash_ticks.saturating_sub(1);
        self.achievement_flash_ticks = self.achievement_flash_ticks.saturating_sub(1);
        self.purchase_flash_ticks = self.purchase_flash_ticks.saturating_sub(1);
        if self.purchase_flash_ticks == 0 {
            self.purchase_flash_strength = 1.0;
        }
        self.cuques_flash_ticks = self.cuques_flash_ticks.saturating_sub(1);
        for t in self.fingerer_flash_ticks.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.upgrade_flash_ticks.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.fingerer_unaffordable_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.upgrade_unaffordable_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        let speed = self.border_speed();
        self.border_phase = self.border_phase.wrapping_add(speed);

        let fps = self.fps();
        if fps > self.best_fps {
            self.best_fps = fps;
        }
        let gained = fps * TICK_DT;
        self.add_cuques(gained);
        self.visual_debt += gained;
        self.clench_ticks = self.clench_ticks.saturating_sub(1);
        for p in self.particles.iter_mut() {
            p.life = p.life.saturating_sub(1);
            p.frac_y -= PARTICLE_FRAC_RISE;
            // Per-particle horizontal drift so co-spawned particles spread
            // out over their lifetime instead of overlapping into garbage.
            p.frac_x = (p.frac_x + p.drift_x).clamp(0.0, 1.0);
        }
        self.particles.retain(|p| p.life > 0);
        for m in self.misclick_particles.iter_mut() {
            m.life = m.life.saturating_sub(1);
        }
        self.misclick_particles.retain(|m| m.life > 0);

        // Count-up tween: rendered numbers chase the real ones with
        // ease-out. Big jumps (golden, F4, max-buy) take ~10-12 frames
        // to land instead of snapping, so the player's eyes track them.
        // Tiny per-tick auto income gets caught instantly, so the HUD
        // never lags noticeably during normal play.
        let tween = 0.18_f64;
        let max_lag = 1e-3;
        let dc = self.cuques - self.displayed_cuques;
        if dc.abs() < max_lag {
            self.displayed_cuques = self.cuques;
        } else {
            self.displayed_cuques += dc * tween;
        }
        let df = fps - self.displayed_fps;
        if df.abs() < max_lag {
            self.displayed_fps = fps;
        } else {
            self.displayed_fps += df * tween;
        }

        self.session_ticks += 1;
        self.total_play_ticks += 1;
        // Run the achievement check *before* the toast popper so an unlock
        // detected this tick can become the on-screen toast on the same
        // tick. Otherwise we'd waste the first tick of the toast's life
        // moving the unlock from the queue to active_unlock_id.
        self.tick_achievements();

        // Toast queue: when no toast is on screen, pop the next pending
        // unlock id and schedule it for TOAST_TICKS. Every other tick
        // the active toast just decays.
        self.active_unlock_ticks = self.active_unlock_ticks.saturating_sub(1);
        if self.active_unlock_ticks == 0 {
            self.active_unlock_id = None;
            if !self.newly_unlocked.is_empty() {
                self.active_unlock_id = Some(self.newly_unlocked.remove(0));
                self.active_unlock_ticks = TOAST_TICKS;
                self.achievement_flash_ticks = ACHIEVEMENT_FLASH_TICKS;
            }
        }
    }

    pub fn tick_achievements(&mut self) {
        for a in ACHIEVEMENTS.iter() {
            if !self.has_achievement(a.id) && (a.unlocked)(self) {
                self.achievements_earned.insert(a.id.to_string());
                self.newly_unlocked.push(a.id.to_string());
            }
        }
    }

    pub fn tick_golden(&mut self) {
        if let Some(g) = self.golden.as_mut() {
            if g.life_ticks == 0 {
                self.golden = None;
                self.golden_cooldown = crate::game::golden::next_cooldown();
            } else {
                g.life_ticks -= 1;
            }
        } else if self.golden_cooldown > 0 {
            self.golden_cooldown -= 1;
        }
    }

    pub fn trigger_clench(&mut self) {
        self.clench_ticks = CLENCH_TICKS;
    }

    /// Spawn a "+N" particle representing cuques earned since the last
    /// auto-particle. Silently skips if there isn't a whole cuque of accrued
    /// income to show — at low FPS the caller is a rate-based timer that
    /// fires faster than cuques arrive, and spawning a "+1" in that window
    /// used to lie (particle flying up while the HUD counter didn't move).
    /// The shown amount is always real cuques that just accrued into
    /// `visual_debt`.
    pub fn spawn_auto_particle(&mut self, frac_x: f32, frac_y: f32) {
        let amount = self.visual_debt.floor() as u64;
        if amount == 0 {
            return;
        }
        self.visual_debt -= amount as f64;
        let drift_x = rand::rng().random_range(-0.008_f32..=0.008);
        self.particles.push(Particle {
            frac_x,
            frac_y,
            life: PARTICLE_LIFE,
            text: format!("+{}", crate::format::big(amount as f64)),
            kind: ParticleKind::Auto,
            drift_x,
        });
    }

    pub fn cost(&self, idx: usize) -> f64 {
        let k = &FINGERERS[idx];
        k.base_cost * k.cost_scale.powi(self.fingerer_count_idx(idx) as i32)
    }

    pub fn can_buy(&self, idx: usize) -> bool {
        self.cuques >= self.cost(idx)
    }

    /// Buy a single unit. Bare mutation only — flash side-effects are
    /// scaled by quantity in `buy_n` / `buy_max` so a single buy and a
    /// bulk buy produce visually distinct feedback.
    fn buy_one_quiet(&mut self, idx: usize) -> bool {
        let c = self.cost(idx);
        if self.cuques >= c
            && let Some(f) = FINGERERS.get(idx)
        {
            self.cuques -= c;
            *self.fingerers_owned.entry(f.id.to_string()).or_insert(0) += 1;
            true
        } else {
            false
        }
    }

    /// Apply purchase flash + per-row green flash, then optionally pop
    /// confetti. Called once per public buy action with the total bought
    /// count, so the loud bulk-buy feedback only fires once.
    fn flash_purchase(&mut self, idx: usize, bought: u32, slot_table: PurchaseSlot) {
        if bought == 0 {
            return;
        }
        // 1 → 1.0, 10 → 1.7, 50 → 2.5, capped at 3.0. sqrt-style growth so
        // a max-buy is dramatic but doesn't blow the eardrums.
        let strength = (1.0 + ((bought as f32) / 10.0).sqrt()).clamp(1.0, 3.0);
        self.trigger_purchase_flash(strength);
        match slot_table {
            PurchaseSlot::Fingerer => {
                if let Some(slot) = self.fingerer_flash_ticks.get_mut(idx) {
                    *slot = PURCHASE_FLASH_TICKS;
                }
            }
            PurchaseSlot::Upgrade => {
                if let Some(slot) = self.upgrade_flash_ticks.get_mut(idx) {
                    *slot = PURCHASE_FLASH_TICKS;
                }
            }
        }
        if bought >= 10 {
            // Big jumps are worth a HUD pop too.
            self.cuques_flash_ticks = HUD_FLASH_TICKS;
        }
        if bought >= 5 {
            self.spawn_confetti(bought.min(8));
        }
    }

    fn flash_unaffordable_fingerer(&mut self, idx: usize) {
        if let Some(slot) = self.fingerer_unaffordable_flash.get_mut(idx) {
            *slot = PURCHASE_FLASH_TICKS / 2;
        }
    }

    fn flash_unaffordable_upgrade(&mut self, idx: usize) {
        if let Some(slot) = self.upgrade_unaffordable_flash.get_mut(idx) {
            *slot = PURCHASE_FLASH_TICKS / 2;
        }
    }

    pub fn buy(&mut self, idx: usize) -> bool {
        if self.buy_one_quiet(idx) {
            self.flash_purchase(idx, 1, PurchaseSlot::Fingerer);
            true
        } else {
            self.flash_unaffordable_fingerer(idx);
            false
        }
    }

    pub fn buy_n(&mut self, idx: usize, n: u32) -> u32 {
        let mut bought = 0;
        for _ in 0..n {
            if !self.buy_one_quiet(idx) {
                break;
            }
            bought += 1;
        }
        if bought == 0 {
            self.flash_unaffordable_fingerer(idx);
        } else {
            self.flash_purchase(idx, bought, PurchaseSlot::Fingerer);
        }
        bought
    }

    pub fn buy_max(&mut self, idx: usize) -> u32 {
        let mut bought = 0;
        while self.buy_one_quiet(idx) {
            bought += 1;
        }
        if bought == 0 {
            self.flash_unaffordable_fingerer(idx);
        } else {
            self.flash_purchase(idx, bought, PurchaseSlot::Fingerer);
        }
        bought
    }

    pub fn buy_upgrade(&mut self, idx: usize) -> bool {
        let Some(u) = UPGRADES.get(idx) else {
            return false;
        };
        if self.has_upgrade(u.id) {
            return false;
        }
        if !u.req.met(self) || self.cuques < u.cost {
            self.flash_unaffordable_upgrade(idx);
            return false;
        }
        self.cuques -= u.cost;
        self.upgrades_earned.insert(u.id.to_string());
        self.flash_purchase(idx, 1, PurchaseSlot::Upgrade);
        true
    }
}

#[derive(Clone, Copy)]
enum PurchaseSlot {
    Fingerer,
    Upgrade,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent_on_current_shape() {
        let state = GameState {
            fingerers_owned: [("index_finger".to_string(), 9)].into_iter().collect(),
            upgrades_earned: ["click_mult_1".to_string()].into_iter().collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };

        let m = state.migrate();

        assert_eq!(m.fingerer_count("index_finger"), 9);
        assert!(m.has_upgrade("click_mult_1"));
        assert!(m.has_achievement("first_finger"));
    }

    #[test]
    fn unknown_ids_in_save_are_ignored_not_resurrected() {
        // Forward-compat: a future version adds `"giga_finger"` to the
        // catalog, player plays, saves. User downgrades to current version.
        // That unknown id must not crash — it just reads as 0.
        let state = GameState {
            fingerers_owned: [("giga_finger_from_the_future".to_string(), 42)]
                .into_iter()
                .collect(),
            ..GameState::default()
        };

        let m = state.migrate();

        assert_eq!(m.fingerer_count("giga_finger_from_the_future"), 42);
        assert_eq!(m.fingerer_count("index_finger"), 0);
        assert!(!m.has_upgrade("click_mult_1"));
    }

    #[test]
    fn save_roundtrip_is_stable_through_json() {
        // Serialize → deserialize → get the same state back. Catches any
        // accidental rename that would make saves non-idempotent.
        let state = GameState {
            cuques: 1234.5,
            total_clicks: 99,
            fingerers_owned: [("index_finger".to_string(), 7)].into_iter().collect(),
            upgrades_earned: ["click_mult_1".to_string()].into_iter().collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };

        let json = serde_json::to_string(&state).expect("serialize");
        let roundtripped: GameState = serde_json::from_str(&json).expect("deserialize");
        let m = roundtripped.migrate();

        assert_eq!(m.cuques, 1234.5);
        assert_eq!(m.total_clicks, 99);
        assert_eq!(m.fingerer_count("index_finger"), 7);
        assert!(m.has_upgrade("click_mult_1"));
        assert!(m.has_achievement("first_finger"));
    }

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn frac_screen_roundtrip_at_corners() {
        let biscuit = r(10, 5, 40, 20);
        // top-left corner
        let (fx, fy) = screen_to_biscuit_frac(10, 5, biscuit);
        assert!(fx <= 0.001 && fy <= 0.001);
        let (col, row) = biscuit_frac_to_screen(fx, fy, biscuit);
        assert_eq!((col, row), (10, 5));

        // bottom-right (one beyond, clamps)
        let (fx, fy) = screen_to_biscuit_frac(50, 25, biscuit);
        assert!(fx >= 0.999 && fy >= 0.999);

        // exact center
        let (col, row) = biscuit_frac_to_screen(0.5, 0.5, biscuit);
        assert_eq!(col, 30);
        assert_eq!(row, 15);
    }

    #[test]
    fn frac_position_survives_biscuit_move() {
        // A point at fraction (0.25, 0.5) of the biscuit must resolve to a
        // proportionally-shifted absolute coord when the biscuit moves /
        // grows.
        let small = r(0, 0, 40, 20);
        let (col_a, row_a) = biscuit_frac_to_screen(0.25, 0.5, small);
        let large = r(10, 5, 80, 40);
        let (col_b, row_b) = biscuit_frac_to_screen(0.25, 0.5, large);
        // Same fractional spot, very different screen coords.
        assert_ne!((col_a, row_a), (col_b, row_b));
        // And the shifted point should still sit at the 25%/50% mark of the
        // new rect.
        assert_eq!(col_b, 30); // 10 + 0.25 * 80
        assert_eq!(row_b, 25); // 5  + 0.5  * 40
    }

    #[test]
    fn zero_size_biscuit_doesnt_panic() {
        let zero = r(0, 0, 0, 0);
        let (fx, fy) = screen_to_biscuit_frac(5, 5, zero);
        assert_eq!((fx, fy), (0.5, 0.5));
        let (col, row) = biscuit_frac_to_screen(0.5, 0.5, zero);
        assert_eq!((col, row), (0, 0));
    }

    // -- Juice-flash invariants ---------------------------------------------

    #[test]
    fn buy_when_broke_sets_unaffordable_flash() {
        // Player clicks an unaffordable fingerer row → buy() returns false
        // AND a red row flash is queued so the rejection is visible. This
        // is the J11 contract; without it the click looks silent.
        let mut s = GameState::default();
        s.cuques = 0.0;
        let bought = s.buy(0);
        assert!(!bought);
        assert!(
            s.fingerer_unaffordable_flash[0] > 0,
            "buy(0) on broke state must flash red"
        );
        assert!(
            s.fingerer_flash_ticks[0] == 0,
            "no purchase flash on reject"
        );
    }

    #[test]
    fn buy_n_when_broke_sets_unaffordable_flash() {
        let mut s = GameState::default();
        s.cuques = 0.0;
        let bought = s.buy_n(0, 10);
        assert_eq!(bought, 0);
        assert!(s.fingerer_unaffordable_flash[0] > 0);
    }

    #[test]
    fn bulk_buy_scales_purchase_flash_strength() {
        // J8: max-buy is louder than a +1. We don't pin exact values (clamp
        // boundaries are tuning), only the relative ordering and bounds.
        let mut s = GameState {
            cuques: 1_000_000.0,
            ..Default::default()
        };
        s.buy(0);
        let single = s.purchase_flash_strength;
        assert!((1.0..=3.0).contains(&single));

        let mut s = GameState {
            cuques: 1_000_000.0,
            ..Default::default()
        };
        s.buy_n(0, 50);
        let bulk = s.purchase_flash_strength;
        assert!(
            bulk > single,
            "bulk strength must exceed single ({bulk} vs {single})"
        );
        assert!(bulk <= 3.0, "bulk strength capped at 3.0");
    }

    #[test]
    fn buy_upgrade_when_broke_sets_unaffordable_flash() {
        let mut s = GameState::default();
        // Pick the cheapest upgrade and try to buy with no money.
        let cheapest_idx = (0..UPGRADES.len())
            .min_by(|&a, &b| UPGRADES[a].cost.partial_cmp(&UPGRADES[b].cost).unwrap())
            .unwrap();
        let bought = s.buy_upgrade(cheapest_idx);
        assert!(!bought);
        assert!(s.upgrade_unaffordable_flash[cheapest_idx] > 0);
    }

    #[test]
    fn migrate_resizes_per_catalog_flash_vecs() {
        // A serialized state from "before this branch shipped" has empty /
        // skipped flash vecs after deserialize. migrate() must size them to
        // the live catalog so paint paths can index without bounds checks
        // in hot loops.
        let json = serde_json::to_string(&GameState::default()).unwrap();
        let mut s: GameState = serde_json::from_str(&json).unwrap();
        // Simulate stale shape: drop the per-catalog vecs.
        s.fingerer_flash_ticks.clear();
        s.upgrade_flash_ticks.clear();
        s.fingerer_unaffordable_flash.clear();
        s.upgrade_unaffordable_flash.clear();
        let m = s.migrate();
        assert_eq!(m.fingerer_flash_ticks.len(), fingerer::count());
        assert_eq!(m.upgrade_flash_ticks.len(), UPGRADES.len());
        assert_eq!(m.fingerer_unaffordable_flash.len(), fingerer::count());
        assert_eq!(m.upgrade_unaffordable_flash.len(), UPGRADES.len());
    }

    #[test]
    fn migrate_seeds_displayed_counters() {
        // J5 contract: a freshly-loaded save shows the live counters at full
        // value, not "tweening up from zero".
        let s = GameState {
            cuques: 5_000.0,
            ..Default::default()
        };
        let m = s.migrate();
        assert_eq!(m.displayed_cuques, 5_000.0);
        // displayed_fps starts at 0 and converges over the first few ticks
        // (otherwise we'd snap-show the FPS before any tick has run).
        assert_eq!(m.displayed_fps, 0.0);
    }

    #[test]
    fn unlock_pop_sets_active_toast_and_gold_flash() {
        // J1 contract: when an achievement triggers, tick() drains
        // newly_unlocked into active_unlock_id and lights the gold border
        // channel.
        let mut s = GameState::default();
        // Force a "First Finger" unlock by simulating one click.
        let biscuit = r(0, 0, 40, 20);
        s.click((20, 10), biscuit);
        s.tick();
        // The fresh tick should have moved the queued unlock onto the screen.
        assert!(s.active_unlock_id.is_some());
        assert!(s.active_unlock_ticks > 0);
        assert!(s.achievement_flash_ticks > 0);
    }
}
