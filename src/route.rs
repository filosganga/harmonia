//! The router: joins every run with copper traces on the solder side, and hops over with a
//! jumper where a trace can't get through or going round costs more (or leaves a walled-in pin
//! unrouted, below).
//!
//! A trace goes from a hole to a neighbouring one: across, down, or diagonally (45°). It touches
//! every hole it passes, so it can't pass a hole that belongs to another run (another run's pin or
//! trace, or a hard keep-out), and two diagonals can't cross between four holes. It may run under parts:
//! it's on the other side of the board. (A run is a net, or one branch of a star net.)
//!
//! A jumper is a wire on the parts' side. It lies straight between two free holes, across, down or
//! at 45°, up to 10 holes long, but never over a part, a pin or a hard keep-out, and never over or across
//! another run's jumper. It may pass under a module's board, like low parts. A trace carries on from
//! where it lands.
//!
//! The last resort is the same kind of wire on the solder side. Nothing sits there but copper, so
//! it may pass under parts and over traces and pins (it's insulated); it still keeps off hard
//! keep-outs and other runs' solder-side wires. It costs a hundred corners in the score, so the
//! router only takes one where no jumper fits (or the way to one costs more), and the placer avoids it. A wire's leg needs a hole of its own,
//! never a pin's: a pin walled in so that not even a wire fits is left `unrouted`, costing more
//! than any wiring.
//!
//! A soft keep-out (one with a weight) takes copper, but each hole of it costs: the router pays
//! that on every step of trace into it and every hole a wire lies over.
//!
//! A soft star net (a star rule with a weight) may break its rule: a run may join another run of
//! its net away from the star point, at a price. The router takes a join when it's cheaper than
//! the run's own way round, counting a jumper or a solder-side wire at what the score charges.
//!
//! Runs are routed one at a time (shortest first, in the first routing; see `route`), and each one is grown like a tree: from the
//! pins joined so far, find the cheapest way to the nearest pin not joined yet ("A*" search: try
//! the most promising holes first, guided by a guess of the cost still to go that's never too
//! high). A trace pays for its length, for each turn, since every bend is one more to make in
//! copper wire, and extra for each diagonal step, since straight traces are easier to lay. A
//! jumper or a solder-side wire is one more step the search may take, from any hole where its leg
//! fits, at what the score charges for it: so a way may have any number of wires, and a long trace
//! round the board loses to a jumper when the jumper is cheaper.
//!
//! Which way each run goes depends on the order the runs are routed in, and on which of several
//! equally cheap ways the search finds first; on a crowded board (pins in a row along an edge) one
//! run can take the only holes a later one needed. So for a finished layout the router makes many
//! complete routings, in different orders, and the score picks the best (`route`).

use crate::board::{Circuit, Geometry, Measures, Place, Point, WEIGHTS};
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

/// Where the copper goes. Each entry is (run, from hole, to hole).
#[derive(Default, Clone)]
pub struct Routing {
    /// One step of trace between neighbouring holes, on the solder side.
    pub traces: Vec<(usize, Point, Point)>,
    /// A jumper on the parts' side.
    pub jumpers: Vec<(usize, Point, Point)>,
    /// A wire on the solder side, the last resort.
    pub bottom: Vec<(usize, Point, Point)>,
    /// A connection the router couldn't make, as (run, the nearest hole already joined, the pin):
    /// the pin is walled in, so not even a wire fits (a wire's leg needs a hole of its own, never a
    /// pin's), or it's off the board. Only shown, never copper.
    pub unrouted: Vec<(usize, Point, Point)>,
    /// Where a run of a soft star net joined another run of its net, away from the star point:
    /// (run, the other run's hole it starts from). Its copper is in `traces`, as the run's own.
    pub joins: Vec<(usize, Point)>,
}

impl Routing {
    /// Length of copper trace, in holes.
    pub fn trace_length(&self) -> f64 {
        self.traces.iter().map(|&(_, a, b)| distance(a, b)).sum::<f64>() + 0.0 // + 0.0: no traces is 0, not -0
    }

    /// How many steps of copper trace go diagonally. Traces are laid in copper wire along the holes,
    /// and straight ones are easier to lay and can line up in parallel (or come from a stripboard's
    /// strips); jumpers and solder-side wires are wires, free to go at 45°, so they don't count.
    pub fn diagonal_steps(&self) -> f64 {
        self.traces.iter().filter(|&&(_, a, b)| a.0 != b.0 && a.1 != b.1).count() as f64
    }

    /// How many straight steps of copper trace run against the board's main direction: across or
    /// down, whichever has fewer. On a stripboard the strips run one way and every piece of copper the
    /// other way needs a wire; on perfboard too, traces side by side in one direction are the easiest
    /// to lay. The router doesn't price this, since the main direction is only known once the whole
    /// board is routed; it matters little, as a trace from one pin to another takes the same steps
    /// across and down whichever way it turns.
    pub fn crosswise_steps(&self) -> f64 {
        let across = self.traces.iter().filter(|&&(_, a, b)| a.1 == b.1).count();
        let down = self.traces.iter().filter(|&&(_, a, b)| a.0 == b.0).count();
        across.min(down) as f64
    }

    /// Length of jumper and solder-side wire, in holes.
    pub fn jumper_length(&self) -> f64 {
        self.jumpers.iter().chain(&self.bottom).map(|&(_, a, b)| distance(a, b)).sum::<f64>() + 0.0
    }

    /// Every hole with copper in it or a wire over it.
    pub fn copper_holes(&self) -> HashSet<Point> {
        let traces = self.traces.iter().flat_map(|&(_, a, b)| [a, b]);
        let wires = self.jumpers.iter().chain(&self.bottom).flat_map(|&(_, a, b)| holes_between(a, b).chain([a, b]));
        traces.chain(wires).collect()
    }

