# TODO

## USe pin for near

```
{ "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2" }
```

We should use pin for example:

```
{ "pins": ["R1.1", "ESP32C6.ESP32C6-L.GP2"] }
```

## Use constraints block

Rather than this

```
  "near": [
    { "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2" }
  ],
  "star": {
    "5v": {
      "at": "C1.+",
      "runs": [
        ["DRV8833.DRV8833-L.Vcc", "Ext.5V"],
        ["ESP32C6.ESP32C6-L.5v"]
      ]
    },
    "GND": {
      "at": "C1.-",
      "runs": [
        ["DRV8833.DRV8833-L.GND", "Ext.GND"],
        ["ESP32C6.ESP32C6-L.GND", "DS3231.DS3231.GND"]
      ]
    }
  }
```

lets have:

```
  "near": [
    { "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2" }
  ],
  "constraints": [
    "star-5v": {
      "star": {
        "at": "C1.+",
        "runs": [
          ["DRV8833.DRV8833-L.Vcc", "Ext.5V"],
          ["ESP32C6.ESP32C6-L.5v"]
        ]
      }
    },
    "r1-near-gp2: {
      "near": {
        "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2"
      }
    }
  ]
```

or perhaps better:

```
  "near": [
    { "part": "R1", "pin": "ESP32C6.ESP32C6-L.GP2" }
  ],
  "constraints": [
    {
      "kind": "star",
      "name": "star-5v-this-is-optional",
      "at": "C1.+",
      "runs": [
        ["DRV8833.DRV8833-L.Vcc", "Ext.5V"],
        ["ESP32C6.ESP32C6-L.5v"]
      ]
    },
    {
      "kind": "near",
      "part": "R1", 
      "pin": "ESP32C6.ESP32C6-L.GP2"
    },
    {
      "kind": "near",
      "name": "another near",
      "part": "R2", 
      "pin": "ESP32C6.ESP32C6-L.GP3"
    }
  ]
```

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