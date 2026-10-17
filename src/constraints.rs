//! The board file: which perfboard you're building on and your own rules for it.
//!
//! It sits next to the netlist with the same name (circuits/555_blinker.net ->
//! circuits/555_blinker.json) and every part of it is optional:
//!
//!     {
//!       "board": "18x12",
//!       "power": "+5V", "ground": "GND",
//!       "constraints": [
//!         { "kind": "fixed",   "part": "U1", "at": [6, 3], "rot": 0 },
//!         { "kind": "edge",    "part": "J1", "edge": "left" },
//!         { "kind": "keepout", "name": "M3 screw", "from": [16, 0], "to": [17, 1] },
//!         { "kind": "near",    "pins": ["C2.1", "U1.5"] },
//!         { "kind": "star",    "name": "ground at C1", "at": "C1.2", "runs": [["U1.1", "D1.1"]] }
//!       ]
//!     }
//!
//! Every rule says its "kind", and may have a "name": it's in the messages about that rule (and a
//! keep-out's is written on it in the pictures).
//!
//! fixed and edge are hard limits: the placer never moves a fixed part and always keeps an edge
//! part touching its edge. A keep-out counts like a collision (the "overlap" measurement): very
//! expensive, but for now a cost, not a limit.
//!
//! near is a preference: the two pins, on the same net, should be joined by as little copper as
//! possible. It costs per hole of copper between them, as routed (through the star point, for two
//! runs of a star net).
//!
//! star says how a net (the one its `at` pin is on) is wired: every run starts at the `at` pin and
//! goes its own way, never sharing copper with another run. Each run in `runs` joins its pins; any
//! other pin of the net gets a run of its own, straight from `at`.

use crate::board::{self, Circuit, Place, Placement, Point, Run};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)] // so a typo like "constraint" is an error, not silently ignored
pub struct BoardFile {
    pub board: Option<String>,
    pub power: Option<String>,
    pub ground: Option<String>,
    #[serde(default)]
    constraints: Vec<Rule>,
}

/// One entry of "constraints". `kind` in the JSON picks the variant, like { "kind": "near", ... }.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Rule {
    /// The part goes exactly there and never moves.
    Fixed {
        name: Option<String>,
        part: String,
        /// Top-left hole of the part, after rotating.
        at: Point,
        /// Quarter turns clockwise, 0-3, like the placer uses.
        #[serde(default)]
        rot: u8,
    },
    /// The part always touches that edge of the board.
    Edge { name: Option<String>, part: String, edge: Edge },
    /// Holes no part may cover.
    Keepout {
        name: Option<String>,
        from: Point,
        to: Point,
    },
    /// Two pins of the same net, like ["R1.2", "U1.8"], joined by as little copper as possible.
    Near { name: Option<String>, pins: [String; 2] },
    /// How the net of the `at` pin is wired.
    Star {
        name: Option<String>,
        /// The pin every run starts from, like "C1.+".
        at: String,
        /// Pins that share a run, like [["U1.8", "J2.1"], ["U2.4"]].
        #[serde(default)]
        runs: Vec<Vec<String>>,
    },
}

