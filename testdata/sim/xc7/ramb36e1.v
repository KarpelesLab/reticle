`timescale 1ns / 1ps
// RAMB36E1 in true dual-port and simple dual-port shapes, as the RAMB18E1
// testbench drives it: pseudo-random inputs over two small address ranges
// (bit 14 random too), so that what one port writes the other reads back,
// and the two clocks half a period apart.
module tb;
  reg ca = 1'b0, cb = 1'b0;
  reg [63:0] lfsr = 64'hfeed_c0de_1357_9bdf;
  reg [63:0] more = 64'h2468_ace0_1122_3344;
  wire [15:0] addra = {1'b0, lfsr[7], 7'b0, lfsr[6:0]};
  wire [15:0] addrb = {1'b0, lfsr[15], 7'b0, lfsr[14:8]};
  wire [31:0] dia = {more[15:0], lfsr[31:16]}, dib = {more[31:16], lfsr[47:32]};
  wire [3:0] dipa = more[35:32], dipb = more[39:36];
  wire ena = lfsr[50] | lfsr[51], enb = lfsr[52] | lfsr[53];
  wire [3:0] wea = {4{lfsr[54] & lfsr[55]}} & {lfsr[56] | lfsr[57], lfsr[58] | lfsr[57], more[40] | more[41], more[42] | more[43]};
  wire [7:0] web = {8{lfsr[59] & lfsr[60]}} & {lfsr[61], lfsr[62], lfsr[63], lfsr[61] | lfsr[0], more[44], more[45], more[46] | more[47], more[48] | more[49]};
  wire rstrama = lfsr[1] & lfsr[2] & lfsr[3] & lfsr[4];
  wire rstramb = lfsr[5] & lfsr[6] & lfsr[8] & lfsr[9];
  wire rstrega = lfsr[10] & lfsr[11] & lfsr[12] & lfsr[13];
  wire rstregb = lfsr[14] & lfsr[15] & lfsr[16] & lfsr[17];
  wire regcea = lfsr[18] | lfsr[19], regceb = lfsr[20] | lfsr[21];

  wire [31:0] a0, b0, a1, b1, a2, b2, a3, b3;
  wire [3:0] pa0, pb0, pa1, pb1, pa2, pb2, pa3, pb3;

  // A 36 bits write-first, B 18 bits read-first registered.
  RAMB36E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(36), .WRITE_WIDTH_A(36), .READ_WIDTH_B(18),
    .WRITE_WIDTH_B(18), .WRITE_MODE_A("WRITE_FIRST"), .WRITE_MODE_B("READ_FIRST"),
    .DOA_REG(0), .DOB_REG(1), .INIT_A(36'h9_a5a5_5a5a), .INIT_B(36'h2_0f0f), .SRVAL_A(36'h6_3c3c_c3c3),
    .SRVAL_B(36'h1_f0f0),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff),
    .INIT_40(256'hfedcba98765432100123456789abcdef_f0e1d2c3b4a5968778695a4b3c2d1e0f),
    .INITP_00(256'h9c3e_a5f0_0f5a_e3c1), .INITP_08(256'h1234_5678_9abc_def0))
    u0 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a0), .DOBDO(b0),
    .DOPADOP(pa0), .DOPBDOP(pb0), .CASCADEINA(1'b0), .CASCADEINB(1'b0),
    .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0));

  // A 9 bits no-change registered with REGCE before RSTREG, B 1 bit.
  RAMB36E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(9), .WRITE_WIDTH_A(9), .READ_WIDTH_B(1),
    .WRITE_WIDTH_B(1), .WRITE_MODE_A("NO_CHANGE"), .WRITE_MODE_B("WRITE_FIRST"),
    .DOA_REG(1), .RSTREG_PRIORITY_A("REGCE"), .SRVAL_A(36'h155), .INIT_A(36'h0aa),
    .INIT_00(256'hdeadbeef_cafef00d_12345678_9abcdef0_0badc0de_feedface_8badf00d_a5a5a5a5),
    .INIT_40(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff),
    .INITP_00(256'hffff_0000_aaaa_5555), .INITP_08(256'h0f0f_f0f0_3333_cccc))
    u1 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a1), .DOBDO(b1),
    .DOPADOP(pa1), .DOPBDOP(pb1), .CASCADEINA(1'b0), .CASCADEINB(1'b0),
    .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0));

  // Simple dual port, 72 bits each way, read port registered.
  RAMB36E1 #(.RAM_MODE("SDP"), .READ_WIDTH_A(72), .WRITE_WIDTH_B(72), .DOA_REG(1),
    .INIT_A(36'h1_2345_6789), .INIT_B(36'h2_abcd_ef01), .SRVAL_A(36'hf_ffff_0000),
    .SRVAL_B(36'h0_0000_ffff),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff),
    .INIT_01(256'hfedcba98765432100123456789abcdef_f0e1d2c3b4a5968778695a4b3c2d1e0f),
    .INIT_40(256'h0123456789abcdef_fedcba9876543210_0011223344556677_8899aabbccddeeff),
    .INITP_00(256'h1234_5678_9abc_def0_0fed_cba9_8765_4321), .INITP_08(256'h5a5a_a5a5))
    u2 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(1'b0), .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0), .RSTREGARSTREG(rstrega),
    .RSTREGB(1'b0), .ADDRARDADDR({1'b0, addra[14], addra[7:0], 6'b0}),
    .ADDRBWRADDR({1'b0, addrb[14], addrb[7:0], 6'b0}),
    .DIADI(dia), .DIBDI(dib), .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(4'b0), .WEBWE(web),
    .DOADO(a2), .DOBDO(b2), .DOPADOP(pa2), .DOPBDOP(pb2), .CASCADEINA(1'b0),
    .CASCADEINB(1'b0), .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0));

  // True dual port, A writes 2 and reads 4, B writes 36 and reads 9.
  RAMB36E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(4), .WRITE_WIDTH_A(2), .READ_WIDTH_B(9),
    .WRITE_WIDTH_B(36), .WRITE_MODE_A("READ_FIRST"), .WRITE_MODE_B("WRITE_FIRST"),
    .DOB_REG(1), .INIT_B(36'h1ff), .SRVAL_B(36'h0ff),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff))
    u3 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a3), .DOBDO(b3),
    .DOPADOP(pa3), .DOPBDOP(pb3), .CASCADEINA(1'b0), .CASCADEINB(1'b0),
    .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0));

  integer i;
  initial begin
    #80;
    for (i = 0; i < 400; i = i + 1) begin
      #1 lfsr = {lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]};
      more = {more[62:0], more[63] ^ more[62] ^ more[60] ^ more[59]};
      #1.5 ca = 1'b1;
      #1 $display("%0d a %h %b %b %b %h %h %b %b", i, a0, pa0, a1, pa1, a2, pa2, a3, pa3);
      #1.5 ca = 1'b0;
      #1 lfsr = {lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]};
      more = {more[62:0], more[63] ^ more[62] ^ more[60] ^ more[59]};
      #1.5 cb = 1'b1;
      #1 $display("%0d b %h %b %b %b %h %h %b %b", i, b0, pb0, b1, pb1, b2, pb2, b3, pb3);
      #1.5 cb = 1'b0;
    end
    $finish;
  end
endmodule
