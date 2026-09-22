//! Turning `always` and `initial` blocks into IR processes.
//!
//! The trigger decides the [`ProcessKind`]; the table is in the module
//! documentation of [`super`]. Two points deserve their own explanation.
//!
//! **Reset extraction.** `always @(posedge clk or negedge rst_n)` lists
//! the clock and the asynchronous reset the same way, and only the body
//! says which is which: a reset is tested in the leading `if` / `else if`
//! chain, before anything else happens. This module walks that chain,
//! collects the nets it tests, and calls every listed edge on one of them
//! a reset; the rest are clocks. A trigger with a single edge is always a
//! clock, so a synchronous reset stays part of the body.
//!
//! **Self-timed blocks.** A block whose body contains a delay, an event
//! control or a `wait` cannot be described by a sensitivity list: it is a
//! [`crate::ir::ProcessKind::Free`] process whose body is wrapped in `forever`, with
//! the header's own event control kept as the first `wait`. That is what
//! `always #5 clk = ~clk;` and most testbench code becomes.

use crate::ir::{Attrs, Edge, NetId, Polarity, ProcessKind};
use crate::verilog::ast::{self, AlwaysKind, Item, Stmt, StmtKind as AstStmt};

use super::super::scope::Symbol;
use super::Lowerer;

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// Lowers an `always`, `always_ff`, `always_comb` or `always_latch`
    /// block.
    pub(super) fn always(&mut self, kind: AlwaysKind, stmt: &'ast Stmt, item: &'ast Item) {
        let (process_kind, body, wrap_forever) = self.always_kind(kind, stmt);
        let attrs = self.attrs_of(&item.attrs);
        let name = stmt.label.as_ref().map(|l| l.name.clone());
        self.emit_process(name, process_kind, body, wrap_forever, attrs, item);
    }

    /// Lowers an `initial` block.
    pub(super) fn initial(&mut self, stmt: &'ast Stmt, item: &'ast Item) {
        let attrs = self.attrs_of(&item.attrs);
        let name = stmt.label.as_ref().map(|l| l.name.clone());
        self.emit_process(name, ProcessKind::Initial, stmt, false, attrs, item);
    }

    /// Creates the process and lowers `body` into it.
    fn emit_process(
        &mut self,
        name: Option<String>,
        kind: ProcessKind,
        body: &'ast Stmt,
        wrap_forever: bool,
        attrs: Attrs,
        item: &'ast Item,
    ) {
        self.driver = if kind == ProcessKind::Initial {
            super::DriverKey::Initial
        } else {
            super::DriverKey::Process(self.b.module().processes.len())
        };
        self.b.span = item.span;
        let scope = self.env.enter_new(String::new());
        let _ = scope;
        let mut p = self.b.process(name.as_deref(), kind);
        p.span = item.span;
        p.attrs = attrs;
        if wrap_forever {
            let mut inner = self.b.block();
            inner.span = body.span;
            self.stmt(body, &mut inner);
            p.span = item.span;
            p.forever(inner.finish());
        } else {
            self.stmt(body, &mut p);
        }
        self.b.end_process(p);
        self.env.leave();
    }

    /// The kind of an `always` block, the statement to lower, and whether
    /// it must be wrapped in `forever`.
    fn always_kind(
        &mut self,
        kind: AlwaysKind,
        stmt: &'ast Stmt,
    ) -> (ProcessKind, &'ast Stmt, bool) {
        if matches!(kind, AlwaysKind::Comb | AlwaysKind::Latch) {
            if has_timing(stmt) {
                self.env.unsupported(
                    stmt.span,
                    "a timing control inside `always_comb` or `always_latch`",
                );
            }
            return (ProcessKind::Comb, stmt, false);
        }
        let AstStmt::Timing(ctrl, inner) = &stmt.kind else {
            // `always` with no timing control: a self-timed loop.
            return (ProcessKind::Free, stmt, true);
        };
        let ast::TimingKind::Event(ev) = &ctrl.kind else {
            // `always #5 ...`: self-timed.
            return (ProcessKind::Free, stmt, true);
        };
        if has_timing(inner) {
            // The body controls its own timing; keep the header event as
            // the first wait of a free-running loop.
            return (ProcessKind::Free, stmt, true);
        }
        match &ev.kind {
            ast::EventControlKind::Any => (ProcessKind::Comb, inner, false),
            ast::EventControlKind::List(_) => {
                let edges = self.event_edges(ev);
                if edges.is_empty() {
                    return (ProcessKind::Comb, inner, false);
                }
                if edges.iter().all(|e| e.polarity == Polarity::Any) {
                    let nets: Vec<NetId> = edges.iter().map(|e| e.net).collect();
                    return (ProcessKind::Sensitive(nets), inner, false);
                }
                let (clocks, resets) = self.split_edges(edges, inner);
                (ProcessKind::Sequential { clocks, resets }, inner, false)
            }
        }
    }

    /// Splits a trigger list into clocks and asynchronous resets.
    fn split_edges(&mut self, edges: Vec<Edge>, body: &'ast Stmt) -> (Vec<Edge>, Vec<Edge>) {
        if edges.len() < 2 {
            return (edges, Vec::new());
        }
        let watched: Vec<NetId> = edges.iter().map(|e| e.net).collect();
        let mut tested = Vec::new();
        self.leading_conditions(body, &watched, &mut tested);
        let (resets, clocks): (Vec<Edge>, Vec<Edge>) = edges
            .iter()
            .partition(|e| tested.contains(&e.net) && e.polarity != Polarity::Any);
        if clocks.is_empty() {
            // Everything looked like a reset; the first edge is the clock.
            let mut resets = resets;
            let first = resets.remove(0);
            return (vec![first], resets);
        }
        (clocks, resets)
    }

    /// Collects the watched nets tested in the leading `if` / `else if`
    /// chain of `body`.
    fn leading_conditions(&mut self, body: &'ast Stmt, watched: &[NetId], out: &mut Vec<NetId>) {
        let mut stmt = body;
        // Step into a `begin ... end` to reach its leading statement.
        while let AstStmt::Block(b) = &stmt.kind {
            match b.stmts.first() {
                Some(first) => stmt = first,
                None => return,
            }
        }
        let mut current = Some(stmt);
        while let Some(AstStmt::If(i)) = current.map(|s| &s.kind) {
            let nets = self.condition_nets(&i.cond);
            let relevant: Vec<NetId> = nets.into_iter().filter(|n| watched.contains(n)).collect();
            if relevant.is_empty() {
                return;
            }
            out.extend(relevant);
            current = i.else_stmt.as_deref();
        }
    }

    /// Every net a condition reads, best effort and without reporting.
    fn condition_nets(&mut self, e: &'ast ast::Expr) -> Vec<NetId> {
        let mut out = Vec::new();
        self.collect_nets(e, &mut out);
        out
    }

    fn collect_nets(&mut self, e: &'ast ast::Expr, out: &mut Vec<NetId>) {
        use ast::ExprKind as K;
        if let Some(Symbol::Net { net, .. }) = self.env.probe(|env| env.resolve_path(e)) {
            out.push(net);
            return;
        }
        match &e.kind {
            K::Unary { operand, .. } => self.collect_nets(operand, out),
            K::Binary { lhs, rhs, .. } => {
                self.collect_nets(lhs, out);
                self.collect_nets(rhs, out);
            }
            K::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                self.collect_nets(cond, out);
                self.collect_nets(then_expr, out);
                self.collect_nets(else_expr, out);
            }
            K::Index { base, .. } | K::Range { base, .. } | K::Member { base, .. } => {
                self.collect_nets(base, out);
            }
            K::Concat(parts) => {
                for p in parts {
                    self.collect_nets(p, out);
                }
            }
            K::Cast { expr, .. } | K::MinTypMax { typ: expr, .. } => self.collect_nets(expr, out),
            _ => {}
        }
    }
}

/// True when a statement contains a delay, event control or `wait`.
pub(super) fn has_timing(s: &Stmt) -> bool {
    match &s.kind {
        AstStmt::Timing(..) | AstStmt::Wait(..) | AstStmt::WaitFork => true,
        AstStmt::Assign(a) => a.timing.is_some(),
        AstStmt::Block(b) | AstStmt::Fork(b, _) => b.stmts.iter().any(has_timing),
        AstStmt::If(i) => {
            has_timing(&i.then_stmt) || i.else_stmt.as_ref().is_some_and(|e| has_timing(e))
        }
        AstStmt::Case(c) => c.items.iter().any(|i| has_timing(&i.body)),
        AstStmt::For(f) => has_timing(&f.body),
        AstStmt::While(_, b)
        | AstStmt::DoWhile(b, _)
        | AstStmt::Repeat(_, b)
        | AstStmt::Forever(b) => has_timing(b),
        AstStmt::Foreach(f) => has_timing(&f.body),
        _ => false,
    }
}
