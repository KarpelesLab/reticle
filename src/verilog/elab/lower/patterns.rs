//! Assignment patterns and whole unpacked arrays (IEEE 1800-2017 §10.9,
//! §7.6).
//!
//! An assignment pattern `'{..}` takes its shape from where it goes: the
//! type of the assignment's target, of a typed context, or of a cast. A
//! packed struct's pattern is its members' values, by position, by name or
//! by `default`; a packed array's is its elements', by position, by index
//! or by `default`. Each item is lowered as a value of its member's or
//! element's type, and the whole is their concatenation, the first member
//! or the left element the most significant.
//!
//! An unpacked array is a memory here, and a memory has no value as a
//! whole: an assignment to a whole array is one assignment per element
//! (from a pattern, or from an array of the same shape), and `==` or `!=`
//! of two arrays the conjunction of their elements' comparisons. Arrays of
//! more than [`ELEMENTS`] elements are refused, since each element is a
//! statement or a comparison of its own.

use crate::ir::builder::BlockBuilder;
use crate::ir::{self, AssignKind, ExprId, MemoryId};
use crate::logic::Logic;
use crate::source::Span;
use crate::verilog::ast::{self, Expr, ExprKind};

use super::super::codes;
use super::super::constant::Evaluator;
use super::super::scope::Symbol;
use super::super::types::{Packed, Range, VType};
use super::Lowerer;
use super::expr::Sink;

