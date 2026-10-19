//! The layout file: one plain file that describes a finished board. Where every part sits, which
//! hole each pin is in and which net it's on, where the copper goes, the score, and how it was made.
//!
//! `place` builds a `Layout` once (routing and measuring it once), saves it as
//! out/<judge>.layout.json, and the SVG pictures, the KiCad board and the DIYLC project are all
//! drawn from it. `draw` reads a saved one back and draws it again, with the copper as it was
//! saved: it doesn't place or route anything, so an old layout draws the same after the router
//! changes.
//!
//! The file is JSON:
//!
//!     { "format": "harmonia-layout", "version": 1, "title": "Placed by rules",
//!       "made": { "circuit": "circuits/555_blinker.net", "judge": "rules", "seed": 1, ... },
//!       "board": { "width": 18, "height": 12, "keepouts": [ { "name": "M3 screw", "from": [16, 0], "to": [17, 1] } ] },
//!                  (a soft keep-out also has its "weight")
//!       "power": "+5V", "ground": "GND",
//!       "parts": [ { "reference": "U1", "at": [6, 3], "rot": 0, "size": [4, 4], ...,
//!                    "pins": [ { "name": "1", "at": [0, 3], "hole": [6, 6], "net": "GND" }, ... ] }, ... ],
//!       "nets": [ { "name": "GND", "pins": ["U1.1", "C1.2", "J1.2"] }, ... ],
//!       "runs": [ { "net": "GND", "star": null, "pins": ["J1.2", "U1.1", "C1.2"],
//!                   "traces": [ [[6, 6], [6, 9]] ], "jumpers": [], "solder_wires": [] }, ... ],
//!       "score": { "score": 57.0, "cost": 84.6, "measures": { "wire": 41.0, ... } },
//!       "problems": [] }
//!
//! A part's `at` and `rot` mean what they mean in the board file's `fixed` rule (top-left hole after
//! turning, quarter turns clockwise); its `size` is before turning, and each pin's `at` is its hole
//! inside the part before turning, `hole` where that lands on the board. A run is what the router
//! joins with one piece of copper (a net, or one branch of a star net), listed in the order they
//! were routed; its traces are straight pieces, hole to hole.
//!
//! Not in the file: net colours (worked out from the nets), the start of the placer, the board
//! file's rules about where parts may go, near and close, and where a soft star's runs joined. The
//! score is kept as it was measured.

use crate::board::{self, BoardSize, Circuit, Component, Measures, Net, Pin, Place, Placement, Point, Run};
use crate::constraints::{Constraints, Keepout};
use crate::diylc;
use crate::diylc_import::Source;
use crate::route::{self, Routing};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// What the file says it is, and which version of the file format. A change to the format that
/// an older reader would get wrong gets a new version.
const FORMAT: &str = "harmonia-layout";
const VERSION: u32 = 1;

/// Lines in the file are kept to about this many characters (see `pretty`).
const LINE: usize = 100;

/// How a layout was made, for the record.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Made {
    /// The circuit file: a KiCad netlist or a DIYLC sketch.
    pub circuit: String,
    /// The board file next to it, if there was one.
    pub board_file: Option<String>,
    /// Who judged the layouts: "rules" or "net".
    pub judge: String,
    /// Where the placer started: "random" or "sketch".
    pub start: String,
    pub seed: u64,
    pub steps: usize,
    /// Which case of the board file's rules this is, counted from 1, out of how many, and the
    /// option it took in each one-of.
    pub case: usize,
    pub cases: usize,
    pub choices: Vec<String>,
}

/// A finished board, in memory: everything the drawings need.
pub struct Layout {
    /// The circuit for this case of the board file's rules. Its runs are in the order the router
    /// laid them, so each of `routing`'s lists has one run's pieces after the other, runs in order.
    pub circuit: Circuit,
    pub placement: Placement,
    pub routing: Routing,
    pub measures: Measures,
    /// Plain-language problems, the `!!` lines: what isn't allowed (`board::clash_words`), and pins left unrouted.
    pub problems: Vec<String>,
    /// What the pictures and the DIYLC project are called, like "Placed by rules".
    pub title: String,
    pub made: Made,
}

