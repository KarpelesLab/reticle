//! Lowering concurrent statements: processes, drivers, instances and
//! generate.
//!
//! # Processes
//!
//! The sensitivity list and the shape of the body together decide the
//! [`ProcessKind`]. `process (all)` is combinational; an explicit list is a
//! `Sensitive` process unless the body is one of the three clocked idioms,
//! which become `Sequential`:
//!
//! ```text
//! process (clk)                     process (clk, rst)           process
//! begin                             begin                        begin
//!   if rising_edge(clk) then          if rst = '1' then            wait until rising_edge(clk);
//!     ...                               ...                        ...
//!   end if;                           elsif rising_edge(clk) then  end process;
//! end process;                          ...
//!                                     end if;
//!                                   end process;
//! ```
//!
//! The first drops the `if` and keeps its body; the second keeps the whole
//! `if` (with the `elsif` turned into the `else` branch) and records the
//! reset as an asynchronous edge, which is the same shape the Verilog
//! frontend produces for `always @(posedge clk or posedge rst)`;
//! `falling_edge` and `clk'event and clk = '1'` are recognised the same
//! way. A process with no sensitivity list and any other `wait` is `Free`,
//! its body wrapped in `forever` because a VHDL process restarts when it
//! reaches its end.
//!
//! # Concurrent assignments
//!
//! A simple waveform becomes one [`crate::ir::Assign`]. A conditional
//! assignment becomes a chain of `Ternary` nodes and a selected assignment
//! a chain of equality tests — the expression form of a `case` — so no
//! process is created for either. An assignment whose arms carry `after`
//! delays or several waveform elements cannot be one expression, so it
//! becomes a combinational process instead.
//!
//! # Generate and blocks
//!
//! Generate statements are unrolled into the enclosing module with a
//! hierarchical name prefix (`g(3).`); the loop parameter is a constant in
//! each copy, so everything inside it folds. An `if`/`case` generate whose
//! condition is not static after elaboration is `V0704`, and a `for`
//! generate over more than 4096 values is `V0712`. A block statement is
//! flattened the same way; a block with its own ports or generics is
//! `V0701`.

use crate::diag::Diagnostic;
use crate::ir::{
    AssignKind, CellKind, Const, Edge, ExprId, ExprKind, Lvalue, ModuleRef, Name, NetId, Polarity,
    PortDir, ProcessKind, ReportSeverity,
};
use crate::source::Span;
use crate::vhdl::ast::{self, Mode};
use crate::vhdl::sema::{DeclId, DeclKind, UnitId, Value};

use super::lower::{AttrTarget, Binding, DriverKey, DriverSite, Lowerer, Sink};
use super::types::{Layout, LayoutKind};
use super::{GenericValue, codes};

/// The largest number of iterations a `for ... generate` may unroll to.
const GENERATE_LIMIT: i64 = 4096;

