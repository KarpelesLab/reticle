//! Types of IR values and constant bit vectors.
//!
//! The IR is explicitly sized: every net and every expression node carries
//! a [`Type`], and the operator rules in [`super::expr`] are stated in terms
//! of these types. Bit vectors are the main path; the other variants exist
//! so simulation-only values (VHDL `integer`, `real`, `string`) survive
//! lowering without a separate representation.
//!
//! [`Const`] is a small 4-state constant used for literals, memory
//! initialisation, parameter values and LUT contents. It will be replaced
//! by `logic::Logic` (phase 0 of the roadmap) once that lands; until then it
//! deliberately supports no arithmetic, only construction and inspection.

use std::fmt;

/// The type of a net or expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    /// A packed bit vector of the given width, with an interpretation flag
    /// used by the signed arithmetic and comparison rules.
    Bits {
        /// Number of bits; `0` is legal for an empty vector.
        width: u32,
        /// True when arithmetic on the value is two's complement.
        signed: bool,
    },
    /// An unpacked array of `len` elements of `elem`, for VHDL arrays and
    /// SystemVerilog unpacked dimensions. Memories are not arrays: they are
    /// separate objects with ports (see [`super::Memory`]).
    Array {
        /// Element type.
        elem: Box<Type>,
        /// Number of elements.
        len: u64,
    },
    /// A 64-bit two's complement integer (VHDL `integer`, Verilog
    /// `integer`/`time` in simulation-only code).
    Integer,
    /// A 64-bit floating point value (`real`).
    Real,
    /// A character string, only meaningful in simulation.
    String,
}

impl Type {
    /// An unsigned bit vector of `width` bits.
    pub fn bits(width: u32) -> Self {
        Type::Bits {
            width,
            signed: false,
        }
    }

    /// A signed bit vector of `width` bits.
    pub fn sbits(width: u32) -> Self {
        Type::Bits {
            width,
            signed: true,
        }
    }

    /// A single unsigned bit.
    pub fn bit() -> Self {
        Type::bits(1)
    }

    /// An unpacked array of `len` elements of `elem`.
    pub fn array(elem: Type, len: u64) -> Self {
        Type::Array {
            elem: Box::new(elem),
            len,
        }
    }

    /// The width of a bit vector; `None` for every other type.
    pub fn width(&self) -> Option<u32> {
        match self {
            Type::Bits { width, .. } => Some(*width),
            _ => None,
        }
    }

    /// True for signed bit vectors and for `Integer`/`Real`.
    pub fn is_signed(&self) -> bool {
        match self {
            Type::Bits { signed, .. } => *signed,
            Type::Integer | Type::Real => true,
            Type::Array { .. } | Type::String => false,
        }
    }

    /// True for [`Type::Bits`].
    pub fn is_bits(&self) -> bool {
        matches!(self, Type::Bits { .. })
    }

    /// True when the type is a bit vector of width one.
    pub fn is_bit(&self) -> bool {
        self.width() == Some(1)
    }

    /// The same bit vector type with the signedness replaced; other types
    /// are returned unchanged.
    pub fn with_signed(&self, signed: bool) -> Type {
        match self {
            Type::Bits { width, .. } => Type::Bits {
                width: *width,
                signed,
            },
            other => other.clone(),
        }
    }
}

impl fmt::Display for Type {
    /// Renders the type in the text format: `u8`, `s16`, `[4]u8`, `int`,
    /// `real`, `string`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Bits { width, signed } => {
                write!(f, "{}{}", if *signed { 's' } else { 'u' }, width)
            }
            Type::Array { elem, len } => write!(f, "[{len}]{elem}"),
            Type::Integer => f.write_str("int"),
            Type::Real => f.write_str("real"),
            Type::String => f.write_str("string"),
        }
    }
}

/// One 4-state bit.
///
/// Will be replaced by the bit type of `logic::Logic` when that module
/// lands; the IR only needs to store and compare bits, not operate on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bit4 {
    /// Logic low.
    Zero,
    /// Logic high.
    One,
    /// Unknown.
    X,
    /// High impedance.
    Z,
}

