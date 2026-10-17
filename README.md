# Perfboard layout + a tiny neural network

Rust, with four small crates (`rand`, `serde`, `serde_json`, and `roxmltree` to read DIYLC files).
Every placer step routes the whole board, so commands take seconds: on the 555, `place` takes about
0.7s, `train` 1.8s and `evolve` 2.9s; placing the bigger cat-feeder board about 2.7s. A board file
with a `one-of` is placed once per case (below), so it takes that many times longer. Anything up to
the time to make and drink an espresso is fine.

```bash
cargo run --release -- place                # auto-place the 555 blinker using the hand-written rules
cargo run --release -- train                # teach the tiny network to score boards like the rules do
cargo run --release -- evolve               # the same, by evolution instead (1 parent + 19 children)
cargo run --release -- place --judge net    # auto-place again, but let the network be the judge
cargo run --release -- place my_sketch.diy  # score a rough DIYLC layout, then fit it on the board
cargo run --release -- nets my_sketch.diy   # just list the parts and connections it understood
cargo run --release -- check my_sketches/   # place, write and re-read every .diy: same connections? (see below)
```

Options: `--steps 50000` (placer effort), `--seed 7` (a different random start), `--board 24x16`
(perfboard size in holes, default 18x12), `--power +12V` / `--ground GND` (which nets are supply and
ground, guessed from common names like `+5V`, `VCC` and `GND` if left out), or a path to another netlist.
Board size, power and ground can also live in the board file (below); a flag wins over it.

Each `place` run prints the board as text (with a `one-of` in the board file, first a line for each case
and which was best) and writes, in `out/` (named `rules` or `net` after the judge):

- `rules.svg`: a picture of the result. Holes are coloured by net, with the copper traces and wires, the parts,
  header pin names and modules, and beside the board the score and every net. Hover a hole for its
  position, pin and net. Hole numbers are the text board's.
- `rules-solder.svg`: the same board turned over to solder it: mirrored (column 0 on the right), copper
  in front, the parts faint behind.
