//! Perfboard layout + a tiny neural network.
//!
//!     cargo run --release -- place                # auto-place using the hand-written rules
//!     cargo run --release -- train                # teach the network to score boards like the rules do
//!     cargo run --release -- evolve               # the same job as train, done by evolution (1 parent + 19 children)
//!     cargo run --release -- place --judge net    # auto-place again, with the network as the judge
//!     cargo run --release -- place --start net    # start the placer from the placing network's layout
//!     cargo run --release -- place my_sketch.diy  # read a rough DIYLC layout, score it, and fit it on the board
//!     cargo run --release -- nets my_sketch.diy   # just show the parts and connections the program understood
//!     cargo run --release -- check sketches/      # place, write and re-read every .diy: same connections?
//!     cargo run --release -- draw out/rules.layout.json  # draw a saved layout again, without placing it
//!     cargo run --release -- bench                # the benchmark: place a fixed set of circuits, record the costs
//!     cargo run --release -- data                 # add a batch of placed random circuits to learn from
//!     cargo run --release -- learn                # train the placing network on them, test it on unseen circuits
//!     cargo run --release -- round                # one round of expert iteration: re-place with the network, keep the better, retrain

mod bench;
mod board;
mod constraints;
mod data;
mod diylc;
mod diylc_import;
mod kicad_pcb;
mod layout;
mod netlist;
mod nn;
mod parallel;
mod place;
mod placing;
mod random_circuit;
mod round;
mod route;
mod sexpr;
mod svg;

use board::{BoardSize, Circuit, Measures};
use layout::{Layout, Made};
use nn::Net;
use svg::Side;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::time::Instant;

const MODEL: &str = "out/model.json";
/// How hard the placer tries, unless `--steps` says otherwise. `bench` always uses it.
const DEFAULT_STEPS: usize = 20_000;

struct Args {
    command: String,
    circuit: String,
    judge: String,
    /// Where the placer starts: "" (at random, or from your sketch) or "net" (the placing network).
    start: String,
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
        start: String::new(),
        steps: DEFAULT_STEPS,
        seed: 1,
        board: None,
        power: None,
        ground: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--judge" => args.judge = it.next().expect("--judge rules|net"),
            "--start" => args.start = it.next().filter(|s| s == "net").expect("--start net"),
            "--steps" => args.steps = it.next().and_then(|s| s.parse().ok()).expect("--steps <number>"),
            "--seed" => args.seed = it.next().and_then(|s| s.parse().ok()).expect("--seed <number>"),
            "--board" => args.board = Some(it.next().and_then(|s| parse_size(&s)).expect("--board <width>x<height>, like 18x12")),
            "--power" => args.power = Some(it.next().expect("--power <net name>")),
            "--ground" => args.ground = Some(it.next().expect("--ground <net name>")),
            "place" | "train" | "evolve" | "nets" | "check" | "draw" | "bench" | "data" | "learn" | "round" => args.command = a,
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
    std::fs::create_dir_all("out").unwrap();
    match args.command.as_str() {
        "check" => return check_cmd(&args),
        "draw" => return draw_cmd(&args.circuit),
        "bench" => {
            bench::run(args.start == "net");
            return;
        }
        "data" => return data::run(),
        "learn" => {
            placing::learn();
            return;
        }
        "round" => return round::run(),
        _ => {}
    }
    let loaded = load(&args.circuit, &args);
    if let Some(path) = &loaded.board_file {
        println!("Board file: {path}\n");
    }
    for why in &loaded.left_out {
        println!("Left out: {why}");
    }
    let cases = &loaded.cases;
    match args.command.as_str() {
        "train" => train(cases, args.seed),
        "evolve" => evolve(cases, args.seed),
        "nets" => print_nets(cases),
        _ => place_cmd(cases, &args, loaded.sketch, loaded.board_file),
    }
}

/// A circuit and the board file next to it, as `load` reads them.
struct Loaded {
    /// The circuit once for each case of the board file's rules (one, unless a one-of makes more).
    cases: Vec<Circuit>,
    /// Cases left out because some part has no place meeting their rules, and why.
    left_out: Vec<String>,
    /// For a DIYLC sketch, where you put things: a starting point.
    sketch: Option<board::Placement>,
    /// The board file's path, if there is one.
    board_file: Option<String>,
}