impl Layout {
    /// Routes and measures `placement` once, and keeps it all together.
    pub fn new(circuit: Circuit, placement: Placement, title: &str, made: Made) -> Layout {
        let g = board::geometry(&circuit, &placement);
        let routing = route::route(&circuit, &g);
        let measures = board::measure_routed(&circuit, &placement, &g, &routing);
        // A layout from the placer is always allowed; a sketch as you drew it may not be.
        let clashes = board::clashes(&circuit, &g, &vec![true; placement.len()]);
        let mut problems = board::clash_words(&circuit, &clashes);
        problems.extend(route::unrouted_problems(&circuit, &g, &routing));
        // Put the runs in the order the router laid them, numbering the copper to match. It's
        // the same copper; only the run numbers change. The file then lists the runs in that
        // order, and reading it back gives the copper in the same order.
        let order = route::order(&circuit, &g);
        let mut renumbered = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            renumbered[old] = new;
        }
        let renumber = |list: Vec<(usize, Point, Point)>| list.into_iter().map(|(run, a, b)| (renumbered[run], a, b)).collect();
        let routing = Routing {
            traces: renumber(routing.traces),
            jumpers: renumber(routing.jumpers),
            bottom: renumber(routing.bottom),
            unrouted: renumber(routing.unrouted),
            joins: routing.joins.into_iter().map(|(run, at)| (renumbered[run], at)).collect(),
        };
        let mut circuit = circuit;
        circuit.runs = order.iter().map(|&old| circuit.runs[old].clone()).collect();
        Layout { circuit, placement, routing, measures, problems, title: title.to_string(), made }
    }
}

// ------------------------------------------------------------ the file, as serde reads and writes it

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)] // so a typo in a hand edit is an error, not silently ignored
struct File {
    format: String,
    version: u32,
    title: String,
    made: Made,
    board: BoardEntry,
    power: Option<String>,
    ground: Option<String>,
    parts: Vec<PartEntry>,
    nets: Vec<Net>,
    runs: Vec<RunEntry>,
    score: Score,
    problems: Vec<String>,
}

/// Just the first two fields, read first so a file of another version gets a plain message.
#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardEntry {
    width: i32,
    height: i32,
    keepouts: Vec<KeepoutEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeepoutEntry {
    name: String,
    from: Point,
    to: Point,
    /// A soft keep-out's weight (parts and copper may go on it); left out for a hard one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartEntry {
    reference: String,
    value: String,
    /// The KiCad footprint name, or "DIYLC:..." for a part read from a DIYLC sketch.
    footprint: String,
    /// Top-left hole after turning, and quarter turns clockwise.
    at: Point,
    rot: u8,
    /// Width and height in holes, before turning.
    size: (i32, i32),
    low: bool,
    /// footprints.json's "diylc" entry: which DIYLC part to draw it as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diylc: Option<diylc::Part>,
    pins: Vec<PinEntry>,
    /// The holes the part takes, inside its outline before turning: only written when it isn't
    /// the whole outline (a module on headers, which low parts can go under).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Vec<Point>>,
    /// For a part read from a DIYLC sketch: your drawing of it, written back moved and turned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diylc_drawing: Option<Source>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinEntry {
    name: String,
    /// Its hole inside the part, before turning.
    at: Point,
    /// Its hole on the board.
    hole: Point,
    /// Between holes, like a tactile switch's legs (`board::Pin::off_grid`).
    #[serde(default, skip_serializing_if = "is_false")]
    off_grid: bool,
    /// Its net, if it's connected to anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    net: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunEntry {
    net: String,
    /// The star point, if this run is a branch of a star net.
    star: Option<String>,
    pins: Vec<String>,
    /// Copper traces on the solder side, as straight pieces: [from hole, to hole].
    traces: Vec<[Point; 2]>,
    /// Jumpers on the parts' side.
    jumpers: Vec<[Point; 2]>,
    /// Wires on the solder side, the last resort.
    solder_wires: Vec<[Point; 2]>,
    /// Connections the router couldn't make: [nearest hole joined, pin]. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    unrouted: Vec<[Point; 2]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Score {
    score: f64,
    cost: f64,
    /// Each of `board::FEATURES`, by name.
    measures: BTreeMap<String, f64>,
}

