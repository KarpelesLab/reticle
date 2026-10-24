// The cell models of xilinx_cells.v against what UG953 says the cells do:
// each flip-flop's power-up value, enable and set or reset; a LUT6's
// table, and its output staying known when an input its function ignores
// is x; a CARRY4 adding; a RAM64X1D's write and two reads; a RAMB18E1's
// write and read on its two ports; and an IOBUF both ways.
//
// Would catch a flip-flop ignoring its INIT or its control pins, a LUT
// indexed the wrong way round or turning every x into its own, a carry
// chain's sum or carry wrong, a RAM read from the wrong address or a
// write lost, or a bidirectional buffer that only drives; would not catch
// the models being wrong where the guide is silent (a read colliding with
// a write), and is no substitute for Vivado's unisims.
`timescale 1ps / 1ps
module xilinx_cells_tb;
  reg clk = 0;
  always #5000 clk = ~clk;
  integer errors = 0;
  task check(input ok, input [8*32-1:0] what);
    if (!ok) begin
      errors = errors + 1;
      $display("FAIL: %0s", what);
    end
  endtask

  // --- flip-flops.
  reg d = 0, ce = 0, sr = 0;
  wire q_re, q_re1, q_se, q_ce, q_pe;
  FDRE #(.INIT(1'b1)) fdre (.Q(q_re), .C(clk), .CE(ce), .R(sr), .D(d));
  FDRE_1 #(.INIT(1'b0)) fdre1 (.Q(q_re1), .C(clk), .CE(ce), .R(sr), .D(d));
  FDSE #(.INIT(1'b0)) fdse (.Q(q_se), .C(clk), .CE(ce), .S(sr), .D(d));
  FDCE #(.INIT(1'b1)) fdce (.Q(q_ce), .C(clk), .CE(ce), .CLR(sr), .D(d));
  FDPE #(.INIT(1'b0)) fdpe (.Q(q_pe), .C(clk), .CE(ce), .PRE(sr), .D(d));

  // --- LUT6: a two-input AND on I0 and I1 (INIT 8 repeated).
  reg [5:0] li = 0;
  wire lo;
  LUT6 #(.INIT(64'h8888888888888888)) lut (.O(lo), .I0(li[0]), .I1(li[1]), .I2(li[2]),
                                            .I3(li[3]), .I4(li[4]), .I5(li[5]));

  // --- CARRY4 as a four-bit adder: S = a ^ b, DI = a.
  reg [3:0] a = 0, b = 0;
  reg cin = 0;
  wire [3:0] sum, co;
  CARRY4 carry (.CO(co), .O(sum), .CI(1'b0), .CYINIT(cin), .DI(a), .S(a ^ b));

  // --- RAM64X1D.
  reg we = 0, wd = 0;
  reg [5:0] wa = 0, ra = 0;
  wire spo, dpo;
  RAM64X1D ram (.SPO(spo), .DPO(dpo), .WCLK(clk), .WE(we), .D(wd),
                .A0(wa[0]), .A1(wa[1]), .A2(wa[2]), .A3(wa[3]), .A4(wa[4]), .A5(wa[5]),
                .DPRA0(ra[0]), .DPRA1(ra[1]), .DPRA2(ra[2]), .DPRA3(ra[3]),
                .DPRA4(ra[4]), .DPRA5(ra[5]));

  // --- RAMB18E1: port B writes, port A reads, both 18 bits wide.
  reg en_a = 0, en_b = 0;
  reg [3:0] web = 0;
  reg [9:0] aa = 0, ab = 0;
  reg [15:0] dib = 0;
  reg [1:0] dipb = 0;
  wire [15:0] doa, dob;
  wire [1:0] dopa, dopb;
  RAMB18E1 bram (.CLKARDCLK(clk), .CLKBWRCLK(clk), .ENARDEN(en_a), .ENBWREN(en_b),
                 .ADDRARDADDR({aa, 4'h0}), .ADDRBWRADDR({ab, 4'h0}), .DIBDI(dib),
                 .DIPBDIP(dipb), .WEBWE(web), .DOADO(doa), .DOBDO(dob), .DOPADOP(dopa),
                 .DOPBDOP(dopb));

  // --- IOBUF on a line the testbench drives too.
  reg ext = 1'bz, t = 1, i = 0;
  wire line, o;
  assign line = ext;
  IOBUF iob (.O(o), .IO(line), .I(i), .T(t));

  initial begin
    #1;
    check(q_re === 1'b1 && q_re1 === 1'b0 && q_se === 1'b0 && q_ce === 1'b1 && q_pe === 1'b0,
          "flip-flop power-up values");
    d = 1;
    @(posedge clk); #100;
    check(q_re === 1'b1 && q_se === 1'b0, "no capture without CE");
    ce = 1;
    @(posedge clk); #100;
    check(q_se === 1'b1 && q_pe === 1'b1, "capture with CE");
    @(negedge clk); #100;
    check(q_re1 === 1'b1, "FDRE_1 on the falling edge");
    d = 0; sr = 1; #100;
    check(q_ce === 1'b0 && q_pe === 1'b1, "asynchronous clear and preset");
    check(q_re === 1'b1, "synchronous reset waits for the edge");
    @(posedge clk); #100;
    check(q_re === 1'b0 && q_se === 1'b1, "synchronous reset and set");
    sr = 0;

    li = 6'b000011; #1; check(lo === 1'b1, "lut 1 and 1");
    li = 6'b000001; #1; check(lo === 1'b0, "lut 1 and 0");
    li = 6'bxxxx11; #1; check(lo === 1'b1, "lut ignores an x it does not read");
    li = 6'b0000x1; #1; check(lo === 1'bx, "lut passes an x it reads");

    a = 4'd9; b = 4'd5; cin = 0; #1; check(sum === 4'd14 && co[3] === 1'b0, "carry 9 + 5");
    a = 4'd9; b = 4'd8; cin = 1; #1; check(sum === 4'd2 && co[3] === 1'b1, "carry 9 + 8 + 1");

    @(negedge clk); we = 1; wa = 6'd37; wd = 1;
    @(negedge clk); we = 0; wa = 6'd36; ra = 6'd37; #1;
    check(dpo === 1'b1 && spo === 1'b0, "ram64 write and two reads");

    @(negedge clk); en_b = 1; web = 4'b0011; ab = 10'd700; dib = 16'hbeef; dipb = 2'b10;
    @(posedge clk); #100;
    check(dob === 16'hbeef && dopb === 2'b10, "bram write first");
    @(negedge clk); en_b = 0; web = 0; en_a = 1; aa = 10'd700;
    @(posedge clk); #100;
    check(doa === 16'hbeef && dopa === 2'b10, "bram read on the other port");

    ext = 1; t = 1; #1; check(o === 1'b1, "iobuf reads the line");
    ext = 1'bz; t = 0; i = 0; #1; check(line === 1'b0 && o === 1'b0, "iobuf drives the line");
    t = 1; #1; check(line === 1'bz, "iobuf releases the line");

    if (errors == 0) $display("PASS: the cell models do what UG953 says");
    else $display("FAIL: %0d check(s)", errors);
    $finish;
  end
endmodule