impl Bit4 {
    /// The character used for this bit in literals.
    pub fn as_char(self) -> char {
        match self {
            Bit4::Zero => '0',
            Bit4::One => '1',
            Bit4::X => 'x',
            Bit4::Z => 'z',
        }
    }

    /// Parses one of `0 1 x z` (either case); `None` for anything else.
    pub fn from_char(c: char) -> Option<Bit4> {
        match c {
            '0' => Some(Bit4::Zero),
            '1' => Some(Bit4::One),
            'x' | 'X' => Some(Bit4::X),
            'z' | 'Z' => Some(Bit4::Z),
            _ => None,
        }
    }

    /// True for `0` and `1`.
    pub fn is_two_state(self) -> bool {
        matches!(self, Bit4::Zero | Bit4::One)
    }
}

/// A constant bit vector with a width and a signedness flag.
///
/// `bits[0]` is the least significant bit and `bits.len() == width` always
/// holds for values built through the constructors. The fields are public
/// so passes can pattern-match, but new values should go through
/// [`Const::new`], [`Const::from_u64`] and friends, which maintain the
/// invariant.
///
/// This type is a placeholder for `logic::Logic` and intentionally offers
/// no arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Const {
    /// Number of bits.
    pub width: u32,
    /// True when the value is two's complement.
    pub signed: bool,
    /// The bits, least significant first.
    pub bits: Vec<Bit4>,
}

impl Const {
    /// Builds a constant from bits (least significant first); the width is
    /// the number of bits.
    pub fn new(bits: Vec<Bit4>, signed: bool) -> Self {
        Const {
            width: super::arena::narrow(bits.len()),
            signed,
            bits,
        }
    }

    /// An unsigned constant holding the low `width` bits of `value`.
    ///
    /// Bits beyond the width are discarded; widths above 64 are zero
    /// extended.
    pub fn from_u64(width: u32, value: u64) -> Self {
        let bits = (0..width)
            .map(|i| {
                if i < 64 && (value >> i) & 1 == 1 {
                    Bit4::One
                } else {
                    Bit4::Zero
                }
            })
            .collect();
        Const {
            width,
            signed: false,
            bits,
        }
    }

    /// A signed constant holding the low `width` bits of `value`'s two's
    /// complement representation, sign extended past 64 bits.
    pub fn from_i64(width: u32, value: i64) -> Self {
        let bits = (0..width)
            .map(|i| {
                let shift = i.min(63);
                if (value >> shift) & 1 == 1 {
                    Bit4::One
                } else {
                    Bit4::Zero
                }
            })
            .collect();
        Const {
            width,
            signed: true,
            bits,
        }
    }

    /// A constant of `width` bits all set to `bit`.
    pub fn filled(width: u32, bit: Bit4) -> Self {
        Const {
            width,
            signed: false,
            bits: vec![bit; super::arena::widen(width)],
        }
    }

    /// A constant of `width` unknown (`x`) bits.
    pub fn undef(width: u32) -> Self {
        Self::filled(width, Bit4::X)
    }

    /// A single bit.
    pub fn bit(bit: Bit4) -> Self {
        Self::filled(1, bit)
    }

    /// The constant `1'b0`.
    pub fn zero() -> Self {
        Self::bit(Bit4::Zero)
    }

    /// The constant `1'b1`.
    pub fn one() -> Self {
        Self::bit(Bit4::One)
    }

    /// The type of this constant.
    pub fn ty(&self) -> Type {
        Type::Bits {
            width: self.width,
            signed: self.signed,
        }
    }

    /// True when every bit is `0` or `1`.
    pub fn is_two_state(&self) -> bool {
        self.bits.iter().all(|b| b.is_two_state())
    }

