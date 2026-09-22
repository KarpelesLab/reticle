//! Semantic analysis of VHDL (IEEE 1076-2008 clauses 5 to 12).
//!
//! The parser produces an unresolved [`ast::DesignFile`]. This module
//! resolves every name in it to a declaration, gives every expression a
//! type, evaluates whatever is locally static, and checks the rules of the
//! language that do not need elaboration (generic values, generate
//! unrolling and instance binding are the follow-up `elab` pass).
//!
//! # Output shape: an annotated AST, not a new tree
//!
//! Analysis does not rewrite the tree. It returns an [`Analysis`] holding
//! three arenas and a set of side tables keyed by the [`Span`] of the AST
//! node they annotate (the tree has no node ids, and every node's span is
//! unique to it within a file):
//!
//! | Table | Key | Value |
//! |---|---|---|
//! | [`Analysis::decl_of`] | span of a name, identifier or designator | the [`DeclId`] it denotes |
//! | [`Analysis::type_of`] | span of an expression, subtype indication, or declared name | its [`TypeId`] |
//! | [`Analysis::value_of`] | span of an expression | its locally static [`Value`] |
//! | [`Analysis::call_of`] | span of a call or operator expression | the [`CallTarget`]: a subprogram or a predefined operation |
//! | [`Analysis::range_of`] | span of a discrete range | its resolved [`Bounds`] and index type |
//!
//! A lowering pass therefore walks the AST exactly as the parser built it
//! and asks the tables as it goes: on an `Expr::Name` it calls `decl_of`
//! to learn which signal it reads and `type_of` for its width; on an
//! `Expr::Binary` it calls `call_of` to learn whether `+` is the predefined
//! integer addition, `numeric_std."+"` or a user function, and `type_of`
//! for the result; on an instantiation it looks up the entity through
//! [`Analysis::unit`] and reads its ports as declarations. Declarations are
//! reached through [`Region`]s: every declarative part (package, entity,
//! architecture, process, subprogram, block, generate body, protected type)
//! is a region listing its declarations in order, so a lowering pass
//! enumerates an architecture's signals without re-parsing declarations.
//!
//! Constraints whose bounds depend on generics are kept as
//! [`Bound::Dynamic`] with the bound expression's span, so elaboration
//! evaluates them after binding generics and never needs to touch the
//! type arena.
//!
//! # The passes
//!
//! 1. [`library::Design`] collects parsed files per library (the bundled
//!    `std` and `ieee` sources from [`super::stdlib`] first), builds the
//!    unit table and orders the units so that every package precedes its
//!    body and its users and every entity precedes its architectures
//!    (clause 13.5: a unit is analysed after the units it depends on).
//! 2. [`check`] analyses each unit in that order against a [`scope`] of
//!    regions. Names are looked up per clause 12.4 (directly visible
//!    declarations in the innermost region first, potentially visible
//!    `use`d declarations afterwards, homographs of overloadable
//!    declarations accumulating across regions). Expressions are typed in
//!    two phases: [`expr`] first infers the set of candidate types of an
//!    expression bottom-up without reporting anything, then resolves it
//!    top-down against the type the context expects, which is how
//!    overloaded operators, enumeration literals shared between types
//!    (`'0'` of `bit` and of `std_ulogic`) and string literals are
//!    disambiguated (clause 12.5). [`overload`] holds the candidate
//!    filtering, [`attrs`] the predefined attributes, and [`constant`] the
//!    static evaluator.
//!
//! # Bundled libraries, and what is missing
//!
//! Analysis needs `std.standard` to exist: `boolean`, `integer` and the
//! other predefined types are ordinary declarations, and
//! [`Design::with_stdlib`] loads them from [`crate::vhdl::stdlib`] before
//! anything else. What is shipped today:
//!
//! | Library | Package | State |
//! |---|---|---|
//! | `std` | `standard` | complete |
//! | `std` | `textio` | declarations complete, subprograms `foreign` |
//! | `std` | `env` | complete, subprograms `foreign` |
//! | `ieee` | `std_logic_1164` | complete, with a body |
//!
//! Not bundled yet: `ieee.numeric_std`, `ieee.numeric_bit`,
//! `ieee.math_real`, `ieee.std_logic_textio` and the Synopsys legacy
//! packages (`std_logic_arith`, `std_logic_unsigned`,
//! `std_logic_signed`). A `use` clause naming one of them produces a
//! single `V0107` error saying the package is not bundled, and analysis
//! carries on with the rest of the design rather than cascading into
//! unknown-identifier errors for every `unsigned` and `to_integer` that
//! follows. [`crate::vhdl::stdlib::missing_package_note`] holds the list.
//!
//! Because `numeric_std` is missing, arithmetic on `unsigned` and
//! `signed` cannot be analysed yet; [`constant`] already implements the
//! static folding for it (see [`builtin`]), so adding the package is a
//! matter of writing the source, not of changing the analyser.
//!
//! # Diagnostics
//!
//! Every problem is a [`crate::diag::Diagnostic`] with the offending span as
//! primary label, the declaration involved as a secondary label where that
//! helps, and notes with "did you mean" suggestions computed over the names
//! visible at that point. Missing `use` clauses are detected by searching
//! every analysed package for the unknown name.

pub mod attrs;
pub mod builtin;
pub mod check;
pub mod constant;
pub mod expr;
pub mod library;
pub mod name;
pub mod overload;
pub mod scope;
pub mod stmt;
pub mod types;

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::intern::{Interner, Symbol};
use crate::source::{SourceId, SourceMap, Span};
use crate::vhdl::ast::{self, Direction, Mode};

pub use constant::Value;
pub use library::{Design, LibraryUnitKind, Unit, UnitId};
pub use types::{Bound, Bounds, Constraint, Field, Type, TypeClass, TypeId, TypeKind};

/// Index of a [`Decl`] in the declaration arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclId(u32);

impl DeclId {
    /// The raw index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index of a [`Region`] in the region arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(u32);

impl RegionId {
    /// The raw index.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One declaration: anything a name can denote.
#[derive(Clone, Debug)]
pub struct Decl {
    /// The designator, case-folded for basic identifiers. Operator symbols
    /// are stored with their quotes (`"+"`), character literals with their
    /// apostrophes (`'0'`).
    pub name: Symbol,
    /// The designator as written, for diagnostics.
    pub spelling: String,
    /// The kind and its details.
    pub kind: DeclKind,
    /// Where the designator is declared (the identifier's span, not the
    /// whole declaration), so diagnostics can point at it.
    pub span: Span,
    /// The region the declaration belongs to.
    pub region: RegionId,
}

/// The object class of an [`DeclKind::Object`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectClass {
    /// A constant, generic, or `constant` parameter.
    Constant,
    /// A signal, port, or `signal` parameter.
    Signal,
    /// A variable or `variable` parameter.
    Variable,
    /// A shared variable.
    SharedVariable,
    /// A file object or `file` parameter.
    File,
}

impl ObjectClass {
    /// The reserved word.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectClass::Constant => "constant",
            ObjectClass::Signal => "signal",
            ObjectClass::Variable => "variable",
            ObjectClass::SharedVariable => "shared variable",
            ObjectClass::File => "file",
        }
    }
}

/// Where an object was declared, which decides which rules apply to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectRole {
    /// A plain object declaration.
    Plain,
    /// A port of an entity, component or block.
    Port,
    /// A generic.
    Generic,
    /// A subprogram parameter.
    Parameter,
    /// A `for` loop parameter (a constant).
    LoopParam,
    /// A `for ... generate` parameter (a constant).
    GenerateParam,
    /// An object alias; reads and writes go through the aliased object.
    Alias,
    /// An external name (VHDL-2008 `<< signal ... >>`).
    External,
}

