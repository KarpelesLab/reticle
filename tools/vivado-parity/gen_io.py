#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/io/: what Vivado does with a pad.

    tools/vivado-parity/gen_io.py testdata/fpga/vivado/io

Two kinds. A sweep, `<o|t|i|b|di|do|dt|db>_<standard>`, has one standard in one
direction on the pads of bank 35, one setting per pair of balls, so both halves
of a tile answer. A case has a few pads, to ask one thing of a bank. Every
standard is what `list_property_value IOSTANDARD` gives for a port here.

`index.txt` says which pad has which setting. A design Vivado refuses is kept,
with the rule named in `refusals.txt`: `tests/fpga_xray_pads.rs` holds Reticle
to the same refusal.

What Vivado says is a mistake and carries on from (`Illegal DRIVE`, `TUNED_SPLIT`)
is left out of the sweep, since Reticle refuses those with the rest of a
constraints file; and `SLEW MEDIUM`, a warning there and a refusal here. Each
has a case of its own.
"""
import itertools
import os
import sys

OUT = sys.argv[1]
SE = ("LVTTL LVCMOS33 LVCMOS25 LVCMOS18 LVCMOS15 LVCMOS12 HSUL_12 HSTL_I HSTL_II HSTL_I_18 HSTL_II_18 SSTL18_I "
      "SSTL18_II SSTL15 SSTL15_R SSTL135 SSTL135_R PCI33_3 MOBILE_DDR").split()
DIFF = ("DIFF_HSTL_I DIFF_HSTL_II DIFF_HSTL_I_18 DIFF_HSTL_II_18 DIFF_SSTL18_I DIFF_SSTL18_II DIFF_SSTL15 "
        "DIFF_SSTL15_R DIFF_SSTL135 DIFF_SSTL135_R DIFF_HSUL_12 DIFF_MOBILE_DDR BLVDS_25 LVDS_25 RSDS_25 TMDS_33 "
        "MINI_LVDS_25 PPDS_25").split()
ONE_WAY = set("HSTL_I HSTL_I_18 SSTL18_I DIFF_HSTL_I DIFF_HSTL_I_18 DIFF_SSTL18_I PPDS_25 RSDS_25 TMDS_33".split())
DRIVES = {"LVTTL": (4, 8, 12, 16, 24), "LVCMOS33": (4, 8, 12, 16), "LVCMOS25": (4, 8, 12, 16),
          "LVCMOS18": (4, 8, 12, 16, 24), "LVCMOS15": (4, 8, 12, 16), "LVCMOS12": (4, 8, 12)}
TERMED = set("HSTL_I HSTL_II HSTL_I_18 HSTL_II_18 SSTL18_I SSTL18_II SSTL15 SSTL15_R SSTL135 SSTL135_R DIFF_HSTL_I "
             "DIFF_HSTL_II DIFF_HSTL_I_18 DIFF_HSTL_II_18 DIFF_SSTL18_I DIFF_SSTL18_II DIFF_SSTL15 DIFF_SSTL15_R "
             "DIFF_SSTL135 DIFF_SSTL135_R".split())
# Twenty-two pairs of bank 35, `_P` then `_N`, leaving its two VREF balls alone.
PAIRS = [("F4", "F3"), ("G5", "G4"), ("G2", "G1"), ("H2", "H1"), ("H5", "H4"), ("F2", "E1"), ("F5", "E5"), ("D4", "C4"),
         ("E3", "D3"), ("E2", "D1"), ("C1", "B1"), ("B2", "A2"), ("C3", "C2"), ("C7", "C6"), ("B4", "A3"), ("A5", "A4"),
         ("B6", "B5"), ("B7", "A7"), ("J3", "H3"), ("K1", "J1"), ("L3", "L2"), ("K3", "K2")]
PULLS = [{"PULLTYPE": p} for p in ("PULLUP", "PULLDOWN", "KEEPER")]
TERMS = [{"IN_TERM": f"UNTUNED_SPLIT_{n}"} for n in (40, 50, 60)]


def tag(p):
    return ",".join(f"{k}={v}" for k, v in sorted(p.items())) or "default"


def write(name, ports, body, reads, xdc, helpers=("L4", "L5", "M5")):
    expr = " ^ ".join(reads) or "1'b0"
    text = f"module {name}(input en, input dn, output r{''.join(', ' + p for p in ports)});\n"
    text += "".join(f"  {b}\n" for b in body) + f"  assign r = {expr};\nendmodule\n"
    open(f"{OUT}/{name}.v", "w").write(text)
    xdc = xdc + [f"set_property -dict {{PACKAGE_PIN {b} IOSTANDARD LVCMOS33}} [get_ports {p}]"
                 for p, b in zip(("dn", "en", "r"), helpers)]
    open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")


def props(ball, std, c):
    items = ([f"PACKAGE_PIN {ball}"] if ball else []) + ([f"IOSTANDARD {std}"] if std else [])
    items += [f"{k} {v}" for k, v in c.items()]
    return "{" + " ".join(items) + "}"


def buffers(kind, k):
    """The ports, instance lines and signals read of one pair of pads."""
    p, n = f"a{k}p", f"a{k}n"
    d = {"out": "output", "tri": "output", "in": "input", "inout": "inout", "din": "input", "dout": "output",
         "dtri": "output", "dinout": "inout"}[kind]
    ports = [f"{d} {p}", f"{d} {n}"]
    if kind == "in":
        return ports, [], [p, n]
    if kind == "out":
        return ports, [f"assign {p} = dn; assign {n} = dn;"], []
    if kind == "tri":
        return ports, [f"OBUFT t{k}p(.O({p}), .I(dn), .T(en)); OBUFT t{k}n(.O({n}), .I(dn), .T(en));"], []
    if kind == "inout":
        return ports, [f"wire x{k}p, x{k}n; IOBUF u{k}p(.IO({p}), .I(dn), .T(en), .O(x{k}p)); "
                       f"IOBUF u{k}n(.IO({n}), .I(dn), .T(en), .O(x{k}n));"], [f"x{k}p", f"x{k}n"]
    if kind == "din":
        return ports, [f"wire x{k}; IBUFDS u{k}(.I({p}), .IB({n}), .O(x{k}));"], [f"x{k}"]
    if kind == "dout":
        return ports, [f"OBUFDS u{k}(.I(dn), .O({p}), .OB({n}));"], []
    if kind == "dtri":
        return ports, [f"OBUFTDS u{k}(.I(dn), .T(en), .O({p}), .OB({n}));"], []
    return ports, [f"wire x{k}; IOBUFDS u{k}(.I(dn), .T(en), .O(x{k}), .IO({p}), .IOB({n}));"], [f"x{k}"]


index = []


def sweep(name, std, kind, combos):
    ports, body, reads, xdc = [], [], [], []
    assert len(combos) <= len(PAIRS), name
    for k, c in enumerate(combos):
        a, b, r = buffers(kind, k)
        ports += a
        body += b
        reads += r
        for ball, suffix in zip(PAIRS[k], "pn"):
            xdc.append(f"set_property -dict {props(ball, std, c)} [get_ports a{k}{suffix}]")
            index.append(f"{name} a{k}{suffix} {ball} {std} {kind} {tag(c)}")
    write(name, ports, body, reads, xdc)


def out_settings(std):
    if std in DRIVES:
        drives = [{"DRIVE": d} for d in DRIVES[std]]
        out = [{}] + drives + [{"SLEW": "FAST"}] + [{"DRIVE": d, "SLEW": "FAST"} for d in DRIVES[std]]
    else:
        out = [{}, {"SLEW": "FAST"}]
    return out + [{"SLEW": "SLOW"}] + PULLS


def in_settings(std):
    terms = TERMS + [{"IN_TERM": "UNTUNED_SPLIT_50", "IBUF_LOW_PWR": "FALSE"}] if std in TERMED else []
    return ([{}] + PULLS + [{"IBUF_LOW_PWR": "FALSE"}, {"IBUF_LOW_PWR": "TRUE"}, {"IN_TERM": "NONE"},
            {"PULLTYPE": "PULLUP", "IBUF_LOW_PWR": "FALSE"}] + terms)


def inout_settings(std):
    terms = TERMS if std in TERMED else []
    drives = [{"DRIVE": d} for d in DRIVES.get(std, ())]
    return [{}] + PULLS + [{"IBUF_LOW_PWR": "FALSE"}] + terms + [{"SLEW": "FAST"}] + drives


def dout_settings(_):
    return [{}, {"SLEW": "FAST"}, {"SLEW": "SLOW"}] + PULLS


def din_settings(std):
    terms = TERMS + [{"IN_TERM": "UNTUNED_SPLIT_50", "IBUF_LOW_PWR": "FALSE"}] if std in TERMED else []
    return ([{}, {"DIFF_TERM": "TRUE"}, {"IBUF_LOW_PWR": "FALSE"}, {"DIFF_TERM": "TRUE", "IBUF_LOW_PWR": "FALSE"},
             {"DIFF_TERM": "FALSE"}] + PULLS + terms)


def dinout_settings(std):
    terms = [{"IN_TERM": "UNTUNED_SPLIT_50"}] if std in TERMED else []
    return [{}, {"DIFF_TERM": "TRUE"}, {"IBUF_LOW_PWR": "FALSE"}, {"SLEW": "FAST"}] + PULLS + terms


def se(ball, std, kind, **p):
    return {"p": ball, "n": None, "std": std, "kind": kind, "props": p}


def df(p, n, std, kind, inst=None, ponly=None, **props_):
    return {"p": p, "n": n, "std": std, "kind": kind, "props": props_, "inst": inst or {}, "ponly": ponly or {}}


def case(name, pads, vref=None, bank="35", helpers=("L4", "L5", "M5")):
    ports, body, reads, xdc = [], [], [], []
    for k, pad in enumerate(pads):
        a = f"a{k}"
        d = {"in": "input", "out": "output", "io": "inout", "unread": "input"}[pad["kind"]]
        if pad["n"] is None:
            ports.append(f"{d} {a}")
            xdc.append(f"set_property -dict {props(pad['p'], pad['std'], pad['props'])} [get_ports {a}]")
            if pad["kind"] == "in":
                reads.append(a)
            elif pad["kind"] == "out":
                body.append(f"assign {a} = dn;")
            elif pad["kind"] == "io":
                body.append(f"wire x{k}; IOBUF u{k}(.IO({a}), .I(dn), .T(en), .O(x{k}));")
                reads.append(f"x{k}")
            continue
        ports += [f"{d} {a}p", f"{d} {a}n"]
        par = ",".join(f".{x}({y})" for x, y in pad["inst"].items())
        par = f" #({par})" if par else ""
        if pad["kind"] == "in":
            body.append(f"wire x{k}; IBUFDS{par} u{k}(.I({a}p), .IB({a}n), .O(x{k}));")
            reads.append(f"x{k}")
        elif pad["kind"] == "unread":
            body.append(f"IBUFDS{par} u{k}(.I({a}p), .IB({a}n), .O());")
        elif pad["kind"] == "out":
            body.append(f"OBUFDS{par} u{k}(.I(dn), .O({a}p), .OB({a}n));")
        else:
            body.append(f"wire x{k}; IOBUFDS{par} u{k}(.I(dn), .T(en), .O(x{k}), .IO({a}p), .IOB({a}n));")
            reads.append(f"x{k}")
        s1, s2 = pad["std"] if isinstance(pad["std"], tuple) else (pad["std"], pad["std"])
        xdc.append(f"set_property -dict {props(pad['p'], s1, pad['props'])} [get_ports {a}p]")
        xdc.append(f"set_property -dict {props(pad['n'], s2, pad['props'])} [get_ports {a}n]")
        for key, value in pad["ponly"].items():
            port, key = (f"{a}n", key[2:]) if key.startswith("N:") else (f"{a}p", key)
            xdc.append(f"set_property {key} {value} [get_ports {port}]")
    if vref is not None:
        xdc.append(f"set_property INTERNAL_VREF {vref} [get_iobanks {bank}]")
    write(name, ports, body, reads, xdc, helpers)


def cases():
    g, h = ("G2", "G1"), ("H2", "H1")
    # An LVDS bank's clock row.
    case("lvds_in_alone", [df(*g, "LVDS_25", "in")])
    case("lvds_in_term", [df(*g, "LVDS_25", "in", DIFF_TERM="TRUE")])
    case("lvds_in_lvcmos25_in", [df(*g, "LVDS_25", "in"), se("H2", "LVCMOS25", "in")])
    case("lvds_in_lvcmos25_out", [df(*g, "LVDS_25", "in"), se("H2", "LVCMOS25", "out")])
    case("lvds_in_lvds_out", [df(*g, "LVDS_25", "in"), df(*h, "LVDS_25", "out")])
    case("lvds_in_lvcmos33_in", [df(*g, "LVDS_25", "in"), se("H2", "LVCMOS33", "in")])
    case("lvds_in_lvcmos33_out", [df(*g, "LVDS_25", "in"), se("H2", "LVCMOS33", "out")])
    case("lvds_term_lvcmos33_in", [df(*g, "LVDS_25", "in", DIFF_TERM="TRUE"), se("H2", "LVCMOS33", "in")])
    case("lvds_term_lvcmos33_out", [df(*g, "LVDS_25", "in", DIFF_TERM="TRUE"), se("H2", "LVCMOS33", "out")])
    case("lvds_out_alone", [df(*g, "LVDS_25", "out")])
    case("lvds_out_lvcmos25_in", [df(*g, "LVDS_25", "out"), se("H2", "LVCMOS25", "in")])
    case("lvds_out_lvcmos33_in", [df(*g, "LVDS_25", "out"), se("H2", "LVCMOS33", "in")])
    case("lvds_out_lvcmos33_out", [df(*g, "LVDS_25", "out"), se("H2", "LVCMOS33", "out")])
    case("tmds_in_alone", [df(*g, "TMDS_33", "in")])
    case("tmds_out_alone", [df(*g, "TMDS_33", "out")])
    case("tmds_in_lvcmos33_out", [df(*g, "TMDS_33", "in"), se("H2", "LVCMOS33", "out")])
    case("tmds_in_lvds_in", [df(*g, "TMDS_33", "in"), df(*h, "LVDS_25", "in")])
    case("rsds_in_lvds_in", [df(*g, "RSDS_25", "in"), df(*h, "LVDS_25", "in")])
    case("mix_tmds_lvcmos33", [df(*g, "TMDS_33", "out"), se("H2", "LVCMOS33", "out")])
    case("mix_lvdsterm_tmds", [df(*g, "LVDS_25", "in", DIFF_TERM="TRUE"), df(*h, "TMDS_33", "in")])
    # A bank's supply.
    for a, b in itertools.product(("in", "out"), repeat=2):
        case(f"v33{a}_25{b}", [se("G2", "LVCMOS33", a), se("H2", "LVCMOS25", b)])
    case("v33in_18in", [se("G2", "LVCMOS33", "in"), se("H2", "LVCMOS18", "in")])
    case("v18in_15in", [se("G2", "LVCMOS18", "in"), se("H2", "LVCMOS15", "in")])
    case("v18in_15out", [se("G2", "LVCMOS18", "in"), se("H2", "LVCMOS15", "out")])
    case("v18out_15in", [se("G2", "LVCMOS18", "out"), se("H2", "LVCMOS15", "in")])
    case("v15in_12in", [se("G2", "LVCMOS15", "in"), se("H2", "LVCMOS12", "in")])
    case("lvttl_lvcmos33", [se("G2", "LVTTL", "out"), se("H2", "LVCMOS33", "out")])
    case("pci_lvcmos33", [se("G2", "PCI33_3", "out"), se("H2", "LVCMOS33", "out")])
    # A bank's reference.
    case("sstl15_in", [se("G2", "SSTL15", "in")])
    case("sstl15_in_sstl135_in", [se("G2", "SSTL15", "in"), se("H2", "SSTL135", "in")])
    case("sstl15_in_hstl_i_in", [se("G2", "SSTL15", "in"), se("H2", "HSTL_I", "in")])
    case("sstl15_in_hstl18_in", [se("G2", "SSTL15", "in"), se("H2", "HSTL_I_18", "in")])
    case("sstl15_in_lvcmos15_out", [se("G2", "SSTL15", "in"), se("H2", "LVCMOS15", "out")])
    case("sstl15_in_lvcmos15_in", [se("G2", "SSTL15", "in"), se("H2", "LVCMOS15", "in")])
    case("sstl15_in_lvcmos33_out", [se("G2", "SSTL15", "in"), se("H2", "LVCMOS33", "out")])
    case("sstl15_in_lvcmos18_in", [se("G2", "SSTL15", "in"), se("H2", "LVCMOS18", "in")])
    case("sstl15_out_lvcmos33_in", [se("G2", "SSTL15", "out"), se("H2", "LVCMOS33", "in")])
    case("sstl15_out_lvcmos18_out", [se("G2", "SSTL15", "out"), se("H2", "LVCMOS18", "out")])
    case("sstl15_term_lvcmos33_out",
         [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_50"), se("H2", "LVCMOS33", "out")])
    case("sstl15_term_alone", [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_50")])
    case("sstl15_term_lvcmos18_in",
         [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_50"), se("H2", "LVCMOS18", "in")])
    # `INTERNAL_VREF`, and the ball it frees.
    case("vref_ball_refused", [se("G2", "SSTL15", "in"), se("J4", "LVCMOS15", "in")])
    case("vref_ball_refused_out", [se("G2", "SSTL15", "in"), se("J4", "LVCMOS15", "out")])
    case("vref_ball2_refused", [se("G2", "SSTL15", "in"), se("D5", "LVCMOS15", "in")])
    case("vref_ball_same_refused", [se("J4", "SSTL15", "in")])
    case("vref_ball_free", [se("G2", "SSTL15", "out"), se("J4", "LVCMOS15", "in")])
    case("vref_ball_lvcmos33", [se("J4", "LVCMOS33", "in"), se("D5", "LVCMOS33", "out")])
    for value in ("0.6", "0.675", "0.75", "0.9", "0.5", "1.2"):
        case("ivref_" + value.replace(".", "p"), [se("G2", "SSTL15", "in"), se("J4", "LVCMOS15", "in")], vref=value)
    case("ivref_same_ball", [se("J4", "SSTL15", "in")], vref="0.75")
    case("ivref_no_input", [se("G2", "SSTL15", "out")], vref="0.75")
    case("ivref_lvcmos", [se("G2", "LVCMOS15", "in")], vref="0.75")
    case("ivref_conflict", [se("G2", "SSTL15", "in"), se("H2", "SSTL135", "in")], vref="0.75")
    case("ivref_hsul", [se("G2", "HSUL_12", "in"), se("D5", "LVCMOS12", "out")], vref="0.6")
    # A differential standard has no reference of its own.
    case("diff_sstl15_in", [df(*g, "DIFF_SSTL15", "in")])
    case("diff_sstl15_in_sstl135_in", [df(*g, "DIFF_SSTL15", "in"), se("H2", "SSTL135", "in")])
    case("diff_sstl15_in_sstl15_in", [df(*g, "DIFF_SSTL15", "in"), se("H2", "SSTL15", "in")])
    # Two true differential output standards in a bank, every pair, and three.
    outs = ["LVDS_25", "RSDS_25", "MINI_LVDS_25", "PPDS_25"]
    for r in (2, 3, 4):
        for c in itertools.combinations(outs, r):
            name = "bias_" + "_".join(x.split("_")[0].lower() for x in c)
            case(name, [df(*PAIRS[2 + i], s, "out") for i, s in enumerate(c)])
    case("bias_lvds_io_mini_io", [df(*g, "LVDS_25", "io"), df(*h, "MINI_LVDS_25", "io")])
    case("bias_blvds_lvds", [df(*g, "BLVDS_25", "out"), df(*h, "LVDS_25", "out")])
    case("bias_blvds_rsds", [df(*g, "BLVDS_25", "out"), df(*h, "RSDS_25", "out")])
    case("bias_lvdsin_rsdsout", [df(*g, "LVDS_25", "in"), df(*h, "RSDS_25", "out")])
    case("bias_rsdsin_lvdsout", [df(*g, "RSDS_25", "in"), df(*h, "LVDS_25", "out")])
    # Which takes the second: the order of the ports, then of the sites.
    y = {"115": g, "109": h, "113": ("H5", "H4"), "119": ("F2", "E1"), "147": ("B7", "A7")}
    for name, pads in {
        "order_rsds115_lvds109": [("RSDS_25", "115"), ("LVDS_25", "109")],
        "order_lvds109_rsds115": [("LVDS_25", "109"), ("RSDS_25", "115")],
        "order_rsds109_lvds115": [("RSDS_25", "109"), ("LVDS_25", "115")],
        "order_ppds115_lvds109": [("PPDS_25", "115"), ("LVDS_25", "109")],
        "order_ppds115_mini109": [("PPDS_25", "115"), ("MINI_LVDS_25", "109")],
        "order_mini115_lvds109": [("MINI_LVDS_25", "115"), ("LVDS_25", "109")],
        "order_3a": [("LVDS_25", "115"), ("LVDS_25", "109"), ("RSDS_25", "113")],
        "order_3b": [("RSDS_25", "115"), ("RSDS_25", "109"), ("LVDS_25", "113")],
        "order_3c": [("RSDS_25", "119"), ("LVDS_25", "115"), ("RSDS_25", "109")],
        "order_3d": [("LVDS_25", "119"), ("RSDS_25", "115"), ("LVDS_25", "109")],
        "order_4": [("LVDS_25", "147"), ("RSDS_25", "119"), ("LVDS_25", "115"), ("RSDS_25", "109")],
    }.items():
        case(name, [df(*y[at], s, "out") for s, at in pads])
    # Other banks, the other side, and a `_SING` tile.
    left = ("P5", "P4", "P3")
    case("left_hstl", [se("A8", "HSTL_II", "out"), se("A9", "HSTL_II", "in")], helpers=left)
    case("left_hstl18", [se("A8", "HSTL_II_18", "out"), se("A9", "HSTL_II_18", "io")], helpers=left)
    case("left_pci", [se("A8", "PCI33_3", "out"), se("A9", "PCI33_3", "in")], helpers=left)
    case("left_rsds", [df("A8", "A9", "RSDS_25", "out"), df("B9", "A10", "RSDS_25", "in", DIFF_TERM="TRUE"),
                       df("B10", "B11", "TMDS_33", "in")], helpers=left)
    case("left_tmds", [df("A8", "A9", "TMDS_33", "in"), df("B9", "A10", "TMDS_33", "out")], helpers=left)
    case("left_lvds", [df("A8", "A9", "LVDS_25", "in", DIFF_TERM="TRUE"), df("B9", "A10", "LVDS_25", "out"),
                       se("B10", "LVCMOS25", "in")], helpers=left)
    case("sing_hstl", [se("E6", "HSTL_II", "out", SLEW="FAST"),
                       se("K5", "HSTL_II", "in", IN_TERM="UNTUNED_SPLIT_50", IBUF_LOW_PWR="FALSE")])
    case("sing_pci", [se("E6", "PCI33_3", "out"), se("K5", "PCI33_3", "in")])
    case("sing_hstl18", [se("E6", "HSTL_II_18", "out", SLEW="FAST"), se("K5", "SSTL18_II", "out", SLEW="FAST")])
    case("sing_lvcmos", [se("E6", "LVCMOS18", "out", PULLTYPE="KEEPER"), se("K5", "LVCMOS18", "in")])
    case("bank34_stepdown", [se("M2", "LVCMOS18", "out")], helpers=("N11", "N12", "N13"))
    case("bank14_stepdown", [se("N11", "LVCMOS18", "out")], helpers=("P5", "P4", "P3"))
    case("bank15_stepdown", [se("A8", "LVCMOS18", "out")], helpers=left)
    case("bank34_vref", [se("M2", "SSTL15", "in")], helpers=("N11", "N12", "N13"))
    case("bank34_lvds", [df("M2", "M1", "LVDS_25", "out")], helpers=("N11", "N12", "N13"))
    case("bank14_lvds", [df("N11", "N12", "LVDS_25", "out")], helpers=("P5", "P4", "P3"))
    # A buffer's own parameters, and a property on one ball.
    case("inst_term", [df(*g, "LVDS_25", "in", inst={"DIFF_TERM": '"TRUE"'})])
    case("inst_lowpwr", [df(*g, "LVDS_25", "in", inst={"IBUF_LOW_PWR": '"FALSE"'})])
    case("inst_std", [df(*g, None, "in", inst={"IOSTANDARD": '"LVDS_25"'})])
    case("inst_std_outranked", [df(*g, "RSDS_25", "in", inst={"IOSTANDARD": '"LVDS_25"', "DIFF_TERM": '"TRUE"'})])
    case("ponly_term", [df(*g, "LVDS_25", "in", ponly={"DIFF_TERM": "TRUE"})])
    case("nonly_term", [df(*g, "LVDS_25", "in", ponly={"N:DIFF_TERM": "TRUE"})])
    case("ponly_pull", [df(*g, "LVDS_25", "in", ponly={"PULLTYPE": "PULLUP"})])
    case("nonly_pull", [df(*g, "LVDS_25", "in", ponly={"N:PULLTYPE": "KEEPER"})])
    # An input nothing reads, and what is still asked of it.
    case("unread", [se("G2", "LVCMOS33", "unread")])
    case("unread_pull", [se("G2", "LVCMOS33", "unread", PULLTYPE="PULLUP")])
    case("unread_sstl", [se("G2", "SSTL15", "unread")])
    case("unread_sstl_term", [se("G2", "SSTL15", "unread", IN_TERM="UNTUNED_SPLIT_50", IBUF_LOW_PWR="FALSE")])
    case("unread_18", [se("G2", "LVCMOS18", "unread")])
    case("unread_pci", [se("G2", "PCI33_3", "unread")])
    case("unread_pair", [df(*g, "LVDS_25", "unread")])
    case("unread_pair_term", [df(*g, "LVDS_25", "unread", DIFF_TERM="TRUE", IBUF_LOW_PWR="FALSE")])
    # What does nothing, or nearly: a property where it means nothing.
    case("term_on_output", [se("G2", "SSTL15", "out", IN_TERM="UNTUNED_SPLIT_50")])
    case("diff_term_on_single", [se("G2", "LVCMOS25", "in", DIFF_TERM="TRUE")])
    case("low_power_on_output", [se("G2", "SSTL15", "out", IBUF_LOW_PWR="FALSE")])
    case("drive_on_input", [se("G2", "LVCMOS33", "in", DRIVE="8", SLEW="FAST")])
    # The same from a `.rcf`, which Vivado does not read.
    case("rcf_sstl15", [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_50", IBUF_LOW_PWR="FALSE", PULLTYPE="KEEPER"),
                        se("H2", "SSTL15", "out", SLEW="FAST"), se("J4", "SSTL15", "in")], vref="0.75")
    open(f"{OUT}/rcf_sstl15.rcf", "w").write(
        "set_io -io_standard SSTL15 -in_term UNTUNED_SPLIT_50 -ibuf_low_pwr no -keeper yes a0 G2\n"
        "set_io -io_standard SSTL15 -slew fast a1 H2\n"
        "set_io -io_standard SSTL15 a2 J4\n"
        "set_internal_vref 35 0.75\n"
        "set_io -io_standard LVCMOS33 dn L4\nset_io -io_standard LVCMOS33 en L5\n"
        "set_io -io_standard LVCMOS33 r M5\n")
    case("rcf_lvds", [df(*g, "LVDS_25", "in", DIFF_TERM="TRUE", IBUF_LOW_PWR="FALSE"), df(*h, "LVDS_25", "out")])
    open(f"{OUT}/rcf_lvds.rcf", "w").write(
        "set_io -io_standard LVDS_25 -diff_term yes -ibuf_low_pwr no a0p G2\n"
        "set_io -io_standard LVDS_25 -diff_term yes -ibuf_low_pwr no a0n G1\n"
        "set_io -io_standard LVDS_25 a1p H2\nset_io -io_standard LVDS_25 a1n H1\n"
        "set_io -io_standard LVCMOS33 dn L4\nset_io -io_standard LVCMOS33 en L5\n"
        "set_io -io_standard LVCMOS33 r M5\n")
    # What Vivado reports and carries on from, and Reticle refuses.
    case("bad_drive", [se("G2", "LVCMOS33", "out", DRIVE="24")])
    case("bad_term", [se("G2", "SSTL15", "in", IN_TERM="TUNED_SPLIT")])
    case("bad_slew", [se("G2", "LVCMOS33", "out", SLEW="MEDIUM")])
    case("bad_term_25", [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_25")])
    case("bad_term_75", [se("G2", "SSTL15", "in", IN_TERM="UNTUNED_SPLIT_75")])
    case("bad_term_lvcmos", [se("G2", "LVCMOS18", "in", IN_TERM="UNTUNED_SPLIT_50")])
    case("bad_type_pair_single", [df(*g, "LVCMOS33", "in")])
    case("bad_type_single_pair", [se("G2", "LVDS_25", "in")])
    case("bad_bidir", [se("G2", "SSTL18_I", "io")])
    case("bad_bidir_pair", [df(*g, "RSDS_25", "io")])
    case("no_standard", [se("G2", None, "in")])
    # Vivado takes these, and Reticle does not: a pair that is not, and two
    # standards on a pair.
    case("not_a_pair", [df("G2", "H1", "LVDS_25", "in")])
    case("pair_mixed", [df(*g, ("LVDS_25", "RSDS_25"), "in")])
    case("pair_backwards", [df("G1", "G2", "LVDS_25", "in")])


def buffer_case(name, primitive, std, pads, connections, params=None, ports=(), pull=None):
    """One buffer by name, on the ball pair G2/G1 of bank 35. `pads` is its pad
    ports with their direction; `connections` the others, from the helper signals
    `s0` `s1` `s2`, or read into the output `r`."""
    par = ",".join(f".{k}({v})" for k, v in (params or {}).items())
    par = f" #({par})" if par else ""
    decl, xdc = [], []
    balls = ("G2", "G1")
    conns = []
    for (port, direction), ball in zip(pads, balls):
        decl.append(f"{direction} {port.lower()}_pad")
        conns.append(f".{port}({port.lower()}_pad)")
        xdc.append(f"set_property -dict {props(ball, std, {})} [get_ports {port.lower()}_pad]")
    reads = []
    for port, value in connections.items():
        if value == "read":
            reads.append(f"x_{port.lower()}")
            conns.append(f".{port}(x_{port.lower()})")
        else:
            conns.append(f".{port}({value})")
    body = [f"wire {', '.join('x_' + p.lower() for p, v in connections.items() if v == 'read')};"] if reads else []
    body.append(f"{primitive}{par} dut({', '.join(conns)});")
    if pull:
        body.append(f"{pull} p(.O({pads[0][0].lower()}_pad));")
    expr = " ^ ".join(reads) or "1'b0"
    text = (f"module {name}(input s0, input s1, input s2, output r{''.join(', ' + d for d in decl)});\n"
            + "".join(f"  {b}\n" for b in body) + f"  assign r = {expr};\nendmodule\n")
    open(f"{OUT}/{name}.v", "w").write(text)
    for port, ball in zip(("s0", "s1", "s2", "r"), ("L4", "L5", "M5", "M4")):
        xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {port}]")
    open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")


def pad_buffers():
    i = [("I", "input")]
    pair_in = [("I", "input"), ("IB", "input")]
    io = [("IO", "inout")]
    pair_io = [("IO", "inout"), ("IOB", "inout")]
    routed = {"IBUFDISABLE": "s0", "O": "read"}
    both = {"IBUFDISABLE": "s0", "INTERMDISABLE": "s1", "O": "read"}
    for std in ("LVCMOS33", "SSTL15"):
        low = std.lower()
        buffer_case(f"buf_ibufdisable_{low}", "IBUF_IBUFDISABLE", std, i, routed)
        buffer_case(f"buf_ibufdisable_off_{low}", "IBUF_IBUFDISABLE", std, i, routed,
                    {"USE_IBUFDISABLE": '"FALSE"'})
        buffer_case(f"buf_ibufdisable_tied0_{low}", "IBUF_IBUFDISABLE", std, i, {"IBUFDISABLE": "1'b0", "O": "read"})
        buffer_case(f"buf_ibufdisable_tied1_{low}", "IBUF_IBUFDISABLE", std, i, {"IBUFDISABLE": "1'b1", "O": "read"})
        buffer_case(f"buf_intermdisable_{low}", "IBUF_INTERMDISABLE", std, i, both)
        buffer_case(f"buf_intermdisable_off_{low}", "IBUF_INTERMDISABLE", std, i, both,
                    {"USE_IBUFDISABLE": '"FALSE"'})
        buffer_case(f"buf_intermdisable_tied_{low}", "IBUF_INTERMDISABLE", std, i,
                    {"IBUFDISABLE": "s0", "INTERMDISABLE": "1'b0", "O": "read"})
    for std in ("LVCMOS33", "SSTL15", "LVCMOS18"):
        low = std.lower()
        full = {"I": "s0", "T": "s1", "IBUFDISABLE": "s2", "INTERMDISABLE": "s2", "O": "read"}
        buffer_case(f"buf_iobuf_intermdisable_{low}", "IOBUF_INTERMDISABLE", std, io, full)
        buffer_case(f"buf_iobuf_intermdisable_off_{low}", "IOBUF_INTERMDISABLE", std, io, full,
                    {"USE_IBUFDISABLE": '"FALSE"'})
    for std in ("LVDS_25", "DIFF_SSTL15", "BLVDS_25", "TMDS_33", "RSDS_25"):
        low = std.lower()
        buffer_case(f"buf_ibufds_ibufdisable_{low}", "IBUFDS_IBUFDISABLE", std, pair_in, routed)
        buffer_case(f"buf_ibufds_intermdisable_{low}", "IBUFDS_INTERMDISABLE", std, pair_in, both)
        buffer_case(f"buf_ibufgds_{low}", "IBUFGDS", std, pair_in, {"O": "read"})
        for prim in ("IBUFDS_DIFF_OUT", "IBUFGDS_DIFF_OUT"):
            buffer_case(f"buf_{prim.lower()}_{low}", prim, std, pair_in, {"O": "read", "OB": "read"})
        buffer_case(f"buf_ibufds_diff_out_o_{low}", "IBUFDS_DIFF_OUT", std, pair_in, {"O": "read"})
        buffer_case(f"buf_ibufds_diff_out_ob_{low}", "IBUFDS_DIFF_OUT", std, pair_in, {"OB": "read"})
        buffer_case(f"buf_ibufds_diff_out_ibufdisable_{low}", "IBUFDS_DIFF_OUT_IBUFDISABLE", std, pair_in,
                    {"IBUFDISABLE": "s0", "O": "read", "OB": "read"})
        buffer_case(f"buf_ibufds_diff_out_intermdisable_{low}", "IBUFDS_DIFF_OUT_INTERMDISABLE", std, pair_in,
                    {"IBUFDISABLE": "s0", "INTERMDISABLE": "s1", "O": "read", "OB": "read"})
    for std in ("LVDS_25", "DIFF_SSTL15", "BLVDS_25", "MINI_LVDS_25"):
        low = std.lower()
        full = {"I": "s0", "T": "s1", "IBUFDISABLE": "s2", "INTERMDISABLE": "s2", "O": "read"}
        buffer_case(f"buf_iobufds_intermdisable_{low}", "IOBUFDS_INTERMDISABLE", std, pair_io, full)
        buffer_case(f"buf_iobufds_diff_out_{low}", "IOBUFDS_DIFF_OUT", std, pair_io,
                    {"I": "s0", "TM": "s1", "TS": "s2", "O": "read", "OB": "read"})
        buffer_case(f"buf_iobufds_diff_out_same_{low}", "IOBUFDS_DIFF_OUT", std, pair_io,
                    {"I": "s0", "TM": "s1", "TS": "s1", "O": "read", "OB": "read"})
        buffer_case(f"buf_iobufds_diff_out_intermdisable_{low}", "IOBUFDS_DIFF_OUT_INTERMDISABLE", std, pair_io,
                    {"I": "s0", "TM": "s1", "TS": "s1", "IBUFDISABLE": "s2", "INTERMDISABLE": "s2", "O": "read",
                     "OB": "read"})
    # A tristate's enable tied, which is not an enable at all.
    for t in ("1'b0", "1'b1"):
        n = t[-1]
        buffer_case(f"buf_obuft_t{n}", "OBUFT", "LVCMOS33", [("O", "output")], {"I": "s0", "T": t})
        buffer_case(f"buf_iobuf_t{n}", "IOBUF", "LVCMOS33", io, {"I": "s0", "T": t, "O": "read"})
        buffer_case(f"buf_obuftds_t{n}", "OBUFTDS", "LVDS_25", [("O", "output"), ("OB", "output")],
                    {"I": "s0", "T": t})
        buffer_case(f"buf_iobufds_t{n}", "IOBUFDS", "LVDS_25", pair_io, {"I": "s0", "T": t, "O": "read"})
    # A parameter of a buffer that is not its pair.
    buffer_case("buf_ibuf_params", "IBUF", None, i, {"O": "read"}, {"IOSTANDARD": '"SSTL15"', "IBUF_LOW_PWR": '"FALSE"'})
    buffer_case("buf_obuf_params", "OBUF", None, [("O", "output")], {"I": "s0"},
                {"IOSTANDARD": '"LVCMOS33"', "DRIVE": "8", "SLEW": '"FAST"'})
    buffer_case("buf_iobuf_params", "IOBUF", None, io, {"I": "s0", "T": "s1", "O": "read"},
                {"IOSTANDARD": '"LVCMOS18"', "DRIVE": "4", "SLEW": '"FAST"'})
    buffer_case("buf_obuft_params", "OBUFT", None, [("O", "output")], {"I": "s0", "T": "s1"},
                {"IOSTANDARD": '"LVCMOS25"', "DRIVE": "16"})
    # A pull on a pad with nothing else on it, in each direction.
    for prim in ("PULLUP", "PULLDOWN", "KEEPER"):
        low = prim.lower()
        for direction in ("input", "output", "inout"):
            name = f"buf_{low}_{direction}"
            text = (f"module {name}(input s0, input s1, input s2, output r, {direction} o_pad);\n"
                    f"  {prim} p(.O(o_pad));\n  assign r = 1'b0;\nendmodule\n")
            open(f"{OUT}/{name}.v", "w").write(text)
            xdc = [f"set_property -dict {{PACKAGE_PIN G2 IOSTANDARD LVCMOS33}} [get_ports o_pad]"]
            for port, ball in zip(("s0", "s1", "s2", "r"), ("L4", "L5", "M5", "M4")):
                xdc.append(f"set_property -dict {{PACKAGE_PIN {ball} IOSTANDARD LVCMOS33}} [get_ports {port}]")
            open(f"{OUT}/{name}.xdc", "w").write("\n".join(xdc) + "\n")
        buffer_case(f"buf_{low}_ibuf", "IBUF", "LVCMOS33", i, {"O": "read"}, pull=prim)
        buffer_case(f"buf_{low}_obuf", "OBUF", "LVCMOS33", [("O", "output")], {"I": "s0"}, pull=prim)
        buffer_case(f"buf_{low}_iobuf", "IOBUF", "LVCMOS33", io, {"I": "s0", "T": "s1", "O": "read"}, pull=prim)


def main():
    os.makedirs(OUT, exist_ok=True)
    pad_buffers()
    for std in SE:
        low = std.lower()
        sweep(f"o_{low}", std, "out", out_settings(std))
        sweep(f"t_{low}", std, "tri", out_settings(std))
        sweep(f"i_{low}", std, "in", in_settings(std))
        if std not in ONE_WAY:
            sweep(f"b_{low}", std, "inout", inout_settings(std))
    for std in DIFF:
        low = std.lower()
        sweep(f"di_{low}", std, "din", din_settings(std))
        sweep(f"do_{low}", std, "dout", dout_settings(std))
        sweep(f"dt_{low}", std, "dtri", dout_settings(std))
        if std not in ONE_WAY:
            sweep(f"db_{low}", std, "dinout", dinout_settings(std))
    cases()
    open(f"{OUT}/index.txt", "w").write("# design port ball standard kind setting\n" + "\n".join(index) + "\n")


main()
