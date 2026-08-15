# Checkpoint 0 acceptance

> Historical checkpoint: these claims describe the synthetic visual spike.
> Current scanner capabilities and limits are documented in the README.

Checkpoint 0 proves the fixture-driven terminal instrument. It does not prove a
repository scanner or the full product.

## Proven in the repository

- The versioned `BEACON OPS` fixture contains 29 nodes, 36 edges, one nine-edge
  static flow, one cycle, and one explicitly inferred unresolved relationship.
- Deterministic target-size snapshots cover 100x30 and 140x40 terminals.
- Amber, green, cyan, red, and monochrome themes render the same semantic roles.
- Every static-flow hop resolves its edge, endpoints, subsystem boundary,
  provenance, confidence, source path, and line range at both target sizes.
- The fifth hop identifies the HTTP to DOMAIN crossing through
  `GET.SYSTEM --calls--> SYSTEM.SERVICE`.
- Compact evidence drawers clear their covered map region without changing map
  geometry.
- Full motion is capped at 12 redraws per second. Reduced motion advances once
  per second. Motion-off is manual-step only. Paused, idle, and motion-off
  states have no scheduled redraw.
- Full playback completes the nine-edge fixture after exactly 108 frames and
  stops at the final dependency without wrapping.
- `ratatui::run` owns raw mode, alternate screen, cursor, and panic restoration.
- Five scripted PTY normal-quit runs exited zero and emitted both alternate-screen
  exit and cursor-show sequences on the development host.
- Five scripted PTY SIGINT runs also exited through the application and emitted
  alternate-screen exit plus cursor-show sequences on the macOS development host.
- Cross-platform PTY integration tests inject both a returned error and a panic
  after the first full frame, then require alternate-screen exit and cursor-show
  sequences before the process exits.
- The focused `windows-conpty` CI job runs those normal, error, and panic paths
  through `portable-pty`, whose native Windows backend is ConPTY.
- Synthetic demo mode performs no parsing, repository traversal, network access,
  cache writes, or mutation of the displayed repository.

Run the local gate:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Rendered proof generated from the same `TestBackend` buffer lives in
`proof/renders/`.

## Human comprehension gate still required

Use at least five fresh developers and counterbalance Wireglyph with ordinary
`tree` and `rg` browsing. Ask each participant to:

1. Find `SERVER.ENTRY` and its source evidence.
2. Identify the HTTP to DOMAIN boundary and the relationship that crosses it.
3. Trace the static system-detail route to `BETTER-SQLITE3`, inspecting evidence
   for every hop.

The product direction advances only if at least four of five finish all tasks,
median Wireglyph time is at least 25 percent faster, wrong turns do not increase,
median confidence is at least four of five, and nobody mistakes static playback
for observed runtime behavior.

Terminal restoration still needs SIGINT repetition in the other terminals
targeted for a future release. Automated PTY and ConPTY checks do not replace
manual release-candidate smokes in each supported terminal.
