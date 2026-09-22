//! Expressions: arena-allocated, typed, spanned trees.
//!
//! Every module owns an expression arena ([`super::Module::exprs`]) and
//! refers to expressions by [`ExprId`] from assignments, cell inputs,
//! instance connections and statements. Nodes are immutable values with a
//! cached [`Type`]; sharing a node between several users is allowed, and
//! passes that rewrite expressions push new nodes rather than editing old
//! ones in place, so ids held elsewhere stay meaningful.
//!
//! # Type rules
//!
//! The IR is explicitly sized: a frontend inserts [`ExprKind::Resize`]
//! wherever its language's context-determined widths demand, and the
//! operators below have simple, fixed rules that [`super::validate`]
//! enforces. [`infer_type`] states them; in short:
//!
//! | Operator                              | Operands                 | Result           |
//! |---------------------------------------|--------------------------|------------------|
//! | `Not`, `Neg`                          | bits                     | same type        |
//! | `Reduce*`, `LogicNot`                 | bits                     | `u1`             |
//! | `And` `Or` `Xor` `Xnor`               | bits of equal width      | same width       |
//! | `Add` `Sub` `Mul` `Div` `Mod` `Pow`   | bits of equal width      | same width       |
//! | `Shl` `Shr` `Sshr`                    | bits, any-width amount   | type of the left |
//! | `Eq` `Ne` `CaseEq` `CaseNe` `WildEq`  | bits of equal width      | `u1`             |
//! | `Lt` `Le` `Gt` `Ge`                   | bits of equal width      | `u1`             |
//! | `LogicAnd` `LogicOr`                  | `u1`, `u1`               | `u1`             |
//! | `Ternary`                             | `u1`, `T`, `T`           | `T`              |
//! | `Concat`                              | bits                     | sum of widths    |
//! | `Replicate`                           | bits                     | count × width    |
//! | `Slice`, `IndexedSlice`               | bits                     | unsigned, width  |
//! | `Index`                               | bits or array            | `u1` or element  |
//! | `Resize`                              | bits or integer          | as requested     |
//! | `MemRead`                             |                          | element type     |
//!
//! A result is signed only when every bit-vector operand is signed, as in
//! Verilog. `Integer` and `Real` operands are accepted by the arithmetic,
//! comparison and negation rules when both sides have the same type.
//! [`ExprKind::Call`] has no rule: its cached type is authoritative.

use super::Name;
use super::arena::define_id;
use super::design::{MemoryId, Module, NetId};
use super::types::{Const, Type};
use crate::source::Span;

define_id!(
    /// Identifies an expression node inside a module's expression arena.
    ExprId,
    "e"
);

/// A unary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// Bitwise complement.
    Not,
    /// Two's complement negation.
    Neg,
    /// AND of all bits.
    ReduceAnd,
    /// OR of all bits.
    ReduceOr,
    /// XOR of all bits.
    ReduceXor,
    /// NAND of all bits.
    ReduceNand,
    /// NOR of all bits.
    ReduceNor,
    /// XNOR of all bits.
    ReduceXnor,
    /// Logical negation: 1 when the operand is all zero.
    LogicNot,
}

impl UnaryOp {
    /// Every operator, in a fixed order.
    pub const ALL: [UnaryOp; 9] = [
        UnaryOp::Not,
        UnaryOp::Neg,
        UnaryOp::ReduceAnd,
        UnaryOp::ReduceOr,
        UnaryOp::ReduceXor,
        UnaryOp::ReduceNand,
        UnaryOp::ReduceNor,
        UnaryOp::ReduceXnor,
        UnaryOp::LogicNot,
    ];

    /// The operator's name in the text format.
    pub fn name(self) -> &'static str {
        match self {
            UnaryOp::Not => "not",
            UnaryOp::Neg => "neg",
            UnaryOp::ReduceAnd => "rand",
            UnaryOp::ReduceOr => "ror",
            UnaryOp::ReduceXor => "rxor",
            UnaryOp::ReduceNand => "rnand",
            UnaryOp::ReduceNor => "rnor",
            UnaryOp::ReduceXnor => "rxnor",
            UnaryOp::LogicNot => "lnot",
        }
    }

    /// The operator with the given text-format name.
    pub fn from_name(name: &str) -> Option<UnaryOp> {
        UnaryOp::ALL.into_iter().find(|op| op.name() == name)
    }

    /// True for the operators whose result is a single bit.
    pub fn is_reduction(self) -> bool {
        !matches!(self, UnaryOp::Not | UnaryOp::Neg)
    }
}

