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
| Tooling | Verilog and VHDL formatters, a language server for both, an incremental build cache, an HTML schematic and reference viewer |
| Embedding | C ABI for use from another tool, and a WebAssembly build with a browser playground under `web/` |
| FPGA | device database (iCE40, ECP5, Xilinx 7 series, generic), primitive mapping, placement and IO constraints, nextpnr and vendor export |

Both languages go all the way through, from source to a synthesised
netlist, a simulation or a proof.

```sh
reticle build   --synth reticle.proj
reticle search  --index registry/ fifo
reticle add     --index registry/ uart_lite ^1.2.0
reticle check   counter.v counter.vhd design.rtl
reticle fmt     --write counter.v
reticle synth   --report --lut 4 --output netlist.rtl counter.vhd
reticle emit    --format verilog netlist.rtl
reticle sim     --vcd waves.vcd --coverage cov.info testbench.v counter.v
reticle sim     --interactive testbench.v counter.v
reticle verify  --depth 20 --trace cex.vcd design.rtl
reticle fpga    --device ice40-hx1k-tq144 --constraints pins.rcf blinky.v
reticle asic    --liberty cells.lib --verilog netlist.v counter.v
reticle timing  --constraints clocks.rcf design.v
reticle timing  --cdc design.v
reticle cache   --top top --output design.rtl leaf.v mid.v top.v
reticle viewer  --synth --output-dir docs/design counter.v
reticle lsp     # started by an editor, speaks the Language Server Protocol
```

`reticle fpga` runs the whole target flow and writes the netlist and
constraints the next tool reads, checking first that every cell is a
primitive the device actually has. Which tool that is depends on the
family: `nextpnr-ice40` and `nextpnr-ecp5` read a JSON netlist and a
`.pcf` / `.lpf`, and the Xilinx 7 series takes structural Verilog, an
`.xdc` and a Vivado script instead.

```sh
# A Digilent Basys 3 (Artix-7 XC7A35T): three files and the command
# that turns them into a bitstream.
reticle fpga --device xc7a35t-cpg236 --constraints board/basys3.rcf blinky.v
vivado -mode batch -source blinky.tcl
```

Reticle can also write the 7-series bitstream itself, from Project
X-Ray's chip database (public domain). The first run downloads a pinned,
hash-checked copy into `~/.cache/reticle`; `reticle fetch` does it ahead
of time, and `--chipdb` points at a copy of your own:

```sh
reticle fpga --device xc7a35t-cpg236 \
    --constraints examples/basys3/sw_led.rcf \
    --bitstream sw_led.bit examples/basys3/sw_led.v
```

Two designs built that way have reached a real Basys 3: `sw_led`, which a
person watched follow two switches, and `blink.v`, a 100 MHz pad clock
through a global buffer into twenty-six flip-flops, which a person
watched blink at the rate it was written for. A carry chain still does
not route.
[`docs/fpga-xray.md`](docs/fpga-xray.md) sets out exactly what is and is
not established, and `docs/fpga.md` says the same for the rest of the
7-series support.

A second real fabric is read the same way, Gowin's, from Project
Apicula's prebuilt chip database: `src/fpga/devices/gowin.dev` describes
the GW2A-18 of a Sipeed Tang Primer 20K and `fpga::apicula` turns that
database into the same architecture, and the same place and route write
a `.fs` for it. One design has run on that board and been watched
working: a button through a lookup table to an LED
(`examples/primer20k`). Nothing clocked routes on it yet;
[`docs/fpga-gowin.md`](docs/fpga-gowin.md) says exactly what is and is
not established.

```sh
cargo build --features cli,apicula,program
reticle fpga --device gw2a-18-pg256 \
    --constraints examples/primer20k/key_led.rcf \
    --bitstream key_led.fs examples/primer20k/key_led.v
reticle program key_led.fs
```

### Loading one into a board

