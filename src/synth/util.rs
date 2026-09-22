//! Small helpers shared by the synthesis passes.
//!
//! Passes build expressions on an existing [`Module`] rather than through
//! [`crate::ir::builder::ModuleBuilder`] (which owns its module), so this
//! module provides typed constructors that infer the node type with the
//! rules in [`crate::ir::expr`], plus unique-name allocation for the nets
//! and cells a pass creates. Every constructor panics on an ill-typed
//! node: synthesis runs on validated input, so such a node is an internal
//! bug, not a user error.

use std::collections::HashSet;

use crate::ir::{
    Attrs, BinaryOp, Cell, CellKind, Const, Expr, ExprId, ExprKind, Module, Name, Net, NetId,
    NetKind, Type, UnaryOp, infer_type,
};
use crate::source::Span;

/// Adds an expression node whose type is inferred from its operands.
///
/// # Panics
///
/// Panics when the node is ill-typed; passes only build well-typed nodes.
pub(crate) fn mk(m: &mut Module, kind: ExprKind, span: Span) -> ExprId {
    let ty = match infer_type(m, &kind) {
        Ok(ty) => ty,
        Err(e) => panic!("synthesis built an ill-typed expression ({kind:?}): {e}"),
    };
    m.add_expr(Expr::new(kind, ty, span))
}

/// A constant node.
pub(crate) fn mk_const(m: &mut Module, value: Const, span: Span) -> ExprId {
    mk(m, ExprKind::Const(value), span)
}

/// A single-bit constant.
pub(crate) fn mk_bit(m: &mut Module, value: bool, span: Span) -> ExprId {
    mk_const(m, Const::from_bool(value), span)
}

/// An all-`x` constant of the given type (a don't-care value).
pub(crate) fn mk_undef(m: &mut Module, ty: &Type, span: Span) -> ExprId {
    let width = ty.width().unwrap_or(1);
    mk_const(m, Const::x(width).with_signed(ty.is_signed()), span)
}

/// The value of a net.
pub(crate) fn mk_net(m: &mut Module, net: NetId, span: Span) -> ExprId {
    mk(m, ExprKind::Net(net), span)
}

/// `base[hi:lo]`.
pub(crate) fn mk_slice(m: &mut Module, base: ExprId, hi: u32, lo: u32, span: Span) -> ExprId {
    mk(m, ExprKind::Slice { base, hi, lo }, span)
}

/// `{parts...}`; a single part is returned as is when it is unsigned.
pub(crate) fn mk_concat(m: &mut Module, parts: Vec<ExprId>, span: Span) -> ExprId {
    if parts.len() == 1 && !m.expr(parts[0]).ty.is_signed() {
        return parts[0];
    }
    mk(m, ExprKind::Concat(parts), span)
}

/// A unary operator.
pub(crate) fn mk_unary(m: &mut Module, op: UnaryOp, expr: ExprId, span: Span) -> ExprId {
    mk(m, ExprKind::Unary { op, expr }, span)
}

/// A binary operator.
pub(crate) fn mk_binary(
    m: &mut Module,
    op: BinaryOp,
    lhs: ExprId,
    rhs: ExprId,
    span: Span,
) -> ExprId {
    mk(m, ExprKind::Binary { op, lhs, rhs }, span)
}

/// `cond ? then_ : else_`.
pub(crate) fn mk_mux(
    m: &mut Module,
    cond: ExprId,
    then_: ExprId,
    else_: ExprId,
    span: Span,
) -> ExprId {
    if then_ == else_ {
        return then_;
    }
    mk(m, ExprKind::Ternary { cond, then_, else_ }, span)
}

/// A resize node, or the operand itself when its type already matches.
pub(crate) fn mk_resize(
    m: &mut Module,
    expr: ExprId,
    width: u32,
    signed: bool,
    span: Span,
) -> ExprId {
    if m.expr(expr).ty == (Type::Bits { width, signed }) {
        return expr;
    }
    mk(
        m,
        ExprKind::Resize {
            expr,
            width,
            signed,
        },
        span,
    )
}

