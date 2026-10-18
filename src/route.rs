//! The router: joins every run with copper traces on the solder side, and hops over with a
//! jumper where a trace can't get through.
//!
//! A trace goes from a hole to a neighbouring one: across, down, or diagonally (45°). It touches
//! every hole it passes, so it can't pass a hole that belongs to another run (another run's pin or
//! trace, or a keep-out), and two diagonals can't cross between four holes. It may run under parts:
//! it's on the other side of the board. (A run is a net, or one branch of a star net.)
//!
//! A jumper is a wire on the parts' side. It lies straight between two free holes, across, down or
//! at 45°, up to 10 holes long, but never over a part, a pin or a keep-out, and never over or across
//! another run's jumper. It may pass under a module's board, like low parts. A trace carries on from
//! where it lands.
//!
//! Where no jumper fits, the last resort is the same kind of wire on the solder side. Nothing sits
//! there but copper, so it may pass under parts and over traces and pins (it's insulated); it still
//! keeps off keep-outs and other runs' solder-side wires. It's only used when no jumper fits, and
//! it costs a lot in the score, so the placer avoids it.
//!
//! Runs are routed one at a time, shortest first, and each one is grown like a tree: from the
//! pins joined so far, find the cheapest way to the nearest pin not joined yet ("A*" search: try
//! the most promising holes first, guided by the straight-line distance still to go). A trace
//! pays for its length and for each turn, since every bend is one more to make in copper wire.

use crate::board::{Circuit, Geometry, Point};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Where the copper goes. Each entry is (run, from hole, to hole).
#[derive(Default, Clone)]
pub struct Routing {
    /// One step of trace between neighbouring holes, on the solder side.
    pub traces: Vec<(usize, Point, Point)>,
    /// A jumper on the parts' side.
    pub jumpers: Vec<(usize, Point, Point)>,
    /// A wire on the solder side, the last resort.
    pub bottom: Vec<(usize, Point, Point)>,
}

impl Routing {
    /// Length of copper trace, in holes.
    pub fn trace_length(&self) -> f64 {
        self.traces.iter().map(|&(_, a, b)| distance(a, b)).sum::<f64>() + 0.0 // + 0.0: no traces is 0, not -0
    }

    /// Length of jumper and solder-side wire, in holes.
    pub fn jumper_length(&self) -> f64 {
        self.jumpers.iter().chain(&self.bottom).map(|&(_, a, b)| distance(a, b)).sum::<f64>() + 0.0
    }

