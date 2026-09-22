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
- Incremental compilation behind the new `cache` feature: a
  content-addressed store of elaborated and synthesised modules, keyed on
  the source text that produced them, the options, the parameter set, the
  compiler version, the feature set and the keys of their dependencies.
  Editing a leaf invalidates it and everything above it; editing a
  top-level file invalidates only that module. Artefacts are the IR's
  `.rtl` text, so an entry is inspectable by hand. `Storage` is a
  four-method trait — the library ships the in-memory backend and the
  `reticle cache` command supplies a directory-backed one — with eviction
  by total size in least-recently-used order and a `verify` that catches a
  store damaged behind the build's back. See `docs/cache.md`, which
  includes the cases where the cache does not help.

- VHDL: the remaining bundled standard libraries — `ieee.numeric_std`,
  `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the
  Synopsys `std_logic_arith`, `std_logic_unsigned` and `std_logic_signed`.
  Each ships its declarations as VHDL and marks every subprogram
  `attribute foreign`; the bodies are native Rust over `logic::Logic`,
  shared between the analyser's constant folding and the elaborator's
  lowering to IR operators. A design using `unsigned` or `signed`
  arithmetic now analyses, elaborates and simulates.

### Fixed

- Verilog `$readmemh`, `$readmemb`, `$writememh` and `$writememb`, with
  their optional start and end addresses. The IR has a statement for
  them, `StmtKind::MemFile`, that names the memory itself; the Verilog
  frontend used to pass the memory as a string, which the simulator
  refused, so no `$readmemh` written in Verilog loaded anything. The
  simulator now loads through its `FileProvider` and hands saved files
  back through `Simulator::written_files`; synthesis reads an
  `initial` block's `$readmemh` through the same trait, given in the new
  `SynthOptions::files`, into the memory's initial contents, and says
  which file it could not load (`S0018`) instead of dropping the call.
  The trait and `MemoryFiles` moved to `ir::memfile` and are still
  re-exported from `sim`.
- Verilog: a system task written without parentheses (`$finish;`,
  `$stop;`, `$display;`) is the same call as its parenthesised form; it
  used to be dropped without a word.
- Verilog: a memory element in a `$display`-family argument
  (`$display("%h", mem[1])`) prints the word, not the memory's name.
