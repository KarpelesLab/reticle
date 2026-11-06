#!/bin/sh
# What each design's synthesised netlist is made of and how deep it is, Vivado's
# synthesis against Reticle's, both counted by Vivado (netlist_qor.tcl):
#   netlist_qor.sh synth <cons dir> <part> <out>
#       Vivado's synth_design on each <design>.v and .xdc of <cons dir> (an
#       achieved.sh `cons` directory), into <out>/<design>/<design>.qor.
#   netlist_qor.sh netlist <seed dir> <part> <out>
#       Reticle's netlist of each <seed dir>/<design>/x.v (achieved.sh's
#       `reticle/s<n>`), linked unplaced, into <out>/<design>/<design>.qor. It
#       is the netlist of a build with --bitstream: the one `reticle fpga`
#       writes without is not structural, and Vivado will not link it.
# Then `netlist_qor.py table vivado=<out> <label>=<out>...` is the table. JOBS
# says how many run at once; all at the lowest priority.
here=$(cd "$(dirname "$0")" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
if [ "$1" = --stage ]; then
  # --stage <stage> <stem> <top> <part> <out dir>
  mkdir -p "$6"
  nice -n 19 ionice -c3 timeout 3000 vivado -mode batch -nojournal -nolog -notrace -tempDir "$6/tmp" \
    -source "$here/netlist_qor.tcl" -tclargs "$2" "$3" "$4" "$5" "$6" > "$6/vivado.log" 2>&1
  rm -rf "$6/tmp"
  exit 0
fi
stage=$1; src=$(cd "$2" && pwd); part=$3; mkdir -p "$4"; out=$(cd "$4" && pwd)
jobs=${JOBS:-6}
self="$here/netlist_qor.sh"
if [ "$stage" = synth ]; then
  ls "$src" | sed -n 's/\.rcf$//p' |
    xargs -P "$jobs" -I{} sh "$self" --stage synth "$src/{}" {} "$part" "$out/{}"
else
  ls "$src" | while read d; do [ -f "$src/$d/x.v" ] && echo "$src/$d/x $d"; done | xargs -P "$jobs" -L 1 sh -c 'sh "$0" --stage netlist "$3" "$4" "$1" "$2/$4"' "$self" "$part" "$out"
fi
