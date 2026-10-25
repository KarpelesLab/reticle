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
//! Paths are timed within one clock's domain. A clock is the signal at a
//! sequential cell's clock pin traced back through the global buffers;
//! its period is the `create_clock` on it, and a clock no constraint
//! names is not timed. A package pin starts and ends paths in every
//! timed domain, which is pessimistic for a pin no clock drives and is
//! what a design whose bus is synchronous to one clock wants.
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

use crate::fpga::Constraints;
use crate::fpga::constraints::matches_glob;
use crate::fpga::place::{Netlist, PathDomain, PinTiming, PlaceTiming};
use crate::ir::{AttrValue, Design, ModuleId};
use crate::logic::Bit;

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
) -> (PlaceTiming, Vec<String>) {
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
                    domain[index] = domain_of(clock_of(c));
                }
            }
            "io" => {
                launch[index] = INPUT_BUFFER;
                capture[index] = OUTPUT_BUFFER;
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
                (launch[index], capture[index]) = (BRAM_LAUNCH, BRAM_SETUP);
                for pin in &inst.pins {
                    let role = netlist.pins[*pin].role.as_str();
                    pins[*pin] = match role {
                        "daddr" | "di" | "den" | "dwe" => PinTiming::End,
                        "do" | "drdy" => PinTiming::Start,
                        _ => PinTiming::Ignored,
                    };
                }
                let dclk = inst
                    .pins
                    .iter()
                    .find(|p| netlist.pins[**p].role == "dclk")
                    .and_then(|p| netlist.pins[*p].signal);
                if let Some(c) = dclk {
                    domain[index] = domain_of(clock_of(c));
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
                    domain[index] = domain_of(clock_of(c));
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
                launch[i] = INPUT_BUFFER + delay.delay_ns - CLOCK_INSERTION;
                domain[i] = PathDomain::Clock(index);
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
            reaches,
            cycles,
            untimed_from,
            untimed_to,
            frozen,
        },
        names,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpga::place::{Instance, NetPin, Signal};
    use crate::ir::{CellId, Const, Id};

    /// **A `LUT6_2`'s inputs reach the half that reads them.** One table,
    /// `O6` and `O5`, `I5` tied high so `O6` is the upper half: the lower
    /// half is `I1`, the upper `I0 & I2`. So `I0` and `I2` reach `O6` only,
    /// `I1` reaches `O5` only, and `I3` neither. Would catch the halves
    /// swapped, `I5` taken as a signal (the whole table, so `I1` would reach
    /// `O6` too), and a mask that ignores the truth table; would not catch a
    /// table whose model differs from the silicon's, which the mapped
    /// netlist's own checks are for.
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
    /// launches at its buffer in every domain, as a pad always did, and the
    /// clock's own pad in the clock's domain. Would catch the delay ignored,
    /// applied to the wrong pad, applied without the clock's arrival taken
    /// off, or left in every domain, or a clock's pad launching in every
    /// domain (as it did); would not
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
        let (timing, names) = place_timing(&design, top, &netlist, &constraints);
        assert_eq!(names, vec!["bus".to_owned()]);
        assert!((timing.launch[2] - (INPUT_BUFFER + 12.0 - CLOCK_INSERTION)).abs() < 1e-9);
        assert_eq!(timing.domain[2], PathDomain::Clock(0));
        assert!((timing.launch[3] - INPUT_BUFFER).abs() < 1e-9);
        assert_eq!(timing.domain[3], PathDomain::Every);
        // The clock's own pad launches what it carries in its clock's
        // domain alone.
        assert_eq!(timing.domain[0], PathDomain::Clock(0));
    }

    /// **A clock manager's register port is timed on its `DCLK`.** An MMCM
    /// whose `CLKIN2` and `CLKINSEL` are tied off, reference clock `ref`,
    /// register clock `bus`, written by a flip-flop on `bus` and read back
    /// by another. The MMCM is in `bus`'s domain, its address an endpoint
    /// and its data out a launch at the block RAM's figure, and its clock
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
                instance(2, "mmcm", "mmcm", vec![2, 3, 4, 5, 6, 7, 8]),
                instance(3, "addr", "ff", vec![9, 10]),
                instance(4, "back", "ff", vec![11, 12]),
            ],
            signals: vec![
                signal("ref", 0, vec![4]),
                signal("bus", 1, vec![5, 9, 11]),
                signal("addr", 10, vec![6]),
                signal("do", 7, vec![12]),
                signal("out0", 8, vec![]),
            ],
            pins,
            off_fabric: Vec::new(),
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
        let (timing, names) = place_timing(&design, top, &netlist, &constraints);
        let bus = names
            .iter()
            .position(|n| n == "bus")
            .expect("bus is a domain");
        assert_eq!(timing.domain[2], PathDomain::Clock(bus));
        assert_eq!(timing.pins[6], PinTiming::End, "the address");
        assert_eq!(timing.pins[7], PinTiming::Start, "the data out");
        assert_eq!(timing.pins[8], PinTiming::Ignored, "the clock out");
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
        };
        let period_of = |rcf: &str, instance: usize| -> (String, f64) {
            let mut diags = crate::diag::Diagnostics::new();
            let constraints = Constraints::parse(rcf, file, &mut diags);
            assert!(!diags.has_errors());
            let (timing, names) = place_timing(&design, top, &netlist, &constraints);
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
}
