#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/idelay/: an `IDELAYE2` beside each of
a bank's pads, one setting a pad, and designs asking where Vivado puts an
`IDELAYCTRL` and what it refuses.

    tools/vivado-parity/gen_idelay.py testdata/fpga/vivado/idelay

`index.txt` says which pad has which. The clock is `clk`, through a `BUFG`, at
200 MHz, which is what `REFCLK_FREQUENCY` says by default; the controls are the
ports `c0` to `c3`. Each delayed input is read by a register, so that nothing is
taken away.
"""
import os
import sys

OUT = sys.argv[1]
# Bank 35, as `gen_iologic.py` has them: one clock region, one `IDELAYCTRL`.
PADS = ("F4 F3 G5 G4 G2 G1 H2 H1 H5 H4 F2 E1 E3 E5 C4 D3 E2 D1 C1 B1 B2 A2 C3 C2 C7 C6 B4 A3 A5 A4 B6 "
        "B5 B7").split()
HELPERS = {"clk": "D4", "c0": "A7", "c1": "J3", "c2": "H3", "c3": "K1", "r": "J1"}

# The controls a variable delay is given unless a setting says otherwise.
VARIABLE = {"pin:C": "gclk", "pin:CE": "c0", "pin:INC": "c1"}
LOAD = dict(VARIABLE, **{"pin:LD": "c2", "pin:CNTVALUEIN": "{c3, c2, c1, c0, c3}"})
PIPE = dict(LOAD, **{"pin:LDPIPEEN": "c3", "pin:REGRST": "c0"})

SWEEP = [
    {},
    {"IDELAY_VALUE": "1"},
    {"IDELAY_VALUE": "2"},
    {"IDELAY_VALUE": "4"},
    {"IDELAY_VALUE": "8"},
    {"IDELAY_VALUE": "16"},
    {"IDELAY_VALUE": "31"},
    {"HIGH_PERFORMANCE_MODE": '"TRUE"'},
    {"SIGNAL_PATTERN": '"CLOCK"'},
    {"IS_IDATAIN_INVERTED": "1'b1"},
    {"CINVCTRL_SEL": '"TRUE"', "pin:CINVCTRL": "c1"},
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"'),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', IDELAY_VALUE="5"),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', IS_C_INVERTED="1'b1"),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', CINVCTRL_SEL='"TRUE"', **{"pin:CINVCTRL": "c2"}),
    dict(LOAD, IDELAY_TYPE='"VAR_LOAD"'),
    dict(LOAD, IDELAY_TYPE='"VAR_LOAD"', IDELAY_VALUE="9"),
    dict(PIPE, IDELAY_TYPE='"VAR_LOAD_PIPE"'),
    dict(PIPE, IDELAY_TYPE='"VAR_LOAD_PIPE"', PIPE_SEL='"TRUE"'),
    dict(LOAD, IDELAY_TYPE='"VAR_LOAD"', PIPE_SEL='"TRUE"'),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', **{"pin:CE": "1'b1"}),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', **{"pin:CE": "1'b0"}),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', **{"pin:INC": "1'b1"}),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', **{"pin:LD": "1'b1"}),
    dict(LOAD, IDELAY_TYPE='"VAR_LOAD"', **{"pin:CNTVALUEIN": "5'd7"}),
    dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', **{"read:CNTVALUEOUT": "1"}),
    {"read:CNTVALUEOUT": "1"},
    {"pin:C": "gclk", "pin:CE": "c0", "pin:INC": "c1", "pin:LD": "c2"},
    {"IDELAY_VALUE": "13", "HIGH_PERFORMANCE_MODE": '"TRUE"', "SIGNAL_PATTERN": '"CLOCK"'},
]
# The delayed input into an `IDDR` beside it, rather than to the fabric, and to
# both.
TO_IDDR = [{"to": "iddr"}, {"to": "iddr", "IDELAY_VALUE": "3"}, {"to": "both"}]


def tag(p):
    return ",".join(f"{k}={v}".replace('"', "").replace("1'b", "") for k, v in sorted(p.items())) or "default"


def instance(k, src, settings, out, pad=True):
    params = {k2: v for k2, v in settings.items() if ":" not in k2 and k2 not in ("to", "fabric", "also")}
    pins = {k2[4:]: v for k2, v in settings.items() if k2.startswith("pin:")}
    par = ",".join(f".{a}({b})" for a, b in params.items())
    par = f" #({par})" if par else ""
    conn = {"IDATAIN" if pad else "DATAIN": src, "DATAOUT": out}
    conn.update(pins)
    body = []
    if "read:CNTVALUEOUT" in settings:
        body.append(f"wire [4:0] cnt{k};")
        conn["CNTVALUEOUT"] = f"cnt{k}"
    conn_s = ", ".join(f".{a}({b})" for a, b in conn.items())
    body.append(f"IDELAYE2{par} u{k}({conn_s});")
    reads = [f"(^cnt{k})"] if "read:CNTVALUEOUT" in settings else []
    return body, reads


def design(name, settings_list, ctrl=True, period="5", extra_xdc=(), ctrl_text=None, pads=PADS,
           diff=False):
    ports, body, reads, xdc, index = [], [], [], [], []
    for k, setting in enumerate(settings_list):
        pad = pads[k]
        p = f"p{k}"
        to = setting.get("to", "fabric")
        if diff:
            ports += [f"input {p}", f"input {p}n"]
            body.append(f"wire b{k}; IBUFDS i{k}(.I({p}), .IB({p}n), .O(b{k}));")
            src = f"b{k}"
            xdc.append(f"set_property -dict {{PACKAGE_PIN {pad} IOSTANDARD LVDS_25}} [get_ports {p}]")
            xdc.append(f"set_property IOSTANDARD LVDS_25 [get_ports {p}n]")
        else:
            ports.append(f"input {p}")
            src = p
            xdc.append(f"set_property -dict {{PACKAGE_PIN {pad} IOSTANDARD LVCMOS33}} [get_ports {p}]")
        body.append(f"wire d{k};")
        b, r = instance(k, src, setting, f"d{k}", pad=not setting.get("fabric"))
        body += b
        reads += r
        if to == "none":
            pass
        if to in ("iddr", "both"):
            body.append(f"wire q1_{k}, q2_{k};")
            body.append(f"IDDR v{k}(.Q1(q1_{k}), .Q2(q2_{k}), .C(gclk), .CE(1'b1), .D(d{k}), .R(1'b0), .S(1'b0));")
            reads += [f"q1_{k}", f"q2_{k}"]
        if to in ("fabric", "both"):
            reads.append(f"d{k}")
        if setting.get("also") == "pad":
            reads.append(src)
        index.append(f"{name} {p} {pad} IDELAYE2 {tag(setting)}")
    if ctrl:
        body.append(ctrl_text or "wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));")
        if "wire rdy" in (ctrl_text or "wire rdy"):
            reads.append("rdy")
    expr = " ^ ".join(reads) or "1'b0"
    text = (f"module {name}(input clk, input c0, input c1, input c2, input c3, output reg r"
            f"{''.join(', ' + x for x in ports)});\n"
            "  wire gclk; BUFG bg(.I(clk), .O(gclk));\n"
            + "".join(f"  {b}\n" for b in body)
            + f"  always @(posedge gclk) r <= {expr};\nendmodule\n")
    open(f"{OUT}/{name}.v", "w").write(text)
    for port, ball in HELPERS.items():
        xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {port}]")
    xdc.append(f"create_clock -period {period} [get_ports clk]")
    xdc += list(extra_xdc)
    open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")
    return index


def raw(name, verilog, xdc):
    open(f"{OUT}/{name}.v", "w").write(verilog)
    open(f"{OUT}/{name}.xdc", "w").write(xdc)


HELPER_XDC = "".join(f"set_property -dict {{PACKAGE_PIN {b} IOSTANDARD LVCMOS33}} [get_ports {p}]\n"
                     for p, b in HELPERS.items()) + "create_clock -period 5 [get_ports clk]\n"


def fabric_source():
    """Delays whose input is the fabric's (`DELAY_SRC DATAIN`): placed by Vivado
    where it likes, so one a design, and compared without its place."""
    cases = {
        "datain_default": {"DELAY_SRC": '"DATAIN"'},
        "datain_value": {"DELAY_SRC": '"DATAIN"', "IDELAY_VALUE": "21"},
        "datain_inverted": {"DELAY_SRC": '"DATAIN"', "IS_DATAIN_INVERTED": "1'b1"},
        "datain_variable": dict(VARIABLE, DELAY_SRC='"DATAIN"', IDELAY_TYPE='"VARIABLE"'),
        "datain_idatain_inverted": {"DELAY_SRC": '"DATAIN"', "IS_IDATAIN_INVERTED": "1'b1"},
    }
    for name, setting in cases.items():
        params = {k: v for k, v in setting.items() if ":" not in k}
        pins = {k[4:]: v for k, v in setting.items() if k.startswith("pin:")}
        par = ",".join(f".{a}({b})" for a, b in params.items())
        conn = ", ".join(f".{a}({b})" for a, b in dict({"DATAIN": "s", "DATAOUT": "d"}, **pins).items())
        raw(name,
            f"module {name}(input clk, input c0, input c1, input c2, input c3, output reg r);\n"
            "  wire gclk; BUFG bg(.I(clk), .O(gclk));\n"
            "  reg s; always @(posedge gclk) s <= c0 ^ c2;\n"
            f"  wire d; IDELAYE2 #({par}) u0({conn});\n"
            "  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));\n"
            "  always @(posedge gclk) r <= d ^ rdy;\nendmodule\n",
            HELPER_XDC)


def more():
    """What the first sweep left open: a `_SING` tile's delay, an inverted `DATAIN`
    beside a pad's, a pad read both through its delay and around it, a delay
    whose output nothing reads, and a fabric delay beside a pad's."""
    sweep = [{}, {"IDELAY_VALUE": "7"}, {"IS_DATAIN_INVERTED": "1'b1"}, {"to": "none"}]
    index = design("idelay_more", sweep, pads=["E6", "K5", "F4", "G5"])
    # A pad read around its delay as well as through it: to the fabric both
    # ways, which is one `ILOGIC` output for two signals, and to the fabric and
    # an `IDDR`.
    index += design("pad_and_delay_fabric", [{"also": "pad"}])
    index += design("pad_and_delay_iddr", [{"also": "pad", "to": "iddr"}])
    # `IS_C_INVERTED`, which sets a bit no `segbits` file names, on each half of
    # a tile and of a `_SING` tile's.
    inverted = dict(VARIABLE, IDELAY_TYPE='"VARIABLE"', IS_C_INVERTED="1'b1")
    index += design("c_inverted", [inverted] * 4, pads=["F4", "F3", "E6", "K5"])
    # One delay from the fabric and one from a pad of bank 34: where Vivado puts
    # the first, and how many controllers.
    raw("datain_with_pad",
        "module datain_with_pad(input clk, input c0, input c1, input c2, input c3, output reg r, input p0);\n"
        "  wire gclk; BUFG bg(.I(clk), .O(gclk));\n"
        "  reg s; always @(posedge gclk) s <= c0 ^ c2;\n"
        "  wire d0, d1;\n"
        "  IDELAYE2 u0(.IDATAIN(p0), .DATAOUT(d0));\n"
        "  IDELAYE2 #(.DELAY_SRC(\"DATAIN\")) u1(.DATAIN(s), .DATAOUT(d1));\n"
        "  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));\n"
        "  always @(posedge gclk) r <= d0 ^ d1 ^ rdy;\nendmodule\n",
        HELPER_XDC + "set_property -dict {PACKAGE_PIN R1 IOSTANDARD LVCMOS33} [get_ports p0]\n")
    return index


