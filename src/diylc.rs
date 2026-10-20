//! DIY Layout Creator (.diy) files: reading a sketch (`read`) and writing a layout (`write`).
//!
//! Both sides share one description of DIYLC: the part classes this program knows (`CLASSES`),
//! its units and turns, and what a part keeps about DIYLC, the `Part` to draw it as (from
//! footprints.json) and the `Source`, your own drawing of it (from a sketch). To support another
//! DIYLC part, add it to `CLASSES` (the `perfboard-diylc-part` skill says how to prove it).

pub mod read;
pub mod write;

pub use read::{Copper, load};
pub use write::write;

use crate::board::Point;
use serde::{Deserialize, Serialize};

/// DIYLC measures in inches; one perfboard hole is 0.1".
pub const HOLE: f64 = 0.1;
/// DIYLC puts a perfboard's first hole one grid step in from the board's corner.
pub const MARGIN: i32 = 1;
/// DIYLC turns parts clockwise in 90 degree steps, like our placer does: its name for each turn.
pub const ORIENTATIONS: [&str; 4] = ["DEFAULT", "_90", "_180", "_270"];

/// What a DIYLC item is, as far as connections go.
#[derive(Clone, Copy)]
pub enum Kind {
    /// A part with pins: DIYLC's names for its pins, in the order of its points (None = number them
    /// 1, 2, ..., or take a header's own labels).
    Part(Option<&'static [&'static str]>),
    /// A copper trace: joins everything it touches along its length.
    Trace,
    /// A jumper or wire: joins whatever is at its two ends.
    Wire,
    /// A solder pad: joins whatever is on it.
    Pad,
    /// Text and shapes: no connections, but kept when inside a group.
    Decoration,
    Board,
}

/// How the writer draws a new part of a class: one from footprints.json, not from a sketch (a
/// sketch's own drawing is written back as it was).
#[derive(Clone, Copy)]
pub enum Drawing {
    /// Two legs, like a resistor: its body's size, its body, border and label colours, and its
    /// other settings (SPACING stands for the distance between its legs), as DIYLC 6.6 writes a
    /// new one.
    TwoLegged { size: &'static str, colors: [&'static str; 3], extra: &'static str },
    /// A DIP chip: two rows of pins.
    Dil,
    /// A pin header: a grid of pins.
    PinHeader,
}

/// A DIYLC part class this program knows.
pub struct Class {
    /// DIYLC's name for it: the item's XML tag without "diylc.", like "passive.Resistor".
    pub name: &'static str,
    /// Its short name: what a footprints.json "diylc" entry asks for, what a part read from a sketch
    /// is called ("DIYLC:Resistor"), and what the pictures draw it as.
    pub short: &'static str,
    pub kind: Kind,
    /// Low enough to sit under a module on headers: it lies flat, or it's a small ceramic disc.
    pub low: bool,
    /// It stands up, so its body is round around its legs; an axial part lies between its legs.
    pub radial: bool,
    /// How to draw a new one, for the classes footprints.json can ask for.
    pub drawing: Option<Drawing>,
}

const TWO: Option<&[&str]> = Some(&["1", "2"]);

