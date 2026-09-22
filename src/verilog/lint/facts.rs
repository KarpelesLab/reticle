//! Facts about a parsed file, collected in one traversal for the rules to
//! query.
//!
//! The linter does not elaborate: there is no hierarchy, no parameter
//! overriding and no type checking. What it has instead is a light-weight
//! per-module symbol table built here in one pass over the AST:
//!
//! - every declaration ([`Decl`]) with its kind, type, initialiser and
//!   attributes, in a flat table indexed by [`DeclId`], plus the module
//!   scope map from name to id (statement-local declarations are in the
//!   table too, flagged `local`, and resolved through a scope stack during
//!   the traversal);
//! - every write ([`Write`]) to a declared name with the assignment kind
//!   and the process it happens in, and every read ([`Read`]);
//! - every process ([`Proc`]): `always` blocks classified as sequential,
//!   combinational or latch from their keyword or sensitivity list,
//!   `initial` / `final` blocks, continuous assignments, instances, gates
//!   and subroutine bodies, each with the generate arms it sits in so two
//!   drivers in exclusive `if`/`case` generate branches are not reported
//!   as conflicting;
//! - every instance with its connections and, when the instantiated module
//!   is in the same file, the direction of each connected port;
//! - identifiers used without a declaration where Verilog would create an
//!   implicit net.
//!
//! Rules read these tables and, when they need structure (case coverage,
//! reachability), walk the statements of a [`Proc`] themselves. Each rule
//! is therefore linear in the size of the tree.

use std::collections::{HashMap, HashSet};

use crate::source::Span;
use crate::verilog::ast::{
    AlwaysKind, Arg, Assertion, Attribute, DataType, DataTypeKind, Declarator, Dim, Direction,
    EventControl, EventControlKind, Expr, ExprKind, ForInit, GateKind, Ident, Item, ItemKind,
    Literal, Module, ModuleKind, NamedConn, NetType, ParamKind, Port, PortConnKind, Ports,
    SourceFile, Stmt, StmtKind, Subroutine, TimingControl, TimingKind,
};

/// Index of a [`Decl`] in [`ModuleFacts::decls`].
pub type DeclId = usize;

/// Index of a [`Proc`] in [`ModuleFacts::procs`].
pub type ProcId = usize;

/// What a declared name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    /// A net (`wire`, `tri`, ...).
    Net(NetType),
    /// A variable (`reg`, `logic`, `int`, ...).
    Var,
    /// A module port, ANSI or non-ANSI.
    Port(PortInfo),
    /// `parameter`, `localparam` or `specparam`.
    Param(ParamKind),
    /// A `genvar`, or the variable of a `for (genvar i ...)` loop.
    Genvar,
    /// A `typedef`.
    Typedef,
    /// A function.
    Function,
    /// A task.
    Task,
    /// An instance name.
    Instance,
    /// A variant of an `enum` type declared in this module.
    EnumVariant,
    /// A port of a task or function.
    SubPort(Direction),
    /// A `foreach` loop variable.
    LoopVar,
}

/// The details of a port declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// The direction; `inout` for interface ports.
    pub direction: Direction,
    /// The explicit net type, when written.
    pub net_type: Option<NetType>,
    /// The port is a variable (`output reg`, `output logic`, `var`).
    pub is_var: bool,
    /// The port is an interface instance rather than a signal.
    pub interface: bool,
}

/// One declared name.
#[derive(Clone, Debug)]
pub struct Decl<'a> {
    /// The name.
    pub name: String,
    /// The name's span.
    pub span: Span,
    /// What it is.
    pub kind: DeclKind,
    /// The declared type, when the declaration has one.
    pub data_type: Option<&'a DataType>,
    /// Unpacked dimensions after the name.
    pub unpacked: &'a [Dim],
    /// The initialiser or default value.
    pub init: Option<&'a Expr>,
    /// Attributes on the declaration (or on its item).
    pub attrs: &'a [Attribute],
    /// Declared inside a block, loop or subroutine rather than at module
    /// level.
    pub local: bool,
    /// Indices into [`ModuleFacts::reads`].
    pub reads: Vec<usize>,
    /// Indices into [`ModuleFacts::writes`].
    pub writes: Vec<usize>,
    /// The variants when the declared type is an inline `enum`.
    pub variants: Option<Vec<String>>,
}

impl Decl<'_> {
    /// True for nets, variables and ports: the things that carry a value.
    pub fn is_signal(&self) -> bool {
        matches!(
            self.kind,
            DeclKind::Net(_) | DeclKind::Var | DeclKind::Port(_)
        )
    }

    /// True when an attribute of that name is present.
    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|a| a.name.name == name)
    }

    /// The port direction, for ports.
    pub fn direction(&self) -> Option<Direction> {
        match self.kind {
            DeclKind::Port(p) => Some(p.direction),
            _ => None,
        }
    }

    /// The net type for nets and net-typed ports (`wire` when a port has
    /// none written and is not a variable).
    pub fn net_type(&self) -> Option<NetType> {
        match self.kind {
            DeclKind::Net(t) => Some(t),
            DeclKind::Port(p) if !p.is_var => Some(p.net_type.unwrap_or(NetType::Wire)),
            _ => None,
        }
    }
}

/// How a name is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteKind {
    /// `=` or a compound operator in a procedure.
    Blocking,
    /// `<=` in a procedure.
    NonBlocking,
    /// A continuous assignment (`assign`, or a net initialiser).
    Continuous,
    /// A variable initialiser (`reg x = 0`).
    Init,
    /// `force`, `release`, procedural `assign` / `deassign`, event trigger.
    Procedural,
    /// Connected to an output port of an instance.
    InstanceOutput,
    /// Connected to an `inout` port, or to a port whose direction is not
    /// known because the module is not in this file.
    InstanceInout,
    /// The output terminal of a gate.
    Gate,
    /// Passed as an argument to a task call, which may write it.
    CallArg,
}

impl WriteKind {
    /// True for the kinds that drive a value in hardware terms and so count
    /// for `multiple-drivers`.
    pub fn is_driver(self) -> bool {
        matches!(
            self,
            WriteKind::Blocking
                | WriteKind::NonBlocking
                | WriteKind::Continuous
                | WriteKind::InstanceOutput
                | WriteKind::Gate
        )
    }
}

/// One write to a declared name.
#[derive(Clone, Copy, Debug)]
pub struct Write<'a> {
    /// The written name.
    pub decl: DeclId,
    /// The target expression's span.
    pub span: Span,
    /// The kind of write.
    pub kind: WriteKind,
    /// The process the write happens in.
    pub proc: ProcId,
    /// True when the whole name is written (`x = ...`), false for a bit,
    /// part, element or member (`x[3] = ...`, `x.f = ...`).
    pub whole: bool,
    /// The target expression; `None` for an implicit `.name` connection.
    pub lhs: Option<&'a Expr>,
}

/// One read of a declared name.
#[derive(Clone, Copy, Debug)]
pub struct Read {
    /// The read name.
    pub decl: DeclId,
    /// Where.
    pub span: Span,
    /// The process the read happens in.
    pub proc: ProcId,
}

/// What a process is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcKind {
    /// An `always` block of any flavour.
    Always(AlwaysKind),
    /// `initial`.
    Initial,
    /// `final`.
    Final,
    /// One `assign lhs = rhs`.
    ContAssign,
    /// A net initialiser, `wire x = ...`.
    NetInit,
    /// A variable initialiser, `reg x = ...`.
    VarInit,
    /// A module instance.
    Instance,
    /// A gate instance.
    Gate,
    /// A function body.
    Function,
    /// A task body.
    Task,
    /// Anything else that reads or writes signals (assertions, aliases).
    Other,
}

/// How an `always` block was classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcClass {
    /// `always_ff`, or `always` with an edge in its sensitivity list.
    Sequential,
    /// `always_comb`, `always @*`, or `always` with a level-sensitive list.
    Combinational,
    /// `always_latch`.
    Latch,
    /// An `always` without an event control at the top (`always #5 ...`).
    Untimed,
    /// Not an `always` block.
    NotAlways,
}

/// One process.
#[derive(Clone, Debug)]
pub struct Proc<'a> {
    /// What it is.
    pub kind: ProcKind,
    /// The classification of an `always` block.
    pub class: ProcClass,
    /// The item's span.
    pub span: Span,
    /// The sensitivity list of an `always` / `always_ff` block.
    pub sens: Option<&'a EventControl>,
    /// The statements: one for a block, the body for a subroutine, none
    /// for an assignment or instance.
    pub stmts: Vec<&'a Stmt>,
    /// The generate arms enclosing the process, as `(construct, arm)`
    /// pairs; two processes are mutually exclusive when they share a
    /// construct with different arms.
    pub gen_arms: Vec<(u32, u32)>,
}

