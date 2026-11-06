`timescale 1ns / 1ps
// STARTUPE2: EOS, CFGCLK and PREQ at each change after time zero, and the
// first rising edges of CFGMCLK, in picoseconds.
module tb;
  wire eos, cfgclk, cfgmclk, preq;
  STARTUPE2 u (.CFGCLK(cfgclk), .CFGMCLK(cfgmclk), .EOS(eos), .PREQ(preq), .CLK(1'b0),
    .GSR(1'b0), .GTS(1'b0), .KEYCLEARB(1'b1), .PACK(1'b0), .USRCCLKO(1'b0), .USRCCLKTS(1'b0),
    .USRDONEO(1'b1), .USRDONETS(1'b1));
  integer n = 0;
  always @(eos or cfgclk or preq) if ($realtime > 0.0) $display("%0d %b %b %b", $rtoi($realtime * 1000.0 + 0.5), eos, cfgclk, preq);
  always @(posedge cfgmclk) begin
    n = n + 1;
    if (n < 6) $display("mclk %0d", $rtoi($realtime * 1000.0 + 0.5));
  end
  initial #400 $finish;
endmodule