/// Every DIYLC class this program reads (and, with a `drawing`, writes). A sketch with any other
/// part is refused, since its connections would be missed.
pub const CLASSES: &[Class] = &[
    Class {
        name: "passive.Resistor",
        short: "Resistor",
        kind: Kind::Part(TWO),
        low: true,
        radial: false,
        drawing: Some(Drawing::TwoLegged {
            size: r#"<length value="6.3" unit="mm"/><width value="2.5" unit="mm"/>"#,
            colors: ["82cffd", "5b90b1", "000000"],
            extra: "<power>HALF</power><colorCode>_5_BAND</colorCode><shape>Standard</shape>",
        }),
    },
    Class {
        name: "passive.RadialCeramicDiskCapacitor",
        short: "RadialCeramicDiskCapacitor",
        kind: Kind::Part(TWO),
        low: true,
        radial: true,
        drawing: Some(Drawing::TwoLegged {
            size: r#"<length value="5.0" unit="mm"/><width value="2.5" unit="mm"/>"#,
            colors: ["f0e68c", "a8a162", "000000"],
            extra: r#"<pinSpacing value="SPACING" unit="in"/><voltage>_63V</voltage>"#,
        }),
    },
    Class { name: "passive.RadialFilmCapacitor", short: "RadialFilmCapacitor", kind: Kind::Part(TWO), low: false, radial: true, drawing: None },
    Class { name: "passive.AxialFilmCapacitor", short: "AxialFilmCapacitor", kind: Kind::Part(TWO), low: true, radial: false, drawing: None },
    Class {
        name: "passive.RadialElectrolytic",
        short: "RadialElectrolytic",
        kind: Kind::Part(Some(&["+", "-"])),
        low: false,
        radial: true,
        drawing: Some(Drawing::TwoLegged {
            size: r#"<length value="5.0" unit="mm"/>"#,
            colors: ["6b6dce", "4a4c90", "ffffff"],
            extra: r#"<pinSpacing value="SPACING" unit="in"/><voltage>_63V</voltage><markerColor hex="8cacea"/><tickColor hex="ffffff"/><polarized>true</polarized><folded>false</folded><height value="0.4" unit="in"/><invert>false</invert>"#,
        }),
    },
    Class {
        name: "semiconductors.LED",
        short: "LED",
        kind: Kind::Part(Some(&["A", "K"])),
        low: false,
        radial: true,
        drawing: Some(Drawing::TwoLegged {
            size: r#"<length value="5.0" unit="mm"/><width value="5.0" unit="mm"/>"#,
            colors: ["5dfc0a", "41b007", "000000"],
            extra: "<value></value>",
        }),
    },
    Class { name: "semiconductors.DiodePlastic", short: "DiodePlastic", kind: Kind::Part(Some(&["A", "K"])), low: true, radial: false, drawing: None },
    // DIYLC's class has two underscores; footprints.json has always called it DIL_IC.
    Class { name: "semiconductors.DIL__IC", short: "DIL_IC", kind: Kind::Part(None), low: false, radial: false, drawing: Some(Drawing::Dil) },
    Class { name: "semiconductors.TransistorTO92", short: "TransistorTO92", kind: Kind::Part(None), low: false, radial: false, drawing: None },
    Class { name: "semiconductors.TransistorTO126", short: "TransistorTO126", kind: Kind::Part(None), low: false, radial: false, drawing: None },
    Class { name: "semiconductors.TransistorTO220", short: "TransistorTO220", kind: Kind::Part(None), low: false, radial: false, drawing: None },
    Class { name: "electromechanical.PinHeader", short: "PinHeader", kind: Kind::Part(None), low: false, radial: false, drawing: Some(Drawing::PinHeader) },
    Class { name: "electromechanical.TactileMicroSwitch", short: "TactileMicroSwitch", kind: Kind::Part(None), low: false, radial: false, drawing: None },
    Class { name: "connectivity.CopperTrace", short: "CopperTrace", kind: Kind::Trace, low: false, radial: false, drawing: None },
    Class { name: "connectivity.Jumper", short: "Jumper", kind: Kind::Wire, low: false, radial: false, drawing: None },
    Class { name: "connectivity.HookupWire", short: "HookupWire", kind: Kind::Wire, low: false, radial: false, drawing: None },
    Class { name: "connectivity.OrthogonalWire", short: "OrthogonalWire", kind: Kind::Wire, low: false, radial: false, drawing: None },
    Class { name: "connectivity.TwistedWire", short: "TwistedWire", kind: Kind::Wire, low: false, radial: false, drawing: None },
    Class { name: "connectivity.SolderPad", short: "SolderPad", kind: Kind::Pad, low: false, radial: false, drawing: None },
    Class { name: "misc.Label", short: "Label", kind: Kind::Decoration, low: false, radial: false, drawing: None },
    Class { name: "misc.WrapLabel", short: "WrapLabel", kind: Kind::Decoration, low: false, radial: false, drawing: None },
    Class { name: "misc.PCBText", short: "PCBText", kind: Kind::Decoration, low: false, radial: false, drawing: None },
    Class { name: "boards.PerfBoard", short: "PerfBoard", kind: Kind::Board, low: false, radial: false, drawing: None },
];

/// Every DIYLC shape (rectangles, ellipses...) and chassis drawing: no connections.
const SHAPE: Class = Class { name: "shapes", short: "shape", kind: Kind::Decoration, low: false, radial: false, drawing: None };

/// The class of a DIYLC item by DIYLC's name for it ("passive.Resistor"), or None for one this
/// program doesn't know.
pub fn class(name: &str) -> Option<&'static Class> {
    if name.starts_with("shapes.") || name.starts_with("chassis.") {
        return Some(&SHAPE);
    }
    CLASSES.iter().find(|c| c.name == name)
}

/// The class a footprints.json "diylc" entry asks for, by its short name ("Resistor").
pub fn class_named(short: &str) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.short == short)
}

/// The "diylc" entry of a footprint in footprints.json (the layout file keeps it too).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// The short name of a class in `CLASSES` that has a `drawing`.
    pub part: String,
    /// For two-legged parts: which pad becomes DIYLC's first leg and which its second (default "1", "2").
    /// It matters for polarised parts: DIYLC's first leg is + on a capacitor, but the anode on an LED.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<String>,
}

/// The original drawing of a part, kept so it can be written back to DIYLC moved and turned
/// (the layout file keeps it too, as the part's "diylc_drawing").
#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    /// The XML of each DIYLC item the part is made of, exactly as in the file.
    pub xml: Vec<String>,
    /// The DIYLC group it was, if any: (group name, ids of its items).
    pub group: Option<(String, Vec<String>)>,
    /// The sketch hole that is the part's top-left corner.
    pub origin: Point,
}

/// Text made safe to put inside XML.
pub fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
