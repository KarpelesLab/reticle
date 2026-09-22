//! Overload resolution (IEEE 1076-2008 clause 12.5) and the implicitly
//! declared operators (clause 9.2).
//!
//! # Candidate types
//!
//! The first phase of expression typing ([`super::expr`]) computes, for
//! every expression, the set of types it could have: a [`Cand`] each. Most
//! are concrete arena types; the literals whose type depends entirely on
//! context are represented by wildcards ([`Cand::StringLit`] matches every
//! one-dimensional character array, [`Cand::Aggregate`] every composite,
//! [`Cand::Null`] every access type), and the two universal types are
//! ordinary [`Cand::Ty`] entries that [`compatible`] treats as implicitly
//! convertible to every integer or floating type (clause 9.3.6, implicit
//! conversion of universal expressions).
//!
//! # Subprogram calls
//!
//! [`match_signature`] pairs the actuals of a call with the formals of one
//! candidate (positional, then named; defaults for the rest) and checks
//! that each actual's candidate set contains something compatible with the
//! formal's type. The checker runs it over every visible declaration with
//! the call's designator, then keeps those whose result is compatible with
//! the context; one survivor is the resolution, none is a "no matching
//! overload" and several an ambiguity, each reported with the candidate
//! profiles. An explicit declaration beats an implicit operator with the
//! same profile (clause 12.3: the implicit one is hidden).
//!
//! # Implicit operators
//!
//! Rather than materialising the thousands of implicit `"+"`, `"="`, ...
//! declarations the standard describes for every type, [`binary_ops`] and
//! [`unary_ops`] compute the implicit interpretations of an operator
//! structurally from the operand candidate sets: equality on any type,
//! ordering on scalars and discrete arrays, arithmetic on numeric types
//! with the physical-type mixing rules, logical and shift operators on
//! `bit`/`boolean` and their arrays, concatenation on one-dimensional
//! arrays, and the VHDL-2008 additions (reductions, mixed scalar/array
//! logic, matching operators, `??`). Each interpretation is an [`OpCand`]
//! naming the result type and the operand types, so the second phase can
//! resolve the operands against exactly the types the chosen
//! interpretation needs.

use crate::intern::Symbol;
use crate::source::Span;
use crate::vhdl::Standard;
use crate::vhdl::ast::{BinaryOp, UnaryOp};

use super::types::{TypeClass, TypeId};
use super::{Analysis, CallTarget, DeclId, Signature};

/// A candidate type of an expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cand {
    /// A concrete type or subtype (the universal types included).
    Ty(TypeId),
    /// A string or bit-string literal: any one-dimensional character
    /// array type.
    StringLit,
    /// An aggregate: any composite type.
    Aggregate,
    /// `null`: any access type.
    Null,
    /// An erroneous expression: compatible with everything.
    Error,
}

/// True when `cand` can be resolved to `expected`.
pub fn compatible(a: &Analysis, cand: Cand, expected: TypeId) -> bool {
    if a.is_error(expected) {
        return true;
    }
    match cand {
        Cand::Error => true,
        Cand::Ty(t) => types_compatible(a, t, expected),
        Cand::StringLit => a.is_string_type(expected),
        Cand::Aggregate => a.class(expected).is_composite(),
        Cand::Null => a.class(expected) == TypeClass::Access,
    }
}

/// True when a value of type `t` may appear where `expected` is required:
/// same base type, or a universal value in an integer/real context (or
/// the reverse, when the context is itself universal).
pub fn types_compatible(a: &Analysis, t: TypeId, expected: TypeId) -> bool {
    if a.is_error(t) || a.is_error(expected) || a.same_base(t, expected) {
        return true;
    }
    let ct = a.class(t);
    let ce = a.class(expected);
    match (ct, ce) {
        (TypeClass::UniversalInteger, c) | (c, TypeClass::UniversalInteger) => c.is_integer(),
        (TypeClass::UniversalReal, c) | (c, TypeClass::UniversalReal) => c.is_real(),
        _ => false,
    }
}

