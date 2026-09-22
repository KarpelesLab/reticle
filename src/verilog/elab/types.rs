//! The Verilog / SystemVerilog type model used by elaboration.
//!
//! Every declared object gets a [`VType`] once its dimensions have been
//! evaluated. The model is deliberately small:
//!
//! - [`VType::Packed`] covers everything integral: scalars, vectors with
//!   one or more packed dimensions, the integer atoms (`int`, `integer`,
//!   `byte`, ...), packed structs (with per-field bit offsets) and enums
//!   (with their variant table). A packed value is a bit vector of a known
//!   width and signedness, which is what the IR's `Type::Bits` carries.
//! - [`VType::Unpacked`] is one unpacked dimension over an element type;
//!   multi-dimensional arrays nest, outermost dimension first.
//! - `Real`, `String`, `Event`, `Chandle` and `Void` are simulation-only
//!   types; they lower to the IR's `Real` / `String` types where that is
//!   possible and are otherwise rejected with a diagnostic.
//! - [`VType::Interface`] names an interface bundle for interface ports.
//!
//! Index and range arithmetic lives here too ([`Range`]), since every
//! bit-select and part-select has to be translated from the declared
//! `[msb:lsb]` numbering to the IR's zero-based, LSB-first numbering.
//!
//! # Rules applied (IEEE 1800-2017 §6, §7; IEEE 1364-2005 §4)
//!
//! - `[N]` as a packed dimension means `[N-1:0]`; as an unpacked dimension
//!   it means `[0:N-1]` (§7.4.2).
//! - `integer`, `int`, `shortint`, `longint`, `byte` are signed; `time` is
//!   unsigned; `bit`, `logic`, `reg` and implicit types are unsigned unless
//!   `signed` is written (§6.8, §6.11).
//! - A packed struct's first member occupies the most significant bits
//!   (§7.2.1); a packed struct is signed only when declared so.
//! - Enum variants without an explicit value continue from the previous
//!   variant plus one, starting at zero (§6.19); the base type defaults to
//!   `int`; `NAME[N]` and `NAME[lo:hi]` generate numbered variants.

use std::fmt;
use std::rc::Rc;

use crate::ir::Type;
use crate::logic::Logic;
use crate::source::Span;
use crate::verilog::ast;

use super::codes;
use super::constant::{Evaluator, Value};
use super::env::ScopeEnv;
use super::scope::Symbol;

/// A declared `[left:right]` dimension.
///
/// `left` is the index written first (the most significant end of a packed
/// dimension, the first element of an unpacked one); either end may be the
/// larger number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    /// The bound written first.
    pub left: i64,
    /// The bound written second.
    pub right: i64,
}

impl Range {
    /// Builds a range from its written bounds.
    pub fn new(left: i64, right: i64) -> Self {
        Range { left, right }
    }

    /// The range of a packed `[N]` dimension: `[N-1:0]`.
    pub fn packed_size(n: i64) -> Self {
        Range {
            left: n - 1,
            right: 0,
        }
    }

    /// The range of an unpacked `[N]` dimension: `[0:N-1]`.
    pub fn unpacked_size(n: i64) -> Self {
        Range {
            left: 0,
            right: n - 1,
        }
    }

    /// Number of indices covered.
    pub fn len(&self) -> u64 {
        self.left.abs_diff(self.right) + 1
    }

    /// True when the range covers no index; never for a well-formed range.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// True for `[7:0]`-style ranges, where the left bound is the larger.
    pub fn is_descending(&self) -> bool {
        self.left >= self.right
    }

    /// The smaller bound.
    pub fn low(&self) -> i64 {
        self.left.min(self.right)
    }

    /// The larger bound.
    pub fn high(&self) -> i64 {
        self.left.max(self.right)
    }

    /// True when `i` is one of the range's indices.
    pub fn contains(&self, i: i64) -> bool {
        i >= self.low() && i <= self.high()
    }

