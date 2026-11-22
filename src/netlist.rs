//! Turns a KiCad netlist (.net) plus our footprint library into a `Design`.
//!
//! The netlist says which pin connects to which net and names each part's
//! footprint. It says nothing about where the pins physically are, so that
//! comes from the footprint library (`footprints`).

use crate::board::{BoardSize, Design, Drawings, Net, Part};
use crate::footprints::{self, Library};
use crate::sexpr::{self, Sexpr};

/// Net names tried, in order, when `--power` / `--ground` aren't given.
const POWER_NAMES: [&str; 6] = ["+5V", "5V", "VCC", "+3V3", "+3.3V", "VDD"];
const GROUND_NAMES: [&str; 3] = ["GND", "VSS", "0"];
/// Pin names that say what a net is, best first; used to name nets read from DIYLC.
pub const NET_NAME_HINTS: [&str; 9] = ["+5V", "5V", "+3V3", "3V3", "+3.3V", "VCC", "VDD", "GND", "VSS"];

pub struct Options {
    pub board: BoardSize,
    pub power: Option<String>,
    pub ground: Option<String>,
    /// Read your own footprints.json (`footprints::user_footprints`)? Not for the benchmark, which must place
    /// the same circuits the same way on every machine.
    pub user_footprints: bool,
}

pub fn load(path: &str, opts: Options) -> Design {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let root = sexpr::parse(&text).unwrap_or_else(|e| panic!("bad netlist {path}: {e}"));
    if root.head() != Some("export") {
        panic!("{path} doesn't look like a KiCad netlist (it should start with \"(export\")");
    }
    let library = footprints::library(path, opts.user_footprints);
    let parts: Vec<Part> =
        root.child("components").into_iter().flat_map(|cs| cs.children("comp")).map(|comp| part(comp, &library)).collect();
    library.note_replaced(parts.iter().map(|c| c.footprint.name.as_str()));
    // KiCad also lists pins that connect to nothing as one-pin nets; there's nothing to wire for those.
    let nets: Vec<Net> =
        root.child("nets").into_iter().flat_map(|ns| ns.children("net")).map(net).filter(|n| n.pins.len() > 1).collect();

    let (power, ground) = power_and_ground(&nets, &opts);
    Design::new(opts.board, parts, nets, power, ground)
}

fn part(comp: &Sexpr, library: &Library) -> Part {
    let reference = comp.value("ref").expect("a (comp) without a (ref)").to_string();
    let footprint = comp.value("footprint").unwrap_or_else(|| panic!("{reference} has no footprint; set one in KiCad"));
    let found = library.find(&reference, footprint);
    let value = comp.value("value").unwrap_or("").to_string();
    Part { reference, value, footprint: found.footprint, drawings: Drawings::new(), layers: found.layers, module: found.module, footprint_from: found.from }
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
