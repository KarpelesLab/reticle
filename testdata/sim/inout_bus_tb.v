// A wire joined to an `inout` port and driven by an `assign` beside it, as
// a testbench drives a pin of the design it tests: the frontend once
// refused it as two drivers, though the simulator resolves the two.
module pad (inout wire io, input wire oe, input wire d, output wire q);
  assign io = oe ? d : 1'bz;
  assign q = io;
endmodule
module inout_bus_tb;
  wire line;
  reg drive = 1'bz;
  reg oe = 0, d = 0;
  wire q;
  assign line = drive;
  pad u (.io(line), .oe(oe), .d(d), .q(q));
  initial begin
    drive = 1'b1; #1;
    if (q !== 1'b1) $display("FAIL: testbench drive %b", q);
    drive = 1'bz; oe = 1; d = 0; #1;
    if (q !== 1'b0) $display("FAIL: pad drive %b", q);
    drive = 1'b1; #1;
    if (line !== 1'bx) $display("FAIL: contention %b", line);
    else $display("PASS: a wire is driven from an inout and an assign, and resolved");
  end
endmodule
