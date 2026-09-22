# Reticle roadmap

Reticle is a VHDL / Verilog compiler written from scratch in Rust. The end
goal is one self-contained toolchain that takes a hardware design from source
text to a simulated, verified, synthesised and placed result, on FPGA or ASIC,
without depending on any external EDA program for the parts that matter.

This file is the plan. It is ordered by dependency, not by importance:
nothing in phase N is started before the parts of phase N-1 it relies on are
usable. Each phase lists what "done" means so progress is measurable.

The design principles that shape every phase:

- **One IR.** Both languages lower to the same intermediate representation,
  and every pass after elaboration (simulation, synthesis, formal, emission)
  works on that IR only. Mixed-language designs are therefore free.
- **Precise diagnostics.** Every IR object carries a span back to source.
  Errors say which line, which signal, and why, the way rustc does, not the
  way vendor tools do.
- **No foreign code.** Hand-written lexers, parsers, simulator, SAT solver
  and file codecs. This keeps the crate auditable and portable (including
  to WebAssembly for a browser playground).
- **Library first.** The CLI is a thin wrapper. Everything is reachable from
  Rust so testbenches, generators and build scripts can be written in Rust
  against the crate directly.
- **Sans-I/O core.** The compiler never touches the filesystem itself; a
  `SourceMap` is handed in and artefacts are returned as values. The CLI does
  the I/O.

## Pipeline overview

```
  .v / .sv          .vhd
     │                │
  verilog::lex     vhdl::lex        (preprocessor, tokens, spans)
  verilog::parse   vhdl::parse      (AST, per language)
     │                │
  verilog::elab    vhdl::elab       (name resolution, types, params/generics,
     │                │              generate, constant evaluation)
     └───────┬────────┘
             ▼
        ir  (hierarchical netlist + processes, 4-state aware)
             │
   ┌─────────┼──────────────┬──────────────┐
   ▼         ▼              ▼              ▼
  sim      synth          formal         emit
 (events, (proc lowering, (BMC, equiv,  (Verilog, VHDL,
  VCD,     opt, FSM/mem    SAT)          JSON, EDIF, BLIF)
  cosim)   inference,
           tech map)
             │
      ┌──────┴──────┐
      ▼             ▼
    fpga          asic
  (device DB,   (Liberty, LEF/DEF,
   placement,    cell mapping,
   routing,      STA, SDC)
   bitstream
   interop)
```

## Phase 0: foundations

The plumbing every later phase relies on. Small, boring, and worth getting
right first because every diagnostic and every test goes through it.

- [x] Crate skeleton, feature gates, lints, CI (fmt / clippy / test on
      Linux, macOS, Windows).
- [x] `source`: `SourceMap`, `SourceId`, `Span`, line/column lookup.
- [x] `diag`: diagnostics with severity, primary and secondary labels, notes,
      and a plain-text renderer with source excerpts. Sorted, deterministic
      output so tests can snapshot it.
- [x] Interner for identifiers (`Symbol`), since HDL designs repeat the same
      names tens of thousands of times.
- [x] `Logic` value type: 4-state (`0 1 X Z`) bit vectors with arbitrary
      width, plus the 9-state `std_logic` encoding for VHDL (`U X 0 1 Z W L
      H -`) and the resolution functions between them. Fast paths for 2-state.
- [x] Test harness: golden-file tests under `testdata/`, one integration
      test per stage, each rewriting its expectations under
      `UPDATE_EXPECT=1`. Thirty-three test binaries, twenty-four of them
      golden-driven.
- [x] CLI skeleton: `reticle <subcommand>`, `--help`, `--version`, exit codes.

Done when: an unknown-file error from the CLI prints a rustc-style message
with a source excerpt, and the golden-test runner passes on an empty corpus.

## Phase 1: Verilog frontend

Verilog first because its grammar is smaller and the synthesisable subset is
very well defined, which makes it the fastest route to a working end-to-end
pipeline. Target is IEEE 1364-2005 plus the synthesisable and testbench
parts of SystemVerilog (IEEE 1800) that people actually use.

