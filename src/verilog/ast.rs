//! The Verilog / SystemVerilog abstract syntax tree.
//!
//! The parser produces exactly what was written, with no semantic
//! interpretation: identifiers are plain strings, numeric literals keep
//! their source text for [`crate::logic::Logic`] to interpret once the
//! context-dependent sizing rules are known, and no name is resolved.
//! Elaboration works from this tree.
//!
//! Every node carries a [`Span`] so later stages can report precisely, and
//! every type derives `Debug`, `Clone` and `PartialEq` so tests can compare
//! trees. Shape follows the standard's grammar loosely: constructs that
//! differ only in a keyword share a node with a discriminating enum (all
//! four `always` forms are one [`ItemKind::Always`]; module, interface,
//! program and primitive share [`Module`]), and constructs that the roadmap
//! treats shallowly (property expressions, clocking bodies, UDP tables) are
//! kept as [`RawTokens`] so they round-trip without a grammar of their own.
//!
//! The one deliberate unification: declarations inside a `begin`/`end`
//! block or a function body are [`StmtKind::Decl`] wrapping an ordinary
//! [`Item`], so a variable declaration has one representation wherever it
//! appears.

use crate::source::Span;

/// A name as written, with where it was written.
///
/// Escaped identifiers are stored without their backslash; `$root` and
/// other system names keep no `$` either (see [`ExprKind::SystemIdent`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    /// The identifier text.
    pub name: String,
    /// Where it appears.
    pub span: Span,
}

impl Ident {
    /// Builds an identifier.
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Ident {
            name: name.into(),
            span,
        }
    }
}

/// A run of tokens kept verbatim for constructs the parser does not model:
/// property expressions, clocking bodies and UDP table rows.
///
/// `text` is the tokens' text joined by single spaces (strings re-quoted),
/// which is stable and readable in dumps; the original spelling is
/// recoverable from `span`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTokens {
    /// The tokens' text, space separated.
    pub text: String,
    /// From the first token to the last.
    pub span: Span,
}

/// One parsed compilation unit: the items of a file, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    /// Top-level items: design units plus what SystemVerilog allows in the
    /// compilation-unit scope (typedefs, imports, functions, parameters).
    pub items: Vec<Item>,
    /// The whole file.
    pub span: Span,
}

/// One `name` or `name = value` inside `(* ... *)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The attribute name.
    pub name: Ident,
    /// The value, when one was given.
    pub value: Option<Expr>,
    /// From the name to the end of the value.
    pub span: Span,
}

// --- items ---------------------------------------------------------------

/// A declaration or construct that can appear in a module, interface,
/// package or the compilation-unit scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Attributes written before the item.
    pub attrs: Vec<Attribute>,
    /// What the item is.
    pub kind: ItemKind,
    /// From the first attribute or keyword to the closing `;` or end
    /// keyword.
    pub span: Span,
}

/// The kinds of [`Item`].
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// `module`, `macromodule`, `interface`, `program` or `primitive`.
    Module(Box<Module>),
    /// `package ... endpackage`.
    Package(Package),
    /// A net declaration (`wire`, `tri1`, `supply0`, ...).
    Net(NetDecl),
    /// A variable declaration (`reg`, `logic`, `int`, `event`, ...).
    Var(VarDecl),
    /// `parameter`, `localparam` or `specparam`.
    Param(ParamDecl),
    /// A non-ANSI port declaration, `input [7:0] a, b;`, also used for
    /// task and function ports declared in the body.
    Port(PortDecl),
    /// `genvar i, j;`.
    Genvar(Vec<Ident>),
    /// `typedef`.
    Typedef(Typedef),
    /// `import pkg::*, pkg::name;`.
    Import(Vec<PackageRef>),
    /// `export pkg::name;` or `export *::*;`.
    Export(Vec<PackageRef>),
    /// A function declaration.
    Function(Subroutine),
    /// A task declaration.
    Task(Subroutine),
    /// `defparam a.b = 1, c = 2;`.
    Defparam(Vec<Defparam>),
    /// A `specify ... endspecify` block, whose contents are skipped.
    Specify(RawTokens),
    /// `initial stmt`.
    Initial(Stmt),
    /// `final stmt`.
    Final(Stmt),
    /// `always`, `always_comb`, `always_ff` or `always_latch`.
    Always(AlwaysKind, Stmt),
    /// A continuous assignment, `assign a = b;`.
    ContAssign(ContAssign),
    /// Gate instantiations, `and g1 (y, a, b);`.
    Gate(GateDecl),
    /// Module, interface or program instantiations.
    Instance(Instantiation),
    /// A `generate ... endgenerate` region.
    Generate(Vec<Item>),
    /// A conditional generate construct.
    GenIf(GenIf),
    /// A `case` generate construct.
    GenCase(GenCase),
    /// A `for` generate loop.
    GenFor(GenFor),
    /// A bare `begin ... end` generate block, accepted for compatibility.
    GenBlock(GenBlock),
    /// `alias a = b = c;`.
    Alias(Vec<Expr>),
    /// A concurrent assertion, `assert property (...)`.
    Assertion(Assertion),
    /// `bind target module inst (...);`.
    Bind(Bind),
    /// A clocking block.
    Clocking(Clocking),
    /// A `property` or `sequence` declaration, kept verbatim.
    PropertyDecl(PropertyDecl),
    /// `modport name (...), name (...);`.
    Modport(Vec<Modport>),
    /// `timeunit 1ns;` or `timeunit 1ns / 1ps;`.
    Timeunit {
        /// The unit literal.
        unit: Literal,
        /// The precision after `/`, when given.
        precision: Option<Literal>,
    },
    /// `timeprecision 1ps;`.
    Timeprecision(Literal),
    /// A compiler directive that reached the parser, such as
    /// `` `default_nettype none ``.
    Directive(Directive),
    /// A UDP `table ... endtable`, one row per entry.
    Table(Vec<RawTokens>),
    /// A lone `;`.
    Empty,
}