/// One formal parameter of a subprogram.
#[derive(Clone, Debug)]
pub struct Param {
    /// The parameter's declaration.
    pub decl: DeclId,
    /// The parameter's subtype.
    pub ty: TypeId,
    /// The mode (`in` when unspecified).
    pub mode: Mode,
    /// The object class (`constant` for `in`, `variable` otherwise, when
    /// unspecified).
    pub class: ObjectClass,
    /// True when a default expression was given.
    pub has_default: bool,
}

/// The profile of a subprogram: what overload resolution compares.
#[derive(Clone, Debug)]
pub struct Signature {
    /// Procedure or function.
    pub kind: ast::SubprogramKind,
    /// The formals in order.
    pub params: Vec<Param>,
    /// The return type, for functions.
    pub ret: Option<TypeId>,
    /// False for `impure` functions (procedures are always false).
    pub pure: bool,
}

/// How a subprogram is implemented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubprogramBody {
    /// No body seen (yet).
    None,
    /// A VHDL body at the given span.
    Vhdl(Span),
    /// Marked with `attribute foreign` (clause 20.2): the simulator supplies
    /// the implementation; the string is the attribute value.
    Foreign(String),
    /// Implicitly declared with a type (file operations, `deallocate`).
    Implicit,
}

/// The kinds of declaration.
#[derive(Clone, Debug)]
pub enum DeclKind {
    /// A design library.
    Library,
    /// A design unit. `region` holds the unit's declarations (an entity's
    /// generics, ports and declarative items; a package's declarations).
    Unit {
        /// The unit.
        unit: UnitId,
        /// The unit's declarative region.
        region: RegionId,
    },
    /// A type declaration.
    Type(TypeId),
    /// A subtype declaration.
    Subtype(TypeId),
    /// A constant, signal, variable or file object.
    Object {
        /// The class.
        class: ObjectClass,
        /// The subtype.
        ty: TypeId,
        /// The mode for ports and parameters.
        mode: Option<Mode>,
        /// Where it was declared.
        role: ObjectRole,
        /// True for a deferred constant (`constant c : t;` in a package)
        /// whose full declaration has not been seen.
        deferred: bool,
    },
    /// A subprogram declaration (or body without prior declaration).
    Subprogram {
        /// The profile.
        sig: Signature,
        /// The body, once seen.
        body: SubprogramBody,
    },
    /// An enumeration literal.
    EnumLiteral {
        /// The enumeration type.
        ty: TypeId,
        /// The literal's position.
        pos: u32,
    },
    /// A unit of a physical type.
    PhysicalUnit {
        /// The physical type.
        ty: TypeId,
        /// The unit's value in primary units.
        scale: i128,
    },
    /// A component declaration; the region holds its generics and ports.
    Component {
        /// The generics.
        generics: Vec<DeclId>,
        /// The ports.
        ports: Vec<DeclId>,
        /// The region holding both.
        region: RegionId,
    },
    /// A non-object alias (of a type, subprogram or literal).
    Alias(DeclId),
    /// A user-defined attribute declaration.
    Attribute(TypeId),
    /// A statement label.
    Label(LabelKind),
    /// A group template or group.
    Group,
    /// A record element, only used for the target of a `decl_of` on a
    /// selected name; the element's type is in the record type.
    RecordElement(TypeId),
    /// A declaration whose analysis failed; references to it are silent.
    Error,
}

/// What a label names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    /// A process.
    Process,
    /// A block.
    Block,
    /// A generate statement.
    Generate,
    /// An instantiation.
    Instance,
    /// A loop statement.
    Loop,
    /// Any other statement.
    Other,
}

/// What a declarative region belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// The root: library names and `std.standard`.
    Root,
    /// A design unit's context clause.
    Context,
    /// A package declaration.
    Package,
    /// A package body.
    PackageBody,
    /// An entity.
    Entity,
    /// An architecture.
    Architecture,
    /// A configuration declaration.
    Configuration,
    /// A component declaration's generics and ports.
    Component,
    /// A subprogram.
    Subprogram,
    /// A process.
    Process,
    /// A block statement.
    Block,
    /// A generate statement body.
    Generate,
    /// A protected type declaration or body.
    Protected,
    /// A loop statement (holding its parameter).
    Loop,
    /// A record type (its elements).
    Record,
}

/// A declarative region: the declarations directly in it, and the ones
/// made potentially visible by `use` clauses in it.
#[derive(Clone, Debug)]
pub struct Region {
    /// What the region belongs to.
    pub kind: RegionKind,
    /// The enclosing region, `None` for the root.
    pub parent: Option<RegionId>,
    /// The declarations in declaration order (implicit ones included).
    pub decls: Vec<DeclId>,
    /// Directly visible declarations by designator.
    names: HashMap<Symbol, Vec<DeclId>>,
    /// Potentially visible declarations (from `use` clauses) by designator.
    used: HashMap<Symbol, Vec<DeclId>>,
}

impl Region {
    fn new(kind: RegionKind, parent: Option<RegionId>) -> Self {
        Region {
            kind,
            parent,
            decls: Vec::new(),
            names: HashMap::new(),
            used: HashMap::new(),
        }
    }

    /// The directly visible declarations named `name` in this region.
    pub fn direct(&self, name: Symbol) -> &[DeclId] {
        self.names.get(&name).map_or(&[], Vec::as_slice)
    }

    /// The declarations made potentially visible in this region by `use`
    /// clauses.
    pub fn potentially_visible(&self, name: Symbol) -> &[DeclId] {
        self.used.get(&name).map_or(&[], Vec::as_slice)
    }

    /// Every directly declared designator, in a deterministic order.
    pub fn names(&self) -> Vec<Symbol> {
        let mut v: Vec<Symbol> = self.names.keys().copied().collect();
        v.sort();
        v
    }
}

/// What a call or operator expression resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallTarget {
    /// A declared subprogram (user-defined, from a package, or implicit).
    Subprogram(DeclId),
    /// A predefined operator of the operand type (clause 9.2), with the
    /// operator symbol.
    Operator(&'static str),
    /// A type conversion to the given type (clause 9.3.6).
    Conversion(TypeId),
    /// An index into an array object.
    Index,
    /// A predefined attribute with arguments (`t'image(x)`).
    Attribute(attrs::Predefined),
    /// A predefined function that is not an operator (`to_string`,
    /// `minimum`, `maximum`, `now`).
    Predefined(&'static str),
}

/// A resolved discrete range.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeInfo {
    /// The index (or loop parameter) type.
    pub ty: TypeId,
    /// The bounds.
    pub bounds: Bounds,
}

/// One analysed source file.
#[derive(Debug)]
pub struct AnalyzedFile {
    /// The source.
    pub source: SourceId,
    /// The library the file was compiled into.
    pub library: Symbol,
    /// The tree.
    pub ast: ast::DesignFile,
}

/// The handles to the predefined types of `std.standard` the checker needs
/// by identity (conditions are `boolean`, delays are `time`, ...).
#[derive(Clone, Copy, Debug)]
pub struct Builtins {
    /// `universal_integer`.
    pub universal_integer: TypeId,
    /// `universal_real`.
    pub universal_real: TypeId,
    /// `boolean`.
    pub boolean: TypeId,
    /// `bit`.
    pub bit: TypeId,
    /// `character`.
    pub character: TypeId,
    /// `severity_level`.
    pub severity_level: TypeId,
    /// `integer`.
    pub integer: TypeId,
    /// `real`.
    pub real: TypeId,
    /// `time`.
    pub time: TypeId,
    /// `natural`.
    pub natural: TypeId,
    /// `positive`.
    pub positive: TypeId,
    /// `string`.
    pub string: TypeId,
    /// `bit_vector`.
    pub bit_vector: TypeId,
    /// `boolean_vector` (VHDL-2008).
    pub boolean_vector: TypeId,
    /// `file_open_kind`.
    pub file_open_kind: TypeId,
    /// `file_open_status`.
    pub file_open_status: TypeId,
    /// The error placeholder type.
    pub error: TypeId,
}

