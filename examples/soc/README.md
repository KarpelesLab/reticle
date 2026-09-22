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
warning[S0018]: the contents of `sw/hello.hex` could not be loaded into `rom`: synthesis was given no files
  ...
note: built `soc`: 5 module(s) from 5 source(s)
```

Synthesis turns the ROM's `$readmemh` into the memory's initial
contents, reading the file through a provider that the library is given
(`SynthOptions::files`); the binary does not give it one yet, so it says
the ROM is left empty rather than building an empty ROM in silence.
`tests/soc.rs` gives it one and checks the ROM holds the program.

## Testing

From the repository root:

```sh
cargo test --all-features --test soc
```

| Test | What it shows |
|------|---------------|
| `hello_hex_is_the_assembled_source` | `sw/hello.hex` is `sw/hello.s` assembled |
| `the_project_resolves_and_elaborates` | the manifest builds from exactly two library packages and `rtl/soc_top.v` |
| `the_soc_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch, and the ROM's initial contents are `sw/hello.hex`, loaded by `$readmemh` |
| `the_line_comes_out_of_the_serial_wire` | the testbench runs, the ROM loads through `$readmemh`, and `Hello from Reticle\n` is decoded from the waveform of `uart_tx` |
| `the_soc_maps_onto_the_hx8k_and_exports_for_nextpnr` | the iCE40 flow flattens it, fits it on the HX8K with the program in the ROM's block RAMs, and writes `soc_top.json` and `soc_top.pcf` |
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
and the register file (four: two copies, one per read port). The ROM's
two carry the program in their `INIT_0`..`INIT_F` parameters, the low
half of each word in one and the high half in the other, and the test
reads every word of `sw/hello.hex` back out of them. Almost all the
logic is the core; the logic depth is 41 LUTs.

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
the files `nextpnr-ice40` reads: every cell in `soc_top.json` a real
primitive, the pins in `soc_top.pcf` ones the board has, and the ROM's
block RAMs holding the program.

The same flow runs from the command line, flattening the core and the
UART into `soc_top` on the way:

```sh
reticle fpga --device ice40-hx8k-ct256 --top soc_top \
  --constraints board/hx8k_breakout.rcf \
  rtl/soc_top.v ../../ip/rv32i/rtl/rv32i.v \
  ../../ip/uart/rtl/uart_tx.v ../../ip/uart/rtl/uart_rx.v ../../ip/uart/rtl/uart.v
```

but its ROM is only as full as synthesis leaves it, and while synthesis
drops `$readmemh` (see [known gaps](#known-gaps)) the binary's ROM is
blank; the files `tests/soc.rs` writes are the ones with the program in.

## Known gaps

Building this example found eight defects in Reticle, each reproduced in
a few lines by a test at the end of `tests/soc.rs`. All eight are fixed.
Each test once asserted its defect was still there, so fixing it failed
the test and pointed back here; they now assert the fix.

| Test | The defect, now fixed |
|------|-----------------------|
| `a_system_task_without_parentheses_runs` | `$finish;` was lowered as an expression and silently discarded; the testbench now says `$finish;` |
| `readmemh_in_verilog_loads_the_memory_in_the_simulator` | the Verilog frontend passed `$readmemh`'s memory as a name, which the simulator refused; the IR now names the memory itself |
| `synthesis_loads_the_contents_readmemh_names` | synthesis dropped `$readmemh` with a note, leaving the ROM empty; it now loads the file into the memory's initial contents |
| `displaying_a_memory_word_prints_the_word` | `$display("%h", mem[1])` printed the memory's name |
| `block_ram_carries_the_contents_it_is_initialised_with` | a memory mapped to `SB_RAM40_4K` got no `INIT_*` parameters, so the exported ROM was blank; it now carries its contents in the layout each family's device file states |
| `a_rom_with_two_read_ports_is_duplicated_across_block_rams` | a read-only memory read from two ports stayed a generic cell; it is now duplicated like a written one |
| `the_hx8k_database_knows_the_breakout_boards_uart_pins` | the HX8K's pin list had only the board's LEDs and clock; it now has the serial and flash pins and treats an unlisted pin as unchecked rather than wrong |
| `reticle_fpga_flattens_a_design_with_instances` | the FPGA flow mapped the top module alone, so any design with an instance was refused |

## What this proves, and what it does not

Proved here, by `cargo test`:

- the manifest resolves `rv32i` and `uart` from `ip/` and builds, through
  `ip::resolve` and `ip::elaborate` and through `reticle build`, with
  `rtl/soc_top.v` the only user HDL;
- synthesis accepts it with no error, warning or latch;
- the testbench, simulated, carries exactly `Hello from Reticle\n` on the
  serial wire, decoded from the pin's waveform, with the program loaded
  into the ROM by the design's own `$readmemh`;
- synthesis gives the ROM that program as its initial contents;
- the iCE40 flow maps every cell to an HX8K primitive, fits in a third of
  the part, puts the program into the ROM's block RAMs, and exports the
  JSON and PCF that `nextpnr-ice40` reads, with no problem left for
  `check_nextpnr_json` to find;
- `reticle fpga` maps a design with instances, flattening it first.

Not proved:

- **that it runs on a board.** No board or IceStorm tools here, and
  where the iCE40 block RAM layout comes from is stated, with its
  confidence, in `src/fpga/devices/ice40.dev`, not checked against a
  part;
- timing: nothing here checks that the design closes at 12 MHz on the
  real part.
