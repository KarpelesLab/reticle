//! Turning a Liberty library into something the mapper can use.
//!
//! [`crate::asic::liberty`] reads a `.lib` file into a faithful but
//! untyped-for-synthesis view: cells, pins, boolean functions, timing
//! tables. The standard-cell mapper
//! ([`crate::synth::techmap::gate_map`]) wants something much smaller —
//! a [`GateLibrary`] of single-output combinational cells, each a truth
//! table, an area and one delay per pin — and the flip-flop mapper wants
//! to know which sequential cell implements which inferred [`Dff`].
//! [`StdCells`] is that translation, and it is deliberately the only
//! place in the crate that knows both vocabularies.
//!
//! [`Dff`]: crate::ir::CellKind::Dff
//!
//! # Combinational cells
//!
//! A cell reaches the mapper when it is not `dont_use`, has exactly one
//! output pin, that pin has a `function` and no `three_state`, the
//! function mentions nothing but the cell's own input pins, the cell has
//! an `area`, and it has at most [`LibraryOptions::max_inputs`] inputs
//! (four by default, which is what the mapper's Boolean matching
//! covers). Its function becomes a truth table through
//! [`BoolExpr::truth_table`] over the input pins **in declaration
//! order**, so pin *i* is variable *i* of the table, which is the order
//! [`Gate::pins`] records and the order the mapper wires.
//!
//! Every cell that does not reach the mapper is kept in
//! [`StdCells::skipped`] with the reason, because "the library has 400
//! cells and the mapper used 9" is a question that has to be answerable.
//! Physical-only cells (fill, tap, decap), tri-state buffers,
//! multi-output cells such as a full adder, wide multiplexers beyond the
//! truth-table width limit and anything marked `dont_use` all land
//! there.
//!
//! [`Gate::pins`]: crate::synth::cells::Gate::pins
//!
//! # Which delay
//!
//! A Liberty cell has no single delay: it has a table per arc, per
//! output edge, sometimes per `when` condition, indexed by input
//! transition and output load. The mapper's cost function needs one
//! number per input pin. The one taken here is
//!
//! > the **worst of `cell_rise` and `cell_fall` over every delay arc
//! > from that pin to the output**, evaluated at a nominal input slew
//! > and a nominal output load.
//!
//! Worst rather than typical because the mapper is minimising the
//! critical path and a cell that is fast one way and slow the other is
//! as slow as its slow way; over every arc including `when`-conditional
//! ones because taking one condition's arc would be a guess about the
//! data. Constraint arcs (setup, hold, recovery, removal, pulse width)
//! and the three-state enable and disable arcs are not delays and are
//! not considered.
//!
//! The nominal operating point is derived from the library itself so
//! that two libraries are compared on the same footing (and so the
//! numbers do not depend on a magic constant in this file):
//!
//! - **load**: `4 x` the input capacitance of the library's smallest
//!   inverter, the usual FO4 point. Without an inverter, half of
//!   `default_max_capacitance`, then zero.
//! - **slew**: the output transition that same FO4 inverter produces,
//!   looked up in its own `rise_transition` / `fall_transition` at the
//!   first characterised slew point. Without one, a tenth of
//!   `default_max_transition`, then zero.
//!
//! Both are overridable ([`LibraryOptions::nominal_load`],
//! [`LibraryOptions::nominal_slew`]). Delays and areas keep the
//! library's own units — nanoseconds and square microns for every PDK
//! Reticle has been pointed at — because the mapper only ever compares
//! them with each other.
//!
//! # Sequential cells
//!
//! Flip-flops are not matched by function; they are matched by
//! *features*. [`StdCells::plan_flop`] takes what the IR inferred (clock
//! edge, clock enable, reset kind, polarity and value) and answers with
//! the library cell to use, which pin each signal goes to, and which
//! signals need an inverter on the way. When no cell has the polarity
//! asked for, an inverter is inserted rather than the flip-flop being
//! left unmapped: open PDKs ship active-low resets only
//! (`sky130_fd_sc_hd__dfrtp_1` has `RESET_B`, SG13G2's `sg13g2_dfrbp_1`
//! has `RESET_B` too), so refusing an active-high reset would refuse
//! most designs. A pin the cell has and the flip-flop does not use is
//! tied to its inactive level for the same reason.
//!
//! Two things are *not* solved by an inverter and are reported as
//! rewrites the caller must do first ([`FlopPlan::lower_enable`],
//! [`FlopPlan::lower_reset`]):
//!
//! - a **clock enable** no cell has becomes a feedback multiplexer on
//!   `d`;
//! - a **synchronous reset** always becomes a multiplexer on `d`, since
//!   Liberty hides a synchronous reset inside `next_state` and no open
//!   PDK ships one as a pin.
//!
//! [`crate::asic::flow`] does both before technology mapping, so the
//! multiplexers are mapped into gates with everything else.
//!
//! A clock of the wrong edge is *not* fixed with an inverter: inverting
//! a clock creates a second clock network with its own skew and
//! duty-cycle distortion, which is a physical-design decision and not a
//! mapper's to take. A design with a negative-edge flip-flop and a
//! library without one is reported.

use std::collections::BTreeMap;

use super::liberty::{BoolExpr, Cell, Library, LutTable, Pin, Register, RegisterKind, TimingType};
use crate::synth::aig::truth::TruthTable;
use crate::synth::cells::{Gate, GateLibrary};

/// Knobs for [`StdCells::from_library`].
#[derive(Clone, Debug)]
pub struct LibraryOptions {
    /// Largest number of input pins a cell may have to be offered to the
    /// mapper. The mapper's Boolean matching covers four; a larger value
    /// only adds cells it will never match.
    pub max_inputs: usize,
    /// Output load the representative delays are measured at, or `None`
    /// for the FO4 point derived from the library (see the module docs).
    pub nominal_load: Option<f64>,
    /// Input slew the representative delays are measured at, or `None`
    /// for the slew an FO4 inverter produces.
    pub nominal_slew: Option<f64>,
    /// Use cells marked `dont_use` anyway. Off by default: the library
    /// author marked them for a reason (characterisation holes, yield,
    /// or a cell kept only for compatibility).
    pub dont_use: bool,
}

impl Default for LibraryOptions {
    fn default() -> Self {
        LibraryOptions {
            max_inputs: 4,
            nominal_load: None,
            nominal_slew: None,
            dont_use: false,
        }
    }
}

/// Why a Liberty cell is not offered to the mapper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// `dont_use : true`.
    DontUse,
    /// The cell has an `ff` or `latch` group whose shape is not one the
    /// flip-flop matcher understands; the text says which part.
    Sequential(String),
    /// A `latch` group: level-sensitive storage is not inferred onto
    /// library cells.
    Latch,
    /// No output pin at all: a fill, tap, decap or antenna cell.
    NoOutput,
    /// More than one output pin (a full adder, a half adder, a cell with
    /// both `Q` and `QN`).
    MultiOutput,
    /// The output has a `three_state` condition.
    ThreeState,
    /// The output pin has no `function`, or one that did not parse.
    NoFunction,
    /// The function mentions names that are not input pins of the cell.
    UnknownPins(Vec<String>),
    /// The cell has no input pins (a constant generator or a tie cell).
    NoInputs,
    /// More inputs than [`LibraryOptions::max_inputs`].
    TooManyInputs(usize),
    /// The cell has no `area`, so the mapper cannot cost it.
    NoArea,
}

