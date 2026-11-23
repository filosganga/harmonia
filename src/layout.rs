//! A finished board, a `Layout`: where each part sits, and the copper joining them. It holds
//! nothing about how it was made or what it was asked to build: no runs, no nets. Which pins its
//! copper joins is worked out from the copper itself (`Copper::joined`).
//!
//! Copper is traces (bare copper along a face, through every hole it touches) and jumper wires (a
//! wire lifted off the board between two holes, with a pin in each). For now every trace runs on the
//! bottom face, at layer -1, and a wire lies on the top face (2) or, the last resort, the bottom
//! one (-2).

use crate::board::{COPPER, LYING, Layer, Placement, Point, distance};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A finished board: where each part sits, and the copper joining them.
#[derive(Clone)]
pub struct Layout {
    pub placement: Placement,
    pub copper: Copper,
}

/// What joins the parts' pins on a layout: traces and jumper wires.
#[derive(Clone, Default)]
pub struct Copper {
    pub traces: Vec<Trace>,
    /// Named W1, W2, ... in this order (`JumperWire::name`): those on the top face first.
    pub wires: Vec<JumperWire>,
}

/// Bare copper running along a face, through every hole it touches, in order: each step goes to a
/// neighbouring hole (across, down or at 45°), except that a trace drawn in a sketch at another
/// angle keeps that step as drawn.
#[derive(Clone)]
pub struct Trace {
    pub layer: Layer,
    pub holes: Vec<Point>,
}

/// A wire lifted off the board between two holes, with a pin in each: on the top face (layer 2) or
/// the bottom one (-2).
#[derive(Clone, Copy)]
pub struct JumperWire {
    pub layer: Layer,
    pub pins: [Point; 2],
}

impl JumperWire {
    /// Its name, from its place among the layout's wires: W1, W2, ...
    pub fn name(index: usize) -> String {
        format!("W{}", index + 1)
    }
}

/// How hard each turn is to bend in copper wire, counted in right angles, which are the easiest.
/// A gentle 45° turn, onto or off a diagonal, is harder to bend so it lands on the right hole, and
/// a sharp 135° turn harder still.
const RIGHT: u32 = 1;
const GENTLE: u32 = 2;
const SHARP: u32 = 3;

/// How hard it is to bend a trace arriving in direction `a` so it leaves in direction `b` (each a
/// step across, down or diagonal), in right angles; 0 if it goes straight on.
pub const fn bend(a: Point, b: Point) -> u32 {
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

/// The holes strictly between `a` and `b`, on the straight line from one to the other.
pub fn holes_between(a: Point, b: Point) -> impl Iterator<Item = Point> {
    let (dx, dy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    (1..n).map(move |k| (a.0 + dx * k, a.1 + dy * k))
}

/// A straight trace from `a` to `b` (across, down or at 45°) as steps between neighbouring holes,
/// the way the router lays it; None for one at another angle.
pub fn unit_steps(a: Point, b: Point) -> Option<Vec<(Point, Point)>> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dx == 0 || dy == 0 || dx.abs() == dy.abs()).then(|| holes_between(a, b).chain([b]).scan(a, |from, to| Some((std::mem::replace(from, to), to))).collect())
}

impl Copper {
    /// Every step of trace, between two holes next to each other, trace by trace.
    pub fn steps(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.traces.iter().flat_map(|t| t.holes.windows(2).map(|w| (w[0], w[1])))
    }

    /// The wires on the top face, then those on the bottom one.
    pub fn top(&self) -> impl Iterator<Item = &JumperWire> {
        self.wires.iter().filter(|w| w.layer > 0)
    }

    pub fn bottom(&self) -> impl Iterator<Item = &JumperWire> {
        self.wires.iter().filter(|w| w.layer < 0)
    }

    /// Length of copper trace, in holes.
    pub fn trace_length(&self) -> f64 {
        self.steps().map(|(a, b)| distance(a, b)).sum::<f64>() + 0.0 // + 0.0: no traces is 0, not -0
    }

    /// Length of jumper wire, in holes: those on the top face, then those on the bottom one.
    pub fn wire_length(&self) -> f64 {
        self.top().chain(self.bottom()).map(|w| distance(w.pins[0], w.pins[1])).sum::<f64>() + 0.0
    }

    /// How many steps of trace go diagonally. Traces are laid in copper wire along the holes, and
    /// straight ones are easier to lay and can line up in parallel (or come from a stripboard's
    /// strips); jumper wires are free to go at 45°, so they don't count.
    pub fn diagonal_steps(&self) -> f64 {
        self.steps().filter(|&(a, b)| a.0 != b.0 && a.1 != b.1).count() as f64
    }

