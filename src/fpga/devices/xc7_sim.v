// 7-series hard blocks, as a simulator sees them, and the lookup tables
// again, for what a simulator makes of an input it does not know.
//
// For simulation only: `reticle sim --device <7-series part>` takes a module
// from here when a design names it and declares nothing of that name. A build
// never reads this file, because a hard block a design names is the device's
// own primitive, and a model in its place would be mapped onto logic.
//
// Written from what each block is documented to do (UG471 for the IO logic,
// UG472 for the clocking, UG473 for block RAM, UG479 for the DSP, UG953 for
// the attributes) and from what Vivado's simulator was seen to answer on the
// testbenches in testdata/sim/xc7/, whose expected outputs are xsim's
// (tests/sim_xc7_primitives.rs). None of the vendor's source is here.
//
// The global set/reset that xsim's `glbl` holds for the first 100 ns is
// modelled in each block that has state: it holds the block at its initial
// value and ignores its clocks. Not modelled anywhere in this file: timing,
// and every setup or hold check.

`timescale 1ps / 1ps

// ---------------------------------------------------------- lookup tables

// The lookup tables are in the cell library (xc7_cells.v) too, written as
// `INIT[index]`, which is what a build maps. A simulator reading that takes
// the language's rule, that an index with an unknown bit reads x, and xsim
// does not: it gives the value every entry the index could name agrees on,
// and x only when they differ (testdata/sim/xc7/lut_unknown_inputs.v). The
// difference is more than cosmetic. The 7-series flow puts two functions
// in one LUT6_2, and one can reach the other's input through the fabric:
// O5 a function of I0 and I3 that the truth table says ignores I1, and I1
// the sum O5's propagate makes through a CARRY4. Under the language's rule
// that is a loop no event ever leaves x, and the routed netlist of the
// language corpus's `v_alu` gave x on every output for it while xsim ran
// it correctly.
module xc7_sim_lut #(parameter integer K = 6, parameter [63:0] INIT = 64'h0)
    (output o, input [K-1:0] i);
  // Over every entry an index with unknown bits could name.
  function agree(input [K-1:0] sel);
    integer j, b;
    reg seen, value, same, named;
    begin
      seen = 1'b0;
      value = 1'bx;
      same = 1'b1;
      for (j = 0; j < (1 << K); j = j + 1) begin
        named = 1'b1;
        for (b = 0; b < K; b = b + 1)
          if (sel[b] === 1'b0 && ((j >> b) & 1) == 1 || sel[b] === 1'b1 && ((j >> b) & 1) == 0)
            named = 1'b0;
        if (named) begin
          if (!seen) value = INIT[j];
          else if (INIT[j] !== value) same = 1'b0;
          seen = 1'b1;
        end
      end
      agree = same ? value : 1'bx;
    end
  endfunction
  assign o = ^i === 1'bx ? agree(i) : INIT[i];
endmodule

module LUT1 #(parameter [1:0] INIT = 2'h0) (output O, input I0);
  xc7_sim_lut #(.K(1), .INIT(INIT)) l (.o(O), .i(I0));
endmodule

module LUT1_L #(parameter [1:0] INIT = 2'h0) (output LO, input I0);
  xc7_sim_lut #(.K(1), .INIT(INIT)) l (.o(LO), .i(I0));
endmodule

module LUT1_D #(parameter [1:0] INIT = 2'h0) (output O, output LO, input I0);
  xc7_sim_lut #(.K(1), .INIT(INIT)) l (.o(O), .i(I0));
  assign LO = O;
endmodule

module LUT2 #(parameter [3:0] INIT = 4'h0) (output O, input I0, I1);
  xc7_sim_lut #(.K(2), .INIT(INIT)) l (.o(O), .i({I1, I0}));
endmodule

module LUT2_L #(parameter [3:0] INIT = 4'h0) (output LO, input I0, I1);
  xc7_sim_lut #(.K(2), .INIT(INIT)) l (.o(LO), .i({I1, I0}));
endmodule

module LUT2_D #(parameter [3:0] INIT = 4'h0) (output O, output LO, input I0, I1);
  xc7_sim_lut #(.K(2), .INIT(INIT)) l (.o(O), .i({I1, I0}));
  assign LO = O;
endmodule

module LUT3 #(parameter [7:0] INIT = 8'h0) (output O, input I0, I1, I2);
  xc7_sim_lut #(.K(3), .INIT(INIT)) l (.o(O), .i({I2, I1, I0}));
endmodule

module LUT3_L #(parameter [7:0] INIT = 8'h0) (output LO, input I0, I1, I2);
  xc7_sim_lut #(.K(3), .INIT(INIT)) l (.o(LO), .i({I2, I1, I0}));
endmodule

module LUT3_D #(parameter [7:0] INIT = 8'h0) (output O, output LO, input I0, I1, I2);
  xc7_sim_lut #(.K(3), .INIT(INIT)) l (.o(O), .i({I2, I1, I0}));
  assign LO = O;
endmodule

module LUT4 #(parameter [15:0] INIT = 16'h0) (output O, input I0, I1, I2, I3);
  xc7_sim_lut #(.K(4), .INIT(INIT)) l (.o(O), .i({I3, I2, I1, I0}));
endmodule

module LUT4_L #(parameter [15:0] INIT = 16'h0) (output LO, input I0, I1, I2, I3);
  xc7_sim_lut #(.K(4), .INIT(INIT)) l (.o(LO), .i({I3, I2, I1, I0}));
endmodule

module LUT4_D #(parameter [15:0] INIT = 16'h0) (output O, output LO, input I0, I1, I2, I3);
  xc7_sim_lut #(.K(4), .INIT(INIT)) l (.o(O), .i({I3, I2, I1, I0}));
  assign LO = O;
endmodule

module LUT5 #(parameter [31:0] INIT = 32'h0) (output O, input I0, I1, I2, I3, I4);
  xc7_sim_lut #(.K(5), .INIT(INIT)) l (.o(O), .i({I4, I3, I2, I1, I0}));
endmodule

module LUT5_L #(parameter [31:0] INIT = 32'h0) (output LO, input I0, I1, I2, I3, I4);
  xc7_sim_lut #(.K(5), .INIT(INIT)) l (.o(LO), .i({I4, I3, I2, I1, I0}));
endmodule

module LUT5_D #(parameter [31:0] INIT = 32'h0) (output O, output LO, input I0, I1, I2, I3, I4);
  xc7_sim_lut #(.K(5), .INIT(INIT)) l (.o(O), .i({I4, I3, I2, I1, I0}));
  assign LO = O;
endmodule

module LUT6 #(parameter [63:0] INIT = 64'h0) (output O, input I0, I1, I2, I3, I4, I5);
  xc7_sim_lut #(.K(6), .INIT(INIT)) l (.o(O), .i({I5, I4, I3, I2, I1, I0}));
endmodule

module LUT6_L #(parameter [63:0] INIT = 64'h0) (output LO, input I0, I1, I2, I3, I4, I5);
  xc7_sim_lut #(.K(6), .INIT(INIT)) l (.o(LO), .i({I5, I4, I3, I2, I1, I0}));
endmodule

module LUT6_D #(parameter [63:0] INIT = 64'h0) (output O, output LO,
    input I0, I1, I2, I3, I4, I5);
  xc7_sim_lut #(.K(6), .INIT(INIT)) l (.o(O), .i({I5, I4, I3, I2, I1, I0}));
  assign LO = O;
endmodule

// O5 is the lower half alone, whatever I5 is.
module LUT6_2 #(parameter [63:0] INIT = 64'h0) (output O5, output O6,
    input I0, I1, I2, I3, I4, I5);
  xc7_sim_lut #(.K(5), .INIT({32'h0, INIT[31:0]})) l5 (.o(O5), .i({I4, I3, I2, I1, I0}));
  xc7_sim_lut #(.K(6), .INIT(INIT)) l6 (.o(O6), .i({I5, I4, I3, I2, I1, I0}));
endmodule

// ---------------------------------------------------------------- IO logic

module IDDR #(parameter DDR_CLK_EDGE = "OPPOSITE_EDGE", parameter [0:0] INIT_Q1 = 1'b0,
    parameter [0:0] INIT_Q2 = 1'b0, parameter [0:0] IS_C_INVERTED = 1'b0,
    parameter [0:0] IS_D_INVERTED = 1'b0, parameter SRTYPE = "SYNC")
  (output Q1, output Q2, input C, input CE, input D, input R, input S);
  wire c = C ^ IS_C_INVERTED;
  wire d = D ^ IS_D_INVERTED;
  localparam OPPOSITE = DDR_CLK_EDGE == "OPPOSITE_EDGE";
  localparam PIPELINED = DDR_CLK_EDGE == "SAME_EDGE_PIPELINED";
  // q1 and q2 are the outputs; rise and fall hold what each edge captured
  // until the rising edge hands it on.
  reg q1 = INIT_Q1, q2 = INIT_Q2, rise = INIT_Q1, fall = INIT_Q2;
  assign Q1 = q1;
  assign Q2 = q2;
  // The global set/reset holds everything at INIT for the first 100 ns.
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  // One block, since a register here has one driver: R and S wake it only
  // when they are asynchronous, and the end of the global reset only then.
  localparam ASYNC = SRTYPE == "ASYNC";
  wire r = ASYNC ? R : 1'b0;
  wire s = ASYNC ? S : 1'b0;
  wire released = ASYNC ? gsr : 1'b1;
  reg c_was = 1'bx;
  // A synchronous R or S acts on each register at that register's own edge:
  // in SAME_EDGE the falling edge resets `fall` and leaves the outputs alone.
  always @(posedge c or negedge c or posedge r or posedge s or posedge gsr or negedge released) begin
    if (gsr) {q1, q2, rise, fall} <= {INIT_Q1, INIT_Q2, INIT_Q1, INIT_Q2};
    else if (ASYNC && R) {q1, q2, rise, fall} <= 4'b0000;
    else if (ASYNC && S) {q1, q2, rise, fall} <= 4'b1111;
    else if (c !== c_was && c) begin
      if (R) {q1, rise} <= 2'b00;
      else if (S) {q1, rise} <= 2'b11;
      else if (CE) begin
        q1 <= PIPELINED ? rise : d;
        rise <= d;
      end
      if (!OPPOSITE) begin
        if (R) q2 <= 1'b0;
        else if (S) q2 <= 1'b1;
        else if (CE) q2 <= fall;
      end
    end else if (c !== c_was) begin
      if (OPPOSITE) begin
        if (R) q2 <= 1'b0;
        else if (S) q2 <= 1'b1;
        else if (CE) q2 <= d;
      end else if (R) fall <= 1'b0;
      else if (S) fall <= 1'b1;
      else if (CE) fall <= d;
    end
    c_was = c;
  end
endmodule

module ODDR #(parameter DDR_CLK_EDGE = "OPPOSITE_EDGE", parameter [0:0] INIT = 1'b0,
    parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_D1_INVERTED = 1'b0,
    parameter [0:0] IS_D2_INVERTED = 1'b0, parameter SRTYPE = "SYNC")
  (output Q, input C, input CE, input D1, input D2, input R, input S);
  wire c = C ^ IS_C_INVERTED;
  wire d1 = D1 ^ IS_D1_INVERTED;
  wire d2 = D2 ^ IS_D2_INVERTED;
  localparam OPPOSITE = DDR_CLK_EDGE == "OPPOSITE_EDGE";
  // One output register, which both edges load while CE is high; in
  // SAME_EDGE `held` takes D2 on the rising edge for the falling one.
  reg q = INIT, held = INIT, armed = 1'b0;
  assign Q = q;
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  localparam ASYNC = SRTYPE == "ASYNC";
  wire r = ASYNC ? R : 1'b0;
  wire s = ASYNC ? S : 1'b0;
  wire released = ASYNC ? gsr : 1'b1;
  reg c_was = 1'bx;
  always @(posedge c or negedge c or posedge r or posedge s or posedge gsr or negedge released) begin
    if (gsr) {q, held} <= {INIT, INIT};
    else if (ASYNC && R) {q, held} <= 2'b00;
    else if (ASYNC && S) {q, held} <= 2'b11;
    else if (c !== c_was && c) begin
      if (R) {q, held} <= 2'b00;
      else if (S) {q, held} <= 2'b11;
      else if (CE) {q, held} <= {d1, d2};
      armed <= CE;
    end else if (c !== c_was) begin
      if (R) q <= 1'b0;
      else if (S) q <= 1'b1;
      else if (CE && (OPPOSITE || armed)) q <= OPPOSITE ? d2 : held;
    end
    c_was = c;
  end
endmodule

// The output serialiser. A word is taken on each rising edge of CLKDIV and
// put out a bit an edge of CLK (both edges in DDR, rising ones in SDR),
// starting the number of CLK cycles after it that UG471 tabulates as the
// latency: 1, 3, 4, 4, 5, 5, 6 for SDR 2 to 8 and 1, 3, 4 for DDR 4, 6, 8,
// which is also what xsim was seen to do. Not modelled: DATA_WIDTH 10 and 14
// (a master and a slave), SHIFTIN and SHIFTOUT, and TBYTE.
module OSERDESE2 #(parameter DATA_RATE_OQ = "DDR", parameter DATA_RATE_TQ = "DDR",
    parameter integer DATA_WIDTH = 4, parameter [0:0] INIT_OQ = 1'b0,
    parameter [0:0] INIT_TQ = 1'b0, parameter SERDES_MODE = "MASTER",
    parameter [0:0] SRVAL_OQ = 1'b0, parameter [0:0] SRVAL_TQ = 1'b0,
    parameter TBYTE_CTL = "FALSE", parameter TBYTE_SRC = "FALSE",
    parameter integer TRISTATE_WIDTH = 4, parameter SIM_DEVICE = "7SERIES",
    parameter [0:0] IS_CLKDIV_INVERTED = 1'b0, parameter [0:0] IS_CLK_INVERTED = 1'b0,
    parameter [0:0] IS_D1_INVERTED = 1'b0, parameter [0:0] IS_D2_INVERTED = 1'b0,
    parameter [0:0] IS_D3_INVERTED = 1'b0, parameter [0:0] IS_D4_INVERTED = 1'b0,
    parameter [0:0] IS_D5_INVERTED = 1'b0, parameter [0:0] IS_D6_INVERTED = 1'b0,
    parameter [0:0] IS_D7_INVERTED = 1'b0, parameter [0:0] IS_D8_INVERTED = 1'b0,
    parameter [0:0] IS_T1_INVERTED = 1'b0, parameter [0:0] IS_T2_INVERTED = 1'b0,
    parameter [0:0] IS_T3_INVERTED = 1'b0, parameter [0:0] IS_T4_INVERTED = 1'b0)
  (output OFB, output OQ, output SHIFTOUT1, output SHIFTOUT2, output TBYTEOUT, output TFB,
   output TQ, input CLK, input CLKDIV, input D1, input D2, input D3, input D4, input D5,
   input D6, input D7, input D8, input OCE, input RST, input SHIFTIN1, input SHIFTIN2,
   input T1, input T2, input T3, input T4, input TBYTEIN, input TCE);
  localparam DDR = DATA_RATE_OQ == "DDR";
  localparam integer STEP = DDR ? 1 : 2;
  localparam integer W = DATA_WIDTH;
  localparam integer LAT = DDR ? (W == 4 ? 1 : W == 6 ? 3 : 4) :
    (W == 2 ? 1 : W == 3 ? 3 : W <= 5 ? 4 : W <= 7 ? 5 : 6);
  wire clk = CLK ^ IS_CLK_INVERTED;
  wire clkdiv = CLKDIV ^ IS_CLKDIV_INVERTED;
  wire [7:0] d = {D8 ^ IS_D8_INVERTED, D7 ^ IS_D7_INVERTED, D6 ^ IS_D6_INVERTED,
    D5 ^ IS_D5_INVERTED, D4 ^ IS_D4_INVERTED, D3 ^ IS_D3_INVERTED, D2 ^ IS_D2_INVERTED,
    D1 ^ IS_D1_INVERTED};
  wire [3:0] t = {T4 ^ IS_T4_INVERTED, T3 ^ IS_T3_INVERTED, T2 ^ IS_T2_INVERTED,
    T1 ^ IS_T1_INVERTED};
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg oq = INIT_OQ, tq = INIT_TQ;
  assign OQ = oq;
  assign OFB = oq;
  assign TQ = DATA_RATE_TQ == "BUF" ? t[0] : tq;
  assign TFB = TQ;
  assign {SHIFTOUT1, SHIFTOUT2, TBYTEOUT} = 3'b000;
  // The last two words taken, and the CLK edge each was taken on.
  reg [7:0] word0 = 8'h00, word1 = 8'h00;
  reg [3:0] tword0 = 4'h0, tword1 = 4'h0;
  // e counts CLK's edges, both of them; a word's bit b is due on edge
  // at + 2 * LAT + b * STEP. After a reset OQ stays at SRVAL_OQ until three
  // CLK cycles after the first CLKDIV edge (`oq_from`), which drops the
  // first word's early bits when the latency is short; a four-bit tristate
  // word is serialised as the data is, and a one-bit one is T1 registered
  // on each rising edge of CLK. That is xsim's behaviour.
  // Bit b of a word is due on edge at + 2 * LAT + b * STEP; an edge with OCE
  // (or TCE) low leaves OQ (or TQ) as it was. Where OCE moves in the middle of
  // a word xsim's serialiser stalls in a way this does not model: it is
  // right for OCE held high, and for OCE low across whole words.
  integer e = 0, at0 = -1000, at1 = -1000, k, oq_from = 0;
  reg clk_was = 1'bx, div_was = 1'bx, starting = 1'b1;
  always @(clk or clkdiv or posedge RST or posedge gsr) begin
    if (gsr || RST) begin
      oq <= gsr ? INIT_OQ : SRVAL_OQ;
      tq <= gsr ? INIT_TQ : SRVAL_TQ;
      at0 = -1000;
      at1 = -1000;
      starting = 1'b1;
    end else begin
      if (clk !== clk_was) begin
        e = e + 1;
        if (clk === 1'b1 && TRISTATE_WIDTH == 1 && TCE) tq <= t[0];
        if (clk === 1'b1 || DDR) begin
          // The bit due on this edge, of whichever word it is.
          k = e - at1 - 2 * LAT;
          if (k >= 0 && k < W * STEP && k % STEP == 0) begin
            if (OCE && e >= oq_from) oq <= word1[k / STEP];
            if (TCE && TRISTATE_WIDTH == 4) tq <= tword1[k / STEP];
          end
          k = e - at0 - 2 * LAT;
          if (k >= 0 && k < W * STEP && k % STEP == 0) begin
            if (OCE && e >= oq_from) oq <= word0[k / STEP];
            if (TCE && TRISTATE_WIDTH == 4) tq <= tword0[k / STEP];
          end
        end
      end
      if (clkdiv === 1'b1 && div_was === 1'b0) begin
        if (starting) oq_from = e + 6;
        starting = 1'b0;
        {word1, tword1, at1} = {word0, tword0, at0};
        word0 = d;
        tword0 = t;
        at0 = e;
      end
    end
    clk_was = clk;
    div_was = clkdiv;
  end
endmodule

// The input deserialiser in NETWORKING mode. The serial input is sampled on
// each rising edge of CLK (SDR) or of CLK and CLKB (DDR), and on each rising
// edge of CLKDIV Q1..Q8 take the eight newest samples but the last few, Q1
// the newest: the lag, in half periods of CLK, is 2, 2, 4, 6, 8, 10, 12 for
// SDR 2 to 8 and 3, 3, 5 for DDR 4, 6, 8, xsim's answer, and Q beyond the
// width goes on with older samples. Out of reset the outputs stay at SRVAL
// until the first CLKDIV edge at least three CLK cycles after the first one.
// Not modelled: BITSLIP, the MEMORY interface types and OVERSAMPLE, CE1 and
// CE2 low, the dynamic clock inversions, and SHIFTIN/SHIFTOUT.
module ISERDESE2 #(parameter DATA_RATE = "DDR", parameter integer DATA_WIDTH = 4,
    parameter DYN_CLKDIV_INV_EN = "FALSE", parameter DYN_CLK_INV_EN = "FALSE",
    parameter [0:0] INIT_Q1 = 1'b0, parameter [0:0] INIT_Q2 = 1'b0,
    parameter [0:0] INIT_Q3 = 1'b0, parameter [0:0] INIT_Q4 = 1'b0,
    parameter INTERFACE_TYPE = "MEMORY", parameter IOBDELAY = "NONE",
    parameter integer NUM_CE = 2, parameter OFB_USED = "FALSE",
    parameter SERDES_MODE = "MASTER", parameter [0:0] SRVAL_Q1 = 1'b0,
    parameter [0:0] SRVAL_Q2 = 1'b0, parameter [0:0] SRVAL_Q3 = 1'b0,
    parameter [0:0] SRVAL_Q4 = 1'b0, parameter SIM_DEVICE = "7SERIES",
    parameter [0:0] IS_CLKB_INVERTED = 1'b0, parameter [0:0] IS_CLKDIVP_INVERTED = 1'b0,
    parameter [0:0] IS_CLKDIV_INVERTED = 1'b0, parameter [0:0] IS_CLK_INVERTED = 1'b0,
    parameter [0:0] IS_D_INVERTED = 1'b0, parameter [0:0] IS_OCLKB_INVERTED = 1'b0,
    parameter [0:0] IS_OCLK_INVERTED = 1'b0)
  (output O, output Q1, output Q2, output Q3, output Q4, output Q5, output Q6, output Q7,
   output Q8, output SHIFTOUT1, output SHIFTOUT2, input BITSLIP, input CE1, input CE2,
   input CLKDIVP, input CLK, input CLKB, input CLKDIV, input D, input DDLY,
   input DYNCLKDIVSEL, input DYNCLKSEL, input OCLK, input OCLKB, input OFB, input RST,
   input SHIFTIN1, input SHIFTIN2);
  localparam DDR = DATA_RATE == "DDR";
  localparam integer W = DATA_WIDTH;
  localparam integer STEP = DDR ? 1 : 2;
  localparam integer LAG = DDR ? (W <= 6 ? 3 : 5) : (W <= 3 ? 2 : 2 * W - 4);
  wire d = D ^ IS_D_INVERTED;
  assign O = IOBDELAY == "IBUF" || IOBDELAY == "BOTH" ? DDLY : d;
  wire serial = OFB_USED == "TRUE" ? OFB : IOBDELAY == "IFD" || IOBDELAY == "BOTH" ? DDLY : d;
  wire clk = CLK ^ IS_CLK_INVERTED;
  wire clkb = CLKB ^ IS_CLKB_INVERTED;
  wire clkdiv = CLKDIV ^ IS_CLKDIV_INVERTED;
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  localparam [7:0] INIT = {4'b0000, INIT_Q4, INIT_Q3, INIT_Q2, INIT_Q1};
  localparam [7:0] SRVAL = {4'b0000, SRVAL_Q4, SRVAL_Q3, SRVAL_Q2, SRVAL_Q1};
  reg [7:0] q = INIT;
  assign {Q8, Q7, Q6, Q5, Q4, Q3, Q2, Q1} = q;
  assign {SHIFTOUT1, SHIFTOUT2} = 2'b00;
  // The samples, by the half period of CLK each was taken in.
  reg samples [0:63];
  integer e = 0, j, from = 0;
  reg clk_was = 1'bx, clkb_was = 1'bx, div_was = 1'bx, starting = 1'b1;
  always @(clk or clkb or clkdiv or posedge RST or posedge gsr) begin
    if (gsr || RST) begin
      // A reset empties the samples as well as Q.
      q <= gsr ? INIT : SRVAL;
      for (j = 0; j < 64; j = j + 1) samples[j] = 1'b0;
      starting = 1'b1;
    end else begin
      if (clk !== clk_was) begin
        e = e + 1;
        if (clk === 1'b1) samples[e % 64] = serial;
      end
      if (DDR && clkb === 1'b1 && clkb_was === 1'b0) samples[e % 64] = serial;
      if (clkdiv === 1'b1 && div_was === 1'b0) begin
        if (starting) begin
          from = e + 6;
          starting = 1'b0;
        end
        if (e >= from)
          for (j = 0; j < 8; j = j + 1) q[j] <= samples[(e - LAG - j * STEP + 64) % 64];
      end
    end
    clk_was = clk;
    clkb_was = clkb;
    div_was = clkdiv;
  end
endmodule

// The input delay, as two stages: an ideal variable line of the tap value
// times a tap, a tap being 1/64 of the reference clock's period in whole
// picoseconds (78 ps at 200 MHz), and then a fixed 600 ps. The first stage is
// ideal, as xsim's is: when the value changes, it shows at once the input as
// it was the new delay ago, so a change while an edge is in flight can put a
// short pulse out, 600 ps later. FIXED and VARIABLE start at IDELAY_VALUE and
// the loading types at zero; on a rising edge of C, LD loads IDELAY_VALUE
// (VARIABLE) or CNTVALUEIN, or else CE moves the value by one each way, round
// from 31 to 0. Not modelled: VAR_LOAD_PIPE's pipeline register apart from
// LD (it loads as VAR_LOAD does), REGRST, CINVCTRL, and the line's first
// nanosecond, where xsim's output has transitions of its own from x.
module IDELAYE2 #(parameter CINVCTRL_SEL = "FALSE", parameter DELAY_SRC = "IDATAIN",
    parameter HIGH_PERFORMANCE_MODE = "FALSE", parameter IDELAY_TYPE = "FIXED",
    parameter integer IDELAY_VALUE = 0, parameter PIPE_SEL = "FALSE",
    parameter real REFCLK_FREQUENCY = 200.0, parameter SIGNAL_PATTERN = "DATA",
    parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_DATAIN_INVERTED = 1'b0,
    parameter [0:0] IS_IDATAIN_INVERTED = 1'b0)
  (output [4:0] CNTVALUEOUT, output DATAOUT, input C, input CE, input CINVCTRL,
   input [4:0] CNTVALUEIN, input DATAIN, input IDATAIN, input INC, input LD, input LDPIPEEN,
   input REGRST);
  localparam integer TAP = $rtoi(1000000.0 / (64.0 * REFCLK_FREQUENCY));
  localparam integer BASE = 600;
  localparam LOADS = IDELAY_TYPE == "VAR_LOAD" || IDELAY_TYPE == "VAR_LOAD_PIPE";
  wire in = DELAY_SRC == "DATAIN" ? DATAIN ^ IS_DATAIN_INVERTED : IDATAIN ^ IS_IDATAIN_INVERTED;
  wire c = C ^ IS_C_INVERTED;
  reg [4:0] value = LOADS ? 5'd0 : IDELAY_VALUE;
  assign CNTVALUEOUT = value;
  always @(posedge c)
    if (IDELAY_TYPE != "FIXED") begin
      if (LD) value <= LOADS ? CNTVALUEIN : IDELAY_VALUE;
      else if (CE) value <= INC ? value + 5'd1 : value - 5'd1;
    end

  // The last changes of the input (`a_*`) and of the first stage (`b_*`),
  // as (time, level) in rings.
  localparam integer H = 32;
  reg [63:0] a_at [0:H-1], b_at [0:H-1];
  reg a_was [0:H-1], b_was [0:H-1];
  integer a_new = 0, b_new = 0, k;
  reg stage = 1'bx, out = 1'bx;
  assign DATAOUT = out;
  function level_a(input [63:0] t);
    integer i;
    begin
      level_a = 1'bx;
      for (i = H - 1; i >= 0; i = i - 1)
        if (a_at[(a_new - i + H) % H] <= t) level_a = a_was[(a_new - i + H) % H];
    end
  endfunction
  function level_b(input [63:0] t);
    integer i;
    begin
      level_b = 1'bx;
      for (i = H - 1; i >= 0; i = i - 1)
        if (b_at[(b_new - i + H) % H] <= t) level_b = b_was[(b_new - i + H) % H];
    end
  endfunction

  // A pool of timers. One of kind 0 puts the first stage right when it is
  // due, unless the delay has changed since it was set; one of kind 1 puts
  // the output right 600 ps after the first stage moved.
  localparam integer N = 16;
  reg [63:0] due [0:N-1];
  reg [31:0] tag [0:N-1];
  reg kind [0:N-1];
  reg [N-1:0] go = 0, done = 0;
  reg [31:0] epoch = 0;
  integer next = 0;
  genvar g;
  for (g = 0; g < N; g = g + 1) begin : timers
    always @(go[g]) begin
      if (due[g] > $time) #(due[g] - $time);
      done[g] = ~done[g];
    end
  end
  task arm(input [63:0] t, input which);
    begin
      due[next] = t;
      tag[next] = epoch;
      kind[next] = which;
      go[next] = ~go[next];
      next = (next + 1) % N;
    end
  endtask
  // The first stage takes level `v` now.
  task stage_to(input v);
    if (v !== stage) begin
      stage = v;
      b_new = (b_new + 1) % H;
      b_at[b_new] = $time;
      b_was[b_new] = v;
      arm($time + BASE, 1'b1);
    end
  endtask
  reg [4:0] used;
  integer m;
  initial begin
    for (m = 0; m < H; m = m + 1) begin
      a_at[m] = 0;
      a_was[m] = 1'bx;
      b_at[m] = 0;
      b_was[m] = 1'bx;
    end
    used = value;
  end
  always @(in or value or done) begin
    if (in !== a_was[a_new]) begin
      a_new = (a_new + 1) % H;
      a_at[a_new] = $time;
      a_was[a_new] = in;
      arm($time + value * TAP, 1'b0);
    end
    if (value !== used) begin
      used = value;
      epoch = epoch + 1;
      for (k = 0; k < H; k = k + 1)
        if (a_at[k] + value * TAP > $time) arm(a_at[k] + value * TAP, 1'b0);
      if ($time >= value * TAP) stage_to(level_a($time - value * TAP));
    end
    for (k = 0; k < N; k = k + 1)
      if (due[k] == $time && kind[k] == 1'b0 && tag[k] == epoch)
        stage_to(level_a($time - value * TAP));
    for (k = 0; k < N; k = k + 1)
      if (due[k] == $time && kind[k] == 1'b1) out = level_b($time - BASE);
  end
endmodule

// The delay controller's RDY, as xsim has it: after two rising edges of
// REFCLK out of reset it rises 100 ps after the next falling one, and once
// it has, a later reset drops it only while RST is high. Not modelled: RDY
// falling when REFCLK stops.
module IDELAYCTRL #(parameter SIM_DEVICE = "7SERIES") (output RDY, input REFCLK, input RST);
  reg rdy = 1'b0;
  assign RDY = rdy;
  integer rises = 0;
  reg calibrated = 1'b0, ref_was = 1'bx;
  always @(REFCLK or RST) begin
    if (RST) rdy <= 1'b0;
    else if (calibrated) rdy <= 1'b1;
    else if (REFCLK === 1'b1 && ref_was !== 1'b1) rises = rises + 1;
    else if (REFCLK === 1'b0 && ref_was !== 1'b0 && rises >= 2) begin
      calibrated = 1'b1;
      rdy <= #100 1'b1;
    end
    ref_was = REFCLK;
  end
endmodule

// ---------------------------------------------------------------- IO buffers

// A buffer passes 0 and 1 and makes x of x and z. A differential input is 1
// for I high and IB low, 0 for the other way round, holds what it had when
// the two are equal, and is x for anything unknown. An enabled
// IBUFDISABLE puts a 1 out. Pulls are drives as strong as any other: the
// simulator has no strengths, so a pulled net something also drives reads x
// where xsim would let the driver win, and KEEPER does nothing.

// A differential receiver. An x on either side is x, and equal levels or a z
// hold what it had, except where EQUAL_X makes equal levels x (the
// bidirectional differential buffers with two outputs, in xsim).
module xc7_sim_diff #(parameter EQUAL_X = 0) (output reg o, input i, input ib);
  initial o = 1'bx;
  always @(i or ib)
    if (i === 1'b1 && ib === 1'b0) o = 1'b1;
    else if (i === 1'b0 && ib === 1'b1) o = 1'b0;
    else if (i === 1'bx || ib === 1'bx || i === 1'bz && ib === 1'bz) o = 1'bx;
    else if (EQUAL_X && i === ib && i !== 1'bz) o = 1'bx;
endmodule

module IBUF #(parameter CAPACITANCE = "DONT_CARE", parameter IBUF_DELAY_VALUE = "0",
    parameter IBUF_LOW_PWR = "TRUE", parameter IFD_DELAY_VALUE = "AUTO",
    parameter IOSTANDARD = "DEFAULT") (output O, input I);
  assign O = I ^ 1'b0;
endmodule

module OBUF #(parameter CAPACITANCE = "DONT_CARE", parameter integer DRIVE = 12,
    parameter IOSTANDARD = "DEFAULT", parameter SLEW = "SLOW") (output O, input I);
  assign O = I ^ 1'b0;
endmodule

module OBUFT #(parameter CAPACITANCE = "DONT_CARE", parameter integer DRIVE = 12,
    parameter IOSTANDARD = "DEFAULT", parameter SLEW = "SLOW") (output O, input I, input T);
  assign O = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
endmodule

module IOBUF #(parameter integer DRIVE = 12, parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter SLEW = "SLOW")
  (output O, inout IO, input I, input T);
  assign IO = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
  assign O = IO ^ 1'b0;
endmodule

module IBUFDS #(parameter CAPACITANCE = "DONT_CARE", parameter DIFF_TERM = "FALSE",
    parameter DQS_BIAS = "FALSE", parameter IBUF_DELAY_VALUE = "0",
    parameter IBUF_LOW_PWR = "TRUE", parameter IFD_DELAY_VALUE = "AUTO",
    parameter IOSTANDARD = "DEFAULT") (output O, input I, input IB);
  xc7_sim_diff u (.o(O), .i(I), .ib(IB));
endmodule

module IBUFGDS #(parameter CAPACITANCE = "DONT_CARE", parameter DIFF_TERM = "FALSE",
    parameter IBUF_DELAY_VALUE = "0", parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT") (output O, input I, input IB);
  xc7_sim_diff u (.o(O), .i(I), .ib(IB));
endmodule

module OBUFDS #(parameter CAPACITANCE = "DONT_CARE", parameter IOSTANDARD = "DEFAULT",
    parameter SLEW = "SLOW") (output O, output OB, input I);
  assign O = I ^ 1'b0;
  assign OB = ~I;
endmodule

module OBUFTDS #(parameter CAPACITANCE = "DONT_CARE", parameter IOSTANDARD = "DEFAULT",
    parameter SLEW = "SLOW") (output O, output OB, input I, input T);
  assign O = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
  assign OB = T === 1'b0 ? ~I : T === 1'b1 ? 1'bz : 1'bx;
endmodule

module IBUFDS_DIFF_OUT #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT")
  (output O, output OB, input I, input IB);
  xc7_sim_diff u (.o(O), .i(I), .ib(IB));
  assign OB = ~O;
endmodule

module IBUF_IBUFDISABLE #(parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SIM_DEVICE = "7SERIES", parameter USE_IBUFDISABLE = "TRUE")
  (output O, input I, input IBUFDISABLE);
  assign O = USE_IBUFDISABLE != "TRUE" ? I : IBUFDISABLE === 1'b1 ? 1'b1 :
    IBUFDISABLE === 1'b0 ? I : 1'bx;
endmodule

module IBUF_INTERMDISABLE #(parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SIM_DEVICE = "7SERIES", parameter USE_IBUFDISABLE = "TRUE")
  (output O, input I, input IBUFDISABLE, input INTERMDISABLE);
  assign O = USE_IBUFDISABLE != "TRUE" ? I : IBUFDISABLE === 1'b1 ? 1'b1 :
    IBUFDISABLE === 1'b0 ? I : 1'bx;
endmodule

module IBUFDS_IBUFDISABLE #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SIM_DEVICE = "7SERIES", parameter USE_IBUFDISABLE = "TRUE")
  (output O, input I, input IB, input IBUFDISABLE);
  wire o;
  xc7_sim_diff u (.o(o), .i(I), .ib(IB));
  assign O = USE_IBUFDISABLE != "TRUE" ? o : IBUFDISABLE === 1'b1 ? 1'b1 :
    IBUFDISABLE === 1'b0 ? o : 1'bx;
endmodule

module IBUFDS_INTERMDISABLE #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SIM_DEVICE = "7SERIES", parameter USE_IBUFDISABLE = "TRUE")
  (output O, input I, input IB, input IBUFDISABLE, input INTERMDISABLE);
  wire o;
  xc7_sim_diff u (.o(o), .i(I), .ib(IB));
  assign O = USE_IBUFDISABLE != "TRUE" ? o : IBUFDISABLE === 1'b1 ? 1'b1 :
    IBUFDISABLE === 1'b0 ? o : 1'bx;
endmodule

// Disabled, both outputs are 1.
module IBUFDS_DIFF_OUT_IBUFDISABLE #(parameter DIFF_TERM = "FALSE",
    parameter DQS_BIAS = "FALSE", parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter SIM_DEVICE = "7SERIES",
    parameter USE_IBUFDISABLE = "TRUE")
  (output O, output OB, input I, input IB, input IBUFDISABLE);
  wire o;
  xc7_sim_diff u (.o(o), .i(I), .ib(IB));
  wire off = USE_IBUFDISABLE == "TRUE" && IBUFDISABLE === 1'b1;
  wire unknown = USE_IBUFDISABLE == "TRUE" && IBUFDISABLE !== 1'b0 && IBUFDISABLE !== 1'b1;
  assign O = off ? 1'b1 : unknown ? 1'bx : o;
  assign OB = off ? 1'b1 : unknown ? 1'bx : ~o;
endmodule

module IBUFDS_DIFF_OUT_INTERMDISABLE #(parameter DIFF_TERM = "FALSE",
    parameter DQS_BIAS = "FALSE", parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter SIM_DEVICE = "7SERIES",
    parameter USE_IBUFDISABLE = "TRUE")
  (output O, output OB, input I, input IB, input IBUFDISABLE, input INTERMDISABLE);
  IBUFDS_DIFF_OUT_IBUFDISABLE #(.USE_IBUFDISABLE(USE_IBUFDISABLE))
    u (.O(O), .OB(OB), .I(I), .IB(IB), .IBUFDISABLE(IBUFDISABLE));
endmodule

module IOBUFDS #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SLEW = "SLOW") (output O, inout IO, inout IOB, input I, input T);
  assign IO = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
  assign IOB = T === 1'b0 ? ~I : T === 1'b1 ? 1'bz : 1'bx;
  xc7_sim_diff u (.o(O), .i(IO), .ib(IOB));
endmodule

// With the output on (T low) the input reads 1, and the pad passes as it
// is, z and all, when it is off: xsim's answer.
module IOBUF_INTERMDISABLE #(parameter integer DRIVE = 12, parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter SIM_DEVICE = "7SERIES",
    parameter SLEW = "SLOW", parameter USE_IBUFDISABLE = "TRUE")
  (output O, inout IO, input I, input IBUFDISABLE, input INTERMDISABLE, input T);
  assign IO = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
  assign O = USE_IBUFDISABLE != "TRUE" ? IO : T === 1'b0 ? 1'b1 :
    IBUFDISABLE === 1'b1 ? 1'b1 : IBUFDISABLE === 1'b0 ? IO : 1'bx;
endmodule

module IOBUFDS_INTERMDISABLE #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT",
    parameter SIM_DEVICE = "7SERIES", parameter SLEW = "SLOW",
    parameter USE_IBUFDISABLE = "TRUE")
  (output O, inout IO, inout IOB, input I, input IBUFDISABLE, input INTERMDISABLE,
   input T);
  assign IO = T === 1'b0 ? I ^ 1'b0 : T === 1'b1 ? 1'bz : 1'bx;
  assign IOB = T === 1'b0 ? ~I : T === 1'b1 ? 1'bz : 1'bx;
  wire o;
  xc7_sim_diff u (.o(o), .i(IO), .ib(IOB));
  assign O = USE_IBUFDISABLE != "TRUE" ? o : T === 1'b0 ? 1'b1 :
    IBUFDISABLE === 1'b1 ? 1'b1 : IBUFDISABLE === 1'b0 ? o : 1'bx;
endmodule

module IOBUFDS_DIFF_OUT #(parameter DIFF_TERM = "FALSE", parameter DQS_BIAS = "FALSE",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT")
  (output O, output OB, inout IO, inout IOB, input I, input TM, input TS);
  assign IO = TM === 1'b0 ? I ^ 1'b0 : TM === 1'b1 ? 1'bz : 1'bx;
  assign IOB = TS === 1'b0 ? ~I : TS === 1'b1 ? 1'bz : 1'bx;
  xc7_sim_diff #(.EQUAL_X(1)) u (.o(O), .i(IO), .ib(IOB));
  assign OB = ~O;
endmodule

module IOBUFDS_DIFF_OUT_INTERMDISABLE #(parameter DIFF_TERM = "FALSE",
    parameter DQS_BIAS = "FALSE", parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter SIM_DEVICE = "7SERIES",
    parameter USE_IBUFDISABLE = "TRUE")
  (output O, output OB, inout IO, inout IOB, input I, input IBUFDISABLE,
   input INTERMDISABLE, input TM, input TS);
  assign IO = TM === 1'b0 ? I ^ 1'b0 : TM === 1'b1 ? 1'bz : 1'bx;
  assign IOB = TS === 1'b0 ? ~I : TS === 1'b1 ? 1'bz : 1'bx;
  wire o;
  xc7_sim_diff #(.EQUAL_X(1)) u (.o(o), .i(IO), .ib(IOB));
  wire off = USE_IBUFDISABLE == "TRUE" && (IBUFDISABLE === 1'b1 || TM === 1'b0);
  assign O = off ? 1'b1 : o;
  assign OB = off ? 1'b1 : ~o;
endmodule

module PULLUP (output O);
  pullup (O);
endmodule

module PULLDOWN (output O);
  pulldown (O);
endmodule

module KEEPER (inout O);
endmodule

// ---------------------------------------------------------------- block RAM

// The array both block RAMs are: 2^ABITS data bits and an eighth as many
// parity bits, seen through two ports. A port's widths count parity (1, 2,
// 4, 9, 18, 36 or 72 bits; 0 for a port that does not read, or does not
// write). A W-bit access takes the address's top bits: a 9-bit one ignores
// the low three, and its byte is data bits 8a..8a+7 with parity bit a.
module xc7_sim_bram #(parameter integer ABITS = 14,
    parameter integer RA = 18, parameter integer WA = 18,
    parameter integer RB = 18, parameter integer WB = 18,
    parameter MODE_A = "WRITE_FIRST", parameter MODE_B = "WRITE_FIRST",
    parameter integer REG_A = 0, parameter integer REG_B = 0,
    parameter PRIO_A = "RSTREG", parameter PRIO_B = "RSTREG",
    parameter [63:0] INIT_DA = 64'h0, parameter [63:0] INIT_DB = 64'h0,
    parameter [63:0] SR_DA = 64'h0, parameter [63:0] SR_DB = 64'h0,
    parameter [7:0] INIT_PA = 8'h0, parameter [7:0] INIT_PB = 8'h0,
    parameter [7:0] SR_PA = 8'h0, parameter [7:0] SR_PB = 8'h0,
    parameter [(1 << ABITS) - 1:0] INIT = 0,
    parameter [(1 << (ABITS - 3)) - 1:0] INITP = 0)
  (input clka, input ena, input rstrama, input rstrega, input regcea, input [ABITS-1:0] addra,
   input [7:0] wea, input [63:0] dia, input [7:0] dipa, output [63:0] doa, output [7:0] dopa,
   input clkb, input enb, input rstramb, input rstregb, input regceb, input [ABITS-1:0] addrb,
   input [7:0] web, input [63:0] dib, input [7:0] dipb, output [63:0] dob, output [7:0] dopb);
  localparam integer WORDS = 1 << (ABITS - 4);
  reg [15:0] mem [0:WORDS-1];
  reg [1:0] par [0:WORDS-1];
  integer k;
  initial
    for (k = 0; k < WORDS; k = k + 1) begin
      mem[k] = INIT[k*16 +: 16];
      par[k] = INITP[k*2 +: 2];
    end

  // Data and parity bits in a word of `w` bits.
  function integer dw(input integer w);
    dw = w >= 9 ? w / 9 * 8 : w;
  endfunction
  function integer pw(input integer w);
    pw = w >= 9 ? w / 9 : 0;
  endfunction
  // A value as a port of width `w` puts it out: the bits it has not got
  // are unknown.
  function [63:0] only_d(input [63:0] v, input integer w);
    integer j;
    begin
      only_d = v;
      for (j = 0; j < 64; j = j + 1)
        if (j >= dw(w)) only_d[j] = 1'bx;
    end
  endfunction
  function [7:0] only_p(input [7:0] v, input integer w);
    integer j;
    begin
      only_p = v;
      for (j = 0; j < 8; j = j + 1)
        if (j >= pw(w)) only_p[j] = 1'bx;
    end
  endfunction

  reg [63:0] rd_d;
  reg [7:0] rd_p;
  // What a `w`-bit read of `addr` sees, into rd_d and rd_p.
  task read(input [ABITS-1:0] addr, input integer w);
    integer j, n, base, b;
    reg [15:0] word;
    begin
      rd_d = only_d(64'h0, w);
      rd_p = only_p(8'h0, w);
      n = dw(w);
      base = addr;
      base = base - base % n;
      for (j = 0; j < n; j = j + 1) begin
        b = base + j;
        word = mem[b / 16];
        rd_d[j] = word[b % 16];
      end
      n = pw(w);
      if (n > 0) begin
        base = addr;
        base = base / 8;
        base = base - base % n;
        for (j = 0; j < n; j = j + 1) begin
          b = base + j;
          rd_p[j] = par[b / 2][b % 2];
        end
      end
    end
  endtask
  // A `w`-bit write of `addr`, byte lane j enabled by we[j]; a write
  // narrower than a byte by we[0].
  task write(input [ABITS-1:0] addr, input integer w, input [7:0] we, input [63:0] d,
      input [7:0] p);
    integer j, n, base, b;
    reg [15:0] word;
    reg [1:0] pair;
    begin
      n = dw(w);
      base = addr;
      base = base - base % n;
      for (j = 0; j < n; j = j + 1)
        if (we[j / 8]) begin
          b = base + j;
          word = mem[b / 16];
          word[b % 16] = d[j];
          mem[b / 16] = word;
        end
      n = pw(w);
      if (n > 0) begin
        base = addr;
        base = base / 8;
        base = base - base % n;
        for (j = 0; j < n; j = j + 1)
          if (we[j]) begin
            b = base + j;
            pair = par[b / 2];
            pair[b % 2] = p[j];
            par[b / 2] = pair;
          end
      end
    end
  endtask
  // Whether a port of write width `w` writes anything for these enables.
  function writes(input integer w, input [7:0] we);
    writes = w == 0 ? 1'b0 : w <= 9 ? we[0] : w == 18 ? |we[1:0] : w == 36 ? |we[3:0] : |we;
  endfunction

  reg [63:0] la_d, lb_d, ra_d, rb_d;
  reg [7:0] la_p, lb_p, ra_p, rb_p;
  assign doa = REG_A ? ra_d : la_d;
  assign dopa = REG_A ? ra_p : la_p;
  assign dob = REG_B ? rb_d : lb_d;
  assign dopb = REG_B ? rb_p : lb_p;

  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg ca_was = 1'bx, cb_was = 1'bx;
  reg [63:0] old_d;
  reg [7:0] old_p;
  reg wrote;
  always @(clka or clkb or posedge gsr) begin
    if (gsr) begin
      {la_d, ra_d} <= {2{only_d(INIT_DA, RA)}};
      {la_p, ra_p} <= {2{only_p(INIT_PA, RA)}};
      {lb_d, rb_d} <= {2{only_d(INIT_DB, RB)}};
      {lb_p, rb_p} <= {2{only_p(INIT_PB, RB)}};
    end else begin
      if (clka === 1'b1 && ca_was === 1'b0) begin
        if (REG_A) begin
          if (PRIO_A == "RSTREG" ? rstrega : regcea && rstrega)
            {ra_d, ra_p} <= {only_d(SR_DA, RA), only_p(SR_PA, RA)};
          else if (regcea) {ra_d, ra_p} <= {la_d, la_p};
        end
        if (ena) begin
          wrote = writes(WA, wea);
          if (RA != 0) read(addra, RA);
          {old_d, old_p} = {rd_d, rd_p};
          if (wrote) write(addra, WA, wea, dia, dipa);
          if (RA != 0) read(addra, RA);
          if (rstrama) {la_d, la_p} <= {only_d(SR_DA, RA), only_p(SR_PA, RA)};
          else if (!wrote || MODE_A == "WRITE_FIRST") {la_d, la_p} <= {rd_d, rd_p};
          else if (MODE_A == "READ_FIRST") {la_d, la_p} <= {old_d, old_p};
        end
      end
      if (clkb === 1'b1 && cb_was === 1'b0) begin
        if (REG_B) begin
          if (PRIO_B == "RSTREG" ? rstregb : regceb && rstregb)
            {rb_d, rb_p} <= {only_d(SR_DB, RB), only_p(SR_PB, RB)};
          else if (regceb) {rb_d, rb_p} <= {lb_d, lb_p};
        end
        if (enb) begin
          wrote = writes(WB, web);
          if (RB != 0) read(addrb, RB);
          {old_d, old_p} = {rd_d, rd_p};
          if (wrote) write(addrb, WB, web, dib, dipb);
          if (RB != 0) read(addrb, RB);
          if (rstramb) {lb_d, lb_p} <= {only_d(SR_DB, RB), only_p(SR_PB, RB)};
          else if (!wrote || MODE_B == "WRITE_FIRST") {lb_d, lb_p} <= {rd_d, rd_p};
          else if (MODE_B == "READ_FIRST") {lb_d, lb_p} <= {old_d, old_p};
        end
      end
    end
    ca_was = clka;
    cb_was = clkb;
  end
endmodule

module RAMB18E1 #(parameter [17:0] INIT_A = 18'h0, parameter [17:0] INIT_B = 18'h0,
    parameter [17:0] SRVAL_A = 18'h0, parameter [17:0] SRVAL_B = 18'h0,
    parameter RAM_MODE = "TDP", parameter integer READ_WIDTH_A = 0,
    parameter integer READ_WIDTH_B = 0, parameter integer WRITE_WIDTH_A = 0,
    parameter integer WRITE_WIDTH_B = 0, parameter WRITE_MODE_A = "WRITE_FIRST",
    parameter WRITE_MODE_B = "WRITE_FIRST", parameter integer DOA_REG = 0,
    parameter integer DOB_REG = 0, parameter RSTREG_PRIORITY_A = "RSTREG",
    parameter RSTREG_PRIORITY_B = "RSTREG", parameter RDADDR_COLLISION_HWCONFIG = "DELAYED_WRITE",
    parameter SIM_COLLISION_CHECK = "ALL", parameter SIM_DEVICE = "7SERIES",
    parameter INIT_FILE = "NONE",
    parameter [0:0] IS_CLKARDCLK_INVERTED = 1'b0, parameter [0:0] IS_CLKBWRCLK_INVERTED = 1'b0,
    parameter [0:0] IS_ENARDEN_INVERTED = 1'b0, parameter [0:0] IS_ENBWREN_INVERTED = 1'b0,
    parameter [0:0] IS_RSTRAMARSTRAM_INVERTED = 1'b0, parameter [0:0] IS_RSTRAMB_INVERTED = 1'b0,
    parameter [0:0] IS_RSTREGARSTREG_INVERTED = 1'b0, parameter [0:0] IS_RSTREGB_INVERTED = 1'b0,
    parameter [255:0] INIT_00 = 256'h0, INIT_01 = 256'h0, INIT_02 = 256'h0, INIT_03 = 256'h0,
      INIT_04 = 256'h0, INIT_05 = 256'h0, INIT_06 = 256'h0, INIT_07 = 256'h0, INIT_08 = 256'h0,
      INIT_09 = 256'h0, INIT_0A = 256'h0, INIT_0B = 256'h0, INIT_0C = 256'h0, INIT_0D = 256'h0,
      INIT_0E = 256'h0, INIT_0F = 256'h0, INIT_10 = 256'h0, INIT_11 = 256'h0, INIT_12 = 256'h0,
      INIT_13 = 256'h0, INIT_14 = 256'h0, INIT_15 = 256'h0, INIT_16 = 256'h0, INIT_17 = 256'h0,
      INIT_18 = 256'h0, INIT_19 = 256'h0, INIT_1A = 256'h0, INIT_1B = 256'h0, INIT_1C = 256'h0,
      INIT_1D = 256'h0, INIT_1E = 256'h0, INIT_1F = 256'h0, INIT_20 = 256'h0, INIT_21 = 256'h0,
      INIT_22 = 256'h0, INIT_23 = 256'h0, INIT_24 = 256'h0, INIT_25 = 256'h0, INIT_26 = 256'h0,
      INIT_27 = 256'h0, INIT_28 = 256'h0, INIT_29 = 256'h0, INIT_2A = 256'h0, INIT_2B = 256'h0,
      INIT_2C = 256'h0, INIT_2D = 256'h0, INIT_2E = 256'h0, INIT_2F = 256'h0, INIT_30 = 256'h0,
      INIT_31 = 256'h0, INIT_32 = 256'h0, INIT_33 = 256'h0, INIT_34 = 256'h0, INIT_35 = 256'h0,
      INIT_36 = 256'h0, INIT_37 = 256'h0, INIT_38 = 256'h0, INIT_39 = 256'h0, INIT_3A = 256'h0,
      INIT_3B = 256'h0, INIT_3C = 256'h0, INIT_3D = 256'h0, INIT_3E = 256'h0, INIT_3F = 256'h0,
      INITP_00 = 256'h0, INITP_01 = 256'h0, INITP_02 = 256'h0, INITP_03 = 256'h0, INITP_04 = 256'h0,
      INITP_05 = 256'h0, INITP_06 = 256'h0, INITP_07 = 256'h0)
  (output [15:0] DOADO, output [15:0] DOBDO, output [1:0] DOPADOP, output [1:0] DOPBDOP,
   input CLKARDCLK, input CLKBWRCLK, input ENARDEN, input ENBWREN, input REGCEAREGCE,
   input REGCEB, input RSTRAMARSTRAM, input RSTRAMB, input RSTREGARSTREG, input RSTREGB,
   input [13:0] ADDRARDADDR, input [13:0] ADDRBWRADDR, input [15:0] DIADI, input [15:0] DIBDI,
   input [1:0] DIPADIP, input [1:0] DIPBDIP, input [1:0] WEA, input [3:0] WEBWE);
  localparam SDP = RAM_MODE == "SDP";
  // In SDP the A side reads and the B side writes, each as wide as both.
  localparam integer RA = READ_WIDTH_A;
  localparam integer WA = SDP ? 0 : WRITE_WIDTH_A;
  localparam integer RB = SDP ? 0 : READ_WIDTH_B;
  localparam integer WB = WRITE_WIDTH_B;
  localparam WIDE_R = SDP && RA == 36;
  localparam WIDE_W = SDP && WB == 36;
  function [63:0] data_of(input [17:0] v, input integer w);
    data_of = w >= 18 ? v[15:0] : w == 9 ? v[7:0] : v;
  endfunction
  function [7:0] par_of(input [17:0] v, input integer w);
    par_of = w >= 18 ? v[17:16] : w == 9 ? v[8] : 8'h0;
  endfunction
  localparam [63:0] INIT_DA = WIDE_R ? {INIT_B[15:0], INIT_A[15:0]} : data_of(INIT_A, RA);
  localparam [7:0] INIT_PA = WIDE_R ? {INIT_B[17:16], INIT_A[17:16]} : par_of(INIT_A, RA);
  localparam [63:0] SR_DA = WIDE_R ? {SRVAL_B[15:0], SRVAL_A[15:0]} : data_of(SRVAL_A, RA);
  localparam [7:0] SR_PA = WIDE_R ? {SRVAL_B[17:16], SRVAL_A[17:16]} : par_of(SRVAL_A, RA);
  wire [63:0] doa, dob;
  wire [7:0] dopa, dopb;
  assign DOADO = doa[15:0];
  assign DOPADOP = dopa[1:0];
  assign DOBDO = WIDE_R ? doa[31:16] : dob[15:0];
  assign DOPBDOP = WIDE_R ? dopa[3:2] : dopb[1:0];
  xc7_sim_bram #(.ABITS(14), .RA(RA), .WA(WA), .RB(RB), .WB(WB), .MODE_A(WRITE_MODE_A),
    .MODE_B(WRITE_MODE_B), .REG_A(DOA_REG), .REG_B(DOB_REG), .PRIO_A(RSTREG_PRIORITY_A),
    .PRIO_B(RSTREG_PRIORITY_B), .INIT_DA(INIT_DA), .INIT_PA(INIT_PA), .SR_DA(SR_DA),
    .SR_PA(SR_PA), .INIT_DB(data_of(INIT_B, RB)), .INIT_PB(par_of(INIT_B, RB)),
    .SR_DB(data_of(SRVAL_B, RB)), .SR_PB(par_of(SRVAL_B, RB)),
    .INIT({INIT_3F, INIT_3E, INIT_3D, INIT_3C, INIT_3B, INIT_3A, INIT_39, INIT_38, INIT_37, INIT_36, INIT_35,
      INIT_34, INIT_33, INIT_32, INIT_31, INIT_30, INIT_2F, INIT_2E, INIT_2D, INIT_2C, INIT_2B,
      INIT_2A, INIT_29, INIT_28, INIT_27, INIT_26, INIT_25, INIT_24, INIT_23, INIT_22, INIT_21,
      INIT_20, INIT_1F, INIT_1E, INIT_1D, INIT_1C, INIT_1B, INIT_1A, INIT_19, INIT_18, INIT_17,
      INIT_16, INIT_15, INIT_14, INIT_13, INIT_12, INIT_11, INIT_10, INIT_0F, INIT_0E, INIT_0D,
      INIT_0C, INIT_0B, INIT_0A, INIT_09, INIT_08, INIT_07, INIT_06, INIT_05, INIT_04, INIT_03,
      INIT_02, INIT_01, INIT_00}),
    .INITP({INITP_07, INITP_06, INITP_05, INITP_04, INITP_03, INITP_02, INITP_01, INITP_00}))
    core (.clka(CLKARDCLK ^ IS_CLKARDCLK_INVERTED), .ena(ENARDEN ^ IS_ENARDEN_INVERTED),
    .rstrama(RSTRAMARSTRAM ^ IS_RSTRAMARSTRAM_INVERTED),
    .rstrega(RSTREGARSTREG ^ IS_RSTREGARSTREG_INVERTED), .regcea(REGCEAREGCE),
    .addra(ADDRARDADDR), .wea({6'b0, WEA}), .dia({48'h0, DIADI}), .dipa({6'b0, DIPADIP}),
    .doa(doa), .dopa(dopa),
    .clkb(CLKBWRCLK ^ IS_CLKBWRCLK_INVERTED), .enb(ENBWREN ^ IS_ENBWREN_INVERTED),
    .rstramb(RSTRAMB ^ IS_RSTRAMB_INVERTED), .rstregb(RSTREGB ^ IS_RSTREGB_INVERTED),
    .regceb(REGCEB), .addrb(ADDRBWRADDR), .web(WIDE_W ? {4'b0, WEBWE} : {6'b0, WEBWE[1:0]}),
    .dib(WIDE_W ? {32'h0, DIBDI, DIADI} : {48'h0, DIBDI}),
    .dipb(WIDE_W ? {4'h0, DIPBDIP, DIPADIP} : {6'b0, DIPBDIP}), .dob(dob), .dopb(dopb));
endmodule

module RAMB36E1 #(parameter [35:0] INIT_A = 36'h0, parameter [35:0] INIT_B = 36'h0,
    parameter [35:0] SRVAL_A = 36'h0, parameter [35:0] SRVAL_B = 36'h0,
    parameter RAM_MODE = "TDP", parameter integer READ_WIDTH_A = 0,
    parameter integer READ_WIDTH_B = 0, parameter integer WRITE_WIDTH_A = 0,
    parameter integer WRITE_WIDTH_B = 0, parameter WRITE_MODE_A = "WRITE_FIRST",
    parameter WRITE_MODE_B = "WRITE_FIRST", parameter integer DOA_REG = 0,
    parameter integer DOB_REG = 0, parameter RSTREG_PRIORITY_A = "RSTREG",
    parameter RSTREG_PRIORITY_B = "RSTREG", parameter RDADDR_COLLISION_HWCONFIG = "DELAYED_WRITE",
    parameter SIM_COLLISION_CHECK = "ALL", parameter SIM_DEVICE = "7SERIES",
    parameter INIT_FILE = "NONE", parameter EN_ECC_READ = "FALSE", parameter EN_ECC_WRITE = "FALSE",
    parameter RAM_EXTENSION_A = "NONE", parameter RAM_EXTENSION_B = "NONE",
    parameter [0:0] IS_CLKARDCLK_INVERTED = 1'b0, parameter [0:0] IS_CLKBWRCLK_INVERTED = 1'b0,
    parameter [0:0] IS_ENARDEN_INVERTED = 1'b0, parameter [0:0] IS_ENBWREN_INVERTED = 1'b0,
    parameter [0:0] IS_RSTRAMARSTRAM_INVERTED = 1'b0, parameter [0:0] IS_RSTRAMB_INVERTED = 1'b0,
    parameter [0:0] IS_RSTREGARSTREG_INVERTED = 1'b0, parameter [0:0] IS_RSTREGB_INVERTED = 1'b0,
    parameter [255:0] INIT_00 = 256'h0, INIT_01 = 256'h0, INIT_02 = 256'h0, INIT_03 = 256'h0,
      INIT_04 = 256'h0, INIT_05 = 256'h0, INIT_06 = 256'h0, INIT_07 = 256'h0, INIT_08 = 256'h0,
      INIT_09 = 256'h0, INIT_0A = 256'h0, INIT_0B = 256'h0, INIT_0C = 256'h0, INIT_0D = 256'h0,
      INIT_0E = 256'h0, INIT_0F = 256'h0, INIT_10 = 256'h0, INIT_11 = 256'h0, INIT_12 = 256'h0,
      INIT_13 = 256'h0, INIT_14 = 256'h0, INIT_15 = 256'h0, INIT_16 = 256'h0, INIT_17 = 256'h0,
      INIT_18 = 256'h0, INIT_19 = 256'h0, INIT_1A = 256'h0, INIT_1B = 256'h0, INIT_1C = 256'h0,
      INIT_1D = 256'h0, INIT_1E = 256'h0, INIT_1F = 256'h0, INIT_20 = 256'h0, INIT_21 = 256'h0,
      INIT_22 = 256'h0, INIT_23 = 256'h0, INIT_24 = 256'h0, INIT_25 = 256'h0, INIT_26 = 256'h0,
      INIT_27 = 256'h0, INIT_28 = 256'h0, INIT_29 = 256'h0, INIT_2A = 256'h0, INIT_2B = 256'h0,
      INIT_2C = 256'h0, INIT_2D = 256'h0, INIT_2E = 256'h0, INIT_2F = 256'h0, INIT_30 = 256'h0,
      INIT_31 = 256'h0, INIT_32 = 256'h0, INIT_33 = 256'h0, INIT_34 = 256'h0, INIT_35 = 256'h0,
      INIT_36 = 256'h0, INIT_37 = 256'h0, INIT_38 = 256'h0, INIT_39 = 256'h0, INIT_3A = 256'h0,
      INIT_3B = 256'h0, INIT_3C = 256'h0, INIT_3D = 256'h0, INIT_3E = 256'h0, INIT_3F = 256'h0,
      INIT_40 = 256'h0, INIT_41 = 256'h0, INIT_42 = 256'h0, INIT_43 = 256'h0, INIT_44 = 256'h0,
      INIT_45 = 256'h0, INIT_46 = 256'h0, INIT_47 = 256'h0, INIT_48 = 256'h0, INIT_49 = 256'h0,
      INIT_4A = 256'h0, INIT_4B = 256'h0, INIT_4C = 256'h0, INIT_4D = 256'h0, INIT_4E = 256'h0,
      INIT_4F = 256'h0, INIT_50 = 256'h0, INIT_51 = 256'h0, INIT_52 = 256'h0, INIT_53 = 256'h0,
      INIT_54 = 256'h0, INIT_55 = 256'h0, INIT_56 = 256'h0, INIT_57 = 256'h0, INIT_58 = 256'h0,
      INIT_59 = 256'h0, INIT_5A = 256'h0, INIT_5B = 256'h0, INIT_5C = 256'h0, INIT_5D = 256'h0,
      INIT_5E = 256'h0, INIT_5F = 256'h0, INIT_60 = 256'h0, INIT_61 = 256'h0, INIT_62 = 256'h0,
      INIT_63 = 256'h0, INIT_64 = 256'h0, INIT_65 = 256'h0, INIT_66 = 256'h0, INIT_67 = 256'h0,
      INIT_68 = 256'h0, INIT_69 = 256'h0, INIT_6A = 256'h0, INIT_6B = 256'h0, INIT_6C = 256'h0,
      INIT_6D = 256'h0, INIT_6E = 256'h0, INIT_6F = 256'h0, INIT_70 = 256'h0, INIT_71 = 256'h0,
      INIT_72 = 256'h0, INIT_73 = 256'h0, INIT_74 = 256'h0, INIT_75 = 256'h0, INIT_76 = 256'h0,
      INIT_77 = 256'h0, INIT_78 = 256'h0, INIT_79 = 256'h0, INIT_7A = 256'h0, INIT_7B = 256'h0,
      INIT_7C = 256'h0, INIT_7D = 256'h0, INIT_7E = 256'h0, INIT_7F = 256'h0, INITP_00 = 256'h0,
      INITP_01 = 256'h0, INITP_02 = 256'h0, INITP_03 = 256'h0, INITP_04 = 256'h0, INITP_05 = 256'h0,
      INITP_06 = 256'h0, INITP_07 = 256'h0, INITP_08 = 256'h0, INITP_09 = 256'h0, INITP_0A = 256'h0,
      INITP_0B = 256'h0, INITP_0C = 256'h0, INITP_0D = 256'h0, INITP_0E = 256'h0, INITP_0F = 256'h0)
  (output CASCADEOUTA, output CASCADEOUTB, output DBITERR, output SBITERR,
   output [7:0] ECCPARITY, output [8:0] RDADDRECC,
   output [31:0] DOADO, output [31:0] DOBDO, output [3:0] DOPADOP, output [3:0] DOPBDOP,
   input CASCADEINA, input CASCADEINB, input INJECTDBITERR, input INJECTSBITERR,
   input CLKARDCLK, input CLKBWRCLK, input ENARDEN, input ENBWREN, input REGCEAREGCE,
   input REGCEB, input RSTRAMARSTRAM, input RSTRAMB, input RSTREGARSTREG, input RSTREGB,
   input [15:0] ADDRARDADDR, input [15:0] ADDRBWRADDR, input [31:0] DIADI, input [31:0] DIBDI,
   input [3:0] DIPADIP, input [3:0] DIPBDIP, input [3:0] WEA, input [7:0] WEBWE);
  localparam SDP = RAM_MODE == "SDP";
  // In SDP the A side reads and the B side writes, each as wide as both.
  localparam integer RA = READ_WIDTH_A;
  localparam integer WA = SDP ? 0 : WRITE_WIDTH_A;
  localparam integer RB = SDP ? 0 : READ_WIDTH_B;
  localparam integer WB = WRITE_WIDTH_B;
  localparam WIDE_R = SDP && RA == 72;
  localparam WIDE_W = SDP && WB == 72;
  function [63:0] data_of(input [35:0] v, input integer w);
    data_of = w >= 36 ? v[31:0] : w == 18 ? v[15:0] : w == 9 ? v[7:0] : v;
  endfunction
  function [7:0] par_of(input [35:0] v, input integer w);
    par_of = w >= 36 ? v[35:32] : w == 18 ? v[17:16] : w == 9 ? v[8] : 8'h0;
  endfunction
  localparam [63:0] INIT_DA = WIDE_R ? {INIT_B[31:0], INIT_A[31:0]} : data_of(INIT_A, RA);
  localparam [7:0] INIT_PA = WIDE_R ? {INIT_B[35:32], INIT_A[35:32]} : par_of(INIT_A, RA);
  localparam [63:0] SR_DA = WIDE_R ? {SRVAL_B[31:0], SRVAL_A[31:0]} : data_of(SRVAL_A, RA);
  localparam [7:0] SR_PA = WIDE_R ? {SRVAL_B[35:32], SRVAL_A[35:32]} : par_of(SRVAL_A, RA);
  wire [63:0] doa, dob;
  wire [7:0] dopa, dopb;
  assign DOADO = doa[31:0];
  assign DOPADOP = dopa[3:0];
  assign DOBDO = WIDE_R ? doa[63:32] : dob[31:0];
  assign DOPBDOP = WIDE_R ? dopa[7:4] : dopb[3:0];
  // Not modelled: cascading, and the error correction.
  assign {CASCADEOUTA, CASCADEOUTB, DBITERR, SBITERR} = 4'b0000;
  assign ECCPARITY = 8'h00;
  assign RDADDRECC = 9'h000;
  xc7_sim_bram #(.ABITS(15), .RA(RA), .WA(WA), .RB(RB), .WB(WB), .MODE_A(WRITE_MODE_A),
    .MODE_B(WRITE_MODE_B), .REG_A(DOA_REG), .REG_B(DOB_REG), .PRIO_A(RSTREG_PRIORITY_A),
    .PRIO_B(RSTREG_PRIORITY_B), .INIT_DA(INIT_DA), .INIT_PA(INIT_PA), .SR_DA(SR_DA),
    .SR_PA(SR_PA), .INIT_DB(data_of(INIT_B, RB)), .INIT_PB(par_of(INIT_B, RB)),
    .SR_DB(data_of(SRVAL_B, RB)), .SR_PB(par_of(SRVAL_B, RB)),
    .INIT({INIT_7F, INIT_7E, INIT_7D, INIT_7C, INIT_7B, INIT_7A, INIT_79, INIT_78, INIT_77, INIT_76, INIT_75,
      INIT_74, INIT_73, INIT_72, INIT_71, INIT_70, INIT_6F, INIT_6E, INIT_6D, INIT_6C, INIT_6B,
      INIT_6A, INIT_69, INIT_68, INIT_67, INIT_66, INIT_65, INIT_64, INIT_63, INIT_62, INIT_61,
      INIT_60, INIT_5F, INIT_5E, INIT_5D, INIT_5C, INIT_5B, INIT_5A, INIT_59, INIT_58, INIT_57,
      INIT_56, INIT_55, INIT_54, INIT_53, INIT_52, INIT_51, INIT_50, INIT_4F, INIT_4E, INIT_4D,
      INIT_4C, INIT_4B, INIT_4A, INIT_49, INIT_48, INIT_47, INIT_46, INIT_45, INIT_44, INIT_43,
      INIT_42, INIT_41, INIT_40, INIT_3F, INIT_3E, INIT_3D, INIT_3C, INIT_3B, INIT_3A, INIT_39,
      INIT_38, INIT_37, INIT_36, INIT_35, INIT_34, INIT_33, INIT_32, INIT_31, INIT_30, INIT_2F,
      INIT_2E, INIT_2D, INIT_2C, INIT_2B, INIT_2A, INIT_29, INIT_28, INIT_27, INIT_26, INIT_25,
      INIT_24, INIT_23, INIT_22, INIT_21, INIT_20, INIT_1F, INIT_1E, INIT_1D, INIT_1C, INIT_1B,
      INIT_1A, INIT_19, INIT_18, INIT_17, INIT_16, INIT_15, INIT_14, INIT_13, INIT_12, INIT_11,
      INIT_10, INIT_0F, INIT_0E, INIT_0D, INIT_0C, INIT_0B, INIT_0A, INIT_09, INIT_08, INIT_07,
      INIT_06, INIT_05, INIT_04, INIT_03, INIT_02, INIT_01, INIT_00}),
    .INITP({INITP_0F, INITP_0E, INITP_0D, INITP_0C, INITP_0B, INITP_0A, INITP_09, INITP_08, INITP_07, INITP_06,
      INITP_05, INITP_04, INITP_03, INITP_02, INITP_01, INITP_00}))
    core (.clka(CLKARDCLK ^ IS_CLKARDCLK_INVERTED), .ena(ENARDEN ^ IS_ENARDEN_INVERTED),
    .rstrama(RSTRAMARSTRAM ^ IS_RSTRAMARSTRAM_INVERTED),
    .rstrega(RSTREGARSTREG ^ IS_RSTREGARSTREG_INVERTED), .regcea(REGCEAREGCE),
    .addra(ADDRARDADDR[14:0]), .wea({4'b0, WEA}), .dia({32'h0, DIADI}), .dipa({4'b0, DIPADIP}),
    .doa(doa), .dopa(dopa),
    .clkb(CLKBWRCLK ^ IS_CLKBWRCLK_INVERTED), .enb(ENBWREN ^ IS_ENBWREN_INVERTED),
    .rstramb(RSTRAMB ^ IS_RSTRAMB_INVERTED), .rstregb(RSTREGB ^ IS_RSTREGB_INVERTED),
    .regceb(REGCEB), .addrb(ADDRBWRADDR[14:0]), .web(WIDE_W ? WEBWE : {4'b0, WEBWE[3:0]}),
    .dib(WIDE_W ? {DIBDI, DIADI} : {32'h0, DIBDI}),
    .dipb(WIDE_W ? {DIPBDIP, DIPADIP} : {4'h0, DIPBDIP}), .dob(dob), .dopb(dopb));
endmodule

// ---------------------------------------------------------------- FIFOs

// What FIFO18E1 and FIFO36E1 share, on one clock (EN_SYN TRUE). On a rising
// edge a write is taken unless the FIFO is full and a read unless it is
// empty, and the other one is an error (WRERR, RDERR) for that cycle. FULL is
// a full FIFO, EMPTY an empty one, ALMOSTFULL at most ALMOST_FULL_OFFSET
// words short of full, ALMOSTEMPTY at most ALMOST_EMPTY_OFFSET words. A
// count is its pointer with the bits above the pointer's width set, and zero
// from a reset until the pointer first moves: xsim's answers.
//
// Two clocks (EN_SYN FALSE, the default) are not modelled: the flags cross
// between them through xsim's synchronisers, cycle by cycle, which nothing
// here has measured well enough to write down. A design asking for them is
// refused at elaboration, by an instance of a module nothing declares that
// names the reason, rather than given one clock's answers.
module xc7_sim_fifo #(parameter integer ABITS = 10, parameter integer CBITS = 12,
    parameter integer DW = 16, parameter integer PW = 2, parameter integer DO_REG = 0,
    parameter integer AE = 1, parameter integer AF = 1,
    parameter [63:0] INIT_D = 64'h0, parameter [7:0] INIT_P = 8'h0,
    parameter [63:0] SR_D = 64'h0, parameter [7:0] SR_P = 8'h0)
  (input clk, input rst, input wren, input rden, input regce, input rstreg,
   input [63:0] di, input [7:0] dip, output [63:0] do_, output [7:0] dop,
   output empty, output full, output almost_empty, output almost_full, output rderr,
   output wrerr, output [CBITS-1:0] rdcount, output [CBITS-1:0] wrcount);
  localparam integer DEPTH = 1 << ABITS;
  localparam [CBITS-1:0] HIGH = ({CBITS{1'b1}} >> ABITS) << ABITS;
  localparam [71:0] DMASK = (72'h1 << DW) - 1;
  localparam [71:0] PMASK = (72'h1 << PW) - 1;
  reg [71:0] mem [0:DEPTH-1];
  reg [ABITS-1:0] rp = 0, wp = 0;
  integer used = 0;
  // Only the bits of the width are known.
  function [63:0] known_d(input [63:0] v);
    integer j;
    begin
      known_d = v;
      for (j = 0; j < 64; j = j + 1) if (j >= DW) known_d[j] = 1'bx;
    end
  endfunction
  function [7:0] known_p(input [7:0] v);
    integer j;
    begin
      known_p = v;
      for (j = 0; j < 8; j = j + 1) if (j >= PW) known_p[j] = 1'bx;
    end
  endfunction
  reg [63:0] latch_d = known_d(INIT_D), reg_d = known_d(INIT_D);
  reg [7:0] latch_p = known_p(INIT_P), reg_p = known_p(INIT_P);
  assign do_ = DO_REG ? reg_d : latch_d;
  assign dop = DO_REG ? reg_p : latch_p;
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg em = 1'b1, fu = 1'b0, ae = 1'b1, af = 1'b0, re = 1'b0, we = 1'b0;
  reg [CBITS-1:0] rc = 0, wc = 0;
  assign {empty, full, almost_empty, almost_full, rderr, wrerr} = {em, fu, ae, af, re, we};
  assign rdcount = rc;
  assign wrcount = wc;
  reg take_w, take_r;
  always @(posedge clk or posedge gsr)
    if (gsr || rst) begin
      rp <= 0;
      wp <= 0;
      used = 0;
      {em, fu, ae, af, re, we} <= 6'b101000;
      rc <= 0;
      wc <= 0;
    end else begin
      take_w = wren && !fu;
      take_r = rden && !em;
      we <= wren && fu;
      re <= rden && em;
      if (take_w) begin
        mem[wp] = ((di & DMASK) << PW) | (dip & PMASK);
        wp <= wp + 1'b1;
        wc <= HIGH | (wp + 1'b1);
      end
      if (take_r) begin
        {latch_d, latch_p} <= {known_d(mem[rp] >> PW), known_p(mem[rp])};
        rp <= rp + 1'b1;
        rc <= HIGH | (rp + 1'b1);
      end
      if (DO_REG && regce) {reg_d, reg_p} <= rstreg ? {known_d(SR_D), known_p(SR_P)} :
        {latch_d, latch_p};
      used = used + take_w - take_r;
      em <= used == 0;
      fu <= used == DEPTH;
      ae <= used <= AE;
      af <= used >= DEPTH - AF;
    end
endmodule

module FIFO18E1 #(parameter [12:0] ALMOST_EMPTY_OFFSET = 13'h0080,
    parameter [12:0] ALMOST_FULL_OFFSET = 13'h0080, parameter integer DATA_WIDTH = 4,
    parameter integer DO_REG = 1, parameter EN_SYN = "FALSE", parameter FIFO_MODE = "FIFO18",
    parameter FIRST_WORD_FALL_THROUGH = "FALSE", parameter [35:0] INIT = 36'h0,
    parameter SIM_DEVICE = "7SERIES", parameter [35:0] SRVAL = 36'h0,
    parameter [0:0] IS_RDCLK_INVERTED = 1'b0, parameter [0:0] IS_RDEN_INVERTED = 1'b0,
    parameter [0:0] IS_RSTREG_INVERTED = 1'b0, parameter [0:0] IS_RST_INVERTED = 1'b0,
    parameter [0:0] IS_WRCLK_INVERTED = 1'b0, parameter [0:0] IS_WREN_INVERTED = 1'b0)
  (output ALMOSTEMPTY, output ALMOSTFULL, output [31:0] DO, output [3:0] DOP, output EMPTY,
   output FULL, output [11:0] RDCOUNT, output RDERR, output [11:0] WRCOUNT, output WRERR,
   input [31:0] DI, input [3:0] DIP, input RDCLK, input RDEN, input REGCE, input RST,
   input RSTREG, input WRCLK, input WREN);
  localparam integer W = DATA_WIDTH;
  localparam integer DW = W == 4 ? 4 : W == 9 ? 8 : W == 18 ? 16 : 32;
  localparam integer PW = W == 4 ? 0 : W == 9 ? 1 : W == 18 ? 2 : 4;
  localparam integer ABITS = W == 4 ? 12 : W == 9 ? 11 : W == 18 ? 10 : 9;
  if (EN_SYN != "TRUE") begin : two_clocks
    xc7_sim_fifo_with_EN_SYN_FALSE_is_not_modelled refused ();
  end
  wire [63:0] do_;
  wire [7:0] dop;
  assign DO = do_[31:0];
  assign DOP = dop[3:0];
  xc7_sim_fifo #(.ABITS(ABITS), .CBITS(12), .DW(DW), .PW(PW), .DO_REG(DO_REG),
    .AE(ALMOST_EMPTY_OFFSET), .AF(ALMOST_FULL_OFFSET),
    .INIT_D(INIT[31:0] & ((64'h1 << DW) - 1)), .INIT_P(INIT >> DW),
    .SR_D(SRVAL[31:0] & ((64'h1 << DW) - 1)), .SR_P(SRVAL >> DW))
    core (.clk(WRCLK ^ IS_WRCLK_INVERTED), .rst(RST ^ IS_RST_INVERTED),
    .wren(WREN ^ IS_WREN_INVERTED), .rden(RDEN ^ IS_RDEN_INVERTED), .regce(REGCE),
    .rstreg(RSTREG ^ IS_RSTREG_INVERTED), .di({32'h0, DI}), .dip({4'h0, DIP}), .do_(do_),
    .dop(dop), .empty(EMPTY), .full(FULL), .almost_empty(ALMOSTEMPTY),
    .almost_full(ALMOSTFULL), .rderr(RDERR), .wrerr(WRERR), .rdcount(RDCOUNT),
    .wrcount(WRCOUNT));
endmodule

module FIFO36E1 #(parameter [12:0] ALMOST_EMPTY_OFFSET = 13'h0080,
    parameter [12:0] ALMOST_FULL_OFFSET = 13'h0080, parameter integer DATA_WIDTH = 4,
    parameter integer DO_REG = 1, parameter EN_ECC_READ = "FALSE",
    parameter EN_ECC_WRITE = "FALSE", parameter EN_SYN = "FALSE", parameter FIFO_MODE = "FIFO36",
    parameter FIRST_WORD_FALL_THROUGH = "FALSE", parameter [71:0] INIT = 72'h0,
    parameter SIM_DEVICE = "7SERIES", parameter [71:0] SRVAL = 72'h0,
    parameter [0:0] IS_RDCLK_INVERTED = 1'b0, parameter [0:0] IS_RDEN_INVERTED = 1'b0,
    parameter [0:0] IS_RSTREG_INVERTED = 1'b0, parameter [0:0] IS_RST_INVERTED = 1'b0,
    parameter [0:0] IS_WRCLK_INVERTED = 1'b0, parameter [0:0] IS_WREN_INVERTED = 1'b0)
  (output ALMOSTEMPTY, output ALMOSTFULL, output DBITERR, output [63:0] DO, output [7:0] DOP,
   output [7:0] ECCPARITY, output EMPTY, output FULL, output [12:0] RDCOUNT, output RDERR,
   output SBITERR, output [12:0] WRCOUNT, output WRERR, input [63:0] DI, input [7:0] DIP,
   input INJECTDBITERR, input INJECTSBITERR, input RDCLK, input RDEN, input REGCE, input RST,
   input RSTREG, input WRCLK, input WREN);
  localparam integer W = DATA_WIDTH;
  localparam integer DW = W == 4 ? 4 : W == 9 ? 8 : W == 18 ? 16 : W == 36 ? 32 : 64;
  localparam integer PW = W == 4 ? 0 : W == 9 ? 1 : W == 18 ? 2 : W == 36 ? 4 : 8;
  localparam integer ABITS = W == 4 ? 13 : W == 9 ? 12 : W == 18 ? 11 : W == 36 ? 10 : 9;
  if (EN_SYN != "TRUE") begin : two_clocks
    xc7_sim_fifo_with_EN_SYN_FALSE_is_not_modelled refused ();
  end
  assign {DBITERR, SBITERR} = 2'b00;
  assign ECCPARITY = 8'h00;
  xc7_sim_fifo #(.ABITS(ABITS), .CBITS(13), .DW(DW), .PW(PW), .DO_REG(DO_REG),
    .AE(ALMOST_EMPTY_OFFSET), .AF(ALMOST_FULL_OFFSET),
    .INIT_D(INIT[63:0] & ((65'h1 << DW) - 1)), .INIT_P(INIT >> DW),
    .SR_D(SRVAL[63:0] & ((65'h1 << DW) - 1)), .SR_P(SRVAL >> DW))
    core (.clk(WRCLK ^ IS_WRCLK_INVERTED), .rst(RST ^ IS_RST_INVERTED),
    .wren(WREN ^ IS_WREN_INVERTED), .rden(RDEN ^ IS_RDEN_INVERTED), .regce(REGCE),
    .rstreg(RSTREG ^ IS_RSTREG_INVERTED), .di(DI), .dip(DIP), .do_(DO), .dop(DOP),
    .empty(EMPTY), .full(FULL), .almost_empty(ALMOSTEMPTY), .almost_full(ALMOSTFULL),
    .rderr(RDERR), .wrerr(WRERR), .rdcount(RDCOUNT), .wrcount(WRCOUNT));
endmodule

// ---------------------------------------------------------------- configuration

// STARTUPE2 as xsim has it: EOS rises as the global reset ends, CFGMCLK runs
// at 65 MHz from time zero (low first), CFGCLK floats and PREQ is low. Not
// modelled: its GSR and GTS inputs, which in the part reset and tristate the
// whole device, and the user clock and DONE pins.
module STARTUPE2 #(parameter PROG_USR = "FALSE", parameter real SIM_CCLK_FREQ = 0.0)
  (output CFGCLK, output CFGMCLK, output EOS, output PREQ, input CLK, input GSR, input GTS,
   input KEYCLEARB, input PACK, input USRCCLKO, input USRCCLKTS, input USRDONEO,
   input USRDONETS);
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg mclk = 1'b0;
  always #7692 mclk = ~mclk;
  assign CFGMCLK = mclk;
  assign EOS = ~gsr;
  assign CFGCLK = 1'bz;
  assign PREQ = 1'b0;
endmodule

// ---------------------------------------------------------------- clocking

module BUFG (output O, input I);
  assign O = I;
endmodule

module IBUFG #(parameter CAPACITANCE = "DONT_CARE", parameter IBUF_DELAY_VALUE = "0",
    parameter IBUF_LOW_PWR = "TRUE", parameter IOSTANDARD = "DEFAULT") (output O, input I);
  assign O = I;
endmodule

// Two inputs and a glitch-free switch between them (UG472, "BUFGCTRL").
// An input is *chosen* by a register its own clock loads on the edge that
// takes it to the parked level (falling, for INIT_OUT 0), from its select
// and from the other input not being both chosen and enabled; IGNORE makes
// that register follow at once. Its enable passes through a latch open
// while that clock is parked, so CE stops a clock without waiting for an
// edge. The output is the chosen and enabled input, and INIT_OUT when
// there is none.
module BUFGCTRL #(parameter [0:0] INIT_OUT = 1'b0, parameter PRESELECT_I0 = "FALSE",
    parameter PRESELECT_I1 = "FALSE", parameter SIM_DEVICE = "7SERIES",
    parameter [0:0] IS_CE0_INVERTED = 1'b0, parameter [0:0] IS_CE1_INVERTED = 1'b0,
    parameter [0:0] IS_I0_INVERTED = 1'b0, parameter [0:0] IS_I1_INVERTED = 1'b0,
    parameter [0:0] IS_IGNORE0_INVERTED = 1'b0, parameter [0:0] IS_IGNORE1_INVERTED = 1'b0,
    parameter [0:0] IS_S0_INVERTED = 1'b0, parameter [0:0] IS_S1_INVERTED = 1'b0)
  (output O, input CE0, input CE1, input I0, input I1, input IGNORE0, input IGNORE1,
   input S0, input S1);
  wire i0 = I0 ^ IS_I0_INVERTED, i1 = I1 ^ IS_I1_INVERTED;
  wire ce0 = CE0 ^ IS_CE0_INVERTED, ce1 = CE1 ^ IS_CE1_INVERTED;
  wire s0 = S0 ^ IS_S0_INVERTED, s1 = S1 ^ IS_S1_INVERTED;
  wire ig0 = IGNORE0 ^ IS_IGNORE0_INVERTED, ig1 = IGNORE1 ^ IS_IGNORE1_INVERTED;
  // Each clock as seen from the parked level: low means parked.
  wire p0 = i0 ^ INIT_OUT, p1 = i1 ^ INIT_OUT;
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg q0 = PRESELECT_I0 == "TRUE", q1 = PRESELECT_I1 == "TRUE";
  reg g0 = 1'b1, g1 = 1'b1;
  wire d0 = s0 & ~(q1 & g1), d1 = s1 & ~(q0 & g0);
  reg p0_was = 1'bx, p1_was = 1'bx;
  // The global reset holds both enables open.
  always @(p0 or ce0 or gsr) if (gsr) g0 <= 1'b1; else if (!p0) g0 <= ce0;
  always @(p1 or ce1 or gsr) if (gsr) g1 <= 1'b1; else if (!p1) g1 <= ce1;
  always @(p0 or d0 or ig0 or gsr) begin
    if (gsr) q0 <= PRESELECT_I0 == "TRUE";
    else if (ig0 || (p0 === 1'b0 && p0_was === 1'b1)) q0 <= d0;
    p0_was = p0;
  end
  always @(p1 or d1 or ig1 or gsr) begin
    if (gsr) q1 <= PRESELECT_I1 == "TRUE";
    else if (ig1 || (p1 === 1'b0 && p1_was === 1'b1)) q1 <= d1;
    p1_was = p1;
  end
  // Low through the global reset, whatever INIT_OUT says: xsim's answer.
  assign O = gsr ? 1'b0 : INIT_OUT ^ (p0 & q0 & g0 | p1 & q1 & g1);
endmodule

module BUFGCE #(parameter CE_TYPE = "SYNC", parameter [0:0] IS_CE_INVERTED = 1'b0,
    parameter [0:0] IS_I_INVERTED = 1'b0, parameter SIM_DEVICE = "7SERIES")
  (output O, input CE, input I);
  wire o;
  BUFGCTRL #(.PRESELECT_I0("TRUE"), .IS_CE0_INVERTED(IS_CE_INVERTED),
    .IS_I0_INVERTED(IS_I_INVERTED))
    u_buf (.O(o), .CE0(CE), .CE1(1'b0), .I0(I), .I1(1'b1), .IGNORE0(1'b0), .IGNORE1(1'b1),
    .S0(1'b1), .S1(1'b0));
  // An asynchronous enable gates the clock as it stands, glitches and all.
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  assign O = CE_TYPE != "ASYNC" ? o : gsr ? 1'b0 : (I ^ IS_I_INVERTED) & (CE ^ IS_CE_INVERTED);
endmodule

module BUFGCE_1 (output O, input CE, input I);
  BUFGCTRL #(.INIT_OUT(1'b1), .PRESELECT_I0("TRUE"))
    u_buf (.O(O), .CE0(CE), .CE1(1'b0), .I0(I), .I1(1'b1), .IGNORE0(1'b0), .IGNORE1(1'b1),
    .S0(1'b1), .S1(1'b0));
endmodule

// S chooses through the enables, so a switch stops the old clock at once.
module BUFGMUX #(parameter CLK_SEL_TYPE = "SYNC") (output O, input I0, input I1, input S);
  BUFGCTRL #(.PRESELECT_I0("TRUE"))
    u_buf (.O(O), .CE0(~S), .CE1(S), .I0(I0), .I1(I1), .IGNORE0(1'b0), .IGNORE1(1'b0),
    .S0(1'b1), .S1(1'b1));
endmodule

module BUFGMUX_1 #(parameter CLK_SEL_TYPE = "SYNC") (output O, input I0, input I1, input S);
  BUFGCTRL #(.INIT_OUT(1'b1), .PRESELECT_I0("TRUE"))
    u_buf (.O(O), .CE0(~S), .CE1(S), .I0(I0), .I1(I1), .IGNORE0(1'b0), .IGNORE1(1'b0),
    .S0(1'b1), .S1(1'b1));
endmodule

module BUFGMUX_CTRL (output O, input I0, input I1, input S);
  BUFGCTRL #(.PRESELECT_I0("TRUE"))
    u_buf (.O(O), .CE0(1'b1), .CE1(1'b1), .I0(I0), .I1(I1), .IGNORE0(1'b0), .IGNORE1(1'b0),
    .S0(~S), .S1(S));
endmodule

module BUFGP (output O, input I);
  assign O = I;
endmodule

// A clock enable as BUFGCE has it, for the horizontal and multi-region
// buffers. xsim's BUFHCE parks low whatever its INIT_OUT; BUFMRCE honours it.
module xc7_sim_bufce #(parameter CE_TYPE = "SYNC", parameter [0:0] INIT_OUT = 1'b0,
    parameter [0:0] IS_CE_INVERTED = 1'b0) (output O, input CE, input I);
  wire o;
  BUFGCTRL #(.INIT_OUT(INIT_OUT), .PRESELECT_I0("TRUE"), .IS_CE0_INVERTED(IS_CE_INVERTED))
    u_buf (.O(o), .CE0(CE), .CE1(1'b0), .I0(I), .I1(1'b1), .IGNORE0(1'b0), .IGNORE1(1'b1),
    .S0(1'b1), .S1(1'b0));
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  assign O = CE_TYPE != "ASYNC" ? o : gsr ? 1'b0 : I & (CE ^ IS_CE_INVERTED);
endmodule

module BUFHCE #(parameter CE_TYPE = "SYNC", parameter [0:0] INIT_OUT = 1'b0,
    parameter [0:0] IS_CE_INVERTED = 1'b0) (output O, input CE, input I);
  xc7_sim_bufce #(.CE_TYPE(CE_TYPE), .IS_CE_INVERTED(IS_CE_INVERTED)) u_buf (.O(O), .CE(CE), .I(I));
endmodule

module BUFMRCE #(parameter CE_TYPE = "SYNC", parameter [0:0] INIT_OUT = 1'b0,
    parameter [0:0] IS_CE_INVERTED = 1'b0) (output O, input CE, input I);
  xc7_sim_bufce #(.CE_TYPE(CE_TYPE), .INIT_OUT(INIT_OUT), .IS_CE_INVERTED(IS_CE_INVERTED))
    u_buf (.O(O), .CE(CE), .I(I));
endmodule

module BUFH (output O, input I);
  assign O = I;
endmodule

module BUFMR (output O, input I);
  assign O = I;
endmodule

module BUFIO (output O, input I);
  assign O = I;
endmodule

// The regional buffer's divider counts the rising edges of its input with
// CE gating that input as it stands, so an enable mid-pulse is an edge.
// From a reset (CLR, or the global one) the first edge raises the output,
// which then stays high for N/2 edges and low for the rest of N; divided by
// one it is high from a rising edge to the falling one. BYPASS passes the
// input, CE and CLR and the global reset included.
module BUFR #(parameter BUFR_DIVIDE = "BYPASS", parameter SIM_DEVICE = "7SERIES")
  (output O, input CE, input CLR, input I);
  localparam integer N = BUFR_DIVIDE == "1" ? 1 : BUFR_DIVIDE == "2" ? 2 :
    BUFR_DIVIDE == "3" ? 3 : BUFR_DIVIDE == "4" ? 4 : BUFR_DIVIDE == "5" ? 5 :
    BUFR_DIVIDE == "6" ? 6 : BUFR_DIVIDE == "7" ? 7 : BUFR_DIVIDE == "8" ? 8 : 0;
  wire ic = I & CE;
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;
  reg o = 1'b0;
  integer n = 0;
  always @(posedge ic or negedge ic or posedge CLR or posedge gsr)
    if (gsr || CLR) begin
      o <= 1'b0;
      n = 0;
    end else if (ic === 1'b1) begin
      o <= N == 1 || n < N / 2;
      n = N == 0 ? 0 : (n + 1) % N;
    end else if (N == 1) o <= 1'b0;
  assign O = N == 0 ? I : o;
endmodule

// ---------------------------------------------------------------- clock managers

// What PLLE2_ADV and MMCME2_ADV share, as xsim was seen to run them.
//
// The input's period is measured, edge to edge. Counting starts at the first
// rising edge of the input after RST falls, or, when RST is low as the
// global reset ends and was low at the last edge, at that edge. Counting n
// edges from there, with M the multiplier and D the input divider:
//
//   the outputs start at n = M*D + 15 + 2*D (PLL) or M*D + 15 + (D-1) (MMCM),
//   LOCKED rises 100 ps after n = 3*M*D + 17 (PLL) or 3*M*D + 15 (MMCM).
//
// Those are xsim's counts, measured for M from 6 to 32 and D from 1 to 3;
// they are not in any document. From the start each output runs at the VCO
// period (the input's times D/M, kept to the femtosecond) times its divide,
// with its duty cycle in half periods of the VCO, and its phase less the
// feedback's, since the feedback is what lines up with the input. A PLL's
// outputs then run half a VCO period early, all but the first rising edge.
// Two oddities are kept because xsim has them: a PLL puts every output high
// for one input period at n = 1, and an MMCM for one input period from the
// first input edge after RST rises, or after time zero. RST stops every
// output at once and drops LOCKED a nanosecond later. Edges fall on whole
// picoseconds, rounded down.
//
// Not modelled: CLKFBIN (the feedback is taken to be the usual loop from
// CLKFBOUT); a fractional multiplier's lock time and start (taken as for the
// nearest whole one; xsim takes a good deal longer); an MMCM's start with D
// above 1 and M odd; a VCO period that is not a whole number of picoseconds,
// whose edges xsim rounds its own way, a picosecond from these;
// CLKOUT4_CASCADE's phase; CLKINSTOPPED and CLKFBSTOPPED; the dynamic
// reconfiguration port's effect on the outputs (it answers, and holds what is
// written); and the fine phase shift (PSDONE answers twelve PSCLK cycles after
// PSEN, which is a reading of UG472, not a measurement).
module xc7_sim_clock_manager #(parameter integer PLL = 1,
    parameter real MULT = 5.0, parameter integer DIVCLK = 1, parameter real PHASEFB = 0.0,
    parameter real DIV0 = 1.0, parameter real DIV1 = 1.0, parameter real DIV2 = 1.0,
    parameter real DIV3 = 1.0, parameter real DIV4 = 1.0, parameter real DIV5 = 1.0,
    parameter real DIV6 = 1.0,
    parameter real DUTY0 = 0.5, parameter real DUTY1 = 0.5, parameter real DUTY2 = 0.5,
    parameter real DUTY3 = 0.5, parameter real DUTY4 = 0.5, parameter real DUTY5 = 0.5,
    parameter real DUTY6 = 0.5,
    parameter real PHASE0 = 0.0, parameter real PHASE1 = 0.0, parameter real PHASE2 = 0.0,
    parameter real PHASE3 = 0.0, parameter real PHASE4 = 0.0, parameter real PHASE5 = 0.0,
    parameter real PHASE6 = 0.0, parameter integer CASCADE4 = 0)
  (input clkin, input rst, output [6:0] clkout, output clkfbout, output reg locked,
   input dclk, input den, input dwe, input [6:0] daddr, input [15:0] di, output reg [15:0] do_,
   output reg drdy, input psclk, input psen, output reg psdone);
  localparam integer MD = $rtoi(MULT * DIVCLK + 0.5);
  localparam integer N_START = PLL ? MD + 15 + 2 * DIVCLK : MD + 15 + (DIVCLK - 1);
  localparam integer N_LOCK = PLL ? 3 * MD + 17 : 3 * MD + 15;

  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;

  // ---- counting edges ----
  // The outputs are set going LEAD input periods before they start, so that
  // one whose phase is behind the feedback's can start first.
  localparam integer LEAD = DIVCLK + 1;
  // Times are whole picoseconds, in 64 bits: a long simulation passes 2^31.
  reg [63:0] t_last = 0, t_in = 0, t_start = 0;
  reg seen = 1'b0, rst_at_edge = 1'b1;
  integer n = 0;
  reg counting = 1'b0, run = 1'b0, pulse = 1'b0, armed = 1'b1, lock_req = 1'b0;
  reg rst_was = 1'b0, gsr_was = 1'b1, clk_was = 1'bx;
  reg [31:0] epoch = 0;
  real vco = 0.0;
  always @(clkin or rst or gsr) begin
    if (rst && !rst_was) begin
      counting = 1'b0;
      run = 1'b0;
      lock_req = 1'b0;
      pulse = 1'b0;
      armed = !PLL;
    end
    if (!gsr && gsr_was && !rst && seen && !rst_at_edge) begin
      // The global reset ended with RST low, and RST was low at the last
      // edge too: that edge counts as the first.
      counting = 1'b1;
      n = 0;
    end
    if (clkin === 1'b1 && clk_was === 1'b0) begin
      if (seen) t_in = $time - t_last;
      seen = 1'b1;
      rst_at_edge = rst;
      t_last = $time;
      if (!PLL) begin
        if (pulse) pulse = 1'b0;
        else if (armed) begin
          pulse = 1'b1;
          armed = 1'b0;
        end
      end
      if (!rst && !gsr) begin
        if (!counting) begin
          counting = 1'b1;
          n = 0;
        end else begin
          n = n + 1;
          if (PLL) pulse = n == 1;
          if (n == N_START - LEAD) begin
            vco = $rtoi(t_in * 1000.0 * DIVCLK / MULT) / 1000.0;
            t_start = $time + LEAD * t_in;
            epoch = epoch + 1;
            run = 1'b1;
          end
          if (n == N_LOCK) lock_req = 1'b1;
        end
      end
    end
    rst_was = rst;
    gsr_was = gsr;
    clk_was = clkin;
  end
  always begin
    locked = 1'bx;
    #100 locked = 1'b0;
    forever begin
      @(lock_req);
      if (lock_req) begin
        #100;
        if (lock_req) locked = 1'b1;
      end else begin
        #1000 locked = 1'b0;
      end
    end
  end

  // ---- the outputs ----
  function real divide_of(input integer k);
    divide_of = k == 0 ? DIV0 : k == 1 ? DIV1 : k == 2 ? DIV2 : k == 3 ? DIV3 :
      k == 4 ? (CASCADE4 ? DIV4 * DIV6 : DIV4) : k == 5 ? DIV5 : k == 6 ? DIV6 : MULT;
  endfunction
  function real duty_of(input integer k);
    duty_of = k == 0 ? DUTY0 : k == 1 ? DUTY1 : k == 2 ? DUTY2 : k == 3 ? DUTY3 :
      k == 4 ? DUTY4 : k == 5 ? DUTY5 : k == 6 ? DUTY6 : 0.5;
  endfunction
  function real phase_of(input integer k);
    phase_of = k == 0 ? PHASE0 : k == 1 ? PHASE1 : k == 2 ? PHASE2 : k == 3 ? PHASE3 :
      k == 4 ? PHASE4 : k == 5 ? PHASE5 : k == 6 ? PHASE6 : PHASEFB;
  endfunction
  wire [7:0] q;
  genvar g;
  for (g = 0; g < 8; g = g + 1) begin : outs
    reg o = 1'b0;
    assign q[g] = o;
    real per, high, phase, shift;
    integer s, my;
    reg [63:0] t, t_next, dt;
    always begin
      o = 1'b0;
      wait (run);
      my = epoch;
      // The feedback is the input's period times D exactly; an output is the
      // VCO's, which is kept to the femtosecond, times its divide.
      per = g == 7 ? t_in * 1.0 * DIVCLK : vco * divide_of(g);
      // The high time in half periods of the VCO, as the counters have it;
      // a fractional divide is half its period.
      if (divide_of(g) != $rtoi(divide_of(g))) high = per * duty_of(g);
      else high = $rtoi(duty_of(g) * 2.0 * divide_of(g) + 0.5) * vco / 2.0;
      if (high <= 0.0) high = vco / 2.0;
      phase = phase_of(g) / 360.0 * per - PHASEFB / 360.0 * t_in * DIVCLK;
      shift = PLL ? vco / 2.0 : 0.0;
      s = 0;
      while (run && epoch == my) begin
        // Edge s: rising when even, falling when odd, of period s / 2.
        // A real becomes a vector rounded half away from zero, so less a
        // half it is the floor of a positive time.
        t = t_start + phase + (s / 2) * per - (s == 0 ? 0.0 : shift)
          + (s % 2 == 1 ? high : 0.0) - 0.5;
        t_next = t_start + phase + ((s + 1) / 2) * per - shift + (s % 2 == 0 ? high : 0.0) - 0.5;
        if (s % 2 == 0 && t_next <= t) s = s + 2;
        else begin
          // In steps, so that a reset is seen before the next start.
          while ($time < t && run && epoch == my) begin
            dt = t - $time;
            if (dt > 10 * t_in && t_in > 0) dt = 10 * t_in;
            #(dt);
          end
          if (run && epoch == my) o = s % 2 == 0;
          s = s + 1;
        end
      end
    end
  end
  assign clkout = (q[6:0] & {7{run}}) | {7{pulse}};
  assign clkfbout = (q[7] & run) | pulse;

  // ---- the dynamic reconfiguration port and the phase shift ----
  reg [15:0] drp [0:127];
  integer k, ps_wait = 0;
  initial begin
    for (k = 0; k < 128; k = k + 1) drp[k] = 16'h0000;
    do_ = 16'h0000;
    drdy = 1'b0;
    psdone = 1'b0;
  end
  always @(posedge dclk) begin
    drdy <= den;
    if (den) begin
      do_ <= drp[daddr];
      if (dwe) drp[daddr] <= di;
    end
  end
  always @(posedge psclk) begin
    psdone <= 1'b0;
    if (psen && ps_wait == 0) ps_wait = 12;
    else if (ps_wait > 0) begin
      ps_wait = ps_wait - 1;
      if (ps_wait == 0) psdone <= 1'b1;
    end
  end
endmodule

module PLLE2_ADV #(parameter BANDWIDTH = "OPTIMIZED", parameter integer CLKFBOUT_MULT = 5,
    parameter real CLKFBOUT_PHASE = 0.0, parameter real CLKIN1_PERIOD = 0.0,
    parameter real CLKIN2_PERIOD = 0.0, parameter integer DIVCLK_DIVIDE = 1,
    parameter real REF_JITTER1 = 0.01, parameter real REF_JITTER2 = 0.01,
    parameter STARTUP_WAIT = "FALSE", parameter COMPENSATION = "ZHOLD",
    parameter integer CLKOUT0_DIVIDE = 1, parameter integer CLKOUT1_DIVIDE = 1,
    parameter integer CLKOUT2_DIVIDE = 1, parameter integer CLKOUT3_DIVIDE = 1,
    parameter integer CLKOUT4_DIVIDE = 1, parameter integer CLKOUT5_DIVIDE = 1,
    parameter real CLKOUT0_DUTY_CYCLE = 0.5, parameter real CLKOUT1_DUTY_CYCLE = 0.5,
    parameter real CLKOUT2_DUTY_CYCLE = 0.5, parameter real CLKOUT3_DUTY_CYCLE = 0.5,
    parameter real CLKOUT4_DUTY_CYCLE = 0.5, parameter real CLKOUT5_DUTY_CYCLE = 0.5,
    parameter real CLKOUT0_PHASE = 0.0, parameter real CLKOUT1_PHASE = 0.0,
    parameter real CLKOUT2_PHASE = 0.0, parameter real CLKOUT3_PHASE = 0.0,
    parameter real CLKOUT4_PHASE = 0.0, parameter real CLKOUT5_PHASE = 0.0,
    parameter [0:0] IS_CLKINSEL_INVERTED = 1'b0, parameter [0:0] IS_PWRDWN_INVERTED = 1'b0,
    parameter [0:0] IS_RST_INVERTED = 1'b0)
  (output CLKFBOUT, output CLKOUT0, output CLKOUT1, output CLKOUT2, output CLKOUT3,
   output CLKOUT4, output CLKOUT5, output [15:0] DO, output DRDY, output LOCKED,
   input CLKFBIN, input CLKIN1, input CLKIN2, input CLKINSEL, input [6:0] DADDR, input DCLK,
   input DEN, input [15:0] DI, input DWE, input PWRDWN, input RST);
  wire [6:0] o;
  wire sel = CLKINSEL ^ IS_CLKINSEL_INVERTED;
  xc7_sim_clock_manager #(.PLL(1), .MULT(CLKFBOUT_MULT), .DIVCLK(DIVCLK_DIVIDE),
    .PHASEFB(CLKFBOUT_PHASE), .DIV0(CLKOUT0_DIVIDE), .DIV1(CLKOUT1_DIVIDE),
    .DIV2(CLKOUT2_DIVIDE), .DIV3(CLKOUT3_DIVIDE), .DIV4(CLKOUT4_DIVIDE), .DIV5(CLKOUT5_DIVIDE),
    .DUTY0(CLKOUT0_DUTY_CYCLE), .DUTY1(CLKOUT1_DUTY_CYCLE), .DUTY2(CLKOUT2_DUTY_CYCLE),
    .DUTY3(CLKOUT3_DUTY_CYCLE), .DUTY4(CLKOUT4_DUTY_CYCLE), .DUTY5(CLKOUT5_DUTY_CYCLE),
    .PHASE0(CLKOUT0_PHASE), .PHASE1(CLKOUT1_PHASE), .PHASE2(CLKOUT2_PHASE),
    .PHASE3(CLKOUT3_PHASE), .PHASE4(CLKOUT4_PHASE), .PHASE5(CLKOUT5_PHASE))
    core (.clkin(sel ? CLKIN1 : CLKIN2), .rst((RST ^ IS_RST_INVERTED) | (PWRDWN ^ IS_PWRDWN_INVERTED)),
    .clkout(o), .clkfbout(CLKFBOUT), .locked(LOCKED), .dclk(DCLK), .den(DEN), .dwe(DWE),
    .daddr(DADDR), .di(DI), .do_(DO), .drdy(DRDY), .psclk(1'b0), .psen(1'b0), .psdone());
  assign {CLKOUT5, CLKOUT4, CLKOUT3, CLKOUT2, CLKOUT1, CLKOUT0} = o[5:0];
endmodule

module MMCME2_ADV #(parameter BANDWIDTH = "OPTIMIZED", parameter real CLKFBOUT_MULT_F = 5.0,
    parameter real CLKFBOUT_PHASE = 0.0, parameter CLKFBOUT_USE_FINE_PS = "FALSE",
    parameter real CLKIN1_PERIOD = 0.0, parameter real CLKIN2_PERIOD = 0.0,
    parameter integer DIVCLK_DIVIDE = 1, parameter real REF_JITTER1 = 0.01,
    parameter real REF_JITTER2 = 0.01, parameter STARTUP_WAIT = "FALSE",
    parameter COMPENSATION = "ZHOLD", parameter SS_EN = "FALSE", parameter SS_MODE = "CENTER_HIGH",
    parameter integer SS_MOD_PERIOD = 10000, parameter CLKOUT4_CASCADE = "FALSE",
    parameter real CLKOUT0_DIVIDE_F = 1.0, parameter integer CLKOUT1_DIVIDE = 1,
    parameter integer CLKOUT2_DIVIDE = 1, parameter integer CLKOUT3_DIVIDE = 1,
    parameter integer CLKOUT4_DIVIDE = 1, parameter integer CLKOUT5_DIVIDE = 1,
    parameter integer CLKOUT6_DIVIDE = 1,
    parameter real CLKOUT0_DUTY_CYCLE = 0.5, parameter real CLKOUT1_DUTY_CYCLE = 0.5,
    parameter real CLKOUT2_DUTY_CYCLE = 0.5, parameter real CLKOUT3_DUTY_CYCLE = 0.5,
    parameter real CLKOUT4_DUTY_CYCLE = 0.5, parameter real CLKOUT5_DUTY_CYCLE = 0.5,
    parameter real CLKOUT6_DUTY_CYCLE = 0.5,
    parameter real CLKOUT0_PHASE = 0.0, parameter real CLKOUT1_PHASE = 0.0,
    parameter real CLKOUT2_PHASE = 0.0, parameter real CLKOUT3_PHASE = 0.0,
    parameter real CLKOUT4_PHASE = 0.0, parameter real CLKOUT5_PHASE = 0.0,
    parameter real CLKOUT6_PHASE = 0.0,
    parameter CLKOUT0_USE_FINE_PS = "FALSE", parameter CLKOUT1_USE_FINE_PS = "FALSE",
    parameter CLKOUT2_USE_FINE_PS = "FALSE", parameter CLKOUT3_USE_FINE_PS = "FALSE",
    parameter CLKOUT4_USE_FINE_PS = "FALSE", parameter CLKOUT5_USE_FINE_PS = "FALSE",
    parameter CLKOUT6_USE_FINE_PS = "FALSE",
    parameter [0:0] IS_CLKINSEL_INVERTED = 1'b0, parameter [0:0] IS_PSEN_INVERTED = 1'b0,
    parameter [0:0] IS_PSINCDEC_INVERTED = 1'b0, parameter [0:0] IS_PWRDWN_INVERTED = 1'b0,
    parameter [0:0] IS_RST_INVERTED = 1'b0)
  (output CLKFBOUT, output CLKFBOUTB, output CLKFBSTOPPED, output CLKINSTOPPED,
   output CLKOUT0, output CLKOUT0B, output CLKOUT1, output CLKOUT1B, output CLKOUT2,
   output CLKOUT2B, output CLKOUT3, output CLKOUT3B, output CLKOUT4, output CLKOUT5,
   output CLKOUT6, output [15:0] DO, output DRDY, output LOCKED, output PSDONE,
   input CLKFBIN, input CLKIN1, input CLKIN2, input CLKINSEL, input [6:0] DADDR, input DCLK,
   input DEN, input [15:0] DI, input DWE, input PSCLK, input PSEN, input PSINCDEC,
   input PWRDWN, input RST);
  wire [6:0] o;
  wire fb;
  wire sel = CLKINSEL ^ IS_CLKINSEL_INVERTED;
  xc7_sim_clock_manager #(.PLL(0), .MULT(CLKFBOUT_MULT_F), .DIVCLK(DIVCLK_DIVIDE),
    .PHASEFB(CLKFBOUT_PHASE), .DIV0(CLKOUT0_DIVIDE_F), .DIV1(CLKOUT1_DIVIDE),
    .DIV2(CLKOUT2_DIVIDE), .DIV3(CLKOUT3_DIVIDE), .DIV4(CLKOUT4_DIVIDE), .DIV5(CLKOUT5_DIVIDE),
    .DIV6(CLKOUT6_DIVIDE), .DUTY0(CLKOUT0_DUTY_CYCLE), .DUTY1(CLKOUT1_DUTY_CYCLE),
    .DUTY2(CLKOUT2_DUTY_CYCLE), .DUTY3(CLKOUT3_DUTY_CYCLE), .DUTY4(CLKOUT4_DUTY_CYCLE),
    .DUTY5(CLKOUT5_DUTY_CYCLE), .DUTY6(CLKOUT6_DUTY_CYCLE), .PHASE0(CLKOUT0_PHASE),
    .PHASE1(CLKOUT1_PHASE), .PHASE2(CLKOUT2_PHASE), .PHASE3(CLKOUT3_PHASE),
    .PHASE4(CLKOUT4_PHASE), .PHASE5(CLKOUT5_PHASE), .PHASE6(CLKOUT6_PHASE),
    .CASCADE4(CLKOUT4_CASCADE == "TRUE"))
    core (.clkin(sel ? CLKIN1 : CLKIN2), .rst((RST ^ IS_RST_INVERTED) | (PWRDWN ^ IS_PWRDWN_INVERTED)),
    .clkout(o), .clkfbout(fb), .locked(LOCKED), .dclk(DCLK), .den(DEN), .dwe(DWE),
    .daddr(DADDR), .di(DI), .do_(DO), .drdy(DRDY), .psclk(PSCLK), .psen(PSEN ^ IS_PSEN_INVERTED),
    .psdone(PSDONE));
  assign {CLKOUT6, CLKOUT5, CLKOUT4, CLKOUT3, CLKOUT2, CLKOUT1, CLKOUT0} = o;
  assign {CLKOUT3B, CLKOUT2B, CLKOUT1B, CLKOUT0B} = ~o[3:0];
  assign CLKFBOUT = fb;
  assign CLKFBOUTB = ~fb;
  assign {CLKFBSTOPPED, CLKINSTOPPED} = 2'b00;
endmodule

// ---------------------------------------------------------------- DSP

// What a DSP48E1 does. This model was testdata/fpga/vivado/dsp/dsp48e1_model.v,
// for simulating a netlist that holds one, before the library took it in;
// the global reset is what it gained here (testdata/sim/xc7/dsp48e1_global_reset.v).
//
// Written here from the block diagram in UG479 and nothing else; it holds none of
// the vendor's model. tests/fpga_dsp_vivado.rs holds it to what the vendor's model
// answered under xsim, over testdata/fpga/vivado/dsp/vectors/, which is what makes
// it worth believing. Of the first 52 settings 51 answered alike and one did not:
// with X and Y both non-zero the vendor's CARRYOUT[3] and CARRYCASCOUT differ, since
// the adder is carry-save. This file's single carry cannot say that, so that
// setting's vectors no longer ask about the carries, and neither should a design.
//
// It covers what this flow writes and a good deal more: every register, the
// inverters, the pre-adder, the cascades, X, Y and Z, the additions and
// subtractions of ALUMODE, and the pattern detector. It does NOT cover ALUMODE's
// logic functions (ALUMODE[2] set), USE_SIMD other than ONE48, the carry-in
// selections from a multiply's sign (CARRYINSEL 110) or from the block below
// (010, 100), or INMODE choosing a register that AREG or BREG has not got; those
// have no vectors, and a design using them here gets an answer nobody checked.
module DSP48E1 #(
  parameter integer ACASCREG = 1,
  parameter integer ADREG = 1,
  parameter integer ALUMODEREG = 1,
  parameter integer AREG = 1,
  parameter AUTORESET_PATDET = "NO_RESET",
  parameter A_INPUT = "DIRECT",
  parameter integer BCASCREG = 1,
  parameter integer BREG = 1,
  parameter B_INPUT = "DIRECT",
  parameter integer CARRYINREG = 1,
  parameter integer CARRYINSELREG = 1,
  parameter integer CREG = 1,
  parameter integer DREG = 1,
  parameter integer INMODEREG = 1,
  parameter [3:0] IS_ALUMODE_INVERTED = 4'b0000,
  parameter [0:0] IS_CARRYIN_INVERTED = 1'b0,
  parameter [0:0] IS_CLK_INVERTED = 1'b0,
  parameter [4:0] IS_INMODE_INVERTED = 5'b00000,
  parameter [6:0] IS_OPMODE_INVERTED = 7'b0000000,
  parameter [47:0] MASK = 48'h3fffffffffff,
  parameter integer MREG = 1,
  parameter integer OPMODEREG = 1,
  parameter [47:0] PATTERN = 48'h000000000000,
  parameter integer PREG = 1,
  parameter SEL_MASK = "MASK",
  parameter SEL_PATTERN = "PATTERN",
  parameter USE_DPORT = "FALSE",
  parameter USE_MULT = "MULTIPLY",
  parameter USE_PATTERN_DETECT = "NO_PATDET",
  parameter USE_SIMD = "ONE48"
) (
  output wire [29:0] ACOUT,
  output wire [17:0] BCOUT,
  output wire CARRYCASCOUT,
  output wire [3:0] CARRYOUT,
  output wire MULTSIGNOUT,
  output wire OVERFLOW,
  output wire [47:0] P,
  output wire PATTERNBDETECT,
  output wire PATTERNDETECT,
  output wire [47:0] PCOUT,
  output wire UNDERFLOW,
  input wire [29:0] A,
  input wire [29:0] ACIN,
  input wire [3:0] ALUMODE,
  input wire [17:0] B,
  input wire [17:0] BCIN,
  input wire [47:0] C,
  input wire CARRYCASCIN,
  input wire CARRYIN,
  input wire [2:0] CARRYINSEL,
  input wire CEA1,
  input wire CEA2,
  input wire CEAD,
  input wire CEALUMODE,
  input wire CEB1,
  input wire CEB2,
  input wire CEC,
  input wire CECARRYIN,
  input wire CECTRL,
  input wire CED,
  input wire CEINMODE,
  input wire CEM,
  input wire CEP,
  input wire CLK,
  input wire [24:0] D,
  input wire [4:0] INMODE,
  input wire MULTSIGNIN,
  input wire [6:0] OPMODE,
  input wire [47:0] PCIN,
  input wire RSTA,
  input wire RSTALLCARRYIN,
  input wire RSTALUMODE,
  input wire RSTB,
  input wire RSTC,
  input wire RSTCTRL,
  input wire RSTD,
  input wire RSTINMODE,
  input wire RSTM,
  input wire RSTP
);
  wire clk = CLK ^ IS_CLK_INVERTED;
  // The global reset holds every register at zero and ignores the clock.
  reg gsr = 1'b1;
  initial #100000 gsr <= 1'b0;

  // The A pipeline.
  wire [29:0] a_in = (A_INPUT == "CASCADE") ? ACIN : A;
  reg [29:0] a1_q = 0;
  reg [29:0] a2_q = 0;
  always @(posedge clk) if (!gsr) begin
    if (RSTA) a1_q <= 0;
    else if (CEA1) a1_q <= a_in;
    if (RSTA) a2_q <= 0;
    else if (CEA2) a2_q <= (AREG == 2) ? a1_q : a_in;
  end
  wire [29:0] a_o = (AREG == 0) ? a_in : a2_q;
  assign ACOUT = (AREG == 2 && ACASCREG == 1) ? a1_q : a_o;

  // The B pipeline.
  wire [17:0] b_in = (B_INPUT == "CASCADE") ? BCIN : B;
  reg [17:0] b1_q = 0;
  reg [17:0] b2_q = 0;
  always @(posedge clk) if (!gsr) begin
    if (RSTB) b1_q <= 0;
    else if (CEB1) b1_q <= b_in;
    if (RSTB) b2_q <= 0;
    else if (CEB2) b2_q <= (BREG == 2) ? b1_q : b_in;
  end
  wire [17:0] b_o = (BREG == 0) ? b_in : b2_q;
  assign BCOUT = (BREG == 2 && BCASCREG == 1) ? b1_q : b_o;

  // The control registers.
  reg [4:0] inmode_q = 0;
  reg [6:0] opmode_q = 0;
  reg [3:0] alumode_q = 0;
  reg [2:0] carryinsel_q = 0;
  reg carryin_q = 0;
  wire [4:0] inmode_d = INMODE ^ IS_INMODE_INVERTED;
  wire [6:0] opmode_d = OPMODE ^ IS_OPMODE_INVERTED;
  wire [3:0] alumode_d = ALUMODE ^ IS_ALUMODE_INVERTED;
  wire carryin_d = CARRYIN ^ IS_CARRYIN_INVERTED;
  always @(posedge clk) if (!gsr) begin
    if (RSTINMODE) inmode_q <= 0;
    else if (CEINMODE) inmode_q <= inmode_d;
    if (RSTCTRL) opmode_q <= 0;
    else if (CECTRL) opmode_q <= opmode_d;
    if (RSTCTRL) carryinsel_q <= 0;
    else if (CECTRL) carryinsel_q <= CARRYINSEL;
    if (RSTALUMODE) alumode_q <= 0;
    else if (CEALUMODE) alumode_q <= alumode_d;
    if (RSTALLCARRYIN) carryin_q <= 0;
    else if (CECARRYIN) carryin_q <= carryin_d;
  end
  wire [4:0] inmode = (INMODEREG == 1) ? inmode_q : inmode_d;
  wire [6:0] opmode = (OPMODEREG == 1) ? opmode_q : opmode_d;
  wire [3:0] alumode = (ALUMODEREG == 1) ? alumode_q : alumode_d;
  wire [2:0] carryinsel = (CARRYINSELREG == 1) ? carryinsel_q : CARRYINSEL;
  wire carryin = (CARRYINREG == 1) ? carryin_q : carryin_d;

  // The pre-adder and the multiplier's operands.
  wire [24:0] a_pick = inmode[1] ? 25'd0 : (inmode[0] ? a1_q[24:0] : a_o[24:0]);
  reg [24:0] d_q = 0;
  reg [24:0] ad_q = 0;
  wire [24:0] d_o = (DREG == 1) ? d_q : D;
  wire [24:0] d_pick = inmode[2] ? d_o : 25'd0;
  wire [24:0] a_neg = inmode[3] ? (25'd0 - a_pick) : a_pick;
  wire [24:0] ad_d = d_pick + a_neg;
  always @(posedge clk) if (!gsr) begin
    if (RSTD) d_q <= 0;
    else if (CED) d_q <= D;
    if (RSTD) ad_q <= 0;
    else if (CEAD) ad_q <= ad_d;
  end
  wire [24:0] ad_o = (ADREG == 1) ? ad_q : ad_d;
  wire [24:0] mult_a = (USE_DPORT == "TRUE") ? ad_o : a_pick;
  wire [17:0] mult_b = inmode[4] ? b1_q : b_o;

  // The multiplier, as one 43-bit product. The silicon has two partial
  // products that X and Y select together.
  wire signed [47:0] mult_a_wide = {{23{mult_a[24]}}, mult_a};
  wire signed [47:0] mult_b_wide = {{30{mult_b[17]}}, mult_b};
  wire [47:0] product = mult_a_wide * mult_b_wide;
  reg [47:0] m_q = 0;
  always @(posedge clk) if (!gsr) begin
    if (RSTM) m_q <= 0;
    else if (CEM) m_q <= product;
  end
  wire [47:0] m_o = (MREG == 1) ? m_q : product;

  // C.
  reg [47:0] c_q = 0;
  always @(posedge clk) if (!gsr) begin
    if (RSTC) c_q <= 0;
    else if (CEC) c_q <= C;
  end
  wire [47:0] c_o = (CREG == 1) ? c_q : C;

  // X, Y and Z.
  reg [47:0] p_q = 0;
  wire [47:0] pcin_s17 = {{17{PCIN[47]}}, PCIN[47:17]};
  wire [47:0] p_s17 = {{17{p_q[47]}}, p_q[47:17]};
  wire [47:0] x_mux = (opmode[1:0] == 2'b00) ? 48'd0 :
                      (opmode[1:0] == 2'b01) ? m_o :
                      (opmode[1:0] == 2'b10) ? p_q : {a_o, b_o};
  wire [47:0] y_mux = (opmode[3:2] == 2'b00) ? 48'd0 :
                      (opmode[3:2] == 2'b01) ? 48'd0 :
                      (opmode[3:2] == 2'b10) ? {48{1'b1}} : c_o;
  wire [47:0] z_mux = (opmode[6:4] == 3'b000) ? 48'd0 :
                      (opmode[6:4] == 3'b001) ? PCIN :
                      (opmode[6:4] == 3'b010) ? p_q :
                      (opmode[6:4] == 3'b011) ? c_o :
                      (opmode[6:4] == 3'b100) ? p_q :
                      (opmode[6:4] == 3'b101) ? pcin_s17 :
                      (opmode[6:4] == 3'b110) ? p_s17 : 48'd0;

  // The carry in.
  wire cin = (carryinsel == 3'b000) ? carryin :
             (carryinsel == 3'b001) ? ~PCIN[47] :
             (carryinsel == 3'b011) ? PCIN[47] :
             (carryinsel == 3'b101) ? ~p_q[47] :
             (carryinsel == 3'b111) ? p_q[47] : 1'b0;

  // The adder. Subtraction is the adder with Z inverted around it, which is
  // what UG479's ALUMODE table says each of the four comes to.
  wire [47:0] xy = x_mux + y_mux;
  wire [48:0] added = {1'b0, z_mux} + {1'b0, xy} + {48'd0, cin};
  wire [48:0] alu_full =
      (alumode[1:0] == 2'b00) ? added :
      (alumode[1:0] == 2'b11) ? ({1'b0, z_mux} - {1'b0, xy} - {48'd0, cin}) :
      (alumode[1:0] == 2'b01) ? ({1'b0, xy} + {48'd0, cin} - {1'b0, z_mux} - 49'd1) :
                                (49'd0 - added - 49'd1);
  wire [47:0] alu = alu_full[47:0];

  // The pattern detector reads the adder's output and registers its answer
  // beside P.
  wire [47:0] pattern = (SEL_PATTERN == "C") ? c_o : PATTERN;
  wire [47:0] mask_in = (SEL_MASK == "C") ? c_o :
                        (SEL_MASK == "ROUNDING_MODE1") ? ~(c_o << 1) :
                        (SEL_MASK == "ROUNDING_MODE2") ? ~(c_o << 2) : MASK;
  wire detect_d = &((~(alu ^ pattern)) | mask_in);
  wire detect_b_d = &((alu ^ pattern) | mask_in);
  reg detect_q = 0;
  reg detect_b_q = 0;
  reg carry_q = 0;
  wire auto_reset = (AUTORESET_PATDET == "RESET_MATCH") ? detect_q :
                    (AUTORESET_PATDET == "RESET_NOT_MATCH") ? (detect_past & ~detect_q) : 1'b0;
  reg detect_past = 0;
  always @(posedge clk) if (!gsr) begin
    if (RSTP || auto_reset) begin
      p_q <= 0;
      detect_q <= 0;
      detect_b_q <= 0;
      carry_q <= 0;
    end else if (CEP) begin
      p_q <= alu;
      detect_q <= detect_d;
      detect_b_q <= detect_b_d;
      carry_q <= alu_full[48];
    end
    if (RSTP) detect_past <= 0;
    else if (CEP) detect_past <= detect_q;
  end
  assign P = (PREG == 1) ? p_q : alu;
  assign PCOUT = P;
  assign PATTERNDETECT = (USE_PATTERN_DETECT == "NO_PATDET") ? 1'bx :
                         (PREG == 1) ? detect_q : detect_d;
  assign PATTERNBDETECT = (USE_PATTERN_DETECT == "NO_PATDET") ? 1'bx :
                          (PREG == 1) ? detect_b_q : detect_b_d;
  wire carry_out = (PREG == 1) ? carry_q : alu_full[48];
  assign CARRYOUT = {carry_out, 3'b000};
  assign CARRYCASCOUT = carry_out;
  assign MULTSIGNOUT = m_o[42];
  assign OVERFLOW = 1'b0;
  assign UNDERFLOW = 1'b0;
endmodule
