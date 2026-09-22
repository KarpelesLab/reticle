//! Simulation values and net storage.
//!
//! Every net is stored as one flat [`Logic`] vector: a bit vector as is, an
//! unpacked array with element `i` at bits `[i * elem_width +: elem_width]`
//! (nested arrays flatten recursively), an `Integer` as 64 signed bits and a
//! `Real` as the 64 bits of its IEEE 754 encoding. Expression evaluation
//! works on [`Value`], which adds `Real` and `Str` variants for the
//! simulation-only types; converting between the two happens at net reads
//! and writes ([`Value::to_logic`] and the read path in `eval.rs`).
//!
//! There is no separate two-state fast path yet: `Logic` is uniform and the
//! event-driven simulator spends its time in scheduling, not in bit
//! operations. The cycle-based mode planned in `ROADMAP.md` is where such a
//! path pays off.
//!
//! [`resolve_wire`] implements the Verilog `wire` resolution used when a
//! net has several continuous drivers (IEEE 1364-2005 §4.6.1, table 4-6):
//! `z` yields to the other driver, agreeing values win, and `0` against
//! `1` (or any `x`) gives `x`.

use crate::ir::{NetKind, Type};
use crate::logic::{Bit, Logic};

use super::elab::{MemId, SigId};

/// A value produced by expression evaluation.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A four-state bit vector, the common case.
    Bits(Logic),
    /// A floating point value (`Type::Real`).
    Real(f64),
    /// A string literal or a string-typed value.
    Str(String),
}

impl Value {
    /// The value as a bit vector: reals round to a 64-bit signed integer
    /// and strings pack one byte per character, last character least
    /// significant, as Verilog string literals do.
    pub fn to_logic(&self) -> Logic {
        match self {
            Value::Bits(l) => l.clone(),
            Value::Real(r) => Logic::from_i64(real_to_i64(*r), 64),
            Value::Str(s) => string_to_logic(s),
        }
    }

    /// The value as a real: bit vectors convert through their integer
    /// value (`x` and `z` give NaN).
    pub fn to_real(&self) -> f64 {
        match self {
            Value::Bits(l) => logic_to_real(l),
            Value::Real(r) => *r,
            Value::Str(_) => f64::NAN,
        }
    }

    /// The truth value used by conditions.
    pub fn truth(&self) -> Bit {
        match self {
            Value::Bits(l) => l.truth(),
            Value::Real(r) => Bit::from_bool(*r != 0.0),
            Value::Str(s) => Bit::from_bool(!s.is_empty()),
        }
    }

    /// The width in bits after [`Value::to_logic`].
    pub fn width(&self) -> u32 {
        match self {
            Value::Bits(l) => l.width(),
            Value::Real(_) => 64,
            Value::Str(s) => saturating_width(s.len().saturating_mul(8)),
        }
    }
}

impl From<Logic> for Value {
    fn from(l: Logic) -> Self {
        Value::Bits(l)
    }
}

/// A real rounded to the nearest integer, saturating at the `i64` range;
/// NaN becomes 0.
fn real_to_i64(r: f64) -> i64 {
    if r.is_nan() {
        return 0;
    }
    // `as` saturates for floats, which is the documented intent here.
    #[allow(clippy::cast_possible_truncation)]
    let v = r.round() as i64;
    v
}

/// A bit vector as a real: its (signed or unsigned) integer value, or NaN
/// when any bit is unknown.
pub(crate) fn logic_to_real(l: &Logic) -> f64 {
    if l.has_unknown() {
        return f64::NAN;
    }
    if l.is_signed() {
        if let Some(v) = l.to_i64() {
            // Precision loss above 2^53 is inherent to the conversion.
            #[allow(clippy::cast_precision_loss)]
            return v as f64;
        }
    } else if let Some(v) = l.to_u64() {
        #[allow(clippy::cast_precision_loss)]
        return v as f64;
    }
    // Wider than 64 bits: accumulate word by word.
    let mut acc = 0.0f64;
    for w in l.value_words().iter().rev() {
        #[allow(clippy::cast_precision_loss)]
        let word = *w as f64;
        acc = acc * 18_446_744_073_709_551_616.0 + word;
    }
    acc
}

