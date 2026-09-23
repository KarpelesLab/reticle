module ram1k (.clk(clk$pad), .we(we$pad), .waddr(waddr$pad), .raddr(raddr$pad), .wdata(wdata$pad), .rdata(rdata$pad));
  input clk$pad;
  input we$pad;
  input [9:0] waddr$pad;
  input [9:0] raddr$pad;
  input [15:0] wdata$pad;
  output [15:0] rdata$pad;
  wire clk;
  wire we;
  wire [9:0] waddr;
  wire [9:0] raddr;
  wire [15:0] wdata;
  wire [15:0] rdata;
  wire [15:0] mem$rd0_w0_d0;
  wire clk$pad;
  wire clk$in0;
  wire we$pad;
  wire we$in0;
  wire [9:0] waddr$pad;
  wire waddr$in0;
  wire waddr$in1;
  wire waddr$in2;
  wire waddr$in3;
  wire waddr$in4;
  wire waddr$in5;
  wire waddr$in6;
  wire waddr$in7;
  wire waddr$in8;
  wire waddr$in9;
  wire [9:0] raddr$pad;
  wire raddr$in0;
  wire raddr$in1;
  wire raddr$in2;
  wire raddr$in3;
  wire raddr$in4;
  wire raddr$in5;
  wire raddr$in6;
  wire raddr$in7;
  wire raddr$in8;
  wire raddr$in9;
  wire [15:0] wdata$pad;
  wire wdata$in0;
  wire wdata$in1;
  wire wdata$in2;
  wire wdata$in3;
  wire wdata$in4;
  wire wdata$in5;
  wire wdata$in6;
  wire wdata$in7;
  wire wdata$in8;
  wire wdata$in9;
  wire wdata$in10;
  wire wdata$in11;
  wire wdata$in12;
  wire wdata$in13;
  wire wdata$in14;
  wire wdata$in15;
  wire [15:0] rdata$pad;
  wire rdata$pin0;
  wire rdata$pin1;
  wire rdata$pin2;
  wire rdata$pin3;
  wire rdata$pin4;
  wire rdata$pin5;
  wire rdata$pin6;
  wire rdata$pin7;
  wire rdata$pin8;
  wire rdata$pin9;
  wire rdata$pin10;
  wire rdata$pin11;
  wire rdata$pin12;
  wire rdata$pin13;
  wire rdata$pin14;
  wire rdata$pin15;
  assign clk = clk$in0;
  assign we = we$in0;
  assign waddr = {waddr$in9, waddr$in8, waddr$in7, waddr$in6, waddr$in5, waddr$in4, waddr$in3, waddr$in2, waddr$in1, waddr$in0};
  assign raddr = {raddr$in9, raddr$in8, raddr$in7, raddr$in6, raddr$in5, raddr$in4, raddr$in3, raddr$in2, raddr$in1, raddr$in0};
  assign wdata = {wdata$in15, wdata$in14, wdata$in13, wdata$in12, wdata$in11, wdata$in10, wdata$in9, wdata$in8, wdata$in7, wdata$in6, wdata$in5, wdata$in4, wdata$in3, wdata$in2, wdata$in1, wdata$in0};
  assign rdata = {mem$rd0_w0_d0[15], mem$rd0_w0_d0[14], mem$rd0_w0_d0[13], mem$rd0_w0_d0[12], mem$rd0_w0_d0[11], mem$rd0_w0_d0[10], mem$rd0_w0_d0[9], mem$rd0_w0_d0[8], mem$rd0_w0_d0[7], mem$rd0_w0_d0[6], mem$rd0_w0_d0[5], mem$rd0_w0_d0[4], mem$rd0_w0_d0[3], mem$rd0_w0_d0[2], mem$rd0_w0_d0[1], mem$rd0_w0_d0[0]};
  assign rdata$pad = {rdata$pin15, rdata$pin14, rdata$pin13, rdata$pin12, rdata$pin11, rdata$pin10, rdata$pin9, rdata$pin8, rdata$pin7, rdata$pin6, rdata$pin5, rdata$pin4, rdata$pin3, rdata$pin2, rdata$pin1, rdata$pin0};
  (* memory = "mem" *)
  RAMB18E1 #(.READ_WIDTH_A(18), .WRITE_WIDTH_A(18), .READ_WIDTH_B(18), .WRITE_WIDTH_B(18), .RAM_MODE("TDP"), .DOA_REG(0), .DOB_REG(0), .SIM_DEVICE("7SERIES"), .INIT_00(256'h000000000000000000000000000000000000000080000001beefdeadbabecafe), .INIT_01(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_02(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_03(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_04(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_05(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_06(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_07(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_08(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_09(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0A(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0B(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0C(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0D(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0E(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_0F(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_10(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_11(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_12(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_13(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_14(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_15(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_16(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_17(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_18(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_19(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1A(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1B(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1C(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1D(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1E(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_1F(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_20(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_21(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_22(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_23(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_24(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_25(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_26(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_27(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_28(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_29(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2A(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2B(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2C(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2D(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2E(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_2F(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_30(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_31(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_32(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_33(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_34(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_35(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_36(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_37(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_38(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_39(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3A(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3B(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3C(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3D(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3E(256'h0000000000000000000000000000000000000000000000000000000000000000), .INIT_3F(256'h0000000000000000000000000000000000000000000000000000000000000000)) mem$ram_w0_d0 (
    .ADDRARDADDR({raddr, 4'h0}),
    .CLKARDCLK(clk),
    .ENARDEN(1'b1),
    .ADDRBWRADDR({waddr, 4'h0}),
    .CLKBWRCLK(clk),
    .ENBWREN(we),
    .WEBWE({we, we, we, we}),
    .DIBDI(wdata),
    .DOADO(mem$rd0_w0_d0)
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
  (* port = "waddr" *)
  IBUF waddr$io4 (
    .I(waddr$pad[4]),
    .O(waddr$in4)
  );
  (* port = "waddr" *)
  IBUF waddr$io5 (
    .I(waddr$pad[5]),
    .O(waddr$in5)
  );
  (* port = "waddr" *)
  IBUF waddr$io6 (
    .I(waddr$pad[6]),
    .O(waddr$in6)
  );
  (* port = "waddr" *)
  IBUF waddr$io7 (
    .I(waddr$pad[7]),
    .O(waddr$in7)
  );
  (* port = "waddr" *)
  IBUF waddr$io8 (
    .I(waddr$pad[8]),
    .O(waddr$in8)
  );
  (* port = "waddr" *)
  IBUF waddr$io9 (
    .I(waddr$pad[9]),
    .O(waddr$in9)
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
  (* port = "raddr" *)
  IBUF raddr$io4 (
    .I(raddr$pad[4]),
    .O(raddr$in4)
  );
  (* port = "raddr" *)
  IBUF raddr$io5 (
    .I(raddr$pad[5]),
    .O(raddr$in5)
  );
  (* port = "raddr" *)
  IBUF raddr$io6 (
    .I(raddr$pad[6]),
    .O(raddr$in6)
  );
  (* port = "raddr" *)
  IBUF raddr$io7 (
    .I(raddr$pad[7]),
    .O(raddr$in7)
  );
  (* port = "raddr" *)
  IBUF raddr$io8 (
    .I(raddr$pad[8]),
    .O(raddr$in8)
  );
  (* port = "raddr" *)
  IBUF raddr$io9 (
    .I(raddr$pad[9]),
    .O(raddr$in9)
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
  (* port = "wdata" *)
  IBUF wdata$io8 (
    .I(wdata$pad[8]),
    .O(wdata$in8)
  );
  (* port = "wdata" *)
  IBUF wdata$io9 (
    .I(wdata$pad[9]),
    .O(wdata$in9)
  );
  (* port = "wdata" *)
  IBUF wdata$io10 (
    .I(wdata$pad[10]),
    .O(wdata$in10)
  );
  (* port = "wdata" *)
  IBUF wdata$io11 (
    .I(wdata$pad[11]),
    .O(wdata$in11)
  );
  (* port = "wdata" *)
  IBUF wdata$io12 (
    .I(wdata$pad[12]),
    .O(wdata$in12)
  );
  (* port = "wdata" *)
  IBUF wdata$io13 (
    .I(wdata$pad[13]),
    .O(wdata$in13)
  );
  (* port = "wdata" *)
  IBUF wdata$io14 (
    .I(wdata$pad[14]),
    .O(wdata$in14)
  );
  (* port = "wdata" *)
  IBUF wdata$io15 (
    .I(wdata$pad[15]),
    .O(wdata$in15)
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
  (* port = "rdata" *)
  OBUF rdata$io8 (
    .I(rdata[8]),
    .O(rdata$pin8)
  );
  (* port = "rdata" *)
  OBUF rdata$io9 (
    .I(rdata[9]),
    .O(rdata$pin9)
  );
  (* port = "rdata" *)
  OBUF rdata$io10 (
    .I(rdata[10]),
    .O(rdata$pin10)
  );
  (* port = "rdata" *)
  OBUF rdata$io11 (
    .I(rdata[11]),
    .O(rdata$pin11)
  );
  (* port = "rdata" *)
  OBUF rdata$io12 (
    .I(rdata[12]),
    .O(rdata$pin12)
  );
  (* port = "rdata" *)
  OBUF rdata$io13 (
    .I(rdata[13]),
    .O(rdata$pin13)
  );
  (* port = "rdata" *)
  OBUF rdata$io14 (
    .I(rdata[14]),
    .O(rdata$pin14)
  );
  (* port = "rdata" *)
  OBUF rdata$io15 (
    .I(rdata[15]),
    .O(rdata$pin15)
  );
endmodule
