//! The analysis pass: design units and declarations.
//!
//! `run` walks the units of a design in dependency order and, for each,
//! builds its declarative regions, declares everything in them, and checks
//! its statements ([`super::stmt`]) and expressions ([`super::expr`]).
//! This file holds the driver, the context clause handling (clause 13.4),
//! and the declarations (clause 6): objects, types, subtypes, aliases,
//! attributes, components, subprograms and packages.
//!
//! The checker is a single `Checker` value threaded through all of it;
//! the other files add `impl Checker` blocks. Diagnostics carry `V0xxx`
//! codes grouped by area: `V01xx` libraries and units, `V02xx` names and
//! declarations, `V03xx` types and expressions, `V04xx` statements, `V05xx`
//! association lists, `V06xx` standard-revision gating.

use std::collections::HashMap;

use crate::diag::{Diagnostic, Diagnostics};
use crate::intern::Symbol;
use crate::source::{SourceMap, Span};
use crate::vhdl::Standard;
use crate::vhdl::ast::{
    self, ContextItem, Declaration, Designator, Direction, InterfaceDecl, LibraryUnit, Mode, Name,
    ObjectKind, SubprogramKind, Suffix, TypeDef,
};

use super::constant::{Value, parse_integer_literal, parse_real_literal};
use super::library::{LibraryUnitKind, UnitId};
use super::overload::Cand;
use super::scope;
use super::types::{Bound, Bounds, Constraint, TypeId, TypeKind};
use super::{
    Analysis, DeclId, DeclKind, ObjectClass, ObjectRole, Param, RegionId, RegionKind, Signature,
    SubprogramBody,
};

/// Frequently used designators, interned once.
///
/// A few are interned for the passes that follow (the lowering pass needs
/// `to_string` and friends to recognise the rendering functions), so not
/// every field is read here.
#[allow(dead_code)]
pub(crate) struct Syms {
    pub work: Symbol,
    pub std: Symbol,
    pub standard: Symbol,
    pub foreign: Symbol,
    pub to_string: Symbol,
    pub to_hstring: Symbol,
    pub to_ostring: Symbol,
    pub to_bstring: Symbol,
    pub minimum: Symbol,
    pub maximum: Symbol,
    pub now: Symbol,
    pub deallocate: Symbol,
    pub file_open: Symbol,
    pub file_close: Symbol,
    pub read: Symbol,
    pub write: Symbol,
    pub flush: Symbol,
    pub endfile: Symbol,
    pub guard: Symbol,
    pub ieee: Symbol,
    pub numeric_std: Symbol,
    pub std_logic_1164: Symbol,
    pub math_real: Symbol,
    pub numeric_bit: Symbol,
    pub std_logic_arith: Symbol,
    pub std_logic_unsigned: Symbol,
    pub std_logic_signed: Symbol,
}

/// The enclosing subprogram, for the rules that depend on it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubCtx {
    pub kind: SubprogramKind,
    pub pure: bool,
    pub ret: Option<TypeId>,
    pub decl: DeclId,
}

/// The enclosing process.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProcCtx {
    pub has_sensitivity: bool,
    pub sensitivity_span: Option<Span>,
    pub has_wait: bool,
}

/// What surrounds the construct being checked.
#[derive(Clone, Debug)]
pub(crate) struct Ctx {
    pub subprogram: Option<SubCtx>,
    pub process: Option<ProcCtx>,
    /// Labels of the enclosing loops, innermost last (`None` for an
    /// unlabelled loop).
    pub loops: Vec<Option<Symbol>>,
    /// True inside a package declaration (deferred constants allowed).
    pub in_package: bool,
    /// The current unit's library.
    pub library: Symbol,
    /// The current unit.
    pub unit: Option<UnitId>,
}

impl Ctx {
    /// A context with nothing open, for a unit of `library`.
    fn new(library: Symbol) -> Ctx {
        Ctx {
            subprogram: None,
            process: None,
            loops: Vec::new(),
            in_package: false,
            library,
            unit: None,
        }
    }
}

/// The analysis state for one design.
pub(crate) struct Checker<'a> {
    pub a: &'a mut Analysis,
    pub map: &'a SourceMap,
    pub diags: &'a mut Diagnostics,
    pub standard: Standard,
    /// The current declarative region.
    pub region: RegionId,
    pub ctx: Ctx,
    pub syms: Syms,
    /// Inferred candidate sets, per expression, for the unit being
    /// analysed.
    pub infer_cache: HashMap<Span, Vec<Cand>>,
    /// True while analysing the bundled libraries, where the style
    /// warnings are pointless.
    pub in_stdlib: bool,
    /// One region per library, holding the primary unit declarations.
    pub library_regions: HashMap<Symbol, RegionId>,
    /// Context declarations by unit, for `context` references.
    pub context_decls: HashMap<UnitId, Vec<ContextItem>>,
    /// The region holding each unit's context clause. Clause 13.1 makes a
    /// primary unit's context clause apply to its secondary units, so an
    /// architecture reads its entity's entry and a package body its
    /// package's.
    pub unit_contexts: HashMap<UnitId, RegionId>,
    /// Non-zero while the actual for a formal of mode `out` is resolved:
    /// naming an object there writes it rather than reading it, whatever
    /// the object's own mode is.
    pub writing_actual: u32,
}

/// Analyses every unit of `order`.
pub(crate) fn run(
    a: &mut Analysis,
    order: &[UnitId],
    map: &SourceMap,
    standard: Standard,
    diags: &mut Diagnostics,
) {
    let syms = {
        let i = &mut a.interner;
        Syms {
            work: i.intern_ci("work"),
            std: i.intern_ci("std"),
            standard: i.intern_ci("standard"),
            foreign: i.intern_ci("foreign"),
            to_string: i.intern_ci("to_string"),
            to_hstring: i.intern_ci("to_hstring"),
            to_ostring: i.intern_ci("to_ostring"),
            to_bstring: i.intern_ci("to_bstring"),
            minimum: i.intern_ci("minimum"),
            maximum: i.intern_ci("maximum"),
            now: i.intern_ci("now"),
            deallocate: i.intern_ci("deallocate"),
            file_open: i.intern_ci("file_open"),
            file_close: i.intern_ci("file_close"),
            read: i.intern_ci("read"),
            write: i.intern_ci("write"),
            flush: i.intern_ci("flush"),
            endfile: i.intern_ci("endfile"),
            guard: i.intern_ci("guard"),
            ieee: i.intern_ci("ieee"),
            numeric_std: i.intern_ci("numeric_std"),
            std_logic_1164: i.intern_ci("std_logic_1164"),
            math_real: i.intern_ci("math_real"),
            numeric_bit: i.intern_ci("numeric_bit"),
            std_logic_arith: i.intern_ci("std_logic_arith"),
            std_logic_unsigned: i.intern_ci("std_logic_unsigned"),
            std_logic_signed: i.intern_ci("std_logic_signed"),
        }
    };
    let root = a.root();
    let work = syms.work;
    let mut c = Checker {
        a,
        map,
        diags,
        standard,
        region: root,
        ctx: Ctx::new(work),
        syms,
        infer_cache: HashMap::new(),
        in_stdlib: false,
        library_regions: HashMap::new(),
        context_decls: HashMap::new(),
        unit_contexts: HashMap::new(),
        writing_actual: 0,
    };
    // Library declarations: every library that has units, plus `std`
    // and `work`.
    let mut libs: Vec<Symbol> = c.a.units.iter().map(|u| u.library).collect();
    libs.push(c.syms.std);
    libs.push(c.syms.work);
    libs.sort();
    libs.dedup();
    let Some(first) = c.a.files.first().map(|f| f.source) else {
        return;
    };
    for lib in libs {
        let region = c.a.add_region(RegionKind::Root, None);
        let spelling = c.a.name(lib).to_owned();
        let span = Span::new(first, 0, 0);
        let decl = c.a.add_decl(root, lib, spelling, DeclKind::Library, span);
        c.a.add_library(lib, decl);
        c.library_regions.insert(lib, region);
    }
    for &u in order {
        c.analyze_unit(u);
    }
}

impl<'a> Checker<'a> {
    // --- small helpers -----------------------------------------------------

    /// Interns a basic identifier case-insensitively, an extended one
    /// exactly (with its backslashes).
    pub(crate) fn ident_sym(&mut self, id: &ast::Ident) -> Symbol {
        if id.extended {
            self.a.interner.intern(&format!("\\{}\\", id.name))
        } else {
            self.a.interner.intern_ci(&id.name)
        }
    }

    /// The symbol and spelling of a designator.
    pub(crate) fn designator_sym(&mut self, d: &Designator) -> (Symbol, String) {
        match d {
            Designator::Ident(i) => (self.ident_sym(i), i.name.clone()),
            Designator::Char { ch, .. } => {
                let s = format!("'{ch}'");
                (self.a.interner.intern(&s), s)
            }
            Designator::Operator { symbol, .. } => {
                let s = format!("\"{}\"", symbol.to_lowercase());
                (self.a.interner.intern(&s), format!("\"{symbol}\""))
            }
        }
    }

