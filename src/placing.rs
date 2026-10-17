//! The placing network (docs/plans/placing-network.md, stage 2): with some parts already on the
//! board, where does the next one go?
//!
//! Every place the next part may take (on the board, off the parts already there and the
//! keep-outs, meeting the board file's rules) is drawn as a picture of the board with the part
//! there: a stack of grids, one per kind of thing (the "channels", see `draw`). A small
//! convolutional network gives each picture a score, and the highest wins.
//!
//! A convolution slides a small grid of weights (3 x 3 holes here) over the whole picture, the same
//! weights everywhere, so it learns local patterns ("a pin right next to another part") wherever
//! they are. Three in a row see 7 x 7 holes around each hole. Adding up what they found over the
//! whole board, and taking the strongest, gives one score.
//!
//! It learns from the placer's layouts (`data`). For each part, in `data::placing_order`, the parts
//! before it are where the placer put them, and the place the placer chose should score highest.
//! Softmax turns the scores of one decision into probabilities that add up to 1; the loss
//! (cross-entropy) is -log of the probability given to the placer's choice, and each training step
//! nudges every weight in the direction that makes it smaller.

use crate::board::{self, Circuit, Component, Place, Point};
use crate::{bench, data, parallel};
use burn::nn::conv::{Conv2d, Conv2dConfig};
use burn::nn::{Linear, LinearConfig, PaddingConfig2d, Relu};
use burn::optim::{AdamConfig, GradientsParams};
use burn::prelude::*;
use burn::tensor::{TensorData, activation::log_softmax};
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::collections::HashSet;
use std::time::Instant;

const MODEL: &str = "out/placer.bpk";
/// Every circuit whose number is a multiple of this is kept aside to test on, never learned from.
const HELD_OUT: u64 = 10;
/// The kinds of thing drawn, one grid each (see `draw`).
const CHANNELS: usize = 9;
/// How many grids each convolution makes from the ones before it.
const FILTERS: [usize; 3] = [16, 32, 32];
/// Wrong places compared with the placer's choice in each training step, picked at random each
/// time (and in the loss on the circuits kept aside, the same ones every epoch). The final test
/// compares it with all of them.
const NEGATIVES: usize = 63;
/// Passes over all the training examples.
const EPOCHS: usize = 5;
/// How big a nudge each step gives the weights (Adam, the optimizer, adapts it for each weight).
const LEARNING_RATE: f64 = 1e-3;
const SEED: u64 = 1;

/// The network: three convolutions, then one score from what they found.
#[derive(Module, Debug)]
pub struct Net {
    conv1: Conv2d,
    conv2: Conv2d,
    conv3: Conv2d,
    score: Linear,
    relu: Relu,
}

impl Net {
    fn new(device: &Device) -> Net {
        // "Same" padding keeps each grid the size of the board.
        let conv = |from, to| Conv2dConfig::new([from, to], [3, 3]).with_padding(PaddingConfig2d::Same).init(device);
        Net {
            conv1: conv(CHANNELS, FILTERS[0]),
            conv2: conv(FILTERS[0], FILTERS[1]),
            conv3: conv(FILTERS[1], FILTERS[2]),
            score: LinearConfig::new(2 * FILTERS[2], 1).init(device),
            relu: Relu::new(),
        }
    }

    /// Pictures [n, CHANNELS, height, width] -> one score each [n]. Higher is better.
    fn forward(&self, pictures: Tensor<4>) -> Tensor<1> {
        // relu keeps a convolution's positive answers and zeroes the rest: "found it here, or not".
        let x = self.relu.forward(self.conv1.forward(pictures));
        let x = self.relu.forward(self.conv2.forward(x));
        let x = self.relu.forward(self.conv3.forward(x));
        let [n, filters, _, _] = x.dims();
        // Added up over the board: how much of each pattern there is (how long the ratsnest lines are).
        let total = x.clone().sum_dim(3).sum_dim(2).reshape([n, filters]).mul_scalar(0.01);
        // The strongest anywhere: whether a pattern is there at all.
        let strongest = x.max_dim(3).max_dim(2).reshape([n, filters]);
        self.score.forward(Tensor::cat(vec![total, strongest], 1)).reshape([n])
    }
}

