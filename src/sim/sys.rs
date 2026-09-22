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
//! | `$monitor*`, `$monitoron`, `$monitoroff` | Output whenever an argument changes, once per slot (§17.1.3) |
//! | `$finish`, `$stop` | End or pause the run (§17.4) |
//! | `$fatal` `$error` `$warning` `$info`, `report` | Severity reports; `$fatal` ends the run |
//! | `$readmemh` `$readmemb` | Load a memory from a [`FileProvider`] (§17.2.8); the memory is passed as an `@mem[0]` read |
//! | `$timeformat` | Sets the `%t` format (§17.3.2) |
//! | `$dumpvars` `$dumpfile` `$dumpon` `$dumpoff` `$dumpall` | `$dumpvars` enables VCD capture; the rest are accepted and ignored |
//! | `$time` `$stime` `$realtime` | The current time in the module's unit (§17.7) |
//! | `$random` `$urandom` `$urandom_range` | A seeded xorshift generator (§17.9.1); deterministic per seed |
//! | `$clog2` `$bits` `$itor` `$rtoi` `$realtobits` `$bitstoreal` `$sformatf` | Utility functions |
//!
//! Formatting understands `%d %i %h %x %o %b %s %c %t %m %e %f %g %%`
//! with an optional `-`, width and precision; the default widths are the
//! standard's (decimal digits for the operand width, one hex digit per
//! nibble, one bit per `%b` character). Unknown tasks are warned about once
//! and ignored; unknown functions return `x` of their cached type.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::diag::Diagnostic;
use crate::ir::{ExprId, ExprKind, ReportSeverity, Span, Type};
use crate::logic::{Bit, Logic};

use super::Simulator;
use super::elab::{InstId, MemId, ProcId};
use super::value::{Value, flat_width, logic_to_string};

/// Supplies file contents to `$readmemh` / `$readmemb` without the
/// simulator touching the filesystem.
pub trait FileProvider {
    /// The text of the file named `path`, or `None` when it does not exist.
    fn read_file(&self, path: &str) -> Option<String>;
}

/// An in-memory [`FileProvider`]: a map from names to contents.
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles {
    files: BTreeMap<String, String>,
}

impl MemoryFiles {
    /// An empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a file.
    pub fn insert(&mut self, name: impl Into<String>, text: impl Into<String>) -> &mut Self {
        self.files.insert(name.into(), text.into());
        self
    }
}

impl FileProvider for MemoryFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
}

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
    /// The last text printed, to print only on change.
    pub(crate) last: Option<String>,
    /// `$monitoroff` suppresses output.
    pub(crate) enabled: bool,
}

