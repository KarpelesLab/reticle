// 7-series cells a design may instantiate by name, as ordinary Verilog.
//
// Written from what each cell does (UG953), and checked against the vendor's
// simulation model: every module here is run on the vectors in
// testdata/fpga/vivado/cells/, whose answers came from Vivado's simulator
// (tests/fpga_xray_vivado_cells.rs). None of the vendor's source is here.
//
// Elaboration adds a module from this file only when a design instantiates
// it and declares nothing of that name, and synthesis then maps it like any
// other logic: what the part ends up holding need not be the named cell.

module LUT1 #(parameter [1:0] INIT = 2'h0) (output O, input I0);
  assign O = INIT[{I0}];
endmodule

module LUT1_L #(parameter [1:0] INIT = 2'h0) (output LO, input I0);
  assign LO = INIT[{I0}];
endmodule

module LUT1_D #(parameter [1:0] INIT = 2'h0) (output O, output LO, input I0);
  assign O = INIT[{I0}];
  assign LO = O;
endmodule

module LUT2 #(parameter [3:0] INIT = 4'h0) (output O, input I0, I1);
  assign O = INIT[{I1, I0}];
endmodule

module LUT2_L #(parameter [3:0] INIT = 4'h0) (output LO, input I0, I1);
  assign LO = INIT[{I1, I0}];
endmodule

module LUT2_D #(parameter [3:0] INIT = 4'h0) (output O, output LO, input I0, I1);
  assign O = INIT[{I1, I0}];
  assign LO = O;
endmodule

module LUT3 #(parameter [7:0] INIT = 8'h0) (output O, input I0, I1, I2);
  assign O = INIT[{I2, I1, I0}];
endmodule

module LUT3_L #(parameter [7:0] INIT = 8'h0) (output LO, input I0, I1, I2);
  assign LO = INIT[{I2, I1, I0}];
endmodule

module LUT3_D #(parameter [7:0] INIT = 8'h0) (output O, output LO, input I0, I1, I2);
  assign O = INIT[{I2, I1, I0}];
  assign LO = O;
endmodule

module LUT4 #(parameter [15:0] INIT = 16'h0) (output O, input I0, I1, I2, I3);
  assign O = INIT[{I3, I2, I1, I0}];
endmodule

module LUT4_L #(parameter [15:0] INIT = 16'h0) (output LO, input I0, I1, I2, I3);
  assign LO = INIT[{I3, I2, I1, I0}];
endmodule

module LUT4_D #(parameter [15:0] INIT = 16'h0) (output O, output LO, input I0, I1, I2, I3);
  assign O = INIT[{I3, I2, I1, I0}];
  assign LO = O;
endmodule

module LUT5 #(parameter [31:0] INIT = 32'h0) (output O, input I0, I1, I2, I3, I4);
  assign O = INIT[{I4, I3, I2, I1, I0}];
endmodule

module LUT5_L #(parameter [31:0] INIT = 32'h0) (output LO, input I0, I1, I2, I3, I4);
  assign LO = INIT[{I4, I3, I2, I1, I0}];
endmodule

module LUT5_D #(parameter [31:0] INIT = 32'h0) (output O, output LO, input I0, I1, I2, I3, I4);
  assign O = INIT[{I4, I3, I2, I1, I0}];
  assign LO = O;
endmodule

module LUT6 #(parameter [63:0] INIT = 64'h0) (output O, input I0, I1, I2, I3, I4, I5);
  assign O = INIT[{I5, I4, I3, I2, I1, I0}];
endmodule

module LUT6_L #(parameter [63:0] INIT = 64'h0) (output LO, input I0, I1, I2, I3, I4, I5);
  assign LO = INIT[{I5, I4, I3, I2, I1, I0}];
endmodule

module LUT6_D #(parameter [63:0] INIT = 64'h0) (output O, output LO, input I0, I1, I2, I3, I4, I5);
  assign O = INIT[{I5, I4, I3, I2, I1, I0}];
  assign LO = O;
endmodule

module LUT6_2 #(parameter [63:0] INIT = 64'h0) (output O5, output O6, input I0, I1, I2, I3, I4, I5);
  wire [31:0] low = INIT[31:0];
  assign O5 = low[{I4, I3, I2, I1, I0}];
  assign O6 = INIT[{I5, I4, I3, I2, I1, I0}];
endmodule

module AND2 (output O, input I0, I1);
  assign O = &{I1, I0};
endmodule

module AND2B1 (output O, input I0, I1);
  assign O = &{I1, ~I0};
endmodule

module AND2B2 (output O, input I0, I1);
  assign O = &{~I1, ~I0};
endmodule

module AND3 (output O, input I0, I1, I2);
  assign O = &{I2, I1, I0};
endmodule

module AND3B1 (output O, input I0, I1, I2);
  assign O = &{I2, I1, ~I0};
endmodule

module AND3B2 (output O, input I0, I1, I2);
  assign O = &{I2, ~I1, ~I0};
endmodule

module AND3B3 (output O, input I0, I1, I2);
  assign O = &{~I2, ~I1, ~I0};
endmodule

module AND4 (output O, input I0, I1, I2, I3);
  assign O = &{I3, I2, I1, I0};
endmodule

module AND4B1 (output O, input I0, I1, I2, I3);
  assign O = &{I3, I2, I1, ~I0};
endmodule

module AND4B2 (output O, input I0, I1, I2, I3);
  assign O = &{I3, I2, ~I1, ~I0};
endmodule

module AND4B3 (output O, input I0, I1, I2, I3);
  assign O = &{I3, ~I2, ~I1, ~I0};
endmodule

module AND4B4 (output O, input I0, I1, I2, I3);
  assign O = &{~I3, ~I2, ~I1, ~I0};
endmodule

module AND5 (output O, input I0, I1, I2, I3, I4);
  assign O = &{I4, I3, I2, I1, I0};
endmodule

module AND5B1 (output O, input I0, I1, I2, I3, I4);
  assign O = &{I4, I3, I2, I1, ~I0};
endmodule

module AND5B2 (output O, input I0, I1, I2, I3, I4);
  assign O = &{I4, I3, I2, ~I1, ~I0};
endmodule

module AND5B3 (output O, input I0, I1, I2, I3, I4);
  assign O = &{I4, I3, ~I2, ~I1, ~I0};
endmodule

