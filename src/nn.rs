//! A tiny neural network written out by hand, no libraries.
//!
//! Each hidden neuron multiplies each input by a weight, adds them up with a
//! bias, and squashes the result with tanh. The output neuron is a plain
//! weighted sum of the hidden neurons. Training nudges every weight a little
//! in the direction that makes the prediction less wrong (that's
//! "backpropagation" + "gradient descent").
//!
//! `mutated` supports the other way to train it, evolution: make randomly
//! changed copies and keep whichever copy is least wrong.

use crate::board::{FEATURES, Measures};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};

/// Rough size of each measurement, so all inputs land around 0..1. Same order as FEATURES.
const SCALE: [f64; FEATURES.len()] = [100.0, 10.0, 5.0, 20.0, 2.0, 20.0, 20.0, 20.0, 10.0];

pub fn inputs(m: &Measures) -> Vec<f64> {
    m.0.iter().zip(SCALE).map(|(v, s)| v / s).collect()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Net {
    /// w1[j][i]: weight from input i to hidden neuron j.
    w1: Vec<Vec<f64>>,
    b1: Vec<f64>,
    /// w2[j]: weight from hidden neuron j to the output.
    w2: Vec<f64>,
    b2: f64,
}

impl Net {
    pub fn new(n_hidden: usize, seed: u64) -> Net {
        let mut rng = StdRng::seed_from_u64(seed);
        Net {
            w1: (0..n_hidden).map(|_| (0..FEATURES.len()).map(|_| 0.5 * gauss(&mut rng)).collect()).collect(),
            b1: vec![0.0; n_hidden],
            w2: (0..n_hidden).map(|_| 0.5 * gauss(&mut rng)).collect(),
            b2: 0.0,
        }
    }

    /// Returns (output, hidden neuron values).
    fn forward(&self, x: &[f64]) -> (f64, Vec<f64>) {
        let hidden: Vec<f64> = self
            .w1
            .iter()
            .zip(&self.b1)
            .map(|(ws, b)| (ws.iter().zip(x).map(|(w, xi)| w * xi).sum::<f64>() + b).tanh())
            .collect();
        let out = self.w2.iter().zip(&hidden).map(|(w, h)| w * h).sum::<f64>() + self.b2;
        (out, hidden)
    }

    /// Predicted score 0-100 for a board's measurements.
    pub fn predict(&self, m: &Measures) -> f64 {
        100.0 * self.forward(&inputs(m)).0
    }

    /// One learning step on one example. Returns the squared error before the step.
    pub fn train_step(&mut self, x: &[f64], target: f64, lr: f64) -> f64 {
        let (out, hidden) = self.forward(x);
        let err = out - target; // how wrong we are
        for (j, h) in hidden.iter().enumerate() {
            let blame = err * self.w2[j] * (1.0 - h * h); // share of the error passed back to hidden neuron j
            self.w2[j] -= lr * err * h;
            for (w, xi) in self.w1[j].iter_mut().zip(x) {
                *w -= lr * blame * xi;
            }
            self.b1[j] -= lr * blame;
        }
        self.b2 -= lr * err;
        err * err
    }

    /// Average squared error over many examples, without learning anything.
    pub fn error(&self, pairs: &[(Vec<f64>, f64)]) -> f64 {
        pairs.iter().map(|(x, y)| (self.forward(x).0 - y).powi(2)).sum::<f64>() / pairs.len() as f64
    }

    /// A copy with every weight and bias moved a random amount (typically about `size`).
    /// This is the "child" in evolution: no idea which way is better, just a random change.
    pub fn mutated(&self, size: f64, rng: &mut impl Rng) -> Net {
        let mut child = self.clone();
        let mut nudge = |v: &mut f64| *v += size * gauss(rng);
        child.w1.iter_mut().flatten().for_each(&mut nudge);
        child.b1.iter_mut().for_each(&mut nudge);
        child.w2.iter_mut().for_each(&mut nudge);
        nudge(&mut child.b2);
        child
    }

    pub fn save(&self, path: &str) {
        std::fs::write(path, serde_json::to_string(self).unwrap()).unwrap();
    }

    pub fn load(path: &str) -> Net {
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| panic!("no {path}: run `cargo run --release -- train` first"));
        let net: Net = serde_json::from_str(&text).unwrap();
        if net.w1.first().map_or(0, |ws| ws.len()) != FEATURES.len() {
            panic!("{path} was trained on different measurements: run `cargo run --release -- train` again");
        }
        net
    }
}

/// Normal-distributed random numbers (Box-Muller), so we don't need another crate.
fn gauss(rng: &mut impl Rng) -> f64 {
    let (u1, u2): (f64, f64) = (rng.random::<f64>().max(1e-12), rng.random());
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}
