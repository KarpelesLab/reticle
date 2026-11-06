#!/usr/bin/env python3
"""The frames of a clock manager's or an XADC's registers in Vivado's
bitstreams, with which of their bits are the block's, for
`tests/fpga_xray_bitstream.rs`'s question of what a frame's ECC leaves out.

    tools/vivado-parity/drp_ecc.py <prjxray-db>/artix7 <fabric> <part> <reticle> <device> <run_all.sh output>... \
        > testdata/fpga/vivado/bitstream/drp_ecc.txt

`<fabric>` is the database's die (`xc7a50t` for the XC7A35T) and `<part>` the
package's directory (`xc7a35tftg256-1`).

Each `.bit` is decoded with `reticle fpga --decode`; the clock manager's tile
(`CMT_TOP_*`) or the XADC's (`MONITOR_BOT`) is found by its features, and each
of its frames 28 and 29 is written whole with the positions of the bits that
are the block's: every bit of a `segbits` feature the decode names in that
tile, or of an XADC register (`XADC.INIT_<r>[<b>]`, at `xray::xadc_init_bit`'s
place: frame `28 + i % 2`, bit `1696 + i / 2`, `i = (r - 0x40) * 16 + b`).
A line is `<design> <address> <stored ECC> <words>... | <word>.<bit>...`.
"""
import json, os, re, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bitstream_fixtures as bf  # noqa: E402


def main():
    db, fabric, part, reticle, device, *runs = sys.argv[1:]
    fars = bf.far_of(bf.layout_rows(os.path.join(db, part, "part.json")))
    grid = json.load(open(os.path.join(db, fabric, "tilegrid.json")))
    segbits = {}

    def bits_of(kind, feature):
        if kind not in segbits:
            d = {}
            path = os.path.join(db, f"segbits_{kind.lower()}.db")
            if os.path.isfile(path):
                for line in open(path):
                    name, *bits = line.split()
                    d[name] = [b for b in bits if not b.startswith("!")]
            segbits[kind] = d
        return segbits[kind].get(f"{kind}.{feature}", [])

    print("# Frames 28 and 29 of a clock manager's or an XADC's tile in Vivado 2026.1's bitstreams;")
    print("# tools/vivado-parity/drp_ecc.py, which says how to read them.")
    for run in runs:
        for top in sorted(os.listdir(run)):
            bit = os.path.join(run, top, f"{top}.bit")
            if not os.path.isfile(bit):
                continue
            decoded = subprocess.run([reticle, "fpga", "--device", device, "--decode", bit],
                                     capture_output=True, text=True, check=True).stdout
            block = {}
            for line in decoded.splitlines():
                tile, _, feature = line.partition(" ")
                kind = re.sub(r"_X\d+Y\d+$", "", tile)
                if not (kind.startswith("CMT_TOP_") or kind == "MONITOR_BOT"):
                    continue
                if kind == "MONITOR_BOT":
                    m = re.fullmatch(r"XADC\.INIT_([0-9A-F]{2})\[(\d+)\]", feature)
                    if not m:
                        continue
                    i = (int(m.group(1), 16) - 0x40) * 16 + int(m.group(2))
                    found = [f"{28 + i % 2}_{1696 + i // 2}"]
                else:
                    found = bits_of(kind, feature)
                block.setdefault(tile, []).extend(found)
            if not block:
                continue
            _, words = bf.stream_words(bit)
            frames = bf.fdri(words)
            index = {far: k for k, far in enumerate(fars) if far is not None}
            for tile, bits in sorted(block.items()):
                window = grid[tile]["bits"]["CLB_IO_CLK"]
                base = int(window["baseaddr"], 16)
                for minor in (28, 29):
                    address = base + minor
                    k = index[address]
                    frame = frames[k * 101:k * 101 + 101]
                    mine = []
                    for b in bits:
                        row, col = map(int, b.split("_"))
                        if row == minor:
                            mine.append(f"{window['offset'] + col // 32}.{col % 32}")
                    if not mine:
                        continue
                    print(f"{top} {address:08x} {frame[50] & 0x1FFF:04x} "
                          + " ".join(f"{w:x}" for w in frame) + " | " + " ".join(sorted(set(mine))))


main()
