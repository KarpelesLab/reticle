//! Building a document [`Index`] from VHDL source and its analysis.
//!
//! VHDL needs no name resolution here: [`crate::vhdl::sema`] has already
//! done it, and its [`Analysis`] carries span-keyed side tables giving
//! every name its declaration and every expression its type. So this
//! module does two smaller things and joins them.
//!
//! - It walks the syntax tree for *structure*: which declaration encloses
//!   which, how far each one extends, what belongs in the outline, and
//!   where an instantiation's port map is. The tree is the only place that
//!   knows those, since the analysis records positions but not extents.
//! - It scans the identifier tokens for *uses*: every identifier whose
//!   span [`Analysis::decl_of`] maps to a declaration in this document
//!   becomes a reference to it. One pass over the token stream catches
//!   simple names, selected names, port-map formals and sensitivity lists
//!   alike, with no walk of the expression grammar at all, and it is
//!   exactly as precise as the analysis is.
//!
//! Types and widths come from the analysis too
//! ([`Analysis::describe_type`]), which is why a VHDL hover can say
//! `std_logic_vector(7 downto 0)` and `8 bits` where the Verilog side can
//! only repeat what was written: the VHDL front end resolved the subtype
//! without needing a top-level design.

use std::collections::HashMap;

use crate::source::{SourceId, SourceMap, Span};
use crate::vhdl::TokenKind;
use crate::vhdl::ast::{self, Declaration, InterfaceDecl, LibraryUnit};
use crate::vhdl::sema::{Analysis, CallTarget, DeclId};

use super::index::{Builder, DeclClass, DeclInfo, Index, InstSite, PortInfo};
use super::text::{collapse, first_line, slice};

/// Walks `file` and returns its index.
///
/// `tokens` is the document's token stream as `(kind, span)` pairs, which
/// is what the use scan runs over: the identifiers in it are the names,
/// and the parentheses around them are what tells a call from a name.
pub fn index(
    map: &SourceMap,
    source: SourceId,
    analysis: &Analysis,
    file: &ast::DesignFile,
    tokens: &[(TokenKind, Span)],
) -> Index {
    let text = map.file(source).text();
    let mut walk = Walk {
        text,
        map,
        source,
        analysis,
        b: Builder::new(true),
        parents: Vec::new(),
        local: HashMap::new(),
    };
    for unit in &file.units {
        walk.unit(unit);
    }
    walk.add_uses(tokens);
    walk.add_calls(tokens);
    walk.b.finish()
}

/// The walk state.
struct Walk<'a> {
    text: &'a str,
    map: &'a SourceMap,
    source: SourceId,
    analysis: &'a Analysis,
    b: Builder,
    parents: Vec<usize>,
    /// Which index each analysed declaration of this document became.
    local: HashMap<DeclId, usize>,
}

