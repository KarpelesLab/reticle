// dvi_tx — DVI video output: a raster, TMDS encoding, 10:1 serialisers.
//
// What it does
//   Produces a DVI signal — which every HDMI sink also accepts — for
//   one of the three modes `video_timing` knows. The whole block runs on
//   `clk_x5`, five times the pixel clock, and a counter divides it into
//   pixels: `pix_en` is high for one cycle in five, and everything that
//   happens once a pixel (the raster, the encoders) happens on it.
//
//   The user side: `x`, `y` and `de` name the pixel being fetched and
//   change on a `pix_en` edge; the colour for it is expected on `r`,
//   `g`, `b` by the next `pix_en` edge, which gives a fetch four cycles
//   of `clk_x5` — a combinational pattern, or one clocked lookup into a
//   frame buffer. `frame` pulses with the top left pixel.
//
//   Three `tmds_encoder`s turn the colour into ten-bit symbols, blue on
//   channel 0 with hsync and vsync as its control bits during blanking,
//   green on 1, red on 2. Each symbol goes out over five cycles of
//   `clk_x5` through a double-data-rate output register — the `ddr`
//   attribute on each two-bit port — two bits a cycle, bit 0 first on
//   the rising edge. The clock channel sends 1111100000 in the same
//   way, which is the pixel clock, aligned with the symbol boundaries.
//
//   One clock domain. The pixel rate is an enable, not a clock, so
//   nothing crosses between clocks: the words the serialisers take are
//   written by the same clock that shifts them out, which a pixel clock
//   and a separate five-times clock could only promise by their phase.
//   `dvi_tx_pll` in the same package gets `clk_x5` from the device's PLL,
//   asked for with a `clock_mhz` attribute.
//
// What it does not do
//   DVI only: no HDMI data islands, so no audio, no InfoFrames and no
//   colour space other than full-range RGB. No DDC or hot-plug, which
//   are a separate I2C conversation and a pin.
//
//   Only the positive leg of each pair. A TMDS pair is differential; on
//   the ECP5 a pseudo-differential IO standard (LVCMOS33D) makes the
//   pair from one output, which is a line in the constraints, and a
//   family without one needs a second port driving the complement.
//
//   The whole block runs at five times the pixel rate: 126 MHz for
//   640 x 480, 200 MHz for 800 x 600 and 371.25 MHz for 1280 x 720. The
//   first closes on both supported families, the second is at the edge of
//   the iCE40, and the third is beyond the fabric of either — it needs
//   the ECP5's gearing primitives (ODDRX2F and a pixel-rate clock
//   domain), which this block does not use.
module dvi_tx #(
    parameter MODE = 0
) (
    input  wire        clk_x5,
    input  wire        rst_n,

    // The pixel being fetched, and the colour for it.
    output wire        pix_en,
    output wire        de,
    output wire        frame,
    output wire [11:0] x,
    output wire [11:0] y,
    input  wire [7:0]  r,
    input  wire [7:0]  g,
    input  wire [7:0]  b,

    // The TMDS lanes, two bits per pin per cycle of `clk_x5`.
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d0,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d1,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d2,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_clk
);
    reg [2:0] phase;
    wire      en = (phase == 3'd4);

    wire hsync, vsync;
    video_timing #(.MODE(MODE)) u_timing (
        .clk   (clk_x5),
        .rst_n (rst_n),
        .en    (en),
        .de    (de),
        .hsync (hsync),
        .vsync (vsync),
        .frame (frame),
        .x     (x),
        .y     (y)
    );

    wire [9:0] sym0, sym1, sym2;
    tmds_encoder u_blue (
        .clk (clk_x5), .rst_n (rst_n), .en (en),
        .de (de), .c ({vsync, hsync}), .d (b), .q (sym0)
    );
    tmds_encoder u_green (
        .clk (clk_x5), .rst_n (rst_n), .en (en),
        .de (de), .c (2'b00), .d (g), .q (sym1)
    );
    tmds_encoder u_red (
        .clk (clk_x5), .rst_n (rst_n), .en (en),
        .de (de), .c (2'b00), .d (r), .q (sym2)
    );

    // The serialisers: loaded in the cycle after the encoders, then
    // shifted two bits a cycle for the next four.
    reg        load;
    reg [9:0]  sh0, sh1, sh2, shc;
    reg [1:0]  out0, out1, out2, outc;

    assign pix_en   = en;
    assign tmds_d0  = out0;
    assign tmds_d1  = out1;
    assign tmds_d2  = out2;
    assign tmds_clk = outc;

    always @(posedge clk_x5 or negedge rst_n) begin
        if (!rst_n) begin
            phase <= 3'd0;
            load  <= 1'b0;
            sh0   <= 10'd0;
            sh1   <= 10'd0;
            sh2   <= 10'd0;
            shc   <= 10'd0;
            out0  <= 2'b00;
            out1  <= 2'b00;
            out2  <= 2'b00;
            outc  <= 2'b00;
        end else begin
            phase <= en ? 3'd0 : phase + 3'd1;
            load  <= en;
            if (load) begin
                out0 <= sym0[1:0];
                out1 <= sym1[1:0];
                out2 <= sym2[1:0];
                outc <= 2'b11;
                sh0  <= {2'b00, sym0[9:2]};
                sh1  <= {2'b00, sym1[9:2]};
                sh2  <= {2'b00, sym2[9:2]};
                shc  <= 10'b0000000111;
            end else begin
                out0 <= sh0[1:0];
                out1 <= sh1[1:0];
                out2 <= sh2[1:0];
                outc <= shc[1:0];
                sh0  <= {2'b00, sh0[9:2]};
                sh1  <= {2'b00, sh1[9:2]};
                sh2  <= {2'b00, sh2[9:2]};
                shc  <= {2'b00, shc[9:2]};
            end
        end
    end
endmodule
