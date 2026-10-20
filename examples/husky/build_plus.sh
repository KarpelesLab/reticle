#!/bin/sh
# Builds NewAE's ChipWhisperer-Husky Plus gateware with Reticle: the same
# checkout as build.sh, built as `fpga/vivado/cwhusky_plus.xpr` builds it.
#
#   examples/husky/build_plus.sh <chipwhisperer-husky-fpga checkout> <output dir>
#
# What differs from build.sh, all from cwhusky_plus.xpr's `sources_1`
# fileset: the part is an XC7A100T (`xc7a100tftg256-3`); the defines are
# PLUS, INTERVAL_MATCHING=1 and ESAD, without BB_LIMIT; and the file list
# adds `semipro_slow_fifo.v` and `sad_x4_slowclock.v` and drops `sad_x2.v`
# and `sad_single_counter.v`, in the xpr's order. The pins are husky.rcf's:
# `cwhusky_100.xdc`, the Plus's constraints, places every port on the
# ball `cwhusky.xdc` does (checked by diffing the two), and differs only in
# declaring the ADC clocks at 250 MHz rather than 200, which are on
# internal nets and not in husky.rcf. The region is the XC7A100T's whole
# grid, 147 by 208 tiles, which declares 77 million pips, past the
# loader's default limit of 48 million; hence `--max-pips`.
#
# `--timing-driven` is what makes the USB register interface work: placed
# for wirelength alone the bitstream configured and answered on the bus,
# but read back nearly every bit as zero, and NewAE's constraints give
# that read path 15 ns (`set_max_delay 15` in cwhusky_100.xdc). Weighted
# by estimated criticality in `clk_usb`'s domain, it reads back what it
# was written. See README.md.
set -eu

husky=${1:?usage: build.sh <chipwhisperer-husky-fpga checkout> <output dir>}
out=${2:?usage: build.sh <chipwhisperer-husky-fpga checkout> <output dir>}
here=$(cd "$(dirname "$0")" && pwd)
fpga=$husky/fpga
reticle=${RETICLE:-reticle}

mkdir -p "$out"
exec "$reticle" fpga \
    --device xc7a100t-ftg256 \
    --top cwhusky_top \
    --define PLUS --define INTERVAL_MATCHING=1 --define ESAD \
    --include-dir "$fpga/tracewhisperer/hdl" --include-dir "$fpga/hdl" \
    --constraints "$here/husky.rcf" \
    --output-dir "$out" \
    --bitstream "$out/cwhusky_plus_top.bit" \
    --region 0,0,147,208 --max-pips 80000000 --timing-driven \
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
    "$fpga/hdl/semipro_slow_fifo.v" \
    "$fpga/hdl/trigger_chooser.v" "$fpga/hdl/trigger_sequencer.v" "$fpga/hdl/sad_x4_slowclock.v" \
    "$fpga/hdl/esad.v" "$fpga/hdl/sad_group_trigger.v" \
    "$fpga/fpga-common/hdl/uart_rx.v" "$fpga/hdl/hw_bb_trig.v" \
    "$fpga/hdl/slow_fifo_wrapper.v" "$fpga/hdl/fast_fifo_wrapper.v" \
    "$fpga/fpga-common/hdl/async_resetFFstyle2.v" \
    "$fpga/hdl/trigger_cdc.v" \
    "$here/fifo_generator.v" "$here/xadc_wiz_0.v" "$here/usr_accesse2.v"