impl Walk<'_> {
    fn parent(&self) -> Option<usize> {
        self.parents.last().copied()
    }

    /// The type the analysis gave a declared name, and its width when the
    /// type fixes one.
    ///
    /// The analysis keys its expression types by span, and a declared name
    /// is one of the spans it records, so this works on the declaration
    /// itself and not only on uses.
    fn typed(&self, name_span: Span) -> (String, Option<u32>) {
        let Some(ty) = self.analysis.type_of(name_span) else {
            return (String::new(), None);
        };
        if self.analysis.is_error(ty) {
            return (String::new(), None);
        }
        (
            self.analysis.describe_type(ty, Some(self.map)),
            self.width_of(ty),
        )
    }

    /// The type of a declared name with its port mode in front.
    fn typed_with_mode(&self, name_span: Span, mode: Option<ast::Mode>) -> (String, Option<u32>) {
        let (described, width) = self.typed(name_span);
        match mode {
            Some(mode) if !described.is_empty() => {
                (format!("{} {described}", mode_str(mode)), width)
            }
            _ => (described, width),
        }
    }

    /// The width of a type, when it has one that does not depend on an
    /// unelaborated generic.
    fn width_of(&self, ty: crate::vhdl::sema::TypeId) -> Option<u32> {
        if let Some(length) = self.analysis.array_length(ty) {
            return u32::try_from(length).ok();
        }
        // A scalar bit type is one bit; every other scalar (an integer, an
        // enumeration) has no width a hardware reader would recognise.
        if self.analysis.is_std_ulogic(ty)
            || self.analysis.same_base(ty, self.analysis.builtins.bit)
        {
            return Some(1);
        }
        None
    }

    /// Records a declaration and remembers which analysed declaration it
    /// came from, so the use scan can find it again.
    fn declare(&mut self, name: &ast::Ident, decl: DeclInfo) -> usize {
        let id = self.b.declare(decl);
        if let Some(analysed) = self.analysis.decl_of(name.span) {
            self.local.entry(analysed).or_insert(id);
        }
        id
    }

    /// A declaration with the common fields filled in.
    fn info(
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

    /// Turns every identifier token the analysis resolved into a reference.
    fn add_uses(&mut self, tokens: &[(TokenKind, Span)]) {
        for &(kind, span) in tokens {
            if kind != TokenKind::Ident {
                continue;
            }
            let Some(analysed) = self.analysis.decl_of(span) else {
                continue;
            };
            let Some(&local) = self.local.get(&analysed) else {
                // A name from another file: `std_logic` and friends. It is
                // resolved, but not to anything this document declares, so
                // there is nothing here to point at.
                continue;
            };
            if self.b.decls()[local].name_span != span {
                self.b.add_ref(span, local);
            }
        }
    }

    /// Turns a call of a subprogram this document declares into a
    /// reference to it.
    ///
    /// A call is not a name as far as [`Analysis::decl_of`] is concerned:
    /// the analysis records what `f(x)` resolved to under the span of the
    /// whole call, in [`Analysis::call_of`]. That span cannot be looked up
    /// backwards, but it can be reconstructed: an identifier followed by a
    /// parenthesis, up to the matching one. A reconstruction that is not
    /// actually a call simply misses in the table, so nothing here is a
    /// guess — only what the analysis confirms becomes a reference.
    fn add_calls(&mut self, tokens: &[(TokenKind, Span)]) {
        for (i, &(kind, span)) in tokens.iter().enumerate() {
            if kind != TokenKind::Ident
                || tokens.get(i + 1).map(|(k, _)| *k) != Some(TokenKind::LParen)
            {
                continue;
            }
            let mut depth = 0usize;
            let Some(close) = tokens[i + 1..].iter().find_map(|(kind, span)| match kind {
                TokenKind::LParen => {
                    depth += 1;
                    None
                }
                TokenKind::RParen => {
                    // The first token looked at is the opening bracket, so
                    // the depth cannot reach zero early; saturate anyway
                    // rather than risk an underflow in a server.
                    depth = depth.saturating_sub(1);
                    (depth == 0).then_some(*span)
                }
                _ => None,
            }) else {
                continue;
            };
            let call = Span::new(self.source, span.start, close.end);
            let Some(CallTarget::Subprogram(analysed)) = self.analysis.call_of(call) else {
                continue;
            };
            let Some(&local) = self.local.get(analysed) else {
                continue;
            };
            if self.b.decls()[local].name_span != span {
                self.b.add_ref(span, local);
            }
        }
    }

    // --- design units ----------------------------------------------------

    fn unit(&mut self, unit: &ast::DesignUnit) {
        match &unit.unit {
            LibraryUnit::Entity(entity) => self.entity(entity),
            LibraryUnit::Architecture(arch) => self.architecture(arch),
            LibraryUnit::Package(package) => self.package(package, package.span),
            LibraryUnit::PackageBody(body) => {
                let mut info = self.info(
                    &body.name,
                    DeclClass::Package,
                    "package body".into(),
                    None,
                    body.span,
                );
                info.text = first_line(slice(self.text, body.span));
                let id = self.declare(&body.name, info);
                self.parents.push(id);
                self.declarations(&body.decls);
                self.parents.pop();
            }
            LibraryUnit::Configuration(config) => {
                let info = self.info(
                    &config.name,
                    DeclClass::Package,
                    "configuration".into(),
                    None,
                    config.span,
                );
                self.declare(&config.name, info);
            }
            LibraryUnit::Context(context) => {
                let info = self.info(
                    &context.name,
                    DeclClass::Package,
                    "context".into(),
                    None,
                    context.span,
                );
                self.declare(&context.name, info);
            }
            LibraryUnit::PackageInstantiation(inst) => {
                let info = self.info(
                    &inst.name,
                    DeclClass::Package,
                    "package instance".into(),
                    None,
                    inst.span,
                );
                self.declare(&inst.name, info);
            }
        }
    }

    fn entity(&mut self, entity: &ast::EntityDecl) {
        let mut info = self.info(
            &entity.name,
            DeclClass::Entity,
            "entity".into(),
            None,
            entity.span,
        );
        info.text = first_line(slice(self.text, entity.span));
        let id = self.declare(&entity.name, info);
        self.parents.push(id);
        self.interfaces(&entity.generics, DeclClass::Parameter);
        let ports = self.interfaces(&entity.ports, DeclClass::Port);
        self.declarations(&entity.decls);
        self.statements(&entity.statements);
        self.parents.pop();
        self.b.decl_mut(id).ports = ports;
    }

    fn architecture(&mut self, arch: &ast::ArchitectureBody) {
        let entity = simple_name(&arch.entity);
        let detail = entity.map_or("architecture".to_string(), |e| format!("of {e}"));
        let mut info = self.info(&arch.name, DeclClass::Architecture, detail, None, arch.span);
        info.text = first_line(slice(self.text, arch.span));
        let id = self.declare(&arch.name, info);
        self.parents.push(id);
        self.declarations(&arch.decls);
        self.statements(&arch.statements);
        self.parents.pop();
    }

    fn package(&mut self, package: &ast::PackageDecl, span: Span) {
        let mut info = self.info(
            &package.name,
            DeclClass::Package,
            "package".into(),
            None,
            span,
        );
        info.text = first_line(slice(self.text, span));
        let id = self.declare(&package.name, info);
        self.parents.push(id);
        self.interfaces(&package.generics, DeclClass::Parameter);
        self.declarations(&package.decls);
        self.parents.pop();
    }

    /// Declares a generic, port or parameter list and returns its ports.
    fn interfaces(&mut self, decls: &[InterfaceDecl], class: DeclClass) -> Vec<PortInfo> {
        let mut ports = Vec::new();
        for decl in decls {
            match decl {
                InterfaceDecl::Object(object) => {
                    for name in &object.names {
                        let (detail, width) = self.typed_with_mode(name.span, object.mode);
                        let info = self.info(name, class, detail.clone(), width, object.span);
                        let detail = if detail.is_empty() {
                            collapse(slice(self.text, object.subtype.span))
                        } else {
                            detail
                        };
                        self.declare(name, info);
                        ports.push(PortInfo {
                            name: name.name.clone(),
                            detail,
                        });
                    }
                }
                InterfaceDecl::Type(ty) => {
                    let info = self.info(&ty.name, DeclClass::Type, "type".into(), None, ty.span);
                    self.declare(&ty.name, info);
                }
                InterfaceDecl::Package(package) => {
                    let info = self.info(
                        &package.name,
                        DeclClass::Package,
                        "package".into(),
                        None,
                        package.span,
                    );
                    self.declare(&package.name, info);
                }
                InterfaceDecl::Subprogram(sub) => {
                    if let ast::Designator::Ident(name) = &sub.spec.designator {
                        let class = subprogram_class(sub.spec.kind);
                        let info = self.info(name, class, String::new(), None, sub.span);
                        self.declare(name, info);
                    }
                }
            }
        }
        ports
    }

    fn declarations(&mut self, decls: &[Declaration]) {
        for decl in decls {
            self.declaration(decl);
        }
    }

    fn declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::Object(object) => {
                let class = match object.kind {
                    ast::ObjectKind::Constant => DeclClass::Constant,
                    ast::ObjectKind::Signal => DeclClass::Signal,
                    ast::ObjectKind::Variable | ast::ObjectKind::SharedVariable => {
                        DeclClass::Constant
                    }
                };
                for name in &object.names {
                    let (detail, width) = self.typed(name.span);
                    let detail = if detail.is_empty() {
                        collapse(slice(self.text, object.subtype.span))
                    } else {
                        detail
                    };
                    let info = self.info(name, class, detail, width, object.span);
                    self.declare(name, info);
                }
            }
            Declaration::File(file) => {
                for name in &file.names {
                    let (detail, width) = self.typed(name.span);
                    let info = self.info(name, DeclClass::Constant, detail, width, file.span);
                    self.declare(name, info);
                }
            }
            Declaration::Type(ty) => {
                let info = self.info(&ty.name, DeclClass::Type, "type".into(), None, ty.span);
                let id = self.declare(&ty.name, info);
                if let Some(ast::TypeDef::Enumeration(literals)) = &ty.def {
                    self.parents.push(id);
                    for literal in literals {
                        if let ast::Designator::Ident(name) = literal {
                            let info = self.info(
                                name,
                                DeclClass::EnumLiteral,
                                ty.name.name.clone(),
                                None,
                                name.span,
                            );
                            self.declare(name, info);
                        }
                    }
                    self.parents.pop();
                }
            }
            Declaration::Subtype(subtype) => {
                let detail = collapse(slice(self.text, subtype.subtype.span));
                let info = self.info(&subtype.name, DeclClass::Type, detail, None, subtype.span);
                self.declare(&subtype.name, info);
            }
            Declaration::Component(component) => {
                let info = self.info(
                    &component.name,
                    DeclClass::Component,
                    "component".into(),
                    None,
                    component.span,
                );
                let id = self.declare(&component.name, info);
                self.parents.push(id);
                self.interfaces(&component.generics, DeclClass::Parameter);
                let ports = self.interfaces(&component.ports, DeclClass::Port);
                self.parents.pop();
                self.b.decl_mut(id).ports = ports;
            }
            Declaration::Subprogram(sub) => {
                self.subprogram(&sub.spec, sub.span, &[], &[]);
            }
            Declaration::SubprogramBody(body) => {
                self.subprogram(&body.spec, body.span, &body.decls, &body.statements);
            }
            Declaration::Package(package) => self.package(package, package.span),
            Declaration::PackageBody(body) => self.declarations(&body.decls),
            Declaration::Alias(alias) => {
                if let ast::Designator::Ident(name) = &alias.designator {
                    let (detail, width) = self.typed(name.span);
                    let info = self.info(name, DeclClass::Constant, detail, width, alias.span);
                    self.declare(name, info);
                }
            }
            Declaration::Attribute(attr) => {
                let info = self.info(
                    &attr.name,
                    DeclClass::Type,
                    "attribute".into(),
                    None,
                    attr.span,
                );
                self.declare(&attr.name, info);
            }
            Declaration::SubprogramInstantiation(_)
            | Declaration::PackageInstantiation(_)
            | Declaration::AttributeSpec(_)
            | Declaration::Use(_)
            | Declaration::GroupTemplate(_)
            | Declaration::Group(_)
            | Declaration::Disconnection(_)
            | Declaration::ConfigurationSpec(_) => {}
        }
    }

    fn subprogram(
        &mut self,
        spec: &ast::SubprogramSpec,
        span: Span,
        decls: &[Declaration],
        statements: &[ast::SequentialStatement],
    ) {
        let ast::Designator::Ident(name) = &spec.designator else {
            // An operator symbol (`function "+"`) has no identifier to
            // point at, so it stays out of the index.
            return;
        };
        let detail = spec.return_type.as_ref().and_then(simple_name).map_or_else(
            || subprogram_class(spec.kind).describe().to_string(),
            |t| format!("return {t}"),
        );
        let mut info = self.info(name, subprogram_class(spec.kind), detail, None, span);
        info.text = first_line(slice(self.text, span));
        let id = self.declare(name, info);
        self.parents.push(id);
        let ports = self.interfaces(&spec.params, DeclClass::Port);
        self.declarations(decls);
        self.sequential(statements);
        self.parents.pop();
        self.b.decl_mut(id).ports = ports;
    }

    // --- statements ------------------------------------------------------

    fn statements(&mut self, statements: &[ast::ConcurrentStatement]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &ast::ConcurrentStatement) {
        let span = statement.span;
        match &statement.kind {
            ast::ConcurrentKind::Process(process) => {
                let detail = process
                    .sensitivity
                    .as_ref()
                    .map_or(String::new(), |s| match s {
                        ast::Sensitivity::Names(names) => format!(
                            "({})",
                            names
                                .iter()
                                .filter_map(simple_name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        ast::Sensitivity::All(_) => "(all)".to_string(),
                    });
                let id = self.labelled(statement, DeclClass::Process, "process", detail);
                self.parents.push(id);
                self.declarations(&process.decls);
                self.sequential(&process.statements);
                self.parents.pop();
            }
            ast::ConcurrentKind::Block(block) => {
                let id = self.labelled(statement, DeclClass::Block, "block", String::new());
                self.parents.push(id);
                self.interfaces(&block.generics, DeclClass::Parameter);
                self.interfaces(&block.ports, DeclClass::Port);
                self.declarations(&block.decls);
                self.statements(&block.statements);
                self.parents.pop();
            }
            ast::ConcurrentKind::Instantiation(inst) => {
                let target = instantiated_name(&inst.unit).unwrap_or_default();
                self.labelled(statement, DeclClass::Instance, "instance", target.clone());
                if let Some(port_map) = &inst.port_map {
                    let connected = port_map
                        .iter()
                        .filter_map(|assoc| {
                            let formal = assoc.formal.as_ref()?;
                            Some((formal_name(formal)?, formal_span(formal)))
                        })
                        .collect();
                    if let Some(span) = self.association_span(port_map) {
                        self.b.add_instance(InstSite {
                            span,
                            target,
                            connected,
                        });
                    }
                }
            }
            ast::ConcurrentKind::ForGenerate(generate) => {
                let id = self.labelled(statement, DeclClass::Block, "for generate", String::new());
                self.parents.push(id);
                let info = DeclInfo {
                    in_outline: false,
                    ..self.info(
                        &generate.param,
                        DeclClass::Genvar,
                        "generate parameter".into(),
                        None,
                        span,
                    )
                };
                self.declare(&generate.param, info);
                self.generate_body(&generate.body);
                self.parents.pop();
            }
            ast::ConcurrentKind::IfGenerate(generate) => {
                let id = self.labelled(statement, DeclClass::Block, "if generate", String::new());
                self.parents.push(id);
                for arm in &generate.arms {
                    self.generate_body(&arm.body);
                }
                if let Some(body) = &generate.else_arm {
                    self.generate_body(body);
                }
                self.parents.pop();
            }
            ast::ConcurrentKind::CaseGenerate(generate) => {
                let id = self.labelled(statement, DeclClass::Block, "case generate", String::new());
                self.parents.push(id);
                for arm in &generate.arms {
                    self.generate_body(&arm.body);
                }
                self.parents.pop();
            }
            ast::ConcurrentKind::SignalAssignment(_)
            | ast::ConcurrentKind::ProcedureCall { .. }
            | ast::ConcurrentKind::Assertion { .. } => {
                if statement.label.is_some() {
                    self.labelled(statement, DeclClass::Block, "statement", String::new());
                }
            }
        }
    }

    /// Declares the label of a concurrent statement, or an anonymous entry
    /// for the outline when it has none.
    fn labelled(
        &mut self,
        statement: &ast::ConcurrentStatement,
        class: DeclClass,
        fallback: &str,
        detail: String,
    ) -> usize {
        match &statement.label {
            Some(label) => {
                let mut info = self.info(label, class, detail, None, statement.span);
                info.text = first_line(slice(self.text, statement.span));
                self.declare(label, info)
            }
            None => {
                let info = DeclInfo {
                    name: fallback.to_string(),
                    class,
                    detail,
                    width: None,
                    text: first_line(slice(self.text, statement.span)),
                    name_span: Span::new(self.source, statement.span.start, statement.span.start),
                    full_span: statement.span,
                    ports: Vec::new(),
                    parent: self.parent(),
                    in_outline: true,
                };
                self.b.declare_anonymous(info)
            }
        }
    }

    fn generate_body(&mut self, body: &ast::GenerateBody) {
        self.declarations(&body.decls);
        self.statements(&body.statements);
    }

    fn sequential(&mut self, statements: &[ast::SequentialStatement]) {
        // Sequential statements declare nothing but loop parameters, and
        // the analysis resolves every name in them, so the walk only has
        // to reach the nested bodies for the loop parameters' sake.
        for statement in statements {
            match &statement.kind {
                ast::SequentialKind::If(if_stmt) => {
                    for arm in &if_stmt.arms {
                        self.sequential(&arm.statements);
                    }
                    if let Some(statements) = &if_stmt.else_statements {
                        self.sequential(statements);
                    }
                }
                ast::SequentialKind::Case(case) => {
                    for arm in &case.arms {
                        self.sequential(&arm.statements);
                    }
                }
                ast::SequentialKind::Loop(loop_stmt) => {
                    if let Some(ast::IterationScheme::For { param, .. }) = &loop_stmt.scheme {
                        let info = DeclInfo {
                            in_outline: false,
                            ..self.info(
                                param,
                                DeclClass::Genvar,
                                "loop parameter".into(),
                                None,
                                statement.span,
                            )
                        };
                        self.declare(param, info);
                    }
                    self.sequential(&loop_stmt.statements);
                }
                _ => {}
            }
        }
    }

    /// The parenthesised extent of an association list.
    fn association_span(&self, assocs: &[ast::AssociationElement]) -> Option<Span> {
        let first = assocs.first()?.span;
        let last = assocs.last()?.span;
        let bytes = self.text.as_bytes();
        let mut start = usize::try_from(first.start).ok()?.min(bytes.len());
        while start > 0 && bytes[start - 1] != b'(' {
            start -= 1;
        }
        let mut end = usize::try_from(last.end).ok()?.min(bytes.len());
        while end < bytes.len() && bytes[end] != b')' {
            end += 1;
        }
        Some(Span::new(
            self.source,
            u32::try_from(start).ok()?,
            u32::try_from(end).ok()?,
        ))
    }
}

/// The written form of a port mode.
fn mode_str(mode: ast::Mode) -> &'static str {
    match mode {
        ast::Mode::In => "in",
        ast::Mode::Out => "out",
        ast::Mode::Inout => "inout",
        ast::Mode::Buffer => "buffer",
        ast::Mode::Linkage => "linkage",
    }
}

