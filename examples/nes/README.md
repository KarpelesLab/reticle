# An NES-compatible console from the IP library

A console that draws a scrolling landscape with sprites walking across
it, out of a picture unit, a processor and a video transmitter from
Reticle's IP library, and puts it on a monitor at 640 x 480.

Two boards, one console. `nes_top` drives DVI, for an ECP5 with a
digital video connector; `nes_basys3` puts VGA on the end, through
[`vga_out`](../../ip/video/vga_out), for a Digilent Basys 3 — which has a
resistor ladder and a DE-15 socket and no DVI connector at all, and
whose part has no double-data-rate register for one either. The console,
the cartridge and the frame buffer are the same in both.

And a third top with no monitor on the end at all: `nes_jpeg` encodes
the frame with [`mjpeg_enc`](../../ip/video/mjpeg_enc) and gives back a
`.jpg`. [`testdata/nes/frame.jpg`](../../testdata/nes/frame.jpg) is one
the hardware wrote.

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
  reticle.proj            the project: mos6502, ppu2c02, dvi_tx, vga_out, mjpeg_enc
  rtl/nes_console.v       the console board: the map, the cartridge, sprite DMA
  rtl/nes_video.v         the frame buffer, and the doubling onto 640 x 480
  rtl/nes_luma.v          a palette index as one eight-bit sample
  rtl/nes_top.v           one top: the three of them on one clock, DVI on the end
  rtl/nes_basys3.v        the next: two dividers, vga_out, and the same console
  rtl/nes_jpeg.v          the third: a frame buffer walked in block order, into mjpeg_enc
  sw/demo.s               the program, in 6502 assembly
  sw/demo.hex             it assembled, which the cartridge loads with $readmemh
  sw/chr.s                the tiles, drawn twice: as art and as bytes
  sw/chr.hex              them assembled
  tb/nes_tb.v             a testbench that hashes a frame off the video port
  tb/nes_vga_tb.v         one that draws a whole frame and records its VGA pins
  board/ecp5_dvi.rcf      pins and clock for an ECP5 45F with a DVI connector
  board/basys3.rcf        pins and clock for the Basys 3's XC7A35T
```

Five library packages and six files of HDL. `mos6502` is the processor,
[`ppu2c02`](../../ip/video/ppu2c02) is the picture unit,
[`dvi_tx`](../../ip/video/dvi_tx) and [`vga_out`](../../ip/video/vga_out)
are the two video outputs and
[`mjpeg_enc`](../../ip/video/mjpeg_enc) is the encoder; everything else
on this page is the console *board*, which is the part that is none of
them. `vga_out` depends on `dvi_tx` for the raster itself, so
`video_timing` is in the design once and not twice.

## The processor is `mos6502` with `DECIMAL_MODE = 0`

That is not an optimisation. The 2A03 in an NES **is** a 6502 with
decimal mode disabled in silicon: D is still a flag, `SED`, `CLD`, `PHP`
and `PLP` all still see it, and `ADC` and `SBC` ignore it and always work
in binary. So the parameter that exists in the library block to save two
adders and a pair of comparators happens to be exactly the difference
between a 6502 and the processor in this console, and setting it to zero
is what makes the core *correct* here, not merely smaller.

`ip/cpu/mos6502`'s own tests run every decimal-mode case twice, once with the
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

`nes_jpeg` has no raster on the far side at all, so its only divider is
the one that makes the dot: `DOT_DIV` cycles of `clk` per dot, exactly
as `nes_top` takes it. **It defaults to one, which is not a board's
number** — a board would divide down to 5.25 MHz the way the other two
tops do. One is what makes the whole thing simulable, because a frame is
then 89,342 clocks instead of two million, and the picture is identical
either way: every part of the console is clocked by `en` and none of it
can tell how often that comes.

`nes_basys3` is the same arrangement on a different oscillator, and with
no PLL at all:

```text
clk      100 MHz         vga_out, the frame buffer's read port
 /4       25.000 MHz     one pixel of the 640 x 480 raster
 /19       5.263 MHz     one dot of the picture unit
 /3        1.754 MHz     one cycle of the processor