/// True when a `Cand::Ty` is exactly `expected` (same base) rather than
/// merely universal-convertible; used to prefer exact interpretations.
pub fn exact(a: &Analysis, cand: Cand, expected: TypeId) -> bool {
    matches!(cand, Cand::Ty(t) if a.same_base(t, expected))
}

/// True when the candidate set contains something compatible with `t`.
pub fn any_compatible(a: &Analysis, cands: &[Cand], t: TypeId) -> bool {
    cands.iter().any(|&c| compatible(a, c, t))
}

/// Removes duplicate candidates, keeping order.
pub fn dedup(a: &Analysis, cands: &mut Vec<Cand>) {
    let mut out: Vec<Cand> = Vec::new();
    for c in cands.drain(..) {
        let dup = out.iter().any(|&o| match (o, c) {
            (Cand::Ty(x), Cand::Ty(y)) => a.same_base(x, y),
            (x, y) => x == y,
        });
        if !dup {
            out.push(c);
        }
    }
    *cands = out;
}

/// One actual of a call, as seen by overload resolution.
#[derive(Clone, Debug)]
pub struct ArgInfo {
    /// The formal name for a named association.
    pub formal: Option<Symbol>,
    /// The candidate types of the actual, or empty for `open`.
    pub cands: Vec<Cand>,
    /// The actual's span.
    pub span: Span,
    /// True for `open`.
    pub open: bool,
}

/// Why a candidate does not match a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// More actuals than formals.
    TooMany,
    /// A named actual with no such formal.
    UnknownFormal(Symbol),
    /// A formal associated twice.
    Duplicate(usize),
    /// A formal without default left unassociated.
    Missing(DeclId),
    /// The actual at index `.0` is not compatible with the formal `.1`.
    Type(usize, usize),
    /// A positional actual after a named one.
    PositionalAfterNamed(usize),
}

/// Pairs actuals with the formals of `sig`. On success returns, for each
/// formal, the index of the actual associated with it (`None` when the
/// default applies).
pub fn match_signature(
    a: &Analysis,
    sig: &Signature,
    args: &[ArgInfo],
) -> Result<Vec<Option<usize>>, Mismatch> {
    let mut assoc: Vec<Option<usize>> = vec![None; sig.params.len()];
    let mut named = false;
    for (i, arg) in args.iter().enumerate() {
        match arg.formal {
            None => {
                if named {
                    return Err(Mismatch::PositionalAfterNamed(i));
                }
                if i >= sig.params.len() {
                    return Err(Mismatch::TooMany);
                }
                assoc[i] = Some(i);
            }
            Some(f) => {
                named = true;
                let Some(p) = sig.params.iter().position(|p| a.decl(p.decl).name == f) else {
                    return Err(Mismatch::UnknownFormal(f));
                };
                if assoc[p].is_some() {
                    return Err(Mismatch::Duplicate(p));
                }
                assoc[p] = Some(i);
            }
        }
    }
    for (p, param) in sig.params.iter().enumerate() {
        match assoc[p] {
            Some(i) => {
                let arg = &args[i];
                if arg.open {
                    continue;
                }
                if !any_compatible(a, &arg.cands, param.ty) {
                    return Err(Mismatch::Type(i, p));
                }
            }
            None => {
                if !param.has_default {
                    return Err(Mismatch::Missing(param.decl));
                }
            }
        }
    }
    Ok(assoc)
}

/// One interpretation of an operator or call.
#[derive(Clone, Debug)]
pub struct OpCand {
    /// The result type.
    pub result: TypeId,
    /// The operand types, in order.
    pub operands: Vec<TypeId>,
    /// What the interpretation resolves to.
    pub target: CallTarget,
}

/// The concrete types of a candidate set (wildcards dropped).
fn concrete(cands: &[Cand]) -> impl Iterator<Item = TypeId> + '_ {
    cands.iter().filter_map(|c| match c {
        Cand::Ty(t) => Some(*t),
        _ => None,
    })
}

