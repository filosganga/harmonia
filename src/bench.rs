//! The benchmark: the yardstick every later change to the placer is measured with.
//!
//! It places each circuit listed in `circuits/bench.txt` from a random start, once per seed, the
//! way `place` does (every case of its board file, keeping the best), and records the best cost, the
//! time, how the cost fell, and each circuit's best layout (the right answers `learn` tests the
//! placing network on). Its settings are constants, so every run measures the same thing.
//!
//! `bench --start net` does the same starting from the placing network's layout, and compares it
//! with the plain placer's last run.
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
/// Where `bench --start net` saves its results (out/bench.json stays the plain placer's).
const NETWORK_RESULTS: &str = "out/bench-net.json";

/// What `bench` found for one circuit, as saved in out/bench.json (or out/bench-net.json).
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
    /// The cost of each seed's starting layout (random, or the network's).
    #[serde(default)]
    start_costs: Vec<f64>,
    /// How each seed's best cost fell: (step, new lowest cost), the steps counted on through the
    /// board file's cases one after the other.
    #[serde(default)]
    progress: Vec<Vec<(usize, f64)>>,
}

/// One run: the best layout's cost, its case and the layout, how long it took, the start's cost,
/// and how the cost fell.
struct Run {
    cost: f64,
    case: usize,
    layout: board::Placement,
    seconds: f64,
    start_cost: f64,
    progress: Vec<(usize, f64)>,
}

#[derive(Serialize, Deserialize)]
pub struct Saved {
    steps: usize,
    seeds: Vec<u64>,
    pub results: Vec<Found>,
}

