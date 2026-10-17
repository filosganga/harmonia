---
name: perfboard-diylc-part
description: Procedure for teaching the perfboard program a new DIYLC part class (a sketch that stops with "is a DIYLC ..., which isn't supported yet"), or a new DIYLC drawing for KiCad footprints, so its pins and connections are read exactly as DIYLC reads them and it is written and drawn correctly. Use when a .diy file is refused, or when asked to support another kind of part.
---

# Supporting a DIYLC part

The reader refuses unknown classes on purpose: a part read wrongly means connections silently missed.
A new class is supported only when its connections are proven to match.

## Reading it (`src/diylc_import.rs`)

1. `kind()`: map the class (`"category.Class"`, as in the error message) to `Kind::Part(names)`.
   `Some(&[...])` for fixed pin names in DIYLC's control-point order (`["+", "-"]`, `["A", "K"]`),
   `None` to number them 1, 2, ... or take header labels (`header_names`). Find the control-point order
   in a real file: the `<points>` / `<controlPoints>` of an item of that class.
2. `is_low()`: true only if it lies flat or is small enough to sit under a module on headers.
3. `is_radial()` / `body_of()`: which holes its body covers. The body decides collisions and what a
   jumper may pass over; too small is worse than too big.
4. Legs between holes (spacing not a multiple of 0.1") are detected by `off_grid` and kept clear by the
   router automatically; nothing to add.

## Writing and drawing it

- A part from a sketch is written back from its own XML (`Source`), moved and turned by `moved()` in
  `src/diylc.rs`, which also turns `<orientation>`. Check the new class has no other field that
  depends on direction.
- For KiCad footprints, DIYLC drawings come from `footprints.json`'s `"diylc"` entry and the matching
  branch in `src/diylc.rs` (`TWO_LEGGED`, `DIL_IC`, `PinHeader`); copy DIYLC's own defaults for a new one.
- `src/svg.rs` `shape()`: add a drawing if a grey box (the default) misleads.

## Prove it

1. A sketch containing the part: `cargo run --release -- nets <file>` lists the pins and nets you expect.
2. `cargo run --release -- check <file>`, and on a folder of real sketches if there is one
   (CLAUDE.local.md says where): 0 wrong.
3. Against DIYLC itself, if a checker is set up (CLAUDE.local.md): compare DIYLC's netlist with `nets`.
4. Add the part to README's "Supported DIYLC parts" list, then run the `drift-check` agent.
