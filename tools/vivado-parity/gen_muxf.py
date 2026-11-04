#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/muxf/: `gen_muxf.py <dir>`.

Each asks Vivado one question about a slice's wide multiplexers, with a clock
and two pins, and the shift register that gives every input a flip-flop of its
own so that nothing here is folded away. `muxf.sh` then runs them and keeps
what came back. Only the answers are committed; these designs are written here.
"""
import math
import os
import sys

TABLES = {
    "la": "64'h6996966996696996",
    "lb": "64'hFE01FE01FE01FE00",
    "lc": "64'h8001800180018000",
    "ld": "64'hA5A5C3C3F00FF00E",
}


def table(name, wire, first):
    """A LUT6 by name reading six bits of the shift register from `first`."""
    pins = ", ".join(f".I{i}(sr[{first + i}])" for i in range(6))
    return f"  LUT6 #(.INIT({TABLES[name]})) {name} (.O({wire}), {pins});\n"


def tables(count):
    """`count` tables on wires a, b, c, d, six bits of register each."""
    out = ""
    for i, (name, wire) in enumerate(zip(TABLES, "abcd")):
        if i < count:
            out += table(name, wire, 6 * i)
    return out


def design(name, comment, top_bit, body):
    return (
        f"// {comment}\n"
        f"module {name} (\n"
        "    input  wire clk,\n"
        "    input  wire si,\n"
        "    output reg  so\n"
        ");\n"
        f"  reg [{top_bit}:0] sr;\n"
        f"  always @(posedge clk) sr <= {{sr[{top_bit - 1}:0], si}};\n"
        f"{body}"
        "endmodule\n"
    )


def f7(name, wire, i0, i1, s, cell="MUXF7"):
    return f"  {cell} {name} (.O({wire}), .I0({i0}), .I1({i1}), .S({s}));\n"


DESIGNS = {
    "f7_ff": design(
        "f7_ff",
        "A MUXF7 by name between two tables by name, into a flip-flop.",
        12,
        "  wire a, b, o;\n"
        + tables(2)
        + f7("dut", "o", "a", "b", "sr[12]")
        + "  always @(posedge clk) so <= o;\n",
    ),
    "f7_out": design(
        "f7_out",
        "A MUXF7 whose output goes on to logic, so that it must leave the slice.",
        12,
        "  wire a, b, o;\n"
        + tables(2)
        + f7("dut", "o", "a", "b", "sr[12]")
        + "  reg q;\n"
        "  always @(posedge clk) q <= sr[0] & sr[12];\n"
        "  always @(posedge clk) so <= q ^ (o & sr[3]);\n",
    ),
    "f7_two": design(
        "f7_two",
        "Two MUXF7 with different selects, one registered and one read by logic.",
        25,
        "  wire a, b, c, d, ab, cd;\n"
        + tables(4)
        + f7("m0", "ab", "a", "b", "sr[24]")
        + f7("m1", "cd", "c", "d", "sr[25]")
        + "  reg q;\n"
        "  always @(posedge clk) q <= ab;\n"
        "  always @(posedge clk) so <= q ^ cd;\n",
    ),
    "f8_ff": design(
        "f8_ff",
        "Four tables, two MUXF7 and a MUXF8 by name, into a flip-flop.",
        25,
        "  wire a, b, c, d, ab, cd, o;\n"
        + tables(4)
        + f7("m0", "ab", "a", "b", "sr[24]")
        + f7("m1", "cd", "c", "d", "sr[24]")
        + f7("dut", "o", "ab", "cd", "sr[25]", "MUXF8")
        + "  always @(posedge clk) so <= o;\n",
    ),
    "f8_out": design(
        "f8_out",
        "The same, with the MUXF8 and a MUXF7 also read outside the slice.",
        26,
        "  wire a, b, c, d, ab, cd, o;\n"
        + tables(4)
        + f7("m0", "ab", "a", "b", "sr[24]")
        + f7("m1", "cd", "c", "d", "sr[24]")
        + f7("dut", "o", "ab", "cd", "sr[25]", "MUXF8")
        + "  reg q;\n"
        "  always @(posedge clk) q <= ab & sr[26];\n"
        "  always @(posedge clk) so <= q ^ (o & sr[3]);\n",
    ),
    "f7_half": design(
        "f7_half",
        "A MUXF7 by name with one table and one flip-flop for its data.",
        7,
        "  wire a, o;\n"
        + tables(1)
        + f7("dut", "o", "a", "sr[6]", "sr[7]")
        + "  always @(posedge clk) so <= o;\n",
    ),
    "f7_shared": design(
        "f7_shared",
        "One table read by two MUXF7: its output cannot be both slices' I0.",
        19,
        "  wire a, b, c, x, y;\n"
        + tables(3)
        + f7("m0", "x", "a", "b", "sr[18]")
        + f7("m1", "y", "a", "c", "sr[19]")
        + "  reg q;\n"
        "  always @(posedge clk) q <= x;\n"
        "  always @(posedge clk) so <= q ^ y;\n",
    ),
    "f7_fanout": design(
        "f7_fanout",
        "A table read by its MUXF7 and by logic elsewhere.",
        13,
        "  wire a, b, o;\n"
        + tables(2)
        + f7("dut", "o", "a", "b", "sr[12]")
        + "  reg q;\n"
        "  always @(posedge clk) q <= a & sr[13];\n"
        "  always @(posedge clk) so <= q ^ o;\n",
    ),
    "f8_half": design(
        "f8_half",
        "A MUXF8 by name with one MUXF7 and one flip-flop for its data.",
        14,
        "  wire a, b, ab, o;\n"
        + tables(2)
        + f7("m0", "ab", "a", "b", "sr[12]")
        + f7("dut", "o", "ab", "sr[13]", "sr[14]", "MUXF8")
        + "  always @(posedge clk) so <= o;\n",
    ),
    "f7_d": design(
        "f7_d",
        "A MUXF7_D, with both of its outputs read.",
        13,
        "  wire a, b, o, lo;\n"
        + tables(2)
        + "  MUXF7_D dut (.O(o), .LO(lo), .I0(a), .I1(b), .S(sr[12]));\n"
        + "  reg q;\n"
        "  always @(posedge clk) q <= lo;\n"
        "  always @(posedge clk) so <= q ^ (o & sr[13]);\n",
    ),
    "f7_carry": design(
        "f7_carry",
        "A table that is both a carry chain's propagate and a MUXF7's data.",
        20,
        "  wire [7:0] x = sr[7:0];\n"
        "  wire [7:0] y = sr[15:8];\n"
        "  reg [7:0] sum;\n"
        "  wire b, o;\n"
        "  LUT6 #(.INIT(64'hFE01FE01FE01FE00)) lb (.O(b), .I0(sr[14]), .I1(sr[15]), .I2(sr[16]), .I3(sr[17]), .I4(sr[18]), .I5(sr[19]));\n"
        "  always @(posedge clk) sum <= x + y;\n"
        + f7("dut", "o", "x[0] ^ y[0]", "b", "sr[20]")
        + "  always @(posedge clk) so <= o ^ (^sum);\n",
    ),
    "f7_pair": design(
        "f7_pair",
        "Two narrow data tables, and a third that shares inputs with one and must leave the slice.",
        12,
        "  wire a = sr[0] & sr[1] & sr[2];\n"
        "  wire b = sr[3] | sr[4] | sr[5];\n"
        "  wire c = sr[0] ^ sr[1] ^ sr[10] ^ sr[11];\n"
        "  wire o;\n"
        + f7("dut", "o", "a", "b", "sr[6]")
        + "  reg q, r;\n"
        "  always @(posedge clk) q <= c & sr[7] & sr[8] & sr[9];\n"
        "  always @(posedge clk) r <= c | sr[7] | sr[8] | sr[9];\n"
        "  always @(posedge clk) so <= q ^ r ^ (o & sr[12]);\n",
    ),
    "tie": design(
        "tie",
        "A choice between two full tables written as logic, where a MUXF7 saves no table: the top one has room for it.",
        13,
        "  wire a, b;\n"
        + tables(2)
        + "  wire o = sr[12] ? b : a;\n"
        "  reg q;\n"
        "  always @(posedge clk) q <= sr[0] & sr[12];\n"
        "  always @(posedge clk) so <= q ^ (o & sr[3]);\n",
    ),
    "f7_from_ff": design(
        "f7_from_ff",
        "A MUXF7 by name fed by flip-flops and not by tables.",
        2,
        "  wire o;\n"
        + f7("dut", "o", "sr[0]", "sr[1]", "sr[2]")
        + "  always @(posedge clk) so <= o;\n",
    ),
    "f8_from_ff": design(
        "f8_from_ff",
        "A MUXF8 by name fed by flip-flops and not by MUXF7s.",
        2,
        "  wire o;\n"
        + f7("dut", "o", "sr[0]", "sr[1]", "sr[2]", "MUXF8")
        + "  always @(posedge clk) so <= o;\n",
    ),
}

for ways in (8, 16, 32, 64):
    bits = int(math.log2(ways))
    top = ways + bits - 1
    DESIGNS[f"mux{ways}"] = design(
        f"mux{ways}",
        f"An inferred {ways}-to-1 multiplexer into a flip-flop.",
        top,
        f"  always @(posedge clk) so <= sr[sr[{top}:{ways}]];\n",
    )

XDC = """create_clock -period 10 -name clk [get_ports clk]
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN L4 [get_ports si]
set_property PACKAGE_PIN L5 [get_ports so]
set_property IOSTANDARD LVCMOS33 [get_ports {clk si so}]
"""
RCF = """create_clock -name clk -period 10 clk
set_io -io_standard LVCMOS33 clk N11
set_io -io_standard LVCMOS33 si L4
set_io -io_standard LVCMOS33 so L5
"""

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
for name, text in DESIGNS.items():
    with open(f"{out}/{name}.v", "w") as f:
        f.write(text)
    with open(f"{out}/{name}.xdc", "w") as f:
        f.write(XDC)
    with open(f"{out}/{name}.rcf", "w") as f:
        f.write(RCF)
