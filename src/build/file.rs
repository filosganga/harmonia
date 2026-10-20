//! The layout file: one plain JSON file that holds a `Build`, what you asked for and what was
//! made of it. `to_json` writes it, `from_json` reads it back.
//!
//!     { "format": "harmonia-layout", "version": 3, "title": "Placed by rules",
//!       "made": { "circuit": "circuits/555_blinker.net", "judge": "rules", "seed": 1, ... },
//!       "design": {
//!         "board": { "width": 18, "height": 12 }, "power": "+5V", "ground": "GND",
//!         "rules": [ { "kind": "edge", "part": "J1", "edge": "left" }, ... ], "choices": [],
//!         "parts": [ { "reference": "U1", "value": "NE555", "footprint": "...", "size": [4, 4],
//!                      "layers": [1, 2], "pins": [ { "name": "1", "at": [0, 3] }, ... ] }, ... ],
//!         "nets": [ { "name": "GND", "pins": ["U1.1", "C1.2", "J1.2"] }, ... ] },
//!       "layout": {
//!         "parts": [ { "reference": "U1", "at": [6, 3], "rot": 0,
//!                      "pins": [ { "name": "1", "hole": [6, 6] }, ... ] }, ... ],
//!         "runs": [ { "net": "GND", "star": null, "pins": ["J1.2", "U1.1", "C1.2"],
//!                     "traces": [ [[6, 6], [6, 9]] ], "jumpers": [], "solder_wires": [] }, ... ] },
//!       "score": { "score": 57.0, "cost": 84.6, "measures": { "wire": 41.0, ... } },
//!       "problems": [] }
//!
//! The design is the circuit as read (parts before turning, each pin's hole inside its part, the
//! nets) and the board file's rules exactly as written, with `choices`: the option this case took
//! in each one-of. The layout is where each part sits (`at` and `rot` mean what they mean in a
//! `fixed` rule: top-left hole after turning, quarter turns clockwise), the hole each pin lands in,
//! and the copper: one run for each piece the router joins (a net, or a branch of a star net), in
//! the order it routes them first; its traces are straight pieces, hole to hole.
//!
//! The score and the problems are written for people to read. Reading the file back works them
//! out again from the design and the layout, with today's measurements.

use super::{Build, Layout, Made};
use crate::board::{BoardSize, Component, Design, Layers, Net, Pin, Place, Placement, Point, Run};
use crate::constraints::{self, Rule};
use crate::diylc::{self, Source};
use crate::route::Routing;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the file says it is, and which version of the file format. A change to the format that
/// an older reader would get wrong gets a new version.
const FORMAT: &str = "harmonia-layout";
const VERSION: u32 = 3;

/// Lines in the file are kept to about this many characters (see `pretty`).
const LINE: usize = 100;

