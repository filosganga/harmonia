//! The board file: which perfboard you're building on and your own rules for it.
//!
//! It sits next to the netlist with the same name (circuits/555_blinker.net ->
//! circuits/555_blinker.json) and every part of it is optional:
//!
//!     {
//!       "board": "18x12",
//!       "power": "+5V", "ground": "GND",
//!       "constraints": [
//!         { "kind": "fixed",       "part": "U1", "at": [6, 3], "rot": 0 },
//!         { "kind": "edge",        "part": "J1", "edge": "left" },
//!         { "kind": "orientation", "part": "C1", "rot": 1 },
//!         { "kind": "within",      "part": "C1", "from": [0, 6], "to": [5, 11] },
//!         { "kind": "one-of", "name": "R3 on the left or right edge", "rules": [
//!           { "kind": "all-of", "rules": [
//!             { "kind": "orientation", "part": "R3", "rot": 1 },
//!             { "kind": "within", "part": "R3", "from": [0, 0], "to": [0, 11] } ] },
//!           { "kind": "all-of", "rules": [
//!             { "kind": "orientation", "part": "R3", "rot": 3 },
//!             { "kind": "within", "part": "R3", "from": [17, 0], "to": [17, 11] } ] } ] },
//!         { "kind": "keepout", "name": "M3 screw", "from": [16, 0], "to": [17, 1] },
//!         { "kind": "near",    "pins": ["C2.1", "U1.5"] },
//!         { "kind": "close",   "pins": ["D1.2", "U1.3"] },
//!         { "kind": "star",    "name": "ground at C1", "at": "C1.2", "runs": [["U1.1", "D1.1"]] }
//!       ]
//!     }
//!
//! Every rule says its "kind", and may have a "name": it's in the messages about that rule (and a
//! keep-out's is written on it in the pictures).
//!
//! all-of and one-of combine any rules: all-of holds when every rule in it does, one-of when at
//! least one does. The board file is read as a list of cases, each a plain list of rules: a one-of
//! makes one case per option, so "A, one-of (B1, B2), C" is two cases, (A, B1, C) and (A, B2, C).
//! The placer places every case and keeps the best layout. A one-of only about where one part goes
//! (fixed, edge, orientation, within, about the same part) makes no cases: the part may take any
//! place one of its options allows, in the same run.
//!
//! fixed, edge, orientation and within say where one part may go, and are hard limits. In each case,
//! each part with such rules gets the list of places (x, y, rot) they allow, and the placer only
//! ever puts the part on one of them. Those places are all on the board and off the keep-outs too.
//! A case where some part has no such place is left out.
//!
//! A keep-out counts like a collision (the "overlap" measurement): very expensive, but for now a
//! cost, not a limit, except for a part with placement rules (above).
//!
//! near is a preference: the two pins, on the same net, should be joined by as little copper as
//! possible. It costs per hole of copper between them, as routed (through the star point, for two
//! runs of a star net). close is the same for two pins of any nets, measured straight across the
//! board: for a connection the board doesn't carry, like a wire from a module's underside.
//!
//! star says how a net (the one its `at` pin is on) is wired: every run starts at the `at` pin and
//! goes its own way, never sharing copper with another run. Each run in `runs` joins its pins; any
//! other pin of the net gets a run of its own, straight from `at`.

use crate::board::{self, Circuit, Component, Place, Placement, Point, Run};
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
    /// The part touches that edge of the board.
    Edge { name: Option<String>, part: String, edge: Edge },
    /// The part is turned this way: quarter turns clockwise from how it's drawn, 0-3.
    Orientation { name: Option<String>, part: String, rot: u8 },
    /// Every hole of the part's outline is in this rectangle, `from` one corner `to` the other.
    Within {
        name: Option<String>,
        part: String,
        from: Point,
        to: Point,
    },
    /// Every rule in `rules` holds.
    #[serde(rename = "all-of")]
    AllOf { name: Option<String>, rules: Vec<Rule> },
    /// At least one rule in `rules` holds: each is a case of its own.
    #[serde(rename = "one-of")]
    OneOf { name: Option<String>, rules: Vec<Rule> },
    /// Holes no part may cover.
    Keepout {
        name: Option<String>,
        from: Point,
        to: Point,
    },
    /// Two pins of the same net, like ["R1.2", "U1.8"], joined by as little copper as possible.
    Near { name: Option<String>, pins: [String; 2] },
    /// Two pins of any nets as close together as possible, straight across the board.
    Close { name: Option<String>, pins: [String; 2] },
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

