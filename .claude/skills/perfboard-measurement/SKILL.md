---
name: perfboard-measurement
description: Checklist for adding, removing, renaming or reweighting one of the perfboard placer's score measurements (board::FEATURES / WEIGHTS), which are also the neural network's inputs, so the network, the router's costs, the outputs and the README stay in step. Use whenever measure() changes what it returns or a rule weight changes.
---

# Changing a measurement

A measurement is in five places that the compiler only partly ties together.

## Code

1. `src/board.rs`: `FEATURES` (names), `WEIGHTS` (one comment per line: what it costs per what), and the
   `Measures([...])` built at the end of `measure_routed()` (which `measure()` calls). All three in the
   **same order**.
2. `src/nn.rs`: `SCALE`, one rough size per measurement so inputs land around 0..1, **same order as
   FEATURES**. The compiler checks the length, not the order.
3. `src/route.rs`: the router's costs are the matching weights × 10 (`STRAIGHT`/`DIAGONAL` for wire,
   `TURN` for turns, `DIAGONAL_EXTRA` for diagonal, `JUMPER_PER_HOLE` for jumper length, `RULE` for a
   soft keep-out's hole or a soft star's join, and `JUMPER` / `SOLDER_WIRE` for each wire, weighed
   against the traces' way round and a join; `crosswise` isn't priced, see `Routing::crosswise_steps`).
   The running total in `route_in_order` (when a candidate routing stops) reads `WEIGHTS[0]` and `[2]` to `[7]` (not turns, `[1]`, which can shrink; leave it out),
   and `route`'s placement cost `[8]` and `[9]`, by position too. `TURN`, `DIAGONAL_EXTRA`, `JUMPER`,
   `JUMPER_PER_HOLE`, `SOLDER_WIRE` and `RULE` read `WEIGHTS[1]`, `[2]`, `[4]`, `[5]`, `[6]` and
   `[10]` by position: inserting a measurement before
   `rules` means changing some of those indexes too, or the router silently prices the wrong thing.
4. The text report, the SVG legend and the layout file (`src/layout.rs`, measurements by name) print every
   measurement themselves; nothing to change there. A layout file saved before a measurement was added,
   removed or renamed is refused by `draw` (its names no longer match `FEATURES`): place it again.

## Network

The saved `out/model.json` is refused only if the *number* of inputs changed. If a measurement now
means something else with the same count, an old model loads silently and judges wrong. Always run
`cargo run --release -- train` (and `evolve` if its figure is quoted) after the change.

## README

- "How a board is judged": one bullet per measurement, and the count in words ("measures N things").
- "Two ways to train": "all N weights" with N = inputs × 8 + 8 + 8 + 1, and the typical errors and times.
- "The pieces", the `src/nn.rs` row: the same weight count.
- "Where the neural network fits": how many numbers the network gets.
- "The layout file": the score has "the eleven measurements by name".
- The timings at the top, if the change touched routing or measuring.
- The benchmark's recorded numbers (`docs/plans/placing-network.md`) were costs under the old
  weights: rerun `bench` and say the old table no longer compares.

## Check

1. `cargo build --release` with no warnings; `cargo clippy` adds none.
2. `cargo run --release -- place` and `place circuits/cat_feeder.diy`: look at the printed measurements,
   the pictures, and the time (CLAUDE.md: say how long the cat-feeder took).
3. `train`: the unseen-board error should stay around a point; say what it became.
4. Run the `drift-check` agent before committing.
