//! A finished board, a `Build`: what you asked for (`Design`), what the program made of it
//! (`Layout`: where each part goes and the copper joining them), and how it was made (`Made`).
//!
//! `place` builds one, routing it once, and saves it as the layout file (out/<judge>.layout.json,
//! `file`); the SVG pictures, the KiCad board and the DIYLC project are all drawn from it. `draw`
//! reads a saved one back and draws it again with its copper as saved, so an old layout draws the
//! same after the router changes, scored with today's measurements and its own design's rules.

pub mod file;

pub use file::{from_json, to_json};

use crate::board::{self, Design, Measures, Placement, Point};
use crate::route::{self, Routing};
use serde::{Deserialize, Serialize};

/// What the program made of a design: where each part goes, and the copper joining them.
#[derive(Clone)]
pub struct Layout {
    pub placement: Placement,
    pub routing: Routing,
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
    /// (the design says which options it took).
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
    /// One line for the drawings: the seed, and how to make the build again.
    pub fn again(&self) -> String {
        let line = format!("seed {}: to make it again, {} ({})", self.seed, self.command, self.version);
        match self.footprints.len() {
            0 => line,
            n => format!("{line}; it also needs {n} footprint{} that aren't built in (see the layout file)", if n == 1 { "" } else { "s" }),
        }
    }
}

/// A design, what was made of it, and how: everything the drawings need. The score and the
/// problems are worked out from the design and the layout, never saved ones.
pub struct Build {
    /// Its runs are in the order the router routes them first (`route::order`), so each of the
    /// layout's copper lists has one run's pieces after the other, runs in order.
    pub design: Design,
    pub layout: Layout,
    /// What the pictures and the DIYLC project are called, like "Placed by rules".
    pub title: String,
    pub made: Made,
    /// The design's measurements with this layout (`board::measure_routed`).
    pub measures: Measures,
    /// Plain-language problems, the `!!` lines: what isn't allowed (`board::clash_words`), and pins
    /// left unrouted.
    pub problems: Vec<String>,
}

impl Build {
    /// Routes `placement` once, and keeps it all together.
    pub fn new(design: Design, placement: Placement, title: &str, made: Made) -> Build {
        let g = board::geometry(&design, &placement);
        let routing = route::route(&design, &placement, &g);
        // Put the runs in the order the router routes them first (`route::order`), numbering the
        // copper to match. It's the same copper; only the run numbers change. The file then lists
        // the runs in that order, and reading it back gives the copper in the same order.
        let order = route::order(&design, &g);
        let mut renumbered = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            renumbered[old] = new;
        }
        let renumber = |list: Vec<(usize, Point, Point)>| list.into_iter().map(|(run, a, b)| (renumbered[run], a, b)).collect();
        let routing = Routing {
            traces: renumber(routing.traces),
            jumpers: renumber(routing.jumpers),
            bottom: renumber(routing.bottom),
            unrouted: renumber(routing.unrouted),
            joins: routing.joins.into_iter().map(|(run, at)| (renumbered[run], at)).collect(),
        };
        let mut design = design;
        design.runs = order.iter().map(|&old| design.runs[old].clone()).collect();
        Build::of(design, Layout { placement, routing }, title.to_string(), made)
    }

    /// A design and a layout made of it, scored, with their problems.
    pub fn of(design: Design, layout: Layout, title: String, made: Made) -> Build {
        let g = board::geometry(&design, &layout.placement);
        let measures = board::measure_routed(&design, &layout.placement, &g, &layout.routing);
        // A layout from the placer is always allowed; a sketch as you drew it may not be.
        let clashes = board::clashes(&design, &g, &vec![true; layout.placement.len()]);
        let mut problems = board::clash_words(&design, &clashes);
        problems.extend(route::unrouted_problems(&design, &g, &layout.routing));
        Build { design, layout, title, made, measures, problems }
    }
}
