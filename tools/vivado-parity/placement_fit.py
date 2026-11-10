#!/usr/bin/env python3
"""The placement's estimate of each connection before it is routed, against what
the route made of it and against Vivado's timer; and the fit of that estimate.

    placement_fit.py fit <every> <run>...         > placement.txt
    placement_fit.py compare [--db <artix7 dir>] <name>=<run>...

A run is achieved.sh's `<out>/reticle`: <run>/s<seed>/<design>/endpoints.txt,
which `reticle fpga --timing-endpoints` writes, with an `estimate` line for each
connection (the placement's estimate, the routed one, the two ends' columns and
rows apart, the fanout, the two pins' kinds) and an `estimate_endpoint` line for
each endpoint (the placement's arrival and the routed one).

`fit` fits, by least squares over every connection of the runs that is not a
clock's nor one the placement charges as a dedicated group's own wire, the
routed estimate as: a base; a table by columns apart from 0 to 8 and two slopes
past it (past 8 and past 16); the same by rows; a figure by fanout band; and a
figure for each kind of sink and of driver crossed by at least MIN_KIND
connections, a lookup table's input by how many of the table's inputs are used.
It writes the fit and one connection in <every>, what
testdata/fpga/vivado/timing/placement.txt holds. The fit predicts the routed
delay better than the placement's line and, put in the placement, did not make
the achieved periods any faster: docs/fpga-vivado-parity.md, "The placement's
estimate, refitted", which has its figures and the patch's shape.

`compare` prints, for each run named: the ratio of the placement's estimate to
the routed one, a connection and an endpoint at a time (median, 5th and 95th
percentile); for each design and clock the placement's latest endpoint, the
route's, and how many of the placement's twenty latest are the route's. With
--db and achieved.sh's routes checkpoints timed by timing_dump.tcl
(<design>/routes/<design>.timing), the same against Vivado's timer, matched as
timing_table.py compare matches. Only numbers come out.
"""
import collections, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import timing_table as tt  # noqa: E402

# A kind of pin crossed by fewer connections than this has no figure of its own.
MIN_KIND = 100
# The tables' length, in columns or rows apart.
STEPS = 8
FANOUT = [(1, 1), (2, 4), (5, 8), (9, 16), (17, 32), (33, 64), (65, 10**9)]


def kind(pin):
    k, role = pin.split(".", 1)
    return k + "." + re.sub(r"\d+$", "", role)


def sink_kind(c):
    k = kind(c["sink"])
    return f"{k}{min(c['used'], 6)}" if k == "lut.i" else k


def builds(run):
    for seed in sorted((s for s in os.listdir(run) if s.startswith("s")), key=lambda s: int(s[1:])):
        for design in sorted(os.listdir(f"{run}/{seed}")):
            path = f"{run}/{seed}/{design}/endpoints.txt"
            if os.path.exists(path):
                yield int(seed[1:]), design, path


def connections(path):
    for line in open(path):
        if not line.startswith("estimate "):
            continue
        w = line.split()
        c = dict(est=float(w[1]), routed=float(w[2]), dx=int(w[3]), dy=int(w[4]),
                 fanout=int(w[5]), local=w[6] == "1", believed=float(w[7]),
                 critical=float(w[8]), driver=w[9], sink=w[10], used=int(w[11]), wire=w[12])
        if c["driver"].startswith("gb.") or c["sink"].endswith(".clk") or "CLK" in c["wire"]:
            continue
        if c["local"] or c["routed"] <= 0.0:
            continue
        yield c


def endpoints(path):
    for line in open(path):
        if line.startswith("estimate_endpoint "):
            w = line.split()
            yield dict(clock=w[1], placed=float(w[2]), routed=float(w[4]), pin=w[6])


