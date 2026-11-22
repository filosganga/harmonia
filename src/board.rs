//! Perfboard model: the circuit, the board, and where the parts are (placement geometry), plus each
//! net's colour for the outputs. How a layout is judged is in the `measure` module (src/measure.rs).
//!
//! Coordinates are perfboard holes: x grows to the right, y grows downwards.
//! A placement gives each component the position of its top-left hole and a
//! rotation (0..3, in 90 degree steps).

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Point = (i32, i32);

#[derive(Clone)]
pub struct Pin {
    pub name: String,
    /// Its hole. For a pin between holes (`off_grid`), the nearest one.
    pub at: Point,
    /// The pin isn't on a hole, like a tactile switch's legs. Copper in the holes around it could
    /// touch it, so the router keeps other nets out of them.
    pub off_grid: bool,
}

/// The shape of a part on the board, in its own holes (before it's turned): its name, its outline,
/// its pins, the holes its body covers, and its look. Many parts can use the same one (each `Part` keeps its
/// own copy).
#[derive(Clone)]
pub struct Footprint {
    /// Full KiCad footprint name, e.g. "Package_DIP:DIP-8_W7.62mm", or "DIYLC:..." for a part read
    /// from a DIYLC sketch.
    pub name: String,
    /// Its outline: w x h holes.
    pub w: i32,
    pub h: i32,
    pub pins: Vec<Pin>,
    /// The holes its part stands on, inside the outline: from the board up to the part's top layer
    /// (`Part::layers`). For most footprints that's the whole outline; a module's only its pins (its
    /// headers), and a two-legged part drawn at an angle the line between its legs.
    pub body: Vec<Point>,
    /// What it looks like from above, in the pictures.
    pub look: Look,
    /// How file formats draw it (footprints.json's "diylc" entry, for DIYLC).
    pub drawings: Drawings,
}

/// What a footprint looks like from above, in the pictures: no file format's words. A look for
/// two legs (lying, round) on a footprint without exactly two pins is drawn as a box.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "kebab-case")]
pub enum Look {
    /// A box over its outline: a chip, a transistor, a switch, anything else.
    #[default]
    Box,
    /// Lying flat between its two legs, like a resistor; with a band at its second leg, a diode.
    Lying {
        #[serde(default, skip_serializing_if = "is_false")]
        band: bool,
    },
    /// Standing up, a round body over its two legs, like an electrolytic capacitor, with a + by
    /// the pin called `plus`; lit, an LED.
    Round {
        plus: String,
        #[serde(default, skip_serializing_if = "is_false")]
        lit: bool,
    },
    /// A small rounded box, like a ceramic disc capacitor.
    Disc,
    /// Only its pins, boxed and named, like a pin header.
    Header,
    /// A board of its own over its outline (a microcontroller board, a breakout), its pins boxed
    /// and named by header.
    OwnBoard,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// A file format that keeps its own drawings of parts and footprints.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Diylc,
}

/// How one file format draws a part or a footprint, in that format's own JSON, kept with it so
/// that format can write it back. Only that format looks inside.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Drawing(pub serde_json::Value);

/// A part's or a footprint's drawings, one per format at most.
pub type Drawings = BTreeMap<Format, Drawing>;

impl Footprint {
    /// Every hole of a w x h outline: the body of an ordinary part.
    pub fn full_body(w: i32, h: i32) -> Vec<Point> {
        (0..w).flat_map(|x| (0..h).map(move |y| (x, y))).collect()
    }
}

/// One thing soldered to the board, named by its reference (R1, U1): a value, a footprint, and
/// the layers it takes.
#[derive(Clone)]
pub struct Part {
    pub reference: String,
    /// What the netlist says it is, e.g. "NE555P" or "10k". Only used when exporting.
    pub value: String,
    pub footprint: Footprint,
    /// How file formats draw this part itself: for a part read from a DIYLC sketch, your drawing
    /// of it, so it can be written back moved.
    pub drawings: Drawings,
    /// Which layers it takes, unless the board file says otherwise (`Design::layers`).
    pub layers: Layers,
    /// A module: a board of its own (a microcontroller, a motor driver breakout), which covers its
    /// whole outline at its layers while standing only on its body. (The pictures draw it by its
    /// footprint's look, `Look::OwnBoard`.)
    pub module: bool,
    /// The footprints.json its footprint came from, if not the built-in library (yours, or one next
    /// to the netlist), so a layout can say it depends on that file.
    pub footprint_from: Option<String>,
}

