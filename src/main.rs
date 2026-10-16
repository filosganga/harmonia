//! Perfboard layout + a tiny neural network.
//!
//!     cargo run --release -- place                # auto-place using the hand-written rules
//!     cargo run --release -- train                # teach the network to score boards like the rules do
//!     cargo run --release -- evolve               # the same job as train, done by evolution (1 parent + 19 children)
//!     cargo run --release -- place --judge net    # auto-place again, with the network as the judge
//!     cargo run --release -- place my_sketch.diy  # read a rough DIYLC layout, score it, and fit it on the board
//!     cargo run --release -- nets my_sketch.diy   # just show the parts and connections the program understood

mod board;
mod constraints;
mod diylc;
mod diylc_import;
mod kicad_pcb;
mod netlist;
mod nn;
mod place;
mod route;
mod sexpr;
mod svg;

use board::{BoardSize, Circuit, Measures};
use nn::Net;
use svg::Side;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::time::Instant;

const MODEL: &str = "out/model.json";

struct Args {
    command: String,
    circuit: String,
    judge: String,
    steps: usize,
    seed: u64,
    board: Option<BoardSize>,
    power: Option<String>,
    ground: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        command: "place".into(),
        circuit: "circuits/555_blinker.net".into(),
        judge: "rules".into(),
        steps: 20_000,
        seed: 1,
        board: None,
        power: None,
        ground: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--judge" => args.judge = it.next().expect("--judge rules|net"),
            "--steps" => args.steps = it.next().and_then(|s| s.parse().ok()).expect("--steps <number>"),
            "--seed" => args.seed = it.next().and_then(|s| s.parse().ok()).expect("--seed <number>"),
            "--board" => args.board = Some(it.next().and_then(|s| parse_size(&s)).expect("--board <width>x<height>, like 18x12")),
            "--power" => args.power = Some(it.next().expect("--power <net name>")),
            "--ground" => args.ground = Some(it.next().expect("--ground <net name>")),
            "place" | "train" | "evolve" | "nets" => args.command = a,
            path => args.circuit = path.into(),
        }
    }
    args
}

/// "18x12" -> 18 holes wide, 12 high
fn parse_size(s: &str) -> Option<BoardSize> {
    let (w, h) = s.split_once('x')?;
    Some(BoardSize { width: w.parse().ok()?, height: h.parse().ok()? })
}

fn main() {
    let args = parse_args();
    // The board file next to the netlist, if any. Flags win over it, so you can try a variation quickly.
    let (board_file_path, board_file) = constraints::read_board_file(&args.circuit).unzip();
    let board_file = board_file.unwrap_or_default();
    let from_file = board_file.board.as_ref().map(|s| parse_size(s).unwrap_or_else(|| panic!("board file: \"board\" should look like \"18x12\"")));
    let opts = netlist::Options {
        board: args.board.or(from_file).unwrap_or(BoardSize { width: 18, height: 12 }),
        power: args.power.clone().or(board_file.power.clone()),
        ground: args.ground.clone().or(board_file.ground.clone()),
    };
    // A DIYLC file is a sketch: its connections, plus where you put things as a starting point.
    let (mut circuit, sketch) = if args.circuit.ends_with(".diy") {
        let (c, s) = diylc_import::load(&args.circuit, opts);
        (c, Some(s))
    } else {
        (netlist::load(&args.circuit, opts), None)
    };
    circuit.constraints = constraints::resolve(&board_file, &circuit);
    circuit.runs = constraints::runs(&circuit); // again, now that the star rules are known
    if let Some(path) = board_file_path {
        println!("Board file: {path}\n");
    }
    std::fs::create_dir_all("out").unwrap();
    match args.command.as_str() {
        "train" => train(&circuit, args.seed),
        "evolve" => evolve(&circuit, args.seed),
        "nets" => print_nets(&circuit),
        _ => place_cmd(&circuit, &args, sketch),
    }
}

