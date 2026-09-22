//! Lowering an elaborated module to [`crate::ir::Module`].
//!
//! This is where the AST becomes IR. One [`Lowerer`] builds one module
//! variant: it owns an [`crate::ir::builder::ModuleBuilder`] and a [`ScopeEnv`]
//! positioned in the module's scope, and walks the module body in three
//! passes.
//!
//! # Passes
//!
//! 1. **Subroutines and genvars.** Functions and tasks are recorded so a
//!    parameter declared later can call them.
//! 2. **Declarations, in source order.** Parameters (with the
//!    instantiation's overrides applied), typedefs, nets, variables,
//!    non-ANSI port declarations and `` `default_nettype `` directives. A
//!    one-dimensional unpacked variable becomes an [`crate::ir::Memory`], every
//!    other declaration becomes an [`crate::ir::Net`] whose [`crate::ir::NetKind`] is
//!    `Wire` for nets and `Reg` for variables. Ports are bound to their
//!    nets at the end of this pass, in the order the header wrote them.
//! 3. **Behaviour.** `defparam`s first (they must reach instances that
//!    appear earlier in the file), then continuous assignments, `always` /
//!    `initial` blocks, gate primitives, instances and generate regions.
//!
//! # Process kinds
//!
//! The trigger of an `always` block decides its [`crate::ir::ProcessKind`]:
//!
//! | Source                                   | Kind                          |
//! |------------------------------------------|-------------------------------|
//! | `always_comb`, `always @*`, `always @(*)`| `Comb`                        |
//! | `always_latch`                           | `Comb` (the latch is inferred later) |
//! | `always_ff @(posedge clk)`, `always @(posedge clk or negedge rst)` | `Sequential { clocks, resets }` |
//! | `always @(a or b)` (no edges)            | `Sensitive`                   |
//! | `always` with no timing control          | `Free`                        |
//! | `initial`                                | `Initial`                     |
//!
//! An edge in the trigger list is a *reset* when the body tests its net in
//! the leading `if` / `else if` chain, which is the standard way an
//! asynchronous reset is written; every other edge is a clock. A block
//! whose body contains a delay, an event control or a `wait` is `Free`
//! whatever its header says, since it controls its own timing.
//!
//! # Uniquification
//!
//! A module instantiated with parameter overrides is elaborated once per
//! distinct override set and gets a name built from the base name and the
//! overrides: `fifo$WIDTH_8$DEPTH_4`. Values are sanitised to the
//! characters a bare IR name allows (`-1` becomes `m1`, other characters
//! become `_`); a collision appends `$2`, `$3` and so on. A module with no
//! overrides keeps its own name.

mod expr;
mod inst;
mod proc;
mod stmt;

use crate::diag::{Diagnostic, Diagnostics, Severity};
use crate::ir::builder::ModuleBuilder;
use crate::ir::{
    self, AttrValue, Attrs, Delay, Design, MemoryId, ModuleId, NetId, NetKind, PortDir,
    ProcessKind, Type,
};
use crate::source::Span;
use crate::verilog::ast::{self, Item, ItemKind};
use crate::verilog::token::Dialect;

use super::ElabOptions;
use super::codes;
use super::constant::{Evaluator, Value};
use super::decls::{self, Override, Overrides};
use super::env::{Context, ScopeEnv};
use super::hier::{ModuleDecl, Table};
use super::package;
use super::scope::Symbol;
use super::types::{self, Packed, Range, VType};
use super::width;

/// What an IR net was declared as, beside its type.
#[derive(Clone, Debug)]
struct NetInfo {
    /// The declared type.
    ty: VType,
    /// True for a variable (`reg`, `logic`, `int`), false for a net.
    is_var: bool,
    /// Where it was declared.
    span: Span,
}

/// A memory: a one-dimensional unpacked array.
#[derive(Clone, Debug)]
struct MemInfo {
    /// The element type.
    elem: VType,
    /// The declared dimension.
    range: Range,
}

/// What drives a net, for the multiple-driver check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DriverKey {
    /// A continuous assignment or a gate.
    Continuous,
    /// The process with this index.
    Process(usize),
    /// An `initial` block or a declaration initialiser. These never
    /// conflict with anything: an initial value plus a driver is how every
    /// testbench clock is written.
    Initial,
    /// An instance output.
    Instance(usize),
}

/// One driver of a net.
#[derive(Clone, Copy, Debug)]
struct Driver {
    key: DriverKey,
    /// True when the driver assigns the whole net.
    whole: bool,
    span: Span,
}

/// A port of the module under construction, before it is bound to a net.
#[derive(Clone, Debug)]
struct PortSlot {
    /// The port name seen from outside.
    name: String,
    /// The direction.
    dir: PortDir,
    /// The net it exposes; `None` until the declaration is seen.
    net: Option<NetId>,
    /// Where it was declared.
    span: Span,
}

/// An interface port of a module, flattened into one IR port per signal.
#[derive(Clone, Debug)]
pub(crate) struct IfacePort {
    /// The port name as written (`bus`).
    pub name: String,
    /// The signals, as `(signal name, IR port name)` pairs in order.
    pub signals: Vec<(String, String)>,
}

