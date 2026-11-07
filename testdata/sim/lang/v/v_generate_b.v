`timescale 1ns/1ps
module leaf #(parameter ID = 0) (input [3:0] a, output [3:0] y);
  assign y = a + ID;
  reg [7:0] tag;
  initial tag = ID * 3;
endmodule
module tb;
  parameter N = 4;
  parameter MODE = 1;
  reg [3:0] a;
  wire [4*N-1:0] y;
  wire [N-1:0] par;
  genvar g;
  generate
    for (g = 0; g < N; g = g + 1) begin : blk
      leaf #(.ID(g)) u (.a(a), .y(y[4*g +: 4]));
      wire [3:0] dbl = y[4*g +: 4] * 2;
      assign par[g] = ^y[4*g +: 4];
      localparam integer SQ = g * g;
    end
  endgenerate
  generate
    if (MODE == 1) begin : m1
      wire [3:0] out = a ^ 4'hf;
    end else begin : m1
      wire [3:0] out = a;
    end
  endgenerate
  generate
    case (N)
      2: begin : cs wire [7:0] v = 8'd2; end
      4: begin : cs wire [7:0] v = 8'd4; end
      default: begin : cs wire [7:0] v = 8'd0; end
    endcase
  endgenerate
  genvar r, c;
  generate
    for (r = 0; r < 2; r = r + 1) begin : row
      for (c = 0; c < 3; c = c + 1) begin : col
        wire [7:0] id = r * 10 + c;
      end
    end
  endgenerate
  integer i;
  initial begin
    a = 4'd5;
    #1;
    for (i = 0; i < N; i = i + 1) $display("y[%0d]=%0d", i, y[4*i +: 4]);
    $display("par=%b", par);
    $display("blk0.dbl=%0d blk3.dbl=%0d", blk[0].dbl, blk[3].dbl);
    $display("SQ %0d %0d", blk[2].SQ, blk[3].SQ);
    $display("m1.out=%b", m1.out);
    $display("cs.v=%0d", cs.v);
    $display("row1.col2.id=%0d row0.col1.id=%0d", row[1].col[2].id, row[0].col[1].id);
    a = 4'd15;
    #1;
    $display("y3=%0d dbl3=%0d par=%b", y[15:12], blk[3].dbl, par);
    $finish;
  end
endmodule
