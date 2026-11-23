//! Turns a KiCad netlist (.net) plus our footprint library into a `Circuit`.
//!
//! The netlist says which pin connects to which net and names each part's
//! footprint. It says nothing about where the pins physically are, so that
//! comes from the footprint library (`footprints`).

use crate::circuit::{Circuit, Drawings, Net, Part};
use crate::footprints::{self, Library};
use crate::sexpr::{self, Sexpr};

/// The circuit in the netlist at `path`, its power and ground not known yet. `user_footprints`: read
/// your own footprints.json (`footprints::user_footprints`)? Not for the benchmark, which must place
/// the same circuits the same way on every machine.
pub fn load(path: &str, user_footprints: bool) -> Circuit {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let root = sexpr::parse(&text).unwrap_or_else(|e| panic!("bad netlist {path}: {e}"));
    if root.head() != Some("export") {
        panic!("{path} doesn't look like a KiCad netlist (it should start with \"(export\")");
    }
    let library = footprints::library(path, user_footprints);
    let parts: Vec<Part> =
        root.child("components").into_iter().flat_map(|cs| cs.children("comp")).map(|comp| part(comp, &library)).collect();
    library.note_replaced(parts.iter().map(|c| c.footprint.name.as_str()));
    // KiCad also lists pins that connect to nothing as one-pin nets; there's nothing to wire for those.
    let nets: Vec<Net> =
        root.child("nets").into_iter().flat_map(|ns| ns.children("net")).map(net).filter(|n| n.pins.len() > 1).collect();
    Circuit::new(parts, nets)
}

fn part(comp: &Sexpr, library: &Library) -> Part {
    let reference = comp.value("ref").expect("a (comp) without a (ref)").to_string();
    let footprint = comp.value("footprint").unwrap_or_else(|| panic!("{reference} has no footprint; set one in KiCad"));
    let found = library.find(&reference, footprint);
    let value = comp.value("value").unwrap_or("").to_string();
    Part { reference, value, footprint: found.footprint, drawings: Drawings::new(), layers: found.layers, footprint_from: found.from }
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
