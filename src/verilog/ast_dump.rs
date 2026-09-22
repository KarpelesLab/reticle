//! A deterministic text rendering of the Verilog AST.
//!
//! Used by the golden tests under `testdata/verilog/parse/` and handy for
//! debugging. One node per line, children indented by two spaces;
//! expressions and data types are rendered inline, with every binary,
//! unary and conditional operator fully parenthesised so precedence and
//! associativity are visible:
//!
//! ```text
//! module counter @1:1
//!   param parameter int W = 8
//!   port input logic clk
//!   port output logic [(W - 1):0] q
//!   always_ff @2:3
//!     @(posedge clk)
//!       q <= (q + 1)
//! ```
//!
//! With [`dump_with_locations`] each item and statement line ends with
//! `@line:col` of its span, which lets the golden files check spans as
//! well as structure. [`dump`] omits the locations.

use std::fmt::Write as _;

use crate::source::{SourceMap, Span};

use super::ast::{
    Arg, AssertSpec, Assertion, Attribute, CastTarget, DataType, DataTypeKind, Declarator, Delay,
    Dim, DimKind, EventControl, EventControlKind, Expr, ExprKind, ForInit, GenBlock, Item,
    ItemKind, Literal, ModportItemKind, Module, NamedConn, ParamDecl, Port, PortConnKind, Ports,
    SourceFile, Stmt, StmtKind, TimingKind,
};

/// Renders a whole file without locations.
pub fn dump(file: &SourceFile) -> String {
    let mut d = Dumper::new(None);
    d.items(&file.items);
    d.out
}

/// Renders a whole file with `@line:col` locations resolved through `map`.
pub fn dump_with_locations(file: &SourceFile, map: &SourceMap) -> String {
    let mut d = Dumper::new(Some(map));
    d.items(&file.items);
    d.out
}

/// Renders one expression inline.
pub fn expr_to_string(expr: &Expr) -> String {
    let mut s = String::new();
    write_expr(&mut s, expr);
    s
}

/// Renders one data type inline; empty for an implicit type with nothing
/// written.
pub fn type_to_string(ty: &DataType) -> String {
    let mut s = String::new();
    write_type(&mut s, ty);
    s
}

/// Renders one statement (and its children) as an indented block.
pub fn stmt_to_string(stmt: &Stmt) -> String {
    let mut d = Dumper::new(None);
    d.stmt(stmt);
    d.out
}

/// Accumulates indented lines.
struct Dumper<'a> {
    out: String,
    indent: usize,
    map: Option<&'a SourceMap>,
}

impl<'a> Dumper<'a> {
    fn new(map: Option<&'a SourceMap>) -> Self {
        Dumper {
            out: String::new(),
            indent: 0,
            map,
        }
    }

