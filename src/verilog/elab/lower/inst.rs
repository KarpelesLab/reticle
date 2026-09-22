//! Generate regions, instances, gate primitives and interfaces.
//!
//! **Generate.** `if` and `case` generates pick a branch by constant
//! evaluation and elaborate it in place; a `for` generate is unrolled, one
//! scope per iteration, with the genvar bound to that iteration's value.
//! Each iteration's scope is named `label[i]`, so a net declared inside
//! becomes `label[3].name` and a hierarchical reference `label[3].name`
//! finds it. An unlabelled block is elaborated in the enclosing scope.
//!
//! **Instances.** A module instantiation evaluates its parameter
//! overrides, elaborates the target once per distinct override set (see
//! [`super::elaborate_module`]) and connects the ports. Positional
//! connections follow the target's port order; `.name`, `.name()` and
//! `.*` work as the standard says. An input connection is resized to the
//! port width, with a warning when that truncates; an output connection
//! must match exactly, since it has to drive a net.
//!
//! **Gates.** `and`, `or`, `xor`, `nand`, `nor`, `xnor`, `buf` and `not`
//! become continuous assignments of the expression they compute, which
//! keeps them visible to constant folding and needs no cell. `bufif` and
//! `notif` become [`crate::ir::CellKind::Tristate`] cells, since a conditional
//! `z` has no expression form. Switch primitives (`nmos`, `cmos`,
//! `tran`, ...) are not supported.
//!
//! **Interfaces.** An interface instance is flattened into the
//! instantiating module: its signals become nets named `bus.addr`, and
//! `bus.addr` in an expression resolves to that net. A module with an
//! interface port gets one IR port per signal of the interface, named
//! `bus_addr`, with the direction its modport gives (without a modport
//! every signal is `inout`). Interface tasks are inlined like any other.
//! Limits: interfaces with parameters that differ per instance are
//! elaborated per instance, generic `interface` ports are only supported
//! when the actual is an interface instance, and nested interfaces are
//! not supported.

use crate::diag::Diagnostic;
use crate::ir::{self, CellKind, ExprId, ModuleId, ModuleRef, Name, PortDir};
use crate::source::Span;
use crate::verilog::ast::{self, GateKind, Item, ItemKind};

use super::super::codes;
use super::super::constant::{Evaluator, Value};
use super::super::decls::{self, Override, Overrides};
use super::super::scope::{ScopeId, Symbol};
use super::super::types::{self, Packed, VType};
use super::super::width::{self, Info};
use super::{Ctx, DriverKey, IfacePort, Lowerer};

/// The maximum number of iterations a generate loop may run.
const MAX_ITERATIONS: i64 = 65_536;

/// What an instance connects to one port.
struct PortInfo {
    name: String,
    dir: PortDir,
    width: u32,
}

impl<'cx, 'ast> Lowerer<'cx, 'ast> {
    // --- generate ----------------------------------------------------------

    /// `if (cond) ... else ...` at item level.
    pub(super) fn gen_if(&mut self, g: &'ast ast::GenIf, span: Span) {
        let taken = self
            .env
            .probe(|env| Evaluator::new(env).eval_self(&g.cond).ok());
        let Some(value) = taken else {
            self.env.error(
                codes::NOT_CONSTANT,
                g.cond.span,
                "the condition of a generate `if` must be constant",
            );
            return;
        };
        let _ = span;
        if value.truth() == crate::logic::Bit::One {
            self.gen_block(&g.then_block, None);
        } else if let Some(e) = &g.else_block {
            self.gen_block(e, None);
        }
    }

