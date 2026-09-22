//! Syntactic widths and constant values, for the rules that compare them.
//!
//! Nothing here is elaboration: a width is known only when it follows from
//! literals and from parameters whose values are themselves literal, and
//! anything else is `None`. Rules report only on what is known, so a
//! parameterised width that elaboration will resolve never produces a
//! false report here.

use crate::verilog::ast::{
    BinaryOp, CastTarget, DataType, DataTypeKind, Dim, DimKind, Expr, ExprKind, IntegerType,
    Literal, RangeKind, UnaryOp,
};

use super::facts::{DeclKind, FileFacts, ModuleFacts};

/// Recursion limit for parameter chains and typedef chains.
const MAX_DEPTH: u32 = 32;

/// A parsed numeric literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Number {
    /// The explicit size before the `'`, when written.
    pub size: Option<u32>,
    /// The value, when every digit is `0`-`9`/`a`-`f` and it fits 128 bits.
    pub value: Option<u128>,
    /// True when some digit is `x`, `z` or `?`.
    pub has_xz: bool,
    /// True for the `'0` / `'1` / `'x` / `'z` fill form.
    pub fill: bool,
    /// The number of bits the digits as written need (leading zeros
    /// dropped); for a value this is its bit length.
    pub min_bits: u32,
    /// True when the value is written in binary, octal or hexadecimal, so
    /// its digit count is meaningful.
    pub based: bool,
    /// How many bits of the literal are `x`, `z` or `?` digits.
    pub wild_bits: u32,
}

impl Number {
    /// True when the literal has no explicit size (and is not a fill).
    pub fn is_unsized(&self) -> bool {
        self.size.is_none() && !self.fill
    }
}

