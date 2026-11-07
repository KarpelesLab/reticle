//! What the VHDL frontend needs from the simulator beyond the IR's own
//! semantics: run-time strings, a signal's history, and initial values.
//!
//! # Strings
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
//! | `$vhdl_real` | the 64 bits of an IEEE 754 double | that `real`; the IR has no real literal |
//! | `$vhdl_int_to_real`, `$vhdl_real_to_int` | a value | `real(i)`, and `integer(r)`, which rounds halves away from zero |
//! | `$vhdl_math` | a function's name, then its arguments | an `ieee.math_real` function, `**` or `mod` of reals |
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
//!
//! # A signal's history
//!
//! `s'event`, `s'last_value` and `s'last_event` (LRM 16.2.4) read what a
//! signal did, not what it holds, and so does `rising_edge(s)` outside the
//! clock condition of a process. The frontend gives such a signal a
//! *shadow* net, marked with two attributes — `vhdl_shadow` names what it
//! holds (`event`, `last_value` or `last_event`) and `vhdl_shadow_of` the
//! net it shadows — and the simulator keeps it:
//!
//! - `last_value` takes the old value whenever the signal changes;
//! - `last_event` takes the time of that change, in femtoseconds, and
//!   starts far in the past, so a signal that never changed has been
//!   stable for ever;
//! - `event` is set when the signal changes and cleared when the delta
//!   cycle ends: before the next batch of non-blocking updates is applied,
//!   and when time advances;
//! - `start` only starts with the signal's initial value: `s'delayed(t)`
//!   is a signal of its own, kept by a process, whose value before `t` has
//!   passed is the signal's initial one.
//!
//! A shadow is updated before the signal's fanout is scheduled, so a
//! process woken by the change reads the new history. A change made by a
//! continuous assignment happens in the active region rather than at a
//! delta boundary, so its `event` stays set until the next one; a process
//! that runs later in the same region for another reason sees it too.
//!
//! # Initial values
//!
//! A VHDL signal holds its initial value from the start, with no event
//! (LRM 14.7.5.1): `signal c : std_logic := '1'` does not rise at time
//! zero. The frontend writes an initial value as an `initial` process
//! marked `vhdl_initial`, and the simulator applies such a process before
//! time zero, silently, when its body is only assignments of constants to
//! whole nets; anything else runs as an ordinary `initial` process.

use crate::ir::{AttrValue, Type};
use crate::logic::{Bit, Logic};

use super::Simulator;
use super::elab::SigId;
use super::value::Value;

/// What a shadow net holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShadowKind {
    /// Set while the signal changed in this delta cycle.
    Event,
    /// The signal's value before its last change.
    LastValue,
    /// The time of the signal's last change, in femtoseconds.
    LastEvent,
    /// Nothing kept: the net only starts with the signal's initial value
    /// (`s'delayed(t)`, which its own process then keeps).
    Start,
}

/// The shadows of the design's signals.
#[derive(Debug, Default)]
pub(crate) struct VhdlState {
    /// Every shadow: the signal shadowed, what is kept, the shadow.
    shadows: Vec<(SigId, ShadowKind, SigId)>,
    /// The `event` shadows set in the current delta cycle.
    raised: Vec<SigId>,
}

