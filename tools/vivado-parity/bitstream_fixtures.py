#!/usr/bin/env python3
"""Turns Vivado's `.bit` files into the answers under
`testdata/fpga/vivado/bitstream/`, which `tests/fpga_xray_bitstream.rs` reads.

    bitstream_fixtures.py options <tag> <part json> <cfg> <mon> <base.bit> <lists> <fixtures> <out dir>...
    bitstream_fixtures.py ecc <fixtures> <count> <part json>=<file.bit>...

`options` writes `<tag>.layout.txt`, `<tag>.frames.txt` and `<tag>.options.txt`:
the part's frame layout, the non-zero words of the base design's frames, and for
each option run its name, what it set, and the length and hash of Vivado's whole
stream, with the packet lines that differ from the base's where there are few
enough to read. A run Vivado refused says why, in its own words.

`ecc` writes `ecc_frames.txt`, a sample of real frames chosen for having many
bits set, from each `.bit` given. A frame is the only thing committed of a
`.bit`, and these are of designs generated here.
"""
import difflib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bit_packets as bp  # noqa: E402


def stream_words(path):
    with open(path, "rb") as f:
        data = bp.payload(f.read())
    sync = data.find(bytes.fromhex("aa995566"))
    lead = sync // 4
    words = [int.from_bytes(data[i:i + 4], "big") for i in range(sync + 4, len(data), 4)]
    return lead, words


def fdri(words):
    out = []
    i = 0
    last = 0
    while i < len(words):
        h = words[i]
        i += 1
        kind = h >> 29
        if kind == 1:
            last = (h >> 13) & 0x3FFF
            n = h & 0x7FF
        elif kind == 2:
            n = h & 0x7FFFFFF
        else:
            continue
        if (h >> 27) & 3 == 2 and last == 2 and n:
            out.extend(words[i:i + n])
        i += n
    return out


def layout_rows(part_json):
    with open(part_json) as f:
        p = json.load(f)
    regions = p["global_clock_regions"]
    bus_ids = {"CLB_IO_CLK": 0, "BLOCK_RAM": 1, "CFG_CLB": 2}
    rows = []
    for half, h in (("top", 0), ("bottom", 1)):
        for name, r in regions[half]["rows"].items():
            for bus, b in r["configuration_buses"].items():
                cols = b["configuration_columns"]
                frames = [cols[str(c)]["frame_count"] for c in range(len(cols))]
                rows.append((bus_ids[bus], h, int(name), frames))
    return sorted(rows)


def far_of(rows):
    """Stream position to frame address, with None for a pad frame."""
    out = []
    for block, half, row, columns in rows:
        for column, n in enumerate(columns):
            for minor in range(n):
                out.append((block << 23) | (half << 22) | (row << 17) | (column << 7) | minor)
        out += [None, None]
    return out


def options(args):
    tag, part_json, cfg, mon, base, lists, fixtures, *dirs = args
    lists = [lists]
    rows = layout_rows(part_json)
    with open(f"{fixtures}/{tag}.layout.txt", "w") as f:
        f.write("# The frame layout: block half row, frames per column. From part.json.\n")
        f.write(f"tiles {cfg} {mon}\n")
        for block, half, row, columns in rows:
            f.write(f"row {block} {half} {row} " + ",".join(map(str, columns)) + "\n")

    lead, words = stream_words(base)
    frames = fdri(words)
    with open(f"{fixtures}/{tag}.frames.txt", "w") as f:
        f.write("# The non-zero words of the frames Vivado wrote for options_probe: stream\n")
        f.write("# position of the word, value. Everything else is zero.\n")
        for i, w in enumerate(frames):
            if w:
                f.write(f"{i} {w:08x}\n")

    base_dump = bp.dump(open(base, "rb").read())
    sets = {}
    for path in lists:
        for line in open(path):
            parts = line.split()
            if len(parts) >= 2:
                sets[parts[0]] = parts[1:]
    with open(f"{fixtures}/{tag}.options.txt", "w") as f:
        f.write(f"# Vivado 2026.1's write_bitstream of options_probe on {tag}, per option.\n")
        f.write("# `stream`: dummy words before the sync word, words after it, hash of those\n")
        f.write("# words. `-` and `+` are packet lines against the base, where few.\n")
        f.write(f"base words={len(words)} lead={lead} fnv={bp.fnv(words):016x}\n")
        for line in base_dump:
            f.write(f"= {line}\n")
        names = []
        for d in dirs:
            names += sorted(
                (n[:-4], d) for n in os.listdir(d) if n.endswith((".bit",))
            )
            names += sorted(
                (n[:-8], d) for n in os.listdir(d) if n.endswith(".refused")
            )
        for name, d in sorted(names):
            f.write(f"run {name}\n")
            refused = f"{d}/{name}.refused"
            dump = []
            if not os.path.exists(refused):
                dump = bp.dump(open(f"{d}/{name}.bit", "rb").read())
            for kv in sets.get(name, []):
                # The one value that is a clock: what the clock said.
                if kv.endswith("USR_ACCESS=TIMESTAMP"):
                    value = next(line[5:] for line in dump if line.startswith("AXSS "))
                    kv = f"{kv.split('=')[0]}=0X{value.upper()}"
                f.write(f"set {kv}\n")
            if os.path.exists(refused):
                why = open(refused).read().strip().split("\n")[0][:200]
                f.write(f"refused {why}\n")
                continue
            lead, words = stream_words(f"{d}/{name}.bit")
            f.write(f"stream words={len(words)} lead={lead} fnv={bp.fnv(words):016x}\n")
            diff = [
                line
                for line in difflib.unified_diff(base_dump, dump, lineterm="", n=0)
                if not line.startswith(("---", "+++", "@@"))
            ]
            if len(diff) <= 24:
                for line in diff:
                    f.write(f"{line[:120]}\n")


def ecc(args):
    fixtures, count, *inputs = args
    count = int(count)
    out = []
    for item in inputs:
        part_json, path = item.split("=", 1)
        fars = far_of(layout_rows(part_json))
        _, words = stream_words(path)
        frames = fdri(words)
        scored = []
        for k in range(len(frames) // 101):
            frame = frames[k * 101:k * 101 + 101]
            ones = sum(bin(w).count("1") for w in frame)
            if ones:
                scored.append((ones, k, frame))
        # The most bits; those that hold a clock tile's bits in word 50; and the
        # block RAM frames, whose ECC is zero.
        scored.sort(key=lambda s: (-s[0], s[1]))
        chosen = scored[:count]
        chosen += [s for s in scored if s[2][50] >> 13][:count // 3]
        chosen += [s for s in scored if fars[s[1]] and (fars[s[1]] >> 23) == 1][:count // 3]
        chosen = {s[1]: s for s in chosen}
        name = os.path.basename(path)
        for k in sorted(chosen):
            ones, _, frame = chosen[k]
            far = fars[k]
            out.append(f"{name} {far:08x} " + " ".join(f"{w:x}" for w in frame))
    with open(f"{fixtures}/ecc_frames.txt", "w") as f:
        f.write("# Whole frames Vivado 2026.1 wrote: file, frame address, 101 words in hex.\n")
        f.write("# From designs tools/vivado-parity/gen_ecc_design.py generates.\n")
        f.write("\n".join(out) + "\n")


if __name__ == "__main__":
    {"options": options, "ecc": ecc}[sys.argv[1]](sys.argv[2:])
