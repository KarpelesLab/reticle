//! Getting back from a [`Span`] to the tree node it came from.
//!
//! [`crate::vhdl::sema`] annotates the parse tree through side tables keyed
//! by span, and stores two things that elaboration has to follow *back*
//! into the tree:
//!
//! - a constraint bound that is not locally static, as
//!   `Bound::Dynamic(span)` — the span of the bound's expression, which
//!   only becomes a number once the generics are known;
//! - a subprogram's body, as `SubprogramBody::Vhdl(span)` — the span of the
//!   body, which elaboration needs in order to inline the call.
//!
//! The tree has no node ids, so [`AstIndex`] builds the reverse maps once
//! per elaboration by walking every declarative part of every design unit.
//! It also collects the identifiers of every attribute specification in the
//! design, since [`crate::vhdl::sema::Analysis::attribute_value`] is a
//! lookup by name and elaboration has to know which names to ask for.
//!
//! Statement ranges (`for i in 0 to n-1 loop`, `for ... generate`) are not
//! indexed: elaboration holds that part of the tree while it unrolls it.

use std::collections::HashMap;

use crate::source::Span;
use crate::vhdl::ast::{self, Declaration, Expr, InterfaceDecl, SubtypeIndication};
use crate::vhdl::sema::Analysis;

/// The reverse maps from span to tree node.
#[derive(Debug, Default)]
pub(crate) struct AstIndex<'a> {
    bounds: HashMap<Span, &'a Expr>,
    bodies: HashMap<Span, &'a ast::SubprogramBody>,
    /// Attribute designators used in attribute specifications, in the
    /// order they were first seen, so output stays deterministic.
    attributes: Vec<String>,
    /// Alias declarations as `(designator span, aliased name span)`.
    aliases: Vec<(Span, Span)>,
}

impl<'a> AstIndex<'a> {
    /// Indexes every analysed design unit.
    pub(crate) fn build(a: &'a Analysis) -> AstIndex<'a> {
        let mut out = AstIndex::default();
        for file in &a.files {
            for du in &file.ast.units {
                out.unit(&du.unit);
            }
        }
        out
    }

    /// The expression whose span is `span`, if it is a constraint bound.
    pub(crate) fn bound(&self, span: Span) -> Option<&'a Expr> {
        self.bounds.get(&span).copied()
    }