module AND5B4 (output O, input I0, I1, I2, I3, I4);
  assign O = &{I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module AND5B5 (output O, input I0, I1, I2, I3, I4);
  assign O = &{~I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module NAND2 (output O, input I0, I1);
  assign O = ~&{I1, I0};
endmodule

module NAND2B1 (output O, input I0, I1);
  assign O = ~&{I1, ~I0};
endmodule

module NAND2B2 (output O, input I0, I1);
  assign O = ~&{~I1, ~I0};
endmodule

module NAND3 (output O, input I0, I1, I2);
  assign O = ~&{I2, I1, I0};
endmodule

module NAND3B1 (output O, input I0, I1, I2);
  assign O = ~&{I2, I1, ~I0};
endmodule

module NAND3B2 (output O, input I0, I1, I2);
  assign O = ~&{I2, ~I1, ~I0};
endmodule

module NAND3B3 (output O, input I0, I1, I2);
  assign O = ~&{~I2, ~I1, ~I0};
endmodule

module NAND4 (output O, input I0, I1, I2, I3);
  assign O = ~&{I3, I2, I1, I0};
endmodule

module NAND4B1 (output O, input I0, I1, I2, I3);
  assign O = ~&{I3, I2, I1, ~I0};
endmodule

module NAND4B2 (output O, input I0, I1, I2, I3);
  assign O = ~&{I3, I2, ~I1, ~I0};
endmodule

module NAND4B3 (output O, input I0, I1, I2, I3);
  assign O = ~&{I3, ~I2, ~I1, ~I0};
endmodule

module NAND4B4 (output O, input I0, I1, I2, I3);
  assign O = ~&{~I3, ~I2, ~I1, ~I0};
endmodule

module NAND5 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{I4, I3, I2, I1, I0};
endmodule

module NAND5B1 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{I4, I3, I2, I1, ~I0};
endmodule

module NAND5B2 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{I4, I3, I2, ~I1, ~I0};
endmodule

module NAND5B3 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{I4, I3, ~I2, ~I1, ~I0};
endmodule

module NAND5B4 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module NAND5B5 (output O, input I0, I1, I2, I3, I4);
  assign O = ~&{~I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module OR2 (output O, input I0, I1);
  assign O = |{I1, I0};
endmodule

module OR2B1 (output O, input I0, I1);
  assign O = |{I1, ~I0};
endmodule

module OR2B2 (output O, input I0, I1);
  assign O = |{~I1, ~I0};
endmodule

module OR3 (output O, input I0, I1, I2);
  assign O = |{I2, I1, I0};
endmodule

module OR3B1 (output O, input I0, I1, I2);
  assign O = |{I2, I1, ~I0};
endmodule

module OR3B2 (output O, input I0, I1, I2);
  assign O = |{I2, ~I1, ~I0};
endmodule

module OR3B3 (output O, input I0, I1, I2);
  assign O = |{~I2, ~I1, ~I0};
endmodule

module OR4 (output O, input I0, I1, I2, I3);
  assign O = |{I3, I2, I1, I0};
endmodule

module OR4B1 (output O, input I0, I1, I2, I3);
  assign O = |{I3, I2, I1, ~I0};
endmodule

module OR4B2 (output O, input I0, I1, I2, I3);
  assign O = |{I3, I2, ~I1, ~I0};
endmodule

module OR4B3 (output O, input I0, I1, I2, I3);
  assign O = |{I3, ~I2, ~I1, ~I0};
endmodule

module OR4B4 (output O, input I0, I1, I2, I3);
  assign O = |{~I3, ~I2, ~I1, ~I0};
endmodule

module OR5 (output O, input I0, I1, I2, I3, I4);
  assign O = |{I4, I3, I2, I1, I0};
endmodule

module OR5B1 (output O, input I0, I1, I2, I3, I4);
  assign O = |{I4, I3, I2, I1, ~I0};
endmodule

module OR5B2 (output O, input I0, I1, I2, I3, I4);
  assign O = |{I4, I3, I2, ~I1, ~I0};
endmodule

module OR5B3 (output O, input I0, I1, I2, I3, I4);
  assign O = |{I4, I3, ~I2, ~I1, ~I0};
endmodule

module OR5B4 (output O, input I0, I1, I2, I3, I4);
  assign O = |{I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module OR5B5 (output O, input I0, I1, I2, I3, I4);
  assign O = |{~I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module NOR2 (output O, input I0, I1);
  assign O = ~|{I1, I0};
endmodule

module NOR2B1 (output O, input I0, I1);
  assign O = ~|{I1, ~I0};
endmodule

module NOR2B2 (output O, input I0, I1);
  assign O = ~|{~I1, ~I0};
endmodule

module NOR3 (output O, input I0, I1, I2);
  assign O = ~|{I2, I1, I0};
endmodule

module NOR3B1 (output O, input I0, I1, I2);
  assign O = ~|{I2, I1, ~I0};
endmodule

module NOR3B2 (output O, input I0, I1, I2);
  assign O = ~|{I2, ~I1, ~I0};
endmodule

module NOR3B3 (output O, input I0, I1, I2);
  assign O = ~|{~I2, ~I1, ~I0};
endmodule

module NOR4 (output O, input I0, I1, I2, I3);
  assign O = ~|{I3, I2, I1, I0};
endmodule

module NOR4B1 (output O, input I0, I1, I2, I3);
  assign O = ~|{I3, I2, I1, ~I0};
endmodule

module NOR4B2 (output O, input I0, I1, I2, I3);
  assign O = ~|{I3, I2, ~I1, ~I0};
endmodule

module NOR4B3 (output O, input I0, I1, I2, I3);
  assign O = ~|{I3, ~I2, ~I1, ~I0};
endmodule

module NOR4B4 (output O, input I0, I1, I2, I3);
  assign O = ~|{~I3, ~I2, ~I1, ~I0};
endmodule

module NOR5 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{I4, I3, I2, I1, I0};
endmodule

module NOR5B1 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{I4, I3, I2, I1, ~I0};
endmodule

module NOR5B2 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{I4, I3, I2, ~I1, ~I0};
endmodule

module NOR5B3 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{I4, I3, ~I2, ~I1, ~I0};
endmodule

module NOR5B4 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module NOR5B5 (output O, input I0, I1, I2, I3, I4);
  assign O = ~|{~I4, ~I3, ~I2, ~I1, ~I0};
endmodule

module XOR2 (output O, input I0, I1);
  assign O = ^{I1, I0};
endmodule

module XOR3 (output O, input I0, I1, I2);
  assign O = ^{I2, I1, I0};
endmodule

module XOR4 (output O, input I0, I1, I2, I3);
  assign O = ^{I3, I2, I1, I0};
endmodule

module XOR5 (output O, input I0, I1, I2, I3, I4);
  assign O = ^{I4, I3, I2, I1, I0};
endmodule

module XNOR2 (output O, input I0, I1);
  assign O = ~^{I1, I0};
endmodule

module XNOR3 (output O, input I0, I1, I2);
  assign O = ~^{I2, I1, I0};
endmodule

module XNOR4 (output O, input I0, I1, I2, I3);
  assign O = ~^{I3, I2, I1, I0};
endmodule

module XNOR5 (output O, input I0, I1, I2, I3, I4);
  assign O = ~^{I4, I3, I2, I1, I0};
endmodule

module INV (output O, input I);
  assign O = ~I;
endmodule
module BUF (output O, input I);
  assign O = I;
endmodule
module GND (output G);
  assign G = 1'b0;
endmodule
module VCC (output P);
  assign P = 1'b1;
endmodule
module AND2B1L #(parameter [0:0] IS_SRI_INVERTED = 1'b0) (output O, input DI, input SRI);
  assign O = DI & ~(SRI ^ IS_SRI_INVERTED);
endmodule
module OR2L #(parameter [0:0] IS_SRI_INVERTED = 1'b0) (output O, input DI, input SRI);
  assign O = DI | (SRI ^ IS_SRI_INVERTED);
endmodule

module MUXF5 (output O, input I0, input I1, input S);
  assign O = S ? I1 : I0;
endmodule

module MUXF5_L (output LO, input I0, input I1, input S);
  assign LO = S ? I1 : I0;
endmodule

module MUXF5_D (output O, output LO, input I0, input I1, input S);
  assign O = S ? I1 : I0;
  assign LO = O;
endmodule

module MUXF6 (output O, input I0, input I1, input S);
  assign O = S ? I1 : I0;
endmodule

module MUXF6_L (output LO, input I0, input I1, input S);
  assign LO = S ? I1 : I0;
endmodule

module MUXF6_D (output O, output LO, input I0, input I1, input S);
  assign O = S ? I1 : I0;
  assign LO = O;
endmodule

module MUXF7 (output O, input I0, input I1, input S);
  assign O = S ? I1 : I0;
endmodule

module MUXF7_L (output LO, input I0, input I1, input S);
  MUXF7 m (.O(LO), .I0(I0), .I1(I1), .S(S));
endmodule

module MUXF7_D (output O, output LO, input I0, input I1, input S);
  MUXF7 m (.O(O), .I0(I0), .I1(I1), .S(S));
  assign LO = O;
endmodule

module MUXF8 (output O, input I0, input I1, input S);
  assign O = S ? I1 : I0;
endmodule

module MUXF8_L (output LO, input I0, input I1, input S);
  MUXF8 m (.O(LO), .I0(I0), .I1(I1), .S(S));
endmodule

module MUXF8_D (output O, output LO, input I0, input I1, input S);
  MUXF8 m (.O(O), .I0(I0), .I1(I1), .S(S));
  assign LO = O;
endmodule

module CARRY4 (output [3:0] CO, output [3:0] O, input CI, input CYINIT, input [3:0] DI, input [3:0] S);
  wire c0 = CI | CYINIT;
  assign CO[0] = S[0] ? c0 : DI[0];
  assign CO[1] = S[1] ? CO[0] : DI[1];
  assign CO[2] = S[2] ? CO[1] : DI[2];
  assign CO[3] = S[3] ? CO[2] : DI[3];
  assign O = S ^ {CO[2:0], c0};
endmodule

module MUXCY (output O, input CI, input DI, input S);
  assign O = S ? CI : DI;
endmodule
module MUXCY_L (output LO, input CI, input DI, input S);
  assign LO = S ? CI : DI;
endmodule
module MUXCY_D (output O, output LO, input CI, input DI, input S);
  assign O = S ? CI : DI;
  assign LO = O;
endmodule
module XORCY (output O, input CI, input LI);
  assign O = CI ^ LI;
endmodule
module XORCY_L (output LO, input CI, input LI);
  assign LO = CI ^ LI;
endmodule
module XORCY_D (output O, output LO, input CI, input LI);
  assign O = CI ^ LI;
  assign LO = O;
endmodule

module FDRE #(parameter [0:0] INIT = 1'b0, parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_D_INVERTED = 1'b0, parameter [0:0] IS_R_INVERTED = 1'b0) (output reg Q, input C, input CE, input R, input D);
  initial Q = INIT;
  generate
    if (IS_C_INVERTED) begin
      always @(negedge C) if (R ^ IS_R_INVERTED) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else begin
      always @(posedge C) if (R ^ IS_R_INVERTED) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end
  endgenerate
endmodule

module FDSE #(parameter [0:0] INIT = 1'b1, parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_D_INVERTED = 1'b0, parameter [0:0] IS_S_INVERTED = 1'b0) (output reg Q, input C, input CE, input S, input D);
  initial Q = INIT;
  generate
    if (IS_C_INVERTED) begin
      always @(negedge C) if (S ^ IS_S_INVERTED) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else begin
      always @(posedge C) if (S ^ IS_S_INVERTED) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end
  endgenerate
endmodule

module FDCE #(parameter [0:0] INIT = 1'b0, parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_D_INVERTED = 1'b0, parameter [0:0] IS_CLR_INVERTED = 1'b0) (output reg Q, input C, input CE, input CLR, input D);
  initial Q = INIT;
  generate
    if (IS_C_INVERTED && IS_CLR_INVERTED) begin
      always @(negedge C or negedge CLR) if (!CLR) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else if (IS_C_INVERTED) begin
      always @(negedge C or posedge CLR) if (CLR) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else if (IS_CLR_INVERTED) begin
      always @(posedge C or negedge CLR) if (!CLR) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else begin
      always @(posedge C or posedge CLR) if (CLR) Q <= 1'b0; else if (CE) Q <= D ^ IS_D_INVERTED;
    end
  endgenerate
endmodule

module FDPE #(parameter [0:0] INIT = 1'b1, parameter [0:0] IS_C_INVERTED = 1'b0, parameter [0:0] IS_D_INVERTED = 1'b0, parameter [0:0] IS_PRE_INVERTED = 1'b0) (output reg Q, input C, input CE, input PRE, input D);
  initial Q = INIT;
  generate
    if (IS_C_INVERTED && IS_PRE_INVERTED) begin
      always @(negedge C or negedge PRE) if (!PRE) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else if (IS_C_INVERTED) begin
      always @(negedge C or posedge PRE) if (PRE) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else if (IS_PRE_INVERTED) begin
      always @(posedge C or negedge PRE) if (!PRE) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end else begin
      always @(posedge C or posedge PRE) if (PRE) Q <= 1'b1; else if (CE) Q <= D ^ IS_D_INVERTED;
    end
  endgenerate
endmodule

module FDRE_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CE, input R, input D);
  initial Q = INIT;
  always @(negedge C) if (R) Q <= 1'b0; else if (CE) Q <= D;
endmodule

module FDSE_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input CE, input S, input D);
  initial Q = INIT;
  always @(negedge C) if (S) Q <= 1'b1; else if (CE) Q <= D;
endmodule

module FDCE_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CE, input CLR, input D);
  initial Q = INIT;
  always @(negedge C or posedge CLR) if (CLR) Q <= 1'b0; else if (CE) Q <= D;
endmodule

module FDPE_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input CE, input PRE, input D);
  initial Q = INIT;
  always @(negedge C or posedge PRE) if (PRE) Q <= 1'b1; else if (CE) Q <= D;
endmodule

module FD #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input D);
  initial Q = INIT;
  always @(posedge C) if (1'b1) Q <= D;
endmodule

module FD_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input D);
  initial Q = INIT;
  always @(negedge C) if (1'b1) Q <= D;
endmodule

module FDE #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CE, input D);
  initial Q = INIT;
  always @(posedge C) if (CE) Q <= D;
endmodule

module FDE_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CE, input D);
  initial Q = INIT;
  always @(negedge C) if (CE) Q <= D;
endmodule

module FDR #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input R, input D);
  initial Q = INIT;
  always @(posedge C) if (R) Q <= 1'b0; else if (1'b1) Q <= D;
