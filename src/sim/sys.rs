//! System tasks and functions.
//!
//! Tasks (`Stmt::SysCall`) and functions (`ExprKind::Call`) named with a
//! `$` prefix follow IEEE 1364-2005 clause 17; VHDL's `report` is accepted
//! as a task too. Implemented:
//!
//! | Name | Behaviour |
//! |------|-----------|
//! | `$display` `$displayb` `$displayh` `$displayo`, `$write*` | Formatted output (§17.1.1); `$display` appends a newline |
//! | `$strobe*` | Output at the end of the time slot (§17.1.2) |
//! | `$monitor*`, `$monitoron`, `$monitoroff` | Output at the end of a slot in which an argument other than `$time`, `$stime` or `$realtime` changed value (§17.1.3) |
//! | `$finish`, `$stop` | End or pause the run (§17.4) |
//! | `$fatal` `$error` `$warning` `$info`, `report` | Severity reports; `$fatal` ends the run |
//! | `$readmemh` `$readmemb` | Load a memory from a [`FileProvider`] (§17.2.9); the IR carries them as [`StmtKind::MemFile`](crate::ir::StmtKind::MemFile), which names the memory itself |
//! | `$writememh` `$writememb` | Save a memory's contents, which [`Simulator::written_files`] hands back |
//! | `$timeformat` | Sets the `%t` format (§17.3.2) |
//! | `$printtimescale` | Prints a scope's time unit and precision as xsim does (§17.3.1) |
//! | `$dumpvars` `$dumpfile` `$dumpon` `$dumpoff` `$dumpall` | `$dumpvars` enables VCD capture; the rest are accepted and ignored |
//! | `$time` `$stime` `$realtime` | The current time in the module's unit (§17.7) |
//! | `$random`, `$dist_uniform` `$dist_normal` `$dist_exponential` `$dist_poisson` `$dist_chi_square` `$dist_t` `$dist_erlang` | The standard's generator (§17.9), in `random.rs`; a seed variable is written back |
//! | `$urandom` `$urandom_range` | A seeded xorshift generator; deterministic per seed |
//! | `$clog2` `$bits` `$itor` `$rtoi` `$realtobits` `$bitstoreal` `$sformatf` | Utility functions |
//!
//! Formatting understands `%d %i %h %x %o %b %s %c %t %m %l %e %f %g %%`
//! with an optional `-`, zero flag, width and precision. Where IEEE
//! 1364-2005 §17.1.1 leaves room, or where xsim 2026.1 reads it otherwise,
//! the output is xsim's, measured line by line
//! (`testdata/sim/display_format_tb.v` and its `.xsim`): a signed `%d`
//! field is as wide as the digits of 2^N - 1 and a sign; any width on
//! `%h`/`%o`/`%b`, even zero, drops leading zeros and then zero-fills; `%X`
//! prints lower case; `%s` prints a NUL byte as a space; the real
//! conversions ignore `-`. `docs/simulation.md` lists each. Unknown tasks
//! are warned about once and ignored; unknown functions return `x` of their
//! cached type.

use std::fmt::Write;

use crate::diag::Diagnostic;
use crate::ir::memfile;
use crate::ir::{ExprId, ExprKind, MemFileOp, MemoryId, ReportSeverity, Span, Type};
use crate::logic::{Bit, Logic};

use super::Simulator;
use super::elab::{InstId, ProcId};
use super::value::{Value, flat_width};

/// The `%t` format set by `$timeformat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimeFormat {
    /// Femtoseconds per displayed unit.
    pub(crate) unit_fs: u64,
    /// Fractional digits.
    pub(crate) precision: u32,
    /// Text appended after the number.
    pub(crate) suffix: String,
    /// Minimum field width.
    pub(crate) min_width: usize,
}

impl TimeFormat {
    /// The default: the simulation precision as unit, no fraction, no
    /// suffix, width 20 (§17.3.2).
    pub(crate) fn for_precision(precision_fs: u64) -> Self {
        TimeFormat {
            unit_fs: precision_fs.max(1),
            precision: 0,
            suffix: String::new(),
            min_width: 20,
        }
    }

    /// Formats a time given in femtoseconds.
    pub(crate) fn format(&self, fs: u128) -> String {
        let unit = u128::from(self.unit_fs);
        let whole = fs / unit;
        let mut text = whole.to_string();
        if self.precision > 0 {
            let rem = fs % unit;
            let scale = 10u128.pow(self.precision.min(18));
            let frac = rem * scale / unit;
            let _ = write!(
                text,
                ".{:0width$}",
                frac,
                width = usize::try_from(self.precision.min(18)).unwrap_or(18)
            );
        }
        text.push_str(&self.suffix);
        text
    }
}

/// An active `$monitor`.
pub(crate) struct Monitor {
    /// The instance the arguments belong to.
    pub(crate) inst: InstId,
    /// The arguments as given to `$monitor`.
    pub(crate) args: Vec<ExprId>,
    /// The radix of unformatted arguments (`$monitorh` prints hex).
    pub(crate) radix: char,
    /// The values of the arguments that trigger it when last printed:
    /// every one but `$time`, `$stime` and `$realtime` (IEEE 1364-2005
    /// §17.1.3). `None` until the first print.
    pub(crate) last: Option<Vec<Value>>,
    /// `$monitoroff` suppresses output.
    pub(crate) enabled: bool,
}

/// True when two monitored values are the same, a NaN equal to itself so
/// that a real argument holding one does not print at every step.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Real(x), Value::Real(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

/// A parsed `%` conversion.
struct Spec {
    /// `-`: justify to the left.
    left: bool,
    /// A width written with a leading zero (`%05d`): pad with zeros.
    zero: bool,
    width: Option<usize>,
    precision: Option<usize>,
    conv: char,
}

