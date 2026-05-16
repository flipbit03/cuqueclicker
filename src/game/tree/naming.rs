//! Procedural name + blurb generation.
//!
//! Names emerge from a grammar conditioned on the rolled primitives
//! (Op + Target). Word lists are inert data — large lists ARE the point,
//! because name diversity scales with `|ADJ| × |NOUN| × patterns`. Aim
//! for 100+ entries in each major pool so two players walking through
//! the same region get distinct flavor on every node.
//!
//! Lists below lean into the project's halfway-crude tone: bodily,
//! sacramental, decay-and-glory imagery. Mix freely; the procgen
//! combines them in ways the author didn't anticipate, which is half
//! the fun.

use crate::game::fingerer::FINGERERS;
use crate::game::powerup::PowerupKind;
use crate::game::tree::primitive::{Op, Primitive, Target};
use crate::game::tree::rng::SplitMix64;

const ADJ: &[&str] = &[
    "Twitchy",
    "Reverent",
    "Sweaty",
    "Furious",
    "Liminal",
    "Resonant",
    "Greasy",
    "Holy",
    "Profane",
    "Hollow",
    "Lurid",
    "Cosmic",
    "Frayed",
    "Tender",
    "Sublime",
    "Bottomless",
    "Throbbing",
    "Crepuscular",
    "Crystalline",
    "Eldritch",
    "Velvet",
    "Fungal",
    "Cobalt",
    "Vermilion",
    "Auric",
    "Pulsing",
    "Sopping",
    "Marbled",
    "Drowsy",
    "Spiteful",
    "Wretched",
    "Languid",
    "Feverish",
    "Slick",
    "Hallowed",
    "Tenebrous",
    "Inverted",
    "Bituminous",
    "Plinking",
    // Expanded set — body / decay / cosmic / sacred / sensual / surreal.
    "Splintered",
    "Gilded",
    "Murmuring",
    "Slathered",
    "Trembling",
    "Glistening",
    "Septic",
    "Ruminant",
    "Ossified",
    "Carnal",
    "Soporific",
    "Lambent",
    "Smoldering",
    "Sinuous",
    "Crooked",
    "Phosphorescent",
    "Spurned",
    "Calcified",
    "Embalmed",
    "Subterranean",
    "Translucent",
    "Whispering",
    "Damp",
    "Heretic",
    "Ecstatic",
    "Twisting",
    "Knurled",
    "Numinous",
    "Diaphanous",
    "Splayed",
    "Glaucous",
    "Iridescent",
    "Mottled",
    "Sallow",
    "Verdant",
    "Acrid",
    "Roiling",
    "Saccharine",
    "Tumescent",
    "Withered",
    "Tortuous",
    "Plangent",
    "Salivating",
    "Veined",
    "Marrowed",
    "Bulging",
    "Quivering",
    "Inveterate",
    "Anointed",
    "Encrusted",
    "Penumbral",
    "Stippled",
    "Fluttering",
    "Pendulous",
    "Crimped",
    "Yielding",
    "Tessellated",
    "Sclerotic",
    "Bovine",
    "Erumpent",
    "Reticulated",
    "Insidious",
    "Vermiform",
    "Argent",
    "Catatonic",
    "Salient",
    "Trundling",
    "Saponaceous",
    "Lucent",
    "Plicate",
    "Lapidary",
    "Tridentine",
    "Mucinous",
    "Plumbeous",
    "Concupiscent",
    "Cavernous",
    "Pliant",
    "Refulgent",
    "Mortal",
    "Lascivious",
    "Brackish",
    "Gibbous",
    "Pellucid",
    "Imploring",
    "Lapsed",
    "Apocryphal",
    "Bilious",
    "Tessellate",
    "Cyclopean",
    "Sublimated",
    "Plinian",
    "Vatic",
    "Lurking",
    "Pendular",
    "Sacerdotal",
    "Foundling",
    "Riant",
    "Sclerous",
    "Beslimed",
    "Beatific",
    "Cunning",
    "Greasewood",
    "Pied",
    "Striated",
    "Tindered",
    "Plinkering",
];

