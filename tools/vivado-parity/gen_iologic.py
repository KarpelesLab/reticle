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


# The serialisers. Each setting is one `ISERDESE2` or `OSERDESE2` on a pad,
# with these pins unless the setting names one (`pin:X`, `open` for none).
# `CLK` is `gclk`, through a `BUFG` from `clk`, and `CLKDIV` `gdiv`, through
# another from `clkd`; data and tristate inputs come from `sh`, a shift
# register clocked by `gdiv`.
ISERDES_PINS = {"D": "pad", "CLK": "gclk", "CLKB": "~gclk", "CLKDIV": "gdiv", "CLKDIVP": "1'b0",
                "OCLK": "gclk", "OCLKB": "~gclk", "CE1": "c0", "CE2": "c1", "RST": "c2", "BITSLIP": "c3",
                "DYNCLKSEL": "1'b0", "DYNCLKDIVSEL": "1'b0"}
OSERDES_PINS = {"CLK": "gclk", "CLKDIV": "gdiv", "OCE": "c0", "RST": "c1",
                **{f"D{k}": f"sh[{k - 1}]" for k in range(1, 9)}}
ISERDES_PORTS = set(ISERDES_PINS) | {"OFB", "DDLY", "SHIFTIN1", "SHIFTIN2"}
OSERDES_PORTS = set(OSERDES_PINS) | {"T1", "T2", "T3", "T4", "TCE", "TBYTEIN", "SHIFTIN1", "SHIFTIN2"}
NETWORKING = {"INTERFACE_TYPE": '"NETWORKING"', "pin:OCLK": "open", "pin:OCLKB": "open"}


def net(width, rate="DDR"):
    return {**NETWORKING, "DATA_RATE": f'"{rate}"', "DATA_WIDTH": str(width)}


ISERDES = [
    {},
    *[net(w, "SDR") for w in range(2, 9)],
    *[net(w) for w in (4, 6, 8)],
    {"INTERFACE_TYPE": '"MEMORY_DDR3"'},
    {"INTERFACE_TYPE": '"MEMORY_QDR"'},
    {"INTERFACE_TYPE": '"OVERSAMPLE"', **{k: v for k, v in NETWORKING.items() if k != "INTERFACE_TYPE"}},
    {"NUM_CE": "1"},
    *[{f"INIT_Q{k}": "1'b1"} for k in range(1, 5)],
    *[{f"SRVAL_Q{k}": "1'b1"} for k in range(1, 5)],
    *[{f"IS_{p}_INVERTED": "1'b1"} for p in ("CLK", "CLKDIV", "CLKDIVP", "D", "OCLK", "OCLKB")],
    {"DYN_CLK_INV_EN": '"TRUE"', "pin:DYNCLKSEL": "c3"},
    {"DYN_CLKDIV_INV_EN": '"TRUE"', "pin:DYNCLKDIVSEL": "c3"},
]
ISERDES_PINS_SWEEP = [
    {"pin:RST": "1'b0"},
    {"pin:RST": "1'b1"},
    {"pin:CE1": "1'b1"},
    {"pin:CE1": "1'b0"},
    {"pin:CE2": "1'b1"},
    {"pin:CE2": "1'b0"},
    {"NUM_CE": "1", "pin:CE2": "open"},
    {"pin:BITSLIP": "1'b0"},
    {"pin:BITSLIP": "1'b1"},
    {"pin:CLKB": "gclk2"},
    {"pin:OCLKB": "gclk"},
    {"pin:CLKDIV": "gclk"},
    {"pin:DYNCLKSEL": "c3"},
    {"pin:DYNCLKDIVSEL": "c3"},
    {"read": "Q1"},
    {"read": "O"},
    {"read": "O,Q1,Q2,Q3,Q4"},
    {**net(8), "read": "Q1"},
    {**net(4), "pin:BITSLIP": "1'b0", "pin:CE1": "1'b1", "pin:CE2": "1'b1", "pin:RST": "1'b0"},
    {**net(4, "SDR"), "NUM_CE": "1", "pin:CE2": "open"},
    {"INIT_Q1": "1'b1", "INIT_Q2": "1'b1", "INIT_Q3": "1'b1", "INIT_Q4": "1'b1",
     "SRVAL_Q1": "1'b1", "SRVAL_Q2": "1'b1", "SRVAL_Q3": "1'b1", "SRVAL_Q4": "1'b1"},
    {**net(8), "IS_CLK_INVERTED": "1'b1"},
    {**net(8), "IS_CLKDIV_INVERTED": "1'b1"},
    {**net(8), "IS_D_INVERTED": "1'b1"},
    {**net(6), "INIT_Q1": "1'b1", "SRVAL_Q2": "1'b1"},
    # On an `_N` ball: the bit is one the database does not name.
    {"IS_CLKDIV_INVERTED": "1'b1"},
    *[{f"pin:{p}": "open"} for p in ("RST", "CE1", "CE2", "BITSLIP", "DYNCLKSEL", "DYNCLKDIVSEL")],
    {**net(4), "pin:OCLK": "gclk"},
]
# What `CLK`, `CLKB`, `OCLK` and `OCLKB` and their inversions set, in the
# combinations Vivado builds: `CLKB` of `~CLK` (`inv`), of `CLK` (`same`),
# unconnected (`open`) or of a second clock (`other`), then `IS_CLK_INVERTED`
# and `IS_CLKB_INVERTED`.
ISERDES_MODES = {
    "mem": {},
    "netddr": net(4),
    "netsdr": net(4, "SDR"),
    "over": {"INTERFACE_TYPE": '"OVERSAMPLE"', "pin:OCLK": "open", "pin:OCLKB": "open"},
    "qdr": {"INTERFACE_TYPE": '"MEMORY_QDR"'},
}
CLKB = {"inv": "~gclk", "same": "gclk", "open": "open", "other": "gclk2"}


