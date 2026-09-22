//! Expression width and sign inference (IEEE 1364-2005 §5.4, §5.5).
//!
//! `info` computes an expression's *self-determined* width and
//! signedness: what it would be if the expression stood alone. Both the
//! constant evaluator and the lowering use it, then widen operands to the
//! context width where the standard makes them context-determined, so the
//! two agree bit for bit.
//!
//! The rules, from Table 5-22 of IEEE 1364-2005 §5.4.1 (the numbers in
//! the table are the width of the *result*; `L(x)` is the self-determined
//! width of `x`, `i` the context width):
//!
//! | Expression                                    | Width               | Operands are |
//! |-----------------------------------------------|---------------------|--------------|
//! | unsized constant                              | 32 (or as written)  | —            |
//! | sized constant                                | as written          | —            |
//! | `i` op `j` for `+ - * / % & | ^ ^~ ~^`        | `max(L(i), L(j))`   | context      |
//! | `+i`, `-i`, `~i`                              | `L(i)`              | context      |
//! | `i op j` for `=== !== == != > >= < <=`        | 1 bit               | self, sized to each other |
//! | `&i ~&i |i ~|i ^i ~^i !i`, `i && j`, `i || j` | 1 bit               | self         |
//! | `i >> j`, `i << j`, `i ** j`                  | `L(i)`              | `i` context, `j` self |
//! | `i ? j : k`                                   | `max(L(j), L(k))`   | `j`,`k` context; `i` self |
//! | `{i, ..}`, `{n{i, ..}}`                       | sum / n × sum       | self         |
//!
//! Signedness (§5.5.1): an expression is signed when every
//! context-determined operand is signed; decimal literals without a base
//! and `'s`-based literals are signed, every other literal is unsigned;
//! bit-selects, part-selects and concatenations are always unsigned;
//! comparison results are unsigned. `$signed` and `$unsigned` override the
//! operand's signedness without changing its bits.
//!
//! [`Info`] also carries `min`, the number of bits the value actually
//! needs. Only a value whose `min` exceeds the target width makes
//! `check_assign` warn about truncation, so writing `0` into a narrow
//! net is silent while writing a wide expression is not.

use crate::diag::Diagnostic;
use crate::source::Span;
use crate::verilog::ast::{self, BinaryOp, CastTarget, Expr, ExprKind, Literal, UnaryOp};

use super::codes;
use super::constant::{self, Evaluator, Value};
use super::env::ScopeEnv;
use super::scope::Symbol;
use super::types::{self, Packed, VType, min_bits};

/// What an expression evaluates to, before any context is applied.
#[derive(Clone, Debug, PartialEq)]
pub enum Info {
    /// An integral value.
    Bits {
        /// The self-determined width.
        width: u32,
        /// True when the value is treated as two's complement.
        signed: bool,
        /// The number of bits the value needs; equal to `width` unless the
        /// expression is a constant that fits in fewer.
        min: u32,
    },
    /// A real number.
    Real,
    /// A string value of the given bit width (8 bits per character).
    Str(u32),
    /// Something with no numeric interpretation (an array, an event, a
    /// void call).
    Other,
}

impl Info {
    /// An integral value of `width` bits.
    pub fn bits(width: u32, signed: bool) -> Info {
        Info::Bits {
            width,
            signed,
            min: width,
        }
    }

    /// The width a value of this kind occupies: 64 for a real, the string
    /// length in bits for a string, one bit for anything else.
    pub fn width(&self) -> u32 {
        match self {
            Info::Bits { width, .. } => *width,
            Info::Real => 64,
            Info::Str(w) => *w,
            Info::Other => 1,
        }
    }

    /// True for signed integral values and reals.
    pub fn is_signed(&self) -> bool {
        match self {
            Info::Bits { signed, .. } => *signed,
            Info::Real => true,
            Info::Str(_) | Info::Other => false,
        }
    }

    /// The bits a constant of this kind really needs.
    pub fn min_width(&self) -> u32 {
        match self {
            Info::Bits { min, .. } => *min,
            other => other.width(),
        }
    }

    /// The kind two context-determined operands share: real wins over
    /// integral (§5.5.1), two strings stay a string, and otherwise the
    /// result is `max` of the widths, signed only when both are.
    pub fn combine(&self, other: &Info) -> Info {
        match (self, other) {
            (Info::Real, _) | (_, Info::Real) => Info::Real,
            (Info::Str(a), Info::Str(b)) => Info::Str(*a.max(b)),
            (Info::Other, _) | (_, Info::Other) => Info::Other,
            (a, b) => Info::Bits {
                width: a.width().max(b.width()),
                signed: a.is_signed() && b.is_signed(),
                min: a.min_width().max(b.min_width()),
            },
        }
    }
}

