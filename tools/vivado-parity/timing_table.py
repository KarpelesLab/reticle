#!/usr/bin/env python3
"""Fits Reticle's 7-series delay model to Vivado's timer, and says how far each
model is from it, a connection and a clock at a time.

    timing_table.py fit <db artix7 dir> <out dir> <run>...
    timing_table.py compare <db artix7 dir> <name>=<run>...
    timing_table.py rust <classes.txt>

A run is what `TIMING=1 run_placed.sh` wrote: for each design,
<run>/<design>/routes/<design>.timing (`timing_dump.tcl`), Reticle's own
endpoints.txt, and x.tcl.

`fit` writes into <out dir>, for testdata/fpga/vivado/timing/:

    classes.txt   each class of wire a pip enters, how many connections of the
                  runs crossed one, and the fitted picoseconds; the charge for a
                  branch; and how much of a prjxray pip delay a wire of no
                  class here is charged
    arcs.txt      Vivado's delay of each cell arc, by cell, pins and type, the
                  median over every cell of the runs, with its range and count
    sample.txt    one connection in SAMPLE_EVERY: its design, Vivado's delay, its
                  branches, and its pips by tile type and wires with prjxray's
                  delay for each, what the test of the table walks

`rust` prints classes.txt as the table `src/fpga/xray/delay.rs` holds.

`compare` prints, for each run named, per design: the ratio of Reticle's
estimate of each connection to Vivado's (median, 5th and 95th percentile,
lowest, highest), and per clock the worst path of each tool. Only answers come
out: numbers about designs written here, timed on Reticle's routes.
"""
import collections, json, math, os, re, statistics, sys

SAMPLE_EVERY = 27
# A class crossed by fewer connections than this has no entry of its own.
MIN_COUNT = 10


def family(tile_type):
    """The tile type, without its side: what both sides' wires share."""
    t = re.sub(r"_W\d+$", "", tile_type)
    m = re.match(r"^[LR](IO[IB]\d+)", t)
    if m:
        return m[1]
    for base in ("CLBLL", "CLBLM", "BRAM", "DSP", "INT"):
        if re.fullmatch(rf"{base}_[LR]", t):
            return base
    return re.sub(r"_[LR]$", "", t)


def wire_class(tile_type, wire):
    """The class of wire `wire` of a tile of `tile_type`, which is what the
    table is keyed by. A slice's pins and outputs keep their number, which is
    which pin of the table it is and is a delay of its own; an interconnect
    track's number is dropped."""
    fam = family(tile_type)
    w = re.sub(r"_L(\d+)$", r"\1", wire)
    if fam in ("CLBLL", "CLBLM"):
        w = re.sub(r"^CLBL[LM]_", "", w)
        return f"{fam}:{w}"
    return f"{fam}:{re.sub(r'\d+$', '', w)}"


def split_pip(pip):
    """`TILE/TYPE.SRC->>DST` as (type, src, dst, buffered): `->>` is a
    buffered pip and `->` a fixed or pass connection."""
    _, rest = pip.split("/", 1)
    ttype, conn = rest.split(".", 1)
    src, arrow, dst = re.split(r"(<<->>|->>|->)", conn)
    return ttype, src, dst, arrow != "->"


def load(path):
    nets = collections.defaultdict(lambda: {"pips": [], "src": None, "sinks": []})
    arcs, paths = [], []
    for line in open(path):
        w = line.split()
        if not w:
            continue
        if w[0] == "pip" and len(w) == 5:
            nets[w[1]]["pips"].append((w[2], w[3], w[4]))
        elif w[0] == "source" and len(w) == 4:
            nets[w[1]]["src"] = (w[2], w[3])
        elif w[0] == "sink" and len(w) == 5 and w[4].lstrip("-").isdigit():
            nets[w[1]]["sinks"].append((w[2], w[3], int(w[4])))
        elif w[0] == "arc" and len(w) == 8:
            arcs.append(w[1:])
        elif w[0] == "path" and len(w) == 12:
            paths.append(w[1:])
    return nets, arcs, paths


def is_clock_net(chain):
    return any("GCLK" in p or "BUFHCLK" in p or "CLK_HROW" in p for p in chain)


