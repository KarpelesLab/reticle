//! Constant folding and algebraic simplification.
//!
//! [`ConstFold`] does three things per run:
//!
//! 1. **Expression rewriting**, bottom-up over every reachable node:
//!    closed subtrees are evaluated with [`crate::synth::eval`]; identities
//!    are applied (`and(x, 1s)`, `or(x, 0)`, `xor(x, 0)`, `add(x, 0)`,
//!    `mul(x, 1)`, shifts by zero, `not(not(x))`, `resize` to the same
//!    type, `mux` with a constant select or equal branches); structural
//!    selections are canonicalised (constant `Index` and unsigned
//!    truncating `Resize` become `Slice`; a `Slice` of a `Slice`, of a
//!    zero-extension or of a `Concat` landing in one part is pushed
//!    down; nested `Concat`s are flattened and adjacent constants
//!    merged). Single-bit muxes over constants become `and` / `or` /
//!    `not` so enables and conditions take a canonical shape. An all-`x`
//!    branch of a mux is a don't-care and the other branch is taken.
//! 2. **Cell folding**: a combinational cell whose inputs are all
//!    constant becomes an `assign` of its value; `mux` / `pmux` with a
//!    constant select, `buf`, and bitwise / arithmetic cells with an
//!    identity operand become an `assign` of the selected input.
//! 3. **Propagation**: an `assign` of a constant or of another net to a
//!    net that is not a port, has no `keep` and no other root reference
//!    is substituted into every reader and removed. In the other
//!    direction, `assign port = %tmp` where `tmp` is an internal net
//!    driven by one cell output and read nowhere else makes the cell
//!    drive the port directly.
//!
//! Every replacement keeps the type of the node it replaces (inserting a
//! `resize` when only the signedness differs), since parents were typed
//! against the original.

use std::collections::HashMap;

use crate::diag::Diagnostics;
use crate::ir::{
    Assign, BinaryOp, Cell, CellKind, Const, ExprId, ExprKind, Lvalue, Module, NetId, Type, UnaryOp,
};
use crate::logic::Bit;
use crate::source::Span;
use crate::synth::eval::eval_closed;
use crate::synth::opt::{expr_net_refs, rewrite_exprs, root_net_refs};
use crate::synth::util::{
    coerce, is_kept, is_port, mk, mk_binary, mk_const, mk_mux, mk_not, mk_slice, mk_unary,
};
use crate::synth::{Pass, PassStats};

/// The constant-folding pass; see the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConstFold;

impl Pass for ConstFold {
    fn name(&self) -> &'static str {
        "const_fold"
    }

    fn run(&self, m: &mut Module, _diags: &mut Diagnostics) -> PassStats {
        let mut stats = PassStats::default();
        let mut folded = 0u64;
        let changed_roots = rewrite_exprs(m, |m, id| {
            let out = simplify(m, id);
            if out != id {
                folded += 1;
            }
            out
        });
        stats.bump("expressions folded", folded);
        stats.bump(
            "roots rewritten",
            u64::try_from(changed_roots).unwrap_or(u64::MAX),
        );
        stats.bump("cells folded", fold_cells(m));
        stats.bump("assigns propagated", propagate(m));
        stats.bump("cell outputs retargeted", retarget(m));
        stats
    }
}

// --- expression rules ------------------------------------------------------

fn c(m: &Module, id: ExprId) -> Option<&Const> {
    m.expr(id).as_const()
}

fn is_all_x(m: &Module, id: ExprId) -> bool {
    c(m, id).is_some_and(|c| c.width() > 0 && (0..c.width()).all(|i| c.bit(i) == Bit::X))
}

fn is_zero(m: &Module, id: ExprId) -> bool {
    c(m, id).is_some_and(|c| c.is_fully_known() && c.is_zero())
}

fn is_ones(m: &Module, id: ExprId) -> bool {
    c(m, id).is_some_and(|c| c.is_fully_known() && c.width() > 0 && c.not().is_zero())
}

fn is_one(m: &Module, id: ExprId) -> bool {
    c(m, id).is_some_and(|c| c.is_fully_known() && c.to_u64() == Some(1))
}

fn bits_of(m: &Module, id: ExprId) -> Option<(u32, bool)> {
    match m.expr(id).ty {
        Type::Bits { width, signed } => Some((width, signed)),
        _ => None,
    }
}

/// Applies the rewrite rules to a node whose operands are already
/// simplified. Always returns a node of the same type.
pub(crate) fn simplify(m: &mut Module, id: ExprId) -> ExprId {
    let ty = m.expr(id).ty.clone();
    let span = m.expr(id).span;
    let out = simplify_inner(m, id, &ty, span);
    coerce(m, out, &ty, span)
}

