//! What a timing-driven placement of a 7-series design measures against.
//!
//! [`place_timing`] describes a [`Netlist`] for
//! [`PlaceTiming`](crate::fpga::place::PlaceTiming): which pins start,
//! continue and end paths, how long each cell takes, and which clock each
//! sequential cell is on, with the periods the constraints give.
//!
//! # Where the numbers come from
//!
//! **Read from Project X-Ray's timing model** (`artix7/timings/*.sdf` at
//! commit 0a0adde, the slow corner's maximum, which is the fourth of the
//! four numbers its pips and the second triple its cells carry):
//!
//! - a lookup table, any input to `O6`: 0.124 ns, and to `O5` 0.152 ns
//!   (`CLBLL_L.sdf`, `LUT6`); [`LUT`] is the first;
//! - a slice flip-flop, clock to `Q` 0.34 ns, and its clock enable's setup
//!   0.109 ns (`slicel.sdf`, `REG_INIT_FF`);
//! - a carry chain: 0.114 ns `CIN` to `CO3`, up to 0.64 ns from `CYINIT`
//!   to an output (`CLBLL_L.sdf`, `CARRY4`); [`CARRY`] is between.
//!
//! **From Vivado 2026.1's `report_timing`**, at speed grade 1 on a block on the
//! XC7A100T, each from a design with that one register setting: [`DSP_COMB`],
//! [`DSP_LAUNCH_A`], [`DSP_LAUNCH_M`], [`DSP_LAUNCH_P`], and [`DSP_SETUP_REG`],
//! [`DSP_SETUP_M`], [`DSP_SETUP_P`]. `Prop_dsp48e1_*` and `Setup_dsp48e1_*`,
//! in `docs/fpga-vivado-parity.md`'s table. And an `IDELAYE2`'s arc,
//! [`idelay_arc`], fitted to the same timer's figures for 34 delays.
//!
//! **Not from it**, and said so: the block RAM's clock to output, the
//! input and output buffers, and the clock managers, whose register port
//! (`DRP`) is timed with the block RAM's figures standing in. `LIOB33.sdf` times
//! only `IBUFDISABLE`; [`BRAM_LAUNCH`], [`INPUT_BUFFER`] and
//! [`OUTPUT_BUFFER`] are the order of Xilinx's DS181 figures, not read off
//! it here.
//!
//! **Fitted, not modelled**, is a connection's delay: [`WIRE`] is a line
//! through what this flow's own router made of a placement of NewAE's
//! ChipWhisperer-Husky Plus, every connection's routed delay summed from
//! its pips' delays (`XrayDatabase::node_delays`) against its tile
//! distance. A timing-driven build prints the fit for its own routing
//! (`routed connections, fitted:`), by least squares over the connections
//! of signals of four sinks or fewer, and the extra a tile the wider ones'
//! take over that line. On 2026-10-22, over the Plus's 192,306
//! connections: **0.310 ns at no distance and 0.048 ns a tile**, and
//! 0.025 ns a tile more on a wide signal, whose tree detours.
//!
//! It was 0.35 ns and **0.035** ns a tile until then, from the 2026-10-21
//! routing by a method nobody recorded: a quarter too cheap a tile. On a
//! probe of the Plus's bus it left the read logic 130 tiles from the pin
//! it drives, a connection the estimate put at 4.9 ns and the router made
//! 9.5. The mean by distance is not a line either — 0.28 ns under
//! two tiles, 1.41 ns at ten to twenty, 7.67 ns at eighty to 160 — so
//! the fit is too dear for the shortest connections and too cheap past a
//! hundred tiles.
//!
//! **All of that is [`DelayModel::Legacy`].** The default,
//! [`DelayModel::Vivado`], has Vivado 2026.1's own figures at speed grade 3,
//! measured on Reticle's implementations ([`VIVADO`] says which are measured
//! and which derived), each connection costed by `xray::delay`'s table fitted
//! to Vivado's timer, and [`VIVADO_WIRE`] for the placement's line. See
//! `docs/fpga-vivado-parity.md`.
//!
//! Paths are timed within one clock's domain. A clock is the signal at a
//! sequential cell's clock pin traced back through the global buffers;
//! its period is the `create_clock` on it, and a clock no constraint
//! names is not timed. Under the legacy model a package pin starts and ends
//! paths in every timed domain, which is pessimistic for a pin no clock
//! drives and is what a design whose bus is synchronous to one clock wants.
//! Under the fitted model a pin is timed only when a constraint puts it in a
//! clock's domain (an input delay, or the clock it carries), as Vivado's
//! timer times no unconstrained pad: timed, a pad's path in `qor/counter64.v`
//! from a register at column 52 to a pin at column 147 was the round's
//! latest, and untimed the median over seeds of the period Vivado's timer has
//! that design achieve went from 6.2 ns to 4.1 (`docs/fpga-vivado-parity.md`).
//! This flow has no `set_output_delay`, so under the fitted model no output pin
//! is timed.
//!
//! The constraints' exceptions are honoured where they name one end: a
//! `set_false_path -from` or `-to` takes the instances its glob matches
//! out of the paths they start or end, and a `set_multicycle_path -to`
//! gives the paths ending at them that many periods. A constraint naming
//! both ends is not applied — telling which paths run between the two
//! takes an analysis per start point — and neither is a hold multicycle,
//! since nothing here checks hold.
//!
//! A `set_input_delay` puts the pads it names in its clock's domain alone,
//! launched that long after the clock's edge less [`CLOCK_INSERTION`], the
//! clock's own way from its pad to the register the data races.

use std::collections::HashSet;

use super::DelayModel;
use crate::fpga::Constraints;
use crate::fpga::arch::{NodeId, RoutingGraph};
use crate::fpga::constraints::matches_glob;
use crate::fpga::place::{Netlist, PathDomain, PinDelays, PinTiming, PlaceTiming, Placement};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

/// A lookup table's input to output, ns.
pub(crate) const LUT: f64 = 0.124;
/// A carry chain cell's input to output, ns: between `CIN` to `CO3`
/// (0.114) and `CYINIT` to an output (up to 0.642).
pub(crate) const CARRY: f64 = 0.3;
/// A `MUXF7`'s data input to output, ns: Vivado 2026.1's routed reports of
/// `testdata/fpga/vivado/muxf/`, grade 1, say 0.212 to 0.247, and 0.296 from `S`.
/// This takes the larger of the data inputs', since that is the usual path.
pub(crate) const MUXF7: f64 = 0.247;
/// A `MUXF8`'s data input to output, ns: the same reports say 0.098 to 0.104.
pub(crate) const MUXF8: f64 = 0.104;
/// A slice flip-flop's clock to `Q`, ns.
pub(crate) const FF_LAUNCH: f64 = 0.34;
/// A slice flip-flop's setup, ns.
pub(crate) const FF_SETUP: f64 = 0.109;
/// A `DSP48E1` with no register at all, `A` to `P`, ns: `B` is 3.656.
pub(crate) const DSP_COMB: f64 = 3.841;
/// `A2` or `B2` to `P` through the multiplier, with no `M` or `P` register, ns.
pub(crate) const DSP_LAUNCH_A: f64 = 4.009;
/// `M` to `P`, with no `P` register, ns; 2.215 to 2.305 by the part of the die,
/// and to `PCOUT` 2.411.
pub(crate) const DSP_LAUNCH_M: f64 = 2.305;
/// The `P` register to `P`, ns.
pub(crate) const DSP_LAUNCH_P: f64 = 0.434;
/// Setup of `A` or `B` at their own register, ns: `A` is 0.362, `B` 0.450.
pub(crate) const DSP_SETUP_REG: f64 = 0.45;
/// Setup of `A` at `M` with no register before it, ns: `B` is 1.743.
pub(crate) const DSP_SETUP_M: f64 = 1.928;
/// Setup of `A` at `P` with no register before it, ns: `B` is 3.536.
pub(crate) const DSP_SETUP_P: f64 = 3.722;
/// `PCIN` to `PCOUT` through a block's adder, with no `P` register, ns.
pub(crate) const DSP_CASCADE: f64 = 1.713;
/// Setup of `PCIN` at a `P` register, ns.
pub(crate) const DSP_SETUP_CASCADE: f64 = 1.4;
/// A shift register's clock to `Q`, ns: Vivado 2026.1's `Prop_srl16e_CLK_Q` and
/// `Prop_srlc32e_CLK_Q` for this part at speed grade 1, which are the same.
pub(crate) const SRL_LAUNCH: f64 = 1.628;
/// A shift register's address to `Q`, ns: `Prop_srl16e_A0_Q`.
pub(crate) const SRL_READ: f64 = 0.299;
/// A shift register's setup, ns: `Setup_srl16e_CLK_D`, 0.222, and the larger
/// `Setup_srl16e_CLK_CE`, 0.517, which is what a chain with an enable has to meet.
pub(crate) const SRL_SETUP: f64 = 0.517;
/// A block RAM's clock to output without its output register, ns.
pub(crate) const BRAM_LAUNCH: f64 = 2.45;
/// A block RAM's input setup, ns.
pub(crate) const BRAM_SETUP: f64 = 0.5;
/// A pad to the fabric through an input buffer, ns.
pub(crate) const INPUT_BUFFER: f64 = 1.2;
/// A clock's arrival at a flip-flop after its edge at the clock's pad, ns,
/// for a clock that comes from a pad through a global buffer: the input
/// buffer, the pad's road to the buffer and the network to a leaf. Only
/// an input delay needs it, since there the data races the clock rather
/// than another register's output. From this flow's own routed estimate
/// of the ChipWhisperer-Husky Plus (2026-10-21): 1.2 + 2.18 + 1.24 ns, the
/// least any of its bus's registers saw, on the road; not measured on a
/// part.
pub(crate) const CLOCK_INSERTION: f64 = 4.6;
/// The fabric to a pad through an output buffer, ns.
pub(crate) const OUTPUT_BUFFER: f64 = 3.0;
/// A connection's delay: intercept, per tile, and per tile more on a wide
/// signal; see the module documentation.
pub(crate) const WIRE: (f64, f64, f64) = (0.31, 0.048, 0.025);

/// Every figure [`place_timing`] times a cell or a connection with, for one
/// [`DelayModel`]: [`LEGACY`] holds the constants above, and [`VIVADO`]
/// Vivado 2026.1's at speed grade 3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Figures {
    /// [`LUT`].
    pub lut: f64,
    /// [`CARRY`], for a pin pair [`Figures::carry_arcs`] has no figure for.
    pub carry: f64,
    /// [`MUXF7`].
    pub muxf7: f64,
    /// [`MUXF8`].
    pub muxf8: f64,
    /// [`FF_LAUNCH`].
    pub ff_launch: f64,
    /// [`FF_SETUP`]: the data input's, and every input's without
    /// [`Figures::ff_setup_enable`].
    pub ff_setup: f64,
    /// A flip-flop's clock enable's setup and its reset's, or `None` for
    /// [`Figures::ff_setup`].
    pub ff_setup_enable: Option<(f64, f64)>,
    /// [`DSP_COMB`].
    pub dsp_comb: f64,
    /// [`DSP_LAUNCH_A`].
    pub dsp_launch_a: f64,
    /// [`DSP_LAUNCH_M`].
    pub dsp_launch_m: f64,
    /// [`DSP_LAUNCH_P`].
    pub dsp_launch_p: f64,
    /// [`DSP_SETUP_REG`].
    pub dsp_setup_reg: f64,
    /// [`DSP_SETUP_M`].
    pub dsp_setup_m: f64,
    /// [`DSP_SETUP_P`].
    pub dsp_setup_p: f64,
    /// [`DSP_CASCADE`].
    pub dsp_cascade: f64,
    /// [`DSP_SETUP_CASCADE`].
    pub dsp_setup_cascade: f64,
    /// [`SRL_LAUNCH`].
    pub srl_launch: f64,
    /// [`SRL_READ`].
    pub srl_read: f64,
    /// [`SRL_SETUP`]: every input's, or the data input's when
    /// [`Figures::srl_setup_enable`] is set.
    pub srl_setup: f64,
    /// A shift register's clock enable's setup, or `None` for
    /// [`Figures::srl_setup`].
    pub srl_setup_enable: Option<f64>,
    /// A distributed RAM's data input's setup, and its write enable's and
    /// write address's when [`Figures::lutram_setup_pins`] has none.
    pub lutram_setup: f64,
    /// A distributed RAM's write enable's and write address's setups.
    pub lutram_setup_pins: Option<(f64, f64)>,
    /// [`BRAM_LAUNCH`].
    pub bram_launch: f64,
    /// [`BRAM_SETUP`]: an address's, and every input's without
    /// [`Figures::bram_setup_pins`].
    pub bram_setup: f64,
    /// A block RAM's data input's, enable's and write enable's setups.
    pub bram_setup_pins: Option<(f64, f64, f64)>,
    /// [`INPUT_BUFFER`].
    pub input_buffer: f64,
    /// [`CLOCK_INSERTION`].
    pub clock_insertion: f64,
    /// [`OUTPUT_BUFFER`].
    pub output_buffer: f64,
    /// [`WIRE`].
    pub wire: (f64, f64, f64),
    /// Each carry chain input's delay to each output, or `None` for
    /// [`Figures::carry`] throughout.
    pub carry_arcs: Option<&'static [(&'static str, [f64; 8])]>,
}

