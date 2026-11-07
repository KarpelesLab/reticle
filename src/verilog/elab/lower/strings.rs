//! SystemVerilog `string` variables (IEEE 1800-2017 §6.16).
//!
//! A string is simulation-only: an IR net of [`Type::String`] that the
//! simulator holds as however many characters it was last given. What is
//! *string-typed* here is what the standard calls a string expression: a
//! `string` variable, a function or method returning one, `$sformatf`, a
//! `string'(..)` cast, and a concatenation, replication or `?:` with one of
//! those inside. A literal alone is not: `"AB"` is the integer `16'h4142`
//! until a string context makes it a string, which is what keeps every
//! Verilog use of a literal as it was.
//!
//! In a string context (an assignment to a string, a string argument, a
//! comparison with a string) the value is built as a string: a literal as
//! its text, a concatenation through `$sformatf("%s%s..")`, an integral
//! value as its characters (`$sformatf("%s", v)`, eight bits a character,
//! leading NULs dropped). A string in an integral context is resized, its
//! characters' bits as a literal's are.
//!
//! The methods are calls the simulator implements, named `string.<method>`
//! (`docs/ir.md`): `len`, `getc`, `toupper`, `tolower`, `substr`,
//! `compare`, `icompare`, `atoi`, `atohex`, `atooct`, `atobin`, `atoreal`,
//! and, assigned back to the variable, `putc`, `itoa`, `hextoa`, `octtoa`,
//! `bintoa` and `realtoa`; `string.replicate` is `{n{s}}` with `n` known
//! only at run time.

use crate::ir::builder::BlockBuilder;
use crate::ir::{self, AssignKind, BinaryOp as IrBin, ExprId, Type};
use crate::source::Span;
use crate::verilog::ast::{self, BinaryOp, CastTarget, Expr, ExprKind};

use super::super::codes;
use super::super::constant::{Evaluator, Value};
use super::super::types::VType;
use super::Lowerer;
use super::expr::Sink;

/// The methods whose result is a string.
const STRING_RESULT: &[&str] = &["toupper", "tolower", "substr"];

/// The methods that change the variable they are called on, and the
/// arguments each takes.
const MUTATORS: &[(&str, usize)] = &[
    ("putc", 2),
    ("itoa", 1),
    ("hextoa", 1),
    ("octtoa", 1),
    ("bintoa", 1),
    ("realtoa", 1),
];