/// Whether a subprogram is a function or a procedure.
fn subprogram_class(kind: ast::SubprogramKind) -> DeclClass {
    match kind {
        ast::SubprogramKind::Function => DeclClass::Function,
        ast::SubprogramKind::Procedure => DeclClass::Procedure,
    }
}

/// The last identifier of a name: `work.counter` is `counter`.
fn simple_name(name: &ast::Name) -> Option<String> {
    match name {
        ast::Name::Simple(ident) => Some(ident.name.clone()),
        ast::Name::Selected {
            suffix: ast::Suffix::Designator(ast::Designator::Ident(ident)),
            ..
        } => Some(ident.name.clone()),
        _ => None,
    }
}

/// The name of the unit an instantiation names.
fn instantiated_name(unit: &ast::InstantiatedUnit) -> Option<String> {
    match unit {
        ast::InstantiatedUnit::Component(name)
        | ast::InstantiatedUnit::Configuration(name)
        | ast::InstantiatedUnit::Entity { name, .. } => simple_name(name),
    }
}

/// The formal name of an association element, when it is a plain name.
fn formal_name(formal: &ast::Expr) -> Option<String> {
    match formal {
        ast::Expr::Name(name) => simple_name(name),
        _ => None,
    }
}

/// Where the formal was written.
fn formal_span(formal: &ast::Expr) -> Span {
    match formal {
        ast::Expr::Name(name) => name.span(),
        other => other.span(),
    }
}

