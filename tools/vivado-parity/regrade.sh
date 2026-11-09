#!/bin/sh
# Reticle's routes of a run_placed.sh run, timed again by Vivado at other parts:
#   regrade.sh <run> <out> <part>...
# For each design of <run> (one with an x.tcl) and each part, <out>/<part>/<design>/
# holds Vivado's checkpoint of the same placement and routes at that part
# (run_placed.tcl's `routes` stage) and timing_dump.tcl's account of it, with the
# run's x.tcl and endpoints.txt linked beside them so that timing_table.py reads
# it as a run. Nothing is placed or routed again: the same cells on the same
# wires, timed at another speed grade, which is what says how a grade's figures
# differ from another's (docs/fpga-vivado-parity.md, "Speed grades"). JOBS how
# many at once; all at the lowest priority.
here=$(cd "$(dirname "$0")" && pwd)
run=$(cd "$1" && pwd); mkdir -p "$2"; out=$(cd "$2" && pwd); shift 2
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
for part in "$@"; do
  for d in $(ls "$run"); do
    [ -f "$run/$d/x.tcl" ] && echo "$part $d"
  done
done | xargs -P "${JOBS:-6}" -L 1 sh -c '
  part=$0; d=$1; o='"$out"'/$part/$d; mkdir -p $o
  ln -sf '"$run"'/$d/x.tcl $o/x.tcl
  [ ! -f '"$run"'/$d/endpoints.txt ] || ln -sf '"$run"'/$d/endpoints.txt $o/endpoints.txt
  [ -f $o/routes/$d.dcp ] || nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -notrace -tempDir $o/tmp \
     -source '"$here"'/run_placed.tcl -tclargs '"$run"'/$d/x $d $part $o/routes routes > $o/routes.log 2>&1
  [ -f $o/routes/$d.timing ] || [ ! -f $o/routes/$d.dcp ] || nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -notrace -tempDir $o/tmp \
     -source '"$here"'/timing_dump.tcl -tclargs $o/routes/$d.dcp $o/routes/$d.timing > $o/timing.log 2>&1
  rm -rf $o/tmp'
