// Behavioural models of the 7-series cells Reticle's technology mapper
// writes, so a mapped netlist (`<top>.v` from `reticle fpga`) can run under
// a testbench in place of the RTL it was mapped from: flip-flops, LUT6,
// LUT6_2, CARRY4, RAM64X1D, RAMB18E1 in the one configuration the mapper uses
// (true dual port, 18 bits wide on both ports, no output registers), and the
// I/O buffers. Each follows UG953's description of the cell; none is
// Vivado's unisim model, and none models timing.
//
// An input left unconnected floats to `z`; every enable, write enable, set
// and reset here is tested with `=== 1'b1`, so an unconnected one is off,
// as the fabric ties it.
`timescale 1ps / 1ps

module FDRE #(parameter [0:0] INIT = 1'b0) (
  output reg Q, input wire C, input wire CE, input wire R, input wire D);
  initial Q = INIT;
  always @(posedge C)
    if (R === 1'b1) Q <= 1'b0;
    else if (CE === 1'b1) Q <= D;
endmodule

module FDRE_1 #(parameter [0:0] INIT = 1'b0) (
  output reg Q, input wire C, input wire CE, input wire R, input wire D);
  initial Q = INIT;
  always @(negedge C)
    if (R === 1'b1) Q <= 1'b0;
    else if (CE === 1'b1) Q <= D;
endmodule

module FDSE #(parameter [0:0] INIT = 1'b1) (
  output reg Q, input wire C, input wire CE, input wire S, input wire D);
  initial Q = INIT;
  always @(posedge C)
    if (S === 1'b1) Q <= 1'b1;
    else if (CE === 1'b1) Q <= D;
endmodule

module FDSE_1 #(parameter [0:0] INIT = 1'b1) (
  output reg Q, input wire C, input wire CE, input wire S, input wire D);
  initial Q = INIT;
  always @(negedge C)
    if (S === 1'b1) Q <= 1'b1;
    else if (CE === 1'b1) Q <= D;
endmodule

module FDCE #(parameter [0:0] INIT = 1'b0) (
  output reg Q, input wire C, input wire CE, input wire CLR, input wire D);
  initial Q = INIT;
  always @(posedge C or posedge CLR)
    if (CLR === 1'b1) Q <= 1'b0;
    else if (CE === 1'b1) Q <= D;
endmodule

module FDPE #(parameter [0:0] INIT = 1'b1) (
  output reg Q, input wire C, input wire CE, input wire PRE, input wire D);
  initial Q = INIT;
  always @(posedge C or posedge PRE)
    if (PRE === 1'b1) Q <= 1'b1;
    else if (CE === 1'b1) Q <= D;
endmodule

// An input that is x or z, as the unisim model does, leaves the output
// known when every value it could take gives the same one: a LUT whose
// function ignores an input must not turn that input's x into its own.
module LUT6 #(parameter [63:0] INIT = 64'h0) (
  output wire O, input wire I0, input wire I1, input wire I2, input wire I3,
  input wire I4, input wire I5);
  function lut(input [63:0] init, input [5:0] sel);
    integer k;
    reg seen0, seen1;
    begin
      if (^sel !== 1'bx) lut = init[sel];
      else begin
        seen0 = 0;
        seen1 = 0;
        for (k = 0; k < 64; k = k + 1)
          if (((k[0] === sel[0]) || (sel[0] !== 1'b0 && sel[0] !== 1'b1)) &&
              ((k[1] === sel[1]) || (sel[1] !== 1'b0 && sel[1] !== 1'b1)) &&
              ((k[2] === sel[2]) || (sel[2] !== 1'b0 && sel[2] !== 1'b1)) &&
              ((k[3] === sel[3]) || (sel[3] !== 1'b0 && sel[3] !== 1'b1)) &&
              ((k[4] === sel[4]) || (sel[4] !== 1'b0 && sel[4] !== 1'b1)) &&
              ((k[5] === sel[5]) || (sel[5] !== 1'b0 && sel[5] !== 1'b1))) begin
            if (init[k]) seen1 = 1;
            else seen0 = 1;
          end
        lut = (seen0 && seen1) ? 1'bx : seen1;
      end
    end
  endfunction
  assign O = lut(INIT, {I5, I4, I3, I2, I1, I0});
endmodule

// Two functions in one table, as UG953's LUT6_2: O6 is the whole table on
// I0..I5, O5 the lower half on I0..I4 whatever I5 is.
module LUT6_2 #(parameter [63:0] INIT = 64'h0) (
  output wire O6, output wire O5, input wire I0, input wire I1, input wire I2,
  input wire I3, input wire I4, input wire I5);
  LUT6 #(.INIT(INIT)) upper (.O(O6), .I0(I0), .I1(I1), .I2(I2), .I3(I3), .I4(I4), .I5(I5));
  LUT6 #(.INIT({INIT[31:0], INIT[31:0]})) lower (.O(O5), .I0(I0), .I1(I1), .I2(I2), .I3(I3),
                                                 .I4(I4), .I5(1'b0));
endmodule

// UG953: O[i] = S[i] ^ carry into bit i; CO[i] = S[i] ? carry in : DI[i];
// the carry into bit 0 is CI | CYINIT.
module CARRY4 (
  output wire [3:0] CO, output wire [3:0] O, input wire CI, input wire CYINIT,
  input wire [3:0] DI, input wire [3:0] S);
  wire c0 = (CI === 1'b1) | (CYINIT === 1'b1);
  assign CO[0] = S[0] ? c0 : DI[0];
  assign CO[1] = S[1] ? CO[0] : DI[1];
  assign CO[2] = S[2] ? CO[1] : DI[2];
  assign CO[3] = S[3] ? CO[2] : DI[3];
  assign O = S ^ {CO[2:0], c0};
endmodule

module RAM64X1D #(parameter [63:0] INIT = 64'h0) (
  output wire SPO, output wire DPO, input wire WCLK, input wire WE, input wire D,
  input wire A0, input wire A1, input wire A2, input wire A3, input wire A4,
  input wire A5, input wire DPRA0, input wire DPRA1, input wire DPRA2,
  input wire DPRA3, input wire DPRA4, input wire DPRA5);
  reg [63:0] mem = INIT;
  wire [5:0] a = {A5, A4, A3, A2, A1, A0};
  wire [5:0] dpra = {DPRA5, DPRA4, DPRA3, DPRA2, DPRA1, DPRA0};
  assign SPO = mem[a];
  assign DPO = mem[dpra];
  always @(posedge WCLK)
    if (WE === 1'b1) mem[a] <= D;
endmodule

// 18 bits wide on both ports: the address is ADDR[13:4], the data DI/DO
// [15:0] with the parity DIP/DOP [1:0] above them, and a write takes
// WE[1:0] per byte. Port A only reads where the mapper uses it, and port B
// writes WRITE_FIRST (the default), so its own output follows the write;
// a WEA that is not zero is reported, not modelled.
// A read of the address the other port writes on the same edge returns the
// old word here; UG473 calls that a collision and leaves it undefined.
module RAMB18E1 #(
  parameter integer READ_WIDTH_A = 18, parameter integer WRITE_WIDTH_A = 18,
  parameter integer READ_WIDTH_B = 18, parameter integer WRITE_WIDTH_B = 18,
  parameter RAM_MODE = "TDP", parameter integer DOA_REG = 0,
  parameter integer DOB_REG = 0, parameter SIM_DEVICE = "7SERIES")
(
  input wire CLKARDCLK, input wire CLKBWRCLK, input wire ENARDEN, input wire ENBWREN,
  input wire REGCEAREGCE, input wire REGCEB, input wire RSTRAMARSTRAM,
  input wire RSTRAMB, input wire RSTREGARSTREG, input wire RSTREGB,
  input wire [13:0] ADDRARDADDR, input wire [13:0] ADDRBWRADDR,
  input wire [15:0] DIADI, input wire [15:0] DIBDI, input wire [1:0] DIPADIP,
  input wire [1:0] DIPBDIP, input wire [1:0] WEA, input wire [3:0] WEBWE,
  output reg [15:0] DOADO, output reg [15:0] DOBDO, output reg [1:0] DOPADOP,
  output reg [1:0] DOPBDOP);
  reg [17:0] mem [0:1023];
  integer i;
  initial begin
    for (i = 0; i < 1024; i = i + 1) mem[i] = 18'd0;
    DOADO = 16'd0; DOPADOP = 2'd0; DOBDO = 16'd0; DOPBDOP = 2'd0;
  end
  always @(posedge CLKARDCLK)
    if (WEA === 2'b01 || WEA === 2'b10 || WEA === 2'b11)
      $display("RAMB18E1 %m: port A writes, which this model does not do");
  wire [9:0] aa = ADDRARDADDR[13:4];
  wire [9:0] ab = ADDRBWRADDR[13:4];
  reg [17:0] wb;
  // Port A reads only: one block writes the array, as the simulator
  // requires, and the mapper never connects WEA.
  always @(posedge CLKARDCLK)
    if (ENARDEN === 1'b1) begin
      if (RSTRAMARSTRAM === 1'b1) {DOPADOP, DOADO} <= 18'd0;
      else {DOPADOP, DOADO} <= mem[aa];
    end
  always @(posedge CLKBWRCLK)
    if (ENBWREN === 1'b1) begin
      wb = mem[ab];
      if (WEBWE[0] === 1'b1) begin wb[7:0] = DIBDI[7:0]; wb[16] = DIPBDIP[0]; end
      if (WEBWE[1] === 1'b1) begin wb[15:8] = DIBDI[15:8]; wb[17] = DIPBDIP[1]; end
      if (RSTRAMB === 1'b1) {DOPBDOP, DOBDO} <= 18'd0;
      else {DOPBDOP, DOBDO} <= {wb[17:16], wb[15:0]};
      if (WEBWE[0] === 1'b1 || WEBWE[1] === 1'b1) mem[ab] <= wb;
    end
endmodule

module IBUF #(parameter IOSTANDARD = "DEFAULT") (output wire O, input wire I);
  assign O = I;
endmodule

module OBUF #(parameter IOSTANDARD = "DEFAULT") (output wire O, input wire I);
  assign O = I;
endmodule

module OBUFT #(parameter IOSTANDARD = "DEFAULT") (output wire O, input wire I, input wire T);
  assign O = (T === 1'b1) ? 1'bz : I;
endmodule

module IOBUF #(parameter IOSTANDARD = "DEFAULT") (
  output wire O, inout wire IO, input wire I, input wire T);
  assign IO = (T === 1'b1) ? 1'bz : I;
  assign O = IO;
endmodule
