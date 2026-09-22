//! Delay models: how long one arc of the timing graph takes.
//!
//! A [`DelayModel`] answers four questions about a design, and knows
//! nothing about the graph that asks them:
//!
//! | Question                      | Method                          |
//! |-------------------------------|---------------------------------|
//! | How long from this cell input pin to that output pin? | [`DelayModel::cell_arc`] |
//! | How long from a driver to one of its loads?           | [`DelayModel::net_arc`]  |
//! | How much capacitance does an input pin present?       | [`DelayModel::pin_capacitance`] |
//! | What setup and hold does this flip-flop need?         | [`DelayModel::constraint`] |
//!
//! Everything is kept as a [`Transition`], a pair of numbers for the
//! rising and the falling output edge, because that is where real slack
//! lives: an inverter that is fast pulling down and slow pulling up turns
//! a comfortable path into a violated one only on one of the two edges.
//! A model also reports the [`Sense`] of an arc, so the analysis knows
//! which input edge produces which output edge.
//!
//! Each arc carries both a `max_delay` and a `min_delay`, used by the
//! setup (late) and hold (early) analyses. A single-corner library has
//! them equal; [`super::sta::TimingOptions`] then applies its derating
//! factors on top.
//!
//! # The three models
//!
//! - [`UnitModel`]: every cell arc costs one unit, every net arc nothing.
//!   It is what the unit tests and the golden files use, because the
//!   answer can be worked out by hand, and it is the sensible default for
//!   a design with no library at all.
//! - [`LibertyModel`] (with the `asic` feature): a real non-linear delay
//!   model over [`crate::asic::liberty::Library`]. `cell_rise`,
//!   `cell_fall`, `rise_transition` and `fall_transition` are looked up
//!   through [`crate::asic::liberty::LutTable::lookup`] with the input
//!   transition on one axis and the output capacitance on the other, the
//!   axes being identified from the table's template. Load capacitance is
//!   the sum of the `capacitance` of every pin the net drives.
//! - [`FpgaModel`]: a table of per-primitive numbers keyed by primitive
//!   name (`SB_LUT4`, `TRELLIS_FF`, ...), falling back to a [`UnitModel`]
//!   for anything not in the table. **The numbers in
//!   [`FpgaModel::placeholders`] are placeholders**, not characterised
//!   data; see the comment on that function.
//!
//! # Not modelled
//!
//! Wire RC (a net is a lumped capacitance and a flat interconnect delay,
//! never a distributed tree), on-chip variation beyond the two flat
//! derating factors, crosstalk, IR drop, temperature inversion, and
//! `when`-conditional arcs: a Liberty arc with a `when` condition is
//! treated as unconditional, which is pessimistic for delay and therefore
//! safe for setup but can be optimistic for hold.

use crate::ir::{Cell, Module, NetId};

/// Which way a signal is moving at a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Edge {
    /// A 0 to 1 transition.
    Rise,
    /// A 1 to 0 transition.
    Fall,
}

impl Edge {
    /// Both edges, in a fixed order.
    pub const ALL: [Edge; 2] = [Edge::Rise, Edge::Fall];

    /// A one-letter tag for reports: `r` or `f`.
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Rise => "r",
            Edge::Fall => "f",
        }
    }

    /// The other edge.
    pub fn opposite(self) -> Edge {
        match self {
            Edge::Rise => Edge::Fall,
            Edge::Fall => Edge::Rise,
        }
    }
}

/// A number per output edge: one for a rising transition, one for a
/// falling one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transition {
    /// The value for a rising transition.
    pub rise: f64,
    /// The value for a falling transition.
    pub fall: f64,
}

impl Transition {
    /// Both edges zero.
    pub const ZERO: Transition = Transition {
        rise: 0.0,
        fall: 0.0,
    };

    /// A pair with the two edges given separately.
    pub fn new(rise: f64, fall: f64) -> Transition {
        Transition { rise, fall }
    }

    /// The same value on both edges.
    pub fn both(value: f64) -> Transition {
        Transition {
            rise: value,
            fall: value,
        }
    }

    /// The value for one edge.
    pub fn get(self, edge: Edge) -> f64 {
        match edge {
            Edge::Rise => self.rise,
            Edge::Fall => self.fall,
        }
    }

    /// Replaces the value for one edge.
    pub fn set(&mut self, edge: Edge, value: f64) {
        match edge {
            Edge::Rise => self.rise = value,
            Edge::Fall => self.fall = value,
        }
    }

    /// The larger of the two edges.
    pub fn worst(self) -> f64 {
        self.rise.max(self.fall)
    }

    /// The smaller of the two edges.
    pub fn best(self) -> f64 {
        self.rise.min(self.fall)
    }

    /// Edge-wise maximum.
    pub fn max_with(self, other: Transition) -> Transition {
        Transition {
            rise: self.rise.max(other.rise),
            fall: self.fall.max(other.fall),
        }
    }

    /// Edge-wise minimum.
    pub fn min_with(self, other: Transition) -> Transition {
        Transition {
            rise: self.rise.min(other.rise),
            fall: self.fall.min(other.fall),
        }
    }

    /// Both edges multiplied by `factor`.
    pub fn scale(self, factor: f64) -> Transition {
        Transition {
            rise: self.rise * factor,
            fall: self.fall * factor,
        }
    }
}

/// How an input edge maps to an output edge on one arc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    /// The output follows the input: a rising output comes from a rising
    /// input (a buffer, an AND, a MUX data input).
    Positive,
    /// The output inverts the input (a NOT, the bubble of a NAND).
    Negative,
    /// Either input edge can produce either output edge (an XOR, an
    /// adder, a MUX select).
    NonUnate,
}

impl Sense {
    /// Which input edges can produce `output`.
    pub fn input_edges(self, output: Edge) -> &'static [Edge] {
        match self {
            Sense::Positive => match output {
                Edge::Rise => &[Edge::Rise],
                Edge::Fall => &[Edge::Fall],
            },
            Sense::Negative => match output {
                Edge::Rise => &[Edge::Fall],
                Edge::Fall => &[Edge::Rise],
            },
            Sense::NonUnate => &Edge::ALL,
        }
    }

    /// A word for reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Sense::Positive => "positive",
            Sense::Negative => "negative",
            Sense::NonUnate => "non-unate",
        }
    }
}

