// An instance array: a connection as wide as the port goes to every
// instance, and one as wide as all of them together is cut, the
// rightmost index taking the least significant bits.
module ia_cell(input [1:0] a, input en, output [1:0] y);
  assign y = en ? a : 2'b00;
endmodule

module instance_array(input [7:0] up, input [7:0] down, input en, output [7:0] y_up, output [7:0] y_down);
  ia_cell u [3:0] (.a(up), .en(en), .y(y_up));
  ia_cell d [0:3] (.a(down), .en(en), .y(y_down));
endmodule
