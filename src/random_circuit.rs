//! Random circuits to learn from: parts from footprints.json, wired like a small hobby board.
//!
//! They aren't real circuits (nothing here would work), but they're meant to look like one to the
//! placer: a supply header, chips with a supply and a ground pin, modules on headers (an Arduino
//! Nano, a Pico) in about half of them, decoupling capacitors from supply to ground, LEDs and transistors with a leg on ground, and small signal nets of two to four pins
//! joining different parts, all joined up into one circuit. The same number always makes the same
//! circuit.

use crate::board::Net;
use crate::netlist::Part;
use rand::{Rng, SeedableRng, rngs::StdRng, seq::SliceRandom};
use serde_json::{Value, json};

pub const POWER: &str = "+5V";
pub const GROUND: &str = "GND";

/// A random circuit: its parts and nets, and its board file as JSON, read like a real one.
pub struct Spec {
    pub parts: Vec<Part>,
    pub nets: Vec<Net>,
    pub board_file: Value,
}

/// One kind of part to pick from.
struct Kind {
    footprint: &'static str,
    /// The reference's letter(s): R for R1, R2...
    prefix: &'static str,
    values: &'static [&'static str],
    pins: usize,
}

const RESISTOR: Kind = kind("Resistor_THT:R_Axial_DIN0207_L6.3mm_D2.5mm_P10.16mm_Horizontal", "R", &["100", "330", "1k", "4.7k", "10k", "47k", "100k"], 2);
const CERAMIC: Kind = kind("Capacitor_THT:C_Disc_D5.0mm_W2.5mm_P5.00mm", "C", &["10nF", "47nF", "100nF"], 2);
const ELECTROLYTIC: Kind = kind("Capacitor_THT:CP_Radial_D5.0mm_P2.50mm", "C", &["1uF", "10uF", "100uF"], 2);
const LED: Kind = kind("LED_THT:LED_D5.0mm", "D", &["LED"], 2);
const SIGNAL_DIODE: Kind = kind("Diode_THT:D_DO-35_SOD27_P7.62mm_Horizontal", "D", &["1N4148"], 2);
const POWER_DIODE: Kind = kind("Diode_THT:D_DO-41_SOD81_P10.16mm_Horizontal", "D", &["1N4007"], 2);
const TRANSISTOR: Kind = kind("Package_TO_SOT_THT:TO-92_Inline_Wide", "Q", &["BC547", "BC557"], 3);
const TO220: Kind = kind("Package_TO_SOT_THT:TO-220-3_Vertical", "Q", &["IRF540N", "L7805"], 3);
const TRIMMER: Kind = kind("Potentiometer_THT:Potentiometer_Bourns_3296W_Vertical", "RV", &["10k", "100k"], 3);
const DIP8: Kind = kind("Package_DIP:DIP-8_W7.62mm", "U", &["NE555P", "LM358", "ATtiny85"], 8);
const DIP14: Kind = kind("Package_DIP:DIP-14_W7.62mm", "U", &["74HC00", "LM324", "CD4093"], 14);
const DIP16: Kind = kind("Package_DIP:DIP-16_W7.62mm", "U", &["CD4017", "74HC595"], 16);
const HEADER2: Kind = kind("Connector_PinHeader_2.54mm:PinHeader_1x02_P2.54mm_Vertical", "J", &["Conn"], 2);
const HEADER3: Kind = kind("Connector_PinHeader_2.54mm:PinHeader_1x03_P2.54mm_Vertical", "J", &["Conn"], 3);
const HEADER4: Kind = kind("Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical", "J", &["Conn"], 4);
const HEADER2X3: Kind = kind("Connector_PinHeader_2.54mm:PinHeader_2x03_P2.54mm_Vertical", "J", &["ISP"], 6);
const TERMINAL: Kind = kind("TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2-5.08_1x02_P5.08mm_Horizontal", "J", &["Terminal"], 2);

const fn kind(footprint: &'static str, prefix: &'static str, values: &'static [&'static str], pins: usize) -> Kind {
    Kind { footprint, prefix, values, pins }
}