/// The methods that read a string, the arguments each takes and the type
/// of what they return.
fn reader(name: &str) -> Option<(usize, Type)> {
    Some(match name {
        "len" | "atoi" | "atohex" | "atooct" | "atobin" => (
            0,
            Type::Bits {
                width: 32,
                signed: true,
            },
        ),
        "getc" => (1, Type::bits(8)),
        "toupper" | "tolower" => (0, Type::String),
        "substr" => (2, Type::String),
        "compare" | "icompare" => (
            1,
            Type::Bits {
                width: 32,
                signed: true,
            },
        ),
        "atoreal" => (0, Type::Real),
        _ => return None,
    })
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// True when `e` is a string expression in its own right (see the
    /// module docs); a literal is not.
    pub(super) fn is_string_typed(&mut self, e: &'ast Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::Scoped { .. } | ExprKind::Member { .. } => matches!(
                self.env.probe(|env| env.type_of_path(e)),
                Some(VType::String)
            ),
            ExprKind::Concat(parts) | ExprKind::Replicate { elems: parts, .. } => {
                parts.iter().any(|p| self.is_string_typed(p))
            }
            ExprKind::Ternary {
                then_expr,
                else_expr,
                ..
            } => self.is_string_typed(then_expr) || self.is_string_typed(else_expr),
            ExprKind::Cast {
                target: CastTarget::Type(t),
                ..
            } => t.kind == ast::DataTypeKind::String,
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::SystemIdent(id) => matches!(id.name.as_str(), "sformatf" | "psprintf"),
                ExprKind::Member { base, name }
                    if STRING_RESULT.contains(&name.name.as_str())
                        && self.is_string_typed(base) =>
                {
                    true
                }
                ExprKind::Member { base, name } if name.name == "name" => self
                    .value_type(base)
                    .is_some_and(|t| t.packed().is_some_and(|p| p.enumeration.is_some())),
                _ => match self.env.probe(|env| env.resolve_callee(callee)) {
                    Some((f, false)) => f
                        .def
                        .ret
                        .as_ref()
                        .is_some_and(|r| r.kind == ast::DataTypeKind::String),
                    _ => false,
                },
            },
            _ => false,
        }
    }

    /// True when `lv` is a whole `string` variable.
    pub(super) fn target_is_string(&self, lv: &ir::Lvalue) -> bool {
        match lv {
            ir::Lvalue::Net(net) => matches!(
                self.nets.get(net.index()).map(|n| &n.ty),
                Some(VType::String)
            ),
            ir::Lvalue::MemElem { mem, .. } => self.mems[mem.index()].elem == VType::String,
            _ => false,
        }
    }

    /// Lowers `e` as a string: what an assignment to a string variable,
    /// or a string argument, receives.
    pub(super) fn string_value(&mut self, e: &'ast Expr, sink: &mut Sink<'_>) -> ExprId {
        self.b.span = e.span;
        if let Some(Value::Str(s)) = self.env.probe(|env| Evaluator::new(env).eval(e, None).ok()) {
            self.b.span = e.span;
            return self.b.string(s);
        }
        match &e.kind {
            ExprKind::Concat(parts) => {
                let ids: Vec<ExprId> = parts.iter().map(|p| self.string_value(p, sink)).collect();
                self.b.span = e.span;
                self.sformat_strings(ids)
            }
            ExprKind::Replicate { count, elems } => {
                let one: Vec<ExprId> = elems.iter().map(|p| self.string_value(p, sink)).collect();
                let Some(n) = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_u32(count).ok())
                else {
                    // A string may be replicated a number of times known
                    // only when it runs (IEEE 1800-2017 §11.4.12.2).
                    let n = self.build_self(count, sink);
                    self.b.span = e.span;
                    let inner = self.sformat_strings(one);
                    return self
                        .b
                        .call("string.replicate", vec![inner, n], Type::String);
                };
                let mut ids = Vec::new();
                for _ in 0..n {
                    ids.extend(one.iter().copied());
                }
                self.b.span = e.span;
                self.sformat_strings(ids)
            }
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                let c = self.expr_bit(cond, sink);
                let t = self.string_value(then_expr, sink);
                let f = self.string_value(else_expr, sink);
                self.b.span = e.span;
                self.b.mux(c, t, f)
            }
            ExprKind::Cast {
                target: CastTarget::Type(t),
                expr,
            } if t.kind == ast::DataTypeKind::String => self.string_value(expr, sink),
            _ => {
                let id = self.build_self(e, sink);
                self.string_of(id)
            }
        }
    }

    /// An already-lowered value as a string: a string as it is, an
    /// integral value as its characters.
    pub(super) fn string_of(&mut self, id: ExprId) -> ExprId {
        let node = self.b.module().expr(id);
        if node.ty == Type::String {
            return id;
        }
        self.b.span = node.span;
        let fmt = self.b.string("%s".to_owned());
        self.b.call("$sformatf", vec![fmt, id], Type::String)
    }

    /// The strings `ids` joined, through `$sformatf("%s%s..")`.
    fn sformat_strings(&mut self, ids: Vec<ExprId>) -> ExprId {
        let fmt = self.b.string("%s".repeat(ids.len()));
        let mut args = Vec::with_capacity(ids.len() + 1);
        args.push(fmt);
        args.extend(ids);
        self.b.call("$sformatf", args, Type::String)
    }

    /// A comparison with a string on either side, or `None` when neither
    /// is one: both are then strings, compared as the standard compares
    /// them.
    pub(super) fn string_compare(
        &mut self,
        e: &'ast Expr,
        op: BinaryOp,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let irop = match op {
            BinaryOp::Eq => IrBin::Eq,
            BinaryOp::Ne => IrBin::Ne,
            BinaryOp::CaseEq => IrBin::CaseEq,
            BinaryOp::CaseNe => IrBin::CaseNe,
            BinaryOp::Lt => IrBin::Lt,
            BinaryOp::Le => IrBin::Le,
            BinaryOp::Gt => IrBin::Gt,
            BinaryOp::Ge => IrBin::Ge,
            _ => return None,
        };
        if !self.is_string_typed(lhs) && !self.is_string_typed(rhs) {
            return None;
        }
        let a = self.string_value(lhs, sink);
        let b = self.string_value(rhs, sink);
        self.b.span = e.span;
        Some(self.b.binary(irop, a, b))
    }

    /// `s.method(..)` used as a value, or `None` when `callee` is not a
    /// method of a string.
    pub(super) fn string_method(
        &mut self,
        e: &'ast Expr,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let ExprKind::Member { base, name } = &callee.kind else {
            return None;
        };
        if !self.is_string_typed(base) {
            return None;
        }
        let method = name.name.as_str();
        let Some((count, ty)) = reader(method) else {
            if MUTATORS.iter().any(|(m, _)| *m == method) {
                self.env.error(
                    codes::TYPE,
                    e.span,
                    format!(
                        "`{method}` changes the string and returns nothing; call it as a statement"
                    ),
                );
            } else {
                self.env.error(
                    codes::UNDEFINED,
                    name.span,
                    format!("a string has no method `{method}`"),
                );
            }
            return Some(self.b.const_u64(1, 0));
        };
        let s = self.string_value(base, sink);
        let mut ids = vec![s];
        ids.extend(self.method_args(e.span, method, args, count, sink)?);
        self.b.span = e.span;
        Some(self.b.call(format!("string.{method}"), ids, ty))
    }

    /// `s.putc(..)`, `s.itoa(..)` and the like as a statement: `s` is
    /// assigned what the method makes of it. `None` when `callee` is not
    /// such a method of a string variable.
    pub(super) fn string_mutator(
        &mut self,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<()> {
        let ExprKind::Member { base, name } = &callee.kind else {
            return None;
        };
        let method = name.name.as_str();
        let (_, count) = *MUTATORS.iter().find(|(m, _)| *m == method)?;
        if !self.is_string_typed(base) {
            return None;
        }
        let target = self.lvalue(base, true)?;
        let mut sink = Sink::Proc(out);
        let s = self.string_value(base, &mut sink);
        let mut ids = vec![s];
        ids.extend(self.method_args(span, method, args, count, &mut sink)?);
        self.b.span = span;
        let v = self.b.call(format!("string.{method}"), ids, Type::String);
        out.span = span;
        out.assign(target.clone(), v, AssignKind::Blocking);
        self.note_assign_drivers(&target, self.driver, span, true);
        Some(())
    }

    /// A method's arguments, each lowered at its own width, a real as a
    /// real.
    fn method_args(
        &mut self,
        span: Span,
        method: &str,
        args: &'ast [ast::Arg],
        count: usize,
        sink: &mut Sink<'_>,
    ) -> Option<Vec<ExprId>> {
        let values: Vec<&'ast Expr> = args.iter().filter_map(|a| a.value.as_ref()).collect();
        if values.len() != count {
            self.env.error(
                codes::ARGUMENTS,
                span,
                format!("`{method}` takes {count} argument(s), not {}", values.len()),
            );
            return None;
        }
        let mut ids = Vec::with_capacity(count);
        for v in values {
            let id = if method == "realtoa" {
                self.real(v, sink)
            } else if method == "compare" || method == "icompare" {
                self.string_value(v, sink)
            } else {
                self.build_self(v, sink)
            };
            ids.push(id);
        }
        Some(ids)
    }
}