const NOUN_INDEX: &[&str] = &[
    "Jab",
    "Probe",
    "Pulse",
    "Tap",
    "Flick",
    "Stab",
    "Prod",
    "Twitch",
    "Dab",
    "Pinch",
    "Wiggle",
    "Press",
    "Spasm",
    "Tickle",
    "Stroke",
    // Expanded — fidgety, tactile, fingertip-scale verbs/nouns.
    "Poke",
    "Brush",
    "Nudge",
    "Squiggle",
    "Tremor",
    "Glide",
    "Pluck",
    "Strum",
    "Trace",
    "Ripple",
    "Quiver",
    "Trill",
    "Twirl",
    "Pry",
    "Nip",
    "Whisk",
    "Twiddle",
    "Slither",
    "Riffle",
    "Tug",
    "Twang",
    "Diddle",
    "Goose",
    "Frot",
    "Skim",
    "Salute",
    "Buzz",
    "Drum",
    "Tease",
    "Flutter",
    "Sashay",
    "Dart",
    "Skitter",
    "Patter",
    "Beckon",
    "Wisp",
    "Pluckety",
    "Jiggle",
    "Trickle",
    "Glimmer",
    "Wink",
    "Pip",
    "Tinkle",
    "Wisp",
    "Flick-Flick",
];

const NOUN_HAND: &[&str] = &[
    "Cup",
    "Palm",
    "Cradle",
    "Grasp",
    "Sweep",
    "Embrace",
    "Knead",
    "Maul",
    "Clutch",
    "Slap",
    "Stretch",
    "Smear",
    "Greet",
    // Expanded — broader manual gestures, full-hand verbs.
    "Welcome",
    "Glance",
    "Spread",
    "Caress",
    "Wrap",
    "Squeeze",
    "Crush",
    "Heft",
    "Ferry",
    "Snatch",
    "Lift",
    "Wield",
    "Hoist",
    "Bear",
    "Massage",
    "Glove",
    "Mitten",
    "Sleeve",
    "Fist",
    "Open-Palm",
    "Manuscript",
    "Cup-and-Lift",
    "Smother",
    "Heel",
    "Knuckle",
    "Pat",
    "Smack",
    "Spank",
    "Cuff",
    "Slip",
    "Slide",
    "Wend",
    "Beckon",
    "Pet",
    "Cradle-Down",
    "Sweep-Across",
    "Encircle",
];

const NOUN_DEEP: &[&str] = &[
    "Plunge",
    "Descent",
    "Channel",
    "Vortex",
    "Abyss",
    "Burrow",
    "Tunnel",
    "Chasm",
    "Maw",
    "Throat",
    "Hollow",
    "Aperture",
    "Conduit",
    // Expanded — deeper, darker, inward.
    "Cavity",
    "Recess",
    "Pit",
    "Sinkhole",
    "Crevice",
    "Cleft",
    "Notch",
    "Slot",
    "Gap",
    "Gully",
    "Trench",
    "Furrow",
    "Trough",
    "Niche",
    "Crater",
    "Dimple",
    "Funnel",
    "Crack",
    "Crevasse",
    "Reservoir",
    "Cistern",
    "Well",
    "Shaft",
    "Drift",
    "Galley",
    "Lumen",
    "Bore",
    "Sluice",
    "Spume",
    "Gullet",
    "Throat-Deep",
    "Underchasm",
    "Inverse",
    "Yawn",
    "Vent",
    "Sump",
    "Pothole",
    "Drain",
];

