#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/memarr/: `gen_memarr.py <dir>`.

Each asks Vivado how it arranges one inferred memory on a 7-series part: which
block RAMs or distributed RAMs, in which aspect, with what around them. Every
input comes from a shift register fed by one pin, so that nothing is constant,
and the word read is folded to one bit, so that every design needs three pins.
`FIXTURE=memarr lutram.sh <out>` runs them and keeps what came back
(`lutram_probe.tcl`, which also writes each block's parameters); only the
answers are committed, and these designs are written here.

The names say the shape: `sdp_<depth>x<width>` one write port and one read
port at another address, read on the clock; `sp_rf_`, `sp_wf_`, `sp_nc_` one
address both read and written, read first, write first and no change; `tdp_`
two such ports; `r2_` a second read port; `reg_` the read registered twice;
`en_` a read enable; `async_` a read on no clock; `rom_` initial contents and
no write; `init_` contents and a write; `asym_` written narrow and read wide;
`style_<kind>_` a `ram_style`.

A word folded to its parity says nothing of the order of its bits: two bits
swapped read the same parity. `bits_` designs, after the rest, fold the word
read under a mask that changes every cycle (`^(q & sr[w-1:0])`), so that a bit
read from the wrong place shows, for the arrangements whose bits travel on more
than one pin or block.
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

rng = random.Random(0x3E3A)


def abits(depth):
    return max(1, (depth - 1).bit_length())


class Taps:
    """Hands out bits of the shift register, lowest first."""

    def __init__(self):
        self.next = 0

    def take(self, width):
        lo = self.next
        self.next += width
        return f"sr[{lo + width - 1}:{lo}]" if width > 1 else f"sr[{lo}]"


def contents(width, depth):
    """Contents no two words alike, written as a loop so that a deep memory's
    are a few lines: the high bits of two multiplicative hashes of the
    address, cut to the word without truncating an assignment."""
    one, two = rng.getrandbits(64) | 1, rng.getrandbits(64) | 1
    return (f"  integer k;\n  reg [127:0] h;\n"
            f"  initial for (k = 0; k < {depth}; k = k + 1) begin\n"
            f"    h = {{k * 64'h{one:016x}, k * 64'h{two:016x}}};\n"
            f"    mem[k] = h[127:{128 - width}];\n  end\n")


def design(name, comment, width, depth, kind="sdp", regs=1, style=None, init=False,
           mask=False):
    a = abits(depth)
    t = Taps()
    attr = f'(* ram_style = "{style}" *) ' if style else ""
    body = f"  {attr}reg [{width - 1}:0] mem [0:{depth - 1}];\n"
    if init or kind == "rom":
        body += contents(width, depth)
    body += f"  reg [{width - 1}:0] q0;\n"
    last = "q0"
    if kind in ("sdp", "en", "async", "r2", "reg"):
        wa, d, we, ra = t.take(a), t.take(width), t.take(1), t.take(a)
        body += f"  always @(posedge clk) if ({we}) mem[{wa}] <= {d};\n"
        if kind == "en":
            re = t.take(1)
            body += f"  always @(posedge clk) if ({re}) q0 <= mem[{ra}];\n"
        elif kind == "async":
            body += f"  always @(posedge clk) q0 <= mem[{ra}];\n"
        else:
            body += f"  always @(posedge clk) q0 <= mem[{ra}];\n"
        if kind == "r2":
            rb = t.take(a)
            body += (f"  reg [{width - 1}:0] p0;\n"
                     f"  always @(posedge clk) p0 <= mem[{rb}];\n")
            last = "q0 ^ p0"
    elif kind == "rom":
        ra = t.take(a)
        body += f"  always @(posedge clk) q0 <= mem[{ra}];\n"
    elif kind in ("sp_rf", "sp_wf", "sp_nc"):
        ad, d, we = t.take(a), t.take(width), t.take(1)
        if kind == "sp_rf":
            body += (f"  always @(posedge clk) begin\n    if ({we}) mem[{ad}] <= {d};\n"
                     f"    q0 <= mem[{ad}];\n  end\n")
        elif kind == "sp_wf":
            body += (f"  always @(posedge clk) begin\n    if ({we}) begin\n"
                     f"      mem[{ad}] <= {d};\n      q0 <= {d};\n"
                     f"    end else q0 <= mem[{ad}];\n  end\n")
        else:
            body += (f"  always @(posedge clk) begin\n    if ({we}) mem[{ad}] <= {d};\n"
                     f"    else q0 <= mem[{ad}];\n  end\n")
    elif kind == "tdp":
        aa, da, wea = t.take(a), t.take(width), t.take(1)
        ab, db, web = t.take(a), t.take(width), t.take(1)
        body += (f"  reg [{width - 1}:0] p0;\n"
                 f"  always @(posedge clk) begin\n    if ({wea}) mem[{aa}] <= {da};\n"
                 f"    q0 <= mem[{aa}];\n  end\n"
                 f"  always @(posedge clk) begin\n    if ({web}) mem[{ab}] <= {db};\n"
                 f"    p0 <= mem[{ab}];\n  end\n")
        last = "q0 ^ p0"
    else:
        raise ValueError(kind)
    if kind == "async":
        # The read is on no clock: what it reads is registered once outside.
        body = body.replace(f"  always @(posedge clk) q0 <= mem[{ra}];\n",
                            f"  always @(*) q0 = mem[{ra}];\n")
    for r in range(1, regs):
        body += f"  reg [{width - 1}:0] q{r};\n  always @(posedge clk) q{r} <= q{r - 1};\n"
    if regs > 1:
        last = f"q{regs - 1}"
    top = t.take(1)
    n = t.next
    if mask:
        # Every data bit has a shift register bit of its own to mask it.
        assert n >= width, name
        last = f"({last}) & sr[{width - 1}:0]"
    body += f"  always @(posedge clk) so <= ^({last}) ^ {top};\n"
    return name, (
        f"// {comment}\n"
        f"module {name} (\n    input  wire clk,\n    input  wire si,\n    output reg  so\n);\n"
        f"  reg [{n - 1}:0] sr;\n  always @(posedge clk) sr <= {{sr[{n - 2}:0], si}};\n"
        + body + "endmodule\n")


def asym(name, comment, narrow, depth, ratio):
    """Written `narrow` bits a word at `depth` words, read `ratio` words at once,
    in the form of the vendor's template for a memory read wider than written."""
    a = abits(depth)
    r = (ratio - 1).bit_length()
    wide = narrow * ratio
    t = Taps()
    wa, d, we, ra = t.take(a), t.take(narrow), t.take(1), t.take(a - r)
    body = (f"  reg [{narrow - 1}:0] mem [0:{depth - 1}];\n"
            f"  always @(posedge clk) if ({we}) mem[{wa}] <= {d};\n"
            f"  reg [{wide - 1}:0] q0;\n  integer i;\n"
            f"  always @(posedge clk)\n    for (i = 0; i < {ratio}; i = i + 1)\n"
            f"      q0[i * {narrow} +: {narrow}] <= mem[{{{ra}, i[{r - 1}:0]}}];\n")
    top = t.take(1)
    n = t.next
    body += f"  always @(posedge clk) so <= ^(q0) ^ {top};\n"
    return name, (
        f"// {comment}\n"
        f"module {name} (\n    input  wire clk,\n    input  wire si,\n    output reg  so\n);\n"
        f"  reg [{n - 1}:0] sr;\n  always @(posedge clk) sr <= {{sr[{n - 2}:0], si}};\n"
        + body + "endmodule\n")


def designs():
    out = []
    sdp = [
        # Either side of distributed RAM's line: 1,024 bits as Vivado counts.
        (32, 31), (32, 32), (64, 15), (64, 16), (128, 7), (128, 8), (256, 4),
        # One block, and the widths around its parity.
        (512, 8), (512, 18), (512, 36), (512, 64), (512, 72),
        (1024, 1), (1024, 9), (1024, 16), (1024, 18), (1024, 20), (1024, 32),
        (1024, 36), (1024, 40), (1024, 72),
        (2048, 8), (2048, 9), (2048, 18), (2048, 24), (2048, 36),
        (4096, 4), (4096, 9), (4096, 16), (4096, 18), (4096, 32),
        (8192, 2), (8192, 4), (8192, 8), (8192, 9), (8192, 16),
        (16384, 1), (16384, 2), (16384, 4), (16384, 8), (16384, 9), (16384, 18),
        (32768, 1), (32768, 2), (32768, 8), (65536, 1),
        # Depths that are not a power of two.
        (1000, 18), (1536, 36), (3000, 12), (6000, 6), (12000, 3), (20000, 2),
    ]
    for depth, width in sdp:
        out.append(design(f"sdp_{depth}x{width}",
                          f"{depth} words of {width}: one write, one read on the clock.",
                          width, depth))
    for depth, width in [(1024, 18), (4096, 18), (16384, 4)]:
        for kind, what in [("sp_rf", "one address, read first"),
                           ("sp_wf", "one address, write first"),
                           ("sp_nc", "one address, no change"),
                           ("tdp", "two addresses each read and written"),
                           ("r2", "one write and two reads"),
                           ("en", "one write, one read with an enable")]:
            out.append(design(f"{kind}_{depth}x{width}", f"{depth} words of {width}: {what}.",
                              width, depth, kind=kind))
        out.append(design(f"reg_{depth}x{width}",
                          f"{depth} words of {width}: one write, the read registered twice.",
                          width, depth, regs=2))
    out.append(design("async_1024x8", "1024 words of 8 read on no clock.", 8, 1024, kind="async"))
    out.append(design("async_64x8", "64 words of 8 read on no clock.", 8, 64, kind="async"))
    for depth, width in [(512, 36), (1024, 18), (2048, 9), (4096, 8), (16384, 2), (1024, 16)]:
        out.append(design(f"rom_{depth}x{width}", f"{depth} words of {width}, initial contents, no write.",
                          width, depth, kind="rom"))
    out.append(design("init_1024x36", "1024 words of 36 with initial contents, written and read.",
                      36, 1024, init=True))
    out.append(design("init_4096x18", "4096 words of 18 with initial contents, written and read.",
                      18, 4096, init=True))
    for depth, width in [(512, 36), (512, 72), (1024, 40), (2048, 18), (16384, 9)]:
        out.append(design(f"init_{depth}x{width}",
                          f"{depth} words of {width} with initial contents, written and read.",
                          width, depth, init=True))
    out.append(design("rom_r2_1024x16", "1024 words of 16, initial contents, read twice.",
                      16, 1024, kind="r2", init=True))
    out.append(asym("asym_4096x8_r32", "4096 words of 8 written, read 32 bits at a time.", 8, 4096, 4))
    out.append(asym("asym_2048x9_r36", "2048 words of 9 written, read 36 bits at a time.", 9, 2048, 4))
    for style, depth, width in [("block", 64, 8), ("distributed", 1024, 8), ("registers", 128, 8),
                                ("block", 32, 32), ("distributed", 256, 16)]:
        out.append(design(f"style_{style}_{depth}x{width}",
                          f"{depth} words of {width}, ram_style \"{style}\".",
                          width, depth, style=style))
    # The order of a word's bits, where it travels on more than one pin or
    # block: simple dual port's halves, a RAMB18E1 beside RAMB36E1s, a
    # parity bit filled, nine columns, a port pair, an output register and
    # contents; last, so that nothing above draws other contents.
    for kind, depth, width, kw in [
        ("sdp", 512, 36, {}), ("sdp", 512, 72, {}), ("sdp", 1024, 40, {}),
        ("sdp", 2048, 24, {}), ("sdp", 4096, 16, {}), ("sdp", 16384, 9, {}),
        ("sdp", 16384, 18, {}), ("tdp", 1024, 18, {}), ("sp_wf", 1024, 18, {}),
        ("sdp", 1024, 18, {"regs": 2}), ("sdp", 512, 36, {"init": True}),
        ("sdp", 1024, 40, {"init": True}),
    ]:
        tag = kind + ("_reg" if kw.get("regs") else "") + ("_init" if kw.get("init") else "")
        out.append(design(f"bits_{tag}_{depth}x{width}",
                          f"{depth} words of {width} ({tag}), read under a mask.",
                          width, depth, kind=kind, mask=True, **kw))
    return out


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, text in designs():
        for ext, body in ((".v", text), (".xdc", XDC), (".rcf", RCF)):
            with open(os.path.join(out, name + ext), "w") as f:
                f.write(body)


main()