/// Places every circuit of the benchmark with every seed, prints a table and saves out/bench.json;
/// or, `from_network`, starts from the placing network's layouts, saves out/bench-net.json and
/// compares with out/bench.json.
pub fn run(from_network: bool) -> Saved {
    let t0 = Instant::now();
    let plain = if from_network { Some(saved().unwrap_or_else(|why| panic!("{why}"))) } else { None };
    // An out/bench.json from before `bench` recorded how the cost falls has nothing to compare with.
    if plain.as_ref().is_some_and(|p| p.results.iter().any(|r| r.progress.is_empty() || r.start_costs.is_empty())) {
        panic!("{RESULTS} is from an older `bench`, without the start costs and progress to compare with: run `bench` again");
    }
    let placer = from_network.then(crate::placing::load);
    let list = std::fs::read_to_string(LIST).unwrap_or_else(|e| panic!("can't read {LIST}: {e}"));
    let paths: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    let circuits: Vec<Vec<Circuit>> = paths.iter().map(|path| load(path)).collect();

    // One job per (circuit, seed). The slowest first (many parts, several cases), so the machine
    // isn't left waiting on one long run at the end.
    let mut jobs: Vec<(usize, usize)> = (0..circuits.len()).flat_map(|c| (0..SEEDS.len()).map(move |s| (c, s))).collect();
    jobs.sort_by_key(|&(c, _)| std::cmp::Reverse(circuits[c].len() * circuits[c][0].components.len()));
    let workers = crate::parallel::workers().min(jobs.len());
    let from = if from_network { "the placing network's layout" } else { "a random start" };
    println!(
        "Benchmark: {} circuits x {} seeds, {STEPS} steps each, from {from} ({workers} runs at a time)\n",
        circuits.len(),
        SEEDS.len()
    );

    let runs = crate::parallel::map(&jobs, |&(c, s)| {
        let from = match &placer {
            Some(net) => crate::StartFrom::Network(net),
            None => crate::StartFrom::Random,
        };
        place(&circuits[c], SEEDS[s], &from)
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
            start_costs: vec![0.0; SEEDS.len()],
            progress: vec![Vec::new(); SEEDS.len()],
        })
        .collect();
    let mut layouts = vec![vec![(0, Vec::new()); SEEDS.len()]; circuits.len()];
    for (&(c, s), run) in jobs.iter().zip(runs) {
        let r = &mut results[c];
        r.costs[s] = run.cost;
        r.seconds[s] = run.seconds;
        r.start_costs[s] = run.start_cost;
        r.progress[s] = run.progress;
        layouts[c][s] = (run.case, run.layout);
    }
    for (r, layouts) in results.iter_mut().zip(layouts) {
        // The first seed with the lowest cost.
        let best = (0..SEEDS.len()).min_by(|&a, &b| r.costs[a].total_cmp(&r.costs[b])).unwrap();
        let (case, layout) = &layouts[best];
        r.best_case = *case;
        r.best_layout = layout.iter().map(|p| [p.x, p.y, p.rot as i32]).collect();
    }
    print_table(&results);
    if let Some(plain) = &plain {
        compare(&plain.results, &results);
    }
    let path = if from_network { NETWORK_RESULTS } else { RESULTS };
    let saved = Saved { steps: STEPS, seeds: SEEDS.to_vec(), results };
    std::fs::write(path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
    println!(
        "\nCosts compare only within a circuit. Times are per run, with up to {workers} runs sharing the machine,\n\
         so a run alone is quicker. Saved to {path} in {:.1}s.",
        t0.elapsed().as_secs_f64()
    );
    saved
}

impl Found {
    /// The mean of the seeds' best costs.
    pub fn mean_cost(&self) -> f64 {
        self.costs.iter().sum::<f64>() / self.costs.len() as f64
    }
}

/// How much lower (negative) or higher the mean costs in `net` are than in `plain`, as one number:
/// the geometric mean over the circuits of net / plain, minus 1. Each circuit counts the same,
/// whatever its costs' size.
pub fn change(plain: &[Found], net: &[Found]) -> f64 {
    let logs: f64 = plain.iter().zip(net).map(|(p, n)| (n.mean_cost() / p.mean_cost()).ln()).sum();
    (logs / net.len() as f64).exp() - 1.0
}

/// Starting from the network (`net`) against the plain placer (`plain`), circuit by circuit: the
/// starting layout's cost, the cost after the same steps, and how many steps each took to reach the
/// plain placer's mean cost.
fn compare(plain: &[Found], net: &[Found]) {
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let best = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    // The first step at which a run's lowest cost got down to `target`, in thousands, or "-" never.
    let steps_to = |progress: &[Vec<(usize, f64)>], target: f64| -> String {
        let one = |p: &Vec<(usize, f64)>| p.iter().find(|&&(_, cost)| cost <= target + 1e-9).map_or("-".to_string(), |&(step, _)| format!("{:.1}k", step as f64 / 1000.0));
        progress.iter().map(one).collect::<Vec<_>>().join(" ")
    };
    println!("\nStarting from the placing network, against the plain placer (out/bench.json):\n");
    println!(
        "{:<38} {:>17} {:>17} {:>17}   steps to the plain placer's mean cost, each seed",
        "", "start cost", "mean after", "best after"
    );
    println!("{:<38} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}   {:<18} network", "circuit", "random", "network", "plain", "network", "plain", "network", "plain");
    let mut better = 0;
    for (p, n) in plain.iter().zip(net) {
        assert_eq!(p.circuit, n.circuit, "out/bench.json is from another benchmark set: run `bench` again");
        let target = mean(&p.costs);
        if mean(&n.costs) < target {
            better += 1;
        }
        println!(
            "{:<38} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}   {:<18} {}",
            n.circuit,
            mean(&p.start_costs),
            mean(&n.start_costs),
            target,
            mean(&n.costs),
            best(&p.costs),
            best(&n.costs),
            steps_to(&p.progress, target),
            steps_to(&n.progress, target)
        );
    }
    println!("\nStarting from the network, the mean cost after {STEPS} steps is lower on {better} of {} circuits.", net.len());
}

/// The circuit once for each case of its board file's rules, read as `place` reads it, ignoring
/// any flags: the benchmark's settings don't change.
pub fn load(path: &str) -> Vec<Circuit> {
    let args = crate::Args {
        command: "bench".into(),
        circuit: path.into(),
        judge: "rules".into(),
        start: String::new(),
        steps: STEPS,
        seed: 1,
        board: None,
        power: None,
        ground: None,
    };
    crate::load(path, &args).cases
}

/// Places every case as `place --seed <seed>` does (with `--start net` if `from` is the network).
/// A DIYLC sketch starts from random too, not from where it was drawn.
fn place(cases: &[Circuit], seed: u64, from: &crate::StartFrom) -> Run {
    let started = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let mut progress: Vec<(usize, f64)> = Vec::new();
    let (case, start, layout) = crate::place_cases(cases, board::score, STEPS, &mut rng, from, false, |k, step| {
        if progress.last().is_none_or(|&(_, lowest)| step.best_cost < lowest) {
            progress.push((k * STEPS + step.step, step.best_cost));
        }
    });
    let cost_of = |p: &board::Placement| board::cost(&board::measure(&cases[case], p));
    Run { cost: cost_of(&layout), case, start_cost: cost_of(&start), layout, seconds: started.elapsed().as_secs_f64(), progress }
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
