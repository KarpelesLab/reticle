//! The VHDL abstract syntax tree.
//!
//! The tree is a faithful, unresolved image of the source: nothing is
//! looked up, no case is folded and no expression is typed. It is what the
//! parser produces and what the semantic pass consumes.
//!
//! # Conventions
//!
//! - Every node carries a [`Span`]; enums expose it through a `span()`
//!   method. Spans cover the whole construct, from its first token to its
//!   terminating `;` when there is one.
//! - Identifiers keep their original spelling in an [`Ident`]. Basic
//!   identifiers are case-insensitive and compared with [`Ident::same_as`];
//!   extended identifiers (`\Foo\`) are case-sensitive. Nothing is interned
//!   at this stage so the tree does not depend on the interner.
//! - Literals stay raw text (`16#FF#`, `10 ns`, `x"F_F"`), because their
//!   interpretation depends on types the parser does not know.
//! - The grammar is ambiguous at parse time in a few well-known places, and
//!   the tree keeps the ambiguity rather than guessing:
//!   - `f(x)` is a [`Name::Call`] whether it turns out to be a function call,
//!     an array index, a type conversion or an attribute argument. Only a
//!     slice with an explicit range (`a(1 to 3)`, `a(natural range 0 to 3)`)
//!     is a [`Name::Slice`].
//!   - `x'range` is a [`Name::Attribute`] and may denote a range wherever a
//!     range is expected ([`Range::Attribute`]).
//!   - A simple name used where a discrete range is expected (`for i in t`)
//!     becomes a [`DiscreteRange::Subtype`].
//!   - `(e)` is [`Expr::Paren`]; anything with more than one element, a
//!     choice or `others` is an [`Aggregate`].
//!   - `label : name;` is a component instantiation, `name(args);` a
//!     concurrent procedure call.
//! - Boxes and `Vec`s rather than an arena, so the tree can be built and
//!   inspected without any context object.

use crate::source::Span;

/// An identifier as written in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    /// The identifier text: original spelling for basic identifiers, the
    /// content between the backslashes (unescaped) for extended ones.
    pub name: String,
    /// Where it was written.
    pub span: Span,
    /// True for an extended identifier `\name\`.
    pub extended: bool,
}

impl Ident {
    /// True when both denote the same name: case-insensitively for basic
    /// identifiers, exactly for extended ones. A basic and an extended
    /// identifier are never the same.
    pub fn same_as(&self, other: &Ident) -> bool {
        if self.extended != other.extended {
            return false;
        }
        if self.extended {
            self.name == other.name
        } else {
            self.name.len() == other.name.len()
                && self
                    .name
                    .chars()
                    .zip(other.name.chars())
                    .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
        }
    }
}

/// A subprogram, alias, attribute or enumeration designator: an identifier,
/// a character literal or an operator symbol such as `"+"`.
#[derive(Debug, Clone, PartialEq)]
pub enum Designator {
    /// A plain identifier.
    Ident(Ident),
    /// A character literal such as `'0'`.
    Char {
        /// The character.
        ch: char,
        /// Where it was written, quotes included.
        span: Span,
    },
    /// An operator symbol such as `"and"` or `"+"`.
    Operator {
        /// The operator text without quotes.
        symbol: String,
        /// Where it was written, quotes included.
        span: Span,
    },
}

impl Designator {
    /// The span of the designator.
    pub fn span(&self) -> Span {
        match self {
            Designator::Ident(i) => i.span,
            Designator::Char { span, .. } | Designator::Operator { span, .. } => *span,
        }
    }
}

// --- design units ---------------------------------------------------------

/// One source file: a sequence of design units.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DesignFile {
    /// The units in source order.
    pub units: Vec<DesignUnit>,
}

/// A library unit together with its context clause.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignUnit {
    /// `library`, `use` and `context` clauses preceding the unit.
    pub context: Vec<ContextItem>,
    /// The unit itself.
    pub unit: LibraryUnit,
    /// From the first context clause to the end of the unit.
    pub span: Span,
}

/// One item of a context clause or a context declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextItem {
    /// `library a, b;`
    Library(LibraryClause),
    /// `use a.b.all;`
    Use(UseClause),
    /// `context lib.ctx;` (VHDL-2008).
    Context(ContextReference),
}

impl ContextItem {
    /// The span of the item.
    pub fn span(&self) -> Span {
        match self {
            ContextItem::Library(c) => c.span,
            ContextItem::Use(c) => c.span,
            ContextItem::Context(c) => c.span,
        }
    }
}

/// `library name {, name};`
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryClause {
    /// The logical library names.
    pub names: Vec<Ident>,
    /// The whole clause.
    pub span: Span,
}

/// `use selected_name {, selected_name};`
#[derive(Debug, Clone, PartialEq)]
pub struct UseClause {
    /// The selected names, each ending in a designator or `all`.
    pub names: Vec<Name>,
    /// The whole clause.
    pub span: Span,
}

/// `context selected_name {, selected_name};` referencing context
/// declarations (VHDL-2008).
#[derive(Debug, Clone, PartialEq)]
pub struct ContextReference {
    /// The context declarations referred to.
    pub names: Vec<Name>,
    /// The whole clause.
    pub span: Span,
}

/// A primary or secondary unit.
#[derive(Debug, Clone, PartialEq)]
pub enum LibraryUnit {
    /// An entity declaration.
    Entity(EntityDecl),
    /// An architecture body.
    Architecture(ArchitectureBody),
    /// A package declaration.
    Package(PackageDecl),
    /// A package body.
    PackageBody(PackageBody),
    /// A package instantiation declaration (VHDL-2008).
    PackageInstantiation(PackageInstantiation),
    /// A configuration declaration.
    Configuration(ConfigurationDecl),
    /// A context declaration (VHDL-2008).
    Context(ContextDecl),
}

impl LibraryUnit {
    /// The span of the unit.
    pub fn span(&self) -> Span {
        match self {
            LibraryUnit::Entity(u) => u.span,
            LibraryUnit::Architecture(u) => u.span,
            LibraryUnit::Package(u) => u.span,
            LibraryUnit::PackageBody(u) => u.span,
            LibraryUnit::PackageInstantiation(u) => u.span,
            LibraryUnit::Configuration(u) => u.span,
            LibraryUnit::Context(u) => u.span,
        }
    }

    /// The name the unit declares.
    pub fn name(&self) -> &Ident {
        match self {
            LibraryUnit::Entity(u) => &u.name,
            LibraryUnit::Architecture(u) => &u.name,
            LibraryUnit::Package(u) => &u.name,
            LibraryUnit::PackageBody(u) => &u.name,
            LibraryUnit::PackageInstantiation(u) => &u.name,
            LibraryUnit::Configuration(u) => &u.name,
            LibraryUnit::Context(u) => &u.name,
        }
    }
}

/// `entity name is [generic (...);] [port (...);] decls [begin stmts] end;`
#[derive(Debug, Clone, PartialEq)]
pub struct EntityDecl {
    /// The entity name.
    pub name: Ident,
    /// The generic clause, empty when absent.
    pub generics: Vec<InterfaceDecl>,
    /// The port clause, empty when absent.
    pub ports: Vec<InterfaceDecl>,
    /// The entity declarative part.
    pub decls: Vec<Declaration>,
    /// The (passive) entity statement part.
    pub statements: Vec<ConcurrentStatement>,
    /// The whole unit.
    pub span: Span,
}

