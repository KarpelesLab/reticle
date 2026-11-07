//! Lowering `wait`.
//!
//! The IR's waits are Verilog's: `#d` resumes in the *inactive* region of
//! the same time step when `d` is zero, and `wait (c)` is level-sensitive,
//! resuming at once when `c` already holds. VHDL's are not (LRM 10.2):
//!
//! - `wait for 0 ns` resumes in the next delta cycle, after the signal
//!   updates of this one;
//! - `wait until c` waits for an *event* on a signal of `c` and only then
//!   tests `c`, so it never resumes at once;
//! - `wait until c for t` gives up at `now + t` whatever `c` is.
//!
//! So a VHDL `wait` with a condition or a zero delay is built from the
//! IR's event wait. Each process that needs one gets a counter variable and
//! a `$wake` signal: the process increments the counter and assigns it to
//! `$wake` with a non-blocking assignment — `after t` for a timeout — and
//! the wait includes an event on `$wake`. A non-blocking update is applied
//! with every other one of its delta, so waking on it is waking in the
//! next delta; and since every assignment writes a new count, one left
//! over from an earlier wait that ended early wakes the process once, the
//! wait sees that its time has not come, and goes on waiting.
//!
//! ```text
//! wait until c for t;        $n = $n + 1; $wake <= $n after t; $deadline = now + t
//!                            forever
//!                              wait on (signals of c), $wake
//!                              if now >= $deadline: break
//!                              if c: break
//!                            end
//! ```
//!
//! A `wait for t` with `t` known and greater than zero, a `wait on s`, a
//! bare `wait`, and the `wait until rising_edge(clk)` a clocked process
//! begins with (recognised before this point) lower as they did.

use crate::ir::builder::BlockBuilder;
use crate::ir::{AssignKind, Edge, ExprId, ExprKind, Lvalue, NetId, NetKind, ProcessKind};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast;

use super::codes;
use super::lower::{DriverKey, DriverSite, Lowerer, Sink};

/// The hidden nets one process's waits share.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaitNets {
    /// A variable counting the wake-ups scheduled.
    count: NetId,
    /// The signal each wake-up is assigned to.
    wake: NetId,
}

