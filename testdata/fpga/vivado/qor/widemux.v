// A 64-to-1 multiplexer on 16 bits: where MUXF7 and MUXF8 earn their place.
module widemux (input wire clk, input wire si, output reg so);
  reg [1029:0] in;
  reg [15:0] q;
  always @(posedge clk) begin
    in <= {in[1028:0], si};
    q <= in[1023:0] >> {in[1029:1024], 4'd0};
    so <= ^q;
  end
endmodule
