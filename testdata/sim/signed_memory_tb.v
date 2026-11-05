// A `reg signed` memory's element is signed (IEEE 1364-2005 §4.5.1, §4.9):
// read into a wider variable it is sign-extended, compared with zero it can
// be less, `>>>` keeps its sign, and `%d` prints it negative. Every line here
// was checked against xsim 2026.1, which prints what the comments say.
module signed_memory_tb;
  reg signed [7:0] m [0:3];
  reg signed [7:0] r;
  reg [7:0] u [0:1];
  integer i;
  integer bad;
  initial begin
    bad = 0;
    m[0] = -8'sd5;
    m[1] = 8'sd3;
    u[0] = 8'd251;
    r = m[0];
    // "  -5   -2   -5"
    $display("%d %d %d", m[0], m[0] + m[1], r);
    i = m[0];
    if (i !== -5) begin bad = bad + 1; $display("FAIL: sign extension gave %0d", i); end
    if (!(m[0] < 0)) begin bad = bad + 1; $display("FAIL: m[0] < 0 is false"); end
    if ((m[0] >>> 1) !== -8'sd3) begin bad = bad + 1; $display("FAIL: >>> gave %0d", m[0] >>> 1); end
    i = m[0] * 2;
    if (i !== -10) begin bad = bad + 1; $display("FAIL: m[0] * 2 gave %0d", i); end
    i = m[0] + m[1];
    if (i !== -2) begin bad = bad + 1; $display("FAIL: m[0] + m[1] gave %0d", i); end
    // An unsigned memory stays unsigned.
    i = u[0];
    if (i !== 251) begin bad = bad + 1; $display("FAIL: an unsigned element gave %0d", i); end
    if (u[0] < 0) begin bad = bad + 1; $display("FAIL: an unsigned element is negative"); end
    if (bad == 0) $display("PASS: signed memory elements");
  end
endmodule