/// A circuit with a layout: to learn from, or the right answers to test on.
struct Lesson {
    circuit: Circuit,
    layout: Vec<Place>,
}

/// One decision: with the parts before `part` in the placing order where the layout has them,
/// where does `part` go? `answer` is where the layout put it, among `candidates`.
struct Decision {
    part: usize,
    scene: Scene,
    candidates: Vec<Place>,
    answer: usize,
}

/// The board so far: what the parts already placed take and where their pins are.
#[derive(Clone)]
struct Scene {
    /// Holes taken by placed parts' bodies.
    taken: HashSet<Point>,
    /// Holes under placed modules, where only low parts fit.
    under: HashSet<Point>,
    /// Holes inside the outlines of placed parts that aren't low (they can't go under a module).
    tall: HashSet<Point>,
    /// For each net, the pins of placed parts on it.
    net_pins: Vec<Vec<Point>>,
    /// Pins of placed parts on the supply and ground nets.
    supply: Vec<Point>,
    ground: Vec<Point>,
}

/// Where a part's body, outline and pins are when it's at `p`.
struct Shape {
    body: Vec<Point>,
    outline: Vec<Point>,
    pins: Vec<Point>,
}

fn shape(comp: &Component, p: Place) -> Shape {
    let at = |hole: Point| {
        let (dx, dy) = board::turn(hole, comp.w, comp.h, p.rot);
        (p.x + dx, p.y + dy)
    };
    Shape {
        body: comp.body.iter().map(|&h| at(h)).collect(),
        outline: Component::full_body(comp.w, comp.h).into_iter().map(at).collect(),
        pins: comp.pins.iter().map(|pin| at(pin.at)).collect(),
    }
}

fn same(a: Place, b: Place) -> bool {
    (a.x, a.y, a.rot % 4) == (b.x, b.y, b.rot % 4)
}

/// The net of each pin of each part, if it's on one.
fn pin_nets(c: &Circuit) -> Vec<Vec<Option<usize>>> {
    let mut nets: Vec<Vec<Option<usize>>> = c.components.iter().map(|comp| vec![None; comp.pins.len()]).collect();
    for (net, pins) in c.net_pins.iter().enumerate() {
        for &(ci, pi) in pins {
            nets[ci][pi] = Some(net);
        }
    }
    nets
}

fn net_index(c: &Circuit, name: &Option<String>) -> Option<usize> {
    name.as_ref().and_then(|n| c.nets.iter().position(|net| &net.name == n))
}

/// Every decision in a layout, in the placing order. Decisions with only one place (a fixed
/// part) aren't decisions, and are left out; so is a part the layout put where no part may go
/// (on another one), which the placer sometimes can't avoid.
fn decisions(lesson: &Lesson) -> Vec<Decision> {
    let c = &lesson.circuit;
    let nets = pin_nets(c);
    let mut scene = Scene::new(c);
    let mut out = Vec::new();
    for part in data::placing_order(c) {
        let candidates = candidates(c, &scene, part);
        if let Some(answer) = candidates.iter().position(|&p| same(p, lesson.layout[part]))
            && candidates.len() > 1
        {
            out.push(Decision { part, scene: scene.clone(), candidates, answer });
        }
        scene.put(c, &nets, part, lesson.layout[part]);
    }
    out
}

impl Scene {
    /// An empty board.
    fn new(c: &Circuit) -> Scene {
        Scene {
            taken: HashSet::new(),
            under: HashSet::new(),
            tall: HashSet::new(),
            net_pins: vec![Vec::new(); c.nets.len()],
            supply: Vec::new(),
            ground: Vec::new(),
        }
    }

