//! The auto-placer: "simulated annealing".
//!
//! It keeps nudging, rotating, swapping and jumping parts, keeping changes the judge likes and,
//! early on, sometimes accepting worse ones so it doesn't get stuck.
//!
//! It only ever holds layouts that are allowed (`board::clashes`: no two parts on one hole, nothing
//! off the board, on a keep-out, or under a module without being low). It starts from one, and a
//! move that would break that is thrown away before it's routed, so the judge only ever compares
//! layouts that can be built.

use crate::board::{self, Circuit, Measures, Place, Placement, measure, random_placement, size};
use crate::constraints;
use rand::Rng;

/// How often a move jumps the part to any place at all, instead of nudging, turning or swapping it.
/// A nudge can't get a part past its neighbours (every place in between collides, so it isn't
/// allowed); a jump can. For a part with placement rules it's also the only way across to another
/// option of a one-of that's far away (the other edge) or a half turn off. Set on the benchmark
/// circuits placed with `place` (total cost, seeds 1-3, and seeds 4-6 to check it on seeds it
/// wasn't picked on): 0.1 gave 7,137 and 9,250, 0.3 gave 6,419 and 6,978, 0.5 gave 6,734 and 7,155.
/// With jumps only for parts with placement rules (as before collisions were a check) it was 7,912
/// on seeds 1-3, about what the placer did before (7,930).
const JUMP: f64 = 0.3;

/// How many moves to try, each step, before one is allowed. A move that isn't allowed costs almost
/// nothing (it's never routed), so trying again is cheap; past this many the step is skipped. On the
/// cat feeder and the LED chaser a step takes about two tries, and none ran out of them.
const TRIES: usize = 20;

/// `cost` is what a board costs, lower is better: the rules' cost, or what the network's score stands
/// for. `start` must be allowed (see `allowed_start`). `on_step` sees every accepted-or-not current
/// board (used by training to collect examples). Returns the best placement found.
pub fn anneal(
    c: &Circuit,
    cost: impl Fn(&Measures) -> f64,
    steps: usize,
    rng: &mut impl Rng,
    start: Placement,
    mut on_step: impl FnMut(&Placement),
) -> Placement {
    assert!(board::allowed(c, &start), "the placer starts from a layout that's allowed");
    // Parts the placer may touch: everything except the ones the board file allows only one place.
    let movable: Vec<usize> = (0..c.components.len()).filter(|&i| c.constraints.allowed[i].as_ref().is_none_or(|a| a.len() > 1)).collect();
    let mut cur = start;
    // Compare boards by cost, not by the 0-100 score: the score flattens out for big circuits (at
    // cost 400 a much worse move barely changes it), which would make the placer accept anything.
    let cost_of = |p: &Placement| cost(&measure(c, p));
    let mut cur_s = cost_of(&cur);
    let (mut best, mut best_s) = (cur.clone(), cur_s);
    if movable.is_empty() {
        return best;
    }

    for i in 0..steps {
        // How much worse (in cost) a move may be and still get accepted now and then; falls to almost 0.
        let temp = 20.0 * (1.0 - i as f64 / steps as f64) + 0.1;
        let Some(new) = (0..TRIES).map(|_| a_move(c, &cur, &movable, rng)).find(|p| board::allowed(c, p)) else {
            on_step(&cur);
            continue;
        };
        let new_s = cost_of(&new);
        if new_s <= cur_s || rng.random::<f64>() < ((cur_s - new_s) / temp).exp() {
            (cur, cur_s) = (new, new_s);
            if cur_s < best_s {
                (best, best_s) = (cur.clone(), cur_s);
            }
        }
        on_step(&cur);
    }
    best
}

/// `cur` with one random change to one of the `movable` parts. It may not be allowed.
fn a_move(c: &Circuit, cur: &Placement, movable: &[usize], rng: &mut impl Rng) -> Placement {
    let (w, h) = (c.board.width, c.board.height);
    let mut new = cur.clone();
    let k = movable[rng.random_range(0..movable.len())];
    let roll: f64 = rng.random();
    if rng.random::<f64>() < JUMP {
        // jump: a part with placement rules to any place they allow, any other part anywhere it
        // fits on the board, turned any way
        new[k] = match &c.constraints.allowed[k] {
            Some(allowed) => allowed[rng.random_range(0..allowed.len())],
            None => {
                let rot = rng.random_range(0..4);
                let (pw, ph) = size(&c.components[k], rot);
                Place { x: rng.random_range(0..=(w - pw).max(0)), y: rng.random_range(0..=(h - ph).max(0)), rot }
            }
        };
    } else if roll < 0.6 {
        // nudge
        new[k].x = (new[k].x + rng.random_range(-2..=2)).clamp(0, w - 1);
        new[k].y = (new[k].y + rng.random_range(-2..=2)).clamp(0, h - 1);
    } else if roll < 0.85 {
        // rotate a quarter turn either way
        new[k].rot = (new[k].rot + if rng.random() { 1 } else { 3 }) % 4;
    } else {
        // swap positions with another part, each keeping its own rotation
        let o = movable[rng.random_range(0..movable.len())];
        (new[k].x, new[k].y, new[o].x, new[o].y) = (new[o].x, new[o].y, new[k].x, new[k].y);
    }
    // Put parts with placement rules back on a place they allow (a move may have taken them off it).
    constraints::apply(c, &mut new);
    new
}

/// A random layout that's allowed: parts put down at random, then moved apart (`allowed_start`).
pub fn random_start(c: &Circuit, rng: &mut impl Rng) -> Result<Placement, String> {
    allowed_start(c, &random_placement(c, rng))
}

/// The layout `from` (a sketch as you drew it, or parts dropped at random), made allowed: each part
/// stays where it is if it can, or else goes to the nearest place where it doesn't clash with the
/// parts already down, keeping its turn if it can. Parts go down in order: those with the fewest
/// places first (a fixed part before one the board file only keeps on an edge, before a free one),
/// then the biggest, so a big part keeps its place and a small one moves out of its way. Err says
/// which part found no room.
pub fn allowed_start(c: &Circuit, from: &Placement) -> Result<Placement, String> {
    let (w, h) = (c.board.width, c.board.height);
    let n = c.components.len();
    let mut order: Vec<usize> = (0..n).collect();
    let places = |i: usize| c.constraints.allowed[i].as_ref().map_or(usize::MAX, Vec::len);
    order.sort_by_key(|&i| (places(i), std::cmp::Reverse(c.components[i].w * c.components[i].h)));

    let mut out = from.clone();
    constraints::apply(c, &mut out);
    let mut placed = vec![false; n];
    for i in order {
        placed[i] = true;
        let fits = |p: Place, out: &mut Placement| {
            out[i] = p;
            board::clashes(c, &board::geometry(c, out), &placed).is_empty()
        };
        if fits(out[i], &mut out) {
            continue;
        }
        // Every place the part could take, nearest first, the same turn before another.
        let at = out[i];
        let mut candidates: Vec<Place> = match &c.constraints.allowed[i] {
            Some(allowed) => allowed.clone(),
            None => (0..4).flat_map(|rot| (0..w).flat_map(move |x| (0..h).map(move |y| Place { x, y, rot }))).collect(),
        };
        candidates.sort_by_key(|q| (q.rot != at.rot, (q.x - at.x).abs() + (q.y - at.y).abs()));
        if !candidates.into_iter().any(|q| fits(q, &mut out)) {
            return Err(format!("no room for {} on the {w}x{h} board, next to the parts already down", c.components[i].reference));
        }
    }
    Ok(out)
}
