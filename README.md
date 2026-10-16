# Perfboard layout + a tiny neural network

Rust, with four small crates (`rand`, `serde`, `serde_json`, and `roxmltree` to read DIYLC files).
Commands on the 555 finish in under two seconds; placing the bigger cat-feeder board takes about 2.5s,
because every placer step routes the whole board.

```bash
cargo run --release -- place                # auto-place the 555 blinker using the hand-written rules
cargo run --release -- train                # teach the tiny network to score boards like the rules do
cargo run --release -- evolve               # the same, by evolution instead (1 parent + 19 children)
cargo run --release -- place --judge net    # auto-place again, but let the network be the judge
cargo run --release -- place my_sketch.diy  # score a rough DIYLC layout, then fit it on the board
cargo run --release -- nets my_sketch.diy   # just list the parts and connections it understood
```

Options: `--steps 50000` (placer effort), `--seed 7` (a different random start), `--board 24x16`
(perfboard size in holes, default 18x12), `--power +12V` / `--ground GND` (which nets are supply and
ground, guessed from common names like `+5V`, `VCC` and `GND` if left out), or a path to another netlist.
Board size, power and ground can also live in the board file (below); a flag wins over it.

Each `place` run prints the board as text and writes, in `out/` (named `rules` or `net` after the judge):

- `rules.svg`: a picture of the result. Holes are coloured by net, with the copper traces and jumpers, the parts,
  header pin names and modules, and beside the board the score and every net. Hover a hole for its
  position, pin and net. Hole numbers are the text board's.
- `rules-solder.svg`: the same board turned over to solder it: mirrored (column 0 on the right), copper
  in front, the parts faint behind.
- `rules.html`: the start (random, or your sketch) and the result side by side, drawn the same way.
- `rules.kicad_pcb`: the layout for KiCad's PCB editor (KiCad 10 opens it). Every pad is on its net, so
  KiCad shows the connections still to wire. The footprints are simple stand-ins built from
  `footprints.json`; Tools → Update Footprints from Library swaps in KiCad's real ones without moving anything.
- `rules.diy`: the layout for DIY Layout Creator, on a perfboard, with the routed copper traces (solder
  side) and jumpers, coloured by net (power red, ground near-black).

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
  moved and turned, your groups still grouped. Your traces are replaced by one jumper per planned wire.
- Supported DIYLC parts: resistors, ceramic/film/electrolytic capacitors, LEDs, plastic diodes, DIP chips,
  TO-92/126/220 transistors, pin headers, tactile switches; copper traces, jumpers, hook-up and
  orthogonal wires, solder pads; labels, text and shapes. Anything else stops with a message naming
  it, because its connections would otherwise be missed.

Checked against DIYLC itself: on 216 DIYLC files, the program finds exactly the connections DIYLC's own
netlist finds, and a re-placed file still has exactly the original's connections.

## The board file: your rules for this board

Next to the netlist, a JSON file with the same name (`circuits/555_blinker.json` for
`circuits/555_blinker.net`) describes the board you're building on. Everything in it is optional:

```json
{
  "board": "18x12",
  "power": "+5V", "ground": "GND",
  "fixed":   { "U1": { "at": [6, 3], "rot": 0 } },
  "edge":    { "J1": "left" },
  "keepout": [ { "from": [16, 0], "to": [17, 1], "why": "M3 screw" } ],
  "near":    [ { "part": "C2", "pin": "U1.5" } ],
  "star":    { "GND": { "at": "C1.2", "runs": [["U1.1", "C2.2"], ["D1.1"]] } }
}
```

- **board / power / ground**: the same as the flags. A flag still wins, for a quick experiment.
- **fixed**: the part goes exactly there, and the placer never moves it. `at` is the top-left hole of the
  part after turning it, `rot` is quarter turns clockwise (0–3).
- **edge**: the part always touches that edge: `left`, `right`, `top` or `bottom`.
- **keepout**: holes no part may cover, from one corner to the other. Shown as `x` on the text board, a
  grey box in the pictures and in DIYLC, and a "rule area" in KiCad, which keeps enforcing it while you edit.

- **near**: the part should sit as close as it can to that pin, like a resistor guarding a GPIO or a
  decoupling capacitor by its chip. A preference: it costs per hole of distance.
- **star**: how a net is wired. Every run starts at the `at` pin and goes its own way, never sharing
  copper with another run, so one part's current doesn't shift the voltage another part sees. Each run
  in `runs` joins its pins in a chain; any other pin of the net gets a run of its own, straight from `at`.

fixed, edge and keepout are hard limits, not preferences. They're checked by the placer, never learned by
the network. Fixed and edge parts can't break their rule; if a keep-out can't be kept (no room left), the
run ends with a `!!` line naming the part. Coordinates are holes, counted like the numbers around the
text board. Pins are named like the `nets` command prints them.

## The pieces

