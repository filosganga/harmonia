//! Writes a layout (src/layout.rs) as a KiCad board (.kicad_pcb) that opens in KiCad's PCB editor.
//!
//! Each part becomes a simple footprint built from footprints.json: round pads (square
//! for pad 1) on a 2.54mm grid, a box on the silkscreen and courtyard, and every pad on
//! its net, so KiCad draws the connections still to be wired (the "ratsnest").
//! Each footprint keeps its real library name: in KiCad, Tools -> Update Footprints from
//! Library swaps in the real drawing without moving anything.
//!
//! Written in KiCad 9's file format, which KiCad 10 opens fine.

use crate::board::{self, Component, Point};
use crate::layout::Layout;
use std::fmt::Write;

/// One hole, in millimetres.
const PITCH: f64 = 2.54;
/// Where hole (0, 0) goes on KiCad's page. A multiple of PITCH, so the holes sit on KiCad's 2.54mm grid.
const ORIGIN: f64 = 40.0 * PITCH;

/// Hole -> millimetres on the page.
fn mm(hole: i32) -> f64 {
    ORIGIN + hole as f64 * PITCH
}

/// The pad a footprint is positioned by: pad "1" if there is one. KiCad's library
/// footprints for through-hole parts put pad 1 at the footprint's origin too.
fn anchor(comp: &Component) -> usize {
    comp.pins.iter().position(|p| p.name == "1").unwrap_or(0)
}

