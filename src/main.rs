//! Perfboard layout + a tiny neural network.
//!
//!     cargo run --release -- place                # auto-place using the hand-written rules
//!     cargo run --release -- train                # teach the network to score boards like the rules do
//!     cargo run --release -- evolve               # the same job as train, done by evolution (1 parent + 19 children)
//!     cargo run --release -- place --judge net    # auto-place again, with the network as the judge
//!     cargo run --release -- place my_sketch.diy  # read a rough DIYLC layout, score it, and fit it on the board
//!     cargo run --release -- nets my_sketch.diy   # just show the parts and connections the program understood
//!     cargo run --release -- check sketches/      # place, write and re-read every .diy: same connections?
//!     cargo run --release -- draw out/rules.layout.json  # draw a saved layout again, without placing it
//!     cargo run --release -- bench                # the benchmark: place a fixed set of circuits, record the costs
//!     cargo run --release -- footprints           # the footprints a netlist can use (built in, yours, and next to a my_circuit.net)
//!
//! Installed (`cargo install --path .`), `harmonia place my_circuit.net --out my_folder` is the same.

mod bench;
mod board;
mod build;
mod check;
mod constraints;
mod diylc;
mod kicad_pcb;
mod learn;
mod netlist;
mod nn;
mod place;
mod route;
mod sexpr;
mod svg;

use board::{BoardSize, Design, Measures};
use build::{Build, Layout, Made};
use nn::Net;
use svg::Side;
use rand::{SeedableRng, rngs::StdRng};
use std::time::Instant;

/// Where the outputs go, unless `--out` says otherwise.
const OUT: &str = "out";
/// The trained network, in the output folder.
fn model_path(out: &str) -> String {
    format!("{out}/model.json")
}
/// How hard the placer tries, unless `--steps` says otherwise. `bench` always uses it.
const DEFAULT_STEPS: usize = 20_000;

struct Args {
    command: String,
    circuit: String,
    judge: String,
    steps: usize,
    seed: u64,
    board: Option<BoardSize>,
    power: Option<String>,
    ground: Option<String>,
    /// The folder the outputs go in.
    out: String,
    /// The board file to use instead of the one next to the circuit.
    board_file: Option<String>,
}

/// What `--help` (or no arguments at all) prints.
const USAGE: &str = "\
harmonia: places parts on a perfboard, routes the copper, and draws it.

  harmonia place <circuit>       place and route a KiCad netlist (.net) or a DIYLC sketch (.diy)
  harmonia nets <circuit>        list the parts and nets it understood
  harmonia footprints [<circuit>] the parts it knows: built in, yours (~/.config/harmonia/footprints.json),
                                 and with a circuit, a footprints.json next to it
  harmonia draw <x.layout.json>  draw a saved layout again
  harmonia check <file or folder of .diy>  place, write and read back: same connections?
  harmonia train | evolve | bench  the network and the benchmark (from the repo)

