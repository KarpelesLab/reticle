//! Liberty (`.lib`) standard-cell library reader.
//!
//! Liberty is the format every ASIC library ships its logical view in: for
//! each cell its area, pin directions and capacitances, boolean function,
//! sequential behaviour (`ff` / `latch` groups), and the timing and power
//! tables the timing analyser interpolates. Reticle reads it in two layers:
//!
//! 1. [`parse_groups`] reads the generic
//!    `group (args) { attribute : value ; }` grammar into a [`Group`]
//!    tree. Everything in the file survives this step; unknown attributes
//!    are data, not errors.
//! 2. [`Library::from_group`] extracts the typed view ([`Library`],
//!    [`Cell`], [`Pin`], [`TimingArc`], [`LutTable`], ...) that the mapper
//!    and the timing analyser consume. Attributes the typed view does not
//!    know remain reachable through the retained [`Library::group`].
//!
//! The reader has been shaped after the open PDK libraries (SKY130,
//! IHP SG13G2, Nangate45 / FreePDK45, GF180MCU): bus pins with a `type`
//! group, `bundle`s, multi-line `values` tables, `leakage_power` groups
//! with `when` conditions, and the `\` line continuations in all of them.
//!
//! Errors are reported as [`Diagnostic`] values with spans. A malformed
//! function or table is a warning that leaves the affected field empty, so
//! one odd cell never blocks the rest of the library.

mod func;
mod syntax;
mod table;

use std::collections::BTreeMap;
use std::fmt;

pub use func::{BoolExpr, FuncError};
pub use syntax::{
    Attribute, Group, IncludeResolver, Value, parse_groups, parse_groups_with_includes,
};
pub use table::{LutTable, LutTemplate};

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

use super::{float_to_int, fmt_num};

/// A unit declaration such as `time_unit : "1ns"`.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// The unit as written (`1ns`, `1pf`, `1V`).
    pub text: String,
    /// Multiplier to the SI base unit (seconds, farads, volts, amperes,
    /// ohms, watts): `1ns` is `1e-9`.
    pub scale: f64,
}

impl Unit {
    /// Parses `1ns`, `0.01pf`, `1kohm`, ... into a scale; unknown unit
    /// names keep a scale of the numeric part.
    pub fn parse(text: &str) -> Unit {
        let text = text.trim();
        let at = text
            .char_indices()
            .find(|(_, c)| c.is_ascii_alphabetic())
            .map_or(text.len(), |(i, _)| i);
        let number: f64 = text[..at].trim().parse().unwrap_or(1.0);
        let unit = text[at..].trim().to_ascii_lowercase();
        let bases = ["ohm", "s", "f", "v", "a", "w"];
        let prefix = bases
            .iter()
            .find_map(|b| unit.strip_suffix(b))
            .unwrap_or(unit.as_str());
        let mult = match prefix {
            "" => 1.0,
            "f" => 1e-15,
            "p" => 1e-12,
            "n" => 1e-9,
            "u" => 1e-6,
            "m" => 1e-3,
            "k" => 1e3,
            "meg" => 1e6,
            _ => 1.0,
        };
        // `M` (mega) is only distinguishable from `m` (milli) by case.
        let mult = if text[at..].starts_with('M') && prefix == "m" {
            1e6
        } else {
            mult
        };
        Unit {
            text: text.to_string(),
            scale: number * mult,
        }
    }
}

/// The library's units.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Units {
    /// `time_unit`.
    pub time: Option<Unit>,
    /// `capacitive_load_unit`.
    pub capacitance: Option<Unit>,
    /// `voltage_unit`.
    pub voltage: Option<Unit>,
    /// `current_unit`.
    pub current: Option<Unit>,
    /// `pulling_resistance_unit`.
    pub resistance: Option<Unit>,
    /// `leakage_power_unit`.
    pub leakage_power: Option<Unit>,
}

/// The library-level `default_*` attributes the mapper and analyser use.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Defaults {
    /// `default_cell_leakage_power`.
    pub cell_leakage_power: Option<f64>,
    /// `default_fanout_load`.
    pub fanout_load: Option<f64>,
    /// `default_inout_pin_cap`.
    pub inout_pin_cap: Option<f64>,
    /// `default_input_pin_cap`.
    pub input_pin_cap: Option<f64>,
    /// `default_output_pin_cap`.
    pub output_pin_cap: Option<f64>,
    /// `default_max_capacitance`.
    pub max_capacitance: Option<f64>,
    /// `default_max_fanout`.
    pub max_fanout: Option<f64>,
    /// `default_max_transition`.
    pub max_transition: Option<f64>,
    /// `default_operating_conditions`.
    pub operating_conditions: Option<String>,
}

/// An `operating_conditions` group.
#[derive(Clone, Debug, PartialEq)]
pub struct OperatingConditions {
    /// The corner name (`tt_025C_1v80`).
    pub name: String,
    /// `process`.
    pub process: Option<f64>,
    /// `temperature`.
    pub temperature: Option<f64>,
    /// `voltage`.
    pub voltage: Option<f64>,
    /// `tree_type`.
    pub tree_type: Option<String>,
}

/// A `type` group describing a bus.
#[derive(Clone, Debug, PartialEq)]
pub struct BusType {
    /// The type name referenced by `bus_type`.
    pub name: String,
    /// `bit_width`.
    pub bit_width: u32,
    /// `bit_from`: the index of the first (most significant) bit.
    pub bit_from: i64,
    /// `bit_to`: the index of the last bit.
    pub bit_to: i64,
}

impl BusType {
    /// The bit indices from `bit_from` to `bit_to`, inclusive.
    pub fn indices(&self) -> Vec<i64> {
        if self.bit_from >= self.bit_to {
            (self.bit_to..=self.bit_from).rev().collect()
        } else {
            (self.bit_from..=self.bit_to).collect()
        }
    }
}

/// Pin direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// `input`.
    Input,
    /// `output`.
    Output,
    /// `inout`.
    Inout,
    /// `internal`.
    Internal,
}

impl Direction {
    /// The Liberty keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
            Direction::Inout => "inout",
            Direction::Internal => "internal",
        }
    }

    fn parse(s: &str) -> Option<Direction> {
        Some(match s {
            "input" => Direction::Input,
            "output" => Direction::Output,
            "inout" => Direction::Inout,
            "internal" => Direction::Internal,
            _ => return None,
        })
    }
}

/// The `timing_type` of an arc. Delay arcs are `Combinational` and the
/// edge types; the rest are constraints checked against a related clock.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TimingType {
    /// `combinational` (the default).
    Combinational,
    /// `combinational_rise`.
    CombinationalRise,
    /// `combinational_fall`.
    CombinationalFall,
    /// `three_state_enable`.
    ThreeStateEnable,
    /// `three_state_disable`.
    ThreeStateDisable,
    /// `rising_edge`: clock-to-output on a rising clock.
    RisingEdge,
    /// `falling_edge`.
    FallingEdge,
    /// `preset`.
    Preset,
    /// `clear`.
    Clear,
    /// `setup_rising`.
    SetupRising,
    /// `setup_falling`.
    SetupFalling,
    /// `hold_rising`.
    HoldRising,
    /// `hold_falling`.
    HoldFalling,
    /// `recovery_rising`.
    RecoveryRising,
    /// `recovery_falling`.
    RecoveryFalling,
    /// `removal_rising`.
    RemovalRising,
    /// `removal_falling`.
    RemovalFalling,
    /// `min_pulse_width`.
    MinPulseWidth,
    /// `minimum_period`.
    MinimumPeriod,
    /// `skew_rising`.
    SkewRising,
    /// `skew_falling`.
    SkewFalling,
    /// `nochange_high_high` and the other `nochange_*` types.
    NoChange(String),
    /// Any other type, kept as written.
    Other(String),
}

