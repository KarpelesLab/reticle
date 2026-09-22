# The Reticle IP library

The first-party half of phase 8. [`docs/ip.md`](ip.md) describes the
machinery — the manifest formats, the resolver, the bus model, the black
boxes. This document describes the **blocks**: fourteen pieces of HDL
that drop into a design the way a crate drops into a Rust program, each
with a manifest, a Rust co-simulation test, and a resource footprint that
was measured rather than guessed.

They live at the top of the repository, in `ip/`, one directory per
package:

```text
ip/
  axil_gpio/     reticle.ip  rtl/axil_gpio.v
  cdc_pulse/     reticle.ip  rtl/cdc_pulse.v
  cdc_sync/      reticle.ip  rtl/cdc_sync.v
  eth_mac_rmii/  reticle.ip  rtl/eth_mac_rmii.v
  fifo_async/    reticle.ip  rtl/fifo_async.v
  fifo_sync/     reticle.ip  rtl/fifo_sync.v
  i2c_master/    reticle.ip  rtl/i2c_master.v
  pwm/           reticle.ip  rtl/pwm.v
  ram_wrapper/   reticle.ip  rtl/ram_sp.v  rtl/ram_sdp.v
  rv32i/         reticle.ip  rtl/rv32i.v
  spi_master/    reticle.ip  rtl/spi_master.v
  spiflash_xip/  reticle.ip  rtl/spiflash_xip.v
  timer/         reticle.ip  rtl/timer.v
  uart/          reticle.ip  rtl/uart_tx.v  rtl/uart_rx.v  rtl/uart.v
```

`ip/` is in the `exclude` list of `Cargo.toml`, so the published `.crate`
does not carry it. The library is HDL, not Rust: nothing under `src/`
reads it — a block reaches a design through `ip::resolve` like any other
package, and the crate stays sans-I/O — so shipping the tree would add
`.v` files to every download without making the crate do anything more.
It is distributed as part of the repository instead.

## The blocks

| Block | Top module | What it is | Depends on |
|-------|------------|------------|------------|
| `fifo_sync` | `fifo_sync` | synchronous FIFO, full / empty / occupancy count, optional first-word fall through | — |
| `cdc_sync` | `cdc_sync` | N-flop clock domain crossing synchroniser, parameterised width and depth | — |
| `cdc_pulse` | `cdc_pulse` | one pulse across two domains through a toggle and a full handshake | `cdc_sync` |
| `fifo_async` | `fifo_async` | asynchronous FIFO, gray-coded pointers, pointer synchronisers | `cdc_sync` |
| `uart` | `uart`, `uart_tx`, `uart_rx` | 8N1 UART, parameterised baud divisor, ready / valid | — |
| `spi_master` | `spi_master` | byte-level SPI master, any CPOL / CPHA | — |
| `i2c_master` | `i2c_master` | byte-level I²C master, 7-bit addressing, clock stretching tolerated | — |
| `pwm` | `pwm` | counter-comparator PWM, duty latched once per period | — |
| `timer` | `timer` | prescaled auto-reload down-counter with a pulse and a sticky interrupt | — |
| `axil_gpio` | `axil_gpio` | AXI4-Lite GPIO subordinate: data, direction and set registers | `cdc_sync` |
| `ram_wrapper` | `ram_sdp`, `ram_sp` | portable block RAM wrappers, single and simple dual port, optional output register | — |
| `rv32i` | `rv32i` | the whole RV32I base integer set, multi-cycle, machine-mode CSRs, traps and interrupts | — |
| `eth_mac_rmii` | `eth_mac_rmii` | Ethernet MAC over RMII: preamble, frame check sequence, inter-frame gap | — |
| `spiflash_xip` | `spiflash_xip` | execute-in-place SPI flash reader, read only and cache-less | — |

