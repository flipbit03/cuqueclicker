use std::collections::{HashMap, HashSet};

use rand::RngExt;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::game::achievement::ACHIEVEMENTS;
use crate::game::fingerer::{self, FINGERERS};
use crate::game::golden::GoldenCuque;
use crate::game::green_coin::GreenCoin;
use crate::game::modifier::{
    FingererAggregate, Modifier, ModifierDuration, ModifierEffect, ModifierSource,
};
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
/// "You can afford this now!" row flash — fires the moment a fingerer or
/// upgrade transitions from unaffordable to affordable. Brief on purpose:
/// short enough that it's clearly an "announcement," not the longer
/// purchase flash that fires on actual buy.
pub const UNLOCK_FLASH_TICKS: u32 = TICK_HZ / 2; // 0.5s
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

/// Global, click-side buffs. Per-fingerer multipliers (the old
/// `Buff::FingererBoost`) live on the modifier system in
/// `crate::game::modifier`; only buffs that affect global click power
/// belong here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Buff {
    ClickFrenzy {
        ticks_remaining: u32,
        initial_ticks: u32,
        mult: f64,
    },
}

impl Buff {
    pub fn ticks_remaining(&self) -> u32 {
        match self {
            Buff::ClickFrenzy {
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
        }
    }
}

/// Per-fingerer persistent state.
///
/// `count` is the number of units the player owns. `modifiers` is the list
/// of [`Modifier`]s attached to this fingerer (Green Coin permanents,
/// Purple Coin temp boosts, future buffs/debuffs); see
/// [`crate::game::modifier`] for the stacking rules. `aggregate` is a
/// derived cache rebuilt from `modifiers` on add/remove/expire and on
/// save load — it's `#[serde(skip)]` because it's pure-derived data, and
/// the live state is always reconstructable from `modifiers`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FingererState {
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    /// Pre-computed aggregate of every effect across every modifier.
    /// Rebuilt by `attach_modifier` / per-tick expiry / `migrate_runtime`.
    /// FPS reads MUST consult this, not the `Vec`.
    #[serde(skip)]
    pub aggregate: FingererAggregate,
}

/// Persistent game state. Catalog-addressed state (`fingerers_state`,
/// `upgrades_earned`, `achievements_earned`) is keyed by STABLE STRING IDS,
/// not positional indices, so reordering / inserting / removing entries in
/// `FINGERERS`, `UPGRADES`, or `ACHIEVEMENTS` never corrupts an old save.
/// Unknown ids in a save are ignored (forward-compat); missing ids default
/// to zero / absent (backward-compat).
#[derive(Clone, Serialize, Deserialize)]
pub struct GameState {
    /// Save schema version. The on-disk migration chain (`crate::save`)
    /// reads this via `peek_version` *before* deserializing into the right
    /// `GameStateVN` struct. A live in-memory state always equals
    /// `crate::save::CURRENT_VERSION` — the chain stamps it on conversion
    /// and `Default` initializes it that way. Pre-versioned saves on disk
    /// have no `version` key, which `peek_version` treats as V1.
    #[serde(default = "default_save_version")]
    pub version: u32,
    #[serde(default)]
    pub cuques: f64,
    #[serde(default)]
    pub total_clicks: u64,
    #[serde(default)]
    pub lifetime_cuques: f64,
    #[serde(default)]
    pub best_fps: f64,
    /// Lifetime grand total of every powerup caught (Lucky, Frenzy, Buff,
    /// Green Coin). Stays a strict rollup so existing achievements that
    /// gate on it continue to work, and pre-V3 saves whose breakdown was
    /// never recorded keep an honest total. The four per-variant counters
    /// below were added in V3; they only count post-V3 catches.
    #[serde(default)]
    pub golden_caught: u64,
    #[serde(default)]
    pub lucky_caught: u64,
    #[serde(default)]
    pub frenzy_caught: u64,
    #[serde(default)]
    pub buff_caught: u64,
    #[serde(default)]
    pub green_coin_caught: u64,

    /// Fingerer id → owned count + attached modifiers + aggregate cache.
    #[serde(default)]
    pub fingerers_state: HashMap<String, FingererState>,
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
    /// Green Coin pity counter. Increments on every regular Golden spawn,
    /// drives a `rng < counter * 0.01` roll for an alongside Green Coin
    /// spawn, and resets the moment a Green Coin appears. Persisted so the
    /// pity timer survives quit/restart.
    #[serde(default)]
    pub goldens_since_green_coin: u32,

