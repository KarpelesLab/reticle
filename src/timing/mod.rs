//! Static timing analysis and clock domain crossing analysis.
//!
//! Two analyses over a module in cell form, sharing one view of what a
//! netlist's clocks and flip-flops are:
//!
//! | Module    | What it does                                               |
//! |-----------|------------------------------------------------------------|
//! | [`graph`] | Builds the timing graph: pins as nodes, delay arcs as edges |
//! | [`delay`] | Delay models: unit, Liberty (`asic`), FPGA primitives       |
//! | [`sta`]   | Arrival and required times, slack, path reports             |
//! | [`cdc`]   | Clock domain crossings and how they are (or are not) synchronised |
//!
//! ```no_run
//! # #[cfg(feature = "timing")] {
//! use reticle::timing::delay::UnitModel;
//! use reticle::timing::sta::{ClockSpec, TimingOptions, TimingSpec, analyze_with};
//! # fn go(module: &reticle::ir::Module) {
//! let spec = TimingSpec::new().with_clock(ClockSpec::new("sys", "clk", 10.0));
//! let report = analyze_with(module, &TimingOptions::default(), &UnitModel::new(), &spec);
//! println!("{}", report.render());
//! # }
//! # }
//! ```
//!
//! # The timing model
//!
//! A design is a graph of pins. A **cell arc** goes from one of a cell's
//! input pins to one of its output pins and costs the delay through the
//! cell; a **net arc** goes from the pin driving a net to each pin
//! loading it and costs the interconnect delay. Sequential cells break
//! the graph: a flip-flop has a clock-to-output arc but no data-to-output
//! arc, so its `q` starts paths and its `d` ends them, with setup and
//! hold checked against its own clock pin.
//!
//! Arrival times are propagated forwards over that graph and required
//! times backwards, both **per edge**: every number is a pair of a rising
//! and a falling value, and each arc says which input edge produces which
//! output edge, so an inverter swaps them. Both a **late (max)** and an
//! **early (min)** set are kept, which is what makes the setup and the
//! hold analysis two views of one propagation rather than two passes.
//!
//! Clocks come from `create_clock`. Each is a period and a waveform; the
//! launch and capture edges of a check are the closest pair of active
//! edges with the capture edge after the launch edge, which is what makes
//! a path between two unrelated clocks report the tight relationship
//! rather than a period. The clock network itself is propagated through
//! the same graph, so [`sta::ClockMode::Propagated`] gives real insertion
//! delay and skew while [`sta::ClockMode::Ideal`] zeroes them.
//!
//! Where the numbers come from is a [`delay::DelayModel`]:
//! [`delay::UnitModel`] charges one unit per cell arc and nothing per net
//! (so a report can be checked by hand), [`delay::LibertyModel`] does a
//! real non-linear table lookup against a `.lib` library, and
//! [`delay::FpgaModel`] holds a small table of per-primitive numbers.
//!
//! # What is not modelled
//!
//! Worth stating plainly, because a timing report that hides its
//! assumptions is worse than none:
//!
//! - **Wire RC.** A net is a lumped capacitance and one flat interconnect
//!   delay. There is no resistance, no distributed tree, no Elmore delay
//!   and no difference between a near and a far load on the same net.
//! - **On-chip variation.** There is one delay per arc, scaled by two
//!   flat derating factors ([`sta::TimingOptions::early_derate`] and
//!   [`sta::TimingOptions::late_derate`]). There is no distance-dependent
//!   derating, no common-path pessimism removal and no statistical
//!   analysis.
//! - **Crosstalk.** No coupling capacitance, no aggressor / victim
//!   analysis, no delta delay.
//! - **Design rules.** Maximum transition, maximum capacitance, maximum
//!   fanout and minimum pulse width are not checked.
//! - **Recovery and removal** on asynchronous set and reset pins, **clock
//!   gating checks**, and **latch time borrowing**: a latch is treated as
//!   transparent with a setup check on its data pin.
//! - **Generated clocks.** A clock produced by dividing another with a
//!   flip-flop is not recognised; constrain it with its own
//!   `create_clock` or it becomes a domain named after its net.
//! - **Signal integrity, IR drop and temperature inversion.**
//!
//! The crossing analysis in [`cdc`] is explicit about a second kind of
//! limit: which of its classifications are structural facts and which are
//! guesses it reports as unverified. Its module documentation has the
//! list.

pub mod cdc;
pub mod delay;
pub mod graph;
pub mod sta;

pub use cdc::{CdcReport, Crossing, CrossingKind, Domain, Reconvergence, analyze_cdc_with};
pub use delay::{DelayModel, Edge, FpgaModel, PrimitiveTiming, Sense, Transition, UnitModel};
pub use graph::{Arc, ArcKind, Pin, PinId, TimingGraph, flatten_for_timing};
pub use sta::{
    Check, ClockMode, ClockSpec, ExceptionKind, PathException, TimingOptions, TimingPath,
    TimingReport, TimingSpec, analyze_with,
};

#[cfg(feature = "asic")]
pub use delay::LibertyModel;

#[cfg(feature = "fpga")]
pub use cdc::analyze_cdc;
#[cfg(feature = "fpga")]
pub use sta::analyze;