    /// The text of a span.
    pub(crate) fn text(&self, span: Span) -> &'a str {
        let f = self.map.file(span.file);
        &f.text()[span.start as usize..span.end as usize]
    }

    /// Pushes a diagnostic.
    pub(crate) fn push(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    /// Reports an error with one primary label.
    pub(crate) fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.diags
            .push(Diagnostic::error(msg).with_code(code).with_span(span));
    }

    /// Reports a warning with one primary label.
    pub(crate) fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        if self.in_stdlib {
            return;
        }
        self.diags
            .push(Diagnostic::warning(msg).with_code(code).with_span(span));
    }

    /// True for VHDL-2008 or later.
    pub(crate) fn v2008(&self) -> bool {
        self.standard >= Standard::Vhdl2008
    }

    /// Reports the use of a VHDL-2008 feature in VHDL-93 mode.
    pub(crate) fn require_2008(&mut self, span: Span, what: &str) -> bool {
        if self.v2008() {
            return true;
        }
        self.push(
            Diagnostic::error(format!("{what} require VHDL-2008"))
                .with_code("V0600")
                .with_span(span)
                .with_note("this file is analysed as VHDL-93; drop the `-- sema: vhdl93` header or avoid the feature"),
        );
        false
    }

    /// Describes a type for a diagnostic.
    pub(crate) fn ty_name(&self, t: TypeId) -> String {
        self.a.describe_type(t, Some(self.map))
    }

    /// Opens a nested region and makes it current; returns the previous.
    pub(crate) fn enter(&mut self, kind: RegionKind) -> RegionId {
        let r = self.a.add_region(kind, Some(self.region));
        std::mem::replace(&mut self.region, r)
    }

    /// Restores the region returned by [`Checker::enter`].
    pub(crate) fn leave(&mut self, prev: RegionId) {
        self.region = prev;
    }

    /// Declares `name` in the current region after checking that it does
    /// not clash with a directly visible declaration of the same region
    /// (overloadable declarations only clash with homographs).
    pub(crate) fn declare(
        &mut self,
        name: Symbol,
        spelling: &str,
        kind: DeclKind,
        span: Span,
    ) -> DeclId {
        let overloadable = scope::is_overloadable(&kind);
        let existing: Vec<DeclId> = self.a.region(self.region).direct(name).to_vec();
        for e in existing {
            let ed = self.a.decl(e);
            let clash = if overloadable {
                // Compare profiles once the new declaration exists; for
                // enum literals the type is enough.
                match (&ed.kind, &kind) {
                    (DeclKind::EnumLiteral { ty: a, .. }, DeclKind::EnumLiteral { ty: b, .. }) => {
                        self.a.same_base(*a, *b)
                    }
                    (
                        DeclKind::Subprogram { sig: s1, .. },
                        DeclKind::Subprogram { sig: s2, .. },
                    ) => same_profile(self.a, s1, s2),
                    _ => !scope::is_overloadable(&ed.kind),
                }
            } else {
                !matches!(ed.kind, DeclKind::Error)
            };
            if clash {
                let prev = ed.span;
                self.push(
                    Diagnostic::error(format!("`{spelling}` is already declared in this region"))
                        .with_code("V0202")
                        .with_label(span, "redeclared here")
                        .with_secondary(prev, "first declared here"),
                );
                break;
            }
        }
        let id = self.a.add_decl(self.region, name, spelling, kind, span);
        self.a.set_ref(span, id);
        id
    }

    /// The declaration of a design unit in a library, by name.
    pub(crate) fn find_unit(
        &self,
        library: Symbol,
        name: Symbol,
        kind: LibraryUnitKind,
    ) -> Option<UnitId> {
        let library = if library == self.syms.work {
            self.ctx.library
        } else {
            library
        };
        self.a
            .units
            .iter()
            .position(|u| u.library == library && u.name == name && u.kind == kind)
            .map(UnitId::from_index)
    }

    // --- units -------------------------------------------------------------

    fn analyze_unit(&mut self, uid: UnitId) {
        let unit = self.a.units[uid.index()].clone();
        let file = &self.a.files[unit.file];
        self.in_stdlib = self.map.file(file.source).name().starts_with("<reticle>/");
        self.ctx = Ctx {
            unit: Some(uid),
            ..Ctx::new(unit.library)
        };
        self.infer_cache.clear();
        // The context region of the unit.
        let root = self.a.root();
        self.region = root;
        self.enter(RegionKind::Context);
        self.unit_contexts.insert(uid, self.region);
        // Implicit `library std, work; use std.standard.all;` (clause
        // 13.2), except inside `std.standard` itself.
        let is_standard = unit.library == self.syms.std && unit.name == self.syms.standard;
        if !is_standard {
            let std_unit =
                self.find_unit(self.syms.std, self.syms.standard, LibraryUnitKind::Package);
            if let Some(su) = std_unit
                && let Some(r) = self.a.units[su.index()].region
            {
                self.use_all(r);
            }
        }
        // AST access: clone the design unit's context clause; the unit body
        // is borrowed through a raw index to avoid cloning big trees.
        let context: Vec<ContextItem> = self.a.files[unit.file].ast.units[unit.index]
            .context
            .clone();
        for item in &context {
            self.context_item(item);
        }
        // The unit body. `Analysis` owns the AST; take it out for the
        // duration of the analysis and put it back, which keeps every
        // method free to borrow `self` mutably.
        let mut ast = std::mem::take(&mut self.a.files[unit.file].ast);
        let du = &ast.units[unit.index];
        match &du.unit {
            LibraryUnit::Entity(e) => self.entity(uid, e),
            LibraryUnit::Architecture(arch) => self.architecture(uid, arch),
            LibraryUnit::Package(p) => self.package(uid, p),
            LibraryUnit::PackageBody(b) => self.package_body(uid, b),
            LibraryUnit::PackageInstantiation(p) => self.package_instantiation(Some(uid), p),
            LibraryUnit::Configuration(cfg) => self.configuration(uid, cfg),
            LibraryUnit::Context(cd) => {
                self.context_decls.insert(uid, cd.items.clone());
                let lib_region = self.library_regions[&unit.library];
                let name = self.ident_sym(&cd.name);
                let prev = std::mem::replace(&mut self.region, lib_region);
                let d = self.declare(
                    name,
                    &cd.name.name,
                    DeclKind::Unit {
                        unit: uid,
                        region: self.region,
                    },
                    cd.name.span,
                );
                self.region = prev;
                self.a.units[uid.index()].decl = Some(d);
            }
        }
        std::mem::swap(&mut self.a.files[unit.file].ast, &mut ast);
        self.a.units[uid.index()].analyzed = true;
        self.region = root;
    }

    /// Declares a primary unit in its library's region.
    fn declare_unit(&mut self, uid: UnitId, name: &ast::Ident, region: RegionId) -> DeclId {
        let lib = self.ctx.library;
        let lib_region = self.library_regions[&lib];
        let sym = self.ident_sym(name);
        let prev = std::mem::replace(&mut self.region, lib_region);
        let d = self.declare(
            sym,
            &name.name,
            DeclKind::Unit { unit: uid, region },
            name.span,
        );
        self.region = prev;
        self.a.units[uid.index()].decl = Some(d);
        self.a.units[uid.index()].region = Some(region);
        d
    }

    fn entity(&mut self, uid: UnitId, e: &ast::EntityDecl) {
        let prev = self.enter(RegionKind::Entity);
        let region = self.region;
        let d = self.declare_unit(uid, &e.name, region);
        // The entity's own name is visible inside it (for `e'path_name`
        // and end labels); alias it into the region.
        let sym = self.ident_sym(&e.name);
        self.a.add_use(region, sym, d);
        self.interface_list(&e.generics, ObjectRole::Generic);
        self.interface_list(&e.ports, ObjectRole::Port);
        self.declarations(&e.decls);
        self.concurrent_statements(&e.statements);
        self.leave(prev);
    }

    fn architecture(&mut self, uid: UnitId, arch: &ast::ArchitectureBody) {
        let ename = match &arch.entity {
            Name::Simple(i) => Some(i.clone()),
            _ => None,
        };
        let Some(ename) = ename else {
            self.error(
                "V0103",
                arch.entity.span(),
                "the entity name of an architecture must be a simple name",
            );
            return;
        };
        let esym = self.ident_sym(&ename);
        let Some(eu) = self.find_unit(self.ctx.library, esym, LibraryUnitKind::Entity) else {
            let lib = self.a.name(self.ctx.library).to_owned();
            let mut d = Diagnostic::error(format!(
                "architecture `{}` of unknown entity `{}`",
                arch.name.name, ename.name
            ))
            .with_code("V0103")
            .with_label(
                ename.span,
                format!("no entity `{}` in library `{lib}`", ename.name),
            );
            let known: Vec<String> = self
                .a
                .units
                .iter()
                .filter(|u| u.kind == LibraryUnitKind::Entity && u.library == self.ctx.library)
                .map(|u| self.a.name(u.name).to_owned())
                .collect();
            if let Some(s) = super::suggest(&ename.name, known.iter().map(String::as_str)) {
                d = d.with_note(format!("did you mean `{s}`?"));
            }
            self.push(d);
            return;
        };
        let Some(eregion) = self.a.units[eu.index()].region else {
            return;
        };
        if let Some(ed) = self.a.units[eu.index()].decl {
            self.a.set_ref(ename.span, ed);
        }
        // The architecture region nests in the entity's region, which
        // itself nests in the entity's context; the architecture's own
        // context clause was processed into `self.region`, so bridge it:
        // the entity's declarations become potentially visible here by
        // copying them, which keeps the lookup a single parent walk.
        if let Some(&ectx) = self.unit_contexts.get(&eu) {
            self.import_uses(ectx);
        }
        let prev = self.enter(RegionKind::Architecture);
        let region = self.region;
        self.import_region(eregion);
        self.a.units[uid.index()].region = Some(region);
        let sym = self.ident_sym(&arch.name);
        let d = self.a.add_decl(
            region,
            sym,
            arch.name.name.clone(),
            DeclKind::Unit { unit: uid, region },
            arch.name.span,
        );
        self.a.units[uid.index()].decl = Some(d);
        self.declarations(&arch.decls);
        self.concurrent_statements(&arch.statements);
        self.leave(prev);
    }

    /// Makes everything a region's `use` clauses made potentially
    /// visible potentially visible in the current region too, without
    /// touching its direct declarations. This is how a secondary unit
    /// inherits its primary's context clause (clause 13.1).
    pub(crate) fn import_uses(&mut self, from: RegionId) {
        let used: Vec<(Symbol, Vec<DeclId>)> = self
            .a
            .region(from)
            .used
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let region = self.region;
        for (name, ds) in used {
            for d in ds {
                self.a.add_use(region, name, d);
            }
        }
    }

    /// Makes every declaration of `from` directly visible in the current
    /// region (used to expose an entity's declarations to its
    /// architecture and a package's to its body).
    pub(crate) fn import_region(&mut self, from: RegionId) {
        let decls: Vec<DeclId> = self.a.region(from).decls.clone();
        let region = self.region;
        for d in decls {
            let name = self.a.decl(d).name;
            let r = &mut self.a.regions[region.index()];
            r.names.entry(name).or_default().push(d);
        }
        // `use` clauses of the entity/package also carry over.
        let used: Vec<(Symbol, Vec<DeclId>)> = self
            .a
            .region(from)
            .used
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        for (name, ds) in used {
            for d in ds {
                self.a.add_use(region, name, d);
            }
        }
    }

    fn package(&mut self, uid: UnitId, p: &ast::PackageDecl) {
        let prev = self.enter(RegionKind::Package);
        let region = self.region;
        self.declare_unit(uid, &p.name, region);
        self.ctx.in_package = true;
        self.interface_list(&p.generics, ObjectRole::Generic);
        self.declarations(&p.decls);
        self.ctx.in_package = false;
        // std.standard: capture the predefined types once declared.
        if self.ctx.library == self.syms.std && self.a.units[uid.index()].name == self.syms.standard
        {
            self.capture_builtins(region);
        }
        self.leave(prev);
    }

    fn capture_builtins(&mut self, region: RegionId) {
        let names = [
            "boolean",
            "bit",
            "character",
            "severity_level",
            "integer",
            "real",
            "time",
            "natural",
            "positive",
            "string",
            "bit_vector",
            "boolean_vector",
            "file_open_kind",
            "file_open_status",
        ];
        for n in names {
            let sym = self.a.interner.intern_ci(n);
            let Some(&d) = self.a.region(region).direct(sym).first() else {
                continue;
            };
            let t = match self.a.decl(d).kind {
                DeclKind::Type(t) | DeclKind::Subtype(t) => t,
                _ => continue,
            };
            let b = &mut self.a.builtins;
            match n {
                "boolean" => b.boolean = t,
                "bit" => b.bit = t,
                "character" => b.character = t,
                "severity_level" => b.severity_level = t,
                "integer" => b.integer = t,
                "real" => b.real = t,
                "time" => b.time = t,
                "natural" => b.natural = t,
                "positive" => b.positive = t,
                "string" => b.string = t,
                "bit_vector" => b.bit_vector = t,
                "boolean_vector" => b.boolean_vector = t,
                "file_open_kind" => b.file_open_kind = t,
                "file_open_status" => b.file_open_status = t,
                _ => {}
            }
        }
    }

    fn package_body(&mut self, uid: UnitId, b: &ast::PackageBody) {
        let name = self.ident_sym(&b.name);
        let Some(pu) = self.find_unit(self.ctx.library, name, LibraryUnitKind::Package) else {
            self.error(
                "V0104",
                b.name.span,
                format!("package body for unknown package `{}`", b.name.name),
            );
            return;
        };
        let Some(pregion) = self.a.units[pu.index()].region else {
            return;
        };
        if let Some(pd) = self.a.units[pu.index()].decl {
            self.a.set_ref(b.name.span, pd);
        }
        if let Some(&pctx) = self.unit_contexts.get(&pu) {
            self.import_uses(pctx);
        }
        let prev = self.enter(RegionKind::PackageBody);
        let region = self.region;
        self.import_region(pregion);
        self.a.units[uid.index()].region = Some(region);
        self.declarations(&b.decls);
        self.check_package_complete(pregion, b.name.span);
        self.leave(prev);
    }

    /// After a package body: every subprogram declared in the package
    /// needs a body (or `foreign`), every deferred constant a value.
    fn check_package_complete(&mut self, pregion: RegionId, body_span: Span) {
        let decls: Vec<DeclId> = self.a.region(pregion).decls.clone();
        for d in decls {
            let decl = self.a.decl(d).clone();
            match &decl.kind {
                DeclKind::Subprogram {
                    body: SubprogramBody::None,
                    ..
                } => {
                    let profile = self.a.describe_subprogram(d);
                    self.push(
                        Diagnostic::error(format!(
                            "`{}` is declared in the package but has no body",
                            decl.spelling
                        ))
                        .with_code("V0105")
                        .with_label(decl.span, format!("declared here as `{profile}`"))
                        .with_secondary(body_span, "this package body does not define it"),
                    );
                }
                DeclKind::Object { deferred: true, .. } => {
                    self.push(
                        Diagnostic::error(format!(
                            "deferred constant `{}` has no full declaration in the package body",
                            decl.spelling
                        ))
                        .with_code("V0106")
                        .with_label(decl.span, "declared here without a value")
                        .with_secondary(
                            body_span,
                            "expected `constant ... := value;` in this body",
                        ),
                    );
                }
                _ => {}
            }
        }
    }

    fn package_instantiation(&mut self, uid: Option<UnitId>, p: &ast::PackageInstantiation) {
        // The uninstantiated package's region is reused: its generic
        // types stay generic, which the type checker treats leniently.
        // Elaboration performs the actual substitution.
        let target = self.resolve_package_name(&p.uninstantiated);
        let region = target.unwrap_or(self.region);
        for elem in &p.generic_map {
            if let ast::Actual::Expr(e) = &elem.actual {
                // Type generics take type marks; values take expressions.
                let cands = self.infer(e);
                if !cands.is_empty() {
                    self.resolve_free(e);
                }
            }
        }
        match uid {
            Some(uid) => {
                self.declare_unit(uid, &p.name, region);
            }
            None => {
                let sym = self.ident_sym(&p.name);
                let unit = self.ctx.unit.unwrap_or(UnitId::from_index(0));
                self.declare(
                    sym,
                    &p.name.name,
                    DeclKind::Unit { unit, region },
                    p.name.span,
                );
            }
        }
    }

    /// Resolves a name that must denote a package; returns its region.
    pub(crate) fn resolve_package_name(&mut self, n: &Name) -> Option<RegionId> {
        match self.classify(n, super::expr::Mode::Commit, None) {
            super::expr::Prefix::Region(d, r) => {
                self.a.set_ref(n.span(), d);
                Some(r)
            }
            super::expr::Prefix::Error => None,
            _ => {
                self.error(
                    "V0206",
                    n.span(),
                    format!("`{}` is not a package", self.text(n.span())),
                );
                None
            }
        }
    }

    fn configuration(&mut self, uid: UnitId, cfg: &ast::ConfigurationDecl) {
        let prev = self.enter(RegionKind::Configuration);
        let region = self.region;
        self.declare_unit(uid, &cfg.name, region);
        // The entity must exist in the same library.
        if let Name::Simple(i) = &cfg.entity {
            let sym = self.ident_sym(i);
            match self.find_unit(self.ctx.library, sym, LibraryUnitKind::Entity) {
                Some(eu) => {
                    if let Some(d) = self.a.units[eu.index()].decl {
                        self.a.set_ref(i.span, d);
                    }
                    if let Some(er) = self.a.units[eu.index()].region {
                        self.import_region(er);
                    }
                }
                None => self.error(
                    "V0102",
                    i.span,
                    format!("configuration of unknown entity `{}`", i.name),
                ),
            }
        }
        self.declarations(&cfg.decls);
        self.block_configuration(&cfg.block);
        self.leave(prev);
    }

    fn block_configuration(&mut self, b: &ast::BlockConfiguration) {
        for u in &b.uses {
            self.use_clause(u);
        }
        for item in &b.items {
            match item {
                ast::ConfigurationItem::Block(inner) => self.block_configuration(inner),
                ast::ConfigurationItem::Component(cc) => {
                    self.component_specification(&cc.spec);
                    if let Some(bi) = &cc.binding {
                        self.binding_indication(bi);
                    }
                    if let Some(inner) = &cc.block {
                        self.block_configuration(inner);
                    }
                }
            }
        }
    }

    pub(crate) fn component_specification(&mut self, spec: &ast::ComponentSpecification) {
        // Only the component name is checked: instance labels live in
        // architectures that a configuration refers to by name.
        if let Name::Simple(i) = &spec.component {
            let sym = self.ident_sym(i);
            let l = scope::lookup(self.a, self.region, sym);
            if let Some(d) = l.single() {
                self.a.set_ref(i.span, d);
            }
        }
    }

    pub(crate) fn binding_indication(&mut self, bi: &ast::BindingIndication) {
        if let Some(ast::EntityAspect::Entity { name, .. }) = &bi.entity_aspect {
            let _ = self.resolve_entity_name(name);
        }
    }

    /// Resolves `[lib.]entity` to the entity's unit, reporting unknown
    /// libraries and entities.
    pub(crate) fn resolve_entity_name(&mut self, n: &Name) -> Option<UnitId> {
        let (lib, ident) = match n {
            Name::Selected { prefix, suffix, .. } => match (prefix.as_ref(), suffix) {
                (Name::Simple(l), Suffix::Designator(Designator::Ident(i))) => {
                    (Some(l.clone()), i.clone())
                }
                _ => {
                    self.error("V0102", n.span(), "expected `library.entity`");
                    return None;
                }
            },
            Name::Simple(i) => (None, i.clone()),
            _ => {
                self.error("V0102", n.span(), "expected `library.entity`");
                return None;
            }
        };
        let lib_sym = match &lib {
            Some(l) => {
                let s = self.ident_sym(l);
                let lk = scope::lookup(self.a, self.region, s);
                match lk.single().map(|d| &self.a.decl(d).kind) {
                    Some(DeclKind::Library) => {
                        self.a.set_ref(l.span, lk.single().unwrap());
                        s
                    }
                    _ => {
                        self.unknown_library(l);
                        return None;
                    }
                }
            }
            None => self.syms.work,
        };
        let sym = self.ident_sym(&ident);
        match self.find_unit(lib_sym, sym, LibraryUnitKind::Entity) {
            Some(u) => {
                if let Some(d) = self.a.units[u.index()].decl {
                    self.a.set_ref(ident.span, d);
                }
                Some(u)
            }
            None => {
                let libname = if lib_sym == self.syms.work {
                    self.a.name(self.ctx.library).to_owned()
                } else {
                    self.a.name(lib_sym).to_owned()
                };
                let known: Vec<String> = self
                    .a
                    .units
                    .iter()
                    .filter(|u| {
                        u.kind == LibraryUnitKind::Entity && self.a.name(u.library) == libname
                    })
                    .map(|u| self.a.name(u.name).to_owned())
                    .collect();
                let mut d =
                    Diagnostic::error(format!("no entity `{}` in library `{libname}`", ident.name))
                        .with_code("V0102")
                        .with_span(ident.span);
                if let Some(s) = super::suggest(&ident.name, known.iter().map(String::as_str)) {
                    d = d.with_note(format!("did you mean `{s}`?"));
                }
                self.push(d);
                None
            }
        }
    }

    fn unknown_library(&mut self, l: &ast::Ident) {
        let known: Vec<String> = self
            .a
            .libraries()
            .iter()
            .map(|(s, _)| self.a.name(*s).to_owned())
            .collect();
        let mut d = Diagnostic::error(format!("unknown library `{}`", l.name))
            .with_code("V0101")
            .with_span(l.span);
        if let Some(s) = super::suggest(&l.name, known.iter().map(String::as_str)) {
            d = d.with_note(format!("did you mean `{s}`?"));
        } else if known.iter().any(|k| k.eq_ignore_ascii_case(&l.name)) {
            d = d.with_note(format!(
                "add `library {};` before the design unit",
                l.name.to_lowercase()
            ));
        }
        self.push(d);
    }

    // --- context clauses ---------------------------------------------------

    pub(crate) fn context_item(&mut self, item: &ContextItem) {
        match item {
            ContextItem::Library(lc) => {
                for name in &lc.names {
                    let sym = self.ident_sym(name);
                    let known = self
                        .a
                        .libraries()
                        .iter()
                        .find(|(s, _)| *s == sym)
                        .map(|(_, d)| *d);
                    match known {
                        Some(d) => {
                            self.a.set_ref(name.span, d);
                            // Make it visible in the context region even if the
                            // root already exposes it (harmless).
                            let region = self.region;
                            self.a.add_use(region, sym, d);
                        }
                        None => {
                            let d = Diagnostic::error(format!("unknown library `{}`", name.name))
                                .with_code("V0101")
                                .with_span(name.span)
                                .with_note("libraries: `std`, `ieee`, `work` and any library a source was compiled into");
                            self.push(d);
                        }
                    }
                }
            }
            ContextItem::Use(uc) => self.use_clause(uc),
            ContextItem::Context(cr) => {
                for n in &cr.names {
                    self.context_reference(n);
                }
            }
        }
    }

    fn context_reference(&mut self, n: &Name) {
        let Name::Selected { prefix, suffix, .. } = n else {
            self.error("V0102", n.span(), "expected `library.context_name`");
            return;
        };
        let (Name::Simple(lib), Suffix::Designator(Designator::Ident(cname))) =
            (prefix.as_ref(), suffix)
        else {
            self.error("V0102", n.span(), "expected `library.context_name`");
            return;
        };
        let lib_sym = self.ident_sym(lib);
        if !self.a.libraries().iter().any(|(s, _)| *s == lib_sym) {
            self.unknown_library(lib);
            return;
        }
        let csym = self.ident_sym(cname);
        let Some(cu) = self.find_unit(lib_sym, csym, LibraryUnitKind::Context) else {
            self.error(
                "V0102",
                cname.span,
                format!(
                    "no context declaration `{}` in library `{}`",
                    cname.name, lib.name
                ),
            );
            return;
        };
        if let Some(d) = self.a.units[cu.index()].decl {
            self.a.set_ref(cname.span, d);
        }
        let items = self.context_decls.get(&cu).cloned().unwrap_or_default();
        for item in &items {
            self.context_item(item);
        }
    }

    /// Makes every declaration of `from` potentially visible in the
    /// current region.
    pub(crate) fn use_all(&mut self, from: RegionId) {
        let decls: Vec<DeclId> = self.a.region(from).decls.clone();
        let region = self.region;
        for d in decls {
            let name = self.a.decl(d).name;
            self.a.add_use(region, name, d);
        }
    }

    pub(crate) fn use_clause(&mut self, uc: &ast::UseClause) {
        for n in &uc.names {
            self.use_name(n);
        }
    }

    fn use_name(&mut self, n: &Name) {
        let Name::Selected { prefix, suffix, .. } = n else {
            self.error(
                "V0102",
                n.span(),
                "a use clause needs a selected name such as `ieee.std_logic_1164.all`",
            );
            return;
        };
        // Resolve the prefix to a region (library or package).
        let region = match self.classify(prefix, super::expr::Mode::Commit, None) {
            super::expr::Prefix::Region(d, r) => {
                self.a.set_ref(prefix.span(), d);
                r
            }
            super::expr::Prefix::Error => return,
            super::expr::Prefix::Type(t)
                if matches!(self.a.ty(self.a.base_type(t)).kind, TypeKind::Enum { .. }) =>
            {
                // VHDL-2008: `use work.pkg.enum_type.all` or a literal.
                let base = self.a.base_type(t);
                let TypeKind::Enum { literals, .. } = self.a.ty(base).kind.clone() else {
                    return;
                };
                let region = self.region;
                match suffix {
                    Suffix::All(_) => {
                        for l in literals {
                            let name = self.a.decl(l).name;
                            self.a.add_use(region, name, l);
                        }
                    }
                    Suffix::Designator(d) => {
                        let (sym, _) = self.designator_sym(d);
                        for l in literals {
                            if self.a.decl(l).name == sym {
                                self.a.add_use(region, sym, l);
                            }
                        }
                    }
                }
                return;
            }
            _ => {
                self.error(
                    "V0206",
                    prefix.span(),
                    format!("`{}` is not a library or package", self.text(prefix.span())),
                );
                return;
            }
        };
        match suffix {
            Suffix::All(_) => self.use_all(region),
            Suffix::Designator(d) => {
                let (sym, spelling) = self.designator_sym(d);
                let found: Vec<DeclId> = self.a.region(region).direct(sym).to_vec();
                if found.is_empty() {
                    let names = self.a.region(region).names();
                    let names: Vec<String> =
                        names.iter().map(|s| self.a.name(*s).to_owned()).collect();
                    let mut diag = Diagnostic::error(format!(
                        "`{spelling}` is not declared in `{}`",
                        self.text(prefix.span())
                    ))
                    .with_code("V0200")
                    .with_span(d.span());
                    if let Some(s) = super::suggest(&spelling, names.iter().map(String::as_str)) {
                        diag = diag.with_note(format!("did you mean `{s}`?"));
                    }
                    self.push(diag);
                    return;
                }
                let cur = self.region;
                for f in found {
                    self.a.add_use(cur, sym, f);
                }
                self.a.set_ref(
                    d.span(),
                    *self.a.region(region).direct(sym).first().unwrap(),
                );
            }
        }
    }

    // --- interface lists ---------------------------------------------------

    /// Declares generics, ports or parameters in the current region.
    /// Returns the declarations in order.
    pub(crate) fn interface_list(
        &mut self,
        list: &[InterfaceDecl],
        role: ObjectRole,
    ) -> Vec<DeclId> {
        let mut out = Vec::new();
        for item in list {
            match item {
                InterfaceDecl::Object(o) => {
                    let (class, mode) = self.interface_class(o, role);
                    let ty = self.resolve_subtype_indication(&o.subtype);
                    if role == ObjectRole::Port {
                        let c = self.a.class(ty);
                        if matches!(
                            c,
                            super::TypeClass::File
                                | super::TypeClass::Access
                                | super::TypeClass::Protected
                        ) {
                            self.error(
                                "V0203",
                                o.subtype.span,
                                format!(
                                    "a port cannot be of {} type `{}`",
                                    class_word(c),
                                    self.ty_name(ty)
                                ),
                            );
                        }
                    }
                    if let Some(def) = &o.default {
                        self.resolve(def, ty);
                        self.check_static_range(def, ty);
                    }
                    for name in &o.names {
                        let sym = self.ident_sym(name);
                        let d = self.declare(
                            sym,
                            &name.name,
                            DeclKind::Object {
                                class,
                                ty,
                                mode: Some(mode),
                                role,
                                deferred: false,
                            },
                            name.span,
                        );
                        self.a.set_type(name.span, ty);
                        // A default on an interface object is recorded at
                        // the declared name's span, so that an association
                        // list can tell that the formal may be left out
                        // (clause 6.5.2) without the value being the
                        // object's own.
                        if let Some(def) = &o.default {
                            self.a.set_defaulted(d);
                            if let Some(v) = self.a.value_of(def.span()).cloned() {
                                self.a.set_value(name.span, v);
                            }
                        }
                        out.push(d);
                    }
                }
                InterfaceDecl::Type(t) => {
                    self.require_2008(t.span, "generic types");
                    let sym = self.ident_sym(&t.name);
                    let ty = self.a.add_type(TypeKind::Generic, Some(sym));
                    let d = self.declare(sym, &t.name.name, DeclKind::Type(ty), t.name.span);
                    self.a.type_mut(ty).decl = Some(d);
                    out.push(d);
                }
                InterfaceDecl::Subprogram(s) => {
                    self.require_2008(s.span, "generic subprograms");
                    let d = self.subprogram_decl(&s.spec, None);
                    if let Some(ast::SubprogramDefault::Name(n)) = &s.default {
                        let _ = self.classify(n, super::expr::Mode::Commit, None);
                    }
                    out.push(d);
                }
                InterfaceDecl::Package(p) => {
                    self.require_2008(p.span, "generic packages");
                    let region = self
                        .resolve_package_name(&p.uninstantiated)
                        .unwrap_or(self.region);
                    let sym = self.ident_sym(&p.name);
                    let unit = self.ctx.unit.unwrap_or(UnitId::from_index(0));
                    let d = self.declare(
                        sym,
                        &p.name.name,
                        DeclKind::Unit { unit, region },
                        p.name.span,
                    );
                    out.push(d);
                }
            }
        }
        out
    }

    /// The object class and mode of an interface object, with the
    /// defaults of clause 6.5.2 applied.
    fn interface_class(
        &mut self,
        o: &ast::InterfaceObject,
        role: ObjectRole,
    ) -> (ObjectClass, Mode) {
        let mode = o.mode.unwrap_or(Mode::In);
        let class = match o.class {
            Some(ast::ObjectClass::Constant) => ObjectClass::Constant,
            Some(ast::ObjectClass::Signal) => ObjectClass::Signal,
            Some(ast::ObjectClass::Variable) => ObjectClass::Variable,
            Some(ast::ObjectClass::File) => ObjectClass::File,
            None => match role {
                ObjectRole::Port => ObjectClass::Signal,
                ObjectRole::Generic => ObjectClass::Constant,
                _ => {
                    if mode == Mode::In {
                        ObjectClass::Constant
                    } else {
                        ObjectClass::Variable
                    }
                }
            },
        };
        if role == ObjectRole::Generic && class != ObjectClass::Constant {
            self.error("V0203", o.span, "a generic must be a constant");
        }
        if role == ObjectRole::Port && class != ObjectClass::Signal {
            self.error("V0203", o.span, "a port must be a signal");
        }
        if role == ObjectRole::Parameter && class == ObjectClass::Constant && mode != Mode::In {
            self.error("V0203", o.span, "a constant parameter must be of mode `in`");
        }
        if role == ObjectRole::Port && mode == Mode::Linkage {
            self.warn(
                "V0203",
                o.span,
                "`linkage` ports cannot be read or assigned",
            );
        }
        (class, mode)
    }

    // --- declarations ------------------------------------------------------

    pub(crate) fn declarations(&mut self, decls: &[Declaration]) {
        for d in decls {
            self.declaration(d);
        }
    }

    pub(crate) fn declaration(&mut self, d: &Declaration) {
        match d {
            Declaration::Object(o) => self.object_decl(o),
            Declaration::File(f) => self.file_decl(f),
            Declaration::Type(t) => self.type_decl(t),
            Declaration::Subtype(s) => self.subtype_decl(s),
            Declaration::Alias(al) => self.alias_decl(al),
            Declaration::Attribute(at) => {
                let ty = self.resolve_type_mark(&at.type_mark);
                let sym = self.ident_sym(&at.name);
                self.declare(sym, &at.name.name, DeclKind::Attribute(ty), at.name.span);
            }
            Declaration::AttributeSpec(spec) => self.attribute_spec(spec),
            Declaration::Component(c) => self.component_decl(c),
            Declaration::Subprogram(s) => {
                self.subprogram_decl(&s.spec, None);
            }
            Declaration::SubprogramBody(b) => self.subprogram_body(b),
            Declaration::SubprogramInstantiation(si) => {
                self.require_2008(si.span, "subprogram instantiations");
                let target = self.classify(&si.uninstantiated, super::expr::Mode::Commit, None);
                let (sym, spelling) = self.designator_sym(&si.designator);
                if let super::expr::Prefix::Overloaded(ds) = target
                    && let Some(&d0) = ds.first()
                    && let DeclKind::Subprogram { sig, .. } = self.a.decl(d0).kind.clone()
                {
                    self.a.set_ref(si.uninstantiated.span(), d0);
                    self.declare(
                        sym,
                        &spelling,
                        DeclKind::Subprogram {
                            sig,
                            body: SubprogramBody::Implicit,
                        },
                        si.designator.span(),
                    );
                } else {
                    self.error(
                        "V0204",
                        si.uninstantiated.span(),
                        "expected the name of an uninstantiated subprogram",
                    );
                }
            }
            Declaration::Package(p) => {
                self.require_2008(p.span, "nested packages");
                let prev = self.enter(RegionKind::Package);
                let region = self.region;
                let sym = self.ident_sym(&p.name);
                let unit = self.ctx.unit.unwrap_or(UnitId::from_index(0));
                // Declare the package in the enclosing region.
                self.region = prev;
                let d = self.declare(
                    sym,
                    &p.name.name,
                    DeclKind::Unit { unit, region },
                    p.name.span,
                );
                let _ = d;
                self.region = region;
                let was = std::mem::replace(&mut self.ctx.in_package, true);
                self.interface_list(&p.generics, ObjectRole::Generic);
                self.declarations(&p.decls);
                self.ctx.in_package = was;
                self.leave(prev);
            }
            Declaration::PackageBody(b) => {
                self.require_2008(b.span, "nested package bodies");
                let sym = self.ident_sym(&b.name);
                let found = self.a.region(self.region).direct(sym).first().copied();
                let Some(pd) = found else {
                    self.error(
                        "V0104",
                        b.name.span,
                        format!("package body for unknown package `{}`", b.name.name),
                    );
                    return;
                };
                let DeclKind::Unit {
                    region: pregion, ..
                } = self.a.decl(pd).kind
                else {
                    self.error(
                        "V0104",
                        b.name.span,
                        format!("`{}` is not a package", b.name.name),
                    );
                    return;
                };
                self.a.set_ref(b.name.span, pd);
                let prev = self.enter(RegionKind::PackageBody);
                self.import_region(pregion);
                self.declarations(&b.decls);
                self.check_package_complete(pregion, b.name.span);
                self.leave(prev);
            }
            Declaration::PackageInstantiation(p) => {
                self.require_2008(p.span, "package instantiations");
                self.package_instantiation(None, p);
            }
            Declaration::Use(u) => self.use_clause(u),
            Declaration::GroupTemplate(g) => {
                let sym = self.ident_sym(&g.name);
                self.declare(sym, &g.name.name, DeclKind::Group, g.name.span);
            }
            Declaration::Group(g) => {
                let sym = self.ident_sym(&g.name);
                self.declare(sym, &g.name.name, DeclKind::Group, g.name.span);
            }
            Declaration::Disconnection(ds) => {
                let ty = self.resolve_type_mark(&ds.type_mark);
                if let ast::SignalList::Names(names) = &ds.signals {
                    for n in names {
                        let v = self.commit_name(n, Some(ty));
                        if let Some(obj) = v.obj
                            && obj.class != ObjectClass::Signal
                        {
                            self.error(
                                "V0204",
                                n.span(),
                                "a disconnection specification names guarded signals",
                            );
                        }
                    }
                }
                let time = self.a.builtins.time;
                self.resolve(&ds.time, time);
            }
            Declaration::ConfigurationSpec(cs) => {
                self.component_specification(&cs.spec);
                self.binding_indication(&cs.binding);
            }
        }
    }

    fn object_decl(&mut self, o: &ast::ObjectDecl) {
        let ty = self.resolve_subtype_indication(&o.subtype);
        let region_kind = self.a.region(self.region).kind;
        let class = match o.kind {
            ObjectKind::Constant => ObjectClass::Constant,
            ObjectKind::Signal => ObjectClass::Signal,
            ObjectKind::Variable => ObjectClass::Variable,
            ObjectKind::SharedVariable => ObjectClass::SharedVariable,
        };
        // Placement rules (clause 6.4.2.3, 6.4.2.4).
        match class {
            ObjectClass::Signal => {
                if matches!(region_kind, RegionKind::Process | RegionKind::Subprogram) {
                    self.error(
                        "V0203",
                        o.span,
                        format!(
                            "a signal cannot be declared in a {}; declare it in the architecture",
                            region_word(region_kind)
                        ),
                    );
                }
            }
            ObjectClass::Variable => {
                if matches!(
                    region_kind,
                    RegionKind::Architecture
                        | RegionKind::Entity
                        | RegionKind::Package
                        | RegionKind::PackageBody
                        | RegionKind::Block
                        | RegionKind::Generate
                ) {
                    self.error(
                        "V0203",
                        o.span,
                        format!("a variable cannot be declared in {} {}; use a signal, or `shared variable`", article(region_kind), region_word(region_kind)),
                    );
                }
            }
            ObjectClass::SharedVariable => {
                if matches!(region_kind, RegionKind::Process | RegionKind::Subprogram) {
                    self.error("V0203", o.span, "a shared variable cannot be declared here");
                }
            }
            _ => {}
        }
        if class == ObjectClass::Signal
            || class == ObjectClass::Variable
            || class == ObjectClass::SharedVariable
        {
            let c = self.a.class(ty);
            if c == super::TypeClass::File {
                self.error(
                    "V0203",
                    o.subtype.span,
                    "an object of a file type must be declared with `file`",
                );
            }
            if class == ObjectClass::Signal && c == super::TypeClass::Access {
                self.error(
                    "V0203",
                    o.subtype.span,
                    "a signal cannot be of an access type",
                );
            }
        }
        let deferred = o.init.is_none() && class == ObjectClass::Constant;
        if deferred && !self.ctx.in_package {
            self.error(
                "V0203",
                o.span,
                "a constant declared outside a package must have a value",
            );
        }
        let init_value = if let Some(init) = &o.init {
            self.resolve(init, ty);
            self.check_static_range(init, ty);
            self.a.value_of(init.span()).cloned()
        } else {
            None
        };
        for name in &o.names {
            let sym = self.ident_sym(name);
            // A deferred constant's full declaration in the package body
            // completes the package declaration rather than redeclaring.
            if class == ObjectClass::Constant
                && !deferred
                && self.a.region(self.region).kind == RegionKind::PackageBody
                && let Some(prev) = self.a.region(self.region).direct(sym).first().copied()
                && let DeclKind::Object {
                    deferred: true,
                    ty: pty,
                    ..
                } = self.a.decl(prev).kind
            {
                if !self.a.same_base(pty, ty) {
                    let (pn, tn) = (self.ty_name(pty), self.ty_name(ty));
                    let pspan = self.a.decl(prev).span;
                    self.push(
                        Diagnostic::error(format!("full declaration of `{}` has type `{tn}` but the deferred constant is `{pn}`", name.name))
                            .with_code("V0300")
                            .with_label(name.span, format!("declared here as `{tn}`"))
                            .with_secondary(pspan, format!("deferred constant of type `{pn}`")),
                    );
                }
                if let DeclKind::Object { deferred, .. } = &mut self.a.decl_mut(prev).kind {
                    *deferred = false;
                }
                self.a.set_ref(name.span, prev);
                if let Some(v) = &init_value {
                    self.a.set_decl_value(prev, v.clone());
                }
                continue;
            }
            let d = self.declare(
                sym,
                &name.name,
                DeclKind::Object {
                    class,
                    ty,
                    mode: None,
                    role: ObjectRole::Plain,
                    deferred,
                },
                name.span,
            );
            self.a.set_type(name.span, ty);
            if let Some(v) = &init_value {
                self.a.set_decl_value(d, v.clone());
            }
        }
    }

    /// After resolving `e` against `ty`: if both the value and the
    /// subtype's range are static, check the value lies inside.
    pub(crate) fn check_static_range(&mut self, e: &ast::Expr, ty: TypeId) {
        let Some(v) = self.a.value_of(e.span()).cloned() else {
            return;
        };
        if !self.a.is_scalar(ty) {
            // Array length check.
            if let (Some(len), Value::Array(av)) = (self.a.array_length(ty), &v)
                && i128::try_from(av.elems.len()).ok() != Some(len)
            {
                let tn = self.ty_name(ty);
                self.push(
                    Diagnostic::error(format!(
                        "value has {} elements but `{tn}` has {len}",
                        av.elems.len()
                    ))
                    .with_code("V0307")
                    .with_label(e.span(), format!("{} elements", av.elems.len())),
                );
            }
            return;
        }
        let Some(range) = self.a.scalar_range(ty) else {
            return;
        };
        let inside = match &v {
            Value::Real(r) => range.contains_real(*r),
            other => other.as_int().and_then(|i| range.contains_int(i)),
        };
        if inside == Some(false) {
            let tn = self.ty_name(ty);
            let shown = self.a.describe_value(&v, ty);
            let lo = range
                .left
                .value()
                .map(|b| self.a.describe_value(b, ty))
                .unwrap_or_default();
            let hi = range
                .right
                .value()
                .map(|b| self.a.describe_value(b, ty))
                .unwrap_or_default();
            self.push(
                Diagnostic::error(format!("value {shown} is out of range for `{tn}`"))
                    .with_code("V0308")
                    .with_label(
                        e.span(),
                        format!("`{tn}` is {lo} {} {hi}", range.dir.as_str()),
                    ),
            );
        }
    }

    fn file_decl(&mut self, f: &ast::FileDecl) {
        let ty = self.resolve_subtype_indication(&f.subtype);
        if self.a.class(ty) != super::TypeClass::File && !self.a.is_error(ty) {
            self.error(
                "V0203",
                f.subtype.span,
                format!("`{}` is not a file type", self.ty_name(ty)),
            );
        }
        if let Some(k) = &f.open_kind {
            let fok = self.a.builtins.file_open_kind;
            self.resolve(k, fok);
        }
        if let Some(n) = &f.logical_name {
            let s = self.a.builtins.string;
            self.resolve(n, s);
        }
        for name in &f.names {
            let sym = self.ident_sym(name);
            self.declare(
                sym,
                &name.name,
                DeclKind::Object {
                    class: ObjectClass::File,
                    ty,
                    mode: None,
                    role: ObjectRole::Plain,
                    deferred: false,
                },
                name.span,
            );
            self.a.set_type(name.span, ty);
        }
    }

    fn type_decl(&mut self, t: &ast::TypeDecl) {
        let sym = self.ident_sym(&t.name);
        // An incomplete declaration seen earlier in this region is
        // completed in place, keeping its `TypeId`.
        let incomplete = self
            .a
            .region(self.region)
            .direct(sym)
            .iter()
            .find_map(|&d| match self.a.decl(d).kind {
                DeclKind::Type(ty) if matches!(self.a.ty(ty).kind, TypeKind::Incomplete) => {
                    Some((d, ty))
                }
                _ => None,
            });
        let Some(def) = &t.def else {
            if incomplete.is_some() {
                self.error(
                    "V0202",
                    t.name.span,
                    format!(
                        "`{}` is already declared as an incomplete type",
                        t.name.name
                    ),
                );
                return;
            }
            let ty = self.a.add_type(TypeKind::Incomplete, Some(sym));
            let d = self.declare(sym, &t.name.name, DeclKind::Type(ty), t.name.span);
            self.a.type_mut(ty).decl = Some(d);
            return;
        };
        if let TypeDef::ProtectedBody(pb) = def {
            self.protected_body(t, pb);
            return;
        }
        // Allocate (or reuse) the type id before analysing the definition
        // so that access types and records can refer to it.
        let ty = match incomplete {
            Some((d, ty)) => {
                self.a.set_ref(t.name.span, d);
                ty
            }
            None => {
                let ty = self.a.add_type(TypeKind::Incomplete, Some(sym));
                let d = self.declare(sym, &t.name.name, DeclKind::Type(ty), t.name.span);
                self.a.type_mut(ty).decl = Some(d);
                ty
            }
        };
        self.a.set_type(t.name.span, ty);
        let kind = match def {
            TypeDef::Enumeration(lits) => {
                let mut literals = Vec::new();
                let mut character = false;
                for (pos, lit) in lits.iter().enumerate() {
                    let (lsym, spelling) = self.designator_sym(lit);
                    if matches!(lit, Designator::Char { .. }) {
                        character = true;
                    }
                    let pos = u32::try_from(pos).expect("enum literal count");
                    let d = self.declare(
                        lsym,
                        &spelling,
                        DeclKind::EnumLiteral { ty, pos },
                        lit.span(),
                    );
                    literals.push(d);
                }
                TypeKind::Enum {
                    literals,
                    character,
                }
            }
            TypeDef::Range(r) => self.scalar_type_def(r),
            TypeDef::Physical(p) => {
                let range = self.resolve_range(
                    &ast::Range::Bounds {
                        left: match &p.range {
                            ast::Range::Bounds { left, .. } => left.clone(),
                            ast::Range::Attribute(n) => ast::Expr::Name(n.clone()),
                        },
                        direction: match &p.range {
                            ast::Range::Bounds { direction, .. } => *direction,
                            _ => Direction::To,
                        },
                        right: match &p.range {
                            ast::Range::Bounds { right, .. } => right.clone(),
                            ast::Range::Attribute(n) => ast::Expr::Name(n.clone()),
                        },
                        span: p.range.span(),
                    },
                    Some(self.a.builtins.universal_integer),
                );
                let psym = self.ident_sym(&p.primary_unit);
                let mut units = Vec::new();
                let d = self.declare(
                    psym,
                    &p.primary_unit.name,
                    DeclKind::PhysicalUnit { ty, scale: 1 },
                    p.primary_unit.span,
                );
                units.push(d);
                for su in &p.secondary_units {
                    let scale = self.secondary_unit_scale(&su.value, ty).unwrap_or(1);
                    let ssym = self.ident_sym(&su.name);
                    let d = self.declare(
                        ssym,
                        &su.name.name,
                        DeclKind::PhysicalUnit { ty, scale },
                        su.name.span,
                    );
                    units.push(d);
                }
                TypeKind::Physical {
                    range: range.bounds,
                    units,
                }
            }
            TypeDef::Array(arr) => {
                let mut indices = Vec::new();
                let mut constraint = Vec::new();
                let mut constrained = false;
                for idx in &arr.indices {
                    match idx {
                        ast::ArrayIndex::Unbounded(tm) => {
                            let it = self.resolve_type_mark(tm);
                            if !self.a.is_discrete(it) && !self.a.is_error(it) {
                                self.error(
                                    "V0203",
                                    tm.span(),
                                    format!(
                                        "array index type `{}` is not discrete",
                                        self.ty_name(it)
                                    ),
                                );
                            }
                            indices.push(it);
                        }
                        ast::ArrayIndex::Constrained(dr) => {
                            constrained = true;
                            let info = self.resolve_discrete_range(dr);
                            indices.push(info.ty);
                            constraint.push(info.bounds);
                        }
                    }
                }
                let element = self.resolve_subtype_indication(&arr.element);
                if constrained {
                    // Clause 5.3.2.1: anonymous base type + named subtype.
                    let base = self.a.add_type(TypeKind::Array { indices, element }, None);
                    TypeKind::Subtype {
                        parent: base,
                        constraint: Some(Constraint::Index(constraint, None)),
                        resolution: None,
                    }
                } else {
                    TypeKind::Array { indices, element }
                }
            }
            TypeDef::Record(rec) => {
                let mut fields: Vec<super::Field> = Vec::new();
                for el in &rec.elements {
                    let fty = self.resolve_subtype_indication(&el.subtype);
                    for name in &el.names {
                        let fsym = self.ident_sym(name);
                        if fields.iter().any(|f| f.name == fsym) {
                            self.error(
                                "V0202",
                                name.span,
                                format!("duplicate record element `{}`", name.name),
                            );
                            continue;
                        }
                        fields.push(super::Field {
                            name: fsym,
                            ty: fty,
                            span: name.span,
                        });
                        self.a.set_type(name.span, fty);
                    }
                }
                TypeKind::Record(fields)
            }
            TypeDef::Access(si) => {
                let designated = self.resolve_subtype_indication(si);
                TypeKind::Access(designated)
            }
            TypeDef::File(tm) => {
                let elem = self.resolve_type_mark(tm);
                TypeKind::File(elem)
            }
            TypeDef::Protected(pt) => {
                self.require_2008(pt.span, "protected types");
                let prev = self.enter(RegionKind::Protected);
                let region = self.region;
                // Complete the type before analysing methods so that
                // they can refer to it.
                self.a.type_mut(ty).kind = TypeKind::Protected(region);
                self.declarations(&pt.decls);
                self.leave(prev);
                TypeKind::Protected(region)
            }
            TypeDef::ProtectedBody(_) => unreachable!(),
        };
        self.a.type_mut(ty).kind = kind;
        // Implicit declarations that come with the type (clause 5.4.3,
        // 5.5.3).
        match &self.a.ty(ty).kind {
            TypeKind::File(elem) => {
                let elem = *elem;
                self.implicit_file_ops(ty, elem);
            }
            TypeKind::Access(_) => {
                self.implicit_deallocate(ty);
            }
            _ => {}
        }
    }

    /// `range l to r`: an integer type when the bounds are integer, a
    /// floating type when they are real (clause 5.2.3, 5.2.5).
    fn scalar_type_def(&mut self, r: &ast::Range) -> TypeKind {
        let (left, right, dir, span) = match r {
            ast::Range::Bounds {
                left,
                right,
                direction,
                span,
            } => (left, right, *direction, *span),
            ast::Range::Attribute(_) => {
                let info = self.resolve_range(r, None);
                return if self.a.class(info.ty).is_real() {
                    TypeKind::Real(info.bounds)
                } else {
                    TypeKind::Integer(info.bounds)
                };
            }
        };
        let lc = self.infer(left);
        let rc = self.infer(right);
        let is_real = |a: &Analysis, c: &[Cand]| {
            c.iter()
                .any(|c| matches!(c, Cand::Ty(t) if a.class(*t).is_real()))
        };
        let real = is_real(self.a, &lc) || is_real(self.a, &rc);
        let ctx_ty = if real {
            self.a.builtins.universal_real
        } else {
            self.a.builtins.universal_integer
        };
        let lt = self.resolve(left, ctx_ty);
        let rt = self.resolve(right, ctx_ty);
        if !self.a.is_error(lt)
            && !self.a.is_error(rt)
            && self.a.class(lt).is_real() != self.a.class(rt).is_real()
        {
            self.error(
                "V0300",
                span,
                "the bounds of a range must both be integer or both be real",
            );
        }
        let lb = self.bound_of(left);
        let rb = self.bound_of(right);
        let bounds = Bounds {
            left: lb,
            dir,
            right: rb,
        };
        if real {
            TypeKind::Real(bounds)
        } else {
            TypeKind::Integer(bounds)
        }
    }

    /// The bound of an already resolved expression: its static value, or
    /// its span when not static.
    pub(crate) fn bound_of(&self, e: &ast::Expr) -> Bound {
        match self.a.value_of(e.span()) {
            Some(v) => Bound::Static(v.clone()),
            None => Bound::Dynamic(e.span()),
        }
    }

    /// The scale of a secondary unit: `n primary_or_secondary_unit`.
    fn secondary_unit_scale(&mut self, value: &ast::Expr, ty: TypeId) -> Option<i128> {
        match value {
            ast::Expr::Literal(ast::Literal {
                kind: ast::LiteralKind::Physical { value, unit },
                ..
            }) => {
                let n = parse_integer_literal(value)
                    .or_else(|| parse_real_literal(value).and_then(round_to_i128));
                let usym = self.ident_sym(unit);
                let found = self.a.region(self.region).direct(usym).first().copied();
                match found.map(|d| (d, self.a.decl(d).kind.clone())) {
                    Some((d, DeclKind::PhysicalUnit { scale, ty: uty })) if uty == ty => {
                        self.a.set_ref(unit.span, d);
                        n.and_then(|n| n.checked_mul(scale))
                    }
                    _ => {
                        self.error(
                            "V0200",
                            unit.span,
                            format!("unknown unit `{}` in this physical type", unit.name),
                        );
                        None
                    }
                }
            }
            ast::Expr::Name(Name::Simple(unit)) => {
                let usym = self.ident_sym(unit);
                let found = self.a.region(self.region).direct(usym).first().copied();
                match found.map(|d| (d, self.a.decl(d).kind.clone())) {
                    Some((d, DeclKind::PhysicalUnit { scale, .. })) => {
                        self.a.set_ref(unit.span, d);
                        Some(scale)
                    }
                    _ => {
                        self.error("V0200", unit.span, format!("unknown unit `{}`", unit.name));
                        None
                    }
                }
            }
            other => {
                self.error(
                    "V0203",
                    other.span(),
                    "a secondary unit is declared as `name = n unit;`",
                );
                None
            }
        }
    }

    /// The implicit `file_open`, `file_close`, `read`, `write`, `flush`
    /// and `endfile` of a file type (clause 5.5.2).
    fn implicit_file_ops(&mut self, file_ty: TypeId, elem: TypeId) {
        let b = self.a.builtins;
        let region = self.region;
        let mk = |c: &mut Checker,
                  name: Symbol,
                  spelling: &str,
                  kind: SubprogramKind,
                  params: Vec<(&str, TypeId, Mode, ObjectClass, bool)>,
                  ret: Option<TypeId>| {
            let mut ps = Vec::new();
            let sub_region = c.a.add_region(RegionKind::Subprogram, Some(region));
            for (pn, pty, mode, class, dflt) in params {
                let psym = c.a.interner.intern_ci(pn);
                let span = c.a.decl(c.a.region(region).decls[0]).span;
                let pd = c.a.add_decl(
                    sub_region,
                    psym,
                    pn,
                    DeclKind::Object {
                        class,
                        ty: pty,
                        mode: Some(mode),
                        role: ObjectRole::Parameter,
                        deferred: false,
                    },
                    span,
                );
                ps.push(Param {
                    decl: pd,
                    ty: pty,
                    mode,
                    class,
                    has_default: dflt,
                });
            }
            let span = c.a.decl(c.a.region(region).decls[0]).span;
            c.a.add_decl(
                region,
                name,
                spelling,
                DeclKind::Subprogram {
                    sig: Signature {
                        kind,
                        params: ps,
                        ret,
                        pure: kind == SubprogramKind::Function,
                    },
                    body: SubprogramBody::Implicit,
                },
                span,
            );
        };
        let s = &self.syms;
        let (file_open, file_close, read, write, flush, endfile) = (
            s.file_open,
            s.file_close,
            s.read,
            s.write,
            s.flush,
            s.endfile,
        );
        mk(
            self,
            file_open,
            "file_open",
            SubprogramKind::Procedure,
            vec![
                ("f", file_ty, Mode::Inout, ObjectClass::File, false),
                (
                    "external_name",
                    b.string,
                    Mode::In,
                    ObjectClass::Constant,
                    false,
                ),
                (
                    "open_kind",
                    b.file_open_kind,
                    Mode::In,
                    ObjectClass::Constant,
                    true,
                ),
            ],
            None,
        );
        mk(
            self,
            file_open,
            "file_open",
            SubprogramKind::Procedure,
            vec![
                (
                    "status",
                    b.file_open_status,
                    Mode::Out,
                    ObjectClass::Variable,
                    false,
                ),
                ("f", file_ty, Mode::Inout, ObjectClass::File, false),
                (
                    "external_name",
                    b.string,
                    Mode::In,
                    ObjectClass::Constant,
                    false,
                ),
                (
                    "open_kind",
                    b.file_open_kind,
                    Mode::In,
                    ObjectClass::Constant,
                    true,
                ),
            ],
            None,
        );
        mk(
            self,
            file_close,
            "file_close",
            SubprogramKind::Procedure,
            vec![("f", file_ty, Mode::Inout, ObjectClass::File, false)],
            None,
        );
        if self.a.class(elem) == super::TypeClass::Array && !self.a.is_constrained(elem) {
            mk(
                self,
                read,
                "read",
                SubprogramKind::Procedure,
                vec![
                    ("f", file_ty, Mode::Inout, ObjectClass::File, false),
                    ("value", elem, Mode::Out, ObjectClass::Variable, false),
                    ("length", b.natural, Mode::Out, ObjectClass::Variable, false),
                ],
                None,
            );
        } else {
            mk(
                self,
                read,
                "read",
                SubprogramKind::Procedure,
                vec![
                    ("f", file_ty, Mode::Inout, ObjectClass::File, false),
                    ("value", elem, Mode::Out, ObjectClass::Variable, false),
                ],
                None,
            );
        }
        mk(
            self,
            write,
            "write",
            SubprogramKind::Procedure,
            vec![
                ("f", file_ty, Mode::Inout, ObjectClass::File, false),
                ("value", elem, Mode::In, ObjectClass::Constant, false),
            ],
            None,
        );
        mk(
            self,
            flush,
            "flush",
            SubprogramKind::Procedure,
            vec![("f", file_ty, Mode::Inout, ObjectClass::File, false)],
            None,
        );
        mk(
            self,
            endfile,
            "endfile",
            SubprogramKind::Function,
            vec![("f", file_ty, Mode::In, ObjectClass::File, false)],
            Some(b.boolean),
        );
    }

    /// The implicit `deallocate` of an access type (clause 5.4.3).
    fn implicit_deallocate(&mut self, access_ty: TypeId) {
        let region = self.region;
        let span = self
            .a
            .decl(*self.a.region(region).decls.last().expect("type declared"))
            .span;
        let sub_region = self.a.add_region(RegionKind::Subprogram, Some(region));
        let psym = self.a.interner.intern_ci("p");
        let pd = self.a.add_decl(
            sub_region,
            psym,
            "p",
            DeclKind::Object {
                class: ObjectClass::Variable,
                ty: access_ty,
                mode: Some(Mode::Inout),
                role: ObjectRole::Parameter,
                deferred: false,
            },
            span,
        );
        let name = self.syms.deallocate;
        self.a.add_decl(
            region,
            name,
            "deallocate",
            DeclKind::Subprogram {
                sig: Signature {
                    kind: SubprogramKind::Procedure,
                    params: vec![Param {
                        decl: pd,
                        ty: access_ty,
                        mode: Mode::Inout,
                        class: ObjectClass::Variable,
                        has_default: false,
                    }],
                    ret: None,
                    pure: false,
                },
                body: SubprogramBody::Implicit,
            },
            span,
        );
    }

    fn protected_body(&mut self, t: &ast::TypeDecl, pb: &ast::ProtectedTypeBody) {
        let sym = self.ident_sym(&t.name);
        let found = self
            .a
            .region(self.region)
            .direct(sym)
            .iter()
            .find_map(|&d| match self.a.decl(d).kind {
                DeclKind::Type(ty) => match self.a.ty(ty).kind {
                    TypeKind::Protected(r) => Some((d, r)),
                    _ => None,
                },
                _ => None,
            });
        let Some((d, pregion)) = found else {
            self.error(
                "V0202",
                t.name.span,
                format!(
                    "protected body for unknown protected type `{}`",
                    t.name.name
                ),
            );
            return;
        };
        self.a.set_ref(t.name.span, d);
        let prev = self.enter(RegionKind::Protected);
        self.import_region(pregion);
        self.declarations(&pb.decls);
        self.check_package_complete(pregion, t.name.span);
        self.leave(prev);
    }

    fn subtype_decl(&mut self, s: &ast::SubtypeDecl) {
        let sym = self.ident_sym(&s.name);
        let target = self.resolve_subtype_indication(&s.subtype);
        // Always a fresh named subtype entry, so `describe` shows the
        // declared name.
        let ty = match &self.a.ty(target).kind {
            // An anonymous subtype created by the indication gets the name.
            TypeKind::Subtype { .. } if self.a.ty(target).name.is_none() => {
                self.a.type_mut(target).name = Some(sym);
                target
            }
            _ => self.a.add_type(
                TypeKind::Subtype {
                    parent: target,
                    constraint: None,
                    resolution: None,
                },
                Some(sym),
            ),
        };
        let d = self.declare(sym, &s.name.name, DeclKind::Subtype(ty), s.name.span);
        self.a.type_mut(ty).decl = Some(d);
        self.a.set_type(s.name.span, ty);
    }

    fn alias_decl(&mut self, al: &ast::AliasDecl) {
        let (sym, spelling) = self.designator_sym(&al.designator);
        if let Some(sig) = &al.signature {
            // Non-object alias of a subprogram or enumeration literal.
            let target = self.classify(&al.target, super::expr::Mode::Commit, None);
            let param_tys: Vec<TypeId> = sig
                .params
                .iter()
                .map(|p| self.resolve_type_mark(p))
                .collect();
            let ret = sig.return_type.as_ref().map(|r| self.resolve_type_mark(r));
            if let super::expr::Prefix::Overloaded(ds) = target {
                let matched = ds.iter().copied().find(|&d| match &self.a.decl(d).kind {
                    DeclKind::Subprogram { sig, .. } => {
                        sig.params.len() == param_tys.len()
                            && sig
                                .params
                                .iter()
                                .zip(&param_tys)
                                .all(|(p, t)| self.a.same_base(p.ty, *t))
                            && match (sig.ret, ret) {
                                (None, None) => true,
                                (Some(a), Some(b)) => self.a.same_base(a, b),
                                _ => false,
                            }
                    }
                    DeclKind::EnumLiteral { ty, .. } => {
                        param_tys.is_empty() && ret.is_some_and(|r| self.a.same_base(r, *ty))
                    }
                    _ => false,
                });
                match matched {
                    Some(d) => {
                        self.a.set_ref(al.target.span(), d);
                        let kind = alias_kind(self.a.decl(d).kind.clone());
                        self.declare(sym, &spelling, kind, al.designator.span());
                    }
                    None => self.error(
                        "V0303",
                        sig.span,
                        "no visible subprogram or literal matches this signature",
                    ),
                }
            } else {
                self.error(
                    "V0204",
                    al.target.span(),
                    "an alias with a signature must name a subprogram or enumeration literal",
                );
            }
            return;
        }
        match self.classify(&al.target, super::expr::Mode::Commit, None) {
            super::expr::Prefix::Type(t) => {
                self.a.set_type(al.target.span(), t);
                self.declare(sym, &spelling, DeclKind::Subtype(t), al.designator.span());
            }
            super::expr::Prefix::Object(obj, oty) => {
                let ty = match &al.subtype {
                    Some(si) => {
                        let t = self.resolve_subtype_indication(si);
                        if !self.a.same_base(t, oty) && !self.a.is_error(t) && !self.a.is_error(oty)
                        {
                            let (tn, on) = (self.ty_name(t), self.ty_name(oty));
                            self.error("V0300", si.span, format!("alias subtype `{tn}` does not match the aliased object's type `{on}`"));
                        }
                        t
                    }
                    None => oty,
                };
                self.a.set_type(al.target.span(), oty);
                let d = self.declare(
                    sym,
                    &spelling,
                    DeclKind::Object {
                        class: obj.class,
                        ty,
                        mode: obj.mode,
                        role: ObjectRole::Alias,
                        deferred: false,
                    },
                    al.designator.span(),
                );
                // Remember the aliased object for the lowering pass.
                self.a.set_ref(al.target.span(), obj.decl);
                let _ = d;
            }
            super::expr::Prefix::Overloaded(ds) => {
                if ds.len() == 1 {
                    let kind = alias_kind(self.a.decl(ds[0]).kind.clone());
                    self.a.set_ref(al.target.span(), ds[0]);
                    self.declare(sym, &spelling, kind, al.designator.span());
                } else {
                    self.error(
                        "V0302",
                        al.target.span(),
                        "the aliased name is overloaded; add a signature to select one",
                    );
                }
            }
            super::expr::Prefix::Region(d, r) => {
                let unit = self.ctx.unit.unwrap_or(UnitId::from_index(0));
                let _ = d;
                self.declare(
                    sym,
                    &spelling,
                    DeclKind::Unit { unit, region: r },
                    al.designator.span(),
                );
            }
            super::expr::Prefix::Value(ty) => {
                // A value with no object (a function call): still an
                // object alias in spirit; treat as a constant.
                self.declare(
                    sym,
                    &spelling,
                    DeclKind::Object {
                        class: ObjectClass::Constant,
                        ty,
                        mode: None,
                        role: ObjectRole::Alias,
                        deferred: false,
                    },
                    al.designator.span(),
                );
            }
            super::expr::Prefix::Values(_)
            | super::expr::Prefix::Unit(_)
            | super::expr::Prefix::Error => {}
        }
    }

    fn attribute_spec(&mut self, spec: &ast::AttributeSpec) {
        let asym = self.ident_sym(&spec.attribute);
        // `foreign` is predefined in std.standard as an attribute of type
        // string (clause 20.2); user attributes must be declared.
        let attr_ty = if asym == self.syms.foreign {
            self.a.builtins.string
        } else {
            let l = scope::lookup(self.a, self.region, asym);
            match l.decls.iter().find_map(|&d| match self.a.decl(d).kind {
                DeclKind::Attribute(t) => Some((d, t)),
                _ => None,
            }) {
                Some((d, t)) => {
                    self.a.set_ref(spec.attribute.span, d);
                    t
                }
                None => {
                    self.error(
                        "V0200",
                        spec.attribute.span,
                        format!(
                            "unknown attribute `{}`; declare it with `attribute {} : type;`",
                            spec.attribute.name, spec.attribute.name
                        ),
                    );
                    self.a.builtins.error
                }
            }
        };
        self.resolve(&spec.value, attr_ty);
        let value = self.a.value_of(spec.value.span()).cloned();
        let ast::EntityNameList::Names(names) = &spec.entities else {
            return;
        };
        for en in names {
            let (sym, spelling) = self.designator_sym(&en.designator);
            let mut found: Vec<DeclId> = self.a.region(self.region).direct(sym).to_vec();
            if found.is_empty() {
                // Package bodies specify attributes of the package's
                // subprograms; labels are declared by statements analysed
                // later, so unknown names in the statement classes are
                // not reported.
                if matches!(
                    spec.class,
                    ast::EntityClass::Label
                        | ast::EntityClass::Entity
                        | ast::EntityClass::Architecture
                ) {
                    continue;
                }
                let l = scope::lookup(self.a, self.region, sym);
                found = l.decls;
            }
            if found.is_empty() {
                self.error(
                    "V0200",
                    en.span,
                    format!("`{spelling}` is not declared in this region"),
                );
                continue;
            }
            // Select by signature when given.
            if let Some(sig) = &en.signature {
                let param_tys: Vec<TypeId> = sig
                    .params
                    .iter()
                    .map(|p| self.resolve_type_mark(p))
                    .collect();
                let ret = sig.return_type.as_ref().map(|r| self.resolve_type_mark(r));
                found.retain(|&d| match &self.a.decl(d).kind {
                    DeclKind::Subprogram { sig, .. } => {
                        sig.params.len() == param_tys.len()
                            && sig
                                .params
                                .iter()
                                .zip(&param_tys)
                                .all(|(p, t)| self.a.same_base(p.ty, *t))
                            && match (sig.ret, ret) {
                                (None, None) => true,
                                (Some(a), Some(b)) => self.a.same_base(a, b),
                                _ => false,
                            }
                    }
                    _ => false,
                });
                if found.is_empty() {
                    self.error(
                        "V0303",
                        sig.span,
                        format!("no `{spelling}` matches this signature"),
                    );
                    continue;
                }
            }
            for d in found {
                self.a.set_ref(en.span, d);
                if let Some(v) = &value {
                    self.a.set_attribute_value(d, asym, v.clone());
                }
                if asym == self.syms.foreign
                    && let DeclKind::Subprogram { body, .. } = &mut self.a.decl_mut(d).kind
                {
                    let text = value
                        .as_ref()
                        .and_then(|v| {
                            v.as_array().map(|a| {
                                a.elems
                                    .iter()
                                    .filter_map(|e| e.as_enum().and_then(char::from_u32))
                                    .collect::<String>()
                            })
                        })
                        .unwrap_or_default();
                    *body = SubprogramBody::Foreign(text);
                }
            }
        }
    }

    fn component_decl(&mut self, c: &ast::ComponentDecl) {
        let prev = self.enter(RegionKind::Component);
        let region = self.region;
        let generics = self.interface_list(&c.generics, ObjectRole::Generic);
        let ports = self.interface_list(&c.ports, ObjectRole::Port);
        self.leave(prev);
        let sym = self.ident_sym(&c.name);
        self.declare(
            sym,
            &c.name.name,
            DeclKind::Component {
                generics,
                ports,
                region,
            },
            c.name.span,
        );
    }

    /// Declares a subprogram from its specification. With `body`, links
    /// it to an earlier declaration of the same profile (or declares it
    /// fresh) and returns the declaration; the parameters are declared in
    /// a new region that is left current for the body's declarations.
    pub(crate) fn subprogram_decl(
        &mut self,
        spec: &ast::SubprogramSpec,
        body: Option<Span>,
    ) -> DeclId {
        let (sym, spelling) = self.designator_sym(&spec.designator);
        let outer = self.region;
        let sub_region = self.a.add_region(RegionKind::Subprogram, Some(outer));
        self.region = sub_region;
        if !spec.generics.is_empty() {
            self.require_2008(spec.span, "generic subprograms");
            self.interface_list(&spec.generics, ObjectRole::Generic);
        }
        let param_decls = self.interface_list(&spec.params, ObjectRole::Parameter);
        let mut params = Vec::new();
        for d in param_decls {
            let (ty, mode, class) = match self.a.decl(d).kind {
                DeclKind::Object {
                    ty, mode, class, ..
                } => (ty, mode.unwrap_or(Mode::In), class),
                _ => continue,
            };
            // Defaults: find the interface object that declared it.
            let has_default = spec.params.iter().any(|p| match p {
                InterfaceDecl::Object(o) => {
                    o.default.is_some() && o.names.iter().any(|n| n.span == self.a.decl(d).span)
                }
                _ => false,
            });
            params.push(Param {
                decl: d,
                ty,
                mode,
                class,
                has_default,
            });
        }
        let ret = spec.return_type.as_ref().map(|r| self.resolve_type_mark(r));
        let pure = match spec.kind {
            SubprogramKind::Function => spec.pure.unwrap_or(true),
            SubprogramKind::Procedure => false,
        };
        if spec.kind == SubprogramKind::Function && spec.return_type.is_none() {
            self.error("V0203", spec.span, "a function needs a `return` type");
        }
        for p in &params {
            if spec.kind == SubprogramKind::Function && p.mode != Mode::In {
                let pspan = self.a.decl(p.decl).span;
                self.error("V0203", pspan, "function parameters must be of mode `in`");
            }
        }
        let sig = Signature {
            kind: spec.kind,
            params,
            ret,
            pure,
        };
        self.region = outer;
        // Link a body to its declaration.
        if let Some(bspan) = body {
            let existing: Vec<DeclId> = self.a.region(outer).direct(sym).to_vec();
            for e in existing {
                let decl = self.a.decl(e).clone();
                if let DeclKind::Subprogram {
                    sig: esig,
                    body: ebody,
                } = &decl.kind
                    && same_profile(self.a, esig, &sig)
                {
                    match ebody {
                        SubprogramBody::None | SubprogramBody::Foreign(_) => {
                            if let DeclKind::Subprogram { body: b, .. } =
                                &mut self.a.decl_mut(e).kind
                                && *b == SubprogramBody::None
                            {
                                *b = SubprogramBody::Vhdl(bspan);
                            }
                            // The body's own parameter declarations are the ones
                            // its statements see; keep the declaration's profile.
                            self.a.set_ref(spec.designator.span(), e);
                            self.region = sub_region;
                            return e;
                        }
                        SubprogramBody::Vhdl(prev) => {
                            let prev = *prev;
                            self.push(
                                Diagnostic::error(format!("`{spelling}` already has a body"))
                                    .with_code("V0202")
                                    .with_label(spec.designator.span(), "second body here")
                                    .with_secondary(prev, "first body here"),
                            );
                            self.region = sub_region;
                            return e;
                        }
                        SubprogramBody::Implicit => {}
                    }
                }
            }
        }
        let d = self.declare(
            sym,
            &spelling,
            DeclKind::Subprogram {
                sig,
                body: body.map_or(SubprogramBody::None, SubprogramBody::Vhdl),
            },
            spec.designator.span(),
        );
        if body.is_some() {
            self.region = sub_region;
        }
        d
    }

    fn subprogram_body(&mut self, b: &ast::SubprogramBody) {
        let outer = self.region;
        let d = self.subprogram_decl(&b.spec, Some(b.span));
        // `subprogram_decl` left the parameter region current.
        let DeclKind::Subprogram { sig, .. } = self.a.decl(d).kind.clone() else {
            self.region = outer;
            return;
        };
        let saved = self.ctx.clone();
        self.ctx.subprogram = Some(SubCtx {
            kind: sig.kind,
            pure: sig.pure,
            ret: sig.ret,
            decl: d,
        });
        self.ctx.process = None;
        self.ctx.loops.clear();
        self.ctx.in_package = false;
        self.declarations(&b.decls);
        self.sequential_statements(&b.statements);
        if sig.kind == SubprogramKind::Function && !returns(&b.statements) && !self.in_stdlib {
            self.warn(
                "V0406",
                b.spec.designator.span(),
                format!(
                    "function `{}` may reach its end without a `return`",
                    self.a.decl(d).spelling
                ),
            );
        }
        self.ctx = saved;
        self.region = outer;
    }
}

