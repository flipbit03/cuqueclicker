use crate::game::state::GameState;

#[derive(Clone, Copy)]
pub enum UpgradeReq {
    /// Player owns at least `n` of the fingerer identified by this stable id.
    OwnedFingerer(&'static str, u32),
    TotalClicks(u64),
    LifetimeCuques(f64),
}

impl UpgradeReq {
    pub fn met(&self, s: &GameState) -> bool {
        match *self {
            UpgradeReq::OwnedFingerer(id, n) => s.fingerer_count(id) >= n,
            UpgradeReq::TotalClicks(n) => s.total_clicks >= n,
            UpgradeReq::LifetimeCuques(n) => s.lifetime_cuques >= n,
        }
    }
}

#[derive(Clone, Copy)]
// The postfix repetition ("Mult") is intentional — each variant is a kind
// of multiplicative modifier. Renaming would be worse.
#[allow(clippy::enum_variant_names)]
pub enum UpgradeEffect {
    /// Multiplies the output of the fingerer identified by this stable id.
    FingererMult(&'static str, f64),
    ClickMult(f64),
    AllFingerersMult(f64),
}

pub struct UpgradeKind {
    /// Stable identifier used as the save-file key. Survives reorders, cost
    /// rebalancing, and renames — flipping an upgrade's position never turns
    /// an already-earned upgrade back into an unearned one.
    pub id: &'static str,
    pub cost: f64,
    pub req: UpgradeReq,
    pub effect: UpgradeEffect,
}

/// Per-fingerer upgrades: 3 tiers (own 1, own 25, own 50) each doubles output.
/// Click upgrades: 3 tiers (50 / 200 / 1000 clicks) each doubles click power.
/// All-fingerer upgrades: lifetime cuques milestones, small boosts.
///
/// Since migration to stable IDs, array order is purely cosmetic — the save
/// records earned upgrades by id, so reordering this list doesn't affect any
/// existing save.
pub const UPGRADES: &[UpgradeKind] = &[
    UpgradeKind {
        id: "click_mult_1",
        cost: 100.0,
        req: UpgradeReq::TotalClicks(50),
        effect: UpgradeEffect::ClickMult(2.0),
    },
    UpgradeKind {
        id: "click_mult_2",
        cost: 5_000.0,
        req: UpgradeReq::TotalClicks(200),
        effect: UpgradeEffect::ClickMult(2.0),
    },
    UpgradeKind {
        id: "click_mult_3",
        cost: 100_000.0,
        req: UpgradeReq::TotalClicks(1_000),
        effect: UpgradeEffect::ClickMult(2.0),
    },
    UpgradeKind {
        id: "index_finger_mult_1",
        cost: 150.0,
        req: UpgradeReq::OwnedFingerer("index_finger", 1),
        effect: UpgradeEffect::FingererMult("index_finger", 2.0),
    },
    UpgradeKind {
        id: "index_finger_mult_2",
        cost: 1_500.0,
        req: UpgradeReq::OwnedFingerer("index_finger", 25),
        effect: UpgradeEffect::FingererMult("index_finger", 2.0),
    },
    UpgradeKind {
        id: "index_finger_mult_3",
        cost: 15_000.0,
        req: UpgradeReq::OwnedFingerer("index_finger", 50),
        effect: UpgradeEffect::FingererMult("index_finger", 2.0),
    },
    UpgradeKind {
        id: "whole_hand_mult_1",
        cost: 1_000.0,
        req: UpgradeReq::OwnedFingerer("whole_hand", 1),
        effect: UpgradeEffect::FingererMult("whole_hand", 2.0),
    },
    UpgradeKind {
        id: "whole_hand_mult_2",
        cost: 10_000.0,
        req: UpgradeReq::OwnedFingerer("whole_hand", 25),
        effect: UpgradeEffect::FingererMult("whole_hand", 2.0),
    },
    UpgradeKind {
        id: "whole_hand_mult_3",
        cost: 100_000.0,
        req: UpgradeReq::OwnedFingerer("whole_hand", 50),
        effect: UpgradeEffect::FingererMult("whole_hand", 2.0),
    },
    UpgradeKind {
        id: "latex_glove_mult_1",
        cost: 11_000.0,
        req: UpgradeReq::OwnedFingerer("latex_glove", 1),
        effect: UpgradeEffect::FingererMult("latex_glove", 2.0),
    },
    UpgradeKind {
        id: "latex_glove_mult_2",
        cost: 110_000.0,
        req: UpgradeReq::OwnedFingerer("latex_glove", 25),
        effect: UpgradeEffect::FingererMult("latex_glove", 2.0),
    },
    UpgradeKind {
        id: "latex_glove_mult_3",
        cost: 1_100_000.0,
        req: UpgradeReq::OwnedFingerer("latex_glove", 50),
        effect: UpgradeEffect::FingererMult("latex_glove", 2.0),
    },
    UpgradeKind {
        id: "robotic_finger_mult_1",
        cost: 120_000.0,
        req: UpgradeReq::OwnedFingerer("robotic_finger", 1),
        effect: UpgradeEffect::FingererMult("robotic_finger", 2.0),
    },
    UpgradeKind {
        id: "robotic_finger_mult_2",
        cost: 1_200_000.0,
        req: UpgradeReq::OwnedFingerer("robotic_finger", 25),
        effect: UpgradeEffect::FingererMult("robotic_finger", 2.0),
    },
    UpgradeKind {
        id: "robotic_finger_mult_3",
        cost: 12_000_000.0,
        req: UpgradeReq::OwnedFingerer("robotic_finger", 50),
        effect: UpgradeEffect::FingererMult("robotic_finger", 2.0),
    },
    UpgradeKind {
        id: "all_fingerers_boost",
        cost: 1_000_000.0,
        req: UpgradeReq::LifetimeCuques(500_000.0),
        effect: UpgradeEffect::AllFingerersMult(1.5),
    },
    UpgradeKind {
        id: "tentacle_mult_1",
        cost: 1_300_000.0,
        req: UpgradeReq::OwnedFingerer("tentacle", 1),
        effect: UpgradeEffect::FingererMult("tentacle", 2.0),
    },
    UpgradeKind {
        id: "tentacle_mult_2",
        cost: 13_000_000.0,
        req: UpgradeReq::OwnedFingerer("tentacle", 25),
        effect: UpgradeEffect::FingererMult("tentacle", 2.0),
    },
    UpgradeKind {
        id: "tentacle_mult_3",
        cost: 130_000_000.0,
        req: UpgradeReq::OwnedFingerer("tentacle", 50),
        effect: UpgradeEffect::FingererMult("tentacle", 2.0),
    },
    UpgradeKind {
        id: "finger_vortex_mult_1",
        cost: 14_000_000.0,
        req: UpgradeReq::OwnedFingerer("finger_vortex", 1),
        effect: UpgradeEffect::FingererMult("finger_vortex", 2.0),
    },
    UpgradeKind {
        id: "finger_vortex_mult_2",
        cost: 140_000_000.0,
        req: UpgradeReq::OwnedFingerer("finger_vortex", 25),
        effect: UpgradeEffect::FingererMult("finger_vortex", 2.0),
    },
    UpgradeKind {
        id: "finger_vortex_mult_3",
        cost: 1_400_000_000.0,
        req: UpgradeReq::OwnedFingerer("finger_vortex", 50),
        effect: UpgradeEffect::FingererMult("finger_vortex", 2.0),
    },
    UpgradeKind {
        id: "dimensional_hole_mult_1",
        cost: 200_000_000.0,
        req: UpgradeReq::OwnedFingerer("dimensional_hole", 1),
        effect: UpgradeEffect::FingererMult("dimensional_hole", 2.0),
    },
    UpgradeKind {
        id: "dimensional_hole_mult_2",
        cost: 2_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("dimensional_hole", 25),
        effect: UpgradeEffect::FingererMult("dimensional_hole", 2.0),
    },
    UpgradeKind {
        id: "dimensional_hole_mult_3",
        cost: 20_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("dimensional_hole", 50),
        effect: UpgradeEffect::FingererMult("dimensional_hole", 2.0),
    },
    UpgradeKind {
        id: "cosmic_finger_mult_1",
        cost: 3_300_000_000.0,
        req: UpgradeReq::OwnedFingerer("cosmic_finger", 1),
        effect: UpgradeEffect::FingererMult("cosmic_finger", 2.0),
    },
    UpgradeKind {
        id: "cosmic_finger_mult_2",
        cost: 33_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("cosmic_finger", 25),
        effect: UpgradeEffect::FingererMult("cosmic_finger", 2.0),
    },
    UpgradeKind {
        id: "cosmic_finger_mult_3",
        cost: 330_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("cosmic_finger", 50),
        effect: UpgradeEffect::FingererMult("cosmic_finger", 2.0),
    },
    UpgradeKind {
        id: "hand_of_god_mult_1",
        cost: 51_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("hand_of_god", 1),
        effect: UpgradeEffect::FingererMult("hand_of_god", 2.0),
    },
    UpgradeKind {
        id: "hand_of_god_mult_2",
        cost: 510_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("hand_of_god", 25),
        effect: UpgradeEffect::FingererMult("hand_of_god", 2.0),
    },
    UpgradeKind {
        id: "hand_of_god_mult_3",
        cost: 5_100_000_000_000.0,
        req: UpgradeReq::OwnedFingerer("hand_of_god", 50),
        effect: UpgradeEffect::FingererMult("hand_of_god", 2.0),
    },
    UpgradeKind {
        id: "greek_kiss_mult_1",
        cost: 35_000.0,
        req: UpgradeReq::OwnedFingerer("greek_kiss", 1),
        effect: UpgradeEffect::FingererMult("greek_kiss", 2.0),
    },
    UpgradeKind {
        id: "greek_kiss_mult_2",
        cost: 350_000.0,
        req: UpgradeReq::OwnedFingerer("greek_kiss", 25),
        effect: UpgradeEffect::FingererMult("greek_kiss", 2.0),
    },
    UpgradeKind {
        id: "greek_kiss_mult_3",
        cost: 3_500_000.0,
        req: UpgradeReq::OwnedFingerer("greek_kiss", 50),
        effect: UpgradeEffect::FingererMult("greek_kiss", 2.0),
    },
];

pub fn available_ids(state: &GameState) -> Vec<usize> {
    UPGRADES
        .iter()
        .enumerate()
        .filter(|(_, u)| !state.has_upgrade(u.id) && u.req.met(state))
        .map(|(i, _)| i)
        .collect()
}