impl<'a> Proc<'a> {
    /// The body of an `always` block with its top-level timing controls
    /// stripped, for rules that look at the first statement.
    pub fn body(&self) -> Option<&'a Stmt> {
        let mut s = *self.stmts.first()?;
        while let StmtKind::Timing(_, inner) = &s.kind {
            s = inner;
        }
        Some(s)
    }

    /// True when `self` and `other` sit in different arms of one generate
    /// `if` or `case`, so only one of them exists in any elaboration.
    pub fn exclusive_with(&self, other: &Proc<'_>) -> bool {
        self.gen_arms
            .iter()
            .any(|&(c, a)| other.gen_arms.iter().any(|&(oc, oa)| oc == c && oa != a))
    }
}

/// One module instance.
#[derive(Clone, Debug)]
pub struct Instance<'a> {
    /// The instantiated module's name.
    pub module: &'a Ident,
    /// The instance name.
    pub name: Option<&'a Ident>,
    /// The instance's span.
    pub span: Span,
    /// The port connections, in source order (`.*` excluded).
    pub conns: Vec<Conn<'a>>,
    /// `.*` was used.
    pub wildcard: bool,
    /// The instantiated module is defined in this file.
    pub known: bool,
    /// The process id of the instance.
    pub proc: ProcId,
}

/// One port connection of an instance.
#[derive(Clone, Copy, Debug)]
pub struct Conn<'a> {
    /// The port name for named connections.
    pub name: Option<&'a Ident>,
    /// The connected expression; `None` for `.x()`, an empty positional
    /// slot or an implicit `.x`.
    pub expr: Option<&'a Expr>,
    /// The port's direction when the module is known.
    pub direction: Option<Direction>,
    /// Positional rather than named.
    pub positional: bool,
    /// Explicitly left open: `.x()` or an empty positional slot.
    pub open: bool,
    /// The connection's span.
    pub span: Span,
}

/// Where an undeclared identifier was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseContext {
    /// In a port connection or gate terminal.
    PortConn,
    /// As the target of a continuous assignment.
    AssignLhs,
}

/// An identifier used where Verilog creates an implicit net, without a
/// declaration in scope.
#[derive(Clone, Debug)]
pub struct Undeclared {
    /// The name and where it was used.
    pub ident: Ident,
    /// The context.
    pub context: UseContext,
}

/// A `(* lint_off *)` or `(* lint_off = "a, b" *)` attribute and the
/// extent of the item or statement it decorates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintOff {
    /// The decorated item's span.
    pub span: Span,
    /// The named lints, or `None` for all.
    pub names: Option<Vec<String>>,
}

/// The facts of one module, interface, program or primitive.
#[derive(Clone, Debug)]
pub struct ModuleFacts<'a> {
    /// The module.
    pub module: &'a Module,
    /// The module item's span.
    pub span: Span,
    /// Every declaration, module-level and local.
    pub decls: Vec<Decl<'a>>,
    /// Module-level names.
    pub scope: HashMap<String, DeclId>,
    /// The ports, in declaration order.
    pub ports: Vec<DeclId>,
    /// Every write.
    pub writes: Vec<Write<'a>>,
    /// Every read.
    pub reads: Vec<Read>,
    /// Every process.
    pub procs: Vec<Proc<'a>>,
    /// Every instance.
    pub instances: Vec<Instance<'a>>,
    /// Undeclared identifiers in implicit-net positions, in source order
    /// (a name may appear more than once).
    pub undeclared: Vec<Undeclared>,
    /// The module (or the compilation unit) has an `import pkg::*`, so
    /// unresolved names may come from the package.
    pub wildcard_import: bool,
    /// `` `default_nettype none `` was in effect where the module starts.
    pub default_nettype_none: bool,
    /// `typedef`s declared in the module.
    pub typedefs: HashMap<String, &'a DataType>,
    /// The variants of each `typedef enum` declared in the module.
    pub enums: HashMap<String, Vec<String>>,
}

impl<'a> ModuleFacts<'a> {
    /// The module name.
    pub fn name(&self) -> &'a str {
        &self.module.name.name
    }

    /// The module-level declaration of `name`.
    pub fn lookup(&self, name: &str) -> Option<DeclId> {
        self.scope.get(name).copied()
    }

    /// The declaration with that id.
    pub fn decl(&self, id: DeclId) -> &Decl<'a> {
        &self.decls[id]
    }

    /// Resolves an expression to the module-level declaration it names,
    /// through indexing, part-selects and member accesses.
    pub fn root_decl(&self, expr: &Expr) -> Option<DeclId> {
        match &expr.kind {
            ExprKind::Ident(id) => self.lookup(&id.name),
            ExprKind::Index { base, .. }
            | ExprKind::Range { base, .. }
            | ExprKind::Member { base, .. } => self.root_decl(base),
            _ => None,
        }
    }

    /// True when the module has an `always` block and no `initial` block:
    /// the shape of a module meant for synthesis rather than a testbench.
    pub fn looks_synthesisable(&self) -> bool {
        let mut has_always = false;
        for p in &self.procs {
            match p.kind {
                ProcKind::Initial => return false,
                ProcKind::Always(_) => has_always = true,
                _ => {}
            }
        }
        has_always
    }
}

/// What the file knows about a module it defines, for resolving instance
/// connections.
#[derive(Clone, Debug)]
pub struct ModuleInfo {
    /// Which keyword declared it.
    pub kind: ModuleKind,
    /// The ports in order, with their directions.
    pub ports: Vec<(String, Direction)>,
}

/// The facts of one file.
#[derive(Clone, Debug, Default)]
pub struct FileFacts<'a> {
    /// Every module, in source order, nested modules after their parent.
    pub modules: Vec<ModuleFacts<'a>>,
    /// Every `lint_off` attribute.
    pub lint_off: Vec<LintOff>,
    /// Every directive that reached the parser, in source order.
    pub directives: Vec<&'a crate::verilog::ast::Directive>,
    /// The spans of every `defparam` item.
    pub defparams: Vec<Span>,
    /// `typedef`s at unit and package level.
    pub typedefs: HashMap<String, &'a DataType>,
    /// The variants of every `typedef enum` at unit and package level.
    pub enums: HashMap<String, Vec<String>>,
    /// Names declared at unit or package level.
    pub unit_names: HashSet<String>,
    /// The modules defined in the file, by name.
    pub module_index: HashMap<String, ModuleInfo>,
    /// The compilation unit has an `import pkg::*`.
    pub unit_wildcard_import: bool,
}

impl<'a> FileFacts<'a> {
    /// Collects the facts of `file`.
    pub fn collect(file: &'a SourceFile) -> Self {
        let mut out = FileFacts::default();
        out.index_modules(&file.items);
        let mut nettype_none = false;
        let mut modules: Vec<&'a Module> = Vec::new();
        for item in &file.items {
            out.note_lint_off(&item.attrs, item.span);
            match &item.kind {
                ItemKind::Module(m) => {
                    let mut nested = out.collect_module(m, item.span, nettype_none);
                    while let Some((m, span)) = nested.pop() {
                        nested.extend(out.collect_module(m, span, nettype_none));
                    }
                    modules.push(m);
                }
                ItemKind::Package(p) => out.declare_unit_items(&p.items),
                ItemKind::Directive(d) => {
                    if d.name == "default_nettype" {
                        nettype_none = d.args.trim() == "none";
                    }
                    out.directives.push(d);
                }
                ItemKind::Import(refs) => {
                    if refs.iter().any(|r| r.item.is_none()) {
                        out.unit_wildcard_import = true;
                    }
                }
                ItemKind::Defparam(ds) => out.defparams.extend(ds.iter().map(|d| d.span)),
                _ => out.declare_unit_item(item),
            }
        }
        out
    }

