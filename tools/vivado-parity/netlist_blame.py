#!/usr/bin/env python3
"""Where a structural netlist's lookup tables go: Vivado's synthesised netlist
(`write_verilog` after `synth_design -flatten_hierarchy full`) or Reticle's
(`x.v` of a `--vivado-placement` build), read the same way.

    netlist_blame.py count <netlist.v>...
        Per netlist: tables by size, CARRY4s, the LUT1 buffers that feed a
        CARRY4's S pins and nothing else, and the S pins no table drives
        (which Vivado's placer gives a route-through).
    netlist_blame.py blame <netlist.v>
        Each table charged to the register groups its output reaches through
        tables, carries and wide multiplexers, split evenly where it reaches
        several, summed per group. A group is a register's name with its bit
        and copy indices taken off and the hierarchy flattened, so that
        Vivado's `gen_sad_counters[5].counter_incr_reg[5][3]` and Reticle's
        `c1545_counter_incr_5_ff_ff3` are both `counter_incr`.
    netlist_blame.py cone <netlist.v> <regex> [depth]
        The tables feeding each input of the cells whose name matches, down
        to anything that is not a table.

docs/fpga-vivado-parity.md, "Area against Vivado's", has what they found.
"""
import collections
import re
import sys

OUTPUTS = ('O', 'Q', 'CO', 'O5', 'O6', 'DOA', 'DOB', 'DOC', 'DOD', 'SPO', 'DPO', 'Q31',
           'DOADO', 'DOBDO', 'DOPADOP', 'DOPBDOP', 'P', 'PCOUT')
CELL = re.compile(r'^(LUT[1-6]|LUT6_2|CARRY4|FD\w+|MUXF[78]|RAM\w+|SRL\w+|DSP48E1)\s*'
                  r'(#\s*\((.*?)\)\s*)?(\\\S+|\w+)\s*\((.*)\)$')


def expand(value):
    """A pin's value with each `x[h:l]` written out bit by bit, MSB first."""
    parts = [x.strip() for x in value.strip('{}').split(',')] if value.startswith('{') else [value]
    out = []
    for x in parts:
        m = re.match(r'^(\\?\S+?)\s*\[(\d+):(\d+)\]$', x)
        if m:
            h, l = int(m.group(2)), int(m.group(3))
            step = -1 if h >= l else 1
            out += [f'{m.group(1)}[{i}]' for i in range(h, l + step, step)]
        else:
            out.append(x)
    return out


def cells(path):
    """(kind, name, {pin: [bits]}, INIT) of every primitive in the netlist."""
    for stmt in open(path).read().split(';'):
        stmt = re.sub(r'\(\*.*?\*\)', '', ' '.join(stmt.split())).strip()
        m = CELL.match(stmt)
        if not m:
            continue
        kind, params, name, pins = m.group(1), m.group(3) or '', m.group(4), m.group(5)
        p = {}
        for pm in re.finditer(r'\.(\w+)\s*\(', pins):
            start, depth, i = pm.end(), 1, pm.end()
            while depth:
                depth += {'(': 1, ')': -1}.get(pins[i], 0)
                i += 1
            p[pm.group(1)] = expand(pins[start:i - 1].strip())
        init = re.search(r'INIT\s*\(\s*([^)]*)\)', params)
        yield kind, name, p, init.group(1) if init else None


def wiring(cs):
    """Who drives each bit and who reads it."""
    driver, readers = {}, collections.defaultdict(list)
    for idx, (kind, _, p, _) in enumerate(cs):
        for pin, bits in p.items():
            for b in bits:
                if pin in OUTPUTS:
                    driver[b] = idx
                else:
                    readers[b].append((idx, pin))
    return driver, readers


