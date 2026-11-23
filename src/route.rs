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
//! another run's jumper. It may pass under a module on headers, like low parts. A trace carries on from
//! where it lands.
//!
//! The last resort is the same kind of wire on the solder side. Nothing sits there but copper, so
//! it may pass under parts and over traces and pins (it's insulated); it still keeps off hard
//! keep-outs and other runs' solder-side wires. It costs a hundred corners in the score, so the
//! router only takes one where no jumper fits (or the way to one costs more), and the placer avoids it. A wire's leg needs a hole of its own,
//! never a pin's or another wire's leg: a pin walled in so that not even a wire fits is left
//! `unrouted`, costing more than any wiring.
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
//! complete routings, in different orders, and the score picks the best (`route`). Each is still
//! built run after run, each the cheapest way the ones before it left, so no run ever leaves room
//! for a later one: the best is then improved a few runs at a time (`improve`), every change judged
//! by the score.
//!
//! `measure` and `measure_once` route a placement (the thorough way, or once, shortest first) and
//! then score it with `measure::measure_routed`: the score never routes, the router asks the score.

mod search;

use crate::board::{COPPER, Face, Geometry, LYING, Place, Point, distance};
use crate::layout::{Copper, JumperWire, Trace};
use crate::measure::{Measures, WEIGHTS, measure_routed};
use crate::rules::Case;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use search::{Grid, RULE, Scratch, Step, empty_grid, join, jumper_shapes, lay, octile, spread};

/// Where the router lays the copper, run by run: its own working copy. Each entry is (run, from
/// hole, to hole). A layout holds only the copper (`copper`).
#[derive(Default, Clone)]
struct Routing {
    /// One step of trace between neighbouring holes, on the solder side.
    traces: Vec<(usize, Point, Point)>,
    /// A jumper on the parts' side.
    jumpers: Vec<(usize, Point, Point)>,
    /// A wire on the solder side, the last resort.
    bottom: Vec<(usize, Point, Point)>,
    /// A connection the router couldn't make, as (run, the nearest hole already joined, the pin):
    /// the pin is walled in, so not even a wire fits (a wire's leg needs a hole of its own, never a
    /// pin's), or it's off the board. Never copper: the score works out what's unrouted from the
    /// copper (`measure::built`); this only keeps the router's running total honest.
    unrouted: Vec<(usize, Point, Point)>,
}

impl Routing {
    /// The copper, as a layout holds it: each run's steps of trace in a row become one trace (a new
    /// one wherever the run changes or the steps don't follow on), then the wires, those on the
    /// top face first. No runs: which pins it joins is worked out from the copper itself.
    fn copper(&self) -> Copper {
        let mut traces: Vec<Trace> = Vec::new();
        let mut last = None;
        for &(run, a, b) in &self.traces {
            match traces.last_mut() {
                Some(t) if last == Some((run, a)) => t.holes.push(b),
                _ => traces.push(Trace { layer: -COPPER, holes: vec![a, b] }),
            }
            last = Some((run, b));
        }
        let top = self.jumpers.iter().map(|&(_, a, b)| JumperWire { layer: LYING, pins: [a, b] });
        let bottom = self.bottom.iter().map(|&(_, a, b)| JumperWire { layer: -LYING, pins: [a, b] });
        Copper { traces, wires: top.chain(bottom).collect() }
    }
}

/// How many complete routings `route` proposes for a finished layout. On the cat feeder with
/// every part fixed where it's soldered (circuits/cat_feeder_fixed), shortest first leaves a pin
/// walled in. As shipped (orders from seed 0), the best of 200 routes every pin with no
/// solder-side wire: cost 677 with the star rules, 625 without (before `improve`); over three other
/// seeds of orders, 653 to 661 and 589 to 623. A router tried before this, which ripped up runs
/// while building them with tolls on the holes they fought over, found 701 and 702.
/// With fewer the best was often still a pin short: 50 found 1632 on one seed. Most candidates stop
/// early (see `route_in_order`); 200 take 0.1 to 0.15s on that board. The placer judges its moves
/// with one (`route_once`).
const CANDIDATES: usize = 200;