/// "U1.8": a pin's name, as nets and board files write it.
fn pin_name(c: &Circuit, (ci, pi): (usize, usize)) -> String {
    format!("{}.{}", c.components[ci].reference, c.components[ci].pins[pi].name)
}

/// The layout as the text of a layout file.
pub fn to_json(layout: &Layout) -> String {
    let c = &layout.circuit;
    let g = board::geometry(c, &layout.placement);
    let mut net_of: HashMap<(usize, usize), &str> = HashMap::new();
    for (net, pins) in c.nets.iter().zip(&c.net_pins) {
        for &p in pins {
            net_of.entry(p).or_insert(&net.name);
        }
    }
    let parts = c
        .components
        .iter()
        .zip(&layout.placement)
        .enumerate()
        .map(|(ci, (comp, place))| PartEntry {
            reference: comp.reference.clone(),
            value: comp.value.clone(),
            footprint: comp.footprint.clone(),
            at: (place.x, place.y),
            rot: place.rot,
            size: (comp.w, comp.h),
            low: comp.low,
            diylc: comp.diylc.clone(),
            pins: comp
                .pins
                .iter()
                .enumerate()
                .map(|(pi, pin)| PinEntry {
                    name: pin.name.clone(),
                    at: pin.at,
                    hole: g.pins[ci][pi],
                    off_grid: pin.off_grid,
                    net: net_of.get(&(ci, pi)).map(|n| n.to_string()),
                })
                .collect(),
            body: (comp.body != Component::full_body(comp.w, comp.h)).then(|| comp.body.clone()),
            diylc_drawing: comp.source.clone(),
        })
        .collect();
    // Each run's copper. The traces as straight pieces, the way the drawings show them; each run's
    // pieces come one after the other (see `Layout::circuit`), so picking them out keeps their order.
    let straight = layout.routing.straight_traces();
    let of_run = |list: &[(usize, Point, Point)], r: usize| list.iter().filter(|t| t.0 == r).map(|&(_, a, b)| [a, b]).collect();
    let runs = c
        .runs
        .iter()
        .enumerate()
        .map(|(r, run)| RunEntry {
            net: c.nets[run.net].name.clone(),
            star: run.star.map(|p| pin_name(c, p)),
            pins: run.pins.iter().map(|&p| pin_name(c, p)).collect(),
            traces: of_run(&straight, r),
            jumpers: of_run(&layout.routing.jumpers, r),
            solder_wires: of_run(&layout.routing.bottom, r),
            unrouted: of_run(&layout.routing.unrouted, r),
        })
        .collect();
    let m = &layout.measures;
    let file = File {
        format: FORMAT.to_string(),
        version: VERSION,
        title: layout.title.clone(),
        made: layout.made.clone(),
        board: BoardEntry {
            width: c.board.width,
            height: c.board.height,
            keepouts: c.constraints.keepout.iter().map(|k| KeepoutEntry { name: k.name.clone(), from: k.from, to: k.to, weight: k.weight }).collect(),
        },
        power: c.power.clone(),
        ground: c.ground.clone(),
        parts,
        nets: c.nets.clone(),
        runs,
        score: Score {
            score: board::score(m),
            cost: board::cost(m),
            measures: board::FEATURES.iter().zip(m.0).map(|(k, v)| (k.to_string(), v)).collect(),
        },
        problems: layout.problems.clone(),
    };
    pretty(&serde_json::to_string(&file).expect("a layout always turns into JSON"))
}

