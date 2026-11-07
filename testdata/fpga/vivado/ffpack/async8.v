// async8: see tools/vivado-parity/gen_ffpack.py.
module async8 (input wire clk, input wire [7:0] d, output wire [7:0] q, input wire en, input wire rst);
  wire [7:0] s;
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f0 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[0]), .Q(s[0]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f1 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[1]), .Q(s[1]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f2 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[2]), .Q(s[2]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f3 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[3]), .Q(s[3]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f4 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[4]), .Q(s[4]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f5 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[5]), .Q(s[5]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f6 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[6]), .Q(s[6]));
  (* DONT_TOUCH = "TRUE" *) FDRE #(.INIT(1'b0)) f7 (.C(clk), .CE(1'b1), .R(1'b0), .D(d[7]), .Q(s[7]));
  (* DONT_TOUCH = "TRUE" *) FDCE #(.INIT(1'b0)) r0 (.C(clk), .CE(en), .CLR(rst), .D(s[0]), .Q(q[0]));
  (* DONT_TOUCH = "TRUE" *) FDPE #(.INIT(1'b1)) r1 (.C(clk), .CE(en), .PRE(rst), .D(s[1]), .Q(q[1]));
  (* DONT_TOUCH = "TRUE" *) FDCE #(.INIT(1'b1)) r2 (.C(clk), .CE(en), .CLR(rst), .D(s[2]), .Q(q[2]));
  (* DONT_TOUCH = "TRUE" *) FDPE #(.INIT(1'b0)) r3 (.C(clk), .CE(en), .PRE(rst), .D(s[3]), .Q(q[3]));
  (* DONT_TOUCH = "TRUE" *) FDCE #(.INIT(1'b0)) r4 (.C(clk), .CE(en), .CLR(rst), .D(s[4]), .Q(q[4]));
  (* DONT_TOUCH = "TRUE" *) FDPE #(.INIT(1'b1)) r5 (.C(clk), .CE(en), .PRE(rst), .D(s[5]), .Q(q[5]));
  (* DONT_TOUCH = "TRUE" *) FDCE #(.INIT(1'b1)) r6 (.C(clk), .CE(en), .CLR(rst), .D(s[6]), .Q(q[6]));
  (* DONT_TOUCH = "TRUE" *) FDPE #(.INIT(1'b0)) r7 (.C(clk), .CE(en), .PRE(rst), .D(s[7]), .Q(q[7]));
endmodule
