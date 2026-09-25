//! Lowering VHDL expressions to IR expression nodes.
//!
//! Every expression is lowered to an [`ExprId`] together with the
//! [`Layout`] its value has, so the caller knows how wide it is and how to
//! place it. Three tables of the analysis do the work that would otherwise
//! need a second type checker:
//!
//! - `value_of` folds a locally static expression to a value, which
//!   becomes one `Const` node whatever it was written as — a literal, a
//!   bit-string, an aggregate, `x"F0" xor "0011"`;
//! - `type_of` gives every node its subtype, hence its layout;
//! - `call_of` says what an operator or a call resolved to: a predefined
//!   operation, a type conversion, an array index, an attribute or a
//!   subprogram.
//!
//! The IR requires operands of equal width, so every predefined operator
//! widens its operands with an explicit `Resize` before building the node.
//! Operators of `ieee.std_logic_1164` are recognised by name and lowered to
//! the same nodes as the predefined ones on `bit`, rather than inlining
//! their nine-state lookup tables; a call into a package Reticle does not
//! bundle yet is `V0710`.

use crate::diag::Diagnostic;
use crate::ir::{BinaryOp, Const, ExprId, ExprKind, Lvalue, Type, UnaryOp as IrUnary};
use crate::logic::{Bit, Logic, Std9};
use crate::source::Span;
use crate::vhdl::ast::{self, Direction};
use crate::vhdl::sema::{CallTarget, DeclId, DeclKind, TypeId, Value, attrs::Predefined};

use super::codes;
use super::lower::{Binding, Lowerer, Sink};
use super::types::{self, ArrayLayout, BitKind, Layout, LayoutKind};

/// The nine `std_ulogic` positions as four-state bits.
fn std9_bit(pos: u32) -> Bit {
    Std9::ALL
        .get(usize::try_from(pos).unwrap_or(0))
        .copied()
        .unwrap_or(Std9::U)
        .to_bit()
}

