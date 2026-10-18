# Harmonia: perfboard layout + a tiny neural network

Before it meant harmony, the Greek ἁρμονία was a carpenter's word for a joint: things fitted together.
Harmonia turns parts, fits them on a perfboard and wires them up.

Rust, with four small crates (`rand`, `serde`, `serde_json`, and `roxmltree` to read DIYLC files).
Every placer step routes the whole board, so commands take seconds: on the 555, `place` takes about
1.9s, `train` 5s and `evolve` 6s; placing the bigger cat-feeder board about 11s (its board file makes two cases). A board file
whose `one-of`s make cases (below) is placed once per case, so it takes that many times longer. Anything up to
the time to make and drink an espresso is fine.

```bash
cargo run --release -- place                # auto-place the 555 blinker using the hand-written rules
cargo run --release -- train                # teach the tiny network to score boards like the rules do
cargo run --release -- evolve               # the same, by evolution instead (1 parent + 19 children)
cargo run --release -- place --judge net    # auto-place again, but let the network be the judge
cargo run --release -- place my_sketch.diy  # score a rough DIYLC layout, then fit it on the board
cargo run --release -- nets my_sketch.diy   # just list the parts and connections it understood
cargo run --release -- check my_sketches/   # place, write and re-read every .diy: same connections? (see below)
cargo run --release -- draw out/rules.layout.json  # draw a saved layout again, without placing it (see below)
cargo run --release -- bench                # the benchmark: place a fixed set of circuits, record the costs (see below)
```

Options: `--steps 50000` (placer effort), `--seed 7` (a different random start), `--board 24x16`
(perfboard size in holes, default 18x12), `--power +12V` / `--ground GND` (which nets are supply and
ground, guessed from common names like `+5V`, `VCC` and `GND` if left out), or a path to another netlist.
Board size, power and ground can also live in the board file (below); a flag wins over it.

**Without Rust:** each [release](https://github.com/filosganga/harmonia/releases) has the program
ready-built for Linux (x86-64), macOS (Apple silicon) and Windows (x86-64). Unpack it and run it from
inside its folder, which also holds `footprints.json` and the example circuits: `./harmonia place`,
`./harmonia place circuits/cat_feeder.diy`, and so on, the same commands without `cargo run --release --`.

Each `place` run prints the board as text (when the board file's `one-of`s make cases, first a line
for each case and which was best) and writes, in `out/` (named `rules` or `net` after the judge):

- `rules.layout.json`: the finished board as one plain file (below). The two pictures, the KiCad
  board and the DIYLC project are all drawn from it, and so is the result in the html.
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
  A connection the router couldn't make (a `!!` line says which) is missing from it.

### The layout file

`rules.layout.json` describes the finished board, so the drawings (and any tool of your own) don't
have to work it out again. It's JSON, with `"format": "harmonia-layout"` and a `"version"` number (a
file of another version is refused, with a message):

- **made**: how it was made: the circuit file, the board file, the judge, where the placer started
  (`random` or `sketch`), the seed, the steps, and which case of the board file's rules
  won (counted from 1, out of how many, and the option it took in each one-of). And a **title**, the
  one the pictures show.
- **board**: its width and height in holes, and the keep-outs (name, from, to).
- **power** and **ground**: the two net names.
- **parts**: each part's reference, value and footprint; `at` and `rot`, meaning what they mean in
  the board file's `fixed` rule; its `size` in holes before turning, whether it's `low`, the
  `"diylc"` entry from `footprints.json` if it has one, and its pins: name, `at` (its hole inside the
  part, before turning), `hole` (where that is on the board), net, and `off_grid` for a pin between
  holes. A part that doesn't take its whole outline (a module, or a DIYLC part drawn at an angle)
  also has its `body`, the holes it takes; a part from a DIYLC sketch has its `diylc_drawing`, your
  drawing of it, which is how the `.diy` keeps it.
- **nets**: each net's name and pins.
- **runs**: what the router joins with one piece of copper (a net, or a branch of a star net, with
  its `star` point), in the order it laid them, with that copper: `traces` as straight pieces,
  `[from hole, to hole]`, then `jumpers` and `solder_wires` the same way, and `unrouted` (only when
  there are any): connections the router couldn't make.