impl SkipReason {
    /// A one-line explanation, as the reports print it.
    pub fn describe(&self) -> String {
        match self {
            SkipReason::DontUse => "marked `dont_use`".to_string(),
            SkipReason::Sequential(why) => format!("sequential cell: {why}"),
            SkipReason::Latch => "a latch; only edge-triggered cells are mapped".to_string(),
            SkipReason::NoOutput => "no output pin (a physical-only cell)".to_string(),
            SkipReason::MultiOutput => "more than one output pin".to_string(),
            SkipReason::ThreeState => "a tri-state output".to_string(),
            SkipReason::NoFunction => "the output has no usable `function`".to_string(),
            SkipReason::UnknownPins(names) => {
                format!(
                    "the function mentions `{}`, not an input pin",
                    names.join("`, `")
                )
            }
            SkipReason::NoInputs => "no input pins".to_string(),
            SkipReason::TooManyInputs(n) => {
                format!("{n} inputs, beyond the truth-table width limit")
            }
            SkipReason::NoArea => "no `area`".to_string(),
        }
    }
}

/// A cell the mapper was not given, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedCell {
    /// The cell name.
    pub cell: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// The electrical numbers of a mapped cell that the truth table does not
/// carry, kept for the drive-strength pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Electrical {
    /// The cell name.
    pub cell: String,
    /// `max_capacitance` of the output pin, when the library states one.
    pub max_capacitance: Option<f64>,
    /// `capacitance` of each input pin, by pin name, in pin order.
    pub input_capacitance: Vec<(String, f64)>,
    /// `cell_footprint`, which is how a library groups drive variants.
    pub footprint: Option<String>,
}

impl Electrical {
    /// The largest input capacitance, which is what a driver of this
    /// cell sees in the worst case.
    pub fn input_load(&self) -> f64 {
        self.input_capacitance
            .iter()
            .map(|(_, c)| *c)
            .fold(0.0, f64::max)
    }

    /// The capacitance of one input pin.
    pub fn capacitance(&self, pin: &str) -> Option<f64> {
        self.input_capacitance
            .iter()
            .find(|(name, _)| name == pin)
            .map(|(_, c)| *c)
    }
}

/// How a signal reaches a flip-flop pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlopPinUse {
    /// Wire the signal straight to the pin.
    Direct(String),
    /// Wire the signal through an inverter: the cell's polarity is the
    /// opposite of the design's.
    Inverted(String),
    /// The cell has the pin but the flip-flop does not use it; tie it to
    /// the given constant, which is its inactive level.
    Tied(String, bool),
}

impl FlopPinUse {
    /// The pin name.
    pub fn pin(&self) -> &str {
        match self {
            FlopPinUse::Direct(p) | FlopPinUse::Inverted(p) | FlopPinUse::Tied(p, _) => p,
        }
    }

    /// True when this use costs an inverter.
    pub fn is_inverted(&self) -> bool {
        matches!(self, FlopPinUse::Inverted(_))
    }

    /// How it reads in a report.
    pub fn describe(&self) -> String {
        match self {
            FlopPinUse::Direct(p) => p.clone(),
            FlopPinUse::Inverted(p) => format!("!{p}"),
            FlopPinUse::Tied(p, v) => format!("{p}={}", u8::from(*v)),
        }
    }
}

/// A control pin of a sequential library cell and the level it is active
/// at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlopControl {
    /// The pin name.
    pub pin: String,
    /// True when the pin is active high.
    pub active_high: bool,
}

/// A sequential cell of the library, as the flip-flop matcher sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct FlopCell {
    /// The cell name.
    pub name: String,
    /// `area`.
    pub area: f64,
    /// True when the cell captures on the rising clock edge.
    pub clk_pos: bool,
    /// The clock pin.
    pub clock_pin: String,
    /// The data pin.
    pub data_pin: String,
    /// The `Q` pin.
    pub q_pin: String,
    /// The inverted output, when the cell has one.
    pub qn_pin: Option<String>,
    /// The clock enable, when the cell has one.
    pub enable: Option<FlopControl>,
    /// The asynchronous clear pin, when the cell has one.
    pub clear: Option<FlopControl>,
    /// The asynchronous preset pin, when the cell has one.
    pub preset: Option<FlopControl>,
}

/// What an inferred flip-flop needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlopRequest {
    /// True when it captures on the rising edge.
    pub clk_pos: bool,
    /// True when it has a clock enable.
    pub enable: bool,
    /// Its reset, if any.
    pub reset: Option<ResetRequest>,
}

/// The reset of an inferred flip-flop, for one bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetRequest {
    /// True for an asynchronous reset.
    pub asynchronous: bool,
    /// True when the reset signal is active high.
    pub active_high: bool,
    /// True when the bit is *set* (loads a one) rather than reset.
    pub sets: bool,
}

impl FlopRequest {
    /// A plain positive-edge flip-flop.
    pub fn new() -> FlopRequest {
        FlopRequest {
            clk_pos: true,
            enable: false,
            reset: None,
        }
    }

    /// How it reads in a diagnostic.
    pub fn describe(&self) -> String {
        let mut out = String::from(if self.clk_pos {
            "a positive-edge clock"
        } else {
            "a negative-edge clock"
        });
        if self.enable {
            out.push_str(", a clock enable");
        }
        match self.reset {
            None => out.push_str(" and no reset"),
            Some(r) => out.push_str(&format!(
                " and an {} active-{} {}",
                if r.asynchronous {
                    "asynchronous"
                } else {
                    "synchronous"
                },
                if r.active_high { "high" } else { "low" },
                if r.sets { "set" } else { "reset" }
            )),
        }
        out
    }
}

impl Default for FlopRequest {
    fn default() -> Self {
        FlopRequest::new()
    }
}

/// The library cell chosen for a flip-flop and how its pins are driven.
#[derive(Clone, Debug, PartialEq)]
pub struct FlopMatch {
    /// The cell name.
    pub cell: String,
    /// `area`.
    pub area: f64,
    /// The clock pin.
    pub clock_pin: String,
    /// The data pin.
    pub data_pin: String,
    /// The output pin.
    pub q_pin: String,
    /// How the clock enable is driven, when the cell has one.
    pub enable: Option<FlopPinUse>,
    /// How the reset (`clear`) pin is driven, when the cell has one.
    pub clear: Option<FlopPinUse>,
    /// How the set (`preset`) pin is driven, when the cell has one.
    pub preset: Option<FlopPinUse>,
}

impl FlopMatch {
    /// How many inverters the match needs.
    pub fn inverters(&self) -> usize {
        [&self.enable, &self.clear, &self.preset]
            .into_iter()
            .flatten()
            .filter(|u| u.is_inverted())
            .count()
    }

