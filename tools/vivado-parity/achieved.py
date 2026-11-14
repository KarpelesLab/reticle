#!/usr/bin/env python3
"""The constraints and the tables of achieved.sh.

    achieved.py constraints <corpus> <out> [periods]
        Each design's .rcf of <corpus> into <out> as an .rcf and an .xdc, every
        clock's period replaced by the periods file's (`design clock ns` lines)
        where it names one, and a design's clocks asynchronous to each other in
        the .xdc, since Reticle times no path between two clocks. The .v is
        linked. A line of the .rcf that is neither a clock nor a pad is kept in
        the .rcf alone (`analyser_fifo`'s false paths, which Vivado needs none of:
        it times no unconstrained pad).
    achieved.py periods <factor> <dir>...
        Each clock's best achieved period over the runs of Vivado's own flow in
        <dir>s, times <factor>, as a periods file.
    achieved.py table [--fixture] own=<dir>[,<dir>] [netlist=<dir>] <label>=<root>...
        Per design and clock, Vivado's best own figure, Vivado's placement and
        routes of Reticle's netlist, and each run root's median over its seeds
        with each seed's figure; then each root's geometric mean against the
        first two columns. A root holds s<seed>/<design>/routes/<design>.result.
        With --fixture, the lines testdata/fpga/vivado/qor/achieved.txt holds.
    achieved.py netlists [--fixture] vnetlist=<dir> <label>=<root>...
        The netlists alone, each placed and routed by Vivado on cons3: per
        design and clock, Vivado's own netlist (achieved.sh's vnetlist/) and
        each root's median over its seeds (pnr/s<seed>/<design>/<design>.result),
        then each root's geometric mean against Vivado's netlist. With
        --fixture, achieved.txt's `vnetlist` and `pnr` lines.
    achieved.py hops <out> [design...]
        Each clock's worst path (the `paths.rpt` achieved.tcl writes) of
        Reticle's routes (<out>/reticle/s<n>/<d>/routes) and of Vivado's
        placement of Reticle's netlist (<out>/pnr/s<n>/<d>), split into the
        clock's terms (the period achieved less the data path), the cells'
        delays and the connections' by where their two ends are: one slice,
        the two slices of one tile, or two tiles. Per clock, the means over
        the seeds both have; then the means over the clocks, and each kind
        of connection's mean delay over every worst path.
"""
import math, os, re, statistics, sys


def clocks(path):
    out = {}
    if os.path.exists(path):
        text = open(path).read()
        for m in re.finditer(r"^clock (\S+) period \S+ slack \S+ achieved (\S+)", text, re.M):
            out[m[1]] = float(m[2])
    return out


def constraints(corpus, out, periods_file=None):
    periods = {}
    if periods_file:
        for line in open(periods_file):
            w = line.split()
            if len(w) == 3:
                periods[(w[0], w[1])] = w[2]
    os.makedirs(out, exist_ok=True)
    for f in sorted(os.listdir(corpus)):
        if not f.endswith(".rcf"):
            continue
        d = f[:-4]
        rcf, xdc, names = [], [], []
        for line in open(f"{corpus}/{f}"):
            m = re.match(r"create_clock -name (\S+) -period (\S+) (\S+)", line)
            if m:
                name, period, port = m[1], periods.get((d, m[1]), m[2]), m[3]
                names.append(name)
                rcf.append(f"create_clock -name {name} -period {period} {port}")
                xdc.append(f"create_clock -period {period} -name {name} [get_ports {{{port}}}]")
                continue
            m = re.match(r"set_io -io_standard (\S+) (\S+) (\S+)", line)
            if m:
                rcf.append(line.rstrip())
                xdc.append(f"set_property PACKAGE_PIN {m[3]} [get_ports {{{m[2]}}}]")
                xdc.append(f"set_property IOSTANDARD {m[1]} [get_ports {{{m[2]}}}]")
                continue
            if line.strip():
                rcf.append(line.rstrip())
        if len(names) > 1:
            xdc.append("set_clock_groups -asynchronous " + " ".join(f"-group {c}" for c in names))
        open(f"{out}/{d}.rcf", "w").write("\n".join(rcf) + "\n")
        open(f"{out}/{d}.xdc", "w").write("\n".join(xdc) + "\n")
        v = f"{out}/{d}.v"
        if not os.path.exists(v):
            os.symlink(os.path.abspath(f"{corpus}/{d}.v"), v)


def periods(factor, dirs):
    best = {}
    for root in dirs:
        for d in sorted(os.listdir(root)):
            for c, v in clocks(f"{root}/{d}/{d}.result").items():
                best[(d, c)] = min(best.get((d, c), math.inf), v)
    for (d, c), v in sorted(best.items()):
        print(d, c, f"{v * factor:.2f}")


