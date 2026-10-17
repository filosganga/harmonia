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

The pictures and other outputs go to `out/`.

## Checking a change

Follow README "Checking a change". In short:

- `cargo run --release -- check circuits/cat_feeder.diy --seed N` with two or three seeds, and on a
  folder of your own DIYLC sketches if you have some.
- Look at the pictures you changed (`out/rules.svg`, `out/rules-solder.svg`), not just the text.
- `cargo clippy` has two known warnings (`board::size`, `diylc::two_legged`); don't add new ones.
- If you changed what a board is measured on, run `cargo run --release -- train` again and say how far
  off the network is on unseen boards.

CI runs the build, clippy, the 555 example and the cat-feeder round trip on every pull request.

## How the project is shaped

These are from README "Principles"; a pull request that goes against them will be asked to change.

- **Rust only for the program.** Everything `cargo run` does is Rust, machine learning included. Dev
  tools around it (a checker, a plot) may be in another language, as long as the program never needs
  them.
- **No flags for model settings.** Hidden neurons, learning rates and rule weights are constants in the
  code. Flags are for the user's problem: the circuit, the board, the seed, how hard to try.
- **Hard rules are never learned.** Where a part may go and what's allowed on the board are enforced by
  the placer; the network only judges taste among layouts that are already allowed.
- **Board-file rules compose.** Add small, simple rules that combine with `all-of` and `one-of` rather
  than one bigger rule for a special case.
- **Docs change with the code.** If a change makes something in README.md wrong (a timing, an output,
  a measurement, a rule), fix the README in the same pull request.

## Working with an AI assistant

`CLAUDE.md` and `.claude/` (skills for common changes, and a `drift-check` agent that looks for
documentation a change made wrong) are there for working with Claude Code. They're also a good
checklist if you work without it.

## License

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
Harmonia, as defined in the Apache-2.0 license, shall be dual licensed as MIT OR Apache-2.0 (see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE)), without any additional terms or
conditions.
