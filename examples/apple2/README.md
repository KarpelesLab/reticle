# An Apple II-compatible computer

A machine with a 6502, 48 KiB of RAM, the Apple II memory map, the Apple
II text screen with its famously scrambled line order, and DVI video. It
boots into a monitor, prints a banner, beeps, and echoes what you type at
it. The processor is [`mos6502`](../../ip/mos6502), the video output is
[`dvi_tx`](../../ip/dvi_tx) and the keyboard is a terminal on
[`uart`](../../ip/uart), all from Reticle's IP library. Three files in
`rtl/` are the only HDL written for the project.

**No Apple software is here and none is needed.** Apple's Monitor ROM,
Integer BASIC, Applesoft BASIC and the Apple II character generator are
Apple's, and still are; not one byte of any of them is in this directory,
in any form. What is here instead was written for this example: a monitor
ROM in 6502 assembly (`sw/monitor.s`) and a character generator drawn dot
by dot (`sw/font.txt`). The *hardware* is a different matter — a memory
map, a set of soft switches and a video addressing scheme are published
facts about a machine rather than anybody's creative expression, and
those are implemented from the documentation, which is why this is an
Apple II-compatible **machine** running **our own** software.

This is [`examples/mos6502_computer`](../mos6502_computer) grown up: same
manifest format, same processor, same test list, but the output is a
screen instead of a serial line, and so
[`tests/apple2.rs`](../../tests/apple2.rs) has to read a screen.
**What that proves, and what it does not, is at the end of this page.**

## What is here

```text
examples/apple2/
  reticle.proj            the project: mos6502, uart and dvi_tx by path, the ECP5 45F
  rtl/apple2_top.v        the board: a PLL, dvi_tx, and the machine
  rtl/apple2.v            the machine: the 6502, 48 KiB, the ROM, the soft switches
  rtl/apple2_video.v      the video scanner: the interleaved text page and the font
  sw/monitor.s            the monitor ROM, in 6502 assembly
  sw/monitor.hex          it assembled, which the ROM loads with $readmemh
  sw/font.txt             the character generator, drawn as art
  sw/font.hex             it converted, which the character generator loads
  tb/apple2_tb.v          a testbench that types at the machine and records its video
  board/ulx3s.rcf         pins and clock for a CABGA381 ECP5
```

## The memory map

It is the Apple II's, because building that machine rather than a machine
like it is the point.

| Address | What |
|---------|------|
| `$0000`–`$BFFF` | RAM, 48 KiB. Zero page, the stack in page one, the input buffer at `$0200`, the text page at `$0400` |
| `$C000` | `KBD` — bit 7 is "a key is waiting", bits 6..0 are the key |
| `$C010` | `KBDSTRB` — touching it says the key has been taken |
| `$C030` | `SPKR` — touching it moves the speaker one way |
| `$C0A0` / `$C0A1` | a serial card in slot 2: data, and "the transmitter will take a byte" |
| `$C100`–`$F7FF` | nothing; reads give zero |
| `$F800`–`$FFFF` | the monitor ROM, 2 KiB, with the three vectors in its last six bytes |

Two rows of that are not choices, they are the part. Zero page and the
stack have to be RAM, because `$0000`–`$00FF` is the 6502's fastest
addressing mode and `$0100`–`$01FF` is the stack with eight bits of
pointer and a hard-wired `$01` in front. And the vectors have to be ROM,
because the core reads `$FFFC` when reset is released and jumps there,
before anything could have written anywhere.

Touching a soft switch *is* the operation: there is nothing stored at
those addresses, which is why 6502 code reaches them with `bit` and `sta`
interchangeably and why a read does the same as a write. The test counts
the speaker moving exactly eight times, which is the boot beep, and eight
`bit $C030`s and no writes at all is what moved it.

## The screen, which is the whole point

40 columns of 24 rows in a 1 KiB page at `$0400`, and the base address of
line N is **not** `$0400 + 40*N`. The Apple II's video counter is a shift
register, and what falls out of it is

```text
    base(N) = $0400 + 128 * (N mod 8) + 40 * (N div 8)
```

