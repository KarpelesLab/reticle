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
//! **Not from it**, and said so: the block RAM's clock to output, the
//! input and output buffers, and the clock managers. `LIOB33.sdf` times
//! only `IBUFDISABLE`; [`BRAM_LAUNCH`], [`INPUT_BUFFER`] and
//! [`OUTPUT_BUFFER`] are the order of Xilinx's DS181 figures, not read off
//! it here.
//!
//! **Fitted, not modelled**, is a connection's delay: [`WIRE`] is a line
//! through what this flow's own router made of a placement of NewAE's
//! ChipWhisperer-Husky Plus (2026-10-21), every connection's routed delay
//! summed from its pips' delays against its tile distance — 0.35 ns at no
//! distance and 0.035 ns a tile for a signal of four sinks or fewer, and
//! about 0.025 ns a tile more for a wider one, whose tree detours.
//!
//! Paths are timed within one clock's domain. A clock is the signal at a
//! sequential cell's clock pin traced back through the global buffers;
//! its period is the `create_clock` on it, and a clock no constraint
//! names is not timed. A package pin starts and ends paths in every
//! timed domain, which is pessimistic for a pin no clock drives and is
//! what a design whose bus is synchronous to one clock wants.

use crate::fpga::Constraints;
use crate::fpga::place::{Netlist, PathDomain, PinTiming, PlaceTiming};

/// A lookup table's input to output, ns.
pub(crate) const LUT: f64 = 0.124;
/// A carry chain cell's input to output, ns: between `CIN` to `CO3`
/// (0.114) and `CYINIT` to an output (up to 0.642).
pub(crate) const CARRY: f64 = 0.3;
/// A slice flip-flop's clock to `Q`, ns.
pub(crate) const FF_LAUNCH: f64 = 0.34;
/// A slice flip-flop's setup, ns.
pub(crate) const FF_SETUP: f64 = 0.109;
/// A block RAM's clock to output without its output register, ns.
pub(crate) const BRAM_LAUNCH: f64 = 2.45;
/// A block RAM's input setup, ns.
pub(crate) const BRAM_SETUP: f64 = 0.5;
/// A pad to the fabric through an input buffer, ns.
pub(crate) const INPUT_BUFFER: f64 = 1.2;
/// The fabric to a pad through an output buffer, ns.
pub(crate) const OUTPUT_BUFFER: f64 = 3.0;
/// A connection's delay: intercept, per tile, and per tile more on a wide
/// signal; see the module documentation.
pub(crate) const WIRE: (f64, f64, f64) = (0.35, 0.035, 0.025);

/// Whether `role` is a clock pin, which no path ends at.
fn is_clock(role: &str) -> bool {
    role.contains("clk") || role == "c"
}

/// The timing a placement of `netlist` is driven by, and the name of each
/// of its domains (the clock's `create_clock` name), in
/// [`PlaceTiming::periods`] order.
#[must_use]
pub fn place_timing(netlist: &Netlist, constraints: &Constraints) -> (PlaceTiming, Vec<String>) {
    let n = netlist.instances.len();
    let mut pins = vec![PinTiming::Ignored; netlist.pins.len()];
    let mut cell = vec![0.0; n];
    let mut launch = vec![0.0; n];
    let mut capture = vec![0.0; n];
    let mut domain = vec![PathDomain::Untimed; n];

    // The root of a clock pin's signal, through the global buffers.
    let root = |mut signal: usize| -> usize {
        for _ in 0..8 {
            let Some(driver) = netlist.signals[signal].driver else {
                break;
            };
            let inst = &netlist.instances[netlist.pins[driver].instance];
            if inst.kind != "gb" {
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
    let mut names: Vec<String> = Vec::new();
    let mut periods: Vec<f64> = Vec::new();
    let mut domain_of = |signal: usize| -> PathDomain {
        let name = &netlist.signals[root(signal)].name;
        let Some(clock) = constraints.clocks.iter().find(|c| c.net == *name) else {
            return PathDomain::Untimed;
        };
        if let Some(index) = names.iter().position(|n| *n == clock.name) {
            return PathDomain::Clock(index);
        }
        names.push(clock.name.clone());
        periods.push(clock.period_ns);
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
                    "gb" | "pll" | "mmcm"
                )
            })
    };
    let frozen: Vec<bool> = netlist
        .instances
        .iter()
        .map(|inst| {
            inst.kind != "gb"
                && inst.pins.iter().any(|p| {
                    !netlist.pins[*p].output
                        && !is_clock(&netlist.pins[*p].role)
                        && from_network(*p)
                })
        })
        .collect();
    for (index, inst) in netlist.instances.iter().enumerate() {
        let kind = inst.kind.as_str();
        let clock = inst
            .pins
            .iter()
            .find(|p| !netlist.pins[**p].output && is_clock(&netlist.pins[**p].role))
            .and_then(|p| netlist.pins[*p].signal);
        match kind {
            "lut" | "carry" | "gb" => {
                cell[index] = match kind {
                    "lut" => LUT,
                    "carry" => CARRY,
                    _ => 0.0,
                };
                for pin in &inst.pins {
                    if !is_clock(&netlist.pins[*pin].role) || kind == "gb" {
                        pins[*pin] = PinTiming::Through;
                    }
                }
            }
            "lutram" => {
                // The read is combinational, the write is clocked.
                cell[index] = LUT;
                capture[index] = FF_SETUP;
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = if is_clock(role) {
                        PinTiming::Ignored
                    } else if netlist.pins[*pin].output || role.starts_with("raddr") {
                        PinTiming::Through
                    } else {
                        PinTiming::End
                    };
                }
                if let Some(c) = clock {
                    domain[index] = domain_of(c);
                }
            }
            "io" => {
                launch[index] = INPUT_BUFFER;
                capture[index] = OUTPUT_BUFFER;
                domain[index] = PathDomain::Every;
                for pin in &inst.pins {
                    pins[*pin] = if netlist.pins[*pin].output {
                        PinTiming::Start
                    } else {
                        PinTiming::End
                    };
                }
            }
            _ => {
                // A flip-flop, a block RAM, and anything else clocked
                // starts paths at its outputs and ends them at its inputs.
                (launch[index], capture[index]) = match kind {
                    "ff" => (FF_LAUNCH, FF_SETUP),
                    "bram" => (BRAM_LAUNCH, BRAM_SETUP),
                    _ => (FF_LAUNCH, FF_SETUP),
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
                    domain[index] = domain_of(c);
                }
            }
        }
    }
    (
        PlaceTiming {
            pins,
            cell,
            launch,
            capture,
            domain,
            periods,
            wire: WIRE,
            rounds: 3,
            weight: 8.0,
            exponent: 3,
            frozen,
        },
        names,
    )
}