// ------------------------------------------------------------ the file, as serde reads and writes it

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)] // so a typo in a hand edit is an error, not silently ignored
struct File {
    format: String,
    version: u32,
    title: String,
    made: Made,
    design: DesignEntry,
    layout: LayoutEntry,
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
struct DesignEntry {
    board: BoardEntry,
    power: Option<String>,
    ground: Option<String>,
    /// The board file's rules, exactly as written.
    rules: Vec<Rule>,
    /// The option this case took in each one-of, like `one-of "USB on a board edge": "left"`.
    choices: Vec<String>,
    parts: Vec<PartEntry>,
    nets: Vec<Net>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardEntry {
    width: i32,
    height: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartEntry {
    reference: String,
    value: String,
    /// The KiCad footprint name, or "DIYLC:..." for a part read from a DIYLC sketch.
    footprint: String,
    /// Width and height in holes, before turning.
    size: (i32, i32),
    /// Which layers above the board it takes, as its footprint or sketch says (a board file's
    /// `layers` rule, in `rules`, may say otherwise).
    layers: Layers,
    /// A module: a board of its own, over its whole outline, standing only on its `body`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    module: bool,
    /// footprints.json's "diylc" entry: which DIYLC part to draw it as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diylc: Option<diylc::Part>,
    pins: Vec<PinEntry>,
    /// The holes the part stands on, inside its outline before turning: only written when it isn't
    /// the whole outline (a module's headers, or a part drawn at an angle).
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
    /// Between holes, like a tactile switch's legs (`board::Pin::off_grid`).
    #[serde(default, skip_serializing_if = "is_false")]
    off_grid: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutEntry {
    parts: Vec<PlacedEntry>,
    runs: Vec<RunEntry>,
}

/// Where a part sits, and the hole each of its pins lands in.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacedEntry {
    reference: String,
    /// Top-left hole after turning, and quarter turns clockwise.
    at: Point,
    rot: u8,
    pins: Vec<HoleEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoleEntry {
    name: String,
    hole: Point,
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
    /// For a branch of a soft star net: where it joined another branch's copper, away from the
    /// star point (each costs the star's weight). Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    joins: Vec<Point>,
}

/// For people reading the file: reading it back works these out again.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Score {
    score: f64,
    cost: f64,
    /// Each of `board::FEATURES`, by name.
    measures: BTreeMap<String, f64>,
}

/// The build as the text of a layout file.
pub fn to_json(build: &Build) -> String {
    let (c, layout) = (&build.design, &build.layout);
    let g = crate::board::geometry(c, &layout.placement);
    let parts = c
        .components
        .iter()
        .map(|comp| PartEntry {
            reference: comp.reference.clone(),
            value: comp.value.clone(),
            footprint: comp.footprint.clone(),
            size: (comp.w, comp.h),
            layers: comp.layers,
            module: comp.module,
            diylc: comp.diylc.clone(),
            pins: comp.pins.iter().map(|pin| PinEntry { name: pin.name.clone(), at: pin.at, off_grid: pin.off_grid }).collect(),
            body: (comp.body != Component::full_body(comp.w, comp.h)).then(|| comp.body.clone()),
            diylc_drawing: comp.source.clone(),
        })
        .collect();
    let placed = c
        .components
        .iter()
        .zip(&layout.placement)
        .enumerate()
        .map(|(ci, (comp, place))| PlacedEntry {
            reference: comp.reference.clone(),
            at: (place.x, place.y),
            rot: place.rot,
            pins: comp.pins.iter().enumerate().map(|(pi, pin)| HoleEntry { name: pin.name.clone(), hole: g.pins[ci][pi] }).collect(),
        })
        .collect();
    // Each run's copper. The traces as straight pieces, the way the drawings show them; each run's
    // pieces come one after the other (see `Build::design`), so picking them out keeps their order.
    let routing = &layout.routing;
    let straight = routing.straight_traces();
    let of_run = |list: &[(usize, Point, Point)], r: usize| list.iter().filter(|t| t.0 == r).map(|&(_, a, b)| [a, b]).collect();
    let runs = c
        .runs
        .iter()
        .enumerate()
        .map(|(r, run)| RunEntry {
            net: c.nets[run.net].name.clone(),
            star: run.star.map(|p| c.pin_name(p)),
            pins: run.pins.iter().map(|&p| c.pin_name(p)).collect(),
            traces: of_run(&straight, r),
            jumpers: of_run(&routing.jumpers, r),
            solder_wires: of_run(&routing.bottom, r),
            unrouted: of_run(&routing.unrouted, r),
            joins: routing.joins.iter().filter(|j| j.0 == r).map(|j| j.1).collect(),
        })
        .collect();
    let m = &build.measures;
    let file = File {
        format: FORMAT.to_string(),
        version: VERSION,
        title: build.title.clone(),
        made: build.made.clone(),
        design: DesignEntry {
            board: BoardEntry { width: c.board.width, height: c.board.height },
            power: c.power.clone(),
            ground: c.ground.clone(),
            rules: c.rules.clone(),
            choices: c.constraints.choices.clone(),
            parts,
            nets: c.nets.clone(),
        },
        layout: LayoutEntry { parts: placed, runs },
        score: Score {
            score: crate::board::score(m),
            cost: crate::board::cost(m),
            measures: crate::board::FEATURES.iter().zip(m.0).map(|(k, v)| (k.to_string(), v)).collect(),
        },
        problems: build.problems.clone(),
    };
    pretty(&serde_json::to_string(&file).expect("a build always turns into JSON"))
}

/// Reads the text of a layout file back into a build, with its copper as saved (nothing is placed
/// or routed again), scored again. Stops with a message if the file isn't a layout file of this
/// version, or disagrees with itself (a pin's `hole` that isn't where its part puts it, or runs
/// that aren't what its design's rules make, say).
pub fn from_json(text: &str) -> Build {
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
    let d = file.design;

    // The design, then the case of its rules the file says it took.
    let components: Vec<Component> = d
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
            layers: p.layers.checked(&format!("layout file: {}", p.reference)),
            module: p.module,
            // Which file each footprint came from is in `made.footprints`; the part's shape is here.
            footprint_from: None,
        })
        .collect();
    let board = BoardSize { width: d.board.width, height: d.board.height };
    if let Some(net) = d.nets.iter().find(|n| n.pins.len() < 2) {
        panic!("layout file: net {} joins fewer than two pins", net.name);
    }
    let unruled = Design::new(board, components, d.nets, d.power, d.ground);
    let (cases, _) = constraints::designs(&unruled, &d.rules);
    let mut design = cases.into_iter().find(|c| c.constraints.choices == d.choices).unwrap_or_else(|| {
        panic!("layout file: its rules have no case that takes {:?}", d.choices)
    });

