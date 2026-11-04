#!/usr/bin/env python3
"""Sweeps a clock manager's parameters through Vivado, and keeps what it wrote.

    cmt_sweep.py lists <dir>          # writes <dir>/<kind>.<n>.list, shards to run
    cmt_sweep.py table <dir> <reticle> <fixture>

`lists` says what to ask. `cmt_sweep.sh` gives each shard to `cmt_sweep.tcl`, on
the checkpoints `clocks.sh` left of `mmcm_all_outs` and `pll_all_outs`.
`table` reads every `.bit` back with `reticle fpga --decode` and writes one line
per run, `<kind> <PARAM=value ...> => <FIELD=value ...>`, with only the register
fields that are not zero: `tests/fpga_xray_cmt_sweep.rs` holds Reticle to it, and
the lines Vivado refused say so.
"""
import concurrent.futures as cf
import hashlib
import os
import re
import subprocess
import sys

SHARDS = 8


def num(v):
    return ("%.3f" % v).rstrip("0").rstrip(".") if isinstance(v, float) else str(v)


def lines():
    out = []

    def add(kind, **params):
        text = " ".join(f"{k}={v}" for k, v in params.items())
        tag = f"{kind}_{hashlib.sha1(text.encode()).hexdigest()[:12]}"
        if not any(x[0] == tag for x in out):
            out.append((tag, kind, params))

    for kind, mult_p, div0_p, max_div, more in (
        ("mmcm", "CLKFBOUT_MULT_F", "CLKOUT0_DIVIDE_F", 106, (1, 5, 6)),
        ("pll", "CLKFBOUT_MULT", "CLKOUT0_DIVIDE", 56, (1, 5)),
    ):
        vco = 900.0 if kind == "mmcm" else 1200.0
        # A real number of Vivado's for a multiplier that is one: "8.000".
        as_mult = (lambda m: "%.3f" % m) if kind == "mmcm" else (lambda m: str(int(m)))
        # Each multiplier under each bandwidth: the two tables.
        for mult in range(2, 65):
            period = 1000.0 * mult / vco
            for band in ("LOW", "HIGH", "OPTIMIZED"):
                add(kind, **{mult_p: as_mult(mult), "CLKIN1_PERIOD": "%.3f" % period, "BANDWIDTH": band})
        # Whether the input's period moves a table: it would for an optimiser.
        for period in (3.0, 5.0, 10.0, 20.0):
            add(kind, **{mult_p: as_mult(8), "CLKIN1_PERIOD": num(period)})
        # Each edge of the VCO's window, either side, to a percent.
        lo, hi = (600.0, 1200.0) if kind == "mmcm" else (800.0, 1600.0)
        for vco_at in (lo * 0.99, lo * 1.01, hi * 0.99, hi * 1.01):
            add(kind, **{mult_p: as_mult(8), "CLKIN1_PERIOD": "%.3f" % (8000.0 / vco_at)})
        # The input divider.
        for div in range(1, max_div + 1):
            pfd = 12.0 if kind == "mmcm" else 20.0
            mult = 64
            add(
                kind,
                **{
                    "DIVCLK_DIVIDE": str(div),
                    mult_p: as_mult(mult),
                    "CLKIN1_PERIOD": "%.3f" % (1000.0 / (pfd * div)),
                },
            )
        # Each output's division, duty cycle and phase, on the outputs that are
        # spelt differently.
        for n in more:
            for div in (1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128):
                add(kind, **{f"CLKOUT{n}_DIVIDE": str(div)})
            for div in (2, 3, 4, 5, 8, 9, 32, 33, 128):
                for duty in (0.25, 0.33, 0.4, 0.6, 0.75):
                    add(kind, **{f"CLKOUT{n}_DIVIDE": str(div), f"CLKOUT{n}_DUTY_CYCLE": num(duty)})
            for div in (1, 2, 3, 5, 8, 16, 17, 33, 64):
                step = 45.0 / div
                for k in (1, 2, 3, 5, 8, 11, 16, 31, 63, 100):
                    phase = step * k
                    if phase < 360.0:
                        add(kind, **{f"CLKOUT{n}_DIVIDE": str(div), f"CLKOUT{n}_PHASE": "%.3f" % phase})
            for phase in (-45.0, -90.0, -180.0, -22.5):
                add(kind, **{f"CLKOUT{n}_DIVIDE": "8", f"CLKOUT{n}_PHASE": num(phase)})
            for div, phase, duty in ((8, 45.0, 0.25), (9, 40.0, 0.5556), (7, 90.0, 0.25), (16, 11.25, 0.75)):
                add(kind, **{f"CLKOUT{n}_DIVIDE": str(div), f"CLKOUT{n}_PHASE": num(phase), f"CLKOUT{n}_DUTY_CYCLE": num(duty)})
        # The feedback counter's phase, and the first output's.
        for phase in (5.625, 11.25, 22.5, 45.0, 90.0, 180.0, 270.0, 359.0):
            add(kind, **{"CLKFBOUT_PHASE": num(phase)})
            add(kind, **{"CLKOUT0_PHASE": num(phase)})
        for duty in (0.25, 0.33, 0.75):
            add(kind, **{"CLKOUT0_DUTY_CYCLE": num(duty)})
    # What only an MMCM has: fractional dividers, in steps of an eighth.
    for whole in (2, 3, 8, 9, 16, 63):
        for eighth in range(8):
            add("mmcm", CLKOUT0_DIVIDE_F="%.3f" % (whole + eighth / 8.0))
            mult = whole + eighth / 8.0
            add("mmcm", CLKFBOUT_MULT_F="%.3f" % mult, CLKIN1_PERIOD="%.3f" % (1000.0 * mult / 900.0))
    for div, mult in ((8.125, 8.125), (8.5, 8.5), (9.25, 8.5), (2.125, 9.875), (7.875, 6.875), (3.5, 15.5)):
        add("mmcm", CLKOUT0_DIVIDE_F="%.3f" % div, CLKFBOUT_MULT_F="%.3f" % mult, CLKIN1_PERIOD="%.3f" % (1000.0 * mult / 900.0))
    for div, phase in ((8.125, 5.538), (8.125, 11.077), (8.125, 22.154), (8.125, 44.308), (8.5, 5.294), (8.5, 21.176), (8.25, 5.455), (8.375, 5.373), (7.875, 5.714)):
        add("mmcm", CLKOUT0_DIVIDE_F="%.3f" % div, CLKOUT0_PHASE="%.3f" % phase)
    for mult, phase in ((8.125, 5.538), (8.125, 11.077), (8.5, 5.294)):
        add("mmcm", CLKFBOUT_MULT_F="%.3f" % mult, CLKFBOUT_PHASE="%.3f" % phase, CLKIN1_PERIOD="%.3f" % (1000.0 * mult / 900.0))
    # Fine phase shift on each output, and the cascade.
    for n in ("CLKOUT0", "CLKOUT1", "CLKOUT2", "CLKOUT3", "CLKOUT4", "CLKOUT5", "CLKOUT6", "CLKFBOUT"):
        add("mmcm", **{f"{n}_USE_FINE_PS": "TRUE"})
    add("mmcm", CLKOUT0_USE_FINE_PS="TRUE", CLKOUT1_USE_FINE_PS="TRUE", CLKFBOUT_USE_FINE_PS="TRUE")
    add("mmcm", CLKOUT1_USE_FINE_PS="TRUE", CLKOUT1_PHASE="45.000", CLKOUT1_DIVIDE="8")
    add("mmcm", CLKOUT1_USE_FINE_PS="TRUE", CLKOUT1_PHASE="11.250", CLKOUT1_DIVIDE="8")
    for div, phase in ((8, 5.625), (8, 22.5), (8, 33.75), (8, 90.0), (16, 45.0), (16, 5.625), (9, 40.0), (9, 10.0)):
        add("mmcm", CLKOUT1_USE_FINE_PS="TRUE", CLKOUT1_PHASE="%.3f" % phase, CLKOUT1_DIVIDE=str(div))
    for phase in (5.625, 22.5, 90.0):
        add("mmcm", CLKOUT0_USE_FINE_PS="TRUE", CLKOUT0_PHASE="%.3f" % phase)
        add("mmcm", CLKFBOUT_USE_FINE_PS="TRUE", CLKFBOUT_PHASE="%.3f" % phase)
    add("mmcm", CLKOUT4_CASCADE="TRUE", CLKOUT6_DIVIDE="2", CLKOUT4_DIVIDE="4")
    add("mmcm", CLKOUT4_CASCADE="TRUE", CLKOUT6_DIVIDE="3", CLKOUT4_DIVIDE="5")
    # Spread spectrum, each mode.
    for mode in ("CENTER_HIGH", "CENTER_LOW", "DOWN_HIGH", "DOWN_LOW"):
        for period in (4000, 10000, 40000):
            add("mmcm", SS_EN="TRUE", SS_MODE=mode, SS_MOD_PERIOD=str(period), BANDWIDTH="LOW", CLKOUT5_DIVIDE="1", CLKOUT6_DIVIDE="1")
    return out


