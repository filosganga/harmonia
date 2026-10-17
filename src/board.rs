//! Perfboard model: the circuit, placement geometry, rule-based scoring, rendering.
//!
//! Coordinates are perfboard holes: x grows to the right, y grows downwards.
//! A placement gives each component the position of its top-left hole and a
//! rotation (0..3, in 90 degree steps).

use rand::Rng;
use std::collections::HashSet;
use std::fmt::Write;

pub type Point = (i32, i32);

/// The eight things the rules measure. They are also the network's inputs.
pub const FEATURES: [&str; 8] = ["wire", "jumpers", "jumper_len", "bottom", "power", "overlap", "area", "near"];

/// How much each measurement costs, in the same order as FEATURES. Lower total cost = better board.
pub const WEIGHTS: [f64; 8] = [
    1.0,  // wire: per hole of copper trace, as routed (see route.rs, whose costs match these)
    5.0,  // jumpers: per jumper on the parts' side, where a trace couldn't get through
    2.0,  // jumper_len: per hole of jumper or solder-side wire: twice a trace, so they stay short
    30.0, // bottom: per wire on the solder side, the last resort
    1.5,  // power: per hole between each 5V pin and its nearest GND pin
    40.0, // overlap: per hole where two parts collide or a part hangs off the board
    0.15, // area: per hole of the rectangle that encloses all parts
    4.0,  // near: per hole between two pins the board file wants near (along copper) or close (straight)
];

/// Turns the cost into a 0-100 score (100 = perfect, about 37 at cost == SCORE_SCALE).
pub const SCORE_SCALE: f64 = 150.0;

#[derive(Clone)]
pub struct Pin {
    pub name: String,
    /// Its hole. For a pin between holes (`off_grid`), the nearest one.
    pub at: Point,
    /// The pin isn't on a hole, like a tactile switch's legs. Copper in the holes around it could
    /// touch it, so the router keeps other nets out of them.
    pub off_grid: bool,
}

#[derive(Clone)]
pub struct Component {
    pub reference: String,
    /// What the netlist says it is, e.g. "NE555P" or "10k". Only used when exporting.
    pub value: String,
    /// Full KiCad footprint name, e.g. "Package_DIP:DIP-8_W7.62mm".
    pub footprint: String,
    /// Which DIYLC part to draw it as, if footprints.json says.
    pub diylc: Option<crate::diylc::Part>,
    /// Its outline: w x h holes before turning.
    pub w: i32,
    pub h: i32,
    pub pins: Vec<Pin>,
    /// The holes inside the outline that are actually taken. For most parts that's all of
    /// them; a module on headers only takes its header holes, and low parts can go under the rest.
    pub body: Vec<Point>,
    /// For parts read from a DIYLC file: the original drawing, so it can be written back moved.
    pub source: Option<crate::diylc_import::Source>,
    /// Low enough to sit under a module on headers (a resistor lying flat, a small ceramic capacitor).
    pub low: bool,
}

impl Component {
    /// Every hole of a w x h outline: the body of an ordinary part.
    pub fn full_body(w: i32, h: i32) -> Vec<Point> {
        (0..w).flat_map(|x| (0..h).map(move |y| (x, y))).collect()
    }
}