/// KiCad wants a unique id on every item. These are made-up but valid, and the same every run.
fn uuid(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

pub fn write(layout: &Layout) -> String {
    let (c, placement) = (&layout.circuit, &layout.placement);
    let g = board::geometry(c, placement);
    let mut ids = 0..;
    let mut s = String::new();
    s += "(kicad_pcb\n  (version 20241229)\n  (generator \"perfboard\")\n  (generator_version \"0.1\")\n";
    s += "  (general (thickness 1.6))\n  (paper \"A4\")\n";
    s += "  (layers\n    (0 \"F.Cu\" signal)\n    (2 \"B.Cu\" signal)\n    (5 \"F.SilkS\" user \"F.Silkscreen\")\n    (7 \"B.SilkS\" user \"B.Silkscreen\")\n    (1 \"F.Mask\" user)\n    (3 \"B.Mask\" user)\n    (25 \"Edge.Cuts\" user)\n    (31 \"F.CrtYd\" user \"F.Courtyard\")\n    (29 \"B.CrtYd\" user \"B.Courtyard\")\n    (35 \"F.Fab\" user)\n    (33 \"B.Fab\" user)\n  )\n";
    s += "  (setup (pad_to_mask_clearance 0))\n";

    // Nets are numbered from 1; 0 means "no net".
    s += "  (net 0 \"\")\n";
    for (i, net) in c.nets.iter().enumerate() {
        writeln!(s, "  (net {} {})", i + 1, quote(&net.name)).unwrap();
    }
    // net_of[component][pin] = the pin's net number
    let mut net_of: Vec<Vec<usize>> = c.components.iter().map(|comp| vec![0; comp.pins.len()]).collect();
    for (n, members) in c.net_pins.iter().enumerate() {
        for &(ci, pi) in members {
            net_of[ci][pi] = n + 1;
        }
    }

    for (ci, (comp, place)) in c.components.iter().zip(placement).enumerate() {
        let a = anchor(comp);
        let (ax, ay) = g.pins[ci][a];
        // Our rotation turns parts clockwise; KiCad measures angles anticlockwise.
        let angle = (360 - 90 * place.rot as i32) % 360;
        // Everything inside a footprint is measured from the anchor pad, before rotating.
        let local = |(x, y): Point| ((x - comp.pins[a].at.0) as f64 * PITCH, (y - comp.pins[a].at.1) as f64 * PITCH);

        writeln!(s, "  (footprint {}\n    (layer \"F.Cu\")\n    (uuid \"{}\")", quote(&comp.footprint), uuid(ids.next().unwrap())).unwrap();
        writeln!(s, "    (at {:.2} {:.2} {angle})", mm(ax), mm(ay)).unwrap();

        // The body box: from the middle of the top-left hole to the bottom-right one, plus half a hole
        // minus a hair, so parts in neighbouring holes don't count as touching.
        let (x0, y0) = local((0, 0));
        let (x1, y1) = local((comp.w - 1, comp.h - 1));
        let half = PITCH / 2.0 - 0.1;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        for (name, text, layer, dy) in [("Reference", &comp.reference, "F.Fab", 0.0), ("Value", &comp.value, "F.Fab", 1.2)] {
            writeln!(
                s,
                "    (property \"{name}\" {} (at {cx:.2} {:.2} {angle}) (layer \"{layer}\") (uuid \"{}\")\n      (effects (font (size 1 1) (thickness 0.15))))",
                quote(text), cy + dy, uuid(ids.next().unwrap())
            )
            .unwrap();
        }
        s += "    (attr through_hole)\n";
        // A module only takes its pin holes, so it gets no courtyard: parts may sit under it.
        let module = comp.body.len() < (comp.w * comp.h) as usize;
        for (layer, width) in [("F.SilkS", 0.12), ("F.CrtYd", 0.05)].into_iter().filter(|&(l, _)| !(module && l == "F.CrtYd")) {
            writeln!(
                s,
                "    (fp_rect (start {:.2} {:.2}) (end {:.2} {:.2}) (stroke (width {width}) (type solid)) (fill no) (layer \"{layer}\") (uuid \"{}\"))",
                x0 - half, y0 - half, x1 + half, y1 + half, uuid(ids.next().unwrap())
            )
            .unwrap();
        }
        for (pi, pin) in comp.pins.iter().enumerate() {
            let (px, py) = local(pin.at);
            let shape = if pi == a { "rect" } else { "circle" };
            let net = match net_of[ci][pi] {
                0 => String::new(),
                n => format!(" (net {n} {})", quote(&c.nets[n - 1].name)),
            };
            writeln!(
                s,
                "    (pad {} thru_hole {shape} (at {px:.2} {py:.2} {angle}) (size 1.7 1.7) (drill 1) (layers \"*.Cu\" \"*.Mask\"){net} (uuid \"{}\"))",
                quote(&pin.name), uuid(ids.next().unwrap())
            )
            .unwrap();
        }
        s += "  )\n";
    }

    // Keep-outs from the board file become KiCad "rule areas": KiCad keeps enforcing them while you edit.
    for k in &c.constraints.keepout {
        let (x0, x1) = (mm(k.from.0.min(k.to.0)) - PITCH / 2.0, mm(k.from.0.max(k.to.0)) + PITCH / 2.0);
        let (y0, y1) = (mm(k.from.1.min(k.to.1)) - PITCH / 2.0, mm(k.from.1.max(k.to.1)) + PITCH / 2.0);
        let name = if k.name.is_empty() { "Keep-out" } else { &k.name };
        writeln!(
            s,
            "  (zone (net 0) (net_name \"\") (layers \"F.Cu\" \"B.Cu\") (uuid \"{}\") (name {}) (hatch full 0.5)\n    (connect_pads (clearance 0)) (min_thickness 0.25)\n    (keepout (tracks not_allowed) (vias not_allowed) (pads not_allowed) (copperpour not_allowed) (footprints not_allowed))\n    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5))\n    (polygon (pts (xy {x0:.2} {y0:.2}) (xy {x1:.2} {y0:.2}) (xy {x1:.2} {y1:.2}) (xy {x0:.2} {y1:.2}))))",
            uuid(ids.next().unwrap()), quote(name)
        )
        .unwrap();
    }

    // The board's outline: one hole's width outside the outer holes, like a real perfboard (and DIYLC).
    writeln!(
        s,
        "  (gr_rect (start {:.2} {:.2}) (end {:.2} {:.2}) (stroke (width 0.1) (type solid)) (fill no) (layer \"Edge.Cuts\") (uuid \"{}\"))",
        mm(-1), mm(-1), mm(c.board.width), mm(c.board.height), uuid(ids.next().unwrap())
    )
    .unwrap();
    s + ")\n"
}

/// A KiCad quoted string: "like this", with \ and " escaped.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}
