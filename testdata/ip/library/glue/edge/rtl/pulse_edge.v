// One cycle of `pulse` for every rising edge of `level`.
module pulse_edge(input clk, input level, output pulse);
  reg last;
  always @(posedge clk)
    last <= level;
  assign pulse = level & ~last;
endmodule
