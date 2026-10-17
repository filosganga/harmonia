//! The benchmark: the yardstick every later change to the placer is measured with.
//!
//! It places each circuit listed in `circuits/bench.txt` from a random start, once per seed, the
//! way `place` does (every case of its board file, keeping the best), and records the best cost, the
//! time, and each circuit's best layout (the right answers `learn` tests the placing network on). Its settings are constants, so every run measures the same thing.
//!
//! The runs don't depend on each other, so several go at once, one per processor core. Each run has
//! its own random numbers, made from its seed, so the costs are the same as running them one by one.

use crate::board::{self, Circuit};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// The benchmark set: one circuit per line, from the repo's root.
const LIST: &str = "circuits/bench.txt";
const SEEDS: [u64; 3] = [1, 2, 3];
const STEPS: usize = crate::DEFAULT_STEPS;
const RESULTS: &str = "out/bench.json";

/// What `bench` found for one circuit, as saved in out/bench.json.
#[derive(Serialize, Deserialize)]
pub struct Found {
    pub circuit: String,
    parts: usize,
    nets: usize,
    /// The best cost each seed reached, in the order of `SEEDS`.
    costs: Vec<f64>,
    /// How long each seed took, in seconds (wall clock, while other runs shared the machine).
    seconds: Vec<f64>,
    /// The best layout of all seeds, [x, y, rot] per part, and which case of the board file it's
    /// for: the right answers when testing the placing network (`learn`).
    pub best_case: usize,
    pub best_layout: Vec<[i32; 3]>,
}

#[derive(Serialize, Deserialize)]
pub struct Saved {
    steps: usize,
    seeds: Vec<u64>,
    pub results: Vec<Found>,
}

/// Places every circuit of the benchmark with every seed, prints a table and saves out/bench.json.
pub fn run() {
    let t0 = Instant::now();
    let list = std::fs::read_to_string(LIST).unwrap_or_else(|e| panic!("can't read {LIST}: {e}"));
    let paths: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let circuits: Vec<Vec<Circuit>> = paths.iter().map(|path| load(path)).collect();

    // One job per (circuit, seed). The slowest first (many parts, several cases), so the machine
    // isn't left waiting on one long run at the end.
    let mut jobs: Vec<(usize, usize)> = (0..circuits.len()).flat_map(|c| (0..SEEDS.len()).map(move |s| (c, s))).collect();
    jobs.sort_by_key(|&(c, _)| std::cmp::Reverse(circuits[c].len() * circuits[c][0].components.len()));
    let workers = crate::parallel::workers().min(jobs.len());
    println!(
        "Benchmark: {} circuits x {} seeds, {STEPS} steps each, from a random start ({workers} runs at a time)\n",
        circuits.len(),
        SEEDS.len()
    );

    let done = crate::parallel::map(&jobs, |&(c, s)| {
        let started = Instant::now();
        let (cost, case, layout) = place(&circuits[c], SEEDS[s]);
        (cost, case, layout, started.elapsed().as_secs_f64())
    });

    let mut results: Vec<Found> = paths
        .iter()
        .zip(&circuits)
        .map(|(path, cases)| Found {
            circuit: path.to_string(),
            parts: cases[0].components.len(),
            nets: cases[0].nets.len(),
            costs: vec![0.0; SEEDS.len()],
            seconds: vec![0.0; SEEDS.len()],
            best_case: 0,
            best_layout: Vec::new(),
        })
        .collect();
    let mut layouts = vec![vec![(0, Vec::new()); SEEDS.len()]; circuits.len()];
    for (&(c, s), (cost, case, layout, seconds)) in jobs.iter().zip(done) {
        results[c].costs[s] = cost;
        results[c].seconds[s] = seconds;
        layouts[c][s] = (case, layout);
    }
    for (r, layouts) in results.iter_mut().zip(layouts) {
        // The first seed with the lowest cost.
        let best = (0..SEEDS.len()).min_by(|&a, &b| r.costs[a].total_cmp(&r.costs[b])).unwrap();
        let (case, layout) = &layouts[best];
        r.best_case = *case;
        r.best_layout = layout.iter().map(|p| [p.x, p.y, p.rot as i32]).collect();
    }
    print_table(&results);
    let saved = Saved { steps: STEPS, seeds: SEEDS.to_vec(), results };
    std::fs::write(RESULTS, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
    println!(
        "\nCosts compare only within a circuit. Times are per run, with up to {workers} runs sharing the machine,\n\
         so a run alone is quicker. Saved to {RESULTS} in {:.1}s.",
        t0.elapsed().as_secs_f64()
    );
}

/// The circuit once for each case of its board file's rules, read as `place` reads it, ignoring
/// any flags: the benchmark's settings don't change.
pub fn load(path: &str) -> Vec<Circuit> {
    let args = crate::Args {
        command: "bench".into(),
        circuit: path.into(),
        judge: "rules".into(),
        steps: STEPS,
        seed: 1,
        board: None,
        power: None,
        ground: None,
    };
    crate::load(path, &args).cases
}

/// Places every case from a random start, as `place --seed <seed>` does, and returns the best
/// layout's cost, its case and the layout. A DIYLC sketch starts from random too, not from where it
/// was drawn: so will the network.
fn place(cases: &[Circuit], seed: u64) -> (f64, usize, board::Placement) {
    let mut rng = StdRng::seed_from_u64(seed);
    let (k, _, layout) = crate::place_cases(cases, board::score, STEPS, &mut rng, None, false);
    (board::cost(&board::measure(&cases[k], &layout)), k, layout)
}

/// What the last `bench` saved (out/bench.json), or why it can't be read.
pub fn saved() -> Result<Saved, String> {
    let text = std::fs::read_to_string(RESULTS).map_err(|e| format!("can't read {RESULTS} ({e}): run `bench` first"))?;
    serde_json::from_str(&text).map_err(|e| format!("{RESULTS} is from an older `bench` ({e}): run `bench` again"))
}

fn print_table(results: &[Found]) {
    let seeds: String = SEEDS.iter().map(|s| format!("{:>9}", format!("seed {s}"))).collect();
    println!("{:<38} {:>5} {:>4}{seeds} {:>8} {:>8} {:>8}", "circuit", "parts", "nets", "best", "mean", "s/run");
    for r in results {
        let costs: String = r.costs.iter().map(|c| format!("{c:>9.1}")).collect();
        let best = r.costs.iter().copied().fold(f64::INFINITY, f64::min);
        let mean = r.costs.iter().sum::<f64>() / r.costs.len() as f64;
        let time = r.seconds.iter().sum::<f64>() / r.seconds.len() as f64;
        println!("{:<38} {:>5} {:>4}{costs} {best:>8.1} {mean:>8.1} {time:>8.1}", r.circuit, r.parts, r.nets);
    }
}
