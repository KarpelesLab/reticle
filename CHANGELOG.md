# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- VHDL: the remaining bundled standard libraries — `ieee.numeric_std`,
  `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the
  Synopsys `std_logic_arith`, `std_logic_unsigned` and `std_logic_signed`.
  Each ships its declarations as VHDL and marks every subprogram
  `attribute foreign`; the bodies are native Rust over `logic::Logic`,
  shared between the analyser's constant folding and the elaborator's
  lowering to IR operators. A design using `unsigned` or `signed`
  arithmetic now analyses, elaborates and simulates.