The last three are the **larger blocks**, and they are larger in a
particular way: each is a whole protocol or a whole machine rather than a
part of one, so each is where a shortcut would have been invisible. They
also fit together. `spiflash_xip` presents the memory port `rv32i` puts
on its instruction side, so a processor executing straight out of a
serial flash is the two of them and one wire.

Everything is **Verilog-2005**, deliberately: it is the path this
compiler exercises hardest, and it is the dialect every other tool reads.
Each source starts with a README-style header saying what the block does
**and what it does not** — the second half is the useful one, because an
IP block's limits are what a user needs before they commit to it.

## Using one

A block is an ordinary IP package, so a project reaches it with a
`depends` line and nothing else:

```text
# reticle.proj
name blinky
top top
device ice40-hx1k-tq144

source rtl/top.v

depends uart      ^1.0.0 path ../reticle/ip/uart
depends fifo_sync ^1.0.0 path ../reticle/ip/fifo_sync
```

`cdc_sync` is not named there and does not have to be: `axil_gpio`,
`cdc_pulse` and `fifo_async` declare it themselves and `PathProvider`
finds it in the directory next to the one it came from. `docs/ip.md` has
the whole resolution story.

Parameters are overridden at instantiation, as Verilog parameters always
are:

```verilog
fifo_sync #(
    .WIDTH (16),
    .DEPTH (64),
    .FWFT  (1)
) u_fifo (
    .clk (clk), .rst_n (rst_n),
    .wr_en (wr_en), .wr_data (wr_data), .full (full),
    .rd_en (rd_en), .rd_data (rd_data), .empty (empty),
    .count (count)
);
```

Three blocks carry parameters marked *derived* — `ADDR_WIDTH` and
`CNT_WIDTH` on the FIFOs and the RAMs. They are `$clog2` of the depth,
and they are parameters rather than localparams only because
Verilog-2005 has no way to put a computed width in an ANSI port list
otherwise. Never override them; overriding `DEPTH` re-evaluates them,
which `derived_parameters_follow_the_depth_they_come_from` checks.

## How it is tested

`tests/ip_library.rs` is the whole of it, and it runs in the default
build. Every block goes through all of:

| Test | What it proves |
|------|----------------|
| `manifests_parse` | every `reticle.ip` parses, names its own directory, lists files that exist and round-trips through `IpManifest::to_text` |
| `packages_resolve_and_elaborate` | every block builds through `ip::resolve` and `ip::elaborate` from a generated project, dependencies included |
| `blocks_synthesise_cleanly` | `synth::run` reports nothing at all — no error, no warning, and no inferred latch |
| `footprints_match_the_documentation` | the table below is the one this run measured |
| `axil_gpio_matches_the_axi4lite_definition` | `bus::match_ports` finds all nineteen AXI4-Lite signals on the GPIO at the widths its parameters imply |

and then a behavioural co-simulation test through `sim::Simulator`, which
is the part that matters:

- **`fifo_sync`** — occupancy counted word by word as it fills and
  drains, a write while full ignored, a simultaneous read and write, and
  the first-word-fall-through timing: with `FWFT = 1` the word is on
  `rd_data` in the cycle after the write, with no read strobe at all.
- **`cdc_sync`** — `q` follows `d` after exactly STAGES cycles, not one
  fewer.
- **`cdc_pulse`** — two clocks whose periods share no factor, three
  requests offered only while the block says it is free, and exactly
  three pulses counted in the destination domain.
- **`fifo_async`** — sixteen words through a four-word FIFO between a
  50-tick and a 71-tick clock, arriving in order and unmangled.
- **`uart`** — the transmitter's own output wired into the receiver, two
  bytes back to back, both recovered with no framing error; and a
  hand-driven line with a broken stop bit, which `rx_error` catches.
- **`spi_master`** — modes 0 and 3, with a slave model that samples
  `mosi` on the rising edge and presents `miso` on the falling one, so
  the bits are checked where a real slave would look at them; `cs_n` is
  checked to fall before the first edge and rise after the last.