/// The most elements a whole-array assignment or comparison is spelled
/// out for.
const ELEMENTS: u64 = 4096;

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// The value of the pattern `'{items}` as one of type `ty`, or `None`
    /// (reported) when the pattern does not fit the type.
    pub(super) fn pattern_value(
        &mut self,
        items: &'ast [ast::PatternItem],
        ty: &VType,
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let VType::Packed(p) = ty else {
            self.env.unsupported(
                span,
                "an assignment pattern for something that is not a vector, struct or array",
            );
            return None;
        };
        if let Some(fields) = p.fields.clone() {
            return self.struct_pattern(items, p, &fields, span, sink);
        }
        // A packed array or a vector: its elements, left first.
        let outer = p.outer();
        let elem = VType::Packed(p.element());
        let n = usize::try_from(outer.len()).unwrap_or(usize::MAX);
        let mut slots: Vec<Option<&'ast Expr>> = vec![None; n];
        let mut default = None;
        let mut position = 0usize;
        for (key, value) in self.pattern_items(items) {
            match key {
                None => {
                    if position >= n {
                        self.too_many(span, n);
                        return None;
                    }
                    slots[position] = Some(value);
                    position += 1;
                }
                Some(k) if matches!(k.kind, ExprKind::Default) => default = Some(value),
                Some(k) => {
                    let index = self.env.probe(|env| Evaluator::new(env).eval_i64(k).ok());
                    let at = index.and_then(|i| left_position(outer, i));
                    let Some(at) = at else {
                        self.env.error(
                            codes::TYPE,
                            k.span,
                            format!("this key is not an index of {outer}"),
                        );
                        return None;
                    };
                    slots[at] = Some(value);
                }
            }
        }
        if position != 0 && position != n {
            self.env.error(
                codes::TYPE,
                span,
                format!("the pattern has {position} elements and the type {n}"),
            );
            return None;
        }
        let mut parts = Vec::with_capacity(n);
        for slot in slots {
            let Some(src) = slot.or(default) else {
                self.env
                    .error(codes::TYPE, span, "an element of the pattern is not given");
                return None;
            };
            parts.push(self.value_as(src, &elem, sink));
        }
        self.b.span = span;
        let whole = if parts.len() == 1 {
            parts[0]
        } else {
            self.b.concat(parts)
        };
        Some(self.coerce(whole, p.width(), p.signed))
    }

    /// A pattern's items as keys and values, a replication `'{n{a, b}}`
    /// spelled out as `n` times its elements.
    fn pattern_items(
        &mut self,
        items: &'ast [ast::PatternItem],
    ) -> Vec<(Option<&'ast Expr>, &'ast Expr)> {
        if let [one] = items
            && one.key.is_none()
            && let ExprKind::Replicate { count, elems } = &one.value.kind
            && let Some(n) = self
                .env
                .probe(|env| Evaluator::new(env).eval_u32(count).ok())
        {
            let mut out = Vec::new();
            for _ in 0..n {
                out.extend(elems.iter().map(|e| (None, e)));
            }
            return out;
        }
        items.iter().map(|i| (i.key.as_ref(), &i.value)).collect()
    }

    fn too_many(&mut self, span: Span, n: usize) {
        self.env.error(
            codes::TYPE,
            span,
            format!("the pattern has more elements than the type's {n}"),
        );
    }

    /// A packed struct's or union's pattern: each member from its
    /// position, its name or the `default`.
    fn struct_pattern(
        &mut self,
        items: &'ast [ast::PatternItem],
        p: &Packed,
        fields: &[super::super::types::Field],
        span: Span,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let mut positional: Vec<&'ast Expr> = Vec::new();
        let mut named: Vec<(&'ast str, &'ast Expr)> = Vec::new();
        let mut default = None;
        for item in items {
            match &item.key {
                None => positional.push(&item.value),
                Some(k) => match &k.kind {
                    ExprKind::Default => default = Some(&item.value),
                    ExprKind::Ident(id) => named.push((id.name.as_str(), &item.value)),
                    _ => {
                        self.env
                            .unsupported(item.span, "a typed assignment pattern key");
                        return None;
                    }
                },
            }
        }
        // A union's members overlap: its pattern gives one of them.
        let union = fields.len() > 1 && fields.iter().all(|f| f.lsb == 0);
        let mut chosen: Vec<(&super::super::types::Field, &'ast Expr)> = Vec::new();
        if union {
            let first = positional
                .first()
                .map(|e| (&fields[0], *e))
                .or_else(|| {
                    named
                        .first()
                        .and_then(|(n, e)| fields.iter().find(|f| f.name == *n).map(|f| (f, *e)))
                })
                .or_else(|| default.map(|e| (&fields[0], e)));
            let Some(one) = first else {
                self.env.error(
                    codes::TYPE,
                    span,
                    "the pattern gives no member of the union",
                );
                return None;
            };
            chosen.push(one);
        } else {
            if !positional.is_empty() && positional.len() != fields.len() {
                self.env.error(
                    codes::TYPE,
                    span,
                    "a positional pattern gives every member of the struct",
                );
                return None;
            }
            for (i, f) in fields.iter().enumerate() {
                let src = positional
                    .get(i)
                    .copied()
                    .or_else(|| named.iter().find(|(n, _)| *n == f.name).map(|(_, e)| *e))
                    .or(default);
                let Some(src) = src else {
                    self.env.error(
                        codes::TYPE,
                        span,
                        format!("struct member `{}` is not given", f.name),
                    );
                    return None;
                };
                chosen.push((f, src));
            }
        }
        // Most significant member first.
        chosen.sort_by_key(|c| std::cmp::Reverse(c.0.lsb));
        let mut parts = Vec::with_capacity(chosen.len() + 1);
        let mut top = p.width();
        for (f, src) in chosen {
            let fw = f.ty.packed().map_or(1, Packed::width);
            if f.lsb + fw < top {
                self.b.span = span;
                parts.push(self.b.constant(Logic::x(top - f.lsb - fw)));
            }
            parts.push(self.value_as(src, &f.ty, sink));
            top = f.lsb;
        }
        if top > 0 {
            self.b.span = span;
            parts.push(self.b.constant(Logic::x(top)));
        }
        self.b.span = span;
        let whole = if parts.len() == 1 {
            parts[0]
        } else {
            self.b.concat(parts)
        };
        Some(self.coerce(whole, p.width(), p.signed))
    }

    /// The memory `e` names whole, with its type, or `None`.
    pub(super) fn whole_array(&mut self, e: &'ast Expr) -> Option<(MemoryId, VType)> {
        if matches!(e.kind, ExprKind::Index { .. } | ExprKind::Range { .. }) {
            return None;
        }
        match self.env.probe(|env| env.resolve_path(e)) {
            Some(Symbol::Memory { mem, ty }) => Some((mem, ty)),
            _ => None,
        }
    }

    /// `a = '{..}` or `a = b` for a whole unpacked array `a`: one
    /// assignment per element. `false` when `lhs` is not a whole array.
    pub(super) fn array_assign(
        &mut self,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        kind: AssignKind,
        span: Span,
        out: &mut BlockBuilder,
    ) -> bool {
        let Some((mem, ty)) = self.whole_array(lhs) else {
            return false;
        };
        let size = self.b.module().memories[mem].size;
        if size > ELEMENTS {
            self.env.unsupported(
                span,
                &format!("an assignment to a whole array of more than {ELEMENTS} elements"),
            );
            return true;
        }
        let Some(values) = self.array_elements(rhs, &ty, size, span, out) else {
            return true;
        };
        let aw = (super::super::types::bits_needed(size.saturating_sub(1))).max(1);
        for (k, v) in values.into_iter().enumerate() {
            self.b.span = span;
            let addr = self
                .b
                .constant(Logic::from_u64(u64::try_from(k).unwrap_or(0), aw));
            out.span = span;
            out.assign(ir::Lvalue::MemElem { mem, addr }, v, kind);
        }
        true
    }

    /// The element values, in memory order, of `rhs` as an array of type
    /// `ty` with `size` elements: a pattern, or another array of the same
    /// shape read element by element.
    fn array_elements(
        &mut self,
        rhs: &'ast Expr,
        ty: &VType,
        size: u64,
        span: Span,
        out: &mut BlockBuilder,
    ) -> Option<Vec<ExprId>> {
        let elem = innermost(ty);
        if let Some((src, src_ty)) = self.whole_array(rhs) {
            if self.b.module().memories[src].size != size || shape(&src_ty) != shape(ty) {
                self.env.error(
                    codes::TYPE,
                    span,
                    "an array is assigned from one of another shape",
                );
                return None;
            }
            let aw = (super::super::types::bits_needed(size.saturating_sub(1))).max(1);
            let mut values = Vec::new();
            for k in 0..size {
                self.b.span = span;
                let addr = self.b.constant(Logic::from_u64(k, aw));
                let read = self.b.mem_read(src, addr);
                let w = elem.packed().map_or(1, Packed::width);
                let signed = elem.packed().is_some_and(|p| p.signed);
                values.push(self.coerce(read, w, signed));
            }
            return Some(values);
        }
        let ExprKind::Pattern(items) = &rhs.kind else {
            self.env.unsupported(
                rhs.span,
                "an assignment to a whole array from anything but a pattern or an array",
            );
            return None;
        };
        let mut values = Vec::new();
        self.pattern_elements(items, ty, span, &mut Sink::Proc(out), &mut values)?;
        Some(values)
    }

    /// The element values a pattern gives an unpacked array of type `ty`,
    /// appended in memory order (each dimension lowest index first).
    fn pattern_elements(
        &mut self,
        items: &'ast [ast::PatternItem],
        ty: &VType,
        span: Span,
        sink: &mut Sink<'_>,
        out: &mut Vec<ExprId>,
    ) -> Option<()> {
        let VType::Unpacked { elem, range } = ty else {
            return None;
        };
        let n = usize::try_from(range.len()).unwrap_or(usize::MAX);
        let mut slots: Vec<Option<&'ast Expr>> = vec![None; n];
        let mut default = None;
        let mut position = 0usize;
        for (key, value) in self.pattern_items(items) {
            match key {
                None => {
                    if position >= n {
                        self.too_many(span, n);
                        return None;
                    }
                    slots[position] = Some(value);
                    position += 1;
                }
                Some(k) if matches!(k.kind, ExprKind::Default) => default = Some(value),
                Some(k) => {
                    let index = self.env.probe(|env| Evaluator::new(env).eval_i64(k).ok());
                    let at = index.and_then(|i| left_position(*range, i));
                    let Some(at) = at else {
                        self.env.error(
                            codes::TYPE,
                            k.span,
                            format!("this key is not an index of {range}"),
                        );
                        return None;
                    };
                    slots[at] = Some(value);
                }
            }
        }
        if position != 0 && position != n {
            self.env.error(
                codes::TYPE,
                span,
                format!("the pattern has {position} elements and the array {n}"),
            );
            return None;
        }
        // The pattern lists elements from the left bound; memory order is
        // lowest index first.
        if range.is_descending() && range.left != range.right {
            slots.reverse();
        }
        for slot in slots {
            let Some(src) = slot.or(default) else {
                self.env
                    .error(codes::TYPE, span, "an element of the pattern is not given");
                return None;
            };
            match (&src.kind, &**elem) {
                (ExprKind::Pattern(inner), VType::Unpacked { .. }) => {
                    self.pattern_elements(inner, elem, src.span, sink, out)?;
                }
                _ => {
                    if matches!(**elem, VType::Unpacked { .. }) {
                        self.env.unsupported(
                            src.span,
                            "an element of a multi-dimensional pattern that is not a pattern",
                        );
                        return None;
                    }
                    out.push(self.value_as(src, elem, sink));
                }
            }
        }
        Some(())
    }

    /// `a == b` or `a != b` of two whole unpacked arrays: every element
    /// equal, or not. `None` when neither side is a whole array.
    pub(super) fn array_compare(
        &mut self,
        e: &'ast Expr,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        equal: bool,
        sink: &mut Sink<'_>,
    ) -> Option<ExprId> {
        let (a, aty) = self.whole_array(lhs)?;
        let size = self.b.module().memories[a].size;
        if size > ELEMENTS {
            self.env.unsupported(
                e.span,
                &format!("a comparison of arrays of more than {ELEMENTS} elements"),
            );
            return Some(self.b.const_bit(false));
        }
        let elem = innermost(&aty);
        let w = elem.packed().map_or(1, Packed::width);
        let aw = (super::super::types::bits_needed(size.saturating_sub(1))).max(1);
        let theirs: Vec<ExprId> = match self.whole_array(rhs) {
            Some((b, bty)) if shape(&bty) == shape(&aty) => (0..size)
                .map(|k| {
                    let addr = self.b.constant(Logic::from_u64(k, aw));
                    self.b.mem_read(b, addr)
                })
                .collect(),
            Some(_) => {
                self.env.error(
                    codes::TYPE,
                    e.span,
                    "arrays of different shapes are compared",
                );
                return Some(self.b.const_bit(false));
            }
            None => {
                let ExprKind::Pattern(items) = &rhs.kind else {
                    self.env.error(
                        codes::TYPE,
                        e.span,
                        "an array is compared with something that is not an array",
                    );
                    return Some(self.b.const_bit(false));
                };
                let mut values = Vec::new();
                self.pattern_elements(items, &aty, e.span, sink, &mut values)?;
                values
            }
        };
        self.b.span = e.span;
        let mut all = self.b.const_bit(true);
        for (k, b) in theirs.into_iter().enumerate() {
            let addr = self
                .b
                .constant(Logic::from_u64(u64::try_from(k).unwrap_or(0), aw));
            let x = self.b.mem_read(a, addr);
            let x = self.coerce(x, w, false);
            let y = self.coerce(b, w, false);
            let same = self.b.eq(x, y);
            all = self.b.binary(ir::BinaryOp::LogicAnd, all, same);
        }
        Some(if equal { all } else { self.b.lnot(all) })
    }
}

/// Where index `i` of `range` is, counted from its left bound.
fn left_position(range: Range, i: i64) -> Option<usize> {
    if !range.contains(i) {
        return None;
    }
    usize::try_from(i.abs_diff(range.left)).ok()
}

/// The element type of an array of arrays.
fn innermost(ty: &VType) -> VType {
    let mut t = ty;
    while let VType::Unpacked { elem, .. } = t {
        t = elem;
    }
    t.clone()
}

/// The lengths of an array's dimensions and its element's width.
fn shape(ty: &VType) -> (Vec<u64>, u32) {
    let mut lens = Vec::new();
    let mut t = ty;
    while let VType::Unpacked { elem, range } = t {
        lens.push(range.len());
        t = elem;
    }
    (lens, t.packed().map_or(1, Packed::width))
}
