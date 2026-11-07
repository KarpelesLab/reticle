`timescale 1ns/1ps
module tb;
  int i, n, s;
  initial begin
    i = 0;
    do begin
      $display("L1 do-while i=%0d", i);
      i++;
    end while (i < 3);
    i = 10;
    do i++; while (i < 5);
    $display("L2 do-while once i=%0d", i);
    s = 0;
    for (int k = 0; k < 10; k++) begin
      if (k == 2) continue;
      if (k == 6) break;
      s += k;
    end
    $display("L3 for break/continue s=%0d", s);
    s = 0;
    for (int a = 0, b = 10; a < b; a += 2, b -= 1) s += a * b;
    $display("L4 multi-var for s=%0d", s);
    n = 0;
    while (1) begin
      n++;
      if (n >= 5) break;
    end
    $display("L5 while break n=%0d", n);
    s = 0;
    repeat (4) s += 3;
    $display("L6 repeat s=%0d", s);
    s = 0;
    for (int k = 0; k < 3; k++)
      for (int m = 0; m < 3; m++) begin
        if (m == k) continue;
        s += 10 * k + m;
      end
    $display("L7 nested continue s=%0d", s);
    s = 0;
    for (int k = 5; k > 0; k--) s = s * 2 + k;
    $display("L8 down s=%0d", s);
    s = 0;
    forever begin
      s++;
      if (s == 7) break;
    end
    $display("L9 forever break s=%0d", s);
    for (byte b = 125; b > 0; b++) s++;
    $display("L10 byte loop s=%0d", s);
    $finish;
  end
endmodule
