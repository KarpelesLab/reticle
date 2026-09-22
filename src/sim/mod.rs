//! Event-driven simulator over the IR's process form.
//!
//! The simulator takes a hierarchical [`Design`], flattens it into an
//! instance tree, and runs its processes, continuous assignments and cells
//! under one scheduler that implements both the Verilog stratified event
//! queue and the VHDL delta cycle. It is 4-state throughout ([`Logic`]),
//! sans-I/O (`$display` text and VCD data are accumulated and handed back
//! as strings; `$readmemh` reads through a caller-supplied
//! [`FileProvider`]), and drivable from Rust through [`Simulator`], which is
//! how testbenches are written as `#[test]`s.
//!
//! # Layout
//!
//! | File          | Role                                                         |
//! |---------------|--------------------------------------------------------------|
//! | `elab.rs`     | Flattening: instances, signals, port aliasing, drivers, fanout |
//! | `value.rs`    | Net storage, wire resolution, bit selection helpers          |
//! | `eval.rs`     | Expression evaluation over the IR arena                      |
//! | `sched.rs`    | The scheduler: regions, time wheel, signal propagation       |
//! | `process.rs`  | Process execution as a resumable frame stack                 |
//! | `sys.rs`      | System tasks, `$display` formatting, `$random`, `$readmem*`  |
//! | `vcd.rs`      | VCD waveform writer                                          |
//! | `api.rs`      | The public co-simulation API                                 |
//! | `assertion/`  | Concurrent assertions: an SVA / PSL subset as automata       |
//! | `coverage.rs` | Line and toggle coverage, rendered as text or LCOV           |
//! | `interactive.rs` | The sans-I/O command session behind interactive mode      |
//!
//! # Elaboration
//!
//! [`Simulator::new`] walks the hierarchy from the top module. Every net of
//! every instance becomes a *signal*, except that a port connected to a
//! plain net of the parent shares the parent's signal: the alias costs
//! nothing at run time. A port connected to anything else (a slice, a
//! concatenation, an expression) becomes an implicit continuous assignment
//! in the parent's context. Continuous assigns, combinational cells and
//! implicit port assigns are *drivers*; a signal with several drivers is
//! resolved with the Verilog `wire` rules (`z` yields, conflicts give
//! `x`), while `reg` and variable nets take the last write. Elaboration
//! also builds the *fanout map* from every signal (and memory) to the
//! drivers, cells and processes that read it, so a change schedules exactly
//! its consumers.
//!
//! # Scheduler
//!
//! Time is a `u64` count of the finest precision found among the design's
//! `timescale`s (1 ps when none is given). One *time slot* runs the regions
//! of IEEE 1364-2005 §11.4 to quiescence:
//!
//! 1. **Active**: process activations, driver and cell evaluations and
//!    blocking updates, in FIFO order. A signal change here schedules its
//!    fanout into the same region.
//! 2. **Inactive**: `#0` continuations; moved to active once it empties.
//! 3. **NBA**: non-blocking assignments (`<=`), `Dff` outputs and
//!    `MemWrite`s collected during the slot, applied in program order once
//!    active and inactive are empty. Their signal changes refill the active
//!    region, so the loop repeats.
//! 4. **Monitor**: `$strobe` and `$monitor` output, once per slot after
//!    everything else has settled.
//!
//! One pass through 1–3 is exactly a VHDL delta cycle (IEEE 1076-2008
//! §14.7.5): signal updates are non-blocking, processes resume on the
//! signals they are sensitive to, and the slot ends when no delta produces
//! a further event. Both languages therefore share one loop; the only
//! per-language choice is made by the frontend when it lowers an
//! assignment as `Blocking` or `NonBlocking`. After the slot the scheduler
//! advances to the earliest pending timed event (delays, `wait for`,
//! transport-delayed drivers) and starts the next slot.
//!
//! Process triggering follows the process kind: `Comb` runs on any change
//! of a net its body reads (computed statically), `Sensitive` on any change
//! of its list, `Sequential` on the listed edges with the edge definitions
//! of §9.7.1 (posedge is `0→1`, `0→x/z` or `x/z→1`; negedge is symmetric),
//! `Initial` once at time 0, and `Free` at time 0, restarting from the top
//! when its body ends (as an `always` block or a VHDL process without a
//! sensitivity list does). A process that runs too many statements without
//! suspending is stopped with an error, as is a slot that never settles.
//!
//! # Processes
//!
//! A process is executed by an interpreter over the IR statements with an
//! explicit frame stack: each frame is a block reference, a program counter
//! and, for loops, the loop state. A `Wait` or a delayed assignment saves
//! the stack and returns to the scheduler; resumption continues from the
//! saved frame. No OS threads and no statement lowering are involved. See
//! `process.rs`.
//!
//! # Co-simulation
//!
//! A test builds or parses a design, creates a [`Simulator`], looks up nets
//! by hierarchical name ([`Simulator::net`]), drives inputs with
//! [`Simulator::set`], advances time with [`Simulator::run_for`] and reads
//! outputs with [`Simulator::get`]. `$display` text accumulates in
//! [`Simulator::output`], runtime problems in [`Simulator::messages`], and
//! [`Simulator::enable_vcd`] starts a waveform capture that
//! [`Simulator::vcd`] returns as text. [`Simulator::enable_fst`] captures
//! the same changes for [`Simulator::dump_fst`] to write in GTKWave's
//! compressed FST format; see [`fst`].
//!
//! # Assertions
//!
//! Immediate assertions are statements and run in `process.rs`. *Concurrent*
//! assertions live in [`assertion`]: a property over a clocking event,
//! compiled to an automaton over boolean predicates and evaluated once per
//! clock edge with one attempt started per cycle. The supported subset of
//! SVA and PSL — and what is deliberately left out — is listed in that
//! module's docs. [`Simulator::add_assertion`] and
//! [`Simulator::add_assertion_text`] add one; [`Simulator::assertion_results`]
//! reports what happened.
//!
//! # Coverage
//!
//! With [`SimOptions::coverage`] on, the run records how often each
//! statement executed and, per net bit, whether it was seen going `0` to
//! `1` and `1` to `0`. [`Simulator::coverage`] returns a
//! [`CoverageReport`], which renders as text or as LCOV `.info`. With the
//! flag off nothing is allocated and nothing is recorded; see
//! [`coverage`].
//!
//! # Interactive mode
//!
//! [`interactive::Session`] is a command interpreter over a [`Simulator`]:
//! `run`, `step`, `break`, `force`, `print`, `watch`, `dump` and the rest,
//! taking a command line and returning the text to show, so the same
//! session drives a terminal or a test. Breakpoints are part of the
//! simulator itself ([`Simulator::add_breakpoint`]) and stop a run at the
//! end of the time slot the change happened in.
//!
//! # Not yet here
//!
//! The cycle-based fast mode and inertial delay on continuous assignments
//! (transport delay is used) are later roadmap items.