    /// Length of copper (traces, jumpers and solder-side wires), in holes, between two holes the
    /// same net joins, going the way the router laid it. A star net's runs only meet at the star
    /// point (unless they joined, see `joins`), so between two runs it goes through there.
    pub fn copper_between(&self, c: &Circuit, net: usize, from: Point, to: Point) -> f64 {
        // Every piece of copper of that net, as links between holes, both ways.
        let mut links: HashMap<Point, Vec<(Point, f64)>> = HashMap::new();
        for &(run, a, b) in self.traces.iter().chain(&self.jumpers).chain(&self.bottom) {
            if c.runs[run].net == net {
                links.entry(a).or_default().push((b, distance(a, b)));
                links.entry(b).or_default().push((a, distance(a, b)));
            }
        }
        // The shortest way along those links (Dijkstra): keep taking the closest hole not done
        // yet, and see how far its neighbours are through it. A BTreeMap, not a HashMap: its order
        // is the same every run, so two holes equally close are taken in the same order, and the
        // lengths are added up the same way (adding fractions in another order can change the last
        // digit, and a placer comparing two equal costs could then go another way).
        let mut best: BTreeMap<Point, f64> = BTreeMap::from([(from, 0.0)]);
        let mut done: HashSet<Point> = HashSet::new();
        loop {
            let closest = best.iter().filter(|(p, _)| !done.contains(*p)).min_by(|a, b| a.1.total_cmp(b.1));
            let Some((&p, &d)) = closest else {
                // Not joined: one of them must be a pin the router couldn't reach (`unrouted`),
                // which costs plenty already. Straight across, then.
                let left_out = |hole: Point| self.unrouted.iter().any(|&(_, _, pin)| pin == hole);
                assert!(left_out(from) || left_out(to), "the router left {from:?} and {to:?} unjoined, but neither is unrouted");
                return distance(from, to);
            };
            if p == to {
                return d;
            }
            done.insert(p);
            for &(q, len) in links.get(&p).into_iter().flatten() {
                if best.get(&q).is_none_or(|&old| d + len < old) {
                    best.insert(q, d + len);
                }
            }
        }
    }

    /// How hard the traces' turns are to bend, in right angles (see `bend`): at holes where exactly
    /// two steps of one run meet, not in a line. A pin or a wire's leg is free, since a trace starts
    /// there, and so is a junction of three or more steps, which is soldered like a pin.
    pub fn turns(&self, g: &Geometry) -> u32 {
        let pins: HashSet<Point> = g.pins.iter().flatten().copied().collect();
        let legs: HashSet<(usize, Point)> =
            self.jumpers.iter().chain(&self.bottom).flat_map(|&(run, a, b)| [(run, a), (run, b)]).collect();
        // For each hole of each run: the directions its steps leave in.
        let mut out_of: HashMap<(usize, Point), Vec<Point>> = HashMap::new();
        for &(run, a, b) in &self.traces {
            let d = (b.0 - a.0, b.1 - a.1);
            out_of.entry((run, a)).or_default().push(d);
            out_of.entry((run, b)).or_default().push((-d.0, -d.1));
        }
        out_of
            .iter()
            .filter(|(at, _)| !pins.contains(&at.1) && !legs.contains(at))
            .map(|(_, dirs)| match dirs[..] {
                [a, b] => bend((-a.0, -a.1), b),
                _ => 0,
            })
            .sum()
    }

    /// The traces as straight runs: steps in a row, in the same direction, joined into one.
    pub fn straight_traces(&self) -> Vec<(usize, Point, Point)> {
        let mut out: Vec<(usize, Point, Point)> = Vec::new();
        for &(run, a, b) in &self.traces {
            if let Some(last) = out.last_mut() {
                let dir = |p: Point, q: Point| ((q.0 - p.0).signum(), (q.1 - p.1).signum());
                if last.0 == run && last.2 == a && dir(last.1, last.2) == dir(a, b) {
                    last.2 = b;
                    continue;
                }
            }
            out.push((run, a, b));
        }
        out
    }
}

/// The `!!` lines for connections the router couldn't make, naming each pin left out.
pub fn unrouted_problems(c: &Circuit, g: &Geometry, routing: &Routing) -> Vec<String> {
    let name_of = |hole: Point| {
        let pins = c.components.iter().enumerate().flat_map(|(ci, comp)| comp.pins.iter().enumerate().map(move |(pi, pin)| (ci, pi, pin)));
        pins.filter(|&(ci, pi, _)| g.pins[ci][pi] == hole)
            .map(|(ci, _, pin)| format!("{}.{}", c.components[ci].reference, pin.name))
            .next()
            .unwrap_or_else(|| format!("the pin at {hole:?}"))
    };
    routing
        .unrouted
        .iter()
        .map(|&(run, _, pin)| {
            let net = &c.nets[c.runs[run].net].name;
            format!("{} isn't connected to the rest of {net}: walled in so not even a wire fits, or off the board", name_of(pin))
        })
        .collect()
}

/// Costs, in tenths of a hole of trace. A straight step is one hole, a diagonal about 1.4.
/// They match the score's weights (board::WEIGHTS) times ten, so the router and the score agree.
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
/// What a diagonal step pays on top of its length: the score's `diagonal` weight times ten. (2 is
/// `diagonal`'s place in board::FEATURES: change it if a measurement goes in before it.)
const DIAGONAL_EXTRA: u32 = (crate::board::WEIGHTS[2] * 10.0) as u32;
/// What a trace pays for each right angle's worth of bending (see `bend`), except at a pin, where
/// it starts anyway: the score's `turns` weight times ten (1 is its place in board::FEATURES).
const TURN: u32 = (crate::board::WEIGHTS[1] * 10.0) as u32;
/// How hard each turn is to bend in copper wire, counted in right angles, which are the easiest.
/// A gentle 45° turn, onto or off a diagonal, is harder to bend so it lands on the right hole, and
/// a sharp 135° turn harder still.
const RIGHT: u32 = 1;
const GENTLE: u32 = 2;
const SHARP: u32 = 3;

/// How hard it is to bend a trace arriving in direction `a` so it leaves in direction `b` (each a
/// step across, down or diagonal), in right angles; 0 if it goes straight on.
const fn bend(a: Point, b: Point) -> u32 {
    // The dot product says how far apart the two directions are: positive below 90°, zero at 90°.
    let dot = a.0 * b.0 + a.1 * b.1;
    if a.0 == b.0 && a.1 == b.1 {
        0
    } else if dot > 0 {
        GENTLE
    } else if dot == 0 {
        RIGHT
    } else {
        SHARP
    }
}

