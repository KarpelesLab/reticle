//! The VHDL type model (IEEE 1076-2008 clause 5).
//!
//! Every type and subtype the analysis encounters becomes one [`Type`] in
//! the arena owned by [`super::Analysis`], addressed by a [`TypeId`]. The
//! model follows the standard's own structure rather than a simplified
//! "width + signedness" view, because overload resolution, closely related
//! type conversions and the predefined attributes all need the real thing:
//!
//! - A **type declaration** produces a *base type* ([`TypeKind::Enum`],
//!   [`TypeKind::Integer`], [`TypeKind::Real`], [`TypeKind::Physical`],
//!   [`TypeKind::Array`], [`TypeKind::Record`], [`TypeKind::Access`],
//!   [`TypeKind::File`], [`TypeKind::Protected`]). Two values belong to the
//!   same type exactly when their base types are the same arena entry; there
//!   is no structural equivalence in VHDL (clause 5.1).
//! - A **subtype** ([`TypeKind::Subtype`]) names a parent (which may itself
//!   be a subtype) plus an optional [`Constraint`] and an optional resolution
//!   function. `natural`, `std_logic` and `std_logic_vector(7 downto 0)` are
//!   all subtypes; [`Analysis::base_type`](super::Analysis::base_type)
//!   strips the layers.
//! - A constrained array type declaration such as `type word is array (15
//!   downto 0) of bit` follows clause 5.3.2.1: an anonymous unconstrained
//!   base type plus a subtype carrying the index constraint, the subtype
//!   bearing the declared name.
//! - Scalar ranges and index constraints keep each bound as a
//!   [`Bound`]: a [`Value`] when the bound is locally static, or the
//!   [`Span`] of its expression when it depends on a generic or a signal.
//!   The elaboration pass re-evaluates dynamic bounds once generics are
//!   known, without touching the type arena.
//! - The two anonymous universal types (clause 5.2.1) are ordinary arena
//!   entries so that literal typing and implicit conversion need no special
//!   representation: an integer literal has type `universal_integer`, which
//!   is implicitly convertible to every integer type.
//!
//! [`Analysis`](super::Analysis) provides the queries that need the whole
//! arena (`base_type`, `is_scalar`, `describe`, ...); this module holds only
//! the data.

use crate::intern::Symbol;
use crate::source::Span;
use crate::vhdl::ast::Direction;

use super::DeclId;
use super::constant::Value;

/// Index of a [`Type`] in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub(crate) u32);

impl TypeId {
    /// The raw index into the arena.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One entry of the type arena.
#[derive(Clone, Debug)]
pub struct Type {
    /// What kind of type or subtype this is.
    pub kind: TypeKind,
    /// The declared name, `None` for anonymous types and subtypes (the base
    /// of a constrained array type, the subtype of a signal declared with an
    /// inline constraint, ...).
    pub name: Option<Symbol>,
    /// The declaration that introduced it, when it has one.
    pub decl: Option<DeclId>,
}

/// One bound of a range or index constraint.
#[derive(Clone, Debug, PartialEq)]
pub enum Bound {
    /// A locally static bound: an integer, a real, an enumeration position
    /// or a physical value, all as a [`Value`].
    Static(Value),
    /// A bound that is not locally static (a generic, a signal, a call to a
    /// user function); the span is the bound's expression so elaboration can
    /// evaluate it later.
    Dynamic(Span),
}

impl Bound {
    /// The static value, if any.
    pub fn value(&self) -> Option<&Value> {
        match self {
            Bound::Static(v) => Some(v),
            Bound::Dynamic(_) => None,
        }
    }

    /// The static integer (or enumeration position) value, if any.
    pub fn int(&self) -> Option<i128> {
        self.value().and_then(Value::as_int)
    }
}

/// A range `left dir right`, with each bound static or dynamic.
#[derive(Clone, Debug, PartialEq)]
pub struct Bounds {
    /// The left bound.
    pub left: Bound,
    /// `to` or `downto`.
    pub dir: Direction,
    /// The right bound.
    pub right: Bound,
}

impl Bounds {
    /// Builds a fully static integer range.
    pub fn int(left: i128, dir: Direction, right: i128) -> Self {
        Bounds {
            left: Bound::Static(Value::Int(left)),
            dir,
            right: Bound::Static(Value::Int(right)),
        }
    }

    /// True when both bounds are static.
    pub fn is_static(&self) -> bool {
        self.left.value().is_some() && self.right.value().is_some()
    }

