//! Writes a layout (src/layout.rs) as a DIYLC project (.diy) that opens in DIY Layout Creator.
//!
//! A .diy file is XML: a perfboard, then each part with the positions of its legs, in
//! inches (one hole = 0.1"). Which DIYLC part to draw comes from the "diylc" entry in
//! footprints.json; a footprint without one is drawn as a labelled box. The routed
//! copper becomes DIYLC copper traces (solder side) and jumpers, coloured by net.

use crate::board::{self, Place, Point};
use crate::layout::Layout;
use serde::{Deserialize, Serialize};
use std::fmt::Write;

/// The "diylc" entry of a footprint in footprints.json (the layout file keeps it too).
#[derive(Clone, Serialize, Deserialize)]
pub struct Part {
    /// DIYLC's name for the part: Resistor, RadialElectrolytic, RadialCeramicDiskCapacitor, LED, DIL_IC or PinHeader.
    pub part: String,
    /// For two-legged parts: which pad becomes DIYLC's first leg and which its second (default "1", "2").
    /// It matters for polarised parts: DIYLC's first leg is + on a capacitor, but the anode on an LED.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<String>,
}

/// Two-legged DIYLC parts: (DIYLC class, body size, body colour, border colour, label colour, extra settings).
/// The sizes, colours and settings are what DIYLC 6.6 itself writes for a new part.
const TWO_LEGGED: [(&str, &str, &str, &str, &str, &str); 4] = [
    ("passive.Resistor", r#"<length value="6.3" unit="mm"/><width value="2.5" unit="mm"/>"#, "82cffd", "5b90b1", "000000",
        "<power>HALF</power><colorCode>_5_BAND</colorCode><shape>Standard</shape>"),
    ("passive.RadialElectrolytic", r#"<length value="5.0" unit="mm"/>"#, "6b6dce", "4a4c90", "ffffff",
        r#"<pinSpacing value="SPACING" unit="in"/><voltage>_63V</voltage><markerColor hex="8cacea"/><tickColor hex="ffffff"/><polarized>true</polarized><folded>false</folded><height value="0.4" unit="in"/><invert>false</invert>"#),
    ("passive.RadialCeramicDiskCapacitor", r#"<length value="5.0" unit="mm"/><width value="2.5" unit="mm"/>"#, "f0e68c", "a8a162", "000000",
        r#"<pinSpacing value="SPACING" unit="in"/><voltage>_63V</voltage>"#),
    ("semiconductors.LED", r#"<length value="5.0" unit="mm"/><width value="5.0" unit="mm"/>"#, "5dfc0a", "41b007", "000000", "<value></value>"),
];

/// Hole -> inches. Hole (0, 0) sits one grid step in from the board's corner, as in DIYLC.
fn inch(hole: i32) -> f64 {
    inch_f(hole as f64)
}

/// Same, for points between holes (like a label in the middle of a resistor).
fn inch_f(hole: f64) -> f64 {
    0.2 + hole * 0.1
}

/// One item of your DIYLC drawing, moved to where the placer put its part: every
/// x="..." y="..." point is turned and shifted, and any <orientation> turned to match.
fn moved(xml: &str, comp: &crate::board::Component, src: &crate::diylc_import::Source, place: &Place) -> String {
    let to_new = |x: f64, y: f64| {
        // Sketch inches -> holes inside the part's outline -> turned -> holes on the board -> inches.
        let (mut lx, mut ly) = (x / 0.1 - src.origin.0 as f64, y / 0.1 - src.origin.1 as f64);
        let (mut w, mut h) = (comp.w as f64, comp.h as f64);
        for _ in 0..place.rot % 4 {
            (lx, ly, w, h) = (h - 1.0 - ly, lx, h, w);
        }
        (inch_f(place.x as f64 + lx), inch_f(place.y as f64 + ly))
    };
    // Finds the next x="..." y="..." pair: (text before it, x, y, text after it).
    fn next_point(rest: &str) -> Option<(&str, f64, f64, &str)> {
        let i = rest.find(" x=\"")?;
        let (x, tail) = rest[i + 4..].split_once('"')?;
        let (y, tail) = tail.strip_prefix(" y=\"")?.split_once('"')?;
        Some((&rest[..i], x.parse().ok()?, y.parse().ok()?, tail))
    }
    let mut moved_points = Vec::new();
    let mut rest = xml;
    while let Some((_, x, y, tail)) = next_point(rest) {
        moved_points.push(to_new(x, y));
        rest = tail;
    }
    // A box (a module's board outline) is two corners, given twice. After turning, the corners
    // may come out the wrong way round, which DIYLC won't draw: put top-left first again.
    if xml.contains("<mode>TwoPoints</mode>") && moved_points.len() == 4 {
        let (a, b) = (moved_points[0], moved_points[1]);
        let (lo, hi) = ((a.0.min(b.0), a.1.min(b.1)), (a.0.max(b.0), a.1.max(b.1)));
        moved_points = vec![lo, hi, lo, hi];
    }
    let mut out = String::new();
    let mut rest = xml;
    let mut k = 0;
    while let Some((before, _, _, tail)) = next_point(rest) {
        let (nx, ny) = moved_points[k];
        write!(out, "{before} x=\"{nx:.3}\" y=\"{ny:.3}\"").unwrap();
        k += 1;
        rest = tail;
    }
    out += rest;
    for (i, o) in ORIENTATIONS.iter().enumerate() {
        let old = format!("<orientation>{o}</orientation>");
        if out.contains(&old) {
            let new = format!("<orientation>{}</orientation>", ORIENTATIONS[(i + place.rot as usize) % 4]);
            out = out.replace(&old, &new);
            break;
        }
    }
    // A label's font can point at another label's ("reference=..."), which breaks once things move: use a plain one.
    while let Some(i) = out.find("<font reference=\"") {
        let end = i + out[i..].find("/>").unwrap() + 2;
        out.replace_range(i..end, r#"<font name="Dialog" size="10" style="0"/>"#);
    }
    out
}

fn point(s: &mut String, (x, y): (f64, f64)) {
    writeln!(s, r#"<point x="{x:.2}" y="{y:.2}"/>"#).unwrap();
}

/// DIYLC turns parts clockwise in 90 degree steps, like our placer does.
const ORIENTATIONS: [&str; 4] = ["DEFAULT", "_90", "_180", "_270"];

pub fn write(layout: &Layout) -> String {
    let (c, placement, title) = (&layout.circuit, &layout.placement, layout.title.as_str());
    let g = board::geometry(c, placement);
    let mut ids = 0..;
    let mut id = || format!("00000000-0000-4000-8000-{:012x}", ids.next().unwrap());
    let to_inch = |(x, y): Point| (inch(x), inch(y));
    let (w, h) = (c.board.width, c.board.height);

    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<project>\n");
    s += "<fileVersion><major>6</major><minor>6</minor><build>0</build></fileVersion>\n";
    writeln!(s, "<title>{}</title>\n<author>perfboard</author>", xml(title)).unwrap();
    // The page, in cm, with a little room around the board.
    writeln!(s, r#"<width value="{:.2}" unit="cm"/><height value="{:.2}" unit="cm"/>"#, (w + 3) as f64 * 0.254, (h + 3) as f64 * 0.254).unwrap();
    s += "<gridSpacing value=\"0.1\" unit=\"in\"/>\n<dotSpacing>1</dotSpacing>\n<components>\n";

    // The perfboard: its corners are one grid step outside the outer holes.
    let (x0, y0, x1, y1) = (0.1, 0.1, 0.1 + (w + 1) as f64 * 0.1, 0.1 + (h + 1) as f64 * 0.1);
    writeln!(s, "<diylc.boards.PerfBoard><id>{}</id><name>Board1</name><alphaPercent><value>100</value></alphaPercent><value></value><controlPoints>", id()).unwrap();
    point(&mut s, (x0, y0));
    point(&mut s, (x1, y1));
    s += "</controlPoints>";
    writeln!(s, r#"<firstPoint x="{x0:.2}" y="{y0:.2}"/><secondPoint x="{x1:.2}" y="{y1:.2}"/>"#).unwrap();
    s += r#"<boardColor hex="f8ebb3"/><borderColor hex="ada47d"/><coordinateColor hex="b6b6b6"/><xType>Numbers</xType><coordinateOrigin>Top_Left</coordinateOrigin><coordinateDisplay>One_Side</coordinateDisplay><yType>Letters</yType><mode>TwoPoints</mode><boardUndersideDisplay>NONE</boardUndersideDisplay><undersideOffset value="0.1" unit="in"/><undersideTransparency>true</undersideTransparency><spacing value="0.1" unit="in"/><padColor hex="da8a67"/>"#;
    s += "\n</diylc.boards.PerfBoard>\n";

    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (ci, (comp, place)) in c.components.iter().zip(placement).enumerate() {
        // A part that came from your own DIYLC file: write back your drawing, moved and turned.
        if let Some(src) = &comp.source {
            for xml in &src.xml {
                s += &moved(xml, comp, src, place);
                s += "\n";
            }
            groups.extend(src.group.clone());
            continue;
        }
        let pins = &g.pins[ci];
        let pin = |name: &str| {
            let i = comp.pins.iter().position(|p| p.name == name);
            pins[i.unwrap_or_else(|| panic!("footprints.json: {} has no pad {name} (see its \"legs\")", comp.reference))]
        };
        let orientation = ORIENTATIONS[place.rot as usize % 4];
        // DIYLC lists a chip's or header's pins column by column (left to right, top to bottom), before rotating.
        let mut by_column: Vec<usize> = (0..comp.pins.len()).collect();
        by_column.sort_by_key(|&i| comp.pins[i].at);

        match comp.diylc.as_ref().map(|p| p.part.as_str()) {
            Some("DIL_IC") => {
                writeln!(s, "<diylc.semiconductors.DIL__IC><id>{}</id><name>{}</name><alphaPercent><value>100</value></alphaPercent><value>{}</value>", id(), xml(&comp.reference), xml(&comp.value)).unwrap();
                writeln!(s, r#"<orientation>{orientation}</orientation><pinCount>_{}</pinCount><pinSpacing value="0.1" unit="in"/><rowSpacing value="{:.1}" unit="in"/><controlPoints>"#, comp.pins.len(), (comp.w - 1) as f64 * 0.1).unwrap();
                by_column.iter().for_each(|&i| point(&mut s, to_inch(pins[i])));
                s += r#"</controlPoints><display>BOTH</display><bodyColor hex="808080"/><borderColor hex="595959"/><labelColor hex="ffffff"/><indentColor hex="595959"/><displayNumbers>NO</displayNumbers>"#;
                s += "\n</diylc.semiconductors.DIL__IC>\n";
            }
            Some("PinHeader") => {
                writeln!(s, "<diylc.electromechanical.PinHeader><id>{}</id><name>{}</name><alphaPercent><value>100</value></alphaPercent><orientation>{orientation}</orientation><controlPoints>", id(), xml(&comp.reference)).unwrap();
                by_column.iter().for_each(|&i| point(&mut s, to_inch(pins[i])));
                // DIYLC's "rows" run across, its "columns" down.
                writeln!(s, r#"</controlPoints><rowCount>{}</rowCount><columnCount>{}</columnCount><spacing value="0.1" unit="in"/><shrouded>false</shrouded><nodeNames></nodeNames><bodyColor hex="808080"/><shroudColor hex="595959"/>"#, comp.w, comp.h).unwrap();
                s += "</diylc.electromechanical.PinHeader>\n";
            }
            Some(part) => {
                let kind = TWO_LEGGED.iter().find(|k| k.0.ends_with(&format!(".{part}")));
                let (class, size, body, border, label, extra) = *kind.unwrap_or_else(|| {
                    panic!("footprints.json: {} wants DIYLC part {part}, which isn't supported (see src/diylc.rs)", comp.reference)
                });
                let legs = &comp.diylc.as_ref().unwrap().legs;
                let (first, second) = match legs.as_slice() {
                    [] => (pin("1"), pin("2")),
                    [a, b] => (pin(a), pin(b)),
                    _ => panic!("footprints.json: \"legs\" for {} should name two pads", comp.reference),
                };
                let spacing = ((first.0 - second.0).abs() + (first.1 - second.1).abs()) as f64 * 0.1;
                let extra = extra.replace("SPACING", &format!("{spacing:.1}"));
                two_legged(&mut s, &id(), class, &comp.reference, size, [body, border, label, "cccccc"], to_inch(first), to_inch(second), "NAME", &extra);
            }
            None => {
                // No DIYLC part for this footprint: a box over its outline, with its name on it; for a
                // module, see-through, since low parts sit under its board.
                let outline = &g.outlines[ci];
                let (x0, x1) = (outline.iter().map(|p| p.0).min().unwrap(), outline.iter().map(|p| p.0).max().unwrap());
                let (y0, y1) = (outline.iter().map(|p| p.1).min().unwrap(), outline.iter().map(|p| p.1).max().unwrap());
                let alpha = if g.bodies[ci].len() < outline.len() { 30 } else { 100 };
                labelled_box(&mut s, [id(), id()], &comp.reference, ((x0, y0), (x1, y1)), "ffffff", alpha);
            }
        }
    }

    // Keep-outs from the board file: see-through grey boxes, labelled with their name.
    for k in &c.constraints.keepout {
        let label = if k.name.is_empty() { "keep-out" } else { &k.name };
        labelled_box(&mut s, [id(), id()], label, (k.from, k.to), "999999", 40);
    }

    // The routed copper: traces on the solder side and jumpers on top, coloured like their net.
    let routing = &layout.routing;
    let colors = board::net_colors(c);
    let copper = routing.straight_traces().into_iter().map(|t| (t, "connectivity.CopperTrace", r#"<thickness value="1.0" unit="mm"/><layer>_1</layer><endingStyle>Round</endingStyle>"#));
    let jumpers = routing.jumpers.iter().map(|&t| (t, "connectivity.Jumper", "<style>SOLID</style>"));
    for ((run, a, b), class, extra) in copper.chain(jumpers) {
        let net = c.runs[run].net;
        let color = &colors[net][1..]; // "#d62828" -> "d62828"
        two_legged(&mut s, &id(), class, &c.nets[net].name, "", ["ffffff", "000000", "000000", color], to_inch(a), to_inch(b), "NONE", extra);
    }
    // DIYLC has no wire on the solder side, so a jumper that says so in its name, in red.
    for &(run, a, b) in &routing.bottom {
        let name = format!("{} (solder side)", c.nets[c.runs[run].net].name);
        two_legged(&mut s, &id(), "connectivity.Jumper", &name, "", ["ffffff", "000000", "000000", "b00020"], to_inch(a), to_inch(b), "NAME", "<style>SOLID</style>");
    }

    s += "</components>\n<groups/>\n<groupsEx>\n";
    for (name, ids) in groups {
        s += "<group><componentIds>";
        for id in ids {
            writeln!(s, "<uuid>{}</uuid>", xml(&id)).unwrap();
        }
        writeln!(s, "</componentIds><name>{}</name></group>", xml(&name)).unwrap();
    }
    s += "</groupsEx>\n<lockedLayers/>\n<lockedComponents/>\n<hiddenLayers/>\n<font name=\"Dialog\" size=\"14\" style=\"0\"/>\n</project>\n";
    s
}

/// A part with two legs, like a resistor or a jumper: leg 1, leg 2, and its label in the middle.
fn two_legged(
    s: &mut String,
    id: &str,
    class: &str,
    name: &str,
    size: &str,
    [body, border, label, lead]: [&str; 4],
    first: (f64, f64),
    second: (f64, f64),
    display: &str,
    extra: &str,
) {
    writeln!(s, "<diylc.{class}><id>{id}</id><name>{}</name><alphaPercent><value>100</value></alphaPercent>{size}<points>", xml(name)).unwrap();
    point(s, first);
    point(s, second);
    point(s, ((first.0 + second.0) / 2.0, (first.1 + second.1) / 2.0));
    writeln!(
        s,
        r#"</points><bodyColor hex="{body}"/><borderColor hex="{border}"/><labelColor hex="{label}"/><leadColor hex="{lead}"/><display>{display}</display><flipStanding>false</flipStanding><labelOriantation>Directional</labelOriantation><moveLabel>false</moveLabel><hideShortLeads>false</hideShortLeads>{extra}"#
    )
    .unwrap();
    writeln!(s, "</diylc.{class}>").unwrap();
}

/// A box covering the holes from one corner to the other, with a label in the middle.
/// `alpha` is how solid it looks, 0-100.
fn labelled_box(s: &mut String, ids: [String; 2], label: &str, (a, b): (Point, Point), fill: &str, alpha: u8) {
    let (x0, y0) = (inch(a.0.min(b.0)) - 0.05, inch(a.1.min(b.1)) - 0.05);
    let (x1, y1) = (inch(a.0.max(b.0)) + 0.05, inch(a.1.max(b.1)) + 0.05);
    writeln!(s, "<diylc.shapes.Rectangle><id>{}</id><name>{}</name><alphaPercent><value>{alpha}</value></alphaPercent><value></value><controlPoints>", ids[0], xml(label)).unwrap();
    point(s, (x0, y0));
    point(s, (x1, y1));
    writeln!(s, r#"</controlPoints><firstPoint x="{x0:.2}" y="{y0:.2}"/><secondPoint x="{x1:.2}" y="{y1:.2}"/><color hex="{fill}"/><borderColor hex="000000"/><borderThickness value="0.2" unit="mm"/><mode>TwoPoints</mode><edgeRadius value="0.0" unit="mm"/>"#).unwrap();
    *s += "</diylc.shapes.Rectangle>\n";
    writeln!(s, "<diylc.misc.Label><id>{}</id><name>{}</name>", ids[1], xml(label)).unwrap();
    point(s, ((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    writeln!(s, r#"<text>{}</text><font name="Dialog" size="10" style="0"/><color hex="000000"/><horizontalAlignment>CENTER</horizontalAlignment><verticalAlignment>CENTER</verticalAlignment><orientation>DEFAULT</orientation>"#, xml(label)).unwrap();
    *s += "</diylc.misc.Label>\n";
}

/// Text made safe to put inside XML.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
