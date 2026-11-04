// The cells a mapped netlist names that examples/husky/sim/xilinx_cells.v has not
// got: a global clock buffer, as a wire, and the slice's wide multiplexers.
module BUFG (output wire O, input wire I);
  assign O = I;
endmodule

module MUXF7 (output wire O, input wire I0, input wire I1, input wire S);
  assign O = S ? I1 : I0;
endmodule

module MUXF8 (output wire O, input wire I0, input wire I1, input wire S);
  assign O = S ? I1 : I0;
endmodule