/// True when the statement list always ends in a return (conservative:
/// a trailing `return`, or an `if`/`case` whose every arm returns, or a
/// bare `loop` without `exit`).
fn returns(stmts: &[ast::SequentialStatement]) -> bool {
    let Some(last) = stmts.last() else {
        return false;
    };
    match &last.kind {
        ast::SequentialKind::Return(_) => true,
        ast::SequentialKind::If(i) => {
            i.arms.iter().all(|a| returns(&a.statements))
                && i.else_statements.as_ref().is_some_and(|e| returns(e))
        }
        ast::SequentialKind::Case(c) => c.arms.iter().all(|a| returns(&a.statements)),
        ast::SequentialKind::Loop(l) => l.scheme.is_none() || returns(&l.statements),
        ast::SequentialKind::Assertion(_) | ast::SequentialKind::Report { .. } => {
            // `assert false` / `report ... severity failure` at the end
            // is a common idiom for "unreachable".
            true
        }
        _ => false,
    }
}

/// The declaration an alias introduces, given the aliased one. A
/// subprogram alias is not a second declaration needing its own body
/// (clause 6.6.2), so its body is marked implicit.
fn alias_kind(kind: DeclKind) -> DeclKind {
    match kind {
        DeclKind::Subprogram { sig, .. } => DeclKind::Subprogram {
            sig,
            body: SubprogramBody::Implicit,
        },
        other => other,
    }
}