const NOUN_DIVINE: &[&str] = &[
    "Benediction",
    "Communion",
    "Vigil",
    "Sermon",
    "Liturgy",
    "Edict",
    "Consecration",
    "Schism",
    "Reckoning",
    "Apotheosis",
    "Doctrine",
    "Rapture",
    "Anointing",
    // Expanded — sacramental, oracular, eschatological.
    "Genesis",
    "Revelation",
    "Catechism",
    "Hymn",
    "Psalm",
    "Vespers",
    "Matins",
    "Compline",
    "Lauds",
    "Tract",
    "Heresy",
    "Paten",
    "Chalice",
    "Reliquary",
    "Censer",
    "Mantra",
    "Veil",
    "Sigil",
    "Talisman",
    "Litany",
    "Beatitude",
    "Eucharist",
    "Acolyte",
    "Tenet",
    "Decree",
    "Mandate",
    "Codex",
    "Saga",
    "Prayer",
    "Invocation",
    "Theophany",
    "Eschaton",
    "Apocalypse",
    "Halo",
    "Crozier",
    "Pontifical",
    "Manticore",
    "Reckoning-Day",
    "Genuflection",
    "Penance",
    "Lustration",
    "Beatification",
    "Sanctifying",
    "Encyclical",
];

const PLACE: &[&str] = &[
    "the Threshold",
    "the Antechamber",
    "the Deep",
    "the Outer Ring",
    "the Inner Sanctum",
    "the Bog",
    "the Crater",
    "the Lower Pit",
    "the Drowsy March",
    "the Garden",
    "the Vestibule",
    "the Crawlspace",
    "the Chord",
    // Expanded — sacred architecture + bodily geography.
    "the Nave",
    "the Altar",
    "the Apse",
    "the Crypt",
    "the Cloister",
    "the Refectory",
    "the Scriptorium",
    "the Catacomb",
    "the Reliquary",
    "the Atrium",
    "the Confessional",
    "the Sacristy",
    "the Narthex",
    "the Ambry",
    "the Reredos",
    "the Pulpit",
    "the Choir-Stall",
    "the Aisle",
    "the Transept",
    "the Hollow",
    "the Vault",
    "the Tabernacle",
    "the Heart",
    "the Marrow",
    "the Coil",
    "the Spire",
    "the Lower Reaches",
    "the Greenwood",
    "the Sump",
    "the Pith",
    "the Bowels",
    "the Heel",
    "the Crook",
    "the Knee",
    "the Yoke",
    "the Throat",
    "the Stair",
    "the Latch",
    "the Hinge",
    "the Cleft",
    "the Furrow",
    "the Long Sigh",
    "the Drown",
    "the Damp",
];

const TITLE: &[&str] = &[
    "Apostle",
    "Witness",
    "Patron",
    "Custodian",
    "Heir",
    "Acolyte",
    "Steward",
    "Pilgrim",
    "Eremite",
    "Curator",
    "Cantor",
    "Reliquary",
    "Suppliant",
    // Expanded — ecclesiastical and visionary roles.
    "Vicar",
    "Prelate",
    "Cardinal",
    "Abbess",
    "Friar",
    "Monk",
    "Nun",
    "Hermit",
    "Anchorite",
    "Ascetic",
    "Sister",
    "Brother",
    "Chaplain",
    "Bishop",
    "Deacon",
    "Lector",
    "Sexton",
    "Verger",
    "Mendicant",
    "Wayfarer",
    "Wanderer",
    "Initiate",
    "Adept",
    "Novice",
    "Disciple",
    "Aspirant",
    "Penitent",
    "Almoner",
    "Devotee",
    "Idolater",
    "Iconoclast",
    "Mystic",
    "Visionary",
    "Oracle",
    "Prophet",
    "Seer",
    "Hierophant",
    "Augur",
    "Sibyl",
    "Confessor",
    "Catechumen",
    "Heretic",
    "Pontiff",
    "Cardinal-Errant",
];

