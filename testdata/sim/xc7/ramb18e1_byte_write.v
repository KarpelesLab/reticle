`timescale 1ns / 1ps
// RAMB18E1 as Vivado 2026.1 placed a 16-word, 32-bit memory with a byte
// enable per byte (`v_ram_b`'s `ram_bw` in the language corpus): true dual
// port, both ports 18 bits and read-first on one clock, port A the low half
// of each word at addresses 0..15 and port B the high half at 1024 + the
// same, `WEA` and `WEBWE[1:0]` the byte enables, `WEBWE[3:2]` and the parity
// inputs tied as Vivado tied them. Two writes are made inside the first
// 100 ns, where the global reset holds the block and they are lost, and the
// rest after it, where they land; a word is read back after every write.
module tb;
  reg clk = 1'b0;
  always #5 clk = ~clk;
  reg [3:0] addr = 4'd0;
  reg [3:0] be = 4'd0;
  reg we = 1'b0;
  reg [31:0] wdata = 32'h0;
  wire [3:0] e = be & {4{we}};
  wire [31:0] rdata;
  wire [1:0] pa, pb;
  RAMB18E1 #(.DOA_REG(0), .DOB_REG(0), .INIT_A(18'h00000), .INIT_B(18'h00000),
    .RAM_MODE("TDP"), .RDADDR_COLLISION_HWCONFIG("DELAYED_WRITE"), .READ_WIDTH_A(18),
    .READ_WIDTH_B(18), .RSTREG_PRIORITY_A("RSTREG"), .RSTREG_PRIORITY_B("RSTREG"),
    .SIM_COLLISION_CHECK("ALL"), .SIM_DEVICE("7SERIES"), .SRVAL_A(18'h00000),
    .SRVAL_B(18'h00000), .WRITE_MODE_A("READ_FIRST"), .WRITE_MODE_B("READ_FIRST"),
    .WRITE_WIDTH_A(18), .WRITE_WIDTH_B(18))
    mem (.ADDRARDADDR({6'b000000, addr, 4'b0000}), .ADDRBWRADDR({6'b100000, addr, 4'b0000}),
    .CLKARDCLK(clk), .CLKBWRCLK(clk), .DIADI(wdata[15:0]), .DIBDI(wdata[31:16]),
    .DIPADIP(2'b11), .DIPBDIP(2'b11), .DOADO(rdata[15:0]), .DOBDO(rdata[31:16]),
    .DOPADOP(pa), .DOPBDOP(pb), .ENARDEN(1'b1), .ENBWREN(1'b1), .REGCEAREGCE(1'b0),
    .REGCEB(1'b0), .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0), .RSTREGARSTREG(1'b0),
    .RSTREGB(1'b0), .WEA(e[1:0]), .WEBWE({2'b00, e[3:2]}));

  task wr(input [3:0] a, input [3:0] en, input [31:0] d);
    begin
      @(negedge clk); we = 1'b1; addr = a; be = en; wdata = d;
      @(negedge clk); we = 1'b0;
    end
  endtask
  task rd(input [3:0] a);
    begin
      @(negedge clk); we = 1'b0; addr = a;
      @(posedge clk); #1;
      $display("%0t rd[%0d] %h %b %b", $time, a, rdata, pa, pb);
    end
  endtask
  task pass;
    begin
      wr(4'd0, 4'b1111, 32'hdeadbeef); rd(4'd0);
      wr(4'd1, 4'b0001, 32'h11223344); rd(4'd1);
      wr(4'd2, 4'b0110, 32'haabbccdd); rd(4'd2);
      wr(4'd3, 4'b1000, 32'h12345678); rd(4'd3);
      wr(4'd0, 4'b0101, 32'h00ff00ff); rd(4'd0);
      wr(4'd15, 4'b1111, 32'hcafef00d); rd(4'd15);
      wr(4'd15, 4'b0000, 32'hffffffff); rd(4'd15);
      // Read-first: the word before the write, on the write's own edge.
      @(negedge clk); we = 1'b1; be = 4'b0011; addr = 4'd5; wdata = 32'h0000abcd;
      @(posedge clk); #1 $display("%0t during %h", $time, rdata);
      @(negedge clk); we = 1'b0;
      rd(4'd5); rd(4'd1); rd(4'd2); rd(4'd3);
    end
  endtask
  initial begin
    wr(4'd7, 4'b1111, 32'h01234567); rd(4'd7);
    wr(4'd6, 4'b0011, 32'h89abcdef); rd(4'd6);
    #100;
    pass;
    rd(4'd7); rd(4'd6);
    $finish;
  end
endmodule