    /// A one-line description: the cell and its pin uses.
    pub fn describe(&self) -> String {
        let mut out = format!(
            "{} ({}, {} -> {}",
            self.cell, self.clock_pin, self.data_pin, self.q_pin
        );
        for use_ in [&self.enable, &self.clear, &self.preset]
            .into_iter()
            .flatten()
        {
            out.push_str(", ");
            out.push_str(&use_.describe());
        }
        out.push(')');
        out
    }
}

/// What has to happen for an inferred flip-flop to become library cells.
#[derive(Clone, Debug, PartialEq)]
pub struct FlopPlan {
    /// Move the clock enable into a feedback multiplexer on `d` first.
    pub lower_enable: bool,
    /// Move the reset into a multiplexer on `d` first (always so for a
    /// synchronous reset).
    pub lower_reset: bool,
    /// The cell to use once those rewrites are done.
    pub cell: Option<FlopMatch>,
    /// Why no cell matches, when `cell` is `None`.
    pub problem: Option<String>,
}

/// A Liberty library seen as a set of cells the mapper can use.
#[derive(Clone, Debug)]
pub struct StdCells {
    library: String,
    gates: GateLibrary,
    electrical: Vec<Electrical>,
    flops: Vec<FlopCell>,
    skipped: Vec<SkippedCell>,
    nominal_slew: f64,
    nominal_load: f64,
}

impl StdCells {
    /// Builds the mapper's view of `library`.
    ///
    /// Nothing is reported through diagnostics: every cell that did not
    /// make it is in [`StdCells::skipped`] with its reason, which the
    /// caller turns into whatever it wants (the flow puts the counts in
    /// its report and the details behind `--verbose`).
    pub fn from_library(library: &Library, options: &LibraryOptions) -> StdCells {
        let (nominal_slew, nominal_load) = nominal_point(library, options);
        let mut builder = GateLibrary::builder(library.name.clone());
        let mut electrical = Vec::new();
        let mut flops = Vec::new();
        let mut skipped = Vec::new();

        for cell in &library.cells {
            if cell.dont_use && !options.dont_use {
                skipped.push(SkippedCell {
                    cell: cell.name.clone(),
                    reason: SkipReason::DontUse,
                });
                continue;
            }
            if cell.is_sequential() {
                match flop_of(cell) {
                    Ok(flop) => {
                        electrical.push(Electrical {
                            cell: cell.name.clone(),
                            max_capacitance: cell.pin(&flop.q_pin).and_then(|p| p.max_capacitance),
                            input_capacitance: cell
                                .inputs()
                                .map(|p| (p.name.clone(), p.capacitance.unwrap_or(0.0)))
                                .collect(),
                            footprint: cell.cell_footprint.clone(),
                        });
                        flops.push(flop);
                    }
                    Err(reason) => skipped.push(SkippedCell {
                        cell: cell.name.clone(),
                        reason,
                    }),
                }
                continue;
            }
            match gate_of(cell, library, options, nominal_slew, nominal_load) {
                Ok((pins, output, table, area, delays, elec)) => {
                    let pin_refs: Vec<&str> = pins.iter().map(String::as_str).collect();
                    builder
                        .gate(cell.name.clone(), &pin_refs, table, area, &delays)
                        .output(output);
                    electrical.push(elec);
                }
                Err(reason) => skipped.push(SkippedCell {
                    cell: cell.name.clone(),
                    reason,
                }),
            }
        }

        flops.sort_by(|a, b| a.name.cmp(&b.name));
        StdCells {
            library: library.name.clone(),
            gates: builder.finish(),
            electrical,
            flops,
            skipped,
            nominal_slew,
            nominal_load,
        }
    }

    /// The library name.
    pub fn name(&self) -> &str {
        &self.library
    }

    /// The combinational cells, as the technology mapper wants them.
    pub fn gates(&self) -> &GateLibrary {
        &self.gates
    }

    /// The sequential cells, sorted by name.
    pub fn flops(&self) -> &[FlopCell] {
        &self.flops
    }

    /// The cells the mapper was not given, in library order.
    pub fn skipped(&self) -> &[SkippedCell] {
        &self.skipped
    }

    /// The input slew the representative delays were measured at.
    pub fn nominal_slew(&self) -> f64 {
        self.nominal_slew
    }

    /// The output load the representative delays were measured at.
    pub fn nominal_load(&self) -> f64 {
        self.nominal_load
    }

    /// The electrical numbers of a mapped cell.
    pub fn electrical(&self, cell: &str) -> Option<&Electrical> {
        self.electrical.iter().find(|e| e.cell == cell)
    }

    /// The name of the cheapest inverter, when the library has one.
    pub fn inverter(&self) -> Option<&str> {
        self.gates.inverter().map(|g| g.name.as_str())
    }

    /// The drive variants of a cell: every gate with the same pins and
    /// the same function, sorted by area, smallest first.
    ///
    /// This is what the drive-strength pass walks. Cells are grouped by
    /// what they *do*, not by their name or `cell_footprint`, so a
    /// library that names its variants `X1` / `X2` and one that names
    /// them `_1` / `_2` are handled the same way.
    pub fn drive_variants(&self, cell: &str) -> Vec<&str> {
        let Some(gate) = self.gates.gate(cell) else {
            return Vec::new();
        };
        let mut family: Vec<&Gate> = self
            .gates
            .gates()
            .iter()
            .filter(|g| g.pins == gate.pins && g.function == gate.function)
            .collect();
        family.sort_by(|a, b| a.area.total_cmp(&b.area).then_with(|| a.name.cmp(&b.name)));
        family.iter().map(|g| g.name.as_str()).collect()
    }

    /// The sequential cell for an inferred flip-flop, and the rewrites
    /// needed before it fits.
    ///
    /// The choice is deterministic: fewest inverters first, then fewest
    /// tied-off pins, then smallest area, then the cell name.
    pub fn plan_flop(&self, request: &FlopRequest) -> FlopPlan {
        let mut plan = FlopPlan {
            lower_enable: false,
            lower_reset: false,
            cell: None,
            problem: None,
        };
        let mut want = *request;
        // A synchronous reset is never a pin: Liberty writes it inside
        // `next_state`, and no open PDK ships one. Lower it first.
        if want.reset.is_some_and(|r| !r.asynchronous) {
            plan.lower_reset = true;
            want.reset = None;
        }
        if let Some(found) = self.best_flop(&want) {
            plan.cell = Some(found);
            return plan;
        }
        if want.enable {
            plan.lower_enable = true;
            want.enable = false;
            if let Some(found) = self.best_flop(&want) {
                plan.cell = Some(found);
                return plan;
            }
        }
        // Nothing fits. An asynchronous reset cannot be moved into the
        // data path (it acts between clock edges) and a clock edge is
        // not something an inverter may fix, so there is no rewrite left
        // to suggest: the flags are cleared and the caller is told.
        plan.lower_enable = false;
        plan.problem = Some(format!(
            "`{}` has no flip-flop with {}; it has {}",
            self.library,
            request.describe(),
            self.flop_summary()
        ));
        plan
    }