- `rules.html`: the start (random, or your sketch) and the result side by side, drawn the same way.
- `rules.kicad_pcb`: the layout for KiCad's PCB editor (KiCad 10 opens it). Every pad is on its net, so
  KiCad shows the connections still to wire (the routed traces aren't in it yet). The footprints are
  simple stand-ins built from `footprints.json`; Tools → Update Footprints from Library swaps in KiCad's
  real ones without moving anything.
- `rules.diy`: the layout for DIY Layout Creator, on a perfboard, with the routed copper traces (solder
  side) and jumpers, coloured by net (power red, ground near-black); solder-side wires as red jumpers.

## Using your own circuit

1. Draw the schematic in KiCad and give every part a through-hole footprint.
2. In the schematic editor: File → Export → Netlist, KiCad format. That writes a `.net` file.
3. `cargo run --release -- place path/to/your.net`

A netlist only says what connects to what. It doesn't say where a part's legs are, so each footprint
also needs an entry in `footprints.json`: its size in holes and which hole each pad sits in (top-left
hole is `[0, 0]`, a hole is 2.54mm). If one is missing, the program names it. The library name before
the `:` is ignored, so `Package_DIP:DIP-8_W7.62mm` is looked up as `DIP-8_W7.62mm`.

The optional `"diylc"` entry says how to draw the part in DIYLC: `Resistor`, `RadialElectrolytic`,
`RadialCeramicDiskCapacitor`, `LED`, `DIL_IC` or `PinHeader`. For two-legged parts, `"legs"` says which pad is
DIYLC's first leg: that's + on a capacitor but the anode on an LED, which is KiCad's pad 2. Without a
`"diylc"` entry the part is drawn as a labelled box.

`"low": true` marks a part low enough to sit under a module on headers (see below); leave it out otherwise.

## Or sketch it in DIYLC

Instead of a KiCad netlist you can give it a `.diy` file: put the parts down roughly in DIYLC and
connect them with copper traces, jumpers or wires. The program reads which pins connect, the same way
DIYLC does (a pin touching a trace is on it; traces on the same copper layer that touch are joined; a
wire joins whatever is at its two ends). Where you put things is only the starting point: it scores
your sketch ("Your sketch: score ...") and then fits everything onto the real board, which comes from
the board file or `--board`, so the sketch can be drawn on a bigger board than the real one.

- **Pin names** come from DIYLC: `R1.1`, `C1.+`, `D1.A`, or your header labels (`ESP32C6-L.5v`).
  Nets are named after them (`5v`, `GND`, `SCL`), and power and ground are guessed from those names.
- **A DIYLC group with pins in it is one part, a module**: say two pin headers, a rectangle for the
  module's board and some labels. It moves as a whole, and only its pin holes are taken: **low parts**
  (a resistor lying flat, a ceramic disc, a diode) can go under it. Taller parts can't, and two modules
  can't overlap. `:` on the text board marks the space under a module.
- **The `.diy` it writes keeps your own drawing**: your parts with their values, colours and labels,
  moved and turned, your groups still grouped. Everything you drew to connect things (traces, jumpers,
  wires, solder pads) is replaced by the routed copper.
- Supported DIYLC parts: resistors, ceramic/film/electrolytic capacitors, LEDs, plastic diodes, DIP chips,
  TO-92/126/220 transistors, pin headers, tactile switches; copper traces, jumpers, hook-up and
  orthogonal wires, solder pads; labels, text and shapes. Anything else stops with a message naming
  it, because its connections would otherwise be missed.

Checked against DIYLC itself: on 216 DIYLC files, the program finds exactly the connections DIYLC's own
netlist finds. And on all 193 of them with connections, the file it writes (re-placed and re-routed)
reads back with exactly the original's connections.

Some parts have legs between holes (a tactile switch's are 0.175" and 0.255" apart). Copper in a hole
next to such a leg could touch it, so the router keeps other nets out of the holes around it.

## The board file: your rules for this board

Next to the netlist, a JSON file with the same name (`circuits/555_blinker.json` for
`circuits/555_blinker.net`) describes the board you're building on. Everything in it is optional:

```json
{
  "board": "18x12",
  "power": "+5V", "ground": "GND",
  "constraints": [
    { "kind": "fixed",       "part": "U1", "at": [6, 3], "rot": 0 },
    { "kind": "edge",        "part": "J1", "edge": "left" },
    { "kind": "orientation", "part": "C1", "rot": 1 },
    { "kind": "within",      "part": "C1", "from": [0, 6], "to": [5, 11] },
    { "kind": "one-of", "name": "R3 on the left or right edge", "rules": [
      { "kind": "all-of", "rules": [
        { "kind": "orientation", "part": "R3", "rot": 1 },
        { "kind": "within", "part": "R3", "from": [0, 0], "to": [0, 11] } ] },
      { "kind": "all-of", "rules": [
        { "kind": "orientation", "part": "R3", "rot": 3 },
        { "kind": "within", "part": "R3", "from": [17, 0], "to": [17, 11] } ] } ] },
    { "kind": "keepout", "name": "M3 screw", "from": [16, 0], "to": [17, 1] },
    { "kind": "near",    "name": "C2 by CTRL", "pins": ["C2.1", "U1.5"] },
    { "kind": "star",    "at": "C1.2", "runs": [["U1.1", "C2.2"], ["D1.1"]] }
  ]
}
```

- **board / power / ground**: the same as the flags. A flag still wins, for a quick experiment.
- **constraints**: your rules, in any order. Each says its `kind`, and may have a `name`: messages
  about the rule use it, and a keep-out's is written on it in the pictures and DIYLC (in KiCad it
  names the rule area).
- **fixed**: the part goes exactly there, and the placer never moves it. `at` is the top-left hole of the
  part after turning it, `rot` is quarter turns clockwise (0–3).
- **edge**: the part touches that edge: `left`, `right`, `top` or `bottom`. Two edge rules next to
  each other (left and top) put it in that corner.
- **orientation**: the part is turned `rot` quarter turns clockwise (0–3) from how it's drawn (in
  DIYLC, as you drew it; for a netlist, as its `footprints.json` entry is).
- **within**: the whole part, its outline, is inside the rectangle from one corner to the other.
- **all-of**: every rule in `rules` holds. **one-of**: at least one does. They hold any rules, about
  any parts, and nest. A one-of makes cases: "A, one-of (B1, B2), C" is placed twice, as (A, B1, C)
  and as (A, B2, C), and the best layout is kept. A one-of inside a one-of just adds options, an
  all-of keeps its rules together in one option, and two one-ofs side by side multiply (2 and 3
  options make 6 cases). `nets` lists the cases; an option is named by its `name`, or its number.
  The example says "R3 stands along the left or the right edge"; "a USB socket's side is on one of
  the board's edges" is a one-of of four all-ofs, each an orientation and a `within` strip as deep as
  the part. "One of these near rules" places each and keeps the cheaper.
