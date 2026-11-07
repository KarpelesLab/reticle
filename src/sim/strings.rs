//! The methods of a SystemVerilog `string` (IEEE 1800-2017 §6.16), as the
//! calls `string.<method>` the Verilog frontend lowers them to. The first
//! argument is always the string; a method that changes it (`putc`,
//! `itoa`, ...) returns the new string, which the frontend assigns back.

use crate::logic::Logic;

use super::value::{Value, logic_to_string};

/// The text of a value used as a string.
fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::Str(s)) => s.clone(),
        Some(other) => logic_to_string(&other.to_logic()),
        None => String::new(),
    }
}

/// An integer argument, `None` when it has unknown bits.
fn int(v: Option<&Value>) -> Option<i64> {
    let l = v?.to_logic();
    if l.has_unknown() {
        return None;
    }
    if l.is_signed() {
        l.to_i64()
    } else {
        l.to_u64().and_then(|u| i64::try_from(u).ok())
    }
}

/// A 32-bit `int` result.
fn int32(n: i64) -> Value {
    // An `int` result wraps as the standard's 32-bit `int` does.
    #[allow(clippy::cast_possible_truncation)]
    let n = n as i32;
    Value::Bits(Logic::from_i64(i64::from(n), 32).as_signed())
}

/// The leading number of `s` in `radix`, as `atoi` and its kin read it:
/// leading white space and a sign, then digits and underscores up to the
/// first character that is neither.
fn leading_number(s: &str, radix: u32) -> i64 {
    let t = s.trim_start();
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut n: i64 = 0;
    for c in t.chars() {
        if c == '_' {
            continue;
        }
        let Some(d) = c.to_digit(radix) else { break };
        n = n.wrapping_mul(i64::from(radix)).wrapping_add(i64::from(d));
    }
    if neg { n.wrapping_neg() } else { n }
}

/// The digits of `l` in base `2^bits`, without leading zeros (`0` for
/// zero), as `hextoa`, `octtoa` and `bintoa` write them.
fn digits(l: &Logic, bits: u32) -> String {
    let w = l.width();
    let n = w.div_ceil(bits);
    let mut out = String::new();
    for i in (0..n).rev() {
        let lo = i * bits;
        let hi = (lo + bits - 1).min(w - 1);
        let d = l.slice(hi, lo);
        let c = match d.to_u64() {
            Some(v) => char::from_digit(u32::try_from(v).unwrap_or(0), 16).unwrap_or('?'),
            None => 'x',
        };
        if out.is_empty() && c == '0' {
            continue;
        }
        out.push(c);
    }
    if out.is_empty() {
        out.push('0');
    }
    out
}