/// Reads a circuit (a KiCad netlist or a DIYLC sketch) and the board file next to it, if any.
fn load(path: &str, args: &Args) -> Loaded {
    // Flags win over the board file, so you can try a variation quickly.
    let (board_file_path, board_file) = constraints::read_board_file(path).unzip();
    let board_file = board_file.unwrap_or_default();
    let from_file = board_file.board.as_ref().map(|s| parse_size(s).unwrap_or_else(|| panic!("board file: \"board\" should look like \"18x12\"")));
    let opts = netlist::Options {
        board: args.board.or(from_file).unwrap_or(BoardSize { width: 18, height: 12 }),
        power: args.power.clone().or(board_file.power.clone()),
        ground: args.ground.clone().or(board_file.ground.clone()),
    };
    let (circuit, sketch) = if path.ends_with(".diy") {
        let (c, s) = diylc_import::load(path, opts);
        (c, Some(s))
    } else {
        (netlist::load(path, opts), None)
    };
    let (cases, left_out) = cases_of(&circuit, &board_file);
    Loaded { cases, left_out, sketch, board_file: board_file_path }
}

/// The circuit once for each case of the board file's rules that can be met, and why the others
/// can't. Stops if none can.
fn cases_of(circuit: &Circuit, board_file: &constraints::BoardFile) -> (Vec<Circuit>, Vec<String>) {
    let (mut cases, mut left_out) = (Vec::new(), Vec::new());
    for case in constraints::cases(board_file) {
        match constraints::resolve(&case, circuit) {
            Ok(rules) => {
                let mut c = circuit.clone();
                c.constraints = rules;
                c.runs = constraints::runs(&c); // again, now that the star rules are known
                cases.push(c);
            }
            Err(why) if case.choices.is_empty() => left_out.push(why),
            Err(why) => left_out.push(format!("{}: {why}", case.choices.join("; "))),
        }
    }
    if cases.is_empty() {
        panic!("board file: no case of its rules can be met:\n  {}", left_out.join("\n  "));
    }
    (cases, left_out)
}

/// Where `place_cases` starts the placer for each case.
enum StartFrom<'a> {
    Random,
    /// Where you drew things in a DIYLC sketch.
    Sketch(&'a board::Placement),
    /// The placing network's layout (`placing::propose`).
    Network(&'a placing::Net),
}

/// Places every case of the board file's rules, starting from `from`, and keeps the layout the
/// judge likes best. Returns which case it was, where it started, and the layout. With `report`,
/// prints a line for each case when there are several. `on_step` sees every step of the placer,
/// with the case it's placing.
fn place_cases(
    cases: &[Circuit],
    judge: impl Fn(&Measures) -> f64,
    steps: usize,
    rng: &mut StdRng,
    from: &StartFrom,
    report: bool,
    mut on_step: impl FnMut(usize, &place::Step),
) -> (usize, board::Placement, board::Placement) {
    let mut best: Option<(usize, board::Placement, board::Placement, f64)> = None;
    for (k, c) in cases.iter().enumerate() {
        let (mut start, from_network) = match from {
            StartFrom::Random => (board::random_placement(c, rng), false),
            StartFrom::Sketch(sketch) => ((*sketch).clone(), false),
            StartFrom::Network(net) => {
                let (layout, stuck) = placing::propose(c, net, rng);
                if report && stuck > 0 {
                    println!("The placing network found no room for {stuck} part(s); they start at random.");
                }
                (layout, true)
            }
        };
        constraints::apply(c, &mut start);
        let given = if from_network { place::Start::Network(start.clone()) } else { place::Start::Given(start.clone()) };
        let layout = place::anneal(c, &judge, steps, rng, given, |s| on_step(k, s));
        if report && cases.len() > 1 {
            println!("{}", board::report(c, &layout, &format!("Case {} of {} ({})", k + 1, cases.len(), case_name(c))));
        }
        let score = judge(&board::measure(c, &layout));
        if best.as_ref().is_none_or(|b| score > b.3) {
            best = Some((k, start, layout, score));
        }
    }
    let (k, start, layout, _) = best.expect("there's at least one case");
    (k, start, layout)
}

/// Which option of each one-of a case took, like `one-of "Pads by the ESP32's back pads": "by GP5"`.
fn case_name(c: &Circuit) -> String {
    c.constraints.choices.join("; ")
}

/// The connections of a circuit, in an order that doesn't depend on the file: each net with two
/// pins or more, as its sorted pin names, sorted.
fn connections(c: &Circuit) -> Vec<Vec<String>> {
    let mut nets: Vec<Vec<String>> = c.nets.iter().filter(|n| n.pins.len() > 1).map(|n| {
        let mut pins = n.pins.clone();
        pins.sort();
        pins
    }).collect();
    nets.sort();
    nets
}

/// How one file did in `check`.
enum Outcome {
    Same,
    NoConnections,
    /// The reader refuses the file (say, a part it doesn't support): nothing to compare.
    Unreadable(String),
    /// When the placer stopped, this many pins still shared a hole or hung off the board, so their
    /// nets may join for that reason alone.
    Unfinished(usize),
    /// Nets in the file but not in what was written back, and the other way round.
    Different(Vec<Vec<String>>, Vec<Vec<String>>),
    /// The layout file, read back, doesn't give the same layout file or the same drawings: what differs.
    LayoutChanged(String),
    /// Something stopped after the file was read: the placer, the router, the writer, or reading
    /// back what was written.
    Crashed(String),
}

/// Where the last stop (panic) happened, kept by `check`'s panic hook for its report.
static LAST_STOP: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Runs `f`; if it stops, returns its message and where it stopped instead.
fn caught<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        let why = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()));
        format!("{} ({})", why.unwrap_or_default(), LAST_STOP.lock().unwrap())
    })
}