/// Reads the text of a layout file back into a layout, with its copper as saved (nothing is placed
/// or routed again). Stops with a message if the file isn't a layout file of this version, or
/// disagrees with itself (a pin's `hole` that isn't where its part puts it, say).
pub fn from_json(text: &str) -> Layout {
    let header: Header = serde_json::from_str(text).unwrap_or_else(|e| panic!("not a layout file (it isn't JSON with an object in it): {e}"));
    if header.format.as_deref() != Some(FORMAT) {
        panic!("not a layout file: it should say \"format\": \"{FORMAT}\" (place writes one, like out/rules.layout.json)");
    }
    match header.version {
        Some(VERSION) => {}
        Some(v) => panic!("this layout file is version {v}, but this program reads version {VERSION}: make it again with `place`"),
        None => panic!("this layout file has no \"version\""),
    }
    let file: File = serde_json::from_str(text).unwrap_or_else(|e| panic!("bad layout file: {e}"));

    let components: Vec<Component> = file
        .parts
        .iter()
        .map(|p| Component {
            reference: p.reference.clone(),
            value: p.value.clone(),
            footprint: p.footprint.clone(),
            diylc: p.diylc.clone(),
            w: p.size.0,
            h: p.size.1,
            pins: p.pins.iter().map(|pin| Pin { name: pin.name.clone(), at: pin.at, off_grid: pin.off_grid }).collect(),
            body: p.body.clone().unwrap_or_else(|| Component::full_body(p.size.0, p.size.1)),
            source: p.diylc_drawing.clone(),
            low: p.low,
        })
        .collect();
    let placement: Placement = file.parts.iter().map(|p| Place { x: p.at.0, y: p.at.1, rot: p.rot }).collect();
    let board = BoardSize { width: file.board.width, height: file.board.height };
    let mut c = Circuit::new(board, components, file.nets, file.power, file.ground);

    // The board file's rules aren't in the layout file, only what the drawings show of them: the
    // keep-outs and which option of each one-of this case took.
    c.constraints = Constraints { choices: file.made.choices.clone(), allowed: vec![None; c.components.len()], ..Default::default() };
    for k in &file.board.keepouts {
        c.constraints.add_keepout(Keepout { from: k.from, to: k.to, name: k.name.clone(), weight: k.weight });
    }

    // The runs and their copper as saved, in the order they were routed.
    let net_index = |name: &str| c.nets.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("layout file: a run is on net {name}, which isn't in \"nets\""));
    let mut runs = Vec::new();
    let mut routing = Routing::default();
    for (r, entry) in file.runs.iter().enumerate() {
        runs.push(Run {
            net: net_index(&entry.net),
            pins: entry.pins.iter().map(|p| c.find_pin(p)).collect(),
            star: entry.star.as_ref().map(|p| c.find_pin(p)),
        });
        for &[a, b] in &entry.traces {
            routing.traces.extend(steps(a, b).into_iter().map(|(a, b)| (r, a, b)));
        }
        routing.jumpers.extend(entry.jumpers.iter().map(|&[a, b]| (r, a, b)));
        routing.bottom.extend(entry.solder_wires.iter().map(|&[a, b]| (r, a, b)));
        routing.unrouted.extend(entry.unrouted.iter().map(|&[a, b]| (r, a, b)));
    }
    c.runs = runs;

    // The score as it was measured: `rules` needs board-file rules the layout file doesn't keep
    // (and where a soft star's runs joined).
    let measures = &file.score.measures;
    let missing: Vec<&str> = board::FEATURES.iter().copied().filter(|k| !measures.contains_key(*k)).collect();
    let extra: Vec<&String> = measures.keys().filter(|k| !board::FEATURES.contains(&k.as_str())).collect();
    if !missing.is_empty() || !extra.is_empty() {
        panic!("layout file: it was measured on other things than this program measures (missing {missing:?}, extra {extra:?}): make it again with `place`");
    }
    let measures = Measures(board::FEATURES.map(|k| measures[k]));

    // What the file says twice must agree: each pin's hole and net.
    let g = board::geometry(&c, &placement);
    for (ci, part) in file.parts.iter().enumerate() {
        for (pi, pin) in part.pins.iter().enumerate() {
            let name = pin_name(&c, (ci, pi));
            if pin.hole != g.pins[ci][pi] {
                panic!("layout file: {name} says it's in hole {:?}, but its part's at, rot and size put it in {:?}", pin.hole, g.pins[ci][pi]);
            }
            let net = c.nets.iter().zip(&c.net_pins).find(|(_, pins)| pins.contains(&(ci, pi))).map(|(n, _)| n.name.clone());
            if pin.net != net {
                panic!("layout file: {name} says it's on net {:?}, but \"nets\" puts it on {net:?}", pin.net);
            }
        }
    }
    Layout { circuit: c, placement, routing, measures, problems: file.problems, title: file.title, made: file.made }
}

