//! Static timing analysis: arrival times, required times, slack and
//! path reports.
//!
//! [`analyze_with`] takes a module in cell form, a [`TimingSpec`] (the
//! clocks and the path exceptions), a [`DelayModel`] and a set of
//! [`TimingOptions`], and returns a [`TimingReport`]. With the `fpga`
//! feature, [`analyze`] is the same thing with the spec read straight
//! out of a [`crate::fpga::Constraints`] value.
//!
//! # The model
//!
//! 1. The module becomes a [`TimingGraph`]: pins are nodes, cell arcs and
//!    net arcs are edges.
//! 2. Every arc is annotated by the delay model, in topological order, so
//!    each arc is asked with the transition time the previous stage
//!    produced.
//! 3. The **clock network** is propagated from each `create_clock`
//!    source through combinational arcs until it reaches a sequential
//!    cell's clock pin. In [`ClockMode::Ideal`] the resulting latency is
//!    discarded and every clock pin sees its edge at the same instant; in
//!    [`ClockMode::Propagated`] it is kept, and the difference between
//!    the launch and the capture flop's latency is the clock skew.
//! 4. **Arrival times** are propagated forwards from the start points:
//!    input ports (at their input delay) and sequential clock pins (at
//!    their clock latency, from where the clock-to-output arc carries
//!    them to `q`).
//! 5. **Required times** are propagated backwards from the end points:
//!    output ports and sequential data pins. Both are kept *relative to
//!    the launching clock edge*, so a required time is a number of
//!    nanoseconds of budget rather than an absolute instant, and the
//!    slack at a pin is `required - arrival`.
//! 6. **Paths** are enumerated backwards from each end point with a
//!    best-first search whose priority is `arrival + suffix`, an exact
//!    bound on a directed acyclic graph, so the N worst paths come out
//!    in order. Path exceptions are applied as paths are completed,
//!    which is what makes a false path skip to the next worst path
//!    rather than silently disappear.
//!
//! Rise and fall are kept apart from end to end: every arrival, required
//! time and arc delay is a [`Transition`], and an arc's
//! [`super::delay::Sense`] says which input edge produces which output
//! edge, so a path through an
//! inverter alternates edges the way the silicon does.
//!
//! # Setup, hold and multiple clocks
//!
//! For each end point the launch and capture edges are the closest pair
//! of active edges of the two clocks with the capture edge after the
//! launch edge, searched over enough periods to cover their relationship.
//! For the same clock that is simply `launch = 0`, `capture = period`.
//! The setup check uses that pair; the hold check uses the same launch
//! edge against the capture edge one capture period earlier.
//! `set_multicycle_path N` moves the setup capture edge `N - 1` periods
//! later and leaves the hold check alone; `set_multicycle_path N -hold`
//! moves the hold capture edge `N` periods earlier.
//!
//! # Not modelled
//!
//! Wire RC and coupling (see [`super::delay`]), on-chip variation beyond
//! the two flat derating factors, generated clocks (a clock divided by a
//! flip-flop is not recognised as a clock), clock gating checks,
//! recovery and removal on asynchronous resets, latch time borrowing,
//! minimum pulse width and maximum transition / capacitance design-rule
//! checks. The per-pin required times in [`TimingReport::pins`] ignore
//! path exceptions; the per-path slacks do not.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt::Write as _;

use crate::ir::Module;
use crate::source::{SourceMap, Span};

use super::delay::{ConstraintQuery, DelayModel, Edge, Transition};
use super::graph::{ArcKind, PinId, PinRole, PointKind, TimingGraph, sequential_checks};

/// How the clock network is treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClockMode {
    /// Every clock pin sees its edge at the same instant: no insertion
    /// delay, no skew. The usual mode before place and route.
    #[default]
    Ideal,
    /// The clock network's own delay is propagated, so a launch flop
    /// deep in the tree and a capture flop near the root differ by their
    /// skew.
    Propagated,
}

/// Which of the two checks a number belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Check {
    /// Late data must arrive before the capture edge: `max` analysis.
    Setup,
    /// Early data must not arrive before the previous capture edge:
    /// `min` analysis.
    Hold,
}

impl Check {
    /// The word used in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Setup => "setup",
            Check::Hold => "hold",
        }
    }
}

/// One clock: a name, the net carrying it, a period and a waveform.
#[derive(Clone, Debug, PartialEq)]
pub struct ClockSpec {
    /// The clock's name, used by path exceptions and by the report.
    pub name: String,
    /// The port or net carrying it.
    pub net: String,
    /// The period.
    pub period: f64,
    /// Where in the period the rising edge is.
    pub rise: f64,
    /// Where in the period the falling edge is.
    pub fall: f64,
    /// Where the constraint was written, when it came from a file.
    pub span: Option<Span>,
    /// True when no constraint named this clock and it was invented for
    /// a clock pin that would otherwise have no check.
    pub inferred: bool,
}

impl ClockSpec {
    /// A clock with a 50% duty cycle starting at zero.
    pub fn new(name: impl Into<String>, net: impl Into<String>, period: f64) -> ClockSpec {
        ClockSpec {
            name: name.into(),
            net: net.into(),
            period,
            rise: 0.0,
            fall: period / 2.0,
            span: None,
            inferred: false,
        }
    }

    /// The same clock with an explicit waveform (`rise`, `fall`).
    pub fn with_waveform(mut self, rise: f64, fall: f64) -> ClockSpec {
        self.rise = rise;
        self.fall = fall;
        self
    }

    /// The time of the `n`th active edge, counting from zero.
    fn edge_at(&self, rising: bool, n: u32) -> f64 {
        let base = if rising { self.rise } else { self.fall };
        base + f64::from(n) * self.period
    }
}

/// What an exception does to the paths it matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExceptionKind {
    /// The path is not checked at all.
    False,
    /// The path is given more than one clock period.
    Multicycle {
        /// How many periods.
        cycles: u32,
        /// True when the constraint moves the hold check rather than the
        /// setup check.
        hold: bool,
    },
}

/// A set of paths, named by where they start and end, and what to do
/// with them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathException {
    /// A glob over start points, or `None` for "anywhere".
    pub from: Option<String>,
    /// A glob over end points, or `None` for "anywhere".
    pub to: Option<String>,
    /// What the exception does.
    pub kind: ExceptionKind,
    /// Where the constraint was written.
    pub span: Option<Span>,
}

/// The clocks and path exceptions an analysis works to.
///
/// This is the analysis' own, feature-independent view of what an SDC
/// file says. With the `fpga` feature, [`TimingSpec::from_constraints`]
/// builds one from a [`crate::fpga::Constraints`] value, which is what
/// [`analyze`] does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimingSpec {
    /// The clocks, in the order they were declared.
    pub clocks: Vec<ClockSpec>,
    /// The exceptions, in the order they were declared.
    pub exceptions: Vec<PathException>,
}

impl TimingSpec {
    /// An empty spec: no clocks and no exceptions.
    pub fn new() -> TimingSpec {
        TimingSpec::default()
    }

    /// Adds a clock.
    pub fn with_clock(mut self, clock: ClockSpec) -> TimingSpec {
        self.clocks.push(clock);
        self
    }

    /// Adds an exception.
    pub fn with_exception(mut self, exception: PathException) -> TimingSpec {
        self.exceptions.push(exception);
        self
    }

    /// Marks every path from `from` to `to` (globs, `None` for
    /// "anywhere") as not to be checked.
    pub fn with_false_path(self, from: Option<&str>, to: Option<&str>) -> TimingSpec {
        self.with_exception(PathException {
            from: from.map(str::to_owned),
            to: to.map(str::to_owned),
            kind: ExceptionKind::False,
            span: None,
        })
    }

    /// Gives every path from `from` to `to` `cycles` clock periods.
    pub fn with_multicycle_path(
        self,
        from: Option<&str>,
        to: Option<&str>,
        cycles: u32,
        hold: bool,
    ) -> TimingSpec {
        self.with_exception(PathException {
            from: from.map(str::to_owned),
            to: to.map(str::to_owned),
            kind: ExceptionKind::Multicycle { cycles, hold },
            span: None,
        })
    }

    /// The clock with the given name.
    pub fn clock(&self, name: &str) -> Option<&ClockSpec> {
        self.clocks.iter().find(|c| c.name == name)
    }

    /// Builds a spec from a constraints value: `create_clock` becomes a
    /// [`ClockSpec`] with a 50% duty cycle (the `.rcf` format records a
    /// period, not a waveform), `set_false_path` and
    /// `set_multicycle_path` become [`PathException`]s.
    #[cfg(feature = "fpga")]
    pub fn from_constraints(constraints: &crate::fpga::Constraints) -> TimingSpec {
        let mut spec = TimingSpec::new();
        for clock in &constraints.clocks {
            spec.clocks.push(ClockSpec {
                name: clock.name.clone(),
                net: clock.net.clone(),
                period: clock.period_ns,
                rise: 0.0,
                fall: clock.period_ns / 2.0,
                span: Some(clock.span),
                inferred: false,
            });
        }
        for path in &constraints.false_paths {
            spec.exceptions.push(PathException {
                from: path.from.clone(),
                to: path.to.clone(),
                kind: ExceptionKind::False,
                span: Some(path.span),
            });
        }
        for mcp in &constraints.multicycle_paths {
            spec.exceptions.push(PathException {
                from: mcp.path.from.clone(),
                to: mcp.path.to.clone(),
                kind: ExceptionKind::Multicycle {
                    cycles: mcp.cycles,
                    hold: mcp.hold,
                },
                span: Some(mcp.path.span),
            });
        }
        spec
    }
}

