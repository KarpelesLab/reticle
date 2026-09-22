//! Timing constraints, and the SDC file an ASIC flow is handed with the
//! netlist.
//!
//! SDC (Synopsys Design Constraints) is the lingua franca of the back
//! end: OpenROAD, OpenSTA and every commercial tool read the same
//! `create_clock` / `set_input_delay` / `set_false_path` vocabulary.
//! [`AsicConstraints`] is the small subset a synthesised block needs and
//! [`AsicConstraints::write_sdc`] renders it.
//!
//! # Why not `fpga::Constraints`
//!
//! [`crate::fpga::Constraints`] covers the same timing ground
//! (`create_clock`, `set_false_path`, `set_multicycle_path`) and adds
//! everything physical about an FPGA: pin assignment, IO standards,
//! placement regions, clock domains — all of it checked against a device
//! database. An ASIC block has none of that: its pins are placed by the
//! floorplan, not by a package, and there is no device file to check
//! against. It does need three things an FPGA constraint file has no use
//! for: `set_load`, `set_driving_cell` and per-port input and output
//! delays that name a library cell.
//!
//! Rather than make the `asic` feature depend on `fpga` for a type whose
//! FPGA half would be dead weight, the timing part is modelled again
//! here, deliberately small. The overlap is exactly the four constructs
//! listed above, and both types render them the same way; a design that
//! has an FPGA constraints value can convert through
//! [`AsicConstraints::from_timing_spec`] rather than by hand, since
//! [`crate::timing::sta::TimingSpec`] is already the analyser's
//! feature-independent view of both.
//!
//! # What is written
//!
//! | Constraint | SDC |
//! |------------|-----|
//! | [`Clock`] | `create_clock -name n -period p [-waveform {r f}] [get_ports {c}]` |
//! | [`PortDelay`] | `set_input_delay` / `set_output_delay` `-clock c [-max\|-min] d [get_ports {...}]` |
//! | [`PortLoad`] | `set_load c [get_ports {...}]` |
//! | [`DrivingCell`] | `set_driving_cell -lib_cell C -pin P [get_ports {...}]` |
//! | [`FalsePath`] | `set_false_path [-from ...] [-through ...] [-to ...]` |
//! | [`MulticyclePath`] | `set_multicycle_path n [-setup\|-hold] [-from ...] [-to ...]` |
//! | [`AsicConstraints::uncertainty`] | `set_clock_uncertainty u [get_clocks {*}]` |
//!
//! Times are in the units the design is constrained in — nanoseconds for
//! every Liberty library Reticle has been pointed at, since `time_unit :
//! "1ns"` is universal — and are written with three decimals.
//! Capacitances are written with [`crate::asic::fmt_num`], which keeps
//! small values exact.
//!
//! Output is deterministic: constraints are written in the order they
//! were added, one per line, and nothing iterates a hash map.
//!
//! Reading SDC back is not implemented: the flow's input is
//! [`AsicConstraints`] built by a caller (or by the CLI from an `.rcf`
//! file through [`crate::timing::sta::TimingSpec`]), and the SDC is an
//! output.

use std::fmt::Write as _;

use super::fmt_num;

/// A clock: a name, the port carrying it, a period and a waveform.
#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    /// The clock name, which path exceptions and reports refer to.
    pub name: String,
    /// The port or net carrying it.
    pub port: String,
    /// The period.
    pub period: f64,
    /// `-waveform {rise fall}`, or `None` for a 50% duty cycle starting
    /// at zero.
    pub waveform: Option<(f64, f64)>,
}

impl Clock {
    /// A clock with a 50% duty cycle starting at zero.
    pub fn new(name: impl Into<String>, port: impl Into<String>, period: f64) -> Clock {
        Clock {
            name: name.into(),
            port: port.into(),
            period,
            waveform: None,
        }
    }

    /// The same clock with an explicit waveform.
    pub fn with_waveform(mut self, rise: f64, fall: f64) -> Clock {
        self.waveform = Some((rise, fall));
        self
    }
}

/// Which analysis a delay or a multicycle constraint applies to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DelayKind {
    /// Both, which is what an SDC line with neither flag means.
    #[default]
    Both,
    /// `-max`: the setup (late) analysis only.
    Max,
    /// `-min`: the hold (early) analysis only.
    Min,
}

impl DelayKind {
    /// The SDC flag, empty for [`DelayKind::Both`].
    pub fn flag(self) -> &'static str {
        match self {
            DelayKind::Both => "",
            DelayKind::Max => " -max",
            DelayKind::Min => " -min",
        }
    }
}

