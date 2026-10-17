// pattern_gen — the pixel source a video output has nothing to say
// without: eight test patterns as a pure function of where the raster is.
//
// What it does
//   `dvi_tx` and `vga_out` both hand out `x`, `y`, `de` and `frame` and
//   expect a colour back on `r`, `g`, `b`. That socket had no occupant
//   in this library: every design that drove either block brought its
//   own picture — `examples/apple2` a text screen, `examples/nes` a
//   frame buffer — so bringing up a new board's video meant writing one
//   before anything could be seen. This is that occupant, and it is
//   deliberately the simplest thing that fills the socket: no memory, no
//   state but a frame counter, and a colour that is a function of the
//   pixel being asked for.
//
//   `pattern` selects at run time, so a board can put it on switches or
//   on a button and walk through all eight without rebuilding. Tying it
//   to a constant instead costs nothing: the seven patterns not selected
//   fold away, because every one of them is combinational logic behind
//   that multiplexer and constant propagation reaches all of it. There
//   is no parameter for "which patterns to build" because there does not
//   need to be.
//
//     0 BARS     eight full-scale colour bars, the standard order:
//                white yellow cyan green magenta red blue black
//     1 STAIRS   a sixteen-step greyscale staircase, left to right
//     2 CHECKER  a checkerboard of SQUARE-pixel squares
//     3 GRID     a one-pixel crosshatch every GRID_PITCH, with a border
//     4 RGBSTEP  red stepping across, green stepping down, blue constant
//     5 SOLID    one colour, SOLID_RGB
//     6 SWEEP    a white vertical bar, one pixel further right each frame
//     7 WHITE    full scale everywhere
//
//   **The colour is combinational, which is this socket's contract.**
//   `examples/apple2`'s video drives `r`, `g` and `b` from an
//   `always @*` off the same `x`, `y` and `de`, and `vga_out` registers
//   its pins itself. A generator that registered its output would shift
//   the picture one pixel against a raster that did not expect it.
//
//   **Black outside the active area.** `de` gates every pattern, so the
//   output is a pure function of `pattern`, `x`, `y` and the frame
//   count inside the picture and zero outside it. `vga_out` blanks
//   again at its pins, which is not redundant: that one is about what a
//   monitor needs, this one is about this block having a defined value
//   everywhere.
//
// Why a size and not a MODE
//   `video_timing` takes `MODE` 0, 1 or 2 and turns it into 640x480,
//   800x600 or 1280x720. This block takes `H_ACTIVE` and `V_ACTIVE`
//   instead, and `ip/video/vga_out`'s README says why that is the right
//   way round: the timings are one statement in one file, and a third
//   copy of the mode table is a third thing to keep in step. Passing the
//   size says what this block actually needs — where the right edge and
//   the bottom are — and it works for a raster `video_timing` does not
//   have. A design wires `H_ACTIVE`/`V_ACTIVE` to match the `MODE` it
//   gave the output block.
//
// What it does not do
//   No memory and no frame buffer, so no picture that is not arithmetic
//   on `x` and `y`.
//
//   No interlace, no 3:2 pulldown, no colour space but RGB, and nothing
//   about gamma: the staircase is a staircase in *code value*, seventeen
//   per step, and a display's response to it is the display's business.
//
//   Not a conformance pattern. These are bring-up patterns — is the
//   geometry right, are the channels the right way round, does the
//   frame tick — and not SMPTE RP 219 or EBU 3325, which specify levels
//   and sub-patterns this does not have. The bars here are full scale
//   (0 and 255), not the 75 % or the 100/0/75/0 of a broadcast bar.
module pattern_gen #(
    // The active area of the raster being fed. `video_timing`'s MODE 0,
    // 1 and 2 are 640x480, 800x600 and 1280x720.
    parameter H_ACTIVE   = 640,
    parameter V_ACTIVE   = 480,
    // Checkerboard square, in pixels. **A power of two**, because the
    // square index is a bit of `x` rather than a division.
    parameter SQUARE     = 16,
    // Crosshatch spacing, in pixels. **A power of two**, for the same
    // reason.
    parameter GRID_PITCH = 32,
    // The colour pattern 5 shows.
    parameter [23:0] SOLID_RGB = 24'h80_80_80
) (
    input  wire        clk,
    input  wire        rst_n,
    // One pixel per edge where this is high, the same signal the output
    // block runs its raster on. Tie it high on a clock that is already
    // the pixel clock.
    input  wire        pix_en,

    // The raster, from `dvi_tx` or `vga_out`.
    input  wire        de,
    input  wire        frame,
    input  wire [11:0] x,
    input  wire [11:0] y,

    // Which pattern, from the list in the header.
    input  wire [2:0]  pattern,

    // The colour of the pixel `x`, `y` names.
    output reg  [7:0]  r,
    output reg  [7:0]  g,
    output reg  [7:0]  b
);
    localparam [2:0] P_BARS    = 3'd0;
    localparam [2:0] P_STAIRS  = 3'd1;
    localparam [2:0] P_CHECKER = 3'd2;
    localparam [2:0] P_GRID    = 3'd3;
    localparam [2:0] P_RGBSTEP = 3'd4;
    localparam [2:0] P_SOLID   = 3'd5;
    localparam [2:0] P_SWEEP   = 3'd6;
    localparam [2:0] P_WHITE   = 3'd7;

    // -----------------------------------------------------------------
    // Where the pixel is, in the units each pattern counts in
    // -----------------------------------------------------------------
    // A bar is an eighth of the width and a step a sixteenth. Both
    // divisions are of *elaboration-time constants*, so what the fabric
    // builds is a comparison against a literal and never a divider. A
    // width that is not a multiple of sixteen makes the last bar and the
    // last step wider by the remainder, which is the only sensible place
    // to put it and is why this is written down rather than rejected.
    localparam [11:0] BAR_W  = H_ACTIVE / 8;
    localparam [11:0] STEP_W = H_ACTIVE / 16;
    localparam [11:0] STEP_H = V_ACTIVE / 16;

    // The index of the bar or step `x` falls in, as a sum of
    // comparisons: a balanced adder tree rather than a priority chain,
    // which is the shape `src/synth/proc/exec.rs` learned to prefer the
    // hard way (see `docs/fpga-trellis.md`).
    function [3:0] step_of;
        input [11:0] pos;
        input [11:0] width;
        begin
            step_of = {3'd0, pos >= (width * 12'd1)}
                    + {3'd0, pos >= (width * 12'd2)}
                    + {3'd0, pos >= (width * 12'd3)}
                    + {3'd0, pos >= (width * 12'd4)}
                    + {3'd0, pos >= (width * 12'd5)}
                    + {3'd0, pos >= (width * 12'd6)}
                    + {3'd0, pos >= (width * 12'd7)}
                    + {3'd0, pos >= (width * 12'd8)}
                    + {3'd0, pos >= (width * 12'd9)}
                    + {3'd0, pos >= (width * 12'd10)}
                    + {3'd0, pos >= (width * 12'd11)}
                    + {3'd0, pos >= (width * 12'd12)}
                    + {3'd0, pos >= (width * 12'd13)}
                    + {3'd0, pos >= (width * 12'd14)}
                    + {3'd0, pos >= (width * 12'd15)};
        end
    endfunction

    // A bar needs seven boundaries, not fifteen, and the sum of seven
    // one-bit terms is three bits wide with nothing to spare and nothing
    // to lose: inside the active area `x` is below `8 * BAR_W`, so the
    // index is 0 to 7, and outside it `de` has already blanked the
    // output.
    wire [2:0] bar = {2'd0, x >= (BAR_W * 12'd1)}
                   + {2'd0, x >= (BAR_W * 12'd2)}
                   + {2'd0, x >= (BAR_W * 12'd3)}
                   + {2'd0, x >= (BAR_W * 12'd4)}
                   + {2'd0, x >= (BAR_W * 12'd5)}
                   + {2'd0, x >= (BAR_W * 12'd6)}
                   + {2'd0, x >= (BAR_W * 12'd7)};
    // Fifteen boundaries, so the sum is 0 to 15 and fills four bits.
    wire [3:0] sx = step_of(x, STEP_W);
    wire [3:0] sy = step_of(y, STEP_H);

    // Seventeen per step, so sixteen steps cover 0 to 255 exactly and
    // the top one is full scale. 17*i is (i<<4) + i and costs an adder.
    function [7:0] level;
        input [3:0] i;
        level = {i, 4'd0} + {4'd0, i};
    endfunction

    // -----------------------------------------------------------------
    // The frame counter, for the one pattern that moves
    // -----------------------------------------------------------------
    // **`frame` is a level, not a pulse.** `video_timing` asserts it
    // combinationally while `h` and `v` are both zero, so at `dvi_tx`'s
    // five-clocks-per-pixel it is high for five edges and at one pixel
    // per clock for one. Counting `frame` alone would advance this by
    // five a frame on one path and one on the other; counting
    // `frame & pix_en` advances it once a frame on both, because a pixel
    // period contains exactly one enabled edge.
    // The sweeping bar's left edge. **The counter wraps at the width
    // rather than at its own width**, which is not fussiness: a free
    // twelve-bit counter reduced into the active area by one subtraction
    // is only right for the first two laps, and after that the bar walks
    // off the right-hand side and never comes back.
    reg [11:0] sweep_x;
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) sweep_x <= 12'd0;
        else if (pix_en && frame)
            sweep_x <= (sweep_x == H_ACTIVE[11:0] - 12'd1)
                       ? 12'd0 : sweep_x + 12'd1;
    end

    // -----------------------------------------------------------------
    // The patterns
    // -----------------------------------------------------------------
    localparam SQ_BIT = $clog2(SQUARE);
    localparam GP_BIT = $clog2(GRID_PITCH);

    wire checker = x[SQ_BIT] ^ y[SQ_BIT];
    wire hatch   = (x[GP_BIT-1:0] == {GP_BIT{1'b0}})
                || (y[GP_BIT-1:0] == {GP_BIT{1'b0}})
                || (x == 12'd0) || (x == H_ACTIVE[11:0] - 12'd1)
                || (y == 12'd0) || (y == V_ACTIVE[11:0] - 12'd1);
    wire in_sweep = (x >= sweep_x) && (x < sweep_x + 12'd16);

    reg [23:0] rgb;
    always @* begin
        case (pattern)
            // The standard order, descending in luminance. Each channel
            // is on for the bars whose index has the matching bit clear,
            // which is the order's own arithmetic: bar 0 is white and
            // bar 7 is black.
            // Red is bar[1], green is bar[2] and blue is bar[0]: the
            // order is a Gray-ish walk and not a plain count, so the
            // channels are not in index order and the obvious mapping
            // puts cyan where magenta belongs.
            P_BARS:    rgb = {{8{~bar[1]}}, {8{~bar[2]}}, {8{~bar[0]}}};
            P_STAIRS:  rgb = {3{level(sx)}};
            P_CHECKER: rgb = {24{checker}};
            P_GRID:    rgb = {24{hatch}};
            P_RGBSTEP: rgb = {level(sx), level(sy), 8'h00};
            P_SOLID:   rgb = SOLID_RGB;
            P_SWEEP:   rgb = {24{in_sweep}};
            P_WHITE:   rgb = 24'hFF_FF_FF;
            default:   rgb = 24'h00_00_00;
        endcase
        if (!de) rgb = 24'h00_00_00;
        r = rgb[23:16];
        g = rgb[15:8];
        b = rgb[7:0];
    end
endmodule