impl Rule {
    /// The rule's kind, as in the JSON, and its name if it has one.
    fn kind_and_name(&self) -> (&'static str, Option<&str>) {
        let (kind, name) = match self {
            Rule::Fixed { name, .. } => ("fixed", name),
            Rule::Edge { name, .. } => ("edge", name),
            Rule::Orientation { name, .. } => ("orientation", name),
            Rule::Within { name, .. } => ("within", name),
            Rule::AllOf { name, .. } => ("all-of", name),
            Rule::OneOf { name, .. } => ("one-of", name),
            Rule::Keepout { name, .. } => ("keepout", name),
            Rule::Near { name, .. } => ("near", name),
            Rule::Close { name, .. } => ("close", name),
            Rule::Star { name, .. } => ("star", name),
        };
        (kind, name.as_deref())
    }

    /// How messages name a rule: `near "R1 guards GP2"`, or just `near` if it has no name.
    fn called(&self) -> String {
        match self.kind_and_name() {
            (kind, Some(n)) => format!("{kind} \"{n}\""),
            (kind, None) => kind.to_string(),
        }
    }

    /// The part a rule is only about where it goes: fixed, edge, orientation and within, and an
    /// all-of or one-of of those about the same part. None for any other rule.
    fn one_part(&self) -> Option<&str> {
        match self {
            Rule::Fixed { part, .. } | Rule::Edge { part, .. } | Rule::Orientation { part, .. } | Rule::Within { part, .. } => Some(part),
            Rule::AllOf { rules, .. } | Rule::OneOf { rules, .. } => {
                let first = rules.first()?.one_part()?;
                rules.iter().all(|r| r.one_part() == Some(first)).then_some(first)
            }
            Rule::Keepout { .. } | Rule::Near { .. } | Rule::Close { .. } | Rule::Star { .. } => None,
        }
    }

    /// For a rule about where one part goes (see `one_part`), that part; None for the others.
    /// Also checks the rule makes sense on a w x h board, whatever the part.
    fn part(&self, w: i32, h: i32) -> Option<&str> {
        let rule = self.called();
        match self {
            Rule::Fixed { part, rot, .. } | Rule::Orientation { part, rot, .. } => {
                if *rot > 3 {
                    panic!("board file: {rule}: {part}'s \"rot\" should be 0-3 (quarter turns clockwise)");
                }
                Some(part)
            }
            Rule::Edge { part, .. } => Some(part),
            Rule::Within { part, from, to, .. } => {
                if rectangle(*from, *to).any(|(x, y)| x < 0 || x >= w || y < 0 || y >= h) {
                    panic!("board file: {rule}: {from:?} to {to:?} goes off the {w}x{h} board");
                }
                Some(part)
            }
            Rule::AllOf { rules, .. } | Rule::OneOf { rules, .. } => {
                rules.iter().for_each(|r| _ = r.part(w, h));
                self.one_part()
            }
            Rule::Keepout { .. } | Rule::Near { .. } | Rule::Close { .. } | Rule::Star { .. } => None,
        }
    }

    /// Does this rule let `comp` sit at `p` on a w x h board? Only for rules with a part (see `part`).
    fn allows(&self, comp: &Component, p: Place, w: i32, h: i32) -> bool {
        let (pw, ph) = board::size(comp, p.rot);
        match self {
            Rule::Fixed { at, rot, .. } => (p.x, p.y, p.rot) == (at.0, at.1, *rot),
            Rule::Edge { edge, .. } => match edge {
                Edge::Left => p.x == 0,
                Edge::Right => p.x + pw == w,
                Edge::Top => p.y == 0,
                Edge::Bottom => p.y + ph == h,
            },
            Rule::Orientation { rot, .. } => p.rot == *rot,
            Rule::Within { from, to, .. } => {
                let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                p.x >= x0 && p.x + pw - 1 <= x1 && p.y >= y0 && p.y + ph - 1 <= y1
            }
            Rule::AllOf { rules, .. } => rules.iter().all(|r| r.allows(comp, p, w, h)),
            Rule::OneOf { rules, .. } => rules.iter().any(|r| r.allows(comp, p, w, h)),
            _ => unreachable!("not about where a part goes"),
        }
    }
}

/// One way for all the board file's rules to hold: a plain list of rules (no all-of or one-of), and
/// for messages, the option it took in each one-of on the way.
#[derive(Default)]
pub struct Case<'a> {
    rules: Vec<&'a Rule>,
    pub choices: Vec<String>,
}

