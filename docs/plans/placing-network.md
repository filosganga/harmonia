# Plan: a network that places parts

Status: stages 0 to 3 done (2026-10-17); stage 4 next (the maintainer said go on before seeing stage 3's numbers). Written 2026-10-17.

## Goal

**Better layouts than the placer finds on its own, in the same time, on circuits the network has
never seen.** Faster placing and learning are welcome side effects, not the goal.

This is "Placing directly" in README "Where the neural network fits". Today the network only copies
the rules' score (`src/nn.rs`, 9 numbers in, 1 out), so it adds nothing. Here it proposes where
parts go.

## The idea in one paragraph

A network places the parts **one at a time**. For each part it looks at the board so far and picks a
hole and a rotation. Then the existing placer (`place::anneal`, which already accepts a `start`)
polishes that layout, and the router wires it as today. The polished layouts are better than what
the network proposed, so they become the next round of training examples. Repeat: a better network
gives the placer better starts, the placer finds better layouts, and those train a better network.
This loop is called **expert iteration**, the same idea as AlphaZero's: the search teaches the
network, and the network guides the search. It's how the result can beat the plain placer, not just
copy it.

What doesn't change: the router stays a search (it has exact rules a network would only get roughly
right). Hard rules stay hard (README "Principles"): every layout the network proposes goes through
`constraints::apply` and the placer, so the network only chooses among allowed places.

## Stages

Each stage ends with something measurable. Stop and ask the maintainer at each **Decision** line.

### Stage 0: the yardstick

Before any network, decide how "better" is measured, and record today's numbers.

- A fixed **benchmark set**: `circuits/555_blinker.net`, `circuits/cat_feeder.diy`, and about 20
  sketches from the DIYLC corpus (CLAUDE.local.md says where it is), picked once and **never used for
  training**. List them in a file in the repo, by name, not by a machine path.
- For each: 3 seeds, the default steps, from a random start. Record the best cost and the time.
- A command for it (e.g. `bench`), so every later stage reruns the same thing. Follow README
  "Principles": the benchmark's settings are constants, not flags.
- Mind CLAUDE.md's waiting time (a couple of minutes). If the whole set takes longer, the command
  may take a subset or a smaller step count. Say which, and why.

**Done (2026-10-17).** What changed from the above, and why:

- **No corpus sketches in the set.** The DIYLC corpus turned out to be auto-backups of one circuit:
  the cat feeder in three versions (`pcb.*` is the cat feeder itself, `pcb-v2.*` a smaller ESP32
  version, `Untitled.*` an earlier Pi Zero one), about 80 different circuits with three nets or
  more, each a step or two from the next. Benchmark sketches would have been near-copies of
  training ones. Instead the set is the 555, the cat feeder, and ten classic hobby circuits written
  as KiCad netlists after datasheet applications and well-known designs (`circuits/bench/`, each
  with its source in the title block), listed in `circuits/bench.txt`. They needed eleven new
  footprints in `footprints.json` (DIP-14 isn't used yet; it's there for stage 1's random circuits).
- **All of it, every step count.** `cargo run --release -- bench` runs the 36 runs (12 circuits x
  seeds 1, 2, 3, 20,000 steps, random start) several at a time, one per core: about 21s on the
  maintainer's 18-core Mac, so no subset or smaller step count is needed. The costs come out the
  same as one run at a time (checked by running it twice and comparing); seed 1 on the 555 gives
  `place`'s 84.6. Results go to `out/bench.json`.

Today's numbers, the plain placer (best cost per seed; lower is better; times are per run with up to
18 runs sharing the machine):

| circuit | parts | nets | seed 1 | seed 2 | seed 3 | best | mean | s/run |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 555_blinker | 8 | 7 | 84.6 | 99.5 | 88.1 | 84.6 | 90.7 | 3.7 |
| cat_feeder | 11 | 18 | 464.2 | 392.9 | 401.7 | 392.9 | 419.6 | 20.5 |
| lm7805_supply | 10 | 5 | 67.6 | 62.1 | 71.0 | 62.1 | 66.9 | 3.4 |
| astable_flasher | 11 | 8 | 93.8 | 112.0 | 98.4 | 93.8 | 101.4 | 3.9 |
| lm386_amp | 11 | 8 | 81.9 | 102.4 | 78.3 | 78.3 | 87.5 | 3.9 |
| lm358_preamp | 14 | 9 | 141.0 | 117.1 | 127.8 | 117.1 | 128.6 | 5.1 |
| attiny85_board | 11 | 10 | 180.4 | 165.7 | 164.3 | 164.3 | 170.1 | 5.4 |
| led_chaser_4017 | 19 | 17 | 260.6 | 211.6 | 251.3 | 211.6 | 241.2 | 10.9 |
| pwm_motor_555 | 15 | 10 | 169.0 | 186.2 | 177.9 | 169.0 | 177.7 | 9.0 |
| npn_load_switch | 10 | 6 | 67.7 | 71.4 | 81.0 | 67.7 | 73.4 | 3.4 |
| lm317_adjustable | 13 | 5 | 102.9 | 123.9 | 116.6 | 102.9 | 114.5 | 4.7 |
| h_bridge_bjt | 15 | 8 | 117.0 | 128.3 | 136.0 | 117.0 | 127.1 | 6.5 |