    /// A one-line list of the sequential cells, for a diagnostic.
    fn flop_summary(&self) -> String {
        if self.flops.is_empty() {
            return "none".to_string();
        }
        let mut items: Vec<String> = self
            .flops
            .iter()
            .map(|f| {
                let mut s = f.name.clone();
                s.push_str(" (");
                s.push_str(if f.clk_pos { "posedge" } else { "negedge" });
                if f.enable.is_some() {
                    s.push_str(", enable");
                }
                if f.clear.is_some() {
                    s.push_str(", clear");
                }
                if f.preset.is_some() {
                    s.push_str(", preset");
                }
                s.push(')');
                s
            })
            .collect();
        items.sort();
        items.join(", ")
    }

    /// The best cell for a request that needs no lowering, if any.
    fn best_flop(&self, want: &FlopRequest) -> Option<FlopMatch> {
        let mut best: Option<(u32, u32, f64, FlopMatch)> = None;
        for flop in &self.flops {
            let Some((inverters, ties, matched)) = fit_flop(flop, want) else {
                continue;
            };
            let key = (inverters, ties, flop.area);
            let better = match &best {
                None => true,
                Some((bi, bt, ba, bm)) => {
                    (key.0, key.1, key.2, matched.cell.as_str()) < (*bi, *bt, *ba, bm.cell.as_str())
                }
            };
            if better {
                best = Some((key.0, key.1, key.2, matched));
            }
        }
        best.map(|(_, _, _, m)| m)
    }

    /// A one-line-per-cell listing, for reports and golden files.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "library `{}`: {} mapper cells, {} flip-flops, {} skipped",
            self.library,
            self.gates.len(),
            self.flops.len(),
            self.skipped.len()
        );
        let _ = writeln!(
            out,
            "delays measured at slew {} and load {}",
            rounded(self.nominal_slew),
            rounded(self.nominal_load)
        );
        let mut gates: Vec<&Gate> = self.gates.gates().iter().collect();
        gates.sort_by(|a, b| a.name.cmp(&b.name));
        for gate in gates {
            let delays: Vec<String> = gate.delays.iter().map(|d| rounded(*d)).collect();
            let _ = writeln!(
                out,
                "  gate {} ({}) -> {} area={} tt={:#x} delays=[{}]",
                gate.name,
                gate.pins.join(", "),
                gate.output,
                super::fmt_num(gate.area),
                gate.function.as_u64(),
                delays.join(", ")
            );
        }
        for flop in &self.flops {
            let mut extra = String::new();
            for (label, control) in [
                ("enable", &flop.enable),
                ("clear", &flop.clear),
                ("preset", &flop.preset),
            ] {
                if let Some(c) = control {
                    let _ = write!(
                        extra,
                        " {label}={}{}",
                        if c.active_high { "" } else { "!" },
                        c.pin
                    );
                }
            }
            let _ = writeln!(
                out,
                "  flop {} ({}{}, {} -> {}{}) area={}",
                flop.name,
                if flop.clk_pos { "" } else { "!" },
                flop.clock_pin,
                flop.data_pin,
                flop.q_pin,
                extra,
                super::fmt_num(flop.area)
            );
        }
        for skip in &self.skipped {
            let _ = writeln!(out, "  skipped {}: {}", skip.cell, skip.reason.describe());
        }
        out
    }
}

/// A number as the listing prints it: five decimals, trimmed. The
/// library's own values carry at most four, and a delay interpolated
/// between two of them is a float whose exact decimal expansion is
/// noise.
fn rounded(v: f64) -> String {
    let mut s = format!("{v:.5}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { "0".to_string() } else { s }
}

/// Whether `flop` can implement `want`, and at what cost in inverters
/// and tied-off pins.
fn fit_flop(flop: &FlopCell, want: &FlopRequest) -> Option<(u32, u32, FlopMatch)> {
    if flop.clk_pos != want.clk_pos {
        return None;
    }
    let mut inverters = 0;
    let mut ties = 0;

    let enable = match (want.enable, &flop.enable) {
        (false, None) => None,
        (false, Some(c)) => {
            ties += 1;
            // Holding the enable at its active level keeps the flop
            // loading every cycle, which is what "no enable" means.
            Some(FlopPinUse::Tied(c.pin.clone(), c.active_high))
        }
        (true, None) => return None,
        (true, Some(c)) => Some(if c.active_high {
            FlopPinUse::Direct(c.pin.clone())
        } else {
            inverters += 1;
            FlopPinUse::Inverted(c.pin.clone())
        }),
    };

    // Which of the cell's two asynchronous pins the request wants.
    let (wanted, other) = match want.reset {
        Some(r) if r.sets => (flop.preset.as_ref(), flop.clear.as_ref()),
        Some(_) => (flop.clear.as_ref(), flop.preset.as_ref()),
        None => (None, None),
    };
    let mut use_of = |control: Option<&FlopControl>, active_high: bool| -> Option<FlopPinUse> {
        let c = control?;
        Some(if c.active_high == active_high {
            FlopPinUse::Direct(c.pin.clone())
        } else {
            inverters += 1;
            FlopPinUse::Inverted(c.pin.clone())
        })
    };
    let driven = match want.reset {
        Some(r) => {
            wanted?;
            use_of(wanted, r.active_high)
        }
        None => None,
    };
    // Every asynchronous pin the cell has and the request does not use
    // is tied inactive.
    let mut tie = |control: Option<&FlopControl>| -> Option<FlopPinUse> {
        let c = control?;
        ties += 1;
        Some(FlopPinUse::Tied(c.pin.clone(), !c.active_high))
    };
    let (clear, preset) = match want.reset {
        Some(r) if r.sets => (tie(other), driven),
        Some(_) => (driven, tie(other)),
        None => (tie(flop.clear.as_ref()), tie(flop.preset.as_ref())),
    };

    Some((
        inverters,
        ties,
        FlopMatch {
            cell: flop.name.clone(),
            area: flop.area,
            clock_pin: flop.clock_pin.clone(),
            data_pin: flop.data_pin.clone(),
            q_pin: flop.q_pin.clone(),
            enable,
            clear,
            preset,
        },
    ))
}

/// The nominal (slew, load) the representative delays are measured at.
fn nominal_point(library: &Library, options: &LibraryOptions) -> (f64, f64) {
    let inverter = smallest_inverter(library);
    let load = options.nominal_load.unwrap_or_else(|| {
        inverter
            .and_then(|c| c.inputs().next().and_then(|p| p.capacitance))
            .map(|c| 4.0 * c)
            .or_else(|| library.defaults.max_capacitance.map(|c| c / 2.0))
            .unwrap_or(0.0)
    });
    let slew = options.nominal_slew.unwrap_or_else(|| {
        inverter
            .and_then(|c| inverter_slew(library, c, load))
            .or_else(|| library.defaults.max_transition.map(|t| t / 10.0))
            .unwrap_or(0.0)
    });
    (slew, load)
}

/// The smallest cell of the library computing `!A` of one input.
fn smallest_inverter(library: &Library) -> Option<&Cell> {
    library
        .cells
        .iter()
        .filter(|c| {
            !c.dont_use && c.inputs().count() == 1 && c.registers.is_empty() && {
                let Some(out) = c.single_output() else {
                    return false;
                };
                let Some(f) = &out.function else {
                    return false;
                };
                out.three_state.is_none()
                    && matches!(f, BoolExpr::Not(inner) if matches!(**inner, BoolExpr::Pin(_)))
            }
        })
        .min_by(|a, b| {
            a.area
                .unwrap_or(f64::INFINITY)
                .total_cmp(&b.area.unwrap_or(f64::INFINITY))
                .then_with(|| a.name.cmp(&b.name))
        })
}

/// The output transition an inverter driving `load` produces, looked up
/// at the first characterised slew point.
fn inverter_slew(library: &Library, cell: &Cell, load: f64) -> Option<f64> {
    let out = cell.single_output()?;
    let input = cell.inputs().next()?;
    let mut worst: Option<f64> = None;
    for arc in arcs_from(out, &input.name) {
        let seed = arc
            .rise_transition
            .as_ref()
            .or(arc.fall_transition.as_ref())
            .and_then(|t| t.index_1.first().copied())
            .unwrap_or(0.0);
        for table in [&arc.rise_transition, &arc.fall_transition]
            .into_iter()
            .flatten()
        {
            if let Some(v) = lookup(library, table, seed, load) {
                worst = Some(worst.map_or(v, |w: f64| w.max(v)));
            }
        }
    }
    worst
}

/// The delay arcs of `pin` whose `related_pin` names `from`.
///
/// `related_pin` may list several pins separated by whitespace, which is
/// what a library does for a symmetric gate, so membership rather than
/// equality decides.
fn arcs_from<'a>(
    pin: &'a Pin,
    from: &'a str,
) -> impl Iterator<Item = &'a super::liberty::TimingArc> + 'a {
    pin.timing.iter().filter(move |arc| {
        arc.related_pin.split_whitespace().any(|p| p == from)
            && !arc.timing_type.is_constraint()
            && !matches!(
                arc.timing_type,
                TimingType::ThreeStateEnable | TimingType::ThreeStateDisable
            )
    })
}

