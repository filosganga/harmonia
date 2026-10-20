//! Perfboard model: the circuit, placement geometry, rule-based scoring, rendering.
//!
//! Coordinates are perfboard holes: x grows to the right, y grows downwards.
//! A placement gives each component the position of its top-left hole and a
//! rotation (0..3, in 90 degree steps).

use rand::Rng;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

pub type Point = (i32, i32);

/// The eleven things the rules measure. They are also the network's inputs. They only judge layouts
/// that are allowed (see `clashes`): the placer never makes any other kind.
pub const FEATURES: [&str; 11] = ["wire", "turns", "diagonal", "crosswise", "jumpers", "jumper_len", "bottom", "unrouted", "power", "crowding", "rules"];

/// How much each measurement costs, in the same order as FEATURES. Lower total cost = better board.
///
/// Built by hand, a perfboard's work is in its corners and jumpers, not its lengths: a straight run
/// of copper or a jumper is about as much work 3 holes long as 10. So a corner costs five holes of
/// trace, and a jumper three corners (the maintainer's exchange rate). Against the weights before
/// (a corner 1, a jumper 5 plus 2 per hole, a solder-side wire 100, a diagonal 3, crowding 2), on
/// the benchmark circuits placed with seeds 1-3 / 4-6: corners + 3 x jumpers 619 / 585 -> 455 / 443
/// (corners 400 / 402 -> 242 / 230, jumpers 73 / 61 -> 71 / 71), solder-side wires 1 / 1 -> 1 / 3,
/// diagonal steps 12 / 12 -> 3 / 3, crowding 123 / 123 -> 82 / 76; copper between `near` pins
/// 42 / 41 -> 63 / 35 and 5V-to-ground 431 / 437 -> 481 / 471. Making near and power five times
/// dearer too packed the parts again (crowding 132 / 104) and the cat feeder came out much messier.
/// (All measured with near at 4 a hole; near and close now default to a weight of 1, 1000 a hole,
/// and one set to 0.004 costs what they did then.)
/// From random starts (`bench`): corners + 3 x jumpers 651 -> 485, solder-side wires 5 -> 5,
/// crowding 115 -> 83.
pub const WEIGHTS: [f64; 11] = [
    1.0,  // wire: per hole of copper trace, as routed (see route/search.rs, whose costs match these)
    5.0,  // turns: per right angle's worth of bending a trace (45° is 2, 135° is 3; see route/search.rs `bend`): a corner is the work, not the length
    15.0, // diagonal: per diagonal step of copper trace, on top of its length, three corners' worth (jumpers and wires don't count)
    0.5,  // crosswise: per straight step of trace running against the board's main direction (see route.rs)
    15.0, // jumpers: per jumper on the parts' side, three corners' worth (the router prices every jumper at this, route/search.rs `JUMPER`)
    0.5,  // jumper_len: per hole of jumper or solder-side wire. Not the work (a long jumper is no harder), but a long jumper takes room later jumpers can't cross: almost free (0.1) brought solder-side wires back
    500.0, // bottom: per wire on the solder side, the last resort, a hundred corners (the router prices every one at this, route/search.rs `SOLDER_WIRE`)
    5000.0, // unrouted: per connection the router couldn't make at all (a pin walled in; nothing for the router to price): a thousand corners, ten solder-side wires
    1.5,  // power: per hole between each 5V pin and its nearest GND pin
    4.0,  // crowding: per pair of neighbouring holes taken by two parts (twice for pins of two nets): room to solder; at 2 the parts packed with the board half empty
    1000.0, // rules: per hole (or quarter turn, or star join) a board-file rule with a weight is broken, times its weight (0-1; near and close, measured in holes of copper between their pins, count as 1 without one): at 1 a hole costs two solder-side wires, nearly a hard limit; 0.004 is what near and close cost before weights (4)
];