/// The small parts that make up most of a board, and how often each is picked (out of 100).
const SMALL_PARTS: [(&Kind, u32); 9] = [
    (&RESISTOR, 34),
    (&CERAMIC, 14),
    (&ELECTROLYTIC, 12),
    (&LED, 10),
    (&TRANSISTOR, 10),
    (&SIGNAL_DIODE, 6),
    (&TRIMMER, 6),
    (&POWER_DIODE, 4),
    (&TO220, 4),
];
const CONNECTORS: [&Kind; 5] = [&HEADER2, &HEADER3, &HEADER4, &HEADER2X3, &TERMINAL];
// Modules on headers ("module": true in footprints.json): only their pins take holes.
const NANO: Kind = kind("Module:Arduino_Nano", "M", &["Arduino Nano"], 30);
const BREAKOUT: Kind = kind("Module:Pololu_Breakout-16_15.2x20.3mm", "M", &["DRV8833", "A4988", "TB6612"], 16);
const FEATHER: Kind = kind("Module:Adafruit_Feather", "M", &["Feather M0", "Feather ESP32"], 28);
const PICO: Kind = kind("Module:RaspberryPi_Pico_Common_THT", "M", &["Pico"], 40);
const HUZZAH: Kind = kind("Module:Adafruit_HUZZAH_ESP8266_breakout", "M", &["HUZZAH ESP8266"], 20);
const MODULES: [&Kind; 5] = [&NANO, &BREAKOUT, &FEATHER, &PICO, &HUZZAH];
const PARTS: std::ops::RangeInclusive<usize> = 6..=20;
/// How much of the board the parts' outlines cover, picked at random in this range (the benchmark
/// circuits cover 19 to 31%). Boards are at least 12 x 8 holes, so a small circuit may cover less.
/// A module's board counts half, since low parts can go under it.
const DENSITY: std::ops::Range<f64> = 0.18..0.32;
const EDGES: [&str; 4] = ["left", "right", "top", "bottom"];

/// Random circuit number `n`.
pub fn make(n: u64) -> Spec {
    let mut rng = StdRng::seed_from_u64(n);
    // Wiring that doesn't join up into one circuit is thrown away and done again from where the
    // random numbers got to, so it's still the same circuit for the same `n`. The parts stay, so
    // small circuits (which fail more often) aren't rarer than big ones; after many failed tries,
    // new parts.
    loop {
        let kinds = pick_parts(&mut rng);
        for _ in 0..100 {
            if let Some(spec) = try_wiring(&kinds, &mut rng) {
                return spec;
            }
        }
    }
}

/// A pin of a part, as (part index, pin number from 1).
type Port = (usize, usize);

