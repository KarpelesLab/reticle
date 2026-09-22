//! Static evaluation of the bundled library functions.
//!
//! The checker does not interpret subprogram bodies, but a locally static
//! expression (clause 9.4.2) may call one: a design that writes
//! `constant one : unsigned(7 downto 0) := to_unsigned(1, 8);` expects the
//! value to be known at analysis time.
//!
//! `Checker::builtin_call` therefore recognises a call whose target is
//! declared in one of the bundled packages and computes the result
//! natively from the arguments' [`Value`]s, using [`Logic`] for the
//! vector arithmetic so the semantics match what the simulator will do.
//! Anything it does not recognise, or any call with a non-static
//! argument, simply yields no value, which makes the expression
//! non-static and leaves it to elaboration.
//!
//! # The arithmetic packages
//!
//! `ieee.numeric_std`, `ieee.numeric_bit` and the three Synopsys legacy
//! packages have no VHDL body at all: every subprogram is declared
//! `attribute foreign`, and this file *is* the implementation for the
//! static case, as [`crate::vhdl::elab`]'s `numeric` module is for the
//! rest. One core folds all five, parameterised on two things:
//!
//! - the element encoding, nine-state for `numeric_std` and the Synopsys
//!   packages, two-state for `numeric_bit`;
//! - the signedness, read from the operand and result type names except
//!   in `std_logic_unsigned` and `std_logic_signed`, where the package
//!   itself decides it.
//!
//! Widths come from the values rather than the types, because the formals
//! of these packages are unconstrained: `l'length` is only known once
//! there is an actual.

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
    /// `ieee.numeric_std`.
    NumericStd,
    /// `ieee.numeric_bit`: the same operations over `bit`.
    NumericBit,
    /// The Synopsys `ieee.std_logic_arith`.
    Arith,
    /// The Synopsys `ieee.std_logic_unsigned`, which makes every
    /// `std_logic_vector` operand unsigned.
    SlvUnsigned,
    /// The Synopsys `ieee.std_logic_signed`, which makes every
    /// `std_logic_vector` operand signed.
    SlvSigned,
    /// `ieee.std_logic_1164`.
    StdLogic1164,
    /// `ieee.math_real`.
    MathReal,
    /// `std.standard`.
    Standard,
}