/// `architecture name of entity is decls begin stmts end;`
#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureBody {
    /// The architecture name.
    pub name: Ident,
    /// The entity it implements.
    pub entity: Name,
    /// The architecture declarative part.
    pub decls: Vec<Declaration>,
    /// The architecture statement part.
    pub statements: Vec<ConcurrentStatement>,
    /// The whole unit.
    pub span: Span,
}

/// `package name is [generic (...); [generic map (...);]] decls end;`
#[derive(Debug, Clone, PartialEq)]
pub struct PackageDecl {
    /// The package name.
    pub name: Ident,
    /// The generic clause (VHDL-2008), empty when absent.
    pub generics: Vec<InterfaceDecl>,
    /// The generic map aspect following the generic clause (VHDL-2008).
    pub generic_map: Option<Vec<AssociationElement>>,
    /// The package declarative part.
    pub decls: Vec<Declaration>,
    /// The whole unit.
    pub span: Span,
}

/// `package body name is decls end;`
#[derive(Debug, Clone, PartialEq)]
pub struct PackageBody {
    /// The package name.
    pub name: Ident,
    /// The package body declarative part.
    pub decls: Vec<Declaration>,
    /// The whole unit.
    pub span: Span,
}

/// `package name is new uninstantiated_name [generic map (...)];`
#[derive(Debug, Clone, PartialEq)]
pub struct PackageInstantiation {
    /// The new package's name.
    pub name: Ident,
    /// The uninstantiated package.
    pub uninstantiated: Name,
    /// The generic map aspect, empty when absent.
    pub generic_map: Vec<AssociationElement>,
    /// The whole declaration.
    pub span: Span,
}

/// `context name is context_items end;`
#[derive(Debug, Clone, PartialEq)]
pub struct ContextDecl {
    /// The context name.
    pub name: Ident,
    /// The clauses it bundles.
    pub items: Vec<ContextItem>,
    /// The whole unit.
    pub span: Span,
}

/// `configuration name of entity is decls block_configuration end;`
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationDecl {
    /// The configuration name.
    pub name: Ident,
    /// The configured entity.
    pub entity: Name,
    /// Use clauses, attribute specifications and group declarations.
    pub decls: Vec<Declaration>,
    /// The block configuration for the architecture.
    pub block: BlockConfiguration,
    /// The whole unit.
    pub span: Span,
}

/// `for block_spec {use_clause} {configuration_item} end for;`
#[derive(Debug, Clone, PartialEq)]
pub struct BlockConfiguration {
    /// The architecture name, block label or generate label (with an
    /// optional generate specification, kept as an indexed or sliced name).
    pub spec: Name,
    /// Use clauses at the start of the block configuration.
    pub uses: Vec<UseClause>,
    /// Nested block and component configurations.
    pub items: Vec<ConfigurationItem>,
    /// The whole configuration.
    pub span: Span,
}

/// An item inside a block configuration.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigurationItem {
    /// A nested block configuration.
    Block(BlockConfiguration),
    /// A component configuration.
    Component(Box<ComponentConfiguration>),
}

impl ConfigurationItem {
    /// The span of the item.
    pub fn span(&self) -> Span {
        match self {
            ConfigurationItem::Block(b) => b.span,
            ConfigurationItem::Component(c) => c.span,
        }
    }
}

/// `for instances : component [binding_indication;] [block_configuration]
/// end for;`
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentConfiguration {
    /// Which instances of which component.
    pub spec: ComponentSpecification,
    /// The binding indication, if any.
    pub binding: Option<BindingIndication>,
    /// The block configuration for the bound architecture, if any.
    pub block: Option<BlockConfiguration>,
    /// The whole configuration.
    pub span: Span,
}

/// `instantiation_list : component_name`
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentSpecification {
    /// The instances.
    pub instances: InstantiationList,
    /// The component.
    pub component: Name,
    /// The whole specification.
    pub span: Span,
}

/// Which instances a component specification applies to.
#[derive(Debug, Clone, PartialEq)]
pub enum InstantiationList {
    /// Explicit instance labels.
    Labels(Vec<Ident>),
    /// `others`
    Others(Span),
    /// `all`
    All(Span),
}

/// `[use entity_aspect] [generic map (...)] [port map (...)]`
#[derive(Debug, Clone, PartialEq)]
pub struct BindingIndication {
    /// The entity aspect after `use`, if any.
    pub entity_aspect: Option<EntityAspect>,
    /// The generic map aspect, if any.
    pub generic_map: Option<Vec<AssociationElement>>,
    /// The port map aspect, if any.
    pub port_map: Option<Vec<AssociationElement>>,
    /// The whole indication.
    pub span: Span,
}

/// What a binding indication binds to.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityAspect {
    /// `entity name [(architecture)]`
    Entity {
        /// The entity.
        name: Name,
        /// The architecture, if given.
        architecture: Option<Ident>,
        /// The whole aspect.
        span: Span,
    },
    /// `configuration name`
    Configuration(Name),
    /// `open`
    Open(Span),
}

impl EntityAspect {
    /// The span of the aspect.
    pub fn span(&self) -> Span {
        match self {
            EntityAspect::Entity { span, .. } | EntityAspect::Open(span) => *span,
            EntityAspect::Configuration(n) => n.span(),
        }
    }
}

// --- interfaces and associations --------------------------------------------

/// An element of a generic, port or parameter list.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfaceDecl {
    /// A constant, signal, variable or file interface object.
    Object(Box<InterfaceObject>),
    /// `type name` (VHDL-2008 generic type).
    Type(InterfaceType),
    /// A generic subprogram (VHDL-2008).
    Subprogram(InterfaceSubprogram),
    /// A generic package (VHDL-2008).
    Package(InterfacePackage),
}

impl InterfaceDecl {
    /// The span of the element.
    pub fn span(&self) -> Span {
        match self {
            InterfaceDecl::Object(o) => o.span,
            InterfaceDecl::Type(t) => t.span,
            InterfaceDecl::Subprogram(s) => s.span,
            InterfaceDecl::Package(p) => p.span,
        }
    }
}

/// `[class] names : [mode] subtype [bus] [:= default]`
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceObject {
    /// The explicit object class, if written.
    pub class: Option<ObjectClass>,
    /// The declared names.
    pub names: Vec<Ident>,
    /// The mode, if written (defaults to `in`).
    pub mode: Option<Mode>,
    /// The subtype indication.
    pub subtype: SubtypeIndication,
    /// True when `bus` follows the subtype.
    pub bus: bool,
    /// The default expression.
    pub default: Option<Expr>,
    /// The whole element.
    pub span: Span,
}

/// The class of an interface object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectClass {
    /// `constant`
    Constant,
    /// `signal`
    Signal,
    /// `variable`
    Variable,
    /// `file`
    File,
}

/// The mode of an interface object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `in`
    In,
    /// `out`
    Out,
    /// `inout`
    Inout,
    /// `buffer`
    Buffer,
    /// `linkage`
    Linkage,
}

/// `type name` in a generic list.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceType {
    /// The generic type name.
    pub name: Ident,
    /// The whole element.
    pub span: Span,
}

/// `function|procedure ... [is <> | is name]` in a generic list.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceSubprogram {
    /// The subprogram specification.
    pub spec: SubprogramSpec,
    /// The default binding, if any.
    pub default: Option<SubprogramDefault>,
    /// The whole element.
    pub span: Span,
}

/// The default of a generic subprogram.
#[derive(Debug, Clone, PartialEq)]
pub enum SubprogramDefault {
    /// `is <>`: bind to the visible subprogram of the same name.
    Box(Span),
    /// `is name`: bind to this subprogram.
    Name(Name),
}