    #[serde(skip)]
    pub clench_ticks: u32,
    #[serde(skip)]
    pub particles: Vec<Particle>,
    /// Screen-anchored "misclick" tap particles — independent buffer because
    /// they don't follow the biscuit (they're feedback for clicks that
    /// MISSED the biscuit, including the dead zone at low zoom).
    #[serde(skip)]
    pub misclick_particles: Vec<MisclickParticle>,
    /// Per-variant Golden Cuque slots — `goldens[GoldenVariant::Lucky as usize]`
    /// holds the on-screen Lucky, etc. Each variant has its own independent
    /// slot AND its own independent cooldown so spawn timing is fully
    /// desynchronized — Lucky's clock doesn't gate Frenzy's, and a Buff
    /// expiring doesn't reset Lucky's cooldown.
    #[serde(skip)]
    pub goldens: [Option<GoldenCuque>; 3],
    /// Per-variant spawn cooldowns, indexed the same as `goldens`. Each
    /// freezes while its own slot is occupied (no point counting down
    /// when there's nowhere to spawn) and rolls a fresh value on
    /// catch/expiry. Initial values come from `next_cooldown()` in
    /// `Default`, which already randomizes them so no two variants
    /// arrive at zero simultaneously on a fresh save.
    #[serde(skip)]
    pub golden_cooldowns: [u32; 3],
    /// On-screen Green Coin, if one is currently visible. Lifetime ticked
    /// down by `tick_green_coin`; cleared on catch or expiry. Not persisted
    /// (parallel to `golden`) — closing and reopening the game shouldn't
    /// preserve a frozen coin frame.
    #[serde(skip)]
    pub green_coin: Option<GreenCoin>,
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
    /// Brief green border channel pulse fired on a Green Coin catch.
    /// Behaves like `lucky_flash_ticks` (plateau-fade); coexists with
    /// other channels so a Green Coin caught during a Frenzy or Lucky
    /// adds a green moiré rather than overwriting them.
    #[serde(skip)]
    pub green_coin_flash_ticks: u32,
    /// HUD title border phase clock. Advances by `border_speed()` each
    /// tick, so the title border visibly speeds up under Frenzy / Lucky /
    /// purchase events. INTENTIONALLY NOT shared with secondary shimmers
    /// (panel borders, sidebar / upgrade rows) — they need a constant-rate
    /// clock so a global speed-up on the HUD doesn't drag them along.
    #[serde(skip)]
    pub border_phase: u32,
    /// Constant-rate phase clock for secondary shimmers — sidebar row,
    /// upgrade row, and panel-border flashes. Advances by exactly 1 per
    /// tick regardless of game state, so e.g. an Achievement / Frenzy
    /// event accelerating `border_phase` doesn't accelerate the
    /// "can't-buy" shimmer that happens to be running on a fingerer
    /// row at the same time.
    #[serde(skip)]
    pub steady_phase: u32,
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
    /// "Just became affordable" flash: a brief one-shot green shimmer
    /// fired the tick a row's affordability flips false → true. Distinct
    /// from `*_flash_ticks` (purchase) — shorter duration, no panel
    /// border bleed — so the player can tell "now buyable" apart from
    /// "you just bought."
    #[serde(skip)]
    pub fingerer_unlock_flash: Vec<u32>,
    #[serde(skip)]
    pub upgrade_unlock_flash: Vec<u32>,
    /// Brief gold shimmer on a fingerer row when a Green Coin catch
    /// targeted it. Closes the visual loop with the floating "+10%
    /// <fingerer>" particle and the green-tinted title-border pulse —
    /// the gold here matches the catch particle, so the player can see
    /// at a glance which row in the sidebar just took the boost.
    #[serde(skip)]
    pub fingerer_green_coin_flash: Vec<u32>,
    /// Previous-tick affordability per row, used to detect the
    /// false→true edge that triggers `*_unlock_flash`. Sized to catalog
    /// length by `migrate()` and seeded at init from the live state, so a
    /// freshly-loaded save with rows already affordable doesn't fire a
    /// fake unlock flash on tick 1.
    #[serde(skip)]
    pub prev_fingerer_affordable: Vec<bool>,
    #[serde(skip)]
    pub prev_upgrade_affordable: Vec<bool>,
    /// Held-spacebar tracking.
    ///
    /// `space_pressed_this_tick` is set whenever `Action::ClickCenter`
    /// arrives (terminal key-repeat fires Press events at ~30Hz, easily
    /// hitting every 50ms tick when a key is genuinely held).
    /// `ticks_since_last_press` is a small countdown that allows up to 3
    /// missed ticks (~150ms) before declaring the key released — handles
    /// real keyboard-repeat jitter so a 1-tick gap doesn't kill the
    /// streak. `space_hold_ticks` is the consecutive "active" tick streak;
    /// `space_held()` is true once it crosses 1 second.
    ///
    /// Net result: spamming spacebar at human speed (≥150ms between
    /// presses) never triggers held; actually holding the key climbs the
    /// streak past 20 ticks within ~1s.
    #[serde(skip)]
    pub space_pressed_this_tick: bool,
    #[serde(skip)]
    pub ticks_since_last_press: u32,
    #[serde(skip)]
    pub space_hold_ticks: u32,
    /// HUD count-up tween: rendered numbers smoothly chase the real ones.
    /// Initialized to the live values on load so the first frame doesn't
    /// look like a count-up from zero.
    #[serde(skip)]
    pub displayed_cuques: f64,
    #[serde(skip)]
    pub displayed_fps: f64,
    /// Brief green flash on the HUD digits when cuques jump UP — golden
    /// catch, frenzy click, F4 dev cheat, etc. ("money coming in")
    #[serde(skip)]
    pub cuques_flash_ticks: u32,
    /// Brief red flash on the HUD digits when cuques drop — successful
    /// purchase, prestige reset (the big -all event). Mirrors
    /// `cuques_flash_ticks` and competes with it: whichever channel is
    /// stronger this frame drives the HUD color sweep, so a buy that
    /// happens during a still-decaying gain pulse correctly flips the
    /// digits red instead of staying green.
    #[serde(skip)]
    pub cuques_spend_flash_ticks: u32,
}

pub const LUCKY_FLASH_TICKS: u32 = 70; // 3.5s at 20Hz
pub const PURCHASE_FLASH_TICKS: u32 = 20; // 1s at 20Hz
/// Green Coin catch pulse — slightly shorter than Lucky's so the celebratory
/// blip lands without lingering for so long it competes with whatever might
/// be running on top (Frenzy, Buff, Lucky).
pub const GREEN_COIN_FLASH_TICKS: u32 = 50; // 2.5s at 20Hz
/// Per-row gold shimmer on the targeted fingerer's sidebar row when a
/// Green Coin catch lands on it. ~2 seconds — long enough for the eye
/// to track from the floating "+10% <fingerer>" particle over to the
/// row, short enough that it doesn't outlive the catch event.
pub const GREEN_COIN_ROW_FLASH_TICKS: u32 = TICK_HZ * 2; // 2.0s at 20Hz

/// Serde default for `GameState::version`. A direct deserialize of the live
/// `GameState` from a pre-versioned save (one without the field) still
/// produces a sensibly-stamped state — though production loads always go
/// through the migration chain in `crate::save`.
fn default_save_version() -> u32 {
    crate::save::CURRENT_VERSION
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            version: crate::save::CURRENT_VERSION,
            cuques: 0.0,
            total_clicks: 0,
            lifetime_cuques: 0.0,
            best_fps: 0.0,
            golden_caught: 0,
            lucky_caught: 0,
            frenzy_caught: 0,
            buff_caught: 0,
            green_coin_caught: 0,
            fingerers_state: HashMap::new(),
            achievements_earned: HashSet::new(),
            upgrades_earned: HashSet::new(),
            prestige: 0,
            total_play_ticks: 0,
            buffs: Vec::new(),
            goldens_since_green_coin: 0,
            clench_ticks: 0,
            particles: Vec::new(),
            misclick_particles: Vec::new(),
            goldens: [None, None, None],
            golden_cooldowns: [
                crate::game::golden::next_cooldown(),
                crate::game::golden::next_cooldown(),
                crate::game::golden::next_cooldown(),
            ],
            green_coin: None,
            session_ticks: 0,
            newly_unlocked: Vec::new(),
            active_unlock_id: None,
            active_unlock_ticks: 0,
            visual_debt: 0.0,
            lucky_flash_ticks: 0,
            achievement_flash_ticks: 0,
            green_coin_flash_ticks: 0,
            border_phase: 0,
            steady_phase: 0,
            purchase_flash_ticks: 0,
            purchase_flash_strength: 1.0,
            fingerer_flash_ticks: vec![0; fingerer::count()],
            upgrade_flash_ticks: vec![0; UPGRADES.len()],
            fingerer_unaffordable_flash: vec![0; fingerer::count()],
            upgrade_unaffordable_flash: vec![0; UPGRADES.len()],
            fingerer_unlock_flash: vec![0; fingerer::count()],
            upgrade_unlock_flash: vec![0; UPGRADES.len()],
            fingerer_green_coin_flash: vec![0; fingerer::count()],
            prev_fingerer_affordable: vec![false; fingerer::count()],
            prev_upgrade_affordable: vec![false; UPGRADES.len()],
            space_pressed_this_tick: false,
            ticks_since_last_press: u32::MAX,
            space_hold_ticks: 0,
            displayed_cuques: 0.0,
            displayed_fps: 0.0,
            cuques_flash_ticks: 0,
            cuques_spend_flash_ticks: 0,
        }
    }
}