/// The [`Info`] of a type: its width and signedness.
pub fn of_type(ty: &VType) -> Info {
    match ty {
        VType::Packed(p) => Info::Bits {
            width: p.width(),
            signed: p.signed,
            min: p.width(),
        },
        VType::Real => Info::Real,
        VType::String => Info::Str(8),
        _ => Info::Other,
    }
}

/// The self-determined width and signedness of `e`.
///
/// `locals` supplies the types of variables that are not in a scope yet
/// (the frame of an interpreted constant function). Returns `None` when a
/// sub-expression could not be resolved; a diagnostic has then been
/// reported unless the environment is probing.
pub(crate) fn info(
    e: &Expr,
    env: &mut ScopeEnv<'_, '_>,
    locals: &[(String, VType)],
) -> Option<Info> {
    let mut w = Widths { env, locals };
    w.info(e)
}

/// Reports `V0007` when assigning a `value` of `from` bits to a target of
/// `to` bits would lose information.
pub(crate) fn check_assign(
    env: &mut ScopeEnv<'_, '_>,
    target: &str,
    to: u32,
    from: &Info,
    span: Span,
) {
    let needed = from.min_width();
    if needed > to {
        env.report(
            Diagnostic::warning(format!(
                "value is truncated from {needed} bits to {to} in this assignment"
            ))
            .with_code(codes::TRUNCATION)
            .with_label(span, format!("{needed} bits wide"))
            .with_note(format!("`{target}` is {to} bits wide")),
        );
    }
}

/// The inference walker.
struct Widths<'a, 'cx, 'ast> {
    env: &'a mut ScopeEnv<'cx, 'ast>,
    locals: &'a [(String, VType)],
}

