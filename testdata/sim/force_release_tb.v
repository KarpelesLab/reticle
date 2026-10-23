// `force` and `release` on a variable, a wire and a net below the testbench,
// with the release rules of IEEE 1364-2005 9.3.2: a released variable keeps
// the forced value until it is next assigned; a released wire returns to
// its drivers. Each check is the testbench's own.
module fr_leaf (input wire a, output wire y);
  wire inner = ~a;
  assign y = inner;
endmodule
module force_release_tb;
  reg a = 0;
  reg v = 0;
  wire w = a;
  wire y;
  integer errors = 0;
  fr_leaf u (.a(a), .y(y));
  initial begin
    #1;
    force v = 1'b1;  #1; if (v !== 1'b1) begin errors = errors + 1; $display("FAIL: variable forced %b", v); end
    v = 0;           #1; if (v !== 1'b1) begin errors = errors + 1; $display("FAIL: assignment beat the force %b", v); end
    release v;       #1; if (v !== 1'b1) begin errors = errors + 1; $display("FAIL: released variable %b", v); end
    v = 0;           #1; if (v !== 1'b0) begin errors = errors + 1; $display("FAIL: variable after release %b", v); end
    force w = 1'b1;  #1; if (w !== 1'b1) begin errors = errors + 1; $display("FAIL: wire forced %b", w); end
    release w;       #1; if (w !== 1'b0) begin errors = errors + 1; $display("FAIL: released wire %b", w); end
    force u.inner = 1'b0; #1; if (y !== 1'b0) begin errors = errors + 1; $display("FAIL: hierarchical force %b", y); end
    release u.inner;      #1; if (y !== 1'b1) begin errors = errors + 1; $display("FAIL: hierarchical release %b", y); end
    if (errors == 0) $display("PASS: force and release, local and through the hierarchy");
    $finish;
  end
endmodule