/// A `module`, `macromodule`, `interface`, `program` or `primitive`.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Which keyword introduced it.
    pub kind: ModuleKind,
    /// `automatic` or `static` after the keyword.
    pub lifetime: Option<Lifetime>,
    /// The module name.
    pub name: Ident,
    /// `import pkg::*;` clauses between the name and the parameter list.
    pub imports: Vec<PackageRef>,
    /// The `#( ... )` parameter port list; `None` when absent, `Some`
    /// (possibly empty) when written.
    pub params: Option<Vec<ParamDecl>>,
    /// The port list.
    pub ports: Ports,
    /// The body.
    pub items: Vec<Item>,
}

/// The keyword that introduced a [`Module`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    /// `module`.
    Module,
    /// `macromodule`.
    Macromodule,
    /// `interface`.
    Interface,
    /// `program`.
    Program,
    /// `primitive` (a user-defined primitive).
    Primitive,
}

impl ModuleKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            ModuleKind::Module => "module",
            ModuleKind::Macromodule => "macromodule",
            ModuleKind::Interface => "interface",
            ModuleKind::Program => "program",
            ModuleKind::Primitive => "primitive",
        }
    }
}

/// `automatic` or `static`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// `automatic`.
    Automatic,
    /// `static`.
    Static,
}

impl Lifetime {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            Lifetime::Automatic => "automatic",
            Lifetime::Static => "static",
        }
    }
}

/// The port list of a module header.
#[derive(Debug, Clone, PartialEq)]
pub enum Ports {
    /// No parentheses at all: `module m;`.
    None,
    /// Verilog-1995 style: names only, with `input`/`output` declarations
    /// in the body. An empty `()` is an empty ANSI list, not this.
    NonAnsi(Vec<NonAnsiPort>),
    /// ANSI style: each port declared in the header.
    Ansi(Vec<Port>),
}

/// One entry of a non-ANSI port list: `a`, `a[3:0]`, `{a, b}`, `.a(b)`,
/// `.a()` or nothing (`,,`).
#[derive(Debug, Clone, PartialEq)]
pub struct NonAnsiPort {
    /// The external name for the `.name(expr)` form.
    pub name: Option<Ident>,
    /// The port expression; `None` for `.a()` and for an empty entry.
    pub expr: Option<Expr>,
    /// The entry.
    pub span: Span,
}

/// An ANSI-style port of a module, or a port of a task or function.
///
/// Fields absent in the source are `None` / implicit; a port that omits
/// its direction and type inherits them from the previous port, which is
/// elaboration's job.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// Attributes before the port.
    pub attrs: Vec<Attribute>,
    /// `input`, `output`, `inout`, `ref` or `const ref`.
    pub direction: Option<Direction>,
    /// An explicit net type such as `wire`.
    pub net_type: Option<NetType>,
    /// `var` was written.
    pub var: bool,
    /// The data type, implicit when only a range or nothing was given.
    /// Interface ports are a [`DataTypeKind::Named`] with a `member` or a
    /// [`DataTypeKind::Interface`].
    pub data_type: DataType,
    /// The port name.
    pub name: Ident,
    /// Unpacked dimensions after the name.
    pub dims: Vec<Dim>,
    /// `= default` value.
    pub default: Option<Expr>,
    /// The whole port.
    pub span: Span,
}

/// A port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `input`.
    Input,
    /// `output`.
    Output,
    /// `inout`.
    Inout,
    /// `ref` (tasks and functions).
    Ref,
    /// `const ref`.
    ConstRef,
}

impl Direction {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
            Direction::Inout => "inout",
            Direction::Ref => "ref",
            Direction::ConstRef => "const ref",
        }
    }
}

/// `package name; items endpackage`.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// `automatic` or `static` after the keyword.
    pub lifetime: Option<Lifetime>,
    /// The package name.
    pub name: Ident,
    /// The body.
    pub items: Vec<Item>,
}

/// `pkg::name` or `pkg::*` in an import or export.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageRef {
    /// The package; `*` for `export *::*`.
    pub package: Ident,
    /// The imported name; `None` for `*`.
    pub item: Option<Ident>,
    /// The reference.
    pub span: Span,
}

/// A net declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NetDecl {
    /// The net type keyword.
    pub net_type: NetType,
    /// `(strong0, weak1)` or `(small)` after the net type.
    pub strength: Option<Strength>,
    /// `vectored` or `scalared`.
    pub vectored: Option<Vectored>,
    /// The data type, implicit for a plain `wire [7:0]`.
    pub data_type: DataType,
    /// `#delay` after the type.
    pub delay: Option<Delay>,
    /// The declared nets.
    pub decls: Vec<Declarator>,
}

/// The net types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum NetType {
    Wire,
    Tri,
    Tri0,
    Tri1,
    Triand,
    Trior,
    Trireg,
    Wand,
    Wor,
    Supply0,
    Supply1,
    Uwire,
    Interconnect,
}

impl NetType {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            NetType::Wire => "wire",
            NetType::Tri => "tri",
            NetType::Tri0 => "tri0",
            NetType::Tri1 => "tri1",
            NetType::Triand => "triand",
            NetType::Trior => "trior",
            NetType::Trireg => "trireg",
            NetType::Wand => "wand",
            NetType::Wor => "wor",
            NetType::Supply0 => "supply0",
            NetType::Supply1 => "supply1",
            NetType::Uwire => "uwire",
            NetType::Interconnect => "interconnect",
        }
    }
}

/// `vectored` or `scalared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vectored {
    /// `vectored`.
    Vectored,
    /// `scalared`.
    Scalared,
}

/// A drive or charge strength, `(strong0, weak1)`, `(pull1)` or `(large)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Strength {
    /// One or two strength keywords, in source order.
    pub levels: Vec<StrengthLevel>,
    /// Including the parentheses.
    pub span: Span,
}

/// One strength keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum StrengthLevel {
    Supply0,
    Strong0,
    Pull0,
    Weak0,
    Highz0,
    Supply1,
    Strong1,
    Pull1,
    Weak1,
    Highz1,
    Small,
    Medium,
    Large,
}