/// Turns the cost into a 0-100 score (100 = perfect, about 37 at cost == SCORE_SCALE).
pub const SCORE_SCALE: f64 = 150.0;

#[derive(Clone)]
pub struct Pin {
    pub name: String,
    /// Its hole. For a pin between holes (`off_grid`), the nearest one.
    pub at: Point,
    /// The pin isn't on a hole, like a tactile switch's legs. Copper in the holes around it could
    /// touch it, so the router keeps other nets out of them.
    pub off_grid: bool,
}

#[derive(Clone)]
pub struct Component {
    pub reference: String,
    /// What the netlist says it is, e.g. "NE555P" or "10k". Only used when exporting.
    pub value: String,
    /// Full KiCad footprint name, e.g. "Package_DIP:DIP-8_W7.62mm".
    pub footprint: String,
    /// Which DIYLC part to draw it as, if footprints.json says.
    pub diylc: Option<crate::diylc::Part>,
    /// Its outline: w x h holes before turning.
    pub w: i32,
    pub h: i32,
    pub pins: Vec<Pin>,
    /// The holes inside the outline that are actually taken. For most parts that's all of
    /// them; a module on headers only takes its header holes, and low parts can go under the rest.
    pub body: Vec<Point>,
    /// For parts read from a DIYLC file: the original drawing, so it can be written back moved.
    pub source: Option<crate::diylc::Source>,
    /// Low enough to sit under a module on headers (a resistor lying flat, a small ceramic capacitor).
    pub low: bool,
    /// The footprints.json its footprint came from, if not the built-in library (yours, or one next
    /// to the netlist), so a layout can say it depends on that file.
    pub footprint_from: Option<String>,
}

impl Component {
    /// Every hole of a w x h outline: the body of an ordinary part.
    pub fn full_body(w: i32, h: i32) -> Vec<Point> {
        (0..w).flat_map(|x| (0..h).map(move |y| (x, y))).collect()
    }

