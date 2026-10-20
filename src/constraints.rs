//! The board file: which perfboard you're building on and your own rules for it.
//!
//! It sits next to the netlist with the same name (circuits/555_blinker.net ->
//! circuits/555_blinker.json), or is the file `--board-file` names, and every part of it is optional:
//!
//!     {
//!       "board": "18x12",
//!       "power": "+5V", "ground": "GND",
//!       "rules": [
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
//!         { "kind": "edge",    "part": "D1", "edge": "right", "weight": 0.01 },
//!         { "kind": "near",    "pins": ["C2.1", "U1.5"], "weight": 0.5 },
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
//! Every rule may have a "weight", 0-1. Without one it's hard: the placer never breaks it (near and
//! close, below, cost as at weight 1). With one it may be broken, and each hole it's broken by costs
//! the weight times a thousand (`board::WEIGHTS`' rules): at 1 that's two solder-side wires, nearly
//! a hard limit; at 0 nothing. A weight on an all-of or one-of is the weight of every rule inside
//! without its own.
//!
//! fixed, edge, orientation and within say where one part may go. In each case, each part with such
//! rules gets the list of places (x, y, rot) its hard ones allow, and the placer only ever puts the
//! part on one of them. Those places are all on the board and off the hard keep-outs too. A case
//! where some part has no such place is left out. A soft one is broken by how many holes (and
//! quarter turns) the part is from where it wants it (`Rule::broken`).
//!
//! A keep-out is a hard limit too: no part or copper covers one, like no two parts share a hole
//! (see `board::clashes`). The placer never makes a layout that breaks it. A soft one is broken by
//! each of its holes a part or copper covers.
//!
//! near: the two pins, on the same net, should be joined by as little copper as possible. It costs
//! per hole of copper between them, as routed (through the star point, for two runs of a star net).
//! close is the same for two pins of any nets, measured straight across the board: for a connection
//! the board doesn't carry, like a wire from a module's underside. Near is only known once the board
//! is routed, so it's always a cost, and close is priced the same way.
//!
//! star says how a net (the one its `at` pin is on) is wired: every run starts at the `at` pin and
//! goes its own way, never sharing copper with another run. Each run in `runs` joins its pins; any
//! other pin of the net gets a run of its own, straight from `at`. A soft one is broken by each join
//! of one run onto another's copper, which the router takes when it's cheaper (see route.rs).

