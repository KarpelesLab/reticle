#!/usr/bin/env python3
"""Writes src/fpga/devices/xc7_cells.v, by families regular enough that writing
them out by hand invites a typo in one of forty."""
import os
out=[]
def m(text): out.append(text.strip('\n')+'\n')
m("""// 7-series cells a design may instantiate by name, as ordinary Verilog.
//
// Written from what each cell does (UG953), and checked against the vendor's
// simulation model: every module here is run on the vectors in
// testdata/fpga/vivado/cells/, whose answers came from Vivado's simulator
// (tests/fpga_xray_vivado_cells.rs). None of the vendor's source is here.
//
// Elaboration adds a module from this file only when a design instantiates
// it and declares nothing of that name, and synthesis then maps it like any
// other logic: what the part ends up holding need not be the named cell.
""")
for n in range(1,7):
    w=1<<n
    ports=', '.join(f'I{i}' for i in range(n))
    index=', '.join(f'I{i}' for i in reversed(range(n)))
    m(f"module LUT{n} #(parameter [{w-1}:0] INIT = {w}'h0) (output O, input {ports});\n  assign O = INIT[{{{index}}}];\nendmodule")
    m(f"module LUT{n}_L #(parameter [{w-1}:0] INIT = {w}'h0) (output LO, input {ports});\n  assign LO = INIT[{{{index}}}];\nendmodule")
    m(f"module LUT{n}_D #(parameter [{w-1}:0] INIT = {w}'h0) (output O, output LO, input {ports});\n  assign O = INIT[{{{index}}}];\n  assign LO = O;\nendmodule")
m("""module LUT6_2 #(parameter [63:0] INIT = 64'h0) (output O5, output O6, input I0, I1, I2, I3, I4, I5);
  wire [31:0] low = INIT[31:0];
  assign O5 = low[{I4, I3, I2, I1, I0}];
  assign O6 = INIT[{I5, I4, I3, I2, I1, I0}];
endmodule""")
for name,expr in (('AND','&'),('NAND','~&'),('OR','|'),('NOR','~|'),('XOR','^'),('XNOR','~^')):
    for n in range(2,6):
        ports=', '.join(f'I{i}' for i in range(n))
        cat=', '.join(f'I{i}' for i in reversed(range(n)))
        m(f"module {name}{n} (output O, input {ports});\n  assign O = {expr}{{{cat}}};\nendmodule")
        if name in('XOR','XNOR'): continue
        for b in range(1,n+1):
            cat=', '.join((f'~I{i}' if i<b else f'I{i}') for i in reversed(range(n)))
            m(f"module {name}{n}B{b} (output O, input {ports});\n  assign O = {expr}{{{cat}}};\nendmodule")
m("""module INV (output O, input I);
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
endmodule""")
for mux in ('MUXF5','MUXF6'):
    m(f"module {mux} (output O, input I0, input I1, input S);\n  assign O = S ? I1 : I0;\nendmodule")
    m(f"module {mux}_L (output LO, input I0, input I1, input S);\n  assign LO = S ? I1 : I0;\nendmodule")
    m(f"module {mux}_D (output O, output LO, input I0, input I1, input S);\n  assign O = S ? I1 : I0;\n  assign LO = O;\nendmodule")
# These two are the slice's own, and a build keeps a named one as the device's
# cell, so the others of their families are written through them.
for mux in ('MUXF7','MUXF8'):
    m(f"module {mux} (output O, input I0, input I1, input S);\n  assign O = S ? I1 : I0;\nendmodule")
    m(f"module {mux}_L (output LO, input I0, input I1, input S);\n  {mux} m (.O(LO), .I0(I0), .I1(I1), .S(S));\nendmodule")
    m(f"module {mux}_D (output O, output LO, input I0, input I1, input S);\n  {mux} m (.O(O), .I0(I0), .I1(I1), .S(S));\n  assign LO = O;\nendmodule")
