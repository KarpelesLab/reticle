#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/lutram/: `gen_lutram.py <dir>`.

Each asks Vivado one question about a slice's distributed RAM: one cell by name
with every input from a flip-flop of its own and random contents, or a small
memory to infer. `lutram.sh` runs them and keeps what came back. Only the
answers are committed; these designs are written here.

`gen_lutram.py --sweep <dir>` writes the block-or-distributed sweep instead,
into a scratch directory, with `names.txt`; `lutram_sweep.tcl` asks Vivado's
synthesis about each, and its answers are committed as sweep.txt.
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

rng = random.Random(0x7A100)


def init(bits):
    return f"{bits}'h{rng.getrandbits(bits):0{bits // 4}x}"


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


def bits(prefix, count, first):
    return ", ".join(f".{prefix}{i}(sr[{first + i}])" for i in range(count))


def vector(name, count, first):
    return f".{name}(sr[{first + count - 1}:{first}])"


def single(cell, abits, suffix="", extra="", vectors=False):
    """A `RAM<n>X1S`, by name."""
    name = f"{cell.lower()}{suffix}"
    at = vector("A", abits, 0) if vectors else bits("A", abits, 0)
    top = abits + 2
    return name, design(
        name,
        f"A {cell}{suffix.replace('_', ' ')} by name, read into a flip-flop.",
        top,
        "  wire o;\n"
        f"  {cell} #(.INIT({init(1 << abits)}){extra}) dut (.O(o), {at},\n"
        f"      .D(sr[{abits}]), .WE(sr[{abits + 1}]), .WCLK(clk));\n"
        f"  always @(posedge clk) so <= o ^ sr[{top}];\n",
    )


def dual(cell, abits, suffix="", extra="", vectors=False, read="both"):
    """A `RAM<n>X1D`, by name; `read` is which outputs are used."""
    name = f"{cell.lower()}{suffix}"
    if vectors:
        ports = f"{vector('A', abits, 0)}, {vector('DPRA', abits, abits)}"
    else:
        ports = bits("A", abits, 0) + ", " + bits("DPRA", abits, abits)
    n = 2 * abits
    outs = {"both": ".SPO(spo), .DPO(dpo)", "dpo": ".SPO(), .DPO(dpo)", "spo": ".SPO(spo), .DPO()"}[read]
    use = {"both": "a ^ b", "dpo": "b", "spo": "a"}[read]
    return name, design(
        name,
        f"A {cell}{suffix.replace('_', ' ')} by name, reading {read}.",
        n + 1,
        "  wire spo, dpo;\n"
        f"  {cell} #(.INIT({init(1 << abits)}){extra}) dut ({outs}, {ports},\n"
        f"      .D(sr[{n}]), .WE(sr[{n + 1}]), .WCLK(clk));\n"
        "  reg a, b;\n"
        "  always @(posedge clk) begin\n"
        "    a <= spo;\n"
        "    b <= dpo;\n"
        f"    so <= {use};\n"
        "  end\n",
    )


def multi(cell, abits, dbits, suffix="", extra="", reads="abcd"):
    """A `RAM32M` or `RAM64M`, by name, each port's address its own."""
    name = f"{cell.lower()}{suffix}"
    params = ", ".join(f".INIT_{p}({init(64)})" for p in "ABCD")
    conns, wires, used, at = [], [], [], 0
    for port in "ABCD":
        conns.append(vector(f"ADDR{port}", abits, at))
        at += abits
    for port in "ABCD":
        conns.append(vector(f"DI{port}", dbits, at) if dbits > 1 else f".DI{port}(sr[{at}])")
        at += dbits
    for port in "ABCD":
        wire = f"do{port.lower()}"
        wires.append(f"  wire [{dbits - 1}:0] {wire};\n")
        conns.append(f".DO{port}({wire})")
        if port.lower() in reads:
            used.append(f"^{wire}")
    conns += [f".WE(sr[{at}])", ".WCLK(clk)"]
    return name, design(
        name,
        f"A {cell}{suffix.replace('_', ' ')} by name, reading ports {reads}.",
        at,
        "".join(wires)
        + f"  {cell} #({params}{extra}) dut ({', '.join(conns)});\n"
        + f"  always @(posedge clk) so <= {' ^ '.join(used)};\n",
    )


def two_bits(cell, abits):
    """`RAM32X2S` and `RAM64X2S` by name."""
    name = cell.lower()
    n = abits
    return name, design(
        name,
        f"A {cell} by name, both outputs read.",
        n + 2,
        "  wire o0, o1;\n"
        f"  {cell} #(.INIT_00({init(1 << abits)}), .INIT_01({init(1 << abits)})) dut (.O0(o0), .O1(o1),\n"
        f"      {bits('A', abits, 0)}, .D0(sr[{n}]), .D1(sr[{n + 1}]), .WE(sr[{n + 2}]), .WCLK(clk));\n"
        "  always @(posedge clk) so <= o0 ^ o1;\n",
    )