/// The decimal digits of an unsigned multi-word value.
fn decimal_words(words: &[u64]) -> String {
    let mut w: Vec<u64> = words.to_vec();
    while w.last() == Some(&0) {
        w.pop();
    }
    if w.is_empty() {
        return "0".to_owned();
    }
    let mut digits = Vec::new();
    while !w.is_empty() {
        let mut rem: u128 = 0;
        for word in w.iter_mut().rev() {
            let cur = (rem << 64) | u128::from(*word);
            *word = u64::try_from(cur / 10).expect("quotient fits");
            rem = cur % 10;
        }
        digits.push(char::from_digit(u32::try_from(rem).expect("digit"), 10).expect("digit"));
        while w.last() == Some(&0) {
            w.pop();
        }
    }
    digits.iter().rev().collect()
}

/// The default `%d` field width for a `width`-bit operand: the decimal
/// digits of its largest unsigned value, `2^width - 1`, plus one for the
/// sign when it is signed.
///
/// This code used to size a signed field for the most negative value,
/// its reading of IEEE 1364-2005 §17.1.1.3, which is one digit narrower at
/// 4, 7 or 10 bits; xsim 2026.1 prints `-1` in a 4-bit signed field as
/// ` -1`, three characters, and this follows what it prints.
fn decimal_width(width: u32, signed: bool) -> usize {
    if width == 0 {
        return 1 + usize::from(signed);
    }
    let digits = if width <= 64 {
        let max = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        max.to_string().len()
    } else {
        // 2^n is never a power of ten, so 2^n - 1 has as many digits as
        // 2^n: floor(n * log10(2)) + 1; the product is small and positive.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let d = (f64::from(width) * std::f64::consts::LOG10_2).floor() as usize + 1;
        d
    };
    digits + usize::from(signed)
}

/// Formats a value in decimal per §17.1.1.2: `x`/`z` when all bits are
/// unknown of one kind, `X`/`Z` otherwise.
///
/// A signed value whose sign bit is a known one and which has any unknown
/// bit prints `x`, as xsim prints it: negating it to find the magnitude
/// makes every bit unknown.
fn decimal(l: &Logic) -> String {
    if l.has_unknown() {
        if l.is_signed() && l.msb() == Bit::One {
            return "x".to_owned();
        }
        let bits = l.bits();
        let all_x = bits.iter().all(|b| *b == Bit::X);
        let all_z = bits.iter().all(|b| *b == Bit::Z);
        let any_x = bits.contains(&Bit::X);
        return match (all_x, all_z, any_x) {
            (true, _, _) => "x",
            (_, true, _) => "z",
            (_, _, true) => "X",
            _ => "Z",
        }
        .to_owned();
    }
    if l.is_signed() && l.msb() == Bit::One {
        let mag = l.neg();
        return format!("-{}", decimal_words(mag.value_words()));
    }
    decimal_words(l.value_words())
}

/// Formats a value with `bits` bits per digit (1 binary, 3 octal, 4 hex).
fn radix(l: &Logic, bits: u32, upper: bool) -> String {
    if l.width() == 0 {
        return "0".to_owned();
    }
    let digits = l.width().div_ceil(bits);
    let mut out = String::with_capacity(usize::try_from(digits).unwrap_or(0));
    for d in (0..digits).rev() {
        let lo = d * bits;
        let hi = (lo + bits - 1).min(l.width() - 1);
        let chunk = l.slice(hi, lo);
        let ch = match chunk.to_u64() {
            Some(v) => {
                let c = char::from_digit(u32::try_from(v).expect("digit"), 16).expect("digit");
                if upper { c.to_ascii_uppercase() } else { c }
            }
            None => {
                let all_x = chunk.bits().iter().all(|b| *b == Bit::X);
                let all_z = chunk.bits().iter().all(|b| *b == Bit::Z);
                let any_x = chunk.bits().contains(&Bit::X);
                match (all_x, all_z, any_x) {
                    (true, _, _) => 'x',
                    (_, true, _) => 'z',
                    (_, _, true) => 'X',
                    _ => 'Z',
                }
            }
        };
        out.push(ch);
    }
    out
}

/// C-style `%e` formatting: `d.dddddde+XX`.
fn exponent_format(v: f64, precision: usize, upper: bool) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    let s = format!("{:.*e}", precision, v);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let e = if upper { 'E' } else { 'e' };
    format!(
        "{mant}{e}{}{:02}",
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    )
}

/// The text `%s` prints for a bit vector: eight bits per character, most
/// significant first, an unknown bit read as zero and a NUL byte printed as
/// a space, as xsim prints them. `strip` (a `%0s`) drops the leading NUL
/// bytes instead, which is what a string literal shorter than its
/// variable is padded with.
fn display_string(l: &Logic, strip: bool) -> String {
    let bytes = l.width().div_ceil(8);
    let mut out = String::new();
    for i in (0..bytes).rev() {
        let lo = i * 8;
        let hi = (lo + 7).min(l.width().saturating_sub(1));
        let byte = l
            .slice(hi, lo)
            .bits()
            .iter()
            .rev()
            .fold(0u8, |acc, b| (acc << 1) | u8::from(*b == Bit::One));
        if byte == 0 {
            if !(strip && out.is_empty()) {
                out.push(' ');
            }
        } else {
            out.push(char::from(byte));
        }
    }
    out
}

/// Pads the digits of a `%h`, `%o` or `%b` conversion as xsim does.
///
/// With no width the digits stand as they are, one per digit position of
/// the operand. A width, even zero, first drops the leading zeros; the
/// result is then zero-filled on the left up to the width, or with `-`
/// space-filled on the right. IEEE 1364-2005 §17.1.1.3 says only that
/// `%0h` suppresses the leading zeros; that a nonzero width fills with
/// zeros rather than spaces was measured (`%4h` of `8'h0f` is `000f`).
fn pad_radix(digits: String, width: Option<usize>, left: bool) -> String {
    let Some(w) = width else { return digits };
    let trimmed = digits.trim_start_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    let n = trimmed.chars().count();
    if n >= w {
        return trimmed.to_owned();
    }
    if left {
        format!("{trimmed}{}", " ".repeat(w - n))
    } else {
        format!("{}{trimmed}", "0".repeat(w - n))
    }
}

