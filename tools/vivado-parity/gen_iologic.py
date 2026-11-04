#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/iologic/: an IO register beside each
of a bank's pads, one setting a pad, to ask Vivado what each setting sets.

    tools/vivado-parity/gen_iologic.py testdata/fpga/vivado/iologic

`index.txt` says which pad has which. The clock is `clk` on `N11`, through a
`BUFG`; the controls are the ports `c0` to `c3`, from pads of bank 14.
"""
import os
import sys

OUT = sys.argv[1]
# All in bank 35, so that a build loads a part of the die and not all of it.
PADS = ("F4 F3 G5 G4 G2 G1 H2 H1 H5 H4 F2 E1 E3 E5 C4 D3 E2 D1 C1 B1 B2 A2 C3 C2 C7 C6 B4 A3 A5 A4 B6 "
        "B5 B7").split()
HELPERS = {"clk": "D4", "c0": "A7", "c1": "J3", "c2": "H3", "c3": "K1", "r": "J1"}

IDDR = [
    {},
    {"DDR_CLK_EDGE": '"SAME_EDGE"'},
    {"DDR_CLK_EDGE": '"SAME_EDGE_PIPELINED"'},
    {"SRTYPE": '"ASYNC"'},
    {"INIT_Q1": "1'b1"},
    {"INIT_Q2": "1'b1"},
    {"IS_C_INVERTED": "1'b1"},
    {"IS_D_INVERTED": "1'b1"},
    {"pin:R": "c1"},
    {"pin:S": "c1"},
    {"pin:R": "c1", "SRTYPE": '"ASYNC"'},
    {"pin:S": "c1", "SRTYPE": '"ASYNC"'},
    {"pin:R": "c1", "INIT_Q1": "1'b1", "INIT_Q2": "1'b1"},
    {"pin:S": "c1", "INIT_Q1": "1'b1"},
    {"pin:R": "1'b1"},
    {"pin:S": "1'b1"},
    {"pin:CE": "1'b1"},
    {"pin:CE": "1'b0"},
    {"pin:CE": "c0"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"', "INIT_Q1": "1'b1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"', "INIT_Q2": "1'b1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"', "pin:R": "c1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE_PIPELINED"', "INIT_Q1": "1'b1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE_PIPELINED"', "INIT_Q2": "1'b1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE_PIPELINED"', "pin:R": "c1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"', "IS_C_INVERTED": "1'b1"},
]
# Both a set and a reset: refused by Vivado (`REQP-77`, `REQP-129`), each alone.
BOTH = [{"pin:R": "c1", "pin:S": "c2"}]
# One clock on `C` and `CB` with the same inversion: `REQP-76`.
SAME_CLOCK = [{"cb": "clk"}]
IDDR_2CLK = [
    {},
    {"cb": "clk_n"},
    {"cb": "clk2"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"'},
    {"DDR_CLK_EDGE": '"SAME_EDGE_PIPELINED"'},
    {"IS_CB_INVERTED": "1'b1", "cb": "clk"},
    {"IS_C_INVERTED": "1'b1", "cb": "clk"},
    {"pin:R": "c1"},
    {"pin:S": "c1"},
]
ODDR = [
    {},
    {"DDR_CLK_EDGE": '"SAME_EDGE"'},
    {"SRTYPE": '"ASYNC"'},
    {"INIT": "1'b1"},
    {"IS_C_INVERTED": "1'b1"},
    {"IS_D1_INVERTED": "1'b1"},
    {"IS_D2_INVERTED": "1'b1"},
    {"pin:R": "c1"},
    {"pin:S": "c1"},
    {"pin:R": "c1", "SRTYPE": '"ASYNC"'},
    {"pin:S": "c1", "SRTYPE": '"ASYNC"'},
    {"pin:S": "c1", "INIT": "1'b1"},
    {"pin:R": "1'b1"},
    {"pin:S": "1'b1"},
    {"pin:CE": "1'b0"},
    {"pin:CE": "c0"},
    {"pin:D1": "1'b0", "pin:D2": "1'b1"},
    {"pin:D1": "1'b1", "pin:D2": "1'b0"},
    {"pin:D1": "1'b0", "pin:D2": "1'b0"},
    {"pin:D1": "1'b1", "pin:D2": "1'b1"},
    {"DDR_CLK_EDGE": '"SAME_EDGE"', "IS_C_INVERTED": "1'b1"},
    {"IS_C_INVERTED": "1'b1", "pin:R": "c1"},
    {"IS_C_INVERTED": "1'b1", "INIT": "1'b1"},
]


def tag(p):
    return ",".join(f"{k}={v}".replace('"', "").replace("1'b", "") for k, v in sorted(p.items())) or "default"


def design(name, primitive, settings):
    ports, body, reads, xdc, index = [], [], [], [], []
    clocks = "clk"
    if any("cb" in s and s["cb"] == "clk2" for s in settings):
        clocks += ", input clk2"
    for k, setting in enumerate(settings):
        pad = PADS[k]
        params = {k2: v for k2, v in setting.items() if not k2.startswith("pin:") and k2 != "cb"}
        pins = {k2[4:]: v for k2, v in setting.items() if k2.startswith("pin:")}
        par = ",".join(f".{k2}({v})" for k2, v in params.items())
        par = f" #({par})" if par else ""
        p = f"p{k}"
        if primitive in ("IDDR", "IDDR_2CLK"):
            ports.append(f"input {p}")
            conn = {"C": "gclk", "CE": "c0", "D": p, "R": "1'b0", "S": "1'b0"}
            if primitive == "IDDR_2CLK":
                cb = setting.get("cb", "gclk_n")
                conn["CB"] = {"clk_n": "~gclk", "clk2": "gclk2", "gclk_n": "~gclk", "clk": "gclk"}[cb]
            conn.update(pins)
            body.append(f"wire q1_{k}, q2_{k};")
            conn_s = ", ".join(f".{a}({b})" for a, b in conn.items())
            body.append(f"{primitive}{par} u{k}(.Q1(q1_{k}), .Q2(q2_{k}), {conn_s});")
            reads += [f"q1_{k}", f"q2_{k}"]
        else:
            ports.append(f"output {p}")
            conn = {"C": "gclk", "CE": "c0", "D1": "c2", "D2": "c3", "R": "1'b0", "S": "1'b0"}
            conn.update(pins)
            conn_s = ", ".join(f".{a}({b})" for a, b in conn.items())
            body.append(f"{primitive}{par} u{k}(.Q({p}), {conn_s});")
        xdc.append(f"set_property -dict {{PACKAGE_PIN {pad} IOSTANDARD LVCMOS33}} [get_ports {p}]")
        index.append(f"{name} {p} {pad} {primitive} {tag(setting)}")
    clocking = ["wire gclk; BUFG bg(.I(clk), .O(gclk));"]
    extra = ""
    if "clk2" in clocks:
        clocking.append("wire gclk2; BUFG bg2(.I(clk2), .O(gclk2));")
        extra = ", input clk2"
        xdc.append("set_property -dict {PACKAGE_PIN F5 IOSTANDARD LVCMOS33} [get_ports clk2]")
    expr = " ^ ".join(reads) or "1'b0"
    text = (f"module {name}(input clk{extra}, input c0, input c1, input c2, input c3, output reg r"
            f"{''.join(', ' + x for x in ports)});\n"
            + "".join(f"  {b}\n" for b in clocking + body)
            + f"  always @(posedge gclk) r <= {expr};\nendmodule\n")
    open(f"{OUT}/{name}.v", "w").write(text)
    for port, ball in HELPERS.items():
        xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {port}]")
    xdc.append("create_clock -period 10 [get_ports clk]")
    open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")
    return index


def main():
    os.makedirs(OUT, exist_ok=True)
    index = []
    index += design("iddr_sweep", "IDDR", IDDR)
    index += design("iddr_2clk_sweep", "IDDR_2CLK", IDDR_2CLK)
    index += design("oddr_sweep", "ODDR", ODDR)
    index += design("iddr_both", "IDDR", BOTH)
    index += design("oddr_both", "ODDR", BOTH)
    index += design("iddr_2clk_same", "IDDR_2CLK", SAME_CLOCK)
    open(f"{OUT}/index.txt", "w").write("# design port ball primitive setting\n" + "\n".join(index) + "\n")


main()