    /// The zero-based position of index `i` inside an *unpacked*
    /// dimension: the lowest index is element zero, whichever way the
    /// range runs, so `mem[0]` of `[0:255]` and of `[255:0]` are both
    /// element zero. `None` when `i` is outside the range.
    pub fn element_offset(&self, i: i64) -> Option<u64> {
        self.contains(i).then(|| i.abs_diff(self.low()))
    }

    /// The zero-based position of index `i` counted from the right bound:
    /// bit 0 of the IR vector for a packed dimension. `None` when `i` is
    /// outside the range.
    pub fn offset(&self, i: i64) -> Option<u64> {
        if !self.contains(i) {
            return None;
        }
        Some(if self.is_descending() {
            i.abs_diff(self.right)
        } else {
            self.right.abs_diff(i)
        })
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}:{}]", self.left, self.right)
    }
}

/// One member of a packed struct.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The member name.
    pub name: String,
    /// The member type, always packed.
    pub ty: VType,
    /// Bit offset of the member's least significant bit inside the struct.
    pub lsb: u32,
    /// Where the member was declared.
    pub span: Span,
}

/// The variant table of an enum type.
#[derive(Clone, Debug, PartialEq)]
pub struct Enumeration {
    /// Variants in declaration order with their values, sized to the base
    /// type.
    pub variants: Vec<(String, Logic)>,
}

impl Enumeration {
    /// The name of the variant holding `value`, if any.
    pub fn name_of(&self, value: &Logic) -> Option<&str> {
        self.variants
            .iter()
            .find(|(_, v)| v.width() == value.width() && v.value_words() == value.value_words())
            .map(|(n, _)| n.as_str())
    }
}

/// An integral type: a bit vector with structure.
#[derive(Clone, Debug, PartialEq)]
pub struct Packed {
    /// True for two's complement interpretation.
    pub signed: bool,
    /// True for `logic` / `reg` / `integer` and nets (may hold `x`/`z`);
    /// false for `bit` / `int` / `byte` and friends.
    pub four_state: bool,
    /// The packed dimensions, outermost first. Empty for a one-bit scalar;
    /// the integer atoms carry their implicit `[N-1:0]`.
    pub dims: Vec<Range>,
    /// Struct members, for a packed struct or union.
    pub fields: Option<Rc<Vec<Field>>>,
    /// The variant table, for an enum.
    pub enumeration: Option<Rc<Enumeration>>,
    /// True when the width was not written but taken from a value (an
    /// untyped `parameter P = 5`, a genvar, an integer literal). Such
    /// values count as only as wide as they need to be when checking for
    /// truncation, since every `x <= 0` would otherwise warn.
    pub is_unsized: bool,
    /// The keyword the type was declared with, for messages (`int`,
    /// `logic`); `None` for implicit types.
    pub keyword: Option<&'static str>,
}

impl Packed {
    /// An unsigned four-state vector of `width` bits.
    pub fn bits(width: u32) -> Self {
        Packed {
            signed: false,
            four_state: true,
            dims: if width == 1 {
                Vec::new()
            } else {
                vec![Range::packed_size(i64::from(width))]
            },
            fields: None,
            enumeration: None,
            is_unsized: false,
            keyword: None,
        }
    }

    /// A signed four-state vector of `width` bits.
    pub fn sbits(width: u32) -> Self {
        let mut p = Self::bits(width);
        p.signed = true;
        p
    }

    /// The 32-bit signed `integer` type.
    pub fn integer() -> Self {
        let mut p = Self::sbits(32);
        p.keyword = Some("integer");
        p
    }

    /// The type of an unsized value: 32-bit signed integer flagged
    /// `unsized`.
    pub fn unsized_int() -> Self {
        let mut p = Self::integer();
        p.is_unsized = true;
        p.keyword = None;
        p
    }

    /// The width in bits: the product of the packed dimensions, or one.
    pub fn width(&self) -> u32 {
        let w = self
            .dims
            .iter()
            .fold(1u64, |acc, r| acc.saturating_mul(r.len()));
        u32::try_from(w).unwrap_or(u32::MAX)
    }

