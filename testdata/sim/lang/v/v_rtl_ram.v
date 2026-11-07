`timescale 1ns/1ps
module ram_bw #(parameter AW = 4) (
  input clk, input we, input [3:0] be, input [AW-1:0] addr,
  input [31:0] wdata, output reg [31:0] rdata
);
  reg [31:0] mem [0:(1<<AW)-1];
  integer k;
  initial for (k = 0; k < (1<<AW); k = k + 1) mem[k] = 32'h0;
  always @(posedge clk) begin
    if (we) begin
      if (be[0]) mem[addr][7:0]   <= wdata[7:0];
      if (be[1]) mem[addr][15:8]  <= wdata[15:8];
      if (be[2]) mem[addr][23:16] <= wdata[23:16];
      if (be[3]) mem[addr][31:24] <= wdata[31:24];
    end
    rdata <= mem[addr];
  end
endmodule
module tb;
  reg clk, we;
  reg [3:0] be, addr;
  reg [31:0] wdata;
  wire [31:0] rdata;
  reg [31:0] model [0:15];
  integer i, errors, b;
  ram_bw #(.AW(4)) dut (.clk(clk), .we(we), .be(be), .addr(addr), .wdata(wdata), .rdata(rdata));
  initial begin clk = 0; forever #5 clk = ~clk; end
  task wr(input [3:0] a, input [3:0] e, input [31:0] d);
    begin
      @(negedge clk); we = 1; addr = a; be = e; wdata = d;
      @(negedge clk); we = 0;
      for (b = 0; b < 4; b = b + 1) if (e[b]) model[a][8*b +: 8] = d[8*b +: 8];
    end
  endtask
  task rd(input [3:0] a);
    begin
      @(negedge clk); we = 0; addr = a;
      @(posedge clk); #1;
      if (rdata !== model[a]) errors = errors + 1;
      $display("%0t rd[%0d]=%h model=%h %s", $time, a, rdata, model[a], rdata === model[a] ? "ok" : "BAD");
    end
  endtask
  initial begin
    errors = 0; we = 0; be = 0; addr = 0; wdata = 0;
    for (i = 0; i < 16; i = i + 1) model[i] = 0;
    wr(4'd0, 4'b1111, 32'hdeadbeef);
    wr(4'd1, 4'b0001, 32'h11223344);
    wr(4'd2, 4'b0110, 32'haabbccdd);
    wr(4'd3, 4'b1000, 32'h12345678);
    wr(4'd0, 4'b0101, 32'h00ff00ff);
    wr(4'd15, 4'b1111, 32'hcafef00d);
    wr(4'd15, 4'b0000, 32'hffffffff);
    for (i = 0; i < 4; i = i + 1) rd(i);
    rd(4'd15);
    @(negedge clk); we = 1; be = 4'b0011; addr = 4'd5; wdata = 32'h0000abcd;
    @(posedge clk); #1 $display("%0t read-during-write rdata=%h", $time, rdata);
    @(negedge clk); we = 0; model[5] = 32'h0000abcd;
    rd(4'd5);
    $display("errors=%0d", errors);
    $finish;
  end
endmodule
