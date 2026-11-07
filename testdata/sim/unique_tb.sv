// `unique`, `unique0` and `priority` on `case` and `if`, as xsim 2026.1
// runs them (`unique_tb.xsim`): which branch is taken, and which
// statements warn. xsim writes its warnings into the same log, with the
// file's path in them (shortened here to its name); the test compares the
// rest of the output and the lines the warnings name.
`timescale 1ns/1ps
module tb;
  logic [1:0] v;
  logic [3:0] o;
  initial begin
    v = 2'd1; o = 0;
    unique case (v) 2'd1: o = 1; 2'b?1: o = 2; endcase
    $display("M1 o=%h", o);
    unique casez (v) 2'd1: o = 3; 2'b?1: o = 4; endcase
    $display("M2 o=%h", o);
    unique0 casez (v) 2'd1: o = 5; 2'b?1: o = 6; endcase
    $display("M3 o=%h", o);
    v = 2'd3;
    priority case (v) 2'd0: o = 7; 2'd1: o = 8; endcase
    $display("M4 o=%h", o);
    unique if (v == 0) o = 9; else o = 10;
    $display("M5 o=%h", o);
    unique0 if (v[0]) o = 11; else if (v[1]) o = 12;
    $display("M6 o=%h", o);
    unique0 if (v == 0) o = 13; else if (v == 1) o = 14;
    $display("M7 o=%h", o);
    priority if (v[0]) o = 15; else if (v[1]) o = 1;
    $display("M8 o=%h", o);
    v = 2'bx0;
    unique if (v[1]) o = 2; else if (v[0]) o = 3;
    $display("M9 o=%h", o);
    unique case (v) 2'd0: o = 4; 2'd2: o = 5; endcase
    $display("M10 o=%h", o);
    unique0 case (v) 2'd0: o = 6; endcase
    $display("M11 o=%h", o);
    // A delay first: in the cases tried, xsim reported no `unique if` with
    // no true condition directly after a constant assignment deciding it.
    v = 2'd2; #1;
    unique if (v == 0) o = 7; else if (v == 1) o = 8;
    $display("M12 o=%h", o);
    priority if (v[1]) o = 9; else if (v == 2) o = 10;
    $display("M13 o=%h", o);
    unique if (v[1]) o = 11; else if (v == 2) o = 12;
    $display("M14 o=%h", o);
    $finish;
  end
endmodule
