#!/usr/bin/env python3
"""Asks Vivado what it makes of each XDC file in testdata/fpga/vivado/xdc/.

Each `<case>.xdc` gets a `<case>.answer` beside it: Vivado's reading of the
file, which is what `tests/fpga_xdc.rs` holds Reticle's to. Needs `vivado` on
the path (`source settings64.sh`); nothing else here does, and the tests read
only what this wrote.

An answer holds:

    port <name> <direction> pin= iostandard= drive= slew= pulltype=
    clock <name> period= waveform= generated= user= virtual= sources=
    pblock <name> ranges= cells=
    tile <a site at a corner of a pblock> <the tile it is in>
    place <cell> loc= bel=
    config <property>=<value>
    timing <a path constraint Vivado kept, each object as class:name>
    refused <line of the case> <Vivado's message id>

Output paths and dates are left out, so that a run on another day changes
nothing it did not measure.
"""
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
DIR = ROOT / "testdata/fpga/vivado/xdc"
TCL = pathlib.Path(__file__).resolve().parent / "xdc_answers.tcl"


def ask(case):
    with tempfile.TemporaryDirectory() as scratch:
        run = subprocess.run(
            ["nice", "-n", "19", "ionice", "-c3", "vivado", "-mode", "batch",
             "-nojournal", "-nolog", "-source", str(TCL), "-tclargs", str(DIR), case],
            cwd=scratch, capture_output=True, text=True, check=False)
    lines = run.stdout.splitlines()
    answers = []
    reading = False
    message = re.compile(
        r"^(?:CRITICAL WARNING|ERROR): \[([^\]]+)\].*\[[^\]]*" + re.escape(case) + r"\.xdc:(\d+)\]$")
    refused = []
    pending = None
    for line in lines:
        if line == "ANSWER begin":
            reading = True
            continue
        if line == "ANSWER end":
            reading = False
            continue
        if reading:
            # A message's source line closes it, sometimes on a line of its own.
            match = message.match(line)
            if match:
                refused.append((int(match.group(2)), match.group(1)))
                pending = None
                continue
            start = re.match(r"^(?:CRITICAL WARNING|ERROR): \[([^\]]+)\]", line)
            if start:
                pending = start.group(1)
                continue
            tail = re.match(r"^\s*\[[^\]]*" + re.escape(case) + r"\.xdc:(\d+)\]$", line)
            if tail and pending:
                refused.append((int(tail.group(1)), pending))
                pending = None
            continue
        if line.startswith("ANSWER "):
            answers.append(" ".join(line[len("ANSWER "):].split()))
    answers.extend(f"refused {n} {code}" for n, code in sorted(refused))
    (DIR / f"{case}.answer").write_text("\n".join(answers) + "\n")
    print(f"{case}: {len(answers)} lines, {len(refused)} refused")


def main():
    cases = sys.argv[1:] or sorted(p.stem for p in DIR.glob("*.xdc"))
    for case in cases:
        ask(case)


if __name__ == "__main__":
    main()