fn simplify_inner(m: &mut Module, id: ExprId, ty: &Type, span: Span) -> ExprId {
    let kind = m.expr(id).kind.clone();
    // Closed subtrees fold to their value.
    if !matches!(
        kind,
        ExprKind::Const(_) | ExprKind::String(_) | ExprKind::Call { .. }
    ) && crate::ir::expr::operands(&kind)
        .iter()
        .all(|o| c(m, *o).is_some())
        && let Some(v) = eval_closed(m, id)
    {
        return mk_const(m, v, span);
    }
    match kind {
        ExprKind::Slice { base, hi, lo } => simplify_slice(m, id, base, hi, lo, span),
        ExprKind::Index { base, index } => {
            let Some(i) = c(m, index).and_then(Const::to_u64) else {
                return id;
            };
            let Some((w, _)) = bits_of(m, base) else {
                return id;
            };
            if i >= u64::from(w) {
                return mk_const(m, Const::x(1), span);
            }
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            let s = mk_slice(m, base, i, i, span);
            simplify(m, s)
        }
        ExprKind::IndexedSlice {
            base,
            offset,
            width,
            up,
        } => {
            let Some(off) = c(m, offset).and_then(Const::to_u64) else {
                return id;
            };
            let Some((w, _)) = bits_of(m, base) else {
                return id;
            };
            let lo = if up {
                off
            } else {
                match off.checked_sub(u64::from(width) - 1) {
                    Some(lo) => lo,
                    None => return id,
                }
            };
            let hi = lo + u64::from(width) - 1;
            if hi >= u64::from(w) {
                return id;
            }
            let s = mk_slice(
                m,
                base,
                u32::try_from(hi).unwrap_or(u32::MAX),
                u32::try_from(lo).unwrap_or(u32::MAX),
                span,
            );
            simplify(m, s)
        }
        ExprKind::Concat(parts) => simplify_concat(m, id, &parts, span),
        ExprKind::Replicate { count, expr } => {
            if count == 1 && !m.expr(expr).ty.is_signed() {
                return expr;
            }
            id
        }
        ExprKind::Unary { op, expr } => simplify_unary(m, id, op, expr, span),
        ExprKind::Binary { op, lhs, rhs } => simplify_binary(m, id, op, lhs, rhs, ty, span),
        ExprKind::Ternary { cond, then_, else_ } => {
            simplify_ternary(m, id, cond, then_, else_, ty, span)
        }
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => simplify_resize(m, id, expr, width, signed, span),
        _ => id,
    }
}

fn simplify_slice(
    m: &mut Module,
    id: ExprId,
    base: ExprId,
    hi: u32,
    lo: u32,
    span: Span,
) -> ExprId {
    let Some((bw, bs)) = bits_of(m, base) else {
        return id;
    };
    if lo == 0 && hi + 1 == bw && !bs {
        return base;
    }
    match m.expr(base).kind.clone() {
        ExprKind::Slice {
            base: b2, lo: lo2, ..
        } => {
            let s = mk_slice(m, b2, lo2 + hi, lo2 + lo, span);
            simplify(m, s)
        }
        ExprKind::Concat(parts) => {
            // Find the part holding [hi:lo] entirely.
            let mut top = bw;
            for part in parts {
                let pw = bits_of(m, part).map_or(0, |(w, _)| w);
                let bottom = top - pw;
                if lo >= bottom && hi < top {
                    let s = mk_slice(m, part, hi - bottom, lo - bottom, span);
                    return simplify(m, s);
                }
                top = bottom;
            }
            id
        }
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => {
            let Some((ew, es)) = bits_of(m, expr) else {
                return id;
            };
            if hi < ew {
                let s = mk_slice(m, expr, hi, lo, span);
                return simplify(m, s);
            }
            if width > ew && !(signed && es) {
                if lo >= ew {
                    return mk_const(m, Const::zero(hi - lo + 1), span);
                }
                // Straddling the extension: the low part of the operand,
                // zero-extended to the slice width.
                let s = mk_slice(m, expr, ew - 1, lo, span);
                let s = simplify(m, s);
                return mk(
                    m,
                    ExprKind::Resize {
                        expr: s,
                        width: hi - lo + 1,
                        signed: false,
                    },
                    span,
                );
            }
            id
        }
        ExprKind::Replicate { expr, .. } => {
            let Some((ew, _)) = bits_of(m, expr) else {
                return id;
            };
            if ew > 0 && hi / ew == lo / ew {
                let s = mk_slice(m, expr, hi % ew, lo % ew, span);
                return simplify(m, s);
            }
            id
        }
        _ => id,
    }
}

