//! DIY Layout Creator (.diy) files: reading a sketch (`read`) and writing a layout (`write`).
//!
//! Both sides share one description of DIYLC: the part classes this program knows (`CLASSES`),
//! its units and turns, and DIYLC's drawings of footprints and parts (`board::Drawing`, keyed
//! `Format::Diylc`), which only this module opens: a footprint's says which class to draw it as
//! (footprints.json's "diylc" entry, `FootprintDrawing`), a part's is your own drawing of it (from
//! a sketch, `PartDrawing`). It also says what each class looks like in the pictures (`look`). To
//! support another DIYLC part, add it to `CLASSES` (the `perfboard-diylc-part` skill says how to
//! prove it).

pub mod read;
pub mod write;

pub use read::load;
pub use write::write;

use crate::board::{Drawing, Footprint, Format, Look, Pin, Point};
use serde::de::DeserializeOwned;
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
pub enum Template {
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
    /// is called ("DIYLC:Resistor"), and what `class_look` turns into its look in the pictures.
    pub short: &'static str,
    pub kind: Kind,
    /// Low enough to sit under a module on headers: it lies flat, or it's a small ceramic disc.
    pub low: bool,
    /// It stands up, so its body is round around its legs; an axial part lies between its legs.
    pub radial: bool,
    /// How to draw a new one, for the classes footprints.json can ask for.
    pub template: Option<Template>,
}

const TWO: Option<&[&str]> = Some(&["1", "2"]);

