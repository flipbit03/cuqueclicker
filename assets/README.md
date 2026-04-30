# Asset bundle

## `jetbrains-mono.atlas`

Pre-rasterised font atlas baked from JetBrains Mono Regular at 15pt, used by
the WebGL2 backend on the wasm build (`src/wasm_app.rs` loads it via
`include_bytes!`). Cell size is 11×22 px; coverage is 99.2%, missing only
exotic super/subscript glyphs JBM doesn't ship (we don't use them).

The atlas covers ASCII (always), Latin-1 supplement (`®`, `±`, `·`, `×`,
`÷`, accented letters for pt_BR), the dashes/ellipsis/superscripts
ranges we use (`—`, `…`, `⁶`), arrows (`→`, `↔`), math operators
(`∈`, `≤`, `≥`), and the box-drawing + block-element ranges that
ratatui's default `Block::bordered()` style needs (`─│┌┐└┘├┤┬┴┼ …`).

### Regenerating

Requires `beamterm-atlas` (`cargo install beamterm-atlas`) and JetBrains
Mono installed system-wide as a complete family (Regular + Bold +
Italic + BoldItalic — beamterm only enumerates fonts with all four
variants present). Then from the repo root:

```sh
beamterm-atlas generate "JetBrains Mono" \
  -s 15 \
  --emoji-font "Apple Color Emoji" \
  -r 0x00A0..0x00FF \
  -r 0x2010..0x2027 \
  -r 0x2070..0x209F \
  -r 0x2190..0x21FF \
  -r 0x2200..0x22FF \
  -r 0x2500..0x259F \
  --check-missing \
  -o assets/jetbrains-mono.atlas
```

If you add new non-ASCII glyphs to the game, run
`rg -on '[^\x00-\x7F]' src/` to enumerate the codepoints, extend the
`-r` ranges, regenerate, and commit the new atlas.

## `JetBrainsMono-OFL.txt`

The SIL Open Font License 1.1 covering JetBrains Mono. The `.atlas`
file embeds the rasterised glyph outlines, which counts as "Font
Software" under the OFL, so the license must travel with the binary
distribution.
