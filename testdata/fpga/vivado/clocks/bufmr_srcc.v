// BUFMR on a single-region clock pad.
module bufmr_srcc (
    input  wire clk,
    input  wire si,
    output reg  so,
    input wire srcc
);
  reg [9:0] sr;
  always @(posedge clk) sr <= {sr[8:0], si};
  wire y;
  wire mr, o;
  BUFMR dut (.O(mr), .I(srcc));
  BUFR u_r (.O(o), .CE(1'b1), .CLR(1'b0), .I(mr));
  reg t;
  always @(posedge o) t <= sr[0];
  assign y = t;
  always @(posedge clk) so <= y ^ sr[0];
endmodule
