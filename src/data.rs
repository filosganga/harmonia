//! Training data for the placing network (docs/plans/placing-network.md, stage 1).
//!
//! `data` makes a batch of random circuits (src/random_circuit.rs), places each the way `bench`
//! does (from a random start, `place`'s default steps), and appends it with the placer's best
//! layout to out/data/layouts.jsonl, one JSON object per line. Each run carries on from the
//! circuits already there, so circuit number n is always the same circuit. `round` later replaces
//! layouts with cheaper ones found from the placing network's start.
//!
//! A layout gives one training example per part, "with these parts down, this one went here",
//! taking the parts in `placing_order`.

use crate::board::{self, Circuit, Net};
use crate::netlist::{self, Part};
use crate::{constraints, parallel, random_circuit};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::fmt::Write as _;
use std::io::Write as _;
use std::time::Instant;

const DIR: &str = "out/data";
const LAYOUTS: &str = "out/data/layouts.jsonl";
/// Circuits added per run: about 90s on an 18-core Mac (circuits with modules need bigger boards).
const BATCH: u64 = 100;
const STEPS: usize = crate::DEFAULT_STEPS;
/// The placer's random numbers for circuit n come from seed n + this, so they're not the same
/// numbers that made the circuit.
const PLACER_SEED: u64 = 1 << 32;

/// One placed circuit: a line of out/data/layouts.jsonl.
#[derive(Serialize, Deserialize)]
pub struct Record {
    /// Which random circuit it is: `random_circuit::make(index)`.
    pub index: u64,
    pub parts: Vec<Part>,
    pub nets: Vec<Net>,
    /// Its board file, as it would be in circuits/<name>.json.
    pub board_file: serde_json::Value,
    /// Which case of the board file's rules the best layout is for.
    pub case: usize,
    /// Where the placer put each part, [x, y, rot], in the order of `parts`.
    pub layout: Vec<[i32; 3]>,
    pub cost: f64,
    pub steps: usize,
}

/// Adds a batch of placed random circuits to out/data/layouts.jsonl and says what's there now.
pub fn run() {
    let t0 = Instant::now();
    std::fs::create_dir_all(DIR).unwrap();
    let start = read().len() as u64;
    let indices: Vec<u64> = (start..start + BATCH).collect();
    let specs: Vec<random_circuit::Spec> = indices.iter().map(|&n| random_circuit::make(n)).collect();
    let circuits: Vec<Vec<Circuit>> = specs.iter().map(|s| cases(&s.parts, &s.nets, &s.board_file)).collect();
    println!(
        "Placing random circuits #{start} to #{}, {STEPS} steps each, {} at a time...",
        start + BATCH - 1,
        parallel::workers()
    );

    // The biggest first, so the machine isn't left waiting on one long run at the end.
    let mut jobs: Vec<usize> = (0..specs.len()).collect();
    jobs.sort_by_key(|&i| Reverse(circuits[i][0].components.len()));
    let placed = parallel::map(&jobs, |&i| {
        let mut rng = StdRng::seed_from_u64(indices[i] + PLACER_SEED);
        let (k, _, layout) = crate::place_cases(&circuits[i], board::score, STEPS, &mut rng, &crate::StartFrom::Random, false, |_, _| {});
        let cost = board::cost(&board::measure(&circuits[i][k], &layout));
        (k, layout, cost)
    });
    let mut records: Vec<(usize, Record)> = jobs
        .iter()
        .zip(placed)
        .map(|(&i, (case, layout, cost))| {
            let s = &specs[i];
            let layout = layout.iter().map(|p| [p.x, p.y, p.rot as i32]).collect();
            (i, Record { index: indices[i], parts: s.parts.clone(), nets: s.nets.clone(), board_file: s.board_file.clone(), case, layout, cost, steps: STEPS })
        })
        .collect();
    records.sort_by_key(|(i, _)| *i);

    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(LAYOUTS).unwrap();
    for (_, r) in &records {
        writeln!(file, "{}", serde_json::to_string(r).unwrap()).unwrap();
    }

    // The batch's first circuit as a netlist and board file, to look at with `place`.
    let first = &records[0].1;
    std::fs::write(format!("{DIR}/sample.net"), kicad_netlist(&first.parts, &first.nets)).unwrap();
    std::fs::write(format!("{DIR}/sample.json"), serde_json::to_string_pretty(&first.board_file).unwrap() + "\n").unwrap();
    let c = &circuits[records[0].0][first.case];
    let order: Vec<&str> = placing_order(c).iter().map(|&i| c.components[i].reference.as_str()).collect();

    println!("Added {BATCH} circuits to {LAYOUTS} in {:.0}s.\n", t0.elapsed().as_secs_f64());
    summary(&read());
    println!("\nCircuit #{start} is in {DIR}/sample.net, with its board file: cargo run --release -- place {DIR}/sample.net");
    println!("Its placing order: {}", order.join(" "));
}