/// What to analyse and how much of it to report.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingOptions {
    /// Ideal or propagated clocks.
    pub clock_mode: ClockMode,
    /// Run the setup (max) analysis.
    pub check_setup: bool,
    /// Run the hold (min) analysis.
    pub check_hold: bool,
    /// How many paths to keep per end point.
    pub paths_per_endpoint: usize,
    /// How many paths to keep in the whole report.
    pub max_paths: usize,
    /// Clock uncertainty (jitter plus margin), subtracted from every
    /// setup budget and added to every hold requirement.
    pub uncertainty: f64,
    /// Multiplies every late (setup) delay.
    pub late_derate: f64,
    /// Multiplies every early (hold) delay.
    pub early_derate: f64,
    /// Period used for a clock pin no `create_clock` reaches.
    pub default_period: f64,
    /// External delay before every input port.
    pub input_delay: f64,
    /// External delay after every output port.
    pub output_delay: f64,
    /// Per-port input delays, by glob; the first match wins.
    pub input_delays: Vec<(String, f64)>,
    /// Per-port output delays, by glob; the first match wins.
    pub output_delays: Vec<(String, f64)>,
    /// The clock that input and output delays are relative to; the
    /// alphabetically first clock when `None`.
    pub io_clock: Option<String>,
    /// Transition time assumed at every start point.
    pub input_slew: Transition,
    /// Capacitance assumed on every output port.
    pub output_load: f64,
    /// Capacitance added to every net on top of the pins it drives.
    pub wire_load: f64,
    /// How many states the path search may expand per end point before
    /// giving up; keeps a pathological fan-in cone bounded.
    pub search_budget: usize,
}

impl Default for TimingOptions {
    /// Ideal clocks, both checks, one path per end point and ten in the
    /// report, no uncertainty, no derating, a 10 ns default period and
    /// no I/O delays.
    fn default() -> Self {
        TimingOptions {
            clock_mode: ClockMode::Ideal,
            check_setup: true,
            check_hold: true,
            paths_per_endpoint: 1,
            max_paths: 10,
            uncertainty: 0.0,
            late_derate: 1.0,
            early_derate: 1.0,
            default_period: 10.0,
            input_delay: 0.0,
            output_delay: 0.0,
            input_delays: Vec::new(),
            output_delays: Vec::new(),
            io_clock: None,
            input_slew: Transition::ZERO,
            output_load: 0.0,
            wire_load: 0.0,
            search_budget: 20_000,
        }
    }
}

/// What one point of a reported path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointRole {
    /// A pin of the clock network, before the launching edge.
    Clock,
    /// The launching pin: a clock pin or an input port.
    Start,
    /// The output pin of a cell arc.
    Cell,
    /// The input pin a net arc lands on.
    Net,
    /// The end point.
    End,
}

impl PointRole {
    /// A word for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            PointRole::Clock => "clock",
            PointRole::Start => "launch",
            PointRole::Cell => "cell",
            PointRole::Net => "net",
            PointRole::End => "endpoint",
        }
    }
}

/// One line of a reported path.
#[derive(Clone, Debug, PartialEq)]
pub struct PathPoint {
    /// The pin's report name.
    pub pin: String,
    /// What the point is.
    pub role: PointRole,
    /// Which way the signal is going here.
    pub edge: Edge,
    /// Delay of the arc that reached this pin.
    pub incr: f64,
    /// Delay from the start of the path to here.
    pub cumulative: f64,
    /// The cell type, or the net name for a net arc.
    pub via: String,
    /// Where the object came from in the source.
    pub span: Option<Span>,
}

/// One reported path, from a start point to an end point.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingPath {
    /// Setup or hold.
    pub check: Check,
    /// The object the path starts at (a cell or a port).
    pub start: String,
    /// The object the path ends at.
    pub end: String,
    /// The pin the path starts at.
    pub start_pin: String,
    /// The pin the path ends at.
    pub end_pin: String,
    /// The cell type of the object the path starts at.
    pub start_type: String,
    /// The cell type of the object the path ends at.
    pub end_type: String,
    /// The clock that launches it.
    pub launch_clock: String,
    /// The clock that captures it.
    pub capture_clock: String,
    /// The launch edge, in the analysis' time base.
    pub launch_edge: f64,
    /// The capture edge used for this check.
    pub capture_edge: f64,
    /// Clock network latency at the capturing clock pin.
    pub capture_latency: f64,
    /// Capture clock latency minus launch clock latency.
    pub clock_skew: f64,
    /// The input or output delay outside the design, when the path ends
    /// at a port.
    pub external_delay: f64,
    /// The setup or hold time of the capturing cell.
    pub constraint: f64,
    /// The uncertainty applied.
    pub uncertainty: f64,
    /// How many periods the path was given; 1 unless a multicycle
    /// exception matched.
    pub multicycle: u32,
    /// The points, launch first.
    pub points: Vec<PathPoint>,
    /// Data arrival time at the end point, from the launch edge.
    pub arrival: f64,
    /// Data required time at the end point, from the launch edge.
    pub required: f64,
    /// `required - arrival` for setup, `arrival - required` for hold.
    pub slack: f64,
}

impl TimingPath {
    /// True when the path fails its check.
    pub fn is_violated(&self) -> bool {
        self.slack < 0.0
    }
}

/// The worst slack at one end point.
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointSlack {
    /// The end point's pin.
    pub pin: String,
    /// Setup or hold.
    pub check: Check,
    /// The worst slack over every path that reaches it.
    pub slack: f64,
    /// The clock that launches the worst path.
    pub launch_clock: String,
    /// The clock that captures it.
    pub capture_clock: String,
}

/// The worst slack of one clock group, which is one launch / capture
/// clock pair and one check.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupSummary {
    /// The launching clock.
    pub launch_clock: String,
    /// The capturing clock.
    pub capture_clock: String,
    /// Setup or hold.
    pub check: Check,
    /// The worst slack in the group.
    pub worst_slack: f64,
    /// The end point that has it.
    pub worst_endpoint: String,
    /// How many end points are in the group.
    pub endpoints: usize,
    /// How many of them have negative slack.
    pub violating: usize,
}

/// The arrival and required times at one pin, for callers that want the
/// propagation itself rather than the paths.
#[derive(Clone, Debug, PartialEq)]
pub struct PinTiming {
    /// The pin's report name.
    pub pin: String,
    /// Latest arrival, per edge; `-inf` where nothing reaches the pin.
    pub arrival_max: Transition,
    /// Earliest arrival, per edge; `+inf` where nothing reaches it.
    pub arrival_min: Transition,
    /// Latest time data may arrive, per edge; `+inf` when unconstrained.
    pub required_max: Transition,
    /// Earliest time data may arrive, per edge; `-inf` when
    /// unconstrained.
    pub required_min: Transition,
    /// The clock that reaches this pin through the clock network.
    pub clock: Option<String>,
    /// Clock network latency at this pin.
    pub clock_latency: Transition,
}

impl PinTiming {
    /// Setup slack at this pin: the worst of the two edges.
    pub fn setup_slack(&self) -> f64 {
        (self.required_max.rise - self.arrival_max.rise)
            .min(self.required_max.fall - self.arrival_max.fall)
    }

    /// Hold slack at this pin: the worst of the two edges.
    pub fn hold_slack(&self) -> f64 {
        (self.arrival_min.rise - self.required_min.rise)
            .min(self.arrival_min.fall - self.required_min.fall)
    }
}

/// A combinational loop, as the report tells it.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopReport {
    /// The pins on the cycle, in order.
    pub pins: Vec<String>,
    /// Where the first cell on the cycle came from.
    pub span: Span,
}

/// Everything one analysis found.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingReport {
    /// The module analysed.
    pub module: String,
    /// The delay model's name.
    pub model: String,
    /// Ideal or propagated clocks.
    pub clock_mode: ClockMode,
    /// The clocks, sorted by name.
    pub clocks: Vec<ClockSpec>,
    /// Per clock group and check, sorted by group then check.
    pub groups: Vec<GroupSummary>,
    /// Every end point with a check, worst slack first.
    pub endpoints: Vec<EndpointSlack>,
    /// The worst paths, worst slack first.
    pub paths: Vec<TimingPath>,
    /// Combinational loops, which are reported instead of walked.
    pub loops: Vec<LoopReport>,
    /// Arrival and required times at every pin, in pin order.
    pub pins: Vec<PinTiming>,
    /// Anything worth saying about the analysis, in a stable order.
    pub notes: Vec<String>,
}

impl TimingReport {
    /// The worst setup slack in the design, if anything was checked.
    pub fn worst_setup(&self) -> Option<f64> {
        self.endpoints
            .iter()
            .filter(|e| e.check == Check::Setup)
            .map(|e| e.slack)
            .reduce(f64::min)
    }

    /// The worst hold slack in the design, if anything was checked.
    pub fn worst_hold(&self) -> Option<f64> {
        self.endpoints
            .iter()
            .filter(|e| e.check == Check::Hold)
            .map(|e| e.slack)
            .reduce(f64::min)
    }

    /// The worst slack at one end point for one check.
    pub fn slack_at(&self, pin: &str, check: Check) -> Option<f64> {
        self.endpoints
            .iter()
            .find(|e| e.pin == pin && e.check == check)
            .map(|e| e.slack)
    }

    /// The timing of one pin by name.
    pub fn pin(&self, name: &str) -> Option<&PinTiming> {
        self.pins.iter().find(|p| p.pin == name)
    }

    /// True when any check failed.
    pub fn has_violations(&self) -> bool {
        self.endpoints.iter().any(|e| e.slack < 0.0)
    }

