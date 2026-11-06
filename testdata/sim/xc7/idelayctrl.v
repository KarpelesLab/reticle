`timescale 1ns / 1ps
// IDELAYCTRL: RDY out of reset, through a second reset, and with RST held
// low from the start; every change of RDY after time zero printed in
// picoseconds.
module tb;
  reg refclk = 1'b0;
  always #2.5 refclk = ~refclk;
  reg rst = 1'b1;
  wire rdy, rdy2;
  IDELAYCTRL u0 (.RDY(rdy), .REFCLK(refclk), .RST(rst));
  IDELAYCTRL #(.SIM_DEVICE("7SERIES")) u1 (.RDY(rdy2), .REFCLK(refclk), .RST(1'b0));
  always @(rdy) if ($realtime > 0.0) $display("rdy %b %0d", rdy, $rtoi($realtime * 1000.0 + 0.5));
  always @(rdy2) if ($realtime > 0.0) #0.001 $display("rdy2 %b %0d", rdy2, $rtoi($realtime * 1000.0 + 0.5));
  initial begin
    #123.4 rst = 1'b0;
    #500 rst = 1'b1;
    #30.3 rst = 1'b0;
    #400 $finish;
  end
endmodule
