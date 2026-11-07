`timescale 1ns/1ps
module tb;
  logic [3:0] a;
  int i;
  initial begin
    a = 4'b1010;
    $display("W1 ==? 1?1? : %b", a ==? 4'b1?1?);
    $display("W2 ==? 0??? : %b", a ==? 4'b0???);
    $display("W3 !=? 1x1x : %b", a !=? 4'b1x1x);
    $display("W4 ==? z : %b", a ==? 4'b10zz);
    a = 4'b1x10;
    $display("W5 x lhs ==? 1?10 : %b", a ==? 4'b1?10);
    $display("W6 x lhs ==? 1010 : %b", a ==? 4'b1010);
    $display("W7 x lhs ==? 0?10 : %b", a ==? 4'b0?10);
    i = 5;
    $display("W8 inside list %b", i inside {1, 3, 5});
    $display("W9 inside range %b", i inside {[6:9]});
    $display("W10 inside mixed %b", i inside {[0:2], 5, [10:20]});
    a = 4'b1100;
    $display("W13 inside wildcard %b", a inside {4'b11??});
    a = 4'bx100;
    $display("W14 x inside %b", a inside {4'b0100, 4'b1100});
    i = -3;
    $display("W15 neg inside %b", i inside {[-5:-1]});
    $finish;
  end
endmodule