    /// One line at the current indentation, with a location suffix when
    /// a map is available.
    fn line(&mut self, text: &str, span: Option<Span>) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        if let (Some(map), Some(span)) = (self.map, span) {
            let (_, loc) = map.locate(span);
            let _ = write!(self.out, " @{loc}");
        }
        self.out.push('\n');
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self)) {
        self.indent += 1;
        f(self);
        self.indent -= 1;
    }

    /// The verbatim body of a shallow construct, as one indented line;
    /// nothing for an empty body.
    fn raw_body(&mut self, body: &super::ast::RawTokens) {
        if !body.text.is_empty() {
            self.nested(|d| d.line(&format!("body: {}", body.text), None));
        }
    }

    // --- items ------------------------------------------------------------

    fn items(&mut self, items: &[Item]) {
        for item in items {
            self.item(item, "");
        }
    }

    /// Renders an item; `prefix` is prepended to its header line (used for
    /// `decl ` inside statements).
    fn item(&mut self, item: &Item, prefix: &str) {
        let attrs = attrs_suffix(&item.attrs);
        let span = Some(item.span);
        match &item.kind {
            ItemKind::Module(m) => self.module(m, prefix, &attrs, item.span),
            ItemKind::Package(p) => {
                let lt = p
                    .lifetime
                    .map_or(String::new(), |l| format!(" {}", l.as_str()));
                self.line(&format!("{prefix}package{lt} {}{attrs}", p.name.name), span);
                self.nested(|d| d.items(&p.items));
            }
            ItemKind::Net(n) => {
                let mut s = format!("{prefix}net {}", n.net_type.as_str());
                if let Some(st) = &n.strength {
                    s.push(' ');
                    s.push_str(&strength_str(&st.levels));
                }
                if let Some(v) = n.vectored {
                    s.push(' ');
                    s.push_str(match v {
                        super::ast::Vectored::Vectored => "vectored",
                        super::ast::Vectored::Scalared => "scalared",
                    });
                }
                push_type(&mut s, &n.data_type);
                if let Some(delay) = &n.delay {
                    s.push(' ');
                    s.push_str(&delay_str(delay));
                }
                s.push(' ');
                s.push_str(&declarators_str(&n.decls));
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Var(v) => {
                let mut s = format!("{prefix}var");
                if let Some(lt) = v.lifetime {
                    s.push(' ');
                    s.push_str(lt.as_str());
                }
                if v.constant {
                    s.push_str(" const");
                }
                if v.var {
                    s.push_str(" var");
                }
                push_type(&mut s, &v.data_type);
                s.push(' ');
                s.push_str(&declarators_str(&v.decls));
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Param(p) => {
                let s = format!("{prefix}param {}{attrs}", param_decl_str(p));
                self.line(&s, span);
            }
            ItemKind::Port(p) => {
                let mut s = format!("{prefix}portdecl {}", p.direction.as_str());
                if let Some(nt) = p.net_type {
                    s.push(' ');
                    s.push_str(nt.as_str());
                }
                if p.var {
                    s.push_str(" var");
                }
                push_type(&mut s, &p.data_type);
                s.push(' ');
                s.push_str(&declarators_str(&p.decls));
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Genvar(names) => {
                let names: Vec<&str> = names.iter().map(|n| n.name.as_str()).collect();
                self.line(&format!("{prefix}genvar {}{attrs}", names.join(", ")), span);
            }
            ItemKind::Typedef(t) => {
                let mut s = format!("{prefix}typedef {}", t.name.name);
                for d in &t.dims {
                    s.push(' ');
                    s.push_str(&dim_str(d));
                }
                match &t.data_type {
                    Some(ty) => {
                        s.push_str(" = ");
                        write_type(&mut s, ty);
                    }
                    None => s.push_str(" (forward)"),
                }
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Import(refs) => {
                self.line(
                    &format!("{prefix}import {}{attrs}", package_refs_str(refs)),
                    span,
                );
            }
            ItemKind::Export(refs) => {
                self.line(
                    &format!("{prefix}export {}{attrs}", package_refs_str(refs)),
                    span,
                );
            }
            ItemKind::Function(f) | ItemKind::Task(f) => {
                let kw = if matches!(item.kind, ItemKind::Function(_)) {
                    "function"
                } else {
                    "task"
                };
                let mut s = format!("{prefix}{kw}");
                if let Some(lt) = f.lifetime {
                    s.push(' ');
                    s.push_str(lt.as_str());
                }
                if let Some(ret) = &f.ret {
                    push_type(&mut s, ret);
                }
                s.push(' ');
                s.push_str(&f.name.name);
                if f.ports.is_none() {
                    s.push_str(" (no port list)");
                }
                s.push_str(&attrs);
                self.line(&s, span);
                self.nested(|d| {
                    if let Some(ports) = &f.ports {
                        for p in ports {
                            d.port(p);
                        }
                    }
                    for st in &f.body {
                        d.stmt(st);
                    }
                });
            }
            ItemKind::Defparam(list) => {
                let parts: Vec<String> = list
                    .iter()
                    .map(|d| {
                        format!(
                            "{} = {}",
                            expr_to_string(&d.target),
                            expr_to_string(&d.value)
                        )
                    })
                    .collect();
                self.line(
                    &format!("{prefix}defparam {}{attrs}", parts.join(", ")),
                    span,
                );
            }
            ItemKind::Specify(_) => self.line(&format!("{prefix}specify (skipped){attrs}"), span),
            ItemKind::Initial(st) => {
                self.line(&format!("{prefix}initial{attrs}"), span);
                self.nested(|d| d.stmt(st));
            }
            ItemKind::Final(st) => {
                self.line(&format!("{prefix}final{attrs}"), span);
                self.nested(|d| d.stmt(st));
            }
            ItemKind::Always(kind, st) => {
                self.line(&format!("{prefix}{}{attrs}", kind.as_str()), span);
                self.nested(|d| d.stmt(st));
            }
            ItemKind::ContAssign(a) => {
                let mut s = format!("{prefix}assign");
                if let Some(st) = &a.strength {
                    s.push(' ');
                    s.push_str(&strength_str(&st.levels));
                }
                if let Some(delay) = &a.delay {
                    s.push(' ');
                    s.push_str(&delay_str(delay));
                }
                let parts: Vec<String> = a
                    .assigns
                    .iter()
                    .map(|p| format!("{} = {}", expr_to_string(&p.lhs), expr_to_string(&p.rhs)))
                    .collect();
                s.push(' ');
                s.push_str(&parts.join(", "));
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Gate(g) => {
                let mut s = format!("{prefix}gate {}", g.kind.as_str());
                if let Some(st) = &g.strength {
                    s.push(' ');
                    s.push_str(&strength_str(&st.levels));
                }
                if let Some(delay) = &g.delay {
                    s.push(' ');
                    s.push_str(&delay_str(delay));
                }
                s.push_str(&attrs);
                self.line(&s, span);
                self.nested(|d| {
                    for inst in &g.instances {
                        let mut s = String::new();
                        if let Some(n) = &inst.name {
                            s.push_str(&n.name);
                        } else {
                            s.push_str("(unnamed)");
                        }
                        for dim in &inst.dims {
                            s.push(' ');
                            s.push_str(&dim_str(dim));
                        }
                        s.push_str(" (");
                        s.push_str(&exprs_str(&inst.conns));
                        s.push(')');
                        d.line(&s, Some(inst.span));
                    }
                });
            }
            ItemKind::Instance(inst) => {
                let mut s = format!("{prefix}instance {}", inst.module.name);
                if !inst.params.is_empty() {
                    s.push_str(" #(");
                    let parts: Vec<String> = inst
                        .params
                        .iter()
                        .map(|p| match (&p.name, &p.value) {
                            (Some(n), Some(v)) => format!(".{}({})", n.name, expr_to_string(v)),
                            (Some(n), None) => format!(".{}()", n.name),
                            (None, Some(v)) => expr_to_string(v),
                            (None, None) => String::new(),
                        })
                        .collect();
                    s.push_str(&parts.join(", "));
                    s.push(')');
                }
                s.push_str(&attrs);
                self.line(&s, span);
                self.nested(|d| {
                    for i in &inst.instances {
                        let mut s = String::new();
                        s.push_str(i.name.as_ref().map_or("(unnamed)", |n| n.name.as_str()));
                        for dim in &i.dims {
                            s.push(' ');
                            s.push_str(&dim_str(dim));
                        }
                        s.push_str(" (");
                        let parts: Vec<String> = i
                            .conns
                            .iter()
                            .map(|c| match &c.kind {
                                PortConnKind::Positional(Some(e)) => expr_to_string(e),
                                PortConnKind::Positional(None) => String::new(),
                                PortConnKind::Named { name, conn } => match conn {
                                    NamedConn::Implicit => format!(".{}", name.name),
                                    NamedConn::Open => format!(".{}()", name.name),
                                    NamedConn::Expr(e) => {
                                        format!(".{}({})", name.name, expr_to_string(e))
                                    }
                                },
                                PortConnKind::Wildcard => ".*".to_string(),
                            })
                            .collect();
                        s.push_str(&parts.join(", "));
                        s.push(')');
                        d.line(&s, Some(i.span));
                    }
                });
            }
            ItemKind::Generate(items) => {
                self.line(&format!("{prefix}generate{attrs}"), span);
                self.nested(|d| d.items(items));
            }
            ItemKind::GenIf(g) => {
                self.line(
                    &format!("{prefix}gen-if ({}){attrs}", expr_to_string(&g.cond)),
                    span,
                );
                self.nested(|d| {
                    d.gen_block(&g.then_block, "then");
                    if let Some(e) = &g.else_block {
                        d.gen_block(e, "else");
                    }
                });
            }
            ItemKind::GenCase(g) => {
                self.line(
                    &format!("{prefix}gen-case ({}){attrs}", expr_to_string(&g.expr)),
                    span,
                );
                self.nested(|d| {
                    for it in &g.items {
                        let head = if it.patterns.is_empty() {
                            "default".to_string()
                        } else {
                            format!("item {}", exprs_str(&it.patterns))
                        };
                        d.line(&head, Some(it.span));
                        d.nested(|d| d.gen_block(&it.block, "block"));
                    }
                });
            }
            ItemKind::GenFor(g) => {
                let genvar = if g.genvar { "genvar " } else { "" };
                self.line(
                    &format!(
                        "{prefix}gen-for ({genvar}{} = {}; {}; {}){attrs}",
                        g.var.name,
                        expr_to_string(&g.init),
                        expr_to_string(&g.cond),
                        expr_to_string(&g.step)
                    ),
                    span,
                );
                self.nested(|d| d.gen_block(&g.body, "block"));
            }
            ItemKind::GenBlock(b) => self.gen_block(b, &format!("{prefix}block")),
            ItemKind::Alias(nets) => {
                self.line(&format!("{prefix}alias {}{attrs}", exprs_str(nets)), span);
            }
            ItemKind::Assertion(a) => self.assertion(a, prefix, &attrs, item.span),
            ItemKind::Bind(b) => {
                let mut s = format!("{prefix}bind {}", expr_to_string(&b.target));
                if !b.instances.is_empty() {
                    s.push_str(": ");
                    s.push_str(&exprs_str(&b.instances));
                }
                s.push_str(&attrs);
                self.line(&s, span);
                let inner = Item {
                    attrs: Vec::new(),
                    kind: ItemKind::Instance(b.inst.clone()),
                    span: item.span,
                };
                self.nested(|d| d.item(&inner, ""));
            }
            ItemKind::Clocking(c) => {
                let mut s = format!("{prefix}clocking");
                if c.is_default {
                    s.push_str(" default");
                }
                if c.is_global {
                    s.push_str(" global");
                }
                if let Some(n) = &c.name {
                    s.push(' ');
                    s.push_str(&n.name);
                }
                if let Some(ev) = &c.event {
                    s.push(' ');
                    s.push_str(&event_control_str(ev));
                }
                s.push_str(&attrs);
                self.line(&s, span);
                self.raw_body(&c.body);
            }
            ItemKind::PropertyDecl(p) => {
                let kw = if p.is_sequence {
                    "sequence"
                } else {
                    "property"
                };
                self.line(&format!("{prefix}{kw} {}{attrs}", p.name.name), span);
                self.raw_body(&p.body);
            }
            ItemKind::Modport(list) => {
                for m in list {
                    self.line(
                        &format!("{prefix}modport {}{attrs}", m.name.name),
                        Some(m.span),
                    );
                    self.nested(|d| {
                        for it in &m.items {
                            let s = match &it.kind {
                                ModportItemKind::Port {
                                    direction,
                                    name,
                                    expr,
                                } => match expr {
                                    Some(e) => format!(
                                        "{} .{}({})",
                                        direction.as_str(),
                                        name.name,
                                        expr_to_string(e)
                                    ),
                                    None => format!("{} {}", direction.as_str(), name.name),
                                },
                                ModportItemKind::Import(n) => format!("import {}", n.name),
                                ModportItemKind::Export(n) => format!("export {}", n.name),
                                ModportItemKind::Clocking(n) => format!("clocking {}", n.name),
                            };
                            d.line(&s, Some(it.span));
                        }
                    });
                }
            }
            ItemKind::Timeunit { unit, precision } => {
                let mut s = format!("{prefix}timeunit {}", literal_str(unit));
                if let Some(p) = precision {
                    s.push_str(" / ");
                    s.push_str(&literal_str(p));
                }
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Timeprecision(p) => {
                self.line(
                    &format!("{prefix}timeprecision {}{attrs}", literal_str(p)),
                    span,
                );
            }
            ItemKind::Directive(d) => {
                let mut s = format!("{prefix}directive `{}", d.name);
                if !d.args.is_empty() {
                    s.push(' ');
                    s.push_str(&d.args);
                }
                s.push_str(&attrs);
                self.line(&s, span);
            }
            ItemKind::Table(rows) => {
                self.line(&format!("{prefix}table{attrs}"), span);
                self.nested(|d| {
                    for r in rows {
                        d.line(&r.text, Some(r.span));
                    }
                });
            }
            ItemKind::Empty => self.line(&format!("{prefix}empty{attrs}"), span),
        }
    }

    fn module(&mut self, m: &Module, prefix: &str, attrs: &str, span: Span) {
        let lt = m
            .lifetime
            .map_or(String::new(), |l| format!(" {}", l.as_str()));
        self.line(
            &format!("{prefix}{}{lt} {}{attrs}", m.kind.as_str(), m.name.name),
            Some(span),
        );
        self.nested(|d| {
            if !m.imports.is_empty() {
                d.line(&format!("import {}", package_refs_str(&m.imports)), None);
            }
            if let Some(params) = &m.params {
                if params.is_empty() {
                    d.line("params (empty)", None);
                }
                for p in params {
                    d.line(&format!("param {}", param_decl_str(p)), None);
                }
            }
            match &m.ports {
                Ports::None => {}
                Ports::NonAnsi(ports) => {
                    d.line("ports (non-ansi)", None);
                    d.nested(|d| {
                        for p in ports {
                            let s = match (&p.name, &p.expr) {
                                (Some(n), Some(e)) => format!(".{}({})", n.name, expr_to_string(e)),
                                (Some(n), None) => format!(".{}()", n.name),
                                (None, Some(e)) => expr_to_string(e),
                                (None, None) => "(empty)".to_string(),
                            };
                            d.line(&format!("port {s}"), Some(p.span));
                        }
                    });
                }
                Ports::Ansi(ports) => {
                    if ports.is_empty() {
                        d.line("ports (empty)", None);
                    }
                    for p in ports {
                        d.port(p);
                    }
                }
            }
            d.items(&m.items);
        });
    }

    fn port(&mut self, p: &Port) {
        let mut s = String::from("port");
        if let Some(dir) = p.direction {
            s.push(' ');
            s.push_str(dir.as_str());
        }
        if let Some(nt) = p.net_type {
            s.push(' ');
            s.push_str(nt.as_str());
        }
        if p.var {
            s.push_str(" var");
        }
        push_type(&mut s, &p.data_type);
        s.push(' ');
        s.push_str(&p.name.name);
        for d in &p.dims {
            s.push(' ');
            s.push_str(&dim_str(d));
        }
        if let Some(e) = &p.default {
            s.push_str(" = ");
            write_expr(&mut s, e);
        }
        s.push_str(&attrs_suffix(&p.attrs));
        self.line(&s, Some(p.span));
    }

    fn gen_block(&mut self, b: &GenBlock, head: &str) {
        let mut s = head.to_string();
        if let Some(l) = &b.label {
            s.push_str(" : ");
            s.push_str(&l.name);
        }
        self.line(&s, Some(b.span));
        self.nested(|d| d.items(&b.items));
    }

    fn assertion(&mut self, a: &Assertion, prefix: &str, attrs: &str, span: Span) {
        let mut s = prefix.to_string();
        if let Some(l) = &a.label {
            s.push_str(&l.name);
            s.push_str(": ");
        }
        s.push_str(a.kind.as_str());
        match a.deferred {
            Some(super::ast::Deferred::Observed) => s.push_str(" #0"),
            Some(super::ast::Deferred::Final) => s.push_str(" final"),
            None => {}
        }
        match &a.spec {
            AssertSpec::Expr(e) => {
                s.push_str(" (");
                write_expr(&mut s, e);
                s.push(')');
            }
            AssertSpec::Property(raw) => {
                s.push_str(" property (");
                s.push_str(&raw.text);
                s.push(')');
            }
        }
        s.push_str(attrs);
        self.line(&s, Some(span));
        self.nested(|d| {
            if let Some(t) = &a.then_stmt {
                d.stmt(t);
            }
            if let Some(e) = &a.else_stmt {
                d.line("else", None);
                d.nested(|d| d.stmt(e));
            }
        });
    }

    // --- statements -------------------------------------------------------

    fn stmt(&mut self, stmt: &Stmt) {
        let mut head = String::new();
        if let Some(l) = &stmt.label {
            head.push_str(&l.name);
            head.push_str(": ");
        }
        let attrs = attrs_suffix(&stmt.attrs);
        let span = Some(stmt.span);
        match &stmt.kind {
            StmtKind::Null => self.line(&format!("{head}null{attrs}"), span),
            StmtKind::Block(b) => {
                head.push_str("begin");
                if let Some(l) = &b.label {
                    head.push_str(" : ");
                    head.push_str(&l.name);
                }
                self.line(&format!("{head}{attrs}"), span);
                self.nested(|d| {
                    for s in &b.stmts {
                        d.stmt(s);
                    }
                });
            }
            StmtKind::Fork(b, join) => {
                head.push_str("fork ");
                head.push_str(join.as_str());
                if let Some(l) = &b.label {
                    head.push_str(" : ");
                    head.push_str(&l.name);
                }
                self.line(&format!("{head}{attrs}"), span);
                self.nested(|d| {
                    for s in &b.stmts {
                        d.stmt(s);
                    }
                });
            }
            StmtKind::Assign(a) => {
                let mut s = format!("{head}{} {}", expr_to_string(&a.lhs), a.op.as_str());
                if let Some(t) = &a.timing {
                    s.push(' ');
                    s.push_str(&timing_str(&t.kind));
                }
                s.push(' ');
                write_expr(&mut s, &a.rhs);
                s.push_str(&attrs);
                self.line(&s, span);
            }
            StmtKind::Expr(e) => {
                self.line(&format!("{head}expr {}{attrs}", expr_to_string(e)), span);
            }
            StmtKind::If(i) => {
                let q = i
                    .qualifier
                    .map_or(String::new(), |q| format!("{} ", q.as_str()));
                self.line(
                    &format!("{head}{q}if ({}){attrs}", expr_to_string(&i.cond)),
                    span,
                );
                self.nested(|d| d.stmt(&i.then_stmt));
                if let Some(e) = &i.else_stmt {
                    self.line("else", None);
                    self.nested(|d| d.stmt(e));
                }
            }
            StmtKind::Case(c) => {
                let q = c
                    .qualifier
                    .map_or(String::new(), |q| format!("{} ", q.as_str()));
                let inside = if c.inside { " inside" } else { "" };
                self.line(
                    &format!(
                        "{head}{q}{} ({}){inside}{attrs}",
                        c.kind.as_str(),
                        expr_to_string(&c.expr)
                    ),
                    span,
                );
                self.nested(|d| {
                    for it in &c.items {
                        let h = if it.patterns.is_empty() {
                            "default".to_string()
                        } else {
                            format!("item {}", exprs_str(&it.patterns))
                        };
                        d.line(&h, Some(it.span));
                        d.nested(|d| d.stmt(&it.body));
                    }
                });
            }
            StmtKind::For(f) => {
                let init: Vec<String> = f
                    .init
                    .iter()
                    .map(|i| match i {
                        ForInit::Decl(v) => {
                            let mut s = String::new();
                            if v.var {
                                s.push_str("var ");
                            }
                            write_type(&mut s, &v.data_type);
                            s.push(' ');
                            s.push_str(&declarators_str(&v.decls));
                            s
                        }
                        ForInit::Assign(e) => expr_to_string(e),
                    })
                    .collect();
                let cond = f.cond.as_ref().map_or(String::new(), expr_to_string);
                self.line(
                    &format!(
                        "{head}for ({}; {cond}; {}){attrs}",
                        init.join(", "),
                        exprs_str(&f.step)
                    ),
                    span,
                );
                self.nested(|d| d.stmt(&f.body));
            }
            StmtKind::While(c, body) => {
                self.line(&format!("{head}while ({}){attrs}", expr_to_string(c)), span);
                self.nested(|d| d.stmt(body));
            }
            StmtKind::DoWhile(body, c) => {
                self.line(
                    &format!("{head}do-while ({}){attrs}", expr_to_string(c)),
                    span,
                );
                self.nested(|d| d.stmt(body));
            }
            StmtKind::Repeat(n, body) => {
                self.line(
                    &format!("{head}repeat ({}){attrs}", expr_to_string(n)),
                    span,
                );
                self.nested(|d| d.stmt(body));
            }
            StmtKind::Forever(body) => {
                self.line(&format!("{head}forever{attrs}"), span);
                self.nested(|d| d.stmt(body));
            }
            StmtKind::Foreach(f) => {
                let vars: Vec<&str> = f
                    .vars
                    .iter()
                    .map(|v| v.as_ref().map_or("", |i| i.name.as_str()))
                    .collect();
                self.line(
                    &format!(
                        "{head}foreach {}[{}]{attrs}",
                        expr_to_string(&f.array),
                        vars.join(", ")
                    ),
                    span,
                );
                self.nested(|d| d.stmt(&f.body));
            }
            StmtKind::Break => self.line(&format!("{head}break{attrs}"), span),
            StmtKind::Continue => self.line(&format!("{head}continue{attrs}"), span),
            StmtKind::Return(v) => match v {
                Some(e) => self.line(&format!("{head}return {}{attrs}", expr_to_string(e)), span),
                None => self.line(&format!("{head}return{attrs}"), span),
            },
            StmtKind::Disable(e) => {
                self.line(&format!("{head}disable {}{attrs}", expr_to_string(e)), span);
            }
            StmtKind::DisableFork => self.line(&format!("{head}disable fork{attrs}"), span),
            StmtKind::Timing(t, body) => {
                self.line(&format!("{head}{}{attrs}", timing_str(&t.kind)), span);
                self.nested(|d| d.stmt(body));
            }
            StmtKind::Wait(c, body) => {
                self.line(&format!("{head}wait ({}){attrs}", expr_to_string(c)), span);
                self.nested(|d| d.stmt(body));
            }
            StmtKind::WaitFork => self.line(&format!("{head}wait fork{attrs}"), span),
            StmtKind::ProcAssign(l, r) => self.line(
                &format!(
                    "{head}proc-assign {} = {}{attrs}",
                    expr_to_string(l),
                    expr_to_string(r)
                ),
                span,
            ),
            StmtKind::Deassign(l) => {
                self.line(
                    &format!("{head}deassign {}{attrs}", expr_to_string(l)),
                    span,
                );
            }
            StmtKind::Force(l, r) => self.line(
                &format!(
                    "{head}force {} = {}{attrs}",
                    expr_to_string(l),
                    expr_to_string(r)
                ),
                span,
            ),
            StmtKind::Release(l) => {
                self.line(&format!("{head}release {}{attrs}", expr_to_string(l)), span);
            }
            StmtKind::Trigger {
                nonblocking,
                target,
            } => {
                let op = if *nonblocking { "->>" } else { "->" };
                self.line(
                    &format!("{head}{op} {}{attrs}", expr_to_string(target)),
                    span,
                );
            }
            StmtKind::Assert(a) => self.assertion(a, &head, &attrs, stmt.span),
            StmtKind::Decl(item) => {
                let inner = Item {
                    attrs: stmt.attrs.clone(),
                    kind: item.kind.clone(),
                    span: stmt.span,
                };
                self.item(&inner, &format!("{head}decl "));
            }
        }
    }
}

// --- inline renderers ------------------------------------------------------

fn attrs_suffix(attrs: &[Attribute]) -> String {
    if attrs.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = attrs
        .iter()
        .map(|a| match &a.value {
            Some(v) => format!("{} = {}", a.name.name, expr_to_string(v)),
            None => a.name.name.clone(),
        })
        .collect();
    format!(" (* {} *)", parts.join(", "))
}

fn strength_str(levels: &[super::ast::StrengthLevel]) -> String {
    let parts: Vec<&str> = levels.iter().map(|l| l.as_str()).collect();
    format!("({})", parts.join(", "))
}

fn delay_str(delay: &Delay) -> String {
    if let [value] = delay.values.as_slice() {
        let text = expr_to_string(value);
        // Names and literals need no parentheses, and an operator
        // expression already comes parenthesised.
        let bare = matches!(value.kind, ExprKind::Literal(_) | ExprKind::Ident(_))
            || text.starts_with('(');
        return if bare {
            format!("#{text}")
        } else {
            format!("#({text})")
        };
    }
    format!("#({})", exprs_str(&delay.values))
}

fn timing_str(kind: &TimingKind) -> String {
    match kind {
        TimingKind::Delay(d) => delay_str(d),
        TimingKind::Event(e) => event_control_str(e),
        TimingKind::RepeatEvent(n, e) => {
            format!("repeat ({}) {}", expr_to_string(n), event_control_str(e))
        }
    }
}

fn event_control_str(ev: &EventControl) -> String {
    match &ev.kind {
        EventControlKind::Any => "@*".to_string(),
        EventControlKind::List(list) => {
            let parts: Vec<String> = list
                .iter()
                .map(|e| {
                    let mut s = String::new();
                    if let Some(edge) = e.edge {
                        s.push_str(edge.as_str());
                        s.push(' ');
                    }
                    write_expr(&mut s, &e.expr);
                    if let Some(c) = &e.iff {
                        s.push_str(" iff ");
                        write_expr(&mut s, c);
                    }
                    s
                })
                .collect();
            format!("@({})", parts.join(", "))
        }
    }
}

fn package_refs_str(refs: &[super::ast::PackageRef]) -> String {
    let parts: Vec<String> = refs
        .iter()
        .map(|r| match &r.item {
            Some(i) => format!("{}::{}", r.package.name, i.name),
            None => format!("{}::*", r.package.name),
        })
        .collect();
    parts.join(", ")
}

fn param_decl_str(p: &ParamDecl) -> String {
    let mut s = p.kind.as_str().to_string();
    if p.is_type {
        s.push_str(" type");
    }
    push_type(&mut s, &p.data_type);
    s.push(' ');
    s.push_str(&declarators_str(&p.decls));
    s
}

fn declarators_str(decls: &[Declarator]) -> String {
    let parts: Vec<String> = decls
        .iter()
        .map(|d| {
            let mut s = d.name.name.clone();
            for dim in &d.dims {
                s.push(' ');
                s.push_str(&dim_str(dim));
            }
            if let Some(init) = &d.init {
                s.push_str(" = ");
                write_expr(&mut s, init);
            }
            s
        })
        .collect();
    parts.join(", ")
}

fn exprs_str(exprs: &[Expr]) -> String {
    let parts: Vec<String> = exprs.iter().map(expr_to_string).collect();
    parts.join(", ")
}

fn literal_str(lit: &Literal) -> String {
    match lit {
        Literal::Number { text, .. } => text.clone(),
        Literal::Str { value, .. } => format!("{value:?}"),
        Literal::Null(_) => "null".to_string(),
        Literal::Unbounded(_) => "$".to_string(),
    }
}

fn dim_str(dim: &Dim) -> String {
    match &dim.kind {
        DimKind::Range(a, b) => format!("[{}:{}]", expr_to_string(a), expr_to_string(b)),
        DimKind::Size(n) => format!("[{}]", expr_to_string(n)),
        DimKind::Unsized => "[]".to_string(),
        DimKind::Queue(None) => "[$]".to_string(),
        DimKind::Queue(Some(n)) => format!("[$:{}]", expr_to_string(n)),
        DimKind::Assoc(None) => "[*]".to_string(),
        DimKind::Assoc(Some(t)) => format!("[{}]", type_to_string(t)),
    }
}

/// Appends a space and the type, unless the type is entirely implicit.
fn push_type(s: &mut String, ty: &DataType) {
    let t = type_to_string(ty);
    if !t.is_empty() {
        s.push(' ');
        s.push_str(&t);
    }
}

fn write_type(s: &mut String, ty: &DataType) {
    let mut parts: Vec<String> = Vec::new();
    match &ty.kind {
        DataTypeKind::Implicit => {}
        DataTypeKind::Integer(i) => parts.push(i.as_str().to_string()),
        DataTypeKind::Real(r) => parts.push(r.as_str().to_string()),
        DataTypeKind::String => parts.push("string".to_string()),
        DataTypeKind::Chandle => parts.push("chandle".to_string()),
        DataTypeKind::Event => parts.push("event".to_string()),
        DataTypeKind::Void => parts.push("void".to_string()),
        DataTypeKind::Enum(e) => {
            let mut h = String::from("enum");
            if let Some(base) = &e.base {
                h.push(' ');
                write_type(&mut h, base);
            }
            let vs: Vec<String> = e
                .variants
                .iter()
                .map(|v| {
                    let mut vs = v.name.name.clone();
                    if let Some(r) = &v.range {
                        vs.push_str(&dim_str(r));
                    }
                    if let Some(val) = &v.value {
                        vs.push_str(" = ");
                        write_expr(&mut vs, val);
                    }
                    vs
                })
                .collect();
            h.push_str(" {");
            h.push_str(&vs.join(", "));
            h.push('}');
            parts.push(h);
        }
        DataTypeKind::Struct(st) => {
            let mut h = String::from(if st.is_union { "union" } else { "struct" });
            if st.tagged {
                h.push_str(" tagged");
            }
            if st.packed {
                h.push_str(" packed");
            }
            if let Some(sg) = ty.signing {
                h.push(' ');
                h.push_str(signing_str(sg));
            }
            h.push_str(" {");
            let ms: Vec<String> = st
                .members
                .iter()
                .map(|m| {
                    format!(
                        "{} {};",
                        type_to_string(&m.data_type),
                        declarators_str(&m.decls)
                    )
                })
                .collect();
            h.push_str(&ms.join(" "));
            h.push('}');
            parts.push(h);
            for d in &ty.packed {
                parts.push(dim_str(d));
            }
            s.push_str(&parts.join(" "));
            return;
        }
        DataTypeKind::Named {
            package,
            name,
            member,
        } => {
            let mut h = String::new();
            if let Some(p) = package {
                h.push_str(&p.name);
                h.push_str("::");
            }
            h.push_str(&name.name);
            if let Some(m) = member {
                h.push('.');
                h.push_str(&m.name);
            }
            parts.push(h);
        }
        DataTypeKind::Interface { modport } => match modport {
            Some(m) => parts.push(format!("interface.{}", m.name)),
            None => parts.push("interface".to_string()),
        },
        DataTypeKind::TypeOf(e) => parts.push(format!("type({})", expr_to_string(e))),
    }
    if let Some(sg) = ty.signing {
        parts.push(signing_str(sg).to_string());
    }
    for d in &ty.packed {
        parts.push(dim_str(d));
    }
    s.push_str(&parts.join(" "));
}

fn signing_str(s: super::ast::Signing) -> &'static str {
    match s {
        super::ast::Signing::Signed => "signed",
        super::ast::Signing::Unsigned => "unsigned",
    }
}

fn write_args(s: &mut String, args: &[Arg]) {
    let parts: Vec<String> = args
        .iter()
        .map(|a| match (&a.name, &a.value) {
            (Some(n), Some(v)) => format!(".{}({})", n.name, expr_to_string(v)),
            (Some(n), None) => format!(".{}()", n.name),
            (None, Some(v)) => expr_to_string(v),
            (None, None) => String::new(),
        })
        .collect();
    s.push_str(&parts.join(", "));
}

fn write_expr(s: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::Literal(l) => s.push_str(&literal_str(l)),
        ExprKind::Ident(i) => s.push_str(&i.name),
        ExprKind::SystemIdent(i) => {
            s.push('$');
            s.push_str(&i.name);
        }
        ExprKind::Scoped { scope, name } => {
            write_expr(s, scope);
            s.push_str("::");
            s.push_str(&name.name);
        }
        ExprKind::Member { base, name } => {
            write_expr(s, base);
            s.push('.');
            s.push_str(&name.name);
        }
        ExprKind::Index { base, index } => {
            write_expr(s, base);
            s.push('[');
            write_expr(s, index);
            s.push(']');
        }
        ExprKind::Range {
            base,
            kind,
            left,
            right,
        } => {
            write_expr(s, base);
            s.push('[');
            write_expr(s, left);
            s.push_str(kind.as_str());
            write_expr(s, right);
            s.push(']');
        }
        ExprKind::Unary { op, operand } => {
            s.push('(');
            s.push_str(op.as_str());
            write_expr(s, operand);
            s.push(')');
        }
        ExprKind::Binary { op, lhs, rhs } => {
            s.push('(');
            write_expr(s, lhs);
            s.push(' ');
            s.push_str(op.as_str());
            s.push(' ');
            write_expr(s, rhs);
            s.push(')');
        }
        ExprKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            s.push('(');
            write_expr(s, cond);
            s.push_str(" ? ");
            write_expr(s, then_expr);
            s.push_str(" : ");
            write_expr(s, else_expr);
            s.push(')');
        }
        ExprKind::Concat(elems) => {
            s.push('{');
            s.push_str(&exprs_str(elems));
            s.push('}');
        }
        ExprKind::Replicate { count, elems } => {
            s.push('{');
            write_expr(s, count);
            s.push('{');
            s.push_str(&exprs_str(elems));
            s.push_str("}}");
        }
        ExprKind::Streaming {
            right_to_left,
            slice,
            elems,
        } => {
            s.push('{');
            s.push_str(if *right_to_left { "<<" } else { ">>" });
            if let Some(sl) = slice {
                s.push(' ');
                write_expr(s, sl);
            }
            s.push_str(" {");
            s.push_str(&exprs_str(elems));
            s.push_str("}}");
        }
        ExprKind::Pattern(items) => {
            s.push_str("'{");
            let parts: Vec<String> = items
                .iter()
                .map(|it| match &it.key {
                    Some(k) => format!("{}: {}", expr_to_string(k), expr_to_string(&it.value)),
                    None => expr_to_string(&it.value),
                })
                .collect();
            s.push_str(&parts.join(", "));
            s.push('}');
        }
        ExprKind::Call { callee, args } => {
            write_expr(s, callee);
            s.push('(');
            write_args(s, args);
            s.push(')');
        }
        ExprKind::New(args) => {
            s.push_str("new");
            if !args.is_empty() {
                s.push('(');
                s.push_str(&exprs_str(args));
                s.push(')');
            }
        }
        ExprKind::Cast { target, expr } => {
            match target {
                CastTarget::Type(t) => write_type(s, t),
                CastTarget::Size(n) => write_expr(s, n),
                CastTarget::Signing(sg) => s.push_str(signing_str(*sg)),
                CastTarget::Const => s.push_str("const"),
            }
            // A pattern operand brings its own `'{`.
            if matches!(expr.kind, ExprKind::Pattern(_)) {
                write_expr(s, expr);
            } else {
                s.push_str("'(");
                write_expr(s, expr);
                s.push(')');
            }
        }
        ExprKind::Inside { expr, set } => {
            s.push('(');
            write_expr(s, expr);
            s.push_str(" inside {");
            s.push_str(&exprs_str(set));
            s.push_str("})");
        }
        ExprKind::ValueRange { low, high } => {
            s.push('[');
            write_expr(s, low);
            s.push(':');
            write_expr(s, high);
            s.push(']');
        }
        ExprKind::MinTypMax { min, typ, max } => {
            write_expr(s, min);
            s.push(':');
            write_expr(s, typ);
            s.push(':');
            write_expr(s, max);
        }
        ExprKind::Type(t) => write_type(s, t),
        ExprKind::Assign { lhs, op, rhs } => {
            s.push('(');
            write_expr(s, lhs);
            s.push(' ');
            s.push_str(op.as_str());
            s.push(' ');
            write_expr(s, rhs);
            s.push(')');
        }
        ExprKind::IncDec {
            increment,
            prefix,
            target,
        } => {
            let op = if *increment { "++" } else { "--" };
            if *prefix {
                s.push_str(op);
                write_expr(s, target);
            } else {
                write_expr(s, target);
                s.push_str(op);
            }
        }
        ExprKind::Default => s.push_str("default"),
    }
}