so the screen is three interleaved groups of eight lines: `$400`, `$480`,
`$500` … `$780` are lines 0 to 7, then `$428`, `$4A8` … are lines 8 to
15, and `$450`, `$4D0` … are lines 16 to 23. The 8 × 8 = 64 bytes the
arithmetic never reaches are the "screen holes", which on a real machine
belong to the peripheral slots.

The shape of that formula is what made it cheap: `128 * (N mod 8)` is the
low three bits of the row moved up to bits 9..7, and `40 * (N div 8) +
column` is a seven-bit number, because the column is under 40 and the
multiplier is 0, 40 or 80. So the hardware address is a concatenation and
one small adder — no multiply, no table:

```verilog
assign vaddr = {4'b0000, 2'b01, row[2:0], seg};   // seg = group + fcol
```

and the software's address is a 48-byte table, because a 6502 has no
multiplier either. `tests/apple2.rs` computes the same 24 numbers from
the formula above — in Rust, from the documentation, not from either of
those — and checks the table against them; then it reads the screen back
off the video signal and checks that row N shows what the monitor wrote
to row N. The hardware does it by concatenation, the software by table
lookup, the test by arithmetic, and all three have to agree.

A byte of the text page is a **display code**, not ASCII. The low six
bits pick one of 64 glyphs and the top two are the attribute:

| Top bits | Codes | Attribute |
|---|---|---|
| `1x` | `$80`–`$FF` | normal |
| `00` | `$00`–`$3F` | inverse |
| `01` | `$40`–`$7F` | flashing |

The machine shows all three on its second line, on purpose, so that a
look at the screen — or a test that decodes it — sees them at once. The
flashing rate is a frame counter's bit: eight frames one way and eight
the other, about 3.7 Hz.

## The picture

`dvi_tx` in MODE 0 is 640 × 480 at 60 Hz, from a 25.175 MHz pixel clock
that `apple2_top` asks the PLL for at five times over, 126 MHz. The Apple
II's 280 × 192 doubled is 560 × 384, which sits inside 640 × 480 with a
40-pixel border either side and 48 lines above and below — so a character
cell is 14 pixels by 16, every dot is a square of four, and everything
outside the picture is black. The test checks all three of those.

Two clocked lookups stand between a byte of RAM and a dot on the screen —
the text page and then the character generator — and a dot is one pixel,
so the fetch runs a whole character cell ahead of the dots being drawn.
`rtl/apple2_video.v` sets out the four-step pipeline; the lead-in cell at
x = 26..39 is the one that fills the shifter for column 0.

The processor gets one bus cycle every 14 pixels. That is not an
arbitrary divisor: it is the Apple II's own ratio, one video byte fetched
per processor cycle, and at this raster's pixel clock it puts the 6502 at
1.798 MHz against an original's 1.023. There is no bus contention to
arbitrate, because the RAM has a port for the processor and a port for
the video; a real Apple II interleaved them on one bus, which is where
its φ0 / φ1 split came from, and a block RAM with two ports is the same
machine with the arbitration taken out.

## The keyboard is the serial port

`$C000` is fed by `uart`'s receiver rather than by a key matrix, so a
terminal on the host is the machine's keyboard: type into `picocom` and
the monitor sees keystrokes. The other way to do it is a PS/2 decoder,
and here is why this is not that.

1. **It is one wire and one library block.** A PS/2 keyboard is a second
   clock domain (the keyboard drives the clock), a synchroniser, an
   11-bit frame with odd parity, a set/reset protocol for the two break
   codes, and a scancode table — a whole block's worth of work, and one
   this repository does not already have, for an input this example uses
   to type eight characters.
2. **A testbench can drive it.** The keyboard being a serial line is what
   lets `tb/apple2_tb.v` type at the machine and read the echo back, and
   the echo is what the typing is handshaked against. A PS/2 keyboard
   would have had to be modelled in the testbench, and then the test
   would be checking Reticle against a model of a keyboard rather than
   against the machine.
3. **It doubles as the transcript.** Everything the monitor prints goes
   to the screen *and* to `$C0A0`, so the terminal shows the session as
   well as typing it. That is what
   [`tests/serial/mod.rs`](../../tests/serial/mod.rs) — the same decoder
   `examples/soc` and `examples/mos6502_computer` use — reads off
   `uart_tx`, and the screen and the wire have to say the same thing.