/// The figures every build had before [`VIVADO`]'s: [`DelayModel::Legacy`].
pub(crate) const LEGACY: Figures = Figures {
    lut: LUT,
    carry: CARRY,
    muxf7: MUXF7,
    muxf8: MUXF8,
    ff_launch: FF_LAUNCH,
    ff_setup: FF_SETUP,
    ff_setup_enable: None,
    dsp_comb: DSP_COMB,
    dsp_launch_a: DSP_LAUNCH_A,
    dsp_launch_m: DSP_LAUNCH_M,
    dsp_launch_p: DSP_LAUNCH_P,
    dsp_setup_reg: DSP_SETUP_REG,
    dsp_setup_m: DSP_SETUP_M,
    dsp_setup_p: DSP_SETUP_P,
    dsp_cascade: DSP_CASCADE,
    dsp_setup_cascade: DSP_SETUP_CASCADE,
    srl_launch: SRL_LAUNCH,
    srl_read: SRL_READ,
    srl_setup: SRL_SETUP,
    srl_setup_enable: None,
    lutram_setup: FF_SETUP,
    lutram_setup_pins: None,
    bram_launch: BRAM_LAUNCH,
    bram_setup: BRAM_SETUP,
    bram_setup_pins: None,
    input_buffer: INPUT_BUFFER,
    clock_insertion: CLOCK_INSERTION,
    output_buffer: OUTPUT_BUFFER,
    wire: WIRE,
    carry_arcs: None,
};

/// Vivado 2026.1's figures at `xc7a100tftg256-3`: [`DelayModel::Vivado`].
///
/// **Measured**, each the mean over every cell of the corpus
/// `testdata/fpga/vivado/timing/arcs.txt` was written from, on Reticle's own
/// placements (the slow corner's maximum of `get_timing_arcs`): a lookup
/// table, 0.13 ns, which is 0.097 on most of its arcs and 0.21 to 0.25 on a
/// quarter of them, by nothing this flow could tell apart (not the pin, the
/// half, the fanout nor the slice); a flip-flop's clock to `Q` before its site
/// is known (0.313 to 0.393 by slice and bel, which
/// [`place_timing_on_sites`] then applies), its data input's setup, 0.027,
/// its clock enable's, 0.218, and its reset's, 0.33; `MUXF7` and `MUXF8` from a
/// data input; a shift register's clock to `Q`, 1.2, its data's setup and its
/// enable's; a distributed RAM's data input's setup, its write enable's and its
/// write address's; a block RAM's clock to output with no output register,
/// 1.846, and its address's, data's, enable's and write enable's setups; the
/// input and output buffers, at `LVCMOS33`; the carry chain, arc by arc
/// ([`CARRY_ARCS`]); and of a `DSP48E1`, the cascade through its adder, its
/// cascade input's setup, `M` and `P` to `P`, and an operand's setup.
///
/// **Derived, not measured:** a shift register's address to `Q`, and the
/// `DSP48E1`'s figures nothing in the corpus used (no register at all, `A`
/// to `P` through the multiplier, setup at `M` and at `P`), are their grade-1
/// figures above times 0.73, the ratio of grade 3 to grade 1 the block's
/// measured arcs keep (1.255 / 1.713, 1.025 / 1.4, 1.671 / 2.305).
/// `CLOCK_INSERTION`, 4.2, is Vivado's clock delay to the registers of
/// `qor/gray_cdc.v` (4.01 to 4.30 ns), one design read off one report.
pub(crate) const VIVADO: Figures = Figures {
    lut: 0.13,
    carry: CARRY,
    muxf7: 0.175,
    muxf8: 0.074,
    ff_launch: 0.345,
    ff_setup: 0.027,
    ff_setup_enable: Some((0.218, 0.33)),
    dsp_comb: 2.80,
    dsp_launch_a: 2.93,
    dsp_launch_m: 1.671,
    dsp_launch_p: 0.329,
    dsp_setup_reg: 0.375,
    dsp_setup_m: 1.41,
    dsp_setup_p: 2.72,
    dsp_cascade: 1.255,
    dsp_setup_cascade: 1.025,
    srl_launch: 1.2,
    srl_read: 0.22,
    srl_setup: 0.057,
    srl_setup_enable: Some(0.368),
    lutram_setup: 0.169,
    lutram_setup_pins: Some((0.384, 0.07)),
    bram_launch: 1.846,
    bram_setup: 0.46,
    bram_setup_pins: Some((0.22, 0.38, 0.45)),
    input_buffer: 1.34,
    clock_insertion: 4.2,
    output_buffer: 3.151,
    wire: VIVADO_WIRE,
    carry_arcs: Some(CARRY_ARCS),
};

/// [`WIRE`] for [`VIVADO`]: the same line, fitted to Vivado's interconnect
/// delay of each connection of the corpus against its tile distance
/// (`timing_table.py fit`, `testdata/fpga/vivado/timing/classes.txt`). The
/// corpus is small designs, whose connections are mostly short; how far the
/// line holds past forty tiles has not been measured.
pub(crate) const VIVADO_WIRE: (f64, f64, f64) = (0.414, 0.0358, 0.0244);

/// A carry chain's arcs at grade 3, by input role, to its outputs in the order
/// `s0`, `s1`, `s2`, `s3`, `co0`, `co1`, `co2`, `co3` (`O` and `CO`);
/// `f64::NEG_INFINITY` where the input does not reach the output. The medians
/// of Vivado's arcs over the corpus's chains in `SLICEL`s (`arcs.txt`; a
/// `SLICEM`'s are within 0.02 ns). `cyinit`'s are prjxray's `CARRY4`
/// figures for it (`CLBLL_L.sdf`) times 0.75, the ratio Vivado's `CI` arcs
/// keep to prjxray's: no chain of the corpus started on it in Vivado's
/// netlist, so they are derived, not measured.
pub(crate) const CARRY_ARCS: &[(&str, [f64; 8])] = &[
    (
        "ci",
        [0.159, 0.230, 0.181, 0.234, 0.187, 0.120, 0.173, 0.089],
    ),
    (
        "cyinit",
        [0.362, 0.449, 0.438, 0.482, 0.402, 0.371, 0.444, 0.435],
    ),
    ("di0", [NO, 0.303, 0.411, 0.454, 0.286, 0.343, 0.400, 0.392]),
    ("di1", [NO, NO, 0.395, 0.441, NO, 0.328, 0.385, 0.374]),
    ("di2", [NO, NO, NO, 0.324, NO, NO, 0.266, 0.295]),
    ("di3", [NO, NO, NO, NO, NO, NO, NO, 0.285]),
    (
        "p0",
        [0.187, 0.303, 0.407, 0.452, 0.276, 0.342, 0.403, 0.395],
    ),
    ("p1", [NO, 0.169, 0.432, 0.477, NO, 0.367, 0.428, 0.412]),
    ("p2", [NO, NO, 0.189, 0.267, NO, NO, 0.236, 0.301]),
    ("p3", [NO, NO, NO, 0.187, NO, NO, NO, 0.299]),
];

/// No path from an input to an output.
const NO: f64 = f64::NEG_INFINITY;

/// The figures of `model`.
pub(crate) fn figures(model: DelayModel) -> &'static Figures {
    match model {
        DelayModel::Legacy => &LEGACY,
        DelayModel::Vivado => &VIVADO,
    }
}

/// An `IDELAYE2`'s count register, `C` to `CNTVALUEOUT`, ns: Vivado 2026.1's
/// `Reg Clk to Q` arc of the delay (`idelay_sweep`).
pub(crate) const IDELAY_COUNT_LAUNCH: f64 = 0.751;

/// An `IDELAYE2`'s data input to `DATAOUT`, ns, at the delay its parameters
/// give: Vivado 2026.1's timer, the slow corner's maximum on the XC7A100T,
/// `Prop_idelaye2_IDATAIN_DATAOUT` or `_DATAIN_DATAOUT`, at speed grade 1 for
/// [`DelayModel::Legacy`], whose other figures are grade 1's, and grade 3 for
/// [`DelayModel::Vivado`], whose are grade 3's.
///
/// **Fitted to Vivado's figures, not read from a document.** The timer's arc
/// on 34 delays at grade 1 (`testdata/fpga/vivado/idelay/`'s sweeps, and
/// probes of `REFCLK_FREQUENCY` at 190, 210 and 300 MHz,
/// `HIGH_PERFORMANCE_MODE` and `SIGNAL_PATTERN`, values 0 to 31), and on 22
/// of the same at grade 3, is this within 1.2 ps of every one:
///
/// `0.815` from `IDATAIN` (`1.012` from `DATAIN`) at grade 1, `0.601`
/// (`0.748`) at grade 3, `+ IDELAY_VALUE × tap`, the same tap at both,
/// where `tap = a / (64 × REFCLK_FREQUENCY) + jitter`, `a` is 1.1132, or
/// 1.094 in `HIGH_PERFORMANCE_MODE`, and `jitter` is 9 ps a tap for the
/// `DATA` pattern, 5 ps in `HIGH_PERFORMANCE_MODE`, and none for `CLOCK`.
/// `1 / (64 × f)` is a tap of a chain calibrated to half the reference
/// clock's period over 32 taps, and `a` the timer's margin on it at this
/// corner; that reading is mine, the figures are Vivado's.
///
/// The timer takes `IDELAY_VALUE` whatever `IDELAY_TYPE` is: a `VAR_LOAD`
/// delay of 9 is nine taps to it (1.679 ns), though nothing of it loads 9. A
/// variable delay moves at run time, which no timer sees.
pub(crate) fn idelay_arc(params: &crate::ir::Attrs, model: DelayModel) -> f64 {
    let text = |name: &str, default: &str| {
        params
            .get(name)
            .and_then(AttrValue::as_str)
            .map_or_else(|| default.to_owned(), str::to_ascii_uppercase)
    };
    let number = |name: &str, default: f64| {
        params.get(name).map_or(default, |v| {
            #[allow(clippy::cast_precision_loss, reason = "a tap count or a frequency")]
            let int = v.as_int().map(|i| i as f64);
            int.or_else(|| v.as_str().and_then(|t| t.trim().parse::<f64>().ok()))
                .unwrap_or(default)
        })
    };
    let fabric = text("DELAY_SRC", "IDATAIN") == "DATAIN";
    let fast = text("HIGH_PERFORMANCE_MODE", "FALSE") == "TRUE";
    let clock = text("SIGNAL_PATTERN", "DATA") == "CLOCK";
    let value = number("IDELAY_VALUE", 0.0).clamp(0.0, 31.0);
    let frequency = number("REFCLK_FREQUENCY", 200.0).max(1.0);
    let margin = if fast { 1.094 } else { 1.1132 };
    let jitter = match (clock, fast) {
        (true, _) => 0.0,
        (false, true) => 0.005,
        (false, false) => 0.009,
    };
    let tap = margin * 1000.0 / (64.0 * frequency) + jitter;
    let intercept = match (model, fabric) {
        (DelayModel::Legacy, false) => 0.815,
        (DelayModel::Legacy, true) => 1.012,
        (DelayModel::Vivado, false) => 0.601,
        (DelayModel::Vivado, true) => 0.748,
    };
    intercept + value * tap
}

/// What a `DSP48E1`'s registers make of its pins.
struct DspTiming {
    /// Its outputs start paths this long after the clock, or follow from its
    /// inputs when `None`.
    launch: Option<f64>,
    /// Its inputs end paths this long before the clock, or reach its outputs
    /// when `None`.
    capture: Option<f64>,
    /// Inputs to outputs, when both are so.
    through: f64,
}

/// A block with `input` registers on the operand with fewer, and `m` and `p`
/// its multiplier's and output's. The table is `report_timing`'s, per
/// setting. The enables and resets are read at the same figure as the data,
/// which is too dear for them, and the two operands at the worse of theirs.
fn dsp_timing(f: &Figures, input: i64, m: i64, p: i64) -> DspTiming {
    let launch = if p > 0 {
        Some(f.dsp_launch_p)
    } else if m > 0 {
        Some(f.dsp_launch_m)
    } else if input > 0 {
        Some(f.dsp_launch_a)
    } else {
        None
    };
    let capture = if input > 0 {
        Some(f.dsp_setup_reg)
    } else if m > 0 {
        Some(f.dsp_setup_m)
    } else if p > 0 {
        Some(f.dsp_setup_p)
    } else {
        None
    };
    DspTiming {
        launch,
        capture,
        through: f.dsp_comb,
    }
}

/// What a path does at one pin of a block: `chained` when `PCIN` reaches its outputs
/// through its adder.
fn dsp_pin(role: &str, output: bool, shape: &DspTiming, chained: bool) -> PinTiming {
    if is_clock(role) {
        PinTiming::Ignored
    } else if chained && role.starts_with("pcin") {
        PinTiming::Through
    } else if output {
        if shape.launch.is_some() && !chained {
            PinTiming::Start
        } else {
            PinTiming::Through
        }
    } else if shape.capture.is_some() {
        PinTiming::End
    } else {
        PinTiming::Through
    }
}