(Rerun the same day after two footprints were corrected against KiCad's own files: the screw
terminal and the trimmer, used by lm7805_supply, lm386_amp, pwm_motor_555, npn_load_switch and
lm317_adjustable.) The spread between seeds (about 30% on the lm386, 20% on the cat feeder and the chaser) is how
much a later stage has to beat before a difference means anything.

### Stage 1: training data

Examples of "this part went here": (circuit, board so far, the part to place next, where it went).

- **Sources:** the corpus sketches not in the benchmark, and **random circuits** made in Rust from
  the parts in `footprints.json` with random nets. The corpus alone (about 190 readable sketches)
  is probably too few. Random circuits are cheap but unrealistic, so keep both and say how many of
  each.
- **Decided 2026-10-17: random circuits only.** The corpus is all the cat feeder (stage 0), so
  training on it would make the cat feeder a circuit the network has seen. The maintainer chose to
  keep it out, so every benchmark circuit stays unseen.
- **Labels:** run the placer on each circuit and save its best layout to `out/` (git-ignored), as
  JSON. A layout gives one example per part, in the placing order below.
- **Placing order:** a fixed rule, e.g. parts with fixed or edge rules first, then by number of
  connections, biggest first. The network then always learns the same kind of decision.
- **Time:** making data for hundreds of circuits takes far longer than an espresso. Make it
  incremental: each run adds a batch within a couple of minutes and appends to what's there.

**Done (2026-10-17).** `cargo run --release -- data` adds 200 random circuits
(`src/random_circuit.rs`) to `out/data/layouts.jsonl`, each placed from a random start with
`place`'s default steps, one per core: about 75s per batch on the maintainer's Mac. Circuit n is
always the same circuit placed the same way (checked: the first batch made twice is the same, byte
for byte). Placing order (`data::placing_order`): parts with placement rules first, then most
connected pins, then biggest, then by name.

Five batches made: **1,000 circuits, 13,122 examples** (one per part). What they're like, and how
they differ from the benchmark circuits:

- 6 to 20 parts, evenly spread (59 to 79 circuits of each size). The benchmark has 8 to 19.
- Parts, most common first: resistors (27%), ceramic and electrolytic capacitors, LEDs, TO-92
  transistors, headers, trimmers, diodes, terminal blocks, DIP-8, TO-220, other headers, DIP-16,
  DIP-14.
- Boards from 96 to 704 holes, sized so the parts cover 18 to 32% of them (the benchmark: 19 to 31%),
  at least 12 x 8 holes, so a small circuit may cover less.
- **Fewer big nets than real circuits.** Signal nets are 79% two-pin, 19% three, 2% four, about one
  net per part. The benchmark circuits have fewer nets per part (0.4 to 0.9) and more of 3 to 5
  pins, and their supply and ground nets reach 8 to 11 pins. If stage 2 does worse on the benchmark
  than on held-out random circuits, this is the first thing to change.
