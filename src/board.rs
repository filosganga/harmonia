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

/// The perfboard itself: its size in holes.
#[derive(Clone, Copy)]
pub struct Board {
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
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
    /// Holes taken by each component's body.
    pub bodies: Vec<Vec<Point>>,
    /// Every hole inside each component's outline (the same as its body, except for modules).
    pub outlines: Vec<Vec<Point>>,
    /// Absolute position of each pin: pins[component][pin].
    pub pins: Vec<Vec<Point>>,
}

pub fn geometry(c: &Circuit, placement: &[Place]) -> Geometry {
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
pub fn net_colors(c: &Circuit) -> Vec<&'static str> {
    let mut others = NET_COLORS.iter().cycle();
    c.nets
        .iter()
        .map(|n| if c.power.as_ref() == Some(&n.name) { "#d62728" } else if c.ground.as_ref() == Some(&n.name) { "#333333" } else { others.next().unwrap() })
        .collect()
}
