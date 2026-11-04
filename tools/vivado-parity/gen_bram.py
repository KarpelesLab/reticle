#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/bram/: `gen_bram.py <dir>`.

Each asks Vivado one question about a block RAM or a FIFO: one cell by name with
every input from a flip-flop of its own and random contents, so that no two bits
are alike, or a memory to infer. `FIXTURE=bram lutram.sh <out>` runs them and
keeps what came back. Only the answers are committed; these designs are written
here.
"""
import os
import random
import sys

XDC = """create_clock -name clk -period 5 [get_ports clk]
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN L4 [get_ports si]
set_property PACKAGE_PIN L5 [get_ports so]
set_property IOSTANDARD LVCMOS33 [get_ports {clk si so}]
"""
RCF = """create_clock -name clk -period 5 clk
set_io -io_standard LVCMOS33 clk N11
set_io -io_standard LVCMOS33 si L4
set_io -io_standard LVCMOS33 so L5
"""

rng = random.Random(0xB4A3)


def hexv(bits):
    return f"{bits}'h{rng.getrandbits(bits):0{(bits + 3) // 4}x}"


def cell_design(name, comment, cell, params, inputs, outputs, clocks, body_extra="", fixed=(), wires=""):
    """A design around one cell: `inputs` (port, width) from a shift register,
    `outputs` (port, width) folded into `so`, `clocks` the ports on `clk`, and
    `fixed` (port, value) what Vivado's rules want tied: a cascade input not
    driven from logic, the top address bit of a lone RAMB36E1 at one, and a
    write enable a mode does not read at zero."""
    total = sum(w for _, w in inputs)
    conns, at = [], 0
    for port, width in inputs:
        if port.startswith("_"):
            pass
        elif width == 1:
            conns.append(f".{port}(sr[{at}])")
        else:
            conns.append(f".{port}(sr[{at + width - 1}:{at}])")
        at += width
    for port, value in fixed:
        conns.append(f".{port}({value})")
    for port in clocks:
        conns.append(f".{port}(clk)")
    wire_lines, folds = [], []
    for port, width in outputs:
        wire = "o_" + port.lower()
        wire_lines.append(f"  wire [{width - 1}:0] {wire};\n")
        conns.append(f".{port}({wire})")
        folds.append(f"^{wire}")
    plist = ",\n    ".join(f".{k}({v})" for k, v in params)
    top = max(total, 1)
    return (
        f"// {comment}\n"
        f"module {name} (\n"
        "    input  wire clk,\n"
        "    input  wire si,\n"
        "    output reg  so\n"
        ");\n"
        f"  reg [{top}:0] sr;\n"
        f"  always @(posedge clk) sr <= {{sr[{top - 1}:0], si}};\n"
        + "".join(wire_lines)
        + wires
        + f"  {cell} #(\n    {plist}) dut (\n    {', '.join(conns)});\n"
        + body_extra
        + f"  always @(posedge clk) so <= {' ^ '.join(folds)} ^ sr[{top}];\n"
        "endmodule\n"
    )


def contents(count, prefix):
    return [(f"{prefix}{i:02X}", hexv(256)) for i in range(count)]


def ramb18(name, comment, ra, wa, rb, wb, mode="TDP", extra=(), init=True):
    params = [("RAM_MODE", f'"{mode}"'), ("READ_WIDTH_A", ra), ("WRITE_WIDTH_A", wa),
              ("READ_WIDTH_B", rb), ("WRITE_WIDTH_B", wb), ("SIM_DEVICE", '"7SERIES"')]
    params += list(extra)
    if init:
        params += contents(64, "INIT_") + contents(8, "INITP_")
    inputs = [("ADDRARDADDR", 14), ("ADDRBWRADDR", 14), ("DIADI", 16), ("DIBDI", 16),
              ("DIPADIP", 2), ("DIPBDIP", 2), ("ENARDEN", 1), ("ENBWREN", 1),
              ("REGCEAREGCE", 1), ("REGCEB", 1), ("RSTRAMARSTRAM", 1), ("RSTRAMB", 1),
              ("RSTREGARSTREG", 1), ("RSTREGB", 1), ("WEBWE", 4)]
    fixed = []
    if mode == "SDP":
        fixed.append(("WEA", "2'b00"))
    else:
        inputs.append(("WEA", 2))
    outputs = [("DOADO", 16), ("DOBDO", 16), ("DOPADOP", 2), ("DOPBDOP", 2)]
    return name, cell_design(name, comment, "RAMB18E1", params, inputs, outputs,
                             ["CLKARDCLK", "CLKBWRCLK"], fixed=fixed)


def ramb36(name, comment, ra, wa, rb, wb, mode="TDP", extra=(), init=True, cascade=None):
    params = [("RAM_MODE", f'"{mode}"'), ("READ_WIDTH_A", ra), ("WRITE_WIDTH_A", wa),
              ("READ_WIDTH_B", rb), ("WRITE_WIDTH_B", wb), ("SIM_DEVICE", '"7SERIES"')]
    params += list(extra)
    if init:
        params += contents(128, "INIT_") + contents(16, "INITP_")
    inputs = [("DIADI", 32), ("DIBDI", 32),
              ("DIPADIP", 4), ("DIPBDIP", 4), ("ENARDEN", 1), ("ENBWREN", 1),
              ("REGCEAREGCE", 1), ("REGCEB", 1), ("RSTRAMARSTRAM", 1), ("RSTRAMB", 1),
              ("RSTREGARSTREG", 1), ("RSTREGB", 1), ("WEBWE", 8)]
    # Vivado's rules (PDCN-135, PDCN-136, DXSTAT-1): with no cascade the top
    # address bit is tied high and the cascade inputs are not from logic.
    total = sum(w for _, w in inputs)
    body = (f"  wire [15:0] addra = {{1'b1, sr[{total + 14}:{total}]}};\n"
            f"  wire [15:0] addrb = {{1'b1, sr[{total + 29}:{total + 15}]}};\n")
    inputs += [("_ADDRA", 15), ("_ADDRB", 15)]
    fixed = [("ADDRARDADDR", "addra"), ("ADDRBWRADDR", "addrb"),
             ("CASCADEINA", "1'b0"), ("CASCADEINB", "1'b0"),
             ("INJECTDBITERR", "1'b0"), ("INJECTSBITERR", "1'b0")]
    if mode == "SDP":
        fixed.append(("WEA", "4'b0000"))
    else:
        inputs.append(("WEA", 4))
    outputs = [("DOADO", 32), ("DOBDO", 32), ("DOPADOP", 4), ("DOPBDOP", 4)]
    return name, cell_design(name, comment, "RAMB36E1", params, inputs, outputs,
                             ["CLKARDCLK", "CLKBWRCLK"], fixed=fixed, wires=body)


def cascade36(name, comment):
    """Two RAMB36E1 as one 64K by 1 memory, through their cascade."""
    def half(inst, ext, cin):
        params = [("RAM_MODE", '"TDP"'), ("READ_WIDTH_A", 1), ("WRITE_WIDTH_A", 1),
                  ("READ_WIDTH_B", 1), ("WRITE_WIDTH_B", 1), ("SIM_DEVICE", '"7SERIES"'),
                  ("RAM_EXTENSION_A", f'"{ext}"'), ("RAM_EXTENSION_B", f'"{ext}"')]
        params += contents(128, "INIT_")
        plist = ",\n    ".join(f".{k}({v})" for k, v in params)
        casc = (".CASCADEINA(1'b0), .CASCADEINB(1'b0), .CASCADEOUTA(ca), .CASCADEOUTB(cb)"
                if cin is None else ".CASCADEINA(ca), .CASCADEINB(cb), .CASCADEOUTA(), .CASCADEOUTB()")
        out = ".DOADO(), .DOBDO()" if cin is None else ".DOADO(qa), .DOBDO(qb)"
        return (f"  RAMB36E1 #(\n    {plist}) {inst} (\n"
                f"    .ADDRARDADDR(sr[15:0]), .ADDRBWRADDR(sr[31:16]), .DIADI({{31'b0, sr[32]}}), .DIBDI({{31'b0, sr[33]}}),\n"
                f"    .DIPADIP(4'b0), .DIPBDIP(4'b0), .ENARDEN(sr[34]), .ENBWREN(sr[35]), .REGCEAREGCE(1'b1), .REGCEB(1'b1),\n"
                f"    .RSTRAMARSTRAM(1'b0), .RSTRAMB(1'b0), .RSTREGARSTREG(1'b0), .RSTREGB(1'b0),\n"
                f"    .WEA({{4{{sr[36]}}}}), .WEBWE({{8{{sr[37]}}}}), .CLKARDCLK(clk), .CLKBWRCLK(clk),\n"
                f"    .INJECTDBITERR(1'b0), .INJECTSBITERR(1'b0), {casc}, {out});\n")
    body = ("  wire ca, cb;\n  wire [31:0] qa, qb;\n" + half("lower", "LOWER", None)
            + half("upper", "UPPER", True))
    return name, (
        f"// {comment}\n"
        f"module {name} (\n    input  wire clk,\n    input  wire si,\n    output reg  so\n);\n"
        "  reg [38:0] sr;\n  always @(posedge clk) sr <= {sr[37:0], si};\n"
        + body + "  always @(posedge clk) so <= qa[0] ^ qb[0] ^ sr[38];\n" "endmodule\n")


def inferred(name, comment, width, depth, read="sync", regs=1, byte_we=False, tdp=False):
    abits = max(1, (depth - 1).bit_length())
    nwe = width // 8 if byte_we else 1
    top = 2 * abits + width + nwe + (width + 1 if tdp else 0)
    body = f"  reg [{width - 1}:0] mem [0:{depth - 1}];\n"
    wa, d = f"sr[{abits - 1}:0]", f"sr[{abits + width - 1}:{abits}]"
    ra = f"sr[{2 * abits + width - 1}:{abits + width}]"
    we0 = 2 * abits + width
    if byte_we:
        # One statement a byte, as the vendor's templates write it: a loop of
        # indexed part-selects made Vivado 2026.1's synthesis crash.
        body += f"  wire [{width - 1}:0] din = {d};\n  always @(posedge clk) begin\n"
        for i in range(nwe):
            body += f"    if (sr[{we0 + i}]) mem[{wa}][{8 * i + 7}:{8 * i}] <= din[{8 * i + 7}:{8 * i}];\n"
        body += "  end\n"
    else:
        body += f"  always @(posedge clk) if (sr[{we0}]) mem[{wa}] <= {d};\n"
    body += f"  reg [{width - 1}:0] q0;\n"
    if read == "sync":
        body += f"  always @(posedge clk) q0 <= mem[{ra}];\n"
    for r in range(1, regs):
        body += f"  reg [{width - 1}:0] q{r};\n  always @(posedge clk) q{r} <= q{r - 1};\n"
    last = f"q{regs - 1}"
    if tdp:
        # A second port writing and reading at its own address.
        b0 = we0 + nwe
        body += (f"  reg [{width - 1}:0] p0;\n"
                 f"  always @(posedge clk) begin if (sr[{b0}]) mem[{ra}] <= sr[{b0 + width}:{b0 + 1}]; p0 <= mem[{wa}]; end\n")
        last = f"{last} ^ p0"
    body += f"  always @(posedge clk) so <= ^({last}) ^ sr[{top}];\n"
    return name, (
        f"// {comment}\n"
        f"module {name} (\n    input  wire clk,\n    input  wire si,\n    output reg  so\n);\n"
        f"  reg [{top}:0] sr;\n  always @(posedge clk) sr <= {{sr[{top - 1}:0], si}};\n"
        + body + "endmodule\n")


def designs():
    out = []
    for w in (1, 2, 4, 9, 18):
        out.append(ramb18(f"b18_w{w}", f"A RAMB18E1 by name, both ports {w} wide.", w, w, w, w))
    out.append(ramb18("b18_a9_b18", "A RAMB18E1, port A 9 wide and port B 18.", 9, 9, 18, 18))
    out.append(ramb18("b18_sdp36", "A RAMB18E1 in simple dual port, 36 wide.", 36, 0, 0, 36, mode="SDP"))
    out.append(ramb18("b18_sdp_r36_w9", "A RAMB18E1 in simple dual port, read 36, written 9.", 36, 0, 0, 9, mode="SDP"))
    out.append(ramb18("b18_doreg", "A RAMB18E1 with both output registers.", 18, 18, 18, 18,
                      extra=[("DOA_REG", 1), ("DOB_REG", 1)]))
    out.append(ramb18("b18_read_first", "A RAMB18E1 reading first on both ports.", 18, 18, 18, 18,
                      extra=[("WRITE_MODE_A", '"READ_FIRST"'), ("WRITE_MODE_B", '"READ_FIRST"')]))
    out.append(ramb18("b18_no_change", "A RAMB18E1 with no change on write, both ports.", 18, 18, 18, 18,
                      extra=[("WRITE_MODE_A", '"NO_CHANGE"'), ("WRITE_MODE_B", '"NO_CHANGE"')]))
    out.append(ramb18("b18_srval", "A RAMB18E1 whose outputs start and reset to values of their own.", 18, 18, 18, 18,
                      extra=[("INIT_A", hexv(18)), ("INIT_B", hexv(18)), ("SRVAL_A", hexv(18)), ("SRVAL_B", hexv(18))]))
    out.append(ramb18("b18_doreg_srval", "A RAMB18E1 with output registers, reset values and REGCE first.", 18, 18, 18, 18,
                      extra=[("DOA_REG", 1), ("DOB_REG", 1), ("INIT_A", hexv(18)), ("SRVAL_B", hexv(18)),
                             ("RSTREG_PRIORITY_A", '"REGCE"'), ("RSTREG_PRIORITY_B", '"REGCE"')]))
    out.append(ramb18("b18_inv", "A RAMB18E1 with its clock, enable and resets inverted.", 18, 18, 18, 18,
                      extra=[("IS_CLKARDCLK_INVERTED", "1'b1"), ("IS_ENBWREN_INVERTED", "1'b1"),
                             ("IS_RSTRAMARSTRAM_INVERTED", "1'b1"), ("IS_RSTREGB_INVERTED", "1'b1")]))
    out.append(ramb18("b18_collision", "A RAMB18E1 with the performance collision mode.", 18, 18, 18, 18,
                      extra=[("RDADDR_COLLISION_HWCONFIG", '"PERFORMANCE"')]))
    for w in (1, 2, 4, 9, 18, 36):
        out.append(ramb36(f"b36_w{w}", f"A RAMB36E1 by name, both ports {w} wide.", w, w, w, w))
    out.append(ramb36("b36_a18_b36", "A RAMB36E1, port A 18 wide and port B 36.", 18, 18, 36, 36))
    out.append(ramb36("b36_sdp72", "A RAMB36E1 in simple dual port, 72 wide.", 72, 0, 0, 72, mode="SDP"))
    out.append(ramb36("b36_doreg", "A RAMB36E1 with both output registers.", 36, 36, 36, 36,
                      extra=[("DOA_REG", 1), ("DOB_REG", 1)]))
    out.append(ramb36("b36_read_first", "A RAMB36E1 reading first on both ports.", 36, 36, 36, 36,
                      extra=[("WRITE_MODE_A", '"READ_FIRST"'), ("WRITE_MODE_B", '"READ_FIRST"')]))
    out.append(ramb36("b36_srval", "A RAMB36E1 whose outputs start and reset to values of their own.", 36, 36, 36, 36,
                      extra=[("INIT_A", hexv(36)), ("INIT_B", hexv(36)), ("SRVAL_A", hexv(36)), ("SRVAL_B", hexv(36))]))
    out.append(ramb36("b36_inv", "A RAMB36E1 with its clock, enable and resets inverted.", 36, 36, 36, 36,
                      extra=[("IS_CLKBWRCLK_INVERTED", "1'b1"), ("IS_ENARDEN_INVERTED", "1'b1"),
                             ("IS_RSTRAMB_INVERTED", "1'b1"), ("IS_RSTREGARSTREG_INVERTED", "1'b1")]))
    out.append(cascade36("b36_cascade", "Two RAMB36E1 as one memory of 65,536 bits, through the cascade."))
    for name, width, depth, kw in [
        ("ib_1024x36", 36, 1024, {}),
        ("ib_512x36", 36, 512, {}),
        ("ib_512x72", 72, 512, {}),
        ("ib_1024x18", 18, 1024, {}),
        ("ib_2048x18", 18, 2048, {}),
        ("ib_4096x9", 9, 4096, {}),
        ("ib_8192x4", 4, 8192, {}),
        ("ib_32768x1", 1, 32768, {}),
        ("ib_1024x36_reg2", 36, 1024, {"regs": 2}),
        ("ib_1024x32_bytes", 32, 1024, {"byte_we": True}),
        ("ib_1024x18_tdp", 18, 1024, {"tdp": True}),
        ("ib_2048x36", 36, 2048, {}),
    ]:
        out.append(inferred(name, f"{depth} words of {width} to infer: {kw or 'one write, one registered read'}.",
                            width, depth, **kw))
    return out


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, text in designs():
        for ext, body in ((".v", text), (".xdc", XDC), (".rcf", RCF)):
            with open(os.path.join(out, name + ext), "w") as f:
                f.write(body)


main()