/// What a model says about one arc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcResult {
    /// Late (setup) delay, per output edge.
    pub max_delay: Transition,
    /// Early (hold) delay, per output edge.
    pub min_delay: Transition,
    /// The output transition time, per output edge, propagated to the
    /// next stage as its input slew.
    pub slew: Transition,
    /// Which input edge produces which output edge.
    pub sense: Sense,
}

impl ArcResult {
    /// An arc with no delay and no slew.
    pub const ZERO: ArcResult = ArcResult {
        max_delay: Transition::ZERO,
        min_delay: Transition::ZERO,
        slew: Transition::ZERO,
        sense: Sense::Positive,
    };

    /// An arc whose early and late delays are the same.
    pub fn fixed(delay: Transition, slew: Transition, sense: Sense) -> ArcResult {
        ArcResult {
            max_delay: delay,
            min_delay: delay,
            slew,
            sense,
        }
    }
}

/// The setup and hold a sequential cell needs on one data pin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SequentialConstraint {
    /// Setup time, per data edge.
    pub setup: Transition,
    /// Hold time, per data edge.
    pub hold: Transition,
}

/// What the analysis tells a model about a cell arc it is asking for.
#[derive(Clone, Copy, Debug)]
pub struct ArcQuery<'a> {
    /// The module the cell lives in.
    pub module: &'a Module,
    /// The cell, when the arc belongs to one.
    pub cell: Option<&'a Cell>,
    /// The cell type name: a primitive keyword, a black box's name, or
    /// `assign` for a continuous assignment.
    pub cell_type: &'a str,
    /// The input pin the arc starts at.
    pub from_port: &'a str,
    /// The output pin the arc ends at.
    pub to_port: &'a str,
    /// The transition time arriving at `from_port`.
    pub input_slew: Transition,
    /// Total capacitance the output drives.
    pub load: f64,
    /// How many pins the output drives.
    pub fanout: usize,
    /// The sense the graph deduced from the primitive, which a model with
    /// better information may override in its answer.
    pub sense: Sense,
}

/// What the analysis tells a model about a net arc it is asking for.
#[derive(Clone, Copy, Debug)]
pub struct NetQuery<'a> {
    /// The module the net lives in.
    pub module: &'a Module,
    /// The net being crossed.
    pub net: NetId,
    /// The driving cell's type name, for models that vary by driver.
    pub driver_type: &'a str,
    /// How many pins the net drives.
    pub fanout: usize,
    /// Total capacitance on the net.
    pub load: f64,
    /// The transition time leaving the driver.
    pub input_slew: Transition,
}

/// What the analysis tells a model about an input pin whose capacitance
/// it needs.
#[derive(Clone, Copy, Debug)]
pub struct PinQuery<'a> {
    /// The module the pin lives in.
    pub module: &'a Module,
    /// The cell, when the pin belongs to one.
    pub cell: Option<&'a Cell>,
    /// The cell type name.
    pub cell_type: &'a str,
    /// The pin name.
    pub port: &'a str,
}

/// What the analysis tells a model about a setup / hold check.
#[derive(Clone, Copy, Debug)]
pub struct ConstraintQuery<'a> {
    /// The module the cell lives in.
    pub module: &'a Module,
    /// The sequential cell.
    pub cell: &'a Cell,
    /// The cell type name.
    pub cell_type: &'a str,
    /// The data pin being checked (`d`, `en`, `rst`, `addr`, ...).
    pub data_port: &'a str,
    /// The clock pin it is checked against.
    pub clock_port: &'a str,
    /// The transition time arriving at the data pin.
    pub data_slew: Transition,
    /// The transition time arriving at the clock pin.
    pub clock_slew: Transition,
    /// True when the cell captures on the rising clock edge.
    pub clock_rising: bool,
}

/// Where delays come from.
///
/// Implementations are consulted arc by arc during the forward pass, in
/// topological order, so an implementation may assume the input slew it
/// is handed has already been computed for every earlier stage.
pub trait DelayModel {
    /// A short name, shown in the report header.
    fn name(&self) -> String;

    /// The delay of one input-pin to output-pin arc inside a cell.
    fn cell_arc(&self, query: &ArcQuery<'_>) -> ArcResult;

    /// The interconnect delay from a driver to one of its loads.
    fn net_arc(&self, query: &NetQuery<'_>) -> ArcResult;

    /// The capacitance an input pin presents to the net driving it.
    /// Zero by default, which makes load-independent models behave.
    fn pin_capacitance(&self, query: &PinQuery<'_>) -> f64 {
        let _ = query;
        0.0
    }

    /// The setup and hold of a sequential cell's data pin.
    fn constraint(&self, query: &ConstraintQuery<'_>) -> SequentialConstraint;

    /// The topology of a cell type the IR's primitive set does not
    /// describe: a [`crate::ir::CellKind::Blackbox`]. `inputs` and
    /// `outputs` are the pin names the instance actually connects.
    ///
    /// `None`, the default, means the model knows nothing about it, and
    /// [`super::graph::TimingGraph`] treats the cell as a boundary: its
    /// outputs start paths and its inputs end them, with no arc across
    /// it. A model backed by a library returns the real answer, which is
    /// what makes a standard-cell netlist time end to end.
    fn cell_topology(
        &self,
        cell_type: &str,
        inputs: &[String],
        outputs: &[String],
    ) -> Option<CellTopology> {
        let _ = (cell_type, inputs, outputs);
        None
    }
}

/// What a delay model knows about the shape of a cell type: which input
/// pin reaches which output pin, and which pins are checked against a
/// clock.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellTopology {
    /// Arcs as `(input pin, output pin, sense)`.
    pub arcs: Vec<(String, String, Sense)>,
    /// Input pins carrying a setup and hold check.
    pub checked_pins: Vec<String>,
    /// The clock pin those checks are against, if the cell is
    /// sequential.
    pub clock_pin: Option<String>,
}

impl CellTopology {
    /// True when the cell has storage, so paths stop at it.
    pub fn is_sequential(&self) -> bool {
        self.clock_pin.is_some()
    }
}

/// A topology guessed from pin names alone: an input called `clk`, `c`,
/// `ck` or `clock` is the clock, every other input is data, and every
/// output is driven.
///
/// A cell with a clock becomes sequential (a clock-to-output arc and a
/// setup / hold check on every data pin) and one without becomes fully
/// combinational (every input reaches every output, non-unate). It is a
/// guess, but the right one for the FPGA primitives whose pins are
/// called `C`/`D`/`Q` and `I0`..`I3`/`O`; anything characterised should
/// come from a library instead.
pub fn topology_from_pin_names(inputs: &[String], outputs: &[String]) -> CellTopology {
    let is_clock = |name: &str| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "clk" | "c" | "ck" | "clock"
        )
    };
    let clock_pin = inputs.iter().find(|n| is_clock(n)).cloned();
    let data: Vec<String> = inputs.iter().filter(|n| !is_clock(n)).cloned().collect();
    let mut arcs = Vec::new();
    match &clock_pin {
        Some(clock) => {
            for out in outputs {
                arcs.push((clock.clone(), out.clone(), Sense::Positive));
            }
        }
        None => {
            for input in &data {
                for out in outputs {
                    arcs.push((input.clone(), out.clone(), Sense::NonUnate));
                }
            }
        }
    }
    CellTopology {
        arcs,
        checked_pins: if clock_pin.is_some() {
            data
        } else {
            Vec::new()
        },
        clock_pin,
    }
}