/// A binary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// Bitwise AND.
    And,
    /// Bitwise OR.
    Or,
    /// Bitwise XOR.
    Xor,
    /// Bitwise XNOR.
    Xnor,
    /// Logical AND of two single bits.
    LogicAnd,
    /// Logical OR of two single bits.
    LogicOr,
    /// Addition, modulo the width.
    Add,
    /// Subtraction, modulo the width.
    Sub,
    /// Multiplication, modulo the width.
    Mul,
    /// Division, truncating towards zero.
    Div,
    /// Remainder, with the sign of the dividend.
    Mod,
    /// Exponentiation, modulo the width.
    Pow,
    /// Shift left, zero filling.
    Shl,
    /// Shift right, zero filling.
    Shr,
    /// Arithmetic shift right, sign filling.
    Sshr,
    /// Equality; `x` on any unknown bit.
    Eq,
    /// Inequality; `x` on any unknown bit.
    Ne,
    /// Case equality: `x` and `z` compare as themselves.
    CaseEq,
    /// Case inequality.
    CaseNe,
    /// Wildcard equality: `x`/`z` bits of the right operand match anything.
    WildEq,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
}

impl BinaryOp {
    /// Every operator, in a fixed order.
    pub const ALL: [BinaryOp; 24] = [
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Xor,
        BinaryOp::Xnor,
        BinaryOp::LogicAnd,
        BinaryOp::LogicOr,
        BinaryOp::Add,
        BinaryOp::Sub,
        BinaryOp::Mul,
        BinaryOp::Div,
        BinaryOp::Mod,
        BinaryOp::Pow,
        BinaryOp::Shl,
        BinaryOp::Shr,
        BinaryOp::Sshr,
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::CaseEq,
        BinaryOp::CaseNe,
        BinaryOp::WildEq,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
    ];

    /// The operator's name in the text format.
    pub fn name(self) -> &'static str {
        match self {
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::Xor => "xor",
            BinaryOp::Xnor => "xnor",
            BinaryOp::LogicAnd => "land",
            BinaryOp::LogicOr => "lor",
            BinaryOp::Add => "add",
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "mul",
            BinaryOp::Div => "div",
            BinaryOp::Mod => "mod",
            BinaryOp::Pow => "pow",
            BinaryOp::Shl => "shl",
            BinaryOp::Shr => "shr",
            BinaryOp::Sshr => "sshr",
            BinaryOp::Eq => "eq",
            BinaryOp::Ne => "ne",
            BinaryOp::CaseEq => "ceq",
            BinaryOp::CaseNe => "cne",
            BinaryOp::WildEq => "weq",
            BinaryOp::Lt => "lt",
            BinaryOp::Le => "le",
            BinaryOp::Gt => "gt",
            BinaryOp::Ge => "ge",
        }
    }

    /// The operator with the given text-format name.
    pub fn from_name(name: &str) -> Option<BinaryOp> {
        BinaryOp::ALL.into_iter().find(|op| op.name() == name)
    }

    /// True for the bitwise operators (`And`, `Or`, `Xor`, `Xnor`).
    pub fn is_bitwise(self) -> bool {
        matches!(
            self,
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::Xnor
        )
    }

    /// True for the arithmetic operators (`Add` … `Pow`).
    pub fn is_arith(self) -> bool {
        matches!(
            self,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::Pow
        )
    }

    /// True for the shift operators.
    pub fn is_shift(self) -> bool {
        matches!(self, BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Sshr)
    }

    /// True for the operators that yield one bit: comparisons and the
    /// logical connectives.
    pub fn is_predicate(self) -> bool {
        !(self.is_bitwise() || self.is_arith() || self.is_shift())
    }

    /// True for `LogicAnd` and `LogicOr`.
    pub fn is_logical(self) -> bool {
        matches!(self, BinaryOp::LogicAnd | BinaryOp::LogicOr)
    }

    /// True for the relational operators `Lt` `Le` `Gt` `Ge`.
    pub fn is_relational(self) -> bool {
        matches!(
            self,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        )
    }
}

