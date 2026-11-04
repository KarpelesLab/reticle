#!/usr/bin/env python3
"""What Vivado wrote for each DSP48E1, in a form a test can hold Reticle to.

For each design with a `.bit` in <out>, decodes it with `reticle fpga --decode`
and writes testdata/fpga/vivado/dsp/answers/<name>.txt: one block a paragraph, its
site features and each input pin's source. `tests/fpga_dsp_vivado.rs` computes the
same from what Reticle writes. Copies the design's source beside it, since the
test has to build it too.

A pin on an `IMUX` or `CTRL` wire is `zero` when that wire takes a `GFAN` that
`GND_WIRE` drives, `one` when nothing drives the wire, and `signal` otherwise; a
local mux's constant is `GND` or `VCC` in the features. Interconnect is compared by
its ends, which is all two routers can be asked to agree on. `CLK` is left out: a
block no signal clocks has no use for what it reads, and Vivado routes it a bounce
wire.

    tools/vivado-parity/dsp_answers.py <reticle> <designs> <out> [name...]
"""
import glob
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ANSWERS = os.path.join(ROOT, "testdata/fpga/vivado/dsp/answers")
DESIGNS = os.path.join(ROOT, "testdata/fpga/vivado/dsp/designs")
DEVICE = "xc7a100t-ftg256"


def input_wires(side):
    """Each block's input pins and the interconnect wire each is on."""
    base = glob.glob(os.path.expanduser("~/.cache/reticle/prjxray-db/*"))[0]
    out = {}
    for line in open(f"{base}/artix7/ppips_dsp_{side}.db"):
        name, _ = line.split()
        _, to, source = name.split(".")
        found = re.match(r"DSP_(IMUX|CTRL)(\d+)_(\d)$", source)
        if to.startswith(("DSP_0_", "DSP_1_")) and found:
            out[to] = found.groups()
    return out


def wires_clock(side, block):
    """Which of a tile's interconnect rows a block's `CLK` is on."""
    base = glob.glob(os.path.expanduser("~/.cache/reticle/prjxray-db/*"))[0]
    for line in open(f"{base}/artix7/ppips_dsp_{side.lower()}.db"):
        name, _ = line.split()
        _, to, source = name.split(".")
        if to == f"{block}_CLK":
            return int(source.rsplit("_", 1)[1])
    raise ValueError(block)


def summary(lines):
    """Blocks of a decode: a sorted list of lines per used block."""
    tiles = {}
    for line in lines:
        words = line.split()
        if len(words) == 2:
            tiles.setdefault(words[0], set()).add(words[1])
    blocks = []
    for tile, features in sorted(tiles.items()):
        found = re.match(r"DSP_([LR])_X(\d+)Y(\d+)$", tile)
        if not found:
            continue
        side, x, y = found.group(1), int(found.group(2)), int(found.group(3))
        wires = input_wires(side.lower())
        used = sorted({f.split(".")[1] for f in features if f.startswith("DSP48.DSP_")})
        for block in used:
            named = []
            states = {"zero": [], "one": [], "signal": []}
            for feature in features:
                if feature.startswith(f"DSP48.{block}."):
                    named.append(feature.split(".", 2)[2])
                elif feature.startswith(f"{block}_"):
                    pin, source = feature.split(".")
                    named.append(f"{pin[6:]}={source[4:7]}")
            for pin, (kind, number, row) in wires.items():
                if not pin.startswith(block + "_"):
                    continue
                inter = f"INT_{side}_X{x}Y{y + int(row)}"
                name = kind + ("_L" if side == "L" else "") + number
                arcs = [f.split(".")[1] for f in tiles.get(inter, ()) if f.startswith(name + ".")]
                if not arcs:
                    state = "one"
                elif arcs[0].startswith("GFAN") and f"{arcs[0]}.GND_WIRE" in tiles.get(inter, ()):
                    state = "zero"
                else:
                    state = "signal"
                states[state].append(pin[6:])
            # The clock by whether it comes off the global network, which is all
            # that can be asked of a clock.
            clock = wires_clock(side, block)
            inter = f"INT_{side}_X{x}Y{y + clock}"
            name = "CLK_L0" if side == "L" else "CLK0"
            arcs = [f.split(".")[1] for f in tiles.get(inter, ()) if f.startswith(name + ".")]
            network = "network" if arcs and arcs[0].startswith("GCLK") else "none"
            out = ["features " + " ".join(sorted(named))]
            out.append(f"clock {network}")
            out += [f"{state} " + " ".join(sorted(pins)) for state, pins in sorted(states.items())]
            blocks.append(out)
    return sorted(blocks)


def main():
    reticle, designs, out = sys.argv[1:4]
    only = sys.argv[4:]
    os.makedirs(ANSWERS, exist_ok=True)
    os.makedirs(DESIGNS, exist_ok=True)
    for path in sorted(glob.glob(f"{designs}/*.v")):
        name = os.path.basename(path)[:-2]
        bit = f"{out}/{name}/{name}.bit"
        if not os.path.exists(bit) or (only and name not in only):
            continue
        run = subprocess.run([reticle, "fpga", "--device", DEVICE, "--decode", bit], capture_output=True, text=True)
        blocks = summary(run.stdout.splitlines())
        with open(f"{ANSWERS}/{name}.txt", "w") as f:
            f.write(f"# What Vivado 2026.1 wrote for {name}; tools/vivado-parity/dsp_answers.py.\n")
            for block in blocks:
                f.write("block\n")
                for line in block:
                    f.write(line + "\n")
        shutil.copy(path, f"{DESIGNS}/{name}.v")


main()