/// Whether `role` is a clock pin, which no path ends at.
fn is_clock(role: &str) -> bool {
    role.contains("clk") || role == "c"
}

/// Which of a lookup table's outputs each of its inputs reaches, as
/// [`PlaceTiming::reaches`] wants it: bit `j` for the `j`th output pin.
/// `O6` reads the upper half of the truth table when `I5` is tied high —
/// a `LUT6_2`, whose `O5` is the lower half — and all of it otherwise.
fn lut_reaches(netlist: &Netlist, instance: usize, init: &crate::ir::Const, out: &mut [u8]) {
    let inst = &netlist.instances[instance];
    let pin = |role: &str| {
        inst.pins
            .iter()
            .copied()
            .find(|p| netlist.pins[*p].role == role)
    };
    let i5_high = pin("i5").is_none_or(|p| {
        netlist.pins[p].signal.is_none() && netlist.pins[p].constant != Some(Bit::Zero)
    });
    let outputs: Vec<&str> = inst
        .pins
        .iter()
        .filter(|p| netlist.pins[**p].output)
        .map(|p| netlist.pins[*p].role.as_str())
        .collect();
    for p in &inst.pins {
        let pin = &netlist.pins[*p];
        if pin.output {
            continue;
        }
        let Some(k) = pin
            .role
            .strip_prefix('i')
            .and_then(|r| r.parse::<u32>().ok())
        else {
            continue;
        };
        let differs = |from: u32, to: u32| {
            (from..to)
                .filter(|i| i & (1 << k) == 0)
                .any(|i| init.bit(i) != init.bit(i | (1 << k)))
        };
        let mut mask = 0u8;
        for (j, role) in outputs.iter().enumerate() {
            let read = match *role {
                "o5" => k < 5 && differs(0, 32),
                _ if i5_high && k != 5 => differs(32, 64),
                _ => differs(0, 64),
            };
            if read && j < 8 {
                mask |= 1 << j;
            }
        }
        out[*p] = mask;
    }
}

/// A clock manager's reference clock: the signal on its `CLKIN1`.
fn manager_input(netlist: &Netlist, instance: usize) -> Option<usize> {
    netlist.instances[instance]
        .pins
        .iter()
        .find(|p| netlist.pins[**p].role == "ref")
        .and_then(|p| netlist.pins[*p].signal)
}

/// A real parameter of `instance`'s cell: an integer, or text such as
/// `6.0`, which is how a black box keeps a real.
fn real_param(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    instance: usize,
    name: &str,
) -> Option<f64> {
    let cell = design.modules[top]
        .cells
        .get(netlist.instances[instance].cell)?;
    let value = cell.params.get(name)?;
    #[allow(clippy::cast_precision_loss, reason = "a divider or a period")]
    let int = value.as_int().map(|i| i as f64);
    int.or_else(|| value.as_str().and_then(|t| t.trim().parse::<f64>().ok()))
}

/// The period of the output `role` of the clock manager `instance` with a
/// reference clock of `input` ns: `input × DIVCLK_DIVIDE × the output's
/// divider ÷ the feedback's multiplier`, the clock Vivado derives for it
/// from the same parameters. An MMCM's are `CLKFBOUT_MULT_F` and, for
/// `CLKOUT0`, `CLKOUT0_DIVIDE_F`; a PLL's `CLKFBOUT_MULT` and
/// `CLKOUTn_DIVIDE`. Each takes the primitive's default when the cell does
/// not set it (UG472: a multiplier of 5, dividers of 1). The inverted
/// outputs (`CLKOUTnB`) have their output's period; the feedback output a
/// divider of one. `None` for a role that is no clock output, or a
/// parameter that is no number.
///
/// What it says is the period the parameters give. NewAE's software
/// writes the logic analyser's dividers through the DRP at run time, to
/// 384 MHz where the parameters say 240: like Vivado, this times the
/// domain at the parameters' figure, and what the part runs is faster.
fn derived_period(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    instance: usize,
    role: &str,
    input: f64,
) -> Option<f64> {
    let mmcm = netlist.instances[instance].kind == "mmcm";
    let param = |name: &str, default: f64| -> Option<f64> {
        match real_param(design, top, netlist, instance, name) {
            Some(v) => Some(v),
            None => {
                let cell = design.modules[top]
                    .cells
                    .get(netlist.instances[instance].cell);
                // Set, but not to a number: no period.
                if cell.is_some_and(|c| c.params.get(name).is_some()) {
                    None
                } else {
                    Some(default)
                }
            }
        }
    };
    let mult = param(
        if mmcm {
            "CLKFBOUT_MULT_F"
        } else {
            "CLKFBOUT_MULT"
        },
        5.0,
    )?;
    let divclk = param("DIVCLK_DIVIDE", 1.0)?;
    let divide = if role == "fbout" || role == "fboutb" {
        1.0
    } else {
        let n = role
            .strip_prefix("out")
            .map(|r| r.trim_end_matches('b'))
            .and_then(|r| r.parse::<u32>().ok())?;
        let name = if mmcm && n == 0 {
            "CLKOUT0_DIVIDE_F".to_owned()
        } else {
            format!("CLKOUT{n}_DIVIDE")
        };
        param(&name, 1.0)?
    };
    (mult > 0.0).then(|| input * divclk * divide / mult)
}

/// A clock signal's name and period through the global buffers that choose
/// between two clocks: the faster of the two inputs' clocks, each found the
/// same way, at most `depth` buffers deep; `base` for a signal no such
/// buffer drives. See `place_timing`.
fn through_muxes(
    netlist: &Netlist,
    signal: usize,
    depth: u32,
    base: &dyn Fn(usize) -> Option<(String, f64)>,
) -> Option<(String, f64)> {
    let mut signal = signal;
    for _ in 0..depth {
        let Some(driver) = netlist.signals[signal].driver else {
            break;
        };
        let inst = &netlist.instances[netlist.pins[driver].instance];
        if inst.kind != "gb" && !super::is_regional_kind(&inst.kind) {
            break;
        }
        let input = |role: &str| {
            inst.pins
                .iter()
                .find(|p| !netlist.pins[**p].output && netlist.pins[**p].role == role)
                .and_then(|p| netlist.pins[*p].signal)
        };
        match (input("i"), input("i1")) {
            (Some(first), Some(second)) => {
                let a = through_muxes(netlist, first, depth - 1, base);
                let b = through_muxes(netlist, second, depth - 1, base);
                return match (a, b) {
                    (Some(a), Some(b)) => Some(if b.1 < a.1 { b } else { a }),
                    (a, b) => a.or(b),
                };
            }
            (Some(only), None) => signal = only,
            _ => break,
        }
    }
    base(signal)
}