// ---------------------------------------------------------------------------
// Keywords
// ---------------------------------------------------------------------------

/// Where in a VHDL file a position is, as far as keyword completion needs
/// to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// Outside any design unit.
    File,
    /// Inside a design unit: its declarative part or its statement part.
    ///
    /// The two are not told apart, because the boundary is the `begin`
    /// keyword and a file being edited often does not have one yet.
    Unit,
    /// Inside a process or subprogram body.
    Sequential,
}

/// The reserved words worth offering in `context`, sorted.
pub fn keywords(context: Context) -> Vec<&'static str> {
    let words: &[&str] = match context {
        Context::File => &[
            "library",
            "use",
            "entity",
            "architecture",
            "package",
            "configuration",
            "context",
            "of",
            "is",
            "end",
        ],
        Context::Unit => &[
            "signal",
            "variable",
            "constant",
            "shared",
            "type",
            "subtype",
            "component",
            "function",
            "procedure",
            "attribute",
            "alias",
            "file",
            "generic",
            "port",
            "map",
            "begin",
            "end",
            "is",
            "of",
            "process",
            "block",
            "generate",
            "for",
            "if",
            "else",
            "elsif",
            "when",
            "with",
            "select",
            "assert",
            "report",
            "severity",
            "entity",
            "architecture",
            "others",
            "all",
            "downto",
            "to",
            "array",
            "record",
            "in",
            "out",
            "inout",
        ],
        Context::Sequential => &[
            "if", "then", "elsif", "else", "end", "case", "when", "others", "loop", "for", "while",
            "next", "exit", "return", "wait", "until", "on", "for", "report", "severity", "assert",
            "null", "variable", "is", "begin", "downto", "to", "and", "or", "not", "xor", "nand",
            "nor", "xnor", "mod", "rem", "abs",
        ],
    };
    let mut out: Vec<&'static str> = words.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::vhdl::{Standard, analyze_source, lex_source, parse_source};

    const COUNTER: &str = "\
library ieee;
use ieee.std_logic_1164.all;

