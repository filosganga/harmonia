# Harmonia: perfboard layout neural network

Goal: a neural network that scores and improves component placement on perfboard, from a netlist
with pinouts and component sizes in perfboard hole coordinates. Good-practice signals: 5V and GND
close together, short wiring, few bends in the traces, straight traces running mostly one way (no diagonals, stripboard-like), few jumpers, no collisions, room around
parts to solder, and the board's own rules.

README.md is the documentation for people and for you: commands, files, rules, how a change is
checked, known limits, principles and next steps. Read it first. This file is only what you need on
top; what depends on one machine (sibling repos, tool paths) goes in a git-ignored CLAUDE.local.md,
if there is one. CONTRIBUTING.md is the same for people.

## How to work here

- The project is also a way to learn Rust and neural networks: explain in plain language, and keep
  the code readable.
- The program is Rust only: everything `cargo run` does, including any machine learning (Burn or
  Candle if a library is needed). Python is fine for dev tools around it (a checker, a converter, a
  plotting script), as long as the program never needs them to run.
- If a model setting needs changing, change the constant, justify it with the unseen-board error, and
  note it in README.md (README "Principles": no flags for these).
- Waiting time: a command may take as long as making and drinking an espresso (a couple of
  minutes); quicker is nicer but not a goal that beats clarity. After touching the placer, router or
  `measure`, time `place circuits/cat_feeder.diy` and say how long it took.
- When README.md's facts change (timings, what an output contains, the measurements), update it in the
  same change.
- TODO.md is the maintainer's own list of ideas, in their words. Read it before designing anything in
  the board file. Don't rewrite those notes; when one is done, ask whether to remove it.

## Checking your work

- `cargo run --release -- check` (README "Checking a change") on `circuits/cat_feeder.diy` with two or
  three seeds, and on a folder of real sketches if there is one (CLAUDE.local.md).
- Look at the pictures you changed, not just the text output.
- After a change meant to make layouts better: `cargo run --release -- bench` (README "The benchmark").
- `cargo clippy` has two old warnings (`board::size`, `diylc::write::two_legged`); don't add new ones.
- Commit subjects use Conventional Commits prefixes (`feat:`, `fix:`, `docs:`, `ci:`, `chore:`...;
  CONTRIBUTING.md "Commit messages"): the release PR builds the changelog and the version from them.
- Before committing, run the `drift-check` agent (`.claude/agents/`) on the change: it looks for
  README/CLAUDE.md facts the change made wrong.

## Skills for common changes (`.claude/skills/`)

- `perfboard-measurement`: adding or changing a score measurement or weight.
- `perfboard-board-rule`: a new board-file rule (hard limit or preference).
- `perfboard-diylc-part`: supporting another DIYLC part.
