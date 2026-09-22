//! The public surface of compiled fast mode: [`Plan`] and [`CompiledSim`].
//!
//! [`CompiledSim`] deliberately mirrors the parts of
//! [`Simulator`](crate::sim::Simulator) that have a cycle-based meaning,
//! with the same names and the same [`NetHandle`] type, so a testbench can
//! be pointed at either engine with almost no edits. The [module
//! docs](super) hold the table of what each method means next to the event
//! simulator's, and what compiled mode does not do at all.
//!
//! [`Plan`] is the step in between: a design that has been proved eligible
//! and lowered, but not yet run. Keeping it separate is what lets a caller
//! look at [`Plan::stats`] and [`Plan::clock`] before paying for a run,
//! and lets a test line the two engines up through [`Plan::state_nets`].

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, Polarity, ReportSeverity};
use crate::logic::Logic;
use crate::sim::elab::SigId;
use crate::sim::{MemHandle, NetHandle, Simulator, Status, Value};

use super::check::{CompileOptions, Ineligible, check};
use super::engine::{Effect, State};
use super::prog::{MsgArg, Program, ProgramStats, Slot};

/// A design that has been proved eligible and lowered to a straight-line
/// program.
///
/// Splitting this from [`CompiledSim`] lets a caller inspect what the
/// lowering produced ([`Plan::stats`]), and which net turned out to be the
/// clock ([`Plan::clock`]), before paying for a run.
pub struct Plan<'d> {
    pub(crate) sim: Simulator<'d>,
    pub(crate) prog: Program,
    /// Where each signal's settled value lives, by [`SigId`] index.
    pub(crate) sig_slot: Vec<Option<Slot>>,
    /// Whether a signal owns its slot, so writing it is meaningful.
    pub(crate) settable: Vec<bool>,
    pub(crate) state_sigs: Vec<SigId>,
    pub(crate) clock: Option<(SigId, Polarity)>,
    pub(crate) stats: ProgramStats,
    pub(crate) output: String,
}

impl std::fmt::Debug for Plan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("top", &self.sim.top_name())
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl<'d> Plan<'d> {
    /// What the lowering produced.
    pub fn stats(&self) -> ProgramStats {
        self.stats
    }

    /// The design's single clock, or `None` for a design with no clocked
    /// storage at all.
    pub fn clock(&self) -> Option<NetHandle> {
        self.clock.map(|(s, _)| NetHandle(s))
    }

    /// True when the clock's active edge is the rising one.
    pub fn clock_rising(&self) -> Option<bool> {
        self.clock.map(|(_, p)| p == Polarity::Pos)
    }

    /// The name of the top instance.
    pub fn top_name(&self) -> &str {
        self.sim.top_name()
    }

    /// Every state element (register, registered memory read) with its
    /// hierarchical name.
    ///
    /// Useful for lining a compiled run up with an event-driven one: these
    /// are the nets whose value has to match after every cycle.
    pub fn state_nets(&self) -> Vec<(String, NetHandle)> {
        self.state_sigs
            .iter()
            .map(|s| (self.sim.signals[s.idx()].name.clone(), NetHandle(*s)))
            .collect()
    }

    /// Turns the plan into a runnable engine.
    pub fn compile(self) -> CompiledSim<'d> {
        let state = State::new(&self.prog);
        let output = self.output.clone();
        CompiledSim {
            plan: self,
            state,
            cycle: 0,
            dirty: true,
            output,
            messages: Diagnostics::new(),
            status: Status::Running,
        }
    }
}

/// A cycle-based, two-state simulation of one design.
///
/// See the [module docs](super) for the model, its limits and the table
/// of what each method means next to the event simulator's.
pub struct CompiledSim<'d> {
    plan: Plan<'d>,
    state: State,
    cycle: u64,
    /// True when an input changed since the last settle.
    dirty: bool,
    output: String,
    messages: Diagnostics,
    status: Status,
}

impl std::fmt::Debug for CompiledSim<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledSim")
            .field("cycle", &self.cycle)
            .field("status", &self.status)
            .field("stats", &self.plan.stats)
            .finish_non_exhaustive()
    }
}