/// Routes every run of the circuit, for parts at `placement` (whose geometry is `g`): makes
/// CANDIDATES complete routings, the first shortest run first and the others in random orders, and
/// keeps the one the score likes best (`measure::cost`, every measurement and rule counted), then
/// improves it a few runs at a time (`improve`). The router has no rules of its own about what to
/// try again or what to keep room for: it proposes, the score judges. The random choices come from
/// fixed seeds, so the same placement always gets the same copper.
pub fn route(c: &Case, placement: &[Place], g: &Geometry) -> Copper {
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
        let (routing, gave_up) = route_in_order(c, g, grid, Routing::default(), &order, budget);
        if gave_up {
            continue;
        }
        let m = measure_routed(c, placement, g, &routing.copper());
        let cost = crate::measure::cost(&m);
        if best.as_ref().is_none_or(|b| cost < b.0) {
            best = Some((cost, cost - fixed(&m), routing));
        }
    }
    let routing = improve(c, placement, g, best.expect("the first candidate has no budget to pass").2);
    // Each run's copper together, runs in `order`'s order (the layout file lists them that way),
    // whichever order the winner laid them in.
    let mut routing = routing;
    let mut rank = vec![0; canonical.len()];
    for (k, &r) in canonical.iter().enumerate() {
        rank[r] = k;
    }
    for list in [&mut routing.traces, &mut routing.jumpers, &mut routing.bottom] {
        list.sort_by_key(|&(r, ..)| rank[r]); // stable: a run's pieces stay in the order laid
    }
    routing.copper()
}

/// Routes every run once, shortest first: what the placer judges each move by (see
/// `place::anneal`). It's `route`'s first candidate, so `route` is never worse.
pub fn route_once(c: &Case, g: &Geometry) -> Copper {
    route_in_order(c, g, empty_grid(c, g), Routing::default(), &order(c, g), f64::INFINITY).0.copper()
}

/// Routes the parts at `placement` the thorough way (`route`), then measures the layout
/// (`measure::measure_routed`).
pub fn measure(c: &Case, placement: &[Place]) -> Measures {
    let g = c.geometry(placement);
    let copper = route(c, placement, &g);
    measure_routed(c, placement, &g, &copper)
}

/// The same, routed once, shortest first (`route_once`): what the placer judges each move by.
pub fn measure_once(c: &Case, placement: &[Place]) -> Measures {
    let g = c.geometry(placement);
    let copper = route_once(c, &g);
    measure_routed(c, placement, &g, &copper)
}

/// How many changes `improve` tries. On the cat feeder with every part fixed where it's soldered
/// (circuits/cat_feeder_fixed), the best of the 200 candidates costs 677 (625 without the star
/// rules); improved, 627 after 1000 tries, 622 after 2000 (582 without the star rules), 615 after
/// 4000, each 2000 about 1.3s on that board. On the same parts as the hand-wired sketch, with a
/// jumper costing 95 instead of 15: the best candidate costs 1503 with 9 jumpers, the hand wiring
/// 1349 with 4, and improved, 1123 with 6. The candidates alone never got below the hand wiring
/// whatever the jumper price: built run after run, each the cheapest way the ones before left, no
/// run leaves room for a later one. Changing a few runs at a time does. (Measured before corners and
/// star joins were counted from the copper and a hole held one wire leg; since then the fixed board's
/// improved routing costs 610.)
const IMPROVE: usize = 2000;

