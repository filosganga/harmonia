# Open questions

Questions from the router rework (October 2026) that need the maintainer's decision, written so a
later session can pick any of them up cold. Each says what we know, with the numbers and how to get
them again, the options, and a suggestion. Remove a question once it's decided.

Measured on 2026-10-20 at the commit that added this file.

## 1. Jumpers against diagonals and corners: what should the weights prefer?

**The question.** On the hand-placed cat feeder, the router uses about twice as many jumpers as the
hand route, and by the score it's right to: the hand route pays for its few jumpers with diagonals
and corners, which the weights price higher. Is that the trade you want?

**What we know.** Same placement (the DIYLC sketch as drawn), same rules:

```
cargo run --release -- place circuits/cat_feeder.diy --board-file circuits/cat_feeder_fixed/hand.json --steps 1
```

prints both wirings of the sketch's placement ("wired as you drew it", "wired by the router"):

| measurement | weight | hand route | router | hand pays | router pays |
|---|---|---|---|---|---|
| wire (holes of trace) | 1 | 273 | 211 | 273 | 211 |
| turns | 5 | 52 | 24 | 260 | 120 |
| diagonal steps | 15 | 20 | 0 | 300 | 0 |
| crosswise steps | 0.5 | 118 | 105 | 59 | 52.5 |
| jumpers | 15 | 4 | 9 | 60 | 135 |
| jumper_len | 0.5 | 15 | 62 | 7.5 | 31 |
| solder-side wires | 500 | 0 | 0 | 0 | 0 |
| power, crowding (the placement's) | | | | 60 | 60 |
| rules (soft star joins, near) | 1000 | 0.01 | 0.025 | 10 | 25 |
| **cost** | | | | **1029** | **635** |

The hand route saves 5 jumpers (75 + 23.5 for their length), but has 20 diagonal steps (300) and
28 more quarter turns (140), and 62 more holes of trace. With every part fixed by the board file
(`circuits/cat_feeder_fixed/orig.net`, `orig.json`), the router finds 677 with 11 jumpers.

The weights say what they say on purpose (`WEIGHTS` in src/board.rs, and the memory "corners over
length"): a jumper is three corners' worth, a diagonal step three corners on top of its length, and
CLAUDE.md asks for "no diagonals, stripboard-like". But your own board has 20 diagonal steps and 4
jumpers. So either the hand route took diagonals because they were easier by hand than a jumper,
and the weights are wrong for you, or the diagonals were a compromise and the router's board is
what you'd rather solder.

**Options.**
- Keep the weights: the router's 9 to 11 jumpers are the better board by your own prices.
- Make jumpers dearer. For the hand route to win on this placement as it is, a jumper would need
  to cost about 95 instead of 15. A dearer jumper also changes what the router proposes, so the
  real result would be a new route somewhere between the two.
- Make diagonals or corners cheaper, if you're happy to solder them.

**How to try it.** A weight is a constant (README "Principles": no flags for these). Change one
in `WEIGHTS`, run the line above and `place circuits/cat_feeder_fixed/orig.net --steps 1` (both
take a second), then `bench` (about 50s) to see what it does to every other circuit. The router
prices its search with the same weights (route/search.rs `JUMPER` and the others), so change those
to match.

**Suggestion.** Look at the router's board first (`out/rules.svg` after the second command) and
the hand route next to it. If you'd solder the router's, nothing to change. If not, tell me which of
your diagonals and corners you'd happily make again, and we'll price them that way.

## 2. A random start can find no room for a part

**The question.** Should the placer try another random start when the first one leaves a part no
room, instead of stopping?

**What we know.** It stops on one seed of six:

```
cargo run --release -- place circuits/cat_feeder_fixed/feeder.net --board-file circuits/cat_feeder_fixed/free.json --seed 2
```

stops with "no case has room for every part: no room for U2 on the 24x18 board, next to the parts
already down". Seeds 1 and 3 to 6 start fine.

`place::allowed_start` puts the parts down one at a time: those with the fewest allowed places
first (J1 and J3, kept on an edge), then the biggest. Each takes the free place nearest to where it
was dropped at random, and nothing already down is ever moved. The three modules are big for a
24x18 board (U3 15x8, U1 9x7, U2 7x6) and can't overlap each other, so once J1, J3, U3 and U1 sit
where the random drop put them, U2 can be left with no gap big enough. The board isn't too small:
other seeds fit everything.

**Options.**
- Try again from a fresh random drop, a few times, before giving up. Only a start that fails
  draws more random numbers, so every seed that works today makes the same layout.
- Place smarter: biggest parts first whatever their rules, or move a part already down when the
  next has no room. Better packing, but it changes every start, so every layout.

**Suggestion.** The retry: small, and nothing that works today changes. (It only matters for a
random start; a sketch's start comes from your drawing.)

## 3. On the benchmark the cat feeder's cost is mostly its board file's rules

**The question.** Should `near` and `close` without a weight cost 1000 per hole, as they do now?
The answer decides what the cat feeder's benchmark numbers mean.

**What we know.** This corrects what was believed during the rework (that seed 3 left a pin
unrouted). Every pin is routed on all three seeds. The breakdown of the three benchmark layouts
(printed by a temporary line in `bench`, not committed):

| seed | cost | rules | rules × 1000 | everything else |
|---|---|---|---|---|
| 1 | 4994 | 4.41 | 4414 | 580: 9 jumpers |
| 2 | 5619 | 5.0 | 5000 | 619: 9 jumpers |
| 3 | 8146 | 5.83 | 5828 | 2318: 16 jumpers, 3 solder-side wires (1500) |

`circuits/cat_feeder.json` (the benchmark's copy of your board file) has three `near` rules and one
`close`, none with a weight. Without one they count 1, so each hole of copper between the two pins
costs 1000, as much as two solder-side wires. The placer then spends most of its effort on those
rules, and the score is 0/100 on every seed. Your own board file for the battery-sense board
(`circuits/cat_feeder_fixed/orig.json`, copied from the cat-feeder repo) gives its `near` 0.01 and
its stars 0.015.

**Options.**
- Give the rules in `circuits/cat_feeder.json` the weights you use now. The cat feeder's benchmark
  numbers then start again from new: the set of circuits stays, but old numbers no longer compare
  for this one circuit.
- Change what `near` and `close` cost without a weight (README "The board file", **weight**): a
  rule with no weight is "hard", and these two can't be hard because they're only known once
  routed, so 1 is "nearly hard". A smaller default would apply to every board file.
- Leave it: the benchmark measures how well the placer keeps hard-ish rules, which is fine too.

**Suggestion.** The first, if 0.01 is what you really use: the benchmark should look like your
real board files. Seed 3's three solder-side wires are a separate thing, a placer that got stuck
on that seed.

## 4. The placer pays for moves that change nothing

**The question.** May the placer's moves change, which changes every layout and needs the benchmark
again? (Deferred to the next time the placer is worked on.)

**What we know** (src/place.rs `a_move`, from reading the code; the share is estimated, not
measured):
- `roll` is drawn before the jump test, so 30% of moves draw a random number they don't use.
- A nudge can be (0, 0), or be clamped to the same hole at an edge (1 in 25 nudges or more).
- A swap can pick the part itself (1 in the number of movable parts).
- A rotation of a part whose rules fix its orientation is put back by `constraints::apply`, and a
  jump can land where the part already is.

Each of these is routed and scored like a real move, a few percent of all moves together.

**Options.**
- Free and output-identical: when the move leaves the layout as it was, don't route it. It would
  score the same and be accepted, which changes nothing, and no random number is drawn for it.
- With the placer work: make every move change something, and drop the unused `roll`. This
  changes the random sequence, so every layout changes; check with `bench` on 6 seeds (3 are too
  noisy for the LED chaser).

**Suggestion.** The free part any time; the rest with the next placer change.

## 5. Both sides of the board, handled the same way

Your note in TODO.md ("Both sides of the board, handled the same way"). Today a part with room
under it (a module on headers, but also a DIYLC two-legged part drawn at an angle, which only takes
its legs: `Component::room_under`) lets any low part go under it.

A starting point for the design, not decided: a few flat layers instead of heights. The solder side
(traces, solder-side wires); the parts' side on the board (parts, jumpers); and raised (a module's
board on its headers). Each thing says which layer it takes, and two things clash only when they
share a hole on the same layer. A part on the bottom is a part on the solder-side layer; an angled
resistor takes its whole box on the parts' side; whether a module is raised could be a setting.
It changes layouts, so it needs its own branch and the benchmark.

## Also worth knowing

- Placing is slower since the router rework: `place circuits/cat_feeder.diy` takes about 37s
  (18s before) and `bench` about 50s (33s), because wires became part of the router's search.
- The installed `harmonia` (`cargo install --path .`) must be reinstalled to read layout files of
  version 2.
