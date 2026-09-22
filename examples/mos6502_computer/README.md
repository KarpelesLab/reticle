# A 6502 computer from the IP library

A small computer that prints `Hello from Reticle` over a serial port.
The processor is [`mos6502`](../../ip/mos6502) and the serial port is
[`uart`](../../ip/uart), both from Reticle's IP library, pulled in by a
project manifest. The only HDL written for the project is its top-level.

This is [`examples/soc`](../soc) again with the other processor, and the
two are meant to be read side by side: same manifest format, same board,
same testbench shape, same test list. Everything that differs between
them differs because a 6502 is not a RISC-V core, and
[`docs/writing-a-cpu.md`](../../docs/writing-a-cpu.md) is the guide that
draws the line between the two. [`tests/mos6502_computer.rs`](../../tests/mos6502_computer.rs)
drives it through every step. **What that proves, and what it does not,
is at the end of this page.**

## What is here

```text
examples/mos6502_computer/
  reticle.proj                the project: mos6502 and uart by path, the HX8K
  rtl/computer_top.v          the one piece of user HDL: ROM, RAM, decoder, UART registers
  sw/hello.s                  the program, in 6502 assembly
  sw/hello.hex                the program assembled, which the ROM loads with $readmemh
  tb/computer_tb.v            a testbench that decodes the serial line back into text
  board/hx8k_breakout.rcf     pins and clock for the iCE40-HX8K breakout board
```

`computer_top` decodes one memory map, because a 6502 has one bus:
instructions, data, the stack and I/O all go through it.

| Address | What | Access |
|---------|------|--------|
| `$0000`–`$03FF` | RAM, 1 KiB | zero page, the stack in page one, and the rest |
| `$D000` | UART `DATA` | write a byte to send it |
| `$D001` | UART `STATUS`, bit 0 = can take a byte | read |
| `$F800`–`$FFFF` | ROM, 2 KiB, loaded from `sw/hello.hex` | the program and the vectors |

Two rows of that table are not design decisions, they are the part:

- **zero page and the stack have to be RAM.** `$0000`–`$00FF` is the
  6502's fastest addressing mode and `$0100`–`$01FF` is the stack, which
  is eight bits of stack pointer plus a hard-wired `$01` — nothing can
  move it. So the RAM goes at the bottom.
- **the vectors have to be ROM.** `$FFFA`–`$FFFB` is NMI, `$FFFC`–`$FFFD`
  is RES and `$FFFE`–`$FFFF` is IRQ and BRK. There is no parameter
  saying where the core starts: it reads `$FFFC` when reset is released
  and jumps there, before anything could have written anywhere. So the
  ROM goes at the top, and its last six bytes are the map.

`rtl/computer_top.v` explains the rest: the one-wait-state bus, why the
ROM needs only one port, and the power-on reset that means the board
needs no reset pin.

The part is the **iCE40 HX8K** (`ice40-hx8k-ct256`), the same as
`examples/soc`. The core is smaller than `rv32i` — about 1730 four-input
LUTs against 2400 — but still more than the HX1K's 1280, and this is the
board whose serial port reaches the host.

## How the bus works

`mos6502` performs exactly one access per bus cycle and holds it until
it sees `ready` high at a rising edge. The ROM and the RAM are read on a
clock edge, which is what lets them become block RAM, so their data is
one clock late. `computer_top` therefore drives `ready` low on the first
clock of every access and high on the second:

```verilog
always @(posedge clk or negedge rst_n) begin
    if (!rst_n) bus_ready <= 1'b0;
    else        bus_ready <= ~bus_ready;
end
```

Every bus cycle costs two clocks and every access costs the same two, so
the core still spends the cycle *counts* the 6502's tables print — it
just spends them at half the clock. A 12 MHz oscillator runs this at
6 MHz, six times an original part. That this works at all is the point
of the core's `ready`: it stalls whole cycles rather than changing the
shape of one, which is the trade the core's header spells out.

## Building

From this directory:

```sh
reticle build --synth reticle.proj
```

resolves the two packages, writes `reticle.lock`, elaborates the design
and synthesises it:

```text
note: built `mos6502_computer`: 5 module(s) from 5 source(s)
```

Synthesis turns the ROM's `$readmemh` into the memory's initial
contents, reading the file through a provider the binary gives it.

