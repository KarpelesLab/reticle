# An NES-compatible console from the IP library

A console that draws a scrolling landscape with sprites walking across
it, out of a picture unit, a processor and a DVI transmitter from
Reticle's IP library, and puts it on a monitor at 640 x 480.

**There is no game here.** Every commercial NES cartridge is copyrighted
and so is Nintendo's lockout silicon; none of it is in this repository,
none of it is needed, and none of it would work. What runs on this
machine is [`sw/demo.s`](sw/demo.s), written for this example, drawing
the eleven tiles in [`sw/chr.s`](sw/chr.s), drawn for this example. The
*hardware* is a different matter: what a 2A03 does with an opcode and
what a 2C02 does with a nametable byte are publicly documented facts
about a machine, and this is an implementation of them from that
documentation. It is a console, not a way to play anything.

This is [`examples/mos6502_computer`](../mos6502_computer) again with a
picture instead of a serial line, and the two are meant to be read side
by side: same manifest format, same testbench shape, same test list, the
same processor. [`tests/nes.rs`](../../tests/nes.rs) drives it through
every step. **What that proves, and what it does not, is at the end of
this page.**

## What is here

```text
examples/nes/
  reticle.proj            the project: mos6502, ppu2c02 and dvi_tx by path, the ECP5 45F
  rtl/nes_console.v       the console board: the map, the cartridge, sprite DMA
  rtl/nes_video.v         the frame buffer, and the doubling onto 640 x 480
  rtl/nes_top.v           the three of them on one clock
  sw/demo.s               the program, in 6502 assembly
  sw/demo.hex             it assembled, which the cartridge loads with $readmemh
  sw/chr.s                the tiles, drawn twice: as art and as bytes
  sw/chr.hex              them assembled
  tb/nes_tb.v             a testbench that hashes a frame off the video port
  board/ecp5_dvi.rcf      pins and clock for an ECP5 45F with a DVI connector
```

Three library packages and three files of HDL. `mos6502` is the
processor, [`ppu2c02`](../../ip/ppu2c02) is the picture unit and
[`dvi_tx`](../../ip/dvi_tx) is the video output; everything else on this
page is the console *board*, which is the part that is neither.

## The processor is `mos6502` with `DECIMAL_MODE = 0`

That is not an optimisation. The 2A03 in an NES **is** a 6502 with
decimal mode disabled in silicon: D is still a flag, `SED`, `CLD`, `PHP`
and `PLP` all still see it, and `ADC` and `SBC` ignore it and always work
in binary. So the parameter that exists in the library block to save two
adders and a pair of comparators happens to be exactly the difference
between a 6502 and the processor in this console, and setting it to zero
is what makes the core *correct* here, not merely smaller.

`ip/mos6502`'s own tests run every decimal-mode case twice, once with the
parameter on and once with it off, which is the check that the zero
setting still has D as a flag and still does binary arithmetic. This
example is what that setting is for.

## The map

One bus, as on any 6502:

| Address | What | Notes |
|---------|------|-------|
| `$0000`–`$07FF` | work RAM, 2 KiB | zero page, the stack in page one, and the rest |
| `$0800`–`$1FFF` | the same RAM again | only eleven address bits are decoded, so a program really can find its stack at `$1100` |
| `$2000`–`$2007` | the picture unit | |
| `$2008`–`$3FFF` | the same eight registers again | only three address bits are decoded inside the range |
| `$4014` | sprite DMA | writing `$XX` copies `$XX00`–`$XXFF` into OAM and stops the processor for 513 cycles |
| `$4000`–`$4017` | otherwise sound and controllers | not built: writes are dropped, reads give zero |
| `$8000`–`$FFFF` | the cartridge's program ROM | NROM-128, so 16 KiB seen twice and the vectors at the top |

The mirroring rows are not decoration. A program that writes `$07FF` and
reads `$0FFF` gets its byte back on a real console and gets it back here,
because both decode eleven bits and not thirteen; a program that writes
`$3F00` through `$2006` reaches the same register as one writing `$2006`.
Games depend on both.

The cartridge is an **NROM board**, the simplest one there is: 16 KiB of
program ROM with no mapper at all, 8 KiB of pattern ROM on the picture
unit's own bus, and the two nametables wired side by side (vertical
mirroring), which is what a program scrolling sideways wants.

## Clocking: one clock for the whole design

A real console divides one master oscillator by four for the picture unit
and by twelve for the processor, so the picture unit runs three times as
fast. Here there is a single clock, `clk_x5` at five times the 640 x 480
pixel rate, and everything below it is an *enable*:

```text
clk_x5   126 MHz         dvi_tx, the frame buffer's read port
 /24     5.25 MHz        one dot of the picture unit
 /3      1.75 MHz        one cycle of the processor
```

`ppu2c02` moves on its `en` and on nothing else, and `mos6502` performs
exactly one bus access per bus cycle and holds it until it sees `ready`
high at a rising edge — so holding `ready` low for two dots out of three
is all it takes to run the processor at a third of the dot rate, and
holding it low for a few hundred cycles is all it takes to stop it for a
sprite DMA. Every cycle count inside the console is therefore the one the
6502's and the 2C02's tables print.

What the single clock costs is 2.2 %: 126 / 24 is 5.25 MHz where a real
part runs at 5.369, so a frame takes 17.02 ms instead of 16.64 and the
console runs at 58.8 frames a second rather than 60.1. Nothing inside
can tell. A stopwatch can, and so could music — which is one more reason
the sound hardware is not here.

What it buys is that **nothing in this design crosses between clocks**.
There are no synchronisers, no asynchronous FIFO and no timing arcs
between domains, which for a design with a frame buffer written by one
raster and read by another is worth a great deal.

## The screen

The console draws 256 x 240 pixels at its own rate and a monitor wants
640 x 480 at 60 Hz, so `rtl/nes_video.v` puts a frame buffer between
them. Doubled, 256 x 240 is 512 x 480: it fills the height exactly and
leaves a 64-pixel border either side, which is why the mapping is

```text
screen column 64..575  ->  console column (column - 64) / 2
screen row    0..479   ->  console row    row / 2
anything else          ->  black
```

The frame buffer holds the console's six-bit palette index rather than a
colour, so it is 64 Ki x 6 bits and `ppu_palette` turns the index into
RGB one cycle later — inside the four cycles of `clk_x5` that `dvi_tx`
gives a fetch. One buffer, not two, so the two rasters walk past each
other about twice a minute and the picture tears. That is the honest
trade for having no clock crossing.

## The demo

`sw/demo.s` sets the machine up and then does nothing but answer the NMI
once a frame. Each frame it moves four sprites one pixel right, DMAs them
into OAM, advances the colour of the stars, and writes the frame number
as the horizontal scroll — so frame *N* is drawn with the background
scrolled *N* pixels left and the sprites *N* pixels right of where the
tables put them. The background is a night sky with stars and clouds over
a grass line, soil and brick, built row by row from a 30-byte table and
then decorated one tile at a time from a table of addresses.

Two things in it are worth knowing before reading it:

- **it does not wait for the picture unit to warm up.** A program for a
  real console reads `$2002` until vblank twice before it touches
  anything, because a real 2C02 ignores writes for about 30,000 cycles
  after power on. This picture unit is ready the moment reset is
  released, so the wait would buy nothing and would cost a whole frame of
  simulation in every test. It is the one place the program would need a
  change to run on real silicon, and it is written in the source so
  nobody has to find it.
- **the sprites are placed so that sprite zero hits.** Sprite zero sits
  over the soil, where the background is opaque, so the sprite zero flag
  really does go up once a frame. The demo does not use it — the test
  does, on the dot.

`sw/chr.s` is the graphics, and every tile in it is drawn **twice**: once
as eight rows of art in the comment above it, where `.` is the
transparent colour and `1`, `2` and `3` are the other three, and once as
the sixteen bytes of the two bitplanes.

```text
; tile $05 — a cut stone, lit from the left
;   ...33...
;   ..3223..
;   .322223.
;   32222223
;   32222223
;   .322223.
;   ..3223..
;   ...33...
tile_gem:
        .byte $18, $24, $42, $81, $81, $42, $24, $18   ; plane 0
        .byte $18, $3c, $7e, $ff, $ff, $7e, $3c, $18   ; plane 1
```

`the_tiles_are_the_art_drawn_above_them` reads the comment, works out
what the sixteen bytes have to be, and compares — so a typo in either one
is a failing test rather than a wrong pixel nobody notices.