/// Parses an integer literal's text; `None` for reals and time literals.
pub fn parse_number(text: &str) -> Option<Number> {
    let text: String = text
        .chars()
        .filter(|c| *c != '_' && !c.is_whitespace())
        .collect();
    if text.contains(['.', 'e', 'E']) && !text.contains('\'') {
        // A real, or `1e3`; time units (`10ns`) also fall through below.
        return None;
    }
    let Some(apos) = text.find('\'') else {
        // Plain decimal, possibly with a time unit.
        let digits = text.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        if digits.is_empty() || digits.len() != text.len() {
            return None;
        }
        let value: u128 = digits.parse().ok()?;
        return Some(Number {
            size: None,
            value: Some(value),
            has_xz: false,
            fill: false,
            min_bits: bit_len(value),
            based: false,
            wild_bits: 0,
        });
    };
    let size = if apos == 0 {
        None
    } else {
        Some(text[..apos].parse::<u32>().ok()?)
    };
    let mut rest = &text[apos + 1..];
    let mut signed = false;
    if let Some(r) = rest.strip_prefix(['s', 'S']) {
        signed = true;
        rest = r;
    }
    let _ = signed;
    let (bits_per_digit, digits) = match rest.chars().next()? {
        'b' | 'B' => (1, &rest[1..]),
        'o' | 'O' => (3, &rest[1..]),
        'd' | 'D' => (0, &rest[1..]),
        'h' | 'H' => (4, &rest[1..]),
        '0' | '1' | 'x' | 'X' | 'z' | 'Z' if rest.len() == 1 && size.is_none() => {
            let c = rest.chars().next()?;
            let has_xz = !matches!(c, '0' | '1');
            return Some(Number {
                size: None,
                value: (!has_xz).then_some(u128::from(c == '1')),
                has_xz,
                fill: true,
                min_bits: 1,
                based: true,
                wild_bits: u32::from(has_xz),
            });
        }
        _ => return None,
    };
    if digits.is_empty() {
        return None;
    }
    let has_xz = digits.contains(['x', 'X', 'z', 'Z', '?']);
    if bits_per_digit == 0 {
        // Decimal.
        if has_xz {
            return Some(Number {
                size,
                value: None,
                has_xz: true,
                fill: false,
                min_bits: 1,
                based: false,
                wild_bits: size.unwrap_or(32),
            });
        }
        let value: u128 = digits.parse().ok()?;
        return Some(Number {
            size,
            value: Some(value),
            has_xz: false,
            fill: false,
            min_bits: bit_len(value),
            based: false,
            wild_bits: 0,
        });
    }
    let significant = digits.trim_start_matches('0');
    let min_bits = if has_xz {
        u32::try_from(significant.len()).unwrap_or(u32::MAX) * bits_per_digit
    } else {
        0
    };
    let mut value: Option<u128> = if has_xz { None } else { Some(0) };
    for c in digits.chars() {
        let Some(v) = value else { break };
        let d = u128::from(c.to_digit(16)?);
        value = v
            .checked_shl(bits_per_digit)
            .filter(|_| v.leading_zeros() >= bits_per_digit)
            .and_then(|s| s.checked_add(d));
    }
    let min_bits = match value {
        Some(v) => bit_len(v),
        None if has_xz => min_bits,
        // Overflowed 128 bits: still wider than any width we compare.
        None => 129,
    };
    let wild_digits = digits
        .chars()
        .filter(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'))
        .count();
    Some(Number {
        size,
        value,
        has_xz,
        fill: false,
        min_bits,
        based: true,
        wild_bits: u32::try_from(wild_digits).unwrap_or(0) * bits_per_digit,
    })
}

/// The number of bits needed to hold `v` (`0` for zero).
fn bit_len(v: u128) -> u32 {
    128 - v.leading_zeros()
}

/// The literal a `Literal` expression holds, when it is a parseable integer.
pub fn literal_of(e: &Expr) -> Option<Number> {
    match &e.kind {
        ExprKind::Literal(Literal::Number { text, .. }) => parse_number(text),
        _ => None,
    }
}

/// Evaluates a constant expression: literals, parameters with constant
/// initialisers, arithmetic, comparisons, `?:` and `$clog2`.
///
/// Names that do not resolve inside the module (a package constant, a
/// parameter an instantiation overrides) yield `None`, which every caller
/// treats as "say nothing".
pub fn const_eval(m: &ModuleFacts<'_>, e: &Expr) -> Option<i128> {
    eval(m, e, 0)
}

fn eval(m: &ModuleFacts<'_>, e: &Expr, depth: u32) -> Option<i128> {
    if depth > MAX_DEPTH {
        return None;
    }
    match &e.kind {
        ExprKind::Literal(Literal::Number { text, .. }) => {
            let n = parse_number(text)?;
            let v = n.value?;
            let mut v = i128::try_from(v).ok()?;
            // A sized signed literal with its top bit set is negative.
            if let Some(size) = n.size
                && size < 127
                && text.contains(['s', 'S'])
                && v >> (size - 1) & 1 == 1
            {
                v -= 1i128 << size;
            }
            Some(v)
        }
        ExprKind::Ident(id) => {
            let d = m.lookup(&id.name)?;
            let decl = m.decl(d);
            match decl.kind {
                DeclKind::Param(_) => eval(m, decl.init?, depth + 1),
                _ => None,
            }
        }
        ExprKind::Unary { op, operand } => {
            let v = eval(m, operand, depth + 1)?;
            match op {
                UnaryOp::Plus => Some(v),
                UnaryOp::Minus => v.checked_neg(),
                UnaryOp::LogicNot => Some(i128::from(v == 0)),
                _ => None,
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval(m, lhs, depth + 1)?;
            let b = eval(m, rhs, depth + 1)?;
            match op {
                BinaryOp::Add => a.checked_add(b),
                BinaryOp::Sub => a.checked_sub(b),
                BinaryOp::Mul => a.checked_mul(b),
                BinaryOp::Div => a.checked_div(b),
                BinaryOp::Mod => a.checked_rem(b),
                BinaryOp::Pow => a.checked_pow(u32::try_from(b).ok()?),
                BinaryOp::Shl | BinaryOp::Ashl => a.checked_shl(u32::try_from(b).ok()?),
                BinaryOp::Shr | BinaryOp::Ashr => a.checked_shr(u32::try_from(b).ok()?),
                BinaryOp::Lt => Some(i128::from(a < b)),
                BinaryOp::Le => Some(i128::from(a <= b)),
                BinaryOp::Gt => Some(i128::from(a > b)),
                BinaryOp::Ge => Some(i128::from(a >= b)),
                BinaryOp::Eq | BinaryOp::CaseEq => Some(i128::from(a == b)),
                BinaryOp::Ne | BinaryOp::CaseNe => Some(i128::from(a != b)),
                BinaryOp::LogicAnd => Some(i128::from(a != 0 && b != 0)),
                BinaryOp::LogicOr => Some(i128::from(a != 0 || b != 0)),
                BinaryOp::BitAnd => Some(a & b),
                BinaryOp::BitOr => Some(a | b),
                BinaryOp::BitXor => Some(a ^ b),
                _ => None,
            }
        }
        ExprKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            let c = eval(m, cond, depth + 1)?;
            eval(m, if c != 0 { then_expr } else { else_expr }, depth + 1)
        }
        ExprKind::Call { callee, args } => {
            if let ExprKind::SystemIdent(id) = &callee.kind
                && id.name == "clog2"
                && args.len() == 1
            {
                let v = eval(m, args[0].value.as_ref()?, depth + 1)?;
                let v = u128::try_from(v).ok()?;
                return Some(i128::from(if v <= 1 { 0 } else { bit_len(v - 1) }));
            }
            None
        }
        ExprKind::Cast { target, expr } => match target {
            CastTarget::Size(_) | CastTarget::Signing(_) | CastTarget::Const => {
                eval(m, expr, depth + 1)
            }
            CastTarget::Type(_) => eval(m, expr, depth + 1),
        },
        _ => None,
    }
}

/// The number of elements of a dimension, when constant.
pub fn dim_len(m: &ModuleFacts<'_>, d: &Dim) -> Option<u64> {
    match &d.kind {
        DimKind::Range(a, b) => {
            let a = const_eval(m, a)?;
            let b = const_eval(m, b)?;
            u64::try_from((a - b).abs() + 1).ok()
        }
        DimKind::Size(n) => u64::try_from(const_eval(m, n)?).ok(),
        _ => None,
    }
}

/// The `[msb:lsb]` bounds of a dimension, normalised to `(lo, hi)`.
pub fn dim_bounds(m: &ModuleFacts<'_>, d: &Dim) -> Option<(i128, i128)> {
    match &d.kind {
        DimKind::Range(a, b) => {
            let a = const_eval(m, a)?;
            let b = const_eval(m, b)?;
            Some((a.min(b), a.max(b)))
        }
        DimKind::Size(n) => {
            let n = const_eval(m, n)?;
            Some((0, n - 1))
        }
        _ => None,
    }
}

/// The packed width of a data type in bits, when it follows from the
/// source.
pub fn type_width(file: &FileFacts<'_>, m: &ModuleFacts<'_>, dt: &DataType) -> Option<u64> {
    type_width_at(file, m, dt, 0)
}

fn type_width_at(
    file: &FileFacts<'_>,
    m: &ModuleFacts<'_>,
    dt: &DataType,
    depth: u32,
) -> Option<u64> {
    if depth > MAX_DEPTH {
        return None;
    }
    let packed = |base: u64| -> Option<u64> {
        let mut w = base;
        for d in &dt.packed {
            w = w.checked_mul(dim_len(m, d)?)?;
        }
        Some(w)
    };
    match &dt.kind {
        DataTypeKind::Implicit => packed(1),
        DataTypeKind::Integer(t) => match t {
            IntegerType::Bit | IntegerType::Logic | IntegerType::Reg => packed(1),
            IntegerType::Byte => Some(8),
            IntegerType::Shortint => Some(16),
            IntegerType::Int | IntegerType::Integer => Some(32),
            IntegerType::Longint | IntegerType::Time => Some(64),
        },
        DataTypeKind::Enum(e) => match &e.base {
            Some(base) => type_width_at(file, m, base, depth + 1),
            None => Some(32),
        },
        DataTypeKind::Struct(s) if s.packed && !s.is_union => {
            let mut w = 0u64;
            for member in &s.members {
                let mw = type_width_at(file, m, &member.data_type, depth + 1)?;
                for d in &member.decls {
                    let mut dw = mw;
                    for dim in &d.dims {
                        dw = dw.checked_mul(dim_len(m, dim)?)?;
                    }
                    w = w.checked_add(dw)?;
                }
            }
            Some(w)
        }
        DataTypeKind::Named {
            package: None,
            name,
            member: None,
        } => {
            let resolved = file.resolve_typedef(m, &name.name)?;
            let w = type_width_at(file, m, resolved, depth + 1)?;
            packed(w)
        }
        _ => None,
    }
}

/// How well an expression's width is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthKind {
    /// The expression is exactly this wide (a sized literal, a signal, a
    /// select, a concatenation).
    Exact,
    /// The expression is at least this wide: an arithmetic or bitwise
    /// operator whose result the assignment context may extend.
    AtLeast,
}

