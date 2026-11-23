//! The circuit: what is to be built, the parts and the nets that must connect them. It says what
//! should be connected, not how. Readers (`netlist`, `diylc`, the layout file) each return one.

use crate::board::{Layers, Point};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Net names tried, in order, when the power or ground net isn't given.
const POWER_NAMES: [&str; 6] = ["+5V", "5V", "VCC", "+3V3", "+3.3V", "VDD"];
const GROUND_NAMES: [&str; 3] = ["GND", "VSS", "0"];
/// Pin names that say what a net is, best first; used to name nets read from DIYLC.
pub const NET_NAME_HINTS: [&str; 9] = ["+5V", "5V", "+3V3", "3V3", "+3.3V", "VCC", "VDD", "GND", "VSS"];

/// What is to be built: the parts, and the nets that must connect them.
#[derive(Clone)]
pub struct Circuit {
    pub parts: Vec<Part>,
    /// Each joins two pins or more: a net with one pin has nothing to join, so the netlist and
    /// sketch readers leave it out (and a layout file with one is refused).
    pub nets: Vec<Net>,
    /// The supply and ground nets, if known (without them the power rule is off).
    pub power: Option<String>,
    pub ground: Option<String>,
    /// For each net, its pins as (part index, pin index): worked out from the ones above.
    pub net_pins: Vec<Vec<(usize, usize)>>,
}

impl Circuit {
    /// A circuit of these parts and nets, its power and ground not known yet (see
    /// `with_power_and_ground`). Stops if a net names a pin that isn't there, or joins fewer than two.
    pub fn new(parts: Vec<Part>, nets: Vec<Net>) -> Circuit {
        let mut c = Circuit { parts, nets, power: None, ground: None, net_pins: Vec::new() };
        c.net_pins = c.nets.iter().map(|net| net.pins.iter().map(|p| c.find_pin(p)).collect()).collect();
        if let Some(net) = c.nets.iter().find(|n| n.pins.len() < 2) {
            panic!("net {} joins fewer than two pins", net.name);
        }
        c
    }

    /// The same circuit, with these power and ground nets. Stops now, with a clear message, if
    /// either doesn't exist.
    pub fn with_power_and_ground(mut self, power: Option<String>, ground: Option<String>) -> Circuit {
        (self.power, self.ground) = (power, ground);
        self.power.iter().chain(&self.ground).for_each(|name| {
            self.net_index(name);
        });
        self
    }

    /// The power and ground nets: as given, or guessed from the nets' names. None, with a note,
    /// for either that can't be told.
    pub fn power_and_ground(&self, power: Option<String>, ground: Option<String>) -> (Option<String>, Option<String>) {
        let guess = |names: &[&str]| names.iter().find_map(|name| self.nets.iter().find(|n| n.name.eq_ignore_ascii_case(name))).map(|n| n.name.clone());
        let power = power.or_else(|| guess(&POWER_NAMES));
        let ground = ground.or_else(|| guess(&GROUND_NAMES));
        for (what, found) in [("power", &power), ("ground", &ground)] {
            if found.is_none() {
                eprintln!("note: can't tell which net is {what}, so the power rule is off; pass --{what} <net name>");
            }
        }
        (power, ground)
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

/// What goes through a hole and is soldered there: one of a part's connections, named by its
/// footprint ("1", "GND"; "U1.8" with its part's reference).
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
    /// The holes its body covers, inside the outline, at its part's layers (`Part::layers`): the
    /// whole outline for most footprints (a board on headers too), the line between its legs for a
    /// two-legged part drawn at an angle, its members' bodies for a DIYLC group that isn't on headers.
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

    /// The holes a part with these layers stands on, in body order: its body, or only its pins if
    /// it's raised (a board on headers stands on its headers, with room under the rest).
    pub fn stands(&self, layers: Layers) -> Vec<Point> {
        self.body.iter().copied().filter(|h| !layers.raised() || self.pins.iter().any(|p| p.at == *h)).collect()
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
    /// Which layers it takes, unless the board file says otherwise (`Case::layers`).
    pub layers: Layers,
    /// The footprints.json its footprint came from, if not the built-in library (yours, or one next
    /// to the netlist), so a layout can say it depends on that file.
    pub footprint_from: Option<String>,
}

/// A set of pins that must end up electrically connected, by name ("U1.8", "R1.2"). It says
/// nothing about how.
#[derive(Clone, Serialize, Deserialize)]
pub struct Net {
    pub name: String,
    pub pins: Vec<String>,
}