/// Pairs each concrete type on one side with the compatible types on the
/// other, so that a wildcard or universal on one side takes the type of
/// the other. Yields `(left, right)` operand types where both have the
/// same base.
fn same_type_pairs(a: &Analysis, l: &[Cand], r: &[Cand]) -> Vec<TypeId> {
    let mut out: Vec<TypeId> = Vec::new();
    let mut push = |t: TypeId| {
        if !out.iter().any(|&o| a.same_base(o, t)) {
            out.push(t);
        }
    };
    for lt in concrete(l) {
        if any_compatible(a, r, lt) {
            push(lt);
        }
    }
    for rt in concrete(r) {
        if any_compatible(a, l, rt) {
            push(rt);
        }
    }
    // A universal interpretation is shadowed by a concrete type of its
    // class: `x + 1` is an `integer` addition, not a universal one.
    let concrete_types: Vec<TypeId> = out
        .iter()
        .copied()
        .filter(|&t| !is_universal(a, t))
        .collect();
    out.retain(|&t| {
        !is_universal(a, t) || !concrete_types.iter().any(|&c| types_compatible(a, t, c))
    });
    out
}

fn is_universal(a: &Analysis, t: TypeId) -> bool {
    matches!(
        a.class(t),
        TypeClass::UniversalInteger | TypeClass::UniversalReal
    )
}

/// Whether `op` is one of the six logical operators.
fn logical(op: BinaryOp) -> bool {
    op.is_logical()
}