/// The timing a placement of `netlist` is driven by, and the name of each
/// of its domains (the clock's `create_clock` name), in
/// [`PlaceTiming::periods`] order. `design`'s module `top` is what the
/// netlist was built from, for the lookup tables' truth tables.
#[must_use]
pub fn place_timing(
    design: &Design,
    top: ModuleId,
    netlist: &Netlist,
    constraints: &Constraints,
    model: DelayModel,
) -> (PlaceTiming, Vec<String>) {
    let f = figures(model);
    let n = netlist.instances.len();
    let mut pins = vec![PinTiming::Ignored; netlist.pins.len()];
    let mut cell = vec![0.0; n];
    let mut launch = vec![0.0; n];
    let mut capture = vec![0.0; n];
    let mut domain = vec![PathDomain::Untimed; n];
    let mut pin_domain: Vec<Option<PathDomain>> = vec![None; netlist.pins.len()];

    // The root of a clock pin's signal, through the global buffers.
    let root = |mut signal: usize| -> usize {
        for _ in 0..8 {
            let Some(driver) = netlist.signals[signal].driver else {
                break;
            };
            let inst = &netlist.instances[netlist.pins[driver].instance];
            if inst.kind != "gb" && !super::is_regional_kind(&inst.kind) {
                break;
            }
            let input = inst
                .pins
                .iter()
                .find(|p| !netlist.pins[**p].output && netlist.pins[**p].role == "i")
                .and_then(|p| netlist.pins[*p].signal);
            match input {
                Some(next) => signal = next,
                None => break,
            }
        }
        signal
    };
    // A clock signal's name and period: a `create_clock` on its root, or,
    // for an output of a clock manager, what the manager makes of its
    // reference clock (`derived_period`). Untimed, as these were, the
    // logic analyser's MMCM on the Husky Plus sampled at 192 MHz and not
    // at 384: its capture's FIFO stayed empty, and with it every glitch
    // test NewAE's suite has.
    let clock_of = |signal: usize| -> Option<(String, f64)> {
        let mut signal = signal;
        // The chain of managers from the clock to its pad, outermost last.
        let mut chain: Vec<(usize, &str)> = Vec::new();
        for _ in 0..4 {
            let at = root(signal);
            let name = &netlist.signals[at].name;
            if let Some(clock) = constraints.clocks.iter().find(|c| c.net == *name) {
                return chain.iter().rev().try_fold(
                    (clock.name.clone(), clock.period_ns),
                    |(_, period), (instance, role)| {
                        let period = derived_period(design, top, netlist, *instance, role, period)?;
                        let inst = &netlist.instances[*instance];
                        Some((format!("{}.{role}", inst.name), period))
                    },
                );
            }
            let driver = netlist.signals[at].driver?;
            let instance = netlist.pins[driver].instance;
            if !matches!(netlist.instances[instance].kind.as_str(), "mmcm" | "pll") {
                break;
            }
            chain.push((instance, netlist.pins[driver].role.as_str()));
            match manager_input(netlist, instance) {
                Some(next) => signal = next,
                None => break,
            }
        }
        // No clock on the pad: the innermost manager's own `CLKIN1_PERIOD`.
        let mut period = None;
        let mut name = String::new();
        for (instance, role) in chain.iter().rev() {
            let input = match period {
                Some(p) => p,
                None => real_param(design, top, netlist, *instance, "CLKIN1_PERIOD")?,
            };
            period = Some(derived_period(
                design, top, netlist, *instance, role, input,
            )?);
            name = format!("{}.{role}", netlist.instances[*instance].name);
        }
        period.map(|p| (name, p))
    };
    // And through a buffer that chooses between two clocks (`BUFGMUX`), the
    // faster of the two: what it drives is timed against each in turn by
    // Vivado, and in one domain here, so in the stricter one. Followed
    // through its first input alone, as `root` follows a buffer, the Husky
    // Plus's shared FIFO (`fifo_wr_clk`, the trace clock or the logic
    // analyser's) was in no domain at all: the analyser's capture registers
    // wrote it at 400 MHz over a path nothing timed, and whether its
    // samples came back whole depended on the seed.
    let clock_of = |signal: usize| through_muxes(netlist, signal, 8, &clock_of);
    let mut names: Vec<String> = Vec::new();
    let mut periods: Vec<f64> = Vec::new();
    let mut domain_of = |clock: Option<(String, f64)>| -> PathDomain {
        let Some((name, period)) = clock else {
            return PathDomain::Untimed;
        };
        if let Some(index) = names.iter().position(|n| *n == name) {
            return PathDomain::Clock(index);
        }
        names.push(name);
        periods.push(period);
        PathDomain::Clock(names.len() - 1)
    };

    // A cell a clock reaches on a data pin is frozen: the clock network
    // leaves for the fabric at few places (an `HROW`'s `BUFHCLK` lines),
    // and on the Husky Plus the rounds moved two such cells — one read a
    // `BUFGMUX`, one a PLL output — until both needed `HCLK_OUT_L3` of one
    // clock row, and the route stopped with them sharing it after 100
    // passes. The wirelength placement had routed.
    let from_network = |pin: usize| -> bool {
        netlist.pins[pin]
            .signal
            .and_then(|s| netlist.signals[s].driver)
            .is_some_and(|d| {
                matches!(
                    netlist.instances[netlist.pins[d].instance].kind.as_str(),
                    "gb" | "bufh" | "bufr" | "bufio" | "pll" | "mmcm"
                )
            })
    };
    let frozen: Vec<bool> = netlist
        .instances
        .iter()
        .map(|inst| {
            inst.kind != "gb"
                && !super::is_regional_kind(&inst.kind)
                && inst.pins.iter().any(|p| {
                    !netlist.pins[*p].output
                        && !is_clock(&netlist.pins[*p].role)
                        && from_network(*p)
                })
        })
        .collect();
    // The constraints' exceptions, by instance name. A path constraint
    // naming one end is applied to the instances that end matches; one
    // naming both needs the paths between them told apart, which this
    // analysis does not do, and it is left out (see the module docs).
    // An instance answers to its own name and, for a pad's buffer
    // (`LED_ADC$io0`), to its port's (`LED_ADC`), which is what a
    // constraint names and what the constraints' own check knows.
    let answers = |glob: &str, instance: usize| {
        let inst = &netlist.instances[instance];
        matches_glob(glob, &inst.name)
            || (inst.kind == "io"
                && inst
                    .name
                    .split_once('$')
                    .is_some_and(|(port, _)| matches_glob(glob, port)))
    };
    let matching = |glob: &Option<String>| -> Vec<bool> {
        (0..n)
            .map(|i| glob.as_ref().is_some_and(|g| answers(g, i)))
            .collect()
    };
    let mut untimed_from = vec![false; n];
    let mut untimed_to = vec![false; n];
    for path in &constraints.false_paths {
        match (&path.from, &path.to) {
            (Some(_), None) => {
                for (i, m) in matching(&path.from).iter().enumerate() {
                    untimed_from[i] |= m;
                }
            }
            (None, Some(_)) => {
                for (i, m) in matching(&path.to).iter().enumerate() {
                    untimed_to[i] |= m;
                }
            }
            _ => {}
        }
    }
    let mut cycles = vec![1u32; n];
    for multi in &constraints.multicycle_paths {
        if multi.hold || multi.path.from.is_some() || multi.path.to.is_none() {
            continue;
        }
        for (i, m) in matching(&multi.path.to).iter().enumerate() {
            if *m {
                cycles[i] = cycles[i].max(multi.cycles);
            }
        }
    }
    let mut reaches = vec![u8::MAX; netlist.pins.len()];
    for (index, inst) in netlist.instances.iter().enumerate() {
        if inst.kind != "lut" {
            continue;
        }
        let init =
            design.modules[top]
                .cells
                .get(inst.cell)
                .and_then(|c| match c.params.get("INIT") {
                    Some(AttrValue::Const(init)) if init.width() == 64 => Some(init),
                    _ => None,
                });
        if let Some(init) = init {
            lut_reaches(netlist, index, init, &mut reaches);
        }
    }
    for (index, inst) in netlist.instances.iter().enumerate() {
        let kind = inst.kind.as_str();
        // The first clock pin that has a signal on it: a tied-off one has
        // nothing to time against.
        let clock = inst
            .pins
            .iter()
            .filter(|p| !netlist.pins[**p].output && is_clock(&netlist.pins[**p].role))
            .find_map(|p| netlist.pins[*p].signal);
        match kind {
            "lut" | "carry" | "gb" | "bufh" | "bufr" | "bufio" | "bufmr" | "mux7" | "mux8" => {
                cell[index] = match kind {
                    "lut" => f.lut,
                    "carry" => f.carry,
                    "mux7" => f.muxf7,
                    "mux8" => f.muxf8,
                    _ => 0.0,
                };
                for pin in &inst.pins {
                    if !is_clock(&netlist.pins[*pin].role)
                        || kind == "gb"
                        || super::is_regional_kind(kind)
                    {
                        pins[*pin] = PinTiming::Through;
                    }
                }
            }
            "srl32" | "srl16" => {
                // Clocked at a fixed address, which is a chain's: its `Q` starts
                // a path, `SRL_LAUNCH` after the clock and five times a flip-
                // flop's. At an address that moves the read is combinational,
                // as a RAM's, and what is lost is the clock's path to `Q`.
                let moves = inst.pins.iter().any(|p| {
                    let pin = &netlist.pins[*p];
                    pin.role.starts_with('a') && pin.signal.is_some()
                });
                capture[index] = f.srl_setup;
                if moves {
                    cell[index] = f.srl_read;
                } else {
                    launch[index] = f.srl_launch;
                }
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = if is_clock(role) {
                        PinTiming::Ignored
                    } else if netlist.pins[*pin].output {
                        if moves && role == "q" {
                            PinTiming::Through
                        } else {
                            PinTiming::Start
                        }
                    } else if role.starts_with('a') {
                        PinTiming::Through
                    } else {
                        PinTiming::End
                    };
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(clock_of(c));
                }
            }
            kind if kind == "lutram" || kind.starts_with("lutram_") => {
                // The read is combinational, the write is clocked. A read
                // address is any address of a shape that reads at it: every
                // shape but a `RAM64X1D` with only `DPO` read reads at its
                // write address too (`SPO`, `DOD`).
                cell[index] = f.lut;
                capture[index] = f.lutram_setup;
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = if is_clock(role) {
                        PinTiming::Ignored
                    } else if netlist.pins[*pin].output
                        || role.starts_with("raddr")
                        || role.starts_with("addr")
                        || (role.starts_with("waddr") && kind != "lutram")
                    {
                        PinTiming::Through
                    } else {
                        PinTiming::End
                    };
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(clock_of(c));
                }
            }
            "io" => {
                launch[index] = f.input_buffer;
                capture[index] = f.output_buffer;
                // A pad carrying a clock launches what it carries as data
                // in that clock's domain, as Vivado has it, and not in
                // every one: on the Husky Plus the logic analyser samples
                // `target_hs1` itself through the clock multiplexers, and
                // timed in its own domain that was the analyser's latest
                // path, 20 ns of 4.17.
                // The pad answers to its port's name, which is what a
                // clock names (its buffer's output is another net).
                let carried = constraints
                    .clocks
                    .iter()
                    .find(|c| answers(&c.net, index))
                    .map(|c| (c.name.clone(), c.period_ns));
                domain[index] = match carried {
                    Some(clock) => domain_of(Some(clock)),
                    // Under the fitted model a pad no constraint puts in a
                    // clock's domain is not timed, as Vivado's timer does not
                    // time one (see the module docs). The legacy model
                    // times it in every domain, as every measured Husky build
                    // was placed.
                    None if model == DelayModel::Vivado => PathDomain::Untimed,
                    None => PathDomain::Every,
                };
                for pin in &inst.pins {
                    pins[*pin] = if netlist.pins[*pin].output {
                        PinTiming::Start
                    } else {
                        PinTiming::End
                    };
                }
            }
            "dsp" => {
                // What a block's pins do depends on which registers it has:
                // see `dsp_timing`. Anything that has one is in that clock's
                // domain, and a block with none is a lookup table of 3.8 ns.
                let cell_of = design.modules[top].cells.get(inst.cell);
                let param = |name: &str| {
                    cell_of
                        .and_then(|c| c.params.get(name))
                        .and_then(AttrValue::as_int)
                        .unwrap_or(1)
                };
                let mut shape = dsp_timing(
                    f,
                    param("AREG").min(param("BREG")),
                    param("MREG"),
                    param("PREG"),
                );
                // A chain of sums: what comes in on `PCIN` goes through this
                // block's adder to the next, `DSP_CASCADE` a block, and is what
                // `fir8`'s 10.988 ns is made of. Without a `P` register that is
                // the block's outputs followed from its cascade in alone: a
                // chain's first block launches, and each after it adds to that,
                // which is what the worst path through one is. With `P`, the
                // chain ends at the setup Vivado gives it.
                let cascade = inst.pins.iter().any(|p| {
                    netlist.pins[*p].role.starts_with("pcin") && netlist.pins[*p].signal.is_some()
                });
                let chained = cascade && param("PREG") == 0;
                if cascade && param("PREG") > 0 {
                    shape.capture = shape.capture.map(|c| c.max(f.dsp_setup_cascade));
                }
                if chained {
                    shape.through = f.dsp_cascade;
                }
                launch[index] = shape.launch.unwrap_or(0.0);
                capture[index] = shape.capture.unwrap_or(0.0);
                cell[index] = shape.through;
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = dsp_pin(role, netlist.pins[*pin].output, &shape, chained);
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(clock_of(c));
                }
            }
            "iodelay" => {
                // Through from its data input to `DATAOUT`, by what its
                // parameters make it (`idelay_arc`); a variable delay's
                // controls end paths on `C`, at a flip-flop's setup, which
                // was not measured, and its count starts them.
                cell[index] = design.modules[top].cells.get(inst.cell).map_or_else(
                    || idelay_arc(&crate::ir::Attrs::new(), model),
                    |c| idelay_arc(&c.params, model),
                );
                (launch[index], capture[index]) = (IDELAY_COUNT_LAUNCH, f.ff_setup);
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = match role {
                        "idatain" | "datain" | "dataout" => PinTiming::Through,
                        _ if is_clock(role) => PinTiming::Ignored,
                        _ if netlist.pins[*pin].output => PinTiming::Start,
                        _ => PinTiming::End,
                    };
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(clock_of(c));
                }
            }
            "mmcm" | "pll" | "sysmon" => {
                // A clock manager's or the XADC's register port is a
                // synchronous interface on its `DCLK`: the address, data
                // and strobes end paths, the data out and ready start
                // them. Nothing else of it is a path in the fabric's
                // sense. It used to fall to the arm below, whose clock is
                // the first pin with a clock's name — `CLKIN2`, or
                // `CLKINSEL`, tied off — so the whole port was in no
                // domain and untimed, and the logic analyser's MMCM on the
                // Husky Plus refused NewAE's divider writes in some builds
                // and took them in others.
                (launch[index], capture[index]) = (f.bram_launch, f.bram_setup);
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = match role {
                        "daddr" | "di" | "den" | "dwe" | "psen" | "psincdec" => PinTiming::End,
                        "do" | "drdy" | "psdone" => PinTiming::Start,
                        _ => PinTiming::Ignored,
                    };
                }
                // The fine phase shift is the same kind of port on `PSCLK`,
                // and the glitch generator's MMCMs on the Husky Plus have
                // it on `clk_usb` with the register port. One domain an
                // instance: `DCLK`'s, or `PSCLK`'s when it has no `DCLK`.
                let clock_of_role = |role: &str| {
                    inst.pins
                        .iter()
                        .find(|p| netlist.pins[**p].role == role)
                        .and_then(|p| netlist.pins[*p].signal)
                };
                let dclk = clock_of_role("dclk").or_else(|| clock_of_role("psclk"));
                if let Some(c) = dclk {
                    domain[index] = domain_of(clock_of(c));
                }
            }
            _ => {
                // A flip-flop, a block RAM, and anything else clocked
                // starts paths at its outputs and ends them at its inputs.
                // A latch is timed as a flip-flop on `G`, which is the edge
                // that closes it: what it holds starts a path and what it
                // takes ends one. A path *through* an open latch, which is
                // what time borrowing is, is not followed, so a design that
                // relies on one is timed as if it could not.
                (launch[index], capture[index]) = match kind {
                    "ff" | "latch" => (f.ff_launch, f.ff_setup),
                    "bram" | "bram36" | "fifo18" | "fifo36" => (f.bram_launch, f.bram_setup),
                    _ => (f.ff_launch, f.ff_setup),
                };
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = if is_clock(role) {
                        PinTiming::Ignored
                    } else if netlist.pins[*pin].output {
                        PinTiming::Start
                    } else {
                        PinTiming::End
                    };
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(clock_of(c));
                }
                // A block RAM's two ports may be on two clocks, each port's
                // pins in its own: see `PlaceTiming::pin_domain`.
                if matches!(kind, "bram" | "bram36" | "fifo18" | "fifo36") {
                    for port in ["p0_", "p1_"] {
                        let clk = format!("{port}clk");
                        let Some(c) = inst
                            .pins
                            .iter()
                            .find(|p| netlist.pins[**p].role == clk)
                            .and_then(|p| netlist.pins[*p].signal)
                        else {
                            continue;
                        };
                        let port_domain = domain_of(clock_of(c));
                        for pin in &inst.pins {
                            if netlist.pins[*pin].role.starts_with(port) {
                                pin_domain[*pin] = Some(port_domain);
                            }
                        }
                    }
                }
            }
        }
    }
    // An input delay puts a pad in its clock's domain, launched that long
    // after the clock's edge, and against the clock's arrival at the
    // register it races (`CLOCK_INSERTION`) rather than an ideal one. A
    // clock that clocks nothing here has no domain, and its delays nothing
    // to apply to.
    for delay in &constraints.input_delays {
        let Some(index) = names.iter().position(|n| *n == delay.clock) else {
            continue;
        };
        for (i, inst) in netlist.instances.iter().enumerate() {
            if inst.kind == "io" && delay.ports.iter().any(|g| answers(g, i)) {
                launch[i] = f.input_buffer + delay.delay_ns - f.clock_insertion;
                domain[i] = PathDomain::Clock(index);
            }
        }
    }
    // A maximum delay with one end gives the paths from or to the
    // instances it names that delay instead of their clock's period: their
    // launch is that much earlier, or their capture that much later, which
    // in a block-based analysis is exact for one end. NewAE give the Husky
    // Plus's bus 15 ns where `clk_usb` is 10.4, through the registers its
    // address and FIFO count are held in and the data pads' tristates.
    // A pad's or anything else's not in one clock's domain is left alone.
    for delay in &constraints.max_delays {
        let (glob, from) = match (&delay.path.from, &delay.path.to) {
            (Some(g), None) => (g, true),
            (None, Some(g)) => (g, false),
            _ => continue,
        };
        for i in 0..n {
            let PathDomain::Clock(d) = domain[i] else {
                continue;
            };
            if !answers(glob, i) {
                continue;
            }
            let relief = delay.delay_ns - periods[d];
            if from {
                launch[i] -= relief;
            } else {
                capture[i] -= relief;
            }
        }
    }
    let pin_delays = own_pin_delays(netlist, &pins, f, model);
    (
        PlaceTiming {
            pins,
            cell,
            launch,
            capture,
            domain,
            periods,
            wire: f.wire,
            rounds: 3,
            weight: 8.0,
            exponent: 3,
            reaches,
            cycles,
            untimed_from,
            untimed_to,
            frozen,
            pin_domain,
            pin_delays,
            // A dedicated group's own wires: see `PlaceTiming::local_wire`.
            // Vivado's timer puts nothing on a carry's cascade, a table's
            // output into its lane's flip-flop or carry, or a multiplexer's
            // inputs (`docs/fpga-vivado-parity.md`, "Filling slices").
            local_wire: (model != DelayModel::Legacy).then_some(0.0),
            local: Vec::new(),
        },
        names,
    )
}