    /// The subprogram body whose span is `span`.
    pub(crate) fn body(&self, span: Span) -> Option<&'a ast::SubprogramBody> {
        self.bodies.get(&span).copied()
    }

    /// Every attribute designator named by an attribute specification.
    pub(crate) fn attributes(&self) -> &[String] {
        &self.attributes
    }

    /// Every alias, as the span of its designator and of the name it
    /// aliases. A subprogram alias copies the aliased profile at the point
    /// it is written, so elaboration follows this pair to find the body.
    pub(crate) fn aliases(&self) -> &[(Span, Span)] {
        &self.aliases
    }

    fn note_attribute(&mut self, name: &str) {
        if !self.attributes.iter().any(|a| a.eq_ignore_ascii_case(name)) {
            self.attributes.push(name.to_owned());
        }
    }

    fn unit(&mut self, u: &'a ast::LibraryUnit) {
        match u {
            ast::LibraryUnit::Entity(e) => {
                self.interfaces(&e.generics);
                self.interfaces(&e.ports);
                self.decls(&e.decls);
                self.statements(&e.statements);
            }
            ast::LibraryUnit::Architecture(arch) => {
                self.decls(&arch.decls);
                self.statements(&arch.statements);
            }
            ast::LibraryUnit::Package(p) => {
                self.interfaces(&p.generics);
                self.decls(&p.decls);
            }
            ast::LibraryUnit::PackageBody(p) => self.decls(&p.decls),
            ast::LibraryUnit::Configuration(c) => self.decls(&c.decls),
            ast::LibraryUnit::PackageInstantiation(_) | ast::LibraryUnit::Context(_) => {}
        }
    }

    fn statements(&mut self, stmts: &'a [ast::ConcurrentStatement]) {
        for s in stmts {
            match &s.kind {
                ast::ConcurrentKind::Process(p) => self.decls(&p.decls),
                ast::ConcurrentKind::Block(b) => {
                    self.interfaces(&b.generics);
                    self.interfaces(&b.ports);
                    self.decls(&b.decls);
                    self.statements(&b.statements);
                }
                ast::ConcurrentKind::ForGenerate(g) => self.generate_body(&g.body),
                ast::ConcurrentKind::IfGenerate(g) => {
                    for arm in &g.arms {
                        self.generate_body(&arm.body);
                    }
                    if let Some(e) = &g.else_arm {
                        self.generate_body(e);
                    }
                }
                ast::ConcurrentKind::CaseGenerate(g) => {
                    for arm in &g.arms {
                        self.generate_body(&arm.body);
                    }
                }
                _ => {}
            }
        }
    }

    fn generate_body(&mut self, body: &'a ast::GenerateBody) {
        self.decls(&body.decls);
        self.statements(&body.statements);
    }

    fn interfaces(&mut self, list: &'a [InterfaceDecl]) {
        for i in list {
            match i {
                InterfaceDecl::Object(o) => self.subtype(&o.subtype),
                InterfaceDecl::Subprogram(s) => self.spec(&s.spec),
                InterfaceDecl::Type(_) | InterfaceDecl::Package(_) => {}
            }
        }
    }

    fn spec(&mut self, spec: &'a ast::SubprogramSpec) {
        self.interfaces(&spec.generics);
        self.interfaces(&spec.params);
    }

    fn decls(&mut self, decls: &'a [Declaration]) {
        for d in decls {
            match d {
                Declaration::Object(o) => self.subtype(&o.subtype),
                Declaration::File(f) => self.subtype(&f.subtype),
                Declaration::Subtype(s) => self.subtype(&s.subtype),
                Declaration::Alias(a) => {
                    if let Some(s) = &a.subtype {
                        self.subtype(s);
                    }
                    self.aliases.push((a.designator.span(), a.target.span()));
                }
                Declaration::AttributeSpec(s) => {
                    let name = s.attribute.name.clone();
                    self.note_attribute(&name);
                }
                Declaration::Type(t) => {
                    if let Some(def) = &t.def {
                        self.type_def(def);
                    }
                }
                Declaration::Component(c) => {
                    self.interfaces(&c.generics);
                    self.interfaces(&c.ports);
                }
                Declaration::Subprogram(s) => self.spec(&s.spec),
                Declaration::SubprogramBody(b) => {
                    self.spec(&b.spec);
                    self.decls(&b.decls);
                    self.bodies.insert(b.span, b);
                }
                Declaration::Package(p) => {
                    self.interfaces(&p.generics);
                    self.decls(&p.decls);
                }
                Declaration::PackageBody(p) => self.decls(&p.decls),
                _ => {}
            }
        }
    }

    fn type_def(&mut self, def: &'a ast::TypeDef) {
        match def {
            ast::TypeDef::Range(r) => self.range(r),
            ast::TypeDef::Physical(p) => self.range(&p.range),
            ast::TypeDef::Array(a) => {
                for i in &a.indices {
                    if let ast::ArrayIndex::Constrained(r) = i {
                        self.discrete_range(r);
                    }
                }
                self.subtype(&a.element);
            }
            ast::TypeDef::Record(r) => {
                for e in &r.elements {
                    self.subtype(&e.subtype);
                }
            }
            ast::TypeDef::Access(s) => self.subtype(s),
            ast::TypeDef::Protected(p) => self.decls(&p.decls),
            ast::TypeDef::ProtectedBody(p) => self.decls(&p.decls),
            ast::TypeDef::Enumeration(_) | ast::TypeDef::File(_) => {}
        }
    }

    fn subtype(&mut self, si: &'a SubtypeIndication) {
        if let Some(c) = &si.constraint {
            self.constraint(c);
        }
    }

    fn constraint(&mut self, c: &'a ast::Constraint) {
        match c {
            ast::Constraint::Range(r) => self.range(r),
            ast::Constraint::Array {
                indices, element, ..
            } => {
                for i in indices {
                    self.discrete_range(i);
                }
                if let Some(e) = element {
                    self.constraint(e);
                }
            }
            ast::Constraint::Record(fields, _) => {
                for f in fields {
                    self.constraint(&f.constraint);
                }
            }
        }
    }

    fn discrete_range(&mut self, r: &'a ast::DiscreteRange) {
        match r {
            ast::DiscreteRange::Range(r) => self.range(r),
            ast::DiscreteRange::Subtype(s) => self.subtype(s),
        }
    }

    fn range(&mut self, r: &'a ast::Range) {
        if let ast::Range::Bounds { left, right, .. } = r {
            self.bounds.insert(left.span(), left);
            self.bounds.insert(right.span(), right);
        }
    }
}
