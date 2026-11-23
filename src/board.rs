//! Perfboard model: the board, and where the parts are (placement geometry), plus each net's colour
//! for the outputs. What is built is in `circuit`; how a layout is judged in `measure`.
//!
//! Coordinates are perfboard holes: x grows to the right, y grows downwards.
//! A placement gives each component the position of its top-left hole and a
//! rotation (0..3, in 90 degree steps).

use crate::circuit::{Circuit, Footprint, Part};

pub type Point = (i32, i32);

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

/// A height from the board, counted outwards from a face: 0 the board itself, 1 bare copper on
/// the face, 2 parts lying on the board and jumper wires, 3 parts raised above those (a board on
/// headers); negative on the bottom face.
pub type Layer = i8;
/// Bare copper on a face: traces, and the pads a part soldered flat sits on.
pub const COPPER: Layer = 1;
/// Parts lying on the board, and jumper wires.
pub const LYING: Layer = 2;
/// Parts raised above those: a board on headers.
pub const RAISED: Layer = 3;

/// Which layers a part takes, `lo` to `hi`, both included, counted from the face it's on. Its body
/// covers its holes at these layers; each of its pins takes its hole from the copper underneath (on
/// the other face) up to `hi`. Two things clash where they share a hole and a layer: so a resistor
/// lying flat (2-2) fits under a board on headers (3-3), and a tall capacitor (2-3) doesn't.
/// Written [lo, hi].
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(from = "[Layer; 2]", into = "[Layer; 2]")]
pub struct Layers {
    pub lo: Layer,
    pub hi: Layer,
}

impl From<[Layer; 2]> for Layers {
    fn from([lo, hi]: [Layer; 2]) -> Layers {
        Layers { lo, hi }
    }
}

impl From<Layers> for [Layer; 2] {
    fn from(l: Layers) -> [Layer; 2] {
        [l.lo, l.hi]
    }
}

impl Layers {
    /// What a part takes unless it says: a board on headers 3-3, a low part (lying flat) 2-2, any
    /// other part 2-3.
    pub fn of(raised: bool, low: bool) -> Layers {
        match (raised, low) {
            (true, _) => Layers { lo: RAISED, hi: RAISED },
            (false, true) => Layers { lo: LYING, hi: LYING },
            (false, false) => Layers { lo: LYING, hi: RAISED },
        }
    }

    /// These layers, or a stop naming `what` if they make no sense.
    pub fn checked(self, what: &str) -> Layers {
        if self.lo < COPPER || self.lo > self.hi || self.hi > RAISED {
            panic!(
                "{what}: \"layers\" should be [from, to], with {COPPER} <= from <= to <= {RAISED} ({COPPER} the copper on the board, {LYING} where parts lie, {RAISED} above them), not [{}, {}]",
                self.lo, self.hi
            );
        }
        self
    }

    /// Raised above the parts lying on the board, like a board on headers: there's room under it.
    pub fn raised(self) -> bool {
        self.lo >= RAISED
    }

    /// What a pin of a part with these layers takes in its hole: from the copper underneath, on the
    /// other face, up to the part's top.
    pub fn pin(self) -> Layers {
        Layers { lo: -COPPER, hi: self.hi }
    }

    pub fn overlaps(self, other: Layers) -> bool {
        self.lo <= other.hi && other.lo <= self.hi
    }

    pub fn takes(self, layer: Layer) -> bool {
        (self.lo..=self.hi).contains(&layer)
    }
}

/// One of the board's two faces. Parts are always on the top one, for now; traces run on the bottom
/// one, and wires on either. `Top as usize` is 0 and `Bottom` 1, so per-face arrays (the router's
/// wires) are indexed by `face as usize`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Top = 0,
    Bottom = 1,
}

/// The perfboard itself: its size in holes.
#[derive(Clone, Copy)]
pub struct Board {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// The face it's on: always the top, for now.
    pub face: Face,
    pub x: i32,
    pub y: i32,
    pub rot: u8,
}

pub type Placement = Vec<Place>;

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
    /// The holes each part's body covers.
    pub bodies: Vec<Vec<Point>>,
    /// The holes each part stands on: its body, or only its pins if it's raised (`Footprint::stands`).
    pub stands: Vec<Vec<Point>>,
    /// Every hole inside each part's outline.
    pub outlines: Vec<Vec<Point>>,
    /// Absolute position of each pin: pins[component][pin].
    pub pins: Vec<Vec<Point>>,
}

/// Where everything is with the parts of `c` at `placement`, each with the `layers` it takes (see
/// `Case::geometry`).
pub fn geometry(c: &Circuit, layers: &[Layers], placement: &[Place]) -> Geometry {
    let mut g = Geometry { bodies: Vec::new(), stands: Vec::new(), outlines: Vec::new(), pins: Vec::new() };
    for ((comp, p), &l) in c.parts.iter().zip(placement).zip(layers) {
        let at = |hole: Point| {
            let (dx, dy) = turn(hole, comp.footprint.w, comp.footprint.h, p.rot);
            (p.x + dx, p.y + dy)
        };
        g.bodies.push(comp.footprint.body.iter().map(|&b| at(b)).collect());
        g.stands.push(comp.footprint.stands(l).into_iter().map(at).collect());
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
pub fn net_colors(c: &Circuit) -> Vec<&'static str> {
    let mut others = NET_COLORS.iter().cycle();
    c.nets
        .iter()
        .map(|n| if c.power.as_ref() == Some(&n.name) { "#d62728" } else if c.ground.as_ref() == Some(&n.name) { "#333333" } else { others.next().unwrap() })
        .collect()
}
