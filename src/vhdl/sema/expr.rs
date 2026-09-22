//! Name resolution and expression typing (IEEE 1076-2008 clauses 8 and 9).
//!
//! # Two phases
//!
//! VHDL expressions cannot be typed bottom-up alone: `'0'` belongs to
//! `bit`, `std_ulogic` and any user character type, `"1010"` to every
//! character array, an aggregate to every composite type, and `+` may be
//! any of a dozen visible functions. The checker therefore types every
//! expression twice:
//!
//! 1. `Checker::infer` walks the expression bottom-up and returns the set
//!    of types it *could* have as [`Cand`]s, reporting nothing. Results are
//!    memoised per span in `infer_cache`.
//! 2. `Checker::resolve` walks it top-down against the type the context
//!    requires, picks the single interpretation compatible with it,
//!    records the resolved type, the resolved declaration of every name,
//!    the [`CallTarget`] of every call and operator, and the static value
//!    when one can be computed. Mismatches and ambiguities are reported
//!    here, where the expected type is known and the message can name it.
//!
//! `Checker::resolve_free` is the context-free entry point used where
//! the language gives no expected type (the selector of a `case`, an
//! actual of a `port map` before the formal is known): it infers, and
//! requires exactly one candidate.
//!
//! # Names
//!
//! `Checker::classify` turns a [`Name`] into a [`Prefix`]: a library or
//! package (a region to select into), a type mark, an object, a value, a
//! set of overloaded subprograms, or an error. Every name form of clause 8
//! goes through it, including the parser's deliberately ambiguous
//! [`Name::Call`], which may be a function call, an array index, a type
//! conversion or an attribute argument.

use crate::diag::Diagnostic;
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast::{
    self, Actual, BinaryOp, Direction, DiscreteRange, Expr, LiteralKind, Name, UnaryOp,
};

use super::attrs::{self, Predefined};
use super::check::Checker;
use super::constant::{ArrayValue, Value, int, parse_integer_literal, parse_real_literal};
use super::overload::{self, Cand};
use super::scope;
use super::types::{Bound, Bounds, Constraint, TypeClass, TypeId, TypeKind};
use super::{CallTarget, DeclId, DeclKind, ObjectClass, ObjectRole, RangeInfo, RegionId};

/// Whether a resolution step reports diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Inference: stay silent, the caller may try another interpretation.
    Infer,
    /// Commit: report what does not resolve.
    Commit,
}

/// What an object name denotes, for the assignment and mode rules.
#[derive(Clone, Copy, Debug)]
pub struct ObjInfo {
    /// The object's declaration (the aliased object for an alias).
    pub decl: DeclId,
    /// Its class.
    pub class: ObjectClass,
    /// Its mode, for ports and parameters.
    pub mode: Option<ast::Mode>,
    /// Where it was declared.
    pub role: ObjectRole,
}

/// What a name denotes.
#[derive(Clone, Debug)]
pub enum Prefix {
    /// A library or package: a region to select names out of.
    Region(DeclId, RegionId),
    /// A type or subtype mark.
    Type(TypeId),
    /// An object, with its subtype.
    Object(ObjInfo, TypeId),
    /// A value that is not an object (a function result, an attribute).
    Value(TypeId),
    /// A value whose type is not yet decided (an overloaded call in
    /// inference mode).
    Values(Vec<TypeId>),
    /// A set of overloaded subprograms or enumeration literals.
    Overloaded(Vec<DeclId>),
    /// A design unit that is not a package (an entity, a configuration).
    Unit(super::UnitId),
    /// Unresolvable; the error has been reported (in `Commit` mode).
    Error,
}

/// What `Checker::commit_name` tells the statement checker.
#[derive(Clone, Debug, Default)]
pub struct NameInfo {
    /// The name's type, when it denotes a value.
    pub ty: Option<TypeId>,
    /// The object, when the name denotes one.
    pub obj: Option<ObjInfo>,
}