impl<'d> CompiledSim<'d> {
    /// Checks `design` and compiles it.
    ///
    /// # Errors
    ///
    /// Returns every reason the design does not qualify; see [`check`].
    pub fn new(
        design: &'d Design,
        options: CompileOptions,
    ) -> Result<CompiledSim<'d>, Vec<Ineligible>> {
        Ok(check(design, options)?.compile())
    }

    /// The design being simulated.
    pub fn design(&self) -> &'d Design {
        self.plan.sim.design()
    }

    /// What the lowering produced.
    pub fn stats(&self) -> ProgramStats {
        self.plan.stats
    }

    /// The design's clock, or `None` for a purely combinational design.
    pub fn clock(&self) -> Option<NetHandle> {
        self.plan.clock()
    }

    /// The name of the top instance, the first component of every
    /// hierarchical name.
    pub fn top_name(&self) -> &str {
        self.plan.sim.top_name()
    }

    /// Looks up a net by hierarchical name, `top.inst.net`.
    pub fn net(&self, path: &str) -> Option<NetHandle> {
        self.plan.sim.net(path)
    }

    /// Looks up a memory by hierarchical name.
    pub fn memory(&self, path: &str) -> Option<MemHandle> {
        self.plan.sim.memory(path)
    }

    /// Every net with its hierarchical name, in hierarchy order.
    pub fn nets(&self) -> Vec<(String, NetHandle)> {
        self.plan.sim.nets()
    }

    /// Every memory with its hierarchical name, in hierarchy order.
    pub fn memories(&self) -> Vec<(String, MemHandle)> {
        self.plan.sim.memories()
    }

    /// Every state element with its hierarchical name.
    pub fn state_nets(&self) -> Vec<(String, NetHandle)> {
        self.plan.state_nets()
    }

    /// The hierarchical name of a net.
    pub fn net_name(&self, net: NetHandle) -> &str {
        self.plan.sim.net_name(net)
    }

    /// The hierarchical path of every instance, top first.
    pub fn instance_paths(&self) -> Vec<String> {
        self.plan.sim.instance_paths()
    }

    /// The number of clock cycles run so far.
    ///
    /// This is the compiled engine's clock: there are no ticks, so a
    /// testbench that needs wall-clock times keeps them itself.
    pub fn time(&self) -> u64 {
        self.cycle
    }

    /// The run state; `Finished` after `$finish`, `Stopped` after `$stop`.
    pub fn status(&self) -> Status {
        self.status
    }

    /// True once `$finish` or a failing assertion ended the run.
    pub fn finished(&self) -> bool {
        self.status == Status::Finished
    }

    /// Settles the combinational region if an input has changed.
    ///
    /// [`CompiledSim::get`] and [`CompiledSim::step`] do this for you; it
    /// is public for a caller that wants the cost at a known place.
    pub fn settle(&mut self) {
        if self.dirty {
            let CompiledSim { plan, state, .. } = self;
            state.run(&plan.prog, &plan.prog.comb);
            self.dirty = false;
        }
    }

    /// The current value of a net, settling first if an input changed.
    pub fn get(&mut self, net: NetHandle) -> Logic {
        self.settle();
        match self.plan.sig_slot[net.0.idx()] {
            Some(slot) => self.state.value(slot),
            None => Logic::zero(0),
        }
    }

    /// Writes a net.
    ///
    /// Writing an input or a register takes effect at the next settle.
    /// Writing a combinationally driven net is allowed — the event
    /// simulator allows it too — but the next settle overwrites it.
    pub fn set(&mut self, net: NetHandle, value: Logic) {
        if let Some(slot) = self.plan.sig_slot[net.0.idx()] {
            self.state.set_value(slot, &value);
            self.dirty = true;
        }
    }

    /// True when the net owns storage, so [`CompiledSim::set`] sticks: an
    /// input, or a register between edges.
    pub fn is_settable(&self, net: NetHandle) -> bool {
        self.plan.settable[net.0.idx()]
    }

    /// One element of a memory, or `None` when out of range.
    pub fn get_mem(&self, mem: MemHandle, index: u64) -> Option<Logic> {
        let i = usize::try_from(index).ok()?;
        let layout = self.plan.prog.mems.get(mem.0.idx())?;
        self.state.mem_value(mem.0.idx(), layout, i)
    }

    /// Writes one element of a memory; false when out of range.
    pub fn set_mem(&mut self, mem: MemHandle, index: u64, value: Logic) -> bool {
        let Ok(i) = usize::try_from(index) else {
            return false;
        };
        let Some(layout) = self.plan.prog.mems.get(mem.0.idx()).cloned() else {
            return false;
        };
        self.state.set_mem_value(mem.0.idx(), &layout, i, &value)
    }

    /// The number of elements of a memory.
    pub fn mem_len(&self, mem: MemHandle) -> usize {
        self.plan.prog.mems.get(mem.0.idx()).map_or(0, |m| m.len)
    }

    /// Runs one clock cycle: settle, compute the next state, commit.
    ///
    /// Returns false when the run is over (`$finish` or `$stop`).
    pub fn step(&mut self) -> bool {
        if self.status == Status::Stopped {
            self.status = Status::Running;
        }
        if self.status != Status::Running {
            return false;
        }
        self.settle();
        {
            let CompiledSim { plan, state, .. } = self;
            state.effects.clear();
            state.run(&plan.prog, &plan.prog.seq);
        }
        self.drain_effects();
        {
            let CompiledSim { plan, state, .. } = self;
            state.commit(&plan.prog);
        }
        self.cycle += 1;
        self.dirty = true;
        true
    }

    /// Runs `n` clock cycles, stopping early on `$finish` or `$stop`.
    ///
    /// Returns the number of cycles actually run.
    pub fn run_cycles(&mut self, n: u64) -> u64 {
        let mut done = 0;
        while done < n {
            if !self.step() {
                break;
            }
            done += 1;
            // The cycle that executed `$stop` still counts; the next call
            // resumes, as it does in the event simulator.
            if self.status != Status::Running {
                break;
            }
        }
        done
    }

    /// The text `$display` and friends produced, including anything the
    /// time-zero initialisers printed.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Takes the output text, leaving it empty.
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }

    /// Runtime diagnostics: failing assertions, and the warnings
    /// elaboration produced.
    pub fn messages(&self) -> &Diagnostics {
        &self.messages
    }

    /// Takes the diagnostics, leaving none.
    pub fn take_messages(&mut self) -> Diagnostics {
        std::mem::take(&mut self.messages)
    }

    /// Formats one message template against the current register file.
    fn format(&self, msg: u32) -> String {
        let template = &self.plan.prog.messages[msg as usize];
        let vals: Vec<Value> = template
            .args
            .iter()
            .map(|a| match a {
                MsgArg::Str(s) => Value::Str(s.clone()),
                MsgArg::Val(slot) => Value::Bits(self.state.value(*slot)),
            })
            .collect();
        self.plan
            .sim
            .format_values(template.inst, &vals, template.radix)
    }

    fn drain_effects(&mut self) {
        if self.state.effects.is_empty() {
            return;
        }
        let effects = std::mem::take(&mut self.state.effects);
        for effect in &effects {
            match effect {
                Effect::Display(msg) => {
                    let text = self.format(*msg);
                    self.output.push_str(&text);
                    if self.plan.prog.messages[*msg as usize].newline {
                        self.output.push('\n');
                    }
                }
                Effect::Assert(msg, severity, span) => {
                    let text = self.format(*msg);
                    let text = if text.is_empty() {
                        "assertion failed".to_owned()
                    } else {
                        text
                    };
                    let diag = match severity {
                        ReportSeverity::Note | ReportSeverity::Warning => Diagnostic::warning(text),
                        _ => Diagnostic::error(text),
                    };
                    self.messages.push(diag.with_span(*span));
                    if *severity == ReportSeverity::Failure {
                        self.status = Status::Finished;
                    }
                }
                Effect::Finish => self.status = Status::Finished,
                Effect::Stop => {
                    if self.status == Status::Running {
                        self.status = Status::Stopped;
                    }
                }
            }
        }
        self.state.effects = effects;
        self.state.effects.clear();
    }
}
