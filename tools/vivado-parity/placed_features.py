#!/usr/bin/env python3
"""Keeps what Vivado's bitstream of Reticle's own routes decodes to, for chosen
probes, from `run_placed.sh` runs that `decode_placed.sh` has been through.

    tools/vivado-parity/placed_features.py <device>=<out> [<device>=<out>...] -- <probe>... \\
        > testdata/fpga/vivado/primitives/placed.txt

A probe is `<name>` for every device given, or `<device>:<name>` for one; a
device may name several runs, `<device>=<out>,<out>`, the first with the probe
being read.
`tests/fpga_xray_placed.rs` builds each again and holds Reticle's own bitstream
to it, feature for feature. Only Vivado's answer is kept: `<out>/<name>/vivado.features`,
which `reticle fpga --decode` wrote of `routes/<name>.bit`.
"""
import os, sys


def main():
    args = sys.argv[1:]
    split = args.index("--")
    runs = dict(a.split("=", 1) for a in args[:split])
    print("# Vivado 2026.1's bitstream of Reticle's own placement and routes of each probe,")
    print("# decoded by `reticle fpga --decode`; tools/vivado-parity/placed_features.py.")
    for probe in args[split + 1:]:
        devices = [probe.split(":", 1)[0]] if ":" in probe else list(runs)
        name = probe.split(":", 1)[-1]
        for device in devices:
            # `<device>=<out>,<out>`: the first run that has the probe.
            path = next(
                p
                for p in (os.path.join(o, name, "vivado.features") for o in runs[device].split(","))
                if os.path.exists(p)
            )
            lines = sorted(set(l.rstrip("\n") for l in open(path) if l.strip()))
            print(f"design {name} {device}")
            for line in lines:
                print(f"  {line}")


main()