/// A straight trace from `a` to `b` (across, down or at 45°) as the steps between neighbouring
/// holes the router laid it in.
fn steps(a: Point, b: Point) -> Vec<(Point, Point)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    if !(dx == 0 || dy == 0 || dx.abs() == dy.abs()) || a == b {
        panic!("layout file: a trace from {a:?} to {b:?} isn't straight across, down or at 45°");
    }
    let (sx, sy) = (dx.signum(), dy.signum());
    let n = dx.abs().max(dy.abs());
    (0..n).map(|k| ((a.0 + sx * k, a.1 + sy * k), (a.0 + sx * (k + 1), a.1 + sy * (k + 1)))).collect()
}

// ------------------------------------------------------------ pretty JSON

/// Pretty JSON from compact JSON (as serde_json writes it, with no spaces outside strings). A value
/// that fits on one line stays on one, `{"name": "1", "at": [0, 3], "hole": [6, 6]}`; a longer one
/// gets each of its items on a line of its own, indented two spaces more. serde_json's own pretty
/// printer puts every number of every point on a line of its own, which makes a long, hard file.
fn pretty(compact: &str) -> String {
    let mut out = String::new();
    write_pretty(compact, 0, 0, &mut out);
    out.push('\n');
    out
}

/// Writes one JSON value, which starts `indent` spaces in, after `before` characters (its key).
fn write_pretty(json: &str, indent: usize, before: usize, out: &mut String) {
    let one_line = spaced(json);
    let container = json.starts_with('{') || json.starts_with('[');
    if !container || json.len() == 2 || indent + before + one_line.len() <= LINE {
        out.push_str(&one_line);
        return;
    }
    let (open, inner, close) = (&json[..1], &json[1..json.len() - 1], &json[json.len() - 1..]);
    let pad = " ".repeat(indent + 2);
    out.push_str(open);
    let mut i = 0;
    while i < inner.len() {
        if i > 0 {
            out.push(',');
        }
        out.push('\n');
        out.push_str(&pad);
        let mut key_len = 0;
        if open == "{" {
            let key_end = end_of(inner, i);
            out.push_str(&inner[i..key_end]);
            out.push_str(": ");
            key_len = key_end - i + 2;
            i = key_end + 1; // past the ':'
        }
        let end = end_of(inner, i);
        write_pretty(&inner[i..end], indent + 2, key_len, out);
        i = end + 1; // past the ',' (or the end)
    }
    out.push('\n');
    out.push_str(&" ".repeat(indent));
    out.push_str(close);
}

/// Where the JSON value starting at `start` ends: just past its closing bracket or quote, or, for a
/// number, true, false or null, at the ',' or bracket after it. Brackets and commas inside strings
/// don't count.
fn end_of(json: &str, start: usize) -> usize {
    let (mut depth, mut in_string, mut escaped) = (0, false, false);
    for (i, b) in json.bytes().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
                if depth == 0 {
                    return i + 1;
                }
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return i, // the end of the list or object it's in
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b',' if depth == 0 => return i,
            _ => {}
        }
    }
    json.len()
}

/// Compact JSON on one line, with a space after each ',' and ':' that isn't inside a string.
fn spaced(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + json.len() / 4);
    let (mut in_string, mut escaped) = (false, false);
    for ch in json.chars() {
        out.push(ch);
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == ',' || ch == ':' {
            out.push(' ');
        }
    }
    out
}
