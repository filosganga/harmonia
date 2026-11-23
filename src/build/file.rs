//! The layout file: one plain JSON file that holds a `Build`, what you asked for and what was
//! made of it. `to_json` writes it, `from_json` reads it back.
//!
//!     { "format": "harmonia-layout", "version": 6, "title": "Placed by rules",
//!       "made": { "circuit": "circuits/555_blinker.net", "judge": "rules", "seed": 1, ... },
//!       "design": {
//!         "board": { "width": 18, "height": 12 }, "power": "+5V", "ground": "GND",
//!         "rules": [ { "kind": "edge", "part": "J1", "edge": "left" }, ... ], "choices": [],
//!         "parts": [ { "reference": "U1", "value": "NE555", "footprint": "...", "size": [4, 4],
//!                      "layers": [2, 3], "look": { "as": "round", "plus": "1" },
//!                      "footprint_drawings": { "diylc": { "part": "RadialElectrolytic" } },
//!                      "pins": [ { "name": "1", "at": [0, 3] }, ... ] }, ... ],
//!         "nets": [ { "name": "GND", "pins": ["U1.1", "C1.2", "J1.2"] }, ... ] },
//!       "layout": {
//!         "parts": [ { "reference": "U1", "at": [6, 3], "rot": 0,
//!                      "pins": [ { "name": "1", "hole": [6, 6] }, ... ] }, ... ],
//!         "copper": {
//!           "traces": [ { "layer": -1, "holes": [[6, 6], [6, 9], [8, 9]] }, ... ],
//!           "jumper_wires": [ { "name": "W1", "layer": 2, "pins": [[3, 4], [3, 8]] }, ... ] } },
//!       "score": { "score": 57.0, "cost": 84.6, "measures": { "wire": 41.0, ... } },
//!       "problems": [] }
//!
//! The design is the circuit as read (parts before turning, each pin's hole inside its part, the
//! nets) and the board file's rules exactly as written, with `choices`: the option this case took
//! in each one-of. The layout is where each part sits (`at` and `rot` mean what they mean in a
//! `fixed` rule: top-left hole after turning, quarter turns clockwise), the hole each pin lands in,
//! and the copper: traces, each by the ends of its straight pieces (its two ends and every hole
//! where it changes direction) on its layer, and jumper wires, W1, W2, ..., each with its layer
//! and the hole of each pin. No runs and no nets: which pins the copper joins is worked out from it.
//!
//! The score and the problems are written for people to read. Reading the file back works them
//! out again from the design and the layout, with today's measurements.

use super::{Build, Made};
use crate::board::{Board, COPPER, Face, LYING, Layer, Layers, Place, Placement, Point};
use crate::circuit::{Circuit, Drawings, Footprint, Look, Net, Part, Pin};
use crate::layout::{Copper, JumperWire, Layout, Trace, unit_steps};
use crate::rules::{self, Rule};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the file says it is, and which version of the file format. A change to the format that
/// an older reader would get wrong gets a new version.
const FORMAT: &str = "harmonia-layout";
const VERSION: u32 = 6;

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
    /// Which layers it takes, counted from its face (`board::Layers`), as its footprint or sketch
    /// says (a board file's `layers` rule, in `rules`, may say otherwise).
    layers: Layers,
    /// What its footprint looks like in the pictures: left out for a box.
    #[serde(default, skip_serializing_if = "is_box")]
    look: Look,
    /// How file formats draw its footprint (footprints.json's "diylc" entry), by format.
    #[serde(default, skip_serializing_if = "Drawings::is_empty")]
    footprint_drawings: Drawings,
    pins: Vec<PinEntry>,
    /// The holes its body covers, inside its outline before turning: only written when it isn't the
    /// whole outline (a part drawn at an angle, or a DIYLC group that isn't a board on headers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Vec<Point>>,
    /// How file formats draw the part itself, by format: for a part read from a DIYLC sketch, your
    /// drawing of it, written back moved and turned.
    #[serde(default, skip_serializing_if = "Drawings::is_empty")]
    drawings: Drawings,
}

fn is_box(look: &Look) -> bool {
    *look == Look::Box
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
    copper: CopperEntry,
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
struct CopperEntry {
    traces: Vec<TraceEntry>,
    jumper_wires: Vec<WireEntry>,
}

/// A trace: its layer (-1, the bottom face's copper), and its holes: its two ends and every hole
/// where it changes direction, so each piece between two is straight across, down or at 45°.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceEntry {
    layer: Layer,
    holes: Vec<Point>,
}

/// A jumper wire: its name (W1, W2, ... in order), its layer (2 on the top face, -2 on the bottom
/// one), and the hole of each of its two pins.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEntry {
    name: String,
    layer: Layer,
    pins: [Point; 2],
}

/// For people reading the file: reading it back works these out again.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Score {
    score: f64,
    cost: f64,
    /// Each of `measure::FEATURES`, by name.
    measures: BTreeMap<String, f64>,
}