    /// The static integer bounds as `(left, right)`, if both are static.
    pub fn ints(&self) -> Option<(i128, i128)> {
        Some((self.left.int()?, self.right.int()?))
    }

    /// The number of values in the range, when static. A null range has
    /// zero.
    pub fn length(&self) -> Option<i128> {
        let (l, r) = self.ints()?;
        Some(match self.dir {
            Direction::To => (r - l + 1).max(0),
            Direction::Downto => (l - r + 1).max(0),
        })
    }

    /// The smaller and larger static bound, if static.
    pub fn low_high(&self) -> Option<(i128, i128)> {
        let (l, r) = self.ints()?;
        Some(match self.dir {
            Direction::To => (l, r),
            Direction::Downto => (r, l),
        })
    }

    /// True when a static integer `v` lies inside a static range.
    /// Returns `None` if the range is not static.
    pub fn contains_int(&self, v: i128) -> Option<bool> {
        let (lo, hi) = self.low_high()?;
        Some(v >= lo && v <= hi)
    }

    /// True when a static real `v` lies inside a static real range.
    pub fn contains_real(&self, v: f64) -> Option<bool> {
        let l = self.left.value()?.as_real()?;
        let r = self.right.value()?.as_real()?;
        let (lo, hi) = match self.dir {
            Direction::To => (l, r),
            Direction::Downto => (r, l),
        };
        Some(v >= lo && v <= hi)
    }

    /// The range with its direction and bounds swapped.
    pub fn reversed(&self) -> Bounds {
        Bounds {
            left: self.right.clone(),
            dir: match self.dir {
                Direction::To => Direction::Downto,
                Direction::Downto => Direction::To,
            },
            right: self.left.clone(),
        }
    }
}

/// A constraint attached to a subtype (clause 5.2.2, 5.3.2.2, 5.3.3).
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    /// A scalar range constraint.
    Range(Bounds),
    /// An index constraint (one entry per dimension) and, in VHDL-2008, an
    /// optional constrained element subtype (`t(0 to 3)(7 downto 0)`).
    Index(Vec<Bounds>, Option<TypeId>),
    /// A VHDL-2008 element-only constraint: `t(open)(7 downto 0)`, as the
    /// constrained element subtype.
    Element(TypeId),
    /// A VHDL-2008 record constraint: the constrained subtype of each
    /// named element.
    Record(Vec<(Symbol, TypeId)>),
}

/// One field of a record type.
#[derive(Clone, Debug)]
pub struct Field {
    /// The field name (case-folded).
    pub name: Symbol,
    /// The field's subtype.
    pub ty: TypeId,
    /// Where the field is declared.
    pub span: Span,
}

/// The variants of the type model; see the module docs.
#[derive(Clone, Debug)]
pub enum TypeKind {
    /// The anonymous `universal_integer` type of integer literals.
    UniversalInteger,
    /// The anonymous `universal_real` type of real literals.
    UniversalReal,
    /// An enumeration type: the literals in position order, each one an
    /// enumeration-literal declaration.
    Enum {
        /// The literal declarations, in position order.
        literals: Vec<DeclId>,
        /// True when at least one literal is a character literal, making
        /// this a *character type* (clause 5.2.2.1) usable as a string
        /// element type.
        character: bool,
    },
    /// An integer type with its declared range.
    Integer(Bounds),
    /// A floating-point type with its declared range.
    Real(Bounds),
    /// A physical type: range, and its units as declarations (the primary
    /// unit first).
    Physical {
        /// The range in primary units.
        range: Bounds,
        /// The unit declarations, primary first.
        units: Vec<DeclId>,
    },
    /// An unconstrained array type: index subtypes and element subtype.
    Array {
        /// One index subtype per dimension.
        indices: Vec<TypeId>,
        /// The element subtype.
        element: TypeId,
    },
    /// A record type.
    Record(Vec<Field>),
    /// An access type designating a subtype.
    Access(TypeId),
    /// A file type of a subtype.
    File(TypeId),
    /// A protected type; its methods live in the given region.
    Protected(super::RegionId),
    /// An incomplete type declaration awaiting its full declaration.
    Incomplete,
    /// A VHDL-2008 generic type (`type t;` in a generic list): any
    /// operation other than the generic subprograms is unknown.
    Generic,
    /// A subtype of `parent`.
    Subtype {
        /// The parent type or subtype.
        parent: TypeId,
        /// The constraint, if any.
        constraint: Option<Constraint>,
        /// The resolution function, if this subtype indication named one.
        resolution: Option<DeclId>,
    },
    /// A placeholder for a type that failed to analyse; compatible with
    /// everything so one error does not cascade.
    Error,
}

