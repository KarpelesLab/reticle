//! Lowering sequential statements, and inlining subprograms.
//!
//! The IR's statement set is close enough to VHDL's that most of this is a
//! direct translation; the four places where it is not are worth naming.
//!
//! **Assignment kinds.** A signal assignment is a non-blocking assignment
//! and a variable assignment a blocking one, which is exactly the VHDL
//! rule: a signal takes its new value at the end of the current delta
//! cycle, a variable immediately.
//!
//! **`report` is an assertion that always fires.** `report m severity s`
//! is `assert false report m severity s`, so it lowers to
//! [`crate::ir::StmtKind::Assert`] with a constant-false condition. An
//! `assert` keeps its condition. The default severity is `error` for an
//! assertion and `note` for a report, as clause 10.3 and 10.4 prescribe.
//!
//! **Loops.** `for p in r loop` becomes [`crate::ir::StmtKind::For`] with a
//! variable net holding the parameter, so `exit` and `next` stay
//! expressible as `break` and `continue`; `while` and the bare `loop`
//! become `While` and `Forever`. The IR's `break` and `continue` leave the
//! innermost loop only, so `exit`/`next` naming an outer loop is `V0701`.
//!
//! **Subprograms are inlined.** A call becomes: one variable net per
//! formal, an assignment of each `in` actual, the body's statements, and a
//! copy back of each `out` actual. A `signal`-class formal whose actual is
//! a plain signal is bound straight to that signal instead, so a procedure
//! that assigns to an `out signal` drives it directly. A function also
//! gets a result net, and `return e` assigns it and raises a "has
//! returned" flag that guards everything after the return. In a continuous
//! context the whole body goes into a generated combinational process, so
//! a function used in a concurrent assignment becomes a process and a net.
//! Recursion is `V0705`.

use crate::diag::Diagnostic;
use crate::ir::builder::BlockBuilder;
use crate::ir::{
    AssignKind, CaseArm, CaseKind, CaseQualifier, Delay, Edge, ExprId, Lvalue, NetId, NetKind,
    ProcessKind, ReportSeverity, TimeUnit,
};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast::{self, Mode};
use crate::vhdl::sema::{DeclId, DeclKind, ObjectClass, SubprogramBody};

use super::codes;
use super::expr::Actual;
use super::lower::{Binding, DriverSite, Frame, Lowerer, Sink};
use super::types::{Layout, LayoutKind};