impl TimingType {
    /// Parses the attribute text.
    pub fn parse(s: &str) -> TimingType {
        match s {
            "combinational" => TimingType::Combinational,
            "combinational_rise" => TimingType::CombinationalRise,
            "combinational_fall" => TimingType::CombinationalFall,
            "three_state_enable" => TimingType::ThreeStateEnable,
            "three_state_disable" => TimingType::ThreeStateDisable,
            "rising_edge" => TimingType::RisingEdge,
            "falling_edge" => TimingType::FallingEdge,
            "preset" => TimingType::Preset,
            "clear" => TimingType::Clear,
            "setup_rising" => TimingType::SetupRising,
            "setup_falling" => TimingType::SetupFalling,
            "hold_rising" => TimingType::HoldRising,
            "hold_falling" => TimingType::HoldFalling,
            "recovery_rising" => TimingType::RecoveryRising,
            "recovery_falling" => TimingType::RecoveryFalling,
            "removal_rising" => TimingType::RemovalRising,
            "removal_falling" => TimingType::RemovalFalling,
            "min_pulse_width" => TimingType::MinPulseWidth,
            "minimum_period" => TimingType::MinimumPeriod,
            "skew_rising" => TimingType::SkewRising,
            "skew_falling" => TimingType::SkewFalling,
            other if other.starts_with("nochange_") => TimingType::NoChange(other.to_string()),
            other => TimingType::Other(other.to_string()),
        }
    }

    /// The attribute text.
    pub fn as_str(&self) -> &str {
        match self {
            TimingType::Combinational => "combinational",
            TimingType::CombinationalRise => "combinational_rise",
            TimingType::CombinationalFall => "combinational_fall",
            TimingType::ThreeStateEnable => "three_state_enable",
            TimingType::ThreeStateDisable => "three_state_disable",
            TimingType::RisingEdge => "rising_edge",
            TimingType::FallingEdge => "falling_edge",
            TimingType::Preset => "preset",
            TimingType::Clear => "clear",
            TimingType::SetupRising => "setup_rising",
            TimingType::SetupFalling => "setup_falling",
            TimingType::HoldRising => "hold_rising",
            TimingType::HoldFalling => "hold_falling",
            TimingType::RecoveryRising => "recovery_rising",
            TimingType::RecoveryFalling => "recovery_falling",
            TimingType::RemovalRising => "removal_rising",
            TimingType::RemovalFalling => "removal_falling",
            TimingType::MinPulseWidth => "min_pulse_width",
            TimingType::MinimumPeriod => "minimum_period",
            TimingType::SkewRising => "skew_rising",
            TimingType::SkewFalling => "skew_falling",
            TimingType::NoChange(s) | TimingType::Other(s) => s,
        }
    }

    /// True for setup, hold, recovery, removal, pulse-width, period, skew
    /// and no-change checks: arcs whose tables are `rise_constraint` /
    /// `fall_constraint` rather than delays.
    pub fn is_constraint(&self) -> bool {
        matches!(
            self,
            TimingType::SetupRising
                | TimingType::SetupFalling
                | TimingType::HoldRising
                | TimingType::HoldFalling
                | TimingType::RecoveryRising
                | TimingType::RecoveryFalling
                | TimingType::RemovalRising
                | TimingType::RemovalFalling
                | TimingType::MinPulseWidth
                | TimingType::MinimumPeriod
                | TimingType::SkewRising
                | TimingType::SkewFalling
                | TimingType::NoChange(_)
        )
    }

    /// True for `setup_rising` / `setup_falling`.
    pub fn is_setup(&self) -> bool {
        matches!(self, TimingType::SetupRising | TimingType::SetupFalling)
    }

    /// True for `hold_rising` / `hold_falling`.
    pub fn is_hold(&self) -> bool {
        matches!(self, TimingType::HoldRising | TimingType::HoldFalling)
    }

    /// True for `rising_edge` / `falling_edge`: a clock-to-output delay.
    pub fn is_edge(&self) -> bool {
        matches!(self, TimingType::RisingEdge | TimingType::FallingEdge)
    }
}

impl fmt::Display for TimingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `timing_sense` of a delay arc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimingSense {
    /// A rising input causes a rising output.
    PositiveUnate,
    /// A rising input causes a falling output.
    NegativeUnate,
    /// Either.
    NonUnate,
}

impl TimingSense {
    /// The attribute text.
    pub fn as_str(self) -> &'static str {
        match self {
            TimingSense::PositiveUnate => "positive_unate",
            TimingSense::NegativeUnate => "negative_unate",
            TimingSense::NonUnate => "non_unate",
        }
    }

    fn parse(s: &str) -> Option<TimingSense> {
        Some(match s {
            "positive_unate" => TimingSense::PositiveUnate,
            "negative_unate" => TimingSense::NegativeUnate,
            "non_unate" => TimingSense::NonUnate,
            _ => return None,
        })
    }
}

/// One `timing` group of a pin: the delay or constraint from
/// `related_pin` to this pin.
#[derive(Clone, Debug, PartialEq)]
pub struct TimingArc {
    /// `related_pin` as written (may name several pins separated by
    /// spaces, or a bus).
    pub related_pin: String,
    /// `timing_type`; `Combinational` when absent.
    pub timing_type: TimingType,
    /// `timing_sense`.
    pub timing_sense: Option<TimingSense>,
    /// `when`: the state-dependent condition this arc applies under.
    pub when: Option<BoolExpr>,
    /// `sdf_cond`, kept as text.
    pub sdf_cond: Option<String>,
    /// `cell_rise`: delay to a rising output.
    pub cell_rise: Option<LutTable>,
    /// `cell_fall`: delay to a falling output.
    pub cell_fall: Option<LutTable>,
    /// `rise_transition`: output rise time.
    pub rise_transition: Option<LutTable>,
    /// `fall_transition`: output fall time.
    pub fall_transition: Option<LutTable>,
    /// `rise_constraint`: the setup / hold / recovery / removal value for
    /// a rising data pin.
    pub rise_constraint: Option<LutTable>,
    /// `fall_constraint`: the same for a falling data pin.
    pub fall_constraint: Option<LutTable>,
    /// Any other table groups (`rise_propagation`, `retaining_rise`, ...)
    /// by name.
    pub other_tables: Vec<(String, LutTable)>,
    /// `intrinsic_rise` from the older linear delay model.
    pub intrinsic_rise: Option<f64>,
    /// `intrinsic_fall` from the older linear delay model.
    pub intrinsic_fall: Option<f64>,
    /// Where the `timing` group was written.
    pub span: Span,
}

