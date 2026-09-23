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
  wire \$lut4 ;
  wire \$lut5 ;
  wire \$lut6 ;
  wire \$lut7 ;
  wire \$lut8 ;
  wire \$lut9 ;
  wire \$lut10 ;
  wire \$lut11 ;
  wire \$lut12 ;
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
  function [0:0] reticle_bits_3_0_0;
    input [2:0] x;
    reticle_bits_3_0_0 = x[0:0];
  endfunction
  function [0:0] reticle_bits_3_1_1;
    input [2:0] x;
    reticle_bits_3_1_1 = x[1:1];
  endfunction
  function [0:0] reticle_bits_3_2_2;
    input [2:0] x;
    reticle_bits_3_2_2 = x[2:2];
  endfunction
  function [0:0] reticle_bits_4_0_0;
    input [3:0] x;
    reticle_bits_4_0_0 = x[0:0];
  endfunction
  function [0:0] reticle_bits_4_1_1;
    input [3:0] x;
    reticle_bits_4_1_1 = x[1:1];
  endfunction
  function [0:0] reticle_bits_4_2_2;
    input [3:0] x;
    reticle_bits_4_2_2 = x[2:2];
  endfunction
  function [0:0] reticle_bits_4_3_3;
    input [3:0] x;
    reticle_bits_4_3_3 = x[3:3];
  endfunction
  function [0:0] reticle_bits_5_0_0;
    input [4:0] x;
    reticle_bits_5_0_0 = x[0:0];
  endfunction
  function [0:0] reticle_bits_5_1_1;
    input [4:0] x;
    reticle_bits_5_1_1 = x[1:1];
  endfunction
  function [0:0] reticle_bits_5_2_2;
    input [4:0] x;
    reticle_bits_5_2_2 = x[2:2];
  endfunction
  function [0:0] reticle_bits_5_3_3;
    input [4:0] x;
    reticle_bits_5_3_3 = x[3:3];
  endfunction
  function [0:0] reticle_bits_5_4_4;
    input [4:0] x;
    reticle_bits_5_4_4 = x[4:4];
  endfunction
  function [0:0] reticle_bits_6_0_0;
    input [5:0] x;
    reticle_bits_6_0_0 = x[0:0];
  endfunction
  function [0:0] reticle_bits_6_1_1;
    input [5:0] x;
    reticle_bits_6_1_1 = x[1:1];
  endfunction
  function [0:0] reticle_bits_6_2_2;
    input [5:0] x;
    reticle_bits_6_2_2 = x[2:2];
  endfunction
  function [0:0] reticle_bits_6_3_3;
    input [5:0] x;
    reticle_bits_6_3_3 = x[3:3];
  endfunction
  function [0:0] reticle_bits_6_4_4;
    input [5:0] x;
    reticle_bits_6_4_4 = x[4:4];
  endfunction
  function [0:0] reticle_bits_6_5_5;
    input [5:0] x;
    reticle_bits_6_5_5 = x[5:5];
  endfunction
  assign clk = clk$in0;
  assign rst = rst$in0;
  assign next = {\$lut5 , \$lut6 , \$lut7 , \$lut8 , \$lut9 , \$lut10 , \$lut11 , \$lut12 };
  assign tick = count[0];
  assign mix = {\$lut0 , \$lut1 , \$lut2 , \$lut3 };
  assign led$pad = {led$pin3, led$pin2, led$pin1, led$pin0};
  assign count = {counter$q7, counter$q6, counter$q5, counter$q4, counter$q3, counter$q2, counter$q1, counter$q0};
  assign led = {out$q3, out$q2, out$q1, out$q0};
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
  (* port = "sw" *)
  IBUF sw$io0 (
    .I(sw$pad[0]),
    .O(sw$in0)
  );
  (* port = "sw" *)
  IBUF sw$io1 (
    .I(sw$pad[1]),
    .O(sw$in1)
  );
  (* port = "sw" *)
  IBUF sw$io2 (
    .I(sw$pad[2]),
    .O(sw$in2)
  );
  (* port = "sw" *)
  IBUF sw$io3 (
    .I(sw$pad[3]),
    .O(sw$in3)
  );
  (* port = "led" *)
  OBUF led$io0 (
    .I(led[0]),
    .O(led$pin0)
  );
  (* port = "led" *)
  OBUF led$io1 (
    .I(led[1]),
    .O(led$pin1)
  );
  (* port = "led" *)
  OBUF led$io2 (
    .I(led[2]),
    .O(led$pin2)
  );
  (* port = "led" *)
  OBUF led$io3 (
    .I(led[3]),
    .O(led$pin3)
  );
  BUFG clk$gbuf (
    .I(clk),
    .O(clk$gb)
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut13  (
    .I0(reticle_bits_2_0_0({sw$in3, count[7]})),
    .I1(reticle_bits_2_1_1({sw$in3, count[7]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut0 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut14  (
    .I0(reticle_bits_2_0_0({sw$in2, count[6]})),
    .I1(reticle_bits_2_1_1({sw$in2, count[6]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut1 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut15  (
    .I0(reticle_bits_2_0_0({sw$in1, count[5]})),
    .I1(reticle_bits_2_1_1({sw$in1, count[5]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut2 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut16  (
    .I0(reticle_bits_2_0_0({sw$in0, count[4]})),
    .I1(reticle_bits_2_1_1({sw$in0, count[4]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut3 )
  );
  LUT6 #(.INIT(64'h8000000000000000)) \$lut17  (
    .I0(reticle_bits_6_0_0({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I1(reticle_bits_6_1_1({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I2(reticle_bits_6_2_2({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I3(reticle_bits_6_3_3({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I4(reticle_bits_6_4_4({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I5(reticle_bits_6_5_5({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .O(\$lut4 )
  );
  LUT6 #(.INIT(64'h6c6c6c6c6c6c6c6c)) \$lut18  (
    .I0(reticle_bits_3_0_0({\$lut4 , count[7], count[6]})),
    .I1(reticle_bits_3_1_1({\$lut4 , count[7], count[6]})),
    .I2(reticle_bits_3_2_2({\$lut4 , count[7], count[6]})),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut5 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut19  (
    .I0(reticle_bits_2_0_0({\$lut4 , count[6]})),
    .I1(reticle_bits_2_1_1({\$lut4 , count[6]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut6 )
  );
  LUT6 #(.INIT(64'h7fffffff80000000)) \$lut20  (
    .I0(reticle_bits_6_0_0({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I1(reticle_bits_6_1_1({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I2(reticle_bits_6_2_2({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I3(reticle_bits_6_3_3({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I4(reticle_bits_6_4_4({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .I5(reticle_bits_6_5_5({count[5], count[4], count[3], count[2], count[1], count[0]})),
    .O(\$lut7 )
  );
  LUT6 #(.INIT(64'h7fff80007fff8000)) \$lut21  (
    .I0(reticle_bits_5_0_0({count[4], count[3], count[2], count[1], count[0]})),
    .I1(reticle_bits_5_1_1({count[4], count[3], count[2], count[1], count[0]})),
    .I2(reticle_bits_5_2_2({count[4], count[3], count[2], count[1], count[0]})),
    .I3(reticle_bits_5_3_3({count[4], count[3], count[2], count[1], count[0]})),
    .I4(reticle_bits_5_4_4({count[4], count[3], count[2], count[1], count[0]})),
    .I5(1'b0),
    .O(\$lut8 )
  );
  LUT6 #(.INIT(64'h7f807f807f807f80)) \$lut22  (
    .I0(reticle_bits_4_0_0({count[3], count[2], count[1], count[0]})),
    .I1(reticle_bits_4_1_1({count[3], count[2], count[1], count[0]})),
    .I2(reticle_bits_4_2_2({count[3], count[2], count[1], count[0]})),
    .I3(reticle_bits_4_3_3({count[3], count[2], count[1], count[0]})),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut9 )
  );
  LUT6 #(.INIT(64'h7878787878787878)) \$lut23  (
    .I0(reticle_bits_3_0_0({count[2], count[1], count[0]})),
    .I1(reticle_bits_3_1_1({count[2], count[1], count[0]})),
    .I2(reticle_bits_3_2_2({count[2], count[1], count[0]})),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut10 )
  );
  LUT6 #(.INIT(64'h6666666666666666)) \$lut24  (
    .I0(reticle_bits_2_0_0({count[1], count[0]})),
    .I1(reticle_bits_2_1_1({count[1], count[0]})),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut11 )
  );
  LUT6 #(.INIT(64'h5555555555555555)) \$lut25  (
    .I0(count[0]),
    .I1(1'b0),
    .I2(1'b0),
    .I3(1'b0),
    .I4(1'b0),
    .I5(1'b0),
    .O(\$lut12 )
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
