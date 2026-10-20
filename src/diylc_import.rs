//! Reads a DIYLC layout (.diy) as a rough sketch: which parts there are, and which pins connect.
//!
//! You put the parts down roughly in DIYLC and connect them with copper traces, jumpers or
//! wires. This works out the connections the way DIYLC does, from the drawing: a pin
//! touching a trace is on it, traces that touch are joined, and a wire joins whatever
//! is at its two ends. Where you drew things doesn't matter beyond that; the placer
//! re-places everything on the real board.
//!
//! A DIYLC group with pins in it (say two pin headers plus a rectangle for a module's
//! board) becomes one part. Only its pins' holes are taken, so low parts can go under it.

use crate::board::{BoardSize, Circuit, Component, Net, Pin, Place, Placement, Point};
use crate::netlist::{self, NET_NAME_HINTS, Options};
use roxmltree::Node;
use std::collections::{BTreeSet, HashMap};

/// DIYLC measures in inches; one perfboard hole is 0.1".
const HOLE: f64 = 0.1;
/// How close (in inches) two things must be to count as touching.
const EPS: f64 = 0.005;
/// Half the side of the square around a pin or wire end that copper must overlap to touch it.
/// DIYLC uses a 4 x 4 pixel square at 200 pixels per inch, so 0.01" each way.
const PIN_REACH: f64 = 0.01;

/// The original drawing of a part, kept so it can be written back to DIYLC moved and turned
/// (the layout file keeps it too, as the part's "diylc_drawing").
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Source {
    /// The XML of each DIYLC item the part is made of, exactly as in the file.
    pub xml: Vec<String>,
    /// The DIYLC group it was, if any: (group name, ids of its items).
    pub group: Option<(String, Vec<String>)>,
    /// The sketch hole that is the part's top-left corner.
    pub origin: Point,
}

/// What a DIYLC item is, as far as connections go.
enum Kind {
    /// A part with pins: DIYLC's names for its pins, in order (None = number them 1, 2, ...).
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

fn kind(class: &str) -> Option<Kind> {
    const TWO: &[&str] = &["1", "2"];
    Some(match class {
        "passive.Resistor" | "passive.RadialCeramicDiskCapacitor" | "passive.RadialFilmCapacitor" | "passive.AxialFilmCapacitor" => Kind::Part(Some(TWO)),
        "passive.RadialElectrolytic" => Kind::Part(Some(&["+", "-"])),
        "semiconductors.LED" | "semiconductors.DiodePlastic" => Kind::Part(Some(&["A", "K"])),
        "semiconductors.DIL__IC" | "semiconductors.TransistorTO92" | "semiconductors.TransistorTO126" | "semiconductors.TransistorTO220"
        | "electromechanical.PinHeader" | "electromechanical.TactileMicroSwitch" => Kind::Part(None),
        "connectivity.CopperTrace" => Kind::Trace,
        "connectivity.Jumper" | "connectivity.HookupWire" | "connectivity.OrthogonalWire" | "connectivity.TwistedWire" => Kind::Wire,
        "connectivity.SolderPad" => Kind::Pad,
        "misc.Label" | "misc.WrapLabel" | "misc.PCBText" => Kind::Decoration,
        c if c.starts_with("shapes.") || c.starts_with("chassis.") => Kind::Decoration,
        "boards.PerfBoard" => Kind::Board,
        _ => return None,
    })
}

/// Parts low enough to sit under a module on headers: ones that lie flat, and small ceramic discs.
fn is_low(class: &str) -> bool {
    matches!(class, "passive.Resistor" | "passive.RadialCeramicDiskCapacitor" | "passive.AxialFilmCapacitor" | "semiconductors.DiodePlastic")
}

/// Radial parts stand up, so their body is a circle around the legs; axial ones lie between their legs.
fn is_radial(class: &str) -> bool {
    matches!(class, "passive.RadialElectrolytic" | "passive.RadialCeramicDiskCapacitor" | "passive.RadialFilmCapacitor" | "semiconductors.LED")
}

type Inches = (f64, f64);

fn hole((x, y): Inches) -> Point {
    ((x / HOLE).round() as i32, (y / HOLE).round() as i32)
}

/// True for a pin that isn't on a hole, like a tactile switch's legs (0.175" and 0.255" apart).
fn off_grid((x, y): Inches) -> bool {
    let off = |v: f64| (v / HOLE - (v / HOLE).round()).abs() > 0.01;
    off(x) || off(y)
}

/// One part as drawn in the sketch, in sketch holes.
struct Drawn<'a> {
    id: String,
    name: String,
    class: String,
    value: String,
    pins: Vec<(String, Inches)>,
    body: Vec<Point>,
    node: Node<'a, 'a>,
}