## The program

`sw/hello.s` is a few dozen bytes: set up the stack, walk a string with
`LDA message,Y`, and hand each byte to a `putc` subroutine that polls
`STATUS` and writes `DATA`. It is ordinary 6502 — `PHA` around the poll
because there is one accumulator and the poll needs it, `Y` left alone
so the caller can keep its index there.

`sw/hello.hex` is that file assembled. It uses the `@hex` address
specifications IEEE 1364-2005 §17.2.9 defines, so the six bytes of
vectors reach the top of a 2 KiB ROM without two thousand lines of
padding in front of them:

```text
@000
78
d8
...
@7fa
22
f8
00
f8
22
f8
```

Those last six bytes are NMI → `$F822`, RES → `$F800` and IRQ → `$F822`,
and the test checks each against the label the source gave it.

To change the program, edit `sw/hello.s` and regenerate the hex:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test mos6502_computer hello_hex
```

The assembler is `tests/mos6502_asm/mod.rs`, the same opcode matrix that
runs `mos6502`'s own tests in `tests/ip_library.rs`. It is built from the
**documented encoding table**, never from the core's decoder, which is
what stops a wrong core and a wrong test agreeing with each other;
`docs/writing-a-cpu.md` argues that at length.

## Testing

From the repository root:

```sh
cargo test --all-features --test mos6502_computer
```

| Test | What it shows |
|------|---------------|
| `hello_hex_is_the_assembled_source` | `sw/hello.hex` is `sw/hello.s` assembled, inside the ROM, with the three vectors pointing at the labels the source names |
| `the_assembler_places_a_string_and_its_terminator` | the `.string` directive the example needed, apart from the program |
| `the_project_resolves_and_elaborates` | the manifest builds from exactly two library packages and `rtl/computer_top.v` |
| `the_computer_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch, and the ROM holds `sw/hello.hex` — with every element the hex did not name still `x`, so nothing invented the padding |
| `the_line_comes_out_of_the_serial_wire` | the testbench runs and `Hello from Reticle\n` is decoded from the waveform of `uart_tx` |
| `the_computer_maps_onto_the_hx8k_and_exports_for_nextpnr` | the iCE40 flow fits it on the HX8K with the program and the vectors in the ROM's block RAMs, and writes `computer_top.json` and `computer_top.pcf` |
| `reticle_build_builds_the_project` | `reticle build --synth` on the manifest |
| `reticle_sim_runs_the_testbench` | the line comes out of `reticle sim` too |
| `reticle_fpga_exports_the_rom_with_the_program_in_it` | and `reticle fpga` exports a ROM that is not blank |

The simulation test decodes the text from the serial pin itself, not
from any register inside the design: it records every change of
`uart_tx` with its time and samples each 8N1 frame at 104 clocks per
bit, the rate `computer_top` is built with. The decoder is
`tests/serial/mod.rs`, shared with `tests/soc.rs`. The testbench does the
same in Verilog and prints what it receives with `$display`, and the two
must agree.

The block RAM check reads the program back **out of the netlist**, byte
by byte, out of the `INIT_*` parameters of the four `SB_RAM40_4K` cells
the ROM became — including the six vector bytes, which are the ones a
board would notice first.

## Resource use

The iCE40 flow in `tests/mos6502_computer.rs`, for `ice40-hx8k-ct256`:

| Resource | Used | HX8K has |
|----------|------|----------|
| `SB_LUT4` | 1714 | 7680 |
| flip-flops (`SB_DFF*`) | 151 | 7680 |
| `SB_CARRY` | 146 | — |
| `SB_RAM40_4K` | 6 | 32 |
| `SB_IO` | 3 | — |
| `SB_GB` | 1 | 8 |

The six block RAMs are the ROM (four) and the RAM (two): both are byte
wide, so both map in the `SB_RAM40_4K`'s 512x8 mode, 2048 bytes over four
blocks and 1024 over two. There is one copy of each, because the 6502
makes one access per cycle and so needs only one read port — where
`examples/soc` needs two copies of its register file, one per read port.
Almost all the logic is the core; the logic depth is 23 LUTs, against
`examples/soc`'s 41.

Side by side with the RISC-V system on the same part:

