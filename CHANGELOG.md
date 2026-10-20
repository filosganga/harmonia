# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/filosganga/harmonia/compare/v0.2.0...v0.3.0) - 2026-10-20

### Added

- the router improves its best routing a few runs at a time
- [**breaking**] parts take layers above the board, and a board file can say how one is mounted
- [**breaking**] a build keeps its design and its rules, and is scored from them
- route by proposing many routings and letting the score choose

### Fixed

- draw a module's headers along whichever lines its pins form

### Other

- trim the both-sides note to what's still open
- the open questions from the router rework, for the maintainer
- note the idea of handling both sides of the board the same way
- move check and the network's training out of main.rs
- write each small helper once
- drop branches that can't happen and a repeated routing
- split the router into the routing, its search, and sketch wiring
- one description of DIYLC for the reader, the writer and the pictures

## [0.2.0](https://github.com/filosganga/harmonia/compare/v0.1.1...v0.2.0) - 2026-10-20

### Added

- your own footprints in ~/.config/harmonia, and never silently
- install harmonia and run it from any folder
- every output says its seed and how to make it again
- [**breaking**] every board-file rule takes a 0-1 weight
- [**breaking**] the board file's list of rules is called rules
- price corners and jumpers, not lengths
- collisions, keep-outs and parts off the board are a check, not a cost
- straight traces, no wires on pins, and dearer solder-side wires

### Fixed

- a module's board ends at a rectangle edge drawn on a line of holes

## [0.1.1](https://github.com/filosganga/harmonia/compare/v0.1.0...v0.1.1) - 2026-10-18

### Added

- module footprints, drawn as see-through boards
- layout file, the one place the drawings come from, and a draw command
- bench command, a yardstick for changes to the placer
- turns in the score and the router, so traces run straight
- crowding replaces area in the score, so parts spread out to be soldered
- close rule, and a one-of about one part's place makes no cases

### Fixed

- footprints checked against KiCad's files, and how to add one
- draw 2-pin headers and wrap long legend titles

### Other

- drop the done TODO about trace corners; few bends in the goal
- drop the done TODO about spreading components
- cat-feeder board file: supply star, C1 at the driver, USB, Pads and cables on edges

## [0.1.0](https://github.com/filosganga/harmonia/releases/tag/v0.1.0) - 2026-10-17

### Added

- board-file one-of as cases, orientation and within rules
- one constraints list in the board file, and near by two pins
- jumpers on both sides, a check command, and docs for a fresh session
- perfboard placer with routing and a tiny neural network

### Other

- release-plz runs as the filosganga-release GitHub App
- commit messages use Conventional Commits prefixes
- releases with release-plz, and binaries on each GitHub release
- actions/checkout v7, off the retired Node 20
- licenses, CONTRIBUTING, CODEOWNERS and CI, ready to be public
- call it Harmonia
