//! Lowering expressions to IR expression nodes.
//!
//! Every IR binary operator needs operands of equal width, so this is
//! where the Verilog sizing rules become explicit [`ExprKind::Resize`]
//! nodes. An expression is lowered against an optional [`Ctx`] (the width
//! and signedness the context demands); the node that comes back always
//! has exactly that type, and the operator inside it was evaluated at the
//! width the standard prescribes:
//!
//! - context-determined operands (the two sides of `+ - * / % & | ^`, the
//!   branches of `?:`, the left side of a shift) are lowered at
//!   `max(self-determined width, context width)`;
//! - self-determined operands (concatenation elements, the right side of a
//!   shift, everything inside a comparison, reduction or logical operator)
//!   are lowered at their own width;
//! - signedness comes from the operands, never from the context
//!   (§5.5.1), so an unsigned expression assigned to a signed target is
//!   computed unsigned and only then reinterpreted.
//!
//! Anything that evaluates to a constant is folded first, so parameters,
//! `$clog2`, enum members and constant functions become literals rather
//! than IR nodes.
//!
//! Selects are translated from the declared `[msb:lsb]` numbering to the
//! IR's zero-based one: a bit-select of `[7:0]` is the index itself, one
//! of `[15:8]` subtracts eight, and an ascending `[0:7]` counts down.
//! Struct fields become constant slices at the field's bit offset.

use crate::diag::Diagnostic;
use crate::ir::builder::BlockBuilder;
use crate::ir::{
    self, AssignKind, BinaryOp as IrBin, ExprId, MemoryId, NetId, ProcessKind, Type,
    UnaryOp as IrUn,
};
use crate::logic::Logic;
use crate::source::Span;
use crate::verilog::ast::{self, BinaryOp, CastTarget, Expr, ExprKind, UnaryOp};

use super::super::codes;
use super::super::constant::{self, Evaluator, Value};
use super::super::scope::{FnRef, Symbol};
use super::super::types::{Packed, Range, VType, bits_needed};
use super::super::width::{self, Info};
use super::{Ctx, Lowerer};

/// Where statements produced while lowering an expression go.
pub(crate) enum Sink<'a> {
    /// Into a procedural block.
    Proc(&'a mut BlockBuilder),
    /// Into a fresh combinational helper process (continuous context).
    Cont,
}

/// Where a value lives.
enum Place {
    /// A net, with its declared type.
    Net(NetId, VType),
    /// A memory element address space.
    Mem(MemoryId),
    /// A computed value.
    Value(ExprId, VType),
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// Lowers `e` in a continuous (non-procedural) context.
    pub(super) fn expr_cont(&mut self, e: &'ast Expr, ctx: Option<Ctx>) -> ExprId {
        self.expr(e, ctx, &mut Sink::Cont)
    }

    /// Lowers `e` to a node of exactly the context's type.
    pub(super) fn expr(&mut self, e: &'ast Expr, ctx: Option<Ctx>, sink: &mut Sink<'_>) -> ExprId {
        let id = self.build(e, ctx.map_or(0, |c| c.width), sink);
        match ctx {
            Some(c) => self.coerce(id, c.width, c.signed),
            None => id,
        }
    }

    /// Lowers `e` to a single bit holding its truth value, as `if`, `?:`
    /// and the logical operators need.
    pub(super) fn expr_bit(&mut self, e: &'ast Expr, sink: &mut Sink<'_>) -> ExprId {
        let id = self.build(e, 0, sink);
        self.as_bit(id)
    }

    /// Reduces a node to one bit: non-zero becomes 1.
    pub(super) fn as_bit(&mut self, id: ExprId) -> ExprId {
        match self.b.module().expr(id).ty.width() {
            Some(1) => id,
            Some(_) => self.b.reduce_or(id),
            None => {
                let ty = self.b.module().expr(id).ty.clone();
                self.b.span = self.b.module().expr(id).span;
                self.env.error(
                    codes::TYPE,
                    self.b.module().expr(id).span,
                    format!("a `{ty}` value has no truth value"),
                );
                self.b.const_bit(false)
            }
        }
    }

    /// Inserts a resize node unless the node already has
    /// the requested type.
    pub(super) fn coerce(&mut self, id: ExprId, width: u32, signed: bool) -> ExprId {
        let ty = self.b.module().expr(id).ty.clone();
        if ty == (Type::Bits { width, signed }) {
            return id;
        }
        // A literal is resized in place rather than wrapped, which keeps
        // the IR readable; the arithmetic is the same as the node's.
        if let Some(c) = self.b.module().expr(id).as_const().cloned() {
            let resized = if signed {
                c.resize(width).with_signed(true)
            } else {
                c.as_unsigned().resize(width)
            };
            self.b.span = self.b.module().expr(id).span;
            return self.b.constant(resized);
        }
        if !ty.is_bits() {
            // Strings and reals reaching an integral context: the value
            // was folded when it was constant, so this is a genuine type
            // error the IR cannot carry.
            let span = self.b.module().expr(id).span;
            self.env.error(
                codes::TYPE,
                span,
                format!("a `{ty}` value cannot be used as a {width}-bit vector"),
            );
            return self.b.const_u64(width, 0);
        }
        self.b.resize(id, width, signed)
    }

    /// The self-determined width and signedness of `e`, or one unsigned
    /// bit when it cannot be inferred.
    fn info_of(&mut self, e: &'ast Expr) -> Info {
        self.env
            .probe(|env| width::info(e, env, &[]))
            .unwrap_or(Info::Bits {
                width: 1,
                signed: false,
                min: 1,
            })
    }

    /// The source type of an expression, for selects.
    fn type_of(&mut self, e: &'ast Expr) -> VType {
        if let Some(t) = self.env.probe(|env| env.type_of_path(e)) {
            return t;
        }
        match self.info_of(e) {
            Info::Bits { width, signed, .. } => VType::Packed(if signed {
                Packed::sbits(width)
            } else {
                Packed::bits(width)
            }),
            Info::Real => VType::Real,
            Info::Str(_) => VType::String,
            Info::Other => VType::bit(),
        }
    }