    /// Puts `part` down at `p`.
    fn put(&mut self, c: &Circuit, nets: &[Vec<Option<usize>>], part: usize, p: Place) {
        let s = shape(&c.components[part], p);
        self.taken.extend(&s.body);
        if s.body.len() < s.outline.len() {
            self.under.extend(s.outline.iter().filter(|h| !s.body.contains(h)));
        }
        if !c.components[part].low {
            self.tall.extend(&s.outline);
        }
        let (supply_net, ground_net) = (net_index(c, &c.power), net_index(c, &c.ground));
        for (pin, &hole) in s.pins.iter().enumerate() {
            if let Some(net) = nets[part][pin] {
                self.net_pins[net].push(hole);
                if Some(net) == supply_net {
                    self.supply.push(hole);
                }
                if Some(net) == ground_net {
                    self.ground.push(hole);
                }
            }
        }
    }
}

/// The places `part` may take on the board so far: the ones its placement rules allow (or
/// anywhere on the board), with its outline on the board, its body off placed parts and
/// keep-outs, and nothing tall under a module.
fn candidates(c: &Circuit, scene: &Scene, part: usize) -> Vec<Place> {
    let comp = &c.components[part];
    let (w, h) = (c.board.width, c.board.height);
    let places: Vec<Place> = match &c.constraints.allowed[part] {
        Some(allowed) => allowed.clone(),
        None => (0..4u8)
            .flat_map(|rot| {
                let (pw, ph) = board::size(comp, rot);
                (0..=h - ph).flat_map(move |y| (0..=w - pw).map(move |x| Place { x, y, rot }))
            })
            .collect(),
    };
    let is_module = comp.body.len() < (comp.w * comp.h) as usize;
    places
        .into_iter()
        .filter(|&p| {
            let s = shape(comp, p);
            s.outline.iter().all(|&(x, y)| x >= 0 && x < w && y >= 0 && y < h)
                && s.body.iter().all(|hole| !scene.taken.contains(hole) && !c.constraints.keepout_holes.contains(hole))
                && (comp.low || s.outline.iter().all(|hole| !scene.under.contains(hole)))
                && (!is_module || s.outline.iter().filter(|h| !s.body.contains(h)).all(|hole| !scene.tall.contains(hole)))
        })
        .collect()
}

/// What each channel shows.
const TAKEN: usize = 0; // holes taken by parts already placed
const BODY: usize = 1; // the new part's body
const PINS: usize = 2; // the new part's pins
const NET_PINS: usize = 3; // placed pins on a net one of the new part's pins is on
const RATSNEST: usize = 4; // straight lines from each new pin to the nearest placed pin of its net
const SUPPLY: usize = 5; // supply pins, placed and new
const GROUND: usize = 6; // ground pins, placed and new
const OUTSIDE: usize = 7; // off the board (a border one hole wide) and keep-outs
const UNDER: usize = 8; // under a module's board (placed, or the new part if it's one): only low parts fit

/// A picture's height and width: the board and a one-hole border around it.
fn picture_size(c: &Circuit) -> (usize, usize) {
    (c.board.height as usize + 2, c.board.width as usize + 2)
}

/// The channels that don't depend on where the new part goes.
fn background(c: &Circuit, part: usize, scene: &Scene, nets: &[Vec<Option<usize>>]) -> Vec<f32> {
    let (ph, pw) = picture_size(c);
    let mut grid = vec![0.0; CHANNELS * ph * pw];
    let mut set = |channel: usize, (x, y): Point| {
        if x >= -1 && y >= -1 && (x + 1) < pw as i32 && (y + 1) < ph as i32 {
            grid[channel * ph * pw + (y + 1) as usize * pw + (x + 1) as usize] = 1.0;
        }
    };
    for x in -1..=c.board.width {
        set(OUTSIDE, (x, -1));
        set(OUTSIDE, (x, c.board.height));
    }
    for y in -1..=c.board.height {
        set(OUTSIDE, (-1, y));
        set(OUTSIDE, (c.board.width, y));
    }
    c.constraints.keepout_holes.iter().for_each(|&hole| set(OUTSIDE, hole));
    scene.taken.iter().for_each(|&hole| set(TAKEN, hole));
    scene.under.iter().for_each(|&hole| set(UNDER, hole));
    scene.supply.iter().for_each(|&hole| set(SUPPLY, hole));
    scene.ground.iter().for_each(|&hole| set(GROUND, hole));
    for net in nets[part].iter().flatten() {
        scene.net_pins[*net].iter().for_each(|&hole| set(NET_PINS, hole));
    }
    grid
}

