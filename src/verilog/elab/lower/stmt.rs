//! Lowering procedural statements.
//!
//! Statements map one to one onto [`crate::ir::StmtKind`], with three exceptions:
//!
//! - a timing control (`#5`, `@(posedge clk)`, `wait (x)`) becomes an
//!   [`crate::ir::WaitKind`] statement followed by the statement it guarded;
//! - a function or task call is inlined: the callee's formals and locals
//!   become process-local variables of the enclosing module, its body is
//!   spliced in, and `output` arguments are copied back afterwards. A
//!   `return` that is not the last statement sets a flag and the rest of
//!   the body runs under `if (!flag)`, which keeps the control flow exact
//!   without needing a jump in the IR;
//! - a compound assignment (`x += y`) and `++` / `--` expand to the plain
//!   assignment they stand for.
//!
//! Declarations inside a block (`begin int i; ... end`) create variables
//! named after the block, so two blocks may declare the same name.

use crate::ir::builder::BlockBuilder;
use crate::ir::{
    self, AssignKind, CaseArm, CaseKind, CaseQualifier, Edge, ExprId, NetId, Polarity,
    ReportSeverity, StmtKind, WaitKind,
};
use crate::source::Span;
use crate::verilog::ast::{self, AssignOp, Expr, ExprKind, ForInit, Stmt, StmtKind as AstStmt};

use super::super::codes;
use super::super::decls;
use super::super::scope::Symbol;
use super::super::types::{self, Packed};
use super::super::width::{self, Info};
use super::{Ctx, Lowerer, Sink};

/// True when `body` contains a `return` that is not its final statement,
/// so the inlined body needs a "has returned" flag.
pub(super) fn has_early_return(body: &[Stmt]) -> bool {
    let mut seen = false;
    for (i, s) in body.iter().enumerate() {
        let last = i + 1 == body.len();
        if contains_return(s) && (!last || !is_plain_return(s)) {
            seen = true;
        }
    }
    seen
}

/// True when the statement is exactly `return ...;`.
fn is_plain_return(s: &Stmt) -> bool {
    matches!(s.kind, AstStmt::Return(_))
}