impl<'a> Lowerer<'a, '_> {
    // --- statement lists ----------------------------------------------------

    /// Lowers a list of sequential statements.
    pub(crate) fn stmts(&mut self, list: &'a [ast::SequentialStatement], out: &mut BlockBuilder) {
        let flag = self.frames.last().and_then(|f| f.flag);
        self.stmts_guarded(list, out, flag);
    }

    /// Lowers statements, guarding everything after a possible `return`
    /// with `if not returned`.
    fn stmts_guarded(
        &mut self,
        list: &'a [ast::SequentialStatement],
        out: &mut BlockBuilder,
        flag: Option<NetId>,
    ) {
        for (i, s) in list.iter().enumerate() {
            if let Some(flag) = flag
                && i > 0
                && list[..i].iter().any(may_return)
            {
                let mut inner = self.b.block();
                inner.span = s.span;
                self.stmts_guarded(&list[i..], &mut inner, Some(flag));
                self.b.span = s.span;
                let f = self.b.net(flag);
                let cond = self.b.lnot(f);
                out.span = s.span;
                out.if_(cond, inner.finish(), Vec::new());
                return;
            }
            self.stmt(s, out);
        }
    }

    fn stmt(&mut self, s: &'a ast::SequentialStatement, out: &mut BlockBuilder) {
        self.b.span = s.span;
        out.span = s.span;
        match &s.kind {
            ast::SequentialKind::Null => {}
            ast::SequentialKind::Wait {
                sensitivity,
                condition,
                timeout,
            } => self.wait_stmt(
                sensitivity.as_ref(),
                condition.as_ref(),
                timeout.as_ref(),
                s.span,
                out,
            ),
            ast::SequentialKind::Assertion(a) => self.assertion(a, out),
            ast::SequentialKind::Report { message, severity } => {
                let sev = self.severity_of(severity.as_ref(), ReportSeverity::Note);
                let msg = self.report_args(message, out);
                self.b.span = s.span;
                let cond = self.b.const_bit(false);
                out.span = s.span;
                out.assert(cond, sev, msg);
            }
            ast::SequentialKind::SignalAssignment(sa) => {
                self.signal_assignment(sa, AssignKind::NonBlocking, out);
            }
            ast::SequentialKind::VariableAssignment(va) => self.variable_assignment(va, out),
            ast::SequentialKind::ProcedureCall(call) => {
                self.procedure_call(call, out);
            }
            ast::SequentialKind::If(i) => self.if_stmt(i, out),
            ast::SequentialKind::Case(c) => self.case_stmt(c, out),
            ast::SequentialKind::Loop(l) => self.loop_stmt(s.label.as_ref(), l, out),
            ast::SequentialKind::Next { label, condition } => {
                self.jump(label.as_ref(), condition.as_ref(), false, s.span, out);
            }
            ast::SequentialKind::Exit { label, condition } => {
                self.jump(label.as_ref(), condition.as_ref(), true, s.span, out);
            }
            ast::SequentialKind::Return(e) => self.return_stmt(e.as_ref(), s.span, out),
        }
    }

    // --- individual statements ----------------------------------------------

    fn wait_stmt(
        &mut self,
        sensitivity: Option<&'a ast::Sensitivity>,
        condition: Option<&'a ast::Expr>,
        timeout: Option<&'a ast::Expr>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        out.span = span;
        if let Some(c) = condition {
            let cond = self.cond(c, &mut Sink::Proc(out));
            out.span = span;
            out.wait_until(cond);
            return;
        }
        if let Some(t) = timeout {
            let d = self.time_expr(t, out);
            out.span = span;
            out.wait_delay(d);
            return;
        }
        if let Some(ast::Sensitivity::Names(names)) = sensitivity {
            let mut edges = Vec::new();
            for n in names {
                if let Some(net) = self.signal_net(n) {
                    edges.push(Edge::any(net));
                }
            }
            if !edges.is_empty() {
                out.span = span;
                out.wait_event(edges);
                return;
            }
        }
        // A bare `wait;` suspends for ever.
        self.b.span = span;
        let never = self.b.const_bit(false);
        out.span = span;
        out.wait_until(never);
    }

    fn assertion(&mut self, a: &'a ast::Assertion, out: &mut BlockBuilder) {
        let cond = self.cond(&a.condition, &mut Sink::Proc(out));
        let sev = self.severity_of(a.severity.as_ref(), ReportSeverity::Error);
        let msg = match &a.report {
            Some(m) => self.report_args(m, out),
            None => Vec::new(),
        };
        out.span = a.span;
        out.assert(cond, sev, msg);
    }

    /// The report arguments of an assertion: one expression, usually a
    /// string.
    fn report_args(&mut self, e: &'a ast::Expr, out: &mut BlockBuilder) -> Vec<ExprId> {
        match self.expr(e, &mut Sink::Proc(out)) {
            Some((id, _)) => vec![id],
            None => Vec::new(),
        }
    }

    /// The severity an expression names.
    pub(crate) fn severity_of(
        &mut self,
        e: Option<&'a ast::Expr>,
        default: ReportSeverity,
    ) -> ReportSeverity {
        let Some(e) = e else { return default };
        match self.eval(e).and_then(|v| v.as_enum()) {
            Some(0) => ReportSeverity::Note,
            Some(1) => ReportSeverity::Warning,
            Some(2) => ReportSeverity::Error,
            Some(3) => ReportSeverity::Failure,
            _ => default,
        }
    }

    /// Lowers a signal assignment, in a process or concurrently.
    pub(crate) fn signal_assignment(
        &mut self,
        sa: &'a ast::SignalAssignment,
        kind: AssignKind,
        out: &mut BlockBuilder,
    ) {
        let Some((target, layout)) = self.lvalue(&sa.target, &mut Sink::Proc(out)) else {
            return;
        };
        match &sa.rhs {
            ast::SignalAssignmentRhs::Simple(w) => {
                self.waveform(&target, &layout, w, kind, sa.span, out);
            }
            ast::SignalAssignmentRhs::Conditional(arms) => {
                self.conditional_waveform(&target, &layout, arms, kind, sa.span, out);
            }
            ast::SignalAssignmentRhs::Selected {
                selector,
                matching,
                arms,
            } => {
                self.selected_waveform(
                    &target, &layout, selector, *matching, arms, kind, sa.span, out,
                );
            }
            ast::SignalAssignmentRhs::Force { .. } => {
                self.unsupported(sa.span, "a `force` assignment");
            }
            ast::SignalAssignmentRhs::Release { .. } => {
                self.unsupported(sa.span, "a `release` assignment");
            }
        }
    }

    fn waveform(
        &mut self,
        target: &Lvalue,
        layout: &Layout,
        w: &'a ast::Waveform,
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let ast::Waveform::Elements(elems) = w else {
            // `unaffected` keeps the current value: nothing to emit.
            return;
        };
        for el in elems {
            let value = self.expr_in(&el.value, layout, &mut Sink::Proc(out));
            let delay = el.after.as_ref().and_then(|d| self.delay_of(d));
            out.span = span;
            match delay {
                Some(d) => out.assign_after(target.clone(), value, kind, d),
                None => out.assign(target.clone(), value, kind),
            }
        }
    }

    fn conditional_waveform(
        &mut self,
        target: &Lvalue,
        layout: &Layout,
        arms: &'a [ast::ConditionalWaveform],
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        // `a when c else b when d else e` becomes nested `if`s.
        let Some((first, rest)) = arms.split_first() else {
            return;
        };
        let Some(cond) = &first.condition else {
            self.waveform(target, layout, &first.waveform, kind, span, out);
            return;
        };
        let c = self.cond(cond, &mut Sink::Proc(out));
        let mut then_ = self.b.block();
        then_.span = span;
        self.waveform(target, layout, &first.waveform, kind, span, &mut then_);
        let mut else_ = self.b.block();
        else_.span = span;
        if !rest.is_empty() {
            self.conditional_waveform(target, layout, rest, kind, span, &mut else_);
        }
        out.span = span;
        out.if_(c, then_.finish(), else_.finish());
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per syntactic part"
    )]
    fn selected_waveform(
        &mut self,
        target: &Lvalue,
        layout: &Layout,
        selector: &'a ast::Expr,
        matching: bool,
        arms: &'a [ast::SelectedWaveform],
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some((subject, sub_layout)) = self.expr(selector, &mut Sink::Proc(out)) else {
            return;
        };
        let mut ir_arms = Vec::new();
        let mut default = None;
        for arm in arms {
            let mut body = self.b.block();
            body.span = arm.span;
            self.waveform(target, layout, &arm.waveform, kind, arm.span, &mut body);
            let body = body.finish();
            if arm
                .choices
                .iter()
                .any(|c| matches!(c, ast::Choice::Others(_)))
            {
                default = Some(body);
                continue;
            }
            let values = self.choice_values(&arm.choices, &sub_layout);
            ir_arms.push(CaseArm { values, body });
        }
        out.span = span;
        out.case(
            subject,
            if matching {
                CaseKind::Z
            } else {
                CaseKind::Plain
            },
            CaseQualifier::None,
            ir_arms,
            default,
        );
    }

    fn variable_assignment(&mut self, va: &'a ast::VariableAssignment, out: &mut BlockBuilder) {
        let Some((target, layout)) = self.lvalue(&va.target, &mut Sink::Proc(out)) else {
            return;
        };
        match &va.rhs {
            ast::VariableAssignmentRhs::Simple(e) => {
                let value = self.expr_in(e, &layout, &mut Sink::Proc(out));
                out.span = va.span;
                out.assign(target, value, AssignKind::Blocking);
            }
            ast::VariableAssignmentRhs::Conditional(arms) => {
                self.conditional_expr(&target, &layout, arms, va.span, out);
            }
            ast::VariableAssignmentRhs::Selected {
                selector,
                matching,
                arms,
            } => {
                let Some((subject, sub_layout)) = self.expr(selector, &mut Sink::Proc(out)) else {
                    return;
                };
                let mut ir_arms = Vec::new();
                let mut default = None;
                for arm in arms {
                    let value = self.expr_in(&arm.value, &layout, &mut Sink::Proc(out));
                    let mut body = self.b.block();
                    body.span = arm.span;
                    body.assign(target.clone(), value, AssignKind::Blocking);
                    let body = body.finish();
                    if arm
                        .choices
                        .iter()
                        .any(|c| matches!(c, ast::Choice::Others(_)))
                    {
                        default = Some(body);
                        continue;
                    }
                    let values = self.choice_values(&arm.choices, &sub_layout);
                    ir_arms.push(CaseArm { values, body });
                }
                out.span = va.span;
                out.case(
                    subject,
                    if *matching {
                        CaseKind::Z
                    } else {
                        CaseKind::Plain
                    },
                    CaseQualifier::None,
                    ir_arms,
                    default,
                );
            }
        }
    }

    fn conditional_expr(
        &mut self,
        target: &Lvalue,
        layout: &Layout,
        arms: &'a [ast::ConditionalExpr],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some((first, rest)) = arms.split_first() else {
            return;
        };
        let value = self.expr_in(&first.value, layout, &mut Sink::Proc(out));
        let Some(cond) = &first.condition else {
            out.span = span;
            out.assign(target.clone(), value, AssignKind::Blocking);
            return;
        };
        let c = self.cond(cond, &mut Sink::Proc(out));
        let mut then_ = self.b.block();
        then_.span = span;
        then_.assign(target.clone(), value, AssignKind::Blocking);
        let mut else_ = self.b.block();
        else_.span = span;
        if !rest.is_empty() {
            self.conditional_expr(target, layout, rest, span, &mut else_);
        }
        out.span = span;
        out.if_(c, then_.finish(), else_.finish());
    }

    fn if_stmt(&mut self, i: &'a ast::IfStatement, out: &mut BlockBuilder) {
        self.if_arms(&i.arms, i.else_statements.as_deref(), i.span, out);
    }

    fn if_arms(
        &mut self,
        arms: &'a [ast::IfArm],
        else_: Option<&'a [ast::SequentialStatement]>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some((first, rest)) = arms.split_first() else {
            if let Some(stmts) = else_ {
                self.stmts(stmts, out);
            }
            return;
        };
        let c = self.cond(&first.condition, &mut Sink::Proc(out));
        let mut then_ = self.b.block();
        then_.span = first.span;
        self.stmts(&first.statements, &mut then_);
        let mut otherwise = self.b.block();
        otherwise.span = span;
        if rest.is_empty() {
            if let Some(stmts) = else_ {
                self.stmts(stmts, &mut otherwise);
            }
        } else {
            self.if_arms(rest, else_, span, &mut otherwise);
        }
        out.span = span;
        out.if_(c, then_.finish(), otherwise.finish());
    }

    fn case_stmt(&mut self, c: &'a ast::CaseStatement, out: &mut BlockBuilder) {
        let Some((subject, layout)) = self.expr(&c.expr, &mut Sink::Proc(out)) else {
            return;
        };
        let mut arms = Vec::new();
        let mut default = None;
        for arm in &c.arms {
            let mut body = self.b.block();
            body.span = arm.span;
            self.stmts(&arm.statements, &mut body);
            let body = body.finish();
            if arm
                .choices
                .iter()
                .any(|ch| matches!(ch, ast::Choice::Others(_)))
            {
                default = Some(body);
                continue;
            }
            let values = self.choice_values(&arm.choices, &layout);
            arms.push(CaseArm { values, body });
        }
        out.span = c.span;
        out.case(
            subject,
            if c.matching {
                CaseKind::Z
            } else {
                CaseKind::Plain
            },
            CaseQualifier::None,
            arms,
            default,
        );
    }

    /// The constants a choice list selects.
    pub(crate) fn choice_values(
        &mut self,
        choices: &'a [ast::Choice],
        layout: &Layout,
    ) -> Vec<ExprId> {
        let mut out = Vec::new();
        for c in choices {
            match c {
                ast::Choice::Others(_) => {}
                ast::Choice::Expr(e) => {
                    let id = self.expr_in(e, layout, &mut Sink::Cont);
                    out.push(id);
                }
                ast::Choice::Range(r) => {
                    let Some((l, rr)) = self.static_range(r) else {
                        self.error(codes::NOT_STATIC, r.span(), "a choice range must be static");
                        continue;
                    };
                    let (lo, hi) = if l <= rr { (l, rr) } else { (rr, l) };
                    if hi - lo > 1024 {
                        self.error(
                            codes::LIMIT,
                            r.span(),
                            "this choice range covers more than 1024 values",
                        );
                        continue;
                    }
                    for v in lo..=hi {
                        let c = Logic::from_i64(v, layout.width.max(1)).with_signed(layout.signed);
                        out.push(self.b.constant(c));
                    }
                }
            }
        }
        out
    }

    fn loop_stmt(
        &mut self,
        label: Option<&'a ast::Ident>,
        l: &'a ast::LoopStatement,
        out: &mut BlockBuilder,
    ) {
        let name = label.map(|i| i.name.clone());
        match &l.scheme {
            None => {
                self.loops.push(name);
                let mut body = self.b.block();
                body.span = l.span;
                self.stmts(&l.statements, &mut body);
                self.loops.pop();
                out.span = l.span;
                out.forever(body.finish());
            }
            Some(ast::IterationScheme::While(c)) => {
                let cond = self.cond(c, &mut Sink::Proc(out));
                self.loops.push(name);
                let mut body = self.b.block();
                body.span = l.span;
                self.stmts(&l.statements, &mut body);
                self.loops.pop();
                out.span = l.span;
                out.while_(cond, body.finish());
            }
            Some(ast::IterationScheme::For { param, range }) => {
                self.for_loop(name, param, range, &l.statements, l.span, out);
            }
        }
    }

    fn for_loop(
        &mut self,
        label: Option<String>,
        param: &'a ast::Ident,
        range: &'a ast::DiscreteRange,
        body: &'a [ast::SequentialStatement],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some((left, right)) = self.static_range(range) else {
            self.error(
                codes::NOT_STATIC,
                range.span(),
                "a loop range must be static after elaboration",
            );
            return;
        };
        let dir = self.range_direction(range);
        let name = self.qualified(&param.name);
        let layout = Layout {
            kind: LayoutKind::Int,
            width: 32,
            signed: true,
            ..self
                .layout_at(range.span())
                .unwrap_or_else(|| self.int_layout())
        };
        let Some(var) = self.new_net(name, layout.clone(), false, span) else {
            return;
        };
        self.b.module_mut().nets[var].kind = NetKind::Variable;
        self.push_scope();
        if let Some(d) = self.cx.object_decl_at(param.span) {
            self.bind(
                d,
                Binding::Net {
                    net: var,
                    layout: layout.clone(),
                },
            );
        }
        self.b.span = span;
        let start = self.b.constant(Logic::from_i64(left, 32).with_signed(true));
        let stop = self
            .b
            .constant(Logic::from_i64(right, 32).with_signed(true));
        let v = self.b.net(var);
        let cond = match dir {
            ast::Direction::To => self.b.le(v, stop),
            ast::Direction::Downto => self.b.ge(v, stop),
        };
        let one = self.b.constant(Logic::from_i64(1, 32).with_signed(true));
        let v2 = self.b.net(var);
        let next = match dir {
            ast::Direction::To => self.b.add(v2, one),
            ast::Direction::Downto => self.b.sub(v2, one),
        };
        self.loops.push(label);
        let mut inner = self.b.block();
        inner.span = span;
        self.stmts(body, &mut inner);
        self.loops.pop();
        self.pop_scope();
        out.span = span;
        out.for_(
            Some((Lvalue::Net(var), start)),
            Some(cond),
            Some((Lvalue::Net(var), next)),
            inner.finish(),
        );
    }

    fn jump(
        &mut self,
        label: Option<&'a ast::Ident>,
        condition: Option<&'a ast::Expr>,
        exit: bool,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        if let Some(l) = label {
            let innermost = self.loops.last().cloned().flatten();
            if innermost
                .as_deref()
                .is_none_or(|n| !n.eq_ignore_ascii_case(&l.name))
            {
                self.unsupported(
                    span,
                    &format!(
                        "`{}` naming a loop other than the innermost one",
                        if exit { "exit" } else { "next" }
                    ),
                );
                return;
            }
        }
        let mut body = self.b.block();
        body.span = span;
        if exit {
            body.break_();
        } else {
            body.continue_();
        }
        match condition {
            Some(c) => {
                let cond = self.cond(c, &mut Sink::Proc(out));
                out.span = span;
                out.if_(cond, body.finish(), Vec::new());
            }
            None => {
                out.span = span;
                if exit {
                    out.break_();
                } else {
                    out.continue_();
                }
            }
        }
    }

    fn return_stmt(&mut self, e: Option<&'a ast::Expr>, span: Span, out: &mut BlockBuilder) {
        let Some(frame) = self.frames.last().cloned() else {
            self.unsupported(span, "`return` outside a subprogram");
            return;
        };
        if let (Some(e), Some(net), Some(layout)) = (e, frame.result, frame.result_layout.clone()) {
            let value = self.expr_in(e, &layout, &mut Sink::Proc(out));
            out.span = span;
            out.assign(Lvalue::Net(net), value, AssignKind::Blocking);
        }
        if let Some(flag) = frame.flag {
            self.b.span = span;
            let one = self.b.const_bit(true);
            out.span = span;
            out.assign(Lvalue::Net(flag), one, AssignKind::Blocking);
        }
    }

    // --- time ---------------------------------------------------------------

    /// A static delay, in the largest unit that divides it evenly.
    pub(crate) fn delay_of(&mut self, e: &'a ast::Expr) -> Option<Delay> {
        let fs = self.eval_int(e)?;
        let fs = u64::try_from(fs).ok()?;
        Some(delay_from_fs(fs))
    }

    /// A time-valued expression, as a femtosecond count.
    fn time_expr(&mut self, e: &'a ast::Expr, out: &mut BlockBuilder) -> ExprId {
        if let Some(fs) = self.eval_int(e).and_then(|v| i64::try_from(v).ok()) {
            self.b.span = e.span();
            return self.b.constant(Logic::from_i64(fs, 64).with_signed(true));
        }
        let layout = Layout {
            kind: LayoutKind::Physical,
            width: 64,
            signed: true,
            ty: self.a().builtins.time,
        };
        self.expr_in(e, &layout, &mut Sink::Proc(out))
    }

    // --- subprogram calls ---------------------------------------------------

    /// The declaration of a subprogram that has a body.
    ///
    /// A subprogram alias copies the profile of the subprogram it names at
    /// the point the alias is written, which in a package declaration is
    /// before the body exists; the same profile declared elsewhere with a
    /// body is the one to inline.
    fn subprogram_with_body(&self, d: DeclId) -> DeclId {
        // An alias carries a copy of the profile, so follow it first.
        let d = self.cx.alias_target.get(&d).copied().unwrap_or(d);
        let DeclKind::Subprogram { sig, body } = &self.a().decl(d).kind else {
            return d;
        };
        if !matches!(body, SubprogramBody::None) {
            return d;
        }
        let name = self.a().decl(d).name;
        for &cand in &self.cx.all_decls {
            if cand == d || self.a().decl(cand).name != name {
                continue;
            }
            let DeclKind::Subprogram {
                sig: other,
                body: other_body,
            } = &self.a().decl(cand).kind
            else {
                continue;
            };
            if !matches!(other_body, SubprogramBody::Vhdl(_))
                || other.params.len() != sig.params.len()
                || !other
                    .params
                    .iter()
                    .zip(&sig.params)
                    .all(|(x, y)| self.a().same_base(x.ty, y.ty))
            {
                continue;
            }
            let ret_ok = match (other.ret, sig.ret) {
                (None, None) => true,
                (Some(x), Some(y)) => self.a().same_base(x, y),
                _ => false,
            };
            if ret_ok {
                return cand;
            }
        }
        d
    }

    /// A concurrent or sequential procedure call.
    pub(crate) fn procedure_call(&mut self, call: &'a ast::Name, out: &mut BlockBuilder) {
        let span = call.span();
        let Some(d) = self.a().decl_of(span) else {
            self.error(codes::UNSUPPORTED, span, "this call cannot be lowered");
            return;
        };
        let args: Vec<Actual<'a>> = match call {
            ast::Name::Call { args, .. } => args.iter().map(Actual::Assoc).collect(),
            _ => Vec::new(),
        };
        self.inline(d, &args, span, None, out);
    }

    /// Inlines a function call, returning its result.
    pub(crate) fn inline_function(
        &mut self,
        d: DeclId,
        args: &[Actual<'a>],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        match sink {
            Sink::Proc(block) => {
                let mut b = std::mem::replace(*block, self.b.block());
                let r = self.inline(d, args, span, want, &mut b);
                **block = b;
                r
            }
            Sink::Cont => {
                let name = self.fresh("comb");
                let index = self.b.module().processes.len();
                let outer = self.driver;
                self.driver = super::lower::DriverKey::Process(index);
                let mut p = self.b.process(Some(&name), ProcessKind::Comb);
                p.span = span;
                let r = self.inline(d, args, span, want, &mut p);
                let pid = self.b.end_process(p);
                if let Some((id, _)) = &r
                    && let Some(net) = self.b.module().expr(*id).as_net()
                {
                    self.record_driver(net, DriverSite::Process(pid), true, span);
                }
                self.driver = outer;
                r
            }
        }
    }

    /// Inlines one subprogram call into an open block.
    #[allow(clippy::too_many_lines, reason = "one block per stage of inlining")]
    fn inline(
        &mut self,
        d: DeclId,
        args: &[Actual<'a>],
        span: Span,
        want: Option<&Layout>,
        out: &mut BlockBuilder,
    ) -> Option<(ExprId, Layout)> {
        let d = self.subprogram_with_body(super::lower::resolve_alias(self.a(), d));
        let spelling = self.a().decl(d).spelling.clone();
        if self.inlining.contains(&d) {
            self.report(
                Diagnostic::error(format!("`{spelling}` calls itself"))
                    .with_code(codes::RECURSION)
                    .with_label(span, "recursive call")
                    .with_secondary(self.a().decl(d).span, "declared here")
                    .with_note("a recursive subprogram cannot be inlined"),
            );
            return None;
        }
        let DeclKind::Subprogram { body, .. } = self.a().decl(d).kind.clone() else {
            return None;
        };
        let SubprogramBody::Vhdl(bspan) = body else {
            if self.report_not_bundled(d, span) {
                return None;
            }
            self.report(
                Diagnostic::error(format!("`{spelling}` has no body to inline"))
                    .with_code(codes::UNSUPPORTED)
                    .with_span(span)
                    .with_note("a `foreign` or undefined subprogram cannot be lowered"),
            );
            return None;
        };
        let Some(def) = self.cx.ast.body(bspan) else {
            self.unsupported(span, "this subprogram body");
            return None;
        };

        let formals = super::lower::interface_objects(&def.spec.params);
        let frame_name = self.fresh(&sanitise_call(&spelling));
        // Actuals are evaluated in the caller's scope, before the frame.
        let mut bound: Vec<(DeclId, FormalBinding)> = Vec::new();
        let mut copies: Vec<(DeclId, Lvalue, Layout)> = Vec::new();
        for (i, (ident, obj)) in formals.iter().enumerate() {
            let Some(fd) = self.cx.object_decl_at(ident.span) else {
                continue;
            };
            let DeclKind::Object {
                ty, class, mode, ..
            } = self.a().decl(fd).kind
            else {
                continue;
            };
            let mode = mode.unwrap_or(Mode::In);
            let actual = pick_actual(self, args, i, ident);
            // An unconstrained formal (`a : bit_vector`) takes its width
            // from the actual, as clause 5.3.2.2 prescribes.
            let layout = match self.layout_of_type_quiet(ty, ident.span) {
                Some(l) => l,
                None => {
                    let from_actual = actual
                        .and_then(Actual::expr)
                        .and_then(|e| self.layout_at(e.span()))
                        .or_else(|| want.cloned());
                    match from_actual {
                        Some(l) => l,
                        None => {
                            self.layout_of_type(ty, ident.span);
                            continue;
                        }
                    }
                }
            };
            let is_in = matches!(mode, Mode::In | Mode::Inout);
            let is_out = matches!(mode, Mode::Out | Mode::Inout | Mode::Buffer);
            // A `signal` formal bound to a plain signal is aliased, so an
            // assignment inside the body drives the caller's signal.
            if class == ObjectClass::Signal
                && let Some(ae) = actual.and_then(Actual::expr)
                && let ast::Expr::Name(n) = ae
                && let Some((Lvalue::Net(net), l)) = self.lvalue_name(n, &mut Sink::Proc(out))
            {
                bound.push((fd, FormalBinding::Alias(net, l)));
                continue;
            }
            let vname = format!("{frame_name}.{}", ident.name);
            let vname = self.unique(vname);
            let Some(net) = self.new_net(vname, layout.clone(), false, ident.span) else {
                continue;
            };
            if is_in {
                let value = match actual.and_then(Actual::expr) {
                    Some(ae) => self.expr_in(ae, &layout, &mut Sink::Proc(out)),
                    None => match &obj.default {
                        Some(def) => self.expr_in(def, &layout, &mut Sink::Proc(out)),
                        None => self.zero_of(&layout),
                    },
                };
                out.span = span;
                out.assign(Lvalue::Net(net), value, AssignKind::Blocking);
            }
            if is_out
                && let Some(ae) = actual.and_then(Actual::expr)
                && let ast::Expr::Name(n) = ae
                && let Some((lv, l)) = self.lvalue_name(n, &mut Sink::Proc(out))
            {
                copies.push((fd, lv, l));
            }
            bound.push((fd, FormalBinding::Local(net, layout)));
        }

        self.push_scope();
        self.inlining.push(d);
        let mut nets: Vec<(DeclId, NetId)> = Vec::new();
        for (fd, b) in bound {
            match b {
                FormalBinding::Alias(net, layout) => {
                    nets.push((fd, net));
                    self.bind(fd, Binding::Net { net, layout });
                }
                FormalBinding::Local(net, layout) => {
                    nets.push((fd, net));
                    self.bind(fd, Binding::Net { net, layout });
                }
            }
        }

        // The result and the "has returned" flag.
        let mut result = None;
        let mut result_layout = None;
        if let Some(ret) = ret_type(self, d) {
            let layout = self
                .layout_of_type_quiet(ret, span)
                .or_else(|| want.cloned());
            match layout {
                Some(layout) => {
                    let rname = self.unique(format!("{frame_name}.result"));
                    if let Some(net) = self.new_net(rname, layout.clone(), false, span) {
                        result = Some(net);
                        result_layout = Some(layout);
                    }
                }
                None => {
                    self.layout_of_type(ret, span);
                }
            }
        }
        let needs_flag =
            def.statements.iter().any(may_return) && !ends_with_return(&def.statements);
        let flag = if needs_flag {
            let fname = self.unique(format!("{frame_name}.returned"));
            let layout = self.bit_layout();
            let net = self.new_net(fname, layout, false, span);
            if let Some(net) = net {
                self.b.span = span;
                let zero = self.b.const_bit(false);
                out.span = span;
                out.assign(Lvalue::Net(net), zero, AssignKind::Blocking);
            }
            net
        } else {
            None
        };

        // The body's own declarations live under the frame's name, so a
        // local variable never shadows a signal of the architecture.
        let saved_prefix = std::mem::replace(&mut self.prefix, format!("{frame_name}."));
        self.in_subprogram += 1;
        let saved_inits = std::mem::take(&mut self.pending_inits);
        self.declarations(&def.decls);
        for (net, value, ispan) in std::mem::take(&mut self.pending_inits) {
            out.span = ispan;
            out.assign(Lvalue::Net(net), value, AssignKind::Blocking);
        }
        self.pending_inits = saved_inits;
        self.in_subprogram -= 1;
        self.frames.push(Frame {
            result,
            result_layout: result_layout.clone(),
            flag,
        });
        self.stmts_guarded(&def.statements, out, flag);
        self.frames.pop();
        self.prefix = saved_prefix;

        // Copy `out` and `inout` formals back.
        for (fd, lv, l) in copies {
            let Some(Binding::Net { net, layout }) = self.lookup(fd).cloned() else {
                continue;
            };
            self.b.span = span;
            let value = self.b.net(net);
            let value = self.coerce(value, &layout, &l, span);
            out.span = span;
            out.assign(lv.clone(), value, AssignKind::Blocking);
            self.note_target(&lv, DriverSite::Fixed, span);
        }
        self.inlining.pop();
        self.pop_scope();
        let _ = nets;

        match (result, result_layout) {
            (Some(net), Some(layout)) => {
                self.b.span = span;
                Some((self.b.net(net), layout))
            }
            _ => None,
        }
    }
}

