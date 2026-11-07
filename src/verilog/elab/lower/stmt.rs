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

use std::rc::Rc;

use crate::ir::builder::BlockBuilder;
use crate::ir::{
    self, AssignKind, CaseArm, CaseKind, CaseQualifier, Edge, ExprId, NetId, Polarity,
    ReportSeverity, StmtKind, WaitKind,
};
use crate::source::Span;
use crate::verilog::ast::{self, AssignOp, Expr, ExprKind, ForInit, Stmt, StmtKind as AstStmt};

use super::super::codes;
use super::super::constant::Evaluator;
use super::super::decls;
use super::super::scope::Symbol;
use super::super::types::{self, Enumeration, Packed, Range, VType, bits_needed};
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

/// The IR qualifier of a `case` or `if` and its default branch: `unique0`
/// is `unique` that may match nothing, so it gets an empty default when it
/// has none, which keeps a match of none silent and leaves a match of two
/// a warning.
fn qualify(
    q: Option<ast::Qualifier>,
    default: Option<ir::Block>,
    b: &ir::builder::ModuleBuilder,
    span: Span,
) -> (CaseQualifier, Option<ir::Block>) {
    match q {
        Some(ast::Qualifier::Unique) => (CaseQualifier::Unique, default),
        Some(ast::Qualifier::Unique0) => {
            let default = default.or_else(|| {
                let mut empty = b.block();
                empty.span = span;
                Some(empty.finish())
            });
            (CaseQualifier::Unique, default)
        }
        Some(ast::Qualifier::Priority) => (CaseQualifier::Priority, default),
        None => (CaseQualifier::None, default),
    }
}

/// What `%p` prints of an argument the simulator cannot spell from its
/// bits alone.
#[derive(Clone, Debug)]
enum PShape {
    /// An enum, printed by the name of its value.
    Enum(Rc<Enumeration>),
    /// A packed struct, printed member by member.
    Struct(Vec<PField>),
    /// An unpacked array held as a memory, printed element by element
    /// from its left bound.
    Array {
        /// The memory.
        mem: ir::MemoryId,
        /// Its declared dimension.
        range: Range,
        /// The shape of an element that is not a plain value.
        elem: Option<Box<PShape>>,
    },
}

/// One member of a struct that `%p` prints.
#[derive(Clone, Debug)]
struct PField {
    name: String,
    lsb: u32,
    width: u32,
    signed: bool,
    shape: Option<PShape>,
}

/// The `%p` shape of a type, when it has one.
fn p_shape_of(ty: &VType) -> Option<PShape> {
    let p = ty.packed()?;
    if let Some(e) = &p.enumeration {
        return Some(PShape::Enum(Rc::clone(e)));
    }
    let fields = p.fields.as_ref()?;
    Some(PShape::Struct(
        fields
            .iter()
            .map(|f| PField {
                name: f.name.clone(),
                lsb: f.lsb,
                width: f.ty.packed().map_or(1, Packed::width).max(1),
                signed: f.ty.packed().is_some_and(|q| q.signed),
                shape: p_shape_of(&f.ty),
            })
            .collect(),
    ))
}

/// The conversions that print `shape`; `zero` (`%0p`) leaves the member
/// names out, as xsim does.
fn p_text(shape: &PShape, zero: bool) -> String {
    let leaf = |inner: &Option<PShape>| match inner {
        Some(s) => p_text(s, zero),
        None => "%p".to_owned(),
    };
    match shape {
        PShape::Enum(e) => {
            let names: Vec<String> = e
                .variants
                .iter()
                .filter_map(|(n, v)| v.to_u64().map(|v| format!("{n}={v:x}")))
                .collect();
            format!("%{{{}}}p", names.join(","))
        }
        PShape::Struct(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|f| {
                    let text = leaf(&f.shape);
                    if zero {
                        text
                    } else {
                        format!("{}:{text}", f.name)
                    }
                })
                .collect();
            format!("'{{{}}}", parts.join(","))
        }
        PShape::Array { range, elem, .. } => {
            let one = match elem {
                Some(s) => p_text(s, zero),
                None => "%p".to_owned(),
            };
            let n = usize::try_from(range.len()).unwrap_or(0);
            format!("'{{{}}}", vec![one; n].join(","))
        }
    }
}

/// A packed type without its enum or struct, as `$typename` spells it.
fn packed_name(p: &Packed) -> String {
    let kw = p.keyword.unwrap_or("logic");
    match kw {
        // The integer atoms: their width is the keyword's.
        "int" | "byte" | "shortint" | "longint" => {
            if p.signed {
                kw.to_owned()
            } else {
                format!("{kw} unsigned")
            }
        }
        "integer" | "time" => kw.to_owned(),
        _ => {
            let mut s = kw.to_owned();
            if p.signed {
                s.push_str(" signed");
            }
            for d in &p.dims {
                s.push_str(&d.to_string());
            }
            s
        }
    }
}

