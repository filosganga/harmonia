//! The router: joins every run with copper traces on the solder side, and hops over with a
//! jumper where a trace can't get through.
//!
//! A trace goes from a hole to a neighbouring one: across, down, or diagonally (45°). It touches
//! every hole it passes, so it can't pass a hole that belongs to another run (another net's pin or
//! trace), and two diagonals can't cross between four holes. It may run under parts: it's on the
//! other side of the board.
//!
//! A jumper is a wire on the parts' side, so it can pass over anything. Its legs go in free holes
//! (not a pin's hole, not under a part), and a trace carries on from where it lands.
//!
//! Runs are routed one at a time, shortest first, and each one is grown like a tree: from the
//! pins joined so far, find the cheapest way to the nearest pin not joined yet ("A*" search: try
//! the most promising holes first, guided by the straight-line distance still to go).

use crate::board::{Circuit, Geometry, Point};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Where the copper goes. Each entry is (run, from hole, to hole).
#[derive(Default)]
pub struct Routing {
    /// One step of trace between neighbouring holes, on the solder side.
    pub traces: Vec<(usize, Point, Point)>,
    /// A jumper on the parts' side.
    pub jumpers: Vec<(usize, Point, Point)>,
}

impl Routing {
    /// Total length of trace and jumper wire, in holes.
    pub fn length(&self) -> f64 {
        let len = |&(_, a, b): &(usize, Point, Point)| (((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) as f64).sqrt();
        self.traces.iter().chain(&self.jumpers).map(len).sum()
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

/// Costs, in tenths of a hole of wire. A straight step is one hole, a diagonal about 1.4.
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
/// What a jumper costs on top of its length: it's one more part to cut, bend and solder.
const JUMPER: u32 = 60;
/// The longest jumper the router tries, in holes.
const MAX_JUMP: i32 = 10;

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
    /// Holes with a pin in them, where no jumper leg can go.
    pin: Vec<bool>,
    /// Holes under a part's body on the parts' side, where no jumper leg can go either.
    covered: Vec<bool>,
    /// For each square of four holes (by its top-left hole): the run whose diagonal crosses it,
    /// and which diagonal (true = top-left to bottom-right).
    diag: Vec<Option<(usize, bool)>>,
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

    /// Can a jumper leg go in this hole, for `run`?
    fn leg(&self, p: Point, run: usize) -> bool {
        let i = self.at(p);
        !self.pin[i] && !self.covered[i] && (self.owner[i] == Owner::Free || self.owner[i] == Owner::Run(run))
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
    let mut grid = Grid { w, h, owner: vec![Owner::Free; n], star: vec![None; n], pin: vec![false; n], covered: vec![false; n], diag: vec![None; n] };

    // Holes under a part's body. Modules don't count: only their pins take holes.
    for (body, outline) in g.bodies.iter().zip(&g.outlines) {
        if body.len() == outline.len() {
            for &p in body {
                if let Some(i) = grid.index(p) {
                    grid.covered[i] = true;
                }
            }
        }
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

    // Shortest runs first: they have the fewest ways around things.
    let holes_of = |r: usize| -> Vec<Point> { c.runs[r].pins.iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let span = |r: usize| {
        let ps = holes_of(r);
        let (xs, ys) = (ps.iter().map(|p| p.0), ps.iter().map(|p| p.1));
        xs.clone().max().unwrap() - xs.min().unwrap() + ys.clone().max().unwrap() - ys.min().unwrap()
    };
    let mut order: Vec<usize> = (0..c.runs.len()).collect();
    order.sort_by_cached_key(|&r| span(r));

    let mut out = Routing::default();
    let mut mem = [Scratch::new(n), Scratch::new(n)];
    for r in order {
        let run = &c.runs[r];
        let start = run.star.unwrap_or(run.pins[0]);
        let start = g.pins[start.0][start.1];
        let (mut remaining, off): (Vec<Point>, Vec<Point>) =
            holes_of(r).into_iter().filter(|&p| p != start).partition(|&p| grid.on_board(p) && grid.on_board(start));
        // A pin off the board can't be routed to. It still costs a jumper straight to it, or a
        // part hanging off the board would save the wiring it should need.
        out.jumpers.extend(off.into_iter().map(|p| (r, start, p)));
        let mut tree = vec![start];
        while !remaining.is_empty() {
            // The pin nearest to what's joined so far.
            let (k, _) = remaining.iter().enumerate().min_by_key(|&(_, &p)| tree.iter().map(|&t| octile(t, p)).min().unwrap()).unwrap();
            let target = remaining.swap_remove(k);
            let path = connect(&grid, &mut mem, &tree, target, r, run.net)
                .unwrap_or_else(|| {
                    // Walled in even for jumpers: a jumper straight to the pin, so the run still joins up.
                    let from = *tree.iter().min_by_key(|&&t| octile(t, target)).unwrap();
                    vec![(from, target, true)]
                });
            for (a, b, jump) in path {
                if jump {
                    out.jumpers.push((r, a, b));
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

/// Distance in tenths of a hole if nothing were in the way (diagonals where they help).
fn octile(a: Point, b: Point) -> u32 {
    let (dx, dy) = ((a.0 - b.0).unsigned_abs(), (a.1 - b.1).unsigned_abs());
    STRAIGHT * dx.max(dy) + (DIAGONAL - STRAIGHT) * dx.min(dy)
}

/// Memory for `spread`, kept between searches so each one doesn't start by allocating and
/// clearing its own. `seen[i] == round` says `cost[i]` and `prev[i]` belong to this search.
struct Scratch {
    cost: Vec<u32>,
    prev: Vec<usize>,
    seen: Vec<u32>,
    round: u32,
    queue: BinaryHeap<Reverse<(u32, u32, usize)>>,
}

impl Scratch {
    fn new(holes: usize) -> Scratch {
        Scratch { cost: vec![0; holes], prev: vec![0; holes], seen: vec![0; holes], round: 0, queue: BinaryHeap::new() }
    }

    /// Cost to reach hole `i` in the last search, if it was reached.
    fn reached(&self, i: usize) -> Option<u32> {
        (self.seen[i] == self.round).then_some(self.cost[i])
    }

    /// The holes from where the last search started to hole `i`.
    fn path_to(&self, grid: &Grid, mut i: usize) -> Vec<Point> {
        let mut path = vec![grid.hole(i)];
        while self.prev[i] != usize::MAX {
            i = self.prev[i];
            path.push(grid.hole(i));
        }
        path.reverse();
        path
    }
}

/// Spreads traces out from `from`, cheapest hole first, calling `done(hole, cost)` on each hole
/// as it gets there; stops when `done` says so (then returns true) or when it runs out of holes
/// it can reach. `mem` then holds the cheapest cost to every hole reached, and the way back.
/// With `towards`, it tries holes in that direction first ("A*"), which finds it sooner.
fn spread(grid: &Grid, mem: &mut Scratch, from: &[Point], towards: Option<Point>, run: usize, net: usize, mut done: impl FnMut(Point, u32) -> bool) -> bool {
    let guess = |p: Point| towards.map_or(0, |t| octile(p, t));
    mem.round += 1;
    mem.queue.clear();
    let Scratch { cost, prev, seen, round, queue } = mem;
    let round = *round;
    for &p in from {
        let i = grid.at(p);
        (cost[i], prev[i], seen[i]) = (0, usize::MAX, round);
        queue.push(Reverse((guess(p), 0, i)));
    }
    while let Some(Reverse((_, so_far, i))) = queue.pop() {
        if so_far > cost[i] {
            continue; // already reached this hole more cheaply
        }
        let p = grid.hole(i);
        if done(p, so_far) {
            return true;
        }
        for (dx, dy) in DIRS {
            let q = (p.0 + dx, p.1 + dy);
            let diagonal = dx != 0 && dy != 0;
            if grid.on_board(q) && grid.open(q, run, net) && (!diagonal || grid.diagonal_free(p, q, run)) {
                let (j, c) = (grid.at(q), so_far + if diagonal { DIAGONAL } else { STRAIGHT });
                if seen[j] != round || c < cost[j] {
                    (cost[j], prev[j], seen[j]) = (c, i, round);
                    queue.push(Reverse((c + guess(q), c, j)));
                }
            }
        }
    }
    false
}

/// The cheapest way from any hole of `tree` to `target`, as a list of (from, to, is it a jumper).
/// Traces only if they get through; otherwise traces up to one jumper and on from where it lands.
/// None if even that can't be done.
fn connect(grid: &Grid, mem: &mut [Scratch; 2], tree: &[Point], target: Point, run: usize, net: usize) -> Option<Vec<(Point, Point, bool)>> {
    let steps = |holes: Vec<Point>| holes.windows(2).map(|w| (w[0], w[1], false)).collect::<Vec<_>>();
    let [near, far] = mem;
    if spread(grid, near, tree, Some(target), run, net, |p, _| p == target) {
        return Some(steps(near.path_to(grid, grid.at(target))));
    }
    // `near` now knows every hole traces can reach from the tree. Spread from the target too, and
    // from each hole it reaches try a jumper back to one of those. Stop once even the shortest
    // jumper from here on would cost more than the best one found.
    let mut best: Option<(u32, usize, usize)> = None; // (cost, hole the tree reaches, hole the target reaches)
    spread(grid, far, &[target], None, run, net, |pb, from_b| {
        if best.is_some_and(|(t, _, _)| from_b + JUMPER + 2 * STRAIGHT >= t) {
            return true;
        }
        if !grid.leg(pb, run) {
            return false;
        }
        for (dx, dy) in DIRS {
            for len in 2..=MAX_JUMP {
                let pa = (pb.0 + dx * len, pb.1 + dy * len);
                if !grid.on_board(pa) {
                    break;
                }
                let a = grid.at(pa);
                if let Some(to_a) = near.reached(a).filter(|_| grid.leg(pa, run)) {
                    let total = to_a + JUMPER + octile(pa, pb) + from_b;
                    if best.is_none_or(|(t, _, _)| total < t) {
                        best = Some((total, a, grid.at(pb)));
                    }
                }
            }
        }
        false
    });
    let (_, a, b) = best?;
    let mut back = far.path_to(grid, b); // target ... b
    back.reverse();
    let mut path = steps(near.path_to(grid, a));
    path.push((grid.hole(a), grid.hole(b), true));
    path.extend(steps(back));
    Some(path)
}