    /// Resolves a type name through the module's typedefs, then the unit's.
    pub fn resolve_typedef(&self, m: &ModuleFacts<'a>, name: &str) -> Option<&'a DataType> {
        m.typedefs
            .get(name)
            .or_else(|| self.typedefs.get(name))
            .copied()
    }

    /// The variants of a `typedef enum` visible in the module.
    pub fn enum_variants<'b>(
        &'b self,
        m: &'b ModuleFacts<'a>,
        name: &str,
    ) -> Option<&'b Vec<String>> {
        m.enums.get(name).or_else(|| self.enums.get(name))
    }

    fn note_lint_off(&mut self, attrs: &[Attribute], span: Span) {
        for a in attrs {
            if a.name.name != "lint_off" {
                continue;
            }
            let names = match &a.value {
                Some(Expr {
                    kind: ExprKind::Literal(Literal::Str { value, .. }),
                    ..
                }) => Some(split_names(value)),
                _ => None,
            };
            self.lint_off.push(LintOff { span, names });
        }
    }

    /// Records the ports and kind of every module in `items`, recursively
    /// for nested modules.
    fn index_modules(&mut self, items: &'a [Item]) {
        for item in items {
            if let ItemKind::Module(m) = &item.kind {
                let ports = module_port_table(m, &self.module_index);
                self.module_index.insert(
                    m.name.name.clone(),
                    ModuleInfo {
                        kind: m.kind,
                        ports,
                    },
                );
                self.index_modules(&m.items);
            }
        }
    }

    fn declare_unit_items(&mut self, items: &'a [Item]) {
        for item in items {
            self.declare_unit_item(item);
        }
    }

    /// Records a unit- or package-level declaration: only its name and, for
    /// typedefs, the type, since no rule reports on unit-level items.
    fn declare_unit_item(&mut self, item: &'a Item) {
        match &item.kind {
            ItemKind::Net(n) => {
                self.unit_names
                    .extend(n.decls.iter().map(|d| d.name.name.clone()));
            }
            ItemKind::Var(v) => {
                self.unit_names
                    .extend(v.decls.iter().map(|d| d.name.name.clone()));
                self.unit_names.extend(enum_variants(&v.data_type));
            }
            ItemKind::Param(p) => {
                self.unit_names
                    .extend(p.decls.iter().map(|d| d.name.name.clone()));
            }
            ItemKind::Typedef(t) => {
                self.unit_names.insert(t.name.name.clone());
                if let Some(dt) = &t.data_type {
                    self.typedefs.insert(t.name.name.clone(), dt);
                    let variants = enum_variants(dt);
                    if !variants.is_empty() {
                        self.unit_names.extend(variants.iter().cloned());
                        self.enums.insert(t.name.name.clone(), variants);
                    }
                }
            }
            ItemKind::Function(s) | ItemKind::Task(s) => {
                self.unit_names.insert(s.name.name.clone());
            }
            ItemKind::Import(refs) if refs.iter().any(|r| r.item.is_none()) => {
                self.unit_wildcard_import = true;
            }
            _ => {}
        }
    }

    /// Collects one module; returns the nested modules it contains.
    fn collect_module(
        &mut self,
        module: &'a Module,
        span: Span,
        nettype_none: bool,
    ) -> Vec<(&'a Module, Span)> {
        let m = ModuleFacts {
            module,
            span,
            decls: Vec::new(),
            scope: HashMap::new(),
            ports: Vec::new(),
            writes: Vec::new(),
            reads: Vec::new(),
            procs: Vec::new(),
            instances: Vec::new(),
            undeclared: Vec::new(),
            wildcard_import: self.unit_wildcard_import,
            default_nettype_none: nettype_none,
            typedefs: HashMap::new(),
            enums: HashMap::new(),
        };
        let mut c = Collector {
            file: self,
            m,
            scopes: Vec::new(),
            proc: 0,
            gen_arms: Vec::new(),
            gen_next: 0,
            implicit: false,
            nested: Vec::new(),
        };
        c.run();
        let Collector { m, nested, .. } = c;
        self.modules.push(m);
        nested
    }
}

