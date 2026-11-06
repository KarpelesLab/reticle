`timescale 1ns / 1ps
// RAMB18E1 in true dual-port and simple dual-port shapes, each port's
// widths and write modes, the output registers and both resets, driven by
// pseudo-random inputs over a small address range so that what one port
// writes the other reads back. The two clocks are half a period apart, so
// no read meets a write to the same address on the same edge.
module tb;
  reg ca = 1'b0, cb = 1'b0;
  reg [63:0] lfsr = 64'h0123_4567_89ab_cdef;
  // Address: bits 3..0 random, bits 6..4 random, the rest zero.
  wire [13:0] addra = {7'b0, lfsr[6:0]};
  wire [13:0] addrb = {7'b0, lfsr[13:7]};
  wire [15:0] dia = lfsr[29:14], dib = lfsr[45:30];
  wire [1:0] dipa = lfsr[47:46], dipb = lfsr[49:48];
  wire ena = lfsr[50] | lfsr[51], enb = lfsr[52] | lfsr[53];
  wire [1:0] wea = {2{lfsr[54] & lfsr[55]}} & {lfsr[56] | lfsr[57], lfsr[58] | lfsr[57]};
  wire [3:0] web = {4{lfsr[59] & lfsr[60]}} & {lfsr[61], lfsr[62], lfsr[63], lfsr[61] | lfsr[0]};
  wire rstrama = lfsr[1] & lfsr[2] & lfsr[3] & lfsr[4];
  wire rstramb = lfsr[5] & lfsr[6] & lfsr[8] & lfsr[9];
  wire rstrega = lfsr[10] & lfsr[11] & lfsr[12] & lfsr[13];
  wire rstregb = lfsr[14] & lfsr[15] & lfsr[16] & lfsr[17];
  wire regcea = lfsr[18] | lfsr[19], regceb = lfsr[20] | lfsr[21];

  wire [15:0] a0, b0, a1, b1, a2, b2, a3, b3, a4, b4;
  wire [1:0] pa0, pb0, pa1, pb1, pa2, pb2, pa3, pb3, pa4, pb4;

  // A 18 bits write-first unregistered, B 9 bits read-first registered.
  RAMB18E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(18), .WRITE_WIDTH_A(18), .READ_WIDTH_B(9),
    .WRITE_WIDTH_B(9), .WRITE_MODE_A("WRITE_FIRST"), .WRITE_MODE_B("READ_FIRST"),
    .DOA_REG(0), .DOB_REG(1), .INIT_A(18'h2a5a5), .INIT_B(18'h15a5a), .SRVAL_A(18'h3c3c3),
    .SRVAL_B(18'h0f0f0),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff),
    .INITP_00(256'h9c3e_a5f0_0f5a_e3c1))
    u0 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a0), .DOBDO(b0),
    .DOPADOP(pa0), .DOPBDOP(pb0));

  // Mixed widths on one port: A writes 4 and reads 2, no change; B writes 18
  // and reads 1, write first, registered with REGCE before RSTREG.
  RAMB18E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(2), .WRITE_WIDTH_A(4), .READ_WIDTH_B(1),
    .WRITE_WIDTH_B(18), .WRITE_MODE_A("NO_CHANGE"), .WRITE_MODE_B("WRITE_FIRST"),
    .DOA_REG(1), .DOB_REG(1), .RSTREG_PRIORITY_B("REGCE"), .SRVAL_A(18'h00003),
    .SRVAL_B(18'h00001), .INIT_A(18'h00002),
    .INIT_00(256'hdeadbeef_cafef00d_12345678_9abcdef0_0badc0de_feedface_8badf00d_a5a5a5a5))
    u1 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a1), .DOBDO(b1),
    .DOPADOP(pa1), .DOPBDOP(pb1));

  // Simple dual port, 36 bits each way, read port registered.
  RAMB18E1 #(.RAM_MODE("SDP"), .READ_WIDTH_A(36), .WRITE_WIDTH_B(36), .DOA_REG(1),
    .INIT_A(18'h12345), .INIT_B(18'h2abcd), .SRVAL_A(18'h3ffff), .SRVAL_B(18'h00000),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff),
    .INIT_01(256'hfedcba98765432100123456789abcdef_f0e1d2c3b4a5968778695a4b3c2d1e0f),
    .INITP_00(256'h1234_5678_9abc_def0_0fed_cba9_8765_4321))
    u2 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(1'b0), .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0), .RSTREGARSTREG(rstrega),
    .RSTREGB(1'b0), .ADDRARDADDR({addra[8:0], 5'b0}), .ADDRBWRADDR({addrb[8:0], 5'b0}),
    .DIADI(dia), .DIBDI(dib), .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(2'b00), .WEBWE(web),
    .DOADO(a2), .DOBDO(b2), .DOPADOP(pa2), .DOPBDOP(pb2));

  // Simple dual port reading 36 bits unregistered, with inverted clocks,
  // enables and resets.
  RAMB18E1 #(.RAM_MODE("SDP"), .READ_WIDTH_A(36), .WRITE_WIDTH_B(36), .DOA_REG(0),
    .IS_CLKARDCLK_INVERTED(1'b1), .IS_ENARDEN_INVERTED(1'b1), .IS_RSTRAMARSTRAM_INVERTED(1'b1),
    .IS_ENBWREN_INVERTED(1'b1), .SRVAL_A(18'h15555), .SRVAL_B(18'h2aaaa),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff))
    u3 (.CLKARDCLK(~ca), .CLKBWRCLK(cb), .ENARDEN(~ena), .ENBWREN(~enb), .REGCEAREGCE(regcea),
    .REGCEB(1'b0), .RSTRAMARSTRAM(~rstrama), .RSTRAMB(1'b0), .RSTREGARSTREG(1'b0),
    .RSTREGB(1'b0), .ADDRARDADDR({addra[8:0], 5'b0}), .ADDRBWRADDR({addrb[8:0], 5'b0}),
    .DIADI(dia), .DIBDI(dib), .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(2'b00), .WEBWE(web),
    .DOADO(a3), .DOBDO(b3), .DOPADOP(pa3), .DOPBDOP(pb3));

  // True dual port 1 bit on A and 4 bits on B, both read first.
  RAMB18E1 #(.RAM_MODE("TDP"), .READ_WIDTH_A(1), .WRITE_WIDTH_A(1), .READ_WIDTH_B(4),
    .WRITE_WIDTH_B(4), .WRITE_MODE_A("READ_FIRST"), .WRITE_MODE_B("NO_CHANGE"),
    .INIT_00(256'h0f1e2d3c4b5a69788796a5b4c3d2e1f0_00112233445566778899aabbccddeeff))
    u4 (.CLKARDCLK(ca), .CLKBWRCLK(cb), .ENARDEN(ena), .ENBWREN(enb), .REGCEAREGCE(regcea),
    .REGCEB(regceb), .RSTRAMARSTRAM(rstrama), .RSTRAMB(rstramb), .RSTREGARSTREG(rstrega),
    .RSTREGB(rstregb), .ADDRARDADDR(addra), .ADDRBWRADDR(addrb), .DIADI(dia), .DIBDI(dib),
    .DIPADIP(dipa), .DIPBDIP(dipb), .WEA(wea), .WEBWE(web), .DOADO(a4), .DOBDO(b4),
    .DOPADOP(pa4), .DOPBDOP(pb4));

  integer i;
  initial begin
    #80;
    for (i = 0; i < 400; i = i + 1) begin
      #1 lfsr = {lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]};
      #1.5 ca = 1'b1;
      #1 $display("%0d a %h %b %b %b %h %h %h %h %b %b", i, a0, pa0, a1, pa1, a2, pa2, a3, pa3, a4, pa4);
      #1.5 ca = 1'b0;
      #1 lfsr = {lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]};
      #1.5 cb = 1'b1;
      #1 $display("%0d b %h %b %b %b %h %h %h %h %b %b", i, b0, pb0, b1, pb1, b2, pb2, b3, pb3, b4, pb4);
      #1.5 cb = 1'b0;
    end
    $finish;
  end
endmodule
