//! Lowering the bundled arithmetic packages to IR operators.
//!
//! `ieee.numeric_std`, `ieee.numeric_bit` and the three Synopsys legacy
//! packages declare their subprograms and stop there: every one is
//! `attribute foreign ... is "reticle: builtin"`, so there is no VHDL body
//! to inline. This module is the other half of that bargain for
//! elaboration — [`crate::vhdl::sema::builtin`] handles the static case —
//! and turns a call into the IR node that means the same thing.
//!
//! # Why not a VHDL body
//!
//! `unsigned` and `signed` are arrays of logic values with arithmetic on
//! them, and the IR already has that arithmetic: `Add`, `Sub`, `Mul`,
//! `Div`, `Mod`, the shifts, the comparisons and `Resize`, each with a
//! signed flag. Inlining a VHDL body would build a ripple adder out of
//! single-bit operations and leave synthesis to recognise it again;
//! emitting one `Add` says what the design meant.
//!
//! # The two rules that matter
//!
//! **Width.** The IR requires both operands of a binary node to have the
//! same width, so every operand is resized explicitly first. The width
//! resized to is the one the package's own definition gives the result:
//! the longer operand for `+`, `-`, `/`, `rem` and `mod`, the sum of the
//! two lengths for `*`, and the vector's length when the other operand is
//! an integer. The result is then coerced to whatever the context asked
//! for, which is where a narrower target truncates.
//!
//! **Signedness.** For `numeric_std`, `numeric_bit` and
//! `std_logic_arith` it comes from the operand type, by name (see
//! [`super::types::is_signed_array`]). For `std_logic_unsigned` and
//! `std_logic_signed`, whose operands are plain `std_logic_vector`, it
//! comes from the package the operator was declared in. That is the whole
//! difference between those two packages, so reading it from the
//! declaration is not a shortcut but the definition.
//!
//! # What is not lowered
//!
//! A rotate by an amount that is not known at elaboration is reported
//! rather than built out of two dynamic shifts. `find_leftmost` and
//! `find_rightmost` fold in the analyser when their argument is static
//! and are reported otherwise. The `to_string` family lowers to the same
//! `$to_string`-style IR calls `ieee.std_logic_1164` uses, since the
//! simulator implements those.
//!
//! Two smaller departures, both where the IR has no node for what the
//! package says:
//!
//! - `?=` and `?/=` become the IR's case equality, which compares `x`
//!   and `z` as themselves, where the package returns `'X'` whenever
//!   either operand holds an unknown. The static folding in
//!   [`crate::vhdl::sema::builtin`] does return `'X'`, and this matches
//!   how `ieee.std_logic_1164`'s own matching operators already lower.
//! - `sll`, `srl`, `sla` and `sra` reverse direction for a negative
//!   count, which is honoured for a count known at elaboration but not
//!   for one computed at run time; a negative count there shifts the
//!   value away instead.

use crate::ir::{BinaryOp, ExprId, Type};
use crate::logic::Logic;
use crate::source::Span;
use crate::vhdl::ast;
use crate::vhdl::sema::{DeclId, DeclKind, TypeClass, TypeId};

use super::codes;
use super::lower::{Lowerer, Sink};
use super::types::{self, ArrayLayout, BitKind, Layout, LayoutKind};

/// One of the bundled packages whose arithmetic is implemented natively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithPkg {
    /// `ieee.numeric_std`, over `std_ulogic`.
    NumericStd,
    /// `ieee.numeric_bit`, over `bit`.
    NumericBit,
    /// The Synopsys `ieee.std_logic_arith`.
    Arith,
    /// The Synopsys `ieee.std_logic_unsigned`.
    Unsigned,
    /// The Synopsys `ieee.std_logic_signed`.
    Signed,
}

