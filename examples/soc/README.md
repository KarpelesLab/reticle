# A RISC-V SoC from the IP library

A small system on chip that prints `Hello from Reticle` over a serial
port. The processor is [`rv32i`](../../ip/rv32i) and the serial port is
[`uart`](../../ip/uart), both from Reticle's IP library, pulled in by a
project manifest. The only HDL written for the project is its top-level.

This is the worked example for phase 8 of the [roadmap](../../ROADMAP.md),
whose completion criterion is "a project manifest pulling in a UART and a
RISC-V core from the library builds, simulates its testbench, and runs on
an iCE40 board with no HDL written by the user beyond a top-level".
[`tests/soc.rs`](../../tests/soc.rs) drives it through each of those
steps. **What that proves, and what it does not, is at the end of this
page.**

## What is here

```text
examples/soc/
  reticle.proj              the project: rv32i and uart by path, the HX8K, the rest
  rtl/soc_top.v             the one piece of user HDL: ROM, RAM, decoder, UART registers
  sw/hello.s                the program, in RISC-V assembly
  sw/hello.hex              the program assembled, which the ROM loads with $readmemh
  tb/soc_tb.v               a testbench that decodes the serial line back into text
  board/hx8k_breakout.rcf   pins and clock for the iCE40-HX8K breakout board
```

`soc_top` decodes a small memory map on the core's data port:

| Address | What | Access |
|---------|------|--------|
| `0x0000_0000` | ROM, 256 words, loaded from `sw/hello.hex` | instruction fetch and loads |
| `0x0001_0000` | RAM, 256 words, byte-writable | loads and stores; the stack |
| `0x0002_0000` | UART `TXDATA` | write a byte to send it |
| `0x0002_0004` | UART `STATUS`, bit 0 = can take a byte | read |

The program polls `STATUS`, writes each byte of the string to `TXDATA`,
then spins. `rtl/soc_top.v` explains the rest: the one-wait-state memory
handshake, why a single ROM port serves both buses, and the power-on
reset that means the board needs no reset pin.

The part is the **iCE40 HX8K** (`ice40-hx8k-ct256`), not the HX1K of the
iCEstick: `rv32i` alone needs about 2400 four-input LUTs and the HX1K has
1280.

## Building

From this directory:

```sh
reticle build --synth reticle.proj
```

resolves the two packages, writes `reticle.lock`, elaborates the design
and synthesises it:

```text
note[S0014]: 1 simulation-only statement dropped (system call)
  ...
note: built `soc`: 5 module(s) from 5 source(s)
```