impl<'a> Lowerer<'a, '_> {
    // --- entry points ------------------------------------------------------

    /// Lowers `e` and coerces it to `want`.
    pub(crate) fn expr_in(
        &mut self,
        e: &'a ast::Expr,
        want: &Layout,
        sink: &mut Sink<'_>,
    ) -> ExprId {
        match self.build(e, Some(want), sink) {
            Some((id, from)) => self.coerce(id, &from, want, e.span()),
            None => self.zero_of(want),
        }
    }

    /// Lowers `e` with its own layout.
    pub(crate) fn expr(
        &mut self,
        e: &'a ast::Expr,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        self.build(e, None, sink)
    }

    /// Lowers `e` to a single bit holding its truth value.
    pub(crate) fn cond(&mut self, e: &'a ast::Expr, sink: &mut Sink<'_>) -> ExprId {
        let Some((id, layout)) = self.build(e, None, sink) else {
            return self.b.const_bit(false);
        };
        if layout.width == 1 && layout.is_bits() {
            return id;
        }
        if !layout.is_bits() {
            self.error(codes::TYPE, e.span(), "this value has no truth value");
            return self.b.const_bit(false);
        }
        self.b.span = e.span();
        self.b.reduce_or(id)
    }

    /// A zero of the given layout, for error recovery.
    pub(crate) fn zero_of(&mut self, want: &Layout) -> ExprId {
        if want.is_bits() {
            self.b
                .constant(Logic::zero(want.width.max(1)).with_signed(want.signed))
        } else {
            self.b.string("")
        }
    }

    /// Inserts a `Resize` unless the node already has the wanted type.
    pub(crate) fn coerce(&mut self, id: ExprId, from: &Layout, to: &Layout, span: Span) -> ExprId {
        if !to.is_bits() || !from.is_bits() {
            return id;
        }
        let ty = self.b.module().expr(id).ty.clone();
        if ty
            == (Type::Bits {
                width: to.width,
                signed: to.signed,
            })
        {
            return id;
        }
        self.b.span = span;
        if let Some(c) = self.b.module().expr(id).as_const().cloned() {
            let resized = if to.signed {
                c.resize(to.width).with_signed(true)
            } else {
                c.as_unsigned().resize(to.width)
            };
            return self.b.constant(resized);
        }
        self.b.resize(id, to.width, to.signed)
    }

    /// A concatenation, folded to one constant when every part is one.
    ///
    /// `(others => '0')` and `x"F0" & "0011"` are constants in the source
    /// and should be constants in the IR too, not an eight-way concat of
    /// single bits.
    pub(crate) fn concat_parts(&mut self, parts: Vec<ExprId>) -> ExprId {
        let mut consts = Vec::with_capacity(parts.len());
        for p in &parts {
            match self.b.module().expr(*p).as_const() {
                Some(c) => consts.push(c.clone()),
                None => return self.b.concat(parts),
            }
        }
        if consts.is_empty() {
            return self.b.concat(parts);
        }
        self.b.constant(Logic::concat_all(consts.iter()))
    }

    /// A constant part-select, folded through a constant operand and
    /// through an enclosing slice (`r.f.g` is one slice of `r`).
    pub(crate) fn slice_bits(&mut self, base: ExprId, hi: u32, lo: u32) -> ExprId {
        if let Some(c) = self.b.module().expr(base).as_const().cloned() {
            return self.b.constant(c.slice(hi, lo));
        }
        if let ExprKind::Slice {
            base: inner,
            lo: outer_lo,
            ..
        } = self.b.module().expr(base).kind
        {
            return self.b.slice(inner, outer_lo + hi, outer_lo + lo);
        }
        self.b.slice(base, hi, lo)
    }

    /// Resizes a node to `width`, keeping its signedness.
    pub(crate) fn widen(&mut self, id: ExprId, width: u32, signed: bool) -> ExprId {
        let ty = self.b.module().expr(id).ty.clone();
        if ty == (Type::Bits { width, signed }) {
            return id;
        }
        if let Some(c) = self.b.module().expr(id).as_const().cloned() {
            let resized = if signed {
                c.resize(width).with_signed(true)
            } else {
                c.as_unsigned().resize(width)
            };
            return self.b.constant(resized);
        }
        self.b.resize(id, width, signed)
    }

    // --- layouts -----------------------------------------------------------

    /// The layout of the expression at `span`, from its analysed type.
    ///
    /// An expression whose type is unconstrained (the result of a function
    /// returning `std_ulogic_vector`, a string literal) has no layout of
    /// its own; the context supplies one, so nothing is reported here.
    pub(crate) fn layout_at(&mut self, span: Span) -> Option<Layout> {
        let ty = self.a().type_of(span)?;
        self.quiet += 1;
        let out = types::layout_of(self, ty, span);
        self.quiet -= 1;
        out
    }

    /// The layout of a subtype.
    pub(crate) fn layout_of_type(&mut self, ty: TypeId, span: Span) -> Option<Layout> {
        types::layout_of(self, ty, span)
    }

    /// The layout of a subtype, without reporting when it has none.
    pub(crate) fn layout_of_type_quiet(&mut self, ty: TypeId, span: Span) -> Option<Layout> {
        self.quiet += 1;
        let out = types::layout_of(self, ty, span);
        self.quiet -= 1;
        out
    }

    // --- constants ---------------------------------------------------------

    /// The IR constant a static value has under `layout`.
    pub(crate) fn const_of(&mut self, v: &Value, layout: &Layout) -> Option<Const> {
        match &layout.kind {
            LayoutKind::Bit(BitKind::StdLogic) => Some(Logic::from_bit(std9_bit(v.as_enum()?))),
            LayoutKind::Bit(_) => Some(Logic::from_u64(u64::from(v.as_enum()? & 1), 1)),
            LayoutKind::Enum(_) => Some(Logic::from_u64(u64::from(v.as_enum()?), layout.width)),
            LayoutKind::Int | LayoutKind::Physical => {
                let i = i64::try_from(v.as_int()?).ok()?;
                Some(Logic::from_i64(i, layout.width).with_signed(true))
            }
            LayoutKind::Array(arr) => {
                let av = v.as_array()?;
                if av.elems.len() != usize::try_from(arr.len).ok()? {
                    return None;
                }
                let mut parts = Vec::with_capacity(av.elems.len());
                for e in &av.elems {
                    parts.push(self.const_of(e, &arr.elem)?);
                }
                Some(Logic::concat_all(parts.iter()))
            }
            LayoutKind::Record(fields) => {
                let Value::Record(vs) = v else { return None };
                if vs.len() != fields.len() {
                    return None;
                }
                let mut parts = Vec::with_capacity(vs.len());
                for ((_, fl), fv) in fields.iter().zip(vs) {
                    parts.push(self.const_of(fv, fl)?);
                }
                Some(Logic::concat_all(parts.iter()))
            }
            LayoutKind::Real | LayoutKind::Str | LayoutKind::Opaque => None,
        }
    }

    /// The text of a `string` value, for report arguments.
    fn string_of(&self, v: &Value) -> Option<String> {
        let a = v.as_array()?;
        a.elems
            .iter()
            .map(|e| char::from_u32(e.as_enum()?))
            .collect()
    }

    // --- the main walk -----------------------------------------------------

    pub(crate) fn build(
        &mut self,
        e: &'a ast::Expr,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        self.b.span = e.span();
        // Anything the analyser folded, or that folds once the generics
        // are known, becomes one constant node.
        if let Some(layout) = self.expr_layout(e, want)
            && let Some(v) = self.eval(e)
        {
            if let LayoutKind::Str = layout.kind
                && let Some(s) = self.string_of(&v)
            {
                self.b.span = e.span();
                return Some((self.b.string(s), layout));
            }
            if let Some(c) = self.const_of(&v, &layout) {
                self.b.span = e.span();
                return Some((self.b.constant(c), layout));
            }
        }
        match e {
            ast::Expr::Paren { inner, .. } => self.build(inner, want, sink),
            ast::Expr::Qualified { operand, span, .. } => {
                let want = self.layout_at(*span).or_else(|| want.cloned());
                self.build(operand, want.as_ref(), sink)
            }
            ast::Expr::Name(n) => self.name_expr(n, want, sink),
            ast::Expr::Literal(_) => {
                self.error(
                    codes::NOT_STATIC,
                    e.span(),
                    "this literal cannot be evaluated",
                );
                None
            }
            ast::Expr::Aggregate(ag) => self.aggregate(ag, want, sink),
            ast::Expr::Unary { op, operand, span } => {
                self.unary_expr(*op, operand, *span, want, sink)
            }
            ast::Expr::Binary { op, lhs, rhs, span } => {
                self.binary_expr(*op, lhs, rhs, *span, want, sink)
            }
            ast::Expr::Allocator { span, .. } => {
                self.error(
                    codes::TYPE,
                    *span,
                    "an allocator needs an access type, which is not synthesisable",
                );
                None
            }
            ast::Expr::Open(span) => {
                self.error(codes::PORT_MAP, *span, "`open` is not a value");
                None
            }
            ast::Expr::Error(_) => None,
        }
    }

    /// The layout an expression should have: its own, or the context's.
    fn expr_layout(&mut self, e: &'a ast::Expr, want: Option<&Layout>) -> Option<Layout> {
        self.layout_at(e.span()).or_else(|| want.cloned())
    }

    // --- names -------------------------------------------------------------

    fn name_expr(
        &mut self,
        n: &'a ast::Name,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        match n {
            ast::Name::Simple(_) | ast::Name::Char { .. } | ast::Name::Operator { .. } => {
                self.object_value(n)
            }
            ast::Name::Selected {
                prefix,
                suffix,
                span,
            } => {
                // `lib.pkg.c` denotes an object or a literal; `s.field`
                // denotes a record element, which is a slice of `s`.
                if let Some(d) = self.a().decl_of(*span)
                    && matches!(
                        self.a().decl(d).kind,
                        DeclKind::Object { .. } | DeclKind::EnumLiteral { .. }
                    )
                {
                    return self.object_value(n);
                }
                let ast::Suffix::Designator(ast::Designator::Ident(field)) = suffix else {
                    return self.object_value(n);
                };
                let (base, layout) = self.name_expr(prefix, None, sink)?;
                let (hi, lo, fl) = match layout.field_bits(&field.name) {
                    Some((hi, lo, fl)) => (hi, lo, fl.clone()),
                    None => {
                        self.error(
                            codes::TYPE,
                            *span,
                            format!("`{}` is not a field of this record", field.name),
                        );
                        return None;
                    }
                };
                self.b.span = *span;
                Some((self.slice_bits(base, hi, lo), fl))
            }
            ast::Name::Call { prefix, args, span } => {
                self.call_expr(n, prefix, args, *span, want, sink)
            }
            ast::Name::Slice {
                prefix,
                range,
                span,
            } => self.slice_expr(prefix, range, *span, sink),
            ast::Name::Attribute { span, .. } => {
                self.error(
                    codes::NOT_STATIC,
                    *span,
                    "this attribute cannot be lowered to the IR",
                );
                None
            }
            ast::Name::External(ext) => {
                self.unsupported(ext.span, "an external name");
                None
            }
        }
    }

    /// The value of a name that denotes an object, constant or literal.
    fn object_value(&mut self, n: &'a ast::Name) -> Option<(ExprId, Layout)> {
        let span = n.span();
        let d = self.a().decl_of(span)?;
        match self.lookup(d).cloned() {
            Some(Binding::Net { net, layout }) => {
                self.b.span = span;
                Some((self.b.net(net), layout))
            }
            Some(Binding::Slice {
                net,
                hi,
                lo,
                layout,
            }) => {
                self.b.span = span;
                let base = self.b.net(net);
                Some((self.slice_bits(base, hi, lo), layout))
            }
            Some(Binding::Mem { .. }) => {
                self.error(
                    codes::TYPE,
                    span,
                    "an array of multi-bit elements can only be read one element at a time",
                );
                None
            }
            Some(Binding::Value { value, ty }) => {
                let layout = self.layout_of_type(ty, span)?;
                let c = self.const_of(&value, &layout)?;
                self.b.span = span;
                Some((self.b.constant(c), layout))
            }
            None => {
                // A constant or literal declared in a package is not bound
                // in a module's scope; its value comes from the analysis.
                if let Some(v) = constant_value(self.a(), d) {
                    let ty = self.a().decl_type(d)?;
                    let layout = self.layout_of_type(ty, span)?;
                    if let Some(c) = self.const_of(&v, &layout) {
                        self.b.span = span;
                        return Some((self.b.constant(c), layout));
                    }
                    if let LayoutKind::Str = layout.kind
                        && let Some(s) = self.string_of(&v)
                    {
                        self.b.span = span;
                        return Some((self.b.string(s), layout));
                    }
                }
                let spelling = self.a().decl(d).spelling.clone();
                self.error(
                    codes::NOT_STATIC,
                    span,
                    format!("`{spelling}` has no value at elaboration"),
                );
                None
            }
        }
    }

    // --- indexing and slicing ----------------------------------------------

    fn call_expr(
        &mut self,
        whole: &'a ast::Name,
        prefix: &'a ast::Name,
        args: &'a [ast::AssociationElement],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        match self.a().call_of(span).cloned() {
            Some(CallTarget::Index) => self.index_expr(prefix, args, span, sink),
            Some(CallTarget::Conversion(t)) => {
                let arg = first_expr(args)?;
                let (id, from) = self.build(arg, None, sink)?;
                // A conversion to an unconstrained array type takes its
                // length from the operand (clause 9.3.6), which is what
                // `std_logic_vector(count)` relies on.
                let to = match self.layout_of_type_quiet(t, span) {
                    Some(l) => l,
                    None if from.array().is_some() => Layout {
                        ty: t,
                        signed: types::is_signed_array(self.a(), t),
                        ..from.clone()
                    },
                    None => self.layout_of_type(t, span)?,
                };
                Some((self.convert(id, &from, &to, span), to))
            }
            Some(CallTarget::Subprogram(d)) => self.subprogram_expr(d, args, span, want, sink),
            Some(CallTarget::Attribute(p)) => {
                self.attribute_call(p, span);
                None
            }
            Some(CallTarget::Operator(sym)) => {
                // An operator written in functional form, `"and"(a, b)`.
                let mut it = args.iter().filter_map(|a| match &a.actual {
                    ast::Actual::Expr(e) => Some(e),
                    _ => None,
                });
                let lhs = it.next()?;
                match it.next() {
                    Some(rhs) => self.operator_sym(sym, Some(lhs), rhs, span, want, sink),
                    None => self.operator_sym(sym, None, lhs, span, want, sink),
                }
            }
            Some(CallTarget::Predefined(name)) => {
                self.unsupported(span, &format!("the predefined function `{name}`"));
                None
            }
            None => {
                let _ = whole;
                self.error(codes::UNSUPPORTED, span, "this call cannot be lowered");
                None
            }
        }
    }

    fn attribute_call(&mut self, p: Predefined, span: Span) {
        self.error(
            codes::NOT_STATIC,
            span,
            format!("the attribute `{p:?}` is not static after elaboration"),
        );
    }

    fn index_expr(
        &mut self,
        prefix: &'a ast::Name,
        args: &'a [ast::AssociationElement],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let idx = first_expr(args)?;
        // A memory is read through `MemRead`, everything else is a bit
        // select of a wide vector.
        if let Some(d) = self.a().decl_of(prefix.span())
            && let Some(Binding::Mem { mem, layout }) = self.lookup(d).cloned()
        {
            let arr = layout.array()?.clone();
            let addr = self.address(idx, &arr, sink)?;
            self.b.span = span;
            return Some((self.b.mem_read(mem, addr), arr.elem.clone()));
        }
        let (base, layout) = self.name_expr(prefix, None, sink)?;
        let Some(arr) = layout.array().cloned() else {
            self.error(codes::TYPE, span, "this value is not an array");
            return None;
        };
        if let Some(i) = self.eval_int(idx) {
            let i = i64::try_from(i).ok()?;
            let Some((hi, lo)) = arr.bits_of(i) else {
                self.error(codes::TYPE, span, format!("index {i} is out of range"));
                return None;
            };
            self.b.span = span;
            return Some((self.slice_bits(base, hi, lo), arr.elem.clone()));
        }
        let off = self.bit_offset(idx, &arr, sink)?;
        self.b.span = span;
        let id = self.b.indexed_slice(base, off, arr.elem.width, true);
        Some((id, arr.elem.clone()))
    }

    /// The memory address of `idx`: the VHDL index minus the low bound.
    fn address(
        &mut self,
        idx: &'a ast::Expr,
        arr: &ArrayLayout,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let (id, layout) = self.build(idx, None, sink)?;
        let low = arr.low();
        if low == 0 {
            return Some(id);
        }
        let width = layout.width.max(1);
        self.b.span = self.b.module().expr(id).span;
        let k = self
            .b
            .constant(Logic::from_i64(low, width).with_signed(true));
        Some(self.b.binary(BinaryOp::Sub, id, k))
    }

    /// The bit offset of element `idx` inside a wide vector.
    fn bit_offset(
        &mut self,
        idx: &'a ast::Expr,
        arr: &ArrayLayout,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let (id, layout) = self.build(idx, None, sink)?;
        let width = layout.width.max(32);
        let id = self.widen(id, width, layout.signed);
        self.b.span = self.b.module().expr(id).span;
        // `downto`: offset = i - right. `to`: offset = right - i.
        let right = match arr.dir {
            Direction::To => arr.left + i64::from(arr.len) - 1,
            Direction::Downto => arr.left - i64::from(arr.len) + 1,
        };
        let off = if right == 0 && arr.dir == Direction::Downto {
            id
        } else {
            let k = self
                .b
                .constant(Logic::from_i64(right, width).with_signed(true));
            match arr.dir {
                Direction::To => self.b.binary(BinaryOp::Sub, k, id),
                Direction::Downto => self.b.binary(BinaryOp::Sub, id, k),
            }
        };
        let ew = arr.elem.width;
        if ew == 1 {
            return Some(off);
        }
        let scale = self
            .b
            .constant(Logic::from_i64(i64::from(ew), width).with_signed(true));
        Some(self.b.binary(BinaryOp::Mul, off, scale))
    }

    fn slice_expr(
        &mut self,
        prefix: &'a ast::Name,
        range: &'a ast::DiscreteRange,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (base, layout) = self.name_expr(prefix, None, sink)?;
        let arr = layout.array()?.clone();
        let (left, right) = self.static_range(range)?;
        let Some((hi, _)) = arr.bits_of(left) else {
            self.error(codes::TYPE, span, "the slice is outside the array");
            return None;
        };
        let Some((_, lo)) = arr.bits_of(right) else {
            self.error(codes::TYPE, span, "the slice is outside the array");
            return None;
        };
        let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
        let out = self.layout_at(span)?;
        self.b.span = span;
        Some((self.slice_bits(base, hi, lo), out))
    }

    /// The static bounds of a discrete range, after elaboration.
    ///
    /// A range written out is evaluated from the tree, since a bound that
    /// mentions a generic is only a span in the analyser's tables; a range
    /// given as a subtype or a `'range` attribute comes from `range_of`.
    pub(crate) fn static_range(&mut self, r: &'a ast::DiscreteRange) -> Option<(i64, i64)> {
        if let ast::DiscreteRange::Range(ast::Range::Bounds { left, right, .. }) = r {
            let l = self.eval_int(left)?;
            let rr = self.eval_int(right)?;
            return Some((i64::try_from(l).ok()?, i64::try_from(rr).ok()?));
        }
        let info = self.a().range_of(r.span()).cloned()?;
        if let Some((l, rr)) = info.bounds.ints() {
            return Some((i64::try_from(l).ok()?, i64::try_from(rr).ok()?));
        }
        let left = self.bound_value(&info.bounds.left)?;
        let right = self.bound_value(&info.bounds.right)?;
        Some((left, right))
    }

    /// The direction of a discrete range.
    pub(crate) fn range_direction(&mut self, r: &'a ast::DiscreteRange) -> Direction {
        if let ast::DiscreteRange::Range(ast::Range::Bounds { direction, .. }) = r {
            return *direction;
        }
        self.a()
            .range_of(r.span())
            .map_or(Direction::To, |i| i.bounds.dir)
    }

    pub(crate) fn bound_value(&mut self, b: &crate::vhdl::sema::Bound) -> Option<i64> {
        match b {
            crate::vhdl::sema::Bound::Static(v) => i64::try_from(v.as_int()?).ok(),
            crate::vhdl::sema::Bound::Dynamic(span) => {
                let e = self.cx.ast.bound(*span)?;
                i64::try_from(self.eval_int(e)?).ok()
            }
        }
    }

    // --- conversions -------------------------------------------------------

    /// A VHDL type conversion between closely related types.
    fn convert(&mut self, id: ExprId, from: &Layout, to: &Layout, span: Span) -> ExprId {
        if !from.is_bits() || !to.is_bits() {
            return id;
        }
        // Integer <-> physical and array <-> array of the same length are
        // reinterpretations; only the width and signedness may change.
        self.coerce(id, from, to, span)
    }

    // --- aggregates --------------------------------------------------------

    fn aggregate(
        &mut self,
        ag: &'a ast::Aggregate,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let layout = self
            .layout_at(ag.span)
            .or_else(|| want.cloned())
            .or_else(|| {
                self.error(
                    codes::TYPE,
                    ag.span,
                    "the type of this aggregate is unknown",
                );
                None
            })?;
        if let Some(fields) = layout.record().map(<[(String, Layout)]>::to_vec) {
            return self.record_aggregate(ag, &layout, &fields, sink);
        }
        let Some(arr) = layout.array().cloned() else {
            self.error(
                codes::TYPE,
                ag.span,
                "this aggregate is not an array or record",
            );
            return None;
        };
        self.array_aggregate(ag, &layout, &arr, sink)
    }

    fn record_aggregate(
        &mut self,
        ag: &'a ast::Aggregate,
        layout: &Layout,
        fields: &[(String, Layout)],
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let mut values: Vec<Option<ExprId>> = vec![None; fields.len()];
        let mut others: Option<&'a ast::Expr> = None;
        let mut positional = 0usize;
        for el in &ag.elements {
            if el.choices.is_empty() {
                if positional < fields.len() {
                    let v = self.expr_in(&el.value, &fields[positional].1, sink);
                    values[positional] = Some(v);
                }
                positional += 1;
                continue;
            }
            for c in &el.choices {
                match c {
                    ast::Choice::Others(_) => others = Some(&el.value),
                    ast::Choice::Expr(ast::Expr::Name(ast::Name::Simple(id))) => {
                        if let Some(i) = fields
                            .iter()
                            .position(|(n, _)| n.eq_ignore_ascii_case(&id.name))
                        {
                            let v = self.expr_in(&el.value, &fields[i].1, sink);
                            values[i] = Some(v);
                        }
                    }
                    other => {
                        self.unsupported(other.span(), "this record aggregate choice");
                    }
                }
            }
        }
        let mut parts = Vec::with_capacity(fields.len());
        for (i, (name, fl)) in fields.iter().enumerate() {
            match values[i] {
                Some(v) => parts.push(v),
                None => match others {
                    Some(e) => {
                        let v = self.expr_in(e, fl, sink);
                        parts.push(v);
                    }
                    None => {
                        self.error(
                            codes::TYPE,
                            ag.span,
                            format!("field `{name}` has no value in this aggregate"),
                        );
                        return None;
                    }
                },
            }
        }
        self.b.span = ag.span;
        Some((self.concat_parts(parts), layout.clone()))
    }

    fn array_aggregate(
        &mut self,
        ag: &'a ast::Aggregate,
        layout: &Layout,
        arr: &ArrayLayout,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let len = usize::try_from(arr.len).ok()?;
        let mut values: Vec<Option<ExprId>> = vec![None; len];
        let mut others: Option<&'a ast::Expr> = None;
        let mut positional = 0usize;
        for el in &ag.elements {
            if el.choices.is_empty() {
                if positional < len {
                    let v = self.expr_in(&el.value, &arr.elem, sink);
                    values[positional] = Some(v);
                }
                positional += 1;
                continue;
            }
            for c in &el.choices {
                match c {
                    ast::Choice::Others(_) => others = Some(&el.value),
                    ast::Choice::Expr(e) => {
                        let Some(i) = self.eval_int(e) else {
                            self.error(
                                codes::NOT_STATIC,
                                e.span(),
                                "an aggregate choice must be static",
                            );
                            continue;
                        };
                        let Some(off) = i64::try_from(i).ok().and_then(|i| arr.offset(i)) else {
                            self.error(codes::TYPE, e.span(), "this choice is out of range");
                            continue;
                        };
                        let v = self.expr_in(&el.value, &arr.elem, sink);
                        values[usize::try_from(off).ok()?] = Some(v);
                    }
                    ast::Choice::Range(r) => {
                        let Some((l, rr)) = self.static_range(r) else {
                            self.error(
                                codes::NOT_STATIC,
                                r.span(),
                                "an aggregate choice range must be static",
                            );
                            continue;
                        };
                        let v = self.expr_in(&el.value, &arr.elem, sink);
                        let (lo, hi) = if l <= rr { (l, rr) } else { (rr, l) };
                        for i in lo..=hi {
                            if let Some(off) = arr.offset(i)
                                && let Ok(off) = usize::try_from(off)
                            {
                                values[off] = Some(v);
                            }
                        }
                    }
                }
            }
        }
        let mut parts = Vec::with_capacity(len);
        for (off, slot) in values.into_iter().enumerate() {
            match slot {
                Some(v) => parts.push(v),
                None => match others {
                    Some(e) => {
                        let v = self.expr_in(e, &arr.elem, sink);
                        parts.push(v);
                    }
                    None => {
                        let _ = off;
                        self.error(
                            codes::TYPE,
                            ag.span,
                            "this aggregate does not give every element a value",
                        );
                        return None;
                    }
                },
            }
        }
        self.b.span = ag.span;
        Some((self.concat_parts(parts), layout.clone()))
    }

    // --- operators ---------------------------------------------------------

    fn unary_expr(
        &mut self,
        op: ast::UnaryOp,
        operand: &'a ast::Expr,
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        match self.a().call_of(span).cloned() {
            Some(CallTarget::Subprogram(d)) => {
                self.subprogram_operator(d, &[operand], span, want, sink)
            }
            _ => self.operator_sym(op.as_str(), None, operand, span, want, sink),
        }
    }

    fn binary_expr(
        &mut self,
        op: ast::BinaryOp,
        lhs: &'a ast::Expr,
        rhs: &'a ast::Expr,
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        match self.a().call_of(span).cloned() {
            Some(CallTarget::Subprogram(d)) => {
                self.subprogram_operator(d, &[lhs, rhs], span, want, sink)
            }
            _ => self.operator_sym(op.as_str(), Some(lhs), rhs, span, want, sink),
        }
    }

    /// Lowers a predefined operator, given its symbol.
    ///
    /// `want` is the layout the context expects, which is what gives an
    /// operand of an unconstrained type (a string literal, the result of a
    /// function returning `std_ulogic_vector`) its width.
    pub(crate) fn operator_sym(
        &mut self,
        sym: &str,
        lhs: Option<&'a ast::Expr>,
        rhs: &'a ast::Expr,
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let result = self.layout_at(span).or_else(|| {
            if is_predicate(sym) {
                Some(self.boolean_layout())
            } else {
                want.cloned()
            }
        });
        let Some(lhs) = lhs else {
            return self.unary_op(sym, rhs, span, result, sink);
        };
        // A predicate's own layout is one bit, so it says nothing about
        // the operands; everything else shares the result's.
        let first = if is_predicate(sym) {
            None
        } else {
            result.clone()
        };
        let (mut l, ll) = self.build(lhs, first.as_ref(), sink)?;
        let second = first.clone().or_else(|| Some(ll.clone()));
        let (mut r, rl) = self.build(rhs, second.as_ref(), sink)?;
        if !ll.is_bits() || !rl.is_bits() {
            self.error(codes::TYPE, span, "this operator needs bit-vector operands");
            return None;
        }
        let result = match result {
            Some(r) => r,
            None if is_predicate(sym) => self.boolean_layout(),
            None => ll.clone(),
        };
        self.b.span = span;
        if sym == "&" {
            // The concatenation's width is the sum, whatever subtype the
            // analyser gave the result; the caller coerces it.
            let id = self.concat_parts(vec![l, r]);
            let from = Layout {
                width: ll.width + rl.width,
                ..result
            };
            return Some((id, from));
        }
        // A scalar against a vector (VHDL-2008 `std_logic and slv`)
        // replicates the scalar.
        if ll.width != rl.width && ll.array().is_some() && rl.array().is_none() {
            r = self.b.replicate(ll.width, r);
        } else if ll.width != rl.width && rl.array().is_some() && ll.array().is_none() {
            l = self.b.replicate(rl.width, l);
        }
        let signed = ll.signed || rl.signed;
        let width = ll.width.max(rl.width);
        let is_shift = matches!(sym, "sll" | "srl" | "sla" | "sra" | "rol" | "ror");
        if !is_shift {
            l = self.widen(l, width, signed);
            r = self.widen(r, width, signed);
        }
        self.b.span = span;
        let id = match sym {
            "and" => self.b.binary(BinaryOp::And, l, r),
            "or" => self.b.binary(BinaryOp::Or, l, r),
            "xor" => self.b.binary(BinaryOp::Xor, l, r),
            "xnor" => self.b.binary(BinaryOp::Xnor, l, r),
            "nand" => {
                let a = self.b.binary(BinaryOp::And, l, r);
                self.b.not(a)
            }
            "nor" => {
                let a = self.b.binary(BinaryOp::Or, l, r);
                self.b.not(a)
            }
            "=" => self.b.binary(BinaryOp::Eq, l, r),
            "/=" => self.b.binary(BinaryOp::Ne, l, r),
            "<" => self.b.binary(BinaryOp::Lt, l, r),
            "<=" => self.b.binary(BinaryOp::Le, l, r),
            ">" => self.b.binary(BinaryOp::Gt, l, r),
            ">=" => self.b.binary(BinaryOp::Ge, l, r),
            "?=" => self.b.binary(BinaryOp::CaseEq, l, r),
            "?/=" => self.b.binary(BinaryOp::CaseNe, l, r),
            "?<" => self.b.binary(BinaryOp::Lt, l, r),
            "?<=" => self.b.binary(BinaryOp::Le, l, r),
            "?>" => self.b.binary(BinaryOp::Gt, l, r),
            "?>=" => self.b.binary(BinaryOp::Ge, l, r),
            "+" => self.b.binary(BinaryOp::Add, l, r),
            "-" => self.b.binary(BinaryOp::Sub, l, r),
            "*" => self.b.binary(BinaryOp::Mul, l, r),
            "/" => self.b.binary(BinaryOp::Div, l, r),
            "rem" => self.b.binary(BinaryOp::Mod, l, r),
            "mod" => self.vhdl_mod(l, r, width, signed),
            "**" => self.b.binary(BinaryOp::Pow, l, r),
            "sll" | "sla" => self.b.binary(BinaryOp::Shl, l, r),
            "srl" => self.b.binary(BinaryOp::Shr, l, r),
            "sra" => self.b.binary(BinaryOp::Sshr, l, r),
            "rol" | "ror" => {
                self.unsupported(span, "the rotate operators");
                return None;
            }
            other => {
                self.unsupported(span, &format!("the operator `{other}`"));
                return None;
            }
        };
        let from = self.node_layout(id, &result);
        Some((self.coerce(id, &from, &result, span), result))
    }

    /// VHDL `mod` takes the sign of the divisor; the IR's `Mod` takes the
    /// sign of the dividend, so the correction is explicit.
    fn vhdl_mod(&mut self, l: ExprId, r: ExprId, width: u32, signed: bool) -> ExprId {
        let rem = self.b.binary(BinaryOp::Mod, l, r);
        if !signed {
            return rem;
        }
        let zero = self.b.constant(Logic::from_i64(0, width).with_signed(true));
        let nonzero = self.b.binary(BinaryOp::Ne, rem, zero);
        let rem_neg = self.b.binary(BinaryOp::Lt, rem, zero);
        let div_neg = self.b.binary(BinaryOp::Lt, r, zero);
        let differ = self.b.binary(BinaryOp::Xor, rem_neg, div_neg);
        let need = self.b.binary(BinaryOp::LogicAnd, nonzero, differ);
        let adjusted = self.b.binary(BinaryOp::Add, rem, r);
        self.b.mux(need, adjusted, rem)
    }

    fn unary_op(
        &mut self,
        sym: &str,
        operand: &'a ast::Expr,
        span: Span,
        result: Option<Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (id, ol) = self.build(operand, result.as_ref(), sink)?;
        let result = match result {
            Some(r) => r,
            None if matches!(sym, "??" | "and" | "or" | "nand" | "nor" | "xor" | "xnor") => {
                self.boolean_layout()
            }
            None => ol.clone(),
        };
        self.b.span = span;
        let out = match sym {
            "+" => id,
            "-" => self.b.neg(id),
            "abs" => {
                let zero = self
                    .b
                    .constant(Logic::from_i64(0, ol.width.max(1)).with_signed(ol.signed));
                let neg = self.b.lt(id, zero);
                let inv = self.b.neg(id);
                self.b.mux(neg, inv, id)
            }
            "not" => self.b.not(id),
            "??" => {
                if ol.width == 1 {
                    id
                } else {
                    self.b.reduce_or(id)
                }
            }
            "and" => self.b.unary(IrUnary::ReduceAnd, id),
            "or" => self.b.unary(IrUnary::ReduceOr, id),
            "xor" => self.b.unary(IrUnary::ReduceXor, id),
            "nand" => self.b.unary(IrUnary::ReduceNand, id),
            "nor" => self.b.unary(IrUnary::ReduceNor, id),
            "xnor" => self.b.unary(IrUnary::ReduceXnor, id),
            other => {
                self.unsupported(span, &format!("the unary operator `{other}`"));
                return None;
            }
        };
        let from = self.node_layout(out, &result);
        Some((self.coerce(out, &from, &result, span), result))
    }

    /// The layout of a freshly built node: the result layout with the
    /// node's real width and signedness.
    fn node_layout(&self, id: ExprId, like: &Layout) -> Layout {
        let ty = &self.b.module().expr(id).ty;
        Layout {
            width: ty.width().unwrap_or(like.width),
            signed: ty.is_signed(),
            ..like.clone()
        }
    }

    // --- subprogram calls ---------------------------------------------------

    /// An operator that resolved to a subprogram: a bundled `ieee`
    /// operator, or a user-defined one that is inlined.
    fn subprogram_operator(
        &mut self,
        d: DeclId,
        args: &[&'a ast::Expr],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if let Some(sym) = self.bundled_operator(d) {
            let sym: &str = sym;
            return match args {
                [only] => self.operator_sym(sym, None, only, span, want, sink),
                [lhs, rhs] => self.operator_sym(sym, Some(lhs), rhs, span, want, sink),
                _ => None,
            };
        }
        if let Some(pkg) = self.arith_package(d) {
            return self.numeric_call(pkg, d, args, span, want, sink);
        }
        let actuals: Vec<Actual<'a>> = args.iter().map(|e| Actual::Expr(e)).collect();
        self.inline_function(d, &actuals, span, want, sink)
    }

    fn subprogram_expr(
        &mut self,
        d: DeclId,
        args: &'a [ast::AssociationElement],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if let Some(sym) = self.bundled_operator(d) {
            let sym: &str = sym;
            let mut it = args.iter().filter_map(|a| match &a.actual {
                ast::Actual::Expr(e) => Some(e),
                _ => None,
            });
            let first = it.next()?;
            return match it.next() {
                Some(second) => self.operator_sym(sym, Some(first), second, span, want, sink),
                None => self.operator_sym(sym, None, first, span, want, sink),
            };
        }
        if let Some((id, layout)) = self.bundled_function(d, args, span, sink) {
            return Some((id, layout));
        }
        if let Some(pkg) = self.arith_package(d) {
            return self.numeric_assoc_call(pkg, d, args, span, want, sink);
        }
        let actuals: Vec<Actual<'a>> = args.iter().map(Actual::Assoc).collect();
        self.inline_function(d, &actuals, span, want, sink)
    }

    /// The operator symbol of a bundled `ieee.std_logic_1164` operator, so
    /// it can be lowered like the predefined one rather than inlined.
    pub(crate) fn bundled_operator(&self, d: DeclId) -> Option<&'static str> {
        let d = super::lower::resolve_alias(self.a(), d);
        if !self.is_std_logic_1164(d) {
            return None;
        }
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        let sym = name.trim_matches('"');
        [
            "and", "or", "nand", "nor", "xor", "xnor", "not", "sll", "srl", "rol", "ror", "?=",
            "?/=", "?<", "?<=", "?>", "?>=", "??",
        ]
        .into_iter()
        .find(|s| *s == sym)
    }

    /// True when the declaration comes from `ieee.std_logic_1164`.
    pub(crate) fn is_std_logic_1164(&self, d: DeclId) -> bool {
        let d = super::lower::resolve_alias(self.a(), d);
        let Some(pkg) = self.a().unit("ieee", "std_logic_1164") else {
            return false;
        };
        let Some(region) = self.a().units[pkg.index()].region else {
            return false;
        };
        self.a().decl(d).region == region
    }

    /// The conversions and testers of `ieee.std_logic_1164`, which are
    /// identities or single nodes under Reticle's encoding.
    fn bundled_function(
        &mut self,
        d: DeclId,
        args: &'a [ast::AssociationElement],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if !self.is_std_logic_1164(d) {
            return None;
        }
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        let arg = first_expr(args)?;
        match name.as_str() {
            "to_bit"
            | "to_bitvector"
            | "to_bit_vector"
            | "to_stdulogic"
            | "to_stdlogicvector"
            | "to_stdulogicvector"
            | "to_std_logic_vector"
            | "to_std_ulogic_vector"
            | "to_x01"
            | "to_x01z"
            | "to_ux01"
            | "to_01"
            | "resolved" => {
                let (id, from) = self.build(arg, None, sink)?;
                let to = self.layout_at(span).unwrap_or_else(|| from.clone());
                Some((self.coerce(id, &from, &to, span), to))
            }
            "is_x" => {
                let (id, _) = self.build(arg, None, sink)?;
                let to = self.layout_at(span)?;
                self.b.span = span;
                Some((self.b.call("$isunknown", vec![id], Type::bit()), to))
            }
            "to_string" | "to_bstring" | "to_ostring" | "to_hstring" => {
                let (id, _) = self.build(arg, None, sink)?;
                let to = self.layout_at(span)?;
                self.b.span = span;
                Some((self.b.call(name, vec![id], Type::String), to))
            }
            "rising_edge" | "falling_edge" => {
                self.error(
                    codes::UNSUPPORTED,
                    span,
                    format!("`{name}` is only recognised as the clock condition of a process"),
                );
                let layout = self.boolean_layout();
                self.b.span = span;
                Some((self.b.const_bit(false), layout))
            }
            _ => None,
        }
    }

    /// Reports a call into a package Reticle does not bundle.
    pub(crate) fn report_not_bundled(&mut self, d: DeclId, span: Span) -> bool {
        let region = self.a().decl(d).region;
        for (lib, pkg, note) in crate::vhdl::stdlib::MISSING {
            if let Some(u) = self.a().unit(lib, pkg)
                && self.a().units[u.index()].region == Some(region)
            {
                let name = self.a().decl(d).spelling.clone();
                self.report(
                    Diagnostic::error(format!("`{name}` comes from `{lib}.{pkg}`"))
                        .with_code(codes::NOT_BUNDLED)
                        .with_span(span)
                        .with_note(*note),
                );
                return true;
            }
        }
        false
    }

    // --- lvalues -----------------------------------------------------------

    /// Lowers an assignment target.
    pub(crate) fn lvalue(
        &mut self,
        target: &'a ast::Target,
        sink: &mut Sink<'_>,
    ) -> Option<(Lvalue, Layout)> {
        match target {
            ast::Target::Name(n) => self.lvalue_name(n, sink),
            ast::Target::Aggregate(ag) => {
                let mut parts = Vec::new();
                let mut width = 0;
                for el in &ag.elements {
                    let t = ast::Target::Name(match &el.value {
                        ast::Expr::Name(n) => n.clone(),
                        other => {
                            self.unsupported(other.span(), "this aggregate target");
                            return None;
                        }
                    });
                    // The cloned name is only used for its spans, which
                    // are the same as the original's.
                    let (lv, l) = self.lvalue_name(
                        match &el.value {
                            ast::Expr::Name(n) => n,
                            _ => return None,
                        },
                        sink,
                    )?;
                    let _ = t;
                    width += l.width;
                    parts.push(lv);
                }
                let layout = self.layout_at(ag.span).map(|l| Layout { width, ..l })?;
                Some((Lvalue::Concat(parts), layout))
            }
        }
    }

    pub(crate) fn lvalue_name(
        &mut self,
        n: &'a ast::Name,
        sink: &mut Sink<'_>,
    ) -> Option<(Lvalue, Layout)> {
        match n {
            ast::Name::Simple(_) | ast::Name::Operator { .. } | ast::Name::Char { .. } => {
                let d = self.a().decl_of(n.span())?;
                match self.lookup(d).cloned() {
                    Some(Binding::Net { net, layout }) => Some((Lvalue::Net(net), layout)),
                    Some(Binding::Slice {
                        net,
                        hi,
                        lo,
                        layout,
                    }) => Some((Lvalue::Slice { net, hi, lo }, layout)),
                    Some(Binding::Mem { .. }) => {
                        self.error(
                            codes::TYPE,
                            n.span(),
                            "a whole memory cannot be assigned at once",
                        );
                        None
                    }
                    _ => {
                        let spelling = self.a().decl(d).spelling.clone();
                        self.error(
                            codes::TYPE,
                            n.span(),
                            format!("`{spelling}` cannot be assigned"),
                        );
                        None
                    }
                }
            }
            ast::Name::Selected {
                prefix,
                suffix,
                span,
            } => {
                let ast::Suffix::Designator(ast::Designator::Ident(field)) = suffix else {
                    return None;
                };
                let (base, layout) = self.lvalue_name(prefix, sink)?;
                let Lvalue::Net(net) = base else {
                    self.unsupported(*span, "a nested record target");
                    return None;
                };
                let (hi, lo, fl) = layout
                    .field_bits(&field.name)
                    .map(|(h, l, f)| (h, l, f.clone()))?;
                Some((Lvalue::Slice { net, hi, lo }, fl))
            }
            ast::Name::Call { prefix, args, span } => {
                let idx = first_expr(args)?;
                if let Some(d) = self.a().decl_of(prefix.span())
                    && let Some(Binding::Mem { mem, layout }) = self.lookup(d).cloned()
                {
                    let arr = layout.array()?.clone();
                    let addr = self.address(idx, &arr, sink)?;
                    return Some((Lvalue::MemElem { mem, addr }, arr.elem.clone()));
                }
                let (base, layout) = self.lvalue_name(prefix, sink)?;
                let Lvalue::Net(net) = base else {
                    self.unsupported(*span, "a nested array target");
                    return None;
                };
                let arr = layout.array()?.clone();
                if let Some(i) = self.eval_int(idx) {
                    let i = i64::try_from(i).ok()?;
                    let (hi, lo) = arr.bits_of(i)?;
                    return Some((Lvalue::Slice { net, hi, lo }, arr.elem.clone()));
                }
                if arr.elem.width != 1 {
                    self.unsupported(*span, "a dynamic index into an array of multi-bit elements");
                    return None;
                }
                let index = self.bit_offset(idx, &arr, sink)?;
                Some((Lvalue::Index { net, index }, arr.elem.clone()))
            }
            ast::Name::Slice {
                prefix,
                range,
                span,
            } => {
                let (base, layout) = self.lvalue_name(prefix, sink)?;
                let Lvalue::Net(net) = base else {
                    self.unsupported(*span, "a nested slice target");
                    return None;
                };
                let arr = layout.array()?.clone();
                let Some((l, r)) = self.static_range(range) else {
                    self.error(
                        codes::NOT_STATIC,
                        range.span(),
                        "a slice assigned to must have bounds that are static after elaboration",
                    );
                    return None;
                };
                let (hi, _) = arr.bits_of(l)?;
                let (_, lo) = arr.bits_of(r)?;
                let (hi, lo) = if hi >= lo { (hi, lo) } else { (lo, hi) };
                // The analyser has a layout for the slice's own subtype
                // unless its bounds came from a generic; the slice of a bit
                // vector is then the base with the width just computed,
                // rather than an assignment silently dropped.
                let out = match self.layout_at(*span) {
                    Some(out) => out,
                    None if arr.elem.width == 1 => Layout {
                        width: hi - lo + 1,
                        ..layout
                    },
                    None => {
                        self.unsupported(*span, "this slice of an array of multi-bit elements");
                        return None;
                    }
                };
                Some((Lvalue::Slice { net, hi, lo }, out))
            }
            ast::Name::Attribute { span, .. } => {
                self.unsupported(*span, "an attribute as an assignment target");
                None
            }
            ast::Name::External(ext) => {
                self.unsupported(ext.span, "an external name as an assignment target");
                None
            }
        }
    }
}