impl StrengthLevel {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            StrengthLevel::Supply0 => "supply0",
            StrengthLevel::Strong0 => "strong0",
            StrengthLevel::Pull0 => "pull0",
            StrengthLevel::Weak0 => "weak0",
            StrengthLevel::Highz0 => "highz0",
            StrengthLevel::Supply1 => "supply1",
            StrengthLevel::Strong1 => "strong1",
            StrengthLevel::Pull1 => "pull1",
            StrengthLevel::Weak1 => "weak1",
            StrengthLevel::Highz1 => "highz1",
            StrengthLevel::Small => "small",
            StrengthLevel::Medium => "medium",
            StrengthLevel::Large => "large",
        }
    }
}

/// A delay: `#10`, `#(1, 2)`, `#(1:2:3, 4:5:6, 7:8:9)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Delay {
    /// One to three delay values.
    pub values: Vec<Expr>,
    /// From the `#` to the end.
    pub span: Span,
}

/// One declared name with its unpacked dimensions and initialiser.
#[derive(Debug, Clone, PartialEq)]
pub struct Declarator {
    /// The name.
    pub name: Ident,
    /// Unpacked dimensions after the name.
    pub dims: Vec<Dim>,
    /// `= value`, which for a `parameter type` is an [`ExprKind::Type`].
    pub init: Option<Expr>,
    /// The whole declarator.
    pub span: Span,
}

/// A variable declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// `automatic` or `static` prefix.
    pub lifetime: Option<Lifetime>,
    /// `const` was written.
    pub constant: bool,
    /// `var` was written.
    pub var: bool,
    /// The data type; implicit only after `var`.
    pub data_type: DataType,
    /// The declared variables.
    pub decls: Vec<Declarator>,
}

/// A `parameter`, `localparam` or `specparam` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    /// Which keyword.
    pub kind: ParamKind,
    /// A `parameter type T = ...` type parameter.
    pub is_type: bool,
    /// The parameter type, implicit when omitted.
    pub data_type: DataType,
    /// The declared parameters; the `init` of a type parameter is an
    /// [`ExprKind::Type`] or a name.
    pub decls: Vec<Declarator>,
}

/// Which parameter keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// `parameter`.
    Parameter,
    /// `localparam`.
    Localparam,
    /// `specparam`.
    Specparam,
}

impl ParamKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            ParamKind::Parameter => "parameter",
            ParamKind::Localparam => "localparam",
            ParamKind::Specparam => "specparam",
        }
    }
}

/// A non-ANSI port declaration, `output reg [7:0] q;`.
#[derive(Debug, Clone, PartialEq)]
pub struct PortDecl {
    /// The direction.
    pub direction: Direction,
    /// An explicit net type.
    pub net_type: Option<NetType>,
    /// `var` was written.
    pub var: bool,
    /// The data type, implicit for `input [7:0] a`.
    pub data_type: DataType,
    /// The declared ports.
    pub decls: Vec<Declarator>,
}

/// `typedef data_type name dims;` or a forward `typedef enum name;`.
#[derive(Debug, Clone, PartialEq)]
pub struct Typedef {
    /// The new type name.
    pub name: Ident,
    /// The aliased type; `None` for a forward declaration.
    pub data_type: Option<DataType>,
    /// Unpacked dimensions after the name.
    pub dims: Vec<Dim>,
}

/// A function or task.
#[derive(Debug, Clone, PartialEq)]
pub struct Subroutine {
    /// `automatic` or `static`.
    pub lifetime: Option<Lifetime>,
    /// The return type for a function (`void` is [`DataTypeKind::Void`],
    /// omitted is implicit); `None` for a task.
    pub ret: Option<DataType>,
    /// The name.
    pub name: Ident,
    /// ANSI ports; `None` when no parenthesised list was written (ports
    /// then appear as [`ItemKind::Port`] declarations in the body).
    pub ports: Option<Vec<Port>>,
    /// Declarations and statements, in order.
    pub body: Vec<Stmt>,
}

/// One `target = value` of a `defparam`.
#[derive(Debug, Clone, PartialEq)]
pub struct Defparam {
    /// The hierarchical parameter name.
    pub target: Expr,
    /// The new value.
    pub value: Expr,
    /// The assignment.
    pub span: Span,
}

/// Which `always` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysKind {
    /// `always`.
    Always,
    /// `always_comb`.
    Comb,
    /// `always_ff`.
    Ff,
    /// `always_latch`.
    Latch,
}

impl AlwaysKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            AlwaysKind::Always => "always",
            AlwaysKind::Comb => "always_comb",
            AlwaysKind::Ff => "always_ff",
            AlwaysKind::Latch => "always_latch",
        }
    }
}

/// `assign [(strength)] [#delay] lhs = rhs, ...;`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContAssign {
    /// Drive strength.
    pub strength: Option<Strength>,
    /// Delay.
    pub delay: Option<Delay>,
    /// The assignments.
    pub assigns: Vec<AssignPair>,
}

/// One `lhs = rhs`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignPair {
    /// The target.
    pub lhs: Expr,
    /// The value.
    pub rhs: Expr,
    /// The assignment.
    pub span: Span,
}

/// Gate instantiations sharing one gate type.
#[derive(Debug, Clone, PartialEq)]
pub struct GateDecl {
    /// The gate keyword.
    pub kind: GateKind,
    /// Drive strength.
    pub strength: Option<Strength>,
    /// Delay.
    pub delay: Option<Delay>,
    /// The instances.
    pub instances: Vec<GateInstance>,
}

/// The gate and switch primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum GateKind {
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    Buf,
    Not,
    Bufif0,
    Bufif1,
    Notif0,
    Notif1,
    Nmos,
    Pmos,
    Cmos,
    Rnmos,
    Rpmos,
    Rcmos,
    Tran,
    Rtran,
    Tranif0,
    Tranif1,
    Rtranif0,
    Rtranif1,
    Pullup,
    Pulldown,
}