```

25.000 MHz is 0.7 % below the mode's nominal 25.175, a 59.6 Hz frame,
which is what every Basys 3 VGA design does and what monitors take.
5.263 MHz is 2.0 % below a real picture unit's 5.369, which is very
slightly closer than the ECP5 build's 2.2 %. `dvi_tx` needed a PLL
because it runs at five times the pixel rate; VGA does not, so there is
none in that build.

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
gives a fetch, and inside the four cycles of the 100 MHz clock that
`vga_out` gives one on the Basys 3. One buffer, not two, so the two
rasters walk past each other and the picture tears: the two frame rates
are 1.24 Hz apart on the ECP5 build and 0.61 Hz apart on the Basys 3, so
the seam walks down the picture about five times every four seconds on
one board and about once every one and a half seconds on the other. That
is the honest trade for having no clock crossing.

### Four bits a channel, on the Basys 3

The Basys 3's video output is twelve bits — four per channel, through a
resistor ladder — and `vga_out` **truncates** the palette's eight bits
to them rather than rounding, for the reasons
[`ip/video/vga_out/README.md`](../../ip/video/vga_out) gives. That is a
real change to the picture and not a rounding error, so here is the
size of it.

`ppu_palette`'s sixty-four entries are not sixty-four colours even at
full depth: ten of them are black — `$xE` and `$xF` are black on a real
part, and this table gives `$0D` and `$1D` black too — and `$20` and
`$30` are both the same white. So the DVI path renders **54** distinct
colours. Truncating each channel to four bits merges exactly **one**
more pair, leaving **53**:

| Entries | Full depth | On the Basys 3 |
|---------|------------|----------------|
| `$09` | `#083A00` | `#003300` |
| `$0B` | `#003C00` | `#003300` |

Two dark greens a shade apart become one dark green. Nothing else in the
palette collides, and `sw/demo.s` uses neither entry, so this demo looks
the same on both boards — but a program that used both would lose the
difference, and that is what four bits a channel costs.
`four_bits_a_channel_merges_one_pair_of_the_palette` in
[`tests/nes.rs`](../../tests/nes.rs) holds both counts and names the
pair, so the numbers on this page cannot drift away from the table.

There is no dithering and no rounding here, and there should not be:
four bits is what the board has. Rounding to nearest instead of
truncating would buy half a step in the middle of the range, and would
cost a saturating add per channel to stop `8'hFF` carrying out of a
four-bit field — and a four-bit R-2R ladder built from 1 % resistors has
more error than half a step in its own components.

## The third output: the picture as a JPEG file

`nes_top` puts the picture on a monitor over DVI and `nes_basys3` puts
it on one over VGA. `nes_jpeg` puts it nowhere: it encodes the frame
with [`mjpeg_enc`](../../ip/video/mjpeg_enc) and hands back every byte
of a `.jpg`, from its `SOI` to its `EOI`. Run it once a frame and the
bytes are Motion JPEG.

[`testdata/nes/frame.jpg`](../../testdata/nes/frame.jpg) is one of
those files, written by the hardware in simulation and committed. 8370
bytes, 256 x 240, greyscale, the demo's second frame. **It opens** —
that is the point of keeping the file rather than a hash of it.

Four things stand between the console and the file, and only one of
them is new:

| | what it is |
|---|---|
| the frame buffer | 64 Ki palette indices, the same memory `nes_video` has |
| [`nes_luma`](rtl/nes_luma.v) | `ppu_palette`'s RGB weighted into one sample |
| the address generator | the buffer walked in block order, in `nes_jpeg` |
| [`mjpeg_enc`](../../ip/video/mjpeg_enc) | the transform, the quantiser, the coder and the markers |

### The frame buffer is why this is cheap here

`mjpeg_enc` takes its samples in **block order**: 64 for one block,
row-major inside it, then the next block across. Not raster order —
eight buffered lines of a 640-wide picture is four block RAMs and a
module about line buffering rather than about JPEG, which is the
argument at the top of the encoder's own header.

