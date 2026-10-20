//! Draws a build (src/build.rs) as an SVG picture of the perfboard: holes coloured by net, the
//! routed copper traces and jumpers, the parts as they look from above, header pin names, and beside
//! the board a legend with the score and every net. Open it in a browser; hovering a hole names
//! it, its pin and net.
//!
//! `Side::Solder` is the board turned over to solder it: mirrored, so column 0 is on the right,
//! with the copper in front and the parts behind the board, faint.

use crate::board::{self, Component, Point};
use crate::build::Build;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Pixels per hole (one hole is 0.1", 2.54mm).
const PX: f64 = 30.0;
const MARGIN: f64 = 40.0;
const LEGEND_W: f64 = 420.0;
/// The colour of a pin connected to nothing.
const LONE: &str = "#9a9a9a";

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Component,
    Solder,
}

const STYLE: &str = "
  text { font-family: -apple-system, 'Segoe UI', Helvetica, Arial, sans-serif; }
  .coord { font-size: 11px; fill: #6b6b6b; text-anchor: middle; font-family: Menlo, Consolas, monospace; }
  .keepout { fill: #555555; fill-opacity: 0.25; stroke: #555555; stroke-dasharray: 4 3; }
  .keepout-name { font-size: 10px; text-anchor: middle; fill: #222222; }
  .module { fill: #7fbf7f; fill-opacity: 0.12; stroke: #4d8f4d; stroke-dasharray: 6 4; stroke-width: 1.5; }
  .module-name { font-size: 11px; fill: #3d6e3d; font-style: italic; paint-order: stroke; stroke: #ffffff; stroke-width: 3px; }
  .lead { stroke: #8f8f8f; stroke-width: 2.5; }
  .jumper-halo { stroke: #ffffff; stroke-width: 7; stroke-linecap: round; }
  .jumper { stroke: #1f4fd6; stroke-width: 4; stroke-linecap: round; }
  .bottom-wire { stroke: #b00020; stroke-width: 4; stroke-linecap: round; stroke-dasharray: 8 5; }
  .unrouted { stroke: #777777; stroke-width: 1.5; stroke-dasharray: 2 4; }
  .resistor { fill: #e0c68b; stroke: #a59268; stroke-width: 1.5; }
  .diode { fill: #444444; stroke: #222222; stroke-width: 1.5; }
  .band { fill: #dddddd; }
  .cap { fill: #333333; fill-opacity: 0.18; stroke: #333333; stroke-width: 1.5; }
  .led { fill: #d62728; fill-opacity: 0.15; stroke: #a01c1c; stroke-width: 1.5; }
  .plus { font-size: 16px; font-weight: bold; text-anchor: middle; fill: #d62728; paint-order: stroke; stroke: #ffffff; stroke-width: 3px; }
  .package { fill: #808080; fill-opacity: 0.2; stroke: #595959; stroke-width: 1.5; }
  .header { fill: none; stroke: #222222; stroke-width: 1.5; }
  .clash { fill: none; stroke: #b00020; stroke-width: 3; }
  .pin { font-size: 8px; text-anchor: middle; fill: #000000; paint-order: stroke; stroke: #ffffff; stroke-width: 2.5px; }
  .part-name, .header-name { font-size: 10px; text-anchor: middle; fill: #111111; paint-order: stroke; stroke: #ffffff; stroke-width: 3px; }
  .header-name { font-weight: 600; }
  .legend-title { font-size: 13px; font-weight: 600; fill: #111111; }
  .legend { font-size: 11px; fill: #222222; }
  .note { font-size: 11px; fill: #555555; }
  .warn { font-size: 11px; fill: #b00020; }
";

/// What to draw a part as: the short name of its DIYLC class (`diylc::CLASSES`, the same name
/// whether it came from footprints.json or a sketch), or "module" for a part parts can go under (a
/// DIYLC group, or a footprint with "module": true).
fn shape(comp: &Component) -> &str {
    let full = comp.body.len() == (comp.w * comp.h) as usize;
    match &comp.diylc {
        Some(p) => &p.part,
        // A group with nothing free under it (say, a header and its label) is drawn like a header.
        None if comp.footprint == "DIYLC:module" && full => "PinHeader",
        // A footprints.json module. (A DIYLC part drawn at an angle only takes its legs too, but it's
        // still drawn as itself, below.)
        None if !full && !comp.footprint.starts_with("DIYLC:") => "module",
        None => comp.footprint.strip_prefix("DIYLC:").unwrap_or(""),
    }
}

/// "ESP32C6-L.5v" -> ("ESP32C6-L", "5v"): the header a pin belongs to, and its own name.
fn split_pin(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or(("", name))
}

/// The leg marked + on a polarised part: the capacitor's +, or the LED's anode.
fn plus_leg(comp: &Component) -> usize {
    let named = comp.diylc.as_ref().and_then(|d| d.legs.first()).map(String::as_str);
    let is_plus = |n: &str| n == "+" || n == "A" || Some(n) == named;
    comp.pins.iter().position(|p| is_plus(&p.name)).unwrap_or(0)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Splits text into lines of at most about `width` characters.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let line = lines.last_mut().unwrap();
        if !line.is_empty() && line.len() + word.len() >= width {
            lines.push(word.to_string());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
}

pub fn draw(build: &Build, side: Side) -> String {
    let (c, title) = (&build.design, build.title.as_str());
    let (w, h) = (c.board.width, c.board.height);
    let mirror = side == Side::Solder;
    let g = board::geometry(c, &build.layout.placement);
    let m = build.measures;

    // Holes -> pixels. Fractions are for points between holes, like the middle of a resistor.
    let px = |x: f64| MARGIN + (if mirror { w as f64 - x } else { x + 1.0 }) * PX;
    let py = |y: f64| MARGIN + (y + 1.0) * PX;
    let at = |(x, y): Point| (px(x as f64), py(y as f64));
    let (left, top) = (MARGIN, MARGIN);
    let (right, bottom) = (MARGIN + (w + 1) as f64 * PX, MARGIN + (h + 1) as f64 * PX);
    // The box around some holes, `pad` holes wider on every side: x, y, width, height.
    let bounds = |holes: &[Point], pad: f64| {
        let xs: Vec<f64> = holes.iter().map(|&p| at(p).0).collect();
        let ys: Vec<f64> = holes.iter().map(|&p| at(p).1).collect();
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min) - pad * PX;
        let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max) + pad * PX;
        (min(&xs), min(&ys), max(&xs) - min(&xs), max(&ys) - min(&ys))
    };
    let rect = |holes: &[Point], pad: f64| {
        let (x, y, bw, bh) = bounds(holes, pad);
        format!(r#"x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}""#)
    };

    // Each pin's net.
    let colors = board::net_colors(c);
    let mut net_of: HashMap<(usize, usize), usize> = HashMap::new();
    for (n, members) in c.net_pins.iter().enumerate() {
        for &p in members {
            net_of.insert(p, n);
        }
    }
    let pin_color = |p: (usize, usize)| net_of.get(&p).map_or(LONE, |&n| colors[n]);

    // Parts that collide, hang off the board, sit on a hard keep-out or don't fit under a module (both
    // parts of a collision): only in a layout that isn't allowed, like a sketch as you drew it.
    let mut misplaced: HashSet<usize> = HashSet::new();
    for clash in board::clashes(c, &g, &vec![true; c.components.len()]) {
        misplaced.insert(clash.part);
        if let board::Why::OnPart(other) = clash.why {
            misplaced.insert(other);
        }
    }
    let clashes = |i: usize| misplaced.contains(&i);

    // The picture is built in layers, drawn bottom to top.
    let (mut base, mut wires, mut holes, mut parts, mut labels, mut names) = (String::new(), String::new(), String::new(), String::new(), String::new(), String::new());

    // --- the board, its hole numbers (the same as the text board's) and keep-outs
    writeln!(base, r##"<rect x="{left}" y="{top}" width="{:.1}" height="{:.1}" rx="6" fill="#eef3e6" stroke="#b5b08a"/>"##, right - left, bottom - top).unwrap();
    for x in 0..w {
        let cx = px(x as f64);
        writeln!(base, r#"<text x="{cx:.1}" y="{:.1}" class="coord">{x}</text>"#, top - 6.6).unwrap();
        writeln!(base, r#"<text x="{cx:.1}" y="{:.1}" class="coord">{x}</text>"#, bottom + 16.5).unwrap();
    }
    for y in 0..h {
        let cy = py(y as f64) + 3.9;
        writeln!(base, r#"<text x="{:.1}" y="{cy:.1}" class="coord">{y}</text>"#, left - 14.3).unwrap();
        writeln!(base, r#"<text x="{:.1}" y="{cy:.1}" class="coord">{y}</text>"#, right + 14.3).unwrap();
    }
    for k in &c.constraints.keepout {
        let (x, y, bw, bh) = bounds(&[k.from, k.to], 0.5);
        writeln!(base, r#"<rect x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" class="keepout"><title>keep-out {}</title></rect>"#, esc(&k.name)).unwrap();
        writeln!(base, r#"<text x="{:.1}" y="{:.1}" class="keepout-name">{}</text>"#, x + bw / 2.0, y + bh / 2.0 + 3.5, esc(&k.name)).unwrap();
    }

    // --- the copper traces, on the solder side
    let routing = &build.layout.routing;
    let opacity = if mirror { 0.9 } else { 0.55 };
    let mut copper: HashMap<Point, usize> = HashMap::new(); // holes the copper touches -> net
    for (run, a, b) in routing.straight_traces() {
        let net = c.runs[run].net;
        let ((x1, y1), (x2, y2)) = (at(a), at(b));
        writeln!(
            wires,
            r#"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="{}" stroke-width="{:.1}" stroke-linecap="round" opacity="{opacity}"><title>trace, {}</title></line>"#,
            colors[net], PX * 0.42, esc(&c.nets[net].name)
        )
        .unwrap();
    }
    // Wires on the solder side, the last resort: dashed, with the copper.
    for &(run, a, b) in &routing.bottom {
        let ((x1, y1), (x2, y2)) = (at(a), at(b));
        let line = format!(r#"x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}""#);
        writeln!(wires, r#"<line {line} class="jumper-halo"/>"#).unwrap();
        writeln!(wires, r#"<line {line} class="bottom-wire"><title>wire on the solder side, {}</title></line>"#, esc(&c.nets[c.runs[run].net].name)).unwrap();
    }
    // Connections the router couldn't make: a thin dotted line, no copper.
    for &(run, a, b) in &routing.unrouted {
        let ((x1, y1), (x2, y2)) = (at(a), at(b));
        writeln!(wires, r#"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" class="unrouted"><title>not connected: {} couldn't be routed here</title></line>"#, esc(&c.nets[c.runs[run].net].name)).unwrap();
    }
    for &(run, a, b) in routing.traces.iter().chain(&routing.jumpers).chain(&routing.bottom) {
        copper.insert(a, c.runs[run].net);
        copper.insert(b, c.runs[run].net);
    }

    // --- holes: a pin's hole in its net's colour, an empty hole small and white
    let mut pin_at: HashMap<Point, (usize, usize)> = HashMap::new();
    for (ci, pins) in g.pins.iter().enumerate() {
        for (pi, &p) in pins.iter().enumerate() {
            pin_at.entry(p).or_insert((ci, pi));
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = at((x, y));
            match pin_at.get(&(x, y)) {
                Some(&(ci, pi)) => {
                    let net = net_of.get(&(ci, pi)).map_or("connected to nothing".to_string(), |&n| format!("net {}", c.nets[n].name));
                    let pin = format!("{}.{}", c.components[ci].reference, c.components[ci].pins[pi].name);
                    writeln!(
                        holes,
                        r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="{}" stroke="#ffffff" stroke-width="1.5"><title>{x},{y} · {} · {}</title></circle>"##,
                        PX * 0.3, pin_color((ci, pi)), esc(&pin), esc(&net)
                    )
                    .unwrap();
                }
                None if copper.contains_key(&(x, y)) => {
                    let net = &c.nets[copper[&(x, y)]];
                    writeln!(
                        holes,
                        r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="{}" stroke="#ffffff" stroke-width="1.5"><title>{x},{y} · trace · net {}</title></circle>"##,
                        PX * 0.22, colors[copper[&(x, y)]], esc(&net.name)
                    )
                    .unwrap();
                }
                None => {
                    writeln!(holes, r##"<circle cx="{cx:.1}" cy="{cy:.1}" r="{:.1}" fill="#ffffff" stroke="#c9c4a0"><title>{x},{y} · empty</title></circle>"##, PX * 0.17).unwrap();
                }
            }
        }
    }

    // --- the parts
    for (i, comp) in c.components.iter().enumerate() {
        let (pins, outline) = (&g.pins[i], &g.outlines[i]);
        let reference = esc(&comp.reference);
        let label = esc(format!("{} {}", comp.reference, comp.value).trim());
        let (ox, oy, ow, oh) = bounds(outline, 0.45);
        let above = |s: &mut String, x: f64, y: f64, class: &str, text: &str| {
            writeln!(s, r#"<text x="{x:.1}" y="{:.1}" class="{class}">{text}</text>"#, y - 5.0).unwrap();
        };
        match shape(comp) {
            // Lying flat between its two legs: a body along the line from one leg to the other.
            "Resistor" | "AxialFilmCapacitor" | "DiodePlastic" if pins.len() == 2 => {
                let ((x1, y1), (x2, y2)) = (at(pins[0]), at(pins[1]));
                let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
                let angle = (y2 - y1).atan2(x2 - x1).to_degrees();
                let length = (((x2 - x1).hypot(y2 - y1)) - 0.8 * PX).clamp(0.8 * PX, 2.5 * PX);
                let width = 0.9 * PX;
                let class = if shape(comp) == "DiodePlastic" { "diode" } else { "resistor" };
                writeln!(parts, r#"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" class="lead"/>"#).unwrap();
                writeln!(parts, r#"<g transform="translate({mx:.1} {my:.1}) rotate({angle:.0})"><title>{label}</title>"#).unwrap();
                writeln!(parts, r#"<rect x="{:.1}" y="{:.1}" width="{length:.1}" height="{width:.1}" rx="{:.1}" class="{class}"/>"#, -length / 2.0, -width / 2.0, width / 2.0).unwrap();
                if class == "diode" {
                    // The cathode band, at the K leg (DIYLC's second).
                    writeln!(parts, r#"<rect x="{:.1}" y="{:.1}" width="4" height="{width:.1}" class="band"/>"#, length / 2.0 - 9.0, -width / 2.0).unwrap();
                }
                writeln!(parts, "</g>").unwrap();
                // Turned so it never reads upside down; a standing one reads from the bottom up.
                let upright = if angle > -90.5 && angle < 89.5 { angle } else { angle - 180.0 };
                writeln!(labels, r#"<text x="{mx:.1}" y="{:.1}" class="part-name" transform="rotate({upright:.0} {mx:.1} {my:.1})">{}</text>"#, my + 3.5, if class == "diode" { reference.clone() } else { label.clone() }).unwrap();
            }
            // Standing up: a round body over its legs.
            "RadialElectrolytic" | "LED" if pins.len() == 2 => {
                let (cx, cy) = (ox + ow / 2.0, oy + oh / 2.0);
                let r = comp.w.max(comp.h) as f64 * PX / 2.0 * 0.95;
                let class = if shape(comp) == "LED" { "led" } else { "cap" };
                writeln!(parts, r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" class="{class}"><title>{label}</title></circle>"#).unwrap();
                // The + just past its leg, on the side away from the other leg.
                let plus = plus_leg(comp);
                let ((x1, y1), (x2, y2)) = (at(pins[plus]), at(pins[1 - plus]));
                let d = (x1 - x2).hypot(y1 - y2).max(1.0);
                let (lx, ly) = (x1 + (x1 - x2) / d * PX * 0.6, y1 + (y1 - y2) / d * PX * 0.6);
                writeln!(labels, r#"<text x="{lx:.1}" y="{:.1}" class="plus">+</text>"#, ly + 5.5).unwrap();
                above(&mut labels, cx, cy - r, "part-name", &label);
            }
            "RadialCeramicDiskCapacitor" | "RadialFilmCapacitor" => {
                writeln!(parts, r#"<rect {} rx="{:.1}" class="cap"><title>{label}</title></rect>"#, rect(outline, 0.4), PX * 0.4).unwrap();
                above(&mut labels, ox + ow / 2.0, oy, "part-name", &label);
            }
            // A module: its own board, dashed, with a header box around each group of its pins.
            "module" => {
                writeln!(base, r#"<rect {} class="module"><title>{reference}</title></rect>"#, rect(outline, 0.45)).unwrap();
                let (nx, anchor) = if mirror { (ox + ow - 4.0, "end") } else { (ox + 4.0, "start") };
                writeln!(names, r#"<text x="{nx:.1}" y="{:.1}" text-anchor="{anchor}" class="module-name">{reference}</text>"#, oy + 12.0).unwrap();
            }
            "PinHeader" => {}
            // Anything else (chips, transistors, switches, boxes): its outline.
            _ => {
                writeln!(parts, r#"<rect {} class="package"><title>{label}</title></rect>"#, rect(outline, 0.45)).unwrap();
                above(&mut labels, ox + ow / 2.0, oy, "part-name", &reference);
            }
        }
        // Pin names, on headers and parts with more than two pins (a resistor's "1" and "2" say
        // nothing): grouped by header ("ESP32C6-L.5v" is on ESP32C6-L), each header boxed and
        // named. A row of pins has its names turned to fit.
        if pins.len() > 2 || matches!(shape(comp), "PinHeader" | "module") {
            // A module from footprints.json has plain pin numbers, no header names: its headers are
            // the lines of pins in its footprint, each named after the part. Columns for a module
            // whose rows run down (a DRV8833 breakout), rows for one whose rows run across (an
            // ESP32-C6-Zero lying across): whichever makes fewer lines.
            let unnamed_module = shape(comp) == "module" && comp.pins.iter().all(|pin| split_pin(&pin.name).0.is_empty());
            let lines = |axis: fn(Point) -> i32| comp.pins.iter().map(|pin| axis(pin.at)).collect::<HashSet<i32>>().len();
            let by_row = lines(|p| p.1) < lines(|p| p.0);
            let mut groups: Vec<(String, &str, Vec<usize>)> = Vec::new(); // (key, name, pins)
            for (pi, pin) in comp.pins.iter().enumerate() {
                let line = if by_row { format!("row {}", pin.at.1) } else { format!("column {}", pin.at.0) };
                let (key, group) = if unnamed_module { (line, "") } else { (split_pin(&pin.name).0.to_string(), split_pin(&pin.name).0) };
                match groups.iter_mut().find(|(k, _, _)| *k == key) {
                    Some((_, _, members)) => members.push(pi),
                    None => groups.push((key, group, vec![pi])),
                }
            }
            for (_, group, members) in groups {
                let holes: Vec<Point> = members.iter().map(|&pi| pins[pi]).collect();
                let in_a_row = holes.len() > 1 && holes.iter().all(|p| p.1 == holes[0].1);
                for (&pi, &p) in members.iter().zip(&holes) {
                    let (x, y) = at(p);
                    let turn = if in_a_row { format!(r#" transform="rotate(-90 {x:.1} {y:.1})""#) } else { String::new() };
                    writeln!(labels, r#"<text x="{x:.1}" y="{:.1}" class="pin"{turn}>{}</text>"#, y + 3.0, esc(split_pin(&comp.pins[pi].name).1)).unwrap();
                }
                let shape = shape(comp);
                if shape == "module" || shape == "PinHeader" {
                    let (hx, hy, hw, _) = bounds(&holes, 0.45);
                    writeln!(parts, r#"<rect {} class="header"><title>{}</title></rect>"#, rect(&holes, 0.45), esc(group)).unwrap();
                    let name = if shape == "module" && !group.is_empty() { esc(group) } else { reference.clone() };
                    above(&mut labels, hx + hw / 2.0, hy, "header-name", &name);
                }
            }
        }
        if clashes(i) {
            writeln!(labels, r#"<rect {} class="clash"><title>{reference} is where it can't be</title></rect>"#, rect(outline, 0.5)).unwrap();
        }
    }

    // --- jumpers, on the parts' side, over everything
    for &(run, a, b) in &routing.jumpers {
        let ((x1, y1), (x2, y2)) = (at(a), at(b));
        let line = format!(r#"x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}""#);
        writeln!(parts, r#"<line {line} class="jumper-halo"/>"#).unwrap();
        writeln!(parts, r#"<line {line} class="jumper"><title>jumper, {}</title></line>"#, esc(&c.nets[c.runs[run].net].name)).unwrap();
    }

    // --- the legend, beside the board
    let (lx, mut ly) = (right + MARGIN, top + 6.0);
    let line_h = 11.0 * 1.3;
    let mut legend = String::new();
    let line = |legend: &mut String, ly: &mut f64, text: &str, class: &str| {
        writeln!(legend, r#"<text x="{lx:.1}" y="{:.1}" class="{class}">{}</text>"#, *ly, esc(text)).unwrap();
        *ly += line_h;
    };
    let side_name = if mirror { "solder side, mirrored" } else { "component side" };
    // Wrapped: with a one-of in the board file, the title names the case, and gets long.
    for text in wrap(&format!("{title}, {side_name}"), 48) {
        line(&mut legend, &mut ly, &text, "legend-title");
    }
    // How it was made, so a picture you like can be made again.
    for text in wrap(&build.made.again(), 60) {
        line(&mut legend, &mut ly, &text, "note");
    }
    if misplaced.is_empty() {
        line(&mut legend, &mut ly, &format!("score {:.0}/100, cost {:.1} (lower is better)", board::score(&m), board::cost(&m)), "legend");
        let measures: Vec<String> = board::FEATURES.iter().zip(m.0).map(|(k, v)| format!("{k} {}", board::shown(k, v))).collect();
        // Wrapped too: eleven measurements don't fit on one line beside the board.
        for text in wrap(&measures.join(" · "), 60) {
            line(&mut legend, &mut ly, &text, "note");
        }
    } else {
        // Like the text report: a layout that can't be built isn't scored (a sketch as you drew it).
        line(&mut legend, &mut ly, "not allowed, so not scored", "legend");
    }
    ly += line_h * 0.5;
    if (0..c.components.len()).any(clashes) {
        for text in wrap("Boxed in red: a part that collides, hangs off the board, sits on a hard keep-out, or doesn't fit under a module.", 60) {
            line(&mut legend, &mut ly, &text, "warn");
        }
        ly += line_h * 0.5;
    }
    // Supply and ground first, then the other nets by name.
    let is_supply = |name: &String| c.power.as_ref() == Some(name) || c.ground.as_ref() == Some(name);
    let mut shown: Vec<usize> = (0..c.nets.len()).collect();
    shown.sort_by_key(|&n| (!is_supply(&c.nets[n].name), c.power.as_ref() != Some(&c.nets[n].name), c.nets[n].name.clone()));
    let swatch = 11.0 * 1.25;
    for n in shown {
        let text = format!("{}: {}", c.nets[n].name, c.nets[n].pins.join(" "));
        writeln!(legend, r#"<rect x="{lx:.1}" y="{:.1}" width="{swatch:.1}" height="{swatch:.1}" rx="{:.1}" fill="{}"/>"#, ly - swatch * 0.8, swatch / 4.0, colors[n]).unwrap();
        let lines = wrap(&text, 54);
        for (k, text) in lines.iter().enumerate() {
            writeln!(legend, r#"<text x="{:.1}" y="{:.1}" class="legend">{}</text>"#, lx + swatch * 1.6, ly + k as f64 * line_h, esc(text)).unwrap();
        }
        ly += line_h * lines.len() as f64 + line_h * 0.45;
    }
    let alone = (0..c.components.len()).flat_map(|ci| (0..c.components[ci].pins.len()).map(move |pi| (ci, pi))).filter(|p| !net_of.contains_key(p)).count();
    if alone > 0 {
        writeln!(legend, r#"<rect x="{lx:.1}" y="{:.1}" width="{swatch:.1}" height="{swatch:.1}" rx="{:.1}" fill="{LONE}"/>"#, ly - swatch * 0.8, swatch / 4.0).unwrap();
        writeln!(legend, r#"<text x="{:.1}" y="{ly:.1}" class="legend">connected to nothing: {alone} pins</text>"#, lx + swatch * 1.6).unwrap();
        ly += line_h * 1.45;
    }
    ly += line_h * 0.5;
    let notes = [
        if mirror {
            "As held to solder: mirrored, so column 0 is on the right. The copper is in front; the parts and jumpers behind the board are faint."
        } else {
            "Thick lines are copper traces on the solder side, seen through the board. Blue lines are jumpers on the parts' side."
        },
        "Dashed red lines are wires on the solder side, the last resort where neither a trace nor a jumper gets through.",
        "A thin dotted grey line is a connection the router couldn't make at all: the pin is walled in, or off the board. It's not in the .diy.",
        "Hole numbers are the text board's, and the board file's.",
        "Hover a hole for its position, pin and net.",
    ];
    for note in notes {
        for text in wrap(note, 60) {
            line(&mut legend, &mut ly, &text, "note");
        }
    }

    let (width, height) = (right + MARGIN + LEGEND_W, (bottom + MARGIN).max(ly + MARGIN));
    // From the parts' side the parts are on top; from the solder side the wires are in front.
    let layers = if mirror { [base, format!("<g opacity=\"0.3\">\n{parts}</g>\n"), wires, holes] } else { [base, wires, holes, parts] };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\">\n\
         <title>{}, {side_name}</title>\n<style>{STYLE}</style>\n<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n{}{labels}{names}{legend}</svg>\n",
        esc(title),
        layers.concat()
    )
}
