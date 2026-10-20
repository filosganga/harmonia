# TODO

## Find the smallest board that fits

If the user wants to take less space, they can pass a smaller board. Later we could add a feature
that iterates over smaller and smaller boards, or takes a list of sizes.

## Both sides of the board, handled the same way

Today parts can go under modules because modules are installed on headers, which raise them. A part
drawn at an angle in DIYLC also lets parts go under it, which is wrong: traces can go under it
because they are on the other side, a jumper wire could too, but not another component.

I am afraid to go too far in this direction and end up building a 3D system; perhaps this should be
a setting on each specific case. Or remove the feature completely: nothing passes under anything on
the same plane.

It's a big refactor: actually handle the two sides in the same way. We need to see how to design
it, as for example a component may be on the bottom as well. For now, keep the current behaviour.