/// Appends the picture of the board so far with the new part at `p` to `out`.
fn draw(c: &Circuit, part: usize, scene: &Scene, background: &[f32], nets: &[Vec<Option<usize>>], p: Place, out: &mut Vec<f32>) {
    let (ph, pw) = picture_size(c);
    let start = out.len();
    out.extend_from_slice(background);
    let grid = &mut out[start..];
    let mut set = |channel: usize, (x, y): Point| {
        grid[channel * ph * pw + (y + 1) as usize * pw + (x + 1) as usize] = 1.0;
    };
    let s = shape(&c.components[part], p);
    s.body.iter().for_each(|&hole| set(BODY, hole));
    s.outline.iter().filter(|h| !s.body.contains(h)).for_each(|&hole| set(UNDER, hole));
    let (supply_net, ground_net) = (net_index(c, &c.power), net_index(c, &c.ground));
    for (pin, &hole) in s.pins.iter().enumerate() {
        set(PINS, hole);
        let Some(net) = nets[part][pin] else { continue };
        if Some(net) == supply_net {
            set(SUPPLY, hole);
        }
        if Some(net) == ground_net {
            set(GROUND, hole);
        }
        if let Some(&to) = nearest(hole, &scene.net_pins[net]) {
            line(hole, to).into_iter().for_each(|h| set(RATSNEST, h));
        }
    }
}

fn distance((ax, ay): Point, (bx, by): Point) -> f32 {
    (((ax - bx).pow(2) + (ay - by).pow(2)) as f32).sqrt()
}

fn nearest(from: Point, pins: &[Point]) -> Option<&Point> {
    pins.iter().min_by(|a, b| distance(from, **a).total_cmp(&distance(from, **b)))
}

/// The holes along a straight line from `a` to `b`, ends included.
fn line(a: Point, b: Point) -> Vec<Point> {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            ((a.0 as f32 + t * (b.0 - a.0) as f32).round() as i32, (a.1 as f32 + t * (b.1 - a.1) as f32).round() as i32)
        })
        .collect()
}

/// The pictures for some decisions, ready for the network: for each, the placer's choice first and
/// then the other places given.
struct Batch {
    pictures: Vec<f32>,
    height: usize,
    width: usize,
    /// How many pictures each decision has.
    sizes: Vec<usize>,
}

impl Batch {
    fn tensor(self, device: &Device) -> Tensor<4> {
        let n = self.sizes.iter().sum();
        Tensor::from_data(TensorData::new(self.pictures, [n, CHANNELS, self.height, self.width]), device)
    }
}

/// A training batch from one lesson: each decision with the placer's choice and up to NEGATIVES
/// other places, picked at random.
fn training_batch(lesson: &Lesson, rng: &mut StdRng) -> Batch {
    let c = &lesson.circuit;
    let nets = pin_nets(c);
    let (height, width) = picture_size(c);
    let mut batch = Batch { pictures: Vec::new(), height, width, sizes: Vec::new() };
    for d in decisions(lesson) {
        let background = background(c, d.part, &d.scene, &nets);
        let mut others: Vec<usize> = (0..d.candidates.len()).filter(|&i| i != d.answer).collect();
        others.shuffle(rng);
        others.truncate(NEGATIVES);
        for i in std::iter::once(d.answer).chain(others.iter().copied()) {
            draw(c, d.part, &d.scene, &background, &nets, d.candidates[i], &mut batch.pictures);
        }
        batch.sizes.push(1 + others.len());
    }
    batch
}

