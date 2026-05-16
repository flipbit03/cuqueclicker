use std::collections::{HashMap, HashSet};

use rand::RngExt;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

use crate::game::achievement::ACHIEVEMENTS;
use crate::game::fingerer::{self, FINGERERS};
use crate::game::modifier::{
    FingererAggregate, Modifier, ModifierDuration, ModifierEffect, ModifierSource,
};
use crate::game::powerup::{self, N_KINDS, Powerup, PowerupKind};
use crate::game::tree::aggregate::TreeAggregate;
use crate::game::tree::coord::TreeCoord;
use crate::game::tree::node::{self, NodeSpec};
use crate::game::tree::state::UpgradeTreeState;

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
/// Cells the edge-unlock wavefront advances per tick. At `TICK_HZ = 20`
/// a value of 2 = 40 cells / sec — a typical 8-cell straight edge fully
/// energizes in ~0.2 s, a longer diagonal in ~0.4 s. Bumping this is
/// the right knob when the user says "make it faster"; going below 1
/// (e.g. tick / 2 cells/tick = slower) needs a different mechanism
/// since we sample integer cells per tick.
pub const EDGE_UNLOCK_CELLS_PER_TICK: u32 = 2;

/// In-flight "path lights up" animation when a buy lights an edge.
/// Lives only at runtime — `#[serde(skip)]`-projected fields don't ever
/// reach disk.
///
/// `gates_destination` is true when the buy newly made the destination
/// reachable — those destinations are held in "not yet reachable" UX
/// until the wave arrives, and get the gold unlock_flash on completion.
/// Edges to already-reachable neighbors animate decoratively (so every
/// newly-lit edge gets the snake) but DON'T gate the destination, since
/// the player was free to buy it before this animation started.
///
/// Wave geometry (leading_inside / trailing_inside / visible length)
/// is computed lazily against `node::edge_path_cells(from, to)`, which
/// returns a canonical lo→hi-ordered path. Caching the offsets on the
/// anim would couple them to the call-site direction; under a renderer
/// that iterated the edge in the opposite (a, b) order they pointed at
/// the wrong end of the line.
#[derive(Clone, Copy, Debug)]
pub struct EdgeUnlockAnim {
    pub from: TreeCoord,
    pub to: TreeCoord,
    pub ticks: u32,
    pub gates_destination: bool,
}

impl EdgeUnlockAnim {
    /// Visible-cell offset of the wavefront — how many cells past the
    /// source-side leading-inside region the head has advanced.
    pub fn visible_advance(&self) -> usize {
        self.ticks.saturating_mul(EDGE_UNLOCK_CELLS_PER_TICK) as usize
    }
}

use node::{count_leading_in_rect, count_trailing_in_rect};
/// Per-tick upward drift for a particle, expressed as a fraction of the
/// biscuit's height. Calibrated to match the original feel before the
/// switch to fractional anchors: the old code rose 0.18 cells/tick on
/// any biscuit size; on a typical ~30-row biscuit that's 0.006 of height
/// per tick — slow enough that a "+1" only travels ~10-12% of the biscuit
/// across its 1-second life, instead of streaking across half of it.
const PARTICLE_FRAC_RISE: f32 = 0.006;
const GOLDEN_REWARD_SECONDS: f64 = 60.0;
const GOLDEN_REWARD_FLAT: f64 = 10.0;
/// Per-click Frenzy bonus: each click during a `Buff::ClickFrenzy` adds
/// `max(FRENZY_FLAT_PER_CLICK, fps * FRENZY_FPS_SECONDS_PER_CLICK)` cuques
/// on top of the regular click power. The FPS-scaled term is what makes
/// late-game Frenzy still feel huge; the flat floor is what keeps an
/// early-game Frenzy from trivializing the cost ladder. Same shape as
/// Lucky's reward formula, but per-click instead of per-catch.
///
/// 5 seconds of FPS per click × ~30 clicks in the 13s buff = ~150 seconds
/// of FPS in 13 seconds = ~12× normal income rate during the buff. Real
/// boost without breaking the early game (where FPS ≈ 0 and the floor
/// caps each click at 10 cuques regardless of how fast you spam).
const FRENZY_FPS_SECONDS_PER_CLICK: f64 = 5.0;
const FRENZY_FLAT_PER_CLICK: f64 = 10.0;

/// Visual flavor for a particle. Drives color/weight in the renderer; the
/// motion model (rise + horizontal drift) is identical across kinds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    /// Default `+1` from a normal click — white→red fade.
    Click,
    /// High-power click (Frenzy active, big upgrade mults). Bold +
    /// warm-yellow accent so it stands out from a swarm of `+1`s.
    ClickBig,
    /// Auto-fingerer income particle.
    Auto,
    /// Golden-catch label ("FRENZY!", "+1.2k", etc). Longer life,
    /// brighter palette.
    Golden,
    /// Bulk-buy confetti pop. Coloured glyphs, shorter than a click.
    Confetti,
}