- [x] Preprocessor: `` `define `` / `` `ifdef `` / `` `include `` / macros with
      arguments, `` `timescale ``, `` `default_nettype ``, with spans that
      trace through expansions.
- [x] Lexer: full token set, numeric literals with bases and `x`/`z` digits,
      escaped identifiers, attributes `(* ... *)`.
- [x] Parser: modules, ports (ANSI and non-ANSI), parameters, nets, regs,
      `always` / `always_ff` / `always_comb` / `always_latch`, `initial`,
      continuous assigns, instances, `generate`, functions, tasks, `case`
      variants, `for`/`while`/`repeat`, `logic`, packed/unpacked arrays,
      `typedef`, `enum`, `struct` (packed), `package` / `import`,
      `interface` (as a bundle of nets), `$display`-family system tasks.
      Error recovery so one mistake yields one diagnostic, not fifty.
- [x] Elaboration: module hierarchy, parameter overrides (`#(...)` and
      `defparam`), generate unrolling, constant expression evaluation,
      implicit nets, width inference and the Verilog sizing rules
      (context-determined expression widths, sign extension), function
      inlining for constant evaluation.
- [x] Lowering to IR.
- [x] Linter rules on the AST that do not need synthesis (unused signals,
      implicit width truncation, latches from incomplete `case`, multiple
      drivers, blocking/non-blocking misuse). 28 rules with levels
      configurable by name and `(* lint_off *)` / `// reticle-lint: off`
      suppressions; see `docs/lints.md`.

Done when: every module in a curated corpus (own tests plus permissively
licensed open designs such as picorv32 and the SERV core) parses, elaborates
and lowers without error, and the linter output on them matches golden
files.

## Phase 2: VHDL frontend

VHDL-2008 with VHDL-93 compatibility. Harder than Verilog: strong typing,
overload resolution, packages, generics on packages and subprograms,
`std_logic` resolution, and the standard libraries (`ieee.std_logic_1164`,
`numeric_std`, `math_real`, `textio`) which must be provided in source form
and compiled like user code.

- [x] Lexer: case-insensitive identifiers, extended identifiers, character
      and string literals, bit-string literals, based literals, physical
      literals.
- [x] Parser: design units (entity, architecture, package, package body,
      configuration, context), declarations, concurrent statements
      (processes, signal assignment, component and entity instantiation,
      generate, block), sequential statements, subprograms, records,
      arrays, access types, files, attributes, aliases, protected types.
- [x] Semantic analysis: the VHDL type system, overload resolution,
      implicit operators, attribute evaluation (`'length`, `'range`,
      `'event`, `'image`...), library and `use` clause visibility, design
      unit dependency ordering, conversion functions. The result is an
      annotated AST (`vhdl::sema::Analysis`): arenas of declarations and
      types plus span-keyed side tables giving every name its declaration,
      every expression its type and static value, and every call its
      target, so elaboration and lowering walk the parser's tree.
- [x] Bundled standard libraries: `std.standard`, `std.textio`, `std.env`
      and `ieee.std_logic_1164`, written from scratch and shipped as VHDL
      source inside the crate, analysed by the same front end as user code.
- [x] The remaining bundled libraries: `ieee.numeric_std`,
      `ieee.numeric_bit`, `ieee.math_real`, `ieee.std_logic_textio` and the
      Synopsys legacy packages (`std_logic_arith`, `std_logic_unsigned`,
      `std_logic_signed`). Each ships its *declarations* as VHDL, which is
      what a design has to see, and marks every subprogram `attribute
      foreign`; the bodies are native Rust over `logic::Logic`, shared
      between the analyser's constant folding (`sema::builtin`) and the
      elaborator's lowering to IR operators (`elab::numeric`). A package
      Reticle still does not ship — `fixed_pkg`, `float_pkg`,
      `numeric_std_unsigned` — yields a single "not bundled" diagnostic
      rather than a cascade.
- [x] Elaboration: generic maps, port maps with conversions, generate
      statements, configuration resolution, default bindings.
