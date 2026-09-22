//! Expression evaluation over the IR arena.
//!
//! [`eval`] walks an expression tree and computes a [`Value`] using the
//! `Logic` operators, so the semantics of IEEE 1364-2005 §5 come from one
//! place. It is written against the small [`Env`] trait rather than the
//! simulator directly: the environment supplies net and memory reads and
//! function calls, which is all that differs between the event-driven
//! simulator, a future cycle-based evaluator and constant folding.
//!
//! Selections outside an operand read as `x` (§5.2.1), an unknown index
//! yields `x`, and an unknown condition in the conditional operator merges
//! both branches bit by bit (§5.1.13). Reals are supported for the
//! arithmetic, relational and logical operators; anything else converts the
//! real to an integer first. `Call` is delegated to the environment, which
//! implements the system functions and reports unknown ones.

use crate::ir::{BinaryOp, ExprId, ExprKind, MemoryId, Module, NetId, Span, Type, UnaryOp};
use crate::logic::{Bit, Logic};

use super::Simulator;
use super::elab::InstId;
use super::value::{Value, elem_count, elem_width, flat_width, merge_unknown, select_bits};

/// What expression evaluation needs from its surroundings.
pub(crate) trait Env<'d> {
    /// The module whose arena the expression ids index.
    fn module(&self) -> &'d Module;
    /// The current value of a net.
    fn read_net(&mut self, net: NetId) -> Value;
    /// The element at `addr`, or `None` when out of range.
    fn read_mem(&mut self, mem: MemoryId, addr: u64) -> Option<Logic>;
    /// A function call with evaluated arguments; `ty` is the cached result
    /// type.
    fn call(&mut self, name: &str, args: Vec<Value>, ty: &Type, span: Span) -> Value;
}

/// An index operand as an integer: `None` when it has unknown bits;
/// values beyond `i64` saturate (they are out of range for any operand).
pub(crate) fn index_value(l: &Logic) -> Option<i64> {
    if l.has_unknown() {
        return None;
    }
    if l.is_signed() {
        return l.to_i64().or(Some(i64::MAX));
    }
    Some(
        l.to_u64()
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(i64::MAX),
    )
}

/// Evaluates expression `id`.
pub(crate) fn eval<'d, E: Env<'d>>(env: &mut E, id: ExprId) -> Value {
    let m = env.module();
    let Some(node) = m.exprs.get(id) else {
        return Value::Bits(Logic::x(1));
    };
    match &node.kind {
        ExprKind::Const(c) => Value::Bits(c.clone()),
        ExprKind::String(s) => Value::Str(s.clone()),
        ExprKind::Net(n) => env.read_net(*n),
        ExprKind::Slice { base, hi, lo } => {
            let ew = m.exprs.get(*base).map_or(1, |b| elem_width(&b.ty));
            let b = eval(env, *base).to_logic();
            let width = hi.saturating_sub(*lo).saturating_add(1).saturating_mul(ew);
            Value::Bits(select_bits(&b, i64::from(*lo) * i64::from(ew), width))
        }
        ExprKind::Index { base, index } => {
            let (ew, count) = m
                .exprs
                .get(*base)
                .map_or((1, 0), |b| (elem_width(&b.ty), elem_count(&b.ty)));
            let b = eval(env, *base).to_logic();
            let i = eval(env, *index).to_logic();
            match index_value(&i) {
                Some(i) if i >= 0 && u64::try_from(i).is_ok_and(|i| i < count) => {
                    Value::Bits(select_bits(&b, i * i64::from(ew), ew))
                }
                _ => Value::Bits(Logic::x(ew)),
            }
        }
        ExprKind::IndexedSlice {
            base,
            offset,
            width,
            up,
        } => {
            let b = eval(env, *base).to_logic();
            let off = eval(env, *offset).to_logic();
            match index_value(&off) {
                Some(o) => {
                    let lo = if *up {
                        o
                    } else {
                        o.saturating_sub(i64::from(*width)).saturating_add(1)
                    };
                    Value::Bits(select_bits(&b, lo, *width))
                }
                None => Value::Bits(Logic::x(*width)),
            }
        }
        ExprKind::Concat(parts) => {
            let vals: Vec<Logic> = parts.iter().map(|p| eval(env, *p).to_logic()).collect();
            Value::Bits(Logic::concat_all(vals.iter()))
        }
        ExprKind::Replicate { count, expr } => {
            Value::Bits(eval(env, *expr).to_logic().replicate(*count))
        }
        ExprKind::Unary { op, expr } => unary(*op, eval(env, *expr)),
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval(env, *lhs);
            let b = eval(env, *rhs);
            binary(*op, a, b)
        }
        ExprKind::Ternary { cond, then_, else_ } => match eval(env, *cond).truth() {
            Bit::One => eval(env, *then_),
            Bit::Zero => eval(env, *else_),
            _ => {
                let t = eval(env, *then_);
                let e = eval(env, *else_);
                match (t, e) {
                    (Value::Real(a), Value::Real(b)) if a == b => Value::Real(a),
                    (Value::Real(_), _) | (_, Value::Real(_)) => Value::Real(f64::NAN),
                    (t, e) => Value::Bits(merge_unknown(&t.to_logic(), &e.to_logic())),
                }
            }
        },
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => {
            let v = eval(env, *expr).to_logic();
            Value::Bits(v.resize(*width).with_signed(*signed))
        }
        ExprKind::MemRead { mem, addr } => {
            let width = flat_width(&node.ty);
            let a = eval(env, *addr).to_logic();
            let elem = match a.to_u64() {
                Some(a) => env.read_mem(*mem, a),
                None => None,
            };
            Value::Bits(elem.unwrap_or_else(|| Logic::x(width)))
        }
        ExprKind::Call { name, args } => {
            let vals: Vec<Value> = args.iter().map(|a| eval(env, *a)).collect();
            env.call(name.as_str(), vals, &node.ty, node.span)
        }
    }
}

