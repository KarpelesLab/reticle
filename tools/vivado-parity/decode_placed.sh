#!/bin/sh
# Decodes both bitstreams of each design of a run_placed.sh output directory and
# says what differs: `decode_placed.sh <out> <device>`, which leaves bits.txt, from
# Reticle's routes, and cells.txt, Reticle's cells under Vivado's routes with the
# routing left out. A design Vivado would not finish has only the second.
out=$(cd "$1" && pwd); device=$2
here=$(cd "$(dirname "$0")" && pwd)
bin=${RETICLE:-target/release/reticle}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
ls "$out" | xargs -P "${JOBS:-6}" -I{} sh -c "
  cd $out/{} 2>/dev/null || exit 0
  [ -f {}.bit ] || exit 0
  nice -n 19 ionice -c3 $bin fpga --device $device --decode {}.bit > reticle.features 2> reticle.decode.log
  if [ -f routes/{}.bit ]; then
    nice -n 19 ionice -c3 $bin fpga --device $device --decode routes/{}.bit > vivado.features 2> vivado.decode.log
    python3 -I $here/compare_bits.py reticle.features vivado.features --show 4 > bits.txt
  fi
  if [ -f placement/{}.bit ]; then
    nice -n 19 ionice -c3 $bin fpga --device $device --decode placement/{}.bit > placement.features 2> placement.decode.log
    python3 -I $here/compare_bits.py reticle.features placement.features --cells --show 4 > cells.txt
  fi"
