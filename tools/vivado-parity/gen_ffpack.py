#!/usr/bin/env python3
"""Writes testdata/fpga/vivado/ffpack/: eight flip-flops held to one slice.

    gen_ffpack.py <dir>

Each design is a first rank of flip-flops placed by Vivado and a second rank of
eight held by `LOC` and `BEL` to one slice, `AFF`..`DFF` and `A5FF`..`D5FF`,
each reading one of the first rank's. A slice has four bypass inputs (`AX`..`DX`),
so at least four of the eight reach their flip-flop some other way, and what
Vivado writes for that way is the question: which table, which input, which
truth table, which mux. The designs differ in what else the slice holds and in
the flip-flops' control signals. `ffpack.sh` runs them; what the bitstreams
decode to is `vivado.txt` there.
"""

import os
import sys

# A SLICEL of a CLBLL_L tile, and a SLICEM of a CLBLM_R tile.
SLICEL = "SLICE_X4Y50"
SLICEM = "SLICE_X2Y50"
BELS = ["AFF", "A5FF", "BFF", "B5FF", "CFF", "C5FF", "DFF", "D5FF"]
LUTS = ["A6LUT", "B6LUT", "C6LUT", "D6LUT"]
INPUTS = ["L4", "L5", "M1", "M2", "M4", "M5", "N1", "N2"]
OUTPUTS = ["N3", "P1", "P3", "P4", "P5", "R1", "R2", "R3"]
EXTRA = ["A2", "A3", "A4", "A5"]
LUT_OUTPUTS = ["B1", "B2", "B4", "B5"]
CONTROL = {"en": "T2", "rst": "T3", "clk2": "N14"}

# name: (site, [(primitive, init) per BEL], control, tables)
#   control: "none" ties CE high and the reset low; "ce_sr" reads `en` and `rst`;
#   "mixed_ce" gives AFF the enable and the rest none; "two_clocks" puts A5FF on
#   clk2.
#   tables: None, or the width of a table on each of A6LUT..D6LUT, read by a
#   pad and not by the slice's flip-flops.
FDRE = ("FDRE", 0)
DESIGNS = {
    "free8": (SLICEL, [FDRE] * 8, "none", None),
    "free8m": (SLICEM, [FDRE] * 8, "none", None),
    "lut4x4": (SLICEL, [FDRE] * 8, "none", 4),
    "lut4x4m": (SLICEM, [FDRE] * 8, "none", 4),
    "lut2x4": (SLICEL, [FDRE] * 8, "none", 2),
    "lut5x4": (SLICEL, [FDRE] * 8, "none", 5),
    "ctrl8": (
        SLICEL,
        [
            ("FDSE", 1),
            ("FDRE", 0),
            ("FDRE", 1),
            ("FDSE", 0),
            ("FDRE", 0),
            ("FDRE", 1),
            ("FDSE", 0),
            ("FDSE", 1),
        ],
        "ce_sr",
        None,
    ),
    "async8": (
        SLICEL,
        [("FDCE", 0), ("FDPE", 1), ("FDCE", 1), ("FDPE", 0)] * 2,
        "ce_sr",
        None,
    ),
    "mixed_ce": (SLICEL, [FDRE] * 8, "mixed_ce", None),
    "two_clocks": (SLICEL, [FDRE] * 8, "two_clocks", None),
}

# A table's function of its inputs, distinct per lane so none is merged.
FUNCTIONS = {2: ["4'h6", "4'h8", "4'hE", "4'h9"],
             4: ["16'h6996", "16'h8000", "16'hFFFE", "16'h1234"],
             5: ["32'h96696996", "32'h80000000", "32'hFFFFFFFE", "32'h12345678"]}


def ff(prim, init, inst, q, d, clk, ce, rst):
    reset = {"FDRE": "R", "FDSE": "S", "FDCE": "CLR", "FDPE": "PRE"}[prim]
    return (
        f"  (* DONT_TOUCH = \"TRUE\" *) {prim} #(.INIT(1'b{init})) {inst} "
        f"(.C({clk}), .CE({ce}), .{reset}({rst}), .D({d}), .Q({q}));\n"
    )