fn simplify_concat(m: &mut Module, id: ExprId, parts: &[ExprId], span: Span) -> ExprId {
    let mut flat: Vec<ExprId> = Vec::with_capacity(parts.len());
    for p in parts {
        match m.expr(*p).kind.clone() {
            ExprKind::Concat(inner) => flat.extend(inner),
            _ => {
                if bits_of(m, *p).is_some_and(|(w, _)| w == 0) {
                    continue;
                }
                flat.push(*p);
            }
        }
    }
    // Merge adjacent constants and adjacent slices of one base.
    let mut merged: Vec<ExprId> = Vec::with_capacity(flat.len());
    for p in flat {
        if let Some(last) = merged.last().copied() {
            if let (Some(a), Some(b)) = (c(m, last), c(m, p)) {
                let v = a.concat(b);
                merged.pop();
                merged.push(mk_const(m, v, span));
                continue;
            }
            if let (
                ExprKind::Slice {
                    base: b1,
                    hi: h1,
                    lo: l1,
                },
                ExprKind::Slice {
                    base: b2,
                    hi: h2,
                    lo: l2,
                },
            ) = (m.expr(last).kind.clone(), m.expr(p).kind.clone())
                && b1 == b2
                && l1 == h2 + 1
            {
                merged.pop();
                let s = mk_slice(m, b1, h1, l2, span);
                let s = simplify(m, s);
                merged.push(s);
                continue;
            }
        }
        merged.push(p);
    }
    if merged.len() == 1 && !m.expr(merged[0]).ty.is_signed() {
        return merged[0];
    }
    if merged == parts {
        return id;
    }
    mk(m, ExprKind::Concat(merged), span)
}

fn simplify_unary(m: &mut Module, id: ExprId, op: UnaryOp, expr: ExprId, span: Span) -> ExprId {
    let inner = m.expr(expr).kind.clone();
    match (op, inner) {
        (
            UnaryOp::Not,
            ExprKind::Unary {
                op: UnaryOp::Not,
                expr: x,
            },
        )
        | (
            UnaryOp::Neg,
            ExprKind::Unary {
                op: UnaryOp::Neg,
                expr: x,
            },
        ) => x,
        (
            UnaryOp::LogicNot,
            ExprKind::Unary {
                op: UnaryOp::LogicNot,
                expr: x,
            },
        ) if m.expr(x).ty.is_bit() => x,
        (UnaryOp::LogicNot, _) if m.expr(expr).ty.is_bit() => mk_not(m, expr, span),
        (UnaryOp::ReduceAnd | UnaryOp::ReduceOr | UnaryOp::ReduceXor, _)
            if m.expr(expr).ty.is_bit() =>
        {
            expr
        }
        (UnaryOp::ReduceNand | UnaryOp::ReduceNor | UnaryOp::ReduceXnor, _)
            if m.expr(expr).ty.is_bit() =>
        {
            mk_not(m, expr, span)
        }
        _ => id,
    }
}

fn simplify_binary(
    m: &mut Module,
    id: ExprId,
    op: BinaryOp,
    lhs: ExprId,
    rhs: ExprId,
    ty: &Type,
    span: Span,
) -> ExprId {
    let (Some((w, _)), Some(_)) = (bits_of(m, lhs), bits_of(m, rhs)) else {
        return id;
    };
    let zero = |m: &mut Module| mk_const(m, Const::zero(w), span);
    match op {
        BinaryOp::And | BinaryOp::LogicAnd => {
            if is_zero(m, lhs) || is_zero(m, rhs) {
                return zero(m);
            }
            if is_ones(m, rhs) || lhs == rhs {
                return lhs;
            }
            if is_ones(m, lhs) {
                return rhs;
            }
        }
        BinaryOp::Or | BinaryOp::LogicOr => {
            if is_ones(m, lhs) || is_ones(m, rhs) {
                return mk_const(m, Const::ones(w), span);
            }
            if is_zero(m, rhs) || lhs == rhs {
                return lhs;
            }
            if is_zero(m, lhs) {
                return rhs;
            }
        }
        BinaryOp::Xor => {
            if is_zero(m, rhs) {
                return lhs;
            }
            if is_zero(m, lhs) {
                return rhs;
            }
            if lhs == rhs {
                return zero(m);
            }
            if is_ones(m, rhs) {
                return mk_unary(m, UnaryOp::Not, lhs, span);
            }
            if is_ones(m, lhs) {
                return mk_unary(m, UnaryOp::Not, rhs, span);
            }
        }
        BinaryOp::Add => {
            if is_zero(m, rhs) {
                return lhs;
            }
            if is_zero(m, lhs) {
                return rhs;
            }
        }
        BinaryOp::Sub => {
            if is_zero(m, rhs) {
                return lhs;
            }
            if lhs == rhs {
                return zero(m);
            }
        }
        BinaryOp::Mul => {
            if is_zero(m, lhs) || is_zero(m, rhs) {
                return zero(m);
            }
            if is_one(m, rhs) {
                return lhs;
            }
            if is_one(m, lhs) {
                return rhs;
            }
        }
        BinaryOp::Div => {
            if is_one(m, rhs) {
                return lhs;
            }
        }
        BinaryOp::Mod => {
            if is_one(m, rhs) {
                return zero(m);
            }
        }
        BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Sshr => {
            if is_zero(m, rhs) {
                return lhs;
            }
            if op != BinaryOp::Sshr
                && let Some(n) = c(m, rhs).and_then(Const::to_u64)
                && n >= u64::from(w)
            {
                return zero(m);
            }
        }
        BinaryOp::Eq | BinaryOp::CaseEq => {
            if lhs == rhs {
                return mk_const(m, Const::from_bool(true), span);
            }
            if w == 1 {
                if is_one(m, rhs) {
                    return lhs;
                }
                if is_zero(m, rhs) {
                    return mk_not(m, lhs, span);
                }
                if is_one(m, lhs) {
                    return rhs;
                }
                if is_zero(m, lhs) {
                    return mk_not(m, rhs, span);
                }
            }
            if let (Some(a), Some(b)) = (c(m, lhs), c(m, rhs))
                && a.is_fully_known()
                && b.is_fully_known()
            {
                let v = a.eq(b);
                return mk_const(m, v, span);
            }
        }
        BinaryOp::Ne | BinaryOp::CaseNe => {
            if lhs == rhs {
                return mk_const(m, Const::from_bool(false), span);
            }
            if w == 1 {
                if is_zero(m, rhs) {
                    return lhs;
                }
                if is_one(m, rhs) {
                    return mk_not(m, lhs, span);
                }
            }
        }
        BinaryOp::WildEq => {
            if let Some(p) = c(m, rhs) {
                if p.is_fully_known() {
                    return mk_binary(m, BinaryOp::Eq, lhs, rhs, span);
                }
                if (0..p.width()).all(|i| !p.bit(i).is_known()) {
                    return mk_const(m, Const::from_bool(true), span);
                }
            }
        }
        BinaryOp::Lt | BinaryOp::Gt => {
            if lhs == rhs {
                return mk_const(m, Const::from_bool(false), span);
            }
        }
        BinaryOp::Le | BinaryOp::Ge => {
            if lhs == rhs {
                return mk_const(m, Const::from_bool(true), span);
            }
        }
        BinaryOp::Xnor | BinaryOp::Pow => {}
    }
    let _ = ty;
    id
}

