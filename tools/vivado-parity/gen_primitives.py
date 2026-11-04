#!/usr/bin/env python3
"""Writes one probe per 7-series primitive into testdata/fpga/vivado/primitives.

Each probe instantiates a single primitive by name with its default
parameters, fed from a shift register and read through a parity flip-flop, so
that one design needs a few pins whatever the primitive has. The port lists
are read from the installed Vivado's own simulation library, and only the
names and widths are kept: nothing of a vendor's model is written out.

    tools/vivado-parity/gen_primitives.py ~/Xilinx/2026.1/Vivado
"""
import re, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "testdata/fpga/vivado/primitives")

# The 7-series primitives, by what they are. Device-independent names that
# only other families have are left out; `GT` and PCIe are listed so that the
# table says they were not forgotten, and have probes of their own to write.
GROUPS = {
    "logic": "LUT1 LUT2 LUT3 LUT4 LUT5 LUT6 LUT6_2 LUT1_L LUT6_L LUT6_D AND2 OR2 XOR2 XNOR2 NAND2 NOR2 "
             "AND2B1L OR2L INV BUF GND VCC MUXF7 MUXF8 MUXF7_L MUXF8_L MUXCY XORCY CARRY4",
    "register": "FDRE FDSE FDCE FDPE FD FDR FDS FDC FDP FDE FDRE_1 FDSE_1 FDCE_1 FDPE_1 "
                "LDCE LDPE LD LDC LDP LDCE_1 LDPE_1",
    "shift": "SRL16E SRLC16E SRLC32E SRL16 SRLC16 SRL16E_1 SRLC16E_1 CFGLUT5",
    "lutram": "RAM32X1S RAM32X1D RAM64X1S RAM64X1D RAM128X1S RAM128X1D RAM256X1S RAM32M RAM64M "
              "RAM16X1S RAM16X1D RAM32X2S RAM64X2S ROM16X1 ROM32X1 ROM64X1 ROM128X1 ROM256X1",
    "block": "RAMB18E1 RAMB36E1 FIFO18E1 FIFO36E1 DSP48E1",
    "clock": "BUFG BUFGCE BUFGCE_1 BUFGCTRL BUFGMUX BUFGMUX_1 BUFGMUX_CTRL BUFH BUFHCE BUFR BUFIO BUFMR BUFMRCE "
             "IBUFG BUFGP MMCME2_BASE MMCME2_ADV PLLE2_BASE PLLE2_ADV",
    "io": "IBUF OBUF OBUFT IOBUF IBUF_IBUFDISABLE IBUF_INTERMDISABLE IOBUF_INTERMDISABLE IOBUF_DCIEN "
          "OBUFT_DCIEN IBUFDS IBUFGDS OBUFDS OBUFTDS IOBUFDS IBUFDS_DIFF_OUT IBUFGDS_DIFF_OUT "
          "IBUFDS_IBUFDISABLE IBUFDS_INTERMDISABLE IBUFDS_DIFF_OUT_IBUFDISABLE IBUFDS_DIFF_OUT_INTERMDISABLE "
          "IOBUFDS_INTERMDISABLE IOBUFDS_DCIEN IOBUFDS_DIFF_OUT IOBUFDS_DIFF_OUT_INTERMDISABLE "
          "IOBUFDS_DIFF_OUT_DCIEN PULLUP PULLDOWN KEEPER",
    "iologic": "IDDR IDDR_2CLK ODDR ISERDESE2 OSERDESE2 IDELAYE2 IDELAYCTRL DCIRESET ZHOLD_DELAY",
    "config": "BSCANE2 CAPTUREE2 DNA_PORT EFUSE_USR FRAME_ECCE2 ICAPE2 STARTUPE2 USR_ACCESSE2 XADC",
}