/// The implicit interpretations of a binary operator on operand
/// candidate sets, given the type the context expects when known.
pub fn binary_ops(
    a: &Analysis,
    op: BinaryOp,
    l: &[Cand],
    r: &[Cand],
    expected: Option<TypeId>,
    standard: Standard,
) -> Vec<OpCand> {
    let b = &a.builtins;
    let mut out: Vec<OpCand> = Vec::new();
    let sym = op.as_str();
    let mut push = |result: TypeId, operands: Vec<TypeId>| {
        if !out
            .iter()
            .any(|o| a.same_base(o.result, result) && same_operands(a, &o.operands, &operands))
        {
            out.push(OpCand {
                result,
                operands,
                target: CallTarget::Operator(sym),
            });
        }
    };
    let v2008 = standard >= Standard::Vhdl2008;
    match op {
        BinaryOp::Eq | BinaryOp::Neq => {
            for t in same_type_pairs(a, l, r) {
                let c = a.class(t);
                if !matches!(c, TypeClass::File | TypeClass::Protected | TypeClass::Other) {
                    push(b.boolean, vec![t, t]);
                }
            }
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            for t in same_type_pairs(a, l, r) {
                let c = a.class(t);
                let ok = c.is_scalar()
                    || (c == TypeClass::Array
                        && a.dimensions(t) == 1
                        && a.element_type(t).is_some_and(|e| a.is_discrete(e)));
                if ok {
                    push(b.boolean, vec![t, t]);
                }
            }
        }
        BinaryOp::MatchEq
        | BinaryOp::MatchNeq
        | BinaryOp::MatchLt
        | BinaryOp::MatchLe
        | BinaryOp::MatchGt
        | BinaryOp::MatchGe => {
            if v2008 {
                for t in same_type_pairs(a, l, r) {
                    if a.is_bit_or_boolean(t) {
                        push(t, vec![t, t]);
                    } else if matches!(op, BinaryOp::MatchEq | BinaryOp::MatchNeq)
                        && a.is_bit_or_boolean_array(t)
                    {
                        push(a.element_type(t).unwrap_or(t), vec![t, t]);
                    }
                }
            }
        }
        _ if logical(op) => {
            for t in same_type_pairs(a, l, r) {
                if a.is_bit_or_boolean(t) || a.is_bit_or_boolean_array(t) {
                    push(t, vec![t, t]);
                }
            }
            if v2008 {
                // Mixed scalar/array forms (clause 9.2.2).
                for lt in concrete(l) {
                    if a.is_bit_or_boolean_array(lt) {
                        let e = a.element_type(lt).unwrap_or(lt);
                        if any_compatible(a, r, e) {
                            push(lt, vec![lt, e]);
                        }
                    }
                }
                for rt in concrete(r) {
                    if a.is_bit_or_boolean_array(rt) {
                        let e = a.element_type(rt).unwrap_or(rt);
                        if any_compatible(a, l, e) {
                            push(rt, vec![e, rt]);
                        }
                    }
                }
            }
        }
        BinaryOp::Sll
        | BinaryOp::Srl
        | BinaryOp::Sla
        | BinaryOp::Sra
        | BinaryOp::Rol
        | BinaryOp::Ror => {
            for lt in concrete(l) {
                if a.is_bit_or_boolean_array(lt) && any_compatible(a, r, b.integer) {
                    push(lt, vec![lt, b.integer]);
                }
            }
        }
        BinaryOp::Add | BinaryOp::Sub => {
            for t in same_type_pairs(a, l, r) {
                if a.class(t).is_numeric() {
                    push(t, vec![t, t]);
                }
            }
        }
        BinaryOp::Mul | BinaryOp::Div => {
            for t in same_type_pairs(a, l, r) {
                let c = a.class(t);
                if c.is_integer() || c.is_real() {
                    push(t, vec![t, t]);
                }
            }
            // Physical mixing: phys * int, phys * real, phys / int,
            // phys / real, int * phys, real * phys, phys / phys.
            for lt in concrete(l) {
                if a.class(lt) == TypeClass::Physical {
                    if any_compatible(a, r, b.integer) {
                        push(lt, vec![lt, b.integer]);
                    }
                    if any_compatible(a, r, b.real) {
                        push(lt, vec![lt, b.real]);
                    }
                    if op == BinaryOp::Div && any_compatible(a, r, lt) {
                        push(b.universal_integer, vec![lt, lt]);
                    }
                }
            }
            if op == BinaryOp::Mul {
                for rt in concrete(r) {
                    if a.class(rt) == TypeClass::Physical {
                        if any_compatible(a, l, b.integer) {
                            push(rt, vec![b.integer, rt]);
                        }
                        if any_compatible(a, l, b.real) {
                            push(rt, vec![b.real, rt]);
                        }
                    }
                }
            }
            // universal_integer * universal_real and the reverse.
            if any_compatible(a, l, b.universal_integer)
                && concrete(r).any(|t| a.class(t) == TypeClass::UniversalReal)
            {
                push(
                    b.universal_real,
                    vec![b.universal_integer, b.universal_real],
                );
            }
            if any_compatible(a, r, b.universal_integer)
                && concrete(l).any(|t| a.class(t) == TypeClass::UniversalReal)
            {
                push(
                    b.universal_real,
                    vec![b.universal_real, b.universal_integer],
                );
            }
        }
        BinaryOp::Mod | BinaryOp::Rem => {
            for t in same_type_pairs(a, l, r) {
                if a.class(t).is_integer() {
                    push(t, vec![t, t]);
                }
            }
        }
        BinaryOp::Pow => {
            for lt in concrete(l) {
                let c = a.class(lt);
                if (c.is_integer() || c.is_real()) && any_compatible(a, r, b.integer) {
                    push(lt, vec![lt, b.integer]);
                }
            }
        }
        BinaryOp::Concat => {
            // arr & arr, arr & elem, elem & arr, elem & elem.
            let mut arrays: Vec<TypeId> = Vec::new();
            for t in concrete(l).chain(concrete(r)) {
                if a.class(t) == TypeClass::Array
                    && a.dimensions(t) == 1
                    && !arrays.iter().any(|&x| a.same_base(x, t))
                {
                    arrays.push(t);
                }
            }
            if let Some(e) = expected
                && a.class(e) == TypeClass::Array
                && a.dimensions(e) == 1
                && !arrays.iter().any(|&x| a.same_base(x, e))
            {
                arrays.push(e);
            }
            for arr in arrays {
                let Some(elem) = a.element_type(arr) else {
                    continue;
                };
                let l_arr = any_compatible(a, l, arr);
                let l_elem = any_compatible(a, l, elem);
                let r_arr = any_compatible(a, r, arr);
                let r_elem = any_compatible(a, r, elem);
                // Prefer the array interpretation of a side that admits
                // both (a string literal against a character array).
                if l_arr && r_arr {
                    push(arr, vec![arr, arr]);
                } else if l_arr && r_elem {
                    push(arr, vec![arr, elem]);
                } else if l_elem && r_arr {
                    push(arr, vec![elem, arr]);
                } else if l_elem && r_elem {
                    push(arr, vec![elem, elem]);
                }
            }
        }
        _ => {}
    }
    out
}