impl GameState {
    /// Initialize ephemeral runtime state that `#[serde(skip)]` left empty
    /// after deserialization, and normalize any fields that need live values
    /// rather than the serde default.
    ///
    /// **Runtime-only.** Persisted-shape migrations live in
    /// `crate::save::versions::vN.rs` (see CLAUDE.md "Save versioning").
    /// This method runs *after* the migration chain has produced a live
    /// `GameState`; it must not assume any particular pre-state and must
    /// be safe to call multiple times.
    pub fn migrate_runtime(mut self) -> Self {
        // `aggregate` is `#[serde(skip)]` — rebuild from the persisted
        // `modifiers` list before any code reads `fps()`.
        for st in self.fingerers_state.values_mut() {
            st.aggregate = FingererAggregate::rebuild(&st.modifiers);
        }
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
        if self.fingerer_unlock_flash.len() != fingerer::count() {
            self.fingerer_unlock_flash = vec![0; fingerer::count()];
        }
        if self.upgrade_unlock_flash.len() != UPGRADES.len() {
            self.upgrade_unlock_flash = vec![0; UPGRADES.len()];
        }
        if self.fingerer_green_coin_flash.len() != fingerer::count() {
            self.fingerer_green_coin_flash = vec![0; fingerer::count()];
        }
        // Seed `prev_affordable` from the LIVE state so a freshly-loaded
        // save with rows already affordable doesn't fire spurious unlock
        // flashes on tick 1. Resize if catalog grew/shrank.
        if self.prev_fingerer_affordable.len() != fingerer::count() {
            self.prev_fingerer_affordable =
                (0..fingerer::count()).map(|i| self.can_buy(i)).collect();
        }
        if self.prev_upgrade_affordable.len() != UPGRADES.len() {
            self.prev_upgrade_affordable = (0..UPGRADES.len())
                .map(|i| {
                    let u = &UPGRADES[i];
                    !self.has_upgrade(u.id) && u.req.met(&self) && self.cuques >= u.cost
                })
                .collect();
        }
        // Re-seed any cooldown left at 0 by an old save shape (the array
        // is `#[serde(skip)]` so it's already at default; this is a
        // defensive guard).
        for cd in self.golden_cooldowns.iter_mut() {
            if *cd == 0 {
                *cd = crate::game::golden::next_cooldown();
            }
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
        self.fingerers_state.get(id).map(|st| st.count).unwrap_or(0)
    }

    pub fn fingerer_count_idx(&self, idx: usize) -> u32 {
        FINGERERS
            .get(idx)
            .map(|f| self.fingerer_count(f.id))
            .unwrap_or(0)
    }

    pub fn fingerers_owned_total(&self) -> u32 {
        self.fingerers_state.values().map(|st| st.count).sum()
    }

    /// Return the cached modifier aggregate for `id`, or the identity
    /// (`Default`) if the fingerer has no entry. Hot-path read for `fps()`
    /// and the sidebar — never iterates the underlying `Vec<Modifier>`.
    pub fn fingerer_aggregate(&self, id: &str) -> FingererAggregate {
        self.fingerers_state
            .get(id)
            .map(|st| st.aggregate)
            .unwrap_or_default()
    }

    /// Attach a modifier to the given fingerer id. Creates the
    /// `FingererState` entry on the fly if absent (count stays 0). Rebuilds
    /// the aggregate cache. Use this from goldens, debug cheats, future
    /// events.
    pub fn attach_modifier(&mut self, fingerer_id: &str, m: Modifier) {
        let st = self
            .fingerers_state
            .entry(fingerer_id.to_string())
            .or_default();
        st.modifiers.push(m);
        st.aggregate = FingererAggregate::rebuild(&st.modifiers);
    }

    /// Pick a random fingerer with `count > 0` and attach `m` to it. Returns
    /// the chosen id, or `None` if no fingerer is owned. Used by the Buff
    /// Golden (Purple Coin), where targeting an un-owned tier is pointless
    /// — a temporary x7 multiplier on a count of zero produces zero output.
    pub fn attach_modifier_random_owned(&mut self, m: Modifier) -> Option<String> {
        let owned: Vec<String> = self
            .fingerers_state
            .iter()
            .filter(|(_, st)| st.count > 0)
            .map(|(id, _)| id.clone())
            .collect();
        if owned.is_empty() {
            return None;
        }
        let pick = owned[rand::rng().random_range(0..owned.len())].clone();
        self.attach_modifier(&pick, m);
        Some(pick)
    }

    /// Pick a random fingerer that is currently *visible in the sidebar*
    /// — by the same `fingerer::visible` rule the UI uses (`idx == 0` ||
    /// `owned > 0` || `lifetime_cuques >= base_cost * 0.5`) — and attach
    /// `m` to it.
    ///
    /// Used by the Green Coin: a *permanent* +10% boost is still useful on
    /// a tier the player can see but hasn't bought yet; when they finally
    /// buy it the boost is already in place. Index Finger is always visible
    /// (`idx == 0`), so as long as `FINGERERS` is non-empty this picks
    /// something. Returns `None` only on an empty catalog (never in
    /// practice).
    pub fn attach_modifier_random_visible(&mut self, m: Modifier) -> Option<String> {
        let visible: Vec<String> = FINGERERS
            .iter()
            .enumerate()
            .filter(|(idx, f)| {
                let owned = self.fingerer_count(f.id);
                fingerer::visible(*idx, owned, self.lifetime_cuques)
            })
            .map(|(_, f)| f.id.to_string())
            .collect();
        if visible.is_empty() {
            return None;
        }
        let pick = visible[rand::rng().random_range(0..visible.len())].clone();
        self.attach_modifier(&pick, m);
        Some(pick)
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
            let Buff::ClickFrenzy { mult, .. } = b;
            m *= *mult;
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
        // Per-fingerer Buff golden contributions used to live here as
        // `Buff::FingererBoost`. They now flow through the modifier
        // aggregate (see `fingerer_aggregate`), keeping `fingerer_mult`
        // strictly about upgrades.
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

    /// Catch the Golden Cuque of the given variant if one is currently
    /// on screen. Applies the variant-specific effect, increments
    /// `golden_caught` + the per-variant counter, re-rolls the shared
    /// spawn cooldown, and returns the flat reward (0.0 for non-Lucky).
    ///
    /// Each variant lives in its own slot of `state.goldens`, so catching
    /// e.g. a Lucky never disturbs an active Frenzy or Buff. Same applies
    /// to expiry / cheat spawns.
    ///
    /// The `Buff` variant attaches a `MulFactor(7.0)` modifier with a
    /// 60-second `Ticks` duration on a random owned fingerer, sourced as
    /// `PurpleCoin`. Pre-#21 this was a global `Buff::FingererBoost`; the
    /// modifier system replaces it.
    pub fn catch_golden(&mut self, variant: crate::game::golden::GoldenVariant) -> f64 {
        use crate::game::golden::GoldenVariant;
        let Some(golden) = self.goldens[variant as usize].take() else {
            return 0.0;
        };
        self.golden_caught += 1;
        // Reset only THIS variant's cooldown — sibling variants keep their
        // own clocks running on independent schedules.
        self.golden_cooldowns[variant as usize] = crate::game::golden::next_cooldown();
        let (reward, label) = match golden.variant {
            GoldenVariant::Lucky => {
                self.lucky_caught += 1;
                let fps = self.fps();
                let r = (fps * GOLDEN_REWARD_SECONDS).max(GOLDEN_REWARD_FLAT);
                self.add_cuques(r);
                self.lucky_flash_ticks = LUCKY_FLASH_TICKS;
                self.cuques_flash_ticks = HUD_FLASH_TICKS;
                (r, format!("+{}", crate::format::big(r)))
            }
            GoldenVariant::Frenzy => {
                self.frenzy_caught += 1;
                let dur = TICK_HZ * 13;
                self.buffs.push(Buff::ClickFrenzy {
                    ticks_remaining: dur,
                    initial_ticks: dur,
                    mult: 777.0,
                });
                (0.0, "FRENZY x777!".into())
            }
            GoldenVariant::Buff => {
                self.buff_caught += 1;
                let dur = TICK_HZ * 60;
                let m = Modifier {
                    source: crate::game::modifier::ModifierSource::PurpleCoin,
                    effects: vec![crate::game::modifier::ModifierEffect::MulFactor(7.0)],
                    duration: ModifierDuration::Ticks(dur),
                    created_at_tick: self.total_play_ticks,
                };
                // Fall back to the first catalog tier if the player owns
                // nothing yet — same defensive behavior the legacy buff had.
                if self.attach_modifier_random_owned(m.clone()).is_none() {
                    let pick = FINGERERS[0].id;
                    self.attach_modifier(pick, m);
                }
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
        // Per-fingerer formula:
        //   pre  = (base * count + flat_fps) * upgrades_mult
        //   post = pre * (1 + add_percent) * mul_factor
        // Reads use the cached aggregate — never iterate the modifiers Vec.
        let base: f64 = FINGERERS
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let count = self.fingerer_count(k.id) as f64;
                let upgrades_mult = self.fingerer_mult(i);
                let agg = self.fingerer_aggregate(k.id);
                let pre = (k.fps_per_unit * count + agg.flat_fps) * upgrades_mult;
                pre * (1.0 + agg.add_percent) * agg.mul_factor
            })
            .sum();
        base * self.prestige_mult()
    }

    pub fn border_speed(&self) -> u32 {
        let mut s: u32 = 1;
        for b in &self.buffs {
            match b {
                Buff::ClickFrenzy { .. } => s = s.max(3),
            }
        }
        // Active timed per-fingerer modifiers (PurpleCoin and friends)
        // bump the border one notch — same baseline the old
        // `Buff::FingererBoost` arm produced.
        if self.fingerers_state.values().any(|st| {
            st.modifiers
                .iter()
                .any(|m| matches!(m.duration, ModifierDuration::Ticks(_)))
        }) {
            s = s.max(2);
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
        // Don't snap `displayed_cuques` to 0 — let it tween down from
        // its pre-reset value over the next ~1s for a "draining"
        // feel. Same for FPS. The red spend-flash is fired below to
        // color the falling counter.
        self.cuques_spend_flash_ticks = HUD_FLASH_TICKS;
        // Wipe count AND modifiers — prestige resets the run, which is the
        // whole point. Permanent Green Coin boosts do not survive a prestige.
        self.fingerers_state.clear();
        self.upgrades_earned.clear();
        self.buffs.clear();
        self.visual_debt = 0.0;
        self.particles.clear();
        self.misclick_particles.clear();
        self.goldens = [None, None, None];
        self.green_coin = None;
        // Pity counter resets too — the new run earns its own Green Coins.
        self.goldens_since_green_coin = 0;
        self.clench_ticks = 0;
        // Fresh per-variant cooldowns so the new run has its own
        // independent rhythm from tick 1.
        for cd in self.golden_cooldowns.iter_mut() {
            *cd = crate::game::golden::next_cooldown();
        }
        true
    }

    pub fn tick(&mut self) {
        // Per-fingerer modifier walk: decrement timed durations, drop expired
        // ones, rebuild the aggregate of any fingerer that lost a modifier.
        // Permanent modifiers are walked over but untouched. The walk runs
        // before the `buffs` walk so a coin caught this same tick already
        // ages by 1 — same convention as Buff::tick.
        for st in self.fingerers_state.values_mut() {
            let before = st.modifiers.len();
            st.modifiers.retain_mut(|m| match &mut m.duration {
                ModifierDuration::Permanent => true,
                ModifierDuration::Ticks(0) => false,
                ModifierDuration::Ticks(n) => {
                    *n -= 1;
                    true
                }
            });
            if before != st.modifiers.len() {
                st.aggregate = FingererAggregate::rebuild(&st.modifiers);
            }
        }

        for b in self.buffs.iter_mut() {
            b.tick();
        }
        self.buffs.retain(|b| b.ticks_remaining() > 0);

        self.lucky_flash_ticks = self.lucky_flash_ticks.saturating_sub(1);
        self.achievement_flash_ticks = self.achievement_flash_ticks.saturating_sub(1);
        self.green_coin_flash_ticks = self.green_coin_flash_ticks.saturating_sub(1);
        self.purchase_flash_ticks = self.purchase_flash_ticks.saturating_sub(1);
        if self.purchase_flash_ticks == 0 {
            self.purchase_flash_strength = 1.0;
        }
        self.cuques_flash_ticks = self.cuques_flash_ticks.saturating_sub(1);
        self.cuques_spend_flash_ticks = self.cuques_spend_flash_ticks.saturating_sub(1);
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
        for t in self.fingerer_unlock_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.upgrade_unlock_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.fingerer_green_coin_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        // Held-spacebar streak with a small grace window. Real key-repeat
        // is bursty (~30Hz nominal but with OS-level jitter), so a strict
        // "every tick must see a press" test breaks on a single missed
        // tick. Instead: a press resets `ticks_since_last_press` to 0;
        // each tick increments it; the streak counts ticks that arrived
        // within the last ~150ms (3 ticks). Spamming with ≥150ms gaps
        // (human tap speed) never builds a streak. Genuine holding (key
        // repeat) keeps `ticks_since_last_press ≤ 1` and the streak
        // climbs by 1 every tick.
        if self.space_pressed_this_tick {
            self.ticks_since_last_press = 0;
        } else {
            self.ticks_since_last_press = self.ticks_since_last_press.saturating_add(1);
        }
        self.space_pressed_this_tick = false;
        const HOLD_GRACE_TICKS: u32 = 3; // ~150ms at 20Hz
        if self.ticks_since_last_press <= HOLD_GRACE_TICKS {
            self.space_hold_ticks = self.space_hold_ticks.saturating_add(1);
        } else {
            self.space_hold_ticks = 0;
        }
        let speed = self.border_speed();
        self.border_phase = self.border_phase.wrapping_add(speed);
        self.steady_phase = self.steady_phase.wrapping_add(1);

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

        // K7: edge-detect false→true affordability flips and fire a brief
        // unlock flash on the row. Detection runs AFTER `add_cuques(gained)`
        // so an income-driven crossover lights up immediately. Two-pass to
        // keep the immutable reads (`can_buy`, `req.met`, etc.) cleanly
        // separated from the mutable writes to the flash + prev vecs.
        let fingerer_now: Vec<bool> = (0..fingerer::count()).map(|i| self.can_buy(i)).collect();
        let upgrade_now: Vec<bool> = UPGRADES
            .iter()
            .map(|u| !self.has_upgrade(u.id) && u.req.met(self) && self.cuques >= u.cost)
            .collect();
        for (i, &now) in fingerer_now.iter().enumerate() {
            let was = self
                .prev_fingerer_affordable
                .get(i)
                .copied()
                .unwrap_or(false);
            if now
                && !was
                && let Some(slot) = self.fingerer_unlock_flash.get_mut(i)
            {
                *slot = UNLOCK_FLASH_TICKS;
            }
            if let Some(slot) = self.prev_fingerer_affordable.get_mut(i) {
                *slot = now;
            }
        }
        for (i, &now) in upgrade_now.iter().enumerate() {
            let was = self
                .prev_upgrade_affordable
                .get(i)
                .copied()
                .unwrap_or(false);
            if now
                && !was
                && let Some(slot) = self.upgrade_unlock_flash.get_mut(i)
            {
                *slot = UNLOCK_FLASH_TICKS;
            }
            if let Some(slot) = self.prev_upgrade_affordable.get_mut(i) {
                *slot = now;
            }
        }

        // Count-up tween: rendered numbers chase the real ones with
        // ease-out for BIG jumps (golden, F4, max-buy) so the eye can
        // track the rise. Small deltas snap — a single +1 manual click
        // would otherwise take ~30 ticks (1.5s) to finish tweening, AND
        // `format::big` floors the in-flight value, so the HUD shows "0"
        // for most of the climb. Counter-productive juice. The threshold
        // (`SNAP_BELOW`) is in absolute cuques: any change smaller than
        // ~5 cuques snaps instantly; bigger ones tween. The same
        // threshold applies to FPS for symmetry — small FPS deltas come
        // from buying a single fingerer, not worth a tween.
        const SNAP_BELOW: f64 = 5.0;
        let tween = 0.18_f64;
        let dc = self.cuques - self.displayed_cuques;
        if dc.abs() < SNAP_BELOW {
            self.displayed_cuques = self.cuques;
        } else {
            self.displayed_cuques += dc * tween;
        }
        let df = fps - self.displayed_fps;
        if df.abs() < SNAP_BELOW {
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
        // Each variant ages on its own clock. Lifetime ticks down per
        // slot; on expiry, that slot's cooldown rolls fresh. Cooldown
        // ticks down only when its variant's slot is empty — no point
        // counting down to zero while there's nowhere to spawn.
        for i in 0..self.goldens.len() {
            if let Some(g) = self.goldens[i].as_mut() {
                if g.life_ticks == 0 {
                    self.goldens[i] = None;
                    self.golden_cooldowns[i] = crate::game::golden::next_cooldown();
                } else {
                    g.life_ticks -= 1;
                }
            } else if self.golden_cooldowns[i] > 0 {
                self.golden_cooldowns[i] -= 1;
            }
        }
    }

    /// Lifetime tick for the Green Coin (mirror of `tick_golden`'s coin
    /// half — Green Coin has no cooldown of its own; spawning is gated on
    /// regular Golden spawns instead). Decrements `life_ticks` each call,
    /// clears the slot when it hits 0.
    pub fn tick_green_coin(&mut self) {
        if let Some(g) = self.green_coin.as_mut() {
            if g.life_ticks == 0 {
                self.green_coin = None;
            } else {
                g.life_ticks -= 1;
            }
        }
    }

    /// Catch the on-screen Green Coin if any. Picks a random owned fingerer
    /// (`count > 0`) and attaches a permanent `+10%` `AddPercent` modifier
    /// sourced as `GreenCoin`. Returns `true` if a coin was consumed (catch
    /// or no-op miss because nothing was owned), `false` if there was no
    /// coin to catch in the first place.
    ///
    /// Edge case: if the player owns nothing yet, the coin is consumed but
    /// no modifier attaches — same defensive behavior as the old Buff
    /// golden when the catalog was empty. The +10% had nothing to land on.
    pub fn catch_green_coin(&mut self) -> bool {
        let Some(g) = self.green_coin.take() else {
            return false;
        };
        let m = Modifier {
            source: ModifierSource::GreenCoin,
            effects: vec![ModifierEffect::AddPercent(
                crate::game::green_coin::GREEN_COIN_ADD_PERCENT,
            )],
            duration: ModifierDuration::Permanent,
            created_at_tick: self.total_play_ticks,
        };
        // Visible-set targeting: a permanent +10% can land on a sidebar-
        // visible tier the player hasn't bought yet, so they get a head
        // start when they finally afford it. (Buff golden uses
        // `attach_modifier_random_owned` instead, since its temp x7 only
        // matters on a tier with non-zero count.)
        let chosen = self.attach_modifier_random_visible(m);
        // Counts toward both the all-time rollup and the dedicated
        // Green Coin counter (V3+). Achievements that gate on
        // `golden_caught` (e.g. "Golden Touch") still fire because the
        // rollup keeps incrementing.
        self.golden_caught += 1;
        self.green_coin_caught += 1;
        self.green_coin_flash_ticks = GREEN_COIN_FLASH_TICKS;
        // Visual feedback: a "+10% <fingerer>" particle anchored at the
        // coin's position + a gold sidebar-row shimmer on the targeted
        // tier (closes the loop between the floating particle and the
        // sidebar badge that just gained another +10%).
        let label = match &chosen {
            Some(id) => {
                let idx = FINGERERS.iter().position(|f| f.id == id);
                if let Some(i) = idx
                    && let Some(slot) = self.fingerer_green_coin_flash.get_mut(i)
                {
                    *slot = GREEN_COIN_ROW_FLASH_TICKS;
                }
                let name = idx
                    .and_then(|i| crate::i18n::t().fingerer_names.get(i).copied())
                    .unwrap_or("?");
                format!("+10% {}", name)
            }
            // No owned fingerer to host the boost — show a neutral marker.
            None => "+10% ???".to_string(),
        };
        self.particles.push(Particle {
            frac_x: g.frac_x,
            frac_y: g.frac_y,
            life: PARTICLE_LIFE * 2,
            text: label,
            kind: ParticleKind::Golden,
            drift_x: 0.0,
        });
        true
    }

    pub fn trigger_clench(&mut self) {
        self.clench_ticks = CLENCH_TICKS;
    }

    /// True when the spacebar has been held continuously for ≥ 1 second.
    /// Driven by `space_hold_ticks` (a streak counter that increments on
    /// every tick where at least one ClickCenter arrived, resets the
    /// instant a tick passes without one). Switches the biscuit's clench
    /// animation from a burning `*` to the spin frames `\ | / -`.
    pub fn space_held(&self) -> bool {
        self.space_hold_ticks >= TICK_HZ
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
        // Floor the result so the cost ALWAYS equals what `format::big`
        // shows the player. The price formula scales by 1.15× per owned
        // unit and produces fractional cuques (e.g. 15 × 1.15⁶ = 34.69).
        // Without flooring, the HUD says "Cuques: 34, cost 34" but the
        // affordability check `cuques >= 34.69` rejects — the player sees
        // a lie. Floor here keeps display, gate, and spend consistent at
        // the integer grain the player actually sees.
        let raw = k.base_cost * k.cost_scale.powi(self.fingerer_count_idx(idx) as i32);
        raw.floor()
    }

    /// Cuques the player can ACTUALLY spend right now: the lesser of real
    /// `cuques` and the displayed counter. Both bounds matter:
    ///
    /// - Gating ONLY on `cuques` (real) lets the row turn green and a
    ///   click succeed before the counter visibly catches up — the
    ///   "I have 8 but the row says I can buy a 17" lie.
    /// - Gating ONLY on `displayed_cuques.floor()` lets a click DRAIN
    ///   real cuques NEGATIVE during a spend's tween-down: real already
    ///   dropped, displayed hasn't caught down yet, gate sees the high
    ///   displayed value and lets the buy through against the depleted
    ///   real. Once `cuques` goes negative, the HUD floor() shows "0"
    ///   for a long time while the slow income climbs back.
    ///
    /// Taking `min(real, displayed.floor())` makes both conditions
    /// equally binding: row turns green only when the visible counter
    /// AND the underlying balance both reach the cost; click succeeds
    /// only when both still hold. No overspend, no visual lie.
    pub fn affordable_cuques(&self) -> f64 {
        self.cuques.min(self.displayed_cuques.floor())
    }

    pub fn can_buy(&self, idx: usize) -> bool {
        self.affordable_cuques() >= self.cost(idx)
    }

    /// Buy a single unit. Bare mutation only — flash side-effects are
    /// scaled by quantity in `buy_n` / `buy_max` so a single buy and a
    /// bulk buy produce visually distinct feedback.
    fn buy_one_quiet(&mut self, idx: usize) -> bool {
        let c = self.cost(idx);
        // Use the same min(real, displayed) gate as `can_buy` so the
        // visible row state and the buy outcome agree, AND we never
        // spend more than `cuques` actually has. We do NOT snap
        // `displayed_cuques` to post-spend `cuques` — the existing tick
        // path tweens it down and the red spend flash colors that fall.
        if self.affordable_cuques() >= c
            && let Some(f) = FINGERERS.get(idx)
        {
            self.cuques -= c;
            self.fingerers_state
                .entry(f.id.to_string())
                .or_default()
                .count += 1;
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
        // A buy is a SPEND — it always fires the red HUD flash so the
        // counter dropping is visibly acknowledged. Earlier this slot
        // mistakenly used `cuques_flash_ticks` (the gain channel),
        // making big buys flash green even though cuques went DOWN.
        // Bulk buys also pop confetti for celebratory feel.
        self.cuques_spend_flash_ticks = HUD_FLASH_TICKS;
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
        // Same min(real, displayed) gate as fingerer buys — see
        // `affordable_cuques` for why both bounds matter.
        if !u.req.met(self) || self.affordable_cuques() < u.cost {
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
    use crate::game::modifier::{Modifier, ModifierEffect, ModifierSource};

    fn fs_with_count(count: u32) -> FingererState {
        FingererState {
            count,
            ..Default::default()
        }
    }

    #[test]
    fn migrate_is_idempotent_on_current_shape() {
        let state = GameState {
            fingerers_state: [("index_finger".to_string(), fs_with_count(9))]
                .into_iter()
                .collect(),
            upgrades_earned: ["click_mult_1".to_string()].into_iter().collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };

        let m = state.migrate_runtime();

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
            fingerers_state: [("giga_finger_from_the_future".to_string(), fs_with_count(42))]
                .into_iter()
                .collect(),
            ..GameState::default()
        };

        let m = state.migrate_runtime();

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
            fingerers_state: [("index_finger".to_string(), fs_with_count(7))]
                .into_iter()
                .collect(),
            upgrades_earned: ["click_mult_1".to_string()].into_iter().collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };

        let json = serde_json::to_string(&state).expect("serialize");
        let roundtripped: GameState = serde_json::from_str(&json).expect("deserialize");
        let m = roundtripped.migrate_runtime();

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
        // (Default already zeroes `cuques`; no explicit reset needed.)
        let mut s = GameState::default();
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
        let bought = s.buy_n(0, 10);
        assert_eq!(bought, 0);
        assert!(s.fingerer_unaffordable_flash[0] > 0);
    }

    #[test]
    fn bulk_buy_scales_purchase_flash_strength() {
        // J8: max-buy is louder than a +1. We don't pin exact values (clamp
        // boundaries are tuning), only the relative ordering and bounds.
        // `displayed_cuques` must mirror `cuques` here because buy()'s
        // affordability gate now reads displayed (matches the visible
        // counter on the HUD) — a default-constructed test state has
        // displayed=0 and would otherwise reject every buy.
        let mut s = GameState {
            cuques: 1_000_000.0,
            displayed_cuques: 1_000_000.0,
            ..Default::default()
        };
        s.buy(0);
        let single = s.purchase_flash_strength;
        assert!((1.0..=3.0).contains(&single));

        let mut s = GameState {
            cuques: 1_000_000.0,
            displayed_cuques: 1_000_000.0,
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
        let m = s.migrate_runtime();
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
        let m = s.migrate_runtime();
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

    // -- Modifier system ----------------------------------------------------

    fn perm_add_percent(pct: f64) -> Modifier {
        Modifier {
            source: ModifierSource::GreenCoin,
            effects: vec![ModifierEffect::AddPercent(pct)],
            duration: ModifierDuration::Permanent,
            created_at_tick: 0,
        }
    }

    fn timed_mul(mult: f64, ticks: u32) -> Modifier {
        Modifier {
            source: ModifierSource::PurpleCoin,
            effects: vec![ModifierEffect::MulFactor(mult)],
            duration: ModifierDuration::Ticks(ticks),
            created_at_tick: 0,
        }
    }

    #[test]
    fn attach_modifier_rebuilds_aggregate() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        s.attach_modifier("index_finger", perm_add_percent(0.10));
        let agg = s.fingerer_aggregate("index_finger");
        assert!((agg.add_percent - 0.10).abs() < 1e-9);

        // Stacking: a second modifier sums into the same aggregate.
        s.attach_modifier("index_finger", perm_add_percent(0.10));
        let agg = s.fingerer_aggregate("index_finger");
        assert!((agg.add_percent - 0.20).abs() < 1e-9);
    }

    #[test]
    fn attach_modifier_creates_state_entry_if_absent() {
        // Attaching to a fingerer the player doesn't own creates a zero-count
        // entry rather than silently dropping the modifier. (Production code
        // pairs `attach_modifier_random_owned` with the count > 0 filter, so
        // this only matters when something explicitly targets a tier.)
        let mut s = GameState::default();
        s.attach_modifier("hand_of_god", perm_add_percent(0.10));
        let st = s.fingerers_state.get("hand_of_god").expect("entry exists");
        assert_eq!(st.count, 0);
        assert_eq!(st.modifiers.len(), 1);
    }

    #[test]
    fn attach_modifier_random_owned_picks_only_owned() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(5));
        // Add an empty entry for an unowned fingerer; the picker must skip it.
        s.fingerers_state
            .insert("hand_of_god".into(), fs_with_count(0));
        let chosen = s.attach_modifier_random_owned(perm_add_percent(0.10));
        assert_eq!(chosen.as_deref(), Some("index_finger"));
    }

    #[test]
    fn attach_modifier_random_owned_returns_none_when_nothing_owned() {
        let mut s = GameState::default();
        let chosen = s.attach_modifier_random_owned(perm_add_percent(0.10));
        assert!(chosen.is_none());
        // No entries created — random_owned doesn't have a target to pick.
        assert!(s.fingerers_state.is_empty());
    }

    #[test]
    fn tick_decrements_timed_modifiers() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        s.attach_modifier("index_finger", timed_mul(2.0, 5));
        s.tick();
        let st = s.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.modifiers.len(), 1);
        assert!(matches!(
            st.modifiers[0].duration,
            ModifierDuration::Ticks(4)
        ));
    }

    #[test]
    fn tick_removes_expired_and_rebuilds_aggregate() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        s.attach_modifier("index_finger", timed_mul(2.0, 1));
        // First tick: Ticks(1) → Ticks(0), still present.
        s.tick();
        assert_eq!(
            s.fingerers_state
                .get("index_finger")
                .unwrap()
                .modifiers
                .len(),
            1
        );
        // Second tick: Ticks(0) is dropped, aggregate rebuilt to identity.
        s.tick();
        let st = s.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.modifiers.len(), 0);
        assert!((st.aggregate.mul_factor - 1.0).abs() < 1e-9);
    }