m("""module CARRY4 (output [3:0] CO, output [3:0] O, input CI, input CYINIT, input [3:0] DI, input [3:0] S);
  wire c0 = CI | CYINIT;
  assign CO[0] = S[0] ? c0 : DI[0];
  assign CO[1] = S[1] ? CO[0] : DI[1];
  assign CO[2] = S[2] ? CO[1] : DI[2];
  assign CO[3] = S[3] ? CO[2] : DI[3];
  assign O = S ^ {CO[2:0], c0};
endmodule""")
m("""module MUXCY (output O, input CI, input DI, input S);
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
endmodule""")
# Flip-flops: the 7-series four, each with its inversions, then the legacy
# forms Vivado retargets onto them.
def ff(name, clk='posedge', rst=None, sync=True, ce=True, init=0, inv=True):
    """rst: None, ('R',0) reset to zero, ('S',1) set to one; sync or async."""
    ports=['input C']+(['input CE'] if ce else [])+(['input '+rst[0]] if rst else [])+['input D']
    params=[f"parameter [0:0] INIT = 1'b{init}"]
    pins=(['C','D']+([rst[0]] if rst else [])+(['CE'] if False else [])) if inv else []
    for p in pins: params.append(f"parameter [0:0] IS_{p}_INVERTED = 1'b0")
    d=' ^ IS_D_INVERTED' if inv else ''
    r=f'{rst[0]}' + (f' ^ IS_{rst[0]}_INVERTED' if inv else '') if rst else None
    ce_='CE' if ce else "1'b1"
    body=[f"  initial Q = INIT;"]
    if inv and (rst is None or sync):
        edges=[('posedge' if clk=='posedge' else 'negedge', 'negedge' if clk=='posedge' else 'posedge')]
    def block(edge, rsttxt, inv_rst):
        if rst is None:
            return f"  always @({edge} C) if ({ce_}) Q <= D{d};"
        if sync:
            return f"  always @({edge} C) if ({rsttxt}) Q <= 1'b{rst[1]}; else if ({ce_}) Q <= D{d};"
        rstedge='negedge' if inv_rst else 'posedge'
        cond=f'!{rst[0]}' if inv_rst else rst[0]
        return f"  always @({edge} C or {rstedge} {rst[0]}) if ({cond}) Q <= 1'b{rst[1]}; else if ({ce_}) Q <= D{d};"
    other='negedge' if clk=='posedge' else 'posedge'
    if inv:
        rinv = f'IS_{rst[0]}_INVERTED' if rst and not sync else None
        if rinv:
            body.append(f"  wire async = 1'b0;")
            body.pop()
            body.append("  generate")
            body.append(f"    if (IS_C_INVERTED && {rinv}) begin\n    {block(other, r, True)}\n    end else if (IS_C_INVERTED) begin\n    {block(other, r, False)}\n    end else if ({rinv}) begin\n    {block(clk, r, True)}\n    end else begin\n    {block(clk, r, False)}\n    end")
            body.append("  endgenerate")
        else:
            body.append("  generate")
            body.append(f"    if (IS_C_INVERTED) begin\n    {block(other, r, False)}\n    end else begin\n    {block(clk, r, False)}\n    end")
            body.append("  endgenerate")
    else:
        body.append(block(clk, r, False))
    m(f"module {name} #({', '.join(params)}) (output reg Q, {', '.join(ports)});\n"+'\n'.join(body)+"\nendmodule")
ff('FDRE',rst=('R',0)); ff('FDSE',rst=('S',1),init=1); ff('FDCE',rst=('CLR',0),sync=False); ff('FDPE',rst=('PRE',1),sync=False,init=1)
ff('FDRE_1',clk='negedge',rst=('R',0),inv=False); ff('FDSE_1',clk='negedge',rst=('S',1),init=1,inv=False); ff('FDCE_1',clk='negedge',rst=('CLR',0),sync=False,inv=False); ff('FDPE_1',clk='negedge',rst=('PRE',1),sync=False,init=1,inv=False)
for name,kw in {'FD':dict(ce=False),'FD_1':dict(ce=False,clk='negedge'),'FDE':{},'FDE_1':dict(clk='negedge'),
  'FDR':dict(ce=False,rst=('R',0)),'FDR_1':dict(ce=False,rst=('R',0),clk='negedge'),
  'FDS':dict(ce=False,rst=('S',1),init=1),'FDS_1':dict(ce=False,rst=('S',1),init=1,clk='negedge'),
  'FDC':dict(ce=False,rst=('CLR',0),sync=False),'FDC_1':dict(ce=False,rst=('CLR',0),sync=False,clk='negedge'),
  'FDP':dict(ce=False,rst=('PRE',1),sync=False,init=1),'FDP_1':dict(ce=False,rst=('PRE',1),sync=False,init=1,clk='negedge')}.items():
    ff(name,inv=False,**kw)