- [x] Lowering to IR, including `std_logic` resolution as explicit IR
      resolution nodes so mixed-language designs get the right semantics.

Done when: the same bar as phase 1, on a VHDL corpus (own tests plus open
designs such as NEORV32), and a mixed-language design with a VHDL top
instantiating a Verilog module simulates correctly.

## Phase 3: IR and design database

Defined alongside phase 1, but stable by the end of phase 2. The IR is the
product; everything else is a producer or consumer of it.

- [x] Hierarchical `Design`: modules with ports, parameters kept as
      metadata, instances, nets, and a `Process` form (structured
      statements with sensitivity, for simulation and for synthesis lowering)
      next to a `Cell` form (combinational and sequential primitives, for
      after synthesis). Both coexist in one module.
- [x] Typed bit vectors, memories (arrays) as first-class objects, signed
      and unsigned arithmetic cells with explicit widths.
- [x] Attributes on every object (`keep`, `ram_style`, `async_reg`,
      user-defined) carried from the source `(* *)` / VHDL attribute
      specifications.
- [x] Flattening, hierarchy preservation flags, unique-ification of
      parameterised modules.
- [x] Textual IR format (`.rtl`) that round-trips, for golden tests and
      debugging.
- [x] Emitters: structural and behavioural Verilog, VHDL, JSON (Yosys-
      compatible shape so nextpnr and existing viewers accept it), BLIF,
      EDIF.

Done when: `reticle emit --verilog` on a lowered design produces Verilog
that re-imports to an IR equal to the original, and the JSON output loads
in nextpnr.

## Phase 4: simulation

An event-driven simulator over the IR's process form. Needed both as the
user-facing simulator and as the reference model for every synthesis pass
(a synthesised design must simulate identically to its source).

- [x] Scheduler implementing the Verilog stratified event queue (active,
      inactive, NBA, monitor regions) and the VHDL delta cycle model, unified
      so a mixed design behaves as each language's standard prescribes.
- [x] 4-state evaluation, `std_logic` resolution, delays (`#`, `after`,
      `wait for`), `wait until`, `$time`, `$finish`, `$display` / `report`
      family with correct formatting.
- [x] Waveform output: VCD, then FST (compressed, GTKWave native).
- [x] Interactive mode: run to time, step, force / release, dump.
- [x] Rust co-simulation API: drive inputs, read outputs, await edges, from
      a Rust test (`#[test]` that instantiates a DUT), in the spirit of
      cocotb but with types. This is how the in-crate IP library is tested.
- [x] Compiled 2-state fast mode: `sim::compiled` proves a design is
      synchronous and two-state safe (`check` names every obstacle with its
      span), lowers it to a straight-line program over a register file of
      machine words — constant folded, common subexpressions shared,
      topologically ordered — and runs a cycle as settle, edge, commit,
      with no event queue. `CompiledSim` mirrors the co-simulation API, so
      a testbench picks the engine per run. `tests/sim_compiled.rs` runs
      both engines on the same design for thousands of random vectors and
      compares every net and memory element each cycle; the measured
      speed-up (3–10x on the IP blocks, 42x on pure combinational logic)
      and what fast mode gives up are in `docs/simulation.md`.
- [x] Assertions: immediate assertions, a useful subset of SVA / PSL
      (sequences, `|->`, `|=>`, `##n`) checked during simulation.
- [x] Coverage: line and toggle coverage reports.

Done when: the standard testbenches of the phase 1 and 2 corpora run to
completion with matching output, and a Rust-driven testbench of a UART
transmits and receives a byte.

## Phase 5: synthesis

From the process form of the IR to a netlist of generic cells, then to
technology cells.

- [x] Process lowering: sensitivity analysis, flip-flop and latch inference
      (with reset and enable extraction), mux tree construction from
      `if` / `case`, `casez` / `casex` and priority handling.
- [x] Memory inference: RAM / ROM recognition from arrays, port collection,
      read-before-write / write-first semantics preserved, initialisation.
