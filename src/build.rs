//! What you ask for (a `Request`: a circuit and its board file) and a finished board, a `Build`: the
//! case of the request it took (`Case`), what the program made of it (`Layout`: where each part goes
//! and the copper joining them), and how it was made (`Made`).
//!
//! `place` builds one, routing it once, and saves it as the layout file (out/<judge>.layout.json,
//! `file`); the SVG pictures, the KiCad board and the DIYLC project are all drawn from it. `draw`
//! reads a saved one back and draws it again with its copper as saved, so an old layout draws the
//! same after the router changes, scored with today's measurements and its own case's rules.

pub mod file;

pub use file::{from_json, to_json};

use crate::board::{Board, Placement};
use crate::circuit::Circuit;
use crate::layout::Layout;
use crate::measure::{self, Measures, Wiring};
use crate::route;
use crate::rules::{self, Case, Rule};
use serde::{Deserialize, Serialize};

/// What you ask the program for: a circuit and its board file (the board, and the rules for it),
/// with your flags applied. Answered with one layout, the best over every case of the rules.
pub struct Request {
    pub circuit: Circuit,
    pub board: Board,
    /// The board file's rules as written.
    pub rules: Vec<Rule>,
}

impl Request {
    /// The request once for each case of its rules that can be met, and why each other case can't
    /// (see `rules::cases`).
    pub fn cases(&self) -> (Vec<Case>, Vec<String>) {
        rules::cases(&self.circuit, self.board, &self.rules)
    }
}

/// How a build was made, for the record.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Made {
    /// The circuit file: a KiCad netlist or a DIYLC sketch.
    pub circuit: String,
    /// The board file next to it, if there was one.
    pub board_file: Option<String>,
    /// Who judged the layouts: "rules" or "net".
    pub judge: String,
    /// Where the placer started: "random" or "sketch".
    pub start: String,
    pub seed: u64,
    pub steps: usize,
    /// Which case of the board file's rules won, counted from 1, out of how many were placed
    /// (the case says which options it took).
    pub case: usize,
    pub cases: usize,
    /// The command that makes this build again, like `cargo run --release -- place
    /// circuits/cat_feeder.diy --seed 1 --steps 20000`: the same placement and copper with the same
    /// code (`version`), circuit and board file (and `model.json`, for the network as judge).
    pub command: String,
    /// Which program made it: its version and commit, like "harmonia 0.1.0, commit 73bf7c9".
    pub version: String,
    /// The footprints that didn't come from the built-in library, like "LED_D5.0mm from
    /// ~/.config/harmonia/footprints.json, replacing a different built-in one": placing again
    /// needs that file too. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprints: Vec<String>,
}

impl Made {
    /// One line for the outputs: the seed, and how to make the build again.
    pub fn again(&self) -> String {
        let line = format!("seed {}: to make it again, {} ({})", self.seed, self.command, self.version);
        match self.footprints.len() {
            0 => line,
            n => format!("{line}; it also needs {n} footprint{} that aren't built in (see the layout file)", if n == 1 { "" } else { "s" }),
        }
    }
}

/// A case, what was made of it, and how: everything the outputs need. The score and the
/// problems are worked out from the case and the layout, never saved ones.
pub struct Build {
    pub case: Case,
    pub layout: Layout,
    /// What the pictures and the DIYLC project are called, like "Placed by rules".
    pub title: String,
    pub made: Made,
    /// The case's measurements with this layout (`measure::measure_routed`).
    pub measures: Measures,
    /// What its copper joins, compared with what the case asks for (`measure::wiring`).
    pub wiring: Wiring,
    /// Plain-language problems, the `!!` lines: what isn't allowed (`measure::clash_words`), and pins
    /// left unrouted.
    pub problems: Vec<String>,
}

impl Build {
    /// Routes `placement` once, and keeps it all together.
    pub fn new(case: Case, placement: Placement, title: &str, made: Made) -> Build {
        let g = case.geometry(&placement);
        let copper = route::route(&case, &placement, &g);
        Build::of(case, Layout { placement, copper }, title.to_string(), made)
    }

    /// A case and a layout made of it, scored, with their problems.
    pub fn of(case: Case, layout: Layout, title: String, made: Made) -> Build {
        let g = case.geometry(&layout.placement);
        let measures = measure::measure_routed(&case, &layout.placement, &g, &layout.copper);
        let wiring = measure::wiring(&case, &g, &layout.copper);
        // A layout from the placer is always allowed; a sketch as you drew it may not be. One that
        // is allowed can't have a short (two parts on one hole make one that isn't).
        let clashes = measure::clashes(&case, &g, &vec![true; layout.placement.len()]);
        if clashes.is_empty() {
            wiring.refuse_shorts();
        }
        let mut problems = measure::clash_words(&case, &clashes);
        problems.extend(measure::unrouted_problems(&case, &wiring));
        Build { case, layout, title, made, measures, wiring, problems }
    }
}
