//! A multiply wider than one `DSP48E1`, over several joined by `PCOUT`.
//!
//! Each operand is cut into 17-bit pieces read unsigned, with one signed piece
//! on top. Each piece of one operand multiplies the other's pieces up a column
//! of blocks, every block adding the one below's result shifted down 17
//! (`OPMODE` `1010101`, `Z = PCIN >>> 17`), so that a column's result is the low
//! 17 bits of each block and the whole of the top one. The columns are then
//! summed in the fabric's carry chains.
//!
//! # Where this is from
//!
//! **Measured**: `testdata/fpga/vivado/dsp/inference.txt`'s `s_32x32`,
//! `u_32x32` and `s_48x48` are four, four and nine blocks of exactly this shape,
//! two or three to a column, with `CARRY4`s behind them. `s_32x32t` and
//! `u_32x32t`, truncated to 32 bits, are three: a piece whose product lies
//! entirely above the result has no block.
//!
//! **What Vivado does and this does not**: an operand one bit past a port
//! (`s_26x18`, `s_25x19`, `u_18x18`) is **one** block there, the missing bit's
//! partial product arriving on `C` selected by a dynamic `OPMODE`; `u_25x18` is
//! two. Here each is two or four. And Vivado moves the register after the
//! multiply back into each block's `P`, and those of `s_48x48` into the middle
//! of a column, where this uses `M` and leaves the rest of the pipeline in the
//! fabric. Both are the same function at more cost; neither is a wrong answer.
//!
//! What was checked of the result is in `tests/fpga_dsp.rs` and
//! `tests/fpga_dsp_vivado.rs`, which also say what they would not catch.

use super::dsp_absorb::{self, Index};
use super::primitives::{add_assign, add_cell, add_net, const_expr, expr, net_expr, slice_expr};
use crate::fpga::device::DspShape;
use crate::ir::{CellId, CellKind, Const, ExprId, ExprKind, Module, Name, NetId, Type};
use crate::source::Span;

/// How wide a low piece is: `PCIN`'s shift, and the widest unsigned operand the
/// 18-bit port takes.
const PIECE: u32 = 17;

/// One operand, as a bit vector with the sign it is read by on top.
#[derive(Clone, Copy, Debug)]
struct Operand {
    /// Its bits; the top one is the sign when it has one.
    value: ExprId,
}

/// One piece of an operand.
#[derive(Clone, Copy, Debug)]
struct Piece {
    /// Its lowest bit in the operand.
    lo: u32,
    width: u32,
    /// Read as two's complement: the operand's top piece.
    signed: bool,
}

/// Cuts `width` bits into pieces, none wider than `port` on top.
fn pieces(width: u32, port: u32) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut lo = 0;
    let mut rest = width;
    while rest > port {
        out.push(Piece {
            lo,
            width: PIECE,
            signed: false,
        });
        lo += PIECE;
        rest -= PIECE;
    }
    out.push(Piece {
        lo,
        width: rest,
        signed: true,
    });
    out
}

/// How many blocks a `result`-bit product takes, with `a` and `b` so cut.
fn block_count(a: &[Piece], b: &[Piece], result: u32) -> usize {
    a.iter()
        .flat_map(|x| b.iter().map(move |y| x.lo + y.lo))
        .filter(|shift| *shift < result)
        .count()
}

/// The piece of an operand, as bits for a port of `pin` bits.
fn piece_on_pin(module: &mut Module, op: Operand, piece: Piece, pin: u32, span: Span) -> ExprId {
    let bits = slice_expr(module, op.value, piece.lo + piece.width - 1, piece.lo, span);
    if piece.width == pin {
        return bits;
    }
    let fill: ExprId = if piece.signed {
        slice_expr(
            module,
            op.value,
            piece.lo + piece.width - 1,
            piece.lo + piece.width - 1,
            span,
        )
    } else {
        const_expr(module, Const::from_u64(0, 1), span)
    };
    let mut parts = vec![fill; (pin - piece.width) as usize];
    parts.push(bits);
    expr(module, ExprKind::Concat(parts), span)
}

/// The operand as a vector of `width` bits, sign or zero extended.
fn extended(module: &mut Module, source: ExprId, sign: bool, width: u32, span: Span) -> ExprId {
    let have = module
        .exprs
        .get(source)
        .and_then(|e| e.ty.width())
        .unwrap_or(1);
    if have == width {
        return source;
    }
    if have > width {
        return slice_expr(module, source, width - 1, 0, span);
    }
    let fill = if sign {
        slice_expr(module, source, have - 1, have - 1, span)
    } else {
        const_expr(module, Const::from_u64(0, 1), span)
    };
    let mut parts = vec![fill; (width - have) as usize];
    parts.push(source);
    expr(module, ExprKind::Concat(parts), span)
}