def count(paths):
    for path in paths:
        cs = list(cells(path))
        driver, readers = wiring(cs)
        kinds = collections.Counter(k for k, _, _, _ in cs)
        tables = [i for i, c in enumerate(cs) if c[0].startswith('LUT')]
        buffers = sum(
            1 for i in tables
            if cs[i][0] == 'LUT1' and (cs[i][3] or '').lower().endswith('h2')
            and readers.get(cs[i][2].get('O', [''])[0])
            and all(cs[r][0] == 'CARRY4' and pin == 'S'
                    for r, pin in readers[cs[i][2]['O'][0]]))
        bare = sum(
            1 for k, _, p, _ in cs if k == 'CARRY4'
            for b in p.get('S', [])
            if not b.startswith(("1'", "4'", '\\<const'))
            and not (b in driver and cs[driver[b]][0].startswith('LUT')))
        sizes = ' '.join(f'LUT{n}={kinds.get(f"LUT{n}", 0)}' for n in range(1, 7))
        print(f'{path}: {len(tables)} tables ({sizes}), {kinds.get("CARRY4", 0)} CARRY4, '
              f'{buffers} LUT1 buffers to S, {bare} S pins no table drives')


def group(name):
    g = name.lstrip('\\').replace('/', '_').replace('.', '_')
    g = re.sub(r'^c\d+_', '', g)
    g = re.sub(r'\[\d+\]', '', g)
    g = re.sub(r'_reg.*', '', g)
    g = re.sub(r'gen_sad_counters_', '', g)
    return re.sub(r'_\d+_ff.*|_ff.*', '', g)


def blame(path):
    cs = list(cells(path))
    _, readers = wiring(cs)
    memo = {}

    def sinks(idx, stack):
        if idx in memo:
            return memo[idx]
        kind, name, p, _ = cs[idx]
        if kind.startswith(('FD', 'RAM', 'SRL', 'DSP')):
            return {group(name)}
        if idx in stack:
            return set()
        got = set()
        for pin, bits in p.items():
            if pin in OUTPUTS:
                for b in bits:
                    for r, _ in readers.get(b, []):
                        got |= sinks(r, stack | {idx})
        memo[idx] = got or {'(a port or nothing)'}
        return memo[idx]

    total = collections.Counter()
    for idx, (kind, _, _, _) in enumerate(cs):
        if kind.startswith('LUT'):
            s = sinks(idx, frozenset())
            for g in s:
                total[g] += 1.0 / len(s)
    for g, v in sorted(total.items(), key=lambda x: (-x[1], x[0])):
        print(f'{v:8.1f} {g}')


def cone(path, pattern, depth):
    cs = list(cells(path))
    driver, _ = wiring(cs)
    seen = set()

    def show(bit, d, indent):
        idx = driver.get(bit)
        if idx is None:
            print(' ' * indent + bit)
            return
        kind, name, p, init = cs[idx]
        if not kind.startswith(('LUT', 'MUXF')):
            print(' ' * indent + f'{bit} <- {kind} {name}')
            return
        print(' ' * indent + f'{bit} <- {kind} {init} {name}' + (' (again)' if idx in seen else ''))
        if idx in seen or d >= depth:
            return
        seen.add(idx)
        for pin in sorted(p):
            if pin.startswith('I') or pin == 'S':
                for b in p[pin]:
                    show(b, d + 1, indent + 2)

    for kind, name, p, _ in cs:
        if re.search(pattern, name):
            print(f'== {kind} {name}')
            for pin in sorted(p):
                if pin not in OUTPUTS:
                    for b in p[pin]:
                        print(f' {pin}:')
                        show(b, 0, 2)


if __name__ == '__main__':
    sys.setrecursionlimit(100000)
    what = sys.argv[1] if len(sys.argv) > 1 else ''
    if what == 'count':
        count(sys.argv[2:])
    elif what == 'blame':
        blame(sys.argv[2])
    elif what == 'cone':
        cone(sys.argv[2], sys.argv[3], int(sys.argv[4]) if len(sys.argv) > 4 else 6)
    else:
        sys.exit(__doc__)