- **`i2c_master`** — a whole seven-bit addressed exchange over an
  open-drain bus model: address for writing, a data byte, a repeated
  start, address for reading, one byte read with a closing NACK and a
  stop. The slave acknowledges each byte and **holds SCL down in the
  middle of one**, which the master has to wait out.
- **`pwm`** — the duty counted over a whole sixteen-cycle period at five
  settings, with the period tick counted too.
- **`timer`** — the gap between interrupts measured and checked against
  `(reload + 1) * (prescale + 1)`, and the sticky flag checked to stay
  until it is cleared.
- **`axil_gpio`** — real AXI4-Lite transactions from Rust: the direction
  register written and read back, the output register reaching the pins,
  the pin state arriving in the input register through its synchroniser,
  the set register ORing bits in, and bits above WIDTH dropped.
- **`ram_wrapper`** — both ports of both shapes, with and without the
  output register, and the read-first behaviour on a write.
- **`rv32i`** — real machine code, assembled in the test and run. See
  [below](#what-the-processor-actually-executes).
- **`eth_mac_rmii`** — the transmitter's own pins looped into the
  receiver, and the frame that comes out compared with the one that went
  in; the same frame with one dibit flipped in the payload, which still
  arrives and whose `rx_crc_ok` is low; the transmitted pins decoded
  independently in Rust and checked against seven octets of 0x55, a
  0xD5 delimiter, the payload and a check sequence computed by the
  testbench's own CRC, with the inter-frame gap counted after it; and a
  hand-driven frame that ends in the middle of an octet, which
  `rx_error` catches and delivers nothing for.
- **`spiflash_xip`** — five word reads against a flash model in the
  style of the `spi_master` slave, which samples `mosi` on the rising
  edge and presents `miso` on the falling one. The command octet and the
  address the flash saw are checked, the word is checked to be the four
  octets assembled little-endian, and `cs_n` is checked to fall a
  divisor before the first edge and to stay low past the last. Then the
  configuration registers are rewritten — a different command, eight
  dummy cycles and half the clock rate — and the reads are checked
  again, with the model expecting the dummy cycles too.

### What the processor actually executes

`rv32i` is the one block where "it simulates" would mean nothing on its
own, so its tests assemble RISC-V machine code — from the base ISA's own
field layout, never from the core's decoder — load it into the
instruction memory and check the architectural state. Both register-file
flavours run every program.

| Test | The program |
|------|-------------|
| `rv32i_builds_constants_and_pc_relative_addresses` | LUI, AUIPC, ADDI, and a write to `x0` that is dropped; the register file read after each instruction |
| `rv32i_computes_every_register_immediate_operation` | all nine of ADDI, SLTI, SLTIU, XORI, ORI, ANDI, SLLI, SRLI, SRAI, including SRAI shifting the sign in and SLTIU sign-extending its immediate before comparing unsigned |
| `rv32i_computes_every_register_register_operation` | all ten of ADD, SUB, SLL, SLT, SLTU, XOR, SRL, SRA, OR, AND, plus a shift amount above 31 to prove only `rs2[4:0]` counts |
| `rv32i_takes_and_declines_every_branch` | each of the six branches given a pair that makes it jump and a pair that makes it fall through, twelve in all, counted rather than positioned |
| `rv32i_jumps_and_links` | JAL and JALR linking the right address, jumping to the right one, and JALR clearing bit 0 of its target |
| `rv32i_loads_and_stores_every_width` | LB, LBU, LH, LHU, LW at every offset in the word, sign extension checked against zero extension, SB and SH landing in one lane and two without touching the rest, and a negative offset |
| `rv32i_survives_memories_that_make_it_wait` | the same program with zero, one and three wait states on both ports |
| `rv32i_traps_on_a_misaligned_access_and_on_nonsense` | a handler at `mtvec` that adds up `mcause` and returns past the faulting instruction, driven through causes 4, 6, 2, 3 and 11, then a JALR to an address that is not a multiple of four, which is cause 0 blamed on the jump |
| `rv32i_reads_and_writes_its_machine_csrs` | CSRRW / CSRRS / CSRRC and the immediate forms over `mstatus`, `mtvec`, `mie`, `mip`, `mhartid`; `mcycle` and `minstret` read twice and their difference checked against the core's own timing; an unknown CSR and a write to a read-only one, both illegal |
| `rv32i_takes_a_timer_interrupt_and_returns_from_it` | a spin loop with MIE and MTIE set, the timer line raised, the handler entered once with cause 0x80000007 and `mepc` inside the loop, MRET returning into it, and the external and software lines with their own causes |
| `rv32i_sums_an_array_in_a_loop` | eight words summed through a `lw` / `add` / `addi` / `bne` loop and the total stored past the end of the array |
| `rv32i_runs_a_recursive_function_on_the_stack` | Fibonacci of ten by the definition: two nested calls per frame, the return address and the argument saved on a stack, 177 calls and about two thousand instructions, and the stack pointer back where it started |

Where a block crosses clock domains, `timing::analyze_cdc` is run over
the synthesised and flattened netlist and the result is *asserted*, not
just printed: `cdc_pulse` must show two two-flop synchronisers and
`fifo_async` two gray buses with the generator found, and neither may
show a single `Unsynchronised` crossing. That is a real check on how the
HDL is written, not on whether it compiles — a synchroniser written as
one shift register instead of separate flops fails it.

### A note on testbench discipline

The two-domain tests present their inputs a fixed `SETUP` before each
edge rather than in the same instant. That is not decoration: an
event-driven simulator is entitled to process a clock edge before the
combinational cone feeding it has finished re-evaluating, so a value
changed in the same instant as the edge that samples it is a race, in
Reticle exactly as in any other simulator and in a real circuit. The
first version of the asynchronous FIFO test drove `wr_en` at the edge and
watched the FIFO overflow, because the flip-flop holding `wr_full`
sampled a stale comparison while the pointer beside it sampled a fresh
one. Present inputs early.

## Resource footprints

Measured, not guessed. Every row below comes from running the block
through this crate's own tools, at the parameters the row names:

- **LUT4** and **LUT6** — `synth::run` followed by
  `synth::techmap::map_module` with `MapOptions::lut(4)` and
  `MapOptions::lut(6)`. Storage is not mapped, so flip-flops appear as
  `dff` cells (one per register, whatever its width) and memories as
  `memory <depth>x<width>` with their ports.
- **iCE40 HX1K** and **ECP5 45F** — the whole `fpga::synthesize_for`
  flow for `ice40-hx1k-tq144` and `ecp5-45f-CABGA381`: generic synthesis,
  block RAM / carry / IO / clock-buffer inference, LUT covering and the
  rewrite to the family's own cells.

Each block is measured **as the top of its own design**, so every port
takes an IO buffer — that is why `axil_gpio` shows 178 `SB_IO`. Dropped
into a design, those disappear; the logic and the flip-flops do not.

`LUT depth` is the depth of the mapped combinational network in cells,
which is the rough shape of the critical path before place and route.

The table is generated by `footprints_match_the_documentation` in
`tests/ip_library.rs` and compared byte for byte, so it cannot drift.
Run the tests with `UPDATE_EXPECT=1` to refresh it after an intended
change, and read the diff: a block that suddenly costs twice as much is
exactly what this table is for.

<!-- footprints: generated by tests/ip_library.rs -->
| Block | Top | Parameters | Target | Cells | LUT depth |
|-------|-----|------------|--------|-------|-----------|
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | LUT4 | 3 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | LUT6 | 3 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | iCE40 HX1K | 8 x SB_CARRY, 1 x SB_GB, 27 x SB_IO, 25 x SB_LUT4, 3 x dff, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | ECP5 45F | 1 x DCCA, 27 x LUT4, 18 x TRELLIS_FF, 27 x TRELLIS_IO, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT4 | 2 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT6 | 2 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | iCE40 HX1K | 8 x SB_CARRY, 1 x SB_GB, 27 x SB_IO, 25 x SB_LUT4, 2 x dff, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | ECP5 45F | 1 x DCCA, 27 x LUT4, 10 x TRELLIS_FF, 27 x TRELLIS_IO, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT4 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT6 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | iCE40 HX1K | 4 x SB_IO, 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | ECP5 45F | 2 x TRELLIS_FF, 4 x TRELLIS_IO | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT4 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT6 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | iCE40 HX1K | 1 x SB_GB, 18 x SB_IO, 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | ECP5 45F | 1 x DCCA, 24 x TRELLIS_FF, 18 x TRELLIS_IO | 0 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT4 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT6 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | iCE40 HX1K | 7 x SB_IO, 4 x SB_LUT4, 6 x dff | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | ECP5 45F | 4 x LUT4, 6 x TRELLIS_FF, 7 x TRELLIS_IO | 1 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT4 | 10 x dff, 41 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT6 | 10 x dff, 37 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | iCE40 HX1K | 8 x SB_CARRY, 2 x SB_GB, 24 x SB_IO, 40 x SB_LUT4, 10 x dff, 1 x memory 16x8, 1 x memrd, 1 x memwr | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | ECP5 45F | 2 x DCCA, 41 x LUT4, 42 x TRELLIS_FF, 24 x TRELLIS_IO, 1 x memory 16x8, 1 x memrd, 1 x memwr | 4 |
| `uart` | `uart` | CLK_DIV=104 | LUT4 | 13 x dff, 119 x lut | 6 |
| `uart` | `uart` | CLK_DIV=104 | LUT6 | 13 x dff, 109 x lut | 5 |
| `uart` | `uart` | CLK_DIV=104 | iCE40 HX1K | 33 x SB_CARRY, 1 x SB_GB, 24 x SB_IO, 123 x SB_LUT4, 13 x dff | 4 |
| `uart` | `uart` | CLK_DIV=104 | ECP5 45F | 1 x DCCA, 119 x LUT4, 72 x TRELLIS_FF, 24 x TRELLIS_IO | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT4 | 10 x dff, 87 x lut | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT6 | 10 x dff, 80 x lut | 4 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | iCE40 HX1K | 22 x SB_CARRY, 1 x SB_GB, 25 x SB_IO, 71 x SB_LUT4, 10 x dff | 3 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | ECP5 45F | 1 x DCCA, 87 x LUT4, 53 x TRELLIS_FF, 25 x TRELLIS_IO | 6 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT4 | 14 x dff, 116 x lut | 7 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT6 | 14 x dff, 90 x lut | 4 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | iCE40 HX1K | 18 x SB_CARRY, 1 x SB_GB, 30 x SB_IO, 116 x SB_LUT4, 14 x dff | 3 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | ECP5 45F | 1 x DCCA, 116 x LUT4, 41 x TRELLIS_FF, 30 x TRELLIS_IO | 7 |
| `pwm` | `pwm` | WIDTH=8 | LUT4 | 2 x dff, 23 x lut | 6 |
| `pwm` | `pwm` | WIDTH=8 | LUT6 | 2 x dff, 17 x lut | 4 |
| `pwm` | `pwm` | WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 1 x SB_GB, 21 x SB_IO, 20 x SB_LUT4, 2 x dff | 6 |
| `pwm` | `pwm` | WIDTH=8 | ECP5 45F | 1 x DCCA, 23 x LUT4, 16 x TRELLIS_FF, 21 x TRELLIS_IO | 6 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT4 | 4 x dff, 61 x lut | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT6 | 4 x dff, 51 x lut | 4 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 1 x SB_GB, 47 x SB_IO, 55 x SB_LUT4, 4 x dff | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | ECP5 45F | 1 x DCCA, 61 x LUT4, 26 x TRELLIS_FF, 47 x TRELLIS_IO | 5 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT4 | 11 x dff, 47 x lut | 2 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT6 | 11 x dff, 38 x lut | 1 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | iCE40 HX1K | 1 x SB_GB, 178 x SB_IO, 47 x SB_LUT4, 11 x dff | 2 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | ECP5 45F | 1 x DCCA, 47 x LUT4, 132 x TRELLIS_FF, 178 x TRELLIS_IO | 2 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT4 | 1 x lut, 1 x memory 256x8, 1 x memrd, 1 x memwr | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT6 | 1 x lut, 1 x memory 256x8, 1 x memrd, 1 x memwr | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | iCE40 HX1K | 27 x SB_IO, 1 x SB_LUT4, 1 x SB_RAM40_4K | 1 |
| `ram_wrapper` | `ram_sp` | WIDTH=8, DEPTH=256, OUT_REG=0 | ECP5 45F | 1 x DP16KD, 1 x LUT4, 27 x TRELLIS_IO | 1 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT4 | 1 x memory 256x8, 1 x memrd, 1 x memwr | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | LUT6 | 1 x memory 256x8, 1 x memrd, 1 x memwr | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | iCE40 HX1K | 36 x SB_IO, 1 x SB_RAM40_4K | 0 |
| `ram_wrapper` | `ram_sdp` | WIDTH=8, DEPTH=256, OUT_REG=0 | ECP5 45F | 1 x DP16KD, 36 x TRELLIS_IO | 0 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | LUT4 | 14 x dff, 2436 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | LUT6 | 14 x dff, 2044 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 28 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | iCE40 HX1K | 220 x SB_CARRY, 1 x SB_GB, 208 x SB_IO, 2325 x SB_LUT4, 14 x dff, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | ECP5 45F | 1 x DCCA, 2438 x LUT4, 358 x TRELLIS_FF, 208 x TRELLIS_IO, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT4 | 16 x dff, 2445 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT6 | 16 x dff, 2075 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 28 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | iCE40 HX1K | 220 x SB_CARRY, 2 x SB_DFFE, 1 x SB_GB, 208 x SB_IO, 2354 x SB_LUT4, 14 x dff, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | ECP5 45F | 1 x DCCA, 2442 x LUT4, 360 x TRELLIS_FF, 208 x TRELLIS_IO, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT4 | 26 x dff, 305 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT6 | 26 x dff, 283 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | iCE40 HX1K | 10 x SB_CARRY, 1 x SB_GB, 34 x SB_IO, 300 x SB_LUT4, 26 x dff | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | ECP5 45F | 1 x DCCA, 305 x LUT4, 194 x TRELLIS_FF, 34 x TRELLIS_IO | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT4 | 9 x dff, 177 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT6 | 9 x dff, 166 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | iCE40 HX1K | 7 x SB_CARRY, 1 x SB_GB, 140 x SB_IO, 171 x SB_LUT4, 9 x dff | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | ECP5 45F | 1 x DCCA, 177 x LUT4, 108 x TRELLIS_FF, 140 x TRELLIS_IO | 4 |
<!-- end footprints -->

### Four things the table shows about the toolchain

All four are gaps in Reticle itself rather than in the blocks, and each
is pinned down by a test so that fixing one fails the test that
describes it. Two of them were found by the original eleven blocks; the
other two by writing the three larger ones.

**iCE40 flip-flops refuse an active-low reset.** Every block resets on
`negedge rst_n`, which is the convention the rest of this repository's IP
uses and the one nearly all real HDL uses. Every `SB_DFF*` primitive the
iCE40 database declares resets *high*, and `fpga::primitives` matches
polarity exactly instead of putting an inverter in front of the reset
net, so it reports `F0310` and leaves generic `dff` cells behind. That is
why the iCE40 rows show `dff` where the ECP5 rows show `TRELLIS_FF`, and
it means the iCE40 target cannot presently finish a design written the
usual way. `ice40_flip_flops_still_refuse_an_active_low_reset` holds the
statement.

**A memory below the block-RAM threshold is left generic.**
`fpga::primitives` decides that a memory too small for a block RAM will
be built from flip-flops or LUT RAM, records the decision in the report —
and nothing performs it. The `$memrd` and `$memwr` cells stay, and
`fpga::check_nextpnr_json`, this crate's own netlist checker, then says
`a memory is left: it did not become block RAM or logic`. It is visible
in the table as the `memory 16x8, memrd, memwr` entries on the two FIFOs,
whose 128 bits fall under the 256-bit threshold; the 256 x 8 RAMs above
it map to `SB_RAM40_4K` and `DP16KD` cleanly.
`small_memories_are_left_generic_after_the_fpga_flow` holds that one.

**A function call inside an asynchronously reset process warns about the
function's own locals.** Writing a CRC step, a decode table or a sign
extension as a Verilog `function` is the readable way to do it, and
calling one from inside `always @(posedge clk or negedge rst_n)` makes
flip-flop inference report every argument and every local of that
function as a register that failed to get an asynchronous reset —
`S0013`, once per name. The netlist is correct: `proc_lower` inlines the
call into pure combinational logic and says so in its own log ("regs to
wires"), and the cells that come out are exactly the ones the function
computes. It is the diagnostic that is wrong, and it is enough to stop a
block passing `blocks_synthesise_cleanly`, which insists on no warning
at all.

`eth_mac_rmii` works around it by calling `crc_step` from a continuous
assignment and using the resulting wire inside the process, which costs
nothing and is silent. `function_locals_are_reported_as_unreset_registers`
holds an eighteen-line reproduction, and checks both that the warning is
still there and that the netlist beside it is right.

**A register file with two read ports is declined with a reason that
reads like an acceptance.** `rv32i` keeps x1..x31 in one array with two
read ports and one write port. On the ECP5 the block RAM mapper turns it
down with

```text
`DP16KD` has 2 read and 2 write port(s), the memory needs 2 and 1
```

Every comparison in that sentence holds — two reads wanted and two
available, one write wanted and two available — and yet it is a refusal.
The constraint the sentence leaves out is that a `DP16KD` port is
*either* a read or a write, so two reads and a write want three ports and
the device has two. The decision is right; the explanation cannot be
acted on.

The mapping a real flow applies here is duplication: two block RAMs
holding the same contents, each with one read port and one write port,
both written together. That is not done either, so the register file
shows up in the table as `memory 32x32, 2 x memrd, memwr` on every
target.
`a_two_read_port_register_file_is_declined_with_a_contradictory_reason`
holds both halves.

## What is not here yet

The roadmap's list for this phase also names SDRAM and HyperRAM
controllers, a USB device and HDMI/DVI output. None of those is here,
and none of them can be until the FPGA backend configures the device
primitives they need: DDR registers and IO delays for a memory
controller's data strobe, a PLL for the pixel clock a display wants and
for the 480 Mbit/s a USB high-speed engine wants. `reticle::fpga` maps
logic, carries, block RAM, IO buffers and clock buffers, and nothing
else; writing an SDRAM controller against primitives the backend cannot
emit would produce a block that simulates and can never be built, which
is worse than not having one.

RMII was the Ethernet interface to pick for exactly that reason: it is
single data rate, two bits per edge on a clock the PHY provides, so the
whole MAC is ordinary logic. RGMII, which is DDR on both directions, is
in the same waiting room as the other four.

The `rv32i` core is big: about 2400 LUT4s, which does not fit an iCE40
HX1K's 1280 and does fit an ECP5 45F many times over. That is a
straightforward multi-cycle machine rather than a squeezed one — one
33-bit adder shared by ADD, SUB, both comparisons and every address, two
barrel shifters, 64-bit `mcycle` and `minstret`, and word-wide muxes the
LUT mapper does not pack especially tightly. Making it smaller is worth
doing and is not worth doing before it is right.