# Which ports reach a pad, by primitive: top-level ports rather than the
# shift register, because a pad is the only thing that can drive them.
DIFF_IN = {"I": "I", "IB": "IB"}
PADS = {
    "IBUF": {"I"}, "IBUFG": {"I"}, "BUFGP": {"I"}, "OBUF": {"O"}, "OBUFT": {"O"}, "IOBUF": {"IO"},
    "IBUF_IBUFDISABLE": {"I"}, "IBUF_INTERMDISABLE": {"I"}, "IOBUF_INTERMDISABLE": {"IO"},
    "IOBUF_DCIEN": {"IO"}, "OBUFT_DCIEN": {"O"},
    "IBUFDS": {"I", "IB"}, "IBUFGDS": {"I", "IB"}, "OBUFDS": {"O", "OB"}, "OBUFTDS": {"O", "OB"},
    "IOBUFDS": {"IO", "IOB"}, "IBUFDS_DIFF_OUT": {"I", "IB"}, "IBUFGDS_DIFF_OUT": {"I", "IB"},
    "IBUFDS_IBUFDISABLE": {"I", "IB"}, "IBUFDS_INTERMDISABLE": {"I", "IB"},
    "IBUFDS_DIFF_OUT_IBUFDISABLE": {"I", "IB"}, "IBUFDS_DIFF_OUT_INTERMDISABLE": {"I", "IB"},
    "IOBUFDS_INTERMDISABLE": {"IO", "IOB"}, "IOBUFDS_DCIEN": {"IO", "IOB"}, "IOBUFDS_DIFF_OUT": {"IO", "IOB"},
    "IOBUFDS_DIFF_OUT_INTERMDISABLE": {"IO", "IOB"}, "IOBUFDS_DIFF_OUT_DCIEN": {"IO", "IOB"},
    "PULLUP": {"O"}, "PULLDOWN": {"O"}, "KEEPER": {"O"},
}
DIFFERENTIAL = {"IB", "OB", "IOB"}
# Clock inputs, which the probe's clock drives, by name.
CLOCK = re.compile(r"^(C|CK|CLK|CLKB|CLKDIV|CLKDIVP|WCLK|RDCLK|WRCLK|RDCLKU|RDCLKL|WRCLKU|WRCLKL|CLKARDCLK|CLKBWRCLK|"
                   r"CLKARDCLKL|CLKARDCLKU|CLKBWRCLKL|CLKBWRCLKU|CLKIN|CLKIN1|CLKIN2|CLKFBIN|C0|C1|D_CLK|DCLK|"
                   r"I0|I1|REFCLK|ICLK|OCLK|OCLKB|CLKFB|ISERDES_CLK|S0_CLK)$")
# `I0`, `I1` and `I` are clocks only on a clock buffer.
CLOCK_ONLY_ON = {"I0", "I1"}
BUFFERS = {"BUFG", "BUFGCE", "BUFGCE_1", "BUFGCTRL", "BUFGMUX", "BUFGMUX_1", "BUFGMUX_CTRL", "BUFH", "BUFHCE", "BUFR",
           "BUFIO", "BUFMR", "BUFMRCE"}


def strip_comments(text):
    return re.sub(r"//[^\n]*|/\*.*?\*/", " ", text, flags=re.S)


def ports_of(vivado, name):
    for sub in ("unisims", "retarget"):
        path = f"{vivado}/data/verilog/src/{sub}/{name}.v"
        if os.path.exists(path):
            text = strip_comments(open(path, errors="replace").read())
            break
    else:
        return None
    # Conditional compilation hides the timing blocks' pins; keep the else.
    text = re.sub(r"`ifdef XIL_TIMING.*?(`else|`endif)", " ", text, flags=re.S)
    module = re.search(rf"module\s+{name}\b.*?endmodule", text, flags=re.S).group(0)
    params = {}
    for m in re.finditer(r"parameter\s+(?:integer\s+|\[[^\]]*\]\s*)?(\w+)\s*=\s*(\d+)\s*[,;)\n]", module):
        params[m.group(1)] = int(m.group(2))
    body = re.split(r"\b(?:function|task|initial|always|assign)\b", module)[0]
    ports = []
    seen = set()
    for m in re.finditer(r"\b(input|output|inout)\b(.*?)(?=;|\)\s*;|\b(?:input|output|inout)\b)", body, flags=re.S):
        direction, rest = m.groups()
        rest = re.sub(r"\b(?:wire|reg|tri0|tri1|tri|signed)\b", " ", rest)
        rng = re.search(r"\[[^\]]*\]", rest)
        names = rest[rng.end():] if rng else rest
        width = 1
        if rng:
            hi, lo = rng.group(0)[1:-1].split(":")
            try:
                width = eval(hi, {}, params) - eval(lo, {}, params) + 1
            except Exception:
                width = 1
        for one in names.split(","):
            one = one.split("=")[0].strip()
            if re.fullmatch(r"\w+", one) and one not in seen:
                seen.add(one)
                ports.append((direction, one, width))
    return ports


def is_clock(prim, port):
    if port in CLOCK_ONLY_ON and prim not in BUFFERS:
        return False
    if port == "I" and prim in BUFFERS:
        return True
    return bool(CLOCK.match(port))


def parameters(prim):
    """What a probe sets so that the cell computes something: a lookup table
    or ROM left at its default of all zeros is a constant, and swept."""
    m = re.fullmatch(r"LUT(\d)(_L|_D)?", prim)
    if m:
        bits = 1 << int(m.group(1))
    elif prim == "LUT6_2":
        bits = 64
    elif prim.startswith("ROM"):
        bits = int(re.match(r"ROM(\d+)", prim).group(1))
    else:
        return ""
    value = 0
    for i in range(bits):
        value |= (bin(i).count("1") & 1) << i
    return f"#(.INIT({bits}'h{value:x})) "