/// `package name is new uninstantiated generic map (...)` in a generic list.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfacePackage {
    /// The generic package name.
    pub name: Ident,
    /// The uninstantiated package.
    pub uninstantiated: Name,
    /// The generic map aspect.
    pub generic_map: InterfacePackageMap,
    /// The whole element.
    pub span: Span,
}

/// The generic map aspect of a generic package.
#[derive(Debug, Clone, PartialEq)]
pub enum InterfacePackageMap {
    /// `generic map (<>)`
    Box(Span),
    /// `generic map (default)`
    Default(Span),
    /// An explicit association list.
    Map(Vec<AssociationElement>),
}

/// One element of an association list: `[formal =>] actual`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationElement {
    /// The formal part, when the association is named. It is a name, or a
    /// conversion function or type applied to a name, kept as an expression.
    pub formal: Option<Expr>,
    /// The actual part.
    pub actual: Actual,
    /// The whole element.
    pub span: Span,
}

/// The actual part of an association.
#[derive(Debug, Clone, PartialEq)]
pub enum Actual {
    /// An expression (or a name, subtype or conversion of a name).
    Expr(Expr),
    /// `open`
    Open(Span),
    /// `inertial expression` (VHDL-2008).
    Inertial(Expr),
    /// A discrete range or a constrained subtype, as found in slices and in
    /// generic type associations.
    Range(DiscreteRange),
}

impl Actual {
    /// The span of the actual.
    pub fn span(&self) -> Span {
        match self {
            Actual::Expr(e) | Actual::Inertial(e) => e.span(),
            Actual::Open(span) => *span,
            Actual::Range(r) => r.span(),
        }
    }
}

// --- declarations ---------------------------------------------------------

/// An item of a declarative part.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    /// A constant, signal or (shared) variable declaration.
    Object(ObjectDecl),
    /// A file declaration.
    File(FileDecl),
    /// A type declaration.
    Type(TypeDecl),
    /// A subtype declaration.
    Subtype(SubtypeDecl),
    /// An alias declaration.
    Alias(AliasDecl),
    /// An attribute declaration.
    Attribute(AttributeDecl),
    /// An attribute specification.
    AttributeSpec(AttributeSpec),
    /// A component declaration.
    Component(ComponentDecl),
    /// A subprogram declaration.
    Subprogram(SubprogramDecl),
    /// A subprogram body.
    SubprogramBody(SubprogramBody),
    /// A subprogram instantiation (VHDL-2008).
    SubprogramInstantiation(SubprogramInstantiation),
    /// A nested package declaration (VHDL-2008).
    Package(PackageDecl),
    /// A nested package body (VHDL-2008).
    PackageBody(PackageBody),
    /// A package instantiation (VHDL-2008).
    PackageInstantiation(PackageInstantiation),
    /// A use clause.
    Use(UseClause),
    /// A group template declaration.
    GroupTemplate(GroupTemplateDecl),
    /// A group declaration.
    Group(GroupDecl),
    /// A disconnection specification.
    Disconnection(DisconnectionSpec),
    /// A configuration specification.
    ConfigurationSpec(ConfigurationSpec),
}

impl Declaration {
    /// The span of the declaration.
    pub fn span(&self) -> Span {
        match self {
            Declaration::Object(d) => d.span,
            Declaration::File(d) => d.span,
            Declaration::Type(d) => d.span,
            Declaration::Subtype(d) => d.span,
            Declaration::Alias(d) => d.span,
            Declaration::Attribute(d) => d.span,
            Declaration::AttributeSpec(d) => d.span,
            Declaration::Component(d) => d.span,
            Declaration::Subprogram(d) => d.span,
            Declaration::SubprogramBody(d) => d.span,
            Declaration::SubprogramInstantiation(d) => d.span,
            Declaration::Package(d) => d.span,
            Declaration::PackageBody(d) => d.span,
            Declaration::PackageInstantiation(d) => d.span,
            Declaration::Use(d) => d.span,
            Declaration::GroupTemplate(d) => d.span,
            Declaration::Group(d) => d.span,
            Declaration::Disconnection(d) => d.span,
            Declaration::ConfigurationSpec(d) => d.span,
        }
    }
}

/// `constant|signal|[shared] variable names : subtype [kind] [:= init];`
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDecl {
    /// Which kind of object.
    pub kind: ObjectKind,
    /// The declared names.
    pub names: Vec<Ident>,
    /// The subtype indication.
    pub subtype: SubtypeIndication,
    /// `register` or `bus` for guarded signals.
    pub signal_kind: Option<SignalKind>,
    /// The initial value.
    pub init: Option<Expr>,
    /// The whole declaration.
    pub span: Span,
}

/// The kind of an [`ObjectDecl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// `constant`
    Constant,
    /// `signal`
    Signal,
    /// `variable`
    Variable,
    /// `shared variable`
    SharedVariable,
}

/// The kind of a guarded signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// `register`
    Register,
    /// `bus`
    Bus,
}

/// `file names : subtype [[open kind] is logical_name];`
#[derive(Debug, Clone, PartialEq)]
pub struct FileDecl {
    /// The declared names.
    pub names: Vec<Ident>,
    /// The file subtype.
    pub subtype: SubtypeIndication,
    /// The `open` kind expression, if any.
    pub open_kind: Option<Expr>,
    /// The logical name expression after `is`, if any. The VHDL-87 `is in`
    /// / `is out` mode is recorded in `mode87`.
    pub logical_name: Option<Expr>,
    /// The VHDL-87 mode written between `is` and the logical name.
    pub mode87: Option<Mode>,
    /// The whole declaration.
    pub span: Span,
}

/// `type name [is definition];`
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    /// The type name.
    pub name: Ident,
    /// The definition; `None` for an incomplete type declaration.
    pub def: Option<TypeDef>,
    /// The whole declaration.
    pub span: Span,
}

/// The definition part of a type declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDef {
    /// `(lit, lit, ...)`
    Enumeration(Vec<Designator>),
    /// `range r`: an integer or floating type, told apart semantically.
    Range(Range),
    /// `range r units ... end units`
    Physical(PhysicalTypeDef),
    /// `array (...) of subtype`
    Array(ArrayTypeDef),
    /// `record ... end record`
    Record(RecordTypeDef),
    /// `access subtype`
    Access(SubtypeIndication),
    /// `file of type_mark`
    File(Name),
    /// `protected ... end protected`
    Protected(ProtectedTypeDecl),
    /// `protected body ... end protected body`
    ProtectedBody(ProtectedTypeBody),
}

/// A physical type definition.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalTypeDef {
    /// The range constraint.
    pub range: Range,
    /// The primary unit.
    pub primary_unit: Ident,
    /// The secondary units, in order.
    pub secondary_units: Vec<SecondaryUnit>,
    /// From `range` to `end units [name]`.
    pub span: Span,
}

/// `name = physical_literal;` inside a physical type definition.
#[derive(Debug, Clone, PartialEq)]
pub struct SecondaryUnit {
    /// The unit name.
    pub name: Ident,
    /// The value, a physical literal (or a bare unit name as a name).
    pub value: Expr,
    /// The whole declaration.
    pub span: Span,
}

/// `array (index, ...) of element`
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayTypeDef {
    /// One entry per dimension.
    pub indices: Vec<ArrayIndex>,
    /// The element subtype.
    pub element: SubtypeIndication,
    /// The whole definition.
    pub span: Span,
}

/// One dimension of an array type definition.
#[derive(Debug, Clone, PartialEq)]
pub enum ArrayIndex {
    /// `type_mark range <>`
    Unbounded(Name),
    /// A discrete range.
    Constrained(DiscreteRange),
}