const ORDINAL: &[&str] = &[
    "First",
    "Third",
    "Seventh",
    "Eleventh",
    "Thirteenth",
    "Twentieth",
    "Forty-Second",
    "Eighty-First",
    "Hundredth",
    "Final",
    // Expanded — give the digit-tap names a wider numeric vocabulary.
    "Second",
    "Fifth",
    "Ninth",
    "Tenth",
    "Twelfth",
    "Fifteenth",
    "Twenty-Third",
    "Twenty-Seventh",
    "Thirty-First",
    "Thirty-Sixth",
    "Fortieth",
    "Forty-Fifth",
    "Fifty-First",
    "Sixtieth",
    "Sixty-Sixth",
    "Sixty-Ninth",
    "Seventy-Fifth",
    "Eightieth",
    "Ninetieth",
    "Ninety-Ninth",
    "Hundred-and-First",
    "Thousandth",
    "Last",
    "Penultimate",
    "Antepenultimate",
    "Forsaken",
    "Lost",
    "Unnamed",
    "Forgotten",
    "Unnumbered",
];

const VERB_BANE: &[&str] = &[
    "Renounce",
    "Forfeit",
    "Banish",
    "Forsake",
    "Withhold",
    "Suppress",
    "Surrender",
    "Eclipse",
    "Quiet",
    "Mute",
    // Expanded — verbs of constraint, smothering, undoing.
    "Smother",
    "Stifle",
    "Repress",
    "Inter",
    "Bury",
    "Drown",
    "Choke",
    "Throttle",
    "Strangle",
    "Crush",
    "Snuff",
    "Extinguish",
    "Dampen",
    "Dim",
    "Hush",
    "Silence",
    "Veil",
    "Cloak",
    "Shroud",
    "Constrict",
    "Sequester",
    "Confine",
    "Cage",
    "Bind",
    "Fetter",
    "Restrain",
    "Tame",
    "Cripple",
    "Maim",
    "Disavow",
    "Recant",
    "Abjure",
    "Curtail",
    "Stymie",
];

const VERB_BOON: &[&str] = &[
    "Quicken",
    "Hasten",
    "Embolden",
    "Sanctify",
    "Bestow",
    "Coronate",
    "Endow",
    "Magnify",
    "Bloom",
    "Awaken",
    // Expanded — verbs of elevation, blessing, kindling.
    "Anoint",
    "Hallow",
    "Bless",
    "Consecrate",
    "Crown",
    "Exalt",
    "Elevate",
    "Empower",
    "Enrich",
    "Enliven",
    "Kindle",
    "Spark",
    "Ignite",
    "Stir",
    "Wake",
    "Rouse",
    "Stoke",
    "Animate",
    "Vivify",
    "Glorify",
    "Honor",
    "Garland",
    "Beatify",
    "Embellish",
    "Lavish",
    "Behold",
    "Adorn",
    "Lifted",
    "Apothesize",
    "Quicken-Through",
    "Reify",
    "Suffuse",
];

const KEYSTONE_PREFIX: &[&str] = &[
    "Doctrine of",
    "The Way of",
    "Pact of",
    "Covenant of",
    "Codex of",
    "Canticle of",
    "Heresy of",
    "Catechism of",
    "Tenet of",
    "Sigil of",
    "Decree of",
    // Expanded — invocations that prefix a keystone like a chapter title.
    "Litany of",
    "Saga of",
    "Schism of",
    "Crusade of",
    "Bequest of",
    "Manifesto of",
    "Apologia of",
    "Order of",
    "Rite of",
    "Liturgy of",
    "Confession of",
    "Apotheosis of",
    "Cult of",
    "Sect of",
    "Ritual of",
    "Brotherhood of",
    "Sisterhood of",
    "Mantle of",
    "Burden of",
    "Calling of",
    "Vows of",
    "Eschaton of",
    "Trial of",
    "Reckoning of",
    "Eclipse of",
    "Vow of",
    "Sermon of",
    "Edict of",
    "Vespers of",
    "Hour of",
    "Crucible of",
    "Heel of",
    "Threshold of",
    "Awakening of",
    "Disappearance of",
];

