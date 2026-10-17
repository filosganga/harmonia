#!/usr/bin/env python3
"""Drafts a footprints.json entry from a KiCad footprint file (.kicad_mod).

    python3 dev/kicad_footprint.py path/to/Some_Footprint.kicad_mod

A dev tool, not part of the program (README "Principles"). It reads the pads (where the legs go)
and the body as drawn on the fabrication layer (F.Fab, the part's real outline), turns millimetres
into perfboard holes (2.54mm apart), and prints a draft entry. Read the notes it prints, check the
pad numbers against the part's datasheet and KiCad symbol, and add "about", "low" and "diylc" by
hand (CONTRIBUTING.md "Adding a footprint").
"""
import json
import math
import sys

HOLE = 2.54  # mm between perfboard holes
# A hole counts as under the body only if the body reaches this far past the hole's centre: a leg
# (about 0.6mm thick) still fits in a hole the body's edge only just reaches.
MARGIN = 0.5
# A pad this close to a hole is on the grid; further off, its leg must be bent to reach a hole.
ON_GRID = 0.15


def parse(text):
    """KiCad's bracket format -> nested lists of strings."""
    stack, cur, i = [], [], 0
    while i < len(text):
        ch = text[i]
        if ch == "(":
            stack.append(cur)
            cur = []
        elif ch == ")":
            done, cur = cur, stack.pop()
            cur.append(done)
        elif ch == '"':
            j = text.index('"', i + 1)
            while text[j - 1] == "\\":
                j = text.index('"', j + 1)
            cur.append(text[i + 1 : j])
            i = j
        elif not ch.isspace():
            j = i
            while j < len(text) and not text[j].isspace() and text[j] not in '()"':
                j += 1
            cur.append(text[i:j])
            i = j - 1
        i += 1
    return cur[0]


def child(node, name):
    return next((c for c in node[1:] if isinstance(c, list) and c and c[0] == name), None)


def xy(node, name):
    c = child(node, name)
    return (float(c[1]), float(c[2])) if c else None


def main(path):
    root = parse(open(path).read())
    name = root[1]
    pads = []  # (number, x, y) in mm
    body = []  # points of the F.Fab drawing, in mm
    for item in root[1:]:
        if not isinstance(item, list):
            continue
        if item[0] == "pad" and item[1] and item[2] == "thru_hole":
            x, y = xy(item, "at")
            pads.append((item[1], x, y))
        layer = child(item, "layer")
        if item[0].startswith("fp_") and layer and layer[1] == "F.Fab":
            if item[0] == "fp_circle":
                (cx, cy), (ex, ey) = xy(item, "center"), xy(item, "end")
                r = math.hypot(ex - cx, ey - cy)
                body += [(cx - r, cy - r), (cx + r, cy + r)]
            elif item[0] == "fp_poly":
                body += [(float(p[1]), float(p[2])) for p in child(item, "pts")[1:] if p[0] == "xy"]
            else:
                body += [p for p in (xy(item, k) for k in ("start", "mid", "end")) if p]
    if not pads:
        sys.exit(f"{name}: no through-hole pads, so it can't go on a perfboard")

    # The hole grid lines up with pad 1 (or the first pad).
    first = next((p for p in pads if p[0] == "1"), pads[0])
    x0, y0 = first[1], first[2]
    notes = []
    holes = {}
    for number, x, y in pads:
        hx, hy = (x - x0) / HOLE, (y - y0) / HOLE
        if abs(hx - round(hx)) * HOLE > ON_GRID or abs(hy - round(hy)) * HOLE > ON_GRID:
            notes.append(f"pad {number} at ({x}, {y})mm is between holes: its leg must be bent, or use another footprint")
        if number in holes and holes[number] != (round(hx), round(hy)):
            notes.append(f"pad {number} appears twice (two legs on one pin); keep one")
        holes.setdefault(number, (round(hx), round(hy)))

    xs = [h[0] for h in holes.values()]
    ys = [h[1] for h in holes.values()]
    if body:
        bx0, bx1 = min(p[0] for p in body), max(p[0] for p in body)
        by0, by1 = min(p[1] for p in body), max(p[1] for p in body)
        notes.append(f"body (F.Fab) is {bx1 - bx0:.2f} x {by1 - by0:.2f}mm")
        xs += [math.ceil((bx0 + MARGIN - x0) / HOLE), math.floor((bx1 - MARGIN - x0) / HOLE)]
        ys += [math.ceil((by0 + MARGIN - y0) / HOLE), math.floor((by1 - MARGIN - y0) / HOLE)]
    else:
        notes.append("no F.Fab drawing: the outline is just the pads; widen it by hand to cover the body")
    left, top = min(xs), min(ys)
    entry = {
        "about": "TODO: what it is, which pad is which.",
        "w": max(xs) - left + 1,
        "h": max(ys) - top + 1,
        "pins": {n: [h[0] - left, h[1] - top] for n, h in sorted(holes.items(), key=lambda kv: (len(kv[0]), kv[0]))},
    }
    pins = json.dumps(entry["pins"])
    print(f'"{name}": {{\n  "about": "{entry["about"]}",\n  "w": {entry["w"]}, "h": {entry["h"]},\n  "pins": {pins}\n}}')
    for note in notes:
        print(f"note: {note}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