/// The build as the text of a layout file.
pub fn to_json(build: &Build) -> String {
    let (c, layout) = (&build.case, &build.layout);
    let circuit = &c.circuit;
    let g = c.geometry(&layout.placement);
    let parts = circuit
        .parts
        .iter()
        .map(|comp| PartEntry {
            reference: comp.reference.clone(),
            value: comp.value.clone(),
            footprint: comp.footprint.name.clone(),
            size: (comp.footprint.w, comp.footprint.h),
            layers: comp.layers,
            look: comp.footprint.look.clone(),
            footprint_drawings: comp.footprint.drawings.clone(),
            pins: comp.footprint.pins.iter().map(|pin| PinEntry { name: pin.name.clone(), at: pin.at, off_grid: pin.off_grid }).collect(),
            body: (comp.footprint.body != Footprint::full_body(comp.footprint.w, comp.footprint.h)).then(|| comp.footprint.body.clone()),
            drawings: comp.drawings.clone(),
        })
        .collect();
    let placed = circuit
        .parts
        .iter()
        .zip(&layout.placement)
        .enumerate()
        .map(|(ci, (comp, place))| PlacedEntry {
            reference: comp.reference.clone(),
            at: (place.x, place.y),
            rot: place.rot,
            pins: comp.footprint.pins.iter().enumerate().map(|(pi, pin)| HoleEntry { name: pin.name.clone(), hole: g.pins[ci][pi] }).collect(),
        })
        .collect();
    // The copper: each trace by the ends of its straight pieces, the wires by name.
    let copper = &layout.copper;
    let traces = copper.traces.iter().map(|t| TraceEntry { layer: t.layer, holes: t.straight_ends() }).collect();
    let jumper_wires = copper.wires.iter().enumerate().map(|(k, w)| WireEntry { name: JumperWire::name(k), layer: w.layer, pins: w.pins }).collect();
    let m = &build.measures;
    let file = File {
        format: FORMAT.to_string(),
        version: VERSION,
        title: build.title.clone(),
        made: build.made.clone(),
        design: DesignEntry {
            board: BoardEntry { width: c.board.width, height: c.board.height },
            power: circuit.power.clone(),
            ground: circuit.ground.clone(),
            rules: c.rules.clone(),
            choices: c.constraints.choices.clone(),
            parts,
            nets: circuit.nets.clone(),
        },
        layout: LayoutEntry { parts: placed, copper: CopperEntry { traces, jumper_wires } },
        score: Score {
            score: crate::measure::score(m),
            cost: crate::measure::cost(m),
            measures: crate::measure::FEATURES.iter().zip(m.0).map(|(k, v)| (k.to_string(), v)).collect(),
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
    let parts: Vec<Part> = d
        .parts
        .iter()
        .map(|p| Part {
            reference: p.reference.clone(),
            value: p.value.clone(),
            footprint: Footprint {
                name: p.footprint.clone(),
                w: p.size.0,
                h: p.size.1,
                pins: p.pins.iter().map(|pin| Pin { name: pin.name.clone(), at: pin.at, off_grid: pin.off_grid }).collect(),
                body: p.body.clone().unwrap_or_else(|| Footprint::full_body(p.size.0, p.size.1)),
                look: p.look.clone(),
                drawings: p.footprint_drawings.clone(),
            },
            drawings: p.drawings.clone(),
            layers: p.layers.checked(&format!("layout file: {}", p.reference)),
            // Which file each footprint came from is in `made.footprints`; the part's shape is here.
            footprint_from: None,
        })
        .collect();
    let board = Board { width: d.board.width, height: d.board.height };
    if let Some(net) = d.nets.iter().find(|n| n.pins.len() < 2) {
        panic!("layout file: net {} joins fewer than two pins", net.name);
    }
    let circuit = Circuit::new(parts, d.nets).with_power_and_ground(d.power, d.ground);
    let (cases, _) = rules::cases(&circuit, board, &d.rules);
    let case = cases.into_iter().find(|c| c.constraints.choices == d.choices).unwrap_or_else(|| {
        panic!("layout file: its rules have no case that takes {:?}", d.choices)
    });

    // Where each part sits, in the case's order of parts.
    let l = file.layout;
    if l.parts.len() != case.circuit.parts.len() || l.parts.iter().zip(&case.circuit.parts).any(|(p, c)| p.reference != c.reference) {
        panic!("layout file: \"layout\" should place the design's parts, in the same order");
    }
    let placement: Placement = l.parts.iter().map(|p| Place { face: Face::Top, x: p.at.0, y: p.at.1, rot: p.rot }).collect();

    let g = case.geometry(&placement);
    // The copper, each trace's listed holes back to every hole it touches.
    let traces = l.copper.traces.iter().map(|t| {
        if t.layer != -COPPER || t.holes.len() < 2 {
            panic!("layout file: a trace should be on layer {} with two holes or more, not on {} with {}", -COPPER, t.layer, t.holes.len());
        }
        let mut holes = vec![t.holes[0]];
        for w in t.holes.windows(2) {
            holes.extend(steps(w[0], w[1]).into_iter().map(|(_, b)| b));
        }
        Trace { layer: t.layer, holes }
    });
    let wires = l.copper.jumper_wires.iter().enumerate().map(|(k, w)| {
        if w.name != JumperWire::name(k) || w.layer.abs() != LYING || w.pins[0] == w.pins[1] {
            panic!("layout file: jumper wire {} should be called {}, on layer {LYING} or -{LYING}, with its pins in two holes", w.name, JumperWire::name(k));
        }
        JumperWire { layer: w.layer, pins: w.pins }
    });
    let copper = Copper { traces: traces.collect(), wires: wires.collect() };

    // What the file says twice must agree: each pin's hole.
    for (ci, part) in l.parts.iter().enumerate() {
        for (pi, pin) in part.pins.iter().enumerate() {
            let name = case.circuit.pin_name((ci, pi));
            if pin.name != case.circuit.parts[ci].footprint.pins[pi].name || pin.hole != g.pins[ci][pi] {
                panic!("layout file: {name} says it's in hole {:?}, but its part's at, rot and size put it in {:?}", pin.hole, g.pins[ci][pi]);
            }
        }
    }
    Build::of(case, Layout { placement, copper }, file.title, file.made)
}

/// A straight trace from `a` to `b` (across, down or at 45°) as the steps between neighbouring
/// holes the router laid it in.
fn steps(a: Point, b: Point) -> Vec<(Point, Point)> {
    (a != b).then(|| unit_steps(a, b)).flatten().unwrap_or_else(|| {
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
