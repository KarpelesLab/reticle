//! The run-time string functions the VHDL frontend lowers to.
//!
//! A VHDL `report` builds its message at run time far more often than not:
//! `"n = " & integer'image(n)`, `to_string(v)`, `to_hstring(addr)`. The IR
//! has a string type and the simulator a string value, so the frontend
//! lowers each of these to an [`crate::ir::ExprKind::Call`] of one of the
//! functions below, and the simulator evaluates it here. Nothing else
//! calls them; they are named with a `$vhdl_` prefix so they cannot meet a
//! Verilog system function.
//!
//! | Function | Arguments | Result |
//! |----------|-----------|--------|
//! | `$vhdl_image_int` | an integer | its decimal image, `-12` |
//! | `$vhdl_image_real` | a real | `2.500000e+00`, see [`crate::logic::vhdl_real_image`] |
//! | `$vhdl_image_time` | a time in femtoseconds | `5000 ps`, see below |
//! | `$vhdl_image_enum` | a position, then one string per literal | the literal at that position |
//! | `$vhdl_to_string` | a vector of single-bit elements | one character per element, `10X1` |
//! | `$vhdl_to_hstring` | the same, then 1 when it is signed | hexadecimal digits, `A6` |
//! | `$vhdl_to_ostring` | the same | octal digits |
//! | `$vhdl_concat` | strings and characters | their concatenation |
//! | `$vhdl_streq` | two strings | one bit, 1 when they are equal |
//!
//! **Time.** IEEE 1076-2008 clause 16.2.2 has `time'image` write the value
//! in the primary unit, which would be `5000000 fs`. Vivado 2026.1's xsim
//! writes `5000 ps` — its resolution limit is a picosecond — and the forms
//! here were measured against it, so a time that is a whole number of
//! picoseconds is written in `ps` and anything finer falls back to `fs`,
//! which xsim cannot represent at all.
//!
//! **Bits.** The simulator is four-state, so a `std_logic` element is one
//! of `0`, `1`, `X` and `Z`: a `'U'` prints as `X`, and `'H'`, `'L'`,
//! `'W'` and `'-'` print as the value they collapsed to. A hexadecimal or
//! octal digit with any unknown bit is `X`, and one whose bits are all `Z`
//! is `Z`, as `ieee.std_logic_1164`'s `to_hstring` has them. A signed
//! vector is sign-extended to a whole number of digits and an unsigned
//! one zero-extended, as `ieee.numeric_std` does; `std_logic_vector`
//! counts as unsigned.

use crate::ir::Type;
use crate::logic::{Bit, Logic};

use super::value::Value;

/// Evaluates one of the run-time string functions, or `None` when `name`
/// is not one of them.
pub(crate) fn call(name: &str, args: &[Value], ty: &Type) -> Option<Value> {
    let _ = ty;
    let text = match name {
        "$vhdl_image_int" => int_image(args.first()?),
        "$vhdl_image_real" => crate::logic::vhdl_real_image(args.first().map_or(0.0, real_of)),
        "$vhdl_image_time" => time_image(args.first()?),
        "$vhdl_image_enum" => enum_image(args),
        "$vhdl_to_string" => bit_string(&logic_of(args.first()?)),
        "$vhdl_to_hstring" => digits(&logic_of(args.first()?), 4, signed_arg(args)),
        "$vhdl_to_ostring" => digits(&logic_of(args.first()?), 3, signed_arg(args)),
        "$vhdl_concat" => args.iter().map(text_of).collect(),
        "$vhdl_streq" => {
            let equal = match (args.first(), args.get(1)) {
                (Some(a), Some(b)) => text_of(a) == text_of(b),
                _ => false,
            };
            return Some(Value::Bits(Logic::from_bool(equal)));
        }
        _ => return None,
    };
    Some(Value::Str(text))
}

/// Whether a vector converted to digits is signed: the second argument
/// when there is one, else the value's own flag.
fn signed_arg(args: &[Value]) -> bool {
    match args.get(1) {
        Some(flag) => flag.truth() == Bit::One,
        None => args.first().is_some_and(|v| v.to_logic().is_signed()),
    }
}

/// The value as a bit vector.
fn logic_of(v: &Value) -> Logic {
    v.to_logic()
}

/// The value as a real.
fn real_of(v: &Value) -> f64 {
    v.to_real()
}

/// The text of a string argument, or the character a bit vector holds.
fn text_of(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        other => {
            let code = other.to_logic().to_u64().unwrap_or(u64::from(b'?'));
            let byte = u8::try_from(code & 0xFF).unwrap_or(b'?');
            char::from(byte).to_string()
        }
    }
}

/// The decimal image of an integer, which VHDL holds signed.
fn int_image(v: &Value) -> String {
    let l = logic_of(v).as_signed();
    match l.to_i64() {
        Some(i) => i.to_string(),
        None if l.has_unknown() => "X".to_owned(),
        None => "?".to_owned(),
    }
}

/// The image of a time held as a femtosecond count.
fn time_image(v: &Value) -> String {
    let l = logic_of(v).as_signed();
    let Some(fs) = l.to_i64() else {
        return "X".to_owned();
    };
    if fs % 1000 == 0 {
        format!("{} ps", fs / 1000)
    } else {
        format!("{fs} fs")
    }
}