#[derive(Clone)]
pub struct Net {
    pub name: String,
    pub pins: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct BoardSize {
    pub width: i32,
    pub height: i32,
}

/// Everything the placer needs. Built by `netlist::load` from a KiCad netlist.
#[derive(Clone)]
pub struct Circuit {
    pub board: BoardSize,
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
    /// The supply and ground nets, if known (without them the power rule is off).
    pub power: Option<String>,
    pub ground: Option<String>,
    /// For each net, its pins as (component index, pin index).
    pub net_pins: Vec<Vec<(usize, usize)>>,
    /// Your own rules for this board (where parts go, keep-outs, near, star), from the board file.
    pub constraints: crate::constraints::Constraints,
    /// What the router joins: one run per net, or one per branch of a star net.
    pub runs: Vec<Run>,
}

/// Pins to join with one piece of copper. Two runs never share copper, even on the same net.
#[derive(Clone)]
pub struct Run {
    pub net: usize,
    /// (component index, pin index); for a star run the star point comes first.
    pub pins: Vec<(usize, usize)>,
    /// The star point, if this run is a branch of a star net.
    pub star: Option<(usize, usize)>,
}

impl Circuit {
    pub fn new(board: BoardSize, components: Vec<Component>, nets: Vec<Net>, power: Option<String>, ground: Option<String>) -> Circuit {
        let mut c = Circuit { board, components, nets, power, ground, net_pins: Vec::new(), constraints: Default::default(), runs: Vec::new() };
        c.net_pins = c.nets.iter().map(|net| net.pins.iter().map(|p| c.find_pin(p)).collect()).collect();
        c.runs = crate::constraints::runs(&c);
        // Stop now, with a clear message, if the power or ground net doesn't exist.
        c.power.iter().chain(&c.ground).for_each(|name| {
            c.net_index(name);
        });
        c
    }

    /// "U1.8" -> (index of U1, index of its pin "8")
    pub fn find_pin(&self, label: &str) -> (usize, usize) {
        let (reference, pin) = label.split_once('.').unwrap_or_else(|| panic!("pin {label} should look like U1.8"));
        let ci = self.components.iter().position(|c| c.reference == reference);
        let ci = ci.unwrap_or_else(|| panic!("a net uses {reference}, which isn't in the component list"));
        let pi = self.components[ci].pins.iter().position(|p| p.name == pin);
        (ci, pi.unwrap_or_else(|| panic!("{label}: its footprint has no pad {pin}")))
    }