// ---------------------------------------------------------------------------
// The unit model

/// Every cell arc one unit, every net arc nothing.
///
/// The point is arithmetic a reader can check: with the defaults, the
/// delay through a chain of `n` cells is exactly `n`, so the slack of a
/// register-to-register path with period `P` is `P - n - setup`. The
/// fields are public so a test can give the model a setup time, a
/// clock-to-Q or an interconnect delay without inventing a library.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitModel {
    /// Delay of every combinational cell arc.
    pub cell_delay: f64,
    /// Delay of the clock-to-output arc of a sequential cell.
    pub clock_to_q: f64,
    /// Delay of every net arc.
    pub net_delay: f64,
    /// Setup time of every sequential data pin.
    pub setup: f64,
    /// Hold time of every sequential data pin.
    pub hold: f64,
    /// Output transition time reported for every arc.
    pub slew: f64,
    /// Capacitance every input pin presents.
    pub pin_capacitance: f64,
}

impl Default for UnitModel {
    /// One unit per cell arc, nothing anywhere else.
    fn default() -> Self {
        UnitModel {
            cell_delay: 1.0,
            clock_to_q: 1.0,
            net_delay: 0.0,
            setup: 0.0,
            hold: 0.0,
            slew: 0.0,
            pin_capacitance: 0.0,
        }
    }
}

impl UnitModel {
    /// The default model: one unit per cell arc, nothing anywhere else.
    pub fn new() -> UnitModel {
        UnitModel::default()
    }

    /// The same model with a different setup time.
    pub fn with_setup(mut self, setup: f64) -> UnitModel {
        self.setup = setup;
        self
    }

    /// The same model with a different hold time.
    pub fn with_hold(mut self, hold: f64) -> UnitModel {
        self.hold = hold;
        self
    }

    /// The same model with a different combinational cell delay.
    pub fn with_cell_delay(mut self, delay: f64) -> UnitModel {
        self.cell_delay = delay;
        self
    }

    /// The same model with a different clock-to-output delay.
    pub fn with_clock_to_q(mut self, delay: f64) -> UnitModel {
        self.clock_to_q = delay;
        self
    }

    /// The same model with a different interconnect delay.
    pub fn with_net_delay(mut self, delay: f64) -> UnitModel {
        self.net_delay = delay;
        self
    }
}

/// True for the cell type names whose input-to-output arc is a
/// clock-to-output arc rather than combinational logic.
fn is_sequential_type(cell_type: &str) -> bool {
    matches!(cell_type, "dff" | "dlatch" | "memrd" | "memwr")
}

impl DelayModel for UnitModel {
    fn name(&self) -> String {
        "unit".to_owned()
    }

    fn cell_arc(&self, query: &ArcQuery<'_>) -> ArcResult {
        let delay = if is_sequential_type(query.cell_type) && query.from_port == "clk" {
            self.clock_to_q
        } else {
            self.cell_delay
        };
        ArcResult::fixed(
            Transition::both(delay),
            Transition::both(self.slew),
            query.sense,
        )
    }

    fn net_arc(&self, query: &NetQuery<'_>) -> ArcResult {
        ArcResult::fixed(
            Transition::both(self.net_delay),
            query.input_slew,
            Sense::Positive,
        )
    }

    fn pin_capacitance(&self, _query: &PinQuery<'_>) -> f64 {
        self.pin_capacitance
    }

    fn constraint(&self, _query: &ConstraintQuery<'_>) -> SequentialConstraint {
        SequentialConstraint {
            setup: Transition::both(self.setup),
            hold: Transition::both(self.hold),
        }
    }
}

// ---------------------------------------------------------------------------
// The FPGA model

/// The numbers one FPGA primitive contributes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveTiming {
    /// Delay of a combinational input-to-output arc.
    pub comb: f64,
    /// Delay of the clock-to-output arc.
    pub clock_to_q: f64,
    /// Setup time on a data pin.
    pub setup: f64,
    /// Hold time on a data pin.
    pub hold: f64,
    /// Interconnect delay charged to each net this primitive drives.
    pub route: f64,
}

impl PrimitiveTiming {
    /// A purely combinational primitive.
    pub const fn comb(delay: f64, route: f64) -> PrimitiveTiming {
        PrimitiveTiming {
            comb: delay,
            clock_to_q: 0.0,
            setup: 0.0,
            hold: 0.0,
            route,
        }
    }

    /// A sequential primitive.
    pub const fn seq(clock_to_q: f64, setup: f64, hold: f64, route: f64) -> PrimitiveTiming {
        PrimitiveTiming {
            comb: 0.0,
            clock_to_q,
            setup,
            hold,
            route,
        }
    }
}

/// Per-primitive delays for FPGA netlists, keyed by primitive name.
///
/// The table lives here rather than in `fpga::device` on purpose: a
/// device file describes what a family *has*, and adding uncharacterised
/// timing numbers to it would make them look authoritative. Anything not
/// in the table falls through to the wrapped [`UnitModel`].
#[derive(Clone, Debug, PartialEq)]
pub struct FpgaModel {
    /// Primitive name (upper case) and its numbers, sorted by name.
    entries: Vec<(String, PrimitiveTiming)>,
    /// Used for anything the table does not name.
    fallback: UnitModel,
    /// Shown in the report header.
    label: String,
}

impl FpgaModel {
    /// An empty table: every primitive falls back to `fallback`.
    pub fn empty(fallback: UnitModel) -> FpgaModel {
        FpgaModel {
            entries: Vec::new(),
            fallback,
            label: "fpga".to_owned(),
        }
    }

