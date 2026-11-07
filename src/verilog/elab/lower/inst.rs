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
    pub(super) fn bind_genvar(&mut self, scope: ScopeId, name: &str, value: i64, span: Span) {
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
    pub(super) fn step_genvar(&mut self, g: &'ast ast::GenFor, current: i64) -> Option<i64> {
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
                let Some(name) = &one.name else {
                    self.env
                        .unsupported(one.span, "an interface instance without a name");
                    continue;
                };
                if one.dims.is_empty() {
                    self.interface_instance(&module, inst, one, &name.name, name.span);
                    continue;
                }
                // An array of interfaces: one flattened instance per index,
                // `ch[0]`, `ch[1]`, reached by indexing `ch` as a generate
                // loop's iterations are. The elements used to be one and
                // the same instance.
                let indices = match one.dims.as_slice() {
                    [dim] => self.array_indices(dim),
                    _ => {
                        self.env.unsupported(
                            one.span,
                            "an interface instance array of more than one dimension",
                        );
                        None
                    }
                };
                let Some(indices) = indices else { continue };
                let mut scopes = Vec::with_capacity(indices.len());
                for i in indices {
                    let element = format!("{}[{i}]", name.name);
                    if let Some(scope) =
                        self.interface_instance(&module, inst, one, &element, name.span)
                    {
                        scopes.push((i, scope));
                    }
                }
                let parent = self.env.scope;
                self.env.cx.scopes.redeclare(
                    parent,
                    name.name.clone(),
                    Symbol::GenArray(scopes),
                    name.span,
                );
            }
            return;
        }
        for one in &inst.instances {
            let Some(name) = &one.name else {
                self.env.unsupported(one.span, "an instance without a name");
                continue;
            };
            if one.dims.is_empty() {
                self.one_instance(&module, &inst.params, name, one, item);
                continue;
            }
            let indices = match one.dims.as_slice() {
                [dim] => self.array_indices(dim),
                _ => {
                    self.env
                        .unsupported(one.span, "an instance array of more than one dimension");
                    None
                }
            };
            if let Some(indices) = indices {
                self.instance_array(&module, &inst.params, name, &indices, one, item);
            }
        }
    }

    /// The indices of an instance array's `[left:right]` or `[n]`, left
    /// first.
    pub(super) fn array_indices(&mut self, dim: &'ast ast::Dim) -> Option<Vec<i64>> {
        let (left, right) = match &dim.kind {
            ast::DimKind::Range(l, r) => {
                let l = Evaluator::new(&mut self.env).eval_i64(l).ok()?;
                let r = Evaluator::new(&mut self.env).eval_i64(r).ok()?;
                (l, r)
            }
            ast::DimKind::Size(n) => {
                let n = Evaluator::new(&mut self.env).eval_i64(n).ok()?;
                if n < 1 {
                    self.env
                        .error(codes::TYPE, dim.span, format!("an instance array of {n}"));
                    return None;
                }
                (0, n - 1)
            }
            _ => {
                self.env
                    .unsupported(dim.span, "this form of instance array dimension");
                return None;
            }
        };
        Some(if left >= right {
            (right..=left).rev().collect()
        } else {
            (left..=right).collect()
        })
    }

    /// `module name [left:right] (...)`: one instance per index, named
    /// `name[index]`.
    ///
    /// Each connection either goes to every instance whole, when it is
    /// as wide as the port, or is cut into one port's width per instance,
    /// when it is that times the number of instances — the instance with
    /// the rightmost index taking the least significant bits (IEEE
    /// 1364-2005 §12.1.2). A module nothing declares has no port widths
    /// to go by, so there a connection one bit wide goes to every
    /// instance and a wider one whose width the count divides is cut;
    /// that is the reading every vendor primitive used this way takes
    /// (`PULLUP p[7:0] (.O(bus))`), and the warning that the module is a
    /// black box already says its ports are not checked.
    fn instance_array(
        &mut self,
        module: &str,
        params: &'ast [ast::ParamOverride],
        name: &ast::Ident,
        indices: &[i64],
        one: &'ast ast::Instance,
        item: &'ast Item,
    ) {
        let overrides = self.build_overrides(module, params, &name.name);
        let known = self.env.cx.table.modules.contains_key(module);
        let target = if known {
            super::elaborate_module(self.env.cx, module, overrides, name.span)
        } else {
            self.env.report(
                Diagnostic::warning(format!("no module named `{module}` was found"))
                    .with_code(codes::BLACKBOX)
                    .with_label(name.span, "instantiated here")
                    .with_note("the instance is kept as a black box; its ports are not checked")
                    .with_note(format!(
                        "Reticle has no library search path, so nothing will find `{module}` \
                         later unless the target supplies it: add the file that defines it"
                    )),
            );
            None
        };
        let params_for_ir = if known {
            Vec::new()
        } else {
            self.build_overrides(module, params, &name.name).all()
        };
        let ports = target.map(|id| self.port_infos(id));
        self.natural_connections = true;
        let connections = self.connections(one, ports.as_deref(), module, target, name);
        self.natural_connections = false;
        let count = u32::try_from(indices.len()).unwrap_or(u32::MAX);

        // How each connection reaches the instances: whole, or cut into
        // `per` bits each.
        let mut plan: Vec<(Name, ExprId, Option<u32>)> = Vec::new();
        for (port, expr) in &connections {
            let width = self.b.module().expr(*expr).ty.width().unwrap_or(1);
            let port_width = ports
                .as_ref()
                .and_then(|ps| ps.iter().find(|p| p.name == port.as_str()))
                .map(|p| p.width);
            let per = match port_width {
                Some(pw) if width == pw => None,
                Some(pw) if Some(width) == pw.checked_mul(count) => Some(pw),
                Some(pw) => {
                    self.env.error(
                        codes::TYPE,
                        one.span,
                        format!(
                            "port `{port}` is {pw} bit(s) wide and the array has {count} \
                             instances, so its connection must be {pw} or {} bits, not {width}",
                            u64::from(pw) * u64::from(count)
                        ),
                    );
                    continue;
                }
                None if width > 1 && width.is_multiple_of(count) => Some(width / count),
                None => None,
            };
            plan.push((port.clone(), *expr, per));
        }

        let module_ref = match target {
            Some(id) => ModuleRef::Resolved(id),
            None => ModuleRef::Unresolved(Name::new(module)),
        };
        let attrs = self.attrs_of(&item.attrs);
        for (position, index) in indices.iter().enumerate() {
            // The rightmost index takes the lowest bits.
            let from_right = count - 1 - u32::try_from(position).unwrap_or(0);
            self.b.span = one.span;
            let mut conns: Vec<(Name, ExprId)> = Vec::with_capacity(plan.len());
            for (port, expr, per) in &plan {
                let piece = match per {
                    None => *expr,
                    Some(w) => self.b.slice(*expr, from_right * w + w - 1, from_right * w),
                };
                conns.push((port.clone(), piece));
            }
            let ir_name = self.env.qualified(&format!("{}[{index}]", name.name));
            let id = self.b.instance(ir_name, module_ref.clone(), conns.clone());
            {
                let inst = &mut self.b.module_mut().instances[id];
                inst.attrs = attrs.clone();
                for (p, v) in &params_for_ir {
                    match v {
                        Override::Value(Value::Logic(l)) => inst.params.set(p.clone(), l.clone()),
                        // A string is its text: `CLK_SEL_TYPE("ASYNC")` is
                        // read as `ASYNC`, not as a quoted key.
                        Override::Value(Value::Str(text)) => {
                            inst.params.set(p.clone(), text.clone())
                        }
                        other => inst.params.set(p.clone(), other.key_text()),
                    }
                }
            }
            if let Some(ports) = &ports {
                for (port, expr) in &conns {
                    let Some(info) = ports.iter().find(|p| p.name == port.as_str()) else {
                        continue;
                    };
                    // An `inout` is a bus; see the single instance's case.
                    if info.dir != PortDir::Out {
                        continue;
                    }
                    for (net, whole) in self.expr_nets(*expr) {
                        self.record_driver(net, DriverKey::Instance(id.index()), whole, one.span);
                    }
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
        let mut overrides = self.build_overrides(module, params, &name.name);
        self.interface_overrides(module, one, &mut overrides);
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
                    .with_note("the instance is kept as a black box; its ports are not checked")
                    .with_note(format!(
                        "Reticle has no library search path, so nothing will find `{module}` \
                         later unless the target supplies it: add the file that defines it"
                    )),
            );
            None
        };
        let params_for_ir = if known {
            Vec::new()
        } else {
            self.build_overrides(module, params, &name.name).all()
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
                    // A string is its text: `CLK_SEL_TYPE("ASYNC")` is read
                    // as `ASYNC`, not as a quoted key.
                    Override::Value(Value::Str(text)) => inst.params.set(p.clone(), text.clone()),
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
                // An `inout` is a bus the instance may drive or leave
                // floating, resolved with the other drivers of the wire
                // like a tri-state, as the IR validator counts it: a
                // testbench that drives `target_io4` beside the `inout` it
                // is joined to is the ordinary pattern, not two drivers.
                if info.dir != PortDir::Out {
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

    /// The declared type of `module`'s parameter `name`, or of its
    /// `index`th when the override is positional, resolved in the current
    /// scope: `None` for an implicitly typed parameter, or a type only the
    /// module itself declares.
    fn param_type(
        &mut self,
        module: &str,
        name: Option<&ast::Ident>,
        index: usize,
    ) -> Option<VType> {
        let decl = self.env.cx.table.modules.get(module).copied()?;
        let list = decl.def.params.as_ref()?;
        let mut all = list
            .iter()
            .filter(|pd| pd.kind == ast::ParamKind::Parameter && !pd.is_type)
            .flat_map(|pd| pd.decls.iter().map(move |d| (pd, d)));
        let (pd, d) = match name {
            Some(n) => all.find(|(_, d)| d.name.name == n.name)?,
            None => all.nth(index)?,
        };
        if pd.data_type.is_empty() {
            return None;
        }
        self.env
            .probe(|env| types::resolve(env, &pd.data_type, &d.dims, true))
    }

    /// Evaluates the parameter overrides of an instantiation, including
    /// any `defparam` aimed at it.
    pub(super) fn build_overrides(
        &mut self,
        module: &str,
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
            // `.CFG('{a: 4'hC, b: 4'hD})`: a pattern takes its type from
            // the parameter it overrides, when that type can be read here.
            if matches!(value.kind, ast::ExprKind::Pattern(_))
                && let Some(ty) = self.param_type(module, p.name.as_ref(), i)
                && let Some(v) = self
                    .env
                    .probe(|env| Evaluator::new(env).eval_as(value, &ty).ok())
            {
                over.push(name, Override::Value(v), p.span);
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
        self.connecting = target;
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
        // `bi.slave`: the interface instance `bi` seen through one of its
        // modports, which the port's own declaration has already applied.
        let through_modport = match &expr.kind {
            ast::ExprKind::Member { base, name } => {
                match self.env.probe(|env| env.resolve_path(base)) {
                    Some(Symbol::Iface {
                        scope, name: iface, ..
                    }) if self
                        .env
                        .cx
                        .table
                        .modules
                        .get(iface.as_str())
                        .is_some_and(|m| modport_directions(m.def, &name.name).is_some()) =>
                    {
                        Some(scope)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let bundle = match through_modport
            .map(Symbol::GenBlock)
            .or_else(|| self.env.probe(|env| env.resolve_path(expr)))
        {
            Some(Symbol::Iface { scope, .. }) => Some(scope),
            // An element of an array of interfaces, `ch[0]`.
            Some(Symbol::GenBlock(scope)) if !self.iface_signals_for(port).is_empty() => {
                Some(scope)
            }
            _ => None,
        };
        if let Some(scope) = bundle {
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
                // The module's port was made from the interface with its
                // own defaults; an instance of it with other parameters
                // has other widths, which used to reach the IR validator
                // as an internal error.
                let have = self.b.module().nets[net].ty.width();
                let want = ports
                    .and_then(|ps| ps.iter().find(|p| p.name == ir_port))
                    .map(|p| p.width);
                if let (Some(have), Some(want)) = (have, want)
                    && have != want
                {
                    self.env.unsupported(
                        span,
                        &format!(
                            "an interface port whose signal `{signal}` is {have} bits in the \
                             interface connected and {want} in the port"
                        ),
                    );
                    continue;
                }
                self.b.span = span;
                let id = self.b.net(net);
                out.push((Name::new(ir_port), id));
            }
            return None;
        }

        let info = ports
            .and_then(|ps| ps.iter().find(|p| p.name == port))
            .filter(|_| !self.natural_connections);
        let Some(info) = info else {
            // A black box, or an instance array, whose connections are
            // cut to the port width afterwards: the connection keeps its
            // natural width.
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
                // An output connected to a narrower or a wider net drives
                // its low bits, as a continuous assignment of the port's
                // value would (IEEE 1364-2005 §12.3.10): the bits above
                // the port's are left undriven, those above the net's are
                // dropped. xsim warns and builds it; it used to be refused.
                // An `inout` would need the bits to go both ways, and is
                // still refused.
                let target = (info.dir == PortDir::Out)
                    .then(|| self.lvalue(expr, false))
                    .flatten();
                let Some(target) = target else {
                    self.env.report(
                        Diagnostic::error(format!(
                            "`{port}` is {} bits wide but is connected to {width} bits",
                            info.width
                        ))
                        .with_code(codes::PORT_WIDTH)
                        .with_label(span, "width mismatch on an inout connection")
                        .with_note("an inout connection must match the port exactly"),
                    );
                    return None;
                };
                self.env.report(
                    Diagnostic::warning(format!(
                        "output `{port}` is {} bits wide and is connected to {width} bits",
                        info.width
                    ))
                    .with_code(codes::PORT_WIDTH)
                    .with_label(span, "connected here")
                    .with_note(if width < info.width {
                        "the port's bits above the connection's are dropped"
                    } else {
                        "the connection's bits above the port's are not driven"
                    }),
                );
                let name = self.fresh(&format!("{port}_out"));
                let net = self.helper_net(name, info.width, span);
                self.b.span = span;
                let value = self.b.net(net);
                let (target, value) = if width < info.width {
                    (target, self.b.slice(value, width - 1, 0))
                } else {
                    let low = match target {
                        ir::Lvalue::Net(n) => ir::Lvalue::Slice {
                            net: n,
                            hi: info.width - 1,
                            lo: 0,
                        },
                        ir::Lvalue::Slice { net: n, lo, .. } => ir::Lvalue::Slice {
                            net: n,
                            hi: lo + info.width - 1,
                            lo,
                        },
                        other => {
                            self.env.unsupported(
                                span,
                                "a wider connection of an output that is not a net or part of one",
                            );
                            let _ = other;
                            return None;
                        }
                    };
                    (low, value)
                };
                self.b.assign(target.clone(), value);
                self.note_assign_drivers(&target, DriverKey::Continuous, span, false);
                return Some(self.b.net(net));
            }
            // An element of a net array, `.y(d[0])`, is a target an
            // `assign` can write but no expression an instance can drive:
            // the port gets a net of its own, assigned into the element,
            // as a tri-state's output into one bit of a net already is.
            // Left as it was, the connection reached the IR validator as an
            // internal error, "connects out port `y` to an expression that
            // cannot be driven".
            if !self.drivable(id)
                && let Some(target) = self.lvalue(expr, false)
            {
                let name = self.fresh(&format!("{port}_out"));
                let net = self.helper_net(name, width, span);
                self.b.span = span;
                let value = self.b.net(net);
                self.b.assign(target.clone(), value);
                self.note_assign_drivers(&target, DriverKey::Continuous, span, false);
                return Some(self.b.net(net));
            }
            Some(id)
        }
    }

    /// Whether an expression is something an instance output can drive:
    /// a net, a constant slice of one, or a concatenation of those, which
    /// is what the IR validator accepts.
    fn drivable(&self, id: ExprId) -> bool {
        match &self.b.module().expr(id).kind {
            ir::ExprKind::Net(_) => true,
            ir::ExprKind::Slice { base, .. } => self.drivable(*base),
            ir::ExprKind::Concat(parts) => parts.iter().all(|p| self.drivable(*p)),
            _ => false,
        }
    }

    /// For each port of `module` connected to an interface instance whose
    /// parameters are not the interface's defaults, overrides `port.PARAM`,
    /// so that the module is elaborated with the interface it is given: an
    /// interface port used to be made from the interface with its defaults
    /// whatever the instance connected (`p_if #(.W(4))` into a port of
    /// `p_if`), and then refused.
    fn interface_overrides(
        &mut self,
        module: &str,
        one: &'ast ast::Instance,
        overrides: &mut Overrides,
    ) {
        let Some(decl) = self.env.cx.table.modules.get(module).copied() else {
            return;
        };
        let port_names: Vec<String> = match &decl.def.ports {
            ast::Ports::Ansi(ports) => ports.iter().map(|p| p.name.name.clone()).collect(),
            _ => Vec::new(),
        };
        for (i, conn) in one.conns.iter().enumerate() {
            let (port, expr) = match &conn.kind {
                ast::PortConnKind::Named {
                    name,
                    conn: ast::NamedConn::Expr(e),
                } => (name.name.clone(), e),
                ast::PortConnKind::Positional(Some(e)) => match port_names.get(i) {
                    Some(n) => (n.clone(), e),
                    None => continue,
                },
                _ => continue,
            };
            // `bi` or `bi.modport`.
            let base = match &expr.kind {
                ast::ExprKind::Member { base, .. } => {
                    match self.env.probe(|env| env.resolve_path(expr)) {
                        Some(Symbol::Iface { .. }) => expr,
                        _ => &**base,
                    }
                }
                _ => expr,
            };
            let Some(Symbol::Iface {
                scope, name: iface, ..
            }) = self.env.probe(|env| env.resolve_path(base))
            else {
                continue;
            };
            let Some(idecl) = self.env.cx.table.modules.get(iface.as_str()).copied() else {
                continue;
            };
            let Some(list) = &idecl.def.params else {
                continue;
            };
            // The defaults, in a scope of their own.
            let temp = self.env.enter_new(String::new());
            let mut none = Overrides::new();
            for pd in list {
                decls::param(&mut self.env, pd, &mut none);
            }
            let mut diffs = Vec::new();
            for pd in list {
                for d in &pd.decls {
                    let name = &d.name.name;
                    let default = self
                        .env
                        .cx
                        .scopes
                        .get(temp)
                        .get(name)
                        .map(|(s, _)| s.clone());
                    let given = self
                        .env
                        .cx
                        .scopes
                        .get(scope)
                        .get(name)
                        .map(|(s, _)| s.clone());
                    if let (
                        Some(Symbol::Const { value: a, .. }),
                        Some(Symbol::Const { value: b, .. }),
                    ) = (default, given)
                        && a != b
                    {
                        diffs.push((name.clone(), b, d.name.span));
                    }
                }
            }
            self.env.leave();
            for (name, value, span) in diffs {
                overrides.push(
                    format!("{port}.{name}"),
                    decls::Override::Value(value),
                    span,
                );
            }
        }
    }

    /// The IR port names of the interface port `port` of the instantiated
    /// module, as `(signal, port name)` pairs.
    ///
    /// They are the module being connected's own (`connecting`): two
    /// modules may each have a port `b`, of one interface through two
    /// modports, and the first module found with one used to answer for
    /// both, in an order a hash map chose.
    fn iface_signals_for(&self, port: &str) -> Vec<(String, String)> {
        self.connecting
            .and_then(|id| self.env.cx.iface_ports.get(&id))
            .and_then(|ports| ports.iter().find(|p| p.name == port))
            .map(|p| p.signals.clone())
            .unwrap_or_default()
    }

    // --- interfaces --------------------------------------------------------

    /// Flattens an interface instance into this module.
    fn interface_instance(
        &mut self,
        iface: &str,
        decl: &'ast ast::Instantiation,
        one: &'ast ast::Instance,
        name: &str,
        name_span: Span,
    ) -> Option<crate::verilog::elab::scope::ScopeId> {
        let module = self.env.cx.table.modules.get(iface).copied()?;
        let mut overrides = self.build_overrides(iface, &decl.params, name);
        // `reg_if #(16) c ()`: positional overrides name the interface's
        // parameters in order, as a module's do.
        super::name_positional(self.env.cx, &module, &mut overrides);
        let scope = self.env.enter_new(format!("{name}."));
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
            name.to_owned(),
            Symbol::Iface {
                scope,
                name: iface.to_owned(),
                modport: None,
            },
            name_span,
        );
        Some(scope)
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
        // The interface as the instance connected it: its parameters' values
        // come with the module's variant (`interface_overrides`).
        let mut overrides = Overrides::new();
        for (p, param, value) in &self.iface_params {
            if *p == port.name {
                overrides.push(param.clone(), value.clone(), port.span);
            }
        }
        let params: Vec<(String, Value)> = Vec::new();
        if let Some(list) = &module.def.params {
            for pd in list {
                decls::param(&mut self.env, pd, &mut overrides);
            }
        }
        // Interface ports (`interface bus_if(input clk)`) become ports of
        // the enclosing module too.
        // Each signal, and whether the interface declares it a variable,
        // which a module may then assign from a procedure through the port
        // as through the interface itself (`s.v = 1` in an `initial`). The
        // port's net used to be a net whatever the signal was, and such an
        // assignment was refused.
        let mut declared: Vec<(String, VType, Span, bool)> = Vec::new();
        if let ast::Ports::Ansi(ports) = &module.def.ports {
            for p in ports {
                if let Some(ty) = types::resolve(&mut self.env, &p.data_type, &p.dims, true) {
                    declared.push((p.name.name.clone(), ty, p.name.span, false));
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
                            declared.push((d.name.name.clone(), ty, d.name.span, true));
                        }
                    }
                }
                ast::ItemKind::Net(nd) => {
                    for d in &nd.decls {
                        if let Some(ty) =
                            types::resolve(&mut self.env, &nd.data_type, &d.dims, true)
                        {
                            declared.push((d.name.name.clone(), ty, d.name.span, false));
                        }
                    }
                }
                _ => {}
            }
        }
        for (signal, ty, decl_span, variable) in declared {
            let dir = match &directions {
                Some(map) => match map.iter().find(|(n, _)| *n == signal) {
                    Some((_, d)) => *d,
                    None => continue,
                },
                None => PortDir::InOut,
            };
            let ir_name = format!("{}_{}", port.name, signal);
            let variable = variable && dir != PortDir::In;
            let Some(net) = self.new_net(ir_name.clone(), ty.clone(), variable, decl_span) else {
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

    /// `value` as a gate's output drives it: a `z` bit becomes `x`, which
    /// an exclusive or with zero does (and synthesis folds away).
    fn strong(&mut self, value: ExprId) -> ExprId {
        let w = self.b.module().expr(value).ty.width().unwrap_or(1);
        let zero = self.b.const_u64(w, 0);
        self.b.xor(value, zero)
    }

    /// Gate and switch instantiations.
    pub(super) fn gates(&mut self, g: &'ast ast::GateDecl, item: &'ast Item) {
        let delay = g.delay.as_ref().and_then(|d| self.delay_of(d));
        for one in &g.instances {
            if !one.dims.is_empty() {
                self.gate_array(g.kind, one, delay);
                continue;
            }
            self.gate(g.kind, one, delay, item);
        }
    }

    /// `and g [3:0] (y, a, b)`: one gate per index, each terminal either
    /// one bit, shared by all, or as many bits as there are gates, the
    /// rightmost index taking the least significant (IEEE 1364-2005 §7.1.5).
    /// Every gate of the logic kinds computes its bit of the same bitwise
    /// function, so the array is that function on the whole vectors.
    fn gate_array(
        &mut self,
        kind: GateKind,
        one: &'ast ast::GateInstance,
        delay: Option<ir::Delay>,
    ) {
        use GateKind as G;
        if !matches!(
            kind,
            G::And | G::Nand | G::Or | G::Nor | G::Xor | G::Xnor | G::Buf | G::Not
        ) {
            self.env
                .unsupported(one.span, &format!("an array of `{}` gates", kind.as_str()));
            return;
        }
        let n = match one.dims.as_slice() {
            [dim] => self.array_indices(dim).map(|ix| ix.len()),
            _ => None,
        };
        let Some(n) = n.and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0) else {
            self.env
                .unsupported(one.span, "a gate array of more than one dimension");
            return;
        };
        if one.conns.len() < 2 {
            self.env.error(
                codes::ARGUMENTS,
                one.span,
                format!("`{}` needs an output and an input", kind.as_str()),
            );
            return;
        }
        let Some(target) = self.lvalue(&one.conns[0], false) else {
            return;
        };
        if self.lvalue_width(&target) != n {
            self.env.error(
                codes::PORT_WIDTH,
                one.conns[0].span,
                format!("an array of {n} gates drives {n} bits"),
            );
            return;
        }
        // Each input, as wide as the array.
        let mut values = Vec::new();
        for input in &one.conns[1..] {
            let w = self.info_of_pub(input).width();
            let id = self.expr_cont(
                input,
                Some(Ctx {
                    width: w.max(1),
                    signed: false,
                }),
            );
            self.b.span = input.span;
            let v = if w == n {
                id
            } else if w == 1 {
                self.b.replicate(n, id)
            } else {
                self.env.error(
                    codes::PORT_WIDTH,
                    input.span,
                    format!("a terminal of an array of {n} gates is 1 or {n} bits, not {w}"),
                );
                return;
            };
            values.push(v);
        }
        self.b.span = one.span;
        let mut value = match kind {
            G::Buf | G::Not => values[0],
            _ => {
                let mut acc = values[0];
                for v in &values[1..] {
                    acc = match kind {
                        G::And | G::Nand => self.b.and(acc, *v),
                        G::Or | G::Nor => self.b.or(acc, *v),
                        _ => self.b.xor(acc, *v),
                    };
                }
                acc
            }
        };
        value = match kind {
            G::Not | G::Nand | G::Nor | G::Xnor => self.b.not(value),
            G::Buf => self.strong(value),
            _ => value,
        };
        self.b.assign_after(target.clone(), value, delay);
        self.note_assign_drivers(&target, DriverKey::Continuous, one.span, false);
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
                    self.b.span = one.span;
                    value = if kind == G::Not {
                        self.b.not(value)
                    } else {
                        // A gate drives `x` for a `z` input (IEEE 1364-2005
                        // §7.2, table 7-3), where an assignment passes `z`
                        // on: `buf (y, a)` with `a` at `z` is `x`.
                        self.strong(value)
                    };
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
                self.b.span = one.span;
                input = if matches!(kind, GateKind::Notif0 | GateKind::Notif1) {
                    self.b.not(input)
                } else {
                    // Enabled, a `bufif` drives `x` for a `z` input (table
                    // 7-4), where `en ? d : 'z` passes `z` on.
                    self.strong(input)
                };
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
                // As any tri-state driver, one of several on a bus.
                self.record_driver(net, DriverKey::Continuous, true, one.span);
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
                    // A pull is weak: the net takes it only where nothing
                    // else drives it (IEEE 1364-2005 §7.10), so `pullup (p)`
                    // beside `assign p = e ? 0 : 'z` reads 1 or 0, not `x`.
                    if let Some(a) = self.b.module_mut().assigns.last_mut() {
                        a.attrs.set(ir::WEAK_ATTR, ir::AttrValue::Int(1));
                    }
                    self.note_assign_drivers(&target, DriverKey::Continuous, one.span, false);
                }
            }
            // A MOS switch passes its data, `z` included, while its gate is
            // on, and leaves the output undriven while it is off (IEEE
            // 1364-2005 §7.7): a tri-state driver. An `x` on the gate makes
            // the output `x` where the standard has `L` or `H`, which has no
            // strengths here. A `cmos` is an `nmos` and a `pmos` side by
            // side.
            G::Nmos | G::Pmos | G::Rnmos | G::Rpmos | G::Cmos | G::Rcmos => {
                let need = if matches!(kind, G::Cmos | G::Rcmos) {
                    4
                } else {
                    3
                };
                if one.conns.len() != need {
                    self.env.error(
                        codes::ARGUMENTS,
                        one.span,
                        format!("`{}` takes {need} terminals", kind.as_str()),
                    );
                    return;
                }
                let Some(ir::Lvalue::Net(net)) = self.lvalue(&one.conns[0], false) else {
                    self.env.unsupported(
                        one.span,
                        "a switch driving something other than a whole net",
                    );
                    return;
                };
                let width = self.net_width(net);
                let data = self.expr_cont(
                    &one.conns[1],
                    Some(Ctx {
                        width,
                        signed: false,
                    }),
                );
                let gates: Vec<(&'ast ast::Expr, bool)> = match kind {
                    G::Nmos | G::Rnmos => vec![(&one.conns[2], true)],
                    G::Pmos | G::Rpmos => vec![(&one.conns[2], false)],
                    _ => vec![(&one.conns[2], true), (&one.conns[3], false)],
                };
                for (gate, on_high) in gates {
                    let mut enable = self.expr_cont(gate, None);
                    enable = self.as_bit(enable);
                    if !on_high {
                        self.b.span = one.span;
                        enable = self.b.lnot(enable);
                    }
                    self.b.span = one.span;
                    let name = self.fresh("switch");
                    self.b.cell(
                        name,
                        CellKind::Tristate,
                        vec![(Name::new("a"), data), (Name::new("en"), enable)],
                        vec![(Name::new("y"), net)],
                    );
                    self.record_driver(net, DriverKey::Continuous, true, one.span);
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