/// The round trip for one DIYLC file: place it as `place` does, write the `.diy` (to
/// out/check.diy), read that back, and compare the connections.
fn check_one(path: &str, args: &Args) -> Outcome {
    let loaded = match caught(|| load(path, args)) {
        Ok(loaded) => loaded,
        Err(why) => return Outcome::Unreadable(why),
    };
    let before = connections(&loaded.cases[0]);
    if before.is_empty() {
        return Outcome::NoConnections;
    }
    let round_trip = || {
        let sketch = loaded.sketch.as_ref().expect("a .diy file has a sketch");
        let mut rng = StdRng::seed_from_u64(args.seed);
        let (k, _, best) = place_cases(&loaded.cases, board::score, args.steps, &mut rng, &StartFrom::Sketch(sketch), true, |_, _| {});
        let c = &loaded.cases[k];
        // check always places by the rules, whatever --judge says.
        let made = made(args, path, loaded.board_file.clone(), "rules", "sketch", &loaded.cases, k);
        let layout = Layout::new(c.clone(), best.clone(), "check", made);
        let written = "out/check.diy";
        let diy = diylc::write(&layout);
        std::fs::write(written, &diy).unwrap();
        // The layout file must keep everything the drawings need: write it, read it back, and the
        // file and the drawings from what was read must be the same as from the layout in memory.
        let saved = layout::to_json(&layout);
        std::fs::write("out/check.layout.json", &saved).unwrap();
        let read = layout::from_json(&saved);
        if let Some(why) = differences(&layout, &read, &saved, &diy) {
            return Outcome::LayoutChanged(why);
        }
        let after = connections(&load(written, args).cases[0]);
        if before == after {
            return Outcome::Same;
        }
        // Pins sharing a hole, or off the board, explain joined nets without any fault in the writer.
        let (w, h) = (c.board.width, c.board.height);
        let mut seen = std::collections::HashSet::new();
        let pins = board::geometry(c, &best).pins;
        let clashing = pins.iter().flatten().filter(|&&(x, y)| x < 0 || x >= w || y < 0 || y >= h || !seen.insert((x, y))).count();
        if clashing > 0 {
            return Outcome::Unfinished(clashing);
        }
        let missing = before.iter().filter(|n| !after.contains(n)).cloned().collect();
        let extra = after.iter().filter(|n| !before.contains(n)).cloned().collect();
        Outcome::Different(missing, extra)
    };
    caught(round_trip).unwrap_or_else(Outcome::Crashed)
}

/// What differs between a layout and the same layout written to a layout file (`saved`) and read
/// back (`read`): the file it writes again, or any of the drawings. None if nothing does.
fn differences(layout: &Layout, read: &Layout, saved: &str, diy: &str) -> Option<String> {
    let checks = [
        ("the layout file written again", saved.to_string(), layout::to_json(read)),
        ("the .diy", diy.to_string(), diylc::write(read)),
        ("the picture", svg::draw(layout, Side::Component), svg::draw(read, Side::Component)),
        ("the solder-side picture", svg::draw(layout, Side::Solder), svg::draw(read, Side::Solder)),
        ("the KiCad board", kicad_pcb::write(layout), kicad_pcb::write(read)),
    ];
    let differ: Vec<&str> = checks.iter().filter(|(_, from_memory, from_file)| from_memory != from_file).map(|(what, ..)| *what).collect();
    (!differ.is_empty()).then(|| format!("read back from out/check.layout.json, {} came out different", differ.join(", ")))
}

