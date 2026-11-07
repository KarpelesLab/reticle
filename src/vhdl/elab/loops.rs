//! Lowering loops, `next` and `exit`.
//!
//! A VHDL loop becomes an IR loop: `loop` a `Forever`, `while` a `While`
//! and `for` a `For` whose parameter is a variable net. The IR's `break`
//! and `continue` leave the innermost loop only, which is all an
//! unlabelled `exit` or `next` asks for.
//!
//! # A jump to an outer loop
//!
//! `exit outer` and `next outer` from inside an inner loop set a flag of
//! the loop they name (`<label>$exit`, `<label>$next`) and break the
//! innermost IR loop. After each loop statement, the loop around it tests
//! the flags of every loop a jump inside it named: its own, which it acts
//! on (clearing `$next` and continuing, or breaking), and those of loops
//! further out, for which it breaks in turn. A flag is cleared when the
//! loop it belongs to starts, so a second run of that loop starts clean.
//!
//! # Unrolled loops
//!
//! A `for` loop whose parameter appears in the bounds of a slice, in an
//! aggregate's choice or in the range of a loop inside it — the byte-write
//! loop `mem(a)(8*i+7 downto 8*i) <= d(8*i+7 downto 8*i)` is the case that
//! matters — is unrolled: its body is lowered once per value of the
//! parameter, with the parameter bound to that value, so the slice is
//! static in each copy. That is exactly what synthesis does with any
//! `for` loop, and what LRM 10.10 allows since a loop parameter is a
//! constant inside its loop. A loop over `bit`, `boolean` or `std_ulogic`
//! is unrolled too, since a counter cannot hold those types' encodings.
//! The iterations follow each other in one block; when a `next` or an
//! `exit` leaves one, each iteration is wrapped in a `forever … break`
//! that the `next` breaks, and an `exit` also sets the loop's `$exit`
//! flag, which ends the run of iterations. More than
//! [`UNROLL_LIMIT`] iterations is `V0713` rather than an unbounded copy.
//!
//! # A range known only at run time
//!
//! `for k2 in k to 3` inside a loop over `k` that is not unrolled, or over
//! a variable, evaluates its bounds when the loop starts (LRM 10.10): the
//! left bound initialises the parameter and the right one is copied to a
//! `$stop` variable the condition reads, so assigning the variables the
//! bounds were computed from inside the loop does not change it.

use crate::ir::builder::BlockBuilder;
use crate::ir::{Lvalue, NetId, NetKind};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast;
use crate::vhdl::sema::{Analysis, DeclId, Value};

use super::codes;
use super::lower::{Binding, Lowerer, Sink};
use super::types::{Layout, LayoutKind};

/// The most iterations a loop is unrolled into.
pub(crate) const UNROLL_LIMIT: i64 = 4096;

/// One loop being lowered.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoopFrame {
    /// The loop's label.
    pub label: Option<String>,
    /// True when the loop is unrolled rather than an IR loop.
    pub unrolled: bool,
    /// Set by an `exit` of this loop that has to leave an IR loop first.
    pub exit_flag: Option<NetId>,
    /// Set by a `next` of this loop that has to leave an IR loop first.
    pub next_flag: Option<NetId>,
    /// Loops further out whose flags a jump inside this loop's innermost
    /// IR loop has set, by depth.
    pub escapes: Vec<usize>,
    /// For an unrolled loop: each iteration must be a loop of its own, so
    /// that a `next` or an escape can break out of it.
    pub wrap: bool,
}

