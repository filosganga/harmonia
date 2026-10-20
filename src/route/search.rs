//! The router's search: the board as the router sees it (`Grid`: who owns each hole, where wires
//! lie), and the A* search that finds the cheapest way from what a run has joined so far to its
//! next pin, in steps of trace and wires (`spread`), with what each step costs.

use super::{Routing, distance};
use crate::board::{Design, Geometry, Point};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

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
pub(super) const fn bend(a: Point, b: Point) -> u32 {
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
pub(super) const RULE: f64 = crate::board::WEIGHTS[10] * 10.0;
/// The longest jumper or solder-side wire the router tries, in holes.
const MAX_JUMP: i32 = 10;

/// True when two straight segments properly cross (touching at an end does not count).
fn segments_cross((p1, p2): (Point, Point), (p3, p4): (Point, Point)) -> bool {
    let cross = |o: Point, a: Point, b: Point| (a.0 - o.0) as i64 * (b.1 - o.1) as i64 - (a.1 - o.1) as i64 * (b.0 - o.0) as i64;
    let (d1, d2) = (cross(p3, p4, p1), cross(p3, p4, p2));
    let (d3, d4) = (cross(p1, p2, p3), cross(p1, p2, p4));
    d1 * d2 < 0 && d3 * d4 < 0
}

/// Every jumper the router may try, as (offset from one leg to the other, the cost of its length),
/// shortest first: straight or at 45°, at least two holes and at most MAX_JUMP holes long.
pub(super) fn jumper_shapes() -> Vec<(Point, u32)> {
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
pub(super) struct Grid {
    pub(super) w: i32,
    pub(super) h: i32,
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
    pub(super) dir_order: [usize; 8],
}

/// Which side of the board a wire is on: the parts' side (a jumper) or the solder side.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Side {
    Top = 0,
    Bottom = 1,
}

/// One step of a path: a trace between neighbouring holes, or a wire on one side.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Step {
    Trace,
    Wire(Side),
}

/// The holes strictly between the two legs of a straight or 45° wire.
pub(super) fn holes_between(a: Point, b: Point) -> impl Iterator<Item = Point> {
    let (dx, dy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    (1..n).map(move |k| (a.0 + dx * k, a.1 + dy * k))
}

impl Grid {
    pub(super) fn on_board(&self, (x, y): Point) -> bool {
        x >= 0 && x < self.w && y >= 0 && y < self.h
    }

    pub(super) fn at(&self, (x, y): Point) -> usize {
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
    pub(super) fn has_way_out(&self, p: Point, run: usize, net: usize) -> bool {
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

/// The board as the router sees it before any copper: parts, pins, keep-outs.
pub(super) fn empty_grid(c: &Design, g: &Geometry) -> Grid {
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

/// Lays one step of `run`'s copper from `a` to `b`: in `out`, and on `grid`, where it now takes
/// its holes (except a star point, which stays open to the net's other runs).
pub(super) fn lay(grid: &mut Grid, out: &mut Routing, run: usize, a: Point, b: Point, step: Step) {
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

/// The cost in tenths of a hole if nothing were in the way: diagonals where they're cheaper than
/// a step across and one down. The search's guess must never be more than the real cost, or it can
/// miss the cheapest way.
pub(super) fn octile(a: Point, b: Point) -> u32 {
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
pub(super) struct Scratch {
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
    pub(super) fn new(holes: usize) -> Scratch {
        let states = holes * HEADINGS;
        Scratch { cost: vec![0; states], prev: vec![0; states], seen: vec![0; states], popped: vec![0; holes], first: vec![0; holes], via: vec![Step::Trace; states], round: 0, queue: BinaryHeap::new() }
    }

    /// Cost to reach hole `i` in the last search, coming in from any direction, if it got there.
    pub(super) fn reached(&self, i: usize) -> Option<u32> {
        (self.popped[i] == self.round).then(|| self.cost[self.first[i]])
    }

    /// The steps from where the last search started to hole `i`, the cheapest way.
    pub(super) fn path(&self, grid: &Grid, i: usize) -> Path {
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

/// Spreads traces out from `from` towards `target`, cheapest hole first, trying holes in the
/// target's direction first ("A*", which finds it sooner); true once it gets there, false if it
/// runs out of holes it can reach. `mem` then holds the cheapest way there (`Scratch::path`). A
/// trace pays TURN per right angle of bending (`bend`), except at a pin. With `shapes`, it also
/// goes over wires: from a hole where a wire's leg fits, a jumper or a
/// solder-side wire of any of those shapes, at what the score charges for it (`JUMPER` or
/// `SOLDER_WIRE`, and its length), landing where a trace carries on. A wire doesn't start from a
/// hole a wire landed in, which would put two legs in one hole.
pub(super) fn spread(grid: &Grid, mem: &mut Scratch, from: &[Point], target: Point, run: usize, net: usize, shapes: Option<&[(Point, u32)]>) -> bool {
    let guess = |p: Point| octile(p, target);
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
            for &((dx, dy), length) in shapes.expect("wires are only queued when there are shapes") {
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
            if p == target {
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
pub(super) type Path = Vec<(Point, Point, Step)>;

/// For a run of a soft star net: the cheapest way in traces from any hole of `others` (the copper
/// of the net's other runs) to `target`, and its cost, without the join's own price. None if
/// traces don't get through, or there's no other copper yet.
pub(super) fn join(grid: &Grid, mem: &mut Scratch, others: &[Point], target: Point, run: usize, net: usize) -> Option<(u32, Path)> {
    if others.is_empty() || !spread(grid, mem, others, target, run, net, None) {
        return None;
    }
    let i = grid.at(target);
    Some((mem.reached(i).unwrap(), mem.path(grid, i)))
}