- No modules (nothing like the cat feeder's ESP32 on headers): `footprints.json` has none.

### Stage 2: the network (Burn)

- **Add Burn** (the maintainer said yes). Use a pure-Rust CPU backend (`ndarray`) first; try
  `wgpu` (Metal on the Mac) only if training is too slow. Don't use the libtorch backend: it
  downloads a C++ library. **Check the current Burn version and its API in its docs before
  writing code**; don't trust memory, the API changes between versions.
- **What it sees: a picture of the board**, one grid per kind of thing (called "channels"):
  holes taken by parts already placed, pins on the next part's nets, supply pins, ground pins,
  keep-outs, off the board, and the places the board file allows for this part.
- **How it chooses (recommended): score each candidate.** For every allowed (hole, rotation) of
  the next part, draw the board as it would be with the part there, and let a small convolutional
  network give it one number. Pick the highest. The part's shape and pins come for free, because
  they're drawn into the picture. Training is "pick the one the placer picked" over all candidates
  (softmax and cross-entropy). The other design (one heat map of scores per rotation in a single
  pass) is faster but has to be told the part's shape some other way. Try it only if scoring every
  candidate is too slow.
- **Measure on held-out circuits:** how often its first choice is the placer's choice, and how
  often the placer's choice is in its top 5. Compare with a trivial baseline: "put it next to the
  part it shares the most nets with".
- **Decision:** if it isn't clearly better than that baseline on unseen circuits, stop and discuss
  before stage 3: more data, other channels, or another design.

**Done (2026-10-17).** `cargo run --release -- learn` (`src/placing.rs`), Burn 0.22.0. What changed
from the above, and why:

- **Backend `flex`, not `ndarray`:** Burn 0.22 deprecated ndarray; flex is its pure-Rust CPU backend.
  0.22 also changed the API (no backend type parameter: tensors carry a `Device`). Clean build 7.3s
  -> 40.5s, 24 -> 206 crates.
- **Candidate scoring, as recommended.** 8 channels: holes taken, the new part's body, its pins,
  placed pins on its nets, ratsnest lines (each new pin to the nearest placed pin of its net), supply
  pins, ground pins, off the board and keep-outs. Three 3 x 3 convolutions (16, 32, 32 filters), then
  the sum and the max over the board, then one score. Candidates are the places the part may take
  (placement rules, on the board, off placed bodies and keep-outs, module rules).
- **Sampled wrong places in training:** each step compares the placer's choice with 63 other places
  picked at random (all ~800 would be over 12 times the pictures, so roughly half an hour an
  epoch at the measured speed); the final test compares with all of them.
  Cross-entropy is written out (`log_softmax`), since Burn's assumes the same number of choices per
  row.
- **Benchmark right answers:** `bench` now saves each circuit's best layout in `out/bench.json`.
- **A second, stronger baseline:** shortest total ratsnest, next to the plan's.

Results (1,000 circuits from `data`, every 10th kept aside; 5 epochs, Adam, learning rate 0.001):

| unseen circuits | network top-1 / top-5 | shortest ratsnest | next to the part sharing most nets | random guess |
|---|---|---|---|---|
| random circuits kept aside (1,326 decisions) | 12.4% / 30.2% | 1.6% / 10.7% | 0.2% / 1.3% | 0.4% / 1.9% |
| benchmark circuits (148 decisions) | 9.5% / 28.4% | 2.6% / 10.0% | 0.5% / 2.2% | 0.5% / 2.5% |

Loss per epoch, training / kept aside: 3.11 / 2.83, 2.88 / 2.75, 2.82 / 2.71, 2.79 / 2.71, 2.76 /
2.68: still falling, so more epochs would likely help. The plan's baseline does no better than a
guess: the placer doesn't put parts as close as it can to their partner, since crowding pushes them
apart.

**Decision (maintainer, 2026-10-17): go on to stage 3.** Training takes about 14 minutes (130s an
epoch on the CPU, not the 45s estimated from a smaller test network), past the usual espresso; the
maintainer chose accuracy over speed for training, and will try faster combinations later (a smaller
network, fewer wrong places, epochs carried over between runs, Metal through `wgpu`).

### Stage 3: the head start

- `place` can start from the network's layout instead of a random one, e.g. `place --start net`,
  like the existing `--judge net`. The placer still has the last word, so the result is always an
  allowed, polished layout. It can still come out worse than a random start; that's what the
  benchmark is for.
- Rerun the stage 0 benchmark. Report two things: the best cost after the same steps, and the steps
  needed to reach today's best cost.
- **Decision:** is it worth going on to stage 4?

**Done (2026-10-17).** `place --start net` and `bench --start net` (saves `out/bench-net.json` and
compares with `out/bench.json`). `placing::propose` places the parts in the placing order, each where
the network scores best (a part with no room left starts at random; the placer sorts it out).
`anneal` now takes a `Start` and reports every step, so `bench` records how the best cost falls.
The plain placer's results are unchanged (the same costs to the decimal, `place` on the 555 84.6,
`place circuits/cat_feeder.diy` 454.3 in 16.6s).

**The hot start, as feared, then fixed.** At the usual starting temperature (20) the network's
head start was thrown away: lower mean cost on only 4 of 12 circuits, and both starts reached the
plain placer's mean only at step 18-19k. Starting cooler from the network
(`place::NETWORK_START_TEMPERATURE`), geometric mean of (mean cost from the network / plain mean
cost) over the 12 circuits, and without the cat feeder:

| start temperature | all 12 | without the cat feeder | lower mean on |
|---:|---:|---:|---:|
| 20 | +3.5% | +1.4% | 4 of 12 |
| **5** | **-0.5%** | **-4.8%** | **7 of 12** |
| 2 | 0.0% | -3.9% | 7 of 12 |
| 0.5 | +11.3% | +7.6% | 3 of 12 |

