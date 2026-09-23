# `vga_out`

VGA video output: the raster of one of three standard modes, colour
driven straight at a board's resistor ladder, and the two sync pins at
the polarity the mode wants.

**Nothing in this package has been run on a board, through Vivado, or
through any vendor tool.** What is proved is what `tests/ip_library.rs`
checks in simulation and what the FPGA flow reports; see *What is and is
not proved* at the end.

## Why it exists

`ip/dvi_tx` drives TMDS differential pairs through double-data-rate
output registers. A board with no HDMI or DVI connector cannot use it,
and a device file that declares no DDR register cannot build it. A
measured run of `examples/apple2` onto `xc7a35t-cpg236` — the Digilent
Basys 3's part — said so in one line:

```
port tmds_d0 is not double data rate: `xc7a35t-cpg236` declares neither an
IO buffer that registers both edges nor a `ddr_out` register
```

Everything else about that design mapped and fitted. VGA is what the
Basys 3 has, and VGA is strictly simpler than DVI: it is the video
timing, plus the colour bits, plus two sync pins.

## Reusing `video_timing` instead of copying it

The manifest carries

```text
depends dvi_tx ^1.0.0
```

which is the same line `dvi_tx_pll` uses, and it is the clean choice
here for two reasons. The manifest format's other option would be to
list `../dvi_tx/rtl/video_timing.v` as a `source` of this package, which
makes the file a member of two packages: two versions, two licences and
two places a change has to be reviewed, for one module. A `depends` line
instead names the package that owns the module, `PathProvider` finds it
in the directory next to this one, and a project that already depends on
`dvi_tx` — one that builds both a DVI and a VGA target from the same
sources, like `examples/apple2` — gets exactly one copy of
`video_timing` in its design.

The consequence worth knowing: depending on `vga_out` pulls in the whole
of `dvi_tx`, encoders and serialisers included, as *sources*. They
elaborate; nothing instantiates them, so nothing reaches a netlist.
`the_machine_maps_onto_the_artix7_for_the_basys3` in `tests/apple2.rs`
is the evidence — the Basys 3 netlist has no double-data-rate register
and no PLL in it, which is what a serialiser would have brought, and it
maps for a part whose device file declares neither.

## Ports

| Port | Direction | Width | What it is |
|------|-----------|-------|------------|
| `clk` | in | 1 | the clock everything runs on |
| `rst_n` | in | 1 | active-low, asynchronous |
| `pix_en` | in | 1 | one pixel per clock edge where this is high; tie high on a clock that *is* the pixel clock |
| `de` | out | 1 | the pixel being fetched is visible |
| `frame` | out | 1 | pulses on the top left pixel |
| `x`, `y` | out | 12 | which pixel is being fetched |
| `r`, `g`, `b` | in | 8 | the colour for it, eight bits a channel |
| `vga_r`, `vga_g`, `vga_b` | out | `BPC` | the ladder |
| `vga_hsync`, `vga_vsync` | out | 1 | the two sync pins |

`de`, `frame`, `x`, `y`, `r`, `g` and `b` are `dvi_tx`'s ports with
`dvi_tx`'s meaning, so a design swaps one block for the other without
rewiring its pattern generator. The difference is the clock: `dvi_tx`
takes `clk_x5`, five times the pixel rate, and *outputs* the `pix_en` it
makes from it; `vga_out` needs no faster clock and takes `pix_en` as an
input, because on a VGA board the pixel rate is whatever the board's
oscillator and divider make it.

## Parameters

| Parameter | Default | Range | What it does |
|-----------|---------|-------|--------------|
| `MODE` | 0 | 0..2 | 0 = 640x480@60, 1 = 800x600@60, 2 = 1280x720@60 — `video_timing`'s own numbering |
| `BPC` | 4 | 1..8 | bits per colour channel the board's ladder has |

`BPC` defaults to 4 because that is the Basys 3, and because four bits a
channel through an R-2R ladder is the commonest VGA connector on a
development board. Other boards differ, which is why it is a parameter:
six bits a channel is the other common width, and a board with a real
video DAC wants eight.

## Truncation, not rounding

The eight bits in become the **top `BPC` bits** out: `r[7 -: BPC]`.
They are truncated, not rounded, on purpose.

* It costs nothing — no adder, no carry, no extra logic level.
* It is monotonic, and both ends of the range stay exact: `8'h00` is
  every output bit low, which is black, and `8'hFF` is every output bit
  high, which is full scale on the ladder. Rounding to nearest would
  have to saturate to keep `8'hFF` from carrying out of a `BPC`-bit
  field, which is an adder and a comparator per channel.
