//! Building a document [`Index`] from a Verilog syntax tree.
//!
//! The Verilog frontend resolves names during elaboration, which needs the
//! whole design and a top module; a language server has neither, and must
//! answer on one half-written file. So this module does its own, smaller
//! job: it walks [`crate::verilog::ast`] once, opening a scope for every
//! declarative region (module, subroutine, named block, generate block),
//! recording every declaration it meets and every simple name it sees
//! used, and resolves the uses against the scope chain at the end — which
//! is what makes a use before its declaration, the normal case for a
//! module instantiated above its definition, resolve anyway.
//!
//! What it deliberately does not do is infer. A width is reported only
//! when the packed dimensions are written as literals or the type keyword
//! fixes it (`int` is 32 bits); `wire [W-1:0] q` has no width here,
//! because `W` may be overridden at every instantiation and a hover that
//! guesses is worse than one that says nothing. Hierarchical names
//! (`u0.state`) resolve their root only, and package-scoped names
//! (`pkg::T`) resolve the package only.

use crate::source::{SourceId, Span};
use crate::verilog::ast::{
    self, DataType, DataTypeKind, Declarator, Dim, DimKind, Expr, ExprKind, Item, ItemKind,
    Literal, Module, ModuleKind, Ports, Stmt, StmtKind,
};
use crate::verilog::{Dialect, Keyword};

use super::index::{Builder, DeclClass, DeclInfo, Index, InstSite, PortInfo};
use super::text::{collapse, first_line, slice};

/// Walks `file` and returns its index.
///
/// `text` is the document the tree was parsed from and `source` its id;
/// every span recorded points into it.
pub fn index(text: &str, source: SourceId, file: &ast::SourceFile) -> Index {
    let mut walk = Walk {
        text,
        source,
        b: Builder::new(false),
        parents: Vec::new(),
    };
    for item in &file.items {
        walk.item(item);
    }
    walk.b.finish()
}

/// The walk state: the text (for reading declarations back), the builder,
/// and the stack of declarations that enclose what is being walked.
struct Walk<'a> {
    text: &'a str,
    source: SourceId,
    b: Builder,
    parents: Vec<usize>,
}

