//! Predefined attributes (IEEE 1076-2008 clause 16.2).
//!
//! [`Predefined`] names every predefined attribute and classifies it by the
//! prefix it accepts (a type, an array object or subtype, a signal, or any
//! named entity) and by whether it takes an argument. The checker uses the
//! table to type an attribute name; the static evaluation of the type and
//! array attributes on locally static subtypes lives here too, so
//! `integer'high`, `std_logic_vector(7 downto 0)'length` and
//! `state_t'pos(idle)` fold to values.
//!
//! Signal attributes (`'event`, `'stable`, ...) are typed here but never
//! evaluated: their value is a simulation matter, and the lowering pass
//! turns them into the corresponding IR constructs by looking at
//! [`super::CallTarget::Attribute`] or the attribute name.

use super::constant::Value;
use super::types::{Bound, Bounds, TypeClass, TypeId, TypeKind};
use super::{Analysis, DeclKind};
use crate::vhdl::ast::Direction;

/// The predefined attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Predefined {
    Base,
    Left,
    Right,
    High,
    Low,
    Ascending,
    Image,
    Value,
    Pos,
    Val,
    Succ,
    Pred,
    LeftOf,
    RightOf,
    Subtype,
    Range,
    ReverseRange,
    Length,
    Element,
    Delayed,
    Stable,
    Quiet,
    Transaction,
    Event,
    Active,
    LastEvent,
    LastActive,
    LastValue,
    Driving,
    DrivingValue,
    SimpleName,
    InstanceName,
    PathName,
}

/// What kind of prefix an attribute needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixKind {
    /// A scalar type or subtype (or, for `'base`, `'image`, ..., any type).
    Type,
    /// An array object or constrained array subtype.
    Array,
    /// A signal.
    Signal,
    /// Any named entity.
    Entity,
}

/// Whether an attribute takes an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    /// Never.
    None,
    /// Optional (a dimension for array attributes, a time for `'delayed`
    /// and friends).
    Optional,
    /// Required (`'image(x)`, `'pos(x)`, ...).
    Required,
}

impl Predefined {
    /// Looks an attribute designator up (case-insensitively).
    pub fn from_name(name: &str) -> Option<Predefined> {
        Some(match name.to_ascii_lowercase().as_str() {
            "base" => Predefined::Base,
            "left" => Predefined::Left,
            "right" => Predefined::Right,
            "high" => Predefined::High,
            "low" => Predefined::Low,
            "ascending" => Predefined::Ascending,
            "image" => Predefined::Image,
            "value" => Predefined::Value,
            "pos" => Predefined::Pos,
            "val" => Predefined::Val,
            "succ" => Predefined::Succ,
            "pred" => Predefined::Pred,
            "leftof" => Predefined::LeftOf,
            "rightof" => Predefined::RightOf,
            "subtype" => Predefined::Subtype,
            "range" => Predefined::Range,
            "reverse_range" => Predefined::ReverseRange,
            "length" => Predefined::Length,
            "element" => Predefined::Element,
            "delayed" => Predefined::Delayed,
            "stable" => Predefined::Stable,
            "quiet" => Predefined::Quiet,
            "transaction" => Predefined::Transaction,
            "event" => Predefined::Event,
            "active" => Predefined::Active,
            "last_event" => Predefined::LastEvent,
            "last_active" => Predefined::LastActive,
            "last_value" => Predefined::LastValue,
            "driving" => Predefined::Driving,
            "driving_value" => Predefined::DrivingValue,
            "simple_name" => Predefined::SimpleName,
            "instance_name" => Predefined::InstanceName,
            "path_name" => Predefined::PathName,
            _ => return None,
        })
    }