/// The literal at the position the first argument holds, from the list of
/// literals that follows it.
fn enum_image(args: &[Value]) -> String {
    let Some((pos, literals)) = args.split_first() else {
        return String::new();
    };
    let pos = logic_of(pos).to_u64().and_then(|p| usize::try_from(p).ok());
    match pos.and_then(|p| literals.get(p)) {
        Some(lit) => text_of(lit),
        None => "X".to_owned(),
    }
}

/// One character per bit, most significant first.
fn bit_string(l: &Logic) -> String {
    l.bits().iter().rev().map(|b| bit_char(*b)).collect()
}

/// The character `ieee.std_logic_1164`'s `to_string` writes for a bit.
fn bit_char(b: Bit) -> char {
    match b {
        Bit::Zero => '0',
        Bit::One => '1',
        Bit::X => 'X',
        Bit::Z => 'Z',
    }
}

/// The value in digits of `per` bits each, most significant first.
fn digits(l: &Logic, per: u32, signed: bool) -> String {
    let width = l.width();
    let pad = (per - width % per) % per;
    let fill = if signed { l.msb() } else { Bit::Zero };
    let mut bits: Vec<Bit> = vec![fill; usize::try_from(pad).unwrap_or(0)];
    bits.extend(l.bits().iter().rev());
    let mut out = String::new();
    for chunk in bits.chunks(usize::try_from(per).unwrap_or(4)) {
        if chunk.iter().all(|b| *b == Bit::Z) {
            out.push('Z');
            continue;
        }
        let mut v = 0u32;
        let mut known = true;
        for b in chunk {
            match b {
                Bit::Zero => v <<= 1,
                Bit::One => v = (v << 1) | 1,
                _ => known = false,
            }
        }
        out.push(if known {
            char::from_digit(v, 16).map_or('?', |c| c.to_ascii_uppercase())
        } else {
            'X'
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        Value::Str(text.to_owned())
    }

    fn bits(text: &str) -> Value {
        Value::Bits(Logic::parse_verilog(text).unwrap())
    }

    fn run(name: &str, args: &[Value]) -> String {
        match call(name, args, &Type::String) {
            Some(Value::Str(t)) => t,
            other => panic!("{name} gave {other:?}"),
        }
    }

    /// Would catch an integer image read unsigned (`4294967295` for -1) or
    /// a time written in femtoseconds when it is whole picoseconds; would
    /// not catch a wrong unit name for a physical type other than `time`,
    /// which the frontend writes itself.
    #[test]
    fn images_of_scalars() {
        assert_eq!(run("$vhdl_image_int", &[bits("32'hffffffff")]), "-1");
        assert_eq!(run("$vhdl_image_int", &[bits("32'd42")]), "42");
        assert_eq!(run("$vhdl_image_time", &[bits("64'd5000000")]), "5000 ps");
        assert_eq!(run("$vhdl_image_time", &[bits("64'd1")]), "1 fs");
        assert_eq!(run("$vhdl_image_real", &[Value::Real(2.5)]), "2.500000e+00");
        let lits = [bits("2'd1"), s("r"), s("g"), s("b")];
        assert_eq!(run("$vhdl_image_enum", &lits), "g");
        assert_eq!(run("$vhdl_image_enum", &[bits("2'd3"), s("r")]), "X");
    }

    /// Would catch digits grouped from the left rather than the right, a
    /// signed vector zero-extended, and an unknown digit printed as a
    /// number; would not catch a nine-state value, which the simulator
    /// does not hold.
    #[test]
    fn vectors_as_text() {
        assert_eq!(run("$vhdl_to_string", &[bits("4'b10xz")]), "10XZ");
        assert_eq!(run("$vhdl_to_hstring", &[bits("8'ha6")]), "A6");
        assert_eq!(run("$vhdl_to_hstring", &[bits("6'b101010")]), "2A");
        let neg = Value::Bits(Logic::parse_verilog("6'b101010").unwrap().as_signed());
        assert_eq!(run("$vhdl_to_hstring", &[neg]), "EA");
        let flag = Value::Bits(Logic::from_bool(true));
        assert_eq!(run("$vhdl_to_hstring", &[bits("6'b101010"), flag]), "EA");
        assert_eq!(run("$vhdl_to_ostring", &[bits("6'b101010")]), "52");
        assert_eq!(run("$vhdl_to_hstring", &[bits("8'b1x00zzzz")]), "XZ");
    }

    /// Would catch a character argument concatenated as its decimal code
    /// and a string comparison that ignores length.
    #[test]
    fn concatenation_and_equality() {
        assert_eq!(
            run("$vhdl_concat", &[s("a="), bits("8'h41"), s("!")]),
            "a=A!"
        );
        let eq = call("$vhdl_streq", &[s("ab"), s("ab")], &Type::bit());
        assert_eq!(eq, Some(Value::Bits(Logic::from_bool(true))));
        let ne = call("$vhdl_streq", &[s("ab"), s("abc")], &Type::bit());
        assert_eq!(ne, Some(Value::Bits(Logic::from_bool(false))));
        assert!(call("$display", &[], &Type::String).is_none());
    }
}