- **keepout**: holes no part may cover, from one corner to the other. Shown as `x` on the text board, a
  grey box in the pictures and in DIYLC, and a "rule area" in KiCad, which keeps enforcing it while you edit.
- **near**: two pins of the same net should be joined by as little copper as possible, like a resistor
  guarding a GPIO or a decoupling capacitor by its chip. It's measured along the copper the router
  laid (traces, jumpers and wires), not in a straight line. On a star net, two pins in different
  runs are only joined through the star point, so that's the way it's measured. A preference: it
  costs per hole of copper.
- **star**: how the net of the `at` pin is wired. Every run starts at `at` and goes its own way, never
  sharing copper with another run, so one part's current doesn't shift the voltage another part sees.
  Each run in `runs` joins its pins in a chain; any other pin of the net gets a run of its own, straight
  from `at`. A net can have one star rule.

fixed, edge, orientation and within are hard limits. In each case, each part they're about gets the
list of places (position and turn) that meet all its rules, while staying on the board and off the
keep-outs. After every move the placer puts the part back on the nearest of them, keeping its turn if
it can. A case where some part has no such place is left out (the run says why); if every case is,
the program stops. A keep-out is, for any other part, a very expensive cost like a collision for now
(see "Next steps"): if one can't be kept (no room left), the run ends with a `!!` line naming the
part. Coordinates are holes, counted like the numbers around the
text board. Pins are named like the `nets` command prints them.

## The pieces

| File | What it does |
|---|---|
| `circuits/555_blinker.net` | The example circuit as a KiCad netlist: the parts, their footprints, and which pins connect (nets). |
| `circuits/555_blinker.json` | Its board file: board size, power and ground, and the rules for this board. |
| `circuits/cat_feeder.diy` | A copy of the cat-feeder's `pcb.diy` (2026-10-17), with `cat_feeder.json`: its star wiring on C1 and R1 near GPIO2. |
| `TODO.md` | Ideas for later, in Filippo's words: mostly a richer board-file format. |
| `src/constraints.rs` | Reads the board file and keeps its rules: where parts may go (fixed, edge, orientation, within, all-of, one-of), keep-outs, near, star. |
| `footprints.json` | Footprint name → size in holes and where each pad is. |
| `src/sexpr.rs` | Reads KiCad's bracket format, `(comp (ref "R1") ...)`, into a tree. |
| `src/netlist.rs` | Turns a KiCad netlist plus `footprints.json` into the circuit the placer works on. |
| `src/board.rs` | Places parts, works out where every pin lands, measures the board and gives it a score. |
| `src/route.rs` | The router: joins every net with copper traces, and hops over with a jumper (or, last, a solder-side wire) where a trace can't get through. |
| `src/svg.rs` | Draws the layout as an SVG picture, from the parts' side or the solder side. |
| `src/kicad_pcb.rs` | Writes the layout as a KiCad board. |
| `src/diylc_import.rs` | Reads a DIYLC sketch: its parts, its modules (groups), and which pins connect. |
| `src/diylc.rs` | Writes the layout as a DIYLC project (your own parts, moved, if it came from one). |
| `src/place.rs` | The auto-placer: keeps nudging/rotating/swapping parts and keeps changes the judge likes. |
| `src/nn.rs` | The neural network, written out by hand (8 hidden neurons, 81 weights). |
| `src/main.rs` | The commands: `place`, `train`, `evolve`, `nets` and `check`; `place` and `check` place every case of the board file's rules and keep the best. `train` and `evolve` make ~1,200 example boards (going through the board file's cases in turn, up to 12), have the rules score them, and train the network to match. |

## How a board is judged

`src/board.rs` measures eight things (weights in `WEIGHTS`):

