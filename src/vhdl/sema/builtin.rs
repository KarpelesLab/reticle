//! Static evaluation of the bundled library functions.
//!
//! The functions of `ieee.numeric_std`, `ieee.std_logic_1164` and
//! `ieee.math_real` have VHDL bodies in [`crate::vhdl::stdlib`], but the
//! checker does not interpret subprogram bodies: a locally static
//! expression (clause 9.4.2) may call them, and a design that writes
//! `constant ONE : unsigned(7 downto 0) := to_unsigned(1, 8);` expects the
//! value to be known at analysis time.
//!
//! `Checker::builtin_call` therefore recognises a call whose target is
//! declared in one of those packages and computes the result natively from
//! the arguments' [`Value`]s, using [`Logic`] for the vector arithmetic so
//! the semantics match what the simulator will do. Anything it does not
//! recognise, or any call with a non-static argument, simply yields no
//! value, which makes the expression non-static and leaves it to
//! elaboration.

use crate::intern::Symbol;
use crate::logic::{Bit, Logic, Std9};
use crate::source::Span;

use super::check::Checker;
use super::constant::{ArrayValue, Value};
use super::library::LibraryUnitKind;
use super::types::{TypeClass, TypeId};
use super::{DeclId, DeclKind, RegionId};

/// Which bundled package a subprogram comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pkg {
    NumericStd,
    StdLogic1164,
    MathReal,
    Standard,
}