    /// Length of copper (traces, jumpers and solder-side wires), in holes, between two holes the
    /// same net joins, going the way the router laid it. A star net's runs only meet at the star
    /// point, so between two runs it goes through there.
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
        // yet, and see how far its neighbours are through it.
        let mut best: HashMap<Point, f64> = HashMap::from([(from, 0.0)]);
        let mut done: HashSet<Point> = HashSet::new();
        loop {
            let closest = best.iter().filter(|(p, _)| !done.contains(*p)).min_by(|a, b| a.1.total_cmp(b.1));
            let Some((&p, &d)) = closest else {
                panic!("the router left {from:?} and {to:?} unjoined, but it joins every pin of a net");
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

/// Costs, in tenths of a hole of trace. A straight step is one hole, a diagonal about 1.4.
/// They match the score's weights (board::WEIGHTS) times ten, so the router and the score agree.
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
/// What a trace pays for each right angle's worth of bending (see `bend`), except at a pin, where
/// it starts anyway.
const TURN: u32 = 10;
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
/// Each hole of a jumper or solder-side wire costs twice a hole of trace. What each wire costs on
/// top of its length (the "jumpers" and "bottom" weights) doesn't change the router's choice: it
/// takes a jumper whenever one fits and a solder-side wire only when none does, and compares wires
/// of the same side by length.
const JUMPER_PER_HOLE: f64 = 20.0;
/// The longest jumper or solder-side wire the router tries, in holes.
const MAX_JUMP: i32 = 10;

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
    /// Keep-out holes: no part, trace or wire.
    keepout: Vec<bool>,
    /// For each square of four holes (by its top-left hole): the run whose diagonal crosses it,
    /// and which diagonal (true = top-left to bottom-right).
    diag: Vec<Option<(usize, bool)>>,
    /// For each side, top and bottom: the wires placed so far, (run, one leg, the other), and
    /// which run's wire has a leg in or lies over each hole.
    wires: [Vec<(usize, Point, Point)>; 2],
    wire_at: [Vec<Option<usize>>; 2],
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

/// Routes every run of the circuit, for parts at `g`.
pub fn route(c: &Circuit, g: &Geometry) -> Routing {
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
        diag: vec![None; n],
        wires: [Vec::new(), Vec::new()],
        wire_at: [vec![None; n], vec![None; n]],
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
    // A keep-out (a screw hole, say) takes no copper either.
    for &p in &c.constraints.keepout_holes {
        let i = grid.at(p);
        (grid.keepout[i], grid.owner[i]) = (true, Owner::Blocked);
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

    let holes_of = |r: usize| -> Vec<Point> { c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let mut out = Routing::default();
    let mut mem = [Scratch::new(n), Scratch::new(n)];
    let shapes = jumper_shapes();
    for r in order(c, g) {
        let run = &c.runs[r];
        let start = run.star.unwrap_or(run.pins[0]);
        let start = g.pins[start.0][start.1];
        let (mut remaining, off): (Vec<Point>, Vec<Point>) =
            holes_of(r).into_iter().filter(|&p| p != start).partition(|&p| grid.on_board(p) && grid.on_board(start));
        // A pin off the board can't be routed to. It still costs a solder-side wire straight to it,
        // or a part hanging off the board would save the wiring it should need.
        out.bottom.extend(off.into_iter().map(|p| (r, start, p)));
        let mut tree = vec![start];
        while !remaining.is_empty() {
            // The pin nearest to what's joined so far.
            let (k, _) = remaining.iter().enumerate().min_by_key(|&(_, &p)| tree.iter().map(|&t| octile(t, p)).min().unwrap()).unwrap();
            let target = remaining.swap_remove(k);
            let Some(path) = connect(&grid, &mut mem, &shapes, &tree, target, r, run.net) else {
                // Walled in even for a wire on either side: a solder-side wire from the nearest hole
                // already joined straight to the pin, at any angle and length, so the run still joins
                // up. It breaks the wire rules (it may end on a pin, cross other wires or a keep-out),
                // but later wires keep off it.
                let from = *tree.iter().min_by_key(|&&t| octile(t, target)).unwrap();
                out.bottom.push((r, from, target));
                grid.wires[Side::Bottom as usize].push((r, from, target));
                // Straight or at 45° it lies over holes, which later wires must keep off too; at
                // another angle it passes between them, and the crossing test covers it.
                let (dx, dy) = ((target.0 - from.0).abs(), (target.1 - from.1).abs());
                let over = (dx == 0 || dy == 0 || dx == dy).then(|| holes_between(from, target)).into_iter().flatten();
                for p in over.chain([from, target]) {
                    let i = grid.at(p);
                    grid.wire_at[Side::Bottom as usize][i] = Some(r);
                }
                tree.push(target);
                continue;
            };
            for (a, b, step) in path {
                if let Step::Wire(side) = step {
                    let list = if side == Side::Top { &mut out.jumpers } else { &mut out.bottom };
                    list.push((r, a, b));
                    grid.wires[side as usize].push((r, a, b));
                    for p in holes_between(a, b).chain([a, b]) {
                        let i = grid.at(p);
                        grid.wire_at[side as usize][i] = Some(r);
                    }
                } else {
                    out.traces.push((r, a, b));
                    if a.0 != b.0 && a.1 != b.1 {
                        let (sq, kind) = grid.square(a, b);
                        grid.diag[sq] = Some((r, kind));
                    }
                }
                for p in [a, b] {
                    let i = grid.at(p);
                    if grid.star[i].is_none() {
                        grid.owner[i] = Owner::Run(r);
                    }
                }
                tree.push(b);
                remaining.retain(|&p| p != b);
            }
        }
    }
    out
}

/// The order `route` routes the runs in: shortest first, since they have the fewest ways around
/// things. A run's copper is laid all at once, so in a `Routing` each run's pieces come one after
/// the other, runs in this order (the layout file keeps them that way).
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

/// Distance in tenths of a hole if nothing were in the way (diagonals where they help).
fn octile(a: Point, b: Point) -> u32 {
    let (dx, dy) = ((a.0 - b.0).unsigned_abs(), (a.1 - b.1).unsigned_abs());
    STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy)
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
    round: u32,
    queue: BinaryHeap<Reverse<(u32, u32, usize)>>,
}

impl Scratch {
    fn new(holes: usize) -> Scratch {
        let states = holes * HEADINGS;
        Scratch { cost: vec![0; states], prev: vec![0; states], seen: vec![0; states], popped: vec![0; holes], first: vec![0; holes], round: 0, queue: BinaryHeap::new() }
    }

    /// Cost to reach hole `i` in the last search, coming in from any direction, if it was reached.
    /// (The search handles every hole it reaches, unless it stopped early: then only those.)
    fn reached(&self, i: usize) -> Option<u32> {
        (self.popped[i] == self.round).then(|| self.cost[self.first[i]])
    }

    /// The holes from where the last search started to hole `i`, the cheapest way.
    fn path_to(&self, grid: &Grid, i: usize) -> Vec<Point> {
        assert!(self.popped[i] == self.round, "path_to a hole the search didn't reach");
        let mut s = self.first[i];
        let mut path = vec![grid.hole(s / HEADINGS)];
        while self.prev[s] != usize::MAX {
            s = self.prev[s];
            path.push(grid.hole(s / HEADINGS));
        }
        path.reverse();
        path
    }
}

/// Spreads traces out from `from`, cheapest hole first, calling `done(hole, cost)` on each hole
/// the first (cheapest) time it gets there; stops when `done` says so (then returns true) or when
/// it runs out of holes it can reach. `mem` then holds the cheapest cost to every hole reached,
/// and the way back. A trace pays TURN per right angle of bending (`bend`), except at a pin.
/// With `towards`, it tries holes in that direction first ("A*"), which finds it sooner.
fn spread(grid: &Grid, mem: &mut Scratch, from: &[Point], towards: Option<Point>, run: usize, net: usize, mut done: impl FnMut(Point, u32) -> bool) -> bool {
    let guess = |p: Point| towards.map_or(0, |t| octile(p, t));
    mem.round += 1;
    mem.queue.clear();
    let Scratch { cost, prev, seen, popped, first, round, queue } = mem;
    let round = *round;
    for &p in from {
        let s = grid.at(p) * HEADINGS + START;
        (cost[s], prev[s], seen[s]) = (0, usize::MAX, round);
        queue.push(Reverse((guess(p), 0, s)));
    }
    while let Some(Reverse((_, so_far, s))) = queue.pop() {
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
        } else if grid.pin[i] || so_far >= cost[first[i]] + AHEAD[first[i] % HEADINGS][heading] * TURN {
            // Coming in the cheapest way and turning would do as well: nothing new from here.
            continue;
        }
        for (k, (dx, dy)) in DIRS.into_iter().enumerate() {
            let q = (p.0 + dx, p.1 + dy);
            let diagonal = dx != 0 && dy != 0;
            if grid.on_board(q) && grid.open(q, run, net) && (!diagonal || grid.diagonal_free(p, q, run)) {
                let turn = if heading == START || grid.pin[i] { 0 } else { bend(DIRS[heading], DIRS[k]) * TURN };
                let c = so_far + if diagonal { DIAGONAL } else { STRAIGHT } + turn;
                let j = grid.at(q);
                let t = j * HEADINGS + k;
                let beaten = popped[j] == round && (grid.pin[j] || c >= cost[first[j]] + AHEAD[first[j] % HEADINGS][k] * TURN);
                if !beaten && (seen[t] != round || c < cost[t]) {
                    (cost[t], prev[t], seen[t]) = (c, s, round);
                    queue.push(Reverse((c + guess(q), c, t)));
                }
            }
        }
    }
    false
}

/// The cheapest way from any hole of `tree` to `target`, as a list of (from, to, what it is).
/// Traces only if they get through; otherwise traces up to one wire and on from where it lands:
/// a jumper on top whenever one fits, however long the traces to it, and a solder-side wire only
/// when none does. None if no wire fits on either side.
fn connect(grid: &Grid, mem: &mut [Scratch; 2], shapes: &[(Point, u32)], tree: &[Point], target: Point, run: usize, net: usize) -> Option<Vec<(Point, Point, Step)>> {
    let steps = |holes: Vec<Point>| holes.windows(2).map(|w| (w[0], w[1], Step::Trace)).collect::<Vec<_>>();
    let [near, far] = mem;
    if spread(grid, near, tree, Some(target), run, net, |p, _| p == target) {
        return Some(steps(near.path_to(grid, grid.at(target))));
    }
    // `near` now knows every hole traces can reach from the tree. Spread from the target too, and
    // from each hole it reaches try a wire back to one of those, shortest first, keeping the best
    // for each side. Stop once even the shortest jumper from here on would cost more than the best
    // jumper found; without one, keep going, since a jumper anywhere beats any solder-side wire.
    let mut best: [Option<(u32, usize, usize)>; 2] = [None, None]; // per side: (cost, hole the tree reaches, hole the target reaches)
    let beaten = |best: Option<(u32, usize, usize)>, cost: u32| best.is_some_and(|(t, ..)| cost >= t);
    spread(grid, far, &[target], None, run, net, |pb, from_b| {
        if beaten(best[0], from_b + shapes[0].1) {
            return true;
        }
        for side in [Side::Top, Side::Bottom] {
            if (side == Side::Bottom && best[0].is_some()) || !grid.leg(pb, run, side) {
                continue;
            }
            for &((dx, dy), length) in shapes {
                if beaten(best[side as usize], from_b + length) {
                    break; // the rest are longer still
                }
                let pa = (pb.0 + dx, pb.1 + dy);
                if !grid.on_board(pa) {
                    continue;
                }
                let a = grid.at(pa);
                if let Some(to_a) = near.reached(a).filter(|_| grid.leg(pa, run, side)) {
                    let total = to_a + length + from_b;
                    if !beaten(best[side as usize], total) && grid.clear(pa, pb, run, side) {
                        best[side as usize] = Some((total, a, grid.at(pb)));
                    }
                }
            }
        }
        false
    });
    let (side, (_, a, b)) = match best {
        [Some(top), _] => (Side::Top, top),
        [None, Some(bottom)] => (Side::Bottom, bottom),
        [None, None] => return None,
    };
    let mut back = far.path_to(grid, b); // target ... b
    back.reverse();
    let mut path = steps(near.path_to(grid, a));
    path.push((grid.hole(a), grid.hole(b), Step::Wire(side)));
    path.extend(steps(back));
    Some(path)
}
