---
name: update-demo
description: Re-record the animated demo MP4 at docs/demo.mp4 using the `--demo-for-recording` dev mode, then refresh the GitHub user-attachments URL embedded in the README. Invoke when the HUD, biscuit art, sidebar, panels, or any visible feature has changed enough that the existing demo no longer represents the current version of the game.
---

# update-demo

Regenerates `docs/demo.mp4` (the animated demo shown at the top of the README) from a fresh auto-played recording of CuqueClicker, renders it with a pitch-black terminal background, and updates the GitHub user-attachments URL that the README embeds.

## When to use

- HUD labels / layout changed
- Biscuit ASCII art changed
- Sidebar fingerer rows changed
- Any panel (Stats / Achievements / Upgrades / Prestige) changed visually
- A new mechanic landed (new buff, new variant, new animation effect)
- Theme / color work in the HUD border changed
- A new release is about to go out and the demo should reflect it

Skip if the change is internal (save format, CI, docs prose) — the demo only cares about what a viewer sees on the terminal.

## Tools required

- Rust toolchain (build the dev binary)
- `asciinema` — `brew install asciinema` if missing
- `agg` — asciinema's GIF renderer (`brew install agg`)
- `ffmpeg` — for GIF → MP4 conversion

## Pipeline

`cuqueclicker --demo-for-recording` → asciinema `.cast` → `agg` GIF (pitch-black theme) → `ffmpeg` MP4.

GitHub's README renderer embeds MP4s as playable videos only via `user-attachments` URLs — so the final step is uploading the MP4 to a GitHub issue comment and pasting the returned URL into the README (bare, on its own line — no `<video>` tag).

## Steps

1. Build a release-optimized dev binary:
   ```sh
   cargo build --release
   ```
   The demo mode (`--demo-for-recording`) is gated behind `build_info::is_dev_build()`, i.e. the Cargo.toml version stays at `0.0.0`. If the version is patched (e.g. during a CI release build), `--demo-for-recording` silently no-ops.

2. Record with asciinema, emitting **asciicast v2** (agg 1.7 handles v2 reliably):
   ```sh
   asciinema rec /tmp/cuqueclicker_demo.cast \
     --output-format asciicast-v2 \
     --window-size 140x42 --overwrite \
     --command "./target/release/cuqueclicker --demo-for-recording 35 --no-debug"
   ```
   - `--demo-for-recording 35` → 35-second auto-play. Accepts any integer; try `25` for a snappier clip or `45` for more variety.
   - `--no-debug` → hides the F1-F4 dev overlay so the demo doesn't advertise cheat keys.
   - `--window-size 140x42` — NOT `--cols`/`--rows` (asciinema silently ignores those and falls back to 80×24). Verify via `head -1 /tmp/cuqueclicker_demo.cast` — the v2 header should read `"width":140,"height":42`.

3. Render to GIF with a **pitch-black custom theme**:
   ```sh
   agg --fps-cap 30 --font-size 20 \
     --theme "000000,ffffff,000000,dd3c69,4ebf22,ddaf3c,26b0d7,b954e1,54e1b9,d9d9d9,4d4d4d,dd3c69,4ebf22,ddaf3c,26b0d7,b954e1,54e1b9,ffffff" \
     /tmp/cuqueclicker_demo.cast /tmp/demo.gif
   ```
   - `--theme custom` (the literal word) is rejected by agg. Pass an 18-color comma-separated palette instead: `bg,fg,<8 normal>,<8 bright>` — that's what agg treats as "custom".
   - **Order is `bg` FIRST, then `fg`** — not `fg,bg`. Flipping it silently gives you an inverted terminal (white bg, black text) and the video is wrong. First slot `000000` = pitch black background; second slot `ffffff` = white text.
   - Always verify afterwards: `ffmpeg -ss 10 -i docs/demo.mp4 -frames:v 1 /tmp/check.png` then `open /tmp/check.png` (or `Read` the image). Eyeball it before committing.
   - `--font-size 20` keeps the text readable at README width.