impl ArrayIndex {
    /// The span of the dimension.
    pub fn span(&self) -> Span {
        match self {
            ArrayIndex::Unbounded(n) => n.span(),
            ArrayIndex::Constrained(r) => r.span(),
        }
    }
}

/// `record elements end record`
#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeDef {
    /// The element declarations.
    pub elements: Vec<RecordElement>,
    /// The whole definition.
    pub span: Span,
}

/// `names : subtype;` inside a record.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordElement {
    /// The element names.
    pub names: Vec<Ident>,
    /// The element subtype.
    pub subtype: SubtypeIndication,
    /// The whole element declaration.
    pub span: Span,
}

/// `protected decls end protected`
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectedTypeDecl {
    /// The declarative items (subprogram declarations, attributes, uses).
    pub decls: Vec<Declaration>,
    /// The whole definition.
    pub span: Span,
}

/// `protected body decls end protected body`
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectedTypeBody {
    /// The declarative items.
    pub decls: Vec<Declaration>,
    /// The whole definition.
    pub span: Span,
}

/// `subtype name is subtype_indication;`
#[derive(Debug, Clone, PartialEq)]
pub struct SubtypeDecl {
    /// The subtype name.
    pub name: Ident,
    /// The subtype indication.
    pub subtype: SubtypeIndication,
    /// The whole declaration.
    pub span: Span,
}

/// `alias designator [: subtype] is name [signature];`
#[derive(Debug, Clone, PartialEq)]
pub struct AliasDecl {
    /// The alias designator.
    pub designator: Designator,
    /// The subtype indication for object aliases.
    pub subtype: Option<SubtypeIndication>,
    /// The aliased name.
    pub target: Name,
    /// The signature for subprogram and enumeration literal aliases.
    pub signature: Option<Signature>,
    /// The whole declaration.
    pub span: Span,
}

/// `attribute name : type_mark;`
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDecl {
    /// The attribute name.
    pub name: Ident,
    /// The attribute's type.
    pub type_mark: Name,
    /// The whole declaration.
    pub span: Span,
}

/// `attribute name of entities : class is value;`
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeSpec {
    /// The attribute being specified.
    pub attribute: Ident,
    /// The entities it applies to.
    pub entities: EntityNameList,
    /// The entity class.
    pub class: EntityClass,
    /// The attribute value.
    pub value: Expr,
    /// The whole specification.
    pub span: Span,
}

/// The entities named by an attribute specification.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityNameList {
    /// Explicit designators.
    Names(Vec<EntityDesignator>),
    /// `others`
    Others(Span),
    /// `all`
    All(Span),
}

/// `designator [signature]` in an attribute specification.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityDesignator {
    /// The designator.
    pub designator: Designator,
    /// The signature, for overloaded subprograms.
    pub signature: Option<Signature>,
    /// The whole designator.
    pub span: Span,
}

/// The class of a named entity in attribute specifications and group
/// templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum EntityClass {
    Entity,
    Architecture,
    Configuration,
    Procedure,
    Function,
    Package,
    Type,
    Subtype,
    Constant,
    Signal,
    Variable,
    Component,
    Label,
    Literal,
    Units,
    Group,
    File,
    Property,
    Sequence,
}

impl EntityClass {
    /// The reserved word for this class.
    pub fn as_str(self) -> &'static str {
        match self {
            EntityClass::Entity => "entity",
            EntityClass::Architecture => "architecture",
            EntityClass::Configuration => "configuration",
            EntityClass::Procedure => "procedure",
            EntityClass::Function => "function",
            EntityClass::Package => "package",
            EntityClass::Type => "type",
            EntityClass::Subtype => "subtype",
            EntityClass::Constant => "constant",
            EntityClass::Signal => "signal",
            EntityClass::Variable => "variable",
            EntityClass::Component => "component",
            EntityClass::Label => "label",
            EntityClass::Literal => "literal",
            EntityClass::Units => "units",
            EntityClass::Group => "group",
            EntityClass::File => "file",
            EntityClass::Property => "property",
            EntityClass::Sequence => "sequence",
        }
    }
}

/// `component name [is] [generic (...);] [port (...);] end component;`
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDecl {
    /// The component name.
    pub name: Ident,
    /// The generic clause, empty when absent.
    pub generics: Vec<InterfaceDecl>,
    /// The port clause, empty when absent.
    pub ports: Vec<InterfaceDecl>,
    /// The whole declaration.
    pub span: Span,
}

/// Whether a subprogram is a procedure or a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubprogramKind {
    /// `procedure`
    Procedure,
    /// `function`
    Function,
}

/// The header of a subprogram: kind, designator, generics and parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SubprogramSpec {
    /// Procedure or function.
    pub kind: SubprogramKind,
    /// `Some(true)` for `pure`, `Some(false)` for `impure`, `None` when
    /// unspecified (functions default to pure).
    pub pure: Option<bool>,
    /// The subprogram designator.
    pub designator: Designator,
    /// The generic clause (VHDL-2008), empty when absent.
    pub generics: Vec<InterfaceDecl>,
    /// The generic map aspect following the generic clause (VHDL-2008).
    pub generic_map: Option<Vec<AssociationElement>>,
    /// The formal parameters, empty when absent.
    pub params: Vec<InterfaceDecl>,
    /// The return type mark, for functions.
    pub return_type: Option<Name>,
    /// The whole specification.
    pub span: Span,
}

/// `subprogram_specification;`
#[derive(Debug, Clone, PartialEq)]
pub struct SubprogramDecl {
    /// The specification.
    pub spec: SubprogramSpec,
    /// The whole declaration.
    pub span: Span,
}

/// `subprogram_specification is decls begin stmts end;`
#[derive(Debug, Clone, PartialEq)]
pub struct SubprogramBody {
    /// The specification.
    pub spec: SubprogramSpec,
    /// The subprogram declarative part.
    pub decls: Vec<Declaration>,
    /// The statements.
    pub statements: Vec<SequentialStatement>,
    /// The whole body.
    pub span: Span,
}

/// `function|procedure designator is new name [signature] [generic map];`
#[derive(Debug, Clone, PartialEq)]
pub struct SubprogramInstantiation {
    /// Procedure or function.
    pub kind: SubprogramKind,
    /// The new subprogram's designator.
    pub designator: Designator,
    /// The uninstantiated subprogram.
    pub uninstantiated: Name,
    /// The signature selecting an overload.
    pub signature: Option<Signature>,
    /// The generic map aspect, empty when absent.
    pub generic_map: Vec<AssociationElement>,
    /// The whole declaration.
    pub span: Span,
}

/// `group name is (class [<>], ...);`
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTemplateDecl {
    /// The template name.
    pub name: Ident,
    /// The entity class entries.
    pub entries: Vec<GroupTemplateEntry>,
    /// The whole declaration.
    pub span: Span,
}

/// One `class [<>]` entry of a group template.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupTemplateEntry {
    /// The entity class.
    pub class: EntityClass,
    /// True when followed by `<>`.
    pub unbounded: bool,
    /// The whole entry.
    pub span: Span,
}

/// `group name : template (constituents);`
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDecl {
    /// The group name.
    pub name: Ident,
    /// The group template.
    pub template: Name,
    /// The constituents, names or character literals kept as expressions.
    pub constituents: Vec<Expr>,
    /// The whole declaration.
    pub span: Span,
}

/// `disconnect signals : type_mark after time;`
#[derive(Debug, Clone, PartialEq)]
pub struct DisconnectionSpec {
    /// The guarded signals.
    pub signals: SignalList,
    /// The type mark.
    pub type_mark: Name,
    /// The disconnection time expression.
    pub time: Expr,
    /// The whole specification.
    pub span: Span,
}