def controllers():
    """Where an `IDELAYCTRL` goes, and how many."""
    # Delays in banks 35 and 34, two clock regions, and one controller.
    two = ["F4", "R1"]
    design("ctrl_two_banks", [{}, {}], pads=two)
    # The same with each in a group of its own, and a controller each.
    design("ctrl_groups", [{}, {}], pads=two, ctrl_text=(
        "wire rdy0, rdy1;\n"
        "  (* IODELAY_GROUP = \"g0\" *) IDELAYCTRL ctl0(.REFCLK(gclk), .RST(c3), .RDY(rdy0));\n"
        "  (* IODELAY_GROUP = \"g1\" *) IDELAYCTRL ctl1(.REFCLK(gclk), .RST(c3), .RDY(rdy1));\n"
        "  wire rdy = rdy0 ^ rdy1;"))
    text = open(f"{OUT}/ctrl_groups.v").read()
    text = text.replace("IDELAYE2 u0(", "(* IODELAY_GROUP = \"g0\" *) IDELAYE2 u0(").replace(
        "IDELAYE2 u1(", "(* IODELAY_GROUP = \"g1\" *) IDELAYE2 u1(")
    open(f"{OUT}/ctrl_groups.v", "w").write(text)
    # No controller at all.
    design("ctrl_none", [{}], ctrl=False)
    # A controller whose `RDY` nothing reads, and one whose `RST` is tied.
    design("ctrl_rdy_unread", [{}], ctrl_text="IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY());")
    design("ctrl_rst_tied", [{}], ctrl_text="wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(1'b0), .RDY(rdy));")
    # A controller and no delay: what its `RDY` becomes.
    raw("ctrl_alone",
        "module ctrl_alone(input clk, input c0, input c1, input c2, input c3, output reg r);\n"
        "  wire gclk; BUFG bg(.I(clk), .O(gclk));\n"
        "  wire rdy; IDELAYCTRL ctl(.REFCLK(gclk), .RST(c3), .RDY(rdy));\n"
        "  always @(posedge gclk) r <= rdy ^ c0;\nendmodule\n", HELPER_XDC)
    # A reference clock off 200 MHz, and `REFCLK_FREQUENCY` off it: what grade 1
    # takes.
    for freq, period in (("190.0", "5.263"), ("210.0", "4.762"), ("300.0", "3.333"), ("200.0", "4")):
        tagged = freq.split(".")[0] + ("_at_250" if period == "4" else "")
        design(f"refclk_{tagged}", [{"REFCLK_FREQUENCY": freq}], period=period)
    # Two delays asking for different reference frequencies of one controller.
    design("refclk_mixed", [{"REFCLK_FREQUENCY": "190.0"}, {"REFCLK_FREQUENCY": "210.0"}])
    # A `DATAIN` delay with an `IDATAIN` source: refused, or not.
    design("src_mismatch", [{"DELAY_SRC": '"DATAIN"'}])
    # A fabric signal on `DATAIN` with the default `DELAY_SRC`: `IDATAIN` is then
    # unconnected.
    design("src_default_fabric", [{"fabric": "1"}])
    # Two controllers and no group, and a delay in a group its controller is not.
    design("ctrl_two_same", [{}], ctrl_text=(
        "wire rdy0, rdy1;\n"
        "  IDELAYCTRL ctl0(.REFCLK(gclk), .RST(c3), .RDY(rdy0));\n"
        "  IDELAYCTRL ctl1(.REFCLK(gclk), .RST(c2), .RDY(rdy1));\n"
        "  wire rdy = rdy0 ^ rdy1;"))
    design("group_mismatch", [{}])
    text = open(f"{OUT}/group_mismatch.v").read()
    open(f"{OUT}/group_mismatch.v", "w").write(
        text.replace("IDELAYE2 u0(", "(* IODELAY_GROUP = \"g0\" *) IDELAYE2 u0("))
    # The value out of range in a fixed delay.
    design("value_32", [{"IDELAY_VALUE": "32"}])