Both files are assembled by `tests/mos6502_asm/mod.rs`, the same opcode
matrix that runs `mos6502`'s own tests, built from the **documented
encoding table** and never from the core's decoder. To change either,
edit the source and regenerate the hex:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test nes hex_is_the_assembled_source
```

## Building

From this directory:

```sh
reticle build --synth reticle.proj
```

resolves the three packages, writes `reticle.lock`, elaborates the design
and synthesises it:

```text
note: built `nes`: 9 module(s) from 9 source(s)
```

Simulation and the FPGA flow from the command line:

```sh
reticle sim tb/nes_tb.v rtl/nes_console.v \
  ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/ppu2c02/rtl/ppu_palette.v ../../ip/ppu2c02/rtl/ppu2c02.v \
  ../../ip/dvi_tx/rtl/tmds_encoder.v ../../ip/dvi_tx/rtl/video_timing.v \
  ../../ip/dvi_tx/rtl/dvi_tx.v
```

which runs the console for two frames and prints

```text
nes_tb: frame 2: 61440 pixels, hash 7f6f6b48
```

and

```sh
reticle fpga --device ecp5-45f-CABGA381 --top nes_top \
  --constraints board/ecp5_dvi.rcf \
  rtl/nes_top.v rtl/nes_console.v rtl/nes_video.v \
  ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/ppu2c02/rtl/ppu_palette.v ../../ip/ppu2c02/rtl/ppu2c02.v \
  ../../ip/dvi_tx/rtl/tmds_encoder.v ../../ip/dvi_tx/rtl/video_timing.v \
  ../../ip/dvi_tx/rtl/dvi_tx.v
```

which writes `nes_top.json` and `nes_top.lpf` for `nextpnr-ecp5`.

## Testing

From the repository root:

```sh
cargo test --all-features --test nes
```

| Test | What it shows |
|------|---------------|
| `demo_hex_is_the_assembled_source` | `sw/demo.hex` is `sw/demo.s` assembled, with the three vectors pointing at the labels the source names |
| `chr_hex_is_the_assembled_source` | and `sw/chr.hex` is `sw/chr.s` assembled, with sprite tile 0 blank because an unused slot still fetches it |
| `the_tiles_are_the_art_drawn_above_them` | all eleven tiles' bytes are the art in the comment over them |
| `the_project_resolves_and_elaborates` | the manifest builds from exactly three library packages and three user sources |
| `the_console_synthesises_without_errors_or_latches` | no error, no warning and no latch, and both cartridge memories hold their hex files — with every byte the files did not name still `x` |
| **`the_frame_comes_out_of_the_video_port`** | **two frames of the demo, and every one of the 61,440 pixels of the second compared against a frame buffer computed independently from the nametable, the tiles and the palette** |
| `the_screen_doubles_the_console_onto_the_tmds_lanes` | and the other half of the chain: a painted frame buffer read back off the four output pins, TMDS symbols decoded, doubled, centred, black border |
| `sprite_zero_hits_on_the_dot_the_pixels_meet` | the flag on exactly the dot the two opaque pixels meet, and never with the background off |
| `nine_sprites_on_a_line_set_the_overflow_flag` | eight on one line and nine on the next |
| `the_write_latch_is_shared_by_2005_and_2006` | one `w` toggle between the two registers, read off the address pins |
| `the_data_port_reads_one_access_behind_and_steps_by_what_2000_says` | the buffered `$2007` read, the palette read that is not buffered, `$3F10` being `$3F00`, and `$2000`'s increment and nametable bits |
| `oam_dma_copies_a_page_and_stops_the_processor` | 513 processor cycles, and nothing retired in them |
| `the_console_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, every cell a primitive, every memory in block RAM, and the files `nextpnr-ecp5` reads |
| `reticle_build_builds_the_project` | `reticle build --synth` on the manifest |
| `reticle_sim_runs_the_testbench` | the same frame hash out of `reticle sim` |
| `reticle_fpga_exports_the_cartridge_with_the_program_in_it` | and `reticle fpga` exports a cartridge that is not blank |
| `dce_keeps_no_reference_to_a_memory_it_removed` | a compiler defect this example found, fixed |
| `lowering_many_small_memories_is_not_quadratic` | and the other one |

**The frame test is the one that matters.** It runs the console for two
whole frames, records `vid_de`, `vid_x`, `vid_y` and `vid_color` — four
pins and nothing else — and compares every pixel of the second frame with
a frame buffer computed in Rust from the nametable the program wrote, the
tiles the cartridge holds and the palette the program sent, by the rules
the 2C02's documentation gives: where a tile comes from, where its
attribute comes from, how two bitplanes become a colour index, which
sprite wins. That model knows nothing about how `ppu2c02` fetches
anything, which is the only reason comparing the two means anything.
[`docs/writing-a-cpu.md`](../../docs/writing-a-cpu.md) section 3 is the
argument, and this is the picture-sized version of
`the_line_comes_out_of_the_serial_wire`.