impl<'a> Lowerer<'a, '_> {
    /// Lowers a loop statement.
    pub(crate) fn loop_stmt(
        &mut self,
        label: Option<&'a ast::Ident>,
        l: &'a ast::LoopStatement,
        out: &mut BlockBuilder,
    ) {
        let name = label.map(|i| i.name.clone());
        match &l.scheme {
            None => {
                self.loops.push(LoopFrame {
                    label: name,
                    ..LoopFrame::default()
                });
                let mut body = self.b.block();
                body.span = l.span;
                self.stmts(&l.statements, &mut body);
                let frame = self.loops.pop().unwrap_or_default();
                self.clear_flags(&frame, l.span, out);
                out.span = l.span;
                out.forever(body.finish());
                self.after_loop(&frame, l.span, out);
            }
            Some(ast::IterationScheme::While(c)) => {
                self.loops.push(LoopFrame {
                    label: name,
                    ..LoopFrame::default()
                });
                let mut body = self.b.block();
                body.span = l.span;
                self.stmts(&l.statements, &mut body);
                let frame = self.loops.pop().unwrap_or_default();
                self.clear_flags(&frame, l.span, out);
                let cond = self.cond(c, &mut Sink::Proc(out));
                out.span = l.span;
                out.while_(cond, body.finish());
                self.after_loop(&frame, l.span, out);
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
        let decl = self.cx.object_decl_at(param.span);
        let dir = self.range_direction(range);
        // The parameter's type is the range's: recorded for the range
        // itself when it is written `l to r`, and for its bounds when they
        // are attributes (`color_t'high downto color_t'low`).
        let range_ty = self.a().range_of(range.span()).map(|r| r.ty);
        let range_layout = match range_ty {
            Some(t) => self.layout_of_type_quiet(t, range.span()),
            None => self.layout_at(range.span()),
        };
        let bit_kind = range_layout
            .as_ref()
            .is_some_and(|l| matches!(l.kind, LayoutKind::Bit(_)));
        let mentions = decl.is_some_and(|d| stmts_mention(self.a(), d, body));
        if let Some((left, right)) = self.static_range(range)
            && let Some(d) = decl
            && (mentions || bit_kind)
        {
            let ty = range_layout.map_or(self.a().builtins.integer, |l| l.ty);
            self.unrolled_for(label, d, ty, (left, right, dir), body, span, out);
            return;
        }
        // The counter: 32 bits, with the kind of the range so an
        // enumeration parameter is still one (`color_t'image(k)`).
        let layout = Layout {
            width: 32,
            signed: true,
            ..range_layout
                .filter(|l| matches!(l.kind, LayoutKind::Int | LayoutKind::Enum(_)))
                .unwrap_or_else(|| self.int_layout())
        };
        let name = self.qualified(&param.name);
        let Some(var) = self.new_net(name, layout.clone(), false, span) else {
            return;
        };
        self.b.module_mut().nets[var].kind = NetKind::Variable;
        let int = self.int_layout();
        let (start, stop) = match self.static_range(range) {
            Some((left, right)) => {
                self.b.span = span;
                let start = self.b.constant(Logic::from_i64(left, 32).with_signed(true));
                let stop = self
                    .b
                    .constant(Logic::from_i64(right, 32).with_signed(true));
                (start, stop)
            }
            None => {
                let ast::DiscreteRange::Range(ast::Range::Bounds { left, right, .. }) = range
                else {
                    self.error(
                        codes::NOT_STATIC,
                        range.span(),
                        "a loop range must be static after elaboration",
                    );
                    return;
                };
                let start = self.expr_in(left, &int, &mut Sink::Proc(out));
                let stop_value = self.expr_in(right, &int, &mut Sink::Proc(out));
                let stop_name = self.fresh(&format!("{}$stop", param.name));
                let Some(stop_net) = self.new_net(stop_name, int.clone(), false, span) else {
                    return;
                };
                self.b.module_mut().nets[stop_net].kind = NetKind::Variable;
                out.span = span;
                out.assign(
                    Lvalue::Net(stop_net),
                    stop_value,
                    crate::ir::AssignKind::Blocking,
                );
                self.b.span = span;
                (start, self.b.net(stop_net))
            }
        };
        self.push_scope();
        if let Some(d) = decl {
            self.bind(
                d,
                Binding::Net {
                    net: var,
                    layout: layout.clone(),
                },
            );
        }
        self.b.span = span;
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
        self.loops.push(LoopFrame {
            label,
            ..LoopFrame::default()
        });
        let mut inner = self.b.block();
        inner.span = span;
        self.stmts(body, &mut inner);
        let frame = self.loops.pop().unwrap_or_default();
        self.pop_scope();
        self.clear_flags(&frame, span, out);
        out.span = span;
        out.for_(
            Some((Lvalue::Net(var), start)),
            Some(cond),
            Some((Lvalue::Net(var), next)),
            inner.finish(),
        );
        self.after_loop(&frame, span, out);
    }

    /// A `for` loop lowered once per value of its parameter.
    #[allow(
        clippy::too_many_arguments,
        reason = "the loop's parts, already taken apart"
    )]
    fn unrolled_for(
        &mut self,
        label: Option<String>,
        param: DeclId,
        ty: crate::vhdl::sema::TypeId,
        (left, right, dir): (i64, i64, ast::Direction),
        body: &'a [ast::SequentialStatement],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let values: Vec<i64> = match dir {
            ast::Direction::To if left <= right => (left..=right).collect(),
            ast::Direction::Downto if left >= right => (right..=left).rev().collect(),
            _ => Vec::new(),
        };
        if i64::try_from(values.len()).unwrap_or(i64::MAX) > UNROLL_LIMIT {
            self.error(
                codes::LIMIT,
                span,
                format!(
                    "this loop has to be unrolled, since a slice moves with its parameter, \
                     and it has more than {UNROLL_LIMIT} iterations"
                ),
            );
            return;
        }
        let enumeration = !matches!(
            self.a().class(ty),
            crate::vhdl::sema::TypeClass::Integer | crate::vhdl::sema::TypeClass::UniversalInteger
        );
        self.loops.push(LoopFrame {
            label,
            unrolled: true,
            ..LoopFrame::default()
        });
        let mut iterations = Vec::with_capacity(values.len());
        for v in values {
            self.push_scope();
            let value = if enumeration {
                Value::Enum(u32::try_from(v).unwrap_or(0))
            } else {
                Value::Int(i128::from(v))
            };
            self.bind(param, Binding::Value { value, ty });
            let mut block = self.b.block();
            block.span = span;
            self.stmts(body, &mut block);
            self.pop_scope();
            iterations.push(block.finish());
        }
        let frame = self.loops.pop().unwrap_or_default();
        self.clear_flags(&frame, span, out);
        if !frame.wrap {
            for it in iterations {
                out.span = span;
                out.nested(None, it);
            }
            self.after_loop(&frame, span, out);
            return;
        }
        let mut run = self.b.block();
        run.span = span;
        for it in iterations {
            let mut once = self.b.block();
            once.span = span;
            once.nested(None, it);
            once.break_();
            run.span = span;
            run.forever(once.finish());
            // Leave the run of iterations when this loop was exited, or
            // when a loop further out was named.
            let mut flags: Vec<NetId> = frame.exit_flag.into_iter().collect();
            for &t in &frame.escapes {
                flags.extend(self.frame_flags(t));
            }
            for f in flags {
                self.b.span = span;
                let c = self.b.net(f);
                let mut leave = self.b.block();
                leave.span = span;
                leave.break_();
                run.span = span;
                run.if_(c, leave.finish(), Vec::new());
            }
        }
        run.break_();
        out.span = span;
        out.forever(run.finish());
        self.after_loop(&frame, span, out);
    }

    /// `next` or `exit`, optionally labelled and conditional.
    pub(crate) fn jump(
        &mut self,
        label: Option<&'a ast::Ident>,
        condition: Option<&'a ast::Expr>,
        exit: bool,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let word = if exit { "exit" } else { "next" };
        let Some(depth) = self.loops.len().checked_sub(1) else {
            self.error(codes::UNSUPPORTED, span, format!("`{word}` outside a loop"));
            return;
        };
        let target = match label {
            None => depth,
            Some(l) => match self.loops.iter().rposition(|f| {
                f.label
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&l.name))
            }) {
                Some(t) => t,
                None => {
                    self.error(
                        codes::UNSUPPORTED,
                        l.span,
                        format!("`{word}` names no enclosing loop `{}`", l.name),
                    );
                    return;
                }
            },
        };
        let mut body = self.b.block();
        body.span = span;
        if target == depth {
            if exit {
                self.break_frame(depth, span, &mut body);
            } else {
                self.continue_frame(depth, &mut body);
            }
        } else {
            let flag = self.frame_flag(target, exit, span);
            if let Some(flag) = flag {
                self.b.span = span;
                let one = self.b.const_bit(true);
                body.assign(Lvalue::Net(flag), one, crate::ir::AssignKind::Blocking);
            }
            if !self.loops[depth].escapes.contains(&target) {
                self.loops[depth].escapes.push(target);
            }
            self.break_frame(depth, span, &mut body);
        }
        let body = body.finish();
        match condition {
            Some(c) => {
                let cond = self.cond(c, &mut Sink::Proc(out));
                out.span = span;
                out.if_(cond, body, Vec::new());
            }
            None => {
                for s in body {
                    out.push(s.kind);
                }
            }
        }
    }

    /// Leaves the loop at `depth` from its innermost IR loop.
    fn break_frame(&mut self, depth: usize, span: Span, out: &mut BlockBuilder) {
        if self.loops[depth].unrolled {
            self.loops[depth].wrap = true;
            if let Some(flag) = self.frame_flag(depth, true, span) {
                self.b.span = span;
                let one = self.b.const_bit(true);
                out.assign(Lvalue::Net(flag), one, crate::ir::AssignKind::Blocking);
            }
        }
        out.break_();
    }

    /// Starts the next iteration of the loop at `depth`, from its innermost
    /// IR loop.
    fn continue_frame(&mut self, depth: usize, out: &mut BlockBuilder) {
        if self.loops[depth].unrolled {
            // The iteration is a `forever … break` of its own.
            self.loops[depth].wrap = true;
            out.break_();
        } else {
            out.continue_();
        }
    }

    /// The `$exit` (or `$next`) flag of the loop at `depth`, created on
    /// first use.
    fn frame_flag(&mut self, depth: usize, exit: bool, span: Span) -> Option<NetId> {
        let existing = if exit {
            self.loops[depth].exit_flag
        } else {
            self.loops[depth].next_flag
        };
        if existing.is_some() {
            return existing;
        }
        let base = self.loops[depth]
            .label
            .clone()
            .unwrap_or_else(|| "loop".to_owned());
        let name = self.fresh(&format!("{base}${}", if exit { "exit" } else { "next" }));
        let layout = self.boolean_layout();
        let net = self.new_net(name, layout, false, span)?;
        self.b.module_mut().nets[net].kind = NetKind::Variable;
        if exit {
            self.loops[depth].exit_flag = Some(net);
        } else {
            self.loops[depth].next_flag = Some(net);
        }
        Some(net)
    }

    /// Every flag of the loop at `depth`.
    fn frame_flags(&self, depth: usize) -> Vec<NetId> {
        let f = &self.loops[depth];
        f.exit_flag.into_iter().chain(f.next_flag).collect()
    }

    /// Clears a loop's flags before it starts.
    fn clear_flags(&mut self, frame: &LoopFrame, span: Span, out: &mut BlockBuilder) {
        for f in frame.exit_flag.into_iter().chain(frame.next_flag) {
            self.b.span = span;
            let zero = self.b.const_bit(false);
            out.span = span;
            out.assign(Lvalue::Net(f), zero, crate::ir::AssignKind::Blocking);
        }
    }

    /// After a loop statement: act on the flags a jump inside it set for
    /// the loop around it, and pass the others on outwards.
    fn after_loop(&mut self, frame: &LoopFrame, span: Span, out: &mut BlockBuilder) {
        let Some(parent) = self.loops.len().checked_sub(1) else {
            return;
        };
        for &t in &frame.escapes {
            if t == parent {
                if let Some(next) = self.loops[parent].next_flag {
                    self.b.span = span;
                    let c = self.b.net(next);
                    let zero = self.b.const_bit(false);
                    let mut then_ = self.b.block();
                    then_.span = span;
                    then_.assign(Lvalue::Net(next), zero, crate::ir::AssignKind::Blocking);
                    self.continue_frame(parent, &mut then_);
                    out.span = span;
                    out.if_(c, then_.finish(), Vec::new());
                }
                if let Some(exit) = self.loops[parent].exit_flag {
                    self.b.span = span;
                    let c = self.b.net(exit);
                    let mut then_ = self.b.block();
                    then_.span = span;
                    self.break_frame(parent, span, &mut then_);
                    out.span = span;
                    out.if_(c, then_.finish(), Vec::new());
                }
            } else {
                for f in self.frame_flags(t) {
                    self.b.span = span;
                    let c = self.b.net(f);
                    let mut then_ = self.b.block();
                    then_.span = span;
                    self.break_frame(parent, span, &mut then_);
                    out.span = span;
                    out.if_(c, then_.finish(), Vec::new());
                }
                if !self.loops[parent].escapes.contains(&t) {
                    self.loops[parent].escapes.push(t);
                }
            }
        }
    }
}