const SUFFIX_BOON: &[&str] = &[
    "Master",
    "Crafter",
    "Bearer",
    "Cantor",
    "Witness",
    "Acolyte",
    "Devotee",
    "Pilgrim",
    "Adept",
    "Initiate",
    "Suppliant",
    "Friend",
    "Disciple",
    "Patron",
    "Steward",
    "Keeper",
    "Speaker",
];

const PREFIX_BANE_DARK: &[&str] = &[
    "Withered",
    "Hollow",
    "Stale",
    "Choked",
    "Smothered",
    "Drowsing",
    "Ashen",
    "Cold",
    "Veiled",
    "Spent",
    "Lapsed",
    "Faltering",
    "Hushed",
    "Wilted",
    "Numb",
];

const PREFIX_BANE_QUIET: &[&str] = &[
    "Pale",
    "Mute",
    "Stilled",
    "Dampened",
    "Soft",
    "Dim",
    "Sober",
    "Subdued",
    "Suspended",
    "Drained",
    "Quenched",
    "Tempered",
    "Reduced",
    "Lessened",
];

fn pick<'a>(rng: &mut SplitMix64, list: &[&'a str]) -> &'a str {
    list[rng.range_usize(list.len())]
}

/// Generate a node title from its rolled primitive stack. Driven entirely by
/// `rng` so the output is byte-identical for the same lot every time.
pub fn make_title(rng: &mut SplitMix64, primitives: &[Primitive]) -> String {
    if primitives.is_empty() {
        return String::from("Empty Reverie");
    }

    let has_bane = primitives.iter().any(|p| p.is_bane());
    let has_boon = primitives.iter().any(|p| !p.is_bane());
    let mixed = has_bane && has_boon;
    let target = primitives[0].target;
    let pool = noun_pool_for_target(target);

    // Keystone-style (mixed-sign): overarching invocation + thematic noun.
    if mixed {
        let pattern = rng.range_usize(6);
        return match pattern {
            0 => format!(
                "{} the {} {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, ADJ),
                pick(rng, pool)
            ),
            1 => format!(
                "{} the {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, NOUN_DIVINE)
            ),
            2 => format!(
                "{} {} of {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, ADJ),
                pick(rng, PLACE)
            ),
            3 => format!(
                "{}: the {} {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, ADJ),
                pick(rng, NOUN_DIVINE)
            ),
            4 => format!(
                "{} the {} {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, ORDINAL),
                pick(rng, pool)
            ),
            _ => format!(
                "{} the {} {}",
                pick(rng, KEYSTONE_PREFIX),
                pick(rng, &["Ascendant", "Descending", "Eternal", "Crooked"][..]),
                pick(rng, NOUN_DIVINE)
            ),
        };
    }

    // Notable-flavor (multi-boon or multi-bane): 2+ primitives, same sign.
    if primitives.len() >= 2 {
        let pattern = rng.range_usize(8);
        return match pattern {
            0 => format!("{} of {}", pick(rng, TITLE), pick(rng, PLACE)),
            1 => format!("{} {}", pick(rng, ADJ), pick(rng, NOUN_DIVINE)),
            2 => format!("{}-{}", pick(rng, ADJ), pick(rng, pool)),
            3 => format!(
                "{} of the {} {}",
                pick(rng, TITLE),
                pick(rng, ADJ),
                pick(rng, pool)
            ),
            4 => format!(
                "the {} {} of {}",
                pick(rng, ADJ),
                pick(rng, pool),
                pick(rng, PLACE)
            ),
            5 => format!("{} {} {}", pick(rng, ADJ), pick(rng, ADJ), pick(rng, pool)),
            6 => format!("{}'s {}", pick(rng, TITLE), pick(rng, NOUN_DIVINE)),
            _ => format!("{} the {}", pick(rng, KEYSTONE_PREFIX), pick(rng, pool)),
        };
    }

    // Single-primitive (Small): light-flavored.
    let p = primitives[0];
    let pattern = rng.range_usize(10);
    if p.is_bane() {
        return match pattern {
            0 => format!("{} the {}", pick(rng, VERB_BANE), pick(rng, pool)),
            1 => format!("{} {}", pick(rng, PREFIX_BANE_DARK), pick(rng, pool)),
            2 => format!("{} {}", pick(rng, PREFIX_BANE_QUIET), pick(rng, pool)),
            3 => format!("{}-{}", pick(rng, PREFIX_BANE_DARK), pick(rng, pool)),
            4 => format!(
                "{} {} {}",
                pick(rng, PREFIX_BANE_QUIET),
                pick(rng, ADJ),
                pick(rng, pool)
            ),
            5 => format!("{} of the {}", pick(rng, pool), pick(rng, PLACE)),
            6 => format!("{} {}", pick(rng, VERB_BANE), pick(rng, NOUN_DIVINE)),
            _ => format!("{} {}", pick(rng, PREFIX_BANE_DARK), pick(rng, NOUN_DIVINE)),
        };
    }
    match pattern {
        0 => format!("{} {}", pick(rng, ADJ), pick(rng, pool)),
        1 => format!("{}-{}", pick(rng, ADJ), pick(rng, SUFFIX_BOON)),
        2 => format!("{} of {}", pick(rng, pool), pick(rng, PLACE)),
        3 => format!("{}-Tap {}", pick(rng, ORDINAL), pick(rng, pool)),
        4 => format!("{} {}", pick(rng, VERB_BOON), pick(rng, pool)),
        5 => format!("{} {}", pick(rng, VERB_BOON), pick(rng, NOUN_DIVINE)),
        6 => format!("{}'s {}", pick(rng, TITLE), pick(rng, pool)),
        7 => format!("{} of the {}", pick(rng, pool), pick(rng, ADJ)),
        8 => format!("{} the {}", pick(rng, VERB_BOON), pick(rng, NOUN_DIVINE)),
        _ => format!("the {} {}", pick(rng, ADJ), pick(rng, pool)),
    }
}