The Verilog testbench folds the same frame into a hash and prints it, the
Rust test folds its own model into the same hash, and the constant is
checked against the model's side — so the two implementations of the
picture have exactly one number between them and it does not come from
the hardware.

## Resource use

The ECP5 flow in `tests/nes.rs`, for `ecp5-45f-CABGA381`:

| Resource | Used | The 45F has |
|----------|------|-------------|
| `LUT4` | 5409 | 43848 |
| `TRELLIS_FF` | 1154 | 43848 |
| `DP16KD` | 39 | — |
| `ODDRX1F` | 4 | — |
| `TRELLIS_IO` | 5 | — |
| `DCCA` | 1 | 16 |

Logic depth 25 — a four-input LUT on an ECP5, so not directly comparable
with `examples/mos6502_computer`'s 23 on an iCE40, but the same order:
adding a picture unit and a serialiser to a 6502 did not make the
longest path much longer.

Every memory in the machine is in block RAM:

| Memory | Shape | `DP16KD` |
|--------|-------|----------|
| `u_video.fb` | 65536 x 6 | 24 |
| `u_console.prg` | 16384 x 8 | 8 |
| `u_console.chr` | 8192 x 8 | 4 |
| `u_console.ram` | 2048 x 8 | 1 |
| `u_console.nt` | 2048 x 8 | 1 |
| `u_console.u_ppu.oam` | 256 x 8 | 1 |

`src/fpga/devices/ecp5.dev` does not state how many block RAMs the part
has, so the flow does not check the count; the 45F's published figure is
108, which this repository has not verified against a part.

### It does not fit the iCE40 HX8K

`examples/soc` and `examples/mos6502_computer` both fit on an
`ice40-hx8k-ct256`, and this does not. The logic would nearly fit — 5409
four-input LUTs against the HX8K's 7680, though an iCE40 mapping is not
an ECP5 one — but **the memory is nowhere near**: 39 `DP16KD` is 638,976
bits, and the HX8K's 32 `SB_RAM40_4K` are 131,072. The frame buffer alone
is three times the whole part.

There is no clever way around that on this design. A line buffer instead
of a frame buffer would cut it to a few kilobits, but a line buffer needs
the console's raster and the screen's raster locked together, and locking
them means running the picture unit at a rate that is not the one its
cycle counts assume. The trade is stated here rather than taken.

The other half of the problem is the clock: `dvi_tx` runs at five times
the pixel rate, 126 MHz for 640 x 480, and the transmitter's own header
says that is at the edge of what an iCE40 does.

## On a board