impl ArithPkg {
    /// Every package, with the unit name it is looked up by.
    const ALL: [(ArithPkg, &'static str); 5] = [
        (ArithPkg::NumericStd, "numeric_std"),
        (ArithPkg::NumericBit, "numeric_bit"),
        (ArithPkg::Arith, "std_logic_arith"),
        (ArithPkg::Unsigned, "std_logic_unsigned"),
        (ArithPkg::Signed, "std_logic_signed"),
    ];

    /// The signedness the package imposes whatever its operands' types,
    /// which only the two `std_logic_vector` packages do.
    fn forced_signed(self) -> Option<bool> {
        match self {
            ArithPkg::Unsigned => Some(false),
            ArithPkg::Signed => Some(true),
            _ => None,
        }
    }
}

/// What every lowering here needs to know about the call it is part of,
/// kept together so each helper takes one context rather than a queue of
/// scalars.
struct Call<'w> {
    /// The call's span: where its diagnostics point, and the key its
    /// analysed subtype is looked up by.
    span: Span,
    /// True when the operation is two's complement.
    signed: bool,
    /// The declared return type, which for these packages is an
    /// unconstrained `unsigned` or `signed`. It has no width of its own,
    /// but it does say how the elements are encoded.
    ret: Option<TypeId>,
    /// The layout the context expects, when it has one.
    want: Option<&'w Layout>,
}

/// The lowered operands of a binary operation, with the two widths the
/// packages define a result by.
struct Operands {
    /// The operands, in order.
    vals: Vec<(ExprId, Layout)>,
    /// The longer vector's length: the width of `+`, `-`, `/`, `rem`,
    /// `mod` and every comparison.
    common: u32,
    /// The sum of the two lengths: the width of `*`.
    product: u32,
}

/// How a shift fills the bits it vacates.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shift {
    /// Towards the most significant end, zero filling.
    Left,
    /// Towards the least significant end, zero filling.
    RightLogical,
    /// Towards the least significant end, sign filling.
    RightArithmetic,
}

impl Shift {
    /// The direction reversed, which a negative count asks for.
    fn reversed(self, signed: bool) -> Shift {
        match self {
            Shift::Left if signed => Shift::RightArithmetic,
            Shift::Left => Shift::RightLogical,
            _ => Shift::Left,
        }
    }

    /// The IR operator.
    fn op(self) -> BinaryOp {
        match self {
            Shift::Left => BinaryOp::Shl,
            Shift::RightLogical => BinaryOp::Shr,
            Shift::RightArithmetic => BinaryOp::Sshr,
        }
    }
}