/// Coerces `expr` to `ty` with a resize when the types differ; used when a
/// rewrite would otherwise change the type a parent node was built with.
pub(crate) fn coerce(m: &mut Module, expr: ExprId, ty: &Type, span: Span) -> ExprId {
    if &m.expr(expr).ty == ty {
        return expr;
    }
    match ty {
        Type::Bits { width, signed } => mk_resize(m, expr, *width, *signed, span),
        _ => expr,
    }
}

/// `not(a)` for a single bit, folding a double negation.
pub(crate) fn mk_not(m: &mut Module, expr: ExprId, span: Span) -> ExprId {
    if let ExprKind::Unary {
        op: UnaryOp::Not | UnaryOp::LogicNot,
        expr: inner,
    } = m.expr(expr).kind
        && m.expr(inner).ty.is_bit()
    {
        return inner;
    }
    if let Some(c) = m.expr(expr).as_const() {
        let v = c.not();
        return mk_const(m, v, span);
    }
    mk_unary(m, UnaryOp::Not, expr, span)
}

/// `and(a, b)` on single bits, with constant absorption.
pub(crate) fn mk_and(m: &mut Module, a: ExprId, b: ExprId, span: Span) -> ExprId {
    match (const_bit(m, a), const_bit(m, b)) {
        (Some(true), _) => return b,
        (_, Some(true)) => return a,
        (Some(false), _) | (_, Some(false)) => return mk_bit(m, false, span),
        _ => {}
    }
    if a == b {
        return a;
    }
    mk_binary(m, BinaryOp::And, a, b, span)
}

/// `or(a, b)` on single bits, with constant absorption.
pub(crate) fn mk_or(m: &mut Module, a: ExprId, b: ExprId, span: Span) -> ExprId {
    match (const_bit(m, a), const_bit(m, b)) {
        (Some(false), _) => return b,
        (_, Some(false)) => return a,
        (Some(true), _) | (_, Some(true)) => return mk_bit(m, true, span),
        _ => {}
    }
    if a == b {
        return a;
    }
    mk_binary(m, BinaryOp::Or, a, b, span)
}

/// The value of a known single-bit constant node.
pub(crate) fn const_bit(m: &Module, id: ExprId) -> Option<bool> {
    let c = m.expr(id).as_const()?;
    if c.width() != 1 {
        return None;
    }
    c.bit(0).to_bool()
}

/// Allocates a name not used by any net of `m`: `base`, then `base_2`,
/// `base_3`, ...
pub(crate) fn fresh_net_name(m: &Module, base: &str) -> Name {
    let taken: HashSet<&str> = m.nets.values().map(|n| n.name.as_str()).collect();
    fresh_name(base, |n| taken.contains(n))
}

/// Allocates a name not used by any cell of `m`.
pub(crate) fn fresh_cell_name(m: &Module, base: &str) -> Name {
    let taken: HashSet<&str> = m.cells.values().map(|c| c.name.as_str()).collect();
    fresh_name(base, |n| taken.contains(n))
}

fn fresh_name(base: &str, taken: impl Fn(&str) -> bool) -> Name {
    if !taken(base) {
        return Name::new(base);
    }
    let mut i = 2u32;
    loop {
        let candidate = format!("{base}_{i}");
        if !taken(&candidate) {
            return Name::new(candidate);
        }
        i += 1;
    }
}

/// Adds a wire with a fresh name derived from `base`.
pub(crate) fn add_wire(m: &mut Module, base: &str, ty: Type, span: Span) -> NetId {
    let name = fresh_net_name(m, base);
    m.nets.push(Net {
        name,
        ty,
        kind: NetKind::Wire,
        attrs: Attrs::new(),
        span,
    })
}

