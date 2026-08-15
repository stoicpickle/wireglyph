# Visual proof

These images are generated from the same Ratatui `TestBackend` buffer exercised
by the golden tests. They are not design mockups.

- `wireglyph-amber-100x30.png`: compact field display
- `wireglyph-amber-140x40.png`: wide instrument console with navigator and inspector
- `wireglyph-amber-flow-05-100x30.png`: compact edge-evidence drawer at the HTTP → DOMAIN boundary
- `wireglyph-amber-play-full-{100x30,140x40}.png`: full-motion mid-edge frames
- `wireglyph-amber-play-{reduced,off}-100x30.png`: discrete and no-motion states
- `wireglyph-{green,cyan,red,mono}-100x30.png`: semantic theme variants
- `wireglyph-self-map-overview-140x40.png`: first-party dogfood proof generated
  by scanning Wireglyph's own Rust source, tests, example, and dependencies

Regenerate the intermediate SVG directly from the renderer:

```sh
cargo run --example render_proof -- proof/wireglyph-amber-100x30.svg 100 30 amber
cargo run --example render_proof -- proof/wireglyph-amber-flow-05-100x30.svg 100 30 amber 5
cargo run --example render_proof -- proof/wireglyph-amber-play-full-100x30.svg 100 30 amber play full 54
cargo run --example render_proof -- proof/wireglyph-amber-play-reduced-100x30.svg 100 30 amber play reduced 4

# Render an exported scanner graph instead of the synthetic fixture.
WIREGLYPH_GRAPH=/tmp/wireglyph-graph.json cargo run --example render_proof -- proof/project.svg 140 40 amber
WIREGLYPH_GRAPH=/tmp/wireglyph-graph.json cargo run --example render_proof -- proof/project-trace.svg 140 40 amber trace
```

The proof exporter exists only to make terminal colors inspectable in code
review. It is not a browser renderer or part of the Wireglyph runtime. Public
proof uses only the synthetic fixture or Wireglyph itself; third-party source
trees and design-reference collections are not committed.
