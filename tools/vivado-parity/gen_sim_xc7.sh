#!/usr/bin/env bash
#
# Asks Vivado's simulator what each testbench in testdata/sim/xc7/ prints,
# with the vendor's unisims_ver library and glbl, and keeps it beside the
# testbench as <name>.xsim: the lines between `run -all` and `$finish`, which
# are the testbench's own. tests/sim_xc7_primitives.rs holds Reticle's
# simulator to them.
#
# Usage: tools/vivado-parity/gen_sim_xc7.sh [name...]
#   with no names, every testbench there.
#   VIVADO, if set, is the Vivado install (default ~/Xilinx/2026.1/Vivado).

set -euo pipefail
cd "$(dirname "$0")/../.."
vivado=${VIVADO:-$HOME/Xilinx/2026.1/Vivado}
# shellcheck disable=SC1091
source "$vivado/settings64.sh" >/dev/null
glbl="$vivado/data/verilog/src/glbl.v"
dir=testdata/sim/xc7
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ $# -eq 0 ]; then
    set -- $(cd "$dir" && ls ./*.v | sed 's|^\./||; s|\.v$||')
fi
for name in "$@"; do
    tb=$(readlink -f "$dir/$name.v")
    rm -rf "${work:?}/$name"
    mkdir -p "$work/$name"
    (
        cd "$work/$name"
        nice -n 19 ionice -c3 xvlog -sv "$tb" "$glbl" >log 2>&1
        nice -n 19 ionice -c3 xelab -L unisims_ver -L secureip -debug off tb glbl -s s >>log 2>&1
        nice -n 19 ionice -c3 xsim s -R >out 2>&1
    ) || { cat "$work/$name/log" "$work/$name/out" 2>/dev/null; exit 1; }
    sed -n '/^run -all$/,/^\$finish called/p' "$work/$name/out" | sed '1d;$d' >"$dir/$name.xsim"
    printf '%s: %s lines\n' "$name" "$(wc -l <"$dir/$name.xsim")"
done