- **wire**: total length of copper trace, in holes, as routed (below).
- **jumpers**: how many jumpers (on the parts' side) the router needed where a trace couldn't get through.
- **jumper_len**: total length of jumpers and solder-side wires. A hole of it costs twice a hole of
  trace, so they stay short.
- **bottom**: how many wires on the solder side, the last resort. Each costs as much as six jumpers.
- **power**: for every supply pin, the distance to the nearest ground pin (keeps supply and ground close).
- **overlap**: holes where two parts collide, a part hangs off the board, sits on a keep-out, or sits under a module without being low enough (very expensive).
- **area**: the size of the rectangle that holds all parts (keeps the board compact).
- **near**: for each `near` rule in the board file, the length of copper between its two pins, as routed.

The weighted sum is the *cost*; the score is `100 * exp(-cost / 150)`, so 100 is perfect.
The placer itself compares costs, not scores: for a big circuit the score is low and flat (a cost
of 400 scores about 6), so comparing scores would make it accept almost any bad move. Scores are
for reading; for a big circuit they're low even when the layout is good.

### How it routes

Every placer step routes the whole board (`src/route.rs`), so the placer sees which layouts need jumpers.

Each net is one run, or one run per branch for a star net. Two runs never share copper or wire, even
two branches of the same star net.

- A **copper trace** is on the solder side. It goes hole to neighbouring hole: across, down, or at 45°.
  It touches every hole it passes, so it can't pass a hole with another run's pin or trace, or a
  keep-out, and two diagonals can't cross between four holes. It can run under parts, since it's on
  the other side.
- A **jumper** is a wire on the parts' side, used where traces can't get through: traces run up to a
  free hole, the jumper hops over, and traces carry on from where it lands. It goes across, down or at
  45°, 2 to 10 holes long. It can't pass over a part, a pin or a keep-out (it can go under a module's
  board, like low parts), or over or across another run's jumper. A jumper is like a part: its legs
  need holes of their own, never a pin's. It may lie over a hole where a solder-side wire ends:
  hopping over things is what it's for.