/// The cross-entropy of each decision (its placer's choice is its first picture), averaged.
fn loss(scores: Tensor<1>, sizes: &[usize]) -> Tensor<1> {
    let mut start = 0;
    let mut losses = Vec::new();
    for &n in sizes {
        // log_softmax: the logarithm of softmax's probabilities, in one step that keeps the sums exact.
        let log_probabilities = log_softmax(scores.clone().slice(start..start + n), 0);
        losses.push(log_probabilities.slice(0..1).neg());
        start += n;
    }
    Tensor::cat(losses, 0).mean()
}

/// How often the network's first choice is the placer's (top-1) and the placer's choice is among its
/// first five (top-5), from 0 to 1, on the circuits kept aside and on the benchmark circuits.
pub struct Tested {
    pub held_out: [f64; 2],
    pub benchmark: [f64; 2],
}

/// Trains the placing network on out/data/layouts.jsonl, tests it on the circuits kept aside and
/// on the benchmark, and saves it to out/placer.bpk.
pub fn learn() -> Tested {
    let t0 = Instant::now();
    let records = data::read();
    if records.is_empty() {
        panic!("no training data yet: run `cargo run --release -- data` first");
    }
    let lesson = |r: &data::Record| Lesson {
        circuit: data::cases(&r.parts, &r.nets, &r.board_file).swap_remove(r.case),
        layout: r.layout.iter().map(|&[x, y, rot]| Place { x, y, rot: rot as u8 }).collect(),
    };
    let train: Vec<Lesson> = records.iter().filter(|r| r.index % HELD_OUT != 0).map(lesson).collect();
    let test: Vec<Lesson> = records.iter().filter(|r| r.index % HELD_OUT == 0).map(lesson).collect();
    let saved = bench::saved().unwrap_or_else(|why| panic!("{why}"));
    let benchmark: Vec<Lesson> = saved
        .results
        .iter()
        .map(|r| Lesson {
            circuit: bench::load(&r.circuit).swap_remove(r.best_case),
            layout: r.best_layout.iter().map(|&[x, y, rot]| Place { x, y, rot: rot as u8 }).collect(),
        })
        .collect();
    println!(
        "Learning from {} random circuits; testing on {} kept aside and the {} benchmark circuits.",
        train.len(),
        test.len(),
        benchmark.len()
    );

    let device = Device::flex().autodiff();
    device.seed(SEED);
    let mut net = Net::new(&device);
    let mut optimizer = AdamConfig::new().init();
    let mut rng = StdRng::seed_from_u64(SEED);
    let workers = parallel::workers();
    for epoch in 1..=EPOCHS {
        let started = Instant::now();
        let mut order: Vec<usize> = (0..train.len()).collect();
        order.shuffle(&mut rng);
        let (mut total, mut steps) = (0.0, 0);
        // Pictures for a few lessons are drawn at once, on every core; then the network learns
        // from them one lesson (one circuit, so one board size) at a time.
        for chunk in order.chunks(2 * workers) {
            let seeds: Vec<(usize, u64)> = chunk.iter().map(|&i| (i, rand::Rng::random(&mut rng))).collect();
            let batches = parallel::map(&seeds, |&(i, seed)| training_batch(&train[i], &mut StdRng::seed_from_u64(seed)));
            for batch in batches.into_iter().filter(|b| !b.sizes.is_empty()) {
                let sizes = batch.sizes.clone();
                let loss = loss(net.forward(batch.tensor(&device)), &sizes);
                total += loss.clone().into_scalar::<f32>() as f64;
                steps += 1;
                let grads = GradientsParams::from_grads(loss.backward(), &net);
                net = optimizer.step(LEARNING_RATE, net, grads);
            }
        }
        let held_out = held_out_loss(&net.valid(), &test);
        println!(
            "  epoch {epoch}: loss {:.3} on the training circuits, {held_out:.3} on the ones kept aside ({:.0}s)",
            total / steps as f64,
            started.elapsed().as_secs_f64()
        );
    }
    net.clone().save_file(MODEL).unwrap_or_else(|e| panic!("can't save {MODEL}: {e:?}"));

    let model = net.valid();
    println!("\nHow often the placer's choice is the first choice (top-1) or among the first five (top-5):");
    let held_out = report("random circuits kept aside", &model, &test);
    let benchmark = report("benchmark circuits", &model, &benchmark);
    println!("\nSaved to {MODEL} in {:.0}s.", t0.elapsed().as_secs_f64());
    Tested { held_out, benchmark }
}

