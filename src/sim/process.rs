//! Process execution as a resumable interpreter.
//!
//! A process is run by an interpreter over the IR statements that keeps
//! its position in an explicit stack of [`Frame`]s: each frame is a
//! reference to a block, a program counter into it and, for loops, the
//! loop's state. Executing a statement either advances the counter, pushes
//! a frame (a nested block, a loop body, a taken `if` branch), unwinds to
//! the innermost loop (`break` / `continue`), or *suspends*: a `wait`, a
//! delayed assignment and `$stop` save the stack as it is and return to the
//! scheduler, which resumes the process later by calling
//! [`Simulator::run_process`] again. This gives every process its own
//! continuation without OS threads, without lowering the structured
//! statements into a flat program, and with a saved state that is a few
//! words per nesting level.
//!
//! Process kinds map onto the same machinery: `Initial` runs its body once
//! and dies; `Free` restarts from the top when the body ends (an `always`
//! block or a VHDL process without sensitivity list); the triggered kinds
//! (`Comb`, `Sequential`, `Sensitive`) run to the end and go idle until the
//! scheduler triggers them again. Triggers are ignored while a process is
//! not idle, so a clocked process that contains a delay does not queue
//! edges it is not waiting for, as in Verilog.
//!
//! The statement semantics follow IEEE 1364-2005 §9: blocking assignments
//! update immediately, non-blocking ones go to the NBA region (with their
//! delay honoured through the time wheel), `if` with an unknown condition
//! takes the `else` branch, `case` compares with `===` and `casez`/`casex`
//! with their wildcard rules, `unique`/`priority` violations are warnings,
//! `MemWrite` behaves like a non-blocking write to the element, and an
//! `Assert` that fails reports with its severity (`Failure` ends the run).
//! A process that executes more than `SimOptions::max_process_steps`
//! statements without suspending is killed with an error.

use crate::diag::Diagnostic;
use crate::ir::{
    AssignKind, Block, CaseKind, CaseQualifier, Edge, ExprId, Lvalue, Polarity, Process,
    ProcessKind, ReportSeverity, Span, Stmt, StmtKind, WaitKind,
};
use crate::logic::{Bit, Logic};

use super::elab::{InstId, ProcId, SigId, Target, expr_reads};
use super::sched::{Event, Timed, edge_matches};
use super::value::{Store, Value};
use super::{Simulator, Status};

/// Where a process is in its life cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcStatus {
    /// Body finished; waiting for a trigger.
    Idle,
    /// In the active or inactive queue.
    Queued,
    /// Currently executing.
    Running,
    /// Suspended until a timed wake-up.
    Delayed,
    /// Suspended in `wait on` until one of its edges occurs.
    WaitingEvent,
    /// Suspended in `wait until` until its condition holds.
    WaitingUntil,
    /// Finished for good (`initial` done, killed, or `$finish`).
    Dead,
}

impl ProcStatus {
    /// True for the two states that a signal change can wake.
    pub(crate) fn is_waiting(self) -> bool {
        matches!(self, ProcStatus::WaitingEvent | ProcStatus::WaitingUntil)
    }
}

/// The loop state of a frame.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FrameKind<'d> {
    /// A plain nested block.
    Plain,
    /// A `for` loop body: re-run the step and condition at the end.
    For {
        /// The loop condition, if any.
        cond: Option<ExprId>,
        /// The step assignment, if any.
        step: Option<&'d (Lvalue, ExprId)>,
    },
    /// A `while` loop body.
    While(ExprId),
    /// A `repeat` loop body with the iterations still to run.
    Repeat(u64),
    /// A `forever` loop body.
    Forever,
}

/// One level of the saved continuation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame<'d> {
    /// The block being executed.
    pub(crate) block: &'d Block,
    /// Index of the next statement.
    pub(crate) pc: usize,
    /// Loop state, if the frame is a loop body.
    pub(crate) kind: FrameKind<'d>,
}

/// A process and its saved continuation.
pub(crate) struct ProcState<'d> {
    /// The owning instance.
    pub(crate) inst: InstId,
    /// Hierarchical name for diagnostics.
    pub(crate) name: String,
    /// The IR process.
    pub(crate) process: &'d Process,
    /// The continuation.
    pub(crate) frames: Vec<Frame<'d>>,
    /// Life-cycle state.
    pub(crate) status: ProcStatus,
    /// Incremented on every wait so stale waiter entries are ignored.
    pub(crate) wait_gen: u32,
    /// The edges a `wait on` is blocked on.
    pub(crate) wait_edges: Vec<(SigId, Polarity)>,
    /// The condition a `wait until` is blocked on.
    pub(crate) until: Option<ExprId>,
    /// A blocking assignment to apply when a delayed assignment resumes.
    pub(crate) pending: Option<(Vec<Store>, Logic)>,
}

