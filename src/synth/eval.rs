//! Expression evaluation over [`Const`] values.
//!
//! [`eval`] computes the value of an expression tree given an [`Env`] that
//! resolves nets and memory reads. Constant folding uses it with an
//! environment that knows nothing (so only closed subtrees fold), process
//! lowering uses it with the values of loop variables, and the test-only
//! self-check simulator uses it with a full net state.
//!
//! Semantics follow IEEE 1364 as implemented by [`Const`] (see
//! [`crate::logic`]): arithmetic on unknown operands is all `x`, an `x`
//! condition of a ternary merges the branches bit by bit, an out-of-range
//! variable index reads `x`, and `Resize` sign-extends only when both the
//! node and the operand are signed.

use crate::ir::{BinaryOp, Const, ExprId, ExprKind, MemoryId, Module, NetId, Type, UnaryOp};
use crate::logic::Bit;

/// Resolves the leaves of an expression.
pub trait Env {
    /// The current value of `net`, or `None` when it is not known.
    fn net(&mut self, net: NetId) -> Option<Const>;

    /// The element of `mem` at `addr`, or `None` when it is not known. The
    /// default knows no memory.
    fn mem(&mut self, mem: MemoryId, addr: &Const) -> Option<Const> {
        let _ = (mem, addr);
        None
    }
}

/// An environment that resolves nothing: only closed expressions evaluate.
pub struct NoEnv;

impl Env for NoEnv {
    fn net(&mut self, _net: NetId) -> Option<Const> {
        None
    }
}

