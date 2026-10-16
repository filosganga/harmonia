# Perfboard layout neural network

Goal: a neural network that scores and improves component placement on perfboard, from a netlist
with pinouts and component sizes in perfboard hole coordinates. Good-practice signals: 5V and GND
close together, short wiring, few jumpers, no collisions, compact board, and the board's own rules.

README.md is the documentation for people and for you: commands, files, rules, how a change is
checked, known limits, principles and next steps. Read it first. This file is only what you need on
top; what depends on this machine (sibling repos, tool paths) is in CLAUDE.local.md.

## Working with Filippo

- He is learning Rust and new to neural networks: explain in plain language, keep the code readable.
- The program is Rust only: everything `cargo run` does, including any machine learning (Burn or
  Candle if a library is needed). Python is fine for dev tools around it (a checker, a converter, a
  plotting script), as long as the program never needs them to run.
- If a model setting needs changing, change the constant, justify it with the unseen-board error, and
  note it in README.md (README "Principles": no flags for these).
- Keep the feedback loop fast: every command should finish in about a second. After touching the
  placer, router or `measure`, time `place circuits/cat_feeder.diy` and say how long it took.
- When README.md's facts change (timings, what an output contains, the measurements), update it in the
  same change.
- TODO.md is Filippo's own list of ideas, in his words. Read it before designing anything in the board
  file. Don't rewrite his notes; when one is done, ask whether to remove it.

## Checking your work

- `cargo run --release -- check` (README "Checking a change") on `circuits/cat_feeder.diy` with two or
  three seeds, and on a folder of real sketches if there is one (CLAUDE.local.md).
- Look at the pictures you changed, not just the text output.
- `cargo clippy` has two old warnings (`board::size`, `diylc::two_legged`); don't add new ones.
- Before committing, run the `drift-check` agent (`.claude/agents/`) on the change: it looks for
  README/CLAUDE.md facts the change made wrong.

## Skills for common changes (`.claude/skills/`)

- `perfboard-measurement`: adding or changing a score measurement or weight.
- `perfboard-board-rule`: a new board-file rule (hard limit or preference).
- `perfboard-diylc-part`: supporting another DIYLC part.