impl GateKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            GateKind::And => "and",
            GateKind::Nand => "nand",
            GateKind::Or => "or",
            GateKind::Nor => "nor",
            GateKind::Xor => "xor",
            GateKind::Xnor => "xnor",
            GateKind::Buf => "buf",
            GateKind::Not => "not",
            GateKind::Bufif0 => "bufif0",
            GateKind::Bufif1 => "bufif1",
            GateKind::Notif0 => "notif0",
            GateKind::Notif1 => "notif1",
            GateKind::Nmos => "nmos",
            GateKind::Pmos => "pmos",
            GateKind::Cmos => "cmos",
            GateKind::Rnmos => "rnmos",
            GateKind::Rpmos => "rpmos",
            GateKind::Rcmos => "rcmos",
            GateKind::Tran => "tran",
            GateKind::Rtran => "rtran",
            GateKind::Tranif0 => "tranif0",
            GateKind::Tranif1 => "tranif1",
            GateKind::Rtranif0 => "rtranif0",
            GateKind::Rtranif1 => "rtranif1",
            GateKind::Pullup => "pullup",
            GateKind::Pulldown => "pulldown",
        }
    }
}

/// One gate instance, `g1 [3:0] (y, a, b)`.
#[derive(Debug, Clone, PartialEq)]
pub struct GateInstance {
    /// The instance name, optional for gates.
    pub name: Option<Ident>,
    /// Instance array dimensions.
    pub dims: Vec<Dim>,
    /// The terminals, in order.
    pub conns: Vec<Expr>,
    /// The instance.
    pub span: Span,
}

/// Instantiations of one module with one parameter override list.
#[derive(Debug, Clone, PartialEq)]
pub struct Instantiation {
    /// The module, interface or program name.
    pub module: Ident,
    /// `#( ... )` parameter overrides, empty when absent.
    pub params: Vec<ParamOverride>,
    /// The instances.
    pub instances: Vec<Instance>,
}

/// One parameter override, positional (`8`) or named (`.W(8)`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamOverride {
    /// The parameter name for the named form.
    pub name: Option<Ident>,
    /// The value; `None` for `.W()`. A type value is an
    /// [`ExprKind::Type`].
    pub value: Option<Expr>,
    /// The override.
    pub span: Span,
}

/// One instance, `u0 [3:0] (.a(x), .b)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// The instance name; `None` only for the non-standard unnamed form.
    pub name: Option<Ident>,
    /// Instance array dimensions.
    pub dims: Vec<Dim>,
    /// Port connections.
    pub conns: Vec<PortConn>,
    /// The instance.
    pub span: Span,
}

/// One port connection.
#[derive(Debug, Clone, PartialEq)]
pub struct PortConn {
    /// The form.
    pub kind: PortConnKind,
    /// The connection.
    pub span: Span,
}

/// The forms of a port connection.
#[derive(Debug, Clone, PartialEq)]
pub enum PortConnKind {
    /// `expr`, or nothing for an empty positional slot.
    Positional(Option<Expr>),
    /// `.name`, `.name()` or `.name(expr)`.
    Named {
        /// The port name.
        name: Ident,
        /// What it connects to.
        conn: NamedConn,
    },
    /// `.*`.
    Wildcard,
}

/// What a named port connection connects to.
#[derive(Debug, Clone, PartialEq)]
pub enum NamedConn {
    /// `.name`: the signal of the same name.
    Implicit,
    /// `.name()`: left unconnected.
    Open,
    /// `.name(expr)`.
    Expr(Expr),
}

/// `if (cond) block [else block]` at item level.
#[derive(Debug, Clone, PartialEq)]
pub struct GenIf {
    /// The condition.
    pub cond: Expr,
    /// The items when true.
    pub then_block: GenBlock,
    /// The items when false.
    pub else_block: Option<GenBlock>,
}

/// `case (expr) ... endcase` at item level.
#[derive(Debug, Clone, PartialEq)]
pub struct GenCase {
    /// The selector.
    pub expr: Expr,
    /// The arms.
    pub items: Vec<GenCaseItem>,
}

/// One arm of a generate `case`.
#[derive(Debug, Clone, PartialEq)]
pub struct GenCaseItem {
    /// The matched values; empty for `default`.
    pub patterns: Vec<Expr>,
    /// The items.
    pub block: GenBlock,
    /// The arm.
    pub span: Span,
}

/// `for (genvar i = 0; i < N; i++) block`.
#[derive(Debug, Clone, PartialEq)]
pub struct GenFor {
    /// `genvar` was written in the initialisation.
    pub genvar: bool,
    /// The loop variable.
    pub var: Ident,
    /// The initial value.
    pub init: Expr,
    /// The condition.
    pub cond: Expr,
    /// The step, an [`ExprKind::Assign`] or [`ExprKind::IncDec`].
    pub step: Expr,
    /// The body.
    pub body: GenBlock,
}

/// A generate block: `begin : label items end`, or a single unbracketed
/// item (then `label` is `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct GenBlock {
    /// `begin : label` or the SystemVerilog `label : begin` form.
    pub label: Option<Ident>,
    /// The items.
    pub items: Vec<Item>,
    /// The block.
    pub span: Span,
}

/// A concurrent (`assert property`) or immediate (`assert (expr)`)
/// assertion with its action block.
#[derive(Debug, Clone, PartialEq)]
pub struct Assertion {
    /// The `label:` before an item-level assertion.
    pub label: Option<Ident>,
    /// `assert`, `assume`, `cover` or `restrict`.
    pub kind: AssertKind,
    /// `#0` or `final` on a deferred immediate assertion.
    pub deferred: Option<Deferred>,
    /// What is asserted.
    pub spec: AssertSpec,
    /// The pass statement.
    pub then_stmt: Option<Box<Stmt>>,
    /// The `else` fail statement.
    pub else_stmt: Option<Box<Stmt>>,
}

/// The assertion keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AssertKind {
    Assert,
    Assume,
    Cover,
    Restrict,
}

impl AssertKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            AssertKind::Assert => "assert",
            AssertKind::Assume => "assume",
            AssertKind::Cover => "cover",
            AssertKind::Restrict => "restrict",
        }
    }
}

/// The deferral of an immediate assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deferred {
    /// `#0`.
    Observed,
    /// `final`.
    Final,
}

/// What an assertion checks.
#[derive(Debug, Clone, PartialEq)]
pub enum AssertSpec {
    /// An immediate boolean expression.
    Expr(Expr),
    /// A `property (...)` or `sequence (...)` specification, verbatim.
    Property(RawTokens),
}