impl Checker<'_> {
    // --- entry points ------------------------------------------------------

    /// The candidate types of `e`, memoised.
    pub(crate) fn infer(&mut self, e: &Expr) -> Vec<Cand> {
        if let Some(c) = self.infer_cache.get(&e.span()) {
            return c.clone();
        }
        let mut cands = self.infer_uncached(e);
        overload::dedup(self.a, &mut cands);
        self.infer_cache.insert(e.span(), cands.clone());
        cands
    }

    fn infer_uncached(&mut self, e: &Expr) -> Vec<Cand> {
        match e {
            Expr::Literal(l) => self.infer_literal(l),
            Expr::Name(n) => self.infer_name(n),
            Expr::Paren { inner, .. } => self.infer(inner),
            Expr::Aggregate(_) => vec![Cand::Aggregate],
            Expr::Qualified { type_mark, .. } => {
                let t = self.resolve_type_mark_quiet(type_mark);
                vec![Cand::Ty(t)]
            }
            Expr::Allocator { kind, .. } => {
                // `new t` has some access type designating `t`; the
                // context decides which.
                let designated = match kind.as_ref() {
                    ast::Allocator::Subtype(si) => self.resolve_type_mark_quiet(&si.type_mark),
                    ast::Allocator::Qualified { type_mark, .. } => {
                        self.resolve_type_mark_quiet(type_mark)
                    }
                };
                let mut out = Vec::new();
                for t in 0..self.a.types.len() {
                    let id = TypeId(u32::try_from(t).expect("type count"));
                    if let TypeKind::Access(d) = self.a.ty(id).kind
                        && self.a.same_base(d, designated)
                    {
                        out.push(Cand::Ty(id));
                    }
                }
                out
            }
            Expr::Binary { op, lhs, rhs, span } => {
                let l = self.infer(lhs);
                let r = self.infer(rhs);
                let mut out: Vec<Cand> = Vec::new();
                for c in overload::binary_ops(self.a, *op, &l, &r, None, self.standard) {
                    out.push(Cand::Ty(c.result));
                }
                for d in self.operator_decls(op.as_str(), 2) {
                    if let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind
                        && let Some(ret) = sig.ret
                        && sig.params.len() == 2
                        && overload::any_compatible(self.a, &l, sig.params[0].ty)
                        && overload::any_compatible(self.a, &r, sig.params[1].ty)
                    {
                        out.push(Cand::Ty(ret));
                    }
                }
                let _ = span;
                out
            }
            Expr::Unary { op, operand, .. } => {
                let o = self.infer(operand);
                let mut out: Vec<Cand> = Vec::new();
                for c in overload::unary_ops(self.a, *op, &o, self.standard) {
                    out.push(Cand::Ty(c.result));
                }
                for d in self.operator_decls(op.as_str(), 1) {
                    if let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind
                        && let Some(ret) = sig.ret
                        && sig.params.len() == 1
                        && overload::any_compatible(self.a, &o, sig.params[0].ty)
                    {
                        out.push(Cand::Ty(ret));
                    }
                }
                out
            }
            Expr::Open(_) | Expr::Error(_) => vec![Cand::Error],
        }
    }

    fn infer_literal(&mut self, l: &ast::Literal) -> Vec<Cand> {
        let b = self.a.builtins;
        match &l.kind {
            LiteralKind::Integer(_) => vec![Cand::Ty(b.universal_integer)],
            LiteralKind::Real(_) => vec![Cand::Ty(b.universal_real)],
            LiteralKind::Physical { unit, .. } => {
                let sym = self.ident_sym(unit);
                let lk = scope::lookup(self.a, self.region, sym);
                lk.decls
                    .iter()
                    .filter_map(|&d| match self.a.decl(d).kind {
                        DeclKind::PhysicalUnit { ty, .. } => Some(Cand::Ty(ty)),
                        _ => None,
                    })
                    .collect()
            }
            LiteralKind::Char(c) => {
                let sym = self.a.interner.intern(&format!("'{c}'"));
                let lk = scope::lookup(self.a, self.region, sym);
                lk.decls
                    .iter()
                    .filter_map(|&d| match self.a.decl(d).kind {
                        DeclKind::EnumLiteral { ty, .. } => Some(Cand::Ty(ty)),
                        _ => None,
                    })
                    .collect()
            }
            LiteralKind::String(_) | LiteralKind::BitString(_) => vec![Cand::StringLit],
            LiteralKind::Null => vec![Cand::Null],
        }
    }

    /// The visible subprogram declarations for an operator symbol with
    /// `arity` parameters.
    pub(crate) fn operator_decls(&mut self, symbol: &str, arity: usize) -> Vec<DeclId> {
        let sym = self
            .a
            .interner
            .intern(&format!("\"{}\"", symbol.to_lowercase()));
        let lk = scope::lookup(self.a, self.region, sym);
        lk.decls
            .into_iter()
            .filter(|&d| match &self.a.decl(d).kind {
                DeclKind::Subprogram { sig, .. } => sig.params.len() == arity && sig.ret.is_some(),
                _ => false,
            })
            .collect()
    }

    /// Resolves `e` against `expected` and returns the resolved type
    /// (`expected` itself when it resolved, the error type otherwise).
    pub(crate) fn resolve(&mut self, e: &Expr, expected: TypeId) -> TypeId {
        let ty = self.resolve_inner(e, expected, Mode::Commit);
        self.a.set_type(e.span(), ty);
        ty
    }

    /// Resolves `e` with no expected type: it must have exactly one
    /// candidate.
    pub(crate) fn resolve_free(&mut self, e: &Expr) -> TypeId {
        let cands = self.infer(e);
        let concrete: Vec<TypeId> = cands
            .iter()
            .filter_map(|c| match c {
                Cand::Ty(t) => Some(*t),
                _ => None,
            })
            .collect();
        if cands.iter().any(|c| matches!(c, Cand::Error)) {
            return self.a.builtins.error;
        }
        match concrete.len() {
            1 => self.resolve(e, concrete[0]),
            0 => {
                if cands.is_empty() {
                    self.error(
                        "V0301",
                        e.span(),
                        "the type of this expression cannot be determined here",
                    );
                } else {
                    self.error(
                        "V0301",
                        e.span(),
                        "this expression needs a context to determine its type; qualify it with `type'(...)`",
                    );
                }
                self.a.builtins.error
            }
            _ => {
                let list: Vec<String> = concrete.iter().map(|&t| self.ty_name(t)).collect();
                let mut d = Diagnostic::error("ambiguous expression")
                    .with_code("V0302")
                    .with_label(e.span(), "the type of this expression is ambiguous");
                for t in list {
                    d = d.with_note(format!("it could be `{t}`"));
                }
                self.push(d.with_note("qualify it with `type'(...)` to choose"));
                self.a.builtins.error
            }
        }
    }

    /// Resolves an expression used as a condition: `boolean`, or, in
    /// VHDL-2008, any type with a `??` operator (clause 9.2.9).
    pub(crate) fn resolve_condition(&mut self, e: &Expr) -> TypeId {
        let boolean = self.a.builtins.boolean;
        let cands = self.infer(e);
        if cands.is_empty() || overload::any_compatible(self.a, &cands, boolean) {
            return self.resolve(e, boolean);
        }
        if self.v2008() {
            // The implicit condition conversion.
            let ops: Vec<TypeId> = cands
                .iter()
                .filter_map(|c| match c {
                    Cand::Ty(t) if self.has_condition_operator(*t) => Some(*t),
                    _ => None,
                })
                .collect();
            if let Some(&t) = ops.first() {
                let ty = self.resolve(e, t);
                self.a.set_implicit_condition(e.span());
                return ty;
            }
        }
        self.resolve(e, boolean)
    }

    /// True when `??` is defined for `t`: `bit` and `boolean`
    /// predefined, plus any visible `"??"` function.
    fn has_condition_operator(&mut self, t: TypeId) -> bool {
        if self.a.is_bit_or_boolean(t) {
            return true;
        }
        self.operator_decls("??", 1).into_iter().any(|d| {
            matches!(&self.a.decl(d).kind, DeclKind::Subprogram { sig, .. }
                if overload::types_compatible(self.a, t, sig.params[0].ty))
        })
    }

    // --- the resolving walk ------------------------------------------------

    fn resolve_inner(&mut self, e: &Expr, expected: TypeId, mode: Mode) -> TypeId {
        match e {
            Expr::Literal(l) => self.resolve_literal(l, expected, mode),
            Expr::Name(n) => self.resolve_name_expr(n, expected, mode),
            Expr::Paren { inner, .. } => {
                let t = self.resolve_inner(inner, expected, mode);
                self.a.set_type(inner.span(), t);
                if let Some(v) = self.a.value_of(inner.span()).cloned() {
                    self.a.set_value(e.span(), v);
                }
                t
            }
            Expr::Aggregate(agg) => self.resolve_aggregate(agg, expected, mode),
            Expr::Qualified {
                type_mark,
                operand,
                span,
            } => {
                let t = self.resolve_type_mark(type_mark);
                self.resolve(operand, t);
                if let Some(v) = self.a.value_of(operand.span()).cloned() {
                    self.a.set_value(*span, v);
                }
                self.check_compatible(*span, t, expected, mode);
                t
            }
            Expr::Allocator { kind, span } => {
                let designated = match kind.as_ref() {
                    ast::Allocator::Subtype(si) => self.resolve_subtype_indication(si),
                    ast::Allocator::Qualified { type_mark, operand } => {
                        let t = self.resolve_type_mark(type_mark);
                        self.resolve(operand, t);
                        t
                    }
                };
                match self.a.designated_type(expected) {
                    Some(d) => {
                        if !self.a.same_base(d, designated) && mode == Mode::Commit {
                            let (dn, an) = (self.ty_name(d), self.ty_name(designated));
                            self.error(
                                "V0300",
                                *span,
                                format!(
                                    "allocator creates a `{an}` but `{}` designates `{dn}`",
                                    self.ty_name(expected)
                                ),
                            );
                        }
                        expected
                    }
                    None => {
                        if mode == Mode::Commit && !self.a.is_error(expected) {
                            let en = self.ty_name(expected);
                            self.error(
                                "V0300",
                                *span,
                                format!("`new` yields an access value, not `{en}`"),
                            );
                        }
                        self.a.builtins.error
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, span } => {
                self.resolve_binary(*op, lhs, rhs, *span, expected, mode)
            }
            Expr::Unary { op, operand, span } => {
                self.resolve_unary(*op, operand, *span, expected, mode)
            }
            Expr::Open(span) => {
                if mode == Mode::Commit {
                    self.error(
                        "V0500",
                        *span,
                        "`open` is only allowed as an actual in an association list",
                    );
                }
                self.a.builtins.error
            }
            Expr::Error(_) => self.a.builtins.error,
        }
    }

    fn resolve_literal(&mut self, l: &ast::Literal, expected: TypeId, mode: Mode) -> TypeId {
        let b = self.a.builtins;
        match &l.kind {
            LiteralKind::Integer(text) => {
                let class = self.a.class(expected);
                let ty = if class.is_integer() {
                    expected
                } else if class.is_real() {
                    // An integer literal is not a real literal (clause 9.3.2);
                    // only `universal_integer` in a real context is wrong.
                    if mode == Mode::Commit {
                        let en = self.ty_name(expected);
                        self.push(
                            Diagnostic::error(format!("integer literal in a `{en}` context"))
                                .with_code("V0300")
                                .with_label(l.span, "this is an integer literal")
                                .with_note(format!("write `{text}.0` for a real literal")),
                        );
                    }
                    b.error
                } else {
                    self.check_compatible(l.span, b.universal_integer, expected, mode);
                    b.error
                };
                if let Some(v) = parse_integer_literal(text) {
                    self.a.set_value(l.span, Value::Int(v));
                } else if mode == Mode::Commit {
                    self.error(
                        "V0305",
                        l.span,
                        format!("`{text}` is not a valid integer literal"),
                    );
                }
                ty
            }
            LiteralKind::Real(text) => {
                let ty = if self.a.class(expected).is_real() {
                    expected
                } else {
                    self.check_compatible(l.span, b.universal_real, expected, mode);
                    b.error
                };
                if let Some(v) = parse_real_literal(text) {
                    self.a.set_value(l.span, Value::Real(v));
                } else if mode == Mode::Commit {
                    self.error(
                        "V0305",
                        l.span,
                        format!("`{text}` is not a valid real literal"),
                    );
                }
                ty
            }
            LiteralKind::Physical { value, unit } => {
                let sym = self.ident_sym(unit);
                let lk = scope::lookup(self.a, self.region, sym);
                let found = lk.decls.iter().copied().find(|&d| {
                    matches!(self.a.decl(d).kind, DeclKind::PhysicalUnit { ty, .. }
                        if overload::types_compatible(self.a, ty, expected))
                });
                match found {
                    Some(d) => {
                        self.a.set_ref(unit.span, d);
                        let DeclKind::PhysicalUnit { ty, scale } = self.a.decl(d).kind else {
                            return b.error;
                        };
                        let scale_f = int_as_f64(scale);
                        let n = parse_integer_literal(value)
                            .map(|n| n.saturating_mul(scale))
                            .or_else(|| {
                                parse_real_literal(value)
                                    .and_then(|r| super::check::round_to_i128(r * scale_f))
                            });
                        if let Some(n) = n {
                            self.a.set_value(l.span, Value::Int(n));
                        }
                        self.check_compatible(l.span, ty, expected, mode);
                        ty
                    }
                    None => {
                        if mode == Mode::Commit {
                            let names = scope::visible_names(self.a, self.region);
                            let mut d =
                                Diagnostic::error(format!("unknown physical unit `{}`", unit.name))
                                    .with_code("V0200")
                                    .with_span(unit.span);
                            if let Some(s) =
                                super::suggest(&unit.name, names.iter().map(String::as_str))
                            {
                                d = d.with_note(format!("did you mean `{s}`?"));
                            }
                            self.push(d);
                        }
                        b.error
                    }
                }
            }
            LiteralKind::Char(c) => {
                let sym = self.a.interner.intern(&format!("'{c}'"));
                let lk = scope::lookup(self.a, self.region, sym);
                let found = lk.decls.iter().copied().find(|&d| {
                    matches!(self.a.decl(d).kind, DeclKind::EnumLiteral { ty, .. }
                        if self.a.same_base(ty, expected))
                });
                match found {
                    Some(d) => {
                        self.a.set_ref(l.span, d);
                        let DeclKind::EnumLiteral { ty, pos } = self.a.decl(d).kind else {
                            return b.error;
                        };
                        self.a.set_value(l.span, Value::Enum(pos));
                        // Keep the subtype the context asked for.
                        let _ = ty;
                        expected
                    }
                    None => {
                        if mode == Mode::Commit && !self.a.is_error(expected) {
                            let en = self.ty_name(expected);
                            let mut d =
                                Diagnostic::error(format!("`'{c}'` is not a value of `{en}`"))
                                    .with_code("V0300")
                                    .with_span(l.span);
                            if lk.decls.is_empty()
                                && (*c == '0' || *c == '1' || *c == 'X' || *c == 'Z')
                            {
                                d = d.with_note(
                                    "add `library ieee; use ieee.std_logic_1164.all;` for `std_logic` values",
                                );
                            }
                            self.push(d);
                        }
                        b.error
                    }
                }
            }
            LiteralKind::String(text) => self.resolve_string(l.span, text, expected, mode),
            LiteralKind::BitString(text) => self.resolve_bit_string(l.span, text, expected, mode),
            LiteralKind::Null => {
                if self.a.class(expected) == TypeClass::Access {
                    self.a.set_value(l.span, Value::Null);
                    expected
                } else {
                    if mode == Mode::Commit && !self.a.is_error(expected) {
                        let en = self.ty_name(expected);
                        self.error("V0300", l.span, format!("`null` is not a value of `{en}`"));
                    }
                    b.error
                }
            }
        }
    }

    /// A string literal: every character must be a literal of the element
    /// type (clause 9.3.2).
    fn resolve_string(&mut self, span: Span, text: &str, expected: TypeId, mode: Mode) -> TypeId {
        if !self.a.is_string_type(expected) {
            if mode == Mode::Commit && !self.a.is_error(expected) {
                let en = self.ty_name(expected);
                self.error(
                    "V0300",
                    span,
                    format!("a string literal is not a value of `{en}`"),
                );
            }
            return self.a.builtins.error;
        }
        let elem = self
            .a
            .element_type(expected)
            .unwrap_or(self.a.builtins.error);
        let mut positions = Vec::new();
        let mut ok = true;
        for c in text.chars() {
            let sym = self.a.interner.intern(&format!("'{c}'"));
            let lk = scope::lookup(self.a, self.region, sym);
            let found = lk
                .decls
                .iter()
                .copied()
                .find_map(|d| match self.a.decl(d).kind {
                    DeclKind::EnumLiteral { ty, pos } if self.a.same_base(ty, elem) => Some(pos),
                    _ => None,
                });
            match found {
                Some(p) => positions.push(p),
                None => {
                    ok = false;
                    if mode == Mode::Commit {
                        let en = self.ty_name(elem);
                        self.error(
                            "V0300",
                            span,
                            format!("`'{c}'` in this string literal is not a value of the element type `{en}`"),
                        );
                        break;
                    }
                }
            }
        }
        if ok {
            let v = self.string_value(expected, positions);
            self.a.set_value(span, v);
        }
        expected
    }

    /// Builds a string value with the index bounds the subtype asks for
    /// (or `1 to n` for an unconstrained one).
    fn string_value(&self, ty: TypeId, positions: Vec<u32>) -> Value {
        let n = i128::try_from(positions.len()).unwrap_or(0);
        let (left, dir) = match self.a.index_constraint(ty).and_then(|c| c.first().cloned()) {
            Some(b) => match (b.left.int(), b.dir) {
                (Some(l), d) => (l, d),
                _ => (1, Direction::To),
            },
            None => (1, Direction::To),
        };
        let _ = n;
        Value::Array(ArrayValue {
            left,
            dir,
            elems: positions.into_iter().map(Value::Enum).collect(),
        })
    }

    /// A bit-string literal: nine-state elements mapped onto the element
    /// type's literals (clause 15.8).
    fn resolve_bit_string(
        &mut self,
        span: Span,
        text: &str,
        expected: TypeId,
        mode: Mode,
    ) -> TypeId {
        if !self.a.is_string_type(expected) {
            if mode == Mode::Commit && !self.a.is_error(expected) {
                let en = self.ty_name(expected);
                self.error(
                    "V0300",
                    span,
                    format!("a bit-string literal is not a value of `{en}`"),
                );
            }
            return self.a.builtins.error;
        }
        let elems = match Logic::parse_vhdl_bit_string_std9(text) {
            Ok(e) => e,
            Err(e) => {
                if mode == Mode::Commit {
                    self.error("V0305", span, format!("invalid bit-string literal: {e}"));
                }
                return expected;
            }
        };
        let elem = self
            .a
            .element_type(expected)
            .unwrap_or(self.a.builtins.error);
        let mut positions = Vec::new();
        for s in &elems {
            let sym = self.a.interner.intern(&format!("'{}'", s.to_char()));
            let lk = scope::lookup(self.a, self.region, sym);
            let found = lk
                .decls
                .iter()
                .copied()
                .find_map(|d| match self.a.decl(d).kind {
                    DeclKind::EnumLiteral { ty, pos } if self.a.same_base(ty, elem) => Some(pos),
                    _ => None,
                });
            match found {
                Some(p) => positions.push(p),
                None => {
                    if mode == Mode::Commit {
                        let en = self.ty_name(elem);
                        self.error(
                            "V0300",
                            span,
                            format!("`'{}'` from this bit-string literal is not a value of the element type `{en}`", s.to_char()),
                        );
                    }
                    return expected;
                }
            }
        }
        let v = self.string_value(expected, positions);
        self.a.set_value(span, v);
        expected
    }

    // --- operators ---------------------------------------------------------

    fn resolve_binary(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        expected: TypeId,
        mode: Mode,
    ) -> TypeId {
        let l = self.infer(lhs);
        let r = self.infer(rhs);
        // Explicit (user or package) operators first; an explicit
        // declaration hides the implicit one with the same profile.
        let mut explicit: Vec<(DeclId, TypeId, TypeId, TypeId)> = Vec::new();
        for d in self.operator_decls(op.as_str(), 2) {
            let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind else {
                continue;
            };
            let (p0, p1, ret) = (sig.params[0].ty, sig.params[1].ty, sig.ret);
            let Some(ret) = ret else { continue };
            if overload::compatible(self.a, Cand::Ty(ret), expected)
                && overload::any_compatible(self.a, &l, p0)
                && overload::any_compatible(self.a, &r, p1)
            {
                explicit.push((d, p0, p1, ret));
            }
        }
        let implicit: Vec<_> =
            overload::binary_ops(self.a, op, &l, &r, Some(expected), self.standard)
                .into_iter()
                .filter(|c| overload::compatible(self.a, Cand::Ty(c.result), expected))
                .filter(|c| {
                    !explicit.iter().any(|(_, p0, p1, ret)| {
                        self.a.same_base(*p0, c.operands[0])
                            && self.a.same_base(*p1, c.operands[1])
                            && self.a.same_base(*ret, c.result)
                    })
                })
                .collect();
        let total = explicit.len() + implicit.len();
        if total == 0 {
            if mode == Mode::Commit {
                self.report_no_operator(
                    op.as_str(),
                    span,
                    &[(&l, lhs.span()), (&r, rhs.span())],
                    expected,
                );
            }
            return self.a.builtins.error;
        }
        if total > 1 {
            // VHDL has no preference rule between operator
            // interpretations: more than one that fits the context is an
            // ambiguity, and the user resolves it by qualifying an operand
            // (clause 12.5).
            if mode == Mode::Commit {
                let mut d =
                    Diagnostic::error(format!("ambiguous use of operator `{}`", op.as_str()))
                        .with_code("V0302")
                        .with_label(span, "several visible operators match");
                for (e, p0, p1, ret) in &explicit {
                    let _ = (p0, p1);
                    let _ = ret;
                    d = d.with_note(format!("candidate: {}", self.a.describe_subprogram(*e)));
                }
                for c in &implicit {
                    d = d.with_note(format!(
                        "candidate: predefined \"{}\"({}, {}) return {}",
                        op.as_str(),
                        self.ty_name(c.operands[0]),
                        self.ty_name(c.operands[1]),
                        self.ty_name(c.result)
                    ));
                }
                self.push(d);
            }
            return self.a.builtins.error;
        }
        if let Some((d, p0, p1, ret)) = explicit.first().copied() {
            self.resolve(lhs, p0);
            self.resolve(rhs, p1);
            self.a.set_call(span, CallTarget::Subprogram(d));
            self.a.set_ref(span, d);
            self.fold_call(span, d, &[lhs.span(), rhs.span()], ret);
            return ret;
        }
        let c = implicit[0].clone();
        self.commit_binary(op, lhs, rhs, span, &c)
    }

    fn commit_binary(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        chosen: &overload::OpCand,
    ) -> TypeId {
        let (operands, result, target) = (
            chosen.operands.as_slice(),
            chosen.result,
            chosen.target.clone(),
        );
        let lt = self.resolve(lhs, operands[0]);
        let rt = self.resolve(rhs, operands[1]);
        self.a.set_call(span, target);
        // Arrays on both sides of a logical operator must be the same
        // length (clause 9.2.2); check when both are static.
        if op.is_logical()
            && self.a.class(lt) == TypeClass::Array
            && self.a.class(rt) == TypeClass::Array
            && let (Some(a), Some(b)) = (self.a.array_length(lt), self.a.array_length(rt))
            && a != b
        {
            self.push(
                Diagnostic::error(format!(
                    "operands of `{}` have different lengths: {a} and {b}",
                    op.as_str()
                ))
                .with_code("V0307")
                .with_label(lhs.span(), format!("{a} elements"))
                .with_secondary(rhs.span(), format!("{b} elements")),
            );
        }
        self.fold_binary(op, span, lhs.span(), rhs.span(), operands, result);
        result
    }

    fn resolve_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
        expected: TypeId,
        mode: Mode,
    ) -> TypeId {
        let o = self.infer(operand);
        let mut explicit: Vec<(DeclId, TypeId, TypeId)> = Vec::new();
        for d in self.operator_decls(op.as_str(), 1) {
            let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind else {
                continue;
            };
            let p0 = sig.params[0].ty;
            let Some(ret) = sig.ret else { continue };
            if overload::compatible(self.a, Cand::Ty(ret), expected)
                && overload::any_compatible(self.a, &o, p0)
            {
                explicit.push((d, p0, ret));
            }
        }
        let implicit: Vec<_> = overload::unary_ops(self.a, op, &o, self.standard)
            .into_iter()
            .filter(|c| overload::compatible(self.a, Cand::Ty(c.result), expected))
            .filter(|c| {
                !explicit.iter().any(|(_, p0, ret)| {
                    self.a.same_base(*p0, c.operands[0]) && self.a.same_base(*ret, c.result)
                })
            })
            .collect();
        match explicit.len() + implicit.len() {
            0 => {
                if mode == Mode::Commit {
                    self.report_no_operator(op.as_str(), span, &[(&o, operand.span())], expected);
                }
                self.a.builtins.error
            }
            1 => {
                if let Some((d, p0, ret)) = explicit.first().copied() {
                    self.resolve(operand, p0);
                    self.a.set_call(span, CallTarget::Subprogram(d));
                    self.a.set_ref(span, d);
                    self.fold_call(span, d, &[operand.span()], ret);
                    ret
                } else {
                    let c = &implicit[0];
                    let (ops, result, target) = (c.operands.clone(), c.result, c.target.clone());
                    self.resolve(operand, ops[0]);
                    self.a.set_call(span, target);
                    self.fold_unary(op, span, operand.span(), ops[0], result);
                    result
                }
            }
            _ => {
                if mode == Mode::Commit {
                    let mut d =
                        Diagnostic::error(format!("ambiguous use of operator `{}`", op.as_str()))
                            .with_code("V0302")
                            .with_label(span, "several visible operators match");
                    for (e, _, _) in &explicit {
                        d = d.with_note(format!("candidate: {}", self.a.describe_subprogram(*e)));
                    }
                    for c in &implicit {
                        d = d.with_note(format!(
                            "candidate: predefined \"{}\"({}) return {}",
                            op.as_str(),
                            self.ty_name(c.operands[0]),
                            self.ty_name(c.result)
                        ));
                    }
                    self.push(d);
                }
                self.a.builtins.error
            }
        }
    }

    fn report_no_operator(
        &mut self,
        op: &str,
        span: Span,
        operands: &[(&Vec<Cand>, Span)],
        expected: TypeId,
    ) {
        let describe = |c: &Vec<Cand>, s: &Checker| -> String {
            let names: Vec<String> = c
                .iter()
                .map(|c| match c {
                    Cand::Ty(t) => s.ty_name(*t),
                    Cand::StringLit => "string literal".into(),
                    Cand::Aggregate => "aggregate".into(),
                    Cand::Null => "null".into(),
                    Cand::Error => "<error>".into(),
                })
                .collect();
            if names.is_empty() {
                "<unknown>".into()
            } else {
                names.join("` or `")
            }
        };
        if operands
            .iter()
            .any(|(c, _)| c.iter().any(|c| matches!(c, Cand::Error)))
        {
            return;
        }
        let en = self.ty_name(expected);
        let mut d = Diagnostic::error(format!("no visible operator `{op}` for these operands"))
            .with_code("V0303")
            .with_label(span, format!("expected `{en}` here"));
        for (c, s) in operands {
            let t = describe(c, self);
            d = d.with_secondary(*s, format!("this operand is `{t}`"));
        }
        if operands.len() == 2
            && operands
                .iter()
                .any(|(c, _)| c.iter().any(|c| matches!(c, Cand::StringLit)))
        {
            d = d.with_note("a string literal takes its type from the context; qualify it with `type'(\"...\")`");
        }
        // The classic missing `use ieee.numeric_std.all`.
        if matches!(op, "+" | "-" | "*" | "/" | "<" | ">" | "<=" | ">=")
            && operands.iter().any(|(c, _)| {
                c.iter()
                    .any(|c| matches!(c, Cand::Ty(t) if self.a.is_std_ulogic_array(*t)))
            })
        {
            d = d.with_note(
                "arithmetic on `std_logic_vector` needs `use ieee.numeric_std.all` and a cast to `unsigned` or `signed`",
            );
        }
        self.push(d);
    }

    // --- static folding ----------------------------------------------------

    fn fold_unary(&mut self, op: UnaryOp, span: Span, operand: Span, oty: TypeId, result: TypeId) {
        let Some(v) = self.a.value_of(operand).cloned() else {
            return;
        };
        let out = match op {
            UnaryOp::Plus => Some(v),
            UnaryOp::Minus => match v {
                Value::Int(i) => i.checked_neg().map(Value::Int),
                Value::Real(r) => Some(Value::Real(-r)),
                _ => None,
            },
            UnaryOp::Abs => match v {
                Value::Int(i) => i.checked_abs().map(Value::Int),
                Value::Real(r) => Some(Value::Real(r.abs())),
                _ => None,
            },
            UnaryOp::Not => self.logical_not(&v, oty),
            UnaryOp::Condition => v.as_bool().map(Value::from_bool),
            UnaryOp::And
            | UnaryOp::Or
            | UnaryOp::Nand
            | UnaryOp::Nor
            | UnaryOp::Xor
            | UnaryOp::Xnor => self.reduce(&v, op),
        };
        let _ = result;
        if let Some(out) = out {
            self.a.set_value(span, out);
        }
    }

    /// `not` on a bit/boolean scalar or array.
    fn logical_not(&self, v: &Value, ty: TypeId) -> Option<Value> {
        let invert = |p: u32| -> u32 { u32::from(p == 0) };
        match v {
            Value::Enum(p) if self.a.is_bit_or_boolean(ty) => Some(Value::Enum(invert(*p))),
            Value::Array(a) => {
                let elems: Option<Vec<Value>> = a
                    .elems
                    .iter()
                    .map(|e| e.as_enum().map(|p| Value::Enum(invert(p))))
                    .collect();
                Some(Value::Array(ArrayValue {
                    left: a.left,
                    dir: a.dir,
                    elems: elems?,
                }))
            }
            _ => None,
        }
    }

    /// A VHDL-2008 reduction operator on a bit/boolean array.
    fn reduce(&self, v: &Value, op: UnaryOp) -> Option<Value> {
        let a = v.as_array()?;
        let bits: Option<Vec<bool>> = a.elems.iter().map(|e| e.as_bool()).collect();
        let bits = bits?;
        let r = match op {
            UnaryOp::And => bits.iter().all(|&b| b),
            UnaryOp::Nand => !bits.iter().all(|&b| b),
            UnaryOp::Or => bits.iter().any(|&b| b),
            UnaryOp::Nor => !bits.iter().any(|&b| b),
            UnaryOp::Xor => bits.iter().filter(|&&b| b).count() % 2 == 1,
            UnaryOp::Xnor => bits.iter().filter(|&&b| b).count() % 2 == 0,
            _ => return None,
        };
        Some(Value::from_bool(r))
    }

    fn fold_binary(
        &mut self,
        op: BinaryOp,
        span: Span,
        lspan: Span,
        rspan: Span,
        operands: &[TypeId],
        result: TypeId,
    ) {
        let (Some(l), Some(r)) = (
            self.a.value_of(lspan).cloned(),
            self.a.value_of(rspan).cloned(),
        ) else {
            return;
        };
        let out = match op {
            BinaryOp::Eq => l.compare(&r).map(|o| Value::from_bool(o.is_eq())),
            BinaryOp::Neq => l.compare(&r).map(|o| Value::from_bool(o.is_ne())),
            BinaryOp::Lt => l.compare(&r).map(|o| Value::from_bool(o.is_lt())),
            BinaryOp::Le => l.compare(&r).map(|o| Value::from_bool(o.is_le())),
            BinaryOp::Gt => l.compare(&r).map(|o| Value::from_bool(o.is_gt())),
            BinaryOp::Ge => l.compare(&r).map(|o| Value::from_bool(o.is_ge())),
            BinaryOp::Add => arith(&l, &r, i128::checked_add, |a, b| a + b),
            BinaryOp::Sub => arith(&l, &r, i128::checked_sub, |a, b| a - b),
            BinaryOp::Mul => self.fold_mul(&l, &r, operands),
            BinaryOp::Div => self.fold_div(&l, &r, span),
            BinaryOp::Mod => match (l.as_int(), r.as_int()) {
                (Some(a), Some(b)) => match int::modulo(a, b) {
                    Some(v) => Some(Value::Int(v)),
                    None => {
                        self.error("V0306", span, "division by zero in a static expression");
                        None
                    }
                },
                _ => None,
            },
            BinaryOp::Rem => match (l.as_int(), r.as_int()) {
                (Some(a), Some(b)) => match int::remainder(a, b) {
                    Some(v) => Some(Value::Int(v)),
                    None => {
                        self.error("V0306", span, "division by zero in a static expression");
                        None
                    }
                },
                _ => None,
            },
            BinaryOp::Pow => match (&l, r.as_int()) {
                (Value::Int(a), Some(n)) => int::power(*a, n).map(Value::Int),
                (Value::Real(a), Some(n)) => i32::try_from(n).ok().map(|n| Value::Real(a.powi(n))),
                _ => None,
            },
            BinaryOp::Concat => self.fold_concat(&l, &r, operands, result),
            BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Nand
            | BinaryOp::Nor
            | BinaryOp::Xor
            | BinaryOp::Xnor => self.fold_logical(op, &l, &r, operands),
            BinaryOp::Sll
            | BinaryOp::Srl
            | BinaryOp::Sla
            | BinaryOp::Sra
            | BinaryOp::Rol
            | BinaryOp::Ror => self.fold_shift(op, &l, &r),
            _ => None,
        };
        if let Some(out) = out {
            self.a.set_value(span, out);
        }
    }

    fn fold_mul(&mut self, l: &Value, r: &Value, operands: &[TypeId]) -> Option<Value> {
        // Physical / integer mixing keeps the physical amount.
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => a.checked_mul(*b).map(Value::Int),
            (Value::Real(a), Value::Real(b)) => Some(Value::Real(a * b)),
            (Value::Int(a), Value::Real(b)) | (Value::Real(b), Value::Int(a)) => {
                let _ = operands;
                Some(Value::Real(*a as f64 * b))
            }
            _ => None,
        }
    }

    fn fold_div(&mut self, l: &Value, r: &Value, span: Span) -> Option<Value> {
        match (l, r) {
            (Value::Int(_), Value::Int(0)) => {
                self.error("V0306", span, "division by zero in a static expression");
                None
            }
            (Value::Int(a), Value::Int(b)) => a.checked_div(*b).map(Value::Int),
            (Value::Real(a), Value::Real(b)) => Some(Value::Real(a / b)),
            (Value::Real(a), Value::Int(b)) => Some(Value::Real(a / *b as f64)),
            _ => None,
        }
    }

    fn fold_concat(
        &self,
        l: &Value,
        r: &Value,
        operands: &[TypeId],
        result: TypeId,
    ) -> Option<Value> {
        let as_elems = |v: &Value, ty: TypeId| -> Option<Vec<Value>> {
            if self.a.class(ty) == TypeClass::Array {
                Some(v.as_array()?.elems.clone())
            } else {
                Some(vec![v.clone()])
            }
        };
        let mut elems = as_elems(l, operands[0])?;
        elems.extend(as_elems(r, operands[1])?);
        let (left, dir) = match self
            .a
            .index_constraint(result)
            .and_then(|c| c.first().cloned())
        {
            Some(b) => (b.left.int().unwrap_or(0), b.dir),
            None => {
                // The result takes the index range of the left operand
                // when it is an array (clause 9.2.5), else `t'left`.
                match l.as_array() {
                    Some(a) => (a.left, a.dir),
                    None => (0, Direction::To),
                }
            }
        };
        Some(Value::Array(ArrayValue { left, dir, elems }))
    }

    fn fold_logical(
        &self,
        op: BinaryOp,
        l: &Value,
        r: &Value,
        operands: &[TypeId],
    ) -> Option<Value> {
        let apply = |a: bool, b: bool| -> bool {
            match op {
                BinaryOp::And => a && b,
                BinaryOp::Or => a || b,
                BinaryOp::Nand => !(a && b),
                BinaryOp::Nor => !(a || b),
                BinaryOp::Xor => a != b,
                BinaryOp::Xnor => a == b,
                _ => false,
            }
        };
        match (l, r) {
            (Value::Enum(a), Value::Enum(b)) => Some(Value::from_bool(apply(*a != 0, *b != 0))),
            (Value::Array(a), Value::Array(b)) if a.elems.len() == b.elems.len() => {
                let elems: Option<Vec<Value>> = a
                    .elems
                    .iter()
                    .zip(&b.elems)
                    .map(|(x, y)| Some(Value::from_bool(apply(x.as_bool()?, y.as_bool()?))))
                    .collect();
                Some(Value::Array(ArrayValue {
                    left: a.left,
                    dir: a.dir,
                    elems: elems?,
                }))
            }
            (Value::Array(a), Value::Enum(s)) => {
                let _ = operands;
                let elems: Option<Vec<Value>> = a
                    .elems
                    .iter()
                    .map(|x| Some(Value::from_bool(apply(x.as_bool()?, *s != 0))))
                    .collect();
                Some(Value::Array(ArrayValue {
                    left: a.left,
                    dir: a.dir,
                    elems: elems?,
                }))
            }
            (Value::Enum(s), Value::Array(b)) => {
                let elems: Option<Vec<Value>> = b
                    .elems
                    .iter()
                    .map(|x| Some(Value::from_bool(apply(*s != 0, x.as_bool()?))))
                    .collect();
                Some(Value::Array(ArrayValue {
                    left: b.left,
                    dir: b.dir,
                    elems: elems?,
                }))
            }
            _ => None,
        }
    }

    fn fold_shift(&self, op: BinaryOp, l: &Value, r: &Value) -> Option<Value> {
        let a = l.as_array()?;
        let n = r.as_int()?;
        let len = i128::try_from(a.elems.len()).ok()?;
        if len == 0 {
            return Some(l.clone());
        }
        let fill = Value::Enum(0);
        let mut elems = Vec::with_capacity(a.elems.len());
        for i in 0..len {
            let src = match op {
                BinaryOp::Sll | BinaryOp::Sla => i + n,
                BinaryOp::Srl | BinaryOp::Sra => i - n,
                BinaryOp::Rol => (i + n).rem_euclid(len),
                BinaryOp::Ror => (i - n).rem_euclid(len),
                _ => return None,
            };
            let v = if (0..len).contains(&src) {
                a.elems[usize::try_from(src).ok()?].clone()
            } else {
                match op {
                    BinaryOp::Sla => a.elems[a.elems.len() - 1].clone(),
                    BinaryOp::Sra => a.elems[0].clone(),
                    _ => fill.clone(),
                }
            };
            elems.push(v);
        }
        Some(Value::Array(ArrayValue {
            left: a.left,
            dir: a.dir,
            elems,
        }))
    }

    // --- type marks, subtypes and ranges -----------------------------------

    /// Resolves a type mark, reporting unknown or non-type names.
    pub(crate) fn resolve_type_mark(&mut self, n: &Name) -> TypeId {
        match self.classify(n, Mode::Commit, None) {
            Prefix::Type(t) => {
                self.a.set_type(n.span(), t);
                t
            }
            Prefix::Error => self.a.builtins.error,
            _ => {
                let text = self.text(n.span()).to_owned();
                let mut d = Diagnostic::error(format!("`{text}` is not a type"))
                    .with_code("V0206")
                    .with_span(n.span());
                if let Name::Simple(i) = n {
                    let sym = self.ident_sym(i);
                    let pkgs = scope::packages_declaring(self.a, sym);
                    if let Some(p) = pkgs.first() {
                        d = d.with_note(format!("`{text}` is declared in `{p}`"));
                    }
                }
                self.push(d);
                self.a.builtins.error
            }
        }
    }

    /// A type mark without diagnostics, for inference.
    fn resolve_type_mark_quiet(&mut self, n: &Name) -> TypeId {
        match self.classify(n, Mode::Infer, None) {
            Prefix::Type(t) => t,
            _ => self.a.builtins.error,
        }
    }

    /// Resolves a subtype indication, building an anonymous subtype when
    /// it carries a constraint or a resolution function.
    pub(crate) fn resolve_subtype_indication(&mut self, si: &ast::SubtypeIndication) -> TypeId {
        let base = self.resolve_type_mark(&si.type_mark);
        let resolution = si
            .resolution
            .as_ref()
            .and_then(|r| self.resolve_resolution(r, base));
        let constraint = si
            .constraint
            .as_ref()
            .and_then(|c| self.resolve_constraint(c, base));
        if resolution.is_none() && constraint.is_none() {
            return base;
        }
        let ty = self.a.add_type(
            TypeKind::Subtype {
                parent: base,
                constraint,
                resolution,
            },
            None,
        );
        self.a.set_type(si.span, ty);
        ty
    }

    fn resolve_resolution(
        &mut self,
        r: &ast::ResolutionIndication,
        base: TypeId,
    ) -> Option<DeclId> {
        match r {
            ast::ResolutionIndication::Function(n) => {
                match self.classify(n, Mode::Commit, None) {
                    Prefix::Overloaded(ds) => {
                        // Pick the one-parameter function whose parameter
                        // is an array of the resolved element type.
                        let found = ds.iter().copied().find(|&d| {
                            matches!(&self.a.decl(d).kind, DeclKind::Subprogram { sig, .. }
                                if sig.params.len() == 1 && sig.ret.is_some_and(|r| overload::types_compatible(self.a, r, base)))
                        }).or_else(|| ds.first().copied());
                        if let Some(d) = found {
                            self.a.set_ref(n.span(), d);
                        }
                        found
                    }
                    Prefix::Error => None,
                    _ => {
                        self.error(
                            "V0206",
                            n.span(),
                            "a resolution indication needs a function name",
                        );
                        None
                    }
                }
            }
            ast::ResolutionIndication::Array(inner, _) => {
                let elem = self.a.element_type(base).unwrap_or(base);
                self.resolve_resolution(inner, elem)
            }
            ast::ResolutionIndication::Record(entries, _) => {
                for e in entries {
                    let sym = self.ident_sym(&e.name);
                    let fty = self.a.field_type(base, sym).unwrap_or(base);
                    self.resolve_resolution(&e.resolution, fty);
                }
                None
            }
        }
    }

    fn resolve_constraint(&mut self, c: &ast::Constraint, base: TypeId) -> Option<Constraint> {
        match c {
            ast::Constraint::Range(r) => {
                if !self.a.is_scalar(base) && !self.a.is_error(base) {
                    let bn = self.ty_name(base);
                    self.error(
                        "V0304",
                        r.span(),
                        format!("`{bn}` is not a scalar type, so it takes an index constraint, not a range"),
                    );
                    return None;
                }
                let info = self.resolve_range(r, Some(base));
                Some(Constraint::Range(info.bounds))
            }
            ast::Constraint::Array {
                indices,
                element,
                span,
            } => {
                let elem_base = self.a.element_type(base);
                if indices.is_empty() {
                    // `(open)` + element constraint (VHDL-2008).
                    self.require_2008(*span, "`(open)` constraints");
                    let e = element.as_ref().and_then(|ec| {
                        let eb = elem_base?;
                        let c = self.resolve_constraint(ec, eb)?;
                        Some(self.anon_subtype(eb, c))
                    })?;
                    return Some(Constraint::Element(e));
                }
                if self.a.class(base) != TypeClass::Array && !self.a.is_error(base) {
                    let bn = self.ty_name(base);
                    self.error(
                        "V0304",
                        *span,
                        format!("`{bn}` is not an array type, so it takes no index constraint"),
                    );
                    return None;
                }
                let dims = self.a.dimensions(base);
                if dims != 0 && indices.len() != dims {
                    let bn = self.ty_name(base);
                    self.error(
                        "V0304",
                        *span,
                        format!(
                            "`{bn}` has {dims} dimension{}, but {} index constraint{} given",
                            if dims == 1 { "" } else { "s" },
                            indices.len(),
                            if indices.len() == 1 { " is" } else { "s are" }
                        ),
                    );
                }
                let index_types: Vec<TypeId> = self
                    .a
                    .array_info(base)
                    .map(|(i, _)| i.to_vec())
                    .unwrap_or_default();
                let mut bounds = Vec::new();
                for (i, dr) in indices.iter().enumerate() {
                    let want = index_types.get(i).copied();
                    let info = self.resolve_discrete_range_in(dr, want);
                    bounds.push(info.bounds);
                }
                let elem = element.as_ref().and_then(|ec| {
                    let eb = elem_base?;
                    let c = self.resolve_constraint(ec, eb)?;
                    Some(self.anon_subtype(eb, c))
                });
                Some(Constraint::Index(bounds, elem))
            }
            ast::Constraint::Record(entries, span) => {
                self.require_2008(*span, "record constraints");
                let mut out = Vec::new();
                for e in entries {
                    let sym = self.ident_sym(&e.name);
                    let Some(fty) = self.a.field_type(base, sym) else {
                        let bn = self.ty_name(base);
                        self.error(
                            "V0202",
                            e.name.span,
                            format!("`{bn}` has no element `{}`", e.name.name),
                        );
                        continue;
                    };
                    if let Some(c) = self.resolve_constraint(&e.constraint, fty) {
                        let t = self.anon_subtype(fty, c);
                        out.push((sym, t));
                    }
                }
                Some(Constraint::Record(out))
            }
        }
    }

    /// An anonymous subtype of `parent` with `constraint`.
    fn anon_subtype(&mut self, parent: TypeId, constraint: Constraint) -> TypeId {
        self.a.add_type(
            TypeKind::Subtype {
                parent,
                constraint: Some(constraint),
                resolution: None,
            },
            None,
        )
    }

    /// Resolves a range, with the index or scalar type it must have.
    pub(crate) fn resolve_range(&mut self, r: &ast::Range, want: Option<TypeId>) -> RangeInfo {
        match r {
            ast::Range::Bounds {
                left,
                direction,
                right,
                span,
            } => {
                let ty = match want {
                    Some(t) => t,
                    None => {
                        // Both bounds must agree; infer from either.
                        let l = self.infer(left);
                        let r2 = self.infer(right);
                        let mut both: Vec<TypeId> = Vec::new();
                        for c in l.iter().chain(&r2) {
                            if let Cand::Ty(t) = c
                                && overload::any_compatible(self.a, &l, *t)
                                && overload::any_compatible(self.a, &r2, *t)
                                && !both.iter().any(|&o| self.a.same_base(o, *t))
                            {
                                both.push(*t);
                            }
                        }
                        // A universal interpretation defers to a concrete one.
                        let concrete: Vec<TypeId> = both
                            .iter()
                            .copied()
                            .filter(|&t| {
                                !matches!(
                                    self.a.class(t),
                                    TypeClass::UniversalInteger | TypeClass::UniversalReal
                                )
                            })
                            .collect();
                        let pick = concrete.first().or(both.first()).copied();
                        match pick {
                            Some(t) => match self.a.class(t) {
                                TypeClass::UniversalInteger => self.a.builtins.integer,
                                TypeClass::UniversalReal => self.a.builtins.real,
                                _ => t,
                            },
                            None => {
                                self.error(
                                    "V0304",
                                    *span,
                                    "the bounds of this range have no common type",
                                );
                                self.a.builtins.error
                            }
                        }
                    }
                };
                let lt = self.resolve(left, ty);
                self.resolve(right, ty);
                let bounds = Bounds {
                    left: self.bound_of(left),
                    dir: *direction,
                    right: self.bound_of(right),
                };
                let ty = if self.a.is_error(ty) { lt } else { ty };
                let info = RangeInfo { ty, bounds };
                self.a.set_range(*span, info.clone());
                info
            }
            ast::Range::Attribute(n) => {
                let info = self.range_attribute(n, want);
                self.a.set_range(n.span(), info.clone());
                info
            }
        }
    }

    /// `x'range` / `x'reverse_range[(dim)]` used as a range.
    fn range_attribute(&mut self, n: &Name, want: Option<TypeId>) -> RangeInfo {
        let err = RangeInfo {
            ty: want.unwrap_or(self.a.builtins.integer),
            bounds: Bounds {
                left: Bound::Dynamic(n.span()),
                dir: Direction::To,
                right: Bound::Dynamic(n.span()),
            },
        };
        // With a dimension argument the parser gives Call(Attribute, args).
        let (attr_name, dim) = match n {
            Name::Call { prefix, args, .. } => {
                let d = args
                    .first()
                    .and_then(|a| match &a.actual {
                        Actual::Expr(e) => self.static_dimension(e),
                        _ => None,
                    })
                    .unwrap_or(1);
                (prefix.as_ref(), d)
            }
            other => (other, 1),
        };
        let Name::Attribute {
            prefix, attribute, ..
        } = attr_name
        else {
            self.error("V0304", n.span(), "expected a range or a range attribute");
            return err;
        };
        let reverse = attribute.name.eq_ignore_ascii_case("reverse_range");
        let ty = match self.classify(prefix, Mode::Commit, None) {
            Prefix::Type(t) => t,
            Prefix::Object(_, t) | Prefix::Value(t) => t,
            Prefix::Error => return err,
            _ => {
                self.error(
                    "V0206",
                    prefix.span(),
                    "the prefix of `'range` must be a type or an array object",
                );
                return err;
            }
        };
        self.a.set_type(n.span(), ty);
        let idx = if self.a.class(ty) == TypeClass::Array {
            attrs::array_attr_type(self.a, ty, Predefined::Left, dim)
                .unwrap_or(self.a.builtins.integer)
        } else {
            ty
        };
        match attrs::range_of(self.a, ty, dim, reverse) {
            Some(bounds) => RangeInfo { ty: idx, bounds },
            None => RangeInfo {
                ty: idx,
                bounds: Bounds {
                    left: Bound::Dynamic(n.span()),
                    dir: Direction::To,
                    right: Bound::Dynamic(n.span()),
                },
            },
        }
    }

    fn static_dimension(&mut self, e: &Expr) -> Option<usize> {
        let ui = self.a.builtins.universal_integer;
        self.resolve(e, ui);
        let v = self.a.value_of(e.span())?.as_int()?;
        usize::try_from(v).ok()
    }

    /// Resolves a discrete range with no expected index type.
    pub(crate) fn resolve_discrete_range(&mut self, dr: &DiscreteRange) -> RangeInfo {
        self.resolve_discrete_range_in(dr, None)
    }

    /// Resolves a discrete range whose index type should be `want`.
    pub(crate) fn resolve_discrete_range_in(
        &mut self,
        dr: &DiscreteRange,
        want: Option<TypeId>,
    ) -> RangeInfo {
        match dr {
            DiscreteRange::Range(r) => {
                let info = self.resolve_range(r, want);
                if !self.a.is_discrete(info.ty)
                    && !self.a.is_error(info.ty)
                    && self.a.class(info.ty) != TypeClass::Physical
                {
                    let tn = self.ty_name(info.ty);
                    self.error(
                        "V0304",
                        r.span(),
                        format!("`{tn}` is not a discrete type, so it cannot index an array or drive a loop"),
                    );
                }
                info
            }
            DiscreteRange::Subtype(si) => {
                let ty = self.resolve_subtype_indication(si);
                let bounds = self.a.scalar_range(ty).unwrap_or(Bounds {
                    left: Bound::Dynamic(si.span),
                    dir: Direction::To,
                    right: Bound::Dynamic(si.span),
                });
                if !self.a.is_discrete(ty) && !self.a.is_error(ty) {
                    let tn = self.ty_name(ty);
                    self.error("V0304", si.span, format!("`{tn}` is not a discrete type"));
                }
                let info = RangeInfo { ty, bounds };
                self.a.set_range(si.span, info.clone());
                info
            }
        }
    }

    fn check_compatible(&mut self, span: Span, got: TypeId, expected: TypeId, mode: Mode) {
        if mode != Mode::Commit
            || overload::types_compatible(self.a, got, expected)
            || self.a.is_error(got)
            || self.a.is_error(expected)
        {
            return;
        }
        let (g, e) = (self.ty_name(got), self.ty_name(expected));
        self.push(
            Diagnostic::error(format!("type mismatch: expected `{e}`, found `{g}`"))
                .with_code("V0300")
                .with_label(span, format!("this is `{g}`"))
                .with_note(format!("expected `{e}`")),
        );
    }
}

/// Widens a physical scale factor to a real for a real physical
/// literal. Scales come from unit declarations and are far below 2^53,
/// so the conversion is exact.
fn int_as_f64(n: i128) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        n as f64
    }
}

/// Arithmetic on two static values of the same class.
fn arith(
    l: &Value,
    r: &Value,
    int_op: fn(i128, i128) -> Option<i128>,
    real_op: fn(f64, f64) -> f64,
) -> Option<Value> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => int_op(*a, *b).map(Value::Int),
        (Value::Real(a), Value::Real(b)) => Some(Value::Real(real_op(*a, *b))),
        _ => None,
    }
}
