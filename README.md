# reticle

[![CI](https://github.com/KarpelesLab/reticle/actions/workflows/ci.yml/badge.svg)](https://github.com/KarpelesLab/reticle/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/reticle.svg)](https://crates.io/crates/reticle)
[![docs.rs](https://img.shields.io/docsrs/reticle)](https://docs.rs/reticle)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A VHDL / Verilog compiler written **from scratch in Rust**, with no foreign
code and no third-party crates.

Reticle is intended to grow into one self-contained hardware toolchain:

- **Frontends** for Verilog-2005 / synthesisable SystemVerilog and VHDL-2008,
  both lowering to a single intermediate representation so mixed-language
  designs need no special handling.
- **Simulation** with an event-driven 4-state simulator, VCD / FST waveforms,
  and a Rust co-simulation API for writing testbenches as `#[test]`s.
- **Synthesis** with flip-flop, memory and FSM inference, an AIG-based logic
  optimiser, and LUT and standard-cell mapping.
- **Targets** for FPGA (device databases, placement constraints, an own
  placer and router for open families, interop with nextpnr and vendor tools)
  and ASIC (Liberty, LEF / DEF, static timing analysis).
- **Verification** through bounded model checking and equivalence checking
  on an in-crate SAT solver.
- **IP integration**: a manifest format for third-party IP, bus interface
  abstractions (AXI, Wishbone, APB), black-box handling for encrypted vendor
  cores, and a first-party IP library tested through the simulator.

The full plan, ordered by dependency and with a definition of done for each
phase, is in [ROADMAP.md](ROADMAP.md).

## Status

Early, but the middle of the pipeline runs end to end. What works today:

| Stage | State |
|-------|-------|
| Verilog / SystemVerilog | preprocessor, lexer, parser; elaboration in progress |
| VHDL-2008 | lexer, parser; semantic analysis in progress |
| Unified IR | design model, validator, round-tripping `.rtl` text format |
| Simulation | event-driven 4-state simulator, VCD and FST waveforms, Rust co-simulation API |
| Synthesis | process lowering, flip-flop / latch / memory / FSM inference, optimisation passes |
| Emission | Verilog, VHDL, Yosys JSON, BLIF, EDIF |
| Formal | CDCL SAT solver, bit-blaster, bounded model checking, k-induction, equivalence checking |

The frontends parse and check real designs but do not yet lower to the IR,
so the stages after elaboration take `.rtl` input for the moment:

```sh
reticle check      counter.v counter.vhd design.rtl
reticle synth      --report --output netlist.rtl design.rtl
reticle emit       --format verilog netlist.rtl
reticle sim        --vcd waves.vcd --fst waves.fst testbench.rtl
reticle verify     --depth 20 --trace cex.vcd design.rtl
```

## Building

```sh
cargo build --release
cargo test
./target/release/reticle --help
```

Every stage is a Cargo feature (`verilog`, `vhdl`, `sim`, `synth`, `fpga`,
`asic`, `formal`, `lsp`, `cli`); the default set is the frontends,
simulator, synthesis and the CLI. See `Cargo.toml`.

## Design principles

- **One IR.** Everything after elaboration works on the same data structure.
- **Precise diagnostics.** Every IR object carries a span back to source, and
  errors are rendered rustc-style with excerpts.
- **No foreign code.** Hand-written lexers, parsers, simulator, SAT solver
  and codecs, so the toolchain is auditable and builds anywhere Rust does,
  including WebAssembly.
- **Library first.** The CLI is a thin wrapper; testbenches, generators and
  build scripts use the crate directly.
- **Sans-I/O core.** The library never touches the filesystem.

## License

MIT, see [LICENSE](LICENSE).