/// The result of analysing a [`Design`].
#[derive(Debug)]
pub struct Analysis {
    /// The identifier interner; VHDL basic identifiers are folded.
    pub interner: Interner,
    /// The declaration arena.
    pub decls: Vec<Decl>,
    /// The type arena.
    pub types: Vec<Type>,
    /// The region arena; index 0 is the root.
    pub regions: Vec<Region>,
    /// The design units, in analysis order.
    pub units: Vec<Unit>,
    /// The analysed files.
    pub files: Vec<AnalyzedFile>,
    /// The predefined types.
    pub builtins: Builtins,
    refs: HashMap<Span, DeclId>,
    exprs: HashMap<Span, TypeId>,
    values: HashMap<Span, Value>,
    calls: HashMap<Span, CallTarget>,
    ranges: HashMap<Span, RangeInfo>,
    decl_values: HashMap<DeclId, Value>,
    implicit_conditions: std::collections::HashSet<Span>,
    attribute_values: HashMap<(DeclId, Symbol), Value>,
    libraries: Vec<(Symbol, DeclId)>,
}

impl Analysis {
    /// The declaration with the given id.
    pub fn decl(&self, id: DeclId) -> &Decl {
        &self.decls[id.index()]
    }

    /// The type with the given id.
    pub fn ty(&self, id: TypeId) -> &Type {
        &self.types[id.index()]
    }

    /// The region with the given id.
    pub fn region(&self, id: RegionId) -> &Region {
        &self.regions[id.index()]
    }

    /// The root region.
    pub fn root(&self) -> RegionId {
        RegionId(0)
    }

    /// The declaration a name at `span` denotes.
    pub fn decl_of(&self, span: Span) -> Option<DeclId> {
        self.refs.get(&span).copied()
    }

    /// The type of the expression, subtype indication or declared name at
    /// `span`.
    pub fn type_of(&self, span: Span) -> Option<TypeId> {
        self.exprs.get(&span).copied()
    }

    /// The locally static value of the expression at `span`.
    pub fn value_of(&self, span: Span) -> Option<&Value> {
        self.values.get(&span)
    }

    /// The static value of a constant declaration, if its initialiser was
    /// static.
    pub fn decl_value(&self, id: DeclId) -> Option<&Value> {
        self.decl_values.get(&id)
    }

    /// What the call or operator expression at `span` resolved to.
    pub fn call_of(&self, span: Span) -> Option<&CallTarget> {
        self.calls.get(&span)
    }

    /// The resolved bounds of the discrete range at `span`.
    pub fn range_of(&self, span: Span) -> Option<&RangeInfo> {
        self.ranges.get(&span)
    }

    /// True when the condition expression at `span` is not `boolean` and
    /// the VHDL-2008 implicit `??` conversion (clause 9.2.9) applies to it:
    /// the lowering pass must test it against `'1'`.
    pub fn implicit_condition(&self, span: Span) -> bool {
        self.implicit_conditions.contains(&span)
    }

    /// The static value an attribute specification gave `attribute` on
    /// `entity` (`attribute ram_style of mem : signal is "block"`).
    pub fn attribute_value(&self, entity: DeclId, attribute: Symbol) -> Option<&Value> {
        self.attribute_values.get(&(entity, attribute))
    }

    /// The design unit named `name` in library `library`, if analysed.
    pub fn unit(&self, library: &str, name: &str) -> Option<UnitId> {
        let lib = self.interner.get_ci(library)?;
        let name = self.interner.get_ci(name)?;
        self.units
            .iter()
            .position(|u| u.library == lib && u.name == name && u.kind.is_primary())
            .map(UnitId::from_index)
    }

    /// The libraries by name, with their declarations.
    pub fn libraries(&self) -> &[(Symbol, DeclId)] {
        &self.libraries
    }

    /// The text of a symbol.
    pub fn name(&self, sym: Symbol) -> &str {
        self.interner.resolve(sym)
    }

    // --- type queries ------------------------------------------------------

    /// Strips subtype layers.
    pub fn base_type(&self, mut id: TypeId) -> TypeId {
        loop {
            match &self.ty(id).kind {
                TypeKind::Subtype { parent, .. } => id = *parent,
                _ => return id,
            }
        }
    }

    /// The class of a type's base.
    pub fn class(&self, id: TypeId) -> TypeClass {
        self.ty(self.base_type(id)).kind.class()
    }

    /// True for the error placeholder.
    pub fn is_error(&self, id: TypeId) -> bool {
        matches!(self.ty(self.base_type(id)).kind, TypeKind::Error)
    }

    /// True when both denote the same base type.
    pub fn same_base(&self, a: TypeId, b: TypeId) -> bool {
        self.base_type(a) == self.base_type(b)
    }

    /// True for a scalar type.
    pub fn is_scalar(&self, id: TypeId) -> bool {
        self.class(id).is_scalar()
    }

    /// True for a discrete type.
    pub fn is_discrete(&self, id: TypeId) -> bool {
        self.class(id).is_discrete()
    }

    /// True when `id` is `boolean` (any subtype of it).
    pub fn is_boolean(&self, id: TypeId) -> bool {
        self.same_base(id, self.builtins.boolean)
    }

    /// The index subtypes and element subtype of an array type, following
    /// subtype layers.
    pub fn array_info(&self, id: TypeId) -> Option<(&[TypeId], TypeId)> {
        match &self.ty(self.base_type(id)).kind {
            TypeKind::Array { indices, element } => Some((indices, *element)),
            _ => None,
        }
    }

    /// The element subtype of an array subtype: the innermost VHDL-2008
    /// element constraint if the subtype has one, else the base type's
    /// element subtype.
    pub fn element_type(&self, id: TypeId) -> Option<TypeId> {
        let mut cur = id;
        loop {
            match &self.ty(cur).kind {
                TypeKind::Subtype {
                    parent, constraint, ..
                } => {
                    match constraint {
                        Some(Constraint::Index(_, Some(e))) | Some(Constraint::Element(e)) => {
                            return Some(*e);
                        }
                        _ => {}
                    }
                    cur = *parent;
                }
                TypeKind::Array { element, .. } => return Some(*element),
                _ => return None,
            }
        }
    }

    /// The subtype of record element `name` as seen through `id`: a
    /// VHDL-2008 record constraint of the subtype if it names the element,
    /// else the declared element subtype.
    pub fn field_type(&self, id: TypeId, name: Symbol) -> Option<TypeId> {
        let mut cur = id;
        loop {
            match &self.ty(cur).kind {
                TypeKind::Subtype {
                    parent, constraint, ..
                } => {
                    if let Some(Constraint::Record(fields)) = constraint
                        && let Some((_, t)) = fields.iter().find(|(n, _)| *n == name)
                    {
                        return Some(*t);
                    }
                    cur = *parent;
                }
                TypeKind::Record(fields) => {
                    return fields.iter().find(|f| f.name == name).map(|f| f.ty);
                }
                _ => return None,
            }
        }
    }

    /// The number of dimensions of an array type.
    pub fn dimensions(&self, id: TypeId) -> usize {
        self.array_info(id).map_or(0, |(i, _)| i.len())
    }

