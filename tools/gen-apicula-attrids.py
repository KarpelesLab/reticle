#!/usr/bin/env python3
"""Writes src/fpga/apicula/attrids.rs from Project Apicula's attrids.py.

Usage: tools/gen-apicula-attrids.py <path to apycula/attrids.py>

Only the four tables the Gowin flow reads are transcribed: the IO block's
and the logic slice's attribute and value names. The output is ordered as
the Python dictionaries are, and a unit test in the generated file checks
the ids the flow depends on.
"""

import hashlib
import runpy
import sys

TABLES = [
    ("IOB_ATTRS", "iob_attrids", "An IO block attribute: `logicinfo['IOB']`'s first key."),
    ("IOB_VALUES", "iob_attrvals", "An IO block attribute value: `logicinfo['IOB']`'s second key."),
    ("CLS_ATTRS", "cls_attrids", "A logic slice attribute: `logicinfo['SLICE']`'s first key."),
    ("CLS_VALUES", "cls_attrvals", "A logic slice attribute value: `logicinfo['SLICE']`'s second key."),
]


def main():
    path = sys.argv[1]
    source = open(path, "rb").read()
    names = runpy.run_path(path)
    out = []
    out.append("//! The attribute and value names Project Apicula's database leaves out.")
    out.append("//!")
    out.append("//! **Generated** by `tools/gen-apicula-attrids.py` from `apycula/attrids.py`")
    out.append("//! of apycula 0.33, whose SHA-256 is")
    out.append(f"//! `{hashlib.sha256(source).hexdigest()}`. Do not edit by hand.")
    out.append("//!")
    out.append("//! A Gowin bel's configuration is keyed in the database by")
    out.append("//! `(attribute id, value id)`, and the ids are only named in that Python")
    out.append("//! file. Only the tables the flow reads are here: the IO block's and the")
    out.append("//! logic slice's. See `docs/fpga-gowin.md`, *What the database does not")
    out.append("//! name*.")
    out.append("//!")
    out.append("//! The tables are Project Apicula's, used under its licence:")
    out.append("//!")
    out.append("//! ```text")
    for line in open(path.replace("apycula/attrids.py", "LICENSE")).read().splitlines():
        out.append(f"//! {line}".rstrip())
    out.append("//! ```")
    out.append("")
    for rust, py, doc in TABLES:
        table = names[py]
        out.append(f"/// {doc}")
        out.append(f"pub(super) const {rust}: &[(&str, i64)] = &[")
        for key, value in table.items():
            assert isinstance(key, str) and isinstance(value, int), (py, key)
            escaped = key.replace("\\", "\\\\").replace('"', '\\"')
            out.append(f'    ("{escaped}", {value}),')
        out.append("];")
        out.append("")
    sys.stdout.write("\n".join(out))


if __name__ == "__main__":
    main()