Options:
  --seed N           the placer's random start (default 1); the same seed makes the same layout
  --steps N          how hard the placer tries (default 20000)
  --board WxH        board size in holes (default: the board file's, else 18x12)
  --power NET, --ground NET   supply and ground nets (default: the board file's, else guessed)
  --board-file F     the board file to use (default: the circuit's name with .json, next to it)
  --out DIR          where place, check, train and evolve write (default out)
  --judge rules|net  who scores layouts (default rules)

The board file (rules, weights) and the outputs are described in the README.";

fn parse_args() -> Args {
    let mut args = Args {
        command: "place".into(),
        circuit: "circuits/555_blinker.net".into(),
        judge: "rules".into(),
        steps: DEFAULT_STEPS,
        seed: 1,
        board: None,
        power: None,
        ground: None,
        out: OUT.into(),
        board_file: None,
    };
    if std::env::args().len() == 1 {
        println!("{USAGE}");
        std::process::exit(0);
    }
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" | "help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--judge" => args.judge = it.next().expect("--judge rules|net"),
            "--steps" => args.steps = it.next().and_then(|s| s.parse().ok()).expect("--steps <number>"),
            "--seed" => args.seed = it.next().and_then(|s| s.parse().ok()).expect("--seed <number>"),
            "--board" => args.board = Some(it.next().and_then(|s| parse_size(&s)).expect("--board <width>x<height>, like 18x12")),
            "--power" => args.power = Some(it.next().expect("--power <net name>")),
            "--ground" => args.ground = Some(it.next().expect("--ground <net name>")),
            "--out" => args.out = it.next().expect("--out <folder>"),
            "--board-file" => args.board_file = Some(it.next().expect("--board-file <file.json>")),
            "place" | "train" | "evolve" | "nets" | "check" | "draw" | "bench" | "footprints" => args.command = a,
            flag if flag.starts_with('-') => {
                eprintln!("unknown option {flag}\n\n{USAGE}");
                std::process::exit(2);
            }
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
    // Only the commands that write there make the output folder (bench writes to out/ itself).
    if matches!(args.command.as_str(), "place" | "check" | "train" | "evolve" | "bench") {
        let dir = if args.command == "bench" { OUT } else { &args.out };
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("can't make the folder {dir}: {e}"));
    }
    match args.command.as_str() {
        "check" => return check::run(&args),
        "draw" => return draw_cmd(&args.circuit),
        "footprints" => return netlist::describe(&args.circuit).iter().for_each(|f| println!("{f}")),
        "bench" => return bench::run(),
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
        "train" => learn::train(cases, args.seed, &again(&args, "train", &args.circuit), &args.out),
        "evolve" => learn::evolve(cases, args.seed, &again(&args, "evolve", &args.circuit), &args.out),
        "nets" => print_nets(cases),
        _ => place_cmd(cases, &args, loaded.sketch, loaded.copper, loaded.board_file),
    }
}

/// A circuit and the board file next to it, as `load` reads them.
struct Loaded {
    /// The circuit once for each case of the board file's rules (one, unless a one-of makes more).
    cases: Vec<Design>,
    /// Cases left out because some part has no place meeting their rules, and why.
    left_out: Vec<String>,
    /// For a DIYLC sketch, where you put things: a starting point.
    sketch: Option<board::Placement>,
    /// For a DIYLC sketch, the copper you drew, to score your own wiring.
    copper: Option<diylc::Copper>,
    /// The board file's path, if there is one.
    board_file: Option<String>,
}

/// Reads a circuit (a KiCad netlist or a DIYLC sketch) and the board file next to it, if any.
fn load(path: &str, args: &Args) -> Loaded {
    // Flags win over the board file, so you can try a variation quickly.
    let (board_file_path, board_file) = constraints::read_board_file(path, args.board_file.as_deref()).unzip();
    let board_file = board_file.unwrap_or_default();
    let from_file = board_file.board.as_ref().map(|s| parse_size(s).unwrap_or_else(|| panic!("board file: \"board\" should look like \"18x12\"")));
    let opts = netlist::Options {
        board: args.board.or(from_file).unwrap_or(BoardSize { width: 18, height: 12 }),
        power: args.power.clone().or(board_file.power.clone()),
        ground: args.ground.clone().or(board_file.ground.clone()),
        user_footprints: args.command != "bench",
    };
    let (circuit, sketch, copper) = if path.ends_with(".diy") {
        let (c, s, copper) = diylc::load(path, opts);
        (c, Some(s), Some(copper))
    } else {
        (netlist::load(path, opts), None, None)
    };
    let (cases, left_out) = constraints::designs(&circuit, &board_file.rules);
    if cases.is_empty() {
        panic!("board file: no case of its rules can be met:\n  {}", left_out.join("\n  "));
    }
    Loaded { cases, left_out, sketch, copper, board_file: board_file_path }
}

/// Places every case of the board file's rules, from the sketch if there is one, and keeps the
/// cheapest layout by `cost` (the rules' cost, or what the network's score stands for). Returns which
/// case it was, where it started (the sketch as you drew it, or the random start), the layout, routed,
/// and its measurements.
/// The placer starts from the sketch made allowed (`place::allowed_start`); a case with no room for
/// some part is left out, and if every case is, the program stops. With `report`, prints a line for
/// each case when there are several, and why a case was left out.
fn place_cases(
    cases: &[Design],
    cost: impl Fn(&Measures) -> f64,
    steps: usize,
    rng: &mut StdRng,
    sketch: Option<&board::Placement>,
    report: bool,
) -> (usize, board::Placement, Layout, Measures) {
    let mut best: Option<(usize, board::Placement, Layout, Measures, f64)> = None;
    let mut no_room = Vec::new();
    for (k, c) in cases.iter().enumerate() {
        let start = match sketch {
            Some(sketch) => place::allowed_start(c, sketch),
            None => place::random_start(c, rng),
        };
        let start = match start {
            Ok(start) => start,
            Err(why) => {
                let why = if cases.len() > 1 { format!("case {} of {} ({}): {why}", k + 1, cases.len(), case_name(c)) } else { why };
                if report {
                    println!("Left out: {why}");
                }
                no_room.push(why);
                continue;
            }
        };
        let layout = place::anneal(c, &cost, steps, rng, start.clone(), |_| {});
        let g = board::geometry(c, &layout);
        let routing = route::route(c, &layout, &g);
        let m = board::measure_routed(c, &layout, &g, &routing);
        if report && cases.len() > 1 {
            println!("{}", board::report(c, &layout, &m, &format!("Case {} of {} ({})", k + 1, cases.len(), case_name(c))));
        }
        let this = cost(&m);
        if best.as_ref().is_none_or(|b| this < b.4) {
            // Show the sketch as you drew it (only its placement rules applied), not as it was moved apart.
            let shown = match sketch {
                Some(sketch) => {
                    let mut drawn = sketch.clone();
                    constraints::apply(c, &mut drawn);
                    drawn
                }
                None => start,
            };
            best = Some((k, shown, Layout { placement: layout, routing }, m, this));
        }
    }
    let Some((k, start, layout, m, _)) = best else {
        panic!("no case has room for every part:\n  {}", no_room.join("\n  "));
    };
    (k, start, layout, m)
}

/// Which option of each one-of a case took, like `one-of "Pads by the ESP32's back pads": "by GP5"`.
fn case_name(c: &Design) -> String {
    c.constraints.choices.join("; ")
}

/// What the program understood from the file: the parts, which pins connect, and the cases of the
/// board file's rules.
fn print_nets(cases: &[Design]) {
    let c = &cases[0]; // the parts and nets are the same in every case (how one is mounted may not be: this is the first's)
    println!("{} parts:", c.components.len());
    for (i, comp) in c.components.iter().enumerate() {
        let kind = if comp.module && c.room_under(i) { "  (module: parts can go under it)" } else { "" };
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

/// Which program this is, for the outputs: its version and the commit it was built from (build.rs).
fn version() -> String {
    format!("harmonia {}, commit {}", env!("CARGO_PKG_VERSION"), env!("HARMONIA_COMMIT"))
}

/// How this program was started: `cargo run --release --` from its repo (cargo tells the program
/// its folder then), or the installed `harmonia`.
fn program() -> &'static str {
    if std::env::var_os("CARGO_MANIFEST_DIR").is_some() { "cargo run --release --" } else { "harmonia" }
}

/// The command that runs `command` on the circuit at `path` again, with the flags in `args` that
/// change what it makes, and `--out` so it writes where it did. The steps only count for placing.
fn again(args: &Args, command: &str, path: &str) -> String {
    let mut out = format!("{} {command} {path} --seed {}", program(), args.seed);
    if command == "place" {
        out += &format!(" --steps {}", args.steps);
        if args.judge != "rules" {
            out += &format!(" --judge {}", args.judge);
        }
    }
    if let Some(b) = args.board {
        out += &format!(" --board {}x{}", b.width, b.height);
    }
    if let Some(f) = &args.board_file {
        out += &format!(" --board-file {f}");
    }
    for (flag, value) in [("--power", &args.power), ("--ground", &args.ground)] {
        if let Some(v) = value {
            out += &format!(" {flag} {v}");
        }
    }
    if args.out != OUT {
        out += &format!(" --out {}", args.out);
    }
    out
}

/// How a layout was made, for its layout file: from `args`, the circuit at `path`, its board file,
/// where the placer started, and which case `k` of `cases` won.
fn made(args: &Args, path: &str, board_file: Option<String>, start: &str, cases: &[Design], k: usize) -> Made {
    Made {
        circuit: path.to_string(),
        board_file,
        judge: args.judge.clone(),
        start: start.to_string(),
        seed: args.seed,
        steps: args.steps,
        case: k + 1,
        cases: cases.len(),
        // `check` places a sketch just as `place` does, so `place` makes its layouts again too.
        command: again(args, "place", path),
        version: version(),
        footprints: {
            let mut from: Vec<String> = cases[k].components.iter().filter_map(|c| c.footprint_from.clone()).collect();
            from.sort();
            from.dedup();
            from
        },
    }
}

fn place_cmd(cases: &[Design], args: &Args, sketch: Option<board::Placement>, copper: Option<diylc::Copper>, board_file: Option<String>) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(args.seed);
    let net = (args.judge == "net").then(|| Net::load(&model_path(&args.out)));
    let judge = |m: &Measures| match &net {
        Some(net) => net.cost(m),
        None => board::cost(m),
    };

    let (start_label, start_name) = if sketch.is_some() { ("Your sketch", "sketch") } else { ("Random start", "random") };
    let (k, start, best, m) = place_cases(cases, judge, args.steps, &mut rng, sketch.as_ref(), true);
    let placed = best.placement.clone();
    let c = &cases[k];
    if cases.len() > 1 {
        println!("\nBest: case {} of {}\n", k + 1, cases.len());
    }
    // The start, for the page that shows it next to the result. It isn't in the layout file.
    let made = made(args, &args.circuit, board_file, start_name, cases, k);
    let start = Build::new(c.clone(), start, start_label, made.clone());
    println!("{}", board::report(c, &start.layout.placement, &start.measures, start_label));
    // Your own wiring, if you drew any, measured like the router's, and the router's on the same
    // placement (as you drew it, allowed or not), to compare.
    if let (Some(sketch), Some(copper)) = (&sketch, copper.as_ref().filter(|w| !w.traces.is_empty() || !w.jumpers.is_empty())) {
        let g = board::geometry(c, sketch);
        let (yours, breaks) = route::from_sketch(c, &g, copper);
        println!("{}", board::report_measures(&board::measure_routed(c, sketch, &g, &yours), "  wired as you drew it"));
        if breaks > 0 {
            println!("    (and it breaks the board file's star rules {breaks} times, which the router never does)");
        }
        println!("{}", board::report_measures(&board::measure(c, sketch), "  wired by the router"));
    }
    println!("{}\n", board::ascii_board(c, &start.layout.placement));

    let label = match case_name(c) {
        choices if choices.is_empty() => format!("Placed by {}", args.judge),
        choices => format!("Placed by {} ({choices})", args.judge),
    };
    println!("{}", board::report(c, &placed, &m, &label));
    if let Some(net) = &net {
        println!("  (the network thought it would score {:.0}; the rules say {:.0})", net.predict(&m), board::score(&m));
    }
    println!("{}", board::ascii_board(c, &placed));

    // The finished board with the copper it was judged by: the layout file, and every drawing made
    // from it, show that one routing.
    let build = Build::routed(c.clone(), best, &label, made);
    let made = &build.made;
    for problem in &build.problems {
        println!("  !! {problem}");
    }
    let saved = format!("{}/{}.layout.json", args.out, args.judge);
    std::fs::write(&saved, build::to_json(&build)).unwrap();
    let path = format!("{}/{}.html", args.out, args.judge);
    let html = format!(
        "<!doctype html><meta charset='utf-8'><title>Perfboard layout</title>\
         <body style='background:#eee;display:flex;gap:16px;flex-wrap:wrap;padding:16px'>{}{}</body>",
        svg::draw(&start, Side::Component),
        svg::draw(&build, Side::Component)
    );
    std::fs::write(&path, html).unwrap();
    let [picture, solder, kicad, diy] = write_drawings(&build, &format!("{}/{}", args.out, args.judge));
    println!("\nSeed {}. To make this layout again: {} ({})", made.seed, made.command, made.version);
    for footprint in &made.footprints {
        println!("  It also needs {footprint}.");
    }
    println!("Layout: {saved} (`draw {saved}` draws it again)");
    println!("Pictures: {picture}, {solder} (turned over to solder), {path} (before and after)");
    println!("For KiCad: {kicad}   For DIYLC: {diy}   ({:.2}s)", t0.elapsed().as_secs_f64());
}

/// Draws a layout as `<stem>.svg`, `<stem>-solder.svg`, `<stem>.kicad_pcb` and `<stem>.diy`, and
/// returns those names.
fn write_drawings(build: &Build, stem: &str) -> [String; 4] {
    let picture = format!("{stem}.svg");
    std::fs::write(&picture, svg::draw(build, Side::Component)).unwrap();
    let solder = format!("{stem}-solder.svg");
    std::fs::write(&solder, svg::draw(build, Side::Solder)).unwrap();
    let kicad = format!("{stem}.kicad_pcb");
    std::fs::write(&kicad, kicad_pcb::write(build)).unwrap();
    let diy = format!("{stem}.diy");
    std::fs::write(&diy, diylc::write(build)).unwrap();
    [picture, solder, kicad, diy]
}

/// Reads a layout file (`place` writes one, like out/rules.layout.json) and draws it again next to
/// it, with its copper as saved (nothing is placed or routed), scored again with today's
/// measurements.
fn draw_cmd(path: &str) {
    let stem = path.strip_suffix(".layout.json").unwrap_or_else(|| panic!("draw reads a layout file, like out/rules.layout.json (place writes one), not {path}"));
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let build = build::from_json(&text);
    let (m, made) = (&build.measures, &build.made);
    println!("{path}: {}, from {} ({})", build.title, made.circuit, made.again());
    println!("  score {:.0}/100, cost {:.1}", board::score(m), board::cost(m));
    for problem in &build.problems {
        println!("  !! {problem}");
    }
    let [picture, solder, kicad, diy] = write_drawings(&build, stem);
    println!("Pictures: {picture}, {solder}   For KiCad: {kicad}   For DIYLC: {diy}");
}
