---
name: perfboard-diylc-part
description: Procedure for teaching the perfboard program a new DIYLC part class (a sketch that stops with "is a DIYLC ..., which isn't supported yet"), or a new DIYLC drawing for KiCad footprints, so its pins and connections are read exactly as DIYLC reads them and it is written and drawn correctly. Use when a .diy file is refused, or when asked to support another kind of part.
---

# Supporting a DIYLC part

The reader refuses unknown classes on purpose: a part read wrongly means connections silently missed.
A new class is supported only when its connections are proven to match.

## One entry in `CLASSES` (`src/diylc.rs`)

Everything the program knows about a DIYLC class is one `Class` in `CLASSES`; the reader
(`src/diylc/read.rs`) and the writer (`src/diylc/write.rs`) read it, and `diylc::class_look` turns
its `short` name into the part's `Look` in the pictures (`src/svg.rs` draws only looks).

1. `name`: the class as in the error message (`"category.Class"`). `short`: what footprints.json's
   `"diylc"` entry asks for, what a sketch part is called (`"DIYLC:<short>"`) and what `diylc::class_look`
   matches on.
2. `kind`: `Kind::Part(Some(&[...]))` for fixed pin names in DIYLC's control-point order (`["+", "-"]`,
   `["A", "K"]`), `Kind::Part(None)` to number them 1, 2, ... or take header labels
   (`read::header_names`). Find the control-point order in a real file: the `<points>` /
   `<controlPoints>` of an item of that class. Wires, traces and pads have their own kinds.
3. `low`: true only if it lies flat or is small enough to sit under a module on headers. It
   becomes the part's layers (`board::Layers::of`): low parts take layer 1 only, others 1 and 2.
4. `radial`: true if it stands up with a round body (then `read::body_of` grows its box to its
   diameter). The body decides collisions and what a jumper may pass over; too small is worse than
   too big.
5. `template`: only if footprints.json should be able to ask for it. `Template::TwoLegged` with
   DIYLC 6.6's own defaults for a new part (size, colours, settings), or a new `Template` variant with
   its branch in `write.rs`; then add its `short` name to the list of `"diylc"` parts in README
   "Using your own circuit".

Legs between holes (spacing not a multiple of 0.1") are detected by `read::off_grid` and kept clear
by the router automatically; nothing to add.

## Writing and drawing it

- A part from a sketch is written back from its own XML (its DIYLC drawing, `PartDrawing`), moved and turned by
  `write::moved()`, which also turns `<orientation>`. Check the new class has no other field that
  depends on direction.
- `diylc::class_look`: give its `short` name a `Look` if a grey box (the default) misleads; a look
  the pictures don't have yet is a new `circuit::Look` variant (`src/circuit.rs`), its branch in `src/svg.rs`, and its
  name in README "The layout file" (a part's `look`).
- DIYLC's drawings (`FootprintDrawing`, `PartDrawing`) are saved in the layout file as opaque JSON
  (`src/build/file.rs`, a part's `"footprint_drawings"` and `"drawings"`, key `"diylc"`), and so is
  its `"look"`: a new field on either drawing changes the file, so raise `VERSION` there if an older
  reader would get it wrong.

## Prove it

1. A sketch containing the part: `cargo run --release -- nets <file>` lists the pins and nets you expect.
2. `cargo run --release -- check <file>`, and on a folder of real sketches if there is one
   (CLAUDE.local.md says where): 0 wrong.
3. Against DIYLC itself, if a checker is set up (CLAUDE.local.md): compare DIYLC's netlist with `nets`.
4. Add the part to README's "Supported DIYLC parts" list, then run the `drift-check` agent.