/// Packs a string as a bit vector, one byte per character, the last
/// character in the low bits.
pub(crate) fn string_to_logic(s: &str) -> Logic {
    let bytes = s.as_bytes();
    let bits: Vec<Logic> = bytes
        .iter()
        .map(|b| Logic::from_u64(u64::from(*b), 8))
        .collect();
    Logic::concat_all(bits.iter())
}

/// Unpacks a bit vector as a string, eight bits per character, dropping
/// leading NUL bytes and mapping unknown bytes to `?`.
pub(crate) fn logic_to_string(l: &Logic) -> String {
    let bytes = l.width().div_ceil(8);
    let mut out = String::new();
    for i in (0..bytes).rev() {
        let lo = i * 8;
        let hi = (lo + 7).min(l.width().saturating_sub(1));
        let byte = l.slice(hi, lo);
        match byte.to_u64() {
            Some(0) if out.is_empty() => {}
            Some(v) => out.push(char::from(u8::try_from(v).unwrap_or(b'?'))),
            None => out.push('?'),
        }
    }
    out
}

/// Narrows a bit count to `u32`, saturating.
pub(crate) fn saturating_width(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The number of bits a value of type `ty` occupies in net storage.
pub(crate) fn flat_width(ty: &Type) -> u32 {
    match ty {
        Type::Bits { width, .. } => *width,
        Type::Array { elem, len } => {
            let elem = u64::from(flat_width(elem));
            u32::try_from(elem.saturating_mul(*len)).unwrap_or(u32::MAX)
        }
        Type::Integer | Type::Real => 64,
        Type::String => 0,
    }
}

/// The width of one element of an array type, or 1 for a bit vector (a
/// bit-select), used to scale indices and slices.
pub(crate) fn elem_width(ty: &Type) -> u32 {
    match ty {
        Type::Array { elem, .. } => flat_width(elem),
        _ => 1,
    }
}

/// The number of indexable elements of a type: array length or bit width.
pub(crate) fn elem_count(ty: &Type) -> u64 {
    match ty {
        Type::Array { len, .. } => *len,
        other => u64::from(flat_width(other)),
    }
}

/// `width` bits of `base` starting at bit `lo` (which may be negative or
/// past the end); bits outside `base` read as `x`, as Verilog prescribes
/// for out-of-range selects (IEEE 1364-2005 §5.2.1).
pub(crate) fn select_bits(base: &Logic, lo: i64, width: u32) -> Logic {
    if width == 0 {
        return Logic::zero(0);
    }
    let base_w = i64::from(base.width());
    let hi = lo + i64::from(width) - 1;
    if lo >= 0 && hi < base_w {
        let lo = u32::try_from(lo).expect("checked non-negative");
        return base.slice(lo + width - 1, lo);
    }
    if hi < 0 || lo >= base_w {
        return Logic::x(width);
    }
    let in_lo = lo.max(0);
    let in_hi = hi.min(base_w - 1);
    let inner = base.slice(
        u32::try_from(in_hi).expect("in range"),
        u32::try_from(in_lo).expect("in range"),
    );
    let below = u32::try_from(in_lo - lo).expect("non-negative");
    let above = u32::try_from(hi - in_hi).expect("non-negative");
    Logic::x(above).concat(&inner).concat(&Logic::x(below))
}

/// Replaces bits `[lo, lo + part.width())` of `base` with `part`, ignoring
/// the bits of `part` that fall outside `base`.
pub(crate) fn replace_bits(base: &Logic, lo: u32, part: &Logic) -> Logic {
    let mut out = base.clone();
    for i in 0..part.width() {
        let at = lo.saturating_add(i);
        if at < out.width() {
            out.set_bit(at, part.bit(i));
        }
    }
    out
}

/// Merges two values bit by bit: positions that agree keep their value,
/// the rest become `x`. Used for a conditional operator with an unknown
/// condition (IEEE 1364-2005 §5.1.13).
pub(crate) fn merge_unknown(a: &Logic, b: &Logic) -> Logic {
    let width = a.width().max(b.width());
    let a = a.resize(width);
    let b = b.resize(width);
    let words = a.value_words().len();
    let mut value = Vec::with_capacity(words);
    let mut unknown = Vec::with_capacity(words);
    for i in 0..words {
        let (av, au) = (a.value_words()[i], a.unknown_words()[i]);
        let (bv, bu) = (b.value_words()[i], b.unknown_words()[i]);
        let differ = (av ^ bv) | au | bu;
        value.push(av & !differ);
        unknown.push(differ);
    }
    Logic::from_planes(width, a.is_signed() && b.is_signed(), value, unknown)
}

/// Resolves two driver contributions of equal width with Verilog `wire`
/// semantics: `z` yields, agreement wins, conflict and `x` give `x`.
pub(crate) fn resolve_wire(a: &Logic, b: &Logic) -> Logic {
    debug_assert_eq!(a.width(), b.width());
    let words = a.value_words().len();
    let mut value = Vec::with_capacity(words);
    let mut unknown = Vec::with_capacity(words);
    for i in 0..words {
        let (av, au) = (a.value_words()[i], a.unknown_words()[i]);
        let (bv, bu) = (b.value_words()[i], b.unknown_words()[i]);
        let az = au & av;
        let bz = bu & bv;
        let ax = au & !av;
        let bx = bu & !bv;
        let use_b = az & !bz;
        let use_a = bz & !az;
        let both_z = az & bz;
        let rest = !(az | bz);
        let any_x = ax | bx;
        let differ = av ^ bv;
        let u = (use_b & bu) | (use_a & au) | both_z | (rest & (any_x | differ));
        let v = (use_b & bv) | (use_a & av) | both_z | (rest & !any_x & av & bv);
        value.push(v);
        unknown.push(u);
    }
    Logic::from_planes(a.width(), false, value, unknown)
}

/// Resolves any number of contributions; no contributions give all `z`.
pub(crate) fn resolve_all<'a>(width: u32, drivers: impl IntoIterator<Item = &'a Logic>) -> Logic {
    drivers
        .into_iter()
        .fold(Logic::z(width), |acc, d| resolve_wire(&acc, d))
}

