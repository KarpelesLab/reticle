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


if __name__ == "__main__":
    what = sys.argv[1]
    if what == "constraints":
        constraints(*sys.argv[2:5])
    elif what == "periods":
        periods(float(sys.argv[2]), sys.argv[3:])
    elif what == "table":
        table(sys.argv[2:])
    else:
        sys.exit(__doc__)
