module ram16 (.clk(clk$pad), .we(we$pad), .waddr(waddr$pad), .raddr(raddr$pad), .wdata(wdata$pad), .rdata(rdata$pad));
  input clk$pad;
  input we$pad;
  input [3:0] waddr$pad;
  input [3:0] raddr$pad;
  input [7:0] wdata$pad;
  output [7:0] rdata$pad;
  wire clk;
  wire we;
  wire [3:0] waddr;
  wire [3:0] raddr;
  wire [7:0] wdata;
  wire [7:0] rdata;
  wire mem$rd0_r0_b0_0;
  wire mem$rd0_r0_b1_0;
  wire mem$rd0_r0_b2_0;
  wire mem$rd0_r0_b3_0;
  wire mem$rd0_r0_b4_0;
  wire mem$rd0_r0_b5_0;
  wire mem$rd0_r0_b6_0;
  wire mem$rd0_r0_b7_0;
  wire clk$pad;
  wire clk$in0;
  wire we$pad;
  wire we$in0;
  wire [3:0] waddr$pad;
  wire waddr$in0;
  wire waddr$in1;
  wire waddr$in2;
  wire waddr$in3;
  wire [3:0] raddr$pad;
  wire raddr$in0;
  wire raddr$in1;
  wire raddr$in2;
  wire raddr$in3;
  wire [7:0] wdata$pad;
  wire wdata$in0;
  wire wdata$in1;
  wire wdata$in2;
  wire wdata$in3;
  wire wdata$in4;
  wire wdata$in5;
  wire wdata$in6;
  wire wdata$in7;
  wire [7:0] rdata$pad;
  wire rdata$pin0;
  wire rdata$pin1;
  wire rdata$pin2;
  wire rdata$pin3;
  wire rdata$pin4;
  wire rdata$pin5;
  wire rdata$pin6;
  wire rdata$pin7;
  assign clk = clk$in0;
  assign we = we$in0;
  assign waddr = {waddr$in3, waddr$in2, waddr$in1, waddr$in0};
  assign raddr = {raddr$in3, raddr$in2, raddr$in1, raddr$in0};
  assign wdata = {wdata$in7, wdata$in6, wdata$in5, wdata$in4, wdata$in3, wdata$in2, wdata$in1, wdata$in0};
  assign rdata = {mem$rd0_r0_b7_0, mem$rd0_r0_b6_0, mem$rd0_r0_b5_0, mem$rd0_r0_b4_0, mem$rd0_r0_b3_0, mem$rd0_r0_b2_0, mem$rd0_r0_b1_0, mem$rd0_r0_b0_0};
  assign rdata$pad = {rdata$pin7, rdata$pin6, rdata$pin5, rdata$pin4, rdata$pin3, rdata$pin2, rdata$pin1, rdata$pin0};
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_0 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[0]),
    .DPO(mem$rd0_r0_b0_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_1 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[1]),
    .DPO(mem$rd0_r0_b1_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_2 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[2]),
    .DPO(mem$rd0_r0_b2_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_3 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[3]),
    .DPO(mem$rd0_r0_b3_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_4 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[4]),
    .DPO(mem$rd0_r0_b4_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_5 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[5]),
    .DPO(mem$rd0_r0_b5_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_6 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[6]),
    .DPO(mem$rd0_r0_b6_0)
  );
  (* memory = "mem" *)
  RAM64X1D mem$dpr0_0_7 (
    .WCLK(clk),
    .WE(we),
    .A0(waddr[0]),
    .A1(waddr[1]),
    .A2(waddr[2]),
    .A3(waddr[3]),
    .A4(1'b0),
    .A5(1'b0),
    .DPRA0(raddr[0]),
    .DPRA1(raddr[1]),
    .DPRA2(raddr[2]),
    .DPRA3(raddr[3]),
    .DPRA4(1'b0),
    .DPRA5(1'b0),
    .D(wdata[7]),
    .DPO(mem$rd0_r0_b7_0)
  );
  (* port = "clk", pin = "W5", io_standard = "LVCMOS33" *)
  IBUF clk$io0 (
    .I(clk$pad),
    .O(clk$in0)
  );
  (* port = "we", pin = "V17", io_standard = "LVCMOS33" *)
  IBUF we$io0 (
    .I(we$pad),
    .O(we$in0)
  );
  (* port = "waddr" *)
  IBUF waddr$io0 (
    .I(waddr$pad[0]),
    .O(waddr$in0)
  );
  (* port = "waddr" *)
  IBUF waddr$io1 (
    .I(waddr$pad[1]),
    .O(waddr$in1)
  );
  (* port = "waddr" *)
  IBUF waddr$io2 (
    .I(waddr$pad[2]),
    .O(waddr$in2)
  );
  (* port = "waddr" *)
  IBUF waddr$io3 (
    .I(waddr$pad[3]),
    .O(waddr$in3)
  );
  (* port = "raddr" *)
  IBUF raddr$io0 (
    .I(raddr$pad[0]),
    .O(raddr$in0)
  );
  (* port = "raddr" *)
  IBUF raddr$io1 (
    .I(raddr$pad[1]),
    .O(raddr$in1)
  );
  (* port = "raddr" *)
  IBUF raddr$io2 (
    .I(raddr$pad[2]),
    .O(raddr$in2)
  );
  (* port = "raddr" *)
  IBUF raddr$io3 (
    .I(raddr$pad[3]),
    .O(raddr$in3)
  );
  (* port = "wdata" *)
  IBUF wdata$io0 (
    .I(wdata$pad[0]),
    .O(wdata$in0)
  );
  (* port = "wdata" *)
  IBUF wdata$io1 (
    .I(wdata$pad[1]),
    .O(wdata$in1)
  );
  (* port = "wdata" *)
  IBUF wdata$io2 (
    .I(wdata$pad[2]),
    .O(wdata$in2)
  );
  (* port = "wdata" *)
  IBUF wdata$io3 (
    .I(wdata$pad[3]),
    .O(wdata$in3)
  );
  (* port = "wdata" *)
  IBUF wdata$io4 (
    .I(wdata$pad[4]),
    .O(wdata$in4)
  );
  (* port = "wdata" *)
  IBUF wdata$io5 (
    .I(wdata$pad[5]),
    .O(wdata$in5)
  );
  (* port = "wdata" *)
  IBUF wdata$io6 (
    .I(wdata$pad[6]),
    .O(wdata$in6)
  );
  (* port = "wdata" *)
  IBUF wdata$io7 (
    .I(wdata$pad[7]),
    .O(wdata$in7)
  );
  (* port = "rdata" *)
  OBUF rdata$io0 (
    .I(rdata[0]),
    .O(rdata$pin0)
  );
  (* port = "rdata" *)
  OBUF rdata$io1 (
    .I(rdata[1]),
    .O(rdata$pin1)
  );
  (* port = "rdata" *)
  OBUF rdata$io2 (
    .I(rdata[2]),
    .O(rdata$pin2)
  );
  (* port = "rdata" *)
  OBUF rdata$io3 (
    .I(rdata[3]),
    .O(rdata$pin3)
  );
  (* port = "rdata" *)
  OBUF rdata$io4 (
    .I(rdata[4]),
    .O(rdata$pin4)
  );
  (* port = "rdata" *)
  OBUF rdata$io5 (
    .I(rdata[5]),
    .O(rdata$pin5)
  );
  (* port = "rdata" *)
  OBUF rdata$io6 (
    .I(rdata[6]),
    .O(rdata$pin6)
  );
  (* port = "rdata" *)
  OBUF rdata$io7 (
    .I(rdata[7]),
    .O(rdata$pin7)
  );
endmodule
