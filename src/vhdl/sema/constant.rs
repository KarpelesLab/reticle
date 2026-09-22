//! Compile-time evaluation of locally static expressions (IEEE 1076-2008
//! clause 9.4).
//!
//! The checker records a [`Value`] for every expression it can evaluate
//! without elaborating anything: literals, constants with static
//! initialisers, enumeration literals, the predefined operators on static
//! operands, predefined attributes of static subtypes, and calls to the
//! functions of the bundled `ieee` packages when their arguments are static
//! (so `to_unsigned(5, 8)` or `x"F0" + 1` fold, as clause 9.4.2 requires of
//! a locally static expression). Anything that depends on a generic, a
//! signal or a user-defined function is left to elaboration, which owns the
//! generic values and can interpret subprogram bodies.
//!
//! Integers are `i128` (VHDL integers are at most 64-bit, and physical
//! values in femtoseconds need more than 32 bits), reals are `f64`,
//! enumeration values are positions, arrays are element vectors with their
//! left bound and direction, and records are field vectors. A `std_logic`
//! or `bit` vector is therefore an array of enumeration positions, and
//! [`Value::to_std9`] / [`Value::from_std9`] bridge it to the crate's
//! [`Std9`] / [`Logic`] representation for the `numeric_std` arithmetic in
//! [`super::check`].

use std::fmt;

use crate::logic::{Bit, Logic, Std9};
use crate::vhdl::ast::Direction;

/// A locally static value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An integer, universal integer, or physical value in primary units.
    Int(i128),
    /// A real or universal real.
    Real(f64),
    /// An enumeration value by position (`boolean`: 0 false, 1 true).
    Enum(u32),
    /// An array value.
    Array(ArrayValue),
    /// A record value, one entry per field in declaration order.
    Record(Vec<Value>),
    /// The `null` access value.
    Null,
}

/// The value of a one-dimensional array.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrayValue {
    /// The left bound (integer or enumeration position).
    pub left: i128,
    /// The index direction.
    pub dir: Direction,
    /// The elements, leftmost first.
    pub elems: Vec<Value>,
}

impl ArrayValue {
    /// The right bound.
    pub fn right(&self) -> i128 {
        let n = i128::try_from(self.elems.len()).unwrap_or(i128::MAX);
        match self.dir {
            Direction::To => self.left + n - 1,
            Direction::Downto => self.left - n + 1,
        }
    }

    /// The element at logical index `i`, if in range.
    pub fn get(&self, i: i128) -> Option<&Value> {
        let off = match self.dir {
            Direction::To => i - self.left,
            Direction::Downto => self.left - i,
        };
        usize::try_from(off).ok().and_then(|o| self.elems.get(o))
    }
}

impl Value {
    /// The boolean value, for an enumeration value of a boolean-like type.
    pub fn from_bool(b: bool) -> Value {
        Value::Enum(u32::from(b))
    }

