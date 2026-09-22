//! A deterministic text rendering of the AST, for golden tests and
//! debugging.
//!
//! Every statement, declaration and design unit becomes one line of the
//! form `Kind [label] @line:col details`, with its children indented by two
//! spaces. Expressions, names, subtypes and ranges are rendered inline by
//! [`dump_expr`] and friends in a compact prefix notation: `(+ a (* b c))`
//! for operators, `f(1, b => 2)` for calls, `(agg others => '0')` for
//! aggregates, `(paren e)` for parenthesised expressions. Spans of inline
//! pieces are not shown; those of the line-level nodes are, so the golden
//! files also pin down where each construct starts.

use std::fmt::Write;

use super::ast::*;
use crate::source::SourceFile;

/// Renders a design file.
pub fn dump(file: &DesignFile, src: &SourceFile) -> String {
    let mut d = Dumper {
        out: String::new(),
        indent: 0,
        src,
    };
    d.line("DesignFile");
    d.nested(|d| {
        for u in &file.units {
            d.design_unit(u);
        }
    });
    d.out
}

/// Renders an expression inline.
pub fn dump_expr(e: &Expr) -> String {
    match e {
        Expr::Binary { op, lhs, rhs, .. } => {
            format!("({} {} {})", op.as_str(), dump_expr(lhs), dump_expr(rhs))
        }
        Expr::Unary { op, operand, .. } => format!("({} {})", op.as_str(), dump_expr(operand)),
        Expr::Name(n) => dump_name(n),
        Expr::Literal(l) => dump_literal(l),
        Expr::Aggregate(a) => dump_aggregate(a),
        Expr::Qualified {
            type_mark, operand, ..
        } => format!(
            "{}'{}",
            dump_name(type_mark),
            dump_qualified_operand(operand)
        ),
        Expr::Allocator { kind, .. } => match &**kind {
            Allocator::Subtype(s) => format!("(new {})", dump_subtype(s)),
            Allocator::Qualified { type_mark, operand } => format!(
                "(new {}'{})",
                dump_name(type_mark),
                dump_qualified_operand(operand)
            ),
        },
        Expr::Paren { inner, .. } => format!("(paren {})", dump_expr(inner)),
        Expr::Open(_) => "open".to_owned(),
        Expr::Error(_) => "<error>".to_owned(),
    }
}

/// The operand of a qualified expression, without the extra `paren`.
fn dump_qualified_operand(e: &Expr) -> String {
    match e {
        Expr::Paren { inner, .. } => format!("({})", dump_expr(inner)),
        other => dump_expr(other),
    }
}

/// Renders a name inline.
pub fn dump_name(n: &Name) -> String {
    match n {
        Name::Simple(i) => dump_ident(i),
        Name::Operator { symbol, .. } => format!("\"{symbol}\""),
        Name::Char { ch, .. } => format!("'{ch}'"),
        Name::Selected { prefix, suffix, .. } => {
            let s = match suffix {
                Suffix::Designator(d) => dump_designator(d),
                Suffix::All(_) => "all".to_owned(),
            };
            format!("{}.{s}", dump_name(prefix))
        }
        Name::Call { prefix, args, .. } => format!("{}({})", dump_name(prefix), dump_assocs(args)),
        Name::Slice { prefix, range, .. } => {
            format!("{}({})", dump_name(prefix), dump_discrete_range(range))
        }
        Name::Attribute {
            prefix,
            signature,
            attribute,
            ..
        } => format!(
            "{}{}'{}",
            dump_name(prefix),
            signature
                .as_ref()
                .map(|s| dump_signature(s))
                .unwrap_or_default(),
            dump_ident(attribute)
        ),
        Name::External(e) => {
            let class = match e.class {
                ExternalClass::Constant => "constant",
                ExternalClass::Signal => "signal",
                ExternalClass::Variable => "variable",
            };
            format!(
                "<<{class} {} : {}>>",
                dump_path(&e.path),
                dump_subtype(&e.subtype)
            )
        }
    }
}

fn dump_ident(i: &Ident) -> String {
    if i.extended {
        format!("\\{}\\", i.name)
    } else {
        i.name.clone()
    }
}

fn dump_designator(d: &Designator) -> String {
    match d {
        Designator::Ident(i) => dump_ident(i),
        Designator::Char { ch, .. } => format!("'{ch}'"),
        Designator::Operator { symbol, .. } => format!("\"{symbol}\""),
    }
}

fn dump_path(p: &ExternalPath) -> String {
    let mut s = match p.kind {
        ExternalPathKind::Package => "@".to_owned(),
        ExternalPathKind::Absolute => ".".to_owned(),
        ExternalPathKind::Relative(n) => "^.".repeat(n as usize),
    };
    let elements: Vec<String> = p
        .elements
        .iter()
        .map(|e| match &e.index {
            Some(i) => format!("{}({})", dump_ident(&e.name), dump_expr(i)),
            None => dump_ident(&e.name),
        })
        .collect();
    s.push_str(&elements.join("."));
    s
}

fn dump_literal(l: &Literal) -> String {
    match &l.kind {
        LiteralKind::Integer(s) | LiteralKind::Real(s) | LiteralKind::BitString(s) => s.clone(),
        LiteralKind::Physical { value, unit } => format!("{value} {}", dump_ident(unit)),
        LiteralKind::Char(c) => format!("'{c}'"),
        LiteralKind::String(s) => format!("\"{s}\""),
        LiteralKind::Null => "null".to_owned(),
    }
}