use std::collections::{BTreeMap, VecDeque};

use crate::diag::Diagnostics;
use crate::ir::{Design, ExprId};
use crate::logic::Logic;

mod api;
pub mod assertion;
pub mod coverage;
mod elab;
mod eval;
pub mod fst;
pub mod interactive;
mod process;
mod sched;
mod sys;
mod value;
mod vcd;

pub use api::{BreakId, MemHandle, NetHandle};
pub use assertion::{AssertionId, AssertionResult};
pub use coverage::{CoverageReport, LineRecord, ToggleRecord};
pub use interactive::{Response, Session, SessionError};
pub use sys::{FileProvider, MemoryFiles};
pub use value::Value;

use elab::{CellState, Driver, InstanceState, MemState, ProcId};
use process::ProcState;
use sched::{Event, Timed};
use sys::TimeFormat;
use vcd::VcdWriter;

/// Whether the simulation is running, paused or over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Events may still be processed.
    Running,
    /// `$stop` was executed; the next run call resumes.
    Stopped,
    /// `$finish` was executed or a failing assertion ended the run.
    Finished,
}

/// Configuration for [`Simulator::new`].
pub struct SimOptions {
    /// The module to instantiate as the root; defaults to the design's top
    /// or, failing that, the only module no instance refers to.
    pub top: Option<String>,
    /// Seed of the `$random` generator.
    pub seed: u64,
    /// Statements one process activation may execute before it is
    /// considered stuck and killed with an error.
    pub max_process_steps: u64,
    /// Events one time slot may process before the run is stopped as a
    /// zero-delay oscillation.
    pub max_slot_events: u64,
    /// Files for `$readmemh` / `$readmemb`; `None` makes every read fail
    /// with a diagnostic.
    pub files: Option<Box<dyn FileProvider>>,
    /// Collect line and toggle coverage ([`Simulator::coverage`]).
    ///
    /// Off by default: an uninstrumented run allocates no tables and does
    /// no accounting.
    pub coverage: bool,
}