/// Places every DIYLC file given (one file, or every .diy in a folder), writes it out the way
/// `place` does, reads that back, and checks it has exactly the connections it started with.
/// Exits with an error if any file comes back wrong or stops the program.
fn check_cmd(args: &Args) {
    let path = std::path::Path::new(&args.circuit);
    let mut files: Vec<String> = if path.is_dir() {
        let entries = std::fs::read_dir(path).unwrap_or_else(|e| panic!("can't read {}: {e}", path.display()));
        entries.filter_map(|e| e.ok()).map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.ends_with(".diy")).collect()
    } else {
        vec![args.circuit.clone()]
    };
    files.sort();
    if files.is_empty() || files.iter().any(|f| !f.ends_with(".diy")) {
        panic!("check reads DIYLC files: give it a .diy file or a folder of them");
    }
    // A stop (panic) in one file goes in the report below, with where it happened, and the check
    // carries on with the next file.
    std::panic::set_hook(Box::new(|info| {
        *LAST_STOP.lock().unwrap() = info.location().map_or(String::new(), |l| format!("at {}:{}", l.file(), l.line()));
    }));
    let (mut same, mut skipped, mut unfinished, mut wrong, mut crashed) = (0, 0, 0, 0, 0);
    for f in &files {
        let name = std::path::Path::new(f).file_name().unwrap().to_string_lossy().into_owned();
        match check_one(f, args) {
            Outcome::Same => same += 1,
            Outcome::NoConnections => skipped += 1,
            Outcome::Unreadable(why) => {
                skipped += 1;
                println!("  can't read  {name}: {why}");
            }
            Outcome::Unfinished(pins) => {
                unfinished += 1;
                println!("  unfinished  {name}: {pins} pins still share a hole or hang off the board; try more --steps");
            }
            Outcome::Different(missing, extra) => {
                wrong += 1;
                println!("  WRONG       {name}: the written file has different connections");
                missing.iter().for_each(|n| println!("                only in the file:    {}", n.join(" ")));
                extra.iter().for_each(|n| println!("                only in the written: {}", n.join(" ")));
            }
            Outcome::LayoutChanged(why) => {
                wrong += 1;
                println!("  WRONG       {name}: {why}");
            }
            Outcome::Crashed(why) => {
                crashed += 1;
                println!("  STOPPED     {name}: {why}");
            }
        }
    }
    let _ = std::panic::take_hook();
    println!(
        "\n{} files: {same} same, {wrong} wrong, {crashed} stopped, {unfinished} unfinished, {skipped} skipped (no connections or can't read)",
        files.len()
    );
    if wrong + crashed > 0 {
        std::process::exit(1);
    }
}

/// What the program understood from the file: the parts, which pins connect, and the cases of the
/// board file's rules.
fn print_nets(cases: &[Circuit]) {
    let c = &cases[0]; // the parts and nets are the same in every case
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
    if cases.len() > 1 {
        println!("\n{} cases of the board file's rules:", cases.len());
        for (k, case) in cases.iter().enumerate() {
            println!("  {:>2}. {}", k + 1, case_name(case));
        }
    }
}

/// How a layout was made, for its layout file: from `args`, the circuit at `path`, its board file,
/// where the placer started, and which case `k` of `cases` won.
fn made(args: &Args, path: &str, board_file: Option<String>, judge: &str, start: &str, cases: &[Circuit], k: usize) -> Made {
    Made {
        circuit: path.to_string(),
        board_file,
        judge: judge.to_string(),
        start: start.to_string(),
        seed: args.seed,
        steps: args.steps,
        case: k + 1,
        cases: cases.len(),
        choices: cases[k].constraints.choices.clone(),
    }
}