// --- does a loop body need its parameter to be static? ---------------------

/// True when `d` appears in a slice's bounds, an aggregate's choice or a
/// loop's range somewhere in `stmts`.
pub(crate) fn stmts_mention(a: &Analysis, d: DeclId, stmts: &[ast::SequentialStatement]) -> bool {
    stmts.iter().any(|s| stmt_mentions(a, d, s))
}

fn stmt_mentions(a: &Analysis, d: DeclId, s: &ast::SequentialStatement) -> bool {
    let e = |x: &ast::Expr| expr_mentions(a, d, x, false);
    let oe = |x: &Option<ast::Expr>| x.as_ref().is_some_and(e);
    match &s.kind {
        ast::SequentialKind::Wait {
            condition, timeout, ..
        } => oe(condition) || oe(timeout),
        ast::SequentialKind::Assertion(x) => e(&x.condition) || oe(&x.report) || oe(&x.severity),
        ast::SequentialKind::Report { message, severity } => e(message) || oe(severity),
        ast::SequentialKind::SignalAssignment(sa) => {
            target_mentions(a, d, &sa.target)
                || match &sa.rhs {
                    ast::SignalAssignmentRhs::Simple(w) => waveform_mentions(a, d, w),
                    ast::SignalAssignmentRhs::Conditional(arms) => arms
                        .iter()
                        .any(|c| waveform_mentions(a, d, &c.waveform) || oe(&c.condition)),
                    ast::SignalAssignmentRhs::Selected { selector, arms, .. } => {
                        e(selector)
                            || arms.iter().any(|c| {
                                waveform_mentions(a, d, &c.waveform)
                                    || choices_mention(a, d, &c.choices)
                            })
                    }
                    ast::SignalAssignmentRhs::Force { arms, .. } => {
                        arms.iter().any(|c| e(&c.value) || oe(&c.condition))
                    }
                    ast::SignalAssignmentRhs::Release { .. } => false,
                }
        }
        ast::SequentialKind::VariableAssignment(va) => {
            target_mentions(a, d, &va.target)
                || match &va.rhs {
                    ast::VariableAssignmentRhs::Simple(x) => e(x),
                    ast::VariableAssignmentRhs::Conditional(arms) => {
                        arms.iter().any(|c| e(&c.value) || oe(&c.condition))
                    }
                    ast::VariableAssignmentRhs::Selected { selector, arms, .. } => {
                        e(selector)
                            || arms
                                .iter()
                                .any(|c| e(&c.value) || choices_mention(a, d, &c.choices))
                    }
                }
        }
        ast::SequentialKind::ProcedureCall(n) => name_mentions(a, d, n, false),
        ast::SequentialKind::If(i) => {
            i.arms
                .iter()
                .any(|arm| e(&arm.condition) || stmts_mention(a, d, &arm.statements))
                || i.else_statements
                    .as_ref()
                    .is_some_and(|s| stmts_mention(a, d, s))
        }
        ast::SequentialKind::Case(c) => {
            e(&c.expr)
                || c.arms.iter().any(|arm| {
                    choices_mention(a, d, &arm.choices) || stmts_mention(a, d, &arm.statements)
                })
        }
        ast::SequentialKind::Loop(l) => {
            let scheme = match &l.scheme {
                None => false,
                Some(ast::IterationScheme::While(c)) => e(c),
                Some(ast::IterationScheme::For { range, .. }) => range_mentions(a, d, range, true),
            };
            scheme || stmts_mention(a, d, &l.statements)
        }
        ast::SequentialKind::Next { condition, .. }
        | ast::SequentialKind::Exit { condition, .. } => oe(condition),
        ast::SequentialKind::Return(x) => oe(x),
        ast::SequentialKind::Null => false,
    }
}