    /// The fields of a record type.
    pub fn record_fields(&self, id: TypeId) -> Option<&[Field]> {
        match &self.ty(self.base_type(id)).kind {
            TypeKind::Record(f) => Some(f),
            _ => None,
        }
    }

    /// The designated subtype of an access type.
    pub fn designated_type(&self, id: TypeId) -> Option<TypeId> {
        match &self.ty(self.base_type(id)).kind {
            TypeKind::Access(t) => Some(*t),
            _ => None,
        }
    }

    /// True when the type is a character type (an enumeration type with at
    /// least one character literal).
    pub fn is_character_type(&self, id: TypeId) -> bool {
        matches!(
            &self.ty(self.base_type(id)).kind,
            TypeKind::Enum {
                character: true,
                ..
            }
        )
    }

    /// True for a one-dimensional array whose element is a character type
    /// (the types a string literal can have, clause 9.3.2).
    pub fn is_string_type(&self, id: TypeId) -> bool {
        match self.array_info(id) {
            Some((idx, elem)) => idx.len() == 1 && self.is_character_type(elem),
            None => false,
        }
    }

    /// True for a one-dimensional array of `bit` or `boolean`, the types
    /// with predefined logical and shift operators (clause 9.2.2, 9.2.3).
    pub fn is_bit_or_boolean_array(&self, id: TypeId) -> bool {
        match self.array_info(id) {
            Some((idx, elem)) => {
                idx.len() == 1
                    && (self.same_base(elem, self.builtins.bit)
                        || self.same_base(elem, self.builtins.boolean))
            }
            None => false,
        }
    }

    /// True when the type or subtype is `bit` or `boolean`.
    pub fn is_bit_or_boolean(&self, id: TypeId) -> bool {
        self.same_base(id, self.builtins.bit) || self.same_base(id, self.builtins.boolean)
    }

    /// The scalar range of a (sub)type: the innermost range constraint, or
    /// the declared range of the base type.
    pub fn scalar_range(&self, id: TypeId) -> Option<Bounds> {
        let mut cur = id;
        loop {
            match &self.ty(cur).kind {
                TypeKind::Subtype {
                    parent, constraint, ..
                } => {
                    if let Some(Constraint::Range(b)) = constraint {
                        return Some(b.clone());
                    }
                    cur = *parent;
                }
                TypeKind::Integer(b) | TypeKind::Real(b) => return Some(b.clone()),
                TypeKind::Physical { range, .. } => return Some(range.clone()),
                TypeKind::Enum { literals, .. } => {
                    let n = i128::try_from(literals.len()).unwrap_or(0);
                    return Some(Bounds {
                        left: Bound::Static(Value::Enum(0)),
                        dir: Direction::To,
                        right: Bound::Static(Value::Enum(u32::try_from(n - 1).unwrap_or(0))),
                    });
                }
                _ => return None,
            }
        }
    }

    /// The index constraint of an array subtype, one entry per dimension,
    /// walking subtype layers until a constraint is found. `None` for an
    /// unconstrained array.
    pub fn index_constraint(&self, id: TypeId) -> Option<Vec<Bounds>> {
        let mut cur = id;
        loop {
            match &self.ty(cur).kind {
                TypeKind::Subtype {
                    parent, constraint, ..
                } => {
                    match constraint {
                        Some(Constraint::Index(b, _)) => return Some(b.clone()),
                        Some(Constraint::Range(b)) => return Some(vec![b.clone()]),
                        _ => {}
                    }
                    cur = *parent;
                }
                _ => return None,
            }
        }
    }

    /// True when an array subtype has an index constraint (fully
    /// constrained in every dimension); scalars and records count as
    /// constrained.
    pub fn is_constrained(&self, id: TypeId) -> bool {
        match self.class(id) {
            TypeClass::Array => self.index_constraint(id).is_some(),
            _ => true,
        }
    }

    /// The static length of a one-dimensional array subtype.
    pub fn array_length(&self, id: TypeId) -> Option<i128> {
        let c = self.index_constraint(id)?;
        if c.len() != 1 {
            return None;
        }
        c[0].length()
    }

    /// The resolution function of a subtype, walking subtype layers.
    pub fn resolution_function(&self, id: TypeId) -> Option<DeclId> {
        let mut cur = id;
        loop {
            match &self.ty(cur).kind {
                TypeKind::Subtype {
                    parent, resolution, ..
                } => {
                    if let Some(r) = resolution {
                        return Some(*r);
                    }
                    cur = *parent;
                }
                _ => return None,
            }
        }
    }

    /// The `std_ulogic` type, if `ieee.std_logic_1164` was analysed: the
    /// enumeration type with the nine literals `'U' 'X' '0' '1' 'Z' 'W'
    /// 'L' 'H' '-'`.
    pub fn std_ulogic(&self) -> Option<TypeId> {
        let pkg = self.unit("ieee", "std_logic_1164")?;
        let region = self.units[pkg.index()].region?;
        let name = self.interner.get_ci("std_ulogic")?;
        let id = *self.region(region).direct(name).first()?;
        match self.decl(id).kind {
            DeclKind::Type(t) => Some(t),
            _ => None,
        }
    }

    /// True when the type is `std_ulogic` or a subtype of it (such as
    /// `std_logic`).
    pub fn is_std_ulogic(&self, id: TypeId) -> bool {
        self.std_ulogic().is_some_and(|s| self.same_base(id, s))
    }

    /// True for a one-dimensional array of `std_ulogic`.
    pub fn is_std_ulogic_array(&self, id: TypeId) -> bool {
        match self.array_info(id) {
            Some((idx, elem)) => idx.len() == 1 && self.is_std_ulogic(elem),
            None => false,
        }
    }

    // --- rendering -----------------------------------------------------------

    /// A human-readable rendering of a type, in the form used by
    /// diagnostics: the declared name when there is one, with the
    /// constraint of anonymous subtypes spelled out
    /// (`std_logic_vector(7 downto 0)`, `integer range 0 to 15`).
    /// Dynamic bounds are shown as their source text when `map` is given.
    pub fn describe_type(&self, id: TypeId, map: Option<&SourceMap>) -> String {
        let mut out = String::new();
        self.write_type(&mut out, id, map, 0);
        out
    }