impl<'d> ProcState<'d> {
    /// A fresh, never-run process.
    pub(crate) fn new(inst: InstId, name: String, process: &'d Process) -> Self {
        ProcState {
            inst,
            name,
            process,
            frames: Vec::new(),
            status: ProcStatus::Idle,
            wait_gen: 0,
            wait_edges: Vec::new(),
            until: None,
            pending: None,
        }
    }

    /// Whether a change of `sig` satisfies what the process waits for.
    pub(crate) fn wait_matches(&self, sig: SigId, old: &Logic, new: &Logic) -> bool {
        match self.status {
            ProcStatus::WaitingUntil => true,
            ProcStatus::WaitingEvent => self
                .wait_edges
                .iter()
                .any(|(s, pol)| *s == sig && edge_matches(*pol, old, new)),
            _ => false,
        }
    }
}

/// What executing one statement asks the interpreter loop to do next.
enum Flow<'d> {
    /// Continue with the next statement.
    Next,
    /// Enter a nested block.
    Push(Frame<'d>),
    /// Leave the innermost loop.
    Break,
    /// Jump to the end of the innermost loop body.
    Continue,
    /// The process suspended; its status is already set.
    Suspend,
}

impl<'d> Simulator<'d> {
    /// Runs or resumes process `pid` until it suspends, finishes its body
    /// or is killed.
    pub(crate) fn run_process(&mut self, pid: ProcId) {
        let inst = self.procs[pid.idx()].inst;
        if self.procs[pid.idx()].status == ProcStatus::Dead {
            return;
        }
        self.procs[pid.idx()].status = ProcStatus::Running;
        if let Some(e) = self.procs[pid.idx()].until.take()
            && self.eval(inst, e).truth() != Bit::One
        {
            self.wait_until(pid, e);
            return;
        }
        if let Some((stores, value)) = self.procs[pid.idx()].pending.take() {
            self.apply_stores(&stores, &value);
        }
        if self.procs[pid.idx()].frames.is_empty() {
            let body = &self.procs[pid.idx()].process.body;
            self.procs[pid.idx()].frames.push(Frame {
                block: body,
                pc: 0,
                kind: FrameKind::Plain,
            });
        }
        let mut steps = 0u64;
        loop {
            steps += 1;
            if steps > self.options.max_process_steps {
                let p = &mut self.procs[pid.idx()];
                p.status = ProcStatus::Dead;
                p.frames.clear();
                let (name, span) = (p.name.clone(), p.process.span);
                self.messages.push(
                    Diagnostic::error(format!(
                        "process `{name}` executed {} statements without suspending and was stopped",
                        self.options.max_process_steps
                    ))
                    .with_label(span, "this process")
                    .with_note("a loop without a delay or wait never yields to the scheduler"),
                );
                return;
            }
            let stmt = {
                let p = &mut self.procs[pid.idx()];
                let Some(frame) = p.frames.last_mut() else {
                    // The body ended.
                    if matches!(p.process.kind, ProcessKind::Free) {
                        let body = &p.process.body;
                        p.frames.push(Frame {
                            block: body,
                            pc: 0,
                            kind: FrameKind::Plain,
                        });
                        continue;
                    }
                    p.status = if matches!(p.process.kind, ProcessKind::Initial) {
                        ProcStatus::Dead
                    } else {
                        ProcStatus::Idle
                    };
                    return;
                };
                if frame.pc >= frame.block.len() {
                    self.end_of_block(pid);
                    continue;
                }
                let stmt: &'d Stmt = &frame.block[frame.pc];
                frame.pc += 1;
                stmt
            };
            match self.exec(pid, inst, stmt) {
                Flow::Next => {}
                Flow::Push(frame) => {
                    if !frame.block.is_empty() {
                        self.procs[pid.idx()].frames.push(frame);
                    }
                }
                Flow::Break => self.unwind_loop(pid, true),
                Flow::Continue => self.unwind_loop(pid, false),
                Flow::Suspend => return,
            }
            if self.status != Status::Running {
                return;
            }
        }
    }