- [x] FSM extraction and re-encoding (binary, one-hot, gray).
- [x] Optimisation: constant folding, dead code elimination, redundant
      register removal, common subexpression merging, width reduction,
      mux and logic simplification, retiming (later).
- [x] Arithmetic lowering: adders, multipliers, comparators, shifters, with
      a choice of architectures; DSP block inference hooks for phase 6.
      `synth::arith` has five adders (ripple, carry-select, lookahead,
      Kogge-Stone, Brent-Kung), four multipliers (array, radix-4 Booth,
      Wallace, Dadda; signed and unsigned), ripple and prefix comparators,
      barrel and funnel shifters, and the combinational restoring divider
      up to a width threshold above which it says to pipeline instead.
      Every one is proved equivalent to the generic cell it replaces with
      `formal::check_equivalent`, and the cell and depth measurements are
      in `docs/arithmetic.md`, generated from the test. The `ArithLower`
      pass belongs after the optimiser and before technology mapping, and
      is opt-in until it has been measured on picorv32 and NEORV32.
      `arith::dsp_candidates` is the recognition side phase 6 consumes;
      mapping to device primitives stays in `fpga::primitives`.
- [x] Cellify: replace the expression trees that survive in cell inputs and
      continuous assigns with discrete cells, so the netlist formats (JSON,
      BLIF, EDIF) can express a synthesised design without technology
      mapping first. Runs after the optimisation loop (`synth::cellify`),
      so the optimiser still sees the expression form.
- [x] Logic optimisation core: an AIG (and-inverter graph) with structural
      hashing, rewriting, balancing and FRAIGing, sufficient to stand in for
      ABC on typical designs.
- [x] Generic technology mapping: k-LUT mapping (for FPGAs) and structural
      cell mapping against a gate library (for ASIC). Area and depth
      oriented modes.
- [x] Post-synthesis verification: `synth::verify::check_synthesis`
      proves the optimised result equivalent to the same source lowered
      with process lowering alone, through phase 7's equivalence engine
      (the engine cannot read the process form, so process lowering is the
      one pass taken on trust; the module docs say so). Simulation of the
      mapped netlist through phase 4 is still open.
- [x] Reports: cell counts, estimated combinational depth, inferred
      memories, flip-flops, latches and FSMs, with source spans.

Done when: picorv32 and NEORV32 synthesise to a generic LUT4 netlist with
cell counts within a small margin of Yosys, and the mapped netlist passes
the equivalence check.

## Phase 6: targets, placement and physical design

The part that turns a netlist into something that runs on a chip. The
approach is to interoperate with existing open tools first so real hardware
runs early, then replace them piece by piece.

FPGA:
- [x] Device database format describing a family's primitives, LUT size,
      FF features, block RAM and DSP shapes, IO and clock resources. The
      `.dev` text format round-trips; iCE40, ECP5 and a generic family are
      compiled in.
- [x] Primitive mapping: block RAM and DSP inference, IO buffers, global
      clock buffers, carry chains, and the family's own LUT and flip-flop
      primitives with their parameters (`SB_LUT4` / `SB_DFF*`, `LUT4` /
      `TRELLIS_FF`), chosen per bit from the variants the device file
      declares, with a shared inverter where the family lacks a reset or
      enable polarity. A memory under the block RAM threshold is built
      from distributed RAM or flip-flops, a register file with more
      readers than a block has ports is duplicated across blocks, and a
      clock constraint on an undriven net instantiates a PLL whose
      dividers `fpga::pll::solve` chooses, with the achieved frequency
      and error reported. A port with a `ddr` clock gets double-data-rate
      registers and one with an `io_delay` a delay element, both from
      the device file. (The narrow width modes of a block RAM are
      wired in bit order, which iCE40 permutes.)
- [x] Vendor and open-flow interop: `fpga::synthesize_for` runs the whole
      target flow (synthesis, primitives, LUT mapping, clean-up, device
      cells) and JSON export hands the result to nextpnr (iCE40 and ECP5;
      Gowin needs a device file), with structural Verilog + XDC / SDC
      constraints for Vivado and Quartus, so a design synthesised by
      Reticle can be placed by existing tools from day one. The exported
      netlist holds nothing but primitives the device database declares,
      which `fpga::check_nextpnr_json` verifies cell by cell, port by
      port and net by net before the tool sees it.
