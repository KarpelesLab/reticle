#!/usr/bin/env python3
"""Fits Reticle's 7-series delay model to Vivado's timer, and says how far each
model is from it, a connection and a clock at a time.

    timing_table.py fit <db artix7 dir> [--part <part>] <out dir> <run>...
    timing_table.py compare <db artix7 dir> [--part <part>] <name>=<run>...
    timing_table.py rust <classes_1.txt> <classes_2.txt> <classes.txt>
    timing_table.py branches <db artix7 dir> <classes dir> <out dir> <regrade out>...

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

`fit --part` names the part the runs were timed at in the files' headers
(xc7a100tftg256-3 unless given): the same runs given to Vivado again at another
speed grade (`regrade.sh`) are fitted the same way, a table for each grade.

`rust` prints the three grades' classes files, grade 1's first, as the table
`src/fpga/xray/delay.rs` holds: each class with its picoseconds at grades 1, 2
and 3. The three are fits of the same connections, so they name the same
classes; a class one of them lacks is refused.

`branches` fits what the branches leaving a wire load a connection through it
with, the classes held as `<classes dir>`'s `classes.txt`, `classes_2.txt` and
`classes_1.txt` have them (rounded as `xray::delay` keeps them): nothing for one
branch, `first` for the second and `more` for each after. Each <regrade out> is
what `regrade.sh` wrote for the parts xc7a100tftg256-3, -2 and -1, the same
routes at each grade. Into <out dir>:

    branches.txt         each grade's fit, the line through the same
                         connections, and the model's error by the signal's
                         fanout with the classes' own charge a branch and with
                         the fit, in ps
    branches_sample.txt  one connection in BRANCH_SAMPLE_EVERY: its design, its
                         fanout, Vivado's ps at grades 3, 2 and 1, the degree of
                         each node it leaves, and its pips as sample.txt has
                         them; what the test of the fit walks

`compare` prints, for each run named, per design: the ratio of Reticle's
estimate of each connection to Vivado's (median, 5th and 95th percentile,
lowest, highest), and per clock the worst path of each tool. The runs are of
xc7a100tftg256-3 unless --part names another (xc7a35tftg256-3), whose die's
grid, Project X-Ray's (xc7a50t for an xc7a35t), places Reticle's wires. Only answers come
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
    for name, pin, ps, chain, degrees, fanout in connection_degrees(nets):
        yield name, pin, ps, chain, sum(max(0, k - 1) for k in degrees), fanout


def connection_degrees(nets):
    """(net, pin, ps, [pip...], [degree...], fanout) per sink a path reaches:
    `degrees` is how many branches leave each node the connection's pips
    leave, its source's first."""
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
            chain, degrees, n = [], [], node
            while prev[n] is not None:
                n, pip = prev[n]
                chain.append(pip)
                degrees.append(len(down[n]))
            chain.reverse()
            degrees.reverse()
            yield name, pin, ps, chain, degrees, len(net["sinks"])


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