/// Pads `text` to `width` with zeros on the left, as the zero flag asks
/// (`%05d`, `%05s`, `%08.2f`). xsim puts the zeros before a sign, so `%05d`
/// of -5 is `000-5`.
fn pad_zero(text: String, width: usize) -> String {
    let n = text.chars().count();
    if n >= width {
        return text;
    }
    format!("{}{text}", "0".repeat(width - n))
}

/// Pads with spaces, or with zeros under the zero flag when not left
/// justified.
fn fill(text: String, spec: &Spec, left: bool) -> String {
    match spec.width {
        Some(w) if spec.zero && !left => pad_zero(text, w),
        width => pad(text, width, left),
    }
}

/// A real converted for an integer conversion (`%d`, `%h` of a real): the
/// nearest 32-bit signed integer, halves away from zero. A value outside
/// that range, or NaN, becomes the most negative one, which is what xsim
/// prints for `1e20`.
fn real_as_integer(r: f64) -> Logic {
    let rounded = r.round();
    let v = if rounded.is_finite()
        && rounded >= f64::from(i32::MIN)
        && rounded <= f64::from(i32::MAX)
    {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        let v = rounded as i32;
        v
    } else {
        i32::MIN
    };
    Logic::from_i64(i64::from(v), 32).as_signed()
}

/// Pads `text` to `width`.
fn pad(text: String, width: Option<usize>, left: bool) -> String {
    match width {
        Some(w) if text.chars().count() < w => {
            let fill = " ".repeat(w - text.chars().count());
            if left {
                format!("{text}{fill}")
            } else {
                format!("{fill}{text}")
            }
        }
        _ => text,
    }
}

/// Parses the conversion after a `%`; returns the spec and the number of
/// characters consumed.
fn parse_spec(rest: &[char]) -> Option<(Spec, usize)> {
    let mut i = 0;
    let mut left = false;
    if rest.get(i) == Some(&'-') {
        left = true;
        i += 1;
    }
    // A leading zero followed by more digits is the zero flag; `%0d` alone
    // is a width of zero.
    let zero = rest.get(i) == Some(&'0') && rest.get(i + 1).is_some_and(char::is_ascii_digit);
    if zero {
        i += 1;
    }
    let mut width = None;
    while let Some(c) = rest.get(i).filter(|c| c.is_ascii_digit()) {
        let d = c.to_digit(10).expect("digit") as usize;
        width = Some(width.unwrap_or(0) * 10 + d);
        i += 1;
    }
    let mut precision = None;
    if rest.get(i) == Some(&'.') {
        i += 1;
        let mut p = 0usize;
        while let Some(c) = rest.get(i).filter(|c| c.is_ascii_digit()) {
            p = p * 10 + c.to_digit(10).expect("digit") as usize;
            i += 1;
        }
        precision = Some(p);
    }
    let conv = *rest.get(i)?;
    Some((
        Spec {
            left,
            zero,
            width,
            precision,
            conv,
        },
        i + 1,
    ))
}