| | `examples/soc` | `examples/mos6502_computer` |
|---|---|---|
| core | `rv32i`, `REGFILE_BRAM=1` | `mos6502`, `DECIMAL_MODE=0` |
| `SB_LUT4` | 2368 | 1714 |
| flip-flops | 372 | 151 |
| `SB_RAM40_4K` | 10 | 6 |
| LUT depth | 41 | 23 |

## On a board

The flow writes `computer_top.json` (the netlist, in the Yosys JSON
format) and `computer_top.pcf` (the pins) to
`target/tmp/mos6502-computer/`. With the
[IceStorm](https://github.com/YosysHQ/icestorm) tools and an
iCE40-HX8K breakout board, the remaining steps would be:

```sh
cd target/tmp/mos6502-computer
nextpnr-ice40 --hx8k --package ct256 --json computer_top.json --pcf computer_top.pcf --asc computer_top.asc
icepack computer_top.asc computer_top.bin
iceprog computer_top.bin
```

and then a terminal on the board's second FTDI serial port at 115200
baud, 8N1 (for example `picocom -b 115200 /dev/ttyUSB1`), which should
show `Hello from Reticle` once per configuration.

**None of that has been done.** This machine has no board attached and
no IceStorm tools installed, and Reticle's own iCE40 place and route uses
a synthetic fabric that cannot program a real part
([`docs/fpga.md`](../../docs/fpga.md)). So what is demonstrated stops at
the files `nextpnr-ice40` reads: every cell in `computer_top.json` a real
primitive, the pins in `computer_top.pcf` ones the board has, and the
ROM's block RAMs holding the program and the vectors.

The same flow runs from the command line, flattening the core and the
UART into `computer_top` on the way:

```sh
reticle fpga --device ice40-hx8k-ct256 --top computer_top \
  --constraints board/hx8k_breakout.rcf \
  rtl/computer_top.v ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/uart/rtl/uart_tx.v ../../ip/uart/rtl/uart_rx.v ../../ip/uart/rtl/uart.v
```

and so does the simulation:

```sh
reticle sim tb/computer_tb.v rtl/computer_top.v ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/uart/rtl/uart_tx.v ../../ip/uart/rtl/uart_rx.v ../../ip/uart/rtl/uart.v
```

which prints `Hello from Reticle`.

## What this proves, and what it does not

Proved here, by `cargo test`:

- the manifest resolves `mos6502` and `uart` from `ip/` and builds,
  through `ip::resolve` and `ip::elaborate` and through `reticle build`,
  with `rtl/computer_top.v` the only user HDL;
- synthesis accepts it with no error, warning or latch;
- the testbench, simulated, carries exactly `Hello from Reticle\n` on
  the serial wire, decoded from the pin's waveform, with the program
  loaded into the ROM by the design's own `$readmemh`, `@` addresses and
  all;
- synthesis gives the ROM that program as its initial contents and
  leaves every byte the file did not name `x`;
- the iCE40 flow maps every cell to an HX8K primitive, fits in under a
  quarter of the part, puts the program *and the three vectors* into the
  ROM's block RAMs, and exports the JSON and PCF that `nextpnr-ice40`
  reads, with no problem left for `check_nextpnr_json` to find;
- `reticle sim` and `reticle fpga` do the same from the command line.

Not proved:

- **that it runs on a board.** No board or IceStorm tools here, and
  where the iCE40 block RAM layout comes from is stated, with its
  confidence, in `src/fpga/devices/ice40.dev`, not checked against a
  part;
- timing: nothing here checks that the design closes at 12 MHz on the
  real part;
- **interrupts.** `irq` and `nmi` are tied low and the program sets I.
  The vectors point at a real `RTI` and the map is complete, but no
  interrupt is taken in this example. `mos6502`'s own tests in
  `tests/ip_library.rs` take them — edge-triggered NMI through the mask,
  level-triggered IRQ, priority, and BRK — and
  [`docs/writing-a-cpu.md`](../../docs/writing-a-cpu.md) says which
  shapes are worth testing.

Unlike `examples/soc`, building this found **no defect in Reticle**.
That is worth stating: the eight the first example found are fixed, and
the second system down the same path — a different processor, a
different bus, a sparse memory image — needed nothing new.