/// Adds a cell with a fresh name derived from `base`.
pub(crate) fn add_cell(
    m: &mut Module,
    base: &str,
    kind: CellKind,
    inputs: Vec<(&str, ExprId)>,
    outputs: Vec<(&str, NetId)>,
    span: Span,
) -> crate::ir::CellId {
    let name = fresh_cell_name(m, base);
    m.cells.push(Cell {
        name,
        kind,
        inputs: inputs.into_iter().map(|(n, e)| (Name::new(n), e)).collect(),
        outputs: outputs
            .into_iter()
            .map(|(n, e)| (Name::new(n), e))
            .collect(),
        params: Attrs::new(),
        attrs: Attrs::new(),
        span,
    })
}

/// True when the net is exposed by a port.
pub(crate) fn is_port(m: &Module, net: NetId) -> bool {
    m.ports.iter().any(|p| p.net == net)
}

/// True when the object must survive optimisation (`keep` attribute).
pub(crate) fn is_kept(attrs: &Attrs) -> bool {
    attrs.is_set("keep")
}

/// Renders a constant the way the `.rtl` text format does: decimal for
/// two-state values up to 64 bits, hexadecimal above, binary when any bit
/// is `x` or `z`.
pub(crate) fn const_text(c: &Const) -> String {
    let s = if c.is_signed() { "s" } else { "" };
    if c.has_unknown() {
        format!("{}'{s}b{}", c.width(), c.to_binary_string())
    } else if let Some(v) = c.to_u64() {
        format!("{}'{s}d{v}", c.width())
    } else {
        c.to_verilog_literal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn builds_and_folds_bits() {
        let span = span();
        let mut m = Module::new("m", span);
        let a = add_wire(&mut m, "a", Type::bit(), span);
        let a2 = add_wire(&mut m, "a", Type::bit(), span);
        assert_eq!(m.nets[a2].name, "a_2");
        let an = mk_net(&mut m, a, span);
        let t = mk_bit(&mut m, true, span);
        let f = mk_bit(&mut m, false, span);
        assert_eq!(mk_and(&mut m, an, t, span), an);
        assert_eq!(mk_or(&mut m, an, f, span), an);
        let af = mk_and(&mut m, an, f, span);
        assert_eq!(const_bit(&m, af), Some(false));
        let ot = mk_or(&mut m, an, t, span);
        assert_eq!(const_bit(&m, ot), Some(true));
        let n = mk_not(&mut m, an, span);
        assert_eq!(mk_not(&mut m, n, span), an);
        let nt = mk_not(&mut m, t, span);
        assert_eq!(const_bit(&m, nt), Some(false));
        assert_eq!(mk_mux(&mut m, an, t, t, span), t);
        assert_eq!(mk_resize(&mut m, an, 1, false, span), an);
        let wide = mk_resize(&mut m, an, 4, false, span);
        assert_eq!(m.expr(wide).ty, Type::bits(4));
        assert_eq!(coerce(&mut m, wide, &Type::bits(4), span), wide);
        let signed = coerce(&mut m, wide, &Type::sbits(4), span);
        assert_eq!(m.expr(signed).ty, Type::sbits(4));
        assert_eq!(mk_concat(&mut m, vec![wide], span), wide);
        let u = mk_undef(&mut m, &Type::bits(3), span);
        assert!(m.expr(u).as_const().unwrap().has_unknown());
        assert!(!is_port(&m, a));
        let c = add_cell(
            &mut m,
            "c",
            CellKind::Not,
            vec![("a", an)],
            vec![("y", a2)],
            span,
        );
        assert_eq!(m.cells[c].name, "c");
        assert_eq!(fresh_cell_name(&m, "c"), Name::new("c_2"));
        assert!(!is_kept(&m.cells[c].attrs));
        assert_eq!(const_text(&Const::from_u64(255, 8)), "8'd255");
        assert_eq!(const_text(&Const::from_i64(-1, 8)), "8'sd255");
        assert_eq!(const_text(&Const::x(2)), "2'bxx");
        assert_eq!(const_text(&Const::ones(72)), "72'hffffffffffffffffff");
    }
}