/// The delays of single pins that `f` has figures for: a flip-flop's clock
/// enable and reset, a shift register's enable, a distributed RAM's write
/// enable and address, a block RAM's inputs by kind, and a carry chain's arcs.
/// Empty for [`LEGACY`], which has none.
fn own_pin_delays(
    netlist: &Netlist,
    pins: &[PinTiming],
    f: &Figures,
    model: DelayModel,
) -> PinDelays {
    let mut own = PinDelays::default();
    if model == DelayModel::Legacy {
        return own;
    }
    own.per_output = true;
    own.capture = vec![None; netlist.pins.len()];
    own.through = vec![None; netlist.pins.len()];
    for inst in &netlist.instances {
        let kind = inst.kind.as_str();
        for pin in &inst.pins {
            let role = netlist.pins[*pin].role.as_str();
            if pins[*pin] != PinTiming::End {
                continue;
            }
            let setup = match kind {
                "ff" | "latch" => f.ff_setup_enable.and_then(|(enable, reset)| match role {
                    "en" => Some(enable),
                    "rst" => Some(reset),
                    _ => None,
                }),
                "srl16" | "srl32" => f.srl_setup_enable.filter(|_| role == "ce"),
                k if k == "lutram" || k.starts_with("lutram_") => {
                    f.lutram_setup_pins.and_then(|(enable, address)| {
                        if role == "we" {
                            Some(enable)
                        } else if role.starts_with("waddr") || role.starts_with("addr") {
                            Some(address)
                        } else {
                            None
                        }
                    })
                }
                "bram" | "bram36" | "fifo18" | "fifo36" => {
                    f.bram_setup_pins.and_then(|(data, enable, write)| {
                        let role = role.split_once('_').map_or(role, |(_, r)| r);
                        if role.starts_with("din") {
                            Some(data)
                        } else if role.starts_with("we") {
                            Some(write)
                        } else if role == "en" {
                            Some(enable)
                        } else {
                            None
                        }
                    })
                }
                _ => None,
            };
            own.capture[*pin] = setup;
        }
        if kind != "carry" {
            continue;
        }
        let Some(arcs) = f.carry_arcs else { continue };
        // Each `Through` output's place among the instance's, as the
        // analysis counts them, and where it is in the table's order.
        let outputs: Vec<Option<usize>> = inst
            .pins
            .iter()
            .filter(|p| netlist.pins[**p].output && pins[**p] == PinTiming::Through)
            .map(|p| {
                let role = netlist.pins[*p].role.as_str();
                let (base, bit) = role.split_at(role.len() - 1);
                let bit = bit.parse::<usize>().ok()?;
                match base {
                    "s" if bit < 4 => Some(bit),
                    "co" if bit < 4 => Some(4 + bit),
                    _ => None,
                }
            })
            .collect();
        if outputs.len() > 8 {
            continue;
        }
        for pin in &inst.pins {
            if netlist.pins[*pin].output || pins[*pin] != PinTiming::Through {
                continue;
            }
            let role = netlist.pins[*pin].role.as_str();
            let Some((_, table)) = arcs.iter().find(|(r, _)| *r == role) else {
                continue;
            };
            let mut row = [f.carry; 8];
            for (j, at) in outputs.iter().enumerate() {
                if let Some(at) = at {
                    row[j] = table[*at];
                }
            }
            own.through[*pin] = Some(row);
        }
    }
    own
}

/// A flip-flop's data input's setup by how its data reaches it, in a `SLICEL`
/// and a `SLICEM`: from a cell of its own slice, inside the site; through the
/// lane's bypass pin `<L>X`; and through the lane's lookup table, a way
/// ([`super::thru`]). See [`place_timing_on_sites`] for where each was read.
const SETUP_INSIDE: (f64, f64) = (-0.039, -0.082);
/// See [`SETUP_INSIDE`].
const SETUP_BYPASS: (f64, f64) = (0.082, 0.058);
/// See [`SETUP_INSIDE`].
const SETUP_WAY: (f64, f64) = (0.122, 0.083);

