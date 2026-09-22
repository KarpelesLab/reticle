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
  cores, a static registry index, an IP-XACT importer, and a first-party IP
  library tested through the simulator.

The full plan, ordered by dependency and with a definition of done for each
phase, is in [ROADMAP.md](ROADMAP.md).

## Status

Early, but the middle of the pipeline runs end to end. What works today:

| Stage | State |
|-------|-------|
| Verilog / SystemVerilog | preprocessor, lexer, parser, 28-rule linter, elaboration and lowering to the IR |
| VHDL-2008 | lexer, parser, semantic analysis, elaboration and lowering to the IR, with std, std_logic_1164, numeric_std, numeric_bit, math_real and the Synopsys packages bundled |
| Unified IR | design model, validator, round-tripping `.rtl` text format |
| Simulation | event-driven 4-state simulator, VCD and FST waveforms, concurrent assertions over an SVA and PSL subset, line and toggle coverage, an interactive session, Rust co-simulation API |
| Synthesis | process lowering, flip-flop / latch / memory / FSM inference, optimisation passes, AIG optimiser, LUT and standard-cell mapping, post-synthesis equivalence checking |
| Emission | Verilog, VHDL, Yosys JSON, BLIF, EDIF |
| Formal | CDCL SAT solver, bit-blaster, bounded model checking, k-induction, equivalence checking |
| ASIC | Liberty, LEF and DEF readers and writers, standard-cell mapping with flip-flop matching, SDC output and an OpenROAD hand-off |
| Timing | static timing analysis with setup and hold, path reports, clock domain crossing checks |
| IP | manifests with dependency resolution and a lock file, bus interfaces, generated interconnect, encrypted-core black boxes, a static registry index, IP-XACT import |
| Tooling | Verilog and VHDL formatters, a language server for both, an incremental build cache |
| Embedding | C ABI for use from another tool, and a WebAssembly build with a browser playground under `web/` |
| FPGA | device database (iCE40, ECP5, generic), primitive mapping, placement and IO constraints, nextpnr and vendor export |

Both languages go all the way through, from source to a synthesised
netlist, a simulation or a proof.

```sh
reticle build   --synth reticle.proj
reticle check   counter.v counter.vhd design.rtl
reticle fmt     --write counter.v
reticle synth   --report --lut 4 --output netlist.rtl counter.vhd
reticle emit    --format verilog netlist.rtl
reticle sim     --vcd waves.vcd --coverage cov.info testbench.v counter.v
reticle verify  --depth 20 --trace cex.vcd design.rtl
reticle fpga    --device ice40-hx1k-tq144 --constraints pins.rcf blinky.v
reticle timing  --constraints clocks.rcf design.v
reticle timing  --cdc design.v
reticle cache   --top top --output design.rtl leaf.v mid.v top.v
```

`reticle fpga` runs the whole target flow and writes the netlist and
constraints that nextpnr reads, checking first that every cell is a
primitive the device actually has.

Sources of one language are elaborated together, so a testbench and the
modules it instantiates go on one command line. A design already in the
`.rtl` IR text format is accepted anywhere a source file is.

## Building

```sh
cargo build --release
cargo test
./target/release/reticle --help
```

Every stage is a Cargo feature (`verilog`, `vhdl`, `sim`, `synth`, `fpga`,
`asic`, `formal`, `lsp`, `cache`, `cli`); the default set is the frontends,
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