/// Xorshift64* step.
fn xorshift(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

impl<'d> Simulator<'d> {
    /// The current time in the instance's time unit, rounded.
    pub(crate) fn module_time(&self, inst: InstId) -> u64 {
        let unit = u128::from(self.instances[inst.idx()].unit_fs);
        let fs = u128::from(self.now) * u128::from(self.precision_fs);
        u64::try_from((fs + unit / 2) / unit).unwrap_or(u64::MAX)
    }

    /// The current time in the instance's time unit as a real.
    fn module_realtime(&self, inst: InstId) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let fs = self.now as f64 * self.precision_fs as f64;
        #[allow(clippy::cast_precision_loss)]
        let unit = self.instances[inst.idx()].unit_fs as f64;
        fs / unit
    }

    /// The next `$urandom` value, from a seeded xorshift generator.
    pub(crate) fn random(&mut self) -> u64 {
        xorshift(&mut self.rng)
    }

    /// Formats one value for conversion `spec`.
    fn format_value(&self, inst: InstId, v: &Value, spec: &Spec) -> String {
        // A real for an integer conversion is first rounded to an
        // `integer`, so it prints in eleven columns like one.
        let integral = |v: &Value| match v {
            Value::Real(r) => real_as_integer(*r),
            other => other.to_logic(),
        };
        let text = match spec.conv.to_ascii_lowercase() {
            'd' | 'i' => {
                let l = integral(v);
                let text = decimal(&l);
                return match spec.width {
                    None => pad(text, Some(decimal_width(l.width(), l.is_signed())), false),
                    // An explicit width does not pad an unknown value:
                    // xsim prints `%5d` of `8'bx` as `x`.
                    Some(_) if l.has_unknown() => text,
                    Some(_) => fill(text, spec, spec.left),
                };
            }
            'h' | 'x' | 'o' | 'b' => {
                let l = integral(v);
                let bits = match spec.conv.to_ascii_lowercase() {
                    'h' | 'x' => 4,
                    'o' => 3,
                    _ => 1,
                };
                // xsim prints `%X` and `%H` in lower case too.
                return pad_radix(radix(&l, bits, false), spec.width, spec.left);
            }
            's' => {
                let text = match v {
                    Value::Str(s) => s.clone(),
                    other => display_string(&other.to_logic(), spec.width == Some(0)),
                };
                // xsim right-justifies a string even after `-`.
                return fill(text, spec, false);
            }
            'c' => {
                let l = v.to_logic();
                let byte = l.resize(8).to_u64().unwrap_or(0);
                let text = char::from(u8::try_from(byte).unwrap_or(b'?')).to_string();
                return pad(text, spec.width, spec.left);
            }
            't' => {
                let unit = u128::from(self.instances[inst.idx()].unit_fs);
                let fs = match v {
                    Value::Real(r) => {
                        #[allow(clippy::cast_precision_loss)]
                        let f = r * unit as f64;
                        if f.is_finite() && f >= 0.0 {
                            // Saturating conversion is the intent.
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            let t = f.round() as u128;
                            t
                        } else {
                            0
                        }
                    }
                    other => u128::from(other.to_logic().to_u64().unwrap_or(0)) * unit,
                };
                let text = self.time_format.format(fs);
                let width = spec.width.or(Some(self.time_format.min_width));
                return pad(text, width.filter(|w| *w > 0), spec.left);
            }
            'e' => exponent_format(v.to_real(), spec.precision.unwrap_or(6), spec.conv == 'E'),
            'f' => format!("{:.*}", spec.precision.unwrap_or(6), v.to_real()),
            'g' => {
                let r = v.to_real();
                let p = spec.precision.unwrap_or(6).max(1);
                let exp = if r == 0.0 {
                    0.0
                } else {
                    r.abs().log10().floor()
                };
                #[allow(clippy::cast_possible_truncation)]
                let exp = exp as i64;
                if exp < -4 || exp >= i64::try_from(p).unwrap_or(i64::MAX) {
                    let s = exponent_format(r, p.saturating_sub(1), spec.conv == 'G');
                    trim_exponent_zeros(&s)
                } else {
                    let decimals =
                        usize::try_from(i64::try_from(p).unwrap_or(0) - 1 - exp).unwrap_or(0);
                    let s = format!("{:.*}", decimals, r);
                    if s.contains('.') {
                        s.trim_end_matches('0').trim_end_matches('.').to_owned()
                    } else {
                        s
                    }
                }
            }
            _ => return format!("%{}", spec.conv),
        };
        // What is left are the real conversions, which xsim right-justifies
        // even after `-` (`%-10.3f` of 3.14159 is `     3.142`).
        fill(text, spec, false)
    }

    /// Expands a format string against `vals`, starting at `next`.
    fn format_string(&self, inst: InstId, fmt: &str, vals: &[Value], next: &mut usize) -> String {
        let chars: Vec<char> = fmt.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c != '%' {
                out.push(c);
                i += 1;
                continue;
            }
            let Some((spec, used)) = parse_spec(&chars[i + 1..]) else {
                out.push('%');
                i += 1;
                continue;
            };
            i += 1 + used;
            match spec.conv {
                '%' => out.push('%'),
                'm' => out.push_str(&pad(
                    self.instances[inst.idx()].path.clone(),
                    spec.width,
                    spec.left,
                )),
                // The library and cell; xsim's library is `work`.
                'l' | 'L' => out.push_str(&pad(
                    format!("work.{}", self.instances[inst.idx()].m.name),
                    spec.width,
                    spec.left,
                )),
                _ => {
                    if let Some(v) = vals.get(*next) {
                        *next += 1;
                        out.push_str(&self.format_value(inst, v, &spec));
                    }
                }
            }
        }
        out
    }

    /// Formats the arguments of a `$display`-family task: string
    /// arguments are format strings, other arguments print in `radix`.
    pub(crate) fn format_values(&self, inst: InstId, vals: &[Value], radix: char) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < vals.len() {
            let v = &vals[i];
            i += 1;
            match v {
                Value::Str(s) => out.push_str(&self.format_string(inst, s, vals, &mut i)),
                other => {
                    let spec = Spec {
                        left: false,
                        zero: false,
                        width: None,
                        precision: None,
                        conv: radix,
                    };
                    out.push_str(&self.format_value(inst, other, &spec));
                }
            }
        }
        out
    }

    /// Evaluates and formats task arguments.
    pub(crate) fn format_args(&mut self, inst: InstId, args: &[ExprId], radix: char) -> String {
        let vals: Vec<Value> = args.iter().map(|a| self.eval(inst, *a)).collect();
        self.format_values(inst, &vals, radix)
    }

    /// Runs the monitor region: `$strobe`s, then `$monitor` if its output
    /// changed.
    pub(crate) fn monitor_region(&mut self) {
        let strobes = std::mem::take(&mut self.strobes);
        for (inst, args, radix) in strobes {
            let text = self.format_args(inst, &args, radix);
            self.output.push_str(&text);
            self.output.push('\n');
        }
        if let Some(mut mon) = self.monitor.take() {
            if mon.enabled {
                // A change of a monitored value prints, and nothing else
                // does: not a time step with other activity, not a
                // variable rewritten with the value it had, not a change
                // of the time alone. The text printed last is not the
                // test, since `$time` in it changes at every step.
                let vals: Vec<Value> = mon.args.iter().map(|a| self.eval(mon.inst, *a)).collect();
                let m = self.instances[mon.inst.idx()].m;
                let key: Vec<Value> = mon
                    .args
                    .iter()
                    .zip(&vals)
                    .filter(|(a, _)| {
                        !matches!(
                            m.exprs.get(**a).map(|e| &e.kind),
                            Some(ExprKind::Call { name, .. })
                                if matches!(name.as_str(), "$time" | "$stime" | "$realtime")
                        )
                    })
                    .map(|(_, v)| v.clone())
                    .collect();
                let changed = match &mon.last {
                    None => true,
                    Some(last) => {
                        last.len() != key.len()
                            || last.iter().zip(&key).any(|(a, b)| !same_value(a, b))
                    }
                };
                if changed {
                    let text = self.format_values(mon.inst, &vals, mon.radix);
                    self.output.push_str(&text);
                    self.output.push('\n');
                    mon.last = Some(key);
                }
            }
            self.monitor = Some(mon);
        }
    }

    /// Warns once per unknown task or function name.
    fn warn_unknown(&mut self, what: &str, name: &str, span: Span) {
        if self.warned_calls.iter().any(|n| n == name) {
            return;
        }
        self.warned_calls.push(name.to_owned());
        self.messages.push(
            Diagnostic::warning(format!("unknown system {what} `{name}` is ignored"))
                .with_span(span),
        );
    }

    /// Executes a system task. Returns `false` when the process must
    /// suspend (`$finish`, `$stop`).
    pub(crate) fn exec_syscall(
        &mut self,
        pid: ProcId,
        inst: InstId,
        name: &str,
        args: &[ExprId],
        span: Span,
    ) -> bool {
        let radix_of = |base: &str| match name.strip_prefix(base) {
            Some("") => Some('d'),
            Some("b") => Some('b'),
            Some("h") => Some('h'),
            Some("o") => Some('o'),
            _ => None,
        };
        if let Some(r) = radix_of("$display") {
            let text = self.format_args(inst, args, r);
            self.output.push_str(&text);
            self.output.push('\n');
            return true;
        }
        if let Some(r) = radix_of("$write") {
            let text = self.format_args(inst, args, r);
            self.output.push_str(&text);
            return true;
        }
        if let Some(r) = radix_of("$strobe") {
            self.strobes.push((inst, args.to_vec(), r));
            return true;
        }
        if let Some(r) = radix_of("$monitor") {
            // A new `$monitor` replaces the one before (§17.1.3); xsim
            // keeps both printing, which is not copied.
            self.monitor = Some(Monitor {
                inst,
                args: args.to_vec(),
                radix: r,
                last: None,
                enabled: true,
            });
            return true;
        }
        match name {
            "$monitoron" | "$monitoroff" => {
                // `$monitoron` prints at once only if a value changed
                // while it was off: what was printed last is still the
                // reference, as xsim keeps it.
                if let Some(m) = &mut self.monitor {
                    m.enabled = name == "$monitoron";
                }
                true
            }
            "$finish" => {
                self.finish();
                self.procs[pid.idx()].status = super::process::ProcStatus::Dead;
                false
            }
            "$stop" => {
                self.stop_at(pid);
                false
            }
            "$fatal" | "$error" | "$warning" | "$info" | "report" => {
                let severity = match name {
                    "$fatal" => ReportSeverity::Failure,
                    "$error" => ReportSeverity::Error,
                    "$warning" => ReportSeverity::Warning,
                    _ => ReportSeverity::Note,
                };
                // `$fatal` may start with a finish number.
                let m = self.instances[inst.idx()].m;
                let skip = usize::from(
                    name == "$fatal"
                        && args.len() >= 2
                        && !matches!(
                            m.exprs.get(args[0]).map(|e| &e.kind),
                            Some(ExprKind::String(_))
                        ),
                );
                let text = self.format_args(inst, &args[skip..], 'd');
                self.report(inst, severity, &text, span);
                true
            }
            "$readmemh" | "$readmemb" | "$writememh" | "$writememb" => {
                // A frontend lowers these to `StmtKind::MemFile`; as a
                // plain call they have no memory to work on.
                self.messages.push(
                    Diagnostic::error(format!(
                        "`{name}` as a system call names no memory; it must be a memory file statement"
                    ))
                    .with_span(span),
                );
                true
            }
            "$timeformat" => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval(inst, *a)).collect();
                let int = |v: Option<&Value>| v.map(|v| v.to_logic().to_i64().unwrap_or(0));
                if let Some(units) = int(vals.first()) {
                    let exp = units.clamp(-15, 0) + 15;
                    self.time_format.unit_fs = 10u64.pow(u32::try_from(exp).unwrap_or(0));
                }
                if let Some(p) = int(vals.get(1)) {
                    self.time_format.precision = u32::try_from(p.clamp(0, 18)).unwrap_or(0);
                }
                if let Some(Value::Str(s)) = vals.get(2) {
                    self.time_format.suffix = s.clone();
                }
                if let Some(w) = int(vals.get(3)) {
                    self.time_format.min_width = usize::try_from(w.max(0)).unwrap_or(0);
                }
                true
            }
            "$dumpvars" => {
                if self.vcd.is_none() {
                    self.enable_vcd();
                }
                true
            }
            "$printtimescale" => {
                let text = self.timescale_text(inst, args);
                self.output.push_str(&text);
                self.output.push('\n');
                true
            }
            "$dumpfile" | "$dumpon" | "$dumpoff" | "$dumpall" | "$dumpflush" | "$fflush" => true,
            _ => {
                self.warn_unknown("task", name, span);
                true
            }
        }
    }

    /// The line `$printtimescale` prints, in xsim's words: `Timescale of
    /// (tb) is 1ns/1ps.` (IEEE 1364-2005 §17.3.1 gives no exact text).
    ///
    /// With no argument the scope is the calling module, named by its
    /// module name as xsim names it; an argument names an instance, by a
    /// path from the calling instance or from the top, and is printed as
    /// its full path. xsim prints an absolute argument `tb.u1` as
    /// `tb.tb.u1`, prefixing the caller's scope, which is not copied.
    fn timescale_text(&mut self, inst: InstId, args: &[ExprId]) -> String {
        let (label, target) = match args.first() {
            None => (self.instances[inst.idx()].m.name.to_string(), inst),
            Some(a) => {
                let written = match self.eval(inst, *a) {
                    Value::Str(s) => s,
                    other => display_string(&other.to_logic(), true),
                };
                let relative = format!("{}.{written}", self.instances[inst.idx()].path);
                let found = self
                    .instances
                    .iter()
                    .position(|i| i.path == relative)
                    .or_else(|| self.instances.iter().position(|i| i.path == written));
                match found {
                    Some(i) => (self.instances[i].path.clone(), InstId::at(i)),
                    None => (written, inst),
                }
            }
        };
        let scale = match self.instances[target.idx()].m.timescale {
            Some(t) => format!(
                "{}{}/{}{}",
                t.unit.value,
                t.unit.unit.name(),
                t.precision.value,
                t.precision.unit.name()
            ),
            None => "1ns/1ps".to_owned(),
        };
        format!("Timescale of ({label}) is {scale}.")
    }

    /// `$readmemh`, `$readmemb`, `$writememh` and `$writememb`
    /// ([`crate::ir::StmtKind::MemFile`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mem_file(
        &mut self,
        inst: InstId,
        op: MemFileOp,
        mem: MemoryId,
        file: ExprId,
        start: Option<ExprId>,
        end: Option<ExprId>,
        base: i64,
        span: Span,
    ) {
        let task = format!("${}", op.keyword());
        let Some(path) = file_name(&self.eval(inst, file)) else {
            self.messages.push(
                Diagnostic::error(format!("{task} needs a file name as first argument"))
                    .with_span(span),
            );
            return;
        };
        let Some(mid) = self.instances[inst.idx()].mems.get(mem.index()).copied() else {
            return;
        };
        let mut bound = |e: Option<ExprId>| -> Result<Option<u64>, ()> {
            match e {
                None => Ok(None),
                Some(e) => self.eval_logic(inst, e).to_u64().map(Some).ok_or(()),
            }
        };
        let (Ok(start), Ok(end)) = (bound(start), bound(end)) else {
            self.messages
                .push(Diagnostic::error(format!("{task}: an address is unknown")).with_span(span));
            return;
        };
        let (name, width, size) = {
            let m = &self.memories[mid.idx()];
            (
                m.name.clone(),
                m.elem_width,
                u64::try_from(m.data.len()).unwrap_or(u64::MAX),
            )
        };
        if !op.is_read() {
            let (first, last) = match memfile::window(size, start, end) {
                Ok(w) => w,
                Err(e) => {
                    self.messages
                        .push(Diagnostic::error(format!("{task}: {e}")).with_span(span));
                    return;
                }
            };
            let data = &self.memories[mid.idx()].data;
            let words: Vec<(u64, crate::logic::Logic)> = walk(first, last)
                .filter_map(|a| {
                    let w = data.get(usize::try_from(a).ok()?)?;
                    Some((a, w.clone()))
                })
                .collect();
            let text = memfile::render(&words, op.is_hex(), base);
            self.written.insert(path, text);
            return;
        }
        let text = match self.written.get(&path) {
            Some(text) => Some(text.clone()),
            None => self.options.files.as_ref().and_then(|f| f.read_file(&path)),
        };
        let Some(text) = text else {
            self.messages
                .push(Diagnostic::error(format!("{task}: cannot read `{path}`")).with_span(span));
            return;
        };
        match memfile::load(&text, op.is_hex(), width, size, start, end, base) {
            Ok(loaded) => {
                for (addr, value) in loaded.words {
                    self.write_mem(mid, addr, &value);
                }
                for w in loaded.warnings {
                    self.messages.push(
                        Diagnostic::warning(format!("{task}: `{path}` into `{name}`: {w}"))
                            .with_span(span),
                    );
                }
            }
            Err(e) => self.messages.push(
                Diagnostic::error(format!("{task}: `{path}` into `{name}`: {e}")).with_span(span),
            ),
        }
    }

    /// Evaluates a system function call.
    pub(crate) fn call_function(
        &mut self,
        inst: InstId,
        name: &str,
        args: Vec<Value>,
        ty: &Type,
        span: Span,
    ) -> Value {
        let width = flat_width(ty);
        let fit = |l: Logic| -> Value {
            if width == 0 || l.width() == width {
                Value::Bits(l)
            } else {
                Value::Bits(l.resize(width))
            }
        };
        match name {
            "$time" | "$stime" => fit(Logic::from_u64(self.module_time(inst), 64)),
            "$realtime" => Value::Real(self.module_realtime(inst)),
            "$random" if args.is_empty() => {
                let r = super::random::random(&mut self.random_seed);
                fit(Logic::from_i64(i64::from(r), 32).as_signed())
            }
            seeded if seeded.starts_with("$random") || seeded.starts_with("$dist_") => {
                // `$random(seed)` and `$dist_*(seed, ...)`. A frontend
                // that can write the seed back calls the `$seed` form,
                // which returns the new seed above the value.
                let (base, write_back) = match seeded.strip_suffix("$seed") {
                    Some(base) => (base, true),
                    None => (seeded, false),
                };
                let ints: Vec<i32> = args
                    .iter()
                    .map(|v| {
                        let l = match v {
                            Value::Real(r) => real_as_integer(*r),
                            other => other.to_logic(),
                        };
                        // The low 32 bits, as C's `long` holds them.
                        #[allow(clippy::cast_possible_truncation)]
                        let i = l.resize(32).as_signed().to_i64().unwrap_or(0) as i32;
                        i
                    })
                    .collect();
                let mut seed = ints.first().copied().unwrap_or(0);
                let Some((value, warning)) =
                    super::random::dist(base, &mut seed, ints.get(1..).unwrap_or(&[]))
                else {
                    self.warn_unknown("function", name, span);
                    return Value::Bits(Logic::x(width.max(1)));
                };
                if let Some(w) = warning {
                    self.messages.push(Diagnostic::warning(w).with_span(span));
                }
                // xsim's seedless `$random` goes on from where the last
                // `$random(seed)` or `$dist_uniform` left its seed; the
                // other distributions leave it alone (measured).
                if matches!(base, "$random" | "$dist_uniform") && warning.is_none() {
                    self.random_seed = seed;
                }
                let value = Logic::from_i64(i64::from(value), 32).as_signed();
                if write_back {
                    let seed = Logic::from_i64(i64::from(seed), 32);
                    return Value::Bits(Logic::concat_all([&seed, &value.as_unsigned()]));
                }
                fit(value)
            }
            "$urandom" => fit(Logic::from_u64(self.random() & 0xFFFF_FFFF, 32)),
            "$urandom_range" => {
                let a = args
                    .first()
                    .and_then(|v| v.to_logic().to_u64())
                    .unwrap_or(0);
                let b = args.get(1).and_then(|v| v.to_logic().to_u64()).unwrap_or(0);
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                let span_ = hi - lo + 1;
                let r = if span_ == 0 {
                    self.random()
                } else {
                    lo + self.random() % span_
                };
                fit(Logic::from_u64(r, 32))
            }
            "$clog2" => {
                let v = args.first().map(|v| v.to_logic());
                let n =
                    match v {
                        Some(l) if l.is_fully_known() => {
                            let words = l.value_words();
                            let top = words.iter().enumerate().rev().find(|(_, w)| **w != 0).map(
                                |(i, w)| {
                                    u64::try_from(i).unwrap_or(0) * 64
                                        + u64::from(64 - w.leading_zeros())
                                },
                            );
                            match top {
                                None => 0,
                                Some(bits) => {
                                    // clog2(n) = bits(n - 1); n is a power of two iff one bit set.
                                    let ones: u32 = words.iter().map(|w| w.count_ones()).sum();
                                    if ones == 1 { bits - 1 } else { bits }
                                }
                            }
                        }
                        _ => return Value::Bits(Logic::x(width.max(1))),
                    };
                fit(Logic::from_u64(n, 32))
            }
            "$bits" => fit(Logic::from_u64(
                u64::from(args.first().map_or(0, Value::width)),
                32,
            )),
            "$itor" => Value::Real(args.first().map_or(0.0, Value::to_real)),
            real if crate::logic::REAL_FUNCTIONS.contains(&real.trim_start_matches('$'))
                && real.starts_with('$') =>
            {
                let reals: Vec<f64> = args.iter().map(Value::to_real).collect();
                Value::Real(
                    crate::logic::real_function(real.trim_start_matches('$'), &reals)
                        .unwrap_or(f64::NAN),
                )
            }
            "$rtoi" => {
                let r = args.first().map_or(0.0, Value::to_real);
                fit(Logic::from_i64(
                    Value::Real(r.trunc()).to_logic().to_i64().unwrap_or(0),
                    64,
                ))
            }
            "$realtobits" => fit(Logic::from_u64(
                args.first().map_or(0.0, Value::to_real).to_bits(),
                64,
            )),
            "$bitstoreal" => Value::Real(
                args.first()
                    .and_then(|v| v.to_logic().to_u64())
                    .map_or(f64::NAN, f64::from_bits),
            ),
            "$signed" => fit(args
                .first()
                .map_or_else(|| Logic::x(1), Value::to_logic)
                .as_signed()),
            "$unsigned" => fit(args
                .first()
                .map_or_else(|| Logic::x(1), Value::to_logic)
                .as_unsigned()),
            "$sformatf" => Value::Str(self.format_values(inst, &args, 'd')),
            _ => {
                self.warn_unknown("function", name, span);
                Value::Bits(Logic::x(width.max(1)))
            }
        }
    }
}