    /// The attribute's name as the standard spells it.
    pub fn name(self) -> &'static str {
        match self {
            Predefined::Base => "base",
            Predefined::Left => "left",
            Predefined::Right => "right",
            Predefined::High => "high",
            Predefined::Low => "low",
            Predefined::Ascending => "ascending",
            Predefined::Image => "image",
            Predefined::Value => "value",
            Predefined::Pos => "pos",
            Predefined::Val => "val",
            Predefined::Succ => "succ",
            Predefined::Pred => "pred",
            Predefined::LeftOf => "leftof",
            Predefined::RightOf => "rightof",
            Predefined::Subtype => "subtype",
            Predefined::Range => "range",
            Predefined::ReverseRange => "reverse_range",
            Predefined::Length => "length",
            Predefined::Element => "element",
            Predefined::Delayed => "delayed",
            Predefined::Stable => "stable",
            Predefined::Quiet => "quiet",
            Predefined::Transaction => "transaction",
            Predefined::Event => "event",
            Predefined::Active => "active",
            Predefined::LastEvent => "last_event",
            Predefined::LastActive => "last_active",
            Predefined::LastValue => "last_value",
            Predefined::Driving => "driving",
            Predefined::DrivingValue => "driving_value",
            Predefined::SimpleName => "simple_name",
            Predefined::InstanceName => "instance_name",
            Predefined::PathName => "path_name",
        }
    }

    /// The prefix kind. `'left` and friends accept both a scalar type and
    /// an array, which the checker decides from the prefix.
    pub fn prefix(self) -> PrefixKind {
        match self {
            Predefined::Base
            | Predefined::Image
            | Predefined::Value
            | Predefined::Pos
            | Predefined::Val
            | Predefined::Succ
            | Predefined::Pred
            | Predefined::LeftOf
            | Predefined::RightOf => PrefixKind::Type,
            Predefined::Left
            | Predefined::Right
            | Predefined::High
            | Predefined::Low
            | Predefined::Ascending
            | Predefined::Subtype
            | Predefined::Range
            | Predefined::ReverseRange
            | Predefined::Length
            | Predefined::Element => PrefixKind::Array,
            Predefined::Delayed
            | Predefined::Stable
            | Predefined::Quiet
            | Predefined::Transaction
            | Predefined::Event
            | Predefined::Active
            | Predefined::LastEvent
            | Predefined::LastActive
            | Predefined::LastValue
            | Predefined::Driving
            | Predefined::DrivingValue => PrefixKind::Signal,
            Predefined::SimpleName | Predefined::InstanceName | Predefined::PathName => {
                PrefixKind::Entity
            }
        }
    }

    /// Whether the attribute takes an argument.
    pub fn arg(self) -> Arg {
        match self {
            Predefined::Image
            | Predefined::Value
            | Predefined::Pos
            | Predefined::Val
            | Predefined::Succ
            | Predefined::Pred
            | Predefined::LeftOf
            | Predefined::RightOf => Arg::Required,
            Predefined::Left
            | Predefined::Right
            | Predefined::High
            | Predefined::Low
            | Predefined::Ascending
            | Predefined::Range
            | Predefined::ReverseRange
            | Predefined::Length
            | Predefined::Element
            | Predefined::Delayed
            | Predefined::Stable
            | Predefined::Quiet => Arg::Optional,
            _ => Arg::None,
        }
    }

    /// True for `'range` and `'reverse_range`, which denote a range rather
    /// than a value.
    pub fn is_range(self) -> bool {
        matches!(self, Predefined::Range | Predefined::ReverseRange)
    }

    /// True for the attributes that denote a type rather than a value.
    pub fn is_type_valued(self) -> bool {
        matches!(
            self,
            Predefined::Base | Predefined::Subtype | Predefined::Element
        )
    }

    /// True for the signal-valued attributes (`'delayed`, `'stable`,
    /// `'quiet`, `'transaction`), which may appear in sensitivity lists.
    pub fn is_signal_valued(self) -> bool {
        matches!(
            self,
            Predefined::Delayed | Predefined::Stable | Predefined::Quiet | Predefined::Transaction
        )
    }
}

/// Which bound a scalar attribute asks for.
fn scalar_bound(a: &Analysis, ty: TypeId, attr: Predefined) -> Option<Value> {
    let b = a.scalar_range(ty)?;
    let (l, r) = (b.left.value()?, b.right.value()?);
    let asc = b.dir == Direction::To;
    Some(match attr {
        Predefined::Left => l.clone(),
        Predefined::Right => r.clone(),
        Predefined::High => {
            if asc {
                r.clone()
            } else {
                l.clone()
            }
        }
        Predefined::Low => {
            if asc {
                l.clone()
            } else {
                r.clone()
            }
        }
        Predefined::Ascending => Value::from_bool(asc),
        _ => return None,
    })
}