    /// The outermost dimension, or `[width-1:0]` for a type declared
    /// without one.
    pub fn outer(&self) -> Range {
        self.dims
            .first()
            .copied()
            .unwrap_or_else(|| Range::packed_size(i64::from(self.width())))
    }

    /// The type of one element of the outermost dimension: a bit for a
    /// vector, a narrower vector for a multi-dimensional packed array.
    pub fn element(&self) -> Packed {
        let dims = self
            .dims
            .get(1..)
            .map(<[Range]>::to_vec)
            .unwrap_or_default();
        Packed {
            signed: false,
            four_state: self.four_state,
            dims,
            fields: None,
            enumeration: None,
            is_unsized: false,
            keyword: None,
        }
    }

    /// The same type re-flagged as signed or unsigned.
    pub fn with_signed(&self, signed: bool) -> Packed {
        let mut p = self.clone();
        p.signed = signed;
        p
    }

    /// The member named `name` of a packed struct.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields
            .as_ref()
            .and_then(|f| f.iter().find(|f| f.name == name))
    }

    /// The IR type: a bit vector of this width and signedness.
    pub fn ir_type(&self) -> Type {
        Type::Bits {
            width: self.width(),
            signed: self.signed,
        }
    }
}

/// The type of a declared object or expression.
#[derive(Clone, Debug, PartialEq)]
pub enum VType {
    /// An integral value.
    Packed(Packed),
    /// One unpacked dimension over an element type.
    Unpacked {
        /// The element type.
        elem: Box<VType>,
        /// The dimension.
        range: Range,
    },
    /// `real`, `shortreal`, `realtime`.
    Real,
    /// `string`.
    String,
    /// `event`.
    Event,
    /// `chandle`.
    Chandle,
    /// `void` (function return type).
    Void,
    /// An interface bundle, for interface ports and instances.
    Interface {
        /// The interface name; empty for a generic `interface` port.
        name: String,
        /// The selected modport.
        modport: Option<String>,
    },
}

impl VType {
    /// A one-bit four-state unsigned scalar.
    pub fn bit() -> Self {
        VType::Packed(Packed::bits(1))
    }

    /// An unsigned four-state vector.
    pub fn bits(width: u32) -> Self {
        VType::Packed(Packed::bits(width))
    }

    /// The `integer` type.
    pub fn integer() -> Self {
        VType::Packed(Packed::integer())
    }

    /// The packed view, for integral types.
    pub fn packed(&self) -> Option<&Packed> {
        match self {
            VType::Packed(p) => Some(p),
            _ => None,
        }
    }

    /// True for integral types.
    pub fn is_integral(&self) -> bool {
        matches!(self, VType::Packed(_))
    }

    /// `$bits`: the number of bits a value of this type occupies; `None`
    /// for types without a bit representation.
    pub fn bit_size(&self) -> Option<u64> {
        match self {
            VType::Packed(p) => Some(u64::from(p.width())),
            VType::Unpacked { elem, range } => {
                elem.bit_size().map(|b| b.saturating_mul(range.len()))
            }
            VType::Real => Some(64),
            VType::String
            | VType::Event
            | VType::Chandle
            | VType::Void
            | VType::Interface { .. } => None,
        }
    }

    /// The innermost element type of an unpacked array (the type itself
    /// when it is not an array).
    pub fn base(&self) -> &VType {
        match self {
            VType::Unpacked { elem, .. } => elem.base(),
            other => other,
        }
    }

    /// The unpacked dimensions, outermost first.
    pub fn unpacked_dims(&self) -> Vec<Range> {
        let mut dims = Vec::new();
        let mut t = self;
        while let VType::Unpacked { elem, range } = t {
            dims.push(*range);
            t = elem;
        }
        dims
    }