/// Position is stored as a fraction of the biscuit rect ([0.0, 1.0] on each
/// axis), matching `Powerup`. The renderer resolves these fractions
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
        /// Legacy field, retained for V2/V3 save compatibility but no
        /// longer read by `click_power()`. The per-click Frenzy bonus
        /// is FPS-scaled (see `FRENZY_FPS_SECONDS_PER_CLICK` /
        /// `FRENZY_FLAT_PER_CLICK` in this module). A future V4
        /// migration can drop this field outright; today it just
        /// serializes as 777.0 and gets ignored.
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

    #[serde(default)]
    pub prestige: u64,
    #[serde(default)]
    pub total_play_ticks: u64,
    #[serde(default)]
    pub buffs: Vec<Buff>,

    /// Persistent upgrade-tree state. Source of truth for which procedural
    /// tree nodes the player has bought; `tree_aggregate` (below, derived,
    /// `#[serde(skip)]`) is the cache that the FPS / click / powerup hot
    /// paths actually read.
    #[serde(default)]
    pub tree: UpgradeTreeState,
    /// Pre-folded contributions from every owned tree node. Rebuilt on load
    /// (`migrate_runtime`) and incrementally updated on buy/refund. O(1)
    /// reads on the hot path; bought set can be arbitrarily large.
    #[serde(skip)]
    pub tree_aggregate: TreeAggregate,
    /// Brief green pulse on a node when it gets bought. Ephemeral — purely
    /// render-side feedback, ticks down each frame.
    #[serde(skip)]
    pub tree_buy_flash: HashMap<TreeCoord, u32>,
    /// Brief yellow pulse on nodes that just became reachable as a result
    /// of a buy elsewhere. Highlights the new shop frontier without
    /// requiring the player to scan the canvas.
    #[serde(skip)]
    pub tree_unlock_flash: HashMap<TreeCoord, u32>,
    /// Brief red pulse on a node that just got refunded. The lot is now
    /// unowned so the flash decays on the visible-but-unowned box.
    #[serde(skip)]
    pub tree_refund_flash: HashMap<TreeCoord, u32>,
    /// In-flight "wire energizing" animations from a just-bought node to
    /// each neighbor that flipped reachable on the buy. The destination
    /// box stays gated as not-yet-reachable for the duration of its
    /// incoming anim; when the wavefront reaches the box, the anim is
    /// removed and `tree_unlock_flash` fires for that lot.
    #[serde(skip)]
    pub tree_edge_anims: Vec<EdgeUnlockAnim>,

    #[serde(skip)]
    pub clench_ticks: u32,
    #[serde(skip)]
    pub particles: Vec<Particle>,
    /// Screen-anchored "misclick" tap particles — independent buffer because
    /// they don't follow the biscuit (they're feedback for clicks that
    /// MISSED the biscuit, including the dead zone at low zoom).
    #[serde(skip)]
    pub misclick_particles: Vec<MisclickParticle>,
    /// All currently on-screen powerups, regardless of kind. Multiple of
    /// any kind can coexist — the spawn side has no per-kind slot cap.
    /// Each entry carries a stable `spawn_id`; click hit-test and the `g`
    /// hotkey reference instances by id, never by Vec index. Not
    /// persisted: closing and reopening the game shouldn't preserve frozen
    /// powerup frames.
    #[serde(skip)]
    pub powerups: Vec<Powerup>,
    /// Monotonic counter that mints `Powerup::spawn_id`. Session-scoped
    /// (re-seeded to 0 on every load) — ids only need to be stable within
    /// a single live state, not across restarts.
    #[serde(skip)]
    pub next_spawn_id: u64,
    /// Per-kind inter-arrival cooldown clocks, indexed by `kind as usize`.
    /// Each ticks down independently and rolls fresh from
    /// `powerup::next_cooldown(kind)` on every spawn (including
    /// force-spawns from the dev cheats). Doesn't freeze when the kind
    /// already has on-screen instances — pile-ups self-resolve via the
    /// short lifetime, so the only cost is "another marker on screen for
    /// 11s".
    #[serde(skip)]
    pub powerup_cooldowns: [u32; N_KINDS],
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
    /// Negative-feedback flash: red row pulse when a click hit a row but
    /// `cuques < cost`. One slot per fingerer index.
    #[serde(skip)]
    pub fingerer_unaffordable_flash: Vec<u32>,
    /// "Just became affordable" flash: a brief one-shot green shimmer
    /// fired the tick a row's affordability flips false → true. Distinct
    /// from `*_flash_ticks` (purchase) — shorter duration, no panel
    /// border bleed — so the player can tell "now buyable" apart from
    /// "you just bought."
    #[serde(skip)]
    pub fingerer_unlock_flash: Vec<u32>,
    /// Brief gold shimmer on a fingerer row when a Green Coin catch
    /// targeted it. Closes the visual loop with the floating
    /// `+10% {fingerer}` particle and the green-tinted title-border
    /// pulse — the gold here matches the catch particle, so the player
    /// can see at a glance which row in the sidebar just took the boost.
    #[serde(skip)]
    pub fingerer_green_coin_flash: Vec<u32>,
    /// Previous-tick affordability per fingerer row, used to detect the
    /// false→true edge that triggers `fingerer_unlock_flash`. Sized to
    /// catalog length by `migrate()` and seeded at init from the live
    /// state, so a freshly-loaded save with rows already affordable
    /// doesn't fire a fake unlock flash on tick 1.
    #[serde(skip)]
    pub prev_fingerer_affordable: Vec<bool>,
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
/// to track from the floating `+10% {fingerer}` particle over to the
/// row, short enough that it doesn't outlive the catch event.
pub const GREEN_COIN_ROW_FLASH_TICKS: u32 = TICK_HZ * 2; // 2.0s at 20Hz
/// Permanent AddPercent the Green Coin attaches on catch. Tunable; bumping
/// it changes the long-term power curve significantly so treat with care.
pub const GREEN_COIN_ADD_PERCENT: f64 = 0.10;

