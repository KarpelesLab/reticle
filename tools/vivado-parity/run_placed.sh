#!/bin/sh
# Reticle's implementation of each design of a directory, timed by Vivado:
#   run_placed.sh <dir> <device> <part> <out> [design...]
# For each, <out>/<design>/ holds Reticle's own run (reticle.log, endpoints.txt,
# <design>.bit); netlist/ Vivado's placement and routes of Reticle's netlist;
# placement/ Vivado's routes on Reticle's placement; and routes/ Vivado's timer and
# bitstream on Reticle's routes. STAGES names fewer of those, and a design that
# already has its Reticle run keeps it.
# With SECOND_PART set, <out>/<design>/second/ is Reticle's routes again on that
# part: the same bits, timed at another speed grade. All at the lowest priority.
dir=$(cd "$1" && pwd); device=$2; part=$3; mkdir -p "$4"; out=$(cd "$4" && pwd); shift 4
here=$(cd "$(dirname "$0")" && pwd)
bin=${RETICLE:-target/release/reticle}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
designs=${*:-$(ls "$dir" | sed -n 's/\.v$//p')}
echo $designs | tr ' ' '\n' | xargs -P "${JOBS:-6}" -I{} sh -c "
  mkdir -p $out/{}; cd $out/{}
  [ -f x.tcl ] || nice -n 19 ionice -c3 timeout 900 $bin fpga --device $device --top {} --timing --timing-driven --report \
    --constraints $dir/{}.rcf --output-dir $out/{} --bitstream $out/{}/{}.bit \
    --timing-endpoints $out/{}/endpoints.txt --vivado-placement $out/{}/x.tcl $dir/{}.v > reticle.log 2>&1
  [ -f x.tcl ] || exit 0
  for stage in ${STAGES:-netlist placement routes}; do
    nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -notrace -tempDir $out/{}/\$stage.tmp \
      -source $here/run_placed.tcl -tclargs $out/{}/x {} $part $out/{}/\$stage \$stage > \$stage.log 2>&1
    rm -rf \$stage.tmp
  done
  [ -z \"${SECOND_PART}\" ] || nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -notrace -tempDir $out/{}/second.tmp \
      -source $here/run_placed.tcl -tclargs $out/{}/x {} ${SECOND_PART} $out/{}/second routes > second.log 2>&1
  rm -rf second.tmp"