impl Pkg {
    /// True for the packages folded by the arithmetic core.
    fn is_arithmetic(self) -> bool {
        matches!(
            self,
            Pkg::NumericStd | Pkg::NumericBit | Pkg::Arith | Pkg::SlvUnsigned | Pkg::SlvSigned
        )
    }
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
            (ieee, self.syms.numeric_bit, Pkg::NumericBit),
            (ieee, self.syms.std_logic_arith, Pkg::Arith),
            (ieee, self.syms.std_logic_unsigned, Pkg::SlvUnsigned),
            (ieee, self.syms.std_logic_signed, Pkg::SlvSigned),
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
            Pkg::Standard => self.fold_standard(&name, &values, &arg_tys, ret),
            _ if pkg.is_arithmetic() => self.fold_numeric(pkg, &name, &values, &arg_tys, ret),
            _ => None,
        }
    }

    /// `math_real`: every function is `foreign` and evaluated here over
    /// `f64`, which is what `real` lowers to everywhere else.
    ///
    /// `"**"` has an overload whose base is an `integer`, so the first
    /// argument is accepted as either; everything else takes reals.
    fn fold_math_real(&self, name: &str, v: &[Value]) -> Option<Value> {
        let real = |v: &Value| -> Option<f64> {
            v.as_real().or_else(|| {
                v.as_int()
                    .and_then(|n| i32::try_from(n).ok())
                    .map(f64::from)
            })
        };
        let x = real(v.first()?)?;
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
            "arcsinh" => x.asinh(),
            "arccosh" => x.acosh(),
            "arctanh" => x.atanh(),
            "realmax" => x.max(v.get(1)?.as_real()?),
            "realmin" => x.min(v.get(1)?.as_real()?),
            "log" => x.ln() / v.get(1)?.as_real()?.ln(),
            "arctan" => x.atan2(v.get(1)?.as_real()?),
            // `x ** y` over reals, and the integer base overload.
            "\"**\"" => x.powf(v.get(1)?.as_real()?),
            // The real `mod`, which like the integer one takes the sign
            // of the divisor.
            "\"mod\"" => {
                let y = v.get(1)?.as_real()?;
                if y == 0.0 {
                    return None;
                }
                x - y * (x / y).floor()
            }
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

    /// The arithmetic packages: `numeric_std`, `numeric_bit` and the
    /// three Synopsys legacy ones, folded by one core.
    ///
    /// `unsigned` and `signed` values are arrays of single-bit
    /// enumeration literals; they are decoded to [`Logic`] with the MSB
    /// leftmost, operated on there, and encoded back with the element
    /// encoding the result type asks for. Two things are decided before
    /// anything else happens:
    ///
    /// - the **encoding**, nine-state for `numeric_std` and the Synopsys
    ///   packages, two-state for `numeric_bit`, taken from the first
    ///   operand or result whose values are single bits (so `to_string`,
    ///   which returns a `string`, still reads its operand's encoding);
    /// - the **signedness**, from the operand and result type names,
    ///   except for `std_logic_unsigned` and `std_logic_signed` where it
    ///   is the package itself that says so.
    ///
    /// Widths come from the *values*, not the types: the formals of these
    /// packages are unconstrained, so `l'length` is only known once an
    /// actual is in hand.
    fn fold_numeric(
        &self,
        pkg: Pkg,
        name: &str,
        v: &[Value],
        arg_tys: &[TypeId],
        ret: TypeId,
    ) -> Option<Value> {
        let sym = name.trim_matches('"');
        let is_vec = |t: TypeId| self.a.class(t) == TypeClass::Array;
        // A type's single-bit element: itself when it is a scalar.
        let elem_of = |t: TypeId| {
            if is_vec(t) {
                self.a.element_type(t)
            } else {
                Some(t)
            }
        };
        let encoding = |t: TypeId| match elem_of(t) {
            Some(e) if self.a.is_std_ulogic(e) => Some(Enc::Std9),
            Some(e) if self.a.same_base(e, self.a.builtins.bit) => Some(Enc::Bit),
            _ => None,
        };
        let enc = arg_tys
            .iter()
            .chain(std::iter::once(&ret))
            .find_map(|t| encoding(*t))
            .unwrap_or(Enc::Std9);
        let signed = match pkg {
            Pkg::SlvUnsigned => false,
            Pkg::SlvSigned => true,
            _ => arg_tys
                .iter()
                .chain(std::iter::once(&ret))
                .any(|t| self.is_signed_type(*t)),
        };
        // The length of operand `i`, when it is a vector value.
        let vlen = |i: usize| -> Option<u32> {
            let a = v.get(i)?.as_array()?;
            u32::try_from(a.elems.len()).ok()
        };
        // Operand `i` as a `Logic` of `width` bits: a vector is decoded
        // and extended, an integer is converted to that length.
        let logic = |i: usize, width: u32| -> Option<Logic> {
            let t = arg_tys.get(i).copied();
            let val = v.get(i)?;
            let l = if t.is_some_and(is_vec) || val.as_array().is_some() {
                decode(val, enc)?.with_signed(signed)
            } else {
                let n = i64::try_from(val.as_int()?).ok()?;
                Logic::from_i64(n, 64).with_signed(true)
            };
            Some(l.resize(width).with_signed(signed))
        };
        // The width a binary operation runs at: the longer vector, or the
        // only vector when the other operand is an integer.
        let common = || -> Option<u32> {
            match (vlen(0), vlen(1)) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            }
        };

        match sym {
            // --- conversions to and from integers ---
            "to_integer" | "conv_integer" => {
                let val = v.first()?;
                let src = arg_tys.first().copied();
                if src.is_some_and(|t| !is_vec(t)) && val.as_array().is_none() {
                    // `conv_integer(integer)` and `conv_integer(std_ulogic)`.
                    return match val {
                        Value::Int(n) => Some(Value::Int(*n)),
                        Value::Enum(_) => {
                            let s = decode_scalar(val, enc)?;
                            Some(Value::Int(i128::from(s.to_bit() == Bit::One)))
                        }
                        _ => None,
                    };
                }
                let l = decode(val, enc)?.with_signed(signed);
                if !l.is_fully_known() {
                    return None;
                }
                Some(Value::Int(i128::from(if signed {
                    l.to_i64()?
                } else {
                    i64::try_from(l.to_u64()?).ok()?
                })))
            }
            "to_unsigned" | "to_signed" => {
                let width = self.size_argument(v, arg_tys, 1)?;
                let x = i64::try_from(v.first()?.as_int()?).ok()?;
                let l = Logic::from_i64(x, 64).with_signed(true).resize(width);
                Some(encode(&l.with_signed(sym == "to_signed"), enc))
            }
            "conv_unsigned" | "conv_signed" | "conv_std_logic_vector" | "ext" | "sxt" => {
                let width = self.size_argument(v, arg_tys, 1)?;
                // How the source extends: `ext` zero fills and `sxt` sign
                // fills whatever the operand; otherwise an integer or a
                // `signed` operand carries its sign.
                let src_signed = match sym {
                    "ext" => false,
                    "sxt" => true,
                    _ => arg_tys
                        .first()
                        .copied()
                        .is_some_and(|t| !is_vec(t) || self.is_signed_type(t)),
                };
                let val = v.first()?;
                let l = match val {
                    Value::Int(n) => Logic::from_i64(i64::try_from(*n).ok()?, 64).as_signed(),
                    Value::Enum(_) => {
                        let s = decode_scalar(val, enc)?;
                        Logic::from_std9(&[s])
                    }
                    _ => decode(val, enc)?,
                };
                let l = l.with_signed(src_signed).resize(width);
                Some(encode(&l.with_signed(signed), enc))
            }
            "resize" => {
                let width = self.size_argument(v, arg_tys, 1)?;
                let l = decode(v.first()?, enc)?.with_signed(signed);
                // Narrowing a `signed` keeps the sign bit and drops the
                // bits under it, which is not a plain truncation.
                let out = if signed && width >= 1 && width < l.width() {
                    let sign = l.slice(l.width() - 1, l.width() - 1);
                    if width == 1 {
                        sign
                    } else {
                        sign.concat(&l.slice(width - 2, 0))
                    }
                } else {
                    l.resize(width)
                };
                Some(encode(&out.with_signed(signed), enc))
            }

            // --- arithmetic ---
            "+" | "-" | "abs" if v.len() == 1 => {
                let l = decode(v.first()?, enc)?.with_signed(signed);
                let out = match sym {
                    "+" => l,
                    "-" => l.neg(),
                    _ => {
                        if l.is_negative() {
                            l.neg()
                        } else {
                            l
                        }
                    }
                };
                Some(encode(&out.with_signed(signed), enc))
            }
            "+" | "-" | "*" | "/" | "rem" | "mod" => {
                let base = common()?;
                let width = if sym == "*" {
                    match (vlen(0), vlen(1)) {
                        (Some(a), Some(b)) => a + b,
                        (Some(a), None) | (None, Some(a)) => a.saturating_mul(2),
                        (None, None) => return None,
                    }
                } else {
                    base
                };
                let a = logic(0, width)?;
                let b = logic(1, width)?;
                let out = match sym {
                    "+" => a.add(&b),
                    "-" => a.sub(&b),
                    "*" => a.mul(&b),
                    "/" => a.div(&b),
                    // The IR and `Logic` give the remainder the sign of
                    // the dividend, which is VHDL's `rem`; `mod` takes the
                    // divisor's sign instead.
                    "rem" => a.rem(&b),
                    _ => vhdl_mod(&a, &b, signed),
                };
                Some(encode(&out.with_signed(signed), enc))
            }

            // --- comparison ---
            "=" | "/=" | "<" | "<=" | ">" | ">=" => {
                let r = self.compare(sym, &logic(0, common()?)?, &logic(1, common()?)?)?;
                Some(Value::from_bool(r))
            }
            "?=" | "?/=" | "?<" | "?<=" | "?>" | "?>=" => {
                let (a, b) = (logic(0, common()?)?, logic(1, common()?)?);
                // A matching operator answers with a logic value, so an
                // unknown anywhere in either operand gives `'X'`.
                if a.has_unknown() || b.has_unknown() {
                    return Some(encode_scalar(Std9::X, enc));
                }
                let r = self.compare(sym.trim_start_matches('?'), &a, &b)?;
                Some(encode_scalar(if r { Std9::One } else { Std9::Zero }, enc))
            }
            "std_match" => {
                let a = decode(v.first()?, enc)?;
                let b = decode(v.get(1)?, enc)?;
                if a.width() != b.width() {
                    return Some(Value::from_bool(false));
                }
                let ea = elements(v.first()?, enc)?;
                let eb = elements(v.get(1)?, enc)?;
                Some(Value::from_bool(
                    ea.iter().zip(&eb).all(|(x, y)| match_element(*x, *y)),
                ))
            }

            // --- shifts and rotates ---
            "shift_left" | "shift_right" | "rotate_left" | "rotate_right" | "sll" | "srl"
            | "rol" | "ror" | "sla" | "sra" | "shl" | "shr" => {
                let l = decode(v.first()?, enc)?.with_signed(signed);
                let w = l.width();
                let n = match v.get(1)? {
                    Value::Int(n) => *n,
                    other => i128::from(decode(other, enc)?.to_u64()?),
                };
                let out = shift(&l, sym, n, signed, w)?;
                Some(encode(&out.resize(w).with_signed(signed), enc))
            }

            // --- extrema and search ---
            "minimum" | "maximum" => {
                let width = common()?;
                let less = self.compare("<", &logic(0, width)?, &logic(1, width)?)?;
                let pick = if (sym == "minimum") == less { 0 } else { 1 };
                match v.get(pick)? {
                    // An integer operand is returned as the vector the
                    // result type calls for.
                    Value::Int(_) => Some(encode(&logic(pick, width)?, enc)),
                    other => Some(other.clone()),
                }
            }
            "find_leftmost" | "find_rightmost" => {
                let t = arg_tys.first().copied()?;
                let elems = elements(v.first()?, enc)?;
                let want = decode_scalar(v.get(1)?, enc)?;
                let bounds = self.a.index_constraint(t).and_then(|c| c.first().cloned());
                // The formal is unconstrained, so the index range is
                // usually the actual's own.
                let (left, dir) = match bounds {
                    Some(b) => (b.left.int().unwrap_or(0), b.dir),
                    None => {
                        let a = v.first()?.as_array()?;
                        (a.left, a.dir)
                    }
                };
                let idx = if sym == "find_leftmost" {
                    elems.iter().position(|s| match_element(*s, want))
                } else {
                    elems.iter().rposition(|s| match_element(*s, want))
                };
                let pos = match idx {
                    Some(i) => {
                        let i = i128::try_from(i).ok()?;
                        match dir {
                            crate::vhdl::ast::Direction::To => left + i,
                            crate::vhdl::ast::Direction::Downto => left - i,
                        }
                    }
                    None => -1,
                };
                Some(Value::Int(pos))
            }

            // --- strength stripping and testing ---
            "to_01" | "to_x01" | "to_x01z" | "to_ux01" => {
                let xmap = v.get(1).and_then(|x| decode_scalar(x, enc));
                let out: Vec<Std9> = elements(v.first()?, enc)?
                    .into_iter()
                    .map(|s| strip(sym, s, xmap))
                    .collect();
                Some(self.encoded_array(&out, enc, ret))
            }
            "is_x" => {
                let elems = elements(v.first()?, enc)?;
                let known = |s: Std9| matches!(s, Std9::Zero | Std9::One | Std9::L | Std9::H);
                Some(Value::from_bool(!elems.iter().all(|s| known(*s))))
            }

            // --- element-wise logic ---
            "not" => {
                if v.first()?.as_array().is_none() {
                    return Some(encode_scalar(not9(decode_scalar(v.first()?, enc)?), enc));
                }
                let out: Vec<Std9> = elements(v.first()?, enc)?.into_iter().map(not9).collect();
                Some(self.encoded_array(&out, enc, ret))
            }
            "and" | "or" | "nand" | "nor" | "xor" | "xnor" => {
                self.fold_elementwise(sym, v, enc, ret)
            }

            // --- rendering ---
            "to_string" | "to_bstring" | "to_ostring" | "to_hstring" => {
                let elems = elements(v.first()?, enc)?;
                let text = match sym {
                    "to_hstring" => group_digits(&elems, 4),
                    "to_ostring" => group_digits(&elems, 3),
                    _ => elems.iter().map(|s| s.to_char()).collect(),
                };
                Some(Value::string(text.chars().map(|c| c as u32)))
            }
            _ => None,
        }
    }

    /// The `size` argument of a conversion: an integer, or the length of
    /// the `size_res` vector the VHDL-2008 overloads take.
    fn size_argument(&self, v: &[Value], arg_tys: &[TypeId], i: usize) -> Option<u32> {
        match v.get(i)? {
            Value::Int(n) => u32::try_from(*n).ok(),
            other => {
                let _ = arg_tys;
                u32::try_from(other.as_array()?.elems.len()).ok()
            }
        }
    }

    /// One comparison of two equally wide values, `None` when either has
    /// an unknown bit.
    fn compare(&self, sym: &str, a: &Logic, b: &Logic) -> Option<bool> {
        let r = match sym {
            "=" => a.eq(b),
            "/=" => a.ne(b),
            "<" => a.lt(b),
            "<=" => a.le(b),
            ">" => a.gt(b),
            ">=" => a.ge(b),
            _ => return None,
        };
        match r.bit(0) {
            Bit::One => Some(true),
            Bit::Zero => Some(false),
            _ => None,
        }
    }

    /// The element-wise logical operators, which also accept one scalar
    /// operand (VHDL-2008) and a single vector (the reductions).
    fn fold_elementwise(&self, op: &str, v: &[Value], enc: Enc, ret: TypeId) -> Option<Value> {
        let Some(second) = v.get(1) else {
            // A reduction: fold the whole vector into one element.
            let elems = elements(v.first()?, enc)?;
            let base = match op {
                "and" | "nand" => Std9::One,
                _ => Std9::Zero,
            };
            let core = match op {
                "nand" => "and",
                "nor" => "or",
                "xnor" => "xor",
                other => other,
            };
            let mut acc = base;
            for e in elems {
                acc = logic_op(core, acc, e);
            }
            if matches!(op, "nand" | "nor" | "xnor") {
                acc = not9(acc);
            }
            return Some(encode_scalar(acc, enc));
        };
        let first = v.first()?;
        let (a, b) = (first.as_array(), second.as_array());
        let out: Vec<Std9> = match (a, b) {
            (Some(_), Some(_)) => {
                let (x, y) = (elements(first, enc)?, elements(second, enc)?);
                if x.len() != y.len() {
                    return None;
                }
                x.iter()
                    .zip(&y)
                    .map(|(p, q)| logic_op(op, *p, *q))
                    .collect()
            }
            (Some(_), None) => {
                let s = decode_scalar(second, enc)?;
                elements(first, enc)?
                    .into_iter()
                    .map(|p| logic_op(op, p, s))
                    .collect()
            }
            (None, Some(_)) => {
                let s = decode_scalar(first, enc)?;
                elements(second, enc)?
                    .into_iter()
                    .map(|q| logic_op(op, s, q))
                    .collect()
            }
            (None, None) => {
                let (s, t) = (decode_scalar(first, enc)?, decode_scalar(second, enc)?);
                return Some(encode_scalar(logic_op(op, s, t), enc));
            }
        };
        Some(self.encoded_array(&out, enc, ret))
    }

    /// Builds an array value with the index bounds of `ty` when it is
    /// constrained, else `n-1 downto 0`.
    fn encoded_array(&self, elems: &[Std9], enc: Enc, ty: TypeId) -> Value {
        let vals: Vec<Value> = elems.iter().map(|s| enc.encode(*s)).collect();
        self.reindex(vals, ty)
    }

    /// True for `signed` and its subtypes.
    ///
    /// The declared name is the only thing that separates `signed` from
    /// `unsigned`: the two are declared side by side as arrays of the
    /// same element type. The whole subtype chain is walked because
    /// VHDL-2008 declares `signed` as a resolved subtype of
    /// `unresolved_signed`, and a design's own
    /// `subtype word is signed(15 downto 0)` adds a further link.
    fn is_signed_type(&self, t: TypeId) -> bool {
        let named = |ty: TypeId| {
            self.a.ty(ty).name.is_some_and(|n| {
                let s = self.a.name(n);
                s.eq_ignore_ascii_case("signed") || s.eq_ignore_ascii_case("unresolved_signed")
            })
        };
        let mut cur = t;
        loop {
            if named(cur) {
                return true;
            }
            match self.a.ty(cur).kind {
                super::TypeKind::Subtype { parent, .. } => cur = parent,
                _ => return false,
            }
        }
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

/// How the single-bit elements of a vector value are encoded.
///
/// The arithmetic packages come in two flavours over the same operations:
/// `numeric_std` and the Synopsys ones hold `std_ulogic`, `numeric_bit`
/// holds `bit`. One enumeration position means different things in the
/// two, so every decode and encode goes through this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Enc {
    /// `std_ulogic` positions, in the IEEE 1164 order `U X 0 1 Z W L H -`.
    Std9,
    /// `bit` positions: `'0'` is 0 and `'1'` is 1.
    Bit,
}