    /// How many straight steps of trace run against the board's main direction: across or down,
    /// whichever has fewer. On a stripboard the strips run one way and every piece of copper the
    /// other way needs a wire; on perfboard too, traces side by side in one direction are the
    /// easiest to lay. The router doesn't price this, since the main direction is only known once
    /// the whole board is routed; it matters little, as a trace from one pin to another takes the
    /// same steps across and down whichever way it turns.
    pub fn crosswise_steps(&self) -> f64 {
        let across = self.steps().filter(|&(a, b)| a.1 == b.1).count();
        let down = self.steps().filter(|&(a, b)| a.0 == b.0).count();
        across.min(down) as f64
    }

    /// How hard the traces' corners are to bend, in right angles (see `bend`). A corner is a hole
    /// where exactly two steps of trace on one face meet, not in a line, with no pin in it (a part's,
    /// `part_pins`, or a wire's): a pin is where a trace starts, and three steps or more meeting is
    /// a junction, soldered like a pin. How the copper is split into traces doesn't matter.
    pub fn turns(&self, part_pins: &HashSet<Point>) -> u32 {
        let wire_pins: HashSet<Point> = self.wires.iter().flat_map(|w| w.pins).collect();
        // For each hole of each face: the directions its steps leave in.
        let mut out_of: HashMap<(Layer, Point), Vec<Point>> = HashMap::new();
        for t in &self.traces {
            for w in t.holes.windows(2) {
                let (a, b) = (w[0], w[1]);
                let d = (b.0 - a.0, b.1 - a.1);
                out_of.entry((t.layer, a)).or_default().push(d);
                out_of.entry((t.layer, b)).or_default().push((-d.0, -d.1));
            }
        }
        out_of
            .iter()
            .filter(|((_, hole), _)| !part_pins.contains(hole) && !wire_pins.contains(hole))
            .map(|(_, dirs)| match dirs[..] {
                [a, b] => bend((-a.0, -a.1), b),
                _ => 0,
            })
            .sum()
    }

    /// Every hole with copper in it or a wire over it.
    pub fn holes(&self) -> HashSet<Point> {
        let traces = self.traces.iter().flat_map(|t| t.holes.iter().copied());
        let wires = self.wires.iter().flat_map(|w| holes_between(w.pins[0], w.pins[1]).chain(w.pins));
        traces.chain(wires).collect()
    }

    /// Length of copper (traces and wires), in holes, between two holes, going the way it's laid;
    /// None if no copper joins them.
    pub fn length_between(&self, from: Point, to: Point) -> Option<f64> {
        // Every piece of copper, as links between holes, both ways.
        let mut links: HashMap<Point, Vec<(Point, f64)>> = HashMap::new();
        for (a, b) in self.steps().chain(self.wires.iter().map(|w| (w.pins[0], w.pins[1]))) {
            links.entry(a).or_default().push((b, distance(a, b)));
            links.entry(b).or_default().push((a, distance(a, b)));
        }
        // The shortest way along those links (Dijkstra): keep taking the closest hole not done
        // yet, and see how far its neighbours are through it. A BTreeMap, not a HashMap: its order
        // is the same every run, so two holes equally close are taken in the same order, and the
        // lengths are added up the same way (adding fractions in another order can change the last
        // digit, and a placer comparing two equal costs could then go another way).
        let mut best: BTreeMap<Point, f64> = BTreeMap::from([(from, 0.0)]);
        let mut done: HashSet<Point> = HashSet::new();
        loop {
            let (&p, &d) = best.iter().filter(|(p, _)| !done.contains(*p)).min_by(|a, b| a.1.total_cmp(b.1))?;
            if p == to {
                return Some(d);
            }
            done.insert(p);
            for &(q, len) in links.get(&p).into_iter().flatten() {
                if best.get(&q).is_none_or(|&old| d + len < old) {
                    best.insert(q, d + len);
                }
            }
        }
    }

    /// The traces as straight pieces: steps in a row in the same direction, joined into one, each
    /// with the index of its trace.
    pub fn straight_pieces(&self) -> Vec<(usize, Point, Point)> {
        let dir = |p: Point, q: Point| ((q.0 - p.0).signum(), (q.1 - p.1).signum());
        let mut out: Vec<(usize, Point, Point)> = Vec::new();
        for (k, t) in self.traces.iter().enumerate() {
            for w in t.holes.windows(2) {
                let (a, b) = (w[0], w[1]);
                match out.last_mut() {
                    Some(last) if last.0 == k && last.2 == a && dir(last.1, last.2) == dir(a, b) => last.2 = b,
                    _ => out.push((k, a, b)),
                }
            }
        }
        out
    }

