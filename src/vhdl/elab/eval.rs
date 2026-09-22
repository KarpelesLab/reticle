//! Static evaluation with the generics bound.
//!
//! [`crate::vhdl::sema`] folds everything that is *locally* static and
//! records it in `Analysis::value_of`. Elaboration has to fold a little
//! more: an expression that mentions a generic is not locally static, yet
//! it must be a number by the time a net is created
//! (`std_logic_vector(WIDTH-1 downto 0)`), a generate is unrolled
//! (`for i in 0 to N-1 generate`) or an `if ... generate` is taken.
//!
//! [`Lowerer::eval`] is that second evaluator. It asks the analyser first,
//! so nothing static is recomputed, and otherwise walks the expression
//! using the same side tables the lowering pass uses: `decl_of` to find
//! the generic or constant a name denotes, `call_of` to learn which
//! operator an `+` is, and `type_of` to know what the operands are. A
//! value it cannot produce is `None`, and the caller turns that into
//! `V0704` with the span of the offending expression.

use crate::vhdl::ast::{self, BinaryOp, UnaryOp};
use crate::vhdl::sema::constant::{ArrayValue, parse_integer_literal, parse_real_literal};
use crate::vhdl::sema::{
    CallTarget, DeclKind, ObjectClass, TypeClass, TypeId, Value, constant::int,
};

use super::lower::{Binding, Lowerer};

