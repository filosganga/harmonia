//! Your own wiring, drawn in a DIYLC sketch, as a `Routing`, so the score measures it the way it
//! measures the router's.

use super::Routing;
use super::search::{Side, holes_between};
use crate::board::{Design, Geometry, Point};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Your own wiring, drawn in a DIYLC sketch, as a routing, so it's measured the way the router's
/// is. Each trace is cut into steps between neighbouring holes (one drawn at another angle than
/// across, down or 45° stays one step), and a step drawn twice counts once. Each piece goes to a
/// run of its net: on a star net, to the run whose pins it reaches without passing the star point.
/// Copper that reaches the pins of two runs that way joins them, as a soft star's join does; it's
/// counted as a join for a soft star, and for a hard one returned as a count of breaks, which the
/// router never makes. A pin your copper doesn't reach is `unrouted`.
pub fn from_sketch(c: &Design, g: &Geometry, copper: &crate::diylc::Copper) -> (Routing, usize) {
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