The transmitter is separate from the keyboard on purpose: `$C0A0` is
write-only and the receiver goes only to the keyboard latch, so nothing
has to arbitrate between "a key" and "a byte".

## The monitor

`sw/monitor.s` is just over a kilobyte: a cursor, a screen driver with
scrolling, a line editor and four commands.

```text
]E F800                          print eight bytes from an address
]D 0300 11 22 33 44 55 66 77 88  write bytes to an address
]T                               paint the character test card
```

Anything else gets a `?` and a beep. `T` fills the rest of the screen
with every glyph the character generator has, shifted by seven each row
so that no two rows are alike and no two columns of a row are — which is
deliberate, and is what lets the test tell the interleaved line order
apart from any other order. Lower case typed at it is folded up, because
an Apple II keyboard had no lower case and the font has none either.

To change the monitor, edit `sw/monitor.s` and regenerate the hex:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test apple2 monitor_hex
```

The assembler is [`tests/mos6502_asm/mod.rs`](../../tests/mos6502_asm/mod.rs),
the same opcode matrix that runs `mos6502`'s own tests, built from the
**documented encoding table** and never from the core's decoder — which
is what stops a wrong core and a wrong test agreeing with each other.
This example added `<` and `>` to it, for the low and high byte of an
address, because a program cannot put the address of a label into a
zero-page pointer without them.

## The font

`sw/font.txt` is the character generator, and it is drawn:

```text
glyph 01 A
..#....
.#.#...
#...#..
#...#..
#####..
#...#..
#...#..
.......
```

64 glyphs, ASCII `$20`–`$5F`, each a 5 × 7 shape in the top left of a
7 × 8 cell — the two right-hand columns and the bottom row are the gap
that makes the screen read as 40 columns rather than as a wall.
`tests/apple2.rs` turns that art into `sw/font.hex`, fails if the
checked-in hex is not what the art says, and then matches the cells it
decodes out of the video signal against **the art**, not against the hex
and not against the RTL. Regenerate it the same way:

```sh
UPDATE_EXPECT=1 cargo test --all-features --test apple2 font_hex
```

No Apple character generator ROM was consulted, copied or reconstructed
for this; the shapes were drawn for this example.

## Building

From this directory:

```sh
reticle build --synth reticle.proj
```

resolves the three packages, writes `reticle.lock`, elaborates the design
and synthesises it. Synthesis turns both `$readmemh`s into the memories'
initial contents, reading the files through a provider the binary gives
it.

## Testing

From the repository root:

```sh
cargo test --all-features --test apple2
```

| Test | What it shows |
|------|---------------|
| `font_hex_is_the_drawn_font` | `sw/font.hex` is `sw/font.txt`'s art, dot for dot |
| `the_font_tells_its_glyphs_apart` | no two of the 128 patterns a cell can show — 64 glyphs, normal and inverse — are alike, so reading a cell back off the screen has one answer |
| `monitor_hex_is_the_assembled_source` | `sw/monitor.hex` is `sw/monitor.s` assembled, inside the ROM, with the three vectors pointing at the labels the source names |
| `the_assembler_takes_the_low_and_high_byte_of_an_address` | the `<` and `>` the assembler gained for this |
| `the_monitors_line_table_is_the_documented_interleave` | the ROM's 48-byte line table is `$0400 + 128 * (N mod 8) + 40 * (N div 8)`, computed in the test from the formula |
| `the_project_resolves_and_elaborates` | the manifest builds from exactly three library packages and the three files in `rtl/` |
| `the_machine_synthesises_without_errors_or_latches` | synthesis has no error, no warning and no latch, and the ROM and the character generator hold what their files say, with everything the files did not name still `x` |
| **`the_screen_comes_out_of_the_video_signal`** | **the whole chain** — see below |
| `the_machine_maps_onto_the_ecp5_and_exports_for_nextpnr` | the ECP5 flow fits it, puts the monitor and the font in block RAM, builds the PLL and the four DDR lanes, and writes the JSON and LPF `nextpnr-ecp5` reads |
| `the_machine_does_not_fit_the_ice40_hx8k` | it does not fit the part the other two examples use, and by how much |
| `reticle_build_builds_the_project` | `reticle build --synth` on the manifest |
| `reticle_sim_runs_the_testbench` | the banner comes out of `reticle sim` too |
| `reticle_fpga_exports_the_rom_with_the_monitor_in_it` | and `reticle fpga` exports memories that are not blank |

`examples/soc` and `examples/mos6502_computer` both end at a serial wire:
a string goes in one end of the design and comes out of a pin, and the
test decodes the pin's waveform. This machine ends at a *screen*, and
`the_screen_comes_out_of_the_video_signal` is the same test for it:

* it samples `de` and the 24-bit colour the video generator hands
  `dvi_tx`, and nothing else — no register inside the design, no peek at
  the text page, no look at the character generator;
* it ties simulation time to pixel number from the first falling edge of
  `de`, and then **checks every other one**: a line is 800 pixels and a
  frame is 480 visible lines and 45 blank ones, which is the VESA timing
  typed into the test rather than read out of `video_timing`;
* it rebuilds the 640 × 480 picture, checks the border is black, checks
  every dot is a square of four pixels, cuts the 560 × 384 middle into
  7 × 8 cells and matches each against `sw/font.txt`;
* and asserts the forty columns of twenty-four rows that come out —
  glyph *and* polarity, so inverse is checked as well as shape.

Then it does it again, one frame later, for the row with the flashing
words on it: the eight cells the monitor wrote with the flashing
attribute have swapped over and nothing else has, which is the whole of
what flashing is. The testbench turns the flashing rate up for this, from
sixteen frames a cycle to two, because the alternative was simulating
sixteen frames of video.

The same run also decodes the session off `uart_tx` with the shared 8N1
receiver and counts the speaker moving, so one simulation proves the
screen, the serial port, the keyboard (the testbench typed the session)
and the soft switches.

It takes about ninety seconds, which is most of this file's run time and
a fair slice of the repository's. A frame of 640 × 480 is 420,000 pixel
slots, the testbench draws two, and Reticle's event simulator runs
unoptimised under `cargo test`. Three things keep it to ninety rather
than four hundred seconds, and each is a deliberate trade:

* **the testbench drives `apple2` from a bare `video_timing` at one pixel
  per clock**, where `apple2_top` drives it from `dvi_tx` at one pixel in
  five. It is the same raster block with the same MODE, and everything in
  the machine that moves with the picture moves on `pix_en`, so the
  frames are identical. What is left out is the TMDS encoders and the
  10:1 serialisers, which are `dvi_tx`'s own and are checked against the
  DVI specification in `tests/ip_library.rs`.
* **the raster is held in reset until the monitor has finished drawing**,
  so the frame recorded is frame 0 and the run spends nothing looking for
  a boundary.
* **the second frame stops after the row it needs.**

## Resource use

The ECP5 flow in `tests/apple2.rs`, for `ecp5-45f-CABGA381`:

| Resource | Used | The 45F has |
|----------|------|-------------|
| `LUT4` | 2755 | 43848 |
| `TRELLIS_FF` | 403 | 43848 |
| `DP16KD` | 50 | 108 |
| `EHXPLLL` | 1 | 4 |
| `ODDRX1F` | 4 | — |
| `TRELLIS_IO` | 8 | — |
| `DCCA` | 1 | 16 |

LUT depth 23, the same as `examples/mos6502_computer`: the deepest path
is still inside the processor. The fifty block RAMs are the interesting
number — 48 of them are the main memory, which is 48 KiB in 2048 × 9 mode
and **two copies**, one per read port, because the processor and the
video scanner read it at the same time. The monitor is one more and the
character generator the last.

### And on the iCE40 HX8K?

**It does not fit**, and the test says so by running the flow rather than
by arguing. For `ice40-hx8k-ct256`, with the board's 12 MHz oscillator
named so the PLL is in the count:

| Resource | Used | The HX8K has |
|----------|------|--------------|
| `SB_LUT4` | 3970 | 7680 |
| flip-flops (`SB_DFF*`) | 409 | 7680 |
| `SB_CARRY` | 409 | — |
| `SB_RAM40_4K` | **197** | **32** |
| `SB_PLL40_CORE` | 1 | 1 |

The logic would have fitted with room to spare — 3970 LUTs of 7680, about
half. The memory is the problem, and not by a little: an HX8K has 32
block RAMs of 4 kbit, 16 KiB in total, and this machine wants 48 KiB of
it *twice*, plus the monitor and the font. Six times the part. There is
no arrangement of this design that fits an HX8K; a machine with 16 KiB
and a single-ported screen might, and would be a different example.

## On a board

The flow writes `apple2_top.json` (the netlist, in the Yosys JSON format)
and `apple2_top.lpf` (the pins) to `target/tmp/apple2/`. With
[nextpnr](https://github.com/YosysHQ/nextpnr) and
[Project Trellis](https://github.com/YosysHQ/prjtrellis), the remaining
steps would be:

```sh
cd target/tmp/apple2
nextpnr-ecp5 --45k --package CABGA381 --json apple2_top.json \
  --lpf apple2_top.lpf --textcfg apple2_top.config