fn dump_aggregate(a: &Aggregate) -> String {
    let items: Vec<String> = a
        .elements
        .iter()
        .map(|el| {
            if el.choices.is_empty() {
                dump_expr(&el.value)
            } else {
                format!("{} => {}", dump_choices(&el.choices), dump_expr(&el.value))
            }
        })
        .collect();
    format!("(agg {})", items.join(", "))
}

/// Renders a choice list inline.
pub fn dump_choices(choices: &[Choice]) -> String {
    let items: Vec<String> = choices
        .iter()
        .map(|c| match c {
            Choice::Expr(e) => dump_expr(e),
            Choice::Range(r) => dump_discrete_range(r),
            Choice::Others(_) => "others".to_owned(),
        })
        .collect();
    items.join(" | ")
}

/// Renders an association list inline, without parentheses.
pub fn dump_assocs(args: &[AssociationElement]) -> String {
    let items: Vec<String> = args
        .iter()
        .map(|a| {
            let actual = match &a.actual {
                Actual::Expr(e) => dump_expr(e),
                Actual::Open(_) => "open".to_owned(),
                Actual::Inertial(e) => format!("inertial {}", dump_expr(e)),
                Actual::Range(r) => dump_discrete_range(r),
            };
            match &a.formal {
                Some(f) => format!("{} => {actual}", dump_expr(f)),
                None => actual,
            }
        })
        .collect();
    items.join(", ")
}

/// Renders a subtype indication inline.
pub fn dump_subtype(s: &SubtypeIndication) -> String {
    let mut out = String::new();
    if let Some(r) = &s.resolution {
        out.push_str(&dump_resolution(r));
        out.push(' ');
    }
    out.push_str(&dump_name(&s.type_mark));
    if let Some(c) = &s.constraint {
        out.push_str(&dump_constraint(c));
    }
    out
}

fn dump_resolution(r: &ResolutionIndication) -> String {
    match r {
        ResolutionIndication::Function(n) => dump_name(n),
        ResolutionIndication::Array(inner, _) => format!("({})", dump_resolution(inner)),
        ResolutionIndication::Record(entries, _) => {
            let items: Vec<String> = entries
                .iter()
                .map(|e| format!("{} {}", dump_ident(&e.name), dump_resolution(&e.resolution)))
                .collect();
            format!("({})", items.join(", "))
        }
    }
}

fn dump_constraint(c: &Constraint) -> String {
    match c {
        Constraint::Range(r) => format!(" range {}", dump_range(r)),
        Constraint::Array {
            indices, element, ..
        } => {
            let inner = if indices.is_empty() {
                "open".to_owned()
            } else {
                let items: Vec<String> = indices.iter().map(dump_discrete_range).collect();
                items.join(", ")
            };
            let element = element
                .as_ref()
                .map(|e| dump_constraint(e))
                .unwrap_or_default();
            format!("({inner}){element}")
        }
        Constraint::Record(entries, _) => {
            let items: Vec<String> = entries
                .iter()
                .map(|e| format!("{}{}", dump_ident(&e.name), dump_constraint(&e.constraint)))
                .collect();
            format!("({})", items.join(", "))
        }
    }
}

/// Renders a range inline.
pub fn dump_range(r: &Range) -> String {
    match r {
        Range::Bounds {
            left,
            direction,
            right,
            ..
        } => format!(
            "{} {} {}",
            dump_expr(left),
            direction.as_str(),
            dump_expr(right)
        ),
        Range::Attribute(n) => dump_name(n),
    }
}

/// Renders a discrete range inline.
pub fn dump_discrete_range(r: &DiscreteRange) -> String {
    match r {
        DiscreteRange::Range(r) => dump_range(r),
        DiscreteRange::Subtype(s) => dump_subtype(s),
    }
}

fn dump_signature(s: &Signature) -> String {
    let params: Vec<String> = s.params.iter().map(dump_name).collect();
    let mut out = format!("[{}", params.join(", "));
    if let Some(r) = &s.return_type {
        if !params.is_empty() {
            out.push(' ');
        }
        out.push_str("return ");
        out.push_str(&dump_name(r));
    }
    out.push(']');
    out
}

fn dump_waveform(w: &Waveform) -> String {
    match w {
        Waveform::Unaffected(_) => "unaffected".to_owned(),
        Waveform::Elements(els) => {
            let items: Vec<String> = els
                .iter()
                .map(|e| match &e.after {
                    Some(t) => format!("{} after {}", dump_expr(&e.value), dump_expr(t)),
                    None => dump_expr(&e.value),
                })
                .collect();
            items.join(", ")
        }
    }
}

fn dump_target(t: &Target) -> String {
    match t {
        Target::Name(n) => dump_name(n),
        Target::Aggregate(a) => dump_aggregate(a),
    }
}

fn dump_delay(d: &DelayMechanism) -> String {
    match d {
        DelayMechanism::Transport(_) => "transport ".to_owned(),
        DelayMechanism::Inertial { reject: None, .. } => "inertial ".to_owned(),
        DelayMechanism::Inertial {
            reject: Some(r), ..
        } => format!("reject {} inertial ", dump_expr(r)),
    }
}