fn target_mentions(a: &Analysis, d: DeclId, t: &ast::Target) -> bool {
    match t {
        ast::Target::Name(n) => name_mentions(a, d, n, false),
        ast::Target::Aggregate(ag) => aggregate_mentions(a, d, ag, false),
    }
}

fn waveform_mentions(a: &Analysis, d: DeclId, w: &ast::Waveform) -> bool {
    match w {
        ast::Waveform::Elements(els) => els.iter().any(|el| {
            expr_mentions(a, d, &el.value, false)
                || el
                    .after
                    .as_ref()
                    .is_some_and(|x| expr_mentions(a, d, x, false))
        }),
        ast::Waveform::Unaffected(_) => false,
    }
}

fn choices_mention(a: &Analysis, d: DeclId, choices: &[ast::Choice]) -> bool {
    choices.iter().any(|c| match c {
        ast::Choice::Expr(x) => expr_mentions(a, d, x, true),
        ast::Choice::Range(r) => range_mentions(a, d, r, true),
        ast::Choice::Others(_) => false,
    })
}

fn aggregate_mentions(a: &Analysis, d: DeclId, ag: &ast::Aggregate, in_range: bool) -> bool {
    ag.elements
        .iter()
        .any(|el| choices_mention(a, d, &el.choices) || expr_mentions(a, d, &el.value, in_range))
}

