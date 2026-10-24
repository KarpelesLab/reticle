// dut.v and the netlist Reticle routed it to, renamed dut_routed, on one
// input sequence: every cycle the two must agree. See
// tests/fpga_xray_routed.rs.
`timescale 1ns / 1ps
module tb;
  reg clk = 0, s = 0;
  reg [15:0] sw = 0;
  wire [11:0] want, got;
  always #5 clk = ~clk;
  dut rtl (.clk(clk), .s(s), .sw(sw), .led(want));
  dut_routed routed (.clk(clk), .s(s), .sw(sw), .led(got));
  integer i, bad = 0;
  initial begin
    for (i = 0; i < 2000; i = i + 1) begin
      @(negedge clk);
      if (got !== want) begin
        bad = bad + 1;
        if (bad < 5) $display("cycle %0d: rtl %h routed %h", i, want, got);
      end
      sw = $random;
      s = $random;
    end
    if (bad == 0) $display("PASS: the routed netlist computes what the RTL does");
    else $display("FAIL: %0d cycle(s) differ", bad);
    $finish;
  end
endmodule