def features(c, sinks, drivers):
    """The fit's terms a connection has, as {index: value}."""
    ax, ay = abs(c["dx"]), abs(c["dy"])
    f, n = {0: 1.0}, 1
    for axis in (ax, ay):
        for k in range(1, STEPS + 1):
            if axis >= k:
                f[n + k - 1] = 1.0
        n += STEPS
        f[n] = float(max(axis - STEPS, 0))
        f[n + 1] = float(max(axis - 2 * STEPS, 0))
        n += 2
    band = next(i for i, (lo, hi) in enumerate(FANOUT) if lo <= c["fanout"] <= hi)
    if band:
        f[n + band - 1] = 1.0
    n += len(FANOUT) - 1
    s = sink_kind(c)
    if s in sinks:
        f[n + sinks.index(s)] = 1.0
    n += len(sinks)
    d = kind(c["driver"])
    if d in drivers:
        f[n + drivers.index(d)] = 1.0
    return {k: v for k, v in f.items() if v}


def names(sinks, drivers):
    out = ["base"]
    for axis in ("dx", "dy"):
        out += [f"{axis}>={k}" for k in range(1, STEPS + 1)]
        out += [f"{axis}-{STEPS}", f"{axis}-{2 * STEPS}"]
    out += [f"fanout_{lo}-{hi if hi < 10**9 else ''}" for lo, hi in FANOUT[1:]]
    out += [f"sink_{s}" for s in sinks] + [f"driver_{d}" for d in drivers]
    return out


def least_squares(rows, n):
    """Plain least squares, coefficients of either sign, over sparse rows."""
    ata = [[0.0] * n for _ in range(n)]
    aty = [0.0] * n
    for f, y in rows:
        items = list(f.items())
        for i, a in items:
            aty[i] += a * y
            row = ata[i]
            for j, b in items:
                row[j] += a * b
    for i in range(n):
        ata[i][i] += 1e-9 * (ata[i][i] + 1.0)
    return tt.gauss(ata, aty)


def fit(every, runs):
    data = [c for run in runs for _, _, path in builds(run) for c in connections(path)]
    sinks = collections.Counter(sink_kind(c) for c in data)
    drivers = collections.Counter(kind(c["driver"]) for c in data)
    # The base is a table's input with all six used, driven by a flip-flop.
    sinks = sorted(k for k, n in sinks.items() if n >= MIN_KIND and k != "lut.i6")
    drivers = sorted(k for k, n in drivers.items() if n >= MIN_KIND and k != "ff.q")
    terms = names(sinks, drivers)
    x = least_squares([(features(c, sinks, drivers), c["routed"]) for c in data], len(terms))
    fitted = [sum(x[i] * v for i, v in features(c, sinks, drivers).items()) for c in data]
    line = [c["est"] / c["routed"] for c in data]
    new = [f / c["routed"] for f, c in zip(fitted, data)]
    print("# The placement's estimate of a connection before it is routed, refitted:")
    print("# tools/vivado-parity/placement_fit.py fit over the `estimate` lines")
    print("# `reticle fpga --timing-endpoints` writes. docs/fpga-vivado-parity.md,")
    print("# \"The placement's estimate, refitted\".")
    print("#   fit <term> <ns>    a figure of the least-squares fit: the base, the steps")
    print("#                      of the tables by columns (dx) and rows (dy) apart and")
    print("#                      their slopes past 8 and 16, the fanout bands, and the")
    print("#                      kinds of sink and driver (a table's input by inputs used)")
    print("#   c <dx> <dy> <fanout> <driver kind.role> <sink kind.role> <table inputs used>")
    print("#     <routed ns> <line's ns>")
    print(f"#                      one connection in {every}: its routed estimate")
    print("#                      (xray::delay's table on its route), and the line's")
    print("#                      (VIVADO_WIRE) the fit replaces")
    print(f"# {len(data)} connections; estimate / routed, median, 5th and 95th percentile, lowest, highest:")
    print(f"# the line {tt.spread(line)}")
    print(f"# the fit  {tt.spread(new)}")
    for t, v in zip(terms, x):
        print(f"fit {t} {v:.4f}")
    for i, c in enumerate(data):
        if i % every == 0:
            print(f"c {c['dx']} {c['dy']} {c['fanout']} {kind(c['driver'])} {kind(c['sink'])} "
                  f"{c['used']} {c['routed']:.3f} {c['est']:.3f}")


