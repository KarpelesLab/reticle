//! How a VHDL subtype becomes an IR type.
//!
//! The IR knows packed bit vectors, unpacked arrays, memories and three
//! simulation-only scalars ([`crate::ir::Type`]). VHDL knows enumerations,
//! integer and physical types, records, multi-dimensional arrays and
//! resolution functions. `Layout` is the bridge: it records, for one
//! subtype, the IR type its values live in and how the bits are arranged,
//! so expression lowering can slice a record field or index an array
//! without consulting the type arena again.
//!
//! # The rules
//!
//! | VHDL subtype                            | IR type            | Encoding |
//! |-----------------------------------------|--------------------|----------|
//! | `std_ulogic`, `std_logic`, `bit`        | `u1`               | `'0'`/`'L'` → 0, `'1'`/`'H'` → 1, `'Z'` → `z`, `'U'`/`'X'`/`'W'`/`'-'` → `x` |
//! | `boolean`                               | `u1`               | `false` → 0, `true` → 1 |
//! | any other enumeration type              | `u<ceil(log2 n)>`  | the literal's position, in declaration order |
//! | `integer` and every integer type/subtype| `s32`              | two's complement |
//! | a physical type (`time`, user units)    | `s64`              | a count of primary units (`time`: femtoseconds) |
//! | `real`                                  | `real`             | simulation only |
//! | `string`                                | `string`           | simulation only (`report`, attributes) |
//! | a 1-D array of a fixed-width element    | one wide vector    | leftmost element in the most significant bits |
//! | a record                                | one wide vector    | first field in the most significant bits |
//!
//! Signedness comes from the type: integer and physical types are signed,
//! and so is an array named `signed` anywhere in its subtype chain — the
//! `ieee.numeric_std` and `ieee.numeric_bit` types, and the Synopsys
//! `std_logic_arith` one, see [`is_signed_array`]. Everything else is
//! unsigned, `std_logic_vector` included; the arithmetic of
//! `ieee.std_logic_signed` is signed because of the package the operator
//! comes from, not because of the operand type.
//!
//! # Enumeration encodings in the output
//!
//! An enumeration that is not `bit`, `boolean` or `std_ulogic` is encoded
//! by declaration order, and the mapping is written onto every net and
//! memory of that type as two attributes, so waveform viewers and reports
//! can name the states:
//!
//! ```text
//! attr enum_type = "state_t"
//! attr enum_literals = "idle,load,run,finish"
//! net %state u2 reg
//! ```
//!
//! # Arrays: wide vectors and memories
//!
//! A one-dimensional array whose element occupies a single bit
//! (`bit_vector`, `std_logic_vector`, an array of `boolean`) becomes one
//! wide net: a dynamic index on it is the IR's `Index` node and a slice is
//! `Slice` or `IndexedSlice`, so no storage object is needed. An array
//! whose element is wider than one bit (an array of vectors, of records or
//! of `integer`) becomes an [`crate::ir::Memory`], which is what a dynamic
//! index into it needs; such an array cannot cross a module boundary and a
//! port of that shape is a diagnostic. Multi-dimensional arrays are not
//! lowered.
//!
//! # What has no representation
//!
//! Access, file and protected types, and `real` or `string` *signals*,
//! have no synthesisable representation; declaring one is `V0707`.

use crate::diag::Diagnostic;
use crate::ir::Type;
use crate::source::Span;
use crate::vhdl::ast::Direction;
use crate::vhdl::sema::{Analysis, Bound, Bounds, TypeClass, TypeId, TypeKind};

use super::codes;

/// What the environment must provide to compute a layout: the analysis,
/// a way to evaluate a generic-dependent bound, and somewhere to report.
pub(crate) trait LayoutEnv<'a> {
    /// The analysis the types come from, with the tree's lifetime.
    fn analysis(&self) -> &'a Analysis;
    /// Evaluates the constraint bound whose expression is at `span`.
    fn eval_bound(&mut self, span: Span) -> Option<i128>;
    /// Reports a problem.
    fn report(&mut self, diag: Diagnostic);
}

/// How one-bit values are spelled in their VHDL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BitKind {
    /// `bit`: `'0'` and `'1'`.
    Bit,
    /// `boolean`: `false` and `true`.
    Boolean,
    /// `std_ulogic` and its subtypes: the nine-state system.
    StdLogic,
}