    /// The integer (or enumeration position, or physical amount), if any.
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Enum(p) => Some(i128::from(*p)),
            _ => None,
        }
    }

    /// The real value, if any (integers are not converted).
    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(r) => Some(*r),
            _ => None,
        }
    }

    /// The enumeration position, if any.
    pub fn as_enum(&self) -> Option<u32> {
        match self {
            Value::Enum(p) => Some(*p),
            _ => None,
        }
    }

    /// The boolean interpretation of an enumeration position.
    pub fn as_bool(&self) -> Option<bool> {
        self.as_enum().map(|p| p != 0)
    }

    /// The array value, if any.
    pub fn as_array(&self) -> Option<&ArrayValue> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Interprets an array of `std_ulogic` positions (the IEEE 1164 order
    /// `U X 0 1 Z W L H -`) as nine-state elements, leftmost first.
    pub fn to_std9(&self) -> Option<Vec<Std9>> {
        let a = self.as_array()?;
        a.elems
            .iter()
            .map(|e| {
                let p = e.as_enum()?;
                Std9::ALL.get(usize::try_from(p).ok()?).copied()
            })
            .collect()
    }

    /// Interprets an array of `bit` positions (`'0'`, `'1'`) as four-state
    /// bits, leftmost first.
    pub fn to_bits(&self) -> Option<Vec<Bit>> {
        let a = self.as_array()?;
        a.elems
            .iter()
            .map(|e| match e.as_enum()? {
                0 => Some(Bit::Zero),
                1 => Some(Bit::One),
                _ => None,
            })
            .collect()
    }

    /// Interprets an array of `std_ulogic` or `bit` positions as a
    /// [`Logic`] vector, the leftmost element being the MSB.
    pub fn to_logic(&self, std_logic: bool) -> Option<Logic> {
        if std_logic {
            let mut v = self.to_std9()?;
            v.reverse();
            Some(Logic::from_std9(&v))
        } else {
            let mut v = self.to_bits()?;
            v.reverse();
            Some(Logic::from_bits(&v))
        }
    }

    /// Builds a `downto` array of `std_ulogic` positions from nine-state
    /// elements given leftmost first, with left bound `len - 1`.
    pub fn from_std9(elems: &[Std9]) -> Value {
        let n = i128::try_from(elems.len()).unwrap_or(0);
        Value::Array(ArrayValue {
            left: n - 1,
            dir: Direction::Downto,
            elems: elems
                .iter()
                .map(|s| Value::Enum(u32::try_from(s.index()).unwrap_or(0)))
                .collect(),
        })
    }

    /// Builds a `downto` array of `std_ulogic` or `bit` positions from a
    /// [`Logic`] vector, MSB leftmost.
    pub fn from_logic(l: &Logic, std_logic: bool) -> Value {
        let n = i128::from(l.width());
        let elems = (0..l.width())
            .rev()
            .map(|i| {
                let b = l.bit(i);
                if std_logic {
                    Value::Enum(u32::try_from(Std9::from_bit(b).index()).unwrap_or(0))
                } else {
                    Value::Enum(u32::from(b == Bit::One))
                }
            })
            .collect();
        Value::Array(ArrayValue {
            left: n - 1,
            dir: Direction::Downto,
            elems,
        })
    }

    /// Builds a `1 to n` string value from character positions.
    pub fn string(positions: impl IntoIterator<Item = u32>) -> Value {
        Value::Array(ArrayValue {
            left: 1,
            dir: Direction::To,
            elems: positions.into_iter().map(Value::Enum).collect(),
        })
    }

    /// Three-way comparison of two scalar values of the same class.
    pub fn compare(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Enum(a), Value::Enum(b)) => Some(a.cmp(b)),
            (Value::Real(a), Value::Real(b)) => a.partial_cmp(b),
            (Value::Array(a), Value::Array(b)) => {
                // Clause 9.2.3: lexicographic on discrete arrays.
                for (x, y) in a.elems.iter().zip(&b.elems) {
                    match x.compare(y)? {
                        std::cmp::Ordering::Equal => {}
                        o => return Some(o),
                    }
                }
                Some(a.elems.len().cmp(&b.elems.len()))
            }
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    /// A type-agnostic rendering: enumeration values print as `#pos`,
    /// arrays as `(..)`. Use [`super::Analysis::describe_value`] for a
    /// rendering that knows the type.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Real(r) => write!(f, "{r:?}"),
            Value::Enum(p) => write!(f, "#{p}"),
            Value::Array(a) => {
                f.write_str("(")?;
                for (i, e) in a.elems.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{e}")?;
                }
                f.write_str(")")
            }
            Value::Record(fields) => {
                f.write_str("(")?;
                for (i, e) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{e}")?;
                }
                f.write_str(")")
            }
            Value::Null => f.write_str("null"),
        }
    }
}

/// Parses the digits of a VHDL abstract literal (clause 15.5): decimal
/// (`42`, `1E3`) or based (`16#FF#`, `2#1010#E2`), with `_` separators.
/// Returns `None` for malformed text or a value that does not fit.
pub fn parse_integer_literal(text: &str) -> Option<i128> {
    let t: String = text.chars().filter(|&c| c != '_').collect();
    let (base, body) = split_base(&t)?;
    let (mantissa, exp) = split_exponent(body);
    if mantissa.contains('.') {
        return None;
    }
    let mut v: i128 = 0;
    for c in mantissa.chars() {
        let d = i128::from(c.to_digit(16)?);
        if d >= base {
            return None;
        }
        v = v.checked_mul(base)?.checked_add(d)?;
    }
    let exp: u32 = match exp {
        Some(e) => e.trim_start_matches('+').parse().ok()?,
        None => 0,
    };
    for _ in 0..exp {
        v = v.checked_mul(base)?;
    }
    Some(v)
}

/// Parses a VHDL real literal: decimal (`3.14`, `1.0E-3`) or based
/// (`16#F.8#`).
pub fn parse_real_literal(text: &str) -> Option<f64> {
    let t: String = text.chars().filter(|&c| c != '_').collect();
    let (base, body) = split_base(&t)?;
    let (mantissa, exp) = split_exponent(body);
    let exp: i32 = match exp {
        Some(e) => e.trim_start_matches('+').parse().ok()?,
        None => 0,
    };
    if base == 10 {
        let v: f64 = mantissa.parse().ok()?;
        return Some(v * 10f64.powi(exp));
    }
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let basef = base as f64;
    let mut v = 0.0;
    for c in int_part.chars() {
        let d = c.to_digit(16)?;
        if i128::from(d) >= base {
            return None;
        }
        v = v * basef + f64::from(d);
    }
    let mut scale = 1.0 / basef;
    for c in frac_part.chars() {
        let d = c.to_digit(16)?;
        if i128::from(d) >= base {
            return None;
        }
        v += f64::from(d) * scale;
        scale /= basef;
    }
    Some(v * basef.powi(exp))
}