/// A star rule, matched up with the circuit: pins are (component, pin) indexes.
pub struct StarRule {
    pub net: usize,
    pub at: (usize, usize),
    pub runs: Vec<Vec<(usize, usize)>>,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// A rectangle of holes no part may cover, `from` one corner `to` the other (both included).
#[derive(Clone)]
pub struct Keepout {
    pub from: Point,
    pub to: Point,
    /// The rule's name, like "M3 screw", or empty.
    pub name: String,
}

impl Keepout {
    pub fn holes(&self) -> impl Iterator<Item = Point> {
        let (x0, x1) = (self.from.0.min(self.to.0), self.from.0.max(self.to.0));
        let (y0, y1) = (self.from.1.min(self.to.1), self.from.1.max(self.to.1));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }
}

/// The board file's rules, matched up with the circuit's parts.
#[derive(Default)]
pub struct Constraints {
    /// fixed[component] = where it must be, if it's fixed
    pub fixed: Vec<Option<Place>>,
    /// edge[component] = the edge it must touch, if any
    pub edge: Vec<Option<Edge>>,
    pub keepout: Vec<Keepout>,
    /// Every hole of every keep-out, for quick lookups.
    pub keepout_holes: HashSet<Point>,
    /// (net, two of its pins as (component, pin)): join them by as little copper as possible.
    pub near: Vec<(usize, [(usize, usize); 2])>,
    pub stars: Vec<StarRule>,
}

/// Reads the board file next to `netlist_path`, if there is one. Returns its path too.
pub fn read_board_file(netlist_path: &str) -> Option<(String, BoardFile)> {
    let path = std::path::Path::new(netlist_path).with_extension("json");
    let path = path.to_string_lossy().into_owned();
    let text = std::fs::read_to_string(&path).ok()?;
    let file = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad board file {path}: {e}"));
    Some((path, file))
}

/// How messages name a rule: `near "R1 guards GP2"`, or just `near` if it has no name.
fn called(kind: &str, name: &Option<String>) -> String {
    match name {
        Some(n) => format!("{kind} \"{n}\""),
        None => kind.to_string(),
    }
}

/// Matches the board file's part names to the circuit and checks the rules make sense.
pub fn resolve(file: &BoardFile, c: &Circuit) -> Constraints {
    let index = |name: &str| {
        c.components.iter().position(|comp| comp.reference == name).unwrap_or_else(|| panic!("board file: there's no part called {name}"))
    };
    // The net a pin is on, if it's connected to anything.
    let net_of = |p: (usize, usize)| c.net_pins.iter().position(|pins| pins.contains(&p));
    let n = c.components.len();
    let (w, h) = (c.board.width, c.board.height);
    let mut out = Constraints { fixed: vec![None; n], edge: vec![None; n], ..Default::default() };
    for rule in &file.constraints {
        match rule {
            Rule::Fixed { name, part, at, rot } => {
                let rule = called("fixed", name);
                if *rot > 3 {
                    panic!("board file: {rule}: {part}'s \"rot\" should be 0-3 (quarter turns clockwise)");
                }
                let i = index(part);
                if out.fixed[i].is_some() {
                    panic!("board file: {rule}: {part} is already fixed by another rule");
                }
                out.fixed[i] = Some(Place { x: at.0, y: at.1, rot: *rot });
            }
            Rule::Edge { name, part, edge } => {
                let i = index(part);
                if out.edge[i].is_some() {
                    panic!("board file: {}: {part} already has an edge rule", called("edge", name));
                }
                out.edge[i] = Some(*edge);
            }
            Rule::Keepout { name, from, to } => {
                let k = Keepout { from: *from, to: *to, name: name.clone().unwrap_or_default() };
                if k.holes().any(|(x, y)| x < 0 || x >= w || y < 0 || y >= h) {
                    panic!("board file: {}: {from:?} to {to:?} goes off the {w}x{h} board", called("keepout", name));
                }
                out.keepout_holes.extend(k.holes());
                out.keepout.push(k);
            }
            Rule::Near { name, pins: [a, b] } => {
                let rule = called("near", name);
                let (pa, pb) = (c.find_pin(a), c.find_pin(b));
                if pa == pb {
                    panic!("board file: {rule}: both pins are {a}");
                }
                // Near is measured along the copper, so there has to be copper between them.
                let net = net_of(pa).filter(|&net| net_of(pb) == Some(net));
                let net = net.unwrap_or_else(|| panic!("board file: {rule}: {a} and {b} aren't on the same net, so no copper joins them"));
                out.near.push((net, [pa, pb]));
            }
            Rule::Star { name, at, runs } => {
                let rule = called("star", name);
                let at_pin = c.find_pin(at);
                let net = net_of(at_pin).unwrap_or_else(|| panic!("board file: {rule}: {at} isn't connected to anything"));
                let net_name = &c.nets[net].name;
                if out.stars.iter().any(|s| s.net == net) {
                    panic!("board file: {rule}: {net_name} already has a star rule");
                }
                let on_net = |pin: &str| {
                    let p = c.find_pin(pin);
                    if !c.net_pins[net].contains(&p) {
                        panic!("board file: {rule}: {pin} isn't on {net_name}, the net of {at}");
                    }
                    p
                };
                let resolved: Vec<Vec<(usize, usize)>> = runs.iter().map(|run| run.iter().map(|pin| on_net(pin)).collect()).collect();
                let mut seen = HashSet::from([at_pin]);
                for (pin, p) in runs.iter().flatten().zip(resolved.iter().flatten()) {
                    if !seen.insert(*p) {
                        panic!("board file: {rule}: {pin} is the star point or in two runs");
                    }
                }
                out.stars.push(StarRule { net, at: at_pin, runs: resolved });
            }
        }
    }
    for i in 0..n {
        if out.fixed[i].is_some() && out.edge[i].is_some() {
            panic!("board file: {} is both fixed and on an edge; pick one", c.components[i].reference);
        }
    }
    // A fixed part that doesn't fit can never be fixed by the placer, so say so now.
    for (i, f) in out.fixed.iter().enumerate() {
        if let Some(p) = f {
            let (pw, ph) = board::size(&c.components[i], p.rot);
            let name = &c.components[i].reference;
            if p.x < 0 || p.y < 0 || p.x + pw > w || p.y + ph > h {
                panic!("board file: fixed {name} at {:?} hangs off the {w}x{h} board", (p.x, p.y));
            }
            let mut body = (p.x..p.x + pw).flat_map(|x| (p.y..p.y + ph).map(move |y| (x, y)));
            if body.any(|hole| out.keepout_holes.contains(&hole)) {
                panic!("board file: fixed {name} sits on a keep-out");
            }
        }
    }
    out
}

/// How each net is wired: one run joining all its pins, or for a star net, one run per branch
/// from the star point. Nets with a single pin have nothing to join.
pub fn runs(c: &Circuit) -> Vec<Run> {
    let mut out = Vec::new();
    for (net, pins) in c.net_pins.iter().enumerate().filter(|(_, pins)| pins.len() > 1) {
        match c.constraints.stars.iter().find(|s| s.net == net) {
            Some(star) => {
                let listed: HashSet<(usize, usize)> = star.runs.iter().flatten().copied().collect();
                let alone = pins.iter().filter(|&&p| p != star.at && !listed.contains(&p)).map(|&p| vec![p]);
                for run in star.runs.iter().cloned().chain(alone) {
                    out.push(Run { net, pins: std::iter::once(star.at).chain(run).collect(), star: Some(star.at) });
                }
            }
            None => out.push(Run { net, pins: pins.clone(), star: None }),
        }
    }
    out
}

/// Puts fixed parts in their place and pushes edge parts against their edge.
/// The placer calls this after every move, so these rules can never be broken.
pub fn apply(c: &Circuit, placement: &mut Placement) {
    let (w, h) = (c.board.width, c.board.height);
    for (i, p) in placement.iter_mut().enumerate() {
        if let Some(f) = c.constraints.fixed[i] {
            *p = f;
        }
        if let Some(e) = c.constraints.edge[i] {
            let (pw, ph) = board::size(&c.components[i], p.rot);
            match e {
                Edge::Left => p.x = 0,
                Edge::Right => p.x = w - pw,
                Edge::Top => p.y = 0,
                Edge::Bottom => p.y = h - ph,
            }
        }
    }
}

/// Plain-language problems with a finished layout. Fixed and edge rules can't break, so only keep-outs are checked.
pub fn problems(c: &Circuit, placement: &[Place]) -> Vec<String> {
    let g = board::geometry(c, placement);
    let mut out = Vec::new();
    for (comp, body) in c.components.iter().zip(&g.bodies) {
        for k in &c.constraints.keepout {
            let holes: HashSet<Point> = k.holes().collect();
            if body.iter().any(|p| holes.contains(p)) {
                let name = if k.name.is_empty() { String::new() } else { format!(" ({})", k.name) };
                out.push(format!("{} is on the keep-out at {:?}-{:?}{name}", comp.reference, k.from, k.to));
            }
        }
    }
    out
}
