# Perfboard layout neural network

Goal: a neural network that scores and improves component placement on perfboard, from a netlist
with pinouts and component sizes in perfboard hole coordinates. Good-practice signals: 5V and GND
close together, short total wiring, few crossings, no collisions, compact board.

- Rust only. Filippo is learning Rust and does not want Python anywhere in this project.
- Filippo is new to neural networks: explain in plain language and keep the network readable.
- Keep the feedback loop fast: every command should finish in about a second.
- See README.md for the commands and what each file does.
