#!/usr/bin/env python3
"""Writes testdata/fpga/vivado/config/vivado.txt from what `config.sh` left:
`config_fixture.py <out> <reticle> <fixture>`.

Every `.bit` is read by `reticle fpga --decode`. For each design:

- `cell <ref> <site>`: where Vivado put each configuration primitive.
- `site <tile type>: <features>`: what the bitstream sets in a configuration
  tile (`CFG_CENTER_MID`, `CFG_CENTER_TOP`), by name, sorted. A bit the
  database has no name for is a `bit <tile type> <frame>_<bit>` line. No other
  tile is here: the rest is the placer's and the router's.
- `ground <tile> <feature>`: a feature of the interconnect column the
  configuration tile's pins enter from (`INT_L_X18`) on the way from a
  `GND_WIRE`: how an input was tied to zero.
- `stream base`, or `stream <packets>`: the stream's packets other than the
  frames and the CRCs, in `bit_packets.py`'s words joined by ` | `, when they
  are not the `base` design's.
- `refused <rules>` in place of all of that, when Vivado wrote no bitstream.
"""
import concurrent.futures as cf
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
import bit_packets  # noqa: E402

TILES = ("CFG_CENTER_MID", "CFG_CENTER_TOP")


def stream(bit):
    with open(bit, "rb") as f:
        lines = bit_packets.dump(f.read())
    return [line for line in lines if not line.startswith(("FDRI", "CRC"))]


def decode(reticle, bit):
    run = subprocess.run(
        ["nice", "-n", "19", reticle, "fpga", "--device", "xc7a100t-ftg256", "--decode", bit],
        capture_output=True,
        text=True,
        check=True,
    )
    sites = {}
    bits = set()
    ground = []
    for line in run.stdout.splitlines():
        if line.startswith("? "):
            tile, at = line[2:].split()
            kind = re.sub(r"_X[0-9]+Y[0-9]+$", "", tile)
            if kind in TILES:
                bits.add(f"{kind} {at}")
            continue
        words = line.split(None, 1)
        if len(words) != 2 or line.startswith("!"):
            continue
        tile, feature = words
        kind = re.sub(r"_X[0-9]+Y[0-9]+$", "", tile)
        if kind in TILES:
            sites.setdefault(kind, []).append(feature)
        elif tile.startswith("INT_L_X18Y"):
            ground.append((tile, feature))
    lines = [f"{kind}: {' '.join(sorted(f))}" for kind, f in sorted(sites.items())]
    return lines, sorted(bits), grounded(ground)


def grounded(features):
    """The features of `features` (tile, name) on a road from a `GND_WIRE`: a
    fan's own hop from one, what a grounded fan drives, and a clock line's
    bounce from a grounded `FAN_ALT`. A fan a signal passes through is not one."""
    have = set(features)
    out = set()
    for tile, name in features:
        to, _, src = name.partition(".")
        if src == "GND_WIRE":
            out.add((tile, name))
        elif src.startswith("GFAN") and (tile, f"{src}.GND_WIRE") in have:
            out.add((tile, name))
    for tile, name in features:
        to, _, src = name.partition(".")
        if to.startswith("CLK_L") and src.startswith("FAN_BOUNCE"):
            alt = "FAN_ALT" + src[len("FAN_BOUNCE"):]
            if any(t == tile and n.startswith(alt + ".") for t, n in out):
                out.add((tile, name))
    return sorted(f"{t} {n}" for t, n in out)


def refusal(log):
    rules = []
    for line in log.splitlines():
        m = re.match(r"(?:ERROR|CRITICAL WARNING): \[((?:DRC )?[A-Za-z_ ]*\d*-?\d+)\]", line)
        if not m or "Common 17" in line or "Vivado 12-1345" in line or "Vivado_Tcl 4-23" in line:
            continue
        if line.startswith("ERROR") or "DRC " in m.group(1):
            rules.append(m.group(1).replace("DRC ", ""))
    return " ".join(sorted(set(rules))) or "unknown"


def one(args):
    out, reticle, name, base = args
    log = open(f"{out}/{name}.log", errors="replace").read()
    bit = f"{out}/{name}/{name}.bit"
    text = f"design {name}\n"
    if not os.path.exists(bit):
        return text + f"  refused {refusal(log)}\n"
    for line in open(f"{out}/{name}/{name}.cells"):
        if line.startswith("cell "):
            w = line.split()
            text += f"  cell {w[3]} {w[5]}\n"
    sites, bits, ground = decode(reticle, bit)
    text += "".join(f"  site {s}\n" for s in sites)
    text += "".join(f"  bit {b}\n" for b in bits)
    text += "".join(f"  ground {g}\n" for g in ground)
    packets = stream(bit)
    text += "  stream base\n" if packets == base else f"  stream {' | '.join(packets)}\n"
    return text


def main():
    out, reticle, fixture = sys.argv[1:4]
    names = sorted(n for n in os.listdir(out) if os.path.isdir(f"{out}/{n}") and not n.endswith(".tmp"))
    base = stream(f"{out}/base/base.bit")
    with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "16"))) as pool:
        parts = list(pool.map(one, [(out, reticle, n, base) for n in names]))
    with open(fixture, "w") as f:
        f.write(
            "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n"
            "# tools/vivado-parity/config.sh and config_fixture.py, which says how to read it.\n"
            f"base-stream {' | '.join(base)}\n"
        )
        f.writelines(parts)


if __name__ == "__main__":
    main()