/// `bind target [: inst, inst] module #(...) name (...);`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    /// The target module or instance.
    pub target: Expr,
    /// Instances after `:`.
    pub instances: Vec<Expr>,
    /// What is bound.
    pub inst: Instantiation,
}

/// `[default] clocking [name] @(event); ... endclocking`.
#[derive(Debug, Clone, PartialEq)]
pub struct Clocking {
    /// `default` was written.
    pub is_default: bool,
    /// `global` was written.
    pub is_global: bool,
    /// The block name.
    pub name: Option<Ident>,
    /// The clocking event; `None` for a `default clocking name;`
    /// reference to a block declared elsewhere.
    pub event: Option<EventControl>,
    /// The body, verbatim (empty for a reference).
    pub body: RawTokens,
}

/// A `property` or `sequence` declaration, verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDecl {
    /// `sequence` rather than `property`.
    pub is_sequence: bool,
    /// The name.
    pub name: Ident,
    /// Everything between the name and the end keyword.
    pub body: RawTokens,
}

/// One `modport name (...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Modport {
    /// The modport name.
    pub name: Ident,
    /// The entries.
    pub items: Vec<ModportItem>,
    /// The modport.
    pub span: Span,
}

/// One entry of a modport.
#[derive(Debug, Clone, PartialEq)]
pub struct ModportItem {
    /// The entry.
    pub kind: ModportItemKind,
    /// Its extent.
    pub span: Span,
}

/// The kinds of modport entry.
#[derive(Debug, Clone, PartialEq)]
pub enum ModportItemKind {
    /// `input a` or `output .a(expr)`; the direction is the most recent
    /// one written.
    Port {
        /// The direction.
        direction: Direction,
        /// The port name.
        name: Ident,
        /// The `.name(expr)` expression.
        expr: Option<Expr>,
    },
    /// `import name`.
    Import(Ident),
    /// `export name`.
    Export(Ident),
    /// `clocking name`.
    Clocking(Ident),
}

/// A compiler directive passed through to the parser.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// The directive name without the backtick: `default_nettype`,
    /// `timescale`, `resetall`.
    pub name: String,
    /// The argument tokens' text, space separated.
    pub args: String,
    /// From the backtick to the end of the arguments.
    pub span: Span,
}

// --- data types --------------------------------------------------------

/// A data type: a kind plus the signing and packed dimensions that the
/// integer kinds and the implicit type accept.
#[derive(Debug, Clone, PartialEq)]
pub struct DataType {
    /// The kind.
    pub kind: DataTypeKind,
    /// `signed` or `unsigned`.
    pub signing: Option<Signing>,
    /// Packed dimensions, `[7:0][3:0]`.
    pub packed: Vec<Dim>,
    /// The type; empty for an implicit type with nothing written.
    pub span: Span,
}

impl DataType {
    /// The implicit type with nothing written, positioned at `span`.
    pub fn implicit(span: Span) -> Self {
        DataType {
            kind: DataTypeKind::Implicit,
            signing: None,
            packed: Vec::new(),
            span,
        }
    }

    /// True when nothing at all was written (kind implicit, no signing, no
    /// dimensions).
    pub fn is_empty(&self) -> bool {
        self.kind == DataTypeKind::Implicit && self.signing.is_none() && self.packed.is_empty()
    }
}

/// `signed` or `unsigned`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signing {
    /// `signed`.
    Signed,
    /// `unsigned`.
    Unsigned,
}

/// The kinds of [`DataType`].
#[derive(Debug, Clone, PartialEq)]
pub enum DataTypeKind {
    /// No type keyword: `wire [7:0]`, `input a`, `parameter P`.
    Implicit,
    /// An integer type keyword.
    Integer(IntegerType),
    /// `real`, `shortreal` or `realtime`.
    Real(RealType),
    /// `string`.
    String,
    /// `chandle`.
    Chandle,
    /// `event`.
    Event,
    /// `void` (function return type only).
    Void,
    /// `enum [base] { ... }`.
    Enum(EnumType),
    /// `struct` or `union`.
    Struct(StructType),
    /// A type by name: `T`, `pkg::T`, or an interface port type
    /// `bus_if.master`.
    Named {
        /// The `pkg::` qualifier.
        package: Option<Ident>,
        /// The type name.
        name: Ident,
        /// The `.modport` of an interface port type.
        member: Option<Ident>,
    },
    /// The generic `interface [.modport]` port type.
    Interface {
        /// The modport.
        modport: Option<Ident>,
    },
    /// `type(expr)` or `type(data_type)` (the latter as an
    /// [`ExprKind::Type`]).
    TypeOf(Box<Expr>),
}

/// The integer type keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum IntegerType {
    Bit,
    Logic,
    Reg,
    Byte,
    Shortint,
    Int,
    Longint,
    Integer,
    Time,
}

impl IntegerType {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            IntegerType::Bit => "bit",
            IntegerType::Logic => "logic",
            IntegerType::Reg => "reg",
            IntegerType::Byte => "byte",
            IntegerType::Shortint => "shortint",
            IntegerType::Int => "int",
            IntegerType::Longint => "longint",
            IntegerType::Integer => "integer",
            IntegerType::Time => "time",
        }
    }
}

/// The real type keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum RealType {
    Real,
    Shortreal,
    Realtime,
}

impl RealType {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            RealType::Real => "real",
            RealType::Shortreal => "shortreal",
            RealType::Realtime => "realtime",
        }
    }
}

/// `enum [base_type] { variants }`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumType {
    /// The base type, when written.
    pub base: Option<Box<DataType>>,
    /// The variants.
    pub variants: Vec<EnumVariant>,
}

/// One enum variant, `A`, `A = 1`, `A[4]` or `A[1:3] = 5`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    /// The name.
    pub name: Ident,
    /// The `[N]` or `[lo:hi]` generator range.
    pub range: Option<Dim>,
    /// The explicit value.
    pub value: Option<Expr>,
    /// The variant.
    pub span: Span,
}