/// Where an assignment lands, after every index has been evaluated.
///
/// A resolved lvalue is a list of stores, most significant first, whose
/// widths add up to the assigned value's width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// Bits `[lo, lo + width)` of a signal.
    Sig {
        /// The signal.
        sig: SigId,
        /// The lowest bit written.
        lo: u32,
        /// The number of bits written.
        width: u32,
    },
    /// One element of a memory.
    Mem {
        /// The memory.
        mem: MemId,
        /// The element index.
        addr: u64,
    },
    /// Bits that go nowhere: an out-of-range index or an unknown address.
    Skip {
        /// The number of bits dropped.
        width: u32,
    },
}

/// Per-net simulation storage.
#[derive(Clone, Debug)]
pub(crate) struct Signal {
    /// The hierarchical name of the first net mapped to this signal, for
    /// messages and waveforms.
    pub(crate) name: String,
    /// The declared type of that net.
    pub(crate) ty: Type,
    /// Wire or register semantics for driver resolution and `release`.
    pub(crate) kind: NetKind,
    /// The current resolved value (what drivers and processes produced).
    pub(crate) value: Logic,
    /// An override from `force`, reported to every reader until released.
    pub(crate) forced: Option<Logic>,
}

impl Signal {
    /// The value readers see.
    pub(crate) fn effective(&self) -> &Logic {
        self.forced.as_ref().unwrap_or(&self.value)
    }