impl Simulator<'_> {
    /// Finds the shadow nets of every instance and applies the initial
    /// values marked `vhdl_initial`. Runs once, after elaboration and
    /// before time zero is scheduled.
    pub(crate) fn vhdl_prepare(&mut self) {
        // Initial values first, so a shadow starts from them.
        self.vhdl_initial_values();
        for i in 0..self.instances.len() {
            let m = self.instances[i].m;
            for (id, net) in m.nets.iter() {
                let (Some(AttrValue::String(kind)), Some(AttrValue::String(of))) = (
                    net.attrs.get("vhdl_shadow"),
                    net.attrs.get("vhdl_shadow_of"),
                ) else {
                    continue;
                };
                let kind = match kind.as_str() {
                    "event" => ShadowKind::Event,
                    "last_value" => ShadowKind::LastValue,
                    "last_event" => ShadowKind::LastEvent,
                    "start" => ShadowKind::Start,
                    _ => continue,
                };
                let Some(base) = m
                    .nets
                    .iter()
                    .find(|(_, n)| n.name.as_str() == of.as_str())
                    .map(|(b, _)| b)
                else {
                    continue;
                };
                let shadow = self.instances[i].nets[id.index()];
                let base = self.instances[i].nets[base.index()];
                let start = match kind {
                    ShadowKind::Event => Logic::zero(1),
                    ShadowKind::LastValue | ShadowKind::Start => {
                        self.signals[base.idx()].value.clone()
                    }
                    ShadowKind::LastEvent => Logic::from_i64(-(1 << 62), 64).with_signed(true),
                };
                self.set_quietly(shadow, start);
                if kind != ShadowKind::Start {
                    self.vhdl.shadows.push((base, kind, shadow));
                }
            }
        }
    }

    /// Applies the `vhdl_initial` processes that only assign constants to
    /// whole nets, and retires them.
    fn vhdl_initial_values(&mut self) {
        for p in 0..self.procs.len() {
            let proc_ = self.procs[p].process;
            if !matches!(proc_.kind, crate::ir::ProcessKind::Initial)
                || proc_.attrs.get("vhdl_initial").is_none()
            {
                continue;
            }
            let inst = self.procs[p].inst;
            let m = self.instances[inst.idx()].m;
            let mut writes = Vec::new();
            let simple = proc_.body.iter().all(|s| match &s.kind {
                crate::ir::StmtKind::Assign {
                    target: crate::ir::Lvalue::Net(n),
                    value,
                    delay: None,
                    ..
                } => match m.exprs.get(*value).map(|e| &e.kind) {
                    Some(crate::ir::ExprKind::Const(c)) => {
                        writes.push((*n, c.clone()));
                        true
                    }
                    _ => false,
                },
                _ => false,
            });
            if !simple {
                continue;
            }
            for (n, c) in writes {
                let sig = self.instances[inst.idx()].nets[n.index()];
                self.set_quietly(sig, c);
            }
            self.procs[p].status = super::process::ProcStatus::Dead;
        }
    }

    /// Sets a signal's value without an event: before time zero, nothing
    /// can be waiting on it.
    fn set_quietly(&mut self, sig: SigId, value: Logic) {
        let s = &mut self.signals[sig.idx()];
        let width = s.value.width();
        let value = if value.width() == width {
            value
        } else {
            value.resize(width)
        };
        s.value = value.with_signed(s.value.is_signed());
    }

    /// Keeps the shadows of `sig`, which just changed from `old`.
    pub(crate) fn vhdl_note_change(&mut self, sig: SigId, old: &Logic) {
        let now =
            i64::try_from(u128::from(self.now) * u128::from(self.precision_fs)).unwrap_or(i64::MAX);
        let mut writes = Vec::new();
        for &(base, kind, shadow) in &self.vhdl.shadows {
            if base != sig {
                continue;
            }
            writes.push(match kind {
                ShadowKind::Event => (shadow, Logic::from_bool(true)),
                ShadowKind::LastValue => (shadow, old.clone()),
                ShadowKind::LastEvent => (shadow, Logic::from_i64(now, 64).with_signed(true)),
                ShadowKind::Start => continue,
            });
            if kind == ShadowKind::Event && !self.vhdl.raised.contains(&shadow) {
                self.vhdl.raised.push(shadow);
            }
        }
        for (shadow, value) in writes {
            self.write_signal(shadow, value);
        }
    }

    /// Ends a delta cycle for the `event` shadows: what changed in it no
    /// longer has an event.
    pub(crate) fn vhdl_end_delta(&mut self) {
        let raised = std::mem::take(&mut self.vhdl.raised);
        for shadow in raised {
            self.write_signal(shadow, Logic::from_bool(false));
        }
    }

    /// True when some signal has a shadow.
    pub(crate) fn vhdl_has_shadows(&self) -> bool {
        !self.vhdl.shadows.is_empty()
    }
}