- **score**: the score, the cost and the twelve measurements by name (below), and the `!!` **problems**.

Not in it: the net colours (worked out from the nets), the start (only `rules.html` shows it), and
the board file's rules about where parts may go, `near` and `close`.

`draw` reads a layout file and draws it again next to it: `draw out/rules.layout.json` writes
`out/rules.svg`, `out/rules-solder.svg`, `out/rules.kicad_pcb` and `out/rules.diy`, the same files
`place` wrote, without placing or routing anything. It uses the copper and the measurements as they
were saved, so a layout saved before a change to the router draws as it did. (The score and cost
beside the board are worked out from those measurements with today's weights; a file measured on
other things than today's measurements is refused.) Copy a
layout file somewhere first to keep it: the next `place` overwrites `out/rules.layout.json`.

## Using your own circuit

1. Draw the schematic in KiCad and give every part a through-hole footprint.
2. In the schematic editor: File → Export → Netlist, KiCad format. That writes a `.net` file.
3. `cargo run --release -- place path/to/your.net`

A netlist only says what connects to what. It doesn't say where a part's legs are, so each footprint
also needs an entry in `footprints.json`: its size in holes and which hole each pad sits in (top-left
hole is `[0, 0]`, a hole is 2.54mm). If one is missing, the program names it. The library name before
the `:` is ignored, so `Package_DIP:DIP-8_W7.62mm` is looked up as `DIP-8_W7.62mm`. CONTRIBUTING.md
"Adding a footprint" says how to measure one; `dev/kicad_footprint.py` drafts it from KiCad's own file.

The optional `"diylc"` entry says how to draw the part in DIYLC: `Resistor`, `RadialElectrolytic`,
`RadialCeramicDiskCapacitor`, `LED`, `DIL_IC` or `PinHeader`. For two-legged parts, `"legs"` says which pad is
DIYLC's first leg: that's + on a capacitor but the anode on an LED, which is KiCad's pad 2. Without a
`"diylc"` entry the part is drawn as a labelled box.

`"low": true` marks a part low enough to sit under a module on headers (see below); leave it out otherwise.
`"module": true` marks a module on headers (an Arduino Nano, a Pico, a motor driver breakout): only its
pins take holes, and low parts can go under its board, like a DIYLC group (below). `footprints.json`
has five, from KiCad's `Module` library.

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
  module's board and some labels. Its outline is the rectangle's holes: a hole counts if the rectangle
  reaches clearly past its centre, so a board drawn with its edge on a line of holes ends there (the
  same rule as for footprints). It moves as a whole, and only its pin holes are taken: **low parts**
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
    { "kind": "close",   "pins": ["D1.2", "U1.3"] },
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
  any parts, and nest. A one-of makes cases (except the kind below): "A, one-of (B1, B2), C" is placed twice, as (A, B1, C)
  and as (A, B2, C), and the best layout is kept. A one-of inside a one-of just adds options, an
  all-of keeps its rules together in one option, and two one-ofs side by side multiply (2 and 3
  options make 6 cases). `nets` lists the cases; an option is named by its `name`, or its number.
  A one-of only about where one part goes (its options are fixed, edge, orientation and within
  rules, or all-ofs of them, about the same part) makes no cases: the part may take any place one
  of its options allows, and the placer tries them all in the same run.
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
- **close**: two pins of any nets should be as close together as possible, straight across the
  board. For a connection the board doesn't carry, like a wire from pads under a module to a header.
  A preference, at the same cost per hole as near, and counted with it.
- **star**: how the net of the `at` pin is wired. Every run starts at `at` and goes its own way, never
  sharing copper with another run, so one part's current doesn't shift the voltage another part sees.
  Each run in `runs` joins its pins in a chain; any other pin of the net gets a run of its own, straight
  from `at`. A net can have one star rule.

fixed, edge, orientation and within are hard limits. In each case, each part they're about gets the
list of places (position and turn) that meet all its rules, while staying on the board and off the
keep-outs. After every move the placer puts the part back on the nearest of them, keeping its turn if
it can; now and then it also jumps the part to any of them, so it can get across to another option
of a one-of, like the other edge. A case where some part has no such place is left out (the run says why); if every case is,
the program stops. A keep-out is, for any other part, a very expensive cost like a collision for now
(see "Next steps"): if one can't be kept (no room left), the run ends with a `!!` line naming the
part. Coordinates are holes, counted like the numbers around the
text board. Pins are named like the `nets` command prints them.

## The pieces

| File | What it does |
|---|---|
| `circuits/555_blinker.net` | The example circuit as a KiCad netlist: the parts, their footprints, and which pins connect (nets). |
| `circuits/555_blinker.json` | Its board file: board size, power and ground, and the rules for this board. |
| `circuits/cat_feeder.diy` | A copy of the cat-feeder's `pcb.diy` (2026-10-17; the RTC's board rectangle since drawn one hole wider on each long side, to its real 8 holes), with `cat_feeder.json`: 5 V and ground starred from where the supply comes in (Q1's cathode, the cable's ground), the driver with its own runs and C1 near it, R1 near GPIO2, `Pads` close to GP5 or GP22 (the ESP32's back pads), the ESP32's USB side on one of the board's edges, and the two cable headers (`Inlet`, `Ext`) on an edge or one hole in. |
| `TODO.md` | The maintainer's own list of ideas for later. |
| `src/constraints.rs` | Reads the board file and keeps its rules: where parts may go (fixed, edge, orientation, within, all-of, one-of), keep-outs, near, close, star. |
| `circuits/bench.txt` | The benchmark set: the circuits `bench` places. Never used to train a network. |
| `circuits/bench/` | Ten classic hobby circuits for the benchmark (a 7805 supply, a transistor flasher, an LM386 amp, a 555 + 4017 LED chaser, ...), written as KiCad netlists after datasheet applications and well-known designs, each with a board file (its size, power, ground, and the connectors on edges). |
| `footprints.json` | Footprint name → size in holes and where each pad is. |
| `dev/kicad_footprint.py` | A dev tool (Python, not needed to run the program): drafts a `footprints.json` entry from a KiCad footprint file. |
| `src/sexpr.rs` | Reads KiCad's bracket format, `(comp (ref "R1") ...)`, into a tree. |
| `src/netlist.rs` | Turns a KiCad netlist plus `footprints.json` into the circuit the placer works on. |
| `src/board.rs` | Places parts, works out where every pin lands, measures the board and gives it a score. |
| `src/route.rs` | The router: joins every net with copper traces, and hops over with a jumper (or, last, a solder-side wire) where a trace can't get through; a pin walled in so that not even a wire fits is left unrouted. |
| `src/layout.rs` | The finished board, routed and measured once (`Layout`), which the three writers below draw; and the layout file: writing it, and reading it back for `draw`. |
| `src/svg.rs` | Draws the layout as an SVG picture, from the parts' side or the solder side. |
| `src/kicad_pcb.rs` | Writes the layout as a KiCad board. |
| `src/diylc_import.rs` | Reads a DIYLC sketch: its parts, its modules (groups), and which pins connect. |
| `src/diylc.rs` | Writes the layout as a DIYLC project (your own parts, moved, if it came from one). |
| `src/place.rs` | The auto-placer: keeps nudging/rotating/swapping parts (and jumping a part with placement rules to another place they allow) and keeps changes the judge likes. |
| `src/nn.rs` | The neural network, written out by hand (8 hidden neurons, 113 weights). |
| `src/bench.rs` | The benchmark: places every circuit of `circuits/bench.txt` with three seeds, several runs at once, and records the best costs. |
| `src/main.rs` | The commands: `place`, `train`, `evolve`, `nets`, `check`, `draw` and `bench`; `place` and `check` place every case of the board file's rules and keep the best. `train` and `evolve` make ~1,200 example boards (going through the board file's cases in turn, up to 12), have the rules score them, and train the network to match. |