def clocking(mode, clkb, inverted, b_inverted):
    s = {**ISERDES_MODES[mode], "pin:CLKB": CLKB[clkb]}
    if inverted:
        s["IS_CLK_INVERTED"] = "1'b1"
    if b_inverted:
        s["IS_CLKB_INVERTED"] = "1'b1"
    return s


def oclocking(oclkb, inverted, b_inverted):
    s = {"pin:OCLKB": {"inv": "~gclk", "same": "gclk", "open": "open"}[oclkb]}
    if inverted:
        s["IS_OCLK_INVERTED"] = "1'b1"
    if b_inverted:
        s["IS_OCLKB_INVERTED"] = "1'b1"
    return s


ISERDES_CLOCKS = [
    *[clocking("netsdr", *c) for c in (("inv", 0, 0), ("inv", 1, 1), ("open", 0, 0), ("open", 1, 0),
                                        ("other", 0, 0), ("other", 0, 1), ("other", 1, 0), ("other", 1, 1),
                                        ("same", 0, 1), ("same", 1, 0))],
    *[clocking("mem", *c) for c in (("inv", 1, 0), ("inv", 1, 1), ("same", 0, 1), ("same", 1, 0),
                                     ("same", 1, 1), ("other", 1, 1))],
    *[clocking("netddr", *c) for c in (("inv", 1, 0), ("same", 1, 1), ("other", 1, 1))],
    clocking("over", "inv", 1, 1),
    clocking("qdr", "same", 1, 1),
    clocking("qdr", "inv", 1, 0),
    *[oclocking(*c) for c in (("inv", 0, 1), ("open", 0, 0), ("open", 1, 0), ("same", 0, 0), ("same", 1, 1))],
]
# `CLKDIVP` on `CLKDIV`'s clock, on a `_P` ball and an `_N` one, and inverted.
CLKDIVP = [
    {"pin:CLKDIVP": "gdiv"},
    {"pin:CLKDIVP": "gdiv"},
    {"pin:CLKDIVP": "gdiv", "IS_CLKDIVP_INVERTED": "1'b1"},
]
# Wider than one block: a master on the pair's `_P` ball and a slave beside it.
ISERDES_WIDE = [
    {**net(10), "slave": True},
    {**net(14), "slave": True},
    {**net(10), "slave": True, "INIT_Q1": "1'b1", "SRVAL_Q3": "1'b1"},
    {**net(14), "slave": True, "IS_D_INVERTED": "1'b1"},
    {**net(10), "slave": True, "IS_CLK_INVERTED": "1'b1"},
]
# A width above four needs `TRISTATE_WIDTH` 1 (`AVAL-69`), and so does a
# `DATA_RATE_TQ` of `SDR` or `BUF` (`AVAL-74`).
OSERDES = [
    {},
    *[{"DATA_RATE_OQ": '"SDR"', "DATA_RATE_TQ": '"SDR"', "DATA_WIDTH": str(w), "TRISTATE_WIDTH": "1"}
      for w in range(2, 9)],
    {"DATA_RATE_OQ": '"SDR"', "DATA_WIDTH": "4"},
    *[{"DATA_WIDTH": str(w), "TRISTATE_WIDTH": "1"} for w in (4, 6, 8)],
    {"DATA_RATE_TQ": '"BUF"', "TRISTATE_WIDTH": "1"},
    {"DATA_RATE_TQ": '"SDR"', "TRISTATE_WIDTH": "1"},
    {"TRISTATE_WIDTH": "1"},
    {"INIT_OQ": "1'b1"},
    {"INIT_TQ": "1'b1"},
    {"SRVAL_OQ": "1'b1"},
    {"SRVAL_TQ": "1'b1"},
    {"IS_CLK_INVERTED": "1'b1"},
    {"IS_CLKDIV_INVERTED": "1'b1"},
    *[{f"IS_D{k}_INVERTED": "1'b1"} for k in range(1, 9)],
    *[{f"IS_T{k}_INVERTED": "1'b1"} for k in range(2, 5)],
    {"TBYTE_CTL": '"TRUE"'},
]
OSERDES_PINS_SWEEP = [
    {"pin:D1": "1'b0"},
    {"pin:D1": "1'b1"},
    {"pin:D8": "1'b0"},
    {"pin:D4": "1'b1"},
    {"pin:OCE": "1'b1"},
    {"pin:OCE": "1'b0"},
    {"pin:RST": "1'b0"},
    {"pin:RST": "1'b1"},
    {"pin:T1": "1'b0"},
    {"pin:TCE": "1'b1"},
    {"pin:TCE": "c0"},
    {"pin:CLKDIV": "gclk"},
    {"DATA_RATE_OQ": '"SDR"', "DATA_RATE_TQ": '"BUF"', "DATA_WIDTH": "4", "TRISTATE_WIDTH": "1"},
    {"DATA_RATE_OQ": '"SDR"', "DATA_RATE_TQ": '"SDR"', "DATA_WIDTH": "8", "TRISTATE_WIDTH": "1",
     "IS_CLK_INVERTED": "1'b1"},
    {"DATA_WIDTH": "8", "TRISTATE_WIDTH": "1", "IS_D3_INVERTED": "1'b1", "pin:D5": "1'b0"},
    {"INIT_OQ": "1'b1", "SRVAL_OQ": "1'b1", "INIT_TQ": "1'b1", "SRVAL_TQ": "1'b1"},
    {"pin:OCE": "open"},
    # On an `_N` ball, as the `ISERDESE2`'s.
    {"IS_CLKDIV_INVERTED": "1'b1"},
    {"pin:RST": "open"},
    {"pin:D1": "open"},
]
# A tristate: `TQ` to an `OBUFT`'s `T`, and `T1` to `T4` from `sh`.
OSERDES_TRISTATE = [
    {"tq": True},
    {"tq": True, "DATA_RATE_TQ": '"BUF"', "TRISTATE_WIDTH": "1"},
    {"tq": True, "DATA_RATE_TQ": '"SDR"', "TRISTATE_WIDTH": "1"},
    {"tq": True, "DATA_RATE_TQ": '"DDR"', "TRISTATE_WIDTH": "1"},
    {"tq": True, "DATA_RATE_OQ": '"SDR"', "DATA_RATE_TQ": '"SDR"', "DATA_WIDTH": "4", "TRISTATE_WIDTH": "1"},
    {"tq": True, "pin:TCE": "1'b1"},
    {"tq": True, "pin:TCE": "1'b0"},
    {"tq": True, "IS_T1_INVERTED": "1'b1"},
    {"tq": True, "IS_T2_INVERTED": "1'b1", "IS_T4_INVERTED": "1'b1"},
    {"tq": True, "INIT_TQ": "1'b1"},
    {"tq": True, "SRVAL_TQ": "1'b1"},
    {"tq": True, "DATA_RATE_TQ": '"BUF"', "TRISTATE_WIDTH": "1", "pin:T1": "1'b0"},
    {"tq": True, "DATA_RATE_TQ": '"BUF"', "TRISTATE_WIDTH": "1", "IS_T1_INVERTED": "1'b1"},
    {"tq": True, "TBYTE_CTL": '"TRUE"'},
    {"tq": True, "pin:TCE": "open"},
]
OSERDES_WIDE = [
    {"DATA_WIDTH": "10", "TRISTATE_WIDTH": "1", "slave": True},
    {"DATA_WIDTH": "14", "TRISTATE_WIDTH": "1", "slave": True},
    {"DATA_WIDTH": "10", "TRISTATE_WIDTH": "1", "slave": True, "IS_D9_INVERTED": "1'b1"},
    {"DATA_WIDTH": "14", "TRISTATE_WIDTH": "1", "slave": True, "INIT_OQ": "1'b1", "SRVAL_OQ": "1'b1"},
    {"DATA_WIDTH": "10", "TRISTATE_WIDTH": "1", "DATA_RATE_TQ": '"BUF"', "slave": True, "tq": True},
]
# An `OSERDESE2` read back by the `ISERDESE2` beside it, through `OFB`.
FEEDBACK = [
    {"ofb": True},
    {"ofb": True, "DATA_WIDTH": "8", "TRISTATE_WIDTH": "1", "INTERFACE_TYPE": '"NETWORKING"',
     "pin:OCLK": "open", "pin:OCLKB": "open"},
]
# Each alone in a design, because Vivado refuses it, by the rule its comment
# names, or because it might have: the last group it builds.
ALONE = {
    # `AVAL-69`: a width above four with `TRISTATE_WIDTH` 4.
    "oserdes_wide_tri4": ("OSERDESE2", {"DATA_WIDTH": "8"}),
    # `AVAL-74`: `DATA_RATE_TQ` of `SDR` with `TRISTATE_WIDTH` 4.
    "oserdes_tq_sdr_tri4": ("OSERDESE2", {"DATA_RATE_TQ": '"SDR"'}),
    # `PDRC-26`: `CLKB` the same clock as `CLK`, as an inversion turns it.
    "iserdes_clkb_same": ("ISERDESE2", {"IS_CLKB_INVERTED": "1'b1"}),
    # `PDRC-37`: `T1` high with no `TQ`, which holds the pad released.
    "oserdes_t1_inverted": ("OSERDESE2", {"IS_T1_INVERTED": "1'b1"}),
    "oserdes_t1_high": ("OSERDESE2", {"pin:T1": "1'b1"}),
    # `THRU-2`: `OFB` or `TFB` read by the fabric.
    "oserdes_ofb_fabric": ("OSERDESE2", {"read": "OFB"}),
    "oserdes_tfb_fabric": ("OSERDESE2", {"read": "TFB"}),
    # `PDRC-26` again: one clock on `CLK` and `CLKB` with no inversion, and in
    # `SDR` an inversion on both or on `CLK` alone of `~CLK`.
    "iserdes_clkb_same_mem": ("ISERDESE2", clocking("mem", "same", 0, 0)),
    "iserdes_sdr_clk_inverted": ("ISERDESE2", clocking("netsdr", "inv", 1, 0)),
    "iserdes_sdr_both_inverted": ("ISERDESE2", clocking("netsdr", "same", 1, 1)),
    # `INBB-3`: no `CLKB` at a double data rate, which synthesis leaves a
    # black box.
    "iserdes_clkb_open": ("ISERDESE2", clocking("mem", "open", 0, 0)),
    # `Route 35-7`: `CLKDIVP` on another clock than `CLKDIV`, which reaches the
    # pin on the same wire.
    "iserdes_clkdivp_clk": ("ISERDESE2", {"pin:CLKDIVP": "gclk"}),
    # Built: `IOBDELAY` with nothing on `DDLY`, `CLKDIVP` on `CLKDIV`'s clock,
    # `MEMORY` with no `OCLK`, and `TFB` read through a second flip-flop.
    "iserdes_iobdelay": ("ISERDESE2", {"IOBDELAY": '"BOTH"'}),
    "iserdes_iobdelay_ibuf": ("ISERDESE2", {"IOBDELAY": '"IBUF"'}),
    "iserdes_iobdelay_ifd": ("ISERDESE2", {"IOBDELAY": '"IFD"'}),
    "iserdes_clkdivp_ddr3": ("ISERDESE2", {"INTERFACE_TYPE": '"MEMORY_DDR3"', "pin:CLKDIVP": "gdiv"}),
    "iserdes_memory_no_oclk": ("ISERDESE2", {"pin:OCLK": "open", "pin:OCLKB": "open"}),
    "oserdes_tfb_pipelined": ("OSERDESE2", {"read": "TFB", "pipe": True}),
}
# `_P` balls of bank 35 whose `_N` is free, and that `_N`, for the wide ones.
PAIRS = [("F4", "F3"), ("G5", "G4"), ("G2", "G1"), ("H2", "H1"), ("H5", "H4"), ("F2", "E1"),
         ("E3", "D3"), ("E2", "D1"), ("C1", "B1"), ("B2", "A2"), ("C3", "C2"), ("C7", "C6")]