impl Checker<'_> {
    /// The region of package `lib.name`, if it has been analysed.
    fn package_region(&self, lib: Symbol, name: Symbol) -> Option<RegionId> {
        self.a
            .units
            .iter()
            .find(|u| u.library == lib && u.name == name && u.kind == LibraryUnitKind::Package)
            .and_then(|u| u.region)
    }

    /// Which bundled package declares `d`, if any.
    fn bundled_package(&self, d: DeclId) -> Option<Pkg> {
        let region = self.a.decl(d).region;
        let ieee = self.syms.ieee;
        let std = self.syms.std;
        for (lib, name, pkg) in [
            (ieee, self.syms.numeric_std, Pkg::NumericStd),
            (ieee, self.syms.std_logic_1164, Pkg::StdLogic1164),
            (ieee, self.syms.math_real, Pkg::MathReal),
            (std, self.syms.standard, Pkg::Standard),
        ] {
            if self.package_region(lib, name) == Some(region) {
                return Some(pkg);
            }
        }
        None
    }

    /// Evaluates a static call to a bundled library function.
    pub(crate) fn builtin_call(&mut self, d: DeclId, args: &[Span], ret: TypeId) -> Option<Value> {
        let pkg = self.bundled_package(d)?;
        let values: Vec<Value> = args
            .iter()
            .map(|s| self.a.value_of(*s).cloned())
            .collect::<Option<Vec<_>>>()?;
        let name = self.a.decl(d).spelling.to_lowercase();
        let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind else {
            return None;
        };
        let arg_tys: Vec<TypeId> = sig.params.iter().map(|p| p.ty).collect();
        match pkg {
            Pkg::MathReal => self.fold_math_real(&name, &values),
            Pkg::StdLogic1164 => self.fold_1164(&name, &values, &arg_tys, ret),
            Pkg::NumericStd => self.fold_numeric_std(&name, &values, &arg_tys, ret),
            Pkg::Standard => self.fold_standard(&name, &values, &arg_tys, ret),
        }
    }

    fn fold_math_real(&self, name: &str, v: &[Value]) -> Option<Value> {
        let x = v.first()?.as_real()?;
        let r = match name {
            "sqrt" => x.sqrt(),
            "cbrt" => x.cbrt(),
            "exp" => x.exp(),
            "log" if v.len() == 1 => x.ln(),
            "log2" => x.log2(),
            "log10" => x.log10(),
            "sin" => x.sin(),
            "cos" => x.cos(),
            "tan" => x.tan(),
            "arcsin" => x.asin(),
            "arccos" => x.acos(),
            "arctan" if v.len() == 1 => x.atan(),
            "sinh" => x.sinh(),
            "cosh" => x.cosh(),
            "tanh" => x.tanh(),
            "floor" => x.floor(),
            "ceil" => x.ceil(),
            "round" => x.round(),
            "trunc" => x.trunc(),
            "sign" => {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            "realmax" => x.max(v.get(1)?.as_real()?),
            "realmin" => x.min(v.get(1)?.as_real()?),
            "log" => x.ln() / v.get(1)?.as_real()?.ln(),
            "arctan" => x.atan2(v.get(1)?.as_real()?),
            _ => return None,
        };
        Some(Value::Real(r))
    }

    fn fold_standard(
        &self,
        name: &str,
        v: &[Value],
        arg_tys: &[TypeId],
        ret: TypeId,
    ) -> Option<Value> {
        match name {
            "minimum" | "maximum" => {
                let (a, b) = (v.first()?, v.get(1)?);
                let ord = a.compare(b)?;
                let pick = if (name == "minimum") == ord.is_le() {
                    a
                } else {
                    b
                };
                Some(pick.clone())
            }
            "to_string" => {
                let t = arg_tys.first().copied()?;
                let text = self.a.describe_value(v.first()?, t);
                let text = text.trim_matches('"').to_owned();
                Some(Value::string(text.chars().map(|c| c as u32)))
            }
            _ => {
                let _ = ret;
                None
            }
        }
    }

    /// `std_logic_1164`: conversions, edge detection and string
    /// rendering. The logic operators are handled as implicit operators
    /// on `std_ulogic`, which this package declares as functions, so they
    /// arrive here too.
    fn fold_1164(&self, name: &str, v: &[Value], arg_tys: &[TypeId], ret: TypeId) -> Option<Value> {
        let scalar = |v: &Value| -> Option<Std9> {
            let p = v.as_enum()?;
            Std9::ALL.get(usize::try_from(p).ok()?).copied()
        };
        match name {
            "\"and\"" | "\"or\"" | "\"nand\"" | "\"nor\"" | "\"xor\"" | "\"xnor\"" => {
                let op = name.trim_matches('"');
                if let (Some(a), Some(b)) =
                    (v.first().and_then(&scalar), v.get(1).and_then(&scalar))
                {
                    return Some(std9_value(logic_op(op, a, b)));
                }
                let (a, b) = (v.first()?.to_std9()?, v.get(1)?.to_std9()?);
                if a.len() != b.len() {
                    return None;
                }
                let out: Vec<Std9> = a
                    .iter()
                    .zip(&b)
                    .map(|(x, y)| logic_op(op, *x, *y))
                    .collect();
                Some(self.std9_array(&out, ret))
            }
            "\"not\"" => {
                if let Some(a) = v.first().and_then(&scalar) {
                    return Some(std9_value(not9(a)));
                }
                let a = v.first()?.to_std9()?;
                let out: Vec<Std9> = a.iter().map(|x| not9(*x)).collect();
                Some(self.std9_array(&out, ret))
            }
            "to_bit" => {
                let a = scalar(v.first()?)?;
                Some(Value::Enum(u32::from(a.to_bit() == Bit::One)))
            }
            "to_stdulogic" => {
                let p = v.first()?.as_enum()?;
                let s = if p == 0 { Std9::Zero } else { Std9::One };
                Some(std9_value(s))
            }
            "to_bitvector" | "to_bit_vector" => {
                let a = v.first()?.to_std9()?;
                let elems: Vec<Value> = a
                    .iter()
                    .map(|s| Value::Enum(u32::from(s.to_bit() == Bit::One)))
                    .collect();
                Some(self.reindex(elems, ret))
            }
            "to_stdulogicvector"
            | "to_stdlogicvector"
            | "to_std_ulogic_vector"
            | "to_std_logic_vector" => {
                let a = v.first()?.as_array()?;
                let out: Vec<Std9> = a
                    .elems
                    .iter()
                    .map(|e| match e.as_enum() {
                        Some(0) => Some(Std9::Zero),
                        Some(1) => Some(Std9::One),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(self.std9_array(&out, ret))
            }
            "to_x01" | "to_x01z" | "to_ux01" | "to_01" => {
                let map = |s: Std9| -> Std9 {
                    match (name, s) {
                        (_, Std9::Zero | Std9::L) => Std9::Zero,
                        (_, Std9::One | Std9::H) => Std9::One,
                        ("to_x01z", Std9::Z) => Std9::Z,
                        ("to_ux01", Std9::U) => Std9::U,
                        ("to_01", _) => Std9::Zero,
                        _ => Std9::X,
                    }
                };
                if let Some(a) = v.first().and_then(&scalar) {
                    return Some(std9_value(map(a)));
                }
                let a = v.first()?.to_std9()?;
                let out: Vec<Std9> = a.iter().map(|s| map(*s)).collect();
                Some(self.std9_array(&out, ret))
            }
            "is_x" => {
                let known = |s: Std9| matches!(s, Std9::Zero | Std9::One | Std9::L | Std9::H);
                if let Some(a) = v.first().and_then(&scalar) {
                    return Some(Value::from_bool(!known(a)));
                }
                let a = v.first()?.to_std9()?;
                Some(Value::from_bool(!a.iter().all(|s| known(*s))))
            }
            "rising_edge" | "falling_edge" => None,
            "to_string" | "to_hstring" | "to_ostring" | "to_bstring" => {
                let t = arg_tys.first().copied()?;
                let text = self.render_std9(v.first()?, t, name)?;
                Some(Value::string(text.chars().map(|c| c as u32)))
            }
            _ => None,
        }
    }

    fn render_std9(&self, v: &Value, _ty: TypeId, name: &str) -> Option<String> {
        let elems = v.to_std9()?;
        let bits: String = elems.iter().map(|s| s.to_char()).collect();
        Some(match name {
            "to_hstring" => group_digits(&elems, 4),
            "to_ostring" => group_digits(&elems, 3),
            _ => bits,
        })
    }

    /// Builds an array value of nine-state elements with the index bounds
    /// of `ty` when it is constrained, else `n-1 downto 0`.
    fn std9_array(&self, elems: &[Std9], ty: TypeId) -> Value {
        let vals: Vec<Value> = elems
            .iter()
            .map(|s| Value::Enum(u32::try_from(s.index()).unwrap_or(0)))
            .collect();
        self.reindex(vals, ty)
    }

    /// Gives `elems` the index bounds of `ty`, or `n-1 downto 0`.
    fn reindex(&self, elems: Vec<Value>, ty: TypeId) -> Value {
        let n = i128::try_from(elems.len()).unwrap_or(0);
        let (left, dir) = match self.a.index_constraint(ty).and_then(|c| c.first().cloned()) {
            Some(b) if b.length() == Some(n) => (b.left.int().unwrap_or(n - 1), b.dir),
            _ => (n - 1, crate::vhdl::ast::Direction::Downto),
        };
        Value::Array(ArrayValue { left, dir, elems })
    }

    /// `numeric_std`: the arithmetic that matters for static
    /// expressions. `unsigned` and `signed` values are nine-state arrays;
    /// they are converted to [`Logic`] (MSB leftmost) for the arithmetic
    /// and back.
    fn fold_numeric_std(
        &self,
        name: &str,
        v: &[Value],
        arg_tys: &[TypeId],
        ret: TypeId,
    ) -> Option<Value> {
        let signed_of = |t: TypeId| -> bool { self.is_signed_type(t) };
        let to_logic = |v: &Value, t: TypeId| -> Option<Logic> {
            let l = v.to_logic(true)?;
            Some(if signed_of(t) { l.as_signed() } else { l })
        };
        let out_len = |t: TypeId| -> Option<u32> {
            self.a.array_length(t).and_then(|n| u32::try_from(n).ok())
        };
        match name {
            "to_integer" => {
                let t = arg_tys.first().copied()?;
                let l = to_logic(v.first()?, t)?;
                if !l.is_fully_known() {
                    return None;
                }
                Some(Value::Int(i128::from(if signed_of(t) {
                    l.to_i64()?
                } else {
                    i64::try_from(l.to_u64()?).ok()?
                })))
            }
            "to_unsigned" | "to_signed" => {
                let n = v.get(1)?.as_int()?;
                let width = u32::try_from(n).ok()?;
                let x = v.first()?.as_int()?;
                let l = if name == "to_signed" {
                    Logic::from_i64(i64::try_from(x).ok()?, width).as_signed()
                } else {
                    Logic::from_u64(u64::try_from(x).ok()?, width)
                };
                Some(Value::from_logic(&l, true))
            }
            "resize" => {
                let t = arg_tys.first().copied()?;
                let l = to_logic(v.first()?, t)?;
                let width = match v.get(1) {
                    Some(Value::Int(n)) => u32::try_from(*n).ok()?,
                    _ => out_len(ret)?,
                };
                Some(Value::from_logic(&l.resize(width), true))
            }
            "\"+\"" | "\"-\"" | "\"*\"" | "\"/\"" | "mod" | "\"mod\"" | "rem" | "\"rem\"" => {
                let (lt, rt) = (arg_tys.first().copied()?, arg_tys.get(1).copied()?);
                let signed = signed_of(lt) || signed_of(rt);
                let width = out_len(ret).or_else(|| {
                    let a = self.a.array_length(lt).and_then(|n| u32::try_from(n).ok());
                    let b = self.a.array_length(rt).and_then(|n| u32::try_from(n).ok());
                    match (a, b) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        _ => None,
                    }
                })?;
                let conv = |x: &Value, t: TypeId| -> Option<Logic> {
                    let l = if self.a.class(t) == TypeClass::Array {
                        to_logic(x, t)?
                    } else {
                        let n = x.as_int()?;
                        if signed {
                            Logic::from_i64(i64::try_from(n).ok()?, width).as_signed()
                        } else {
                            Logic::from_u64(u64::try_from(n).ok()?, width)
                        }
                    };
                    Some(l.resize(width).with_signed(signed))
                };
                let a = conv(v.first()?, lt)?;
                let b = conv(v.get(1)?, rt)?;
                let r = match name.trim_matches('"') {
                    "+" => a.add(&b),
                    "-" => a.sub(&b),
                    "*" => a.mul(&b).resize(width),
                    "/" => a.div(&b),
                    "mod" => a.rem(&b),
                    "rem" => a.rem(&b),
                    _ => return None,
                };
                Some(Value::from_logic(&r.resize(width), true))
            }
            "\"=\"" | "\"/=\"" | "\"<\"" | "\"<=\"" | "\">\"" | "\">=\"" => {
                let (lt, rt) = (arg_tys.first().copied()?, arg_tys.get(1).copied()?);
                let signed = signed_of(lt) || signed_of(rt);
                let width = {
                    let a = self.a.array_length(lt).and_then(|n| u32::try_from(n).ok());
                    let b = self.a.array_length(rt).and_then(|n| u32::try_from(n).ok());
                    a.unwrap_or(64).max(b.unwrap_or(64)) + 1
                };
                let conv = |x: &Value, t: TypeId| -> Option<Logic> {
                    let l = if self.a.class(t) == TypeClass::Array {
                        to_logic(x, t)?
                    } else {
                        let n = x.as_int()?;
                        Logic::from_i64(i64::try_from(n).ok()?, width)
                    };
                    Some(l.resize(width).with_signed(signed))
                };
                let a = conv(v.first()?, lt)?;
                let b = conv(v.get(1)?, rt)?;
                let r = match name.trim_matches('"') {
                    "=" => a.eq(&b),
                    "/=" => a.ne(&b),
                    "<" => a.lt(&b),
                    "<=" => a.le(&b),
                    ">" => a.gt(&b),
                    ">=" => a.ge(&b),
                    _ => return None,
                };
                Some(Value::from_bool(r.bit(0) == Bit::One))
            }
            "shift_left" | "shift_right" | "rotate_left" | "rotate_right" | "sll" | "srl" => {
                let t = arg_tys.first().copied()?;
                let l = to_logic(v.first()?, t)?;
                let n = u32::try_from(v.get(1)?.as_int()?).ok()?;
                let w = l.width();
                let r = match name {
                    "shift_left" | "sll" => l.shl(n),
                    "shift_right" | "srl" => {
                        if signed_of(t) {
                            l.sshr(n)
                        } else {
                            l.shr(n)
                        }
                    }
                    "rotate_left" => {
                        if w == 0 {
                            l
                        } else {
                            let n = n % w;
                            l.shl(n).or(&l.shr(w - n))
                        }
                    }
                    _ => {
                        if w == 0 {
                            l
                        } else {
                            let n = n % w;
                            l.shr(n).or(&l.shl(w - n))
                        }
                    }
                };
                Some(Value::from_logic(&r.resize(w), true))
            }
            "minimum" | "maximum" => {
                let (lt, rt) = (arg_tys.first().copied()?, arg_tys.get(1).copied()?);
                let a = to_logic(v.first()?, lt)?;
                let b = to_logic(v.get(1)?, rt)?;
                let less = a.lt(&b).bit(0) == Bit::One;
                let pick = if (name == "minimum") == less {
                    v.first()?
                } else {
                    v.get(1)?
                };
                Some(pick.clone())
            }
            "find_leftmost" | "find_rightmost" => {
                let t = arg_tys.first().copied()?;
                let elems = v.first()?.to_std9()?;
                let want = v.get(1)?.as_enum()?;
                let want = Std9::ALL.get(usize::try_from(want).ok()?).copied()?;
                let bounds = self.a.index_constraint(t)?.first().cloned()?;
                let left = bounds.left.int()?;
                let idx = if name == "find_leftmost" {
                    elems.iter().position(|s| *s == want)
                } else {
                    elems.iter().rposition(|s| *s == want)
                };
                let pos = match idx {
                    Some(i) => {
                        let i = i128::try_from(i).ok()?;
                        match bounds.dir {
                            crate::vhdl::ast::Direction::To => left + i,
                            crate::vhdl::ast::Direction::Downto => left - i,
                        }
                    }
                    None => -1,
                };
                Some(Value::Int(pos))
            }
            "to_01" => {
                let a = v.first()?.to_std9()?;
                let out: Vec<Std9> = a
                    .iter()
                    .map(|s| match s {
                        Std9::One | Std9::H => Std9::One,
                        _ => Std9::Zero,
                    })
                    .collect();
                Some(self.std9_array(&out, ret))
            }
            _ => None,
        }
    }

    /// True for `signed` and its subtypes (by declared name, since the
    /// two `numeric_std` types differ only by name).
    fn is_signed_type(&self, t: TypeId) -> bool {
        let base = self.a.base_type(t);
        let mut cur = t;
        loop {
            if let Some(n) = self.a.ty(cur).name
                && self.a.name(n).eq_ignore_ascii_case("signed")
            {
                return true;
            }
            match self.a.ty(cur).kind {
                super::TypeKind::Subtype { parent, .. } => cur = parent,
                _ => break,
            }
        }
        self.a
            .ty(base)
            .name
            .is_some_and(|n| self.a.name(n).eq_ignore_ascii_case("signed"))
    }
}

fn std9_value(s: Std9) -> Value {
    Value::Enum(u32::try_from(s.index()).unwrap_or(0))
}

fn not9(s: Std9) -> Std9 {
    match s {
        Std9::Zero | Std9::L => Std9::One,
        Std9::One | Std9::H => Std9::Zero,
        Std9::U => Std9::U,
        _ => Std9::X,
    }
}

/// The IEEE 1164 logic tables, via the four-state view with `U`
/// propagation.
fn logic_op(op: &str, a: Std9, b: Std9) -> Std9 {
    if a == Std9::U || b == Std9::U {
        return Std9::U;
    }
    let (x, y) = (a.to_bit(), b.to_bit());
    let known = |v: Bit| v == Bit::Zero || v == Bit::One;
    // The tables short-circuit: `0 and X` is `0`, `1 or X` is `1`.
    let r = match op {
        "and" | "nand" => {
            if x == Bit::Zero || y == Bit::Zero {
                Some(false)
            } else if known(x) && known(y) {
                Some(x == Bit::One && y == Bit::One)
            } else {
                None
            }
        }
        "or" | "nor" => {
            if x == Bit::One || y == Bit::One {
                Some(true)
            } else if known(x) && known(y) {
                Some(x == Bit::One || y == Bit::One)
            } else {
                None
            }
        }
        "xor" | "xnor" => {
            if known(x) && known(y) {
                Some((x == Bit::One) != (y == Bit::One))
            } else {
                None
            }
        }
        _ => None,
    };
    match r {
        None => Std9::X,
        Some(v) => {
            let v = if matches!(op, "nand" | "nor" | "xnor") {
                !v
            } else {
                v
            };
            if v { Std9::One } else { Std9::Zero }
        }
    }
}

/// Renders nine-state elements as hex or octal digits, `X` for any group
/// containing an unknown, as `to_hstring` does.
fn group_digits(elems: &[Std9], per: usize) -> String {
    let pad = (per - elems.len() % per) % per;
    let mut padded: Vec<Std9> = vec![Std9::Zero; pad];
    padded.extend_from_slice(elems);
    let mut out = String::new();
    for chunk in padded.chunks(per) {
        let mut v = 0u32;
        let mut known = true;
        for s in chunk {
            match s.to_bit() {
                Bit::Zero => v <<= 1,
                Bit::One => v = (v << 1) | 1,
                _ => known = false,
            }
        }
        if known {
            out.push(char::from_digit(v, 16).unwrap_or('0').to_ascii_uppercase());
        } else {
            out.push('X');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logic_tables() {
        assert_eq!(logic_op("and", Std9::Zero, Std9::X), Std9::Zero);
        assert_eq!(logic_op("and", Std9::One, Std9::X), Std9::X);
        assert_eq!(logic_op("or", Std9::One, Std9::X), Std9::One);
        assert_eq!(logic_op("or", Std9::Zero, Std9::Z), Std9::X);
        assert_eq!(logic_op("xor", Std9::One, Std9::H), Std9::Zero);
        assert_eq!(logic_op("nand", Std9::One, Std9::One), Std9::Zero);
        assert_eq!(logic_op("and", Std9::U, Std9::Zero), Std9::U);
        assert_eq!(not9(Std9::L), Std9::One);
        assert_eq!(not9(Std9::Z), Std9::X);
    }

    #[test]
    fn hex_grouping() {
        use Std9::{One, X, Zero};
        assert_eq!(group_digits(&[One, Zero, One, Zero], 4), "A");
        assert_eq!(
            group_digits(&[One, One, One, One, Zero, Zero, Zero, Zero], 4),
            "F0"
        );
        assert_eq!(group_digits(&[One, X, One, Zero], 4), "X");
        // Shorter than one digit: padded on the left with zeroes.
        assert_eq!(group_digits(&[One, One], 4), "3");
    }
}
