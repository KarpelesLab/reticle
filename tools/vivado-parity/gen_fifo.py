#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/fifo/: `gen_fifo.py <dir>`.

Each asks Vivado about one `FIFO18E1` or `FIFO36E1` by name, in one setting:
its data in from a shift register, its writes and reads enabled at random once
a reset at power-up is over, and every output folded into `so`. The reset is
what the vendor's model asks for (five cycles high, no enable during it, and
time after), so that a simulation of either build answers. `FIXTURE=fifo
lutram.sh <out>` runs them and keeps what came back. Only answers are
committed; these designs are written here.

The first seventeen are the ones `docs/fpga-vivado-parity.md` first read; the
designs after them are added at the end so that the random values of those
seventeen stay what they were. A design whose name starts `fx_` is one Vivado
is expected to refuse, for the rule it gives.
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

rng = random.Random(0xF1F0)
ZERO = "1'b0"
ONE = "1'b1"


def hexv(bits):
    return f"{bits}'h{rng.getrandbits(bits):0{(bits + 3) // 4}x}"


def fifo(name, comment, cell, width, params, inverted=(), driven=(), loc=None, ecc=False):
    """A `FIFO18E1` (`cell`) or `FIFO36E1` `width` bits wide, `params` beside
    its `DATA_WIDTH`.

    `inverted` names the pins given `IS_<pin>_INVERTED`, each then driven the
    other way round from a flip-flop of its own, so that no synthesis can fold
    an inverter into the parameter; `driven` names `RSTREG`, `REGCE` and the
    two error injections, driven from the shift register rather than tied;
    `loc` is a site to hold the cell on; `ecc` folds the error correction's
    outputs into `so` too."""
    wide = cell == "FIFO36E1"
    data = 64 if wide else 32
    parity = 8 if wide else 4
    count = 13 if wide else 12
    top = data + parity + 2
    params = [("DATA_WIDTH", width)] + params
    params = params + [(f"IS_{pin}_INVERTED", "1'b1") for pin in inverted]
    plist = ",\n    ".join(f".{k}({v})" for k, v in params)
    outs = [("DO", data), ("DOP", parity), ("ALMOSTEMPTY", 1), ("ALMOSTFULL", 1),
            ("EMPTY", 1), ("FULL", 1), ("RDCOUNT", count), ("WRCOUNT", count),
            ("RDERR", 1), ("WRERR", 1)]
    if ecc:
        outs += [("DBITERR", 1), ("SBITERR", 1), ("ECCPARITY", 8)]
    wires = "".join(f"  wire [{w - 1}:0] o_{p.lower()};\n" for p, w in outs)
    regs = []

    def pin(name, value):
        # An inverted pin is driven inverted, from a register of its own.
        if name not in inverted:
            return value
        reg = f"{name.lower()}_n"
        regs.append(f"  reg {reg} = 1'b1;\n  always @(posedge clk) {reg} <= ~({value});\n")
        return reg

    def ctrl(name, tied, bit):
        return pin(name, f"sr[{bit}]" if name in driven else tied)

    conns = [f".DI(sr[{data - 1}:0])", f".DIP(sr[{data + parity - 1}:{data}])",
             f".WREN({pin('WREN', 'wren')})", f".RDEN({pin('RDEN', 'rden')})",
             f".RST({pin('RST', 'rst')})", f".RSTREG({ctrl('RSTREG', ZERO, 3)})",
             f".REGCE({ctrl('REGCE', ONE, 5)})", ".WRCLK(clk)", ".RDCLK(clk)"]
    if wide:
        conns += [f".INJECTDBITERR({ctrl('INJECTDBITERR', ZERO, 7)})",
                  f".INJECTSBITERR({ctrl('INJECTSBITERR', ZERO, 11)})"]
    conns += [f".{p}(o_{p.lower()})" for p, _ in outs]
    # Only the outputs the width drives are folded: the vendor's model reads
    # an `x` on the others, which would make `so` an `x` from the first cycle.
    used = {4: 4, 9: 8, 18: 16, 36: 32, 72: 64}.get(width, data)
    widths = {"DO": min(used, data), "DOP": min(max(width - used, 0), parity)}
    folds = " ^ ".join(
        f"^o_{p.lower()}" if widths.get(p, w) == w else f"^o_{p.lower()}[{widths[p] - 1}:0]"
        for p, w in outs if widths.get(p, w) > 0)
    return name, (
        f"// {comment}\n"
        f"module {name} (\n"
        "    input  wire clk,\n"
        "    input  wire si,\n"
        "    output reg  so\n"
        ");\n"
        f"  reg [{top}:0] sr;\n"
        f"  always @(posedge clk) sr <= {{sr[{top - 1}:0], si}};\n"
        "  // Five cycles of reset from power-up, then ten with nothing enabled,\n"
        "  // then writes and reads at random.\n"
        "  reg [3:0] boot = 4'd0;\n"
        "  always @(posedge clk) if (boot != 4'd15) boot <= boot + 4'd1;\n"
        "  wire rst = boot < 4'd5;\n"
        "  wire live = boot == 4'd15;\n"
        f"  wire wren = live & sr[{top - 1}];\n"
        f"  wire rden = live & sr[{top - 2}];\n"
        + wires
        + "".join(regs)
        + (f"  (* LOC = \"{loc}\" *)\n" if loc else "")
        + f"  {cell} #(\n    {plist}) dut (\n    {', '.join(conns)});\n"
        + f"  always @(posedge clk) so <= {folds} ^ sr[{top}];\n"
        "endmodule\n"
    )