/// The loss on the circuits kept aside, each decision against NEGATIVES other places (the same
/// ones every epoch), to see whether learning carries over to circuits it doesn't learn from.
fn held_out_loss(model: &Net, test: &[Lesson]) -> f64 {
    let device = Device::flex();
    let seeds: Vec<usize> = (0..test.len()).collect();
    let batches = parallel::map(&seeds, |&i| training_batch(&test[i], &mut StdRng::seed_from_u64(i as u64)));
    let (mut total, mut count) = (0.0, 0);
    for batch in batches.into_iter().filter(|b| !b.sizes.is_empty()) {
        let sizes = batch.sizes.clone();
        total += loss(model.forward(batch.tensor(&device)), &sizes).into_scalar::<f32>() as f64 * sizes.len() as f64;
        count += sizes.len();
    }
    total / count as f64
}

/// The chance the answer is among the first `k` by score (higher first), counting ties fairly:
/// with `better` places scoring higher and `tied` scoring the same (the answer included), it's
/// in the first k in (k - better) of the `tied` orders of the tie.
fn top(scores: &[f32], answer: usize, k: usize) -> f64 {
    let s = scores[answer];
    let better = scores.iter().filter(|&&x| x > s).count();
    let tied = scores.iter().filter(|&&x| x == s).count();
    (k.saturating_sub(better) as f64 / tied as f64).min(1.0)
}

/// The network's score for each candidate place of `part` on the board so far.
fn scores(model: &Net, c: &Circuit, part: usize, scene: &Scene, nets: &[Vec<Option<usize>>], candidates: &[Place], device: &Device) -> Vec<f32> {
    let (height, width) = picture_size(c);
    let background = background(c, part, scene, nets);
    let mut out = Vec::with_capacity(candidates.len());
    for chunk in candidates.chunks(1024) {
        let mut batch = Batch { pictures: Vec::new(), height, width, sizes: vec![chunk.len()] };
        for &p in chunk {
            draw(c, part, scene, &background, nets, p, &mut batch.pictures);
        }
        out.extend(model.forward(batch.tensor(device)).try_to_vec_as::<f32>().expect("the scores can be read"));
    }
    out
}

/// The network saved by `learn`, for placing.
pub fn load() -> Net {
    if !std::path::Path::new(MODEL).exists() {
        panic!("no placing network yet ({MODEL}): run `cargo run --release -- learn` first");
    }
    Net::new(&Device::flex()).load_file(MODEL)
}

/// The network's layout for a circuit: each part in the placing order goes where the network scores
/// best, with the parts before it where it put them. A part with nowhere left to go (the parts
/// before it took the room) stays where `random_placement` put it, for the placer to sort out.
/// Returns the layout and how many parts that happened to.
pub fn propose(c: &Circuit, model: &Net, rng: &mut impl Rng) -> (board::Placement, usize) {
    let device = Device::flex();
    let nets = pin_nets(c);
    let mut layout = board::random_placement(c, rng);
    let mut scene = Scene::new(c);
    let mut stuck = 0;
    for part in data::placing_order(c) {
        let candidates = candidates(c, &scene, part);
        match candidates.len() {
            0 => stuck += 1,
            1 => layout[part] = candidates[0],
            _ => {
                let scores = scores(model, c, part, &scene, &nets, &candidates, &device);
                let best = (0..candidates.len()).max_by(|&a, &b| scores[a].total_cmp(&scores[b])).unwrap();
                layout[part] = candidates[best];
            }
        }
        scene.put(c, &nets, part, layout[part]);
    }
    (layout, stuck)
}