endmodule

module FDR_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input R, input D);
  initial Q = INIT;
  always @(negedge C) if (R) Q <= 1'b0; else if (1'b1) Q <= D;
endmodule

module FDS #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input S, input D);
  initial Q = INIT;
  always @(posedge C) if (S) Q <= 1'b1; else if (1'b1) Q <= D;
endmodule

module FDS_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input S, input D);
  initial Q = INIT;
  always @(negedge C) if (S) Q <= 1'b1; else if (1'b1) Q <= D;
endmodule

module FDC #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CLR, input D);
  initial Q = INIT;
  always @(posedge C or posedge CLR) if (CLR) Q <= 1'b0; else if (1'b1) Q <= D;
endmodule

module FDC_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input C, input CLR, input D);
  initial Q = INIT;
  always @(negedge C or posedge CLR) if (CLR) Q <= 1'b0; else if (1'b1) Q <= D;
endmodule

module FDP #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input PRE, input D);
  initial Q = INIT;
  always @(posedge C or posedge PRE) if (PRE) Q <= 1'b1; else if (1'b1) Q <= D;
endmodule

module FDP_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input C, input PRE, input D);
  initial Q = INIT;
  always @(negedge C or posedge PRE) if (PRE) Q <= 1'b1; else if (1'b1) Q <= D;
endmodule

