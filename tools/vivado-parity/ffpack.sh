#!/bin/sh
# Runs testdata/fpga/vivado/ffpack/ through Vivado, at the lowest priority:
# `ffpack.sh <out>`. Each design leaves <out>/<top>/<top>.bit and .cells (the
# slice held to, in Vivado's words). The features in
# testdata/fpga/vivado/ffpack/vivado.txt are then those bitstreams decoded:
#   RETICLE_FFPACK_BITS=<out> UPDATE_EXPECT=1 cargo test --all-features \
#     --test fpga_xray_ffpack_vivado
here=$(cd "$(dirname "$0")" && pwd)
dir=$here/../../testdata/fpga/vivado/ffpack
python3 "$here/gen_ffpack.py" "$dir"
dir=$(cd "$dir" && pwd)
mkdir -p "$1"; out=$(cd "$1" && pwd)
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
ls "$dir" | sed -n 's/\.v$//p' | xargs -P "${JOBS:-6}" -I{} sh -c \
  "nice -n 19 ionice -c3 timeout 1800 vivado -mode batch -nojournal -nolog -tempDir $out/{}.tmp -source $here/ffpack_probe.tcl -tclargs $dir {} ${PART:-xc7a100tftg256-3} $out/{} > $out/{}.log 2>&1; rm -rf $out/{}.tmp"