/// One operand of a multiply: its value, how many bits matter, and whether it
/// widens by its sign; as `primitives::significant_operand` reads it.
pub(super) struct Wide {
    /// The expression.
    pub source: ExprId,
    /// The bits of it that carry the value.
    pub bits: u32,
    /// Copies its sign when widening; else its top is a zero.
    pub sign_extend: bool,
}

/// What a split came to.
pub(super) struct Split {
    /// The flip-flop after the product that moved into `M`, if one did.
    pub taken: Option<CellId>,
    /// How many blocks it took.
    pub blocks: usize,
}

/// Replaces `y = a * b` with blocks and adders, or does nothing and gives
/// `None` when that is more blocks than `limit`.
#[allow(clippy::too_many_arguments, reason = "one multiply's whole context")]
#[allow(clippy::too_many_lines, reason = "one structure, built once")]
pub(super) fn split(
    module: &mut Module,
    shape: &DspShape,
    index: &mut Index,
    a: &Wide,
    b: &Wide,
    y: NetId,
    uses: &[usize],
    limit: usize,
    name: &str,
    span: Span,
) -> Option<Split> {
    let result = module.nets.get(y)?.ty.width()?;
    // The bits an operand needs: its own, plus the zero a read as two's
    // complement needs on top, and never more than the result does.
    let need = |op: &Wide| (op.bits + u32::from(!op.sign_extend)).min(result);
    let (na, nb) = (need(a), need(b));
    // The wider on `a`, as Vivado has it, unless the other way is fewer blocks.
    let forward = (pieces(na, shape.a_width), pieces(nb, shape.b_width));
    let swapped = (pieces(nb, shape.a_width), pieces(na, shape.b_width));
    let swap = block_count(&swapped.0, &swapped.1, result)
        < block_count(&forward.0, &forward.1, result)
        || (block_count(&swapped.0, &swapped.1, result)
            == block_count(&forward.0, &forward.1, result)
            && nb > na);
    let (a, b, a_pieces, b_pieces, na, nb) = if swap {
        (b, a, swapped.0, swapped.1, nb, na)
    } else {
        (a, b, forward.0, forward.1, na, nb)
    };
    let count = block_count(&a_pieces, &b_pieces, result);
    if count > limit {
        return None;
    }

    let a_source = extended(module, a.source, a.sign_extend, na, span);
    let b_source = extended(module, b.source, b.sign_extend, nb, span);
    // Registers: what an operand was traced back through goes into every
    // block, so every block sees the same cycle.
    let operands = dsp_absorb::operand_plan(module, index, a_source, b_source, span);
    let after = dsp_absorb::plain_output(module, index, operands.clock(), y, uses);
    let a_op = Operand { value: operands.a };
    let b_op = Operand { value: operands.b };

    let p_width = shape.p_width;
    let mut total: Option<ExprId> = None;
    let a_pin = shape.pin_width("A").unwrap_or(shape.a_width);
    let b_pin = shape.pin_width("B").unwrap_or(shape.b_width);
    for (column, b_piece) in b_pieces.iter().enumerate() {
        let shift = b_piece.lo;
        if shift >= result {
            continue;
        }
        let want = result - shift;
        let mut low_parts: Vec<ExprId> = Vec::new();
        let mut cascade: Option<NetId> = None;
        let mut top: Option<NetId> = None;
        let used: Vec<&Piece> = a_pieces.iter().filter(|p| p.lo < want).collect();
        for (row, a_piece) in used.iter().enumerate() {
            let last = row + 1 == used.len();
            let cell_name = format!("{name}$dsp{column}_{row}");
            let a_value = piece_on_pin(module, a_op, **a_piece, a_pin, span);
            let b_value = piece_on_pin(module, b_op, *b_piece, b_pin, span);
            let mut inputs: Vec<(Name, ExprId)> = vec![
                (Name::new(shape.port("a").unwrap_or("A")), a_value),
                (Name::new(shape.port("b").unwrap_or("B")), b_value),
            ];
            for (pin, value) in &shape.ties {
                let value = match (pin.as_str(), cascade) {
                    ("PCIN", Some(net)) => net_expr(module, net, span),
                    ("OPMODE", Some(_)) => const_expr(module, Const::from_u64(0b101_0101, 7), span),
                    _ => const_expr(module, value.clone(), span),
                };
                inputs.push((Name::new(pin.as_str()), value));
            }
            let p = add_net(
                module,
                &format!("{cell_name}$p"),
                Type::sbits(p_width),
                span,
            );
            let mut outputs = vec![(Name::new("P"), p)];
            let pcout = if last {
                None
            } else {
                let net = add_net(
                    module,
                    &format!("{cell_name}$pc"),
                    Type::bits(p_width),
                    span,
                );
                outputs.push((Name::new("PCOUT"), net));
                Some(net)
            };
            let cell = add_cell(
                module,
                &cell_name,
                CellKind::Blackbox(Name::new(shape.name.clone())),
                inputs,
                outputs,
                span,
            );
            for (key, value) in &shape.params {
                module.cells[cell].params.set(key.as_str(), value.clone());
            }
            operands.apply(module, cell, after.as_ref(), shape);
            cascade = pcout;
            if last {
                top = Some(p);
            } else {
                let value = net_expr(module, p, span);
                low_parts.push(slice_expr(module, value, PIECE - 1, 0, span));
            }
        }
        let top = top?;
        // The column: the low 17 bits of each block below the top, and the top
        // block's result, whose sign fills any bits above it.
        let low_bits = PIECE * u32::try_from(low_parts.len()).unwrap_or(0);
        let top_value = net_expr(module, top, span);
        let top_bits = want.saturating_sub(low_bits);
        let high = if top_bits <= p_width {
            slice_expr(module, top_value, top_bits.max(1) - 1, 0, span)
        } else {
            let sign = slice_expr(module, top_value, p_width - 1, p_width - 1, span);
            let mut parts = vec![sign; (top_bits - p_width) as usize];
            parts.push(top_value);
            expr(module, ExprKind::Concat(parts), span)
        };
        let column_value = if low_parts.is_empty() {
            high
        } else {
            let mut parts = Vec::with_capacity(low_parts.len() + 1);
            parts.push(high);
            parts.extend(low_parts.iter().rev().copied());
            let joined = expr(module, ExprKind::Concat(parts), span);
            slice_expr(module, joined, want - 1, 0, span)
        };
        // Into the running sum: the bits below this column's shift are final.
        total = Some(match total {
            None => column_value,
            Some(sum) => {
                let upper = slice_expr(module, sum, result - 1, shift, span);
                let out = add_net(
                    module,
                    &format!("{name}$sum{column}"),
                    Type::bits(want),
                    span,
                );
                add_cell(
                    module,
                    &format!("{name}$add{column}"),
                    CellKind::Add,
                    vec![(Name::new("a"), upper), (Name::new("b"), column_value)],
                    vec![(Name::new("y"), out)],
                    span,
                );
                let added = net_expr(module, out, span);
                let lower = slice_expr(module, sum, shift - 1, 0, span);
                expr(module, ExprKind::Concat(vec![added, lower]), span)
            }
        });
    }
    let total = total?;
    let (target, value, taken) = match after {
        Some(stage) => {
            let (q, value) = stage.connect(module, y, total, span)?;
            (q, value, Some(stage.cell()))
        }
        None => (y, total, None),
    };
    let value = retype(module, value, target, span);
    add_assign(module, target, value, span);
    Some(Split {
        taken,
        blocks: count,
    })
}