    /// Whether other parts can go under it: its body doesn't take its whole outline. A module on
    /// headers (a footprints.json module, a DIYLC group); also a DIYLC two-legged part drawn at an
    /// angle, which only takes its legs.
    pub fn room_under(&self) -> bool {
        self.body.len() < (self.w * self.h) as usize
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Net {
    pub name: String,
    pub pins: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct BoardSize {
    pub width: i32,
    pub height: i32,
}

/// What you asked for: the board, the parts and how they connect, and your rules for the board as
/// written in the board file, for one case of them (`constraints.choices` says which). The placer,
/// the router and the score all work on one; with a `Layout` it makes a `Build`.
///
/// `Design::new` makes one from a KiCad netlist, a DIYLC sketch or a layout file, without rules; `constraints::
/// designs` turns it into one per case of the board file's rules. The fields after `rules` are
/// worked out from the ones before.
#[derive(Clone)]
pub struct Design {
    pub board: BoardSize,
    pub components: Vec<Component>,
    /// Each joins two pins or more: a net with one pin has nothing to join, so the netlist and
    /// sketch readers leave it out (and a layout file with one is refused).
    pub nets: Vec<Net>,
    /// The supply and ground nets, if known (without them the power rule is off).
    pub power: Option<String>,
    pub ground: Option<String>,
    /// The board file's rules as written: all of them, every case's.
    pub rules: Vec<crate::constraints::Rule>,
    /// For each net, its pins as (component index, pin index).
    pub net_pins: Vec<Vec<(usize, usize)>>,
    /// This case's rules, matched up with the parts (where parts go, keep-outs, near, star).
    pub constraints: crate::constraints::Constraints,
    /// What the router joins: one run per net, or one per branch of a star net.
    pub runs: Vec<Run>,
}

/// Pins to join with one piece of copper. Two runs never share copper, even on the same net,
/// unless a soft star joins them (`Routing::joins`).
#[derive(Clone)]
pub struct Run {
    pub net: usize,
    /// (component index, pin index); for a star run the star point comes first.
    pub pins: Vec<(usize, usize)>,
    /// The star point, if this run is a branch of a star net.
    pub star: Option<(usize, usize)>,
}

impl Design {
    /// A design without rules yet: `constraints::designs` adds them, one design per case.
    pub fn new(board: BoardSize, components: Vec<Component>, nets: Vec<Net>, power: Option<String>, ground: Option<String>) -> Design {
        let mut c = Design { board, components, nets, power, ground, rules: Vec::new(), net_pins: Vec::new(), constraints: Default::default(), runs: Vec::new() };
        c.net_pins = c.nets.iter().map(|net| net.pins.iter().map(|p| c.find_pin(p)).collect()).collect();
        if let Some(net) = c.nets.iter().find(|n| n.pins.len() < 2) {
            panic!("net {} joins fewer than two pins", net.name);
        }
        // Stop now, with a clear message, if the power or ground net doesn't exist.
        c.power.iter().chain(&c.ground).for_each(|name| {
            c.net_index(name);
        });
        c
    }

    /// (index of U1, index of its pin "8") -> "U1.8": `find_pin` the other way.
    pub fn pin_name(&self, (ci, pi): (usize, usize)) -> String {
        format!("{}.{}", self.components[ci].reference, self.components[ci].pins[pi].name)
    }

    /// "U1.8" -> (index of U1, index of its pin "8")
    pub fn find_pin(&self, label: &str) -> (usize, usize) {
        let (reference, pin) = label.split_once('.').unwrap_or_else(|| panic!("pin {label} should look like U1.8"));
        let ci = self.components.iter().position(|c| c.reference == reference);
        let ci = ci.unwrap_or_else(|| panic!("a net uses {reference}, which isn't in the component list"));
        let pi = self.components[ci].pins.iter().position(|p| p.name == pin);
        (ci, pi.unwrap_or_else(|| panic!("{label}: its footprint has no pad {pin}")))
    }

    fn net_index(&self, name: &str) -> usize {
        self.nets.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("no net called {name}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub rot: u8,
}

pub type Placement = Vec<Place>;

/// Every part dropped at a random place (parts with placement rules on the nearest place they
/// allow). Parts may land on each other: `place::random_start` moves them apart.
pub fn random_placement(c: &Design, rng: &mut impl Rng) -> Placement {
    let (w, h) = (c.board.width, c.board.height);
    let mut placement: Placement = c
        .components
        .iter()
        .map(|_| Place { x: rng.random_range(0..w - 1), y: rng.random_range(0..h - 1), rot: rng.random_range(0..4) })
        .collect();
    crate::constraints::apply(c, &mut placement);
    placement
}

/// Width and height of a component after `rot` quarter turns.
pub fn size(comp: &Component, rot: u8) -> (i32, i32) {
    if rot % 2 == 0 { (comp.w, comp.h) } else { (comp.h, comp.w) }
}

/// Turns a hole inside a w x h outline `rot` quarter turns clockwise (the outline's top-left stays at 0, 0).
pub fn turn((x, y): Point, w: i32, h: i32, rot: u8) -> Point {
    let (mut x, mut y, mut w, mut h) = (x, y, w, h);
    for _ in 0..rot % 4 {
        (x, y, w, h) = (h - 1 - y, x, h, w);
    }
    (x, y)
}

/// Where everything physically is on the board.
pub struct Geometry {
    /// Holes taken by each component's body.
    pub bodies: Vec<Vec<Point>>,
    /// Every hole inside each component's outline (the same as its body, except for modules).
    pub outlines: Vec<Vec<Point>>,
    /// Absolute position of each pin: pins[component][pin].
    pub pins: Vec<Vec<Point>>,
}

pub fn geometry(c: &Design, placement: &[Place]) -> Geometry {
    let mut g = Geometry { bodies: Vec::new(), outlines: Vec::new(), pins: Vec::new() };
    for (comp, p) in c.components.iter().zip(placement) {
        let at = |hole: Point| {
            let (dx, dy) = turn(hole, comp.w, comp.h, p.rot);
            (p.x + dx, p.y + dy)
        };
        g.bodies.push(comp.body.iter().map(|&b| at(b)).collect());
        g.outlines.push(Component::full_body(comp.w, comp.h).into_iter().map(at).collect());
        g.pins.push(comp.pins.iter().map(|pin| at(pin.at)).collect());
    }
    g
}

fn manhattan(a: Point, b: Point) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// Distance between two holes, in holes, straight across the board.
pub fn distance(a: Point, b: Point) -> f64 {
    (((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) as f64).sqrt()
}

/// The corners of the smallest rectangle around some holes: (left, top), (right, bottom).
pub fn bounds(holes: &[Point]) -> (Point, Point) {
    let (xs, ys) = (holes.iter().map(|p| p.0), holes.iter().map(|p| p.1));
    ((xs.clone().min().unwrap(), ys.clone().min().unwrap()), (xs.max().unwrap(), ys.max().unwrap()))
}

/// The raw measurements, in the order of FEATURES.
#[derive(Clone, Copy, Debug)]
pub struct Measures(pub [f64; FEATURES.len()]);

pub fn measure(c: &Design, placement: &[Place]) -> Measures {
    let g = geometry(c, placement);
    let routing = crate::route::route(c, placement, &g);
    measure_routed(c, placement, &g, &routing)
}

/// The same, routed once, shortest first (`route::route_once`): what the placer judges each move by.
pub fn measure_once(c: &Design, placement: &[Place]) -> Measures {
    let g = geometry(c, placement);
    let routing = crate::route::route_once(c, &g);
    measure_routed(c, placement, &g, &routing)
}

/// The same, for parts at `placement` (whose geometry is `g`) already routed as `routing` (so a
/// layout is routed only once).
pub fn measure_routed(c: &Design, placement: &[Place], g: &Geometry, routing: &crate::route::Routing) -> Measures {
    let pins_of = |net: usize| -> Vec<Point> { c.net_pins[net].iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let net_pins = |name: &Option<String>| name.as_ref().map(|n| pins_of(c.net_index(n))).unwrap_or_default();
    let (vcc, gnd) = (net_pins(&c.power), net_pins(&c.ground));
    let power: i32 = vcc.iter().map(|&v| gnd.iter().map(|&g| manhattan(v, g)).min().unwrap_or(0)).sum();

    let crowding = crowding(c, g);

    // For each "near" rule: holes of copper between its two pins, the way the router joined them.
    // For each "close" rule: holes between its two pins, straight across the board. Each times the
    // rule's weight, so a softened rule counts less.
    let along_copper = c.constraints.near.iter().map(|r| {
        let [(ca, pa), (cb, pb)] = r.pins;
        r.weight * routing.copper_between(c, r.net, g.pins[ca][pa], g.pins[cb][pb])
    });
    let straight = c.constraints.close.iter().map(|r| {
        let [(ca, pa), (cb, pb)] = r.pins;
        r.weight * distance(g.pins[ca][pa], g.pins[cb][pb])
    });
    // Soft placement rules, as broken as they are where the parts are; holes of soft keep-outs that
    // a part or copper covers; and joins between runs of a soft star net (see route.rs).
    let placed = c.constraints.placement_cost(placement);
    let soft = &c.constraints.soft_keepout;
    let on_keepouts: f64 = if soft.is_empty() {
        0.0
    } else {
        // In a fixed order (a BTreeSet), so the weights add up the same way every run.
        let mut covered: BTreeSet<Point> = g.bodies.iter().flatten().filter(|p| soft.contains_key(p)).copied().collect();
        covered.extend(routing.copper_holes().into_iter().filter(|p| soft.contains_key(p)));
        covered.iter().map(|p| soft[p]).sum()
    };
    let star_weight = |run: usize| c.constraints.stars.iter().find(|s| s.net == c.runs[run].net).and_then(|s| s.weight).expect("only a soft star's runs join");
    let joins: f64 = routing.joins.iter().map(|&(run, _)| star_weight(run)).sum();
    let rules: f64 = along_copper.chain(straight).sum::<f64>() + placed + on_keepouts + joins + 0.0; // + 0.0: no rules is 0, not -0

    Measures([
        routing.trace_length(),
        routing.turns(g) as f64,
        routing.diagonal_steps(),
        routing.crosswise_steps(),
        routing.jumpers.len() as f64,
        routing.jumper_length(),
        routing.bottom.len() as f64,
        routing.unrouted.len() as f64,
        power as f64, crowding as f64, rules])
}

/// How crowded the parts are, for soldering: pairs of neighbouring holes (across or down) taken by
/// two different parts, where the iron has no room. A pair of pins a solder blob would wrongly join
/// (on different nets, or one on a net and one unused) counts twice. Parts may be as far apart as they like beyond that;
/// wire length keeps them from drifting.
fn crowding(c: &Design, g: &Geometry) -> i32 {
    // Which part takes each hole (the first one, if two collide: only a layout that isn't allowed has
    // collisions, and the placer never makes one).
    let mut part_at: HashMap<Point, usize> = HashMap::new();
    for (i, body) in g.bodies.iter().enumerate() {
        for &hole in body {
            part_at.entry(hole).or_insert(i);
        }
    }
    // The net of the pin in each hole; a pin on no net is None.
    let mut net_at: HashMap<Point, Option<usize>> = g.pins.iter().flatten().map(|&hole| (hole, None)).collect();
    for (net, pins) in c.net_pins.iter().enumerate() {
        for &(ci, pi) in pins {
            net_at.insert(g.pins[ci][pi], Some(net));
        }
    }
    let mut crowding = 0;
    for (&(x, y), &i) in &part_at {
        for next in [(x + 1, y), (x, y + 1)] {
            if part_at.get(&next).is_some_and(|&j| j != i) {
                crowding += 1;
                if let (Some(a), Some(b)) = (net_at.get(&(x, y)), net_at.get(&next))
                    && a != b
                {
                    crowding += 1;
                }
            }
        }
    }
    crowding
}

/// What's wrong with one hole of a layout that isn't allowed.
#[derive(Clone, Copy, PartialEq)]
pub enum Why {
    /// Another part (this index) already takes the hole.
    OnPart(usize),
    /// The hole is off the board.
    OffBoard,
    /// The hole is on a hard keep-out (this index in `Constraints::keepout`).
    OnKeepout(usize),
    /// The hole is under this module's board, and the part isn't low enough to go there.
    UnderModule(usize),
}

/// One thing that makes a layout impossible to build: `part` at `hole`, and why.
pub struct Clash {
    pub part: usize,
    pub hole: Point,
    pub why: Why,
}

/// Everything that makes a layout impossible to build: two parts taking one hole, a part hanging off
/// the board, a part on a keep-out, or a part under a module without being low enough (see
/// `Component::low`; two modules can't overlap either). An empty list means the layout is allowed.
/// Collisions and keep-outs count the holes a part takes; hanging off the board counts its whole
/// outline (a module's board can't hang off the perfboard either). Only the parts `placed` says are
/// counted, so a layout can be checked while it's being put together.
pub fn clashes(c: &Design, g: &Geometry, placed: &[bool]) -> Vec<Clash> {
    let (w, h) = (c.board.width, c.board.height);
    let keepouts = &c.constraints.keepout;
    let mut out = Vec::new();
    let mut taken: HashMap<Point, usize> = HashMap::new();
    for (part, body) in g.bodies.iter().enumerate().filter(|&(i, _)| placed[i]) {
        for &hole in body {
            match taken.get(&hole) {
                Some(&other) => out.push(Clash { part, hole, why: Why::OnPart(other) }),
                None => _ = taken.insert(hole, part),
            }
            if c.constraints.keepout_holes.contains(&hole) {
                let k = keepouts.iter().position(|k| k.weight.is_none() && k.holes().any(|p| p == hole)).expect("a keep-out hole is in some hard keep-out");
                out.push(Clash { part, hole, why: Why::OnKeepout(k) });
            }
        }
    }
    for (part, outline) in g.outlines.iter().enumerate().filter(|&(i, _)| placed[i]) {
        out.extend(outline.iter().filter(|&&(x, y)| x < 0 || x >= w || y < 0 || y >= h).map(|&hole| Clash { part, hole, why: Why::OffBoard }));
    }
    for (m, (outline, body)) in g.outlines.iter().zip(&g.bodies).enumerate().filter(|&(i, _)| placed[i]) {
        if !c.components[m].room_under() {
            continue;
        }
        let under: HashSet<Point> = outline.iter().filter(|p| !body.contains(p)).copied().collect();
        for (part, comp) in c.components.iter().enumerate().filter(|&(i, _)| placed[i]) {
            if part != m && !comp.low {
                out.extend(g.outlines[part].iter().filter(|h| under.contains(h)).map(|&hole| Clash { part, hole, why: Why::UnderModule(m) }));
            }
        }
    }
    out
}

/// Is the layout allowed: can it be built? (No clashes, see `clashes`.)
pub fn allowed(c: &Design, placement: &[Place]) -> bool {
    clashes(c, &geometry(c, placement), &vec![true; placement.len()]).is_empty()
}

/// The clashes in plain words, one line per part and what's wrong with it, like "R1 and C2 share 2 holes".
pub fn clash_words(c: &Design, clashes: &[Clash]) -> Vec<String> {
    let mut seen: Vec<(usize, Why, usize)> = Vec::new();
    for clash in clashes {
        match seen.iter_mut().find(|(part, why, _)| (*part, *why) == (clash.part, clash.why)) {
            Some((.., holes)) => *holes += 1,
            None => seen.push((clash.part, clash.why, 1)),
        }
    }
    let name = |i: usize| &c.components[i].reference;
    let holes = |n: usize| if n == 1 { "1 hole".to_string() } else { format!("{n} holes") };
    seen.iter()
        .map(|&(part, why, n)| match why {
            Why::OnPart(other) => format!("{} and {} share {}", name(other), name(part), holes(n)),
            Why::OffBoard => format!("{} hangs off the board ({})", name(part), holes(n)),
            Why::OnKeepout(k) => {
                let k = &c.constraints.keepout[k];
                let called = if k.name.is_empty() { String::new() } else { format!(" ({})", k.name) };
                format!("{} is on the keep-out at {:?}-{:?}{called} ({})", name(part), k.from, k.to, holes(n))
            }
            Why::UnderModule(m) => format!("{} is under {}'s board but isn't low enough to fit there ({})", name(part), name(m), holes(n)),
        })
        .collect()
}

/// A measurement as the report and the pictures show it: whole numbers, except `rules`, which is
/// weighted (0.004 of a rule broken still costs 4) and so keeps up to three decimals.
pub fn shown(name: &str, v: f64) -> String {
    if name == "rules" {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        format!("{v:.0}")
    }
}

pub fn cost(m: &Measures) -> f64 {
    m.0.iter().zip(WEIGHTS).map(|(v, w)| v * w).sum()
}

/// 0-100, higher is better.
pub fn score(m: &Measures) -> f64 {
    100.0 * (-cost(m) / SCORE_SCALE).exp()
}

/// One line about a layout: its score, cost and measurements (`m`), or, for one that isn't allowed
/// (a sketch can be drawn that way), what's wrong with it instead: it isn't scored.
pub fn report(c: &Design, placement: &[Place], m: &Measures, title: &str) -> String {
    let wrong = clashes(c, &geometry(c, placement), &vec![true; placement.len()]);
    if !wrong.is_empty() {
        return format!("{title}: not allowed, so not scored: {}", clash_words(c, &wrong).join("; "));
    }
    report_measures(m, title)
}

/// The same line, for measurements already made.
pub fn report_measures(m: &Measures, title: &str) -> String {
    let parts: Vec<String> = FEATURES.iter().zip(m.0).map(|(k, v)| format!("{k}={}", shown(k, v))).collect();
    format!("{title}: score {:.0}/100  (cost {:.1}: {})", score(m), cost(m), parts.join(", "))
}

// ---------------------------------------------------------------- rendering

/// Text picture: each part gets a letter, UPPERCASE on its pins, lowercase on its body, x = keep-out, # = collision.
pub fn ascii_board(c: &Design, placement: &[Place]) -> String {
    let (w, h) = (c.board.width, c.board.height);
    let g = geometry(c, placement);
    let mut grid = vec![vec!['.'; w as usize]; h as usize];
    for (x, y) in c.constraints.keepout.iter().flat_map(|k| k.holes()) {
        grid[y as usize][x as usize] = 'x';
    }
    let on_board = |(x, y): Point| x >= 0 && x < w && y >= 0 && y < h;
    // Under a module (inside its outline but not taken): ':' where nothing else is.
    for (outline, body) in g.outlines.iter().zip(&g.bodies) {
        for &p in outline.iter().filter(|&&p| on_board(p) && !body.contains(&p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            if *cell == '.' {
                *cell = ':';
            }
        }
    }
    for (i, body) in g.bodies.iter().enumerate() {
        let letter = (b'a' + i as u8) as char;
        for &p in body.iter().filter(|&&p| on_board(p)) {
            grid[p.1 as usize][p.0 as usize] = letter;
        }
    }
    for clash in clashes(c, &g, &vec![true; placement.len()]).iter().filter(|clash| on_board(clash.hole)) {
        grid[clash.hole.1 as usize][clash.hole.0 as usize] = '#';
    }
    for (i, pins) in g.pins.iter().enumerate() {
        let letter = (b'A' + i as u8) as char;
        for &p in pins.iter().filter(|&&p| on_board(p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            if *cell != '#' {
                *cell = letter;
            }
        }
    }
    let mut out = String::from("   ");
    out.extend((0..w).map(|x| char::from_digit((x % 10) as u32, 10).unwrap()));
    for (y, row) in grid.iter().enumerate() {
        write!(out, "\n{y:2} {}", row.iter().collect::<String>()).unwrap();
    }
    let mut legend: Vec<String> =
        c.components.iter().enumerate().map(|(i, comp)| format!("{}={}", (b'A' + i as u8) as char, comp.reference)).collect();
    if !c.constraints.keepout.is_empty() {
        legend.push("x=keep-out".into());
    }
    if c.components.iter().any(Component::room_under) {
        legend.push(":=under a module".into());
    }
    out + "\n   " + &legend.join("  ")
}

const NET_COLORS: [&str; 14] = [
    "#1f77b4", "#2ca02c", "#17becf", "#8c564b", "#bcbd22", "#393b79", "#637939",
    "#8c6d31", "#843c39", "#7b4173", "#a6761d", "#1b9e77", "#756bb1", "#e6550d",
];

/// A colour for each net: power red, ground near-black, the rest from NET_COLORS.
/// The SVG picture and the DIYLC jumpers both use these, so a net looks the same in both.
pub fn net_colors(c: &Design) -> Vec<&'static str> {
    let mut others = NET_COLORS.iter().cycle();
    c.nets
        .iter()
        .map(|n| if c.power.as_ref() == Some(&n.name) { "#d62728" } else if c.ground.as_ref() == Some(&n.name) { "#333333" } else { others.next().unwrap() })
        .collect()
}