impl TimingArc {
    /// The larger of the rise and fall delays at the given input slew and
    /// output load, when this is a delay arc with tables.
    pub fn max_delay(&self, slew: f64, load: f64) -> Option<f64> {
        let r = self.cell_rise.as_ref().and_then(|t| t.lookup(slew, load));
        let f = self.cell_fall.as_ref().and_then(|t| t.lookup(slew, load));
        match (r, f) {
            (Some(r), Some(f)) => Some(r.max(f)),
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        }
    }
}

/// An `internal_power` group of a pin.
#[derive(Clone, Debug, PartialEq)]
pub struct InternalPower {
    /// `related_pin`.
    pub related_pin: Option<String>,
    /// `related_pg_pin`.
    pub related_pg_pin: Option<String>,
    /// `when`.
    pub when: Option<BoolExpr>,
    /// `rise_power`.
    pub rise_power: Option<LutTable>,
    /// `fall_power`.
    pub fall_power: Option<LutTable>,
    /// `power` (for pins without a rise/fall split).
    pub power: Option<LutTable>,
}

/// A `leakage_power` group of a cell.
#[derive(Clone, Debug, PartialEq)]
pub struct LeakagePower {
    /// `when`: the input state this value applies to; `None` for the
    /// unconditional value.
    pub when: Option<BoolExpr>,
    /// `value`.
    pub value: f64,
    /// `related_pg_pin`.
    pub related_pg_pin: Option<String>,
}

/// A power or ground pin (`pg_pin` group).
#[derive(Clone, Debug, PartialEq)]
pub struct PgPin {
    /// The pin name.
    pub name: String,
    /// `pg_type`: `primary_power`, `primary_ground`, `nwell`, `pwell`, ...
    pub pg_type: Option<String>,
    /// `voltage_name`.
    pub voltage_name: Option<String>,
}

/// Whether a register is edge- or level-sensitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegisterKind {
    /// An `ff` / `ff_bank` group.
    Ff,
    /// A `latch` / `latch_bank` group.
    Latch,
}

/// An `ff`, `ff_bank`, `latch` or `latch_bank` group: the sequential
/// element of a cell, whose state variables the output functions refer
/// to.
#[derive(Clone, Debug, PartialEq)]
pub struct Register {
    /// Edge or level sensitive.
    pub kind: RegisterKind,
    /// The two state variable names from the group arguments (`IQ`,
    /// `IQ_N`).
    pub outputs: (String, String),
    /// Bank width for `ff_bank` / `latch_bank`.
    pub bits: Option<u32>,
    /// `next_state` (flip-flops) or `data_in` (latches).
    pub data: Option<BoolExpr>,
    /// `clocked_on` (flip-flops) or `enable` (latches).
    pub clock: Option<BoolExpr>,
    /// `clocked_on_also` / `enable_also`.
    pub clock_also: Option<BoolExpr>,
    /// `clear`: asynchronous reset condition.
    pub clear: Option<BoolExpr>,
    /// `preset`: asynchronous set condition.
    pub preset: Option<BoolExpr>,
    /// `clear_preset_var1`: the first output when clear and preset are
    /// both active (`L`, `H`, `N`, `T`, `X`).
    pub clear_preset_var1: Option<String>,
    /// `clear_preset_var2`.
    pub clear_preset_var2: Option<String>,
}

/// One pin of a cell. Bus and bundle members are expanded into one
/// [`Pin`] per bit with `bus` and `bit` set.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    /// The pin name (`A`, `D[3]`).
    pub name: String,
    /// `direction`.
    pub direction: Option<Direction>,
    /// The bus or bundle this pin was expanded from.
    pub bus: Option<String>,
    /// The bit index inside the bus, or the member position in a bundle.
    pub bit: Option<i64>,
    /// `capacitance`.
    pub capacitance: Option<f64>,
    /// `rise_capacitance`.
    pub rise_capacitance: Option<f64>,
    /// `fall_capacitance`.
    pub fall_capacitance: Option<f64>,
    /// `max_capacitance`.
    pub max_capacitance: Option<f64>,
    /// `min_capacitance`.
    pub min_capacitance: Option<f64>,
    /// `max_transition`.
    pub max_transition: Option<f64>,
    /// `max_fanout`.
    pub max_fanout: Option<f64>,
    /// `fanout_load`.
    pub fanout_load: Option<f64>,
    /// `drive_strength`.
    pub drive_strength: Option<f64>,
    /// `function`, parsed.
    pub function: Option<BoolExpr>,
    /// `function` as written, kept for messages and for expressions the
    /// parser rejected.
    pub function_text: Option<String>,
    /// `three_state`: the condition under which the output is high-Z.
    pub three_state: Option<BoolExpr>,
    /// `clock : true`.
    pub clock: bool,
    /// `clock_gate_clock_pin`, `clock_gate_enable_pin`, ... flags.
    pub clock_gate_pin: Option<String>,
    /// `related_power_pin`.
    pub related_power_pin: Option<String>,
    /// `related_ground_pin`.
    pub related_ground_pin: Option<String>,
    /// `timing` groups.
    pub timing: Vec<TimingArc>,
    /// `internal_power` groups.
    pub internal_power: Vec<InternalPower>,
    /// Where the pin group was written.
    pub span: Span,
}

impl Pin {
    fn new(name: &str, span: Span) -> Pin {
        Pin {
            name: name.to_string(),
            direction: None,
            bus: None,
            bit: None,
            capacitance: None,
            rise_capacitance: None,
            fall_capacitance: None,
            max_capacitance: None,
            min_capacitance: None,
            max_transition: None,
            max_fanout: None,
            fanout_load: None,
            drive_strength: None,
            function: None,
            function_text: None,
            three_state: None,
            clock: false,
            clock_gate_pin: None,
            related_power_pin: None,
            related_ground_pin: None,
            timing: Vec::new(),
            internal_power: Vec::new(),
            span,
        }
    }

    /// True for `input` and `inout` pins.
    pub fn is_input(&self) -> bool {
        matches!(self.direction, Some(Direction::Input | Direction::Inout))
    }

    /// True for `output` and `inout` pins.
    pub fn is_output(&self) -> bool {
        matches!(self.direction, Some(Direction::Output | Direction::Inout))
    }

    /// The timing arcs from a given related pin.
    pub fn arcs_from<'a>(&'a self, related: &'a str) -> impl Iterator<Item = &'a TimingArc> + 'a {
        self.timing.iter().filter(move |t| t.related_pin == related)
    }
}

/// One cell of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// The cell name.
    pub name: String,
    /// `area`.
    pub area: Option<f64>,
    /// `dont_use : true`.
    pub dont_use: bool,
    /// `dont_touch : true`.
    pub dont_touch: bool,
    /// `cell_footprint`.
    pub cell_footprint: Option<String>,
    /// `cell_leakage_power`.
    pub cell_leakage_power: Option<f64>,
    /// `leakage_power` groups.
    pub leakage_power: Vec<LeakagePower>,
    /// `ff`, `ff_bank`, `latch` and `latch_bank` groups.
    pub registers: Vec<Register>,
    /// Signal pins, bus and bundle members expanded.
    pub pins: Vec<Pin>,
    /// Power and ground pins.
    pub pg_pins: Vec<PgPin>,
    /// `clock_gating_integrated_cell`, when the cell is a clock gate.
    pub clock_gating_integrated_cell: Option<String>,
    /// Where the cell group was written.
    pub span: Span,
}

