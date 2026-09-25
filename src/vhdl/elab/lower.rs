//! Building one IR module out of one entity and architecture.
//!
//! [`Lowerer`] owns a [`ModuleBuilder`] and a stack of scopes mapping the
//! analyser's [`DeclId`]s to what they became in the IR: a net, a memory or
//! a static value. It walks the entity's generics and ports, then the
//! entity's and the architecture's declarative parts, then their
//! statements, adding to the same module throughout — generate statements
//! and blocks are flattened into it under a hierarchical name prefix
//! rather than becoming modules of their own.
//!
//! Two things are decided only once the whole module has been walked and
//! are therefore done in [`Lowerer::finish`]:
//!
//! - **Net kinds.** A signal driven by a process is a `reg`, a signal
//!   driven by a concurrent assignment or by nothing is a `wire`, and a
//!   variable is a `var`. Which of the three a signal is only follows from
//!   its drivers, so the kind is set at the end.
//! - **Resolution.** A signal driven from more than one place needs the
//!   resolution its subtype names. Each driver is moved onto its own net
//!   `<signal>$d<n>` — by rewriting the driver's target, so nothing that
//!   *reads* the signal changes — and the signal itself is driven by one
//!   assignment holding the resolution function's logic. A signal with one
//!   driver, the overwhelming case, is left exactly as it was lowered.

use std::collections::HashMap;

use crate::diag::Diagnostic;
use crate::ir::builder::{BlockBuilder, ModuleBuilder};
use crate::ir::{
    self, AttrValue, Attrs, BinaryOp, CellId, Const, Id, InstanceId, Lvalue, MemoryId, ModuleId,
    Name, NetId, NetKind, PortDir, ProcessId, ProcessKind, Stmt, StmtKind,
};
use crate::source::Span;
use crate::vhdl::ast::{self, Mode};
use crate::vhdl::sema::{Analysis, DeclId, DeclKind, ObjectClass, TypeId, Value};

use super::types::{self, Layout, LayoutEnv, LayoutKind};
use super::{Elab, GenericValue, codes};

/// Where statements produced while lowering an expression go.
pub(crate) enum Sink<'s> {
    /// Into an open procedural block.
    Proc(&'s mut BlockBuilder),
    /// Into a fresh combinational helper process (a continuous context).
    Cont,
}

/// What a VHDL declaration became.
#[derive(Clone, Debug)]
pub(crate) enum Binding {
    /// A net holding a signal or variable.
    Net {
        /// The net.
        net: NetId,
        /// Its layout.
        layout: Layout,
    },
    /// A memory holding an array of wide elements.
    Mem {
        /// The memory.
        mem: MemoryId,
        /// The layout of the whole array.
        layout: Layout,
    },
    /// A part of a net: an object alias of a slice or an element.
    Slice {
        /// The aliased net.
        net: NetId,
        /// Most significant bit of the alias.
        hi: u32,
        /// Least significant bit of the alias.
        lo: u32,
        /// The alias's layout.
        layout: Layout,
    },
    /// A constant, generic or loop parameter with a known value.
    Value {
        /// The value.
        value: Value,
        /// Its subtype.
        ty: TypeId,
    },
}

/// What kind of thing drives a net.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DriverKey {
    /// A concurrent assignment.
    Continuous(usize),
    /// A process.
    Process(usize),
    /// An instance output.
    Instance(usize),
    /// A cell output (a tristate bus driver).
    Cell(usize),
    /// An initial value, which never conflicts with a real driver.
    Initial,
}

/// Where a driver writes, so it can be moved onto a resolution net.
#[derive(Clone, Debug)]
pub(crate) enum DriverSite {
    /// The continuous assignment at this index.
    Assign(usize),
    /// Every assignment to the net inside this process.
    Process(ProcessId),
    /// A connection of this instance.
    Instance(InstanceId, Name),
    /// An output of this cell.
    Cell(CellId, Name),
    /// Nothing that can be moved (an initial value).
    Fixed,
}

/// One driver of a net.
#[derive(Clone, Debug)]
pub(crate) struct Driver {
    key: DriverKey,
    site: DriverSite,
    whole: bool,
    span: Span,
}

/// What a net was declared as.
#[derive(Clone, Debug)]
pub(crate) struct NetInfo {
    /// The declared subtype's layout.
    pub layout: Layout,
    /// True for a signal, false for a variable.
    pub is_signal: bool,
    /// True when the subtype names a resolution function.
    pub resolved: bool,
    /// Where it was declared.
    pub span: Span,
}

/// One frame of subprogram inlining.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    /// The net holding the function's result.
    pub result: Option<NetId>,
    /// The result's layout.
    pub result_layout: Option<Layout>,
    /// The net set to 1 once the subprogram has returned.
    pub flag: Option<NetId>,
}

