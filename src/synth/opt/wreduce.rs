//! Width reduction.
//!
//! [`WReduce`] narrows operators whose full width is not needed:
//!
//! - **Unused upper bits.** A `Slice` (or an unsigned truncating
//!   `Resize`) of an `add`, `sub`, `mul`, `and`, `or`, `xor`, `xnor`,
//!   `not` or `neg` that drops the top bits is pushed into the operator:
//!   the low `k` bits of these operations depend only on the low `k` bits
//!   of their operands, so the operator is rebuilt at width `k` over
//!   truncated operands. A slice of a `mux` is pushed into both branches.
//! - **Provably zero upper bits.** When both operands of an `and`, `or`,
//!   `xor`, `add`, `mul`, `eq` or `ne` are zero-extensions (or small
//!   constants), the operation is performed at the width its inputs need
//!   (`max` of the operand widths, one more for `add`, the sum for `mul`)
//!   and the result zero-extended. The same applies to the two branches
//!   of a `mux`.
//!
//! Signed operands are only narrowed by the first rule (truncation), since
//! sign extension is not a zero upper half. The follow-up `const_fold`
//! turns the inserted truncations into slices and folds the constants.

use crate::diag::Diagnostics;
use crate::ir::{BinaryOp, ExprId, ExprKind, Module, Type, UnaryOp};
use crate::source::Span;
use crate::synth::opt::rewrite_exprs;
use crate::synth::util::{coerce, mk, mk_binary, mk_mux, mk_slice, mk_unary};
use crate::synth::{Pass, PassStats};

/// The width-reduction pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct WReduce;

impl Pass for WReduce {
    fn name(&self) -> &'static str {
        "wreduce"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let mut reduced = 0u64;
        rewrite_exprs(m, |m, id| {
            let out = reduce(m, id);
            if out != id {
                reduced += 1;
            }
            out
        });
        stats.bump("operators narrowed", reduced);
        stats
    }
}

fn bits_of(m: &Module, id: ExprId) -> Option<(u32, bool)> {
    match m.expr(id).ty {
        Type::Bits { width, signed } => Some((width, signed)),
        _ => None,
    }
}

/// True for operators whose low bits depend only on the operands' low
/// bits.
fn low_bits_local(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::Xnor
    )
}

/// Truncates `e` to `width` bits, keeping its signedness flag so operator
/// signedness is unchanged.
fn truncate(m: &mut Module, e: ExprId, width: u32, span: Span) -> ExprId {
    let Some((w, signed)) = bits_of(m, e) else {
        return e;
    };
    if w == width {
        return e;
    }
    if !signed {
        return mk_slice(m, e, width - 1, 0, span);
    }
    mk(
        m,
        ExprKind::Resize {
            expr: e,
            width,
            signed: true,
        },
        span,
    )
}