ecppack apple2_top.config apple2_top.bit
openFPGALoader -b ulx3s apple2_top.bit
```

and then a monitor on the DVI connector and a terminal on the board's
serial port at 115200 baud, 8N1.

**None of that has been done**, and `board/ulx3s.rcf` will not do it as
written. `src/fpga/devices/ecp5.dev` says in so many words that its pin
list for this package is partial — it holds a ULX3S board's 25 MHz
oscillator and its eight LEDs and nothing else, because those came from a
board file rather than from Lattice's package drawings. So the constraint
file uses those nine pins, which means the DVI lanes and the serial port
are on pins a real ULX3S wires to LEDs. The flow, the netlist and the
constraint file are all real; the pinout is a placeholder, and it is a
placeholder on purpose rather than a guess at pins nobody here has
checked. Putting this on a board means adding the rest of the package to
`ecp5.dev` from the datasheet and then writing the four GPDI pairs and
the two FTDI pins into `board/ulx3s.rcf`.

The same flow runs from the command line:

```sh
reticle fpga --device ecp5-45f-CABGA381 --top apple2_top \
  --constraints board/ulx3s.rcf \
  rtl/apple2_top.v rtl/apple2.v rtl/apple2_video.v \
  ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/uart/rtl/uart_tx.v ../../ip/uart/rtl/uart_rx.v ../../ip/uart/rtl/uart.v \
  ../../ip/dvi_tx/rtl/tmds_encoder.v ../../ip/dvi_tx/rtl/video_timing.v \
  ../../ip/dvi_tx/rtl/dvi_tx.v
