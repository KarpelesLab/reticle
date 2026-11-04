#!/usr/bin/env python3
"""What Vivado made of each design of `gen_dsp_inference.py`, a line each.

Reads the <top>.facts `dsp_facts.tcl` left and writes, per design, how many
blocks and other cells it has, and per block the registers on, its mode pins
and what its data pins are tied to or fed by.

    tools/vivado-parity/dsp_inference_report.py <out> > testdata/fpga/vivado/dsp/inference.txt
"""
import collections
import os
import re
import sys

REGS = ("ACASCREG", "AREG", "BCASCREG", "BREG", "CREG", "MREG", "PREG", "OPMODEREG", "ALUMODEREG",
        "INMODEREG", "CARRYINREG", "CARRYINSELREG", "ADREG", "DREG")


def bus_value(pins, name):
    """A mode bus as a binary number, or `x` for a bit a signal drives."""
    bits = []
    index = 0
    while f"{name}[{index}]" in pins:
        net = pins[f"{name}[{index}]"]
        bits.append("1" if net == "<const1>" or net.startswith("VCC") else "0" if net == "<const0>" or net.startswith("GND") else "x")
        index += 1
    return "".join(reversed(bits))


def describe(pins, name):
    value = bus_value(pins, name)
    if set(value) <= {"0"}:
        return "zero"
    if set(value) <= {"1"}:
        return "ones"
    if "x" not in value:
        return value
    signals = sum(1 for bit in value if bit == "x")
    return f"{signals}sig"


def main():
    out = sys.argv[1]
    print("# What Vivado 2026.1 synthesis, place and route made of each design of")
    print("# tools/vivado-parity/gen_dsp_inference.py on xc7a100tftg256-1.")
    print("# Made by tools/vivado-parity/dsp_inference_report.py from dsp_facts.tcl.")
    for name in sorted(os.listdir(out)):
        path = os.path.join(out, name, f"{name}.facts")
        if not os.path.exists(path):
            continue
        blocks = []
        others = collections.Counter()
        for line in open(path):
            words = line.split()
            if words[0] == "cell":
                blocks.append({"name": words[1], "loc": words[3], "params": {}, "pins": {}})
            elif words[0] == "param":
                blocks[-1]["params"][words[1]] = " ".join(words[2:])
            elif words[0] == "pin":
                blocks[-1]["pins"][words[1]] = words[3][4:]
            elif words[0] == "other" and words[1] not in ("GND", "VCC", "IBUF", "OBUF", "BUFG"):
                others[re.sub(r"LUT\d|LUT\d_\d", "LUT", words[1])] += 1
        print(f"{name} dsp={len(blocks)} " + " ".join(f"{k}={v}" for k, v in sorted(others.items())))
        for block in blocks:
            params = block["params"]
            pins = block["pins"]
            regs = " ".join(f"{r}={params[r]}" for r in REGS if params[r] not in ("0",) and r not in ("ADREG", "DREG"))
            ctrl = " ".join(p + ("=1" if pins[p] == "<const1>" else "=0" if pins[p] == "<const0>" else "=sig")
                            for p in ("CEA1", "CEA2", "CEB1", "CEB2", "CEM", "CEP", "RSTA", "RSTB", "RSTM", "RSTP"))
            used = sum(1 for k, v in pins.items() if k.startswith("P[") and v)
            print(f"  {block['loc']} {regs} OPMODE={bus_value(pins, 'OPMODE')} INMODE={bus_value(pins, 'INMODE')} "
                  f"ALUMODE={bus_value(pins, 'ALUMODE')} A={describe(pins, 'A')} B={describe(pins, 'B')} "
                  f"C={describe(pins, 'C')} D={describe(pins, 'D')} P_read={used} "
                  f"ACOUT={'y' if pins.get('ACOUT[0]') else 'n'} BCOUT={'y' if pins.get('BCOUT[0]') else 'n'} "
                  f"PCOUT={'y' if pins.get('PCOUT[0]') else 'n'} PCIN={describe(pins, 'PCIN')} "
                  f"AI={params['A_INPUT']} BI={params['B_INPUT']} {ctrl}")


if __name__ == "__main__":
    main()