impl Cell {
    /// True when the cell contains a flip-flop or latch.
    pub fn is_sequential(&self) -> bool {
        !self.registers.is_empty()
    }

    /// True when the cell is purely combinational: no register, no
    /// three-state output, and every output has a function.
    pub fn is_combinational(&self) -> bool {
        !self.is_sequential()
            && self
                .pins
                .iter()
                .filter(|p| p.is_output())
                .all(|p| p.function.is_some() && p.three_state.is_none())
            && self.pins.iter().any(|p| p.is_output())
    }

    /// The pin with this name.
    pub fn pin(&self, name: &str) -> Option<&Pin> {
        self.pins.iter().find(|p| p.name == name)
    }

    /// Input pins in declaration order.
    pub fn inputs(&self) -> impl Iterator<Item = &Pin> {
        self.pins.iter().filter(|p| p.is_input())
    }

    /// Output pins in declaration order.
    pub fn outputs(&self) -> impl Iterator<Item = &Pin> {
        self.pins.iter().filter(|p| p.is_output())
    }

    /// The single output pin of a simple gate (exactly one output).
    pub fn single_output(&self) -> Option<&Pin> {
        let mut outs = self.outputs();
        let first = outs.next()?;
        outs.next().is_none().then_some(first)
    }

    /// The register of a single-register cell.
    pub fn register(&self) -> Option<&Register> {
        (self.registers.len() == 1).then(|| &self.registers[0])
    }

    /// The largest input capacitance among the input pins.
    pub fn max_input_capacitance(&self) -> Option<f64> {
        self.inputs().filter_map(|p| p.capacitance).reduce(f64::max)
    }
}

/// A typed Liberty library.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    /// The library name.
    pub name: String,
    /// Units.
    pub units: Units,
    /// Library-level defaults.
    pub defaults: Defaults,
    /// `delay_model` (`table_lookup` for every modern library).
    pub delay_model: Option<String>,
    /// `nom_process`.
    pub nom_process: Option<f64>,
    /// `nom_temperature`.
    pub nom_temperature: Option<f64>,
    /// `nom_voltage`.
    pub nom_voltage: Option<f64>,
    /// `operating_conditions` groups.
    pub operating_conditions: Vec<OperatingConditions>,
    /// `lu_table_template` and `power_lut_template` groups.
    pub templates: Vec<LutTemplate>,
    /// `type` groups describing buses.
    pub bus_types: Vec<BusType>,
    /// The cells, in file order.
    pub cells: Vec<Cell>,
    /// The generic tree the typed view was built from, for attributes
    /// the view does not model.
    pub group: Group,
    cell_index: BTreeMap<String, usize>,
}

impl Library {
    /// Parses Liberty text into a library. Syntax errors are reported to
    /// `diags`; `None` is returned only when no `library` group could be
    /// found at all. `include_file` directives are reported as errors;
    /// use [`Library::parse_with_includes`] to resolve them.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Option<Library> {
        Self::parse_with_includes(text, file, diags, None)
    }

    /// Like [`Library::parse`], resolving `include_file(name)` through the
    /// given callback, which returns the included text and the
    /// [`SourceId`] it was registered under.
    pub fn parse_with_includes<'r>(
        text: &str,
        file: SourceId,
        diags: &mut Diagnostics,
        resolver: Option<&'r mut IncludeResolver<'r>>,
    ) -> Option<Library> {
        let groups = parse_groups_with_includes(text, file, diags, resolver);
        let library = groups.into_iter().find(|g| g.name == "library");
        match library {
            Some(g) => Some(Library::from_group(g, diags)),
            None => {
                let end = u32::try_from(text.len()).unwrap_or(u32::MAX);
                diags.push(
                    Diagnostic::error("no `library` group found").with_span(Span::new(
                        file,
                        0,
                        end.min(1),
                    )),
                );
                None
            }
        }
    }

    /// Builds the typed view from a parsed `library` group. Problems in
    /// individual attributes (unparsable functions, non-numeric areas)
    /// are warnings that leave the field unset.
    pub fn from_group(group: Group, diags: &mut Diagnostics) -> Library {
        let mut units = Units::default();
        for (key, slot) in [
            ("time_unit", &mut units.time),
            ("voltage_unit", &mut units.voltage),
            ("current_unit", &mut units.current),
            ("pulling_resistance_unit", &mut units.resistance),
            ("leakage_power_unit", &mut units.leakage_power),
        ] {
            if let Some(text) = group.attr_str(key) {
                *slot = Some(Unit::parse(text));
            }
        }
        if let Some(v) = group.attr("capacitive_load_unit") {
            let items = v.as_list();
            let number = items.first().and_then(|v| v.as_f64()).unwrap_or(1.0);
            let name = items.get(1).and_then(|v| v.as_str()).unwrap_or("pf");
            units.capacitance = Some(Unit::parse(&format!("{}{name}", fmt_num(number))));
        }

        let defaults = Defaults {
            cell_leakage_power: group.attr_f64("default_cell_leakage_power"),
            fanout_load: group.attr_f64("default_fanout_load"),
            inout_pin_cap: group.attr_f64("default_inout_pin_cap"),
            input_pin_cap: group.attr_f64("default_input_pin_cap"),
            output_pin_cap: group.attr_f64("default_output_pin_cap"),
            max_capacitance: group.attr_f64("default_max_capacitance"),
            max_fanout: group.attr_f64("default_max_fanout"),
            max_transition: group.attr_f64("default_max_transition"),
            operating_conditions: group
                .attr_str("default_operating_conditions")
                .map(str::to_string),
        };

        let operating_conditions = group
            .groups_named("operating_conditions")
            .map(|g| OperatingConditions {
                name: g.arg_name().unwrap_or_default().to_string(),
                process: g.attr_f64("process"),
                temperature: g.attr_f64("temperature"),
                voltage: g.attr_f64("voltage"),
                tree_type: g.attr_str("tree_type").map(str::to_string),
            })
            .collect();

        let templates: Vec<LutTemplate> = group
            .groups
            .iter()
            .filter(|g| g.name.ends_with("_template"))
            .map(LutTemplate::from_group)
            .collect();

        let bus_types: Vec<BusType> = group
            .groups_named("type")
            .filter_map(|g| {
                let bit_from = g.attr_f64("bit_from");
                let bit_to = g.attr_f64("bit_to");
                let width = g.attr_f64("bit_width");
                let (from, to) = match (bit_from, bit_to, width) {
                    (Some(f), Some(t), _) => (f, t),
                    (None, None, Some(w)) => (w - 1.0, 0.0),
                    _ => return None,
                };
                let (from, to) = (float_to_int(from)?, float_to_int(to)?);
                let width = match width.and_then(float_to_int) {
                    Some(w) => w,
                    None => (from - to).abs() + 1,
                };
                Some(BusType {
                    name: g.arg_name().unwrap_or_default().to_string(),
                    bit_width: u32::try_from(width).ok()?,
                    bit_from: from,
                    bit_to: to,
                })
            })
            .collect();

        let mut ctx = Context {
            templates: &templates,
            bus_types: &bus_types,
            diags,
        };
        let cells: Vec<Cell> = group.groups_named("cell").map(|g| ctx.cell(g)).collect();
        let mut cell_index = BTreeMap::new();
        for (i, cell) in cells.iter().enumerate() {
            cell_index.entry(cell.name.clone()).or_insert(i);
        }

        Library {
            name: group.arg_name().unwrap_or_default().to_string(),
            units,
            defaults,
            delay_model: group.attr_str("delay_model").map(str::to_string),
            nom_process: group.attr_f64("nom_process"),
            nom_temperature: group.attr_f64("nom_temperature"),
            nom_voltage: group.attr_f64("nom_voltage"),
            operating_conditions,
            templates,
            bus_types,
            cells,
            cell_index,
            group,
        }
    }

    /// The cell with this name.
    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cell_index.get(name).map(|&i| &self.cells[i])
    }

    /// The pin `pin` of cell `cell`.
    pub fn pin(&self, cell: &str, pin: &str) -> Option<&Pin> {
        self.cell(cell)?.pin(pin)
    }

    /// The template with this name.
    pub fn template(&self, name: &str) -> Option<&LutTemplate> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// The cells usable for mapping: not `dont_use`, combinational with
    /// a single output.
    pub fn mappable_gates(&self) -> impl Iterator<Item = &Cell> {
        self.cells
            .iter()
            .filter(|c| !c.dont_use && c.is_combinational() && c.single_output().is_some())
    }

    /// A one-line-per-cell listing of the library sorted by cell name,
    /// with area, sequential/dont_use flags and output functions, for
    /// debugging and golden tests.
    pub fn to_gate_summary(&self) -> String {
        let mut out = format!("library {}: {} cells", self.name, self.cells.len());
        let unit = |u: &Option<Unit>| u.as_ref().map_or("?".to_string(), |u| u.text.clone());
        out.push_str(&format!(
            " (time {}, capacitance {}, voltage {})\n",
            unit(&self.units.time),
            unit(&self.units.capacitance),
            unit(&self.units.voltage)
        ));
        let mut cells: Vec<&Cell> = self.cells.iter().collect();
        cells.sort_by(|a, b| a.name.cmp(&b.name));
        for cell in cells {
            out.push_str(&cell.name);
            out.push_str("  area=");
            out.push_str(&cell.area.map_or("?".to_string(), fmt_num));
            if cell.is_sequential() {
                out.push_str("  seq");
            }
            if cell.dont_use {
                out.push_str("  dont_use");
            }
            for pin in cell.outputs() {
                out.push_str("  ");
                out.push_str(&pin.name);
                out.push('=');
                match (&pin.function, &pin.function_text) {
                    (Some(f), _) => out.push_str(&f.to_string()),
                    (None, Some(t)) => out.push_str(&format!("?{t:?}")),
                    (None, None) => out.push('?'),
                }
                if let Some(z) = &pin.three_state {
                    out.push_str(&format!(" z={z}"));
                }
            }
            for reg in &cell.registers {
                let kind = match reg.kind {
                    RegisterKind::Ff => "ff",
                    RegisterKind::Latch => "latch",
                };
                out.push_str(&format!("  {kind}({},{})", reg.outputs.0, reg.outputs.1));
                if let Some(d) = &reg.data {
                    out.push_str(&format!(" data={d}"));
                }
                if let Some(c) = &reg.clock {
                    out.push_str(&format!(" clock={c}"));
                }
                if let Some(c) = &reg.clear {
                    out.push_str(&format!(" clear={c}"));
                }
                if let Some(p) = &reg.preset {
                    out.push_str(&format!(" preset={p}"));
                }
            }
            out.push('\n');
        }
        out
    }
}

