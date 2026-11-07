"""Compares Reticle's routes with Vivado's on the same placement, a connection
at a time, from two routes.tcl dumps of one build: <vivado> with Vivado's
routes (the pins free) and <reticle> with Reticle's, both timed by Vivado.

    routes.py summary <within ns> <vivado> <reticle> [<vivado> <reticle>...]
        Over the connections on Reticle's paths within <within> of each
        clock's latest: how many, each tool's mean interconnect delay, how
        many end on another input of their table than Vivado's, and what each
        kind costs Reticle more.
    routes.py show <within ns> <vivado> <reticle>
        Those connections one by one, the worst first, with the wires each
        tool's route takes.
    routes.py pins <within ns> <vivado> <reticle> [...]
        Which input of a table each tool ends those connections on.
    routes.py connections <dump>...
        The `c <driver site> <sink site> <fanout> <ps>` lines `fill.py
        distance` reads, for every connection between two slices.
"""
import re
import sys
from collections import Counter


def paths_of(route):
    """Each node of a ROUTE tree with the nodes from the root to it."""
    out, stack, cur = {}, [], []
    for t in re.findall(r'\{|\}|[^\s{}]+', route):
        if t == '{':
            stack.append(list(cur))
        elif t == '}':
            cur = stack.pop() if stack else []
        else:
            cur = cur + [t]
            out.setdefault(t, cur)
    return out


def load(fn):
    """(nets, sinks by (net, cell pin), paths) of one dump."""
    nets, sinks, paths, net = {}, {}, [], None
    for line in open(fn):
        head, _, rest = line.partition(' ')
        f = line.split()
        if head == 'n' and len(f) >= 4:
            net = f[1]
            nets[net] = {'fo': int(f[2]), 'drv': f[3]}
        elif head == 'r' and net in nets:
            nets[net]['route'] = paths_of(rest)
        elif head == 's' and len(f) >= 6 and f[1] in nets:
            n = nets[f[1]]
            sinks[(f[1], f[-1])] = {'site': f[2], 'delay': float(f[-2]), 'fo': n['fo'],
                                    'drv': n['drv'], 'nodes': n.get('route', {}).get(f[3], [])}
        elif head == 'p':
            paths.append((f[1], float(f[2]), f[3:]))
    return nets, sinks, paths


def wire_class(node):
    """A node's wire without its tile and track: `INT_L_X4Y5/NN6BEG1` is NN6BEG."""
    tile, _, wire = node.partition('/')
    if tile.startswith('CLB'):
        return re.sub(r'^CLBL[LM]_', '', wire)
    return re.sub(r'_L$', '', re.sub(r'\d+$', '', wire))


def critical(viv, ret, within):
    """The connections on Reticle's paths within `within` ns of each clock's
    latest, with Reticle's and Vivado's figures for each."""
    worst = {}
    for clock, slack, _ in ret[2]:
        worst[clock] = min(worst.get(clock, 1e9), slack)
    pins = set()
    for clock, slack, p in ret[2]:
        if slack <= worst[clock] + within:
            pins.update(p)
    return [(key, r, viv[1][key]) for key, r in ret[1].items()
            if key[1] in pins and key in viv[1]]


def pairs(args):
    return [(load(a), load(b)) for a, b in zip(args[::2], args[1::2])]


def summary(within, args):
    tot = Counter()
    for (viv, ret), name in zip(pairs(args), args[1::2]):
        c = Counter()
        for _, r, v in critical(viv, ret, within):
            same = r['site'] == v['site']
            c['n'] += 1
            c['R'] += r['delay']
            c['V'] += v['delay']
            c['same' if same else 'other'] += 1
            c['same_d' if same else 'other_d'] += r['delay'] - v['delay']
        tot.update(c)
        line(name, c)
    line('all', tot)


def line(name, c):
    n = max(1, c['n'])
    print(f"{name}: {c['n']} connections, Reticle {c['R'] / n:.0f} ps, Vivado {c['V'] / n:.0f}; "
          f"same input {c['same']} ({c['same_d'] / max(1, c['same']):+.0f}), "
          f"another {c['other']} ({c['other_d'] / max(1, c['other']):+.0f})")


def show(within, args):
    (viv, ret), = pairs(args)
    rows = sorted(critical(viv, ret, within), key=lambda t: t[2]['delay'] - t[1]['delay'])
    for key, r, v in rows:
        print(f"{r['delay'] - v['delay']:+5.0f} {key[0]} -> {key[1]} (fanout {r['fo']}), "
              f"{r['drv']} to {r['site']}, Vivado's to {v['site']}")
        for who, x in (('R', r), ('V', v)):
            print(f"   {who} {x['delay']:4.0f} {' '.join(wire_class(n) for n in x['nodes'][2:-1])}")


def pins(within, args):
    cr, cv, dr, dv = Counter(), Counter(), Counter(), Counter()
    for viv, ret in pairs(args):
        for _, r, v in critical(viv, ret, within):
            for x, c, d in ((r, cr, dr), (v, cv, dv)):
                p = x['site'].split('/')[-1]
                if len(p) == 2 and p[1].isdigit():
                    c[p[1]] += 1
                    d[p[1]] += x['delay']
    print('input: Reticle (mean ps), Vivado (mean ps)')
    for p in '123456':
        print(f"{p}: {cr[p]} ({dr[p] / max(1, cr[p]):.0f}), {cv[p]} ({dv[p] / max(1, cv[p]):.0f})")


def connections(files):
    for fn in files:
        nets, sinks, _ = load(fn)
        for s in sinks.values():
            d, k = s['drv'].split('/')[0], s['site'].split('/')[0]
            if d.startswith('SLICE_') and k.startswith('SLICE_'):
                print('c', d, k, s['fo'], f"{s['delay']:g}")


if __name__ == '__main__':
    what, rest = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 2 else (None, [])
    if what == 'summary':
        summary(float(rest[0]), rest[1:])
    elif what == 'show':
        show(float(rest[0]), rest[1:])
    elif what == 'pins':
        pins(float(rest[0]), rest[1:])
    elif what == 'connections':
        connections(rest)
    else:
        sys.exit(__doc__)