/// The payload of an expression node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// A literal bit vector.
    Const(Const),
    /// A string literal, for system call and report arguments.
    String(String),
    /// The value of a net.
    Net(NetId),
    /// A constant part-select `base[hi:lo]`, inclusive on both ends.
    Slice {
        /// The vector (or array) being sliced.
        base: ExprId,
        /// Index of the most significant bit kept.
        hi: u32,
        /// Index of the least significant bit kept.
        lo: u32,
    },
    /// A variable bit- or element-select `base[index]`.
    Index {
        /// The vector or array being indexed.
        base: ExprId,
        /// The index; an out-of-range value reads as `x`.
        index: ExprId,
    },
    /// A variable part-select `base[offset +: width]` (`up`) or
    /// `base[offset -: width]` (not `up`).
    IndexedSlice {
        /// The vector being sliced.
        base: ExprId,
        /// The starting bit index.
        offset: ExprId,
        /// Number of bits selected.
        width: u32,
        /// True when the slice extends towards the most significant bit.
        up: bool,
    },
    /// Concatenation; the first element becomes the most significant bits.
    Concat(Vec<ExprId>),
    /// `count` copies of `expr` concatenated.
    Replicate {
        /// Number of copies.
        count: u32,
        /// The vector to repeat.
        expr: ExprId,
    },
    /// A unary operator applied to `expr`.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        expr: ExprId,
    },
    /// A binary operator applied to `lhs` and `rhs`.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: ExprId,
        /// Right operand.
        rhs: ExprId,
    },
    /// `cond ? then_ : else_`; an `x` condition merges the two branches.
    Ternary {
        /// Single-bit condition.
        cond: ExprId,
        /// Value when the condition is 1.
        then_: ExprId,
        /// Value when the condition is 0.
        else_: ExprId,
    },
    /// Width change: truncation or extension, sign extending when `signed`
    /// and the operand is signed.
    Resize {
        /// The operand.
        expr: ExprId,
        /// The resulting width.
        width: u32,
        /// Signedness of the result; also selects sign extension.
        signed: bool,
    },
    /// An asynchronous read of one memory element.
    MemRead {
        /// The memory.
        mem: MemoryId,
        /// The element address.
        addr: ExprId,
    },
    /// A call to a function that has not been lowered (user functions before
    /// inlining, system functions such as `$clog2`). The node's cached type
    /// is the declared return type.
    Call {
        /// The function name as written in the source.
        name: Name,
        /// Arguments in call order.
        args: Vec<ExprId>,
    },
}

/// One expression node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// The operator or leaf.
    pub kind: ExprKind,
    /// The type of the value, cached at construction. [`infer_type`]
    /// recomputes it from the operands; the two must agree for every node
    /// except `Call`, which the validator checks.
    pub ty: Type,
    /// Where the expression came from.
    pub span: Span,
}

impl Expr {
    /// Builds a node with an explicit type.
    pub fn new(kind: ExprKind, ty: Type, span: Span) -> Self {
        Expr { kind, ty, span }
    }

    /// The width of the value when it is a bit vector.
    pub fn width(&self) -> Option<u32> {
        self.ty.width()
    }

    /// The constant held by a `Const` node.
    pub fn as_const(&self) -> Option<&Const> {
        match &self.kind {
            ExprKind::Const(c) => Some(c),
            _ => None,
        }
    }

    /// The net read by a `Net` node.
    pub fn as_net(&self) -> Option<NetId> {
        match &self.kind {
            ExprKind::Net(n) => Some(*n),
            _ => None,
        }
    }
}

/// Why [`infer_type`] could not assign a type to a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    /// An operand id does not exist in the module.
    DanglingExpr(ExprId),
    /// A net id does not exist in the module.
    DanglingNet(NetId),
    /// A memory id does not exist in the module.
    DanglingMemory(MemoryId),
    /// An operand has a type the operator does not accept.
    BadOperand {
        /// Which operand, counting from zero.
        index: usize,
        /// Its type.
        found: Type,
        /// What the rule wanted, in words.
        expected: &'static str,
    },
    /// Two operands that must agree do not.
    Mismatch {
        /// Type of the first operand.
        lhs: Type,
        /// Type of the second operand.
        rhs: Type,
    },
    /// A constant selection lies outside the operand.
    OutOfRange {
        /// The width or length of the operand.
        width: u64,
    },
    /// The node kind has no inference rule (`Call`).
    NoRule,
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeError::DanglingExpr(id) => write!(f, "expression {id} does not exist"),
            TypeError::DanglingNet(id) => write!(f, "net {id} does not exist"),
            TypeError::DanglingMemory(id) => write!(f, "memory {id} does not exist"),
            TypeError::BadOperand {
                index,
                found,
                expected,
            } => write!(f, "operand {index} is `{found}`, expected {expected}"),
            TypeError::Mismatch { lhs, rhs } => {
                write!(f, "operand types `{lhs}` and `{rhs}` do not match")
            }
            TypeError::OutOfRange { width } => {
                write!(f, "selection is outside the operand of width {width}")
            }
            TypeError::NoRule => f.write_str("the type of a call cannot be inferred"),
        }
    }
}

