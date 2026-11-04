#!/usr/bin/env python3
"""What differs between two decodings of one implementation, by class.

    tools/vivado-parity/compare_bits.py <reticle.features> <vivado.features> [--cells] [--show N]

Each file is what `reticle fpga --decode` printed: `<tile> <feature>`, and
`? <tile> <bit>` for a set bit the database has no name for. The classes are
this script's reading of a feature's name; a class is a question to answer, not
an answer, and `docs/fpga-vivado-parity.md` says which have been answered.
With `--cells`, routing is left out, for two implementations that route apart.
"""
import collections, re, sys


def read(path):
    out = set()
    for line in open(path):
        line = line.rstrip("\n")
        if line:
            tile, feature = line.split(" ", 1)
            out.add((tile, feature))
    return out


def is_routing(tile, feature):
    """A pip: in an interconnect or clock tile, or `<wire>.<wire>` in a block RAM's
    or a pad's logic tile, whose wires are all in capitals with no site named."""
    if tile.startswith(("INT_", "CLK_", "HCLK_", "LIOI", "RIOI")):
        return True
    return tile.startswith("BRAM") and bool(re.match(r"BRAM_[A-Z0-9_]+\.", feature))


def classify(tile, feature):
    if tile == "?":
        return "a bit the database does not name"
    f = re.sub(r"^(SLICE[LM]_X\d|IOB_Y\d|RAMB18_Y\d|BUFGCTRL_X0Y\d+|BUFGCTRL\.BUFGCTRL_X0Y\d+)\.", "", feature)
    if feature.startswith("IOB") or tile.startswith(("LIOB", "RIOB")):
        return "pad: " + re.sub(r"\d+", "#", f.split(".")[0] + "." + ".".join(f.split(".")[1:2]))
    if tile.startswith(("LIOI", "RIOI")):
        return "IO logic: " + re.sub(r"\d+", "#", f)
    if tile.startswith(("CLK_", "HCLK_")):
        return "clock tile: " + re.sub(r"\d+", "#", tile.split("_X")[0])
    if tile.startswith(("CLB", "BRAM")) is False and re.search(r"\.VCC_WIRE$", feature):
        return "interconnect: a tie to VCC_WIRE"
    if re.search(r"\.GND_WIRE$", feature):
        return "interconnect: a tie to GND_WIRE"
    if tile.startswith("INT_"):
        return "interconnect: a pip"
    if tile.startswith("BRAM"):
        return "block RAM: " + re.sub(r"\d+", "#", f.split("[")[0])
    if tile.startswith("CLB"):
        name = f.split("[")[0]
        return "slice: " + re.sub(r"^[A-D](5?)(LUT|FF|FFMUX|OUTMUX|USED)", r"#\1\2", name)
    return "other: " + tile.split("_X")[0] + " " + re.sub(r"\d+", "#", f)


def main():
    ours, theirs = read(sys.argv[1]), read(sys.argv[2])
    if "--cells" in sys.argv:
        keep = lambda item: not is_routing(*item)
        ours, theirs = set(filter(keep, ours)), set(filter(keep, theirs))
    show = 3
    if "--show" in sys.argv:
        show = int(sys.argv[sys.argv.index("--show") + 1])
    print(f"reticle features {len(ours)}, vivado features {len(theirs)}, in common {len(ours & theirs)}")
    for name, only in (("only reticle sets", ours - theirs), ("only vivado sets", theirs - ours)):
        classes = collections.defaultdict(list)
        for item in sorted(only):
            classes[classify(*item)].append(item)
        print(f"{name}: {len(only)}")
        for cls, items in sorted(classes.items(), key=lambda kv: (-len(kv[1]), kv[0])):
            print(f"  {len(items):5} {cls}")
            for tile, feature in items[:show]:
                print(f"        {tile} {feature}")


main()