    // Where each part sits, in the design's order of parts.
    let l = file.layout;
    if l.parts.len() != design.components.len() || l.parts.iter().zip(&design.components).any(|(p, c)| p.reference != c.reference) {
        panic!("layout file: \"layout\" should place the design's parts, in the same order");
    }
    let placement: Placement = l.parts.iter().map(|p| Place { x: p.at.0, y: p.at.1, rot: p.rot }).collect();

    // The runs, in the order the router routes them first, as `Build::routed` keeps them: they must be
    // the ones the design's rules make, or the copper would be on the wrong runs.
    let g = crate::board::geometry(&design, &placement);
    let order = crate::route::order(&design, &g);
    design.runs = order.iter().map(|&r| design.runs[r].clone()).collect();
    let said: Vec<(String, Option<String>, Vec<String>)> = l.runs.iter().map(|e| (e.net.clone(), e.star.clone(), e.pins.clone())).collect();
    let made_by_rules: Vec<(String, Option<String>, Vec<String>)> = design.runs.iter().map(|run: &Run| (design.nets[run.net].name.clone(), run.star.map(|p| design.pin_name(p)), run.pins.iter().map(|&p| design.pin_name(p)).collect())).collect();
    if said != made_by_rules {
        panic!("layout file: its runs aren't the ones its design's rules make");
    }
    let mut routing = Routing::default();
    for (r, entry) in l.runs.iter().enumerate() {
        for &[a, b] in &entry.traces {
            routing.traces.extend(steps(a, b).into_iter().map(|(a, b)| (r, a, b)));
        }
        routing.jumpers.extend(entry.jumpers.iter().map(|&[a, b]| (r, a, b)));
        routing.bottom.extend(entry.solder_wires.iter().map(|&[a, b]| (r, a, b)));
        routing.unrouted.extend(entry.unrouted.iter().map(|&[a, b]| (r, a, b)));
        routing.joins.extend(entry.joins.iter().map(|&at| (r, at)));
    }

    // What's left unrouted is one of its run's pins.
    for &(r, _, pin) in &routing.unrouted {
        if !design.runs[r].pins.iter().any(|&(ci, pi)| g.pins[ci][pi] == pin) {
            panic!("layout file: {} has something unrouted at {pin:?}, which isn't one of its pins", design.nets[design.runs[r].net].name);
        }
    }
    // What the file says twice must agree: each pin's hole.
    for (ci, part) in l.parts.iter().enumerate() {
        for (pi, pin) in part.pins.iter().enumerate() {
            let name = design.pin_name((ci, pi));
            if pin.name != design.components[ci].pins[pi].name || pin.hole != g.pins[ci][pi] {
                panic!("layout file: {name} says it's in hole {:?}, but its part's at, rot and size put it in {:?}", pin.hole, g.pins[ci][pi]);
            }
        }
    }
    Build::of(design, Layout { placement, routing }, file.title, file.made)
}

/// A straight trace from `a` to `b` (across, down or at 45°) as the steps between neighbouring
/// holes the router laid it in.
fn steps(a: Point, b: Point) -> Vec<(Point, Point)> {
    (a != b).then(|| crate::route::unit_steps(a, b)).flatten().unwrap_or_else(|| {
        panic!("layout file: a trace from {a:?} to {b:?} isn't straight across, down or at 45°")
    })
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