impl Enc {
    /// The nine-state value at enumeration position `p`.
    fn decode(self, p: u32) -> Option<Std9> {
        match self {
            Enc::Std9 => Std9::ALL.get(usize::try_from(p).ok()?).copied(),
            Enc::Bit => match p {
                0 => Some(Std9::Zero),
                1 => Some(Std9::One),
                _ => None,
            },
        }
    }

    /// The enumeration position of `s`, as a value of the element type.
    fn encode(self, s: Std9) -> Value {
        match self {
            Enc::Std9 => Value::Enum(u32::try_from(s.index()).unwrap_or(0)),
            Enc::Bit => Value::Enum(u32::from(s.to_bit() == Bit::One)),
        }
    }
}

/// The elements of a vector value, leftmost first.
fn elements(v: &Value, enc: Enc) -> Option<Vec<Std9>> {
    let a = v.as_array()?;
    a.elems.iter().map(|e| enc.decode(e.as_enum()?)).collect()
}

/// A vector value as a [`Logic`], the leftmost element the most
/// significant bit.
fn decode(v: &Value, enc: Enc) -> Option<Logic> {
    let mut e = elements(v, enc)?;
    e.reverse();
    Some(Logic::from_std9(&e))
}

/// A [`Logic`] as a `downto` array value with left bound `width - 1`.
fn encode(l: &Logic, enc: Enc) -> Value {
    let elems: Vec<Value> = (0..l.width())
        .rev()
        .map(|i| enc.encode(Std9::from_bit(l.bit(i))))
        .collect();
    let n = i128::try_from(elems.len()).unwrap_or(0);
    Value::Array(ArrayValue {
        left: n - 1,
        dir: crate::vhdl::ast::Direction::Downto,
        elems,
    })
}

