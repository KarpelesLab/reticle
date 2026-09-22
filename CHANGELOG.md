# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.1](https://github.com/KarpelesLab/reticle/compare/v0.0.0...v0.0.1) - 2026-09-22

### Added

- *(formal)* decide combinational miters by SAT sweeping
- *(cli)* give every synthesising command a file provider
- *(fpga)* describe block RAM layouts and partial pin lists in the device database
- *(ip)* add usb_device_fs, a full-speed USB device that enumerates
- *(ip)* add eth_mac_rgmii on eth_mac_rmii's frame logic
- *(ip)* add dvi_tx, DVI output serialised through DDR registers
- *(ip)* add hyperram_ctrl, a HyperBus controller through the DDR IO path
- *(ip)* add sdram_ctrl, an SDR SDRAM controller held to the datasheet
- *(cli)* add lsp, search, add, asic and an interactive sim
- *(fpga)* configure double-data-rate IO registers and IO delays
- *(fpga)* instantiate a PLL for a clock the board does not have
- *(fpga)* duplicate a block RAM for a register file's read ports
- *(fpga)* build a memory that misses a block RAM out of logic
- *(fpga)* invert a reset the family has no polarity for
- *(synth)* add arithmetic lowering with a choice of architectures
- *(viewer)* render a design as schematic and reference HTML pages
- *(cli)* add `reticle cache` over a directory-backed store
- *(cache)* add a content-addressed cache of elaborated modules
- *(ip)* add rv32i, eth_mac_rmii and spiflash_xip to the library
- *(sim)* add a compiled two-state cycle-based fast mode
- *(ip)* add a static registry index and the library half of `reticle add`
- *(ip)* import IP-XACT component descriptions
- *(asic)* map designs onto a Liberty library and hand off to OpenROAD
- *(ffi)* add a C API and a WebAssembly surface for embedding
- *(synth)* report estimated combinational depth
- *(fpga)* add placement, routing and bitstream generation
- *(lsp)* add a language server for Verilog and VHDL
- *(cli)* check assertions and write coverage from sim
- *(sim)* add an interactive session with breakpoints on net changes
- *(sim)* add line and toggle coverage with a text and LCOV report
- *(sim)* add concurrent assertions with an SVA and PSL subset
- *(ip)* add the Reticle IP library blocks under ip/
- *(vhdl)* bundle the remaining ieee libraries with native builtin bodies
- *(cli)* add the build command for IP projects
- *(ip)* lower VHDL sources in a project build
- *(ip)* add IP and project manifests with dependency resolution
- *(cli)* add the timing command
- *(timing)* add clock domain crossing analysis
- *(timing)* add static timing analysis
- *(vhdl)* add elaboration and lowering to the IR
- *(cli)* add the fpga command and a synth equivalence flag
- *(fpga)* map to device primitives and complete the nextpnr flow
- *(synth)* add post-synthesis equivalence checking
- *(synth)* add the cellify pass
- *(cli)* expose technology mapping from synth
- *(fpga)* add device database, primitive mapping and constraints
- *(synth)* add AIG optimiser and LUT/gate technology mapping
- *(cli)* take Verilog sources for every stage, add fmt
- *(verilog)* add elaboration and lowering to the IR
- *(vhdl)* add semantic analysis and the std/ieee libraries
- *(vhdl)* add the source formatter
- *(verilog)* add the source formatter
- *(fmt)* add a Wadler document printer and a line diff
- *(cli)* write FST waveforms from sim
- *(sim)* add FST waveform writer with in-crate LZ4 and zlib
- *(ir)* add flattening, uniquification and hierarchy queries
- *(verilog)* add AST-level linter with 28 rules
- *(asic)* add Liberty, LEF and DEF readers and writers
- *(cli)* wire check, synth, emit, sim and verify subcommands
- *(synth)* add process lowering, FF/latch/memory/FSM inference and opt passes
- *(ir)* add Verilog, VHDL, JSON, BLIF and EDIF emitters
- *(formal)* add bit-blaster, BMC, k-induction, equivalence checking and reachability lint
- *(sim)* add event-driven simulator, VCD writer and cosim API
- *(verilog)* add parser and AST
- *(vhdl)* add the VHDL-2008 parser and AST
- *(formal)* add CDCL SAT solver and Tseitin encoder
- *(ir)* add the unified design IR with text format
- *(verilog)* add preprocessor and lexer
- add string interner and 4-state Logic value type
- *(vhdl)* add the VHDL-2008 lexer

