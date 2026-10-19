//! Turns a KiCad netlist (.net) plus our footprint library into a `Circuit`.
//!
//! The netlist says which pin connects to which net and names each part's
//! footprint. It says nothing about where the pins physically are, so that
//! comes from `footprints.json`: footprint name -> size and pad positions in holes.
//! The repo's footprints.json is built into the program, so it works from any folder; a
//! footprints.json next to the netlist adds parts to it (or replaces ones with the same name).

use crate::board::{BoardSize, Circuit, Component, Net, Pin, Point};
use crate::diylc;
use crate::sexpr::{self, Sexpr};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

pub const FOOTPRINTS: &str = "footprints.json";
/// The repo's footprints.json, built into the program when it's compiled.
const BUILT_IN: &str = include_str!("../footprints.json");

/// Net names tried, in order, when `--power` / `--ground` aren't given.
const POWER_NAMES: [&str; 6] = ["+5V", "5V", "VCC", "+3V3", "+3.3V", "VDD"];
const GROUND_NAMES: [&str; 3] = ["GND", "VSS", "0"];
/// Pin names that say what a net is, best first; used to name nets read from DIYLC.
pub const NET_NAME_HINTS: [&str; 9] = ["+5V", "5V", "+3V3", "3V3", "+3.3V", "VCC", "VDD", "GND", "VSS"];

/// One entry of footprints.json. Other keys are ignored.
#[derive(Deserialize)]
struct Footprint {
    /// What it is, in words: printed by the `footprints` command.
    about: Option<String>,
    w: i32,
    h: i32,
    /// Pad number -> hole, counted from the footprint's top-left hole.
    pins: BTreeMap<String, Point>,
    diylc: Option<diylc::Part>,
    /// Low enough to sit under a module on headers. Leave out if it isn't.
    #[serde(default)]
    low: bool,
    /// A module on headers (an Arduino Nano, a motor driver breakout): only its pin holes are
    /// taken, and low parts can go under its board, like a DIYLC group with pins in it.
    #[serde(default)]
    module: bool,
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
    let library = library(path);

    let components: Vec<Component> =
        root.child("components").into_iter().flat_map(|cs| cs.children("comp")).map(|comp| component(comp, &library)).collect();
    // KiCad also lists pins that connect to nothing as one-pin nets; there's nothing to wire for those.
    let nets: Vec<Net> =
        root.child("nets").into_iter().flat_map(|ns| ns.children("net")).map(net).filter(|n| n.pins.len() > 1).collect();

    let (power, ground) = power_and_ground(&nets, &opts);
    Circuit::new(opts.board, components, nets, power, ground)
}

/// The built-in footprints, plus (or replaced by) those in a footprints.json next to the circuit
/// at `path`, if there is one.
fn library(path: &str) -> HashMap<String, Footprint> {
    let mut library: HashMap<String, Footprint> = serde_json::from_str(BUILT_IN).unwrap_or_else(|e| panic!("bad built-in {FOOTPRINTS}: {e}"));
    let local = std::path::Path::new(path).with_file_name(FOOTPRINTS);
    if let Ok(text) = std::fs::read_to_string(&local) {
        let more: HashMap<String, Footprint> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad {}: {e}", local.display()));
        library.extend(more);
    }
    library
}

/// Every footprint a circuit at `path` can use (see `library`), in words, by name: its size in
/// holes, each pad's hole (before turning), and what it is. For the `footprints` command.
pub fn describe(path: &str) -> Vec<String> {
    let library = library(path);
    let mut names: Vec<&String> = library.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let fp = &library[name];
            let kind = if fp.module { ", a module on headers" } else if fp.low { ", low" } else { "" };
            // Pads by number where they're numbers (1, 2, ... 10), else by name.
            let mut pins: Vec<(&String, &Point)> = fp.pins.iter().collect();
            pins.sort_by_key(|(pad, _)| (pad.parse::<u32>().ok(), pad.to_string()));
            let pads: Vec<String> = pins.iter().map(|(pad, (x, y))| format!("{pad}@{x},{y}")).collect();
            let about = fp.about.as_deref().map(|a| format!("\n    {a}")).unwrap_or_default();
            format!("{name}: {}x{} holes{kind}\n    pads {}{about}", fp.w, fp.h, pads.join(" "))
        })
        .collect()
}

fn component(comp: &Sexpr, library: &HashMap<String, Footprint>) -> Component {
    let reference = comp.value("ref").expect("a (comp) without a (ref)").to_string();
    let footprint = comp.value("footprint").unwrap_or_else(|| panic!("{reference} has no footprint; set one in KiCad"));
    // "Package_DIP:DIP-8_W7.62mm" -> "DIP-8_W7.62mm": the library name before ':' doesn't matter here.
    let name = footprint.rsplit(':').next().unwrap();
    let fp = library.get(name).unwrap_or_else(|| {
        panic!("{reference} uses footprint {name}, which isn't in the built-in {FOOTPRINTS} yet: add its size and pads in holes there, or in a {FOOTPRINTS} next to the netlist")
    });
    let pins = fp.pins.iter().map(|(name, &at)| Pin { name: name.clone(), at, off_grid: false }).collect();
    let value = comp.value("value").unwrap_or("").to_string();
    let body = if fp.module { fp.pins.values().copied().collect() } else { Component::full_body(fp.w, fp.h) };
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