    /// Handles reaching the end of the innermost frame's block.
    fn end_of_block(&mut self, pid: ProcId) {
        let inst = self.procs[pid.idx()].inst;
        let kind = self.procs[pid.idx()]
            .frames
            .last()
            .map(|f| f.kind)
            .expect("frame present");
        let again = match kind {
            FrameKind::Plain => false,
            FrameKind::Forever => true,
            FrameKind::Repeat(left) => {
                if left > 1 {
                    if let Some(f) = self.procs[pid.idx()].frames.last_mut() {
                        f.kind = FrameKind::Repeat(left - 1);
                    }
                    true
                } else {
                    false
                }
            }
            FrameKind::While(cond) => self.eval(inst, cond).truth() == Bit::One,
            FrameKind::For { cond, step } => {
                if let Some((lv, e)) = step {
                    self.blocking_assign(inst, lv, *e);
                }
                match cond {
                    Some(c) => self.eval(inst, c).truth() == Bit::One,
                    None => true,
                }
            }
        };
        let frames = &mut self.procs[pid.idx()].frames;
        if again {
            if let Some(f) = frames.last_mut() {
                f.pc = 0;
            }
        } else {
            frames.pop();
        }
    }

    /// Pops frames up to the innermost loop; `exit` leaves the loop, else
    /// the loop's end-of-body logic runs next.
    fn unwind_loop(&mut self, pid: ProcId, exit: bool) {
        let frames = &mut self.procs[pid.idx()].frames;
        while let Some(f) = frames.last_mut() {
            if matches!(f.kind, FrameKind::Plain) {
                frames.pop();
                continue;
            }
            if exit {
                frames.pop();
            } else {
                f.pc = f.block.len();
            }
            return;
        }
    }

    /// Performs a blocking assignment immediately.
    fn blocking_assign(&mut self, inst: InstId, lv: &Lvalue, e: ExprId) {
        let v = self.eval(inst, e);
        let target = self.target_from_lvalue(inst, lv);
        let value = self.logic_for_target(&target, v);
        let stores = self.resolve_target(inst, &target);
        self.apply_stores(&stores, &value);
    }

    /// Ticks for a delay expression in the instance's time unit.
    fn expr_delay_ticks(&mut self, inst: InstId, e: ExprId) -> u64 {
        let unit = self.instances[inst.idx()].unit_ticks;
        match self.eval(inst, e) {
            Value::Real(r) => {
                #[allow(clippy::cast_precision_loss)]
                let ticks = (r * unit as f64).round();
                if ticks.is_nan() || ticks <= 0.0 {
                    0
                } else {
                    // Saturating float-to-int conversion is the intent.
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let t = ticks as u64;
                    t
                }
            }
            v => v.to_logic().to_u64().unwrap_or(0).saturating_mul(unit),
        }
    }

    /// Suspends `pid` for `ticks` (zero goes to the inactive region).
    pub(crate) fn suspend_for(&mut self, pid: ProcId, ticks: u64) {
        if ticks == 0 {
            self.procs[pid.idx()].status = ProcStatus::Queued;
            self.inactive.push_back(Event::Proc(pid));
        } else {
            self.procs[pid.idx()].status = ProcStatus::Delayed;
            self.schedule_after(ticks, Timed::Wake(pid));
        }
    }

    /// Blocks `pid` on the given edges.
    fn wait_event(&mut self, pid: ProcId, edges: &[Edge]) {
        let inst = self.procs[pid.idx()].inst;
        let p = &mut self.procs[pid.idx()];
        p.wait_gen = p.wait_gen.wrapping_add(1);
        let generation = p.wait_gen;
        p.wait_edges.clear();
        p.status = ProcStatus::WaitingEvent;
        for edge in edges {
            let sig = self.instances[inst.idx()].nets[edge.net.index()];
            self.procs[pid.idx()].wait_edges.push((sig, edge.polarity));
            self.sig_waiters[sig.idx()].push((pid, generation));
        }
    }

    /// Blocks `pid` until `cond` holds, re-checking on any change of the
    /// nets it reads.
    fn wait_until(&mut self, pid: ProcId, cond: ExprId) {
        let inst = self.procs[pid.idx()].inst;
        let m = self.instances[inst.idx()].m;
        let mut nets = std::collections::BTreeSet::new();
        let mut mems = std::collections::BTreeSet::new();
        expr_reads(m, cond, &mut nets, &mut mems);
        let p = &mut self.procs[pid.idx()];
        p.wait_gen = p.wait_gen.wrapping_add(1);
        let generation = p.wait_gen;
        p.wait_edges.clear();
        p.until = Some(cond);
        p.status = ProcStatus::WaitingUntil;
        if nets.is_empty() {
            // Nothing can change the condition: the process is stuck.
            let (name, span) = (p.name.clone(), p.process.span);
            p.status = ProcStatus::Dead;
            self.messages.push(
                Diagnostic::warning(format!(
                    "process `{name}` waits on a condition that reads no net and can never resume"
                ))
                .with_span(span),
            );
            return;
        }
        for n in nets {
            let sig = self.instances[inst.idx()].nets[n.index()];
            self.sig_waiters[sig.idx()].push((pid, generation));
        }
    }