    /// The summary table alone: the clocks and the worst slack per clock
    /// group.
    pub fn render_summary(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "timing summary for module `{}` ({} delays, {} clocks)",
            self.module,
            self.model,
            match self.clock_mode {
                ClockMode::Ideal => "ideal",
                ClockMode::Propagated => "propagated",
            }
        );
        if self.clocks.is_empty() {
            out.push_str("  no clocks\n");
        } else {
            out.push_str("  clock            period    waveform      source\n");
            for clock in &self.clocks {
                let _ = writeln!(
                    out,
                    "  {:<15} {:>7} {:>6} {:>6}      {}{}",
                    clock.name,
                    num(clock.period),
                    num(clock.rise),
                    num(clock.fall),
                    clock.net,
                    if clock.inferred { " (inferred)" } else { "" }
                );
            }
        }
        if self.groups.is_empty() {
            out.push_str("  no checked end points\n");
            return out;
        }
        out.push_str(
            "  group                      check   endpoints  violating   worst slack  endpoint\n",
        );
        for group in &self.groups {
            let _ = writeln!(
                out,
                "  {:<26} {:<7} {:>9} {:>10} {:>13}  {}",
                format!("{} -> {}", group.launch_clock, group.capture_clock),
                group.check.as_str(),
                group.endpoints,
                group.violating,
                num(group.worst_slack),
                group.worst_endpoint
            );
        }
        out
    }

    /// The whole report: the summary, then every reported path in
    /// detail, then the loops and notes.
    pub fn render(&self) -> String {
        self.render_inner(None)
    }

    /// The whole report with `file:line:col` locations resolved through
    /// `sources`.
    pub fn render_sources(&self, sources: &SourceMap) -> String {
        self.render_inner(Some(sources))
    }

    fn render_inner(&self, sources: Option<&SourceMap>) -> String {
        let mut out = self.render_summary();
        for (i, path) in self.paths.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "path {} of {}: {} slack {} ({})",
                i + 1,
                self.paths.len(),
                path.check.as_str(),
                num(path.slack),
                if path.is_violated() {
                    "VIOLATED"
                } else {
                    "met"
                }
            );
            let _ = writeln!(
                out,
                "  startpoint: {} ({})",
                path.start_pin,
                describe(&path.start_type, &path.launch_clock)
            );
            let _ = writeln!(
                out,
                "  endpoint:   {} ({})",
                path.end_pin,
                describe(&path.end_type, &path.capture_clock)
            );
            if path.multicycle != 1 {
                let _ = writeln!(out, "  multicycle: {} periods", path.multicycle);
            }
            let _ = writeln!(out, "      incr     total  edge  pin");
            for point in &path.points {
                let _ = write!(
                    out,
                    "  {:>8}  {:>8}    {}   {:<24} {} {}",
                    num(point.incr),
                    num(point.cumulative),
                    point.edge.as_str(),
                    point.pin,
                    point.role.as_str(),
                    point.via
                );
                if let (Some(map), Some(span)) = (sources, point.span) {
                    let (file, loc) = map.locate(span);
                    let _ = write!(out, "  at {}:{}:{}", file, loc.line, loc.col);
                }
                let _ = writeln!(out);
            }
            let _ = writeln!(
                out,
                "  {:>8}            data arrival time",
                num(path.arrival)
            );
            let _ = writeln!(
                out,
                "  {:>8}            capture edge {} at {} (launch {} at {})",
                num(path.capture_edge - path.launch_edge),
                path.capture_clock,
                num(path.capture_edge),
                path.launch_clock,
                num(path.launch_edge)
            );
            for (value, label) in [
                (path.capture_latency, "clock network delay (capture)"),
                (path.uncertainty, "clock uncertainty"),
                (path.external_delay, "external delay"),
                (path.constraint, path.check.as_str()),
            ] {
                if value.abs() > 1e-12 || label == path.check.as_str() {
                    let _ = writeln!(out, "  {:>8}            {label}", num(value));
                }
            }
            let _ = writeln!(
                out,
                "  {:>8}            data required time",
                num(path.required)
            );
            let _ = writeln!(
                out,
                "  {:>8}            slack ({})",
                num(path.slack),
                if path.is_violated() {
                    "VIOLATED"
                } else {
                    "met"
                }
            );
            if path.clock_skew.abs() > 1e-12 {
                let _ = writeln!(
                    out,
                    "  {:>8}            clock skew (capture - launch)",
                    num(path.clock_skew)
                );
            }
        }
        if !self.loops.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "combinational loops: {}", self.loops.len());
            for l in &self.loops {
                let mut line = format!("  {}", l.pins.join(" -> "));
                if let (Some(map), span) = (sources, l.span) {
                    let (file, loc) = map.locate(span);
                    let _ = write!(line, "  at {}:{}:{}", file, loc.line, loc.col);
                }
                let _ = writeln!(out, "{line} -> ...");
            }
        }
        if !self.notes.is_empty() {
            let _ = writeln!(out);
            for note in &self.notes {
                let _ = writeln!(out, "note: {note}");
            }
        }
        out
    }
}

/// How a start or end point is described in a path header: what the
/// object is, and which clock it belongs to.
fn describe(cell_type: &str, clock: &str) -> String {
    let what = match cell_type {
        "port" => "port",
        "dff" => "flip-flop",
        "dlatch" => "latch",
        "memrd" => "memory read port",
        "memwr" => "memory write port",
        "assign" => "continuous assignment",
        other => other,
    };
    if clock.is_empty() {
        what.to_owned()
    } else {
        format!("{what} clocked by {clock}")
    }
}

/// Three decimals, and a dash for the infinities that mean "no answer".
///
/// Adding a positive zero turns a negative zero into a positive one, so
/// a report never shows `-0.000`.
fn num(value: f64) -> String {
    if value.is_infinite() || value.is_nan() {
        "-".to_owned()
    } else {
        format!("{:.3}", value + 0.0)
    }
}

/// Matches a glob with `*` (any run) and `?` (one character);
/// everything else is literal.
///
/// `fpga::constraints` has the same matcher, but `timing` must build
/// with no other feature on, so it cannot borrow it.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// ---------------------------------------------------------------------------
// The analysis

/// Runs the analysis over a module in cell form.
///
/// A hierarchical design is flattened first with
/// [`super::graph::flatten_for_timing`]; a module that still holds
/// instances is analysed with those instances as black-box boundaries
/// and the report says so.
pub fn analyze_with(
    module: &Module,
    options: &TimingOptions,
    model: &dyn DelayModel,
    spec: &TimingSpec,
) -> TimingReport {
    Analysis::new(module, options, model, spec).run()
}

/// Runs the analysis with the clocks and path exceptions taken from a
/// constraints value.
#[cfg(feature = "fpga")]
pub fn analyze(
    module: &Module,
    options: &TimingOptions,
    model: &dyn DelayModel,
    constraints: &crate::fpga::Constraints,
) -> TimingReport {
    let spec = TimingSpec::from_constraints(constraints);
    analyze_with(module, options, model, &spec)
}

/// How many paths the search looks at per end point when the spec has
/// path exceptions; see [`Analysis::paths_to`].
const EXCEPTION_SEARCH_DEPTH: usize = 32;

const NEG_INF: Transition = Transition {
    rise: f64::NEG_INFINITY,
    fall: f64::NEG_INFINITY,
};
const POS_INF: Transition = Transition {
    rise: f64::INFINITY,
    fall: f64::INFINITY,
};

struct Analysis<'a> {
    module: &'a Module,
    options: &'a TimingOptions,
    model: &'a dyn DelayModel,
    spec: &'a TimingSpec,
    graph: TimingGraph,
    slews: Vec<Transition>,
    clocks: Vec<ClockSpec>,
    /// Which clock, if any, reaches each pin through the clock network.
    pin_clock: Vec<Option<usize>>,
    /// Clock network latency at each pin, late and early.
    latency_max: Vec<Transition>,
    latency_min: Vec<Transition>,
    /// Effective arc delays after derating.
    arc_max: Vec<Transition>,
    arc_min: Vec<Transition>,
    arrival_max: Vec<Transition>,
    arrival_min: Vec<Transition>,
    required_max: Vec<Transition>,
    required_min: Vec<Transition>,
    seeded: Vec<bool>,
    notes: Vec<String>,
}