# Latches.
def ld(name,gate_low=False,ge=False,rst=None,init=0,inv=True):
    ports=['input D','input G']+(['input GE'] if ge else [])+(['input '+rst[0]] if rst else [])
    params=[f"parameter [0:0] INIT = 1'b{init}"]
    pins=(['G']+([rst[0]] if rst else [])) if inv else []
    for p in pins: params.append(f"parameter [0:0] IS_{p}_INVERTED = 1'b0")
    g='G'+(' ^ IS_G_INVERTED' if inv else ''); g=f"!({g})" if gate_low else f"({g})"
    en=f"{g} && GE" if ge else g
    dd='D'
    r=None
    if rst: r=rst[0]+(f' ^ IS_{rst[0]}_INVERTED' if inv else '')
    head=f"if ({r}) Q = 1'b{rst[1]}; else " if rst else ''
    m(f"module {name} #({', '.join(params)}) (output reg Q, {', '.join(ports)});\n  initial Q = INIT;\n  always @* {head}if ({en}) Q = {dd};\nendmodule")
ld('LDCE',ge=True,rst=('CLR',0)); ld('LDPE',ge=True,rst=('PRE',1),init=1)
ld('LDCE_1',gate_low=True,ge=True,rst=('CLR',0),inv=False); ld('LDPE_1',gate_low=True,ge=True,rst=('PRE',1),init=1,inv=False)
for name,kw in {'LD':{},'LD_1':dict(gate_low=True),'LDC':dict(rst=('CLR',0)),'LDC_1':dict(rst=('CLR',0),gate_low=True),
  'LDP':dict(rst=('PRE',1),init=1),'LDP_1':dict(rst=('PRE',1),init=1,gate_low=True),'LDE':dict(ge=True),'LDE_1':dict(ge=True,gate_low=True)}.items():
    ld(name,inv=False,**kw)
# Shift registers.
def srl(name, depth, ce, cascade, neg):
    bits=5 if depth==32 else 4
    if depth==32:
        addr='input [4:0] A'; idx='A'
    else:
        addr='input A0, input A1, input A2, input A3'; idx='{A3, A2, A1, A0}'
    ports=['output Q']+(['output Q15' if depth==16 else 'output Q31'] if cascade else [])+[addr]+(['input CE'] if ce else [])
    ck='CLK'; ports+=[f'input {ck}','input D']
    inv=f"parameter [0:0] IS_CLK_INVERTED = 1'b0" if name in('SRL16E','SRLC16E','SRLC32E') else None
    params=[f"parameter [{depth-1}:0] INIT = {depth}'h0"]+([inv] if inv else [])
    edge='negedge' if neg else 'posedge'
    upd=f"if ({'CE' if ce else chr(39).join(['1','b1'])}) r <= {{r[{depth-2}:0], D}};"
    if inv:
        alw=f"  generate\n    if (IS_CLK_INVERTED) begin\n      always @(negedge {ck}) {upd}\n    end else begin\n      always @(posedge {ck}) {upd}\n    end\n  endgenerate"
    else:
        alw=f"  always @({edge} {ck}) {upd}"
    extra=(f"  assign Q{depth-1} = r[{depth-1}];\n" if cascade else '')
    m(f"module {name} #({', '.join(params)}) ({', '.join(ports)});\n  reg [{depth-1}:0] r = INIT;\n  assign Q = r[{idx}];\n{extra}{alw}\nendmodule")
srl('SRL16',16,False,False,False); srl('SRL16_1',16,False,False,True); srl('SRL16E',16,True,False,False); srl('SRL16E_1',16,True,False,True)
srl('SRLC16',16,False,True,False); srl('SRLC16_1',16,False,True,True); srl('SRLC16E',16,True,True,False); srl('SRLC16E_1',16,True,True,True)
srl('SRLC32E',32,True,True,False)
m("""module SRL32E #(parameter [31:0] INIT = 32'h0) (output Q, input [4:0] A, input CE, input CLK, input D);
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
endmodule""")
# Distributed memories. One bit wide, single and dual port.
def adr(prefix,n): return ', '.join(f'{prefix}{i}' for i in reversed(range(n)))
def inp(prefix,n): return ', '.join(f'input {prefix}{i}' for i in range(n))
def write(ck,neg,body,invp):
    if invp:
        return f"  generate\n    if (IS_WCLK_INVERTED) begin\n      always @(negedge {ck}) {body}\n    end else begin\n      always @(posedge {ck}) {body}\n    end\n  endgenerate"
    return f"  always @({'negedge' if neg else 'posedge'} {ck}) {body}"