def table(args):
    fixture = "--fixture" in args
    own, net, runs = [], None, []
    for a in args:
        if a == "--fixture":
            continue
        k, v = a.split("=", 1)
        if k == "own":
            own = v.split(",")
        elif k == "netlist":
            net = v
        else:
            runs.append((k, v))
    designs = sorted({d for _, r in runs for s in os.listdir(r) if s.startswith("s")
                      for d in os.listdir(f"{r}/{s}")})
    if not fixture:
        print("| design | clock | Vivado | Vivado, Reticle's netlist |" + "".join(f" {k} |" for k, _ in runs))
        print("|---|---|---|---|" + "---|" * len(runs))
    ratios = {k: ([], []) for k, _ in runs}
    for d in designs:
        best = {}
        for o in own:
            for c, v in clocks(f"{o}/{d}/{d}.result").items():
                best[c] = min(best.get(c, math.inf), v)
        n = clocks(f"{net}/{d}/{d}.result") if net else {}
        per = []
        for _, root in runs:
            seeds = sorted((s for s in os.listdir(root) if s.startswith("s")), key=lambda s: int(s[1:]))
            per.append([(s[1:], clocks(f"{root}/{s}/{d}/routes/{d}.result")) for s in seeds])
        for c in sorted(set(best) | {c for p in per for _, r in p for c in r}):
            if fixture:
                if c in best:
                    print(f"vivado {d} {c} {best[c]:.3f}")
                if c in n:
                    print(f"netlist {d} {c} {n[c]:.3f}")
            row = f"| {d} | {c} | {best.get(c, math.nan):.3f} | {n.get(c, math.nan):.3f} |"
            for (k, _), p in zip(runs, per):
                vals = [(s, r[c]) for s, r in p if c in r]
                if fixture:
                    for s, v in vals:
                        print(f"reticle {k} {s} {d} {c} {v:.3f}")
                if not vals:
                    row += " - |"
                    continue
                med = statistics.median(v for _, v in vals)
                row += f" {med:.3f} ({' '.join(f'{v:.2f}' for _, v in vals)}) |"
                if c in best:
                    ratios[k][0].append(med / best[c])
                if c in n:
                    ratios[k][1].append(med / n[c])
            if not fixture:
                print(row)
    if not fixture:
        for k, (a, b) in ratios.items():
            if a:
                g = statistics.geometric_mean
                print(f"{k}: geometric mean {g(a):.3f} of Vivado's own"
                      + (f", {g(b):.3f} of Vivado's on the netlist" if b else ""))


def netlists(args):
    fixture = "--fixture" in args
    vnet, runs = None, []
    for a in args:
        if a == "--fixture":
            continue
        k, v = a.split("=", 1)
        if k == "vnetlist":
            vnet = v
        else:
            runs.append((k, v))
    designs = sorted(os.listdir(vnet))
    ratios = {k: [] for k, _ in runs}
    for d in designs:
        own = clocks(f"{vnet}/{d}/{d}.result")
        per = []
        for _, root in runs:
            seeds = sorted((s for s in os.listdir(root) if s.startswith("s")), key=lambda s: int(s[1:]))
            per.append([(s[1:], clocks(f"{root}/{s}/{d}/{d}.result")) for s in seeds])
        for c in sorted(own):
            if fixture:
                print(f"vnetlist {d} {c} {own[c]:.3f}")
            row = f"| {d} | {c} | {own[c]:.3f} |"
            for (k, _), p in zip(runs, per):
                vals = [(s, r[c]) for s, r in p if c in r]
                if fixture:
                    for s, v in vals:
                        print(f"pnr {k} {s} {d} {c} {v:.3f}")
                if not vals:
                    row += " - |"
                    continue
                med = statistics.median(v for _, v in vals)
                row += f" {med:.3f} ({' '.join(f'{v:.2f}' for _, v in vals)}) |"
                ratios[k].append(med / own[c])
            if not fixture:
                print(row)
    if not fixture:
        for k, r in ratios.items():
            if r:
                print(f"{k}: geometric mean {statistics.geometric_mean(r):.3f} of Vivado's netlist")


def where(a, b):
    """`slice`, `tile` or `far`: where the two ends of a connection are, by
    their sites. A slice's tile holds `SLICE_X<2k>` and `SLICE_X<2k+1>` of a
    row; anything not a slice is `far`."""
    xy = lambda site: re.match(r"SLICE_X(\d+)Y(\d+)$", site or "")
    if a is None or b is None:
        return "far"
    if a == b:
        return "slice"
    pa, pb = xy(a), xy(b)
    if pa and pb and pa[2] == pb[2] and int(pa[1]) // 2 == int(pb[1]) // 2:
        return "tile"
    return "far"


