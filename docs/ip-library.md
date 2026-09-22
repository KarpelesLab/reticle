# The Reticle IP library

The first-party half of phase 8. [`docs/ip.md`](ip.md) describes the
machinery — the manifest formats, the resolver, the bus model, the black
boxes. This document describes the **blocks**: nineteen pieces of HDL
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
  dvi_tx/        reticle.ip  rtl/tmds_encoder.v  rtl/video_timing.v  rtl/dvi_tx.v
  dvi_tx_pll/    reticle.ip  rtl/dvi_tx_pll.v
  eth_mac_rgmii/ reticle.ip  rtl/eth_mac_rgmii.v
  eth_mac_rmii/  reticle.ip  rtl/eth_mac_tx.v  rtl/eth_mac_rx.v  rtl/eth_mac_rmii.v
  fifo_async/    reticle.ip  rtl/fifo_async.v
  fifo_sync/     reticle.ip  rtl/fifo_sync.v
  hyperram_ctrl/ reticle.ip  rtl/hyperram_ctrl.v
  i2c_master/    reticle.ip  rtl/i2c_master.v
  pwm/           reticle.ip  rtl/pwm.v
  ram_wrapper/   reticle.ip  rtl/ram_sp.v  rtl/ram_sdp.v
  rv32i/         reticle.ip  rtl/rv32i.v
  sdram_ctrl/    reticle.ip  rtl/sdram_ctrl.v
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
| `eth_mac_rmii` | `eth_mac_rmii`, `eth_mac_tx`, `eth_mac_rx` | Ethernet MAC over RMII: preamble, frame check sequence, inter-frame gap | — |
| `spiflash_xip` | `spiflash_xip` | execute-in-place SPI flash reader, read only and cache-less | — |
| `sdram_ctrl` | `sdram_ctrl` | SDR SDRAM controller for x16 parts: power-up sequence, refresh, open rows per bank, datasheet timings in nanoseconds | — |
| `hyperram_ctrl` | `hyperram_ctrl` | HyperBus controller: command-address, fixed or variable latency, DDR data and RWDS, register access | — |
| `dvi_tx` | `dvi_tx`, `tmds_encoder`, `video_timing` | DVI output: 640x480, 800x600 and 1280x720 timings, TMDS 8b/10b with DC balance, 10:1 serialisation through DDR outputs | — |
| `dvi_tx_pll` | `dvi_tx_pll` | `dvi_tx` with its five-times clock from the device's PLL | `dvi_tx` |
| `eth_mac_rgmii` | `eth_mac_rgmii` | gigabit Ethernet MAC over RGMII: the RMII MAC's frame logic an octet a cycle behind DDR IO, optional IO delays | `eth_mac_rmii` |

`rv32i`, `eth_mac_rmii` and `spiflash_xip` are the **larger blocks**,
and they are larger in a particular way: each is a whole protocol or a
whole machine rather than a part of one, so each is where a shortcut
would have been invisible. They also fit together. `spiflash_xip`
presents the memory port `rv32i` puts on its instruction side, so a
processor executing straight out of a serial flash is the two of them
and one wire.

`sdram_ctrl` is the first of the blocks that **need a device
primitive**, the ones phase 8 waited on the FPGA backend for: it forwards
its clock to the part through a double-data-rate output register, which
it asks for with a `ddr` attribute on the port, so the ECP5 row of its
footprint carries an `ODDRX1F` and the iCE40 row an `SB_IO` in DDR mode.
`hyperram_ctrl` puts every HyperBus pin through one — nine `IDDRX1F` and
ten `ODDRX1F` on the ECP5 — and asks for an IO delay on CK, the
`DELAYG` that moves each clock edge into the middle of the byte it
clocks. `dvi_tx` serialises through them, and `dvi_tx_pll` gets its
five-times pixel clock from a `clock_mhz` attribute on a net nothing
drives, which is how a design asks for a PLL: an `SB_PLL40_CORE` or an
`EHXPLLL` appears in its footprint, fed from the board's 25 MHz.
`eth_mac_rgmii` is `eth_mac_rmii`'s frame logic — the same `eth_mac_tx`
and `eth_mac_rx`, which the RMII block was split into so both could
share them, eight bits a cycle instead of two — behind DDR registers on
every RGMII pin, with the clock skew RGMII needs available from the IO
delay element: six `DELAYG` in its ECP5 row. The split cost the RMII
block nothing; its footprint did not move by a cell.

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