This design has a frame buffer already, because the console's 5.25 MHz
raster and a monitor's 25 MHz one have nothing to do with each other.
So the encoder costs it no buffering at all: the address generator
counts `{block row, row in block}` for the Y and `{block column, column
in block}` for the X, and that is the whole of it. **256 x 240 is
exactly 32 x 30 blocks**, both dimensions a multiple of eight, so the
encoder's one rule about picture size is met without padding anything.
That is luck, and it is why this example is the one the encoder was
wired to first.

The counters stand still until the encoder takes the sample and then
spend one cycle waiting for the memory, so a sample every two cycles at
best. It does not matter: `jpeg_fdct` takes **648 cycles a block**,
which is 622,080 for this picture's 960 blocks, five times what the
feed could deliver. `in_ready` is low for most of a frame.

### Greyscale, and one palette

The encoder is greyscale and the console's picture is a six-bit palette
index, so `nes_luma` sits between them: the index goes to
`ppu_palette` — the same table the DVI and VGA tops use — and its RGB
is weighted by BT.601's coefficients at eight bits,
`(77 R + 150 G + 29 B) / 256`, as twelve shifted terms and no
multiplier. The weights sum to exactly 256, so white is 255.

**There is no second palette.** A 64-entry luma table would have been
smaller than the arithmetic and it would have been the palette written
twice, which is the thing `mjpeg_enc` refuses to do with quantisation
tables for the same reason. `the_luma_is_the_luma_of_the_palettes_own_colours`
checks all 64 against the independent transcription the VGA tests use.

### One whole frame, and the write that has to be dropped

`arm` asks for a frame. The controller waits for the start of the next
one, lets the console fill the buffer for exactly that frame, and at the
following frame boundary freezes the writes and starts the encoder. So
the file is one frame as the console drew it, with no seam — which
`nes_video` does not promise and does not need to.

What is frozen is the buffer, not the console: it goes on running and
its writes are dropped while the encode is in flight. That is what a
capture slower than its source does, and it is why there is no second
buffer — two would be another 24 `DP16KD`.

One off-by-one is worth the sentence. `frozen` is a register, so it is
still low on the edge that sets it, and that edge is dot 0 of line 0 of
the *next* frame, where `vid_de` is already high. Without the term that
drops that one write, the file is one frame plus the top-left pixel of
the one after it. `the_captured_frame_is_one_whole_frame` catches it:
it reads the buffer out of the simulated memory after `done` and hashes
it, and the hash is `FRAME_HASH` — the same frame that
`the_frame_comes_out_of_the_video_port` holds against the model pixel
by pixel.

### What came out

Measured by `the_frame_comes_out_as_a_jpeg_file`, at a flat
quantisation table of 16 and Annex K's Huffman tables:

| | |
|---|---|
| the file | 8370 bytes, 1.09 bits a pixel |
| what ffmpeg calls it | `mjpeg (Baseline), gray(bt470bg), 256x240` |
| what ffmpeg's decode is worth | **39.8 dB** against the samples that went in |
| the first byte to `done` | 713,640 clocks |
| the release of reset to `done` | 892,913 clocks, two console frames of which are the console drawing |

The quantisation table is flat on purpose: a table whose entries are
all equal cannot be put in the wrong order, which keeps the zig-zag of
T.81 A.3.4 out of this example entirely — it is the encoder's business
and its own corpus checks it against a varied table.

The Huffman tables are read out of
`testdata/ip/mjpeg_enc_corpus.txt` rather than written again here. They
are data: `mjpeg_enc` derives its codes from whatever table it is given
and writes that same table into the file, so this repository holds
Annex K once.