impl<'a> Lowerer<'a, '_> {
    /// Which bundled arithmetic package declared `d`, if any.
    pub(crate) fn arith_package(&self, d: DeclId) -> Option<ArithPkg> {
        let d = super::lower::resolve_alias(self.a(), d);
        let region = self.a().decl(d).region;
        ArithPkg::ALL.into_iter().find_map(|(pkg, name)| {
            let u = self.a().unit("ieee", name)?;
            (self.a().units[u.index()].region == Some(region)).then_some(pkg)
        })
    }

    /// The bundled package `d` was declared in, as `lib.pkg`.
    ///
    /// Used to explain why a `foreign` subprogram of one of them cannot
    /// be inlined: there is nothing to inline, and the ones that lower to
    /// hardware already have.
    pub(crate) fn bundled_package_of(&self, d: DeclId) -> Option<String> {
        let d = super::lower::resolve_alias(self.a(), d);
        let region = self.a().decl(d).region;
        let a = self.a();
        let unit = a
            .units
            .iter()
            .find(|u| u.region == Some(region) && u.kind.is_primary())?;
        let lib = a.name(unit.library);
        (lib.eq_ignore_ascii_case("ieee") || lib.eq_ignore_ascii_case("std"))
            .then(|| format!("{lib}.{}", a.name(unit.name)))
    }

    /// Lowers a call to one of those packages, or reports why it cannot
    /// be lowered.
    pub(crate) fn numeric_call(
        &mut self,
        pkg: ArithPkg,
        d: DeclId,
        args: &[&'a ast::Expr],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let d = super::lower::resolve_alias(self.a(), d);
        let name = self.a().decl(d).spelling.to_ascii_lowercase();
        let sym = name.trim_matches('"').to_owned();
        let DeclKind::Subprogram { sig, .. } = &self.a().decl(d).kind else {
            return None;
        };
        let ptys: Vec<TypeId> = sig.params.iter().map(|p| p.ty).collect();
        let ret = sig.ret;

        // The element-wise logical operators, the reductions and the
        // scalar/vector forms behave exactly as they do on
        // `std_ulogic_vector`, so the general operator path handles them.
        if matches!(
            sym.as_str(),
            "and" | "or" | "nand" | "nor" | "xor" | "xnor" | "not"
        ) {
            return match args {
                [only] => self.operator_sym(&sym, None, only, span, want, sink),
                [lhs, rhs] => self.operator_sym(&sym, Some(lhs), rhs, span, want, sink),
                _ => None,
            };
        }

        let signed = pkg.forced_signed().unwrap_or_else(|| {
            ptys.iter()
                .chain(ret.iter())
                .any(|t| types::is_signed_array(self.a(), *t))
        });
        let call = Call {
            span,
            signed,
            ret,
            want,
        };
        let right = if signed {
            Shift::RightArithmetic
        } else {
            Shift::RightLogical
        };

        match sym.as_str() {
            "+" | "-" | "abs" if args.len() == 1 => self.numeric_unary(&sym, args[0], &call, sink),
            "+" | "-" | "*" | "/" | "rem" | "mod" => {
                self.numeric_arith(&sym, args, &ptys, &call, sink)
            }
            "=" | "/=" | "<" | "<=" | ">" | ">=" | "?=" | "?/=" | "?<" | "?<=" | "?>" | "?>=" => {
                self.numeric_compare(&sym, args, &ptys, &call, sink)
            }
            // `sla` is a left shift, and the Synopsys `shl` and `shr`
            // follow the operand's own signedness as the named functions
            // do.
            "shift_left" | "sll" | "sla" | "shl" => {
                self.numeric_shift(Shift::Left, args, &call, sink)
            }
            "shift_right" | "sra" | "shr" => self.numeric_shift(right, args, &call, sink),
            // `srl` is defined as a logical shift even for `signed`.
            "srl" => self.numeric_shift(Shift::RightLogical, args, &call, sink),
            "rotate_left" | "rol" => self.numeric_rotate(true, args, &call, sink),
            "rotate_right" | "ror" => self.numeric_rotate(false, args, &call, sink),
            "resize" => self.numeric_resize(args, &ptys, &call, sink),
            "to_integer" | "conv_integer" => {
                let (id, from) = self.build(args[0], None, sink)?;
                let src = from.signed || ptys.first().is_some_and(|t| self.is_int_type(*t));
                let to = self.layout_at(span).unwrap_or_else(|| self.int_layout());
                self.b.span = span;
                let id = self.widen(id, to.width, src);
                Some((id, to))
            }
            "to_unsigned"
            | "to_signed"
            | "conv_unsigned"
            | "conv_signed"
            | "conv_std_logic_vector"
            | "ext"
            | "sxt" => self.numeric_convert(&sym, args, &ptys, &call, sink),
            // Under Reticle's encoding these are the identity: a lowered
            // bit is already one of `0`, `1`, `x` and `z`.
            "to_01" | "to_x01" | "to_x01z" | "to_ux01" => {
                let (id, from) = self.build(args[0], None, sink)?;
                let to = self
                    .layout_at(span)
                    .or_else(|| want.cloned())
                    .unwrap_or_else(|| from.clone());
                Some((self.coerce(id, &from, &to, span), to))
            }
            "is_x" => {
                let (id, _) = self.build(args[0], None, sink)?;
                let to = self
                    .layout_at(span)
                    .unwrap_or_else(|| self.boolean_layout());
                self.b.span = span;
                Some((self.b.call("$isunknown", vec![id], Type::bit()), to))
            }
            "std_match" => self.numeric_match(args, &call, sink),
            "minimum" | "maximum" => self.numeric_extremum(&sym, args, &ptys, &call, sink),
            "to_string" | "to_bstring" | "to_ostring" | "to_hstring" => {
                let (id, _) = self.build(args[0], None, sink)?;
                let to = self.layout_at(span)?;
                self.b.span = span;
                Some((self.b.call(name, vec![id], Type::String), to))
            }
            _ => {
                self.unsupported(span, &format!("`{name}` of a bundled arithmetic package"));
                None
            }
        }
    }

    /// The same call written with association elements rather than bare
    /// operands, which is how `resize(a, 8)` arrives. A named or `open`
    /// association is left to the ordinary path.
    pub(crate) fn numeric_assoc_call(
        &mut self,
        pkg: ArithPkg,
        d: DeclId,
        args: &'a [ast::AssociationElement],
        span: Span,
        want: Option<&Layout>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let exprs: Vec<&'a ast::Expr> = args
            .iter()
            .filter_map(|a| match &a.actual {
                ast::Actual::Expr(e) => Some(e),
                _ => None,
            })
            .collect();
        if exprs.len() != args.len() || exprs.is_empty() {
            return None;
        }
        self.numeric_call(pkg, d, &exprs, span, want, sink)
    }

    /// True for an integer type or subtype.
    fn is_int_type(&self, t: TypeId) -> bool {
        matches!(
            self.a().class(t),
            TypeClass::Integer | TypeClass::UniversalInteger
        )
    }

    /// True when the formal at `i` is an array, so the actual there is a
    /// vector rather than an integer.
    fn formal_is_vector(&self, ptys: &[TypeId], i: usize) -> bool {
        ptys.get(i)
            .is_some_and(|t| self.a().class(*t) == TypeClass::Array)
    }

    /// Lowers the operands of a binary operation and works out the two
    /// widths a result may take.
    ///
    /// A vector operand contributes its own length; an integer operand
    /// contributes nothing, since the package converts it to the vector's
    /// length.
    fn operand_widths(
        &mut self,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        sink: &mut Sink<'_>,
    ) -> Option<Operands> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.build(a, None, sink)?);
        }
        let width_of = |i: usize| self.formal_is_vector(ptys, i).then(|| vals[i].1.width);
        let (lw, rw) = (width_of(0), width_of(1));
        let (common, product) = match (lw, rw) {
            (Some(a), Some(b)) => (a.max(b), a.saturating_add(b)),
            (Some(a), None) | (None, Some(a)) => (a, a.saturating_mul(2)),
            (None, None) => return None,
        };
        Some(Operands {
            vals,
            common,
            product,
        })
    }

    fn numeric_arith(
        &mut self,
        sym: &str,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let ops = self.operand_widths(args, ptys, sink)?;
        let width = if sym == "*" { ops.product } else { ops.common };
        let result = self.result_layout(call, &ops.vals, width);
        self.b.span = call.span;
        let l = self.widen(ops.vals[0].0, width, call.signed);
        let r = self.widen(ops.vals[1].0, width, call.signed);
        let id = match sym {
            "+" => self.b.binary(BinaryOp::Add, l, r),
            "-" => self.b.binary(BinaryOp::Sub, l, r),
            "*" => self.b.binary(BinaryOp::Mul, l, r),
            "/" => self.b.binary(BinaryOp::Div, l, r),
            // The IR's `Mod` takes the sign of the dividend, which is
            // VHDL's `rem`; `mod` takes the divisor's, so it is corrected.
            "rem" => self.b.binary(BinaryOp::Mod, l, r),
            "mod" => self.vhdl_mod_op(l, r, width, call.signed),
            _ => return None,
        };
        Some(self.finish(id, width, call, result))
    }

    fn numeric_unary(
        &mut self,
        sym: &str,
        arg: &'a ast::Expr,
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (id, from) = self.build(arg, None, sink)?;
        let width = from.width;
        let result = self.vector_layout(call, &from, width);
        self.b.span = call.span;
        let id = match sym {
            "+" => id,
            "-" => self.b.neg(id),
            "abs" => {
                let zero = self
                    .b
                    .constant(Logic::from_i64(0, width.max(1)).with_signed(true));
                let neg = self.b.binary(BinaryOp::Lt, id, zero);
                let inv = self.b.neg(id);
                self.b.mux(neg, inv, id)
            }
            _ => return None,
        };
        Some(self.finish(id, width, call, result))
    }

    fn numeric_compare(
        &mut self,
        sym: &str,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let ops = self.operand_widths(args, ptys, sink)?;
        let width = ops.common;
        self.b.span = call.span;
        let l = self.widen(ops.vals[0].0, width, call.signed);
        let r = self.widen(ops.vals[1].0, width, call.signed);
        let id = match sym {
            "=" => self.b.binary(BinaryOp::Eq, l, r),
            "/=" => self.b.binary(BinaryOp::Ne, l, r),
            "<" | "?<" => self.b.binary(BinaryOp::Lt, l, r),
            "<=" | "?<=" => self.b.binary(BinaryOp::Le, l, r),
            ">" | "?>" => self.b.binary(BinaryOp::Gt, l, r),
            ">=" | "?>=" => self.b.binary(BinaryOp::Ge, l, r),
            // The matching operators answer with a logic value, so an
            // unknown operand gives an unknown answer rather than `false`.
            "?=" => self.b.binary(BinaryOp::CaseEq, l, r),
            "?/=" => self.b.binary(BinaryOp::CaseNe, l, r),
            _ => return None,
        };
        let to = self
            .layout_at(call.span)
            .unwrap_or_else(|| self.boolean_layout());
        Some(self.finish_bit(id, call, to))
    }

    fn numeric_shift(
        &mut self,
        how: Shift,
        args: &[&'a ast::Expr],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let (arg, from) = self.build(args[0], None, sink)?;
        let width = from.width;
        let result = self.vector_layout(call, &from, width);
        // A static count decides the direction, which the `sll` family
        // reverses for a negative one.
        let (how, count) = match self.eval_int(args[1]) {
            Some(n) if n < 0 => (how.reversed(call.signed), Some(-n)),
            Some(n) => (how, Some(n)),
            None => (how, None),
        };
        self.b.span = call.span;
        let amount = match count {
            Some(n) => {
                // Shifting by more than the width empties the value, so
                // the count is clamped rather than wrapped.
                let n = u64::try_from(n).unwrap_or(0).min(u64::from(width));
                self.b.const_u64(32, n)
            }
            None => {
                let (id, l) = self.build(args[1], None, sink)?;
                self.b.span = call.span;
                self.widen(id, l.width.max(32), false)
            }
        };
        let base = self.widen(arg, width, call.signed);
        let id = self.b.binary(how.op(), base, amount);
        Some(self.finish(id, width, call, result))
    }

    fn numeric_rotate(
        &mut self,
        left: bool,
        args: &[&'a ast::Expr],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let (arg, from) = self.build(args[0], None, sink)?;
        let width = from.width;
        let result = self.vector_layout(call, &from, width);
        if width == 0 {
            return Some((arg, result));
        }
        let Some(n) = self.eval_int(args[1]) else {
            self.error(
                codes::NOT_STATIC,
                args[1].span(),
                "a rotate by an amount that is not static cannot be lowered yet",
            );
            return None;
        };
        // A rotate left by `n` is a rotate right by `width - n`, so one
        // pair of shifts covers both directions.
        let w = i128::from(width);
        let n = if left {
            n.rem_euclid(w)
        } else {
            (-n).rem_euclid(w)
        };
        let n = u64::try_from(n).unwrap_or(0);
        self.b.span = call.span;
        let base = self.widen(arg, width, false);
        let id = if n == 0 {
            base
        } else {
            let up = self.b.const_u64(32, n);
            let down = self.b.const_u64(32, u64::from(width) - n);
            let hi = self.b.binary(BinaryOp::Shl, base, up);
            let lo = self.b.binary(BinaryOp::Shr, base, down);
            self.b.binary(BinaryOp::Or, hi, lo)
        };
        Some(self.finish(id, width, call, result))
    }

    /// `resize(arg, n)`. Growing extends with the sign bit or with
    /// zeroes; narrowing a `signed` keeps the sign bit and drops the bits
    /// under it, which is what the package promises and is *not* a plain
    /// truncation.
    fn numeric_resize(
        &mut self,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let (arg, from) = self.build(args[0], None, sink)?;
        let Some(new) = self.size_argument(args, ptys, 1, sink) else {
            self.error(
                codes::NOT_STATIC,
                args[1].span(),
                "the new size of `resize` must be known at elaboration",
            );
            return None;
        };
        let result = self.vector_layout(call, &from, new);
        self.b.span = call.span;
        let id = if call.signed && new >= 1 && new < from.width {
            let sign = self.b.slice(arg, from.width - 1, from.width - 1);
            if new == 1 {
                sign
            } else {
                let low = self.b.slice(arg, new - 2, 0);
                self.b.concat(vec![sign, low])
            }
        } else {
            self.widen(arg, new, call.signed)
        };
        Some(self.finish(id, new, call, result))
    }

    /// The conversions that produce a vector of a given size.
    fn numeric_convert(
        &mut self,
        sym: &str,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        let (arg, from) = self.build(args[0], None, sink)?;
        // How the *source* extends: an integer and a `signed` vector
        // carry their sign, everything else is zero filled. `ext` and
        // `sxt` say so themselves.
        let src_signed = match sym {
            "ext" => false,
            "sxt" => true,
            _ => ptys
                .first()
                .is_some_and(|t| self.is_int_type(*t) || types::is_signed_array(self.a(), *t)),
        };
        let size = self
            .size_argument(args, ptys, 1, sink)
            .or_else(|| self.layout_at(call.span).map(|l| l.width))
            .or_else(|| call.want.map(|l| l.width));
        let Some(size) = size else {
            self.error(
                codes::NOT_STATIC,
                call.span,
                format!("the size of `{sym}` must be known at elaboration"),
            );
            return None;
        };
        let result = self.vector_layout(call, &from, size);
        self.b.span = call.span;
        let id = self.widen(arg, size, src_signed);
        Some(self.finish(id, size, call, result))
    }

    /// `std_match(l, r)`: the pattern's `'-'` positions, which are
    /// unknown bits under Reticle's encoding, match anything.
    fn numeric_match(
        &mut self,
        args: &[&'a ast::Expr],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let (l, ll) = self.build(args[0], None, sink)?;
        let (r, rl) = self.build(args[1], None, sink)?;
        let width = ll.width.max(rl.width);
        self.b.span = call.span;
        let l = self.widen(l, width, false);
        let r = self.widen(r, width, false);
        let id = self.b.binary(BinaryOp::WildEq, l, r);
        let to = self
            .layout_at(call.span)
            .unwrap_or_else(|| self.boolean_layout());
        Some(self.finish_bit(id, call, to))
    }

    fn numeric_extremum(
        &mut self,
        sym: &str,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        call: &Call<'_>,
        sink: &mut Sink<'_>,
    ) -> Option<(ExprId, Layout)> {
        if args.len() != 2 {
            return None;
        }
        let ops = self.operand_widths(args, ptys, sink)?;
        let width = ops.common;
        let result = self.result_layout(call, &ops.vals, width);
        self.b.span = call.span;
        let l = self.widen(ops.vals[0].0, width, call.signed);
        let r = self.widen(ops.vals[1].0, width, call.signed);
        let less = self.b.binary(BinaryOp::Lt, l, r);
        let id = if sym == "minimum" {
            self.b.mux(less, l, r)
        } else {
            self.b.mux(less, r, l)
        };
        Some(self.finish(id, width, call, result))
    }

    /// The `size` argument at index `i`: an integer, or the length of the
    /// `size_res` vector the VHDL-2008 overloads take instead.
    fn size_argument(
        &mut self,
        args: &[&'a ast::Expr],
        ptys: &[TypeId],
        i: usize,
        sink: &mut Sink<'_>,
    ) -> Option<u32> {
        let e = *args.get(i)?;
        if self.formal_is_vector(ptys, i) {
            let (_, l) = self.build(e, None, sink)?;
            return Some(l.width);
        }
        u32::try_from(self.eval_int(e)?.max(0)).ok()
    }

    /// Coerces a vector-valued result of `width` bits to the layout the
    /// context wants.
    fn finish(
        &mut self,
        id: ExprId,
        width: u32,
        call: &Call<'_>,
        result: Layout,
    ) -> (ExprId, Layout) {
        let from = Layout {
            width,
            signed: call.signed,
            ..result.clone()
        };
        (self.coerce(id, &from, &result, call.span), result)
    }

    /// The same for a one-bit result: a comparison or a test.
    fn finish_bit(&mut self, id: ExprId, call: &Call<'_>, to: Layout) -> (ExprId, Layout) {
        let from = Layout {
            width: 1,
            signed: false,
            ..to.clone()
        };
        (self.coerce(id, &from, &to, call.span), to)
    }

    /// The layout a vector-valued result takes: the analysed subtype, the
    /// context's, or, when the return type is unconstrained and nothing
    /// else says, a fresh `width-1 downto 0` vector.
    ///
    /// That last case is the usual one here, since these packages return
    /// `unsigned` and `signed` without a constraint: the length follows
    /// from the operands, and only the element encoding comes from the
    /// type.
    fn vector_layout(&mut self, call: &Call<'_>, like: &Layout, width: u32) -> Layout {
        if let Some(l) = self.layout_at(call.span) {
            return l;
        }
        if let Some(l) = call.want {
            return l.clone();
        }
        let ty = call.ret.unwrap_or(like.ty);
        let elem = self
            .a()
            .element_type(ty)
            .and_then(|e| self.layout_of_type_quiet(e, call.span))
            .or_else(|| like.array().map(|a| a.elem.clone()))
            .unwrap_or(Layout {
                ty,
                kind: LayoutKind::Bit(BitKind::StdLogic),
                width: 1,
                signed: false,
            });
        Layout {
            ty,
            kind: LayoutKind::Array(Box::new(ArrayLayout {
                elem,
                left: i64::from(width).saturating_sub(1),
                dir: ast::Direction::Downto,
                len: width,
            })),
            width,
            signed: call.signed,
        }
    }

    /// [`Self::vector_layout`] with the operands as the shape to fall
    /// back on: the first that is an array, else the first of all.
    fn result_layout(&mut self, call: &Call<'_>, vals: &[(ExprId, Layout)], width: u32) -> Layout {
        let base = vals
            .iter()
            .find(|(_, l)| l.array().is_some())
            .or_else(|| vals.first())
            .map(|(_, l)| l.clone());
        match base {
            Some(l) => self.vector_layout(call, &l, width),
            None => self.int_layout(),
        }
    }

    /// VHDL's `mod`, whose result takes the sign of the divisor, built on
    /// the IR's remainder, which takes the sign of the dividend.
    fn vhdl_mod_op(&mut self, l: ExprId, r: ExprId, width: u32, signed: bool) -> ExprId {
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
}