- [x] Placement constraints as a first-class language: pin assignment, IO
      standards, placement regions (pblocks), keep-hierarchy, relative
      placement macros, clock domains. Declared in the source via attributes
      or in a constraints file, and checked against the device database.
- [x] Own placer (analytic then simulated-annealing refinement) and router
      (PathFinder-style negotiated congestion) for open families, starting
      with iCE40 as the smallest useful target. The routing architecture
      is data too (`fpga::arch`, an `arch` section of the `.dev` text
      format): tiles on a grid, wires with spans, pips with their
      configuration bits, and which wire each bel pin reaches, expanded
      into a `RoutingGraph`. The placer solves a quadratic net model by
      conjugate gradient, legalises onto sites and refines by simulated
      annealing over a half-perimeter cost, honouring fixed pins,
      regions, `keep_hierarchy` and `rloc` macros. See `docs/fpga.md`.
- [x] Bitstream generation for open families, or hand-off to the vendor
      bitstream tool with everything else done in Reticle.
      `fpga::bitstream` writes the IceStorm-style `.asc` and reads it
      back; every bit position comes from the architecture file, not
      from Rust. **The architecture that ships is synthetic**: an
      iCE40-shaped fabric, not an iCE40, because Project IceStorm's chip
      database is not here and must not be invented. The flow, the
      algorithms and the formats are real and tested end to end; a real
      database replaces the built-in one by parsing a file, with no code
      change. `docs/fpga.md` lists exactly what it would have to supply.

ASIC:
- [x] Liberty (`.lib`) parser: cells, pins, functions, timing tables.
- [x] LEF / DEF read and write.
- [x] Standard-cell mapping through phase 5 against a Liberty library:
      `asic::library` turns a `.lib` into the mapper's gate library
      (functions as truth tables, areas, one representative delay per
      pin at an FO4 point derived from the library itself) and reports
      every cell it cannot use and why; `asic::flow::synthesize_asic`
      runs synthesis, flip-flop legalisation, cell mapping, flip-flop
      mapping and an optional drive-strength pass, and reports area and
      the critical path (Liberty delays) from one call. Flip-flops are
      matched on edge, reset kind and polarity and enable, inserting an
      inverter when the library lacks a polarity and moving an enable or
      a synchronous reset into the data path when it lacks the pin. The
      reference target is a *synthetic* library under `testdata/asic/`,
      shaped after an open PDK; SKY130 or IHP SG13G2 drops in unchanged
      (no process data is vendored). See `docs/asic.md`.
- [x] Hand-off to OpenROAD for placement and routing, later an own flow:
      `asic::export_openroad` returns the gate-level Verilog, the SDC
      (`asic::sdc`), a generated Tcl script that reads the LEF, the
      Liberty, the netlist and the constraints and runs floorplan,
      placement, CTS, routing and reporting in that order, the DEF when
      a floorplan is given, and the command line — spawning nothing.
      `asic::check_physical` verifies the netlist against the Liberty
      and the LEF cell by cell and pin by pin before the tool sees it.

Shared:
- [x] Static timing analysis: the `timing` feature builds a pin-level
      timing graph (cell arcs, net arcs, sequential cells breaking it),
      propagates arrival and required times forwards and backwards with
      rise and fall kept apart, and reports slack per end point and the
      N worst paths with source spans. Clocks, false paths and
      multicycle paths come from `Constraints` (`create_clock`,
      `set_false_path`, `set_multicycle_path`); ideal and propagated
      clock modes, input and output delays, and setup and hold are all
      supported. Delays come from a `DelayModel`: a unit model, a
      Liberty non-linear model over `asic::liberty`, and a table of
      FPGA primitive numbers. Combinational loops are reported rather
      than walked into. See `docs/timing.md`.