## How a board is judged

`src/board.rs` measures twelve things (weights in `WEIGHTS`):

- **wire**: total length of copper trace, in holes, as routed (below).
- **turns**: how hard the traces' bends are to make. Traces are laid with copper wire, and each
  bend is one more to make by hand, so a straight trace beats a zig-zag. A right angle, the easiest,
  counts 1; a gentle 45° turn, onto or off a diagonal, counts 2, since it's harder to land on the
  right hole; a sharp 135° turn counts 3. A pin or a wire's leg is free, since a trace starts
  there, and so is a junction where three or more steps meet, which is soldered like a pin.
  Jumpers never bend: each is one straight wire.
- **diagonal**: steps of copper trace laid at 45°, each costing three holes of trace on top of its
  length. Straight traces are easier to lay along the holes and line up side by side; a board full of
  diagonals (V shapes and staircases) is hard to build and couldn't be a stripboard. So the router
  takes a diagonal only where nothing straight gets through. Jumpers and solder-side wires are wires,
  free to go at 45°: they don't count.
- **crosswise**: straight steps of copper trace in the board's minority direction, half a hole each:
  whichever of across and down this layout's traces take less (the other is the main direction). On
  a stripboard the strips run one way and copper the other way needs a wire; on perfboard too,
  parallel traces in one direction are the easiest to lay. So the placer prefers layouts where most
  copper runs one way. Some always runs the other way, since not every pin of a net can be on one line.
