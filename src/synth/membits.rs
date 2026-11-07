//! The bits of a memory write port's enable.
//!
//! A write of part of a memory element (`mem[a][15:8] <= d`,
//! [`Lvalue::MemPart`](crate::ir::Lvalue::MemPart)) is lowered to a write
//! port whose `en` has a bit per bit of the element: the write's condition on
//! the bits it writes, constant zeros on the rest, built as a concatenation
//! of copies of the condition (`{16'd0, {8{c}}, 8'd0}`). [`enable_bits`]
//! reads such an enable back bit by bit, so that the ports of one address can
//! be merged ([`super::proc`]) and a memory's byte lanes found
//! (`fpga::lanes`, and the block RAM mapper's byte write enables).

use crate::ir::{ExprId, ExprKind, Module};
use crate::synth::util::same_expr;

/// What enables one bit of an element, for one write port.
#[derive(Clone, Copy, Debug)]
pub(crate) enum EnBit {
    /// Never written by the port.
    Zero,
    /// Written whenever the port writes.
    One,
    /// Written when this one-bit expression is one.
    Cond(ExprId),
    /// Written when bit `k` of this expression is one: an enable the
    /// lanes cannot be read off, a write at an offset known only at run
    /// time.
    Bit(ExprId, u32),
}

/// The enable of each bit of an element `width` bits wide, least
/// significant first, as a write port's `en` gives it: a one-bit enable
/// enables every bit alike.
pub(crate) fn enable_bits(m: &Module, en: ExprId, width: u32) -> Vec<EnBit> {
    let w = m.expr(en).ty.width().unwrap_or(1);
    if w == 1 {
        let bit = match m.expr(en).as_const().and_then(|c| c.bit(0).to_bool()) {
            Some(false) => EnBit::Zero,
            Some(true) => EnBit::One,
            None => EnBit::Cond(en),
        };
        return vec![bit; width as usize];
    }
    let mut out = Vec::with_capacity(width as usize);
    bits_of(m, en, &mut out);
    out.resize(width as usize, EnBit::Zero);
    out
}

/// Appends the enables of `e`'s bits, least significant first.
fn bits_of(m: &Module, e: ExprId, out: &mut Vec<EnBit>) {
    let node = m.expr(e);
    let w = node.ty.width().unwrap_or(1);
    match &node.kind {
        ExprKind::Const(c) => {
            for k in 0..w {
                out.push(match c.bit(k).to_bool() {
                    Some(true) => EnBit::One,
                    _ => EnBit::Zero,
                });
            }
        }
        ExprKind::Concat(parts) => {
            for p in parts.iter().rev() {
                bits_of(m, *p, out);
            }
        }
        ExprKind::Replicate { expr, count } if m.expr(*expr).ty.width() == Some(1) => {
            let mut one = Vec::new();
            bits_of(m, *expr, &mut one);
            let bit = one.first().copied().unwrap_or(EnBit::Zero);
            for _ in 0..*count {
                out.push(bit);
            }
        }
        _ if w == 1 => out.push(EnBit::Cond(e)),
        _ => {
            for k in 0..w {
                out.push(EnBit::Bit(e, k));
            }
        }
    }
}

/// Whether two bits are enabled by the same thing.
pub(crate) fn same_bit(m: &Module, a: EnBit, b: EnBit) -> bool {
    match (a, b) {
        (EnBit::Zero, EnBit::Zero) | (EnBit::One, EnBit::One) => true,
        (EnBit::Cond(x), EnBit::Cond(y)) => same_expr(m, x, y, 16),
        (EnBit::Bit(x, i), EnBit::Bit(y, j)) => i == j && same_expr(m, x, y, 16),
        _ => false,
    }
}

/// A one-bit expression of what enables a bit: `None` for [`EnBit::Zero`].
pub(crate) fn bit_expr(m: &mut Module, bit: EnBit, span: crate::source::Span) -> Option<ExprId> {
    use crate::synth::util::{mk_const, mk_slice};
    Some(match bit {
        EnBit::Zero => return None,
        EnBit::One => mk_const(m, crate::ir::Const::ones(1), span),
        EnBit::Cond(c) => c,
        EnBit::Bit(e, k) => mk_slice(m, e, k, k, span),
    })
}

/// The enable `bits` describe (least significant first), as an expression
/// of their width: runs of one condition as copies of it, runs of a wider
/// expression's bits as a slice of it, most significant first, as
/// synthesis builds an enable to begin with.
pub(crate) fn enable_expr(m: &mut Module, bits: &[EnBit], span: crate::source::Span) -> ExprId {
    use crate::ir::Const;
    use crate::synth::util::{mk, mk_concat, mk_const, mk_slice};
    let mut parts: Vec<ExprId> = Vec::new();
    let mut at = 0;
    while at < bits.len() {
        let mut end = at + 1;
        let run = |a: EnBit, b: EnBit, step: u32| match (a, b) {
            (EnBit::Bit(x, i), EnBit::Bit(y, j)) => x == y && i + step == j,
            _ => same_bit(m, a, b),
        };
        while end < bits.len() && run(bits[end - 1], bits[end], 1) {
            end += 1;
        }
        let n = u32::try_from(end - at).unwrap_or(u32::MAX);
        let part = match bits[at] {
            EnBit::Zero => mk_const(m, Const::zero(n), span),
            EnBit::One => mk_const(m, Const::ones(n), span),
            EnBit::Cond(c) if n == 1 => c,
            EnBit::Cond(c) => mk(m, ExprKind::Replicate { expr: c, count: n }, span),
            EnBit::Bit(e, k) => mk_slice(m, e, k + n - 1, k, span),
        };
        parts.push(part);
        at = end;
    }
    parts.reverse();
    mk_concat(m, parts, span)
}
