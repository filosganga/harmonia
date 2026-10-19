# Contributing to Harmonia

Thanks for looking. Harmonia is a small project and also a way to learn Rust and neural networks,
so plain, readable code matters more here than clever or fast code.

## Before you start

[README.md](README.md) is the whole documentation: the commands, the board file, how a board is
judged and routed, the known limits and the next steps. Read it first. For anything bigger than a
fix, open an issue to talk it over before writing the code.

## Building and running

You need a recent stable Rust (the crate uses edition 2024).

```bash
cargo build --release
cargo run --release -- place                          # the 555 example
cargo run --release -- place circuits/cat_feeder.diy  # a bigger, real board
```

The pictures and other outputs go to `out/` (or the folder `--out` names).

## Checking a change

Follow README "Checking a change". In short:

- `cargo run --release -- check circuits/cat_feeder.diy --seed N` with two or three seeds, and on a
  folder of your own DIYLC sketches if you have some.
- Look at the pictures you changed (`out/rules.svg`, `out/rules-solder.svg`), not just the text.
- If the change is meant to make layouts better, `cargo run --release -- bench` and compare the costs
  with the last run (`out/bench.json`); after a change to the score's weights or measurements the
  old costs no longer compare (README "The benchmark").
- `cargo clippy` has two known warnings (`board::size`, `diylc::two_legged`); don't add new ones.
- If you changed what a board is measured on, run `cargo run --release -- train` again and say how far
  off the network is on unseen boards.

CI runs the build, clippy, the 555 example (and `draw` on the layout file it saves, which must give
the same files) and the cat-feeder round trip on every pull request.

## Adding a footprint

A KiCad netlist names each part's footprint but not where its legs are, so every footprint a netlist
uses needs an entry in `footprints.json`. When one is missing, `place` stops with "R5 uses footprint
X, which isn't in the built-in footprints.json yet". The repo's `footprints.json` is built into the
program: `cargo run` rebuilds it by itself, an installed `harmonia` needs `cargo install --path .`
again. A part only one project uses can go in a `footprints.json` next to that netlist instead,
which adds entries (or replaces ones with the same name) for circuits in that folder. (DIYLC sketches don't need entries: their parts come from
DIYLC. For a DIYLC part the reader refuses, see `.claude/skills/perfboard-diylc-part`.)

1. **The name** is the footprint's name after the `:`, exactly as the netlist writes it:
   `Package_TO_SOT_THT:TO-92_Inline_Wide` is looked up as `TO-92_Inline_Wide`. Use the names of
   KiCad's own libraries, so a netlist exported from KiCad finds them.
2. **Find KiCad's file for it**, a `.kicad_mod`. KiCad installs its libraries as folders named
   `<Library>.pretty` (Preferences → Configure Paths shows where; on macOS they're inside the app, in
   `KiCad.app/Contents/SharedSupport/footprints/`), and they're online at
   <https://gitlab.com/kicad/libraries/kicad-footprints>.
3. **Draft the entry** from it:

   ```bash
   python3 dev/kicad_footprint.py path/to/TO-220-3_Vertical.kicad_mod
   ```

   It turns millimetres into holes (2.54mm apart, lined up with pad 1) and prints `w`, `h` and
   `pins`, plus notes. The outline is the holes under the part's body as KiCad draws it on the
   fabrication layer (F.Fab, the real body): a hole counts only if the body reaches 0.5mm past its
   centre, since a leg still fits beside a body that only just reaches a hole. That's why a TO-220,
   10mm wide, comes out 3 holes wide and 2 deep (the body stands behind its legs). Pins are
   `[x, y]` from the outline's top-left hole, x to the right and y down, with the part as KiCad
   draws it, unturned.
4. **Read the notes.** A pad "between holes" means its leg must be bent to reach one. Often KiCad
   has a variant on the grid: `TO-92_Inline` has its legs 1.27mm apart, `TO-92_Inline_Wide` 2.54mm,
   the way a TO-92 is usually bent onto perfboard. `footprints.json` can't put a leg between
   holes.
5. **Check which pad is which.** The netlist joins pins by pad number, and what each number means
   comes from the part's KiCad symbol and datasheet: for KiCad's BC547 pad 1 is the collector, for
   an L7805 the input. A wrong order wires the wrong legs without any error. Write it in `"about"`.
