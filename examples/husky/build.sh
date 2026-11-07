#!/bin/sh
# Builds NewAE's ChipWhisperer-Husky gateware with Reticle, from a checkout
# of their source, with the stand-ins in this directory for the Vivado IP
# cores it instantiates. README.md says what each stand-in is and what is
# known about the result.
#
#   examples/husky/build.sh <chipwhisperer-husky-fpga checkout> <output dir>
#
# The checkout is chipwhisperer-husky-fpga 2dfe1bc with its submodules,
# `fpga/tracewhisperer` at 542b1cc (the `pro` branch; the commit the
# superproject pins is not public). The file list and the three defines
# are the `sources_1` fileset of `fpga/vivado/cwhusky.xpr`, in its order;
# the xpr also names `fpga-common/hdl/async_resetFFstyle2.v` through a
# path outside the checkout (`../../plus_optimization/...`), and the copy
# inside it is used instead. `hdl/trigger_cdc.v` is not in the xpr's list
# but `reg_chipwhisperer.v` instantiates it.
#
# The region is the whole die (the database's grid runs to 114,156): the
# design fills it, and the default, a box around the constrained pins,
# holds 26 800 flip-flop sites against the design's 27 618.
# `--wide-muxes off` because a build of this design that uses `MUXF7` and
# `MUXF8` has not been on a board: docs/fpga-vivado-parity.md says what it
# saves, and the choice has to be measured there before it is made here.
#
# `--srl-min 0` keeps the flip-flops of a chain flip-flops and the copies of one
# copies, as they were when every number here was measured. Without it the design
# has shift registers as Vivado's has, about which nothing has been measured on a
# part; see docs/fpga-vivado-parity.md. Taking it away is a new build to measure.
# `--memories legacy` for the same reason: the default maps memories as
# Vivado does, which changes this design's netlist and placement, and that
# build has not been on a board either (docs/fpga-vivado-parity.md).
# `--delay-model legacy` for the same reason again: the default model is
# fitted to Vivado's timer, and a timing-driven build steers by it, so it
# places and routes this design differently. Every build of it measured on
# a board was made with the legacy model, and this one is that build byte for
# byte; see docs/fpga-vivado-parity.md.
# `--synthesis legacy` for the same reason once more: the default synthesises
# as Vivado does, subtractions, orderings and equalities on the carry chain and
# the logic mapped for depth, which is a different netlist, and none of it has
# been on a board. Subtractions on the chain are the one change of this kind
# that has been, and the Husky Plus's captures went wrong with them for a reason
# not found (`MapOptions::min_subtract_carry_width`); see
# docs/fpga-vivado-parity.md, "Synthesis against Vivado's".
# `--route-throughs off` for the same reason too: by default a flip-flop may
# take its data through a lookup table nothing else needs, as Vivado's do, so
# a slice holds eight rather than four, which packs and routes this design
# differently, and that has not been on a board; see docs/fpga-vivado-parity.md,
# "Eight flip-flops a slice".
set -eu

husky=${1:?usage: build.sh <chipwhisperer-husky-fpga checkout> <output dir>}
out=${2:?usage: build.sh <chipwhisperer-husky-fpga checkout> <output dir>}
here=$(cd "$(dirname "$0")" && pwd)
fpga=$husky/fpga
reticle=${RETICLE:-reticle}

mkdir -p "$out"
exec "$reticle" fpga \
    --wide-muxes off --memories legacy --delay-model legacy --synthesis legacy \
    --srl-min 0 --route-throughs off \
    --device xc7a35t-ftg256 \
    --top cwhusky_top \
    --define INTERVAL_MATCHING=1 --define ESAD --define BB_LIMIT \
    --include-dir "$fpga/tracewhisperer/hdl" --include-dir "$fpga/hdl" \
    --constraints "$here/husky.rcf" \
    --output-dir "$out" \
    --bitstream "$out/cwhusky_top.bit" \
    --region 0,0,114,156 \
    "$fpga/hdl/includes.v" "$fpga/hdl/setup.v" "$fpga/hdl/registers.v" \
    "$fpga/hdl/clock_managment_advanced.v" "$fpga/hdl/fifo_top_husky.v" \
    "$fpga/hdl/openadc_interface.v" "$fpga/hdl/reg_chipwhisperer.v" \
    "$fpga/hdl/reg_clockglitch.v" "$fpga/hdl/reg_openadc.v" \
    "$fpga/hdl/reg_openadc_adcfifo.v" "$fpga/hdl/trigger_resync.v" \
    "$fpga/hdl/trigger_unit.v" "$fpga/hdl/usb_reg_main.v" "$fpga/hdl/cwhusky_top.v" \
    "$fpga/hdl/reg_husky_adc.v" "$fpga/hdl/reg_mmcm_drp.v" "$fpga/hdl/xadc.v" \
    "$fpga/hdl/slow_debug.v" "$fpga/hdl/clockglitch_a7.v" \
    "$fpga/hdl/mmcm_phaseshift_interface.v" "$fpga/hdl/reg_la.v" "$fpga/hdl/userio.v" \
    "$fpga/tracewhisperer/hdl/trace_top.v" "$fpga/tracewhisperer/hdl/reg_trace.v" \
    "$fpga/tracewhisperer/hdl/fe_capture_trace.v" \
    "$fpga/tracewhisperer/phywhisperer-common/hardware/hdl/fifo.v" \
    "$fpga/tracewhisperer/phywhisperer-common/hardware/hdl/fe_capture_main.v" \
    "$fpga/tracewhisperer/phywhisperer-common/hardware/hdl/reg_main.v" \
    "$fpga/tracewhisperer/hdl/simple_trigger.v" "$fpga/hdl/sad.v" \
    "$fpga/hdl/edge_trigger.v" "$fpga/hdl/trigger_pulse_cdc.v" \
    "$fpga/fpga-common/hdl/cdc_simple.v" "$fpga/fpga-common/hdl/fifo_async.v" \
    "$fpga/fpga-common/hdl/cdc_pulse.v" "$fpga/fpga-common/hdl/fifo_sync.v" \
    "$fpga/fpga-common/hdl/cdc_bus.v" "$fpga/hdl/sad_x2_slowclock.v" \
    "$fpga/hdl/trigger_chooser.v" "$fpga/hdl/trigger_sequencer.v" "$fpga/hdl/sad_x2.v" \
    "$fpga/hdl/esad.v" "$fpga/hdl/sad_single_counter.v" "$fpga/hdl/sad_group_trigger.v" \
    "$fpga/fpga-common/hdl/uart_rx.v" "$fpga/hdl/hw_bb_trig.v" \
    "$fpga/hdl/slow_fifo_wrapper.v" "$fpga/hdl/fast_fifo_wrapper.v" \
    "$fpga/fpga-common/hdl/async_resetFFstyle2.v" \
    "$fpga/hdl/trigger_cdc.v" \
    "$here/fifo_generator.v" "$here/xadc_wiz_0.v" "$here/usr_accesse2.v"