fn simplify_ternary(
    m: &mut Module,
    id: ExprId,
    cond: ExprId,
    then_: ExprId,
    else_: ExprId,
    ty: &Type,
    span: Span,
) -> ExprId {
    if then_ == else_ {
        return then_;
    }
    match c(m, cond).map(Const::truth) {
        Some(Bit::One) => return then_,
        Some(Bit::Zero) => return else_,
        _ => {}
    }
    if is_all_x(m, then_) {
        return else_;
    }
    if is_all_x(m, else_) {
        return then_;
    }
    // Canonical polarity: `mux(not(c), a, b)` is `mux(c, b, a)`.
    if let ExprKind::Unary {
        op: UnaryOp::Not | UnaryOp::LogicNot,
        expr: inner,
    } = m.expr(cond).kind
        && m.expr(inner).ty.is_bit()
    {
        let t = mk_mux(m, inner, else_, then_, span);
        return simplify(m, t);
    }
    // Nested muxes on the same condition.
    if let ExprKind::Ternary {
        cond: c2,
        then_: t2,
        ..
    } = m.expr(then_).kind
        && c2 == cond
    {
        let t = mk_mux(m, cond, t2, else_, span);
        return simplify(m, t);
    }
    if let ExprKind::Ternary {
        cond: c2,
        else_: e2,
        ..
    } = m.expr(else_).kind
        && c2 == cond
    {
        let t = mk_mux(m, cond, then_, e2, span);
        return simplify(m, t);
    }
    // Single-bit muxes over constants are gates.
    if ty.is_bit() {
        let (t1, t0) = (is_one(m, then_), is_zero(m, then_));
        let (e1, e0) = (is_one(m, else_), is_zero(m, else_));
        if t1 && e0 {
            return cond;
        }
        if t0 && e1 {
            return mk_not(m, cond, span);
        }
        if e0 {
            return mk_binary(m, BinaryOp::And, cond, then_, span);
        }
        if t1 {
            return mk_binary(m, BinaryOp::Or, cond, else_, span);
        }
        if t0 {
            let nc = mk_not(m, cond, span);
            return mk_binary(m, BinaryOp::And, nc, else_, span);
        }
        if e1 {
            let nc = mk_not(m, cond, span);
            return mk_binary(m, BinaryOp::Or, nc, then_, span);
        }
    }
    id
}

fn simplify_resize(
    m: &mut Module,
    id: ExprId,
    expr: ExprId,
    width: u32,
    signed: bool,
    span: Span,
) -> ExprId {
    let Some((ew, es)) = bits_of(m, expr) else {
        return id;
    };
    if ew == width && es == signed {
        return expr;
    }
    // Unsigned truncation is a slice.
    if width < ew && !signed {
        let s = mk_slice(m, expr, width - 1, 0, span);
        return simplify(m, s);
    }
    if width == 0 {
        return id;
    }
    // Resize chains.
    if let ExprKind::Resize {
        expr: inner,
        width: w1,
        signed: s1,
    } = m.expr(expr).kind
        && let Some((iw, _)) = bits_of(m, inner)
    {
        let truncates = width <= iw;
        let same_extension = s1 == signed && w1 > iw && width >= w1;
        if truncates || same_extension {
            let r = mk(
                m,
                ExprKind::Resize {
                    expr: inner,
                    width,
                    signed,
                },
                span,
            );
            return simplify(m, r);
        }
    }
    id
}

// --- cells --------------------------------------------------------------------