fn dump_mode(m: Mode) -> &'static str {
    match m {
        Mode::In => "in",
        Mode::Out => "out",
        Mode::Inout => "inout",
        Mode::Buffer => "buffer",
        Mode::Linkage => "linkage",
    }
}

fn dump_signal_rhs(rhs: &SignalAssignmentRhs) -> String {
    match rhs {
        SignalAssignmentRhs::Simple(w) => dump_waveform(w),
        SignalAssignmentRhs::Conditional(arms) => {
            let items: Vec<String> = arms
                .iter()
                .map(|a| match &a.condition {
                    Some(c) => format!("{} when {}", dump_waveform(&a.waveform), dump_expr(c)),
                    None => dump_waveform(&a.waveform),
                })
                .collect();
            items.join(" else ")
        }
        SignalAssignmentRhs::Selected {
            selector,
            matching,
            arms,
        } => {
            let items: Vec<String> = arms
                .iter()
                .map(|a| {
                    format!(
                        "{} when {}",
                        dump_waveform(&a.waveform),
                        dump_choices(&a.choices)
                    )
                })
                .collect();
            format!(
                "with {} select{} {}",
                dump_expr(selector),
                if *matching { "?" } else { "" },
                items.join(", ")
            )
        }
        SignalAssignmentRhs::Force { mode, arms } => format!(
            "force {}{}",
            mode.map(|m| format!("{} ", dump_mode(m)))
                .unwrap_or_default(),
            dump_conditional_exprs(arms)
        ),
        SignalAssignmentRhs::Release { mode } => format!(
            "release{}",
            mode.map(|m| format!(" {}", dump_mode(m)))
                .unwrap_or_default()
        ),
    }
}

fn dump_conditional_exprs(arms: &[ConditionalExpr]) -> String {
    let items: Vec<String> = arms
        .iter()
        .map(|a| match &a.condition {
            Some(c) => format!("{} when {}", dump_expr(&a.value), dump_expr(c)),
            None => dump_expr(&a.value),
        })
        .collect();
    items.join(" else ")
}

fn dump_variable_rhs(rhs: &VariableAssignmentRhs) -> String {
    match rhs {
        VariableAssignmentRhs::Simple(e) => dump_expr(e),
        VariableAssignmentRhs::Conditional(arms) => dump_conditional_exprs(arms),
        VariableAssignmentRhs::Selected {
            selector,
            matching,
            arms,
        } => {
            let items: Vec<String> = arms
                .iter()
                .map(|a| format!("{} when {}", dump_expr(&a.value), dump_choices(&a.choices)))
                .collect();
            format!(
                "with {} select{} {}",
                dump_expr(selector),
                if *matching { "?" } else { "" },
                items.join(", ")
            )
        }
    }
}

fn dump_entity_class(c: EntityClass) -> &'static str {
    c.as_str()
}

fn dump_subprogram_head(s: &SubprogramSpec) -> String {
    let mut out = String::new();
    match s.pure {
        Some(true) => out.push_str("pure "),
        Some(false) => out.push_str("impure "),
        None => {}
    }
    out.push_str(match s.kind {
        SubprogramKind::Function => "function ",
        SubprogramKind::Procedure => "procedure ",
    });
    out.push_str(&dump_designator(&s.designator));
    if let Some(r) = &s.return_type {
        out.push_str(" return ");
        out.push_str(&dump_name(r));
    }
    out
}

fn dump_entity_aspect(a: &EntityAspect) -> String {
    match a {
        EntityAspect::Entity {
            name, architecture, ..
        } => match architecture {
            Some(arch) => format!("entity {}({})", dump_name(name), dump_ident(arch)),
            None => format!("entity {}", dump_name(name)),
        },
        EntityAspect::Configuration(n) => format!("configuration {}", dump_name(n)),
        EntityAspect::Open(_) => "open".to_owned(),
    }
}

fn dump_instantiation_list(l: &InstantiationList) -> String {
    match l {
        InstantiationList::Labels(ls) => {
            let items: Vec<String> = ls.iter().map(dump_ident).collect();
            items.join(", ")
        }
        InstantiationList::Others(_) => "others".to_owned(),
        InstantiationList::All(_) => "all".to_owned(),
    }
}

fn dump_binding(b: &BindingIndication) -> String {
    let mut parts = Vec::new();
    if let Some(a) = &b.entity_aspect {
        parts.push(format!("use {}", dump_entity_aspect(a)));
    }
    if let Some(g) = &b.generic_map {
        parts.push(format!("generic map ({})", dump_assocs(g)));
    }
    if let Some(p) = &b.port_map {
        parts.push(format!("port map ({})", dump_assocs(p)));
    }
    parts.join(" ")
}

struct Dumper<'a> {
    out: String,
    indent: usize,
    src: &'a SourceFile,
}

