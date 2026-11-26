// sdram_burst_fpga — a whole design around `sdram_burst_xc7`, for measuring
// the controller with `reticle fpga`: the clocks from a PLL, and four
// clients that keep every port in use.
//
// The clients are pseudo-random: each is a 32-bit LFSR that offers a
// command when one bit says so, with its address, length and direction
// from others, and write data from the register; read data is folded into
// one bit a port. They are not a test of anything — the simulation in
// tests/ip_sdram_burst.rs is — but they make every client input depend on
// a register and every output reach a pin, so synthesis keeps all of the
// controller and the timing report is the controller's.
//
// Clocks: 50 MHz in, a PLLE2 at VCO 1000 MHz (DS181: 800 to 1866 MHz at
// grade -2), 166.67 MHz (CLKOUT1, divide by 6) for the controller and
// 200 MHz (CLKOUT2, divide by 5) for the IDELAYCTRL. Not CLKOUT0: logic
// clocked from a PLLE2's CLKOUT0 is in no domain of this flow's timing
// estimate (2026-11-27), so the controller would not have been timed.
module sdram_burst_fpga (
    input  wire        clk_in,
    input  wire        rst_n,
    output wire [3:0]  led,
    output wire        sdram_clk,
    output wire        sdram_cke,
    output wire        sdram_cs_n,
    output wire        sdram_ras_n,
    output wire        sdram_cas_n,
    output wire        sdram_we_n,
    output wire [1:0]  sdram_ba,
    output wire [12:0] sdram_a,
    output wire [1:0]  sdram_dqm,
    inout  wire [15:0] sdram_dq
);
    wire fb, c166, c200, locked, clk, clk_ref;
    PLLE2_BASE #(
        .CLKIN1_PERIOD(20.0), .DIVCLK_DIVIDE(1), .CLKFBOUT_MULT(20),
        .CLKOUT1_DIVIDE(6), .CLKOUT2_DIVIDE(5)
    ) pll (
        .CLKIN1(clk_in), .CLKFBIN(fb), .CLKFBOUT(fb), .CLKOUT1(c166),
        .CLKOUT2(c200), .LOCKED(locked), .RST(1'b0), .PWRDWN(1'b0)
    );
    BUFG g0 (.I(c166), .O(clk));
    BUFG g1 (.I(c200), .O(clk_ref));

    // Reset: held until the PLL locks, released synchronously.
    reg [1:0] rs;
    always @(posedge clk or negedge locked)
        if (!locked) rs <= 2'b00;
        else rs <= {rs[0], rst_n};
    wire srst_n = rs[1];

    localparam N = 4;
    reg  [31:0]      lfsr [0:N-1];
    reg  [N-1:0]     fold;
    wire [N-1:0]     cmd_valid, cmd_ready, cmd_we, wr_valid, wr_ready;
    wire [N-1:0]     rd_valid, rd_last;
    wire [N*24-1:0]  cmd_addr;
    wire [N*4-1:0]   cmd_len;
    wire [N*16-1:0]  wr_data, rd_data;
    wire [N*2-1:0]   wr_be;
    genvar p;
    generate
        for (p = 0; p < N; p = p + 1) begin : client
            wire [31:0] r = lfsr[p];
            assign cmd_valid[p] = r[0];
            assign cmd_we[p]    = r[1] & (p != 3);
            assign cmd_len[p*4 +: 4] = r[5:2];
            // Stay inside a row: start no later than column 496.
            assign cmd_addr[p*24 +: 24] = {r[28:13], r[12], r[11:5], 1'b0} & 24'hFFFDFF;
            assign wr_valid[p] = r[6] | r[7];
            assign wr_data[p*16 +: 16] = r[31:16];
            assign wr_be[p*2 +: 2] = 2'b11;
            always @(posedge clk or negedge srst_n)
                if (!srst_n) begin
                    lfsr[p] <= 32'h1 + p;
                    fold[p] <= 1'b0;
                end else begin
                    lfsr[p] <= {lfsr[p][30:0], lfsr[p][31] ^ lfsr[p][21] ^ lfsr[p][1] ^ lfsr[p][0]};
                    if (rd_valid[p]) fold[p] <= fold[p] ^ (^rd_data[p*16 +: 16]) ^ rd_last[p];
                end
        end
    endgenerate

    wire init_done, cal_ok, cal_busy, dly_rdy, cal_edge;
    wire [4:0] cal_tap;
    wire [1:0] cal_lat;
    sdram_burst_xc7 #(.CLK_PS(6000)) mem (
        .clk(clk), .clk_ref(clk_ref), .rst_n(srst_n), .ref_rst(~locked),
        .dly_rdy(dly_rdy),
        .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_we(cmd_we),
        .cmd_addr(cmd_addr), .cmd_len(cmd_len),
        .wr_valid(wr_valid), .wr_ready(wr_ready), .wr_data(wr_data), .wr_be(wr_be),
        .rd_valid(rd_valid), .rd_data(rd_data), .rd_last(rd_last),
        .init_done(init_done), .cal_req(1'b0), .cal_busy(cal_busy),
        .cal_ok(cal_ok), .cal_tap(cal_tap), .cal_edge(cal_edge), .cal_lat(cal_lat),
        .sdram_clk(sdram_clk), .sdram_cke(sdram_cke), .sdram_cs_n(sdram_cs_n),
        .sdram_ras_n(sdram_ras_n), .sdram_cas_n(sdram_cas_n),
        .sdram_we_n(sdram_we_n), .sdram_ba(sdram_ba), .sdram_a(sdram_a),
        .sdram_dqm(sdram_dqm), .sdram_dq(sdram_dq)
    );

    reg [3:0] led_q;
    always @(posedge clk or negedge srst_n)
        if (!srst_n) led_q <= 4'd0;
        else led_q <= {^fold ^ ^cmd_ready ^ ^wr_ready, init_done & dly_rdy,
                       cal_ok ^ cal_busy ^ cal_edge, ^cal_tap ^ ^cal_lat};
    assign led = led_q;
endmodule