def designs():
    sync = [("EN_SYN", '"TRUE"'), ("DO_REG", 0)]
    out = []
    for width in (4, 9, 18):
        out.append(fifo(f"f18_w{width}", f"A FIFO18E1 {width} wide, its clocks apart.", "FIFO18E1", width,
                        [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1)]))
    out.append(fifo("f18_w36", "A FIFO18E1 36 wide.", "FIFO18E1", 36,
                    [("FIFO_MODE", '"FIFO18_36"'), ("DO_REG", 1)]))
    out.append(fifo("f18_sync", "A FIFO18E1 on one clock.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"')] + sync))
    out.append(fifo("f18_sync_reg", "A FIFO18E1 on one clock, its output registered.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)]))
    out.append(fifo("f18_fwft", "A FIFO18E1 whose first word falls through.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1), ("FIRST_WORD_FALL_THROUGH", '"TRUE"')]))
    out.append(fifo("f18_offsets", "A FIFO18E1 with flags of its own.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1),
                     ("ALMOST_EMPTY_OFFSET", f"13'h{rng.randrange(6, 0x3f0):03x}"),
                     ("ALMOST_FULL_OFFSET", f"13'h{rng.randrange(6, 0x3f0):03x}")]))
    out.append(fifo("f18_values", "A FIFO18E1 starting and resetting to values of its own.", "FIFO18E1", 36,
                    [("FIFO_MODE", '"FIFO18_36"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1),
                     ("INIT", hexv(36)), ("SRVAL", hexv(36))]))
    for width in (4, 9, 18, 36):
        out.append(fifo(f"f36_w{width}", f"A FIFO36E1 {width} wide, its clocks apart.", "FIFO36E1", width,
                        [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1)]))
    out.append(fifo("f36_w72", "A FIFO36E1 72 wide.", "FIFO36E1", 72,
                    [("FIFO_MODE", '"FIFO36_72"'), ("DO_REG", 1)]))
    out.append(fifo("f36_sync", "A FIFO36E1 on one clock.", "FIFO36E1", 36,
                    [("FIFO_MODE", '"FIFO36"')] + sync))
    out.append(fifo("f36_fwft", "A FIFO36E1 whose first word falls through.", "FIFO36E1", 36,
                    [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1), ("FIRST_WORD_FALL_THROUGH", '"TRUE"')]))
    out.append(fifo("f36_offsets", "A FIFO36E1 with flags of its own.", "FIFO36E1", 36,
                    [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1),
                     ("ALMOST_EMPTY_OFFSET", f"13'h{rng.randrange(6, 0x3f0):03x}"),
                     ("ALMOST_FULL_OFFSET", f"13'h{rng.randrange(6, 0x3f0):03x}")]))

    # Every pin that can be inverted, inverted, on one clock and on two.
    inv = ("RDCLK", "WRCLK", "RDEN", "WREN", "RST", "RSTREG")
    out.append(fifo("f18_inv", "A FIFO18E1 on one clock with every inverter it has set.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)],
                    inverted=inv, driven=("RSTREG",)))
    out.append(fifo("f18_inv_async", "A FIFO18E1, its clocks apart, every inverter set.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1)], inverted=inv))
    out.append(fifo("f36_inv", "A FIFO36E1 on one clock with every inverter it has set.", "FIFO36E1", 36,
                    [("FIFO_MODE", '"FIFO36"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)],
                    inverted=inv, driven=("RSTREG",)))
    # The output register's reset and clock enable from logic.
    out.append(fifo("f18_ctrl", "A FIFO18E1 on one clock, its register reset and enabled by logic.",
                    "FIFO18E1", 18, [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)],
                    driven=("RSTREG", "REGCE")))
    out.append(fifo("f36_ctrl", "A FIFO36E1 on one clock, its register reset and enabled by logic.",
                    "FIFO36E1", 36, [("FIFO_MODE", '"FIFO36"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)],
                    driven=("RSTREG", "REGCE")))
    out.append(fifo("f36_w72_sync", "A FIFO36E1 72 wide on one clock.", "FIFO36E1", 72,
                    [("FIFO_MODE", '"FIFO36_72"')] + sync))
    out.append(fifo("f36_values", "A FIFO36E1 starting and resetting to values of its own.", "FIFO36E1", 72,
                    [("FIFO_MODE", '"FIFO36_72"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1),
                     ("INIT", hexv(72)), ("SRVAL", hexv(72))]))
    # Error correction, which only a FIFO36E1 72 wide has.
    for read, write, tag in (("TRUE", "TRUE", "ecc"), ("TRUE", "FALSE", "ecc_read"),
                             ("FALSE", "TRUE", "ecc_write")):
        out.append(fifo(f"f36_{tag}", f"A FIFO36E1 72 wide, EN_ECC_READ {read} and EN_ECC_WRITE {write}.",
                        "FIFO36E1", 72, [("FIFO_MODE", '"FIFO36_72"'), ("DO_REG", 1),
                                         ("EN_ECC_READ", f'"{read}"'), ("EN_ECC_WRITE", f'"{write}"')],
                        driven=("INJECTDBITERR", "INJECTSBITERR"), ecc=True))
    # On a `BRAM_L` tile, where the width's bits had not been measured.
    out.append(fifo("f18_left", "A FIFO18E1 18 wide on a BRAM_L tile.", "FIFO18E1", 18,
                    [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1)], loc="RAMB18_X2Y38"))
    out.append(fifo("f18_left_w36", "A FIFO18E1 36 wide on a BRAM_L tile.", "FIFO18E1", 36,
                    [("FIFO_MODE", '"FIFO18_36"'), ("DO_REG", 1)], loc="RAMB18_X2Y38"))
    out.append(fifo("f36_left", "A FIFO36E1 9 wide on a BRAM_L tile.", "FIFO36E1", 9,
                    [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1)], loc="RAMB36_X2Y19"))
    # What Vivado may refuse, for the rule it gives.
    out.append(fifo("fx_async_noreg", "A FIFO18E1, its clocks apart, without its output register.",
                    "FIFO18E1", 18, [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 0)]))
    out.append(fifo("fx_sync_fwft", "A FIFO18E1 on one clock whose first word falls through.",
                    "FIFO18E1", 18, [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 0),
                                     ("FIRST_WORD_FALL_THROUGH", '"TRUE"')]))
    out.append(fifo("fx_f18_w36_mode", "A FIFO18E1 36 wide in FIFO18 mode.", "FIFO18E1", 36,
                    [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1)]))
    out.append(fifo("fx_f36_w72_mode", "A FIFO36E1 72 wide in FIFO36 mode.", "FIFO36E1", 72,
                    [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1)]))
    out.append(fifo("fx_f18_w72", "A FIFO18E1 72 wide.", "FIFO18E1", 72,
                    [("FIFO_MODE", '"FIFO18_36"'), ("DO_REG", 1)]))
    out.append(fifo("fx_ecc_narrow", "A FIFO36E1 36 wide with error correction.", "FIFO36E1", 36,
                    [("FIFO_MODE", '"FIFO36"'), ("DO_REG", 1), ("EN_ECC_READ", '"TRUE"'),
                     ("EN_ECC_WRITE", '"TRUE"')], ecc=True))
    out.append(fifo("fx_offset_big", "A FIFO18E1 whose almost-full offset is past its depth.",
                    "FIFO18E1", 18, [("FIFO_MODE", '"FIFO18"'), ("DO_REG", 1),
                                     ("ALMOST_FULL_OFFSET", "13'h1fff")]))
    # Initial and reset values at every width, every bit random.
    for width in (4, 9, 18):
        out.append(fifo(f"f18_values_w{width}", f"A FIFO18E1 {width} wide with values of its own.",
                        "FIFO18E1", width, [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1),
                                            ("INIT", hexv(36)), ("SRVAL", hexv(36))]))
    for width in (4, 9, 18, 36):
        out.append(fifo(f"f36_values_w{width}", f"A FIFO36E1 {width} wide with values of its own.",
                        "FIFO36E1", width, [("FIFO_MODE", '"FIFO36"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1),
                                            ("INIT", hexv(72)), ("SRVAL", hexv(72))]))
    # 36 wide on one clock without the register, whose register clock a
    # FIFO36E1 72 wide leaves alone.
    out.append(fifo("f18_w36_sync", "A FIFO18E1 36 wide on one clock.", "FIFO18E1", 36,
                    [("FIFO_MODE", '"FIFO18_36"')] + sync))
    # Some inverters and not the others, so that each is seen on its own pin:
    # with all of them set, any two could be confused.
    out.append(fifo("f18_inv_some", "A FIFO18E1 on one clock, its reset and write enable inverted.",
                    "FIFO18E1", 18, [("FIFO_MODE", '"FIFO18"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1)],
                    inverted=("RST", "WREN"), driven=("RSTREG",)))
    return out


# The modes of `ranges()`: (tag, parameters).
MODES = [("async", [("DO_REG", 1)]),
         ("fwft", [("DO_REG", 1), ("FIRST_WORD_FALL_THROUGH", '"TRUE"')]),
         ("sync", [("EN_SYN", '"TRUE"'), ("DO_REG", 1)])]


def ranges():
    """Designs whose flag offsets are both past any depth, one per cell, width
    and mode, so that Vivado's refusal (`AVAL-8`, `AVAL-9`) says the range it
    allows there. Not fixtures: `fifo_ranges.py` keeps what Vivado said."""
    out = []
    for cell, mode, widths in (("FIFO18E1", "FIFO18", (4, 9, 18)), ("FIFO18E1", "FIFO18_36", (36,)),
                               ("FIFO36E1", "FIFO36", (4, 9, 18, 36)), ("FIFO36E1", "FIFO36_72", (72,))):
        for width in widths:
            for tag, params in MODES:
                name = f"fr_{cell[4:6]}_w{width}_{tag}"
                out.append(fifo(name, f"A {cell} {width} wide, {tag}, both offsets past its depth.",
                                cell, width, [("FIFO_MODE", f'"{mode}"')] + params
                                + [("ALMOST_EMPTY_OFFSET", "13'h1fff"),
                                   ("ALMOST_FULL_OFFSET", "13'h1fff")]))
    return out


def values():
    """Ten designs per cell and width whose initial and reset values are
    random, so that which bit of `INIT` and `SRVAL` each of the tile's
    `ZINIT` and `ZSRVAL` bits is can be solved for. Not fixtures:
    `fifo_values.py` keeps the answer."""
    vrng = random.Random(0xF1F1)
    out = []
    for cell, mode, widths in (("FIFO18E1", "FIFO18", (4, 9, 18)), ("FIFO18E1", "FIFO18_36", (36,)),
                               ("FIFO36E1", "FIFO36", (4, 9, 18, 36)), ("FIFO36E1", "FIFO36_72", (72,))):
        bits = 72 if cell == "FIFO36E1" else 36
        for width in widths:
            for k in range(10):
                init = f"{bits}'h{vrng.getrandbits(bits):0{bits // 4}x}"
                srval = f"{bits}'h{vrng.getrandbits(bits):0{bits // 4}x}"
                name = f"fv_{cell[4:6]}_w{width}_{k}"
                out.append(fifo(name, f"A {cell} {width} wide with random values, {k}.", cell, width,
                                [("FIFO_MODE", f'"{mode}"'), ("EN_SYN", '"TRUE"'), ("DO_REG", 1),
                                 ("INIT", init), ("SRVAL", srval)]))
    return out


def main():
    if sys.argv[1] == "--ranges":
        out = sys.argv[2]
        made = ranges()
    elif sys.argv[1] == "--values":
        out = sys.argv[2]
        made = values()
    else:
        out = sys.argv[1]
        made = designs()
    os.makedirs(out, exist_ok=True)
    for name, text in made:
        for ext, body in ((".v", text), (".xdc", XDC), (".rcf", RCF)):
            with open(os.path.join(out, name + ext), "w") as f:
                f.write(body)


main()