/// How a formal parameter is represented while a call is inlined.
enum FormalBinding {
    /// Aliased to the caller's signal.
    Alias(NetId, Layout),
    /// A local variable net.
    Local(NetId, Layout),
}

/// The return type of a subprogram declaration.
fn ret_type(low: &Lowerer<'_, '_>, d: DeclId) -> Option<crate::vhdl::sema::TypeId> {
    match &low.a().decl(d).kind {
        DeclKind::Subprogram { sig, .. } => sig.ret,
        _ => None,
    }
}

/// Matches an actual to the formal at `index`, by name or by position.
fn pick_actual<'a>(
    low: &Lowerer<'a, '_>,
    args: &[Actual<'a>],
    index: usize,
    ident: &ast::Ident,
) -> Option<Actual<'a>> {
    for a in args {
        if let Some(f) = a.formal()
            && let ast::Expr::Name(ast::Name::Simple(n)) = f
            && n.name.eq_ignore_ascii_case(&ident.name)
        {
            return Some(*a);
        }
    }
    let _ = low;
    let positional: Vec<&Actual<'a>> = args.iter().filter(|a| a.formal().is_none()).collect();
    positional.get(index).copied().copied()
}

/// A subprogram name reduced to what a bare IR name allows.
fn sanitise_call(name: &str) -> String {
    let trimmed = name.trim_matches('"');
    let s = super::sanitise(trimmed);
    if s.is_empty() { "call".to_owned() } else { s }
}

