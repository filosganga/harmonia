# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