impl<'a> Lowerer<'a, '_> {
    /// Lowers a `wait` statement.
    pub(crate) fn wait_stmt(
        &mut self,
        sensitivity: Option<&'a ast::Sensitivity>,
        condition: Option<&'a ast::Expr>,
        timeout: Option<&'a ast::Expr>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        out.span = span;
        let mut signals: Vec<NetId> = Vec::new();
        if let Some(ast::Sensitivity::Names(names)) = sensitivity {
            for n in names {
                if let Some(net) = self.signal_net(n) {
                    signals.push(net);
                }
            }
        }
        match (condition, timeout) {
            // `wait;` and `wait on s;`.
            (None, None) => {
                if signals.is_empty() {
                    self.b.span = span;
                    let never = self.b.const_bit(false);
                    out.wait_until(never);
                } else {
                    let edges = signals.into_iter().map(Edge::any).collect();
                    out.wait_event(edges);
                }
            }
            // `wait for t;`
            (None, Some(t)) if signals.is_empty() => self.wait_for(t, span, out),
            _ => self.wait_general(signals, condition, timeout, span, out),
        }
    }

    /// `wait for t`: the IR's delay when `t` is known to be positive, the
    /// next delta cycle when it is zero, and a test of `t` otherwise.
    fn wait_for(&mut self, t: &'a ast::Expr, span: Span, out: &mut BlockBuilder) {
        let d = self.time_expr(t, out);
        match self.b.module().expr(d).as_const().and_then(Logic::to_i64) {
            Some(0) => self.wait_delta(span, out),
            Some(_) => {
                out.span = span;
                out.wait_delay(d);
            }
            None => {
                self.b.span = span;
                let zero = self.b.constant(Logic::from_i64(0, 64).with_signed(true));
                let is_zero = self.b.eq(d, zero);
                let mut then_ = self.b.block();
                then_.span = span;
                self.wait_delta(span, &mut then_);
                let mut else_ = self.b.block();
                else_.span = span;
                else_.wait_delay(d);
                out.span = span;
                out.if_(is_zero, then_.finish(), else_.finish());
            }
        }
    }

    /// Waits for the next delta cycle.
    fn wait_delta(&mut self, span: Span, out: &mut BlockBuilder) {
        let Some(w) = self.wait_nets(span) else {
            return;
        };
        self.schedule_wake(w, None, span, out);
        out.span = span;
        out.wait_event(vec![Edge::any(w.wake)]);
    }

    /// Any other `wait`: an event on the sensitivity set (the signals of
    /// the condition when there is no `on`), then the condition, with a
    /// timeout.
    fn wait_general(
        &mut self,
        mut signals: Vec<NetId>,
        condition: Option<&'a ast::Expr>,
        timeout: Option<&'a ast::Expr>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        // The condition is evaluated inside the loop, after each wake-up;
        // whatever an inlined function in it computes is computed there.
        let mut test = self.b.block();
        test.span = span;
        let cond = condition.map(|c| self.cond(c, &mut Sink::Proc(&mut test)));
        let test = test.finish();
        if signals.is_empty() && condition.is_some() {
            let mut found = Vec::new();
            if let Some(c) = cond {
                self.signals_read(c, &mut found);
            }
            for s in &test {
                crate::ir::walk::stmt_exprs(s, &mut |e| self.signals_read(e, &mut found));
            }
            signals = found;
        }
        let mut deadline = None;
        let mut wake = None;
        if let Some(t) = timeout {
            let Some(delay) = self.delay_of(t) else {
                self.unsupported(t.span(), "a `wait ... for` whose time is not static");
                return;
            };
            let Some(w) = self.wait_nets(span) else {
                return;
            };
            let d = self.time_expr(t, out);
            let now = self.now_expr(span);
            self.b.span = span;
            let at = self.b.add(now, d);
            let name = self.fresh("wait$deadline");
            let layout = self.physical_layout();
            let Some(dl) = self.new_net(name, layout, false, span) else {
                return;
            };
            self.b.module_mut().nets[dl].kind = NetKind::Variable;
            out.span = span;
            out.assign(Lvalue::Net(dl), at, AssignKind::Blocking);
            self.schedule_wake(w, Some(delay), span, out);
            deadline = Some(dl);
            wake = Some(w.wake);
        }
        if signals.is_empty() && wake.is_none() {
            // A condition over no signal is never re-evaluated: the wait
            // lasts for ever (LRM 10.2).
            self.b.span = span;
            let never = self.b.const_bit(false);
            out.span = span;
            out.wait_until(never);
            return;
        }
        let mut edges: Vec<Edge> = signals.into_iter().map(Edge::any).collect();
        if let Some(w) = wake {
            edges.push(Edge::any(w));
        }
        let mut body = self.b.block();
        body.span = span;
        body.wait_event(edges);
        if let Some(dl) = deadline {
            let now = self.now_expr(span);
            self.b.span = span;
            let at = self.b.net(dl);
            let due = self.b.ge(now, at);
            let mut leave = self.b.block();
            leave.span = span;
            leave.break_();
            body.span = span;
            body.if_(due, leave.finish(), Vec::new());
        }
        for s in test {
            body.push(s.kind);
        }
        let cond = match cond {
            Some(c) => c,
            None => {
                self.b.span = span;
                self.b.const_bit(true)
            }
        };
        let mut leave = self.b.block();
        leave.span = span;
        leave.break_();
        body.span = span;
        body.if_(cond, leave.finish(), Vec::new());
        out.span = span;
        out.forever(body.finish());
    }

    /// Schedules a wake-up of this process: now, which is the next delta,
    /// or after `delay`.
    fn schedule_wake(
        &mut self,
        w: WaitNets,
        delay: Option<crate::ir::Delay>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        self.b.span = span;
        let c = self.b.net(w.count);
        let one = self.b.constant(Logic::from_i64(1, 32).with_signed(true));
        let next = self.b.add(c, one);
        out.span = span;
        out.assign(Lvalue::Net(w.count), next, AssignKind::Blocking);
        let c = self.b.net(w.count);
        match delay {
            Some(d) => out.assign_after(Lvalue::Net(w.wake), c, AssignKind::NonBlocking, d),
            None => out.assign(Lvalue::Net(w.wake), c, AssignKind::NonBlocking),
        }
    }

    /// The hidden nets of the current process's waits, made on first use.
    fn wait_nets(&mut self, span: Span) -> Option<WaitNets> {
        let DriverKey::Process(index) = self.driver else {
            self.error(codes::UNSUPPORTED, span, "a `wait` outside a process");
            return None;
        };
        if let Some(w) = self.wait_hidden.get(&index) {
            return Some(*w);
        }
        let int = self.int_layout();
        let count_name = self.fresh("wait$count");
        let count = self.new_net(count_name, int.clone(), false, span)?;
        self.b.module_mut().nets[count].kind = NetKind::Variable;
        let wake_name = self.fresh("wait$wake");
        let wake = self.new_net(wake_name, int, true, span)?;
        // The counter starts at zero, before anything runs.
        self.b.span = span;
        let zero = self.b.constant(Logic::from_i64(0, 32).with_signed(true));
        let mut p = self.b.process(None, ProcessKind::Initial);
        p.span = span;
        p.blocking(count, zero);
        self.b.end_process(p);
        self.record_driver(count, DriverSite::Fixed, true, span);
        let w = WaitNets { count, wake };
        self.wait_hidden.insert(index, w);
        Some(w)
    }

    /// The current time, in femtoseconds, as a signed 64-bit value.
    pub(crate) fn now_expr(&mut self, span: Span) -> ExprId {
        self.b.span = span;
        let t = self.b.call("$time", Vec::new(), crate::ir::Type::bits(64));
        self.b.resize(t, 64, true)
    }

    /// The layout of `time`.
    pub(crate) fn physical_layout(&self) -> super::types::Layout {
        super::types::Layout {
            kind: super::types::LayoutKind::Physical,
            width: 64,
            signed: true,
            ty: self.a().builtins.time,
        }
    }

    /// Adds the signals expression `e` reads to `out`.
    fn signals_read(&self, e: ExprId, out: &mut Vec<NetId>) {
        let m = self.b.module();
        let mut stack = vec![e];
        while let Some(id) = stack.pop() {
            let Some(node) = m.exprs.get(id) else {
                continue;
            };
            if let ExprKind::Net(n) = node.kind {
                // A shadow (`clk'event`) changes with its signal: the
                // signal is what to wait on.
                let n = self.shadow_base.get(&n).copied().unwrap_or(n);
                if self.nets.get(n.index()).is_some_and(|i| i.is_signal) && !out.contains(&n) {
                    out.push(n);
                }
                continue;
            }
            stack.extend(crate::ir::expr::operands(&node.kind));
        }
    }
}