/// Builds one IR module from one entity and architecture.
pub(crate) struct Lowerer<'a, 'e> {
    /// The shared elaboration state.
    pub cx: &'e mut Elab<'a>,
    /// The module under construction.
    pub b: ModuleBuilder,
    /// Scopes, innermost last.
    pub scopes: Vec<HashMap<DeclId, Binding>>,
    /// One entry per net, in [`NetId`] order.
    pub nets: Vec<NetInfo>,
    /// One entry per net, in [`NetId`] order.
    pub drivers: Vec<Vec<Driver>>,
    /// The hierarchical prefix generate statements and blocks add.
    pub prefix: String,
    /// Counter for generated names.
    pub counter: u32,
    /// What the statements being lowered count as.
    pub driver: DriverKey,
    /// Subprograms currently being inlined.
    pub inlining: Vec<DeclId>,
    /// One frame per inlined subprogram.
    pub frames: Vec<Frame>,
    /// Configuration specifications in scope, innermost last.
    pub configs: Vec<&'a ast::ConfigurationSpec>,
    /// Labels of loops being unrolled, innermost last.
    pub loops: Vec<Option<String>>,
    /// The generics, as IR parameters, in declaration order.
    pub params: Vec<(String, AttrValue)>,
    /// Non-zero while layouts are computed speculatively, so a type that
    /// has none is a fall-back rather than a diagnostic.
    pub quiet: u32,
    /// Non-zero while a subprogram body is being inlined: its variables
    /// are re-initialised on entry rather than once at time zero.
    pub in_subprogram: u32,
    /// Variable initialisers waiting to be emitted into the inlined body.
    pub pending_inits: Vec<(NetId, crate::ir::ExprId, Span)>,
    /// Interpreted calls currently on the stack (see [`super::interp`]).
    pub interp_depth: usize,
    /// Statements the interpretation of the outermost static call has
    /// executed, its budget against a body that never terminates.
    pub steps: u32,
}

impl<'a> LayoutEnv<'a> for Lowerer<'a, '_> {
    fn analysis(&self) -> &'a Analysis {
        self.cx.a
    }

    fn eval_bound(&mut self, span: Span) -> Option<i128> {
        let e = self.cx.ast.bound(span)?;
        self.eval(e)?.as_int()
    }

    fn report(&mut self, diag: Diagnostic) {
        if self.quiet == 0 {
            self.cx.report(diag);
        }
    }
}

/// Elaborates one entity with one architecture into a module.
pub(crate) fn lower_entity<'a>(
    cx: &mut Elab<'a>,
    entity: crate::vhdl::sema::UnitId,
    arch: crate::vhdl::sema::UnitId,
    generics: Vec<GenericValue>,
    span: Span,
    key: &str,
) -> Option<ModuleId> {
    let a = cx.a;
    let ast::LibraryUnit::Entity(edecl) = cx.unit_ast(entity)? else {
        return None;
    };
    let ast::LibraryUnit::Architecture(adecl) = cx.unit_ast(arch)? else {
        return None;
    };
    let name = a.name(a.units[entity.index()].name).to_owned();

    let mut low = Lowerer {
        cx,
        b: ModuleBuilder::new(name, span),
        scopes: vec![HashMap::new()],
        nets: Vec::new(),
        drivers: Vec::new(),
        prefix: String::new(),
        counter: 0,
        driver: DriverKey::Continuous(0),
        inlining: Vec::new(),
        frames: Vec::new(),
        configs: Vec::new(),
        loops: Vec::new(),
        params: Vec::new(),
        quiet: 0,
        in_subprogram: 0,
        pending_inits: Vec::new(),
        interp_depth: 0,
        steps: 0,
    };
    low.b.span = span;
    // VHDL's time resolution limit is a femtosecond, and `wait for`
    // carries no unit of its own in the IR, so every VHDL module says so.
    low.b.module_mut().timescale = Some(ir::Timescale {
        unit: ir::Delay::new(1, ir::TimeUnit::Fs),
        precision: ir::Delay::new(1, ir::TimeUnit::Fs),
    });

    let applied = low.bind_generics(entity, edecl, &generics);
    if let Some(d) = a.units[entity.index()].decl {
        low.apply_attributes(d, AttrTarget::Module);
    }
    low.declare_ports(entity, edecl);
    low.declarations(&edecl.decls);
    low.declarations(&adecl.decls);
    low.collect_configurations(&edecl.decls);
    low.collect_configurations(&adecl.decls);
    low.concurrent_statements(&edecl.statements);
    low.concurrent_statements(&adecl.statements);
    low.finish_drivers();

    let params = low.params.clone();
    let mut module = low.b.finish();
    for (pname, value) in params {
        module.params.push(ir::Param {
            name: Name::new(pname),
            value,
            attrs: Attrs::new(),
            span,
        });
    }
    let module_name = cx.module_name(entity, arch, &applied);
    module.name = Name::new(module_name.clone());
    Some(cx.register(key, module_name, module))
}