def serdes_design(name, primitive, settings, pads=None):
    """One design, a serialiser a pad; `pads` is the ball of each, and of each
    slave, from `PAIRS` when a setting has one."""
    ports, body, reads, xdc, index = [], [], [], [], []
    second = any(v == "gclk2" for s in settings for v in s.values())
    for k, setting in enumerate(settings):
        ball, slave_ball = (PAIRS[k] if pads == "pairs" else (PADS[k], None))
        params = {a: v for a, v in setting.items()
                  if not a.startswith("pin:") and a not in ("read", "slave", "tq", "ofb", "pipe")}
        p = f"p{k}"
        width = int(params.get("DATA_WIDTH", "4"))
        if primitive == "ISERDESE2" or setting.get("ofb"):
            conn = dict(ISERDES_PINS)
            ipar = params
            if setting.get("ofb"):
                ipar = {a: v for a, v in params.items() if a in ("DATA_WIDTH", "INTERFACE_TYPE")}
                ipar["OFB_USED"] = '"TRUE"'
                conn["D"] = "open"
                conn["OFB"] = f"ofb_{k}"
            conn["D"] = p if conn["D"] == "pad" else conn["D"]
            conn.update({a[4:]: v for a, v in setting.items() if a.startswith("pin:") and a[4:] in ISERDES_PORTS})
            if not setting.get("ofb"):
                ports.append(f"input {p}")
            names = setting.get("read", "Q1,Q2,Q3,Q4,Q5,Q6,Q7,Q8").split(",")
            outs = {q: f"{q.lower()}_{k}" for q in names}
            if setting.get("slave"):
                outs.update({"SHIFTOUT1": f"so1_{k}", "SHIFTOUT2": f"so2_{k}"})
            body.append(f"wire {', '.join(outs.values())};")
            reads += [v for q, v in outs.items() if not q.startswith("SHIFT")]
            body.append(serdes("ISERDESE2", ipar, {**outs, **conn}, f"i{k}" if setting.get("ofb") else f"u{k}"))
            index.append(f"{name} {p} {ball} ISERDESE2 {tag(setting)}")
            if setting.get("slave"):
                sconn = {a: v for a, v in conn.items() if a != "D"}
                sconn.update({"SHIFTIN1": f"so1_{k}", "SHIFTIN2": f"so2_{k}"})
                souts = {f"Q{j}": f"s{j}_{k}" for j in range(3, 9)}
                body.append(f"wire {', '.join(souts.values())};")
                reads += list(souts.values())
                spar = {**params, "SERDES_MODE": '"SLAVE"'}
                body.append(serdes("ISERDESE2", spar, {**souts, **sconn}, f"s{k}"))
                index.append(f"{name} {p}s {slave_ball} ISERDESE2 slave")
        if primitive == "OSERDESE2":
            conn = dict(OSERDES_PINS)
            conn.update({a[4:]: v for a, v in setting.items() if a.startswith("pin:") and a[4:] in OSERDES_PORTS})
            if setting.get("tq"):
                outs = {"OQ": f"oq_{k}", "TQ": f"tq_{k}"}
                conn.update({f"T{j}": f"sh[{7 + j}]" for j in range(1, 5) if f"pin:T{j}" not in setting})
                conn.setdefault("TCE", "c0")
                body.append(f"wire oq_{k}, tq_{k};")
                body.append(f"OBUFT t{k}(.I(oq_{k}), .T(tq_{k}), .O({p}));")
            else:
                outs = {"OQ": p}
            if setting.get("ofb"):
                outs["OFB"] = f"ofb_{k}"
                body.insert(0, f"wire ofb_{k};")
            for fb in ("OFB", "TFB"):
                if fb in setting.get("read", "").split(","):
                    outs[fb] = f"{fb.lower()}_{k}"
                    body.append(f"wire {fb.lower()}_{k};")
                    reads.append(f"{fb.lower()}_{k}")
            ports.append(f"output {p}")
            mpar = {a: v for a, v in params.items() if not (setting.get("ofb") and a in ("INTERFACE_TYPE",))}
            if width > 8:
                outs_s = {"SHIFTOUT1": f"so1_{k}", "SHIFTOUT2": f"so2_{k}"}
                body.append(f"wire so1_{k}, so2_{k};")
                sconn = {a: conn[a] for a in ("CLK", "CLKDIV", "OCE", "RST")}
                for j in range(3, width - 8 + 3):
                    sconn[f"D{j}"] = f"sh[{j + 5}]"
                spar = {a: v for a, v in mpar.items() if not a.startswith("IS_D") and not a.startswith("INIT_")
                        and not a.startswith("SRVAL_")}
                if "IS_D9_INVERTED" in mpar:
                    spar["IS_D3_INVERTED"] = "1'b1"
                    del mpar["IS_D9_INVERTED"]
                spar["SERDES_MODE"] = '"SLAVE"'
                body.append(serdes("OSERDESE2", spar, {**outs_s, **sconn}, f"s{k}"))
                conn.update({"SHIFTIN1": f"so1_{k}", "SHIFTIN2": f"so2_{k}"})
                index.append(f"{name} {p}s {slave_ball} OSERDESE2 slave")
            body.append(serdes("OSERDESE2", mpar, {**outs, **conn}, f"u{k}"))
            index.append(f"{name} {p} {ball} OSERDESE2 {tag(setting)}")
        xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {p}]")
    head = ["wire gclk; BUFG bg(.I(clk), .O(gclk));", "wire gdiv; BUFG bd(.I(clkd), .O(gdiv));"]
    extra = ", input clkd"
    xdc.append("set_property -dict {PACKAGE_PIN F5 IOSTANDARD LVCMOS33} [get_ports clkd]")
    if second:
        head.append("wire gclk2; BUFG bg2(.I(clk2), .O(gclk2));")
        extra += ", input clk2"
        xdc.append("set_property -dict {PACKAGE_PIN D13 IOSTANDARD LVCMOS33} [get_ports clk2]")
    head.append("reg [15:0] sh; always @(posedge gdiv) sh <= {sh[14:0], c3};")
    expr = " ^ ".join(reads) or "sh[15]"
    text = (f"module {name}(input clk{extra}, input c0, input c1, input c2, input c3, output reg r"
            f"{''.join(', ' + x for x in ports)});\n"
            + "".join(f"  {b}\n" for b in head + body)
            + (f"  reg r0; always @(posedge gdiv) r0 <= {expr};\n  always @(posedge gdiv) r <= r0;\n"
               if any(s.get("pipe") for s in settings) else f"  always @(posedge gdiv) r <= {expr};\n")
            + "endmodule\n")
    open(f"{OUT}/{name}.v", "w").write(text)
    for port, ball in HELPERS.items():
        xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {port}]")
    xdc.append("create_clock -period 10 [get_ports clk]")
    xdc.append("create_clock -period 40 [get_ports clkd]")
    open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")
    return index