/// Builds one IR module from one AST module.
pub(crate) struct Lowerer<'cx, 'ast> {
    env: ScopeEnv<'cx, 'ast>,
    b: ModuleBuilder,
    nets: Vec<NetInfo>,
    mems: Vec<MemInfo>,
    drivers: Vec<Vec<Driver>>,
    ports: Vec<PortSlot>,
    iface_ports: Vec<IfacePort>,
    /// `None` once `` `default_nettype none `` has been seen.
    default_nettype: Option<ast::NetType>,
    /// Counter for generated names (helper processes, inline frames).
    counter: u32,
    /// What the statements being lowered count as, for driver
    /// bookkeeping.
    driver: DriverKey,
    /// `defparam` overrides for instances of this module, by instance name.
    defparams: Vec<(Vec<String>, &'ast ast::Expr, Span)>,
    /// Subroutines currently being inlined, innermost last.
    inlining: Vec<String>,
    /// One entry per inlined subroutine: its result net and its "has
    /// returned" flag, when it needs one.
    inline_frames: Vec<(Option<NetId>, Option<NetId>)>,
}

/// Elaborates every root module of `files` and returns the design.
///
/// Diagnostics go to `diags`; `None` means an error was reported and no
/// usable design was produced. The returned design has passed
/// [`crate::ir::validate`]; a failure there is a bug in this frontend and is
/// reported as an internal error (`V0024`).
pub(crate) fn lower_all(
    files: &[(&ast::SourceFile, Dialect)],
    opts: &ElabOptions,
    diags: &mut Diagnostics,
) -> Option<Design> {
    let table = Table::collect(files, diags);
    let mut cx = Context::new(table, opts.clone());
    cx.diags.append(diags);
    package::elaborate_unit(&mut cx);

    // Which modules to elaborate as roots, and which is the top.
    let roots = cx.table.roots();
    let top_name = match &opts.top {
        Some(name) => {
            if !cx.table.modules.contains_key(name) {
                cx.diags.push(
                    Diagnostic::error(format!("no module named `{name}` to use as the top"))
                        .with_code(codes::TOP)
                        .with_note(if cx.table.order.is_empty() {
                            "the source declares no modules".to_owned()
                        } else {
                            format!("the source declares: {}", cx.table.order.join(", "))
                        }),
                );
                *diags = cx.diags;
                return None;
            }
            Some(name.clone())
        }
        None => roots.first().cloned(),
    };
    if top_name.is_none() && cx.table.modules.is_empty() {
        cx.diags.push(
            Diagnostic::warning("the source declares no modules")
                .with_code(codes::TOP)
                .with_note("elaboration produced an empty design"),
        );
    }

    // Every root is elaborated with its default parameters, so a file of
    // independent modules yields a design holding all of them.
    let mut order: Vec<String> = Vec::new();
    if let Some(top) = &top_name {
        order.push(top.clone());
    }
    for r in &roots {
        if !order.contains(r) {
            order.push(r.clone());
        }
    }
    let mut top_id = None;
    for name in &order {
        let params = if Some(name) == top_name.as_ref() {
            top_overrides(&mut cx, name, opts)
        } else {
            Overrides::new()
        };
        let span = cx.table.modules[name].span();
        let id = elaborate_module(&mut cx, name, params, span);
        if Some(name) == top_name.as_ref() {
            top_id = id;
        }
    }
    cx.design.top = top_id;

    let mut design = cx.design;
    let mut out = cx.diags;
    if out.has_errors() {
        design.top = None;
        *diags = out;
        return None;
    }
    let mut problems = ir::validate::validate(&design);
    if !problems.is_empty() {
        for d in problems.iter() {
            out.push(
                Diagnostic::error(format!(
                    "internal error: lowered IR is invalid: {}",
                    d.message
                ))
                .with_code(codes::INTERNAL)
                .with_note("this is a bug in the Verilog frontend, not in the source"),
            );
        }
        problems = Diagnostics::new();
        drop(problems);
        *diags = out;
        return None;
    }
    *diags = out;
    Some(design)
}

/// Turns the caller's `--top` parameter overrides into [`Overrides`].
fn top_overrides(cx: &mut Context<'_>, module: &str, opts: &ElabOptions) -> Overrides {
    let mut over = Overrides::new();
    if opts.params.is_empty() {
        return over;
    }
    let span = cx.table.modules[module].span();
    for (name, text) in &opts.params {
        match crate::logic::Logic::parse_verilog(text) {
            Ok(l) => over.push(name.clone(), Override::Value(Value::Logic(l)), span),
            Err(_) => over.push(
                name.clone(),
                Override::Value(Value::Str(text.clone())),
                span,
            ),
        }
    }
    over
}

/// The name of the variant of `module` produced by `overrides`.
fn variant_name(cx: &Context<'_>, module: &str, applied: &[(String, Override)]) -> String {
    if applied.is_empty() {
        return module.to_owned();
    }
    let mut name = String::from(module);
    for (param, value) in applied {
        name.push('$');
        name.push_str(&sanitise(param));
        name.push('_');
        name.push_str(&sanitise(&value.key_text()));
    }
    if !cx.module_names.contains(&name) {
        return name;
    }
    // Same spelling, different values: append a counter.
    for n in 2.. {
        let candidate = format!("{name}${n}");
        if !cx.module_names.contains(&candidate) {
            return candidate;
        }
    }
    name
}

/// Maps a parameter value's text to the characters a bare IR name allows.
fn sanitise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' => out.push(c),
            '-' if i == 0 => out.push('m'),
            _ => out.push('_'),
        }
    }
    out
}