/// True when `s` contains a `return` anywhere.
fn contains_return(s: &Stmt) -> bool {
    match &s.kind {
        AstStmt::Return(_) => true,
        AstStmt::Block(b) | AstStmt::Fork(b, _) => b.stmts.iter().any(contains_return),
        AstStmt::If(i) => {
            contains_return(&i.then_stmt)
                || i.else_stmt.as_ref().is_some_and(|e| contains_return(e))
        }
        AstStmt::Case(c) => c.items.iter().any(|i| contains_return(&i.body)),
        AstStmt::For(f) => contains_return(&f.body),
        AstStmt::While(_, b)
        | AstStmt::DoWhile(b, _)
        | AstStmt::Repeat(_, b)
        | AstStmt::Forever(b)
        | AstStmt::Timing(_, b)
        | AstStmt::Wait(_, b) => contains_return(b),
        AstStmt::Foreach(f) => contains_return(&f.body),
        _ => false,
    }
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// Lowers a statement list into `out`.
    pub(super) fn stmts(&mut self, stmts: &'ast [Stmt], out: &mut BlockBuilder) {
        for s in stmts {
            self.stmt(s, out);
        }
    }

    /// Lowers a statement list, guarding what follows an early `return`
    /// with `if (!flag)`.
    pub(super) fn stmts_guarded(
        &mut self,
        stmts: &'ast [Stmt],
        out: &mut BlockBuilder,
        flag: Option<NetId>,
    ) {
        let Some(flag) = flag else {
            self.stmts(stmts, out);
            return;
        };
        for (i, s) in stmts.iter().enumerate() {
            self.stmt(s, out);
            let rest = &stmts[i + 1..];
            if contains_return(s) && !rest.is_empty() {
                let mut inner = self.b.block();
                inner.span = rest[0].span;
                self.stmts_guarded(rest, &mut inner, Some(flag));
                self.b.span = rest[0].span;
                let f = self.b.net(flag);
                let cond = self.b.lnot(f);
                out.span = rest[0].span;
                out.if_(cond, inner.finish(), Vec::new());
                return;
            }
        }
    }

    /// Lowers one statement.
    pub(super) fn stmt(&mut self, s: &'ast Stmt, out: &mut BlockBuilder) {
        out.span = s.span;
        self.b.span = s.span;
        match &s.kind {
            AstStmt::Null => {}
            AstStmt::Block(b) => self.block_stmt(b, out),
            AstStmt::Assign(a) => self.assign_stmt(a, s.span, out),
            AstStmt::Expr(e) => self.expr_stmt(e, s.span, out),
            AstStmt::If(i) => {
                let cond = self.expr_bit(&i.cond, &mut Sink::Proc(out));
                let mut then_b = self.b.block();
                then_b.span = i.then_stmt.span;
                self.stmt(&i.then_stmt, &mut then_b);
                let mut else_b = self.b.block();
                if let Some(e) = &i.else_stmt {
                    else_b.span = e.span;
                    self.stmt(e, &mut else_b);
                }
                out.span = s.span;
                out.if_(cond, then_b.finish(), else_b.finish());
            }
            AstStmt::Case(c) => self.case_stmt(c, s.span, out),
            AstStmt::For(f) => self.for_stmt(f, s.span, out),
            AstStmt::While(cond, body) => {
                let c = self.expr_bit(cond, &mut Sink::Proc(out));
                let body = self.loop_body(body);
                out.span = s.span;
                out.while_(c, body);
            }
            AstStmt::DoWhile(body, cond) => {
                // `do body while (c)` runs the body once, then loops.
                self.stmt(body, out);
                let c = self.expr_bit(cond, &mut Sink::Proc(out));
                let body = self.loop_body(body);
                out.span = s.span;
                out.while_(c, body);
            }
            AstStmt::Repeat(count, body) => {
                let info = self
                    .env
                    .probe(|env| width::info(count, env, &[]))
                    .unwrap_or(Info::bits(32, true));
                let n = self.expr(
                    count,
                    Some(Ctx {
                        width: info.width().max(1),
                        signed: info.is_signed(),
                    }),
                    &mut Sink::Proc(out),
                );
                let body = self.loop_body(body);
                out.span = s.span;
                out.repeat(n, body);
            }
            AstStmt::Forever(body) => {
                let body = self.loop_body(body);
                out.span = s.span;
                out.forever(body);
            }
            AstStmt::Break => out.break_(),
            AstStmt::Continue => out.continue_(),
            AstStmt::Return(value) => self.return_stmt(value.as_ref(), s.span, out),
            AstStmt::Timing(ctrl, body) => {
                self.timing(ctrl, out);
                self.stmt(body, out);
            }
            AstStmt::Wait(cond, body) => {
                let c = self.expr_bit(cond, &mut Sink::Proc(out));
                out.span = s.span;
                out.push(StmtKind::Wait(WaitKind::Until(c)));
                self.stmt(body, out);
            }
            AstStmt::Assert(a) => self.assertion(a, s.span, out),
            AstStmt::Decl(item) => {
                self.block_decl(item);
                self.block_decl_init(item, out);
            }
            AstStmt::Foreach(_) => self.env.unsupported(s.span, "`foreach`"),
            AstStmt::Fork(..) => self
                .env
                .unsupported(s.span, "`fork` / `join` (no parallel processes in the IR)"),
            AstStmt::Disable(_) => self.env.unsupported(s.span, "`disable`"),
            AstStmt::DisableFork => self.env.unsupported(s.span, "`disable fork`"),
            AstStmt::WaitFork => self.env.unsupported(s.span, "`wait fork`"),
            AstStmt::ProcAssign(..) => self
                .env
                .unsupported(s.span, "a procedural continuous `assign`"),
            AstStmt::Deassign(_) => self.env.unsupported(s.span, "`deassign`"),
            AstStmt::Force(..) => self.env.unsupported(s.span, "`force`"),
            AstStmt::Release(_) => self.env.unsupported(s.span, "`release`"),
            AstStmt::Trigger { .. } => self.env.unsupported(s.span, "an event trigger `->`"),
        }
    }

    /// The body of a loop, with the inlined-function return check appended
    /// when one is active.
    fn loop_body(&mut self, body: &'ast Stmt) -> ir::Block {
        let mut b = self.b.block();
        b.span = body.span;
        self.stmt(body, &mut b);
        if let Some((_, Some(flag))) = self.inline_frames.last().copied()
            && contains_return(body)
        {
            self.b.span = body.span;
            let f = self.b.net(flag);
            let mut inner = self.b.block();
            inner.span = body.span;
            inner.break_();
            b.span = body.span;
            b.if_(f, inner.finish(), Vec::new());
        }
        b.finish()
    }

    /// `begin ... end`, with its own scope when it is labelled.
    fn block_stmt(&mut self, b: &'ast ast::Block, out: &mut BlockBuilder) {
        match &b.label {
            None => self.stmts(&b.stmts, out),
            Some(label) => {
                let scope = self.env.enter_new(format!("{}.", label.name));
                let _ = scope;
                let mut inner = self.b.block();
                inner.span = b.span;
                self.stmts(&b.stmts, &mut inner);
                self.env.leave();
                out.span = b.span;
                out.nested(Some(&label.name), inner.finish());
            }
        }
    }

    /// A declaration inside a block.
    fn block_decl(&mut self, item: &'ast ast::Item) {
        match &item.kind {
            ast::ItemKind::Var(vd) => {
                for d in &vd.decls {
                    let Some(ty) = types::resolve(&mut self.env, &vd.data_type, &d.dims, true)
                    else {
                        continue;
                    };
                    let ir_name = self.env.qualified(&d.name.name);
                    let Some(net) = self.new_var(ir_name, ty.clone(), d.name.span) else {
                        continue;
                    };
                    self.env
                        .declare(&d.name.name, Symbol::Net { net, ty }, d.name.span);
                }
            }
            ast::ItemKind::Param(pd) => {
                let mut none = decls::Overrides::new();
                decls::param(&mut self.env, pd, &mut none);
            }
            ast::ItemKind::Typedef(td) => decls::typedef(&mut self.env, td),
            ast::ItemKind::Net(_) => self
                .env
                .unsupported(item.span, "a net declaration inside a block"),
            ast::ItemKind::Port(_) | ast::ItemKind::Empty => {}
            _ => self
                .env
                .unsupported(item.span, "this declaration inside a block"),
        }
    }

    /// Initialises the variables a block declaration gives a value.
    pub(super) fn block_decl_init(&mut self, item: &'ast ast::Item, out: &mut BlockBuilder) {
        let ast::ItemKind::Var(vd) = &item.kind else {
            return;
        };
        for d in &vd.decls {
            let Some(init) = &d.init else { continue };
            let Some((Symbol::Net { net, ty }, _, _)) = self.env.lookup(&d.name.name) else {
                continue;
            };
            let (net, ty) = (*net, ty.clone());
            let width = ty.packed().map_or(1, Packed::width);
            let signed = ty.packed().is_some_and(|p| p.signed);
            let value = self.expr(init, Some(Ctx { width, signed }), &mut Sink::Proc(out));
            out.span = d.span;
            out.assign(net, value, AssignKind::Blocking);
            self.record_driver(net, self.driver, true, d.span);
        }
    }

    /// `lhs = rhs;` and friends.
    fn assign_stmt(&mut self, a: &'ast ast::Assign, span: Span, out: &mut BlockBuilder) {
        let kind = match a.op {
            AssignOp::NonBlocking => AssignKind::NonBlocking,
            _ => AssignKind::Blocking,
        };
        // An intra-assignment timing control: `x <= #2 y` keeps the delay,
        // `x = @(posedge clk) y` waits first.
        let mut delay = None;
        if let Some(t) = &a.timing {
            match &t.kind {
                ast::TimingKind::Delay(d) => {
                    delay = d.values.first().and_then(|e| self.delay_from_expr(e));
                }
                _ => {
                    self.timing(t, out);
                }
            }
        }
        if a.op == AssignOp::Blocking || a.op == AssignOp::NonBlocking {
            self.do_assign(&a.lhs, &a.rhs, kind, delay, span, out);
        } else {
            self.assign_expr(&a.lhs, a.op, Some(&a.rhs), span, out);
        }
    }

    /// Emits an assignment expression into whichever sink is active.
    pub(super) fn assign_expr_sink(
        &mut self,
        lhs: &'ast Expr,
        op: AssignOp,
        rhs: Option<&'ast Expr>,
        span: Span,
        sink: &mut Sink<'_>,
    ) {
        match sink {
            Sink::Proc(block) => {
                let mut b = std::mem::replace(*block, BlockBuilder::new(span));
                self.assign_expr(lhs, op, rhs, span, &mut b);
                **block = b;
            }
            Sink::Cont => self
                .env
                .unsupported(span, "an assignment expression outside a procedural block"),
        }
    }

    /// True when an expression's own type is signed, which decides how it
    /// is extended into an assignment's context (§5.5.1).
    pub(super) fn value_is_signed(&mut self, e: &'ast Expr) -> bool {
        self.env
            .probe(|env| width::info(e, env, &[]))
            .is_some_and(|i| i.is_signed())
    }

    /// Emits `target <op>= value`, or `target = value` for a plain op.
    pub(super) fn assign_expr(
        &mut self,
        lhs: &'ast Expr,
        op: AssignOp,
        rhs: Option<&'ast Expr>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some(target) = self.lvalue(lhs, true) else {
            return;
        };
        let width = self.lvalue_width(&target);
        let signed = self
            .env
            .probe(|env| env.type_of_path(lhs))
            .and_then(|t| t.packed().map(|p| p.signed))
            .unwrap_or(false);
        let ctx = Ctx { width, signed };
        let current = self.expr(lhs, Some(ctx), &mut Sink::Proc(out));
        let value = match rhs {
            Some(r) => {
                let info = self
                    .env
                    .probe(|env| width::info(r, env, &[]))
                    .unwrap_or(Info::bits(width, signed));
                let rw = match op {
                    AssignOp::Shl | AssignOp::Shr | AssignOp::Ashl | AssignOp::Ashr => Ctx {
                        width: info.width().max(1),
                        signed: false,
                    },
                    _ => ctx,
                };
                self.expr(r, Some(rw), &mut Sink::Proc(out))
            }
            None => self.b.const_u64(width, 1),
        };
        self.b.span = span;
        let irop = match op {
            AssignOp::Add => ir::BinaryOp::Add,
            AssignOp::Sub => ir::BinaryOp::Sub,
            AssignOp::Mul => ir::BinaryOp::Mul,
            AssignOp::Div => ir::BinaryOp::Div,
            AssignOp::Mod => ir::BinaryOp::Mod,
            AssignOp::And => ir::BinaryOp::And,
            AssignOp::Or => ir::BinaryOp::Or,
            AssignOp::Xor => ir::BinaryOp::Xor,
            AssignOp::Shl | AssignOp::Ashl => ir::BinaryOp::Shl,
            AssignOp::Shr => ir::BinaryOp::Shr,
            AssignOp::Ashr => ir::BinaryOp::Sshr,
            AssignOp::Blocking | AssignOp::NonBlocking => {
                out.span = span;
                out.assign(target.clone(), value, AssignKind::Blocking);
                self.note_assign_drivers(&target, self.driver, span, true);
                return;
            }
        };
        let combined = self.b.binary(irop, current, value);
        let fitted = self.coerce(combined, width, signed);
        out.span = span;
        out.assign(target.clone(), fitted, AssignKind::Blocking);
        self.note_assign_drivers(&target, self.driver, span, true);
    }

    /// Emits a plain blocking or non-blocking assignment.
    fn do_assign(
        &mut self,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        kind: AssignKind,
        delay: Option<ir::Delay>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some(target) = self.lvalue(lhs, true) else {
            return;
        };
        let width = self.lvalue_width(&target);
        let signed = self
            .env
            .probe(|env| env.type_of_path(lhs))
            .and_then(|t| t.packed().map(|p| p.signed))
            .unwrap_or(false);
        if let Some(info) = self.env.probe(|env| width::info(rhs, env, &[])) {
            let name = self.lvalue_name(&target);
            width::check_assign(&mut self.env, &name, width, &info, span);
        }
        let value = self.expr(rhs, Some(Ctx { width, signed }), &mut Sink::Proc(out));
        out.span = span;
        match delay {
            Some(d) => out.assign_after(target.clone(), value, kind, d),
            None => out.assign(target.clone(), value, kind),
        }
        self.note_assign_drivers(&target, self.driver, span, true);
    }

    /// An expression used as a statement: a call, `++`, or an assignment.
    fn expr_stmt(&mut self, e: &'ast Expr, span: Span, out: &mut BlockBuilder) {
        match &e.kind {
            ExprKind::Assign { lhs, op, rhs } => self.assign_expr(lhs, *op, Some(rhs), span, out),
            ExprKind::IncDec {
                increment, target, ..
            } => {
                let op = if *increment {
                    AssignOp::Add
                } else {
                    AssignOp::Sub
                };
                self.assign_expr(target, op, None, span, out);
            }
            ExprKind::Call { callee, args } => self.call_stmt(callee, args, span, out),
            ExprKind::Ident(_) | ExprKind::Member { .. } | ExprKind::Scoped { .. } => {
                // A task called without parentheses.
                self.call_stmt(e, &[], span, out);
            }
            ExprKind::Cast { expr, .. } => {
                // `void'(f(x))`: the call still happens.
                self.expr_stmt(expr, span, out);
            }
            _ => {
                let _ = self.expr(e, None, &mut Sink::Proc(out));
            }
        }
    }

    /// A task or system task call statement.
    fn call_stmt(
        &mut self,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        if let ExprKind::SystemIdent(id) = &callee.kind {
            self.system_task(&id.name, args, span, out);
            return;
        }
        let Some((f, _)) = self.env.resolve_callee(callee) else {
            return;
        };
        let mut b = std::mem::replace(out, BlockBuilder::new(span));
        let _ = self.inline_into(f, args, span, &mut b);
        *out = b;
    }

    /// A `$display`-family or control system task.
    fn system_task(
        &mut self,
        name: &str,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        out.span = span;
        match name {
            "finish" => out.finish_sim(),
            "stop" => out.stop(),
            "fatal" => {
                let msg = self.syscall_args(args, out);
                self.b.span = span;
                let cond = self.b.const_bit(false);
                out.span = span;
                out.assert(cond, ReportSeverity::Failure, msg);
            }
            "error" | "warning" | "info" => {
                let msg = self.syscall_args(args, out);
                self.b.span = span;
                let cond = self.b.const_bit(false);
                let severity = match name {
                    "error" => ReportSeverity::Error,
                    "warning" => ReportSeverity::Warning,
                    _ => ReportSeverity::Note,
                };
                out.span = span;
                out.assert(cond, severity, msg);
            }
            "readmemh" | "readmemb" => {
                // The IR has no memory-valued expression, so the memory is
                // named by a string argument.
                let mut ids = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    let Some(v) = &a.value else { continue };
                    if i == 1
                        && let Some(Symbol::Memory { .. }) =
                            self.env.probe(|env| env.resolve_path(v))
                        && let Some(path) = super::path_components(v)
                    {
                        self.b.span = v.span;
                        let name = self.env.qualified(&path.join("."));
                        let id = self.b.string(name);
                        ids.push(id);
                        continue;
                    }
                    let id = self.expr(v, None, &mut Sink::Proc(out));
                    ids.push(id);
                }
                out.span = span;
                out.syscall(format!("${name}"), ids);
            }
            _ => {
                let ids = self.syscall_args(args, out);
                out.span = span;
                out.syscall(format!("${name}"), ids);
            }
        }
    }

    /// Lowers the arguments of a system task.
    ///
    /// An argument that names a scope, an instance or an array rather than
    /// a value (`$dumpvars(0, tb)`, `$readmemh(f, mem)`) becomes a string
    /// holding its name, since the IR has no expression for those.
    fn syscall_args(&mut self, args: &'ast [ast::Arg], out: &mut BlockBuilder) -> Vec<ExprId> {
        let mut ids = Vec::new();
        for a in args {
            let Some(v) = &a.value else { continue };
            if let Some(name) = self.scope_argument(v) {
                self.b.span = v.span;
                let id = self.b.string(name);
                ids.push(id);
                continue;
            }
            let id = self.expr(v, None, &mut Sink::Proc(out));
            ids.push(id);
        }
        ids
    }

    /// The name an argument denotes when it is a scope, an instance or an
    /// array rather than a value.
    fn scope_argument(&mut self, e: &'ast Expr) -> Option<String> {
        let sym = self.env.probe(|env| env.resolve_path(e))?;
        let qualified = match sym {
            Symbol::Memory { mem, .. } => {
                return Some(self.b.module().memories[mem].name.to_string());
            }
            Symbol::Instance { .. } | Symbol::GenBlock(_) | Symbol::Iface { .. } => true,
            _ => false,
        };
        if !qualified {
            return None;
        }
        super::path_components(e).map(|p| p.join("."))
    }

    /// `return [value];` inside an inlined subroutine.
    fn return_stmt(&mut self, value: Option<&'ast Expr>, span: Span, out: &mut BlockBuilder) {
        let Some((result, flag)) = self.inline_frames.last().copied() else {
            self.env
                .error(codes::TYPE, span, "`return` outside a function or task");
            return;
        };
        if let (Some(value), Some(result)) = (value, result) {
            let width = self.net_width(result);
            let signed = self.net_type(result).packed().is_some_and(|p| p.signed);
            let v = self.expr(value, Some(Ctx { width, signed }), &mut Sink::Proc(out));
            out.span = span;
            out.assign(result, v, AssignKind::Blocking);
            self.record_driver(result, self.driver, true, span);
        }
        if let Some(flag) = flag {
            self.b.span = span;
            let one = self.b.const_bit(true);
            out.span = span;
            out.assign(flag, one, AssignKind::Blocking);
        }
    }

    /// `case`, `casez`, `casex`.
    fn case_stmt(&mut self, c: &'ast ast::Case, span: Span, out: &mut BlockBuilder) {
        if c.inside {
            self.env
                .unsupported(span, "`case ... inside` (use `case` or `if`)");
            return;
        }
        // Every item is sized with the subject (§12.5).
        let mut info = self
            .env
            .probe(|env| width::info(&c.expr, env, &[]))
            .unwrap_or(Info::bits(1, false));
        for item in &c.items {
            for p in &item.patterns {
                if let Some(i) = self.env.probe(|env| width::info(p, env, &[])) {
                    info = info.combine(&i);
                }
            }
        }
        let ctx = Ctx {
            width: info.width().max(1),
            signed: info.is_signed(),
        };
        let subject = self.expr(&c.expr, Some(ctx), &mut Sink::Proc(out));
        let mut arms = Vec::new();
        let mut default = None;
        for item in &c.items {
            let mut body = self.b.block();
            body.span = item.span;
            self.stmt(&item.body, &mut body);
            let body = body.finish();
            if item.patterns.is_empty() {
                if default.is_none() {
                    default = Some(body);
                }
                continue;
            }
            let mut values = Vec::with_capacity(item.patterns.len());
            for p in &item.patterns {
                values.push(self.expr(p, Some(ctx), &mut Sink::Proc(out)));
            }
            arms.push(CaseArm { values, body });
        }
        let kind = match c.kind {
            ast::CaseKind::Case => CaseKind::Plain,
            ast::CaseKind::Casez => CaseKind::Z,
            ast::CaseKind::Casex => CaseKind::X,
        };
        let qualifier = match c.qualifier {
            Some(ast::Qualifier::Unique | ast::Qualifier::Unique0) => CaseQualifier::Unique,
            Some(ast::Qualifier::Priority) => CaseQualifier::Priority,
            None => CaseQualifier::None,
        };
        out.span = span;
        out.case(subject, kind, qualifier, arms, default);
    }

    /// `for (init; cond; step) body`.
    fn for_stmt(&mut self, f: &'ast ast::For, span: Span, out: &mut BlockBuilder) {
        // Loop variables declared in the header live in a scope of their
        // own so two loops may use the same name.
        let loop_name = self.fresh("loop");
        self.env.enter_new(format!("{loop_name}."));
        let mut init: Option<(ir::Lvalue, ExprId)> = None;
        let mut extra = Vec::new();
        for i in &f.init {
            match i {
                ForInit::Decl(vd) => {
                    for d in &vd.decls {
                        let Some(ty) = types::resolve(&mut self.env, &vd.data_type, &d.dims, true)
                        else {
                            continue;
                        };
                        let ir_name = self.env.qualified(&d.name.name);
                        let Some(net) = self.new_var(ir_name, ty.clone(), d.name.span) else {
                            continue;
                        };
                        self.env.declare(
                            &d.name.name,
                            Symbol::Net {
                                net,
                                ty: ty.clone(),
                            },
                            d.name.span,
                        );
                        let Some(value) = &d.init else { continue };
                        let width = ty.packed().map_or(1, Packed::width);
                        let signed = ty.packed().is_some_and(|p| p.signed);
                        let v = self.expr(value, Some(Ctx { width, signed }), &mut Sink::Proc(out));
                        self.record_driver(net, self.driver, true, d.span);
                        let entry = (ir::Lvalue::Net(net), v);
                        if init.is_none() {
                            init = Some(entry);
                        } else {
                            extra.push(entry);
                        }
                    }
                }
                ForInit::Assign(e) => {
                    if let ExprKind::Assign { lhs, op, rhs } = &e.kind
                        && *op == AssignOp::Blocking
                        && let Some(target) = self.lvalue(lhs, true)
                    {
                        let width = self.lvalue_width(&target);
                        let signed = self
                            .env
                            .probe(|env| env.type_of_path(lhs))
                            .and_then(|t| t.packed().map(|p| p.signed))
                            .unwrap_or(false);
                        let v = self.expr(rhs, Some(Ctx { width, signed }), &mut Sink::Proc(out));
                        self.note_assign_drivers(&target, self.driver, e.span, true);
                        if init.is_none() {
                            init = Some((target, v));
                        } else {
                            extra.push((target, v));
                        }
                    } else {
                        self.expr_stmt(e, e.span, out);
                    }
                }
            }
        }
        // Extra initialisations run before the loop.
        for (target, v) in extra {
            out.span = span;
            out.assign(target, v, AssignKind::Blocking);
        }
        let cond = f
            .cond
            .as_ref()
            .map(|c| self.expr_bit(c, &mut Sink::Proc(out)));
        // The step: the IR holds one assignment, so further ones are
        // appended to the body.
        let mut step: Option<(ir::Lvalue, ExprId)> = None;
        let mut body = self.b.block();
        body.span = f.body.span;
        self.stmt(&f.body, &mut body);
        for st in &f.step {
            let pair = match &st.kind {
                ExprKind::Assign { lhs, op, rhs } if *op == AssignOp::Blocking => {
                    let signed = self.value_is_signed(rhs);
                    self.lvalue(lhs, true).map(|target| {
                        let width = self.lvalue_width(&target);
                        let v =
                            self.expr(rhs, Some(Ctx { width, signed }), &mut Sink::Proc(&mut body));
                        (target, v)
                    })
                }
                ExprKind::IncDec {
                    increment, target, ..
                } => {
                    let signed = self.value_is_signed(target);
                    self.lvalue(target, true).map(|lv| {
                        let width = self.lvalue_width(&lv);
                        let cur = self.expr(
                            target,
                            Some(Ctx { width, signed }),
                            &mut Sink::Proc(&mut body),
                        );
                        let one = if signed {
                            self.b.const_i64(width, 1)
                        } else {
                            self.b.const_u64(width, 1)
                        };
                        let op = if *increment {
                            ir::BinaryOp::Add
                        } else {
                            ir::BinaryOp::Sub
                        };
                        let v = self.b.binary(op, cur, one);
                        (lv, v)
                    })
                }
                _ => {
                    self.expr_stmt(st, st.span, &mut body);
                    None
                }
            };
            if let Some((target, v)) = pair {
                self.note_assign_drivers(&target, self.driver, st.span, true);
                if step.is_none() {
                    step = Some((target, v));
                } else {
                    body.span = st.span;
                    body.assign(target, v, AssignKind::Blocking);
                }
            }
        }
        if let Some((_, Some(flag))) = self.inline_frames.last().copied()
            && contains_return(&f.body)
        {
            self.b.span = f.body.span;
            let fnet = self.b.net(flag);
            let mut inner = self.b.block();
            inner.break_();
            body.if_(fnet, inner.finish(), Vec::new());
        }
        self.env.leave();
        out.span = span;
        out.for_(init, cond, step, body.finish());
    }

    /// A delay or event control.
    fn timing(&mut self, ctrl: &'ast ast::TimingControl, out: &mut BlockBuilder) {
        out.span = ctrl.span;
        match &ctrl.kind {
            ast::TimingKind::Delay(d) => {
                let Some(e) = d.values.first() else { return };
                let id = match self.delay_from_expr(e) {
                    Some(delay) => {
                        self.b.span = ctrl.span;
                        self.b.const_u64(64, delay.value)
                    }
                    None => self.expr(e, None, &mut Sink::Proc(out)),
                };
                out.push(StmtKind::Wait(WaitKind::Delay(id)));
            }
            ast::TimingKind::Event(ev) => {
                let edges = self.event_edges(ev);
                out.push(StmtKind::Wait(WaitKind::Event(edges)));
            }
            ast::TimingKind::RepeatEvent(count, ev) => {
                let n = self.expr(count, None, &mut Sink::Proc(out));
                let edges = self.event_edges(ev);
                let mut inner = self.b.block();
                inner.span = ctrl.span;
                inner.push(StmtKind::Wait(WaitKind::Event(edges)));
                out.repeat(n, inner.finish());
            }
        }
    }

    /// The edges of an event control.
    pub(super) fn event_edges(&mut self, ev: &'ast ast::EventControl) -> Vec<Edge> {
        let mut edges = Vec::new();
        match &ev.kind {
            ast::EventControlKind::Any => {}
            ast::EventControlKind::List(list) => {
                for item in list {
                    if item.iff.is_some() {
                        self.env
                            .unsupported(item.span, "an `iff` condition on an event");
                    }
                    let polarity = match item.edge {
                        Some(ast::Edge::Posedge) => Polarity::Pos,
                        Some(ast::Edge::Negedge) => Polarity::Neg,
                        _ => Polarity::Any,
                    };
                    for net in self.event_nets(&item.expr) {
                        edges.push(Edge { net, polarity });
                    }
                }
            }
        }
        edges
    }

    /// The nets an event expression watches.
    fn event_nets(&mut self, e: &'ast Expr) -> Vec<NetId> {
        match self.env.probe(|env| env.resolve_path(e)) {
            Some(Symbol::Net { net, .. }) => vec![net],
            _ => match &e.kind {
                ExprKind::Index { base, .. } | ExprKind::Range { base, .. } => {
                    // An edge on a bit of a vector watches the vector.
                    self.event_nets(base)
                }
                ExprKind::Concat(parts) => parts.iter().flat_map(|p| self.event_nets(p)).collect(),
                _ => {
                    self.env.error(
                        codes::UNSUPPORTED,
                        e.span,
                        "an event expression must name a signal",
                    );
                    Vec::new()
                }
            },
        }
    }

    /// An immediate assertion.
    fn assertion(&mut self, a: &'ast ast::Assertion, span: Span, out: &mut BlockBuilder) {
        let ast::AssertSpec::Expr(cond) = &a.spec else {
            self.env
                .unsupported(span, "a concurrent assertion with a property");
            return;
        };
        if a.kind != ast::AssertKind::Assert {
            self.env
                .unsupported(span, &format!("`{}`", a.kind.as_str()));
            return;
        }
        let c = self.expr_bit(cond, &mut Sink::Proc(out));
        let mut message = Vec::new();
        let mut severity = ReportSeverity::Error;
        if let Some(else_stmt) = &a.else_stmt
            && let AstStmt::Expr(e) = &else_stmt.kind
            && let ExprKind::Call { callee, args } = &e.kind
            && let ExprKind::SystemIdent(id) = &callee.kind
        {
            severity = match id.name.as_str() {
                "fatal" => ReportSeverity::Failure,
                "warning" => ReportSeverity::Warning,
                "info" | "display" => ReportSeverity::Note,
                _ => ReportSeverity::Error,
            };
            message = self.syscall_args(args, out);
        }
        out.span = span;
        out.assert(c, severity, message);
        if let Some(then_stmt) = &a.then_stmt {
            self.stmt(then_stmt, out);
        }
    }
}