    /// The IR type: bit vectors for packed values, nested arrays for
    /// unpacked ones, `Real` and `String` for those; `None` for types the
    /// IR cannot carry (events, handles, interfaces, void).
    pub fn ir_type(&self) -> Option<Type> {
        match self {
            VType::Packed(p) => Some(p.ir_type()),
            VType::Unpacked { elem, range } => Some(Type::array(elem.ir_type()?, range.len())),
            VType::Real => Some(Type::Real),
            VType::String => Some(Type::String),
            VType::Event | VType::Chandle | VType::Void | VType::Interface { .. } => None,
        }
    }
}

impl fmt::Display for VType {
    /// Renders the type the way it would be declared: `logic [7:0]`,
    /// `int`, `logic [7:0] [0:3]`, `real`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VType::Packed(p) => {
                if p.enumeration.is_some() {
                    f.write_str("enum ")?;
                } else if p.fields.is_some() {
                    f.write_str("struct packed ")?;
                }
                if let Some(kw) = p.keyword {
                    f.write_str(kw)?;
                    if p.signed
                        && kw != "integer"
                        && !matches!(kw, "int" | "byte" | "shortint" | "longint")
                    {
                        f.write_str(" signed")?;
                    }
                    if !p.signed
                        && matches!(kw, "int" | "byte" | "shortint" | "longint" | "integer")
                    {
                        f.write_str(" unsigned")?;
                    }
                    if matches!(kw, "logic" | "reg" | "bit" | "wire") {
                        for d in &p.dims {
                            write!(f, " {d}")?;
                        }
                    }
                    Ok(())
                } else {
                    if p.signed {
                        f.write_str("signed")?;
                        if !p.dims.is_empty() {
                            f.write_str(" ")?;
                        }
                    }
                    if p.dims.is_empty() {
                        if !p.signed {
                            f.write_str("1-bit")?;
                        }
                        return Ok(());
                    }
                    for (i, d) in p.dims.iter().enumerate() {
                        if i > 0 {
                            f.write_str(" ")?;
                        }
                        write!(f, "{d}")?;
                    }
                    Ok(())
                }
            }
            VType::Unpacked { elem, range } => write!(f, "{elem} {range}"),
            VType::Real => f.write_str("real"),
            VType::String => f.write_str("string"),
            VType::Event => f.write_str("event"),
            VType::Chandle => f.write_str("chandle"),
            VType::Void => f.write_str("void"),
            VType::Interface { name, modport } => {
                if name.is_empty() {
                    f.write_str("interface")?;
                } else {
                    f.write_str(name)?;
                }
                if let Some(m) = modport {
                    write!(f, ".{m}")?;
                }
                Ok(())
            }
        }
    }
}

/// Number of bits needed to represent `v` (at least one).
pub fn bits_needed(v: u64) -> u32 {
    (64 - v.leading_zeros()).max(1)
}

/// Number of bits needed to represent the fully known value `v` as an
/// unsigned quantity, or as a signed one when it is negative; at least
/// one. Values with unknown bits or wider than 64 bits report their full
/// width.
pub fn min_bits(v: &Logic) -> u32 {
    if v.is_negative() {
        return v
            .to_i64()
            .map_or(v.width(), |i| bits_needed(i.unsigned_abs()) + 1)
            .min(v.width());
    }
    if v.has_unknown() {
        return v.width();
    }
    match v.to_u64() {
        Some(u) => bits_needed(u).min(v.width().max(1)),
        None => v.width(),
    }
}

// ---------------------------------------------------------------------------
// Resolution of AST data types
// ---------------------------------------------------------------------------

/// The width, signedness and state of each integer keyword (§6.11).
fn atom(t: ast::IntegerType) -> Packed {
    let (width, signed, four_state) = match t {
        ast::IntegerType::Bit => (1, false, false),
        ast::IntegerType::Logic | ast::IntegerType::Reg => (1, false, true),
        ast::IntegerType::Byte => (8, true, false),
        ast::IntegerType::Shortint => (16, true, false),
        ast::IntegerType::Int => (32, true, false),
        ast::IntegerType::Longint => (64, true, false),
        ast::IntegerType::Integer => (32, true, true),
        ast::IntegerType::Time => (64, false, true),
    };
    let mut p = if signed {
        Packed::sbits(width)
    } else {
        Packed::bits(width)
    };
    p.four_state = four_state;
    p.keyword = Some(t.as_str());
    p
}