- [x] Clock domain crossing analysis using the IR's knowledge of clocks:
      every flip-flop is assigned a domain by tracing its clock pin to a
      source, and each crossing is classified as an unsynchronised
      crossing, a two-flop synchroniser, a gray-coded bus, a handshake
      or an asynchronous FIFO, with reconvergent synchronisers reported
      separately. Every finding says whether it is a structural fact or
      an unverifiable guess.

Done when: a blinky and a UART echo run on an iCE40 board with the whole
flow inside Reticle, the same designs run on an ECP5 and a Xilinx 7-series
board through interop, and a small block maps onto SKY130 cells with
timing reported.

## Phase 7: verification and formal

- [x] SAT solver (CDCL) in-crate, with a Tseitin CNF builder.
- [x] Bit-blaster from IR to CNF.
- [x] Bounded model checking of assertions and `assume` / `cover`
      properties over unrolled IR, with counter-example traces as VCD.
- [x] k-induction for unbounded proofs on suitable designs.
- [x] Combinational and sequential equivalence checking, used by phase 5's
      self-check.
- [x] Reachability-based lint (dead states, unreachable branches).

Done when: an incorrect FIFO with a full/empty assertion yields a counter-
example trace, and the fixed FIFO is proven by induction.

## Phase 8: IP integration and the Reticle IP library

The reason this project exists beyond "another synthesiser". Third-party
and first-party IP should drop into a design as easily as a Rust crate.

- [x] IP package manifest (`reticle.ip` in the IP's directory, not
      `reticle.toml`: the crate has no TOML parser and will not grow one,
      so it is the line-oriented format the `.rcf`, `.dev` and `.rtl` files
      already use): sources per language, top entity, parameters with types
      and ranges, bus interfaces exposed, target constraints, licence,
      version. See `docs/ip.md`.
- [x] Project manifest for the user's design (`reticle.proj`): dependencies
      on IP packages by path, git URL or registry, target device,
      constraints files, testbenches. Resolution is depth first, selects the
      highest version satisfying every requirement, reports conflicts and
      cycles with the path through the graph, and writes a `reticle.lock`.
      `git` and `registry` are grammar only so far, since fetching one is
      network I/O and the library does none; `reticle build`, `reticle sim`
      and `reticle test` are not wired to it yet.
- [x] Bus interface abstraction: AXI4 / AXI4-Lite / AXI4-Stream, Wishbone
      (classic and pipelined), APB, Avalon-MM, described once as data under
      `src/ip/buses/` so port maps are generated and checked — missing
      signals, reversed directions, widths that contradict the parameters —
      with interconnect generation: an AXI4-Lite crossbar and a Wishbone
      arbiter, both tested by simulating transactions.
- [x] Vendor and encrypted IP: black-box declarations from a stub, so a
      design using an encrypted core still elaborates, lints, and simulates
      with a behavioural model, and is emitted for the vendor tool to fill.
- [x] IP-XACT import for existing IP catalogues: `ipxact::import` reads a
      component description — IEEE 1685-2009 with the `spirit` namespace,
      1685-2014 and 1685-2022 with `ipxact`, since catalogues are a mix —
      and produces a `reticle.ip` plus a report of what was translated,
      approximated and dropped. VLNV to package name and version, model
      ports with directions and vector bounds evaluated against the
      component's parameters, file sets to `source` lines, parameters to
      `param` lines, and bus interfaces matched against the built-in bus
      set by a documented VLNV table; memory maps become a description
      rather than invented logic. The XML reader it needs is
      `src/ip/xml.rs`, which refuses external entities and DTDs on
      purpose. See `docs/ip.md`.
- [x] Generators: parameterised IP written in Rust against the IR builder
      API (the way Chisel or Amaranth do it), for blocks that are painful
      to express in HDL (wide crossbars, CORDIC tables, filter banks). The
      crossbar and the arbiter are the first two; the arithmetic generators
      are still to come.
