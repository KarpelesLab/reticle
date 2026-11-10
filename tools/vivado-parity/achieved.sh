#!/bin/sh
# The clock period each design achieves, Vivado's own flow against Reticle's
# implementation, both by Vivado's timer:
#   achieved.sh <corpus> <part> <out> [seed...]
# <corpus> holds <design>.v and <design>.rcf (clocks and pads). Into <out>:
#   cons0/, own0/   the constraints as given, and Vivado's own flow on them;
#   cons1/, own1/   every clock at 0.85 of what own0 achieved, and the flow again;
#   cons2/, own2/   at 0.85 of the best of those two, and again. Vivado's figure
#                   is the best of the three: given slack, it stops working.
#   reticle/s<n>/   Reticle at each seed (default 1 to 5) on cons1, its routes
#                   timed by Vivado (achieved.tcl's `routes`). Reticle's
#                   criticality is against the longer of the period and the
#                   longest path, so any period under what it achieves is the
#                   same request. A period it meets is not: until 2026-11-10 the
#                   router weighed such a design's latest paths as slack (and
#                   the placer still does); the router's criticality is now
#                   against the longest path whatever the period
#                   (docs/fpga-vivado-parity.md, "Routing for the latest path,
#                   met or not").
#   cons3/          every clock at 0.80 of the best of own0 to own2, which none
#                   of the two below meets: given slack, Vivado's placer stops.
#   vnetlist/       Vivado's synthesis of each design on cons3, placed and routed
#                   by Vivado with place_design and route_design alone.
#   pnr/s<n>/       Reticle's netlist of each seed of NETLIST_SEEDS (default 1)
#                   placed and routed the same way on cons3: beside vnetlist,
#                   only the netlist differs.
# A stage whose result is there and finished is not run again, so a second
# build's figures can be had in <out> beside the first's own runs.
# Then `achieved.py table own=<out>/own0,<out>/own1,<out>/own2 <label>=<out>/reticle`
# is the table, and `achieved.py netlists vnetlist=<out>/vnetlist
# <label>=<out>/pnr` the netlists' own. (Until 2026-11-09 `netlist/` was seed 1's
# netlist placed and routed on Reticle's own constraints, cons1's periods with
# the clocks not declared asynchronous: Vivado then worked on paths between
# clocks Reticle does not time, and stopped at a period met. That measured
# `sad`'s ADC clock at 4.455 ns against 2.819 with the clocks apart.) RETICLE
# names the binary, RETICLE_ARGS adds to its arguments, DEVICE is Reticle's name
# for the part (xc7a100t-ftg256), JOBS how many run at once; all at the lowest
# priority.
here=$(cd "$(dirname "$0")" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
if [ "$1" = --stage ]; then
  # --stage <stage> <stem> <top> <part> <out dir>: one run of achieved.tcl.
  mkdir -p "$6"
  # A stage that finished before is not asked again, as a seed's build is not.
  grep -qs '^end' "$6/$4.result" && exit 0
  nice -n 19 ionice -c3 timeout 3000 vivado -mode batch -nojournal -nolog -notrace -tempDir "$6/tmp" \
    -source "$here/achieved.tcl" -tclargs "$2" "$3" "$4" "$5" "$6" > "$6/vivado.log" 2>&1
  rm -rf "$6/tmp"
  exit 0
fi
corpus=$(cd "$1" && pwd); part=$2; mkdir -p "$3"; out=$(cd "$3" && pwd); shift 3
seeds=${*:-1 2 3 4 5}
bin=${RETICLE:-target/release/reticle}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
jobs=${JOBS:-6}
self="$here/achieved.sh"
designs=$(ls "$corpus" | sed -n 's/\.rcf$//p')
own() { # <cons dir> <own dir>
  for d in $designs; do echo $d; done |
    xargs -P "$jobs" -I{} sh "$self" --stage own "$1/{}" {} "$part" "$2/{}"
}
python3 -I "$here/achieved.py" constraints "$corpus" "$out/cons0"
own "$out/cons0" "$out/own0"
python3 -I "$here/achieved.py" periods 0.85 "$out/own0" > "$out/periods1.txt"
python3 -I "$here/achieved.py" constraints "$corpus" "$out/cons1" "$out/periods1.txt"
own "$out/cons1" "$out/own1"
python3 -I "$here/achieved.py" periods 0.85 "$out/own0" "$out/own1" > "$out/periods2.txt"
python3 -I "$here/achieved.py" constraints "$corpus" "$out/cons2" "$out/periods2.txt"
own "$out/cons2" "$out/own2"
for s in $seeds; do for d in $designs; do echo "$s $d"; done; done | xargs -P "$jobs" -L 1 sh -c '
  s=$0; d=$1; dir='"$out"'/reticle/s$s/$d; mkdir -p $dir
  [ -f $dir/x.tcl ] || nice -n 19 ionice -c3 timeout 3000 '"$bin"' fpga --device '"${DEVICE:-xc7a100t-ftg256}"' \
    --top $d --timing --timing-driven --report --place-seed $s '"${RETICLE_ARGS:-}"' \
    --constraints '"$out"'/cons1/$d.rcf --output-dir $dir --bitstream $dir/$d.bit \
    --timing-endpoints $dir/endpoints.txt --vivado-placement $dir/x.tcl '"$out"'/cons1/$d.v > $dir/reticle.log 2>&1
  [ ! -f $dir/x.tcl ] || sh '"$self"' --stage routes $dir/x $d '"$part"' $dir/routes'
python3 -I "$here/achieved.py" periods 0.80 "$out/own0" "$out/own1" "$out/own2" > "$out/periods3.txt"
python3 -I "$here/achieved.py" constraints "$corpus" "$out/cons3" "$out/periods3.txt"
for d in $designs; do echo $d; done |
  xargs -P "$jobs" -I{} sh "$self" --stage vnetlist "$out/cons3/{}" {} "$part" "$out/vnetlist/{}"
for s in ${NETLIST_SEEDS:-1}; do for d in $designs; do echo "$s $d"; done; done | xargs -P "$jobs" -L 1 sh -c '
  s=$0; d=$1; dir='"$out"'/pnr/s$s/$d
  [ -f '"$out"'/reticle/s$s/$d/x.v ] || exit 0
  mkdir -p $dir/in; ln -sf '"$out"'/reticle/s$s/$d/x.v $dir/in/x.v; cp '"$out"'/cons3/$d.xdc $dir/in/x.xdc
  sh '"$self"' --stage netlist $dir/in/x $d '"$part"' $dir'