module LDCE #(parameter [0:0] INIT = 1'b0, parameter [0:0] IS_G_INVERTED = 1'b0, parameter [0:0] IS_CLR_INVERTED = 1'b0) (output reg Q, input D, input G, input GE, input CLR);
  initial Q = INIT;
  always @* if (CLR ^ IS_CLR_INVERTED) Q = 1'b0; else if ((G ^ IS_G_INVERTED) && GE) Q = D;
endmodule

module LDPE #(parameter [0:0] INIT = 1'b1, parameter [0:0] IS_G_INVERTED = 1'b0, parameter [0:0] IS_PRE_INVERTED = 1'b0) (output reg Q, input D, input G, input GE, input PRE);
  initial Q = INIT;
  always @* if (PRE ^ IS_PRE_INVERTED) Q = 1'b1; else if ((G ^ IS_G_INVERTED) && GE) Q = D;
endmodule

module LDCE_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G, input GE, input CLR);
  initial Q = INIT;
  always @* if (CLR) Q = 1'b0; else if (!(G) && GE) Q = D;
endmodule

module LDPE_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input D, input G, input GE, input PRE);
  initial Q = INIT;
  always @* if (PRE) Q = 1'b1; else if (!(G) && GE) Q = D;
endmodule

module LD #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G);
  initial Q = INIT;
  always @* if ((G)) Q = D;
endmodule

module LD_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G);
  initial Q = INIT;
  always @* if (!(G)) Q = D;
endmodule

module LDC #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G, input CLR);
  initial Q = INIT;
  always @* if (CLR) Q = 1'b0; else if ((G)) Q = D;
endmodule

module LDC_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G, input CLR);
  initial Q = INIT;
  always @* if (CLR) Q = 1'b0; else if (!(G)) Q = D;
endmodule

module LDP #(parameter [0:0] INIT = 1'b1) (output reg Q, input D, input G, input PRE);
  initial Q = INIT;
  always @* if (PRE) Q = 1'b1; else if ((G)) Q = D;
endmodule

module LDP_1 #(parameter [0:0] INIT = 1'b1) (output reg Q, input D, input G, input PRE);
  initial Q = INIT;
  always @* if (PRE) Q = 1'b1; else if (!(G)) Q = D;
endmodule

module LDE #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G, input GE);
  initial Q = INIT;
  always @* if ((G) && GE) Q = D;
endmodule

module LDE_1 #(parameter [0:0] INIT = 1'b0) (output reg Q, input D, input G, input GE);
  initial Q = INIT;
  always @* if (!(G) && GE) Q = D;
endmodule

module SRL16 #(parameter [15:0] INIT = 16'h0) (output Q, input A0, input A1, input A2, input A3, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  always @(posedge CLK) if (1'b1) r <= {r[14:0], D};
endmodule

module SRL16_1 #(parameter [15:0] INIT = 16'h0) (output Q, input A0, input A1, input A2, input A3, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  always @(negedge CLK) if (1'b1) r <= {r[14:0], D};
endmodule

module SRL16E #(parameter [15:0] INIT = 16'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0) (output Q, input A0, input A1, input A2, input A3, input CE, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[14:0], D};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[14:0], D};
    end
  endgenerate
endmodule

module SRL16E_1 #(parameter [15:0] INIT = 16'h0) (output Q, input A0, input A1, input A2, input A3, input CE, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  always @(negedge CLK) if (CE) r <= {r[14:0], D};
endmodule

module SRLC16 #(parameter [15:0] INIT = 16'h0) (output Q, output Q15, input A0, input A1, input A2, input A3, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  assign Q15 = r[15];
  always @(posedge CLK) if (1'b1) r <= {r[14:0], D};
endmodule

module SRLC16_1 #(parameter [15:0] INIT = 16'h0) (output Q, output Q15, input A0, input A1, input A2, input A3, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  assign Q15 = r[15];
  always @(negedge CLK) if (1'b1) r <= {r[14:0], D};
endmodule

module SRLC16E #(parameter [15:0] INIT = 16'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0) (output Q, output Q15, input A0, input A1, input A2, input A3, input CE, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  assign Q15 = r[15];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[14:0], D};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[14:0], D};
    end
  endgenerate
endmodule

module SRLC16E_1 #(parameter [15:0] INIT = 16'h0) (output Q, output Q15, input A0, input A1, input A2, input A3, input CE, input CLK, input D);
  reg [15:0] r = INIT;
  assign Q = r[{A3, A2, A1, A0}];
  assign Q15 = r[15];
  always @(negedge CLK) if (CE) r <= {r[14:0], D};
endmodule

module SRLC32E #(parameter [31:0] INIT = 32'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0) (output Q, output Q31, input [4:0] A, input CE, input CLK, input D);
  reg [31:0] r = INIT;
  assign Q = r[A];
  assign Q31 = r[31];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[30:0], D};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[30:0], D};
    end
  endgenerate
endmodule

module SRL32E #(parameter [31:0] INIT = 32'h0) (output Q, input [4:0] A, input CE, input CLK, input D);
  reg [31:0] r = INIT;
  assign Q = r[A];
  always @(posedge CLK) if (CE) r <= {r[30:0], D};
endmodule
module CFGLUT5 #(parameter [31:0] INIT = 32'h0, parameter [0:0] IS_CLK_INVERTED = 1'b0)
  (output CDO, output O5, output O6, input I4, input I3, input I2, input I1, input I0, input CDI, input CE, input CLK);
  reg [31:0] r = INIT;
  wire [15:0] low = r[15:0];
  assign O6 = r[{I4, I3, I2, I1, I0}];
  assign O5 = low[{I3, I2, I1, I0}];
  assign CDO = r[31];
  generate
    if (IS_CLK_INVERTED) begin
      always @(negedge CLK) if (CE) r <= {r[30:0], CDI};
    end else begin
      always @(posedge CLK) if (CE) r <= {r[30:0], CDI};
    end
  endgenerate
endmodule

module RAM16X1S #(parameter [15:0] INIT = 16'h0) (output O, input A0, input A1, input A2, input A3, input D, input WCLK, input WE);
  RAM32X1S #(.INIT({16'h0, INIT})) r (.O(O), .A0(A0), .A1(A1), .A2(A2), .A3(A3), .A4(1'b0), .D(D), .WCLK(WCLK), .WE(WE));
endmodule

module RAM16X1D #(parameter [15:0] INIT = 16'h0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input D, input WCLK, input WE);
  RAM32X1D #(.INIT({16'h0, INIT})) r (.DPO(DPO), .SPO(SPO), .A0(A0), .A1(A1), .A2(A2), .A3(A3), .A4(1'b0), .DPRA0(DPRA0), .DPRA1(DPRA1), .DPRA2(DPRA2), .DPRA3(DPRA3), .DPRA4(1'b0), .D(D), .WCLK(WCLK), .WE(WE));
endmodule

module RAM16X1S_1 #(parameter [15:0] INIT = 16'h0) (output O, input A0, input A1, input A2, input A3, input D, input WCLK, input WE);
  RAM32X1S_1 #(.INIT({16'h0, INIT})) r (.O(O), .A0(A0), .A1(A1), .A2(A2), .A3(A3), .A4(1'b0), .D(D), .WCLK(WCLK), .WE(WE));
endmodule