`sdram_ctrl` takes the same approach further. A manifest has no
arithmetic, so the timings are declared the way a datasheet states them —
`T_RCD_NS`, `T_RP_NS`, `T_RAS_NS`, `T_RC_NS`, `T_RFC_NS`, `T_WR_NS`,
`T_RRD_NS`, `T_REFI_NS` and `T_INIT_US` next to `CLK_MHZ` — and the
cycle counts `TRCD` to `TINIT` are derived parameters whose defaults do
the division: rounded up for a minimum, rounded down for the refresh
interval, which is a maximum. A design states the part and the clock and
never the cycles.

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
- **`sdram_ctrl`** — against an SDRAM model written in the test that
  **enforces the datasheet** rather than storing whatever it is given.
  It is clocked by the forwarded clock, so it samples the command pins
  where the part does, and it records a violation for a command before
  the power-up wait, an ACTIVE before the initialisation sequence or to
  an open bank, a READ or WRITE to a closed bank or inside tRCD, a
  PRECHARGE inside tRAS or tWR, an ACTIVE inside tRP, tRC or tRRD,
  anything inside tRFC or tMRD, a REFRESH with a row open, a refresh
  interval longer than tREFI, a mode word that is not the one asked for,
  and the controller driving `dq` while the part does. Read data is on
  the bus for exactly the one cycle the CAS latency says, with garbage
  either side. The cycle counts it enforces are computed in Rust from the
  nanoseconds, independently of the block. Two parts run the same
  workload — a Micron MT48LC16M16A2 at 75 MHz and CAS latency 2, an ISSI
  IS42S16400 with a smaller geometry at 48 MHz and CAS latency 3: a row
  written and read back with one ACTIVE for all thirty-two accesses,
  each byte enable alone, a row miss and back, every bank at the top of
  the address space, four hundred pseudo-random accesses across banks
  and rows, then four refresh intervals of idling and the data read
  again. And because a model that accepts anything proves nothing,
  `the_sdram_model_catches_a_controller_that_breaks_the_datasheet` gives
  the model a part slower than the controller was built for, one timing
  at a time — tRCD, tRP, tRAS, tRC, tWR and the refresh interval — and
  requires the model to name the rule each one breaks.
- **`hyperram_ctrl`** — against a HyperRAM model that **enforces the
  initial latency**: it counts CK edges from the fall of CS#, reads the
  command-address from the first six, and expects a write's first byte on
  exactly the edge its latency names, so data a clock early is DQ driven
  during the latency count and data a clock late is a first beat with
  nothing on it. It answers reads with RWDS toggling from the first beat,
  drives RWDS during CA the way the part does, doubles every third
  transaction in variable-latency mode as a refresh collision would, and
  checks CS#'s recovery time, whole words, and that the two ends never
  drive the bus together. Between them sit the IO registers as the FPGA
  backend builds them — a DDR output that launches its two halves in
  the next cycle, a DDR input that samples both edges, CK a quarter cycle
  late through the IO delay — with the testbench stepping in quarter
  cycles. The identification and configuration registers are read, and
  then every latency the part has, three to seven clocks, fixed and
  variable, is written to configuration register 0 and used for writes,
  masked writes and reads checked against a reference and against the
  model's own contents. A controller built believing the part is at
  five clocks, or at seven, is caught by the model, and so is one that
  does not wait out the recovery time.
