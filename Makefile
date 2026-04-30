# Convenience targets — the canonical commands are still `cargo build /
# clippy / test` and `trunk serve` directly. This file just bundles the
# wasm prereq install + dev-server invocation so a fresh clone can do
# `make web` and have the browser game running on http://0.0.0.0:8088/
# without remembering the incantation.

.PHONY: web web-install web-build

# Live-reload dev server. trunk watches `src/`, rebuilds on save, and
# pushes `window.location.reload()` to every connected browser via the
# WebSocket injected into `index.html` — no manual refresh needed.
# Address + port come from `Trunk.toml`.
web: web-install
	trunk serve

# One-shot release build into `dist/` — useful for `gh-pages` style
# static hosting or for inspecting the bundled artifacts.
web-build: web-install
	trunk build --release

# Idempotent prereq install. `cargo install trunk` is a no-op once
# trunk is on PATH; `rustup target add` is a no-op once the wasm32
# target is installed. Safe to run unconditionally.
web-install:
	@command -v trunk >/dev/null || cargo install trunk
	@rustup target list --installed | grep -q '^wasm32-unknown-unknown$$' \
		|| rustup target add wasm32-unknown-unknown
