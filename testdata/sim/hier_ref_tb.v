// Hierarchical references into the design under test, as NewAE's Husky
// testbench makes them: read in an expression, watched by an event control,
// read by a net's initialiser, and forced and released, all through
// instances below the testbench, one of them declared below the lines that
// name it. Each check is the testbench's own.
module hr_leaf (input wire clk, input wire a, output reg [3:0] count, output wire y);
  initial count = 0;
  always @(posedge clk) count <= count + 1;
  wire flag = count == 4'd3;
  wire inner = ~a;
  assign y = inner;
endmodule

module hr_mid (input wire clk, input wire a, output wire y);
  hr_leaf u_leaf (.clk(clk), .a(a), .count(), .y(y));
endmodule

module hier_ref_tb;
  reg clk = 0;
  always #5 clk = ~clk;
  reg a = 0;
  wire y;
  integer edges = 0, errors = 0;
  wire flag_copy = u_mid.u_leaf.flag;  // a net's initialiser
  always @(posedge u_mid.u_leaf.flag) edges = edges + 1;
  initial begin
    #2;
    if (u_mid.u_leaf.count !== 4'd0) errors = errors + 1;
    #40;
    if (u_mid.u_leaf.count !== 4'd4 || edges != 1) errors = errors + 1;

    force u_mid.u_leaf.inner = 1'b0; #1;
    if (y !== 1'b0) errors = errors + 1;
    release u_mid.u_leaf.inner; #1;
    if (y !== 1'b1) errors = errors + 1;
    if (errors == 0 && flag_copy === (u_mid.u_leaf.count == 4'd3))
      $display("PASS: read, watched, initialised from, forced and released through the hierarchy");
    else
      $display("FAIL: %0d check(s), count %0d edges %0d", errors, u_mid.u_leaf.count, edges);
    $finish;
  end
  hr_mid u_mid (.clk(clk), .a(a), .y(y));
endmodule