- **`dvi_tx`** — the TMDS encoder is where these blocks are usually
  wrong, so it is tested **exhaustively against the specification's own
  algorithm**, written out in Rust from the DVI 1.0 flow chart with an
  unbounded disparity. A search from zero finds every running disparity
  the algorithm can reach (−8 to +8, in steps of two), and for each of
  those nine states and each of the 256 bytes the encoder is driven into
  the state by the shortest byte sequence, checked on the way, and then
  given the byte: the symbol and the disparity it leaves, read from the
  register, must be the specification's, and the symbol must decode back
  to the byte through an independent decoder. The four control symbols
  are checked too. `video_timing` is walked through a whole frame of
  each of the three modes, every pixel compared with where `de`, the two
  syncs, `x`, `y` and `frame` must be for VESA's and CEA-861's numbers.
  And the whole transmitter is run for two lines of 640 x 480 with a
  pattern on every lane: the serial bits are collected from the DDR
  ports, the clock lane is checked to be 1111100000 at every symbol, the
  symbols are cut at its edges and compared with the reference encoder's
  for every visible pixel on all three lanes, and blanking is checked to
  carry hsync on lane 0 exactly where the mode puts it. The transmitter
  is also asserted to be **one clock domain** — the pixel rate is an
  enable — so `timing::analyze_cdc` finds nothing crossing, and
  `dvi_tx_pll` is taken through the FPGA flow for both families, which
  must build the PLL from the 25 MHz reference within 1 % of 126 MHz and
  put every lane through a DDR register on the PLL's clock.
- **`eth_mac_rgmii`** — the transmitter's pins looped into the
  receiver's, as the RMII test does, through the DDR registers modelled
  as the backend builds them: what the output registers take at one edge
  is on the pins, low nibble then high, for the next cycle, and the input
  registers hand the pair over at the edge after. A 64-octet frame and a
  one-octet frame come back intact with a good check sequence, and the
  transmitter takes an octet **every** cycle, which is what gigabit
  means. The pins are decoded in Rust: TX_CTL's two halves always agree,
  TXC is 2'b01 so it rises with every low nibble, and the octets are
  seven 0x55, 0xD5, the payload and the testbench's own check sequence,
  followed by a gap of at least twelve cycles. A bit flipped in the
  payload arrives with `rx_crc_ok` low, and RX_CTL's halves made to
  disagree mid-frame — the PHY's receive error — raise `rx_error` and
  withhold the frame's last octet. `timing::analyze_cdc` must find
  exactly two domains, `tx_clk` and `rgmii_rxc`, and nothing crossing
  between them.

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