/// An external delay on a set of ports, relative to a clock.
#[derive(Clone, Debug, PartialEq)]
pub struct PortDelay {
    /// The ports, as names or globs.
    pub ports: Vec<String>,
    /// The clock the delay is relative to.
    pub clock: String,
    /// The delay.
    pub delay: f64,
    /// Which analysis it applies to.
    pub kind: DelayKind,
}

impl PortDelay {
    /// A delay on one port, for both analyses.
    pub fn new(port: impl Into<String>, clock: impl Into<String>, delay: f64) -> PortDelay {
        PortDelay {
            ports: vec![port.into()],
            clock: clock.into(),
            delay,
            kind: DelayKind::Both,
        }
    }
}

/// The capacitance a set of output ports drives.
#[derive(Clone, Debug, PartialEq)]
pub struct PortLoad {
    /// The ports, as names or globs.
    pub ports: Vec<String>,
    /// The capacitance, in the library's capacitance unit.
    pub capacitance: f64,
}

/// The cell driving a set of input ports, which sets their slew.
#[derive(Clone, Debug, PartialEq)]
pub struct DrivingCell {
    /// The ports, as names or globs.
    pub ports: Vec<String>,
    /// The library cell.
    pub cell: String,
    /// Its output pin.
    pub pin: String,
    /// `-input_transition_rise`, when given.
    pub input_transition_rise: Option<f64>,
    /// `-input_transition_fall`, when given.
    pub input_transition_fall: Option<f64>,
}

/// A set of paths that is not checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FalsePath {
    /// `-from`, a glob over start points.
    pub from: Option<String>,
    /// `-through`, a glob over pins on the way.
    pub through: Option<String>,
    /// `-to`, a glob over end points.
    pub to: Option<String>,
}

/// A set of paths given more than one clock period.
#[derive(Clone, Debug, PartialEq)]
pub struct MulticyclePath {
    /// `-from`, a glob over start points.
    pub from: Option<String>,
    /// `-through`, a glob over pins on the way.
    pub through: Option<String>,
    /// `-to`, a glob over end points.
    pub to: Option<String>,
    /// How many periods.
    pub cycles: u32,
    /// True when the constraint moves the hold check (`-hold`) rather
    /// than the setup check (`-setup`).
    pub hold: bool,
}

/// The timing constraints of one block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AsicConstraints {
    /// The clocks.
    pub clocks: Vec<Clock>,
    /// `set_input_delay` constraints.
    pub input_delays: Vec<PortDelay>,
    /// `set_output_delay` constraints.
    pub output_delays: Vec<PortDelay>,
    /// `set_load` constraints.
    pub loads: Vec<PortLoad>,
    /// `set_driving_cell` constraints.
    pub driving_cells: Vec<DrivingCell>,
    /// `set_false_path` constraints.
    pub false_paths: Vec<FalsePath>,
    /// `set_multicycle_path` constraints.
    pub multicycle_paths: Vec<MulticyclePath>,
    /// Clock uncertainty applied to every clock, when given.
    pub uncertainty: Option<f64>,
}

impl AsicConstraints {
    /// Constraints with nothing in them.
    pub fn new() -> AsicConstraints {
        AsicConstraints::default()
    }

    /// Adds a clock.
    pub fn with_clock(mut self, clock: Clock) -> AsicConstraints {
        self.clocks.push(clock);
        self
    }

    /// Adds an input delay.
    pub fn with_input_delay(mut self, delay: PortDelay) -> AsicConstraints {
        self.input_delays.push(delay);
        self
    }

    /// Adds an output delay.
    pub fn with_output_delay(mut self, delay: PortDelay) -> AsicConstraints {
        self.output_delays.push(delay);
        self
    }

    /// Adds a load on a port.
    pub fn with_load(mut self, port: impl Into<String>, capacitance: f64) -> AsicConstraints {
        self.loads.push(PortLoad {
            ports: vec![port.into()],
            capacitance,
        });
        self
    }

    /// Adds a driving cell on a port.
    pub fn with_driving_cell(
        mut self,
        port: impl Into<String>,
        cell: impl Into<String>,
        pin: impl Into<String>,
    ) -> AsicConstraints {
        self.driving_cells.push(DrivingCell {
            ports: vec![port.into()],
            cell: cell.into(),
            pin: pin.into(),
            input_transition_rise: None,
            input_transition_fall: None,
        });
        self
    }

