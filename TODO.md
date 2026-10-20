# TODO

## Find the smallest board that fits

If the user wants to take less space, they can pass a smaller board. Later we could add a feature
that iterates over smaller and smaller boards, or takes a list of sizes.

## Both sides of the board, handled the same way

It's a big refactor: actually handle the two sides in the same way. We need to see how to design
it, as for example a component may be on the bottom as well. I am afraid to go too far in this
direction and end up building a 3D system.

(Done on the parts' side: layers, and a `layers` rule for how one part is mounted; see README "How a
board is judged" and docs/open-questions.md #5.)