def compare(db, named):
    name_at = {xy: n for n, xy in tt.grid(db, "xc7a100t").items()} if db else {}
    for label, run in named:
        conn, ends, viv_conn, viv_ends = [], [], [], []
        clocks = collections.defaultdict(list)
        for seed, design, path in builds(run):
            conn += [c["est"] / c["routed"] for c in connections(path)]
            by = collections.defaultdict(list)
            for e in endpoints(path):
                if e["routed"] > 0:
                    ends.append(e["placed"] / e["routed"])
                by[e["clock"]].append(e)
            for clock, es in by.items():
                placed = sorted(es, key=lambda e: -e["placed"])[:20]
                routed = sorted(es, key=lambda e: -e["routed"])[:20]
                both = len({e["pin"] for e in placed} & {e["pin"] for e in routed})
                clocks[(design, clock)].append((placed[0]["placed"], routed[0]["routed"],
                                                both / len(placed)))
            timing = f"{os.path.dirname(path)}/routes/{design}.timing"
            if not db or not os.path.exists(timing):
                continue
            nets, _, paths = tt.load(timing)
            theirs = {}
            for net, pin, ps, chain, _, _ in tt.connections(nets):
                node = chain[-1].split("/")[0] + "/" + tt.split_pip(chain[-1])[2]
                if ps / 1000.0 >= tt.SHORTEST_NS and not tt.is_clock_net(chain):
                    theirs[node] = max(theirs.get(node, 0), ps / 1000.0)
            for c in connections(path):
                m = re.match(r"X(\d+)Y(\d+)/(\S+)$", c["wire"])
                tile = m and name_at.get((int(m[1]), int(m[2])))
                if tile and f"{tile}/{m[3]}" in theirs:
                    viv_conn.append(c["est"] / theirs[f"{tile}/{m[3]}"])
            ours = {}
            for e in endpoints(path):
                cell, role = e["pin"].rsplit(".", 1)
                ours[(e["clock"], tt.safe(cell), role)] = e["placed"]
            for start, end, frm, to, data, logic, route, slack, skew, setup, levels in paths:
                key = tt.vivado_key(to, {})
                if start != end or not key or (end,) + key not in ours:
                    continue
                try:
                    need = float(data) + float(setup) - float(skew)
                except ValueError:
                    continue
                if need > 0:
                    viv_ends.append(ours[(end,) + key] / need)
        print(f"run {label}")
        print(f"  connections, placement / routed {len(conn)} {tt.spread(conn)}")
        print(f"  endpoints, placement / routed {len(ends)} {tt.spread(ends)}")
        if viv_conn:
            print(f"  connections, placement / Vivado {len(viv_conn)} {tt.spread(viv_conn)}")
        if viv_ends:
            print(f"  endpoints, placement / Vivado {len(viv_ends)} {tt.spread(viv_ends)}")
        for (design, clock), v in sorted(clocks.items()):
            med = lambda i: tt.quantile([x[i] for x in v], 0.5)
            print(f"  clock {design} {clock} placement_latest {med(0):.3f} routed_latest {med(1):.3f} "
                  f"top20_shared {med(2):.2f} seeds {len(v)}")


if __name__ == "__main__":
    if len(sys.argv) > 3 and sys.argv[1] == "fit":
        fit(int(sys.argv[2]), sys.argv[3:])
    elif len(sys.argv) > 2 and sys.argv[1] == "compare":
        rest, db = sys.argv[2:], None
        if rest[:1] == ["--db"]:
            db, rest = rest[1], rest[2:]
        compare(db, [a.split("=", 1) for a in rest])
    else:
        sys.exit(__doc__)
