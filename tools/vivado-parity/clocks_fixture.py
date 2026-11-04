#!/usr/bin/env python3
"""Writes testdata/fpga/vivado/clocks/vivado.txt from what `clocks.sh` left:
`clocks_fixture.py <out> <reticle> <fixture>`.

Every `.bit` is read by `reticle fpga --decode`. A `site` line is one clock site
of the design, with the features set on it: which of several sites is which is
the placer's. A `bit` line is a set bit the database has no name for. A `refused`
line is Vivado's refusal, with its rule, and there is nothing else for that
design.
"""
import concurrent.futures as cf
import os
import re
import subprocess
import sys

SITE = re.compile(
    r"^(BUFGCTRL\.BUFGCTRL_X0Y\d+|BUFHCE\.BUFHCE_X\d+Y\d+|BUFR_Y\d|BUFIO\.BUFIO_Y\d|BUFMRCE\.BUFMRCE_Y\d|MMCME2_ADV|PLLE2_ADV)\.(.*)$"
)


def read(reticle, bit):
    run = subprocess.run(
        ["nice", "-n", "19", reticle, "fpga", "--device", "xc7a100t-ftg256", "--offline", "--decode", bit],
        capture_output=True,
        text=True,
        check=True,
    )
    sites = {}
    bits = []
    for line in run.stdout.splitlines():
        if line.startswith("? "):
            tile, at = line[2:].split()
            bits.append(f"{re.sub(r'_X[0-9]+Y[0-9]+$', '', tile)} {at}")
            continue
        words = line.split(None, 1)
        if len(words) != 2 or line.startswith("!"):
            continue
        tile, feature = words
        m = SITE.match(feature)
        if m:
            sites.setdefault((tile, m.group(1)), []).append(m.group(2))
    site_lines = []
    for (_, name), features in sites.items():
        kind = re.sub(r"_X\d+Y\d+|_Y\d", "", name).split(".")[0]
        site_lines.append(f"{kind}: {' '.join(sorted(features))}")
    return sorted(site_lines), sorted(bits)


def one(args):
    out, reticle, name = args
    log = open(f"{out}/{name}.log", errors="replace").read()
    bit = f"{out}/{name}/{name}.bit"
    text = f"design {name}\n"
    if not os.path.exists(bit):
        rules = []
        for line in log.splitlines():
            m = re.match(r"(?:ERROR|CRITICAL WARNING): \[((?:DRC )?[A-Za-z_ ]*\d*-?\d+)\]", line)
            if not m or "Common 17" in line or "Vivado 12-1345" in line or "Vivado_Tcl 4-23" in line:
                continue
            if line.startswith("ERROR") or "DRC " in m.group(1):
                rules.append(m.group(1).replace("DRC ", ""))
        return text + f"  refused {' '.join(sorted(set(rules))) or 'unknown'}\n"
    for line in open(f"{out}/{name}/{name}.cells"):
        if line.startswith("cell "):
            w = line.split()
            if w[3] in ("IBUF", "OBUF"):
                continue
            kind = re.sub(r"_X\d+Y\d+$", "", w[7])
            text += f"  cell {w[1]} {w[3]} {kind}\n"
    sites, bits = read(reticle, bit)
    text += "".join(f"  site {s}\n" for s in sites)
    text += "".join(f"  bit {b}\n" for b in bits)
    return text


def main():
    out, reticle, fixture = sys.argv[1:4]
    names = sorted(n for n in os.listdir(out) if os.path.isdir(f"{out}/{n}") and not n.endswith(".tmp"))
    with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "16"))) as pool:
        parts = list(pool.map(one, [(out, reticle, n) for n in names]))
    with open(fixture, "w") as f:
        f.write(
            "# What Vivado 2026.1 made of each design here, on xc7a100tftg256-1, from\n"
            "# tools/vivado-parity/clocks.sh and clocks_fixture.py, which says how to read it.\n"
        )
        f.writelines(parts)


if __name__ == "__main__":
    main()
