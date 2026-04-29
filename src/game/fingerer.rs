pub struct FingererStats {
    /// Stable identifier used as the save-file key. Survives catalog reorders,
    /// insertions, and cosmetic renames — so reshuffling tiers does NOT corrupt
    /// a user's owned counts.
    pub id: &'static str,
    pub icon: &'static str,
    pub base_cost: f64,
    pub cost_scale: f64,
    pub fps_per_unit: f64,
    /// Multiplier on the orbital "poke" pulse rate in `ui::hands`. Higher =
    /// fast, fidgety pokes (Index Finger); lower = slow, majestic, statelier
    /// motion (Hand of God). Pure animation timing, no game effect — see
    /// `hands::draw` for how it folds into the period divisor.
    pub poke_speed: f32,
}

// Poke-speed scale: 1.0 = baseline pulse rate. Lower tiers are jittery
// (>1.0); higher tiers are slow & majestic (<1.0). The numbers are author
// taste — change freely, no game logic depends on them.
pub const FINGERERS: &[FingererStats] = &[
    FingererStats {
        id: "index_finger",
        icon: ">>",
        base_cost: 15.0,
        cost_scale: 1.15,
        fps_per_unit: 0.1,
        poke_speed: 2.4, // fidgety, twitchy
    },
    FingererStats {
        id: "whole_hand",
        icon: "[]",
        base_cost: 100.0,
        cost_scale: 1.15,
        fps_per_unit: 1.0,
        poke_speed: 1.8,
    },
    FingererStats {
        id: "latex_glove",
        icon: "<>",
        base_cost: 1_100.0,
        cost_scale: 1.15,
        fps_per_unit: 8.0,
        poke_speed: 1.5,
    },
    FingererStats {
        id: "greek_kiss",
        icon: ":*",
        base_cost: 3_500.0,
        cost_scale: 1.15,
        fps_per_unit: 22.0,
        poke_speed: 1.25,
    },
    FingererStats {
        id: "robotic_finger",
        icon: "##",
        base_cost: 12_000.0,
        cost_scale: 1.15,
        fps_per_unit: 47.0,
        poke_speed: 1.0, // baseline (mechanical, even cadence)
    },
    FingererStats {
        id: "tentacle",
        icon: "~~",
        base_cost: 130_000.0,
        cost_scale: 1.15,
        fps_per_unit: 260.0,
        poke_speed: 0.85,
    },
    FingererStats {
        id: "finger_vortex",
        icon: "@@",
        base_cost: 1_400_000.0,
        cost_scale: 1.15,
        fps_per_unit: 1_400.0,
        poke_speed: 0.7,
    },
    FingererStats {
        id: "dimensional_hole",
        icon: "()",
        base_cost: 20_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 7_800.0,
        poke_speed: 0.55,
    },
    FingererStats {
        id: "cosmic_finger",
        icon: "**",
        base_cost: 330_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 44_000.0,
        poke_speed: 0.4,
    },
    FingererStats {
        id: "hand_of_god",
        icon: "^^",
        base_cost: 5_100_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 260_000.0,
        poke_speed: 0.25, // slow, statelier — divine majesty
    },
];

pub fn count() -> usize {
    FINGERERS.len()
}

/// Whether a fingerer should be visible in the sidebar.
/// Show when the player either already owns one, or their lifetime_cuques
/// has reached half the base cost (so they can see what's coming).
pub fn visible(idx: usize, owned: u32, lifetime_cuques: f64) -> bool {
    if idx == 0 || owned > 0 {
        return true;
    }
    let k = &FINGERERS[idx];
    lifetime_cuques >= k.base_cost * 0.5
}