Each block is measured **as the top of its own design**, with the
constraints its own source states as attributes merged in, as
`Constraints::merge_attrs` does in a user's flow. So every port takes an
IO buffer — that is why `axil_gpio` shows 178 `SB_IO` — and a `ddr`
port takes its double-data-rate register. Dropped into a design, the
buffers disappear; the logic and the flip-flops do not.

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
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 18 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 290 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=0 | ECP5 45F | 1 x DCCA, 27 x LUT4, 2 x TRELLIS_DPR16X4, 18 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT4 | 2 x dff, 27 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | LUT6 | 2 x dff, 22 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 2 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 10 x SB_DFFER, 1 x SB_GB, 27 x SB_IO, 290 x SB_LUT4 | 4 |
| `fifo_sync` | `fifo_sync` | WIDTH=8, DEPTH=16, FWFT=1 | ECP5 45F | 1 x DCCA, 27 x LUT4, 2 x TRELLIS_DPR16X4, 10 x TRELLIS_FF, 27 x TRELLIS_IO | 3 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT4 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | LUT6 | 2 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | iCE40 HX1K | 2 x SB_DFFR, 4 x SB_IO, 1 x SB_LUT4 | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=1, STAGES=2 | ECP5 45F | 2 x TRELLIS_FF, 4 x TRELLIS_IO | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT4 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | LUT6 | 3 x dff | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | iCE40 HX1K | 24 x SB_DFFR, 1 x SB_GB, 18 x SB_IO, 1 x SB_LUT4 | 0 |
| `cdc_sync` | `cdc_sync` | WIDTH=8, STAGES=3 | ECP5 45F | 1 x DCCA, 24 x TRELLIS_FF, 18 x TRELLIS_IO | 0 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT4 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | LUT6 | 6 x dff, 4 x lut | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | iCE40 HX1K | 1 x SB_DFFER, 5 x SB_DFFR, 7 x SB_IO, 6 x SB_LUT4 | 1 |
| `cdc_pulse` | `cdc_pulse` | (defaults) | ECP5 45F | 4 x LUT4, 6 x TRELLIS_FF, 7 x TRELLIS_IO | 1 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT4 | 10 x dff, 41 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | LUT6 | 10 x dff, 37 x lut, 1 x memory 16x8, 1 x memrd, 1 x memwr | 3 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | iCE40 HX1K | 8 x SB_CARRY, 128 x SB_DFFE, 41 x SB_DFFR, 1 x SB_DFFS, 2 x SB_GB, 24 x SB_IO, 298 x SB_LUT4 | 4 |
| `fifo_async` | `fifo_async` | WIDTH=8, DEPTH=16 | ECP5 45F | 2 x DCCA, 41 x LUT4, 2 x TRELLIS_DPR16X4, 42 x TRELLIS_FF, 24 x TRELLIS_IO | 4 |
| `uart` | `uart` | CLK_DIV=104 | LUT4 | 13 x dff, 119 x lut | 6 |
| `uart` | `uart` | CLK_DIV=104 | LUT6 | 13 x dff, 109 x lut | 5 |
| `uart` | `uart` | CLK_DIV=104 | iCE40 HX1K | 33 x SB_CARRY, 26 x SB_DFFER, 10 x SB_DFFES, 34 x SB_DFFR, 2 x SB_DFFS, 1 x SB_GB, 24 x SB_IO, 124 x SB_LUT4 | 4 |
| `uart` | `uart` | CLK_DIV=104 | ECP5 45F | 1 x DCCA, 119 x LUT4, 72 x TRELLIS_FF, 24 x TRELLIS_IO | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT4 | 10 x dff, 87 x lut | 6 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | LUT6 | 10 x dff, 80 x lut | 4 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | iCE40 HX1K | 22 x SB_CARRY, 35 x SB_DFFER, 1 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 25 x SB_IO, 72 x SB_LUT4 | 3 |
| `spi_master` | `spi_master` | CPOL=0, CPHA=0, CLK_DIV=4, WIDTH=8 | ECP5 45F | 1 x DCCA, 87 x LUT4, 53 x TRELLIS_FF, 25 x TRELLIS_IO | 6 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT4 | 14 x dff, 116 x lut | 7 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | LUT6 | 14 x dff, 90 x lut | 4 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | iCE40 HX1K | 18 x SB_CARRY, 20 x SB_DFFER, 4 x SB_DFFES, 17 x SB_DFFR, 1 x SB_GB, 30 x SB_IO, 117 x SB_LUT4 | 3 |
| `i2c_master` | `i2c_master` | CLK_DIV=30 | ECP5 45F | 1 x DCCA, 116 x LUT4, 41 x TRELLIS_FF, 30 x TRELLIS_IO | 7 |
| `pwm` | `pwm` | WIDTH=8 | LUT4 | 2 x dff, 23 x lut | 6 |
| `pwm` | `pwm` | WIDTH=8 | LUT6 | 2 x dff, 17 x lut | 4 |
| `pwm` | `pwm` | WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 8 x SB_DFFER, 8 x SB_DFFR, 1 x SB_GB, 21 x SB_IO, 21 x SB_LUT4 | 6 |
| `pwm` | `pwm` | WIDTH=8 | ECP5 45F | 1 x DCCA, 23 x LUT4, 16 x TRELLIS_FF, 21 x TRELLIS_IO | 6 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT4 | 4 x dff, 61 x lut | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | LUT6 | 4 x dff, 51 x lut | 4 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | iCE40 HX1K | 7 x SB_CARRY, 17 x SB_DFFER, 9 x SB_DFFR, 1 x SB_GB, 47 x SB_IO, 56 x SB_LUT4 | 5 |
| `timer` | `timer` | WIDTH=16, PRESCALE_WIDTH=8 | ECP5 45F | 1 x DCCA, 61 x LUT4, 26 x TRELLIS_FF, 47 x TRELLIS_IO | 5 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT4 | 11 x dff, 47 x lut | 2 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | LUT6 | 11 x dff, 38 x lut | 1 |
| `axil_gpio` | `axil_gpio` | WIDTH=8 | iCE40 HX1K | 116 x SB_DFFER, 16 x SB_DFFR, 1 x SB_GB, 178 x SB_IO, 48 x SB_LUT4 | 2 |
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
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | iCE40 HX1K | 220 x SB_CARRY, 1024 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 5047 x SB_LUT4 | 36 |
| `rv32i` | `rv32i` | REGFILE_BRAM=0 | ECP5 45F | 1 x DCCA, 2486 x LUT4, 32 x TRELLIS_DPR16X4, 358 x TRELLIS_FF, 208 x TRELLIS_IO | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT4 | 16 x dff, 2445 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | LUT6 | 16 x dff, 2075 x lut, 1 x memory 32x32, 2 x memrd, 1 x memwr | 28 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | iCE40 HX1K | 220 x SB_CARRY, 2 x SB_DFFE, 292 x SB_DFFER, 66 x SB_DFFR, 1 x SB_GB, 208 x SB_IO, 2355 x SB_LUT4, 4 x SB_RAM40_4K | 34 |
| `rv32i` | `rv32i` | REGFILE_BRAM=1 | ECP5 45F | 1 x DCCA, 4 x DP16KD, 2445 x LUT4, 360 x TRELLIS_FF, 208 x TRELLIS_IO | 34 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT4 | 26 x dff, 305 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | LUT6 | 26 x dff, 283 x lut | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | iCE40 HX1K | 10 x SB_CARRY, 127 x SB_DFFER, 64 x SB_DFFES, 3 x SB_DFFR, 1 x SB_GB, 34 x SB_IO, 301 x SB_LUT4 | 4 |
| `eth_mac_rmii` | `eth_mac_rmii` | IFG_CYCLES=48 | ECP5 45F | 1 x DCCA, 305 x LUT4, 194 x TRELLIS_FF, 34 x TRELLIS_IO | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT4 | 9 x dff, 177 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | LUT6 | 9 x dff, 166 x lut | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | iCE40 HX1K | 7 x SB_CARRY, 97 x SB_DFFER, 3 x SB_DFFES, 8 x SB_DFFR, 1 x SB_GB, 140 x SB_IO, 172 x SB_LUT4 | 4 |
| `spiflash_xip` | `spiflash_xip` | CLK_DIV=2, READ_CMD=8'h03, DUMMY_CYCLES=0 | ECP5 45F | 1 x DCCA, 177 x LUT4, 108 x TRELLIS_FF, 140 x TRELLIS_IO | 4 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT4 | 36 x dff, 429 x lut | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | LUT6 | 36 x dff, 348 x lut | 9 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | iCE40 HX1K | 24 x SB_CARRY, 180 x SB_DFFER, 3 x SB_DFFR, 36 x SB_DFFS, 1 x SB_GB, 121 x SB_IO, 418 x SB_LUT4 | 10 |
| `sdram_ctrl` | `sdram_ctrl` | CLK_MHZ=50, CAS_LATENCY=2 | ECP5 45F | 1 x DCCA, 427 x LUT4, 1 x ODDRX1F, 219 x TRELLIS_FF, 121 x TRELLIS_IO | 10 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT4 | 26 x dff, 214 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | LUT6 | 26 x dff, 187 x lut | 7 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | iCE40 HX1K | 13 x SB_CARRY, 102 x SB_DFFER, 4 x SB_DFFES, 11 x SB_DFFR, 1 x SB_DFFS, 1 x SB_GB, 91 x SB_IO, 213 x SB_LUT4 | 8 |
| `hyperram_ctrl` | `hyperram_ctrl` | ADDR_WIDTH=22, CK_DELAY=100 | ECP5 45F | 1 x DCCA, 1 x DELAYG, 9 x IDDRX1F, 214 x LUT4, 10 x ODDRX1F, 118 x TRELLIS_FF, 91 x TRELLIS_IO | 7 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx` | `dvi_tx` | MODE=0 | LUT6 | 18 x dff, 363 x lut | 7 |
| `dvi_tx` | `dvi_tx` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 569 x SB_LUT4 | 8 |
| `dvi_tx` | `dvi_tx` | MODE=0 | ECP5 45F | 1 x DCCA, 480 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 9 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT4 | 18 x dff, 480 x lut | 9 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | LUT6 | 18 x dff, 363 x lut | 7 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | iCE40 HX1K | 187 x SB_CARRY, 72 x SB_DFFER, 52 x SB_DFFR, 1 x SB_GB, 57 x SB_IO, 569 x SB_LUT4, 1 x SB_PLL40_CORE | 8 |
| `dvi_tx_pll` | `dvi_tx_pll` | MODE=0 | ECP5 45F | 1 x DCCA, 1 x EHXPLLL, 480 x LUT4, 4 x ODDRX1F, 124 x TRELLIS_FF, 57 x TRELLIS_IO | 9 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT4 | 28 x dff, 396 x lut | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | LUT6 | 28 x dff, 349 x lut | 4 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | iCE40 HX1K | 10 x SB_CARRY, 119 x SB_DFFER, 64 x SB_DFFES, 7 x SB_DFFR, 2 x SB_GB, 39 x SB_IO, 393 x SB_LUT4 | 5 |
| `eth_mac_rgmii` | `eth_mac_rgmii` | IFG_CYCLES=12, TX_DELAY=80, RX_DELAY=80 | ECP5 45F | 2 x DCCA, 6 x DELAYG, 5 x IDDRX1F, 394 x LUT4, 6 x ODDRX1F, 190 x TRELLIS_FF, 39 x TRELLIS_IO | 5 |
<!-- end footprints -->

### Five things writing these blocks found

All five were gaps in Reticle itself rather than in the blocks, and each
is pinned down by a test. Two were found by the original eleven blocks,
two by writing the three larger ones, and one by the blocks that need
device primitives. **The first four have since been fixed**, and their
tests now hold the fix rather than the gap; the fifth is described last
and is still open.

**iCE40 flip-flops refused an active-low reset. Fixed.** Every block
resets on `negedge rst_n`, which is the convention the rest of this
repository's IP uses and the one nearly all real HDL uses. Every
`SB_DFF*` primitive the iCE40 database declares resets *high*, and
`fpga::techcells` used to match polarity exactly instead of putting an
inverter in front of the reset net, so it reported `F0310` and left
generic `dff` cells behind — which is why the iCE40 rows used to show
`dff` where the ECP5 rows show `TRELLIS_FF`, and why the iCE40 target
could not finish a design written the usual way.

The fix is the one `asic::library` has always applied to a polarity no
standard cell has: use the primitive with the other polarity and invert
the net feeding it. The inverter is one of the device's own LUTs, and
there is **one per net**, not one per flip-flop, so `cdc_sync`'s reset
costs one `SB_LUT4` between its two flops and `rv32i`'s costs one
between three hundred. The same is done for a clock-enable polarity a
family lacks, which is now expressible in a `.dev` `mode` clause as
`enable_low`. What is *not* inverted is the clock: a wrong edge would
mean a second clock network with its own skew, which is a
physical-design decision rather than a mapper's, so it is still reported.
`fpga::CellMapReport::inverted` lists the nets and
`ice40_flip_flops_take_an_active_low_reset_through_one_inverter` holds
the fix.

**A memory below the block-RAM threshold was left generic. Fixed.**
`fpga::primitives` decided that a memory too small for a block RAM would
be built from flip-flops or LUT RAM, recorded the decision in the report
— and nothing performed it. The `$memrd` and `$memwr` cells stayed, and
`fpga::check_nextpnr_json`, this crate's own netlist checker, then said
`a memory is left: it did not become block RAM or logic`. It was visible
in the table as the `memory 16x8, memrd, memwr` entries on the two FIFOs,
whose 128 bits fall under the 256-bit threshold; the 256 x 8 RAMs above
it mapped to `SB_RAM40_4K` and `DP16KD` cleanly.

The fallback is performed now, and which of the two it is comes from the
device file rather than from the family name. A device that declares a
distributed RAM primitive with a usable port map gets one — the ECP5's
`TRELLIS_DPR16X4`, whose shape Reticle reads off that port map (four
`dout` pins wide, four `raddr` pins deep) so that nothing about 16 x 4 is
written in Rust. A device that declares none, as the iCE40 database does,
gets one flip-flop per bit with a write enable decoded per word and a
multiplexer per read port, and a clocked read port gets its output
register. Several read ports mean several copies of a distributed RAM,
since it has one read port; one array of flip-flops serves them all with
one multiplexer each.

A memory the fallback still cannot build is *named*, not silently
mangled: initial contents that flip-flops cannot be preloaded with, a
write that is not clocked, write ports on different clocks, or more than
`MapOptions::max_logic_bits` (4096 by default, lifted by an explicit
`ram_style`) — a few thousand flip-flops are almost never what was
meant, and "it does not fit" is the useful answer.
`the_logic_fallback_answers_like_the_memory_it_replaced` in
`tests/fpga_flow.rs` drives the same stimulus into a design before and
after the lowering and insists the two answer alike;
`small_memories_become_logic_after_the_fpga_flow` holds the library
half.

**A function call inside an asynchronously reset process warned about
the function's own locals. Fixed.** Writing a CRC step, a decode table or
a sign extension as a Verilog `function` is the readable way to do it,
and calling one from inside `always @(posedge clk or negedge rst_n)` made
flip-flop inference report every argument and every local of that
function as a register that failed to get an asynchronous reset,
`S0013`, once per name. The netlist was always correct, since the call is
inlined into pure combinational logic; the diagnostic was wrong, and it
was enough to stop a block passing `blocks_synthesise_cleanly`, which
insists on no warning at all.

The fix is to warn only about nets the process assigns non-blockingly. A
blocking assignment inside a clocked block is a temporary, computed and
consumed within the cycle, and needs no reset; an inlined function leaves
one per local and per argument.
`function_locals_are_not_reported_as_unreset_registers` holds an
eighteen-line reproduction and now checks the warning is absent.

**A register file with two read ports was declined with a reason that
read like an acceptance. Fixed, twice over.**
`rv32i` keeps x1..x31 in one array with two read ports and one write
port. On the ECP5 the block RAM mapper turned it down with

```text
`DP16KD` has 2 read and 2 write port(s), the memory needs 2 and 1
```

Every comparison in that sentence holds — two reads wanted and two
available, one write wanted and two available — and yet it was a refusal,
because the constraint it left out is that a `DP16KD` port is *either* a
read or a write. The decision was right; the explanation could not be
acted on. It now names the constraint that applies:

```text
`DP16KD` has 2 port(s) and each serves either a read or a write,
but the memory needs 3 (2 read, 1 write)
```

The mapping a real flow applies here is **duplication**: one copy of the
contents per read port, each block with one read and one write port of
its own, all written together from the one writer. That is what the
mapper does now, whenever the memory has exactly one write port and more
read ports than a block can serve — two writers would need the copies
kept in step between them, which the blocks cannot do, and that case is
still declined with the sentence above. `BramMapping::copies` reports the
factor.

With `REGFILE_BRAM = 1` the two reads are clocked, which is the shape a
block RAM has, and the register file maps onto **four `DP16KD`**: two
copies, two blocks wide each, since thirty-two bits do not fit one
block's eighteen. With `REGFILE_BRAM = 0` the same array is read
combinationally and no block RAM does that — the core's own header says
that variant wants distributed RAM or flip-flops — so it takes the logic
fallback instead, thirty-two `TRELLIS_DPR16X4` on the ECP5 and a thousand
flip-flops on the iCE40. Both are in the table.

That asynchronous check is new too, and it matters beyond the register
file: before it, *any* memory over the threshold with a combinational
read was given a block whose clock pin nothing drove.
`a_two_read_port_register_file_is_duplicated_across_block_rams` and
`an_asynchronous_register_file_takes_the_logic_fallback` hold the two
halves, and `regfile_ecp5` in `testdata/fpga/` takes the same shape
through the whole flow.

**A project loses a top that its own sources instantiate with a
parameter override. Open.** `dvi_tx`'s package first shipped the
transmitter and a wrapper around it, `dvi_tx_pll`, which instantiates it
as `dvi_tx #(.MODE(MODE))`. A project whose top is `dvi_tx` then failed
to build with `P0401`, "the project's top `dvi_tx` is not in the
design", although the source is right there. `ip::elaborate` elaborates
the Verilog with `ElabOptions::new(dialect)` and never passes the
project's `top` through `with_top`, so the frontend chooses the roots
itself — the modules nothing instantiates — and elaborates everything
else only as their instances; an instance with a parameter override is
elaborated under a name derived from the override (`leaf$W_1`, even
when the value is the default), so no module keeps the plain name. The
same module elaborates as a top through `verilog::elaborate` with
`with_top`, so only a project build meets it.
`a_project_top_that_is_also_instantiated_with_an_override_is_lost`
holds a ten-line reproduction, and checks that the same design builds
once the override is removed; the likely fix is passing `project.top`
as the elaboration top. Until then the wrapper is a package of its own,
`dvi_tx_pll`, which depends on `dvi_tx` and so never shares its sources.

