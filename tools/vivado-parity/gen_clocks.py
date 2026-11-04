#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/clocks/: `gen_clocks.py <dir>`.

Each asks Vivado one thing about a clock buffer or a clock manager: one
parameter away from a plain design, or one wiring. `clocks.sh` runs them and
keeps what came back; `testdata/fpga/vivado/clocks/vivado.txt` holds the answers.
Only the answers are committed, and these designs are written here.

Every design has the same three pins and a shift register, so that each control
is a real signal of the design's own and nothing here is folded away. A buffer's
output clocks a flip-flop, whose output reaches `so`. A design that Vivado is
expected to refuse is named `no_*`.
"""
import os
import sys

# Every pad a design here may use. Two of each kind of clock-capable input, and
# in a second bank for what must not cross one.
PINS = {
    "clk": "N11",  # IO_L13P_T2_MRCC_14
    "si": "L4",
    "so": "L5",
    "clk2": "N14",  # IO_L12P_T1_MRCC_14
    "srcc": "N13",  # IO_L11P_T1_SRCC_14
    "plain": "L3",  # no clock capability
    "d_pad": "T10",
    "q_pad": "T9",
    "far": "C13",  # IO_L12N_T1_MRCC_15, another bank
    "far_d": "H11",  # bank 15
    "fb_out": "R16",
    "fb_in": "N13",  # IO_L11P_T1_SRCC_14
    "rpad": "D4",  # IO_L12P_T1_MRCC_35, on the right of the die
}


def design(name, comment, top, body, ports=()):
    """A module with a register `sr[top:0]`, `body`, and the wires `y` of it
    reaching `so`: `body` assigns `y`."""
    # Ten flip-flops or more: under eight clock pins Reticle puts no buffer on a
    # clock and Vivado does, and that is not what is being asked here.
    top = max(top, 9)
    extra = "".join(f",\n    {kind} wire {p}" for kind, p in ports)
    v = (
        f"// {comment}\n"
        f"module {name} (\n"
        "    input  wire clk,\n"
        "    input  wire si,\n"
        f"    output reg  so{extra}\n"
        ");\n"
        f"  reg [{top}:0] sr;\n"
        + (
            f"  always @(posedge clk) sr <= {{sr[{top - 1}:0], si}};\n"
            if top > 0
            else "  always @(posedge clk) sr <= si;\n"
        )
        + "  wire y;\n"
        + f"{body}"
        + "  always @(posedge clk) so <= y ^ sr[0];\n"
        "endmodule\n"
    )
    return v, ["clk", "si", "so"] + [p for _, p in ports]


CLOCKED = "  reg t;\n  always @(posedge o) t <= sr[0];\n  assign y = t;\n"


def named(params):
    return "#(" + ", ".join(f".{k}({v})" for k, v in params.items()) + ") " if params else ""


DESIGNS = {}
# Lines a design's `.xdc` has beyond its pins: a site, to see which bits that
# site's are. Only Vivado reads them; the `.rcf` has no such line.
LOCS = {}
# And lines its `.rcf` has: a clock both tools are told of.
RCF = {}


def add(name, comment, top, body, ports=()):
    DESIGNS[name] = design(name, comment, top, body, ports)


# --- BUFGCTRL -----------------------------------------------------------------
CTRL_PINS = {"CE0": 0, "CE1": 1, "S0": 2, "S1": 3, "IGNORE0": 4, "IGNORE1": 5}


def bufgctrl(name, comment, params=None, ties=None, cell="BUFGCTRL"):
    ties = ties or {}
    wires = ", ".join(
        f".{pin}({ties.get(pin, f'sr[{at}]')})" for pin, at in CTRL_PINS.items()
    )
    add(
        name,
        comment,
        5,
        "  wire o;\n"
        f"  {cell} {named(params)}dut (.O(o), .I0(clk), .I1(clk2), {wires});\n" + CLOCKED,
        [("input", "clk2")],
    )


bufgctrl("gctrl_all", "BUFGCTRL with every control a signal.")
for p, v in [
    ("INIT_OUT", "1"),
    ("PRESELECT_I0", '"TRUE"'),
    ("PRESELECT_I1", '"TRUE"'),
]:
    bufgctrl(f"gctrl_{p.lower()}", f"BUFGCTRL with {p}.", {p: v})
for pin in CTRL_PINS:
    bufgctrl(f"gctrl_inv_{pin.lower()}", f"BUFGCTRL, {pin} inverted.", {f"IS_{pin}_INVERTED": "1'b1"})
    for value in (0, 1):
        bufgctrl(f"gctrl_{pin.lower()}_{value}", f"BUFGCTRL, {pin} tied to {value}.", ties={pin: f"1'b{value}"})
bufgctrl(
    "gctrl_inv_tied",
    "BUFGCTRL, CE0 tied high and inverted.",
    {"IS_CE0_INVERTED": "1'b1"},
    {"CE0": "1'b1"},
)
bufgctrl(
    "gctrl_as_bufg",
    "BUFGCTRL with its pins where a BUFG's are.",
    ties={"CE0": "1'b1", "CE1": "1'b0", "S0": "1'b1", "S1": "1'b0", "IGNORE0": "1'b0", "IGNORE1": "1'b1"},
)

for name, comment, s in [
    ("gmuxctrl", "BUFGMUX_CTRL with a select.", "sr[0]"),
    ("gmuxctrl_s0", "BUFGMUX_CTRL, select tied low.", "1'b0"),
    ("gmuxctrl_s1", "BUFGMUX_CTRL, select tied high.", "1'b1"),
]:
    add(
        name,
        comment,
        1,
        f"  wire o;\n  BUFGMUX_CTRL dut (.O(o), .I0(clk), .I1(clk2), .S({s}));\n" + CLOCKED,
        [("input", "clk2")],
    )
add(
    "gmux_plain",
    "BUFGMUX, as a thing to compare BUFGMUX_CTRL with.",
    1,
    "  wire o;\n  BUFGMUX dut (.O(o), .I0(clk), .I1(clk2), .S(sr[0]));\n" + CLOCKED,
    [("input", "clk2")],
)

bufgctrl(
    "gctrl_inv_tied0",
    "BUFGCTRL, CE0 tied low and inverted.",
    {"IS_CE0_INVERTED": "1'b1"},
    {"CE0": "1'b0"},
)
add(
    "gctrl_open",
    "BUFGCTRL with only its clocks connected.",
    1,
    "  wire o;\n  BUFGCTRL dut (.O(o), .I0(clk), .I1(clk2));\n" + CLOCKED,
    [("input", "clk2")],
)
bufgctrl("gctrl_inv_i0", "BUFGCTRL, I0 inverted.", {"IS_I0_INVERTED": "1'b1"})
bufgctrl("gctrl_inv_i1", "BUFGCTRL, I1 inverted.", {"IS_I1_INVERTED": "1'b1"})

# --- The clock inputs ---------------------------------------------------------
for cell in ("IBUFGDS", "IBUFDS"):
    add(
        f"{cell.lower()}_bufg",
        f"{cell} into a BUFG.",
        1,
        f"  wire n, o;\n  {cell} pad (.O(n), .I(clk2), .IB(clk2b));\n  BUFG dut (.O(o), .I(n));\n" + CLOCKED,
        [("input", "clk2"), ("input", "clk2b")],
    )

# --- BUFH and BUFHCE ----------------------------------------------------------
add("bufh_pad", "BUFH on a clock pad, clocking a flip-flop.", 1, "  wire o;\n  BUFH dut (.O(o), .I(clk2));\n" + CLOCKED, [("input", "clk2")])
add(
    "bufh_bufg",
    "BUFH on a BUFG, clocking a flip-flop.",
    1,
    "  wire g, o;\n  BUFG up (.O(g), .I(clk2));\n  BUFH dut (.O(o), .I(g));\n" + CLOCKED,
    [("input", "clk2")],
)
add(
    "bufh_data",
    "BUFH's output read as data, as the primitive's probe does.",
    1,
    "  wire o;\n  BUFH dut (.O(o), .I(clk));\n  reg q;\n  always @(posedge clk) q <= o;\n  assign y = q;\n",
)
for name, comment, params, ce in [
    ("bufhce_ce", "BUFHCE with an enable.", None, "sr[1]"),
    ("bufhce_ce1", "BUFHCE, enable tied high.", None, "1'b1"),
    ("bufhce_ce0", "BUFHCE, enable tied low.", None, "1'b0"),
    ("bufhce_async", "BUFHCE, CE_TYPE ASYNC.", {"CE_TYPE": '"ASYNC"'}, "sr[1]"),
    ("bufhce_init1", "BUFHCE, INIT_OUT 1.", {"INIT_OUT": "1"}, "sr[1]"),
    ("bufhce_inv", "BUFHCE, enable inverted.", {"IS_CE_INVERTED": "1'b1"}, "sr[1]"),
]:
    add(
        name,
        comment,
        1,
        f"  wire g, o;\n  BUFG up (.O(g), .I(clk2));\n  BUFHCE {named(params)}dut (.O(o), .CE({ce}), .I(g));\n" + CLOCKED,
        [("input", "clk2")],
    )
add(
    "bufh_two",
    "Two BUFHs on one BUFG, each with its own flip-flops.",
    1,
    "  wire g, o, p;\n  BUFG up (.O(g), .I(clk2));\n  BUFH a (.O(o), .I(g));\n  BUFH b (.O(p), .I(g));\n"
    "  reg t, u;\n  always @(posedge o) t <= sr[0];\n  always @(posedge p) u <= sr[1];\n  assign y = t ^ u;\n",
    [("input", "clk2")],
)

# --- BUFR ---------------------------------------------------------------------


def bufr(name, comment, params=None, ce="1'b1", clr="1'b0", i="clk2", more="", ports=(("input", "clk2"),)):
    add(
        name,
        comment,
        2,
        f"  wire o;\n{more}  BUFR {named(params)}dut (.O(o), .CE({ce}), .CLR({clr}), .I({i}));\n" + CLOCKED,
        list(ports),
    )


bufr("bufr_plain", "BUFR on a clock pad, not dividing.")
for div in ("1", "2", "3", "4", "5", "6", "7", "8"):
    bufr(f"bufr_div{div}", f"BUFR dividing by {div}.", {"BUFR_DIVIDE": f'"{div}"'})
bufr("bufr_ce", "BUFR with an enable.", ce="sr[1]")
bufr("bufr_clr", "BUFR with a clear.", clr="sr[1]")
bufr("bufr_both", "BUFR with an enable and a clear, dividing.", {"BUFR_DIVIDE": '"4"'}, ce="sr[1]", clr="sr[2]")
bufr("bufr_ce0", "BUFR, enable tied low.", ce="1'b0")
bufr("bufr_clr1", "BUFR, clear tied high.", clr="1'b1")
bufr("bufr_srcc", "BUFR on a single-region clock pad.", i="srcc", ports=(("input", "srcc"),))
bufr("bufr_far", "BUFR on a clock pad of another bank.", i="far", ports=(("input", "far"),))
bufr(
    "bufr_bufg",
    "BUFR on a BUFG.",
    i="g",
    more="  wire g;\n  BUFG up (.O(g), .I(clk2));\n",
)
bufr(
    "bufr_fabric",
    "BUFR on a flip-flop's output.",
    i="d",
    more="  reg d;\n  always @(posedge clk) d <= sr[1];\n",
    ports=(),
)
bufr("bufr_plainpad", "BUFR on a pad with no clock capability.", i="plain", ports=(("input", "plain"),))
add(
    "bufr_mmcm",
    "BUFR on an MMCM's output.",
    2,
    "  wire fb, c0, o;\n"
    "  MMCME2_ADV #(.CLKFBOUT_MULT_F(8.0), .CLKIN1_PERIOD(10.0), .CLKOUT0_DIVIDE_F(8.0), .COMPENSATION(\"INTERNAL\")) m "
    "(.CLKFBOUT(fb), .CLKOUT0(c0), .CLKFBIN(fb), .CLKIN1(clk2), .CLKIN2(1'b0), .CLKINSEL(1'b1), .DADDR(7'b0), .DCLK(1'b0), "
    ".DEN(1'b0), .DI(16'b0), .DWE(1'b0), .PSCLK(1'b0), .PSEN(1'b0), .PSINCDEC(1'b0), .PWRDWN(1'b0), .RST(1'b0));\n"
    "  BUFR dut (.O(o), .CE(1'b1), .CLR(1'b0), .I(c0));\n" + CLOCKED,
    [("input", "clk2")],
)

for n in range(4):
    bufr(f"bufr_loc{n}", f"BUFR on site {n} of its row.")
    LOCS[f"bufr_loc{n}"] = f"set_property LOC BUFR_X0Y{4 + n} [get_cells dut]\n"
    bufr(f"bufr_right{n}", f"BUFR on site {n} of a row on the right.", i="rpad", ports=(("input", "rpad"),))
    LOCS[f"bufr_right{n}"] = f"set_property LOC BUFR_X1Y{8 + n} [get_cells dut]\n"

BUFR_ON_MR = "  BUFR u_r (.O(o), .CE(1'b1), .CLR(1'b0), .I(mr));\n" + CLOCKED

# --- BUFIO --------------------------------------------------------------------
IDDR_LOAD = "  IDDR u_load (.C(o), .CE(1'b1), .D(d_pad), .S(1'b0), .R(1'b0), .Q1(q1), .Q2());\n  assign y = q1;\n"
add(
    "bufio_iddr",
    "BUFIO on a clock pad, clocking an IDDR on a pad of its bank.",
    1,
    "  wire o, q1;\n  BUFIO dut (.O(o), .I(clk2));\n" + IDDR_LOAD,
    [("input", "clk2"), ("input", "d_pad")],
)
add(
    "bufio_oddr",
    "BUFIO on a clock pad, clocking an ODDR on a pad of its bank.",
    1,
    "  wire o, q;\n  BUFIO dut (.O(o), .I(clk2));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("input", "clk2"), ("output", "q_pad")],
)
add(
    "bufio_srcc",
    "BUFIO on a single-region clock pad.",
    1,
    "  wire o, q;\n  BUFIO dut (.O(o), .I(srcc));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("input", "srcc"), ("output", "q_pad")],
)
for n in range(4):
    add(
        f"bufio_loc{n}",
        f"BUFIO on site {n} of its row.",
        1,
        "  wire o, q;\n  BUFIO dut (.O(o), .I(clk2));\n"
        "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
        "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
        [("input", "clk2"), ("output", "q_pad")],
    )
    LOCS[f"bufio_loc{n}"] = f"set_property LOC BUFIO_X0Y{4 + n} [get_cells dut]\n"
add(
    "no_bufio_ff",
    "BUFIO clocking a slice's flip-flop, which it cannot reach.",
    1,
    "  wire o;\n  BUFIO dut (.O(o), .I(clk2));\n" + CLOCKED,
    [("input", "clk2")],
)
add(
    "no_bufio_data",
    "BUFIO's output read as data.",
    1,
    "  wire o;\n  BUFIO dut (.O(o), .I(clk2));\n  reg t;\n  always @(posedge clk) t <= o;\n  assign y = t;\n",
    [("input", "clk2")],
)
add(
    "no_bufio_fabric",
    "BUFIO on a flip-flop's output.",
    1,
    "  wire o, q;\n  reg d;\n  always @(posedge clk) d <= sr[1];\n  BUFIO dut (.O(o), .I(d));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("output", "q_pad")],
)
add(
    "no_bufio_bufg",
    "BUFIO on a BUFG.",
    1,
    "  wire g, o, q;\n  BUFG up (.O(g), .I(clk2));\n  BUFIO dut (.O(o), .I(g));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("input", "clk2"), ("output", "q_pad")],
)
add(
    "no_bufio_far",
    "BUFIO on a pad of one bank clocking an ODDR on a pad of another.",
    1,
    "  wire o, q;\n  BUFIO dut (.O(o), .I(clk2));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(far_d), .I(q));\n  assign y = sr[1];\n",
    [("input", "clk2"), ("output", "far_d")],
)
add(
    "no_bufio_plain",
    "BUFIO on a pad with no clock capability.",
    1,
    "  wire o, q;\n  BUFIO dut (.O(o), .I(plain));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("input", "plain"), ("output", "q_pad")],
)

# --- BUFMR and BUFMRCE --------------------------------------------------------
add(
    "bufmr_bufr",
    "BUFMR on a clock pad, driving a BUFR.",
    1,
    "  wire mr, o;\n  BUFMR dut (.O(mr), .I(clk2));\n" + BUFR_ON_MR,
    [("input", "clk2")],
)
add(
    "bufmr_bufio",
    "BUFMR on a clock pad, driving a BUFIO that clocks an ODDR.",
    1,
    "  wire mr, o, q;\n  BUFMR dut (.O(mr), .I(clk2));\n  BUFIO u_i (.O(o), .I(mr));\n"
    "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("input", "clk2"), ("output", "q_pad")],
)
for name, comment, params, ce in [
    ("bufmrce_ce", "BUFMRCE with an enable.", None, "sr[1]"),
    ("bufmrce_ce1", "BUFMRCE, enable tied high.", None, "1'b1"),
    ("bufmrce_ce0", "BUFMRCE, enable tied low.", None, "1'b0"),
    ("bufmrce_async", "BUFMRCE, CE_TYPE ASYNC.", {"CE_TYPE": '"ASYNC"'}, "sr[1]"),
    ("bufmrce_init1", "BUFMRCE, INIT_OUT 1.", {"INIT_OUT": "1"}, "sr[1]"),
    ("bufmrce_inv", "BUFMRCE, enable inverted.", {"IS_CE_INVERTED": "1'b1"}, "sr[1]"),
]:
    add(
        name,
        comment,
        1,
        f"  wire mr, o;\n  BUFMRCE {named(params)}dut (.O(mr), .CE({ce}), .I(clk2));\n" + BUFR_ON_MR,
        [("input", "clk2")],
    )
add(
    "bufmr_srcc",
    "BUFMR on a single-region clock pad.",
    1,
    "  wire mr, o;\n  BUFMR dut (.O(mr), .I(srcc));\n" + BUFR_ON_MR,
    [("input", "srcc")],
)
add(
    "no_bufmr_ff",
    "BUFMR clocking a slice's flip-flop.",
    1,
    "  wire o;\n  BUFMR dut (.O(o), .I(clk2));\n" + CLOCKED,
    [("input", "clk2")],
)
add(
    "bufmr_fabric",
    "BUFMR on a flip-flop's output.",
    1,
    "  wire mr, o;\n  reg d;\n  always @(posedge clk) d <= sr[1];\n  BUFMR dut (.O(mr), .I(d));\n" + BUFR_ON_MR,
)
add(
    "no_bufmr_bufg",
    "BUFMR driving a BUFG.",
    1,
    "  wire mr, o;\n  BUFMR dut (.O(mr), .I(clk2));\n  BUFG u (.O(o), .I(mr));\n" + CLOCKED,
    [("input", "clk2")],
)

# --- MMCM and PLL -------------------------------------------------------------
DRP_PLL = [".DADDR(7'b0)", ".DCLK(1'b0)", ".DEN(1'b0)", ".DI(16'b0)", ".DWE(1'b0)"]
DRP_MMCM = DRP_PLL + [".PSCLK(1'b0)", ".PSEN(1'b0)", ".PSINCDEC(1'b0)"]


def manager(
    name,
    comment,
    cell,
    params=None,
    outs=("CLKOUT0",),
    feedback="local",
    conn="",
    top=1,
    ports=(),
):
    """A clock manager with `outs` on BUFGs, each clocking a flip-flop.

    `feedback` is `local` (CLKFBOUT to CLKFBIN), `bufg`, or `pad` (out one pad
    and in another).
    """
    mmcm = cell.startswith("MMCM")
    base = cell.endswith("_BASE")
    mult = "CLKFBOUT_MULT_F" if mmcm else "CLKFBOUT_MULT"
    div0 = "CLKOUT0_DIVIDE_F" if mmcm else "CLKOUT0_DIVIDE"
    p = {mult: "8.0" if mmcm else "8", "CLKIN1_PERIOD": "10.0", div0: "8.0" if mmcm else "8"}
    p.update(params or {})
    body = "  wire fb, fbi, locked;\n"
    if feedback == "local":
        body += "  assign fbi = fb;\n"
    elif feedback == "bufg":
        body += "  BUFG u_fb (.O(fbi), .I(fb));\n"
    else:
        body += "  OBUF u_fbo (.O(fb_out), .I(fb));\n  IBUF u_fbi (.O(fbi), .I(fb_in));\n"
        ports = tuple(ports) + (("output", "fb_out"), ("input", "fb_in"))
    conns = [".CLKFBOUT(fb)", ".CLKFBIN(fbi)", ".CLKIN1(clk2)", ".LOCKED(locked)", ".RST(1'b0)", ".PWRDWN(1'b0)"]
    if not base:
        conns += DRP_MMCM if mmcm else DRP_PLL
        if "CLKINSEL" not in conn:
            conns += [".CLKIN2(1'b0)", ".CLKINSEL(1'b1)"]
    for i, o in enumerate(outs):
        body += f"  wire c{i}, g{i};\n  BUFG u_g{i} (.O(g{i}), .I(c{i}));\n"
        conns.append(f".{o}(c{i})")
    if conn:
        conns = [c for c in conns if not any(c.startswith(f".{x.split('(')[0].strip().lstrip('.')}(") for x in conn.split(", "))]
        conns.append(conn)
    body += f"  {cell} {named(p)}dut ({', '.join(conns)});\n"
    terms = []
    for i in range(len(outs)):
        body += f"  reg t{i};\n  always @(posedge g{i}) t{i} <= locked;\n"
        terms.append(f"t{i}")
    body += f"  assign y = {' ^ '.join(terms)};\n"
    add(name, comment, top, body, [("input", "clk2")] + list(ports))


for cell, short in [("MMCME2_ADV", "mmcm"), ("PLLE2_ADV", "pll")]:
    mmcm = short == "mmcm"
    manager(f"{short}_plain", f"{cell}, compensation left to its default, feedback direct.", cell)
    manager(f"{short}_base", f"{cell[:-3]}BASE with the same parameters.", cell[:-3] + "BASE")
    manager(f"{short}_fb_bufg", f"{cell} with feedback through a BUFG.", cell, feedback="bufg")
    manager(f"{short}_fb_pad", f"{cell} with feedback through two pads.", cell, feedback="pad", params={"COMPENSATION": '"EXTERNAL"'}, ports=())
    for comp, fb in [("INTERNAL", "local"), ("ZHOLD", "local"), ("ZHOLD", "bufg"), ("BUF_IN", "local"), ("BUF_IN", "bufg"), ("EXTERNAL", "bufg")] + ([("INTERNAL", "bufg")]):
        manager(
            f"{short}_comp_{comp.lower()}_{fb}",
            f"{cell}, COMPENSATION {comp}, feedback {fb}.",
            cell,
            {"COMPENSATION": f'"{comp}"'},
            feedback=fb,
        )
    manager(f"{short}_startup_wait", f"{cell}, STARTUP_WAIT.", cell, {"STARTUP_WAIT": '"TRUE"'})
    for band in ("LOW", "HIGH", "OPTIMIZED"):
        manager(f"{short}_band_{band.lower()}", f"{cell}, BANDWIDTH {band}.", cell, {"BANDWIDTH": f'"{band}"'})
    for d in (2, 3):
        manager(
            f"{short}_divclk{d}",
            f"{cell}, DIVCLK_DIVIDE {d}.",
            cell,
            {"DIVCLK_DIVIDE": str(d), "CLKFBOUT_MULT_F" if mmcm else "CLKFBOUT_MULT": ("16.0" if mmcm else "16") if d == 2 else ("24.0" if mmcm else "24")},
        )
    # Phase and duty cycle, each on one output.
    for phase in ("11.25", "22.5", "45.0", "90.0", "180.0", "270.0", "-45.0"):
        manager(
            f"{short}_phase_{phase.replace('.', 'p').replace('-', 'm')}",
            f"{cell}, CLKOUT1_PHASE {phase}.",
            cell,
            {"CLKOUT1_DIVIDE": "8", "CLKOUT1_PHASE": phase},
            outs=("CLKOUT0", "CLKOUT1"),
        )
    manager(f"{short}_phase0", f"{cell}, CLKOUT0_PHASE 90.", cell, {"CLKOUT0_PHASE": "90.0"})
    manager(f"{short}_phasefb", f"{cell}, CLKFBOUT_PHASE 90.", cell, {"CLKFBOUT_PHASE": "90.0"})
    for duty in ("0.25", "0.33", "0.75"):
        manager(
            f"{short}_duty_{duty.replace('.', 'p')}",
            f"{cell}, CLKOUT1_DUTY_CYCLE {duty}.",
            cell,
            {"CLKOUT1_DIVIDE": "8", "CLKOUT1_DUTY_CYCLE": duty},
            outs=("CLKOUT0", "CLKOUT1"),
        )
    manager(f"{short}_duty_odd", f"{cell}, an odd division with an unequal duty cycle.", cell, {"CLKOUT1_DIVIDE": "7", "CLKOUT1_DUTY_CYCLE": "0.25"}, outs=("CLKOUT0", "CLKOUT1"))
    manager(f"{short}_duty0", f"{cell}, CLKOUT0_DUTY_CYCLE 0.25.", cell, {"CLKOUT0_DUTY_CYCLE": "0.25"})
    for inv in ("RST", "PWRDWN", "CLKINSEL"):
        manager(f"{short}_inv_{inv.lower()}", f"{cell}, IS_{inv}_INVERTED.", cell, {f"IS_{inv}_INVERTED": "1'b1"})
    manager(
        f"{short}_rst",
        f"{cell} with a reset and a power-down that are signals.",
        cell,
        conn=".RST(sr[0]), .PWRDWN(sr[1])",
    )
    manager(
        f"{short}_clkinsel",
        f"{cell} choosing between two clocks with a signal.",
        cell,
        {"CLKIN2_PERIOD": "10.0"},
        conn=".CLKIN2(clk), .CLKINSEL(sr[0])",
    )
    manager(
        f"{short}_clkin2",
        f"{cell} on CLKIN2, CLKINSEL tied low.",
        cell,
        {"CLKIN2_PERIOD": "10.0"},
        conn=".CLKIN2(clk), .CLKINSEL(1'b0)",
    )
    manager(
        f"{short}_drp",
        f"{cell} with its dynamic reconfiguration port in use.",
        cell,
        conn=".DADDR(sr[6:0]), .DCLK(clk), .DEN(sr[7]), .DI(sr[23:8]), .DWE(sr[24])",
        top=24,
    )
    manager(
        f"{short}_all_outs",
        f"{cell} with all its plain outputs in use, each a division of its own.",
        cell,
        ({"CLKOUT1_DIVIDE": "9", "CLKOUT2_DIVIDE": "10", "CLKOUT3_DIVIDE": "11", "CLKOUT4_DIVIDE": "12", "CLKOUT5_DIVIDE": "13", "CLKOUT6_DIVIDE": "14"} if mmcm else {"CLKOUT1_DIVIDE": "9", "CLKOUT2_DIVIDE": "10", "CLKOUT3_DIVIDE": "11", "CLKOUT4_DIVIDE": "12", "CLKOUT5_DIVIDE": "13"}),
        outs=tuple(f"CLKOUT{i}" for i in range(7 if mmcm else 6)),
    )
    manager(f"{short}_ref_jitter", f"{cell}, REF_JITTER1.", cell, {"REF_JITTER1": "0.1"})

# What only an MMCM has.
for name, comment, params, outs, conn in [
    ("mmcm_frac_mult", "CLKFBOUT_MULT_F fractional.", {"CLKFBOUT_MULT_F": "8.125", "CLKOUT0_DIVIDE_F": "8.0"}, ("CLKOUT0",), ""),
    ("mmcm_frac_mult2", "CLKFBOUT_MULT_F of 6.875.", {"CLKFBOUT_MULT_F": "6.875"}, ("CLKOUT0",), ""),
    ("mmcm_frac_div", "CLKOUT0_DIVIDE_F fractional.", {"CLKOUT0_DIVIDE_F": "8.125"}, ("CLKOUT0",), ""),
    ("mmcm_frac_div2", "CLKOUT0_DIVIDE_F of 7.875.", {"CLKOUT0_DIVIDE_F": "7.875"}, ("CLKOUT0",), ""),
    ("mmcm_frac_both", "Both fractional.", {"CLKFBOUT_MULT_F": "8.5", "CLKOUT0_DIVIDE_F": "9.25"}, ("CLKOUT0",), ""),
    ("mmcm_frac_phase", "A fractional divider with a phase on it.", {"CLKOUT0_DIVIDE_F": "8.125", "CLKOUT0_PHASE": "45.0"}, ("CLKOUT0",), ""),
    ("mmcm_casc4", "CLKOUT4_CASCADE.", {"CLKOUT4_CASCADE": '"TRUE"', "CLKOUT4_DIVIDE": "4", "CLKOUT6_DIVIDE": "2"}, ("CLKOUT0", "CLKOUT4"), ""),
    ("mmcm_fine0", "CLKOUT0_USE_FINE_PS.", {"CLKOUT0_USE_FINE_PS": '"TRUE"'}, ("CLKOUT0",), ""),
    ("mmcm_finefb", "CLKFBOUT_USE_FINE_PS.", {"CLKFBOUT_USE_FINE_PS": '"TRUE"'}, ("CLKOUT0",), ""),
    ("mmcm_fine_signal", "CLKOUT0_USE_FINE_PS, with phase-shift pins that are signals.", {"CLKOUT0_USE_FINE_PS": '"TRUE"'}, ("CLKOUT0",), ".PSCLK(clk), .PSEN(sr[0]), .PSINCDEC(sr[1])"),
    ("mmcm_inv_psen", "IS_PSEN_INVERTED.", {"IS_PSEN_INVERTED": "1'b1"}, ("CLKOUT0",), ""),
    ("mmcm_inv_psincdec", "IS_PSINCDEC_INVERTED.", {"IS_PSINCDEC_INVERTED": "1'b1"}, ("CLKOUT0",), ""),
    ("mmcm_ss", "Spread spectrum.", {"SS_EN": '"TRUE"', "SS_MODE": '"CENTER_HIGH"', "SS_MOD_PERIOD": "10000", "CLKFBOUT_MULT_F": "8.0", "CLKOUT0_DIVIDE_F": "8.0", "BANDWIDTH": '"LOW"'}, ("CLKOUT0",), ""),
    ("mmcm_outb", "CLKOUT0B and CLKOUT1B in use.", {"CLKOUT1_DIVIDE": "8"}, ("CLKOUT0B", "CLKOUT1B"), ""),
    ("mmcm_fboutb", "CLKFBOUTB as an output.", None, ("CLKOUT0", "CLKFBOUTB"), ""),
    ("mmcm_status", "CLKINSTOPPED and CLKFBSTOPPED read.", None, ("CLKOUT0",), ""),
]:
    manager(name, f"MMCME2_ADV, {comment}", "MMCME2_ADV", params, outs, conn=conn, top=2)

# Every way a BUFMR reaches a BUFR or a BUFIO: each of its two sites in a row,
# against each buffer of its own row and the rows either side.
for mr in (2, 3):
    for r in range(12):
        name = f"bufmr_x{mr}_r{r}"
        add(
            name,
            f"BUFMRCE_X0Y{mr} driving BUFR_X0Y{r}.",
            1,
            "  wire mr, o;\n  BUFMR dut (.O(mr), .I(clk2));\n" + BUFR_ON_MR,
            [("input", "clk2")],
        )
        LOCS[name] = (
            f"set_property LOC BUFMRCE_X0Y{mr} [get_cells dut]\n"
            f"set_property LOC BUFR_X0Y{r} [get_cells u_r]\n"
        )
    for r in range(4, 8):
        name = f"bufmr_x{mr}_io{r}"
        add(
            name,
            f"BUFMRCE_X0Y{mr} driving BUFIO_X0Y{r}.",
            1,
            "  wire mr, o, q;\n  BUFMR dut (.O(mr), .I(clk2));\n  BUFIO u_i (.O(o), .I(mr));\n"
            "  ODDR u_load (.C(o), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
            "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
            [("input", "clk2"), ("output", "q_pad")],
        )
        LOCS[name] = (
            f"set_property LOC BUFMRCE_X0Y{mr} [get_cells dut]\n"
            f"set_property LOC BUFIO_X0Y{r} [get_cells u_i]\n"
        )
for r in range(8, 12):
    name = f"bufmr_right_r{r}"
    add(
        name,
        f"A BUFMR on the right of the die driving BUFR_X1Y{r}.",
        1,
        "  wire mr, o;\n  BUFMR dut (.O(mr), .I(rpad));\n" + BUFR_ON_MR,
        [("input", "rpad")],
    )
    LOCS[name] = f"set_property LOC BUFR_X1Y{r} [get_cells u_r]\n"

# Each parameter of the other of a row's two BUFMRCEs.
for name, comment, params, ce in [
    ("bufmrce_s0_ce", "with an enable", None, "sr[1]"),
    ("bufmrce_s0_ce1", "enable tied high", None, "1'b1"),
    ("bufmrce_s0_ce0", "enable tied low", None, "1'b0"),
    ("bufmrce_s0_async", "CE_TYPE ASYNC", {"CE_TYPE": '"ASYNC"'}, "sr[1]"),
    ("bufmrce_s0_init1", "INIT_OUT 1", {"INIT_OUT": "1"}, "sr[1]"),
    ("bufmrce_s0_inv", "enable inverted", {"IS_CE_INVERTED": "1'b1"}, "sr[1]"),
]:
    add(
        name,
        f"BUFMRCE on the first site of its row, {comment}.",
        1,
        f"  wire mr, o;\n  BUFMRCE {named(params)}dut (.O(mr), .CE({ce}), .I(clk2));\n" + BUFR_ON_MR,
        [("input", "clk2")],
    )
    LOCS[name] = "set_property LOC BUFMRCE_X0Y2 [get_cells dut]\n"
add(
    "bufmr_s0_fabric",
    "BUFMR on the first site of its row, on a flip-flop's output.",
    1,
    "  wire mr, o;\n  reg d;\n  always @(posedge clk) d <= sr[1];\n  BUFMR dut (.O(mr), .I(d));\n" + BUFR_ON_MR,
)
LOCS["bufmr_s0_fabric"] = "set_property LOC BUFMRCE_X0Y2 [get_cells dut]\n"
add(
    "bufmr_ccio3",
    "BUFMR on the second site of its row, on the pad of CCIO3.",
    1,
    "  wire mr, o;\n  BUFMR dut (.O(mr), .I(clk));\n" + BUFR_ON_MR,
)
LOCS["bufmr_ccio3"] = "set_property LOC BUFMRCE_X0Y3 [get_cells dut]\n"
# A clock manager on the right of the die, whose tiles are the other type.
for short, cell in [("mmcm", "MMCME2_ADV"), ("pll", "PLLE2_ADV")]:
    def on_right(name, comment, **more):
        manager(name, comment, cell, more.get("params"), feedback=more.get("feedback", "local"))
        text, ports = DESIGNS[name]
        DESIGNS[name] = (text.replace("clk2", "rpad"), [p.replace("clk2", "rpad") for p in ports])
    on_right(f"{short}_right", f"{cell} on the right of the die.")
    on_right(f"{short}_right_zhold", f"{cell} on the right, feedback through a BUFG.", feedback="bufg")
    for comp, fb in [("ZHOLD", "pad"), ("BUF_IN", "pad"), ("INTERNAL", "pad"), ("EXTERNAL", "local")]:
        manager(
            f"{short}_comp_{comp.lower()}_{fb}",
            f"{cell}, COMPENSATION {comp}, feedback {fb}.",
            cell,
            {"COMPENSATION": f'"{comp}"'},
            feedback=fb,
        )
    manager(
        f"{short}_clkin2_pads",
        f"{cell} choosing between two clock pads with a signal, with a reset.",
        cell,
        {"CLKIN2_PERIOD": "10.0"},
        conn=".CLKIN2(srcc), .CLKINSEL(sr[0]), .RST(sr[1])",
        ports=(("input", "srcc"),),
    )
# Whether a clock on the input pad stands for CLKIN1_PERIOD in Vivado's rules:
# the attribute alone says 10 MHz, which puts the VCO at 60, and the clock says
# 100 MHz, which puts it at 600. NewAE's glitch MMCM is the first.
for short, cell, mult in [("mmcm", "MMCME2_ADV", "CLKFBOUT_MULT_F"), ("pll", "PLLE2_ADV", "CLKFBOUT_MULT")]:
    for tag, clocked in [("period_clocked", True), ("period_alone", False)]:
        name = f"{short}_{tag}"
        manager(name, f"{cell}, CLKIN1_PERIOD 100 with a VCO of 60 MHz by it.", cell,
                {"CLKIN1_PERIOD": "100.0", mult: "6.0" if short == "mmcm" else "6", ("CLKOUT0_DIVIDE_F" if short == "mmcm" else "CLKOUT0_DIVIDE"): "6.0" if short == "mmcm" else "6"})
        if clocked:
            LOCS[name] = "create_clock -period 10 -name clk2 [get_ports clk2]\n"
            RCF[name] = "create_clock -name clk2 -period 10 clk2\n"
for name, comment, params, conn in [
    ("mmcm_fine_psen", "fine phase shift, PSEN alone a signal.", {"CLKOUT0_USE_FINE_PS": '"TRUE"'}, ".PSEN(sr[0])"),
    ("mmcm_fine_psclk", "fine phase shift, PSCLK alone a signal.", {"CLKOUT0_USE_FINE_PS": '"TRUE"'}, ".PSCLK(clk)"),
    ("mmcm_fine_psincdec", "fine phase shift, PSINCDEC alone a signal.", {"CLKOUT0_USE_FINE_PS": '"TRUE"'}, ".PSINCDEC(sr[0])"),
    ("mmcm_ps_signal", "phase-shift pins that are signals, with no output using them.", None, ".PSCLK(clk), .PSEN(sr[0]), .PSINCDEC(sr[1])"),
]:
    manager(name, f"MMCME2_ADV, {comment}", "MMCME2_ADV", params, conn=conn, top=2)
add(
    "oddr_bufg",
    "An ODDR on a global clock, for what is set for it beside a BUFIO's.",
    1,
    "  wire q;\n"
    "  ODDR u_load (.C(clk), .CE(1'b1), .D1(sr[0]), .D2(sr[1]), .S(1'b0), .R(1'b0), .Q(q));\n"
    "  OBUF u_q (.O(q_pad), .I(q));\n  assign y = sr[1];\n",
    [("output", "q_pad")],
)


def write(out):
    os.makedirs(out, exist_ok=True)
    for name, (v, ports) in DESIGNS.items():
        with open(os.path.join(out, f"{name}.v"), "w") as f:
            f.write(v)
        xdc = "create_clock -period 10 -name clk [get_ports clk]\n"
        rcf = "create_clock -name clk -period 10 clk\n"
        for p in ports:
            differential = p in ("clk2", "clk2b") and name.startswith("ibuf")
            # A pair in a bank of its own, as a 2.5 V standard wants.
            pin = {"clk2": "D4", "clk2b": "C4"}[p] if differential else PINS[p]
            std = "LVDS_25" if differential else "LVCMOS33"
            xdc += f"set_property PACKAGE_PIN {pin} [get_ports {p}]\n"
            xdc += f"set_property IOSTANDARD {std} [get_ports {p}]\n"
            rcf += f"set_io -io_standard {std} {p} {pin}\n"
        xdc += LOCS.get(name, "")
        with open(os.path.join(out, f"{name}.xdc"), "w") as f:
            f.write(xdc)
        rcf += RCF.get(name, "")
        with open(os.path.join(out, f"{name}.rcf"), "w") as f:
            f.write(rcf)


if __name__ == "__main__":
    write(sys.argv[1])
    print(len(DESIGNS), "designs")