/// Splits `base#digits#[exp]` into `(base, "digits[exp]")`; decimal text
/// yields base 10 unchanged.
fn split_base(t: &str) -> Option<(i128, &str)> {
    let Some(hash) = t.find('#') else {
        return Some((10, t));
    };
    let base: i128 = t[..hash].parse().ok()?;
    if !(2..=16).contains(&base) {
        return None;
    }
    let rest = &t[hash + 1..];
    let close = rest.find('#')?;
    let digits = &rest[..close];
    let exp = &rest[close + 1..];
    // Reassemble digits + exponent for the caller; the exponent keeps its
    // leading `e`.
    if exp.is_empty() {
        Some((base, digits))
    } else {
        // Based literals cannot borrow a joined string; handle here by
        // returning the digits and letting the exponent be parsed below.
        Some((base, &rest[..close + 1 + exp.len()]))
    }
}

/// Splits `mantissa[e|E exponent]` (or `mantissa#exponent` for a based
/// literal rejoined by [`split_base`]).
fn split_exponent(body: &str) -> (&str, Option<&str>) {
    if let Some(i) = body.find('#') {
        let exp = body[i + 1..].trim_start_matches(['e', 'E']);
        return (&body[..i], Some(exp));
    }
    match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    }
}

/// Integer arithmetic of the predefined operators, `None` on overflow or
/// division by zero.
pub mod int {
    /// `a mod b` with the sign of `b` (clause 9.2.7).
    pub fn modulo(a: i128, b: i128) -> Option<i128> {
        if b == 0 {
            return None;
        }
        let r = a.checked_rem(b)?;
        if r != 0 && (r < 0) != (b < 0) {
            r.checked_add(b)
        } else {
            Some(r)
        }
    }

    /// `a rem b` with the sign of `a`.
    pub fn remainder(a: i128, b: i128) -> Option<i128> {
        if b == 0 {
            return None;
        }
        a.checked_rem(b)
    }

    /// `a ** n` for a non-negative exponent.
    pub fn power(a: i128, n: i128) -> Option<i128> {
        if n < 0 {
            return None;
        }
        let mut r: i128 = 1;
        for _ in 0..n {
            r = r.checked_mul(a)?;
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_integer_literals() {
        assert_eq!(parse_integer_literal("42"), Some(42));
        assert_eq!(parse_integer_literal("1_000"), Some(1000));
        assert_eq!(parse_integer_literal("1E3"), Some(1000));
        assert_eq!(parse_integer_literal("16#FF#"), Some(255));
        assert_eq!(parse_integer_literal("2#1010#E2"), Some(40));
        assert_eq!(parse_integer_literal("2#12#"), None);
        assert_eq!(parse_integer_literal("1.5"), None);
        assert_eq!(parse_integer_literal("17#1#"), None);
    }

    #[test]
    fn parses_real_literals() {
        assert_eq!(parse_real_literal("3.5"), Some(3.5));
        assert_eq!(parse_real_literal("1.0E-3"), Some(0.001));
        assert_eq!(parse_real_literal("16#F.8#"), Some(15.5));
        assert_eq!(parse_real_literal("2#1.1#E1"), Some(3.0));
    }

    #[test]
    fn integer_ops() {
        assert_eq!(int::modulo(-7, 3), Some(2));
        assert_eq!(int::modulo(7, -3), Some(-2));
        assert_eq!(int::remainder(-7, 3), Some(-1));
        assert_eq!(int::remainder(7, 0), None);
        assert_eq!(int::power(2, 10), Some(1024));
        assert_eq!(int::power(2, -1), None);
    }

    #[test]
    fn logic_round_trip() {
        let v = Value::from_std9(&[Std9::One, Std9::Zero, Std9::Z]);
        let a = v.as_array().unwrap();
        assert_eq!(a.left, 2);
        assert_eq!(a.right(), 0);
        assert_eq!(v.to_std9().unwrap(), vec![Std9::One, Std9::Zero, Std9::Z]);
        let l = v.to_logic(true).unwrap();
        assert_eq!(l.width(), 3);
        assert_eq!(l.bit(2), Bit::One);
        assert_eq!(l.bit(0), Bit::Z);
        assert_eq!(Value::from_logic(&l, true), v);
        assert_eq!(a.get(2), Some(&Value::Enum(3)));
        assert_eq!(a.get(3), None);
    }

    #[test]
    fn comparisons() {
        use std::cmp::Ordering;
        assert_eq!(Value::Int(1).compare(&Value::Int(2)), Some(Ordering::Less));
        assert_eq!(
            Value::string([1, 2]).compare(&Value::string([1, 2, 3])),
            Some(Ordering::Less)
        );
        assert_eq!(Value::Int(1).compare(&Value::Real(1.0)), None);
        assert_eq!(Value::from_bool(true).as_bool(), Some(true));
        assert_eq!(format!("{}", Value::string([1, 2])), "(#1, #2)");
    }
}