def paths(rpt):
    """Each path of a report_timing file: its group, slack, period achieved,
    data path, and each connection as (delay, where, from site, to site)."""
    out = []
    rule = "-------------------------------------------------------------------    -------------------"
    for block in open(rpt).read().split("\nSlack")[1:]:
        group = re.search(r"Path Group:\s+(\S+)", block)
        slack = float(re.match(r"\s*\(\w+\)\s*:\s*(-?[\d.]+)ns", block)[1])
        req = float(re.search(r"Requirement:\s+([\d.]+)ns", block)[1])
        data = float(re.search(r"Data Path Delay:\s+([\d.]+)ns", block)[1])
        sections = block.split(rule)
        hops, site, pending = [], None, None
        for line in (sections[2] if len(sections) > 2 else "").splitlines():
            m = re.match(r"\s+net \(fo=\d+, \w+\)\s+([\d.]+)\s", line)
            if m:
                pending = (float(m[1]), site)
                continue
            m = re.match(r"\s+(\S+)\s+(?:[A-Z]\w*\s+)?[rf]\s+\S+/\S+$", line.rstrip())
            if m and pending:
                hops.append((pending[0], where(pending[1], m[1]), pending[1], m[1]))
                pending = None
                continue
            m = re.match(r"\s+(\S+)\s+\S+ \((Prop|Setup)", line)
            if m:
                site = m[1]
        out.append(dict(group=group[1] if group else "?", slack=slack, achieved=req - slack,
                        data=data, hops=hops))
    return out


def hops(args):
    out, only = args[0], set(args[1:])
    kinds = ("slice", "tile", "far")
    flows = {"ours": "reticle/s{s}/{d}/routes/paths.rpt", "pnr": "pnr/s{s}/{d}/paths.rpt"}
    designs = sorted(f[:-2] for f in os.listdir(f"{out}/cons1") if f.endswith(".v"))
    data = {}
    for flow, pattern in flows.items():
        for s in range(1, 21):
            for d in designs:
                if only and d not in only:
                    continue
                rpt = f"{out}/" + pattern.format(s=s, d=d)
                if not os.path.exists(rpt):
                    continue
                worst = {}
                for p in paths(rpt):
                    if p["group"] not in worst or p["slack"] < worst[p["group"]]["slack"]:
                        worst[p["group"]] = p
                for c, p in worst.items():
                    data.setdefault((d, c), {}).setdefault(flow, {})[s] = p
    mean = lambda xs: sum(xs) / len(xs) if xs else float("nan")
    rows = []
    print("| design | clock | flow | seeds | achieved | clock's terms | cells | one slice | one tile | two tiles |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for k in sorted(data):
        if set(data[k]) != set(flows):
            continue
        seeds = sorted(set(data[k]["ours"]) & set(data[k]["pnr"]))
        if not seeds:
            continue
        row = {}
        for flow in flows:
            ps = [data[k][flow][s] for s in seeds]
            route = {w: [sum(h[0] for h in p["hops"] if h[1] == w) for p in ps] for w in kinds}
            count = {w: [sum(1 for h in p["hops"] if h[1] == w) for p in ps] for w in kinds}
            r = dict(achieved=mean([p["achieved"] for p in ps]),
                     clock=mean([p["achieved"] - p["data"] for p in ps]),
                     cells=mean([p["data"] - sum(h[0] for h in p["hops"]) for p in ps]),
                     **{w: mean(route[w]) for w in kinds}, **{"n" + w: mean(count[w]) for w in kinds})
            row[flow] = r
            print(f"| `{k[0]}` | `{k[1]}` | {flow} | {len(seeds)} | {r['achieved']:.3f} | {r['clock']:.3f} | "
                  f"{r['cells']:.3f} | " + " | ".join(f"{r[w]:.3f} ({r['n' + w]:.1f})" for w in kinds) + " |")
        rows.append(row)
    print()
    for term in ("achieved", "clock", "cells") + kinds + tuple("n" + w for w in kinds):
        o, v = mean([r["ours"][term] for r in rows]), mean([r["pnr"][term] for r in rows])
        print(f"{term}: ours {o:.3f} pnr {v:.3f} difference {o - v:+.3f}")
    for flow in flows:
        every = [h for r in data.values() for p in r.get(flow, {}).values() for h in p["hops"]]
        for w in kinds:
            hs = [h[0] for h in every if h[1] == w]
            nz = [x for x in hs if x > 0]
            print(f"{flow} {w}: {len(hs)} connections, mean {mean(hs):.3f} ns; {len(nz)} not zero, mean {mean(nz):.3f}")


if __name__ == "__main__":
    what = sys.argv[1]
    if what == "constraints":
        constraints(*sys.argv[2:5])
    elif what == "periods":
        periods(float(sys.argv[2]), sys.argv[3:])
    elif what == "table":
        table(sys.argv[2:])
    elif what == "netlists":
        netlists(sys.argv[2:])
    elif what == "hops":
        hops(sys.argv[2:])
    else:
        sys.exit(__doc__)