### Fixed

- *(cache)* record the files synthesis reads and re-check them on every hit
- *(verilog,sim,synth)* load memory files, run bare system tasks, print memory words
- *(fpga)* flatten the design in synthesize_for
- *(fpga)* give block RAM its initial contents and duplicate read-only memories
- *(cli)* let sim read $readmemh files from disk
- *(ip,fpga)* keep a project's top, and give a zero-step delay nothing
- *(asic)* round the area in the flow report
- *(test)* skip the C link test on the MSVC target
- *(synth)* let the gate mapper invert a gate's output
- *(viewer)* keep a bus width when its wire label has to be cut
- *(viewer)* cut a wire label to the room before the next box
- *(synth,fpga)* two defects writing the RISC-V core exposed
- *(sim)* close eight divergences a code review found in compiled mode
- *(sim)* apply chained asynchronous resets in dependency order
- *(sim)* give every compiled operation kind its own cache key
- *(ip)* word the inferred bus prefix note for the case with no prefix
- *(sim)* compute the gzip trailer length in 64-bit arithmetic
- *(timing)* recognise a synchroniser written as a shifting register
- *(timing)* say when a netlist's storage is hidden in black boxes
- *(synth)* keep an asynchronous reset asynchronous after a VHDL frontend
- *(vhdl)* analyse component instantiations and defaulted generics
- make the cli feature build and pin golden line endings
- *(test)* skip environment-dependent tests instead of failing
- *(test)* drop a duplicated feature guard in the lint test

### Other

- *(synth)* share the FRAIG simulation classes and sweep loop
- *(soc)* assert the four FPGA backend fixes, and update the docs
- *(ip)* prove phase 8 on a RISC-V SoC built from the IP library
- *(test)* move the RV32I assembler into a shared test module
- mark the device-primitive IP blocks done in the roadmap
- *(fpga)* pin a zero-step IO delay still building a delay element
- mark arithmetic lowering done in the roadmap
- *(synth)* prove and measure the arithmetic architectures
- *(cache)* write up the incremental build, measurements included
- *(cache)* cover the invalidation directions and a damaged store
- record the larger IP blocks in the roadmap and changelog
- *(sim)* re-measure compiled fast mode after the correctness fixes
- *(sim)* report the compiled fast mode measurement properly
- *(ip)* document the registry index and the IP-XACT import
- note the ASIC standard-cell flow in the README
- *(asic)* document the ASIC flow and tick phase 6
- note the C API, WebAssembly build and language server
- *(ffi)* pass --lib in the documented cargo rustc commands
- *(fpga)* document the routing architecture and tick phase 6
- *(sim)* add a simulation guide covering the whole stage
- *(ip)* point at the IP library and tick phase 8's library item
- *(ip)* co-simulate and measure every IP library block
- note the bundled ieee packages in the README
- *(vhdl)* record where the numeric lowering departs from the packages
- *(ip)* drop the unused requirement table from the lock builder
- *(ip)* document the manifest formats and tick phase 8
- *(ip)* add golden project builds and example IP packages
- *(timing)* document the timing model and what it leaves out
- *(timing)* add golden timing and crossing reports
- describe the source formatters
- *(sim)* mention FST capture and guard its test by feature
- *(ir)* point walk's module docs at the new hier module
- guard integration tests by feature and widen the matrix
- *(ir)* use logic::Logic as the IR constant type

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