fn try_wiring(kinds: &[&'static Kind], rng: &mut StdRng) -> Option<Spec> {
    let mut nets: Vec<Vec<Port>> = vec![Vec::new(), Vec::new()]; // supply, ground; then signal nets
    let (supply, ground) = (0, 1);
    let mut free: Vec<Port> = Vec::new(); // pins still to wire into signal nets

    // Which pins go to supply and ground, by kind, the way such parts usually sit on a board.
    for (i, k) in kinds.iter().enumerate() {
        let mut on = |net: usize, pin: usize| nets[net].push((i, pin));
        let mut rest: Vec<usize> = (1..=k.pins).collect();
        let take = |pin: usize, rest: &mut Vec<usize>| rest.retain(|&p| p != pin);
        let footprint = k.footprint;
        if i == 0 {
            // The supply comes in on the first connector.
            on(supply, 1);
            on(ground, 2);
            rest.clear();
        } else if footprint == DIP8.footprint || footprint == DIP14.footprint || footprint == DIP16.footprint {
            // Ground on the last pin of the left row, supply on the last pin, as on a 74HC00, CD4017 or
            // LM358. (The values, like NE555P, are only labels: these circuits don't work.)
            let pins = k.pins;
            on(ground, pins / 2);
            on(supply, pins);
            take(pins / 2, &mut rest);
            take(pins, &mut rest);
            rest.retain(|_| rng.random_bool(0.75)); // some pins stay unused
        } else if footprint == RESISTOR.footprint {
            let roll: f64 = rng.random();
            if roll < 0.2 {
                on(supply, 1); // a pull-up
                take(1, &mut rest);
            } else if roll < 0.35 {
                on(ground, 2); // a pull-down
                take(2, &mut rest);
            }
        } else if footprint == CERAMIC.footprint || footprint == ELECTROLYTIC.footprint {
            // Pin 2 is - on an electrolytic. Often a capacitor to ground, and often decoupling,
            // from supply to ground.
            if rng.random_bool(0.7) {
                on(ground, 2);
                take(2, &mut rest);
                if rng.random_bool(0.4) {
                    on(supply, 1);
                    take(1, &mut rest);
                }
            }
        } else if footprint == LED.footprint {
            if rng.random_bool(0.6) {
                on(ground, 1); // the cathode
                take(1, &mut rest);
            }
        } else if footprint == TRANSISTOR.footprint || footprint == TO220.footprint || footprint == TRIMMER.footprint {
            if rng.random_bool(0.5) {
                on(ground, 3); // an emitter or source, or a trimmer's end
                take(3, &mut rest);
            }
        } else if MODULES.iter().any(|m| m.footprint == footprint) {
            // A module: a pin to the supply, one or two to ground, and only some of the others
            // used (a microcontroller board has far more pins than a small circuit needs).
            let mut pins = rest.clone();
            pins.shuffle(rng);
            on(supply, pins[0]);
            on(ground, pins[1]);
            if rng.random_bool(0.5) {
                on(ground, pins[2]);
            }
            rest = pins[3..].iter().copied().filter(|_| rng.random_bool(0.25)).collect();
        } else if CONNECTORS.iter().any(|c| c.footprint == footprint) && k.pins > 2 && rng.random_bool(0.6) {
            on(ground, k.pins);
            take(k.pins, &mut rest);
        }
        free.extend(rest.into_iter().map(|pin| (i, pin)));
    }

    // Join every part to the circuit: in a random order, one pin of each part to a free pin of a
    // part before it. Parts whose pins all went to supply or ground are joined through those.
    let mut order: Vec<usize> = (0..kinds.len()).collect();
    order.shuffle(rng);
    let mut joined: Vec<usize> = Vec::new();
    for &i in &order {
        let mine: Vec<Port> = free.iter().copied().filter(|p| p.0 == i).collect();
        let earlier: Vec<Port> = free.iter().copied().filter(|p| joined.contains(&p.0)).collect();
        if let (Some(a), Some(b)) = (pick(&mine, rng), pick(&earlier, rng)) {
            free.retain(|&p| p != a && p != b);
            nets.push(vec![a, b]);
        }
        joined.push(i);
    }

    // The pins left make nets of two to four pins, each on a different part; now and then one joins
    // a net that's there already instead.
    free.shuffle(rng);
    while let Some(first) = free.pop() {
        let signal = 2..nets.len();
        let roomy: Vec<usize> = signal.filter(|&n| nets[n].len() < 4 && nets[n].iter().all(|p| p.0 != first.0)).collect();
        if !roomy.is_empty() && (rng.random_bool(0.3) || free.iter().all(|p| p.0 == first.0)) {
            let n = roomy[rng.random_range(0..roomy.len())];
            nets[n].push(first);
            continue;
        }
        let size = match rng.random_range(0..100) {
            0..55 => 2,
            55..85 => 3,
            _ => 4,
        };
        let mut net = vec![first];
        while net.len() < size {
            let Some(k) = free.iter().position(|p| net.iter().all(|q| q.0 != p.0)) else { break };
            net.push(free.remove(k));
        }
        if net.len() == 1 {
            nets[ground].push(first); // nothing left to pair with
        } else {
            nets.push(net);
        }
    }

    if nets[supply].len() < 2 || nets[ground].len() < 2 || !joined_up(kinds.len(), &nets) || shorted(kinds, &nets) {
        return None;
    }
    Some(spec(kinds, &nets, rng))
}

/// The kinds of part in the circuit: a supply connector first, then chips, modules, other
/// connectors and small parts.
fn pick_parts(rng: &mut StdRng) -> Vec<&'static Kind> {
    let count = rng.random_range(PARTS);
    let mut kinds: Vec<&Kind> = vec![if rng.random_bool(0.7) { &HEADER2 } else { &TERMINAL }];
    let chips = match rng.random_range(0..100) {
        0..35 => 0,
        35..80 => 1,
        _ => 2,
    };
    for _ in 0..chips {
        kinds.push(match rng.random_range(0..10) {
            0..6 => &DIP8,
            6..8 => &DIP14,
            _ => &DIP16,
        });
    }
    let modules = match rng.random_range(0..100) {
        0..55 => 0,
        55..90 => 1,
        _ => 2,
    };
    for _ in 0..modules {
        kinds.push(MODULES[rng.random_range(0..MODULES.len())]);
    }
    for _ in 0..rng.random_range(0..=2) {
        kinds.push(CONNECTORS[rng.random_range(0..CONNECTORS.len())]);
    }
    while kinds.len() < count {
        // Roll 0-99 and walk down the list: each kind takes as many numbers as its weight.
        let mut roll = rng.random_range(0..100);
        for (k, weight) in SMALL_PARTS {
            if roll < weight {
                kinds.push(k);
                break;
            }
            roll -= weight;
        }
    }
    kinds
}