5 is kept. (It was chosen on the benchmark, as this plan says; the benchmark is no longer quite
unseen for this one setting.)

With 5, per circuit (mean of 3 seeds; steps to reach the plain placer's mean, each seed, "-" never):

| circuit | start: random / network | mean after 20k: plain / network | steps: plain | steps: network |
|---|---:|---:|---|---|
| 555_blinker | 420 / 95 | 90.7 / **73.1** | 18.3k - 18.6k | 6.8k 0.3k 9.2k |
| cat_feeder | 8297 / 7182 | **419.6** / 682.4 | - 33.0k 36.7k | - - - |
| lm7805_supply | 483 / 81 | 66.9 / **60.8** | - 19.6k - | 14.3k 16.6k 13.5k |
| astable_flasher | 606 / 144 | 101.4 / **90.1** | 18.2k - 19.9k | 11.7k 7.7k 6.0k |
| lm386_amp | 531 / 115 | 87.5 / **76.5** | 19.1k - 18.0k | 8.3k 0.6k 7.5k |
| lm358_preamp | 1147 / 356 | 128.6 / **122.4** | - 18.6k 19.8k | - 17.8k 12.5k |
| attiny85_board | 992 / 346 | **170.1** / 198.0 | - 18.7k 18.0k | - - - |
| led_chaser_4017 | 2178 / 763 | **241.2** / 246.3 | - 18.7k - | 13.2k - 13.8k |
| pwm_motor_555 | 1463 / 390 | **177.7** / 179.2 | 19.3k - - | 19.1k 14.1k - |
| npn_load_switch | 487 / 196 | 73.4 / **71.0** | 18.6k 19.8k - | 16.0k 16.7k 15.4k |
| lm317_adjustable | 713 / 171 | **114.5** / 117.4 | 18.9k - - | - 14.3k - |
| h_bridge_bjt | 654 / 192 | 127.1 / **114.6** | 19.1k - - | 17.7k 13.7k 11.3k |

(The cat feeder's steps count on through its two cases, 20k each.)

- The network's layout alone costs 2.5 to 6 times less than a random start, except on the cat
  feeder: it has modules (the ESP32 and two others on headers), the random training circuits have
  none, and the network places the big modules so that one has no room left and starts on top of
  the others. Modules in the training data are the obvious next fix (stage 1's risk).
- Where it helps it gets there sooner: within 0.3k to 12k steps on the 555, the LM386 amp and the
  transistor flasher (the plain placer: 18k to 20k, or never), a few thousand steps sooner on the
  others.
- Steps are counted to the plain placer's **mean** cost, not its best as asked above: the best is
  the luckiest of three seeds, which a single run rarely reaches, so it would mostly print "never".
  The mean is the typical result.
- Using the network costs well under a second per circuit; `bench --start net` takes about 27s.

### Stage 4: the loop (expert iteration)

- Round n: place the training circuits starting from network n, polish with the placer, keep the
  better of the new layout and the old label, retrain into network n+1.
- After each round, rerun the benchmark. Stop when a round no longer helps.
- Report the curve: benchmark cost per round, against the plain placer's.

## Rules for the work (from CLAUDE.md and README)

- Explain in plain language as you go. This project is also how the maintainer learns Rust and
  neural networks, so say what a convolution, softmax or an epoch is when they first come up.
- Model settings (layers, channels, learning rate, rounds) are constants in the code, justified by
  the held-out numbers, and noted in README. No flags for them.
- Rust only for everything `cargo run` does. Python is fine for a plot.
- After touching the placer, router or `measure`: time `place circuits/cat_feeder.diy` and say how
  long it took. Run `check` (README "Checking a change") with two or three seeds.
- Update README in the same change when its facts change: "Where the neural network fits", "The
  pieces", "Next steps", the commands and the timings.
- Conventional Commits, and run the `drift-check` agent before each commit.

## Risks

- **It may not carry over to new circuits.** That's what stage 2's decision is for, before
  investing in stages 3 and 4.
- **Too little realistic data.** Random circuits help with volume but may teach habits that don't
  fit real boards. Watch the benchmark, not the training numbers.
- **Burn makes the build much slower** and adds many crates. Accepted by the maintainer. Say how
  long a clean `cargo build --release` takes before and after.
- **The head start may not help.** Annealing starts hot and shakes the layout up, so a good start
  can get thrown away early. (Stage 3: it did, at the usual temperature; starting at 5 fixed it.) If stage 3 shows that, try starting the placer cooler when it starts
  from the network. That's a constant in `place.rs`, justified by the benchmark.