/// The file name a `$readmem*` argument holds: a string, or the ASCII
/// text of a bit vector (a string parameter or a `reg` holding one).
fn file_name(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Bits(l) => memfile::name_from_bits(l),
        _ => None,
    }
}

/// The addresses from `first` to `last` inclusive, downwards when `last`
/// is the smaller.
fn walk(first: u64, last: u64) -> Box<dyn Iterator<Item = u64>> {
    if first <= last {
        Box::new(first..=last)
    } else {
        Box::new((last..=first).rev())
    }
}

/// Removes trailing zeros of the mantissa in an exponent form.
fn trim_exponent_zeros(s: &str) -> String {
    match s.split_once(['e', 'E']) {
        Some((mant, exp)) if mant.contains('.') => {
            let mant = mant.trim_end_matches('0').trim_end_matches('.');
            let e = if s.contains('E') { 'E' } else { 'e' };
            format!("{mant}{e}{exp}")
        }
        _ => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(s: &str) -> Logic {
        Logic::parse_verilog(s).unwrap()
    }

    #[test]
    fn decimal_and_radix() {
        assert_eq!(decimal(&l("8'd200")), "200");
        assert_eq!(decimal(&Logic::from_i64(-5, 8)), "-5");
        assert_eq!(decimal(&l("8'hxx")), "x");
        assert_eq!(decimal(&l("8'hzz")), "z");
        assert_eq!(decimal(&l("8'b0000x000")), "X");
        assert_eq!(decimal(&l("8'b0000z000")), "Z");
        assert_eq!(decimal(&Logic::ones(70)), "1180591620717411303423");
        assert_eq!(decimal(&Logic::zero(0)), "0");
        assert_eq!(radix(&l("8'hab"), 4, false), "ab");
        assert_eq!(radix(&l("8'hab"), 4, true), "AB");
        assert_eq!(radix(&l("8'b1010x0zz"), 4, false), "aX");
        assert_eq!(radix(&l("8'hxx"), 4, false), "xx");
        assert_eq!(radix(&l("8'hzz"), 4, false), "zz");
        assert_eq!(radix(&l("8'bzzzzzz0z"), 4, false), "zZ");
        assert_eq!(radix(&l("7'd127"), 3, false), "177");
        assert_eq!(radix(&l("3'b1x0"), 1, false), "1x0");
        assert_eq!(radix(&Logic::zero(0), 4, false), "0");
        assert_eq!(decimal_width(8, false), 3);
        assert_eq!(decimal_width(8, true), 4);
        assert_eq!(decimal_width(64, false), 20);
        assert_eq!(decimal_width(0, false), 1);
        assert_eq!(decimal_width(100, false), 31);
    }

    /// The field widths and unknown-bit spellings xsim prints, which are
    /// not the ones IEEE 1364-2005 §17.1.1.3 reads as: a signed field is
    /// sized from 2^N - 1, not from -2^(N-1), and a negative signed value
    /// with an unknown bit is `x`. Would catch the standard's narrower
    /// widths back (4, 7 and 10 bits are where they differ) or the sign
    /// test dropped; would not catch a wrong width above 65 bits, which
    /// xsim was not asked.
    #[test]
    fn signed_decimal_fields_are_xsims() {
        let widths: Vec<usize> = [1, 2, 4, 7, 8, 10, 32, 64, 65]
            .iter()
            .map(|w| decimal_width(*w, true))
            .collect();
        assert_eq!(widths, [2, 2, 3, 4, 4, 5, 11, 21, 21]);
        assert_eq!(decimal(&l("8'sb1000000z")), "x");
        assert_eq!(decimal(&l("8'sb1xxxxxxx")), "x");
        assert_eq!(decimal(&l("8'sb0000000z")), "Z");
        assert_eq!(decimal(&l("8'sbx0000000")), "X");
        assert_eq!(decimal(&l("8'b1000000z")), "Z");
    }

    /// `%h`, `%o` and `%b` with and without widths, `%s` of bit vectors,
    /// the zero flag and a real in an integer conversion, against what
    /// xsim printed. Would catch leading zeros kept under `%0h`, spaces
    /// instead of zeros under `%4h`, a NUL byte dropped or printed as `?`,
    /// an unknown bit in a character, `-` lost on a zero-filled number, or
    /// a real out of the 32-bit range wrapped; would not catch how these
    /// are chosen from the conversion character, which
    /// `display_formats_as_xsim_does` covers end to end.
    #[test]
    fn padding_and_strings_are_xsims() {
        assert_eq!(pad_radix("05".into(), None, false), "05");
        assert_eq!(pad_radix("05".into(), Some(0), false), "5");
        assert_eq!(pad_radix("00000000".into(), Some(0), false), "0");
        assert_eq!(pad_radix("0f".into(), Some(3), false), "00f");
        assert_eq!(pad_radix("0f".into(), Some(4), true), "f   ");
        assert_eq!(pad_radix("017".into(), Some(2), false), "17");
        assert_eq!(pad_radix("0Xx".into(), Some(0), false), "Xx");
        assert_eq!(pad_radix("ab".into(), Some(1), false), "ab");
        assert_eq!(display_string(&l("32'h00006162"), false), "  ab");
        assert_eq!(display_string(&l("32'h00006162"), true), "ab");
        assert_eq!(display_string(&l("32'h41004200"), true), "A B ");
        assert_eq!(display_string(&l("8'bxxxxxxx1"), false), "\u{1}");
        assert_eq!(display_string(&l("16'h41xx"), false), "A ");
        assert_eq!(pad_zero("7".into(), 5), "00007");
        assert_eq!(pad_zero("-5".into(), 5), "000-5");
        assert_eq!(pad_zero("123".into(), 2), "123");
        assert_eq!(real_as_integer(2.5), Logic::from_i64(3, 32).as_signed());
        assert_eq!(real_as_integer(-2.5), Logic::from_i64(-3, 32).as_signed());
        assert_eq!(real_as_integer(-0.5), Logic::from_i64(-1, 32).as_signed());
        assert_eq!(
            real_as_integer(1e20),
            Logic::from_i64(-(1 << 31), 32).as_signed()
        );
        let (spec, _) = parse_spec(&['0', '5', 'd']).unwrap();
        assert!(spec.zero);
        assert_eq!(spec.width, Some(5));
        let (spec, _) = parse_spec(&['0', 'd']).unwrap();
        assert!(!spec.zero);
        assert_eq!(spec.width, Some(0));
    }

    #[test]
    fn formats() {
        assert_eq!(exponent_format(1234.5, 2, false), "1.23e+03");
        assert_eq!(exponent_format(0.00125, 3, true), "1.250E-03");
        assert_eq!(exponent_format(f64::INFINITY, 3, true), "inf");
        assert_eq!(trim_exponent_zeros("1.2500e+03"), "1.25e+03");
        assert_eq!(trim_exponent_zeros("12"), "12");
        assert_eq!(pad("ab".into(), Some(4), false), "  ab");
        assert_eq!(pad("ab".into(), Some(4), true), "ab  ");
        assert_eq!(pad("ab".into(), Some(1), true), "ab");
        let (spec, used) = parse_spec(&['-', '1', '2', '.', '3', 'f', 'x']).unwrap();
        assert!(spec.left);
        assert_eq!(spec.width, Some(12));
        assert_eq!(spec.precision, Some(3));
        assert_eq!(spec.conv, 'f');
        assert_eq!(used, 6);
        assert!(parse_spec(&[]).is_none());
        let tf = TimeFormat {
            unit_fs: 1_000_000,
            precision: 3,
            suffix: " ns".into(),
            min_width: 0,
        };
        assert_eq!(tf.format(1_234_500_000), "1234.500 ns");
        assert_eq!(TimeFormat::for_precision(1000).format(5000), "5");
    }

    #[test]
    fn random_is_deterministic() {
        let mut a = 42u64;
        let mut b = 42u64;
        assert_eq!(xorshift(&mut a), xorshift(&mut b));
        assert_ne!(xorshift(&mut a), xorshift(&mut a));
    }
}
