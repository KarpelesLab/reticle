// sdram_burst_xc7 — `sdram_burst` with its 7-series pins: the controller
// and `sdram_phy_xc7`, wired.
//
// The ports are the controller's client ports and status, the two clocks
// and the SDRAM's pins. `clk` is the controller's clock (166 MHz for a
// -6 part at CAS latency 3) and `clk_ref` the IDELAYCTRL's reference;
// both come from the caller's PLL on global buffers. `ref_rst` holds the
// IDELAYCTRL in reset until that PLL has locked, and `dly_rdy` says it has
// calibrated: it is up within 3.67 µs of the reset (DS181 Table 25), long
// before the controller's 200 µs power-up pause is over, so the read
// calibration that follows the pause always runs on calibrated taps.
module sdram_burst_xc7 #(
    parameter NPORTS         = 4,
    parameter [31:0] PRIO    = 32'h8,
    parameter BATCH          = 8,
    parameter CLK_MHZ        = 166,
    parameter CLK_PS         = 1000000 / CLK_MHZ,
    parameter CAS_LATENCY    = 3,
    parameter T_INIT_US      = 200,
    parameter INIT_REFRESHES = 8,
    parameter T_RCD_NS       = 15,
    parameter T_RP_NS        = 15,
    parameter T_RAS_NS       = 42,
    parameter T_RC_NS        = 60,
    parameter T_RFC_NS       = 60,
    parameter T_REFI_NS      = 7812,
    parameter T_WR           = 2,
    parameter T_RRD          = 2,
    parameter T_MRD          = 2,
    parameter RD2WR_GAP      = 3,
    // Seven cycles of this PHY and its pins before the latency-0
    // candidate, and the nominal latency 1: README §"Read capture".
    parameter PHY_RD_LAT     = 8,
    parameter CAL_EN         = 1,
    parameter CAL_BANK       = 3,
    parameter CAL_ROW        = 8191,
    parameter DEF_TAP        = 0,
    parameter DEF_EDGE       = 0,
    parameter DEF_LAT        = 1,
    parameter REFCLK_MHZ     = 200
) (
    input  wire                    clk,
    input  wire                    clk_ref,
    input  wire                    rst_n,
    input  wire                    ref_rst,
    output wire                    dly_rdy,

    input  wire [NPORTS-1:0]       cmd_valid,
    output wire [NPORTS-1:0]       cmd_ready,
    input  wire [NPORTS-1:0]       cmd_we,
    input  wire [NPORTS*24-1:0]    cmd_addr,
    input  wire [NPORTS*4-1:0]     cmd_len,
    input  wire [NPORTS-1:0]       wr_valid,
    output wire [NPORTS-1:0]       wr_ready,
    input  wire [NPORTS*16-1:0]    wr_data,
    input  wire [NPORTS*2-1:0]     wr_be,
    output wire [NPORTS-1:0]       rd_valid,
    output wire [NPORTS*16-1:0]    rd_data,
    output wire [NPORTS-1:0]       rd_last,

    output wire                    init_done,
    input  wire                    cal_req,
    output wire                    cal_busy,
    output wire                    cal_ok,
    output wire [4:0]              cal_tap,
    output wire                    cal_edge,
    output wire [1:0]              cal_lat,

    output wire                    sdram_clk,
    output wire                    sdram_cke,
    output wire                    sdram_cs_n,
    output wire                    sdram_ras_n,
    output wire                    sdram_cas_n,
    output wire                    sdram_we_n,
    output wire [1:0]              sdram_ba,
    output wire [12:0]             sdram_a,
    output wire [1:0]              sdram_dqm,
    inout  wire [15:0]             sdram_dq
);
    wire        cke, cs_n, ras_n, cas_n, we_n, dq_oe, dly_ld;
    wire [1:0]  ba, dqm;
    wire [12:0] a;
    wire [15:0] dq_o, dq_rise, dq_fall;
    wire [4:0]  dly_tap;

    sdram_burst #(
        .NPORTS(NPORTS), .PRIO(PRIO), .BATCH(BATCH), .CLK_MHZ(CLK_MHZ), .CLK_PS(CLK_PS),
        .CAS_LATENCY(CAS_LATENCY), .T_INIT_US(T_INIT_US),
        .INIT_REFRESHES(INIT_REFRESHES), .T_RCD_NS(T_RCD_NS), .T_RP_NS(T_RP_NS),
        .T_RAS_NS(T_RAS_NS), .T_RC_NS(T_RC_NS), .T_RFC_NS(T_RFC_NS),
        .T_REFI_NS(T_REFI_NS), .T_WR(T_WR), .T_RRD(T_RRD), .T_MRD(T_MRD),
        .RD2WR_GAP(RD2WR_GAP), .PHY_RD_LAT(PHY_RD_LAT), .CAL_EN(CAL_EN),
        .CAL_BANK(CAL_BANK), .CAL_ROW(CAL_ROW), .DEF_TAP(DEF_TAP),
        .DEF_EDGE(DEF_EDGE), .DEF_LAT(DEF_LAT)
    ) ctl (
        .clk(clk), .rst_n(rst_n),
        .cmd_valid(cmd_valid), .cmd_ready(cmd_ready), .cmd_we(cmd_we),
        .cmd_addr(cmd_addr), .cmd_len(cmd_len),
        .wr_valid(wr_valid), .wr_ready(wr_ready), .wr_data(wr_data), .wr_be(wr_be),
        .rd_valid(rd_valid), .rd_data(rd_data), .rd_last(rd_last),
        .init_done(init_done), .cal_req(cal_req), .cal_busy(cal_busy),
        .cal_ok(cal_ok), .cal_tap(cal_tap), .cal_edge(cal_edge), .cal_lat(cal_lat),
        .sdram_cke(cke), .sdram_cs_n(cs_n), .sdram_ras_n(ras_n),
        .sdram_cas_n(cas_n), .sdram_we_n(we_n), .sdram_ba(ba), .sdram_a(a),
        .sdram_dqm(dqm), .sdram_dq_o(dq_o), .sdram_dq_oe(dq_oe),
        .dq_rise(dq_rise), .dq_fall(dq_fall), .dly_tap(dly_tap), .dly_ld(dly_ld)
    );

    sdram_phy_xc7 #(.REFCLK_MHZ(REFCLK_MHZ)) phy (
        .clk(clk), .clk_ref(clk_ref), .ref_rst(ref_rst), .dly_rdy(dly_rdy),
        .cke(cke), .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n),
        .ba(ba), .a(a), .dqm(dqm), .dq_o(dq_o), .dq_oe(dq_oe),
        .dly_tap(dly_tap), .dly_ld(dly_ld), .dq_rise(dq_rise), .dq_fall(dq_fall),
        .sdram_clk(sdram_clk), .sdram_cke(sdram_cke), .sdram_cs_n(sdram_cs_n),
        .sdram_ras_n(sdram_ras_n), .sdram_cas_n(sdram_cas_n),
        .sdram_we_n(sdram_we_n), .sdram_ba(sdram_ba), .sdram_a(sdram_a),
        .sdram_dqm(sdram_dqm), .sdram_dq(sdram_dq)
    );
endmodule