    /// The copper drawn in a DIYLC sketch (by net, as the reader works it out), as a layout's copper:
    /// net by net, each trace cut into steps between neighbouring holes (a step drawn twice in a net
    /// once; one at another angle as drawn), every trace on the bottom face; then the wires on the
    /// top face, then those on the bottom one. Pieces of no length are left out.
    pub fn from_sketch(sketch: &crate::board::Copper, nets: usize) -> Copper {
        let mut out = Copper::default();
        let mut seen: HashSet<(usize, Point, Point)> = HashSet::new();
        for net in 0..nets {
            for &(_, a, b) in sketch.traces.iter().filter(|t| t.0 == net) {
                for (p, q) in unit_steps(a, b).unwrap_or_else(|| vec![(a, b)]) {
                    if p != q && seen.insert((net, p.min(q), p.max(q))) {
                        out.traces.push(Trace { layer: -COPPER, holes: vec![p, q] });
                    }
                }
            }
        }
        for (list, layer) in [(&sketch.jumpers, LYING), (&sketch.bottom, -LYING)] {
            for net in 0..nets {
                let wires = list.iter().filter(|&&(n, a, b)| n == net && a != b);
                out.wires.extend(wires.map(|&(_, a, b)| JumperWire { layer, pins: [a, b] }));
            }
        }
        out
    }

    /// Which holes the copper joins, given the holes with a pin in them (`pins`: a part's pins; a
    /// wire's are its own): a trace joins every hole it touches, a pin joins whatever is in its
    /// hole on either face, and a wire joins its two pins (not the holes it passes over). Copper
    /// touching a hole in `apart` isn't joined there: it splits a star net into its branches.
    pub fn joined(&self, pins: &[Point], apart: &[Point]) -> Joined {
        let links: Vec<(Point, Point)> = self.steps().chain(self.wires.iter().map(|w| (w.pins[0], w.pins[1]))).collect();
        let every = pins.iter().copied().chain(links.iter().flat_map(|&(a, b)| [a, b]));
        let (lo, hi) = every.fold(((i32::MAX, i32::MAX), (i32::MIN, i32::MIN)), |(lo, hi), p| ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1))));
        let w = (hi.0 - lo.0 + 1).max(0) as usize;
        let n = w * (hi.1 - lo.1 + 1).max(0) as usize;
        let mut j = Joined { corner: lo, w, parent: (0..n).collect() };
        // Every trace is on one face and every pin goes through both, so a hole is one node.
        for (a, b) in links {
            if !apart.contains(&a) && !apart.contains(&b) {
                j.join(a, b);
            }
        }
        j
    }
}

/// Holes joined by copper, in groups (a union-find: each hole points towards its group's root),
/// over the smallest rectangle around the pins and copper it was made from.
pub struct Joined {
    /// The rectangle's top-left hole, and its width: hole (x, y) is at (y - top) * w + (x - left).
    corner: Point,
    w: usize,
    parent: Vec<usize>,
}

impl Joined {
    fn index(&self, (x, y): Point) -> usize {
        (y - self.corner.1) as usize * self.w + (x - self.corner.0) as usize
    }

    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]]; // halve the way up as we go
            i = self.parent[i];
        }
        i
    }

    /// The group a hole is in, named by one of its holes. (A hole outside the rectangle has no
    /// copper and no pin: it's a group of its own.)
    pub fn find(&mut self, p: Point) -> Point {
        let inside = p.0 >= self.corner.0 && p.1 >= self.corner.1 && ((p.0 - self.corner.0) as usize) < self.w && self.index(p) < self.parent.len();
        if !inside {
            return p;
        }
        let r = self.root(self.index(p));
        (self.corner.0 + (r % self.w) as i32, self.corner.1 + (r / self.w) as i32)
    }

    fn join(&mut self, a: Point, b: Point) {
        let (ra, rb) = (self.root(self.index(a)), self.root(self.index(b)));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }

    /// Every hole in the same group as `p` (in the rectangle, row by row).
    pub fn group_of(&mut self, p: Point) -> Vec<Point> {
        let root = self.find(p);
        let (w, h) = (self.w as i32, (self.parent.len() / self.w.max(1)) as i32);
        let holes: Vec<Point> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| (self.corner.0 + x, self.corner.1 + y)).collect();
        holes.into_iter().filter(|&h| self.find(h) == root).collect()
    }
}