/// Makes a finished routing better, a few runs at a time: rips up a random run and every run with
/// copper or a pin in the box around its own (grown by 0 to 3 holes, at random), routes them again in a
/// random order, and keeps the change if the whole board costs less (`measure::cost`). The score
/// judges every change, as it judges the candidates; the random choices come from a fixed seed.
fn improve(c: &Case, placement: &[Place], g: &Geometry, start: Routing) -> Routing {
    let cost = |r: &Routing| crate::measure::cost(&measure_routed(c, placement, g, &r.copper()));
    let mut rng = StdRng::seed_from_u64(1);
    let (mut best, mut best_cost) = (start.clone(), cost(&start));
    // The holes a run's pins and copper take.
    let holes = |routing: &Routing, r: usize| -> Vec<Point> {
        let copper = routing.traces.iter().chain(&routing.jumpers).chain(&routing.bottom).filter(|t| t.0 == r).flat_map(|&(_, a, b)| [a, b]);
        c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).chain(copper).collect()
    };
    for _ in 0..IMPROVE {
        let r = rng.random_range(0..c.runs.len());
        let ((x0, y0), (x1, y1)) = crate::board::bounds(&holes(&best, r));
        let grow = rng.random_range(0..=3);
        let near = |p: &Point| (x0 - grow..=x1 + grow).contains(&p.0) && (y0 - grow..=y1 + grow).contains(&p.1);
        let mut again: Vec<usize> = (0..c.runs.len()).filter(|&o| o == r || holes(&best, o).iter().any(near)).collect();
        // A soft star's runs may have joined each other's copper: they go again together, so none
        // is left joined to copper that moved.
        let soft = |net: usize| c.constraints.stars.iter().any(|s| s.net == net && s.weight.is_some());
        let nets: Vec<usize> = again.iter().map(|&o| c.runs[o].net).filter(|&net| soft(net)).collect();
        let together: Vec<usize> = (0..c.runs.len()).filter(|o| !again.contains(o) && nets.contains(&c.runs[*o].net)).collect();
        again.extend(together);
        again.shuffle(&mut rng);
        // The rest stays as it is: laid on the board first, for the runs routed again to go round.
        let mut grid = empty_grid(c, g);
        grid.dir_order.shuffle(&mut rng);
        let mut kept = Routing::default();
        for (list, step) in [(&best.traces, Step::Trace), (&best.jumpers, Step::Wire(Face::Top)), (&best.bottom, Step::Wire(Face::Bottom))] {
            for &(run, a, b) in list.iter().filter(|t| !again.contains(&t.0)) {
                lay(&mut grid, &mut kept, run, a, b, step);
            }
        }
        kept.unrouted = best.unrouted.iter().filter(|t| !again.contains(&t.0)).copied().collect();
        let (changed, _) = route_in_order(c, g, grid, kept, &again, f64::INFINITY);
        let now = cost(&changed);
        if now < best_cost {
            (best, best_cost) = (changed, now);
        }
    }
    best
}

/// Routes the runs one at a time, in `order`, on `grid` (the parts and keep-outs, and any copper
/// already laid, which `out` holds).
/// It keeps a running total of what the score will charge for the copper, counting only what can
/// only grow as more is laid (wire, diagonal, crosswise, jumpers, jumper_len, bottom, unrouted;
/// turns can shrink, when a later step makes a bend a junction), and gives up (true) once that
/// passes `budget`. The total counts the jumpers, wires and unrouted pins already in `out`, but not
/// its traces: a budget only means something when `out` starts empty (`improve` gives none).
fn route_in_order(c: &Case, g: &Geometry, mut grid: Grid, mut out: Routing, order: &[usize], budget: f64) -> (Routing, bool) {
    let w = WEIGHTS;
    let (mut wire, mut diagonal, mut across, mut down, mut wire_len) = (0.0, 0.0, 0usize, 0usize, 0.0);
    let n = (grid.w * grid.h) as usize;
    let holes_of = |r: usize| -> Vec<Point> { c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
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
            let own = spread(&grid, &mut mem, &tree, target, r, run.net, Some(&shapes)).then(|| mem.found(&grid, target));
            // A soft star's run may join another run of its net instead, if that's cheaper.
            let joined = soft_star.and_then(|weight| {
                let others = out.traces.iter().chain(&out.jumpers).chain(&out.bottom).filter(|&&(o, ..)| o != r && c.runs[o].net == run.net);
                // Not the star point: starting there is the run's own way, not a join.
                let others: Vec<Point> = others.flat_map(|&(_, a, b)| [a, b]).filter(|&p| p != start).collect();
                join(&grid, &mut mem, &others, target, r, run.net).map(|(cost, path)| (cost + (RULE * weight).round() as u32, path))
            });
            let path = match (own, joined) {
                (own, Some((cost, path))) if own.as_ref().is_none_or(|(own_cost, _)| cost < *own_cost) => path,
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

/// The order `route_once` routes the runs in, and `route`'s first candidate: shortest first, since
/// they have the fewest ways around things. `route` sorts the winner's pieces into this order before
/// turning them into copper, so the same placement always gives its traces and wires in the same
/// order.
fn order(c: &Case, g: &Geometry) -> Vec<usize> {
    let span = |r: usize| {
        let ps: Vec<Point> = c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect();
        let (xs, ys) = (ps.iter().map(|p| p.0), ps.iter().map(|p| p.1));
        xs.clone().max().unwrap() - xs.min().unwrap() + ys.clone().max().unwrap() - ys.min().unwrap()
    };
    let mut order: Vec<usize> = (0..c.runs.len()).collect();
    order.sort_by_cached_key(|&r| span(r));
    order
}

