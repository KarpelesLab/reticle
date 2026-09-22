# The Reticle IP library

The first-party half of phase 8. [`docs/ip.md`](ip.md) describes the
machinery — the manifest formats, the resolver, the bus model, the black
boxes. This document describes the **blocks**: eleven pieces of HDL that
drop into a design the way a crate drops into a Rust program, each with a
manifest, a Rust co-simulation test, and a resource footprint that was
measured rather than guessed.

They live at the top of the repository, in `ip/`, one directory per
package:

```text
ip/
  axil_gpio/    reticle.ip  rtl/axil_gpio.v
  cdc_pulse/    reticle.ip  rtl/cdc_pulse.v
  cdc_sync/     reticle.ip  rtl/cdc_sync.v
  fifo_async/   reticle.ip  rtl/fifo_async.v
  fifo_sync/    reticle.ip  rtl/fifo_sync.v
  i2c_master/   reticle.ip  rtl/i2c_master.v
  pwm/          reticle.ip  rtl/pwm.v
  ram_wrapper/  reticle.ip  rtl/ram_sp.v  rtl/ram_sdp.v
  spi_master/   reticle.ip  rtl/spi_master.v
  timer/        reticle.ip  rtl/timer.v
  uart/         reticle.ip  rtl/uart_tx.v  rtl/uart_rx.v  rtl/uart.v
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
<!-- end footprints -->

### Two things the table shows about the toolchain

Both are gaps in Reticle itself rather than in the blocks, and both are
pinned down by a test so that fixing one fails the test that describes
it.

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

## What is not here yet

The roadmap's list for this phase also names SDRAM and HyperRAM
controllers, an Ethernet MAC, a USB device, HDMI/DVI output and a small
RISC-V core. None of those is here. They are an order of magnitude larger
than what is, and each of them wants something the toolchain does not yet
have — DDR primitives and PLL configuration for the memory controllers
and the video output, a serial-interface engine for USB. The eleven
blocks here are the ones a design needs first and the ones that can be
proved right in a test that runs in a second.