for words,n,invp in ((16,4,False),(32,5,True),(64,6,True),(128,7,True),(256,8,False),(512,9,False)):
    i=f"parameter [{words-1}:0] INIT = {words}'h0"+(", parameter [0:0] IS_WCLK_INVERTED = 1'b0" if invp else '')
    if words<=256:
      addrp=(f'input [{n-1}:0] A' if words>=256 else inp('A',n)); a=('A' if words>=256 else '{'+adr('A',n)+'}')
      m(f"module RAM{words}X1S #({i}) (output O, {addrp}, input D, input WCLK, input WE);\n  reg [{words-1}:0] mem = INIT;\n  assign O = mem[{a}];\n{write('WCLK',False,f'if (WE) mem[{a}] <= D;',invp)}\nendmodule")
    if words<=128:
      m(f"module RAM{words}X1S_1 #(parameter [{words-1}:0] INIT = {words}'h0) (output O, {inp('A',n)}, input D, input WCLK, input WE);\n  reg [{words-1}:0] mem = INIT;\n  assign O = mem[{{{adr('A',n)}}}];\n  always @(negedge WCLK) if (WE) mem[{{{adr('A',n)}}}] <= D;\nendmodule")
    if words<=128 or words==256:
      if words>=128:
        addrp=f'input [{n-1}:0] A, input [{n-1}:0] DPRA'; a='A'; b='DPRA'
      else:
        addrp=f"{inp('A',n)}, {inp('DPRA',n)}"; a='{'+adr('A',n)+'}'; b='{'+adr('DPRA',n)+'}'
      m(f"module RAM{words}X1D #({i}) (output DPO, output SPO, {addrp}, input D, input WCLK, input WE);\n  reg [{words-1}:0] mem = INIT;\n  assign SPO = mem[{a}];\n  assign DPO = mem[{b}];\n{write('WCLK',False,f'if (WE) mem[{a}] <= D;',invp)}\nendmodule")
    if words<=64:
      m(f"module RAM{words}X1D_1 #(parameter [{words-1}:0] INIT = {words}'h0) (output DPO, output SPO, {inp('A',n)}, {inp('DPRA',n)}, input D, input WCLK, input WE);\n  reg [{words-1}:0] mem = INIT;\n  assign SPO = mem[{{{adr('A',n)}}}];\n  assign DPO = mem[{{{adr('DPRA',n)}}}];\n  always @(negedge WCLK) if (WE) mem[{{{adr('A',n)}}}] <= D;\nendmodule")
# Wider single-port shapes: an INIT per bit. The 8-bit ones have vector data,
# the others a pin per bit, which is how the vendor's library has them.
for words,n,width in ((16,4,2),(16,4,4),(16,4,8),(32,5,2),(32,5,4),(32,5,8),(64,6,2)):
    params=', '.join(f"parameter [{words-1}:0] INIT_{b:02d} = {words}'h0" for b in range(width))
    rows='\n'.join(f"  reg [{words-1}:0] b{b} = INIT_{b:02d};" for b in range(width))
    if width==8:
        pins=f"output [7:0] O, {inp('A',n)}, input [7:0] D"
        reads='\n'.join(f"  assign O[{b}] = b{b}[a];" for b in range(width))
        writes=' '.join(f"b{b}[a] <= D[{b}];" for b in range(width))
    else:
        pins=', '.join(f"output O{b}" for b in range(width))+f", {inp('A',n)}, "+', '.join(f"input D{b}" for b in range(width))
        reads='\n'.join(f"  assign O{b} = b{b}[a];" for b in range(width))
        writes=' '.join(f"b{b}[a] <= D{b};" for b in range(width))
    m(f"module RAM{words}X{width}S #({params}) ({pins}, input WCLK, input WE);\n  wire [{n-1}:0] a = {{{adr('A',n)}}};\n{rows}\n{reads}\n  always @(posedge WCLK) if (WE) begin {writes} end\nendmodule")
# The multi-port memories of one SLICEM.
ports='ABCD'
def ramm(name,words,n,width,names,addr_shape):
    pass
m("""module RAM32M #(parameter [63:0] INIT_A = 64'h0, parameter [63:0] INIT_B = 64'h0, parameter [63:0] INIT_C = 64'h0,
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
endmodule""")
for words,n in ((16,4),(32,5),(64,6),(128,7),(256,8)):
    p=f"parameter [{words-1}:0] INIT = {words}'h0"
    addrp=(f'input [{n-1}:0] A' if words==256 else inp('A',n)); a=('A' if words==256 else '{'+adr('A',n)+'}')
    if words==256: addrp=inp('A',n); a='{'+adr('A',n)+'}'
    m(f"module ROM{words}X1 #({p}) (output O, {addrp});\n  assign O = INIT[{a}];\nendmodule")
# Pads and clock buffers built from what the flow already has.
m("""module IOBUF (output O, inout IO, input I, input T);
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
endmodule""")
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../src/fpga/devices/xc7_cells.v'),'w').write('\n'.join(out))