impl<'a> Lowerer<'a, '_> {
    /// Lowers a list of concurrent statements.
    pub(crate) fn concurrent_statements(&mut self, stmts: &'a [ast::ConcurrentStatement]) {
        for s in stmts {
            self.concurrent(s);
        }
    }

    fn concurrent(&mut self, s: &'a ast::ConcurrentStatement) {
        self.b.span = s.span;
        let label = s.label.as_ref().map(|l| l.name.clone());
        match &s.kind {
            ast::ConcurrentKind::Process(p) => self.process(label.as_deref(), p),
            ast::ConcurrentKind::SignalAssignment(csa) => {
                self.concurrent_assign(&csa.assignment);
            }
            ast::ConcurrentKind::ProcedureCall { call, .. } => {
                let index = self.b.module().processes.len();
                let outer = self.driver;
                self.driver = DriverKey::Process(index);
                let mut p = self.b.process(label.as_deref(), ProcessKind::Comb);
                p.span = s.span;
                self.procedure_call(call, &mut p);
                let pid = self.b.end_process(p);
                self.attribute_drivers(pid);
                self.driver = outer;
            }
            ast::ConcurrentKind::Assertion { assertion, .. } => {
                let index = self.b.module().processes.len();
                let outer = self.driver;
                self.driver = DriverKey::Process(index);
                let kind = if self.eval(&assertion.condition).is_some() {
                    ProcessKind::Initial
                } else {
                    ProcessKind::Comb
                };
                let mut p = self.b.process(label.as_deref(), kind);
                p.span = s.span;
                let cond = self.cond(&assertion.condition, &mut Sink::Proc(&mut p));
                let sev = self.severity_of(assertion.severity.as_ref(), ReportSeverity::Error);
                let msg = match &assertion.report {
                    Some(m) => match self.expr(m, &mut Sink::Proc(&mut p)) {
                        Some((id, _)) => vec![id],
                        None => Vec::new(),
                    },
                    None => Vec::new(),
                };
                p.span = s.span;
                p.assert(cond, sev, msg);
                self.b.end_process(p);
                self.driver = outer;
            }
            ast::ConcurrentKind::Instantiation(inst) => {
                self.instantiation(label.as_deref(), inst, s.span);
            }
            ast::ConcurrentKind::Block(b) => self.block(label.as_deref(), b),
            ast::ConcurrentKind::ForGenerate(g) => self.for_generate(label.as_deref(), g),
            ast::ConcurrentKind::IfGenerate(g) => self.if_generate(label.as_deref(), g),
            ast::ConcurrentKind::CaseGenerate(g) => self.case_generate(label.as_deref(), g),
        }
    }

    /// Records the drivers of every assignment a helper process makes.
    fn attribute_drivers(&mut self, pid: crate::ir::ProcessId) {
        let targets = collect_targets(&self.b.module().processes[pid].body);
        let span = self.b.module().processes[pid].span;
        for (net, whole) in targets {
            self.record_driver(net, DriverSite::Process(pid), whole, span);
        }
    }

    // --- processes ----------------------------------------------------------

    fn process(&mut self, label: Option<&str>, p: &'a ast::ProcessStatement) {
        let index = self.b.module().processes.len();
        let outer = self.driver;
        self.driver = DriverKey::Process(index);
        self.push_scope();
        let saved = self.prefix.clone();
        if let Some(l) = label {
            self.prefix = format!("{}{l}.", self.prefix);
        }
        self.declarations(&p.decls);
        let name = label.map(|l| format!("{saved}{l}"));

        let kind = self.process_kind(p);
        let mut proc = self.b.process(name.as_deref(), kind.kind);
        proc.span = p.span;
        match kind.shape {
            Shape::Plain => self.stmts(&p.statements, &mut proc),
            Shape::Body(stmts) => self.stmts(stmts, &mut proc),
            Shape::Reset { cond, reset, body } => {
                let c = self.cond(cond, &mut Sink::Proc(&mut proc));
                let mut then_ = self.b.block();
                then_.span = p.span;
                self.stmts(reset, &mut then_);
                let mut else_ = self.b.block();
                else_.span = p.span;
                self.stmts(body, &mut else_);
                proc.span = p.span;
                proc.if_(c, then_.finish(), else_.finish());
            }
            Shape::Forever(stmts) => {
                let mut inner = self.b.block();
                inner.span = p.span;
                self.stmts(stmts, &mut inner);
                proc.span = p.span;
                proc.forever(inner.finish());
            }
        }
        let pid = self.b.end_process(proc);
        self.attribute_drivers(pid);
        self.prefix = saved;
        self.pop_scope();
        self.driver = outer;
    }

    /// The kind of a process and which statements make up its body.
    fn process_kind(&mut self, p: &'a ast::ProcessStatement) -> ProcessShape<'a> {
        match &p.sensitivity {
            Some(ast::Sensitivity::All(_)) => ProcessShape {
                kind: ProcessKind::Comb,
                shape: Shape::Plain,
            },
            Some(ast::Sensitivity::Names(names)) => {
                if let Some(shape) = self.clocked_shape(&p.statements) {
                    return shape;
                }
                let nets: Vec<NetId> = names.iter().filter_map(|n| self.signal_net(n)).collect();
                ProcessShape {
                    kind: ProcessKind::Sensitive(nets),
                    shape: Shape::Plain,
                }
            }
            None => {
                if let Some(shape) = self.wait_until_shape(&p.statements) {
                    return shape;
                }
                ProcessShape {
                    kind: ProcessKind::Free,
                    shape: Shape::Forever(&p.statements),
                }
            }
        }
    }

    /// Recognises the two clocked `if` idioms.
    fn clocked_shape(&mut self, stmts: &'a [ast::SequentialStatement]) -> Option<ProcessShape<'a>> {
        let [only] = stmts else { return None };
        let ast::SequentialKind::If(i) = &only.kind else {
            return None;
        };
        let edges: Vec<Option<Edge>> = i.arms.iter().map(|a| self.edge_of(&a.condition)).collect();
        let clocked = edges.iter().filter(|e| e.is_some()).count();
        if clocked == 0 {
            return None;
        }
        if clocked > 1 {
            self.unsupported(i.span, "a process testing more than one clock edge");
            return Some(ProcessShape {
                kind: ProcessKind::Comb,
                shape: Shape::Body(&[]),
            });
        }
        match (i.arms.len(), &edges[..]) {
            (1, [Some(edge)]) if i.else_statements.is_none() => Some(ProcessShape {
                kind: ProcessKind::Sequential {
                    clocks: vec![*edge],
                    resets: Vec::new(),
                },
                shape: Shape::Body(&i.arms[0].statements),
            }),
            (2, [None, Some(edge)]) if i.else_statements.is_none() => {
                let Some(reset) = self.reset_edge(&i.arms[0].condition) else {
                    self.unsupported(
                        i.arms[0].span,
                        "a clocked process whose first branch is not a reset test",
                    );
                    return Some(ProcessShape {
                        kind: ProcessKind::Comb,
                        shape: Shape::Body(&[]),
                    });
                };
                Some(ProcessShape {
                    kind: ProcessKind::Sequential {
                        clocks: vec![*edge],
                        resets: vec![reset],
                    },
                    shape: Shape::Reset {
                        cond: &i.arms[0].condition,
                        reset: &i.arms[0].statements,
                        body: &i.arms[1].statements,
                    },
                })
            }
            _ => {
                self.unsupported(i.span, "this clocked process shape");
                Some(ProcessShape {
                    kind: ProcessKind::Comb,
                    shape: Shape::Body(&[]),
                })
            }
        }
    }

    /// Recognises `wait until rising_edge(clk);` as the first statement.
    fn wait_until_shape(
        &mut self,
        stmts: &'a [ast::SequentialStatement],
    ) -> Option<ProcessShape<'a>> {
        let (first, rest) = stmts.split_first()?;
        let ast::SequentialKind::Wait {
            sensitivity,
            condition,
            timeout,
        } = &first.kind
        else {
            return None;
        };
        if sensitivity.is_some() || timeout.is_some() {
            return None;
        }
        let edge = self.edge_of(condition.as_ref()?)?;
        if rest.iter().any(has_wait) {
            return None;
        }
        Some(ProcessShape {
            kind: ProcessKind::Sequential {
                clocks: vec![edge],
                resets: Vec::new(),
            },
            shape: Shape::Body(rest),
        })
    }

    /// The clock edge a condition tests, if it is one.
    fn edge_of(&mut self, e: &'a ast::Expr) -> Option<Edge> {
        match e {
            ast::Expr::Paren { inner, .. } => self.edge_of(inner),
            ast::Expr::Name(ast::Name::Call { prefix, args, span }) => {
                let d = self.a().decl_of(*span)?;
                if !self.is_std_logic_1164(d) {
                    return None;
                }
                let name = self.a().decl(d).spelling.to_ascii_lowercase();
                let polarity = match name.as_str() {
                    "rising_edge" => Polarity::Pos,
                    "falling_edge" => Polarity::Neg,
                    _ => return None,
                };
                let _ = prefix;
                let arg = super::expr::first_expr(args)?;
                let ast::Expr::Name(n) = arg else { return None };
                let net = self.signal_net(n)?;
                Some(Edge { net, polarity })
            }
            ast::Expr::Binary {
                op: ast::BinaryOp::And,
                lhs,
                rhs,
                ..
            } => {
                let (event, level) = match (self.is_event(lhs), self.is_event(rhs)) {
                    (Some(n), None) => (n, rhs.as_ref()),
                    (None, Some(n)) => (n, lhs.as_ref()),
                    _ => return None,
                };
                let polarity = self.level_polarity(level, event)?;
                Some(Edge {
                    net: event,
                    polarity,
                })
            }
            _ => None,
        }
    }

    /// The net of an `x'event` attribute.
    fn is_event(&mut self, e: &'a ast::Expr) -> Option<NetId> {
        let ast::Expr::Name(ast::Name::Attribute {
            prefix, attribute, ..
        }) = e
        else {
            return None;
        };
        if !attribute.name.eq_ignore_ascii_case("event") {
            return None;
        }
        self.signal_net(prefix)
    }

    /// The polarity of `x = '1'` / `x = '0'` on the given net.
    fn level_polarity(&mut self, e: &'a ast::Expr, net: NetId) -> Option<Polarity> {
        let ast::Expr::Binary {
            op: ast::BinaryOp::Eq,
            lhs,
            rhs,
            ..
        } = e
        else {
            return None;
        };
        let ast::Expr::Name(n) = lhs.as_ref() else {
            return None;
        };
        if self.signal_net(n)? != net {
            return None;
        }
        match self.eval(rhs).and_then(|v| v.as_enum()) {
            // `std_ulogic` positions: '0' is 2, '1' is 3; `bit`: 0 and 1.
            Some(3 | 1) => Some(Polarity::Pos),
            Some(2 | 0) => Some(Polarity::Neg),
            _ => None,
        }
    }

    /// The asynchronous reset edge a condition tests.
    fn reset_edge(&mut self, e: &'a ast::Expr) -> Option<Edge> {
        match e {
            ast::Expr::Paren { inner, .. } => self.reset_edge(inner),
            ast::Expr::Name(n) => {
                let net = self.signal_net(n)?;
                Some(Edge::pos(net))
            }
            ast::Expr::Binary {
                op: ast::BinaryOp::Eq,
                lhs,
                ..
            } => {
                let ast::Expr::Name(n) = lhs.as_ref() else {
                    return None;
                };
                let net = self.signal_net(n)?;
                let polarity = self.level_polarity(e, net)?;
                Some(Edge { net, polarity })
            }
            ast::Expr::Unary {
                op: ast::UnaryOp::Not,
                operand,
                ..
            } => {
                let net = match operand.as_ref() {
                    ast::Expr::Name(n) => self.signal_net(n)?,
                    _ => return None,
                };
                Some(Edge::neg(net))
            }
            _ => None,
        }
    }

    /// The net a simple signal name denotes.
    pub(crate) fn signal_net(&mut self, n: &'a ast::Name) -> Option<NetId> {
        let d = self.a().decl_of(n.span())?;
        match self.lookup(d) {
            Some(Binding::Net { net, .. }) => Some(*net),
            _ => None,
        }
    }

    // --- concurrent assignments ---------------------------------------------

    fn concurrent_assign(&mut self, sa: &'a ast::SignalAssignment) {
        let Some((target, layout)) = self.lvalue(&sa.target, &mut Sink::Cont) else {
            return;
        };
        match &sa.rhs {
            ast::SignalAssignmentRhs::Simple(ast::Waveform::Elements(elems))
                if elems.len() == 1 =>
            {
                let el = &elems[0];
                let value = self.expr_in(&el.value, &layout, &mut Sink::Cont);
                let delay = el.after.as_ref().and_then(|d| self.delay_of(d));
                self.emit_assign(&target, value, delay, sa.span);
            }
            ast::SignalAssignmentRhs::Simple(ast::Waveform::Unaffected(_)) => {}
            ast::SignalAssignmentRhs::Conditional(arms) if arms.iter().all(simple_waveform) => {
                let Some(value) = self.conditional_value(arms, &layout) else {
                    return;
                };
                self.emit_assign(&target, value, None, sa.span);
            }
            ast::SignalAssignmentRhs::Selected {
                selector,
                matching,
                arms,
            } if arms.iter().all(simple_selected) && !*matching => {
                let Some(value) = self.selected_value(selector, arms, &layout) else {
                    return;
                };
                self.emit_assign(&target, value, None, sa.span);
            }
            _ => {
                // Delays or several waveform elements need statements.
                let index = self.b.module().processes.len();
                let outer = self.driver;
                self.driver = DriverKey::Process(index);
                let mut p = self.b.process(None, ProcessKind::Comb);
                p.span = sa.span;
                self.signal_assignment(sa, AssignKind::NonBlocking, &mut p);
                let pid = self.b.end_process(p);
                self.attribute_drivers(pid);
                self.driver = outer;
            }
        }
    }

    /// Emits one continuous assignment, or a tristate cell when the value
    /// is the `x when en else 'Z'` shape.
    fn emit_assign(
        &mut self,
        target: &Lvalue,
        value: ExprId,
        delay: Option<crate::ir::Delay>,
        span: Span,
    ) {
        if delay.is_none()
            && let Lvalue::Net(net) = target
            && let Some((data, enable)) = self.tristate_shape(value)
        {
            let name = self.fresh("tri");
            let index = self.b.module().cells.len();
            self.driver = DriverKey::Cell(index);
            self.b.span = span;
            let cell = self.b.cell(
                name,
                CellKind::Tristate,
                vec![(Name::new("a"), data), (Name::new("en"), enable)],
                vec![(Name::new("y"), *net)],
            );
            self.record_driver(*net, DriverSite::Cell(cell, Name::new("y")), true, span);
            return;
        }
        let index = self.b.module().assigns.len();
        self.driver = DriverKey::Continuous(index);
        self.b.span = span;
        self.b.assign_after(target.clone(), value, delay);
        self.note_target(target, DriverSite::Assign(index), span);
    }

    /// `mux(en, data, 'Z')`, the shape of a bus driver.
    fn tristate_shape(&self, value: ExprId) -> Option<(ExprId, ExprId)> {
        let ExprKind::Ternary { cond, then_, else_ } = &self.b.module().expr(value).kind else {
            return None;
        };
        let zero = self.b.module().expr(*else_).as_const()?;
        let width = self.b.module().expr(*then_).ty.width()?;
        if *zero != Const::z(width) {
            return None;
        }
        Some((*then_, *cond))
    }

    /// A conditional assignment as a chain of `Ternary` nodes.
    fn conditional_value(
        &mut self,
        arms: &'a [ast::ConditionalWaveform],
        layout: &Layout,
    ) -> Option<ExprId> {
        let (first, rest) = arms.split_first()?;
        let value = self.waveform_value(&first.waveform, layout)?;
        let Some(cond) = &first.condition else {
            return Some(value);
        };
        let c = self.cond(cond, &mut Sink::Cont);
        let other = match rest.is_empty() {
            true => self.zero_of(layout),
            false => self.conditional_value(rest, layout)?,
        };
        self.b.span = first.span;
        Some(self.b.mux(c, value, other))
    }

    /// A selected assignment as a chain of equality tests: the expression
    /// form of a `case`.
    fn selected_value(
        &mut self,
        selector: &'a ast::Expr,
        arms: &'a [ast::SelectedWaveform],
        layout: &Layout,
    ) -> Option<ExprId> {
        let (subject, sub_layout) = self.expr(selector, &mut Sink::Cont)?;
        let mut default = None;
        let mut pairs: Vec<(Vec<ExprId>, ExprId)> = Vec::new();
        for arm in arms {
            let value = self.waveform_value(&arm.waveform, layout)?;
            if arm
                .choices
                .iter()
                .any(|c| matches!(c, ast::Choice::Others(_)))
            {
                default = Some(value);
                continue;
            }
            let values = self.choice_values(&arm.choices, &sub_layout);
            pairs.push((values, value));
        }
        let mut acc = default.unwrap_or_else(|| self.zero_of(layout));
        for (values, value) in pairs.into_iter().rev() {
            let mut cond = None;
            for v in values {
                self.b.span = selector.span();
                let eq = self.b.eq(subject, v);
                cond = Some(match cond {
                    Some(c) => self.b.lor(c, eq),
                    None => eq,
                });
            }
            if let Some(c) = cond {
                self.b.span = selector.span();
                acc = self.b.mux(c, value, acc);
            }
        }
        Some(acc)
    }

    fn waveform_value(&mut self, w: &'a ast::Waveform, layout: &Layout) -> Option<ExprId> {
        let ast::Waveform::Elements(elems) = w else {
            return None;
        };
        let el = elems.first()?;
        Some(self.expr_in(&el.value, layout, &mut Sink::Cont))
    }

    // --- blocks and generate ------------------------------------------------

    fn block(&mut self, label: Option<&str>, b: &'a ast::BlockStatement) {
        if !b.generics.is_empty() || !b.ports.is_empty() {
            self.unsupported(b.span, "a block with its own generics or ports");
            return;
        }
        if b.guard.is_some() {
            self.unsupported(b.span, "a guarded block");
            return;
        }
        let saved = self.prefix.clone();
        if let Some(l) = label {
            self.prefix = format!("{saved}{l}.");
        }
        self.push_scope();
        let configs = self.configs.len();
        self.declarations(&b.decls);
        self.collect_configurations(&b.decls);
        self.concurrent_statements(&b.statements);
        self.configs.truncate(configs);
        self.pop_scope();
        self.prefix = saved;
    }

    fn for_generate(&mut self, label: Option<&str>, g: &'a ast::ForGenerate) {
        let Some((left, right)) = self.static_range(&g.range) else {
            self.error(
                codes::NOT_STATIC,
                g.range.span(),
                "a generate range must be static after elaboration",
            );
            return;
        };
        let dir = self.range_direction(&g.range);
        let count = match dir {
            ast::Direction::To => right - left + 1,
            ast::Direction::Downto => left - right + 1,
        };
        if count > GENERATE_LIMIT {
            self.error(
                codes::LIMIT,
                g.span,
                format!("this generate unrolls to {count} copies, more than {GENERATE_LIMIT}"),
            );
            return;
        }
        let param = self.cx.object_decl_at(g.param.span);
        let ty = param
            .and_then(|d| self.a().decl_type(d))
            .unwrap_or(self.a().builtins.integer);
        let label = label.unwrap_or("gen").to_owned();
        let saved = self.prefix.clone();
        let mut i = left;
        for _ in 0..count.max(0) {
            self.prefix = format!("{saved}{label}({i}).");
            self.push_scope();
            if let Some(d) = param {
                self.bind(
                    d,
                    Binding::Value {
                        value: Value::Int(i128::from(i)),
                        ty,
                    },
                );
            }
            self.generate_body(&g.body);
            self.pop_scope();
            i += match dir {
                ast::Direction::To => 1,
                ast::Direction::Downto => -1,
            };
        }
        self.prefix = saved;
    }

    fn if_generate(&mut self, label: Option<&str>, g: &'a ast::IfGenerate) {
        let label = label.unwrap_or("gen").to_owned();
        for arm in &g.arms {
            let Some(v) = self.eval(&arm.condition).and_then(|v| v.as_bool()) else {
                self.error(
                    codes::NOT_STATIC,
                    arm.condition.span(),
                    "a generate condition must be static after elaboration",
                );
                return;
            };
            if v {
                self.generate_arm(&label, &arm.body);
                return;
            }
        }
        if let Some(body) = &g.else_arm {
            self.generate_arm(&label, body);
        }
    }

    fn case_generate(&mut self, label: Option<&str>, g: &'a ast::CaseGenerate) {
        let label = label.unwrap_or("gen").to_owned();
        let Some(sel) = self.eval(&g.expr) else {
            self.error(
                codes::NOT_STATIC,
                g.expr.span(),
                "a generate selector must be static after elaboration",
            );
            return;
        };
        let mut fallback = None;
        for arm in &g.arms {
            for c in &arm.choices {
                match c {
                    ast::Choice::Others(_) => fallback = Some(&arm.body),
                    ast::Choice::Expr(e) => {
                        if self.eval(e) == Some(sel.clone()) {
                            self.generate_arm(&label, &arm.body);
                            return;
                        }
                    }
                    ast::Choice::Range(r) => {
                        if let (Some((l, rr)), Some(v)) = (self.static_range(r), sel.as_int()) {
                            let (lo, hi) = if l <= rr { (l, rr) } else { (rr, l) };
                            if v >= i128::from(lo) && v <= i128::from(hi) {
                                self.generate_arm(&label, &arm.body);
                                return;
                            }
                        }
                    }
                }
            }
        }
        if let Some(body) = fallback {
            self.generate_arm(&label, body);
        }
    }

    /// Lowers the chosen arm of an `if`/`case` generate under its prefix.
    fn generate_arm(&mut self, label: &str, body: &'a ast::GenerateBody) {
        let saved = self.prefix.clone();
        self.prefix = match &body.label {
            Some(alt) => format!("{saved}{label}({}).", alt.name),
            None => format!("{saved}{label}."),
        };
        self.push_scope();
        self.generate_body(body);
        self.pop_scope();
        self.prefix = saved;
    }

    fn generate_body(&mut self, body: &'a ast::GenerateBody) {
        let configs = self.configs.len();
        self.declarations(&body.decls);
        self.collect_configurations(&body.decls);
        self.concurrent_statements(&body.statements);
        self.configs.truncate(configs);
    }

    // --- instances -----------------------------------------------------------

    fn instantiation(&mut self, label: Option<&str>, inst: &'a ast::Instantiation, span: Span) {
        let Some(label) = label else {
            self.error(codes::UNSUPPORTED, span, "an instance needs a label");
            return;
        };
        let Some(target) = self.bind_instance(label, inst, span) else {
            return;
        };
        let entity = target.entity;
        let formal_generics = match target.component {
            Some(c) => component_generics(self.a(), c),
            None => self.cx.generics_of(entity),
        };
        let formal_ports = match target.component {
            Some(c) => component_ports(self.a(), c),
            None => self.cx.ports_of(entity),
        };
        let mut generics = self.map_generics(entity, &formal_generics, inst.generic_map.as_deref());
        if let Some(extra) = target.generic_map {
            let mut more = self.map_generics(entity, &formal_generics, Some(extra));
            for g in more.drain(..) {
                if !generics.iter().any(|x| x.decl == g.decl) {
                    generics.push(g);
                }
            }
        }
        let Some(module) = self
            .cx
            .elaborate_entity(entity, target.architecture, generics, span)
        else {
            return;
        };

        let index = self.b.module().instances.len();
        let outer = self.driver;
        self.driver = DriverKey::Instance(index);
        let name = format!("{}{label}", self.prefix);
        let connections = self.map_ports(module, &formal_ports, inst.port_map.as_deref(), span);
        self.b.span = span;
        let id = self
            .b
            .instance(name, ModuleRef::Resolved(module), connections.list);
        for (net, port, whole) in connections.drivers {
            self.record_driver(net, DriverSite::Instance(id, port), whole, span);
        }
        if let Some(c) = target.component {
            self.apply_attributes(c, AttrTarget::Instance(id));
        }
        self.driver = outer;
    }

    /// Resolves what an instantiation binds to.
    fn bind_instance(
        &mut self,
        label: &str,
        inst: &'a ast::Instantiation,
        span: Span,
    ) -> Option<Target<'a>> {
        match &inst.unit {
            ast::InstantiatedUnit::Entity { name, architecture } => {
                let entity = self.entity_of_name(name)?;
                let arch = architecture.as_ref().and_then(|a| {
                    let sym = self.a().interner.get_ci(&a.name)?;
                    self.cx.index.architecture_named(self.a(), entity, sym)
                });
                Some(Target {
                    entity,
                    architecture: arch,
                    component: None,
                    generic_map: None,
                })
            }
            ast::InstantiatedUnit::Configuration(name) => {
                let d = self.a().decl_of(name.span())?;
                let DeclKind::Unit { unit, .. } = self.a().decl(d).kind else {
                    return None;
                };
                self.target_of_configuration(unit, span)
            }
            ast::InstantiatedUnit::Component(name) => {
                let d = self.a().decl_of(name.span());
                if let Some(d) = d
                    && let DeclKind::Unit { unit, .. } = self.a().decl(d).kind
                {
                    return Some(Target {
                        entity: unit,
                        architecture: None,
                        component: None,
                        generic_map: None,
                    });
                }
                let Some(component) = d else {
                    self.error(codes::UNBOUND, span, "this component is not declared");
                    return None;
                };
                // A configuration specification for this instance wins
                // over the default binding.
                if let Some(t) = self.configured_binding(label, component, span) {
                    return Some(t);
                }
                let spelling = self.a().decl(component).spelling.clone();
                let lib = self.a().interner.get_ci(self.cx.opts.library_name());
                let sym = self.a().interner.get_ci(&spelling);
                let entity = match (lib, sym) {
                    (Some(l), Some(s)) => self.cx.index.entity(self.a(), l, s),
                    _ => None,
                };
                let Some(entity) = entity else {
                    self.report(
                        Diagnostic::error(format!(
                            "component `{spelling}` binds to no entity"
                        ))
                        .with_code(codes::UNBOUND)
                        .with_span(span)
                        .with_note(
                            "the default binding needs an entity of the same name in the working library, or a configuration specification",
                        ),
                    );
                    return None;
                };
                Some(Target {
                    entity,
                    architecture: None,
                    component: Some(component),
                    generic_map: None,
                })
            }
        }
    }

    /// The binding a configuration specification gives this instance.
    fn configured_binding(
        &mut self,
        label: &str,
        component: DeclId,
        span: Span,
    ) -> Option<Target<'a>> {
        let specs: Vec<&'a ast::ConfigurationSpec> = self.configs.clone();
        for c in specs.iter().rev() {
            if !self.spec_matches(c, label, component) {
                continue;
            }
            let aspect = c.binding.entity_aspect.as_ref()?;
            let ast::EntityAspect::Entity {
                name, architecture, ..
            } = aspect
            else {
                continue;
            };
            let entity = self.entity_of_name(name)?;
            let arch = architecture.as_ref().and_then(|a| {
                let sym = self.a().interner.get_ci(&a.name)?;
                self.cx.index.architecture_named(self.a(), entity, sym)
            });
            let _ = span;
            return Some(Target {
                entity,
                architecture: arch,
                component: Some(component),
                generic_map: c.binding.generic_map.as_deref(),
            });
        }
        None
    }

    fn spec_matches(
        &mut self,
        c: &'a ast::ConfigurationSpec,
        label: &str,
        component: DeclId,
    ) -> bool {
        // The component of a configuration specification may or may not
        // have a recorded declaration; fall back to the written name.
        let names_component = match self.a().decl_of(c.spec.component.span()) {
            Some(d) => d == component,
            None => match c.spec.component.root() {
                ast::Name::Simple(i) => i
                    .name
                    .eq_ignore_ascii_case(&self.a().decl(component).spelling),
                _ => false,
            },
        };
        if !names_component {
            return false;
        }
        match &c.spec.instances {
            ast::InstantiationList::All(_) | ast::InstantiationList::Others(_) => true,
            ast::InstantiationList::Labels(ls) => {
                ls.iter().any(|l| l.name.eq_ignore_ascii_case(label))
            }
        }
    }

    /// The entity and architecture a configuration declaration names.
    fn target_of_configuration(&mut self, unit: UnitId, span: Span) -> Option<Target<'a>> {
        let ast::LibraryUnit::Configuration(c) = self.cx.unit_ast(unit)? else {
            return None;
        };
        let entity = self.entity_of_name(&c.entity)?;
        let arch = match &c.block.spec {
            ast::Name::Simple(i) => {
                let sym = self.a().interner.get_ci(&i.name)?;
                self.cx.index.architecture_named(self.a(), entity, sym)
            }
            _ => None,
        };
        let _ = span;
        Some(Target {
            entity,
            architecture: arch,
            component: None,
            generic_map: None,
        })
    }

    /// The entity a name denotes: `e`, `work.e` or `lib.e`.
    fn entity_of_name(&mut self, name: &'a ast::Name) -> Option<UnitId> {
        if let Some(d) = self.a().decl_of(name.span())
            && let DeclKind::Unit { unit, .. } = self.a().decl(d).kind
        {
            return Some(unit);
        }
        let (library, entity) = split_entity_name(name)?;
        let library = if library.eq_ignore_ascii_case("work") {
            self.cx.opts.library_name().to_owned()
        } else {
            library
        };
        let lib = self.a().interner.get_ci(&library)?;
        let sym = self.a().interner.get_ci(&entity)?;
        self.cx.index.entity(self.a(), lib, sym)
    }

    /// The declaration a formal part names, looking through a type
    /// conversion (`std_logic_vector(q) => actual`).
    fn formal_decl(&self, f: &'a ast::Expr) -> Option<DeclId> {
        if let Some(d) = self.a().decl_of(f.span()) {
            return Some(d);
        }
        let ast::Expr::Name(ast::Name::Call { args, .. }) = f else {
            return None;
        };
        let inner = super::expr::first_expr(args)?;
        self.a().decl_of(inner.span())
    }

    /// Evaluates a generic map against the target entity's generics.
    fn map_generics(
        &mut self,
        entity: UnitId,
        formals: &[DeclId],
        map: Option<&'a [ast::AssociationElement]>,
    ) -> Vec<GenericValue> {
        let Some(map) = map else { return Vec::new() };
        let entity_generics = self.cx.generics_of(entity);
        let mut out = Vec::new();
        let mut position = 0usize;
        for el in map {
            let formal = match &el.formal {
                Some(f) => self.formal_decl(f),
                None => {
                    let d = formals.get(position).copied();
                    position += 1;
                    d
                }
            };
            let Some(formal) = formal else { continue };
            let name = self.a().decl(formal).name;
            let Some(&target) = entity_generics
                .iter()
                .find(|&&d| self.a().decl(d).name == name)
            else {
                continue;
            };
            let ast::Actual::Expr(e) = &el.actual else {
                continue;
            };
            let Some(value) = self.eval(e) else {
                self.error(
                    codes::NOT_STATIC,
                    e.span(),
                    "a generic value must be static after elaboration",
                );
                continue;
            };
            out.push(GenericValue {
                decl: target,
                value,
            });
        }
        out
    }

    /// Builds the port connections of an instance.
    fn map_ports(
        &mut self,
        module: crate::ir::ModuleId,
        formals: &[DeclId],
        map: Option<&'a [ast::AssociationElement]>,
        span: Span,
    ) -> Connections {
        let mut out = Connections::default();
        let Some(map) = map else { return out };
        let mut position = 0usize;
        for el in map {
            let formal = match &el.formal {
                Some(f) => self.formal_decl(f),
                None => {
                    let d = formals.get(position).copied();
                    position += 1;
                    d
                }
            };
            let Some(formal) = formal else { continue };
            let spelling = self.a().decl(formal).spelling.clone();
            let DeclKind::Object { mode, ty, .. } = self.a().decl(formal).kind else {
                continue;
            };
            let dir = match mode.unwrap_or(Mode::In) {
                Mode::In => PortDir::In,
                Mode::Out | Mode::Buffer => PortDir::Out,
                Mode::Inout => PortDir::InOut,
                Mode::Linkage => continue,
            };
            let Some(port) = self
                .cx
                .design
                .module(module)
                .ports
                .iter()
                .find(|p| p.name.as_str().eq_ignore_ascii_case(&spelling))
                .cloned()
            else {
                self.error(
                    codes::PORT_MAP,
                    el.span,
                    format!("the instantiated entity has no port `{spelling}`"),
                );
                continue;
            };
            let port_ty = self.cx.design.module(module).nets[port.net].ty.clone();
            match &el.actual {
                ast::Actual::Open(_) => {}
                ast::Actual::Range(r) => {
                    self.unsupported(r.span(), "a range as a port actual");
                }
                ast::Actual::Expr(e) | ast::Actual::Inertial(e) => {
                    if matches!(e, ast::Expr::Open(_)) {
                        continue;
                    }
                    // The formal's subtype may mention the target's own
                    // generics, which mean nothing here; the width that
                    // matters is the one the target's net has.
                    let width = port_ty.width().unwrap_or(1);
                    let layout = match self.layout_of_type_quiet(ty, el.span) {
                        Some(l) => Layout {
                            width,
                            signed: port_ty.is_signed(),
                            ..l
                        },
                        None => Layout {
                            ty,
                            kind: LayoutKind::Opaque,
                            width,
                            signed: port_ty.is_signed(),
                        },
                    };
                    if dir == PortDir::In {
                        let value = self.expr_in(e, &layout, &mut Sink::Cont);
                        out.list.push((port.name.clone(), value));
                        continue;
                    }
                    // An output must be connected to something drivable.
                    let ast::Expr::Name(n) = e else {
                        self.error(
                            codes::PORT_MAP,
                            e.span(),
                            "an `out` or `inout` port must be connected to a signal",
                        );
                        continue;
                    };
                    let Some((lv, _)) = self.lvalue_name(n, &mut Sink::Cont) else {
                        continue;
                    };
                    let Some(value) = self.lvalue_expr(&lv) else {
                        self.error(
                            codes::PORT_MAP,
                            e.span(),
                            "this actual cannot be driven by an output port",
                        );
                        continue;
                    };
                    let whole = matches!(lv, Lvalue::Net(_));
                    for net in lv.nets() {
                        out.drivers.push((net, port.name.clone(), whole));
                    }
                    out.list.push((port.name.clone(), value));
                }
            }
        }
        let _ = span;
        out
    }

    /// The expression that reads what an lvalue writes, for connecting an
    /// output port.
    fn lvalue_expr(&mut self, lv: &Lvalue) -> Option<ExprId> {
        match lv {
            Lvalue::Net(n) => Some(self.b.net(*n)),
            Lvalue::Slice { net, hi, lo } => {
                let base = self.b.net(*net);
                Some(self.b.slice(base, *hi, *lo))
            }
            Lvalue::Concat(parts) => {
                let mut ids = Vec::with_capacity(parts.len());
                for p in parts {
                    ids.push(self.lvalue_expr(p)?);
                }
                Some(self.b.concat(ids))
            }
            Lvalue::Index { .. } | Lvalue::MemElem { .. } => None,
        }
    }
}