    fn write_type(&self, out: &mut String, id: TypeId, map: Option<&SourceMap>, depth: u32) {
        let t = self.ty(id);
        if let Some(n) = t.name {
            out.push_str(self.name(n));
            return;
        }
        match &t.kind {
            TypeKind::UniversalInteger => out.push_str("universal_integer"),
            TypeKind::UniversalReal => out.push_str("universal_real"),
            TypeKind::Error => out.push_str("<error>"),
            TypeKind::Subtype {
                parent,
                constraint,
                resolution,
            } => {
                if depth > 8 {
                    out.push_str("...");
                    return;
                }
                if let Some(r) = resolution {
                    out.push('(');
                    out.push_str(&self.decl(*r).spelling);
                    out.push_str(") ");
                }
                self.write_type(out, *parent, map, depth + 1);
                if let Some(c) = constraint {
                    self.write_constraint(out, c, map);
                }
            }
            TypeKind::Array { indices, element } => {
                out.push_str("array (");
                for (i, idx) in indices.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_type(out, *idx, map, depth + 1);
                    out.push_str(" range <>");
                }
                out.push_str(") of ");
                self.write_type(out, *element, map, depth + 1);
            }
            TypeKind::Record(_) => out.push_str("record"),
            TypeKind::Enum { .. } => out.push_str("enumeration"),
            TypeKind::Integer(_) => out.push_str("integer type"),
            TypeKind::Real(_) => out.push_str("floating type"),
            TypeKind::Physical { .. } => out.push_str("physical type"),
            TypeKind::Access(d) => {
                out.push_str("access ");
                self.write_type(out, *d, map, depth + 1);
            }
            TypeKind::File(d) => {
                out.push_str("file of ");
                self.write_type(out, *d, map, depth + 1);
            }
            TypeKind::Protected(_) => out.push_str("protected"),
            TypeKind::Incomplete => out.push_str("incomplete type"),
            TypeKind::Generic => out.push_str("generic type"),
        }
    }

    fn write_constraint(&self, out: &mut String, c: &Constraint, map: Option<&SourceMap>) {
        match c {
            Constraint::Range(b) => {
                out.push_str(" range ");
                self.write_bounds(out, b, map);
            }
            Constraint::Index(dims, elem) => {
                out.push('(');
                for (i, b) in dims.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_bounds(out, b, map);
                }
                out.push(')');
                if let Some(e) = elem {
                    self.write_element_constraint(out, *e, map);
                }
            }
            Constraint::Element(e) => {
                out.push_str("(open)");
                self.write_element_constraint(out, *e, map);
            }
            Constraint::Record(fields) => {
                out.push('(');
                for (i, (name, t)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(self.name(*name));
                    self.write_element_constraint(out, *t, map);
                }
                out.push(')');
            }
        }
    }

    /// Writes only the constraint part of an anonymous element subtype
    /// (`(7 downto 0)` of `t(open)(7 downto 0)`).
    fn write_element_constraint(&self, out: &mut String, t: TypeId, map: Option<&SourceMap>) {
        match &self.ty(t).kind {
            TypeKind::Subtype {
                constraint: Some(c),
                ..
            } if self.ty(t).name.is_none() => self.write_constraint(out, c, map),
            _ => {
                out.push(' ');
                self.write_type(out, t, map, 0);
            }
        }
    }

    fn write_bounds(&self, out: &mut String, b: &Bounds, map: Option<&SourceMap>) {
        self.write_bound(out, &b.left, map);
        out.push(' ');
        out.push_str(b.dir.as_str());
        out.push(' ');
        self.write_bound(out, &b.right, map);
    }

    fn write_bound(&self, out: &mut String, b: &Bound, map: Option<&SourceMap>) {
        match b {
            Bound::Static(v) => {
                let _ = write!(out, "{}", self.render_scalar(v));
            }
            Bound::Dynamic(span) => match map {
                Some(map) => {
                    let f = map.file(span.file);
                    out.push_str(&f.text()[span.start as usize..span.end as usize]);
                }
                None => out.push('?'),
            },
        }
    }

    /// Renders a scalar static value without a type (integers and reals
    /// as numbers, enumeration positions as `#n`).
    fn render_scalar(&self, v: &Value) -> String {
        match v {
            Value::Real(r) => format_real(*r),
            other => other.to_string(),
        }
    }

    /// Renders a static value the way VHDL would spell it for type `ty`:
    /// enumeration literals by name, strings in quotes, physical values
    /// with their primary unit, arrays and records as aggregates.
    pub fn describe_value(&self, v: &Value, ty: TypeId) -> String {
        let base = self.base_type(ty);
        match (&self.ty(base).kind, v) {
            (TypeKind::Enum { literals, .. }, Value::Enum(p)) => literals
                .get(usize::try_from(*p).unwrap_or(usize::MAX))
                .map_or_else(|| format!("#{p}"), |d| self.decl(*d).spelling.clone()),
            (TypeKind::Physical { units, .. }, Value::Int(i)) => {
                let unit = units
                    .first()
                    .map_or("", |u| self.decl(*u).spelling.as_str());
                format!("{i} {unit}")
            }
            (TypeKind::Real(_) | TypeKind::UniversalReal, Value::Real(r)) => format_real(*r),
            (TypeKind::Array { element, .. }, Value::Array(a)) => {
                if self.is_character_type(*element) {
                    let mut s = String::from("\"");
                    for e in &a.elems {
                        let lit = self.describe_value(e, *element);
                        // Character literals render as 'c'; strip the quotes.
                        let c = lit.trim_matches('\'');
                        if c == "\"" {
                            s.push_str("\"\"");
                        } else {
                            s.push_str(c);
                        }
                    }
                    s.push('"');
                    s
                } else {
                    let mut s = String::from("(");
                    for (i, e) in a.elems.iter().enumerate() {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        s.push_str(&self.describe_value(e, *element));
                    }
                    s.push(')');
                    s
                }
            }
            (TypeKind::Record(fields), Value::Record(vals)) => {
                let mut s = String::from("(");
                for (i, (f, e)) in fields.iter().zip(vals).enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    let _ = write!(
                        s,
                        "{} => {}",
                        self.name(f.name),
                        self.describe_value(e, f.ty)
                    );
                }
                s.push(')');
                s
            }
            (_, v) => self.render_scalar(v),
        }
    }

    /// A rendering of a subprogram's profile for diagnostics:
    /// `function "+"(l : unsigned; r : natural) return unsigned`.
    pub fn describe_subprogram(&self, id: DeclId) -> String {
        let d = self.decl(id);
        let DeclKind::Subprogram { sig, .. } = &d.kind else {
            return d.spelling.clone();
        };
        let mut s = String::new();
        s.push_str(match sig.kind {
            ast::SubprogramKind::Function => {
                if sig.pure {
                    "function "
                } else {
                    "impure function "
                }
            }
            ast::SubprogramKind::Procedure => "procedure ",
        });
        s.push_str(&d.spelling);
        if !sig.params.is_empty() {
            s.push('(');
            for (i, p) in sig.params.iter().enumerate() {
                if i > 0 {
                    s.push_str("; ");
                }
                let _ = write!(
                    s,
                    "{} : {}",
                    self.decl(p.decl).spelling,
                    self.describe_type(p.ty, None)
                );
            }
            s.push(')');
        }
        if let Some(r) = sig.ret {
            let _ = write!(s, " return {}", self.describe_type(r, None));
        }
        s
    }

    /// The declared type of an object, literal or unit declaration.
    pub fn decl_type(&self, id: DeclId) -> Option<TypeId> {
        match &self.decl(id).kind {
            DeclKind::Object { ty, .. }
            | DeclKind::EnumLiteral { ty, .. }
            | DeclKind::PhysicalUnit { ty, .. }
            | DeclKind::RecordElement(ty) => Some(*ty),
            DeclKind::Type(t) | DeclKind::Subtype(t) => Some(*t),
            DeclKind::Alias(target) => self.decl_type(*target),
            _ => None,
        }
    }

    // --- mutation (crate-private, used by the checker) ---------------------

    pub(crate) fn add_region(&mut self, kind: RegionKind, parent: Option<RegionId>) -> RegionId {
        let id = RegionId(u32::try_from(self.regions.len()).expect("region count"));
        self.regions.push(Region::new(kind, parent));
        id
    }

    pub(crate) fn add_type(&mut self, kind: TypeKind, name: Option<Symbol>) -> TypeId {
        let id = TypeId(u32::try_from(self.types.len()).expect("type count"));
        self.types.push(Type {
            kind,
            name,
            decl: None,
        });
        id
    }

    pub(crate) fn type_mut(&mut self, id: TypeId) -> &mut Type {
        &mut self.types[id.index()]
    }

    pub(crate) fn decl_mut(&mut self, id: DeclId) -> &mut Decl {
        &mut self.decls[id.index()]
    }

    /// Appends a declaration to a region and makes it directly visible
    /// there. The caller checks for duplicates first.
    pub(crate) fn add_decl(
        &mut self,
        region: RegionId,
        name: Symbol,
        spelling: impl Into<String>,
        kind: DeclKind,
        span: Span,
    ) -> DeclId {
        let id = DeclId(u32::try_from(self.decls.len()).expect("decl count"));
        self.decls.push(Decl {
            name,
            spelling: spelling.into(),
            kind,
            span,
            region,
        });
        let r = &mut self.regions[region.index()];
        r.decls.push(id);
        r.names.entry(name).or_default().push(id);
        id
    }

    /// Makes `decl` potentially visible in `region` under `name`.
    pub(crate) fn add_use(&mut self, region: RegionId, name: Symbol, decl: DeclId) {
        let r = &mut self.regions[region.index()];
        let v = r.used.entry(name).or_default();
        if !v.contains(&decl) {
            v.push(decl);
        }
    }

    pub(crate) fn set_ref(&mut self, span: Span, decl: DeclId) {
        self.refs.insert(span, decl);
    }

    pub(crate) fn set_type(&mut self, span: Span, ty: TypeId) {
        self.exprs.insert(span, ty);
    }

    pub(crate) fn set_value(&mut self, span: Span, v: Value) {
        self.values.insert(span, v);
    }

    pub(crate) fn set_decl_value(&mut self, id: DeclId, v: Value) {
        self.decl_values.insert(id, v);
    }

    pub(crate) fn set_call(&mut self, span: Span, target: CallTarget) {
        self.calls.insert(span, target);
    }

    pub(crate) fn set_range(&mut self, span: Span, info: RangeInfo) {
        self.ranges.insert(span, info);
    }

    pub(crate) fn set_implicit_condition(&mut self, span: Span) {
        self.implicit_conditions.insert(span);
    }

    pub(crate) fn set_attribute_value(&mut self, entity: DeclId, attribute: Symbol, v: Value) {
        self.attribute_values.insert((entity, attribute), v);
    }

    pub(crate) fn add_library(&mut self, name: Symbol, decl: DeclId) {
        self.libraries.push((name, decl));
    }

    pub(crate) fn new_empty(interner: Interner) -> Analysis {
        let mut a = Analysis {
            interner,
            decls: Vec::new(),
            types: Vec::new(),
            regions: Vec::new(),
            units: Vec::new(),
            files: Vec::new(),
            builtins: Builtins {
                universal_integer: TypeId(0),
                universal_real: TypeId(0),
                boolean: TypeId(0),
                bit: TypeId(0),
                character: TypeId(0),
                severity_level: TypeId(0),
                integer: TypeId(0),
                real: TypeId(0),
                time: TypeId(0),
                natural: TypeId(0),
                positive: TypeId(0),
                string: TypeId(0),
                bit_vector: TypeId(0),
                boolean_vector: TypeId(0),
                file_open_kind: TypeId(0),
                file_open_status: TypeId(0),
                error: TypeId(0),
            },
            refs: HashMap::new(),
            exprs: HashMap::new(),
            values: HashMap::new(),
            calls: HashMap::new(),
            ranges: HashMap::new(),
            decl_values: HashMap::new(),
            implicit_conditions: std::collections::HashSet::new(),
            attribute_values: HashMap::new(),
            libraries: Vec::new(),
        };
        a.add_region(RegionKind::Root, None);
        let error = a.add_type(TypeKind::Error, None);
        let ui = a.add_type(TypeKind::UniversalInteger, None);
        let ur = a.add_type(TypeKind::UniversalReal, None);
        a.builtins = Builtins {
            universal_integer: ui,
            universal_real: ur,
            boolean: error,
            bit: error,
            character: error,
            severity_level: error,
            integer: error,
            real: error,
            time: error,
            natural: error,
            positive: error,
            string: error,
            bit_vector: error,
            boolean_vector: error,
            file_open_kind: error,
            file_open_status: error,
            error,
        };
        a
    }
}

