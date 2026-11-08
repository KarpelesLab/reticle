`timescale 1ns / 1ps
// DSP48E1 as Vivado 2026.1 placed `vh_pipeline`'s `(a + b) * a` in the
// language corpus: the pre-adder on (`USE_DPORT`, `INMODE = 5'b00100`, so
// `(D + A2) * B2`), `A` and `D` and the pre-adder registered once, `B`
// twice, no multiplier register, `P` registered, `OPMODE = 7'b0000101`, and
// every clock enable and constant input tied as Vivado tied them. The
// operands move every cycle from time zero, so the first 100 ns are under
// the global reset, and `P` is printed after each edge.
module tb;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg [31:0] lfsr = 32'h1234_5679;
  wire [7:0] a = lfsr[7:0], b = lfsr[15:8];
  wire [47:0] p;
  DSP48E1 #(.ACASCREG(1), .ADREG(1), .ALUMODEREG(0), .AREG(1), .AUTORESET_PATDET("NO_RESET"),
    .A_INPUT("DIRECT"), .BCASCREG(2), .BREG(2), .B_INPUT("DIRECT"), .CARRYINREG(0),
    .CARRYINSELREG(0), .CREG(0), .DREG(1), .INMODEREG(0), .MASK(48'h3FFFFFFFFFFF), .MREG(0),
    .OPMODEREG(0), .PATTERN(48'h000000000000), .PREG(1), .SEL_MASK("MASK"),
    .SEL_PATTERN("PATTERN"), .USE_DPORT("TRUE"), .USE_MULT("MULTIPLY"),
    .USE_PATTERN_DETECT("NO_PATDET"), .USE_SIMD("ONE48"))
    u (.A({22'b0, b}), .ACIN(30'b0), .ALUMODE(4'b0000), .B({10'b0, a}), .BCIN(18'b0),
    .C({48{1'b1}}), .CARRYCASCIN(1'b0), .CARRYIN(1'b0), .CARRYINSEL(3'b000), .CEA1(1'b0),
    .CEA2(1'b1), .CEAD(1'b1), .CEALUMODE(1'b0), .CEB1(1'b1), .CEB2(1'b1), .CEC(1'b0),
    .CECARRYIN(1'b0), .CECTRL(1'b0), .CED(1'b1), .CEINMODE(1'b0), .CEM(1'b0), .CEP(1'b1),
    .CLK(clk), .D({17'b0, a}), .INMODE(5'b00100), .MULTSIGNIN(1'b0), .OPMODE(7'b0000101),
    .P(p), .PCIN(48'b0), .RSTA(1'b0), .RSTALLCARRYIN(1'b0), .RSTALUMODE(1'b0), .RSTB(1'b0),
    .RSTC(1'b0), .RSTCTRL(1'b0), .RSTD(1'b0), .RSTINMODE(1'b0), .RSTM(1'b0), .RSTP(1'b0));
  integer i;
  initial begin
    for (i = 0; i < 24; i = i + 1) begin
      @(posedge clk); #1;
      $display("%0d a=%h b=%h p=%h", i, a, b, p);
      lfsr = {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
    end
    $finish;
  end
endmodule