## What is not here yet

The roadmap's list for this phase also names SDRAM and HyperRAM
controllers, a USB device, HDMI/DVI output and RGMII. None of those is
here yet. They waited on the FPGA backend configuring the device
primitives they need — DDR registers and IO delays for a memory
controller's data strobe, a PLL for the pixel clock a display wants and
for the 480 Mbit/s a USB high-speed engine wants — because writing an
SDRAM controller against primitives the backend cannot emit would
produce a block that simulates and can never be built, which is worse
than not having one.

**That prerequisite is now met**, for both families the repository
ships (see [`docs/fpga.md`](fpga.md)):

- a port with a `ddr` attribute is two bits per pin, registered on both
  edges — in the iCE40's `SB_IO` itself, or in an `IDDRX1F` /
  `ODDRX1F` beside the ECP5's buffer;
- an `io_delay` attribute puts a `DELAYG` between an ECP5 pin and the
  fabric (the iCE40 has no programmable delay, and says so);
- a clock constraint on a net nothing drives instantiates the device's
  PLL, with `fpga::pll::solve` choosing the dividers and the report
  stating the frequency reached and the error: 125 MHz from the ULX3S's
  25 is exact, 74.25 MHz comes out at 75.

What is still missing is the blocks themselves, and a word of caution
for whoever writes them: every one of those primitives is checked
against the device database and the netlist checker, not against
silicon, and the simulator treats a primitive as a black box, so a DDR
block's tests will have to model the two edges at the port the way the
convention in `docs/fpga.md` states them.

RMII was the Ethernet interface to pick first for exactly that reason:
it is single data rate, two bits per edge on a clock the PHY provides,
so the whole MAC is ordinary logic. RGMII, which is DDR on both
directions, is now buildable too.

The `rv32i` core is big: about 2400 LUT4s, which does not fit an iCE40
HX1K's 1280 and does fit an ECP5 45F many times over. That is a
straightforward multi-cycle machine rather than a squeezed one — one
33-bit adder shared by ADD, SUB, both comparisons and every address, two
barrel shifters, 64-bit `mcycle` and `minstret`, and word-wide muxes the
LUT mapper does not pack especially tightly. Making it smaller is worth
doing and is not worth doing before it is right.