    /// The built-in table.
    ///
    /// **These numbers are placeholders.** They are not taken from any
    /// vendor datasheet or from the open timing databases (`icetime`'s
    /// `timings_*.txt`, prjtrellis' `.db`): none of those is vendored
    /// here, and inventing a source would be worse than saying so. They
    /// are ordered sensibly relative to each other — a LUT costs more
    /// than a carry bit, routing costs more than either, a flip-flop's
    /// setup is a fraction of its clock-to-Q — so a report built on them
    /// ranks paths the way a real one would, while the absolute slack
    /// means nothing. Replace the table with
    /// [`FpgaModel::with_primitive`] when real numbers are available.
    pub fn placeholders() -> FpgaModel {
        let mut model = FpgaModel::empty(UnitModel::default());
        // iCE40 (SB_*), ECP5 / Trellis, and the generic family's names.
        let lut = PrimitiveTiming::comb(0.45, 0.90);
        let carry = PrimitiveTiming::comb(0.12, 0.25);
        let ff = PrimitiveTiming::seq(0.55, 0.30, 0.05, 0.90);
        let bram = PrimitiveTiming::seq(2.10, 0.40, 0.05, 1.20);
        let io = PrimitiveTiming::comb(1.20, 1.00);
        let gbuf = PrimitiveTiming::comb(0.25, 0.50);
        for (name, timing) in [
            ("SB_LUT4", lut),
            ("SB_CARRY", carry),
            ("SB_DFF", ff),
            ("SB_DFFE", ff),
            ("SB_DFFR", ff),
            ("SB_DFFS", ff),
            ("SB_DFFSR", ff),
            ("SB_DFFER", ff),
            ("SB_DFFES", ff),
            ("SB_DFFESR", ff),
            ("SB_DFFN", ff),
            ("SB_RAM40_4K", bram),
            ("SB_IO", io),
            ("SB_GB", gbuf),
            ("LUT4", lut),
            ("CCU2C", carry),
            ("TRELLIS_FF", ff),
            ("DP16KD", bram),
            ("TRELLIS_IO", io),
            ("DCCA", gbuf),
            ("GENERIC_LUT4", lut),
            ("GENERIC_FF", ff),
        ] {
            model = model.with_primitive(name, timing);
        }
        model
    }

    /// Adds or replaces one primitive's numbers. The name is matched
    /// case-insensitively.
    pub fn with_primitive(mut self, name: &str, timing: PrimitiveTiming) -> FpgaModel {
        let key = name.to_ascii_uppercase();
        match self.entries.binary_search_by(|(n, _)| n.as_str().cmp(&key)) {
            Ok(at) => self.entries[at].1 = timing,
            Err(at) => self.entries.insert(at, (key, timing)),
        }
        self
    }

    /// A different label for the report header.
    pub fn with_label(mut self, label: impl Into<String>) -> FpgaModel {
        self.label = label.into();
        self
    }

    /// The numbers for a primitive, if the table names it.
    pub fn primitive(&self, name: &str) -> Option<PrimitiveTiming> {
        let key = name.to_ascii_uppercase();
        self.entries
            .binary_search_by(|(n, _)| n.as_str().cmp(&key))
            .ok()
            .map(|at| self.entries[at].1)
    }

    /// Every primitive the table names, in name order.
    pub fn primitives(&self) -> impl Iterator<Item = (&str, PrimitiveTiming)> {
        self.entries.iter().map(|(n, t)| (n.as_str(), *t))
    }
}

impl DelayModel for FpgaModel {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn cell_arc(&self, query: &ArcQuery<'_>) -> ArcResult {
        let Some(timing) = self.primitive(query.cell_type) else {
            return self.fallback.cell_arc(query);
        };
        let delay = if query.from_port.eq_ignore_ascii_case("clk")
            || query.from_port.eq_ignore_ascii_case("c")
        {
            timing.clock_to_q
        } else {
            timing.comb
        };
        ArcResult::fixed(Transition::both(delay), Transition::ZERO, query.sense)
    }

    fn net_arc(&self, query: &NetQuery<'_>) -> ArcResult {
        let Some(timing) = self.primitive(query.driver_type) else {
            return self.fallback.net_arc(query);
        };
        ArcResult::fixed(
            Transition::both(timing.route),
            query.input_slew,
            Sense::Positive,
        )
    }

    fn pin_capacitance(&self, query: &PinQuery<'_>) -> f64 {
        self.fallback.pin_capacitance(query)
    }

    fn constraint(&self, query: &ConstraintQuery<'_>) -> SequentialConstraint {
        let Some(timing) = self.primitive(query.cell_type) else {
            return self.fallback.constraint(query);
        };
        SequentialConstraint {
            setup: Transition::both(timing.setup),
            hold: Transition::both(timing.hold),
        }
    }

    fn cell_topology(
        &self,
        cell_type: &str,
        inputs: &[String],
        outputs: &[String],
    ) -> Option<CellTopology> {
        // The table says nothing about pins, so the shape is guessed
        // from their names; see [`topology_from_pin_names`].
        let _ = self.primitive(cell_type)?;
        Some(topology_from_pin_names(inputs, outputs))
    }
}

// ---------------------------------------------------------------------------
// The Liberty model

#[cfg(feature = "asic")]
mod liberty_model {
    use super::{
        ArcQuery, ArcResult, CellTopology, ConstraintQuery, DelayModel, NetQuery, PinQuery,
        SequentialConstraint, Transition, UnitModel,
    };
    use crate::asic::liberty::{Library, LutTable, TimingArc, TimingSense};
    use crate::timing::delay::{Edge, Sense};