/// Applies a unary operator.
pub(crate) fn unary(op: UnaryOp, v: Value) -> Value {
    if let Value::Real(r) = v {
        return match op {
            UnaryOp::Neg => Value::Real(-r),
            UnaryOp::LogicNot => Value::Bits(Logic::from_bool(r == 0.0)),
            _ => unary(op, Value::Bits(Value::Real(r).to_logic())),
        };
    }
    let l = v.to_logic();
    Value::Bits(match op {
        UnaryOp::Not => l.not(),
        UnaryOp::Neg => l.neg(),
        UnaryOp::ReduceAnd => l.reduce_and(),
        UnaryOp::ReduceOr => l.reduce_or(),
        UnaryOp::ReduceXor => l.reduce_xor(),
        UnaryOp::ReduceNand => l.reduce_nand(),
        UnaryOp::ReduceNor => l.reduce_nor(),
        UnaryOp::ReduceXnor => l.reduce_xnor(),
        UnaryOp::LogicNot => l.logical_not(),
    })
}

/// Applies a binary operator.
pub(crate) fn binary(op: BinaryOp, a: Value, b: Value) -> Value {
    match (&a, &b) {
        (Value::Str(x), Value::Str(y)) if matches!(op, BinaryOp::Eq | BinaryOp::CaseEq) => {
            return Value::Bits(Logic::from_bool(x == y));
        }
        (Value::Str(x), Value::Str(y)) if matches!(op, BinaryOp::Ne | BinaryOp::CaseNe) => {
            return Value::Bits(Logic::from_bool(x != y));
        }
        (Value::Real(_), _) | (_, Value::Real(_)) => {
            if let Some(v) = real_binary(op, a.to_real(), b.to_real()) {
                return v;
            }
        }
        _ => {}
    }
    Value::Bits(bits_binary(op, a.to_logic(), b.to_logic()))
}

