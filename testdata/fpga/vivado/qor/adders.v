// A tree of registered adders: carry chains with general routing between them.
module adders (input wire clk, input wire si, output reg so);
  reg [255:0] in;
  reg [31:0] a [0:6];
  integer i;
  always @(posedge clk) begin
    in <= {in[254:0], si};
    a[0] <= in[31:0] + in[63:32];
    a[1] <= in[95:64] + in[127:96];
    a[2] <= in[159:128] + in[191:160];
    a[3] <= in[223:192] + in[255:224];
    a[4] <= a[0] - a[1];
    a[5] <= a[2] - a[3];
    a[6] <= a[4] + a[5];
    so <= ^a[6];
  end
endmodule
