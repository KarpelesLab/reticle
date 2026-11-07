// Writes to part of a memory element in SystemVerilog: an element that is
// a packed array of bytes, `mem[a][i] <= d` writing one byte of it and
// `mem[a][i][j]` one bit, under `always_ff`, a struct-free `logic` memory
// sized `[4]`, and the byte-write loop with an `int` variable. Every line
// it prints is held to what xsim 2026.1 printed for it, `mem_part_sv_tb.xsim`
// beside it.
`timescale 1ns / 1ps
module mem_part_sv_tb;
  logic [3:0][7:0] mem [4];
  logic [1:0][0:3] asc [2];
  logic clk = 0;
  logic [3:0] we;
  logic [1:0] addr;
  logic [3:0][7:0] d;

  task automatic show(input [8*10-1:0] what);
    $display("%0s: %h %h %h %h | %h %h", what, mem[0], mem[1], mem[2], mem[3], asc[0], asc[1]);
  endtask

  always_ff @(posedge clk)
    for (int i = 0; i < 4; i++)
      if (we[i]) mem[addr][i] <= d[i];

  initial begin
    show("start");
    mem[0] = '0;
    mem[0][1] = 8'h5a;
    mem[0][3][7] = 1'b1;
    mem[0][2][3:0] = 4'hc;
    show("blocking");
    asc[0] = '0;
    asc[0][1] = 4'h9;
    asc[0][0][0] = 1'b1;
    asc[1] = '1;
    asc[1][0][1 +: 2] = 2'b00;
    show("ascending");
    for (int k = 0; k < 4; k++) mem[1][k] = 8'(k + 1);
    show("loop");
    mem[2] = '1;
    addr = 2;
    d = 32'h01020304;
    we = 4'b1001;
    #1 clk = 1;
    #1 clk = 0;
    show("nba");
    we = 4'b0110;
    addr = 3;
    mem[3] = '0;
    d = 32'hcafef00d;
    #1 clk = 1;
    #1 clk = 0;
    show("nba2");
    $finish;
  end
endmodule
