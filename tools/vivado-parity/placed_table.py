#!/usr/bin/env python3
"""Reduces a run_placed.sh output directory to one committed file.

    tools/vivado-parity/placed_table.py <out> <vivado.txt> > placed.txt

For each design, one `design` line and under it:

    vivado_own <ns>             Vivado's implementation, from run_all's vivado.txt
    netlist <ns> <wns>          Reticle's netlist, Vivado's placement, routes, timer
    placement <ns> <wns>        Reticle's placement, Vivado's routes and timer
    routes <ns> <wns>           Reticle's routes, Vivado's timer
    second <ns> <wns>           the same at the part SECOND_PART named
    estimate <ns>               Reticle's own worst, over every endpoint it times
    same_endpoint <ns>          Reticle's estimate of the endpoint Vivado says is
                                worst on Reticle's routes
    endpoints <grade> <n> <median> <lo> <hi>
                                estimate / Vivado over the endpoints both time,
                                on Reticle's routes
    connections <grade> <n> <median> <lo> <hi> <fanout band: median...>
                                the same a connection at a time
    routes_set / _inside_a_site / _refused  how Vivado took the routes
    routes_nets_not_routed <n>  after route_design -preserve
    failed <step> and message   a step Vivado refused

Every `ns` is the data path of the clock named `clk`, and the worst path of
each tool need not be the same path, which `same_endpoint` is there for.
"""
import glob, os, re, statistics, sys

out, quoted = sys.argv[1:3]

own = {}
design = None
for line in open(quoted):
    if line.startswith("design "):
        design = line.split()[1]
    m = re.match(r"\s+clock clk period \S+ slack \S+ delay (\S+)", line)
    if m:
        own[design] = float(m[1])

print("# Vivado 2026.1's timer on Reticle's implementations; tools/vivado-parity/placed_table.py.")


def result(design, stage):
    path = f"{out}/{design}/{stage}/{design}.result"
    return open(path).read() if os.path.exists(path) else ""


def clock_line(text):
    return re.search(r"^clock clk period \S+ slack (\S+) delay (\S+)", text, re.M)


def safe(name):
    return re.sub(r"[^A-Za-z0-9_]", "_", name)


# Vivado spells a pin as the cell's; Reticle as the role the device file gives.
ROLE = {"D": "d", "CE": "en", "R": "rst", "S": "rst", "CLR": "rst", "PRE": "rst"}


def vivado_endpoints(design, stage):
    path = f"{out}/{design}/{stage}/{design}.endpoints"
    found = {}
    if not os.path.exists(path):
        return found
    for line in open(path):
        words = line.split()
        if len(words) < 5 or words[0] != "clk" or not words[3]:
            continue
        cell, _, pin = words[1].rpartition("/")
        cell = re.sub(r"^c\d+_", "", cell)
        try:
            found[(cell, ROLE.get(pin, pin))] = float(words[3])
        except ValueError:
            pass
    return found


def reticle_connections(design):
    found = {}
    path = f"{out}/{design}/endpoints.txt"
    if not os.path.exists(path):
        return found
    for line in open(path):
        m = re.match(r"connection (\S+) (\d+) (.*)\.(\w+)$", line)
        if m:
            found[(safe(m[3]), m[4])] = (float(m[1]), int(m[2]))
    return found


def locks(design):
    """Each lookup table's logical input to its physical one, from the placement."""
    found = {}
    for m in re.finditer(r"LOCK_PINS \{([^}]*)\} \[get_cells (\S+)\]", open(f"{out}/{design}/x.tcl").read()):
        found[m[2]] = dict(pair.split(":") for pair in m[1].split())
    return found


def vivado_connections(design, stage):
    """The slow corner's delay in ns by (cell, Reticle's role), lookup table
    inputs and flip-flop inputs only, which are the pins that map one to one."""
    path = f"{out}/{design}/{stage}/{design}.connections"
    found = {}
    if not os.path.exists(path):
        return found
    lock = locks(design)
    for line in open(path):
        words = line.split()
        if len(words) < 2 or words[0].count("/") != 1:
            continue
        cell, pin = words[0].split("/")
        key = re.sub(r"^c\d+_", "", cell)
        role = None
        if cell in lock and pin in lock[cell]:
            key = re.sub(r"__O[56]$", "", key)
            role = "i" + str(int(lock[cell][pin][1:]) - 1)
        elif pin in ROLE:
            role = ROLE[pin]
        if role is None:
            continue
        try:
            found[(key, role)] = max(found.get((key, role), 0.0), float(words[1]) / 1000.0)
        except ValueError:
            pass
    return found


def reticle_endpoints(design):
    found = {}
    path = f"{out}/{design}/endpoints.txt"
    if not os.path.exists(path):
        return found
    for line in open(path):
        m = re.match(r"endpoint clk (\S+) \S+ (.*)\.(\w+)$", line)
        if m:
            key = (safe(m[2]), m[3])
            found[key] = max(found.get(key, 0.0), float(m[1]))
    return found


for design in sorted(d for d in os.listdir(out) if os.path.isfile(f"{out}/{d}/reticle.log")):
    print(f"design {design}")
    if design in own:
        print(f"  vivado_own {own[design]:.3f}")
    log = open(f"{out}/{design}/reticle.log", errors="replace").read()
    for stage in ("netlist", "placement", "routes", "second"):
        text = result(design, stage)
        m = clock_line(text)
        if m:
            print(f"  {stage} {float(m[2]):.3f} {float(m[1]):.3f}")
        for keep in re.findall(r"^(?:failed|message|routes_set|routes_inside_a_site|routes_refused|nets_not_routed|routed_pips) .*$", text, re.M):
            if keep.startswith("routes_"):
                if stage == "routes":
                    print(f"  {keep}")
            elif stage == "routes" or keep.startswith(("failed", "message", "nets_not")):
                print(f"  {stage}_{keep}"[:300])
    late = re.findall(r"routed timing estimate:? .*?\bclk ([\d.]+) ns", log)
    if late:
        print(f"  estimate {late[-1]}")
    ours = reticle_endpoints(design)
    conn = reticle_connections(design)
    bands = ((1, 4), (5, 16), (17, 10**9))
    for stage, grade in (("routes", 1), ("second", 3)):
        theirs = vivado_endpoints(design, stage)
        both = [ours[k] / theirs[k] for k in ours if k in theirs and theirs[k] > 0]
        if both:
            print(f"  endpoints {grade} {len(both)} {statistics.median(both):.3f} {min(both):.3f} {max(both):.3f}")
            common = [k for k in theirs if k in ours]
            worst = max(common, key=lambda k: theirs[k])
            print(f"  same_endpoint {grade} {ours[worst]:.3f} vivado {theirs[worst]:.3f} at {worst[0]}.{worst[1]}")
        wires = vivado_connections(design, stage)
        pairs = [(conn[k][0] / wires[k], conn[k][1]) for k in conn if k in wires and wires[k] > 0]
        if pairs:
            ratios = [r for r, _ in pairs]
            text = " ".join(
                f"fanout{lo}+:{statistics.median(in_band):.3f}"
                for lo, hi in bands
                if (in_band := [r for r, f in pairs if lo <= f <= hi])
            )
            print(f"  connections {grade} {len(pairs)} {statistics.median(ratios):.3f} {min(ratios):.3f} {max(ratios):.3f} {text}")
