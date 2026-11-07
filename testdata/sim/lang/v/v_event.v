`timescale 1ns/1ps
module tb;
  event go, done;
  reg [3:0] cnt;
  reg flag;
  initial cnt = 0;
  always @(go) begin
    cnt = cnt + 1;
    $display("%0t got go cnt=%0d", $time, cnt);
    -> done;
  end
  always @(done) $display("%0t got done", $time);
  initial begin
    flag = 0;
    #1 -> go;
    #1 -> go;
    #1 -> go; -> go;
    #1;
    @(done) $display("%0t never (done already fired)", $time);
  end
  initial begin
    #10;
    $display("%0t waiting flag", $time);
    wait (flag) $display("%0t flag seen", $time);
    wait (flag) $display("%0t flag already high", $time);
  end
  initial begin
    #15 flag = 1;
    #1 -> go;
    #1 $display("%0t final cnt=%0d", $time, cnt);
    $finish;
  end
  initial begin
    #5 @(posedge flag) $display("%0t posedge flag", $time);
  end
  initial begin
    #5 @(go or posedge flag) $display("%0t go or flag", $time);
  end
endmodule