/// `value` with the type the net it drives has.
fn retype(module: &mut Module, value: ExprId, net: NetId, span: Span) -> ExprId {
    let Some(want) = module.nets.get(net).map(|n| n.ty.clone()) else {
        return value;
    };
    let have = module.exprs.get(value).map(|e| e.ty.clone());
    if have.as_ref() == Some(&want) {
        return value;
    }
    let (Type::Bits { width, signed }, Some(have)) = (want, have) else {
        return value;
    };
    let _ = have;
    expr(
        module,
        ExprKind::Resize {
            expr: value,
            width,
            signed,
        },
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(list: &[Piece]) -> Vec<u32> {
        list.iter().map(|p| p.width).collect()
    }

    /// The cuts Vivado's blocks show: `A=17`, `B=18` for the low piece, up to
    /// its port's width for the top one. Would catch a piece one wider than
    /// `PCIN`'s shift, which would double-count a bit between two blocks.
    #[test]
    fn an_operand_is_cut_as_vivados_blocks_show() {
        assert_eq!(widths(&pieces(25, 25)), [25]);
        assert_eq!(widths(&pieces(26, 25)), [17, 9]);
        assert_eq!(widths(&pieces(32, 25)), [17, 15]);
        assert_eq!(widths(&pieces(48, 25)), [17, 17, 14]);
        assert_eq!(widths(&pieces(19, 18)), [17, 2]);
        assert!(pieces(32, 18)[1].signed && !pieces(32, 18)[0].signed);
    }

    /// `s_32x32t`'s three blocks.
    #[test]
    fn a_product_wholly_above_the_result_has_no_block() {
        let (a, b) = (pieces(32, 25), pieces(32, 18));
        assert_eq!(block_count(&a, &b, 64), 4);
        assert_eq!(block_count(&a, &b, 32), 3);
        assert_eq!(block_count(&a, &b, 17), 1);
    }
}
