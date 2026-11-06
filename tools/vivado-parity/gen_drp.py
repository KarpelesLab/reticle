#!/usr/bin/env python3
"""Writes each DRP block's primitive probe again with its DRP pins tied one way
and another, for `drp_ecc.py`'s question of what a frame's ECC leaves out.

    tools/vivado-parity/gen_drp.py <out>

For `MMCME2_ADV`, `PLLE2_ADV` and `XADC`: `_drp_on` is the probe as it is, every
pin a signal; `_drp_off` ties `DEN`, `DWE` and `DCLK` low; `_den_off` ties `DEN`
alone. The clock managers also get `_dclk_off`, `_dwe_off`, `_den_dwe_off` and
`_den_dclk_off`. The constraints are the probe's, for the FTG256. Then
`run_all.sh <out> xc7a35tftg256-1 <runs>`.
"""
import os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROBES = os.path.join(HERE, "..", "..", "testdata", "fpga", "vivado", "primitives")
TIES = {
    "drp_on": [],
    "drp_off": ["DEN", "DWE", "DCLK"],
    "den_off": ["DEN"],
}
MORE = {
    "dclk_off": ["DCLK"],
    "dwe_off": ["DWE"],
    "den_dwe_off": ["DEN", "DWE"],
    "den_dclk_off": ["DEN", "DCLK"],
}


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for p in ("mmcme2_adv", "plle2_adv", "xadc"):
        v = open(os.path.join(PROBES, f"{p}_probe.v")).read()
        x = open(os.path.join(PROBES, f"{p}_probe.xdc")).read()
        ties = dict(TIES)
        if p != "xadc":
            ties.update(MORE)
        for tag, pins in ties.items():
            name = f"{p}_{tag}"
            text = v.replace(f"module {p}_probe", f"module {name}")
            for pin in pins:
                text = re.sub(rf"\.{pin}\([^)]*\)", f".{pin}(1'b0)", text)
            open(os.path.join(out, f"{name}.v"), "w").write(text)
            open(os.path.join(out, f"{name}.xdc"), "w").write(x)


main()