/// Fraction of a tree node's original cost returned on refund. The
/// remaining fraction is the "exploration tax" — without this gap,
/// buy/refund is a zero-cost move and the player can spam every
/// combination for free. 0.70 = 30% loss on each refund, enough to make
/// reckless paths expensive without punishing genuine course corrections.
pub const TREE_REFUND_FRACTION: f64 = 0.70;

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
            prestige: 0,
            total_play_ticks: 0,
            buffs: Vec::new(),
            tree: {
                let mut t = UpgradeTreeState::default();
                // The (0, 0) lot is the cuque-anchor: rendered as the
                // ass sprite, no primitives, auto-owned at startup. Its
                // king-neighbors are reachable from frame 1 because of
                // the procgen's `anchor_of` guarantee.
                t.bought.insert(TreeCoord::ORIGIN);
                t
            },
            tree_aggregate: TreeAggregate::default(),
            tree_buy_flash: HashMap::new(),
            tree_unlock_flash: HashMap::new(),
            tree_refund_flash: HashMap::new(),
            tree_edge_anims: Vec::new(),
            clench_ticks: 0,
            particles: Vec::new(),
            misclick_particles: Vec::new(),
            powerups: Vec::new(),
            next_spawn_id: 0,
            powerup_cooldowns: {
                let mut cds = [0u32; N_KINDS];
                for kind in PowerupKind::ALL {
                    cds[kind as usize] = powerup::next_cooldown(kind);
                }
                cds
            },
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
            fingerer_unaffordable_flash: vec![0; fingerer::count()],
            fingerer_unlock_flash: vec![0; fingerer::count()],
            fingerer_green_coin_flash: vec![0; fingerer::count()],
            prev_fingerer_affordable: vec![false; fingerer::count()],
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
        // Ensure the anchor (origin) is always owned — pre-V4 saves
        // (and any future shape change that resets `bought`) need it
        // here so the player's starting frontier exists. The anchor has
        // no primitives so adding it to `bought` is harmless even if
        // it's already there.
        self.tree.bought.insert(TreeCoord::ORIGIN);
        // `tree_aggregate` is also `#[serde(skip)]` — rebuild from `tree.bought`.
        self.tree_aggregate.rebuild_from_bought(&self.tree.bought);
        // Per-catalog flash slots are runtime-only — re-size if the catalog
        // grew/shrank since this save was written.
        if self.fingerer_flash_ticks.len() != fingerer::count() {
            self.fingerer_flash_ticks = vec![0; fingerer::count()];
        }
        if self.fingerer_unaffordable_flash.len() != fingerer::count() {
            self.fingerer_unaffordable_flash = vec![0; fingerer::count()];
        }
        if self.fingerer_unlock_flash.len() != fingerer::count() {
            self.fingerer_unlock_flash = vec![0; fingerer::count()];
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
        // Re-seed any per-kind cooldown left at 0 (the array is
        // `#[serde(skip)]` so it's already at default after deserialize;
        // this is a defensive guard against saves that walked through an
        // older shape that had per-cooldown layout drift).
        for kind in PowerupKind::ALL {
            let i = kind as usize;
            if self.powerup_cooldowns[i] == 0 {
                self.powerup_cooldowns[i] = powerup::next_cooldown(kind);
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
        // Frenzy click (FPS-scaled bonus, often hundreds-to-millions) or
        // any bulk jump does.
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
        // Click contributions come from the tree exclusively (old hardcoded
        // ClickMult upgrades are retired). Same fold order as the modifier
        // formula on fingerers: flat first, then (1 + add_percent), then
        // mul_factor.
        let t = &self.tree_aggregate;
        let base = (1.0 + t.click_flat) * (1.0 + t.click_add) * t.click_mul;
        // Per-click Frenzy bonus, FPS-scaled. The legacy `mult: 777.0`
        // on `Buff::ClickFrenzy` is now ignored at click-time (kept on
        // the struct only for V2/V3/V4 save compatibility); each click
        // during Frenzy adds a flat-or-fps-scaled bonus instead. This
        // is what keeps an early-game Frenzy (FPS ≈ 0) capped at the
        // FRENZY_FLAT floor (10 cuques/click) instead of trivializing
        // the cost ladder via ×777, while a late-game Frenzy still
        // delivers a ~12× income-rate boost during the buff via the
        // FRENZY_FPS_SECONDS_PER_CLICK term.
        let frenzy_active = self
            .buffs
            .iter()
            .any(|b| matches!(b, Buff::ClickFrenzy { .. }));
        if frenzy_active {
            let bonus = (self.fps() * FRENZY_FPS_SECONDS_PER_CLICK).max(FRENZY_FLAT_PER_CLICK);
            base + bonus
        } else {
            base
        }
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

    /// Mint a fresh, monotonic spawn id. Session-scoped — wraps around at
    /// `u64::MAX` (would take ~5 trillion years at one spawn per nanosecond,
    /// so wrap collision isn't a real concern).
    pub fn mint_spawn_id(&mut self) -> u64 {
        let id = self.next_spawn_id;
        self.next_spawn_id = self.next_spawn_id.wrapping_add(1);
        id
    }

    /// Catch the powerup with the given `spawn_id`, if it's still on
    /// screen. Applies the kind-specific effect, increments
    /// `golden_caught` (lifetime grand total — keeps the existing
    /// achievements working) plus the per-kind counter, and returns the
    /// flat reward (only Lucky is non-zero).
    ///
    /// The Vec is unbounded; multiple powerups of the same kind can
    /// coexist. Catching one never disturbs the others — `swap_remove`
    /// is fine because input routes by `spawn_id`, not by Vec index.
    ///
    /// Per-kind cooldown is NOT touched here — it ticks independently
    /// from spawns; a catch just removes the on-screen instance.
    pub fn catch_powerup(&mut self, spawn_id: u64) -> f64 {
        let Some(idx) = self.powerups.iter().position(|p| p.spawn_id == spawn_id) else {
            return 0.0;
        };
        let p = self.powerups.swap_remove(idx);
        self.golden_caught += 1;
        let (reward, label) = match p.kind {
            PowerupKind::Lucky => {
                self.lucky_caught += 1;
                let fps = self.fps();
                let reward_mul = self
                    .tree_aggregate
                    .powerup_reward_mul
                    .get(PowerupKind::Lucky as usize)
                    .copied()
                    .unwrap_or(1.0);
                let r = ((fps * GOLDEN_REWARD_SECONDS).max(GOLDEN_REWARD_FLAT)) * reward_mul;
                self.add_cuques(r);
                self.lucky_flash_ticks = LUCKY_FLASH_TICKS;
                self.cuques_flash_ticks = HUD_FLASH_TICKS;
                (r, format!("+{}", crate::format::big(r)))
            }
            PowerupKind::Frenzy => {
                self.frenzy_caught += 1;
                let duration_mul = self
                    .tree_aggregate
                    .powerup_duration_mul
                    .get(PowerupKind::Frenzy as usize)
                    .copied()
                    .unwrap_or(1.0);
                let dur = ((TICK_HZ * 13) as f64 * duration_mul).round() as u32;
                // `mult: 777.0` is a legacy field — `click_power()` no
                // longer reads it. The per-click Frenzy bonus is FPS-
                // scaled via `FRENZY_FPS_SECONDS_PER_CLICK` /
                // `FRENZY_FLAT_PER_CLICK`. Kept on the struct so V2/V3
                // saves with this field deserialize cleanly.
                self.buffs.push(Buff::ClickFrenzy {
                    ticks_remaining: dur,
                    initial_ticks: dur,
                    mult: 777.0,
                });
                (0.0, "FRENZY!".into())
            }
            PowerupKind::Buff => {
                self.buff_caught += 1;
                let duration_mul = self
                    .tree_aggregate
                    .powerup_duration_mul
                    .get(PowerupKind::Buff as usize)
                    .copied()
                    .unwrap_or(1.0);
                let reward_mul = self
                    .tree_aggregate
                    .powerup_reward_mul
                    .get(PowerupKind::Buff as usize)
                    .copied()
                    .unwrap_or(1.0);
                let dur = ((TICK_HZ * 60) as f64 * duration_mul).round() as u32;
                let m = Modifier {
                    source: ModifierSource::PurpleCoin,
                    effects: vec![ModifierEffect::MulFactor(7.0 * reward_mul)],
                    duration: ModifierDuration::Ticks(dur),
                    created_at_tick: self.total_play_ticks,
                };
                // Fall back to the first catalog tier if the player owns
                // nothing yet — same defensive behavior the legacy buff
                // had: a x7 mul on count=0 is wasted, so just attach
                // somewhere visible to keep the modifier system honest.
                if self.attach_modifier_random_owned(m.clone()).is_none() {
                    let pick = FINGERERS[0].id;
                    self.attach_modifier(pick, m);
                }
                (0.0, "BOOSTED x7!".into())
            }
            PowerupKind::GreenCoin => {
                self.green_coin_caught += 1;
                self.green_coin_flash_ticks = GREEN_COIN_FLASH_TICKS;
                let strength = GREEN_COIN_ADD_PERCENT * self.tree_aggregate.green_coin_strength_mul;
                let m = Modifier {
                    source: ModifierSource::GreenCoin,
                    effects: vec![ModifierEffect::AddPercent(strength)],
                    duration: ModifierDuration::Permanent,
                    created_at_tick: self.total_play_ticks,
                };
                // Visible-set targeting: a permanent +10% can land on a
                // sidebar-visible tier the player hasn't bought yet, so
                // they get a head start when they finally afford it.
                let chosen = self.attach_modifier_random_visible(m);
                // `attach_modifier_random_visible` is only `None` if the
                // visible set is empty, which requires both an empty
                // `FINGERERS` catalog AND `lifetime_cuques < 0.5 * cost`
                // for every tier. Index Finger is always visible
                // (`fingerer::visible` short-circuits on `idx == 0`), so
                // as long as `FINGERERS` is non-empty (currently 10
                // entries, asserted as a project invariant in CLAUDE.md)
                // this branch can't fire. Fail loud in dev so we notice
                // if that invariant ever shifts.
                debug_assert!(
                    chosen.is_some(),
                    "Green Coin catch found no visible fingerer — Index Finger should always be visible"
                );
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
                    // Defensive fallback for release builds — render a
                    // neutral marker rather than panic if the invariant
                    // ever breaks. The debug_assert above catches it in
                    // dev.
                    None => "+10% ???".to_string(),
                };
                (0.0, label)
            }
        };
        self.particles.push(Particle {
            frac_x: p.frac_x,
            frac_y: p.frac_y,
            life: PARTICLE_LIFE * 2,
            text: label,
            kind: ParticleKind::Golden,
            drift_x: 0.0,
        });
        reward
    }

    pub fn fps(&self) -> f64 {
        // Per-fingerer formula:
        //   flat_total = modifier.flat + tree.per_fingerer.flat + tree.all_fingerers.flat
        //   add_total  = modifier.add  + tree.per_fingerer.add  + tree.all_fingerers.add
        //   mul_total  = modifier.mul  * tree.per_fingerer.mul  * tree.all_fingerers.mul
        //   pre  = base * count + flat_total
        //   post = pre * (1 + add_total) * mul_total
        // Both aggregates are pre-folded caches — O(1) reads per fingerer.
        let base: f64 = FINGERERS
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let count = self.fingerer_count(k.id) as f64;
                let mod_agg = self.fingerer_aggregate(k.id);
                let tree = self.tree_aggregate.effective_for_fingerer(i);
                let flat_total = mod_agg.flat_fps + tree.flat_fps;
                let add_total = mod_agg.add_percent + tree.add_percent;
                let mul_total = mod_agg.mul_factor * tree.mul_factor;
                let pre = k.fps_per_unit * count + flat_total;
                pre * (1.0 + add_total) * mul_total
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
        let base = 1.0 + 0.01 * self.prestige as f64;
        let t = &self.tree_aggregate;
        (base + t.prestige_add) * t.prestige_mul
    }

    pub fn prestige_earned_total(&self) -> u64 {
        // Guard the only failure modes the math actually has: NaN /
        // INFINITY from a corrupted `lifetime_cuques`, and negative
        // values that have no business in a lifetime counter. Past
        // that, trust the player's number — long-haul legit play can
        // reach surprisingly high paper counts, and capping the result
        // would silently truncate their earned progression. The f64-
        // to-u64 saturation cast only kicks in around `raw > u64::MAX
        // ≈ 1.8e19` which corresponds to `lifetime_cuques > ~3e44` —
        // unreachable through any non-corrupted save state.
        let raw = (self.lifetime_cuques / 1_000_000.0).sqrt().floor();
        if !raw.is_finite() || raw < 0.0 {
            0
        } else {
            raw as u64
        }
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
        // Tree is also a run-state — prestige resets it fully. Bought set
        // empties; aggregate snaps back to identity.
        self.tree.bought.clear();
        // Re-seed the anchor so the player's frontier exists on the
        // very next tick of the post-prestige run.
        self.tree.bought.insert(TreeCoord::ORIGIN);
        self.tree.cursor = TreeCoord::ORIGIN;
        self.tree.last_bought = None;
        self.tree_aggregate.reset();
        self.tree_buy_flash.clear();
        self.tree_unlock_flash.clear();
        self.tree_refund_flash.clear();
        self.tree_edge_anims.clear();
        self.buffs.clear();
        self.visual_debt = 0.0;
        self.particles.clear();
        self.misclick_particles.clear();
        self.powerups.clear();
        self.next_spawn_id = 0;
        self.clench_ticks = 0;
        // Fresh per-kind cooldowns so the new run has its own independent
        // rhythm from tick 1.
        for kind in PowerupKind::ALL {
            self.powerup_cooldowns[kind as usize] = powerup::next_cooldown(kind);
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
        for t in self.fingerer_unaffordable_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.fingerer_unlock_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        for t in self.fingerer_green_coin_flash.iter_mut() {
            *t = t.saturating_sub(1);
        }
        // Tree-node flash maps. Saturating-sub then drop zeros — keeps
        // the maps from growing unboundedly over a long session.
        self.tree_buy_flash.retain(|_, t| {
            *t = t.saturating_sub(1);
            *t > 0
        });
        self.tree_unlock_flash.retain(|_, t| {
            *t = t.saturating_sub(1);
            *t > 0
        });
        self.tree_refund_flash.retain(|_, t| {
            *t = t.saturating_sub(1);
            *t > 0
        });
        // Edge-unlock anims: tick each one's wavefront forward. When the
        // head reaches the path length, the anim is done — drop it and
        // fire the destination box's unlock_flash so the player gets the
        // familiar gold pulse to punctuate arrival.
        let mut just_unlocked: Vec<TreeCoord> = Vec::new();
        // Prune anims whose path geometry went stale BEFORE bumping ticks,
        // so a now-invalid anim doesn't linger one extra tick gating the
        // destination as `tree_unlock_pending`.
        self.tree_edge_anims
            .retain(|a| !node::edge_path_cells(a.from, a.to).is_empty());
        for anim in &mut self.tree_edge_anims {
            anim.ticks = anim.ticks.saturating_add(1);
        }
        self.tree_edge_anims.retain(|a| {
            let path = node::edge_path_cells(a.from, a.to);
            if path.is_empty() {
                return false;
            }
            // `edge_path_cells` returns the canonical lo→hi-ordered path;
            // figure out which end of it is the anim's source (anim.from)
            // and count the leading-inside-source / trailing-inside-dest
            // cells against THIS path. Wave is done when the visible
            // advance has crossed the visible-cell stretch between the
            // two box silhouettes.
            let from_at_start = (a.from.x, a.from.y) <= (a.to.x, a.to.y);
            let Some(from_spec) = node::node_at(a.from.x, a.from.y) else {
                return false;
            };
            let Some(to_spec) = node::node_at(a.to.x, a.to.y) else {
                return false;
            };
            let (source_leading, dest_trailing) = if from_at_start {
                (
                    count_leading_in_rect(
                        &path,
                        from_spec.box_x,
                        from_spec.box_y,
                        from_spec.box_w,
                        from_spec.box_h,
                    ),
                    count_trailing_in_rect(
                        &path,
                        to_spec.box_x,
                        to_spec.box_y,
                        to_spec.box_w,
                        to_spec.box_h,
                    ),
                )
            } else {
                (
                    count_trailing_in_rect(
                        &path,
                        from_spec.box_x,
                        from_spec.box_y,
                        from_spec.box_w,
                        from_spec.box_h,
                    ),
                    count_leading_in_rect(
                        &path,
                        to_spec.box_x,
                        to_spec.box_y,
                        to_spec.box_w,
                        to_spec.box_h,
                    ),
                )
            };
            let visible_len = path
                .len()
                .saturating_sub(source_leading)
                .saturating_sub(dest_trailing);
            if a.visible_advance() >= visible_len {
                if a.gates_destination {
                    just_unlocked.push(a.to);
                }
                false
            } else {
                true
            }
        });
        for to in just_unlocked {
            self.tree_unlock_flash.insert(to, UNLOCK_FLASH_TICKS);
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
        // keep the immutable reads (`can_buy`) cleanly separated from the
        // mutable writes to the flash + prev vecs.
        let fingerer_now: Vec<bool> = (0..fingerer::count()).map(|i| self.can_buy(i)).collect();
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

    /// Tick every on-screen powerup down by one frame and decrement every
    /// per-kind cooldown. Expired entries (those that just hit 0) are
    /// dropped in place via `retain_mut`. Cooldowns tick **independently**
    /// of on-screen instances — multiple of the same kind can coexist, so
    /// freezing the clock while occupied would block the parallelism this
    /// refactor exists to enable.
    pub fn tick_powerups(&mut self) {
        self.powerups.retain_mut(|p| {
            if p.life_ticks == 0 {
                false
            } else {
                p.life_ticks -= 1;
                true
            }
        });
        for cd in self.powerup_cooldowns.iter_mut() {
            *cd = cd.saturating_sub(1);
        }
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
        // Tree cost-mul (procgen `CostMul` primitives) folds in here so
        // discounts/inflation behave the same for display, gate, and spend.
        let raw = k.base_cost * k.cost_scale.powi(self.fingerer_count_idx(idx) as i32);
        let cost_mul = self
            .tree_aggregate
            .per_fingerer
            .get(idx)
            .map(|c| c.cost_mul)
            .unwrap_or(1.0);
        (raw * cost_mul).floor().max(1.0)
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
    fn flash_purchase_fingerer(&mut self, idx: usize, bought: u32) {
        if bought == 0 {
            return;
        }
        // 1 → 1.0, 10 → 1.7, 50 → 2.5, capped at 3.0. sqrt-style growth so
        // a max-buy is dramatic but doesn't blow the eardrums.
        let strength = (1.0 + ((bought as f32) / 10.0).sqrt()).clamp(1.0, 3.0);
        self.trigger_purchase_flash(strength);
        if let Some(slot) = self.fingerer_flash_ticks.get_mut(idx) {
            *slot = PURCHASE_FLASH_TICKS;
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

    pub fn buy(&mut self, idx: usize) -> bool {
        if self.buy_one_quiet(idx) {
            self.flash_purchase_fingerer(idx, 1);
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
            self.flash_purchase_fingerer(idx, bought);
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
            self.flash_purchase_fingerer(idx, bought);
        }
        bought
    }

    // -- Tree purchase / refund --------------------------------------------

    /// True iff `lot` is reachable from the player's owned set: either it
    /// already neighbors an owned node, or `lot` is the origin (the
    /// player's starting position, always reachable). Used as the
    /// "prereq met" gate alongside cost affordability.
    pub fn tree_reachable(&self, lot: TreeCoord) -> bool {
        if lot == TreeCoord::ORIGIN {
            return true;
        }
        for n in node::neighbors_of(lot) {
            if self.tree.bought.contains(&n) && node::edge_exists(lot, n) {
                return true;
            }
        }
        false
    }

    /// True while `lot` has at least one in-flight edge-unlock animation
    /// converging on it — i.e. a wavefront is still walking the connecting
    /// path. The render and buy paths gate "is this lot reachable yet?"
    /// through this so the player can't click through a node before its
    /// path finishes lighting up.
    pub fn tree_unlock_pending(&self, lot: TreeCoord) -> bool {
        self.tree_edge_anims
            .iter()
            .any(|a| a.to == lot && a.gates_destination)
    }

    /// True iff the player can buy the node at `lot` right now: it exists,
    /// isn't already owned, is reachable, and they can afford it.
    pub fn can_buy_tree_node(&self, lot: TreeCoord) -> bool {
        if self.tree.bought.contains(&lot) {
            return false;
        }
        let Some(node) = node::node_at(lot.x, lot.y) else {
            return false;
        };
        if !self.tree_reachable(lot) {
            return false;
        }
        if self.tree_unlock_pending(lot) {
            return false;
        }
        self.affordable_cuques() >= node.cost
    }

    /// Buy the node at `lot`. Returns the bought `NodeSpec` on success, or
    /// `None` on any rejection (no node, already owned, not reachable, or
    /// not affordable). Subtracts cost, marks owned, folds the node's
    /// primitives into `tree_aggregate`.
    pub fn buy_tree_node(&mut self, lot: TreeCoord) -> Option<NodeSpec> {
        let node = node::node_at(lot.x, lot.y)?;
        if self.tree.bought.contains(&lot) {
            return None;
        }
        if !self.tree_reachable(lot) {
            return None;
        }
        // Reject while the lot still has an in-flight wavefront — the
        // player must wait for the path to finish energizing before they
        // can buy the destination.
        if self.tree_unlock_pending(lot) {
            return None;
        }
        if self.affordable_cuques() < node.cost {
            return None;
        }
        // Snapshot reachability of neighbors BEFORE marking the lot owned,
        // so post-buy we can flash only the lots that flipped false→true
        // on this buy (vs. ones that already had another owned pathway).
        let neighbors = node::neighbors_of(lot);
        let was_reachable: [bool; 8] = std::array::from_fn(|i| self.tree_reachable(neighbors[i]));

        self.cuques -= node.cost;
        self.tree.bought.insert(lot);
        self.tree.last_bought = Some(lot);
        self.tree_aggregate.fold_in_node(&node);
        self.trigger_purchase_flash(1.5);
        self.cuques_spend_flash_ticks = HUD_FLASH_TICKS;
        self.tree_buy_flash.insert(lot, PURCHASE_FLASH_TICKS);

        // Animate the edge from this lot to EVERY procedural neighbor
        // that isn't already owned — `gates_destination` distinguishes
        // newly-reachable neighbors (which gate purchase + fire the
        // gold unlock_flash on completion) from already-reachable ones
        // (which just get the decorative wire-up animation).
        for (i, n) in neighbors.into_iter().enumerate() {
            if self.tree.bought.contains(&n) {
                continue;
            }
            if node::node_at(n.x, n.y).is_none() {
                continue;
            }
            if !node::edge_exists(lot, n) {
                continue;
            }
            self.tree_edge_anims.push(EdgeUnlockAnim {
                from: lot,
                to: n,
                ticks: 0,
                gates_destination: !was_reachable[i],
            });
        }
        Some(node)
    }

    /// True iff refunding `lot` would not orphan any other owned node from
    /// the origin. A node is orphaned if it can no longer reach the origin
    /// via owned king-neighbors with existing edges.
    pub fn can_refund_tree_node(&self, lot: TreeCoord) -> bool {
        if !self.tree.bought.contains(&lot) {
            return false;
        }
        // Origin itself cannot be refunded — it's the anchor.
        if lot == TreeCoord::ORIGIN {
            return false;
        }
        // BFS from origin through owned ∖ {lot}; if every other owned node
        // is reachable, the refund is safe.
        if self.tree.bought.len() <= 1 {
            return true;
        }
        let mut seen: HashSet<TreeCoord> = HashSet::new();
        let mut stack: Vec<TreeCoord> = vec![TreeCoord::ORIGIN];
        seen.insert(TreeCoord::ORIGIN);
        while let Some(c) = stack.pop() {
            for n in node::neighbors_of(c) {
                if n == lot {
                    continue;
                }
                if !self.tree.bought.contains(&n) {
                    continue;
                }
                if seen.contains(&n) {
                    continue;
                }
                if !node::edge_exists(c, n) {
                    continue;
                }
                seen.insert(n);
                stack.push(n);
            }
        }
        // We need every owned-node-except-lot to be reachable.
        for owned in &self.tree.bought {
            if *owned == lot {
                continue;
            }
            if !seen.contains(owned) {
                return false;
            }
        }
        true
    }

    /// Refund the node at `lot`. Returns the amount of cuques returned on
    /// success, or 0.0 on rejection. Refund returns
    /// `cost * TREE_REFUND_FRACTION` — the remaining fraction is the
    /// exploration tax (see the constant for rationale). Connectivity
    /// guard: rejects if it would orphan any other owned node.
    pub fn refund_tree_node(&mut self, lot: TreeCoord) -> f64 {
        if !self.can_refund_tree_node(lot) {
            return 0.0;
        }
        let Some(node) = node::node_at(lot.x, lot.y) else {
            // Ghost lot in `bought` (e.g. survived a procgen change that
            // moved its spec to `None`). Clean it out of the set so the
            // user doesn't end up with stuck phantom-owned entries, and
            // pay no cuques back since we can't compute the refund.
            self.tree.bought.remove(&lot);
            if self.tree.last_bought == Some(lot) {
                self.tree.last_bought = None;
            }
            return 0.0;
        };
        self.tree.bought.remove(&lot);
        if self.tree.last_bought == Some(lot) {
            self.tree.last_bought = None;
        }
        self.tree_aggregate.fold_out_node(&node);
        let refunded = (node.cost * TREE_REFUND_FRACTION).floor();
        self.cuques += refunded;
        // Red pulse on the now-unowned lot. The lot still renders (as an
        // unowned reachable/dotted box depending on connectivity), so the
        // flash decays visibly there.
        self.tree_refund_flash.insert(lot, PURCHASE_FLASH_TICKS);
        // Same green-flash channel as a powerup catch — cuques are flowing
        // back to the player.
        self.cuques_flash_ticks = HUD_FLASH_TICKS;
        refunded
    }
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
        let mut state = GameState {
            fingerers_state: [("index_finger".to_string(), fs_with_count(9))]
                .into_iter()
                .collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };
        state.tree.bought.insert(TreeCoord::ORIGIN);

        let m = state.migrate_runtime();

        assert_eq!(m.fingerer_count("index_finger"), 9);
        assert!(m.tree.bought.contains(&TreeCoord::ORIGIN));
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
    }

    #[test]
    fn save_roundtrip_is_stable_through_json() {
        // Serialize → deserialize → get the same state back. Catches any
        // accidental rename that would make saves non-idempotent.
        let mut state = GameState {
            cuques: 1234.5,
            total_clicks: 99,
            fingerers_state: [("index_finger".to_string(), fs_with_count(7))]
                .into_iter()
                .collect(),
            achievements_earned: ["first_finger".to_string()].into_iter().collect(),
            ..GameState::default()
        };
        state.tree.bought.insert(TreeCoord::new(2, -1));

        let json = serde_json::to_string(&state).expect("serialize");
        let roundtripped: GameState = serde_json::from_str(&json).expect("deserialize");
        let m = roundtripped.migrate_runtime();

        assert_eq!(m.cuques, 1234.5);
        assert_eq!(m.total_clicks, 99);
        assert_eq!(m.fingerer_count("index_finger"), 7);
        assert!(m.tree.bought.contains(&TreeCoord::new(2, -1)));
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
    fn origin_is_auto_owned_on_default() {
        // The (0, 0) lot is the cuque-anchor — auto-owned at startup so
        // the player's king-neighbor frontier exists from frame 1. Buying
        // it is a no-op (no cost, no primitives), refunding it is rejected.
        let s = GameState::default();
        assert!(s.tree.bought.contains(&TreeCoord::ORIGIN));
        let spec = node::node_at(0, 0).expect("anchor always exists");
        assert!(spec.is_anchor);
        assert!(spec.primitives.is_empty());
        assert_eq!(spec.cost, 0.0);
    }

    #[test]
    fn buy_tree_node_at_origin_is_noop() {
        // Origin is auto-owned in Default, so any attempt to buy it
        // returns None ("already owned"). Cuques aren't spent.
        let mut s = GameState {
            cuques: 1_000_000.0,
            displayed_cuques: 1_000_000.0,
            ..Default::default()
        };
        let pre = s.cuques;
        let bought = s.buy_tree_node(TreeCoord::ORIGIN);
        assert!(bought.is_none(), "origin already owned — buy returns None");
        assert!(s.tree.bought.contains(&TreeCoord::ORIGIN));
        assert_eq!(s.cuques, pre, "no cuques spent on a no-op buy");
    }

    #[test]
    fn refund_origin_is_rejected() {
        // Origin is the anchor — non-refundable regardless of state.
        let mut s = GameState {
            cuques: 1_000_000.0,
            displayed_cuques: 1_000_000.0,
            ..Default::default()
        };
        let pre = s.cuques;
        assert_eq!(s.refund_tree_node(TreeCoord::ORIGIN), 0.0);
        assert!(s.tree.bought.contains(&TreeCoord::ORIGIN));
        assert_eq!(s.cuques, pre, "no cuques returned on a refund-rejection");
    }

    #[test]
    fn refund_returns_only_a_fraction_of_cost() {
        // The exploration tax means buy + refund is a net loss. Without
        // this, the player could spam every node combination for free.
        let mut s = GameState {
            cuques: 1_000_000.0,
            displayed_cuques: 1_000_000.0,
            ..Default::default()
        };
        let pre = s.cuques;
        // Origin is already auto-owned; pick a reachable king-neighbor of
        // origin to buy + refund. The procgen anchor-edge rule guarantees
        // every neighbor has an edge to origin, so at least one neighbor
        // is reachable.
        let neighbor = node::neighbors_of(TreeCoord::ORIGIN)
            .into_iter()
            .find(|n| node::node_at(n.x, n.y).is_some() && node::edge_exists(TreeCoord::ORIGIN, *n))
            .expect("at least one reachable neighbor in the procgen");
        let n_node = s
            .buy_tree_node(neighbor)
            .expect("affordable with 1M cuques");
        let after_buy = s.cuques;
        assert!((after_buy - (pre - n_node.cost)).abs() < 1e-6);

        // Refund: gets back fraction * cost, loses (1 - fraction) * cost.
        let refunded = s.refund_tree_node(neighbor);
        let expected = (n_node.cost * TREE_REFUND_FRACTION).floor();
        assert!((refunded - expected).abs() < 1e-6);
        let after_refund = s.cuques;
        assert!(after_refund > after_buy);
        assert!(
            after_refund < pre,
            "refund must NOT restore full state — exploration tax must show up as a net loss"
        );
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
        s.fingerer_unaffordable_flash.clear();
        let m = s.migrate_runtime();
        assert_eq!(m.fingerer_flash_ticks.len(), fingerer::count());
        assert_eq!(m.fingerer_unaffordable_flash.len(), fingerer::count());
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

    // -- Powerups -----------------------------------------------------------

    use crate::game::powerup::{Powerup, PowerupKind};

    fn fake_powerup(state: &mut GameState, kind: PowerupKind) -> u64 {
        let id = state.mint_spawn_id();
        state.powerups.push(Powerup {
            kind,
            spawn_id: id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: kind.lifetime_ticks(),
        });
        id
    }

    #[test]
    fn catch_green_coin_increments_grand_total_and_per_variant_counter() {
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        let id = fake_powerup(&mut s, PowerupKind::GreenCoin);
        s.catch_powerup(id);
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
        let id = fake_powerup(&mut s, PowerupKind::GreenCoin);

        s.catch_powerup(id);

        assert!(s.powerups.is_empty());
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
        // Visible-set targeting means even on a brand-new save a Green Coin
        // attaches somewhere — Index Finger is always visible (`idx == 0`
        // short-circuits in `fingerer::visible`).
        let mut s = GameState::default();
        let id = fake_powerup(&mut s, PowerupKind::GreenCoin);

        s.catch_powerup(id);

        assert!(s.powerups.is_empty());
        let st = s
            .fingerers_state
            .get(FINGERERS[0].id)
            .expect("modifier landed on Index Finger");
        assert_eq!(st.modifiers.len(), 1);
        assert!((st.aggregate.add_percent - 0.10).abs() < 1e-9);
    }

    #[test]
    fn attach_modifier_random_visible_can_pick_unowned_when_lifetime_unlocks_it() {
        let mut s = GameState {
            lifetime_cuques: 60.0,
            ..Default::default()
        };
        let m = perm_add_percent(0.10);
        let chosen = s.attach_modifier_random_visible(m);
        let id = chosen.expect("at least one visible fingerer always exists");
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
    fn catch_powerup_returns_zero_when_id_unknown() {
        let mut s = GameState::default();
        assert_eq!(s.catch_powerup(9_999), 0.0);
    }

    #[test]
    fn tick_powerups_decrements_lifetime_and_drops_at_zero() {
        let mut s = GameState::default();
        let id = s.mint_spawn_id();
        s.powerups.push(Powerup {
            kind: PowerupKind::GreenCoin,
            spawn_id: id,
            frac_x: 0.5,
            frac_y: 0.5,
            life_ticks: 2,
        });
        // Mirrors the legacy `tick_golden` cadence: each tick decrements
        // by 1; the entry survives the tick that brings life_ticks to 0,
        // and the *next* tick drops it. Same convention the renderer
        // relied on (one final visible frame before disappearance).
        s.tick_powerups();
        assert_eq!(s.powerups[0].life_ticks, 1);
        s.tick_powerups();
        assert_eq!(s.powerups[0].life_ticks, 0);
        s.tick_powerups();
        assert!(s.powerups.is_empty());
    }

    #[test]
    fn green_coin_stacks_additively_on_repeat_catches() {
        // Two Green Coins on the same fingerer = +20%, not +21%.
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        for _ in 0..2 {
            let id = fake_powerup(&mut s, PowerupKind::GreenCoin);
            s.catch_powerup(id);
        }
        let st = s.fingerers_state.get("index_finger").unwrap();
        // RNG randomly picks the only owned fingerer both times.
        assert_eq!(st.modifiers.len(), 2);
        assert!((st.aggregate.add_percent - 0.20).abs() < 1e-9);
    }

    #[test]
    fn prestige_reset_clears_powerup_state() {
        let mut s = GameState {
            lifetime_cuques: 1_000_000_000.0,
            ..Default::default()
        };
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        let _ = fake_powerup(&mut s, PowerupKind::Lucky);
        let _ = fake_powerup(&mut s, PowerupKind::GreenCoin);
        s.prestige_reset();
        assert!(s.powerups.is_empty());
        assert_eq!(s.next_spawn_id, 0);
    }

    #[test]
    fn catch_powerup_only_removes_targeted_id() {
        // Multiple powerups of mixed kinds coexist; catching one by id
        // leaves the others untouched (no Vec-index aliasing).
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        let lucky_id = fake_powerup(&mut s, PowerupKind::Lucky);
        let frenzy_id = fake_powerup(&mut s, PowerupKind::Frenzy);
        let buff_id = fake_powerup(&mut s, PowerupKind::Buff);
        s.catch_powerup(frenzy_id);
        let remaining: Vec<u64> = s.powerups.iter().map(|p| p.spawn_id).collect();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&lucky_id));
        assert!(remaining.contains(&buff_id));
    }

    #[test]
    fn buff_stacks_multiplicatively_on_same_fingerer() {
        // Two Buff catches on one fingerer = MulFactor 7² = 49 for the
        // duration. The modifier system already supports this; assert at
        // this layer that nothing in the catch path caps it.
        let mut s = GameState::default();
        s.fingerers_state
            .insert("index_finger".into(), fs_with_count(1));
        for _ in 0..2 {
            let id = fake_powerup(&mut s, PowerupKind::Buff);
            s.catch_powerup(id);
        }
        let st = s.fingerers_state.get("index_finger").unwrap();
        assert_eq!(st.modifiers.len(), 2);
        assert!((st.aggregate.mul_factor - 49.0).abs() < 1e-9);
    }

    #[test]
    fn mint_spawn_id_is_monotonic() {
        let mut s = GameState::default();
        let a = s.mint_spawn_id();
        let b = s.mint_spawn_id();
        let c = s.mint_spawn_id();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c, 2);
    }

    #[test]
    fn green_coin_catch_always_has_a_target_on_fresh_state() {
        // Index Finger is always visible by the `idx == 0` short-circuit
        // in `fingerer::visible`, so the Green Coin catch path's
        // attach-modifier-random-visible branch never returns None. This
        // test enforces the invariant: the catch's particle label must
        // never be the "+10% ???" fallback for a default game state.
        // (The debug_assert in catch_powerup also guards this in dev,
        // but a unit test catches it in release builds too.)
        let mut s = GameState::default();
        let id = fake_powerup(&mut s, PowerupKind::GreenCoin);
        s.catch_powerup(id);
        // The most recent particle is the catch label.
        let label = &s.particles.last().expect("catch spawns a particle").text;
        assert!(
            !label.contains("???"),
            "GreenCoin catch produced unreachable '+10% ???' fallback: {label}"
        );
        assert!(
            label.starts_with("+10% "),
            "GreenCoin catch label must start with '+10% ', got {label}"
        );
    }

    #[test]
    fn frenzy_click_yield_is_bounded_in_early_game() {
        // Balance regression guard. A single Frenzy on a fresh save
        // (FPS=0, click_power=1) used to mint ~30k cuques across 13s of
        // clicks — enough to unlock 5-6 fingerer tiers, trivializing
        // the cost ladder. With the FPS-scaled per-click bonus, the
        // floor is `FRENZY_FLAT_PER_CLICK` (10/click). 13s ≈ 39 clicks
        // → ~390 cuques cap. That's enough to buy Index Finger and a
        // couple of Tier-1s, but Tier 2+ (cost 1100+) stays gated until
        // FPS comes online.
        let mut s = GameState::default();
        let biscuit = r(0, 0, 40, 20);
        // Activate Frenzy, then simulate 13s × 20Hz = 260 ticks of
        // clicking once per tick (40 clicks at 3 clicks/sec ≈ 39, but
        // we click every tick to be conservative — that's actually
        // 260 clicks if we let it run a full buff lifetime).
        s.buffs.push(Buff::ClickFrenzy {
            ticks_remaining: TICK_HZ * 13,
            initial_ticks: TICK_HZ * 13,
            mult: 777.0,
        });
        // Simulate human click cadence: one click every 5 ticks (4Hz)
        // for the buff's lifetime. That's the fastest human-sustainable
        // tap rate.
        let mut clicks = 0;
        for _ in 0..(TICK_HZ * 13) {
            // Sample every fifth tick.
            if clicks * 5 < s.total_play_ticks as u32 + 5 {
                s.click((20, 10), biscuit);
                clicks += 1;
            }
            s.tick();
        }
        assert!(
            s.cuques < 2_000.0,
            "early-game Frenzy must not blow past ~1k cuques; got {}",
            s.cuques
        );
        // Sanity: the buff did boost something (more than zero clicks
        // worth of base power = 1).
        assert!(
            s.cuques > clicks as f64,
            "Frenzy should still meaningfully boost clicks; got {} from {} clicks",
            s.cuques,
            clicks
        );
    }

    #[test]
    fn frenzy_click_yield_scales_with_fps_late_game() {
        // Late-game contract: at high FPS, each Frenzy click is worth
        // roughly `fps * FRENZY_FPS_SECONDS_PER_CLICK` (5s of FPS per
        // click). Set up a state where fps() returns ~1000 by giving
        // many fingerers, then assert the per-click yield is the
        // scaled term, not the flat floor.
        let mut s = GameState::default();
        // Buy enough mid-tier fingerers to push fps into the thousands.
        // Whole Hand (idx 1) = 1.0 fps each; 2000 of them = 2000 fps.
        // (This skips visibility/cost gating since we mutate the count
        // map directly.)
        s.fingerers_state
            .insert("whole_hand".into(), fs_with_count(2000));
        let fps = s.fps();
        assert!(
            fps > 100.0,
            "test setup expected fps>100, got {fps} — adjust the count if fingerer base changed"
        );
        // Activate Frenzy and click once.
        s.buffs.push(Buff::ClickFrenzy {
            ticks_remaining: TICK_HZ * 13,
            initial_ticks: TICK_HZ * 13,
            mult: 777.0,
        });
        let cuques_before = s.cuques;
        let biscuit = r(0, 0, 40, 20);
        s.click((20, 10), biscuit);
        let yield_per_click = s.cuques - cuques_before;
        // Should be roughly fps * 5.0 + base click_power (no upgrades = 1).
        let expected = fps * FRENZY_FPS_SECONDS_PER_CLICK + 1.0;
        // Floor of FRENZY_FLAT_PER_CLICK doesn't kick in here because
        // fps * 5 > 10.
        assert!(
            (yield_per_click - expected).abs() < expected * 0.01,
            "expected ~{expected}/click at fps={fps}, got {yield_per_click}"
        );
    }

    #[test]
    fn no_frenzy_means_no_bonus() {
        // Negative test: without a Frenzy buff active, click_power is
        // just the base × upgrades — no FPS-scaled bonus added.
        let s = GameState::default();
        // No upgrades, no Frenzy.
        assert_eq!(s.click_power(), 1.0);
    }

    #[test]
    fn catch_powerup_increments_grand_total_for_every_kind() {
        // Achievements like "Golden Touch" gate on `golden_caught`
        // (lifetime grand total). The catch path must bump it
        // regardless of kind, otherwise GreenCoin catches stop counting
        // toward the rollup that pre-V3 saves rely on.
        for kind in PowerupKind::ALL {
            let mut s = GameState::default();
            s.fingerers_state
                .insert("index_finger".into(), fs_with_count(1));
            let id = fake_powerup(&mut s, kind);
            let prior = s.golden_caught;
            s.catch_powerup(id);
            assert_eq!(
                s.golden_caught,
                prior + 1,
                "{kind:?} catch must bump golden_caught"
            );
        }
    }
}
