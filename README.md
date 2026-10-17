# Harmonia: perfboard layout + a tiny neural network

Before it meant harmony, the Greek ἁρμονία was a carpenter's word for a joint: things fitted together.
Harmonia turns parts, fits them on a perfboard and wires them up.

Rust, with four small crates (`rand`, `serde`, `serde_json`, and `roxmltree` to read DIYLC files), and
[Burn](https://burn.dev) for the placing network, which is big: it brings about 180 more crates, and a
clean `cargo build --release` takes about 40s instead of 7s.
Every placer step routes the whole board, so commands take seconds: on the 555, `place` takes about
2.4s, `train` 6s and `evolve` 7s; placing the bigger cat-feeder board about 16s (its board file makes two cases). A board file
whose `one-of`s make cases (below) is placed once per case, so it takes that many times longer. Anything up to
the time to make and drink an espresso is fine. One exception: `learn`, training the placing network,
takes about 14 minutes; for training, accuracy matters more than speed.

```bash
cargo run --release -- place                # auto-place the 555 blinker using the hand-written rules
cargo run --release -- train                # teach the tiny network to score boards like the rules do
cargo run --release -- evolve               # the same, by evolution instead (1 parent + 19 children)
cargo run --release -- place --judge net    # auto-place again, but let the network be the judge
cargo run --release -- place --start net    # start the placer from the placing network's layout (see below)
cargo run --release -- place my_sketch.diy  # score a rough DIYLC layout, then fit it on the board
cargo run --release -- nets my_sketch.diy   # just list the parts and connections it understood
cargo run --release -- check my_sketches/   # place, write and re-read every .diy: same connections? (see below)
cargo run --release -- draw out/rules.layout.json  # draw a saved layout again, without placing it (see below)
cargo run --release -- bench                # the benchmark: place a fixed set of circuits, record the costs (see below)
cargo run --release -- data                 # add a batch of placed random circuits, to train a placing network on (see below)
cargo run --release -- learn                # train the placing network on them and test it on circuits it never saw
```

Options: `--steps 50000` (placer effort), `--seed 7` (a different random start), `--start net` (start
from the placing network's layout instead, below), `--board 24x16`
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
- `rules.html`: the start (random, your sketch, or the placing network's layout with `--start net`) and the result side by side, drawn the same way.
- `rules.kicad_pcb`: the layout for KiCad's PCB editor (KiCad 10 opens it). Every pad is on its net, so
  KiCad shows the connections still to wire (the routed traces aren't in it yet). The footprints are
  simple stand-ins built from `footprints.json`; Tools → Update Footprints from Library swaps in KiCad's
  real ones without moving anything.
- `rules.diy`: the layout for DIY Layout Creator, on a perfboard, with the routed copper traces (solder
  side) and jumpers, coloured by net (power red, ground near-black); solder-side wires as red jumpers.

### The layout file

`rules.layout.json` describes the finished board, so the drawings (and any tool of your own) don't
have to work it out again. It's JSON, with `"format": "harmonia-layout"` and a `"version"` number (a
file of another version is refused, with a message):

- **made**: how it was made: the circuit file, the board file, the judge, where the placer started
  (`random`, `sketch` or `network`), the seed, the steps, and which case of the board file's rules
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
  `[from hole, to hole]`, then `jumpers` and `solder_wires` the same way.
- **score**: the score, the cost and the nine measurements by name (below), and the `!!` **problems**.

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
| `circuits/cat_feeder.diy` | A copy of the cat-feeder's `pcb.diy` (2026-10-17), with `cat_feeder.json`: 5 V and ground starred from where the supply comes in (Q1's cathode, the cable's ground), the driver with its own runs and C1 near it, R1 near GPIO2, `Pads` close to GP5 or GP22 (the ESP32's back pads), the ESP32's USB side on one of the board's edges, and the two cable headers (`Inlet`, `Ext`) on an edge or one hole in. |
| `TODO.md` | The maintainer's own list of ideas for later. |
| `src/constraints.rs` | Reads the board file and keeps its rules: where parts may go (fixed, edge, orientation, within, all-of, one-of), keep-outs, near, close, star. |
| `circuits/bench.txt` | The benchmark set: the circuits `bench` places. Never used to train a network. |
| `circuits/bench/` | Ten classic hobby circuits for the benchmark (a 7805 supply, a transistor flasher, an LM386 amp, a 555 + 4017 LED chaser, ...), written as KiCad netlists after datasheet applications and well-known designs, each with a board file (its size, power, ground, and the connectors on edges). |
| `footprints.json` | Footprint name → size in holes and where each pad is. |
| `dev/kicad_footprint.py` | A dev tool (Python, not needed to run the program): drafts a `footprints.json` entry from a KiCad footprint file. |
| `src/sexpr.rs` | Reads KiCad's bracket format, `(comp (ref "R1") ...)`, into a tree. |
| `src/netlist.rs` | Turns a KiCad netlist (or, for `data`, a list of parts and nets) plus `footprints.json` into the circuit the placer works on. |
| `src/board.rs` | Places parts, works out where every pin lands, measures the board and gives it a score. |
| `src/route.rs` | The router: joins every net with copper traces, and hops over with a jumper (or, last, a solder-side wire) where a trace can't get through. |
| `src/layout.rs` | The finished board, routed and measured once (`Layout`), which the three writers below draw; and the layout file: writing it, and reading it back for `draw`. |
| `src/svg.rs` | Draws the layout as an SVG picture, from the parts' side or the solder side. |
| `src/kicad_pcb.rs` | Writes the layout as a KiCad board. |
| `src/diylc_import.rs` | Reads a DIYLC sketch: its parts, its modules (groups), and which pins connect. |
| `src/diylc.rs` | Writes the layout as a DIYLC project (your own parts, moved, if it came from one). |
| `src/random_circuit.rs` | Makes random circuits from the parts in `footprints.json`, wired like a small hobby board, with a board file. The same number always makes the same circuit. |
| `src/data.rs` | The `data` command: places random circuits and keeps their layouts, the training data for a placing network; and the order that network places parts in. |
| `src/parallel.rs` | Runs independent jobs (placing several circuits) on every processor core at once. |
| `src/place.rs` | The auto-placer: keeps nudging/rotating/swapping parts (and jumping a part with placement rules to another place they allow) and keeps changes the judge likes. Starts at random, from a given layout, or from the placing network's (cooler, so it keeps the good start). |
| `src/nn.rs` | The neural network, written out by hand (8 hidden neurons, 89 weights). |
| `src/bench.rs` | The benchmark: places every circuit of `circuits/bench.txt` with three seeds, several runs at once, and records the best costs and layouts and how the cost fell; `--start net` starts from the placing network and compares with the plain placer. |
| `src/placing.rs` | The placing network (Burn): draws the board with the next part at each place it may take, scores each picture with a small convolutional network, and the `learn` command that trains and tests it; and `propose`, the network's whole layout, for `place --start net`. |
| `src/main.rs` | The commands: `place`, `train`, `evolve`, `nets`, `check`, `draw`, `bench`, `data` and `learn`; `place` and `check` place every case of the board file's rules and keep the best. `train` and `evolve` make ~1,200 example boards (going through the board file's cases in turn, up to 12), have the rules score them, and train the network to match. |

## How a board is judged

`src/board.rs` measures nine things (weights in `WEIGHTS`):

- **wire**: total length of copper trace, in holes, as routed (below).
- **turns**: how hard the traces' bends are to make. Traces are laid with copper wire, and each
  bend is one more to make by hand, so a straight trace beats a zig-zag. A right angle, the easiest,
  counts 1; a gentle 45° turn, onto or off a diagonal, counts 2, since it's harder to land on the
  right hole; a sharp 135° turn counts 3. A pin or a wire's leg is free, since a trace starts
  there, and so is a junction where three or more steps meet, which is soldered like a pin.
  Jumpers never bend: each is one straight wire.
- **jumpers**: how many jumpers (on the parts' side) the router needed where a trace couldn't get through.
- **jumper_len**: total length of jumpers and solder-side wires. A hole of it costs twice a hole of
  trace, so they stay short.
- **bottom**: how many wires on the solder side, the last resort. Each costs as much as six jumpers.
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
  (the `turns` weight), so the router takes a straight way over a slightly shorter zig-zag, and a
  right angle over two 45° turns unless the diagonal saves a lot. Making turns dearer than that
  was tried: the router then detours, boxes later runs in, and needs more jumpers.
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
  all 89 weights the right way at once.
- **`evolve` (evolution):** make 19 copies of the network with every weight changed a little at
  random, keep whichever of the 20 is least wrong, repeat 1,000 times. It never works out a
  direction; it only needs to know which copy did better.

Typical result on the 555: `train` is off by about 0.9 points on unseen boards (0.7 to 1.0) in 6s, `evolve` by
about 1.3 points (1.1 to 1.6, depending on the seed) in 7s, after trying 19,000 networks. When you have a right answer for every example,
gradient descent wins. Evolution is the tool when all you can say is "this one did better than that one",
such as a network that proposes whole layouts.

## Data for a placing network

A network that places parts (docs/plans/placing-network.md) learns from layouts the placer found.
`data` makes them:

```bash
cargo run --release -- data
```

Each run makes a batch of random circuits (parts from `footprints.json`: a supply header, chips with
a supply and a ground pin, capacitors and LEDs to ground, small signal nets, all joined up, on a board
about as crowded as the benchmark's, with some connectors on edges; the sizes are constants in
`src/random_circuit.rs` and `src/data.rs`), places each from a random start with
`place`'s default steps, and appends them to `out/data/layouts.jsonl`, one circuit per line: its
parts, nets and board file, and the best layout and its cost. A layout here is only where each
part goes, `[x, y, rot]` in the order of `parts` (the `at` and `rot` of a layout file, above): the
rest of a layout file can be worked out again from the circuit. A run takes about 75s on an 18-core Mac and
carries on from the circuits already there. It takes no options, so circuit number n is always the
same circuit, placed the same way, as long as the code is the same: after a change to the placer,
the router, the score's weights, `footprints.json` or `src/random_circuit.rs`, delete `out/data/`
and make it again, or the file mixes layouts (and costs) from before and after; then `bench` and
`learn` again, since the placing network's numbers below came from the old layouts. It prints how many circuits and training examples there are (one per part) and what
they're like, and writes the batch's first circuit as `out/data/sample.net` (with its board file) so
you can `place` it and look.

The benchmark circuits are never in it: training only sees random circuits, so every benchmark
circuit is one the network has never seen.

## The placing network

`learn` trains a network that, with some parts already on the board, says where the next one goes.
Parts are placed one at a time, in a fixed order (parts with placement rules first, then the ones
with the most connected pins, then the bigger ones). For each place the next part may take (allowed
by the board file, on the board, off the parts already there and the keep-outs), the board is drawn
as it would be with the part there: a stack of grids, one per kind of thing (holes taken, the new
part's body and pins, placed pins on its nets, a "ratsnest" line from each new pin to the nearest
placed pin of its net, supply and ground pins, off the board and keep-outs). A small convolutional
network gives each picture a score: three layers of 3 x 3 filters, each sliding the same small grid of
weights over the whole board to find local patterns, then what they found added up and its strongest
value, into one number. The highest score wins.

```bash
cargo run --release -- learn
```

It learns from `out/data/layouts.jsonl` (`data`, above): for each part, with the parts before it where
the placer put them, the placer's choice should score highest. Each step compares the placer's choice
with some other places picked at random, through softmax (scores into probabilities that add up to 1)
and cross-entropy (the loss: -log of the probability given to the placer's choice). Every tenth circuit
is kept aside and never learned from. It then tests on those and on the benchmark circuits (the best
layouts `bench` saved in `out/bench.json` are their right answers): how often the placer's choice is
its first choice (top-1) or among its first five (top-5), next to simple rules: the shortest total
ratsnest, next to the placed part it shares the most nets with, and a random guess. The network is
saved to `out/placer.bpk`. Like `bench` and `data`, it takes no options.

Its settings are constants in `src/placing.rs` (`FILTERS`, `NEGATIVES`, `EPOCHS`, `LEARNING_RATE`),
chosen on the circuits kept aside; the benchmark row is only reported, never tuned on, so it stays
a test on circuits nothing was fitted to. The numbers (2026-10-17, 1,000 circuits from `data`, 5 epochs, about 14 minutes on an
18-core Mac's CPU):

| unseen circuits | network top-1 / top-5 | shortest ratsnest | next to the part sharing most nets | random guess |
|---|---|---|---|---|
| random circuits kept aside (1,326 decisions) | 12.4% / 30.2% | 1.6% / 10.7% | 0.2% / 1.3% | 0.4% / 1.9% |
| benchmark circuits (148 decisions) | 9.5% / 28.4% | 2.6% / 10.0% | 0.5% / 2.2% | 0.5% / 2.5% |

Learning from random circuits carries over to the real ones: nearly as good on the benchmark.

**Placing with it.** `place --start net` starts the placer
from the network's layout instead of a random one (or your DIYLC sketch: the network wins over it): each part, in the placing order, goes where the
network scores best, with the parts before it where it put them (under a second). A part with no
room left starts at random. The placer still has the last word, so the result is always polished,
but it starts cooler (`place::NETWORK_START_TEMPERATURE`, 5 against the usual 20), since a hot
start shakes a good layout up; 5 was the best of 20, 5, 2 and 0.5 on the benchmark circuits, so for
this one setting the benchmark isn't quite unseen. `bench --start net` measures it against the plain placer (below). On 2026-10-17, with the network
above: the network's layout alone costs 2.5 to 6 times less than a random start (not on the cat
feeder), and after the same
20,000 steps the mean cost is lower on 7 of the 12 benchmark circuits, 4.8% lower on average without
the cat feeder. Where it helps it gets there sooner: within 0.3k to 12k steps on the 555, the LM386
amp and the transistor flasher, where the plain placer takes 18k to 20k or never does, and a few
thousand steps sooner on the others. It does worse on the cat feeder, the ATtiny85 board, the LM317
supply, and (by 1 to 2%) the LED chaser and the PWM controller. The cat feeder has
modules (an ESP32 on headers), which the random training circuits don't have: the network places the
big modules badly, and one ends up with no room.

## Checking a change

Run both circuits and look at the pictures (`out/rules.svg`, `out/rules-solder.svg`) and the time
printed at the end. After a change to routing, the DIYLC files, the board file, the layout file or
any of the drawings, also check that the `.diy` the program writes still has exactly the connections
it was given (and that the layout file keeps everything the drawings need):

```bash
cargo run --release -- check circuits/cat_feeder.diy --seed 2   # one file
cargo run --release -- check my_sketches/ --steps 2000          # every .diy in a folder
```

`check` places each file as `place` does from the sketch (every case, keeping the best, with a line
for each case when there are several; it ignores `--start`), writes the `.diy` (to `out/check.diy`, overwritten for each
file), reads it back and compares the connections. It also writes the layout file (`out/check.layout.json`),
reads it back, and checks that what was read writes the same layout file again and the same `.diy`,
pictures and KiCad board as the layout in memory, so the layout file keeps everything the drawings
need. For each file it says one of:

- **same**: passes (only counted).
- **WRONG**: the written file has different connections; it lists the nets that differ. Or the
  layout file read back came out different; it says which of the files did.
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

### The benchmark

After a change meant to make layouts better (the placer, the router, a network), run the benchmark
and compare the costs with the last run (`out/bench.json`) or with the baseline, the plain placer's numbers recorded in
[docs/plans/placing-network.md](docs/plans/placing-network.md):

```bash
cargo run --release -- bench
```

It places each circuit listed in `circuits/bench.txt` (the 555, the cat feeder and the hobby
circuits in `circuits/bench/`) from a random start, with three seeds and `place`'s default steps,
the way `place` does, and prints each seed's best cost, the best and mean of the three, and the time
per run; `out/bench.json` keeps them, with each circuit's best layout (the right answers `learn`
tests the placing network on), as `[x, y, rot]` per part like `out/data/layouts.jsonl`, not as a
layout file. It takes no options but one: `bench --start net` starts every run
from the placing network's layout, saves to `out/bench-net.json` (`out/bench.json` stays the plain
placer's), and prints a comparison with the plain placer's last run: the starting layouts' cost, the
mean and best cost after the same steps, and the step at which each run got down to the plain
placer's mean cost. Otherwise every run measures the same thing. The runs go several at a time,
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
  not enough it gets one solder-side wire straight to the pin.
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
  power and ground, the seed, how hard the placer tries, which judge and where it starts.
- **Plain code over clever code**, with comments that explain why in plain words.

## Where the neural network fits

Right now the network learns to copy the rules. On boards it has never seen it is off by
about a point, and the placer does just as well using it as the judge. On its own that
isn't useful, since we already have the rules. It proves the plumbing works.

It becomes useful when the "right answers" come from something the rules can't express:

1. **Your taste.** Rate layouts yourself; the network learns what you like, including things
   that are hard to write down as a rule.
2. **Richer input.** Feed it the board itself (which hole holds what) instead of nine numbers,
   so it can spot patterns no rule measures. The placing network (item 3) already sees the board this
   way, with Burn.
3. **Placing directly.** Train a network that proposes where the parts go, so the placer starts
   from a good layout instead of a random one. Under way: [docs/plans/placing-network.md](docs/plans/placing-network.md)
   (its status line says how far it got); "The placing network" above is what's built.

## Next steps

In order:

1. **Split "allowed" from "good".** Collisions, hanging off the board and keep-outs become exact
   checks outside the score, so the score (and the network) only compares layouts that are allowed.
2. **A `rate` command**: show two layouts, you pick the better one, and the network learns your taste
   from those picks ("Your taste" above). Then more circuits to learn from.
3. **Faster routing**, if waits grow past an espresso (see Known limits).

Meanwhile, a network that places parts is being built, stage by stage:
[docs/plans/placing-network.md](docs/plans/placing-network.md).

Smaller ideas, in the maintainer's words, are in [TODO.md](TODO.md).

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, check a change, and how the project is shaped.

Harmonia is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