module RAM16X1D_1 #(parameter [15:0] INIT = 16'h0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input D, input WCLK, input WE);
  RAM32X1D_1 #(.INIT({16'h0, INIT})) r (.DPO(DPO), .SPO(SPO), .A0(A0), .A1(A1), .A2(A2), .A3(A3), .A4(1'b0), .DPRA0(DPRA0), .DPRA1(DPRA1), .DPRA2(DPRA2), .DPRA3(DPRA3), .DPRA4(1'b0), .D(D), .WCLK(WCLK), .WE(WE));
endmodule

module RAM32X1S #(parameter [31:0] INIT = 32'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output O, input A0, input A1, input A2, input A3, input A4, input D, input WCLK, input WE);
  reg [31:0] mem = INIT;
  assign O = mem[{A4, A3, A2, A1, A0}];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
    end
  endgenerate
endmodule

module RAM32X1S_1 #(parameter [31:0] INIT = 32'h0) (output O, input A0, input A1, input A2, input A3, input A4, input D, input WCLK, input WE);
  reg [31:0] mem = INIT;
  assign O = mem[{A4, A3, A2, A1, A0}];
  always @(negedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
endmodule

module RAM32X1D #(parameter [31:0] INIT = 32'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input A4, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input DPRA4, input D, input WCLK, input WE);
  reg [31:0] mem = INIT;
  assign SPO = mem[{A4, A3, A2, A1, A0}];
  assign DPO = mem[{DPRA4, DPRA3, DPRA2, DPRA1, DPRA0}];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
    end
  endgenerate
endmodule

module RAM32X1D_1 #(parameter [31:0] INIT = 32'h0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input A4, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input DPRA4, input D, input WCLK, input WE);
  reg [31:0] mem = INIT;
  assign SPO = mem[{A4, A3, A2, A1, A0}];
  assign DPO = mem[{DPRA4, DPRA3, DPRA2, DPRA1, DPRA0}];
  always @(negedge WCLK) if (WE) mem[{A4, A3, A2, A1, A0}] <= D;
endmodule

module RAM64X1S #(parameter [63:0] INIT = 64'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input D, input WCLK, input WE);
  reg [63:0] mem = INIT;
  assign O = mem[{A5, A4, A3, A2, A1, A0}];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
    end
  endgenerate
endmodule

module RAM64X1S_1 #(parameter [63:0] INIT = 64'h0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input D, input WCLK, input WE);
  reg [63:0] mem = INIT;
  assign O = mem[{A5, A4, A3, A2, A1, A0}];
  always @(negedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
endmodule

module RAM64X1D #(parameter [63:0] INIT = 64'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input A4, input A5, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input DPRA4, input DPRA5, input D, input WCLK, input WE);
  reg [63:0] mem = INIT;
  assign SPO = mem[{A5, A4, A3, A2, A1, A0}];
  assign DPO = mem[{DPRA5, DPRA4, DPRA3, DPRA2, DPRA1, DPRA0}];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
    end
  endgenerate
endmodule

module RAM64X1D_1 #(parameter [63:0] INIT = 64'h0) (output DPO, output SPO, input A0, input A1, input A2, input A3, input A4, input A5, input DPRA0, input DPRA1, input DPRA2, input DPRA3, input DPRA4, input DPRA5, input D, input WCLK, input WE);
  reg [63:0] mem = INIT;
  assign SPO = mem[{A5, A4, A3, A2, A1, A0}];
  assign DPO = mem[{DPRA5, DPRA4, DPRA3, DPRA2, DPRA1, DPRA0}];
  always @(negedge WCLK) if (WE) mem[{A5, A4, A3, A2, A1, A0}] <= D;
endmodule

module RAM128X1S #(parameter [127:0] INIT = 128'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input A6, input D, input WCLK, input WE);
  reg [127:0] mem = INIT;
  assign O = mem[{A6, A5, A4, A3, A2, A1, A0}];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[{A6, A5, A4, A3, A2, A1, A0}] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[{A6, A5, A4, A3, A2, A1, A0}] <= D;
    end
  endgenerate
endmodule

module RAM128X1S_1 #(parameter [127:0] INIT = 128'h0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input A6, input D, input WCLK, input WE);
  reg [127:0] mem = INIT;
  assign O = mem[{A6, A5, A4, A3, A2, A1, A0}];
  always @(negedge WCLK) if (WE) mem[{A6, A5, A4, A3, A2, A1, A0}] <= D;
endmodule

module RAM128X1D #(parameter [127:0] INIT = 128'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0) (output DPO, output SPO, input [6:0] A, input [6:0] DPRA, input D, input WCLK, input WE);
  reg [127:0] mem = INIT;
  assign SPO = mem[A];
  assign DPO = mem[DPRA];
  generate
    if (IS_WCLK_INVERTED) begin
      always @(negedge WCLK) if (WE) mem[A] <= D;
    end else begin
      always @(posedge WCLK) if (WE) mem[A] <= D;
    end
  endgenerate
endmodule

module RAM256X1S #(parameter [255:0] INIT = 256'h0) (output O, input [7:0] A, input D, input WCLK, input WE);
  reg [255:0] mem = INIT;
  assign O = mem[A];
  always @(posedge WCLK) if (WE) mem[A] <= D;
endmodule

module RAM256X1D #(parameter [255:0] INIT = 256'h0) (output DPO, output SPO, input [7:0] A, input [7:0] DPRA, input D, input WCLK, input WE);
  reg [255:0] mem = INIT;
  assign SPO = mem[A];
  assign DPO = mem[DPRA];
  always @(posedge WCLK) if (WE) mem[A] <= D;
endmodule

module RAM16X2S #(parameter [15:0] INIT_00 = 16'h0, parameter [15:0] INIT_01 = 16'h0) (output O0, output O1, input A0, input A1, input A2, input A3, input D0, input D1, input WCLK, input WE);
  wire [3:0] a = {A3, A2, A1, A0};
  reg [15:0] b0 = INIT_00;
  reg [15:0] b1 = INIT_01;
  assign O0 = b0[a];
  assign O1 = b1[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D0; b1[a] <= D1; end
endmodule

module RAM16X4S #(parameter [15:0] INIT_00 = 16'h0, parameter [15:0] INIT_01 = 16'h0, parameter [15:0] INIT_02 = 16'h0, parameter [15:0] INIT_03 = 16'h0) (output O0, output O1, output O2, output O3, input A0, input A1, input A2, input A3, input D0, input D1, input D2, input D3, input WCLK, input WE);
  wire [3:0] a = {A3, A2, A1, A0};
  reg [15:0] b0 = INIT_00;
  reg [15:0] b1 = INIT_01;
  reg [15:0] b2 = INIT_02;
  reg [15:0] b3 = INIT_03;
  assign O0 = b0[a];
  assign O1 = b1[a];
  assign O2 = b2[a];
  assign O3 = b3[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D0; b1[a] <= D1; b2[a] <= D2; b3[a] <= D3; end
endmodule

module RAM16X8S #(parameter [15:0] INIT_00 = 16'h0, parameter [15:0] INIT_01 = 16'h0, parameter [15:0] INIT_02 = 16'h0, parameter [15:0] INIT_03 = 16'h0, parameter [15:0] INIT_04 = 16'h0, parameter [15:0] INIT_05 = 16'h0, parameter [15:0] INIT_06 = 16'h0, parameter [15:0] INIT_07 = 16'h0) (output [7:0] O, input A0, input A1, input A2, input A3, input [7:0] D, input WCLK, input WE);
  wire [3:0] a = {A3, A2, A1, A0};
  reg [15:0] b0 = INIT_00;
  reg [15:0] b1 = INIT_01;
  reg [15:0] b2 = INIT_02;
  reg [15:0] b3 = INIT_03;
  reg [15:0] b4 = INIT_04;
  reg [15:0] b5 = INIT_05;
  reg [15:0] b6 = INIT_06;
  reg [15:0] b7 = INIT_07;
  assign O[0] = b0[a];
  assign O[1] = b1[a];
  assign O[2] = b2[a];
  assign O[3] = b3[a];
  assign O[4] = b4[a];
  assign O[5] = b5[a];
  assign O[6] = b6[a];
  assign O[7] = b7[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D[0]; b1[a] <= D[1]; b2[a] <= D[2]; b3[a] <= D[3]; b4[a] <= D[4]; b5[a] <= D[5]; b6[a] <= D[6]; b7[a] <= D[7]; end
endmodule

module RAM32X2S #(parameter [31:0] INIT_00 = 32'h0, parameter [31:0] INIT_01 = 32'h0) (output O0, output O1, input A0, input A1, input A2, input A3, input A4, input D0, input D1, input WCLK, input WE);
  wire [4:0] a = {A4, A3, A2, A1, A0};
  reg [31:0] b0 = INIT_00;
  reg [31:0] b1 = INIT_01;
  assign O0 = b0[a];
  assign O1 = b1[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D0; b1[a] <= D1; end
endmodule

module RAM32X4S #(parameter [31:0] INIT_00 = 32'h0, parameter [31:0] INIT_01 = 32'h0, parameter [31:0] INIT_02 = 32'h0, parameter [31:0] INIT_03 = 32'h0) (output O0, output O1, output O2, output O3, input A0, input A1, input A2, input A3, input A4, input D0, input D1, input D2, input D3, input WCLK, input WE);
  wire [4:0] a = {A4, A3, A2, A1, A0};
  reg [31:0] b0 = INIT_00;
  reg [31:0] b1 = INIT_01;
  reg [31:0] b2 = INIT_02;
  reg [31:0] b3 = INIT_03;
  assign O0 = b0[a];
  assign O1 = b1[a];
  assign O2 = b2[a];
  assign O3 = b3[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D0; b1[a] <= D1; b2[a] <= D2; b3[a] <= D3; end
endmodule

module RAM32X8S #(parameter [31:0] INIT_00 = 32'h0, parameter [31:0] INIT_01 = 32'h0, parameter [31:0] INIT_02 = 32'h0, parameter [31:0] INIT_03 = 32'h0, parameter [31:0] INIT_04 = 32'h0, parameter [31:0] INIT_05 = 32'h0, parameter [31:0] INIT_06 = 32'h0, parameter [31:0] INIT_07 = 32'h0) (output [7:0] O, input A0, input A1, input A2, input A3, input A4, input [7:0] D, input WCLK, input WE);
  wire [4:0] a = {A4, A3, A2, A1, A0};
  reg [31:0] b0 = INIT_00;
  reg [31:0] b1 = INIT_01;
  reg [31:0] b2 = INIT_02;
  reg [31:0] b3 = INIT_03;
  reg [31:0] b4 = INIT_04;
  reg [31:0] b5 = INIT_05;
  reg [31:0] b6 = INIT_06;
  reg [31:0] b7 = INIT_07;
  assign O[0] = b0[a];
  assign O[1] = b1[a];
  assign O[2] = b2[a];
  assign O[3] = b3[a];
  assign O[4] = b4[a];
  assign O[5] = b5[a];
  assign O[6] = b6[a];
  assign O[7] = b7[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D[0]; b1[a] <= D[1]; b2[a] <= D[2]; b3[a] <= D[3]; b4[a] <= D[4]; b5[a] <= D[5]; b6[a] <= D[6]; b7[a] <= D[7]; end
endmodule

module RAM64X2S #(parameter [63:0] INIT_00 = 64'h0, parameter [63:0] INIT_01 = 64'h0) (output O0, output O1, input A0, input A1, input A2, input A3, input A4, input A5, input D0, input D1, input WCLK, input WE);
  wire [5:0] a = {A5, A4, A3, A2, A1, A0};
  reg [63:0] b0 = INIT_00;
  reg [63:0] b1 = INIT_01;
  assign O0 = b0[a];
  assign O1 = b1[a];
  always @(posedge WCLK) if (WE) begin b0[a] <= D0; b1[a] <= D1; end
endmodule

module RAM32M #(parameter [63:0] INIT_A = 64'h0, parameter [63:0] INIT_B = 64'h0, parameter [63:0] INIT_C = 64'h0,
  parameter [63:0] INIT_D = 64'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0)
  (output [1:0] DOA, output [1:0] DOB, output [1:0] DOC, output [1:0] DOD, input [4:0] ADDRA, input [4:0] ADDRB,
   input [4:0] ADDRC, input [4:0] ADDRD, input [1:0] DIA, input [1:0] DIB, input [1:0] DIC, input [1:0] DID,
   input WCLK, input WE);
  reg [63:0] a = INIT_A;
  reg [63:0] b = INIT_B;
  reg [63:0] c = INIT_C;
  reg [63:0] d = INIT_D;
  wire [5:0] ra = {ADDRA, 1'b0};
  wire [5:0] rb = {ADDRB, 1'b0};
  wire [5:0] rc = {ADDRC, 1'b0};
  wire [5:0] rd = {ADDRD, 1'b0};
  wire [5:0] w0 = {ADDRD, 1'b0};
  wire [5:0] w1 = {ADDRD, 1'b1};
  assign DOA = {a[ra | 6'd1], a[ra]};
  assign DOB = {b[rb | 6'd1], b[rb]};
  assign DOC = {c[rc | 6'd1], c[rc]};
  assign DOD = {d[rd | 6'd1], d[rd]};
  wire clk = WCLK ^ IS_WCLK_INVERTED;
  always @(posedge clk) if (WE) begin
    a[w0] <= DIA[0]; a[w1] <= DIA[1];
    b[w0] <= DIB[0]; b[w1] <= DIB[1];
    c[w0] <= DIC[0]; c[w1] <= DIC[1];
    d[w0] <= DID[0]; d[w1] <= DID[1];
  end
endmodule
module RAM64M #(parameter [63:0] INIT_A = 64'h0, parameter [63:0] INIT_B = 64'h0, parameter [63:0] INIT_C = 64'h0,
  parameter [63:0] INIT_D = 64'h0, parameter [0:0] IS_WCLK_INVERTED = 1'b0)
  (output DOA, output DOB, output DOC, output DOD, input [5:0] ADDRA, input [5:0] ADDRB, input [5:0] ADDRC,
   input [5:0] ADDRD, input DIA, input DIB, input DIC, input DID, input WCLK, input WE);
  reg [63:0] a = INIT_A;
  reg [63:0] b = INIT_B;
  reg [63:0] c = INIT_C;
  reg [63:0] d = INIT_D;
  assign DOA = a[ADDRA];
  assign DOB = b[ADDRB];
  assign DOC = c[ADDRC];
  assign DOD = d[ADDRD];
  wire clk = WCLK ^ IS_WCLK_INVERTED;
  always @(posedge clk) if (WE) begin
    a[ADDRD] <= DIA;
    b[ADDRD] <= DIB;
    c[ADDRD] <= DIC;
    d[ADDRD] <= DID;
  end
endmodule

module RAM32M_1 #(parameter [63:0] INIT_A = 64'h0, parameter [63:0] INIT_B = 64'h0, parameter [63:0] INIT_C = 64'h0,
  parameter [63:0] INIT_D = 64'h0)
  (output [1:0] DOA, output [1:0] DOB, output [1:0] DOC, output [1:0] DOD, input [4:0] ADDRA, input [4:0] ADDRB,
   input [4:0] ADDRC, input [4:0] ADDRD, input [1:0] DIA, input [1:0] DIB, input [1:0] DIC, input [1:0] DID,
   input WCLK, input WE);
  RAM32M #(.INIT_A(INIT_A), .INIT_B(INIT_B), .INIT_C(INIT_C), .INIT_D(INIT_D), .IS_WCLK_INVERTED(1'b1)) r (
    .DOA(DOA), .DOB(DOB), .DOC(DOC), .DOD(DOD), .ADDRA(ADDRA), .ADDRB(ADDRB), .ADDRC(ADDRC), .ADDRD(ADDRD),
    .DIA(DIA), .DIB(DIB), .DIC(DIC), .DID(DID), .WCLK(WCLK), .WE(WE));
endmodule
module RAM64M_1 #(parameter [63:0] INIT_A = 64'h0, parameter [63:0] INIT_B = 64'h0, parameter [63:0] INIT_C = 64'h0,
  parameter [63:0] INIT_D = 64'h0)
  (output DOA, output DOB, output DOC, output DOD, input [5:0] ADDRA, input [5:0] ADDRB, input [5:0] ADDRC,
   input [5:0] ADDRD, input DIA, input DIB, input DIC, input DID, input WCLK, input WE);
  RAM64M #(.INIT_A(INIT_A), .INIT_B(INIT_B), .INIT_C(INIT_C), .INIT_D(INIT_D), .IS_WCLK_INVERTED(1'b1)) r (
    .DOA(DOA), .DOB(DOB), .DOC(DOC), .DOD(DOD), .ADDRA(ADDRA), .ADDRB(ADDRB), .ADDRC(ADDRC), .ADDRD(ADDRD),
    .DIA(DIA), .DIB(DIB), .DIC(DIC), .DID(DID), .WCLK(WCLK), .WE(WE));
endmodule
module RAM128X1D_1 #(parameter [127:0] INIT = 128'h0) (output DPO, output SPO, input [6:0] A, input [6:0] DPRA, input D, input WCLK, input WE);
  RAM128X1D #(.INIT(INIT), .IS_WCLK_INVERTED(1'b1)) r (.DPO(DPO), .SPO(SPO), .A(A), .DPRA(DPRA), .D(D), .WCLK(WCLK), .WE(WE));
endmodule

module ROM16X1 #(parameter [15:0] INIT = 16'h0) (output O, input A0, input A1, input A2, input A3);
  assign O = INIT[{A3, A2, A1, A0}];
endmodule

module ROM32X1 #(parameter [31:0] INIT = 32'h0) (output O, input A0, input A1, input A2, input A3, input A4);
  assign O = INIT[{A4, A3, A2, A1, A0}];
endmodule

module ROM64X1 #(parameter [63:0] INIT = 64'h0) (output O, input A0, input A1, input A2, input A3, input A4, input A5);
  assign O = INIT[{A5, A4, A3, A2, A1, A0}];
endmodule

module ROM128X1 #(parameter [127:0] INIT = 128'h0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input A6);
  assign O = INIT[{A6, A5, A4, A3, A2, A1, A0}];
endmodule

module ROM256X1 #(parameter [255:0] INIT = 256'h0) (output O, input A0, input A1, input A2, input A3, input A4, input A5, input A6, input A7);
  assign O = INIT[{A7, A6, A5, A4, A3, A2, A1, A0}];
endmodule

module IOBUF (output O, inout IO, input I, input T);
  assign IO = T ? 1'bz : I;
  assign O = IO;
endmodule
module BUFGP (output O, input I);
  wire pad;
  IBUFG u_pad (.O(pad), .I(I));
  BUFG u_buf (.O(O), .I(pad));
endmodule
module BUFGCE_1 (output O, input CE, input I);
  wire n = ~I;
  BUFGCE u_buf (.O(O), .CE(CE), .I(n));
endmodule
module BUFGMUX_1 (output O, input I0, input I1, input S);
  BUFGMUX u_buf (.O(O), .I0(I0), .I1(I1), .S(S));
endmodule
module PLLE2_BASE #(parameter BANDWIDTH = "OPTIMIZED", parameter CLKFBOUT_MULT = 5,
    parameter CLKFBOUT_PHASE = 0.0, parameter CLKIN1_PERIOD = 0.0, parameter DIVCLK_DIVIDE = 1,
    parameter REF_JITTER1 = 0.01, parameter STARTUP_WAIT = "FALSE",
    parameter CLKOUT0_DIVIDE = 1, parameter CLKOUT1_DIVIDE = 1, parameter CLKOUT2_DIVIDE = 1,
    parameter CLKOUT3_DIVIDE = 1, parameter CLKOUT4_DIVIDE = 1, parameter CLKOUT5_DIVIDE = 1,
    parameter CLKOUT0_DUTY_CYCLE = 0.5, parameter CLKOUT1_DUTY_CYCLE = 0.5, parameter CLKOUT2_DUTY_CYCLE = 0.5,
    parameter CLKOUT3_DUTY_CYCLE = 0.5, parameter CLKOUT4_DUTY_CYCLE = 0.5, parameter CLKOUT5_DUTY_CYCLE = 0.5,
    parameter CLKOUT0_PHASE = 0.0, parameter CLKOUT1_PHASE = 0.0, parameter CLKOUT2_PHASE = 0.0,
    parameter CLKOUT3_PHASE = 0.0, parameter CLKOUT4_PHASE = 0.0, parameter CLKOUT5_PHASE = 0.0)
  (output CLKFBOUT, output CLKOUT0, output CLKOUT1, output CLKOUT2, output CLKOUT3, output CLKOUT4,
   output CLKOUT5, output LOCKED, input CLKFBIN, input CLKIN1, input PWRDWN, input RST);
  PLLE2_ADV #(.BANDWIDTH(BANDWIDTH), .CLKFBOUT_MULT(CLKFBOUT_MULT), .CLKFBOUT_PHASE(CLKFBOUT_PHASE),
    .CLKIN1_PERIOD(CLKIN1_PERIOD), .DIVCLK_DIVIDE(DIVCLK_DIVIDE), .REF_JITTER1(REF_JITTER1),
    .STARTUP_WAIT(STARTUP_WAIT), .CLKOUT0_DIVIDE(CLKOUT0_DIVIDE), .CLKOUT1_DIVIDE(CLKOUT1_DIVIDE),
    .CLKOUT2_DIVIDE(CLKOUT2_DIVIDE), .CLKOUT3_DIVIDE(CLKOUT3_DIVIDE), .CLKOUT4_DIVIDE(CLKOUT4_DIVIDE),
    .CLKOUT5_DIVIDE(CLKOUT5_DIVIDE), .CLKOUT0_DUTY_CYCLE(CLKOUT0_DUTY_CYCLE),
    .CLKOUT1_DUTY_CYCLE(CLKOUT1_DUTY_CYCLE), .CLKOUT2_DUTY_CYCLE(CLKOUT2_DUTY_CYCLE),
    .CLKOUT3_DUTY_CYCLE(CLKOUT3_DUTY_CYCLE), .CLKOUT4_DUTY_CYCLE(CLKOUT4_DUTY_CYCLE),
    .CLKOUT5_DUTY_CYCLE(CLKOUT5_DUTY_CYCLE), .CLKOUT0_PHASE(CLKOUT0_PHASE), .CLKOUT1_PHASE(CLKOUT1_PHASE),
    .CLKOUT2_PHASE(CLKOUT2_PHASE), .CLKOUT3_PHASE(CLKOUT3_PHASE), .CLKOUT4_PHASE(CLKOUT4_PHASE),
    .CLKOUT5_PHASE(CLKOUT5_PHASE))
    u_pll (.CLKFBOUT(CLKFBOUT), .CLKOUT0(CLKOUT0), .CLKOUT1(CLKOUT1), .CLKOUT2(CLKOUT2), .CLKOUT3(CLKOUT3),
      .CLKOUT4(CLKOUT4), .CLKOUT5(CLKOUT5), .LOCKED(LOCKED), .CLKFBIN(CLKFBIN), .CLKIN1(CLKIN1),
      .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'b0), .DCLK(1'b0), .DEN(1'b0), .DI(16'b0), .DWE(1'b0),
      .PWRDWN(PWRDWN), .RST(RST));
endmodule
module MMCME2_BASE #(parameter BANDWIDTH = "OPTIMIZED", parameter CLKFBOUT_MULT_F = 5.0,
    parameter CLKFBOUT_PHASE = 0.0, parameter CLKIN1_PERIOD = 0.0, parameter DIVCLK_DIVIDE = 1,
    parameter REF_JITTER1 = 0.01, parameter STARTUP_WAIT = "FALSE", parameter CLKOUT4_CASCADE = "FALSE",
    parameter CLKOUT0_DIVIDE_F = 1.0, parameter CLKOUT1_DIVIDE = 1, parameter CLKOUT2_DIVIDE = 1,
    parameter CLKOUT3_DIVIDE = 1, parameter CLKOUT4_DIVIDE = 1, parameter CLKOUT5_DIVIDE = 1, parameter CLKOUT6_DIVIDE = 1,
    parameter CLKOUT0_DUTY_CYCLE = 0.5, parameter CLKOUT1_DUTY_CYCLE = 0.5, parameter CLKOUT2_DUTY_CYCLE = 0.5,
    parameter CLKOUT3_DUTY_CYCLE = 0.5, parameter CLKOUT4_DUTY_CYCLE = 0.5, parameter CLKOUT5_DUTY_CYCLE = 0.5,
    parameter CLKOUT6_DUTY_CYCLE = 0.5, parameter CLKOUT0_PHASE = 0.0, parameter CLKOUT1_PHASE = 0.0,
    parameter CLKOUT2_PHASE = 0.0, parameter CLKOUT3_PHASE = 0.0, parameter CLKOUT4_PHASE = 0.0,
    parameter CLKOUT5_PHASE = 0.0, parameter CLKOUT6_PHASE = 0.0)
  (output CLKFBOUT, output CLKFBOUTB, output CLKOUT0, output CLKOUT0B, output CLKOUT1, output CLKOUT1B,
   output CLKOUT2, output CLKOUT2B, output CLKOUT3, output CLKOUT3B, output CLKOUT4, output CLKOUT5,
   output CLKOUT6, output LOCKED, input CLKFBIN, input CLKIN1, input PWRDWN, input RST);
  MMCME2_ADV #(.BANDWIDTH(BANDWIDTH), .CLKFBOUT_MULT_F(CLKFBOUT_MULT_F), .CLKFBOUT_PHASE(CLKFBOUT_PHASE),
    .CLKIN1_PERIOD(CLKIN1_PERIOD), .DIVCLK_DIVIDE(DIVCLK_DIVIDE), .REF_JITTER1(REF_JITTER1),
    .STARTUP_WAIT(STARTUP_WAIT), .CLKOUT4_CASCADE(CLKOUT4_CASCADE), .CLKOUT0_DIVIDE_F(CLKOUT0_DIVIDE_F),
    .CLKOUT1_DIVIDE(CLKOUT1_DIVIDE), .CLKOUT2_DIVIDE(CLKOUT2_DIVIDE), .CLKOUT3_DIVIDE(CLKOUT3_DIVIDE),
    .CLKOUT4_DIVIDE(CLKOUT4_DIVIDE), .CLKOUT5_DIVIDE(CLKOUT5_DIVIDE), .CLKOUT6_DIVIDE(CLKOUT6_DIVIDE),
    .CLKOUT0_DUTY_CYCLE(CLKOUT0_DUTY_CYCLE), .CLKOUT1_DUTY_CYCLE(CLKOUT1_DUTY_CYCLE),
    .CLKOUT2_DUTY_CYCLE(CLKOUT2_DUTY_CYCLE), .CLKOUT3_DUTY_CYCLE(CLKOUT3_DUTY_CYCLE),
    .CLKOUT4_DUTY_CYCLE(CLKOUT4_DUTY_CYCLE), .CLKOUT5_DUTY_CYCLE(CLKOUT5_DUTY_CYCLE),
    .CLKOUT6_DUTY_CYCLE(CLKOUT6_DUTY_CYCLE), .CLKOUT0_PHASE(CLKOUT0_PHASE), .CLKOUT1_PHASE(CLKOUT1_PHASE),
    .CLKOUT2_PHASE(CLKOUT2_PHASE), .CLKOUT3_PHASE(CLKOUT3_PHASE), .CLKOUT4_PHASE(CLKOUT4_PHASE),
    .CLKOUT5_PHASE(CLKOUT5_PHASE), .CLKOUT6_PHASE(CLKOUT6_PHASE))
    u_mmcm (.CLKFBOUT(CLKFBOUT), .CLKFBOUTB(CLKFBOUTB), .CLKOUT0(CLKOUT0), .CLKOUT0B(CLKOUT0B),
      .CLKOUT1(CLKOUT1), .CLKOUT1B(CLKOUT1B), .CLKOUT2(CLKOUT2), .CLKOUT2B(CLKOUT2B), .CLKOUT3(CLKOUT3),
      .CLKOUT3B(CLKOUT3B), .CLKOUT4(CLKOUT4), .CLKOUT5(CLKOUT5), .CLKOUT6(CLKOUT6), .LOCKED(LOCKED),
      .CLKFBIN(CLKFBIN), .CLKIN1(CLKIN1), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'b0), .DCLK(1'b0),
      .DEN(1'b0), .DI(16'b0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0), .PWRDWN(PWRDWN), .RST(RST));
endmodule
module BUFMR (output O, input I);
  BUFMRCE_REGION u_buf (.O(O), .CE(1'b1), .I(I));
endmodule
module BUFH (output O, input I);
  BUFHCE u_buf (.O(O), .CE(1'b1), .I(I));
endmodule
module BUFGMUX_CTRL (output O, input I0, input I1, input S);
  BUFGCTRL #(.PRESELECT_I0("TRUE"), .IS_S0_INVERTED(1'b1)) u_buf (.O(O), .I0(I0), .I1(I1),
    .CE0(1'b1), .CE1(1'b1), .S0(S), .S1(S), .IGNORE0(1'b0), .IGNORE1(1'b0));
endmodule
module IBUFGDS #(parameter DIFF_TERM = "FALSE", parameter IBUF_LOW_PWR = "TRUE",
    parameter IOSTANDARD = "DEFAULT", parameter CAPACITANCE = "DONT_CARE",
    parameter IBUF_DELAY_VALUE = "0") (output O, input I, input IB);
  IBUFDS #(.DIFF_TERM(DIFF_TERM), .IBUF_LOW_PWR(IBUF_LOW_PWR), .IOSTANDARD(IOSTANDARD))
    u_pad (.O(O), .I(I), .IB(IB));
endmodule