/// `AHEAD[h][g]`: at most how much more bending, in right angles, a trace that came into a hole
/// heading `h` may need from there than one that came in heading `g`, whichever way it goes on.
/// So once a hole was reached heading `h` at cost c, reaching it heading `g` at c + AHEAD[h][g] *
/// TURN or more can't lead anywhere cheaper. (START, a search's own start, turns free.)
const AHEAD: [[u32; HEADINGS]; HEADINGS] = {
    const fn bend_from(h: usize, x: usize) -> u32 {
        if h == START { 0 } else { bend(DIRS[h], DIRS[x]) }
    }
    let mut table = [[0; HEADINGS]; HEADINGS];
    let mut h = 0;
    while h < HEADINGS {
        let mut g = 0;
        while g < HEADINGS {
            let mut x = 0;
            while x < DIRS.len() {
                let (from_h, from_g) = (bend_from(h, x), bend_from(g, x));
                if from_h > from_g && from_h - from_g > table[h][g] {
                    table[h][g] = from_h - from_g;
                }
                x += 1;
            }
            g += 1;
        }
        h += 1;
    }
    table
};
/// What each hole of a jumper or solder-side wire costs: the score's `jumper_len` weight times ten
/// (5 is its place in board::FEATURES).
const JUMPER_PER_HOLE: f64 = crate::board::WEIGHTS[5] * 10.0;
/// What a jumper and a solder-side wire cost on top of their length: the score's `jumpers` and
/// `bottom` weights times ten (4 and 6 in board::FEATURES). The search weighs them against the
/// traces' way round (`spread`) and a soft star's join, so a solder-side wire, a hundred corners,
/// is only taken where no jumper fits or the way to one costs more.
const JUMPER: u32 = (crate::board::WEIGHTS[4] * 10.0) as u32;
const SOLDER_WIRE: u32 = (crate::board::WEIGHTS[6] * 10.0) as u32;
/// What breaking a soft rule costs per hole (or join) at weight 1: the score's `rules` weight times
/// ten (10 in board::FEATURES).
const RULE: f64 = crate::board::WEIGHTS[10] * 10.0;
/// The longest jumper or solder-side wire the router tries, in holes.
const MAX_JUMP: i32 = 10;
/// How many complete routings `route` proposes for a finished layout. On the cat feeder with
/// every part fixed where it's soldered (circuits/cat_feeder_fixed), shortest first leaves a pin
/// walled in. As shipped (orders from seed 0), the best of 200 routes every pin with no
/// solder-side wire: cost 677 with the star rules, 625 without; over three other seeds of orders,
/// 653 to 661 and 589 to 623. A rip-up router tried before this found 701 and 702. With fewer the
/// best was often still a pin short: 50 found 1632 on one seed. Most candidates stop early (see
/// `route_in_order`); 200 take 0.1 to 0.15s on that board. The placer judges its moves with one
/// (`route_once`).
const CANDIDATES: usize = 200;

/// Distance between two holes, in holes.
fn distance(a: Point, b: Point) -> f64 {
    (((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) as f64).sqrt()
}

/// True when two straight segments properly cross (touching at an end does not count).
fn segments_cross((p1, p2): (Point, Point), (p3, p4): (Point, Point)) -> bool {
    let cross = |o: Point, a: Point, b: Point| (a.0 - o.0) as i64 * (b.1 - o.1) as i64 - (a.1 - o.1) as i64 * (b.0 - o.0) as i64;
    let (d1, d2) = (cross(p3, p4, p1), cross(p3, p4, p2));
    let (d3, d4) = (cross(p1, p2, p3), cross(p1, p2, p4));
    d1 * d2 < 0 && d3 * d4 < 0
}

/// Every jumper the router may try, as (offset from one leg to the other, the cost of its length),
/// shortest first: straight or at 45°, at least two holes and at most MAX_JUMP holes long.
fn jumper_shapes() -> Vec<(Point, u32)> {
    let mut out: Vec<(Point, u32)> = (-MAX_JUMP..=MAX_JUMP)
        .flat_map(|dx| (-MAX_JUMP..=MAX_JUMP).map(move |dy| (dx, dy)))
        .filter(|&(dx, dy)| dx == 0 || dy == 0 || dx.abs() == dy.abs())
        .filter(|&(dx, dy)| (4..=MAX_JUMP * MAX_JUMP).contains(&(dx * dx + dy * dy)))
        .map(|d| (d, (JUMPER_PER_HOLE * distance((0, 0), d)).round() as u32))
        .collect();
    out.sort_by_key(|&(_, cost)| cost);
    out
}

const DIRS: [Point; 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Who owns a hole: nobody yet, a pin that's on no run (nothing may touch it), or a run.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    Free,
    Blocked,
    Run(usize),
}

#[derive(Clone)]
struct Grid {
    w: i32,
    h: i32,
    owner: Vec<Owner>,
    /// A star point: the net whose runs all start from this hole.
    star: Vec<Option<usize>>,
    /// Holes with a pin in them.
    pin: Vec<bool>,
    /// Holes under a part's body on the parts' side (not a module's: a jumper fits under its board).
    body: Vec<bool>,
    /// Hard keep-out holes: no part, trace or wire.
    keepout: Vec<bool>,
    /// What copper pays for each hole, on top of its usual cost: a soft keep-out's price.
    extra: Vec<u32>,
    /// For each square of four holes (by its top-left hole): the run whose diagonal crosses it,
    /// and which diagonal (true = top-left to bottom-right).
    diag: Vec<Option<(usize, bool)>>,
    /// For each side, top and bottom: the wires placed so far, (run, one leg, the other), and
    /// which run's wire has a leg in or lies over each hole.
    wires: [Vec<(usize, Point, Point)>; 2],
    wire_at: [Vec<Option<usize>>; 2],
    /// The order the search tries the eight directions in (indexes into DIRS), which decides
    /// between ways that cost the same: `route` shuffles it for each candidate.
    dir_order: [usize; 8],
}

/// Which side of the board a wire is on: the parts' side (a jumper) or the solder side.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Top = 0,
    Bottom = 1,
}

/// One step of a path: a trace between neighbouring holes, or a wire on one side.
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Trace,
    Wire(Side),
}