impl<'a> Lowerer<'a, '_> {
    /// Evaluates `e` with the generics and constants in scope, or `None`
    /// when it is not static after elaboration.
    pub(crate) fn eval(&mut self, e: &'a ast::Expr) -> Option<Value> {
        // The analyser records a value for a signal name when the
        // declaration gave it an initial value; that is its value at time
        // zero, not a static one, so an expression that reads a signal or
        // a variable is never folded here.
        if !self.reads_object(e)
            && let Some(v) = self.a().value_of(e.span())
        {
            return Some(v.clone());
        }
        match e {
            ast::Expr::Paren { inner, .. } => self.eval(inner),
            ast::Expr::Qualified { operand, .. } => self.eval(operand),
            ast::Expr::Literal(l) => self.eval_literal(l),
            ast::Expr::Name(n) => self.eval_name(n),
            ast::Expr::Unary { op, operand, span } => {
                let v = self.eval(operand)?;
                let ty = self.a().type_of(operand.span())?;
                self.fold_unary(*op, &v, ty, *span)
            }
            ast::Expr::Binary {
                op, lhs, rhs, span, ..
            } => {
                let l = self.eval(lhs)?;
                let r = self.eval(rhs)?;
                let lt = self.a().type_of(lhs.span())?;
                self.fold_binary(*op, &l, &r, lt, *span)
            }
            ast::Expr::Aggregate(_) | ast::Expr::Allocator { .. } => None,
            ast::Expr::Open(_) | ast::Expr::Error(_) => None,
        }
    }

    /// True when an expression reads a signal or a variable, so its value
    /// is not fixed at elaboration.
    pub(crate) fn reads_object(&mut self, e: &'a ast::Expr) -> bool {
        match e {
            ast::Expr::Paren { inner, .. } => self.reads_object(inner),
            ast::Expr::Qualified { operand, .. } => self.reads_object(operand),
            ast::Expr::Name(n) => self.name_reads_object(n),
            ast::Expr::Unary { operand, .. } => self.reads_object(operand),
            ast::Expr::Binary { lhs, rhs, .. } => self.reads_object(lhs) || self.reads_object(rhs),
            ast::Expr::Aggregate(ag) => {
                for el in &ag.elements {
                    if self.reads_object(&el.value) {
                        return true;
                    }
                }
                false
            }
            ast::Expr::Allocator { .. } => true,
            ast::Expr::Literal(_) | ast::Expr::Open(_) | ast::Expr::Error(_) => false,
        }
    }

    fn name_reads_object(&mut self, n: &'a ast::Name) -> bool {
        if let Some(d) = self.a().decl_of(n.span())
            && matches!(
                self.lookup(d),
                Some(Binding::Net { .. } | Binding::Mem { .. } | Binding::Slice { .. })
            )
        {
            return true;
        }
        match n {
            ast::Name::Selected { prefix, .. } | ast::Name::Slice { prefix, .. } => {
                self.name_reads_object(prefix)
            }
            ast::Name::Attribute {
                prefix, attribute, ..
            } => {
                // The bound attributes of an object depend on its subtype,
                // not on its value; the signal attributes do depend on it.
                if is_bound_attribute(&attribute.name) {
                    false
                } else {
                    self.name_reads_object(prefix)
                }
            }
            ast::Name::Call { prefix, args, .. } => {
                if self.name_reads_object(prefix) {
                    return true;
                }
                for a in args {
                    if let ast::Actual::Expr(e) | ast::Actual::Inertial(e) = &a.actual
                        && self.reads_object(e)
                    {
                        return true;
                    }
                }
                false
            }
            ast::Name::External(_) => true,
            _ => false,
        }
    }

    /// Evaluates `e` as an integer.
    pub(crate) fn eval_int(&mut self, e: &'a ast::Expr) -> Option<i128> {
        self.eval(e)?.as_int()
    }

    fn eval_literal(&mut self, l: &ast::Literal) -> Option<Value> {
        match &l.kind {
            ast::LiteralKind::Integer(t) => parse_integer_literal(t).map(Value::Int),
            ast::LiteralKind::Real(t) => parse_real_literal(t).map(Value::Real),
            ast::LiteralKind::String(s) => {
                Some(Value::string(s.chars().map(|c| u32::from(c as u8))))
            }
            _ => None,
        }
    }

    fn eval_name(&mut self, n: &'a ast::Name) -> Option<Value> {
        if let Some(d) = self.a().decl_of(n.span()) {
            match self.lookup(d) {
                Some(Binding::Value { value, .. }) => return Some(value.clone()),
                // A signal or variable has no value at elaboration, even
                // when its declaration gave it an initial one.
                Some(Binding::Net { .. } | Binding::Mem { .. } | Binding::Slice { .. }) => {
                    return None;
                }
                None => {}
            }
            match self.a().decl(d).kind {
                // Only a constant's declared value is a value; a signal's
                // initialiser is its value at time zero, not for ever.
                DeclKind::Object {
                    class: ObjectClass::Constant,
                    ..
                } => {
                    if let Some(v) = self.a().decl_value(d) {
                        return Some(v.clone());
                    }
                }
                DeclKind::EnumLiteral { pos, .. } => return Some(Value::Enum(pos)),
                _ => {}
            }
        }
        match n {
            ast::Name::Call { prefix, args, span } => match self.a().call_of(*span).cloned() {
                Some(CallTarget::Index) => {
                    let base = self.eval_name(prefix)?;
                    let arr = base.as_array()?.clone();
                    let arg = args.first()?;
                    let ast::Actual::Expr(ie) = &arg.actual else {
                        return None;
                    };
                    let i = self.eval_int(ie)?;
                    arr.get(i).cloned()
                }
                Some(CallTarget::Conversion(t)) => {
                    let arg = args.first()?;
                    let ast::Actual::Expr(ie) = &arg.actual else {
                        return None;
                    };
                    let v = self.eval(ie)?;
                    self.convert_value(&v, t)
                }
                _ => None,
            },
            ast::Name::Slice {
                prefix,
                range,
                span,
            } => {
                let _ = span;
                let base = self.eval_name(prefix)?;
                let arr = base.as_array()?.clone();
                let info = self.a().range_of(range.span())?.clone();
                let (l, r) = info.bounds.ints()?;
                let mut elems = Vec::new();
                let mut i = l;
                loop {
                    elems.push(arr.get(i)?.clone());
                    if i == r {
                        break;
                    }
                    i += match info.bounds.dir {
                        ast::Direction::To => 1,
                        ast::Direction::Downto => -1,
                    };
                }
                Some(Value::Array(ArrayValue {
                    left: l,
                    dir: info.bounds.dir,
                    elems,
                }))
            }
            _ => None,
        }
    }

    /// A type conversion between numeric classes.
    fn convert_value(&mut self, v: &Value, to: TypeId) -> Option<Value> {
        let class = self.a().class(to);
        match (class, v) {
            (TypeClass::Integer | TypeClass::UniversalInteger, Value::Real(r)) => {
                Some(Value::Int(real_to_int(*r)))
            }
            (TypeClass::Real | TypeClass::UniversalReal, Value::Int(i)) => {
                Some(Value::Real(*i as f64))
            }
            _ => Some(v.clone()),
        }
    }

    fn fold_unary(
        &mut self,
        op: UnaryOp,
        v: &Value,
        ty: TypeId,
        span: crate::source::Span,
    ) -> Option<Value> {
        let _ = span;
        match op {
            UnaryOp::Plus => Some(v.clone()),
            UnaryOp::Minus => match v {
                Value::Int(i) => Some(Value::Int(-i)),
                Value::Real(r) => Some(Value::Real(-r)),
                _ => None,
            },
            UnaryOp::Abs => match v {
                Value::Int(i) => Some(Value::Int(i.abs())),
                Value::Real(r) => Some(Value::Real(r.abs())),
                _ => None,
            },
            UnaryOp::Not => self.map_logic(v, ty, |b| !b),
            UnaryOp::Condition => v.as_bool().map(Value::from_bool),
            UnaryOp::And | UnaryOp::Nand => {
                let bits = self.logic_bits(v)?;
                let r = bits.iter().all(|b| *b);
                Some(Value::from_bool(if op == UnaryOp::And { r } else { !r }))
            }
            UnaryOp::Or | UnaryOp::Nor => {
                let bits = self.logic_bits(v)?;
                let r = bits.iter().any(|b| *b);
                Some(Value::from_bool(if op == UnaryOp::Or { r } else { !r }))
            }
            UnaryOp::Xor | UnaryOp::Xnor => {
                let bits = self.logic_bits(v)?;
                let r = bits.iter().filter(|b| **b).count() % 2 == 1;
                Some(Value::from_bool(if op == UnaryOp::Xor { r } else { !r }))
            }
        }
    }

    fn fold_binary(
        &mut self,
        op: BinaryOp,
        l: &Value,
        r: &Value,
        lty: TypeId,
        span: crate::source::Span,
    ) -> Option<Value> {
        let _ = span;
        use BinaryOp as B;
        match op {
            B::Add | B::Sub | B::Mul | B::Div | B::Mod | B::Rem | B::Pow => {
                self.fold_arith(op, l, r)
            }
            B::Eq => Some(Value::from_bool(l == r)),
            B::Neq => Some(Value::from_bool(l != r)),
            B::Lt | B::Le | B::Gt | B::Ge => {
                let o = l.compare(r)?;
                Some(Value::from_bool(match op {
                    B::Lt => o.is_lt(),
                    B::Le => o.is_le(),
                    B::Gt => o.is_gt(),
                    _ => o.is_ge(),
                }))
            }
            B::And | B::Or | B::Nand | B::Nor | B::Xor | B::Xnor => {
                self.fold_logical(op, l, r, lty)
            }
            B::Concat => fold_concat(l, r),
            _ => None,
        }
    }

    fn fold_arith(&mut self, op: BinaryOp, l: &Value, r: &Value) -> Option<Value> {
        use BinaryOp as B;
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                let v = match op {
                    B::Add => a.checked_add(*b)?,
                    B::Sub => a.checked_sub(*b)?,
                    B::Mul => a.checked_mul(*b)?,
                    B::Div => a.checked_div(*b)?,
                    B::Mod => int::modulo(*a, *b)?,
                    B::Rem => int::remainder(*a, *b)?,
                    B::Pow => int::power(*a, *b)?,
                    _ => return None,
                };
                Some(Value::Int(v))
            }
            (Value::Real(a), Value::Real(b)) => Some(Value::Real(match op {
                B::Add => a + b,
                B::Sub => a - b,
                B::Mul => a * b,
                B::Div => a / b,
                B::Pow => a.powf(*b),
                _ => return None,
            })),
            // `time * integer` and `real * integer` mix classes.
            (Value::Int(a), Value::Real(b)) => Some(Value::Real(match op {
                B::Mul => *a as f64 * b,
                _ => return None,
            })),
            (Value::Real(a), Value::Int(b)) => Some(match op {
                B::Mul => Value::Real(a * *b as f64),
                B::Div => Value::Real(a / *b as f64),
                B::Pow => Value::Real(a.powi(i32::try_from(*b).ok()?)),
                _ => return None,
            }),
            _ => None,
        }
    }

    fn fold_logical(&mut self, op: BinaryOp, l: &Value, r: &Value, lty: TypeId) -> Option<Value> {
        use BinaryOp as B;
        let apply = |a: bool, b: bool| match op {
            B::And => a && b,
            B::Or => a || b,
            B::Nand => !(a && b),
            B::Nor => !(a || b),
            B::Xor => a != b,
            _ => a == b,
        };
        match (l, r) {
            (Value::Enum(a), Value::Enum(b)) => Some(Value::from_bool(apply(*a != 0, *b != 0))),
            (Value::Array(a), Value::Array(b)) if a.elems.len() == b.elems.len() => {
                let _ = lty;
                let elems = a
                    .elems
                    .iter()
                    .zip(&b.elems)
                    .map(|(x, y)| {
                        let (x, y) = (x.as_enum()?, y.as_enum()?);
                        Some(Value::Enum(u32::from(apply(x != 0, y != 0))))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(Value::Array(ArrayValue {
                    left: a.left,
                    dir: a.dir,
                    elems,
                }))
            }
            _ => None,
        }
    }

    /// The bits of a `bit` or `std_ulogic` array value.
    fn logic_bits(&mut self, v: &Value) -> Option<Vec<bool>> {
        let a = v.as_array()?;
        a.elems.iter().map(|e| e.as_bool()).collect()
    }

    /// Applies `f` to every bit of a scalar or array logic value.
    fn map_logic(
        &mut self,
        v: &Value,
        _ty: TypeId,
        f: impl Fn(bool) -> bool + Copy,
    ) -> Option<Value> {
        match v {
            Value::Enum(p) => Some(Value::from_bool(f(*p != 0))),
            Value::Array(a) => {
                let elems = a
                    .elems
                    .iter()
                    .map(|e| Some(Value::Enum(u32::from(f(e.as_enum()? != 0)))))
                    .collect::<Option<Vec<_>>>()?;
                Some(Value::Array(ArrayValue {
                    left: a.left,
                    dir: a.dir,
                    elems,
                }))
            }
            _ => None,
        }
    }
}

/// Rounds a real to an integer the way a VHDL type conversion does,
/// clamped to the range an `i64` holds.
fn real_to_int(r: f64) -> i128 {
    let v = r.round().clamp(-9.0e18, 9.0e18);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "clamped into the i64 range first"
    )]
    let n = v as i64;
    i128::from(n)
}

/// True for the attributes whose value follows from a subtype rather
/// than from a signal's history.
fn is_bound_attribute(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "left"
            | "right"
            | "high"
            | "low"
            | "length"
            | "ascending"
            | "range"
            | "reverse_range"
            | "base"
            | "subtype"
            | "element"
            | "pos"
            | "val"
            | "succ"
            | "pred"
            | "leftof"
            | "rightof"
            | "image"
            | "value"
    )
}

/// `a & b` on arrays and elements.
fn fold_concat(l: &Value, r: &Value) -> Option<Value> {
    let mut elems = Vec::new();
    let dir = match l {
        Value::Array(a) => a.dir,
        _ => ast::Direction::To,
    };
    match l {
        Value::Array(a) => elems.extend(a.elems.iter().cloned()),
        other => elems.push(other.clone()),
    }
    match r {
        Value::Array(a) => elems.extend(a.elems.iter().cloned()),
        other => elems.push(other.clone()),
    }
    let n = i128::try_from(elems.len()).ok()?;
    Some(Value::Array(ArrayValue {
        left: match dir {
            ast::Direction::To => 0,
            ast::Direction::Downto => n - 1,
        },
        dir,
        elems,
    }))
}