/// What a placement says of the cells' own figures, for a `model` that has
/// them: a flip-flop's clock to `Q` by its slice and bel, 0.341 ns in a
/// `SLICEL`'s `AFF` to `DFF` and 0.393 in a `SLICEM`'s, 0.313 and 0.361 in
/// their `A5FF` to `D5FF` (Vivado 2026.1, grade 3, every flip-flop of the
/// corpus: `arcs.txt`). And its data input's setup by where the data comes
/// from, in a `SLICEL` and a `SLICEM`: -0.039 and -0.082 ns from a cell of its
/// own slice, which reaches it inside the site (`SETUP_INSIDE`); 0.082 and
/// 0.058 through the slice's bypass pin from anywhere else (`SETUP_BYPASS`);
/// and 0.122 and 0.083 through the lane's table on one of `ways`, the ways the
/// router is given ([`crate::fpga::place::route_throughs_needed`];
/// `SETUP_WAY`). A way ends at the lane's `<L>FF` alone, and is open only
/// when the lane's signals do not fit without it, so a flip-flop on that bel
/// fed from elsewhere takes it.
///
/// Each is the mean of Vivado's setup over the flip-flops of three seeds of
/// the timing corpus as this flow placed and routed them, 13,224 in all,
/// classed by the site pin Vivado says each is reached by
/// (`docs/fpga-vivado-parity.md`, "Filling slices"). Before, -0.050 from the
/// slice and 0.079 from anywhere else, measured before there were ways, were
/// every slice's: a `SLICEM`'s ends were 0.02 to 0.03 ns late and a way's in a
/// `SLICEL` 0.04 early. A pin's setup is one of two figures about 0.1 ns apart,
/// by something not read, and these are means over both. One figure
/// for all, the 0.027 before placement, put a register-to-register path's end
/// 0.05 ns early, and those were the endpoints the estimate was furthest under
/// Vivado on. Nothing for [`DelayModel::Legacy`], whose one figure is every
/// flip-flop's.
pub fn place_timing_on_sites(
    timing: &mut PlaceTiming,
    model: DelayModel,
    netlist: &Netlist,
    graph: &RoutingGraph,
    placement: &Placement,
    ways: &[NodeId],
) {
    if model == DelayModel::Legacy {
        return;
    }
    // The data inputs a way open to the router ends on.
    let through: HashSet<NodeId> = ways
        .iter()
        .flat_map(|way| graph.outgoing(*way).iter().map(|pip| graph.pip(*pip).to))
        .collect();
    for (index, inst) in netlist.instances.iter().enumerate() {
        if inst.kind != "ff" {
            continue;
        }
        let Some(site) = placement.site_of(index) else {
            continue;
        };
        let bel = graph.sites[site].bel.as_str();
        let slicem = bel.starts_with("SLICEM");
        let five = bel.ends_with("5FF");
        timing.launch[index] = match (slicem, five) {
            (false, false) => 0.341,
            (false, true) => 0.313,
            (true, false) => 0.393,
            (true, true) => 0.361,
        };
        // The slice a bel is in: `SLICEL_X0_AFF` is in `SLICEL_X0`.
        let slice_of = |site: usize| {
            let site = &graph.sites[site];
            let bel = site.bel.as_str();
            let end = bel.match_indices('_').nth(1).map_or(bel.len(), |(i, _)| i);
            (site.tile, &bel[..end])
        };
        let here = slice_of(site);
        let by_slice = |(l, m): (f64, f64)| if slicem { m } else { l };
        let way = graph.sites[site]
            .pin("d")
            .is_some_and(|d| through.contains(&d));
        for pin in &inst.pins {
            let p = &netlist.pins[*pin];
            if p.role != "d" || timing.pin_delays.capture.len() <= *pin {
                continue;
            }
            let inside = p
                .signal
                .and_then(|s| netlist.signals[s].driver)
                .and_then(|d| placement.site_of(netlist.pins[d].instance))
                .is_some_and(|d| slice_of(d) == here);
            timing.pin_delays.capture[*pin] = Some(if inside {
                by_slice(SETUP_INSIDE)
            } else if way {
                by_slice(SETUP_WAY)
            } else {
                by_slice(SETUP_BYPASS)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::place::{Instance, NetPin, Signal};
    use crate::ir::{CellId, Const, Id};

    /// **A flip-flop's setup is the one for how its data reaches it, in its
    /// kind of slice.** Two tiles of a table and three flip-flops, a
    /// `SLICEL`'s `AFF` and `A5FF` and a `SLICEM`'s `AFF`, each `AFF`'s `D`
    /// reachable by a way of its own. A flip-flop fed from its own slice's
    /// table gets [`SETUP_INSIDE`]; one fed from the other tile
    /// [`SETUP_BYPASS`], or [`SETUP_WAY`] when its lane's way is among those
    /// open, each the `SLICEL`'s or the `SLICEM`'s figure by its bel; the
    /// `A5FF` the bypass's whatever is open, as no way reaches it. The legacy
    /// model sets none.
    ///
    /// Would catch the ways ignored, a way taken for one it does not reach,
    /// the two slices' figures swapped and the slice read from the driver's
    /// bel. Would not catch a figure itself being wrong: those are Vivado's
    /// means (`docs/fpga-vivado-parity.md`, "Filling slices"), which only a
    /// run against Vivado's timer asks, nor the router passing a way by,
    /// which `route_throughs_needed` opening only what a lane needs rules out.
    #[test]
    fn a_flip_flops_setup_is_the_one_for_how_its_data_arrives() {
        use crate::fpga::arch::{Arch, BelDecl, PipDecl, TileType, WireDecl, WireRef};
        let mut arch = Arch::new("t", "test", 2, 1);
        let mut tile = TileType::new("clb", "logic_tile", 8, 8);
        for name in ["o6", "byp", "way_l", "way_m", "d_l", "d_l5", "d_m", "i"] {
            tile.wires.push(WireDecl {
                name: name.to_owned(),
                dx: 0,
                dy: 0,
            });
        }
        for (from, to) in [
            ("o6", "d_l"),
            ("o6", "byp"),
            ("byp", "d_l"),
            ("byp", "d_l5"),
            ("byp", "d_m"),
            ("way_l", "d_l"),
            ("way_m", "d_m"),
        ] {
            tile.pips.push(PipDecl {
                from: WireRef::local(from),
                to: WireRef::local(to),
                bits: Vec::new(),
            });
        }
        let mut lut = BelDecl::new("SLICEL_X0_A6LUT", "lut");
        lut.pins.push(("i0".to_owned(), WireRef::local("i")));
        lut.pins.push(("o".to_owned(), WireRef::local("o6")));
        tile.bels.push(lut);
        for (name, d) in [
            ("SLICEL_X0_AFF", "d_l"),
            ("SLICEL_X0_A5FF", "d_l5"),
            ("SLICEM_X0_AFF", "d_m"),
        ] {
            let mut ff = BelDecl::new(name, "ff");
            ff.pins.push(("d".to_owned(), WireRef::local(d)));
            tile.bels.push(ff);
        }
        arch.tile_types.push(tile);
        arch.set_tile(0, 0, 0);
        arch.set_tile(1, 0, 0);
        let graph = arch.build_graph();
        let site = |x: u32, bel: &str| {
            (0..graph.sites.len())
                .find(|s| graph.sites[*s].tile == (x, 0) && graph.sites[*s].bel == bel)
                .unwrap()
        };
        let node = |x: u32, name: &str| {
            (0..graph.nodes.len())
                .find(|n| graph.nodes[*n].name == name && graph.nodes[*n].tile == (x, 0))
                .map(|n| NodeId::try_from(n).unwrap())
                .unwrap()
        };
        // Instances: 0 a table in tile 0, 1 a table in tile 1; then the
        // flip-flops, 2 in tile 0's `SLICEL` `AFF` reading table 0, and 3, 4
        // and 5 in tile 1's `AFF`, `A5FF` and `SLICEM` `AFF` reading table 0.
        type Spec<'a> = (&'a str, &'a [(&'a str, usize, bool)]);
        let specs: [Spec<'_>; 6] = [
            ("lut", &[("o", 0, true)]),
            ("lut", &[("o", 1, true)]),
            ("ff", &[("d", 0, false)]),
            ("ff", &[("d", 0, false)]),
            ("ff", &[("d", 0, false)]),
            ("ff", &[("d", 0, false)]),
        ];
        let mut netlist = Netlist {
            instances: Vec::new(),
            pins: Vec::new(),
            signals: (0..2)
                .map(|n| Signal {
                    name: format!("s{n}"),
                    driver: None,
                    sinks: Vec::new(),
                })
                .collect(),
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        for (instance, (kind, pins)) in specs.iter().enumerate() {
            let mut own = Vec::new();
            for (role, signal, output) in *pins {
                let pin = netlist.pins.len();
                netlist.pins.push(NetPin {
                    instance,
                    port: role.to_uppercase(),
                    bit: 0,
                    role: (*role).to_owned(),
                    output: *output,
                    signal: Some(*signal),
                    constant: None,
                });
                if *output {
                    netlist.signals[*signal].driver = Some(pin);
                } else {
                    netlist.signals[*signal].sinks.push(pin);
                }
                own.push(pin);
            }
            netlist.instances.push(Instance {
                cell: CellId::from_index(instance),
                name: format!("{kind}{instance}"),
                primitive: kind.to_uppercase(),
                kind: (*kind).to_owned(),
                pins: own,
                pin: None,
            });
        }
        let mut placement = Placement::new(6, graph.sites.len());
        for (instance, (x, bel)) in [
            (0, "SLICEL_X0_A6LUT"),
            (1, "SLICEL_X0_A6LUT"),
            (0, "SLICEL_X0_AFF"),
            (1, "SLICEL_X0_AFF"),
            (1, "SLICEL_X0_A5FF"),
            (1, "SLICEM_X0_AFF"),
        ]
        .iter()
        .enumerate()
        {
            placement.place(instance, site(*x, bel));
        }
        let n = netlist.pins.len();
        let timing = PlaceTiming {
            pins: vec![PinTiming::Ignored; n],
            cell: vec![0.0; 6],
            launch: vec![0.0; 6],
            capture: vec![0.0; 6],
            domain: vec![PathDomain::Untimed; 6],
            cycles: Vec::new(),
            untimed_from: Vec::new(),
            untimed_to: Vec::new(),
            periods: Vec::new(),
            wire: (0.0, 0.0, 0.0),
            rounds: 0,
            weight: 0.0,
            exponent: 1,
            reaches: Vec::new(),
            frozen: Vec::new(),
            pin_domain: Vec::new(),
            pin_delays: PinDelays {
                capture: vec![None; n],
                ..PinDelays::default()
            },
            local_wire: None,
            local: Vec::new(),
        };
        let setups = |model: DelayModel, ways: &[NodeId]| {
            let mut timing = timing.clone();
            place_timing_on_sites(&mut timing, model, &netlist, &graph, &placement, ways);
            (2..6)
                .map(|ff| timing.pin_delays.capture[netlist.instances[ff].pins[0]])
                .collect::<Vec<_>>()
        };
        let (l, m) = (0, 1);
        let pick = |pair: (f64, f64), which: usize| Some(if which == l { pair.0 } else { pair.1 });
        assert_eq!(
            setups(DelayModel::Vivado, &[]),
            vec![
                pick(SETUP_INSIDE, l),
                pick(SETUP_BYPASS, l),
                pick(SETUP_BYPASS, l),
                pick(SETUP_BYPASS, m),
            ]
        );
        let ways = [node(1, "way_l"), node(1, "way_m")];
        assert_eq!(
            setups(DelayModel::Vivado, &ways),
            vec![
                pick(SETUP_INSIDE, l),
                pick(SETUP_WAY, l),
                pick(SETUP_BYPASS, l),
                pick(SETUP_WAY, m),
            ]
        );
        // Tile 0's way is not tile 1's.
        assert_eq!(
            setups(DelayModel::Vivado, &[node(0, "way_l")])[1],
            pick(SETUP_BYPASS, l)
        );
        assert_eq!(setups(DelayModel::Legacy, &ways), vec![None; 4]);
    }

    /// **A `LUT6_2`'s inputs reach the half that reads them.** One table,
    /// `O6` and `O5`, `I5` tied high so `O6` is the upper half: the lower
    /// half is `I1`, the upper `I0 & I2`. So `I0` and `I2` reach `O6` only,
    /// `I1` reaches `O5` only, and `I3` neither. Would catch the halves
    /// swapped, `I5` taken as a signal (the whole table, so `I1` would reach
    /// `O6` too), and a mask that ignores the truth table; would not catch a
    /// table whose model differs from the silicon's, which the mapped
    /// netlist's own checks are for.
    /// Each register setting against `report_timing`'s figure for it. Would
    /// catch a register read for another or a setup taken from the wrong one.
    /// Whether the figures hold of a placed path is a different question,
    /// which only a comparison with Vivado's own routed report asks.
    #[test]
    fn a_blocks_paths_are_the_ones_its_registers_make() {
        let none = dsp_timing(&LEGACY, 0, 0, 0);
        assert_eq!((none.launch, none.capture), (None, None));
        assert!((none.through - 3.841).abs() < 1e-9);
        assert_eq!(dsp_timing(&LEGACY, 0, 0, 1).launch, Some(0.434));
        assert_eq!(dsp_timing(&LEGACY, 0, 0, 1).capture, Some(3.722));
        assert_eq!(dsp_timing(&LEGACY, 0, 1, 0).launch, Some(2.305));
        assert_eq!(dsp_timing(&LEGACY, 0, 1, 0).capture, Some(1.928));
        assert_eq!(dsp_timing(&LEGACY, 1, 0, 0).launch, Some(4.009));
        assert_eq!(dsp_timing(&LEGACY, 2, 1, 1).capture, Some(0.45));
        assert_eq!(dsp_timing(&LEGACY, 2, 1, 1).launch, Some(0.434));
    }

    /// **A chain of sums is one long path**, as Vivado's timer has `fir8`'s: `PCIN`
    /// goes on to `PCOUT` through a block with no `P`, and ends at a block with one.
    /// Would catch each block's cascade ending there, which is what made `fir8` look
    /// like 4.9 ns beside Vivado's 10.988; would not catch a figure wrong, or say
    /// that the estimate of a routed chain agrees with Vivado's, which nothing has.
    #[test]
    fn a_chain_of_sums_is_one_long_path() {
        let mut middle = dsp_timing(&LEGACY, 2, 1, 0);
        middle.through = DSP_CASCADE;
        assert_eq!(dsp_pin("pcin7", false, &middle, true), PinTiming::Through);
        assert_eq!(dsp_pin("pcout7", true, &middle, true), PinTiming::Through);
        assert_eq!(dsp_pin("a3", false, &middle, true), PinTiming::End);
        assert_eq!(dsp_pin("clk", false, &middle, true), PinTiming::Ignored);
        let last = dsp_timing(&LEGACY, 2, 1, 1);
        assert_eq!(dsp_pin("pcin7", false, &last, false), PinTiming::End);
        assert_eq!(dsp_pin("p3", true, &last, false), PinTiming::Start);
    }

    /// **Under the Vivado model a carry chain's arcs and a flip-flop's
    /// enable and reset are their own.** A `CARRY4` with every pin, outputs in
    /// the order the device file gives them (`O` then `CO`), and an `FDRE`:
    /// `CI` reaches `CO3` in 0.089 ns and `S0` in 0.395, `DI1` does not reach
    /// `O0` at all, and the flip-flop's enable needs 0.218 ns and its reset
    /// 0.33 where its data input takes the instance's 0.027. The legacy model
    /// has none of it. Would catch the outputs counted in another order, an arc
    /// read from the wrong row or column, a pin's setup given to another, or
    /// the legacy model given the new figures; would not catch a figure wrong
    /// about the part, which `testdata/fpga/vivado/timing/arcs.txt` is.
    #[test]
    fn a_carry_chains_arcs_and_a_flip_flops_setups_are_their_own() {
        let mut pins = Vec::new();
        let mut instance_pins = [Vec::new(), Vec::new()];
        let mut add = |instance: usize, role: String, output: bool| {
            instance_pins[instance].push(pins.len());
            pins.push(NetPin {
                instance,
                port: role.to_uppercase(),
                bit: 0,
                role,
                output,
                signal: None,
                constant: None,
            });
        };
        add(0, "ci".to_owned(), false);
        add(0, "cyinit".to_owned(), false);
        for i in 0..4 {
            add(0, format!("p{i}"), false);
            add(0, format!("di{i}"), false);
        }
        for i in 0..4 {
            add(0, format!("s{i}"), true);
        }
        for i in 0..4 {
            add(0, format!("co{i}"), true);
        }
        for (role, output) in [
            ("clk", false),
            ("d", false),
            ("en", false),
            ("rst", false),
            ("q", true),
        ] {
            add(1, role.to_owned(), output);
        }
        let instance = |kind: &str, pins: Vec<usize>| Instance {
            cell: CellId::from_index(0),
            name: kind.to_owned(),
            primitive: kind.to_uppercase(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        let [carry, ff] = instance_pins;
        let netlist = Netlist {
            instances: vec![instance("carry", carry), instance("ff", ff)],
            signals: Vec::new(),
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let classes: Vec<PinTiming> = netlist
            .pins
            .iter()
            .map(|p| match (p.instance, p.role.as_str(), p.output) {
                (0, _, _) => PinTiming::Through,
                (1, "clk", _) => PinTiming::Ignored,
                (1, _, true) => PinTiming::Start,
                _ => PinTiming::End,
            })
            .collect();
        let pin = |instance: usize, role: &str| {
            netlist
                .pins
                .iter()
                .position(|p| p.instance == instance && p.role == role)
                .unwrap()
        };
        let own = own_pin_delays(&netlist, &classes, &VIVADO, DelayModel::Vivado);
        let arcs = |role: &str| own.through[pin(0, role)].unwrap();
        assert!((arcs("ci")[7] - 0.089).abs() < 1e-9);
        assert!((arcs("p0")[7] - 0.395).abs() < 1e-9);
        assert!((arcs("p0")[0] - 0.187).abs() < 1e-9);
        assert_eq!(arcs("di1")[0], f64::NEG_INFINITY);
        assert_eq!(own.capture[pin(1, "en")], Some(0.218));
        assert_eq!(own.capture[pin(1, "rst")], Some(0.33));
        assert_eq!(own.capture[pin(1, "d")], None);
        let legacy = own_pin_delays(&netlist, &classes, &LEGACY, DelayModel::Legacy);
        assert_eq!(legacy, PinDelays::default());
    }

    #[test]
    fn a_paired_tables_inputs_reach_the_half_that_reads_them() {
        let mut init = Const::zero(64);
        for i in 0..64u32 {
            let bit = |k: u32| i & (1 << k) != 0;
            let value = if bit(5) { bit(0) && bit(2) } else { bit(1) };
            if value {
                init.set_bit(i, Bit::One);
            }
        }
        let mut pins = Vec::new();
        for (role, output, constant) in [
            ("i0", false, None),
            ("i1", false, None),
            ("i2", false, None),
            ("i3", false, None),
            ("i5", false, Some(Bit::One)),
            ("o", true, None),
            ("o5", true, None),
        ] {
            pins.push(NetPin {
                instance: 0,
                port: role.to_uppercase(),
                bit: 0,
                role: role.to_owned(),
                output,
                signal: if constant.is_some() {
                    None
                } else {
                    Some(pins.len())
                },
                constant,
            });
        }
        let netlist = Netlist {
            instances: vec![Instance {
                cell: CellId::from_index(0),
                name: "t".to_owned(),
                primitive: "LUT6_2".to_owned(),
                kind: "lut".to_owned(),
                pins: (0..pins.len()).collect(),
                pin: None,
            }],
            signals: (0..pins.len())
                .map(|i| Signal {
                    name: format!("s{i}"),
                    driver: None,
                    sinks: Vec::new(),
                })
                .collect(),
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let mut reaches = vec![u8::MAX; netlist.pins.len()];
        lut_reaches(&netlist, 0, &init, &mut reaches);
        // Bit 0 is `O6` (the first output pin), bit 1 `O5`.
        assert_eq!(&reaches[..4], &[0b01, 0b10, 0b01, 0b00]);
    }

    /// **An input delay puts its pad in its clock's domain, launched late.**
    /// A clock pad through a global buffer clocks a flip-flop that reads
    /// two other pads, `rd` and `data`; `rd` alone has an input delay of
    /// a period and two nanoseconds. It launches at that, plus its buffer,
    /// less the clock's arrival, and only in the clock's domain; `data`
    /// launches at its buffer in every domain under the legacy model, as a
    /// pad always did, and in none under the fitted one; and the clock's own
    /// pad in the clock's domain. Would catch the delay ignored,
    /// applied to the wrong pad, applied without the clock's arrival taken
    /// off, or left in every domain, or a clock's pad launching in every
    /// domain (as it did), or a maximum delay not moving the launch of the
    /// pad it is from or the capture of the register it is to by its
    /// difference from the period, or moving another's, or the fitted model
    /// timing a pad no constraint names, or not timing one an input delay
    /// names; would not
    /// catch [`CLOCK_INSERTION`] being the wrong number for a part, which
    /// only a part says.
    #[test]
    fn an_input_delay_launches_its_pad_late_in_its_clocks_domain() {
        let pin = |instance: usize, role: &str, output: bool, signal: usize| NetPin {
            instance,
            port: role.to_uppercase(),
            bit: 0,
            role: role.to_owned(),
            output,
            signal: Some(signal),
            constant: None,
        };
        let instance = |i: usize, name: &str, kind: &str, pins: Vec<usize>| Instance {
            cell: CellId::from_index(i),
            name: name.to_owned(),
            primitive: String::new(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        // Signals: 0 the clock pad, 1 the buffered clock, 2 `rd`, 3 `data`.
        let pins = vec![
            pin(0, "din", true, 0),
            pin(1, "i", false, 0),
            pin(1, "o", true, 1),
            pin(2, "din", true, 2),
            pin(3, "din", true, 3),
            pin(4, "clk", false, 1),
            pin(4, "d", false, 2),
            pin(4, "ce", false, 3),
        ];
        let signal = |name: &str, driver: usize, sinks: Vec<usize>| Signal {
            name: name.to_owned(),
            driver: Some(driver),
            sinks,
        };
        let netlist = Netlist {
            instances: vec![
                instance(0, "clk$io0", "io", vec![0]),
                instance(1, "clk$gbuf", "gb", vec![1, 2]),
                instance(2, "rd$io0", "io", vec![3]),
                instance(3, "data$io0", "io", vec![4]),
                instance(4, "q", "ff", vec![5, 6, 7]),
            ],
            signals: vec![
                signal("clk", 0, vec![1]),
                signal("clk$gb", 2, vec![5]),
                signal("rd", 3, vec![6]),
                signal("data", 4, vec![7]),
            ],
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.rcf", "").unwrap();
        let mut diags = crate::diag::Diagnostics::new();
        let constraints = Constraints::parse(
            "create_clock -name bus -period 10 clk\nset_input_delay -clock bus 12 rd\n",
            file,
            &mut diags,
        );
        assert!(!diags.has_errors());
        let mut design = Design::new();
        let top = design.add_module(crate::ir::Module::new(
            "t",
            crate::source::Span::new(file, 0, 0),
        ));
        let (timing, names) =
            place_timing(&design, top, &netlist, &constraints, DelayModel::Legacy);
        assert_eq!(names, vec!["bus".to_owned()]);
        assert!((timing.launch[2] - (INPUT_BUFFER + 12.0 - CLOCK_INSERTION)).abs() < 1e-9);
        assert_eq!(timing.domain[2], PathDomain::Clock(0));
        assert!((timing.launch[3] - INPUT_BUFFER).abs() < 1e-9);
        assert_eq!(timing.domain[3], PathDomain::Every);
        // The clock's own pad launches what it carries in its clock's
        // domain alone.
        assert_eq!(timing.domain[0], PathDomain::Clock(0));

        // A maximum delay of 15 ns from `rd` launches it five earlier than
        // its 10 ns clock would; one to `q` captures there five later.
        let relaxed = Constraints::parse(
            "create_clock -name bus -period 10 clk\nset_input_delay -clock bus 12 rd\n\
             set_max_delay 15 -from rd\nset_max_delay 15 -to q\n",
            file,
            &mut diags,
        );
        assert!(!diags.has_errors());
        let (eased, _) = place_timing(&design, top, &netlist, &relaxed, DelayModel::Legacy);
        assert!((timing.launch[2] - eased.launch[2] - 5.0).abs() < 1e-9);
        assert!((timing.capture[4] - eased.capture[4] - 5.0).abs() < 1e-9);
        assert!(
            (timing.launch[3] - eased.launch[3]).abs() < 1e-9,
            "`data` is untouched"
        );

        // Under the fitted model `data`, which no constraint names, is not
        // timed at all, as Vivado's timer times no pad without a delay on
        // it; `rd` and the clock's own pad are timed as before.
        let (fitted, _) = place_timing(&design, top, &netlist, &constraints, DelayModel::Vivado);
        assert_eq!(fitted.domain[3], PathDomain::Untimed);
        assert_eq!(fitted.domain[2], PathDomain::Clock(0));
        assert_eq!(fitted.domain[0], PathDomain::Clock(0));
    }

    /// **What a buffer choosing between two clocks drives is timed at the
    /// faster.** Two pads with clocks of 10 and 4 ns into a `BUFGMUX`'s
    /// first and second inputs, and a flip-flop on its output: the
    /// flip-flop is in the 4 ns clock's domain. Followed through the first
    /// input alone it was in the 10 ns one; on the Husky Plus the logic
    /// analyser's clock was the second input of the buffer clocking its
    /// FIFO, and the FIFO's write was timed in no domain. Would catch the
    /// second input ignored, or the slower clock taken; would not catch a
    /// mux whose select is a constant (a `set_case_analysis`), which this
    /// does not read and Vivado would.
    #[test]
    fn a_clock_multiplexers_output_is_timed_at_its_faster_clock() {
        let pin = |instance: usize, role: &str, output: bool, signal: usize| NetPin {
            instance,
            port: role.to_uppercase(),
            bit: 0,
            role: role.to_owned(),
            output,
            signal: Some(signal),
            constant: None,
        };
        let instance = |i: usize, name: &str, kind: &str, pins: Vec<usize>| Instance {
            cell: CellId::from_index(i),
            name: name.to_owned(),
            primitive: String::new(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        // Signals: 0 `slow`, 1 `fast`, 2 the multiplexed clock, 3 `d`.
        let pins = vec![
            pin(0, "din", true, 0),
            pin(1, "din", true, 1),
            pin(2, "i", false, 0),
            pin(2, "i1", false, 1),
            pin(2, "o", true, 2),
            pin(3, "din", true, 3),
            pin(4, "clk", false, 2),
            pin(4, "d", false, 3),
        ];
        let signal = |name: &str, driver: usize, sinks: Vec<usize>| Signal {
            name: name.to_owned(),
            driver: Some(driver),
            sinks,
        };
        let netlist = Netlist {
            instances: vec![
                instance(0, "slow$io0", "io", vec![0]),
                instance(1, "fast$io0", "io", vec![1]),
                instance(2, "mux", "gb", vec![2, 3, 4]),
                instance(3, "d$io0", "io", vec![5]),
                instance(4, "q", "ff", vec![6, 7]),
            ],
            signals: vec![
                signal("slow", 0, vec![2]),
                signal("fast", 1, vec![3]),
                signal("muxed", 4, vec![6]),
                signal("d", 5, vec![7]),
            ],
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.rcf", "").unwrap();
        let mut diags = crate::diag::Diagnostics::new();
        let constraints = Constraints::parse(
            "create_clock -name slow -period 10 slow\ncreate_clock -name fast -period 4 fast\n",
            file,
            &mut diags,
        );
        assert!(!diags.has_errors());
        let mut design = Design::new();
        let top = design.add_module(crate::ir::Module::new(
            "t",
            crate::source::Span::new(file, 0, 0),
        ));
        let (timing, names) =
            place_timing(&design, top, &netlist, &constraints, DelayModel::Legacy);
        let PathDomain::Clock(d) = timing.domain[4] else {
            panic!("the flip-flop is in no domain: {:?}", timing.domain[4]);
        };
        assert_eq!(names[d], "fast");
        assert!((timing.periods[d] - 4.0).abs() < 1e-9);
    }

    /// **A clock manager's register port is timed on its `DCLK`.** An MMCM
    /// whose `CLKIN2` and `CLKINSEL` are tied off, reference clock `ref`,
    /// register clock `bus`, written by a flip-flop on `bus` and read back
    /// by another. The MMCM is in `bus`'s domain, its address an endpoint
    /// and its data out a launch at the block RAM's figure, its fine phase
    /// shift's enable an endpoint and its done a launch, and its clock
    /// output no path at all. Its clock used to be the first pin named for
    /// one, `CLKIN2`, which carries nothing, so the whole port was
    /// untimed. Would catch the domain taken from another pin again, or a
    /// clock output timed as data; would not catch the stand-in figures
    /// being wrong for the part, which DS181 has and this does not.
    #[test]
    fn a_clock_managers_register_port_is_timed_on_its_dclk() {
        let pin = |instance: usize, role: &str, output: bool, signal: Option<usize>| NetPin {
            instance,
            port: role.to_uppercase(),
            bit: 0,
            role: role.to_owned(),
            output,
            signal,
            constant: None,
        };
        let instance = |i: usize, name: &str, kind: &str, pins: Vec<usize>| Instance {
            cell: CellId::from_index(i),
            name: name.to_owned(),
            primitive: String::new(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        // Signals: 0 `ref`, 1 `bus`, 2 the address, 3 the data out, 4 the
        // clock out.
        let pins = vec![
            pin(0, "din", true, Some(0)),
            pin(1, "din", true, Some(1)),
            pin(2, "clkin2", false, None),
            pin(2, "clkinsel", false, None),
            pin(2, "ref", false, Some(0)),
            pin(2, "dclk", false, Some(1)),
            pin(2, "daddr", false, Some(2)),
            pin(2, "do", true, Some(3)),
            pin(2, "out0", true, Some(4)),
            pin(2, "psen", false, Some(2)),
            pin(2, "psdone", true, Some(5)),
            pin(3, "clk", false, Some(1)),
            pin(3, "q", true, Some(2)),
            pin(4, "clk", false, Some(1)),
            pin(4, "d", false, Some(3)),
        ];
        let signal = |name: &str, driver: usize, sinks: Vec<usize>| Signal {
            name: name.to_owned(),
            driver: Some(driver),
            sinks,
        };
        let netlist = Netlist {
            instances: vec![
                instance(0, "ref$io0", "io", vec![0]),
                instance(1, "bus$io0", "io", vec![1]),
                instance(2, "mmcm", "mmcm", vec![2, 3, 4, 5, 6, 7, 8, 9, 10]),
                instance(3, "addr", "ff", vec![11, 12]),
                instance(4, "back", "ff", vec![13, 14]),
            ],
            signals: vec![
                signal("ref", 0, vec![4]),
                signal("bus", 1, vec![5, 11, 13]),
                signal("addr", 12, vec![6, 9]),
                signal("do", 7, vec![14]),
                signal("out0", 8, vec![]),
                signal("psdone", 10, vec![]),
            ],
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.rcf", "").unwrap();
        let mut diags = crate::diag::Diagnostics::new();
        let constraints = Constraints::parse(
            "create_clock -name bus -period 10 bus\ncreate_clock -name ref -period 40 ref\n",
            file,
            &mut diags,
        );
        assert!(!diags.has_errors());
        let mut design = Design::new();
        let top = design.add_module(crate::ir::Module::new(
            "t",
            crate::source::Span::new(file, 0, 0),
        ));
        let (timing, names) =
            place_timing(&design, top, &netlist, &constraints, DelayModel::Legacy);
        let bus = names
            .iter()
            .position(|n| n == "bus")
            .expect("bus is a domain");
        assert_eq!(timing.domain[2], PathDomain::Clock(bus));
        assert_eq!(timing.pins[6], PinTiming::End, "the address");
        assert_eq!(timing.pins[7], PinTiming::Start, "the data out");
        assert_eq!(timing.pins[8], PinTiming::Ignored, "the clock out");
        assert_eq!(timing.pins[9], PinTiming::End, "the phase shift's enable");
        assert_eq!(timing.pins[10], PinTiming::Start, "the phase shift's done");
        assert_eq!(timing.pins[4], PinTiming::Ignored, "the reference clock");
        assert!((timing.launch[2] - BRAM_LAUNCH).abs() < 1e-9);
        assert!((timing.capture[2] - BRAM_SETUP).abs() < 1e-9);
    }

    /// **A clock manager's outputs are clocks of their own.** An MMCM
    /// whose reference is the pad `ref`, multiplying by `6.0` (text, as a
    /// black box keeps a real) and dividing `CLKOUT0` by `5.0` and
    /// `CLKOUT1` by 12, clocks one flip-flop from each output, the first
    /// through a global buffer. With `ref` at 10 ns the two are domains of
    /// 10 × 5 ÷ 6 and 10 × 12 ÷ 6 ns, named for the manager's outputs; with
    /// no clock on `ref`, its `CLKIN1_PERIOD` of 5 ns stands in. Would catch
    /// an output left untimed (as every one was), the divider of one
    /// output used for the other, `CLKOUT0`'s fractional divider not read,
    /// the buffer not looked through, or a real given as text read as no
    /// number; would not catch a manager's own clock out of its range,
    /// which the bitstream writer refuses, nor a runtime divider write,
    /// which no static analysis sees.
    #[test]
    fn a_clock_managers_outputs_are_clocks_of_their_own() {
        use crate::ir::{Attrs, Cell, CellKind, Name};
        let pin = |instance: usize, role: &str, output: bool, signal: usize| NetPin {
            instance,
            port: role.to_uppercase(),
            bit: 0,
            role: role.to_owned(),
            output,
            signal: Some(signal),
            constant: None,
        };
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.rcf", "").unwrap();
        let mut design = Design::new();
        let top = design.add_module(crate::ir::Module::new(
            "t",
            crate::source::Span::new(file, 0, 0),
        ));
        let mut params = Attrs::new();
        params.set("CLKFBOUT_MULT_F", "6.0");
        params.set("CLKOUT0_DIVIDE_F", "5.0");
        params.set("CLKOUT1_DIVIDE", 12i64);
        params.set("CLKIN1_PERIOD", "5.0");
        let mut cells = Vec::new();
        for (name, params) in [("pad", Attrs::new()), ("m", params)] {
            cells.push(design.modules[top].cells.push(Cell {
                name: Name::new(name),
                kind: CellKind::Blackbox(Name::new("MMCME2_ADV")),
                inputs: Vec::new(),
                outputs: Vec::new(),
                params,
                attrs: Attrs::new(),
                span: crate::source::Span::new(file, 0, 0),
            }));
        }
        let instance = |cell: CellId, name: &str, kind: &str, pins: Vec<usize>| Instance {
            cell,
            name: name.to_owned(),
            primitive: String::new(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        // Signals: 0 `ref`, 1 out0, 2 out0 buffered, 3 out1, 4 and 5 data.
        let pins = vec![
            pin(0, "din", true, 0),
            pin(1, "ref", false, 0),
            pin(1, "out0", true, 1),
            pin(1, "out1", true, 3),
            pin(2, "i", false, 1),
            pin(2, "o", true, 2),
            pin(3, "clk", false, 2),
            pin(3, "q", true, 4),
            pin(4, "clk", false, 3),
            pin(4, "q", true, 5),
        ];
        let signal = |name: &str, driver: usize, sinks: Vec<usize>| Signal {
            name: name.to_owned(),
            driver: Some(driver),
            sinks,
        };
        let netlist = Netlist {
            instances: vec![
                instance(cells[0], "ref$io0", "io", vec![0]),
                instance(cells[1], "m", "mmcm", vec![1, 2, 3]),
                instance(cells[0], "bufg", "gb", vec![4, 5]),
                instance(cells[0], "fast", "ff", vec![6, 7]),
                instance(cells[0], "slow", "ff", vec![8, 9]),
            ],
            signals: vec![
                signal("ref", 0, vec![1]),
                signal("m_out0", 2, vec![4]),
                signal("clk0", 5, vec![6]),
                signal("m_out1", 3, vec![8]),
                signal("a", 7, vec![]),
                signal("b", 9, vec![]),
            ],
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let period_of = |rcf: &str, instance: usize| -> (String, f64) {
            let mut diags = crate::diag::Diagnostics::new();
            let constraints = Constraints::parse(rcf, file, &mut diags);
            assert!(!diags.has_errors());
            let (timing, names) =
                place_timing(&design, top, &netlist, &constraints, DelayModel::Legacy);
            let PathDomain::Clock(d) = timing.domain[instance] else {
                panic!("{instance} is untimed: {:?}", timing.domain[instance]);
            };
            (names[d].clone(), timing.periods[d])
        };
        let close = |(name, got): (String, f64), want_name: &str, want: f64| {
            assert_eq!(name, want_name);
            assert!((got - want).abs() < 1e-9, "{name}: {got} ns, want {want}");
        };
        let rcf = "create_clock -name ref -period 10 ref\n";
        close(period_of(rcf, 3), "m.out0", 10.0 * 5.0 / 6.0);
        close(period_of(rcf, 4), "m.out1", 10.0 * 12.0 / 6.0);
        close(period_of("", 3), "m.out0", 5.0 * 5.0 / 6.0);
    }

    /// **An input delay's arc is Vivado's timer's**, within 1.5 ps, on every
    /// delay asked: `Prop_idelaye2_IDATAIN_DATAOUT` and `_DATAIN_DATAOUT` of the
    /// routed `idelay_sweep`, `idelay_iddr`, `idelay_lvds`, `datain_default`
    /// and `datain_value`, and of probes at 190, 210 and 300 MHz, in
    /// `HIGH_PERFORMANCE_MODE` and with the `CLOCK` pattern
    /// (`(REFCLK_FREQUENCY, HIGH_PERFORMANCE_MODE, SIGNAL_PATTERN, IDELAY_TYPE,
    /// IDELAY_VALUE, DELAY_SRC, ns)`), at grade 1 for the legacy model; and at
    /// grade 3 for the Vivado model, which Vivado said is every one of those
    /// 0.214 ns less (0.264 from `DATAIN`), checked on 22 of them, a few here.
    /// Would catch the tap off by a picosecond, the reference clock or either
    /// mode ignored, a loaded delay timed at no taps, `DATAIN`'s intercept for
    /// `IDATAIN`'s, or one grade's for the other's; would not catch the arc at
    /// another corner, which was not asked, or say that a routed path through
    /// one is Vivado's, which nothing here compares.
    #[test]
    fn an_input_delays_arc_is_vivados_timers() {
        let vivado: [(&str, &str, &str, &str, i64, &str, f64); 36] = [
            ("200.0", "FALSE", "DATA", "FIXED", 0, "IDATAIN", 0.815),
            ("200.0", "FALSE", "DATA", "FIXED", 1, "IDATAIN", 0.911),
            ("200.0", "FALSE", "DATA", "FIXED", 2, "IDATAIN", 1.007),
            ("200.0", "FALSE", "DATA", "FIXED", 3, "IDATAIN", 1.103),
            ("200.0", "FALSE", "DATA", "FIXED", 4, "IDATAIN", 1.199),
            ("200.0", "FALSE", "DATA", "FIXED", 6, "IDATAIN", 1.392),
            ("200.0", "FALSE", "DATA", "FIXED", 8, "IDATAIN", 1.583),
            ("200.0", "FALSE", "DATA", "FIXED", 13, "IDATAIN", 2.062),
            ("200.0", "FALSE", "DATA", "FIXED", 16, "IDATAIN", 2.350),
            ("200.0", "FALSE", "DATA", "FIXED", 31, "IDATAIN", 3.790),
            ("200.0", "FALSE", "DATA", "VARIABLE", 0, "IDATAIN", 0.815),
            ("200.0", "FALSE", "DATA", "VARIABLE", 5, "IDATAIN", 1.295),
            ("200.0", "FALSE", "DATA", "VAR_LOAD", 9, "IDATAIN", 1.679),
            (
                "200.0",
                "FALSE",
                "DATA",
                "VAR_LOAD_PIPE",
                0,
                "IDATAIN",
                0.815,
            ),
            ("200.0", "FALSE", "CLOCK", "FIXED", 0, "IDATAIN", 0.815),
            ("200.0", "FALSE", "CLOCK", "FIXED", 13, "IDATAIN", 1.945),
            ("200.0", "TRUE", "DATA", "FIXED", 0, "IDATAIN", 0.815),
            ("200.0", "TRUE", "DATA", "FIXED", 1, "IDATAIN", 0.905),
            ("200.0", "TRUE", "DATA", "FIXED", 8, "IDATAIN", 1.539),
            ("200.0", "TRUE", "DATA", "FIXED", 13, "IDATAIN", 1.991),
            ("200.0", "TRUE", "DATA", "FIXED", 31, "IDATAIN", 3.620),
            ("200.0", "TRUE", "CLOCK", "FIXED", 13, "IDATAIN", 1.926),
            ("200.0", "TRUE", "CLOCK", "FIXED", 31, "IDATAIN", 3.465),
            ("190.0", "FALSE", "DATA", "FIXED", 8, "IDATAIN", 1.619),
            ("190.0", "FALSE", "DATA", "FIXED", 31, "IDATAIN", 3.931),
            ("190.0", "TRUE", "DATA", "FIXED", 31, "IDATAIN", 3.759),
            ("210.0", "FALSE", "DATA", "FIXED", 0, "IDATAIN", 0.815),
            ("210.0", "FALSE", "DATA", "FIXED", 8, "IDATAIN", 1.549),
            ("210.0", "FALSE", "DATA", "FIXED", 31, "IDATAIN", 3.661),
            ("210.0", "TRUE", "DATA", "FIXED", 31, "IDATAIN", 3.493),
            ("300.0", "FALSE", "DATA", "FIXED", 0, "IDATAIN", 0.815),
            ("300.0", "FALSE", "DATA", "FIXED", 8, "IDATAIN", 1.351),
            ("300.0", "FALSE", "DATA", "FIXED", 31, "IDATAIN", 2.891),
            ("300.0", "TRUE", "DATA", "FIXED", 31, "IDATAIN", 2.736),
            ("200.0", "FALSE", "DATA", "FIXED", 0, "DATAIN", 1.012),
            ("200.0", "FALSE", "DATA", "FIXED", 21, "DATAIN", 3.027),
        ];
        for (reference, fast, pattern, kind, value, source, ns) in vivado {
            let mut params = crate::ir::Attrs::new();
            params.set("REFCLK_FREQUENCY", reference);
            params.set("HIGH_PERFORMANCE_MODE", fast);
            params.set("SIGNAL_PATTERN", pattern);
            params.set("IDELAY_TYPE", kind);
            params.set("IDELAY_VALUE", value);
            params.set("DELAY_SRC", source);
            let ours = idelay_arc(&params, DelayModel::Legacy);
            assert!(
                (ours - ns).abs() < 0.0015,
                "{reference} {fast} {pattern} {kind} {value} {source}: {ours:.4} where Vivado \
                 says {ns}"
            );
        }
        // And with nothing said, the defaults: 200 MHz, `DATA`, `IDATAIN`, 0.
        let none = crate::ir::Attrs::new();
        assert!((idelay_arc(&none, DelayModel::Legacy) - 0.815).abs() < 1e-9);
        // Grade 3, Vivado's figures for the Vivado model.
        let grade3: [(&str, &str, &str, i64, &str, f64); 7] = [
            ("200.0", "FALSE", "DATA", 0, "IDATAIN", 0.601),
            ("200.0", "FALSE", "DATA", 31, "IDATAIN", 3.576),
            ("200.0", "TRUE", "DATA", 13, "IDATAIN", 1.777),
            ("200.0", "TRUE", "CLOCK", 31, "IDATAIN", 3.251),
            ("300.0", "TRUE", "DATA", 31, "IDATAIN", 2.522),
            ("190.0", "FALSE", "DATA", 8, "IDATAIN", 1.405),
            ("200.0", "FALSE", "DATA", 21, "DATAIN", 2.763),
        ];
        for (reference, fast, pattern, value, source, ns) in grade3 {
            let mut params = crate::ir::Attrs::new();
            params.set("REFCLK_FREQUENCY", reference);
            params.set("HIGH_PERFORMANCE_MODE", fast);
            params.set("SIGNAL_PATTERN", pattern);
            params.set("IDELAY_VALUE", value);
            params.set("DELAY_SRC", source);
            let ours = idelay_arc(&params, DelayModel::Vivado);
            assert!(
                (ours - ns).abs() < 0.0015,
                "grade 3 {reference} {fast} {pattern} {value} {source}: {ours:.4} where \
                 Vivado says {ns}"
            );
        }
    }

    /// **A pad's delay is on the path from its pad**, as Vivado's timer has it:
    /// `IDATAIN` and `DATAOUT` go through it at its arc, a fixed delay of no
    /// clock, so that a register behind it is reached from the pad through the
    /// delay. It was a flip-flop to this analysis, so that the pad's path ended
    /// at the delay and the register's started there, in no domain at all.
    /// Would catch that back; the arc's figure is the test above's.
    #[test]
    fn a_pads_delay_is_on_the_path_from_its_pad() {
        let pin = |instance: usize, role: &str, output: bool, signal: usize| NetPin {
            instance,
            port: role.to_uppercase(),
            bit: 0,
            role: role.to_owned(),
            output,
            signal: Some(signal),
            constant: None,
        };
        let instance = |i: usize, name: &str, kind: &str, pins: Vec<usize>| Instance {
            cell: CellId::from_index(i),
            name: name.to_owned(),
            primitive: String::new(),
            kind: kind.to_owned(),
            pins,
            pin: None,
        };
        // Signals: 0 the pad, 1 the delayed data.
        let pins = vec![
            pin(0, "din", true, 0),
            pin(1, "idatain", false, 0),
            pin(1, "dataout", true, 1),
            pin(2, "d", false, 1),
        ];
        let signal = |name: &str, driver: usize, sinks: Vec<usize>| Signal {
            name: name.to_owned(),
            driver: Some(driver),
            sinks,
        };
        let netlist = Netlist {
            instances: vec![
                instance(0, "p$io0", "io", vec![0]),
                instance(1, "u0", "iodelay", vec![1, 2]),
                instance(2, "q", "ff", vec![3]),
            ],
            signals: vec![signal("p", 0, vec![1]), signal("d", 2, vec![3])],
            pins,
            off_fabric: Vec::new(),
            sterilises: Vec::new(),
            gate_inverted: Vec::new(),
        };
        let mut map = crate::source::SourceMap::new();
        let file = map.add("t.rcf", "").unwrap();
        let mut design = Design::new();
        let top = design.add_module(crate::ir::Module::new(
            "t",
            crate::source::Span::new(file, 0, 0),
        ));
        for (model, arc) in [(DelayModel::Legacy, 0.815), (DelayModel::Vivado, 0.601)] {
            let (timing, _) = place_timing(&design, top, &netlist, &Constraints::new(), model);
            assert_eq!(timing.pins[1], PinTiming::Through);
            assert_eq!(timing.pins[2], PinTiming::Through);
            assert!((timing.cell[1] - arc).abs() < 1e-9, "{}", timing.cell[1]);
        }
    }
}