/// The board file's rules as cases: one, unless a one-of makes more (see `rule_cases`).
pub fn cases(file: &BoardFile) -> Vec<Case<'_>> {
    every_case(&file.constraints)
}

/// Every way a list of rules can all hold: each case of the first rule with each case of the rest.
/// (Two one-ofs of 2 and 3 options make 2 x 3 = 6 cases.)
fn every_case(rules: &[Rule]) -> Vec<Case<'_>> {
    let mut cases = vec![Case::default()];
    for rule in rules {
        let options = rule_cases(rule);
        cases = cases
            .iter()
            .flat_map(|case| {
                options.iter().map(|option| Case {
                    rules: case.rules.iter().chain(&option.rules).copied().collect(),
                    choices: case.choices.iter().chain(&option.choices).cloned().collect(),
                })
            })
            .collect();
    }
    cases
}

/// The cases of one rule: itself, unless it's an all-of or a one-of.
fn rule_cases(rule: &Rule) -> Vec<Case<'_>> {
    match rule {
        Rule::AllOf { rules, .. } | Rule::OneOf { rules, .. } if rules.is_empty() => {
            panic!("board file: {} has no rules in it", rule.called())
        }
        // A one-of only about where one part goes makes no cases: the part may take any place one
        // of its options allows (see `allows`), so its options are all tried in the same run.
        Rule::OneOf { .. } if rule.one_part().is_some() => vec![Case { rules: vec![rule], choices: Vec::new() }],
        Rule::AllOf { rules, .. } => every_case(rules),
        Rule::OneOf { rules, .. } => {
            let mut out = Vec::new();
            for (k, option) in rules.iter().enumerate() {
                // Name the option by its own name if it has one, else by its number.
                let which = match option.kind_and_name().1 {
                    Some(name) => format!("\"{name}\""),
                    None => format!("option {} of {}", k + 1, rules.len()),
                };
                for mut case in rule_cases(option) {
                    case.choices.insert(0, format!("{}: {which}", rule.called()));
                    out.push(case);
                }
            }
            out
        }
        _ => vec![Case { rules: vec![rule], choices: Vec::new() }],
    }
}

/// A star rule, matched up with the circuit: pins are (component, pin) indexes.
#[derive(Clone)]
pub struct StarRule {
    pub net: usize,
    pub at: (usize, usize),
    pub runs: Vec<Vec<(usize, usize)>>,
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Every hole of a rectangle, `from` one corner `to` the other (both included).
fn rectangle(from: Point, to: Point) -> impl Iterator<Item = Point> {
    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
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
        rectangle(self.from, self.to)
    }
}

