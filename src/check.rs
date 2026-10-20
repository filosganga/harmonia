//! `check`: the round trip for DIYLC files. Each is placed as `place` places it, written out,
//! and read back, and must come back with the connections it started with. Its layout file, read
//! back, must give the same drawings.

use crate::board::{self, Design};
use crate::build::{self, Build};
use crate::svg::{self, Side};
use crate::{Args, diylc, kicad_pcb, load, made, place_cases};
use rand::{SeedableRng, rngs::StdRng};

/// The connections of a circuit, in an order that doesn't depend on the file: each net, as its
/// sorted pin names, sorted.
fn connections(c: &Design) -> Vec<Vec<String>> {
    let mut nets: Vec<Vec<String>> = c.nets.iter().map(|n| {
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
    /// When the placer stopped, the router couldn't reach this many pins, so the written file
    /// doesn't join them.
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
        let (k, _, best, _) = place_cases(&loaded.cases, board::cost, args.steps, &mut rng, Some(sketch), true);
        let c = &loaded.cases[k];
        let made = made(args, path, loaded.board_file.clone(), "sketch", &loaded.cases, k);
        let placed = best.placement.clone();
        let build = Build::routed(c.clone(), best, "check", made);
        let written = format!("{}/check.diy", args.out);
        let diy = diylc::write(&build);
        std::fs::write(&written, &diy).unwrap();
        // The layout file must keep everything the drawings need: write it, read it back, and the
        // file and the drawings from what was read must be the same as from the layout in memory.
        let saved = build::to_json(&build);
        std::fs::write(format!("{}/check.layout.json", args.out), &saved).unwrap();
        let read = build::from_json(&saved);
        if let Some(why) = differences(&build, &read, &saved, &diy) {
            return Outcome::LayoutChanged(why);
        }
        let after = connections(&load(&written, args).cases[0]);
        if before == after {
            return Outcome::Same;
        }
        // The placer's layouts are allowed, so no two pins share a hole and none is off the board:
        // nets can't join by themselves. Pins the router couldn't reach aren't joined in the drawing at all. Leave them out on both
        // sides: if everything else matches, only they are missing; if not, the writer is wrong.
        let g = board::geometry(c, &placed);
        let holes: std::collections::HashSet<board::Point> = build.layout.routing.unrouted.iter().map(|&(_, _, hole)| hole).collect();
        let mut unrouted = std::collections::HashSet::new();
        for (ci, comp) in c.components.iter().enumerate() {
            for pi in 0..comp.pins.len() {
                if holes.contains(&g.pins[ci][pi]) {
                    unrouted.insert(c.pin_name((ci, pi)));
                }
            }
        }
        let without = |nets: &[Vec<String>]| -> Vec<Vec<String>> {
            let mut out: Vec<Vec<String>> = nets.iter().map(|n| n.iter().filter(|p| !unrouted.contains(*p)).cloned().collect::<Vec<_>>()).filter(|n| n.len() > 1).collect();
            out.sort();
            out
        };
        let (before, after) = (without(&before), without(&after));
        if before == after {
            return Outcome::Unfinished(unrouted.len());
        }
        let missing = before.iter().filter(|n| !after.contains(n)).cloned().collect();
        let extra = after.iter().filter(|n| !before.contains(n)).cloned().collect();
        Outcome::Different(missing, extra)
    };
    caught(round_trip).unwrap_or_else(Outcome::Crashed)
}

/// What differs between a layout and the same layout written to a layout file (`saved`) and read
/// back (`read`): the file it writes again, or any of the drawings. None if nothing does.
fn differences(build: &Build, read: &Build, saved: &str, diy: &str) -> Option<String> {
    let checks = [
        ("the layout file written again", saved.to_string(), build::to_json(read)),
        ("the .diy", diy.to_string(), diylc::write(read)),
        ("the picture", svg::draw(build, Side::Component), svg::draw(read, Side::Component)),
        ("the solder-side picture", svg::draw(build, Side::Solder), svg::draw(read, Side::Solder)),
        ("the KiCad board", kicad_pcb::write(build), kicad_pcb::write(read)),
    ];
    let differ: Vec<&str> = checks.iter().filter(|(_, from_memory, from_file)| from_memory != from_file).map(|(what, ..)| *what).collect();
    (!differ.is_empty()).then(|| format!("read back from the layout file, {} came out different", differ.join(", ")))
}

/// Places every DIYLC file given (one file, or every .diy in a folder), writes it out the way
/// `place` does, reads that back, and checks it has exactly the connections it started with.
/// Exits with an error if any file comes back wrong or stops the program.
pub fn run(args: &Args) {
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
                println!("  unfinished  {name}: {pins} pins couldn't be reached by the router; try more --steps");
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
