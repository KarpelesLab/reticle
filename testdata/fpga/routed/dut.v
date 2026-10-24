// Carry chains, wide and narrow tables, and registers fed back, on a
// Basys 3's switches and LEDs.
module dut (input wire clk, input wire s, input wire [15:0] sw, output wire [11:0] led);
  wire [7:0] a = sw[7:0], b = sw[15:8];
  reg [7:0] q = 8'd0;
  reg [3:0] r = 4'd0;
  wire [7:0] sum = a + b;
  always @(posedge clk) begin
    q <= s ? (sum ^ {q[6:0], q[7]}) : (a & ~b) | (q >> 1);
    r <= (a[3:0] == b[7:4]) ? r + 1'b1 : {r[2:0], ^q};
  end
  assign led = {r, q};
endmodule