- **jumpers**: how many jumpers (on the parts' side) the router needed where a trace couldn't get through.
- **jumper_len**: total length of jumpers and solder-side wires. A hole of it costs twice a hole of
  trace, so they stay short.
- **bottom**: how many wires on the solder side, the last resort. Each costs as much as twenty jumpers,
  so the placer finds room for jumpers instead.
- **unrouted**: connections the router couldn't make at all: a pin walled in by other copper so that
  not even a wire fits (a wire's leg needs a hole of its own, never a pin's), or a pin off the board.
  Each costs a thousand holes of trace, far more than any wiring, so the placer keeps away from such
  layouts; one that's left is drawn as a thin dotted line, with a `!!` line naming the pin.
- **power**: for every supply pin, the distance to the nearest ground pin (keeps supply and ground close).
- **overlap**: holes where two parts collide, a part hangs off the board, sits on a keep-out, or sits under a module without being low enough (very expensive).
- **crowding**: pairs of neighbouring holes (across or down) taken by two different parts, where the
  soldering iron has no room. A pair of pins a solder blob would wrongly join (on different nets, or
  one on a net and one unused) counts twice. So parts get room around them and spread over the board. Beyond that, being far
  apart earns nothing, and wire length keeps them from drifting. If you want a smaller layout, give
  it a smaller board.
- **near**: for each `near` rule in the board file, the length of copper between its two pins, as
  routed; and for each `close` rule, the distance between its two pins, straight across the board.

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
  the other side. A right angle costs as much as a hole of trace, a 45° turn two and a 135° turn three
  (the `turns` weight), so the router takes a straight way over a slightly shorter zig-zag. A diagonal
  step pays three holes on top of its length (the `diagonal` weight), so the router goes across and
  down instead unless nothing straight gets through. Making turns dearer than that was tried: the
  router then detours, boxes later runs in, and needs more jumpers. The `crosswise` measurement isn't
  priced by the router, since the main direction is only known once the board is routed; a trace
  from one pin to another takes the same steps across and down whichever way it turns.
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
("A*" search). A pin that can't be reached, because it's walled in so that not even a wire fits or
it's off the board, is left out of its run: `unrouted`, above. No wire ever ends on a pin.
(A part hanging off the board is boxed in red too.)

In the pictures, jumpers are solid blue and solder-side wires dashed red. In the `.diy`, both are
DIYLC jumpers (DIYLC has no wire on the solder side), the solder-side ones red and named "(solder side)".

## Two ways to train the network

Both commands train the same 9-neuron network on the same example boards, then test it on
boards it never saw and save it to `out/model.json` (so the last one you ran is the judge).

- **`train` (gradient descent):** for each board, compare the network's guess with the rules'
  score and nudge every weight in the direction that makes it less wrong. One example points
  all 113 weights the right way at once.
- **`evolve` (evolution):** make 19 copies of the network with every weight changed a little at
  random, keep whichever of the 20 is least wrong, repeat 1,000 times. It never works out a
  direction; it only needs to know which copy did better.

Typical result on the 555: `train` is off by about 0.7 points on unseen boards (0.6 to 0.8) in 5s, `evolve` by
about 1.9 points (1.3 to 2.3, depending on the seed) in 6s, after trying 19,000 networks. When you have a right answer for every example,
gradient descent wins. Evolution is the tool when all you can say is "this one did better than that one",
such as a network that proposes whole layouts.

## Checking a change

Run both circuits and look at the pictures (`out/rules.svg`, `out/rules-solder.svg`) and the time
printed at the end. After a change to routing, the DIYLC files, the board file, the layout file or
any of the drawings, also check that the `.diy` the program writes still has exactly the connections
it was given (and that the layout file keeps everything the drawings need):

```bash
cargo run --release -- check circuits/cat_feeder.diy --seed 2   # one file
cargo run --release -- check my_sketches/ --steps 2000          # every .diy in a folder
```

`check` places each file as `place` does, from the sketch (every case, keeping the best, with a line
for each case when there are several), writes the `.diy` (to `out/check.diy`, overwritten for each
file), reads it back and compares the connections. It also writes the layout file (`out/check.layout.json`),
reads it back, and checks that what was read writes the same layout file again and the same `.diy`,
pictures and KiCad board as the layout in memory, so the layout file keeps everything the drawings
need. For each file it says one of:

- **same**: passes (only counted).
- **WRONG**: the written file has different connections; it lists the nets that differ. Or the
  layout file read back came out different; it says which of the files did.
- **STOPPED**: the program stopped after reading the file (in the placer, the router, the writer, or
  reading back what it wrote), with the message and where in the code.
- **unfinished**: the connections differ, but when the placer stopped some pins still shared a hole,
  hung off the board (which joins nets by itself) or couldn't be reached by the router. Try more `--steps`.
- **skipped**: no connections, or the reader refuses the file (a part it doesn't support), with why.

It exits with an error if any file was WRONG or STOPPED. Try two or three seeds: a layout where
something goes wrong only shows up with some of them. A folder of many sketches is the best test; it
found the tactile-switch problem above.

After changing what `measure` returns, run `train` again. `place --judge net` only refuses a model trained on a different *number* of
measurements; one trained when a measurement meant something else loads silently and judges wrong.

### The benchmark

After a change meant to make layouts better (the placer, the router, a network), run the benchmark
and compare the costs with the last run (`out/bench.json`), made before the change:

```bash
cargo run --release -- bench
```

It places each circuit listed in `circuits/bench.txt` (the 555, the cat feeder and the hobby
circuits in `circuits/bench/`) from a random start, with three seeds and `place`'s default steps,
the way `place` does, and prints each seed's best cost, the best and mean of the three, and the time
per run. It takes no options, so every run measures the same thing. The runs go several at a time,
one per processor core (about 21s on an 18-core Mac); each has its own random numbers from its seed,
so the costs are the same as running them one by one. Costs only compare within a circuit, and only
under the same weights: a change to a measurement or a weight changes what a cost means, so run the
benchmark before and after it and compare the layouts, not the old numbers.

## Known limits

- **Speed.** Every placer step routes the whole board, and each case is placed in full, one after
  the other (times at the top). One-ofs that make cases, side by side, multiply them. Re-routing
  only what a move changes, or placing the cases at the same time, would make it quicker; neither is
  built.
- **Layouts depend on the seed.** On the cat feeder, different seeds land on quite different layouts
  and costs, and a part whose rules send it far from where you drew it starts on top of others. Try
  more `--steps` and two or three `--seed`s, and compare the costs.
- **The router is greedy.** It routes one run at a time, shortest first, and never undoes one. A
  different order can sometimes avoid a jumper it needed. A connection gets at most one wire; if that's
  not enough, the pin is left unrouted (very expensive, so the placer finds another layout).
- **Big penalties make the placer's search stiff.** A collision costs more than a solder-side wire
  and an unrouted pin far more, so the placer rarely lets a part pass through another to reach a
  better place. A crowded board started at random can end with collisions or unrouted pins: the cat
  feeder in `bench` (random starts) does, though placed from the sketch it doesn't, and short `check`
  runs on sketches with modules drawn on top of each other end unfinished more often. Making
  collisions an exact check instead of a cost ("Next steps" 1) is the way out.
- **Collisions and keep-outs are only a cost.** They're the `overlap` measurement, heavily weighted, so
  the placer avoids them, but with `--judge net` it's the network that decides how bad they are.
  (A part with placement rules never sits on a keep-out or off the board.)
- **KiCad has no traces.** `rules.kicad_pcb` has the parts and their nets, not the routed copper.
  The copper is in the layout file (`runs`) and in the `Layout` that `src/kicad_pcb.rs` is given,
  so writing it into the KiCad board only needs that writer to draw it.
- **DIP pin names from DIYLC.** Reading a `.diy`, a DIP chip's right-hand pins are numbered top to
  bottom (5–8) where the chip's own numbering runs bottom to top, so the top-right pin is called 5,
  not 8. The connections are right; only the names are off. Check with `nets` before naming such a pin
  in a board file.
- **Angled parts from DIYLC.** A two-legged part drawn at an angle only takes its two leg holes, so the
  placer treats it like a module: low parts may go under it, and jumpers may pass over it.
- **Scores of big circuits are low.** The weights were set on the 555; the cat-feeder board scores
  in single digits even when it's good. Compare costs between layouts of the same circuit, not across circuits.

## Principles

- **The program is Rust only**, including the machine learning. Dev tools around it (checkers,
  converters, plots) may be in Python, but nothing the program needs to run.
- **Hard rules, preferences, taste.** Facts about the hardware and limits of a board (fixed parts,
  edges, keep-outs, collisions) should be hard rules the placer enforces, never learned; fixed parts
  and the other placement rules already are, the rest is "Next steps" 1. What one board wants goes in its board file:
  `near` and `close` with a fixed cost per hole, `star` as how its nets are split into runs. The network is for
  taste among layouts that are already allowed.
- **Model settings are the author's job.** Hidden neurons, learning rates, generations and the rule
  weights are constants in the code, not flags. Flags are for your problem: the circuit, the board,
  power and ground, the seed, how hard the placer tries.
- **Plain code over clever code**, with comments that explain why in plain words.

## Where the neural network fits

Right now the network learns to copy the rules. On boards it has never seen it is off by
under a point, but as the placer's judge it fails: with today's big penalties (collisions,
solder-side wires, unrouted pins) almost every example board scores about 0, so it learns little
more than "bad", and `place --judge net` leaves parts on top of each other (on the 555, seeds 1-3:
19 to 21 holes of collision, costs about 3,000 against the rules' 104 to 109). Training it on the
cost instead of the 0-100 score would likely help. Even at its best it isn't useful on its own,
since we already have the rules. It proves the plumbing works.

It becomes useful when the "right answers" come from something the rules can't express:

1. **Your taste.** Rate layouts yourself; the network learns what you like, including things
   that are hard to write down as a rule.
2. **Richer input.** Feed it the board itself (which hole holds what) instead of twelve numbers,
   so it can spot patterns no rule measures. That's when a library like Burn starts paying off.
3. **Placing directly.** Train a network that proposes where the parts go, so the placer starts
   from a good layout instead of a random one. Tried and parked:
   [docs/plans/placing-network.md](docs/plans/placing-network.md) (the code is on the branch `better-placing`).

## Next steps

In order:

1. **Split "allowed" from "good".** Collisions, hanging off the board and keep-outs become exact
   checks outside the score, so the score (and the network) only compares layouts that are allowed.
2. **A `rate` command**: show two layouts, you pick the better one, and the network learns your taste
   from those picks ("Your taste" above). Then more circuits to learn from.
3. **Faster routing**, if waits grow past an espresso (see Known limits).

A network that places parts was built and parked (on the branch `better-placing`); see
[docs/plans/placing-network.md](docs/plans/placing-network.md).

Smaller ideas, in the maintainer's words, are in [TODO.md](TODO.md).

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, check a change, and how the project is shaped.

Harmonia is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