impl<'a> Elab<'a> {
    /// The tree of a design unit.
    pub(crate) fn unit_ast(&self, u: crate::vhdl::sema::UnitId) -> Option<&'a ast::LibraryUnit> {
        let unit = &self.a.units[u.index()];
        let file = self.a.files.get(unit.file)?;
        file.ast.units.get(unit.index).map(|d| &d.unit)
    }
}

impl<'a, 'e> Lowerer<'a, 'e> {
    /// The analysis, with the lifetime of the tree.
    pub(crate) fn a(&self) -> &'a Analysis {
        self.cx.a
    }

    // --- scopes ------------------------------------------------------------

    /// Enters a nested scope.
    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Leaves the innermost scope.
    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Binds a declaration.
    pub(crate) fn bind(&mut self, decl: DeclId, binding: Binding) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(decl, binding);
        }
    }

    /// Looks a declaration up, innermost scope first.
    pub(crate) fn lookup(&self, decl: DeclId) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(&decl))
    }

    // --- diagnostics -------------------------------------------------------

    /// Reports an error with a code and a span.
    pub(crate) fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.cx.error(code, span, msg);
    }

    /// Reports a diagnostic.
    pub(crate) fn report(&mut self, d: Diagnostic) {
        self.cx.report(d);
    }

    /// Reports an unsupported construct, naming it.
    pub(crate) fn unsupported(&mut self, span: Span, what: &str) {
        self.cx.report(
            Diagnostic::error(format!("{what} cannot be lowered to the IR yet"))
                .with_code(codes::UNSUPPORTED)
                .with_span(span),
        );
    }

    // --- names -------------------------------------------------------------

    /// The hierarchical name of `name` in the current generate prefix,
    /// made unique among the module's nets and memories.
    pub(crate) fn qualified(&mut self, name: &str) -> String {
        let base = format!("{}{}", self.prefix, name);
        self.unique(base)
    }

    /// Makes a name unique among the module's nets and memories.
    pub(crate) fn unique(&mut self, base: String) -> String {
        if !self.name_taken(&base) {
            return base;
        }
        for n in 2.. {
            let candidate = format!("{base}${n}");
            if !self.name_taken(&candidate) {
                return candidate;
            }
        }
        base
    }

    fn name_taken(&self, name: &str) -> bool {
        let m = self.b.module();
        m.net_by_name(name).is_some() || m.memory_by_name(name).is_some()
    }

    /// A fresh name unique inside the module.
    pub(crate) fn fresh(&mut self, base: &str) -> String {
        self.counter += 1;
        let name = format!("{}{base}${}", self.prefix, self.counter);
        self.unique(name)
    }

    // --- nets and memories -------------------------------------------------

    /// Creates a net for a signal or variable of the given layout.
    pub(crate) fn new_net(
        &mut self,
        name: String,
        layout: Layout,
        is_signal: bool,
        span: Span,
    ) -> Option<NetId> {
        if !layout.is_bits() {
            self.error(
                codes::TYPE,
                span,
                "a `real` or `string` object has no representation in the IR",
            );
            return None;
        }
        let resolved = self.a().resolution_function(layout.ty).is_some();
        let ty = layout.ir_type();
        self.b.span = span;
        let kind = if is_signal {
            NetKind::Wire
        } else {
            NetKind::Variable
        };
        let net = self.b.add_net_kind(name, ty, kind);
        debug_assert_eq!(net.index(), self.nets.len());
        if let Some((tyname, literals)) = layout.enum_attrs(self.a()) {
            self.b.net_attr(net, "enum_type", tyname);
            self.b.net_attr(net, "enum_literals", literals);
        }
        self.nets.push(NetInfo {
            layout,
            is_signal,
            resolved,
            span,
        });
        self.drivers.push(Vec::new());
        Some(net)
    }

    /// Creates a memory for an array of wide elements.
    pub(crate) fn new_memory(
        &mut self,
        name: String,
        layout: &Layout,
        span: Span,
    ) -> Option<MemoryId> {
        let arr = layout.array()?;
        let elem = arr.elem.ir_type();
        let size = u64::from(arr.len);
        self.b.span = span;
        let mem = self.b.memory(name, elem, size);
        if let Some((tyname, literals)) = arr.elem.enum_attrs(self.a()) {
            let m = &mut self.b.module_mut().memories[mem];
            m.attrs.set("enum_type", tyname);
            m.attrs.set("enum_literals", literals);
        }
        Some(mem)
    }

    /// The layout of `bit`.
    pub(crate) fn bit_layout(&self) -> Layout {
        Layout {
            ty: self.a().builtins.bit,
            kind: LayoutKind::Bit(super::types::BitKind::Bit),
            width: 1,
            signed: false,
        }
    }

    /// The layout of `boolean`.
    pub(crate) fn boolean_layout(&self) -> Layout {
        Layout {
            ty: self.a().builtins.boolean,
            kind: LayoutKind::Bit(super::types::BitKind::Boolean),
            width: 1,
            signed: false,
        }
    }

    /// The layout of `integer`.
    pub(crate) fn int_layout(&self) -> Layout {
        Layout {
            ty: self.a().builtins.integer,
            kind: LayoutKind::Int,
            width: 32,
            signed: true,
        }
    }

    /// The layout of a net.
    pub(crate) fn net_layout(&self, net: NetId) -> Layout {
        self.nets[net.index()].layout.clone()
    }

    // --- drivers -----------------------------------------------------------

    /// Records that something drives `net`.
    pub(crate) fn record_driver(&mut self, net: NetId, site: DriverSite, whole: bool, span: Span) {
        let key = self.driver;
        if let Some(list) = self.drivers.get_mut(net.index()) {
            list.push(Driver {
                key,
                site,
                whole,
                span,
            });
        }
    }

    /// Records every net a target writes.
    pub(crate) fn note_target(&mut self, target: &Lvalue, site: DriverSite, span: Span) {
        let whole = matches!(target, Lvalue::Net(_));
        for n in target.nets() {
            self.record_driver(n, site.clone(), whole, span);
        }
    }

    // --- generics ----------------------------------------------------------

    /// Binds the entity's generics and records them as IR parameters.
    ///
    /// Returns the generics whose value differs from their default, which
    /// is what the module's name is built from.
    fn bind_generics(
        &mut self,
        entity: crate::vhdl::sema::UnitId,
        edecl: &'a ast::EntityDecl,
        overrides: &[GenericValue],
    ) -> Vec<(String, String)> {
        let decls = self.cx.generics_of(entity);
        let objects = interface_objects(&edecl.generics);
        let mut applied = Vec::new();
        for (i, &d) in decls.iter().enumerate() {
            let spelling = self.a().decl(d).spelling.clone();
            let ty = self.a().decl_type(d);
            let default = self.a().decl_value(d).cloned().or_else(|| {
                let (_, obj) = objects.get(i)?;
                let init = obj.default.as_ref()?;
                self.eval(init)
            });
            let over = overrides.iter().find(|g| g.decl == d);
            let value = match (over, &default) {
                (Some(g), _) => Some(g.value.clone()),
                (None, Some(v)) => Some(v.clone()),
                (None, None) => None,
            };
            let Some(value) = value else {
                let span = objects.get(i).map_or(edecl.span, |(id, _)| id.span);
                self.report(
                    Diagnostic::error(format!("generic `{spelling}` has no value"))
                        .with_code(codes::GENERIC)
                        .with_span(span)
                        .with_note("give it a default or a value in the generic map"),
                );
                continue;
            };
            if let Some(ty) = ty {
                let text = value_text(self.a(), &value, ty);
                if default.as_ref() != Some(&value) {
                    applied.push((spelling.clone(), text.clone()));
                }
                self.params
                    .push((spelling, attr_of_value(self.a(), &value, ty)));
                self.bind(d, Binding::Value { value, ty });
            }
        }
        applied
    }

    // --- ports -------------------------------------------------------------

    /// Declares one net per port and exposes it.
    fn declare_ports(&mut self, entity: crate::vhdl::sema::UnitId, edecl: &'a ast::EntityDecl) {
        let decls = self.cx.ports_of(entity);
        let objects = interface_objects(&edecl.ports);
        for (i, &d) in decls.iter().enumerate() {
            let span = objects.get(i).map_or(edecl.span, |(id, _)| id.span);
            let spelling = self.a().decl(d).spelling.clone();
            let DeclKind::Object { ty, mode, .. } = self.a().decl(d).kind else {
                continue;
            };
            let dir = match mode.unwrap_or(Mode::In) {
                Mode::In => PortDir::In,
                Mode::Out | Mode::Buffer => PortDir::Out,
                Mode::Inout => PortDir::InOut,
                Mode::Linkage => {
                    self.unsupported(span, "a `linkage` port");
                    continue;
                }
            };
            let Some(layout) = types::layout_of(self, ty, span) else {
                continue;
            };
            if layout.wants_memory() {
                self.error(
                    codes::TYPE,
                    span,
                    format!(
                        "port `{spelling}` is an array of multi-bit elements, which becomes a memory and cannot cross a module boundary"
                    ),
                );
                continue;
            }
            let name = self.qualified(&spelling);
            let Some(net) = self.new_net(name.clone(), layout, true, span) else {
                continue;
            };
            self.apply_attributes(d, AttrTarget::Net(net));
            self.b.span = span;
            self.b.add_port(name, dir, net);
            self.bind(
                d,
                Binding::Net {
                    net,
                    layout: self.nets[net.index()].layout.clone(),
                },
            );
            // A port with a default value that is left open gets it as an
            // initial value; a connected one overwrites it in the parent.
            if dir == PortDir::In
                && let Some((_, obj)) = objects.get(i)
                && let Some(init) = &obj.default
            {
                self.initial_value(net, init, span);
            }
        }
    }

    // --- declarations ------------------------------------------------------

    /// Lowers a declarative part in source order.
    pub(crate) fn declarations(&mut self, decls: &'a [ast::Declaration]) {
        for d in decls {
            self.declaration(d);
        }
    }

    fn declaration(&mut self, d: &'a ast::Declaration) {
        match d {
            ast::Declaration::Object(o) => self.object_decl(o),
            ast::Declaration::File(f) => {
                self.error(
                    codes::TYPE,
                    f.span,
                    "a file object is not synthesisable and has no IR representation",
                );
            }
            ast::Declaration::Alias(al) => self.alias_decl(al),
            ast::Declaration::SubprogramInstantiation(si) => {
                self.unsupported(si.span, "a subprogram instantiation");
            }
            ast::Declaration::Disconnection(s) => {
                self.unsupported(s.span, "a disconnection specification");
            }
            ast::Declaration::Group(_)
            | ast::Declaration::GroupTemplate(_)
            | ast::Declaration::Type(_)
            | ast::Declaration::Subtype(_)
            | ast::Declaration::Attribute(_)
            | ast::Declaration::AttributeSpec(_)
            | ast::Declaration::Component(_)
            | ast::Declaration::Subprogram(_)
            | ast::Declaration::SubprogramBody(_)
            | ast::Declaration::Package(_)
            | ast::Declaration::PackageBody(_)
            | ast::Declaration::PackageInstantiation(_)
            | ast::Declaration::ConfigurationSpec(_)
            | ast::Declaration::Use(_) => {}
        }
    }

    /// An alias declaration.
    ///
    /// An *object* alias binds its designator to the part of the net the
    /// aliased name denotes, so a read or an assignment through the alias
    /// is a read or an assignment of that slice. A type, subtype or
    /// subprogram alias needs nothing: the analyser already resolved every
    /// use of it to the aliased declaration.
    fn alias_decl(&mut self, al: &'a ast::AliasDecl) {
        let ast::Designator::Ident(id) = &al.designator else {
            return;
        };
        let Some(d) = self.cx.object_decl_at(id.span) else {
            return;
        };
        let Some((lv, layout)) = self.lvalue_name(&al.target, &mut Sink::Cont) else {
            return;
        };
        match lv {
            Lvalue::Net(net) => self.bind(d, Binding::Net { net, layout }),
            Lvalue::Slice { net, hi, lo } => {
                self.bind(
                    d,
                    Binding::Slice {
                        net,
                        hi,
                        lo,
                        layout,
                    },
                );
            }
            _ => self.unsupported(al.span, "an alias of this object"),
        }
    }

    /// `constant`, `signal` or `variable`.
    fn object_decl(&mut self, o: &'a ast::ObjectDecl) {
        if o.kind == ast::ObjectKind::SharedVariable {
            self.unsupported(o.span, "a shared variable");
            return;
        }
        for name in &o.names {
            let Some(d) = self.cx.object_decl_at(name.span) else {
                continue;
            };
            let DeclKind::Object { ty, class, .. } = self.a().decl(d).kind else {
                continue;
            };
            match class {
                ObjectClass::Constant => self.constant_decl(d, ty, o, name.span),
                ObjectClass::Signal => self.storage_decl(d, ty, o, name.span, true),
                ObjectClass::Variable => self.storage_decl(d, ty, o, name.span, false),
                ObjectClass::SharedVariable | ObjectClass::File => {
                    self.unsupported(name.span, "this object class");
                }
            }
        }
    }

    /// A constant: a static value when it folds, a driven net otherwise.
    fn constant_decl(&mut self, d: DeclId, ty: TypeId, o: &'a ast::ObjectDecl, span: Span) {
        if let Some(v) = self
            .a()
            .decl_value(d)
            .cloned()
            .or_else(|| o.init.as_ref().and_then(|e| self.eval(e)))
        {
            self.bind(d, Binding::Value { value: v, ty });
            return;
        }
        // A constant whose value is not static still has one at run time;
        // it becomes a net driven by a continuous assignment.
        let Some(init) = &o.init else {
            self.error(
                codes::NOT_STATIC,
                span,
                "a deferred constant has no value here",
            );
            return;
        };
        let Some(layout) = types::layout_of(self, ty, span) else {
            return;
        };
        if !layout.is_bits() {
            // A `real` or `string` constant has no hardware value; it is
            // still usable in reports and attributes through `value_of`.
            return;
        }
        let spelling = self.a().decl(d).spelling.clone();
        let name = self.qualified(&spelling);
        let Some(net) = self.new_net(name, layout.clone(), true, span) else {
            return;
        };
        self.bind(
            d,
            Binding::Net {
                net,
                layout: layout.clone(),
            },
        );
        let value = self.expr_in(init, &layout, &mut Sink::Cont);
        self.b.span = span;
        let index = self.b.module().assigns.len();
        self.driver = DriverKey::Continuous(index);
        self.b.assign(net, value);
        self.record_driver(net, DriverSite::Assign(index), true, span);
    }

    /// A signal or variable: a net, or a memory for an array of wide
    /// elements.
    fn storage_decl(
        &mut self,
        d: DeclId,
        ty: TypeId,
        o: &'a ast::ObjectDecl,
        span: Span,
        is_signal: bool,
    ) {
        let Some(layout) = types::layout_of(self, ty, span) else {
            return;
        };
        let spelling = self.a().decl(d).spelling.clone();
        let name = self.qualified(&spelling);
        if layout.wants_memory() {
            let Some(mem) = self.new_memory(name, &layout, span) else {
                return;
            };
            self.apply_attributes(d, AttrTarget::Memory(mem));
            if let Some(init) = &o.init
                && let Some(v) = self.eval(init)
                && let Some(consts) = self.memory_init(&v, &layout)
            {
                self.b.module_mut().memories[mem].init = Some(consts);
            }
            self.bind(d, Binding::Mem { mem, layout });
            return;
        }
        let Some(net) = self.new_net(name, layout.clone(), is_signal, span) else {
            return;
        };
        self.apply_attributes(d, AttrTarget::Net(net));
        self.bind(d, Binding::Net { net, layout });
        if let Some(init) = &o.init {
            self.initial_value(net, init, span);
        }
    }

    /// The initial contents of a memory, if the initialiser folds.
    fn memory_init(&mut self, v: &Value, layout: &Layout) -> Option<Vec<Const>> {
        let arr = layout.array()?;
        let av = v.as_array()?;
        let mut out = Vec::with_capacity(av.elems.len());
        // The IR stores element 0 first; VHDL's element 0 is the one at
        // index 0 of the range, which is `offset` from the left.
        for o in 0..arr.len {
            let idx = arr.index_at(o);
            let e = av.get(i128::from(idx))?;
            out.push(self.const_of(e, &arr.elem)?);
        }
        Some(out)
    }

    /// Emits an initial value as a time-zero process, which is what a
    /// VHDL initial value is: assigned once, before simulation starts.
    pub(crate) fn initial_value(&mut self, net: NetId, init: &'a ast::Expr, span: Span) {
        let layout = self.net_layout(net);
        let outer = self.driver;
        self.driver = DriverKey::Initial;
        let value = self.expr_in(init, &layout, &mut Sink::Cont);
        if self.in_subprogram > 0 {
            // A subprogram's variables are elaborated afresh on every
            // call, so the initialiser belongs at the top of the body.
            self.pending_inits.push((net, value, span));
            self.driver = outer;
            return;
        }
        self.b.span = span;
        let mut p = self.b.process(None, ProcessKind::Initial);
        p.span = span;
        p.blocking(net, value);
        self.b.end_process(p);
        self.record_driver(net, DriverSite::Fixed, true, span);
        self.driver = outer;
    }

    // --- attributes --------------------------------------------------------

    /// Copies the attribute specifications that name `decl` onto an IR
    /// object.
    pub(crate) fn apply_attributes(&mut self, decl: DeclId, target: AttrTarget) {
        let names: Vec<String> = self.cx.ast.attributes().to_vec();
        for name in names {
            let Some(sym) = self.a().interner.get_ci(&name) else {
                continue;
            };
            let Some(v) = self.a().attribute_value(decl, sym).cloned() else {
                continue;
            };
            // The value has the *attribute's* type, not the annotated
            // object's.
            let ty = self
                .cx
                .attr_types
                .get(&sym)
                .copied()
                .unwrap_or(self.a().builtins.integer);
            let value = attr_of_value(self.a(), &v, ty);
            match target {
                AttrTarget::Net(net) => self.b.net_attr(net, name.clone(), value),
                AttrTarget::Memory(mem) => {
                    self.b.module_mut().memories[mem]
                        .attrs
                        .set(name.clone(), value);
                }
                AttrTarget::Instance(inst) => {
                    self.b.module_mut().instances[inst]
                        .attrs
                        .set(name.clone(), value);
                }
                AttrTarget::Module => self.b.attr(name.clone(), value),
            }
        }
    }

    // --- configuration specifications --------------------------------------

    /// Records the configuration specifications of a declarative part.
    pub(crate) fn collect_configurations(&mut self, decls: &'a [ast::Declaration]) {
        for d in decls {
            if let ast::Declaration::ConfigurationSpec(c) = d {
                self.configs.push(c);
            }
        }
    }

    // --- finishing ---------------------------------------------------------

    /// Sets net kinds and lowers resolution for multiply driven signals.
    fn finish_drivers(&mut self) {
        let count = self.nets.len();
        for i in 0..count {
            let net = NetId::from_index(i);
            self.resolve_net(net);
        }
        for i in 0..self.nets.len() {
            let net = NetId::from_index(i);
            let info = &self.nets[i];
            if !info.is_signal {
                continue;
            }
            let driven_by_process = self.drivers[i]
                .iter()
                .any(|d| matches!(d.site, DriverSite::Process(_) | DriverSite::Fixed));
            self.b.module_mut().nets[net].kind = if driven_by_process {
                NetKind::Reg
            } else {
                NetKind::Wire
            };
        }
    }

    /// Moves every driver of a multiply driven signal onto its own net and
    /// drives the signal with the resolution logic.
    fn resolve_net(&mut self, net: NetId) {
        let drivers = self.drivers[net.index()].clone();
        let mut keys: Vec<DriverKey> = Vec::new();
        for d in &drivers {
            if d.key == DriverKey::Initial || !d.whole {
                continue;
            }
            if !keys.contains(&d.key) {
                keys.push(d.key);
            }
        }
        if keys.len() < 2 {
            return;
        }
        let info = self.nets[net.index()].clone();
        let name = self.b.module().nets[net].name.as_str().to_owned();
        if !info.resolved {
            let mut d = Diagnostic::error(format!(
                "signal `{name}` has {} drivers but its subtype has no resolution function",
                keys.len()
            ))
            .with_code(codes::MULTIPLE_DRIVERS)
            .with_span(info.span);
            for driver in drivers
                .iter()
                .filter(|d| d.whole && d.key != DriverKey::Initial)
            {
                d = d.with_secondary(driver.span, "driven here");
            }
            self.report(d.with_note(
                "use a resolved subtype such as `std_logic`, or drive the signal from one place",
            ));
            return;
        }
        let ty = self.b.module().nets[net].ty.clone();
        let mut shadows = Vec::new();
        for (n, key) in keys.iter().copied().enumerate() {
            let sname = self.unique(format!("{name}$d{n}"));
            self.b.span = info.span;
            let shadow = self.b.add_net_kind(sname, ty.clone(), NetKind::Reg);
            self.nets.push(info.clone());
            self.drivers.push(Vec::new());
            for d in drivers.iter().filter(|d| d.key == key) {
                self.move_driver(&d.site, net, shadow);
                self.drivers[shadow.index()].push(d.clone());
            }
            shadows.push(shadow);
        }
        // Drive the signal with the resolution of its drivers.
        let width = ty.width().unwrap_or(1);
        self.b.span = info.span;
        let mut acc = self.b.net(shadows[0]);
        for &s in &shadows[1..] {
            let next = self.b.net(s);
            let zs = self.b.constant(Const::z(width));
            let acc_is_z = self.b.binary(BinaryOp::CaseEq, acc, zs);
            let next_is_z = self.b.binary(BinaryOp::CaseEq, next, zs);
            let xs = self.b.constant(Const::x(width));
            let conflict = self.b.mux(next_is_z, acc, xs);
            acc = self.b.mux(acc_is_z, next, conflict);
        }
        let index = self.b.module().assigns.len();
        self.b.assign(net, acc);
        self.drivers[net.index()].retain(|d| d.key == DriverKey::Initial);
        self.drivers[net.index()].push(Driver {
            key: DriverKey::Continuous(index),
            site: DriverSite::Assign(index),
            whole: true,
            span: info.span,
        });
    }

    /// Repoints one driver from `from` to `to`.
    fn move_driver(&mut self, site: &DriverSite, from: NetId, to: NetId) {
        match site {
            DriverSite::Assign(i) => {
                if let Some(a) = self.b.module_mut().assigns.get_mut(*i) {
                    retarget_lvalue(&mut a.target, from, to);
                }
            }
            DriverSite::Process(p) => {
                let mut body = std::mem::take(&mut self.b.module_mut().processes[*p].body);
                retarget_block(&mut body, from, to);
                self.b.module_mut().processes[*p].body = body;
            }
            DriverSite::Cell(cell, port) => {
                if let Some(slot) = self.b.module_mut().cells[*cell]
                    .outputs
                    .iter_mut()
                    .find(|(n, _)| n == port)
                {
                    slot.1 = to;
                }
            }
            DriverSite::Instance(inst, port) => {
                self.b.span = self.b.module().instances[*inst].span;
                let e = self.b.net(to);
                if let Some(slot) = self.b.module_mut().instances[*inst]
                    .connections
                    .iter_mut()
                    .find(|(n, _)| n == port)
                {
                    slot.1 = e;
                }
            }
            DriverSite::Fixed => {}
        }
    }
}

