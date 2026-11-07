`timescale 1ns/1ps
module tb;
  int a;
  int errors;
  initial begin
    errors = 0;
    a = 5;
    assert (a == 5) $display("AS1 pass a=%0d", a);
    else $display("AS1 fail");
    assert (a == 6) $display("AS2 pass?");
    else begin
      errors++;
      $display("AS2 else branch a=%0d", a);
    end
    #1;
    assert (a > 0);
    $display("AS3 after plain assert");
    assert (a < 0) else $error("AS4 error message a=%0d", a);
    #1;
    assert (a < 0) else $warning("AS5 warning message a=%0d", a);
    #1;
    assert (a < 0) else $info("AS6 info message");
    #1;
    a = 0;
    assert (a);
    #1;
    assert (1'bx) $display("AS8 x pass"); else $display("AS8 x fail branch");
    $display("AS9 errors=%0d", errors);
    $finish;
  end
endmodule