/// A known width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Width {
    /// Bits.
    pub bits: u64,
    /// Exact or a lower bound.
    pub kind: WidthKind,
}

impl Width {
    fn exact(bits: u64) -> Self {
        Width {
            bits,
            kind: WidthKind::Exact,
        }
    }

    fn at_least(bits: u64) -> Self {
        Width {
            bits,
            kind: WidthKind::AtLeast,
        }
    }
}

/// The packed width of one element of a declaration (the whole value for
/// a scalar or vector, one element for an unpacked array).
pub fn decl_width(file: &FileFacts<'_>, m: &ModuleFacts<'_>, id: usize) -> Option<u64> {
    let decl = m.decl(id);
    match decl.kind {
        DeclKind::Net(_) | DeclKind::Var | DeclKind::Port(_) | DeclKind::SubPort(_) => {
            type_width(file, m, decl.data_type?)
        }
        DeclKind::Param(_) => {
            let dt = decl.data_type?;
            if dt.is_empty() {
                // `parameter P = 8'hff` takes the width of its literal;
                // an unsized literal leaves it unknown.
                literal_of(decl.init?)?.size.map(u64::from)
            } else {
                type_width(file, m, dt)
            }
        }
        _ => None,
    }
}

/// The width of an expression, when it follows from literals and constant
/// declarations.
pub fn expr_width(file: &FileFacts<'_>, m: &ModuleFacts<'_>, e: &Expr) -> Option<Width> {
    match &e.kind {
        ExprKind::Literal(Literal::Number { text, .. }) => {
            let n = parse_number(text)?;
            n.size.map(|s| Width::exact(u64::from(s)))
        }
        ExprKind::Ident(id) => {
            let d = m.lookup(&id.name)?;
            if !m.decl(d).unpacked.is_empty() {
                return None;
            }
            decl_width(file, m, d).map(Width::exact)
        }
        ExprKind::Index { .. } => {
            // Count the index levels down to the root.
            let mut levels = 0usize;
            let mut cur = e;
            while let ExprKind::Index { base, .. } = &cur.kind {
                levels += 1;
                cur = base;
            }
            let ExprKind::Ident(id) = &cur.kind else {
                return None;
            };
            let d = m.lookup(&id.name)?;
            let decl = m.decl(d);
            let unpacked = decl.unpacked.len();
            let dt = decl.data_type?;
            if levels < unpacked {
                return None;
            }
            let packed_levels = levels - unpacked;
            // Packed dimensions from the outermost; the base type's own
            // width is the innermost.
            let base = type_width(
                file,
                m,
                &DataType {
                    kind: dt.kind.clone(),
                    signing: dt.signing,
                    packed: Vec::new(),
                    span: dt.span,
                },
            )?;
            let dims: Vec<u64> = dt
                .packed
                .iter()
                .map(|d| dim_len(m, d))
                .collect::<Option<_>>()?;
            if packed_levels == 0 {
                return Some(Width::exact(dims.iter().product::<u64>() * base));
            }
            if packed_levels <= dims.len() {
                return Some(Width::exact(
                    dims[packed_levels..].iter().product::<u64>() * base,
                ));
            }
            if packed_levels == dims.len() + 1 && base > 1 {
                return Some(Width::exact(1));
            }
            None
        }
        ExprKind::Range {
            kind, left, right, ..
        } => match kind {
            RangeKind::Fixed => {
                let a = const_eval(m, left)?;
                let b = const_eval(m, right)?;
                u64::try_from((a - b).abs() + 1).ok().map(Width::exact)
            }
            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                u64::try_from(const_eval(m, right)?).ok().map(Width::exact)
            }
        },
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => expr_width(file, m, operand),
            _ => Some(Width::exact(1)),
        },
        ExprKind::Binary { op, lhs, rhs } => match op {
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitXnor => {
                let a = expr_width(file, m, lhs)?;
                let b = expr_width(file, m, rhs)?;
                Some(Width::at_least(a.bits.max(b.bits)))
            }
            BinaryOp::Pow | BinaryOp::Shl | BinaryOp::Shr | BinaryOp::Ashl | BinaryOp::Ashr => {
                let a = expr_width(file, m, lhs)?;
                Some(Width::at_least(a.bits))
            }
            _ => Some(Width::exact(1)),
        },
        ExprKind::Ternary {
            then_expr,
            else_expr,
            ..
        } => {
            let a = expr_width(file, m, then_expr)?;
            let b = expr_width(file, m, else_expr)?;
            let kind = if a.kind == WidthKind::Exact && b.kind == WidthKind::Exact {
                WidthKind::Exact
            } else {
                WidthKind::AtLeast
            };
            Some(Width {
                bits: a.bits.max(b.bits),
                kind,
            })
        }
        ExprKind::Concat(elems) => {
            let mut w = 0u64;
            for x in elems {
                w = w.checked_add(expr_width(file, m, x)?.bits)?;
            }
            Some(Width::exact(w))
        }
        ExprKind::Replicate { count, elems } => {
            let n = u64::try_from(const_eval(m, count)?).ok()?;
            let mut w = 0u64;
            for x in elems {
                w = w.checked_add(expr_width(file, m, x)?.bits)?;
            }
            w.checked_mul(n).map(Width::exact)
        }
        ExprKind::Cast { target, expr } => match target {
            CastTarget::Size(n) => u64::try_from(const_eval(m, n)?).ok().map(Width::exact),
            CastTarget::Type(t) => type_width(file, m, t).map(Width::exact),
            CastTarget::Signing(_) | CastTarget::Const => expr_width(file, m, expr),
        },
        ExprKind::Call { callee, args } => match &callee.kind {
            ExprKind::Ident(id) => {
                let d = m.lookup(&id.name)?;
                let decl = m.decl(d);
                if decl.kind != DeclKind::Function {
                    return None;
                }
                let ret = decl.data_type?;
                if matches!(ret.kind, DataTypeKind::Void) {
                    return None;
                }
                type_width(file, m, ret).map(Width::exact)
            }
            ExprKind::SystemIdent(id) if matches!(id.name.as_str(), "signed" | "unsigned") => {
                expr_width(file, m, args.first()?.value.as_ref()?)
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literal_forms() {
        let n = parse_number("8'hff").unwrap();
        assert_eq!((n.size, n.value, n.min_bits), (Some(8), Some(255), 8));
        let n = parse_number("8'h1ff").unwrap();
        assert_eq!(n.min_bits, 9);
        let n = parse_number("4'b1x0z").unwrap();
        assert!(n.has_xz && n.value.is_none());
        assert_eq!(n.min_bits, 4);
        assert_eq!(n.wild_bits, 2);
        assert_eq!(parse_number("8'h1?").unwrap().wild_bits, 4);
        let n = parse_number("2'b1xx").unwrap();
        assert_eq!(n.min_bits, 3);
        let n = parse_number("42").unwrap();
        assert!(n.is_unsized());
        assert_eq!(n.value, Some(42));
        let n = parse_number("'0").unwrap();
        assert!(n.fill && !n.is_unsized());
        let n = parse_number("16'd65_535").unwrap();
        assert_eq!(n.value, Some(65535));
        assert_eq!(parse_number("1.5e3"), None);
        assert_eq!(parse_number("10ns"), None);
        assert_eq!(parse_number("4'sd3").unwrap().value, Some(3));
        assert_eq!(parse_number("8 'h ff").unwrap().value, Some(255));
    }
}