/// Which IR object an attribute specification applies to.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AttrTarget {
    /// A net.
    Net(NetId),
    /// A memory.
    Memory(MemoryId),
    /// An instance.
    Instance(InstanceId),
    /// The module itself.
    Module,
}

/// Replaces `from` with `to` in an assignment target.
fn retarget_lvalue(lv: &mut Lvalue, from: NetId, to: NetId) {
    match lv {
        Lvalue::Net(n) | Lvalue::Slice { net: n, .. } | Lvalue::Index { net: n, .. } => {
            if *n == from {
                *n = to;
            }
        }
        Lvalue::Concat(parts) => {
            for p in parts {
                retarget_lvalue(p, from, to);
            }
        }
        Lvalue::MemElem { .. } => {}
    }
}

/// Replaces `from` with `to` in every assignment target of a block.
fn retarget_block(block: &mut [Stmt], from: NetId, to: NetId) {
    for s in block.iter_mut() {
        if let StmtKind::Assign { target, .. } = &mut s.kind {
            retarget_lvalue(target, from, to);
        }
        if let StmtKind::For { init, step, .. } = &mut s.kind {
            if let Some((t, _)) = init {
                retarget_lvalue(t, from, to);
            }
            if let Some((t, _)) = step {
                retarget_lvalue(t, from, to);
            }
        }
        for b in s.blocks_mut() {
            retarget_block(b, from, to);
        }
    }
}

