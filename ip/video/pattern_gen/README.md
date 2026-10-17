# `pattern_gen`

Eight video test patterns as a pure function of where the raster is:
colour bars, a greyscale staircase, a checkerboard, a crosshatch, RGB
steps, a solid colour, a sweeping bar and full white.

**Nothing in this package has been run on a board, through Vivado, or
through any vendor tool, and no monitor has shown one of these
patterns.** What is proved is what `tests/ip_library.rs` checks in
simulation and what the FPGA flow reports; see *What is and is not
proved* at the end.

## Why it exists

`ip/video/dvi_tx` and `ip/video/vga_out` both hand out `x`, `y`, `de`
and `frame` and expect a colour back on `r`, `g`, `b`. That socket had
no occupant. Every design in this tree that drove either block brought
its own picture — `examples/apple2` a text screen out of RAM,
`examples/nes` a frame buffer out of a PPU — so the first step in
bringing up a new board's video was writing a pixel source before
anything at all could be seen, and the first thing anyone writes is
some colour bars.

It is deliberately the simplest thing that fills the socket: no memory,
no line buffer, and no state but a twelve-bit counter for the one
pattern that moves.

## The patterns

`pattern` selects at run time, so a board can put it on switches or
step it with a button and walk through all eight without rebuilding.

| `pattern` | | what it draws |
|---|---|---|
| 0 | `BARS` | eight full-scale colour bars: white, yellow, cyan, green, magenta, red, blue, black |
| 1 | `STAIRS` | a sixteen-step greyscale staircase, black at the left and full scale at the right |
| 2 | `CHECKER` | a checkerboard of `SQUARE`-pixel squares |
| 3 | `GRID` | a one-pixel crosshatch every `GRID_PITCH`, with a border on the outermost active pixels |
| 4 | `RGBSTEP` | red stepping across, green stepping down, blue off |
| 5 | `SOLID` | one colour, `SOLID_RGB` |
| 6 | `SWEEP` | a sixteen-pixel white bar, one pixel further right each frame |
| 7 | `WHITE` | full scale everywhere |

Tying `pattern` to a constant costs nothing: each pattern is
combinational logic behind one multiplexer, so constant propagation
removes the other seven. That is why there is no parameter for which
patterns to build.

### What each one is for

`BARS` says whether the channels are the right way round and in the
right order — a swapped pair shows up instantly, because the eye knows
these eight colours and their order. `STAIRS` says whether the colour
bits reach the pins in the right significance, and at `BPC = 4` its
sixteen steps are exactly the sixteen levels a four-bit ladder can
make. `CHECKER` at a small square is the hardest thing for a marginal
link to carry and the easiest place to see a dropped or doubled pixel.
`GRID` says whether the geometry is right and whether the display is
overscanning: if the border is not visible on all four sides, the
picture is being cropped. `RGBSTEP` is two staircases at once, which
catches an `x`/`y` swap. `SOLID` and `WHITE` are flat fields, for
checking uniformity and for making the output draw its worst-case
current. `SWEEP` is the only one that moves, and it is there to answer
"is the frame signal arriving at all".

## Why a size and not a `MODE`

`video_timing` takes `MODE` 0, 1 or 2 and turns it into 640x480,
800x600 or 1280x720. This block takes `H_ACTIVE` and `V_ACTIVE`
instead.

`ip/video/vga_out`'s README argues that the mode table should be one
statement in one file, which is why that package depends on `dvi_tx`
for `video_timing` rather than copying it. A third copy of the same
three numbers here would be a third thing to keep in step, and Verilog
has no way to read another module's `localparam` without a header file.
What this block actually needs is narrower than a mode: where the right
edge is and where the bottom is. Taking those directly says so, and it
works for a raster `video_timing` has not got — a panel at 480x272, for
one.

A design wires `H_ACTIVE` and `V_ACTIVE` to match the `MODE` it gave
the output block. Getting them out of step is visible immediately in
every pattern, which is the saving grace of a mistake that cannot be
caught at elaboration.

## `frame` is a level, not a pulse

`video_timing` asserts `frame` combinationally while its `h` and `v`
counters are both zero:

```verilog
assign frame = (h == 12'd0) & (v == 12'd0);
```

So it is high for one whole *pixel period*, which is one clock when a
design runs `vga_out` at the pixel rate and **five** when `dvi_tx` runs
the raster at five times it. A counter clocked on `frame` alone would
therefore advance five times a frame on the DVI path and once a frame
on the VGA path, and the sweeping bar would move at five different
speeds depending on which output block it was wired to.

`pix_en && frame` is the condition, because a pixel period contains
exactly one enabled edge whatever the ratio.
`pattern_gen_sweeps_one_pixel_a_frame_at_either_pixel_rate` runs both.

## The colour is combinational

`r`, `g` and `b` are driven from an `always @*` off the current `x`,
`y` and `de`, which is this socket's contract:
`examples/apple2`'s video does the same, and `vga_out` registers its
own pins. A generator that registered its output would shift the
picture one pixel against a raster that did not expect it.

Outside the active area every channel is zero. `vga_out` blanks again
at its pins and that is not redundant: its blanking is about what a
monitor needs from the back porch, and this one is about this block
having a defined value at every position.

## Powers of two

`SQUARE` and `GRID_PITCH` must be powers of two. The checkerboard's
parity is a single bit of `x` exclusive-ored with the same bit of `y`,
and the crosshatch's line test is "the low bits of `x` are zero", which
is correct only for a power of two. The alternative is a divider and a
remainder per pixel for a block whose whole point is that it is cheap;
the constraint is stated here, in the manifest and in the header, and
is not checked at elaboration.

`H_ACTIVE` and `V_ACTIVE` need no such constraint, but a width that is
not a multiple of sixteen makes the last bar and the last staircase
step wider by the remainder, because the boundaries are at multiples of
`H_ACTIVE / 16` and the division is truncating. 640, 800 and 1280 are
all multiples of sixteen, and so are 480, 600 and 720.

## Using it

```verilog
    wire        de, frame, pix_en;
    wire [11:0] x, y;
    wire [7:0]  r, g, b;

    pattern_gen #(
        .H_ACTIVE (640),
        .V_ACTIVE (480)
    ) u_pat (
        .clk (clk), .rst_n (rst_n), .pix_en (pix_en),
        .de (de), .frame (frame), .x (x), .y (y),
        .pattern (sw[2:0]),
        .r (r), .g (g), .b (b));

    vga_out #(.MODE (0), .BPC (4)) u_vga (
        .clk (clk), .rst_n (rst_n), .pix_en (pix_en),
        .de (de), .frame (frame), .x (x), .y (y),
        .r (r), .g (g), .b (b),
        .vga_r (vga_r), .vga_g (vga_g), .vga_b (vga_b),
        .vga_hsync (vga_hsync), .vga_vsync (vga_vsync));
```

`dvi_tx` is the same wiring with `clk_x5` in place of `clk`, and it
produces `pix_en` itself rather than taking it.

## What is and is not proved

Proved, by `tests/ip_library.rs`:

* every pixel of every pattern, at a 64 x 32 raster, against a model
  written from this README and the module header and from nothing in
  the RTL — the whole active area and four columns and two lines of
  blanking beyond it;
* the sweeping bar advancing one pixel a frame and wrapping at the
  right edge, over two full laps, at one pixel per clock and at one
  pixel in five;
* the block synthesises with no diagnostic and no inferred latch, and
  maps for the iCE40 and the ECP5 with the footprint
  `docs/ip-library.md` records.

The exhaustive test is at 64 x 32 and not at 640 x 480 on purpose: 64
and 32 divide by sixteen exactly as 640 and 480 do, so the boundaries
fall the same way, and a small raster is the difference between
comparing a sample of the pixels and comparing all of them.

Not proved, and not claimed: anything at a size whose sixteenth is not
a whole number, where the header says the last step is wider and
nothing measures it; anything about `SQUARE` or `GRID_PITCH` not being
a power of two, which is a stated constraint and not a checked one; and
that any of this looks right on a screen. No bitstream containing this
block has been built or loaded, and no monitor has shown one of these
patterns.