impl Widths<'_, '_, '_> {
    fn local(&self, name: &str) -> Option<&VType> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
    }

    fn const_u32(&mut self, e: &Expr) -> Option<u32> {
        let mut ev = Evaluator::new(self.env);
        ev.eval_u32(e).ok()
    }

    fn info(&mut self, e: &Expr) -> Option<Info> {
        match &e.kind {
            ExprKind::Literal(lit) => Some(self.literal(lit)),
            ExprKind::Ident(id) => {
                if let Some(ty) = self.local(&id.name) {
                    return Some(of_type(ty));
                }
                match self.env.lookup(&id.name) {
                    Some((Symbol::Net { ty, .. } | Symbol::Memory { ty, .. }, _, _)) => {
                        Some(of_type(ty))
                    }
                    Some((Symbol::Const { value, ty }, _, _)) => {
                        Some(const_info(value, ty.as_ref()))
                    }
                    Some((Symbol::Type(ty), _, _)) => Some(of_type(ty)),
                    Some(_) => Some(Info::Other),
                    None => {
                        self.env.undefined(id);
                        None
                    }
                }
            }
            ExprKind::SystemIdent(id) => Some(match id.name.as_str() {
                "time" | "stime" | "realtime" => Info::bits(64, false),
                "random" | "urandom" => Info::bits(32, true),
                _ => Info::bits(32, true),
            }),
            ExprKind::Scoped { .. } | ExprKind::Member { .. } => {
                match self.env.probe(|env| env.resolve_path(e)) {
                    Some(Symbol::Net { ty, .. } | Symbol::Memory { ty, .. } | Symbol::Type(ty)) => {
                        Some(of_type(&ty))
                    }
                    Some(Symbol::Const { value, ty }) => Some(const_info(&value, ty.as_ref())),
                    Some(_) => Some(Info::Other),
                    None => self.member_info(e),
                }
            }
            ExprKind::Index { base, .. } => {
                let b = self.type_of(base)?;
                Some(match b {
                    VType::Unpacked { elem, .. } => of_type(&elem),
                    VType::Packed(p) => Info::bits(p.element().width(), false),
                    VType::String => Info::bits(8, false),
                    other => of_type(&other),
                })
            }
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => {
                let b = self.type_of(base)?;
                let elem = match &b {
                    VType::Packed(p) => p.element().width(),
                    VType::Unpacked { elem, .. } => {
                        return Some(of_type(elem));
                    }
                    _ => 1,
                };
                let count = match kind {
                    ast::RangeKind::Fixed => {
                        let a = i64::from(self.const_u32(left)?);
                        let b = i64::from(self.const_u32(right)?);
                        u32::try_from(a.abs_diff(b) + 1).unwrap_or(u32::MAX)
                    }
                    _ => self.const_u32(right)?,
                };
                Some(Info::bits(count.saturating_mul(elem), false))
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => self.info(operand),
                _ => Some(Info::bits(1, false)),
            },
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::Ternary {
                then_expr,
                else_expr,
                ..
            } => {
                let a = self.info(then_expr)?;
                let b = self.info(else_expr)?;
                Some(a.combine(&b))
            }
            ExprKind::Concat(parts) => {
                let mut total = 0u32;
                let mut min = 0u32;
                for p in parts {
                    let i = self.info(p)?;
                    total = total.saturating_add(i.width());
                    min = min.saturating_add(i.width());
                }
                Some(Info::Bits {
                    width: total,
                    signed: false,
                    min,
                })
            }
            ExprKind::Replicate { count, elems } => {
                let n = self.const_u32(count)?;
                let mut total = 0u32;
                for p in elems {
                    total = total.saturating_add(self.info(p)?.width());
                }
                Some(Info::bits(total.saturating_mul(n), false))
            }
            ExprKind::Call { callee, args } => self.call(callee, args),
            ExprKind::Cast { target, expr } => match target {
                CastTarget::Const => self.info(expr),
                CastTarget::Signing(s) => {
                    let i = self.info(expr)?;
                    Some(Info::Bits {
                        width: i.width(),
                        signed: *s == ast::Signing::Signed,
                        min: i.min_width(),
                    })
                }
                CastTarget::Size(n) => Some(Info::bits(self.const_u32(n)?, false)),
                CastTarget::Type(dt) => {
                    let ty = types::resolve(self.env, dt, &[], true)?;
                    Some(of_type(&ty))
                }
            },
            ExprKind::Inside { .. } => Some(Info::bits(1, false)),
            ExprKind::MinTypMax { typ, .. } => self.info(typ),
            ExprKind::Assign { lhs, .. } | ExprKind::IncDec { target: lhs, .. } => self.info(lhs),
            ExprKind::ValueRange { low, .. } => self.info(low),
            ExprKind::Type(dt) => {
                let ty = types::resolve(self.env, dt, &[], true)?;
                Some(of_type(&ty))
            }
            ExprKind::Pattern(_)
            | ExprKind::Streaming { .. }
            | ExprKind::New(_)
            | ExprKind::Default => Some(Info::Other),
        }
    }

    /// The type of a sub-expression used as a base (a name, a member, an
    /// element of something).
    fn type_of(&mut self, e: &Expr) -> Option<VType> {
        if let ExprKind::Ident(id) = &e.kind
            && let Some(ty) = self.local(&id.name)
        {
            return Some(ty.clone());
        }
        if let Some(ty) = self.env.type_of_path(e) {
            return Some(ty);
        }
        match self.info(e)? {
            Info::Bits { width, signed, .. } => Some(VType::Packed(if signed {
                Packed::sbits(width)
            } else {
                Packed::bits(width)
            })),
            Info::Real => Some(VType::Real),
            Info::Str(_) => Some(VType::String),
            Info::Other => None,
        }
    }

    /// A `base.name` that is not a scope member: a packed struct field.
    fn member_info(&mut self, e: &Expr) -> Option<Info> {
        let ExprKind::Member { base, name } = &e.kind else {
            return None;
        };
        let base_ty = self.type_of(base)?;
        let p = base_ty.packed()?;
        let f = p.field(&name.name)?;
        Some(of_type(&f.ty))
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Option<Info> {
        use BinaryOp as B;
        match op {
            B::LogicAnd | B::LogicOr | B::Implies | B::Equiv => Some(Info::bits(1, false)),
            B::Lt
            | B::Le
            | B::Gt
            | B::Ge
            | B::Eq
            | B::Ne
            | B::CaseEq
            | B::CaseNe
            | B::WildEq
            | B::WildNe => {
                // Operands still have to be inferable: an undefined name
                // must be reported even though the result is one bit.
                self.info(lhs)?;
                self.info(rhs)?;
                Some(Info::bits(1, false))
            }
            B::Shl | B::Shr | B::Ashl | B::Ashr | B::Pow => {
                let a = self.info(lhs)?;
                self.info(rhs)?;
                Some(a)
            }
            _ => {
                let a = self.info(lhs)?;
                let b = self.info(rhs)?;
                Some(a.combine(&b))
            }
        }
    }

    fn call(&mut self, callee: &Expr, args: &[ast::Arg]) -> Option<Info> {
        if let ExprKind::SystemIdent(id) = &callee.kind {
            return Some(self.system_call(&id.name, args));
        }
        let (f, is_task) = self.env.probe(|env| env.resolve_callee(callee))?;
        if is_task {
            return Some(Info::Other);
        }
        let ret = f.def.ret.as_ref()?;
        if ret.kind == ast::DataTypeKind::Void {
            return Some(Info::Other);
        }
        if ret.is_empty() {
            return Some(Info::bits(1, false));
        }
        self.env.enter(f.home);
        let ty = types::resolve(self.env, ret, &[], true);
        self.env.leave();
        Some(of_type(&ty?))
    }

    fn system_call(&mut self, name: &str, args: &[ast::Arg]) -> Info {
        let first = args.first().and_then(|a| a.value.as_ref());
        match name {
            "signed" | "unsigned" => {
                let w = first.and_then(|e| self.info(e)).map_or(32, |i| i.width());
                Info::bits(w, name == "signed")
            }
            "clog2"
            | "bits"
            | "size"
            | "high"
            | "low"
            | "left"
            | "right"
            | "increment"
            | "dimensions"
            | "unpacked_dimensions"
            | "countones"
            | "rtoi"
            | "random"
            | "urandom"
            | "urandom_range"
            | "fopen"
            | "fgetc"
            | "ferror"
            | "feof" => Info::bits(32, true),
            "onehot" | "onehot0" | "isunknown" | "test$plusargs" | "value$plusargs" => {
                Info::bits(1, false)
            }
            "time" | "stime" => Info::bits(64, false),
            "realtime" | "itor" | "sqrt" | "ln" | "log10" | "exp" | "pow" | "floor" | "ceil" => {
                Info::Real
            }
            "sformatf" | "psprintf" => Info::Str(8),
            _ => Info::bits(32, true),
        }
    }

    fn literal(&mut self, lit: &Literal) -> Info {
        match lit {
            Literal::Number { text, .. } => {
                // `'0`, `'1`, `'x`, `'z` take the context's width; as a
                // self-determined value they are one bit.
                if text.len() == 2 && text.starts_with('\'') {
                    return Info::bits(1, false);
                }
                if let Some((_, _)) = constant::time_literal(text) {
                    return Info::Real;
                }
                match crate::logic::Logic::parse_verilog(text) {
                    Ok(l) => Info::Bits {
                        width: l.width(),
                        signed: l.is_signed(),
                        min: min_bits(&l),
                    },
                    Err(_) => {
                        if text.contains(['.', 'e', 'E']) {
                            Info::Real
                        } else {
                            Info::bits(32, true)
                        }
                    }
                }
            }
            Literal::Str { value, .. } => {
                let w = u32::try_from(value.len().max(1) * 8).unwrap_or(u32::MAX);
                Info::Str(w)
            }
            Literal::Null(_) | Literal::Unbounded(_) => Info::Other,
        }
    }
}