/// `struct` or `union`, `[packed] [tagged] [signing] { members }`.
#[derive(Debug, Clone, PartialEq)]
pub struct StructType {
    /// `union` rather than `struct`.
    pub is_union: bool,
    /// `packed` was written.
    pub packed: bool,
    /// `tagged` was written (unions).
    pub tagged: bool,
    /// The members.
    pub members: Vec<StructMember>,
}

/// One member declaration of a struct or union.
#[derive(Debug, Clone, PartialEq)]
pub struct StructMember {
    /// The member type.
    pub data_type: DataType,
    /// The names declared with that type.
    pub decls: Vec<Declarator>,
    /// The declaration.
    pub span: Span,
}

/// A packed or unpacked dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct Dim {
    /// The form.
    pub kind: DimKind,
    /// Including the brackets.
    pub span: Span,
}

/// The forms of a dimension.
#[derive(Debug, Clone, PartialEq)]
pub enum DimKind {
    /// `[msb:lsb]`.
    Range(Expr, Expr),
    /// `[N]`.
    Size(Expr),
    /// `[]`, a dynamic array.
    Unsized,
    /// `[$]` or `[$:N]`, a queue.
    Queue(Option<Expr>),
    /// `[*]` or `[type]`, an associative array.
    Assoc(Option<DataType>),
}

// --- statements ----------------------------------------------------------

/// A procedural statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    /// A `label:` before the statement.
    pub label: Option<Ident>,
    /// Attributes before the statement.
    pub attrs: Vec<Attribute>,
    /// What the statement is.
    pub kind: StmtKind,
    /// From the label or first keyword to the closing `;` or `end`.
    pub span: Span,
}

/// The kinds of [`Stmt`].
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// A lone `;`.
    Null,
    /// `begin ... end`.
    Block(Block),
    /// `fork ... join`, `join_any` or `join_none`.
    Fork(Block, JoinKind),
    /// A blocking, non-blocking or compound assignment.
    Assign(Box<Assign>),
    /// An expression used as a statement: a call, `++`/`--`, a `void'()`
    /// cast.
    Expr(Expr),
    /// `if`.
    If(If),
    /// `case`, `casez`, `casex`.
    Case(Case),
    /// `for`.
    For(For),
    /// `while (cond) body`.
    While(Expr, Box<Stmt>),
    /// `do body while (cond);`.
    DoWhile(Box<Stmt>, Expr),
    /// `repeat (count) body`.
    Repeat(Expr, Box<Stmt>),
    /// `forever body`.
    Forever(Box<Stmt>),
    /// `foreach (array[i, j]) body`.
    Foreach(Foreach),
    /// `break;`.
    Break,
    /// `continue;`.
    Continue,
    /// `return [expr];`.
    Return(Option<Expr>),
    /// `disable name;`.
    Disable(Expr),
    /// `disable fork;`.
    DisableFork,
    /// A delay or event control followed by a statement, `#10 x = 1;`,
    /// `@(posedge clk);`.
    Timing(TimingControl, Box<Stmt>),
    /// `wait (cond) body`.
    Wait(Expr, Box<Stmt>),
    /// `wait fork;`.
    WaitFork,
    /// Procedural continuous `assign lhs = rhs;`.
    ProcAssign(Expr, Expr),
    /// `deassign lhs;`.
    Deassign(Expr),
    /// `force lhs = rhs;`.
    Force(Expr, Expr),
    /// `release lhs;`.
    Release(Expr),
    /// `-> event;` or `->> event;`.
    Trigger {
        /// `->>` (non-blocking) rather than `->`.
        nonblocking: bool,
        /// The event.
        target: Expr,
    },
    /// An immediate assertion, or a concurrent one inside a procedure.
    Assert(Assertion),
    /// A declaration inside a block or subroutine body.
    Decl(Box<Item>),
}

/// A `begin ... end` or `fork ... join` body.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The `begin : name` label.
    pub label: Option<Ident>,
    /// The statements, including declarations.
    pub stmts: Vec<Stmt>,
    /// From `begin`/`fork` to `end`/`join`.
    pub span: Span,
}

/// How a `fork` ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// `join`.
    All,
    /// `join_any`.
    Any,
    /// `join_none`.
    None,
}

impl JoinKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            JoinKind::All => "join",
            JoinKind::Any => "join_any",
            JoinKind::None => "join_none",
        }
    }
}

/// `lhs op [timing] rhs`.
#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
    /// The target.
    pub lhs: Expr,
    /// `=`, `<=` or a compound operator.
    pub op: AssignOp,
    /// An intra-assignment delay or event control.
    pub timing: Option<TimingControl>,
    /// The value.
    pub rhs: Expr,
}

/// The assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`.
    Blocking,
    /// `<=`.
    NonBlocking,
    /// `+=`.
    Add,
    /// `-=`.
    Sub,
    /// `*=`.
    Mul,
    /// `/=`.
    Div,
    /// `%=`.
    Mod,
    /// `&=`.
    And,
    /// `|=`.
    Or,
    /// `^=`.
    Xor,
    /// `<<=`.
    Shl,
    /// `>>=`.
    Shr,
    /// `<<<=`.
    Ashl,
    /// `>>>=`.
    Ashr,
}

impl AssignOp {
    /// The operator text.
    pub fn as_str(self) -> &'static str {
        match self {
            AssignOp::Blocking => "=",
            AssignOp::NonBlocking => "<=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Mod => "%=",
            AssignOp::And => "&=",
            AssignOp::Or => "|=",
            AssignOp::Xor => "^=",
            AssignOp::Shl => "<<=",
            AssignOp::Shr => ">>=",
            AssignOp::Ashl => "<<<=",
            AssignOp::Ashr => ">>>=",
        }
    }
}

/// `if (cond) then [else other]`, with an optional qualifier.
#[derive(Debug, Clone, PartialEq)]
pub struct If {
    /// `unique`, `unique0` or `priority`.
    pub qualifier: Option<Qualifier>,
    /// The condition.
    pub cond: Expr,
    /// The statement when true.
    pub then_stmt: Box<Stmt>,
    /// The statement when false.
    pub else_stmt: Option<Box<Stmt>>,
}

/// `unique`, `unique0` or `priority` on an `if` or `case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Qualifier {
    Unique,
    Unique0,
    Priority,
}