fn range_mentions(a: &Analysis, d: DeclId, r: &ast::DiscreteRange, in_range: bool) -> bool {
    match r {
        ast::DiscreteRange::Range(ast::Range::Bounds { left, right, .. }) => {
            expr_mentions(a, d, left, in_range) || expr_mentions(a, d, right, in_range)
        }
        ast::DiscreteRange::Range(ast::Range::Attribute(n)) => name_mentions(a, d, n, in_range),
        ast::DiscreteRange::Subtype(_) => false,
    }
}

/// True when `d` is named inside `e` at a place that has to be static
/// (`in_range`), or inside a slice range or a choice below it.
fn expr_mentions(a: &Analysis, d: DeclId, e: &ast::Expr, in_range: bool) -> bool {
    match e {
        ast::Expr::Binary { lhs, rhs, .. } => {
            expr_mentions(a, d, lhs, in_range) || expr_mentions(a, d, rhs, in_range)
        }
        ast::Expr::Unary { operand, .. } => expr_mentions(a, d, operand, in_range),
        ast::Expr::Name(n) => name_mentions(a, d, n, in_range),
        ast::Expr::Aggregate(ag) => aggregate_mentions(a, d, ag, in_range),
        ast::Expr::Qualified { operand, .. } => expr_mentions(a, d, operand, in_range),
        ast::Expr::Paren { inner, .. } => expr_mentions(a, d, inner, in_range),
        ast::Expr::Literal(_)
        | ast::Expr::Allocator { .. }
        | ast::Expr::Open(_)
        | ast::Expr::Error(_) => false,
    }
}

