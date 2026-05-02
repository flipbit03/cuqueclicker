# CuqueClicker

A TUI parody of Cookie Clicker — you finger an ASCII anus instead of clicking a cookie. Bilingual pun: in Brazilian Portuguese "Cookie Clicker" ≈ "Cu que clicker" ("the ass that clicks"). The Portuguese framing is the whole joke; keep it in mind when making copy/naming calls.

Written in Rust with `ratatui` + `crossterm`. Runs as a single self-contained binary on Linux (x86_64 & aarch64 musl), macOS (aarch64), and Windows (x86_64 MSVC, static CRT). Shipped via GitHub Releases + crates.io.

## Audience & tone

- **README tone is "halfway-crude"**: name the bit (parody, ass, finger a cuque, Portuguese pun) explicitly but don't lean on shock value. Technical sections (Install/Controls/License) stay plain and professional.
- **Public docs are English-only.** pt_BR lives inside the game (auto-detected from `$LANG`); it does not leak into README/CLI help. Don't leave Portuguese words ("Papel de Seda", "Prestígio") in the EN strings — translate them. Each locale is internally consistent.

## Project policies (specific to this repo)

- **Saves must always load cleanly across versions — never break or lose a user's savestate.** Any schema change (renamed field, added field, reordered variant, inserted tier, new serialized sub-state) must be paired with explicit migration code in `GameState::migrate()` that rewrites old saves into the new shape. Design every change with this in mind *before* touching the struct. `serde` aliases and leave-the-old-name-around shims are not the mechanism — do clean renames and absorb the cost in `migrate()`.
- **Catalog state (fingerers / upgrades / achievements) is addressed by stable string IDs, not positional indices.** `GameState` stores `fingerers_owned: HashMap<String, u32>`, `upgrades_earned: HashSet<String>`, `achievements_earned: HashSet<String>`. Each `FINGERERS`/`UPGRADES`/`ACHIEVEMENTS` entry has an `id: &'static str` that MUST stay stable across the lifetime of the game — renaming an id silently orphans every player's progress on that item. Reordering, inserting, or removing entries in these tables is free: unknown ids in a save are ignored (forward-compat), missing ids default to zero/absent (backward-compat). New content = add catalog entries; retired content = remove them. Neither requires a migration.
- **Every migration branch must ship with a unit test.** `#[cfg(test)]` module lives next to `migrate()` in `src/game/state.rs`. Each test constructs an old-shape `GameState` (or deserializes an old JSON fixture), runs `migrate()`, and asserts the resulting counts, ids, and invariants are what the live game expects. Never commit a migration without the test that proves it.
- **MIT licensed.** `LICENSE` carries the standard MIT text; `Cargo.toml` uses `license = "MIT"` (SPDX). If someone wants to fork / vendor / adapt, that's fine.
- **No CHANGELOG.md.** Rely on git log + GitHub Release notes.
- **No code-signing / notarization.** Windows users click through SmartScreen; macOS users run `xattr -d com.apple.quarantine` or right-click→Open. Do not add signing steps without explicit ask.
- **No backward-compat hacks in general** — delete dead code rather than keeping `// removed` markers, don't rename `_unused` vars, don't re-export removed types. (User's global convention, reinforced here.)

## Dev vs release: the two gates

Two independent mechanisms decide whether a binary is "dev":

1. **`Cargo.toml` `version = "0.0.0"`** — pinned in the repo. `release.yml` `sed`-patches it from the git tag at build time. Anything compiled from an unpatched tree reports `0.0.0`.
2. **`build_info::is_dev_build()`** = `VERSION == "0.0.0"`. This, AND NOT the `binary-release` cargo feature, gates dev-only surfaces.

Dev-only surfaces (all require `is_dev_build()`):
- **Debug pane** (F8 / F2 / F3 / F5 force-spawn Lucky / Frenzy / Buff / GreenCoin powerups; F4 gives free cuques). Also requires `!cli.no_debug`. `--no-debug` is opt-**out** on dev; it's labelled "disable" not "hide" because the point is that the cheats are gone, not just invisible.
- **`--demo-for-recording [SECONDS]`** (hidden from `--help`). Runs the auto-driver on an ephemeral rich state.

The `binary-release` cargo feature is a **different** gate — it only toggles how `cuqueclicker self update` re-installs:
- Feature ON (set by `release.yml`) → re-run the installer script (curl+sh / irm+iex).
- Feature OFF (local build, `cargo install`) → `cargo install cuqueclicker --force`.

HUD shows `v0.0.0 (dev)` in dev, plain `vX.Y.Z` in release. The title is built in `src/ui/mod.rs` from `env!("CARGO_PKG_VERSION")`; `i18n::title` is just the bare name.

## Invariants — don't casually break these

- **Catalog `id` strings are load-bearing forever.** Once a `FingererStats`, `UpgradeKind`, or `AchievementKind` ships with an id, renaming that id silently zeroes every existing save's progress on that item. Treat ids like primary keys. Cosmetic names are in i18n and can change freely; ids stay.
- **Exactly 10 fingerers, aligned to hotkeys `1`–`9`, `0`.** Don't add an 11th without also rethinking keybinds. We explicitly dropped the Singularity tier for this reason.
- **`--demo-for-recording` must never touch the save file or the lock.** It runs on `build_demo_state()`, skips `save::acquire_lock()`, and the tick loop early-returns before the save-interval check. Two live sessions (one normal, one demo) must coexist.
- **Single-instance lock uses `std::fs::File::try_lock`** (stdlib, Rust ≥ 1.89). Do **not** reintroduce the `fs4` crate. The `.lock` file on disk is just a handle target — OS releases the lock on process exit, clean or crash. If it ever appears stuck, the fix is "close the other instance or delete the lock file", not new code.
- **Active buffs persist across quit/restart; powerups and per-kind cooldowns don't.** `buffs` is serialized (click-side only — `Buff::ClickFrenzy`); the powerup Vec, `next_spawn_id`, and `powerup_cooldowns` are all `#[serde(skip)]` and re-seeded on load (each cooldown rolls a fresh exponential sample in `Default`/`migrate_runtime`). Per-fingerer modifiers (Purple Coin, Green Coin, future events) live on `FingererState.modifiers` and ARE persisted; the `aggregate` cache they drive is `#[serde(skip)]` and rebuilt by `migrate_runtime`.
- **Per-fingerer modifier source ids are load-bearing forever.** `ModifierSource::id()` strings (`green_coin`, `purple_coin`, ...) appear in serialized saves. Renaming silently invalidates every existing save's modifier list. Treat them like fingerer/upgrade ids: cosmetic names live in `i18n.rs`; the id stays stable.

## Save versioning

The on-disk format is dispatched by a `version: u32` field at the top of every save. Pre-versioned saves (everything written by `main` before this branch) are V1 by definition — `crate::save::migrate::peek_version` returns 1 when the field is absent.

Layout:

```
src/save/
├── mod.rs              # public API: load_from_str / save_to_string + CURRENT_VERSION
├── migrate.rs          # peek_version(json) -> u32
└── versions/
    ├── mod.rs
    ├── v1.rs           # FROZEN: pre-versioned shape
    ├── v2.rs           # FROZEN: per-fingerer modifiers, Green Coin pity counter
    ├── v3.rs           # FROZEN once shipped: drops `goldens_since_green_coin`
    └── ...
```

Loading is a chain: `peek_version → deserialize as GameStateVN → fold through From impls → into_current() → migrate_runtime()`. `migrate_runtime` is **runtime-only** — it seeds ephemeral `#[serde(skip)]` fields (flash vecs, count-up tweens, modifier aggregate cache) and MUST NOT do shape transforms. All persisted-shape changes live in the `From<Vn> for Vn+1` impl in `vN+1.rs`.

The freeze rule: **once `vN.rs` is on `main`, do not edit it** (except to fix a migration bug). Schema changes go in `vN+1.rs` together with a `From<vN>`-style conversion. Each version's frozen file owns its own copies of every persisted enum/struct so future changes to live types in `crate::game::*` cannot retroactively reshape what `vN` means on disk.

Every migration branch ships with a unit test that constructs a `GameStateVN` fixture, runs the chain, and asserts the resulting state. The mandatory-test rule from "Project policies" applies to migration steps as much as to the live code.

When bumping `CURRENT_VERSION`:

1. Add `vN+1.rs` and route it in `load_from_str`'s match.
2. Implement `From<GameStateVN> for GameStateV{N+1}` (the actual shape transform).
3. Implement `GameStateV{N+1}::into_current() -> GameState`.
4. Update the live `GameState` struct + `GameState::default()` to match the V{N+1} shape.
5. Bump `CURRENT_VERSION = N+1` in `src/save/mod.rs`.
6. Tests: deserialize a vN fixture, walk it through the chain, assert outcomes.

## Modifier system

Per-fingerer modifiers live on `FingererState.modifiers: Vec<Modifier>`. Each `Modifier` has a stable `ModifierSource` id, zero or more `ModifierEffect`s, and an optional `ModifierDuration`. See `src/game/modifier.rs` for the type definitions and stacking rules; the short version:

- **`AddPercent` values across modifiers SUM** (two +10% Green Coins = +20%, not +21%).
- **`MulFactor` values across modifiers MULTIPLY** (two x2 buffs = x4).
- **`FlatFps` values SUM**, applied before any percent.

Final per-fingerer FPS:
```
((base * count + flat_fps) * upgrades_mult) * (1 + add_percent) * mul_factor
```

The `aggregate: FingererAggregate` cache on `FingererState` is `#[serde(skip)]` and rebuilt in three situations only: (a) modifier add/remove via `attach_modifier` / `attach_modifier_random_*`, (b) per-tick expiry walk if any timed modifier expired, (c) save load via `migrate_runtime`. Hot-path reads (`fps()`, sidebar render) MUST go through the aggregate; never iterate `Vec<Modifier>` per FPS read.

**Picking a target fingerer** for a "random fingerer" event:

- **`attach_modifier_random_owned`** filters by `count > 0`. Use this when the modifier is meaningless on a 0-owned tier — e.g. the Buff Golden's temp `MulFactor(7.0)` multiplies zero output if the player doesn't own one.
- **`attach_modifier_random_visible`** filters by `fingerer::visible(idx, count, lifetime_cuques)` — the same rule the sidebar uses to decide which rows to show. Use this when the modifier is **permanent** and useful to "save for later" — e.g. the Green Coin's permanent `+10%` AddPercent: landing on a tier the player can see but hasn't bought yet still benefits them when they finally afford it. Index Finger is always visible (`idx == 0` short-circuits), so this picker is never empty in practice.

**Adding a new buff/debuff source** = add a variant to `ModifierSource` and map its `id()`. No new tick code unless the trigger is novel (the existing per-tick walk already decrements timed modifiers and rebuilds aggregates on expiry). Cosmetic names belong in `i18n.rs`; the id stays stable forever.

The `Buff` enum (`src/game/state.rs`) is now click-side only (`ClickFrenzy`). Anything that targets a specific fingerer is a `Modifier`, not a `Buff` — Purple Coin (Buff golden), Green Coin, future Blizzard / RedCoin / etc. The V1→V2 migration absorbs in-flight `BuffV1::FingererBoost` entries into per-fingerer modifiers.

## Powerup system

All on-screen powerups (Lucky / Frenzy / Buff / GreenCoin) live in a single `Vec<Powerup>` on `GameState` keyed by `PowerupKind`. `Powerup` carries a stable `spawn_id: u64` minted from `GameState::next_spawn_id`; click hit-test and the `g` hotkey reference instances by id, never by Vec index. The Vec is unbounded — multiple of any kind can coexist on screen and pile-ups self-resolve via the short (~11s) lifetime.

- **`PowerupKind` is the only discriminator.** Per-kind config (`lifetime_ticks`, `mean_cooldown_ticks`, render palette) lives as methods on the enum; the `Buff::ClickFrenzy` click buff is the only thing that's still a `Buff`. There is no separate `GoldenVariant` or `GreenCoin` type — the kind is the whole story.
- **`mean_cooldown_ticks(kind)` is the single tuning knob.** Raise to make the kind rarer, lower to see it more often. `min_cooldown_ticks` (5s) and `max_cooldown_ticks` (4×mean) are safety rails on the truncated-exponential sampler in `powerup::next_cooldown`, not gameplay knobs.
- **Cooldowns are sampled inter-arrival, not rolled per tick.** One RNG draw per spawn (~25 ns). Per-kind clocks tick independently; cooldown does NOT freeze when the kind already has on-screen instances.
- **Spawns are dispersed**: `sim::pick_dispersed_frac` retries up to 8 times to keep a new powerup ≥ 5% of the biscuit away from existing ones, then accepts a plain-random position (best-effort, not a spatial index). Slight overlap is fine; exact overlap is rejected.
- **Stacking is unrestricted.** Two Buff catches on one fingerer = `MulFactor(7)² = ×49` for the duration; three = ×343. Multiple Green Coins on one tier sum (+10% each). The modifier system already supports it; we don't add caps. Big numbers are the genre.
- **Adding a new kind is a single-file change.** Append a variant to `PowerupKind`, add `lifetime_ticks` / `mean_cooldown_ticks` arms, add a render-palette arm in `src/ui/biscuit.rs::powerup_palette`, and add a catch-effect arm in `GameState::catch_powerup`. No new `Action`, no new slot, no new rect array — input routing and tick/spawn loops inherit the new kind unchanged.

The `goldens_since_green_coin` pity counter died with V3 — every kind (including GreenCoin) now uses the same per-kind cooldown mechanism. Truncation in `next_cooldown` already prevents pathological droughts; if we ever miss the "ramp-up after droughts" feel we can re-introduce it as a layered effect on `next_cooldown(GreenCoin)`.

## Debug pane policy

Any new feature whose trigger depends on RNG, time, or other non-input signal **must** ship with a corresponding F-key in `src/ui/debug_pane.rs::DEBUG_KEYS` that forces the event in dev builds. Gate every dev hotkey behind `is_dev_build() && !cli.no_debug`, same as F2/F3/F4/F5/F8.

Strict content (new upgrade tiers, new fingerers, new achievements) doesn't need a debug key — only event triggers.

Current dev hotkeys:

| Key | Action |
|-----|--------|
| F8  | Force-spawn Lucky powerup  *(F1 is hijacked by browsers for Help)* |
| F2  | Force-spawn Frenzy powerup |
| F3  | Force-spawn Buff powerup (Purple Coin → x7 fingerer modifier) |
| F4  | +1M cuques |
| F5  | Force-spawn Green Coin (permanent +10% on a visible fingerer) |

Each F-key pushes a fresh `Powerup` onto `state.powerups` regardless of how many of that kind are already on screen — pressing F8 twice in a row produces two Lucky's, which is the headline behavior of the Vec-based refactor.

When adding a new key, update both `DEBUG_KEYS` (the rendered list) and the corresponding `KeyCode::F(N)` arm in `src/input.rs`. The two must stay aligned — the pane is the single source of truth for what's advertised.

## Quality-test gate for new features

**Every new gameplay feature ships with a `tu`-driven smoke test that the assistant runs before declaring the feature done.** Type checks and `cargo test` verify code correctness; they do *not* verify that the feature actually works on screen. The TUI is the product — if the marker doesn't render, the badge doesn't update, or the click doesn't catch, no amount of green tests catches that. Drive the live binary, screenshot the result, assert what you see.

The recipe (works for any feature involving rendering, input, or RNG-gated events):

1. **Build release**: `cargo build --release`. Dev binary, `(dev)` in the HUD, all F-key cheats live.
2. **Spawn a fresh-save session** under `tu` with an isolated config dir so the test never touches the real save:
   ```sh
   TMPSAVE=$(mktemp -d)
   tu run --name <feature>-test --size 140x42 \
       --env XDG_CONFIG_HOME=$TMPSAVE -- \
       /full/path/target/release/cuqueclicker
   ```
3. **Drive it with the relevant cheats**. F4 grants 1M cuques (so you can buy what you need), then F8/F2/F3/F5 force-spawn the powerup variants. Bare digits buy fingerers; `g` catches the on-screen powerup. Use `tu screenshot --png --output /tmp/<step>.png` and then `Read` the image — **prefer PNG over text screenshots**: the text variant strips color, so green vs purple vs gold tints (which are often the whole point of the visual feedback) collapse into invisible whitespace. Reach for the text form only when you don't need color at all (e.g. asserting a specific string is on screen). Do NOT rely on `tu wait --stable` — the HUD count-up and casino border animate continuously, so the screen never stabilizes. Insert short `sleep 0.3`-`0.5` pauses between actions instead.
4. **For letter keys, prefer `tu type "<letter>"` over `tu press <letter>`.** Empirically, `tu press g` did not deliver a 'g' keystroke into the alt-screen TUI under tu 0.6.1 in this repo's setup; `tu type "g"` does. Use `tu press` for named keys (F1-F12, Enter, Escape, etc.) and `tu type` for letters/digits.
5. **Assert on the screenshot.** For PNG: read it with the `Read` tool and visually inspect colors + glyphs. For text: parse with `python3 -c 'import json,sys;d=json.load(sys.stdin);…'` and grep for marker glyphs (e.g. `( $ )` for Green Coin), sidebar badges (e.g. `+10%`), and HUD numbers the feature should change.
6. **Tear down**: `tu press q --name <feature>-test` (clean quit so the save lock releases), then `tu kill --name <feature>-test` and `rm -rf $TMPSAVE`.

Two important rules:

- **Never run the test against the real save dir.** Always pass `--env XDG_CONFIG_HOME=<tempdir>`. The single-instance lock will block a real running game, and a stray test write would corrupt the player's save.
- **Quit cleanly with `q` before `tu kill`.** Killing while the alt-screen is up can leave the host terminal in a wrecked state on some configurations; quitting first lets the binary restore the screen.

The smoke test for #21 (Green Coin) walked: launch fresh save → F5 (force-spawn) → PNG screenshot (assert green `( $ )` marker rendered in green palette, distinct from gold/purple) → `tu type "g"` (catch) → PNG screenshot (assert `+10%` badge on Index Finger row, green-tinted title-bar pulse from the new border channel firing). That sequence is the template for any future powerup. Note the visible-set targeting: the badge can land on a tier the player hasn't bought yet (any sidebar-visible row), so the test doesn't need to buy a fingerer first.

## Visual / animation policy

- **HUD border animation is casino-style, NOT always-spinning.** Baseline is flat grey. Activity (click, buy, buff, achievement) ramps *up* into chromatic pulses; idle ramps back down. Shader math is additive per channel with coprime cycle lengths (11/13/17/23), so stacked events compose into wilder patterns instead of overriding each other.
- **Pulses go grey → white+color → grey, not grey → grey+color.** The carrier goes to **true white** during activity; colors modulate that white. Pulsing between grey and grey-plus-color reads as dim.
- **Decay is plateau + smoothstep, not linear.** Sits at full strength, then fades smoothly. No hard cut, no constant shrink.
- **Zoom is stepped — 4 hand-tuned ASCII levels, not interpolated.** `BISCUIT_LEVELS` in `src/ui/biscuit.rs` ships full / medium / small / tiny variants (100% / 70% / 45% / 25%). Each level is an explicit art string; nothing is computed at runtime from a single template. Don't add levels casually (more art to maintain), don't remove one without keeping `level_label()` aligned with the remaining indices.
- **Hands rings cap per type at `PER_TYPE_CAP = 40`**, ring width `PER_RING = 48`. Visually identical above 40 — don't bother rendering more.

## Demo recording

Full recipe lives in `.claude/skills/update-demo/SKILL.md`. Invoke via `/update-demo`. Headline rules duplicated here because they are **load-bearing** and cost real time when forgotten:

- Record with `asciinema --window-size 140x42` — **`--cols` / `--rows` are silently ignored** in asciinema 3.x and fall back to 80×24. Always verify with `head -1 /tmp/*.cast`.
- `--output-format asciicast-v2`. Default v3 is rejected by `agg`/`svg-term-cli`.
- `agg --theme ...` does **not** accept the literal word `custom`. For pitch-black bg, pass an 18-color palette string: `"ffffff,000000,<8 normal>,<8 bright>"`.
- Pipeline is `asciinema → agg (GIF) → ffmpeg (MP4, yuv420p, +faststart)`. Target MP4 ~2-3 MB for a 35s clip.
- **GitHub's README renderer strips `<video>` tags and `raw.githubusercontent.com` video URLs.** The only form that auto-embeds is a bare `https://github.com/user-attachments/assets/<uuid>` URL on its own line. Upload is browser-drag-drop into https://github.com/flipbit03/cuqueclicker/issues/1 — **`gh` CLI cannot produce user-attachments URLs.** After re-recording, hand the MP4 to the user, wait for them to paste the URL back, then edit README.

The demo driver at `src/app.rs::demo_driver_tick` deterministically cycles golden variants **Buff → Frenzy → Lucky** (Buff first so the purple powerup is guaranteed on camera). `build_demo_state()` sets `golden_cooldown: 0` so the first Buff spawns on tick 1.

## Release workflow

Cutting a release is a human action:
1. `git tag vX.Y.Z && git push origin vX.Y.Z`
2. Create a GitHub Release on that tag (in the web UI) with notes.
3. `release.yml` fires automatically: patches Cargo.toml version from tag, publishes to crates.io, builds the 4-target matrix, uploads binaries as release assets.

Requirements / secrets:
- Repo secret `CARGO_REGISTRY_TOKEN` — Cargo API token scoped to the `cuqueclicker` crate.
- `publish` job runs **first** and feeds nothing downstream — build/upload don't depend on crates.io success.
- Windows build passes `RUSTFLAGS=-C target-feature=+crt-static` so the `.exe` has no VC++ redist dependency.
- Linux targets build under `musl-tools` for static binaries.
- macOS is only `aarch64-apple-darwin`. No Intel Mac. No static-linking claim on macOS — just a regular dylib-linked binary.

## Commands you'll run

```sh
cargo build --release            # produces target/release/cuqueclicker (dev binary, v0.0.0)
cargo fmt --check                # CI gate — must pass
cargo clippy -- -D warnings      # CI gate — must pass (warnings are errors)
cargo test

# Dev-only debug run (F1-F4 cheats on, debug overlay visible):
cargo run --release

# Dev-only debug run with cheats OFF (simulates release UX on dev code):
cargo run --release -- --no-debug

# Demo recording — see /update-demo skill for full pipeline.
./target/release/cuqueclicker --demo-for-recording 35 --no-debug
```

CI runs `cargo fmt --check` and `cargo clippy -- -D warnings` on every push. Common lint fixes you'll re-encounter: prefer `is_multiple_of`, prefer `div_ceil`, collapse nested `if`s. `UpgradeEffect` carries `#[allow(clippy::enum_variant_names)]` intentionally.

## Gotchas inventory

- **`--no-debug`'s help text says "disable", not "hide".** Cheats aren't hidden; they're physically gone.
- **`cuqueclicker self update` runs before terminal-raw-mode setup.** Never call it from inside the alt-screen — it'd corrupt stdio. `main.rs` parses the subcommand first, then enters raw mode only if no subcommand matched.
- **Clap derive: multi-line `.args([...])` triggers rustfmt churn** — keep args inline or use a single-line vec.
- **`dtolnay/rust-toolchain@stable`** tracks latest stable, not pinned. If upstream Rust removes a feature we're using, CI breaks silently until next push.
- **Saving on every tick would thrash the disk** — `SAVE_INTERVAL_TICKS = TICK_HZ * 10` (save every 10s). Demo mode skips this entirely.

## Repo layout (quick map, read the code for details)

```
src/
├── main.rs              # clap CLI, subcommand dispatch, terminal setup
├── app.rs               # App state, tick loop, event dispatch, demo driver
├── build_info.rs        # VERSION const + is_dev_build()
├── format.rs            # number formatting for HUD (k/M/B/T suffixes)
├── input.rs             # InputContext + raw events → Action router
├── sim.rs               # Action / BuyQty / sim_tick / spawn-rolls / apply_action
├── self_cmd.rs          # `cuqueclicker self update` impl
├── i18n.rs              # EN + pt_BR strings, $LANG auto-detect
├── platform/            # Persistence + InstanceLock — native vs web parity
├── save/                # versioned migration chain
│   ├── mod.rs           # load_from_str / save_to_string / CURRENT_VERSION
│   ├── migrate.rs       # peek_version
│   └── versions/        # vN.rs frozen schemas + From<Vn> chain steps
├── game/
│   ├── state.rs         # GameState, FingererState, Buff (click-only), migrate_runtime
│   ├── fingerer.rs      # 10 tiers: Index Finger → Hand of God
│   ├── modifier.rs      # Modifier / Effect / Duration / Source / FingererAggregate
│   ├── upgrade.rs       # 34 upgrades across tiers
│   ├── achievement.rs
│   └── powerup.rs       # PowerupKind (Lucky/Frenzy/Buff/GreenCoin) + truncated-exponential cooldown sampler
└── ui/
    ├── mod.rs           # top-level draw, HUD title w/ version + (dev)
    ├── border.rs        # casino-style animated border
    ├── biscuit.rs       # the ASCII ass + draw_powerup (unified across kinds)
    ├── hands.rs         # colored rings of fingerers
    ├── sidebar.rs, stats.rs, upgrades.rs, prestige.rs, achievements.rs, effects.rs
    └── debug_pane.rs    # F-key cheats overlay, dev-only

.github/workflows/       # ci.yml (lint/test/build matrix × 4), release.yml
.claude/skills/update-demo/SKILL.md   # demo re-record pipeline
install.sh, install.ps1  # curl|sh and irm|iex one-liners, idempotent
```

## Related / upstream reference

`flipbit03/terminal-use` (`tu`) is the same author's other Rust project and our reference point for release/CI structure. When in doubt about workflow patterns, go check `tu`. Notable deltas: CuqueClicker adds Windows to the matrix (tu is Linux+macOS only), has an in-game rendered demo (tu has a NetHack gameplay demo), and ships pt_BR i18n.