/// The copper you drew, in holes of the board, each piece with the index of its net in the
/// circuit: what `route::from_sketch` turns into a routing, to score your own wiring the way the
/// router's is scored. A piece that joins no pins is left out.
#[derive(Default, Clone)]
pub struct Copper {
    /// Copper traces, (net, one end, the other), as drawn: a trace may be many holes long.
    pub traces: Vec<(usize, Point, Point)>,
    /// Jumpers and wires on the parts' side.
    pub jumpers: Vec<(usize, Point, Point)>,
    /// Wires on the solder side: a jumper whose name says "(solder side)", as this program writes them.
    pub bottom: Vec<(usize, Point, Point)>,
}

/// Everything that conducts, for working out the nets.
#[derive(Clone, Copy)]
enum Conductor {
    Pin(usize, usize),
    /// Two ends, half the width, copper layer.
    Trace(Inches, Inches, f64, u8),
    /// Two ends, and whether it's on the solder side.
    Wire(Inches, Inches, bool),
    /// Centre, radius, copper layer.
    Pad(Inches, f64, u8),
}

pub fn load(path: &str, opts: Options) -> (Circuit, Placement, Copper) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {path}: {e}"));
    let doc = roxmltree::Document::parse(&text).unwrap_or_else(|e| panic!("bad DIYLC file {path}: {e}"));
    let root = doc.root_element();
    let items: Vec<Node> = child(root, "components").map(|c| c.children().filter(|n| n.is_element()).collect()).unwrap_or_default();

    let mut parts: Vec<Drawn> = Vec::new();
    let mut conductors: Vec<Conductor> = Vec::new();
    let mut decorations: HashMap<String, Node> = HashMap::new();
    let mut board_origin: Option<Point> = None;
    for item in items {
        if item.attribute("reference").is_some() {
            continue; // XStream's pointer back to an item already listed
        }
        let class = item.tag_name().name().strip_prefix("diylc.").unwrap_or(item.tag_name().name()).to_string();
        let name = text_of(item, "name").unwrap_or_default();
        let points = points(item);
        let k = kind(&class).unwrap_or_else(|| {
            panic!("{path}: {name} is a DIYLC {class}, which isn't supported yet, so its connections would be missed")
        });
        match k {
            Kind::Part(names) => {
                let count = if names.is_some() { 2 } else { points.len() };
                let pin_names: Vec<String> = match (names, header_names(item, count)) {
                    (Some(n), _) => n.iter().map(|s| s.to_string()).collect(),
                    (None, Some(n)) => n,
                    (None, None) => (1..=count).map(|i| i.to_string()).collect(),
                };
                let pins: Vec<(String, Inches)> = pin_names.into_iter().zip(points.iter().copied()).collect();
                let body = body_of(&class, item, &pins);
                parts.push(Drawn { id: text_of(item, "id").unwrap_or_default(), name, class, value: value_of(item), pins, body, node: item });
            }
            Kind::Trace => conductors.push(Conductor::Trace(points[0], points[1], size_of(item, "thickness").unwrap_or(0.04) / 2.0, layer_of(item))),
            Kind::Wire => {
                let end = if class == "connectivity.Jumper" { points[1] } else { *points.last().unwrap() };
                conductors.push(Conductor::Wire(points[0], end, name.ends_with("(solder side)")));
            }
            Kind::Pad => conductors.push(Conductor::Pad(points[0], size_of(item, "size").unwrap_or(0.09) / 2.0, layer_of(item))),
            Kind::Decoration => {
                decorations.insert(text_of(item, "id").unwrap_or_default(), item);
            }
            Kind::Board => {
                // DIYLC puts the first hole one grid step in from the board's corner.
                let (x, y) = hole(points[0]);
                board_origin.get_or_insert((x + 1, y + 1));
            }
        }
    }
    for (pi, p) in parts.iter().enumerate() {
        if p.name.contains('.') {
            panic!("{path}: part name {} has a '.' in it, which would confuse pin names like R1.2", p.name);
        }
        for i in 0..p.pins.len() {
            conductors.push(Conductor::Pin(pi, i));
        }
    }

    let (nets, net_of) = nets(&parts, &conductors);
    let (components, places, owner) = components(path, &root, &parts, &decorations);
    // A pin's name in the circuit: "R1.2" for a part on its own, "ESP32C6.ESP32C6-L.5v" in a module.
    let label = |pi: usize, i: usize| {
        let (ci, in_group) = owner[pi];
        let pin = &parts[pi].pins[i].0;
        if in_group { format!("{}.{}.{pin}", components[ci].reference, parts[pi].name) } else { format!("{}.{pin}", components[ci].reference) }
    };
    let nets = name_nets(nets.iter().map(|members| members.iter().map(|&(pi, i)| label(pi, i)).collect()).collect());

    // Where you drew each part becomes the placer's starting point, measured from the first hole of your board.
    let origin = board_origin.unwrap_or_else(|| {
        let all = places.iter();
        (all.clone().map(|p| p.0).min().unwrap_or(0), all.map(|p| p.1).min().unwrap_or(0))
    });
    let sketch: Placement = places.iter().map(|&(x, y)| Place { x: x - origin.0, y: y - origin.1, rot: 0 }).collect();

    // Your copper, in the same holes as the sketch.
    let on_board = |p: Inches| {
        let (x, y) = hole(p);
        (x - origin.0, y - origin.1)
    };
    let mut copper = Copper::default();
    for (conductor, net) in conductors.iter().zip(&net_of) {
        match (*conductor, *net) {
            (Conductor::Trace(a, b, ..), Some(net)) => copper.traces.push((net, on_board(a), on_board(b))),
            (Conductor::Wire(a, b, bottom), Some(net)) => {
                let list = if bottom { &mut copper.bottom } else { &mut copper.jumpers };
                list.push((net, on_board(a), on_board(b)));
            }
            _ => {}
        }
    }

    let (power, ground) = netlist::power_and_ground(&nets, &opts);
    let board: BoardSize = opts.board;
    (Circuit::new(board, components, nets, power, ground), sketch, copper)
}