The note is the `$readmemh`; see [known gaps](#known-gaps) below.

## Testing

From the repository root:

```sh
cargo test --all-features --test soc
```

| Test | What it shows |
|------|---------------|
| `hello_hex_is_the_assembled_source` | `sw/hello.hex` is `sw/hello.s` assembled |
| `the_project_resolves_and_elaborates` | the manifest builds from exactly two library packages and `rtl/soc_top.v` |
| `the_soc_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch |
| `the_line_comes_out_of_the_serial_wire` | the testbench runs, and `Hello from Reticle\n` is decoded from the waveform of `uart_tx` |
| `the_soc_maps_onto_the_hx8k_and_exports_for_nextpnr` | the iCE40 flow fits it on the HX8K and writes `soc_top.json` and `soc_top.pcf` |
| `reticle_build_builds_the_project` | `reticle build --synth` on the manifest |
| `reticle_sim_cannot_run_the_testbench_yet` | what `reticle sim` does with the testbench today |

The simulation test decodes the text from the serial pin itself, not from
any register inside the design: it records every change of `uart_tx`
with its time and samples each 8N1 frame at 104 clocks per bit, the rate
`soc_top` is built with. The testbench does the same in Verilog and
prints what it receives with `$display`, and the two must agree.

To change the program, edit `sw/hello.s` and regenerate the hex:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test soc hello_hex
```

The assembler is `tests/rv32i_asm/mod.rs`, the same encoders that run
`rv32i`'s own tests in `tests/ip_library.rs`, with a small front end for
source text: labels, the base instructions, `nop`, `j`, `ret`, and
`.equ`, `.word`, `.string` and `.align`.

## Resource use

The iCE40 flow in `tests/soc.rs`, for `ice40-hx8k-ct256`:

| Resource | Used | HX8K has |
|----------|------|----------|
| `SB_LUT4` | 2368 | 7680 |
| flip-flops (`SB_DFF*`) | 372 | 7680 |
| `SB_CARRY` | 241 | — |
| `SB_RAM40_4K` | 10 | 32 |
| `SB_IO` | 3 | — |
| `SB_GB` | 1 | 8 |

The ten block RAMs are the ROM (two), the four RAM byte lanes (one each)
and the register file (four: two copies, one per read port). Almost all
the logic is the core; the logic depth is 41 LUTs.

## On a board

The flow writes `soc_top.json` (the netlist, in the Yosys JSON format)
and `soc_top.pcf` (the pins) to `target/tmp/soc/`. With the
[IceStorm](https://github.com/YosysHQ/icestorm) tools and an
iCE40-HX8K breakout board, the remaining steps would be:

```sh
cd target/tmp/soc
nextpnr-ice40 --hx8k --package ct256 --json soc_top.json --pcf soc_top.pcf --asc soc_top.asc
icepack soc_top.asc soc_top.bin
iceprog soc_top.bin
```

and then a terminal on the board's second FTDI serial port at 115200
baud, 8N1 (for example `picocom -b 115200 /dev/ttyUSB1`), which should
show `Hello from Reticle` once per configuration.

**None of that has been done.** This machine has no board attached and
no IceStorm tools installed, and Reticle's own iCE40 place and route uses
a synthetic fabric that cannot program a real part
([`docs/fpga.md`](../../docs/fpga.md)). So what is demonstrated stops at
the files `nextpnr-ice40` reads, and those files would not yet work,
because of the gaps below: in particular the ROM in `soc_top.json` has no
contents.

## Known gaps

Building this example found eight defects in Reticle. Each is reproduced
in a few lines by a test at the end of `tests/soc.rs` that asserts the
defect is still there, so fixing one fails its test and points back here.

| Test | The defect | What it costs this example |
|------|------------|----------------------------|
| `a_system_task_without_parentheses_is_dropped` | `$finish;` is lowered as an expression and silently discarded; `$finish(0);` works | the testbench says `$finish(0)` |
| `readmemh_in_verilog_never_reaches_the_simulator` | the Verilog frontend passes `$readmemh`'s memory as a name, the simulator wants a memory read, so every call fails with "needs a memory"; `reticle sim` also gives the simulator no file access | the ROM cannot be loaded as written |
| `synthesis_drops_the_contents_readmemh_loads` | synthesis drops `$readmemh` with a note | the synthesised ROM is empty |
| `block_ram_loses_the_contents_it_is_initialised_with` | a memory mapped to `SB_RAM40_4K` gets no `INIT_*` parameters | the ROM in the exported netlist is blank |
| `the_hx8k_database_lacks_the_breakout_boards_uart_pins` | the HX8K's device database has only the breakout board's LEDs and clock | Reticle refuses the constraints for the serial port (nextpnr would take them) |
| `reticle_fpga_does_not_flatten_a_design_with_instances` | `reticle fpga` maps the top module without flattening it | the binary cannot run the iCE40 flow on this design; the test flattens first |
| `a_rom_with_two_read_ports_is_not_duplicated_across_block_rams` | a ROM read from two ports stays a generic cell, where a RAM would be duplicated | `soc_top` shares one ROM port between the buses, which this core allows |
| `displaying_a_memory_word_prints_the_memory_name` | `$display("%h", mem[1])` prints the memory's name | nothing; found while chasing the `$readmemh` one |

Until the `$readmemh` defects are fixed, `tests/soc.rs` puts the words of
`sw/hello.hex` into the ROM's initial contents in the IR (`preload_rom`)
before simulating and synthesising. That is the one step the binary
cannot take, which is why `reticle sim` on the testbench times out today
instead of printing the line, and why the iCE40 flow here runs through
the library rather than `reticle fpga`.

## What this proves, and what it does not

Proved here, by `cargo test`:

- the manifest resolves `rv32i` and `uart` from `ip/` and builds, through
  `ip::resolve` and `ip::elaborate` and through `reticle build`, with
  `rtl/soc_top.v` the only user HDL;
- synthesis accepts it with no error, warning or latch;
- the testbench, simulated, carries exactly `Hello from Reticle\n` on the
  serial wire, decoded from the pin's waveform, with the program in the
  ROM;
- the iCE40 flow maps every cell to an HX8K primitive, fits in a third of
  the part, and exports the JSON and PCF that `nextpnr-ice40` reads.

Not proved:

- **that it runs on a board.** No board or IceStorm tools here, and the
  exported netlist's ROM is empty until the block RAM defect is fixed;
- that `reticle sim` runs the testbench, or `reticle fpga` maps the
  design, from the command line: both are blocked by the defects above;
- timing: nothing here checks that the design closes at 12 MHz on the
  real part.