fn place_cmd(cases: &[Circuit], args: &Args, sketch: Option<board::Placement>, board_file: Option<String>) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(args.seed);
    let net = (args.judge == "net").then(|| Net::load(MODEL));
    let judge = |m: &Measures| match &net {
        Some(net) => net.predict(m),
        None => board::score(m),
    };

    let placer = (args.start == "net").then(placing::load);
    let (from, start_label, start_name) = match (&placer, &sketch) {
        (Some(placer), _) => (StartFrom::Network(placer), "The placing network's layout", "network"),
        (None, Some(sketch)) => (StartFrom::Sketch(sketch), "Your sketch", "sketch"),
        (None, None) => (StartFrom::Random, "Random start", "random"),
    };
    let (k, start, best) = place_cases(cases, judge, args.steps, &mut rng, &from, true, |_, _| {});
    let c = &cases[k];
    if cases.len() > 1 {
        println!("\nBest: case {} of {}\n", k + 1, cases.len());
    }
    println!("{}", board::report(c, &start, start_label));
    println!("{}\n", board::ascii_board(c, &start));

    let label = match case_name(c) {
        choices if choices.is_empty() => format!("Placed by {}", args.judge),
        choices => format!("Placed by {} ({choices})", args.judge),
    };
    println!("{}", board::report(c, &best, &label));
    if let Some(net) = &net {
        let m = board::measure(c, &best);
        println!("  (the network thought it would score {:.0}; the rules say {:.0})", net.predict(&m), board::score(&m));
    }
    println!("{}", board::ascii_board(c, &best));
    for problem in constraints::problems(c, &best) {
        println!("  !! {problem}");
    }

    // The finished board, routed and measured once: the layout file, and every drawing made from it.
    let made = made(args, &args.circuit, board_file, &args.judge, start_name, cases, k);
    let layout = Layout::new(c.clone(), best, &label, made.clone());
    let saved = format!("out/{}.layout.json", args.judge);
    std::fs::write(&saved, layout::to_json(&layout)).unwrap();
    // The start, for the page that shows it next to the result. It isn't in the layout file.
    let start = Layout::new(c.clone(), start, start_label, made);
    let path = format!("out/{}.html", args.judge);
    let html = format!(
        "<!doctype html><meta charset='utf-8'><title>Perfboard layout</title>\
         <body style='background:#eee;display:flex;gap:16px;flex-wrap:wrap;padding:16px'>{}{}</body>",
        svg::draw(&start, Side::Component),
        svg::draw(&layout, Side::Component)
    );
    std::fs::write(&path, html).unwrap();
    let [picture, solder, kicad, diy] = write_drawings(&layout, &format!("out/{}", args.judge));
    println!("\nLayout: {saved} (`draw {saved}` draws it again)");
    println!("Pictures: {picture}, {solder} (turned over to solder), {path} (before and after)");
    println!("For KiCad: {kicad}   For DIYLC: {diy}   ({:.2}s)", t0.elapsed().as_secs_f64());
}

/// Draws a layout as `<stem>.svg`, `<stem>-solder.svg`, `<stem>.kicad_pcb` and `<stem>.diy`, and
/// returns those names.
fn write_drawings(layout: &Layout, stem: &str) -> [String; 4] {
    let picture = format!("{stem}.svg");
    std::fs::write(&picture, svg::draw(layout, Side::Component)).unwrap();
    let solder = format!("{stem}-solder.svg");
    std::fs::write(&solder, svg::draw(layout, Side::Solder)).unwrap();
    let kicad = format!("{stem}.kicad_pcb");
    std::fs::write(&kicad, kicad_pcb::write(layout)).unwrap();
    let diy = format!("{stem}.diy");
    std::fs::write(&diy, diylc::write(layout)).unwrap();
    [picture, solder, kicad, diy]
}

/// Reads a layout file (`place` writes one, like out/rules.layout.json) and draws it again next to
/// it, as it was saved: nothing is placed or routed.
fn draw_cmd(path: &str) {
    let stem = path.strip_suffix(".layout.json").unwrap_or_else(|| panic!("draw reads a layout file, like out/rules.layout.json (place writes one), not {path}"));
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let layout = layout::from_json(&text);
    let (m, made) = (&layout.measures, &layout.made);
    println!("{path}: {}, from {} (seed {}, {} steps)", layout.title, made.circuit, made.seed, made.steps);
    println!("  score {:.0}/100, cost {:.1}", board::score(m), board::cost(m));
    for problem in &layout.problems {
        println!("  !! {problem}");
    }
    let [picture, solder, kicad, diy] = write_drawings(&layout, stem);
    println!("Pictures: {picture}, {solder}   For KiCad: {kicad}   For DIYLC: {diy}");
}

/// Example boards for training: (to learn from, kept aside to test).
/// Random boards, plus boards caught part-way through auto-placement so there are good ones too,
/// taking the cases of the board file's rules in turn.
fn make_examples(cases: &[Circuit], rng: &mut StdRng) -> (Vec<Measures>, Vec<Measures>) {
    let mut examples: Vec<Measures> = Vec::new();
    for i in 0..12 {
        let c = &cases[i % cases.len()];
        let mut snap_rng = StdRng::seed_from_u64(rng.random());
        place::anneal(c, board::score, 4000, rng, place::Start::Random, |s| {
            if snap_rng.random::<f64>() < 0.02 {
                examples.push(board::measure(c, s.current));
            }
        });
    }
    for i in 0..examples.len() / 4 {
        let c = &cases[i % cases.len()];
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
fn train(cases: &[Circuit], seed: u64) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(cases, &mut rng);

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
fn evolve(cases: &[Circuit], seed: u64) {
    const CHILDREN: usize = 19;
    const GENERATIONS: usize = 1000;
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(cases, &mut rng);
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