4. Convert GIF → MP4 (h264, faststart for browser streaming):
   ```sh
   ffmpeg -y -i /tmp/demo.gif \
     -movflags +faststart -pix_fmt yuv420p \
     -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" \
     docs/demo.mp4
   ```
   - `-pix_fmt yuv420p` + even-dimension scale filter → broadly compatible h264 that plays in Safari, Chrome, Firefox, and GitHub's video element.
   - Expect ~2–3 MB for a 35s clip. If it's 10+ MB, shorten the recording.

5. Sanity-check:
   ```sh
   ls -la docs/demo.mp4
   ffprobe docs/demo.mp4 2>&1 | grep -E 'Duration|Video:'
   ```

6. **Ask the user** to upload the MP4 to GitHub — this is the one step you cannot automate. Tell them:
   > Please upload `docs/demo.mp4` as a comment on https://github.com/flipbit03/cuqueclicker/issues/1 (drag-drop the file into the comment box), then paste the resulting `https://github.com/user-attachments/assets/<uuid>` URL back here so I can swap it into the README.
   - That specific issue exists as the durable host for demo assets — reuse it on every re-record.
   - **`gh` CLI cannot do this** — the user-attachments endpoint is browser-only. Don't try `gh issue comment`, `gh api`, curl, or any scripted workaround; they all produce URLs that GitHub's README renderer refuses to auto-embed.
   - The user can post or discard the comment after uploading — the asset persists on GitHub either way.
   - Wait for the user to paste the URL back before proceeding.

7. Paste the user-supplied URL into `README.md` as a bare line (mirrors `flipbit03/terminal-use`'s pattern):
   ```md
   https://github.com/user-attachments/assets/<uuid>
   ```
   GitHub's README renderer auto-expands bare user-attachments URLs into inline `<video>` elements. Do NOT wrap in a `<video>` tag — GitHub's sanitizer strips custom video markup in READMEs.

8. Commit:
   ```sh
   git add docs/demo.mp4 README.md && git commit -m "docs: re-record demo"
   ```

## Knobs to play with

- **Duration** — `--demo-for-recording N` where N is seconds. Shorter = smaller MP4.
- **Terminal size** — `--window-size` on asciinema (NOT `--cols`/`--rows`; those are silently ignored). 140x42 is tuned to fit the full biscuit + sidebar + help bar with margin.
- **Demo content** — the auto-driver lives in `src/app.rs::demo_driver_tick`. It deterministically cycles Golden variants **Buff → Frenzy → Lucky** so every recording shows each flavor (Buff = purple). Tweak what it clicks/buys/swaps if the default schedule doesn't show off a new feature well.
- **Initial state** — `src/app.rs::build_demo_state`. Bump owned counts / cuques / prestige here to start in a richer state. `golden_cooldown: 0` makes the first Buff spawn on tick 1 so the purple powerup lands in the clip's first couple of seconds.
- **Theme palette** — the `--theme` string accepts any 18-color spec. Swap `000000` for a dark-blue if pitch black ever feels too aggressive.

## Common issues

- `only asciicast v1 and v2 formats can be opened` — you used asciinema's default (v3). Re-record with `--output-format asciicast-v2`.
- `"custom" isn't a valid value for '--theme <THEME>'` — the word `custom` alone is rejected; pass the actual 18-color palette as the theme value.
- MP4 is huge (>10 MB) — shorten the recording or shrink the terminal.
- Demo won't start — you're in a release build (version patched away from `0.0.0`). Demo mode is dev-only by design.
- Debug pane still visible in the recording — you forgot `--no-debug`.
- Video looks cramped / way smaller than the intended dimensions — you used `--cols N --rows N` instead of `--window-size NxN`. Inspect `head -1 /tmp/cuqueclicker_demo.cast` to confirm the recorded `"width"`/`"height"`.
- README video doesn't play on GitHub — you embedded the raw repo URL (`raw.githubusercontent.com/.../demo.mp4`) or wrapped the URL in `<video>`. GitHub only auto-embeds bare `user-attachments` URLs on their own line.
- No purple powerup visible in the clip — the demo schedule or `build_demo_state` got edited. Verify `demo_driver_tick` still forces `GoldenVariant::Buff` on the first spawn (count 0 → Buff), and that `build_demo_state` sets `golden_cooldown: 0`.