    /// Builds the node for `e` at `max(self width, want)` with its own
    /// signedness.
    fn build(&mut self, e: &'ast Expr, want: u32, sink: &mut Sink<'_>) -> ExprId {
        self.b.span = e.span;
        if let Some(id) = self.try_const(e, want) {
            return id;
        }
        self.b.span = e.span;
        match &e.kind {
            ExprKind::Literal(ast::Literal::Str { value, .. }) => self.b.string(value.clone()),
            ExprKind::Literal(_) => {
                // A literal that did not fold is malformed; the evaluator
                // reported it.
                self.b.const_u64(want.max(1), 0)
            }
            ExprKind::Ident(_) | ExprKind::Scoped { .. } => self.name_value(e),
            ExprKind::Member { .. } => self.member(e, sink),
            ExprKind::Index { base, index } => self.index(e, base, index, sink),
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => self.part_select(e, base, *kind, left, right, sink),
            ExprKind::Unary { op, operand } => self.unary(e, *op, operand, want, sink),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, want, sink),
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                let info = self.info_of(e);
                let w = eval_width(&info, want, true);
                let signed = info.is_signed();
                let c = self.expr_bit(cond, sink);
                let t = self.expr(then_expr, Some(Ctx { width: w, signed }), sink);
                let f = self.expr(else_expr, Some(Ctx { width: w, signed }), sink);
                self.b.span = e.span;
                self.b.mux(c, t, f)
            }
            ExprKind::Concat(parts) => {
                let mut ids = Vec::with_capacity(parts.len());
                for p in parts {
                    let info = self.info_of(p);
                    let id = self.expr(
                        p,
                        Some(Ctx {
                            width: info.width().max(1),
                            signed: false,
                        }),
                        sink,
                    );
                    ids.push(id);
                }
                self.b.span = e.span;
                self.b.concat(ids)
            }
            ExprKind::Replicate { count, elems } => {
                let n = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_u32(count).ok());
                let Some(n) = n else {
                    self.env.error(
                        codes::NOT_CONSTANT,
                        count.span,
                        "a replication count must be constant",
                    );
                    return self.b.const_u64(want.max(1), 0);
                };
                let mut ids = Vec::with_capacity(elems.len());
                for p in elems {
                    let info = self.info_of(p);
                    let id = self.expr(
                        p,
                        Some(Ctx {
                            width: info.width().max(1),
                            signed: false,
                        }),
                        sink,
                    );
                    ids.push(id);
                }
                self.b.span = e.span;
                let inner = if ids.len() == 1 {
                    ids[0]
                } else {
                    self.b.concat(ids)
                };
                self.b.replicate(n, inner)
            }
            ExprKind::Call { callee, args } => self.call(e, callee, args, want, sink),
            ExprKind::Cast { target, expr } => self.cast(e, target, expr, want, sink),
            ExprKind::Inside { expr, set } => self.inside(e, expr, set, sink),
            ExprKind::MinTypMax { typ, .. } => self.build(typ, want, sink),
            ExprKind::Assign { lhs, op, rhs } => {
                self.assign_expr_sink(lhs, *op, Some(rhs), e.span, sink);
                self.build(lhs, want, sink)
            }
            ExprKind::IncDec {
                increment, target, ..
            } => {
                let op = if *increment {
                    ast::AssignOp::Add
                } else {
                    ast::AssignOp::Sub
                };
                self.assign_expr_sink(target, op, None, e.span, sink);
                self.build(target, want, sink)
            }
            ExprKind::SystemIdent(id) => {
                let info = self.info_of(e);
                let w = info.width().max(1);
                self.b.span = e.span;
                self.b.call(
                    format!("${}", id.name),
                    Vec::new(),
                    Type::Bits {
                        width: w,
                        signed: info.is_signed(),
                    },
                )
            }
            ExprKind::Pattern(_) => {
                self.env
                    .unsupported(e.span, "an assignment pattern outside a constant");
                self.b.const_u64(want.max(1), 0)
            }
            ExprKind::Streaming { .. } => {
                self.env.unsupported(e.span, "the streaming operator");
                self.b.const_u64(want.max(1), 0)
            }
            ExprKind::New(_) => {
                self.env.unsupported(e.span, "`new`");
                self.b.const_u64(want.max(1), 0)
            }
            ExprKind::ValueRange { .. } | ExprKind::Default | ExprKind::Type(_) => {
                self.env.error(
                    codes::TYPE,
                    e.span,
                    "this is not valid where a value is required",
                );
                self.b.const_u64(want.max(1), 0)
            }
        }
    }

    /// Folds `e` when it is constant, sized to `want` bits.
    fn try_const(&mut self, e: &'ast Expr, want: u32) -> Option<ExprId> {
        if matches!(e.kind, ExprKind::Literal(ast::Literal::Str { .. })) && want == 0 {
            return None;
        }
        let ctx = (want > 0).then_some(want);
        let value = self
            .env
            .probe(|env| Evaluator::new(env).eval(e, ctx).ok())?;
        self.b.span = e.span;
        Some(match value {
            Value::Logic(l) => {
                let l = if want > 0 { l.resize(want) } else { l };
                self.b.constant(l)
            }
            Value::Str(s) => {
                if want > 0 {
                    let bits = constant::string_bits(&s).resize(want);
                    self.b.constant(bits)
                } else {
                    self.b.string(s)
                }
            }
            Value::Real(r) => {
                // The IR has no real literal; a real constant reaching a
                // design is rounded to an integer.
                let rounded = Value::Real(r).as_i64().unwrap_or(0);
                let w = if want > 0 { want } else { 64 };
                self.b
                    .constant(Logic::from_i64(rounded, w).with_signed(true))
            }
            Value::Array(_) => return None,
        })
    }

    /// A reference to a name that is not constant: a net.
    fn name_value(&mut self, e: &'ast Expr) -> ExprId {
        let sym = self.env.resolve_path(e);
        match sym {
            Some(Symbol::Net { net, .. }) => {
                self.b.span = e.span;
                self.b.net(net)
            }
            Some(Symbol::Memory { .. }) => {
                self.env.error(
                    codes::TYPE,
                    e.span,
                    "a whole array cannot be used as a value; index it",
                );
                self.b.const_bit(false)
            }
            Some(Symbol::Const { value, .. }) => {
                self.b.span = e.span;
                match value.to_logic() {
                    Some(l) => self.b.constant(l),
                    None => self.b.const_bit(false),
                }
            }
            Some(_) => {
                self.env
                    .error(codes::TYPE, e.span, "this name is not a value");
                self.b.const_bit(false)
            }
            None => self.b.const_bit(false),
        }
    }

    /// `base.name`: a struct field, an interface signal or a package
    /// member.
    fn member(&mut self, e: &'ast Expr, sink: &mut Sink<'_>) -> ExprId {
        let ExprKind::Member { base, name } = &e.kind else {
            unreachable!("member called on another node");
        };
        // An interface signal or a member of a generate block resolves as
        // a path.
        if let Some(sym) = self.env.probe(|env| env.resolve_path(e)) {
            return match sym {
                Symbol::Net { net, .. } => {
                    self.b.span = e.span;
                    self.b.net(net)
                }
                Symbol::Const { value, .. } => {
                    self.b.span = e.span;
                    match value.to_logic() {
                        Some(l) => self.b.constant(l),
                        None => self.b.const_bit(false),
                    }
                }
                _ => {
                    self.env
                        .error(codes::TYPE, e.span, "this name is not a value");
                    self.b.const_bit(false)
                }
            };
        }
        // Otherwise it is a field of a packed struct.
        let base_ty = self.type_of(base);
        let Some(field) = base_ty.packed().and_then(|p| p.field(&name.name)).cloned() else {
            self.env.resolve_path(e);
            return self.b.const_bit(false);
        };
        let id = self.build(base, 0, sink);
        let w = field.ty.packed().map_or(1, Packed::width);
        self.b.span = e.span;
        let slice = self.b.slice(id, field.lsb + w - 1, field.lsb);
        if field.ty.packed().is_some_and(|p| p.signed) {
            self.b.resize(slice, w, true)
        } else {
            slice
        }
    }

    /// Resolves the base of a select.
    fn place(&mut self, e: &'ast Expr, sink: &mut Sink<'_>) -> Place {
        if let Some(sym) = self.env.probe(|env| env.resolve_path(e)) {
            match sym {
                Symbol::Net { net, ty } => return Place::Net(net, ty),
                Symbol::Memory { mem, .. } => return Place::Mem(mem),
                _ => {}
            }
        }
        let ty = self.type_of(e);
        let id = self.build(e, 0, sink);
        Place::Value(id, ty)
    }

    /// `base[index]`.
    fn index(
        &mut self,
        e: &'ast Expr,
        base: &'ast Expr,
        index: &'ast Expr,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        match self.place(base, sink) {
            Place::Mem(mem) => {
                let addr = self.mem_address(mem, index, sink);
                self.b.span = e.span;
                self.b.mem_read(mem, addr)
            }
            Place::Net(net, ty) => {
                let id = self.b.net(net);
                self.bit_select(id, &ty, index, e.span, sink)
            }
            Place::Value(id, ty) => self.bit_select(id, &ty, index, e.span, sink),
        }
    }

    /// The IR address expression for `mem[index]`.
    pub(super) fn mem_address(
        &mut self,
        mem: MemoryId,
        index: &'ast Expr,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let range = self.mems[mem.index()].range;
        let size = self.b.module().memories[mem].size;
        let bits = bits_needed(size.saturating_sub(1)).max(1);
        let low = range.low();
        let info = self.info_of(index);
        let width = info
            .width()
            .max(bits)
            .max(bits_needed(low.unsigned_abs()) + 1)
            .clamp(1, 64);
        let id = self.expr(
            index,
            Some(Ctx {
                width,
                signed: info.is_signed(),
            }),
            sink,
        );
        if low == 0 {
            return id;
        }
        let k = self
            .b
            .constant(Logic::from_i64(low, width).with_signed(info.is_signed()));
        self.b.sub(id, k)
    }

    /// Translates a declared index into the IR's zero-based numbering and
    /// returns a node of at least `bits` bits.
    fn normalise_index(
        &mut self,
        index: &'ast Expr,
        range: Range,
        bits: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let info = self.info_of(index);
        let base_bits = info
            .width()
            .max(bits)
            .max(bits_needed(range.right.unsigned_abs()) + 1);
        let width = base_bits.clamp(1, 64);
        let signed = info.is_signed();
        let id = self.expr(index, Some(Ctx { width, signed }), sink);
        if range.right == 0 && range.is_descending() {
            return id;
        }
        let k = self
            .b
            .constant(Logic::from_i64(range.right, width).with_signed(signed));
        if range.is_descending() {
            self.b.sub(id, k)
        } else {
            self.b.sub(k, id)
        }
    }

    /// `base[index]` on an integral value.
    fn bit_select(
        &mut self,
        base: ExprId,
        ty: &VType,
        index: &'ast Expr,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let VType::Packed(p) = ty else {
            self.env
                .error(codes::TYPE, span, format!("`{ty}` cannot be indexed"));
            return self.b.const_bit(false);
        };
        let outer = p.outer();
        let ew = p.element().width().max(1);
        let constant = self
            .env
            .probe(|env| Evaluator::new(env).eval_i64(index).ok());
        if let Some(i) = constant {
            self.b.span = span;
            return match outer.offset(i) {
                Some(off) => {
                    let lo = u32::try_from(off * u64::from(ew)).unwrap_or(0);
                    self.b.slice(base, lo + ew - 1, lo)
                }
                None => {
                    self.env.warning(
                        codes::OUT_OF_RANGE,
                        index.span,
                        format!("index {i} is outside {outer}; the result is always x"),
                    );
                    self.b.constant(Logic::x(ew))
                }
            };
        }
        let idx_bits = bits_needed(u64::from(p.width())).max(1);
        let norm = self.normalise_index(index, outer, idx_bits, sink);
        self.b.span = span;
        if ew == 1 {
            self.b.index(base, norm)
        } else {
            let scale = self
                .b
                .constant(Logic::from_u64(u64::from(ew), self.expr_width(norm)));
            let offset = self.b.binary(IrBin::Mul, norm, scale);
            self.b.indexed_slice(base, offset, ew, true)
        }
    }

    /// The width of an IR node.
    fn expr_width(&self, id: ExprId) -> u32 {
        self.b.module().expr(id).ty.width().unwrap_or(1)
    }

    /// `base[a:b]`, `base[a +: w]` and `base[a -: w]`.
    fn part_select(
        &mut self,
        e: &'ast Expr,
        base: &'ast Expr,
        kind: ast::RangeKind,
        left: &'ast Expr,
        right: &'ast Expr,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let (id, ty) = match self.place(base, sink) {
            Place::Net(net, ty) => (self.b.net(net), ty),
            Place::Value(id, ty) => (id, ty),
            Place::Mem(_) => {
                self.env
                    .unsupported(e.span, "a part-select of a whole array");
                return self.b.const_bit(false);
            }
        };
        let Some(p) = ty.packed().cloned() else {
            self.env
                .error(codes::TYPE, e.span, format!("`{ty}` cannot be sliced"));
            return self.b.const_bit(false);
        };
        let outer = p.outer();
        let ew = p.element().width().max(1);
        let lo_const = self
            .env
            .probe(|env| Evaluator::new(env).eval_i64(left).ok());
        let hi_const = self
            .env
            .probe(|env| Evaluator::new(env).eval_i64(right).ok());
        self.b.span = e.span;
        match kind {
            ast::RangeKind::Fixed => {
                let (Some(a), Some(b)) = (lo_const, hi_const) else {
                    self.env.error(
                        codes::NOT_CONSTANT,
                        e.span,
                        "the bounds of a `[a:b]` part-select must be constant",
                    );
                    return self.b.const_bit(false);
                };
                let (Some(oa), Some(ob)) = (outer.offset(a), outer.offset(b)) else {
                    self.env.warning(
                        codes::OUT_OF_RANGE,
                        e.span,
                        format!("part-select [{a}:{b}] is outside {outer}"),
                    );
                    let width = u32::try_from(a.abs_diff(b) + 1).unwrap_or(1) * ew;
                    return self.b.constant(Logic::x(width));
                };
                let (hi, lo) = (oa.max(ob), oa.min(ob));
                let hi = u32::try_from((hi + 1) * u64::from(ew) - 1).unwrap_or(0);
                let lo = u32::try_from(lo * u64::from(ew)).unwrap_or(0);
                self.b.slice(id, hi, lo)
            }
            ast::RangeKind::IndexedUp | ast::RangeKind::IndexedDown => {
                let Some(w) = hi_const
                    .and_then(|w| u32::try_from(w).ok())
                    .filter(|w| *w > 0)
                else {
                    self.env.error(
                        codes::NOT_CONSTANT,
                        right.span,
                        "the width of an indexed part-select must be a positive constant",
                    );
                    return self.b.const_bit(false);
                };
                let count = w.saturating_mul(ew);
                let up = kind == ast::RangeKind::IndexedUp;
                if let Some(a) = lo_const {
                    // A constant base index becomes a plain slice.
                    let low = if outer.is_descending() == up {
                        a
                    } else if outer.is_descending() {
                        a - i64::from(w) + 1
                    } else {
                        a + i64::from(w) - 1
                    };
                    let high_decl = if outer.is_descending() {
                        low + i64::from(w) - 1
                    } else {
                        low - i64::from(w) + 1
                    };
                    let (Some(ol), Some(oh)) = (outer.offset(low), outer.offset(high_decl)) else {
                        self.env.warning(
                            codes::OUT_OF_RANGE,
                            e.span,
                            format!("part-select of {w} bits at {a} is outside {outer}"),
                        );
                        return self.b.constant(Logic::x(count));
                    };
                    let lo = u32::try_from(ol.min(oh) * u64::from(ew)).unwrap_or(0);
                    return self.b.slice(id, lo + count - 1, lo);
                }
                let idx_bits = bits_needed(u64::from(p.width())).max(1);
                let norm = self.normalise_index(left, outer, idx_bits, sink);
                self.b.span = e.span;
                let nw = self.expr_width(norm);
                let base_off = if outer.is_descending() == up {
                    norm
                } else {
                    let k = self.b.const_u64(nw, u64::from(w - 1));
                    self.b.sub(norm, k)
                };
                let offset = if ew == 1 {
                    base_off
                } else {
                    let scale = self.b.const_u64(nw, u64::from(ew));
                    self.b.binary(IrBin::Mul, base_off, scale)
                };
                self.b.indexed_slice(id, offset, count, true)
            }
        }
    }

    fn unary(
        &mut self,
        e: &'ast Expr,
        op: UnaryOp,
        operand: &'ast Expr,
        want: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => {
                let info = self.info_of(e);
                let w = eval_width(&info, want, true);
                let ctx = Ctx {
                    width: w,
                    signed: info.is_signed(),
                };
                let id = self.expr(operand, Some(ctx), sink);
                self.b.span = e.span;
                match op {
                    UnaryOp::Plus => id,
                    UnaryOp::Minus => self.b.neg(id),
                    _ => self.b.not(id),
                }
            }
            UnaryOp::LogicNot => {
                let id = self.build(operand, 0, sink);
                self.b.span = e.span;
                self.b.lnot(id)
            }
            _ => {
                let info = self.info_of(operand);
                let id = self.expr(
                    operand,
                    Some(Ctx {
                        width: info.width().max(1),
                        signed: info.is_signed(),
                    }),
                    sink,
                );
                self.b.span = e.span;
                let irop = match op {
                    UnaryOp::ReduceAnd => IrUn::ReduceAnd,
                    UnaryOp::ReduceNand => IrUn::ReduceNand,
                    UnaryOp::ReduceOr => IrUn::ReduceOr,
                    UnaryOp::ReduceNor => IrUn::ReduceNor,
                    UnaryOp::ReduceXor => IrUn::ReduceXor,
                    _ => IrUn::ReduceXnor,
                };
                self.b.unary(irop, id)
            }
        }
    }

    fn binary(
        &mut self,
        e: &'ast Expr,
        op: BinaryOp,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        want: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        use BinaryOp as B;
        // Logical connectives: both sides are self-determined truths.
        if matches!(op, B::LogicAnd | B::LogicOr | B::Implies | B::Equiv) {
            let a = self.expr_bit(lhs, sink);
            let b = self.expr_bit(rhs, sink);
            self.b.span = e.span;
            return match op {
                B::LogicAnd => self.b.land(a, b),
                B::LogicOr => self.b.lor(a, b),
                B::Implies => {
                    let na = self.b.lnot(a);
                    self.b.lor(na, b)
                }
                _ => self.b.eq(a, b),
            };
        }
        // Comparisons: operands sized to each other, result one bit.
        if matches!(
            op,
            B::Lt
                | B::Le
                | B::Gt
                | B::Ge
                | B::Eq
                | B::Ne
                | B::CaseEq
                | B::CaseNe
                | B::WildEq
                | B::WildNe
        ) {
            let li = self.info_of(lhs);
            let ri = self.info_of(rhs);
            let c = li.combine(&ri);
            let ctx = Ctx {
                width: c.width().max(1),
                signed: c.is_signed(),
            };
            let a = self.expr(lhs, Some(ctx), sink);
            let b = self.expr(rhs, Some(ctx), sink);
            self.b.span = e.span;
            let irop = match op {
                B::Lt => IrBin::Lt,
                B::Le => IrBin::Le,
                B::Gt => IrBin::Gt,
                B::Ge => IrBin::Ge,
                B::Eq => IrBin::Eq,
                B::Ne => IrBin::Ne,
                B::CaseEq => IrBin::CaseEq,
                B::CaseNe => IrBin::CaseNe,
                B::WildEq | B::WildNe => IrBin::WildEq,
                _ => unreachable!("comparison operators only"),
            };
            let id = self.b.binary(irop, a, b);
            return if op == B::WildNe { self.b.lnot(id) } else { id };
        }

        let info = self.info_of(e);
        let w = eval_width(&info, want, truncatable(op));
        let signed = info.is_signed();
        let ctx = Ctx { width: w, signed };
        let a = self.expr(lhs, Some(ctx), sink);
        // Shift amounts and exponents are self-determined; the IR wants
        // the exponent at the base's width, which preserves its value.
        let b = match op {
            B::Shl | B::Shr | B::Ashl | B::Ashr => {
                let ri = self.info_of(rhs);
                self.expr(
                    rhs,
                    Some(Ctx {
                        width: ri.width().max(1),
                        signed: false,
                    }),
                    sink,
                )
            }
            B::Pow => {
                let ri = self.info_of(rhs);
                self.expr(
                    rhs,
                    Some(Ctx {
                        width: w,
                        signed: ri.is_signed(),
                    }),
                    sink,
                )
            }
            _ => self.expr(rhs, Some(ctx), sink),
        };
        self.b.span = e.span;
        let irop = match op {
            B::Add => IrBin::Add,
            B::Sub => IrBin::Sub,
            B::Mul => IrBin::Mul,
            B::Div => IrBin::Div,
            B::Mod => IrBin::Mod,
            B::Pow => IrBin::Pow,
            B::Shl | B::Ashl => IrBin::Shl,
            B::Shr => IrBin::Shr,
            B::Ashr => IrBin::Sshr,
            B::BitAnd => IrBin::And,
            B::BitOr => IrBin::Or,
            B::BitXor => IrBin::Xor,
            B::BitXnor => IrBin::Xnor,
            _ => unreachable!("handled above"),
        };
        self.b.binary(irop, a, b)
    }

    fn cast(
        &mut self,
        e: &'ast Expr,
        target: &'ast CastTarget,
        inner: &'ast Expr,
        want: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        match target {
            CastTarget::Const => self.build(inner, want, sink),
            CastTarget::Signing(s) => {
                let info = self.info_of(inner);
                let w = info.width().max(1);
                let id = self.expr(
                    inner,
                    Some(Ctx {
                        width: w,
                        signed: info.is_signed(),
                    }),
                    sink,
                );
                self.coerce(id, w, *s == ast::Signing::Signed)
            }
            CastTarget::Size(n) => {
                let w = self.env.probe(|env| Evaluator::new(env).eval_u32(n).ok());
                let Some(w) = w.filter(|w| *w > 0) else {
                    self.env.error(
                        codes::NOT_CONSTANT,
                        n.span,
                        "a size cast must use a positive constant width",
                    );
                    return self.build(inner, want, sink);
                };
                let info = self.info_of(inner);
                let id = self.build(inner, w, sink);
                self.coerce(id, w, info.is_signed())
            }
            CastTarget::Type(dt) => {
                let ty = super::super::types::resolve(&mut self.env, dt, &[], true);
                let Some(p) = ty.as_ref().and_then(VType::packed) else {
                    self.env
                        .unsupported(e.span, "a cast to a non-integral type");
                    return self.build(inner, want, sink);
                };
                let (w, signed) = (p.width(), p.signed);
                let id = self.build(inner, w, sink);
                self.coerce(id, w, signed)
            }
        }
    }

    /// `expr inside { ... }` as a chain of comparisons.
    fn inside(
        &mut self,
        e: &'ast Expr,
        subject: &'ast Expr,
        set: &'ast [Expr],
        sink: &mut Sink<'_>,
    ) -> ExprId {
        let mut info = self.info_of(subject);
        for s in set {
            match &s.kind {
                ExprKind::ValueRange { low, high } => {
                    let l = self.info_of(low);
                    let h = self.info_of(high);
                    info = info.combine(&l).combine(&h);
                }
                _ => {
                    let i = self.info_of(s);
                    info = info.combine(&i);
                }
            }
        }
        let ctx = Ctx {
            width: info.width().max(1),
            signed: info.is_signed(),
        };
        let x = self.expr(subject, Some(ctx), sink);
        let mut result: Option<ExprId> = None;
        for s in set {
            let hit = match &s.kind {
                ExprKind::ValueRange { low, high } => {
                    let lo = self.expr(low, Some(ctx), sink);
                    let hi = self.expr(high, Some(ctx), sink);
                    self.b.span = s.span;
                    let a = self.b.ge(x, lo);
                    let b = self.b.le(x, hi);
                    self.b.land(a, b)
                }
                _ => {
                    let v = self.expr(s, Some(ctx), sink);
                    self.b.span = s.span;
                    self.b.binary(IrBin::WildEq, x, v)
                }
            };
            self.b.span = e.span;
            result = Some(match result {
                None => hit,
                Some(acc) => self.b.lor(acc, hit),
            });
        }
        match result {
            Some(id) => id,
            None => self.b.const_bit(false),
        }
    }

    // --- calls -------------------------------------------------------------

    fn call(
        &mut self,
        e: &'ast Expr,
        callee: &'ast Expr,
        args: &'ast [ast::Arg],
        want: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        if let ExprKind::SystemIdent(id) = &callee.kind {
            return self.system_call(e, &id.name, args, want, sink);
        }
        let Some((f, is_task)) = self.env.resolve_callee(callee) else {
            return self.b.const_u64(want.max(1), 0);
        };
        if is_task {
            self.env
                .error(codes::TYPE, e.span, "a task call cannot be used as a value");
            return self.b.const_u64(want.max(1), 0);
        }
        match self.inline_call(f, args, e.span, sink) {
            Some(id) => id,
            None => self.b.const_u64(want.max(1), 0),
        }
    }

    /// A system function that survives into the IR.
    fn system_call(
        &mut self,
        e: &'ast Expr,
        name: &str,
        args: &'ast [ast::Arg],
        want: u32,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        match name {
            "signed" | "unsigned" => {
                let Some(arg) = args.first().and_then(|a| a.value.as_ref()) else {
                    self.env.error(
                        codes::ARGUMENTS,
                        e.span,
                        format!("`${name}` takes one argument"),
                    );
                    return self.b.const_u64(want.max(1), 0);
                };
                let info = self.info_of(arg);
                let w = info.width().max(1);
                let id = self.expr(
                    arg,
                    Some(Ctx {
                        width: w,
                        signed: info.is_signed(),
                    }),
                    sink,
                );
                self.coerce(id, w, name == "signed")
            }
            _ => {
                let info = self.info_of(e);
                let mut ids = Vec::new();
                for a in args {
                    if let Some(v) = &a.value {
                        let id = self.build(v, 0, sink);
                        ids.push(id);
                    }
                }
                let ty = match info {
                    Info::Real => Type::Bits {
                        width: 64,
                        signed: true,
                    },
                    Info::Str(_) => Type::String,
                    other => Type::Bits {
                        width: other.width().max(1),
                        signed: other.is_signed(),
                    },
                };
                self.b.span = e.span;
                self.b.call(format!("${name}"), ids, ty)
            }
        }
    }

    /// Inlines a call of `f`, returning the net holding its result.
    ///
    /// The body is lowered into `sink`; in a continuous context a
    /// combinational helper process is created to hold it, so a function
    /// used in `assign` becomes a process plus a net.
    pub(super) fn inline_call(
        &mut self,
        f: FnRef<'ast>,
        args: &'ast [ast::Arg],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        match sink {
            Sink::Proc(block) => {
                let mut b = std::mem::replace(*block, BlockBuilder::new(span));
                let r = self.inline_into(f, args, span, &mut b);
                **block = b;
                r
            }
            Sink::Cont => {
                // A call in a continuous assignment becomes a
                // combinational helper process holding the inlined body.
                let name = self.fresh("comb");
                let index = self.b.module().processes.len();
                let outer = self.driver;
                self.driver = super::DriverKey::Process(index);
                let mut p = self.b.process(Some(&name), ProcessKind::Comb);
                p.span = span;
                let r = self.inline_into(f, args, span, &mut p);
                self.b.end_process(p);
                if let Some(id) = r
                    && let Some(net) = self.b.module().expr(id).as_net()
                {
                    self.record_driver(net, super::DriverKey::Process(index), true, span);
                }
                self.driver = outer;
                r
            }
        }
    }

    /// Inlines a call into an open block.
    pub(super) fn inline_into(
        &mut self,
        f: FnRef<'ast>,
        args: &'ast [ast::Arg],
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<ExprId> {
        let name = f.def.name.name.clone();
        if self.inlining.contains(&name) {
            self.env.report(
                Diagnostic::error(format!("`{name}` calls itself"))
                    .with_code(codes::RECURSION)
                    .with_label(span, "recursive call")
                    .with_secondary(f.def.name.span, "declared here")
                    .with_note("recursive functions and tasks cannot be inlined"),
            );
            return None;
        }
        let formals = constant::formals(f.def);
        let slots = match constant::bind_args(&formals, args) {
            Ok(s) => s,
            Err(msg) => {
                self.env.error(
                    codes::ARGUMENTS,
                    span,
                    format!("in call of `{name}`: {msg}"),
                );
                return None;
            }
        };

        // Actual arguments are evaluated in the caller's scope, before the
        // frame exists.
        let mut inputs: Vec<Option<ExprId>> = Vec::with_capacity(formals.len());
        let mut outputs: Vec<Option<ir::Lvalue>> = Vec::with_capacity(formals.len());
        for (slot, formal) in slots.iter().zip(&formals) {
            let is_in = matches!(
                formal.dir,
                ast::Direction::Input | ast::Direction::Inout | ast::Direction::ConstRef
            );
            let is_out = matches!(
                formal.dir,
                ast::Direction::Output | ast::Direction::Inout | ast::Direction::Ref
            );
            match slot {
                Some(e) => {
                    let value = if is_in {
                        Some(self.expr(e, None, &mut Sink::Proc(out)))
                    } else {
                        None
                    };
                    let target = if is_out { self.lvalue(e, true) } else { None };
                    inputs.push(value);
                    outputs.push(target);
                }
                None => {
                    if is_in && !is_out {
                        self.env.error(
                            codes::ARGUMENTS,
                            span,
                            format!(
                                "missing argument `{}` in call of `{name}`",
                                formal.name.name
                            ),
                        );
                        return None;
                    }
                    inputs.push(None);
                    outputs.push(None);
                }
            }
        }

        let frame = self.fresh(&name);
        let scope = self.env.cx.scopes.push(Some(f.home), format!("{frame}."));
        self.env.enter(scope);
        self.inlining.push(name.clone());

        // The formals and the result live in nets of the enclosing module.
        let mut result = None;
        for (i, formal) in formals.iter().enumerate() {
            let ty = if formal.data_type.is_empty() && formal.dims.is_empty() {
                VType::bit()
            } else {
                match super::super::types::resolve(
                    &mut self.env,
                    formal.data_type,
                    formal.dims,
                    true,
                ) {
                    Some(t) => t,
                    None => VType::bit(),
                }
            };
            let ir_name = self.env.qualified(&formal.name.name);
            let Some(net) = self.new_var(ir_name, ty.clone(), formal.name.span) else {
                continue;
            };
            self.env.declare(
                &formal.name.name,
                Symbol::Net {
                    net,
                    ty: ty.clone(),
                },
                formal.name.span,
            );
            if let Some(value) = inputs[i] {
                let w = ty.packed().map_or(1, Packed::width);
                let signed = ty.packed().is_some_and(|p| p.signed);
                let v = self.coerce(value, w, signed);
                out.span = span;
                out.assign(net, v, AssignKind::Blocking);
                self.record_driver(net, self.driver, true, span);
            }
        }
        if let Some(ret) = &f.def.ret
            && ret.kind != ast::DataTypeKind::Void
        {
            let ty = if ret.is_empty() {
                VType::bit()
            } else {
                super::super::types::resolve(&mut self.env, ret, &[], true)
                    .unwrap_or_else(VType::bit)
            };
            let ir_name = self.env.qualified(&name);
            if let Some(net) = self.new_var(ir_name, ty.clone(), f.def.name.span) {
                self.env
                    .declare(&name, Symbol::Net { net, ty }, f.def.name.span);
                result = Some(net);
            }
        }

        // The body.
        let needs_flag = super::stmt::has_early_return(&f.def.body);
        let flag = if needs_flag {
            let ir_name = self.env.qualified("$returned");
            let net = self.new_var(ir_name, VType::bit(), f.def.name.span)?;
            out.span = span;
            let zero = self.b.const_bit(false);
            out.assign(net, zero, AssignKind::Blocking);
            self.env.declare(
                "$returned",
                Symbol::Net {
                    net,
                    ty: VType::bit(),
                },
                f.def.name.span,
            );
            Some(net)
        } else {
            None
        };
        self.inline_frames.push((result, flag));
        self.stmts_guarded(&f.def.body, out, flag);
        self.inline_frames.pop();

        // Copy outputs back.
        for (i, formal) in formals.iter().enumerate() {
            let Some(target) = outputs[i].clone() else {
                continue;
            };
            let Some((Symbol::Net { net, ty }, _, _)) = self.env.lookup(&formal.name.name) else {
                continue;
            };
            let (net, ty) = (*net, ty.clone());
            let value = self.b.net(net);
            let w = self.lvalue_width(&target);
            let signed = ty.packed().is_some_and(|p| p.signed);
            let v = self.coerce(value, w, signed);
            out.span = span;
            out.assign(target.clone(), v, AssignKind::Blocking);
            self.note_assign_drivers(&target, self.driver, span, true);
        }

        self.inlining.pop();
        self.env.leave();
        let net = result?;
        self.b.span = span;
        Some(self.b.net(net))
    }

    /// Creates a process-local variable net.
    pub(super) fn new_var(&mut self, name: String, ty: VType, span: Span) -> Option<NetId> {
        let Some(irt) = ty.ir_type() else {
            self.env.error(
                codes::TYPE,
                span,
                format!("`{ty}` has no representation in the IR"),
            );
            return None;
        };
        if matches!(irt, Type::Array { .. }) {
            self.env
                .unsupported(span, "an unpacked array inside a subroutine");
            return None;
        }
        self.b.span = span;
        let net = self.b.add_net_kind(name, irt, ir::NetKind::Variable);
        debug_assert_eq!(net.index(), self.nets.len());
        self.nets.push(super::NetInfo {
            ty,
            is_var: true,
            span,
        });
        self.drivers.push(Vec::new());
        Some(net)
    }

    // --- lvalues -----------------------------------------------------------

    /// Lowers an assignment target.
    ///
    /// `procedural` selects the net-versus-variable rule that applies.
    pub(super) fn lvalue(&mut self, e: &'ast Expr, procedural: bool) -> Option<ir::Lvalue> {
        match &e.kind {
            ExprKind::Ident(_) | ExprKind::Scoped { .. } | ExprKind::Member { .. } => {
                match self.env.probe(|env| env.resolve_path(e)) {
                    Some(Symbol::Net { net, .. }) => {
                        self.check_assignable(net, procedural, e.span);
                        Some(ir::Lvalue::Net(net))
                    }
                    Some(Symbol::Memory { .. }) => {
                        self.env
                            .unsupported(e.span, "an assignment to a whole array");
                        None
                    }
                    Some(Symbol::Const { .. }) => {
                        self.env.error(
                            codes::TYPE,
                            e.span,
                            "a parameter or constant cannot be assigned",
                        );
                        None
                    }
                    Some(_) | None => {
                        // A struct field, or an implicit net in a
                        // continuous assignment.
                        if let ExprKind::Member { base, name } = &e.kind {
                            return self.field_lvalue(base, name, procedural);
                        }
                        if !procedural
                            && let ExprKind::Ident(id) = &e.kind
                            && let Some(net) = self.implicit_net(id)
                        {
                            return Some(ir::Lvalue::Net(net));
                        }
                        self.env.resolve_path(e);
                        None
                    }
                }
            }
            ExprKind::Index { base, index } => match self.env.probe(|env| env.resolve_path(base)) {
                Some(Symbol::Memory { mem, .. }) => {
                    let addr = self.mem_address(mem, index, &mut Sink::Cont);
                    Some(ir::Lvalue::MemElem { mem, addr })
                }
                Some(Symbol::Net { net, ty }) => {
                    self.check_assignable(net, procedural, e.span);
                    let p = ty.packed()?;
                    let outer = p.outer();
                    let ew = p.element().width().max(1);
                    let constant = self
                        .env
                        .probe(|env| Evaluator::new(env).eval_i64(index).ok());
                    if let Some(i) = constant {
                        let Some(off) = outer.offset(i) else {
                            self.env.warning(
                                codes::OUT_OF_RANGE,
                                index.span,
                                format!("index {i} is outside {outer}; the assignment is dropped"),
                            );
                            return None;
                        };
                        let lo = u32::try_from(off * u64::from(ew)).unwrap_or(0);
                        return Some(ir::Lvalue::Slice {
                            net,
                            hi: lo + ew - 1,
                            lo,
                        });
                    }
                    if ew != 1 {
                        self.env.unsupported(
                            e.span,
                            "an assignment to a variable element of a packed array",
                        );
                        return None;
                    }
                    let idx_bits = bits_needed(u64::from(p.width())).max(1);
                    let norm = self.normalise_index(index, outer, idx_bits, &mut Sink::Cont);
                    Some(ir::Lvalue::Index { net, index: norm })
                }
                _ => {
                    self.env.resolve_path(base);
                    None
                }
            },
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => {
                let Some(Symbol::Net { net, ty }) = self.env.probe(|env| env.resolve_path(base))
                else {
                    self.env.resolve_path(base);
                    return None;
                };
                self.check_assignable(net, procedural, e.span);
                let p = ty.packed()?;
                let outer = p.outer();
                let ew = p.element().width().max(1);
                let a = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_i64(left).ok());
                let b = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_i64(right).ok());
                let (Some(a), Some(b)) = (a, b) else {
                    self.env.unsupported(
                        e.span,
                        "an assignment to a part-select with a non-constant bound",
                    );
                    return None;
                };
                let (low, high) = match kind {
                    ast::RangeKind::Fixed => (a.min(b), a.max(b)),
                    ast::RangeKind::IndexedUp => (a, a + b - 1),
                    ast::RangeKind::IndexedDown => (a - b + 1, a),
                };
                let (Some(ol), Some(oh)) = (outer.offset(low), outer.offset(high)) else {
                    self.env.warning(
                        codes::OUT_OF_RANGE,
                        e.span,
                        format!("part-select [{high}:{low}] is outside {outer}"),
                    );
                    return None;
                };
                let lo = u32::try_from(ol.min(oh) * u64::from(ew)).unwrap_or(0);
                let hi = u32::try_from((ol.max(oh) + 1) * u64::from(ew) - 1).unwrap_or(0);
                Some(ir::Lvalue::Slice { net, hi, lo })
            }
            ExprKind::Concat(parts) => {
                let mut out = Vec::with_capacity(parts.len());
                for p in parts {
                    out.push(self.lvalue(p, procedural)?);
                }
                Some(ir::Lvalue::Concat(out))
            }
            _ => {
                self.env
                    .error(codes::TYPE, e.span, "this is not a valid assignment target");
                None
            }
        }
    }

    /// An assignment to a packed struct field.
    fn field_lvalue(
        &mut self,
        base: &'ast Expr,
        name: &ast::Ident,
        procedural: bool,
    ) -> Option<ir::Lvalue> {
        let base_lv = self.lvalue(base, procedural)?;
        let base_ty = self.type_of(base);
        let field = base_ty.packed().and_then(|p| p.field(&name.name)).cloned();
        let Some(field) = field else {
            self.env.error(
                codes::UNDEFINED,
                name.span,
                format!("no member `{}` in `{base_ty}`", name.name),
            );
            return None;
        };
        let w = field.ty.packed().map_or(1, Packed::width);
        match base_lv {
            ir::Lvalue::Net(net) => Some(ir::Lvalue::Slice {
                net,
                hi: field.lsb + w - 1,
                lo: field.lsb,
            }),
            ir::Lvalue::Slice { net, lo, .. } => Some(ir::Lvalue::Slice {
                net,
                hi: lo + field.lsb + w - 1,
                lo: lo + field.lsb,
            }),
            _ => {
                self.env
                    .unsupported(name.span, "an assignment to this struct member");
                None
            }
        }
    }

    /// Creates an implicit net for an undeclared name used in a
    /// continuous context, per `` `default_nettype ``.
    pub(super) fn implicit_net(&mut self, id: &ast::Ident) -> Option<NetId> {
        if self.env.lookup(&id.name).is_some() {
            return None;
        }
        if self.default_nettype.is_none() {
            self.env.report(
                Diagnostic::error(format!("`{}` is not declared", id.name))
                    .with_code(codes::IMPLICIT_NET)
                    .with_label(id.span, "no declaration")
                    .with_note("`default_nettype none` forbids implicit nets"),
            );
            return None;
        }
        let ir_name = self.env.qualified(&id.name);
        let net = self.new_net(ir_name, VType::bit(), false, id.span)?;
        self.env.declare(
            &id.name,
            Symbol::Net {
                net,
                ty: VType::bit(),
            },
            id.span,
        );
        Some(net)
    }

    /// The width an lvalue receives.
    pub(super) fn lvalue_width(&self, lv: &ir::Lvalue) -> u32 {
        match lv {
            ir::Lvalue::Net(n) => self.net_width(*n),
            ir::Lvalue::Slice { hi, lo, .. } => hi - lo + 1,
            ir::Lvalue::Index { net, .. } => self
                .net_type(*net)
                .packed()
                .map_or(1, |p| p.element().width()),
            ir::Lvalue::Concat(parts) => parts
                .iter()
                .map(|p| self.lvalue_width(p))
                .fold(0u32, u32::saturating_add),
            ir::Lvalue::MemElem { mem, .. } => self.mems[mem.index()]
                .elem
                .packed()
                .map_or(1, Packed::width),
        }
    }

    /// A readable name for an lvalue, for diagnostics.
    pub(super) fn lvalue_name(&self, lv: &ir::Lvalue) -> String {
        match lv {
            ir::Lvalue::Net(n)
            | ir::Lvalue::Slice { net: n, .. }
            | ir::Lvalue::Index { net: n, .. } => self.b.module().nets[*n].name.to_string(),
            ir::Lvalue::Concat(_) => "{...}".to_owned(),
            ir::Lvalue::MemElem { mem, .. } => self.b.module().memories[*mem].name.to_string(),
        }
    }

    /// Records the drivers an assignment to `lv` creates.
    pub(super) fn note_assign_drivers(
        &mut self,
        lv: &ir::Lvalue,
        key: super::DriverKey,
        span: Span,
        _procedural: bool,
    ) {
        match lv {
            ir::Lvalue::Net(n) => self.record_driver(*n, key, true, span),
            ir::Lvalue::Slice { net, .. } | ir::Lvalue::Index { net, .. } => {
                self.record_driver(*net, key, false, span);
            }
            ir::Lvalue::Concat(parts) => {
                for p in parts {
                    self.note_assign_drivers(p, key, span, _procedural);
                }
            }
            ir::Lvalue::MemElem { .. } => {}
        }
    }
}

/// The width an operator is evaluated at.
///
/// The standard's rule is `max(self-determined width, context width)`
/// (§5.4.1). When the context is *narrower* than the operator's own width
/// and the operator only ever propagates information towards the more
/// significant bits — addition, subtraction, multiplication, negation, the
/// bitwise operators, a left shift, the branches of `?:` — the low bits of
/// the wide result are the low bits of the narrow one, so the operator is
/// evaluated at the context's width instead. That is what keeps
/// `q <= q + 1` an eight-bit addition instead of a thirty-two-bit one,
/// and it is exactly the arithmetic the standard prescribes, modulo the
/// bits that are discarded anyway.
fn eval_width(info: &Info, want: u32, truncatable: bool) -> u32 {
    if truncatable && want > 0 {
        return want.max(1);
    }
    info.width().max(want).max(1)
}

/// True for the operators whose low bits depend only on their operands'
/// low bits.
fn truncatable(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitXnor
            | BinaryOp::Shl
            | BinaryOp::Ashl
    )
}