/// Renders a real the way VHDL prints one: always with a decimal point.
pub(crate) fn format_real(r: f64) -> String {
    if r.is_finite() && r == r.trunc() && r.abs() < 1e15 {
        format!("{r:.1}")
    } else {
        format!("{r}")
    }
}

/// Levenshtein distance, for "did you mean" suggestions.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Picks the closest candidate to `name` among `candidates`, if it is close
/// enough to be a plausible typo (distance at most a third of the length,
/// at least one).
pub(crate) fn suggest<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let lower = name.to_lowercase();
    let max = (lower.chars().count() / 3).max(1);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        if c.is_empty() || c.starts_with('"') {
            continue;
        }
        let d = edit_distance(&lower, &c.to_lowercase());
        if d == 0 || d > max {
            continue;
        }
        if best.is_none_or(|(bd, bc)| d < bd || (d == bd && c < bc)) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::source::SourceMap;
    use crate::vhdl::Standard;

    /// Analyses one snippet against the bundled libraries and returns the
    /// analysis together with the sources, so a test can look values and
    /// types up by the text of the expression that produced them.
    fn analyze(src: &str) -> (SourceMap, Diagnostics, Analysis) {
        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let mut design = Design::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
        let id = map.add("t.vhd", src.to_owned()).unwrap();
        design.add_source(&map, id, "work", &mut diags);
        let a = design.analyze(&map, &mut diags);
        diags.sort();
        (map, diags, a)
    }

    /// Wraps declarations in a package and analyses them.
    fn package(decls: &str) -> (SourceMap, Diagnostics, Analysis) {
        analyze(&format!(
            "library ieee;\nuse ieee.std_logic_1164.all;\npackage p is\n{decls}\nend package p;\n"
        ))
    }

    /// The static value of the constant named `name`, rendered with its
    /// type the way VHDL spells it.
    fn value_of(a: &Analysis, name: &str) -> Option<String> {
        let sym = a.interner.get_ci(name)?;
        let unit = a.unit("work", "p")?;
        let region = a.units[unit.index()].region?;
        let d = *a.region(region).direct(sym).first()?;
        let ty = a.decl_type(d)?;
        Some(a.describe_value(a.decl_value(d)?, ty))
    }

    /// The rendered type of the constant named `name`.
    fn type_of(a: &Analysis, name: &str) -> Option<String> {
        let sym = a.interner.get_ci(name)?;
        let unit = a.unit("work", "p")?;
        let region = a.units[unit.index()].region?;
        let d = *a.region(region).direct(sym).first()?;
        Some(a.describe_type(a.decl_type(d)?, None))
    }

    // --- the type model ---------------------------------------------------

    #[test]
    fn predefined_types_have_the_expected_shape() {
        let (map, diags, a) = package("  constant c : integer := 0;");
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let b = a.builtins;

        assert_eq!(a.class(b.integer), TypeClass::Integer);
        assert_eq!(a.class(b.real), TypeClass::Real);
        assert_eq!(a.class(b.time), TypeClass::Physical);
        assert_eq!(a.class(b.boolean), TypeClass::Enum);
        assert_eq!(a.class(b.string), TypeClass::Array);

        // `natural` and `positive` are subtypes of `integer`, so they
        // share its base type but keep their own ranges.
        assert!(a.same_base(b.natural, b.integer));
        assert!(a.same_base(b.positive, b.integer));
        assert_ne!(b.natural, b.integer);
        assert_eq!(a.scalar_range(b.natural).unwrap().low_high().unwrap().0, 0);
        assert_eq!(a.scalar_range(b.positive).unwrap().low_high().unwrap().0, 1);
        assert_eq!(
            a.scalar_range(b.integer).unwrap().ints().unwrap(),
            (-2147483648, 2147483647)
        );

        // `string` is a one-dimensional array of a character type.
        assert!(a.is_string_type(b.string));
        assert_eq!(a.dimensions(b.string), 1);
        assert!(a.is_character_type(a.element_type(b.string).unwrap()));
        // ... and is unconstrained, so it has no index constraint.
        assert!(!a.is_constrained(b.string));
        assert!(a.array_length(b.string).is_none());

        // `bit_vector` is an array of `bit`, which is not a character type
        // for these purposes but is a bit type.
        assert!(a.is_bit_or_boolean_array(b.bit_vector));
        assert!(a.is_bit_or_boolean(b.bit));
        assert!(!a.is_bit_or_boolean(b.integer));
    }

    #[test]
    fn subtypes_carry_constraints_and_resolution() {
        let (map, diags, a) = package(
            "  subtype byte_t is natural range 0 to 255;\n\
             \x20 subtype word_t is std_logic_vector(31 downto 0);\n\
             \x20 constant b : byte_t := 7;\n\
             \x20 constant w : word_t := (others => '0');",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));

        assert_eq!(type_of(&a, "b").as_deref(), Some("byte_t"));
        assert_eq!(type_of(&a, "w").as_deref(), Some("word_t"));

        let sym = a.interner.get_ci("byte_t").unwrap();
        let unit = a.unit("work", "p").unwrap();
        let region = a.units[unit.index()].region.unwrap();
        let d = *a.region(region).direct(sym).first().unwrap();
        let byte_t = a.decl_type(d).unwrap();
        assert!(a.same_base(byte_t, a.builtins.integer));
        assert_eq!(a.scalar_range(byte_t).unwrap().ints().unwrap(), (0, 255));

        let sym = a.interner.get_ci("word_t").unwrap();
        let d = *a.region(region).direct(sym).first().unwrap();
        let word_t = a.decl_type(d).unwrap();
        assert!(a.is_constrained(word_t));
        assert_eq!(a.array_length(word_t), Some(32));
        assert!(a.is_std_ulogic_array(word_t));
        // `std_logic_vector` resolves its elements, so the subtype has a
        // resolution function reachable through its layers.
        assert!(a.resolution_function(word_t).is_some());
    }

    #[test]
    fn array_and_record_types() {
        let (map, diags, a) = package(
            "  type pair_t is record\n\
             \x20   x : integer;\n\
             \x20   y : integer;\n\
             \x20 end record;\n\
             \x20 type grid_t is array (0 to 3, 0 to 1) of bit;\n\
             \x20 constant p : pair_t := (1, 2);",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));

        let unit = a.unit("work", "p").unwrap();
        let region = a.units[unit.index()].region.unwrap();
        let pick = |name: &str| {
            let sym = a.interner.get_ci(name).unwrap();
            let d = *a.region(region).direct(sym).first().unwrap();
            a.decl_type(d).unwrap()
        };

        let pair = pick("pair_t");
        assert_eq!(a.class(pair), TypeClass::Record);
        let fields = a.record_fields(pair).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(a.name(fields[0].name), "x");
        let x = a.interner.get_ci("x").unwrap();
        assert!(a.same_base(a.field_type(pair, x).unwrap(), a.builtins.integer));

        let grid = pick("grid_t");
        assert_eq!(a.dimensions(grid), 2);
        assert_eq!(a.index_constraint(grid).unwrap().len(), 2);
        assert_eq!(a.array_length(grid), None); // not one-dimensional

        assert_eq!(value_of(&a, "p").as_deref(), Some("(x => 1, y => 2)"));
    }

    // --- static evaluation -------------------------------------------------

    #[test]
    fn folds_static_scalar_expressions() {
        let (map, diags, a) = package(
            "  constant a : integer := 2 + 3 * 4;\n\
             \x20 constant b : integer := (2 + 3) * 4;\n\
             \x20 constant c : integer := 17 mod (-5);\n\
             \x20 constant d : integer := 17 rem (-5);\n\
             \x20 constant e : integer := 2 ** 10;\n\
             \x20 constant f : integer := abs (-7);\n\
             \x20 constant g : integer := 16#FF#;\n\
             \x20 constant h : real := 3.0 / 2.0;\n\
             \x20 constant i : boolean := 3 > 2;\n\
             \x20 constant j : time := 10 ns * 3;\n\
             \x20 constant k : string := \"ab\" & \"cd\";",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        assert_eq!(value_of(&a, "a").as_deref(), Some("14"));
        assert_eq!(value_of(&a, "b").as_deref(), Some("20"));
        // `mod` takes the sign of the right operand, `rem` of the left.
        assert_eq!(value_of(&a, "c").as_deref(), Some("-3"));
        assert_eq!(value_of(&a, "d").as_deref(), Some("2"));
        assert_eq!(value_of(&a, "e").as_deref(), Some("1024"));
        assert_eq!(value_of(&a, "f").as_deref(), Some("7"));
        assert_eq!(value_of(&a, "g").as_deref(), Some("255"));
        assert_eq!(value_of(&a, "h").as_deref(), Some("1.5"));
        assert_eq!(value_of(&a, "i").as_deref(), Some("true"));
        // A physical value folds to its primary unit: 30 ns is 30000000 fs.
        assert_eq!(value_of(&a, "j").as_deref(), Some("30000000 fs"));
        assert_eq!(value_of(&a, "k").as_deref(), Some("\"abcd\""));
    }

    #[test]
    fn folds_static_vectors_and_aggregates() {
        let (map, diags, a) = package(
            "  constant a : bit_vector(3 downto 0) := \"1010\";\n\
             \x20 constant b : bit_vector(3 downto 0) := a and \"1100\";\n\
             \x20 constant c : bit_vector(3 downto 0) := not a;\n\
             \x20 constant d : bit_vector(7 downto 0) := a & \"0101\";\n\
             \x20 constant e : bit := a(3);\n\
             \x20 constant f : bit_vector(3 downto 0) := (others => '1');\n\
             \x20 constant g : bit_vector(3 downto 0) := (3 => '1', others => '0');\n\
             \x20 constant h : std_logic_vector(3 downto 0) := x\"A\";\n\
             \x20 constant i : natural := a'length;",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        assert_eq!(value_of(&a, "a").as_deref(), Some("\"1010\""));
        assert_eq!(value_of(&a, "b").as_deref(), Some("\"1000\""));
        assert_eq!(value_of(&a, "c").as_deref(), Some("\"0101\""));
        assert_eq!(value_of(&a, "d").as_deref(), Some("\"10100101\""));
        assert_eq!(value_of(&a, "e").as_deref(), Some("'1'"));
        assert_eq!(value_of(&a, "f").as_deref(), Some("\"1111\""));
        assert_eq!(value_of(&a, "g").as_deref(), Some("\"1000\""));
        assert_eq!(value_of(&a, "h").as_deref(), Some("\"1010\""));
        assert_eq!(value_of(&a, "i").as_deref(), Some("4"));
    }

    #[test]
    fn folds_attributes() {
        let (map, diags, a) = package(
            "  type colour_t is (red, green, blue);\n\
             \x20 type vec_t is array (7 downto 0) of bit;\n\
             \x20 constant a : natural  := colour_t'pos(green);\n\
             \x20 constant b : colour_t := colour_t'val(2);\n\
             \x20 constant c : colour_t := colour_t'succ(red);\n\
             \x20 constant d : colour_t := colour_t'pred(blue);\n\
             \x20 constant e : string   := colour_t'image(blue);\n\
             \x20 constant f : colour_t := colour_t'value(\"red\");\n\
             \x20 constant g : natural  := vec_t'length;\n\
             \x20 constant h : integer  := vec_t'left;\n\
             \x20 constant i : integer  := vec_t'low;\n\
             \x20 constant j : boolean  := vec_t'ascending;\n\
             \x20 constant k : string   := integer'image(42);",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        assert_eq!(value_of(&a, "a").as_deref(), Some("1"));
        assert_eq!(value_of(&a, "b").as_deref(), Some("blue"));
        assert_eq!(value_of(&a, "c").as_deref(), Some("green"));
        assert_eq!(value_of(&a, "d").as_deref(), Some("green"));
        assert_eq!(value_of(&a, "e").as_deref(), Some("\"blue\""));
        assert_eq!(value_of(&a, "f").as_deref(), Some("red"));
        assert_eq!(value_of(&a, "g").as_deref(), Some("8"));
        assert_eq!(value_of(&a, "h").as_deref(), Some("7"));
        assert_eq!(value_of(&a, "i").as_deref(), Some("0"));
        assert_eq!(value_of(&a, "j").as_deref(), Some("false"));
        assert_eq!(value_of(&a, "k").as_deref(), Some("\"42\""));
    }

    #[test]
    fn range_violations_are_reported_statically() {
        let (map, diags, _) = package("  constant a : natural := -1;");
        let r = diags.render(&map);
        assert!(r.contains("out of range"), "{r}");
        assert!(r.contains("natural"), "{r}");

        let (map, diags, _) = package("  constant a : bit_vector(3 downto 0) := \"10101\";");
        let r = diags.render(&map);
        assert!(r.contains("5 elements"), "{r}");
    }

    // --- overload resolution -----------------------------------------------

    #[test]
    fn resolves_overloaded_operators_by_context() {
        let (map, diags, a) = package(
            "  constant i : integer := 1 + 1;\n\
             \x20 constant r : real := 1.0 + 1.0;\n\
             \x20 constant t : time := 1 ns + 1 ns;\n\
             \x20 constant s : std_logic := '0' and '1';\n\
             \x20 constant v : std_logic_vector(1 downto 0) := \"01\" or \"10\";",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        assert_eq!(value_of(&a, "i").as_deref(), Some("2"));
        assert_eq!(value_of(&a, "r").as_deref(), Some("2.0"));
        assert_eq!(value_of(&a, "t").as_deref(), Some("2000000 fs"));
        // `and` on `std_ulogic` is a function from the 1164 package, not a
        // predefined operator, and it folds through the builtin tables.
        assert_eq!(value_of(&a, "s").as_deref(), Some("'0'"));
        assert_eq!(value_of(&a, "v").as_deref(), Some("\"11\""));
    }

    #[test]
    fn resolves_overloaded_subprograms_by_argument_and_result() {
        let src = "\
package q is
  function f (a : integer) return integer;
  function f (a : real) return real;
  function g (a : integer) return integer;
  function g (a : integer) return boolean;
end package q;

package body q is
  function f (a : integer) return integer is
  begin
    return a;
  end function;
  function f (a : real) return real is
  begin
    return a;
  end function;
  function g (a : integer) return integer is
  begin
    return a;
  end function;
  function g (a : integer) return boolean is
  begin
    return a > 0;
  end function;
end package body q;

use work.q.all;
package p is
  constant a : integer := f(1);
  constant b : real    := f(1.0);
  constant c : integer := g(1);
  constant d : boolean := g(1);
end package p;
";
        let (map, diags, a) = analyze(src);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        // Each call resolved: the checker recorded a target for it.
        assert_eq!(type_of(&a, "a").as_deref(), Some("integer"));
        assert_eq!(type_of(&a, "b").as_deref(), Some("real"));
        assert_eq!(type_of(&a, "c").as_deref(), Some("integer"));
        assert_eq!(type_of(&a, "d").as_deref(), Some("boolean"));
    }

    #[test]
    fn reports_ambiguity_and_no_match() {
        // Two interpretations that both fit the context.
        let src = "\
package q is
  type t1 is (red, blue);
  type t2 is (red, blue);
end package q;
use work.q.all;
package p is
  constant a : boolean := red = blue;
end package p;
";
        let (map, diags, _) = analyze(src);
        let r = diags.render(&map);
        assert!(r.contains("ambiguous"), "{r}");

        // No operator at all for these operand types.
        let (map, diags, _) = package("  constant a : integer := 1 + true;");
        let r = diags.render(&map);
        assert!(r.contains("no visible operator"), "{r}");
    }

    #[test]
    fn an_explicit_declaration_hides_the_predefined_operator() {
        let src = "\
package q is
  function \"+\" (a, b : bit_vector) return bit_vector;
end package q;
package body q is
  function \"+\" (a, b : bit_vector) return bit_vector is
  begin
    return a xor b;
  end function;
end package body q;

use work.q.all;
package p is
  constant a : bit_vector(3 downto 0) := \"0011\" + \"0101\";
end package p;
";
        let (map, diags, a) = analyze(src);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        // The constant's subtype is the anonymous constrained one, which
        // `describe_type` spells out.
        assert_eq!(type_of(&a, "a").as_deref(), Some("bit_vector(3 downto 0)"));
    }

    #[test]
    fn universal_literals_convert_to_the_context_type() {
        let (map, diags, a) = package(
            "  type small_t is range 0 to 15;\n\
             \x20 constant a : small_t := 7;\n\
             \x20 constant b : small_t := a + 1;\n\
             \x20 constant c : real := 1.0 * 2.0;",
        );
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        assert_eq!(type_of(&a, "b").as_deref(), Some("small_t"));
        assert_eq!(value_of(&a, "b").as_deref(), Some("8"));
        assert_eq!(value_of(&a, "c").as_deref(), Some("2.0"));
    }

    // --- visibility ---------------------------------------------------------

    #[test]
    fn a_missing_use_clause_is_suggested() {
        let (map, diags, _) =
            analyze("package p is\n  constant a : std_logic := '0';\nend package p;\n");
        let r = diags.render(&map);
        assert!(r.contains("cannot find `std_logic`"), "{r}");
        assert!(r.contains("use ieee.std_logic_1164.all"), "{r}");
    }

    #[test]
    fn an_unbundled_package_is_named_in_the_diagnostic() {
        let (map, diags, _) =
            analyze("library ieee;\nuse ieee.numeric_std.all;\npackage p is\nend package p;\n");
        let r = diags.render(&map);
        assert!(r.contains("numeric_std"), "{r}");
        assert!(r.contains("not bundled"), "{r}");
        assert_eq!(diags.error_count(), 1, "{r}");
    }

    #[test]
    fn edit_distance_and_suggestions() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(
            suggest(
                "std_logic_vectr",
                ["std_logic_vector", "std_logic"].into_iter()
            ),
            Some("std_logic_vector")
        );
        assert_eq!(suggest("clk", ["reset", "data"].into_iter()), None);
        assert_eq!(suggest("clk", ["clk"].into_iter()), None);
        assert_eq!(suggest("clkk", ["clk", "clkx"].into_iter()), Some("clk"));
    }

    #[test]
    fn format_reals() {
        assert_eq!(format_real(1.0), "1.0");
        assert_eq!(format_real(2.5), "2.5");
        assert_eq!(format_real(1e20), "100000000000000000000");
    }

    #[test]
    fn empty_analysis_has_root_and_universals() {
        let a = Analysis::new_empty(Interner::new());
        assert_eq!(a.regions.len(), 1);
        assert_eq!(
            a.class(a.builtins.universal_integer),
            TypeClass::UniversalInteger
        );
        assert_eq!(a.class(a.builtins.universal_real), TypeClass::UniversalReal);
        assert!(a.is_error(a.builtins.error));
        assert_eq!(
            a.describe_type(a.builtins.universal_integer, None),
            "universal_integer"
        );
    }
}