/// One of `items` at random, if there are any.
fn pick<T: Copy>(items: &[T], rng: &mut StdRng) -> Option<T> {
    (!items.is_empty()).then(|| items[rng.random_range(0..items.len())])
}

/// Whether the nets join every part into one circuit.
fn joined_up(parts: usize, nets: &[Vec<Port>]) -> bool {
    // Each part starts as its own group; a net merges the groups of all its parts.
    let mut group: Vec<usize> = (0..parts).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            i = group[i];
        }
        i
    }
    for net in nets {
        for w in net.windows(2) {
            let (a, b) = (root(&mut group, w[0].0), root(&mut group, w[1].0));
            group[a] = b;
        }
    }
    let first = root(&mut group, 0);
    (0..parts).all(|i| root(&mut group, i) == first)
}

/// Whether some two-legged part has both legs on one net (a resistor from ground to ground).
fn shorted(kinds: &[&Kind], nets: &[Vec<Port>]) -> bool {
    nets.iter().any(|net| (0..kinds.len()).any(|i| kinds[i].pins == 2 && net.iter().filter(|p| p.0 == i).count() == 2))
}

/// The parts named R1, C1, U1..., the nets, and a board file: a board the parts cover a random
/// share of, the supply connector on an edge, and some of the other connectors too.
fn spec(kinds: &[&Kind], nets: &[Vec<Port>], rng: &mut StdRng) -> Spec {
    let mut counts = std::collections::HashMap::new();
    let parts: Vec<Part> = kinds
        .iter()
        .map(|k| {
            let n = counts.entry(k.prefix).or_insert(0);
            *n += 1;
            let value = k.values[rng.random_range(0..k.values.len())];
            Part { reference: format!("{}{n}", k.prefix), value: value.into(), footprint: k.footprint.into() }
        })
        .collect();
    let name = |n: usize| match n {
        0 => POWER.to_string(),
        1 => GROUND.to_string(),
        _ => format!("N{}", n - 1),
    };
    let nets: Vec<Net> = nets
        .iter()
        .enumerate()
        .map(|(n, pins)| Net { name: name(n), pins: pins.iter().map(|&(i, pin)| format!("{}.{pin}", parts[i].reference)).collect() })
        .collect();

    // Board size: the parts' outlines cover DENSITY of it, about 3:2, and the longest part fits.
    let sizes = crate::netlist::sizes();
    let size = |k: &Kind| sizes[k.footprint.rsplit(':').next().unwrap()];
    let is_module = |k: &Kind| MODULES.iter().any(|m| m.footprint == k.footprint);
    let outline = |k: &Kind| (size(k).0 * size(k).1) as f64;
    let area: f64 = kinds.iter().map(|k| if is_module(k) { outline(k) / 2.0 } else { outline(k) }).sum();
    let longest = kinds.iter().map(|k| size(k).0.max(size(k).1)).max().unwrap();
    // Two modules can't overlap, so leave room for their whole boards too.
    let modules_area: f64 = kinds.iter().filter(|k| is_module(k)).map(|k| outline(k)).sum();
    let board_area = (area / rng.random_range(DENSITY)).max(1.5 * modules_area);
    let width = ((board_area * 1.5).sqrt().round() as i32).max(longest + 2).max(12);
    let height = ((board_area / width as f64).round() as i32).max(longest.min(width) / 2 + 2).max(8);

    let mut rules = vec![json!({ "kind": "edge", "part": parts[0].reference, "edge": EDGES[rng.random_range(0..4)] })];
    for (i, k) in kinds.iter().enumerate().skip(1) {
        if CONNECTORS.iter().any(|c| c.footprint == k.footprint) && rng.random_bool(0.6) {
            rules.push(json!({ "kind": "edge", "part": parts[i].reference, "edge": EDGES[rng.random_range(0..4)] }));
        }
    }
    let board_file = json!({ "board": format!("{width}x{height}"), "power": POWER, "ground": GROUND, "constraints": rules });
    Spec { parts, nets, board_file }
}