- A **wire on the solder side** is the last resort: it's only used when no jumper fits, however long
  the traces to a jumper would be. Same shape and legs, but nothing sits on that side except copper,
  so it can pass under parts and over traces and pins (it's insulated). It still keeps off keep-outs
  and other runs' solder-side wires.

Runs are routed one at a time, shortest first. Each run grows like a tree: from what's joined so far,
the cheapest way to the nearest pin not joined yet, trying holes in the direction of that pin first
("A*" search). Two kinds of solder-side wire break the rules above, and both still count as one:

- If no wire fits on either side (walled in on both ends), a wire goes straight from the nearest hole
  already joined to the pin, at any angle and length. It may end on the pin and cross other wires or a
  keep-out; wires routed after it keep off it.
- A pin of a part hanging off the board gets one straight from its run's first pin, so hanging a part
  off the board never saves wiring. (The part is boxed in red too.)

In the pictures, jumpers are solid blue and solder-side wires dashed red. In the `.diy`, both are
DIYLC jumpers (DIYLC has no wire on the solder side), the solder-side ones red and named "(solder side)".

## Two ways to train the network

Both commands train the same 9-neuron network on the same example boards, then test it on
boards it never saw and save it to `out/model.json` (so the last one you ran is the judge).

- **`train` (gradient descent):** for each board, compare the network's guess with the rules'
  score and nudge every weight in the direction that makes it less wrong. One example points
  all 81 weights the right way at once.
- **`evolve` (evolution):** make 19 copies of the network with every weight changed a little at
  random, keep whichever of the 20 is least wrong, repeat 1,000 times. It never works out a
  direction; it only needs to know which copy did better.

Typical result on the 555: `train` is off by about 0.8 points on unseen boards in 1.8s, `evolve` by
1 to 1.5 points (it depends on the seed) in 2.9s, after trying 19,000 networks. When you have a right answer for every example,
gradient descent wins. Evolution is the tool when all you can say is "this one did better than that one",
such as a network that proposes whole layouts.

## Checking a change

Run both circuits and look at the pictures (`out/rules.svg`, `out/rules-solder.svg`) and the time
printed at the end. After a change to routing, the DIYLC files or the board file, also check that the
`.diy` the program writes still has exactly the connections it was given:

```bash
cargo run --release -- check circuits/cat_feeder.diy --seed 2   # one file
cargo run --release -- check my_sketches/ --steps 2000          # every .diy in a folder
```

`check` places each file as `place` does (every case, keeping the best, with a line for each case
when there's a `one-of`), writes the `.diy` (to `out/check.diy`, overwritten for each
file), reads it back and compares the connections. For each file it says one of:

- **same**: passes (only counted).
- **WRONG**: the written file has different connections; it lists the nets that differ.
- **STOPPED**: the program stopped after reading the file (in the placer, the router, the writer, or
  reading back what it wrote), with the message and where in the code.
- **unfinished**: the connections differ, but when the placer stopped some pins still shared a hole or
  hung off the board, which joins nets by itself. Try more `--steps`.
- **skipped**: no connections, or the reader refuses the file (a part it doesn't support), with why.

It exits with an error if any file was WRONG or STOPPED. Try two or three seeds: a layout where
something goes wrong only shows up with some of them. A folder of many sketches is the best test; it
found the tactile-switch problem above.

After changing what `measure` returns, run `train` again. `place --judge net` only refuses a model trained on a different *number* of
measurements; one trained when a measurement meant something else loads silently and judges wrong.

## Known limits

- **Speed.** Every placer step routes the whole board, and each case of a `one-of` is placed in full,
  one after the other (times at the top). Many one-ofs side by side multiply the cases. Re-routing
  only what a move changes, or placing the cases at the same time, would make it quicker; neither is
  built.
- **A case starts from your sketch.** With a `one-of`, a case that moves a part far from where you
  drew it (to the other edge) starts with that part on top of others, and may still have collisions
  when the steps run out; try more `--steps`.
- **The router is greedy.** It routes one run at a time, shortest first, and never undoes one. A
  different order can sometimes avoid a jumper it needed. A connection gets at most one wire; if that's
  not enough it gets one solder-side wire straight to the pin.
- **Collisions and keep-outs are only a cost.** They're the `overlap` measurement, heavily weighted, so
  the placer avoids them, but with `--judge net` it's the network that decides how bad they are.
  (A part with placement rules never sits on a keep-out or off the board.)
- **KiCad has no traces.** `rules.kicad_pcb` has the parts and their nets, not the routed copper.
- **DIP pin names from DIYLC.** Reading a `.diy`, a DIP chip's right-hand pins are numbered top to
  bottom (5–8) where the chip's own numbering runs bottom to top, so the top-right pin is called 5,
  not 8. The connections are right; only the names are off. Check with `nets` before naming such a pin
  in a board file.
- **Angled parts from DIYLC.** A two-legged part drawn at an angle only takes its two leg holes, so the
  placer treats it like a module: low parts may go under it, and jumpers may pass over it.
- **Scores of big circuits are low.** The weights were set on the 555; the cat-feeder board scores
  about 5–15 even when it's good. Compare costs between layouts of the same circuit, not across circuits.

## Principles

- **The program is Rust only**, including the machine learning. Dev tools around it (checkers,
  converters, plots) may be in Python, but nothing the program needs to run.
- **Hard rules, preferences, taste.** Facts about the hardware and limits of a board (fixed parts,
  edges, keep-outs, collisions) should be hard rules the placer enforces, never learned; fixed parts
  and the other placement rules already are, the rest is "Next steps" 1. What one board wants goes in its board file:
  `near` with a fixed cost per hole, `star` as how its nets are split into runs. The network is for
  taste among layouts that are already allowed.
- **Model settings are the author's job.** Hidden neurons, learning rates, generations and the rule
  weights are constants in the code, not flags. Flags are for your problem: the circuit, the board,
  power and ground, the seed, how hard the placer tries.
- **Plain code over clever code**, with comments that explain why in plain words.

## Where the neural network fits

Right now the network learns to copy the rules. On boards it has never seen it is off by
about a point, and the placer does just as well using it as the judge. On its own that
isn't useful, since we already have the rules. It proves the plumbing works.

It becomes useful when the "right answers" come from something the rules can't express:

1. **Your taste.** Rate layouts yourself; the network learns what you like, including things
   that are hard to write down as a rule.
2. **Richer input.** Feed it the board itself (which hole holds what) instead of eight numbers,
   so it can spot patterns no rule measures. That's when a library like Burn starts paying off.
3. **Placing directly.** Train a network that proposes a layout in one shot instead of the
   placer trying thousands of moves.

## Next steps

In order:

1. **Split "allowed" from "good".** Collisions, hanging off the board and keep-outs become exact
   checks outside the score, so the score (and the network) only compares layouts that are allowed.
2. **A `rate` command**: show two layouts, you pick the better one, and the network learns your taste
   from those picks ("Your taste" above). Then more circuits to learn from.
3. **Faster routing**, if waits grow past an espresso (see Known limits).

Smaller ideas, in Filippo's words, are in [TODO.md](TODO.md).
