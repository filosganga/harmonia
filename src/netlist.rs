//! Turns a KiCad netlist (.net), or any list of parts and nets, plus our footprint library into a
//! `Circuit`.
//!
//! The netlist says which pin connects to which net and names each part's
//! footprint. It says nothing about where the pins physically are, so that
//! comes from `footprints.json`: footprint name -> size and pad positions in holes.

use crate::board::{BoardSize, Circuit, Component, Net, Pin, Point};
use crate::diylc;
use crate::sexpr::{self, Sexpr};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

pub const FOOTPRINTS: &str = "footprints.json";

/// Net names tried, in order, when `--power` / `--ground` aren't given.
const POWER_NAMES: [&str; 6] = ["+5V", "5V", "VCC", "+3V3", "+3.3V", "VDD"];
const GROUND_NAMES: [&str; 3] = ["GND", "VSS", "0"];
/// Pin names that say what a net is, best first; used to name nets read from DIYLC.
pub const NET_NAME_HINTS: [&str; 9] = ["+5V", "5V", "+3V3", "3V3", "+3.3V", "VCC", "VDD", "GND", "VSS"];

/// One entry of footprints.json. Other keys (like "about") are just notes and are ignored.
#[derive(Deserialize)]
struct Footprint {
    w: i32,
    h: i32,
    /// Pad number -> hole, counted from the footprint's top-left hole.
    pins: BTreeMap<String, Point>,
    diylc: Option<diylc::Part>,
    /// Low enough to sit under a module on headers. Leave out if it isn't.
    #[serde(default)]
    low: bool,
}

pub struct Options {
    pub board: BoardSize,
    pub power: Option<String>,
    pub ground: Option<String>,
}

pub fn load(path: &str, opts: Options) -> Circuit {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let root = sexpr::parse(&text).unwrap_or_else(|e| panic!("bad netlist {path}: {e}"));
    if root.head() != Some("export") {
        panic!("{path} doesn't look like a KiCad netlist (it should start with \"(export\")");
    }
    let parts: Vec<Part> = root.child("components").into_iter().flat_map(|cs| cs.children("comp")).map(part).collect();
    let nets: Vec<Net> = root.child("nets").into_iter().flat_map(|ns| ns.children("net")).map(net).collect();
    from_parts(&parts, nets, opts)
}

/// A part as a netlist names it: its reference (R1), value (10k) and footprint.
#[derive(Clone, serde::Serialize, Deserialize)]
pub struct Part {
    pub reference: String,
    pub value: String,
    pub footprint: String,
}

/// The circuit made of these parts, their footprints looked up in footprints.json, and these nets.
/// One-pin nets are dropped (KiCad lists pins that connect to nothing that way): there's nothing to wire.
pub fn from_parts(parts: &[Part], nets: Vec<Net>, opts: Options) -> Circuit {
    let library = library();
    let components: Vec<Component> = parts.iter().map(|p| component(p, &library)).collect();
    let nets: Vec<Net> = nets.into_iter().filter(|n| n.pins.len() > 1).collect();
    let (power, ground) = power_and_ground(&nets, &opts);
    Circuit::new(opts.board, components, nets, power, ground)
}

fn library() -> HashMap<String, Footprint> {
    let lib_text = std::fs::read_to_string(FOOTPRINTS).unwrap_or_else(|e| panic!("can't read {FOOTPRINTS}: {e}"));
    serde_json::from_str(&lib_text).unwrap_or_else(|e| panic!("bad {FOOTPRINTS}: {e}"))
}

/// Each footprint's outline in holes (w, h), as footprints.json gives it.
pub fn sizes() -> HashMap<String, (i32, i32)> {
    library().into_iter().map(|(name, fp)| (name, (fp.w, fp.h))).collect()
}

fn part(comp: &Sexpr) -> Part {
    let reference = comp.value("ref").expect("a (comp) without a (ref)").to_string();
    let footprint = comp.value("footprint").unwrap_or_else(|| panic!("{reference} has no footprint; set one in KiCad")).to_string();
    let value = comp.value("value").unwrap_or("").to_string();
    Part { reference, value, footprint }
}

fn component(part: &Part, library: &HashMap<String, Footprint>) -> Component {
    let (reference, footprint) = (part.reference.clone(), &part.footprint);
    // "Package_DIP:DIP-8_W7.62mm" -> "DIP-8_W7.62mm": the library name before ':' doesn't matter here.
    let name = footprint.rsplit(':').next().unwrap();
    let fp = library.get(name).unwrap_or_else(|| {
        panic!("{reference} uses footprint {name}, which isn't in {FOOTPRINTS} yet: add its size and pads in holes")
    });
    let pins = fp.pins.iter().map(|(name, &at)| Pin { name: name.clone(), at, off_grid: false }).collect();
    let value = part.value.clone();
    let body = Component::full_body(fp.w, fp.h);
    Component { reference, value, footprint: footprint.to_string(), diylc: fp.diylc.clone(), w: fp.w, h: fp.h, pins, body, source: None, low: fp.low }
}

fn net(net: &Sexpr) -> Net {
    // Labels local to a sheet come out as "/DIS"; drop the sheet path for readability.
    let name = net.value("name").expect("a (net) without a (name)").trim_start_matches('/').to_string();
    let pins = net
        .children("node")
        .map(|node| {
            let part = node.value("ref").expect("a (node) without a (ref)");
            let pin = node.value("pin").expect("a (node) without a (pin)");
            format!("{part}.{pin}")
        })
        .collect();
    Net { name, pins }
}

/// The first net named like one of `names` (ignoring upper/lower case).
fn guess(nets: &[Net], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| nets.iter().find(|n| n.name.eq_ignore_ascii_case(name))).map(|n| n.name.clone())
}

/// The power and ground nets: as given, or guessed from their names. None if neither works.
pub fn power_and_ground(nets: &[Net], opts: &Options) -> (Option<String>, Option<String>) {
    let power = opts.power.clone().or_else(|| guess(nets, &POWER_NAMES));
    let ground = opts.ground.clone().or_else(|| guess(nets, &GROUND_NAMES));
    for (what, found) in [("power", &power), ("ground", &ground)] {
        if found.is_none() {
            eprintln!("note: can't tell which net is {what}, so the power rule is off; pass --{what} <net name>");
        }
    }
    (power, ground)
}