def connections(nets):
    """(net, pin, ps, [pip...], branches, fanout) per sink a path reaches.

    `branches` is how many more branches than one leave the nodes on the
    connection's way, its source's included."""
    for name, net in nets.items():
        if not net["src"] or not net["pips"]:
            continue
        down = collections.defaultdict(list)
        for up, dn, pip in net["pips"]:
            down[up].append((dn, pip))
        start = net["src"][1]
        prev = {start: None}
        queue = [start]
        while queue:
            n = queue.pop()
            for dn, pip in down[n]:
                if dn not in prev:
                    prev[dn] = (n, pip)
                    queue.append(dn)
        for pin, node, ps in net["sinks"]:
            if node not in prev or node == start:
                continue
            chain, branches, n = [], 0, node
            while prev[n] is not None:
                n, pip = prev[n]
                chain.append(pip)
                branches += max(0, len(down[n]) - 1)
            chain.reverse()
            yield name, pin, ps, chain, branches, len(net["sinks"])


def cell_places(run, design, nets):
    """Each slice cell's place as `<M|L>.<bel kind>`: which slice it is in,
    read off the wires its pins are on, and its bel without the letter
    (`6LUT`, `5LUT`, `FF`, `5FF`)."""
    bel = {}
    for m in re.finditer(r"^\s+(\S+) (SLICE_\S+)/(\S+)$", open(f"{run}/{design}/x.tcl").read(), re.M):
        bel[m[1]] = re.sub(r"^[A-H]", "", m[3])
    kind = {}
    for net in nets.values():
        pins = list(net["sinks"]) + ([(net["src"][0], net["src"][1], 0)] if net["src"] else [])
        for pin, node, _ in pins:
            cell = pin.rsplit("/", 1)[0]
            m = re.search(r"/CLBL[LM]_(M|LL|L)_", node or "")
            if m:
                kind[cell] = "M" if m[1] == "M" else "L"
    return {c: f"{kind.get(c, '?')}.{b}" for c, b in bel.items()}


def designs_of(run):
    return sorted(d for d in os.listdir(run)
                  if os.path.isfile(f"{run}/{d}/routes/{d}.timing"))


def legacy_ps(db, ttype, src, dst, buffered):
    """What `DelayModel::Legacy` charges a pip: prjxray's delay, or for one it
    does not time 175 ps if it is switched and nothing if it is always on
    (`XrayDatabase::node_delays`; a `->` pip is Vivado's always-on kind)."""
    known = prjxray_delays(db, ttype).get((src, dst))
    if known is not None:
        return known
    return 175 if buffered else 0


def prjxray_delays(db, tile_type, cache={}):
    """prjxray's slow maximum of each pip of a tile type, in ps."""
    if tile_type not in cache:
        table = {}
        path = f"{db}/tile_type_{tile_type}.json"
        if os.path.exists(path):
            for pip in json.load(open(path))["pips"].values():
                timing = pip.get("src_to_dst") or {}
                delay = timing.get("delay")
                if delay:
                    table[(pip["src_wire"], pip["dst_wire"])] = round(float(delay[3]) * 1000)
        cache[tile_type] = table
    return cache[tile_type]


def gauss(a, b):
    """Solves a x = b, a square and well conditioned (the ridge sees to it)."""
    n = len(b)
    m = [row[:] + [b[i]] for i, row in enumerate(a)]
    for c in range(n):
        p = max(range(c, n), key=lambda r: abs(m[r][c]))
        m[c], m[p] = m[p], m[c]
        pivot = m[c][c]
        for r in range(c + 1, n):
            f = m[r][c] / pivot
            if f:
                row, top = m[r], m[c]
                for k in range(c, n + 1):
                    row[k] -= f * top[k]
    x = [0.0] * n
    for r in range(n - 1, -1, -1):
        x[r] = (m[r][n] - sum(m[r][k] * x[k] for k in range(r + 1, n))) / m[r][r]
    return x