/// What the program understood from the file: the parts and which pins connect.
fn print_nets(c: &Circuit) {
    println!("{} parts:", c.components.len());
    for comp in &c.components {
        let kind = if comp.body.len() < (comp.w * comp.h) as usize { "  (module: parts can go under it)" } else { "" };
        println!("  {:<12} {:>2}x{:<2} holes, {} pins{kind}", comp.reference, comp.w, comp.h, comp.pins.len());
    }
    let or_none = |n: &Option<String>| n.clone().unwrap_or_else(|| "none".into());
    println!("\n{} nets (power: {}, ground: {}):", c.nets.len(), or_none(&c.power), or_none(&c.ground));
    for net in &c.nets {
        println!("  {:<10} {}", net.name, net.pins.join("  "));
    }
}

fn place_cmd(c: &Circuit, args: &Args, sketch: Option<board::Placement>) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(args.seed);
    let net = (args.judge == "net").then(|| Net::load(MODEL));
    let judge = |m: &Measures| match &net {
        Some(net) => net.predict(m),
        None => board::score(m),
    };

    let start_label = if sketch.is_some() { "Your sketch" } else { "Random start" };
    let mut start = sketch.unwrap_or_else(|| board::random_placement(c, &mut rng));
    constraints::apply(c, &mut start);
    println!("{}", board::report(c, &start, start_label));
    println!("{}\n", board::ascii_board(c, &start));

    let best = place::anneal(c, judge, args.steps, &mut rng, Some(start.clone()), |_| {});
    let label = format!("Placed by {}", args.judge);
    println!("{}", board::report(c, &best, &label));
    if let Some(net) = &net {
        let m = board::measure(c, &best);
        println!("  (the network thought it would score {:.0}; the rules say {:.0})", net.predict(&m), board::score(&m));
    }
    println!("{}", board::ascii_board(c, &best));
    for problem in constraints::problems(c, &best) {
        println!("  !! {problem}");
    }

    let path = format!("out/{}.html", args.judge);
    let html = format!(
        "<!doctype html><meta charset='utf-8'><title>Perfboard layout</title>\
         <body style='background:#eee;display:flex;gap:16px;flex-wrap:wrap;padding:16px'>{}{}</body>",
        svg::draw(c, &start, start_label, Side::Component),
        svg::draw(c, &best, &label, Side::Component)
    );
    std::fs::write(&path, html).unwrap();
    let picture = format!("out/{}.svg", args.judge);
    std::fs::write(&picture, svg::draw(c, &best, &label, Side::Component)).unwrap();
    let solder = format!("out/{}-solder.svg", args.judge);
    std::fs::write(&solder, svg::draw(c, &best, &label, Side::Solder)).unwrap();
    let kicad = format!("out/{}.kicad_pcb", args.judge);
    std::fs::write(&kicad, kicad_pcb::write(c, &best)).unwrap();
    let diy = format!("out/{}.diy", args.judge);
    std::fs::write(&diy, diylc::write(c, &best, &label)).unwrap();
    println!("\nPictures: {picture}, {solder} (turned over to solder), {path} (before and after)");
    println!("For KiCad: {kicad}   For DIYLC: {diy}   ({:.2}s)", t0.elapsed().as_secs_f64());
}

/// Example boards for training: (to learn from, kept aside to test).
/// Random boards, plus boards caught part-way through auto-placement so there are good ones too.
fn make_examples(c: &Circuit, rng: &mut StdRng) -> (Vec<Measures>, Vec<Measures>) {
    let mut examples: Vec<Measures> = Vec::new();
    for _ in 0..12 {
        let mut snap_rng = StdRng::seed_from_u64(rng.random());
        place::anneal(c, board::score, 4000, rng, None, |p| {
            if snap_rng.random::<f64>() < 0.02 {
                examples.push(board::measure(c, p));
            }
        });
    }
    for _ in 0..examples.len() / 4 {
        examples.push(board::measure(c, &board::random_placement(c, rng)));
    }
    examples.shuffle(rng);
    let test = examples.split_off(examples.len() * 4 / 5);
    println!("Made {} example boards ({} to learn from, {} kept aside to test)\n", examples.len() + test.len(), examples.len(), test.len());
    (examples, test)
}