def inferred(name, width, depth, reads=1, read="async", initial=False):
    """A memory to infer: one write port and `reads` read ports, each read
    registered after it (`async`), or read on the clock (`sync`); with
    `initial`, every word given a random first value."""
    abits = max(1, (depth - 1).bit_length())
    top = abits + width + reads * abits
    body = f"  reg [{width - 1}:0] mem [0:{depth - 1}];\n"
    if initial:
        body += "  initial begin\n"
        for word in range(depth):
            body += f"    mem[{word}] = {init(width)};\n"
        body += "  end\n"
    body += f"  always @(posedge clk) if (sr[{top}]) mem[sr[{abits - 1}:0]] <= sr[{abits + width - 1}:{abits}];\n"
    terms = []
    for r in range(reads):
        low = abits + width + r * abits
        ra = f"sr[{low + abits - 1}:{low}]"
        body += f"  reg [{width - 1}:0] q{r};\n"
        if read == "async":
            body += f"  always @(posedge clk) q{r} <= mem[{ra}];\n"
        else:
            body += f"  reg [{width - 1}:0] p{r};\n"
            body += f"  always @(posedge clk) begin p{r} <= mem[{ra}]; q{r} <= p{r}; end\n"
        terms.append(f"^q{r}")
    body += f"  always @(posedge clk) so <= {' ^ '.join(terms)};\n"
    comment = f"{depth} words of {width}, {reads} read(s), {read}: inferred."
    if initial:
        comment = comment[:-1] + ", with initial contents."
    return name, design(name, comment, top, body)


def designs():
    inv = ", .IS_WCLK_INVERTED(1'b1)"
    out = [
        single("RAM16X1S", 4),
        single("RAM32X1S", 5),
        single("RAM64X1S", 6),
        single("RAM128X1S", 7),
        single("RAM256X1S", 8, vectors=True),
        dual("RAM16X1D", 4),
        dual("RAM32X1D", 5),
        dual("RAM64X1D", 6),
        dual("RAM128X1D", 7, vectors=True),
        dual("RAM32X1D", 5, "_dpo", read="dpo"),
        dual("RAM64X1D", 6, "_dpo", read="dpo"),
        dual("RAM64X1D", 6, "_spo", read="spo"),
        multi("RAM32M", 5, 2),
        multi("RAM64M", 6, 1),
        multi("RAM32M", 5, 2, "_dc", reads="dc"),
        multi("RAM64M", 6, 1, "_d", reads="d"),
        two_bits("RAM32X2S", 5),
        two_bits("RAM64X2S", 6),
        single("RAM32X1S", 5, "_wclkinv", inv),
        single("RAM64X1S", 6, "_wclkinv", inv),
        dual("RAM64X1D", 6, "_wclkinv", inv),
        multi("RAM32M", 5, 2, "_wclkinv", inv),
    ]
    for width, depth, reads in [
        (1, 16, 1), (8, 16, 1),
        (1, 32, 1), (2, 32, 1), (6, 32, 1), (7, 32, 1), (8, 32, 1), (32, 32, 1),
        (1, 64, 1), (3, 64, 1), (8, 64, 1),
        (1, 128, 1), (2, 128, 1),
        (1, 256, 1), (2, 256, 1),
        (8, 32, 2), (8, 32, 3), (8, 32, 4), (32, 32, 2),
        (8, 64, 2), (8, 64, 3),
        (4, 128, 2),
    ]:
        name = f"mem{depth}x{width}" + (f"r{reads}" if reads > 1 else "")
        out.append(inferred(name, width, depth, reads))
    out.append(inferred("mem32x8sync", 8, 32, read="sync"))
    # Initial contents, which inference must carry into every shape's tables.
    for width, depth in [(2, 32), (6, 32), (3, 64), (1, 128), (1, 256)]:
        out.append(inferred(f"mem{depth}x{width}init", width, depth, initial=True))
    return out


def sweep(out):
    """Depth by width by kind of read, for synthesis alone."""
    os.makedirs(out, exist_ok=True)
    names = []
    for read in ("async", "sync"):
        for abits in range(1, 15):
            depth = 1 << abits
            for width in (1, 2, 4, 8, 9, 16, 18, 32, 36, 64, 72):
                if depth * width > 1 << 19:
                    continue
                name = f"s_{read}_{depth}_{width}"
                top = abits * 2 + width
                ra = f"sr[{abits + width + abits - 1}:{abits + width}]"
                if read == "async":
                    rd = f"assign o = mem[{ra}];"
                else:
                    rd = f"always @(posedge clk) p <= mem[{ra}]; assign o = p;"
                text = (
                    f"module {name} (input wire clk, input wire si, output reg so);\n"
                    f"  reg [{top}:0] sr;\n"
                    f"  always @(posedge clk) sr <= {{sr[{top - 1}:0], si}};\n"
                    f"  reg [{width - 1}:0] mem [0:{depth - 1}];\n"
                    f"  reg [{width - 1}:0] p; wire [{width - 1}:0] o;\n"
                    f"  always @(posedge clk) if (sr[{top}]) mem[sr[{abits - 1}:0]] <= sr[{abits + width - 1}:{abits}];\n"
                    f"  {rd}\n"
                    "  always @(posedge clk) so <= ^o;\n"
                    "endmodule\n"
                )
                with open(os.path.join(out, name + ".v"), "w") as f:
                    f.write(text)
                names.append(name)
    with open(os.path.join(out, "names.txt"), "w") as f:
        f.write("\n".join(names) + "\n")


def main():
    if sys.argv[1] == "--sweep":
        sweep(sys.argv[2])
        return
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, text in designs():
        for ext, body in ((".v", text), (".xdc", XDC), (".rcf", RCF)):
            with open(os.path.join(out, name + ext), "w") as f:
                f.write(body)


main()