/// The signals of a disconnection specification.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalList {
    /// Explicit signal names.
    Names(Vec<Name>),
    /// `others`
    Others(Span),
    /// `all`
    All(Span),
}

/// `for component_specification binding_indication; [end for;]`
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigurationSpec {
    /// Which instances of which component.
    pub spec: ComponentSpecification,
    /// How they are bound.
    pub binding: BindingIndication,
    /// The whole specification.
    pub span: Span,
}

// --- types, subtypes and ranges ---------------------------------------------

/// `[resolution] type_mark [constraint]`
#[derive(Debug, Clone, PartialEq)]
pub struct SubtypeIndication {
    /// The resolution indication, if any.
    pub resolution: Option<ResolutionIndication>,
    /// The type mark (a type or subtype name).
    pub type_mark: Name,
    /// The constraint, if any.
    pub constraint: Option<Constraint>,
    /// The whole indication.
    pub span: Span,
}

/// A resolution indication (VHDL-2008 element resolutions included).
#[derive(Debug, Clone, PartialEq)]
pub enum ResolutionIndication {
    /// A resolution function name.
    Function(Name),
    /// `(resolution)`: applies to the elements of an array.
    Array(Box<ResolutionIndication>, Span),
    /// `(name resolution, ...)`: applies to the elements of a record.
    Record(Vec<RecordResolution>, Span),
}

impl ResolutionIndication {
    /// The span of the indication.
    pub fn span(&self) -> Span {
        match self {
            ResolutionIndication::Function(n) => n.span(),
            ResolutionIndication::Array(_, span) | ResolutionIndication::Record(_, span) => *span,
        }
    }
}

/// `name resolution` inside a record element resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordResolution {
    /// The record element.
    pub name: Ident,
    /// Its resolution.
    pub resolution: ResolutionIndication,
    /// The whole entry.
    pub span: Span,
}

/// A constraint on a subtype indication.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// `range r`
    Range(Range),
    /// `(discrete_range, ...) [element_constraint]` or `(open)
    /// [element_constraint]` (VHDL-2008).
    Array {
        /// The index ranges; empty for `(open)`.
        indices: Vec<DiscreteRange>,
        /// The element constraint following the index constraint.
        element: Option<Box<Constraint>>,
        /// The whole constraint.
        span: Span,
    },
    /// `(name constraint, ...)` (VHDL-2008).
    Record(Vec<RecordConstraint>, Span),
}

impl Constraint {
    /// The span of the constraint.
    pub fn span(&self) -> Span {
        match self {
            Constraint::Range(r) => r.span(),
            Constraint::Array { span, .. } | Constraint::Record(_, span) => *span,
        }
    }
}

/// `name constraint` inside a record constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordConstraint {
    /// The record element.
    pub name: Ident,
    /// Its constraint.
    pub constraint: Constraint,
    /// The whole entry.
    pub span: Span,
}

/// A range.
#[derive(Debug, Clone, PartialEq)]
pub enum Range {
    /// `left to right` or `left downto right`.
    Bounds {
        /// The left bound.
        left: Expr,
        /// The direction.
        direction: Direction,
        /// The right bound.
        right: Expr,
        /// The whole range.
        span: Span,
    },
    /// A range attribute such as `x'range` or `x'reverse_range(1)`.
    Attribute(Name),
}

impl Range {
    /// The span of the range.
    pub fn span(&self) -> Span {
        match self {
            Range::Bounds { span, .. } => *span,
            Range::Attribute(n) => n.span(),
        }
    }
}

/// The direction of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `to`
    To,
    /// `downto`
    Downto,
}

impl Direction {
    /// The reserved word.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::To => "to",
            Direction::Downto => "downto",
        }
    }
}

/// A discrete range: a range or a discrete subtype indication.
#[derive(Debug, Clone, PartialEq)]
pub enum DiscreteRange {
    /// An explicit range or range attribute.
    Range(Range),
    /// A subtype indication (`natural`, `natural range 0 to 3`).
    Subtype(Box<SubtypeIndication>),
}

impl DiscreteRange {
    /// The span of the range.
    pub fn span(&self) -> Span {
        match self {
            DiscreteRange::Range(r) => r.span(),
            DiscreteRange::Subtype(s) => s.span,
        }
    }
}

/// `[type_mark {, type_mark} [return type_mark]]`
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// The parameter type marks.
    pub params: Vec<Name>,
    /// The return type mark.
    pub return_type: Option<Name>,
    /// The whole signature, brackets included.
    pub span: Span,
}

// --- names and expressions --------------------------------------------------

/// A name.
#[derive(Debug, Clone, PartialEq)]
pub enum Name {
    /// A simple name.
    Simple(Ident),
    /// An operator symbol used as a name, such as `"+"` in `"+"(a, b)`.
    Operator {
        /// The operator text without quotes.
        symbol: String,
        /// Where it was written.
        span: Span,
    },
    /// A character literal used as a name, such as `'0'` in `'0'(...)`.
    Char {
        /// The character.
        ch: char,
        /// Where it was written.
        span: Span,
    },
    /// `prefix.suffix`
    Selected {
        /// The prefix.
        prefix: Box<Name>,
        /// The suffix.
        suffix: Suffix,
        /// The whole name.
        span: Span,
    },
    /// `prefix(args)`: a function call, index, type conversion or attribute
    /// argument, to be told apart semantically.
    Call {
        /// The prefix.
        prefix: Box<Name>,
        /// The arguments; empty for `f()` (which is not valid VHDL but
        /// parses).
        args: Vec<AssociationElement>,
        /// The whole name.
        span: Span,
    },
    /// `prefix(discrete_range)`
    Slice {
        /// The prefix.
        prefix: Box<Name>,
        /// The range.
        range: Box<DiscreteRange>,
        /// The whole name.
        span: Span,
    },
    /// `prefix[signature]'attribute`. An argument, if any, appears as a
    /// [`Name::Call`] on this name.
    Attribute {
        /// The prefix.
        prefix: Box<Name>,
        /// The signature before the tick.
        signature: Option<Box<Signature>>,
        /// The attribute designator (`range` and `subtype` included).
        attribute: Ident,
        /// The whole name.
        span: Span,
    },
    /// `<< class path : subtype >>` (VHDL-2008).
    External(Box<ExternalName>),
}

impl Name {
    /// The span of the name.
    pub fn span(&self) -> Span {
        match self {
            Name::Simple(i) => i.span,
            Name::Operator { span, .. }
            | Name::Char { span, .. }
            | Name::Selected { span, .. }
            | Name::Call { span, .. }
            | Name::Slice { span, .. }
            | Name::Attribute { span, .. } => *span,
            Name::External(e) => e.span,
        }
    }

    /// The innermost prefix: the simple name, operator or external name the
    /// selections, calls and attributes are applied to.
    pub fn root(&self) -> &Name {
        match self {
            Name::Selected { prefix, .. }
            | Name::Call { prefix, .. }
            | Name::Slice { prefix, .. }
            | Name::Attribute { prefix, .. } => prefix.root(),
            _ => self,
        }
    }
}

/// The suffix of a selected name.
#[derive(Debug, Clone, PartialEq)]
pub enum Suffix {
    /// An identifier, character literal or operator symbol.
    Designator(Designator),
    /// `all`
    All(Span),
}

impl Suffix {
    /// The span of the suffix.
    pub fn span(&self) -> Span {
        match self {
            Suffix::Designator(d) => d.span(),
            Suffix::All(span) => *span,
        }
    }
}