def solve(rows, n, ridge=1e-3):
    """Least squares with every coefficient at least zero: `rows` is
    [(features dict, target)]. A coefficient the free solution makes negative
    is held at zero, the most negative first, until none is."""
    ata = [[0.0] * n for _ in range(n)]
    aty = [0.0] * n
    for features, y in rows:
        items = list(features.items())
        for i, a in items:
            aty[i] += a * y
            for j, b in items:
                ata[i][j] += a * b
    for i in range(n):
        ata[i][i] += ridge * (ata[i][i] + 1.0)
    free = list(range(n))
    while True:
        sub = gauss([[ata[i][j] for j in free] for i in free], [aty[i] for i in free])
        worst = min(range(len(free)), key=lambda k: sub[k])
        if sub[worst] >= 0:
            x = [0.0] * n
            for k, i in enumerate(free):
                x[i] = sub[k]
            return x
        del free[worst]


def quantile(values, q):
    values = sorted(values)
    if not values:
        return float("nan")
    k = (len(values) - 1) * q
    lo = math.floor(k)
    hi = min(lo + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (k - lo)


def spread(values):
    return (f"{statistics.median(values):.3f} {quantile(values, 0.05):.3f} "
            f"{quantile(values, 0.95):.3f} {min(values):.3f} {max(values):.3f}")


def grid(db, part="xc7a100t", cache={}):
    """Each tile's grid coordinates, which are what Reticle's distances count."""
    if part not in cache:
        tiles = json.load(open(f"{db}/{part}/tilegrid.json"))
        cache[part] = {name: (t["grid_x"], t["grid_y"]) for name, t in tiles.items()}
    return cache[part]


def wire_line(db, lines):
    """`timing::WIRE`'s three numbers from (tiles, fanout, ns): a least-squares
    line through the connections of signals of WIDE_FANOUT sinks or fewer, and
    the extra a tile the wider ones' take over it, as `reticle fpga` fits its
    own routes."""
    narrow = [(t, d) for t, f, d in lines if f <= WIDE_FANOUT]
    wide = [(t, d) for t, f, d in lines if f > WIDE_FANOUT]
    n = len(narrow)
    st = sum(t for t, _ in narrow)
    sd = sum(d for _, d in narrow)
    stt = sum(t * t for t, _ in narrow)
    std = sum(t * d for t, d in narrow)
    slope = (n * std - st * sd) / (n * stt - st * st)
    intercept = (sd - slope * st) / n
    covered = sum(t for t, _ in wide)
    extra = (sum(d for _, d in wide) - intercept * len(wide) - slope * covered) / covered
    return intercept, slope, extra


WIDE_FANOUT = 4


def fit(db, out, runs):
    samples = []
    lines = []
    at = grid(db)
    for run in runs:
        for design in designs_of(run):
            nets, _, _ = load(f"{run}/{design}/routes/{design}.timing")
            for net, pin, ps, chain, branches, fanout in connections(nets):
                if ps < 0 or is_clock_net(chain):
                    continue
                samples.append((design, pin, ps, [split_pip(p) for p in chain], branches))
                a = at.get(nets[net]["src"][1].split("/")[0])
                b = at.get(chain[-1].split("/")[0])
                if a and b and ps > 0:
                    lines.append((abs(a[0] - b[0]) + abs(a[1] - b[1]), fanout, ps / 1000.0))
    counts = collections.Counter()
    for _, _, _, chain, _ in samples:
        for ttype, src, dst, _ in chain:
            counts[wire_class(ttype, dst)] += 1
    named = sorted(k for k, n in counts.items() if n >= MIN_COUNT)
    index = {k: i for i, k in enumerate(named)}
    branch, other = len(named), len(named) + 1
    rows = []
    for _, _, ps, chain, branches in samples:
        features = collections.Counter()
        for ttype, src, dst, buffered in chain:
            key = wire_class(ttype, dst)
            if key in index:
                features[index[key]] += 1
            else:
                features[other] += prjxray_delays(db, ttype).get((src, dst), 0)
        if branches:
            features[branch] = branches
        rows.append((dict(features), ps))
    x = solve(rows, len(named) + 2)
    ratios = [sum(x[i] * a for i, a in features.items()) / ps for features, ps in rows if ps > 0]
    os.makedirs(out, exist_ok=True)
    with open(f"{out}/classes.txt", "w") as f:
        f.write("# Vivado 2026.1's interconnect delays at xc7a100tftg256-3, fitted per class of\n"
                "# the wire a pip enters; tools/vivado-parity/timing_table.py fit.\n")
        f.write(f"connections {len(rows)} fitted {spread(ratios)}\n")
        f.write(f"branch {x[branch]:.1f}\n")
        f.write(f"other {x[other]:.4f}\n")
        f.write("wire %.3f %.4f %.4f\n" % wire_line(db, lines))
        for k in named:
            f.write(f"class {k} {counts[k]} {x[index[k]]:.1f}\n")
    with open(f"{out}/sample.txt", "w") as f:
        f.write("# One connection in %d of the fit: its design, Vivado's ps, its branches, and\n"
                "# each pip as <tile type>.<wire it enters>.<prjxray's delay for it, or ->;\n"
                "# tools/vivado-parity/timing_table.py fit.\n" % SAMPLE_EVERY)
        for i, (design, pin, ps, chain, branches) in enumerate(samples):
            if i % SAMPLE_EVERY == 0:
                pips = [f"{t}.{dst}.{prjxray_delays(db, t).get((src, dst), '-')}"
                        for t, src, dst, _ in chain]
                f.write(f"{design} {ps} {branches} " + " ".join(pips) + "\n")
    arcs = collections.defaultdict(list)
    for run in runs:
        for design in designs_of(run):
            nets, found, _ = load(f"{run}/{design}/routes/{design}.timing")
            where = cell_places(run, design, nets)
            for cell, ref, frm, to, kind, rise, fall in found:
                try:
                    ns = max(float(rise), float(fall))
                except ValueError:
                    continue
                if ref.startswith(("DSP", "RAMB", "FIFO")):
                    frm, to = re.sub(r"\[\d+\]", "", frm), re.sub(r"\[\d+\]", "", to)
                arcs[(ref, where.get(cell, "-"), frm, to, kind)].append(ns)
    with open(f"{out}/arcs.txt", "w") as f:
        f.write("# Vivado 2026.1's cell arcs at xc7a100tftg256-3, slow corner's maximum in ns:\n"
                "# cell, where (slice kind and bel kind, or -), from, to, type, mean, median,\n"
                "# lowest, highest, count. A block's bus indices are folded together;\n"
                "# tools/vivado-parity/timing_table.py fit.\n")
        for key in sorted(arcs):
            v = arcs[key]
            f.write(f"{' '.join(key)} {statistics.fmean(v):.3f} {statistics.median(v):.3f} {min(v):.3f} {max(v):.3f} {len(v)}\n")
    print(f"{len(rows)} connections, {len(named)} classes, fitted {spread(ratios)}")


# Vivado spells a pin as the cell's; Reticle as the role the device file gives.
ROLE = {"D": "d", "CE": "en", "R": "rst", "S": "rst", "CLR": "rst", "PRE": "rst"}


def safe(name):
    return re.sub(r"[^A-Za-z0-9_]", "_", name)


def locks(run, design):
    found = {}
    path = f"{run}/{design}/x.tcl"
    for m in re.finditer(r"LOCK_PINS \{([^}]*)\} \[get_cells (\S+)\]", open(path).read()):
        found[m[2]] = dict(pair.split(":") for pair in m[1].split())
    return found


def vivado_key(pin, lock):
    """Vivado's `cell/PIN` as Reticle's (instance, role), or None for a pin
    that does not map one to one: lookup table and flip-flop inputs."""
    if pin.count("/") != 1:
        return None
    cell, name = pin.split("/")
    key = re.sub(r"^c\d+_", "", cell)
    if cell in lock and name in lock[cell]:
        return (re.sub(r"__O[56]$", "", key), "i" + str(int(lock[cell][name][1:]) - 1))
    if name in ROLE:
        return (key, ROLE[name])
    return None


# Connections Vivado puts under this are a carry chain's own and a slice's
# inside, a few picoseconds, where a ratio says nothing.
SHORTEST_NS = 0.02


def compare(db, named):
    """For each run and design: the ratio of Reticle's estimate to Vivado's,
    a connection and an endpoint at a time, and each clock's worst path.

    A connection is the same one when it ends on the same wire: Reticle's
    `connection` line names the wire its route reached the sink on, as
    `X<x>Y<y>/<wire>` in grid coordinates, and Vivado's sink is a node named by
    its tile and wire. A lookup table's pins are the router's to choose, so a
    cell's pin names do not say which connection is which."""
    name_at = {xy: name for name, xy in grid(db).items()}
    print("# Reticle's estimate against Vivado 2026.1's timer on the same routes, at\n"
          "# xc7a100tftg256-3; tools/vivado-parity/timing_table.py compare. Ratios are\n"
          "# estimate / Vivado: median, 5th and 95th percentile, lowest, highest.")
    for label, run in named:
        every = []
        for design in designs_of(run):
            nets, _, paths = load(f"{run}/{design}/routes/{design}.timing")
            lock = locks(run, design)
            theirs = {}
            for net, pin, ps, chain, _, _ in connections(nets):
                node = chain[-1].split("/")[0] + "/" + split_pip(chain[-1])[2]
                if ps / 1000.0 >= SHORTEST_NS and not is_clock_net(chain):
                    theirs[node] = max(theirs.get(node, 0), ps / 1000.0)
            ours, arrival = {}, {}
            text = open(f"{run}/{design}/endpoints.txt").read()
            for m in re.finditer(r"^connection (\S+) \d+ \S+ X(\d+)Y(\d+)/(\S+)$", text, re.M):
                tile = name_at.get((int(m[2]), int(m[3])))
                if tile:
                    ours[f"{tile}/{m[4]}"] = float(m[1])
            for m in re.finditer(r"^endpoint (\S+) (\S+) \S+ (.*)\.(\w+)$", text, re.M):
                key = (m[1], safe(m[3]), m[4])
                arrival[key] = max(arrival.get(key, 0.0), float(m[2]))
            ratios = [ours[k] / theirs[k] for k in ours if k in theirs]
            every += ratios
            print(f"design {design} {label}")
            if ratios:
                print(f"  connections {len(ratios)} {spread(ratios)}")
            # Each endpoint's worst path within one clock, as the period less
            # Vivado's slack would put it without the uncertainty: data path,
            # setup and the clock's skew, which is what Reticle's arrival with
            # its capture stands for, Reticle having no skew.
            vivado = {}
            for start, end, frm, to, data, logic, route, slack, skew, setup, levels in paths:
                if start != end:
                    continue
                key = vivado_key(to, {})
                try:
                    need = float(data) + float(setup) - float(skew)
                except ValueError:
                    continue
                if key:
                    vivado[(end,) + key] = (need, float(slack))
            both = [k for k in vivado if k in arrival]
            if both:
                ratios = [arrival[k] / vivado[k][0] for k in both if vivado[k][0] > 0]
                print(f"  endpoints {len(ratios)} {spread(ratios)}")
            for clock in sorted({k[0] for k in both}):
                mine = [k for k in both if k[0] == clock]
                worst = max(mine, key=lambda k: vivado[k][0])
                ours_worst = max(arrival[k] for k in mine)
                print(f"  clock {clock} vivado {vivado[worst][0]:.3f} slack {vivado[worst][1]:.3f} "
                      f"at {worst[1]}.{worst[2]} reticle_there {arrival[worst]:.3f} "
                      f"reticle_worst {ours_worst:.3f}")
        if every:
            print(f"all {label} connections {len(every)} {spread(every)}")


if __name__ == "__main__":
    if sys.argv[1] == "fit":
        fit(sys.argv[2], sys.argv[3], sys.argv[4:])
    elif sys.argv[1] == "rust":
        for line in open(sys.argv[2]):
            w = line.split()
            if w[0] == "class":
                print(f'    ("{w[1]}", {round(float(w[3]))}),')
            elif w[0] in ("branch", "other"):
                print(f"// {w[0]} {w[1]}")
    elif sys.argv[1] == "compare":
        compare(sys.argv[2], [a.split("=", 1) for a in sys.argv[3:]])