def serdes(primitive, params, conn, name):
    par = ", ".join(f".{a}({v})" for a, v in params.items())
    par = f" #({par})" if par else ""
    pins = ", ".join(f".{a}({'' if v == 'open' else v})" for a, v in conn.items())
    return f"{primitive}{par} {name}({pins});"


def main():
    os.makedirs(OUT, exist_ok=True)
    index = []
    index += design("iddr_sweep", "IDDR", IDDR)
    index += design("iddr_2clk_sweep", "IDDR_2CLK", IDDR_2CLK)
    index += design("oddr_sweep", "ODDR", ODDR)
    index += design("iddr_both", "IDDR", BOTH)
    index += design("oddr_both", "ODDR", BOTH)
    index += design("iddr_2clk_same", "IDDR_2CLK", SAME_CLOCK)
    index += serdes_design("iserdes_sweep", "ISERDESE2", ISERDES)
    index += serdes_design("iserdes_pins", "ISERDESE2", ISERDES_PINS_SWEEP)
    index += serdes_design("iserdes_wide", "ISERDESE2", ISERDES_WIDE, "pairs")
    index += serdes_design("iserdes_clocks", "ISERDESE2", ISERDES_CLOCKS)
    index += serdes_design("iserdes_clkdivp", "ISERDESE2", CLKDIVP)
    index += serdes_design("oserdes_sweep", "OSERDESE2", OSERDES)
    index += serdes_design("oserdes_pins", "OSERDESE2", OSERDES_PINS_SWEEP)
    index += serdes_design("oserdes_tristate", "OSERDESE2", OSERDES_TRISTATE)
    index += serdes_design("oserdes_wide", "OSERDESE2", OSERDES_WIDE, "pairs")
    index += serdes_design("serdes_feedback", "OSERDESE2", FEEDBACK)
    for name, (primitive, setting) in ALONE.items():
        index += serdes_design(name, primitive, [setting])
    open(f"{OUT}/index.txt", "w").write("# design port ball primitive setting\n" + "\n".join(index) + "\n")


main()