/// The key identifying one parameter set of one module.
fn variant_key(module: &str, overrides: &Overrides) -> String {
    let mut applied: Vec<String> = overrides
        .entries()
        .map(|(n, v, ..)| format!("{n}={}", v.key_text()))
        .collect();
    applied.sort();
    format!("{module}#{}", applied.join(","))
}

/// Elaborates `module` with `overrides`, reusing an existing variant.
pub(crate) fn elaborate_module(
    cx: &mut Context<'_>,
    module: &str,
    mut overrides: Overrides,
    inst_span: Span,
) -> Option<ModuleId> {
    let key = variant_key(module, &overrides);
    if let Some(&id) = cx.cache.get(&key) {
        return Some(id);
    }
    let decl = cx.table.modules.get(module).copied()?;
    if cx.in_progress.contains(&key) {
        cx.diags.push(
            Diagnostic::error(format!("module `{module}` instantiates itself"))
                .with_code(codes::HIERARCHY)
                .with_label(inst_span, "recursive instantiation")
                .with_secondary(decl.span(), "module declared here"),
        );
        return None;
    }
    cx.in_progress.push(key.clone());

    // Positional overrides are matched to the parameters in declaration
    // order before anything is evaluated.
    name_positional(cx, &decl, &mut overrides);

    let scope = cx.scopes.push(Some(cx.unit), "");
    let dialect = decl.dialect;
    let mut low = {
        let mut env = ScopeEnv::new(cx, scope, dialect);
        // The module's own name resolves inside it, so `$dumpvars(0, tb)`
        // and hierarchical references to it work.
        env.declare(
            module,
            Symbol::Instance {
                module: module.to_owned(),
            },
            decl.span(),
        );
        let b = ModuleBuilder::new(module, decl.span());
        Lowerer {
            env,
            b,
            nets: Vec::new(),
            mems: Vec::new(),
            drivers: Vec::new(),
            ports: Vec::new(),
            iface_ports: Vec::new(),
            default_nettype: Some(ast::NetType::Wire),
            counter: 0,
            driver: DriverKey::Continuous,
            defparams: Vec::new(),
            inlining: Vec::new(),
            inline_frames: Vec::new(),
        }
    };
    low.b.span = decl.span();
    if let Some((uv, uu, pv, pu)) = decl.timescale {
        low.b.module_mut().timescale = Some(ir::Timescale {
            unit: Delay::new(uv, uu),
            precision: Delay::new(pv, pu),
        });
    }
    for attr in &decl.item.attrs {
        let value = low.attr_value(attr);
        low.b.attr(attr.name.name.clone(), value);
    }
    let params = low.run(decl.def, &mut overrides);
    decls::report_unused_overrides(&mut low.env, module, &overrides);
    let applied = overrides.applied();
    let iface_ports = low.iface_ports.clone();
    let mut ir_module = low.b.finish();

    let cx = low.env.cx;
    let name = variant_name(cx, module, &applied);
    ir_module.name = ir::Name::new(name.clone());
    for (param, value) in params {
        ir_module.params.push(ir::Param {
            name: ir::Name::new(param),
            value: attr_of_value(&value),
            attrs: Attrs::new(),
            span: decl.span(),
        });
    }
    cx.module_names.insert(name);
    let id = cx.design.add_module(ir_module);
    cx.cache.insert(key.clone(), id);
    cx.iface_ports.insert(id, iface_ports);
    cx.in_progress.retain(|k| k != &key);
    Some(id)
}

/// Gives positional parameter overrides the names of the parameters they
/// override, in declaration order.
fn name_positional(cx: &mut Context<'_>, decl: &ModuleDecl<'_>, overrides: &mut Overrides) {
    let mut names: Vec<String> = Vec::new();
    if let Some(params) = &decl.def.params {
        for pd in params {
            if pd.kind == ast::ParamKind::Parameter {
                names.extend(pd.decls.iter().map(|d| d.name.name.clone()));
            }
        }
    }
    if names.is_empty() {
        for item in &decl.def.items {
            if let ItemKind::Param(pd) = &item.kind
                && pd.kind == ast::ParamKind::Parameter
            {
                names.extend(pd.decls.iter().map(|d| d.name.name.clone()));
            }
        }
    }
    let renamed = overrides.name_positional(&names);
    for (span, index) in renamed {
        cx.diags.push(
            Diagnostic::error(format!(
                "module `{}` has {} parameter(s); there is no parameter {} to override",
                decl.def.name.name,
                names.len(),
                index + 1
            ))
            .with_code(codes::PARAM_NOT_FOUND)
            .with_span(span),
        );
    }
}