/// Evaluates a scalar type attribute without argument (`t'left`,
/// `t'ascending`) on a static subtype.
pub fn eval_scalar(a: &Analysis, ty: TypeId, attr: Predefined) -> Option<Value> {
    scalar_bound(a, ty, attr)
}

/// The bounds of dimension `dim` (1-based) of a constrained array subtype
/// when static.
pub fn array_bounds(a: &Analysis, ty: TypeId, dim: usize) -> Option<Bounds> {
    let c = a.index_constraint(ty)?;
    c.get(dim.checked_sub(1)?).cloned()
}

/// Evaluates an array attribute on a constrained subtype with static
/// bounds.
pub fn eval_array(a: &Analysis, ty: TypeId, attr: Predefined, dim: usize) -> Option<Value> {
    let b = array_bounds(a, ty, dim)?;
    let asc = b.dir == Direction::To;
    Some(match attr {
        Predefined::Left => b.left.value()?.clone(),
        Predefined::Right => b.right.value()?.clone(),
        Predefined::High => {
            if asc {
                b.right.value()?.clone()
            } else {
                b.left.value()?.clone()
            }
        }
        Predefined::Low => {
            if asc {
                b.left.value()?.clone()
            } else {
                b.right.value()?.clone()
            }
        }
        Predefined::Length => Value::Int(b.length()?),
        Predefined::Ascending => Value::from_bool(asc),
        _ => return None,
    })
}

/// The result type of an array attribute: the index subtype for the
/// bound attributes, `universal_integer` for `'length`, `boolean` for
/// `'ascending`.
pub fn array_attr_type(a: &Analysis, ty: TypeId, attr: Predefined, dim: usize) -> Option<TypeId> {
    let (indices, _) = a.array_info(ty)?;
    match attr {
        Predefined::Left | Predefined::Right | Predefined::High | Predefined::Low => {
            indices.get(dim.checked_sub(1)?).copied()
        }
        Predefined::Length => Some(a.builtins.universal_integer),
        Predefined::Ascending => Some(a.builtins.boolean),
        _ => None,
    }
}

/// Evaluates the type attributes with an argument on static operands:
/// `'pos`, `'val`, `'succ`, `'pred`, `'leftof`, `'rightof`, `'image`,
/// `'value`.
pub fn eval_with_arg(a: &Analysis, ty: TypeId, attr: Predefined, arg: &Value) -> Option<Value> {
    let base = a.base_type(ty);
    let class = a.ty(base).kind.class();
    match attr {
        Predefined::Pos => arg.as_int().map(Value::Int),
        Predefined::Val => {
            let n = arg.as_int()?;
            match class {
                TypeClass::Enum => {
                    let TypeKind::Enum { literals, .. } = &a.ty(base).kind else {
                        return None;
                    };
                    if n < 0 || usize::try_from(n).ok()? >= literals.len() {
                        return None;
                    }
                    Some(Value::Enum(u32::try_from(n).ok()?))
                }
                TypeClass::Integer | TypeClass::Physical => Some(Value::Int(n)),
                _ => None,
            }
        }
        Predefined::Succ | Predefined::Pred | Predefined::LeftOf | Predefined::RightOf => {
            let asc = a
                .scalar_range(ty)
                .map(|b| b.dir == Direction::To)
                .unwrap_or(true);
            let forward = match attr {
                Predefined::Succ => true,
                Predefined::Pred => false,
                Predefined::RightOf => asc,
                _ => !asc,
            };
            let delta: i128 = if forward { 1 } else { -1 };
            match arg {
                Value::Enum(p) => {
                    let TypeKind::Enum { literals, .. } = &a.ty(base).kind else {
                        return None;
                    };
                    let n = i128::from(*p) + delta;
                    if n < 0 || usize::try_from(n).ok()? >= literals.len() {
                        return None;
                    }
                    Some(Value::Enum(u32::try_from(n).ok()?))
                }
                Value::Int(i) => Some(Value::Int(i.checked_add(delta)?)),
                _ => None,
            }
        }
        Predefined::Image => Some(image(a, ty, arg)),
        Predefined::Value => value_of_string(a, ty, arg),
        _ => None,
    }
}

