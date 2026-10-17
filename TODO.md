# TODO

## Add an edge contraint

You sometime would like the edge of a component to be on the edge of the board, for example if you have the usb port.

(Not the same as the board file's existing `"edge": { "J1": "left" }`: that only makes the part touch
that edge, in any rotation. This asks for a given side of the part, like the USB port's, to face it.)

I guess this can be its own constraint or perhaps mising up two contraints, like the area you can use and the orientation.

We also will require a OR contraint to say for example that the USB must be on an edge you can put two set of rules in or: 4 pair of area and orientation will make it.

example:

```
  "near": [
    { "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2" }
  ],
  "constraints": [
    ...,
    "this-or-that": {
      "one-of": {
        "constraints": [..., ...]
      }
    },
    ...
  ]
```