/// Evaluates `string.<method>`; `None` when `name` is not one.
pub(crate) fn method(name: &str, args: &[Value]) -> Option<Value> {
    let method = name.strip_prefix("string.")?;
    let s = text(args.first());
    let arg = |i: usize| args.get(i);
    Some(match method {
        "len" => int32(i64::try_from(s.len()).unwrap_or(0)),
        "getc" => {
            let c = int(arg(1))
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| s.as_bytes().get(i).copied())
                .unwrap_or(0);
            Value::Bits(Logic::from_u64(u64::from(c), 8))
        }
        "replicate" => Value::Str(match int(arg(1)) {
            Some(n) if n > 0 => s.repeat(usize::try_from(n).unwrap_or(0)),
            _ => String::new(),
        }),
        "toupper" => Value::Str(s.to_ascii_uppercase()),
        "tolower" => Value::Str(s.to_ascii_lowercase()),
        "substr" => {
            let i = int(arg(1));
            let j = int(arg(2));
            let len = i64::try_from(s.len()).unwrap_or(i64::MAX);
            Value::Str(match (i, j) {
                (Some(i), Some(j)) if i >= 0 && j >= i && j < len => {
                    let (i, j) = (
                        usize::try_from(i).unwrap_or(0),
                        usize::try_from(j).unwrap_or(0),
                    );
                    String::from_utf8_lossy(&s.as_bytes()[i..=j]).into_owned()
                }
                _ => String::new(),
            })
        }
        "compare" | "icompare" => {
            let t = text(arg(1));
            let (a, b) = if method == "icompare" {
                (s.to_ascii_lowercase(), t.to_ascii_lowercase())
            } else {
                (s, t)
            };
            // As C's `strcmp`, which xsim follows: the difference of the
            // first two characters that differ, the end of a string
            // counting as a NUL ("Hello" against "World" is -15).
            let (a, b) = (a.as_bytes(), b.as_bytes());
            let n = a.len().max(b.len());
            let d = (0..n)
                .map(|k| {
                    i64::from(a.get(k).copied().unwrap_or(0))
                        - i64::from(b.get(k).copied().unwrap_or(0))
                })
                .find(|d| *d != 0)
                .unwrap_or(0);
            int32(d)
        }
        "atoi" => int32(leading_number(&s, 10)),
        "atohex" => int32(leading_number(&s, 16)),
        "atooct" => int32(leading_number(&s, 8)),
        "atobin" => int32(leading_number(&s, 2)),
        "atoreal" => {
            let t = s.trim_start();
            let end = t
                .char_indices()
                .find(|(i, c)| {
                    !(c.is_ascii_digit()
                        || *c == '.'
                        || *c == '_'
                        || ((*c == '-' || *c == '+') && (*i == 0 || t[..*i].ends_with(['e', 'E'])))
                        || *c == 'e'
                        || *c == 'E')
                })
                .map_or(t.len(), |(i, _)| i);
            Value::Real(t[..end].replace('_', "").parse().unwrap_or(0.0))
        }
        "putc" => {
            let mut bytes = s.into_bytes();
            let c = arg(2)
                .map(Value::to_logic)
                .and_then(|l| l.resize(8).to_u64());
            if let (Some(i), Some(c)) = (int(arg(1)).and_then(|i| usize::try_from(i).ok()), c)
                && c != 0
                && i < bytes.len()
            {
                bytes[i] = u8::try_from(c).unwrap_or(b'?');
            }
            Value::Str(String::from_utf8_lossy(&bytes).into_owned())
        }
        "itoa" => Value::Str(match arg(1).map(Value::to_logic) {
            Some(l) if !l.has_unknown() => {
                if l.is_signed() {
                    l.to_i64().map_or_else(|| "0".to_owned(), |n| n.to_string())
                } else {
                    l.to_u64().map_or_else(|| "0".to_owned(), |n| n.to_string())
                }
            }
            _ => "x".to_owned(),
        }),
        "hextoa" => Value::Str(digits(
            &arg(1).map_or_else(|| Logic::zero(1), Value::to_logic),
            4,
        )),
        "octtoa" => Value::Str(digits(
            &arg(1).map_or_else(|| Logic::zero(1), Value::to_logic),
            3,
        )),
        "bintoa" => Value::Str(digits(
            &arg(1).map_or_else(|| Logic::zero(1), Value::to_logic),
            1,
        )),
        "realtoa" => Value::Str(format!("{}", arg(1).map_or(0.0, Value::to_real))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.to_owned())
    }

    fn i(n: i64) -> Value {
        Value::Bits(Logic::from_i64(n, 32).as_signed())
    }

    #[test]
    fn readers() {
        assert_eq!(method("string.len", &[s("abc")]), Some(int32(3)));
        assert_eq!(
            method("string.getc", &[s("abc"), i(1)]),
            Some(Value::Bits(Logic::from_u64(u64::from(b'b'), 8)))
        );
        assert_eq!(
            method("string.getc", &[s("abc"), i(5)]),
            Some(Value::Bits(Logic::from_u64(0, 8)))
        );
        assert_eq!(
            method("string.substr", &[s("hello"), i(1), i(3)]),
            Some(s("ell"))
        );
        assert_eq!(
            method("string.substr", &[s("hello"), i(3), i(9)]),
            Some(s(""))
        );
        assert_eq!(method("string.toupper", &[s("aB1")]), Some(s("AB1")));
        assert_eq!(
            method("string.compare", &[s("ab"), s("b")]),
            Some(int32(-1))
        );
        assert_eq!(
            method("string.compare", &[s("Hello"), s("World")]),
            Some(int32(-15))
        );
        assert_eq!(
            method("string.compare", &[s("ab"), s("a")]),
            Some(int32(98))
        );
        assert_eq!(
            method("string.icompare", &[s("AB"), s("ab")]),
            Some(int32(0))
        );
        assert_eq!(method("string.atoi", &[s(" -12_3x")]), Some(int32(-123)));
        assert_eq!(method("string.atohex", &[s("fF")]), Some(int32(255)));
        assert_eq!(
            method("string.atoreal", &[s("2.5e1z")]),
            Some(Value::Real(25.0))
        );
        assert_eq!(
            method("string.replicate", &[s("ab"), i(3)]),
            Some(s("ababab"))
        );
        assert_eq!(method("string.replicate", &[s("ab"), i(-1)]), Some(s("")));
        assert_eq!(method("string.nonesuch", &[s("")]), None);
        assert_eq!(method("len", &[s("")]), None);
    }

    #[test]
    fn mutators() {
        assert_eq!(
            method(
                "string.putc",
                &[s("abc"), i(1), Value::Bits(Logic::from_u64(0x5a, 8))]
            ),
            Some(s("aZc"))
        );
        assert_eq!(
            method(
                "string.putc",
                &[s("abc"), i(7), Value::Bits(Logic::from_u64(0x5a, 8))]
            ),
            Some(s("abc"))
        );
        assert_eq!(method("string.itoa", &[s(""), i(-42)]), Some(s("-42")));
        assert_eq!(method("string.hextoa", &[s(""), i(255)]), Some(s("ff")));
        assert_eq!(method("string.bintoa", &[s(""), i(5)]), Some(s("101")));
        assert_eq!(method("string.octtoa", &[s(""), i(8)]), Some(s("10")));
    }
}
