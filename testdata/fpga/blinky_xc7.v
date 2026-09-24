module blinky (.clk(clk$pad), .rst(rst$pad), .sw(sw$pad), .led(led$pad));
  input clk$pad;
  input rst$pad;
  input [3:0] sw$pad;
  output [3:0] led$pad;
  wire clk;
  wire rst;
  wire [3:0] led;
  wire [7:0] count;
  wire [7:0] next;
  wire tick;
  wire [3:0] mix;
  wire inc$p3;
  wire inc$p2;
  wire inc$p1;
  wire inc$p0;
  wire [3:0] inc$o0;
  wire [3:0] inc$co0;
  wire inc$p7;
  wire inc$p6;
  wire inc$p5;
  wire inc$p4;
  wire [3:0] inc$o1;
  wire [3:0] inc$co1;
  wire clk$pad;
  wire clk$in0;
  wire rst$pad;
  wire rst$in0;
  wire [3:0] sw$pad;
  wire sw$in0;
  wire sw$in1;
  wire sw$in2;
  wire sw$in3;
  wire [3:0] led$pad;
  wire led$pin0;
  wire led$pin1;
  wire led$pin2;
  wire led$pin3;
  wire clk$gb;
  wire \$lut0 ;
  wire \$lut1 ;
  wire \$lut2 ;
  wire \$lut3 ;
  wire counter$q0;
  wire counter$q1;
  wire counter$q2;
  wire counter$q3;
  wire counter$q4;
  wire counter$q5;
  wire counter$q6;
  wire counter$q7;
  wire out$q0;
  wire out$q1;
  wire out$q2;
  wire out$q3;
  function [0:0] reticle_bits_2_0_0;
    input [1:0] x;
    reticle_bits_2_0_0 = x[0:0];
  endfunction
  function [0:0] reticle_bits_2_1_1;
    input [1:0] x;
    reticle_bits_2_1_1 = x[1:1];
  endfunction
  assign clk = clk$in0;
  assign rst = rst$in0;
  assign next = {inc$o1[3], inc$o1[2], inc$o1[1], inc$o1[0], inc$o0[3], inc$o0[2], inc$o0[1], inc$o0[0]};
  assign tick = count[0];
  assign mix = {\$lut0 , \$lut1 , \$lut2 , \$lut3 };
  assign inc$p3 = count[3];
  assign inc$p2 = count[2];
  assign inc$p1 = count[1];
  assign inc$p7 = count[7];
  assign inc$p6 = count[6];
  assign inc$p5 = count[5];
  assign inc$p4 = count[4];
  assign led$pad = {led$pin3, led$pin2, led$pin1, led$pin0};
  assign count = {counter$q7, counter$q6, counter$q5, counter$q4, counter$q3, counter$q2, counter$q1, counter$q0};
  assign led = {out$q3, out$q2, out$q1, out$q0};
  CARRY4 inc$carry0 (
    .S({inc$p3, inc$p2, inc$p1, inc$p0}),
    .DI({count[3], count[2], count[1], count[0]}),
    .CYINIT(1'b0),
    .CI(1'b0),
    .O(inc$o0),
    .CO(inc$co0)
  );
  CARRY4 inc$carry1 (
    .S({inc$p7, inc$p6, inc$p5, inc$p4}),
    .DI({count[7], count[6], count[5], count[4]}),
    .CI(inc$co0[3]),
    .CYINIT(1'b0),
    .O(inc$o1),
    .CO(inc$co1)
  );
  (* port = "clk", pin = "W5", io_standard = "LVCMOS33" *)
  IBUF clk$io0 (
    .I(clk$pad),
    .O(clk$in0)
  );
  (* port = "rst", pin = "U18", io_standard = "LVCMOS33" *)
  IBUF rst$io0 (
    .I(rst$pad),
    .O(rst$in0)
  );
  (* port = "sw", pin = "V17", io_standard = "LVCMOS33" *)
  IBUF sw$io0 (
    .I(sw$pad[0]),
    .O(sw$in0)
  );
  (* port = "sw", pin = "V16", io_standard = "LVCMOS33" *)
  IBUF sw$io1 (
    .I(sw$pad[1]),
    .O(sw$in1)
  );
  (* port = "sw", pin = "W16", io_standard = "LVCMOS33" *)
  IBUF sw$io2 (
    .I(sw$pad[2]),
    .O(sw$in2)
  );
  (* port = "sw", pin = "W17", io_standard = "LVCMOS33" *)
  IBUF sw$io3 (
    .I(sw$pad[3]),
    .O(sw$in3)
  );
  (* port = "led", pin = "U16", io_standard = "LVCMOS33", drive = 12, slew = "slow" *)
  OBUF led$io0 (
    .I(led[0]),
    .O(led$pin0)
  );
  (* port = "led", pin = "E19", io_standard = "LVCMOS33", drive = 12, slew = "slow" *)
  OBUF led$io1 (
    .I(led[1]),
    .O(led$pin1)
  );
  (* port = "led", pin = "U19", io_standard = "LVCMOS33", drive = 12, slew = "slow" *)
  OBUF led$io2 (
    .I(led[2]),
    .O(led$pin2)
  );
  (* port = "led", pin = "V19", io_standard = "LVCMOS33", drive = 12, slew = "slow" *)
  OBUF led$io3 (
    .I(led[3]),
    .O(led$pin3)
  );
  BUFG clk$gbuf (
    .I(clk),
    .O(clk$gb)
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut4  (
    .I0(reticle_bits_2_0_0({sw$in3, count[7]})),
    .I1(reticle_bits_2_1_1({sw$in3, count[7]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut0 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut5  (
    .I0(reticle_bits_2_0_0({sw$in2, count[6]})),
    .I1(reticle_bits_2_1_1({sw$in2, count[6]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut1 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut6  (
    .I0(reticle_bits_2_0_0({sw$in1, count[5]})),
    .I1(reticle_bits_2_1_1({sw$in1, count[5]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut2 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut7  (
    .I0(reticle_bits_2_0_0({sw$in0, count[4]})),
    .I1(reticle_bits_2_1_1({sw$in0, count[4]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut3 )
  );
  LUT6 #(.INIT(64'h5555555555555555)) \$lut8  (
    .I0(count[0]),
    .I1(1'b0),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(inc$p0)
  );
  FDRE #(.INIT(1'b0)) counter$ff0 (
    .C(clk$gb),
    .D(next[0]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q0)
  );
  FDRE #(.INIT(1'b0)) counter$ff1 (
    .C(clk$gb),
    .D(next[1]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q1)
  );
  FDRE #(.INIT(1'b0)) counter$ff2 (
    .C(clk$gb),
    .D(next[2]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q2)
  );
  FDRE #(.INIT(1'b0)) counter$ff3 (
    .C(clk$gb),
    .D(next[3]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q3)
  );
  FDSE #(.INIT(1'b1)) counter$ff4 (
    .C(clk$gb),
    .D(next[4]),
    .CE(1'b1),
    .S(rst),
    .Q(counter$q4)
  );
  FDRE #(.INIT(1'b0)) counter$ff5 (
    .C(clk$gb),
    .D(next[5]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q5)
  );
  FDRE #(.INIT(1'b0)) counter$ff6 (
    .C(clk$gb),
    .D(next[6]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q6)
  );
  FDRE #(.INIT(1'b0)) counter$ff7 (
    .C(clk$gb),
    .D(next[7]),
    .CE(1'b1),
    .R(rst),
    .Q(counter$q7)
  );
  FDCE #(.INIT(1'b0)) out$ff0 (
    .C(clk$gb),
    .D(mix[0]),
    .CE(tick),
    .CLR(rst),
    .Q(out$q0)
  );
  FDCE #(.INIT(1'b0)) out$ff1 (
    .C(clk$gb),
    .D(mix[1]),
    .CE(tick),
    .CLR(rst),
    .Q(out$q1)
  );
  FDCE #(.INIT(1'b0)) out$ff2 (
    .C(clk$gb),
    .D(mix[2]),
    .CE(tick),
    .CLR(rst),
    .Q(out$q2)
  );
  FDCE #(.INIT(1'b0)) out$ff3 (
    .C(clk$gb),
    .D(mix[3]),
    .CE(tick),
    .CLR(rst),
    .Q(out$q3)
  );
endmodule