/// Evaluates `id` in `module` under `env`. Returns `None` when a leaf is
/// unknown, the expression is not a bit vector (strings, unlowered calls,
/// arrays) or a selection is malformed.
pub fn eval(module: &Module, id: ExprId, env: &mut dyn Env) -> Option<Const> {
    let expr = module.exprs.get(id)?;
    let Type::Bits { width, signed } = expr.ty else {
        return None;
    };
    let value = match &expr.kind {
        ExprKind::Const(c) => c.clone(),
        ExprKind::String(_) | ExprKind::Call { .. } => return None,
        ExprKind::Net(net) => env.net(*net)?,
        ExprKind::Slice { base, hi, lo } => {
            let b = eval(module, *base, env)?;
            if *hi < *lo || *hi >= b.width() {
                return None;
            }
            b.slice(*hi, *lo)
        }
        ExprKind::Index { base, index } => {
            let b = eval(module, *base, env)?;
            let i = eval(module, *index, env)?;
            match i.to_u64() {
                Some(i) if i < u64::from(b.width()) => {
                    Const::from_bit(b.bit(u32::try_from(i).ok()?))
                }
                _ => Const::x(1),
            }
        }
        ExprKind::IndexedSlice {
            base,
            offset,
            width: w,
            up,
        } => {
            let b = eval(module, *base, env)?;
            let off = eval(module, *offset, env)?;
            let Some(off) = off.to_u64() else {
                return Some(Const::x(*w));
            };
            let lo = if *up {
                off
            } else {
                off.checked_sub(u64::from(*w) - 1)?
            };
            let bits: Vec<Bit> = (0..*w)
                .map(|k| {
                    let i = lo + u64::from(k);
                    if i < u64::from(b.width()) {
                        b.bit(u32::try_from(i).unwrap_or(u32::MAX))
                    } else {
                        Bit::X
                    }
                })
                .collect();
            Const::from_bits(&bits)
        }
        ExprKind::Concat(parts) => {
            let mut values = Vec::with_capacity(parts.len());
            for part in parts {
                values.push(eval(module, *part, env)?);
            }
            Const::concat_all(values.iter())
        }
        ExprKind::Replicate { count, expr } => eval(module, *expr, env)?.replicate(*count),
        ExprKind::Unary { op, expr } => {
            let v = eval(module, *expr, env)?;
            match op {
                UnaryOp::Not => v.not(),
                UnaryOp::Neg => v.neg(),
                UnaryOp::ReduceAnd => v.reduce_and(),
                UnaryOp::ReduceOr => v.reduce_or(),
                UnaryOp::ReduceXor => v.reduce_xor(),
                UnaryOp::ReduceNand => v.reduce_nand(),
                UnaryOp::ReduceNor => v.reduce_nor(),
                UnaryOp::ReduceXnor => v.reduce_xnor(),
                UnaryOp::LogicNot => v.logical_not(),
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval(module, *lhs, env)?;
            let b = eval(module, *rhs, env)?;
            if !op.is_shift() && a.width() != b.width() {
                return None;
            }
            match op {
                BinaryOp::And => a.and(&b),
                BinaryOp::Or => a.or(&b),
                BinaryOp::Xor => a.xor(&b),
                BinaryOp::Xnor => a.xnor(&b),
                BinaryOp::LogicAnd => a.logical_and(&b),
                BinaryOp::LogicOr => a.logical_or(&b),
                BinaryOp::Add => a.add(&b),
                BinaryOp::Sub => a.sub(&b),
                BinaryOp::Mul => a.mul(&b),
                BinaryOp::Div => a.div(&b),
                BinaryOp::Mod => a.rem(&b),
                BinaryOp::Pow => a.pow(&b),
                BinaryOp::Shl => a.shl_by(&b),
                BinaryOp::Shr => a.shr_by(&b),
                BinaryOp::Sshr => a.sshr_by(&b),
                BinaryOp::Eq => a.eq(&b),
                BinaryOp::Ne => a.ne(&b),
                BinaryOp::CaseEq => a.case_eq(&b),
                BinaryOp::CaseNe => a.case_ne(&b),
                BinaryOp::WildEq => a.wildcard_eq(&b),
                BinaryOp::Lt => a.lt(&b),
                BinaryOp::Le => a.le(&b),
                BinaryOp::Gt => a.gt(&b),
                BinaryOp::Ge => a.ge(&b),
            }
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            let c = eval(module, *cond, env)?;
            match c.truth() {
                Bit::One => eval(module, *then_, env)?,
                Bit::Zero => eval(module, *else_, env)?,
                _ => {
                    let t = eval(module, *then_, env)?;
                    let e = eval(module, *else_, env)?;
                    merge_unknown(&t, &e)
                }
            }
        }
        ExprKind::Resize {
            expr,
            width: w,
            signed: s,
        } => {
            let v = eval(module, *expr, env)?;
            let v = if *s { v } else { v.as_unsigned() };
            v.resize(*w)
        }
        ExprKind::MemRead { mem, addr } => {
            let a = eval(module, *addr, env)?;
            env.mem(*mem, &a)?
        }
    };
    if value.width() != width {
        return None;
    }
    Some(value.with_signed(signed))
}

/// The bitwise merge of two values as an `x` ternary condition yields:
/// bits that agree and are known are kept, every other bit is `x`.
pub fn merge_unknown(a: &Const, b: &Const) -> Const {
    if a == b {
        return a.clone();
    }
    let bits: Vec<Bit> = (0..a.width())
        .map(|i| {
            let (x, y) = (a.bit(i), b.get(i).unwrap_or(Bit::X));
            if x == y && x.is_known() { x } else { Bit::X }
        })
        .collect();
    Const::from_bits(&bits).with_signed(a.is_signed() && b.is_signed())
}

/// Evaluates a closed expression (one without net or memory leaves).
pub fn eval_closed(module: &Module, id: ExprId) -> Option<Const> {
    eval(module, id, &mut NoEnv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::source::{SourceMap, Span};

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    struct Fixed(Const);

    impl Env for Fixed {
        fn net(&mut self, _net: NetId) -> Option<Const> {
            Some(self.0.clone())
        }
        fn mem(&mut self, _mem: MemoryId, addr: &Const) -> Option<Const> {
            Some(addr.resize(8))
        }
    }

    #[test]
    fn evaluates_every_kind() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let mem = b.memory("mem", Type::bits(8), 16);
        let an = b.net(a);
        let c3 = b.const_u64(8, 3);
        let sum = b.add(an, c3);
        let sl = b.slice(sum, 3, 0);
        let idx = b.index(an, c3);
        let big = b.const_u64(8, 200);
        let bad_idx = b.index(an, big);
        let is = b.indexed_slice(an, c3, 2, true);
        let isd = b.indexed_slice(an, c3, 2, false);
        let cat = b.concat(vec![sl, is]);
        let rep = b.replicate(2, sl);
        let mut unaries = Vec::new();
        for op in UnaryOp::ALL {
            unaries.push(b.unary(op, an));
        }
        let one = b.const_bit(true);
        let mut binaries = Vec::new();
        for op in BinaryOp::ALL {
            if op.is_logical() {
                binaries.push(b.binary(op, one, one));
            } else {
                binaries.push(b.binary(op, an, c3));
            }
        }
        let x = b.constant(Const::x(1));
        let t1 = b.mux(one, an, c3);
        let tx = b.mux(x, an, c3);
        let rs = b.sext(an, 12);
        let rz = b.zext(an, 12);
        let rd = b.mem_read(mem, sl);
        let s = b.string("s");
        let call = b.call("$clog2", vec![an], Type::bits(8));
        let m = b.finish();
        let mut env = Fixed(Const::from_u64(0x55, 8));
        let ev = |id| eval(&m, id, &mut Fixed(Const::from_u64(0x55, 8)));
        assert_eq!(ev(sum), Some(Const::from_u64(0x58, 8)));
        assert_eq!(ev(sl), Some(Const::from_u64(8, 4)));
        assert_eq!(ev(idx), Some(Const::from_bool(false)));
        assert_eq!(ev(bad_idx), Some(Const::x(1)));
        assert_eq!(ev(is), Some(Const::from_u64(0b10, 2)));
        assert_eq!(ev(isd), Some(Const::from_u64(0b01, 2)));
        assert_eq!(ev(cat), Some(Const::from_u64(0b10_0010, 6)));
        assert_eq!(ev(rep), Some(Const::from_u64(0x88, 8)));
        for id in unaries.iter().chain(&binaries) {
            assert!(ev(*id).is_some(), "{:?}", m.expr(*id).kind);
        }
        assert_eq!(ev(t1), Some(Const::from_u64(0x55, 8)));
        // 0x55 = 0101_0101, 3 = 0000_0011: bits 7, 5, 3, 1, 0 agree.
        assert_eq!(
            ev(tx).map(|v| v.to_binary_string()),
            Some("0x0x0xx1".to_owned())
        );
        assert_eq!(ev(rs), Some(Const::from_u64(0x55, 12).as_signed()));
        assert_eq!(ev(rz), Some(Const::from_u64(0x55, 12)));
        assert_eq!(eval(&m, rd, &mut env), Some(Const::from_u64(8, 8)));
        assert_eq!(ev(s), None);
        assert_eq!(ev(call), None);
        assert_eq!(eval_closed(&m, an), None);
        assert_eq!(eval_closed(&m, c3), Some(Const::from_u64(3, 8)));
        assert_eq!(eval_closed(&m, ExprId::from_index_test(999)), None);
    }

    impl ExprId {
        fn from_index_test(i: usize) -> ExprId {
            <ExprId as crate::ir::Id>::from_index(i)
        }
    }

    #[test]
    fn merge_keeps_agreeing_known_bits() {
        let a = Const::from_u64(0b1100, 4);
        let b = Const::from_u64(0b1010, 4);
        assert_eq!(merge_unknown(&a, &b).to_binary_string(), "1xx0");
        assert_eq!(merge_unknown(&a, &a), a);
    }
}