def main():
    mode = sys.argv[1]
    root = sys.argv[2]
    all_lines = lines()
    if mode == "lists":
        os.makedirs(root, exist_ok=True)
        for kind in ("mmcm", "pll"):
            these = [x for x in all_lines if x[1] == kind]
            for s in range(SHARDS):
                with open(f"{root}/{kind}.{s}.list", "w") as f:
                    for tag, _, params in these[s::SHARDS]:
                        f.write(tag + " " + " ".join(f"{k} {v}" for k, v in params.items()) + "\n")
        print(len(all_lines), "runs")
        return
    reticle, fixture = sys.argv[3], sys.argv[4]
    logs = {}

    def one(item):
        tag, kind, params = item
        name = " ".join(f"{k}={v}" for k, v in params.items())
        refused = f"{root}/{kind}/{tag}.refused"
        if os.path.exists(refused):
            # The rule is in the log, between this run's mark and the next.
            rules = set()
            for shard in range(SHARDS):
                log = logs.setdefault(shard, {})
                if kind not in log:
                    log[kind] = open(f"{root}/{kind}.{shard}.list.log", errors="replace").read()
                text = log[kind]
                at = text.find(f"RUN {tag}\n")
                if at < 0:
                    continue
                end = text.find("\nRUN ", at + 1)
                rules.update(re.findall(r"(?:ERROR|CRITICAL WARNING): \[(?:DRC )?([A-Za-z_]+[ -][0-9-]+)\]", text[at:end]))
                break
            rules -= {"Common 17-39", "Vivado 12-1345"}
            kept = " ".join(sorted(r.replace(" ", "-") for r in rules)) or "unknown"
            return kind, name, f"refused {kept}", []
        bit = f"{root}/{kind}/{tag}.bit"
        if not os.path.exists(bit):
            return kind, name, "refused unknown", []
        run = subprocess.run(
            ["nice", "-n", "19", reticle, "fpga", "--device", "xc7a100t-ftg256", "--offline", "--decode", bit],
            capture_output=True,
            text=True,
        )
        prefix = "MMCME2_ADV." if kind == "mmcm" else "PLLE2_ADV."
        fields = {}
        unnamed = []
        for line in run.stdout.splitlines():
            if line.startswith("? ") and "CMT_TOP" in line:
                unnamed.append(line[2:].split()[-1])
            words = line.split(None, 1)
            if len(words) != 2 or not words[1].startswith(prefix):
                continue
            field = words[1][len(prefix):]
            m = re.match(r"(.*)\[(\d+)\]$", field)
            if m:
                fields[m.group(1)] = fields.get(m.group(1), 0) | (1 << int(m.group(2)))
            else:
                fields[field] = 1
        return kind, name, fields, unnamed

    with cf.ThreadPoolExecutor(int(os.environ.get("JOBS", "16"))) as pool:
        out = list(pool.map(one, all_lines))
    # The base each run moved from: a run of no change, the first with nothing
    # asked but what its multiplier already was.
    bases = {}
    for kind, name, fields, _ in out:
        wanted = {"mmcm": "CLKFBOUT_MULT_F=8.000 CLKIN1_PERIOD=10", "pll": "CLKFBOUT_MULT=8 CLKIN1_PERIOD=10"}[kind]
        if name == wanted and isinstance(fields, dict):
            bases[kind] = fields
    with open(fixture, "w") as f:
        f.write(
            "# What Vivado 2026.1 wrote for a clock manager's registers, on xc7a100tftg256-1, from\n"
            "# tools/vivado-parity/cmt_sweep.py. Each line is a parameter set on a routed MMCME2_ADV or\n"
            "# PLLE2_ADV, the cell of mmcm_all_outs or pll_all_outs here, with its feedback direct;\n"
            "# then what its bitstream holds, decoded by this repository's database, as the register\n"
            "# fields that differ from its `base` line. `refused` is its refusal instead.\n"
        )
        for kind in ("mmcm", "pll"):
            text = " ".join(f"{k}={v:#x}" for k, v in sorted(bases[kind].items()))
            f.write(f"base {kind} {text}\n")
        for kind, name, fields, unnamed in out:
            if not isinstance(fields, dict):
                f.write(f"{kind} {name} => {fields}\n")
                continue
            base = bases[kind]
            changed = {k: fields.get(k, 0) for k in sorted(set(base) | set(fields)) if base.get(k, 0) != fields.get(k, 0)}
            text = " ".join(f"{k}={v:#x}" for k, v in changed.items())
            if unnamed:
                text += " unnamed=" + ",".join(sorted(unnamed))
            f.write(f"{kind} {name} => {text}\n")


if __name__ == "__main__":
    main()