def fit(db, out, runs, part="xc7a100tftg256-3"):
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
        f.write(f"# Vivado 2026.1's interconnect delays at {part}, fitted per class of\n"
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
        f.write(f"# Vivado 2026.1's cell arcs at {part}, slow corner's maximum in ns:\n"
                "# cell, where (slice kind and bel kind, or -), from, to, type, mean, median,\n"
                "# lowest, highest, count. A block's bus indices are folded together;\n"
                "# tools/vivado-parity/timing_table.py fit.\n")
        for key in sorted(arcs):
            v = arcs[key]
            f.write(f"{' '.join(key)} {statistics.fmean(v):.3f} {statistics.median(v):.3f} {min(v):.3f} {max(v):.3f} {len(v)}\n")
    print(f"{len(rows)} connections, {len(named)} classes, fitted {spread(ratios)}")


GRADE_PARTS = ("xc7a100tftg256-3", "xc7a100tftg256-2", "xc7a100tftg256-1")
# branches_sample.txt keeps one connection in this many: their routes are
# longer than sample.txt's, a wide signal's, so one in twice as many.
BRANCH_SAMPLE_EVERY = 54
FANOUT_BUCKETS = ((1, 4), (5, 16), (17, 64), (65, 128), (129, 10 ** 9))


def class_table(path):
    """A classes file as `xray::delay` keeps it: each class's cost rounded to
    the picosecond, the share of prjxray's delay in thousandths, and the
    charge a branch the classes were fitted beside."""
    table, other, branch = {}, 0, 0.0
    for line in open(path):
        w = line.split()
        if w and w[0] == "class":
            table[w[1]] = round(float(w[3]))
        elif w and w[0] == "other":
            other = round(float(w[1]) * 1000)
        elif w and w[0] == "branch":
            branch = float(w[1])
    return table, other, branch


def wires_ps(db, table, other, chain):
    """What `xray::delay` charges a connection's wires: each one's class, or a
    share of prjxray's delay for the pip into it."""
    total = 0
    for ttype, src, dst, _ in chain:
        cost = table.get(wire_class(ttype, dst))
        if cost is None:
            known = prjxray_delays(db, ttype).get((src, dst))
            cost = known * other // 1000 if known is not None else 0
        total += cost
    return total


def branches(db, classes_dir, out, regraded):
    """Fits the branches' load with the classes held, at each grade."""
    tables = [class_table(f"{classes_dir}/{name}")
              for name in ("classes.txt", "classes_2.txt", "classes_1.txt")]
    found = []
    for root in regraded:
        for design in designs_of(f"{root}/{GRADE_PARTS[0]}"):
            by_grade = []
            for part in GRADE_PARTS:
                path = f"{root}/{part}/{design}/routes/{design}.timing"
                if not os.path.isfile(path):
                    break
                nets, _, _ = load(path)
                by_grade.append({(net, pin): (ps, chain, degrees, fanout)
                                 for net, pin, ps, chain, degrees, fanout in connection_degrees(nets)})
            if len(by_grade) < 3:
                continue
            for key, (ps, chain, degrees, fanout) in sorted(by_grade[0].items()):
                if ps < 0 or is_clock_net(chain) or any(key not in g for g in by_grade[1:]):
                    continue
                found.append((design, fanout, [g[key][0] for g in by_grade],
                              [split_pip(p) for p in chain], degrees))
    os.makedirs(out, exist_ok=True)
    with open(f"{out}/branches.txt", "w") as f:
        f.write("# Vivado 2026.1's interconnect delays of Reticle's routes at xc7a100tftg256-3,\n"
                "# -2 and -1, against the classes' own figures: what the branches leaving a\n"
                "# wire add, nothing for one, `first` for the second and `more` for each after;\n"
                "# `linear` one figure a branch, fitted to the same connections, and\n"
                "# `classes` the one the classes were fitted beside. Then the model's error\n"
                "# (model less Vivado, ps, mean and root mean square) by the signal's fanout,\n"
                "# with the classes' charge and with the fit. tools/vivado-parity/timing_table.py\n"
                "# branches.\n")
        for g, (table, other, classes_branch) in enumerate(tables):
            grade = 3 - g
            rows, lines, errors = [], [], []
            for design, fanout, ps, chain, degrees in found:
                rest = ps[g] - wires_ps(db, table, other, chain)
                rows.append(({0: sum(1 for k in degrees if k >= 2), 1: sum(k - 2 for k in degrees if k >= 2)}, rest))
                lines.append(({0: sum(k - 1 for k in degrees if k >= 2)}, rest))
                errors.append((fanout, rest, degrees))
            first, more = solve(rows, 2, ridge=0.0)
            linear = solve(lines, 1, ridge=0.0)[0]
            f.write(f"grade {grade} connections {len(rows)} first {first:.1f} more {more:.1f} "
                    f"linear {linear:.1f} classes {classes_branch:.1f}\n")
            for lo, hi in FANOUT_BUCKETS:
                old, new = [], []
                for fanout, rest, degrees in errors:
                    if lo <= fanout <= hi:
                        old.append(classes_branch * sum(k - 1 for k in degrees if k >= 2) - rest)
                        new.append(sum(first + more * (k - 2) for k in degrees if k >= 2) - rest)
                if old:
                    rms = lambda v: math.sqrt(sum(x * x for x in v) / len(v))
                    f.write(f"error {grade} fanout {lo}-{hi if hi < 10 ** 9 else ''} n {len(old)} "
                            f"classes {statistics.fmean(old):+.1f} {rms(old):.1f} "
                            f"fit {statistics.fmean(new):+.1f} {rms(new):.1f}\n")
    with open(f"{out}/branches_sample.txt", "w") as f:
        f.write("# One connection in %d of branches.txt's: its design, its signal's fanout,\n"
                "# Vivado's ps at grades 3, 2 and 1, the branches leaving each node it\n"
                "# leaves (its source's first), and each pip as <tile type>.<wire it\n"
                "# enters>.<prjxray's delay for it, or ->; tools/vivado-parity/timing_table.py\n"
                "# branches.\n" % BRANCH_SAMPLE_EVERY)
        for i, (design, fanout, ps, chain, degrees) in enumerate(found):
            if i % BRANCH_SAMPLE_EVERY == 0:
                pips = [f"{t}.{dst}.{prjxray_delays(db, t).get((src, dst), '-')}" for t, src, dst, _ in chain]
                f.write(f"{design} {fanout} {ps[0]} {ps[1]} {ps[2]} {','.join(map(str, degrees))} "
                        + " ".join(pips) + "\n")
    print(f"{len(found)} connections")


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


def die_grid(part):
    """Project X-Ray's grid directory for a part: the xc7a35t is the xc7a50t's."""
    die = part[:part.index("t") + 1]
    return "xc7a50t" if die == "xc7a35t" else die


def compare(db, named, part="xc7a100tftg256-3"):
    """For each run and design: the ratio of Reticle's estimate to Vivado's,
    a connection and an endpoint at a time, and each clock's worst path.

    A connection is the same one when it ends on the same wire: Reticle's
    `connection` line names the wire its route reached the sink on, as
    `X<x>Y<y>/<wire>` in grid coordinates, and Vivado's sink is a node named by
    its tile and wire. A lookup table's pins are the router's to choose, so a
    cell's pin names do not say which connection is which."""
    name_at = {xy: name for name, xy in grid(db, die_grid(part)).items()}
    print("# Reticle's estimate against Vivado 2026.1's timer on the same routes, at\n"
          f"# {part}; tools/vivado-parity/timing_table.py compare. Ratios are\n"
          "# estimate / Vivado: median, 5th and 95th percentile, lowest, highest.")
    for label, run in named:
        every, ends = [], []
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
                ends += ratios
            for clock in sorted({k[0] for k in both}):
                mine = [k for k in both if k[0] == clock]
                worst = max(mine, key=lambda k: vivado[k][0])
                ours_worst = max(arrival[k] for k in mine)
                print(f"  clock {clock} vivado {vivado[worst][0]:.3f} slack {vivado[worst][1]:.3f} "
                      f"at {worst[1]}.{worst[2]} reticle_there {arrival[worst]:.3f} "
                      f"reticle_worst {ours_worst:.3f}")
        if every:
            print(f"all {label} connections {len(every)} {spread(every)}")
        if ends:
            print(f"all {label} endpoints {len(ends)} {spread(ends)}")


if __name__ == "__main__":
    if sys.argv[1] == "fit":
        rest = sys.argv[3:]
        part = "xc7a100tftg256-3"
        if rest[:1] == ["--part"]:
            part, rest = rest[1], rest[2:]
        fit(sys.argv[2], rest[0], rest[1:], part)
    elif sys.argv[1] == "rust":
        grades = []
        for path in sys.argv[2:5]:
            table = {}
            for line in open(path):
                w = line.split()
                if w[0] == "class":
                    table[w[1]] = round(float(w[3]))
                elif w[0] in ("branch", "other", "wire"):
                    print(f"// {path}: {' '.join(w)}")
            grades.append(table)
        if len({tuple(sorted(t)) for t in grades}) != 1:
            sys.exit("the three fits do not name the same classes")
        for name in sorted(grades[0]):
            print(f'    ("{name}", [{", ".join(str(t[name]) for t in grades)}]),')
    elif sys.argv[1] == "branches":
        branches(sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5:])
    elif sys.argv[1] == "compare":
        rest = sys.argv[3:]
        part = "xc7a100tftg256-3"
        if rest[:1] == ["--part"]:
            part, rest = rest[1], rest[2:]
        compare(sys.argv[2], [a.split("=", 1) for a in rest], part)
