#!/bin/sh
# Runs testdata/fpga/vivado/config/ through Vivado, at the lowest priority:
# `config.sh <out>`. Each design leaves <out>/<top>/<top>.bit and .cells (where
# its configuration primitives went, in Vivado's words, or which step refused
# and why), and <out>/<top>.log.
here=$(cd "$(dirname "$0")" && pwd)
dir=$here/../../testdata/fpga/vivado/config
python3 "$here/gen_config.py" "$dir"
dir=$(cd "$dir" && pwd)
mkdir -p "$1"; out=$(cd "$1" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
ls "$dir" | sed -n 's/\.v$//p' | xargs -P "${JOBS:-24}" -I{} sh -c \
  "nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -tempDir $out/{}.tmp -source $here/config_probe.tcl -tclargs $dir {} ${PART:-xc7a100tftg256-1} $out/{} > $out/{}.log 2>&1; rm -rf $out/{}.tmp"
