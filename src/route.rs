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

mod search;
mod sketch;

pub use sketch::from_sketch;

use crate::board::{Circuit, Geometry, Measures, Place, Point, WEIGHTS};
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use search::{Grid, RULE, Scratch, Step, bend, empty_grid, holes_between, join, jumper_shapes, lay, octile, spread};
use std::collections::{BTreeMap, HashMap, HashSet};

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
            let own = spread(&grid, &mut mem, &tree, target, r, run.net, Some(&shapes)).then(|| {
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
