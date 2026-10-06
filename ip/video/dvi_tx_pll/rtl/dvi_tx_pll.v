// dvi_tx_pll — `dvi_tx` with its five-times clock made by the device's PLL.
//
// What it does
//   Declares `clk_x5`, uses it, leaves it undriven and puts a
//   `clock_mhz` attribute on it: that is how a design asks Reticle's FPGA
//   flow for a PLL (docs/fpga.md, "Generated clocks"). The flow
//   instantiates the device's PLL fed from `clk_ref`, which the board's
//   constraints must describe with a `create_clock`, and reports the
//   frequency it achieved and the error. The request is five times the
//   pixel clock rounded to a whole MHz — 126 for 640 x 480, whose pixel
//   clock is 25.175 MHz, which is 0.1 % fast and well inside what a
//   monitor accepts; 200 for 800 x 600; 371 for 1280 x 720.
//
// What it does not do
//   The PLL's lock output is not brought out by the flow, so the reset
//   is `rst_n` alone; hold it long enough for the PLL to lock (a few
//   hundred microseconds) or the first frame comes out ragged. The
//   wrapper cannot be simulated, because nothing in the source drives
//   its clock — simulate `dvi_tx` with a clock of its own, as the tests
//   do.
module dvi_tx_pll #(
    parameter MODE = 0
) (
    input  wire        clk_ref,
    input  wire        rst_n,
    output wire        pix_en,
    output wire        de,
    output wire        frame,
    output wire [11:0] x,
    output wire [11:0] y,
    input  wire [7:0]  r,
    input  wire [7:0]  g,
    input  wire [7:0]  b,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d0,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d1,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_d2,
    (* ddr = "clk_x5" *)
    output wire [1:0]  tmds_clk
);
    localparam X5_MHZ = (MODE == 2) ? 371 : (MODE == 1) ? 200 : 126;

    (* clock_mhz = X5_MHZ *)
    wire clk_x5;

    dvi_tx #(.MODE(MODE)) u_dvi (
        .clk_x5   (clk_x5),
        .rst_n    (rst_n),
        .pix_en   (pix_en),
        .de       (de),
        .frame    (frame),
        .x        (x),
        .y        (y),
        .r        (r),
        .g        (g),
        .b        (b),
        .tmds_d0  (tmds_d0),
        .tmds_d1  (tmds_d1),
        .tmds_d2  (tmds_d2),
        .tmds_clk (tmds_clk)
    );
endmodule