entity counter is
  generic (W : natural := 8);
  port (clk : in std_logic;
        rst : in std_logic;
        q   : out std_logic_vector(7 downto 0));
end entity counter;

architecture rtl of counter is
  signal count : std_logic_vector(7 downto 0);
begin
  tick : process (clk, rst) is
  begin
    if rst = '1' then
      count <= (others => '0');
    end if;
  end process tick;
  q <= count;
end architecture rtl;
";

    fn build(src: &str) -> (String, Index) {
        let mut map = SourceMap::new();
        let id = map.add("t.vhd", src).unwrap();
        let mut diags = Diagnostics::new();
        let analysis = analyze_source(&mut map, id, Standard::Vhdl2008, &mut diags);
        let file = parse_source(&map, id, Standard::Vhdl2008, &mut Diagnostics::new());
        let tokens: Vec<(TokenKind, Span)> =
            lex_source(&map, id, Standard::Vhdl2008, &mut Diagnostics::new())
                .iter()
                .map(|t| (t.kind, t.span))
                .collect();
        let index = index(&map, id, &analysis, &file, &tokens);
        (map.file(id).text().to_string(), index)
    }

    fn at(text: &str, needle: &str) -> u32 {
        u32::try_from(text.find(needle).expect("needle")).unwrap()
    }

    #[test]
    fn declares_the_entity_and_its_ports() {
        let (_, index) = build(COUNTER);
        let entity = index.find("counter", &[DeclClass::Entity]).unwrap();
        let names: Vec<&str> = index.decls[entity]
            .ports
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["clk", "rst", "q"]);
        assert_eq!(index.decls[entity].ports[0].detail, "in std_logic");
        assert_eq!(
            index.decls[entity].ports[2].detail,
            "out std_logic_vector(7 downto 0)"
        );
    }

    #[test]
    fn types_and_widths_come_from_the_analysis() {
        let (text, index) = build(COUNTER);
        let count = index.decl_at(at(&text, "count :")).unwrap();
        assert_eq!(index.decls[count].class, DeclClass::Signal);
        assert_eq!(index.decls[count].detail, "std_logic_vector(7 downto 0)");
        assert_eq!(index.decls[count].width, Some(8));
        let clk = index.decl_at(at(&text, "clk : in")).unwrap();
        assert_eq!(index.decls[clk].detail, "in std_logic");
        assert_eq!(index.decls[clk].width, Some(1));
        // The generic writes no mode, so none is shown.
        let w = index.find("W", &[DeclClass::Parameter]).unwrap();
        assert_eq!(index.decls[w].detail, "natural");
    }

    #[test]
    fn uses_resolve_across_the_architecture() {
        let (text, index) = build(COUNTER);
        let count = index.decl_at(at(&text, "count :")).unwrap();
        assert_eq!(index.decl_at(at(&text, "count <= (others")), Some(count));
        assert_eq!(index.decl_at(at(&text, "count;")), Some(count));
        assert_eq!(index.occurrences(count).len(), 3);

        let clk = index.decl_at(at(&text, "clk : in")).unwrap();
        // Declaration, sensitivity list.
        assert_eq!(index.occurrences(clk).len(), 2);
    }

    #[test]
    fn names_are_case_insensitive() {
        let (text, index) = build(
            "entity e is end entity;\narchitecture a of E is\n  signal Count : bit;\nbegin\n  COUNT <= '1';\nend architecture;\n",
        );
        let count = index.decl_at(at(&text, "Count :")).unwrap();
        assert_eq!(index.decl_at(at(&text, "COUNT <=")), Some(count));
        assert!(index.same_name("Count", "COUNT"));
    }

    #[test]
    fn the_outline_holds_units_processes_and_instances() {
        let (_, index) = build(COUNTER);
        let outline = index.outline();
        let roots: Vec<&str> = outline
            .iter()
            .map(|n| index.decls[n.decl].name.as_str())
            .collect();
        assert_eq!(roots, ["counter", "rtl"]);
        let arch = &outline[1];
        let children: Vec<&str> = arch
            .children
            .iter()
            .map(|n| index.decls[n.decl].name.as_str())
            .collect();
        assert_eq!(children, ["count", "tick"]);
        let tick = arch.children.last().unwrap();
        assert_eq!(index.decls[tick.decl].class, DeclClass::Process);
        assert_eq!(index.decls[tick.decl].detail, "(clk, rst)");
    }

    #[test]
    fn records_port_maps_of_instantiations() {
        let src = "\
entity sub is port (clk : in bit; q : out bit); end entity;
architecture s of sub is begin end architecture;

entity top is end entity;
architecture t of top is
  signal c : bit;
  signal d : bit;
begin
  u0 : entity work.sub port map (clk => c, q => d);
end architecture;
";
        let (text, index) = build(src);
        let site = index.instance_at(at(&text, "clk => c")).unwrap();
        assert_eq!(site.target, "sub");
        let formals: Vec<&str> = site.connected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(formals, ["clk", "q"]);
        let inst = index.find("u0", &[DeclClass::Instance]).unwrap();
        assert_eq!(index.decls[inst].detail, "sub");
    }

    #[test]
    fn components_carry_their_ports() {
        let src = "\
entity top is end entity;
architecture t of top is
  component sub is port (clk : in bit); end component;
begin
end architecture;
";
        let (_, index) = build(src);
        let component = index.find("sub", &[DeclClass::Component]).unwrap();
        assert_eq!(index.decls[component].ports.len(), 1);
        assert_eq!(index.decls[component].ports[0].name, "clk");
    }

    #[test]
    fn types_subprograms_and_enumerations() {
        let src = "\
package p is
  type state_t is (idle, run);
  function f (x : bit) return bit;
end package;

package body p is
  function f (x : bit) return bit is
  begin
    return x;
  end function;
end package body;
";
        let (_, index) = build(src);
        assert!(index.find("state_t", &[DeclClass::Type]).is_some());
        assert!(index.find("idle", &[DeclClass::EnumLiteral]).is_some());
        let f = index.find("f", &[DeclClass::Function]).unwrap();
        assert_eq!(index.decls[f].detail, "return bit");
    }

    #[test]
    fn survives_a_file_that_does_not_analyse() {
        let (_, index) = build("entity e is port (a : in nonexistent_t); end entity;\n");
        assert!(index.find("e", &[DeclClass::Entity]).is_some());
        // The port is still declared, with no type the analysis could give.
        let port = index.find("a", &[DeclClass::Port]).unwrap();
        assert_eq!(index.decls[port].width, None);
    }

    #[test]
    fn a_call_refers_to_the_subprogram() {
        let src = "\
entity e is end entity;
architecture a of e is
  function twice (x : bit) return bit is
  begin
    return x;
  end function;
  signal s : bit;
begin
  s <= twice(s);
end architecture;
";
        let (text, index) = build(src);
        let f = index.find("twice", &[DeclClass::Function]).unwrap();
        assert_eq!(index.decl_at(at(&text, "twice(s)")), Some(f));
        assert_eq!(index.occurrences(f).len(), 2);
    }

    #[test]
    fn keyword_lists_are_sorted() {
        for context in [Context::File, Context::Unit, Context::Sequential] {
            let words = keywords(context);
            assert!(words.windows(2).all(|w| w[0] < w[1]), "{context:?}");
        }
        assert!(keywords(Context::File).contains(&"entity"));
        assert!(keywords(Context::Unit).contains(&"process"));
        assert!(keywords(Context::Sequential).contains(&"elsif"));
    }
}