/// What a cell's output reduces to.
enum CellFold {
    /// An existing expression (one of the inputs).
    Expr(ExprId),
    /// A constant value.
    Const(Const),
    /// A slice of an existing expression.
    Slice(ExprId, u32, u32),
}

/// Folds cells with constant or identity inputs into assigns; returns the
/// number of cells replaced.
fn fold_cells(m: &mut Module) -> u64 {
    let mut decisions: Vec<(crate::ir::CellId, CellFold)> = Vec::new();
    for (id, cell) in m.cells.iter() {
        if is_kept(&cell.attrs) || !cell.kind.is_combinational() {
            continue;
        }
        if let Some(fold) = cell_value(m, cell) {
            decisions.push((id, fold));
        }
    }
    if decisions.is_empty() {
        return 0;
    }
    let mut doomed = Vec::new();
    for (id, fold) in decisions {
        let cell = &m.cells[id];
        let Some(y) = cell.outputs.first().map(|(_, n)| *n) else {
            continue;
        };
        let (span, attrs) = (cell.span, cell.attrs.clone());
        let target_ty = m.nets[y].ty.clone();
        let value = match fold {
            CellFold::Expr(e) => e,
            CellFold::Const(v) => mk_const(m, v.with_signed(target_ty.is_signed()), span),
            CellFold::Slice(base, hi, lo) => mk_slice(m, base, hi, lo, span),
        };
        if m.expr(value).ty.width() != target_ty.width() {
            continue;
        }
        m.assigns.push(Assign {
            target: Lvalue::Net(y),
            value,
            delay: None,
            attrs,
            span,
        });
        doomed.push(id);
    }
    m.cells.retain(|id, _| !doomed.contains(&id));
    u64::try_from(doomed.len()).unwrap_or(u64::MAX)
}

/// The value a combinational cell's output reduces to, if any.
fn cell_value(m: &Module, cell: &Cell) -> Option<CellFold> {
    let input = |p: &str| cell.input(p);
    let a = input("a");
    let b = input("b");
    let y = cell.output("y")?;
    let width = m.nets[y].ty.width()?;
    let all_const = cell.inputs.iter().all(|(_, e)| c(m, *e).is_some());
    if all_const {
        return eval_cell(m, cell).map(CellFold::Const);
    }
    let pick = |e: ExprId| Some(CellFold::Expr(e));
    let zero = || Some(CellFold::Const(Const::zero(width)));
    match &cell.kind {
        CellKind::Buf => pick(a?),
        CellKind::Mux => {
            let s = input("s")?;
            let (a, b) = (a?, b?);
            if a == b {
                return pick(a);
            }
            match c(m, s).map(Const::truth) {
                Some(Bit::One) => pick(b),
                Some(Bit::Zero) => pick(a),
                _ => None,
            }
        }
        CellKind::Pmux => {
            let s = input("s")?;
            let sel = c(m, s)?;
            if !sel.is_fully_known() {
                return None;
            }
            let (a, b) = (a?, b?);
            let set: Vec<u32> = (0..sel.width())
                .filter(|i| sel.bit(*i) == Bit::One)
                .collect();
            match set.as_slice() {
                [] => pick(a),
                [i] => Some(CellFold::Slice(b, (i + 1) * width - 1, i * width)),
                _ => None,
            }
        }
        CellKind::And => {
            let (a, b) = (a?, b?);
            if is_ones(m, b) || a == b {
                pick(a)
            } else if is_ones(m, a) {
                pick(b)
            } else if is_zero(m, a) || is_zero(m, b) {
                zero()
            } else {
                None
            }
        }
        CellKind::Or => {
            let (a, b) = (a?, b?);
            if is_zero(m, b) || a == b {
                pick(a)
            } else if is_zero(m, a) {
                pick(b)
            } else if is_ones(m, a) || is_ones(m, b) {
                Some(CellFold::Const(Const::ones(width)))
            } else {
                None
            }
        }
        CellKind::Xor | CellKind::Add => {
            let (a, b) = (a?, b?);
            if is_zero(m, b) {
                pick(a)
            } else if is_zero(m, a) {
                pick(b)
            } else {
                None
            }
        }
        CellKind::Sub => {
            let (a, b) = (a?, b?);
            if is_zero(m, b) { pick(a) } else { None }
        }
        CellKind::Mul => {
            let (a, b) = (a?, b?);
            if is_one(m, b) {
                pick(a)
            } else if is_one(m, a) {
                pick(b)
            } else if is_zero(m, a) || is_zero(m, b) {
                zero()
            } else {
                None
            }
        }
        CellKind::Shl | CellKind::Shr | CellKind::Sshr => {
            let (a, b) = (a?, b?);
            if is_zero(m, b) { pick(a) } else { None }
        }
        _ => None,
    }
}

