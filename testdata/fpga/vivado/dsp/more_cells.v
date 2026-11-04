// The cells a mapped netlist names that examples/husky/sim/xilinx_cells.v has not
// got: a global clock buffer, as a wire, the slice's wide multiplexers, and the
// shift registers mapping now writes, copied from src/fpga/devices/xc7_cells.v.
module BUFG (output wire O, input wire I);
  assign O = I;
endmodule

module MUXF7 (output wire O, input wire I0, input wire I1, input wire S);
  assign O = S ? I1 : I0;
endmodule

module MUXF8 (output wire O, input wire I0, input wire I1, input wire S);
  assign O = S ? I1 : I0;
endmodule
module SRL16E #(parameter [15:0] INIT = 16'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0) (output Q, input A0, input A1, input A2, input A3, input CE, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[14:0], D};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[14:0], D};
    end
  endgenerate
endmodule
module SRLC32E #(parameter [31:0] INIT = 32'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0) (output Q, output Q31, input [4:0] A, input CE, input CLK, input D);
  reg [31:0] r = INIT;
  assign Q = r[A];
  assign Q31 = r[31];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[30:0], D};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[30:0], D};
    end
  endgenerate
endmodule
