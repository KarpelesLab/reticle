// A FIFO in a block RAM with its pointers and flags.
module bram_fifo (input wire clk, input wire si, output reg so);
  reg [35:0] din;
  reg [35:0] mem [0:1023];
  reg [35:0] dout;
  reg [9:0] wp, rp;
  reg [10:0] used;
  wire wr = din[0] & (used != 11'd1024);
  wire rd = din[1] & (used != 11'd0);
  always @(posedge clk) begin
    din <= {din[34:0], si};
    if (wr) begin mem[wp] <= din; wp <= wp + 10'd1; end
    if (rd) rp <= rp + 10'd1;
    dout <= mem[rp];
    used <= used + wr - rd;
    so <= ^dout ^ used[10];
  end
endmodule