def probe(prim, ports):
    pads = PADS.get(prim, set())
    inputs = 0
    outputs = 0
    conns = []
    top = []
    for direction, port, width in ports:
        if port in pads:
            top.append(f"{direction if direction != 'output' else 'output'} wire {('[%d:0] ' % (width-1)) if width > 1 else ''}{port.lower()}_pad")
            conns.append(f".{port}({port.lower()}_pad)")
        elif direction == "input" and is_clock(prim, port) and width == 1:
            conns.append(f".{port}(clk)")
        elif direction == "input":
            conns.append(f".{port}(sr[{inputs + width - 1}:{inputs}])" if width > 1 else f".{port}(sr[{inputs}])")
            inputs += width
        elif direction == "output":
            conns.append(f".{port}(out[{outputs + width - 1}:{outputs}])" if width > 1 else f".{port}(out[{outputs}])")
            outputs += width
        else:
            conns.append(f".{port}()")
    lines = [f"// {prim} by name, with default parameters. Generated by tools/vivado-parity/gen_primitives.py.",
             f"module {prim.lower()}_probe (", "    input  wire clk,", "    input  wire si,", "    output reg  so"]
    lines[-1] += ","
    for t in top:
        lines.append(f"    {t},")
    lines[-1] = lines[-1].rstrip(",")
    lines.append(");")
    lines.append(f"  reg [{max(inputs, 1) - 1}:0] sr;")
    lines.append(f"  wire [{max(outputs, 1) - 1}:0] out;")
    if inputs > 1:
        lines.append(f"  always @(posedge clk) sr <= {{sr[{inputs - 2}:0], si}};")
    else:
        lines.append("  always @(posedge clk) sr <= si;")
    if outputs == 0:
        lines.append("  assign out = sr;")
    # Registered first, so that a cell equal to `sr[0]` cannot cancel out.
    lines.append("  reg q;")
    lines.append("  always @(posedge clk) q <= ^out;")
    lines.append("  always @(posedge clk) so <= q ^ sr[0];")
    lines.append(f"  {prim} {parameters(prim)}dut ({', '.join(conns)});")
    lines.append("endmodule")
    return "\n".join(lines) + "\n", top


# Bank 34 at 3.3 V for single-ended pins, away from the configuration pins; a
# pair in bank 35 for differential.


def constraints(prim, top):
    names = [re.split(r"\s+", t)[-1] for t in top]
    single = ["M1", "M2", "M4", "M5"]
    xdc = ["create_clock -period 10 -name clk [get_ports clk]", "set_property PACKAGE_PIN N11 [get_ports clk]",
           "set_property PACKAGE_PIN L4 [get_ports si]", "set_property PACKAGE_PIN L5 [get_ports so]"]
    rcf = ["create_clock -name clk -period 10 clk", "set_io -io_standard LVCMOS33 clk N11",
           "set_io -io_standard LVCMOS33 si L4", "set_io -io_standard LVCMOS33 so L5"]
    xdc.append("set_property IOSTANDARD LVCMOS33 [get_ports {clk si so}]")
    diff = any(n[:-4].upper() in DIFFERENTIAL for n in names)
    for n in names:
        p = n[:-4].upper()
        if diff:
            continue
        pin = single.pop(0)
        xdc += [f"set_property PACKAGE_PIN {pin} [get_ports {n}]", f"set_property IOSTANDARD LVCMOS33 [get_ports {n}]"]
        rcf.append(f"set_io -io_standard LVCMOS33 {n} {pin}")
    if diff:
        master = names[0]
        xdc += [f"set_property PACKAGE_PIN A5 [get_ports {master}]", f"set_property IOSTANDARD LVDS_25 [get_ports {{{' '.join(names)}}}]"]
        rcf += [f"set_io -io_standard LVDS_25 {master} A5", f"set_io -io_standard LVDS_25 {names[1]} A4"]
    return "\n".join(xdc) + "\n", "\n".join(rcf) + "\n"


def main():
    vivado = os.path.expanduser(sys.argv[1])
    os.makedirs(OUT, exist_ok=True)
    table = []
    for group, names in GROUPS.items():
        for prim in names.split():
            ports = ports_of(vivado, prim)
            if ports is None:
                print(f"no ports for {prim}", file=sys.stderr)
                continue
            text, top = probe(prim, ports)
            xdc, rcf = constraints(prim, top)
            for suffix, body in ((".v", text), (".xdc", xdc), (".rcf", rcf)):
                with open(f"{OUT}/{prim.lower()}_probe{suffix}", "w") as f:
                    f.write(body)
            table.append((group, prim))
    # `index.txt` holds a status per primitive by hand, as a test's expectation.
    known = {}
    if os.path.exists(f"{OUT}/index.txt"):
        known = {l.split()[1]: l.split()[2] for l in open(f"{OUT}/index.txt") if l[0] != "#"}
    head = [l for l in open(f"{OUT}/index.txt") if l[0] == "#"] if known else []
    with open(f"{OUT}/index.txt", "w") as f:
        f.writelines(head)
        for group, prim in table:
            f.write(f"{group} {prim} {known.get(prim, 'gap')}\n")
    print(len(table), "probes")


main()