/// Rounds a real to an integer, or `None` when it does not fit: the
/// value of a physical literal such as `1.5 ns` is `round(1.5 * 1000)`
/// femtoseconds, and a literal too large for `i128` is not static.
pub(crate) fn round_to_i128(r: f64) -> Option<i128> {
    let r = r.round();
    if r.is_finite() && r.abs() < 1.0e38 {
        // The bound keeps the conversion inside `i128`, so it is exact.
        #[allow(clippy::cast_possible_truncation)]
        Some(r as i128)
    } else {
        None
    }
}

/// True when two signatures have the same parameter and result type
/// profile (clause 4.5.1).
pub(crate) fn same_profile(a: &Analysis, x: &Signature, y: &Signature) -> bool {
    x.kind == y.kind
        && x.params.len() == y.params.len()
        && x.params
            .iter()
            .zip(&y.params)
            .all(|(p, q)| a.same_base(p.ty, q.ty))
        && match (x.ret, y.ret) {
            (None, None) => true,
            (Some(r), Some(s)) => a.same_base(r, s),
            _ => false,
        }
}

fn class_word(c: super::TypeClass) -> &'static str {
    match c {
        super::TypeClass::File => "file",
        super::TypeClass::Access => "access",
        super::TypeClass::Protected => "protected",
        _ => "this",
    }
}

pub(crate) fn region_word(k: RegionKind) -> &'static str {
    match k {
        RegionKind::Root | RegionKind::Context => "context",
        RegionKind::Package => "package",
        RegionKind::PackageBody => "package body",
        RegionKind::Entity => "entity",
        RegionKind::Architecture => "architecture",
        RegionKind::Configuration => "configuration",
        RegionKind::Component => "component",
        RegionKind::Subprogram => "subprogram",
        RegionKind::Process => "process",
        RegionKind::Block => "block",
        RegionKind::Generate => "generate statement",
        RegionKind::Protected => "protected type",
        RegionKind::Loop => "loop",
        RegionKind::Record => "record",
    }
}

fn article(k: RegionKind) -> &'static str {
    match k {
        RegionKind::Architecture | RegionKind::Entity => "an",
        _ => "a",
    }
}