- [x] The Reticle IP library, each block with a Rust co-simulation test and
      a documented resource footprint per target: FIFOs (sync / async),
      CDC synchronisers (level and pulse), UART, SPI, I²C, PWM, timers, an
      AXI4-Lite GPIO and block RAM wrappers. Fourteen packages under
      `ip/`, written in Verilog-2005, driven through `sim::Simulator` by
      `tests/ip_library.rs`, and measured for LUT4, LUT6, iCE40 and ECP5 in
      a table that test generates. The AXI4-Lite crossbar and the Wishbone
      arbiter are the interconnect, under generators above. See
      `docs/ip-library.md`.
- [x] The larger library blocks, the three the FPGA backend can support:
      `rv32i`, the whole RV32I base integer set in a multi-cycle
      machine-mode core with traps, interrupts and the machine CSRs, tested
      by assembling RISC-V machine code and running it — every instruction
      class, an array summed in a loop and Fibonacci computed recursively
      on a stack; `eth_mac_rmii`, an Ethernet MAC over RMII with the frame
      check sequence computed and checked, tested by looping its
      transmitter into its receiver and by rejecting a frame with a flipped
      dibit; and `spiflash_xip`, a read-only execute-in-place path from a
      serial flash, tested against a flash model on the four wires.
- [x] The library blocks that need device primitives first, built on
      the DDR registers (`ddr`), IO delays (`io_delay`) and PLLs
      (`clock_mhz`) the FPGA backend now configures: `sdram_ctrl`, an
      SDR SDRAM controller with the power-up sequence, refresh,
      open-row tracking per bank and datasheet timings derived from
      nanoseconds, tested against an SDRAM model that fails on any
      timing violation; `hyperram_ctrl`, a HyperBus controller with
      fixed and variable latency, DDR data and RWDS and register
      access, tested against a model that enforces the latency;
      `dvi_tx` (and `dvi_tx_pll`), DVI output at 640x480, 800x600 and
      1280x720 with a TMDS encoder checked exhaustively against the
      specification's algorithm and 10:1 serialisation through DDR
      outputs from a PLL-made five-times clock; `eth_mac_rgmii`, a
      gigabit MAC on `eth_mac_rmii`'s frame logic behind DDR IO with
      optional IO delays, tested by loopback; and `usb_device_fs` (and
      `usb_device_fs_pll`), a full-speed device with NRZI, bit
      stuffing and CRC5 / CRC16 checked and a control endpoint that
      enumerates, tested by a USB host model. Each block's header says
      what it does not do — no bursts on either memory controller,
      gigabit only for RGMII, endpoint 0 only for USB. See
      `docs/ip-library.md`.
- [x] Registry: a static index (git repository of manifests) that
      `reticle add` searches, in the style of a crates.io index: one file
      per package under a name-derived path, one line per release with
      checksum, dependencies and a yanked flag, in the same line-oriented
      format as the manifests. `Index` parses, renders, searches with a
      deterministic ranking and selects versions; `RegistryProvider` is a
      `SourceProvider`, so a registry dependency resolves like a path one
      through a caller-supplied fetcher; `registry::add` picks the best
      version and rewrites the project manifest, keeping its comments and
      layout. The `reticle add` and `reticle search` commands themselves
      are phase 9.

Done when: a project manifest pulling in a UART and a RISC-V core from the
library builds, simulates its testbench, and runs on an iCE40 board with
no HDL written by the user beyond a top-level.