/// State shared while building the typed view of one library.
struct Context<'a> {
    templates: &'a [LutTemplate],
    bus_types: &'a [BusType],
    diags: &'a mut Diagnostics,
}

impl Context<'_> {
    fn warn(&mut self, span: Span, message: String) {
        self.diags
            .push(Diagnostic::warning(message).with_span(span));
    }

    /// Parses a boolean-expression attribute, warning on failure.
    fn expr(&mut self, g: &Group, key: &str) -> Option<BoolExpr> {
        let attr = g.attribute(key)?;
        let text = attr.value.as_str()?;
        match BoolExpr::parse(text) {
            Ok(e) => Some(e),
            Err(err) => {
                self.warn(
                    attr.span,
                    format!("cannot parse `{key}` expression `{text}`: {err}"),
                );
                None
            }
        }
    }

    fn number(&mut self, g: &Group, key: &str) -> Option<f64> {
        let attr = g.attribute(key)?;
        match attr.value.as_f64() {
            Some(v) => Some(v),
            None => {
                self.warn(
                    attr.span,
                    format!("`{key}` should be a number, found `{}`", attr.value),
                );
                None
            }
        }
    }

    fn table(&mut self, g: &Group) -> LutTable {
        let t = LutTable::from_group(g, self.templates);
        if !t.is_consistent() {
            self.warn(
                g.span,
                format!(
                    "table `{}` has {} values for {}x{} indices",
                    g.name,
                    t.values.len(),
                    t.index_1.len().max(1),
                    t.index_2.len().max(1)
                ),
            );
        }
        t
    }

    fn cell(&mut self, g: &Group) -> Cell {
        let mut cell = Cell {
            name: g.arg_name().unwrap_or_default().to_string(),
            area: self.number(g, "area"),
            dont_use: g.attr_bool("dont_use").unwrap_or(false),
            dont_touch: g.attr_bool("dont_touch").unwrap_or(false),
            cell_footprint: g.attr_str("cell_footprint").map(str::to_string),
            cell_leakage_power: self.number(g, "cell_leakage_power"),
            leakage_power: Vec::new(),
            registers: Vec::new(),
            pins: Vec::new(),
            pg_pins: Vec::new(),
            clock_gating_integrated_cell: g
                .attr_str("clock_gating_integrated_cell")
                .map(str::to_string),
            span: g.span,
        };
        for sub in &g.groups {
            match sub.name.as_str() {
                "pin" => {
                    let names = expand_pin_name(sub.arg_name().unwrap_or_default());
                    for name in names {
                        let mut pin = Pin::new(&name, sub.span);
                        self.apply_pin(&mut pin, sub);
                        cell.pins.push(pin);
                    }
                }
                "bus" => self.bus(&mut cell, sub),
                "bundle" => self.bundle(&mut cell, sub),
                "pg_pin" => cell.pg_pins.push(PgPin {
                    name: sub.arg_name().unwrap_or_default().to_string(),
                    pg_type: sub.attr_str("pg_type").map(str::to_string),
                    voltage_name: sub.attr_str("voltage_name").map(str::to_string),
                }),
                "ff" | "ff_bank" => {
                    let reg = self.register(sub, RegisterKind::Ff);
                    cell.registers.push(reg);
                }
                "latch" | "latch_bank" => {
                    let reg = self.register(sub, RegisterKind::Latch);
                    cell.registers.push(reg);
                }
                "leakage_power" => {
                    let when = self.expr(sub, "when");
                    if let Some(value) = self.number(sub, "value") {
                        cell.leakage_power.push(LeakagePower {
                            when,
                            value,
                            related_pg_pin: sub.attr_str("related_pg_pin").map(str::to_string),
                        });
                    }
                }
                _ => {}
            }
        }
        cell
    }

    fn register(&mut self, g: &Group, kind: RegisterKind) -> Register {
        let (data_key, clock_key, also_key) = match kind {
            RegisterKind::Ff => ("next_state", "clocked_on", "clocked_on_also"),
            RegisterKind::Latch => ("data_in", "enable", "enable_also"),
        };
        let arg = |i: usize| {
            g.args
                .get(i)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Register {
            kind,
            outputs: (arg(0), arg(1)),
            bits: g
                .args
                .get(2)
                .and_then(Value::as_f64)
                .and_then(float_to_int)
                .and_then(|v| u32::try_from(v).ok()),
            data: self.expr(g, data_key),
            clock: self.expr(g, clock_key),
            clock_also: self.expr(g, also_key),
            clear: self.expr(g, "clear"),
            preset: self.expr(g, "preset"),
            clear_preset_var1: g.attr_str("clear_preset_var1").map(str::to_string),
            clear_preset_var2: g.attr_str("clear_preset_var2").map(str::to_string),
        }
    }

    /// Overlays the attributes and groups of `g` on `pin`: attributes
    /// present in `g` replace the pin's, timing and power arcs are
    /// appended. Nested `pin` groups (bus members) are skipped.
    fn apply_pin(&mut self, pin: &mut Pin, g: &Group) {
        if let Some(d) = g.attr_str("direction") {
            match Direction::parse(d) {
                Some(dir) => pin.direction = Some(dir),
                None => {
                    let span = g.attribute("direction").map_or(g.span, |a| a.span);
                    self.warn(span, format!("unknown pin direction `{d}`"));
                }
            }
        }
        macro_rules! num {
            ($($key:literal => $field:ident),* $(,)?) => {
                $( if g.attr($key).is_some() { pin.$field = self.number(g, $key); } )*
            };
        }
        num! {
            "capacitance" => capacitance,
            "rise_capacitance" => rise_capacitance,
            "fall_capacitance" => fall_capacitance,
            "max_capacitance" => max_capacitance,
            "min_capacitance" => min_capacitance,
            "max_transition" => max_transition,
            "max_fanout" => max_fanout,
            "fanout_load" => fanout_load,
            "drive_strength" => drive_strength,
        }
        if let Some(text) = g.attr_str("function") {
            pin.function_text = Some(text.to_string());
            pin.function = self.expr(g, "function");
        }
        if g.attr("three_state").is_some() {
            pin.three_state = self.expr(g, "three_state");
        }
        if let Some(c) = g.attr_bool("clock") {
            pin.clock = c;
        }
        for key in [
            "clock_gate_clock_pin",
            "clock_gate_enable_pin",
            "clock_gate_test_pin",
            "clock_gate_out_pin",
        ] {
            if g.attr_bool(key) == Some(true) {
                pin.clock_gate_pin = Some(key.to_string());
            }
        }
        if let Some(p) = g.attr_str("related_power_pin") {
            pin.related_power_pin = Some(p.to_string());
        }
        if let Some(p) = g.attr_str("related_ground_pin") {
            pin.related_ground_pin = Some(p.to_string());
        }
        for sub in &g.groups {
            match sub.name.as_str() {
                "timing" => {
                    let arc = self.timing(sub);
                    pin.timing.push(arc);
                }
                "internal_power" => {
                    let ip = self.internal_power(sub);
                    pin.internal_power.push(ip);
                }
                _ => {}
            }
        }
    }

    fn timing(&mut self, g: &Group) -> TimingArc {
        let mut arc = TimingArc {
            related_pin: g.attr_str("related_pin").unwrap_or_default().to_string(),
            timing_type: g
                .attr_str("timing_type")
                .map_or(TimingType::Combinational, TimingType::parse),
            timing_sense: None,
            when: self.expr(g, "when"),
            sdf_cond: g.attr_str("sdf_cond").map(str::to_string),
            cell_rise: None,
            cell_fall: None,
            rise_transition: None,
            fall_transition: None,
            rise_constraint: None,
            fall_constraint: None,
            other_tables: Vec::new(),
            intrinsic_rise: self.number(g, "intrinsic_rise"),
            intrinsic_fall: self.number(g, "intrinsic_fall"),
            span: g.span,
        };
        if let Some(s) = g.attr_str("timing_sense") {
            arc.timing_sense = TimingSense::parse(s);
            if arc.timing_sense.is_none() {
                let span = g.attribute("timing_sense").map_or(g.span, |a| a.span);
                self.warn(span, format!("unknown timing_sense `{s}`"));
            }
        }
        for sub in &g.groups {
            let table = self.table(sub);
            match sub.name.as_str() {
                "cell_rise" => arc.cell_rise = Some(table),
                "cell_fall" => arc.cell_fall = Some(table),
                "rise_transition" => arc.rise_transition = Some(table),
                "fall_transition" => arc.fall_transition = Some(table),
                "rise_constraint" => arc.rise_constraint = Some(table),
                "fall_constraint" => arc.fall_constraint = Some(table),
                _ => arc.other_tables.push((sub.name.clone(), table)),
            }
        }
        arc
    }

    fn internal_power(&mut self, g: &Group) -> InternalPower {
        let mut ip = InternalPower {
            related_pin: g.attr_str("related_pin").map(str::to_string),
            related_pg_pin: g.attr_str("related_pg_pin").map(str::to_string),
            when: self.expr(g, "when"),
            rise_power: None,
            fall_power: None,
            power: None,
        };
        for sub in &g.groups {
            let table = self.table(sub);
            match sub.name.as_str() {
                "rise_power" => ip.rise_power = Some(table),
                "fall_power" => ip.fall_power = Some(table),
                "power" => ip.power = Some(table),
                _ => {}
            }
        }
        ip
    }

    fn bus(&mut self, cell: &mut Cell, g: &Group) {
        let bus_name = g.arg_name().unwrap_or_default().to_string();
        let bus_type = g
            .attr_str("bus_type")
            .and_then(|t| self.bus_types.iter().find(|b| b.name == t).cloned());
        let indices = match bus_type {
            Some(t) => t.indices(),
            None => {
                // Without a type declaration, the member pin groups
                // decide which bits exist.
                let mut seen = Vec::new();
                for sub in g.groups_named("pin") {
                    for name in expand_pin_name(sub.arg_name().unwrap_or_default()) {
                        if let Some(i) = bit_index(&name)
                            && !seen.contains(&i)
                        {
                            seen.push(i);
                        }
                    }
                }
                if seen.is_empty() {
                    self.warn(
                        g.span,
                        format!("bus `{bus_name}` has no known `bus_type` and no member pins"),
                    );
                }
                seen
            }
        };
        for bit in indices {
            let name = format!("{bus_name}[{bit}]");
            let mut pin = Pin::new(&name, g.span);
            pin.bus = Some(bus_name.clone());
            pin.bit = Some(bit);
            self.apply_pin(&mut pin, g);
            for sub in g.groups_named("pin") {
                let members = expand_pin_name(sub.arg_name().unwrap_or_default());
                if members.iter().any(|m| m == &name) {
                    pin.span = sub.span;
                    self.apply_pin(&mut pin, sub);
                }
            }
            cell.pins.push(pin);
        }
    }

    fn bundle(&mut self, cell: &mut Cell, g: &Group) {
        let bundle_name = g.arg_name().unwrap_or_default().to_string();
        let members: Vec<String> = g
            .attr("members")
            .map(|v| {
                v.as_list()
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if members.is_empty() {
            self.warn(g.span, format!("bundle `{bundle_name}` has no `members`"));
        }
        for (i, name) in members.iter().enumerate() {
            let mut pin = Pin::new(name, g.span);
            pin.bus = Some(bundle_name.clone());
            pin.bit = i64::try_from(i).ok();
            self.apply_pin(&mut pin, g);
            for sub in g.groups_named("pin") {
                if sub.arg_name() == Some(name.as_str()) {
                    pin.span = sub.span;
                    self.apply_pin(&mut pin, sub);
                }
            }
            cell.pins.push(pin);
        }
    }
}

/// Expands `D[3:0]` into `D[3]`, `D[2]`, `D[1]`, `D[0]`; any other name is
/// returned as is.
fn expand_pin_name(name: &str) -> Vec<String> {
    if let Some(open) = name.find('[')
        && name.ends_with(']')
        && let Some(colon) = name[open..].find(':')
    {
        let base = &name[..open];
        let hi = name[open + 1..open + colon].trim().parse::<i64>();
        let lo = name[open + colon + 1..name.len() - 1].trim().parse::<i64>();
        if let (Ok(hi), Ok(lo)) = (hi, lo) {
            let range: Vec<i64> = if hi >= lo {
                (lo..=hi).rev().collect()
            } else {
                (hi..=lo).collect()
            };
            return range.into_iter().map(|i| format!("{base}[{i}]")).collect();
        }
    }
    vec![name.to_string()]
}

/// The index in `name[index]`.
fn bit_index(name: &str) -> Option<i64> {
    let open = name.find('[')?;
    name.strip_suffix(']')?[open + 1..].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    const LIB: &str = r#"
library (demo) {
  delay_model : table_lookup;
  time_unit : "1ns";
  voltage_unit : "1V";
  current_unit : "1mA";
  leakage_power_unit : "1pW";
  pulling_resistance_unit : "1kohm";
  capacitive_load_unit (1.0, pf);
  default_max_transition : 1.5;
  nom_voltage : 1.8;
  operating_conditions (tt_025C_1v80) { process : 1; temperature : 25; voltage : 1.8; }
  default_operating_conditions : tt_025C_1v80;
  lu_table_template (delay_2x2) {
    variable_1 : input_net_transition;
    variable_2 : total_output_net_capacitance;
    index_1 ("0.01, 1.0");
    index_2 ("0.001, 0.1");
  }
  type (BUS2) { base_type : array; data_type : bit; bit_width : 2; bit_from : 1; bit_to : 0; }
  cell (inv) {
    area : 3.75;
    cell_leakage_power : 0.002;
    leakage_power () { when : "!A"; value : 0.001; }
    pin (A) { direction : input; capacitance : 0.0015; }
    pin (Y) {
      direction : output;
      function : "!A";
      max_capacitance : 0.3;
      timing () {
        related_pin : "A";
        timing_sense : negative_unate;
        cell_rise (delay_2x2) { values ("0.1, 0.2", "0.3, 0.4"); }
        cell_fall (delay_2x2) { index_1 ("0.02, 2.0"); values ("0.11, 0.21", "0.31, 0.41"); }
      }
    }
  }
  cell (dff) {
    area : 20;
    ff (IQ, IQ_N) { next_state : "D"; clocked_on : "CLK"; clear : "!RESET_B"; }
    pin (CLK) { direction : input; clock : true; }
    pin (D) {
      direction : input;
      timing () { related_pin : "CLK"; timing_type : setup_rising; rise_constraint (scalar) { values ("0.05"); } fall_constraint (scalar) { values ("0.06"); } }
      timing () { related_pin : "CLK"; timing_type : hold_rising; rise_constraint (scalar) { values ("-0.01"); } }
    }
    pin (RESET_B) { direction : input; }
    pin (Q) { direction : output; function : "IQ"; timing () { related_pin : "CLK"; timing_type : rising_edge; } }
  }
  cell (reg2) {
    area : 40;
    dont_use : true;
    ff_bank (IQ, IQ_N, 2) { next_state : "D"; clocked_on : "CLK"; }
    pin (CLK) { direction : input; clock : true; }
    bus (D) {
      bus_type : BUS2;
      direction : input;
      capacitance : 0.002;
      pin (D[1]) { capacitance : 0.003; }
    }
    bus (Q) {
      bus_type : BUS2;
      direction : output;
      function : "IQ";
      pin (Q[1:0]) { timing () { related_pin : "CLK"; timing_type : rising_edge; } }
    }
    bundle (QN) {
      members (QN0, QN1);
      direction : output;
      function : "IQ_N";
    }
  }
  cell (tbuf) {
    area : 5;
    pin (A) { direction : input; }
    pin (OE) { direction : input; }
    pin (Y) { direction : output; function : "A"; three_state : "!OE"; }
  }
}
"#;

    fn parse(text: &str) -> (Library, Diagnostics, SourceMap) {
        let mut map = SourceMap::new();
        let id = map.add("demo.lib", text).unwrap();
        let mut diags = Diagnostics::new();
        let lib = Library::parse(text, id, &mut diags).expect("library");
        (lib, diags, map)
    }

    #[test]
    fn typed_view_of_a_small_library() {
        let (lib, diags, map) = parse(LIB);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        assert_eq!(lib.name, "demo");
        assert_eq!(lib.units.time.as_ref().unwrap().scale, 1e-9);
        assert_eq!(lib.units.capacitance.as_ref().unwrap().scale, 1e-12);
        assert_eq!(lib.units.voltage.as_ref().unwrap().scale, 1.0);
        assert_eq!(lib.units.current.as_ref().unwrap().scale, 1e-3);
        assert_eq!(lib.units.leakage_power.as_ref().unwrap().scale, 1e-12);
        assert_eq!(lib.units.resistance.as_ref().unwrap().scale, 1e3);
        assert_eq!(lib.defaults.max_transition, Some(1.5));
        assert_eq!(
            lib.defaults.operating_conditions.as_deref(),
            Some("tt_025C_1v80")
        );
        assert_eq!(lib.operating_conditions[0].temperature, Some(25.0));
        assert_eq!(lib.templates.len(), 1);
        assert_eq!(lib.templates[0].variables.len(), 2);
        assert_eq!(lib.cells.len(), 4);

        let inv = lib.cell("inv").unwrap();
        assert_eq!(inv.area, Some(3.75));
        assert!(inv.is_combinational());
        assert!(!inv.is_sequential());
        assert_eq!(inv.leakage_power.len(), 1);
        assert_eq!(
            inv.leakage_power[0].when.as_ref().unwrap().to_string(),
            "!A"
        );
        let y = lib.pin("inv", "Y").unwrap();
        assert_eq!(y.function.as_ref().unwrap().to_string(), "!A");
        assert_eq!(y.max_capacitance, Some(0.3));
        let arc = &y.timing[0];
        assert_eq!(arc.related_pin, "A");
        assert_eq!(arc.timing_type, TimingType::Combinational);
        assert_eq!(arc.timing_sense, Some(TimingSense::NegativeUnate));
        let rise = arc.cell_rise.as_ref().unwrap();
        assert_eq!(rise.index_1, vec![0.01, 1.0], "inherited from template");
        assert_eq!(rise.index_2, vec![0.001, 0.1]);
        assert_eq!(rise.lookup(0.01, 0.1), Some(0.2));
        let fall = arc.cell_fall.as_ref().unwrap();
        assert_eq!(fall.index_1, vec![0.02, 2.0], "table overrides template");
        assert_eq!(arc.max_delay(0.02, 0.001), Some(0.11));

        let dff = lib.cell("dff").unwrap();
        assert!(dff.is_sequential());
        assert!(!dff.is_combinational());
        let reg = dff.register().unwrap();
        assert_eq!(reg.kind, RegisterKind::Ff);
        assert_eq!(reg.outputs, ("IQ".to_string(), "IQ_N".to_string()));
        assert_eq!(reg.data.as_ref().unwrap().to_string(), "D");
        assert_eq!(reg.clear.as_ref().unwrap().to_string(), "!RESET_B");
        assert!(lib.pin("dff", "CLK").unwrap().clock);
        let d = lib.pin("dff", "D").unwrap();
        let setup = d.timing.iter().find(|t| t.timing_type.is_setup()).unwrap();
        assert_eq!(
            setup.rise_constraint.as_ref().unwrap().lookup(0.0, 0.0),
            Some(0.05)
        );
        assert_eq!(
            setup.fall_constraint.as_ref().unwrap().lookup(0.0, 0.0),
            Some(0.06)
        );
        let hold = d.timing.iter().find(|t| t.timing_type.is_hold()).unwrap();
        assert_eq!(hold.rise_constraint.as_ref().unwrap().values, vec![-0.01]);
        assert!(hold.timing_type.is_constraint());
        assert!(lib.pin("dff", "Q").unwrap().timing[0].timing_type.is_edge());

        let tbuf = lib.cell("tbuf").unwrap();
        assert!(!tbuf.is_combinational());
        assert_eq!(
            tbuf.pin("Y")
                .unwrap()
                .three_state
                .as_ref()
                .unwrap()
                .to_string(),
            "!OE"
        );

        let mappable: Vec<&str> = lib.mappable_gates().map(|c| c.name.as_str()).collect();
        assert_eq!(mappable, vec!["inv"]);
    }

    #[test]
    fn buses_and_bundles_expand_into_pins() {
        let (lib, diags, map) = parse(LIB);
        assert!(diags.is_empty(), "{}", diags.render(&map));
        let reg = lib.cell("reg2").unwrap();
        assert!(reg.dont_use);
        assert_eq!(reg.registers[0].bits, Some(2));
        let names: Vec<&str> = reg.pins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["CLK", "D[1]", "D[0]", "Q[1]", "Q[0]", "QN0", "QN1"]
        );
        let d1 = reg.pin("D[1]").unwrap();
        assert_eq!(d1.bus.as_deref(), Some("D"));
        assert_eq!(d1.bit, Some(1));
        assert_eq!(d1.direction, Some(Direction::Input));
        assert_eq!(d1.capacitance, Some(0.003), "member overrides bus");
        assert_eq!(reg.pin("D[0]").unwrap().capacitance, Some(0.002));
        let q0 = reg.pin("Q[0]").unwrap();
        assert_eq!(q0.function.as_ref().unwrap().to_string(), "IQ");
        assert_eq!(q0.timing.len(), 1, "ranged member pin applies to both");
        let qn1 = reg.pin("QN1").unwrap();
        assert_eq!(qn1.bus.as_deref(), Some("QN"));
        assert_eq!(qn1.bit, Some(1));
        assert_eq!(qn1.function.as_ref().unwrap().to_string(), "IQ_N");
    }

    #[test]
    fn bad_values_are_warnings_with_spans() {
        let text = "library (x) {\n  cell (c) {\n    area : big;\n    pin (Y) { direction : output; function : \"A &\"; }\n    pin (Z) { direction : sideways; }\n  }\n}\n";
        let (lib, diags, map) = parse(text);
        assert_eq!(diags.error_count(), 0);
        assert_eq!(diags.warning_count(), 3, "{}", diags.render(&map));
        let rendered = diags.render(&map);
        assert!(rendered.contains("`area` should be a number"), "{rendered}");
        assert!(rendered.contains("cannot parse `function`"), "{rendered}");
        assert!(rendered.contains("3:5"), "{rendered}");
        let c = lib.cell("c").unwrap();
        assert_eq!(c.area, None);
        assert_eq!(c.pin("Y").unwrap().function, None);
        assert_eq!(c.pin("Y").unwrap().function_text.as_deref(), Some("A &"));
        assert_eq!(c.pin("Z").unwrap().direction, None);
    }

    #[test]
    fn missing_library_group_is_an_error() {
        let mut map = SourceMap::new();
        let id = map.add("x.lib", "cell (c) { }").unwrap();
        let mut diags = Diagnostics::new();
        assert!(Library::parse("cell (c) { }", id, &mut diags).is_none());
        assert_eq!(diags.error_count(), 1);
    }

    #[test]
    fn unit_parsing() {
        assert_eq!(Unit::parse("1ns").scale, 1e-9);
        assert_eq!(Unit::parse("1ps").scale, 1e-12);
        assert_eq!(Unit::parse("1pf").scale, 1e-12);
        assert_eq!(Unit::parse("1ff").scale, 1e-15);
        assert_eq!(Unit::parse("1V").scale, 1.0);
        assert_eq!(Unit::parse("1mA").scale, 1e-3);
        assert_eq!(Unit::parse("1uA").scale, 1e-6);
        assert_eq!(Unit::parse("1kohm").scale, 1e3);
        assert_eq!(Unit::parse("1Mohm").scale, 1e6);
        assert_eq!(Unit::parse("1pW").scale, 1e-12);
        assert_eq!(Unit::parse("1nW").scale, 1e-9);
        assert_eq!(Unit::parse("0.01pf").scale, 1e-14);
    }

    #[test]
    fn gate_summary_is_sorted_and_stable() {
        let (lib, _, _) = parse(LIB);
        let summary = lib.to_gate_summary();
        let lines: Vec<&str> = summary.lines().collect();
        assert_eq!(
            lines[0],
            "library demo: 4 cells (time 1ns, capacitance 1pf, voltage 1V)"
        );
        assert_eq!(
            lines[1],
            "dff  area=20  seq  Q=IQ  ff(IQ,IQ_N) data=D clock=CLK clear=!RESET_B"
        );
        assert_eq!(lines[2], "inv  area=3.75  Y=!A");
        assert!(lines[3].starts_with("reg2  area=40  seq  dont_use  Q[1]=IQ  Q[0]=IQ"));
        assert_eq!(lines[4], "tbuf  area=5  Y=A z=!OE");
    }

    #[test]
    fn pin_name_expansion() {
        assert_eq!(expand_pin_name("A"), vec!["A"]);
        assert_eq!(expand_pin_name("D[2:0]"), vec!["D[2]", "D[1]", "D[0]"]);
        assert_eq!(expand_pin_name("D[0:1]"), vec!["D[0]", "D[1]"]);
        assert_eq!(expand_pin_name("D[3]"), vec!["D[3]"]);
        assert_eq!(bit_index("D[3]"), Some(3));
        assert_eq!(bit_index("D"), None);
    }
}
