// Every flip-flop position of two slices of each tile kind, and a block RAM of
// each size: what the logic location file says of each is the fixture.
module ll_probe (input wire clk, input wire si, output wire so);
    wire [32:0] q;
    assign q[0] = si;
    genvar i;
    generate for (i = 0; i < 32; i = i + 1) begin : g
        (* DONT_TOUCH = "TRUE" *) FDRE f (.C(clk), .CE(1'b1), .R(1'b0), .D(q[i]), .Q(q[i + 1]));
    end endgenerate
    wire [31:0] m18, m36;
    (* DONT_TOUCH = "TRUE" *) RAMB18E1 #(.READ_WIDTH_A(18), .WRITE_WIDTH_A(18), .INIT_00(256'h1), .INITP_00(256'h1)) r18 (
        .CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(1'b1), .ENBWREN(1'b0),
        .RSTRAMARSTRAM(1'b0), .RSTREGARSTREG(1'b0), .RSTRAMB(1'b0), .RSTREGB(1'b0),
        .REGCEAREGCE(1'b1), .REGCEB(1'b0), .WEA(2'b0), .WEBWE(4'b0),
        .ADDRARDADDR({q[10:1], 4'b0}), .ADDRBWRADDR(14'b0), .DIADI(16'b0), .DIBDI(16'b0),
        .DIPADIP(2'b0), .DIPBDIP(2'b0), .DOADO(m18[15:0]));
    (* DONT_TOUCH = "TRUE" *) RAMB36E1 #(.READ_WIDTH_A(36), .WRITE_WIDTH_A(36), .INIT_00(256'h1), .INITP_00(256'h1)) r36 (
        .CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(1'b1), .ENBWREN(1'b0),
        .RSTRAMARSTRAM(1'b0), .RSTREGARSTREG(1'b0), .RSTRAMB(1'b0), .RSTREGB(1'b0),
        .REGCEAREGCE(1'b1), .REGCEB(1'b0), .WEA(4'b0), .WEBWE(8'b0),
        .ADDRARDADDR({1'b1, q[20:11], 5'b0}), .ADDRBWRADDR(16'h8000), .DIADI(32'b0), .DIBDI(32'b0),
        .DIPADIP(4'b0), .DIPBDIP(4'b0),
        .CASCADEINA(1'b0), .CASCADEINB(1'b0), .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0),
        .DOADO(m36));
    assign so = q[32] ^ m18[0] ^ m36[0];
endmodule