Where that stands: `examples/soc` is that project — `rv32i` and `uart`
by path, an HX8K target (the core alone outgrows the HX1K), one
top-level of user HDL with a ROM, a RAM and a memory-mapped UART, and a
program that prints a line. `tests/soc.rs` shows it **builds** (through
`ip::resolve` / `ip::elaborate` and `reticle build --synth`, with no
latch), **simulates its testbench** with `Hello from Reticle\n` decoded
from the waveform of the serial pin (through the library: `reticle sim`
cannot load the ROM yet), and **maps** onto the HX8K (2368
LUT4s, 10 block RAMs) with the JSON and PCF `nextpnr-ice40` reads
exported. It does **not** show it running on a board, and cannot yet:
no board is attached, Reticle's own iCE40 place and route is a synthetic
fabric, and building the example found eight defects (each pinned in
`tests/soc.rs`), four of which stood between it and working silicon —
`$readmemh` from Verilog loads nothing in the simulator and is dropped
by synthesis, block RAM was mapped without its `INIT_*` contents, and
`reticle fpga` did not flatten. The last two are fixed (the ROM's block
RAMs now carry the program, and `reticle fpga` maps the SoC), as are the
HX8K pin list and ROM duplication; while the `$readmemh` ones are open
the test preloads the ROM in the IR and runs the iCE40 flow through the
library. Open until those are fixed and the board run has been done by
hand.

## Phase 9: developer experience

- [x] Language server (LSP) for both languages: diagnostics as you type
      (parse errors, the Verilog lint rules by name, VHDL semantic errors),
      go-to-definition, references, hover with resolved types and widths,
      document symbols, completion (including the port names of the unit an
      instantiation is connecting), rename, and formatting and range
      formatting through the existing formatters. The wire protocol, JSON
      included, is hand written; the server is sans-I/O
      (`Server::handle(Json) -> Vec<Json>`) with the stream behind a thin
      transport. It answers from the document a request names: there is no
      workspace index and no elaboration, so a width that depends on a
      parameter is reported as unknown rather than guessed, and a rename
      that could reach another file is refused with a reason. See
      `docs/lsp.md`.
- [x] Formatter for Verilog and VHDL (`verilog::format`, `vhdl::format`,
      shared `fmt_doc` printer; see `docs/formatting.md`).
- [x] Schematic / netlist viewer output (an HTML page rendering the IR)
      and documentation generation from source comments and port lists.
      `viewer::render(design, &SourceMap, &ViewerOptions) -> Site` returns
      the pages as `(path, contents)` pairs, so the library stays sans-I/O
      and `reticle viewer` writes them. The schematic lays the cell form
      out as a layered graph — longest-path ranking, barycentre ordering,
      orthogonal wires through channels between the columns — and draws it
      as inline SVG with pan, zoom, net highlighting, per-cell details and
      a search box, in a page that loads nothing from the network. The
      reference pages take their prose from the lexers' comment side
      tables. See `docs/viewer.md`.
- [x] WebAssembly build of the frontends and simulator for a browser
      playground (`wasm` feature, `web/reticle.js` and `web/index.html`;
      see `docs/wasm.md`).
- [x] C API for embedding the frontends and simulator in other tools
      (`ffi` feature, `src/ffi/reticle.h`; see `docs/ffi.md`).
- [x] Incremental compilation: cache elaborated modules keyed on source
      hash so large designs re-simulate quickly after a small edit
      (`cache` feature, `reticle cache`). The store is content addressed:
      a module's key is the digest of the source text of every file that
      defines it, the elaboration options, the parameter set, the compiler
      version, the feature set and the keys of its dependencies, so
      editing a leaf invalidates everything above it and nothing below.
      Artefacts are the IR's `.rtl` text, which already round-trips
      exactly; synthesised modules are cached too, under a key that adds
      the synthesis options. `Storage` is a four-method trait with an
      in-memory backend in the library and a directory-backed one in the
      binary, so the library stays sans-I/O. Measured: 2.6x on a
      631-module Verilog design, 3.1x with synthesis, 6x on VHDL, and
      slower than not caching on a cold build — see `docs/cache.md`.

## Non-goals (for now)

- Full SystemVerilog UVM class-based verification. Classes, constrained
  random and the UVM library are a project on their own; the co-simulation
  API in Rust covers the same need with better tooling.
- Analog / mixed-signal (Verilog-AMS, VHDL-AMS).
- Replacing vendor bitstream tools for closed families. Interop is the
  strategy there until the formats are documented.

## Versioning

`0.x` until phase 5 lands. Each phase with a "done when" that is met is a
minor release. The IR text format and the `reticle.toml` manifests get
stability guarantees at 1.0; the Rust API does not before then.
