`timescale 1ns/1ps
module tb;
  integer i, j, n;
  reg [3:0] c4;
  reg [7:0] acc;
  initial begin
    acc = 0;
    for (i = 0; i < 5; i = i + 1) acc = acc + i;
    $display("for sum %0d i=%0d", acc, i);
    for (i = 10; i > 0; i = i - 3) $display("for down %0d", i);
    for (i = 0; i < 0; i = i + 1) $display("never");
    n = 0;
    for (i = 0; i < 3; i = i + 1)
      for (j = 0; j < 3; j = j + 1)
        if (i != j) n = n + 1;
    $display("nested %0d", n);
    i = 0;
    while (i < 4) begin $display("while %0d", i); i = i + 1; end
    i = 100;
    while (i < 4) $display("never while");
    repeat (3) $display("repeat");
    n = 2; repeat (n + 1) $display("repeat expr");
    n = -1; repeat (n) $display("repeat neg never");
    n = 0; repeat (0) $display("repeat 0 never");
    i = 0;
    c4 = 0; n = 0;
    for (c4 = 0; c4 < 15; c4 = c4 + 1) n = n + 1;
    $display("4-bit loop n=%0d c4=%0d", n, c4);
    acc = 8'd250;
    for (i = 0; i < 10; i = i + 1) acc = acc + 1;
    $display("wrap acc=%0d", acc);
    i = 0;
    for (j = 0; j < 100; j = j + 1) begin
      if (j % 7 == 0) i = i + j;
    end
    $display("mod7 sum %0d", i);
    for (i = 0; i < 3; i = i + 1) begin : inner
      integer k;
      k = i * 10;
      $display("inner k=%0d", k);
    end
    $finish;
  end
endmodule