`reticle program` puts a bitstream into an attached Xilinx 7-series or
Gowin GW2A part over JTAG, through the FTDI adapter a Digilent Basys 3 or
a Tang Primer 20K dock carries on board, with no vendor tool and nothing
else installed:

```sh
cargo build --features cli,program
reticle program --probe        # read IDCODE and status, write nothing
reticle program design.bit     # or design.fs
```

It writes the part's volatile configuration memory only; a power cycle
undoes it, and there is no flash programming in the crate at all. The
FTDI MPSSE encoding, the IEEE 1149.1 TAP state machine and the UG470
configuration sequence are pure library code in `src/program` with no
I/O and no `unsafe`, tested against a model TAP rather than against a
board; only the transport touches a device. It is behind the `program`
feature, off by default, because it is the one feature with a dependency
(`rawusb`, a sibling Karpeles Lab crate with none of its own), so
`cargo add reticle` still resolves to nothing.

It was proved first on a Basys 3, loaded with Project X-Ray's own
Vivado-built harness bitstream, which asserts `DONE`. [`docs/programming.md`](docs/programming.md) says exactly
what that does and does not establish.

Sources of one language are elaborated together, so a testbench and the
modules it instantiates go on one command line. A design already in the
`.rtl` IR text format is accepted anywhere a source file is.

## Worked examples and guides

Three complete systems are built out of the IP library in `examples/`,
each with a project manifest, a little user HDL and a test that drives it
from the manifest to the files `nextpnr` reads:

- [`examples/soc`](examples/soc) — a RISC-V system on chip around
  [`ip/rv32i`](ip/rv32i) and [`ip/uart`](ip/uart) that prints a line over
  a serial port.
- [`examples/mos6502_computer`](examples/mos6502_computer) — the same
  system around [`ip/mos6502`](ip/mos6502): one bus, RAM at the bottom
  for zero page and the stack, ROM at the top for the vectors.
- [`examples/apple2`](examples/apple2) — an Apple II-compatible machine
  with 48 KiB, the interleaved text page at `$0400` and video through
  [`ip/dvi_tx`](ip/dvi_tx) on an ECP5 or [`ip/vga_out`](ip/vga_out) on a
  Digilent Basys 3, running a monitor ROM and a character generator
  written for the example. Its tests decode the 40 x 24 character screen
  back out of a whole frame of video — once off the DVI colour bus and
  once off the VGA pins.
- [`examples/nes`](examples/nes) — an NES-compatible console around
  [`ip/mos6502`](ip/mos6502) with `DECIMAL_MODE = 0`, which is what the
  processor in an NES is, plus [`ip/ppu2c02`](ip/ppu2c02) and
  [`ip/dvi_tx`](ip/dvi_tx) on an ECP5 or [`ip/vga_out`](ip/vga_out) on a
  Digilent Basys 3. It runs a demo written for the example and contains
  no part of any commercial cartridge; the test compares every one of a
  frame's 61,440 pixels against a frame buffer computed from the
  documentation, and a second one reads a whole 640 x 480 frame back off
  the VGA pins with the palette truncated to the board's four bits a
  channel.

[`docs/writing-a-cpu.md`](docs/writing-a-cpu.md) is the guide behind the
processors in them: how to package a processor as IP, from the manifest
and the bus contract to testing a core so the test cannot agree with a
wrong core, cycle accuracy, interrupts and reset, reproducing documented
quirks, and what the two cores cost on real parts. The machinery itself
is in [`docs/ip.md`](docs/ip.md) and the blocks in
[`docs/ip-library.md`](docs/ip-library.md).

## Building

```sh
cargo build --release
cargo test
./target/release/reticle --help
```

Every stage is a Cargo feature (`verilog`, `vhdl`, `sim`, `synth`, `fpga`,
`asic`, `formal`, `lsp`, `cache`, `cli`); the default set is the frontends,
simulator, synthesis and the CLI. See `Cargo.toml`.
`asic`, `formal`, `lsp`, `viewer`, `cli`); the default set is the
frontends, simulator, synthesis and the CLI. See `Cargo.toml`.

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