/// What an instantiation binds to.
struct Target<'a> {
    entity: UnitId,
    architecture: Option<UnitId>,
    component: Option<DeclId>,
    generic_map: Option<&'a [ast::AssociationElement]>,
}

/// The port connections of one instance.
#[derive(Default)]
struct Connections {
    list: Vec<(Name, ExprId)>,
    drivers: Vec<(NetId, Name, bool)>,
}

/// The kind of a process and which of its statements make up the body.
struct ProcessShape<'a> {
    kind: ProcessKind,
    shape: Shape<'a>,
}

/// Which statements a process body is made of.
enum Shape<'a> {
    /// Every statement of the process.
    Plain,
    /// A sub-list (the body of the clocked `if`, or what follows the
    /// `wait until`).
    Body(&'a [ast::SequentialStatement]),
    /// An asynchronous reset: `if cond then reset else body end if`.
    Reset {
        cond: &'a ast::Expr,
        reset: &'a [ast::SequentialStatement],
        body: &'a [ast::SequentialStatement],
    },
    /// Every statement, wrapped in `forever`.
    Forever(&'a [ast::SequentialStatement]),
}

/// Splits `e`, `work.e` or `lib.e` into a library and an entity name;
/// a bare name means the working library.
pub(crate) fn split_entity_name(name: &ast::Name) -> Option<(String, String)> {
    match name {
        ast::Name::Simple(i) => Some(("work".to_owned(), i.name.clone())),
        ast::Name::Selected { prefix, suffix, .. } => {
            let ast::Suffix::Designator(ast::Designator::Ident(e)) = suffix else {
                return None;
            };
            let ast::Name::Simple(l) = prefix.as_ref() else {
                return None;
            };
            Some((l.name.clone(), e.name.clone()))
        }
        _ => None,
    }
}

/// The generics of a component declaration.
fn component_generics(a: &crate::vhdl::sema::Analysis, c: DeclId) -> Vec<DeclId> {
    match &a.decl(c).kind {
        DeclKind::Component { generics, .. } => generics.clone(),
        _ => Vec::new(),
    }
}

/// The ports of a component declaration.
fn component_ports(a: &crate::vhdl::sema::Analysis, c: DeclId) -> Vec<DeclId> {
    match &a.decl(c).kind {
        DeclKind::Component { ports, .. } => ports.clone(),
        _ => Vec::new(),
    }
}

/// True when a conditional waveform is one element without a delay.
fn simple_waveform(w: &ast::ConditionalWaveform) -> bool {
    match &w.waveform {
        ast::Waveform::Elements(e) => e.len() == 1 && e[0].after.is_none(),
        ast::Waveform::Unaffected(_) => false,
    }
}

/// True when a selected waveform is one element without a delay.
fn simple_selected(w: &ast::SelectedWaveform) -> bool {
    match &w.waveform {
        ast::Waveform::Elements(e) => e.len() == 1 && e[0].after.is_none(),
        ast::Waveform::Unaffected(_) => false,
    }
}

/// True when a statement suspends.
fn has_wait(s: &ast::SequentialStatement) -> bool {
    match &s.kind {
        ast::SequentialKind::Wait { .. } => true,
        ast::SequentialKind::If(i) => {
            i.arms.iter().any(|a| a.statements.iter().any(has_wait))
                || i.else_statements
                    .as_ref()
                    .is_some_and(|e| e.iter().any(has_wait))
        }
        ast::SequentialKind::Case(c) => c.arms.iter().any(|a| a.statements.iter().any(has_wait)),
        ast::SequentialKind::Loop(l) => l.statements.iter().any(has_wait),
        _ => false,
    }
}

/// Every net a block of statements assigns, with whether it is written
/// whole.
fn collect_targets(block: &[crate::ir::Stmt]) -> Vec<(NetId, bool)> {
    let mut out = Vec::new();
    walk_targets(block, &mut out);
    out
}

fn walk_targets(block: &[crate::ir::Stmt], out: &mut Vec<(NetId, bool)>) {
    for s in block {
        if let crate::ir::StmtKind::Assign { target, .. } = &s.kind {
            let whole = matches!(target, Lvalue::Net(_));
            for n in target.nets() {
                if !out.iter().any(|(m, w)| *m == n && *w == whole) {
                    out.push((n, whole));
                }
            }
        }
        for b in s.blocks() {
            walk_targets(b, out);
        }
    }
}
