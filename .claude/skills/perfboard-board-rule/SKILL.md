---
name: perfboard-board-rule
description: Procedure for adding or changing a rule in the perfboard board file (circuits/<name>.json, read by src/constraints.rs), deciding whether it is a hard limit or a preference and wiring it into the placer, router or score accordingly. Use when asked for a new constraint, a new kind of board-file rule, or a change to fixed, edge, orientation, within, all-of, one-of, keepout, near, close or star.
---

# Adding a board-file rule

## First

- Read `TODO.md`: the maintainer writes there how they want the board file to grow. Every rule is an entry of the board file's `rules` list (or of an all-of's or one-of's `rules`) with a `kind` and an optional
  `name`; follow that format or ask, don't invent a parallel one.
- Decide what kind of rule it is (README "Principles"). This decides where the code goes:

| Kind | Example | Where |
|---|---|---|
| Hard limit on where a part goes | fixed, edge, orientation, within | a `Rule` with a part: `Rule::one_part` and `Rule::part` (name the part; `part` also checks the rule alone) and `Rule::broken` (how far this place is from what it wants: 0, infinite for a hard rule, or weighted holes). `resolve` lists each part's allowed places and `apply` keeps the part on them; nothing else to wire |
| Hard limit on what may share the board, for every part | hard keep-out holes under parts, no two parts on a hole | `board::clashes` (with words in `board::clash_words`): the placer starts from an allowed layout (`place::allowed_start`) and throws away any move that isn't allowed, before routing it. A new kind of clash is a `board::Why` variant, and is named in svg.rs's "Boxed in red" note and README's lists of what's allowed ("How a board is judged", "Or sketch it in DIYLC") |
| Hard limit on copper | hard keep-out holes, star separation | `src/route/search.rs` grid (`owner`, `keepout`), or `constraints::runs` for how a net splits; with a weight, the grid's `extra` (soft keep-outs) or `route::join` (soft star). `route::from_sketch` (your own wiring, measured) also counts star joins and breaks: a new copper rule must be counted there too |
| Priced | near, close | a measurement (both count in `rules`), each rule times its `weight` (`HARD`, 1, if left out: they're only known once routed); a new one: use the `perfboard-measurement` skill |

  Hard limits are never learned by the network. A priced rule never makes a layout illegal, however
  high its weight.

  Every rule takes a `weight`, 0-1 (the maintainer's design: hard by default, softened by a weight).
  `cases` checks the range and passes an all-of's or one-of's weight down to the rules inside
  without one. A new rule must say how it's broken with a weight, in holes, and price that in the
  `rules` measurement (times `WEIGHTS`' rules, 1000): a placement rule in `Rule::broken` (it then
  needs nothing else: `resolve` fills `place_cost`); a keep-out in `soft_keepout` (the router's
  `extra`); a star by its joins (`route::join`). Without a weight it stays as hard as it was.

  all-of and one-of are not kinds of rule to wire anywhere: when the board file is read, `cases`
  expands them into plain lists of rules (a one-of makes one case per option), and every case is
  resolved and placed on its own. The exception is a one-of only about where one part goes
  (`Rule::one_part`): it stays whole, and `Rule::broken` lets the part take any place one of its
  options allows (its least broken option). A new placement rule only needs `one_part`, `part` and `broken`; any other new rule only ever
  sees one case.

## Code (`src/constraints.rs`)

1. `Rule`: a new variant (its `kind` is the variant's name in lowercase; a two-word kind needs
   `#[serde(rename = "all-of")]`-style, like `AllOf` and `OneOf`), with `name: Option<String>`
   and `weight: Option<f64>` like the others (and an arm in `Rule::own_weight`), and a doc comment per field. The enum has `deny_unknown_fields`, so a typo in the
   JSON is an error; keep that for any struct a variant uses.
2. Add it to `Rule::kind_and_name`, which names it in messages. Placement rules that contradict
   need no check of their own: `resolve` returns an error (the case is left out) when a part is
   left with no allowed place. Other repeats that
   make no sense (two stars on one net) stop with a message, since the list allows them.
3. Prefer a new simple rule that composes with all-of / one-of over a bigger special-purpose one
   (the maintainer's choice: consistency and composability before convenience).
4. Other rules (not about where a part goes): `Constraints` gets the rule resolved to indexes
   (component index, `(component, pin)` via `c.find_pin`), so the placer never looks names up.
5. `resolve` (or `Rule::part`, for a placement rule): check everything that is a mistake in the
   file and stop with `panic!("board file: ...")` in plain words naming the part or pin (missing
   part, pin not on that net, a rectangle off the board). A placement rule that is fine but leaves a
   part no place (a `fixed` hanging off the board) isn't a mistake to stop on: `resolve` returns an
   error for it and that case is left out.
6. The module's doc comment shows every kind in its JSON example; keep it loadable.
7. If the pictures, the KiCad board or the `.diy` draw the rule (read from `c.constraints`, like
   keep-outs), the layout file must carry it too: `src/layout.rs` keeps only the keep-outs and the
   one-of choices, and `draw` (and `check`'s read-back) rebuild nothing else. Add it to the file
   (and raise `VERSION` if an older reader would get it wrong), or keep it out of the writers.

## Docs and examples

- README "The board file": the JSON example, one bullet saying what the rule does and whether it's
  a hard limit or always a cost, and a line under **weight** on how it's broken with a weight.
- A real use in `circuits/*.json` if one of the example circuits needs it.
- Pins are named the way `cargo run --release -- nets <file>` prints them. Mind the README's known
  limit on DIP pin names read from DIYLC.

## Check

1. `cargo run --release -- place` and `place circuits/cat_feeder.diy`: the rule visibly holds in the
   pictures; say the time.
2. A deliberately wrong board file (typo'd key, unknown part, wrong pin) stops with a clear message.
3. If the rule touches routing: `cargo run --release -- check circuits/cat_feeder.diy` with two or
   three seeds, and on a folder of real sketches if there is one (CLAUDE.local.md says where).
4. Run the `drift-check` agent before committing.