/// The copper drawn in a DIYLC sketch, in holes of the board, each piece with the index of its net
/// in the circuit: what `route::from_sketch` turns into a routing, to score your own wiring the way the
/// router's is scored. A piece that joins no pins is left out.
#[derive(Default, Clone)]
pub struct Copper {
    /// Copper traces, (net, one end, the other), as drawn: a trace may be many holes long.
    pub traces: Vec<(usize, Point, Point)>,
    /// Jumpers and wires on the parts' side.
    pub jumpers: Vec<(usize, Point, Point)>,
    /// Wires on the solder side: a jumper whose name says "(solder side)", as this program writes them.
    pub bottom: Vec<(usize, Point, Point)>,
}

/// Which layers above the board a part takes, `lo` to `hi`, both included: 0 is the board's
/// surface (copper, a module soldered flat on its pads), 1 where parts lie and jumper wires run,
/// 2 above them (a module's board on headers). A part takes them over its body from the surface
/// up, `0..=hi` (its legs, or itself), and a module's board `lo..=hi` over the rest of its
/// outline; two things clash where they share a hole and a layer. So a resistor lying flat (1-1)
/// fits under a module on headers (2-2), and a tall capacitor (1-2) doesn't. Written [lo, hi].
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(from = "[u8; 2]", into = "[u8; 2]")]
pub struct Layers {
    pub lo: u8,
    pub hi: u8,
}

impl From<[u8; 2]> for Layers {
    fn from([lo, hi]: [u8; 2]) -> Layers {
        Layers { lo, hi }
    }
}

impl From<Layers> for [u8; 2] {
    fn from(l: Layers) -> [u8; 2] {
        [l.lo, l.hi]
    }
}

impl Layers {
    /// The top layer.
    pub const TOP: u8 = 2;

    /// What a part takes unless it says: a module on headers 2-2, a low part (lying flat) 1-1,
    /// any other part 1-2.
    pub fn of(module: bool, low: bool) -> Layers {
        match (module, low) {
            (true, _) => Layers { lo: 2, hi: 2 },
            (false, true) => Layers { lo: 1, hi: 1 },
            (false, false) => Layers { lo: 1, hi: 2 },
        }
    }

    /// These layers, or a stop naming `what` if they make no sense.
    pub fn checked(self, what: &str) -> Layers {
        if self.lo > self.hi || self.hi > Layers::TOP {
            panic!(
                "{what}: \"layers\" should be [from, to], with 0 <= from <= to <= {} (0 the board's surface, 1 where parts lie, 2 above them), not [{}, {}]",
                Layers::TOP, self.lo, self.hi
            );
        }
        self
    }

    /// What it takes where it stands: from the surface up.
    pub fn standing(self) -> Layers {
        Layers { lo: 0, hi: self.hi }
    }

    pub fn overlaps(self, other: Layers) -> bool {
        self.lo <= other.hi && other.lo <= self.hi
    }