/// The shape of a lowered subtype.
#[derive(Clone, Debug)]
pub(crate) enum LayoutKind {
    /// A single bit.
    Bit(BitKind),
    /// An enumeration encoded by position, with the literals in order.
    Enum(Vec<String>),
    /// An integer type or subtype.
    Int,
    /// A physical type, counted in primary units.
    Physical,
    /// `real`.
    Real,
    /// `string` or another array of a character type.
    Str,
    /// A one-dimensional array.
    Array(Box<ArrayLayout>),
    /// A record, fields in declaration order.
    Record(Vec<(String, Layout)>),
    /// A bit vector whose internal structure does not matter here: the
    /// width of an instance's port, taken from the module it connects to
    /// when the formal's own subtype mentions the target's generics.
    Opaque,
}

/// The layout of a one-dimensional array.
#[derive(Clone, Debug)]
pub(crate) struct ArrayLayout {
    /// The element's layout.
    pub elem: Layout,
    /// The left bound of the index range.
    pub left: i64,
    /// The index direction.
    pub dir: Direction,
    /// The number of elements.
    pub len: u32,
}

impl ArrayLayout {
    /// The offset of index `i` from the leftmost element, if in range.
    pub(crate) fn offset(&self, i: i64) -> Option<u32> {
        let off = match self.dir {
            Direction::To => i.checked_sub(self.left)?,
            Direction::Downto => self.left.checked_sub(i)?,
        };
        u32::try_from(off).ok().filter(|o| *o < self.len)
    }

    /// The smaller of the two index bounds.
    pub(crate) fn low(&self) -> i64 {
        match self.dir {
            Direction::To => self.left,
            Direction::Downto => self.left - i64::from(self.len) + 1,
        }
    }

    /// The index at offset `o` from the leftmost element.
    pub(crate) fn index_at(&self, o: u32) -> i64 {
        match self.dir {
            Direction::To => self.left + i64::from(o),
            Direction::Downto => self.left - i64::from(o),
        }
    }

    /// The bit range `(hi, lo)` of the element at index `i`.
    pub(crate) fn bits_of(&self, i: i64) -> Option<(u32, u32)> {
        let off = self.offset(i)?;
        let w = self.elem.width;
        let lo = (self.len - 1 - off) * w;
        Some((lo + w - 1, lo))
    }
}

/// One subtype, ready to be lowered.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    /// The subtype this describes.
    pub ty: TypeId,
    /// Its shape.
    pub kind: LayoutKind,
    /// The width in bits; zero for `real` and `string`.
    pub width: u32,
    /// True when arithmetic on the value is two's complement.
    pub signed: bool,
}

impl Layout {
    /// The IR type values of this subtype live in.
    pub(crate) fn ir_type(&self) -> Type {
        match self.kind {
            LayoutKind::Real => Type::Real,
            LayoutKind::Str => Type::String,
            _ => Type::Bits {
                width: self.width,
                signed: self.signed,
            },
        }
    }

    /// True when the value is a bit vector (everything but `real` and
    /// `string`).
    pub(crate) fn is_bits(&self) -> bool {
        !matches!(self.kind, LayoutKind::Real | LayoutKind::Str)
    }

