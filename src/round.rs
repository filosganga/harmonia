//! Expert iteration (docs/plans/placing-network.md, stage 4): the placer and the placing network
//! teach each other.
//!
//! The network gives the placer good starts; from them the placer finds layouts better than the
//! network's own; those become the network's new examples. So a better network gives better
//! starts, which give better layouts, which train a better network. It's how the result can beat
//! the plain placer instead of only copying it (the same idea as AlphaZero's: the search teaches
//! the network, and the network guides the search).
//!
//! One `round`:
//! 1. Re-place every circuit in out/data/layouts.jsonl twice, with new random numbers each round:
//!    starting from the current network (`place --start net`), and from a random start, as the
//!    plain placer does. The cheapest of the layout there and the two new ones is kept: the
//!    examples only ever get better. The random start is the guard: with only the network's start,
//!    the examples drift towards the network's own habits (round 1 crammed the LED chaser into one
//!    corner), and the next network learns them even more.
//! 2. Train the network again on them, from scratch, as `learn` does, and keep a copy of it as
//!    out/placer-round-<n>.bpk.
//! 3. Measure it with `bench --start net`, and add the round's numbers to out/rounds.json.

use crate::{bench, board, data, parallel, placing};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::time::Instant;

const ROUNDS: &str = "out/rounds.json";
/// The placer's random numbers for circuit n in round r come from seed n + r * this, so every
/// round tries something new (and seed + RANDOM_START for the run from a random start).
const ROUND_SEED: u64 = 1 << 40;
const RANDOM_START: u64 = 1 << 39;

/// What one round did, as saved in out/rounds.json.
#[derive(Serialize, Deserialize)]
struct Round {
    round: usize,
    /// How many circuits got a cheaper layout, of how many; and how many of those cheaper layouts
    /// came from the network's start and from a random start (rounds before the guard: none
    /// recorded).
    improved: usize,
    circuits: usize,
    #[serde(default)]
    from_network: usize,
    #[serde(default)]
    from_random: usize,
    /// The examples' cost after the round against before, as one number (geometric mean of
    /// after / before, minus 1): how much better the network's teacher got.
    examples_change: f64,
    /// The network's top-1 and top-5 (0 to 1) on the circuits kept aside and on the benchmark.
    held_out: [f64; 2],
    benchmark: [f64; 2],
    /// `bench --start net` against the plain placer: the change in mean cost over the 12
    /// circuits (geometric mean, minus 1), and each circuit's mean cost.
    bench_change: f64,
    bench_costs: Vec<(String, f64)>,
    seconds: f64,
}

/// Runs one round of expert iteration and says how it went, with the rounds before it.
pub fn run() {
    let t0 = Instant::now();
    let mut rounds: Vec<Round> = std::fs::read_to_string(ROUNDS)
        .map(|text| serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad {ROUNDS}: {e}")))
        .unwrap_or_default();
    let round = rounds.len() + 1;
    let plain = bench::saved().unwrap_or_else(|why| panic!("{why}"));
    let net = placing::load();

    // 1. Re-place every circuit from the network and from a random start, keeping the cheapest.
    let mut records = data::read();
    if records.is_empty() {
        panic!("no training data yet: run `cargo run --release -- data` first");
    }
    println!(
        "Round {round}: placing {} circuits again, from the network and from a random start, {} at a time...",
        records.len(),
        parallel::workers()
    );
    let mut jobs: Vec<usize> = (0..records.len()).collect();
    jobs.sort_by_key(|&i| std::cmp::Reverse(records[i].parts.len())); // the slowest first
    let placed = parallel::map(&jobs, |&i| {
        let r = &records[i];
        let cases = data::cases(&r.parts, &r.nets, &r.board_file);
        let seed = r.index + round as u64 * ROUND_SEED;
        let place = |from: &crate::StartFrom, seed: u64| {
            let mut rng = StdRng::seed_from_u64(seed);
            let (k, _, layout) = crate::place_cases(&cases, board::score, crate::DEFAULT_STEPS, &mut rng, from, false, |_, _| {});
            let cost = board::cost(&board::measure(&cases[k], &layout));
            (k, layout, cost)
        };
        let network = place(&crate::StartFrom::Network(&net), seed);
        let random = place(&crate::StartFrom::Random, seed + RANDOM_START);
        // The cheaper of the two, and whether it came from the network's start.
        if network.2 <= random.2 { (network, true) } else { (random, false) }
    });
    let (mut improved, mut from_network, mut logs) = (0, 0, 0.0);
    for (&i, ((case, layout, cost), network)) in jobs.iter().zip(placed) {
        let r = &mut records[i];
        if cost < r.cost {
            logs += (cost / r.cost).ln();
            improved += 1;
            from_network += network as usize;
            r.case = case;
            r.layout = layout.iter().map(|p| [p.x, p.y, p.rot as i32]).collect();
            r.cost = cost;
        }
    }
    let examples_change = (logs / records.len() as f64).exp() - 1.0;
    data::write_all(&records);
    println!(
        "{improved} of {} circuits got a cheaper layout ({from_network} from the network's start, {} from a random one); \
         the examples' cost changed by {:+.1}% ({:.0}s).\n",
        records.len(),
        improved - from_network,
        100.0 * examples_change,
        t0.elapsed().as_secs_f64()
    );

    // 2. Train again on the better examples, and keep this round's network.
    let tested = placing::learn();
    let copy = format!("out/placer-round-{round}.bpk");
    std::fs::copy(placing::MODEL, &copy).unwrap_or_else(|e| panic!("can't copy {} to {copy}: {e}", placing::MODEL));

    // 3. Measure it.
    println!();
    let saved = bench::run(true);
    let bench_change = bench::change(&plain.results, &saved.results);
    rounds.push(Round {
        round,
        improved,
        circuits: records.len(),
        from_network,
        from_random: improved - from_network,
        examples_change,
        held_out: tested.held_out,
        benchmark: tested.benchmark,
        bench_change,
        bench_costs: saved.results.iter().map(|r| (r.circuit.clone(), r.mean_cost())).collect(),
        seconds: t0.elapsed().as_secs_f64(),
    });
    std::fs::write(ROUNDS, serde_json::to_string_pretty(&rounds).unwrap()).unwrap();

    println!("\nRounds so far (bench: mean cost starting from the network, against the plain placer):");
    println!("  round  improved (network+random)  examples  held-out top-1/top-5  benchmark top-1/top-5     bench");
    for r in &rounds {
        // Rounds before the guard placed from the network only and didn't count where layouts came from.
        let split = if r.from_network + r.from_random == r.improved { format!("{:>4}+{:<4}", r.from_network, r.from_random) } else { "no guard ".to_string() };
        println!(
            "  {:>5}  {:>4}/{:<4} ({split})     {:>+7.1}%  {:>9.1}% / {:>4.1}%  {:>10.1}% / {:>4.1}%  {:>+8.1}%",
            r.round,
            r.improved,
            r.circuits,
            100.0 * r.examples_change,
            100.0 * r.held_out[0],
            100.0 * r.held_out[1],
            100.0 * r.benchmark[0],
            100.0 * r.benchmark[1],
            100.0 * r.bench_change
        );
    }
    println!("\nSaved to {ROUNDS}; this round took {:.0} minutes.", t0.elapsed().as_secs_f64() / 60.0);
}