/// Real arithmetic; `None` for operators that have no real form.
fn real_binary(op: BinaryOp, a: f64, b: f64) -> Option<Value> {
    let bit = |c: bool| Value::Bits(Logic::from_bool(c));
    Some(match op {
        BinaryOp::Add => Value::Real(a + b),
        BinaryOp::Sub => Value::Real(a - b),
        BinaryOp::Mul => Value::Real(a * b),
        BinaryOp::Div => Value::Real(a / b),
        BinaryOp::Mod => Value::Real(a % b),
        BinaryOp::Pow => Value::Real(a.powf(b)),
        BinaryOp::Eq | BinaryOp::CaseEq | BinaryOp::WildEq => bit(a == b),
        BinaryOp::Ne | BinaryOp::CaseNe => bit(a != b),
        BinaryOp::Lt => bit(a < b),
        BinaryOp::Le => bit(a <= b),
        BinaryOp::Gt => bit(a > b),
        BinaryOp::Ge => bit(a >= b),
        BinaryOp::LogicAnd => bit(a != 0.0 && b != 0.0),
        BinaryOp::LogicOr => bit(a != 0.0 || b != 0.0),
        _ => return None,
    })
}

/// Extends both operands to the wider width, each by its own signedness.
fn harmonize(a: Logic, b: Logic) -> (Logic, Logic) {
    let w = a.width().max(b.width());
    let a = if a.width() == w { a } else { a.resize(w) };
    let b = if b.width() == w { b } else { b.resize(w) };
    (a, b)
}

/// Applies a binary operator to bit vectors, sizing them defensively when
/// the IR was not explicitly sized.
pub(crate) fn bits_binary(op: BinaryOp, a: Logic, b: Logic) -> Logic {
    match op {
        BinaryOp::Shl => a.shl_by(&b),
        BinaryOp::Shr => a.shr_by(&b),
        BinaryOp::Sshr => a.sshr_by(&b),
        BinaryOp::Pow => a.pow(&b),
        BinaryOp::LogicAnd => a.logical_and(&b),
        BinaryOp::LogicOr => a.logical_or(&b),
        _ => {
            let (a, b) = harmonize(a, b);
            match op {
                BinaryOp::And => a.and(&b),
                BinaryOp::Or => a.or(&b),
                BinaryOp::Xor => a.xor(&b),
                BinaryOp::Xnor => a.xnor(&b),
                BinaryOp::Add => a.add(&b),
                BinaryOp::Sub => a.sub(&b),
                BinaryOp::Mul => a.mul(&b),
                BinaryOp::Div => a.div(&b),
                BinaryOp::Mod => a.rem(&b),
                BinaryOp::Eq => a.eq(&b),
                BinaryOp::Ne => a.ne(&b),
                BinaryOp::CaseEq => a.case_eq(&b),
                BinaryOp::CaseNe => a.case_ne(&b),
                BinaryOp::WildEq => a.wildcard_eq(&b),
                BinaryOp::Lt => a.lt(&b),
                BinaryOp::Le => a.le(&b),
                BinaryOp::Gt => a.gt(&b),
                BinaryOp::Ge => a.ge(&b),
                BinaryOp::Shl
                | BinaryOp::Shr
                | BinaryOp::Sshr
                | BinaryOp::Pow
                | BinaryOp::LogicAnd
                | BinaryOp::LogicOr => unreachable!("handled above"),
            }
        }
    }
}

/// The simulator as an evaluation environment for one instance.
pub(crate) struct SimEnv<'a, 'd> {
    /// The simulator.
    pub(crate) sim: &'a mut Simulator<'d>,
    /// The instance whose nets and memories are read.
    pub(crate) inst: InstId,
}

impl<'d> Env<'d> for SimEnv<'_, 'd> {
    fn module(&self) -> &'d Module {
        self.sim.instances[self.inst.idx()].m
    }

    fn read_net(&mut self, net: NetId) -> Value {
        let state = &self.sim.instances[self.inst.idx()];
        let Some(sig) = state.nets.get(net.index()) else {
            return Value::Bits(Logic::x(1));
        };
        let ty = &state.m.nets[net].ty;
        let v = self.sim.signals[sig.idx()].effective();
        match ty {
            Type::Real => Value::Real(v.to_u64().map_or(f64::NAN, f64::from_bits)),
            Type::Integer => Value::Bits(v.clone().as_signed()),
            Type::Bits { signed, .. } => Value::Bits(v.clone().with_signed(*signed)),
            Type::Array { .. } | Type::String => Value::Bits(v.clone()),
        }
    }

    fn read_mem(&mut self, mem: MemoryId, addr: u64) -> Option<Logic> {
        let mid = *self.sim.instances[self.inst.idx()].mems.get(mem.index())?;
        let index = usize::try_from(addr).ok()?;
        self.sim.memories[mid.idx()].data.get(index).cloned()
    }

    fn call(&mut self, name: &str, args: Vec<Value>, ty: &Type, span: Span) -> Value {
        self.sim.call_function(self.inst, name, args, ty, span)
    }
}