/// The holes strictly between the two legs of a straight or 45° wire.
fn holes_between(a: Point, b: Point) -> impl Iterator<Item = Point> {
    let (dx, dy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    (1..n).map(move |k| (a.0 + dx * k, a.1 + dy * k))
}

impl Grid {
    fn on_board(&self, (x, y): Point) -> bool {
        x >= 0 && x < self.w && y >= 0 && y < self.h
    }

    fn at(&self, (x, y): Point) -> usize {
        (y * self.w + x) as usize
    }

    fn hole(&self, i: usize) -> Point {
        (i as i32 % self.w, i as i32 / self.w)
    }

    /// Like `at`, but None for a hole off the board.
    fn index(&self, p: Point) -> Option<usize> {
        self.on_board(p).then(|| self.at(p))
    }

    /// Can `run` (on `net`) put copper in this hole?
    fn open(&self, p: Point, run: usize, net: usize) -> bool {
        let i = self.at(p);
        self.star[i] == Some(net) || self.owner[i] == Owner::Free || self.owner[i] == Owner::Run(run)
    }

    /// Can a wire on `side` lie over this hole, for `run`? Not over a keep-out, or another run's
    /// wire on the same side. On top, not over a pin or a part either; the solder side has none.
    fn passable(&self, i: usize, run: usize, side: Side) -> bool {
        let ours = self.wire_at[side as usize][i].is_none_or(|r| r == run);
        ours && !self.keepout[i] && (side == Side::Bottom || (!self.pin[i] && !self.body[i]))
    }

    /// Can a wire leg for `run`, on `side`, go in this hole? A jumper is like a part, so its leg needs
    /// a hole of its own: no pin, nobody else's copper, and a wire could lie over it.
    fn leg(&self, p: Point, run: usize, side: Side) -> bool {
        let i = self.at(p);
        !self.pin[i] && (self.owner[i] == Owner::Free || self.owner[i] == Owner::Run(run)) && self.passable(i, run, side)
    }

    /// Can a trace leave the pin at `p` for `run`, to a neighbouring hole? If not, the pin is walled
    /// in: no trace leaves it, so no wire can carry on from it either.
    fn has_way_out(&self, p: Point, run: usize, net: usize) -> bool {
        DIRS.iter().map(|&(dx, dy)| (p.0 + dx, p.1 + dy)).any(|q| self.on_board(q) && self.open(q, run, net) && self.diagonal_free(p, q, run))
    }

    /// What a wire from `a` to `b` pays for the holes it lies over (see `extra`). Its legs pay as
    /// the traces' ends.
    fn wire_extra(&self, a: Point, b: Point) -> u32 {
        holes_between(a, b).map(|p| self.extra[self.at(p)]).sum()
    }

    /// Can a wire for `run` lie straight from `a` to `b` on `side`, without crossing another run's?
    fn clear(&self, a: Point, b: Point, run: usize, side: Side) -> bool {
        holes_between(a, b).all(|p| self.passable(self.at(p), run, side))
            && !self.wires[side as usize].iter().any(|&(r, c, d)| r != run && segments_cross((a, b), (c, d)))
    }

    /// The square a diagonal step from `a` to `b` crosses, and which diagonal it is.
    fn square(&self, a: Point, b: Point) -> (usize, bool) {
        (self.at((a.0.min(b.0), a.1.min(b.1))), (b.0 - a.0) == (b.1 - a.1))
    }

    /// A diagonal step is blocked if another run's diagonal already crosses that square.
    fn diagonal_free(&self, a: Point, b: Point, run: usize) -> bool {
        let (sq, kind) = self.square(a, b);
        match self.diag[sq] {
            Some((r, k)) => r == run || k == kind,
            None => true,
        }
    }
}

/// Routes every run of the circuit, for parts at `placement` (whose geometry is `g`): makes
/// CANDIDATES complete routings, the first shortest run first and the others in random orders, and
/// keeps the one the score likes best (`board::cost`, every measurement and rule counted). The
/// router has no rules of its own about what to try again or what to keep room for: it proposes,
/// the score judges. The random orders come from a fixed seed, so the same placement always gets
/// the same copper.
pub fn route(c: &Circuit, placement: &[Place], g: &Geometry) -> Routing {
    let canonical = order(c, g);
    let empty = empty_grid(c, g);
    // What the placement alone costs (power and crowding): the same for every candidate, so a
    // candidate whose copper so far already costs more than the best's copper can't win, and stops.
    let fixed = |m: &Measures| WEIGHTS[8] * m.0[8] + WEIGHTS[9] * m.0[9];
    let mut rng = StdRng::seed_from_u64(0);
    let mut best: Option<(f64, f64, Routing)> = None; // (cost, what its copper may cost, routing)
    for k in 0..CANDIDATES {
        let mut grid = empty.clone();
        let mut order = canonical.clone();
        if k > 0 {
            order.shuffle(&mut rng);
            grid.dir_order.shuffle(&mut rng);
        }
        let budget = best.as_ref().map_or(f64::INFINITY, |b| b.1);
        let (routing, gave_up) = route_in_order(c, g, grid, &order, budget);
        if gave_up {
            continue;
        }
        let m = crate::board::measure_routed(c, placement, g, &routing);
        let cost = crate::board::cost(&m);
        if best.as_ref().is_none_or(|b| cost < b.0) {
            best = Some((cost, cost - fixed(&m), routing));
        }
    }
    // Each run's copper together, runs in `order`'s order (the layout file lists them that way),
    // whichever order the winner laid them in.
    let mut routing = best.expect("the first candidate has no budget to pass").2;
    let mut rank = vec![0; canonical.len()];
    for (k, &r) in canonical.iter().enumerate() {
        rank[r] = k;
    }
    for list in [&mut routing.traces, &mut routing.jumpers, &mut routing.bottom, &mut routing.unrouted] {
        list.sort_by_key(|&(r, ..)| rank[r]); // stable: a run's pieces stay in the order laid
    }
    routing.joins.sort_by_key(|&(r, _)| rank[r]);
    routing
}

/// Routes every run once, shortest first: what the placer judges each move by (see
/// `place::anneal`). It's `route`'s first candidate, so `route` is never worse.
pub fn route_once(c: &Circuit, g: &Geometry) -> Routing {
    route_in_order(c, g, empty_grid(c, g), &order(c, g), f64::INFINITY).0
}

/// The board as the router sees it before any copper: parts, pins, keep-outs.
fn empty_grid(c: &Circuit, g: &Geometry) -> Grid {
    let (w, h) = (c.board.width, c.board.height);
    let n = (w * h) as usize;
    let mut grid = Grid {
        w,
        h,
        owner: vec![Owner::Free; n],
        star: vec![None; n],
        pin: vec![false; n],
        body: vec![false; n],
        keepout: vec![false; n],
        extra: vec![0; n],
        diag: vec![None; n],
        wires: [Vec::new(), Vec::new()],
        wire_at: [vec![None; n], vec![None; n]],
        dir_order: [0, 1, 2, 3, 4, 5, 6, 7],
    };

    // Holes under a part's body. Not a module's: only its pins take holes, and a jumper fits under
    // its board.
    for (body, outline) in g.bodies.iter().zip(&g.outlines) {
        if body.len() == outline.len() {
            for &p in body {
                if let Some(i) = grid.index(p) {
                    grid.body[i] = true;
                }
            }
        }
    }
    // A keep-out (a screw hole, say) takes no copper either; a soft one does, at its price.
    for &p in &c.constraints.keepout_holes {
        let i = grid.at(p);
        (grid.keepout[i], grid.owner[i]) = (true, Owner::Blocked);
    }
    // Not under a part, though: the part pays for that hole already, and the score counts it once.
    let under_parts: HashSet<Point> = g.bodies.iter().flatten().copied().collect();
    for (&p, &weight) in c.constraints.soft_keepout.iter().filter(|(p, _)| !under_parts.contains(p)) {
        let i = grid.at(p);
        grid.extra[i] = (RULE * weight).round() as u32;
    }
    // Every pin's hole belongs to its run; a pin on no run blocks its hole.
    for &p in g.pins.iter().flatten() {
        if let Some(i) = grid.index(p) {
            (grid.pin[i], grid.owner[i]) = (true, Owner::Blocked);
        }
    }
    for (r, run) in c.runs.iter().enumerate() {
        for &(ci, pi) in &run.pins {
            if let Some(i) = grid.index(g.pins[ci][pi]) {
                if run.star == Some((ci, pi)) {
                    grid.star[i] = Some(run.net);
                }
                grid.owner[i] = Owner::Run(r);
            }
        }
    }
    // A pin between holes could touch copper in any hole around it, so those holes are its run's
    // only (or nobody's, for a pin on no run, or where two such pins of different runs meet).
    let run_of: std::collections::HashMap<(usize, usize), usize> =
        c.runs.iter().enumerate().flat_map(|(r, run)| run.pins.iter().map(move |&p| (p, r))).collect();
    for (ci, comp) in c.components.iter().enumerate() {
        for (pi, _) in comp.pins.iter().enumerate().filter(|(_, pin)| pin.off_grid) {
            let (x, y) = g.pins[ci][pi];
            let mine = run_of.get(&(ci, pi)).map_or(Owner::Blocked, |&r| Owner::Run(r));
            for p in (x - 1..=x + 1).flat_map(|nx| (y - 1..=y + 1).map(move |ny| (nx, ny))) {
                if let Some(i) = grid.index(p).filter(|&i| !grid.pin[i]) {
                    grid.owner[i] = if grid.owner[i] == Owner::Free || grid.owner[i] == mine { mine } else { Owner::Blocked };
                }
            }
        }
    }
    grid
}

/// Routes the runs one at a time, in `order`, on `grid` (the parts and keep-outs, no copper yet).
/// It keeps a running total of what the score will charge for the copper, counting only what can
/// only grow as more is laid (wire, diagonal, crosswise, jumpers, jumper_len, bottom, unrouted;
/// turns can shrink, when a later step makes a bend a junction), and gives up (true) once that
/// passes `budget`.
fn route_in_order(c: &Circuit, g: &Geometry, mut grid: Grid, order: &[usize], budget: f64) -> (Routing, bool) {
    let w = WEIGHTS;
    let (mut wire, mut diagonal, mut across, mut down, mut wire_len) = (0.0, 0.0, 0usize, 0usize, 0.0);
    let n = (grid.w * grid.h) as usize;
    let holes_of = |r: usize| -> Vec<Point> { c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let mut out = Routing::default();
    let mut mem = Scratch::new(n);
    let shapes = jumper_shapes();
    for &r in order {
        let run = &c.runs[r];
        // A star's runs start at the star point; another run at its first pin a trace can leave,
        // so a walled-in pin is the one left unrouted, and named as such.
        let start = run.star.unwrap_or_else(|| {
            let way_out = |&&(ci, pi): &&(usize, usize)| grid.on_board(g.pins[ci][pi]) && grid.has_way_out(g.pins[ci][pi], r, run.net);
            *run.pins.iter().find(way_out).unwrap_or(&run.pins[0])
        });
        let start = g.pins[start.0][start.1];
        let (mut remaining, off): (Vec<Point>, Vec<Point>) =
            holes_of(r).into_iter().filter(|&p| p != start).partition(|&p| grid.on_board(p) && grid.on_board(start));
        // A pin off the board can't be routed to: it's left unrouted. Only a layout that isn't
        // allowed has one (a sketch as you drew it); the placer never makes such a layout.
        out.unrouted.extend(off.into_iter().map(|p| (r, start, p)));
        let mut tree = vec![start];
        // A soft star's weight, if this run is a branch of one.
        let soft_star = c.constraints.stars.iter().find(|s| run.star.is_some() && s.net == run.net).and_then(|s| s.weight);
        while !remaining.is_empty() {
            // The pin nearest to what's joined so far.
            let (k, _) = remaining.iter().enumerate().min_by_key(|&(_, &p)| tree.iter().map(|&t| octile(t, p)).min().unwrap()).unwrap();
            let target = remaining.swap_remove(k);
            let own = spread(&grid, &mut mem, &tree, Some(target), r, run.net, Some(&shapes), |p, _| p == target).then(|| {
                let i = grid.at(target);
                (mem.reached(i).unwrap(), mem.path(&grid, i))
            });
            // A soft star's run may join another run of its net instead, if that's cheaper.
            let joined = soft_star.and_then(|weight| {
                let others = out.traces.iter().chain(&out.jumpers).chain(&out.bottom).filter(|&&(o, ..)| o != r && c.runs[o].net == run.net);
                // Not the star point: starting there is the run's own way, not a join.
                let others: Vec<Point> = others.flat_map(|&(_, a, b)| [a, b]).filter(|&p| p != start).collect();
                join(&grid, &mut mem, &others, target, r, run.net).map(|(cost, path)| (cost + (RULE * weight).round() as u32, path))
            });
            let path = match (own, joined) {
                (own, Some((cost, path))) if own.as_ref().is_none_or(|(own_cost, _)| cost < *own_cost) => {
                    out.joins.push((r, path[0].0));
                    path
                }
                (Some((_, path)), _) => path,
                (None, _) => {
                    // Walled in even for a wire on either side. A wire can't end on the pin (its leg
                    // needs a hole of its own), so the pin is left unrouted: very expensive, so the
                    // placer keeps away from layouts like this one.
                    let from = *tree.iter().min_by_key(|&&t| octile(t, target)).unwrap();
                    out.unrouted.push((r, from, target));
                    continue;
                }
            };
            for (a, b, step) in path {
                if step == Step::Trace {
                    wire += distance(a, b);
                    diagonal += if a.0 != b.0 && a.1 != b.1 { 1.0 } else { 0.0 };
                    across += (a.1 == b.1) as usize;
                    down += (a.0 == b.0) as usize;
                } else {
                    wire_len += distance(a, b);
                }
                lay(&mut grid, &mut out, r, a, b, step);
                tree.push(b);
                remaining.retain(|&p| p != b);
            }
            let so_far = w[0] * wire + w[2] * diagonal + w[3] * across.min(down) as f64 + w[4] * out.jumpers.len() as f64
                + w[5] * wire_len + w[6] * out.bottom.len() as f64 + w[7] * out.unrouted.len() as f64;
            if so_far > budget {
                return (out, true);
            }
        }
    }
    (out, false)
}

/// Lays one step of `run`'s copper from `a` to `b`: in `out`, and on `grid`, where it now takes
/// its holes (except a star point, which stays open to the net's other runs).
fn lay(grid: &mut Grid, out: &mut Routing, run: usize, a: Point, b: Point, step: Step) {
    if let Step::Wire(side) = step {
        let list = if side == Side::Top { &mut out.jumpers } else { &mut out.bottom };
        list.push((run, a, b));
        grid.wires[side as usize].push((run, a, b));
        for p in holes_between(a, b).chain([a, b]) {
            let i = grid.at(p);
            grid.wire_at[side as usize][i] = Some(run);
        }
    } else {
        out.traces.push((run, a, b));
        if a.0 != b.0 && a.1 != b.1 {
            let (sq, kind) = grid.square(a, b);
            grid.diag[sq] = Some((run, kind));
        }
    }
    for p in [a, b] {
        let i = grid.at(p);
        if grid.star[i].is_none() {
            grid.owner[i] = Owner::Run(run);
        }
    }
}

/// The order `route_once` routes the runs in, and `route`'s first candidate: shortest first, since
/// they have the fewest ways around things. A run's copper is laid all at once, and `route` hands
/// back each run's pieces in this order whichever order won, so in a `Routing` each run's pieces
/// come one after the other, runs in this order (the layout file keeps them that way).
pub fn order(c: &Circuit, g: &Geometry) -> Vec<usize> {
    let span = |r: usize| {
        let ps: Vec<Point> = c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect();
        let (xs, ys) = (ps.iter().map(|p| p.0), ps.iter().map(|p| p.1));
        xs.clone().max().unwrap() - xs.min().unwrap() + ys.clone().max().unwrap() - ys.min().unwrap()
    };
    let mut order: Vec<usize> = (0..c.runs.len()).collect();
    order.sort_by_cached_key(|&r| span(r));
    order
}

/// The cost in tenths of a hole if nothing were in the way: diagonals where they're cheaper than
/// a step across and one down. The search's guess must never be more than the real cost, or it can
/// miss the cheapest way.
fn octile(a: Point, b: Point) -> u32 {
    let (dx, dy) = ((a.0 - b.0).unsigned_abs(), (a.1 - b.1).unsigned_abs());
    let diagonal = (DIAGONAL + DIAGONAL_EXTRA).min(2 * STRAIGHT);
    STRAIGHT * dx.max(dy) + (diagonal - STRAIGHT) * dx.min(dy)
}

/// The search doesn't just go from hole to hole: it remembers which way it came in, so that a
/// step in another direction can pay for the turn. Each hole is HEADINGS places to be: arrived
/// along one of the eight DIRS, or not arrived at all (where a search starts), which turns free.
const HEADINGS: usize = DIRS.len() + 1;
const START: usize = DIRS.len();

/// Memory for `spread`, kept between searches so each one doesn't start by allocating and
/// clearing its own. Indexed by state: hole * HEADINGS + heading. `seen[s] == round` says
/// `cost[s]` and `prev[s]` belong to this search; `popped[i] == round` says hole `i` was handled,
/// and `first[i]` is the state it was first handled in, its cheapest.
struct Scratch {
    cost: Vec<u32>,
    prev: Vec<usize>,
    seen: Vec<u32>,
    popped: Vec<u32>,
    first: Vec<usize>,
    /// How each state was reached from `prev`: by a trace step, or by a wire on one side.
    via: Vec<Step>,
    round: u32,
    queue: BinaryHeap<Reverse<(u32, u32, usize)>>,
}

impl Scratch {
    fn new(holes: usize) -> Scratch {
        let states = holes * HEADINGS;
        Scratch { cost: vec![0; states], prev: vec![0; states], seen: vec![0; states], popped: vec![0; holes], first: vec![0; holes], via: vec![Step::Trace; states], round: 0, queue: BinaryHeap::new() }
    }

    /// Cost to reach hole `i` in the last search, coming in from any direction, if it was reached.
    /// (The search handles every hole it reaches, unless it stopped early: then only those.)
    fn reached(&self, i: usize) -> Option<u32> {
        (self.popped[i] == self.round).then(|| self.cost[self.first[i]])
    }

    /// The steps from where the last search started to hole `i`, the cheapest way.
    fn path(&self, grid: &Grid, i: usize) -> Path {
        assert!(self.popped[i] == self.round, "path to a hole the search didn't reach");
        let mut s = self.first[i];
        let mut out = Vec::new();
        while self.prev[s] != usize::MAX {
            let p = self.prev[s];
            out.push((grid.hole(p / HEADINGS), grid.hole(s / HEADINGS), self.via[s]));
            s = p;
        }
        out.reverse();
        out
    }
}

/// Spreads traces out from `from`, cheapest hole first, calling `done(hole, cost)` on each hole
/// the first (cheapest) time it gets there; stops when `done` says so (then returns true) or when
/// it runs out of holes it can reach. `mem` then holds the cheapest cost to every hole reached,
/// and the way back. A trace pays TURN per right angle of bending (`bend`), except at a pin.
/// With `towards`, it tries holes in that direction first ("A*"), which finds it sooner.
/// With `shapes`, it also goes over wires: from a hole where a wire's leg fits, a jumper or a
/// solder-side wire of any of those shapes, at what the score charges for it (`JUMPER` or
/// `SOLDER_WIRE`, and its length), landing where a trace carries on. A wire doesn't start from a
/// hole a wire landed in, which would put two legs in one hole.
#[allow(clippy::too_many_arguments)]
fn spread(grid: &Grid, mem: &mut Scratch, from: &[Point], towards: Option<Point>, run: usize, net: usize, shapes: Option<&[(Point, u32)]>, mut done: impl FnMut(Point, u32) -> bool) -> bool {
    let guess = |p: Point| towards.map_or(0, |t| octile(p, t));
    mem.round += 1;
    mem.queue.clear();
    let Scratch { cost, prev, seen, popped, first, via, round, queue } = mem;
    let round = *round;
    for &p in from {
        let s = grid.at(p) * HEADINGS + START;
        (cost[s], prev[s], seen[s], via[s]) = (0, usize::MAX, round, Step::Trace);
        queue.push(Reverse((guess(p), 0, s)));
    }
    // Wires are tried lazily: when the search first gets to a hole where a leg fits, it queues the
    // hole's wires on each side as one entry, at what the cheapest of them could cost (WIRES + 2 *
    // hole + side, past the holes' own states), and only works out which fit if the search gets
    // that far. Most connections are made in traces long before, and then no wire is tried.
    const WIRES: usize = usize::MAX / 2;
    while let Some(Reverse((_, so_far, s))) = queue.pop() {
        if s >= WIRES {
            let (i, side) = ((s - WIRES) / 2, if (s - WIRES).is_multiple_of(2) { Side::Top } else { Side::Bottom });
            let (p, from) = (grid.hole(i), first[i]);
            let price = if side == Side::Top { JUMPER } else { SOLDER_WIRE };
            for &((dx, dy), length) in shapes.unwrap_or_default() {
                let q = (p.0 + dx, p.1 + dy);
                if !grid.on_board(q) || popped[grid.at(q)] == round || !grid.leg(q, run, side) || !grid.clear(p, q, run, side) {
                    continue;
                }
                let t = grid.at(q) * HEADINGS + START;
                let c = cost[from] + price + length + grid.wire_extra(p, q) + grid.extra[grid.at(q)];
                if seen[t] != round || c < cost[t] {
                    (cost[t], prev[t], seen[t], via[t]) = (c, from, round, Step::Wire(side));
                    queue.push(Reverse((c + guess(q), c, t)));
                }
            }
            continue;
        }
        if so_far > cost[s] {
            continue; // already got here, the same way in, more cheaply
        }
        let (i, heading) = (s / HEADINGS, s % HEADINGS);
        let p = grid.hole(i);
        if popped[i] != round {
            (popped[i], first[i]) = (round, s);
            if done(p, so_far) {
                return true;
            }
            // Wires from here: only the first time here, the cheapest, since a wire leaves in any
            // direction without a turn to pay.
            for side in [Side::Top, Side::Bottom] {
                let Some(shapes) = shapes else { break };
                if via[s] != Step::Trace || !grid.leg(p, run, side) {
                    continue;
                }
                let price = if side == Side::Top { JUMPER } else { SOLDER_WIRE };
                let on_board = shapes.iter().filter(|&&((dx, dy), _)| grid.on_board((p.0 + dx, p.1 + dy)));
                let Some(least) = on_board.map(|&((dx, dy), length)| length + guess((p.0 + dx, p.1 + dy))).min() else { continue };
                queue.push(Reverse((so_far + price + least, so_far + price, WIRES + 2 * i + side as usize)));
            }
        } else if grid.pin[i] || so_far >= cost[first[i]] + AHEAD[first[i] % HEADINGS][heading] * TURN {
            // Coming in the cheapest way and turning would do as well: nothing new from here.
            continue;
        }
        for k in grid.dir_order {
            let (dx, dy) = DIRS[k];
            let q = (p.0 + dx, p.1 + dy);
            let diagonal = dx != 0 && dy != 0;
            if grid.on_board(q) && grid.open(q, run, net) && (!diagonal || grid.diagonal_free(p, q, run)) {
                let turn = if heading == START || grid.pin[i] { 0 } else { bend(DIRS[heading], DIRS[k]) * TURN };
                let j = grid.at(q);
                let c = so_far + if diagonal { DIAGONAL + DIAGONAL_EXTRA } else { STRAIGHT } + turn + grid.extra[j];
                let t = j * HEADINGS + k;
                let beaten = popped[j] == round && (grid.pin[j] || c >= cost[first[j]] + AHEAD[first[j] % HEADINGS][k] * TURN);
                if !beaten && (seen[t] != round || c < cost[t]) {
                    (cost[t], prev[t], seen[t], via[t]) = (c, s, round, Step::Trace);
                    queue.push(Reverse((c + guess(q), c, t)));
                }
            }
        }
    }
    false
}

/// A way from one hole to another: (from, to, what it is) for each step.
type Path = Vec<(Point, Point, Step)>;

/// For a run of a soft star net: the cheapest way in traces from any hole of `others` (the copper
/// of the net's other runs) to `target`, and its cost, without the join's own price. None if
/// traces don't get through, or there's no other copper yet.
fn join(grid: &Grid, mem: &mut Scratch, others: &[Point], target: Point, run: usize, net: usize) -> Option<(u32, Path)> {
    if others.is_empty() || !spread(grid, mem, others, Some(target), run, net, None, |p, _| p == target) {
        return None;
    }
    let i = grid.at(target);
    Some((mem.reached(i).unwrap(), mem.path(grid, i)))
}

/// Your own wiring, drawn in a DIYLC sketch, as a routing, so it's measured the way the router's
/// is. Each trace is cut into steps between neighbouring holes (one drawn at another angle than
/// across, down or 45° stays one step), and a step drawn twice counts once. Each piece goes to a
/// run of its net: on a star net, to the run whose pins it reaches without passing the star point.
/// Copper that reaches the pins of two runs that way joins them, as a soft star's join does; it's
/// counted as a join for a soft star, and for a hard one returned as a count of breaks, which the
/// router never makes. A pin your copper doesn't reach is `unrouted`.
pub fn from_sketch(c: &Circuit, g: &Geometry, copper: &crate::diylc::Copper) -> (Routing, usize) {
    let pin_hole = |(ci, pi): (usize, usize)| g.pins[ci][pi];
    // Every step of trace, once, by net.
    let mut steps: Vec<(usize, Point, Point)> = Vec::new();
    let mut seen: HashSet<(usize, Point, Point)> = HashSet::new();
    for &(net, a, b) in &copper.traces {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let pieces: Vec<(Point, Point)> = if dx == 0 || dy == 0 || dx.abs() == dy.abs() {
            holes_between(a, b).chain([b]).scan(a, |from, to| Some((std::mem::replace(from, to), to))).collect()
        } else {
            vec![(a, b)]
        };
        for (p, q) in pieces {
            if p != q && seen.insert((net, p.min(q), p.max(q))) {
                steps.push((net, p, q));
            }
        }
    }
    let wires = |list: &[(usize, Point, Point)]| list.iter().filter(|&&(_, a, b)| a != b).copied().collect::<Vec<_>>();
    let (jumpers, bottom) = (wires(&copper.jumpers), wires(&copper.bottom));

    // Which pieces are joined, by hole, net by net (a union-find): all of the net's copper, and
    // the same without the star point, which splits a star net into its branches.
    fn find(parent: &mut HashMap<Point, Point>, p: Point) -> Point {
        let up = *parent.entry(p).or_insert(p);
        if up == p {
            return p;
        }
        let root = find(parent, up);
        parent.insert(p, root);
        root
    }
    let mut out = Routing::default();
    let mut hard_breaks = 0;
    for (net, _) in c.nets.iter().enumerate() {
        let runs: Vec<usize> = (0..c.runs.len()).filter(|&r| c.runs[r].net == net).collect();
        let Some(&first_run) = runs.first() else { continue };
        let star = c.runs[first_run].star.map(pin_hole);
        let pieces: Vec<(Point, Point, Option<Side>)> = steps.iter().map(|&(n, a, b)| (n, a, b, None))
            .chain(jumpers.iter().map(|&(n, a, b)| (n, a, b, Some(Side::Top))))
            .chain(bottom.iter().map(|&(n, a, b)| (n, a, b, Some(Side::Bottom))))
            .filter(|&(n, ..)| n == net)
            .map(|(_, a, b, side)| (a, b, side))
            .collect();
        let (mut whole, mut branches) = (HashMap::new(), HashMap::new());
        for &(a, b, _) in &pieces {
            let (ra, rb) = (find(&mut whole, a), find(&mut whole, b));
            whole.insert(ra, rb);
            if Some(a) != star && Some(b) != star {
                let (ra, rb) = (find(&mut branches, a), find(&mut branches, b));
                branches.insert(ra, rb);
            }
        }
        // Each branch's runs: those with a pin in it (not counting the star point).
        let mut runs_in: BTreeMap<Point, Vec<usize>> = BTreeMap::new();
        for &r in &runs {
            for &pin in &c.runs[r].pins {
                let hole = pin_hole(pin);
                if Some(hole) != star {
                    let branch = find(&mut branches, hole);
                    let list = runs_in.entry(branch).or_default();
                    if !list.contains(&r) {
                        list.push(r);
                    }
                }
            }
        }
        let soft = c.constraints.stars.iter().find(|s| s.net == net).and_then(|s| s.weight).is_some();
        for (&branch, list) in &runs_in {
            for &r in &list[1..] {
                if star.is_some() && soft {
                    out.joins.push((r, branch));
                } else if star.is_some() {
                    hard_breaks += 1;
                }
            }
        }
        let run_of = |p: Point, branches: &mut HashMap<Point, Point>| {
            let branch = find(branches, p);
            runs_in.get(&branch).map_or(first_run, |list| list[0])
        };
        for &(a, b, side) in &pieces {
            let r = run_of(if Some(a) == star { b } else { a }, &mut branches);
            match side {
                None => out.traces.push((r, a, b)),
                Some(Side::Top) => out.jumpers.push((r, a, b)),
                Some(Side::Bottom) => out.bottom.push((r, a, b)),
            }
        }
        // A pin the copper doesn't join to the rest of its net: the rest being the star point, or
        // else the biggest group of the net's pins the copper joins.
        let pins: Vec<((usize, usize), Point)> = c.net_pins[net].iter().map(|&p| (p, pin_hole(p))).collect();
        let anchor = star.unwrap_or_else(|| {
            let group = |hole: Point, whole: &mut HashMap<Point, Point>| find(whole, hole);
            let roots: Vec<Point> = pins.iter().map(|&(_, hole)| group(hole, &mut whole)).collect();
            let biggest = (0..pins.len()).max_by_key(|&i| (roots.iter().filter(|&&r| r == roots[i]).count(), std::cmp::Reverse(i))).unwrap();
            pins[biggest].1
        });
        let main = find(&mut whole, anchor);
        for &(pin, hole) in &pins {
            if find(&mut whole, hole) != main {
                let r = runs.iter().copied().find(|&r| c.runs[r].pins.contains(&pin)).unwrap_or(first_run);
                out.unrouted.push((r, anchor, hole));
            }
        }
    }
    (out, hard_breaks)
}