def design(name, site, flops, control, tables):
    ports = ["input wire clk", "input wire [7:0] d", "output wire [7:0] q"]
    if control != "none":
        ports += ["input wire en", "input wire rst"]
    if control == "two_clocks":
        ports.append("input wire clk2")
    if tables:
        ports += ["input wire [3:0] e", "output wire [3:0] o"]
    v = f"// {name}: see tools/vivado-parity/gen_ffpack.py.\n"
    v += f"module {name} ({', '.join(ports)});\n"
    v += "  wire [7:0] s;\n"
    for i in range(8):
        v += ff("FDRE", 0, f"f{i}", f"s[{i}]", f"d[{i}]", "clk", "1'b1", "1'b0")
    for i, (prim, init) in enumerate(flops):
        clk = "clk2" if control == "two_clocks" and i == 1 else "clk"
        if control == "ce_sr":
            ce, rst = "en", "rst"
        elif control == "mixed_ce":
            ce, rst = ("en" if i == 0 else "1'b1"), "1'b0"
        else:
            ce, rst = "1'b1", "1'b0"
        v += ff(prim, init, f"r{i}", f"q[{i}]", f"s[{i}]", clk, ce, rst)
    if tables:
        ins = ", ".join(f".I{k}(t[{k}])" for k in range(tables))
        v += "  wire [3:0] t;\n"
        for k in range(4):
            v += ff("FDRE", 0, f"g{k}", f"t[{k}]", f"e[{k}]", "clk", "1'b1", "1'b0")
        if tables == 5:
            # A fifth input: the first rank's own flip-flop beside the others.
            v += "  wire t4;\n"
            v += ff("FDRE", 0, "g4", "t4", "e[0] ^ e[1]", "clk", "1'b1", "1'b0")
            ins = ", ".join(f".I{k}(t[{k}])" for k in range(4)) + ", .I4(t4)"
        for k in range(4):
            v += (
                f"  (* DONT_TOUCH = \"TRUE\" *) LUT{tables} #(.INIT({FUNCTIONS[tables][k]})) "
                f"l{k} ({ins}, .O(o[{k}]));\n"
            )
    v += "endmodule\n"
    x = "create_clock -period 4 -name clk [get_ports clk]\n"
    x += "set_property PACKAGE_PIN N11 [get_ports clk]\n"
    for i in range(8):
        x += f"set_property PACKAGE_PIN {INPUTS[i]} [get_ports {{d[{i}]}}]\n"
        x += f"set_property PACKAGE_PIN {OUTPUTS[i]} [get_ports {{q[{i}]}}]\n"
    if control != "none":
        x += f"set_property PACKAGE_PIN {CONTROL['en']} [get_ports en]\n"
        x += f"set_property PACKAGE_PIN {CONTROL['rst']} [get_ports rst]\n"
    if control == "two_clocks":
        x += "create_clock -period 4 -name clk2 [get_ports clk2]\n"
        x += f"set_property PACKAGE_PIN {CONTROL['clk2']} [get_ports clk2]\n"
    if tables:
        for k in range(4):
            x += f"set_property PACKAGE_PIN {EXTRA[k]} [get_ports {{e[{k}]}}]\n"
            x += f"set_property PACKAGE_PIN {LUT_OUTPUTS[k]} [get_ports {{o[{k}]}}]\n"
    x += "set_property IOSTANDARD LVCMOS33 [get_ports *]\n"
    x += "set_property IOB FALSE [get_cells -hier -filter {REF_NAME =~ FD*}]\n"
    for i, bel in enumerate(BELS):
        x += f"set_property BEL {bel} [get_cells r{i}]\n"
        x += f"set_property LOC {site} [get_cells r{i}]\n"
    if tables:
        for k, bel in enumerate(LUTS):
            x += f"set_property BEL {bel} [get_cells l{k}]\n"
            x += f"set_property LOC {site} [get_cells l{k}]\n"
    return v, x


