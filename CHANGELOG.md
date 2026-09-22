# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- IP library: three larger blocks under `ip/`, taking it to fourteen.
  `rv32i` is the whole RV32I base integer instruction set in a
  multi-cycle machine-mode core with traps, interrupts and the machine
  CSRs, tested by assembling RISC-V machine code and running it — every
  instruction class, an array summed in a loop, and Fibonacci computed
  recursively on a stack. `eth_mac_rmii` is an Ethernet MAC over RMII,
  which is single data rate and so needs no device primitive the FPGA
  backend lacks, tested by looping its transmitter into its receiver and
  by rejecting a frame with a flipped dibit. `spiflash_xip` is a
  read-only execute-in-place path from a serial NOR flash, presenting
  the same memory port `rv32i` puts on its instruction side. Each has a
  manifest, a co-simulation test and a measured footprint in
  `docs/ip-library.md`.
- VHDL: the remaining bundled standard libraries — `ieee.numeric_std`,
  `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the
  Synopsys `std_logic_arith`, `std_logic_unsigned` and `std_logic_signed`.
  Each ships its declarations as VHDL and marks every subprogram
  `attribute foreign`; the bodies are native Rust over `logic::Logic`,
  shared between the analyser's constant folding and the elaborator's
  lowering to IR operators. A design using `unsigned` or `signed`
  arithmetic now analyses, elaborates and simulates.