    #[test]
    fn permanent_modifier_does_not_decrement() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        s.attach_modifier("index_finger", perm_add_percent(0.10));
        for _ in 0..50 {
            s.tick();
        }
        let st = s.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.modifiers.len(), 1);
        assert!(matches!(
            st.modifiers[0].duration,
            ModifierDuration::Permanent
        ));
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
    }

    #[test]
    fn prestige_reset_clears_modifiers() {
        // Prestige resets the run — permanent Green Coin boosts must not
        // survive. Otherwise a prestiged player would carry +N% on tier-1
        // forever.
        let mut s = GameState {
            lifetime_cuques: 1_000_000_000.0,
            ..Default::default()
        };
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(5));
        s.attach_modifier("index_finger", perm_add_percent(0.30));
        assert!(s.prestige_reset());
        assert!(s.fingerers_state.is_empty());
    }

    #[test]
    fn fps_uses_aggregate_add_percent() {
        // Same fingerer count, +10% AddPercent modifier → fps 10% higher.
        let mut bare = GameState::default();
        bare.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        let bare_fps = bare.fps();

        let mut boosted = GameState::default();
        boosted
            .fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        boosted.attach_modifier("index_finger", perm_add_percent(0.10));
        let boosted_fps = boosted.fps();

        assert!(bare_fps > 0.0);
        assert!((boosted_fps - bare_fps * 1.10).abs() < 1e-9);
    }

    #[test]
    fn migrate_runtime_rebuilds_aggregate_after_serde_skip() {
        // The aggregate field is `#[serde(skip)]`; a state freshly
        // deserialized from JSON has it at the identity Default. Running
        // migrate_runtime() must reconstitute it from the modifier list.
        let mut s = GameState::default();
        s.fingerers_state.insert(
            "index_finger".into(),
            FingererState {
                count: 1,
                modifiers: vec![perm_add_percent(0.25)],
                aggregate: FingererAggregate::default(), // simulate post-deserialize
            },
        );
        let m = s.migrate_runtime();
        let agg = m.fingerer_aggregate("index_finger");
        assert!((agg.add_percent - 0.25).abs() < 1e-9);
    }

    // -- Green Coin ---------------------------------------------------------

    use crate::game::green_coin::{GREEN_COIN_LIFE_TICKS, GreenCoin};

    fn fake_green_coin() -> GreenCoin {
        GreenCoin {
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: GREEN_COIN_LIFE_TICKS,
        }
    }

    #[test]
    fn catch_green_coin_increments_grand_total_and_per_variant_counter() {
        let mut s = GameState {
            green_coin: Some(fake_green_coin()),
            ..Default::default()
        };
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        assert!(s.catch_green_coin());
        assert_eq!(s.golden_caught, 1, "rollup increments");
        assert_eq!(s.green_coin_caught, 1, "per-variant increments");
        assert_eq!(s.lucky_caught, 0);
        assert_eq!(s.frenzy_caught, 0);
        assert_eq!(s.buff_caught, 0);
    }

    #[test]
    fn catch_green_coin_attaches_permanent_modifier() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(3));
        s.green_coin = Some(fake_green_coin());

        let caught = s.catch_green_coin();

        assert!(caught);
        assert!(s.green_coin.is_none());
        let st = s.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.modifiers.len(), 1);
        let m = &st.modifiers[0];
        assert!(matches!(m.source, ModifierSource::GreenCoin));
        assert!(matches!(m.duration, ModifierDuration::Permanent));
        assert!(matches!(
            m.effects[0],
            ModifierEffect::AddPercent(v) if (v - 0.10).abs() < 1e-9
        ));
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
    }

    #[test]
    fn catch_green_coin_with_no_owned_lands_on_index_finger() {
        // The visible-set targeting (vs the old owned-only) means even on a
        // brand-new save, a Green Coin attaches somewhere — Index Finger is
        // always visible (`idx == 0` short-circuits in `fingerer::visible`).
        // The previous "consumes without attaching" no-op behavior is gone.
        let mut s = GameState {
            green_coin: Some(fake_green_coin()),
            ..Default::default()
        };

        let caught = s.catch_green_coin();

        assert!(caught);
        assert!(s.green_coin.is_none());
        let st = s
            .fingerers_state
            .get(FINGERERS[0].id)
            .expect("modifier landed on Index Finger");
        assert_eq!(st.modifiers.len(), 1);
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
    }

    #[test]
    fn attach_modifier_random_visible_can_pick_unowned_when_lifetime_unlocks_it() {
        // Tier 1 (Whole Hand) has base_cost 100; visibility threshold is
        // 0.5 * base_cost = 50. With lifetime_cuques == 60, Whole Hand is
        // visible despite being unowned. Index Finger is always visible
        // (idx == 0). The picker can land on either; with a deterministic
        // seed we can't pin which, but the modifier lands SOMEWHERE.
        let mut s = GameState {
            lifetime_cuques: 60.0,
            ..Default::default()
        };
        let m = perm_add_percent(0.10);
        let chosen = s.attach_modifier_random_visible(m);
        let id = chosen.expect("at least one visible fingerer always exists");
        // Allowed targets at this lifetime: Index Finger + Whole Hand.
        let visible_ids: Vec<&str> = FINGERERS
            .iter()
            .enumerate()
            .filter(|(idx, f)| {
                fingerer::visible(*idx, 0, s.lifetime_cuques) && (*idx == 0 || f.id == "whole_hand")
            })
            .map(|(_, f)| f.id)
            .collect();
        assert!(visible_ids.contains(&id.as_str()));
    }

    #[test]
    fn catch_green_coin_returns_false_when_no_coin() {
        let mut s = GameState::default();
        assert!(!s.catch_green_coin());
    }

    #[test]
    fn tick_green_coin_decrements_lifetime_and_clears_at_zero() {
        let mut s = GameState {
            green_coin: Some(GreenCoin {
                frac_x: 0.5,
                frac_y: 0.5,
                life_ticks: 2,
            }),
            ..Default::default()
        };
        s.tick_green_coin();
        assert_eq!(s.green_coin.as_ref().unwrap().life_ticks, 1);
        s.tick_green_coin();
        // Now `life_ticks` is 0 but slot still occupied.
        assert_eq!(s.green_coin.as_ref().unwrap().life_ticks, 0);
        s.tick_green_coin();
        // The next tick clears it (mirrors `tick_golden`'s convention).
        assert!(s.green_coin.is_none());
    }

    #[test]
    fn green_coin_stacks_additively_on_repeat_catches() {
        // Two Green Coins on the same fingerer = +20%, not +21%.
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        for _ in 0..2 {
            s.green_coin = Some(fake_green_coin());
            s.catch_green_coin();
        }
        let st = s.fingerers_state.get("index_finger").unwrap();
        // RNG randomly picks the only owned fingerer both times.
        assert_eq!(st.modifiers.len(), 2);
        assert!((st.aggregate.add_percent - 0.20).abs() < 1e-9);
    }

    #[test]
    fn prestige_reset_clears_green_coin_state() {
        let mut s = GameState {
            lifetime_cuques: 1_000_000_000.0,
            ..Default::default()
        };
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        s.goldens_since_green_coin = 7;
        s.green_coin = Some(fake_green_coin());
        s.prestige_reset();
        assert!(s.green_coin.is_none());
        assert_eq!(s.goldens_since_green_coin, 0);
    }
}