/// A parsed `%` conversion.
struct Spec {
    left: bool,
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

/// The number of decimal digits needed for the largest value of `width`
/// bits (with a sign for signed values).
fn decimal_width(width: u32, signed: bool) -> usize {
    let magnitude = if signed {
        width.saturating_sub(1)
    } else {
        width
    };
    if magnitude == 0 {
        return 1 + usize::from(signed);
    }
    let digits = if magnitude <= 64 {
        let max = if magnitude == 64 {
            u64::MAX
        } else {
            (1u64 << magnitude) - 1
        };
        max.to_string().len()
    } else {
        // ceil(bits * log10(2)); the product is small and positive.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let d = (f64::from(magnitude) * std::f64::consts::LOG10_2).ceil() as usize;
        d
    };
    digits + usize::from(signed)
}

/// Formats a value in decimal per §17.1.1.2: `x`/`z` when all bits are
/// unknown of one kind, `X`/`Z` otherwise.
fn decimal(l: &Logic) -> String {
    if l.has_unknown() {
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

/// Parses `$readmem` text into `(address, value)` pairs.
///
/// Whitespace-separated tokens; `//` and `/* */` comments; `@hex` sets the
/// address; digits may contain `_`, `x` and `z`.
pub(crate) fn parse_readmem(
    text: &str,
    hex: bool,
    width: u32,
) -> Result<Vec<(u64, Logic)>, String> {
    let mut out = Vec::new();
    let mut addr = 0u64;
    let mut chars = text.chars().peekable();
    let mut token = String::new();
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let flush =
        |token: &mut String, addr: &mut u64, out: &mut Vec<(u64, Logic)>| -> Result<(), String> {
            if token.is_empty() {
                return Ok(());
            }
            if let Some(a) = token.strip_prefix('@') {
                *addr = u64::from_str_radix(&a.replace('_', ""), 16)
                    .map_err(|_| format!("bad address `{token}`"))?;
            } else {
                let literal = format!(
                    "{}'{}{}",
                    width,
                    if hex { 'h' } else { 'b' },
                    token.replace('_', "")
                );
                let value = Logic::parse_verilog(&literal)
                    .map_err(|e| format!("bad value `{token}`: {e}"))?;
                out.push((*addr, value));
                *addr += 1;
            }
            token.clear();
            Ok(())
        };
    while let Some(c) = chars.next() {
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            flush(&mut token, &mut addr, &mut out)?;
            in_line_comment = true;
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            flush(&mut token, &mut addr, &mut out)?;
            in_block_comment = true;
            continue;
        }
        if c.is_whitespace() {
            flush(&mut token, &mut addr, &mut out)?;
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut addr, &mut out)?;
    Ok(out)
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

    /// The next `$random` value.
    pub(crate) fn random(&mut self) -> u64 {
        xorshift(&mut self.rng)
    }

    /// Formats one value for conversion `spec`.
    fn format_value(&self, inst: InstId, v: &Value, spec: &Spec) -> String {
        let text = match spec.conv.to_ascii_lowercase() {
            'd' | 'i' => {
                let l = v.to_logic();
                let text = decimal(&l);
                let width = match spec.width {
                    Some(w) => Some(w),
                    None => Some(decimal_width(l.width(), l.is_signed())),
                };
                return pad(text, width.filter(|w| *w > 0), spec.left);
            }
            'h' | 'x' | 'o' | 'b' => {
                let l = v.to_logic();
                let bits = match spec.conv.to_ascii_lowercase() {
                    'h' | 'x' => 4,
                    'o' => 3,
                    _ => 1,
                };
                let text = radix(&l, bits, spec.conv.is_ascii_uppercase());
                let default = l.width().div_ceil(bits);
                let width = spec.width.or(Some(usize::try_from(default).unwrap_or(0)));
                return pad(text, width.filter(|w| *w > 0), spec.left);
            }
            's' => match v {
                Value::Str(s) => s.clone(),
                other => logic_to_string(&other.to_logic()),
            },
            'c' => {
                let l = v.to_logic();
                let byte = l.resize(8).to_u64().unwrap_or(0);
                char::from(u8::try_from(byte).unwrap_or(b'?')).to_string()
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
            _ => format!("%{}", spec.conv),
        };
        pad(text, spec.width, spec.left)
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
        for (inst, args) in strobes {
            let text = self.format_args(inst, &args, 'd');
            self.output.push_str(&text);
            self.output.push('\n');
        }
        if let Some(mon) = self.monitor.take() {
            let mut mon = mon;
            if mon.enabled {
                let text = self.format_args(mon.inst, &mon.args, 'd');
                if mon.last.as_deref() != Some(text.as_str()) {
                    self.output.push_str(&text);
                    self.output.push('\n');
                    mon.last = Some(text);
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

    /// The memory an argument designates: a `MemRead` of it.
    fn mem_arg(&self, inst: InstId, e: ExprId) -> Option<MemId> {
        let state = &self.instances[inst.idx()];
        match state.m.exprs.get(e)?.kind {
            ExprKind::MemRead { mem, .. } => state.mems.get(mem.index()).copied(),
            _ => None,
        }
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
        if radix_of("$strobe").is_some() {
            self.strobes.push((inst, args.to_vec()));
            return true;
        }
        if radix_of("$monitor").is_some() {
            self.monitor = Some(Monitor {
                inst,
                args: args.to_vec(),
                last: None,
                enabled: true,
            });
            return true;
        }
        match name {
            "$monitoron" | "$monitoroff" => {
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
            "$readmemh" | "$readmemb" => {
                self.readmem(inst, name == "$readmemh", args, span);
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
            "$dumpfile" | "$dumpon" | "$dumpoff" | "$dumpall" | "$dumpflush" | "$fflush"
            | "$printtimescale" => true,
            _ => {
                self.warn_unknown("task", name, span);
                true
            }
        }
    }

    /// `$readmemh` / `$readmemb`.
    fn readmem(&mut self, inst: InstId, hex: bool, args: &[ExprId], span: Span) {
        let Some(Value::Str(path)) = args.first().map(|a| self.eval(inst, *a)) else {
            self.messages.push(
                Diagnostic::error("$readmem needs a file name as first argument").with_span(span),
            );
            return;
        };
        let Some(mem) = args.get(1).and_then(|a| self.mem_arg(inst, *a)) else {
            self.messages.push(
                Diagnostic::error("$readmem needs a memory (`@mem[0]`) as second argument")
                    .with_span(span),
            );
            return;
        };
        let start = args.get(2).and_then(|a| self.eval_logic(inst, *a).to_u64());
        let end = args.get(3).and_then(|a| self.eval_logic(inst, *a).to_u64());
        let text = self.options.files.as_ref().and_then(|f| f.read_file(&path));
        let Some(text) = text else {
            self.messages
                .push(Diagnostic::error(format!("$readmem: cannot read `{path}`")).with_span(span));
            return;
        };
        let width = self.memories[mem.idx()].elem_width;
        match parse_readmem(&text, hex, width) {
            Ok(entries) => {
                let base = start.unwrap_or(0);
                for (offset, value) in entries {
                    let addr = base.saturating_add(offset);
                    if end.is_some_and(|e| addr > e) {
                        break;
                    }
                    if !self.write_mem(mem, addr, &value) {
                        self.messages.push(
                            Diagnostic::warning(format!(
                                "$readmem: address {addr} is outside `{}`",
                                self.memories[mem.idx()].name
                            ))
                            .with_span(span),
                        );
                        break;
                    }
                }
            }
            Err(e) => self
                .messages
                .push(Diagnostic::error(format!("$readmem: `{path}`: {e}")).with_span(span)),
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
            "$random" => {
                let r = self.random();
                fit(Logic::from_i64((r & 0xFFFF_FFFF).cast_signed(), 32).as_signed())
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
    fn readmem_parsing() {
        let text = "// header\n@2 ab cd /* skip */ 1_0\n@0 ff\n";
        let entries = parse_readmem(text, true, 8).unwrap();
        assert_eq!(
            entries,
            vec![
                (2, l("8'hab")),
                (3, l("8'hcd")),
                (4, l("8'h10")),
                (0, l("8'hff"))
            ]
        );
        let bin = parse_readmem("1x 01", false, 2).unwrap();
        assert_eq!(bin, vec![(0, l("2'b1x")), (1, l("2'b01"))]);
        assert!(parse_readmem("@zz", true, 8).is_err());
        assert!(parse_readmem("gg", true, 8).is_err());
    }

    #[test]
    fn random_is_deterministic() {
        let mut a = 42u64;
        let mut b = 42u64;
        assert_eq!(xorshift(&mut a), xorshift(&mut b));
        assert_ne!(xorshift(&mut a), xorshift(&mut a));
    }

    #[test]
    fn memory_files() {
        let mut files = MemoryFiles::new();
        files.insert("a.hex", "00");
        assert_eq!(files.read_file("a.hex").as_deref(), Some("00"));
        assert_eq!(files.read_file("b.hex"), None);
    }
}