    /// The value as an unsigned integer when it is two-state and no set bit
    /// lies above bit 63; `None` otherwise.
    pub fn to_u64(&self) -> Option<u64> {
        let mut value = 0u64;
        for (i, bit) in self.bits.iter().enumerate() {
            match bit {
                Bit4::Zero => {}
                Bit4::One if i < 64 => value |= 1 << i,
                Bit4::One | Bit4::X | Bit4::Z => return None,
            }
        }
        Some(value)
    }

    /// True when the value is two-state and equal to zero.
    pub fn is_zero(&self) -> bool {
        self.bits.iter().all(|b| *b == Bit4::Zero)
    }

    /// The same bits with the signedness flag replaced.
    pub fn with_signed(mut self, signed: bool) -> Self {
        self.signed = signed;
        self
    }

    /// The bits as a string, most significant first (`10xz`).
    pub fn to_binary_string(&self) -> String {
        self.bits.iter().rev().map(|b| b.as_char()).collect()
    }
}

impl fmt::Display for Const {
    /// Renders the constant as a Verilog-style sized literal, which is also
    /// its canonical form in the text format: decimal for two-state values
    /// up to 64 bits (`8'd255`, `8'sd255`), hexadecimal for wider two-state
    /// values, binary when any bit is `x` or `z` (`4'b10xz`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = if self.signed { "s" } else { "" };
        if self.width <= 64
            && let Some(v) = self.to_u64()
        {
            return write!(f, "{}'{s}d{v}", self.width);
        }
        if self.is_two_state() {
            write!(f, "{}'{s}h", self.width)?;
            let mut digits = String::new();
            for chunk in self.bits.chunks(4) {
                let mut nibble = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    if *bit == Bit4::One {
                        nibble |= 1 << i;
                    }
                }
                digits.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
            }
            let digits: String = digits.chars().rev().collect();
            return f.write_str(&digits);
        }
        write!(f, "{}'{s}b{}", self.width, self.to_binary_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_display_and_queries() {
        assert_eq!(Type::bits(8).to_string(), "u8");
        assert_eq!(Type::sbits(16).to_string(), "s16");
        assert_eq!(Type::array(Type::bits(8), 4).to_string(), "[4]u8");
        assert_eq!(Type::Integer.to_string(), "int");
        assert_eq!(Type::Real.to_string(), "real");
        assert_eq!(Type::String.to_string(), "string");
        assert!(Type::bit().is_bit());
        assert_eq!(Type::array(Type::bits(8), 4).width(), None);
        assert!(Type::sbits(4).is_signed());
        assert!(
            !Type::bits(4)
                .with_signed(true)
                .with_signed(false)
                .is_signed()
        );
    }

    #[test]
    fn const_construction_and_display() {
        let c = Const::from_u64(8, 255);
        assert_eq!(c.to_string(), "8'd255");
        assert_eq!(c.to_u64(), Some(255));
        assert_eq!(c.ty(), Type::bits(8));
        assert_eq!(Const::from_u64(4, 0x1f).to_u64(), Some(0xf));
        assert_eq!(Const::from_i64(8, -1).to_string(), "8'sd255");
        assert_eq!(Const::from_i64(70, -2).bits[69], Bit4::One);
        assert_eq!(Const::undef(4).to_string(), "4'bxxxx");
        assert_eq!(
            Const::new(vec![Bit4::Z, Bit4::X, Bit4::One, Bit4::Zero], false).to_string(),
            "4'b01xz"
        );
        assert!(Const::zero().is_zero());
        assert_eq!(Const::one().to_string(), "1'd1");
        let wide = Const::from_u64(72, u64::MAX);
        assert_eq!(wide.to_string(), "72'h00ffffffffffffffff");
        assert_eq!(wide.to_u64(), Some(u64::MAX));
        let mut top = Const::from_u64(65, 0);
        top.bits[64] = Bit4::One;
        assert_eq!(top.to_string(), "65'h10000000000000000");
        assert_eq!(Const::from_u64(3, 5).to_binary_string(), "101");
        assert_eq!(Bit4::from_char('X'), Some(Bit4::X));
        assert_eq!(Bit4::from_char('q'), None);
    }
}