impl<'a> Analysis<'a> {
    fn new(
        module: &'a Module,
        options: &'a TimingOptions,
        model: &'a dyn DelayModel,
        spec: &'a TimingSpec,
    ) -> Analysis<'a> {
        let mut graph = TimingGraph::build_with(module, model);
        let slews = graph.annotate(
            module,
            model,
            options.input_slew,
            options.output_load,
            options.wire_load,
        );
        let n = graph.pins.len();
        let arc_max = graph
            .arcs
            .iter()
            .map(|a| a.delay.max_delay.scale(options.late_derate))
            .collect();
        let arc_min = graph
            .arcs
            .iter()
            .map(|a| a.delay.min_delay.scale(options.early_derate))
            .collect();
        let notes = graph
            .notes
            .iter()
            .filter(|n| !n.starts_with("annotated with "))
            .cloned()
            .collect();
        Analysis {
            module,
            options,
            model,
            spec,
            graph,
            slews,
            clocks: Vec::new(),
            pin_clock: vec![None; n],
            latency_max: vec![Transition::ZERO; n],
            latency_min: vec![POS_INF; n],
            arc_max,
            arc_min,
            arrival_max: vec![NEG_INF; n],
            arrival_min: vec![POS_INF; n],
            required_max: vec![POS_INF; n],
            required_min: vec![NEG_INF; n],
            seeded: vec![false; n],
            notes,
        }
    }

    fn run(mut self) -> TimingReport {
        self.resolve_clocks();
        self.propagate_clocks();
        self.propagate_arrivals();
        self.propagate_requireds();
        let (endpoints, paths) = self.enumerate();
        let groups = summarise(&endpoints);
        let pins = self.pin_timings();
        let loops = self
            .graph
            .loops
            .iter()
            .map(|l| LoopReport {
                pins: l.names.clone(),
                span: l.span,
            })
            .collect::<Vec<_>>();
        if !loops.is_empty() {
            self.notes.push(format!(
                "{} combinational loop(s) found; the pins on them have no arrival time",
                loops.len()
            ));
        }
        let mut notes = self.notes;
        notes.dedup();
        let mut clocks = self.clocks;
        clocks.sort_by(|a, b| a.name.cmp(&b.name));
        TimingReport {
            module: self.module.name.as_str().to_owned(),
            model: self.model.name(),
            clock_mode: self.options.clock_mode,
            clocks,
            groups,
            endpoints,
            paths,
            loops,
            pins,
            notes,
        }
    }

    // --- clocks ------------------------------------------------------

    /// The pin a clock enters the design at: its port, or the driver of
    /// the net it names.
    fn clock_source(&self, net_name: &str) -> Option<PinId> {
        if let Some(pin) = self.graph.pin_by_name(net_name)
            && self.graph.pin(pin).cell_type == "port"
        {
            return Some(pin);
        }
        let net = self.module.net_by_name(net_name)?;
        self.graph.driver_of(net)
    }

    fn resolve_clocks(&mut self) {
        self.clocks = self.spec.clocks.clone();
        self.clocks.sort_by(|a, b| a.name.cmp(&b.name));
        for clock in &self.clocks {
            if self.clock_source(&clock.net).is_none() {
                self.notes.push(format!(
                    "clock `{}` is defined on `{}`, which is not a port or a driven net",
                    clock.name, clock.net
                ));
            }
        }
    }

    fn propagate_clocks(&mut self) {
        // Seed each clock at its source pin.
        for (i, clock) in self.clocks.clone().iter().enumerate() {
            if let Some(pin) = self.clock_source(&clock.net) {
                self.pin_clock[pin.index()] = Some(i);
                self.latency_min[pin.index()] = Transition::ZERO;
            }
        }
        // One pass in topological order carries every clock forward. A
        // sequential cell's clock pin is where the clock network stops:
        // what leaves such a pin is data, not clock.
        let order = self.graph.order.clone();
        let stops: Vec<bool> = {
            let mut stops = vec![false; self.graph.pins.len()];
            for pin in &self.graph.clock_pins {
                stops[pin.index()] = true;
            }
            stops
        };
        for pin in order {
            let Some(clock) = self.pin_clock[pin.index()] else {
                continue;
            };
            if stops[pin.index()] {
                continue;
            }
            for &arc_index in self.graph.fanout_indices(pin) {
                let arc = &self.graph.arcs[arc_index];
                let (to, sense) = (arc.to, arc.delay.sense);
                let (dmax, dmin) = (self.arc_max[arc_index], self.arc_min[arc_index]);
                match self.pin_clock[to.index()] {
                    None => self.pin_clock[to.index()] = Some(clock),
                    Some(other) if other != clock => {
                        let note = format!(
                            "pin `{}` is reached by clocks `{}` and `{}`; the first is used",
                            self.graph.name(to),
                            self.clocks[other].name,
                            self.clocks[clock].name
                        );
                        if !self.notes.contains(&note) {
                            self.notes.push(note);
                        }
                        continue;
                    }
                    Some(_) => {}
                }
                let (from_max, from_min) =
                    (self.latency_max[pin.index()], self.latency_min[pin.index()]);
                let mut to_max = self.latency_max[to.index()];
                let mut to_min = self.latency_min[to.index()];
                for out in Edge::ALL {
                    for input in sense.input_edges(out) {
                        to_max.set(
                            out,
                            to_max.get(out).max(from_max.get(*input) + dmax.get(out)),
                        );
                        if from_min.get(*input).is_finite() {
                            to_min.set(
                                out,
                                to_min.get(out).min(from_min.get(*input) + dmin.get(out)),
                            );
                        }
                    }
                }
                self.latency_max[to.index()] = to_max;
                self.latency_min[to.index()] = to_min;
            }
        }
        // A clock pin nothing reached gets a clock of its own, so its
        // flop is still checked instead of silently dropped.
        let mut invented: Vec<(String, Vec<PinId>)> = Vec::new();
        for pin in self.graph.clock_pins.clone() {
            if self.pin_clock[pin.index()].is_some() {
                continue;
            }
            let net = self.graph.pin(pin).nets.first().copied();
            let name = net
                .and_then(|n| self.module.nets.get(n))
                .map_or_else(|| "unclocked".to_owned(), |n| n.name.as_str().to_owned());
            match invented.iter_mut().find(|(n, _)| *n == name) {
                Some((_, pins)) => pins.push(pin),
                None => invented.push((name, vec![pin])),
            }
        }
        // Appended in name order, after the declared clocks, so every
        // index handed out above stays valid.
        invented.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, pins) in invented {
            self.notes.push(format!(
                "no `create_clock` reaches `{name}`; assuming a {} period",
                num(self.options.default_period)
            ));
            let index = self.clocks.len();
            self.clocks.push(ClockSpec {
                name: name.clone(),
                net: name,
                period: self.options.default_period,
                rise: 0.0,
                fall: self.options.default_period / 2.0,
                span: None,
                inferred: true,
            });
            for pin in pins {
                self.pin_clock[pin.index()] = Some(index);
            }
        }
        // A pin the clock never reached has no early latency; zero is
        // the honest answer there, not infinity.
        for slot in &mut self.latency_min {
            if !slot.rise.is_finite() {
                slot.rise = 0.0;
            }
            if !slot.fall.is_finite() {
                slot.fall = 0.0;
            }
        }
        if self.options.clock_mode == ClockMode::Ideal {
            self.latency_max.fill(Transition::ZERO);
            self.latency_min.fill(Transition::ZERO);
        }
    }

    // --- arrival and required ---------------------------------------

    /// The input or output delay a port gets, by glob.
    fn io_delay(&self, port: &str, output: bool) -> f64 {
        let (list, default) = if output {
            (&self.options.output_delays, self.options.output_delay)
        } else {
            (&self.options.input_delays, self.options.input_delay)
        };
        list.iter()
            .find(|(glob, _)| glob_match(glob, port))
            .map_or(default, |(_, d)| *d)
    }

    fn propagate_arrivals(&mut self) {
        // Only a start point with nothing feeding it is seeded. A
        // flip-flop's `q` is a start point too, but its arrival comes
        // from its own clock-to-output arc, which is what makes the
        // clock latency show up in the path.
        for start in self.graph.start_points.clone() {
            let pin = start.pin;
            if !self.graph.fanin_indices(pin).is_empty() {
                continue;
            }
            let value = match start.kind {
                PointKind::Port => self.io_delay(&self.graph.pin(pin).port, false),
                _ => 0.0,
            };
            self.arrival_max[pin.index()] = Transition::both(value);
            self.arrival_min[pin.index()] = Transition::both(value);
            self.seeded[pin.index()] = true;
        }
        // A clock pin's data arrival is the clock's own latency: the
        // clock-to-output arc then carries it to `q`.
        for pin in self.graph.clock_pins.clone() {
            self.arrival_max[pin.index()] = self.latency_max[pin.index()];
            self.arrival_min[pin.index()] = self.latency_min[pin.index()];
            self.seeded[pin.index()] = true;
        }
        for pin in self.graph.order.clone() {
            let from_max = self.arrival_max[pin.index()];
            let from_min = self.arrival_min[pin.index()];
            if from_max.rise.is_infinite() && from_max.fall.is_infinite() {
                continue;
            }
            for &arc_index in self.graph.fanout_indices(pin) {
                let arc = &self.graph.arcs[arc_index];
                let (to, sense) = (arc.to, arc.delay.sense);
                if self.seeded[to.index()] {
                    continue;
                }
                let (dmax, dmin) = (self.arc_max[arc_index], self.arc_min[arc_index]);
                let mut to_max = self.arrival_max[to.index()];
                let mut to_min = self.arrival_min[to.index()];
                for out in Edge::ALL {
                    for input in sense.input_edges(out) {
                        if from_max.get(*input).is_finite() {
                            to_max.set(
                                out,
                                to_max.get(out).max(from_max.get(*input) + dmax.get(out)),
                            );
                        }
                        if from_min.get(*input).is_finite() {
                            to_min.set(
                                out,
                                to_min.get(out).min(from_min.get(*input) + dmin.get(out)),
                            );
                        }
                    }
                }
                self.arrival_max[to.index()] = to_max;
                self.arrival_min[to.index()] = to_min;
            }
        }
    }

    /// The setup or hold time the model gives an end point.
    fn constraint_of(&self, end: &super::graph::EndPoint, check: Check) -> f64 {
        let pin = self.graph.pin(end.pin);
        let Some(cell) = (match pin.site {
            super::graph::PinSite::CellInput(id, _) => self.module.cells.get(id),
            _ => None,
        }) else {
            return 0.0;
        };
        // The clock pin comes from the end point rather than the
        // primitive table, so a black box described by a library is
        // checked too.
        let Some(clock_port) = end
            .clock_pin
            .map(|p| self.graph.pin(p).port.clone())
            .or_else(|| sequential_checks(&cell.kind).1.map(str::to_owned))
        else {
            return 0.0;
        };
        let clock_slew = end
            .clock_pin
            .map_or(Transition::ZERO, |p| self.slews[p.index()]);
        let value = self.model.constraint(&ConstraintQuery {
            module: self.module,
            cell,
            cell_type: &pin.cell_type,
            data_port: &pin.port,
            clock_port: &clock_port,
            data_slew: self.slews[end.pin.index()],
            clock_slew,
            clock_rising: end.clock_rising,
        });
        match check {
            Check::Setup => value.setup.worst(),
            Check::Hold => value.hold.worst(),
        }
    }

    /// The default launch and capture edges for a pair of clocks: the
    /// closest pair with the capture edge strictly after the launch
    /// edge.
    fn edge_pair(&self, launch: usize, capture: usize, rising: bool) -> (f64, f64) {
        let l = &self.clocks[launch];
        let c = &self.clocks[capture];
        if launch == capture {
            return (l.edge_at(rising, 0), c.edge_at(rising, 1));
        }
        // Enough edges to cover any sane ratio of the two periods.
        let mut best: Option<(f64, f64)> = None;
        for k in 0..32u32 {
            let tl = l.edge_at(rising, k);
            for j in 0..32u32 {
                let tc = c.edge_at(rising, j);
                let delta = tc - tl;
                if delta <= 1e-12 {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((bl, bc)) => {
                        let bd = bc - bl;
                        delta < bd - 1e-12 || ((delta - bd).abs() <= 1e-12 && tl < bl)
                    }
                };
                if better {
                    best = Some((tl, tc));
                }
            }
        }
        best.unwrap_or((l.rise, c.rise + c.period))
    }

    fn propagate_requireds(&mut self) {
        let ends = self.graph.end_points.clone();
        for end in &ends {
            let capture = match end.kind {
                PointKind::Sequential => end.clock_pin.and_then(|p| self.pin_clock[p.index()]),
                PointKind::Port => self.io_clock(),
                PointKind::Blackbox => None,
            };
            let Some(capture) = capture else {
                continue;
            };
            // The aggregate propagation assumes the launch clock is the
            // capture clock, which is exact for the common case and is
            // corrected per path by the enumeration.
            let (launch_edge, capture_edge) = self.edge_pair(capture, capture, end.clock_rising);
            let clock_pin = end.clock_pin.unwrap_or(end.pin);
            let latency_late = self.latency_max[clock_pin.index()].worst();
            let latency_early = self.latency_min[clock_pin.index()].best();
            if self.options.check_setup {
                let setup = self.constraint_of(end, Check::Setup);
                let extra = match end.kind {
                    PointKind::Port => self.io_delay(&self.graph.pin(end.pin).port.clone(), true),
                    _ => 0.0,
                };
                let value = (capture_edge - launch_edge) + latency_early
                    - setup
                    - extra
                    - self.options.uncertainty;
                self.required_max[end.pin.index()] =
                    self.required_max[end.pin.index()].min_with(Transition::both(value));
            }
            if self.options.check_hold {
                let hold = self.constraint_of(end, Check::Hold);
                let hold_edge = capture_edge - self.clocks[capture].period;
                let value =
                    (hold_edge - launch_edge) + latency_late + hold + self.options.uncertainty;
                self.required_min[end.pin.index()] =
                    self.required_min[end.pin.index()].max_with(Transition::both(value));
            }
        }
        for pin in self.graph.order.clone().into_iter().rev() {
            for &arc_index in self.graph.fanout_indices(pin) {
                let arc = &self.graph.arcs[arc_index];
                let (to, sense) = (arc.to, arc.delay.sense);
                let (dmax, dmin) = (self.arc_max[arc_index], self.arc_min[arc_index]);
                let to_max = self.required_max[to.index()];
                let to_min = self.required_min[to.index()];
                let mut from_max = self.required_max[pin.index()];
                let mut from_min = self.required_min[pin.index()];
                for out in Edge::ALL {
                    for input in sense.input_edges(out) {
                        if to_max.get(out).is_finite() {
                            from_max.set(
                                *input,
                                from_max.get(*input).min(to_max.get(out) - dmax.get(out)),
                            );
                        }
                        if to_min.get(out).is_finite() {
                            from_min.set(
                                *input,
                                from_min.get(*input).max(to_min.get(out) - dmin.get(out)),
                            );
                        }
                    }
                }
                self.required_max[pin.index()] = from_max;
                self.required_min[pin.index()] = from_min;
            }
        }
    }

    /// The clock input and output delays are relative to.
    fn io_clock(&self) -> Option<usize> {
        match &self.options.io_clock {
            Some(name) => self.clocks.iter().position(|c| c.name == *name),
            None => {
                if self.clocks.is_empty() {
                    None
                } else {
                    Some(0)
                }
            }
        }
    }

    fn pin_timings(&self) -> Vec<PinTiming> {
        self.graph
            .pins
            .iter()
            .enumerate()
            .map(|(i, pin)| PinTiming {
                pin: pin.name.clone(),
                arrival_max: self.arrival_max[i],
                arrival_min: self.arrival_min[i],
                required_max: self.required_max[i],
                required_min: self.required_min[i],
                clock: self.pin_clock[i].map(|c| self.clocks[c].name.clone()),
                clock_latency: self.latency_max[i],
            })
            .collect()
    }

    // --- path enumeration --------------------------------------------

    /// Every name an exception may refer to a pin by: the pin itself,
    /// the cell or port that owns it, and the nets it touches.
    fn names_of(&self, pin: PinId) -> Vec<String> {
        let p = self.graph.pin(pin);
        let mut names = vec![p.name.clone(), p.owner.clone()];
        for net in &p.nets {
            if let Some(net) = self.module.nets.get(*net) {
                names.push(net.name.as_str().to_owned());
            }
        }
        // A sequential start or end point is usually named by the net
        // its `q` drives, which is not one of this pin's own nets.
        if let super::graph::PinSite::CellInput(id, _) | super::graph::PinSite::CellOutput(id, _) =
            p.site
            && let Some(cell) = self.module.cells.get(id)
        {
            for (_, net) in &cell.outputs {
                if let Some(net) = self.module.nets.get(*net) {
                    names.push(net.name.as_str().to_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// The exception that applies to a path, if any: a false path wins
    /// over a multicycle, and the largest cycle count wins among
    /// multicycles.
    fn exception_for(
        &self,
        start_names: &[String],
        end_names: &[String],
        check: Check,
    ) -> Option<&PathException> {
        let mut best: Option<&PathException> = None;
        for exception in &self.spec.exceptions {
            let from_ok = exception
                .from
                .as_ref()
                .is_none_or(|g| start_names.iter().any(|n| glob_match(g, n)));
            let to_ok = exception
                .to
                .as_ref()
                .is_none_or(|g| end_names.iter().any(|n| glob_match(g, n)));
            if !from_ok || !to_ok {
                continue;
            }
            match exception.kind {
                ExceptionKind::False => return Some(exception),
                ExceptionKind::Multicycle { hold, .. } => {
                    if hold != (check == Check::Hold) {
                        continue;
                    }
                    let better = match best.map(|b| b.kind) {
                        Some(ExceptionKind::Multicycle { cycles: c, .. }) => {
                            matches!(exception.kind, ExceptionKind::Multicycle { cycles, .. } if cycles > c)
                        }
                        _ => true,
                    };
                    if better {
                        best = Some(exception);
                    }
                }
            }
        }
        best
    }

    fn enumerate(&mut self) -> (Vec<EndpointSlack>, Vec<TimingPath>) {
        let mut endpoints = Vec::new();
        let mut paths = Vec::new();
        let mut unconstrained = 0usize;
        let ends = self.graph.end_points.clone();
        for end in &ends {
            let capture = match end.kind {
                PointKind::Sequential => end.clock_pin.and_then(|p| self.pin_clock[p.index()]),
                PointKind::Port => self.io_clock(),
                PointKind::Blackbox => None,
            };
            let Some(capture) = capture else {
                if self.arrival_max[end.pin.index()].worst().is_finite() {
                    unconstrained += 1;
                }
                continue;
            };
            for check in [Check::Setup, Check::Hold] {
                if (check == Check::Setup && !self.options.check_setup)
                    || (check == Check::Hold && !self.options.check_hold)
                {
                    continue;
                }
                let found = self.paths_to(end, capture, check);
                if let Some(first) = found.first() {
                    endpoints.push(EndpointSlack {
                        pin: first.end_pin.clone(),
                        check,
                        slack: first.slack,
                        launch_clock: first.launch_clock.clone(),
                        capture_clock: first.capture_clock.clone(),
                    });
                }
                paths.extend(found);
            }
        }
        if unconstrained > 0 {
            self.notes.push(format!(
                "{unconstrained} end point(s) have no capture clock and were not checked"
            ));
        }
        endpoints.sort_by(|a, b| {
            a.slack
                .total_cmp(&b.slack)
                .then_with(|| a.check.cmp(&b.check))
                .then_with(|| a.pin.cmp(&b.pin))
        });
        paths.sort_by(|a, b| {
            a.slack
                .total_cmp(&b.slack)
                .then_with(|| a.check.cmp(&b.check))
                .then_with(|| a.end_pin.cmp(&b.end_pin))
                .then_with(|| a.start_pin.cmp(&b.start_pin))
                .then_with(|| a.points.len().cmp(&b.points.len()))
        });
        paths.truncate(self.options.max_paths);
        (endpoints, paths)
    }

    /// The worst paths to one end point, worst first, with exceptions
    /// applied.
    fn paths_to(
        &self,
        end: &super::graph::EndPoint,
        capture: usize,
        check: Check,
    ) -> Vec<TimingPath> {
        let want = self.options.paths_per_endpoint.max(1);
        // A path exception changes a path's slack, so the worst path by
        // arrival is no longer necessarily the worst by slack: look at
        // more of them and re-rank. Deeper than this and a huge fan-in
        // cone would dominate the run time, which the budget also caps.
        let depth = if self.spec.exceptions.is_empty() {
            want
        } else {
            want.max(EXCEPTION_SEARCH_DEPTH)
        };
        let end_names = self.names_of(end.pin);
        let arrival = match check {
            Check::Setup => &self.arrival_max,
            Check::Hold => &self.arrival_min,
        };
        let mut heap: BinaryHeap<Ranked> = BinaryHeap::new();
        for edge in Edge::ALL {
            let value = arrival[end.pin.index()].get(edge);
            if !value.is_finite() {
                continue;
            }
            heap.push(Ranked {
                key: rank(check, value),
                pin: end.pin,
                edge,
                suffix: 0.0,
                trail: Vec::new(),
            });
        }
        let mut out = Vec::new();
        let mut budget = self.options.search_budget;
        while let Some(state) = heap.pop() {
            if out.len() >= depth || budget == 0 {
                break;
            }
            budget -= 1;
            if self.seeded[state.pin.index()] {
                if let Some(path) = self.finish_path(end, capture, check, &state, &end_names) {
                    out.push(path);
                }
                continue;
            }
            for &arc_index in self.graph.fanin_indices(state.pin) {
                let arc = &self.graph.arcs[arc_index];
                let (from, sense) = (arc.from, arc.delay.sense);
                let delay = match check {
                    Check::Setup => self.arc_max[arc_index],
                    Check::Hold => self.arc_min[arc_index],
                }
                .get(state.edge);
                for input in sense.input_edges(state.edge) {
                    let value = arrival[from.index()].get(*input);
                    if !value.is_finite() {
                        continue;
                    }
                    let suffix = state.suffix + delay;
                    let mut trail = state.trail.clone();
                    trail.push((arc_index, state.edge));
                    heap.push(Ranked {
                        key: rank(check, value + suffix),
                        pin: from,
                        edge: *input,
                        suffix,
                        trail,
                    });
                }
            }
        }
        // Applying a multicycle exception changes a path's slack, so the
        // order the search produced may no longer hold.
        out.sort_by(|a, b| {
            a.slack
                .total_cmp(&b.slack)
                .then_with(|| a.start_pin.cmp(&b.start_pin))
        });
        // The same route reached on the rising and on the falling edge
        // is one path in a report; the worse edge is the one that
        // matters, and it now sorts first.
        let mut seen: Vec<String> = Vec::new();
        out.retain(|path| {
            let route = path
                .points
                .iter()
                .map(|p| p.pin.as_str())
                .collect::<Vec<_>>()
                .join(">");
            if seen.contains(&route) {
                false
            } else {
                seen.push(route);
                true
            }
        });
        out.truncate(want);
        out
    }

    fn finish_path(
        &self,
        end: &super::graph::EndPoint,
        capture: usize,
        check: Check,
        state: &Ranked,
        end_names: &[String],
    ) -> Option<TimingPath> {
        let start_pin = state.pin;
        let start_names = self.names_of(start_pin);
        let exception = self.exception_for(&start_names, end_names, check);
        let mut multicycle = 1u32;
        match exception.map(|e| e.kind) {
            Some(ExceptionKind::False) => return None,
            Some(ExceptionKind::Multicycle { cycles, .. }) => multicycle = cycles.max(1),
            None => {}
        }
        let launch = self.pin_clock[start_pin.index()].or_else(|| {
            if self.graph.pin(start_pin).cell_type == "port" {
                self.io_clock()
            } else {
                None
            }
        });
        let launch = launch.unwrap_or(capture);
        let rising = end.clock_rising;
        let (launch_edge, capture_edge) = self.edge_pair(launch, capture, rising);
        let capture_period = self.clocks[capture].period;
        let capture_edge = match check {
            Check::Setup => capture_edge + f64::from(multicycle - 1) * capture_period,
            Check::Hold => {
                capture_edge - capture_period - f64::from(multicycle - 1) * capture_period
            }
        };
        let capture_latency = end.clock_pin.map_or(Transition::ZERO, |p| match check {
            Check::Setup => self.latency_min[p.index()],
            Check::Hold => self.latency_max[p.index()],
        });
        let launch_latency = self.latency_max[start_pin.index()];
        let constraint = self.constraint_of(end, check);
        let extra = match end.kind {
            PointKind::Port => self.io_delay(&self.graph.pin(end.pin).port, true),
            _ => 0.0,
        };
        // The path's own arrival, which is the seed plus every arc on
        // the trail; the aggregate arrival at the pin may be larger
        // because another path reaches it.
        let seed = match check {
            Check::Setup => self.arrival_max[start_pin.index()].get(state.edge),
            Check::Hold => self.arrival_min[start_pin.index()].get(state.edge),
        };
        let arrival = seed + state.suffix;
        let (required, slack) = match check {
            Check::Setup => {
                let required = (capture_edge - launch_edge) + capture_latency.worst()
                    - constraint
                    - extra
                    - self.options.uncertainty;
                (required, required - arrival)
            }
            Check::Hold => {
                let required = (capture_edge - launch_edge)
                    + capture_latency.worst()
                    + constraint
                    + extra
                    + self.options.uncertainty;
                (required, arrival - required)
            }
        };
        let points = self.trail_points(start_pin, state, seed);
        Some(TimingPath {
            check,
            start: self.graph.pin(start_pin).owner.clone(),
            end: self.graph.pin(end.pin).owner.clone(),
            start_pin: self.graph.name(start_pin).to_owned(),
            end_pin: self.graph.name(end.pin).to_owned(),
            start_type: self.graph.pin(start_pin).cell_type.clone(),
            end_type: self.graph.pin(end.pin).cell_type.clone(),
            launch_clock: self.clocks[launch].name.clone(),
            capture_clock: self.clocks[capture].name.clone(),
            launch_edge,
            capture_edge,
            capture_latency: capture_latency.worst(),
            clock_skew: capture_latency.worst() - launch_latency.worst(),
            external_delay: match check {
                Check::Setup => -extra,
                Check::Hold => extra,
            },
            constraint: match check {
                Check::Setup => -constraint,
                Check::Hold => constraint,
            },
            uncertainty: match check {
                Check::Setup => -self.options.uncertainty,
                Check::Hold => self.options.uncertainty,
            },
            multicycle,
            points,
            arrival,
            required,
            slack,
        })
    }

    fn trail_points(&self, start_pin: PinId, state: &Ranked, seed: f64) -> Vec<PathPoint> {
        let mut points = Vec::with_capacity(state.trail.len() + 1);
        let start = self.graph.pin(start_pin);
        points.push(PathPoint {
            pin: start.name.clone(),
            role: if start.role == PinRole::Clock {
                PointRole::Clock
            } else {
                PointRole::Start
            },
            edge: state.edge,
            incr: seed,
            cumulative: seed,
            via: match self.pin_clock[start_pin.index()] {
                Some(c) if start.role == PinRole::Clock => self.clocks[c].name.clone(),
                _ => start.cell_type.clone(),
            },
            span: Some(start.span),
        });
        let mut total = seed;
        for (arc_index, edge) in state.trail.iter().rev() {
            let arc = &self.graph.arcs[*arc_index];
            let delay = arc.delay.max_delay.get(*edge);
            total += delay;
            let pin = self.graph.pin(arc.to);
            points.push(PathPoint {
                pin: pin.name.clone(),
                role: match arc.kind {
                    ArcKind::Cell => PointRole::Cell,
                    ArcKind::Net => PointRole::Net,
                },
                edge: *edge,
                incr: delay,
                cumulative: total,
                via: match arc.kind {
                    ArcKind::Cell => pin.cell_type.clone(),
                    ArcKind::Net => arc
                        .net
                        .and_then(|n| self.module.nets.get(n))
                        .map_or_else(String::new, |n| n.name.as_str().to_owned()),
                },
                span: Some(pin.span),
            });
        }
        if state.trail.is_empty() {
            // The path is a single pin: the start point is the end
            // point, which happens when a port feeds a port.
            if let Some(only) = points.last_mut() {
                only.role = PointRole::End;
            }
        }
        points
    }
}

/// Groups the end points by launch / capture clock and check.
fn summarise(endpoints: &[EndpointSlack]) -> Vec<GroupSummary> {
    let mut groups: Vec<GroupSummary> = Vec::new();
    for end in endpoints {
        let at = groups.iter().position(|g| {
            g.launch_clock == end.launch_clock
                && g.capture_clock == end.capture_clock
                && g.check == end.check
        });
        match at {
            Some(i) => {
                groups[i].endpoints += 1;
                if end.slack < 0.0 {
                    groups[i].violating += 1;
                }
                if end.slack < groups[i].worst_slack {
                    groups[i].worst_slack = end.slack;
                    groups[i].worst_endpoint = end.pin.clone();
                }
            }
            None => groups.push(GroupSummary {
                launch_clock: end.launch_clock.clone(),
                capture_clock: end.capture_clock.clone(),
                check: end.check,
                worst_slack: end.slack,
                worst_endpoint: end.pin.clone(),
                endpoints: 1,
                violating: usize::from(end.slack < 0.0),
            }),
        }
    }
    groups.sort_by(|a, b| {
        a.launch_clock
            .cmp(&b.launch_clock)
            .then_with(|| a.capture_clock.cmp(&b.capture_clock))
            .then_with(|| a.check.cmp(&b.check))
    });
    groups
}

/// The search key: the setup search wants the largest total, the hold
/// search the smallest, and a binary heap only pops the largest.
fn rank(check: Check, value: f64) -> f64 {
    match check {
        Check::Setup => value,
        Check::Hold => -value,
    }
}

/// One state of the best-first path search.
struct Ranked {
    key: f64,
    pin: PinId,
    edge: Edge,
    suffix: f64,
    trail: Vec<(usize, Edge)>,
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        // `total_cmp` gives a total order over floats, and the tie
        // breaks keep the search deterministic.
        self.key
            .total_cmp(&other.key)
            .then_with(|| other.trail.len().cmp(&self.trail.len()))
            .then_with(|| other.pin.cmp(&self.pin))
            .then_with(|| other.edge.cmp(&self.edge))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::types::Type;
    use crate::ir::{CellKind, Module};
    use crate::source::SourceMap;
    use crate::timing::delay::{
        ArcQuery, ArcResult, NetQuery, PinQuery, Sense, SequentialConstraint, UnitModel,
    };

    /// Timing numbers are floats, so tests compare with a tolerance
    /// rather than with `==`: one femtosecond is orders of magnitude
    /// below anything a library characterises, so a difference this
    /// small can only be floating-point rounding.
    const EPS: f64 = 1e-9;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < EPS
    }

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("timing-test", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn dff() -> CellKind {
        CellKind::Dff {
            clk_pos: true,
            has_enable: false,
            reset: None,
        }
    }

    /// `d -> ff1 -> and(b) -> ff2 -> q`, all on one clock.
    fn reg_to_reg() -> Module {
        let mut b = ModuleBuilder::new("rr", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let other = b.input("b", Type::bit());
        let q = b.output("q", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let gated = b.add_net("gated", Type::bit());
        let (ce, de) = (b.net(clk), b.net(d));
        b.cell(
            "ff1",
            dff(),
            vec![("clk".into(), ce), ("d".into(), de)],
            vec![("q".into(), mid)],
        );
        let (me, be) = (b.net(mid), b.net(other));
        b.cell2("g", CellKind::And, me, be, gated);
        let ge = b.net(gated);
        b.cell(
            "ff2",
            dff(),
            vec![("clk".into(), ce), ("d".into(), ge)],
            vec![("q".into(), q)],
        );
        b.finish()
    }

    fn sys(period: f64) -> TimingSpec {
        TimingSpec::new().with_clock(ClockSpec::new("sys", "clk", period))
    }

    /// One unit per cell arc, 0.2 setup, 0.1 hold.
    fn model() -> UnitModel {
        UnitModel::new().with_setup(0.2).with_hold(0.1)
    }

    #[test]
    fn register_to_register_slack_by_hand() {
        let m = reg_to_reg();
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &sys(10.0));
        // ff1/clk -> ff1/q is one unit (clock-to-Q), g/a -> g/y is one
        // more, the two net arcs are free: arrival at ff2/d is 2.000.
        // Setup: 10.000 (capture edge) - 0.200 (setup) - 2.000 = 7.800.
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 7.8));
        // Hold: the hold capture edge is 10.000 - 10.000 = 0.000, so the
        // requirement is 0.100. The earliest data at ff2/d does not come
        // from ff1 but from the port `b` through the AND, one unit after
        // the edge, so the slack is 1.000 - 0.100 = 0.900.
        assert!(close(report.slack_at("ff2/d", Check::Hold).unwrap(), 0.9));
        // ff1/d is fed straight from the port with no delay at all.
        assert!(close(report.slack_at("ff1/d", Check::Setup).unwrap(), 9.8));
        assert!(close(report.slack_at("ff1/d", Check::Hold).unwrap(), -0.1));
        // The output port sees ff2/q one unit after the clock edge.
        assert!(close(report.slack_at("q", Check::Setup).unwrap(), 9.0));
        assert!(close(report.worst_setup().unwrap(), 7.8));
        assert!(close(report.worst_hold().unwrap(), -0.1));
        assert!(report.has_violations());
        assert_eq!(report.clocks.len(), 1);
        assert_eq!(report.model, "unit");
        assert!(report.loops.is_empty());
    }

    #[test]
    fn arrival_and_required_propagation() {
        let m = reg_to_reg();
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &sys(10.0));
        // Arrival: the clock pin is the seed at 0, `q` one unit later,
        // the net arc is free, the AND another unit.
        assert!(close(report.pin("ff1/clk").unwrap().arrival_max.rise, 0.0));
        assert!(close(report.pin("ff1/q").unwrap().arrival_max.rise, 1.0));
        assert!(close(report.pin("g/a").unwrap().arrival_max.rise, 1.0));
        assert!(close(report.pin("g/y").unwrap().arrival_max.rise, 2.0));
        assert!(close(report.pin("ff2/d").unwrap().arrival_max.rise, 2.0));
        // Required, counted back from 9.800 at ff2/d.
        assert!(close(report.pin("ff2/d").unwrap().required_max.rise, 9.8));
        assert!(close(report.pin("g/y").unwrap().required_max.rise, 9.8));
        assert!(close(report.pin("g/a").unwrap().required_max.rise, 8.8));
        assert!(close(report.pin("ff1/q").unwrap().required_max.rise, 8.8));
        // Slack is the same either way round.
        assert!(close(report.pin("g/a").unwrap().setup_slack(), 7.8));
        assert!(close(report.pin("ff2/d").unwrap().hold_slack(), 0.9));
        // The early arrival at ff2/d is the port path, not ff1's.
        assert!(close(report.pin("ff2/d").unwrap().arrival_min.rise, 1.0));
        assert_eq!(report.pin("ff1/clk").unwrap().clock.as_deref(), Some("sys"));
    }

    #[test]
    fn a_false_path_removes_the_check() {
        let m = reg_to_reg();
        // Cutting every path into ff2 leaves it with no check at all.
        let spec = sys(10.0).with_false_path(None, Some("ff2"));
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &spec);
        assert!(report.slack_at("ff2/d", Check::Setup).is_none());
        assert!(report.slack_at("ff2/d", Check::Hold).is_none());
        // The other end points are untouched.
        assert!(close(report.slack_at("ff1/d", Check::Setup).unwrap(), 9.8));
        assert!(!report.paths.iter().any(|p| p.end_pin == "ff2/d"));
        // Cutting only the register-to-register path leaves the one in
        // from the port `b`: 10.000 - 0.200 - 1.000 (the AND) = 8.800.
        let spec = sys(10.0).with_false_path(Some("ff1"), Some("ff2"));
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &spec);
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 8.8));
        assert!(
            !report
                .paths
                .iter()
                .any(|p| p.end_pin == "ff2/d" && p.start == "ff1")
        );
    }

    #[test]
    fn a_multicycle_path_buys_a_period() {
        let m = reg_to_reg();
        let options = TimingOptions {
            paths_per_endpoint: 4,
            max_paths: 50,
            ..TimingOptions::default()
        };
        let spec = sys(10.0).with_multicycle_path(Some("ff1"), Some("ff2"), 2, false);
        let report = analyze_with(&m, &options, &model(), &spec);
        // The register-to-register path gets exactly one extra period:
        // 7.800 + 10.000.
        let path = report
            .paths
            .iter()
            .find(|p| p.end_pin == "ff2/d" && p.check == Check::Setup && p.start == "ff1")
            .unwrap();
        assert_eq!(path.multicycle, 2);
        assert!(close(path.slack, 17.8));
        // The path in from the port `b` is not covered by the exception,
        // so it is now the worst one at that end point.
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 8.8));
        // `-hold` moves the hold check one period earlier instead: the
        // hold capture edge goes from 0.000 to -10.000, so the earliest
        // arrival of 1.000 has 1.000 + 10.000 - 0.100 of slack.
        let spec = sys(10.0).with_multicycle_path(None, Some("ff2"), 2, true);
        let report = analyze_with(&m, &options, &model(), &spec);
        assert!(close(report.slack_at("ff2/d", Check::Hold).unwrap(), 10.9));
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 7.8));
    }

    #[test]
    fn two_clocks_use_the_closest_edge_pair() {
        let mut b = ModuleBuilder::new("cross", span());
        let fast = b.input("fast", Type::bit());
        let slow = b.input("slow", Type::bit());
        let d = b.input("d", Type::bit());
        let q = b.output("q", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let (fe, se, de) = (b.net(fast), b.net(slow), b.net(d));
        b.cell(
            "src",
            dff(),
            vec![("clk".into(), fe), ("d".into(), de)],
            vec![("q".into(), mid)],
        );
        let me = b.net(mid);
        b.cell(
            "dst",
            dff(),
            vec![("clk".into(), se), ("d".into(), me)],
            vec![("q".into(), q)],
        );
        let m = b.finish();
        let spec = TimingSpec::new()
            .with_clock(ClockSpec::new("fast", "fast", 10.0))
            .with_clock(ClockSpec::new("slow", "slow", 15.0));
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &spec);
        // Edges of `fast` are 0, 10, 20, 30; of `slow` 0, 15, 30. The
        // tightest pair with capture after launch is 10 -> 15, five
        // nanoseconds apart, so the budget is 5.000 - 0.200 and the
        // arrival is one unit of clock-to-Q.
        assert!(close(report.slack_at("dst/d", Check::Setup).unwrap(), 3.8));
        let path = report
            .paths
            .iter()
            .find(|p| p.end_pin == "dst/d" && p.check == Check::Setup)
            .unwrap();
        assert_eq!(path.launch_clock, "fast");
        assert_eq!(path.capture_clock, "slow");
        assert!(close(path.launch_edge, 10.0));
        assert!(close(path.capture_edge, 15.0));
        assert!(
            report
                .groups
                .iter()
                .any(|g| g.launch_clock == "fast" && g.capture_clock == "slow")
        );
    }

    #[test]
    fn a_propagated_clock_shows_its_skew() {
        // The capture flop's clock goes through an extra buffer, so it
        // sees the edge one unit late and gains that unit of setup.
        let mut b = ModuleBuilder::new("skew", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let q = b.output("q", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let clk2 = b.add_net("clk2", Type::bit());
        let (ce, de) = (b.net(clk), b.net(d));
        b.cell(
            "cbuf",
            CellKind::Buf,
            vec![("a".into(), ce)],
            vec![("y".into(), clk2)],
        );
        b.cell(
            "src",
            dff(),
            vec![("clk".into(), ce), ("d".into(), de)],
            vec![("q".into(), mid)],
        );
        let (c2, me) = (b.net(clk2), b.net(mid));
        b.cell(
            "dst",
            dff(),
            vec![("clk".into(), c2), ("d".into(), me)],
            vec![("q".into(), q)],
        );
        let m = b.finish();
        let ideal = analyze_with(&m, &TimingOptions::default(), &model(), &sys(10.0));
        assert!(close(ideal.slack_at("dst/d", Check::Setup).unwrap(), 8.8));
        let options = TimingOptions {
            clock_mode: ClockMode::Propagated,
            ..TimingOptions::default()
        };
        let report = analyze_with(&m, &options, &model(), &sys(10.0));
        // Capture latency is one unit (the buffer), launch latency is
        // zero, so setup gains a unit and hold loses one.
        assert!(close(report.slack_at("dst/d", Check::Setup).unwrap(), 9.8));
        assert!(close(report.slack_at("dst/d", Check::Hold).unwrap(), -0.1));
        let path = report
            .paths
            .iter()
            .find(|p| p.end_pin == "dst/d" && p.check == Check::Setup)
            .unwrap();
        assert!(close(path.clock_skew, 1.0));
        assert!(close(path.capture_latency, 1.0));
    }

    #[test]
    fn input_and_output_delays_eat_the_budget() {
        let m = reg_to_reg();
        let options = TimingOptions {
            input_delay: 1.0,
            output_delay: 2.0,
            input_delays: vec![("b".to_owned(), 3.0)],
            ..TimingOptions::default()
        };
        let report = analyze_with(&m, &options, &model(), &sys(10.0));
        // `d` arrives 1.000 late, so ff1/d has 10.000 - 0.200 - 1.000.
        assert!(close(report.slack_at("ff1/d", Check::Setup).unwrap(), 8.8));
        // `b` is 3.000 late and goes through the AND, so ff2/d gets
        // 10.000 - 0.200 - (3.000 + 1.000) = 5.800, worse than the
        // register-to-register path's 7.800.
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 5.8));
        // The output port loses its 2.000 of external delay.
        assert!(close(report.slack_at("q", Check::Setup).unwrap(), 7.0));
    }

    #[test]
    fn uncertainty_and_derating_move_every_number() {
        let m = reg_to_reg();
        let options = TimingOptions {
            uncertainty: 0.5,
            late_derate: 2.0,
            ..TimingOptions::default()
        };
        let report = analyze_with(&m, &options, &model(), &sys(10.0));
        // Arrival doubles to 4.000 and the budget loses 0.500.
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 5.3));
        // Hold is an early check, so derating the late corner leaves it
        // alone; the uncertainty still makes it harder. The earliest
        // data is the port path at 1.000 against 0.100 + 0.500.
        assert!(close(report.slack_at("ff2/d", Check::Hold).unwrap(), 0.4));
    }

    #[test]
    fn a_combinational_loop_is_reported_not_walked() {
        let mut b = ModuleBuilder::new("loopy", span());
        let a = b.input("a", Type::bit());
        let y = b.output("y", Type::bit());
        let t = b.add_net("t", Type::bit());
        let (ae, te) = (b.net(a), b.net(t));
        let ye = b.net(y);
        b.cell2("g1", CellKind::And, ae, ye, t);
        b.cell(
            "g2",
            CellKind::Not,
            vec![("a".into(), te)],
            vec![("y".into(), y)],
        );
        let m = b.finish();
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &sys(10.0));
        assert_eq!(report.loops.len(), 1);
        assert!(
            report
                .notes
                .iter()
                .any(|n| n.contains("combinational loop"))
        );
        let text = report.render();
        assert!(text.contains("combinational loops: 1"), "{text}");
    }

    /// A model whose rising and falling delays differ, to check that the
    /// two are kept apart along a path.
    struct Asymmetric;

    impl DelayModel for Asymmetric {
        fn name(&self) -> String {
            "asymmetric".to_owned()
        }
        fn cell_arc(&self, query: &ArcQuery<'_>) -> ArcResult {
            // Rising outputs take 3, falling outputs take 1.
            ArcResult::fixed(Transition::new(3.0, 1.0), Transition::ZERO, query.sense)
        }
        fn net_arc(&self, query: &NetQuery<'_>) -> ArcResult {
            ArcResult::fixed(Transition::ZERO, query.input_slew, Sense::Positive)
        }
        fn pin_capacitance(&self, _query: &PinQuery<'_>) -> f64 {
            0.0
        }
        fn constraint(&self, _query: &ConstraintQuery<'_>) -> SequentialConstraint {
            SequentialConstraint::default()
        }
    }

    #[test]
    fn rise_and_fall_stay_apart_through_an_inverter() {
        let mut b = ModuleBuilder::new("inv", span());
        let clk = b.input("clk", Type::bit());
        let d = b.input("d", Type::bit());
        let q = b.output("q", Type::bit());
        let mid = b.add_net("mid", Type::bit());
        let inv = b.add_net("inv", Type::bit());
        let (ce, de) = (b.net(clk), b.net(d));
        b.cell(
            "src",
            dff(),
            vec![("clk".into(), ce), ("d".into(), de)],
            vec![("q".into(), mid)],
        );
        let me = b.net(mid);
        b.cell(
            "n",
            CellKind::Not,
            vec![("a".into(), me)],
            vec![("y".into(), inv)],
        );
        let ie = b.net(inv);
        b.cell(
            "dst",
            dff(),
            vec![("clk".into(), ce), ("d".into(), ie)],
            vec![("q".into(), q)],
        );
        let m = b.finish();
        let report = analyze_with(&m, &TimingOptions::default(), &Asymmetric, &sys(20.0));
        let src_q = report.pin("src/q").unwrap();
        assert!(close(src_q.arrival_max.rise, 3.0));
        assert!(close(src_q.arrival_max.fall, 1.0));
        // The inverter flips the edges: a rising output at `n/y` comes
        // from the falling arrival at `n/a` (1.000) plus the rising
        // delay (3.000), and a falling output from 3.000 + 1.000.
        let inv_y = report.pin("n/y").unwrap();
        assert!(close(inv_y.arrival_max.rise, 4.0));
        assert!(close(inv_y.arrival_max.fall, 4.0));
        // Both edges are 4.000 here, so the slack is 20.000 - 4.000.
        assert!(close(report.slack_at("dst/d", Check::Setup).unwrap(), 16.0));
    }

    #[test]
    fn several_paths_per_endpoint_come_out_worst_first() {
        let m = reg_to_reg();
        let options = TimingOptions {
            paths_per_endpoint: 4,
            max_paths: 50,
            ..TimingOptions::default()
        };
        let report = analyze_with(&m, &options, &model(), &sys(10.0));
        let to_ff2: Vec<&TimingPath> = report
            .paths
            .iter()
            .filter(|p| p.end_pin == "ff2/d" && p.check == Check::Setup)
            .collect();
        // Two ways in: from ff1 through the AND, and from the port `b`.
        assert!(to_ff2.len() >= 2);
        for pair in to_ff2.windows(2) {
            assert!(pair[0].slack <= pair[1].slack + EPS);
        }
        // Every point's cumulative delay is the running sum.
        for path in &report.paths {
            let mut total = 0.0;
            for (i, point) in path.points.iter().enumerate() {
                total += point.incr;
                assert!(close(point.cumulative, total), "point {i} of {path:?}");
            }
            assert!(close(total, path.arrival));
        }
    }

    #[test]
    fn an_unconstrained_clock_is_invented_and_announced() {
        let m = reg_to_reg();
        let options = TimingOptions {
            default_period: 4.0,
            ..TimingOptions::default()
        };
        let report = analyze_with(&m, &options, &model(), &TimingSpec::new());
        assert_eq!(report.clocks.len(), 1);
        assert_eq!(report.clocks[0].name, "clk");
        assert!(report.clocks[0].inferred);
        assert!(close(report.slack_at("ff2/d", Check::Setup).unwrap(), 1.8));
        assert!(report.notes.iter().any(|n| n.contains("create_clock")));
    }

    #[test]
    fn rendering_is_stable_and_summarises() {
        let m = reg_to_reg();
        let report = analyze_with(&m, &TimingOptions::default(), &model(), &sys(10.0));
        let text = report.render();
        assert_eq!(text, report.render());
        assert!(text.starts_with("timing summary for module `rr`"));
        assert!(text.contains("sys -> sys"));
        assert!(text.contains("data required time"));
        assert!(report.render_summary().lines().count() < text.lines().count());
        // With a source map the points carry locations.
        let mut map = SourceMap::new();
        let _ = map.add("timing-test", "");
        assert!(report.render_sources(&map).contains("at timing-test:1:1"));
        assert_eq!(Check::Hold.as_str(), "hold");
        assert_eq!(PointRole::Net.as_str(), "net");
    }

    #[test]
    fn globs_match_the_way_sdc_does() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("ff*", "ff2"));
        assert!(glob_match("u_cpu/*", "u_cpu/alu"));
        assert!(glob_match("d?", "d1"));
        assert!(!glob_match("d?", "d12"));
        assert!(!glob_match("ff*", "gf2"));
        assert!(glob_match("", ""));
    }

    #[cfg(feature = "fpga")]
    #[test]
    fn constraints_become_a_spec() {
        use crate::diag::Diagnostics;
        let mut map = SourceMap::new();
        let file = map
            .add(
                "c.rcf",
                "create_clock -name sys -period 20 clk\nset_false_path -from d -to q\n\
                 set_multicycle_path 3 -from a -to b\n",
            )
            .unwrap();
        let mut diags = Diagnostics::new();
        let text = map.file(file).text().to_owned();
        let constraints = crate::fpga::Constraints::parse(&text, file, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let spec = TimingSpec::from_constraints(&constraints);
        assert_eq!(spec.clocks.len(), 1);
        assert!(close(spec.clock("sys").unwrap().period, 20.0));
        assert!(close(spec.clock("sys").unwrap().fall, 10.0));
        assert_eq!(spec.exceptions.len(), 2);
        assert_eq!(spec.exceptions[0].kind, ExceptionKind::False);
        assert_eq!(
            spec.exceptions[1].kind,
            ExceptionKind::Multicycle {
                cycles: 3,
                hold: false
            }
        );
        // `analyze` is `analyze_with` over the converted spec.
        let m = reg_to_reg();
        let a = analyze(&m, &TimingOptions::default(), &model(), &constraints);
        let b = analyze_with(&m, &TimingOptions::default(), &model(), &spec);
        assert_eq!(a.render(), b.render());
    }
}