/// The network's inputs and the right answer (the rules' score, scaled to 0..1) for each board.
fn pairs(boards: &[Measures]) -> Vec<(Vec<f64>, f64)> {
    boards.iter().map(|m| (nn::inputs(m), board::score(m) / 100.0)).collect()
}

/// Teach the network to score boards the way the rules do, by gradient descent.
///
/// 1. Make a pile of example boards.
/// 2. Ask the rules for each board's score: that's the "right answer".
/// 3. Show the network each board many times, nudging every weight in the direction that makes it less wrong.
/// 4. Test it on boards it has never seen.
fn train(c: &Circuit, seed: u64) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(c, &mut rng);

    let mut net = Net::new(8, 0);
    let mut pairs = pairs(&train);
    for epoch in 1..=200 {
        pairs.shuffle(&mut rng);
        let lr = if epoch < 120 { 0.05 } else { 0.01 };
        let loss = pairs.iter().map(|(x, y)| net.train_step(x, *y, lr)).sum::<f64>() / pairs.len() as f64;
        if [1, 2, 5, 10, 25, 50, 100, 150, 200].contains(&epoch) {
            println!("  round {epoch:3}: typical error {:5.1} points", 100.0 * loss.sqrt());
        }
    }
    finish(&net, &test, t0);
}

/// The same job as `train`, done by evolution instead:
///
/// 1. Start from the same random network (the parent).
/// 2. Make 19 children, each a copy of the parent with every weight changed a little at random.
/// 3. Keep whichever of the 20 is least wrong on the example boards; it becomes the new parent.
/// 4. Repeat. Changes get smaller towards the end, to fine-tune.
fn evolve(c: &Circuit, seed: u64) {
    const CHILDREN: usize = 19;
    const GENERATIONS: usize = 1000;
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(c, &mut rng);
    let pairs = pairs(&train);

    let mut parent = Net::new(8, 0);
    let mut parent_err = parent.error(&pairs);
    let mut improved = 0;
    for generation in 1..=GENERATIONS {
        // From 0.1 down to 0.003: big jumps early to explore, tiny ones at the end to fine-tune.
        let size = 0.1 * 0.03f64.powf(generation as f64 / GENERATIONS as f64);
        let (best, best_err) = (0..CHILDREN)
            .map(|_| parent.mutated(size, &mut rng))
            .map(|child| {
                let err = child.error(&pairs);
                (child, err)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if best_err < parent_err {
            (parent, parent_err) = (best, best_err);
            improved += 1;
        }
        if [1, 2, 5, 10, 25, 50, 100, 250, 500, 750, 1000].contains(&generation) {
            println!("  generation {generation:4}: typical error {:5.1} points", 100.0 * parent_err.sqrt());
        }
    }
    println!("\nTried {} children; in {improved} of {GENERATIONS} generations the best child beat its parent.", GENERATIONS * CHILDREN);
    finish(&parent, &test, t0);
}

/// Test the trained network on boards it never saw, and save it for `place --judge net`.
fn finish(net: &Net, test: &[Measures], t0: Instant) {
    let mean_err = test.iter().map(|m| (net.predict(m) - board::score(m)).abs()).sum::<f64>() / test.len() as f64;
    println!("\nOn {} unseen boards the network is off by {mean_err:.1} points on average.", test.len());
    println!("A few of them:");
    let mut sorted = test.to_vec();
    sorted.sort_by(|a, b| board::score(a).total_cmp(&board::score(b)));
    for m in sorted.iter().step_by((sorted.len() / 6).max(1)) {
        println!("  rules say {:5.1}   network says {:5.1}", board::score(m), net.predict(m));
    }

    net.save(MODEL);
    println!("\nSaved to {MODEL} in {:.2}s. Try: cargo run --release -- place --judge net", t0.elapsed().as_secs_f64());
}