fn name_mentions(a: &Analysis, d: DeclId, n: &ast::Name, in_range: bool) -> bool {
    match n {
        ast::Name::Simple(id) => in_range && a.decl_of(id.span) == Some(d),
        ast::Name::Operator { .. } | ast::Name::Char { .. } | ast::Name::External(_) => false,
        ast::Name::Selected { prefix, .. } => name_mentions(a, d, prefix, in_range),
        ast::Name::Call { prefix, args, .. } => {
            name_mentions(a, d, prefix, in_range)
                || args.iter().any(|arg| match &arg.actual {
                    ast::Actual::Expr(x) | ast::Actual::Inertial(x) => {
                        expr_mentions(a, d, x, in_range)
                    }
                    ast::Actual::Range(r) => range_mentions(a, d, r, true),
                    ast::Actual::Open(_) => false,
                })
        }
        ast::Name::Slice { prefix, range, .. } => {
            name_mentions(a, d, prefix, in_range) || range_mentions(a, d, range, true)
        }
        ast::Name::Attribute { prefix, .. } => name_mentions(a, d, prefix, in_range),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::rtl;

    /// Would catch every `for` loop being unrolled, which would change the
    /// IR of every existing design with a loop; would not catch a loop
    /// that should be unrolled and is not, which the simulation fixture
    /// `loop_slices_tb.vhd` catches.
    #[test]
    fn a_loop_indexing_bits_stays_a_loop() {
        let out = rtl(
            "entity e is port (d : in bit_vector(3 downto 0); q : out bit); end entity;\n\
             architecture a of e is begin\n\
               process (d) variable x : bit; begin\n\
                 x := '0';\n\
                 for i in 0 to 3 loop x := x xor d(i); end loop;\n\
                 q <= x;\n\
               end process;\n\
             end architecture;",
        );
        assert!(out.contains("for %i = "), "{out}");
    }

    /// Would catch a loop whose parameter bounds a slice being lowered as a
    /// loop (its slice then has no static bounds), or its copies losing
    /// their order; would not catch a wrong value in a copy, which the
    /// simulation fixture does.
    #[test]
    fn a_loop_whose_parameter_bounds_a_slice_is_unrolled() {
        let out = rtl("entity e is port (d : in bit_vector(15 downto 0);\n\
                               q : out bit_vector(15 downto 0)); end entity;\n\
             architecture a of e is begin\n\
               process (d) begin\n\
                 for i in 0 to 1 loop\n\
                   q(8 * i + 7 downto 8 * i) <= d(15 - 8 * i downto 8 - 8 * i);\n\
                 end loop;\n\
               end process;\n\
             end architecture;");
        assert!(!out.contains("for %i"), "{out}");
        let first = out.find("%q[7:0] <= %d[15:8]");
        let second = out.find("%q[15:8] <= %d[7:0]");
        assert!(first.is_some() && second.is_some(), "{out}");
        assert!(first < second, "{out}");
    }
}