/// Evaluates one dimension to a [`Range`]; `packed` selects the meaning of
/// the `[N]` form (§7.4.2).
fn dimension(env: &mut ScopeEnv<'_, '_>, dim: &ast::Dim, packed: bool) -> Option<Range> {
    match &dim.kind {
        ast::DimKind::Range(a, b) => {
            let mut ev = Evaluator::new(env);
            let left = ev.eval_i64(a).ok()?;
            let right = ev.eval_i64(b).ok()?;
            Some(Range::new(left, right))
        }
        ast::DimKind::Size(n) => {
            let mut ev = Evaluator::new(env);
            let size = ev.eval_i64(n).ok()?;
            if size <= 0 {
                env.error(
                    codes::CONST_EVAL,
                    dim.span,
                    format!("dimension size must be positive, found {size}"),
                );
                return None;
            }
            Some(if packed {
                Range::packed_size(size)
            } else {
                Range::unpacked_size(size)
            })
        }
        ast::DimKind::Unsized => {
            env.unsupported(dim.span, "dynamic array dimension `[]`");
            None
        }
        ast::DimKind::Queue(_) => {
            env.unsupported(dim.span, "queue dimension `[$]`");
            None
        }
        ast::DimKind::Assoc(_) => {
            env.unsupported(dim.span, "associative array dimension");
            None
        }
    }
}

/// Resolves an AST data type with its unpacked dimensions to a [`VType`].
///
/// `dims` are the unpacked dimensions written after the declared name.
/// When `allow_unpacked` is false an unpacked dimension is reported as an
/// error (parameters of a port list, function return types).
pub(crate) fn resolve(
    env: &mut ScopeEnv<'_, '_>,
    dt: &ast::DataType,
    dims: &[ast::Dim],
    allow_unpacked: bool,
) -> Option<VType> {
    let mut base = base_type(env, dt)?;

    // Signing overrides what the keyword implies (§6.8).
    if let Some(sign) = dt.signing {
        let signed = sign == ast::Signing::Signed;
        match &mut base {
            VType::Packed(p) => p.signed = signed,
            other => {
                env.error(
                    codes::TYPE,
                    dt.span,
                    format!("`{}` cannot be declared signed or unsigned", other),
                );
            }
        }
    }

    // Packed dimensions apply outermost first.
    if !dt.packed.is_empty() {
        let VType::Packed(p) = &mut base else {
            env.error(
                codes::TYPE,
                dt.span,
                format!("`{base}` cannot have packed dimensions"),
            );
            return None;
        };
        let mut ranges = Vec::with_capacity(dt.packed.len());
        for d in &dt.packed {
            ranges.push(dimension(env, d, true)?);
        }
        // A scalar base contributes no dimension of its own; a wider one
        // (an `int`, a struct) keeps its dimensions innermost.
        let mut dims_new = ranges;
        dims_new.extend(p.dims.iter().copied());
        if p.dims.is_empty() && p.width() > 1 {
            dims_new.push(Range::packed_size(i64::from(p.width())));
        }
        p.dims = dims_new;
        p.is_unsized = false;
    }

    // Unpacked dimensions wrap the type, outermost first.
    if dims.is_empty() {
        return Some(base);
    }
    if !allow_unpacked {
        env.error(
            codes::UNSUPPORTED,
            dims[0].span,
            "an unpacked dimension is not allowed here",
        );
        return None;
    }
    let mut ranges = Vec::with_capacity(dims.len());
    for d in dims {
        ranges.push(dimension(env, d, false)?);
    }
    let mut ty = base;
    for r in ranges.into_iter().rev() {
        ty = VType::Unpacked {
            elem: Box::new(ty),
            range: r,
        };
    }
    Some(ty)
}