/// Prints top-1 and top-5 on some lessons, for the network and the simple rules to beat, and
/// returns the network's.
fn report(what: &str, model: &Net, lessons: &[Lesson]) -> [f64; 2] {
    let device = Device::flex();
    // [network, shortest ratsnest, next to the part sharing most nets, random] x [top-1, top-5]
    let mut hits = [[0.0; 2]; 4];
    let mut count = 0;
    for lesson in lessons {
        let c = &lesson.circuit;
        let nets = pin_nets(c);
        for d in decisions(lesson) {
            let network = scores(model, c, d.part, &d.scene, &nets, &d.candidates, &device);
            let ratsnest: Vec<f32> = d.candidates.iter().map(|&p| -ratsnest_length(c, &d, &nets, p)).collect();
            let neighbour: Vec<f32> = d.candidates.iter().map(|&p| -distance_to_partner(c, lesson, &d, &nets, p)).collect();
            let random = vec![0.0; d.candidates.len()];
            for (rule, scores) in [network, ratsnest, neighbour, random].iter().enumerate() {
                hits[rule][0] += top(scores, d.answer, 1);
                hits[rule][1] += top(scores, d.answer, 5);
            }
            count += 1;
        }
    }
    println!("\n  {what} ({count} decisions)             top-1   top-5");
    let names = ["the network", "shortest ratsnest", "next to the part sharing most nets", "random guess"];
    for (name, [one, five]) in names.iter().zip(hits) {
        println!("    {name:<36} {:>5.1}%  {:>5.1}%", 100.0 * one / count as f64, 100.0 * five / count as f64);
    }
    [hits[0][0] / count as f64, hits[0][1] / count as f64]
}

/// A simple rule to beat: the total straight-line distance from each of the new part's pins to the
/// nearest placed pin of its net.
fn ratsnest_length(c: &Circuit, d: &Decision, nets: &[Vec<Option<usize>>], p: Place) -> f32 {
    let s = shape(&c.components[d.part], p);
    s.pins
        .iter()
        .enumerate()
        .filter_map(|(pin, &hole)| nets[d.part][pin].and_then(|net| nearest(hole, &d.scene.net_pins[net]).map(|&to| distance(hole, to))))
        .sum()
}

/// The plan's simple rule: put the part next to the placed part it shares the most nets with, as
/// the distance between their middles. 0 for every place if no placed part shares a net.
fn distance_to_partner(c: &Circuit, lesson: &Lesson, d: &Decision, nets: &[Vec<Option<usize>>], p: Place) -> f32 {
    let mine: HashSet<usize> = nets[d.part].iter().flatten().copied().collect();
    let placed: HashSet<usize> = data::placing_order(c).into_iter().take_while(|&q| q != d.part).collect();
    let partner = placed
        .iter()
        .map(|&q| (nets[q].iter().flatten().filter(|n| mine.contains(n)).collect::<HashSet<_>>().len(), q))
        .filter(|&(shared, _)| shared > 0)
        .max();
    let Some((_, q)) = partner else { return 0.0 };
    let middle = |outline: &[Point]| {
        let n = outline.len() as f32;
        (outline.iter().map(|h| h.0 as f32).sum::<f32>() / n, outline.iter().map(|h| h.1 as f32).sum::<f32>() / n)
    };
    let (a, b) = (middle(&shape(&c.components[d.part], p).outline), middle(&shape(&c.components[q], lesson.layout[q]).outline));
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}