The flow writes `nes_top.json` (the netlist, in the Yosys JSON format)
and `nes_top.lpf` (the pins) to `target/tmp/nes/`. With the
[prjtrellis](https://github.com/YosysHQ/prjtrellis) tools and an ECP5
board with a DVI or HDMI connector, the remaining steps would be:

```sh
cd target/tmp/nes
nextpnr-ecp5 --45k --package CABGA381 --json nes_top.json --lpf nes_top.lpf --textcfg nes_top.config
ecppack nes_top.config nes_top.bit
openFPGALoader -b <board> nes_top.bit
```

Two things about this board file are choices rather than facts, and both
are in [`board/ecp5_dvi.rcf`](board/ecp5_dvi.rcf):

- **`clk_x5` is an input pin here, and on a real board it would come from
  the device's PLL.** [`ip/dvi_tx_pll`](../../ip/dvi_tx_pll) is `dvi_tx`
  with exactly that change — a `clock_mhz` attribute on an undriven wire,
  which is how a design asks Reticle's flow for a PLL — and `nes_top`
  would swap `input wire clk_x5` for `input wire clk_ref` and
  `(* clock_mhz = 126 *) wire clk_x5;`. It is a pin here because a design
  whose clock nothing drives cannot be simulated, and this one is
  simulated hard.
- **each TMDS lane is one pin here and one differential pair on a
  connector.** On the ECP5 a pseudo-differential IO standard (LVCMOS33D)
  makes the pair out of one output, which is a line in the constraints
  and not a change to the design.

**None of that has been done.** This machine has no board attached and no
prjtrellis tools installed, and Reticle's own place and route uses a
synthetic fabric that cannot program a real part
([`docs/fpga.md`](../../docs/fpga.md)). What is demonstrated stops at the
files `nextpnr-ecp5` reads.

## What is not modelled

The picture unit's own list is at the top of
[`ip/ppu2c02/rtl/ppu2c02.v`](../../ip/ppu2c02/rtl/ppu2c02.v); the short
version, and what the console adds to it:

- **no sound.** There is no APU: no pulse channels, no triangle, no
  noise, no sample playback, and no frame counter, so nothing raises the
  IRQ a frame counter raises. `$4000`–`$4013` and `$4015` are decoded and
  dropped. This was the last item on the list and it is the one that was
  not reached; a console with a picture and no sound is honestly half a
  console.
- **no controllers.** `$4016` and `$4017` read as zero. The demo needs no
  input, and a shift register that nothing tests is not worth shipping.
- **no mapper.** NROM only: 16 KiB of program ROM, 8 KiB of pattern ROM,
  fixed mirroring, no bank switching and no cartridge RAM. Most cartridges
  are not NROM.
- **8 x 8 sprites only.** `$2000` bit 5 is stored and ignored.
- **the sprite overflow bug is not reproduced.** The flag goes up when a
  ninth in-range sprite is found, which is what the flag is for; the real
  evaluator also advances its byte index when it should not, so it both
  misses overflows and invents them.
- **no colour emphasis**, no odd-frame dot skip, no open bus, and no
  `$2007` access to pattern or nametable memory while rendering.
- **no composite video.** The output is a palette index per pixel, turned
  into RGB by a table written for this repository. A 2C02 has no colours
  in it; what a screen showed depended on the screen.

## What this proves, and what it does not

Proved here, by `cargo test`:

- the manifest resolves `mos6502`, `ppu2c02` and `dvi_tx` from `ip/` and
  builds, through `ip::resolve` and `ip::elaborate` and through `reticle
  build`, with three files of user HDL;
- synthesis accepts it with no error, warning or latch, and loads both
  cartridge memories from the hex files, leaving every byte the files did
  not name `x`;
- **the console, simulated for two frames, puts on its video port exactly
  the 61,440 pixels that the nametable, the pattern table, the palette
  and the scroll say it should** — background, attributes, fine and
  coarse scroll across two nametables, four sprites with horizontal and
  vertical flip, palette selection and behind-the-background priority,
  every pixel compared, with the model computed from the documentation
  and not from the hardware;
- the sprite zero flag goes up on the dot the two opaque pixels meet, and
  not before;
- the `$2005`/`$2006` write latch is one bit shared by both registers and
  a read of `$2002` puts it back, observed on the address pins;
- sprite DMA moves a page into OAM in 513 processor cycles with the
  processor stopped for all of them;
- the frame buffer, the doubling, the palette and the transmitter carry a
  painted picture out to the four TMDS pins, decoded back from ten-bit
  symbols with the inverse of what DVI 1.0 specifies;
- the ECP5 flow maps every cell to a device primitive, puts every memory
  into block RAM with the cartridge's contents in it, and exports the
  JSON and LPF `nextpnr-ecp5` reads, with no problem left for
  `check_nextpnr_json` to find;
- `reticle build`, `reticle sim` and `reticle fpga` do the same from the
  command line, and the testbench's own hash of the frame agrees with the
  model's.

Not proved:

- **that it runs on a board.** No board and no prjtrellis tools here, and
  where the ECP5 primitive descriptions come from is stated, with its
  confidence, in `src/fpga/devices/ecp5.dev`, not checked against a part.
  Nothing here has been on a screen.
- **timing.** Nothing checks that the design closes at 126 MHz on the
  real part, and 126 MHz through a TMDS serialiser is the most demanding
  thing in this repository. It is entirely possible that it does not.
- **that a commercial game would run.** It would not, and that is not an
  accident: there is no mapper, no sound, no controller and no 8 x 16
  sprite, and every cartridge worth naming needs at least three of those.
  This console runs the program in `sw/`.
- **the whole picture through the whole chain in one simulation.** The
  frame test proves the console's 61,440 pixels and the TMDS test proves
  the screen's, but they are two runs: one frame of 640 x 480 through the
  transmitter is 2.1 million cycles of a design this size, which is
  minutes of simulation for the one link the two tests already cover from
  either end.
- **anything about sound**, because there is none.

Building this found **two defects in Reticle**, both in the FPGA flow and
both fixed, with a regression test each at the bottom of `tests/nes.rs`:
dead code elimination panicked when a dead expression still named a
memory that had just been removed, and `unique_name` looked up every
candidate with a scan of the whole module, so a memory lowered into
hundreds of cells cost a scan per cell per cell — six minutes for this
picture unit, twenty seconds after.