    /// Adds a false path.
    pub fn with_false_path(mut self, from: Option<&str>, to: Option<&str>) -> AsicConstraints {
        self.false_paths.push(FalsePath {
            from: from.map(str::to_owned),
            through: None,
            to: to.map(str::to_owned),
        });
        self
    }

    /// Adds a setup multicycle path of `cycles` periods.
    pub fn with_multicycle_path(
        mut self,
        from: Option<&str>,
        to: Option<&str>,
        cycles: u32,
    ) -> AsicConstraints {
        self.multicycle_paths.push(MulticyclePath {
            from: from.map(str::to_owned),
            through: None,
            to: to.map(str::to_owned),
            cycles,
            hold: false,
        });
        self
    }

    /// The clock named `name`.
    pub fn clock(&self, name: &str) -> Option<&Clock> {
        self.clocks.iter().find(|c| c.name == name)
    }

    /// True when nothing is constrained.
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
            && self.input_delays.is_empty()
            && self.output_delays.is_empty()
            && self.loads.is_empty()
            && self.driving_cells.is_empty()
            && self.false_paths.is_empty()
            && self.multicycle_paths.is_empty()
            && self.uncertainty.is_none()
    }

    /// Renders the constraints as an SDC file.
    ///
    /// `design` names the block, for the header comment only.
    pub fn write_sdc(&self, design: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# SDC constraints for `{design}`, written by Reticle.");
        let _ = writeln!(
            out,
            "# Times are in the library's time unit (nanoseconds for every\n\
             # Liberty library in use), capacitances in its capacitive load unit."
        );
        for clock in &self.clocks {
            let _ = write!(
                out,
                "create_clock -name {} -period {}",
                clock.name,
                time(clock.period)
            );
            if let Some((rise, fall)) = clock.waveform {
                let _ = write!(out, " -waveform {{{} {}}}", time(rise), time(fall));
            }
            let _ = writeln!(out, " [get_ports {{{}}}]", clock.port);
        }
        if let Some(u) = self.uncertainty {
            let _ = writeln!(out, "set_clock_uncertainty {} [get_clocks {{*}}]", time(u));
        }
        for (command, delays) in [
            ("set_input_delay", &self.input_delays),
            ("set_output_delay", &self.output_delays),
        ] {
            for delay in delays {
                let _ = writeln!(
                    out,
                    "{command} -clock {}{} {} [get_ports {{{}}}]",
                    delay.clock,
                    delay.kind.flag(),
                    time(delay.delay),
                    delay.ports.join(" ")
                );
            }
        }
        for load in &self.loads {
            let _ = writeln!(
                out,
                "set_load {} [get_ports {{{}}}]",
                fmt_num(load.capacitance),
                load.ports.join(" ")
            );
        }
        for driver in &self.driving_cells {
            let _ = write!(
                out,
                "set_driving_cell -lib_cell {} -pin {}",
                driver.cell, driver.pin
            );
            if let Some(v) = driver.input_transition_rise {
                let _ = write!(out, " -input_transition_rise {}", time(v));
            }
            if let Some(v) = driver.input_transition_fall {
                let _ = write!(out, " -input_transition_fall {}", time(v));
            }
            let _ = writeln!(out, " [get_ports {{{}}}]", driver.ports.join(" "));
        }
        for path in &self.false_paths {
            let _ = writeln!(
                out,
                "set_false_path{}",
                endpoints(&path.from, &path.through, &path.to)
            );
        }
        for path in &self.multicycle_paths {
            let _ = writeln!(
                out,
                "set_multicycle_path {} {}{}",
                path.cycles,
                if path.hold { "-hold" } else { "-setup" },
                endpoints(&path.from, &path.through, &path.to)
            );
        }
        out
    }

    /// The clocks and path exceptions as the static timing analyser's
    /// own view of them.
    #[cfg(feature = "timing")]
    pub fn to_timing_spec(&self) -> crate::timing::sta::TimingSpec {
        use crate::timing::sta::{ClockSpec, ExceptionKind, PathException, TimingSpec};
        let mut spec = TimingSpec::new();
        for clock in &self.clocks {
            let mut c = ClockSpec::new(clock.name.clone(), clock.port.clone(), clock.period);
            if let Some((rise, fall)) = clock.waveform {
                c = c.with_waveform(rise, fall);
            }
            spec.clocks.push(c);
        }
        for path in &self.false_paths {
            spec.exceptions.push(PathException {
                from: path.from.clone(),
                to: path.to.clone(),
                kind: ExceptionKind::False,
                span: None,
            });
        }
        for path in &self.multicycle_paths {
            spec.exceptions.push(PathException {
                from: path.from.clone(),
                to: path.to.clone(),
                kind: ExceptionKind::Multicycle {
                    cycles: path.cycles,
                    hold: path.hold,
                },
                span: None,
            });
        }
        spec
    }

    /// Fills the I/O delays and the uncertainty of a [`TimingOptions`]
    /// from these constraints, leaving everything else as given.
    ///
    /// A `-min` delay lands in the early analysis and a `-max` one in the
    /// late analysis; the analyser keeps one number per port, so a
    /// constraint that gives different values for the two is taken at its
    /// larger, which is the pessimistic reading.
    ///
    /// [`TimingOptions`]: crate::timing::sta::TimingOptions
    #[cfg(feature = "timing")]
    pub fn apply_to_options(&self, options: &mut crate::timing::sta::TimingOptions) {
        for (delays, slot) in [(&self.input_delays, 0usize), (&self.output_delays, 1usize)] {
            for delay in delays {
                for port in &delay.ports {
                    let target = if slot == 0 {
                        &mut options.input_delays
                    } else {
                        &mut options.output_delays
                    };
                    match target.iter_mut().find(|(p, _)| p == port) {
                        Some((_, v)) => *v = v.max(delay.delay),
                        None => target.push((port.clone(), delay.delay)),
                    }
                }
            }
        }
        if let Some(u) = self.uncertainty {
            options.uncertainty = u;
        }
        if options.io_clock.is_none() {
            options.io_clock = self
                .input_delays
                .first()
                .or_else(|| self.output_delays.first())
                .map(|d| d.clock.clone());
        }
    }

    /// Builds constraints from the analyser's view of an SDC file, which
    /// is how an FPGA-style constraints value (or the CLI's `.rcf`
    /// reader) reaches this module.
    ///
    /// Only what [`TimingSpec`] holds comes across: clocks, false paths
    /// and multicycle paths. Loads and driving cells have no equivalent
    /// there and stay empty.
    ///
    /// [`TimingSpec`]: crate::timing::sta::TimingSpec
    #[cfg(feature = "timing")]
    pub fn from_timing_spec(spec: &crate::timing::sta::TimingSpec) -> AsicConstraints {
        use crate::timing::sta::ExceptionKind;
        let mut out = AsicConstraints::new();
        for clock in &spec.clocks {
            let mut c = Clock::new(clock.name.clone(), clock.net.clone(), clock.period);
            if clock.rise != 0.0 || (clock.fall - clock.period / 2.0).abs() > f64::EPSILON {
                c = c.with_waveform(clock.rise, clock.fall);
            }
            out.clocks.push(c);
        }
        for exception in &spec.exceptions {
            match exception.kind {
                ExceptionKind::False => out.false_paths.push(FalsePath {
                    from: exception.from.clone(),
                    through: None,
                    to: exception.to.clone(),
                }),
                ExceptionKind::Multicycle { cycles, hold } => {
                    out.multicycle_paths.push(MulticyclePath {
                        from: exception.from.clone(),
                        through: None,
                        to: exception.to.clone(),
                        cycles,
                        hold,
                    });
                }
            }
        }
        out
    }
}

