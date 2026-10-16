//! The board file: which perfboard you're building on and your own rules for it.
//!
//! It sits next to the netlist with the same name (circuits/555_blinker.net ->
//! circuits/555_blinker.json) and every part of it is optional:
//!
//!     {
//!       "board": "18x12",
//!       "power": "+5V", "ground": "GND",
//!       "fixed":   { "U1": { "at": [6, 3], "rot": 0 } },
//!       "edge":    { "J1": "left" },
//!       "keepout": [ { "from": [0, 0], "to": [1, 1], "why": "M3 screw" } ],
//!       "near":    [ { "part": "C2", "pin": "U1.8" } ],
//!       "star":    { "GND": { "at": "C1.2", "runs": [["U1.1", "D1.1"]] } }
//!     }
//!
//! fixed, edge and keepout are hard limits, not preferences: the placer never moves a fixed
//! part, always keeps an edge part touching its edge, and treats keep-out holes as taken.
//!
//! near is a preference: the part should be as close as it can to that pin. It costs per hole.
//!
//! star says how a net is wired: every run starts at the `at` pin and goes its own way, never
//! sharing copper with another run. Each run in `runs` joins its pins; any other pin of the net
//! gets a run of its own, straight from `at`.

use crate::board::{self, Circuit, Place, Placement, Point, Run};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)] // so a typo like "fixd" is an error, not silently ignored
pub struct BoardFile {
    pub board: Option<String>,
    pub power: Option<String>,
    pub ground: Option<String>,
    #[serde(default)]
    fixed: HashMap<String, Fixed>,
    #[serde(default)]
    edge: HashMap<String, Edge>,
    #[serde(default)]
    keepout: Vec<Keepout>,
    #[serde(default)]
    near: Vec<Near>,
    #[serde(default)]
    star: HashMap<String, Star>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Near {
    part: String,
    /// A pin, like "U1.8".
    pin: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Star {
    /// The pin every run starts from, like "C1.+".
    at: String,
    /// Pins that share a run, like [["U1.8", "J2.1"], ["U2.4"]].
    #[serde(default)]
    runs: Vec<Vec<String>>,
}

/// A star rule, matched up with the circuit: pins are (component, pin) indexes.
pub struct StarRule {
    pub net: usize,
    pub at: (usize, usize),
    pub runs: Vec<Vec<(usize, usize)>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixed {
    /// Top-left hole of the part, after rotating.
    at: Point,
    /// Quarter turns clockwise, 0-3, like the placer uses.
    #[serde(default)]
    rot: u8,
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
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Keepout {
    pub from: Point,
    pub to: Point,
    #[serde(default)]
    pub why: String,
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
    /// (part, pin) pairs: the part should sit as close as it can to the pin.
    pub near: Vec<(usize, (usize, usize))>,
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

/// Matches the board file's part names to the circuit and checks the rules make sense.
pub fn resolve(file: &BoardFile, c: &Circuit) -> Constraints {
    let index = |name: &str| {
        c.components.iter().position(|comp| comp.reference == name).unwrap_or_else(|| panic!("board file: there's no part called {name}"))
    };
    let n = c.components.len();
    let mut out = Constraints { fixed: vec![None; n], edge: vec![None; n], ..Default::default() };
    for (name, f) in &file.fixed {
        if f.rot > 3 {
            panic!("board file: {name}'s \"rot\" should be 0-3 (quarter turns clockwise)");
        }
        out.fixed[index(name)] = Some(Place { x: f.at.0, y: f.at.1, rot: f.rot });
    }
    for (name, &e) in &file.edge {
        let i = index(name);
        if out.fixed[i].is_some() {
            panic!("board file: {name} is both fixed and on an edge; pick one");
        }
        out.edge[i] = Some(e);
    }
    let (w, h) = (c.board.width, c.board.height);
    for k in &file.keepout {
        if k.holes().any(|(x, y)| x < 0 || x >= w || y < 0 || y >= h) {
            panic!("board file: keep-out {:?} to {:?} goes off the {w}x{h} board", k.from, k.to);
        }
        out.keepout_holes.extend(k.holes());
    }
    out.keepout = file.keepout.clone();
    for near in &file.near {
        out.near.push((index(&near.part), c.find_pin(&near.pin)));
    }
    for (net_name, star) in &file.star {
        let net = c.nets.iter().position(|n| &n.name == net_name).unwrap_or_else(|| panic!("board file: star: there's no net called {net_name}"));
        let on_net = |pin: &str| {
            let p = c.find_pin(pin);
            if !c.net_pins[net].contains(&p) {
                panic!("board file: star {net_name}: {pin} isn't on {net_name}");
            }
            p
        };
        let at = on_net(&star.at);
        let runs: Vec<Vec<(usize, usize)>> = star.runs.iter().map(|run| run.iter().map(|pin| on_net(pin)).collect()).collect();
        let mut seen = HashSet::from([at]);
        for (pin, p) in star.runs.iter().flatten().zip(runs.iter().flatten()) {
            if !seen.insert(*p) {
                panic!("board file: star {net_name}: {pin} is the star point or in two runs");
            }
        }
        out.stars.push(StarRule { net, at, runs });
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
                let why = if k.why.is_empty() { String::new() } else { format!(" ({})", k.why) };
                out.push(format!("{} is on the keep-out at {:?}-{:?}{why}", comp.reference, k.from, k.to));
            }
        }
    }
    out
}
