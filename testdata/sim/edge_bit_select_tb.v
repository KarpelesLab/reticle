// An edge of a bit-select, a part-select or a vector is an edge of its least
// significant bit (IEEE 1364-2005 §9.7.2), and of nothing else: `x[1]` rises
// twice below while `x` changes four times. Checked against xsim 2026.1,
// which counts the same.
module edge_bit_select_tb;
  reg [1:0] x;
  reg [3:0] y;
  integer rise1, fall0, rise_y, rise_part;
  time waited;
  always @(posedge x[1]) rise1 = rise1 + 1;
  always @(negedge x[0]) fall0 = fall0 + 1;
  always @(posedge y) rise_y = rise_y + 1;
  always @(posedge y[3:2]) rise_part = rise_part + 1;
  initial begin
    @(posedge x[1]);
    waited = $time;
  end
  initial begin
    waited = 0;
    x = 0; y = 0;
    // Counted from here, past the edges of time 0 (x and y leaving x), which
    // race the always blocks' first wait.
    #1 rise1 = 0; fall0 = 0; rise_y = 0; rise_part = 0;
    #1 x = 2;
    #1 x = 1;
    #1 x = 3;
    #1 x = 0;
    // `y` rises at its bit 0 twice and at bit 2 twice, and at bit 3, the top
    // of both, once.
    #1 y = 4'b1000;
    #1 y = 4'b1001;
    #1 y = 4'b0000;
    #1 y = 4'b0101;
    #1 y = 4'b0000;
    #1 y = 4'b0100;
    #1 y = 4'b0000;
    #1;
    if (rise1 == 2 && fall0 == 1 && rise_y == 2 && rise_part == 2 && waited == 64'd2)
      $display("PASS: edges of a select");
    else
      $display("FAIL: rise1=%0d fall0=%0d rise_y=%0d rise_part=%0d waited=%0d",
               rise1, fall0, rise_y, rise_part, waited);
  end
endmodule