/// Splits `"a, b c"` into names.
fn split_names(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The variant names of an inline `enum` type, expanded for `A[3]` and
/// `A[1:2]` generators; empty for any other type.
fn enum_variants(dt: &DataType) -> Vec<String> {
    let DataTypeKind::Enum(e) = &dt.kind else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for v in &e.variants {
        match v.range.as_ref().map(|r| &r.kind) {
            None => out.push(v.name.name.clone()),
            Some(crate::verilog::ast::DimKind::Size(n)) => {
                let n = literal_value(n).unwrap_or(0).min(64);
                for i in 0..n {
                    out.push(format!("{}{i}", v.name.name));
                }
            }
            Some(crate::verilog::ast::DimKind::Range(a, b)) => {
                if let (Some(a), Some(b)) = (literal_value(a), literal_value(b)) {
                    let (lo, hi) = (a.min(b), a.max(b));
                    for i in lo..=hi.min(lo + 64) {
                        out.push(format!("{}{i}", v.name.name));
                    }
                }
            }
            Some(_) => out.push(v.name.name.clone()),
        }
    }
    out
}

/// A plain decimal literal's value, for enum generators.
fn literal_value(e: &Expr) -> Option<u64> {
    match &e.kind {
        ExprKind::Literal(Literal::Number { text, .. }) => text.replace('_', "").parse().ok(),
        _ => None,
    }
}

/// The root identifier of a port expression, for non-ANSI headers.
fn root_ident(e: &Expr) -> Option<&Ident> {
    match &e.kind {
        ExprKind::Ident(id) => Some(id),
        ExprKind::Index { base, .. } | ExprKind::Range { base, .. } => root_ident(base),
        ExprKind::Concat(v) => v.first().and_then(root_ident),
        _ => None,
    }
}

/// True when the type makes a port a variable rather than a net.
fn is_var_type(dt: &DataType) -> bool {
    matches!(
        dt.kind,
        DataTypeKind::Integer(_)
            | DataTypeKind::Real(_)
            | DataTypeKind::String
            | DataTypeKind::Enum(_)
            | DataTypeKind::Struct(_)
    )
}

/// True when the port's type names an interface.
fn is_interface_port(port: &Port, index: &HashMap<String, ModuleInfo>) -> bool {
    match &port.data_type.kind {
        DataTypeKind::Interface { .. } => true,
        DataTypeKind::Named {
            package: None,
            name,
            member,
        } => {
            member.is_some()
                || (port.direction.is_none()
                    && index
                        .get(&name.name)
                        .is_some_and(|i| i.kind == ModuleKind::Interface))
        }
        _ => false,
    }
}

/// The ordered `(name, direction)` port table of a module header.
fn module_port_table(m: &Module, index: &HashMap<String, ModuleInfo>) -> Vec<(String, Direction)> {
    match &m.ports {
        Ports::None => Vec::new(),
        Ports::Ansi(ports) => {
            let mut last = Direction::Inout;
            ports
                .iter()
                .map(|p| {
                    if let Some(d) = p.direction {
                        last = d;
                    }
                    let dir = if is_interface_port(p, index) {
                        Direction::Inout
                    } else {
                        last
                    };
                    (p.name.name.clone(), dir)
                })
                .collect()
        }
        Ports::NonAnsi(ports) => {
            let mut dirs: HashMap<&str, Direction> = HashMap::new();
            for item in &m.items {
                if let ItemKind::Port(p) = &item.kind {
                    for d in &p.decls {
                        dirs.insert(&d.name.name, p.direction);
                    }
                }
            }
            ports
                .iter()
                .map(|p| {
                    let inner = p.expr.as_ref().and_then(root_ident);
                    let name = p
                        .name
                        .as_ref()
                        .or(inner)
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    let dir = inner
                        .and_then(|i| dirs.get(i.name.as_str()).copied())
                        .unwrap_or(Direction::Inout);
                    (name, dir)
                })
                .collect()
        }
    }
}

/// Which terminals of a gate are outputs.
enum GateOutputs {
    /// The first `n` terminals.
    First(usize),
    /// All but the last terminal (`buf`, `not`).
    AllButLast,
    /// The first two terminals are bidirectional (`tran` family).
    Bidir,
}

fn gate_outputs(kind: GateKind) -> GateOutputs {
    use GateKind as G;
    match kind {
        G::Buf | G::Not => GateOutputs::AllButLast,
        G::Tran | G::Rtran | G::Tranif0 | G::Tranif1 | G::Rtranif0 | G::Rtranif1 => {
            GateOutputs::Bidir
        }
        _ => GateOutputs::First(1),
    }
}

/// The single-module traversal.
struct Collector<'a, 'f> {
    file: &'f mut FileFacts<'a>,
    m: ModuleFacts<'a>,
    /// Statement-level scopes, innermost last.
    scopes: Vec<HashMap<String, DeclId>>,
    /// The current process.
    proc: ProcId,
    /// The enclosing generate arms.
    gen_arms: Vec<(u32, u32)>,
    /// Next generate construct id.
    gen_next: u32,
    /// True while visiting a position where an undeclared identifier would
    /// become an implicit net.
    implicit: bool,
    /// Nested module declarations, collected after this module.
    nested: Vec<(&'a Module, Span)>,
}

/// The result of a name lookup.
enum Found {
    /// A declaration in this module.
    Decl(DeclId),
    /// A unit- or package-level name; nothing is recorded for those.
    Unit,
    /// Nothing.
    None,
}

impl<'a> Collector<'a, '_> {
    fn run(&mut self) {
        let module = self.m.module;
        // Phase A: declarations, so use-before-declaration resolves.
        if let Some(params) = &module.params {
            for p in params {
                self.declare_params(p, &[]);
            }
        }
        if let Ports::Ansi(ports) = &module.ports {
            let mut last = Direction::Inout;
            for p in ports {
                if let Some(d) = p.direction {
                    last = d;
                }
                self.file.note_lint_off(&p.attrs, p.span);
                let interface = is_interface_port(p, &self.file.module_index);
                let id = self.add_decl(Decl {
                    name: p.name.name.clone(),
                    span: p.name.span,
                    kind: DeclKind::Port(PortInfo {
                        direction: if interface { Direction::Inout } else { last },
                        net_type: p.net_type,
                        is_var: p.var || is_var_type(&p.data_type),
                        interface,
                    }),
                    data_type: Some(&p.data_type),
                    unpacked: &p.dims,
                    init: p.default.as_ref(),
                    attrs: &p.attrs,
                    local: false,
                    reads: Vec::new(),
                    writes: Vec::new(),
                    variants: None,
                });
                self.m.ports.push(id);
                self.declare_inline_enum(&p.data_type);
            }
        }
        if module.imports.iter().any(|r| r.item.is_none()) {
            self.m.wildcard_import = true;
        }
        self.declare_items(&module.items);

        // Phase B: reads, writes, processes.
        if let Ports::Ansi(ports) = &module.ports {
            for p in ports {
                if let Some(d) = &p.default {
                    let name = p.name.clone();
                    self.begin_proc(
                        ProcKind::VarInit,
                        ProcClass::NotAlways,
                        p.span,
                        None,
                        vec![],
                    );
                    self.write_ident(&name, None, WriteKind::Init, true);
                    self.visit_expr(d);
                }
            }
        }
        self.visit_items(&module.items);
    }

    // --- declarations -------------------------------------------------

    /// Adds a declaration to the module scope or the innermost local scope,
    /// merging a re-declaration of a non-ANSI port (`output q; reg q;`).
    fn add_decl(&mut self, decl: Decl<'a>) -> DeclId {
        let local = decl.local;
        if !local && let Some(&id) = self.m.scope.get(&decl.name) {
            let existing = &mut self.m.decls[id];
            match (existing.kind, decl.kind) {
                (DeclKind::Port(_), DeclKind::Net(_) | DeclKind::Var) => {
                    // The port keeps its kind; the body declaration
                    // supplies the type and dimensions.
                    if let DeclKind::Port(mut p) = existing.kind {
                        p.is_var = p.is_var || decl.kind == DeclKind::Var;
                        if let DeclKind::Net(t) = decl.kind {
                            p.net_type = Some(t);
                        }
                        existing.kind = DeclKind::Port(p);
                    }
                    if existing.data_type.is_none_or(|t| {
                        t.is_empty() || decl.data_type.is_some_and(|d| !d.is_empty())
                    }) {
                        existing.data_type = decl.data_type;
                    }
                    if !decl.unpacked.is_empty() {
                        existing.unpacked = decl.unpacked;
                    }
                    if decl.init.is_some() {
                        existing.init = decl.init;
                    }
                }
                (DeclKind::Net(_) | DeclKind::Var, DeclKind::Port(p)) => {
                    existing.kind = DeclKind::Port(PortInfo {
                        is_var: p.is_var || existing.kind == DeclKind::Var,
                        net_type: match existing.kind {
                            DeclKind::Net(t) => Some(t),
                            _ => p.net_type,
                        },
                        ..p
                    });
                    self.m.ports.push(id);
                }
                _ => {}
            }
            return id;
        }
        let id = self.m.decls.len();
        let name = decl.name.clone();
        self.m.decls.push(decl);
        if local && let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, id);
        } else {
            self.m.scope.insert(name, id);
        }
        id
    }

    fn declare_names(
        &mut self,
        decls: &'a [Declarator],
        kind: DeclKind,
        data_type: Option<&'a DataType>,
        attrs: &'a [Attribute],
        local: bool,
    ) {
        for d in decls {
            let id = self.add_decl(Decl {
                name: d.name.name.clone(),
                span: d.name.span,
                kind,
                data_type,
                unpacked: &d.dims,
                init: d.init.as_ref(),
                attrs,
                local,
                reads: Vec::new(),
                writes: Vec::new(),
                variants: None,
            });
            if matches!(kind, DeclKind::Port(_)) && !self.m.ports.contains(&id) {
                self.m.ports.push(id);
            }
        }
        if let Some(dt) = data_type {
            let variants = enum_variants(dt);
            if !variants.is_empty() {
                for d in decls {
                    if let Some(id) = self.m.scope.get(&d.name.name).copied() {
                        self.m.decls[id].variants = Some(variants.clone());
                    }
                }
            }
            self.declare_inline_enum(dt);
        }
    }

    /// Declares the variants of an inline `enum { ... }` type.
    fn declare_inline_enum(&mut self, dt: &'a DataType) {
        let DataTypeKind::Enum(e) = &dt.kind else {
            return;
        };
        let names = enum_variants(dt);
        for (i, name) in names.into_iter().enumerate() {
            let span = e.variants.get(i.min(e.variants.len().saturating_sub(1)));
            self.add_decl(Decl {
                name,
                span: span.map_or(dt.span, |v| v.name.span),
                kind: DeclKind::EnumVariant,
                data_type: None,
                unpacked: &[],
                init: None,
                attrs: &[],
                local: false,
                reads: Vec::new(),
                writes: Vec::new(),
                variants: None,
            });
        }
    }

    fn declare_params(&mut self, p: &'a crate::verilog::ast::ParamDecl, attrs: &'a [Attribute]) {
        let dt = (!p.is_type).then_some(&p.data_type);
        self.declare_names(&p.decls, DeclKind::Param(p.kind), dt, attrs, false);
    }

    fn declare_items(&mut self, items: &'a [Item]) {
        for item in items {
            self.declare_item(item, false);
        }
    }

    /// Phase A for one item: records what it declares.
    fn declare_item(&mut self, item: &'a Item, local: bool) {
        match &item.kind {
            ItemKind::Net(n) => self.declare_names(
                &n.decls,
                DeclKind::Net(n.net_type),
                Some(&n.data_type),
                &item.attrs,
                local,
            ),
            ItemKind::Var(v) => {
                self.declare_names(
                    &v.decls,
                    DeclKind::Var,
                    Some(&v.data_type),
                    &item.attrs,
                    local,
                );
            }
            ItemKind::Param(p) => {
                let dt = (!p.is_type).then_some(&p.data_type);
                self.declare_names(&p.decls, DeclKind::Param(p.kind), dt, &item.attrs, local);
            }
            ItemKind::Port(p) => {
                let kind = DeclKind::Port(PortInfo {
                    direction: p.direction,
                    net_type: p.net_type,
                    is_var: p.var || is_var_type(&p.data_type),
                    interface: false,
                });
                self.declare_names(&p.decls, kind, Some(&p.data_type), &item.attrs, local);
            }
            ItemKind::Genvar(ids) => {
                for id in ids {
                    self.simple_decl(id, DeclKind::Genvar, local);
                }
            }
            ItemKind::Typedef(t) => {
                self.simple_decl(&t.name, DeclKind::Typedef, local);
                if let Some(dt) = &t.data_type {
                    self.m.typedefs.insert(t.name.name.clone(), dt);
                    let variants = enum_variants(dt);
                    if !variants.is_empty() {
                        self.m.enums.insert(t.name.name.clone(), variants);
                    }
                    self.declare_inline_enum(dt);
                }
            }
            ItemKind::Function(s) => {
                let id = self.simple_decl(&s.name, DeclKind::Function, local);
                self.m.decls[id].data_type = s.ret.as_ref();
            }
            ItemKind::Task(s) => {
                self.simple_decl(&s.name, DeclKind::Task, local);
            }
            ItemKind::Instance(inst) => {
                for i in &inst.instances {
                    if let Some(name) = &i.name {
                        self.simple_decl(name, DeclKind::Instance, local);
                    }
                }
            }
            ItemKind::Import(refs) => {
                if refs.iter().any(|r| r.item.is_none()) {
                    self.m.wildcard_import = true;
                }
            }
            ItemKind::Generate(items) => self.declare_items(items),
            ItemKind::GenBlock(b) => self.declare_items(&b.items),
            ItemKind::GenIf(g) => {
                self.declare_items(&g.then_block.items);
                if let Some(e) = &g.else_block {
                    self.declare_items(&e.items);
                }
            }
            ItemKind::GenCase(g) => {
                for arm in &g.items {
                    self.declare_items(&arm.block.items);
                }
            }
            ItemKind::GenFor(f) => {
                if f.genvar {
                    self.simple_decl(&f.var, DeclKind::Genvar, local);
                }
                self.declare_items(&f.body.items);
            }
            _ => {}
        }
    }

    fn simple_decl(&mut self, id: &Ident, kind: DeclKind, local: bool) -> DeclId {
        self.add_decl(Decl {
            name: id.name.clone(),
            span: id.span,
            kind,
            data_type: None,
            unpacked: &[],
            init: None,
            attrs: &[],
            local,
            reads: Vec::new(),
            writes: Vec::new(),
            variants: None,
        })
    }

    // --- lookup and recording -------------------------------------------

    fn lookup(&self, name: &str) -> Found {
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.get(name) {
                return Found::Decl(id);
            }
        }
        if let Some(&id) = self.m.scope.get(name) {
            return Found::Decl(id);
        }
        if self.file.unit_names.contains(name) {
            return Found::Unit;
        }
        Found::None
    }

    fn begin_proc(
        &mut self,
        kind: ProcKind,
        class: ProcClass,
        span: Span,
        sens: Option<&'a EventControl>,
        stmts: Vec<&'a Stmt>,
    ) -> ProcId {
        let id = self.m.procs.len();
        self.m.procs.push(Proc {
            kind,
            class,
            span,
            sens,
            stmts,
            gen_arms: self.gen_arms.clone(),
        });
        self.proc = id;
        id
    }

    fn read_ident(&mut self, id: &Ident) {
        match self.lookup(&id.name) {
            Found::Decl(d) => {
                let idx = self.m.reads.len();
                self.m.reads.push(Read {
                    decl: d,
                    span: id.span,
                    proc: self.proc,
                });
                self.m.decls[d].reads.push(idx);
            }
            Found::Unit => {}
            Found::None => {
                if self.implicit {
                    self.m.undeclared.push(Undeclared {
                        ident: id.clone(),
                        context: UseContext::PortConn,
                    });
                }
            }
        }
    }

    fn write_ident(&mut self, id: &Ident, lhs: Option<&'a Expr>, kind: WriteKind, whole: bool) {
        match self.lookup(&id.name) {
            Found::Decl(d) => {
                let idx = self.m.writes.len();
                self.m.writes.push(Write {
                    decl: d,
                    span: lhs.map_or(id.span, |e| e.span),
                    kind,
                    proc: self.proc,
                    whole,
                    lhs,
                });
                self.m.decls[d].writes.push(idx);
            }
            Found::Unit => {}
            Found::None => {
                if self.implicit {
                    let context = if kind == WriteKind::Continuous {
                        UseContext::AssignLhs
                    } else {
                        UseContext::PortConn
                    };
                    self.m.undeclared.push(Undeclared {
                        ident: id.clone(),
                        context,
                    });
                }
            }
        }
    }

    // --- items -------------------------------------------------------------

    fn visit_items(&mut self, items: &'a [Item]) {
        for item in items {
            self.visit_item(item);
        }
    }

    fn visit_item(&mut self, item: &'a Item) {
        self.file.note_lint_off(&item.attrs, item.span);
        match &item.kind {
            ItemKind::Module(m) => self.nested.push((m, item.span)),
            ItemKind::Package(_) => {}
            ItemKind::Net(n) => {
                for d in &n.decls {
                    self.visit_dims(&d.dims);
                    if let Some(init) = &d.init {
                        self.begin_proc(
                            ProcKind::NetInit,
                            ProcClass::NotAlways,
                            d.span,
                            None,
                            vec![],
                        );
                        self.write_ident(&d.name, None, WriteKind::Continuous, true);
                        self.visit_expr(init);
                    }
                }
            }
            ItemKind::Var(v) => {
                for d in &v.decls {
                    self.visit_dims(&d.dims);
                    if let Some(init) = &d.init {
                        self.begin_proc(
                            ProcKind::VarInit,
                            ProcClass::NotAlways,
                            d.span,
                            None,
                            vec![],
                        );
                        self.write_ident(&d.name, None, WriteKind::Init, true);
                        self.visit_expr(init);
                    }
                }
            }
            ItemKind::Param(_)
            | ItemKind::Port(_)
            | ItemKind::Genvar(_)
            | ItemKind::Typedef(_)
            | ItemKind::Import(_)
            | ItemKind::Export(_)
            | ItemKind::Specify(_)
            | ItemKind::Clocking(_)
            | ItemKind::PropertyDecl(_)
            | ItemKind::Modport(_)
            | ItemKind::Timeunit { .. }
            | ItemKind::Timeprecision(_)
            | ItemKind::Table(_)
            | ItemKind::Bind(_)
            | ItemKind::Empty => {}
            ItemKind::Function(s) => self.visit_subroutine(s, ProcKind::Function, item.span),
            ItemKind::Task(s) => self.visit_subroutine(s, ProcKind::Task, item.span),
            ItemKind::Defparam(ds) => {
                self.file.defparams.extend(ds.iter().map(|d| d.span));
            }
            ItemKind::Initial(s) => {
                self.begin_proc(
                    ProcKind::Initial,
                    ProcClass::NotAlways,
                    item.span,
                    None,
                    vec![s],
                );
                self.visit_stmt(s);
            }
            ItemKind::Final(s) => {
                self.begin_proc(
                    ProcKind::Final,
                    ProcClass::NotAlways,
                    item.span,
                    None,
                    vec![s],
                );
                self.visit_stmt(s);
            }
            ItemKind::Always(kind, s) => {
                let (class, sens) = classify_always(*kind, s);
                self.begin_proc(ProcKind::Always(*kind), class, item.span, sens, vec![s]);
                self.visit_stmt(s);
            }
            ItemKind::ContAssign(ca) => {
                if let Some(d) = &ca.delay {
                    self.visit_exprs(&d.values);
                }
                for pair in &ca.assigns {
                    self.begin_proc(
                        ProcKind::ContAssign,
                        ProcClass::NotAlways,
                        pair.span,
                        None,
                        vec![],
                    );
                    self.implicit = true;
                    self.visit_lhs(&pair.lhs, WriteKind::Continuous);
                    self.implicit = false;
                    self.visit_expr(&pair.rhs);
                }
            }
            ItemKind::Gate(g) => {
                if let Some(d) = &g.delay {
                    self.visit_exprs(&d.values);
                }
                let outputs = gate_outputs(g.kind);
                for inst in &g.instances {
                    self.begin_proc(
                        ProcKind::Gate,
                        ProcClass::NotAlways,
                        inst.span,
                        None,
                        vec![],
                    );
                    self.visit_dims(&inst.dims);
                    let n = inst.conns.len();
                    for (i, conn) in inst.conns.iter().enumerate() {
                        self.implicit = true;
                        match outputs {
                            GateOutputs::First(k) if i < k => {
                                self.visit_lhs(conn, WriteKind::Gate);
                            }
                            GateOutputs::AllButLast if i + 1 < n => {
                                self.visit_lhs(conn, WriteKind::Gate);
                            }
                            GateOutputs::Bidir if i < 2 => {
                                self.visit_lhs(conn, WriteKind::InstanceInout);
                                self.visit_expr(conn);
                            }
                            _ => self.visit_expr(conn),
                        }
                        self.implicit = false;
                    }
                }
            }
            ItemKind::Instance(inst) => self.visit_instantiation(inst),
            ItemKind::Generate(items) => self.visit_items(items),
            ItemKind::GenBlock(b) => self.visit_items(&b.items),
            ItemKind::GenIf(g) => {
                self.visit_expr(&g.cond);
                let id = self.gen_next;
                self.gen_next += 1;
                self.gen_arms.push((id, 0));
                self.visit_items(&g.then_block.items);
                self.gen_arms.pop();
                if let Some(e) = &g.else_block {
                    self.gen_arms.push((id, 1));
                    self.visit_items(&e.items);
                    self.gen_arms.pop();
                }
            }
            ItemKind::GenCase(g) => {
                self.visit_expr(&g.expr);
                let id = self.gen_next;
                self.gen_next += 1;
                for (arm, item) in g.items.iter().enumerate() {
                    self.visit_exprs(&item.patterns);
                    self.gen_arms
                        .push((id, u32::try_from(arm).unwrap_or(u32::MAX)));
                    self.visit_items(&item.block.items);
                    self.gen_arms.pop();
                }
            }
            ItemKind::GenFor(f) => {
                self.visit_expr(&f.init);
                self.visit_expr(&f.cond);
                self.visit_expr(&f.step);
                self.visit_items(&f.body.items);
            }
            ItemKind::Alias(exprs) => {
                self.begin_proc(
                    ProcKind::Other,
                    ProcClass::NotAlways,
                    item.span,
                    None,
                    vec![],
                );
                self.visit_exprs(exprs);
            }
            ItemKind::Assertion(a) => {
                self.begin_proc(
                    ProcKind::Other,
                    ProcClass::NotAlways,
                    item.span,
                    None,
                    vec![],
                );
                self.visit_assertion(a);
            }
            ItemKind::Directive(d) => {
                self.file.directives.push(d);
            }
        }
    }

    fn visit_subroutine(&mut self, s: &'a Subroutine, kind: ProcKind, span: Span) {
        self.begin_proc(
            kind,
            ProcClass::NotAlways,
            span,
            None,
            s.body.iter().collect(),
        );
        self.scopes.push(HashMap::new());
        if let Some(ports) = &s.ports {
            let mut last = Direction::Input;
            for p in ports {
                if let Some(d) = p.direction {
                    last = d;
                }
                self.add_decl(Decl {
                    name: p.name.name.clone(),
                    span: p.name.span,
                    kind: DeclKind::SubPort(last),
                    data_type: Some(&p.data_type),
                    unpacked: &p.dims,
                    init: p.default.as_ref(),
                    attrs: &p.attrs,
                    local: true,
                    reads: Vec::new(),
                    writes: Vec::new(),
                    variants: None,
                });
            }
        }
        // Body-declared ports (`input [7:0] a;`) and locals.
        for st in &s.body {
            if let StmtKind::Decl(item) = &st.kind {
                match &item.kind {
                    ItemKind::Port(p) => {
                        let kind = DeclKind::SubPort(p.direction);
                        self.declare_names(&p.decls, kind, Some(&p.data_type), &item.attrs, true);
                    }
                    _ => self.declare_item(item, true),
                }
            }
        }
        for st in &s.body {
            self.visit_stmt(st);
        }
        self.scopes.pop();
    }

    fn visit_instantiation(&mut self, inst: &'a crate::verilog::ast::Instantiation) {
        let target = self.file.module_index.get(&inst.module.name).cloned();
        for p in &inst.params {
            if let Some(v) = &p.value {
                self.visit_expr(v);
            }
        }
        for i in &inst.instances {
            let proc = self.begin_proc(
                ProcKind::Instance,
                ProcClass::NotAlways,
                i.span,
                None,
                vec![],
            );
            self.visit_dims(&i.dims);
            let mut conns = Vec::new();
            let mut wildcard = false;
            for (pos, c) in i.conns.iter().enumerate() {
                match &c.kind {
                    PortConnKind::Positional(e) => {
                        let direction = target.as_ref().and_then(|t| t.ports.get(pos)).map(|p| p.1);
                        conns.push(Conn {
                            name: None,
                            expr: e.as_ref(),
                            direction,
                            positional: true,
                            open: e.is_none(),
                            span: c.span,
                        });
                        if let Some(e) = e {
                            self.connect(e, direction);
                        }
                    }
                    PortConnKind::Named { name, conn } => {
                        let direction = target.as_ref().and_then(|t| {
                            t.ports.iter().find(|(n, _)| *n == name.name).map(|p| p.1)
                        });
                        let (expr, open) = match conn {
                            NamedConn::Implicit => {
                                self.connect_name(name, direction);
                                (None, false)
                            }
                            NamedConn::Open => (None, true),
                            NamedConn::Expr(e) => {
                                self.connect(e, direction);
                                (Some(e), false)
                            }
                        };
                        conns.push(Conn {
                            name: Some(name),
                            expr,
                            direction,
                            positional: false,
                            open,
                            span: c.span,
                        });
                    }
                    PortConnKind::Wildcard => wildcard = true,
                }
            }
            self.m.instances.push(Instance {
                module: &inst.module,
                name: i.name.as_ref(),
                span: i.span,
                conns,
                wildcard,
                known: target.is_some(),
                proc,
            });
        }
    }

    /// Records the reads and writes of a port connection expression.
    fn connect(&mut self, e: &'a Expr, direction: Option<Direction>) {
        self.implicit = true;
        match direction {
            Some(Direction::Input | Direction::ConstRef) => self.visit_expr(e),
            Some(Direction::Output) => self.visit_lhs(e, WriteKind::InstanceOutput),
            _ => {
                self.visit_lhs(e, WriteKind::InstanceInout);
                self.visit_expr(e);
            }
        }
        self.implicit = false;
    }

    /// Records the reads and writes of an implicit `.name` connection.
    fn connect_name(&mut self, name: &Ident, direction: Option<Direction>) {
        self.implicit = true;
        match direction {
            Some(Direction::Input | Direction::ConstRef) => self.read_ident(name),
            Some(Direction::Output) => {
                self.write_ident(name, None, WriteKind::InstanceOutput, true)
            }
            _ => {
                self.write_ident(name, None, WriteKind::InstanceInout, true);
                self.read_ident(name);
            }
        }
        self.implicit = false;
    }

    fn visit_assertion(&mut self, a: &'a Assertion) {
        if let crate::verilog::ast::AssertSpec::Expr(e) = &a.spec {
            self.visit_expr(e);
        }
        if let Some(s) = &a.then_stmt {
            self.visit_stmt(s);
        }
        if let Some(s) = &a.else_stmt {
            self.visit_stmt(s);
        }
    }

    fn visit_dims(&mut self, dims: &'a [Dim]) {
        for d in dims {
            match &d.kind {
                crate::verilog::ast::DimKind::Range(a, b) => {
                    self.visit_expr(a);
                    self.visit_expr(b);
                }
                crate::verilog::ast::DimKind::Size(n) => self.visit_expr(n),
                crate::verilog::ast::DimKind::Queue(Some(n)) => self.visit_expr(n),
                _ => {}
            }
        }
    }

    // --- statements ----------------------------------------------------

    fn visit_stmt(&mut self, s: &'a Stmt) {
        self.file.note_lint_off(&s.attrs, s.span);
        match &s.kind {
            StmtKind::Null
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::DisableFork
            | StmtKind::WaitFork
            | StmtKind::Disable(_) => {}
            StmtKind::Block(b) | StmtKind::Fork(b, _) => {
                self.scopes.push(HashMap::new());
                for st in &b.stmts {
                    if let StmtKind::Decl(item) = &st.kind {
                        self.declare_item(item, true);
                    }
                }
                for st in &b.stmts {
                    self.visit_stmt(st);
                }
                self.scopes.pop();
            }
            StmtKind::Assign(a) => {
                let kind = match a.op {
                    crate::verilog::ast::AssignOp::NonBlocking => WriteKind::NonBlocking,
                    _ => WriteKind::Blocking,
                };
                self.visit_lhs(&a.lhs, kind);
                if !matches!(
                    a.op,
                    crate::verilog::ast::AssignOp::Blocking
                        | crate::verilog::ast::AssignOp::NonBlocking
                ) {
                    // Compound assignment reads the target too.
                    self.visit_expr(&a.lhs);
                }
                if let Some(t) = &a.timing {
                    self.visit_timing(t);
                }
                self.visit_expr(&a.rhs);
            }
            StmtKind::Expr(e) => {
                if let ExprKind::Call { callee, args } = &e.kind
                    && let ExprKind::Ident(id) = &callee.kind
                {
                    // A task call may write its arguments.
                    self.read_ident(id);
                    self.visit_call_args(args, true);
                } else {
                    self.visit_expr(e);
                }
            }
            StmtKind::If(i) => {
                self.visit_expr(&i.cond);
                self.visit_stmt(&i.then_stmt);
                if let Some(e) = &i.else_stmt {
                    self.visit_stmt(e);
                }
            }
            StmtKind::Case(c) => {
                self.visit_expr(&c.expr);
                for item in &c.items {
                    self.visit_exprs(&item.patterns);
                    self.visit_stmt(&item.body);
                }
            }
            StmtKind::For(f) => {
                self.scopes.push(HashMap::new());
                for init in &f.init {
                    match init {
                        ForInit::Decl(v) => {
                            self.declare_names(
                                &v.decls,
                                DeclKind::Var,
                                Some(&v.data_type),
                                &[],
                                true,
                            );
                            for d in &v.decls {
                                if let Some(e) = &d.init {
                                    self.write_ident(&d.name, None, WriteKind::Init, true);
                                    self.visit_expr(e);
                                }
                            }
                        }
                        ForInit::Assign(e) => self.visit_expr(e),
                    }
                }
                if let Some(c) = &f.cond {
                    self.visit_expr(c);
                }
                self.visit_exprs(&f.step);
                self.visit_stmt(&f.body);
                self.scopes.pop();
            }
            StmtKind::While(c, body) | StmtKind::DoWhile(body, c) | StmtKind::Repeat(c, body) => {
                self.visit_expr(c);
                self.visit_stmt(body);
            }
            StmtKind::Wait(c, body) => {
                self.visit_expr(c);
                self.visit_stmt(body);
            }
            StmtKind::Forever(body) => self.visit_stmt(body),
            StmtKind::Foreach(f) => {
                self.visit_expr(&f.array);
                self.scopes.push(HashMap::new());
                for v in f.vars.iter().flatten() {
                    self.simple_decl(v, DeclKind::LoopVar, true);
                }
                self.visit_stmt(&f.body);
                self.scopes.pop();
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.visit_expr(e);
                }
            }
            StmtKind::Timing(t, body) => {
                self.visit_timing(t);
                self.visit_stmt(body);
            }
            StmtKind::ProcAssign(l, r) | StmtKind::Force(l, r) => {
                self.visit_lhs(l, WriteKind::Procedural);
                self.visit_expr(r);
            }
            StmtKind::Deassign(l) | StmtKind::Release(l) => {
                self.visit_lhs(l, WriteKind::Procedural);
            }
            StmtKind::Trigger { target, .. } => self.visit_lhs(target, WriteKind::Procedural),
            StmtKind::Assert(a) => self.visit_assertion(a),
            StmtKind::Decl(item) => {
                // Declared when the enclosing block was entered; only the
                // initialisers remain.
                match &item.kind {
                    ItemKind::Var(v) => {
                        for d in &v.decls {
                            self.visit_dims(&d.dims);
                            if let Some(e) = &d.init {
                                self.write_ident(&d.name, None, WriteKind::Init, true);
                                self.visit_expr(e);
                            }
                        }
                    }
                    ItemKind::Net(n) => {
                        for d in &n.decls {
                            if let Some(e) = &d.init {
                                self.write_ident(&d.name, None, WriteKind::Continuous, true);
                                self.visit_expr(e);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn visit_timing(&mut self, t: &'a TimingControl) {
        match &t.kind {
            TimingKind::Delay(d) => self.visit_exprs(&d.values),
            TimingKind::Event(ec) => self.visit_event(ec),
            TimingKind::RepeatEvent(e, ec) => {
                self.visit_expr(e);
                self.visit_event(ec);
            }
        }
    }

    fn visit_event(&mut self, ec: &'a EventControl) {
        if let EventControlKind::List(list) = &ec.kind {
            for e in list {
                self.visit_expr(&e.expr);
                if let Some(iff) = &e.iff {
                    self.visit_expr(iff);
                }
            }
        }
    }

    // --- expressions ---------------------------------------------------

    fn visit_exprs(&mut self, exprs: &'a [Expr]) {
        for e in exprs {
            self.visit_expr(e);
        }
    }

    /// Visits `e` with implicit-net creation off (index expressions,
    /// replication counts and the like never create nets).
    fn visit_expr_no_implicit(&mut self, e: &'a Expr) {
        let saved = self.implicit;
        self.implicit = false;
        self.visit_expr(e);
        self.implicit = saved;
    }

    fn visit_call_args(&mut self, args: &'a [Arg], may_write: bool) {
        let saved = self.implicit;
        self.implicit = false;
        for a in args {
            if let Some(v) = &a.value {
                self.visit_expr(v);
                if may_write {
                    self.visit_lhs(v, WriteKind::CallArg);
                }
            }
        }
        self.implicit = saved;
    }

    fn visit_expr(&mut self, e: &'a Expr) {
        match &e.kind {
            ExprKind::Literal(_)
            | ExprKind::Default
            | ExprKind::Type(_)
            | ExprKind::SystemIdent(_)
            | ExprKind::Scoped { .. } => {}
            ExprKind::Ident(id) => self.read_ident(id),
            ExprKind::Member { base, .. } => self.visit_expr_no_implicit(base),
            ExprKind::Index { base, index } => {
                self.visit_expr(base);
                self.visit_expr_no_implicit(index);
            }
            ExprKind::Range {
                base, left, right, ..
            } => {
                self.visit_expr(base);
                self.visit_expr_no_implicit(left);
                self.visit_expr_no_implicit(right);
            }
            ExprKind::Unary { operand, .. } => self.visit_expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.visit_expr(lhs);
                self.visit_expr(rhs);
            }
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                self.visit_expr(cond);
                self.visit_expr(then_expr);
                self.visit_expr(else_expr);
            }
            ExprKind::Concat(v) | ExprKind::New(v) => self.visit_exprs(v),
            ExprKind::Replicate { count, elems } => {
                self.visit_expr_no_implicit(count);
                self.visit_exprs(elems);
            }
            ExprKind::Streaming { slice, elems, .. } => {
                if let Some(s) = slice {
                    self.visit_expr_no_implicit(s);
                }
                self.visit_exprs(elems);
            }
            ExprKind::Pattern(items) => {
                for item in items {
                    if let Some(k) = &item.key
                        && !matches!(k.kind, ExprKind::Default | ExprKind::Type(_))
                    {
                        self.visit_expr_no_implicit(k);
                    }
                    self.visit_expr(&item.value);
                }
            }
            ExprKind::Call { callee, args } => {
                match &callee.kind {
                    ExprKind::Ident(id) => self.read_ident(id),
                    ExprKind::SystemIdent(_) => {}
                    _ => self.visit_expr_no_implicit(callee),
                }
                self.visit_call_args(args, false);
            }
            ExprKind::Cast { target, expr } => {
                if let crate::verilog::ast::CastTarget::Size(n) = target {
                    self.visit_expr_no_implicit(n);
                }
                self.visit_expr(expr);
            }
            ExprKind::Inside { expr, set } => {
                self.visit_expr(expr);
                self.visit_exprs(set);
            }
            ExprKind::ValueRange { low, high } => {
                self.visit_expr(low);
                self.visit_expr(high);
            }
            ExprKind::MinTypMax { min, typ, max } => {
                self.visit_expr(min);
                self.visit_expr(typ);
                self.visit_expr(max);
            }
            ExprKind::Assign { lhs, op, rhs } => {
                let kind = match op {
                    crate::verilog::ast::AssignOp::NonBlocking => WriteKind::NonBlocking,
                    _ => WriteKind::Blocking,
                };
                self.visit_lhs(lhs, kind);
                self.visit_expr(rhs);
            }
            ExprKind::IncDec { target, .. } => {
                self.visit_lhs(target, WriteKind::Blocking);
                self.visit_expr(target);
            }
        }
    }

    /// Records the writes of an assignment target and the reads of any
    /// index expressions in it.
    fn visit_lhs(&mut self, e: &'a Expr, kind: WriteKind) {
        match &e.kind {
            ExprKind::Ident(id) => self.write_ident(id, Some(e), kind, true),
            ExprKind::Index { .. } | ExprKind::Range { .. } | ExprKind::Member { .. } => {
                self.lhs_partial(e, e, kind);
            }
            ExprKind::Concat(v) | ExprKind::Streaming { elems: v, .. } => {
                for x in v {
                    self.visit_lhs(x, kind);
                }
            }
            _ => self.visit_expr(e),
        }
    }

    /// Descends a bit-, part-, element- or member-select target to its root
    /// identifier, recording a partial write.
    fn lhs_partial(&mut self, e: &'a Expr, full: &'a Expr, kind: WriteKind) {
        match &e.kind {
            ExprKind::Ident(id) => self.write_ident(id, Some(full), kind, false),
            ExprKind::Index { base, index } => {
                self.lhs_partial(base, full, kind);
                self.visit_expr_no_implicit(index);
            }
            ExprKind::Range {
                base, left, right, ..
            } => {
                self.lhs_partial(base, full, kind);
                self.visit_expr_no_implicit(left);
                self.visit_expr_no_implicit(right);
            }
            ExprKind::Member { base, .. } => {
                let saved = self.implicit;
                self.implicit = false;
                self.lhs_partial(base, full, kind);
                self.implicit = saved;
            }
            _ => self.visit_expr(e),
        }
    }
}

/// Classifies an `always` block from its keyword and sensitivity list.
fn classify_always(kind: AlwaysKind, s: &Stmt) -> (ProcClass, Option<&EventControl>) {
    let sens = match &s.kind {
        StmtKind::Timing(
            TimingControl {
                kind: TimingKind::Event(ec),
                ..
            },
            _,
        ) => Some(ec),
        _ => None,
    };
    let class = match kind {
        AlwaysKind::Comb => ProcClass::Combinational,
        AlwaysKind::Ff => ProcClass::Sequential,
        AlwaysKind::Latch => ProcClass::Latch,
        AlwaysKind::Always => match sens.map(|ec| &ec.kind) {
            None => ProcClass::Untimed,
            Some(EventControlKind::Any) => ProcClass::Combinational,
            Some(EventControlKind::List(list)) => {
                if list.iter().any(|e| e.edge.is_some()) {
                    ProcClass::Sequential
                } else {
                    ProcClass::Combinational
                }
            }
        },
    };
    (class, sens)
}

// --- traversal helpers ---------------------------------------------------

/// The statements directly inside `s`, in source order.
pub fn stmt_children(s: &Stmt) -> Vec<&Stmt> {
    match &s.kind {
        StmtKind::Block(b) | StmtKind::Fork(b, _) => b.stmts.iter().collect(),
        StmtKind::If(i) => {
            let mut v = vec![&*i.then_stmt];
            v.extend(i.else_stmt.as_deref());
            v
        }
        StmtKind::Case(c) => c.items.iter().map(|i| &i.body).collect(),
        StmtKind::For(f) => vec![&*f.body],
        StmtKind::While(_, b)
        | StmtKind::DoWhile(b, _)
        | StmtKind::Repeat(_, b)
        | StmtKind::Forever(b)
        | StmtKind::Wait(_, b)
        | StmtKind::Timing(_, b) => vec![b],
        StmtKind::Foreach(f) => vec![&*f.body],
        StmtKind::Assert(a) => a
            .then_stmt
            .as_deref()
            .into_iter()
            .chain(a.else_stmt.as_deref())
            .collect(),
        _ => Vec::new(),
    }
}

/// Calls `f` on `s` and, in pre-order, on every statement inside it.
pub fn walk_stmt<'a>(s: &'a Stmt, f: &mut impl FnMut(&'a Stmt)) {
    f(s);
    for child in stmt_children(s) {
        walk_stmt(child, f);
    }
}

/// Calls `f` on every statement of every process of `m`.
pub fn walk_proc<'a>(p: &Proc<'a>, f: &mut impl FnMut(&'a Stmt)) {
    for s in &p.stmts {
        walk_stmt(s, f);
    }
}

/// Calls `f` on every item of `items`, descending into generate
/// constructs but not into nested modules or packages.
pub fn walk_items<'a>(items: &'a [Item], f: &mut impl FnMut(&'a Item)) {
    for item in items {
        f(item);
        match &item.kind {
            ItemKind::Generate(items) => walk_items(items, f),
            ItemKind::GenBlock(b) => walk_items(&b.items, f),
            ItemKind::GenIf(g) => {
                walk_items(&g.then_block.items, f);
                if let Some(e) = &g.else_block {
                    walk_items(&e.items, f);
                }
            }
            ItemKind::GenCase(g) => {
                for arm in &g.items {
                    walk_items(&arm.block.items, f);
                }
            }
            ItemKind::GenFor(g) => walk_items(&g.body.items, f),
            _ => {}
        }
    }
}

/// Calls `f` on `e` and, in pre-order, on every expression inside it.
pub fn walk_expr<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Literal(_)
        | ExprKind::Ident(_)
        | ExprKind::SystemIdent(_)
        | ExprKind::Type(_)
        | ExprKind::Default
        | ExprKind::Scoped { .. } => {}
        ExprKind::Member { base, .. } => walk_expr(base, f),
        ExprKind::Index { base, index } => {
            walk_expr(base, f);
            walk_expr(index, f);
        }
        ExprKind::Range {
            base, left, right, ..
        } => {
            walk_expr(base, f);
            walk_expr(left, f);
            walk_expr(right, f);
        }
        ExprKind::Unary { operand, .. } => walk_expr(operand, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            walk_expr(cond, f);
            walk_expr(then_expr, f);
            walk_expr(else_expr, f);
        }
        ExprKind::Concat(v) | ExprKind::New(v) => {
            for x in v {
                walk_expr(x, f);
            }
        }
        ExprKind::Replicate { count, elems } => {
            walk_expr(count, f);
            for x in elems {
                walk_expr(x, f);
            }
        }
        ExprKind::Streaming { slice, elems, .. } => {
            if let Some(s) = slice {
                walk_expr(s, f);
            }
            for x in elems {
                walk_expr(x, f);
            }
        }
        ExprKind::Pattern(items) => {
            for item in items {
                if let Some(k) = &item.key {
                    walk_expr(k, f);
                }
                walk_expr(&item.value, f);
            }
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, f);
            for a in args {
                if let Some(v) = &a.value {
                    walk_expr(v, f);
                }
            }
        }
        ExprKind::Cast { target, expr } => {
            if let crate::verilog::ast::CastTarget::Size(n) = target {
                walk_expr(n, f);
            }
            walk_expr(expr, f);
        }
        ExprKind::Inside { expr, set } => {
            walk_expr(expr, f);
            for x in set {
                walk_expr(x, f);
            }
        }
        ExprKind::ValueRange { low, high } => {
            walk_expr(low, f);
            walk_expr(high, f);
        }
        ExprKind::MinTypMax { min, typ, max } => {
            walk_expr(min, f);
            walk_expr(typ, f);
            walk_expr(max, f);
        }
        ExprKind::Assign { lhs, rhs, .. } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::IncDec { target, .. } => walk_expr(target, f),
    }
}