# The designs both tools build: sixteen flip-flops held to the two slices of
# CLBLL_L_X4Y50, by `LOC` for Vivado and by a region of that tile (grid column
# 14, row 155) for Reticle, which cannot fix a cell to a site. A tile of sixteen
# is two slices of eight, so neither can be built without a way through a table
# for half of them. `lut2x16` adds a two-input table on each of the eight lanes,
# of inputs no other table reads, so none is paired into another's site. The
# first rank is of `FDSE`s, which no tool takes for the held `FDRE` reading the
# same signal: of `FDRE`s, Reticle merged each held one with the next of the
# chain. Reticle maps the tables again and names them `$lut<n>`, so its region
# takes them by that name; they are the only tables these designs have.
# name: whether it has the tables
BOTH = {"free16": False, "lut2x16": True}
TILE = (14, 155)
SLICES = ["SLICE_X4Y50", "SLICE_X5Y50"]
PINS16 = ["L4", "L5", "M1", "M2", "M4", "M5", "N1", "N2", "N3", "P1", "P3", "P4",
          "P5", "R1", "R2", "R3", "A2", "A3", "A4", "A5", "A7", "B1", "B2", "B4",
          "B5", "B6", "B7"]


def design16(name, tables):
    ports = ["input wire clk", "input wire si", "output wire [15:0] q"]
    if tables:
        ports += ["input wire si2", "output wire [7:0] o"]
    v = f"// {name}: see tools/vivado-parity/gen_ffpack.py.\n"
    v += f"module {name} ({', '.join(ports)});\n"
    v += "  wire [15:0] s;\n"
    for i in range(16):
        d = "si" if i == 0 else f"s[{i - 1}]"
        v += ff("FDSE", 1, f"f{i}", f"s[{i}]", d, "clk", "1'b1", "1'b0")
    for i in range(16):
        v += ff("FDRE", 0, f"r{i}", f"q[{i}]", f"s[{i}]", "clk", "1'b1", "1'b0")
    if tables:
        v += "  wire [15:0] t;\n"
        for i in range(16):
            d = "si2" if i == 0 else f"t[{i - 1}]"
            v += ff("FDSE", 1, f"g{i}", f"t[{i}]", d, "clk", "1'b1", "1'b0")
        for k in range(8):
            v += (
                f"  (* DONT_TOUCH = \"TRUE\" *) LUT2 #(.INIT({FUNCTIONS[2][k % 4]})) "
                f"l{k} (.I0(t[{2 * k}]), .I1(t[{2 * k + 1}]), .O(o[{k}]));\n"
            )
    v += "endmodule\n"
    pins = list(PINS16)
    ports = ["si"] + [f"q[{i}]" for i in range(16)]
    if tables:
        ports += ["si2"] + [f"o[{k}]" for k in range(8)]
    x = "create_clock -period 4 -name clk [get_ports clk]\n"
    x += "set_property PACKAGE_PIN N11 [get_ports clk]\n"
    rcf = "create_clock -name clk -period 4 clk\n"
    rcf += "set_io -io_standard LVCMOS33 clk N11\n"
    for port, pin in zip(ports, pins):
        x += f"set_property PACKAGE_PIN {pin} [get_ports {{{port}}}]\n"
        rcf += f"set_io -io_standard LVCMOS33 {port} {pin}\n"
    x += "set_property IOSTANDARD LVCMOS33 [get_ports *]\n"
    x += "set_property IOB FALSE [get_cells -hier -filter {REF_NAME =~ FD*}]\n"
    for i in range(16):
        x += f"set_property LOC {SLICES[i // 8]} [get_cells r{i}]\n"
    if tables:
        for k in range(8):
            x += f"set_property LOC {SLICES[k // 4]} [get_cells l{k}]\n"
    rcf += f"region slices {TILE[0]} {TILE[1]} {TILE[0]} {TILE[1]}\n"
    rcf += "assign r* region slices\n"
    if tables:
        rcf += "assign ?lut* region slices\n"
    return v, x, rcf


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, (site, flops, control, tables) in DESIGNS.items():
        v, x = design(name, site, flops, control, tables)
        with open(os.path.join(out, f"{name}.v"), "w") as f:
            f.write(v)
        with open(os.path.join(out, f"{name}.xdc"), "w") as f:
            f.write(x)
    for name, tables in BOTH.items():
        v, x, rcf = design16(name, tables)
        for ext, text in (("v", v), ("xdc", x), ("rcf", rcf)):
            with open(os.path.join(out, f"{name}.{ext}"), "w") as f:
                f.write(text)


if __name__ == "__main__":
    main()
