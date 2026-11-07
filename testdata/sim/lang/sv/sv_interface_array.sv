`timescale 1ns/1ps
interface ch_if;
  logic [3:0] v;
endinterface

module drv #(parameter int K = 0) (ch_if c);
  assign c.v = 4'(K * 3);
endmodule

module tb;
  ch_if ch [3] ();
  genvar g;
  generate
    for (g = 0; g < 3; g++) begin : gd
      drv #(.K(g + 1)) d (.c(ch[g]));
    end
  endgenerate
  initial begin
    #1;
    $display("IA1 ch0=%0d ch1=%0d ch2=%0d", ch[0].v, ch[1].v, ch[2].v);
    $finish;
  end
endmodule