/// `<< class path : subtype >>`
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalName {
    /// The object class.
    pub class: ExternalClass,
    /// The path to the object.
    pub path: ExternalPath,
    /// The object's subtype.
    pub subtype: SubtypeIndication,
    /// The whole name, `<<` and `>>` included.
    pub span: Span,
}

/// The class of an external name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalClass {
    /// `constant`
    Constant,
    /// `signal`
    Signal,
    /// `variable`
    Variable,
}

/// The pathname of an external name.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalPath {
    /// How the path is anchored.
    pub kind: ExternalPathKind,
    /// The path elements, the last one being the object.
    pub elements: Vec<PathElement>,
    /// The whole path.
    pub span: Span,
}

/// How an external pathname is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalPathKind {
    /// `@lib.pkg.object`
    Package,
    /// `.top.a.b`
    Absolute,
    /// `a.b` or `^.^.a.b`, with the number of `^` steps.
    Relative(u32),
}

/// One element of an external pathname: a label with an optional generate
/// index.
#[derive(Debug, Clone, PartialEq)]
pub struct PathElement {
    /// The label or object name.
    pub name: Ident,
    /// The generate index in parentheses, if any.
    pub index: Option<Expr>,
    /// The whole element.
    pub span: Span,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `lhs op rhs`
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: Box<Expr>,
        /// The right operand.
        rhs: Box<Expr>,
        /// The whole expression.
        span: Span,
    },
    /// `op operand`
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
        /// The whole expression.
        span: Span,
    },
    /// A name (which may be a call or a type conversion).
    Name(Name),
    /// A literal.
    Literal(Literal),
    /// An aggregate.
    Aggregate(Aggregate),
    /// `type_mark'(operand)`; the operand is a parenthesised expression or
    /// an aggregate.
    Qualified {
        /// The type mark.
        type_mark: Name,
        /// The operand.
        operand: Box<Expr>,
        /// The whole expression.
        span: Span,
    },
    /// `new subtype` or `new qualified_expression`.
    Allocator {
        /// What is allocated.
        kind: Box<Allocator>,
        /// The whole expression.
        span: Span,
    },
    /// `(expr)`
    Paren {
        /// The inner expression.
        inner: Box<Expr>,
        /// The whole expression, parentheses included.
        span: Span,
    },
    /// `open`, only valid as an actual.
    Open(Span),
    /// A placeholder for an expression that failed to parse; the error has
    /// been reported. Produced only where keeping the enclosing statement
    /// is more useful than dropping it (an `if` or `while` condition).
    Error(Span),
}

impl Expr {
    /// The span of the expression.
    pub fn span(&self) -> Span {
        match self {
            Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Qualified { span, .. }
            | Expr::Allocator { span, .. }
            | Expr::Paren { span, .. }
            | Expr::Open(span)
            | Expr::Error(span) => *span,
            Expr::Name(n) => n.span(),
            Expr::Literal(l) => l.span,
            Expr::Aggregate(a) => a.span,
        }
    }
}

/// What an allocator creates.
#[derive(Debug, Clone, PartialEq)]
pub enum Allocator {
    /// `new subtype_indication`
    Subtype(SubtypeIndication),
    /// `new type_mark'(value)`
    Qualified {
        /// The type mark.
        type_mark: Name,
        /// The initial value.
        operand: Expr,
    },
}

/// A binary operator, in the order of IEEE 1076-2008 clause 9.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum BinaryOp {
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    MatchEq,
    MatchNeq,
    MatchLt,
    MatchLe,
    MatchGt,
    MatchGe,
    Sll,
    Srl,
    Sla,
    Sra,
    Rol,
    Ror,
    Add,
    Sub,
    Concat,
    Mul,
    Div,
    Mod,
    Rem,
    Pow,
}

impl BinaryOp {
    /// The operator as written (lowercase for reserved words).
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::Nand => "nand",
            BinaryOp::Nor => "nor",
            BinaryOp::Xor => "xor",
            BinaryOp::Xnor => "xnor",
            BinaryOp::Eq => "=",
            BinaryOp::Neq => "/=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::MatchEq => "?=",
            BinaryOp::MatchNeq => "?/=",
            BinaryOp::MatchLt => "?<",
            BinaryOp::MatchLe => "?<=",
            BinaryOp::MatchGt => "?>",
            BinaryOp::MatchGe => "?>=",
            BinaryOp::Sll => "sll",
            BinaryOp::Srl => "srl",
            BinaryOp::Sla => "sla",
            BinaryOp::Sra => "sra",
            BinaryOp::Rol => "rol",
            BinaryOp::Ror => "ror",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Concat => "&",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "mod",
            BinaryOp::Rem => "rem",
            BinaryOp::Pow => "**",
        }
    }

    /// True for the six logical operators.
    pub fn is_logical(self) -> bool {
        matches!(
            self,
            BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Nand
                | BinaryOp::Nor
                | BinaryOp::Xor
                | BinaryOp::Xnor
        )
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `abs`
    Abs,
    /// `not`
    Not,
    /// `??`, the VHDL-2008 condition operator.
    Condition,
    /// `and` as a reduction operator (VHDL-2008).
    And,
    /// `or` as a reduction operator (VHDL-2008).
    Or,
    /// `nand` as a reduction operator (VHDL-2008).
    Nand,
    /// `nor` as a reduction operator (VHDL-2008).
    Nor,
    /// `xor` as a reduction operator (VHDL-2008).
    Xor,
    /// `xnor` as a reduction operator (VHDL-2008).
    Xnor,
}

impl UnaryOp {
    /// The operator as written.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Minus => "-",
            UnaryOp::Abs => "abs",
            UnaryOp::Not => "not",
            UnaryOp::Condition => "??",
            UnaryOp::And => "and",
            UnaryOp::Or => "or",
            UnaryOp::Nand => "nand",
            UnaryOp::Nor => "nor",
            UnaryOp::Xor => "xor",
            UnaryOp::Xnor => "xnor",
        }
    }
}

/// A literal with its raw text.
#[derive(Debug, Clone, PartialEq)]
pub struct Literal {
    /// What kind of literal.
    pub kind: LiteralKind,
    /// Where it was written.
    pub span: Span,
}

/// The kinds of literal.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralKind {
    /// An integer literal, raw text (`42`, `16#FF#`, `1E3`).
    Integer(String),
    /// A real literal, raw text (`3.14`, `1.0E-3`).
    Real(String),
    /// A physical literal: an abstract literal followed by a unit name.
    Physical {
        /// The raw abstract literal text.
        value: String,
        /// The unit.
        unit: Ident,
    },
    /// A character literal.
    Char(char),
    /// A string literal, delimiters removed and doubled quotes unescaped.
    String(String),
    /// A bit-string literal, raw text including base and length.
    BitString(String),
    /// `null`
    Null,
}

/// `(element_association, ...)`
#[derive(Debug, Clone, PartialEq)]
pub struct Aggregate {
    /// The element associations, in order.
    pub elements: Vec<ElementAssociation>,
    /// The whole aggregate, parentheses included.
    pub span: Span,
}

/// `[choices =>] expr` inside an aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementAssociation {
    /// The choices; empty for a positional association.
    pub choices: Vec<Choice>,
    /// The element value.
    pub value: Expr,
    /// The whole association.
    pub span: Span,
}

/// One alternative of a choice list.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    /// A (simple) expression or an element name.
    Expr(Expr),
    /// A discrete range.
    Range(DiscreteRange),
    /// `others`
    Others(Span),
}