    /// `case (expr) ... endcase` at item level.
    pub(super) fn gen_case(&mut self, g: &'ast ast::GenCase, span: Span) {
        let mut info = self
            .env
            .probe(|env| width::info(&g.expr, env, &[]))
            .unwrap_or(Info::bits(32, true));
        for arm in &g.items {
            for p in &arm.patterns {
                if let Some(i) = self.env.probe(|env| width::info(p, env, &[])) {
                    info = info.combine(&i);
                }
            }
        }
        let w = info.width().max(1);
        let subject = self
            .env
            .probe(|env| Evaluator::new(env).eval_logic(&g.expr, Some(w)).ok());
        let Some(subject) = subject.map(|l| l.resize(w)) else {
            self.env.error(
                codes::NOT_CONSTANT,
                g.expr.span,
                "the selector of a generate `case` must be constant",
            );
            return;
        };
        let _ = span;
        for arm in &g.items {
            for p in &arm.patterns {
                let value = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_logic(p, Some(w)).ok());
                let Some(value) = value.map(|l| l.resize(w)) else {
                    self.env.error(
                        codes::NOT_CONSTANT,
                        p.span,
                        "a generate `case` item must be constant",
                    );
                    continue;
                };
                if subject.case_eq(&value).truth() == crate::logic::Bit::One {
                    self.gen_block(&arm.block, None);
                    return;
                }
            }
        }
        if let Some(default) = g.items.iter().find(|a| a.patterns.is_empty()) {
            self.gen_block(&default.block, None);
        }
    }

    /// `for (genvar i = ...; ...; ...) ...`, unrolled.
    pub(super) fn gen_for(&mut self, g: &'ast ast::GenFor, span: Span) {
        let label = g
            .body
            .label
            .as_ref()
            .map(|l| l.name.clone())
            .unwrap_or_else(|| self.fresh("genblk"));
        let Some(start) = self
            .env
            .probe(|env| Evaluator::new(env).eval_i64(&g.init).ok())
        else {
            self.env.report(
                Diagnostic::error("the initial value of a generate loop must be constant")
                    .with_code(codes::NOT_CONSTANT)
                    .with_span(g.init.span),
            );
            return;
        };
        // The control scope holds the genvar while the bounds are checked.
        let control = self.env.enter_new(String::new());
        let mut value = start;
        let mut iterations: Vec<(i64, ScopeId)> = Vec::new();
        let mut count = 0i64;
        loop {
            self.bind_genvar(control, &g.var.name, value, g.var.span);
            let cond = self
                .env
                .probe(|env| Evaluator::new(env).eval_self(&g.cond).ok());
            let Some(cond) = cond else {
                self.env.report(
                    Diagnostic::error("the condition of a generate loop must be constant")
                        .with_code(codes::NOT_CONSTANT)
                        .with_span(g.cond.span),
                );
                break;
            };
            if cond.truth() != crate::logic::Bit::One {
                break;
            }
            count += 1;
            if count > MAX_ITERATIONS {
                self.env.report(
                    Diagnostic::error(format!(
                        "generate loop ran more than {MAX_ITERATIONS} iterations"
                    ))
                    .with_code(codes::LIMIT)
                    .with_span(span),
                );
                break;
            }
            let scope = self.env.enter_new(format!("{label}[{value}]."));
            self.bind_genvar(scope, &g.var.name, value, g.var.span);
            iterations.push((value, scope));
            self.gen_items(&g.body.items);
            self.env.leave();

            let Some(next) = self.step_genvar(g, value) else {
                break;
            };
            if next == value {
                self.env.report(
                    Diagnostic::error("the step of this generate loop does not change the genvar")
                        .with_code(codes::LIMIT)
                        .with_span(g.step.span),
                );
                break;
            }
            value = next;
        }
        self.env.leave();
        let scope = self.env.scope;
        self.env
            .cx
            .scopes
            .redeclare(scope, label, Symbol::GenArray(iterations), g.var.span);
    }

    /// Binds the genvar to `value` inside `scope`.
    fn bind_genvar(&mut self, scope: ScopeId, name: &str, value: i64, span: Span) {
        self.env.cx.scopes.redeclare(
            scope,
            name.to_owned(),
            Symbol::Const {
                value: Value::int(value),
                ty: None,
            },
            span,
        );
    }

    /// Evaluates the step expression of a generate loop.
    fn step_genvar(&mut self, g: &'ast ast::GenFor, current: i64) -> Option<i64> {
        match &g.step.kind {
            ast::ExprKind::Assign { rhs, .. } => {
                self.env.probe(|env| Evaluator::new(env).eval_i64(rhs).ok())
            }
            ast::ExprKind::IncDec { increment, .. } => {
                Some(if *increment { current + 1 } else { current - 1 })
            }
            _ => {
                self.env.unsupported(g.step.span, "this generate loop step");
                None
            }
        }
    }

    /// A labelled or bare generate block.
    pub(super) fn gen_block(&mut self, b: &'ast ast::GenBlock, forced: Option<&str>) {
        let label = b
            .label
            .as_ref()
            .map(|l| l.name.clone())
            .or_else(|| forced.map(str::to_owned));
        match label {
            None => self.gen_items(&b.items),
            Some(label) => {
                let scope = self.env.enter_new(format!("{label}."));
                self.gen_items(&b.items);
                self.env.leave();
                let parent = self.env.scope;
                self.env
                    .cx
                    .scopes
                    .redeclare(parent, label, Symbol::GenBlock(scope), b.span);
            }
        }
    }

    /// Declarations then behaviour, as for a module body.
    fn gen_items(&mut self, items: &'ast [Item]) {
        let mut none = Overrides::new();
        let mut params = Vec::new();
        self.declare_subroutines(items);
        self.declarations(items, &mut none, &mut params);
        self.collect_defparams(items);
        self.items(items);
    }

    // --- instances ---------------------------------------------------------

    /// One `module_name #(...) name (...), name2 (...);` item.
    pub(super) fn instantiation(&mut self, inst: &'ast ast::Instantiation, item: &'ast Item) {
        let module = inst.module.name.clone();
        if self.env.cx.table.is_interface(&module) {
            for one in &inst.instances {
                self.interface_instance(&module, inst, one);
            }
            return;
        }
        for one in &inst.instances {
            if !one.dims.is_empty() {
                self.env
                    .unsupported(one.span, "an instance array (`name [3:0] (...)`)");
                continue;
            }
            let Some(name) = &one.name else {
                self.env.unsupported(one.span, "an instance without a name");
                continue;
            };
            self.one_instance(&module, &inst.params, name, one, item);
        }
    }

    /// Elaborates the target and connects one instance.
    fn one_instance(
        &mut self,
        module: &str,
        params: &'ast [ast::ParamOverride],
        name: &ast::Ident,
        one: &'ast ast::Instance,
        item: &'ast Item,
    ) {
        let overrides = self.build_overrides(params, &name.name);
        let known = self.env.cx.table.modules.contains_key(module);
        let target = if known {
            super::elaborate_module(self.env.cx, module, overrides, name.span)
        } else {
            if !overrides.is_empty() {
                // Parameters of a black box are kept as metadata below.
            }
            self.env.report(
                Diagnostic::warning(format!("no module named `{module}` was found"))
                    .with_code(codes::BLACKBOX)
                    .with_label(name.span, "instantiated here")
                    .with_note("the instance is kept as a black box; its ports are not checked"),
            );
            None
        };
        let params_for_ir = if known {
            Vec::new()
        } else {
            self.build_overrides(params, &name.name).applied()
        };

        let ports = target.map(|id| self.port_infos(id));
        let connections = self.connections(one, ports.as_deref(), module, target, name);
        let module_ref = match target {
            Some(id) => ModuleRef::Resolved(id),
            None => ModuleRef::Unresolved(Name::new(module)),
        };
        self.b.span = one.span;
        let ir_name = self.env.qualified(&name.name);
        let id = self.b.instance(ir_name, module_ref, connections.clone());
        let attrs = self.attrs_of(&item.attrs);
        {
            let inst = &mut self.b.module_mut().instances[id];
            inst.attrs = attrs;
            for (p, v) in &params_for_ir {
                match v {
                    Override::Value(Value::Logic(l)) => inst.params.set(p.clone(), l.clone()),
                    other => inst.params.set(p.clone(), other.key_text()),
                }
            }
        }
        self.env.declare(
            &name.name,
            Symbol::Instance {
                module: module.to_owned(),
            },
            name.span,
        );
        // Record the drivers the instance's outputs create.
        if let Some(ports) = &ports {
            let index = id.index();
            for (port, expr) in &connections {
                let Some(info) = ports.iter().find(|p| p.name == port.as_str()) else {
                    continue;
                };
                if info.dir == PortDir::In {
                    continue;
                }
                let nets = self.expr_nets(*expr);
                for (net, whole) in nets {
                    self.record_driver(net, DriverKey::Instance(index), whole, one.span);
                }
            }
        }
    }

    /// The nets an instance connection drives, with whether the whole net
    /// is driven.
    fn expr_nets(&self, id: ExprId) -> Vec<(ir::NetId, bool)> {
        match &self.b.module().expr(id).kind {
            ir::ExprKind::Net(n) => vec![(*n, true)],
            ir::ExprKind::Slice { base, .. } => self
                .expr_nets(*base)
                .into_iter()
                .map(|(n, _)| (n, false))
                .collect(),
            ir::ExprKind::Concat(parts) => parts.iter().flat_map(|p| self.expr_nets(*p)).collect(),
            _ => Vec::new(),
        }
    }

    /// The ports of an elaborated module.
    fn port_infos(&self, id: ModuleId) -> Vec<PortInfo> {
        let m = self.env.cx.design.module(id);
        m.ports
            .iter()
            .map(|p| PortInfo {
                name: p.name.to_string(),
                dir: p.dir,
                width: m.nets[p.net].ty.width().unwrap_or(1),
            })
            .collect()
    }

    /// Evaluates the parameter overrides of an instantiation, including
    /// any `defparam` aimed at it.
    fn build_overrides(
        &mut self,
        params: &'ast [ast::ParamOverride],
        inst_name: &str,
    ) -> Overrides {
        let mut over = Overrides::new();
        for (i, p) in params.iter().enumerate() {
            let Some(value) = &p.value else { continue };
            let name = match &p.name {
                Some(n) => n.name.clone(),
                None => format!("${i}"),
            };
            if let Some(ty) = self.type_override(value) {
                over.push(name, Override::Type(ty), p.span);
                continue;
            }
            let Some(v) = self
                .env
                .probe(|env| Evaluator::new(env).eval_self(value).ok())
            else {
                let mut ev = Evaluator::new(&mut self.env);
                let _ = ev.eval_self(value);
                continue;
            };
            over.push(name, Override::Value(v), p.span);
        }
        let defparams: Vec<(Vec<String>, &'ast ast::Expr, Span)> = self
            .defparams
            .iter()
            .filter(|(path, ..)| path.len() >= 2 && path[path.len() - 2] == inst_name)
            .cloned()
            .collect();
        for (path, value, span) in defparams {
            let Some(v) = self
                .env
                .probe(|env| Evaluator::new(env).eval_self(value).ok())
            else {
                self.env.error(
                    codes::NOT_CONSTANT,
                    span,
                    "a `defparam` value must be constant",
                );
                continue;
            };
            over.push(path[path.len() - 1].clone(), Override::Value(v), span);
        }
        over
    }

    /// The type a parameter override denotes, when it is a type.
    fn type_override(&mut self, value: &'ast ast::Expr) -> Option<VType> {
        match &value.kind {
            ast::ExprKind::Type(dt) => types::resolve(&mut self.env, dt, &[], true),
            ast::ExprKind::Ident(_) | ast::ExprKind::Scoped { .. } => {
                match self.env.probe(|env| env.resolve_path(value)) {
                    Some(Symbol::Type(t)) => Some(t),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Builds the connection list of one instance.
    fn connections(
        &mut self,
        one: &'ast ast::Instance,
        ports: Option<&[PortInfo]>,
        module: &str,
        target: Option<ModuleId>,
        inst_name: &ast::Ident,
    ) -> Vec<(Name, ExprId)> {
        let mut out: Vec<(Name, ExprId)> = Vec::new();
        let mut connected: Vec<String> = Vec::new();
        let mut wildcard = false;
        for (i, conn) in one.conns.iter().enumerate() {
            match &conn.kind {
                ast::PortConnKind::Wildcard => wildcard = true,
                ast::PortConnKind::Positional(expr) => {
                    let port = match ports {
                        Some(ports) => match ports.get(i) {
                            Some(p) => p.name.clone(),
                            None => {
                                self.env.report(
                                    Diagnostic::error(format!(
                                        "module `{module}` has {} port(s); there is no port {}",
                                        ports.len(),
                                        i + 1
                                    ))
                                    .with_code(codes::PORT_NOT_FOUND)
                                    .with_span(conn.span),
                                );
                                continue;
                            }
                        },
                        None => format!("p{i}"),
                    };
                    let Some(expr) = expr else {
                        connected.push(port);
                        continue;
                    };
                    if let Some(id) = self.connect(&port, expr, ports, conn.span, &mut out) {
                        out.push((Name::new(port.clone()), id));
                    }
                    connected.push(port);
                }
                ast::PortConnKind::Named { name, conn: c } => {
                    let port = name.name.clone();
                    if ports.is_some_and(|ps| !ps.iter().any(|p| p.name == port))
                        && !self.is_iface_port(target, &port)
                    {
                        self.env.report(
                            Diagnostic::error(format!("module `{module}` has no port `{port}`"))
                                .with_code(codes::PORT_NOT_FOUND)
                                .with_label(name.span, "no such port")
                                .with_note(match ports {
                                    Some(ps) => format!(
                                        "its ports are: {}",
                                        ps.iter()
                                            .map(|p| p.name.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    ),
                                    None => String::new(),
                                }),
                        );
                        continue;
                    }
                    connected.push(port.clone());
                    match c {
                        ast::NamedConn::Open => {
                            self.warn_unconnected(&port, ports, name.span, module);
                        }
                        ast::NamedConn::Implicit => {
                            let implicit =
                                ast::Expr::new(ast::ExprKind::Ident(name.clone()), name.span);
                            let leaked: &'ast ast::Expr = Box::leak(Box::new(implicit));
                            if let Some(id) =
                                self.connect(&port, leaked, ports, name.span, &mut out)
                            {
                                out.push((Name::new(port), id));
                            }
                        }
                        ast::NamedConn::Expr(e) => {
                            if let Some(id) = self.connect(&port, e, ports, conn.span, &mut out) {
                                out.push((Name::new(port), id));
                            }
                        }
                    }
                }
            }
        }
        if wildcard && let Some(ports) = ports {
            let names: Vec<String> = ports
                .iter()
                .filter(|p| !connected.contains(&p.name))
                .map(|p| p.name.clone())
                .collect();
            for port in names {
                let ident = ast::Ident::new(port.clone(), inst_name.span);
                let implicit = ast::Expr::new(ast::ExprKind::Ident(ident), inst_name.span);
                let leaked: &'ast ast::Expr = Box::leak(Box::new(implicit));
                if self.env.probe(|env| env.lookup(&port).is_some()) {
                    if let Some(id) =
                        self.connect(&port, leaked, Some(ports), inst_name.span, &mut out)
                    {
                        out.push((Name::new(port.clone()), id));
                    }
                    connected.push(port);
                }
            }
        }
        // Ports nobody connected.
        if let Some(ports) = ports {
            for p in ports {
                if !connected.contains(&p.name) {
                    self.warn_unconnected(&p.name, Some(ports), one.span, module);
                }
            }
        }
        out
    }

    /// True when `port` is an interface port of the target module.
    fn is_iface_port(&self, target: Option<ModuleId>, port: &str) -> bool {
        target
            .and_then(|id| self.env.cx.iface_ports.get(&id))
            .is_some_and(|ps| ps.iter().any(|p| p.name == port))
    }

    /// Warns about a port left unconnected, except for outputs, which are
    /// routinely left open on purpose.
    fn warn_unconnected(
        &mut self,
        port: &str,
        ports: Option<&[PortInfo]>,
        span: Span,
        module: &str,
    ) {
        let dir = ports
            .and_then(|ps| ps.iter().find(|p| p.name == port))
            .map(|p| p.dir);
        if dir == Some(PortDir::Out) {
            return;
        }
        let what = match dir {
            Some(PortDir::In) => "input",
            Some(PortDir::InOut) => "inout",
            _ => "port",
        };
        self.env.report(
            Diagnostic::warning(format!("{what} `{port}` of `{module}` is not connected"))
                .with_code(codes::PORT_UNCONNECTED)
                .with_label(span, "instantiated here")
                .with_note("an unconnected input reads as x"),
        );
    }

    /// Lowers one port connection, expanding interface bundles.
    fn connect(
        &mut self,
        port: &str,
        expr: &'ast ast::Expr,
        ports: Option<&[PortInfo]>,
        span: Span,
        out: &mut Vec<(Name, ExprId)>,
    ) -> Option<ExprId> {
        // An interface bundle expands into one connection per signal.
        if let Some(sym) = self.env.probe(|env| env.resolve_path(expr))
            && let Symbol::Iface { scope, .. } = sym
        {
            let signals: Vec<(String, String)> = ports.map(|_| Vec::new()).unwrap_or_default();
            let _ = signals;
            let target_signals = self.iface_signals_for(port);
            for (signal, ir_port) in target_signals {
                let net = self
                    .env
                    .cx
                    .scopes
                    .get(scope)
                    .get(&signal)
                    .and_then(|(s, _)| match s {
                        Symbol::Net { net, .. } => Some(*net),
                        _ => None,
                    });
                let Some(net) = net else {
                    self.env.error(
                        codes::UNDEFINED,
                        span,
                        format!("the interface has no signal `{signal}`"),
                    );
                    continue;
                };
                self.b.span = span;
                let id = self.b.net(net);
                out.push((Name::new(ir_port), id));
            }
            return None;
        }

        let info = ports.and_then(|ps| ps.iter().find(|p| p.name == port));
        let Some(info) = info else {
            // A black box: the connection keeps its natural width.
            let id = self.expr_cont(expr, None);
            return Some(id);
        };
        if info.dir == PortDir::In {
            let rhs = self
                .env
                .probe(|env| width::info(expr, env, &[]))
                .unwrap_or(Info::bits(info.width, false));
            width::check_assign(&mut self.env, port, info.width, &rhs, span);
            let id = self.expr_cont(
                expr,
                Some(Ctx {
                    width: info.width,
                    signed: rhs.is_signed(),
                }),
            );
            Some(id)
        } else {
            let id = self.expr_cont(expr, None);
            let width = self.b.module().expr(id).ty.width().unwrap_or(0);
            if width != info.width {
                self.env.report(
                    Diagnostic::error(format!(
                        "`{port}` is {} bits wide but is connected to {width} bits",
                        info.width
                    ))
                    .with_code(codes::PORT_WIDTH)
                    .with_label(span, "width mismatch on an output connection")
                    .with_note("an output connection must match the port exactly"),
                );
                return None;
            }
            Some(id)
        }
    }

    /// The IR port names of the interface port `port` of the instantiated
    /// module, as `(signal, port name)` pairs.
    fn iface_signals_for(&self, port: &str) -> Vec<(String, String)> {
        for ports in self.env.cx.iface_ports.values() {
            if let Some(p) = ports.iter().find(|p| p.name == port) {
                return p.signals.clone();
            }
        }
        Vec::new()
    }

    // --- interfaces --------------------------------------------------------

    /// Flattens an interface instance into this module.
    fn interface_instance(
        &mut self,
        iface: &str,
        decl: &'ast ast::Instantiation,
        one: &'ast ast::Instance,
    ) {
        let Some(name) = &one.name else {
            self.env
                .unsupported(one.span, "an interface instance without a name");
            return;
        };
        let Some(module) = self.env.cx.table.modules.get(iface).copied() else {
            return;
        };
        let mut overrides = self.build_overrides(&decl.params, &name.name);
        let scope = self.env.enter_new(format!("{}.", name.name));
        let mut params: Vec<(String, Value)> = Vec::new();
        if let Some(list) = &module.def.params {
            for pd in list {
                decls::param(&mut self.env, pd, &mut overrides);
            }
        }
        // The interface's own ports are aliased to what the instance
        // connects them to.
        if let ast::Ports::Ansi(ports) = &module.def.ports {
            for (i, p) in ports.iter().enumerate() {
                let actual = one
                    .conns
                    .iter()
                    .enumerate()
                    .find_map(|(j, c)| match &c.kind {
                        ast::PortConnKind::Named { name, conn } => match conn {
                            ast::NamedConn::Expr(e) if name.name == p.name.name => Some(e),
                            _ => None,
                        },
                        ast::PortConnKind::Positional(e) => {
                            (i == j).then_some(e.as_ref()).flatten()
                        }
                        ast::PortConnKind::Wildcard => None,
                    });
                match actual.and_then(|e| self.env.probe(|env| env.resolve_path(e))) {
                    Some(Symbol::Net { net, ty }) => {
                        self.env
                            .declare(&p.name.name, Symbol::Net { net, ty }, p.name.span);
                    }
                    _ => {
                        // Not a plain signal: give the interface its own
                        // net and drive it.
                        if let Some(ty) = types::resolve(&mut self.env, &p.data_type, &p.dims, true)
                        {
                            let ir_name = self.env.qualified(&p.name.name);
                            if let Some(net) = self.new_net(ir_name, ty.clone(), false, p.name.span)
                            {
                                self.env.declare(
                                    &p.name.name,
                                    Symbol::Net { net, ty },
                                    p.name.span,
                                );
                                if let Some(e) = actual {
                                    self.continuous_assign_to(net, e, None, p.name.span);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.declare_subroutines(&module.def.items);
        self.declarations(&module.def.items, &mut overrides, &mut params);
        // An interface may hold behaviour of its own.
        self.items(&module.def.items);
        self.env.leave();
        let parent = self.env.scope;
        self.env.cx.scopes.redeclare(
            parent,
            name.name.clone(),
            Symbol::Iface {
                scope,
                name: iface.to_owned(),
                modport: None,
            },
            name.span,
        );
    }

    /// Declares an interface port: one IR port per signal of the
    /// interface, named `port_signal`.
    pub(super) fn interface_port(
        &mut self,
        port: &'ast ast::Ident,
        iface: &str,
        modport: Option<&str>,
        _dir: PortDir,
        span: Span,
    ) {
        if iface.is_empty() {
            self.env
                .unsupported(span, "a generic `interface` port (name the interface type)");
            return;
        }
        let Some(module) = self.env.cx.table.modules.get(iface).copied() else {
            self.env.error(
                codes::MODULE_NOT_FOUND,
                span,
                format!("no interface named `{iface}`"),
            );
            return;
        };
        let directions = modport.map(|m| modport_directions(module.def, m));
        if let Some(None) = directions.as_ref().map(Option::as_ref) {
            self.env.error(
                codes::UNDEFINED,
                span,
                format!(
                    "interface `{iface}` has no modport `{}`",
                    modport.unwrap_or("")
                ),
            );
        }
        let directions = directions.flatten();
        let scope = self.env.enter_new(format!("{}.", port.name));
        let mut signals: Vec<(String, String)> = Vec::new();
        let mut overrides = Overrides::new();
        let params: Vec<(String, Value)> = Vec::new();
        if let Some(list) = &module.def.params {
            for pd in list {
                decls::param(&mut self.env, pd, &mut overrides);
            }
        }
        // Interface ports (`interface bus_if(input clk)`) become ports of
        // the enclosing module too.
        let mut declared: Vec<(String, VType, Span)> = Vec::new();
        if let ast::Ports::Ansi(ports) = &module.def.ports {
            for p in ports {
                if let Some(ty) = types::resolve(&mut self.env, &p.data_type, &p.dims, true) {
                    declared.push((p.name.name.clone(), ty, p.name.span));
                }
            }
        }
        for item in &module.def.items {
            match &item.kind {
                ast::ItemKind::Var(vd) => {
                    for d in &vd.decls {
                        if let Some(ty) =
                            types::resolve(&mut self.env, &vd.data_type, &d.dims, true)
                        {
                            declared.push((d.name.name.clone(), ty, d.name.span));
                        }
                    }
                }
                ast::ItemKind::Net(nd) => {
                    for d in &nd.decls {
                        if let Some(ty) =
                            types::resolve(&mut self.env, &nd.data_type, &d.dims, true)
                        {
                            declared.push((d.name.name.clone(), ty, d.name.span));
                        }
                    }
                }
                _ => {}
            }
        }
        for (signal, ty, decl_span) in declared {
            let dir = match &directions {
                Some(map) => match map.iter().find(|(n, _)| *n == signal) {
                    Some((_, d)) => *d,
                    None => continue,
                },
                None => PortDir::InOut,
            };
            let ir_name = format!("{}_{}", port.name, signal);
            let Some(net) = self.new_net(ir_name.clone(), ty.clone(), false, decl_span) else {
                continue;
            };
            self.env
                .declare(&signal, Symbol::Net { net, ty }, decl_span);
            self.b.span = decl_span;
            self.b.add_port(ir_name.clone(), dir, net);
            signals.push((signal, ir_name));
        }
        self.declare_subroutines(&module.def.items);
        self.env.leave();
        let parent = self.env.scope;
        self.env.cx.scopes.redeclare(
            parent,
            port.name.clone(),
            Symbol::Iface {
                scope,
                name: iface.to_owned(),
                modport: modport.map(str::to_owned),
            },
            port.span,
        );
        self.iface_ports.push(IfacePort {
            name: port.name.clone(),
            signals,
        });
        let _ = params;
    }

    // --- gate primitives ---------------------------------------------------

    /// Gate and switch instantiations.
    pub(super) fn gates(&mut self, g: &'ast ast::GateDecl, item: &'ast Item) {
        let delay = g.delay.as_ref().and_then(|d| self.delay_of(d));
        for one in &g.instances {
            if !one.dims.is_empty() {
                self.env.unsupported(one.span, "a gate instance array");
                continue;
            }
            self.gate(g.kind, one, delay, item);
        }
    }

    /// One gate instance.
    fn gate(
        &mut self,
        kind: GateKind,
        one: &'ast ast::GateInstance,
        delay: Option<ir::Delay>,
        item: &'ast Item,
    ) {
        use GateKind as G;
        match kind {
            G::And | G::Nand | G::Or | G::Nor | G::Xor | G::Xnor => {
                if one.conns.len() < 3 {
                    self.env.error(
                        codes::ARGUMENTS,
                        one.span,
                        format!(
                            "`{}` needs an output and at least two inputs",
                            kind.as_str()
                        ),
                    );
                    return;
                }
                let Some(target) = self.lvalue(&one.conns[0], false) else {
                    return;
                };
                let width = self.lvalue_width(&target);
                let ctx = Ctx {
                    width,
                    signed: false,
                };
                let mut acc: Option<ExprId> = None;
                for input in &one.conns[1..] {
                    let id = self.expr_cont(input, Some(ctx));
                    self.b.span = one.span;
                    acc = Some(match acc {
                        None => id,
                        Some(prev) => match kind {
                            G::And | G::Nand => self.b.and(prev, id),
                            G::Or | G::Nor => self.b.or(prev, id),
                            _ => self.b.xor(prev, id),
                        },
                    });
                }
                let Some(mut value) = acc else { return };
                if matches!(kind, G::Nand | G::Nor | G::Xnor) {
                    value = self.b.not(value);
                }
                self.b.span = one.span;
                self.b.assign_after(target.clone(), value, delay);
                let attrs = self.attrs_of(&item.attrs);
                if let Some(a) = self.b.module_mut().assigns.last_mut() {
                    a.attrs = attrs;
                }
                self.note_assign_drivers(&target, DriverKey::Continuous, one.span, false);
            }
            G::Buf | G::Not => {
                // Every terminal but the last is an output.
                if one.conns.len() < 2 {
                    self.env.error(
                        codes::ARGUMENTS,
                        one.span,
                        format!("`{}` needs an output and an input", kind.as_str()),
                    );
                    return;
                }
                let last = one.conns.len() - 1;
                for target_expr in &one.conns[..last] {
                    let Some(target) = self.lvalue(target_expr, false) else {
                        continue;
                    };
                    let width = self.lvalue_width(&target);
                    let mut value = self.expr_cont(
                        &one.conns[last],
                        Some(Ctx {
                            width,
                            signed: false,
                        }),
                    );
                    if kind == G::Not {
                        self.b.span = one.span;
                        value = self.b.not(value);
                    }
                    self.b.span = one.span;
                    self.b.assign_after(target.clone(), value, delay);
                    self.note_assign_drivers(&target, DriverKey::Continuous, one.span, false);
                }
            }
            G::Bufif0 | G::Bufif1 | G::Notif0 | G::Notif1 => {
                if one.conns.len() != 3 {
                    self.env.error(
                        codes::ARGUMENTS,
                        one.span,
                        format!(
                            "`{}` needs an output, an input and an enable",
                            kind.as_str()
                        ),
                    );
                    return;
                }
                let Some(target) = self.lvalue(&one.conns[0], false) else {
                    return;
                };
                let ir::Lvalue::Net(net) = target else {
                    self.env.unsupported(
                        one.span,
                        "a tri-state gate driving something other than a whole net",
                    );
                    return;
                };
                let width = self.net_width(net);
                let ctx = Ctx {
                    width,
                    signed: false,
                };
                let mut input = self.expr_cont(&one.conns[1], Some(ctx));
                if matches!(kind, GateKind::Notif0 | GateKind::Notif1) {
                    self.b.span = one.span;
                    input = self.b.not(input);
                }
                let mut enable = self.expr_cont(&one.conns[2], None);
                enable = self.as_bit(enable);
                if matches!(kind, GateKind::Bufif0 | GateKind::Notif0) {
                    self.b.span = one.span;
                    enable = self.b.lnot(enable);
                }
                self.b.span = one.span;
                let cell_name = match &one.name {
                    Some(n) => self.env.qualified(&n.name),
                    None => self.fresh("tri"),
                };
                self.b.cell(
                    cell_name,
                    CellKind::Tristate,
                    vec![(Name::new("a"), input), (Name::new("en"), enable)],
                    vec![(Name::new("y"), net)],
                );
                let index = self.b.module().cells.len() - 1;
                self.record_driver(net, DriverKey::Instance(index), true, one.span);
            }
            G::Pullup | G::Pulldown => {
                for target_expr in &one.conns {
                    let Some(target) = self.lvalue(target_expr, false) else {
                        continue;
                    };
                    let width = self.lvalue_width(&target);
                    self.b.span = one.span;
                    let value = if kind == G::Pullup {
                        self.b.constant(crate::logic::Logic::ones(width))
                    } else {
                        self.b.constant(crate::logic::Logic::zero(width))
                    };
                    self.b.assign(target.clone(), value);
                    self.note_assign_drivers(&target, DriverKey::Continuous, one.span, false);
                }
            }
            other => {
                self.env.unsupported(
                    one.span,
                    &format!("the `{}` switch primitive", other.as_str()),
                );
            }
        }
    }
}

/// The port directions a modport declares, as `(signal, direction)`.
fn modport_directions(m: &ast::Module, modport: &str) -> Option<Vec<(String, PortDir)>> {
    for item in &m.items {
        let ItemKind::Modport(list) = &item.kind else {
            continue;
        };
        for mp in list {
            if mp.name.name != modport {
                continue;
            }
            let mut out = Vec::new();
            for entry in &mp.items {
                if let ast::ModportItemKind::Port {
                    direction, name, ..
                } = &entry.kind
                {
                    let dir = match direction {
                        ast::Direction::Input => PortDir::In,
                        ast::Direction::Output => PortDir::Out,
                        _ => PortDir::InOut,
                    };
                    out.push((name.name.clone(), dir));
                }
            }
            return Some(out);
        }
    }
    None
}

/// The packed width of a type, for interface signals.
#[allow(dead_code)]
fn width_of(ty: &VType) -> u32 {
    ty.packed().map_or(1, Packed::width)
}