    /// Which quantity an axis of a lookup table carries.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Axis {
        /// An input transition time.
        Slew,
        /// An output (or related-pin) capacitance.
        Load,
        /// Anything else; treated as the first axis of the pair.
        Other,
    }

    fn classify(variable: &str) -> Axis {
        if variable.contains("transition") || variable.contains("slew") {
            Axis::Slew
        } else if variable.contains("capacitance") || variable.contains("fanout") {
            Axis::Load
        } else {
            Axis::Other
        }
    }

    /// A non-linear delay model over a Liberty library.
    ///
    /// Cells are matched to library cells by name: an
    /// [`crate::ir::CellKind::Blackbox`] uses the black box's name, and
    /// any other primitive uses the mapping installed with
    /// [`LibertyModel::map`], falling back to the primitive's own
    /// keyword. A cell the library does not have, a pin it does not
    /// have, or an arc with no usable table, falls through to the
    /// wrapped [`UnitModel`], so a partially mapped netlist still gets a
    /// report instead of an error.
    #[derive(Clone, Debug)]
    pub struct LibertyModel<'a> {
        library: &'a Library,
        mapping: Vec<(String, String)>,
        fallback: UnitModel,
    }

    impl<'a> LibertyModel<'a> {
        /// A model over `library`, with a unit model for everything the
        /// library does not describe.
        pub fn new(library: &'a Library) -> LibertyModel<'a> {
            LibertyModel {
                library,
                mapping: Vec::new(),
                fallback: UnitModel::default(),
            }
        }

        /// Uses `fallback` for cells the library does not describe.
        pub fn with_fallback(mut self, fallback: UnitModel) -> LibertyModel<'a> {
            self.fallback = fallback;
            self
        }

        /// Maps an IR cell type name (a primitive keyword such as `and`,
        /// or a black box name) to a library cell name.
        pub fn map(mut self, cell_type: &str, library_cell: &str) -> LibertyModel<'a> {
            let key = cell_type.to_owned();
            let value = library_cell.to_owned();
            match self.mapping.binary_search_by(|(k, _)| k.as_str().cmp(&key)) {
                Ok(at) => self.mapping[at].1 = value,
                Err(at) => self.mapping.insert(at, (key, value)),
            }
            self
        }

        /// The library cell backing an IR cell type, if there is one.
        pub fn library_cell(&self, cell_type: &str) -> Option<&'a crate::asic::liberty::Cell> {
            let mapped = self
                .mapping
                .binary_search_by(|(k, _)| k.as_str().cmp(cell_type))
                .ok()
                .map(|at| self.mapping[at].1.as_str());
            self.library.cell(mapped.unwrap_or(cell_type))
        }

        /// Looks a table up with the transition on one axis and the
        /// capacitance on the other, working out which is which from the
        /// table's template, and clamping both to the characterised
        /// range rather than extrapolating out of it.
        fn lookup(&self, table: &LutTable, slew: f64, load: f64) -> Option<f64> {
            let (var1, var2) = self
                .library
                .template(&table.template)
                .map(|t| {
                    (
                        t.variables.first().map_or(Axis::Other, |v| classify(v)),
                        t.variables.get(1).map_or(Axis::Other, |v| classify(v)),
                    )
                })
                .unwrap_or((Axis::Slew, Axis::Load));
            let value_for = |axis: Axis, other: Axis| match axis {
                Axis::Slew => slew,
                Axis::Load => load,
                // An unnamed first axis is the transition unless the
                // second one already is.
                Axis::Other if other == Axis::Slew => load,
                Axis::Other => slew,
            };
            let x = clamp_to(&table.index_1, value_for(var1, var2));
            let y = clamp_to(&table.index_2, value_for(var2, var1));
            table.lookup(x, y)
        }

        fn sense_of(&self, arc: &TimingArc, default: Sense) -> Sense {
            match arc.timing_sense {
                Some(TimingSense::PositiveUnate) => Sense::Positive,
                Some(TimingSense::NegativeUnate) => Sense::Negative,
                Some(TimingSense::NonUnate) => Sense::NonUnate,
                None => default,
            }
        }
    }

    /// Clamps `value` into the range an index covers; an empty or
    /// single-point index leaves it alone.
    fn clamp_to(index: &[f64], value: f64) -> f64 {
        match (index.first(), index.last()) {
            (Some(&lo), Some(&hi)) if hi > lo => value.max(lo).min(hi),
            _ => value,
        }
    }

    impl DelayModel for LibertyModel<'_> {
        fn name(&self) -> String {
            format!("liberty({})", self.library.name)
        }

        fn cell_arc(&self, query: &ArcQuery<'_>) -> ArcResult {
            let Some(cell) = self.library_cell(query.cell_type) else {
                return self.fallback.cell_arc(query);
            };
            let Some(pin) = cell.pin(query.to_port) else {
                return self.fallback.cell_arc(query);
            };
            let mut best: Option<ArcResult> = None;
            for arc in pin.arcs_from(query.from_port) {
                if arc.timing_type.is_constraint() {
                    continue;
                }
                let sense = self.sense_of(arc, query.sense);
                // A rising output is produced by whichever input edge
                // this arc's sense says produces it.
                let slew_for = |out: Edge| {
                    let edges = sense.input_edges(out);
                    edges
                        .iter()
                        .map(|e| query.input_slew.get(*e))
                        .fold(f64::NEG_INFINITY, f64::max)
                };
                let rise_slew = slew_for(Edge::Rise);
                let fall_slew = slew_for(Edge::Fall);
                let rise = arc
                    .cell_rise
                    .as_ref()
                    .and_then(|t| self.lookup(t, rise_slew, query.load))
                    .or(arc.intrinsic_rise);
                let fall = arc
                    .cell_fall
                    .as_ref()
                    .and_then(|t| self.lookup(t, fall_slew, query.load))
                    .or(arc.intrinsic_fall);
                let (Some(rise), Some(fall)) = (rise, fall) else {
                    continue;
                };
                let out_rise = arc
                    .rise_transition
                    .as_ref()
                    .and_then(|t| self.lookup(t, rise_slew, query.load))
                    .unwrap_or(0.0);
                let out_fall = arc
                    .fall_transition
                    .as_ref()
                    .and_then(|t| self.lookup(t, fall_slew, query.load))
                    .unwrap_or(0.0);
                let candidate = ArcResult::fixed(
                    Transition::new(rise, fall),
                    Transition::new(out_rise, out_fall),
                    sense,
                );
                best = Some(match best {
                    // Several arcs between the same pins (one per `when`
                    // condition) are merged pessimistically: the largest
                    // delay and the largest slew of the set.
                    Some(prev) => ArcResult {
                        max_delay: prev.max_delay.max_with(candidate.max_delay),
                        min_delay: prev.min_delay.min_with(candidate.min_delay),
                        slew: prev.slew.max_with(candidate.slew),
                        sense: if prev.sense == candidate.sense {
                            prev.sense
                        } else {
                            Sense::NonUnate
                        },
                    },
                    None => candidate,
                });
            }
            best.unwrap_or_else(|| self.fallback.cell_arc(query))
        }

        fn net_arc(&self, query: &NetQuery<'_>) -> ArcResult {
            // No wire model: a Liberty library says nothing about
            // interconnect, so the net arc is whatever the fallback
            // model charges (zero by default).
            self.fallback.net_arc(query)
        }

        fn cell_topology(
            &self,
            cell_type: &str,
            inputs: &[String],
            outputs: &[String],
        ) -> Option<CellTopology> {
            let cell = self.library_cell(cell_type)?;
            // The clock pin is the one the library marks `clock`, or
            // the one the constraint arcs relate to.
            let mut topology = CellTopology {
                clock_pin: cell
                    .pins
                    .iter()
                    .find(|p| p.clock && inputs.contains(&p.name))
                    .map(|p| p.name.clone()),
                ..CellTopology::default()
            };
            for pin in cell.pins.iter().filter(|p| outputs.contains(&p.name)) {
                for arc in &pin.timing {
                    if arc.timing_type.is_constraint() {
                        continue;
                    }
                    // `related_pin` may name several pins at once.
                    for related in arc.related_pin.split_whitespace() {
                        if !inputs.iter().any(|i| i == related) {
                            continue;
                        }

                        let sense = self.sense_of(arc, Sense::NonUnate);
                        let entry = (related.to_owned(), pin.name.clone(), sense);
                        if !topology.arcs.contains(&entry) {
                            topology.arcs.push(entry);
                        }
                    }
                }
            }
            for pin in cell.pins.iter().filter(|p| inputs.contains(&p.name)) {
                if pin.timing.iter().any(|a| a.timing_type.is_constraint()) {
                    topology.checked_pins.push(pin.name.clone());
                    if topology.clock_pin.is_none() {
                        topology.clock_pin = pin
                            .timing
                            .iter()
                            .filter(|a| a.timing_type.is_constraint())
                            .flat_map(|a| a.related_pin.split_whitespace())
                            .find(|r| inputs.iter().any(|i| i == r))
                            .map(str::to_owned);
                    }
                }
            }
            Some(topology)
        }

        fn pin_capacitance(&self, query: &PinQuery<'_>) -> f64 {
            self.library_cell(query.cell_type)
                .and_then(|c| c.pin(query.port))
                .and_then(|p| p.capacitance)
                .or(self.library.defaults.input_pin_cap)
                .unwrap_or_else(|| self.fallback.pin_capacitance(query))
        }

        fn constraint(&self, query: &ConstraintQuery<'_>) -> SequentialConstraint {
            let Some(cell) = self.library_cell(query.cell_type) else {
                return self.fallback.constraint(query);
            };
            let Some(pin) = cell.pin(query.data_port) else {
                return self.fallback.constraint(query);
            };
            let mut out = SequentialConstraint::default();
            let mut found = false;
            for arc in pin.arcs_from(query.clock_port) {
                let (setup, hold) = (arc.timing_type.is_setup(), arc.timing_type.is_hold());
                if !setup && !hold {
                    continue;
                }
                let rise = arc
                    .rise_constraint
                    .as_ref()
                    .and_then(|t| self.lookup(t, query.data_slew.rise, query.clock_slew.worst()));
                let fall = arc
                    .fall_constraint
                    .as_ref()
                    .and_then(|t| self.lookup(t, query.data_slew.fall, query.clock_slew.worst()));
                let value = Transition::new(rise.unwrap_or(0.0), fall.unwrap_or(0.0));
                if rise.is_none() && fall.is_none() {
                    continue;
                }
                found = true;
                if setup {
                    out.setup = out.setup.max_with(value);
                } else {
                    out.hold = out.hold.max_with(value);
                }
            }
            if found {
                out
            } else {
                self.fallback.constraint(query)
            }
        }
    }
}