    /// Executes one statement.
    fn exec(&mut self, pid: ProcId, inst: InstId, stmt: &'d Stmt) -> Flow<'d> {
        match &stmt.kind {
            StmtKind::Assign {
                target,
                value,
                kind,
                delay,
            } => {
                let v = self.eval(inst, *value);
                let target = self.target_from_lvalue(inst, target);
                let v = self.logic_for_target(&target, v);
                let stores = self.resolve_target(inst, &target);
                let ticks = delay.map(|d| self.delay_ticks(d));
                match (kind, ticks) {
                    (AssignKind::Blocking, None) => {
                        self.apply_stores(&stores, &v);
                    }
                    (AssignKind::Blocking, Some(t)) => {
                        self.procs[pid.idx()].pending = Some((stores, v));
                        self.suspend_for(pid, t);
                        return Flow::Suspend;
                    }
                    (AssignKind::NonBlocking, None | Some(0)) => self.nba.push((stores, v)),
                    (AssignKind::NonBlocking, Some(t)) => {
                        self.schedule_after(t, Timed::Nba(stores, v));
                    }
                }
                Flow::Next
            }
            StmtKind::If { cond, then_, else_ } => {
                let block = if self.eval(inst, *cond).truth() == Bit::One {
                    then_
                } else {
                    else_
                };
                Flow::Push(Frame {
                    block,
                    pc: 0,
                    kind: FrameKind::Plain,
                })
            }
            StmtKind::Case {
                subject,
                kind,
                qualifier,
                arms,
                default,
            } => self.exec_case(inst, stmt.span, *subject, *kind, *qualifier, arms, default),
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some((lv, e)) = init {
                    self.blocking_assign(inst, lv, *e);
                }
                if let Some(c) = cond
                    && self.eval(inst, *c).truth() != Bit::One
                {
                    return Flow::Next;
                }
                Flow::Push(Frame {
                    block: body,
                    pc: 0,
                    kind: FrameKind::For {
                        cond: *cond,
                        step: step.as_ref(),
                    },
                })
            }
            StmtKind::While { cond, body } => {
                if self.eval(inst, *cond).truth() != Bit::One {
                    return Flow::Next;
                }
                Flow::Push(Frame {
                    block: body,
                    pc: 0,
                    kind: FrameKind::While(*cond),
                })
            }
            StmtKind::Repeat { count, body } => {
                let n = self.eval_logic(inst, *count).to_u64().unwrap_or(0);
                if n == 0 {
                    return Flow::Next;
                }
                Flow::Push(Frame {
                    block: body,
                    pc: 0,
                    kind: FrameKind::Repeat(n),
                })
            }
            StmtKind::Forever { body } => Flow::Push(Frame {
                block: body,
                pc: 0,
                kind: FrameKind::Forever,
            }),
            StmtKind::Block { body, .. } => Flow::Push(Frame {
                block: body,
                pc: 0,
                kind: FrameKind::Plain,
            }),
            StmtKind::Wait(WaitKind::Delay(e)) => {
                let ticks = self.expr_delay_ticks(inst, *e);
                self.suspend_for(pid, ticks);
                Flow::Suspend
            }
            StmtKind::Wait(WaitKind::Event(edges)) => {
                self.wait_event(pid, edges);
                Flow::Suspend
            }
            StmtKind::Wait(WaitKind::Until(e)) => {
                if self.eval(inst, *e).truth() == Bit::One {
                    return Flow::Next;
                }
                self.wait_until(pid, *e);
                Flow::Suspend
            }
            StmtKind::SysCall { name, args } => {
                if self.exec_syscall(pid, inst, name.as_str(), args, stmt.span) {
                    Flow::Next
                } else {
                    Flow::Suspend
                }
            }
            StmtKind::MemWrite {
                mem,
                addr,
                value,
                enable,
            } => {
                let enabled = match enable {
                    Some(en) => self.eval(inst, *en).truth() == Bit::One,
                    None => true,
                };
                if enabled {
                    let v = self.eval_logic(inst, *value);
                    let target = Target::Mem {
                        mem: self.instances[inst.idx()].mems[mem.index()],
                        addr: *addr,
                    };
                    let stores = self.resolve_target(inst, &target);
                    self.nba.push((stores, v));
                }
                Flow::Next
            }
            StmtKind::Assert {
                cond,
                severity,
                message,
            } => {
                if self.eval(inst, *cond).truth() != Bit::One {
                    let text = self.format_args(inst, message, 'd');
                    self.report(inst, *severity, &text, stmt.span);
                }
                Flow::Next
            }
            StmtKind::Finish => {
                self.finish();
                self.procs[pid.idx()].status = ProcStatus::Dead;
                Flow::Suspend
            }
            StmtKind::Stop => {
                self.stop_at(pid);
                Flow::Suspend
            }
            StmtKind::Break => Flow::Break,
            StmtKind::Continue => Flow::Continue,
        }
    }

