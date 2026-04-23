pub struct FingererStats {
    /// Stable identifier used as the save-file key. Survives catalog reorders,
    /// insertions, and cosmetic renames — so reshuffling tiers does NOT corrupt
    /// a user's owned counts.
    pub id: &'static str,
    pub icon: &'static str,
    pub base_cost: f64,
    pub cost_scale: f64,
    pub fps_per_unit: f64,
}

pub const FINGERERS: &[FingererStats] = &[
    FingererStats {
        id: "index_finger",
        icon: ">>",
        base_cost: 15.0,
        cost_scale: 1.15,
        fps_per_unit: 0.1,
    },
    FingererStats {
        id: "whole_hand",
        icon: "[]",
        base_cost: 100.0,
        cost_scale: 1.15,
        fps_per_unit: 1.0,
    },
    FingererStats {
        id: "latex_glove",
        icon: "<>",
        base_cost: 1_100.0,
        cost_scale: 1.15,
        fps_per_unit: 8.0,
    },
    FingererStats {
        id: "greek_kiss",
        icon: ":*",
        base_cost: 3_500.0,
        cost_scale: 1.15,
        fps_per_unit: 22.0,
    },
    FingererStats {
        id: "robotic_finger",
        icon: "##",
        base_cost: 12_000.0,
        cost_scale: 1.15,
        fps_per_unit: 47.0,
    },
    FingererStats {
        id: "tentacle",
        icon: "~~",
        base_cost: 130_000.0,
        cost_scale: 1.15,
        fps_per_unit: 260.0,
    },
    FingererStats {
        id: "finger_vortex",
        icon: "@@",
        base_cost: 1_400_000.0,
        cost_scale: 1.15,
        fps_per_unit: 1_400.0,
    },
    FingererStats {
        id: "dimensional_hole",
        icon: "()",
        base_cost: 20_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 7_800.0,
    },
    FingererStats {
        id: "cosmic_finger",
        icon: "**",
        base_cost: 330_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 44_000.0,
    },
    FingererStats {
        id: "hand_of_god",
        icon: "^^",
        base_cost: 5_100_000_000.0,
        cost_scale: 1.15,
        fps_per_unit: 260_000.0,
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