    pub fn takes(self, layer: u8) -> bool {
        (self.lo..=self.hi).contains(&layer)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Net {
    pub name: String,
    pub pins: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct BoardSize {
    pub width: i32,
    pub height: i32,
}

/// What you asked for: the board, the parts and how they connect, and your rules for the board as
/// written in the board file, for one case of them (`constraints.choices` says which). The placer,
/// the router and the score all work on one; with a `Layout` it makes a `Build`.
///
/// `Design::new` makes one from a KiCad netlist, a DIYLC sketch or a layout file, without rules; `constraints::
/// designs` turns it into one per case of the board file's rules. The fields after `rules` are
/// worked out from the ones before.
#[derive(Clone)]
pub struct Design {
    pub board: BoardSize,
    pub parts: Vec<Part>,
    /// Each joins two pins or more: a net with one pin has nothing to join, so the netlist and
    /// sketch readers leave it out (and a layout file with one is refused).
    pub nets: Vec<Net>,
    /// The supply and ground nets, if known (without them the power rule is off).
    pub power: Option<String>,
    pub ground: Option<String>,
    /// The board file's rules as written: all of them, every case's.
    pub rules: Vec<crate::constraints::Rule>,
    /// For each net, its pins as (component index, pin index).
    pub net_pins: Vec<Vec<(usize, usize)>>,
    /// This case's rules, matched up with the parts (where parts go, keep-outs, near, star).
    pub constraints: crate::constraints::Constraints,
    /// What the router joins: one run per net, or one per branch of a star net.
    pub runs: Vec<Run>,
}

/// Pins to join with one piece of copper. Two runs never share copper, even on the same net,
/// unless a soft star joins them (`Routing::joins`).
#[derive(Clone)]
pub struct Run {
    pub net: usize,
    /// (component index, pin index); for a star run the star point comes first.
    pub pins: Vec<(usize, usize)>,
    /// The star point, if this run is a branch of a star net.
    pub star: Option<(usize, usize)>,
}

impl Design {
    /// A design without rules yet: `constraints::designs` adds them, one design per case.
    pub fn new(board: BoardSize, parts: Vec<Part>, nets: Vec<Net>, power: Option<String>, ground: Option<String>) -> Design {
        let mut c = Design { board, parts, nets, power, ground, rules: Vec::new(), net_pins: Vec::new(), constraints: Default::default(), runs: Vec::new() };
        c.net_pins = c.nets.iter().map(|net| net.pins.iter().map(|p| c.find_pin(p)).collect()).collect();
        if let Some(net) = c.nets.iter().find(|n| n.pins.len() < 2) {
            panic!("net {} joins fewer than two pins", net.name);
        }
        // Stop now, with a clear message, if the power or ground net doesn't exist.
        c.power.iter().chain(&c.ground).for_each(|name| {
            c.net_index(name);
        });
        c
    }

    /// The layers part `i` takes: the board file's `layers` rule for it, or its own.
    pub fn layers(&self, i: usize) -> Layers {
        self.constraints.layers.get(i).copied().flatten().unwrap_or(self.parts[i].layers)
    }

    /// Whether another part may sit inside part `i`'s outline: a module's board raised above the
    /// parts, or a part drawn at an angle, which only takes the line between its legs.
    pub fn room_under(&self, i: usize) -> bool {
        let comp = &self.parts[i];
        if comp.module { self.layers(i).lo >= Layers::TOP } else { comp.footprint.body.len() < (comp.footprint.w * comp.footprint.h) as usize }
    }

    /// (index of U1, index of its pin "8") -> "U1.8": `find_pin` the other way.
    pub fn pin_name(&self, (ci, pi): (usize, usize)) -> String {
        format!("{}.{}", self.parts[ci].reference, self.parts[ci].footprint.pins[pi].name)
    }

    /// "U1.8" -> (index of U1, index of its pin "8")
    pub fn find_pin(&self, label: &str) -> (usize, usize) {
        let (reference, pin) = label.split_once('.').unwrap_or_else(|| panic!("pin {label} should look like U1.8"));
        let ci = self.parts.iter().position(|c| c.reference == reference);
        let ci = ci.unwrap_or_else(|| panic!("a net uses {reference}, which isn't in the component list"));
        let pi = self.parts[ci].footprint.pins.iter().position(|p| p.name == pin);
        (ci, pi.unwrap_or_else(|| panic!("{label}: its footprint has no pad {pin}")))
    }

    /// "5V" -> the index of the net called that.
    pub fn net_index(&self, name: &str) -> usize {
        self.nets.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("no net called {name}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub rot: u8,
}

pub type Placement = Vec<Place>;

/// Every part dropped at a random place (parts with placement rules on the nearest place they
/// allow). Parts may land on each other: `place::random_start` moves them apart.
pub fn random_placement(c: &Design, rng: &mut impl Rng) -> Placement {
    let (w, h) = (c.board.width, c.board.height);
    let mut placement: Placement = c
        .parts
        .iter()
        .map(|_| Place { x: rng.random_range(0..w - 1), y: rng.random_range(0..h - 1), rot: rng.random_range(0..4) })
        .collect();
    crate::constraints::apply(c, &mut placement);
    placement
}

/// Width and height of a component after `rot` quarter turns.
pub fn size(comp: &Part, rot: u8) -> (i32, i32) {
    if rot % 2 == 0 { (comp.footprint.w, comp.footprint.h) } else { (comp.footprint.h, comp.footprint.w) }
}

/// Turns a hole inside a w x h outline `rot` quarter turns clockwise (the outline's top-left stays at 0, 0).
pub fn turn((x, y): Point, w: i32, h: i32, rot: u8) -> Point {
    let (mut x, mut y, mut w, mut h) = (x, y, w, h);
    for _ in 0..rot % 4 {
        (x, y, w, h) = (h - 1 - y, x, h, w);
    }
    (x, y)
}

/// Where everything physically is on the board.
pub struct Geometry {
    /// Holes taken by each component's body.
    pub bodies: Vec<Vec<Point>>,
    /// Every hole inside each component's outline (the same as its body, except for modules).
    pub outlines: Vec<Vec<Point>>,
    /// Absolute position of each pin: pins[component][pin].
    pub pins: Vec<Vec<Point>>,
}

pub fn geometry(c: &Design, placement: &[Place]) -> Geometry {
    let mut g = Geometry { bodies: Vec::new(), outlines: Vec::new(), pins: Vec::new() };
    for (comp, p) in c.parts.iter().zip(placement) {
        let at = |hole: Point| {
            let (dx, dy) = turn(hole, comp.footprint.w, comp.footprint.h, p.rot);
            (p.x + dx, p.y + dy)
        };
        g.bodies.push(comp.footprint.body.iter().map(|&b| at(b)).collect());
        g.outlines.push(Footprint::full_body(comp.footprint.w, comp.footprint.h).into_iter().map(at).collect());
        g.pins.push(comp.footprint.pins.iter().map(|pin| at(pin.at)).collect());
    }
    g
}

/// Distance between two holes, in holes, straight across the board.
pub fn distance(a: Point, b: Point) -> f64 {
    (((a.0 - b.0).pow(2) + (a.1 - b.1).pow(2)) as f64).sqrt()
}

/// The corners of the smallest rectangle around some holes: (left, top), (right, bottom).
pub fn bounds(holes: &[Point]) -> (Point, Point) {
    let (xs, ys) = (holes.iter().map(|p| p.0), holes.iter().map(|p| p.1));
    ((xs.clone().min().unwrap(), ys.clone().min().unwrap()), (xs.max().unwrap(), ys.max().unwrap()))
}

const NET_COLORS: [&str; 14] = [
    "#1f77b4", "#2ca02c", "#17becf", "#8c564b", "#bcbd22", "#393b79", "#637939",
    "#8c6d31", "#843c39", "#7b4173", "#a6761d", "#1b9e77", "#756bb1", "#e6550d",
];

/// A colour for each net: power red, ground near-black, the rest from NET_COLORS.
/// The SVG picture and the DIYLC jumpers both use these, so a net looks the same in both.
pub fn net_colors(c: &Design) -> Vec<&'static str> {
    let mut others = NET_COLORS.iter().cycle();
    c.nets
        .iter()
        .map(|n| if c.power.as_ref() == Some(&n.name) { "#d62728" } else if c.ground.as_ref() == Some(&n.name) { "#333333" } else { others.next().unwrap() })
        .collect()
}
