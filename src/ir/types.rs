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

/// One 4-state bit; re-exported from [`crate::logic`].
pub use crate::logic::Bit;

/// A constant bit vector: [`crate::logic::Logic`] under its IR name.
///
/// Constants in the IR are plain `Logic` values so the frontends' constant
/// evaluation, the simulator and constant folding share one representation
/// and one set of operators. The text format renders them through
/// [`text`](super::text) in a canonical Verilog-style sized form.
pub type Const = crate::logic::Logic;

/// The IR type of a constant: `Bits` with its width and signedness.
pub fn const_type(c: &Const) -> Type {
    Type::Bits {
        width: c.width(),
        signed: c.is_signed(),
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
    fn const_type_follows_width_and_sign() {
        assert_eq!(const_type(&Const::from_u64(255, 8)), Type::bits(8));
        assert_eq!(
            const_type(&Const::from_i64(-1, 8)),
            Type::bits(8).with_signed(true)
        );
    }
}
