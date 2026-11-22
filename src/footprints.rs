//! The footprint library: reads footprints.json (footprint name -> size and pad positions in holes)
//! and gives a part its `board::Footprint`.
//!
//! The repo's footprints.json is built into the program, so it works from any folder. Your own
//! footprints.json in the config folder (`user_footprints`) adds footprints for every circuit, and one
//! next to the netlist for the circuits in that folder; each replaces entries of the same name.

use crate::board::{Drawing, Drawings, Footprint, Format, Layers, Look, Pin, Point};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const FOOTPRINTS: &str = "footprints.json";
/// The repo's footprints.json, built into the program when it's compiled.
const BUILT_IN: &str = include_str!("../footprints.json");

/// One entry of footprints.json. Other keys are ignored.
#[derive(Deserialize, PartialEq)]
struct Entry {
    /// What it is, in words: printed by the `footprints` command.
    about: Option<String>,
    w: i32,
    h: i32,
    /// Pad number -> hole, counted from the footprint's top-left hole.
    pins: BTreeMap<String, Point>,
    /// How DIYLC draws it: kept as the footprint's DIYLC drawing, unread (only `diylc` reads it).
    diylc: Option<serde_json::Value>,
    /// Low enough to sit under a module on headers. Leave out if it isn't.
    #[serde(default)]
    low: bool,
    /// A module (an Arduino Nano, a motor driver breakout): a board of its own, standing on its
    /// pins, like a DIYLC group with pins in it. On headers unless `layers` says otherwise, so low
    /// parts and jumper wires can go under it.
    #[serde(default)]
    module: bool,
    /// Which layers above the board it takes, [from, to] (see `board::Layers`), if not what
    /// `module` and `low` say: a module on headers [2, 2], a low part [1, 1], any other [1, 2].
    #[serde(default)]
    layers: Option<Layers>,
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
pub struct Library {
    entries: HashMap<String, Entry>,
    /// For each footprint not from the built-in library: the file it came from, and what it
    /// replaced if that was different ("a different built-in one", or one from an earlier file).
    from: HashMap<String, (PathBuf, Option<String>)>,
    /// The footprints.json files read, in order.
    read: Vec<PathBuf>,
}

/// The built-in footprints, then those in your own footprints.json (`user_footprints`, if `user`),
/// then those in one next to the circuit at `path`: the more local file wins. An entry the same as
/// the one it replaces changes nothing (say, one copied from the repo's file).
pub fn library(path: &str, user: bool) -> Library {
    let entries = serde_json::from_str(BUILT_IN).unwrap_or_else(|e| panic!("bad built-in {FOOTPRINTS}: {e}"));
    let mut lib = Library { entries, from: HashMap::new(), read: Vec::new() };
    let local = Path::new(path).with_file_name(FOOTPRINTS);
    for file in user_footprints().filter(|_| user).into_iter().chain([local]) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let more: HashMap<String, Entry> = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad {}: {e}", file.display()));
        for (name, entry) in more {
            let replaced = match lib.entries.get(&name) {
                // The "diylc" entry is compared as written: only diylc knows what it means.
                Some(old) if *old == entry => continue,
                Some(_) => Some(match lib.from.get(&name) {
                    Some((earlier, _)) => format!("a different one from {}", earlier.display()),
                    None => "a different built-in one".to_string(),
                }),
                None => None,
            };
            lib.from.insert(name.clone(), (file.clone(), replaced));
            lib.entries.insert(name, entry);
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
    let mut names: Vec<&String> = library.entries.keys().collect();
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
            let entry = &library.entries[name];
            let kind = match (entry.layers, entry.module, entry.low) {
                (Some(l), module, _) => format!("{}, on layers {}-{}", if module { ", a module" } else { "" }, l.lo, l.hi),
                (None, true, _) => ", a module on headers".to_string(),
                (None, false, true) => ", low".to_string(),
                (None, false, false) => String::new(),
            };
            let origin = match library.from.get(name) {
                Some((file, Some(what))) => format!(" (from {}, replacing {what})", file.display()),
                Some((file, None)) => format!(" (from {})", file.display()),
                None => String::new(),
            };
            // Pads by number where they're numbers (1, 2, ... 10), else by name.
            let mut pins: Vec<(&String, &Point)> = entry.pins.iter().collect();
            pins.sort_by_key(|(pad, _)| (pad.parse::<u32>().ok(), pad.to_string()));
            let pads: Vec<String> = pins.iter().map(|(pad, (x, y))| format!("{pad}@{x},{y}")).collect();
            let about = entry.about.as_deref().map(|a| format!("\n    {a}")).unwrap_or_default();
            format!("{name}: {}x{} holes{kind}{origin}\n    pads {}{about}", entry.w, entry.h, pads.join(" "))
        }))
        .collect()
}

/// What footprints.json gives a part that uses one of its footprints: the footprint itself, and
/// what the entry says about the part.
pub struct Found {
    pub footprint: Footprint,
    /// Which layers the part takes: the entry's `layers`, or what `module` and `low` say.
    pub layers: Layers,
    /// A module (see `Part::module`).
    pub module: bool,
    /// The file it came from, if not the built-in library, and what it replaced: see `Part::footprint_from`.
    pub from: Option<String>,
}

impl Library {
    /// What the entry for `full_name` (a KiCad name, "Package_DIP:DIP-8_W7.62mm": the library name
    /// before ':' doesn't matter here) gives part `reference`: its footprint, and what the entry
    /// says about the part (`Found`). Stops, saying where to add one, if there's none.
    pub fn find(&self, reference: &str, full_name: &str) -> Found {
        let name = full_name.rsplit(':').next().unwrap();
        let entry = self.entries.get(name).unwrap_or_else(|| {
            let user = user_footprints().map(|f| format!(", in yours ({})", f.display())).unwrap_or_default();
            panic!("{reference} uses footprint {name}, which isn't in the built-in {FOOTPRINTS} yet: add its size and pads in holes there{user}, or in a {FOOTPRINTS} next to the netlist")
        });
        let pins = entry.pins.iter().map(|(pad, &at)| Pin { name: pad.clone(), at, off_grid: false }).collect();
        let body = if entry.module { entry.pins.values().copied().collect() } else { Footprint::full_body(entry.w, entry.h) };
        let from = self.from.get(name).map(|(file, replaced)| match replaced {
            Some(what) => format!("{name} from {}, replacing {what}", file.display()),
            None => format!("{name} from {}", file.display()),
        });
        let layers = entry.layers.map_or(Layers::of(entry.module, entry.low), |l| l.checked(&format!("{FOOTPRINTS}: {name}")));
        Found {
            footprint: Footprint {
                name: full_name.to_string(),
                w: entry.w,
                h: entry.h,
                pins,
                body,
                // A module is a board of its own; what a DIYLC drawing says comes later (`diylc::look`).
                look: if entry.module { Look::OwnBoard } else { Look::Box },
                drawings: entry.diylc.iter().map(|d| (Format::Diylc, Drawing(d.clone()))).collect::<Drawings>(),
            },
            layers,
            module: entry.module,
            from,
        }
    }

    /// Says when a footprint a circuit uses (full names in `used`) replaced a different one: never
    /// silently.
    pub fn note_replaced<'a>(&self, used: impl IntoIterator<Item = &'a str>) {
        let used: HashSet<&str> = used.into_iter().map(|name| name.rsplit(':').next().unwrap()).collect();
        for (name, (file, replaced)) in self.from.iter().filter(|(name, _)| used.contains(name.as_str())) {
            if let Some(what) = replaced {
                println!("Note: {name} comes from {}, replacing {what}", file.display());
            }
        }
    }
}