    /// The array layout, when this is a one-dimensional array.
    pub(crate) fn array(&self) -> Option<&ArrayLayout> {
        match &self.kind {
            LayoutKind::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The fields, when this is a record.
    pub(crate) fn record(&self) -> Option<&[(String, Layout)]> {
        match &self.kind {
            LayoutKind::Record(f) => Some(f),
            _ => None,
        }
    }

    /// The bit range `(hi, lo)` of the named record field.
    pub(crate) fn field_bits(&self, name: &str) -> Option<(u32, u32, &Layout)> {
        let fields = self.record()?;
        let mut lo = self.width;
        for (n, f) in fields {
            lo -= f.width;
            if n.eq_ignore_ascii_case(name) {
                return Some((lo + f.width - 1, lo, f));
            }
        }
        None
    }

    /// True when the array should be lowered to an [`crate::ir::Memory`]:
    /// its elements are wider than one bit, so a dynamic index needs
    /// storage rather than a bit select.
    pub(crate) fn wants_memory(&self) -> bool {
        self.array().is_some_and(|a| a.elem.width > 1)
    }

    /// The attributes that record an enumeration encoding, if any.
    pub(crate) fn enum_attrs(&self, a: &Analysis) -> Option<(String, String)> {
        let LayoutKind::Enum(literals) = &self.kind else {
            return None;
        };
        let name = a
            .ty(a.base_type(self.ty))
            .name
            .map_or_else(|| "enumeration".to_owned(), |s| a.name(s).to_owned());
        let mut list = String::new();
        for (i, l) in literals.iter().enumerate() {
            if i > 0 {
                list.push(',');
            }
            list.push_str(l);
        }
        Some((name, list))
    }
}

/// True when `ty` is an array type whose arithmetic is two's complement:
/// `ieee.numeric_std`'s `signed`, `ieee.numeric_bit`'s, or the Synopsys
/// `std_logic_arith` type of the same name.
///
/// The declared name is the only thing that tells the two apart —
/// `unsigned` and `signed` are declared side by side as arrays of the same
/// element type — so the name is what is tested. The whole subtype chain
/// is walked because VHDL-2008 declares `signed` as a resolved subtype of
/// `unresolved_signed`, and a design's own
/// `subtype word is signed(15 downto 0)` adds a further link.
pub(crate) fn is_signed_array(a: &Analysis, ty: TypeId) -> bool {
    let mut cur = ty;
    loop {
        if a.ty(cur).name.is_some_and(|n| {
            let s = a.name(n);
            s.eq_ignore_ascii_case("signed") || s.eq_ignore_ascii_case("unresolved_signed")
        }) {
            return true;
        }
        match a.ty(cur).kind {
            TypeKind::Subtype { parent, .. } => cur = parent,
            _ => return false,
        }
    }
}

/// The number of bits needed to hold `n` distinct positions.
pub(crate) fn encoding_width(n: usize) -> u32 {
    let mut bits = 1;
    while (1usize << bits) < n {
        bits += 1;
    }
    u32::try_from(bits).unwrap_or(32)
}

/// Computes the layout of `ty`, reporting against `span` when it has none.
pub(crate) fn layout_of<'a>(env: &mut dyn LayoutEnv<'a>, ty: TypeId, span: Span) -> Option<Layout> {
    layout_inner(env, ty, span, 0)
}

fn layout_inner<'a>(
    env: &mut dyn LayoutEnv<'a>,
    ty: TypeId,
    span: Span,
    depth: u32,
) -> Option<Layout> {
    if depth > 16 {
        env.report(
            Diagnostic::error("this type nests too deeply to lower")
                .with_code(codes::TYPE)
                .with_span(span),
        );
        return None;
    }
    let a = env.analysis();
    let base = a.base_type(ty);
    let class = a.class(ty);
    let unsupported = |env: &mut dyn LayoutEnv<'a>, what: &str| {
        env.report(
            Diagnostic::error(format!("{what} has no representation in the IR"))
                .with_code(codes::TYPE)
                .with_span(span)
                .with_note("access, file and protected types are not synthesisable"),
        );
        None::<Layout>
    };
    match class {
        TypeClass::Enum => {
            let kind = if a.is_boolean(ty) {
                LayoutKind::Bit(BitKind::Boolean)
            } else if a.same_base(ty, a.builtins.bit) {
                LayoutKind::Bit(BitKind::Bit)
            } else if a.is_std_ulogic(ty) {
                LayoutKind::Bit(BitKind::StdLogic)
            } else {
                let TypeKind::Enum { literals, .. } = &a.ty(base).kind else {
                    return None;
                };
                let names = literals
                    .iter()
                    .map(|d| a.decl(*d).spelling.clone())
                    .collect::<Vec<_>>();
                LayoutKind::Enum(names)
            };
            let width = match &kind {
                LayoutKind::Enum(l) => encoding_width(l.len()),
                _ => 1,
            };
            Some(Layout {
                ty,
                kind,
                width,
                signed: false,
            })
        }
        TypeClass::Integer | TypeClass::UniversalInteger => Some(Layout {
            ty,
            kind: LayoutKind::Int,
            width: 32,
            signed: true,
        }),
        TypeClass::Physical => Some(Layout {
            ty,
            kind: LayoutKind::Physical,
            width: 64,
            signed: true,
        }),
        TypeClass::Real | TypeClass::UniversalReal => Some(Layout {
            ty,
            kind: LayoutKind::Real,
            width: 0,
            signed: false,
        }),
        TypeClass::Record => {
            let raw = a.record_fields(base)?;
            let fields: Vec<(String, TypeId)> = raw
                .iter()
                .map(|f| {
                    let name = a.name(f.name).to_owned();
                    let t = a.field_type(ty, f.name).unwrap_or(f.ty);
                    (name, t)
                })
                .collect();
            let mut out = Vec::with_capacity(fields.len());
            let mut width = 0u32;
            for (name, ft) in fields {
                let l = layout_inner(env, ft, span, depth + 1)?;
                if !l.is_bits() {
                    return unsupported(env, "a record with a `real` or `string` field");
                }
                width = width.saturating_add(l.width);
                out.push((name, l));
            }
            Some(Layout {
                ty,
                kind: LayoutKind::Record(out),
                width,
                signed: false,
            })
        }
        TypeClass::Array => {
            if a.dimensions(ty) != 1 {
                env.report(
                    Diagnostic::error("a multi-dimensional array cannot be lowered yet")
                        .with_code(codes::UNSUPPORTED)
                        .with_span(span),
                );
                return None;
            }
            let elem_ty = a.element_type(ty)?;
            let is_text = a
                .element_type(ty)
                .is_some_and(|e| a.same_base(e, a.builtins.character));
            if is_text && a.array_length(ty).is_none() {
                // An unconstrained string: only usable as a report
                // argument or an attribute value.
                return Some(Layout {
                    ty,
                    kind: LayoutKind::Str,
                    width: 0,
                    signed: false,
                });
            }
            let Some(bounds) = a.index_constraint(ty).and_then(|c| c.first().cloned()) else {
                let name = env.analysis().describe_type(ty, None);
                env.report(
                    Diagnostic::error(format!("`{name}` is unconstrained here"))
                        .with_code(codes::NOT_STATIC)
                        .with_span(span)
                        .with_note("give the object an index constraint so its width is known"),
                );
                return None;
            };
            let (left, right) = resolve_bounds(env, &bounds, span)?;
            let len = match bounds.dir {
                Direction::To => right - left + 1,
                Direction::Downto => left - right + 1,
            };
            let len = u32::try_from(len.max(0)).unwrap_or(0);
            let elem = layout_inner(env, elem_ty, span, depth + 1)?;
            if !elem.is_bits() {
                return unsupported(env, "an array of `real` or `string`");
            }
            let width = elem.width.saturating_mul(len);
            let signed = is_signed_array(env.analysis(), ty);
            let left = i64::try_from(left).unwrap_or(0);
            Some(Layout {
                ty,
                kind: LayoutKind::Array(Box::new(ArrayLayout {
                    elem,
                    left,
                    dir: bounds.dir,
                    len,
                })),
                width,
                signed,
            })
        }
        TypeClass::Access => unsupported(env, "an access type"),
        TypeClass::File => unsupported(env, "a file type"),
        TypeClass::Protected => unsupported(env, "a protected type"),
        TypeClass::Other => {
            if a.is_error(ty) {
                return None;
            }
            unsupported(env, "this type")
        }
    }
}

