#!/bin/sh
# The same designs through Reticle: `run_reticle.sh <dir> <device> <out>`.
# Each leaves <out>/<top>/<top>.result in the form run_vivado.tcl writes.
dir=$(cd "$1" && pwd); device=$2; mkdir -p "$3"; out=$(cd "$3" && pwd)
bin=${RETICLE:-target/release/reticle}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
ls "$dir" | sed -n 's/\.v$//p' | xargs -P "${JOBS:-24}" -I{} sh -c "
  mkdir -p $out/{}; start=\$(date +%s.%N)
  nice -n 19 ionice -c3 timeout 900 $bin fpga --device $device --top {} --timing --timing-driven --report \
    --constraints $dir/{}.rcf --output-dir $out/{} --bitstream $out/{}/{}.bit $dir/{}.v > $out/{}.log 2>&1
  code=\$?; end=\$(date +%s.%N)
  { echo exit \$code; echo seconds total \$(echo \"\$end - \$start\" | bc); } > $out/{}/{}.result"