impl Choice {
    /// The span of the choice.
    pub fn span(&self) -> Span {
        match self {
            Choice::Expr(e) => e.span(),
            Choice::Range(r) => r.span(),
            Choice::Others(span) => *span,
        }
    }
}

// --- concurrent statements --------------------------------------------------

/// A labelled concurrent statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcurrentStatement {
    /// The label, if any.
    pub label: Option<Ident>,
    /// The statement.
    pub kind: ConcurrentKind,
    /// The whole statement, label and `;` included.
    pub span: Span,
}

/// The kinds of concurrent statement.
#[derive(Debug, Clone, PartialEq)]
pub enum ConcurrentKind {
    /// A process.
    Process(ProcessStatement),
    /// A block.
    Block(BlockStatement),
    /// A concurrent signal assignment.
    SignalAssignment(ConcurrentSignalAssignment),
    /// A concurrent procedure call.
    ProcedureCall {
        /// True for `postponed`.
        postponed: bool,
        /// The call (a name, possibly with arguments).
        call: Name,
    },
    /// A concurrent assertion.
    Assertion {
        /// True for `postponed`.
        postponed: bool,
        /// The assertion.
        assertion: Assertion,
    },
    /// A component, entity or configuration instantiation.
    Instantiation(Instantiation),
    /// `for` generate.
    ForGenerate(ForGenerate),
    /// `if` generate.
    IfGenerate(IfGenerate),
    /// `case` generate (VHDL-2008).
    CaseGenerate(CaseGenerate),
}

/// `[postponed] process [(sensitivity)] [is] decls begin stmts end process;`
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessStatement {
    /// True for `postponed`.
    pub postponed: bool,
    /// The sensitivity list, if any.
    pub sensitivity: Option<Sensitivity>,
    /// The process declarative part.
    pub decls: Vec<Declaration>,
    /// The statements.
    pub statements: Vec<SequentialStatement>,
    /// From `process` (or `postponed`) to the closing `;`.
    pub span: Span,
}

/// The sensitivity list of a process or wait statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Sensitivity {
    /// Explicit signal names.
    Names(Vec<Name>),
    /// `all` (VHDL-2008).
    All(Span),
}

/// `block [(guard)] [is] [generic ...] [port ...] decls begin stmts end block;`
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStatement {
    /// The guard condition.
    pub guard: Option<Expr>,
    /// The generic clause, empty when absent.
    pub generics: Vec<InterfaceDecl>,
    /// The generic map aspect.
    pub generic_map: Option<Vec<AssociationElement>>,
    /// The port clause, empty when absent.
    pub ports: Vec<InterfaceDecl>,
    /// The port map aspect.
    pub port_map: Option<Vec<AssociationElement>>,
    /// The block declarative part.
    pub decls: Vec<Declaration>,
    /// The statements.
    pub statements: Vec<ConcurrentStatement>,
    /// From `block` to the closing `;`.
    pub span: Span,
}

/// A concurrent signal assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcurrentSignalAssignment {
    /// True for `postponed`.
    pub postponed: bool,
    /// True for `guarded`.
    pub guarded: bool,
    /// The assignment.
    pub assignment: SignalAssignment,
}

/// `assert condition [report expr] [severity expr]`
#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    /// The condition.
    pub condition: Expr,
    /// The report expression.
    pub report: Option<Expr>,
    /// The severity expression.
    pub severity: Option<Expr>,
    /// From `assert` to the last expression.
    pub span: Span,
}

/// `unit [generic map (...)] [port map (...)];`
#[derive(Debug, Clone, PartialEq)]
pub struct Instantiation {
    /// What is instantiated.
    pub unit: InstantiatedUnit,
    /// The generic map aspect.
    pub generic_map: Option<Vec<AssociationElement>>,
    /// The port map aspect.
    pub port_map: Option<Vec<AssociationElement>>,
    /// From the unit to the closing `;`.
    pub span: Span,
}

/// The instantiated unit of an instantiation statement.
#[derive(Debug, Clone, PartialEq)]
pub enum InstantiatedUnit {
    /// `[component] name`
    Component(Name),
    /// `entity name [(architecture)]`
    Entity {
        /// The entity.
        name: Name,
        /// The architecture, if given.
        architecture: Option<Ident>,
    },
    /// `configuration name`
    Configuration(Name),
}

/// `for param in range generate body end generate;`
#[derive(Debug, Clone, PartialEq)]
pub struct ForGenerate {
    /// The generate parameter.
    pub param: Ident,
    /// The iteration range.
    pub range: DiscreteRange,
    /// The body.
    pub body: GenerateBody,
    /// From `for` to the closing `;`.
    pub span: Span,
}

/// `if [label:] cond generate body {elsif ...} [else ...] end generate;`
#[derive(Debug, Clone, PartialEq)]
pub struct IfGenerate {
    /// The `if` and `elsif` arms.
    pub arms: Vec<IfGenerateArm>,
    /// The `else` arm (VHDL-2008).
    pub else_arm: Option<GenerateBody>,
    /// From `if` to the closing `;`.
    pub span: Span,
}

/// One `if` or `elsif` arm of an if-generate.
#[derive(Debug, Clone, PartialEq)]
pub struct IfGenerateArm {
    /// The condition.
    pub condition: Expr,
    /// The body.
    pub body: GenerateBody,
    /// From `if`/`elsif` to the end of the body.
    pub span: Span,
}

/// `case expr generate {when [label:] choices => body} end generate;`
#[derive(Debug, Clone, PartialEq)]
pub struct CaseGenerate {
    /// The selector expression.
    pub expr: Expr,
    /// The alternatives.
    pub arms: Vec<CaseGenerateArm>,
    /// From `case` to the closing `;`.
    pub span: Span,
}

/// One `when` alternative of a case-generate.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseGenerateArm {
    /// The choices.
    pub choices: Vec<Choice>,
    /// The body.
    pub body: GenerateBody,
    /// From `when` to the end of the body.
    pub span: Span,
}

/// `[label:] [decls begin] stmts [end [label];]`
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateBody {
    /// The alternative label (VHDL-2008).
    pub label: Option<Ident>,
    /// The declarative part.
    pub decls: Vec<Declaration>,
    /// The statements.
    pub statements: Vec<ConcurrentStatement>,
    /// The body.
    pub span: Span,
}

// --- sequential statements --------------------------------------------------

/// A labelled sequential statement.
#[derive(Debug, Clone, PartialEq)]
pub struct SequentialStatement {
    /// The label, if any.
    pub label: Option<Ident>,
    /// The statement.
    pub kind: SequentialKind,
    /// The whole statement, label and `;` included.
    pub span: Span,
}

/// The kinds of sequential statement.
#[derive(Debug, Clone, PartialEq)]
pub enum SequentialKind {
    /// `wait [on ...] [until ...] [for ...];`
    Wait {
        /// The sensitivity clause.
        sensitivity: Option<Sensitivity>,
        /// The condition clause.
        condition: Option<Expr>,
        /// The timeout clause.
        timeout: Option<Expr>,
    },
    /// An assertion.
    Assertion(Assertion),
    /// `report expr [severity expr];`
    Report {
        /// The message.
        message: Expr,
        /// The severity expression.
        severity: Option<Expr>,
    },
    /// A signal assignment.
    SignalAssignment(SignalAssignment),
    /// A variable assignment.
    VariableAssignment(VariableAssignment),
    /// A procedure call.
    ProcedureCall(Name),
    /// An if statement.
    If(IfStatement),
    /// A case statement.
    Case(CaseStatement),
    /// A loop.
    Loop(LoopStatement),
    /// `next [label] [when cond];`
    Next {
        /// The loop label.
        label: Option<Ident>,
        /// The condition.
        condition: Option<Expr>,
    },
    /// `exit [label] [when cond];`
    Exit {
        /// The loop label.
        label: Option<Ident>,
        /// The condition.
        condition: Option<Expr>,
    },
    /// `return [expr];`
    Return(Option<Expr>),
    /// `null;`
    Null,
}

