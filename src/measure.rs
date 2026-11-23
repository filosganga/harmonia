//! The score: what the rules measure on a layout, what each measurement costs, and what makes a
//! layout impossible to build (`clashes`). It judges a layout already routed: `route::measure`
//! routes one first.

use crate::board::{Geometry, Place, Point, distance};
use crate::rules::Case;
use crate::route::Routing;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

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

/// The raw measurements, in the order of FEATURES.
#[derive(Clone, Copy, Debug)]
pub struct Measures(pub [f64; FEATURES.len()]);

/// What the rules measure on parts at `placement` (whose geometry is `g`), already routed as
/// `routing` (so a layout is routed only once).
pub fn measure_routed(c: &Case, placement: &[Place], g: &Geometry, routing: &Routing) -> Measures {
    let pins_of = |net: usize| -> Vec<Point> { c.circuit.net_pins[net].iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let net_pins = |name: &Option<String>| name.as_ref().map(|n| pins_of(c.circuit.net_index(n))).unwrap_or_default();
    let (vcc, gnd) = (net_pins(&c.circuit.power), net_pins(&c.circuit.ground));
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
        let mut covered: BTreeSet<Point> = g.stands.iter().flatten().filter(|p| soft.contains_key(p)).copied().collect();
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

fn manhattan(a: Point, b: Point) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// How crowded the parts are, for soldering: pairs of neighbouring holes (across or down) taken by
/// two different parts, where the iron has no room. A pair of pins a solder blob would wrongly join
/// (on different nets, or one on a net and one unused) counts twice. Parts may be as far apart as they like beyond that;
/// wire length keeps them from drifting.
fn crowding(c: &Case, g: &Geometry) -> i32 {
    // Which part takes each hole (the first one, if two collide: only a layout that isn't allowed has
    // collisions, and the placer never makes one).
    let mut part_at: HashMap<Point, usize> = HashMap::new();
    for (i, stands) in g.stands.iter().enumerate() {
        for &hole in stands {
            part_at.entry(hole).or_insert(i);
        }
    }
    // The net of the pin in each hole; a pin on no net is None.
    let mut net_at: HashMap<Point, Option<usize>> = g.pins.iter().flatten().map(|&hole| (hole, None)).collect();
    for (net, pins) in c.circuit.net_pins.iter().enumerate() {
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
    /// Another part (this index) also stands on the hole, and they share a layer there.
    OnPart(usize),
    /// The hole is off the board.
    OffBoard,
    /// The hole is on a hard keep-out (this index in `Constraints::keepout`).
    OnKeepout(usize),
    /// The hole is under this raised part's body (a board on headers), and the part reaches into a
    /// layer it takes.
    UnderModule(usize),
}

/// One thing that makes a layout impossible to build: `part` at `hole`, and why.
pub struct Clash {
    pub part: usize,
    pub hole: Point,
    pub why: Why,
}

/// Everything that makes a layout impossible to build: two parts sharing a hole and a layer (see
/// `Layers`), told apart by where: both standing on the hole, or one under a raised part's body (two
/// boards on headers can't overlap either); a part hanging off the board; a part on a keep-out. An
/// empty list means the layout is allowed. Collisions and keep-outs count the holes a part stands
/// on; hanging off the board counts its whole outline (a board on headers can't hang off the
/// perfboard either). Only the parts `placed` says are counted, so a layout can be checked while
/// it's being put together.
pub fn clashes(c: &Case, g: &Geometry, placed: &[bool]) -> Vec<Clash> {
    let (w, h) = (c.board.width, c.board.height);
    let keepouts = &c.constraints.keepout;
    let mut out = Vec::new();
    // The layers part `i` takes at a hole of its own: from the copper up at one of its pins.
    let span = |i: usize, hole: Point| if g.pins[i].contains(&hole) { c.layers(i).pin() } else { c.layers(i) };
    // The first part standing on each hole.
    let mut taken: HashMap<Point, usize> = HashMap::new();
    for (part, stands) in g.stands.iter().enumerate().filter(|&(i, _)| placed[i]) {
        for &hole in stands {
            match taken.get(&hole) {
                // It clashes with the first earlier part on the hole it shares a layer with (two can
                // share a hole on different layers: a part lying over one soldered flat).
                Some(&first) => {
                    let other = (first..part).filter(|&o| placed[o] && g.stands[o].contains(&hole)).find(|&o| span(o, hole).overlaps(span(part, hole)));
                    if let Some(other) = other {
                        out.push(Clash { part, hole, why: Why::OnPart(other) });
                    }
                }
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
    // A raised part's body, over the holes it covers but doesn't stand on: a part under it clashes
    // where they share a layer.
    for (m, (body, stands)) in g.bodies.iter().zip(&g.stands).enumerate().filter(|&(i, _)| placed[i]) {
        let under: HashSet<Point> = body.iter().filter(|p| !stands.contains(p)).copied().collect();
        if under.is_empty() {
            continue;
        }
        for part in (0..g.bodies.len()).filter(|&i| placed[i] && i != m) {
            out.extend(g.bodies[part].iter().filter(|&&h| under.contains(&h) && span(part, h).overlaps(c.layers(m))).map(|&hole| Clash { part, hole, why: Why::UnderModule(m) }));
        }
    }
    out
}

/// Is the layout allowed: can it be built? (No clashes, see `clashes`.)
pub fn allowed(c: &Case, placement: &[Place]) -> bool {
    clashes(c, &c.geometry(placement), &vec![true; placement.len()]).is_empty()
}

/// The clashes in plain words, one line per part and what's wrong with it, like "R1 and C2 share 2 holes".
pub fn clash_words(c: &Case, clashes: &[Clash]) -> Vec<String> {
    let mut seen: Vec<(usize, Why, usize)> = Vec::new();
    for clash in clashes {
        match seen.iter_mut().find(|(part, why, _)| (*part, *why) == (clash.part, clash.why)) {
            Some((.., holes)) => *holes += 1,
            None => seen.push((clash.part, clash.why, 1)),
        }
    }
    let name = |i: usize| &c.circuit.parts[i].reference;
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
pub fn report(c: &Case, placement: &[Place], m: &Measures, title: &str) -> String {
    let wrong = clashes(c, &c.geometry(placement), &vec![true; placement.len()]);
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
pub fn ascii_board(c: &Case, placement: &[Place]) -> String {
    let (w, h) = (c.board.width, c.board.height);
    let g = c.geometry(placement);
    let mut grid = vec![vec!['.'; w as usize]; h as usize];
    for (x, y) in c.constraints.keepout.iter().flat_map(|k| k.holes()) {
        grid[y as usize][x as usize] = 'x';
    }
    let on_board = |(x, y): Point| x >= 0 && x < w && y >= 0 && y < h;
    // Under a board of its own (inside its outline, not stood on): ':' where nothing else is.
    for (_, (outline, stands)) in g.outlines.iter().zip(&g.stands).enumerate().filter(|&(k, _)| c.own_board(k)) {
        for &p in outline.iter().filter(|&&p| on_board(p) && !stands.contains(&p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            if *cell == '.' {
                *cell = ':';
            }
        }
    }
    for (i, stands) in g.stands.iter().enumerate() {
        let letter = (b'a' + i as u8) as char;
        for &p in stands.iter().filter(|&&p| on_board(p)) {
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
        c.circuit.parts.iter().enumerate().map(|(i, comp)| format!("{}={}", (b'A' + i as u8) as char, comp.reference)).collect();
    if !c.constraints.keepout.is_empty() {
        legend.push("x=keep-out".into());
    }
    if (0..placement.len()).any(|k| c.own_board(k) && g.stands[k].len() < g.outlines[k].len()) {
        legend.push(":=under a module".into());
    }
    out + "\n   " + &legend.join("  ")
}
