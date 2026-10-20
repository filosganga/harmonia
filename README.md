# Harmonia: perfboard layout + a tiny neural network

Before it meant harmony, the Greek ἁρμονία was a carpenter's word for a joint: things fitted together.
Harmonia turns parts, fits them on a perfboard and wires them up.

Rust, with four small crates (`rand`, `serde`, `serde_json`, and `roxmltree` to read DIYLC files).
Every placer step routes the whole board, so commands take seconds: on the 555, `place` takes about
4s, `train` 11s and `evolve` 12s; placing the bigger cat-feeder board about 37s (its board file makes two cases). A board file
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
cargo run --release -- footprints           # the parts it knows: size in holes, where each pad is
cargo run --release -- --help               # every command and option
```

To use it from any folder, install it: `cargo install --path .` puts a `harmonia` command in
`~/.cargo/bin`, and `harmonia place my.net` is then the same as `cargo run --release -- place my.net`
(install again after changing the code). The footprint library is built in, so it works anywhere;
your own footprints go in `~/.config/harmonia/footprints.json` (below).

Options: `--steps 50000` (placer effort), `--seed 7` (a different random start), `--board 24x16`
(perfboard size in holes, default 18x12), `--out pictures` (the folder `place`, `check`, `train` and `evolve` write in, default `out`; `draw` writes next to the layout file, `bench` always to `out/`), `--board-file try.json` (another
board file instead of the one next to the circuit, to compare variants), `--power +12V` / `--ground GND` (which nets are supply and
ground, guessed from common names like `+5V`, `VCC` and `GND` if left out), or a path to another netlist.
Board size, power and ground can also live in the board file (below); a flag wins over it.

**Without Rust:** each [release](https://github.com/filosganga/harmonia/releases) has the program
ready-built for Linux (x86-64), macOS (Apple silicon) and Windows (x86-64). The footprint library is
built in, so it runs from any folder; its folder also holds the example circuits: `./harmonia place`,
`./harmonia place circuits/cat_feeder.diy`, and so on, the same commands without `cargo run --release --`.
(The make-it-again line says `harmonia`: from that folder, type `./harmonia`.)

Each `place` run prints the board as text (when the board file's `one-of`s make cases, first a line
for each case and which was best), then the seed and the command that makes the same layout again,
with the program's version and commit (`cargo run --release -- place circuits/cat_feeder.diy --seed 1
--steps 20000`). Every output says it too: the pictures under their title, KiCad in the title block,
DIYLC in the project's description, the layout file in `made`. The same seed, steps, circuit, board
file and code always make the same placement and copper (with `--judge net`, the same `model.json`
too); the files differ at most in that line. A changed board file (a weight tuned) or code makes
another layout, so keep a copy of the board file with a layout you like. `train` and `evolve`
print their seed and command the same way. `place` writes, in `out/` (or the `--out` folder; named `rules` or `net` after the judge):

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

`rules.layout.json` holds the finished board, a *build*: what you asked for (the **design**) and
what the program made of it (the **layout**), so the drawings (and any tool of your own) don't have
to work it out again, and a saved board can be scored again later. It's JSON, with `"format":
"harmonia-layout"` and a `"version"` number (a file of another version is refused, with a message):

- **made**: how it was made: the circuit file, the board file, the judge, where the placer started
  (`random` or `sketch`), the seed, the steps, and which case of the board file's rules won
  (counted from 1, out of how many); the **command** that makes it again and the program's
  **version** and commit; and the **footprints** that didn't come from the built-in library (which
  file, and whether it replaced a different one), since placing again needs that file too.
- **title**: the one the pictures show.
- **design**: what you asked for.
  - **board** (its width and height in holes), **power** and **ground** (the two net names).
  - **rules**: the board file's rules as written (what you left out is left out), and
    **choices**: the option this case took in each one-of.
  - **parts**: each part's reference, value and footprint; its `size` in holes before turning,
    whether it's `low`, the `"diylc"` entry from `footprints.json` if it has one, and its pins: name,
    `at` (its hole inside the part, before turning), and `off_grid` for a pin between holes. A part
    that doesn't take its whole outline (a module, or a DIYLC part drawn at an angle) also has its
    `body`, the holes it takes; a part from a DIYLC sketch has its `diylc_drawing`, your drawing of
    it, which is how the `.diy` keeps it.
  - **nets**: each net's name and pins (two or more).
- **layout**: what was made of it.
  - **parts**: where each part sits, `at` and `rot` meaning what they mean in the board file's
    `fixed` rule, and the `hole` each of its pins lands in.
  - **runs**: what the router joins with one piece of copper (a net, or a branch of a star net,
    with its `star` point), in the order it routes them first (shortest first), with that copper:
    `traces` as straight pieces, `[from hole, to hole]`, then `jumpers` and `solder_wires` the same
    way, `unrouted` (only when there are any): connections the router couldn't make, and `joins`
    (only for a soft star's branch that took any): where it joined another branch's copper.
- **score**: the score, the cost and the eleven measurements by name (below), and **problems**:
  the `!!` lines. Both are written for people to read; reading the file back works them out again.

Not in it: the net colours (worked out from the nets), and the start (only `rules.html` shows it).

`draw` reads a layout file and draws it again next to it: `draw out/rules.layout.json` writes
`out/rules.svg`, `out/rules-solder.svg`, `out/rules.kicad_pcb` and `out/rules.diy`, the same files
`place` wrote, without placing or routing anything: the copper is drawn as it was saved, so a layout
saved before a change to the router draws as it did. The score and the problems are worked out
again from the file's design and layout, with today's measurements and weights. Copy a
layout file somewhere first to keep it: the next `place` overwrites `out/rules.layout.json`.

## Using your own circuit

1. Draw the schematic in KiCad and give every part a through-hole footprint.
2. In the schematic editor: File → Export → Netlist, KiCad format. That writes a `.net` file.
3. `cargo run --release -- place path/to/your.net`

A netlist only says what connects to what. It doesn't say where a part's legs are, so each footprint
also needs an entry in `footprints.json`. There are three, each adding parts and replacing ones of
the same name: the repo's, built into the program; your own, for every circuit, in
`~/.config/harmonia/footprints.json` (`$XDG_CONFIG_HOME/harmonia`, `%APPDATA%\harmonia` on Windows);
and one next to a netlist, for the circuits in that folder. The more local file wins, never
silently: when an entry a circuit uses replaces a different one, `place` and `nets` print a note,
and the layout file lists it (an exact copy changes nothing). `footprints` lists what a circuit can
use, which files it read, and where each entry came from. `bench` reads only the built-in library
and the circuit's own folder, so it places the same way on every machine. Each entry is: its size in holes and which hole each pad sits in (top-left
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
the board file or `--board`, so the sketch can be drawn on a bigger board than the real one. If you
wired the sketch, two more lines measure your own wiring as drawn ("wired as you drew it") and the
router's on the same placement ("wired by the router"), so you can see which the score prefers. Both
measure the sketch exactly as you drew it: before the board file's placement rules move or turn
anything, and whether it's allowed or not (two parts sharing a hole are scored anyway; a pin off the
real board counts as unrouted). Your wiring's star-rule breaks are counted like the router's joins (with
a weight) or on a line of their own (without one).

- **Your sketch doesn't have to be allowed.** Parts drawn on top of each other, off the real board,
  on a keep-out (without a weight) or under a module are moved, each to the nearest place where it fits, before the
  placer starts (biggest parts first, so the small one moves). The run says what was wrong ("Your
  sketch: not allowed, so not scored: ..."), and the start in `rules.html` boxes those parts in red
  and isn't scored either.
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
  TO-92/126/220 transistors, pin headers, tactile switches; copper traces, jumpers, hook-up,
  orthogonal and twisted wires, solder pads; labels, text, shapes and chassis drawings. Anything else stops with a message naming
  it, because its connections would otherwise be missed.

Checked against DIYLC itself: on 216 DIYLC files, the program finds exactly the connections DIYLC's own
netlist finds. And on all 194 of them with connections, the file it writes (re-placed and re-routed)
reads back with exactly the original's connections.

Some parts have legs between holes (a tactile switch's are 0.175" and 0.255" apart). Copper in a hole
next to such a leg could touch it, so the router keeps other nets out of the holes around it.

## The board file: your rules for this board

Next to the netlist, a JSON file with the same name (`circuits/555_blinker.json` for
`circuits/555_blinker.net`) describes the board you're building on (`--board-file` picks another
one, say a copy with a weight changed). Everything in it is optional:

```json
{
  "board": "18x12",
  "power": "+5V", "ground": "GND",
  "rules": [
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
    { "kind": "edge",    "name": "D1 on the right, if it can", "part": "D1", "edge": "right", "weight": 0.01 },
    { "kind": "near",    "name": "C2 by CTRL", "pins": ["C2.1", "U1.5"], "weight": 0.05 },
    { "kind": "close",   "pins": ["D1.2", "U1.3"] },
    { "kind": "star",    "at": "C1.2", "runs": [["U1.1", "C2.2"], ["D1.1"]] }
  ]
}
```

- **board / power / ground**: the same as the flags. A flag still wins, for a quick experiment.
- **rules**: your rules, in any order, and all of them hold (like an `all-of`, below). Each says its `kind`, and may have a `name`: messages
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
- **keepout**: holes no part may cover, from one corner to the other, and no copper either. Shown as `x` on the text board, a
  grey box in the pictures and in DIYLC, and a "rule area" in KiCad, which keeps enforcing it while you
  edit (a soft one's area allows everything, and only marks the place).
- **near**: two pins of the same net should be joined by as little copper as possible, like a resistor
  guarding a GPIO or a decoupling capacitor by its chip. It's measured along the copper the router
  laid (traces, jumpers and wires), not in a straight line. On a star net, two pins in different
  runs are only joined through the star point (or a join, if the star has a weight), so that's the way it's measured. It costs per hole
  of copper: without a `weight` (below), as if it had a weight of 1.
- **close**: two pins of any nets should be as close together as possible, straight across the
  board. For a connection the board doesn't carry, like a wire from pads under a module to a header.
  It costs per hole like near, is counted with it, and takes a `weight` too.
- **weight**, on any rule: how much it matters, from 0 to 1. Left out, the rule is hard: the placer
  never breaks it (below). With one, it may be broken, at a cost of the weight times a thousand for
  each hole it's broken by. At 1 that's as much as two solder-side wires, so the placer trades any
  corners, jumpers or crowding to keep it, and only three solder-side wires or an unrouted pin cost
  more: nearly hard, but still a cost. Smaller weights trade against the wiring: 0.005 makes a hole
  cost as much as a corner, 0.015 as much as a jumper, and 0 costs nothing. How a rule is broken:
  - fixed, edge, orientation, within: by how many holes the part is away from where the rule wants
    it (for within, how far its outline sticks out of the rectangle), and each quarter turn off counts
    as one. With a weight the part may go anywhere, paying that.
  - near and close: every hole between the two pins counts. Copper between two pins is only known
    once the board is routed, so the placer can't keep them out of a layout the way it does the
    others: left without a weight, they cost as at weight 1. Before weights they cost 4 a hole, a
    weight of 0.004.
  - keepout: each of its holes a part or copper covers.
  - star: each time a run joins another run of the net away from the star point (see "How it routes").
  - all-of, one-of: a weight here is the weight of every rule inside that has none of its own. A
    one-of about where one part goes counts its least broken option.

  The `rules` measurement adds it all up.
- **star**: how the net of the `at` pin is wired. Every run starts at `at` and goes its own way, never
  sharing copper with another run (unless the rule has a weight, see "How it routes"), so one part's current doesn't shift the voltage another part sees.
  Each run in `runs` joins its pins in a chain; any other pin of the net gets a run of its own, straight
  from `at`. A net can have one star rule.

Without a weight, fixed, edge, orientation and within are hard limits. In each case, each part they're about gets the
list of places (position and turn) that meet all its hard rules, while staying on the board and off the
hard keep-outs. After every move the placer puts the part back on the nearest of them, keeping its turn if
it can; now and then it also jumps the part to any of them, so it can get across to another option
of a one-of, like the other edge. A case where some part has no such place is left out (the run says why); if every case is,
the program stops. A keep-out without a weight is a hard limit for every part: no part ever covers one (see what's
allowed, in "How a board is judged" below). Coordinates are holes, counted like the numbers around the
text board. Pins are named like the `nets` command prints them.

## The pieces

| File | What it does |
|---|---|
| `circuits/555_blinker.net` | The example circuit as a KiCad netlist: the parts, their footprints, and which pins connect (nets). |
| `circuits/555_blinker.json` | Its board file: board size, power and ground, and the rules for this board. |
| `circuits/cat_feeder.diy` | A copy of the cat-feeder's `pcb.diy` (2026-10-17; the RTC's board rectangle since drawn one hole wider on each long side, to its real 8 holes), with `cat_feeder.json`: 5 V and ground starred from where the supply comes in (Q1's cathode, the cable's ground), the driver with its own runs and C1 near it, R1 near GPIO2, `Pads` close to GP5 or GP22 (the ESP32's back pads), the ESP32's USB side on one of the board's edges, and the two cable headers (`Inlet`, `Ext`) on an edge or one hole in. |
| `circuits/cat_feeder_fixed/` | The cat feeder as a KiCad netlist (`orig.net`, its module pins named, e.g. `U1.GP2`), its modules in its own `footprints.json`, with every part `fixed` where it's soldered on the real board (`orig.json`; `orig-nostar.json` without the star rules). Routed by hand with 4 jumpers and no solder-side wire, so CI checks the router joins every pin without one too. Also `feeder.net`, the same board with two voltage dividers and a capacitor more, and board files for it, given with `--board-file` (`free.json` places everything; `keep*.json` and `add*.json` keep the modules, or more, where they are), and `hand.json` / `hand-nostar.json`: `orig.json`'s star and near rules with the sketch's pin names (`place circuits/cat_feeder.diy --board-file circuits/cat_feeder_fixed/hand.json` measures the hand wiring). |
| `TODO.md` | The maintainer's own list of ideas for later. |
| `src/constraints.rs` | Reads the board file and keeps its rules: where parts may go (fixed, edge, orientation, within, all-of, one-of), keep-outs, near, close, star; and makes one design per case of them. |
| `circuits/bench.txt` | The benchmark set: the circuits `bench` places. Never used to train a network. |
| `circuits/bench/` | Ten classic hobby circuits for the benchmark (a 7805 supply, a transistor flasher, an LM386 amp, a 555 + 4017 LED chaser, ...), written as KiCad netlists after datasheet applications and well-known designs, each with a board file (its size, power, ground, and the connectors on edges). |
| `footprints.json` | Footprint name → size in holes and where each pad is. |
| `build.rs` | Runs before compiling: tells the program the git commit it's built from, for the "make it again" line on every output. |
| `dev/kicad_footprint.py` | A dev tool (Python, not needed to run the program): drafts a `footprints.json` entry from a KiCad footprint file. |
| `src/sexpr.rs` | Reads KiCad's bracket format, `(comp (ref "R1") ...)`, into a tree. |
| `src/netlist.rs` | Turns a KiCad netlist plus `footprints.json` (built in, yours in `~/.config/harmonia`, and one next to the netlist) into the circuit the placer works on. |
| `src/board.rs` | Places parts, works out where every pin lands, measures the board and gives it a score. |
| `src/route.rs` | The router: joins every net with copper traces, and hops over with a jumper (or, last, a solder-side wire) where a trace can't get through or going round costs more; for a finished layout it makes 200 complete routings in different orders and keeps the one the score likes best; a pin walled in so that not even a wire fits is left unrouted. It also turns a sketch's own copper into a routing, to measure your wiring. |
| `src/route/search.rs` | The router's search: the board as it sees it (who owns each hole, where wires lie) and the A* search for the cheapest way to a run's next pin, in traces and wires, with what each step costs. |
| `src/route/sketch.rs` | Your own wiring, from a DIYLC sketch, as a routing the score can measure. |
| `src/build.rs` | A finished board, a `Build`: what you asked for (`Design`, in `src/board.rs`: the parts, nets, board and your rules), what was made of it (`Layout`: where each part goes and the copper), and how; scored once, and drawn by the three writers below. |
| `src/build/file.rs` | The layout file: writing a build, and reading it back (for `draw`, and `check`'s round trip). |
| `src/svg.rs` | Draws the layout as an SVG picture, from the parts' side or the solder side. |
| `src/kicad_pcb.rs` | Writes the layout as a KiCad board. |
| `src/diylc.rs` | What the program knows about DIYLC, shared by the reader and the writer: the part classes it supports (`CLASSES`: each one's pins, whether it's low, its body, how to draw a new one), DIYLC's units and turns. |
| `src/diylc/read.rs` | Reads a DIYLC sketch: its parts, its modules (groups), which pins connect, and the copper you drew, to measure your own wiring. |
| `src/diylc/write.rs` | Writes the layout as a DIYLC project (your own parts, moved, if it came from one). |
| `src/place.rs` | The auto-placer: starts from an allowed layout and keeps nudging/rotating/swapping/jumping parts, only ever to allowed layouts, keeping changes the judge likes. |
| `src/nn.rs` | The neural network, written out by hand (8 hidden neurons, 105 weights). |
| `src/bench.rs` | The benchmark: places every circuit of `circuits/bench.txt` with three seeds, several runs at once, and records the best costs. |
| `src/main.rs` | The commands: `place`, `train`, `evolve`, `nets`, `check`, `draw`, `bench` and `footprints` (no arguments or `--help` prints them all); `place` and `check` place every case of the board file's rules and keep the best. `train` and `evolve` make ~1,200 example boards (going through the board file's cases in turn, up to 12), have the rules score them, and train the network to match. |

## How a board is judged

First, whether it's **allowed**: whether it can be built at all. A layout isn't allowed if two parts
take the same hole, a part hangs off the board (a module's board too), a part sits on a keep-out (one without a weight), or
a part that isn't low sits under a module's board (`board::clashes`). That's a yes-or-no check, not
a cost: the placer never makes a layout that isn't allowed, so the score below only ever compares
layouts that can be built.

Then how good it is. `src/board.rs` measures eleven things (weights in `WEIGHTS`):

- **wire**: total length of copper trace, in holes, as routed (below).
- **turns**: how hard the traces' bends are to make. Traces are laid with copper wire, and each
  bend is one more to make by hand, while a longer straight run is hardly more work: so a right
  angle costs as much as five holes of trace. A right angle, the easiest,
  counts 1; a gentle 45° turn, onto or off a diagonal, counts 2, since it's harder to land on the
  right hole; a sharp 135° turn counts 3. A pin or a wire's leg is free, since a trace starts
  there, and so is a junction where three or more steps meet, which is soldered like a pin.
  Jumpers never bend: each is one straight wire.
- **diagonal**: steps of copper trace laid at 45°, each costing three corners (fifteen holes of
  trace) on top of its length. Straight traces are easier to lay along the holes and line up side by side; a board full of
  diagonals (V shapes and staircases) is hard to build and couldn't be a stripboard. So the router
  takes a diagonal only where nothing straight gets through. Jumpers and solder-side wires are wires,
  free to go at 45°: they don't count.
- **crosswise**: straight steps of copper trace in the board's minority direction, half a hole each:
  whichever of across and down this layout's traces take less (the other is the main direction). On
  a stripboard the strips run one way and copper the other way needs a wire; on perfboard too,
  parallel traces in one direction are the easiest to lay. So the placer prefers layouts where most
  copper runs one way. Some always runs the other way, since not every pin of a net can be on one line.
- **jumpers**: how many jumpers (on the parts' side) the router needed where a trace couldn't get
  through or going round cost more. Each costs three corners: cut, bend both legs, solder both ends.
- **jumper_len**: total length of jumpers and solder-side wires. A hole of it costs half a hole of
  trace. A jumper 10 holes long is no more work than one 3 holes long, but it lies over holes that
  later jumpers can't cross; with length almost free, jumpers got three times longer and the last
  runs needed solder-side wires.
- **bottom**: how many wires on the solder side, the last resort. Each costs a hundred corners
  (more than thirty jumpers), so the placer finds room for jumpers instead.
- **unrouted**: connections the router couldn't make at all: a pin walled in by other copper so that
  not even a wire fits (a wire's leg needs a hole of its own, never a pin's).
  Each costs five thousand holes of trace (as much as ten solder-side wires), far more than any wiring, so the placer keeps away from such
  layouts; one that's left is drawn as a thin dotted line, with a `!!` line naming the pin. It stays
  a cost, not part of "allowed": it's only known once the board is routed, it's the greedy router's
  limit rather than the board's (it tries 200 orders, but not every way, and a wire added by hand fixes it),
  and at the placer's low temperatures that already works like a "never".
- **power**: for every supply pin, the distance to the nearest ground pin (keeps supply and ground close).
- **crowding**: pairs of neighbouring holes (across or down) taken by two different parts, where the
  soldering iron has no room, four holes of trace each. A pair of pins a solder blob would wrongly join (on different nets, or
  one on a net and one unused) counts twice. So parts get room around them and spread over the board. Beyond that, being far
  apart earns nothing, and wire length keeps them from drifting. If you want a smaller layout, give
  it a smaller board.
- **rules**: how much the board file's rules with a weight are broken, each times its weight
  ("The board file" says how each kind is broken), a thousand per hole of that. That includes, for each `near` rule, the length of copper between its two pins, as
  routed, and for each `close` rule, the distance between its two pins, straight across the board,
  at weight 1 if left out: a rule at 1 outweighs everything but an unrouted pin. (Copper between two
  pins side by side is 1 hole long.) The report shows it with up to three decimals, since 0.005 of
  it still costs 5; the other measurements are rounded to whole numbers.

The weighted sum is the *cost*; the score is `100 * exp(-cost / 150)`, so 100 is perfect.
The placer itself compares costs, not scores: for a big circuit the score is low and flat (the cat
feeder's wiring alone costs about 700, which scores under 1), so comparing scores would make it accept almost any bad move. Scores are
for reading; for a big circuit they're low even when the layout is good.

### How it routes

Every placer step routes the whole board (`src/route.rs`), so the placer sees which layouts need jumpers.

Each net is one run, or one run per branch for a star net. Two runs never share copper or wire, even
two branches of the same star net, unless its star rule has a weight: then a run may start from
another run's copper instead of its own, a join, which costs the weight times a thousand. The router
takes a join where it's cheaper than the run's own way, counting a jumper or a solder-side wire on
that way at what the score charges (the `jumpers` and `bottom` weights).

A soft keep-out (with a weight) is open to traces and wires, but each hole of it a trace steps into
or a wire lies over costs the weight times a thousand, so the router goes round it unless that costs more.

- A **copper trace** is on the solder side. It goes hole to neighbouring hole: across, down, or at 45°.
  It touches every hole it passes, so it can't pass a hole with another run's pin or trace, or a
  hard keep-out, and two diagonals can't cross between four holes. It can run under parts, since it's on
  the other side. A right angle costs as much as five holes of trace, a 45° turn twice that and a
  135° turn three times (the `turns` weight), so the router takes a straight way over a zig-zag
  even when the zig-zag is a few holes shorter. A diagonal step pays three corners on top of its
  length (the `diagonal` weight), so the router goes across and down instead unless nothing
  straight gets through. Dearer corners alone (when a jumper still cost 5 plus 2 a hole) made the
  router detour, box later runs in and need more jumpers; priced together with jumpers, as three
  corners each and cheap per hole, they give far fewer corners and about as many jumpers (the
  numbers are in the comment on `WEIGHTS`). The `crosswise` measurement isn't
  priced by the router, since the main direction is only known once the board is routed; a trace
  from one pin to another takes the same steps across and down whichever way it turns.
- A **jumper** is a wire on the parts' side, used where traces can't get through, or where going
  round costs more than the jumper (three corners and half a hole per hole, as the score charges): traces
  run up to a free hole, the jumper hops over, and traces carry on from where it lands. It goes across, down or at
  45°, 2 to 10 holes long. It can't pass over a part, a pin or a hard keep-out (it can go under a module's
  board, like low parts), or over or across another run's jumper. A jumper is like a part: its legs
  need holes of their own, never a pin's. It may lie over a hole where a solder-side wire ends:
  hopping over things is what it's for.
- A **wire on the solder side** is the last resort: it costs a hundred corners, as in the score, so
  it's only used when no jumper fits, or the way to one would cost more than a hundred corners. Same shape and legs, but nothing sits on that side except copper,
  so it can pass under parts and over traces and pins (it's insulated). It still keeps off hard keep-outs
  and other runs' solder-side wires.

Runs are routed one at a time (shortest first, in the first routing; see below). Each run grows like a tree: from what's joined so far,
the cheapest way to the nearest pin not joined yet, trying holes in the direction of that pin first
("A*" search). Traces, jumpers and solder-side wires are all steps of that one search, each at what
the score charges, so a way may have any number of wires, and a long trace round the board loses to
a jumper that costs less. A pin that can't be reached, because it's walled in so that not even a
wire fits, is left out of its run: `unrouted`, above. No wire ever ends on a pin.

Which way a run goes depends on the order the runs are routed in, and on which of several equally
cheap ways the search finds first, and that can wall in a pin routed later: on the cat feeder with
every part fixed (`circuits/cat_feeder_fixed`), shortest first, the motor traces coming down to the
cable header took the only free holes beside its corner pin. So for a finished layout the router
doesn't route once: it makes 200 complete routings, the first shortest run first and the others in
random orders (and with the search's ties broken in random orders), and the score picks the best,
every measurement and rule counted. The router has no rules of its own about what to try again or
what to keep room for: it proposes, the score judges. A routing whose copper so far already costs
more than the best one's stops there, so most stop early (all 200 take about 0.1 to 0.15s on the
cat feeder with its parts fixed). The random orders come from a fixed seed, so the same placement always gets the same copper.

The placer can't afford 200 routings for every move, so it judges each move by the first one,
shortest first, alone (`route_once`); judging moves by the best of several also made the 4017 LED
chaser end with solder-side wires on three seeds of six, against none. `train` and `evolve` measure
their example boards the same way, so the network learns what the placer will show it (trained on
boards routed the thorough way, it rarely saw a pin left unrouted and placed the 555 with
solder-side wires). Every layout reported or saved (the placer's result, your sketch, `check`, the
layout file) is routed the thorough way, which is never worse than the first routing.

In the pictures, jumpers are solid blue and solder-side wires dashed red. In the `.diy`, both are
DIYLC jumpers (DIYLC has no wire on the solder side), the solder-side ones red and named "(solder side)".

## Two ways to train the network

Both commands train the same 9-neuron network on the same example boards, then test it on
boards it never saw and save it as `model.json` in the output folder (`out/`, or `--out`; so the last one you ran is the judge, for a `place --judge net` with the same `--out`).

- **`train` (gradient descent):** for each board, compare the network's guess with the rules'
  score and nudge every weight in the direction that makes it less wrong. One example points
  all 105 weights the right way at once.
- **`evolve` (evolution):** make 19 copies of the network with every weight changed a little at
  random, keep whichever of the 20 is least wrong, repeat 1,000 times. It never works out a
  direction; it only needs to know which copy did better.

Typical result on the 555: `train` is off by about 0.7 points on unseen boards (0.7 to 0.8) in 11s, `evolve` by
about 1.2 points (1.0 to 1.6, depending on the seed) in 12s, after trying 19,000 networks. When you have a right answer for every example,
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

After a change to routing, also route the cat feeder with every part where it's soldered:
`cargo run --release -- place circuits/cat_feeder_fixed/orig.net --steps 1` must say `bottom=0,
unrouted=0` (it was routed by hand that way; CI checks it).

`check` places each file as `place` does, from the sketch (every case, keeping the best, with a line
for each case when there are several), writes the `.diy` (to `out/check.diy`, or in the `--out` folder, overwritten for each
file), reads it back and compares the connections. It also writes the layout file (`out/check.layout.json`),
reads it back, and checks that what was read writes the same layout file again and the same `.diy`,
pictures and KiCad board as the layout in memory, so the layout file keeps everything the drawings
need. For each file it says one of:

- **same**: passes (only counted).
- **WRONG**: the written file has different connections; it lists the nets that differ. Or the
  layout file read back came out different; it says which of the files did.
- **STOPPED**: the program stopped after reading the file (in the placer, the router, the writer, or
  reading back what it wrote), with the message and where in the code.
- **unfinished**: the connections differ, but only because some pins couldn't be reached by the
  router. Try more `--steps`.
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
one per processor core (about 50s on an 18-core Mac); each has its own random numbers from its seed,
so the costs are the same as running them one by one. Costs only compare within a circuit, and only
under the same weights: a change to a measurement or a weight changes what a cost means, so run the
benchmark before and after it and compare the layouts, not the old numbers.

## Known limits

- **Speed.** Every placer step routes the whole board, and each case is placed in full, one after
  the other (times at the top). One-ofs that make cases, side by side, multiply them. Re-routing
  only what a move changes, or placing the cases at the same time, would make it quicker; neither is
  built.
- **Layouts depend on the seed.** On the cat feeder, different seeds land on quite different layouts
  and costs. Try more `--steps` and two or three `--seed`s, and compare the costs. Each output names
  its seed, so a layout you like can be made again (with the same code and board file).
- **The router is greedy.** It routes one run at a time and never undoes one. For a finished
  layout it tries 200 orders and keeps the best, but the placer judges its moves by one order,
  shortest first, so it may pass over a placement that another order would route well. A pin walled
  in in every order tried is left unrouted (very expensive, so the placer finds another layout).
- **A part can't pass through another.** The placer only ever holds allowed layouts, so a nudge
  that would put a part on top of its neighbour is thrown away. To get past, it jumps: three moves
  in ten put a part anywhere it fits on the board. On a crowded board, a place that's only reachable
  by moving two parts at once is still hard to find.
- **KiCad has no traces.** `rules.kicad_pcb` has the parts and their nets, not the routed copper.
  The copper is in the layout file (`runs`) and in the `Build` that `src/kicad_pcb.rs` is given,
  so writing it into the KiCad board only needs that writer to draw it.
- **DIP pin names from DIYLC.** Reading a `.diy`, a DIP chip's right-hand pins are numbered top to
  bottom (5–8) where the chip's own numbering runs bottom to top, so the top-right pin is called 5,
  not 8. The connections are right; only the names are off. Check with `nets` before naming such a pin
  in a board file.
- **Angled parts from DIYLC.** A two-legged part drawn at an angle only takes its two leg holes, so the
  placer treats it like a module: low parts may go under it, and jumpers may pass over it.
- **Scores of big circuits are low.** The score's scale (`SCORE_SCALE`) was set on the 555, under
  cheaper corners; the cat-feeder board scores under 1 even when it's good, and a board file with a
  near or close rule at weight 1 always scores 0 (each costs a thousand a hole, and copper between
  two pins is at least a hole). Compare costs between layouts of the same circuit, not across circuits.

## Principles

- **The program is Rust only**, including the machine learning. Dev tools around it (checkers,
  converters, plots) may be in Python, but nothing the program needs to run.
- **Hard rules, preferences, taste.** Facts about the hardware and limits of a board (fixed parts,
  edges, keep-outs, collisions) are hard rules the placer enforces, never learned: the placement
  rules and what's allowed ("How a board is judged"). What one board wants goes in its board file:
  every rule there is hard unless a `weight` (0-1) softens it into a cost (`near` and `close`, which
  can only be measured once routed, cost as at weight 1), and `star` says how its nets are split into runs. The network is for
  taste among layouts that are already allowed.
- **Model settings are the author's job.** Hidden neurons, learning rates, generations and the score's
  weights (`WEIGHTS`) are constants in the code, not flags. Flags are for your problem: the circuit, the board,
  power and ground, the seed, how hard the placer tries.
- **Plain code over clever code**, with comments that explain why in plain words.

## Where the neural network fits

Right now the network learns to copy the rules. On boards it has never seen it is off by
under a point, and as the placer's judge it does a little worse than the rules on the 555 (seeds 1-3,
in the rules' cost: 127.5, 151 and 152.5, against the rules' own 133, 153 and 121). It learns from allowed boards
only, and the placer can't make any other kind, so it never has to learn that a collision is bad.
Its 0-100 score is still flat for big circuits (under 1 for the cat feeder), so as a judge there it
would likely do worse; training it on the cost instead would help. Even at its best it isn't useful
on its own, since we already have the rules. It proves the plumbing works.

It becomes useful when the "right answers" come from something the rules can't express:

1. **Your taste.** Rate layouts yourself; the network learns what you like, including things
   that are hard to write down as a rule.
2. **Richer input.** Feed it the board itself (which hole holds what) instead of eleven numbers,
   so it can spot patterns no rule measures. That's when a library like Burn starts paying off.
3. **Placing directly.** Train a network that proposes where the parts go, so the placer starts
   from a good layout instead of a random one. Tried and parked:
   [docs/plans/placing-network.md](docs/plans/placing-network.md) (the code is on the branch `better-placing`).

## Next steps

In order:

1. **Tune the rule weights** on real boards (the cat feeder first), then put the score's own
   weights on the same 0-1 scale.
2. **A `rate` command**: show two layouts, you pick the better one, and the network learns your taste
   from those picks ("Your taste" above). Then more circuits to learn from.
3. **Faster routing**, if waits grow past an espresso (see Known limits).

A network that places parts was built and parked (on the branch `better-placing`); see
[docs/plans/placing-network.md](docs/plans/placing-network.md).

Smaller ideas, in the maintainer's words, are in [TODO.md](TODO.md).

## Contributing and license

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, check a change, and how the project is shaped.

Harmonia is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