/// Evaluates a cell whose inputs are all constants.
fn eval_cell(m: &Module, cell: &Cell) -> Option<Const> {
    let val = |p: &str| -> Option<Const> { eval_closed(m, cell.input(p)?) };
    Some(match &cell.kind {
        CellKind::Not => val("a")?.not(),
        CellKind::Buf => val("a")?,
        CellKind::And => val("a")?.and(&val("b")?),
        CellKind::Or => val("a")?.or(&val("b")?),
        CellKind::Xor => val("a")?.xor(&val("b")?),
        CellKind::Add => val("a")?.add(&val("b")?),
        CellKind::Sub => val("a")?.sub(&val("b")?),
        CellKind::Mul => val("a")?.mul(&val("b")?),
        CellKind::Div => val("a")?.div(&val("b")?),
        CellKind::Mod => val("a")?.rem(&val("b")?),
        CellKind::Shl => val("a")?.shl_by(&val("b")?),
        CellKind::Shr => val("a")?.shr_by(&val("b")?),
        CellKind::Sshr => val("a")?.sshr_by(&val("b")?),
        CellKind::Eq => val("a")?.eq(&val("b")?),
        CellKind::Ne => val("a")?.ne(&val("b")?),
        CellKind::Lt => val("a")?.lt(&val("b")?),
        CellKind::Le => val("a")?.le(&val("b")?),
        CellKind::Gt => val("a")?.gt(&val("b")?),
        CellKind::Ge => val("a")?.ge(&val("b")?),
        CellKind::ReduceAnd => val("a")?.reduce_and(),
        CellKind::ReduceOr => val("a")?.reduce_or(),
        CellKind::ReduceXor => val("a")?.reduce_xor(),
        CellKind::Mux => {
            let (a, b, s) = (val("a")?, val("b")?, val("s")?);
            match s.truth() {
                Bit::One => b,
                Bit::Zero => a,
                _ => crate::synth::eval::merge_unknown(&a, &b),
            }
        }
        CellKind::Pmux => {
            let (a, b, s) = (val("a")?, val("b")?, val("s")?);
            let w = a.width();
            let set: Vec<u32> = (0..s.width()).filter(|i| s.bit(*i) == Bit::One).collect();
            match set.as_slice() {
                [] if s.is_fully_known() => a,
                [i] if s.is_fully_known() => b.slice((i + 1) * w - 1, i * w),
                _ => Const::x(w),
            }
        }
        CellKind::Lut { init, .. } => {
            let a = val("a")?;
            match a.to_u64() {
                Some(i) if i < u64::from(init.width()) => {
                    Const::from_bit(init.bit(u32::try_from(i).ok()?))
                }
                _ => Const::x(1),
            }
        }
        CellKind::Tristate => {
            let (a, en) = (val("a")?, val("en")?);
            match en.truth() {
                Bit::One => a,
                Bit::Zero => Const::z(a.width()),
                _ => Const::x(a.width()),
            }
        }
        _ => return None,
    })
}

// --- propagation --------------------------------------------------------------

/// Substitutes constant and alias assigns into their readers.
fn propagate(m: &mut Module) -> u64 {
    let root_refs = root_net_refs(m);
    // Candidate: assign %n = const | %other, where n is internal.
    let mut alias: HashMap<NetId, ExprId> = HashMap::new();
    let mut assign_index: HashMap<NetId, usize> = HashMap::new();
    for (i, a) in m.assigns.iter().enumerate() {
        let Lvalue::Net(n) = a.target else {
            continue;
        };
        if is_port(m, n) || is_kept(&m.nets[n].attrs) || is_kept(&a.attrs) {
            continue;
        }
        // The only root reference must be this assignment itself.
        if root_refs[n.index()] != 1 {
            continue;
        }
        let simple = match m.expr(a.value).kind {
            ExprKind::Const(_) => true,
            ExprKind::Net(other) => other != n,
            _ => false,
        };
        if !simple || m.expr(a.value).ty.width() != m.nets[n].ty.width() {
            continue;
        }
        alias.insert(n, a.value);
        assign_index.insert(n, i);
    }
    if alias.is_empty() {
        return 0;
    }
    // Resolve chains, guarding against cycles.
    let resolve = |m: &Module, start: NetId| -> Option<ExprId> {
        let mut cur = *alias.get(&start)?;
        let mut hops = 0;
        loop {
            match m.expr(cur).kind {
                ExprKind::Net(next) if alias.contains_key(&next) => {
                    hops += 1;
                    if hops > alias.len() {
                        return None;
                    }
                    cur = alias[&next];
                }
                _ => return Some(cur),
            }
        }
    };
    let mut replacement: HashMap<ExprId, ExprId> = HashMap::new();
    let mut removed: Vec<usize> = Vec::new();
    let mut count = 0u64;
    let nets: Vec<NetId> = alias.keys().copied().collect();
    let mut sorted = nets;
    sorted.sort();
    for n in sorted {
        let Some(target) = resolve(m, n) else {
            continue;
        };
        let target_ty = m.expr(target).ty.clone();
        let net_ty = m.nets[n].ty.clone();
        // Every reader node `Net(n)` maps to the target, coerced to the
        // net's type so parents keep their types.
        let mut nodes = Vec::new();
        m.for_each_expr(|id, e| {
            if e.kind == ExprKind::Net(n) {
                nodes.push(id);
            }
        });
        let value = if target_ty == net_ty {
            target
        } else {
            let span = m.nets[n].span;
            coerce(m, target, &net_ty, span)
        };
        for node in nodes {
            replacement.insert(node, value);
        }
        removed.push(assign_index[&n]);
        count += 1;
    }
    m.map_exprs(|id| replacement.get(&id).copied().unwrap_or(id));
    removed.sort_unstable();
    removed.dedup();
    let mut i = 0;
    m.assigns.retain(|_| {
        let keep = !removed.contains(&i);
        i += 1;
        keep
    });
    count
}