/// One case of the board file's rules, matched up with the circuit's parts.
#[derive(Default, Clone)]
pub struct Constraints {
    /// Which option of each one-of this case took, like `one-of "Pads by the ESP32's back pads": "by GP5"`.
    pub choices: Vec<String>,
    /// allowed[component] = the only places it may take, if the board file has rules about where
    /// it goes (fixed, edge, orientation, within). Never empty.
    pub allowed: Vec<Option<Vec<Place>>>,
    pub keepout: Vec<Keepout>,
    /// Every hole of every keep-out, for quick lookups.
    pub keepout_holes: HashSet<Point>,
    /// (net, two of its pins as (component, pin)): join them by as little copper as possible.
    pub near: Vec<(usize, [(usize, usize); 2])>,
    /// Two pins as (component, pin), of any nets: as close together as possible.
    pub close: Vec<[(usize, usize); 2]>,
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

/// Matches one case's part names to the circuit and checks its rules make sense. Stops on a mistake
/// in the board file; returns why if the rules are fine but some part has no place meeting them.
pub fn resolve(case: &Case, c: &Circuit) -> Result<Constraints, String> {
    let index = |name: &str| {
        c.components.iter().position(|comp| comp.reference == name).unwrap_or_else(|| panic!("board file: there's no part called {name}"))
    };
    // The net a pin is on, if it's connected to anything.
    let net_of = |p: (usize, usize)| c.net_pins.iter().position(|pins| pins.contains(&p));
    let n = c.components.len();
    let (w, h) = (c.board.width, c.board.height);
    let mut out = Constraints { choices: case.choices.clone(), allowed: vec![None; n], ..Default::default() };
    // rules_for[component] = the rules about where it goes
    let mut rules_for: Vec<Vec<&Rule>> = vec![Vec::new(); n];
    for &rule in &case.rules {
        if let Some(part) = rule.part(w, h) {
            rules_for[index(part)].push(rule);
            continue;
        }
        let called = rule.called();
        match rule {
            Rule::Keepout { name, from, to } => {
                let k = Keepout { from: *from, to: *to, name: name.clone().unwrap_or_default() };
                if k.holes().any(|(x, y)| x < 0 || x >= w || y < 0 || y >= h) {
                    panic!("board file: {called}: {from:?} to {to:?} goes off the {w}x{h} board");
                }
                out.keepout_holes.extend(k.holes());
                out.keepout.push(k);
            }
            Rule::Near { pins: [a, b], .. } => {
                let (pa, pb) = (c.find_pin(a), c.find_pin(b));
                if pa == pb {
                    panic!("board file: {called}: both pins are {a}");
                }
                // Near is measured along the copper, so there has to be copper between them.
                let net = net_of(pa).filter(|&net| net_of(pb) == Some(net));
                let net = net.unwrap_or_else(|| panic!("board file: {called}: {a} and {b} aren't on the same net, so no copper joins them"));
                out.near.push((net, [pa, pb]));
            }
            Rule::Close { pins: [a, b], .. } => {
                let (pa, pb) = (c.find_pin(a), c.find_pin(b));
                if pa == pb {
                    panic!("board file: {called}: both pins are {a}");
                }
                out.close.push([pa, pb]);
            }
            Rule::Star { at, runs, .. } => {
                let at_pin = c.find_pin(at);
                let net = net_of(at_pin).unwrap_or_else(|| panic!("board file: {called}: {at} isn't connected to anything"));
                let net_name = &c.nets[net].name;
                if out.stars.iter().any(|s| s.net == net) {
                    panic!("board file: {called}: {net_name} already has a star rule");
                }
                let on_net = |pin: &str| {
                    let p = c.find_pin(pin);
                    if !c.net_pins[net].contains(&p) {
                        panic!("board file: {called}: {pin} isn't on {net_name}, the net of {at}");
                    }
                    p
                };
                let resolved: Vec<Vec<(usize, usize)>> = runs.iter().map(|run| run.iter().map(|pin| on_net(pin)).collect()).collect();
                let mut seen = HashSet::from([at_pin]);
                for (pin, p) in runs.iter().flatten().zip(resolved.iter().flatten()) {
                    if !seen.insert(*p) {
                        panic!("board file: {called}: {pin} is the star point or in two runs");
                    }
                }
                out.stars.push(StarRule { net, at: at_pin, runs: resolved });
            }
            _ => unreachable!("rules about where a part goes were handled above"),
        }
    }
    // Every place a part's rules allow, out of all of them (each hole, each quarter turn). A part
    // with rules also stays on the board and off the keep-outs, so a rule never pushes it there.
    for (i, rules) in rules_for.iter().enumerate().filter(|(_, rules)| !rules.is_empty()) {
        let comp = &c.components[i];
        let fits = |p: Place| {
            let (pw, ph) = board::size(comp, p.rot);
            let on_keepout = comp.body.iter().any(|&hole| {
                let (dx, dy) = board::turn(hole, comp.w, comp.h, p.rot);
                out.keepout_holes.contains(&(p.x + dx, p.y + dy))
            });
            p.x + pw <= w && p.y + ph <= h && !on_keepout
        };
        let places: Vec<Place> = (0..4)
            .flat_map(|rot| (0..w).flat_map(move |x| (0..h).map(move |y| Place { x, y, rot })))
            .filter(|&p| fits(p) && rules.iter().all(|r| r.allows(comp, p, w, h)))
            .collect();
        if places.is_empty() {
            let names: Vec<String> = rules.iter().map(|r| r.called()).collect();
            return Err(format!(
                "no place for {} meets all its rules ({}) while staying on the {w}x{h} board and off the keep-outs",
                comp.reference,
                names.join(", ")
            ));
        }
        out.allowed[i] = Some(places);
    }
    Ok(out)
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

/// Moves every part with placement rules to the nearest place they allow, keeping its rotation if
/// one allows it (so a quarter turn by the placer stays a quarter turn when the rules allow it).
/// The placer calls this after every move, so these rules can never be broken.
pub fn apply(c: &Circuit, placement: &mut Placement) {
    for (p, allowed) in placement.iter_mut().zip(&c.constraints.allowed) {
        if let Some(allowed) = allowed {
            let away = |q: &&Place| (q.rot != p.rot, (q.x - p.x).abs() + (q.y - p.y).abs());
            *p = *allowed.iter().min_by_key(away).expect("resolve never leaves a part without a place");
        }
    }
}

/// Plain-language problems with a finished layout. Placement rules can't break, so only keep-outs are checked.
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