/// The [`Info`] of a constant symbol: its declared type when it has one,
/// otherwise the value's own width, with `min` from the value's bits so a
/// small parameter does not trip the truncation warning.
fn const_info(value: &Value, ty: Option<&VType>) -> Info {
    let base = match ty {
        Some(t) => of_type(t),
        None => of_type(&value.natural_type()),
    };
    match (base, value) {
        (Info::Bits { width, signed, .. }, Value::Logic(l)) => Info::Bits {
            width,
            signed,
            min: min_bits(l).min(width),
        },
        (other, _) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_queries() {
        let i = Info::bits(8, false);
        assert_eq!(i.width(), 8);
        assert!(!i.is_signed());
        assert_eq!(i.min_width(), 8);
        assert_eq!(Info::Real.width(), 64);
        assert!(Info::Real.is_signed());
        assert_eq!(Info::Str(24).width(), 24);
        assert_eq!(Info::Other.width(), 1);
        assert!(!Info::Other.is_signed());
        assert_eq!(Info::Str(8).min_width(), 8);
    }

    #[test]
    fn combine_follows_the_standard() {
        let u8_ = Info::bits(8, false);
        let s16 = Info::bits(16, true);
        assert_eq!(u8_.combine(&s16), Info::bits(16, false));
        assert_eq!(
            Info::bits(4, true).combine(&Info::bits(8, true)),
            Info::bits(8, true)
        );
        assert_eq!(u8_.combine(&Info::Real), Info::Real);
        assert_eq!(Info::Real.combine(&u8_), Info::Real);
        assert_eq!(Info::Str(8).combine(&Info::Str(16)), Info::Str(16));
        assert_eq!(Info::Str(8).combine(&u8_), Info::bits(8, false));
        assert_eq!(Info::Other.combine(&u8_), Info::Other);
    }

    #[test]
    fn type_info() {
        assert_eq!(of_type(&VType::bits(8)), Info::bits(8, false));
        assert_eq!(
            of_type(&VType::Packed(Packed::sbits(4))),
            Info::bits(4, true)
        );
        assert_eq!(of_type(&VType::Real), Info::Real);
        assert_eq!(of_type(&VType::String), Info::Str(8));
        assert_eq!(of_type(&VType::Event), Info::Other);
    }

    #[test]
    fn constant_info_keeps_minimum_width() {
        let v = Value::Logic(crate::logic::Logic::from_u64(3, 32));
        let i = const_info(&v, Some(&VType::bits(32)));
        assert_eq!(
            i,
            Info::Bits {
                width: 32,
                signed: false,
                min: 2
            }
        );
        assert_eq!(const_info(&Value::Real(1.0), None), Info::Real);
    }

    /// Width inference through the lowering: the `.rtl` text shows every
    /// resize the sizing rules demand.
    mod inference {
        use crate::verilog::Dialect;
        use crate::verilog::elab::tests::rtl;

        /// The body of module `t` lowered from `decls` plus `body`.
        #[track_caller]
        fn lower(decls: &str, body: &str) -> String {
            let text = format!("module t;\n{decls}\n{body}\nendmodule\n");
            rtl(&text, Dialect::SystemVerilog)
        }

        /// The lowered text contains `want`.
        #[track_caller]
        fn has(decls: &str, body: &str, want: &str) {
            let out = lower(decls, body);
            assert!(out.contains(want), "expected `{want}` in:\n{out}");
        }

        /// §5.4.1: `a = b + c` with a wider target does the addition at
        /// the target's width, so the carry is kept.
        #[test]
        fn assignment_widens_its_operands() {
            has(
                "logic [15:0] a; logic [7:0] b, c;",
                "assign a = b + c;",
                "assign %a = add(resize(%b, u16), resize(%c, u16))",
            );
        }

        /// A narrower target lets the operator run at the target width,
        /// since the discarded bits cannot influence the kept ones.
        #[test]
        fn assignment_narrows_truncatable_operators() {
            has(
                "logic [7:0] a; logic [15:0] b, c;",
                "assign a = b & c;",
                "assign %a = and(resize(%b, u8), resize(%c, u8))",
            );
        }

        /// A comparison sizes its operands to each other, not to the
        /// context.
        #[test]
        fn comparison_operands_size_to_each_other() {
            has(
                "logic [15:0] a; logic [7:0] b; logic y;",
                "assign y = a > b;",
                "assign %y = gt(%a, resize(%b, u16))",
            );
        }

        /// A signed operand is sign-extended into a signed context and
        /// read as unsigned in an unsigned one (§5.5.1).
        #[test]
        fn signedness_decides_the_extension() {
            has(
                "logic signed [7:0] a; logic signed [15:0] y;",
                "assign y = a;",
                "assign %y = resize(%a, s16)",
            );
            has(
                "logic signed [7:0] a; logic [7:0] b; logic [15:0] y;",
                "assign y = a + b;",
                "assign %y = add(resize(%a, u16), resize(%b, u16))",
            );
        }

        /// The right operand of a shift keeps its own width.
        #[test]
        fn shift_amounts_are_self_determined() {
            has(
                "logic [15:0] d; logic [3:0] n; logic [15:0] y;",
                "assign y = d << n;",
                "assign %y = shl(%d, %n)",
            );
        }

        /// Reductions and logical connectives produce one bit, which is
        /// then extended to the target.
        #[test]
        fn one_bit_results_extend() {
            has(
                "logic [7:0] d; logic [7:0] y;",
                "assign y = ^d;",
                "assign %y = resize(rxor(%d), u8)",
            );
            has(
                "logic a, b; logic [3:0] y;",
                "assign y = a && b;",
                "assign %y = resize(land(%a, %b), u4)",
            );
        }

        /// A multi-bit condition is reduced before it reaches the IR,
        /// which wants a single bit.
        #[test]
        fn conditions_reduce_to_one_bit() {
            has(
                "logic [3:0] c; logic [7:0] a, b, y;",
                "assign y = c ? a : b;",
                "assign %y = mux(ror(%c), %a, %b)",
            );
        }

        /// Concatenation elements keep their own widths.
        #[test]
        fn concatenation_elements_are_self_determined() {
            has(
                "logic [3:0] a; logic [7:0] b; logic [11:0] y;",
                "assign y = {a, b};",
                "assign %y = {%a, %b}",
            );
        }
    }
}
