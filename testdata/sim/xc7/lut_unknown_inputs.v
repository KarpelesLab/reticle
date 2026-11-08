`timescale 1ns / 1ps
// Lookup tables with unknown and floating inputs: whether the output is
// known when the inputs that are not known cannot change it. Every width,
// the `_L` and `_D` forms, and both outputs of a `LUT6_2`, each with a truth
// table that ignores some of its inputs. Also the loop a `LUT6_2` can be
// in, which the 7-series flow builds: `O5` reads only `I0` and `I3`, and
// `O6`, through a lookup table and back, reads `O5` on `I1`. Under the
// language's own rule, an unknown index reads `x`, that loop never leaves
// `x`; the routed netlist of the language corpus's `v_alu` held one.
module tb;
  reg [5:0] i = 6'b0;
  wire o1, o2, o3, o4, o5, o6;
  wire l1, l2, l3, l4, l5, l6;
  wire d1, d2, d3, d4, d5, d6, e1, e2, e3, e4, e5, e6;
  wire p5, p6;
  // Each reads I0 and not the others: the table repeats I0.
  LUT1 #(.INIT(2'h2)) u1 (.O(o1), .I0(i[0]));
  LUT2 #(.INIT(4'ha)) u2 (.O(o2), .I0(i[0]), .I1(i[1]));
  LUT3 #(.INIT(8'haa)) u3 (.O(o3), .I0(i[0]), .I1(i[1]), .I2(i[2]));
  LUT4 #(.INIT(16'haaaa)) u4 (.O(o4), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]));
  LUT5 #(.INIT(32'haaaaaaaa)) u5 (.O(o5), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]),
    .I4(i[4]));
  LUT6 #(.INIT(64'haaaaaaaaaaaaaaaa)) u6 (.O(o6), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]),
    .I4(i[4]), .I5(i[5]));
  // Each reads its top input and not the others.
  LUT1_L #(.INIT(2'h1)) v1 (.LO(l1), .I0(i[0]));
  LUT2_L #(.INIT(4'hc)) v2 (.LO(l2), .I0(i[0]), .I1(i[1]));
  LUT3_L #(.INIT(8'hf0)) v3 (.LO(l3), .I0(i[0]), .I1(i[1]), .I2(i[2]));
  LUT4_L #(.INIT(16'hff00)) v4 (.LO(l4), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]));
  LUT5_L #(.INIT(32'hffff0000)) v5 (.LO(l5), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]),
    .I4(i[4]));
  LUT6_L #(.INIT(64'hffffffff00000000)) v6 (.LO(l6), .I0(i[0]), .I1(i[1]), .I2(i[2]),
    .I3(i[3]), .I4(i[4]), .I5(i[5]));
  // AND of everything: known only when some input is a known zero.
  LUT1_D #(.INIT(2'h2)) w1 (.O(d1), .LO(e1), .I0(i[0]));
  LUT2_D #(.INIT(4'h8)) w2 (.O(d2), .LO(e2), .I0(i[0]), .I1(i[1]));
  LUT3_D #(.INIT(8'h80)) w3 (.O(d3), .LO(e3), .I0(i[0]), .I1(i[1]), .I2(i[2]));
  LUT4_D #(.INIT(16'h8000)) w4 (.O(d4), .LO(e4), .I0(i[0]), .I1(i[1]), .I2(i[2]), .I3(i[3]));
  LUT5_D #(.INIT(32'h80000000)) w5 (.O(d5), .LO(e5), .I0(i[0]), .I1(i[1]), .I2(i[2]),
    .I3(i[3]), .I4(i[4]));
  LUT6_D #(.INIT(64'h8000000000000000)) w6 (.O(d6), .LO(e6), .I0(i[0]), .I1(i[1]), .I2(i[2]),
    .I3(i[3]), .I4(i[4]), .I5(i[5]));
  // O5 is I0 ^ I3; O6 (I5 high) is I1 ^ I2.
  LUT6_2 #(.INIT(64'h3c3c3c3c55aa55aa)) x (.O5(p5), .O6(p6), .I0(i[0]), .I1(i[1]),
    .I2(i[2]), .I3(i[3]), .I4(i[4]), .I5(i[5]));

  // The loop: O5 = I0 ^ I3, through a buffer back to I1; O6 = I1 ^ I2.
  reg a = 1'b0, b = 1'b0, c = 1'b0;
  wire q5, q6, back;
  LUT6_2 #(.INIT(64'h3c3c3c3c55aa55aa)) y (.O5(q5), .O6(q6), .I0(a), .I1(back), .I2(c),
    .I3(b), .I4(1'b0), .I5(1'b1));
  LUT6 #(.INIT(64'haaaaaaaaaaaaaaaa)) z (.O(back), .I0(q5), .I1(1'b1), .I2(1'b1), .I3(1'b1),
    .I4(1'b1), .I5(1'b1));

  task show(input [5:0] v);
    begin
      i = v;
      #1 $display("%b | %b%b%b%b%b%b %b%b%b%b%b%b %b%b%b%b%b%b %b%b%b%b%b%b %b%b", i,
        o1, o2, o3, o4, o5, o6, l1, l2, l3, l4, l5, l6, d1, d2, d3, d4, d5, d6,
        e1, e2, e3, e4, e5, e6, p5, p6);
    end
  endtask
  initial begin
    show(6'b000000);
    show(6'b111111);
    show(6'bxxxxx1);
    show(6'bxxxxx0);
    show(6'bzzzzz1);
    show(6'b1xxxxx);
    show(6'b0xxxxx);
    show(6'bxxxxxx);
    show(6'b11111x);
    show(6'b111110);
    show(6'b0x1x1x);
    show(6'b10x110);
    show(6'b10x01x);
    show(6'b1x10x1);
    show(6'b1x00x0);
    show(6'b01x0x1);
    show(6'b00z1x0);
    #1 $display("loop %b %b %b", q5, back, q6);
    a = 1'b1;
    #1 $display("loop %b %b %b", q5, back, q6);
    c = 1'b1;
    #1 $display("loop %b %b %b", q5, back, q6);
    b = 1'b1;
    #1 $display("loop %b %b %b", q5, back, q6);
    $finish;
  end
endmodule
