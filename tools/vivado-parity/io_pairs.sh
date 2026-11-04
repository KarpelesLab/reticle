#!/bin/sh
# Runs io_pairs.tcl in slices, at the lowest priority: `io_pairs.sh <out>`, then
# `io_pairs.py <out>/p*.txt > testdata/fpga/vivado/io/pairs.txt`.
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1"; out=$(cd "$1" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
seq 0 23 | xargs -P "${JOBS:-24}" -I{} sh -c \
  "cd $out && nice -n 19 ionice -c3 vivado -mode batch -nojournal -nolog -source $here/io_pairs.tcl -tclargs {} 24 $out/p{}.txt > $out/p{}.log 2>&1"