What it costs on an ECP5, and the part it does not fit, are under
[Resource use](#resource-use) with the other two tops.

### What it would take to get the bytes off a part

Nothing in this design is a transport: `out_byte`, `out_valid` and
`out_ready` end at a port. What would have to go behind them, and what
it would have to carry:

8370 bytes is one frame at this quantisation table, and the encode is
713,640 clocks. At 60 MHz that is 11.9 ms, which is less than the 17.0
ms the console spends drawing a frame — so the steady state with `arm`
held high is one file every two frames, about **29 a second**, or
246 KB/s. Full-speed USB carries about 1 MB/s of bulk data in practice,
so it would fit with room to spare, and the Cynthion's ULPI PHY is high
speed anyway.

**That arithmetic assumes a 60 MHz clock this design has not been shown
to reach.** 25 levels of four-input LUT is deep, and
`docs/fpga-trellis.md` puts 19 levels below 60 MHz on an ECP5 — which
that page is careful to call an inference from the level count and a
placement experiment rather than a measurement, because Reticle's delay
model for this family is placeholders and the slack it reports is a
count of levels and not a time. Halve the clock and the encode no
longer fits inside a frame and the rate falls with it.

And a host would see a byte stream, not a camera. There is no USB video
class in `ip/usb/` — it is CDC-ACM, a full-speed device, a ULPI device,
a host, a hub and a proxy — so the near thing is `usb_cdc_acm` and a
program on the other end that cuts the stream at `FF D8` and writes
files. A device a video-conferencing application opens by itself is a
different and much larger piece of work, and so is H.264: baseline AVC
is intra prediction, motion estimation, CAVLC and a deblocking filter,
each of them comparable to this whole encoder, and the motion
estimation alone wants reference frames this part has not got the
memory for. Motion JPEG is the codec that fits.

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

resolves the four packages, writes `reticle.lock`, elaborates the design
and synthesises it:

```text
note: built `nes`: 9 module(s) from 11 source(s)
```

Simulation and the FPGA flow from the command line:

```sh
reticle sim tb/nes_tb.v rtl/nes_console.v \
  ../../ip/cpu/mos6502/rtl/mos6502.v \
  ../../ip/video/ppu2c02/rtl/ppu_palette.v ../../ip/video/ppu2c02/rtl/ppu2c02.v \
  ../../ip/video/dvi_tx/rtl/tmds_encoder.v ../../ip/video/dvi_tx/rtl/video_timing.v \
  ../../ip/video/dvi_tx/rtl/dvi_tx.v
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
  ../../ip/cpu/mos6502/rtl/mos6502.v \
  ../../ip/video/ppu2c02/rtl/ppu_palette.v ../../ip/video/ppu2c02/rtl/ppu2c02.v \
  ../../ip/video/dvi_tx/rtl/tmds_encoder.v ../../ip/video/dvi_tx/rtl/video_timing.v \
  ../../ip/video/dvi_tx/rtl/dvi_tx.v
```

which writes `nes_top.json` and `nes_top.lpf` for `nextpnr-ecp5`. The
Basys 3 target is the same manifest with a different top, device and
board file, so it is a command line rather than a second `device` line:

```sh
reticle fpga --device xc7a35t-cpg236 --top nes_basys3 \
  --constraints board/basys3.rcf \
  rtl/nes_basys3.v rtl/nes_console.v rtl/nes_video.v \
  ../../ip/cpu/mos6502/rtl/mos6502.v \
  ../../ip/video/ppu2c02/rtl/ppu_palette.v ../../ip/video/ppu2c02/rtl/ppu2c02.v \
  ../../ip/video/dvi_tx/rtl/video_timing.v ../../ip/video/vga_out/rtl/vga_out.v
```

which writes `nes_basys3.v`, `nes_basys3.xdc` and `nes_basys3.tcl` for
Vivado. Note what is *not* on that command line: `tmds_encoder.v` and
`dvi_tx.v`, because the VGA path needs only `video_timing` out of that
package.

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
| `the_project_resolves_and_elaborates` | the manifest builds from exactly five library packages and six user sources |
| `the_console_synthesises_without_errors_or_latches` | no error, no warning and no latch, and both cartridge memories hold their hex files — with every byte the files did not name still `x` |
| **`the_frame_comes_out_of_the_video_port`** | **two frames of the demo, and every one of the 61,440 pixels of the second compared against a frame buffer computed independently from the nametable, the tiles and the palette** |
| `the_screen_doubles_the_console_onto_the_tmds_lanes` | and the other half of the chain: a painted frame buffer read back off the four output pins, TMDS symbols decoded, doubled, centred, black border |
| **`the_frame_comes_out_of_the_vga_pins`** | **both halves at once on the other board: all 307,200 pixels of a 640 x 480 frame decoded off the twelve colour pins and the two syncs, against the same model doubled and truncated to four bits a channel** |
| `the_palette_is_the_one_the_tmds_test_reads_by_hand` | the two transcriptions of the palette in the test file agree |
| `four_bits_a_channel_merges_one_pair_of_the_palette` | 54 colours become 53 on this board, and which pair stops being two |
| `sprite_zero_hits_on_the_dot_the_pixels_meet` | the flag on exactly the dot the two opaque pixels meet, and never with the background off |
| `nine_sprites_on_a_line_set_the_overflow_flag` | eight on one line and nine on the next |
| `the_write_latch_is_shared_by_2005_and_2006` | one `w` toggle between the two registers, read off the address pins |
| `the_data_port_reads_one_access_behind_and_steps_by_what_2000_says` | the buffered `$2007` read, the palette read that is not buffered, `$3F10` being `$3F00`, and `$2000`'s increment and nametable bits |
| `oam_dma_copies_a_page_and_stops_the_processor` | 513 processor cycles, and nothing retired in them |
| `the_luma_is_the_luma_of_the_palettes_own_colours` | all 64 of `nes_luma`'s samples against the palette's own RGB, weighted independently |
| `the_captured_frame_is_one_whole_frame` | what `nes_jpeg` hands the encoder is one frame of the demo and not a torn pair, hashed against the frame the test above checked pixel by pixel |
| **`the_frame_comes_out_as_a_jpeg_file`** | **the picture back as 8370 bytes of `.jpg`, `SOI` to `EOI`, byte for byte against a file ffmpeg decoded** |
| `the_console_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, every cell a primitive, every memory in block RAM, and the files `nextpnr-ecp5` reads |
| `the_jpeg_build_maps_onto_the_ecp5` | and what the encoding build comes to: 46 block RAMs, which is more than a Cynthion's part is specified to have |
| `the_console_maps_onto_the_artix7_for_the_basys3` | the 7-series flow fits `nes_basys3` on an XC7A35T with no PLL and no DDR anywhere, and writes the netlist, the XDC and the Vivado script |
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

### And off the VGA pins

`the_frame_comes_out_of_the_vga_pins` takes that model the rest of the
way. `tb/nes_vga_tb.v` wires the console to `nes_video` and `vga_out`
exactly as `rtl/nes_basys3.v` does, and the test samples **pins**: the
twelve colour bits and the two sync bits of a VGA socket. It rebuilds
the 640 x 480 frame from the raster's own arithmetic in
[`tests/video/mod.rs`](../../tests/video/mod.rs) — the decoder
`examples/apple2` uses for the same job — doubles and centres the
console's 256 x 240, truncates the reference palette to four bits a
channel, and compares all 307,200 pixels including the black border.

The DVI half could never be one simulation: `dvi_tx` runs at five times
the pixel rate, so a whole frame through it is 2.1 million cycles. VGA
needs no faster clock, and the testbench is arranged so that only the
work that has to happen does:

- the console runs at **one dot per clock**, the fastest it goes, with
  the raster held in reset. It moves on `en` and on nothing else, so the
  frames it draws are the ones the board draws at one dot in nineteen;
- when frame 1 is complete the console's `en` goes low **for good**, on
  a frame boundary — `vid_frame` marks dot 0, where no pixel is being
  written — so the buffer is frozen and there is no tearing to reason
  about;
- only then does the raster go, for **one** frame, which is its frame 0:
  nothing is spent looking for a frame boundary and nothing is spent
  drawing a frame nobody reads.

That is 180,000 clocks of console and 420,000 of raster, and the test
costs about 62 seconds. It is the longest test in the file and runs on
its own thread, so `cargo test --test nes` goes from about 43 seconds to
about 61.

One thing about it is a property of the testbench and not of the board:
at one pixel per clock the frame buffer's read register costs a whole
pixel, so the colour a pin carries is the one the raster asked for *two*
pixels earlier rather than one. On the board the raster takes one pixel
in four and that register settles inside the other three. The test shifts
by two and says so; with the wrong number, 8,720 of the 307,200 pixels
come out wrong, which is the tile edges.

## Resource use

The ECP5 flow in `tests/nes.rs`, for `ecp5-45f-CABGA381`:

| Resource | Used | The 45F has |
|----------|------|-------------|
| `LUT4` | 4946 | 43848 |
| `TRELLIS_FF` | 1154 | 43848 |
| `CCU2C` | 313 | — |
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

### And on the Basys 3's XC7A35T

The 7-series flow in `tests/nes.rs`, for `xc7a35t-cpg236`:

| Resource | `nes_basys3` | `nes_top` on the same part | The XC7A35T has |
|----------|--------------|----------------------------|-----------------|
| `LUT6` | 3887 | 4184 | 20800 |
| `CARRY4` | 81 | 137 | — |
| flip-flops | 1082 | 1154 | 41600 |
| `RAMB18E1` | 39 | 39 | 100 |
| `BUFG` | 1 | 1 | 32 |
| logic depth | 23 | 24 | — |

The middle column is a measurement and not a buildable target, and it is
an **older** one: the left column is what `tests/nes.rs` reports today
and the middle is what `docs/fpga.md` recorded when the swap was made,
so the saving below is that round's and not a subtraction of these two.
`nes_top` cannot be built *for a board* on this part — its four TMDS ports have
nowhere to go, which the next section quotes — so that column is what
the flow reports when it is asked for the netlist and not for pins. It
is here because it is the fair way to say what the swap saves: **398
LUT6, 57 CARRY4 and 72 flip-flops**, and a level of logic depth. That is
three TMDS encoders, their running-disparity registers and three 10:1
serialisers gone, and most of the carry chain goes with the encoders,
which count the ones in a byte. The memory does not move, because the
frame buffer is on the console's side of the swap and the console is
untouched.

For comparison, `examples/apple2` saved 269 LUT6 and 92 flip-flops on
the same change — a similar number for a much smaller design, which is
what you would expect of a block whose cost does not depend on what is
in front of it.

### It does not fit the iCE40 HX8K

`examples/soc` and `examples/mos6502_computer` both fit on an
`ice40-hx8k-ct256`, and this does not. The logic would fit — 4946
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

### And the encoding build

The ECP5 flow in `tests/nes.rs`, for `ecp5-45f-CABGA381`:

| Resource | `nes_jpeg` | `nes_top` | The 45F has |
|----------|-----------|-----------|-------------|
| `LUT4` | 7863 | 4946 | 43848 |
| `TRELLIS_FF` | 1932 | 1154 | 43848 |
| `CCU2C` | 645 | 313 | — |
| `DP16KD` | 46 | 39 | — |
| logic depth | 25 | 25 | — |

The two columns are not a difference anyone can subtract: `nes_jpeg`
has no transmitter in it, so what the encoder added and what `dvi_tx`
took away are both in the gap. The block RAM is the exception and is
exact — **the encoder's five memories are 7 `DP16KD`**, which is what
`docs/ip-library.md` reports for it on its own:

| Memory | `DP16KD` |
|--------|----------|
| `u_enc.u_ent.tbl` | 3 |
| `u_enc.cfg` | 1 |
| `u_enc.u_fdct.blk` | 1 |
| `u_enc.u_quant.blk` | 1 |
| `u_enc.u_quant.tbl` | 1 |

The logic depth did not move, which says the encoder's longest path is
shorter than the console's.

### It does not fit the Cynthion either

The board this repository is otherwise built against is a Cynthion,
whose part is an `LFE5U-12F`. Lattice's own table — quoted in
`src/fpga/devices/ecp5.dev` — gives it 12k LUTs and **32** sysMEM
blocks. The logic would fit with room. The memory does not: 46 against
32, and the console alone already wants 39, of which the frame buffer
is 24.

That is a statement about the part number and not about silicon.
`src/fpga/device.rs` records that an `LFE5U-12F` and an `LFE5U-25F` are
the same die and that Project Trellis carries **one** database for both
— at commit `015e033` the two directories are the same git tree — so
the fabric that comes out of it is the 25F's whichever part number
asked, and the 25F is specified at 56 blocks. The tools would place
this and raise a `SiteOverrun` *warning* rather than refuse it, and
that warning is a warning precisely because which of the die's sites
the smaller part number covers is not stated anywhere this project can
read. Nothing here has tried it on a part.
`the_jpeg_build_maps_onto_the_ecp5` asserts the 46, so this section
cannot go stale quietly.

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
  the device's PLL.** [`ip/video/dvi_tx_pll`](../../ip/video/dvi_tx_pll)
  is `dvi_tx` with exactly that change — a `clock_mhz` attribute on an
  undriven wire, which is how a design asks Reticle's flow for a PLL —
  and `nes_top` would swap `input wire clk_x5` for `input wire clk_ref`
  and `(* clock_mhz = 126 *) wire clk_x5;`. It is a pin here because a
  design whose clock nothing drives cannot be simulated, and this one is
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

## On a board: the Basys 3

The Basys 3 is the other target, and it exists because the first one
cannot be built for it. Asking the 7-series flow for `nes_top` ends in
four lines like this one:

```text
error[F0304]: port `tmds_d0` asks for double-data-rate registers, but
`xc7a35t-cpg236` declares neither an IO buffer that registers both edges
nor a `ddr_out` register
```

Everything else about the console mapped and fitted. VGA was the only
missing piece, and `rtl/nes_basys3.v` is what closes it: the same
`nes_console` and `nes_video`, behind [`vga_out`](../../ip/video/vga_out)
instead of `dvi_tx`, with three things only a board needs.

- **No PLL.** The Basys 3 has one oscillator, 100 MHz on W5. A two-bit
  divider makes the 25 MHz pixel rate and a /19 divider makes the
  picture unit's dot rate, and the design has no PLL in it at all —
  `dvi_tx` needed one because it runs at five times the pixel rate, and
  VGA does not. The table under *Clocking* above has the four rates.
- **A power-on reset.** A counter holds `rst_n` low for the first 32768
  clocks, 330 microseconds. An FPGA starts every flip-flop at zero when
  it is configured, so the board needs no reset pin — which is just as
  well, because the Basys 3 wires none to a dedicated pin.
- **Pins.** [`board/basys3.rcf`](board/basys3.rcf) names them,
  transcribed from the comment block in `examples/soc/board/basys3.rcf`,
  which is Digilent's published `Basys-3-Master.xdc`. This design uses
  fifteen of them: the oscillator, twelve colour bits and two syncs. The
  console has no serial port, no sound and no controller, so none of the
  board's other connectors is touched.

The 7-series flow writes three files, and
`the_console_maps_onto_the_artix7_for_the_basys3` puts them in
`target/tmp/nes-basys3/`:

```text
nes_basys3.v      the netlist, as structural Verilog of 7-series primitives
nes_basys3.xdc    the pins and the 100 MHz clock
nes_basys3.tcl    a Vivado batch script that reads both and writes a bitstream
```

```sh
cd target/tmp/nes-basys3
vivado -mode batch -source nes_basys3.tcl
```

which ends at `nes_basys3.bit`, for `openFPGALoader -b basys3` or
Digilent's own tool.

**None of that has been done either.** No Vivado has read those files
and no bitstream exists. What the test checks is that every pin
`board/basys3.rcf` names is one the device database records, that every
port of `nes_basys3` has a pin, that every cell of the netlist is a
primitive the part has, that there is no PLL and no double-data-rate
register anywhere in it, and that the script names the part and the
steps. Whether Vivado accepts the result, and whether a monitor locks to
what comes out of the DE-15 socket, are separate claims and this
example makes neither.

## What is not modelled

The picture unit's own list is at the top of
[`ip/video/ppu2c02/rtl/ppu2c02.v`](../../ip/video/ppu2c02/rtl/ppu2c02.v);
the short version, and what the console adds to it:

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

- the manifest resolves `mos6502`, `ppu2c02`, `dvi_tx` and `vga_out` from
  `ip/` and builds, through `ip::resolve` and `ip::elaborate` and through
  `reticle build`, with four files of user HDL and with `video_timing`
  pulled in once for both video paths;
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
- **and the whole chain end to end on the other board**: the same frame
  of the demo, through the frame buffer, the doubling and `vga_out`,
  with all 307,200 pixels of the 640 x 480 raster read back off the
  twelve colour pins and compared against the model doubled, centred and
  truncated to four bits a channel — and the two sync pins checked at
  four places on each of the 525 lines;
- what four bits a channel costs the palette, counted rather than warned
  about: 54 distinct colours become 53, and the pair that merges is
  named;
- **and the picture comes back out as a file**: `nes_jpeg` freezes one
  whole frame in the buffer, walks it in block order through `nes_luma`
  into `mjpeg_enc`, and gives back 8370 bytes that are
  `testdata/nes/frame.jpg` byte for byte — a file ffmpeg decodes as
  baseline greyscale JPEG, 256 x 240, at 39.8 dB against the samples
  that went in;
- the ECP5 flow maps every cell to a device primitive, puts every memory
  into block RAM with the cartridge's contents in it, and exports the
  JSON and LPF `nextpnr-ecp5` reads, with no problem left for
  `check_nextpnr_json` to find;
- the 7-series flow does the same for `nes_basys3` on an XC7A35T — every
  cell a primitive, every memory still in block RAM, no PLL and no
  double-data-rate register anywhere — and writes the netlist, the XDC
  and the Vivado script;
- `reticle build`, `reticle sim` and `reticle fpga` do the same from the
  command line, and the testbench's own hash of the frame agrees with the
  model's.

Not proved:

- **that it runs on a board.** No board, no prjtrellis tools and no
  Vivado here, and where the ECP5 and 7-series primitive descriptions
  come from is stated, with its confidence, in
  `src/fpga/devices/ecp5.dev` and `src/fpga/devices/xc7.dev`, not checked
  against a part. Nothing here has been on a screen.
- **that a monitor locks to the VGA signal.** The timings and the sync
  polarities are VESA's and the pinout is Digilent's published board
  file, both read off pins in simulation — but the only thing that
  settles a video output is a monitor, and none has been connected.
- **timing.** Nothing checks that the design closes at 126 MHz on the
  real part, and 126 MHz through a TMDS serialiser is the most demanding
  thing in this repository. It is entirely possible that it does not. The
  Basys 3 build asks for far less — 100 MHz through logic 23 levels deep,
  and no serialiser at all — but nothing checks that either.
- **that a commercial game would run.** It would not, and that is not an
  accident: there is no mapper, no sound, no controller and no 8 x 16
  sprite, and every cartridge worth naming needs at least three of those.
  This console runs the program in `sw/`.
- **the whole picture through the whole chain in one simulation, on the
  DVI path.** `the_frame_comes_out_of_the_vga_pins` does exactly that for
  the VGA path, because VGA needs no clock at five times the pixel rate;
  through `dvi_tx` the same frame is 2.1 million cycles of a design this
  size, which is minutes of simulation for a link the frame test and the
  TMDS test already cover from either end.
- **that the bytes get anywhere.** `nes_jpeg` ends at a port. There is
  no transport behind it, nothing in `ip/usb/` that a host would open as
  a camera, and the rate arithmetic in the JPEG section above assumes a
  60 MHz clock that nothing has shown this design reaches.
- **anything about sound**, because there is none.

Building this found **two defects in Reticle**, both in the FPGA flow and
both fixed, with a regression test each at the bottom of `tests/nes.rs`:
dead code elimination panicked when a dead expression still named a
memory that had just been removed, and `unique_name` looked up every
candidate with a scan of the whole module, so a memory lowered into
hundreds of cells cost a scan per cell per cell — six minutes for this
picture unit, twenty seconds after.
