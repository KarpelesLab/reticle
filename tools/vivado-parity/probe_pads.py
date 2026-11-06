#!/usr/bin/env python3
"""What Vivado's own bitstream of each primitive probe sets in the tiles its
constraints pin, for `tests/fpga_xray_probes.rs`.

    tools/vivado-parity/probe_pads.py <run_all.sh output> <device> <reticle> > <ledger dir>/pads.txt

Each `.bit` is read back with `reticle fpga --decode`, and kept are the
features of every pad tile (`LIOB33`, `RIOB33`), IO logic tile (`LIOI3`,
`RIOI3`), bank clock tile (`HCLK_IOI3`), configuration tile (`CFG_CENTER_*`)
and the XADC's (`MONITOR_BOT`), and the bits of those the database has no name
for. Where a design's logic and clocks went is the two tools' own business, so:

- a pip of an IO logic tile is written as the wire it drives and `driven`:
  which clock leaf or interconnect input reached a pin is the router's, and
  whether the pin is reached at all is not;
- a pip of a bank clock tile is left out, and the tile's position too, and
  which of its regional buffers is used (`BUFR_Y#`, `BUFIO_Y#`): its clock
  spine is the clock placer's, and which bank's buffer took a multi-region
  buffer's clock is a placement.

`tests/fpga_xray_probes.rs` reduces Reticle's bitstream of the same probe in
the same words; `docs/fpga-vivado-parity.md` has what came of it.
"""
import os, re, subprocess, sys

KEEP = re.compile(r"^([LR]IOB33|[LR]IOI3|HCLK_IOI3|CFG_CENTER|MONITOR_BOT)")
PIP = re.compile(r"^(HCLK_IOI|IOI|LIOI|RIOI|LIOB|RIOB|CFG_CENTER|MONITOR)_[A-Z0-9_]*\.[A-Z0-9_]+$")


def reduce(text):
    out = []
    for line in text.splitlines():
        if not line or line.startswith(("!", "decoded")):
            continue
        tile, _, feature = line.partition(" ")
        if tile == "?":
            if KEEP.match(feature):
                out.append(line)
            continue
        if not KEEP.match(tile):
            continue
        kind = re.sub(r"_X\d+Y\d+$", "", tile)
        if kind == "HCLK_IOI3":
            if PIP.match(feature):
                continue
            tile = kind
            feature = re.sub(r"^(BUFR|BUFIO)_Y\d+", r"\1_Y#", feature)
        elif PIP.match(feature):
            feature = re.sub(r"\d+", "#", feature.split(".")[0]) + " driven"
        out.append(f"{tile} {feature}")
    return sorted(out)


def main():
    runs, device, reticle = sys.argv[1:4]
    print(f"# What Vivado 2026.1 set in the tiles the constraints pin, for each probe on {device};")
    print("# tools/vivado-parity/probe_pads.py, which says how it is reduced.")
    for top in sorted(os.listdir(runs)):
        bit = os.path.join(runs, top, f"{top}.bit")
        if not os.path.isfile(bit):
            continue
        text = subprocess.run([reticle, "fpga", "--device", device, "--decode", bit],
                              capture_output=True, text=True, check=True).stdout
        print(f"design {top}")
        for line in reduce(text):
            print(f"  {line}")


main()
