#!/usr/bin/env python3
"""What each setting of the sweep changed in the block's tile, as a table.

Reads <out>/<name>/<name>.bit as `gen_dsp_sweep.py` and `run_all.sh` left them,
decodes each with `reticle fpga --decode`, and writes one line per setting: what
it added to and took away from the `base` setting's features in the block's tile,
with the tile's position taken out so that the sites compare. A bit the database
has no name for is written as `?<frame>_<bit>`; `USE_MULT_NONE`, which Reticle
names, is written by that name when the decoder knows it.

    tools/vivado-parity/dsp_sweep_report.py <reticle> <out> > testdata/fpga/vivado/dsp/sweep.txt
"""
import os
import re
import subprocess
import sys

DEVICE = "xc7a100t-ftg256"


def decode(reticle, path):
    run = subprocess.run(
        [reticle, "fpga", "--device", DEVICE, "--decode", path],
        capture_output=True,
        text=True,
    )
    return run.stdout.splitlines()


def block(lines):
    """The DSP tile's features and unnamed bits, keyed on the tile and block they are in."""
    tiles = {}
    for line in lines:
        words = line.split()
        if words[0] == "?" and len(words) == 3 and words[1].startswith("DSP_"):
            tiles.setdefault((words[1], "?"), set()).add("?" + words[2])
        elif words[0].startswith("DSP_") and len(words) == 2:
            found = re.search(r"DSP_(\d)", words[1])
            name = re.sub(r"DSP_\d", "DSP_k", words[1])
            name = re.sub(r"_(GND|VCC)_[LR]$", r"_\1", name)
            tiles.setdefault((words[0], found.group(1)), set()).add(name)
    return tiles


def main():
    reticle, out = sys.argv[1:3]
    results = {}
    names = sorted(d for d in os.listdir(out) if os.path.exists(os.path.join(out, d, f"{d}.bit")))
    for name in names:
        results[name] = block(decode(reticle, os.path.join(out, name, f"{name}.bit")))
    base = next(iter(results["base"].values()))
    def plain(values):
        hidden = re.compile(r"^(DSP48\.DSP_\d\.)?(MASK|PATTERN)\[")
        return sorted(v for v in values if not hidden.match(v))
    print("# One DSP48E1 at DSP48_X2Y50 unless the setting names another, through Vivado 2026.1.")
    print("# Made by tools/vivado-parity/gen_dsp_sweep.py and dsp_sweep_report.py.")
    print("# `?f_b` is a set bit of the block's tile that the database has no name for.")
    for name in names:
        tiles = results[name]
        for (tile, which), values in sorted(tiles.items()):
            add = sorted(values - base)
            gone = sorted(base - values)
            print(f"{name} {tile.split('_X')[0]} DSP_{which} add {' '.join(add) or '-'} ; gone {' '.join(gone) or '-'}")
    print("# base's own features, MASK and PATTERN left out:")
    for value in plain(base):
        print(f"base-feature {value}")


if __name__ == "__main__":
    main()
