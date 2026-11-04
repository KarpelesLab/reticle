// What a DSP48E1 does, for simulating a netlist that holds one.
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

  // The A pipeline.
  wire [29:0] a_in = (A_INPUT == "CASCADE") ? ACIN : A;
  reg [29:0] a1_q = 0;
  reg [29:0] a2_q = 0;
  always @(posedge clk) begin
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
  always @(posedge clk) begin
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
  always @(posedge clk) begin
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
  always @(posedge clk) begin
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
  always @(posedge clk) begin
    if (RSTM) m_q <= 0;
    else if (CEM) m_q <= product;
  end
  wire [47:0] m_o = (MREG == 1) ? m_q : product;

  // C.
  reg [47:0] c_q = 0;
  always @(posedge clk) begin
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
  always @(posedge clk) begin
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
