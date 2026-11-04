#!/bin/sh
# Asks Vivado what each parameter of a clock manager does: `cmt_sweep.sh <clocks>
# <out>`, where <clocks> is what `clocks.sh` left. Then
#   python3 cmt_sweep.py table <out> <reticle> testdata/fpga/vivado/clocks/registers.txt
here=$(cd "$(dirname "$0")" && pwd)
clocks=$(cd "$1" && pwd)
mkdir -p "$2"; out=$(cd "$2" && pwd)
python3 "$here/cmt_sweep.py" lists "$out"
. "${VIVADO:-$HOME/Xilinx/2026.1/Vivado}/settings64.sh" >/dev/null 2>&1
for kind in mmcm pll; do
  for list in "$out/$kind".*.list; do
    echo "$kind $list"
  done
done | xargs -P "${JOBS:-8}" -L1 sh -c \
  "nice -n 19 ionice -c3 timeout 7200 vivado -mode batch -nojournal -nolog -tempDir \$1.tmp -source $here/cmt_sweep.tcl -tclargs $clocks/\$0_all_outs/\$0_all_outs.dcp \$1 $out/\$0 > \$1.log 2>&1; rm -rf \$1.tmp"