/// Replaces out/data/layouts.jsonl with `records` (written to a new file first, so a stop halfway
/// leaves the old one).
pub fn write_all(records: &[Record]) {
    let new = format!("{LAYOUTS}.new");
    let lines: String = records.iter().map(|r| serde_json::to_string(r).unwrap() + "\n").collect();
    std::fs::write(&new, lines).unwrap();
    std::fs::rename(&new, LAYOUTS).unwrap();
}

/// Every record in out/data/layouts.jsonl (none if it isn't there yet).
pub fn read() -> Vec<Record> {
    let text = std::fs::read_to_string(LAYOUTS).unwrap_or_default();
    text.lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line in {LAYOUTS}: {e}"))).collect()
}

/// The circuit once for each case of its board file's rules, as `place` would read it from a
/// netlist and board file.
pub fn cases(parts: &[Part], nets: &[Net], board_file: &serde_json::Value) -> Vec<Circuit> {
    let file: constraints::BoardFile = serde_json::from_value(board_file.clone()).unwrap_or_else(|e| panic!("bad board file: {e}"));
    let board = file.board.as_deref().and_then(crate::parse_size).expect("a random circuit's board file has a board size");
    let opts = netlist::Options { board, power: file.power.clone(), ground: file.ground.clone() };
    let circuit = netlist::from_parts(parts, nets.to_vec(), opts);
    crate::cases_of(&circuit, &file).0
}

/// The order the network places parts in, the same every time: parts with placement rules first
/// (they have the least choice), then the ones with the most connected pins, then the bigger ones,
/// then by name.
pub fn placing_order(c: &Circuit) -> Vec<usize> {
    let mut connected = vec![0; c.components.len()];
    for pins in &c.net_pins {
        for &(part, _) in pins {
            connected[part] += 1;
        }
    }
    let mut order: Vec<usize> = (0..c.components.len()).collect();
    order.sort_by_key(|&i| {
        let comp = &c.components[i];
        (c.constraints.allowed[i].is_none(), Reverse(connected[i]), Reverse(comp.w * comp.h), comp.reference.clone())
    });
    order
}

/// How many circuits and examples there are, and what the circuits are like.
fn summary(records: &[Record]) {
    let examples: usize = records.iter().map(|r| r.parts.len()).sum();
    println!("{} circuits, {examples} training examples (one per part).", records.len());
    let histogram = |what: &str, values: Vec<usize>, width: usize| {
        let (lo, hi) = (*values.iter().min().unwrap(), *values.iter().max().unwrap());
        println!("\n{what}:");
        for from in (lo / width * width..=hi).step_by(width) {
            let n = values.iter().filter(|&&v| v >= from && v < from + width).count();
            let bar = "#".repeat((60 * n).div_ceil(values.len()));
            println!("  {from:>3}-{:<3} {n:>5} {bar}", from + width - 1);
        }
    };
    histogram("Parts per circuit", records.iter().map(|r| r.parts.len()).collect(), 3);
    histogram("Nets per circuit (with two pins or more)", records.iter().map(|r| r.nets.iter().filter(|n| n.pins.len() > 1).count()).collect(), 3);
    let holes: Vec<usize> = records.iter().map(|r| board_holes(&r.board_file)).collect();
    println!("\nBoards from {} to {} holes.", holes.iter().min().unwrap(), holes.iter().max().unwrap());
}

fn board_holes(board_file: &serde_json::Value) -> usize {
    let size = board_file["board"].as_str().and_then(crate::parse_size).unwrap();
    (size.width * size.height) as usize
}

/// The parts and nets as a KiCad netlist, which `place` reads like one exported from KiCad.
fn kicad_netlist(parts: &[Part], nets: &[Net]) -> String {
    let mut s = String::from("(export (version \"E\")\n  (design (source \"random circuit\") (tool \"harmonia data\"))\n  (components\n");
    for p in parts {
        writeln!(s, "    (comp (ref \"{}\") (value \"{}\") (footprint \"{}\"))", p.reference, p.value, p.footprint).unwrap();
    }
    s += "  )\n  (nets\n";
    for (k, net) in nets.iter().enumerate() {
        let nodes: Vec<String> = net
            .pins
            .iter()
            .map(|pin| {
                let (part, pad) = pin.split_once('.').unwrap();
                format!("(node (ref \"{part}\") (pin \"{pad}\"))")
            })
            .collect();
        writeln!(s, "    (net (code \"{}\") (name \"{}\") {})", k + 1, net.name, nodes.join(" ")).unwrap();
    }
    s += "  ))\n";
    s
}
