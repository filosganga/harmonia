//! `train` and `evolve`: teach the network to score boards the way the rules do, from example
//! boards, by gradient descent or by evolution, and save it for `place --judge net`.

use crate::board::{self, Design, Measures};
use crate::nn::{self, Net};
use crate::{OUT, model_path, place, program, version};
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::time::Instant;

/// Example boards for training: (to learn from, kept aside to test).
/// Random boards (allowed ones, like every board the placer sees), plus boards caught part-way through auto-placement so there are good ones too,
/// taking the cases of the board file's rules in turn. Each is measured as the placer judges a move,
/// routed once (`board::measure_once`): a network that learned from boards routed the thorough way
/// rarely saw a pin left unrouted, didn't learn it's bad, and placed the 555 with solder-side wires.
fn make_examples(cases: &[Design], rng: &mut StdRng) -> (Vec<Measures>, Vec<Measures>) {
    let mut examples: Vec<Measures> = Vec::new();
    for i in 0..12 {
        let c = &cases[i % cases.len()];
        let mut snap_rng = StdRng::seed_from_u64(rng.random());
        let start = place::random_start(c, rng).unwrap_or_else(|why| panic!("{why}"));
        place::anneal(c, board::cost, 4000, rng, start, |p| {
            if snap_rng.random::<f64>() < 0.02 {
                examples.push(board::measure_once(c, p));
            }
        });
    }
    for i in 0..examples.len() / 4 {
        let c = &cases[i % cases.len()];
        let board = place::random_start(c, rng).unwrap_or_else(|why| panic!("{why}"));
        examples.push(board::measure_once(c, &board));
    }
    examples.shuffle(rng);
    let test = examples.split_off(examples.len() * 4 / 5);
    println!("Made {} example boards ({} to learn from, {} kept aside to test)\n", examples.len() + test.len(), examples.len(), test.len());
    (examples, test)
}

/// The network's inputs and the right answer (the rules' score, scaled to 0..1) for each board.
fn pairs(boards: &[Measures]) -> Vec<(Vec<f64>, f64)> {
    boards.iter().map(|m| (nn::inputs(m), board::score(m) / 100.0)).collect()
}

/// Teach the network to score boards the way the rules do, by gradient descent.
///
/// 1. Make a pile of example boards.
/// 2. Ask the rules for each board's score: that's the "right answer".
/// 3. Show the network each board many times, nudging every weight in the direction that makes it less wrong.
/// 4. Test it on boards it has never seen.
pub fn train(cases: &[Design], seed: u64, again: &str, out: &str) {
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(cases, &mut rng);

    let mut net = Net::new(8);
    let mut pairs = pairs(&train);
    for epoch in 1..=200 {
        pairs.shuffle(&mut rng);
        let lr = if epoch < 120 { 0.05 } else { 0.01 };
        let loss = pairs.iter().map(|(x, y)| net.train_step(x, *y, lr)).sum::<f64>() / pairs.len() as f64;
        if [1, 2, 5, 10, 25, 50, 100, 150, 200].contains(&epoch) {
            println!("  round {epoch:3}: typical error {:5.1} points", 100.0 * loss.sqrt());
        }
    }
    finish(&net, &test, t0, seed, again, out);
}

/// The same job as `train`, done by evolution instead:
///
/// 1. Start from the same random network (the parent).
/// 2. Make 19 children, each a copy of the parent with every weight changed a little at random.
/// 3. Keep whichever of the 20 is least wrong on the example boards; it becomes the new parent.
/// 4. Repeat. Changes get smaller towards the end, to fine-tune.
pub fn evolve(cases: &[Design], seed: u64, again: &str, out: &str) {
    const CHILDREN: usize = 19;
    const GENERATIONS: usize = 1000;
    let t0 = Instant::now();
    let mut rng = StdRng::seed_from_u64(seed);
    let (train, test) = make_examples(cases, &mut rng);
    let pairs = pairs(&train);

    let mut parent = Net::new(8);
    let mut parent_err = parent.error(&pairs);
    let mut improved = 0;
    for generation in 1..=GENERATIONS {
        // From 0.1 down to 0.003: big jumps early to explore, tiny ones at the end to fine-tune.
        let size = 0.1 * 0.03f64.powf(generation as f64 / GENERATIONS as f64);
        let (best, best_err) = (0..CHILDREN)
            .map(|_| parent.mutated(size, &mut rng))
            .map(|child| {
                let err = child.error(&pairs);
                (child, err)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if best_err < parent_err {
            (parent, parent_err) = (best, best_err);
            improved += 1;
        }
        if [1, 2, 5, 10, 25, 50, 100, 250, 500, 750, 1000].contains(&generation) {
            println!("  generation {generation:4}: typical error {:5.1} points", 100.0 * parent_err.sqrt());
        }
    }
    println!("\nTried {} children; in {improved} of {GENERATIONS} generations the best child beat its parent.", GENERATIONS * CHILDREN);
    finish(&parent, &test, t0, seed, again, out);
}

/// Test the trained network on boards it never saw, and save it for `place --judge net`. `seed`
/// and `again` say how to make the same network again; it's saved in the folder `out`.
fn finish(net: &Net, test: &[Measures], t0: Instant, seed: u64, again: &str, out: &str) {
    let mean_err = test.iter().map(|m| (net.predict(m) - board::score(m)).abs()).sum::<f64>() / test.len() as f64;
    println!("\nOn {} unseen boards the network is off by {mean_err:.1} points on average.", test.len());
    println!("A few of them:");
    let mut sorted = test.to_vec();
    sorted.sort_by(|a, b| board::score(a).total_cmp(&board::score(b)));
    for m in sorted.iter().step_by((sorted.len() / 6).max(1)) {
        println!("  rules say {:5.1}   network says {:5.1}", board::score(m), net.predict(m));
    }

    let model = model_path(out);
    net.save(&model);
    let same_out = if out == OUT { String::new() } else { format!(" --out {out}") };
    println!("\nSaved to {model} in {:.2}s. Try: {} place --judge net{same_out}", t0.elapsed().as_secs_f64(), program());
    println!("Seed {seed}. To make this network again: {again} ({})", version());
}