/// `t'image(x)` for a scalar static value.
pub fn image(a: &Analysis, ty: TypeId, v: &Value) -> Value {
    let base = a.base_type(ty);
    let text = match (&a.ty(base).kind, v) {
        (TypeKind::Enum { literals, .. }, Value::Enum(p)) => literals
            .get(usize::try_from(*p).unwrap_or(usize::MAX))
            .map(|d| a.decl(*d).spelling.to_lowercase())
            .unwrap_or_default(),
        (TypeKind::Physical { units, .. }, Value::Int(i)) => {
            let unit = units
                .first()
                .map_or(String::new(), |u| a.decl(*u).spelling.to_lowercase());
            format!("{i} {unit}")
        }
        (_, Value::Real(r)) => super::format_real(*r),
        (_, v) => v.to_string(),
    };
    Value::string(text.chars().map(|c| c as u32))
}

/// `t'value(s)` for a static string: the inverse of `'image` for integer,
/// real and enumeration types.
fn value_of_string(a: &Analysis, ty: TypeId, s: &Value) -> Option<Value> {
    let arr = s.as_array()?;
    let text: String = arr
        .elems
        .iter()
        .map(|e| char::from_u32(e.as_enum()?))
        .collect::<Option<String>>()?;
    let text = text.trim();
    let base = a.base_type(ty);
    match &a.ty(base).kind {
        TypeKind::Integer(_) => super::constant::parse_integer_literal(text).map(Value::Int),
        TypeKind::Real(_) => super::constant::parse_real_literal(text).map(Value::Real),
        TypeKind::Enum { literals, .. } => {
            let lower = text.to_lowercase();
            literals
                .iter()
                .position(|d| a.decl(*d).spelling.to_lowercase() == lower)
                .and_then(|p| u32::try_from(p).ok())
                .map(Value::Enum)
        }
        TypeKind::Physical { units, .. } => {
            let (num, unit) = text.split_once(char::is_whitespace)?;
            let n = super::constant::parse_integer_literal(num.trim())?;
            let unit = unit.trim().to_lowercase();
            let scale = units.iter().find_map(|u| {
                let d = a.decl(*u);
                match &d.kind {
                    DeclKind::PhysicalUnit { scale, .. } if d.spelling.to_lowercase() == unit => {
                        Some(*scale)
                    }
                    _ => None,
                }
            })?;
            Some(Value::Int(n.checked_mul(scale)?))
        }
        _ => None,
    }
}

/// The static range of a scalar or array attribute prefix for `'range`.
pub fn range_of(a: &Analysis, ty: TypeId, dim: usize, reverse: bool) -> Option<Bounds> {
    let b = if a.class(ty) == TypeClass::Array {
        array_bounds(a, ty, dim)?
    } else {
        a.scalar_range(ty)?
    };
    Some(if reverse { b.reversed() } else { b })
}

/// True when the bounds carry a static value on both ends.
pub fn is_static_bounds(b: &Bounds) -> bool {
    matches!((&b.left, &b.right), (Bound::Static(_), Bound::Static(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for attr in [
            Predefined::Base,
            Predefined::ReverseRange,
            Predefined::LastValue,
            Predefined::PathName,
            Predefined::Event,
        ] {
            assert_eq!(Predefined::from_name(attr.name()), Some(attr));
            assert_eq!(
                Predefined::from_name(&attr.name().to_uppercase()),
                Some(attr)
            );
        }
        assert_eq!(Predefined::from_name("nope"), None);
        assert_eq!(Predefined::Image.arg(), Arg::Required);
        assert_eq!(Predefined::Length.arg(), Arg::Optional);
        assert_eq!(Predefined::Event.arg(), Arg::None);
        assert!(Predefined::Range.is_range());
        assert!(Predefined::Stable.is_signal_valued());
        assert_eq!(Predefined::Event.prefix(), PrefixKind::Signal);
    }
}