/// The identifiers of an interface list, each with the object it belongs
/// to, flattened so `generic (a, b : integer)` yields two entries.
pub(crate) fn interface_objects(
    list: &[ast::InterfaceDecl],
) -> Vec<(&ast::Ident, &ast::InterfaceObject)> {
    let mut out = Vec::new();
    for i in list {
        if let ast::InterfaceDecl::Object(o) = i {
            for n in &o.names {
                out.push((n, o.as_ref()));
            }
        }
    }
    out
}

/// The IR attribute value of a static VHDL value.
pub(crate) fn attr_of_value(a: &Analysis, v: &Value, ty: TypeId) -> AttrValue {
    match v {
        Value::Int(i) => {
            i64::try_from(*i).map_or_else(|_| AttrValue::String(i.to_string()), AttrValue::Int)
        }
        Value::Real(r) => AttrValue::String(format!("{r}")),
        // `boolean` is what `(* keep *)` and friends are written with, and
        // the IR spells those flags as 0 and 1.
        Value::Enum(p) if a.is_boolean(ty) => AttrValue::Int(i64::from(*p)),
        _ => AttrValue::String(value_text(a, v, ty)),
    }
}

/// A static value as plain text: the characters of a string or bit-string
/// value, and the analyser's rendering for everything else.
pub(crate) fn value_text(a: &Analysis, v: &Value, ty: TypeId) -> String {
    // Only `character` itself has a position that is a code point:
    // `std_ulogic` and `bit` are enumerations of character literals too, and
    // mapping their positions to characters spelled `'1'` as U+0003.
    if let Value::Array(arr) = v
        && a.element_type(ty)
            .is_some_and(|e| a.base_type(e) == a.builtins.character)
        && let Some(text) = arr
            .elems
            .iter()
            .map(|e| char::from_u32(e.as_enum()?))
            .collect::<Option<String>>()
    {
        return text;
    }
    let text = a.describe_value(v, ty);
    // An array is described as the VHDL literal `"1111"`; the quotes are
    // the literal's, not part of the value an IR parameter carries.
    match v {
        Value::Array(_) => text.trim_matches('"').to_owned(),
        _ => text,
    }
}

/// Follows a chain of non-object aliases to the declaration it names.
pub(crate) fn resolve_alias(a: &Analysis, mut d: DeclId) -> DeclId {
    for _ in 0..16 {
        match a.decl(d).kind {
            DeclKind::Alias(target) => d = target,
            _ => break,
        }
    }
    d
}

impl<'a> Elab<'a> {
    /// The object declaration whose designator is at `span`.
    pub(crate) fn object_decl_at(&self, span: Span) -> Option<DeclId> {
        self.decl_at
            .get(&span)?
            .iter()
            .copied()
            .find(|&d| matches!(self.a.decl(d).kind, DeclKind::Object { .. }))
    }
}