6. **Add by hand** what a footprint file doesn't say:
   - `"about"`: what the part is and which pad is which.
   - `"low": true` if the part lies flat enough to sit under a module on headers (a resistor, a
     ceramic disc, a diode lying down). Leave it out otherwise.
   - `"module": true` for a module on headers (KiCad's `Module` library: an Arduino Nano, a Pico, a
     breakout board): only its pins take holes, and low parts can go under its board.
   - `"diylc"`, if DIYLC has a matching part: `{ "part": "Resistor" }` and the others README "Using
     your own circuit" lists, with `"legs"` for a polarised two-legged part. Without it the part is
     drawn in the `.diy` as a labelled box.
7. **Without a KiCad file**, measure from the datasheet the same way: the leg spacing and the body's
   size in mm, divided by 2.54. KiCad's names carry the main ones: `P10.16mm` is the leg spacing (4
   holes), `W7.62mm` a DIP's row spacing (3 holes), `D5.0mm` a round body's diameter.
8. **Check it**: `cargo run --release -- footprints your.net` shows the entry as the program read it,
   and `nets your.net` lists the part with its size and number of pins;
   `place` it and look at `out/rules.svg` (each leg in its hole, the outline over the body). With a
   `"diylc"` entry, open `out/rules.diy` in DIYLC. In KiCad, open `out/rules.kicad_pcb` and run
   Tools → Update Footprints from Library: the real footprint's pads should land on the same holes.

Changing an entry that's already there moves the legs of every circuit that uses it, the benchmark
circuits too: rerun `bench` and say which numbers changed.

## How the project is shaped

These are from README "Principles"; a pull request that goes against them will be asked to change.

- **Rust only for the program.** Everything `cargo run` does is Rust, machine learning included. Dev
  tools around it (a checker, a plot) may be in another language, as long as the program never needs
  them.
- **No flags for model settings.** Hidden neurons, learning rates and the score's weights (`WEIGHTS`) are
  constants in the code (a board file's own rule `weight` is the user's problem, not a model setting). Flags are for the user's problem: the circuit, the board, the seed, how hard to try.
- **Hard rules are never learned.** Where a part may go and what's allowed on the board are enforced by
  the placer (unless the board file gives a rule a `weight`, which makes it a cost); the network only judges taste among layouts that are already allowed.
- **Board-file rules compose.** Add small, simple rules that combine with `all-of` and `one-of` rather
  than one bigger rule for a special case.
- **Docs change with the code.** If a change makes something in README.md wrong (a timing, an output,
  a measurement, a rule), fix the README in the same pull request.

## Commit messages

Start each commit's subject with what kind of change it is, in the
[Conventional Commits](https://www.conventionalcommits.org) style, then say what it does in plain
words, lowercase:

- `feat:` something new a user can do (a command, a board-file rule, a DIYLC part).
- `fix:` something that was wrong now works.
- `docs:`, `refactor:`, `perf:`, `test:`, `ci:`, `chore:` for the rest.

Add `!` after the type (`feat!:`) when old board files, sketches or commands stop working. The
release PR uses these to sort the changelog and to pick the next version.

## Releasing

Releases are made with [release-plz](https://release-plz.dev) (`release-plz.toml`,
`.github/workflows/release.yml`):

1. After each push to main, release-plz opens or updates a pull request named like "chore: release
   v0.2.0". It bumps `version` in `Cargo.toml` (and `Cargo.lock`) and adds the commits since the last
   release to `CHANGELOG.md`. Edit the version or the changelog in that pull request if you want.
2. Merging it is the release: release-plz tags the merge commit `v0.2.0` and creates the GitHub
   release, and the same workflow builds the program for Linux, macOS and Windows and attaches it.

So every release is a commit in the history, with its version written in `Cargo.toml`. Nothing is
published on crates.io. release-plz works as the maintainer's `filosganga-release` GitHub App (the
`APP_CLIENT_ID` variable and `APP_PRIVATE_KEY` secret), so CI runs on release PRs like on any other.

## Working with an AI assistant

`CLAUDE.md` and `.claude/` (skills for common changes, and a `drift-check` agent that looks for
documentation a change made wrong) are there for working with Claude Code. They're also a good
checklist if you work without it.

## License

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
Harmonia, as defined in the Apache-2.0 license, shall be dual licensed as MIT OR Apache-2.0 (see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE)), without any additional terms or
conditions.
