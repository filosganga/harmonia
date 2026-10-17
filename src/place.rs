//! The auto-placer: "simulated annealing".
//!
//! It keeps nudging, rotating and swapping parts, keeping changes the judge
//! likes and, early on, sometimes accepting worse ones so it doesn't get stuck.

use crate::board::{Circuit, Measures, Placement, SCORE_SCALE, measure, random_placement};
use crate::constraints;
use rand::Rng;

/// How much worse (in cost) a move may be and still get accepted now and then, at the start. It
/// falls to almost 0 by the end.
const START_TEMPERATURE: f64 = 20.0;
/// The same, when the placer starts from the placing network's layout: a good start is worth
/// keeping, and a hot start shakes it up.
pub const NETWORK_START_TEMPERATURE: f64 = 5.0;

/// Where the placer starts.
pub enum Start {
    /// Anywhere at random.
    Random,
    /// This layout: your sketch, say.
    Given(Placement),
    /// The placing network's layout (see `NETWORK_START_TEMPERATURE`).
    Network(Placement),
}

/// What the placer is doing, for `on_step` to look at after every step.
pub struct Step<'a> {
    /// Which step, from 0.
    pub step: usize,
    /// The current board, whether this step's move was kept or not.
    pub current: &'a Placement,
    /// The lowest cost found so far.
    pub best_cost: f64,
}

/// `judge` scores a board 0-100. `on_step` sees every step (training uses it to collect example
/// boards, the benchmark to see how fast the cost falls). Returns the best placement found.
pub fn anneal(
    c: &Circuit,
    judge: impl Fn(&Measures) -> f64,
    steps: usize,
    rng: &mut impl Rng,
    start: Start,
    mut on_step: impl FnMut(&Step),
) -> Placement {
    let (w, h) = (c.board.width, c.board.height);
    // Parts the placer may touch: everything except the ones the board file allows only one place.
    let movable: Vec<usize> = (0..c.components.len()).filter(|&i| c.constraints.allowed[i].as_ref().is_none_or(|a| a.len() > 1)).collect();
    let (mut cur, start_temperature) = match start {
        Start::Random => (random_placement(c, rng), START_TEMPERATURE),
        Start::Given(p) => (p, START_TEMPERATURE),
        Start::Network(p) => (p, NETWORK_START_TEMPERATURE),
    };
    // Compare boards by cost, not by the 0-100 score: the score flattens out for big circuits
    // (at cost 400 a much worse move barely changes it), which would make the placer accept anything.
    let cost_of = |p: &Placement| -SCORE_SCALE * (judge(&measure(c, p)).max(1e-9) / 100.0).ln();
    let mut cur_s = cost_of(&cur);
    let (mut best, mut best_s) = (cur.clone(), cur_s);
    if movable.is_empty() {
        return best;
    }

    for i in 0..steps {
        let temp = start_temperature * (1.0 - i as f64 / steps as f64) + 0.1;
        let mut new = cur.clone();
        let k = movable[rng.random_range(0..movable.len())];
        let roll: f64 = rng.random();
        let allowed = c.constraints.allowed[k].as_ref();
        if let Some(allowed) = allowed.filter(|_| rng.random::<f64>() < 0.1) {
            // jump to any place the part's rules allow: the only way across to another option of
            // a one-of that's far away (the other edge) or a half turn off
            new[k] = allowed[rng.random_range(0..allowed.len())];
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
        let new_s = cost_of(&new);
        if new_s <= cur_s || rng.random::<f64>() < ((cur_s - new_s) / temp).exp() {
            (cur, cur_s) = (new, new_s);
            if cur_s < best_s {
                (best, best_s) = (cur.clone(), cur_s);
            }
        }
        on_step(&Step { step: i, current: &cur, best_cost: best_s });
    }
    best
}