/// Each net as its pins: (drawn part, pin).
type Nets = Vec<Vec<(usize, usize)>>;

/// Joins everything that touches, then lists each group of two or more pins; and for each
/// conductor, which of those groups it's in (None if it joins fewer than two pins).
fn nets(parts: &[Drawn], conductors: &[Conductor]) -> (Nets, Vec<Option<usize>>) {
    let n = conductors.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    // Each conductor as segments (a point is a segment of length 0), how far its copper reaches
    // from them (half its width), and its copper layer. None = a pin or wire end, which goes
    // through the board and so touches copper on any layer.
    let shape = |c: &Conductor| -> (Vec<(Inches, Inches)>, f64, Option<u8>) {
        match *c {
            Conductor::Pin(pi, i) => (vec![(parts[pi].pins[i].1, parts[pi].pins[i].1)], 0.0, None),
            Conductor::Trace(a, b, r, layer) => (vec![(a, b)], r, Some(layer)),
            // A wire only touches things at its ends; what it passes over isn't connected.
            Conductor::Wire(a, b, _) => (vec![(a, a), (b, b)], 0.0, None),
            Conductor::Pad(p, r, layer) => (vec![(p, p)], r, Some(layer)),
        }
    };
    let shapes: Vec<_> = conductors.iter().map(shape).collect();
    for i in 0..n {
        for j in i + 1..n {
            let ((si, ri, li), (sj, rj, lj)) = (&shapes[i], &shapes[j]);
            let touching = match (li, lj) {
                (Some(a), Some(b)) if a != b => false, // copper on different layers never touches
                (Some(_), Some(_)) => si.iter().any(|&a| sj.iter().any(|&b| segment_distance(a, b) <= ri + rj + EPS)),
                (None, None) => si.iter().any(|&a| sj.iter().any(|&b| segment_distance(a, b) <= EPS)),
                // A pin or wire end touches copper the way DIYLC decides it: the copper covers
                // the point, or overlaps a small square (2 x PIN_REACH wide) around it.
                (None, Some(_)) => si.iter().any(|&(p, _)| sj.iter().any(|&seg| square_to_segment(p, seg) <= *rj)),
                (Some(_), None) => sj.iter().any(|&(p, _)| si.iter().any(|&seg| square_to_segment(p, seg) <= *ri)),
            };
            if touching {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: HashMap<usize, BTreeSet<(usize, usize)>> = HashMap::new();
    for (i, c) in conductors.iter().enumerate() {
        if let Conductor::Pin(pi, pin) = *c {
            groups.entry(find(&mut parent, i)).or_default().insert((pi, pin));
        }
    }
    let mut out: Vec<(Vec<(usize, usize)>, usize)> = groups.into_iter().filter(|(_, g)| g.len() > 1).map(|(root, g)| (g.into_iter().collect(), root)).collect();
    out.sort();
    let net_of = (0..n).map(|i| {
        let root = find(&mut parent, i);
        out.iter().position(|&(_, r)| r == root)
    });
    let net_of = net_of.collect();
    (out.into_iter().map(|(g, _)| g).collect(), net_of)
}

/// Turns the drawn parts into circuit parts: one per DIYLC group with pins in it, one per other part.
/// Also returns each one's top-left hole in the sketch, and for each drawn part (its component, in a group?).
fn components(path: &str, root: &Node, parts: &[Drawn], decorations: &HashMap<String, Node>) -> (Vec<Component>, Vec<Point>, Vec<(usize, bool)>) {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(gx) = child(*root, "groupsEx") {
        for g in gx.children().filter(|n| n.has_tag_name("group")) {
            let ids = child(g, "componentIds").map(|c| c.children().filter(|n| n.has_tag_name("uuid")).filter_map(|n| n.text()).map(String::from).collect()).unwrap_or_default();
            groups.push((text_of(g, "name").unwrap_or_default(), ids));
        }
    }
    let mut owner: Vec<Option<(usize, bool)>> = vec![None; parts.len()];
    let mut out: Vec<Component> = Vec::new();
    let mut places: Vec<Point> = Vec::new();
    let add = |reference: String, members: Vec<usize>, extra: Vec<Node>, group: Option<(String, Vec<String>)>, out: &mut Vec<Component>, places: &mut Vec<Point>| {
        let in_group = group.is_some();
        // Holes of the module's board outline (rectangles in the group), if any.
        let mut outline: Vec<Point> = extra.iter().filter(|n| n.tag_name().name().starts_with("diylc.shapes.")).flat_map(|n| covered(&points(*n))).collect();
        let mut pins: Vec<(String, Point, bool)> = Vec::new();
        let mut body: Vec<Point> = Vec::new();
        for &pi in &members {
            let p = &parts[pi];
            for (name, at) in &p.pins {
                let name = if in_group { format!("{}.{name}", p.name) } else { name.clone() };
                pins.push((name, hole(*at), off_grid(*at)));
            }
            body.extend(&p.body);
        }
        body.sort();
        body.dedup();
        outline.extend(&body);
        let (x0, x1) = (outline.iter().map(|p| p.0).min().unwrap(), outline.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (outline.iter().map(|p| p.1).min().unwrap(), outline.iter().map(|p| p.1).max().unwrap());
        let local = |(x, y): Point| (x - x0, y - y0);
        let single = members.len() == 1 && !in_group;
        let first = &parts[members[0]];
        let xml = members.iter().map(|&pi| parts[pi].node).chain(extra).map(|n| n.document().input_text()[n.range()].to_string()).collect();
        out.push(Component {
            reference,
            value: if single { first.value.clone() } else { String::new() },
            footprint: format!("DIYLC:{}", if single { first.class.rsplit('.').next().unwrap() } else { "module" }),
            diylc: None,
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
            pins: pins.into_iter().map(|(name, at, off_grid)| Pin { name, at: local(at), off_grid }).collect(),
            body: body.into_iter().map(local).collect(),
            source: Some(Source { xml, group, origin: (x0, y0) }),
            low: single && is_low(&first.class),
            footprint_from: None,
        });
        places.push((x0, y0));
    };
    for (gname, ids) in &groups {
        let members: Vec<usize> = (0..parts.len()).filter(|&pi| owner[pi].is_none() && ids.contains(&parts[pi].id)).collect();
        if members.is_empty() {
            continue; // a group of labels and shapes only: nothing to place
        }
        let extra: Vec<Node> = ids.iter().filter_map(|id| decorations.get(id).copied()).collect();
        for &pi in &members {
            owner[pi] = Some((out.len(), true));
        }
        let name = if gname.is_empty() { parts[members[0]].name.clone() } else { gname.clone() };
        add(name, members, extra, Some((gname.clone(), ids.clone())), &mut out, &mut places);
    }
    for pi in 0..parts.len() {
        if owner[pi].is_none() {
            owner[pi] = Some((out.len(), false));
            add(parts[pi].name.clone(), vec![pi], Vec::new(), None, &mut out, &mut places);
        }
    }
    // DIYLC lets two things share a name; here each part needs its own, so add -2, -3...
    let mut seen = BTreeSet::new();
    for c in &mut out {
        let base = c.reference.clone();
        let mut k = 2;
        while !seen.insert(c.reference.clone()) {
            c.reference = format!("{base}-{k}");
            k += 1;
        }
        if c.reference != base {
            eprintln!("note: {path} has two parts called {base}; calling this one {}", c.reference);
        }
    }
    (out, places, owner.into_iter().map(Option::unwrap).collect())
}

/// Names each net after the most telling pin name in it: a supply name like 5V or GND first,
/// then any word (SCL, GP19), else after its first pin, like "Net-R1.2".
fn name_nets(nets: Vec<Vec<String>>) -> Vec<Net> {
    let mut used = BTreeSet::new();
    nets.into_iter()
        .map(|pins| {
            let pin_name = |label: &String| label.rsplit('.').next().unwrap().to_string();
            let hint = NET_NAME_HINTS.iter().find_map(|h| pins.iter().map(pin_name).find(|n| n.eq_ignore_ascii_case(h)));
            let word = || pins.iter().map(pin_name).find(|n| n.len() > 1 && !n.chars().all(|c| c.is_ascii_digit()));
            let base = hint.or_else(word).unwrap_or_else(|| format!("Net-{}", pins[0]));
            let mut name = base.clone();
            let mut k = 2;
            while !used.insert(name.clone()) {
                name = format!("{base}-{k}");
                k += 1;
            }
            Net { name, pins }
        })
        .collect()
}

/// The holes a part's body takes in the sketch.
fn body_of(class: &str, item: Node, pins: &[(String, Inches)]) -> Vec<Point> {
    let holes: Vec<Point> = pins.iter().map(|(_, at)| hole(*at)).collect();
    let (mut x0, mut x1) = (holes.iter().map(|p| p.0).min().unwrap(), holes.iter().map(|p| p.0).max().unwrap());
    let (mut y0, mut y1) = (holes.iter().map(|p| p.1).min().unwrap(), holes.iter().map(|p| p.1).max().unwrap());
    if holes.len() == 2 && x0 != x1 && y0 != y1 {
        return holes; // a two-legged part drawn at an angle: just its legs
    }
    if is_radial(class) {
        // A standing part is round: grow its box to its diameter along the legs and its width across them.
        let along = size_of(item, "length").map(|d| (d / HOLE).round() as i32).unwrap_or(1).max(1);
        let across = size_of(item, "width").map(|d| (d / HOLE).round() as i32).unwrap_or(along).max(1);
        let (want_x, want_y) = if y0 == y1 { (along, across) } else { (across, along) };
        let grow = |lo: &mut i32, hi: &mut i32, want: i32| {
            while *hi - *lo + 1 < want {
                *hi += 1;
                if *hi - *lo + 1 < want {
                    *lo -= 1;
                }
            }
        };
        grow(&mut x0, &mut x1, want_x);
        grow(&mut y0, &mut y1, want_y);
    }
    (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y))).collect()
}

/// The holes inside a rectangle given by two corner points. A hole counts only if the rectangle
/// reaches clearly past its centre (a fifth of a hole, about 0.5mm, as for footprints): a module's
/// board drawn with its edge on a line of holes, one hole beyond its headers, ends there, and the
/// headers sit on its edge.
fn covered(corners: &[Inches]) -> Vec<Point> {
    if corners.len() < 2 {
        return Vec::new();
    }
    let (a, b) = (corners[0], corners[1]);
    let lo = |u: f64, v: f64| (u.min(v) / HOLE + 0.2).ceil() as i32;
    let hi = |u: f64, v: f64| (u.max(v) / HOLE - 0.2).floor() as i32;
    let (x0, x1, y0, y1) = (lo(a.0, b.0), hi(a.0, b.0), lo(a.1, b.1), hi(a.1, b.1));
    (x0..=x1).flat_map(|x| (y0..=y1).map(move |y| (x, y))).collect()
}

/// Shortest distance between two straight segments (a point is a segment of length 0).
fn segment_distance((a, b): (Inches, Inches), (c, d): (Inches, Inches)) -> f64 {
    let cross = |o: Inches, p: Inches, q: Inches| (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0);
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0; // they cross
    }
    point_to_segment(a, (c, d)).min(point_to_segment(b, (c, d))).min(point_to_segment(c, (a, b))).min(point_to_segment(d, (a, b)))
}

/// Shortest distance from a segment to the square of half-side PIN_REACH around `p` (0 if they overlap).
fn square_to_segment(p: Inches, seg: (Inches, Inches)) -> f64 {
    let r = PIN_REACH;
    let inside = |q: Inches| (q.0 - p.0).abs() <= r && (q.1 - p.1).abs() <= r;
    if inside(seg.0) || inside(seg.1) {
        return 0.0;
    }
    let c = [(p.0 - r, p.1 - r), (p.0 + r, p.1 - r), (p.0 + r, p.1 + r), (p.0 - r, p.1 + r)];
    (0..4).map(|k| segment_distance(seg, (c[k], c[(k + 1) % 4]))).fold(f64::MAX, f64::min)
}

fn point_to_segment(p: Inches, (a, b): (Inches, Inches)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

// ------------------------------------------------------------ small XML helpers

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|n| n.has_tag_name(name))
}