/// Every DIYLC class this program reads (and, with a `template`, writes). A sketch with any other
/// part is refused, since its connections would be missed.
pub const CLASSES: &[Class] = &[
    Class {
        name: "passive.Resistor",
        short: "Resistor",
        kind: Kind::Part(TWO),
        low: true,
        radial: false,
        template: Some(Template::TwoLegged {
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
        template: Some(Template::TwoLegged {
            size: r#"<length value="5.0" unit="mm"/><width value="2.5" unit="mm"/>"#,
            colors: ["f0e68c", "a8a162", "000000"],
            extra: r#"<pinSpacing value="SPACING" unit="in"/><voltage>_63V</voltage>"#,
        }),
    },
    Class { name: "passive.RadialFilmCapacitor", short: "RadialFilmCapacitor", kind: Kind::Part(TWO), low: false, radial: true, template: None },
    Class { name: "passive.AxialFilmCapacitor", short: "AxialFilmCapacitor", kind: Kind::Part(TWO), low: true, radial: false, template: None },
    Class {
        name: "passive.RadialElectrolytic",
        short: "RadialElectrolytic",
        kind: Kind::Part(Some(&["+", "-"])),
        low: false,
        radial: true,
        template: Some(Template::TwoLegged {
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
        template: Some(Template::TwoLegged {
            size: r#"<length value="5.0" unit="mm"/><width value="5.0" unit="mm"/>"#,
            colors: ["5dfc0a", "41b007", "000000"],
            extra: "<value></value>",
        }),
    },
    Class { name: "semiconductors.DiodePlastic", short: "DiodePlastic", kind: Kind::Part(Some(&["A", "K"])), low: true, radial: false, template: None },
    // DIYLC's class has two underscores; footprints.json has always called it DIL_IC.
    Class { name: "semiconductors.DIL__IC", short: "DIL_IC", kind: Kind::Part(None), low: false, radial: false, template: Some(Template::Dil) },
    Class { name: "semiconductors.TransistorTO92", short: "TransistorTO92", kind: Kind::Part(None), low: false, radial: false, template: None },
    Class { name: "semiconductors.TransistorTO126", short: "TransistorTO126", kind: Kind::Part(None), low: false, radial: false, template: None },
    Class { name: "semiconductors.TransistorTO220", short: "TransistorTO220", kind: Kind::Part(None), low: false, radial: false, template: None },
    Class { name: "electromechanical.PinHeader", short: "PinHeader", kind: Kind::Part(None), low: false, radial: false, template: Some(Template::PinHeader) },
    Class { name: "electromechanical.TactileMicroSwitch", short: "TactileMicroSwitch", kind: Kind::Part(None), low: false, radial: false, template: None },
    Class { name: "connectivity.CopperTrace", short: "CopperTrace", kind: Kind::Trace, low: false, radial: false, template: None },
    Class { name: "connectivity.Jumper", short: "Jumper", kind: Kind::Wire, low: false, radial: false, template: None },
    Class { name: "connectivity.HookupWire", short: "HookupWire", kind: Kind::Wire, low: false, radial: false, template: None },
    Class { name: "connectivity.OrthogonalWire", short: "OrthogonalWire", kind: Kind::Wire, low: false, radial: false, template: None },
    Class { name: "connectivity.TwistedWire", short: "TwistedWire", kind: Kind::Wire, low: false, radial: false, template: None },
    Class { name: "connectivity.SolderPad", short: "SolderPad", kind: Kind::Pad, low: false, radial: false, template: None },
    Class { name: "misc.Label", short: "Label", kind: Kind::Decoration, low: false, radial: false, template: None },
    Class { name: "misc.WrapLabel", short: "WrapLabel", kind: Kind::Decoration, low: false, radial: false, template: None },
    Class { name: "misc.PCBText", short: "PCBText", kind: Kind::Decoration, low: false, radial: false, template: None },
    Class { name: "boards.PerfBoard", short: "PerfBoard", kind: Kind::Board, low: false, radial: false, template: None },
];

/// Every DIYLC shape (rectangles, ellipses...) and chassis drawing: no connections.
const SHAPE: Class = Class { name: "shapes", short: "shape", kind: Kind::Decoration, low: false, radial: false, template: None };

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

/// DIYLC's drawing of a footprint: the "diylc" entry of a footprint in footprints.json (the layout
/// file keeps it too).
#[derive(Deserialize)]
struct FootprintDrawing {
    /// The short name of a class in `CLASSES` that has a `template`.
    part: String,
    /// For two-legged parts: which pad becomes DIYLC's first leg and which its second (default "1", "2").
    /// It matters for polarised parts: DIYLC's first leg is + on a capacitor, but the anode on an LED.
    #[serde(default)]
    legs: Vec<String>,
}

/// DIYLC's drawing of a part read from a sketch: your original drawing of it, kept so it can be
/// written back to DIYLC moved and turned (the layout file keeps it too).
#[derive(Serialize, Deserialize)]
struct PartDrawing {
    /// The XML of each DIYLC item the part is made of, exactly as in the file.
    xml: Vec<String>,
    /// The DIYLC group it was, if any: (group name, ids of its items).
    group: Option<(String, Vec<String>)>,
    /// The sketch hole that is the part's top-left corner.
    origin: Point,
}

/// A DIYLC drawing, opened: `whose` names it in the message if it isn't the shape DIYLC's
/// drawings have (a footprints.json "diylc" entry written wrong, say).
fn open<T: DeserializeOwned>(drawing: &Drawing, whose: &str) -> T {
    serde_json::from_value(drawing.0.clone()).unwrap_or_else(|e| panic!("{whose}: its \"diylc\" drawing isn't right: {e}"))
}

/// A DIYLC drawing, sealed again, to keep with a part or footprint.
fn seal(drawing: &impl Serialize) -> Drawing {
    Drawing(serde_json::to_value(drawing).expect("a DIYLC drawing turns into JSON"))
}

/// What a footprint with a DIYLC drawing looks like in the pictures, from the class it asks for;
/// None if it has none.
pub fn look(footprint: &Footprint) -> Option<Look> {
    let drawing: FootprintDrawing = open(footprint.drawings.get(&Format::Diylc)?, &footprint.name);
    Some(class_look(&drawing.part, &footprint.pins, drawing.legs.first().map(String::as_str)))
}

/// What a part of the DIYLC class called `short` looks like, with these pins: `first_leg` is the
/// pad a footprints.json entry makes DIYLC's first leg, the + of a polarised part.
fn class_look(short: &str, pins: &[Pin], first_leg: Option<&str>) -> Look {
    match short {
        "Resistor" | "AxialFilmCapacitor" => Look::Lying { band: false },
        "DiodePlastic" => Look::Lying { band: true },
        "RadialElectrolytic" | "LED" => {
            // The leg marked +: the capacitor's +, the LED's anode, or DIYLC's first leg.
            let is_plus = |n: &str| n == "+" || n == "A" || Some(n) == first_leg;
            let plus = pins.iter().find(|p| is_plus(&p.name)).or(pins.first());
            Look::Round { plus: plus.map(|p| p.name.clone()).unwrap_or_default(), lit: short == "LED" }
        }
        "RadialCeramicDiskCapacitor" | "RadialFilmCapacitor" => Look::Disc,
        "PinHeader" => Look::Header,
        _ => Look::Box,
    }
}

/// Text made safe to put inside XML.
pub fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