/// An actual argument of a subprogram call.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Actual<'a> {
    /// A bare expression (an operator operand).
    Expr(&'a ast::Expr),
    /// An association element, possibly named.
    Assoc(&'a ast::AssociationElement),
}

impl<'a> Actual<'a> {
    /// The actual's expression, if it has one.
    pub(crate) fn expr(self) -> Option<&'a ast::Expr> {
        match self {
            Actual::Expr(e) => Some(e),
            Actual::Assoc(a) => match &a.actual {
                ast::Actual::Expr(e) | ast::Actual::Inertial(e) => Some(e),
                _ => None,
            },
        }
    }

    /// The formal the actual names, if it is a named association.
    pub(crate) fn formal(self) -> Option<&'a ast::Expr> {
        match self {
            Actual::Expr(_) => None,
            Actual::Assoc(a) => a.formal.as_ref(),
        }
    }
}

/// True for the operators whose result is one bit whatever the operands
/// are: the comparisons and the VHDL-2008 condition operator.
fn is_predicate(sym: &str) -> bool {
    matches!(sym, "=" | "/=" | "<" | "<=" | ">" | ">=" | "??")
}

/// The static value of a constant or enumeration-literal declaration.
fn constant_value(a: &crate::vhdl::sema::Analysis, d: DeclId) -> Option<Value> {
    match a.decl(d).kind {
        DeclKind::EnumLiteral { pos, .. } => Some(Value::Enum(pos)),
        DeclKind::Object {
            class: crate::vhdl::sema::ObjectClass::Constant,
            ..
        } => a.decl_value(d).cloned(),
        _ => None,
    }
}

/// The first positional expression of an association list.
pub(crate) fn first_expr(args: &[ast::AssociationElement]) -> Option<&ast::Expr> {
    args.iter().find_map(|a| match &a.actual {
        ast::Actual::Expr(e) => Some(e),
        _ => None,
    })
}