```

and so does the simulation, which prints the session:

```sh
reticle sim tb/apple2_tb.v rtl/apple2.v rtl/apple2_video.v \
  ../../ip/mos6502/rtl/mos6502.v \
  ../../ip/uart/rtl/uart_tx.v ../../ip/uart/rtl/uart_rx.v ../../ip/uart/rtl/uart.v \
  ../../ip/dvi_tx/rtl/tmds_encoder.v ../../ip/dvi_tx/rtl/video_timing.v \
  ../../ip/dvi_tx/rtl/dvi_tx.v
```

```text
    RETICLE APPLE ][ COMPATIBLE 6502
ATTRIBUTES: NORMAL INVERSE FLASHING
TEXT 40X24  $0400 INTERLEAVED
]E F800
F800: 78 D8 A2 FF 9A 2C 10 C0
]D 0300 11 22 33 44 55 66 77 88
]E 0300
0300: 11 22 33 44 55 66 77 88
]T
```

`rtl/apple2_top.v` itself cannot be simulated, for the same reason
`dvi_tx_pll` cannot: nothing in the source drives `clk_x5`, because the
PLL that does is instantiated by the FPGA flow.

## Two defects this found in Reticle

Both are in `src/fpga/constraints.rs`, both are about a design that is
still hierarchical when its board file is checked, and both are fixed
with a regression test next to the code.

1. **A port with IO options but no pin was reported as having its pin
   overridden.** `(* ddr = "clk_x5" *) output wire [1:0] tmds_d0` makes
   `Constraints::from_attrs` produce an assignment to *no* pin, so the
   DDR option reaches the IO buffer; merging a board file that does name
   a pin then printed `overriding pin  from a source attribute`, with a
   hole where the pin name should be, once per lane. An assignment with
   no pin is not a pin assignment, and is no longer treated as one.
   (`options_without_a_pin_do_not_override_a_pin`)
2. **The reference clock of a PLL request was reported as clocking
   nothing.** A design asks for a generated clock by declaring a net,
   using it, leaving it undriven and constraining it — `docs/fpga.md`
   says so — and the check that recognises that was asking `driven_nets`,
   which counts a net an instance *touches* whichever way the port goes.
   In a hierarchical design the net the PLL is asked for is read by every
   block that uses it, so the request went unrecognised and both it and
   its reference clock drew a warning. The check now resolves an
   instance's connections by the direction of the port each reaches; a
   constrained net that is neither written nor read is still a constraint
   on nothing and still warns.
   (`a_pll_reference_clock_is_recognised_through_an_instance`)

## What this proves, and what it does not

Proved here, by `cargo test`:

- the manifest resolves `mos6502`, `uart` and `dvi_tx` from `ip/` and
  builds, through `ip::resolve` and `ip::elaborate` and through
  `reticle build`, with the three files in `rtl/` the only user HDL;
- synthesis accepts it with no error, warning or latch, and loads the
  monitor and the character generator into their memories, leaving every
  element neither file named `x`;
- **the screen**: forty columns of twenty-four rows, decoded out of a
  whole frame of video by a test that never looks inside the design —
  the interleaved line order, the inverse and flashing attributes, the
  pixel doubling, the centring, and the character generator as drawn;
- the interleave from the other side too: the monitor's line table is the
  documented formula, computed in the test from the formula;
- the flashing attribute actually alternating, across two frames;
- the session decoded off `uart_tx` as well, by the decoder `examples/soc`
  and `examples/mos6502_computer` share — which also proves the keyboard,
  because the testbench typed that session in and handshaked on the echo;
- the speaker moving exactly as many times as the monitor touched
  `$C030`, which is the soft switches answering a read;
- the ECP5 flow mapping every cell to a device primitive, fitting in
  6 % of the LUTs and under half the block RAM, building the PLL and the
  four double-data-rate lanes, and exporting the JSON and LPF
  `nextpnr-ecp5` reads with nothing left for `check_nextpnr_json` to
  find;
- that it does not fit an HX8K, by running the flow for one;
- `reticle build`, `reticle sim` and `reticle fpga` doing the same from
  the command line.

Not proved, and in one case not built:

- **that it runs on a board.** No board here, no Trellis tools, and
  `board/ulx3s.rcf` is on placeholder pins for the reason given above.
  Reticle's own place and route uses a synthetic fabric that cannot
  program a real part ([`docs/fpga.md`](../../docs/fpga.md)). What is
  demonstrated stops at the files `nextpnr-ecp5` reads.
- **timing.** Nothing here checks that 126 MHz closes on the real part,
  and 126 MHz through a 6502's address decode is the part of this design
  most likely not to.
- **the TMDS chain.** The frame test stops at the colour `dvi_tx` is
  handed; the encoders and serialisers after it are library IP with their
  own tests, and no test in this file drives both at once.
- **graphics.** There is no lo-res and no hi-res: `$C050`–`$C057` are not
  decoded, there is no MIXED and no PAGE2, and the screen is always the
  text page at `$0400`. That is a scope line drawn on purpose. The way
  this example proves what is on the screen is by decoding a whole frame
  of video, one frame is a good part of a minute of simulation, and a
  mode nothing decoded back off the wire would be a mode nobody had
  checked — which would be worse than not having it. A second video mode
  is a day's work and another minute of test time, and it should be
  spent when somebody wants to run a program that needs one.
- **scrolling.** The monitor scrolls, but the session the testbench types
  never reaches the bottom of the screen, so no test sees it happen.
- **interrupts.** `irq` and `nmi` are tied low and the monitor sets I.
  The vectors point at a real `RTI` and the map is complete, but no
  interrupt is taken here. A real Apple II is the same in this respect —
  nothing on its motherboard raises one — and `mos6502`'s own tests in
  `tests/ip_library.rs` take them.
- **the flashing rate.** The testbench runs it at two frames a cycle, not
  the sixteen the machine is built with, for the reason above. That the
  counter counts frames is plain in `rtl/apple2_video.v`; that it counts
  *sixteen* of them is not checked.
