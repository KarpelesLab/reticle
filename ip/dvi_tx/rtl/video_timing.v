// video_timing — the raster of one of three standard video modes.
//
// What it does
//   Counts pixels and lines for the mode MODE selects and says, for the
//   pixel it is on, whether it is visible (`de`), where the syncs are,
//   and which pixel it is (`x`, `y`, meaningful while `de` is high).
//   Everything advances on an edge where `en` is high, so a design
//   running faster than the pixel rate gives it a pixel enable.
//
//     MODE  resolution       pixel clock   h: active fp sync bp   v: active fp sync bp   sync
//     0     640 x 480 @ 60   25.175 MHz      640  16  96  48       480  10  2  33        - -
//     1     800 x 600 @ 60   40.000 MHz      800  40 128  88       600   1  4  23        + +
//     2    1280 x 720 @ 60   74.250 MHz     1280 110  40 220       720   5  5  20        + +
//
//   Those are the VESA DMT timings for the first two and CEA-861's for
//   the third. Each line is active pixels, front porch, sync, back
//   porch; each frame is the same in lines. `frame` pulses for the
//   first pixel of each frame, the top left one.
//
// What it does not do
//   No other modes, no interlace, no reduced blanking. The pixel clock
//   is the user's to make: this block counts, it does not know what
//   frequency it runs at.
module video_timing #(
    parameter MODE = 0
) (
    input  wire        clk,
    input  wire        rst_n,
    input  wire        en,
    output wire        de,
    output wire        hsync,
    output wire        vsync,
    output wire        frame,
    output wire [11:0] x,
    output wire [11:0] y
);
    localparam H_ACTIVE = (MODE == 2) ? 1280 : (MODE == 1) ? 800 : 640;
    localparam H_FP     = (MODE == 2) ? 110  : (MODE == 1) ? 40  : 16;
    localparam H_SYNC   = (MODE == 2) ? 40   : (MODE == 1) ? 128 : 96;
    localparam H_BP     = (MODE == 2) ? 220  : (MODE == 1) ? 88  : 48;
    localparam V_ACTIVE = (MODE == 2) ? 720  : (MODE == 1) ? 600 : 480;
    localparam V_FP     = (MODE == 2) ? 5    : (MODE == 1) ? 1   : 10;
    localparam V_SYNC   = (MODE == 2) ? 5    : (MODE == 1) ? 4   : 2;
    localparam V_BP     = (MODE == 2) ? 20   : (MODE == 1) ? 23  : 33;
    // Sync polarity: 1 for a pulse that is high.
    localparam SYNC_POS = (MODE == 0) ? 0 : 1;

    localparam [11:0] H_LAST       = H_ACTIVE + H_FP + H_SYNC + H_BP - 1;
    localparam [11:0] V_LAST       = V_ACTIVE + V_FP + V_SYNC + V_BP - 1;
    localparam [11:0] H_VISIBLE    = H_ACTIVE;
    localparam [11:0] V_VISIBLE    = V_ACTIVE;
    localparam [11:0] H_SYNC_START = H_ACTIVE + H_FP;
    localparam [11:0] H_SYNC_END   = H_ACTIVE + H_FP + H_SYNC;
    localparam [11:0] V_SYNC_START = V_ACTIVE + V_FP;
    localparam [11:0] V_SYNC_END   = V_ACTIVE + V_FP + V_SYNC;
    localparam        POLARITY     = SYNC_POS;

    reg [11:0] h;
    reg [11:0] v;

    wire h_sync_on = (h >= H_SYNC_START) & (h < H_SYNC_END);
    wire v_sync_on = (v >= V_SYNC_START) & (v < V_SYNC_END);

    assign de    = (h < H_VISIBLE) & (v < V_VISIBLE);
    assign hsync = POLARITY ? h_sync_on : ~h_sync_on;
    assign vsync = POLARITY ? v_sync_on : ~v_sync_on;
    assign frame = (h == 12'd0) & (v == 12'd0);
    assign x     = h;
    assign y     = v;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            h <= 12'd0;
            v <= 12'd0;
        end else if (en) begin
            if (h == H_LAST) begin
                h <= 12'd0;
                v <= (v == V_LAST) ? 12'd0 : v + 12'd1;
            end else begin
                h <= h + 12'd1;
            end
        end
    end
endmodule
