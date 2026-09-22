// A testbench: clock generation, stimulus with delays, tasks, $display,
// $monitor and $finish.
`timescale 1ns / 1ps
module tb;
  reg clk = 0;
  reg rst;
  reg [7:0] d;
  wire [7:0] q;
  integer errors = 0;
  integer fd;

  always #5 clk = ~clk;

  counter #(.WIDTH(8)) dut (
    .clk   (clk),
    .rst_n (~rst),
    .en    (1'b1),
    .up    (1'b1),
    .load  (1'b0),
    .d     (d),
    .q     (q),
    .wrap  ()
  );

  task automatic check(input [7:0] expected);
    begin
      if (q !== expected) begin
        $display("%0t: FAIL q=%h expected=%h", $time, q, expected);
        errors = errors + 1;
      end
    end
  endtask

  task reset_dut;
    begin
      rst = 1;
      repeat (2) @(posedge clk);
      #1 rst = 0;
    end
  endtask

  initial begin
    $dumpfile("tb.vcd");
    $dumpvars(0, tb);
    $monitor("t=%0t q=%0d", $time, q);
    d = 8'h00;
    reset_dut;
    @(posedge clk);
    #1 check(8'h01);
    repeat (10) @(negedge clk);
    check(8'h0b);
    fd = $fopen("log.txt", "w");
    $fdisplay(fd, "errors=%0d", errors);
    $fclose(fd);
    if (errors == 0) $display("PASS");
    else $display("FAIL: %0d errors", errors);
    #100 $finish;
  end

  initial begin : watchdog
    #100000;
    $display("timeout");
    $finish(1);
  end
endmodule