    fn net_index(&self, name: &str) -> usize {
        self.nets.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("no net called {name}"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub rot: u8,
}

pub type Placement = Vec<Place>;

pub fn random_placement(c: &Circuit, rng: &mut impl Rng) -> Placement {
    let (w, h) = (c.board.width, c.board.height);
    let mut placement: Placement = c
        .components
        .iter()
        .map(|_| Place { x: rng.random_range(0..w - 1), y: rng.random_range(0..h - 1), rot: rng.random_range(0..4) })
        .collect();
    crate::constraints::apply(c, &mut placement);
    placement
}

/// Width and height of a component after `rot` quarter turns.
pub fn size(comp: &Component, rot: u8) -> (i32, i32) {
    if rot % 2 == 0 { (comp.w, comp.h) } else { (comp.h, comp.w) }
}

/// Turns a hole inside a w x h outline `rot` quarter turns clockwise (the outline's top-left stays at 0, 0).
pub fn turn((x, y): Point, w: i32, h: i32, rot: u8) -> Point {
    let (mut x, mut y, mut w, mut h) = (x, y, w, h);
    for _ in 0..rot % 4 {
        (x, y, w, h) = (h - 1 - y, x, h, w);
    }
    (x, y)
}

/// Where everything physically is on the board.
pub struct Geometry {
    /// Holes taken by each component's body.
    pub bodies: Vec<Vec<Point>>,
    /// Every hole inside each component's outline (the same as its body, except for modules).
    pub outlines: Vec<Vec<Point>>,
    /// Absolute position of each pin: pins[component][pin].
    pub pins: Vec<Vec<Point>>,
}

pub fn geometry(c: &Circuit, placement: &[Place]) -> Geometry {
    let mut g = Geometry { bodies: Vec::new(), outlines: Vec::new(), pins: Vec::new() };
    for (comp, p) in c.components.iter().zip(placement) {
        let at = |hole: Point| {
            let (dx, dy) = turn(hole, comp.w, comp.h, p.rot);
            (p.x + dx, p.y + dy)
        };
        g.bodies.push(comp.body.iter().map(|&b| at(b)).collect());
        g.outlines.push(Component::full_body(comp.w, comp.h).into_iter().map(at).collect());
        g.pins.push(comp.pins.iter().map(|pin| at(pin.at)).collect());
    }
    g
}

fn manhattan(a: Point, b: Point) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// The raw measurements, in the order of FEATURES.
#[derive(Clone, Copy, Debug)]
pub struct Measures(pub [f64; FEATURES.len()]);

pub fn measure(c: &Circuit, placement: &[Place]) -> Measures {
    let (w, h) = (c.board.width, c.board.height);
    let g = geometry(c, placement);
    let routing = crate::route::route(c, &g);

    let pins_of = |net: usize| -> Vec<Point> { c.net_pins[net].iter().map(|&(ci, pi)| g.pins[ci][pi]).collect() };
    let net_pins = |name: &Option<String>| name.as_ref().map(|n| pins_of(c.net_index(n))).unwrap_or_default();
    let (vcc, gnd) = (net_pins(&c.power), net_pins(&c.ground));
    let power: i32 = vcc.iter().map(|&v| gnd.iter().map(|&g| manhattan(v, g)).min().unwrap_or(0)).sum();

    // Collisions and keep-outs only count the holes a part actually takes; hanging off the
    // board counts its whole outline (a module's board can't hang off the perfboard either).
    let mut seen = HashSet::new();
    let mut overlap = 0;
    for &cell in g.bodies.iter().flatten() {
        if !seen.insert(cell) || c.constraints.keepout_holes.contains(&cell) {
            overlap += 1;
        }
    }
    overlap += g.outlines.iter().flatten().filter(|&&(x, y)| x < 0 || x >= w || y < 0 || y >= h).count() as i32;
    overlap += under_modules(c, &g).len() as i32;

    let outline = || g.outlines.iter().flatten();
    let (min_x, max_x) = (outline().map(|p| p.0).min().unwrap(), outline().map(|p| p.0).max().unwrap());
    let (min_y, max_y) = (outline().map(|p| p.1).min().unwrap(), outline().map(|p| p.1).max().unwrap());
    let area = (max_x - min_x + 1) * (max_y - min_y + 1);

    // For each "near" rule: holes of copper between its two pins, the way the router joined them.
    // For each "close" rule: holes between its two pins, straight across the board.
    let along_copper = c.constraints.near.iter().map(|&(net, [(ca, pa), (cb, pb)])| routing.copper_between(c, net, g.pins[ca][pa], g.pins[cb][pb]));
    let straight = c.constraints.close.iter().map(|&[(ca, pa), (cb, pb)]| {
        let ((xa, ya), (xb, yb)) = (g.pins[ca][pa], g.pins[cb][pb]);
        (((xa - xb).pow(2) + (ya - yb).pow(2)) as f64).sqrt()
    });
    let near: f64 = along_copper.chain(straight).sum::<f64>() + 0.0; // + 0.0: no rules is 0, not -0

    Measures([
        routing.trace_length(),
        routing.jumpers.len() as f64,
        routing.jumper_length(),
        routing.bottom.len() as f64,
        power as f64, overlap as f64, area as f64, near])
}

/// Holes where something that doesn't fit sits under a module: (part, hole). Only low parts
/// (see `Component::low`) fit under a module's board, and two modules can't overlap.
pub fn under_modules(c: &Circuit, g: &Geometry) -> Vec<(usize, Point)> {
    let mut out = Vec::new();
    for (m, (outline, body)) in g.outlines.iter().zip(&g.bodies).enumerate() {
        if body.len() == outline.len() {
            continue; // not a module
        }
        let under: HashSet<Point> = outline.iter().filter(|p| !body.contains(p)).copied().collect();
        for (p, comp) in c.components.iter().enumerate() {
            if p != m && !comp.low {
                out.extend(g.outlines[p].iter().filter(|h| under.contains(h)).map(|&h| (p, h)));
            }
        }
    }
    out
}

pub fn cost(m: &Measures) -> f64 {
    m.0.iter().zip(WEIGHTS).map(|(v, w)| v * w).sum()
}

/// 0-100, higher is better.
pub fn score(m: &Measures) -> f64 {
    100.0 * (-cost(m) / SCORE_SCALE).exp()
}

pub fn report(c: &Circuit, placement: &[Place], title: &str) -> String {
    let m = measure(c, placement);
    let parts: Vec<String> = FEATURES.iter().zip(m.0).map(|(k, v)| format!("{k}={v:.0}")).collect();
    format!("{title}: score {:.0}/100  (cost {:.1}: {})", score(&m), cost(&m), parts.join(", "))
}

// ---------------------------------------------------------------- rendering

/// Text picture: each part gets a letter, UPPERCASE on its pins, lowercase on its body, x = keep-out, # = collision.
pub fn ascii_board(c: &Circuit, placement: &[Place]) -> String {
    let (w, h) = (c.board.width, c.board.height);
    let g = geometry(c, placement);
    let mut grid = vec![vec!['.'; w as usize]; h as usize];
    for &(x, y) in &c.constraints.keepout_holes {
        grid[y as usize][x as usize] = 'x';
    }
    let on_board = |(x, y): Point| x >= 0 && x < w && y >= 0 && y < h;
    // Under a module (inside its outline but not taken): ':' where nothing else is.
    for (outline, body) in g.outlines.iter().zip(&g.bodies) {
        for &p in outline.iter().filter(|&&p| on_board(p) && !body.contains(&p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            if *cell == '.' {
                *cell = ':';
            }
        }
    }
    for (i, body) in g.bodies.iter().enumerate() {
        let letter = (b'a' + i as u8) as char;
        for &p in body.iter().filter(|&&p| on_board(p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            *cell = if *cell == '.' || *cell == ':' { letter } else { '#' };
        }
    }
    for (_, p) in under_modules(c, &g).into_iter().filter(|&(_, p)| on_board(p)) {
        grid[p.1 as usize][p.0 as usize] = '#';
    }
    for (i, pins) in g.pins.iter().enumerate() {
        let letter = (b'A' + i as u8) as char;
        for &p in pins.iter().filter(|&&p| on_board(p)) {
            let cell = &mut grid[p.1 as usize][p.0 as usize];
            if *cell != '#' {
                *cell = letter;
            }
        }
    }
    let mut out = String::from("   ");
    out.extend((0..w).map(|x| char::from_digit((x % 10) as u32, 10).unwrap()));
    for (y, row) in grid.iter().enumerate() {
        write!(out, "\n{y:2} {}", row.iter().collect::<String>()).unwrap();
    }
    let mut legend: Vec<String> =
        c.components.iter().enumerate().map(|(i, comp)| format!("{}={}", (b'A' + i as u8) as char, comp.reference)).collect();
    if !c.constraints.keepout.is_empty() {
        legend.push("x=keep-out".into());
    }
    if c.components.iter().any(|comp| comp.body.len() < (comp.w * comp.h) as usize) {
        legend.push(":=under a module".into());
    }
    out + "\n   " + &legend.join("  ")
}

const NET_COLORS: [&str; 14] = [
    "#1f77b4", "#2ca02c", "#17becf", "#8c564b", "#bcbd22", "#393b79", "#637939",
    "#8c6d31", "#843c39", "#7b4173", "#a6761d", "#1b9e77", "#756bb1", "#e6550d",
];

/// A colour for each net: power red, ground near-black, the rest from NET_COLORS.
/// The SVG picture and the DIYLC jumpers both use these, so a net looks the same in both.
pub fn net_colors(c: &Circuit) -> Vec<&'static str> {
    let mut others = NET_COLORS.iter().cycle();
    c.nets
        .iter()
        .map(|n| if c.power.as_ref() == Some(&n.name) { "#d62728" } else if c.ground.as_ref() == Some(&n.name) { "#333333" } else { others.next().unwrap() })
        .collect()
}