/// The unsigned source of a zero-extension: the operand of a `Resize`
/// that does not sign-extend, or a constant reduced to its significant
/// bits (never fewer than one).
fn zext_source(m: &Module, id: ExprId) -> Option<(ExprId, u32)> {
    match &m.expr(id).kind {
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => {
            let (w, s) = bits_of(m, *expr)?;
            if *width > w && !(*signed && s) {
                Some((*expr, w))
            } else {
                None
            }
        }
        ExprKind::Const(c) if c.is_fully_known() && !c.is_signed() => {
            let significant = (0..c.width())
                .rev()
                .find(|i| c.bit(*i) == crate::logic::Bit::One)
                .map_or(1, |i| i + 1);
            if significant < c.width() {
                Some((id, significant))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The type-preserving reduction of one node whose operands are already
/// reduced.
fn reduce(m: &mut Module, id: ExprId) -> ExprId {
    let ty = m.expr(id).ty.clone();
    let span = m.expr(id).span;
    let out = reduce_inner(m, id, span);
    coerce(m, out, &ty, span)
}

fn reduce_inner(m: &mut Module, id: ExprId, span: Span) -> ExprId {
    match m.expr(id).kind.clone() {
        ExprKind::Slice { base, hi, lo } => {
            let Some((bw, _)) = bits_of(m, base) else {
                return id;
            };
            if hi + 1 >= bw {
                return id;
            }
            match narrow_to(m, base, hi + 1, span) {
                Some(n) => mk_slice(m, n, hi, lo, span),
                None => id,
            }
        }
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => {
            let Some((ew, _)) = bits_of(m, expr) else {
                return id;
            };
            if width >= ew {
                return id;
            }
            match narrow_to(m, expr, width, span) {
                Some(n) => mk(
                    m,
                    ExprKind::Resize {
                        expr: n,
                        width,
                        signed,
                    },
                    span,
                ),
                None => id,
            }
        }
        ExprKind::Binary { op, lhs, rhs } => reduce_zext_binary(m, id, op, lhs, rhs, span),
        ExprKind::Ternary { cond, then_, else_ } => {
            let (Some((a, wa)), Some((b, wb))) = (zext_source(m, then_), zext_source(m, else_))
            else {
                return id;
            };
            let Some((full, _)) = bits_of(m, id) else {
                return id;
            };
            let n = wa.max(wb);
            if n >= full {
                return id;
            }
            let a = zext(m, a, n, span);
            let b = zext(m, b, n, span);
            let mux = mk_mux(m, cond, a, b, span);
            zext(m, mux, full, span)
        }
        _ => id,
    }
}

/// Zero-extends (or truncates) `e` to `width`, unsigned.
fn zext(m: &mut Module, e: ExprId, width: u32, span: Span) -> ExprId {
    if bits_of(m, e) == Some((width, false)) {
        return e;
    }
    if let Some(c) = m.expr(e).as_const() {
        let v = c.clone().as_unsigned().resize(width);
        return crate::synth::util::mk_const(m, v, span);
    }
    mk(
        m,
        ExprKind::Resize {
            expr: e,
            width,
            signed: false,
        },
        span,
    )
}

/// Rebuilds `e` at `width` bits (narrower than its own) when its low bits
/// are computable from truncated operands.
fn narrow_to(m: &mut Module, e: ExprId, width: u32, span: Span) -> Option<ExprId> {
    match m.expr(e).kind.clone() {
        ExprKind::Binary { op, lhs, rhs } if low_bits_local(op) => {
            let a = truncate(m, lhs, width, span);
            let b = truncate(m, rhs, width, span);
            Some(mk_binary(m, op, a, b, span))
        }
        ExprKind::Unary {
            op: op @ (UnaryOp::Not | UnaryOp::Neg),
            expr,
        } => {
            let a = truncate(m, expr, width, span);
            Some(mk_unary(m, op, a, span))
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            let a = truncate(m, then_, width, span);
            let b = truncate(m, else_, width, span);
            Some(mk_mux(m, cond, a, b, span))
        }
        ExprKind::Const(c) => {
            let v = c.resize(width);
            Some(crate::synth::util::mk_const(m, v, span))
        }
        _ => None,
    }
}

fn reduce_zext_binary(
    m: &mut Module,
    id: ExprId,
    op: BinaryOp,
    lhs: ExprId,
    rhs: ExprId,
    span: Span,
) -> ExprId {
    let Some((full, signed)) = bits_of(m, lhs) else {
        return id;
    };
    if signed || bits_of(m, rhs).is_some_and(|(_, s)| s) {
        return id;
    }
    let (Some((a, wa)), Some((b, wb))) = (zext_source(m, lhs), zext_source(m, rhs)) else {
        return id;
    };
    let n = match op {
        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::Eq | BinaryOp::Ne => wa.max(wb),
        BinaryOp::Add => wa.max(wb) + 1,
        BinaryOp::Mul => wa + wb,
        _ => return id,
    };
    if n >= full {
        return id;
    }
    let a = zext(m, a, n, span);
    let b = zext(m, b, n, span);
    let inner = mk_binary(m, op, a, b, span);
    if op.is_predicate() {
        inner
    } else {
        zext(m, inner, full, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::synth::opt::ConstFold;
    use crate::synth::opt::testutil::{run, run_fix, span};

    #[test]
    fn slice_of_add_narrows_the_adder() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(16));
        let c = b.input("c", Type::bits(16));
        let y = b.output("y", Type::bits(4));
        let (an, cn) = (b.net(a), b.net(c));
        let sum = b.add(an, cn);
        let sl = b.slice(sum, 3, 0);
        b.assign(y, sl);
        let mut m = b.finish();
        let stats = run(&WReduce, &mut m);
        assert_eq!(stats.get("operators narrowed"), 1);
        run_fix(&ConstFold, &mut m);
        assert!(
            m.to_text().contains("assign %y = add(%a[3:0], %c[3:0])"),
            "{}",
            m.to_text()
        );
    }

    #[test]
    fn zero_extended_operands_shrink() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let s = b.input("s", Type::bit());
        let y = b.output("y", Type::bits(16));
        let z = b.output("z", Type::bits(16));
        let e = b.output("e", Type::bit());
        let (an, cn, sn) = (b.net(a), b.net(c), b.net(s));
        let za = b.zext(an, 16);
        let zc = b.zext(cn, 16);
        let sum = b.add(za, zc);
        b.assign(y, sum);
        let k = b.const_u64(16, 5);
        let mux = b.mux(sn, za, k);
        b.assign(z, mux);
        let eq = b.eq(za, k);
        b.assign(e, eq);
        let mut m = b.finish();
        run_fix(&WReduce, &mut m);
        run_fix(&ConstFold, &mut m);
        let t = m.to_text();
        assert!(
            t.contains("assign %y = resize(add(resize(%a, u5), resize(%c, u5)), u16)"),
            "{t}"
        );
        assert!(
            t.contains("assign %z = resize(mux(%s, %a, 4'd5), u16)"),
            "{t}"
        );
        assert!(t.contains("assign %e = eq(%a, 4'd5)"), "{t}");
    }

    #[test]
    fn signed_and_full_width_are_left_alone() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::sbits(4));
        let y = b.output("y", Type::sbits(8));
        let an = b.net(a);
        let sa = b.sext(an, 8);
        let sum = b.add(sa, sa);
        b.assign(y, sum);
        let mut m = b.finish();
        let stats = run(&WReduce, &mut m);
        assert!(!stats.changed);
    }
}
