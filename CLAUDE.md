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
- **Debug pane** (F1 / F2 / F3 spawn Lucky/Frenzy/Buff goldens; F4 gives free cuques). Also requires `!cli.no_debug`. `--no-debug` is opt-**out** on dev; it's labelled "disable" not "hide" because the point is that the cheats are gone, not just invisible.
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
- **Active buffs persist across quit/restart; goldens and `golden_cooldown` don't.** `buffs` is serialized; `golden` and `golden_cooldown` are `#[serde(skip)]` and re-seeded on load. If you add new persistent buff state, make sure it's `Serialize + Deserialize` on `GameState` — and that `Buff::FingererBoost` variants use the stable `fingerer_id: String`, not a position index.

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
├── save.rs              # load/save + single-instance lock via std::fs::File::try_lock
├── self_cmd.rs          # `cuqueclicker self update` impl
├── i18n.rs              # EN + pt_BR strings, $LANG auto-detect
├── game/
│   ├── state.rs         # GameState, Buff, Particle, migrate()
│   ├── fingerer.rs      # 10 tiers: Index Finger → Hand of God
│   ├── upgrade.rs       # 34 upgrades across tiers
│   ├── achievement.rs
│   └── golden.rs        # Golden Cuque: Lucky | Frenzy | Buff
└── ui/
    ├── mod.rs           # top-level draw, HUD title w/ version + (dev)
    ├── border.rs        # casino-style animated border
    ├── biscuit.rs       # the ASCII ass, smooth zoom
    ├── hands.rs         # colored rings of fingerers
    ├── sidebar.rs, stats.rs, upgrades.rs, prestige.rs, achievements.rs, effects.rs
    └── debug_pane.rs    # F-key cheats overlay, dev-only

.github/workflows/       # ci.yml (lint/test/build matrix × 4), release.yml
.claude/skills/update-demo/SKILL.md   # demo re-record pipeline
install.sh, install.ps1  # curl|sh and irm|iex one-liners, idempotent
```

## Related / upstream reference

`flipbit03/terminal-use` (`tu`) is the same author's other Rust project and our reference point for release/CI structure. When in doubt about workflow patterns, go check `tu`. Notable deltas: CuqueClicker adds Windows to the matrix (tu is Linux+macOS only), has an in-game rendered demo (tu has a NetHack gameplay demo), and ships pt_BR i18n.