/// The IR attribute value of an elaborated constant.
fn attr_of_value(v: &Value) -> AttrValue {
    match v {
        Value::Logic(l) => AttrValue::Const(l.clone()),
        Value::Str(s) => AttrValue::String(s.clone()),
        Value::Real(r) => AttrValue::String(format!("{r}")),
        Value::Array(_) => AttrValue::String(v.key_text()),
    }
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    /// Runs the three passes over a module body and returns the resolved
    /// parameters in declaration order.
    fn run(&mut self, m: &'ast ast::Module, overrides: &mut Overrides) -> Vec<(String, Value)> {
        let mut params: Vec<(String, Value)> = Vec::new();
        decls::import(&mut self.env, &m.imports);

        // Header parameter list.
        if let Some(list) = &m.params {
            for pd in list {
                params.extend(decls::param(&mut self.env, pd, overrides));
            }
        }

        // Pass 1: subroutines and genvars, so declarations may call them.
        self.declare_subroutines(&m.items);

        // Port list.
        match &m.ports {
            ast::Ports::None => {}
            ast::Ports::Ansi(ports) => self.ansi_ports(ports),
            ast::Ports::NonAnsi(ports) => self.non_ansi_ports(ports),
        }

        // Pass 2: declarations.
        self.declarations(&m.items, overrides, &mut params);
        self.bind_ports();

        // Pass 3: behaviour.
        self.collect_defparams(&m.items);
        self.items(&m.items);
        params
    }

    // --- naming helpers ----------------------------------------------------

    /// A fresh name unique inside the module.
    fn fresh(&mut self, base: &str) -> String {
        self.counter += 1;
        format!("{base}${}", self.counter)
    }

    /// The IR name of `name` declared in the current scope.
    fn qualified(&self, name: &str) -> String {
        self.env.qualified(name)
    }

    // --- nets --------------------------------------------------------------

    /// Creates an IR net of the given declared type.
    fn new_net(&mut self, name: String, ty: VType, is_var: bool, span: Span) -> Option<NetId> {
        let Some(irt) = ty.ir_type() else {
            self.env.error(
                codes::TYPE,
                span,
                format!("`{ty}` has no representation in the IR"),
            );
            return None;
        };
        if matches!(irt, Type::Array { .. }) {
            self.env.error(
                codes::TYPE,
                span,
                "an unpacked array must be declared as a memory",
            );
            return None;
        }
        self.b.span = span;
        let kind = if is_var { NetKind::Reg } else { NetKind::Wire };
        let net = self.b.add_net_kind(name, irt, kind);
        debug_assert_eq!(net.index(), self.nets.len());
        self.nets.push(NetInfo { ty, is_var, span });
        self.drivers.push(Vec::new());
        Some(net)
    }

    /// Creates an IR memory for a one-dimensional unpacked array.
    fn new_memory(&mut self, name: String, ty: &VType, span: Span) -> Option<MemoryId> {
        let VType::Unpacked { elem, range } = ty else {
            return None;
        };
        if matches!(**elem, VType::Unpacked { .. }) {
            self.env
                .unsupported(span, "multi-dimensional unpacked array");
            return None;
        }
        let Some(elem_ir) = elem.ir_type() else {
            self.env.error(
                codes::TYPE,
                span,
                format!("`{elem}` has no representation in the IR"),
            );
            return None;
        };
        self.b.span = span;
        let mem = self.b.memory(name, elem_ir, range.len());
        debug_assert_eq!(mem.index(), self.mems.len());
        self.mems.push(MemInfo {
            elem: (**elem).clone(),
            range: *range,
        });
        Some(mem)
    }

    /// The declared type of a net.
    fn net_type(&self, net: NetId) -> &VType {
        &self.nets[net.index()].ty
    }

    /// The width of a net.
    fn net_width(&self, net: NetId) -> u32 {
        self.net_type(net).packed().map_or(1, Packed::width)
    }

    /// Records a driver of `net`, reporting a conflict.
    fn record_driver(&mut self, net: NetId, key: DriverKey, whole: bool, span: Span) {
        let Some(existing) = self.drivers.get(net.index()) else {
            return;
        };
        if key == DriverKey::Initial {
            self.drivers[net.index()].push(Driver { key, whole, span });
            return;
        }
        let conflict = existing
            .iter()
            .filter(|d| d.key != DriverKey::Initial)
            .find(|d| d.key != key && (d.whole || whole))
            .copied();
        if let Some(first) = conflict {
            let name = self.b.module().nets[net].name.clone();
            let d = Diagnostic::error(format!("`{name}` is driven from more than one place"))
                .with_code(codes::MULTIPLE_DRIVERS)
                .with_label(span, describe_driver(key))
                .with_secondary(first.span, describe_driver(first.key))
                .with_note("a signal must have exactly one driver");
            self.env.report(d);
        }
        self.drivers[net.index()].push(Driver { key, whole, span });
    }

    /// Checks that a procedural assignment targets a variable and a
    /// continuous one a net, per the dialect's rules.
    fn check_assignable(&mut self, net: NetId, procedural: bool, span: Span) {
        let info = &self.nets[net.index()];
        let is_var = info.is_var;
        let decl_span = info.span;
        let name = self.b.module().nets[net].name.clone();
        if procedural && !is_var {
            self.env.report(
                Diagnostic::error(format!(
                    "`{name}` is a net and cannot be assigned from a procedural block"
                ))
                .with_code(codes::PROC_ASSIGN_NET)
                .with_label(span, "procedural assignment")
                .with_secondary(decl_span, "declared as a net here")
                .with_note("declare it as `reg` or `logic` to assign it here"),
            );
        } else if !procedural && is_var {
            // SystemVerilog allows a continuous assignment to a variable;
            // Verilog-2005 does not.
            let sev = if self.env.dialect == Dialect::SystemVerilog {
                return;
            } else {
                Severity::Error
            };
            self.env.report_at(
                sev,
                codes::CONT_ASSIGN_VAR,
                span,
                format!("`{name}` is a variable and cannot be continuously assigned"),
            );
        }
    }

    // --- ports -------------------------------------------------------------

    /// Declares the ports of an ANSI header.
    fn ansi_ports(&mut self, ports: &'ast [ast::Port]) {
        let mut last_dir = ast::Direction::Input;
        let mut last_type: Option<&'ast ast::DataType> = None;
        let mut last_net: Option<ast::NetType> = None;
        let mut last_var = false;
        for p in ports {
            let dir = p.direction.unwrap_or(last_dir);
            last_dir = dir;
            let inherits = p.data_type.is_empty() && p.direction.is_none() && !p.var;
            let data_type = if inherits {
                last_type.unwrap_or(&p.data_type)
            } else {
                &p.data_type
            };
            if !inherits {
                last_type = Some(data_type);
                last_net = p.net_type;
                last_var = p.var;
            }
            let net_type = if inherits { last_net } else { p.net_type };
            let var = if inherits { last_var } else { p.var };
            self.declare_port(p, dir, data_type, net_type, var);
        }
    }

    /// Declares one ANSI port.
    fn declare_port(
        &mut self,
        p: &'ast ast::Port,
        dir: ast::Direction,
        data_type: &'ast ast::DataType,
        net_type: Option<ast::NetType>,
        var: bool,
    ) {
        let Some(ty) = types::resolve(&mut self.env, data_type, &p.dims, true) else {
            return;
        };
        if let VType::Interface { name, modport } = &ty {
            self.interface_port(&p.name, name, modport.as_deref(), dir_of(dir), p.span);
            return;
        }
        let ir_dir = dir_of(dir);
        let is_var = var
            || (net_type.is_none() && ir_dir != PortDir::In && self.is_variable_type(data_type));
        if let VType::Unpacked { .. } = ty {
            self.env.unsupported(p.span, "an unpacked array as a port");
            return;
        }
        let ir_name = self.qualified(&p.name.name);
        let Some(net) = self.new_net(ir_name, ty.clone(), is_var, p.name.span) else {
            return;
        };
        for attr in &p.attrs {
            let value = self.attr_value(attr);
            self.b.net_attr(net, attr.name.name.clone(), value);
        }
        self.env
            .declare(&p.name.name, Symbol::Net { net, ty }, p.name.span);
        self.ports.push(PortSlot {
            name: p.name.name.clone(),
            dir: ir_dir,
            net: Some(net),
            span: p.name.span,
        });
    }

    /// Records the ports of a Verilog-1995 header; the declarations in the
    /// body supply the directions and types.
    fn non_ansi_ports(&mut self, ports: &'ast [ast::NonAnsiPort]) {
        for p in ports {
            let Some(expr) = &p.expr else {
                continue;
            };
            match &expr.kind {
                ast::ExprKind::Ident(id) => self.ports.push(PortSlot {
                    name: p.name.as_ref().map_or(id.name.clone(), |n| n.name.clone()),
                    dir: PortDir::In,
                    net: None,
                    span: id.span,
                }),
                _ => self.env.unsupported(
                    expr.span,
                    "a port expression that is not a simple name (use one port per signal)",
                ),
            }
        }
    }

    /// Binds every port to its net, reporting ports with no declaration.
    fn bind_ports(&mut self) {
        let slots = std::mem::take(&mut self.ports);
        for slot in &slots {
            let net = match slot.net {
                Some(net) => Some(net),
                None => match self.env.lookup(&slot.name) {
                    Some((Symbol::Net { net, .. }, _, _)) => Some(*net),
                    _ => None,
                },
            };
            let Some(net) = net else {
                self.env.error(
                    codes::UNDEFINED,
                    slot.span,
                    format!("port `{}` has no declaration in the module body", slot.name),
                );
                continue;
            };
            self.b.span = slot.span;
            self.b.add_port(slot.name.clone(), slot.dir, net);
        }
        self.ports = slots;
    }

    /// The direction recorded for a non-ANSI port declaration.
    fn set_port_dir(&mut self, name: &str, dir: PortDir) {
        if let Some(slot) = self.ports.iter_mut().find(|s| s.name == name) {
            slot.dir = dir;
        }
    }

    // --- declarations ------------------------------------------------------

    /// Records every function and task of the body.
    fn declare_subroutines(&mut self, items: &'ast [Item]) {
        for item in items {
            match &item.kind {
                ItemKind::Function(f) => decls::subroutine(&mut self.env, f, false),
                ItemKind::Task(t) => decls::subroutine(&mut self.env, t, true),
                ItemKind::Genvar(names) => {
                    for n in names {
                        self.env.declare(&n.name, Symbol::Genvar, n.span);
                    }
                }
                _ => {}
            }
        }
    }

    /// Pass 2: every declaration of the body, in source order.
    fn declarations(
        &mut self,
        items: &'ast [Item],
        overrides: &mut Overrides,
        params: &mut Vec<(String, Value)>,
    ) {
        for item in items {
            match &item.kind {
                ItemKind::Import(refs) => decls::import(&mut self.env, refs),
                ItemKind::Param(pd) => {
                    params.extend(decls::param(&mut self.env, pd, overrides));
                }
                ItemKind::Typedef(td) => decls::typedef(&mut self.env, td),
                ItemKind::Net(nd) => self.net_decl(nd, item),
                ItemKind::Var(vd) => self.var_decl(vd, item),
                ItemKind::Port(pd) => self.port_decl(pd, item),
                ItemKind::Directive(d) => self.directive(d),
                _ => {}
            }
        }
    }

    /// Applies a compiler directive that reached the parser.
    fn directive(&mut self, d: &ast::Directive) {
        if d.name == "default_nettype" {
            self.default_nettype = match d.args.trim() {
                "none" => None,
                "wire" | "" => Some(ast::NetType::Wire),
                "tri" => Some(ast::NetType::Tri),
                "wand" => Some(ast::NetType::Wand),
                "wor" => Some(ast::NetType::Wor),
                other => {
                    self.env.warning(
                        codes::IMPLICIT_NET,
                        d.span,
                        format!("unknown net type `{other}` in `default_nettype"),
                    );
                    Some(ast::NetType::Wire)
                }
            };
        }
    }

    /// A `wire` / `tri` / `supply0` declaration.
    fn net_decl(&mut self, nd: &'ast ast::NetDecl, item: &'ast Item) {
        if !matches!(
            nd.net_type,
            ast::NetType::Wire
                | ast::NetType::Tri
                | ast::NetType::Uwire
                | ast::NetType::Supply0
                | ast::NetType::Supply1
                | ast::NetType::Wand
                | ast::NetType::Wor
        ) {
            self.env
                .unsupported(item.span, &format!("net type `{}`", nd.net_type.as_str()));
        }
        for d in &nd.decls {
            let Some(ty) = types::resolve(&mut self.env, &nd.data_type, &d.dims, true) else {
                continue;
            };
            let Some(net) = self.declare_object(&d.name, ty, false, item) else {
                continue;
            };
            match nd.net_type {
                ast::NetType::Supply0 | ast::NetType::Supply1 => {
                    let width = self.net_width(net);
                    let bit = u64::from(nd.net_type == ast::NetType::Supply1);
                    let value = if bit == 0 {
                        crate::logic::Logic::zero(width)
                    } else {
                        crate::logic::Logic::ones(width)
                    };
                    self.b.span = d.span;
                    let c = self.b.constant(value);
                    self.b.assign(net, c);
                    self.record_driver(net, DriverKey::Continuous, true, d.span);
                }
                _ => {}
            }
            // `wire x = expr;` is a continuous assignment.
            if let Some(init) = &d.init {
                let delay = nd.delay.as_ref().and_then(|dl| self.delay_of(dl));
                self.continuous_assign_to(net, init, delay, d.span);
            }
        }
    }

    /// A `reg` / `logic` / `int` declaration.
    fn var_decl(&mut self, vd: &'ast ast::VarDecl, item: &'ast Item) {
        for d in &vd.decls {
            let Some(ty) = types::resolve(&mut self.env, &vd.data_type, &d.dims, true) else {
                continue;
            };
            if matches!(ty, VType::Event) {
                self.env
                    .unsupported(d.span, "an `event` variable (simulation only)");
                continue;
            }
            let Some(net) = self.declare_object(&d.name, ty, true, item) else {
                continue;
            };
            if let Some(init) = &d.init {
                // A variable initialiser is a time-zero assignment.
                self.initial_value(net, init, d.span);
            }
        }
    }

    /// Declares a net, variable or memory, reusing the net of a port that
    /// was declared in a Verilog-1995 header.
    fn declare_object(
        &mut self,
        name: &ast::Ident,
        ty: VType,
        is_var: bool,
        item: &'ast Item,
    ) -> Option<NetId> {
        if let VType::Unpacked { .. } = ty {
            let ir_name = self.qualified(&name.name);
            let mem = self.new_memory(ir_name, &ty, name.span)?;
            for attr in &item.attrs {
                let value = self.attr_value(attr);
                self.b.module_mut().memories[mem]
                    .attrs
                    .set(attr.name.name.clone(), value);
            }
            self.env
                .declare(&name.name, Symbol::Memory { mem, ty }, name.span);
            return None;
        }
        // A port declared in the header and given a type here keeps its net.
        if let Some((Symbol::Net { net, .. }, _, _)) = self.env.lookup(&name.name)
            && self.ports.iter().any(|p| p.name == name.name)
        {
            let net = *net;
            self.nets[net.index()].is_var |= is_var;
            if is_var {
                self.b.module_mut().nets[net].kind = NetKind::Reg;
            }
            return Some(net);
        }
        let ir_name = self.qualified(&name.name);
        let net = self.new_net(ir_name, ty.clone(), is_var, name.span)?;
        for attr in &item.attrs {
            let value = self.attr_value(attr);
            self.b.net_attr(net, attr.name.name.clone(), value);
        }
        self.env
            .declare(&name.name, Symbol::Net { net, ty }, name.span);
        Some(net)
    }

    /// A non-ANSI `input` / `output` / `inout` declaration.
    fn port_decl(&mut self, pd: &'ast ast::PortDecl, item: &'ast Item) {
        let dir = dir_of(pd.direction);
        for d in &pd.decls {
            let Some(ty) = types::resolve(&mut self.env, &pd.data_type, &d.dims, true) else {
                continue;
            };
            let is_var = pd.var
                || (pd.net_type.is_none()
                    && dir != PortDir::In
                    && self.is_variable_type(&pd.data_type));
            if self.ports.iter().all(|p| p.name != d.name.name) {
                self.env.error(
                    codes::UNDEFINED,
                    d.name.span,
                    format!("`{}` is not in the module's port list", d.name.name),
                );
                continue;
            }
            if let Some((Symbol::Net { net, .. }, _, _)) = self.env.lookup(&d.name.name) {
                // Already created by an earlier declaration of the same
                // name (`output q;` then `reg q;`).
                let net = *net;
                self.set_port_dir(&d.name.name, dir);
                if is_var {
                    self.nets[net.index()].is_var = true;
                    self.b.module_mut().nets[net].kind = NetKind::Reg;
                }
                continue;
            }
            let ir_name = self.qualified(&d.name.name);
            let Some(net) = self.new_net(ir_name, ty.clone(), is_var, d.name.span) else {
                continue;
            };
            for attr in &item.attrs {
                let value = self.attr_value(attr);
                self.b.net_attr(net, attr.name.name.clone(), value);
            }
            self.env
                .declare(&d.name.name, Symbol::Net { net, ty }, d.name.span);
            self.set_port_dir(&d.name.name, dir);
            if let Some(slot) = self.ports.iter_mut().find(|p| p.name == d.name.name) {
                slot.net = Some(net);
            }
        }
    }

    /// True for a data type that declares a variable rather than a net:
    /// an integer or real keyword, an enum, a struct, or a name that
    /// resolves to a type. An implicit type (`output [7:0] q`) is a net,
    /// as both languages prescribe.
    fn is_variable_type(&mut self, dt: &ast::DataType) -> bool {
        match &dt.kind {
            ast::DataTypeKind::Integer(_)
            | ast::DataTypeKind::Real(_)
            | ast::DataTypeKind::String
            | ast::DataTypeKind::Enum(_)
            | ast::DataTypeKind::Struct(_) => true,
            ast::DataTypeKind::Named { package, name, .. } => {
                let sym = match package {
                    Some(pkg) => {
                        let (pkg, name) = (pkg.clone(), name.clone());
                        self.env.probe(|env| env.resolve_scoped(&pkg, &name))
                    }
                    None => self.env.lookup(&name.name).map(|(s, _, _)| s.clone()),
                };
                matches!(sym, Some(Symbol::Type(_)))
            }
            _ => false,
        }
    }

    /// The IR value of a source attribute.
    fn attr_value(&mut self, attr: &ast::Attribute) -> AttrValue {
        let Some(value) = &attr.value else {
            return AttrValue::Int(1);
        };
        let v = self
            .env
            .probe(|env| Evaluator::new(env).eval_self(value).ok());
        match v {
            Some(Value::Logic(l)) => AttrValue::Const(l),
            Some(Value::Str(s)) => AttrValue::String(s),
            Some(other) => AttrValue::String(other.key_text()),
            None => AttrValue::Int(1),
        }
    }

    // --- behaviour ---------------------------------------------------------

    /// Collects every `defparam` of the body, including generate regions.
    fn collect_defparams(&mut self, items: &'ast [Item]) {
        for item in items {
            match &item.kind {
                ItemKind::Defparam(list) => {
                    for dp in list {
                        if let Some(path) = path_components(&dp.target) {
                            self.defparams.push((path, &dp.value, dp.span));
                        } else {
                            self.env.unsupported(dp.span, "this `defparam` target path");
                        }
                    }
                }
                ItemKind::Generate(items) => self.collect_defparams(items),
                _ => {}
            }
        }
    }

    /// Pass 3: the behavioural items of a module or generate block.
    fn items(&mut self, items: &'ast [Item]) {
        for item in items {
            self.b.span = item.span;
            match &item.kind {
                ItemKind::ContAssign(ca) => self.cont_assign(ca, item),
                ItemKind::Always(kind, stmt) => self.always(*kind, stmt, item),
                ItemKind::Initial(stmt) => self.initial(stmt, item),
                ItemKind::Final(stmt) => {
                    self.env.unsupported(stmt.span, "`final` block");
                }
                ItemKind::Instance(inst) => self.instantiation(inst, item),
                ItemKind::Gate(g) => self.gates(g, item),
                ItemKind::Generate(items) => self.items(items),
                ItemKind::GenIf(g) => self.gen_if(g, item.span),
                ItemKind::GenCase(g) => self.gen_case(g, item.span),
                ItemKind::GenFor(g) => self.gen_for(g, item.span),
                ItemKind::GenBlock(b) => self.gen_block(b, None),
                ItemKind::Assertion(a) => {
                    self.env
                        .unsupported(item.span, &format!("concurrent `{}`", a.kind.as_str()));
                }
                ItemKind::Modport(_) => {}
                ItemKind::Directive(d) => self.directive(d),
                ItemKind::Specify(_)
                | ItemKind::Defparam(_)
                | ItemKind::Genvar(_)
                | ItemKind::Import(_)
                | ItemKind::Export(_)
                | ItemKind::Param(_)
                | ItemKind::Typedef(_)
                | ItemKind::Net(_)
                | ItemKind::Var(_)
                | ItemKind::Port(_)
                | ItemKind::Function(_)
                | ItemKind::Task(_)
                | ItemKind::Timeunit { .. }
                | ItemKind::Timeprecision(_)
                | ItemKind::Empty => {}
                ItemKind::Module(m) => {
                    self.env
                        .unsupported(m.name.span, "a nested module declaration");
                }
                ItemKind::Package(p) => {
                    self.env
                        .unsupported(p.name.span, "a package inside a module");
                }
                ItemKind::Alias(_) => self.env.unsupported(item.span, "`alias`"),
                ItemKind::Bind(_) => self.env.unsupported(item.span, "`bind`"),
                ItemKind::Clocking(_) => self.env.unsupported(item.span, "a clocking block"),
                ItemKind::PropertyDecl(_) => self
                    .env
                    .unsupported(item.span, "a `property` or `sequence` declaration"),
                ItemKind::Table(_) => self
                    .env
                    .unsupported(item.span, "a user-defined primitive table"),
            }
        }
    }

    /// `assign lhs = rhs, ...;`
    fn cont_assign(&mut self, ca: &'ast ast::ContAssign, item: &'ast Item) {
        let delay = ca.delay.as_ref().and_then(|d| self.delay_of(d));
        for pair in &ca.assigns {
            self.b.span = pair.span;
            let Some(target) = self.lvalue(&pair.lhs, false) else {
                continue;
            };
            let width = self.lvalue_width(&target);
            let info = self.env.probe(|env| width::info(&pair.rhs, env, &[]));
            let signed = target_signed(&target, &self.nets);
            let value = self.expr_cont(&pair.rhs, Some(Ctx { width, signed }));
            if let Some(info) = info {
                let name = self.lvalue_name(&target);
                width::check_assign(&mut self.env, &name, width, &info, pair.span);
            }
            self.b.span = pair.span;
            self.b.assign_after(target.clone(), value, delay);
            let attrs = self.attrs_of(&item.attrs);
            if let Some(a) = self.b.module_mut().assigns.last_mut() {
                a.attrs = attrs;
            }
            self.note_assign_drivers(&target, DriverKey::Continuous, pair.span, false);
        }
    }

    /// A continuous assignment to one net, used by `wire x = e;`.
    fn continuous_assign_to(
        &mut self,
        net: NetId,
        rhs: &'ast ast::Expr,
        delay: Option<Delay>,
        span: Span,
    ) {
        let width = self.net_width(net);
        let signed = self.net_type(net).packed().is_some_and(|p| p.signed);
        let value = self.expr_cont(rhs, Some(Ctx { width, signed }));
        self.b.span = span;
        self.b.assign_after(net, value, delay);
        self.record_driver(net, DriverKey::Continuous, true, span);
    }

    /// A variable initialiser: an `initial` process assigning at time zero.
    fn initial_value(&mut self, net: NetId, init: &'ast ast::Expr, span: Span) {
        let width = self.net_width(net);
        let signed = self.net_type(net).packed().is_some_and(|p| p.signed);
        let value = self.expr_cont(init, Some(Ctx { width, signed }));
        self.b.span = span;
        let mut p = self.b.process(None, ProcessKind::Initial);
        p.span = span;
        p.blocking(net, value);
        self.b.end_process(p);
        self.record_driver(net, DriverKey::Initial, true, span);
    }

    /// The attributes of an item as IR attributes.
    fn attrs_of(&mut self, attrs: &[ast::Attribute]) -> Attrs {
        let mut out = Attrs::new();
        for a in attrs {
            let v = self.attr_value(a);
            out.set(a.name.name.clone(), v);
        }
        out
    }

    /// A constant delay, when it is a simple one.
    fn delay_of(&mut self, d: &ast::Delay) -> Option<Delay> {
        let first = d.values.first()?;
        self.delay_from_expr(first)
    }

    /// Converts a delay expression to an IR [`Delay`] in the module's time
    /// unit.
    fn delay_from_expr(&mut self, e: &ast::Expr) -> Option<Delay> {
        let unit = self
            .b
            .module()
            .timescale
            .map_or(ir::TimeUnit::Ns, |t| t.unit.unit);
        if let ast::ExprKind::Literal(ast::Literal::Number { text, .. }) = &e.kind
            && let Some((value, suffix)) = super::constant::time_literal(text)
            && let Some(u) = ir::TimeUnit::from_name(suffix)
        {
            // The rounding is deliberate: the IR holds whole units.
            let whole = value.round().clamp(0.0, 9.007_199_254_740_992e15);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the value is whole, non-negative and below 2^53"
            )]
            let v = whole as u64;
            return Some(Delay::new(v, u));
        }
        let v = self.env.probe(|env| Evaluator::new(env).eval_i64(e).ok())?;
        Some(Delay::new(u64::try_from(v).unwrap_or(0), unit))
    }
}