/// The expressions written directly in `s`, excluding those of the
/// statements nested inside it.
pub fn stmt_exprs(s: &Stmt) -> Vec<&Expr> {
    let mut out = Vec::new();
    match &s.kind {
        StmtKind::Assign(a) => {
            out.push(&a.lhs);
            out.push(&a.rhs);
        }
        StmtKind::Expr(e) | StmtKind::Disable(e) | StmtKind::Deassign(e) | StmtKind::Release(e) => {
            out.push(e);
        }
        StmtKind::If(i) => out.push(&i.cond),
        StmtKind::Case(c) => {
            out.push(&c.expr);
            for item in &c.items {
                out.extend(item.patterns.iter());
            }
        }
        StmtKind::For(f) => {
            for init in &f.init {
                match init {
                    ForInit::Decl(v) => out.extend(v.decls.iter().filter_map(|d| d.init.as_ref())),
                    ForInit::Assign(e) => out.push(e),
                }
            }
            out.extend(f.cond.as_ref());
            out.extend(f.step.iter());
        }
        StmtKind::While(c, _) | StmtKind::DoWhile(_, c) | StmtKind::Repeat(c, _) => out.push(c),
        StmtKind::Wait(c, _) => out.push(c),
        StmtKind::Foreach(f) => out.push(&f.array),
        StmtKind::Return(e) => out.extend(e.as_ref()),
        StmtKind::ProcAssign(a, b) | StmtKind::Force(a, b) => {
            out.push(a);
            out.push(b);
        }
        StmtKind::Trigger { target, .. } => out.push(target),
        StmtKind::Assert(a) => {
            if let crate::verilog::ast::AssertSpec::Expr(e) = &a.spec {
                out.push(e);
            }
        }
        StmtKind::Decl(item) => match &item.kind {
            ItemKind::Var(v) => out.extend(v.decls.iter().filter_map(|d| d.init.as_ref())),
            ItemKind::Net(n) => out.extend(n.decls.iter().filter_map(|d| d.init.as_ref())),
            _ => {}
        },
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::source::SourceMap;
    use crate::verilog::{Dialect, NoIncludes, parse_source};

    fn parse(src: &str) -> (SourceMap, SourceFile) {
        let mut map = SourceMap::new();
        let id = map.add("t.sv", src).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(
            &mut map,
            id,
            Dialect::SystemVerilog,
            &mut NoIncludes,
            &mut diags,
        );
        assert!(diags.is_empty(), "{}", diags.render(&map));
        (map, file)
    }

    #[test]
    fn collects_decls_reads_and_writes() {
        let (_, file) = parse(
            "module m(input clk, input [3:0] a, output reg [3:0] q, output w);\n\
             wire t = a[0];\n\
             assign w = t & a[1];\n\
             always @(posedge clk) q <= a;\n\
             endmodule\n",
        );
        let facts = FileFacts::collect(&file);
        assert_eq!(facts.modules.len(), 1);
        let m = &facts.modules[0];
        assert_eq!(m.ports.len(), 4);
        let a = m.lookup("a").unwrap();
        assert_eq!(m.decl(a).reads.len(), 3);
        assert!(m.decl(a).writes.is_empty());
        let q = m.lookup("q").unwrap();
        assert_eq!(m.decl(q).writes.len(), 1);
        assert_eq!(m.writes[m.decl(q).writes[0]].kind, WriteKind::NonBlocking);
        let t = m.lookup("t").unwrap();
        assert_eq!(m.writes[m.decl(t).writes[0]].kind, WriteKind::Continuous);
        let seq = m
            .procs
            .iter()
            .find(|p| matches!(p.kind, ProcKind::Always(_)))
            .unwrap();
        assert_eq!(seq.class, ProcClass::Sequential);
        assert!(seq.sens.is_some());
    }

    #[test]
    fn resolves_instance_directions_and_undeclared_names() {
        let (_, file) = parse(
            "module leaf(input i, output o);\nassign o = i;\nendmodule\n\
             module top;\nwire x;\nleaf u0 (.i(x), .o(y));\nleaf u1 (x, z);\nendmodule\n",
        );
        let facts = FileFacts::collect(&file);
        let top = &facts.modules[1];
        assert_eq!(top.instances.len(), 2);
        assert_eq!(top.instances[0].conns[1].direction, Some(Direction::Output));
        assert!(top.instances[1].conns[0].positional);
        let names: Vec<_> = top
            .undeclared
            .iter()
            .map(|u| u.ident.name.as_str())
            .collect();
        assert_eq!(names, ["y", "z"]);
        let x = top.lookup("x").unwrap();
        assert_eq!(top.decl(x).reads.len(), 2);
    }

    #[test]
    fn generate_arms_are_exclusive() {
        let (_, file) = parse(
            "module m #(parameter P = 0) (input a, output y);\n\
             generate if (P) begin : g1 assign y = a; end else begin : g2 assign y = ~a; end endgenerate\n\
             endmodule\n",
        );
        let facts = FileFacts::collect(&file);
        let m = &facts.modules[0];
        let assigns: Vec<_> = m
            .procs
            .iter()
            .filter(|p| p.kind == ProcKind::ContAssign)
            .collect();
        assert_eq!(assigns.len(), 2);
        assert!(assigns[0].exclusive_with(assigns[1]));
    }

    #[test]
    fn local_scopes_and_lint_off_attributes() {
        let (_, file) = parse(
            "module m;\ninteger n;\n(* lint_off = \"unused-signal, tabs\" *) reg r;\n\
             initial begin : b\n integer k;\n for (k = 0; k < 2; k = k + 1) n = k;\nend\nendmodule\n",
        );
        let facts = FileFacts::collect(&file);
        let m = &facts.modules[0];
        let k = m.decls.iter().find(|d| d.name == "k").unwrap();
        assert!(k.local);
        assert_eq!(k.writes.len(), 2);
        assert_eq!(
            facts.lint_off[0].names.as_deref(),
            Some(&["unused-signal".to_string(), "tabs".to_string()][..])
        );
    }
}
