#!/usr/bin/env python3
"""Holds each decoded Vivado bitstream to two rules of its clock tiles, and
prints every file whose bits are not the rule's, then how many were checked.

    tools/vivado-parity/clock_rules.py <prjxray-db>/artix7/<fabric> <features>...

`<features>` is what `reticle fpga --decode` printed of a `.bit`; `<fabric>` is
the database's die (`xc7a50t` for the XC7A35T, `xc7a100t`).

- **GCLK**: a global track `k` leaves its buffer's tile (`CLK_BUFG_*_CK_GCLK<k>`)
  and is cut at every `CLK_BUFG_REBUF`. The live segments are the buffer's own
  and every one between it and each clock row whose `CLK_HROW_*_CK_GCLK<k>_ACTIVE`
  is set; each live segment `[lo, hi]` sets `lo`'s `GCLK<k>_ENABLE_BELOW` and
  `hi`'s `GCLK<k>_ENABLE_ABOVE`, and no other enable is set.
- **BUFHCLK**: the row's `HCLK_CMT` (side 0, the left) or `HCLK_CMT_L` (side 1)
  has `HCLK_CMT_CK_BUFHCLK<n>_USED` for every `BUFHCE_X<side>Y<n>` in use in the
  row's `CLK_HROW`, and nothing when the side has no such tile.

`docs/fpga-vivado-parity.md` ("Vivado's bits beside a clock, answered") has what
it found.
"""
import collections, json, os, re, sys


def main():
    fabric, *paths = sys.argv[1:]
    grid = json.load(open(os.path.join(fabric, "tilegrid.json")))

    def y(tile):
        return int(re.search(r"Y(\d+)$", tile).group(1))

    rebufs = sorted(y(t) for t in grid if t.startswith("CLK_BUFG_REBUF"))
    cmts = {t for t in grid if t.startswith("HCLK_CMT")}

    def segment(at):
        lo = max([r for r in rebufs if r < at], default=None)
        hi = min([r for r in rebufs if r > at], default=None)
        return lo, hi

    def enables(k, bufg, rows):
        live = {segment(bufg)}
        for row in rows:
            live.add(segment(row))
            a, b = sorted((bufg, row))
            for r in rebufs:
                if a < r < b:
                    live.add(segment(r - 0.5))
                    live.add(segment(r + 0.5))
        out = set()
        for lo, hi in live:
            if lo is not None:
                out.add((lo, f"GCLK{k}_ENABLE_BELOW"))
            if hi is not None:
                out.add((hi, f"GCLK{k}_ENABLE_ABOVE"))
        return out

    wrong = collections.Counter()
    for path in paths:
        feats = [l.split(" ", 1) for l in open(path).read().splitlines()
                 if l and not l.startswith(("?", "!", "decoded"))]
        tracks = {}
        for t, f in feats:
            m = re.match(r"CLK_BUFG_CK_GCLK(\d+)\.CLK_BUFG_BUFGCTRL\d+_O", f)
            if m and t.startswith("CLK_BUFG_"):
                tracks[int(m.group(1))] = y(t)
        rows = collections.defaultdict(set)
        for t, f in feats:
            m = re.match(r"CLK_HROW_R_CK_GCLK(\d+)_ACTIVE", f)
            if m and t.startswith("CLK_HROW"):
                rows[int(m.group(1))].add(y(t))
        have = {(y(t), f) for t, f in feats if t.startswith("CLK_BUFG_REBUF") and "_ENABLE_" in f}
        want = set()
        for k, at in tracks.items():
            want |= enables(k, at, rows.get(k, set()))
        if have != want:
            wrong["gclk"] += 1
            print("GCLK", path, "Vivado only", sorted(have - want), "rule only", sorted(want - have))
        used = set()
        for t, f in feats:
            m = re.match(r"BUFHCE\.BUFHCE_X(\d)Y(\d+)\.IN_USE", f)
            if m and t.startswith("CLK_HROW"):
                prefix = "HCLK_CMT_X" if m.group(1) == "0" else "HCLK_CMT_L_X"
                used |= {(c, f"HCLK_CMT_CK_BUFHCLK{m.group(2)}_USED") for c in cmts
                         if c.startswith(prefix) and y(c) == y(t)}
        marked = {(t, f) for t, f in feats if re.match(r"HCLK_CMT_CK_BUFHCLK\d+_USED", f)}
        if marked != used:
            wrong["bufhclk"] += 1
            print("BUFHCLK", path, "Vivado only", sorted(marked - used), "rule only", sorted(used - marked))
    print(f"checked {len(paths)}, disagreeing {dict(wrong)}")


main()