/// The target of an assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A name.
    Name(Name),
    /// An aggregate.
    Aggregate(Aggregate),
}

impl Target {
    /// The span of the target.
    pub fn span(&self) -> Span {
        match self {
            Target::Name(n) => n.span(),
            Target::Aggregate(a) => a.span,
        }
    }
}

/// `target <= [delay] rhs`
#[derive(Debug, Clone, PartialEq)]
pub struct SignalAssignment {
    /// The target.
    pub target: Target,
    /// The delay mechanism.
    pub delay: Option<DelayMechanism>,
    /// The right-hand side.
    pub rhs: SignalAssignmentRhs,
    /// From the target to the end of the right-hand side.
    pub span: Span,
}

/// The right-hand side of a signal assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalAssignmentRhs {
    /// A single waveform.
    Simple(Waveform),
    /// `waveform when cond {else waveform when cond} [else waveform]`.
    Conditional(Vec<ConditionalWaveform>),
    /// `with expr select [?] target <= waveform when choices, ...`.
    Selected {
        /// The selector expression.
        selector: Expr,
        /// True for `select?`.
        matching: bool,
        /// The alternatives.
        arms: Vec<SelectedWaveform>,
    },
    /// `force [mode] expr [when ...]` (VHDL-2008).
    Force {
        /// `in` or `out`.
        mode: Option<Mode>,
        /// The conditional expressions; a single unconditional arm for a
        /// plain force.
        arms: Vec<ConditionalExpr>,
    },
    /// `release [mode]` (VHDL-2008).
    Release {
        /// `in` or `out`.
        mode: Option<Mode>,
    },
}

/// `transport` or `[reject time] inertial`.
#[derive(Debug, Clone, PartialEq)]
pub enum DelayMechanism {
    /// `transport`
    Transport(Span),
    /// `[reject time] inertial`
    Inertial {
        /// The reject time limit.
        reject: Option<Expr>,
        /// The whole mechanism.
        span: Span,
    },
}

/// A waveform.
#[derive(Debug, Clone, PartialEq)]
pub enum Waveform {
    /// `element {, element}`
    Elements(Vec<WaveformElement>),
    /// `unaffected`
    Unaffected(Span),
}

impl Waveform {
    /// The span of the waveform.
    pub fn span(&self) -> Span {
        match self {
            Waveform::Elements(els) => match (els.first(), els.last()) {
                (Some(a), Some(b)) => a.span.to(b.span),
                _ => panic!("empty waveform"),
            },
            Waveform::Unaffected(span) => *span,
        }
    }
}

/// `value [after time]`; the value may be `null`.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformElement {
    /// The value.
    pub value: Expr,
    /// The delay.
    pub after: Option<Expr>,
    /// The whole element.
    pub span: Span,
}

/// `waveform [when condition]`
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalWaveform {
    /// The waveform.
    pub waveform: Waveform,
    /// The condition; `None` for the final unconditional `else`.
    pub condition: Option<Expr>,
    /// The whole arm.
    pub span: Span,
}

/// `waveform when choices`
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedWaveform {
    /// The waveform.
    pub waveform: Waveform,
    /// The choices.
    pub choices: Vec<Choice>,
    /// The whole arm.
    pub span: Span,
}

/// `expr [when condition]`
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalExpr {
    /// The expression.
    pub value: Expr,
    /// The condition; `None` for the final unconditional `else`.
    pub condition: Option<Expr>,
    /// The whole arm.
    pub span: Span,
}

/// `expr when choices`
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedExpr {
    /// The expression.
    pub value: Expr,
    /// The choices.
    pub choices: Vec<Choice>,
    /// The whole arm.
    pub span: Span,
}

/// `target := rhs`
#[derive(Debug, Clone, PartialEq)]
pub struct VariableAssignment {
    /// The target.
    pub target: Target,
    /// The right-hand side.
    pub rhs: VariableAssignmentRhs,
    /// From the target to the end of the right-hand side.
    pub span: Span,
}

/// The right-hand side of a variable assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum VariableAssignmentRhs {
    /// A single expression.
    Simple(Expr),
    /// `expr when cond {else expr when cond} [else expr]` (VHDL-2008).
    Conditional(Vec<ConditionalExpr>),
    /// `with expr select [?] target := expr when choices, ...` (VHDL-2008).
    Selected {
        /// The selector expression.
        selector: Expr,
        /// True for `select?`.
        matching: bool,
        /// The alternatives.
        arms: Vec<SelectedExpr>,
    },
}

/// `if cond then stmts {elsif cond then stmts} [else stmts] end if;`
#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    /// The `if` and `elsif` arms.
    pub arms: Vec<IfArm>,
    /// The `else` statements.
    pub else_statements: Option<Vec<SequentialStatement>>,
    /// From `if` to the closing `;`.
    pub span: Span,
}

/// One `if` or `elsif` arm.
#[derive(Debug, Clone, PartialEq)]
pub struct IfArm {
    /// The condition.
    pub condition: Expr,
    /// The statements.
    pub statements: Vec<SequentialStatement>,
    /// From `if`/`elsif` to the end of the statements.
    pub span: Span,
}

/// `case[?] expr is {when choices => stmts} end case[?];`
#[derive(Debug, Clone, PartialEq)]
pub struct CaseStatement {
    /// The selector expression.
    pub expr: Expr,
    /// True for `case?`.
    pub matching: bool,
    /// The alternatives.
    pub arms: Vec<CaseArm>,
    /// From `case` to the closing `;`.
    pub span: Span,
}

/// One `when` alternative of a case statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseArm {
    /// The choices.
    pub choices: Vec<Choice>,
    /// The statements.
    pub statements: Vec<SequentialStatement>,
    /// From `when` to the end of the statements.
    pub span: Span,
}

/// `[iteration_scheme] loop stmts end loop;`
#[derive(Debug, Clone, PartialEq)]
pub struct LoopStatement {
    /// The iteration scheme.
    pub scheme: Option<IterationScheme>,
    /// The statements.
    pub statements: Vec<SequentialStatement>,
    /// From the scheme (or `loop`) to the closing `;`.
    pub span: Span,
}

/// `while cond` or `for param in range`.
#[derive(Debug, Clone, PartialEq)]
pub enum IterationScheme {
    /// `while condition`
    While(Expr),
    /// `for param in range`
    For {
        /// The loop parameter.
        param: Ident,
        /// The iteration range.
        range: DiscreteRange,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SourceId, Span};

    fn ident(name: &str, extended: bool) -> Ident {
        let map = {
            let mut m = crate::source::SourceMap::new();
            m.add("x", "").unwrap();
            m
        };
        let id: SourceId = map.files().next().unwrap().0;
        Ident {
            name: name.to_owned(),
            span: Span::new(id, 0, 0),
            extended,
        }
    }

    #[test]
    fn ident_equality() {
        assert!(ident("Clk", false).same_as(&ident("CLK", false)));
        assert!(!ident("Clk", true).same_as(&ident("CLK", true)));
        assert!(ident("Clk", true).same_as(&ident("Clk", true)));
        assert!(!ident("clk", false).same_as(&ident("clk", true)));
        assert!(!ident("clk", false).same_as(&ident("clock", false)));
    }
}