/// How a driver is described in a multiple-driver diagnostic.
fn describe_driver(key: DriverKey) -> &'static str {
    match key {
        DriverKey::Continuous => "driven here by a continuous assignment",
        DriverKey::Process(_) => "driven here by a procedural block",
        DriverKey::Initial => "given an initial value here",
        DriverKey::Instance(_) => "driven here by an instance output",
    }
}

/// The IR direction of an AST one.
fn dir_of(dir: ast::Direction) -> PortDir {
    match dir {
        ast::Direction::Input => PortDir::In,
        ast::Direction::Output => PortDir::Out,
        ast::Direction::Inout => PortDir::InOut,
        ast::Direction::Ref | ast::Direction::ConstRef => PortDir::InOut,
    }
}

/// The components of a dotted name path, if it is one.
fn path_components(e: &ast::Expr) -> Option<Vec<String>> {
    match &e.kind {
        ast::ExprKind::Ident(id) => Some(vec![id.name.clone()]),
        ast::ExprKind::Member { base, name } => {
            let mut path = path_components(base)?;
            path.push(name.name.clone());
            Some(path)
        }
        _ => None,
    }
}

/// True when the target of an assignment is signed.
fn target_signed(target: &ir::Lvalue, nets: &[NetInfo]) -> bool {
    match target {
        ir::Lvalue::Net(n) => nets
            .get(n.index())
            .and_then(|i| i.ty.packed().map(|p| p.signed))
            .unwrap_or(false),
        _ => false,
    }
}

/// The width and signedness an expression is lowered to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ctx {
    /// The target width.
    pub width: u32,
    /// The target signedness.
    pub signed: bool,
}

pub(crate) use expr::Sink;