/// Evaluates both bounds of a constraint, resolving `Bound::Dynamic`
/// against the generics in scope.
fn resolve_bounds<'a>(
    env: &mut dyn LayoutEnv<'a>,
    bounds: &Bounds,
    span: Span,
) -> Option<(i128, i128)> {
    let left = resolve_bound(env, &bounds.left, span)?;
    let right = resolve_bound(env, &bounds.right, span)?;
    Some((left, right))
}

fn resolve_bound<'a>(env: &mut dyn LayoutEnv<'a>, bound: &Bound, span: Span) -> Option<i128> {
    match bound {
        Bound::Static(v) => match v.as_int() {
            Some(i) => Some(i),
            None => {
                env.report(
                    Diagnostic::error("this index bound is not an integer")
                        .with_code(codes::NOT_STATIC)
                        .with_span(span),
                );
                None
            }
        },
        Bound::Dynamic(bspan) => match env.eval_bound(*bspan) {
            Some(i) => Some(i),
            None => {
                env.report(
                    Diagnostic::error("this bound is not static after elaboration")
                        .with_code(codes::NOT_STATIC)
                        .with_label(*bspan, "cannot be evaluated")
                        .with_note("array bounds must follow from the generics"),
                );
                None
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_widths() {
        assert_eq!(encoding_width(1), 1);
        assert_eq!(encoding_width(2), 1);
        assert_eq!(encoding_width(3), 2);
        assert_eq!(encoding_width(4), 2);
        assert_eq!(encoding_width(5), 3);
        assert_eq!(encoding_width(256), 8);
    }

    #[test]
    fn array_bit_ranges() {
        let elem = Layout {
            ty: TypeId(0),
            kind: LayoutKind::Bit(BitKind::Bit),
            width: 1,
            signed: false,
        };
        let a = ArrayLayout {
            elem,
            left: 7,
            dir: Direction::Downto,
            len: 8,
        };
        assert_eq!(a.offset(7), Some(0));
        assert_eq!(a.offset(0), Some(7));
        assert_eq!(a.offset(8), None);
        assert_eq!(a.bits_of(7), Some((7, 7)));
        assert_eq!(a.bits_of(0), Some((0, 0)));
        assert_eq!(a.index_at(0), 7);
    }
}