| File | What it does |
|---|---|
| `circuits/555_blinker.net` | The example circuit as a KiCad netlist: the parts, their footprints, and which pins connect (nets). |
| `circuits/555_blinker.json` | Its board file: board size, power and ground, and the rules for this board. |
| `circuits/cat_feeder.diy` | A copy of the cat-feeder's `pcb.diy` (2026-10-17), with `cat_feeder.json`: its star wiring on C1 and R1 near GPIO2. |
| `src/constraints.rs` | Reads the board file and keeps its rules: fixed parts, edges, keep-outs, near, star. |
| `footprints.json` | Footprint name → size in holes and where each pad is. |
| `src/sexpr.rs` | Reads KiCad's bracket format, `(comp (ref "R1") ...)`, into a tree. |
| `src/netlist.rs` | Turns a KiCad netlist plus `footprints.json` into the circuit the placer works on. |
| `src/board.rs` | Places parts, works out where every pin lands, measures the board and gives it a score. |
| `src/route.rs` | The router: joins every net with copper traces, and hops over with a jumper where a trace can't get through. |
| `src/svg.rs` | Draws the layout as an SVG picture, from the parts' side or the solder side. |
| `src/kicad_pcb.rs` | Writes the layout as a KiCad board. |
| `src/diylc_import.rs` | Reads a DIYLC sketch: its parts, its modules (groups), and which pins connect. |
| `src/diylc.rs` | Writes the layout as a DIYLC project (your own parts, moved, if it came from one). |
| `src/place.rs` | The auto-placer: keeps nudging/rotating/swapping parts and keeps changes the judge likes. |
| `src/nn.rs` | The neural network, written out by hand (8 hidden neurons, ~60 weights). |
| `src/main.rs` | The `place`, `train` and `evolve` commands. `train` and `evolve` make ~1,200 example boards, have the rules score them, and train the network to match. |

## How a board is judged

`src/board.rs` measures six things (weights in `WEIGHTS`):

- **wire**: total length of copper trace and jumper wire, in holes, as routed (below).
- **jumpers**: how many jumpers the router needed where a trace couldn't get through.
- **power**: for every supply pin, the distance to the nearest ground pin (keeps supply and ground close).
- **overlap**: holes where two parts collide, a part hangs off the board, sits on a keep-out, or sits under a module without being low enough (very expensive).
- **area**: the size of the rectangle that holds all parts (keeps the board compact).
- **near**: for each `near` rule in the board file, the holes between the part and its pin.

The weighted sum is the *cost*; the score is `100 * exp(-cost / 150)`, so 100 is perfect.
The placer itself compares costs, not scores: for a big circuit the score is low and flat (a cost
of 400 scores about 6), so comparing scores would make it accept almost any bad move. Scores are
for reading; for a big circuit they're low even when the layout is good.

### How it routes

Every placer step routes the whole board (`src/route.rs`), so the placer sees which layouts need jumpers.

- A **copper trace** is on the solder side. It goes hole to neighbouring hole: across, down, or at 45°.
  It touches every hole it passes, so it can't pass a hole with another net's pin or trace, and two
  diagonals can't cross between four holes. It can run under parts, since it's on the other side.
- A **jumper** is a wire on the parts' side, so it can pass over anything. Where traces can't get
  through, traces run up to a free hole, a jumper (straight, up to 10 holes) hops over, and traces carry
  on from where it lands. Its legs never share a hole with a pin.

Nets are routed one at a time, shortest first; a star net is routed as one run per branch. Each run
grows like a tree: from what's joined so far, the cheapest way to the nearest pin not joined yet, trying
holes in the direction of that pin first ("A*" search). If even a jumper can't get there (walled in on
both ends), it gets a jumper straight from pin to pin, and it still counts as a jumper.

## Two ways to train the network

Both commands train the same 9-neuron network on the same example boards, then test it on
boards it never saw and save it to `out/model.json` (so the last one you ran is the judge).

- **`train` (gradient descent):** for each board, compare the network's guess with the rules'
  score and nudge every weight in the direction that makes it less wrong. One example points
  all 65 weights the right way at once.
- **`evolve` (evolution):** make 19 copies of the network with every weight changed a little at
  random, keep whichever of the 20 is least wrong, repeat 1,000 times. It never works out a
  direction; it only needs to know which copy did better.

Typical result on the 555: `train` is off by about 0.6 points on unseen boards in 1.7s, `evolve` by
about 0.7 points in 2.8s, after trying 19,000 networks. When you have a right answer for every example,
gradient descent wins. Evolution is the tool when all you can say is "this one did better than that one",
such as a network that proposes whole layouts.

## Where the neural network fits

Right now the network learns to copy the rules. On boards it has never seen it is off by
about a point, and the placer does just as well using it as the judge. On its own that
isn't useful, since we already have the rules. It proves the plumbing works.

It becomes useful when the "right answers" come from something the rules can't express:

1. **Your taste.** Rate layouts yourself; the network learns what you like, including things
   that are hard to write down as a rule.
2. **Richer input.** Feed it the board itself (which hole holds what) instead of six numbers,
   so it can spot patterns no rule measures. That's when a library like Burn starts paying off.
3. **Placing directly.** Train a network that proposes a layout in one shot instead of the
   placer trying thousands of moves.
