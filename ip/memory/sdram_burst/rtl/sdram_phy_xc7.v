// sdram_phy_xc7 — the 7-series pins of `sdram_burst`.
//
// What it does
//   Every output to the SDRAM leaves through an ODDR in its pad's OLOGIC,
//   both edges given the same bit (SAME_EDGE), so each pin is launched by
//   the same kind of register at the same clock edge and the skew between
//   them is the board's and the output buffers'. The clock is forwarded
//   the same way, through an ODDR with D1 = 0 and D2 = 1: the pin carries
//   `clk` inverted, so the part samples a command, an address or a write
//   word half a cycle after the FPGA launched it, with half a cycle of
//   setup and half of hold (UG471, "ODDR" and its clock-forwarding use).
//
//   DQ. Each bit is a three-state buffer, written as `assign` and
//   inferred, whose output is an ODDR like the others and whose enable
//   is a register of the fabric. The controller raises its enable a
//   whole cycle before a write's first word and drops it a cycle after
//   the last, so the slower path from the fabric to T costs nothing.
//   The input
//   goes through an IDELAYE2 in VAR_LOAD mode, whose tap the controller
//   loads (`dly_tap` when `dly_ld` is high, on `clk`), into an IDDR in
//   SAME_EDGE_PIPELINED mode: Q1 is the bit sampled on a rising edge and
//   Q2 the one sampled on the falling edge after it, both handed over on
//   the next rising edge (UG471, "IDDR"). The controller chooses between
//   the two and among four latencies.
//
//   The IDELAYCTRL is here too, calibrating every delay of the bank
//   against `clk_ref`, which must be REFCLK_MHZ: 200 MHz on any grade, or
//   300 MHz on grades -2 and -3 (DS181 Table 25). The tap is then
//   1 / (32 x 2 x REFCLK) — 78 ps at 200 MHz, 52 ps at 300 — so the 32
//   taps span 2.4 ns or 1.6 ns.
//
// Two things this flow cannot build, and what is here instead
//   * An instantiated IOBUF on one bit of an inout vector: flattening
//     refuses the connection (`I0033`, "an inout port must share a net with
//     its connection"), so each bit is an inferred buffer instead.
//   * An ODDR into a buffer's T (the OLOGIC's three-state register, TQ):
//     the router finds "no path from the driver ... to `T`: the
//     architecture has no wire joining them". So the enable is a fabric
//     register, as above.
//
// What it does not do
//   No clock generation: `clk` and `clk_ref` come from the caller's PLL or
//   MMCM, on global buffers. No output delay (an Artix-7 HR bank has no
//   ODELAYE2), so the forwarded clock's phase is fixed at half a cycle.
module sdram_phy_xc7 #(
    parameter REFCLK_MHZ = 200
) (
    input  wire        clk,
    input  wire        clk_ref,
    // High while the reference clock is not yet good.
    input  wire        ref_rst,
    output wire        dly_rdy,

    // From the controller.
    input  wire        cke,
    input  wire        cs_n,
    input  wire        ras_n,
    input  wire        cas_n,
    input  wire        we_n,
    input  wire [1:0]  ba,
    input  wire [12:0] a,
    input  wire [1:0]  dqm,
    input  wire [15:0] dq_o,
    input  wire        dq_oe,
    input  wire [4:0]  dly_tap,
    input  wire        dly_ld,
    output wire [15:0] dq_rise,
    output wire [15:0] dq_fall,

    // The pins.
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
    localparam real REF = REFCLK_MHZ;

    IDELAYCTRL ctl (.REFCLK(clk_ref), .RST(ref_rst), .RDY(dly_rdy));

    ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0)) o_clk (
        .Q(sdram_clk), .C(clk), .CE(1'b1), .D1(1'b0), .D2(1'b1), .R(1'b0), .S(1'b0));

    // Commands and addresses: 1 + 4 + 2 + 13 + 2 = 22 pins.
    wire [21:0] ctl_in  = {cke, cs_n, ras_n, cas_n, we_n, ba, a, dqm};
    wire [21:0] ctl_out;
    // CS# starts high (deselected) and DQM high, as §7.1 wants them.
    localparam [21:0] CTL_INIT = {1'b0, 1'b1, 1'b1, 1'b1, 1'b1, 2'b00, 13'd0, 2'b11};
    genvar i;
    generate
        for (i = 0; i < 22; i = i + 1) begin : octl
            ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(CTL_INIT[i])) o (
                .Q(ctl_out[i]), .C(clk), .CE(1'b1), .D1(ctl_in[i]), .D2(ctl_in[i]),
                .R(1'b0), .S(1'b0));
        end
    endgenerate
    assign {sdram_cke, sdram_cs_n, sdram_ras_n, sdram_cas_n, sdram_we_n,
            sdram_ba, sdram_a, sdram_dqm} = ctl_out;

    // The three-state control, a register of the fabric for each pin,
    // kept apart so each can sit by its pad: high releases.
    (* keep *) reg [15:0] dq_t;
    always @(posedge clk or posedge ref_rst)
        if (ref_rst) dq_t <= 16'hFFFF;
        else dq_t <= {16{~dq_oe}};

    generate
        for (i = 0; i < 16; i = i + 1) begin : dq
            wire o, dly;
            ODDR #(.DDR_CLK_EDGE("SAME_EDGE"), .INIT(1'b0)) od (
                .Q(o), .C(clk), .CE(1'b1), .D1(dq_o[i]), .D2(dq_o[i]), .R(1'b0), .S(1'b0));
            assign sdram_dq[i] = dq_t[i] ? 1'bz : o;
            IDELAYE2 #(
                .IDELAY_TYPE("VAR_LOAD"),
                .DELAY_SRC("IDATAIN"),
                .IDELAY_VALUE(0),
                .HIGH_PERFORMANCE_MODE("TRUE"),
                .REFCLK_FREQUENCY(REF),
                .SIGNAL_PATTERN("DATA")
            ) d (
                .IDATAIN(sdram_dq[i]), .DATAOUT(dly), .C(clk), .CE(1'b0), .INC(1'b0),
                .LD(dly_ld), .CNTVALUEIN(dly_tap), .LDPIPEEN(1'b0), .REGRST(1'b0),
                .CINVCTRL(1'b0), .DATAIN(1'b0), .CNTVALUEOUT());
            IDDR #(.DDR_CLK_EDGE("SAME_EDGE_PIPELINED")) id (
                .Q1(dq_rise[i]), .Q2(dq_fall[i]), .C(clk), .CE(1'b1), .D(dly),
                .R(1'b0), .S(1'b0));
        end
    endgenerate
endmodule
