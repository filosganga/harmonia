---
name: perfboard-board-rule
description: Procedure for adding or changing a rule in the perfboard board file (circuits/<name>.json, read by src/constraints.rs), deciding whether it is a hard limit or a preference and wiring it into the placer, router or score accordingly. Use when Filippo asks for a new constraint, a new board-file key, or a change to fixed, edge, keepout, near or star.
---

# Adding a board-file rule

## First

- Read `TODO.md`: Filippo has written how he wants the board file to grow (a single `constraints`
  list with a `kind`, `near` by two pins, a part side facing an edge, `one-of`). Follow his format
  or ask; don't invent a parallel one.
- Decide what kind of rule it is (README "Principles"). This decides where the code goes:

| Kind | Example | Where |
|---|---|---|
| Hard limit on where a part goes | fixed, edge | `constraints::apply`, run after every placer move, so it can't break |
| Hard limit on copper | keep-out holes, star separation | `src/route.rs` grid (`owner`, `keepout`), or `constraints::runs` for how a net splits |
| Preference | near | a new measurement with a fixed weight: use the `perfboard-measurement` skill |

  Hard limits are never learned by the network. A preference never makes a layout illegal.

## Code (`src/constraints.rs`)

1. `BoardFile`: a new optional field with `#[serde(default)]`. The struct has
   `deny_unknown_fields`, so a typo in the JSON is an error; keep that for new structs too.
2. A private struct for the rule's JSON, with a doc comment per field.
3. `Constraints`: the rule resolved to indexes (component index, `(component, pin)` via
   `c.find_pin`), so the placer never looks names up.
4. `resolve`: check everything that can be wrong and stop with `panic!("board file: ...")` in plain
   words naming the part or pin (missing part, pin not on that net, off the board, two rules that
   contradict).
5. The module's doc comment shows every key in its JSON example; keep it loadable.

## Docs and examples

- README "The board file": the JSON example and one bullet saying what the rule does and whether it's
  a hard limit or a preference.
- A real use in `circuits/*.json` if one of the example circuits needs it.
- Pins are named the way `cargo run --release -- nets <file>` prints them. Mind the README's known
  limit on DIP pin names read from DIYLC.

## Check

1. `cargo run --release -- place` and `place circuits/cat_feeder.diy`: the rule visibly holds in the
   pictures; say the time.
2. A deliberately wrong board file (typo'd key, unknown part, wrong pin) stops with a clear message.
3. If the rule touches routing: `cargo run --release -- check circuits/cat_feeder.diy` with two or
   three seeds, and the corpus run in CLAUDE.local.md.
4. Run the `drift-check` agent before committing.