/// Resolves the base of a data type, before signing and dimensions.
fn base_type(env: &mut ScopeEnv<'_, '_>, dt: &ast::DataType) -> Option<VType> {
    match &dt.kind {
        ast::DataTypeKind::Implicit => Some(VType::Packed(Packed::bits(1))),
        ast::DataTypeKind::Integer(t) => Some(VType::Packed(atom(*t))),
        ast::DataTypeKind::Real(_) => Some(VType::Real),
        ast::DataTypeKind::String => Some(VType::String),
        ast::DataTypeKind::Chandle => Some(VType::Chandle),
        ast::DataTypeKind::Event => Some(VType::Event),
        ast::DataTypeKind::Void => Some(VType::Void),
        ast::DataTypeKind::Enum(e) => enum_type(env, e, dt.span),
        ast::DataTypeKind::Struct(s) => struct_type(env, s, dt.span),
        ast::DataTypeKind::Named {
            package,
            name,
            member,
        } => named_type(env, package.as_ref(), name, member.as_ref()),
        ast::DataTypeKind::Interface { modport } => Some(VType::Interface {
            name: String::new(),
            modport: modport.as_ref().map(|m| m.name.clone()),
        }),
        ast::DataTypeKind::TypeOf(e) => {
            if let ast::ExprKind::Type(inner) = &e.kind {
                return resolve(env, inner, &[], true);
            }
            if let Some(ty) = env.type_of_path(e) {
                return Some(ty);
            }
            match super::width::info(e, env, &[])? {
                super::width::Info::Bits { width, signed, .. } => Some(VType::Packed(if signed {
                    Packed::sbits(width)
                } else {
                    Packed::bits(width)
                })),
                super::width::Info::Real => Some(VType::Real),
                super::width::Info::Str(_) => Some(VType::String),
                super::width::Info::Other => None,
            }
        }
    }
}

/// Resolves `T`, `pkg::T` or an interface port type `bus_if.master`.
fn named_type(
    env: &mut ScopeEnv<'_, '_>,
    package: Option<&ast::Ident>,
    name: &ast::Ident,
    member: Option<&ast::Ident>,
) -> Option<VType> {
    if let Some(pkg) = package {
        return match env.resolve_scoped(pkg, name) {
            Some(Symbol::Type(ty)) => Some(ty),
            Some(_) => {
                env.error(
                    codes::TYPE,
                    name.span,
                    format!("`{}::{}` is not a type", pkg.name, name.name),
                );
                None
            }
            None => None,
        };
    }
    match env.lookup(&name.name) {
        Some((Symbol::Type(ty), _, _)) => {
            let ty = ty.clone();
            if let Some(m) = member {
                env.error(
                    codes::TYPE,
                    m.span,
                    format!("`{}` is a type; `.{}` is not valid here", name.name, m.name),
                );
            }
            Some(ty)
        }
        Some((Symbol::Iface { name: iface, .. }, _, _)) => {
            let iface = iface.clone();
            Some(VType::Interface {
                name: iface,
                modport: member.map(|m| m.name.clone()),
            })
        }
        _ => {
            // An interface used as a port type is known from the source
            // table rather than from a scope.
            if env.cx.table.is_interface(&name.name) {
                return Some(VType::Interface {
                    name: name.name.clone(),
                    modport: member.map(|m| m.name.clone()),
                });
            }
            env.undefined(name);
            None
        }
    }
}