/// Choose a noun word-list keyed to the primitive's target — fingerer
/// biomes get different vocabulary than click / powerup / global biomes.
fn noun_pool_for_target(t: Target) -> &'static [&'static str] {
    match t {
        Target::Fingerer(idx) => fingerer_noun_pool(idx as usize),
        Target::AllFingerers => NOUN_DIVINE,
        Target::Click => NOUN_INDEX,
        Target::PowerupSpawn(_) | Target::PowerupReward(_) | Target::PowerupDuration(_) => {
            NOUN_DIVINE
        }
        Target::Prestige => NOUN_DIVINE,
        Target::GreenCoinStrength => NOUN_DEEP,
    }
}

fn fingerer_noun_pool(idx: usize) -> &'static [&'static str] {
    // Loose mapping from fingerer tier to vocabulary "level". Lower tiers
    // get fidgety/tactile nouns; higher tiers get cosmic / divine ones.
    if idx >= FINGERERS.len() {
        return NOUN_DIVINE;
    }
    match idx {
        0 | 1 => NOUN_INDEX,
        2 | 3 => NOUN_HAND,
        4 | 5 => NOUN_HAND,
        6 | 7 => NOUN_DEEP,
        _ => NOUN_DIVINE,
    }
}

/// Render a short human-readable description of a single primitive. Used by
/// the info pane to show what a node does. Math symbols + magnitude
/// numbers are locale-neutral; only the connector ("to", "cost on", …)
/// and the target label flip per locale.
pub fn primitive_blurb(p: Primitive) -> String {
    use crate::format::{flat_magnitude, mul_magnitude, percent_magnitude};
    let lang = crate::i18n::t();
    let target = target_label(p.target);
    match p.op {
        Op::AddPercent => format!(
            "{} {} {}",
            percent_magnitude(p.magnitude),
            lang.tree_blurb_to,
            target
        ),
        Op::MulFactor => format!(
            "{} {} {}",
            mul_magnitude(p.magnitude),
            lang.tree_blurb_to,
            target
        ),
        Op::FlatAdd => format!(
            "{} {} {}",
            flat_magnitude(p.magnitude),
            lang.tree_blurb_flat_to,
            target
        ),
        Op::CostMul => format!(
            "{} {} {}",
            mul_magnitude(p.magnitude),
            lang.tree_blurb_cost_on,
            target
        ),
        Op::SpawnRateMul => format!(
            "{} {} {}",
            mul_magnitude(p.magnitude),
            lang.tree_blurb_spawn_rate_for,
            target
        ),
        Op::EffectMul => format!(
            "{} {} {}",
            mul_magnitude(p.magnitude),
            lang.tree_blurb_effect_on,
            target
        ),
    }
}