/// True when a statement can execute a `return`.
fn may_return(s: &ast::SequentialStatement) -> bool {
    match &s.kind {
        ast::SequentialKind::Return(_) => true,
        ast::SequentialKind::If(i) => {
            i.arms.iter().any(|a| a.statements.iter().any(may_return))
                || i.else_statements
                    .as_ref()
                    .is_some_and(|e| e.iter().any(may_return))
        }
        ast::SequentialKind::Case(c) => c.arms.iter().any(|a| a.statements.iter().any(may_return)),
        ast::SequentialKind::Loop(l) => l.statements.iter().any(may_return),
        _ => false,
    }
}

/// True when the last statement is an unconditional `return`.
fn ends_with_return(stmts: &[ast::SequentialStatement]) -> bool {
    stmts
        .last()
        .is_some_and(|s| matches!(s.kind, ast::SequentialKind::Return(_)))
        && stmts.len() == stmts.iter().filter(|s| !may_return(s)).count() + 1
}

/// The largest whole unit a femtosecond count fits.
pub(crate) fn delay_from_fs(fs: u64) -> Delay {
    for unit in TimeUnit::ALL.into_iter().rev() {
        let scale = unit.in_fs();
        if fs.is_multiple_of(scale) && fs / scale > 0 {
            return Delay::new(fs / scale, unit);
        }
    }
    Delay::new(fs, TimeUnit::Fs)
}
