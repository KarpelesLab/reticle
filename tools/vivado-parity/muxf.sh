#!/bin/sh
# Runs testdata/fpga/vivado/muxf/ through Vivado, at the lowest priority:
# `muxf.sh <out>`. Each design leaves <out>/<top>/<top>.bit, .cells (where its
# wide multiplexers went, in Vivado's words) and .timing. The features in
# testdata/fpga/vivado/muxf/vivado.txt are then those bitstreams decoded:
#   RETICLE_MUXF_BITS=<out> UPDATE_EXPECT=1 cargo test --all-features \
#     --test fpga_xray_muxf
here=$(cd "$(dirname "$0")" && pwd)
dir=$here/../../testdata/fpga/vivado/muxf
python3 "$here/gen_muxf.py" "$dir"
dir=$(cd "$dir" && pwd)
mkdir -p "$1"; out=$(cd "$1" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
ls "$dir" | sed -n 's/\.v$//p' | xargs -P "${JOBS:-12}" -I{} sh -c \
  "nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -tempDir $out/{}.tmp -source $here/muxf_probe.tcl -tclargs $dir {} ${PART:-xc7a100tftg256-1} $out/{} > $out/{}.log 2>&1; rm -rf $out/{}.tmp"
