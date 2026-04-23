use crate::game::state::GameState;

pub type Predicate = fn(&GameState) -> bool;

pub struct AchievementKind {
    /// Stable identifier used as the save-file key. Survives reorders and
    /// renames — flipping a predicate's position in the array never turns an
    /// already-earned achievement back into a locked one.
    pub id: &'static str,
    pub unlocked: Predicate,
}

pub const ACHIEVEMENTS: &[AchievementKind] = &[
    AchievementKind {
        id: "first_finger",
        unlocked: |s| s.total_clicks >= 1,
    },
    AchievementKind {
        id: "warming_up",
        unlocked: |s| s.lifetime_cuques >= 100.0,
    },
    AchievementKind {
        id: "seasoned_fingerer",
        unlocked: |s| s.lifetime_cuques >= 10_000.0,
    },
    AchievementKind {
        id: "cuque_mogul",
        unlocked: |s| s.lifetime_cuques >= 1_000_000.0,
    },
    AchievementKind {
        id: "automation",
        unlocked: |s| s.fingerers_owned_total() >= 1,
    },
    AchievementKind {
        id: "factory_of_fingers",
        unlocked: |s| s.fingerer_count("whole_hand") >= 10,
    },
    AchievementKind {
        id: "latex_enjoyer",
        unlocked: |s| s.fingerer_count("latex_glove") >= 10,
    },
    AchievementKind {
        id: "golden_touch",
        unlocked: |s| s.golden_caught >= 1,
    },
    AchievementKind {
        id: "golden_hoarder",
        unlocked: |s| s.golden_caught >= 10,
    },
    AchievementKind {
        id: "rise_of_the_machines",
        unlocked: |s| s.fingerer_count("robotic_finger") >= 1,
    },
];

pub fn count() -> usize {
    ACHIEVEMENTS.len()
}