use crate::board::{self, Design, Component, Place, Placement, Point, Run};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)] // so a typo like "rule" (or an old file's "constraints") is an error, not silently ignored
pub struct BoardFile {
    pub board: Option<String>,
    pub power: Option<String>,
    pub ground: Option<String>,
    /// Your rules for this board, all of which hold: the same as an all-of's `rules`.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// One entry of "rules" (the board file's, or an all-of's or one-of's). `kind` in the JSON picks the variant, like { "kind": "near", ... }.
///
/// Every rule may have a `weight`, 0-1, which softens it: see `weight`.
///
/// The layout file keeps them as written, so it writes them back the same way, leaving out what
/// was left out.
#[derive(Deserialize, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Rule {
    /// The part goes exactly there and never moves.
    Fixed {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        part: String,
        /// Top-left hole of the part, after rotating.
        at: Point,
        /// Quarter turns clockwise, 0-3, like the placer uses.
        #[serde(default, skip_serializing_if = "is_zero")]
        rot: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// The part touches that edge of the board.
    Edge {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        part: String,
        edge: Edge,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// The part is turned this way: quarter turns clockwise from how it's drawn, 0-3.
    Orientation {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        part: String,
        rot: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// Every hole of the part's outline is in this rectangle, `from` one corner `to` the other.
    Within {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        part: String,
        from: Point,
        to: Point,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// Every rule in `rules` holds. A weight here is the weight of every rule in it that has none.
    #[serde(rename = "all-of")]
    AllOf {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        rules: Vec<Rule>,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// At least one rule in `rules` holds: each is a case of its own. A weight here is the weight
    /// of every rule in it that has none.
    #[serde(rename = "one-of")]
    OneOf {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        rules: Vec<Rule>,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// Holes no part or copper may cover (or, with a weight, may at a cost).
    Keepout {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        from: Point,
        to: Point,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// Two pins of the same net, like ["R1.2", "U1.8"], joined by as little copper as possible.
    Near {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        pins: [String; 2],
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// Two pins of any nets as close together as possible, straight across the board.
    Close {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        pins: [String; 2],
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
    /// How the net of the `at` pin is wired.
    Star {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// The pin every run starts from, like "C1.+".
        at: String,
        /// Pins that share a run, like [["U1.8", "J2.1"], ["U2.4"]].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        runs: Vec<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<f64>,
    },
}

/// For leaving a `fixed` rule's `rot` out when it's 0, as it may be written.
fn is_zero(rot: &u8) -> bool {
    *rot == 0
}

/// What a near or close rule left without a weight costs as. They're measured along the routed
/// copper (or across the board), so there's no telling before routing whether a move breaks them,
/// the way the placer can for the others: as hard as they get is a weight of 1.
const HARD: f64 = 1.0;

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

    /// The weight the rule itself says, if any (not one it gets from an all-of or one-of around it).
    fn own_weight(&self) -> Option<f64> {
        match self {
            Rule::Fixed { weight, .. }
            | Rule::Edge { weight, .. }
            | Rule::Orientation { weight, .. }
            | Rule::Within { weight, .. }
            | Rule::AllOf { weight, .. }
            | Rule::OneOf { weight, .. }
            | Rule::Keepout { weight, .. }
            | Rule::Near { weight, .. }
            | Rule::Close { weight, .. }
            | Rule::Star { weight, .. } => *weight,
        }
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

    /// How far `comp` at `p` on a w x h board is from what this rule wants, times the rule's weight
    /// (its own, or `inherited` from an all-of or one-of around it): 0 where the rule holds. Far is
    /// counted in holes, and quarter turns. A hard rule (no weight) that doesn't hold is infinitely
    /// broken: the part may never go there. An all-of adds up its rules, a one-of takes its least
    /// broken option. Only for rules with a part (see `part`), and places on the board.
    fn broken(&self, comp: &Component, p: Place, w: i32, h: i32, inherited: Option<f64>) -> f64 {
        let weight = self.own_weight().or(inherited);
        let (pw, ph) = board::size(comp, p.rot);
        // Quarter turns from `rot`, either way round: 0, 1 or 2.
        let turns = |rot: u8| {
            let d = (p.rot + 4 - rot) as i32 % 4;
            d.min(4 - d)
        };
        let away = match self {
            Rule::Fixed { at, rot, .. } => (p.x - at.0).abs() + (p.y - at.1).abs() + turns(*rot),
            Rule::Edge { edge, .. } => match edge {
                Edge::Left => p.x,
                Edge::Right => w - (p.x + pw),
                Edge::Top => p.y,
                Edge::Bottom => h - (p.y + ph),
            },
            Rule::Orientation { rot, .. } => turns(*rot),
            Rule::Within { from, to, .. } => {
                // How many holes the outline sticks out of the rectangle, across and down.
                let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                (x0 - p.x).max(0) + (p.x + pw - 1 - x1).max(0) + (y0 - p.y).max(0) + (p.y + ph - 1 - y1).max(0)
            }
            Rule::AllOf { rules, .. } => return rules.iter().map(|r| r.broken(comp, p, w, h, weight)).sum(),
            Rule::OneOf { rules, .. } => return rules.iter().map(|r| r.broken(comp, p, w, h, weight)).fold(f64::INFINITY, f64::min),
            _ => unreachable!("not about where a part goes"),
        };
        match weight {
            _ if away == 0 => 0.0,
            None => f64::INFINITY,
            Some(weight) => weight * away as f64,
        }
    }
}

/// One way for all the board file's rules to hold: a plain list of rules (no all-of or one-of), and
/// for messages, the option it took in each one-of on the way.
#[derive(Default)]
pub struct Case<'a> {
    /// Each rule with its weight: its own, or one from an all-of or one-of around it.
    rules: Vec<(&'a Rule, Option<f64>)>,
    pub choices: Vec<String>,
}

/// The board file's rules as cases: one, unless a one-of makes more (see `rule_cases`). The file's
/// `rules` all hold, like an all-of's.
fn cases(rules: &[Rule]) -> Vec<Case<'_>> {
    check_weights(rules);
    every_case(rules, None)
}

/// `design` (with no rules yet) with the board file's `rules`, once for each case of them that can
/// be met; and why each other case can't (some part has no place meeting its rules). Stops on a
/// mistake in the board file.
pub fn designs(design: &Design, rules: &[Rule]) -> (Vec<Design>, Vec<String>) {
    let (mut designs, mut left_out) = (Vec::new(), Vec::new());
    for case in cases(rules) {
        match resolve(&case, design) {
            Ok(constraints) => {
                let mut d = design.clone();
                d.rules = rules.to_vec();
                d.constraints = constraints;
                d.runs = runs(&d); // now that the star rules are known
                designs.push(d);
            }
            Err(why) if case.choices.is_empty() => left_out.push(why),
            Err(why) => left_out.push(format!("{}: {why}", case.choices.join("; "))),
        }
    }
    (designs, left_out)
}

/// Stops on a weight outside 0-1, in any rule however deep.
fn check_weights(rules: &[Rule]) {
    for rule in rules {
        if let Some(w) = rule.own_weight().filter(|w| !(0.0..=1.0).contains(w)) {
            panic!("board file: {}: \"weight\" should be 0-1 (1 is nearly a hard limit, 0 costs nothing; leave it out for a hard one), not {w}", rule.called());
        }
        if let Rule::AllOf { rules, .. } | Rule::OneOf { rules, .. } = rule {
            check_weights(rules);
        }
    }
}

/// Every way a list of rules can all hold: each case of the first rule with each case of the rest.
/// (Two one-ofs of 2 and 3 options make 2 x 3 = 6 cases.)
/// `inherited` is the weight of an all-of or one-of they're in, for the rules without their own.
fn every_case(rules: &[Rule], inherited: Option<f64>) -> Vec<Case<'_>> {
    let mut cases = vec![Case::default()];
    for rule in rules {
        let options = rule_cases(rule, inherited);
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
fn rule_cases(rule: &Rule, inherited: Option<f64>) -> Vec<Case<'_>> {
    let weight = rule.own_weight().or(inherited);
    match rule {
        Rule::AllOf { rules, .. } | Rule::OneOf { rules, .. } if rules.is_empty() => {
            panic!("board file: {} has no rules in it", rule.called())
        }
        // A one-of only about where one part goes makes no cases: the part may take any place one
        // of its options allows (see `broken`), so its options are all tried in the same run.
        Rule::OneOf { .. } if rule.one_part().is_some() => vec![Case { rules: vec![(rule, weight)], choices: Vec::new() }],
        Rule::AllOf { rules, .. } => every_case(rules, weight),
        Rule::OneOf { rules, .. } => {
            let mut out = Vec::new();
            for (k, option) in rules.iter().enumerate() {
                // Name the option by its own name if it has one, else by its number.
                let which = match option.kind_and_name().1 {
                    Some(name) => format!("\"{name}\""),
                    None => format!("option {} of {}", k + 1, rules.len()),
                };
                for mut case in rule_cases(option, weight) {
                    case.choices.insert(0, format!("{}: {which}", rule.called()));
                    out.push(case);
                }
            }
            out
        }
        _ => vec![Case { rules: vec![(rule, weight)], choices: Vec::new() }],
    }
}

/// A near rule, matched up with the circuit: pins are (component, pin) indexes.
#[derive(Clone)]
pub struct NearRule {
    pub net: usize,
    pub pins: [(usize, usize); 2],
    /// 0-1: each hole of copper between the pins costs this times `board::WEIGHTS`' rules.
    pub weight: f64,
}

/// A close rule, matched up with the circuit: pins are (component, pin) indexes, of any nets.
#[derive(Clone)]
pub struct CloseRule {
    pub pins: [(usize, usize); 2],
    /// 0-1, like a near rule's.
    pub weight: f64,
}

/// A star rule, matched up with the circuit: pins are (component, pin) indexes.
#[derive(Clone)]
pub struct StarRule {
    pub net: usize,
    pub at: (usize, usize),
    pub runs: Vec<Vec<(usize, usize)>>,
    /// None: runs never meet but at `at`. 0-1: a run may join another run of this net anywhere, and
    /// each join costs this times `board::WEIGHTS`' rules (see `route`).
    pub weight: Option<f64>,
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
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
    /// None: nothing goes there, ever. 0-1: parts and copper may, and each hole they cover costs
    /// this times `board::WEIGHTS`' rules.
    pub weight: Option<f64>,
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
    /// Every keep-out, hard or soft (see `Keepout::weight`). Add them with `add_keepout`.
    pub keepout: Vec<Keepout>,
    /// Every hole of every hard keep-out, for quick lookups.
    pub keepout_holes: HashSet<Point>,
    /// Every hole of a soft keep-out, and its weight (the highest, where two overlap).
    pub soft_keepout: HashMap<Point, f64>,
    /// place_cost[component][place]: how broken its soft placement rules are there, weighted (see
    /// `Rule::broken`). Places they don't break aren't in it.
    pub place_cost: Vec<HashMap<Place, f64>>,
    /// Two pins of one net: join them by as little copper as possible.
    pub near: Vec<NearRule>,
    /// Two pins of any nets: as close together as possible.
    pub close: Vec<CloseRule>,
    pub stars: Vec<StarRule>,
}

impl Constraints {
    /// Adds a keep-out: its holes go in `keepout_holes` if it's hard, `soft_keepout` if it has a weight.
    pub fn add_keepout(&mut self, k: Keepout) {
        match k.weight {
            None => self.keepout_holes.extend(k.holes()),
            Some(w) => {
                for hole in k.holes() {
                    let at = self.soft_keepout.entry(hole).or_insert(w);
                    *at = at.max(w);
                }
            }
        }
        self.keepout.push(k);
    }

    /// How broken the soft placement rules are, weighted, with the parts at `placement`.
    pub fn placement_cost(&self, placement: &[Place]) -> f64 {
        self.place_cost.iter().zip(placement).filter_map(|(costs, p)| costs.get(p)).sum::<f64>() + 0.0
    }
}

/// Reads the board file `chosen`, or else the one next to `netlist_path` if there is one (the same
/// name, with .json). Returns its path too.
pub fn read_board_file(netlist_path: &str, chosen: Option<&str>) -> Option<(String, BoardFile)> {
    let path = match chosen {
        Some(path) => path.to_string(),
        None => std::path::Path::new(netlist_path).with_extension("json").to_string_lossy().into_owned(),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if chosen.is_some() => panic!("can't read the board file {path}: {e}"),
        Err(_) => return None,
    };
    let file = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad board file {path}: {e}"));
    Some((path, file))
}

/// Matches one case's part names to the circuit and checks its rules make sense. Stops on a mistake
/// in the board file; returns why if the rules are fine but some part has no place meeting them.
pub fn resolve(case: &Case, c: &Design) -> Result<Constraints, String> {
    let index = |name: &str| {
        c.components.iter().position(|comp| comp.reference == name).unwrap_or_else(|| panic!("board file: there's no part called {name}"))
    };
    // The net a pin is on, if it's connected to anything.
    let net_of = |p: (usize, usize)| c.net_pins.iter().position(|pins| pins.contains(&p));
    let n = c.components.len();
    let (w, h) = (c.board.width, c.board.height);
    let mut out = Constraints { choices: case.choices.clone(), allowed: vec![None; n], place_cost: vec![HashMap::new(); n], ..Default::default() };
    // rules_for[component] = the rules about where it goes, with their weights
    let mut rules_for: Vec<Vec<(&Rule, Option<f64>)>> = vec![Vec::new(); n];
    for &(rule, weight) in &case.rules {
        if let Some(part) = rule.part(w, h) {
            rules_for[index(part)].push((rule, weight));
            continue;
        }
        let called = rule.called();
        match rule {
            Rule::Keepout { name, from, to, .. } => {
                let k = Keepout { from: *from, to: *to, name: name.clone().unwrap_or_default(), weight };
                if k.holes().any(|(x, y)| x < 0 || x >= w || y < 0 || y >= h) {
                    panic!("board file: {called}: {from:?} to {to:?} goes off the {w}x{h} board");
                }
                out.add_keepout(k);
            }
            Rule::Near { pins: [a, b], .. } => {
                let (pa, pb) = (c.find_pin(a), c.find_pin(b));
                if pa == pb {
                    panic!("board file: {called}: both pins are {a}");
                }
                // Near is measured along the copper, so there has to be copper between them.
                let net = net_of(pa).filter(|&net| net_of(pb) == Some(net));
                let net = net.unwrap_or_else(|| panic!("board file: {called}: {a} and {b} aren't on the same net, so no copper joins them"));
                out.near.push(NearRule { net, pins: [pa, pb], weight: weight.unwrap_or(HARD) });
            }
            Rule::Close { pins: [a, b], .. } => {
                let (pa, pb) = (c.find_pin(a), c.find_pin(b));
                if pa == pb {
                    panic!("board file: {called}: both pins are {a}");
                }
                out.close.push(CloseRule { pins: [pa, pb], weight: weight.unwrap_or(HARD) });
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
                out.stars.push(StarRule { net, at: at_pin, runs: resolved, weight });
            }
            _ => unreachable!("rules about where a part goes were handled above"),
        }
    }
    // Every place a part's hard rules allow, out of all of them (each hole, each quarter turn), and
    // how broken its soft ones are there. A part with rules also stays on the board and off the
    // hard keep-outs, so a rule never pushes it there.
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
        let broken = |p: Place| rules.iter().map(|(r, weight)| r.broken(comp, p, w, h, *weight)).sum::<f64>();
        let places: Vec<(Place, f64)> = (0..4)
            .flat_map(|rot| (0..w).flat_map(move |x| (0..h).map(move |y| Place { x, y, rot })))
            .filter(|&p| fits(p))
            .map(|p| (p, broken(p)))
            .filter(|(_, cost)| cost.is_finite())
            .collect();
        if places.is_empty() {
            let names: Vec<String> = rules.iter().filter(|(_, weight)| weight.is_none()).map(|(r, _)| r.called()).collect();
            return Err(format!(
                "no place for {} meets all its hard rules ({}) while staying on the {w}x{h} board and off the hard keep-outs",
                comp.reference,
                names.join(", ")
            ));
        }
        out.allowed[i] = Some(places.iter().map(|&(p, _)| p).collect());
        out.place_cost[i] = places.into_iter().filter(|&(_, cost)| cost > 0.0).collect();
    }
    Ok(out)
}

/// How each net is wired: one run joining all its pins, or for a star net, one run per branch
/// from the star point.
pub fn runs(c: &Design) -> Vec<Run> {
    let mut out = Vec::new();
    for (net, pins) in c.net_pins.iter().enumerate() {
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
pub fn apply(c: &Design, placement: &mut Placement) {
    for (p, allowed) in placement.iter_mut().zip(&c.constraints.allowed) {
        if let Some(allowed) = allowed {
            let away = |q: &&Place| (q.rot != p.rot, (q.x - p.x).abs() + (q.y - p.y).abs());
            *p = *allowed.iter().min_by_key(away).expect("resolve never leaves a part without a place");
        }
    }
}