impl<'d> Simulator<'d> {
    /// Evaluates expression `e` of instance `inst`.
    pub(crate) fn eval(&mut self, inst: InstId, e: ExprId) -> Value {
        eval(&mut SimEnv { sim: self, inst }, e)
    }

    /// Evaluates `e` as a bit vector.
    pub(crate) fn eval_logic(&mut self, inst: InstId, e: ExprId) -> Logic {
        self.eval(inst, e).to_logic()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::ModuleBuilder;
    use crate::source::SourceMap;
    use std::collections::HashMap;

    struct TestEnv<'d> {
        m: &'d Module,
        nets: HashMap<NetId, Value>,
        mem: Vec<Logic>,
        calls: Vec<String>,
    }

    impl<'d> Env<'d> for TestEnv<'d> {
        fn module(&self) -> &'d Module {
            self.m
        }
        fn read_net(&mut self, net: NetId) -> Value {
            self.nets
                .get(&net)
                .cloned()
                .unwrap_or(Value::Bits(Logic::x(flat_width(&self.m.nets[net].ty))))
        }
        fn read_mem(&mut self, _mem: MemoryId, addr: u64) -> Option<Logic> {
            self.mem.get(usize::try_from(addr).ok()?).cloned()
        }
        fn call(&mut self, name: &str, args: Vec<Value>, ty: &Type, _span: Span) -> Value {
            self.calls.push(format!("{name}/{}", args.len()));
            Value::Bits(Logic::x(flat_width(ty)))
        }
    }

    fn l(s: &str) -> Logic {
        Logic::parse_verilog(s).unwrap()
    }

    fn span() -> Span {
        let mut map = SourceMap::new();
        let id = map.add("t", "").unwrap();
        Span::new(id, 0, 0)
    }

    #[test]
    fn evaluates_selects_and_operators() {
        let mut b = ModuleBuilder::new("m", span());
        let a = b.input("a", Type::bits(8));
        let i = b.input("i", Type::bits(4));
        let arr = b.input("arr", Type::array(Type::bits(4), 3));
        let r = b.input("r", Type::Real);
        let mem = b.memory("mem", Type::bits(8), 2);
        let av = b.net(a);
        let iv = b.net(i);
        let arrv = b.net(arr);
        let rv = b.net(r);
        let one = b.const_u64(8, 1);
        let sum = b.add(av, one);
        let sl = b.slice(av, 3, 0);
        let idx = b.index(av, iv);
        let isl = b.indexed_slice(av, iv, 4, true);
        let dsl = b.indexed_slice(av, iv, 4, false);
        let el = b.index(arrv, iv);
        let asl = b.slice(arrv, 2, 1);
        let cat = b.concat(vec![sl, sl]);
        let rep = b.replicate(2, sl);
        let neg = b.neg(av);
        let red = b.reduce_or(av);
        let lnot = b.lnot(av);
        let cond = b.eq(av, one);
        let tern = b.mux(cond, av, one);
        let xcond = b.index(av, iv);
        let xtern = b.mux(xcond, av, one);
        let rs = b.sext(sl, 8);
        let mr = b.mem_read(mem, iv);
        let call = b.call("$mystery", vec![av], Type::bits(3));
        let radd = b.add(rv, rv);
        let rlt = b.lt(rv, rv);
        let rneg = b.neg(rv);
        let s1 = b.string("ab");
        let s2 = b.string("ab");
        let seq = b.eq(s1, s2);
        let sne = b.ne(s1, s2);
        let shl = b.shl(av, iv);
        let pow = b.binary(BinaryOp::Pow, av, iv);
        let m = b.finish();
        let mut env = TestEnv {
            m: &m,
            nets: HashMap::new(),
            mem: vec![l("8'hab"), l("8'hcd")],
            calls: Vec::new(),
        };
        env.nets.insert(a, Value::Bits(l("8'b10110001")));
        env.nets.insert(i, Value::Bits(l("4'd2")));
        env.nets.insert(arr, Value::Bits(l("12'h321")));
        env.nets.insert(r, Value::Real(1.5));
        assert_eq!(eval(&mut env, sum), Value::Bits(l("8'b10110010")));
        assert_eq!(eval(&mut env, sl), Value::Bits(l("4'b0001")));
        assert_eq!(eval(&mut env, idx), Value::Bits(l("1'b0")));
        assert_eq!(eval(&mut env, isl), Value::Bits(l("4'b1100")));
        assert_eq!(eval(&mut env, dsl), Value::Bits(l("4'b001x")));
        assert_eq!(eval(&mut env, el), Value::Bits(l("4'h3")));
        assert_eq!(eval(&mut env, asl), Value::Bits(l("8'h32")));
        assert_eq!(eval(&mut env, cat), Value::Bits(l("8'h11")));
        assert_eq!(eval(&mut env, rep), Value::Bits(l("8'h11")));
        assert_eq!(eval(&mut env, neg), Value::Bits(l("8'b01001111")));
        assert_eq!(eval(&mut env, red), Value::Bits(l("1'b1")));
        assert_eq!(eval(&mut env, lnot), Value::Bits(l("1'b0")));
        assert_eq!(eval(&mut env, tern), Value::Bits(l("8'd1")));
        assert_eq!(eval(&mut env, rs), Value::Bits(l("8'd1").as_signed()));
        assert_eq!(eval(&mut env, mr), Value::Bits(l("8'hxx")));
        assert_eq!(eval(&mut env, call), Value::Bits(l("3'bxxx")));
        assert_eq!(env.calls, ["$mystery/1"]);
        assert_eq!(eval(&mut env, radd), Value::Real(3.0));
        assert_eq!(eval(&mut env, rlt), Value::Bits(l("1'b0")));
        assert_eq!(eval(&mut env, rneg), Value::Real(-1.5));
        assert_eq!(eval(&mut env, seq), Value::Bits(l("1'b1")));
        assert_eq!(eval(&mut env, sne), Value::Bits(l("1'b0")));
        assert_eq!(eval(&mut env, shl), Value::Bits(l("8'b11000100")));
        assert_eq!(
            eval(&mut env, pow),
            Value::Bits(l("8'b10110001").pow(&l("4'd2")))
        );
        // Out-of-range and unknown indices read x.
        env.nets.insert(i, Value::Bits(l("4'd9")));
        assert_eq!(eval(&mut env, idx), Value::Bits(l("1'bx")));
        assert_eq!(eval(&mut env, el), Value::Bits(l("4'hx")));
        assert_eq!(eval(&mut env, isl), Value::Bits(l("4'bxxxx")));
        env.nets.insert(i, Value::Bits(l("4'bxx00")));
        assert_eq!(eval(&mut env, xtern), Value::Bits(l("8'bx0xx0001")));
        assert_eq!(eval(&mut env, isl), Value::Bits(l("4'bxxxx")));
        env.nets.insert(i, Value::Bits(l("4'd1")));
        assert_eq!(eval(&mut env, mr), Value::Bits(l("8'hcd")));
        assert_eq!(eval(&mut env, ExprId(999)), Value::Bits(l("1'bx")));
    }

    #[test]
    fn real_and_unknown_branches() {
        assert_eq!(
            unary(UnaryOp::Not, Value::Real(1.0)),
            Value::Bits(Logic::from_i64(1, 64).not())
        );
        assert_eq!(
            unary(UnaryOp::LogicNot, Value::Real(0.0)),
            Value::Bits(l("1'b1"))
        );
        assert_eq!(
            binary(BinaryOp::And, Value::Real(3.0), Value::Real(1.0)),
            Value::Bits(Logic::from_i64(1, 64))
        );
        assert_eq!(
            binary(BinaryOp::Ge, Value::Real(3.0), Value::Bits(l("8'd2"))),
            Value::Bits(l("1'b1"))
        );
        assert_eq!(
            binary(BinaryOp::LogicAnd, Value::Real(3.0), Value::Real(0.0)),
            Value::Bits(l("1'b0"))
        );
        assert_eq!(
            binary(BinaryOp::LogicOr, Value::Real(3.0), Value::Real(0.0)),
            Value::Bits(l("1'b1"))
        );
        assert_eq!(
            binary(BinaryOp::Mod, Value::Real(7.0), Value::Real(4.0)),
            Value::Real(3.0)
        );
        assert_eq!(
            binary(BinaryOp::Pow, Value::Real(2.0), Value::Real(3.0)),
            Value::Real(8.0)
        );
        assert_eq!(
            binary(BinaryOp::Sub, Value::Real(2.0), Value::Real(3.0)),
            Value::Real(-1.0)
        );
        assert_eq!(
            binary(BinaryOp::Mul, Value::Real(2.0), Value::Real(3.0)),
            Value::Real(6.0)
        );
        assert_eq!(
            binary(BinaryOp::Div, Value::Real(6.0), Value::Real(3.0)),
            Value::Real(2.0)
        );
        for (op, want) in [
            (BinaryOp::Eq, true),
            (BinaryOp::Ne, false),
            (BinaryOp::Le, true),
            (BinaryOp::Gt, false),
        ] {
            assert_eq!(
                binary(op, Value::Real(1.0), Value::Real(1.0)),
                Value::Bits(Logic::from_bool(want))
            );
        }
        // Mixed widths are harmonised instead of panicking.
        assert_eq!(
            bits_binary(BinaryOp::Add, l("4'd15"), l("8'd1")),
            l("8'd16")
        );
        assert_eq!(
            bits_binary(BinaryOp::Xnor, l("2'b10"), l("2'b11")),
            l("2'b10")
        );
        assert_eq!(
            bits_binary(BinaryOp::Or, l("2'b10"), l("2'b01")),
            l("2'b11")
        );
        assert_eq!(
            bits_binary(BinaryOp::Xor, l("2'b10"), l("2'b11")),
            l("2'b01")
        );
        assert_eq!(bits_binary(BinaryOp::Mul, l("4'd3"), l("4'd5")), l("4'd15"));
        assert_eq!(bits_binary(BinaryOp::Div, l("4'd7"), l("4'd2")), l("4'd3"));
        assert_eq!(bits_binary(BinaryOp::Mod, l("4'd7"), l("4'd2")), l("4'd1"));
        assert_eq!(bits_binary(BinaryOp::Shr, l("4'd8"), l("2'd1")), l("4'd4"));
        assert_eq!(
            bits_binary(BinaryOp::Sshr, l("4'd8").as_signed(), l("2'd1")),
            l("4'b1100").as_signed()
        );
        assert_eq!(
            bits_binary(BinaryOp::CaseEq, l("2'bx1"), l("2'bx1")),
            l("1'b1")
        );
        assert_eq!(
            bits_binary(BinaryOp::CaseNe, l("2'bx1"), l("2'bx1")),
            l("1'b0")
        );
        assert_eq!(
            bits_binary(BinaryOp::WildEq, l("2'b01"), l("2'bx1")),
            l("1'b1")
        );
        assert_eq!(bits_binary(BinaryOp::Ne, l("2'b01"), l("2'b11")), l("1'b1"));
        assert_eq!(bits_binary(BinaryOp::Lt, l("2'b01"), l("2'b11")), l("1'b1"));
        assert_eq!(bits_binary(BinaryOp::Gt, l("2'b01"), l("2'b11")), l("1'b0"));
        assert_eq!(bits_binary(BinaryOp::Ge, l("2'b01"), l("2'b11")), l("1'b0"));
        assert_eq!(bits_binary(BinaryOp::Le, l("2'b01"), l("2'b11")), l("1'b1"));
        assert_eq!(
            bits_binary(BinaryOp::LogicOr, l("2'b00"), l("2'b10")),
            l("1'b1")
        );
        assert_eq!(index_value(&l("4'bx000")), None);
        assert_eq!(index_value(&Logic::from_i64(-1, 4)), Some(-1));
        assert_eq!(index_value(&Logic::ones(70)), Some(i64::MAX));
    }
}