/// The `-from` / `-through` / `-to` part of a path exception.
fn endpoints(from: &Option<String>, through: &Option<String>, to: &Option<String>) -> String {
    let mut out = String::new();
    for (flag, value) in [("-from", from), ("-through", through), ("-to", to)] {
        if let Some(v) = value {
            let _ = write!(out, " {flag} [get_pins {{{v}}}]");
        }
    }
    out
}

/// A time as SDC writes it: three decimals, which is a picosecond when
/// the unit is a nanosecond.
fn time(v: f64) -> String {
    format!("{v:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AsicConstraints {
        AsicConstraints::new()
            .with_clock(Clock::new("sys", "clk", 10.0))
            .with_clock(Clock::new("slow", "clk2", 40.0).with_waveform(0.0, 10.0))
            .with_input_delay(PortDelay::new("d", "sys", 1.5))
            .with_output_delay(PortDelay::new("q", "sys", 2.25))
            .with_load("q", 0.025)
            .with_driving_cell("d", "INV_X1", "Y")
            .with_false_path(Some("rst_n"), None)
            .with_multicycle_path(Some("a"), Some("b"), 2)
    }

    #[test]
    fn the_sdc_has_one_line_per_constraint() {
        let sdc = sample().write_sdc("top");
        let expected = "\
create_clock -name sys -period 10.000 [get_ports {clk}]
create_clock -name slow -period 40.000 -waveform {0.000 10.000} [get_ports {clk2}]
set_input_delay -clock sys 1.500 [get_ports {d}]
set_output_delay -clock sys 2.250 [get_ports {q}]
set_load 0.025 [get_ports {q}]
set_driving_cell -lib_cell INV_X1 -pin Y [get_ports {d}]
set_false_path -from [get_pins {rst_n}]
set_multicycle_path 2 -setup -from [get_pins {a}] -to [get_pins {b}]
";
        let body: String = sdc
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(body, expected, "{sdc}");
        assert!(sdc.starts_with("# SDC constraints for `top`"));
    }

    #[test]
    fn the_optional_parts_are_written_when_set() {
        let mut c = AsicConstraints::new().with_clock(Clock::new("sys", "clk", 8.0));
        c.uncertainty = Some(0.1);
        c.input_delays.push(PortDelay {
            ports: vec!["a".into(), "b".into()],
            clock: "sys".into(),
            delay: 0.5,
            kind: DelayKind::Min,
        });
        c.output_delays.push(PortDelay {
            ports: vec!["y*".into()],
            clock: "sys".into(),
            delay: 3.0,
            kind: DelayKind::Max,
        });
        c.driving_cells.push(DrivingCell {
            ports: vec!["a".into()],
            cell: "BUF_X2".into(),
            pin: "Y".into(),
            input_transition_rise: Some(0.05),
            input_transition_fall: Some(0.04),
        });
        c.multicycle_paths.push(MulticyclePath {
            from: None,
            through: Some("u1/A".into()),
            to: Some("ff*".into()),
            cycles: 3,
            hold: true,
        });
        let sdc = c.write_sdc("m");
        assert!(
            sdc.contains("set_clock_uncertainty 0.100 [get_clocks {*}]"),
            "{sdc}"
        );
        assert!(
            sdc.contains("set_input_delay -clock sys -min 0.500 [get_ports {a b}]"),
            "{sdc}"
        );
        assert!(
            sdc.contains("set_output_delay -clock sys -max 3.000 [get_ports {y*}]"),
            "{sdc}"
        );
        assert!(
            sdc.contains(
                "set_driving_cell -lib_cell BUF_X2 -pin Y -input_transition_rise 0.050 \
                 -input_transition_fall 0.040 [get_ports {a}]"
            ),
            "{sdc}"
        );
        assert!(
            sdc.contains(
                "set_multicycle_path 3 -hold -through [get_pins {u1/A}] -to [get_pins {ff*}]"
            ),
            "{sdc}"
        );
        assert_eq!(DelayKind::default(), DelayKind::Both);
        assert_eq!(DelayKind::Both.flag(), "");
    }

    #[test]
    fn empty_constraints_write_only_a_header() {
        let c = AsicConstraints::new();
        assert!(c.is_empty());
        assert!(c.clock("sys").is_none());
        let sdc = c.write_sdc("m");
        assert!(sdc.lines().all(|l| l.starts_with('#')), "{sdc}");
        assert!(!sample().is_empty());
        assert_eq!(sample().clock("sys").map(|c| c.period), Some(10.0));
    }

    #[cfg(feature = "timing")]
    #[test]
    fn the_analyser_sees_the_same_clocks_and_exceptions() {
        use crate::timing::sta::{ExceptionKind, TimingOptions};
        let c = sample();
        let spec = c.to_timing_spec();
        assert_eq!(spec.clocks.len(), 2);
        assert_eq!(spec.clocks[0].name, "sys");
        assert_eq!(spec.clocks[0].net, "clk");
        assert_eq!(spec.clocks[1].fall, 10.0);
        assert_eq!(spec.exceptions.len(), 2);
        assert_eq!(spec.exceptions[0].kind, ExceptionKind::False);
        assert_eq!(
            spec.exceptions[1].kind,
            ExceptionKind::Multicycle {
                cycles: 2,
                hold: false
            }
        );
        let mut options = TimingOptions::default();
        c.apply_to_options(&mut options);
        assert_eq!(options.input_delays, [("d".to_string(), 1.5)]);
        assert_eq!(options.output_delays, [("q".to_string(), 2.25)]);
        assert_eq!(options.io_clock.as_deref(), Some("sys"));

        // And the round trip through the analyser's view keeps them.
        let back = AsicConstraints::from_timing_spec(&spec);
        assert_eq!(back.clocks, c.clocks);
        assert_eq!(back.false_paths, c.false_paths);
        assert_eq!(back.multicycle_paths, c.multicycle_paths);
        assert!(back.loads.is_empty());
    }
}
