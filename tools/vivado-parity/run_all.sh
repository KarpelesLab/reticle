#!/bin/sh
# Runs every design of a fixture directory through Vivado, $JOBS at a time, at
# the lowest priority. `run_all.sh <dir> <part> <out>`.
dir=$(cd "$1" && pwd); part=$2; mkdir -p "$3"; out=$(cd "$3" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
ls "$dir" | sed -n 's/\.v$//p' | xargs -P "${JOBS:-24}" -I{} sh -c \
  "nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -tempDir $out/{}.tmp -source $here/run_vivado.tcl -tclargs $dir {} $part $out/{} > $out/{}.log 2>&1; rm -rf $out/{}.tmp"
