# ADR 0001: Prove the terminal instrument before the scanner

> Historical decision record: this ADR describes Checkpoint 0. The scanner and
> real-project modes that followed are documented in the README.

- Status: accepted for Checkpoint 0
- Date: 2026-08-14

## Context

Wireglyph must help a developer form a trustworthy first mental model of an
unfamiliar repository. Its wedge is not merely rendering a dependency graph in
a terminal; it is the combination of an inviting diagnostic-instrument surface,
evidence for every relationship, progressive disclosure, and motion that never
pretends static analysis is observed runtime behavior.

The largest unknown is comprehension under terminal constraints. Parser breadth,
packaging, and scanner performance cannot answer that question.

## Decision

Checkpoint 0 is a Rust application using Ratatui 0.30 and Crossterm 0.29. It
loads a versioned synthetic JSON graph and does not inspect the repository.

- The fixture is a believable 29-node web application called `BEACON OPS`.
- Layout will be deterministic, clustered by subsystem, and layered along a
  central BOOT -> HTTP -> DOMAIN -> DATA spine.
- Ratatui Canvas will draw grid, group geometry, edges, and moving markers.
  Ordinary widgets will draw labels and inspector text.
- Wide terminals use navigator, map, and inspector columns. Below 128 columns
  or 36 rows, navigator and inspector become overlay drawers without moving the
  graph. Below 100x30, the app shows an explicit minimum-size screen.
- Idle rendering is event-driven. Playback alone may redraw at 10-15 FPS.
- Amber, green, cyan, muted red, and monochrome themes share semantic tokens.
- Every path in this checkpoint is labeled `STATIC PATH`. The single inferred
  edge uses text, shape, and line pattern rather than color alone.
- `ratatui::run` owns raw-mode and alternate-screen cleanup. PTY lifecycle tests
  will be added in the playback/safety slice after the visual contract settles.

## TUI bail gate

The full product remains conditional. The spike advances only if fresh users can:

1. find `SERVER.ENTRY` and its evidence;
2. identify the HTTP -> DOMAIN boundary and the relationship crossing it; and
3. trace the static system-detail route to `BETTER-SQLITE3`, opening evidence for
   every hop.

At least four of five participants must finish all tasks, median TUI time must
beat ordinary file browsing by 25 percent, wrong turns must not increase, and no
participant may describe the static path as observed runtime execution.

Technical proof also requires readable 100x30 and 140x40 captures, no label
overlap on the selected path, zero idle redraws, playback capped below 15 FPS,
and clean terminal restoration across quit, error, panic, and SIGINT cases.

## Explicit non-goals

- Repository traversal or parsing
- Tree-sitter and language adapters
- Cross-file call-graph inference
- Cache or config-file design
- Runtime tracing
- Network access, cloud services, or AI explanations
- Packaging and naming decisions
- Browser or image-protocol fallback

## Consequences

The graph contract and renderer can evolve against deterministic evidence before
scanner complexity arrives. If the comprehension gate fails, we revise or stop
the TUI direction without having invested in a parser architecture.

Dependency and lifecycle APIs were verified through current Context7 documentation
for Ratatui, Crossterm, and Insta before pinning this decision.
