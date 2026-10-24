#!/bin/sh
# Runs NewAE's whole-design testbench, `fpga/sim/cwhusky_tb.v`, on the
# netlist Reticle mapped the Husky Plus to (`cwhusky_top.v` in a
# build_plus.sh output directory) instead of on NewAE's RTL: the same
# testbench and parameters as run_cwhusky_tb.sh, with the cells modelled by
# xilinx_cells.v and the clocking and I/O primitives by xilinx_primitives.v.
# It says whether what synthesis and mapping wrote still captures; it says
# nothing about placement, routing or the bitstream.
#
#   examples/husky/sim/run_cwhusky_netlist.sh <checkout> <cwhusky_top.v> <log> [NAME=VALUE ...]
#
# Two things are rewritten on the way in, in a scratch directory:
#
# - The netlist writes a real parameter of a black-box cell as a string
#   (`.CLKFBOUT_MULT_F("32")`), which the models would read as the bits
#   of its characters; the clock managers' real parameters are turned back
#   into real literals.
# - Flattening names a net by the hierarchical name of the driver it was
#   merged into, so some of the names the testbench reaches for are gone.
#   Each is replaced by the net that drove it, worked out from NewAE's RTL
#   for the FIFO Generator stand-ins in fifo_generator.v.
set -eu
husky=${1:?usage: run_cwhusky_netlist.sh <checkout> <netlist> <log> [NAME=VALUE ...]}
netlist=${2:?usage: run_cwhusky_netlist.sh <checkout> <netlist> <log> [NAME=VALUE ...]}
log=${3:?usage: run_cwhusky_netlist.sh <checkout> <netlist> <log> [NAME=VALUE ...]}
shift 3
here=$(cd "$(dirname "$0")" && pwd)
fpga=$husky/fpga
reticle=${RETICLE:-reticle}

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

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
f='U_dut.\\oadc.U_fifo.'
sed -e "s/U_dut\.oadc\.U_fifo\.adc_datain/U_dut.\\\\oadc.ADC_data_tofifo /g" \
    -e "s/U_dut\.oadc\.U_fifo\.fast_fifo_empty/${f}U_fast_fifo_wrapper.U_slower_fifos.u_fifo.empty /g" \
    -e "s/U_dut\.oadc\.U_fifo\.slow_fifo_empty/(!${f}U_slow_fifo_wrapper.U_fifo2.u_fifo.held )/g" \
    -e "s/U_dut\.oadc\.U_fifo\.error_flag/${f}error_flag /g" \
    -e "s/U_dut\.reg_clockglitch\.U_clockglitch\.glitch_enable/U_dut.\\\\reg_clockglitch.U_clockglitch.glitch_go_r /g" \
    -e "s/U_dut\.reg_clockglitch\.U_clockglitch\.glitch_go/U_dut.\\\\reg_clockglitch.U_clockglitch.glitch_go_source /g" \
    -e "s/U_dut\.reg_clockglitch\.U_clockglitch\.glitch_mmcm1_clk_out_buf/U_dut.\\\\reg_clockglitch.U_clockglitch.glitch_mmcm1_clk_out_buf /g" \
    -e "s/U_dut\.reg_clockglitch\.resync\.idle/U_dut.\\\\reg_clockglitch.resync.idle /g" \
    "$fpga/sim/cwhusky_tb.v" > "$work/cwhusky_tb.v"
cp "$fpga/sim/tb_reg_tasks.v" "$work/"
sed -E -e 's/\.((CLK[A-Z0-9_]*(_F|_PERIOD|_PHASE|_DUTY_CYCLE))|REF_JITTER[12])\("([0-9]+)"\)/.\1(\4.0)/g' \
    -e 's/\.((CLK[A-Z0-9_]*(_F|_PERIOD|_PHASE|_DUTY_CYCLE))|REF_JITTER[12])\("([-0-9.eE+]+)"\)/.\1(\4)/g' \
    "$netlist" > "$work/netlist.v"

cd "$work"
# shellcheck disable=SC2086
"$reticle" sim --quiet --sv --top cwhusky_tb ${SIM_ARGS:-} \
    --define __ICARUS__ --define PLUS --define ESAD --define INTERVAL_MATCHING=1 \
    --include-dir "$fpga/hdl" --include-dir "$fpga/tracewhisperer/hdl" \
    $args \
    cwhusky_tb.v tb_reg_tasks.v netlist.v \
    "$here/xilinx_cells.v" "$here/xilinx_primitives.v" > "$log" 2>&1