#[cfg(feature = "asic")]
pub use liberty_model::LibertyModel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::types::Type;
    use crate::source::{SourceMap, Span};

    /// Timing values are floats; comparing them needs a tolerance. One
    /// femtosecond is far below anything a library characterises, so a
    /// difference this small can only come from floating-point rounding.
    pub(crate) const EPS: f64 = 1e-9;

    pub(crate) fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < EPS
    }

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("timing-test", "").unwrap();
        Span::new(id, 0, 0)
    }

    fn module() -> crate::ir::Module {
        let mut b = ModuleBuilder::new("m", span());
        let _ = b.input("a", Type::bit());
        b.finish()
    }

    #[test]
    fn transitions_and_senses() {
        let t = Transition::new(2.0, 3.0);
        assert!(close(t.get(Edge::Rise), 2.0));
        assert!(close(t.get(Edge::Fall), 3.0));
        assert!(close(t.worst(), 3.0));
        assert!(close(t.best(), 2.0));
        assert!(close(t.scale(2.0).rise, 4.0));
        let mut u = Transition::ZERO;
        u.set(Edge::Fall, 1.0);
        assert!(close(u.fall, 1.0));
        assert!(close(t.max_with(u).rise, 2.0));
        assert!(close(t.min_with(u).rise, 0.0));
        assert_eq!(Edge::Rise.opposite(), Edge::Fall);
        assert_eq!(Edge::Rise.as_str(), "r");
        assert_eq!(Sense::Positive.input_edges(Edge::Rise), &[Edge::Rise]);
        assert_eq!(Sense::Negative.input_edges(Edge::Rise), &[Edge::Fall]);
        assert_eq!(Sense::NonUnate.input_edges(Edge::Fall).len(), 2);
        assert_eq!(Sense::NonUnate.as_str(), "non-unate");
        assert!(close(ArcResult::ZERO.max_delay.rise, 0.0));
    }

    #[test]
    fn unit_model_is_one_per_cell_arc() {
        let m = module();
        let model = UnitModel::new().with_setup(0.2).with_hold(0.1);
        assert_eq!(model.name(), "unit");
        let query = ArcQuery {
            module: &m,
            cell: None,
            cell_type: "and",
            from_port: "a",
            to_port: "y",
            input_slew: Transition::ZERO,
            load: 0.0,
            fanout: 1,
            sense: Sense::Positive,
        };
        let arc = model.cell_arc(&query);
        assert!(close(arc.max_delay.rise, 1.0));
        assert!(close(arc.min_delay.fall, 1.0));
        // A flip-flop's clock-to-Q uses its own number.
        let seq = ArcQuery {
            cell_type: "dff",
            from_port: "clk",
            to_port: "q",
            ..query
        };
        let model = model.with_clock_to_q(0.4);
        assert!(close(model.cell_arc(&seq).max_delay.rise, 0.4));
        // Net arcs are free by default.
        let net = NetQuery {
            module: &m,
            net: crate::ir::NetId(0),
            driver_type: "and",
            fanout: 2,
            load: 0.0,
            input_slew: Transition::both(0.3),
        };
        assert!(close(model.net_arc(&net).max_delay.rise, 0.0));
        assert!(close(model.net_arc(&net).slew.rise, 0.3));
        assert!(close(
            model.with_net_delay(0.5).net_arc(&net).max_delay.rise,
            0.5
        ));
        assert!(close(
            model.pin_capacitance(&PinQuery {
                module: &m,
                cell: None,
                cell_type: "and",
                port: "a",
            }),
            0.0
        ));
    }

    #[test]
    fn unit_model_constraints() {
        let m = module();
        let b = {
            let mut b = ModuleBuilder::new("m2", span());
            let clk = b.input("clk", Type::bit());
            let d = b.input("d", Type::bit());
            let q = b.output("q", Type::bits(1));
            let (ce, de) = (b.net(clk), b.net(d));
            b.cell(
                "ff",
                crate::ir::CellKind::Dff {
                    clk_pos: true,
                    has_enable: false,
                    reset: None,
                },
                vec![("clk".into(), ce), ("d".into(), de)],
                vec![("q".into(), q)],
            );
            b.finish()
        };
        let cell = b.cells.values().next().unwrap();
        let model = UnitModel::new().with_setup(0.25).with_hold(0.1);
        let c = model.constraint(&ConstraintQuery {
            module: &m,
            cell,
            cell_type: "dff",
            data_port: "d",
            clock_port: "clk",
            data_slew: Transition::ZERO,
            clock_slew: Transition::ZERO,
            clock_rising: true,
        });
        assert!(close(c.setup.rise, 0.25));
        assert!(close(c.hold.fall, 0.1));
    }

    #[test]
    fn fpga_model_table_and_fallback() {
        let m = module();
        let model = FpgaModel::placeholders();
        assert_eq!(model.name(), "fpga");
        assert!(model.primitive("sb_lut4").is_some());
        assert!(model.primitive("nothing").is_none());
        assert!(model.primitives().count() >= 20);
        let lut = ArcQuery {
            module: &m,
            cell: None,
            cell_type: "SB_LUT4",
            from_port: "I0",
            to_port: "O",
            input_slew: Transition::ZERO,
            load: 0.0,
            fanout: 1,
            sense: Sense::NonUnate,
        };
        assert!(close(model.cell_arc(&lut).max_delay.rise, 0.45));
        // Unknown primitives fall through to the unit model.
        let unknown = ArcQuery {
            cell_type: "mystery",
            ..lut
        };
        assert!(close(model.cell_arc(&unknown).max_delay.rise, 1.0));
        let ff = ArcQuery {
            cell_type: "SB_DFF",
            from_port: "C",
            to_port: "Q",
            ..lut
        };
        assert!(close(model.cell_arc(&ff).max_delay.rise, 0.55));
        let net = NetQuery {
            module: &m,
            net: crate::ir::NetId(0),
            driver_type: "SB_LUT4",
            fanout: 3,
            load: 0.0,
            input_slew: Transition::ZERO,
        };
        assert!(close(model.net_arc(&net).max_delay.rise, 0.90));
        let net = NetQuery {
            driver_type: "mystery",
            ..net
        };
        assert!(close(model.net_arc(&net).max_delay.rise, 0.0));
        // A replaced entry wins, and the label is configurable.
        let model = model
            .with_primitive("SB_LUT4", PrimitiveTiming::comb(0.1, 0.2))
            .with_label("ice40");
        assert!(close(model.cell_arc(&lut).max_delay.rise, 0.1));
        assert_eq!(model.name(), "ice40");
        assert!(close(
            FpgaModel::empty(UnitModel::default())
                .cell_arc(&lut)
                .max_delay
                .rise,
            1.0
        ));
    }

    #[cfg(feature = "asic")]
    mod liberty {
        use super::super::*;
        use super::{close, span};
        use crate::asic::liberty::Library;
        use crate::diag::Diagnostics;
        use crate::ir::builder::ModuleBuilder;
        use crate::ir::types::Type;
        use crate::ir::{CellKind, Name};
        use crate::source::SourceMap;

        const LIB: &str = r#"
library(tiny) {
  capacitive_load_unit (1, pf);
  default_input_pin_cap : 0.004;
  lu_table_template(d2) {
    variable_1 : input_net_transition;
    variable_2 : total_output_net_capacitance;
    index_1 ("0.01, 0.20");
    index_2 ("0.001, 0.050");
  }
  lu_table_template(c2) {
    variable_1 : constrained_pin_transition;
    variable_2 : related_pin_transition;
    index_1 ("0.01, 0.20");
    index_2 ("0.01, 0.20");
  }
  cell(INVX1) {
    pin(A) { direction : input; capacitance : 0.007; }
    pin(Y) {
      direction : output;
      function : "!A";
      timing() {
        related_pin : "A";
        timing_sense : negative_unate;
        cell_rise(d2) { values("0.020, 0.120", "0.040, 0.140"); }
        cell_fall(d2) { values("0.010, 0.060", "0.020, 0.070"); }
        rise_transition(d2) { values("0.030, 0.150", "0.050, 0.170"); }
        fall_transition(d2) { values("0.015, 0.075", "0.025, 0.085"); }
      }
    }
  }
  cell(DFFX1) {
    ff(IQ, IQN) { clocked_on : "CK"; next_state : "D"; }
    pin(CK) { direction : input; clock : true; capacitance : 0.006; }
    pin(D) {
      direction : input;
      capacitance : 0.004;
      timing() {
        related_pin : "CK";
        timing_type : setup_rising;
        rise_constraint(c2) { values("0.080, 0.090", "0.095, 0.110"); }
        fall_constraint(c2) { values("0.070, 0.080", "0.085, 0.100"); }
      }
      timing() {
        related_pin : "CK";
        timing_type : hold_rising;
        rise_constraint(c2) { values("0.020, 0.025", "0.030, 0.035"); }
        fall_constraint(c2) { values("0.010, 0.015", "0.020, 0.025"); }
      }
    }
    pin(Q) {
      direction : output;
      function : "IQ";
      timing() {
        related_pin : "CK";
        timing_type : rising_edge;
        cell_rise(d2) { values("0.200, 0.320", "0.215, 0.340"); }
        cell_fall(d2) { values("0.180, 0.290", "0.195, 0.310"); }
        rise_transition(d2) { values("0.030, 0.160", "0.040, 0.175"); }
        fall_transition(d2) { values("0.025, 0.130", "0.035, 0.145"); }
      }
    }
  }
}
"#;

        fn library() -> Library {
            let mut map = SourceMap::new();
            let file = map.add("tiny.lib", LIB).unwrap();
            let mut diags = Diagnostics::new();
            let lib = Library::parse(LIB, file, &mut diags).expect("the library parses");
            assert!(!diags.has_errors(), "{}", diags.render(&map));
            lib
        }

        fn module() -> crate::ir::Module {
            let mut b = ModuleBuilder::new("m", span());
            let a = b.input("a", Type::bit());
            let y = b.output("y", Type::bit());
            let ae = b.net(a);
            b.cell(
                "inv1",
                CellKind::Blackbox(Name::new("INVX1")),
                vec![("A".into(), ae)],
                vec![("Y".into(), y)],
            );
            b.finish()
        }

        #[test]
        fn looks_delays_up_through_the_tables() {
            let lib = library();
            let model = LibertyModel::new(&lib);
            assert_eq!(model.name(), "liberty(tiny)");
            let m = module();
            let cell = m.cells.values().next().unwrap();
            let query = ArcQuery {
                module: &m,
                cell: Some(cell),
                cell_type: "INVX1",
                from_port: "A",
                to_port: "Y",
                input_slew: Transition::both(0.01),
                load: 0.001,
                fanout: 1,
                sense: Sense::NonUnate,
            };
            // The corner of the table, so the value is exact.
            let arc = model.cell_arc(&query);
            assert_eq!(arc.sense, Sense::Negative);
            assert!(close(arc.max_delay.rise, 0.020));
            assert!(close(arc.max_delay.fall, 0.010));
            assert!(close(arc.slew.rise, 0.030));
            // The middle of both axes: the bilinear average of the four
            // corners, (0.020 + 0.120 + 0.040 + 0.140) / 4 = 0.080.
            let mid = ArcQuery {
                input_slew: Transition::both(0.105),
                load: 0.0255,
                ..query
            };
            assert!(close(model.cell_arc(&mid).max_delay.rise, 0.080));
            // Outside the characterised range the lookup clamps rather
            // than extrapolating into nonsense.
            let outside = ArcQuery {
                input_slew: Transition::both(0.0),
                load: -1.0,
                ..query
            };
            assert!(close(model.cell_arc(&outside).max_delay.rise, 0.020));
            // An unknown cell falls through to the wrapped unit model.
            let unknown = ArcQuery {
                cell_type: "NOTACELL",
                ..query
            };
            assert!(close(model.cell_arc(&unknown).max_delay.rise, 1.0));
            // Pin capacitance comes from the pin, then the default.
            assert!(close(
                model.pin_capacitance(&PinQuery {
                    module: &m,
                    cell: Some(cell),
                    cell_type: "INVX1",
                    port: "A",
                }),
                0.007
            ));
            assert!(close(
                model.pin_capacitance(&PinQuery {
                    module: &m,
                    cell: Some(cell),
                    cell_type: "INVX1",
                    port: "NOPIN",
                }),
                0.004
            ));
        }

        #[test]
        fn reads_setup_and_hold_from_the_constraint_tables() {
            let lib = library();
            let model = LibertyModel::new(&lib);
            let m = module();
            let cell = m.cells.values().next().unwrap();
            let query = ConstraintQuery {
                module: &m,
                cell,
                cell_type: "DFFX1",
                data_port: "D",
                clock_port: "CK",
                data_slew: Transition::both(0.01),
                clock_slew: Transition::both(0.01),
                clock_rising: true,
            };
            let c = model.constraint(&query);
            assert!(close(c.setup.rise, 0.080));
            assert!(close(c.setup.fall, 0.070));
            assert!(close(c.hold.rise, 0.020));
            assert!(close(c.hold.fall, 0.010));
            // An unknown cell falls back.
            let unknown = ConstraintQuery {
                cell_type: "NOTACELL",
                ..query
            };
            assert!(close(
                model
                    .with_fallback(UnitModel::new().with_setup(0.5))
                    .constraint(&unknown)
                    .setup
                    .rise,
                0.5
            ));
        }

        #[test]
        fn describes_the_shape_of_a_library_cell() {
            let lib = library();
            let model = LibertyModel::new(&lib);
            let inv = model
                .cell_topology("INVX1", &["A".to_owned()], &["Y".to_owned()])
                .expect("the inverter is in the library");
            assert_eq!(
                inv.arcs,
                [("A".to_owned(), "Y".to_owned(), Sense::Negative)]
            );
            assert!(!inv.is_sequential());
            let ff = model
                .cell_topology(
                    "DFFX1",
                    &["CK".to_owned(), "D".to_owned()],
                    &["Q".to_owned()],
                )
                .expect("the flip-flop is in the library");
            // Only the clock reaches Q: the flop breaks the graph.
            assert_eq!(
                ff.arcs,
                [("CK".to_owned(), "Q".to_owned(), Sense::NonUnate)]
            );
            assert_eq!(ff.checked_pins, ["D"]);
            assert_eq!(ff.clock_pin.as_deref(), Some("CK"));
            assert!(ff.is_sequential());
            assert!(model.cell_topology("NOTACELL", &[], &[]).is_none());
            // A mapping lets a primitive keyword stand for a library
            // cell.
            let mapped = model.map("not", "INVX1");
            assert!(mapped.library_cell("not").is_some());
        }

        #[test]
        fn the_name_heuristic_covers_primitives_with_no_library() {
            let comb =
                topology_from_pin_names(&["I0".to_owned(), "I1".to_owned()], &["O".to_owned()]);
            assert_eq!(comb.arcs.len(), 2);
            assert!(!comb.is_sequential());
            let ff = topology_from_pin_names(
                &["C".to_owned(), "D".to_owned(), "E".to_owned()],
                &["Q".to_owned()],
            );
            assert_eq!(ff.clock_pin.as_deref(), Some("C"));
            assert_eq!(ff.checked_pins, ["D", "E"]);
            assert_eq!(ff.arcs, [("C".to_owned(), "Q".to_owned(), Sense::Positive)]);
        }
    }
}