fn same_operands(a: &Analysis, x: &[TypeId], y: &[TypeId]) -> bool {
    x.len() == y.len() && x.iter().zip(y).all(|(p, q)| a.same_base(*p, *q))
}

/// The implicit interpretations of a unary operator.
pub fn unary_ops(a: &Analysis, op: UnaryOp, operand: &[Cand], standard: Standard) -> Vec<OpCand> {
    let b = &a.builtins;
    let mut out: Vec<OpCand> = Vec::new();
    let sym = op.as_str();
    let v2008 = standard >= Standard::Vhdl2008;
    for t in concrete(operand) {
        let c = a.class(t);
        let push = |out: &mut Vec<OpCand>, result: TypeId| {
            if !out.iter().any(|o| a.same_base(o.operands[0], t)) {
                out.push(OpCand {
                    result,
                    operands: vec![t],
                    target: CallTarget::Operator(sym),
                });
            }
        };
        match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Abs => {
                if c.is_numeric() {
                    push(&mut out, t);
                }
            }
            UnaryOp::Not => {
                if a.is_bit_or_boolean(t) || a.is_bit_or_boolean_array(t) {
                    push(&mut out, t);
                }
            }
            UnaryOp::Condition => {
                if v2008 && a.same_base(t, b.bit) {
                    push(&mut out, b.boolean);
                }
                if a.same_base(t, b.boolean) {
                    push(&mut out, b.boolean);
                }
            }
            UnaryOp::And
            | UnaryOp::Or
            | UnaryOp::Nand
            | UnaryOp::Nor
            | UnaryOp::Xor
            | UnaryOp::Xnor => {
                if v2008 && a.is_bit_or_boolean_array(t) {
                    let e = a.element_type(t).unwrap_or(t);
                    push(&mut out, e);
                }
            }
        }
    }
    out
}

/// True for the type conversions clause 9.3.6 allows between `from` and
/// `to`: numeric types among themselves, and array types with the same
/// dimensionality whose element types are the same and whose index types
/// are convertible (closely related types).
pub fn closely_related(a: &Analysis, from: TypeId, to: TypeId) -> bool {
    if a.is_error(from) || a.is_error(to) || a.same_base(from, to) {
        return true;
    }
    let (cf, ct) = (a.class(from), a.class(to));
    if cf.is_numeric() && ct.is_numeric() && cf != TypeClass::Physical && ct != TypeClass::Physical
    {
        return true;
    }
    if cf == TypeClass::Array && ct == TypeClass::Array {
        let Some((fi, fe)) = a.array_info(from) else {
            return false;
        };
        let Some((ti, te)) = a.array_info(to) else {
            return false;
        };
        if fi.len() != ti.len() {
            return false;
        }
        if !a.same_base(fe, te) && !(a.class(fe) == TypeClass::Array && closely_related(a, fe, te))
        {
            return false;
        }
        return fi.iter().zip(ti).all(|(x, y)| {
            a.same_base(*x, *y) || (a.class(*x).is_integer() && a.class(*y).is_integer())
        });
    }
    false
}