/// `fmt` with `.suffix` written after each `%m` conversion; `%%` is left
/// alone.
fn with_scope(fmt: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + suffix.len());
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c != '%' {
            continue;
        }
        // The flags, width and precision, then the conversion.
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() || d == '-' || d == '.' {
                out.push(d);
                chars.next();
            } else {
                break;
            }
        }
        if let Some(conv) = chars.next() {
            out.push(conv);
            if conv == 'm' {
                out.push('.');
                out.push_str(suffix);
            }
        }
    }
    out
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
            AstStmt::If(i) if i.qualifier.is_some() => self.qualified_if(i, s.span, out),
            AstStmt::If(i) => {
                // A condition fixed at elaboration — a genvar or a parameter,
                // as in `if (j == 1) count <= 0;` inside a generate loop —
                // keeps only the branch it takes. The other is not code that
                // can run, and lowering it would make every iteration of the
                // loop a driver of what only one of them writes.
                let fixed = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_i64(&i.cond).ok());
                if let Some(value) = fixed {
                    if value != 0 {
                        self.stmt(&i.then_stmt, out);
                    } else if let Some(e) = &i.else_stmt {
                        self.stmt(e, out);
                    }
                    return;
                }
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
            AstStmt::Foreach(f) => self.foreach_stmt(f, s.span, out),
            AstStmt::Fork(b, join) => self.fork_stmt(b, *join, s.span, out),
            AstStmt::Disable(target) => {
                // A block is named by its label; a path to it names the
                // same block (the simulator looks in this instance).
                let name = match &target.kind {
                    ExprKind::Member { name, .. } | ExprKind::Ident(name) => name,
                    _ => {
                        self.env.unsupported(target.span, "`disable` of this");
                        return;
                    }
                };
                out.span = s.span;
                out.disable(name.name.as_str());
            }
            AstStmt::DisableFork => {
                out.span = s.span;
                out.push(StmtKind::DisableFork);
            }
            AstStmt::WaitFork => {
                out.span = s.span;
                out.push(StmtKind::WaitFork);
            }
            // A procedural continuous `assign` holds a variable at a value
            // until `deassign`, which leaves it there until it is next
            // assigned (IEEE 1364-2005 §9.3.1): what `force` and `release`
            // do to a variable, which they are lowered as. A `force` beside
            // it would win where the standard says it should, too.
            AstStmt::ProcAssign(target, value) => self.force(target, value, s.span, out),
            AstStmt::Deassign(target) => {
                if let Some(net) = self.forced_net(target, "deassign") {
                    out.span = s.span;
                    out.push(ir::StmtKind::Release { target: net });
                }
            }
            AstStmt::Force(target, value) => self.force(target, value, s.span, out),
            AstStmt::Release(target) => {
                if let Some(net) = self.forced_net(target, "release") {
                    out.span = s.span;
                    out.push(ir::StmtKind::Release { target: net });
                }
            }
            AstStmt::Trigger {
                nonblocking,
                target,
            } => {
                // `-> e` toggles the bit the event is (`var_decl`).
                let Some(lv) = self.lvalue(target, true) else {
                    return;
                };
                if self.lvalue_width(&lv) != 1 {
                    self.env
                        .error(codes::TYPE, target.span, "`->` triggers a named event");
                    return;
                }
                let current = self.expr(
                    target,
                    Some(Ctx {
                        width: 1,
                        signed: false,
                    }),
                    &mut Sink::Proc(out),
                );
                self.b.span = s.span;
                let flipped = self.b.not(current);
                out.span = s.span;
                let kind = if *nonblocking {
                    AssignKind::NonBlocking
                } else {
                    AssignKind::Blocking
                };
                out.assign(lv.clone(), flipped, kind);
                self.note_assign_drivers(&lv, self.driver, s.span, true);
            }
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
                let parent = self.env.scope;
                let free = self.env.lookup(&label.name).is_none();
                let scope = self.env.enter_new(format!("{}.", label.name));
                // A named block is a scope its variables can be named
                // through, `blk.v` (IEEE 1364-2005 §12.6), as a generate
                // block's are.
                if free {
                    self.env.cx.scopes.redeclare(
                        parent,
                        label.name.clone(),
                        Symbol::GenBlock(scope),
                        label.span,
                    );
                    // A block named in an `initial` or `always` is a scope of
                    // the module (`tb.blk.v`), not of the process.
                    if let Some((_, root)) = self.env.own
                        && root != parent
                        && self.env.cx.scopes.get(root).get(&label.name).is_none()
                    {
                        self.env.cx.scopes.redeclare(
                            root,
                            label.name.clone(),
                            Symbol::GenBlock(scope),
                            label.span,
                        );
                    }
                }
                let mut inner = self.b.block();
                inner.span = b.span;
                self.stmts(&b.stmts, &mut inner);
                self.env.leave();
                out.span = b.span;
                out.nested(Some(&label.name), inner.finish());
            }
        }
    }

    /// `fork ... join`: each statement of the block is a branch, run as a
    /// process of its own; a declaration in it is the parent's, made
    /// before the branches start (IEEE 1800-2017 §9.3.2).
    fn fork_stmt(
        &mut self,
        b: &'ast ast::Block,
        join: ast::JoinKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        if let Some(label) = &b.label {
            let _ = self.env.enter_new(format!("{}.", label.name));
        }
        let mut setup = self.b.block();
        setup.span = b.span;
        let mut branches = Vec::new();
        for st in &b.stmts {
            if let AstStmt::Decl(item) = &st.kind {
                self.block_decl(item);
                self.block_decl_init(item, &mut setup);
                continue;
            }
            let mut inner = self.b.block();
            inner.span = st.span;
            self.stmt(st, &mut inner);
            branches.push(inner.finish());
        }
        let join = match join {
            ast::JoinKind::All => ir::JoinKind::All,
            ast::JoinKind::Any => ir::JoinKind::Any,
            ast::JoinKind::None => ir::JoinKind::None,
        };
        setup.span = span;
        setup.fork(branches, join);
        match &b.label {
            Some(label) => {
                self.env.leave();
                out.span = span;
                out.nested(Some(&label.name), setup.finish());
            }
            None => {
                for stmt in setup.finish() {
                    out.span = stmt.span;
                    out.push(stmt.kind);
                }
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
            let value = self.value_as(init, &ty, &mut Sink::Proc(out));
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
                    // The IR's assignment delay is a constant. One that is
                    // not used to be dropped without a word, so that
                    // `q <= #(d) v` assigned at once.
                    if delay.is_none() && !d.values.is_empty() {
                        self.env.unsupported(
                            t.span,
                            "an intra-assignment delay that is not a constant expression",
                        );
                        return;
                    }
                }
                _ if a.op == AssignOp::Blocking || a.op == AssignOp::NonBlocking => {
                    self.event_assign(a, t, kind, span, out);
                    return;
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

    /// `x = @(e) v` and `x <= @(e) v` (and `repeat (n) @(e)`): the value is
    /// taken now and assigned after the event (IEEE 1364-2005 §9.7.7). The
    /// blocking form waits where it is; the non-blocking one goes on at
    /// once, its assignment made by a process of its own that waits for the
    /// event. Both used to wait first and take the value after, and the
    /// non-blocking one held up the process that made it.
    fn event_assign(
        &mut self,
        a: &'ast ast::Assign,
        t: &'ast ast::TimingControl,
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let Some(target) = self.lvalue(&a.lhs, true) else {
            return;
        };
        let width = self.lvalue_width(&target);
        let ty = self
            .value_type(&a.lhs)
            .filter(|t| !matches!(t, VType::Unpacked { .. }))
            .unwrap_or_else(|| VType::Packed(Packed::bits(width)));
        let value = self.value_as(&a.rhs, &ty, &mut Sink::Proc(out));
        let name = self.fresh("intra");
        let Some(held) = self.new_var(name, ty, span) else {
            return;
        };
        out.span = span;
        out.assign(held, value, AssignKind::Blocking);
        self.record_driver(held, self.driver, true, span);
        let mut later = self.b.block();
        later.span = span;
        self.timing(t, &mut later);
        self.b.span = span;
        let v = self.b.net(held);
        later.span = span;
        later.assign(target.clone(), v, kind);
        self.note_assign_drivers(&target, self.driver, span, true);
        let later = later.finish();
        out.span = span;
        if kind == AssignKind::NonBlocking {
            out.fork(vec![later], ir::JoinKind::None);
        } else {
            for stmt in later {
                out.span = stmt.span;
                out.push(stmt.kind);
            }
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

    /// `force net = value;`, which the simulator keeps following the value
    /// until a `release` (see [`ir::StmtKind::Force`]).
    fn force(&mut self, target: &'ast Expr, value: &'ast Expr, span: Span, out: &mut BlockBuilder) {
        let Some(net) = self.forced_net(target, "force") else {
            return;
        };
        let v = if matches!(self.net_type(net), crate::verilog::elab::types::VType::Real) {
            self.real(value, &mut Sink::Proc(out))
        } else {
            let width = self.net_width(net);
            let signed = self.net_type(net).packed().is_some_and(|p| p.signed);
            self.expr(value, Some(Ctx { width, signed }), &mut Sink::Proc(out))
        };
        out.span = span;
        out.push(ir::StmtKind::Force {
            target: net,
            value: v,
        });
    }

    /// The whole net a `force` or `release` names: a net or variable of
    /// this module, or one below it through a hierarchical name. Part of a
    /// net is not supported.
    fn forced_net(&mut self, target: &'ast Expr, what: &str) -> Option<NetId> {
        // Path resolution sees through `w[0]` to `w`, so `force w[0] = 0`
        // used to force the whole of `w`.
        if matches!(target.kind, ExprKind::Index { .. } | ExprKind::Range { .. }) {
            self.env.unsupported(
                target.span,
                &format!("`{what}` of part of a net or variable"),
            );
            return None;
        }
        match self.env.probe(|env| env.resolve_path(target)) {
            Some(Symbol::Net { net, .. }) => Some(net),
            _ => {
                self.env.unsupported(
                    target.span,
                    &format!("`{what}` of anything but a whole net or variable"),
                );
                None
            }
        }
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
        if self.target_is_real(&target) {
            // `r += x` on a real: real arithmetic, a real result.
            let current = self.real(lhs, &mut Sink::Proc(out));
            let value = match rhs {
                Some(r) => self.real(r, &mut Sink::Proc(out)),
                None => self.real_one(),
            };
            self.b.span = span;
            let irop = match op {
                AssignOp::Add => Some(ir::BinaryOp::Add),
                AssignOp::Sub => Some(ir::BinaryOp::Sub),
                AssignOp::Mul => Some(ir::BinaryOp::Mul),
                AssignOp::Div => Some(ir::BinaryOp::Div),
                AssignOp::Mod => Some(ir::BinaryOp::Mod),
                _ => None,
            };
            let result = match irop {
                Some(irop) => self.b.binary(irop, current, value),
                None => value,
            };
            out.span = span;
            out.assign(target.clone(), result, AssignKind::Blocking);
            self.note_assign_drivers(&target, self.driver, span, true);
            return;
        }
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
        if delay.is_none() && self.array_assign(lhs, rhs, kind, span, out) {
            return;
        }
        if let ExprKind::Streaming {
            right_to_left,
            slice,
            elems,
        } = &lhs.kind
        {
            self.unstream(
                *right_to_left,
                slice.as_deref(),
                elems,
                rhs,
                kind,
                span,
                out,
            );
            return;
        }
        let Some(target) = self.lvalue(lhs, true) else {
            return;
        };
        let width = self.lvalue_width(&target);
        let signed = self
            .env
            .probe(|env| env.type_of_path(lhs))
            .and_then(|t| t.packed().map(|p| p.signed))
            .unwrap_or(false);
        let value = if matches!(rhs.kind, ExprKind::Pattern(_)) {
            let ty = self
                .value_type(lhs)
                .unwrap_or_else(|| VType::Packed(super::super::types::Packed::bits(width)));
            self.value_as(rhs, &ty, &mut Sink::Proc(out))
        } else if self.target_is_real(&target) {
            self.real(rhs, &mut Sink::Proc(out))
        } else if self.target_is_string(&target) {
            self.string_value(rhs, &mut Sink::Proc(out))
        } else {
            if let Some(info) = self.env.probe(|env| width::info(rhs, env, &[])) {
                let name = self.lvalue_name(&target);
                width::check_assign(&mut self.env, &name, width, &info, span);
            }
            self.expr(rhs, Some(Ctx { width, signed }), &mut Sink::Proc(out))
        };
        out.span = span;
        match delay {
            Some(d) => out.assign_after(target.clone(), value, kind, d),
            None => out.assign(target.clone(), value, kind),
        }
        self.note_assign_drivers(&target, self.driver, span, true);
    }

    /// `{<< s {x, y}} = v` or `{>> s {x, y}} = v`: the targets take the
    /// value's leftmost bits, their slices reversed for `<<` (IEEE
    /// 1800-2017 §11.4.14.3).
    #[allow(clippy::too_many_arguments)]
    fn unstream(
        &mut self,
        right_to_left: bool,
        slice: Option<&'ast Expr>,
        elems: &'ast [Expr],
        rhs: &'ast Expr,
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let mut targets = Vec::with_capacity(elems.len());
        for el in elems {
            let Some(t) = self.lvalue(el, true) else {
                return;
            };
            targets.push(t);
        }
        let target = if targets.len() == 1 {
            targets.pop().expect("one target")
        } else {
            ir::Lvalue::Concat(targets)
        };
        let w = self.lvalue_width(&target);
        let info = self
            .env
            .probe(|env| width::info(rhs, env, &[]))
            .unwrap_or(Info::bits(w, false));
        let vw = info.width().max(1);
        let value = self.expr(
            rhs,
            Some(Ctx {
                width: vw,
                signed: false,
            }),
            &mut Sink::Proc(out),
        );
        self.b.span = span;
        // The leftmost `w` bits of the value; a narrower value is a
        // narrower stream, which the standard calls an error and xsim fills
        // with zeros on the right.
        let value = if vw > w {
            self.b.slice(value, vw - 1, vw - w)
        } else if vw < w {
            let pad = self.b.const_u64(w - vw, 0);
            self.b.concat(vec![value, pad])
        } else {
            value
        };
        let value = if right_to_left {
            let Some(size) = self.slice_size(slice) else {
                return;
            };
            self.reverse_slices(value, w, size)
        } else {
            value
        };
        out.span = span;
        out.assign(target.clone(), value, kind);
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
            ExprKind::Ident(_)
            | ExprKind::Member { .. }
            | ExprKind::Scoped { .. }
            | ExprKind::SystemIdent(_) => {
                // A task called without parentheses: `$finish;` is the
                // same call as `$finish();` (IEEE 1364-2005 §17.4.1).
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
        if self.string_mutator(callee, args, span, out).is_some() {
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
            "printtimescale" => {
                // The argument names a scope, which the simulator looks up
                // by its path; it is never a value, and a path into the
                // hierarchy from the top is not resolved here.
                let mut ids = Vec::new();
                for v in args.iter().filter_map(|a| a.value.as_ref()) {
                    let Some(path) = super::path_components(v) else {
                        self.env.error(
                            codes::ARGUMENTS,
                            v.span,
                            "`$printtimescale` takes the name of a module instance",
                        );
                        return;
                    };
                    self.b.span = v.span;
                    ids.push(self.b.string(path.join(".")));
                }
                out.span = span;
                out.syscall(format!("${name}"), ids);
            }
            "readmemh" | "readmemb" | "writememh" | "writememb" => {
                let op = ir::MemFileOp::from_keyword(name).expect("one of the four tasks");
                self.mem_file_task(op, args, span, out);
            }
            _ => {
                let ids = self.syscall_args(args, out);
                out.span = span;
                out.syscall(format!("${name}"), ids);
            }
        }
    }

    /// `$readmemh(file, mem [, start [, end]])` and its three siblings,
    /// as a [`StmtKind::MemFile`] that names the memory itself.
    ///
    /// The start and end addresses are translated into the IR's
    /// zero-based numbering like any index; the memory's declared lowest
    /// index becomes the statement's `base`, which is what `@hex` lines in
    /// the file are measured against.
    fn mem_file_task(
        &mut self,
        op: ir::MemFileOp,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) {
        let task = format!("${}", op.keyword());
        let values: Vec<Option<&'ast Expr>> = args.iter().map(|a| a.value.as_ref()).collect();
        let usage = "a file name, a memory, and optionally a start and an end address";
        let (Some(Some(file)), Some(Some(target))) = (values.first(), values.get(1)) else {
            self.env
                .error(codes::ARGUMENTS, span, format!("`{task}` takes {usage}"));
            return;
        };
        if values.len() > 4 || (values.len() == 4 && values[2].is_none()) {
            self.env
                .error(codes::ARGUMENTS, span, format!("`{task}` takes {usage}"));
            return;
        }
        let mem = match self.env.probe(|env| env.resolve_path(target)) {
            Some(Symbol::Memory { mem, .. }) if !matches!(target.kind, ExprKind::Index { .. }) => {
                mem
            }
            _ => {
                self.env.error(
                    codes::TYPE,
                    target.span,
                    format!("the second argument of `{task}` must be a memory (an unpacked array)"),
                );
                return;
            }
        };
        let file = self.expr(file, None, &mut Sink::Proc(out));
        let range = self.mems[mem.index()].range;
        let size = self.b.module().memories[mem].size;
        let mut bounds = [None, None];
        for (slot, v) in bounds.iter_mut().zip(values.iter().skip(2)) {
            let Some(v) = v else { continue };
            let constant = self.env.probe(|env| Evaluator::new(env).eval_i64(v).ok());
            *slot = Some(match constant {
                // The usual case, folded so the IR shows the element.
                Some(i) => match range.element_offset(i) {
                    Some(offset) => {
                        self.b.span = v.span;
                        self.b
                            .const_u64(bits_needed(size.saturating_sub(1)).max(1), offset)
                    }
                    None => {
                        self.env.error(
                            codes::OUT_OF_RANGE,
                            v.span,
                            format!("address {i} is outside {range}"),
                        );
                        return;
                    }
                },
                None => self.mem_address(mem, v, &mut Sink::Proc(out)),
            });
        }
        let [start, end] = bounds;
        let base = range.low();
        out.span = span;
        out.push(StmtKind::MemFile {
            op,
            mem,
            file,
            start,
            end,
            base,
        });
    }

    /// Lowers the arguments of a system task.
    ///
    /// An argument that names a scope, an instance or a whole array rather
    /// than a value (`$dumpvars(0, tb)`) becomes a string holding its
    /// name, since the IR has no expression for those. An element of an
    /// array (`mem[1]`) is a value like any other.
    fn syscall_args(&mut self, args: &'ast [ast::Arg], out: &mut BlockBuilder) -> Vec<ExprId> {
        let values: Vec<&'ast Expr> = args.iter().filter_map(|a| a.value.as_ref()).collect();
        // Each format literal, with its `%p` of an enum, a struct or an
        // array spelled out, and the shapes of the arguments those print.
        let mut texts: Vec<Option<String>> = vec![None; values.len()];
        let mut shapes: Vec<Option<PShape>> = vec![None; values.len()];
        let mut k = 0;
        while k < values.len() {
            if let ExprKind::Literal(ast::Literal::Str { value, .. }) = &values[k].kind {
                let (text, found, next) = self.p_rewrite(value, &values, k);
                for (arg, shape) in found {
                    shapes[arg] = Some(shape);
                }
                texts[k] = Some(text);
                k = next.max(k + 1);
            } else {
                k += 1;
            }
        }
        let mut ids = Vec::new();
        for (k, v) in values.iter().copied().enumerate() {
            if let Some(text) = texts[k].take() {
                let text = self.scoped(&text);
                self.b.span = v.span;
                ids.push(self.b.string(text));
                continue;
            }
            if let Some(shape) = shapes[k].take() {
                let value = match shape {
                    PShape::Array { .. } => None,
                    _ => Some(self.expr(v, None, &mut Sink::Proc(out))),
                };
                self.b.span = v.span;
                self.p_leaves(value, &shape, &mut ids);
                continue;
            }
            if let Some(name) = self.scope_argument(v) {
                self.b.span = v.span;
                let id = self.b.string(name);
                ids.push(id);
                continue;
            }
            // A real stays a real, so `%f` of `1.5` is 1.5 and `%d` of
            // it rounds as a real does; built as an integer it was 2.
            let real = matches!(
                self.env.probe(|env| width::info(v, env, &[])),
                Some(Info::Real)
            );
            let id = if real {
                self.real(v, &mut Sink::Proc(out))
            } else {
                self.expr(v, None, &mut Sink::Proc(out))
            };
            ids.push(id);
        }
        ids
    }

    /// A string literal argument of a formatting task or function, with
    /// each `%m` extended by the scope it is written in; `None` for any
    /// other argument, or a literal that needs no change.
    ///
    /// `%m` names the scope (IEEE 1364-2005 clause 17), which inside a
    /// named block, a task or a function is below the module instance the
    /// simulator knows: xsim prints `tb.blk`, `tb.where`. The simulator
    /// supplies the instance's path and the literal carries the rest, so
    /// `"%m"` in block `blk` becomes `"%m.blk"`.
    pub(super) fn format_literal(&mut self, v: &'ast Expr) -> Option<ExprId> {
        let ExprKind::Literal(ast::Literal::Str { value, .. }) = &v.kind else {
            return None;
        };
        let text = self.scoped(value);
        if text == *value {
            return None;
        }
        self.b.span = v.span;
        Some(self.b.string(text))
    }

    /// `text` with each `%m` extended by the scope, as
    /// [`Self::format_literal`] does.
    fn scoped(&self, text: &str) -> String {
        if !text.contains("%m") {
            return text.to_owned();
        }
        let suffix = self.scope_suffix();
        if suffix.is_empty() {
            return text.to_owned();
        }
        with_scope(text, &suffix)
    }

    /// The format literal `values[k]` with each `%p` whose argument is an
    /// enum, a struct or an unpacked array spelled out, so that the
    /// simulator, which sees only bits, prints it as xsim does: `'{a:3,b:15}`
    /// for a struct (`'{3,15}` under `%0p`), `GRN` for an enum, `'{1,2,255}`
    /// for an array. A struct or array becomes one conversion per member,
    /// and an enum `%{RED=0,GRN=1}p`, the variants it may name. Returns the
    /// text, the arguments it rewrote with their shapes, and the index of
    /// the first argument it does not consume.
    fn p_rewrite(
        &mut self,
        text: &str,
        values: &[&'ast Expr],
        k: usize,
    ) -> (String, Vec<(usize, PShape)>, usize) {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut found = Vec::new();
        let mut next = k + 1;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            if c != '%' {
                out.push(c);
                continue;
            }
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '-' || chars[i] == '.')
            {
                i += 1;
            }
            let spec: String = chars[start..i].iter().collect();
            let Some(&conv) = chars.get(i) else {
                out.push('%');
                out.push_str(&spec);
                break;
            };
            i += 1;
            if matches!(conv, '%' | 'm' | 'l' | 'L') {
                out.push('%');
                out.push_str(&spec);
                out.push(conv);
                continue;
            }
            let arg = next;
            next += 1;
            let shape = match values.get(arg) {
                Some(v) if conv == 'p' => self.p_shape(v),
                _ => None,
            };
            match shape {
                Some(shape) => {
                    out.push_str(&p_text(&shape, spec == "0"));
                    found.push((arg, shape));
                }
                None => {
                    out.push('%');
                    out.push_str(&spec);
                    out.push(conv);
                }
            }
        }
        (out, found, next)
    }

    /// How `%p` must spell argument `v`, when it is not a plain value.
    fn p_shape(&mut self, v: &'ast Expr) -> Option<PShape> {
        if !matches!(v.kind, ExprKind::Index { .. })
            && let Some(Symbol::Memory { mem, .. }) = self.env.probe(|env| env.resolve_path(v))
        {
            let info = &self.mems[mem.index()];
            return Some(PShape::Array {
                mem,
                range: info.range,
                elem: p_shape_of(&info.elem).map(Box::new),
            });
        }
        // A member's type is its struct's to say.
        if let ExprKind::Member { base, name } = &v.kind
            && let Some(PShape::Struct(fields)) = self.p_shape(base)
        {
            return fields
                .into_iter()
                .find(|f| f.name == name.name)
                .and_then(|f| f.shape);
        }
        p_shape_of(&self.type_of(v))
    }

    /// The values the conversions [`p_text`] wrote for `shape` print, in
    /// order: `value` itself for an enum, its fields for a struct, the
    /// elements for an array (which has no single value).
    fn p_leaves(&mut self, value: Option<ExprId>, shape: &PShape, ids: &mut Vec<ExprId>) {
        match shape {
            PShape::Enum(_) => ids.extend(value),
            PShape::Struct(fields) => {
                let Some(value) = value else { return };
                for f in fields {
                    let bits = self.b.slice(value, f.lsb + f.width - 1, f.lsb);
                    let bits = self.coerce(bits, f.width, f.signed);
                    match &f.shape {
                        Some(inner) => self.p_leaves(Some(bits), inner, ids),
                        None => ids.push(bits),
                    }
                }
            }
            PShape::Array { mem, range, elem } => {
                let size = self.b.module().memories[*mem].size;
                let width = bits_needed(size.saturating_sub(1)).max(1);
                let low = range.low();
                let step: i64 = if range.left <= range.right { 1 } else { -1 };
                let mut i = range.left;
                loop {
                    let addr = self.b.const_u64(width, i.abs_diff(low));
                    let read = self.b.mem_read(*mem, addr);
                    match elem {
                        Some(inner) => self.p_leaves(Some(read), inner, ids),
                        None => ids.push(read),
                    }
                    if i == range.right {
                        break;
                    }
                    i += step;
                }
            }
        }
    }

    /// What `$typename` returns for an expression or a type, spelled as
    /// xsim 2026.1 spells it (IEEE 1800-2017 §20.6.1 leaves the details
    /// to the tool): `int`, `int unsigned`, `logic signed[3:0]`,
    /// `logic[7:0]$[0:3]` for an unpacked array, `enum logic[1:0] tb.c_t`
    /// and `struct tb.s_t` with the typedef's scope, `pkg::t` from a
    /// package. Another typedef is not named: xsim prints its type.
    pub(super) fn type_name_of(&mut self, arg: &'ast Expr) -> String {
        let ty = match &arg.kind {
            ExprKind::Type(dt) => types::resolve(&mut self.env, dt, &[], true),
            _ => Some(self.type_of(arg)),
        };
        let Some(mut ty) = ty else {
            return String::new();
        };
        // A net declared without a data type is a `wire`.
        if let Some(Symbol::Net { net, .. }) = self.env.probe(|env| env.resolve_path(arg))
            && self.nets.get(net.index()).is_some_and(|n| !n.is_var)
            && let VType::Packed(p) = &mut ty
            && p.keyword.is_none()
        {
            p.keyword = Some("wire");
        }
        self.type_name(&ty)
    }

    /// [`Self::type_name_of`] for a resolved type.
    fn type_name(&self, ty: &VType) -> String {
        match ty {
            VType::Packed(p) if p.enumeration.is_some() || p.fields.is_some() => {
                let mut base = p.clone();
                base.enumeration = None;
                let head = if p.enumeration.is_some() {
                    format!("enum {}", packed_name(&base))
                } else {
                    "struct".to_owned()
                };
                match self.typedef_name(p) {
                    Some(name) => format!("{head} {name}"),
                    None => head,
                }
            }
            VType::Packed(p) => packed_name(p),
            VType::Unpacked { .. } => {
                let mut dims = String::new();
                let mut elem = ty;
                while let VType::Unpacked { elem: inner, range } = elem {
                    dims.push_str(&range.to_string());
                    elem = inner;
                }
                format!("{}${dims}", self.type_name(elem))
            }
            other => other.to_string(),
        }
    }

    /// The name of the typedef that declared enum or struct `p`, found by
    /// identity in the scopes visible here and the packages: `tb.c_t` for
    /// one in module `tb`, `pkg::c_t` for one in a package.
    fn typedef_name(&self, p: &Packed) -> Option<String> {
        let same = |q: &Packed| match (&p.enumeration, &q.enumeration, &p.fields, &q.fields) {
            (Some(a), Some(b), _, _) => std::rc::Rc::ptr_eq(a, b),
            (None, None, Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            _ => false,
        };
        let find = |scope| {
            self.env
                .cx
                .scopes
                .get(scope)
                .names()
                .find_map(|(name, sym)| match sym {
                    Symbol::Type(VType::Packed(q)) if same(q) => Some(name.to_owned()),
                    _ => None,
                })
        };
        for (pkg, scope) in &self.env.cx.packages {
            if let Some(name) = find(*scope) {
                return Some(format!("{pkg}::{name}"));
            }
        }
        let mut cur = Some(self.env.scope);
        while let Some(scope) = cur {
            if let Some(name) = find(scope) {
                return Some(format!("{}.{name}", self.b.module().name));
            }
            cur = self.env.cx.scopes.get(scope).parent;
        }
        None
    }

    /// The named scopes between the module and the statement being
    /// lowered, dotted: named blocks, generate blocks, and the task or
    /// function an inlined body came from. An unnamed scope the lowering
    /// made for itself (a `for` loop's own) is not one.
    fn scope_suffix(&self) -> String {
        let qualified = self.env.qualified("");
        let parts: Vec<&str> = qualified
            .split('.')
            .filter(|c| !c.is_empty())
            .filter_map(|c| match c.split_once('$') {
                // A fresh name: `where$3` is an inlined call of `where`.
                Some(("loop", _)) => None,
                Some((base, _)) => Some(base),
                None => Some(c),
            })
            .collect();
        parts.join(".")
    }

    /// The name an argument denotes when it is a scope, an instance or an
    /// array rather than a value.
    fn scope_argument(&mut self, e: &'ast Expr) -> Option<String> {
        let sym = self.env.probe(|env| env.resolve_path(e))?;
        let qualified = match sym {
            // Name resolution sees through a select to what is selected
            // from, so `mem[1]` resolves to `mem`; the word is meant.
            Symbol::Memory { .. } if matches!(e.kind, ExprKind::Index { .. }) => return None,
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
            let ty = self.net_type(result).clone();
            let v = self.value_as(value, &ty, &mut Sink::Proc(out));
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
            self.case_inside(c, span, out);
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
        let (qualifier, default) = qualify(c.qualifier, default, &self.b, span);
        out.span = span;
        out.case(subject, kind, qualifier, arms, default);
    }

    /// `unique if`, `unique0 if` or `priority if` and its `else if` chain,
    /// as `case (1'b1)` with the same qualifier, which is what they mean
    /// (IEEE 1800-2017 §12.4.2): the first true condition's branch runs,
    /// and the simulator warns when none is true and there is no `else`,
    /// or, for `unique` and `unique0`, when more than one is.
    fn qualified_if(&mut self, first: &'ast ast::If, span: Span, out: &mut BlockBuilder) {
        let mut arms = Vec::new();
        let mut cur = first;
        let mut last_else = None;
        loop {
            let cond = self.expr_bit(&cur.cond, &mut Sink::Proc(out));
            let mut body = self.b.block();
            body.span = cur.then_stmt.span;
            self.stmt(&cur.then_stmt, &mut body);
            arms.push(CaseArm {
                values: vec![cond],
                body: body.finish(),
            });
            match &cur.else_stmt {
                Some(e) => match &e.kind {
                    AstStmt::If(next) if next.qualifier.is_none() => cur = next,
                    _ => {
                        last_else = Some(e);
                        break;
                    }
                },
                None => break,
            }
        }
        let default = last_else.map(|e| {
            let mut b = self.b.block();
            b.span = e.span;
            self.stmt(e, &mut b);
            b.finish()
        });
        let (qualifier, default) = qualify(first.qualifier, default, &self.b, span);
        self.b.span = span;
        let one = self.b.const_bit(true);
        out.span = span;
        out.case(one, CaseKind::Plain, qualifier, arms, default);
    }

    /// `case (x) inside`: the first arm whose set the subject is `inside`
    /// (IEEE 1800-2017 §12.5.4), so a range or a wildcard in an item
    /// matches as `inside` matches it; a chain of `if`s.
    fn case_inside(&mut self, c: &'ast ast::Case, span: Span, out: &mut BlockBuilder) {
        let mut tests = Vec::new();
        let mut default = Vec::new();
        for item in &c.items {
            let mut body = self.b.block();
            body.span = item.span;
            self.stmt(&item.body, &mut body);
            let body = body.finish();
            if item.patterns.is_empty() {
                if default.is_empty() {
                    default = body;
                }
                continue;
            }
            let hit = self.inside(&c.expr, &c.expr, &item.patterns, &mut Sink::Proc(out));
            tests.push((hit, body));
        }
        let mut rest = default;
        for (hit, body) in tests.into_iter().rev() {
            let mut b = self.b.block();
            b.span = span;
            b.if_(hit, body, rest);
            rest = b.finish();
        }
        for stmt in rest {
            out.span = stmt.span;
            out.push(stmt.kind);
        }
    }

    /// `foreach (a[i, j]) body`: one loop per named dimension, outermost
    /// first, each from its left bound to its right (IEEE 1800-2017
    /// §12.7.3); the unpacked dimensions come before the packed ones.
    fn foreach_stmt(&mut self, f: &'ast ast::Foreach, span: Span, out: &mut BlockBuilder) {
        let mut dims = Vec::new();
        let mut ty = self.value_type(&f.array);
        while let Some(VType::Unpacked { elem, range }) = ty {
            dims.push(range);
            ty = Some(*elem);
        }
        if let Some(VType::Packed(p)) = &ty {
            dims.extend(p.dims.iter().copied());
        }
        if f.vars.len() > dims.len() {
            self.env.error(
                codes::TYPE,
                f.array.span,
                format!(
                    "`foreach` names {} loop variables and the array has {} dimensions",
                    f.vars.len(),
                    dims.len()
                ),
            );
            return;
        }
        let loop_name = self.fresh("foreach");
        self.env.enter_new(format!("{loop_name}."));
        let mut loops = Vec::new();
        for (var, range) in f.vars.iter().zip(&dims) {
            let Some(var) = var else { continue };
            let ty = VType::integer();
            let ir_name = self.env.qualified(&var.name);
            let Some(net) = self.new_var(ir_name, ty.clone(), var.span) else {
                continue;
            };
            self.env
                .declare(&var.name, Symbol::Net { net, ty }, var.span);
            loops.push((net, *range));
        }
        let mut body = self.b.block();
        body.span = f.body.span;
        self.stmt(&f.body, &mut body);
        let mut inner = body.finish();
        for (net, range) in loops.into_iter().rev() {
            self.b.span = span;
            let start = self.b.const_i64(32, range.left);
            let end = self.b.const_i64(32, range.right);
            let i = self.b.net(net);
            let up = range.left <= range.right;
            let cond = if up {
                self.b.binary(ir::BinaryOp::Le, i, end)
            } else {
                self.b.binary(ir::BinaryOp::Ge, i, end)
            };
            let one = self.b.const_i64(32, 1);
            let next = if up {
                self.b.add(i, one)
            } else {
                self.b.sub(i, one)
            };
            self.record_driver(net, self.driver, true, span);
            let mut l = self.b.block();
            l.span = span;
            l.for_(
                Some((ir::Lvalue::Net(net), start)),
                Some(cond),
                Some((ir::Lvalue::Net(net), next)),
                inner,
            );
            inner = l.finish();
        }
        self.env.leave();
        for stmt in inner {
            out.span = stmt.span;
            out.push(stmt.kind);
        }
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
                        let v = self.value_as(value, &ty, &mut Sink::Proc(out));
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
                        let v = if self.target_is_real(&target) {
                            self.real(rhs, &mut Sink::Proc(out))
                        } else if self.target_is_string(&target) {
                            self.string_value(rhs, &mut Sink::Proc(out))
                        } else {
                            self.expr(rhs, Some(Ctx { width, signed }), &mut Sink::Proc(out))
                        };
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
                        // A wait counts the module's units; a fraction of
                        // one is a real, which the simulator rounds to the
                        // precision.
                        self.b.span = ctrl.span;
                        let (unit_fs, _) = self.time_unit();
                        let fs = delay.to_fs();
                        if unit_fs == 0 || fs.is_multiple_of(unit_fs) {
                            self.b.const_u64(64, fs / unit_fs.max(1))
                        } else {
                            #[allow(clippy::cast_precision_loss, reason = "a delay below 2^53 fs")]
                            let units = fs as f64 / unit_fs as f64;
                            self.real_const(units)
                        }
                    }
                    None => self.expr(e, None, &mut Sink::Proc(out)),
                };
                out.push(StmtKind::Wait(WaitKind::Delay(id)));
            }
            ast::TimingKind::Event(ev) => {
                let edges = self.event_edges(ev);
                let guard = self.event_iff(ev);
                self.wait_guarded(edges, guard, ctrl.span, out);
            }
            ast::TimingKind::RepeatEvent(count, ev) => {
                let n = self.expr(count, None, &mut Sink::Proc(out));
                let edges = self.event_edges(ev);
                let guard = self.event_iff(ev);
                let mut inner = self.b.block();
                inner.span = ctrl.span;
                self.wait_guarded(edges, guard, ctrl.span, &mut inner);
                out.span = ctrl.span;
                out.repeat(n, inner.finish());
            }
        }
    }

    /// Waits for `edges`, and while `guard` (an `iff`) does not hold, for
    /// them again: `@(posedge clk iff en)` is an edge at which `en` is 1
    /// (IEEE 1800-2017 §9.4.2.3).
    fn wait_guarded(
        &mut self,
        edges: Vec<Edge>,
        guard: Option<&'ast Expr>,
        span: Span,
        out: &mut BlockBuilder,
    ) {
        out.span = span;
        out.push(StmtKind::Wait(WaitKind::Event(edges.clone())));
        let Some(cond) = guard else { return };
        let c = self.expr_bit(cond, &mut Sink::Proc(out));
        self.b.span = span;
        let not_yet = self.b.lnot(c);
        let mut again = self.b.block();
        again.span = span;
        again.push(StmtKind::Wait(WaitKind::Event(edges)));
        out.span = span;
        out.while_(not_yet, again.finish());
    }

    /// The `iff` of an event control of one event, or `None` for none; a
    /// list of several with an `iff` is refused.
    pub(super) fn event_iff(&mut self, ev: &'ast ast::EventControl) -> Option<&'ast Expr> {
        let ast::EventControlKind::List(list) = &ev.kind else {
            return None;
        };
        match list.as_slice() {
            [one] => one.iff.as_ref(),
            many => {
                if let Some(item) = many.iter().find(|i| i.iff.is_some()) {
                    self.env.unsupported(
                        item.span,
                        "an `iff` condition in a list of more than one event",
                    );
                }
                None
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
                    let polarity = match item.edge {
                        Some(ast::Edge::Posedge) => Polarity::Pos,
                        Some(ast::Edge::Negedge) => Polarity::Neg,
                        _ => Polarity::Any,
                    };
                    let nets = if polarity == Polarity::Any {
                        self.event_nets(&item.expr)
                    } else {
                        self.edge_net(&item.expr).into_iter().collect()
                    };
                    for net in nets {
                        edges.push(Edge { net, polarity });
                    }
                }
            }
        }
        edges
    }

    /// The one-bit net whose edge `posedge e` or `negedge e` waits for.
    ///
    /// A one-bit net is itself. Anything else, a bit-select `x[i]`, a
    /// part-select or a whole vector, is an edge of the expression's least
    /// significant bit (IEEE 1364-2005 §9.7.2), so it gets a net of its own,
    /// driven by a continuous assignment of that bit, and the edge is that
    /// net's. Watching the whole vector, as an edge on `x[1]` did, was an
    /// edge on a net of two bits, which the IR refuses (`V0024`); and an edge
    /// of `x` would have been one of any of its bits.
    fn edge_net(&mut self, e: &'ast Expr) -> Option<NetId> {
        if let Some(Symbol::Net { net, .. }) = self.env.probe(|env| env.resolve_path(e))
            && self.net_width(net) == 1
        {
            return Some(net);
        }
        let value = self.expr_cont(e, None);
        let width = self.b.module().expr(value).ty.width()?;
        if width == 0 {
            return None;
        }
        self.b.span = e.span;
        let bit = if width == 1 {
            value
        } else {
            self.b.slice(value, 0, 0)
        };
        let name = format!("edge${}", self.b.module().nets.len());
        let net = self.new_net(name, VType::bit(), false, e.span)?;
        self.b.span = e.span;
        self.b.assign(net, bit);
        Some(net)
    }

    /// The nets an event expression watches.
    fn event_nets(&mut self, e: &'ast Expr) -> Vec<NetId> {
        let selected = matches!(e.kind, ExprKind::Index { .. } | ExprKind::Range { .. });
        match self.env.probe(|env| env.resolve_path(e)) {
            Some(Symbol::Net { net, .. }) if !selected => vec![net],
            _ => match &e.kind {
                ExprKind::Concat(parts) => parts.iter().flat_map(|p| self.event_nets(p)).collect(),
                _ => {
                    // `@(v[2])` waits for that bit to change, `@(a + b)` for
                    // the sum to: a net of its own holds the value, and the
                    // wait is for it. Watching the whole of `v` used to wake
                    // on a change of any of its bits.
                    let value = self.expr_cont(e, None);
                    let Some(width) = self.b.module().expr(value).ty.width().filter(|w| *w > 0)
                    else {
                        self.env.error(
                            codes::UNSUPPORTED,
                            e.span,
                            "an event expression must name a signal or compute a value",
                        );
                        return Vec::new();
                    };
                    let name = format!("event${}", self.b.module().nets.len());
                    let Some(net) =
                        self.new_net(name, VType::Packed(Packed::bits(width)), false, e.span)
                    else {
                        return Vec::new();
                    };
                    self.b.span = e.span;
                    self.b.assign(net, value);
                    vec![net]
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
        // With no pass action and no fail action but `$error`, the
        // assertion is one statement; the message with none is xsim's.
        let error_args = match &a.else_stmt {
            None => Some(None),
            Some(else_stmt) => match &else_stmt.kind {
                AstStmt::Expr(e) => match &e.kind {
                    ExprKind::Call { callee, args } => match &callee.kind {
                        ExprKind::SystemIdent(id) if id.name == "error" => Some(Some(args)),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
        };
        if a.then_stmt.is_none()
            && let Some(args) = error_args
        {
            let message = match args {
                Some(args) => self.syscall_args(args, out),
                None => self.violation(span),
            };
            out.span = span;
            out.assert(c, ReportSeverity::Error, message);
            return;
        }
        // Otherwise the pass action runs when the condition is true and
        // the fail action, or the default error, when it is false or
        // unknown (IEEE 1800-2017 §16.3): never both.
        let mut pass = self.b.block();
        if let Some(then_stmt) = &a.then_stmt {
            pass.span = then_stmt.span;
            self.stmt(then_stmt, &mut pass);
        }
        let mut fail = self.b.block();
        fail.span = span;
        match &a.else_stmt {
            Some(else_stmt) => self.stmt(else_stmt, &mut fail),
            None => {
                let message = self.violation(span);
                self.b.span = span;
                let never = self.b.const_bit(false);
                fail.span = span;
                fail.assert(never, ReportSeverity::Error, message);
            }
        }
        out.span = span;
        out.if_(c, pass.finish(), fail.finish());
    }

    /// The message of a failing assertion that gives none: xsim 2026.1
    /// prints `Error: Assertion violation`.
    fn violation(&mut self, span: Span) -> Vec<ExprId> {
        self.b.span = span;
        vec![self.b.string("Assertion violation".to_owned())]
    }
}