/// One element value as a nine-state value.
fn decode_scalar(v: &Value, enc: Enc) -> Option<Std9> {
    enc.decode(v.as_enum()?)
}

/// One nine-state value as an element value.
fn encode_scalar(s: Std9, enc: Enc) -> Value {
    enc.encode(s)
}

/// The strength strippers, with `to_01`'s explicit map for the values
/// that are neither high nor low.
fn strip(name: &str, s: Std9, xmap: Option<Std9>) -> Std9 {
    match (name, s) {
        (_, Std9::Zero | Std9::L) => Std9::Zero,
        (_, Std9::One | Std9::H) => Std9::One,
        ("to_x01z", Std9::Z) => Std9::Z,
        ("to_ux01", Std9::U) => Std9::U,
        ("to_01", _) => xmap.unwrap_or(Std9::Zero),
        _ => Std9::X,
    }
}

/// `std_match`'s element test: a don't-care on either side matches
/// anything, and the rest compare after their strength is stripped, so
/// `'H'` matches `'1'`.
fn match_element(a: Std9, b: Std9) -> bool {
    if a == Std9::DontCare || b == Std9::DontCare {
        return true;
    }
    strip("to_x01", a, None) == strip("to_x01", b, None)
}

/// One shift or rotate of `l` by `n` places, in `w` bits.
///
/// The operators of the `sll` family take an `integer` and reverse
/// direction for a negative count; the named functions take a `natural`
/// and never see one. A right shift of a `signed` fills with the sign
/// bit, except `srl`, which the package defines as a logical shift
/// whatever the operand.
fn shift(l: &Logic, name: &str, n: i128, signed: bool, w: u32) -> Option<Logic> {
    let left_named = matches!(
        name,
        "shift_left" | "sla" | "sll" | "shl" | "rotate_left" | "rol"
    );
    let (left, n) = if n < 0 {
        (!left_named, -n)
    } else {
        (left_named, n)
    };
    if matches!(name, "rotate_left" | "rotate_right" | "rol" | "ror") {
        if w == 0 {
            return Some(l.clone());
        }
        let k = u32::try_from(n.rem_euclid(i128::from(w))).ok()?;
        let k = if left { k } else { (w - k) % w };
        if k == 0 {
            return Some(l.clone());
        }
        return Some(l.shl(k).or(&l.shr(w - k)));
    }
    let k = u32::try_from(n.min(i128::from(w))).ok()?;
    let arithmetic = signed && name != "srl";
    Some(if left {
        l.shl(k)
    } else if arithmetic {
        l.sshr(k)
    } else {
        l.shr(k)
    })
}