impl Walk<'_> {
    /// The declaration currently being walked into.
    fn parent(&self) -> Option<usize> {
        self.parents.last().copied()
    }

    /// A declaration with the common fields filled in.
    fn decl(
        &self,
        name: &ast::Ident,
        class: DeclClass,
        detail: String,
        width: Option<u32>,
        full_span: Span,
    ) -> DeclInfo {
        DeclInfo {
            name: name.name.clone(),
            class,
            detail,
            width,
            text: collapse(slice(self.text, full_span)),
            name_span: name.span,
            full_span,
            ports: Vec::new(),
            parent: self.parent(),
            in_outline: true,
        }
    }

    /// An empty span at the end of `span`, used when a construct has no
    /// name of its own.
    fn anonymous_name(&self, span: Span) -> Span {
        Span::new(self.source, span.start, span.start)
    }

    // --- items -----------------------------------------------------------

    fn item(&mut self, item: &Item) {
        for attr in &item.attrs {
            if let Some(value) = &attr.value {
                self.expr(value);
            }
        }
        match &item.kind {
            ItemKind::Module(m) => self.module(item.span, m),
            ItemKind::Package(p) => self.package(item.span, p),
            ItemKind::Net(net) => {
                let base = net_detail(self.text, net.net_type.as_str(), &net.data_type);
                self.declarators(
                    &net.decls,
                    DeclClass::Net,
                    &base,
                    width_of(&net.data_type, true),
                    item.span,
                );
                self.data_type(&net.data_type);
            }
            ItemKind::Var(var) => {
                let base = type_detail(self.text, &var.data_type);
                let class = if var.constant {
                    DeclClass::Constant
                } else {
                    DeclClass::Variable
                };
                self.declarators(
                    &var.decls,
                    class,
                    &base,
                    width_of(&var.data_type, true),
                    item.span,
                );
                self.data_type(&var.data_type);
            }
            ItemKind::Param(param) => {
                let base = type_detail(self.text, &param.data_type);
                let base = if base.is_empty() {
                    param.kind.as_str().to_string()
                } else {
                    format!("{} {base}", param.kind.as_str())
                };
                let class = match param.kind {
                    ast::ParamKind::Parameter => DeclClass::Parameter,
                    _ => DeclClass::Constant,
                };
                // A parameter with no written type is sized by its value,
                // which needs elaboration, so no width is reported.
                self.declarators(
                    &param.decls,
                    class,
                    &base,
                    width_of(&param.data_type, false),
                    item.span,
                );
                self.data_type(&param.data_type);
            }
            ItemKind::Port(port) => {
                let base = port_detail(
                    self.text,
                    Some(port.direction),
                    port.net_type,
                    &port.data_type,
                );
                self.declarators(
                    &port.decls,
                    DeclClass::Port,
                    &base,
                    width_of(&port.data_type, true),
                    item.span,
                );
                self.data_type(&port.data_type);
            }
            ItemKind::Genvar(names) => {
                for name in names {
                    let decl = self.decl(name, DeclClass::Genvar, "genvar".into(), None, item.span);
                    self.b.declare(decl);
                }
            }
            ItemKind::Typedef(td) => {
                let detail = td
                    .data_type
                    .as_ref()
                    .map_or(String::new(), |dt| type_detail(self.text, dt));
                let decl = self.decl(&td.name, DeclClass::Type, detail, None, item.span);
                self.b.declare(decl);
                if let Some(dt) = &td.data_type {
                    self.data_type(dt);
                }
            }
            ItemKind::Import(refs) | ItemKind::Export(refs) => {
                for r in refs {
                    self.b.use_name(r.package.span, &r.package.name);
                }
            }
            ItemKind::Function(sub) => self.subroutine(item.span, sub, DeclClass::Function),
            ItemKind::Task(sub) => self.subroutine(item.span, sub, DeclClass::Procedure),
            ItemKind::Defparam(sets) => {
                for set in sets {
                    self.expr(&set.target);
                    self.expr(&set.value);
                }
            }
            ItemKind::Initial(stmt) => {
                self.process(item.span, "initial", None, stmt);
            }
            ItemKind::Final(stmt) => {
                self.process(item.span, "final", None, stmt);
            }
            ItemKind::Always(kind, stmt) => {
                // The sensitivity list is what tells two `always` blocks
                // apart in an outline, so it becomes the detail.
                let detail = match &stmt.kind {
                    StmtKind::Timing(control, _) => Some(collapse(slice(self.text, control.span))),
                    _ => None,
                };
                self.process(item.span, kind.as_str(), detail, stmt);
            }
            ItemKind::ContAssign(assign) => {
                for pair in &assign.assigns {
                    self.expr(&pair.lhs);
                    self.expr(&pair.rhs);
                }
            }
            ItemKind::Gate(gate) => {
                for inst in &gate.instances {
                    if let Some(name) = &inst.name {
                        let decl = self.decl(
                            name,
                            DeclClass::Instance,
                            gate.kind.as_str().to_string(),
                            None,
                            inst.span,
                        );
                        self.b.declare(decl);
                    }
                    for conn in &inst.conns {
                        self.expr(conn);
                    }
                }
            }
            ItemKind::Instance(inst) => self.instantiation(inst),
            ItemKind::Generate(items) => {
                for item in items {
                    self.item(item);
                }
            }
            ItemKind::GenIf(gen_if) => {
                self.expr(&gen_if.cond);
                self.gen_block(&gen_if.then_block);
                if let Some(block) = &gen_if.else_block {
                    self.gen_block(block);
                }
            }
            ItemKind::GenCase(gen_case) => {
                self.expr(&gen_case.expr);
                for arm in &gen_case.items {
                    for pattern in &arm.patterns {
                        self.expr(pattern);
                    }
                    self.gen_block(&arm.block);
                }
            }
            ItemKind::GenFor(gen_for) => {
                self.b.push_scope();
                let decl = DeclInfo {
                    name: gen_for.var.name.clone(),
                    class: DeclClass::Genvar,
                    detail: "genvar".to_string(),
                    width: None,
                    text: collapse(slice(self.text, gen_for.var.span)),
                    name_span: gen_for.var.span,
                    full_span: item.span,
                    ports: Vec::new(),
                    parent: self.parent(),
                    in_outline: false,
                };
                self.b.declare(decl);
                self.expr(&gen_for.init);
                self.expr(&gen_for.cond);
                self.expr(&gen_for.step);
                self.gen_block(&gen_for.body);
                self.b.pop_scope();
            }
            ItemKind::GenBlock(block) => self.gen_block(block),
            ItemKind::Alias(exprs) => {
                for expr in exprs {
                    self.expr(expr);
                }
            }
            ItemKind::Assertion(assertion) => self.assertion(assertion),
            ItemKind::Bind(bind) => {
                self.expr(&bind.target);
                for inst in &bind.instances {
                    self.expr(inst);
                }
                self.instantiation(&bind.inst);
            }
            ItemKind::Modport(modports) => {
                for modport in modports {
                    for entry in &modport.items {
                        match &entry.kind {
                            ast::ModportItemKind::Port { name, expr, .. } => {
                                self.b.use_name(name.span, &name.name);
                                if let Some(expr) = expr {
                                    self.expr(expr);
                                }
                            }
                            ast::ModportItemKind::Import(name)
                            | ast::ModportItemKind::Export(name)
                            | ast::ModportItemKind::Clocking(name) => {
                                self.b.use_name(name.span, &name.name);
                            }
                        }
                    }
                }
            }
            ItemKind::Clocking(clocking) => {
                if let Some(event) = &clocking.event {
                    self.event_control(event);
                }
            }
            ItemKind::PropertyDecl(_)
            | ItemKind::Specify(_)
            | ItemKind::Table(_)
            | ItemKind::Timeunit { .. }
            | ItemKind::Timeprecision(_)
            | ItemKind::Directive(_)
            | ItemKind::Empty => {}
        }
    }

    /// A module, interface, program or primitive.
    fn module(&mut self, span: Span, m: &Module) {
        let class = match m.kind {
            ModuleKind::Interface | ModuleKind::Program => DeclClass::Interface,
            _ => DeclClass::Module,
        };
        let mut decl = self.decl(&m.name, class, m.kind.as_str().to_string(), None, span);
        decl.text = first_line(slice(self.text, span));
        let id = self.b.declare(decl);
        self.b.push_scope();
        self.parents.push(id);

        for import in &m.imports {
            self.b.use_name(import.package.span, &import.package.name);
        }
        if let Some(params) = &m.params {
            for param in params {
                let base = type_detail(self.text, &param.data_type);
                let base = if base.is_empty() {
                    param.kind.as_str().to_string()
                } else {
                    format!("{} {base}", param.kind.as_str())
                };
                let full = param.decls.first().map_or(span, |d| d.span);
                self.declarators(
                    &param.decls,
                    DeclClass::Parameter,
                    &base,
                    width_of(&param.data_type, false),
                    full,
                );
                self.data_type(&param.data_type);
            }
        }

        // The header's port names, in order, so the module's port list can
        // be filled in once the body has declared their types.
        let mut port_names: Vec<String> = Vec::new();
        match &m.ports {
            Ports::None => {}
            Ports::NonAnsi(ports) => {
                for port in ports {
                    if let Some(name) = &port.name {
                        port_names.push(name.name.clone());
                    }
                    if let Some(expr) = &port.expr {
                        if port.name.is_none()
                            && let ExprKind::Ident(ident) = &expr.kind
                        {
                            port_names.push(ident.name.clone());
                        }
                        self.expr(expr);
                    }
                }
            }
            Ports::Ansi(ports) => {
                for port in ports {
                    port_names.push(port.name.name.clone());
                    let detail =
                        port_detail(self.text, port.direction, port.net_type, &port.data_type);
                    let mut decl = self.decl(
                        &port.name,
                        DeclClass::Port,
                        detail,
                        width_of(&port.data_type, true),
                        port.span,
                    );
                    decl.text = collapse(slice(self.text, port.span));
                    self.b.declare(decl);
                    self.data_type(&port.data_type);
                    for dim in &port.dims {
                        self.dim(dim);
                    }
                    if let Some(default) = &port.default {
                        self.expr(default);
                    }
                }
            }
        }

        for item in &m.items {
            self.item(item);
        }

        // Fill the port list from whatever the body ended up declaring, so
        // a non-ANSI port shows the type its `input`/`wire` gave it.
        let ports = self.port_infos(id, &port_names);
        self.b.decl_mut(id).ports = ports;

        self.parents.pop();
        self.b.pop_scope();
    }

    /// The port details of `owner`, in `names` order.
    fn port_infos(&self, owner: usize, names: &[String]) -> Vec<PortInfo> {
        names
            .iter()
            .map(|name| {
                let detail = self
                    .b
                    .decls()
                    .iter()
                    .find(|d| {
                        d.parent == Some(owner) && &d.name == name && d.class == DeclClass::Port
                    })
                    .map(DeclInfo::summary)
                    .unwrap_or_default();
                PortInfo {
                    name: name.clone(),
                    detail,
                }
            })
            .collect()
    }

    fn package(&mut self, span: Span, p: &ast::Package) {
        let mut decl = self.decl(&p.name, DeclClass::Package, "package".into(), None, span);
        decl.text = first_line(slice(self.text, span));
        let id = self.b.declare(decl);
        self.b.push_scope();
        self.parents.push(id);
        for item in &p.items {
            self.item(item);
        }
        self.parents.pop();
        self.b.pop_scope();
    }

    fn subroutine(&mut self, span: Span, sub: &ast::Subroutine, class: DeclClass) {
        let detail = sub
            .ret
            .as_ref()
            .map_or(String::new(), |dt| type_detail(self.text, dt));
        let mut decl = self.decl(&sub.name, class, detail, None, span);
        decl.text = first_line(slice(self.text, span));
        let id = self.b.declare(decl);
        self.b.push_scope();
        self.parents.push(id);
        let mut port_names = Vec::new();
        if let Some(ports) = &sub.ports {
            for port in ports {
                port_names.push(port.name.name.clone());
                let detail = port_detail(self.text, port.direction, port.net_type, &port.data_type);
                let decl = self.decl(
                    &port.name,
                    DeclClass::Port,
                    detail,
                    width_of(&port.data_type, true),
                    port.span,
                );
                self.b.declare(decl);
                self.data_type(&port.data_type);
                if let Some(default) = &port.default {
                    self.expr(default);
                }
            }
        }
        for stmt in &sub.body {
            self.stmt(stmt);
        }
        let ports = self.port_infos(id, &port_names);
        self.b.decl_mut(id).ports = ports;
        self.parents.pop();
        self.b.pop_scope();
    }

    /// An `always`, `initial` or `final` block, which the outline shows as
    /// a process.
    fn process(&mut self, span: Span, keyword: &str, detail: Option<String>, stmt: &Stmt) {
        let decl = DeclInfo {
            name: keyword.to_string(),
            class: DeclClass::Process,
            detail: detail.unwrap_or_default(),
            width: None,
            text: first_line(slice(self.text, span)),
            name_span: self.anonymous_name(span),
            full_span: span,
            ports: Vec::new(),
            parent: self.parent(),
            in_outline: true,
        };
        let id = self.b.declare_anonymous(decl);
        self.b.push_scope();
        self.parents.push(id);
        self.stmt(stmt);
        self.parents.pop();
        self.b.pop_scope();
    }

    fn instantiation(&mut self, inst: &ast::Instantiation) {
        self.b.use_name(inst.module.span, &inst.module.name);
        for param in &inst.params {
            if let Some(value) = &param.value {
                self.expr(value);
            }
        }
        for instance in &inst.instances {
            let full = instance.span;
            if let Some(name) = &instance.name {
                let decl = self.decl(
                    name,
                    DeclClass::Instance,
                    inst.module.name.clone(),
                    None,
                    full,
                );
                self.b.declare(decl);
            }
            let start = instance.name.as_ref().map_or(full.start, |n| n.span.end);
            let mut connected = Vec::new();
            for conn in &instance.conns {
                match &conn.kind {
                    ast::PortConnKind::Positional(expr) => {
                        if let Some(expr) = expr {
                            self.expr(expr);
                        }
                    }
                    ast::PortConnKind::Named { name, conn } => {
                        connected.push((name.name.clone(), name.span));
                        match conn {
                            ast::NamedConn::Expr(expr) => self.expr(expr),
                            // `.name` connects the signal of the same name,
                            // so the name is a use of that signal too.
                            ast::NamedConn::Implicit => {
                                self.b.use_name(name.span, &name.name);
                            }
                            ast::NamedConn::Open => {}
                        }
                    }
                    ast::PortConnKind::Wildcard => {}
                }
            }
            self.b.add_instance(InstSite {
                span: Span::new(self.source, start, full.end),
                target: inst.module.name.clone(),
                connected,
            });
        }
    }

    fn gen_block(&mut self, block: &ast::GenBlock) {
        let id = block.label.as_ref().map(|label| {
            let decl = self.decl(
                label,
                DeclClass::Block,
                "generate block".into(),
                None,
                block.span,
            );
            self.b.declare(decl)
        });
        self.b.push_scope();
        if let Some(id) = id {
            self.parents.push(id);
        }
        for item in &block.items {
            self.item(item);
        }
        if id.is_some() {
            self.parents.pop();
        }
        self.b.pop_scope();
    }

    fn assertion(&mut self, assertion: &ast::Assertion) {
        if let ast::AssertSpec::Expr(expr) = &assertion.spec {
            self.expr(expr);
        }
        if let Some(stmt) = &assertion.then_stmt {
            self.stmt(stmt);
        }
        if let Some(stmt) = &assertion.else_stmt {
            self.stmt(stmt);
        }
    }

    /// Declares one name per declarator of a declaration.
    fn declarators(
        &mut self,
        decls: &[Declarator],
        class: DeclClass,
        detail: &str,
        width: Option<u32>,
        full_span: Span,
    ) {
        for d in decls {
            let detail = if d.dims.is_empty() {
                detail.to_string()
            } else {
                let dims: String = d
                    .dims
                    .iter()
                    .map(|dim| collapse(slice(self.text, dim.span)))
                    .collect();
                format!("{detail} {dims}").trim().to_string()
            };
            let mut decl = self.decl(&d.name, class, detail, width, full_span);
            decl.text = collapse(slice(self.text, full_span));
            self.b.declare(decl);
            for dim in &d.dims {
                self.dim(dim);
            }
            if let Some(init) = &d.init {
                self.expr(init);
            }
        }
    }

    // --- statements ------------------------------------------------------

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Null
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::WaitFork
            | StmtKind::DisableFork => {}
            StmtKind::Block(block) | StmtKind::Fork(block, _) => self.block(block),
            StmtKind::Assign(assign) => {
                self.expr(&assign.lhs);
                if let Some(timing) = &assign.timing {
                    self.timing(timing);
                }
                self.expr(&assign.rhs);
            }
            StmtKind::Expr(expr)
            | StmtKind::Disable(expr)
            | StmtKind::Release(expr)
            | StmtKind::Deassign(expr) => self.expr(expr),
            StmtKind::If(if_stmt) => {
                self.expr(&if_stmt.cond);
                self.stmt(&if_stmt.then_stmt);
                if let Some(other) = &if_stmt.else_stmt {
                    self.stmt(other);
                }
            }
            StmtKind::Case(case) => {
                self.expr(&case.expr);
                for item in &case.items {
                    for pattern in &item.patterns {
                        self.expr(pattern);
                    }
                    self.stmt(&item.body);
                }
            }
            StmtKind::For(for_stmt) => {
                self.b.push_scope();
                for init in &for_stmt.init {
                    match init {
                        ast::ForInit::Decl(var) => {
                            let detail = type_detail(self.text, &var.data_type);
                            self.declarators(
                                &var.decls,
                                DeclClass::Variable,
                                &detail,
                                width_of(&var.data_type, true),
                                stmt.span,
                            );
                        }
                        ast::ForInit::Assign(expr) => self.expr(expr),
                    }
                }
                if let Some(cond) = &for_stmt.cond {
                    self.expr(cond);
                }
                for step in &for_stmt.step {
                    self.expr(step);
                }
                self.stmt(&for_stmt.body);
                self.b.pop_scope();
            }
            StmtKind::While(cond, body) | StmtKind::Repeat(cond, body) => {
                self.expr(cond);
                self.stmt(body);
            }
            StmtKind::DoWhile(body, cond) => {
                self.stmt(body);
                self.expr(cond);
            }
            StmtKind::Forever(body) => self.stmt(body),
            StmtKind::Foreach(foreach) => {
                self.expr(&foreach.array);
                self.b.push_scope();
                for var in foreach.vars.iter().flatten() {
                    let decl = self.decl(
                        var,
                        DeclClass::Variable,
                        "loop index".into(),
                        None,
                        var.span,
                    );
                    let mut decl = decl;
                    decl.in_outline = false;
                    self.b.declare(decl);
                }
                self.stmt(&foreach.body);
                self.b.pop_scope();
            }
            StmtKind::Return(expr) => {
                if let Some(expr) = expr {
                    self.expr(expr);
                }
            }
            StmtKind::Timing(control, body) => {
                self.timing(control);
                self.stmt(body);
            }
            StmtKind::Wait(cond, body) => {
                self.expr(cond);
                self.stmt(body);
            }
            StmtKind::ProcAssign(lhs, rhs) | StmtKind::Force(lhs, rhs) => {
                self.expr(lhs);
                self.expr(rhs);
            }
            StmtKind::Trigger { target, .. } => self.expr(target),
            StmtKind::Assert(assertion) => self.assertion(assertion),
            StmtKind::Decl(item) => self.item(item),
        }
    }

    fn block(&mut self, block: &ast::Block) {
        let id = block.label.as_ref().map(|label| {
            let decl = self.decl(label, DeclClass::Block, "block".into(), None, block.span);
            self.b.declare(decl)
        });
        self.b.push_scope();
        if let Some(id) = id {
            self.parents.push(id);
        }
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
        if id.is_some() {
            self.parents.pop();
        }
        self.b.pop_scope();
    }

    fn timing(&mut self, control: &ast::TimingControl) {
        match &control.kind {
            ast::TimingKind::Delay(delay) => {
                for value in &delay.values {
                    self.expr(value);
                }
            }
            ast::TimingKind::Event(event) => self.event_control(event),
            ast::TimingKind::RepeatEvent(count, event) => {
                self.expr(count);
                self.event_control(event);
            }
        }
    }

    fn event_control(&mut self, event: &ast::EventControl) {
        if let ast::EventControlKind::List(exprs) = &event.kind {
            for entry in exprs {
                self.expr(&entry.expr);
                if let Some(iff) = &entry.iff {
                    self.expr(iff);
                }
            }
        }
    }

    // --- expressions and types -------------------------------------------

    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Literal(_) | ExprKind::Default | ExprKind::SystemIdent(_) => {}
            ExprKind::Ident(ident) => self.b.use_name(ident.span, &ident.name),
            // Only the root of a hierarchical or scoped name can be
            // resolved without elaboration.
            ExprKind::Scoped { scope, .. } | ExprKind::Member { base: scope, .. } => {
                self.expr(scope);
            }
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            ExprKind::Range {
                base, left, right, ..
            } => {
                self.expr(base);
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                self.expr(cond);
                self.expr(then_expr);
                self.expr(else_expr);
            }
            ExprKind::Concat(exprs) | ExprKind::New(exprs) => {
                for e in exprs {
                    self.expr(e);
                }
            }
            ExprKind::Replicate { count, elems } => {
                self.expr(count);
                for e in elems {
                    self.expr(e);
                }
            }
            ExprKind::Streaming { slice, elems, .. } => {
                if let Some(slice) = slice {
                    self.expr(slice);
                }
                for e in elems {
                    self.expr(e);
                }
            }
            ExprKind::Pattern(items) => {
                for item in items {
                    if let Some(key) = &item.key {
                        self.expr(key);
                    }
                    self.expr(&item.value);
                }
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                for arg in args {
                    if let Some(value) = &arg.value {
                        self.expr(value);
                    }
                }
            }
            ExprKind::Cast { target, expr } => {
                match target {
                    ast::CastTarget::Type(dt) => self.data_type(dt),
                    ast::CastTarget::Size(size) => self.expr(size),
                    ast::CastTarget::Signing(_) | ast::CastTarget::Const => {}
                }
                self.expr(expr);
            }
            ExprKind::Inside { expr, set } => {
                self.expr(expr);
                for e in set {
                    self.expr(e);
                }
            }
            ExprKind::ValueRange { low, high } => {
                self.expr(low);
                self.expr(high);
            }
            ExprKind::MinTypMax { min, typ, max } => {
                self.expr(min);
                self.expr(typ);
                self.expr(max);
            }
            ExprKind::Type(dt) => self.data_type(dt),
            ExprKind::Assign { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::IncDec { target, .. } => self.expr(target),
        }
    }

    fn data_type(&mut self, dt: &DataType) {
        for dim in &dt.packed {
            self.dim(dim);
        }
        match &dt.kind {
            DataTypeKind::Named { package, name, .. } => match package {
                Some(pkg) => self.b.use_name(pkg.span, &pkg.name),
                None => self.b.use_name(name.span, &name.name),
            },
            DataTypeKind::Enum(en) => {
                if let Some(base) = &en.base {
                    self.data_type(base);
                }
                for variant in &en.variants {
                    let decl = self.decl(
                        &variant.name,
                        DeclClass::EnumLiteral,
                        "enumeration literal".into(),
                        None,
                        variant.span,
                    );
                    self.b.declare(decl);
                    if let Some(value) = &variant.value {
                        self.expr(value);
                    }
                }
            }
            DataTypeKind::Struct(st) => {
                for member in &st.members {
                    self.data_type(&member.data_type);
                    for d in &member.decls {
                        if let Some(init) = &d.init {
                            self.expr(init);
                        }
                    }
                }
            }
            DataTypeKind::TypeOf(expr) => self.expr(expr),
            _ => {}
        }
    }

    fn dim(&mut self, dim: &Dim) {
        match &dim.kind {
            DimKind::Range(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            DimKind::Size(n) => self.expr(n),
            DimKind::Queue(Some(n)) => self.expr(n),
            DimKind::Assoc(Some(dt)) => self.data_type(dt),
            DimKind::Unsized | DimKind::Queue(None) | DimKind::Assoc(None) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Types and widths
// ---------------------------------------------------------------------------

/// The type as written, or the empty string when nothing was.
fn type_detail(text: &str, dt: &DataType) -> String {
    if dt.is_empty() {
        String::new()
    } else {
        collapse(slice(text, dt.span))
    }
}

/// `wire [7:0]`: the net keyword and whatever type followed it.
fn net_detail(text: &str, net_type: &str, dt: &DataType) -> String {
    let written = type_detail(text, dt);
    if written.is_empty() {
        net_type.to_string()
    } else {
        format!("{net_type} {written}")
    }
}

/// `input wire [7:0]`: the direction, net keyword and type.
fn port_detail(
    text: &str,
    direction: Option<ast::Direction>,
    net_type: Option<ast::NetType>,
    dt: &DataType,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(direction) = direction {
        parts.push(direction.as_str().to_string());
    }
    if let Some(net) = net_type {
        parts.push(net.as_str().to_string());
    }
    let written = type_detail(text, dt);
    if !written.is_empty() {
        parts.push(written);
    }
    parts.join(" ")
}

/// The width a type fixes, when the source alone settles it.
///
/// `implicit_is_one` distinguishes a net or port with no type written,
/// which is one bit, from a parameter with none, whose width comes from
/// its value and so is not known here.
pub fn width_of(dt: &DataType, implicit_is_one: bool) -> Option<u32> {
    if !dt.packed.is_empty() {
        let mut total: u32 = 1;
        for dim in &dt.packed {
            total = total.checked_mul(dim_width(dim)?)?;
        }
        return Some(total);
    }
    match &dt.kind {
        DataTypeKind::Implicit => implicit_is_one.then_some(1),
        DataTypeKind::Integer(int) => Some(match int {
            ast::IntegerType::Bit | ast::IntegerType::Logic | ast::IntegerType::Reg => 1,
            ast::IntegerType::Byte => 8,
            ast::IntegerType::Shortint => 16,
            ast::IntegerType::Int | ast::IntegerType::Integer => 32,
            ast::IntegerType::Longint | ast::IntegerType::Time => 64,
        }),
        _ => None,
    }
}

/// The number of bits one packed dimension covers, when its bounds are
/// literals.
fn dim_width(dim: &Dim) -> Option<u32> {
    match &dim.kind {
        DimKind::Range(msb, lsb) => {
            let msb = const_int(msb)?;
            let lsb = const_int(lsb)?;
            u32::try_from(msb.abs_diff(lsb).checked_add(1)?).ok()
        }
        DimKind::Size(n) => u32::try_from(const_int(n)?).ok(),
        _ => None,
    }
}

/// The value of an expression that is a plain decimal literal, possibly
/// negated.
///
/// Anything else — a parameter, a sized literal, an expression over them —
/// is not evaluated: that is elaboration's job, and it needs the parameter
/// overrides a language server does not have.
fn const_int(expr: &Expr) -> Option<i64> {
    match &expr.kind {
        ExprKind::Literal(Literal::Number { text, .. }) => {
            let digits: String = text.chars().filter(|c| *c != '_').collect();
            digits.parse::<i64>().ok()
        }
        ExprKind::Unary { op, operand } => {
            let value = const_int(operand)?;
            match op {
                ast::UnaryOp::Minus => value.checked_neg(),
                ast::UnaryOp::Plus => Some(value),
                _ => None,
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Keywords
// ---------------------------------------------------------------------------

/// Where in a Verilog file a position is, as far as keyword completion
/// needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Outside any module: only design units may start here.
    File,
    /// Inside a module or package body, at item level.
    Item,
    /// Inside a procedural block or subroutine body.
    Statement,
}

/// The keywords worth offering in `context`, filtered to those the dialect
/// actually reserves, sorted and without duplicates.
pub fn keywords(context: Context, dialect: Dialect) -> Vec<&'static str> {
    let words: &[&str] = match context {
        Context::File => &[
            "module",
            "macromodule",
            "primitive",
            "package",
            "interface",
            "program",
            "typedef",
            "parameter",
            "localparam",
            "function",
            "task",
            "import",
            "endmodule",
        ],
        Context::Item => &[
            "wire",
            "tri",
            "supply0",
            "supply1",
            "reg",
            "logic",
            "bit",
            "integer",
            "real",
            "time",
            "input",
            "output",
            "inout",
            "parameter",
            "localparam",
            "assign",
            "always",
            "always_comb",
            "always_ff",
            "always_latch",
            "initial",
            "final",
            "generate",
            "endgenerate",
            "genvar",
            "function",
            "task",
            "typedef",
            "enum",
            "struct",
            "defparam",
            "endmodule",
            "begin",
            "end",
            "if",
            "else",
            "for",
            "case",
        ],
        Context::Statement => &[
            "begin",
            "end",
            "if",
            "else",
            "case",
            "casez",
            "casex",
            "endcase",
            "default",
            "for",
            "while",
            "repeat",
            "forever",
            "do",
            "break",
            "continue",
            "return",
            "fork",
            "join",
            "join_any",
            "join_none",
            "wait",
            "disable",
            "assign",
            "deassign",
            "force",
            "release",
            "posedge",
            "negedge",
            "unique",
            "priority",
            "reg",
            "logic",
            "integer",
            "automatic",
        ],
    };
    let mut out: Vec<&'static str> = words
        .iter()
        .filter_map(|word| Keyword::lookup(word, dialect).map(Keyword::as_str))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::source::SourceMap;
    use crate::verilog::{NoIncludes, parse_source};

    fn build(src: &str) -> (String, Index) {
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
        let text = map.file(id).text().to_string();
        let index = index(&text, id, &file);
        (text, index)
    }

    fn at(text: &str, needle: &str) -> u32 {
        u32::try_from(text.find(needle).expect("needle")).unwrap()
    }

    const COUNTER: &str = "\
module counter #(parameter W = 8) (
  input wire clk,
  input wire rst,
  output reg [7:0] q
);
  wire [W-1:0] next;
  assign next = q + 1'b1;
  always @(posedge clk) begin
    if (rst) q <= 8'd0;
    else q <= next;
  end
endmodule
";

    #[test]
    fn declares_ports_nets_and_parameters() {
        let (text, index) = build(COUNTER);
        let module = index.find("counter", &[DeclClass::Module]).unwrap();
        assert_eq!(index.decls[module].class, DeclClass::Module);
        assert_eq!(
            index.decls[module]
                .ports
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["clk", "rst", "q"]
        );
        assert_eq!(index.decls[module].ports[2].detail, "output reg [7:0]");

        let q = index.decl_at(at(&text, "q\n)")).unwrap();
        assert_eq!(index.decls[q].class, DeclClass::Port);
        assert_eq!(index.decls[q].width, Some(8));
        assert_eq!(index.decls[q].detail, "output reg [7:0]");

        let clk = index.decl_at(at(&text, "clk,")).unwrap();
        assert_eq!(index.decls[clk].width, Some(1));
        assert_eq!(index.decls[clk].detail, "input wire");

        // `W` is a parameter, so `[W-1:0]` has no width the source settles.
        let next = index.decl_at(at(&text, "next;")).unwrap();
        assert_eq!(index.decls[next].width, None);
        assert_eq!(index.decls[next].detail, "wire [W-1:0]");

        let w = index.find("W", &[DeclClass::Parameter]).unwrap();
        assert_eq!(index.decls[w].detail, "parameter");
        assert_eq!(index.decls[w].width, None);
    }

    #[test]
    fn resolves_uses_to_declarations() {
        let (text, index) = build(COUNTER);
        let q_decl = index.decl_at(at(&text, "q\n)")).unwrap();
        // Every mention of `q` resolves to the port.
        for needle in ["q + 1", "q <= 8'd0", "q <= next"] {
            assert_eq!(index.decl_at(at(&text, needle)), Some(q_decl), "{needle}");
        }
        assert_eq!(index.occurrences(q_decl).len(), 4);
    }

    #[test]
    fn resolves_a_use_before_its_declaration() {
        let (text, index) =
            build("module top; sub u0 (.a(1'b0)); endmodule\nmodule sub(input a); endmodule\n");
        let sub = index.find("sub", &[DeclClass::Module]).unwrap();
        assert_eq!(index.decl_at(at(&text, "sub u0")), Some(sub));
    }

    #[test]
    fn inner_blocks_shadow() {
        let (text, index) = build(
            "module m;\n  integer i;\n  initial begin : inner\n    integer i;\n    i = 1;\n  end\nendmodule\n",
        );
        let outer = index.decl_at(at(&text, "i;\n  initial")).unwrap();
        let inner = index.decl_at(at(&text, "i;\n    i = 1")).unwrap();
        assert_ne!(outer, inner);
        assert_eq!(index.decl_at(at(&text, "i = 1")), Some(inner));
        assert!(index.find("inner", &[DeclClass::Block]).is_some());
    }

    #[test]
    fn non_ansi_ports_are_one_declaration() {
        let (text, index) =
            build("module m(a, y);\n  input [3:0] a;\n  output y;\n  wire y;\nendmodule\n");
        let a = index.decl_at(at(&text, "a, y")).unwrap();
        assert_eq!(index.decls[a].class, DeclClass::Port);
        assert_eq!(index.decls[a].width, Some(4));
        let module = index.find("m", &[DeclClass::Module]).unwrap();
        assert_eq!(index.decls[module].ports.len(), 2);
        assert_eq!(index.decls[module].ports[0].detail, "input [3:0]");
        // `output y;` and `wire y;` name the same object, so the three
        // mentions of `y` are occurrences of one declaration.
        let y = index.decl_at(at(&text, "y;\n  wire")).unwrap();
        assert_eq!(index.occurrences(y).len(), 3);
    }

    #[test]
    fn records_instantiation_connection_lists() {
        let (text, index) = build(
            "module sub(input clk, output q); endmodule\nmodule top;\n  wire c, d;\n  sub u0 (.clk(c), .q(d));\nendmodule\n",
        );
        let site = index.instance_at(at(&text, ".clk(c)")).unwrap();
        assert_eq!(site.target, "sub");
        let formals: Vec<&str> = site.connected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(formals, ["clk", "q"]);
        let inst = index.find("u0", &[DeclClass::Instance]).unwrap();
        assert_eq!(index.decls[inst].detail, "sub");
    }

    #[test]
    fn implicit_named_connections_are_uses() {
        let (text, index) = build(
            "module sub(input clk); endmodule\nmodule top;\n  wire clk;\n  sub u0 (.clk);\nendmodule\n",
        );
        let clk = index.decl_at(at(&text, "clk;\n  sub")).unwrap();
        assert_eq!(index.decls[clk].class, DeclClass::Net);
        assert_eq!(index.occurrences(clk).len(), 2);
    }

    #[test]
    fn the_outline_nests_processes_and_instances() {
        let (_, index) = build(COUNTER);
        let outline = index.outline();
        assert_eq!(outline.len(), 1);
        let names: Vec<&str> = outline[0]
            .children
            .iter()
            .map(|n| index.decls[n.decl].name.as_str())
            .collect();
        assert_eq!(names, ["W", "clk", "rst", "q", "next", "always"]);
        let always = outline[0].children.last().unwrap();
        assert_eq!(index.decls[always.decl].detail, "@(posedge clk)");
    }

    #[test]
    fn widths_come_from_literals_and_keywords() {
        let (text, index) = build(
            "module m;\n  logic [31:0] a;\n  int b;\n  byte c;\n  logic d;\n  logic [3:0][7:0] e;\n  logic [N:0] f;\nendmodule\n",
        );
        let width = |needle: &str| index.decls[index.decl_at(at(&text, needle)).unwrap()].width;
        assert_eq!(width("a;"), Some(32));
        assert_eq!(width("b;"), Some(32));
        assert_eq!(width("c;"), Some(8));
        assert_eq!(width("d;"), Some(1));
        assert_eq!(width("e;"), Some(32));
        assert_eq!(width("f;"), None);
    }

    #[test]
    fn functions_and_their_ports() {
        let (text, index) = build(
            "module m;\n  function automatic [7:0] add(input [7:0] x, input [7:0] y);\n    add = x + y;\n  endfunction\nendmodule\n",
        );
        let f = index.find("add", &[DeclClass::Function]).unwrap();
        assert_eq!(index.decls[f].ports.len(), 2);
        assert_eq!(index.decls[f].ports[0].detail, "input [7:0]");
        let x = index.decl_at(at(&text, "x + y")).unwrap();
        assert_eq!(index.decls[x].class, DeclClass::Port);
    }

    #[test]
    fn typedefs_enums_and_generate_blocks() {
        let (text, index) = build(
            "module m;\n  typedef enum logic [1:0] { IDLE, RUN } state_t;\n  state_t s;\n  genvar i;\n  generate for (i = 0; i < 2; i = i + 1) begin : g\n    wire w;\n  end endgenerate\nendmodule\n",
        );
        let t = index.find("state_t", &[DeclClass::Type]).unwrap();
        assert_eq!(index.decl_at(at(&text, "state_t s")), Some(t));
        assert!(index.find("IDLE", &[DeclClass::EnumLiteral]).is_some());
        assert!(index.find("g", &[DeclClass::Block]).is_some());
    }

    #[test]
    fn keyword_lists_are_gated_and_sorted() {
        let sv = keywords(Context::Item, Dialect::SystemVerilog);
        assert!(sv.contains(&"always_ff"));
        assert!(sv.windows(2).all(|w| w[0] < w[1]));
        let v2005 = keywords(Context::Item, Dialect::Verilog2005);
        assert!(!v2005.contains(&"always_ff"));
        assert!(v2005.contains(&"always"));
        assert!(keywords(Context::File, Dialect::Verilog2005).contains(&"module"));
        assert!(keywords(Context::Statement, Dialect::Verilog2005).contains(&"begin"));
    }

    #[test]
    fn survives_a_broken_file() {
        // The parser recovers and the walk must not panic on the remains.
        let (_, index) = build("module m(input a;\n  wire ;\n  assign = a;\nendmodule\n");
        assert!(index.find("m", &[DeclClass::Module]).is_some());
    }
}