    /// Executes a `case` statement: picks the first matching arm.
    #[allow(clippy::too_many_arguments)]
    fn exec_case(
        &mut self,
        inst: InstId,
        span: Span,
        subject: ExprId,
        kind: CaseKind,
        qualifier: CaseQualifier,
        arms: &'d [crate::ir::CaseArm],
        default: &'d Option<Block>,
    ) -> Flow<'d> {
        let subj = self.eval_logic(inst, subject);
        let mut matched: Option<&'d Block> = None;
        let mut count = 0usize;
        for arm in arms {
            for v in &arm.values {
                let item = self.eval_logic(inst, *v);
                let (s, i) = if item.width() == subj.width() {
                    (subj.clone(), item)
                } else {
                    let w = subj.width().max(item.width());
                    (subj.resize(w), item.resize(w))
                };
                let hit = match kind {
                    CaseKind::Plain => s.case_eq(&i).truth() == Bit::One,
                    CaseKind::Z => s.casez_match(&i),
                    CaseKind::X => s.casex_match(&i),
                };
                if hit {
                    count += 1;
                    if matched.is_none() {
                        matched = Some(&arm.body);
                    }
                    break;
                }
            }
        }
        match qualifier {
            CaseQualifier::Unique if count > 1 => self.messages.push(
                Diagnostic::warning(format!(
                    "unique case: {count} arms match {} at time {}",
                    subj.to_verilog_literal(),
                    self.now
                ))
                .with_span(span),
            ),
            CaseQualifier::Unique | CaseQualifier::Priority if count == 0 && default.is_none() => {
                self.messages.push(
                    Diagnostic::warning(format!(
                        "{} case: no arm matches {} at time {}",
                        if qualifier == CaseQualifier::Unique {
                            "unique"
                        } else {
                            "priority"
                        },
                        subj.to_verilog_literal(),
                        self.now
                    ))
                    .with_span(span),
                );
            }
            _ => {}
        }
        match matched.or(default.as_ref()) {
            Some(block) => Flow::Push(Frame {
                block,
                pc: 0,
                kind: FrameKind::Plain,
            }),
            None => Flow::Next,
        }
    }

    /// Ends the run (`$finish`, a failing assertion).
    pub(crate) fn finish(&mut self) {
        self.status = Status::Finished;
    }

    /// Pauses the run (`$stop`); `pid` resumes first when the run
    /// continues.
    pub(crate) fn stop_at(&mut self, pid: ProcId) {
        self.status = Status::Stopped;
        self.procs[pid.idx()].status = ProcStatus::Queued;
        self.active.push_front(Event::Proc(pid));
    }

    /// Reports an assertion or `$error`-family message with its severity:
    /// text goes to the output, a diagnostic to the messages, and
    /// `Failure` ends the run.
    pub(crate) fn report(
        &mut self,
        inst: InstId,
        severity: ReportSeverity,
        text: &str,
        span: Span,
    ) {
        let label = match severity {
            ReportSeverity::Note => "Note",
            ReportSeverity::Warning => "Warning",
            ReportSeverity::Error => "Error",
            ReportSeverity::Failure => "Failure",
        };
        self.output.push_str(label);
        self.output.push_str(": ");
        self.output.push_str(text);
        self.output.push('\n');
        let path = self.instances[inst.idx()].path.clone();
        let time = self.now;
        let diag = match severity {
            ReportSeverity::Note => Diagnostic::note(text),
            ReportSeverity::Warning => Diagnostic::warning(text),
            ReportSeverity::Error | ReportSeverity::Failure => Diagnostic::error(text),
        };
        self.messages.push(
            diag.with_span(span)
                .with_note(format!("reported by {path} at time {time}")),
        );
        if severity == ReportSeverity::Failure {
            self.finish();
        }
    }
}