/// Evaluates one of the run-time string functions, or `None` when `name`
/// is not one of them.
pub(crate) fn call(name: &str, args: &[Value], ty: &Type) -> Option<Value> {
    if let Some(v) = real_call(name, args, ty) {
        return Some(v);
    }
    let text = match name {
        "$vhdl_image_int" => int_image(args.first()?),
        "$vhdl_image_real" => crate::logic::vhdl_real_image(args.first().map_or(0.0, real_of)),
        "$vhdl_image_time" => time_image(args.first()?),
        "$vhdl_image_enum" => enum_image(args),
        "$vhdl_to_string" => bit_string(&logic_of(args.first()?)),
        "$vhdl_to_hstring" => digits(&logic_of(args.first()?), 4, signed_arg(args)),
        "$vhdl_to_ostring" => digits(&logic_of(args.first()?), 3, signed_arg(args)),
        "$vhdl_concat" => args.iter().map(text_of).collect(),
        "$vhdl_time_in" => time_in(args.first()?, args.get(1)?),
        "$vhdl_real_digits" => {
            let r = args.first().map_or(f64::NAN, real_of);
            match logic_of(args.get(1)?).to_u64() {
                // No digits is the real's image (LRM 5.7).
                Some(0) | None => crate::logic::vhdl_real_image(r),
                Some(d) => format!("{r:.*}", usize::try_from(d).unwrap_or(0)),
            }
        }
        "$vhdl_real_format" => real_format(
            args.first().map_or(f64::NAN, real_of),
            &text_of(args.get(1)?),
        ),
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

/// The `real` functions: a constant, the two conversions, and the
/// subprograms of `ieee.math_real` (LRM 16.12 for the package; the
/// functions are Rust's `f64` ones). `$vhdl_math` takes the function's
/// name first.
fn real_call(name: &str, args: &[Value], ty: &Type) -> Option<Value> {
    let real = |i: usize| args.get(i).map_or(f64::NAN, real_of);
    Some(match name {
        "$vhdl_real" => Value::Real(
            args.first()
                .and_then(|v| v.to_logic().to_u64())
                .map_or(f64::NAN, f64::from_bits),
        ),
        "$vhdl_int_to_real" => {
            let l = logic_of(args.first()?).as_signed();
            #[allow(clippy::cast_precision_loss, reason = "real(i) is inexact past 2^53")]
            let r = l.to_i64().map_or(f64::NAN, |i| i as f64);
            Value::Real(r)
        }
        "$vhdl_real_to_int" => {
            // A conversion to an integer type rounds to the nearest,
            // halves away from zero (LRM 9.3.6), as `f64::round` does.
            let width = ty.width().unwrap_or(32);
            let r = real(0).round();
            if !r.is_finite() {
                return Some(Value::Bits(Logic::x(width)));
            }
            #[allow(clippy::cast_possible_truncation, reason = "saturating is the intent")]
            let i = r as i64;
            Value::Bits(Logic::from_i64(i, width).with_signed(true))
        }
        "$vhdl_math" => {
            let Some(Value::Str(f)) = args.first() else {
                return None;
            };
            let x = real(1);
            let y = real(2);
            Value::Real(match f.as_str() {
                "sqrt" => x.sqrt(),
                "cbrt" => x.cbrt(),
                "exp" => x.exp(),
                "log" if args.len() == 2 => x.ln(),
                "log" => x.ln() / y.ln(),
                "log2" => x.log2(),
                "log10" => x.log10(),
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "arcsin" => x.asin(),
                "arccos" => x.acos(),
                "arctan" if args.len() == 2 => x.atan(),
                "arctan" => x.atan2(y),
                "sinh" => x.sinh(),
                "cosh" => x.cosh(),
                "tanh" => x.tanh(),
                "arcsinh" => x.asinh(),
                "arccosh" => x.acosh(),
                "arctanh" => x.atanh(),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "round" => x.round(),
                "trunc" => x.trunc(),
                "abs" => x.abs(),
                "sign" => {
                    if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                "realmax" => x.max(y),
                "realmin" => x.min(y),
                "**" => x.powf(y),
                // `mod` takes the sign of the divisor, as the integer one does.
                "mod" => x - y * (x / y).floor(),
                _ => f64::NAN,
            })
        }
        _ => return None,
    })
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

/// `to_string(t, unit)`: the time in the unit, followed by the unit's name.
///
/// LRM 5.7 has a time that is not a whole number of units written as a
/// real (`1.5 ns`); xsim writes the whole number of units it holds,
/// truncated (`1 ns`), and a unit finer than its picosecond resolution as
/// picoseconds. Both were measured, and both are xsim's here.
fn time_in(t: &Value, unit: &Value) -> String {
    let (Some(t), Some(u)) = (
        logic_of(t).as_signed().to_i64(),
        logic_of(unit).as_signed().to_i64(),
    ) else {
        return "X".to_owned();
    };
    let names = [
        (1, "fs"),
        (1_000, "ps"),
        (1_000_000, "ns"),
        (1_000_000_000, "us"),
        (1_000_000_000_000, "ms"),
        (1_000_000_000_000_000, "sec"),
        (60_000_000_000_000_000, "min"),
        (3_600_000_000_000_000_000, "hr"),
    ];
    let u = u.max(1_000);
    let Some(&(_, name)) = names.iter().find(|(fs, _)| *fs == u) else {
        return format!("{} ps", t / 1_000);
    };
    format!("{} {name}", t / u)
}

/// `to_string(r, format)`: one C conversion, `%f`, `%e` or `%g`, with an
/// optional precision (`"%.3f"`); text around it is kept.
fn real_format(r: f64, fmt: &str) -> String {
    let Some(pos) = fmt.find('%') else {
        return fmt.to_owned();
    };
    let (head, rest) = fmt.split_at(pos);
    let spec: String = rest[1..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let tail = &rest[1 + spec.len()..];
    let Some(conv) = tail.chars().next() else {
        return fmt.to_owned();
    };
    let precision = spec
        .split_once('.')
        .map_or(6, |(_, p)| p.parse::<usize>().unwrap_or(6));
    let body = match conv {
        'f' | 'F' => format!("{r:.precision$}"),
        'e' | 'E' => {
            let t = format!("{r:.precision$e}");
            match t.split_once('e') {
                Some((m, e)) => {
                    let e: i32 = e.parse().unwrap_or(0);
                    let sign = if e < 0 { '-' } else { '+' };
                    format!("{m}e{sign}{:02}", e.unsigned_abs())
                }
                None => t,
            }
        }
        _ => format!("{r}"),
    };
    format!("{head}{body}{}", &tail[conv.len_utf8()..])
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

    /// Would catch a conversion that truncates (`integer(-2.5)` is -3), a
    /// `real` constant read through the wrong bits, and `mod` with the
    /// dividend's sign; would not catch a function missing from the
    /// table, which answers NaN rather than failing.
    #[test]
    fn reals() {
        let r = |name: &str, args: &[Value], ty: &Type| call(name, args, ty);
        let bits = Value::Bits(Logic::from_u64(2.5f64.to_bits(), 64));
        assert_eq!(
            r("$vhdl_real", &[bits], &Type::Real),
            Some(Value::Real(2.5))
        );
        let i = r("$vhdl_real_to_int", &[Value::Real(-2.5)], &Type::sbits(32));
        assert_eq!(
            i,
            Some(Value::Bits(Logic::from_i64(-3, 32).with_signed(true)))
        );
        let back = r(
            "$vhdl_int_to_real",
            &[Value::Bits(Logic::from_i64(-7, 32))],
            &Type::Real,
        );
        assert_eq!(back, Some(Value::Real(-7.0)));
        let m = r(
            "$vhdl_math",
            &[s("mod"), Value::Real(-1.0), Value::Real(3.0)],
            &Type::Real,
        );
        assert_eq!(m, Some(Value::Real(2.0)));
        let sq = r("$vhdl_math", &[s("sqrt"), Value::Real(16.0)], &Type::Real);
        assert_eq!(sq, Some(Value::Real(4.0)));
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
