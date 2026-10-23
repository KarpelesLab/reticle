#!/bin/sh
# Runs NewAE's own whole-design testbench, `fpga/sim/cwhusky_tb.v`, in
# Reticle's simulator, as NewAE's `make all VARIANT=PLUS` runs it under
# iverilog: the same defines (`__ICARUS__` included, which iverilog sets
# itself and which NewAE's sources test), every testbench parameter at the
# makefile's default unless given as NAME=VALUE after the output file, the
# Husky Plus's file list as build_plus.sh has it, the FIFO Generator's
# place taken by NewAE's own FIFOs (`NOXILINXFIFO`, NewAE's own default
# for simulation) over the `xpm_memory_sdpram` stand-in, and the Xilinx
# primitives from `xilinx_primitives.v` in place of Vivado's unisims.
#
#   examples/husky/sim/run_cwhusky_tb.sh <chipwhisperer-husky-fpga checkout> <log> [NAME=VALUE ...]
set -eu
husky=${1:?usage: run_cwhusky_tb.sh <checkout> <log> [NAME=VALUE ...]}
log=${2:?usage: run_cwhusky_tb.sh <checkout> <log> [NAME=VALUE ...]}
shift 2
here=$(cd "$(dirname "$0")/.." && pwd)
fpga=$husky/fpga
reticle=${RETICLE:-reticle}

# The makefile's defaults (fpga/sim/makefile), for the parameters
# cwhusky_tb takes.
params="ADC_LOW_RES=0 SLOW_ADC=0 FAST_ADC=0 NOM_ADC=0 FIFO_SAMPLES=90 PRESAMPLES=0
OFFSET_ENABLE=0 OFFSET_MIN=0 OFFSET_MAX=1000 TRIGGER_DELAY=1500 READ_DELAY=150
NUM_SEGMENTS=1 NUM_GLITCHES=1 MAX_GLITCH_OFFSET=0 MAX_GLITCH_REPEATS=0 SHORT_TRIGGER=0
SEGMENT_CYCLES=100 SEGMENT_CYCLE_COUNTER_EN=0 TRIGGER_NOW=0 STREAM=0
STREAM_SEGMENT_THRESHOLD=512 TIMEOUT_CYCLES=50000 ERRORS_OK=0 PRESAMP_ERROR=0
DOWNSAMPLE=0 DUMP=0 SEED=1"
for override in "$@"; do
    params=$(printf '%s\n' $params | grep -v "^${override%%=*}=" || true)
    params="$params $override"
done
args=""
for p in $params; do
    args="$args --param p$p"
done

# The design's sources, from build_plus.sh's list.
sources=$(sed -n '/exec "\$reticle" fpga/,$p' "$here/build_plus.sh" \
    | grep -o '"\$[a-z]*/[^"]*\.v"' | tr -d '"' \
    | sed "s|\$fpga|$fpga|; s|\$here|$here|")

cd "$fpga/sim"
# shellcheck disable=SC2086
exec "$reticle" sim --quiet --sv --top cwhusky_tb \
    --define __ICARUS__ --define PLUS --define ESAD --define INTERVAL_MATCHING=1 \
    --define TRACE --define NOXILINXFIFO --define REGULARFIFO --define NOFASTFIFO \
    --include-dir "$fpga/hdl" --include-dir "$fpga/tracewhisperer/hdl" \
    $args \
    cwhusky_tb.v tb_reg_tasks.v $sources \
    "$here/xpm_memory_sdpram.v" "$here/sim/xilinx_primitives.v" > "$log" 2>&1