impl Qualifier {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            Qualifier::Unique => "unique",
            Qualifier::Unique0 => "unique0",
            Qualifier::Priority => "priority",
        }
    }
}

/// `case (expr) items endcase`.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// `unique`, `unique0` or `priority`.
    pub qualifier: Option<Qualifier>,
    /// `case`, `casez` or `casex`.
    pub kind: CaseKind,
    /// `case (expr) inside`.
    pub inside: bool,
    /// The selector.
    pub expr: Expr,
    /// The arms.
    pub items: Vec<CaseItem>,
}

/// `case`, `casez` or `casex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CaseKind {
    Case,
    Casez,
    Casex,
}

impl CaseKind {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            CaseKind::Case => "case",
            CaseKind::Casez => "casez",
            CaseKind::Casex => "casex",
        }
    }
}

/// One arm of a `case`.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseItem {
    /// The matched values; empty for `default`. In `case inside` a value
    /// may be an [`ExprKind::ValueRange`].
    pub patterns: Vec<Expr>,
    /// The statement.
    pub body: Stmt,
    /// The arm.
    pub span: Span,
}

/// `for (init; cond; step) body`.
#[derive(Debug, Clone, PartialEq)]
pub struct For {
    /// The initialisations.
    pub init: Vec<ForInit>,
    /// The condition, absent in `for (;;)`.
    pub cond: Option<Expr>,
    /// The step expressions ([`ExprKind::Assign`], [`ExprKind::IncDec`] or
    /// calls).
    pub step: Vec<Expr>,
    /// The body.
    pub body: Box<Stmt>,
}

/// One initialisation of a `for`.
#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    /// `int i = 0, j = 0`.
    Decl(VarDecl),
    /// `i = 0`, an [`ExprKind::Assign`].
    Assign(Expr),
}

/// `foreach (array[i, j]) body`.
#[derive(Debug, Clone, PartialEq)]
pub struct Foreach {
    /// The array.
    pub array: Expr,
    /// The loop variables, one per dimension; `None` for a skipped one.
    pub vars: Vec<Option<Ident>>,
    /// The body.
    pub body: Box<Stmt>,
}

/// A delay or event control.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingControl {
    /// The form.
    pub kind: TimingKind,
    /// The control.
    pub span: Span,
}

/// The forms of [`TimingControl`].
#[derive(Debug, Clone, PartialEq)]
pub enum TimingKind {
    /// `#delay`.
    Delay(Delay),
    /// `@event`.
    Event(EventControl),
    /// `repeat (count) @event` (intra-assignment only).
    RepeatEvent(Expr, EventControl),
}

/// `@*`, `@(*)`, `@(posedge clk, negedge rst)`, `@name`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventControl {
    /// The form.
    pub kind: EventControlKind,
    /// From the `@` to the end.
    pub span: Span,
}

/// The forms of [`EventControl`].
#[derive(Debug, Clone, PartialEq)]
pub enum EventControlKind {
    /// `@*` or `@(*)`.
    Any,
    /// A list of event expressions separated by `or` or `,`.
    List(Vec<EventExpr>),
}

/// One `[posedge|negedge|edge] expr [iff cond]`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventExpr {
    /// The edge.
    pub edge: Option<Edge>,
    /// The signal or expression.
    pub expr: Expr,
    /// The `iff` condition.
    pub iff: Option<Expr>,
    /// The event expression.
    pub span: Span,
}

/// An edge keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Edge {
    Posedge,
    Negedge,
    Edge,
}

impl Edge {
    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Posedge => "posedge",
            Edge::Negedge => "negedge",
            Edge::Edge => "edge",
        }
    }
}

// --- expressions ---------------------------------------------------------

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// The kind.
    pub kind: ExprKind,
    /// The expression.
    pub span: Span,
}

impl Expr {
    /// Builds an expression.
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }
}

/// A literal.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Any numeric literal, kept as written: `8'hff`, `42`, `1.5e3`,
    /// `10ns`, `'0`, `'x`.
    Number {
        /// The literal text.
        text: String,
        /// The literal.
        span: Span,
    },
    /// A string literal, with escapes decoded.
    Str {
        /// The decoded value.
        value: String,
        /// The literal including quotes.
        span: Span,
    },
    /// `null`.
    Null(Span),
    /// `$` as an unbounded index or queue bound.
    Unbounded(Span),
}

impl Literal {
    /// The literal's span.
    pub fn span(&self) -> Span {
        match self {
            Literal::Number { span, .. } | Literal::Str { span, .. } => *span,
            Literal::Null(span) | Literal::Unbounded(span) => *span,
        }
    }
}