/// A classification of base types used by the implicit-operator rules of
/// clause 9.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClass {
    /// `universal_integer`.
    UniversalInteger,
    /// `universal_real`.
    UniversalReal,
    /// An enumeration type.
    Enum,
    /// An integer type.
    Integer,
    /// A floating-point type.
    Real,
    /// A physical type.
    Physical,
    /// An array type.
    Array,
    /// A record type.
    Record,
    /// An access type.
    Access,
    /// A file type.
    File,
    /// A protected type.
    Protected,
    /// An incomplete, generic or erroneous type.
    Other,
}

impl TypeKind {
    /// The class of a base-type kind. Call it on a base type only: a
    /// subtype reports [`TypeClass::Other`].
    pub fn class(&self) -> TypeClass {
        match self {
            TypeKind::UniversalInteger => TypeClass::UniversalInteger,
            TypeKind::UniversalReal => TypeClass::UniversalReal,
            TypeKind::Enum { .. } => TypeClass::Enum,
            TypeKind::Integer(_) => TypeClass::Integer,
            TypeKind::Real(_) => TypeClass::Real,
            TypeKind::Physical { .. } => TypeClass::Physical,
            TypeKind::Array { .. } => TypeClass::Array,
            TypeKind::Record(_) => TypeClass::Record,
            TypeKind::Access(_) => TypeClass::Access,
            TypeKind::File(_) => TypeClass::File,
            TypeKind::Protected(_) => TypeClass::Protected,
            TypeKind::Incomplete
            | TypeKind::Generic
            | TypeKind::Error
            | TypeKind::Subtype { .. } => TypeClass::Other,
        }
    }
}

impl TypeClass {
    /// True for the integer, real, physical and universal classes: the ones
    /// with predefined arithmetic (clause 9.2.5).
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            TypeClass::UniversalInteger
                | TypeClass::UniversalReal
                | TypeClass::Integer
                | TypeClass::Real
                | TypeClass::Physical
        )
    }

    /// True for the integer classes (universal included).
    pub fn is_integer(self) -> bool {
        matches!(self, TypeClass::UniversalInteger | TypeClass::Integer)
    }

    /// True for the floating classes (universal included).
    pub fn is_real(self) -> bool {
        matches!(self, TypeClass::UniversalReal | TypeClass::Real)
    }

    /// True for the discrete classes: enumeration and integer.
    pub fn is_discrete(self) -> bool {
        matches!(
            self,
            TypeClass::Enum | TypeClass::Integer | TypeClass::UniversalInteger
        )
    }

    /// True for scalar classes (clause 5.2): discrete, floating, physical.
    pub fn is_scalar(self) -> bool {
        self.is_discrete() || self.is_real() || self == TypeClass::Physical
    }

    /// True for composite classes (clause 5.3): array and record.
    pub fn is_composite(self) -> bool {
        matches!(self, TypeClass::Array | TypeClass::Record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_arithmetic() {
        let b = Bounds::int(7, Direction::Downto, 0);
        assert_eq!(b.length(), Some(8));
        assert_eq!(b.low_high(), Some((0, 7)));
        assert_eq!(b.contains_int(7), Some(true));
        assert_eq!(b.contains_int(8), Some(false));
        let r = b.reversed();
        assert_eq!(r.dir, Direction::To);
        assert_eq!(r.ints(), Some((0, 7)));
        let null = Bounds::int(1, Direction::To, 0);
        assert_eq!(null.length(), Some(0));
        let mut m = crate::source::SourceMap::new();
        let id = m.add("x", "").unwrap();
        let dynamic = Bounds {
            left: Bound::Dynamic(Span::new(id, 0, 0)),
            dir: Direction::To,
            right: Bound::Static(Value::Int(3)),
        };
        assert!(!dynamic.is_static());
        assert_eq!(dynamic.length(), None);
    }

    #[test]
    fn classes() {
        assert!(TypeClass::Physical.is_numeric());
        assert!(TypeClass::Physical.is_scalar());
        assert!(!TypeClass::Physical.is_discrete());
        assert!(TypeClass::Enum.is_discrete());
        assert!(TypeClass::Array.is_composite());
        assert!(!TypeClass::Array.is_scalar());
        assert_eq!(TypeKind::Error.class(), TypeClass::Other);
        assert_eq!(TypeKind::UniversalReal.class(), TypeClass::UniversalReal);
    }
}
