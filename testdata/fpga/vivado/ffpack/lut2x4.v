// lut2x4: see tools/vivado-parity/gen_ffpack.py.
module lut2x4 (input wire clk, input wire [7:0] d, output wire [7:0] q, input wire [3:0] e, output wire [3:0] o);
  wire [7:0] s;
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f0 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[0]), .Q(s[0]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f1 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[1]), .Q(s[1]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f2 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[2]), .Q(s[2]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f3 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[3]), .Q(s[3]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f4 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[4]), .Q(s[4]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f5 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[5]), .Q(s[5]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f6 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[6]), .Q(s[6]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f7 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[7]), .Q(s[7]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r0 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[0]), .Q(q[0]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r1 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[1]), .Q(q[1]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r2 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[2]), .Q(q[2]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r3 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[3]), .Q(q[3]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r4 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[4]), .Q(q[4]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r5 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[5]), .Q(q[5]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r6 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[6]), .Q(q[6]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) r7 (.C(clk), .CE(1'b1), .R(1'b0), .D(s[7]), .Q(q[7]));
  wire [3:0] t;
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) g0 (.C(clk), .CE(1'b1), .R(1'b0), .D(e[0]), .Q(t[0]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) g1 (.C(clk), .CE(1'b1), .R(1'b0), .D(e[1]), .Q(t[1]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) g2 (.C(clk), .CE(1'b1), .R(1'b0), .D(e[2]), .Q(t[2]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) g3 (.C(clk), .CE(1'b1), .R(1'b0), .D(e[3]), .Q(t[3]));
  (* DONT_TOUCH = "TRUE" *) LUT2 #(.INIT(4'h6)) l0 (.I0(t[0]), .I1(t[1]), .O(o[0]));
  (* DONT_TOUCH = "TRUE" *) LUT2 #(.INIT(4'h8)) l1 (.I0(t[0]), .I1(t[1]), .O(o[1]));
  (* DONT_TOUCH = "TRUE" *) LUT2 #(.INIT(4'hE)) l2 (.I0(t[0]), .I1(t[1]), .O(o[2]));
  (* DONT_TOUCH = "TRUE" *) LUT2 #(.INIT(4'h9)) l3 (.I0(t[0]), .I1(t[1]), .O(o[3]));
endmodule