/// Makes cells drive ports directly when an internal net only relays a
/// cell output to a port (or kept net) through an alias assign.
fn retarget(m: &mut Module) -> u64 {
    let expr_refs = expr_net_refs(m);
    let root_refs = root_net_refs(m);
    let mut cell_of: HashMap<NetId, (crate::ir::CellId, usize)> = HashMap::new();
    for (id, cell) in m.cells.iter() {
        for (i, (_, n)) in cell.outputs.iter().enumerate() {
            cell_of.insert(*n, (id, i));
        }
    }
    let mut removed: Vec<usize> = Vec::new();
    let mut edits: Vec<(crate::ir::CellId, usize, NetId)> = Vec::new();
    let mut taken: Vec<NetId> = Vec::new();
    for (i, a) in m.assigns.iter().enumerate() {
        let Lvalue::Net(target) = a.target else {
            continue;
        };
        let ExprKind::Net(src) = m.expr(a.value).kind else {
            continue;
        };
        if src == target || is_port(m, src) || is_kept(&m.nets[src].attrs) {
            continue;
        }
        if !m.nets[src].attrs.is_empty() || m.nets[src].ty != m.nets[target].ty {
            continue;
        }
        // `src` is read only by this assign and driven only by the cell.
        if expr_refs[src.index()] != 1 || root_refs[src.index()] != 1 {
            continue;
        }
        let Some((cell, port)) = cell_of.get(&src) else {
            continue;
        };
        if taken.contains(&src) {
            continue;
        }
        taken.push(src);
        edits.push((*cell, *port, target));
        removed.push(i);
    }
    if edits.is_empty() {
        return 0;
    }
    for (cell, port, target) in &edits {
        m.cells[*cell].outputs[*port].1 = *target;
    }
    let mut i = 0;
    m.assigns.retain(|_| {
        let keep = !removed.contains(&i);
        i += 1;
        keep
    });
    u64::try_from(edits.len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::ir::{Name, Type};
    use crate::synth::opt::testutil::{run, run_fix, span};

    fn text(m: &Module) -> String {
        m.to_text()
    }

    #[test]
    fn folds_constants_and_identities() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let s = b.input("s", Type::bit());
        let y = b.output("y", Type::bits(8));
        let z = b.output("z", Type::bits(8));
        let w = b.output("w", Type::bits(8));
        let f = b.output("f", Type::bit());
        let g = b.output("g", Type::bit());
        let (an, sn) = (b.net(a), b.net(s));
        let (two, three, zero, ones, one) = (
            b.const_u64(8, 2),
            b.const_u64(8, 3),
            b.const_u64(8, 0),
            b.const_u64(8, 255),
            b.const_u64(8, 1),
        );
        // y = or(and(a, 1s), xor(k, k)) with k = 2 * 3 + 0  -> a
        let k = b.mul(two, three);
        let k0 = b.add(k, zero);
        let masked = b.and(an, ones);
        let xz = b.xor(k0, k0);
        let y_v = b.or(masked, xz);
        b.assign(y, y_v);
        // z = not(not(mul(shl(a, 0), 1)))  -> a
        let sh = b.shl(an, zero);
        let m1 = b.mul(sh, one);
        let n1 = b.not(m1);
        let n2 = b.not(n1);
        b.assign(z, n2);
        // w = resize(resize(mux(s, a, a), 16), 8) -> a
        let mx = b.mux(sn, an, an);
        let r1 = b.zext(mx, 16);
        let r2 = b.resize(r1, 8, false);
        b.assign(w, r2);
        // f = eq(and(s, 1), 1) -> s ; g = mux(s, 0, 1) -> not(s)
        let t = b.const_bit(true);
        let fl = b.const_bit(false);
        let sa = b.and(sn, t);
        let fe = b.eq(sa, t);
        b.assign(f, fe);
        let gm = b.mux(sn, fl, t);
        b.assign(g, gm);
        let mut m = b.finish();
        let stats = run_fix(&ConstFold, &mut m);
        assert!(stats.get("expressions folded") > 0);
        let t = text(&m);
        assert!(t.contains("assign %y = %a\n"), "{t}");
        assert!(t.contains("assign %z = %a\n"), "{t}");
        assert!(t.contains("assign %w = %a\n"), "{t}");
        assert!(t.contains("assign %f = %s\n"), "{t}");
        assert!(t.contains("assign %g = not(%s)\n"), "{t}");
        assert!(!run(&ConstFold, &mut m).changed);
    }

    #[test]
    fn folds_structural_selections() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bit());
        let w = b.output("w", Type::bits(8));
        let v = b.output("v", Type::bits(2));
        let (an, cn) = (b.net(a), b.net(c));
        let hi = b.slice(an, 7, 4);
        let cat = b.concat(vec![hi, cn]);
        let sl = b.slice(cat, 3, 0);
        b.assign(y, sl);
        let idx = b.const_u64(3, 6);
        let bit = b.index(an, idx);
        b.assign(z, bit);
        let lo = b.slice(an, 3, 0);
        let ext = b.zext(lo, 16);
        let back = b.slice(ext, 7, 0);
        b.assign(w, back);
        let off = b.const_u64(3, 2);
        let is = b.indexed_slice(an, off, 2, true);
        b.assign(v, is);
        let mut m = b.finish();
        run_fix(&ConstFold, &mut m);
        let t = text(&m);
        assert!(t.contains("assign %y = %c\n"), "{t}");
        assert!(t.contains("assign %z = %a[6:6]\n"), "{t}");
        assert!(t.contains("assign %w = resize(%a[3:0], u8)\n"), "{t}");
        assert!(t.contains("assign %v = %a[3:2]\n"), "{t}");
    }

    #[test]
    fn folds_cells_and_propagates() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let s = b.input("s", Type::bit());
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bits(4));
        let k = b.output("k", Type::bits(4));
        let t1 = b.add_net("t1", Type::bits(4));
        let t2 = b.add_net("t2", Type::bits(4));
        let t3 = b.add_net("t3", Type::bits(4));
        let t4 = b.add_net("t4", Type::bits(4));
        let (an, sn) = (b.net(a), b.net(s));
        let (two, three, ones, zero) = (
            b.const_u64(4, 2),
            b.const_u64(4, 3),
            b.const_u64(4, 15),
            b.const_bit(false),
        );
        // A cell with constant inputs, one with an identity input, a mux
        // with a constant select and a pmux with a constant select.
        b.cell2("c1", CellKind::Add, two, three, t1);
        b.cell2("c2", CellKind::And, an, ones, t2);
        b.cell(
            "c3",
            CellKind::Mux,
            vec![
                (Name::new("a"), an),
                (Name::new("b"), two),
                (Name::new("s"), zero),
            ],
            vec![(Name::new("y"), t3)],
        );
        let arms = b.concat(vec![two, an]);
        let sel = b.const_u64(2, 2);
        b.cell(
            "c4",
            CellKind::Pmux,
            vec![
                (Name::new("a"), an),
                (Name::new("b"), arms),
                (Name::new("s"), sel),
            ],
            vec![(Name::new("y"), t4)],
        );
        let (t1n, t2n, t3n, t4n) = (b.net(t1), b.net(t2), b.net(t3), b.net(t4));
        let sum = b.add(t1n, t2n);
        b.assign(y, sum);
        let m2 = b.mux(sn, t3n, t4n);
        b.assign(z, m2);
        b.assign(k, t1n);
        let mut m = b.finish();
        let stats = run_fix(&ConstFold, &mut m);
        assert_eq!(stats.get("cells folded"), 4);
        assert_eq!(stats.get("assigns propagated"), 4);
        let t = text(&m);
        assert!(m.cells.is_empty());
        assert!(t.contains("assign %y = add(4'd5, %a)\n"), "{t}");
        assert!(t.contains("assign %z = mux(%s, %a, 4'd2)\n"), "{t}");
        assert!(t.contains("assign %k = 4'd5\n"), "{t}");
        assert_eq!(m.assigns.len(), 3);
        run(&crate::synth::opt::Dce, &mut m);
        assert!(m.net_by_name("t1").is_none());
    }

    #[test]
    fn retargets_cell_outputs_to_ports() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let c = b.input("c", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let tmp = b.add_net("tmp", Type::bits(4));
        let (an, cn) = (b.net(a), b.net(c));
        b.cell2("x", CellKind::Xor, an, cn, tmp);
        let tn = b.net(tmp);
        b.assign(y, tn);
        let mut m = b.finish();
        let stats = run_fix(&ConstFold, &mut m);
        assert_eq!(stats.get("cell outputs retargeted"), 1);
        assert!(text(&m).contains("cell x xor (a=%a, b=%c) -> (y=%y)"));
        assert!(m.assigns.is_empty());
    }

    #[test]
    fn keeps_ports_and_kept_nets() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(4));
        let y = b.output("y", Type::bits(4));
        let z = b.output("z", Type::bits(4));
        let kept = b.add_net("kept", Type::bits(4));
        b.net_attr(kept, "keep", 1);
        let an = b.net(a);
        b.assign(y, an);
        let five = b.const_u64(4, 5);
        b.assign(kept, five);
        let kn = b.net(kept);
        b.assign(z, kn);
        let mut m = b.finish();
        let stats = run_fix(&ConstFold, &mut m);
        assert_eq!(stats.get("assigns propagated"), 0);
        assert_eq!(m.assigns.len(), 3);
    }
}