/// Computes the type of a node from its operands, following the table in
/// the module documentation.
pub fn infer_type(module: &Module, kind: &ExprKind) -> Result<Type, TypeError> {
    let ty = |id: ExprId| -> Result<&Type, TypeError> {
        module
            .exprs
            .get(id)
            .map(|e| &e.ty)
            .ok_or(TypeError::DanglingExpr(id))
    };
    let bits = |index: usize, id: ExprId| -> Result<(u32, bool), TypeError> {
        match ty(id)? {
            Type::Bits { width, signed } => Ok((*width, *signed)),
            found => Err(TypeError::BadOperand {
                index,
                found: found.clone(),
                expected: "a bit vector",
            }),
        }
    };
    match kind {
        ExprKind::Const(c) => Ok(c.ty()),
        ExprKind::String(_) => Ok(Type::String),
        ExprKind::Net(net) => module
            .nets
            .get(*net)
            .map(|n| n.ty.clone())
            .ok_or(TypeError::DanglingNet(*net)),
        ExprKind::Slice { base, hi, lo } => {
            let len = u64::from(*hi) - u64::from(*lo) + 1;
            match ty(*base)? {
                Type::Bits { width, .. } => {
                    if hi < lo || u64::from(*hi) >= u64::from(*width) {
                        return Err(TypeError::OutOfRange {
                            width: u64::from(*width),
                        });
                    }
                    Ok(Type::bits(*hi - *lo + 1))
                }
                Type::Array { elem, len: n } => {
                    if hi < lo || u64::from(*hi) >= *n {
                        return Err(TypeError::OutOfRange { width: *n });
                    }
                    Ok(Type::Array {
                        elem: elem.clone(),
                        len,
                    })
                }
                found => Err(TypeError::BadOperand {
                    index: 0,
                    found: found.clone(),
                    expected: "a bit vector or array",
                }),
            }
        }
        ExprKind::Index { base, index } => {
            bits(1, *index)?;
            match ty(*base)? {
                Type::Bits { .. } => Ok(Type::bit()),
                Type::Array { elem, .. } => Ok((**elem).clone()),
                found => Err(TypeError::BadOperand {
                    index: 0,
                    found: found.clone(),
                    expected: "a bit vector or array",
                }),
            }
        }
        ExprKind::IndexedSlice {
            base,
            offset,
            width,
            ..
        } => {
            let (base_width, _) = bits(0, *base)?;
            bits(1, *offset)?;
            if *width > base_width {
                return Err(TypeError::OutOfRange {
                    width: u64::from(base_width),
                });
            }
            Ok(Type::bits(*width))
        }
        ExprKind::Concat(parts) => {
            let mut total = 0u32;
            for (i, part) in parts.iter().enumerate() {
                let (w, _) = bits(i, *part)?;
                total = total.saturating_add(w);
            }
            Ok(Type::bits(total))
        }
        ExprKind::Replicate { count, expr } => {
            let (w, _) = bits(0, *expr)?;
            Ok(Type::bits(w.saturating_mul(*count)))
        }
        ExprKind::Unary { op, expr } => {
            let operand = ty(*expr)?;
            match op {
                UnaryOp::Not => match operand {
                    Type::Bits { .. } => Ok(operand.clone()),
                    found => Err(TypeError::BadOperand {
                        index: 0,
                        found: found.clone(),
                        expected: "a bit vector",
                    }),
                },
                UnaryOp::Neg => match operand {
                    Type::Bits { .. } | Type::Integer | Type::Real => Ok(operand.clone()),
                    found => Err(TypeError::BadOperand {
                        index: 0,
                        found: found.clone(),
                        expected: "a bit vector, integer or real",
                    }),
                },
                _ => {
                    bits(0, *expr)?;
                    Ok(Type::bit())
                }
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let l = ty(*lhs)?.clone();
            let r = ty(*rhs)?.clone();
            if op.is_shift() {
                bits(0, *lhs)?;
                bits(1, *rhs)?;
                return Ok(l);
            }
            if op.is_logical() {
                for (i, t) in [(0, &l), (1, &r)] {
                    if !t.is_bit() {
                        return Err(TypeError::BadOperand {
                            index: i,
                            found: t.clone(),
                            expected: "a single bit",
                        });
                    }
                }
                return Ok(Type::bit());
            }
            match (&l, &r) {
                (
                    Type::Bits {
                        width: wl,
                        signed: sl,
                    },
                    Type::Bits {
                        width: wr,
                        signed: sr,
                    },
                ) => {
                    if wl != wr {
                        return Err(TypeError::Mismatch { lhs: l, rhs: r });
                    }
                    if op.is_predicate() {
                        Ok(Type::bit())
                    } else {
                        Ok(Type::Bits {
                            width: *wl,
                            signed: *sl && *sr,
                        })
                    }
                }
                (Type::Integer, Type::Integer) | (Type::Real, Type::Real) => {
                    if op.is_bitwise() {
                        return Err(TypeError::BadOperand {
                            index: 0,
                            found: l,
                            expected: "a bit vector",
                        });
                    }
                    if op.is_predicate() {
                        Ok(Type::bit())
                    } else {
                        Ok(l)
                    }
                }
                _ => Err(TypeError::Mismatch { lhs: l, rhs: r }),
            }
        }
        ExprKind::Ternary { cond, then_, else_ } => {
            let c = ty(*cond)?;
            if !c.is_bit() {
                return Err(TypeError::BadOperand {
                    index: 0,
                    found: c.clone(),
                    expected: "a single bit",
                });
            }
            let t = ty(*then_)?;
            let e = ty(*else_)?;
            match (t, e) {
                (
                    Type::Bits {
                        width: wt,
                        signed: st,
                    },
                    Type::Bits {
                        width: we,
                        signed: se,
                    },
                ) if wt == we => Ok(Type::Bits {
                    width: *wt,
                    signed: *st && *se,
                }),
                (t, e) if t == e => Ok(t.clone()),
                (t, e) => Err(TypeError::Mismatch {
                    lhs: t.clone(),
                    rhs: e.clone(),
                }),
            }
        }
        ExprKind::Resize {
            expr,
            width,
            signed,
        } => match ty(*expr)? {
            Type::Bits { .. } | Type::Integer => Ok(Type::Bits {
                width: *width,
                signed: *signed,
            }),
            found => Err(TypeError::BadOperand {
                index: 0,
                found: found.clone(),
                expected: "a bit vector or integer",
            }),
        },
        ExprKind::MemRead { mem, addr } => {
            bits(1, *addr)?;
            module
                .memories
                .get(*mem)
                .map(|m| m.elem.clone())
                .ok_or(TypeError::DanglingMemory(*mem))
        }
        ExprKind::Call { .. } => Err(TypeError::NoRule),
    }
}

/// Which expression nodes `kind` refers to directly, in a fixed order.
pub fn operands(kind: &ExprKind) -> Vec<ExprId> {
    match kind {
        ExprKind::Const(_) | ExprKind::String(_) | ExprKind::Net(_) => Vec::new(),
        ExprKind::Slice { base, .. } => vec![*base],
        ExprKind::Index { base, index } => vec![*base, *index],
        ExprKind::IndexedSlice { base, offset, .. } => vec![*base, *offset],
        ExprKind::Concat(parts) => parts.clone(),
        ExprKind::Replicate { expr, .. } => vec![*expr],
        ExprKind::Unary { expr, .. } => vec![*expr],
        ExprKind::Binary { lhs, rhs, .. } => vec![*lhs, *rhs],
        ExprKind::Ternary { cond, then_, else_ } => vec![*cond, *then_, *else_],
        ExprKind::Resize { expr, .. } => vec![*expr],
        ExprKind::MemRead { addr, .. } => vec![*addr],
        ExprKind::Call { args, .. } => args.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_names_round_trip() {
        for op in UnaryOp::ALL {
            assert_eq!(UnaryOp::from_name(op.name()), Some(op));
        }
        for op in BinaryOp::ALL {
            assert_eq!(BinaryOp::from_name(op.name()), Some(op));
        }
        assert_eq!(BinaryOp::from_name("nope"), None);
        assert_eq!(UnaryOp::from_name("nope"), None);
        assert!(BinaryOp::Eq.is_predicate());
        assert!(!BinaryOp::Add.is_predicate());
        assert!(BinaryOp::Lt.is_relational());
        assert!(UnaryOp::ReduceAnd.is_reduction());
        assert!(!UnaryOp::Neg.is_reduction());
    }
}