/// Builds an enum type and declares its variants in the current scope
/// (§6.19).
fn enum_type(env: &mut ScopeEnv<'_, '_>, e: &ast::EnumType, span: Span) -> Option<VType> {
    let base = match &e.base {
        Some(dt) => resolve(env, dt, &[], false)?,
        None => VType::Packed(Packed::integer()),
    };
    let Some(p) = base.packed().cloned() else {
        env.error(codes::TYPE, span, "an enum base type must be integral");
        return None;
    };
    let width = p.width();
    let mut variants: Vec<(String, Logic, Span)> = Vec::new();
    let mut next = Logic::zero(width).with_signed(p.signed);
    let one = Logic::from_u64(1, width).with_signed(p.signed);
    for v in &e.variants {
        // `NAME[N]` and `NAME[lo:hi]` generate numbered variants.
        let indices: Vec<i64> = match &v.range {
            None => Vec::new(),
            Some(dim) => {
                let r = dimension(env, dim, false)?;
                let (lo, hi) = if matches!(dim.kind, ast::DimKind::Size(_)) {
                    (0, i64::try_from(r.len()).unwrap_or(0) - 1)
                } else {
                    (r.low(), r.high())
                };
                (lo..=hi).collect()
            }
        };
        if let Some(value) = &v.value {
            let mut ev = Evaluator::new(env);
            let l = ev.eval_logic(value, Some(width)).ok()?;
            next = l.resize(width).with_signed(p.signed);
        }
        if indices.is_empty() {
            variants.push((v.name.name.clone(), next.clone(), v.span));
            next = next.add(&one);
        } else {
            for i in indices {
                variants.push((format!("{}{i}", v.name.name), next.clone(), v.span));
                next = next.add(&one);
            }
        }
    }
    let enumeration = Rc::new(Enumeration {
        variants: variants
            .iter()
            .map(|(n, v, _)| (n.clone(), v.clone()))
            .collect(),
    });
    let mut ty = p;
    ty.enumeration = Some(enumeration);
    ty.is_unsized = false;
    let vtype = VType::Packed(ty);
    for (name, value, span) in variants {
        // Re-resolving the same `typedef enum` must not report the
        // variants as duplicates, so the declaration is silent.
        let sym = Symbol::Const {
            value: Value::Logic(value),
            ty: Some(vtype.clone()),
        };
        let scope = env.scope;
        env.cx.scopes.redeclare(scope, name, sym, span);
    }
    Some(vtype)
}

