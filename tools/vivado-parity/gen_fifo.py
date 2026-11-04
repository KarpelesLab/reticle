#!/usr/bin/env python3
"""Writes the designs of testdata/fpga/vivado/fifo/: `gen_fifo.py <dir>`.

Each asks Vivado about one `FIFO18E1` or `FIFO36E1` by name, in one setting:
its data in from a shift register, its writes and reads enabled at random once
a reset at power-up is over, and every output folded into `so`. The reset is
what the vendor's model asks for (five cycles high, no enable during it, and
time after), so that a simulation of either build answers. `FIXTURE=fifo
lutram.sh <out>` runs them and keeps what came back. Only answers are
committed; these designs are written here.
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


def hexv(bits):
    return f"{bits}'h{rng.getrandbits(bits):0{(bits + 3) // 4}x}"


def fifo(name, comment, cell, width, params):
    """A `FIFO18E1` (`cell`) or `FIFO36E1` `width` bits wide, `params` beside
    its `DATA_WIDTH`."""
    wide = cell == "FIFO36E1"
    data = 64 if wide else 32
    parity = 8 if wide else 4
    count = 13 if wide else 12
    top = data + parity + 2
    params = [("DATA_WIDTH", width)] + params
    plist = ",\n    ".join(f".{k}({v})" for k, v in params)
    outs = [("DO", data), ("DOP", parity), ("ALMOSTEMPTY", 1), ("ALMOSTFULL", 1),
            ("EMPTY", 1), ("FULL", 1), ("RDCOUNT", count), ("WRCOUNT", count),
            ("RDERR", 1), ("WRERR", 1)]
    wires = "".join(f"  wire [{w - 1}:0] o_{p.lower()};\n" for p, w in outs)
    conns = [f".DI(sr[{data - 1}:0])", f".DIP(sr[{data + parity - 1}:{data}])",
             ".WREN(wren)", ".RDEN(rden)", ".RST(rst)", ".RSTREG(1'b0)", ".REGCE(1'b1)",
             ".WRCLK(clk)", ".RDCLK(clk)"]
    if wide:
        conns += [".INJECTDBITERR(1'b0)", ".INJECTSBITERR(1'b0)"]
    conns += [f".{p}(o_{p.lower()})" for p, _ in outs]
    folds = " ^ ".join(f"^o_{p.lower()}" for p, _ in outs)
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
    return out


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, text in designs():
        for ext, body in ((".v", text), (".xdc", XDC), (".rcf", RCF)):
            with open(os.path.join(out, name + ext), "w") as f:
                f.write(body)


main()