/// The kinds of [`Expr`].
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Literal(Literal),
    /// A simple name.
    Ident(Ident),
    /// A `$name`: a system function or task, or `$root` / `$unit`. The
    /// name is stored without the `$`.
    SystemIdent(Ident),
    /// `scope::name`.
    Scoped {
        /// The package or scope.
        scope: Box<Expr>,
        /// The member.
        name: Ident,
    },
    /// `base.name`: a struct member, an interface signal or a hierarchical
    /// path component.
    Member {
        /// The left side.
        base: Box<Expr>,
        /// The member.
        name: Ident,
    },
    /// `base[index]`.
    Index {
        /// The indexed value.
        base: Box<Expr>,
        /// The index.
        index: Box<Expr>,
    },
    /// `base[a:b]`, `base[a+:b]`, `base[a-:b]`.
    Range {
        /// The selected value.
        base: Box<Expr>,
        /// Which form.
        kind: RangeKind,
        /// The left bound (or base index).
        left: Box<Expr>,
        /// The right bound (or width).
        right: Box<Expr>,
    },
    /// A prefix operator.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
    },
    /// A binary operator.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: Box<Expr>,
        /// The right operand.
        rhs: Box<Expr>,
    },
    /// `cond ? a : b`.
    Ternary {
        /// The condition.
        cond: Box<Expr>,
        /// The value when true.
        then_expr: Box<Expr>,
        /// The value when false.
        else_expr: Box<Expr>,
    },
    /// `{a, b, c}`.
    Concat(Vec<Expr>),
    /// `{count{a, b}}`.
    Replicate {
        /// The repetition count.
        count: Box<Expr>,
        /// The repeated elements.
        elems: Vec<Expr>,
    },
    /// `{<< [slice] {a, b}}` or `{>> ...}`.
    Streaming {
        /// `<<` (right-to-left) rather than `>>`.
        right_to_left: bool,
        /// The slice size, an expression or [`ExprKind::Type`].
        slice: Option<Box<Expr>>,
        /// The streamed elements.
        elems: Vec<Expr>,
    },
    /// `'{ ... }`, an assignment pattern.
    Pattern(Vec<PatternItem>),
    /// A function, task, method or system call.
    Call {
        /// What is called.
        callee: Box<Expr>,
        /// The arguments.
        args: Vec<Arg>,
    },
    /// `new`, `new(args)` or `new[size]`.
    New(Vec<Expr>),
    /// `target'(expr)` or `type'{pattern}`.
    Cast {
        /// The cast target.
        target: CastTarget,
        /// The value.
        expr: Box<Expr>,
    },
    /// `expr inside { values }`.
    Inside {
        /// The tested value.
        expr: Box<Expr>,
        /// The set; ranges are [`ExprKind::ValueRange`].
        set: Vec<Expr>,
    },
    /// `[low:high]` in an `inside` set or `case inside` pattern.
    ValueRange {
        /// The low bound.
        low: Box<Expr>,
        /// The high bound.
        high: Box<Expr>,
    },
    /// `min:typ:max`.
    MinTypMax {
        /// Minimum.
        min: Box<Expr>,
        /// Typical.
        typ: Box<Expr>,
        /// Maximum.
        max: Box<Expr>,
    },
    /// A data type in expression position: `$bits(int)`, `.T(logic)`,
    /// a streaming slice size.
    Type(Box<DataType>),
    /// `lhs op rhs` as an expression (`for` steps, and SystemVerilog's
    /// assignment expressions).
    Assign {
        /// The target.
        lhs: Box<Expr>,
        /// The operator.
        op: AssignOp,
        /// The value.
        rhs: Box<Expr>,
    },
    /// `++x`, `x++`, `--x`, `x--`.
    IncDec {
        /// `++` (true) or `--`.
        increment: bool,
        /// Prefix (true) or postfix.
        prefix: bool,
        /// The variable.
        target: Box<Expr>,
    },
    /// `default` as an assignment-pattern key.
    Default,
}

/// Which part-select form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeKind {
    /// `[a:b]`.
    Fixed,
    /// `[a+:b]`.
    IndexedUp,
    /// `[a-:b]`.
    IndexedDown,
}

impl RangeKind {
    /// The separator text: `:`, `+:` or `-:`.
    pub fn as_str(self) -> &'static str {
        match self {
            RangeKind::Fixed => ":",
            RangeKind::IndexedUp => "+:",
            RangeKind::IndexedDown => "-:",
        }
    }
}

/// The prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `!`.
    LogicNot,
    /// `~`.
    BitNot,
    /// `&` reduction.
    ReduceAnd,
    /// `~&` reduction.
    ReduceNand,
    /// `|` reduction.
    ReduceOr,
    /// `~|` reduction.
    ReduceNor,
    /// `^` reduction.
    ReduceXor,
    /// `~^` or `^~` reduction.
    ReduceXnor,
}

impl UnaryOp {
    /// The operator text.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Minus => "-",
            UnaryOp::LogicNot => "!",
            UnaryOp::BitNot => "~",
            UnaryOp::ReduceAnd => "&",
            UnaryOp::ReduceNand => "~&",
            UnaryOp::ReduceOr => "|",
            UnaryOp::ReduceNor => "~|",
            UnaryOp::ReduceXor => "^",
            UnaryOp::ReduceXnor => "~^",
        }
    }
}

/// The binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Mod,
    /// `**`.
    Pow,
    /// `<<`.
    Shl,
    /// `>>`.
    Shr,
    /// `<<<`.
    Ashl,
    /// `>>>`.
    Ashr,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `===`.
    CaseEq,
    /// `!==`.
    CaseNe,
    /// `==?`.
    WildEq,
    /// `!=?`.
    WildNe,
    /// `&`.
    BitAnd,
    /// `|`.
    BitOr,
    /// `^`.
    BitXor,
    /// `~^` or `^~`.
    BitXnor,
    /// `&&`.
    LogicAnd,
    /// `||`.
    LogicOr,
    /// `->`.
    Implies,
    /// `<->`.
    Equiv,
}

impl BinaryOp {
    /// The operator text.
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Pow => "**",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
            BinaryOp::Ashl => "<<<",
            BinaryOp::Ashr => ">>>",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::CaseEq => "===",
            BinaryOp::CaseNe => "!==",
            BinaryOp::WildEq => "==?",
            BinaryOp::WildNe => "!=?",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "^",
            BinaryOp::BitXnor => "~^",
            BinaryOp::LogicAnd => "&&",
            BinaryOp::LogicOr => "||",
            BinaryOp::Implies => "->",
            BinaryOp::Equiv => "<->",
        }
    }
}

/// One item of an assignment pattern: `value`, `key: value`,
/// `default: value` or `type: value`.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternItem {
    /// The key: an expression, an [`ExprKind::Type`] or
    /// [`ExprKind::Default`].
    pub key: Option<Expr>,
    /// The value.
    pub value: Expr,
    /// The item.
    pub span: Span,
}

/// One call argument: positional (possibly empty) or `.name(value)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// The parameter name for the named form.
    pub name: Option<Ident>,
    /// The value; `None` for an empty positional slot or `.name()`.
    pub value: Option<Expr>,
    /// The argument.
    pub span: Span,
}

/// The target of a `'()` cast.
#[derive(Debug, Clone, PartialEq)]
pub enum CastTarget {
    /// A type, including `void`, `string` and named types.
    Type(Box<DataType>),
    /// A width, `8'(x)` or `(W+1)'(x)`.
    Size(Box<Expr>),
    /// `signed'(x)` or `unsigned'(x)`.
    Signing(Signing),
    /// `const'(x)`.
    Const,
}
