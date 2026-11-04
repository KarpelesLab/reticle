// A RAM256X1S by name, read into a flip-flop.
module ram256x1s (
    input  wire clk,
    input  wire si,
    output reg  so
);
  reg [10:0] sr;
  always @(posedge clk) sr <= {sr[9:0], si};
  wire o;
  RAM256X1S #(.INIT(256'h6213ae7f78f686b7434311d2bbf56ffffe550703019ce845db8d69f9301c1203)) dut (.O(o), .A(sr[7:0]),
      .D(sr[8]), .WE(sr[9]), .WCLK(clk));
  always @(posedge clk) so <= o ^ sr[10];
endmodule