/// Builds a packed struct or union type with per-field bit offsets
/// (§7.2.1): the first member occupies the most significant bits.
fn struct_type(env: &mut ScopeEnv<'_, '_>, s: &ast::StructType, span: Span) -> Option<VType> {
    if !s.packed {
        env.unsupported(span, "unpacked struct or union");
        return None;
    }
    let mut fields: Vec<Field> = Vec::new();
    let mut widths: Vec<u32> = Vec::new();
    for m in &s.members {
        for d in &m.decls {
            let ty = resolve(env, &m.data_type, &d.dims, false)?;
            let Some(p) = ty.packed() else {
                env.error(
                    codes::TYPE,
                    d.span,
                    format!(
                        "member `{}` of a packed struct must be integral",
                        d.name.name
                    ),
                );
                return None;
            };
            widths.push(p.width());
            fields.push(Field {
                name: d.name.name.clone(),
                ty: ty.clone(),
                lsb: 0,
                span: d.span,
            });
        }
    }
    if fields.is_empty() {
        env.error(codes::TYPE, span, "a packed struct must have members");
        return None;
    }
    let total: u32 = widths.iter().copied().fold(0u32, u32::saturating_add);
    if s.is_union {
        // Every member of a packed union starts at bit zero and they all
        // have the same width (§7.3.1); the widest wins if they differ.
        let width = widths.iter().copied().max().unwrap_or(0);
        for f in &mut fields {
            f.lsb = 0;
        }
        let mut p = Packed::bits(width);
        p.fields = Some(Rc::new(fields));
        p.signed = s.tagged && false;
        return Some(VType::Packed(p));
    }
    let mut offset = total;
    for (f, w) in fields.iter_mut().zip(&widths) {
        offset -= w;
        f.lsb = offset;
    }
    let mut p = Packed::bits(total);
    p.fields = Some(Rc::new(fields));
    Some(VType::Packed(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_map_indices() {
        let d = Range::new(7, 0);
        assert!(d.is_descending());
        assert_eq!(d.len(), 8);
        assert_eq!(d.offset(0), Some(0));
        assert_eq!(d.offset(7), Some(7));
        assert_eq!(d.offset(8), None);
        let a = Range::new(0, 7);
        assert!(!a.is_descending());
        assert_eq!(a.offset(0), Some(7));
        assert_eq!(a.offset(7), Some(0));
        let off = Range::new(4, 1);
        assert_eq!(off.offset(1), Some(0));
        assert_eq!(off.offset(4), Some(3));
        assert_eq!(off.offset(0), None);
        assert_eq!(d.element_offset(3), Some(3));
        assert_eq!(a.element_offset(3), Some(3));
        assert_eq!(Range::new(4, 1).element_offset(1), Some(0));
        assert_eq!(d.element_offset(9), None);
        assert_eq!(Range::packed_size(8), Range::new(7, 0));
        assert_eq!(Range::unpacked_size(8), Range::new(0, 7));
        assert_eq!(Range::new(-2, 2).len(), 5);
        assert_eq!(Range::new(-2, 2).low(), -2);
        assert_eq!(Range::new(-2, 2).high(), 2);
        assert!(!Range::new(0, 0).is_empty());
        assert_eq!(Range::new(3, 0).to_string(), "[3:0]");
    }

    #[test]
    fn packed_widths_and_elements() {
        let scalar = Packed::bits(1);
        assert_eq!(scalar.width(), 1);
        assert!(scalar.dims.is_empty());
        assert_eq!(scalar.outer(), Range::new(0, 0));
        let mut two_d = Packed::bits(1);
        two_d.dims = vec![Range::new(1, 0), Range::new(7, 0)];
        assert_eq!(two_d.width(), 16);
        assert_eq!(two_d.element().width(), 8);
        assert_eq!(two_d.element().element().width(), 1);
        assert_eq!(Packed::integer().width(), 32);
        assert!(Packed::integer().signed);
        assert!(Packed::unsized_int().is_unsized);
        assert_eq!(Packed::sbits(4).with_signed(false).ir_type(), Type::bits(4));
    }

    #[test]
    fn vtype_bits_and_ir() {
        let v = VType::Unpacked {
            elem: Box::new(VType::bits(8)),
            range: Range::new(0, 3),
        };
        assert_eq!(v.bit_size(), Some(32));
        assert_eq!(v.ir_type(), Some(Type::array(Type::bits(8), 4)));
        assert_eq!(v.base(), &VType::bits(8));
        assert_eq!(v.unpacked_dims(), vec![Range::new(0, 3)]);
        assert_eq!(v.to_string(), "[7:0] [0:3]");
        assert_eq!(VType::Real.bit_size(), Some(64));
        assert_eq!(VType::String.bit_size(), None);
        assert_eq!(VType::Event.ir_type(), None);
        assert_eq!(VType::integer().to_string(), "integer");
        assert_eq!(VType::bit().to_string(), "1-bit");
        assert!(VType::bit().is_integral());
        assert_eq!(
            VType::Interface {
                name: "bus_if".into(),
                modport: Some("master".into())
            }
            .to_string(),
            "bus_if.master"
        );
    }

    #[test]
    fn minimal_widths() {
        assert_eq!(bits_needed(0), 1);
        assert_eq!(bits_needed(1), 1);
        assert_eq!(bits_needed(255), 8);
        assert_eq!(bits_needed(256), 9);
        assert_eq!(min_bits(&Logic::from_u64(5, 32)), 3);
        assert_eq!(min_bits(&Logic::from_i64(-1, 32)), 2);
        assert_eq!(min_bits(&Logic::x(8)), 8);
        assert_eq!(min_bits(&Logic::from_u64(0, 1)), 1);
    }

    #[test]
    fn enumeration_lookup() {
        let e = Enumeration {
            variants: vec![
                ("A".into(), Logic::from_u64(0, 2)),
                ("B".into(), Logic::from_u64(1, 2)),
            ],
        };
        assert_eq!(e.name_of(&Logic::from_u64(1, 2)), Some("B"));
        assert_eq!(e.name_of(&Logic::from_u64(1, 3)), None);
    }
}