/// A table lookup with the transition on one axis and the load on the
/// other, deciding which is which from the table's template the way
/// [`crate::timing::delay`] does.
fn lookup(library: &Library, table: &LutTable, slew: f64, load: f64) -> Option<f64> {
    let axes = library.template(&table.template).map(|t| {
        let axis = |i: usize| t.variables.get(i).map_or(2u8, |v| classify(v));
        (axis(0), axis(1))
    });
    let (x, y) = match axes {
        Some((0, _)) | None => (slew, load),
        Some((1, _)) => (load, slew),
        Some((_, 0)) => (load, slew),
        Some((_, _)) => (slew, load),
    };
    table.lookup(x, y)
}

/// `0` for a transition axis, `1` for a load axis, `2` for anything else.
fn classify(variable: &str) -> u8 {
    if variable.contains("transition") || variable.contains("slew") {
        0
    } else if variable.contains("capacitance") || variable.contains("fanout") {
        1
    } else {
        2
    }
}

/// The mapper's view of one combinational cell.
type GateParts = (Vec<String>, String, TruthTable, f64, Vec<f64>, Electrical);

fn gate_of(
    cell: &Cell,
    library: &Library,
    options: &LibraryOptions,
    slew: f64,
    load: f64,
) -> Result<GateParts, SkipReason> {
    let outputs = cell.outputs().count();
    if outputs == 0 {
        return Err(SkipReason::NoOutput);
    }
    if outputs > 1 {
        return Err(SkipReason::MultiOutput);
    }
    let out = cell.single_output().ok_or(SkipReason::MultiOutput)?;
    if out.three_state.is_some() {
        return Err(SkipReason::ThreeState);
    }
    let function = out.function.as_ref().ok_or(SkipReason::NoFunction)?;
    let inputs: Vec<&Pin> = cell.inputs().collect();
    if inputs.is_empty() {
        return Err(SkipReason::NoInputs);
    }
    if inputs.len() > options.max_inputs {
        return Err(SkipReason::TooManyInputs(inputs.len()));
    }
    let names: Vec<&str> = inputs.iter().map(|p| p.name.as_str()).collect();
    let unknown: Vec<String> = function
        .variables()
        .into_iter()
        .filter(|v| !names.contains(&v.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(SkipReason::UnknownPins(unknown));
    }
    let area = cell.area.ok_or(SkipReason::NoArea)?;
    let bits = function
        .truth_table(&names)
        .ok_or(SkipReason::TooManyInputs(inputs.len()))?;
    let table = TruthTable::from_u64(inputs.len(), bits);
    let delays: Vec<f64> = names
        .iter()
        .map(|name| pin_delay(library, out, name, slew, load))
        .collect();
    let electrical = Electrical {
        cell: cell.name.clone(),
        max_capacitance: out.max_capacitance,
        input_capacitance: inputs
            .iter()
            .map(|p| (p.name.clone(), p.capacitance.unwrap_or(0.0)))
            .collect(),
        footprint: cell.cell_footprint.clone(),
    };
    Ok((
        names.iter().map(|s| (*s).to_owned()).collect(),
        out.name.clone(),
        table,
        area,
        delays,
        electrical,
    ))
}

/// The representative delay from one input pin to the output; see the
/// module docs for which arc this is.
fn pin_delay(library: &Library, out: &Pin, pin: &str, slew: f64, load: f64) -> f64 {
    let mut worst = 0.0f64;
    let mut seen = false;
    for arc in arcs_from(out, pin) {
        for table in [&arc.cell_rise, &arc.cell_fall].into_iter().flatten() {
            if let Some(v) = lookup(library, table, slew, load) {
                worst = if seen { worst.max(v) } else { v };
                seen = true;
            }
        }
        for intrinsic in [arc.intrinsic_rise, arc.intrinsic_fall]
            .into_iter()
            .flatten()
        {
            worst = if seen {
                worst.max(intrinsic)
            } else {
                intrinsic
            };
            seen = true;
        }
    }
    worst.max(0.0)
}

/// The flip-flop view of a sequential cell.
fn flop_of(cell: &Cell) -> Result<FlopCell, SkipReason> {
    if cell.registers.iter().any(|r| r.kind == RegisterKind::Latch) {
        return Err(SkipReason::Latch);
    }
    let register = cell
        .register()
        .ok_or_else(|| SkipReason::Sequential("more than one `ff` group".to_string()))?;
    if register.bits.is_some() {
        return Err(SkipReason::Sequential(
            "an `ff_bank`; only one-bit flip-flops are mapped".to_string(),
        ));
    }
    let area = cell.area.ok_or(SkipReason::NoArea)?;
    let clock = register
        .clock
        .as_ref()
        .ok_or_else(|| SkipReason::Sequential("no `clocked_on`".to_string()))?;
    let (clock_pin, clk_pos) = literal(clock).ok_or_else(|| {
        SkipReason::Sequential(format!("`clocked_on : \"{clock}\"` is not a pin"))
    })?;
    let inputs: Vec<&str> = cell.inputs().map(|p| p.name.as_str()).collect();
    if !inputs.contains(&clock_pin.as_str()) {
        return Err(SkipReason::Sequential(format!(
            "`clocked_on` names `{clock_pin}`, not an input pin"
        )));
    }

    let next = register
        .data
        .as_ref()
        .ok_or_else(|| SkipReason::Sequential("no `next_state`".to_string()))?;
    let (data_pin, enable) = data_and_enable(next, register, &inputs)?;

    let control = |expr: &Option<BoolExpr>,
                   what: &str|
     -> Result<Option<FlopControl>, SkipReason> {
        let Some(expr) = expr else { return Ok(None) };
        let (pin, active_high) = literal(expr)
            .ok_or_else(|| SkipReason::Sequential(format!("`{what} : \"{expr}\"` is not a pin")))?;
        if !inputs.contains(&pin.as_str()) {
            return Err(SkipReason::Sequential(format!(
                "`{what}` names `{pin}`, not an input pin"
            )));
        }
        Ok(Some(FlopControl { pin, active_high }))
    };
    let clear = control(&register.clear, "clear")?;
    let preset = control(&register.preset, "preset")?;

    let state = &register.outputs.0;
    let inverted = &register.outputs.1;
    let mut q_pin = None;
    let mut qn_pin = None;
    for pin in cell.outputs() {
        match &pin.function {
            Some(BoolExpr::Pin(p)) if p == state => q_pin = Some(pin.name.clone()),
            Some(BoolExpr::Pin(p)) if p == inverted => qn_pin = Some(pin.name.clone()),
            Some(BoolExpr::Not(inner)) if matches!(&**inner, BoolExpr::Pin(p) if p == state) => {
                qn_pin = Some(pin.name.clone());
            }
            _ => {}
        }
    }
    let q_pin = q_pin.ok_or_else(|| {
        SkipReason::Sequential(format!("no output pin with `function : \"{state}\"`"))
    })?;

    Ok(FlopCell {
        name: cell.name.clone(),
        area,
        clk_pos,
        clock_pin,
        data_pin,
        q_pin,
        qn_pin,
        enable,
        clear,
        preset,
    })
}

/// The data pin and the clock enable of a `next_state` expression.
///
/// Two shapes are recognised: a bare pin (`next_state : "D"`), and the
/// enable shape every library writes a clock-enable flop with,
/// `next_state : "(D & E) + (IQ & !E)"` — recognised by evaluating it
/// rather than by matching the tree, so the many ways of writing the
/// same function all work.
fn data_and_enable(
    next: &BoolExpr,
    register: &Register,
    inputs: &[&str],
) -> Result<(String, Option<FlopControl>), SkipReason> {
    let state = register.outputs.0.as_str();
    let vars = next.variables();
    if let BoolExpr::Pin(d) = next {
        if inputs.contains(&d.as_str()) {
            return Ok((d.clone(), None));
        }
        return Err(SkipReason::Sequential(format!(
            "`next_state` names `{d}`, not an input pin"
        )));
    }
    if vars.len() == 3 && vars.iter().any(|v| v == state) {
        let pins: Vec<&String> = vars.iter().filter(|v| v.as_str() != state).collect();
        for (data, enable) in [(pins[0], pins[1]), (pins[1], pins[0])] {
            if !inputs.contains(&data.as_str()) || !inputs.contains(&enable.as_str()) {
                continue;
            }
            let order = [data.as_str(), enable.as_str(), state];
            let Some(actual) = next.truth_table(&order) else {
                continue;
            };
            for active_high in [true, false] {
                if actual == enable_table(active_high) {
                    return Ok((
                        data.clone(),
                        Some(FlopControl {
                            pin: enable.clone(),
                            active_high,
                        }),
                    ));
                }
            }
        }
    }
    Err(SkipReason::Sequential(format!(
        "`next_state : \"{next}\"` is neither a data pin nor an enable multiplexer"
    )))
}

/// The truth table of `E ? D : IQ` over the variable order
/// `[D, E, IQ]`, with `E` active high or low.
fn enable_table(active_high: bool) -> u64 {
    let mut table = 0u64;
    for row in 0..8u64 {
        let (d, e, q) = (row & 1 == 1, row & 2 == 2, row & 4 == 4);
        let take = if active_high { e } else { !e };
        if if take { d } else { q } {
            table |= 1 << row;
        }
    }
    table
}

/// A pin, possibly negated: `A` gives `(A, true)`, `!A` gives
/// `(A, false)`.
fn literal(expr: &BoolExpr) -> Option<(String, bool)> {
    match expr {
        BoolExpr::Pin(p) => Some((p.clone(), true)),
        BoolExpr::Not(inner) => match &**inner {
            BoolExpr::Pin(p) => Some((p.clone(), false)),
            _ => None,
        },
        _ => None,
    }
}

/// How many cells were skipped for each reason, sorted by reason, for a
/// report that does not want the full list.
pub fn skip_summary(skipped: &[SkippedCell]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for skip in skipped {
        *counts.entry(skip.reason.describe()).or_default() += 1;
    }
    counts.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::source::SourceMap;

    /// A tiny library covering the cases the conversion has to get
    /// right: an inverter, a NAND2 whose function must become the right
    /// table, and one cell per skip reason.
    const TINY: &str = r#"
library (tiny) {
  time_unit : "1ns";
  capacitive_load_unit (1.0, pf);
  default_max_transition : 1.0;
  lu_table_template (d2) {
    variable_1 : input_net_transition;
    variable_2 : total_output_net_capacitance;
    index_1 ("0.01, 0.1");
    index_2 ("0.001, 0.01");
  }
  cell (INV) {
    area : 1.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (Y) {
      direction : output;
      function : "!A";
      max_capacitance : 0.05;
      timing () {
        related_pin : "A";
        cell_rise (d2) { values ("0.010, 0.020", "0.030, 0.040"); }
        cell_fall (d2) { values ("0.008, 0.018", "0.028, 0.038"); }
        rise_transition (d2) { values ("0.012, 0.022", "0.032, 0.042"); }
        fall_transition (d2) { values ("0.011, 0.021", "0.031, 0.041"); }
      }
    }
  }
  cell (NAND2) {
    area : 2.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (B) { direction : input; capacitance : 0.003; }
    pin (Y) {
      direction : output;
      function : "!(A B)";
      timing () {
        related_pin : "A B";
        cell_rise (d2) { values ("0.020, 0.030", "0.040, 0.050"); }
        cell_fall (d2) { values ("0.025, 0.035", "0.045, 0.055"); }
      }
    }
  }
  cell (FILL) { area : 0.5; }
  cell (HA) {
    area : 4.0;
    pin (A) { direction : input; }
    pin (B) { direction : input; }
    pin (S) { direction : output; function : "A^B"; }
    pin (CO) { direction : output; function : "A B"; }
  }
  cell (TBUF) {
    area : 3.0;
    pin (A) { direction : input; }
    pin (OE) { direction : input; }
    pin (Y) { direction : output; function : "A"; three_state : "!OE"; }
  }
  cell (SLOW) {
    area : 2.0;
    dont_use : true;
    pin (A) { direction : input; }
    pin (Y) { direction : output; function : "A"; }
  }
  cell (MUX4) {
    area : 9.0;
    pin (A) { direction : input; }
    pin (B) { direction : input; }
    pin (C) { direction : input; }
    pin (D) { direction : input; }
    pin (S0) { direction : input; }
    pin (S1) { direction : input; }
    pin (Y) { direction : output; function : "(A !S0 !S1) + (B S0 !S1) + (C !S0 S1) + (D S0 S1)"; }
  }
  cell (NOAREA) {
    pin (A) { direction : input; }
    pin (Y) { direction : output; function : "A"; }
  }
  cell (LATCH) {
    area : 5.0;
    latch (IQ, IQ_N) { data_in : "D"; enable : "G"; }
    pin (D) { direction : input; }
    pin (G) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
}
"#;

    /// Flip-flops covering every reset and enable shape the matcher has
    /// to deal with, written the way an open PDK writes them.
    const FLOPS: &str = r#"
library (flops) {
  time_unit : "1ns";
  cell (DFF) {
    area : 10.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
    pin (QN) { direction : output; function : "IQ_N"; }
  }
  cell (DFFN) {
    area : 11.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "!CLK"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
  cell (DFFR) {
    area : 12.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; clear : "!RN"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (RN) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
  cell (DFFS) {
    area : 12.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; preset : "!SN"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (SN) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
  cell (DFFE) {
    area : 14.0;
    ff (IQ, IQ_N) { next_state : "(D E) + (IQ !E)"; clocked_on : "CLK"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (E) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
}
"#;

    fn parse(text: &str) -> Library {
        let mut sources = SourceMap::new();
        let file = sources.add("t.lib", text.to_string()).unwrap();
        let mut diags = Diagnostics::new();
        let library = Library::parse(text, file, &mut diags).expect("a library");
        assert!(!diags.has_errors(), "{}", diags.render(&sources));
        library
    }

    fn tiny() -> StdCells {
        StdCells::from_library(&parse(TINY), &LibraryOptions::default())
    }

    #[test]
    fn a_function_becomes_the_right_truth_table() {
        let cells = tiny();
        let nand2 = cells.gates().gate("NAND2").expect("NAND2 is mappable");
        assert_eq!(nand2.pins, ["A", "B"]);
        assert_eq!(nand2.output, "Y");
        // !(A & B) over [A, B]: 1 for every pattern but 0b11.
        assert_eq!(nand2.function.as_u64(), 0b0111);
        assert_eq!(nand2.area, 2.0);
        let inv = cells.gates().gate("INV").expect("INV is mappable");
        assert_eq!(inv.function.as_u64(), 0b01);
        assert_eq!(cells.inverter(), Some("INV"));
        // The mapper can match through it.
        let v = |i| TruthTable::var(2, i);
        let (gate, m) = cells
            .gates()
            .match_function(&v(0).and(&v(1)).not())
            .unwrap();
        assert_eq!(gate.name, "NAND2");
        assert_eq!(m.inverters(), 0);
    }

    #[test]
    fn the_representative_delay_is_the_worst_edge_at_the_nominal_point() {
        let cells = tiny();
        // FO4 of the inverter: 4 x 0.002 pF.
        assert!((cells.nominal_load() - 0.008).abs() < 1e-12);
        // The slew is the inverter's own output transition there.
        assert!(cells.nominal_slew() > 0.0);
        let inv = cells.gates().gate("INV").unwrap();
        let slew = cells.nominal_slew();
        let load = cells.nominal_load();
        let rise = 0.010
            + (0.030 - 0.010) * (slew - 0.01) / 0.09
            + (0.020 - 0.010) * (load - 0.001) / 0.009;
        // `cell_rise` is the worse of the two tables here.
        assert!(
            (inv.delay(0) - rise).abs() < 1e-9,
            "{} vs {rise}",
            inv.delay(0)
        );
        // A `related_pin` naming two pins gives both pins an arc.
        let nand2 = cells.gates().gate("NAND2").unwrap();
        assert!(nand2.delay(0) > 0.0);
        assert!(nand2.delay(1) > 0.0);
    }

    #[test]
    fn unusable_cells_are_reported_one_by_one() {
        let cells = tiny();
        let reason = |name: &str| {
            cells
                .skipped()
                .iter()
                .find(|s| s.cell == name)
                .unwrap_or_else(|| panic!("{name} should have been skipped"))
                .reason
                .clone()
        };
        assert_eq!(reason("FILL"), SkipReason::NoOutput);
        assert_eq!(reason("HA"), SkipReason::MultiOutput);
        assert_eq!(reason("TBUF"), SkipReason::ThreeState);
        assert_eq!(reason("SLOW"), SkipReason::DontUse);
        assert_eq!(reason("MUX4"), SkipReason::TooManyInputs(6));
        assert_eq!(reason("NOAREA"), SkipReason::NoArea);
        assert_eq!(reason("LATCH"), SkipReason::Latch);
        assert_eq!(cells.gates().len(), 2);
        assert!(cells.flops().is_empty());
        // Every reason renders.
        for skip in cells.skipped() {
            assert!(!skip.reason.describe().is_empty());
        }
        let summary = skip_summary(cells.skipped());
        assert_eq!(summary.iter().map(|(_, n)| n).sum::<usize>(), 7);
        // `dont_use` cells can be asked for.
        let with = StdCells::from_library(
            &parse(TINY),
            &LibraryOptions {
                dont_use: true,
                ..LibraryOptions::default()
            },
        );
        assert!(with.gates().gate("SLOW").is_some());
        // A wider limit lets the six-input multiplexer through.
        let wide = StdCells::from_library(
            &parse(TINY),
            &LibraryOptions {
                max_inputs: 6,
                ..LibraryOptions::default()
            },
        );
        assert!(wide.gates().gate("MUX4").is_some());
        assert!(!wide.to_text().is_empty());
    }

    #[test]
    fn a_function_over_unknown_names_is_refused() {
        let text = r#"
library (odd) {
  cell (WEIRD) {
    area : 1.0;
    pin (A) { direction : input; }
    pin (Y) { direction : output; function : "A & IQ"; }
  }
}
"#;
        let cells = StdCells::from_library(&parse(text), &LibraryOptions::default());
        assert_eq!(
            cells.skipped()[0].reason,
            SkipReason::UnknownPins(vec!["IQ".to_string()])
        );
    }

    fn flops() -> StdCells {
        StdCells::from_library(&parse(FLOPS), &LibraryOptions::default())
    }

    fn request(clk_pos: bool, enable: bool, reset: Option<ResetRequest>) -> FlopRequest {
        FlopRequest {
            clk_pos,
            enable,
            reset,
        }
    }

    fn reset(asynchronous: bool, active_high: bool, sets: bool) -> Option<ResetRequest> {
        Some(ResetRequest {
            asynchronous,
            active_high,
            sets,
        })
    }

    #[test]
    fn flip_flops_are_read_out_of_their_ff_groups() {
        let cells = flops();
        assert_eq!(cells.flops().len(), 5);
        let dffe = cells.flops().iter().find(|f| f.name == "DFFE").unwrap();
        assert_eq!(dffe.data_pin, "D");
        assert_eq!(
            dffe.enable,
            Some(FlopControl {
                pin: "E".to_string(),
                active_high: true
            })
        );
        let dff = cells.flops().iter().find(|f| f.name == "DFF").unwrap();
        assert_eq!(dff.qn_pin.as_deref(), Some("QN"));
        assert!(dff.clk_pos);
        let dffn = cells.flops().iter().find(|f| f.name == "DFFN").unwrap();
        assert!(!dffn.clk_pos);
        let dffr = cells.flops().iter().find(|f| f.name == "DFFR").unwrap();
        assert_eq!(
            dffr.clear,
            Some(FlopControl {
                pin: "RN".to_string(),
                active_high: false
            })
        );
    }

    #[test]
    fn every_reset_and_enable_combination_is_planned() {
        let cells = flops();
        // Plain.
        let plan = cells.plan_flop(&request(true, false, None));
        let cell = plan.cell.expect("a plain flop");
        assert_eq!(cell.cell, "DFF");
        assert_eq!(cell.inverters(), 0);
        assert!(!plan.lower_enable && !plan.lower_reset);

        // Negative edge: its own cell, never an inverted clock.
        let plan = cells.plan_flop(&request(false, false, None));
        assert_eq!(plan.cell.expect("a negedge flop").cell, "DFFN");

        // Enable: the enable cell, straight through.
        let plan = cells.plan_flop(&request(true, true, None));
        let cell = plan.cell.expect("an enable flop");
        assert_eq!(cell.cell, "DFFE");
        assert_eq!(cell.enable, Some(FlopPinUse::Direct("E".to_string())));
        assert!(!plan.lower_enable);

        // Asynchronous active-low reset: DFFR, no inverter.
        let plan = cells.plan_flop(&request(true, false, reset(true, false, false)));
        let cell = plan.cell.expect("a reset flop");
        assert_eq!(cell.cell, "DFFR");
        assert_eq!(cell.clear, Some(FlopPinUse::Direct("RN".to_string())));
        assert_eq!(cell.inverters(), 0);

        // Asynchronous active-high reset: the same cell with an inverter,
        // which is the gap the FPGA side leaves open.
        let plan = cells.plan_flop(&request(true, false, reset(true, true, false)));
        let cell = plan.cell.expect("a reset flop with an inverter");
        assert_eq!(cell.cell, "DFFR");
        assert_eq!(cell.clear, Some(FlopPinUse::Inverted("RN".to_string())));
        assert_eq!(cell.inverters(), 1);

        // A set (reset value 1) goes to the preset cell.
        let plan = cells.plan_flop(&request(true, false, reset(true, false, true)));
        let cell = plan.cell.expect("a set flop");
        assert_eq!(cell.cell, "DFFS");
        assert_eq!(cell.preset, Some(FlopPinUse::Direct("SN".to_string())));
        let plan = cells.plan_flop(&request(true, false, reset(true, true, true)));
        assert_eq!(
            plan.cell.expect("a set flop").preset,
            Some(FlopPinUse::Inverted("SN".to_string()))
        );

        // A synchronous reset is always lowered into the data path.
        let plan = cells.plan_flop(&request(true, false, reset(false, true, false)));
        assert!(plan.lower_reset);
        assert!(!plan.lower_enable);
        assert_eq!(plan.cell.expect("a plain flop after lowering").cell, "DFF");

        // Enable plus asynchronous reset: no cell has both, so the enable
        // is lowered and the reset keeps its pin.
        let plan = cells.plan_flop(&request(true, true, reset(true, false, false)));
        assert!(plan.lower_enable);
        assert!(!plan.lower_reset);
        let cell = plan.cell.expect("a reset flop");
        assert_eq!(cell.cell, "DFFR");
        assert_eq!(cell.enable, None);

        // Enable plus synchronous reset: both go into the data path.
        let plan = cells.plan_flop(&request(true, true, reset(false, true, false)));
        assert!(plan.lower_reset);
        assert!(!plan.lower_enable);
        assert_eq!(plan.cell.expect("the enable flop").cell, "DFFE");

        // Negative edge with a reset: the library has no such cell.
        let plan = cells.plan_flop(&request(false, false, reset(true, false, false)));
        assert!(plan.cell.is_none());
        let problem = plan.problem.expect("a diagnostic");
        assert!(problem.contains("negative-edge"), "{problem}");
        assert!(problem.contains("DFFR"), "{problem}");
    }

    #[test]
    fn a_pin_the_flip_flop_does_not_use_is_tied_off() {
        // A library whose only flip-flop has an active-high reset: a
        // design with no reset at all still maps, with the pin tied low.
        let text = r#"
library (only_reset) {
  cell (DFFR) {
    area : 12.0;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; clear : "R"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (R) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
}
"#;
        let cells = StdCells::from_library(&parse(text), &LibraryOptions::default());
        let plan = cells.plan_flop(&request(true, false, None));
        let cell = plan.cell.expect("the reset flop with its pin tied");
        assert_eq!(cell.clear, Some(FlopPinUse::Tied("R".to_string(), false)));
        assert_eq!(cell.inverters(), 0);
        assert!(cell.describe().contains("R=0"));
    }

    #[test]
    fn a_sequential_cell_the_matcher_cannot_read_is_reported() {
        let text = r#"
library (odd) {
  cell (SCANFF) {
    area : 12.0;
    ff (IQ, IQ_N) { next_state : "(D & !SE) + (SI & SE) + IQ"; clocked_on : "CLK"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) { direction : input; }
    pin (SI) { direction : input; }
    pin (SE) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; }
  }
  cell (BANK) {
    area : 20.0;
    ff_bank (IQ, IQ_N, 2) { next_state : "D"; clocked_on : "CLK"; }
    pin (CLK) { direction : input; clock : true; }
  }
}
"#;
        let cells = StdCells::from_library(&parse(text), &LibraryOptions::default());
        assert!(cells.flops().is_empty());
        assert_eq!(cells.skipped().len(), 2);
        for skip in cells.skipped() {
            assert!(matches!(skip.reason, SkipReason::Sequential(_)), "{skip:?}");
        }
        let plan = cells.plan_flop(&FlopRequest::new());
        assert!(plan.cell.is_none());
        assert!(plan.problem.unwrap().contains("it has none"));
    }

    #[test]
    fn drive_variants_are_grouped_by_what_they_do() {
        let text = r#"
library (sized) {
  cell (INV_X1) {
    area : 1.0;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (Y) { direction : output; function : "!A"; max_capacitance : 0.05; }
  }
  cell (INV_X4) {
    area : 3.0;
    pin (A) { direction : input; capacitance : 0.008; }
    pin (Y) { direction : output; function : "!A"; max_capacitance : 0.2; }
  }
  cell (BUF_X1) {
    area : 1.5;
    pin (A) { direction : input; capacitance : 0.002; }
    pin (Y) { direction : output; function : "A"; max_capacitance : 0.05; }
  }
}
"#;
        let cells = StdCells::from_library(&parse(text), &LibraryOptions::default());
        assert_eq!(cells.drive_variants("INV_X1"), ["INV_X1", "INV_X4"]);
        assert_eq!(cells.drive_variants("BUF_X1"), ["BUF_X1"]);
        assert!(cells.drive_variants("nope").is_empty());
        let e = cells.electrical("INV_X4").expect("electrical data");
        assert_eq!(e.max_capacitance, Some(0.2));
        assert!((e.input_load() - 0.008).abs() < 1e-12);
        assert!(cells.electrical("nope").is_none());
    }
}
