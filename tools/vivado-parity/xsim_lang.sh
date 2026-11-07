#!/bin/bash
# Writes what Vivado's simulator prints for each language testbench of
# testdata/sim/lang/: <name>.xsim beside <name>.v, .sv or .vhd.
#
#   tools/vivado-parity/xsim_lang.sh [testbench...]
#
# With no arguments, every testbench that has no .xsim yet. Each is a
# self-contained file whose top is `tb`; it is compiled with `xvlog`,
# `xvlog --sv` or `xvhdl --2008`, elaborated with `xelab tb` and run to its
# end with `xsim -R`. What is kept is what the design printed: the lines
# between xsim's `Time resolution` banner and its `$finish`/exit lines, less
# the `Time: ... Iteration: ...` line xsim adds after each VHDL report. A
# path xsim names (its own warnings say which file) is cut to the file's
# name, so that the answers do not depend on where the checkout is.
#
# Needs Vivado's settings64.sh; VIVADO_SETTINGS names it (default
# ~/Xilinx/2026.1/Vivado/settings64.sh). The answers committed beside the
# testbenches were written by Vivado 2026.1.
set -u
here=$(cd "$(dirname "$0")/../.." && pwd)
settings=${VIVADO_SETTINGS:-$HOME/Xilinx/2026.1/Vivado/settings64.sh}
# shellcheck disable=SC1090
source "$settings" > /dev/null 2>&1 || { echo "cannot source $settings" >&2; exit 1; }

if [ $# -eq 0 ]; then
    set --
    for f in "$here"/testdata/sim/lang/*/*.v "$here"/testdata/sim/lang/*/*.sv \
             "$here"/testdata/sim/lang/*/*.vhd; do
        [ -e "$f" ] || continue
        [ -s "${f%.*}.xsim" ] || set -- "$@" "$f"
    done
fi

for t in "$@"; do
    f=$(readlink -f "$t")
    out="${f%.*}.xsim"
    work=$(mktemp -d)
    (
        cd "$work" || exit 1
        case "$f" in
            *.vhd) tool="xvhdl --2008" ;;
            *.sv) tool="xvlog --sv" ;;
            *) tool="xvlog" ;;
        esac
        if ! $tool "$f" > comp.log 2>&1; then
            echo "xsim refuses $t:" >&2
            grep ERROR comp.log >&2
            exit 1
        fi
        if ! xelab tb -s snap -debug off > elab.log 2>&1; then
            echo "xelab refuses $t:" >&2
            grep ERROR elab.log >&2
            exit 1
        fi
        xsim snap -R > run.log 2>&1
        awk '/^Time resolution is/{f=1;next} /^\$finish called/{exit}
             /^INFO: \[Common 17-206\]/{exit} /^exit$/{exit} /^run -all$/{next}
             f && !/^Time: .* Iteration: /' run.log |
            sed -e "s|$(dirname "$f")/||g" > "$out"
        echo "wrote $out"
    )
    rm -rf "$work"
done