* What it buys is half a step of accuracy in the middle of the range.
  A four-bit R-2R ladder built from 1 % resistors into a 75 ohm load has
  more error than that in its own components.

What this means for a picture: at `BPC = 4` the block renders 4096
colours where the DVI path renders 16.7 million, and every channel value
is quantised **down** to a multiple of 17/255 of full scale. A design
that draws only black and white — `examples/apple2` is one — comes out
identical on both paths. A design with gradients or blended colour does
not: it will band, and each band sits slightly darker than the DVI path
puts it.

## Blanking

Outside the active area every colour bit is driven low. This is a
correctness requirement rather than tidiness: a monitor takes its black
level from the back porch, so colour driven during the blanking interval
either makes it lose sync or drags the black level until the picture
rolls. `de` from the raster is the gate, and
`vga_out_forces_black_through_both_blanking_intervals` walks a whole
frame with white driven into every pixel and asserts the pins are zero
everywhere `de` is low — in the horizontal blanking of every line and
through every line of the vertical blanking.

## Sync polarity

From `video_timing`, which has it per mode: mode 0's syncs pulse **low**
and modes 1 and 2's pulse **high**. `vga_out` passes them through and
does not restate the rule — with one exception. A reset value has to be
a constant, so the module carries a `SYNC_IDLE` localparam for the level
the pins hold in reset.
`vga_out_syncs_idle_at_the_polarity_of_the_mode` asserts that this level
is the same one the block drives during the back porch, for all three
modes, so the constant cannot drift away from `video_timing`'s rule
without a test failing.

## One pixel of latency

The five output pins are registered on `pix_en`. The colour, hsync and
vsync a pin carries during pixel N+1 are the ones the raster asked for
at pixel N. All five move together, so the picture is shifted one pixel
to the right and nothing is skewed against anything else — which is why
a register is worth having: it is one flip-flop per pin and it keeps a
combinational pattern generator's glitches off a wire going to an
analogue input.

`de`, `frame`, `x` and `y` are the raster's own and are **not** delayed,
because they are the fetch interface and not the signal. A test that
decodes a picture off the pins has to account for the pixel; both of the
ones in this repository do, and say so.

## Using it

```verilog
wire        pix_en;
wire        de;
wire [11:0] x, y;
wire [7:0]  r, g, b;

// 100 MHz in, 25 MHz pixels.
reg [1:0] div = 2'd0;
always @(posedge clk) div <= div + 2'd1;
assign pix_en = (div == 2'd3);

vga_out #(
    .MODE (0),      // 640 x 480 at 60 Hz
    .BPC  (4)       // the Basys 3's ladder
) u_vga (
    .clk       (clk),
    .rst_n     (rst_n),
    .pix_en    (pix_en),
    .de        (de),
    .frame     (),
    .x         (x),
    .y         (y),
    .r         (r),
    .g         (g),
    .b         (b),
    .vga_r     (vga_r),
    .vga_g     (vga_g),
    .vga_b     (vga_b),
    .vga_hsync (vga_hsync),
    .vga_vsync (vga_vsync)
);
```

`examples/apple2/rtl/apple2_basys3.v` is that, with an Apple II behind
it, and `examples/apple2/board/basys3.rcf` is the pinout.

## What is and is not proved

Proved, by `tests/ip_library.rs`:

* the raster at the pins is the mode's raster, against VESA's numbers
  typed into the test rather than read out of the RTL;
* black through both blanking intervals, over a whole frame;
* the sync polarity of each of the three modes, at the pins;
* a known pattern in gives the expected pixels out, decoded from the
  pins, at `BPC` of 4, 8 and 1;
* the block synthesises with no diagnostic and no inferred latch, and
  maps for the iCE40 and the ECP5 with the footprint `docs/ip-library.md`
  records.

Proved by `tests/apple2.rs`: a whole Apple II text screen read back off
the five VGA pins and matched, cell by cell, against the font as drawn —
and the same design mapped onto `xc7a35t-cpg236` with an XDC and a
Vivado script written out.

Not proved, and not claimed: that a monitor locks to this signal. No
bitstream from this package has been built, loaded or looked at on a
screen. The timings are VESA's, the polarities are VESA's, and the
pinout is Digilent's published board file — but the only thing that
settles a video output is a monitor, and none has been connected.
