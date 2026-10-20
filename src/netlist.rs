//! Turns a KiCad netlist (.net) plus our footprint library into a `Design`.
//!
//! The netlist says which pin connects to which net and names each part's
//! footprint. It says nothing about where the pins physically are, so that
//! comes from `footprints.json`: footprint name -> size and pad positions in holes.
//! The repo's footprints.json is built into the program, so it works from any folder. Your own
//! footprints.json in the config folder (`user_footprints`) adds parts for every circuit, and one
//! next to the netlist for the circuits in that folder; each replaces entries of the same name.

use crate::board::{BoardSize, Design, Component, Net, Pin, Point};
use crate::diylc;
use crate::sexpr::{self, Sexpr};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const FOOTPRINTS: &str = "footprints.json";
/// The repo's footprints.json, built into the program when it's compiled.
const BUILT_IN: &str = include_str!("../footprints.json");

/// Net names tried, in order, when `--power` / `--ground` aren't given.
const POWER_NAMES: [&str; 6] = ["+5V", "5V", "VCC", "+3V3", "+3.3V", "VDD"];
const GROUND_NAMES: [&str; 3] = ["GND", "VSS", "0"];
/// Pin names that say what a net is, best first; used to name nets read from DIYLC.
pub const NET_NAME_HINTS: [&str; 9] = ["+5V", "5V", "+3V3", "3V3", "+3.3V", "VCC", "VDD", "GND", "VSS"];

/// One entry of footprints.json. Other keys are ignored.
#[derive(Deserialize, PartialEq)]
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
    /// Read your own footprints.json (`user_footprints`)? Not for the benchmark, which must place
    /// the same circuits the same way on every machine.
    pub user_footprints: bool,
}

pub fn load(path: &str, opts: Options) -> Design {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let root = sexpr::parse(&text).unwrap_or_else(|e| panic!("bad netlist {path}: {e}"));
    if root.head() != Some("export") {
        panic!("{path} doesn't look like a KiCad netlist (it should start with \"(export\")");
    }
    let library = library(path, opts.user_footprints);
    let components: Vec<Component> =
        root.child("components").into_iter().flat_map(|cs| cs.children("comp")).map(|comp| component(comp, &library)).collect();
    // Say when a footprint this circuit uses replaced a different one: never silently.
    let used: std::collections::HashSet<&str> = components.iter().map(|c| c.footprint.rsplit(':').next().unwrap()).collect();
    for (name, (file, replaced)) in library.from.iter().filter(|(name, _)| used.contains(name.as_str())) {
        if let Some(what) = replaced {
            println!("Note: {name} comes from {}, replacing {what}", file.display());
        }
    }
    // KiCad also lists pins that connect to nothing as one-pin nets; there's nothing to wire for those.
    let nets: Vec<Net> =
        root.child("nets").into_iter().flat_map(|ns| ns.children("net")).map(net).filter(|n| n.pins.len() > 1).collect();

    let (power, ground) = power_and_ground(&nets, &opts);
    Design::new(opts.board, components, nets, power, ground)
}

/// Your own footprints.json, for every circuit: in harmonia's config folder,
/// `$XDG_CONFIG_HOME/harmonia` or else `~/.config/harmonia` (`%APPDATA%\harmonia` on Windows).
/// None if there's no home folder to look in.
pub fn user_footprints() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let config = if cfg!(windows) { var("APPDATA") } else { var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config"))) };
    Some(config?.join("harmonia").join(FOOTPRINTS))
}

/// The footprints a circuit can use, and where each came from.
struct Library {
    parts: HashMap<String, Footprint>,
    /// For each footprint not from the built-in library: the file it came from, and what it
    /// replaced if that was different ("a different built-in one", or one from an earlier file).
    from: HashMap<String, (PathBuf, Option<String>)>,
    /// The footprints.json files read, in order.
    read: Vec<PathBuf>,
}