fn target_label(t: Target) -> String {
    let lang = crate::i18n::t();
    match t {
        Target::Fingerer(idx) => fingerer_label(idx as usize),
        Target::AllFingerers => lang.tree_target_all_fingerers.to_string(),
        Target::Click => lang.tree_target_click.to_string(),
        Target::PowerupSpawn(k) => {
            lang.tree_target_powerup_spawn_fmt
                .replacen("{}", powerup_label(k), 1)
        }
        Target::PowerupReward(k) => {
            lang.tree_target_powerup_reward_fmt
                .replacen("{}", powerup_label(k), 1)
        }
        Target::PowerupDuration(k) => {
            lang.tree_target_powerup_duration_fmt
                .replacen("{}", powerup_label(k), 1)
        }
        Target::Prestige => lang.tree_target_prestige.to_string(),
        Target::GreenCoinStrength => lang.tree_target_green_coin_strength.to_string(),
    }
}

fn fingerer_label(idx: usize) -> String {
    crate::i18n::t()
        .fingerer_names
        .get(idx)
        .copied()
        .unwrap_or("?")
        .to_string()
}

fn powerup_label(k: PowerupKind) -> &'static str {
    match k {
        PowerupKind::Lucky => "Lucky",
        PowerupKind::Frenzy => "Frenzy",
        PowerupKind::Buff => "Buff Coin",
        PowerupKind::GreenCoin => "Green Coin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_title_deterministic_for_same_rng() {
        let p = &[Primitive {
            op: Op::AddPercent,
            target: Target::Fingerer(0),
            magnitude: 0.1,
        }];
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        assert_eq!(make_title(&mut a, p), make_title(&mut b, p));
    }

    #[test]
    fn keystone_titles_use_keystone_prefixes() {
        // Mixed sign → keystone-flavored prefix. Run many rolls to assert
        // the prefix family is always one of the keystone openings.
        let ps = &[
            Primitive {
                op: Op::MulFactor,
                target: Target::Fingerer(0),
                magnitude: 3.0,
            },
            Primitive {
                op: Op::MulFactor,
                target: Target::AllFingerers,
                magnitude: 0.5,
            },
        ];
        for seed in 0..50 {
            let mut r = SplitMix64::new(seed);
            let t = make_title(&mut r, ps);
            assert!(
                KEYSTONE_PREFIX.iter().any(|p| t.starts_with(p)),
                "keystone title `{t}` missing keystone prefix"
            );
        }
    }

    #[test]
    fn primitive_blurb_signed_addpercent() {
        let b = primitive_blurb(Primitive {
            op: Op::AddPercent,
            target: Target::Click,
            magnitude: 0.18,
        });
        assert!(b.contains("+18"), "got {b}");
    }
}