    /// The width in bits.
    pub(crate) fn width(&self) -> u32 {
        self.value.width()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(s: &str) -> Logic {
        Logic::parse_verilog(s).unwrap()
    }

    #[test]
    fn wire_resolution_table() {
        let a = l("8'b0101zzxx");
        let b = l("8'b0011zx0z");
        // pairs (a,b): 0/0=0, 1/0=x, 0/1=x, 1/1=1, z/z=z, z/x=x, x/0=x, x/z=x
        assert_eq!(resolve_wire(&a, &b), l("8'b0xx1zxxx"));
        assert_eq!(resolve_all(4, [&l("4'bz1zz"), &l("4'b0z1z")]), l("4'b011z"));
        assert_eq!(resolve_all(2, []), l("2'bzz"));
    }

    #[test]
    fn select_out_of_range_reads_x() {
        let v = l("4'b1010");
        assert_eq!(select_bits(&v, 1, 2), l("2'b01"));
        assert_eq!(select_bits(&v, 3, 2), l("2'bx1"));
        assert_eq!(select_bits(&v, -1, 2), l("2'b0x"));
        assert_eq!(select_bits(&v, 4, 2), l("2'bxx"));
        assert_eq!(select_bits(&v, -5, 2), l("2'bxx"));
        assert_eq!(select_bits(&v, 0, 0).width(), 0);
    }

    #[test]
    fn replace_and_merge() {
        let v = l("8'h00");
        assert_eq!(replace_bits(&v, 4, &l("4'hf")), l("8'hf0"));
        assert_eq!(replace_bits(&v, 6, &l("4'hf")), l("8'hc0"));
        assert_eq!(merge_unknown(&l("4'b1100"), &l("4'b1010")), l("4'b1xx0"));
    }

    #[test]
    fn conversions() {
        assert_eq!(string_to_logic("AB"), l("16'h4142"));
        assert_eq!(logic_to_string(&l("24'h004142")), "AB");
        assert_eq!(logic_to_string(&l("8'bxxxxxxxx")), "?");
        assert_eq!(Value::Real(2.6).to_logic(), Logic::from_i64(3, 64));
        assert_eq!(Value::Real(f64::NAN).to_logic(), Logic::from_i64(0, 64));
        assert_eq!(Value::Str("A".into()).to_logic(), l("8'h41"));
        assert_eq!(Value::Bits(l("8'd5")).to_real(), 5.0);
        assert_eq!(Value::Bits(Logic::from_i64(-3, 8)).to_real(), -3.0);
        assert!(Value::Bits(l("4'bx000")).to_real().is_nan());
        assert!(Value::Str("s".into()).to_real().is_nan());
        assert_eq!(Value::Str(String::new()).truth(), Bit::Zero);
        assert_eq!(Value::Real(1.5).truth(), Bit::One);
        assert_eq!(Value::Real(0.0).width(), 64);
        assert_eq!(Value::Str("ab".into()).width(), 16);
        let wide = Logic::ones(70);
        assert!(logic_to_real(&wide) > 1.0e21);
    }

    #[test]
    fn type_widths() {
        assert_eq!(flat_width(&Type::bits(8)), 8);
        assert_eq!(flat_width(&Type::array(Type::bits(8), 4)), 32);
        assert_eq!(
            flat_width(&Type::array(Type::array(Type::bits(2), 3), 4)),
            24
        );
        assert_eq!(flat_width(&Type::Integer), 64);
        assert_eq!(flat_width(&Type::Real), 64);
        assert_eq!(flat_width(&Type::String), 0);
        assert_eq!(elem_width(&Type::array(Type::bits(8), 4)), 8);
        assert_eq!(elem_width(&Type::bits(8)), 1);
        assert_eq!(elem_count(&Type::array(Type::bits(8), 4)), 4);
        assert_eq!(elem_count(&Type::bits(8)), 8);
    }

    #[test]
    fn signal_effective_value() {
        let mut s = Signal {
            name: "a".into(),
            ty: Type::bits(4),
            kind: NetKind::Wire,
            value: l("4'd3"),
            forced: None,
        };
        assert_eq!(s.effective(), &l("4'd3"));
        s.forced = Some(l("4'd9"));
        assert_eq!(s.effective(), &l("4'd9"));
        assert_eq!(s.width(), 4);
    }
}