def dci_reset():
    """`DCIRESET`, which this part has although its banks have no DCI: its
    reset from the fabric, tied low and left open, and its `LOCKED` read and
    not."""
    for name, rst, read in (("dcireset_routed", "s", True), ("dcireset_rst_tied", "1'b0", True),
                            ("dcireset_rst_open", None, True), ("dcireset_unread", "s", False)):
        conn = [f".RST({rst})"] if rst else []
        conn.append(".LOCKED(locked)" if read else ".LOCKED()")
        raw(name,
            f"module {name}(input clk, input c0, input c1, input c2, input c3, output reg r);\n"
            "  wire gclk; BUFG bg(.I(clk), .O(gclk));\n"
            "  reg s; always @(posedge gclk) s <= c0 ^ c2;\n"
            f"  wire locked; DCIRESET u0({', '.join(conn)});\n"
            f"  always @(posedge gclk) r <= {'locked ^ ' if read else ''}c1;\nendmodule\n",
            HELPER_XDC)


def main():
    os.makedirs(OUT, exist_ok=True)
    index = []
    index += design("idelay_sweep", SWEEP)
    index += design("idelay_iddr", TO_IDDR)
    index += design("idelay_lvds", [{}, {"IDELAY_VALUE": "6"}], pads=["G2", "H2"], diff=True)
    index += more()
    fabric_source()
    controllers()
    dci_reset()
    open(f"{OUT}/index.txt", "w").write("# design port ball primitive setting\n" + "\n".join(index) + "\n")


main()