fn text_of(node: Node, name: &str) -> Option<String> {
    child(node, name).map(|n| n.text().unwrap_or("").to_string())
}

/// The item's points, in inches: from <points>, <controlPoints> (<controlPoints2> in some files) or a single <point>.
fn points(node: Node) -> Vec<Inches> {
    let list = node.children().find(|n| n.tag_name().name() == "points" || n.tag_name().name().starts_with("controlPoints"));
    let nodes: Vec<Node> = match list {
        Some(l) => l.children().filter(|n| n.has_tag_name("point")).collect(),
        None => child(node, "point").into_iter().collect(),
    };
    let num = |n: &Node, a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    nodes.iter().map(|n| (num(n, "x"), num(n, "y"))).collect()
}

/// A size like <thickness value="1.0" unit="mm"/>, in inches.
fn size_of(node: Node, name: &str) -> Option<f64> {
    let n = child(node, name)?;
    let value: f64 = n.attribute("value")?.parse().ok()?;
    let per_inch = match n.attribute("unit")? {
        "in" => 1.0,
        "mm" => 25.4,
        "cm" => 2.54,
        _ => return None,
    };
    Some(value / per_inch)
}

/// Which copper layer a trace or pad is on: <layer>_1</layer> -> 1 (the default).
fn layer_of(node: Node) -> u8 {
    text_of(node, "layer").and_then(|l| l.trim_start_matches('_').parse().ok()).unwrap_or(1)
}

/// A part's value: <value value="1.0" unit="K"/> -> "1.0K", or <value>NE555P</value>.
fn value_of(node: Node) -> String {
    match child(node, "value") {
        Some(v) => match (v.attribute("value"), v.attribute("unit")) {
            (Some(x), Some(u)) => format!("{x}{u}"),
            _ => v.text().unwrap_or("").to_string(),
        },
        None => String::new(),
    }
}

/// A part's own pin names, if it has one for every pin: a pin header's labels ("5v,GND,3V3,...")
/// or a transistor's pinout (REGULATOR_AOI -> A, O, I; BJT_EBC -> E, B, C).
fn header_names(node: Node, count: usize) -> Option<Vec<String>> {
    let names: Vec<String> = match (text_of(node, "nodeNames"), text_of(node, "pinout")) {
        (Some(list), _) => list.split(',').map(|s| s.trim().to_string()).collect(),
        (None, Some(pinout)) => pinout.rsplit('_').next()?.chars().map(String::from).collect(),
        _ => return None,
    };
    (names.len() == count && names.iter().all(|n| !n.is_empty())).then_some(names)
}
