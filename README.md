# CuqueClicker

A parody idle-clicker in your terminal. Like Cookie Clicker — but instead of a cookie you click, it's an ASCII **ass** you *finger*.

**[Play in your browser!](https://flipbit03.github.io/cuqueclicker/)**

https://github.com/user-attachments/assets/5d10dd73-53e2-4162-9759-b61b0bcb508e

## The name

CuqueClicker is a bilingual pun.

In Brazilian Portuguese, "**Cookie Clicker**" sounds almost identical to "**Cu que clicker**" — roughly *"the ass that clicks (or gets clicked)"*. Swap the **cookie** for a **cu** (ass), and the whole game follows: you don't *click* a *cookie*, you *finger* a *cuque*. Every mechanic, upgrade, and achievement is built around that phonetic accident.

## Install

**Linux / macOS (curl + sh):**
```sh
curl -fsSL https://raw.githubusercontent.com/flipbit03/cuqueclicker/main/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/flipbit03/cuqueclicker/main/install.ps1 | iex
```

**Via Cargo (any platform with Rust):**
```sh
cargo install cuqueclicker
```

**To update** from within an installed build: `cuqueclicker self update`. This re-runs whichever install path you used the first time (installer script or `cargo install --force`).

## Controls

| Key | Action |
| --- | --- |
| `Space` / `Enter` / left-click on the cuque | Finger |
| `1`–`9`, `0` | Buy that fingerer |
| `Shift` + digit | Buy 10 |
| `Alt` + digit | Buy max affordable |
| `t` | Upgrade tree |
| `p` | Prestige panel |
| `s` | Stats panel |
| `a` | Achievements panel |
| `g` | Grab active Golden Cuque / powerup (keyboard equivalent of clicking it) |
| `-` / `+` | Zoom out / in (mouse scroll also works) |
| `q` / `Esc` | Quit (autosaves) |

**macOS note:** the kernel can suspend a backgrounded SSH-only process (or any terminal-only app) after a short idle, which freezes the simulation and stops cuque accrual. The game resumes on input, so you'll never lose progress, but you also won't accumulate cuques while parked. To run continuously through long idle periods, launch under `caffeinate`:

```sh
caffeinate -i cuqueclicker
```

The `-i` flag prevents idle sleep without overriding lid-close sleep, so closing the lid still puts the Mac to sleep normally.

## License

MIT. See [LICENSE](LICENSE).