impl Default for SimOptions {
    fn default() -> Self {
        SimOptions {
            top: None,
            seed: 1,
            max_process_steps: 10_000_000,
            max_slot_events: 1_000_000,
            files: None,
            coverage: false,
        }
    }
}

impl std::fmt::Debug for SimOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimOptions")
            .field("top", &self.top)
            .field("seed", &self.seed)
            .field("max_process_steps", &self.max_process_steps)
            .field("max_slot_events", &self.max_slot_events)
            .field("files", &self.files.is_some())
            .field("coverage", &self.coverage)
            .finish()
    }
}

/// A callback registered with [`Simulator::on_change`].
type ChangeCallback = Box<dyn FnMut(u64, &Logic)>;

/// A running simulation of one design.
///
/// See the [module docs](self) for the model and `api.rs` for the methods.
pub struct Simulator<'d> {
    design: &'d Design,
    options: SimOptions,
    /// Femtoseconds per time tick.
    precision_fs: u64,
    instances: Vec<InstanceState<'d>>,
    signals: Vec<value::Signal>,
    sig_fanout: Vec<Vec<elab::Fanout>>,
    sig_drivers: Vec<Vec<elab::DriverId>>,
    sig_waiters: Vec<Vec<(ProcId, u32)>>,
    sig_callbacks: Vec<Vec<usize>>,
    callbacks: Vec<ChangeCallback>,
    memories: Vec<MemState>,
    drivers: Vec<Driver>,
    cells: Vec<CellState<'d>>,
    procs: Vec<ProcState<'d>>,
    now: u64,
    active: VecDeque<Event>,
    inactive: VecDeque<Event>,
    nba: Vec<(Vec<value::Store>, Logic)>,
    timed: BTreeMap<u64, Vec<Timed>>,
    strobes: Vec<(elab::InstId, Vec<ExprId>)>,
    monitor: Option<sys::Monitor>,
    status: Status,
    output: String,
    messages: Diagnostics,
    vcd: Option<VcdWriter>,
    rng: u64,
    time_format: TimeFormat,
    warned_calls: Vec<String>,
    assertions: Vec<assertion::runtime::Checker>,
    /// Signals any assertion reads, sorted, with their sampled values.
    assert_watch: Vec<elab::SigId>,
    assert_sample: Vec<Logic>,
    /// Clock signals of the assertions, sorted.
    assert_clocks: Vec<elab::SigId>,
    coverage: Option<Box<coverage::Coverage>>,
    breaks: Vec<api::Breakpoint>,
    break_hits: Vec<u32>,
    next_break: u32,
}

impl std::fmt::Debug for Simulator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulator")
            .field("time", &self.now)
            .field("status", &self.status)
            .field("instances", &self.instances.len())
            .field("signals", &self.signals.len())
            .field("processes", &self.procs.len())
            .field("assertions", &self.assertions.len())
            .finish_non_exhaustive()
    }
}