impl Dumper<'_> {
    fn line(&mut self, text: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self)) {
        self.indent += 1;
        f(self);
        self.indent -= 1;
    }

    fn loc(&self, span: crate::source::Span) -> String {
        format!("@{}", self.src.loc(span.start))
    }

    /// `Kind [label] @loc rest`
    fn node(&mut self, kind: &str, label: Option<&Ident>, span: crate::source::Span, rest: &str) {
        let mut s = kind.to_owned();
        if let Some(l) = label {
            s.push(' ');
            s.push_str(&dump_ident(l));
        }
        s.push(' ');
        s.push_str(&self.loc(span));
        if !rest.is_empty() {
            s.push(' ');
            s.push_str(rest);
        }
        self.line(&s);
    }

    // --- units --------------------------------------------------------------------

    fn design_unit(&mut self, u: &DesignUnit) {
        self.node("DesignUnit", None, u.span, "");
        self.nested(|d| {
            for c in &u.context {
                d.context_item(c);
            }
            d.library_unit(&u.unit);
        });
    }

    fn context_item(&mut self, c: &ContextItem) {
        match c {
            ContextItem::Library(l) => {
                let names: Vec<String> = l.names.iter().map(dump_ident).collect();
                self.node("Library", None, l.span, &names.join(", "));
            }
            ContextItem::Use(u) => self.use_clause(u),
            ContextItem::Context(r) => {
                let names: Vec<String> = r.names.iter().map(dump_name).collect();
                self.node("ContextRef", None, r.span, &names.join(", "));
            }
        }
    }

    fn use_clause(&mut self, u: &UseClause) {
        let names: Vec<String> = u.names.iter().map(dump_name).collect();
        self.node("Use", None, u.span, &names.join(", "));
    }

    fn library_unit(&mut self, u: &LibraryUnit) {
        match u {
            LibraryUnit::Entity(e) => {
                self.node("Entity", Some(&e.name), e.span, "");
                self.nested(|d| {
                    d.interfaces("Generic", &e.generics);
                    d.interfaces("Port", &e.ports);
                    d.decls(&e.decls);
                    if !e.statements.is_empty() {
                        d.line("Begin");
                        d.concurrent_statements(&e.statements);
                    }
                });
            }
            LibraryUnit::Architecture(a) => {
                let rest = format!("of {}", dump_name(&a.entity));
                self.node("Architecture", Some(&a.name), a.span, &rest);
                self.nested(|d| {
                    d.decls(&a.decls);
                    d.line("Begin");
                    d.concurrent_statements(&a.statements);
                });
            }
            LibraryUnit::Package(p) => self.package(p),
            LibraryUnit::PackageBody(b) => self.package_body(b),
            LibraryUnit::PackageInstantiation(p) => self.package_instantiation(p),
            LibraryUnit::Configuration(c) => {
                let rest = format!("of {}", dump_name(&c.entity));
                self.node("Configuration", Some(&c.name), c.span, &rest);
                self.nested(|d| {
                    d.decls(&c.decls);
                    d.block_configuration(&c.block);
                });
            }
            LibraryUnit::Context(c) => {
                self.node("Context", Some(&c.name), c.span, "");
                self.nested(|d| {
                    for i in &c.items {
                        d.context_item(i);
                    }
                });
            }
        }
    }

    fn package(&mut self, p: &PackageDecl) {
        self.node("Package", Some(&p.name), p.span, "");
        self.nested(|d| {
            d.interfaces("Generic", &p.generics);
            if let Some(m) = &p.generic_map {
                d.line(&format!("GenericMap ({})", dump_assocs(m)));
            }
            d.decls(&p.decls);
        });
    }

    fn package_body(&mut self, b: &PackageBody) {
        self.node("PackageBody", Some(&b.name), b.span, "");
        self.nested(|d| d.decls(&b.decls));
    }

    fn package_instantiation(&mut self, p: &PackageInstantiation) {
        let mut rest = format!("is new {}", dump_name(&p.uninstantiated));
        if !p.generic_map.is_empty() {
            let _ = write!(rest, " generic map ({})", dump_assocs(&p.generic_map));
        }
        self.node("PackageInstantiation", Some(&p.name), p.span, &rest);
    }

    fn block_configuration(&mut self, b: &BlockConfiguration) {
        let rest = format!("for {}", dump_name(&b.spec));
        self.node("BlockConfig", None, b.span, &rest);
        self.nested(|d| {
            for u in &b.uses {
                d.use_clause(u);
            }
            for i in &b.items {
                match i {
                    ConfigurationItem::Block(b) => d.block_configuration(b),
                    ConfigurationItem::Component(c) => {
                        let rest = format!(
                            "for {} : {}",
                            dump_instantiation_list(&c.spec.instances),
                            dump_name(&c.spec.component)
                        );
                        d.node("ComponentConfig", None, c.span, &rest);
                        d.nested(|d| {
                            if let Some(b) = &c.binding {
                                d.line(&format!("Binding {}", dump_binding(b)));
                            }
                            if let Some(b) = &c.block {
                                d.block_configuration(b);
                            }
                        });
                    }
                }
            }
        });
    }

    // --- interfaces and declarations -----------------------------------------------

    fn interfaces(&mut self, kind: &str, list: &[InterfaceDecl]) {
        for i in list {
            let rest = match i {
                InterfaceDecl::Object(o) => {
                    let mut s = String::new();
                    if let Some(c) = o.class {
                        s.push_str(match c {
                            ObjectClass::Constant => "constant ",
                            ObjectClass::Signal => "signal ",
                            ObjectClass::Variable => "variable ",
                            ObjectClass::File => "file ",
                        });
                    }
                    let names: Vec<String> = o.names.iter().map(dump_ident).collect();
                    s.push_str(&names.join(", "));
                    s.push_str(" : ");
                    if let Some(m) = o.mode {
                        s.push_str(dump_mode(m));
                        s.push(' ');
                    }
                    s.push_str(&dump_subtype(&o.subtype));
                    if o.bus {
                        s.push_str(" bus");
                    }
                    if let Some(e) = &o.default {
                        let _ = write!(s, " := {}", dump_expr(e));
                    }
                    s
                }
                InterfaceDecl::Type(t) => format!("type {}", dump_ident(&t.name)),
                InterfaceDecl::Subprogram(s) => {
                    let mut out = dump_subprogram_head(&s.spec);
                    if let Some(def) = &s.default {
                        out.push_str(" is ");
                        out.push_str(&match def {
                            SubprogramDefault::Box(_) => "<>".to_owned(),
                            SubprogramDefault::Name(n) => dump_name(n),
                        });
                    }
                    out
                }
                InterfaceDecl::Package(p) => {
                    let map = match &p.generic_map {
                        InterfacePackageMap::Box(_) => "<>".to_owned(),
                        InterfacePackageMap::Default(_) => "default".to_owned(),
                        InterfacePackageMap::Map(m) => dump_assocs(m),
                    };
                    format!(
                        "package {} is new {} generic map ({map})",
                        dump_ident(&p.name),
                        dump_name(&p.uninstantiated)
                    )
                }
            };
            self.node(kind, None, i.span(), &rest);
            if let InterfaceDecl::Subprogram(s) = i {
                self.nested(|d| d.subprogram_lists(&s.spec));
            }
        }
    }

    fn subprogram_lists(&mut self, s: &SubprogramSpec) {
        self.interfaces("Generic", &s.generics);
        if let Some(m) = &s.generic_map {
            self.line(&format!("GenericMap ({})", dump_assocs(m)));
        }
        self.interfaces("Param", &s.params);
    }

    fn decls(&mut self, decls: &[Declaration]) {
        for d in decls {
            self.decl(d);
        }
    }

    fn decl(&mut self, decl: &Declaration) {
        match decl {
            Declaration::Object(o) => {
                let kind = match o.kind {
                    ObjectKind::Constant => "Constant",
                    ObjectKind::Signal => "Signal",
                    ObjectKind::Variable => "Variable",
                    ObjectKind::SharedVariable => "SharedVariable",
                };
                let names: Vec<String> = o.names.iter().map(dump_ident).collect();
                let mut rest = format!("{} : {}", names.join(", "), dump_subtype(&o.subtype));
                match o.signal_kind {
                    Some(SignalKind::Register) => rest.push_str(" register"),
                    Some(SignalKind::Bus) => rest.push_str(" bus"),
                    None => {}
                }
                if let Some(e) = &o.init {
                    let _ = write!(rest, " := {}", dump_expr(e));
                }
                self.node(kind, None, o.span, &rest);
            }
            Declaration::File(f) => {
                let names: Vec<String> = f.names.iter().map(dump_ident).collect();
                let mut rest = format!("{} : {}", names.join(", "), dump_subtype(&f.subtype));
                if let Some(k) = &f.open_kind {
                    let _ = write!(rest, " open {}", dump_expr(k));
                }
                if let Some(n) = &f.logical_name {
                    rest.push_str(" is ");
                    if let Some(m) = f.mode87 {
                        rest.push_str(dump_mode(m));
                        rest.push(' ');
                    }
                    rest.push_str(&dump_expr(n));
                }
                self.node("File", None, f.span, &rest);
            }
            Declaration::Type(t) => self.type_decl(t),
            Declaration::Subtype(s) => {
                let rest = format!("is {}", dump_subtype(&s.subtype));
                self.node("Subtype", Some(&s.name), s.span, &rest);
            }
            Declaration::Alias(a) => {
                let mut rest = dump_designator(&a.designator);
                if let Some(s) = &a.subtype {
                    let _ = write!(rest, " : {}", dump_subtype(s));
                }
                let _ = write!(rest, " is {}", dump_name(&a.target));
                if let Some(s) = &a.signature {
                    rest.push_str(&dump_signature(s));
                }
                self.node("Alias", None, a.span, &rest);
            }
            Declaration::Attribute(a) => {
                let rest = format!(": {}", dump_name(&a.type_mark));
                self.node("Attribute", Some(&a.name), a.span, &rest);
            }
            Declaration::AttributeSpec(a) => {
                let entities = match &a.entities {
                    EntityNameList::Names(ns) => {
                        let items: Vec<String> = ns
                            .iter()
                            .map(|n| {
                                let mut s = dump_designator(&n.designator);
                                if let Some(sig) = &n.signature {
                                    s.push_str(&dump_signature(sig));
                                }
                                s
                            })
                            .collect();
                        items.join(", ")
                    }
                    EntityNameList::Others(_) => "others".to_owned(),
                    EntityNameList::All(_) => "all".to_owned(),
                };
                let rest = format!(
                    "of {entities} : {} is {}",
                    dump_entity_class(a.class),
                    dump_expr(&a.value)
                );
                self.node("AttributeSpec", Some(&a.attribute), a.span, &rest);
            }
            Declaration::Component(c) => {
                self.node("Component", Some(&c.name), c.span, "");
                self.nested(|d| {
                    d.interfaces("Generic", &c.generics);
                    d.interfaces("Port", &c.ports);
                });
            }
            Declaration::Subprogram(s) => {
                let rest = dump_subprogram_head(&s.spec);
                self.node("SubprogramDecl", None, s.span, &rest);
                self.nested(|d| d.subprogram_lists(&s.spec));
            }
            Declaration::SubprogramBody(b) => {
                let rest = dump_subprogram_head(&b.spec);
                self.node("SubprogramBody", None, b.span, &rest);
                self.nested(|d| {
                    d.subprogram_lists(&b.spec);
                    d.decls(&b.decls);
                    d.line("Begin");
                    d.sequential_statements(&b.statements);
                });
            }
            Declaration::SubprogramInstantiation(s) => {
                let mut rest = format!(
                    "{} {} is new {}",
                    match s.kind {
                        SubprogramKind::Function => "function",
                        SubprogramKind::Procedure => "procedure",
                    },
                    dump_designator(&s.designator),
                    dump_name(&s.uninstantiated)
                );
                if let Some(sig) = &s.signature {
                    rest.push_str(&dump_signature(sig));
                }
                if !s.generic_map.is_empty() {
                    let _ = write!(rest, " generic map ({})", dump_assocs(&s.generic_map));
                }
                self.node("SubprogramInstantiation", None, s.span, &rest);
            }
            Declaration::Package(p) => self.package(p),
            Declaration::PackageBody(b) => self.package_body(b),
            Declaration::PackageInstantiation(p) => self.package_instantiation(p),
            Declaration::Use(u) => self.use_clause(u),
            Declaration::GroupTemplate(g) => {
                let items: Vec<String> = g
                    .entries
                    .iter()
                    .map(|e| {
                        let mut s = dump_entity_class(e.class).to_owned();
                        if e.unbounded {
                            s.push_str(" <>");
                        }
                        s
                    })
                    .collect();
                let rest = format!("is ({})", items.join(", "));
                self.node("GroupTemplate", Some(&g.name), g.span, &rest);
            }
            Declaration::Group(g) => {
                let items: Vec<String> = g.constituents.iter().map(dump_expr).collect();
                let rest = format!(": {} ({})", dump_name(&g.template), items.join(", "));
                self.node("Group", Some(&g.name), g.span, &rest);
            }
            Declaration::Disconnection(s) => {
                let signals = match &s.signals {
                    SignalList::Names(ns) => {
                        let items: Vec<String> = ns.iter().map(dump_name).collect();
                        items.join(", ")
                    }
                    SignalList::Others(_) => "others".to_owned(),
                    SignalList::All(_) => "all".to_owned(),
                };
                let rest = format!(
                    "{signals} : {} after {}",
                    dump_name(&s.type_mark),
                    dump_expr(&s.time)
                );
                self.node("Disconnect", None, s.span, &rest);
            }
            Declaration::ConfigurationSpec(c) => {
                let rest = format!(
                    "for {} : {} {}",
                    dump_instantiation_list(&c.spec.instances),
                    dump_name(&c.spec.component),
                    dump_binding(&c.binding)
                );
                self.node("ConfigurationSpec", None, c.span, &rest);
            }
        }
    }

    fn type_decl(&mut self, t: &TypeDecl) {
        let Some(def) = &t.def else {
            self.node("Type", Some(&t.name), t.span, "");
            return;
        };
        let rest = match def {
            TypeDef::Enumeration(lits) => {
                let items: Vec<String> = lits.iter().map(dump_designator).collect();
                format!("is ({})", items.join(", "))
            }
            TypeDef::Range(r) => format!("is range {}", dump_range(r)),
            TypeDef::Physical(p) => format!(
                "is range {} units {}",
                dump_range(&p.range),
                dump_ident(&p.primary_unit)
            ),
            TypeDef::Array(a) => {
                let items: Vec<String> = a
                    .indices
                    .iter()
                    .map(|i| match i {
                        ArrayIndex::Unbounded(n) => format!("{} range <>", dump_name(n)),
                        ArrayIndex::Constrained(r) => dump_discrete_range(r),
                    })
                    .collect();
                format!(
                    "is array ({}) of {}",
                    items.join(", "),
                    dump_subtype(&a.element)
                )
            }
            TypeDef::Record(_) => "is record".to_owned(),
            TypeDef::Access(s) => format!("is access {}", dump_subtype(s)),
            TypeDef::File(n) => format!("is file of {}", dump_name(n)),
            TypeDef::Protected(_) => "is protected".to_owned(),
            TypeDef::ProtectedBody(_) => "is protected body".to_owned(),
        };
        self.node("Type", Some(&t.name), t.span, &rest);
        self.nested(|d| match def {
            TypeDef::Physical(p) => {
                for u in &p.secondary_units {
                    let rest = format!("= {}", dump_expr(&u.value));
                    d.node("Unit", Some(&u.name), u.span, &rest);
                }
            }
            TypeDef::Record(r) => {
                for e in &r.elements {
                    let names: Vec<String> = e.names.iter().map(dump_ident).collect();
                    let rest = format!("{} : {}", names.join(", "), dump_subtype(&e.subtype));
                    d.node("Element", None, e.span, &rest);
                }
            }
            TypeDef::Protected(p) => d.decls(&p.decls),
            TypeDef::ProtectedBody(p) => d.decls(&p.decls),
            _ => {}
        });
    }

    // --- concurrent statements ---------------------------------------------------

    fn concurrent_statements(&mut self, stmts: &[ConcurrentStatement]) {
        for s in stmts {
            self.concurrent_statement(s);
        }
    }

    fn concurrent_statement(&mut self, s: &ConcurrentStatement) {
        let label = s.label.as_ref();
        match &s.kind {
            ConcurrentKind::Process(p) => {
                let mut rest = String::new();
                if p.postponed {
                    rest.push_str("postponed ");
                }
                match &p.sensitivity {
                    Some(Sensitivity::All(_)) => rest.push_str("(all)"),
                    Some(Sensitivity::Names(ns)) => {
                        let items: Vec<String> = ns.iter().map(dump_name).collect();
                        let _ = write!(rest, "({})", items.join(", "));
                    }
                    None => {}
                }
                self.node("Process", label, s.span, rest.trim_end());
                self.nested(|d| {
                    d.decls(&p.decls);
                    d.line("Begin");
                    d.sequential_statements(&p.statements);
                });
            }
            ConcurrentKind::Block(b) => {
                let rest = b
                    .guard
                    .as_ref()
                    .map(|g| format!("guard ({})", dump_expr(g)))
                    .unwrap_or_default();
                self.node("Block", label, s.span, &rest);
                self.nested(|d| {
                    d.interfaces("Generic", &b.generics);
                    if let Some(m) = &b.generic_map {
                        d.line(&format!("GenericMap ({})", dump_assocs(m)));
                    }
                    d.interfaces("Port", &b.ports);
                    if let Some(m) = &b.port_map {
                        d.line(&format!("PortMap ({})", dump_assocs(m)));
                    }
                    d.decls(&b.decls);
                    d.line("Begin");
                    d.concurrent_statements(&b.statements);
                });
            }
            ConcurrentKind::SignalAssignment(a) => {
                let mut rest = String::new();
                if a.postponed {
                    rest.push_str("postponed ");
                }
                rest.push_str(&dump_target(&a.assignment.target));
                rest.push_str(" <= ");
                if a.guarded {
                    rest.push_str("guarded ");
                }
                if let Some(d) = &a.assignment.delay {
                    rest.push_str(&dump_delay(d));
                }
                rest.push_str(&dump_signal_rhs(&a.assignment.rhs));
                self.node("SignalAssign", label, s.span, &rest);
            }
            ConcurrentKind::ProcedureCall { postponed, call } => {
                let rest = format!(
                    "{}{}",
                    if *postponed { "postponed " } else { "" },
                    dump_name(call)
                );
                self.node("Call", label, s.span, &rest);
            }
            ConcurrentKind::Assertion {
                postponed,
                assertion,
            } => {
                let rest = format!(
                    "{}{}",
                    if *postponed { "postponed " } else { "" },
                    dump_assertion(assertion)
                );
                self.node("Assert", label, s.span, &rest);
            }
            ConcurrentKind::Instantiation(i) => {
                let rest = match &i.unit {
                    InstantiatedUnit::Component(n) => format!("component {}", dump_name(n)),
                    InstantiatedUnit::Entity { name, architecture } => match architecture {
                        Some(a) => format!("entity {}({})", dump_name(name), dump_ident(a)),
                        None => format!("entity {}", dump_name(name)),
                    },
                    InstantiatedUnit::Configuration(n) => {
                        format!("configuration {}", dump_name(n))
                    }
                };
                self.node("Instance", label, s.span, &rest);
                self.nested(|d| {
                    if let Some(m) = &i.generic_map {
                        d.line(&format!("GenericMap ({})", dump_assocs(m)));
                    }
                    if let Some(m) = &i.port_map {
                        d.line(&format!("PortMap ({})", dump_assocs(m)));
                    }
                });
            }
            ConcurrentKind::ForGenerate(g) => {
                let rest = format!(
                    "{} in {}",
                    dump_ident(&g.param),
                    dump_discrete_range(&g.range)
                );
                self.node("ForGenerate", label, s.span, &rest);
                self.nested(|d| d.generate_body(&g.body));
            }
            ConcurrentKind::IfGenerate(g) => {
                self.node("IfGenerate", label, s.span, "");
                self.nested(|d| {
                    for arm in &g.arms {
                        let rest = format!("{}{}", alt_label(&arm.body), dump_expr(&arm.condition));
                        d.node("Arm", None, arm.span, &rest);
                        d.nested(|d| d.generate_body(&arm.body));
                    }
                    if let Some(e) = &g.else_arm {
                        d.node("Else", None, e.span, alt_label(e).trim_end());
                        d.nested(|d| d.generate_body(e));
                    }
                });
            }
            ConcurrentKind::CaseGenerate(g) => {
                self.node("CaseGenerate", label, s.span, &dump_expr(&g.expr));
                self.nested(|d| {
                    for arm in &g.arms {
                        let rest =
                            format!("{}{}", alt_label(&arm.body), dump_choices(&arm.choices));
                        d.node("When", None, arm.span, &rest);
                        d.nested(|d| d.generate_body(&arm.body));
                    }
                });
            }
        }
    }

    fn generate_body(&mut self, b: &GenerateBody) {
        if !b.decls.is_empty() {
            self.decls(&b.decls);
            self.line("Begin");
        }
        self.concurrent_statements(&b.statements);
    }

    // --- sequential statements ------------------------------------------------------

    fn sequential_statements(&mut self, stmts: &[SequentialStatement]) {
        for s in stmts {
            self.sequential_statement(s);
        }
    }

    fn sequential_statement(&mut self, s: &SequentialStatement) {
        let label = s.label.as_ref();
        match &s.kind {
            SequentialKind::Wait {
                sensitivity,
                condition,
                timeout,
            } => {
                let mut parts = Vec::new();
                match sensitivity {
                    Some(Sensitivity::All(_)) => parts.push("on all".to_owned()),
                    Some(Sensitivity::Names(ns)) => {
                        let items: Vec<String> = ns.iter().map(dump_name).collect();
                        parts.push(format!("on {}", items.join(", ")));
                    }
                    None => {}
                }
                if let Some(c) = condition {
                    parts.push(format!("until {}", dump_expr(c)));
                }
                if let Some(t) = timeout {
                    parts.push(format!("for {}", dump_expr(t)));
                }
                self.node("Wait", label, s.span, &parts.join(" "));
            }
            SequentialKind::Assertion(a) => {
                self.node("Assert", label, s.span, &dump_assertion(a));
            }
            SequentialKind::Report { message, severity } => {
                let mut rest = dump_expr(message);
                if let Some(sev) = severity {
                    let _ = write!(rest, " severity {}", dump_expr(sev));
                }
                self.node("Report", label, s.span, &rest);
            }
            SequentialKind::SignalAssignment(a) => {
                let mut rest = format!("{} <= ", dump_target(&a.target));
                if let Some(d) = &a.delay {
                    rest.push_str(&dump_delay(d));
                }
                rest.push_str(&dump_signal_rhs(&a.rhs));
                self.node("SignalAssign", label, s.span, &rest);
            }
            SequentialKind::VariableAssignment(a) => {
                let rest = format!(
                    "{} := {}",
                    dump_target(&a.target),
                    dump_variable_rhs(&a.rhs)
                );
                self.node("VarAssign", label, s.span, &rest);
            }
            SequentialKind::ProcedureCall(n) => self.node("Call", label, s.span, &dump_name(n)),
            SequentialKind::If(i) => {
                self.node("If", label, s.span, "");
                self.nested(|d| {
                    for arm in &i.arms {
                        d.node("Arm", None, arm.span, &dump_expr(&arm.condition));
                        d.nested(|d| d.sequential_statements(&arm.statements));
                    }
                    if let Some(e) = &i.else_statements {
                        d.line("Else");
                        d.nested(|d| d.sequential_statements(e));
                    }
                });
            }
            SequentialKind::Case(c) => {
                let kind = if c.matching { "Case?" } else { "Case" };
                self.node(kind, label, s.span, &dump_expr(&c.expr));
                self.nested(|d| {
                    for arm in &c.arms {
                        d.node("When", None, arm.span, &dump_choices(&arm.choices));
                        d.nested(|d| d.sequential_statements(&arm.statements));
                    }
                });
            }
            SequentialKind::Loop(l) => {
                let (kind, rest) = match &l.scheme {
                    None => ("Loop", String::new()),
                    Some(IterationScheme::While(c)) => ("While", dump_expr(c)),
                    Some(IterationScheme::For { param, range }) => (
                        "For",
                        format!("{} in {}", dump_ident(param), dump_discrete_range(range)),
                    ),
                };
                self.node(kind, label, s.span, &rest);
                self.nested(|d| d.sequential_statements(&l.statements));
            }
            SequentialKind::Next {
                label: target,
                condition,
            } => {
                let rest = next_exit_rest(target, condition);
                self.node("Next", label, s.span, &rest);
            }
            SequentialKind::Exit {
                label: target,
                condition,
            } => {
                let rest = next_exit_rest(target, condition);
                self.node("Exit", label, s.span, &rest);
            }
            SequentialKind::Return(e) => {
                let rest = e.as_ref().map(dump_expr).unwrap_or_default();
                self.node("Return", label, s.span, &rest);
            }
            SequentialKind::Null => self.node("Null", label, s.span, ""),
        }
    }
}

fn alt_label(b: &GenerateBody) -> String {
    b.label
        .as_ref()
        .map(|l| format!("{}: ", dump_ident(l)))
        .unwrap_or_default()
}

fn next_exit_rest(target: &Option<Ident>, condition: &Option<Expr>) -> String {
    let mut parts = Vec::new();
    if let Some(t) = target {
        parts.push(dump_ident(t));
    }
    if let Some(c) = condition {
        parts.push(format!("when {}", dump_expr(c)));
    }
    parts.join(" ")
}

fn dump_assertion(a: &Assertion) -> String {
    let mut rest = dump_expr(&a.condition);
    if let Some(r) = &a.report {
        let _ = write!(rest, " report {}", dump_expr(r));
    }
    if let Some(s) = &a.severity {
        let _ = write!(rest, " severity {}", dump_expr(s));
    }
    rest
}