/// VHDL's `mod`, whose result takes the sign of the divisor, built on the
/// remainder, which takes the sign of the dividend.
fn vhdl_mod(a: &Logic, b: &Logic, signed: bool) -> Logic {
    let r = a.rem(b);
    if !signed || r.has_unknown() || r.is_zero() {
        return r;
    }
    if r.is_negative() != b.is_negative() {
        r.add(b)
    } else {
        r
    }
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

    /// A signed `Logic` of `width` bits, as the arithmetic core builds
    /// its operands.
    fn s(value: i64, width: u32) -> Logic {
        Logic::from_i64(value, width).as_signed()
    }

    /// The unsigned equivalent.
    fn u(value: u64, width: u32) -> Logic {
        Logic::from_u64(value, width)
    }

    #[test]
    fn encodings_round_trip() {
        // `'1'` is position 3 in the nine-state set and position 1 in
        // `bit`, so the two encodings disagree on every literal.
        assert_eq!(Enc::Std9.encode(Std9::One), Value::Enum(3));
        assert_eq!(Enc::Bit.encode(Std9::One), Value::Enum(1));
        assert_eq!(Enc::Std9.decode(3), Some(Std9::One));
        assert_eq!(Enc::Bit.decode(1), Some(Std9::One));
        assert_eq!(Enc::Bit.decode(3), None);
        // An unknown has no `bit`, so it encodes as `'0'`.
        assert_eq!(Enc::Std9.encode(Std9::X), Value::Enum(1));
        assert_eq!(Enc::Bit.encode(Std9::X), Value::Enum(0));

        for enc in [Enc::Std9, Enc::Bit] {
            let v = encode(&u(0b1100_1000, 8), enc);
            assert_eq!(decode(&v, enc).unwrap().to_u64(), Some(200));
            let a = v.as_array().unwrap();
            assert_eq!(a.left, 7);
            assert_eq!(a.elems.len(), 8);
        }
    }

    /// `rem` takes the sign of the dividend and `mod` the sign of the
    /// divisor, which is the pair most often got wrong.
    #[test]
    fn remainder_and_modulo_signs() {
        let cases = [
            (7i64, 3i64, 1i64, 1i64),
            (-7, 3, -1, 2),
            (7, -3, 1, -2),
            (-7, -3, -1, -1),
            (-6, 3, 0, 0),
        ];
        for (a, b, rem, modulo) in cases {
            let (x, y) = (s(a, 8), s(b, 8));
            assert_eq!(x.rem(&y).to_i64(), Some(rem), "{a} rem {b}");
            assert_eq!(vhdl_mod(&x, &y, true).to_i64(), Some(modulo), "{a} mod {b}");
        }
        // Unsigned: both are the plain remainder.
        let (x, y) = (u(200, 8), u(30, 8));
        assert_eq!(x.rem(&y).to_u64(), Some(20));
        assert_eq!(vhdl_mod(&x, &y, false).to_u64(), Some(20));
        // Division by zero yields unknown bits rather than a panic.
        assert!(vhdl_mod(&s(7, 8), &s(0, 8), true).has_unknown());
    }

    #[test]
    fn shifts_fill_the_right_way() {
        // Right of a signed value fills with the sign bit.
        assert_eq!(
            shift(&s(-8, 8), "shift_right", 1, true, 8)
                .unwrap()
                .to_i64(),
            Some(-4)
        );
        // `srl` is a logical shift even for a signed operand.
        assert_eq!(
            shift(&s(-8, 8), "srl", 1, true, 8).unwrap().to_u64(),
            Some(0b0111_1100)
        );
        // An unsigned right shift never fills with ones.
        assert_eq!(
            shift(&u(200, 8), "shift_right", 4, false, 8)
                .unwrap()
                .to_u64(),
            Some(12)
        );
        // A left shift drops what runs off the top.
        assert_eq!(
            shift(&u(200, 8), "shift_left", 1, false, 8)
                .unwrap()
                .to_u64(),
            Some(144)
        );
        // Shifting by the whole width empties the value.
        assert_eq!(
            shift(&u(200, 8), "shift_left", 8, false, 8)
                .unwrap()
                .to_u64(),
            Some(0)
        );
        // A negative count on an operator reverses the direction.
        assert_eq!(
            shift(&u(200, 8), "sll", -4, false, 8).unwrap().to_u64(),
            Some(12)
        );
        // Rotates keep every bit, and by the width are the identity.
        assert_eq!(
            shift(&u(0b1100_1000, 8), "rotate_left", 3, false, 8)
                .unwrap()
                .to_u64(),
            Some(0b0100_0110)
        );
        assert_eq!(
            shift(&u(0b1100_1000, 8), "rotate_right", 3, false, 8)
                .unwrap()
                .to_u64(),
            Some(0b0001_1001)
        );
        assert_eq!(
            shift(&u(0b1100_1000, 8), "rotate_left", 8, false, 8)
                .unwrap()
                .to_u64(),
            Some(0b1100_1000)
        );
    }

    #[test]
    fn matching_and_stripping() {
        // A don't-care on either side matches anything.
        assert!(match_element(Std9::DontCare, Std9::One));
        assert!(match_element(Std9::Zero, Std9::DontCare));
        // The weak levels match the forcing ones.
        assert!(match_element(Std9::H, Std9::One));
        assert!(match_element(Std9::L, Std9::Zero));
        assert!(!match_element(Std9::Zero, Std9::One));
        // Anything unknown matches only another unknown.
        assert!(match_element(Std9::U, Std9::X));
        assert!(!match_element(Std9::X, Std9::One));

        assert_eq!(strip("to_x01", Std9::Z, None), Std9::X);
        assert_eq!(strip("to_x01z", Std9::Z, None), Std9::Z);
        assert_eq!(strip("to_ux01", Std9::U, None), Std9::U);
        assert_eq!(strip("to_x01", Std9::U, None), Std9::X);
        assert_eq!(strip("to_01", Std9::Z, None), Std9::Zero);
        assert_eq!(strip("to_01", Std9::Z, Some(Std9::One)), Std9::One);
        assert_eq!(strip("to_01", Std9::H, Some(Std9::One)), Std9::One);
    }

    /// The width rules: `+` runs at the longer operand's length and `*`
    /// at the sum of the two, with each operand extended by its own
    /// signedness first.
    #[test]
    fn width_and_extension_rules() {
        // Unsigned extension is with zeroes: 200 stays 200 in 16 bits.
        assert_eq!(u(200, 8).resize(16).to_u64(), Some(200));
        // Signed extension keeps the value: -7 stays -7.
        assert_eq!(s(-7, 8).resize(16).to_i64(), Some(-7));
        // The 8-by-8 product needs all 16 bits.
        let p = u(200, 16).mul(&u(100, 16));
        assert_eq!(p.to_u64(), Some(20_000));
        let q = s(-7, 16).mul(&s(3, 16));
        assert_eq!(q.to_i64(), Some(-21));
        // The same product in 8 bits wraps, which is what an 8-bit
        // target asks for.
        assert_eq!(u(200, 8).mul(&u(100, 8)).to_u64(), Some(20_000 % 256));
    }
}