/// The built-in footprints, then those in your own footprints.json (`user_footprints`, if `user`),
/// then those in one next to the circuit at `path`: the more local file wins. An entry the same as
/// the one it replaces changes nothing (say, one copied from the repo's file).
fn library(path: &str, user: bool) -> Library {
    let parts = serde_json::from_str(BUILT_IN).unwrap_or_else(|e| panic!("bad built-in {FOOTPRINTS}: {e}"));
    let mut lib = Library { parts, from: HashMap::new(), read: Vec::new() };
    let local = Path::new(path).with_file_name(FOOTPRINTS);
    for file in user_footprints().filter(|_| user).into_iter().chain([local]) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let more: HashMap<String, Footprint> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad {}: {e}", file.display()));
        for (name, fp) in more {
            let replaced = match lib.parts.get(&name) {
                Some(old) if *old == fp => continue,
                Some(_) => Some(match lib.from.get(&name) {
                    Some((earlier, _)) => format!("a different one from {}", earlier.display()),
                    None => "a different built-in one".to_string(),
                }),
                None => None,
            };
            lib.from.insert(name.clone(), (file.clone(), replaced));
            lib.parts.insert(name, fp);
        }
        lib.read.push(file);
    }
    lib
}

/// Every footprint a circuit at `path` can use (see `library`), in words, by name: its size in
/// holes, each pad's hole (before turning), and what it is; first, which files were read. For the
/// `footprints` command.
pub fn describe(path: &str) -> Vec<String> {
    let library = library(path, true);
    let read = &library.read;
    let mut names: Vec<&String> = library.parts.keys().collect();
    names.sort();
    let files: Vec<String> = read.iter().map(|f| f.display().to_string()).collect();
    let mut from = String::from("Footprints: the built-in ones");
    if !files.is_empty() {
        from += &format!(", then {}", files.join(", then "));
    }
    if let Some(user) = user_footprints().filter(|f| !read.contains(f)) {
        from += &format!(" (your own go in {})", user.display());
    }
    std::iter::once(from + "\n")
        .chain(names.into_iter()
        .map(|name| {
            let fp = &library.parts[name];
            let kind = if fp.module { ", a module on headers" } else if fp.low { ", low" } else { "" };
            let origin = match library.from.get(name) {
                Some((file, Some(what))) => format!(" (from {}, replacing {what})", file.display()),
                Some((file, None)) => format!(" (from {})", file.display()),
                None => String::new(),
            };
            // Pads by number where they're numbers (1, 2, ... 10), else by name.
            let mut pins: Vec<(&String, &Point)> = fp.pins.iter().collect();
            pins.sort_by_key(|(pad, _)| (pad.parse::<u32>().ok(), pad.to_string()));
            let pads: Vec<String> = pins.iter().map(|(pad, (x, y))| format!("{pad}@{x},{y}")).collect();
            let about = fp.about.as_deref().map(|a| format!("\n    {a}")).unwrap_or_default();
            format!("{name}: {}x{} holes{kind}{origin}\n    pads {}{about}", fp.w, fp.h, pads.join(" "))
        }))
        .collect()
}

fn component(comp: &Sexpr, library: &Library) -> Component {
    let reference = comp.value("ref").expect("a (comp) without a (ref)").to_string();
    let footprint = comp.value("footprint").unwrap_or_else(|| panic!("{reference} has no footprint; set one in KiCad"));
    // "Package_DIP:DIP-8_W7.62mm" -> "DIP-8_W7.62mm": the library name before ':' doesn't matter here.
    let name = footprint.rsplit(':').next().unwrap();
    let fp = library.parts.get(name).unwrap_or_else(|| {
        let user = user_footprints().map(|f| format!(", in yours ({})", f.display())).unwrap_or_default();
        panic!("{reference} uses footprint {name}, which isn't in the built-in {FOOTPRINTS} yet: add its size and pads in holes there{user}, or in a {FOOTPRINTS} next to the netlist")
    });
    let pins = fp.pins.iter().map(|(name, &at)| Pin { name: name.clone(), at, off_grid: false }).collect();
    let value = comp.value("value").unwrap_or("").to_string();
    let body = if fp.module { fp.pins.values().copied().collect() } else { Component::full_body(fp.w, fp.h) };
    let footprint_from = library.from.get(name).map(|(file, replaced)| match replaced {
        Some(what) => format!("{name} from {}, replacing {what}", file.display()),
        None => format!("{name} from {}", file.display()),
    });
    Component { reference, value, footprint: footprint.to_string(), diylc: fp.diylc.clone(), w: fp.w, h: fp.h, pins, body, source: None, low: fp.low, footprint_from }
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
