//! Concurrent and sequential statements (IEEE 1076-2008 clauses 10 and 11).
//!
//! The statement checker resolves the names in every statement, checks the
//! rules that depend on where the statement appears, and records the
//! labels of processes, blocks, generates and instances so the lowering
//! pass can find them. The rules it enforces beyond typing:
//!
//! - **Assignment targets** (clause 10.5): a signal assignment needs a
//!   signal target, a variable assignment a variable target; neither may
//!   write a constant, a generic, a loop parameter or an `in` port.
//! - **Signals in subprograms** (clause 4.3): a function body may not
//!   assign a signal at all, and a pure function may neither call an
//!   impure one nor use a signal attribute that depends on simulation
//!   time.
//! - **Process form** (clause 11.3): a process has a sensitivity list or
//!   `wait` statements, never both; a sensitivity list names static
//!   signals; a process without either never resumes.
//! - **`wait` placement** (clause 10.2): not inside a function, and not
//!   in a process with a sensitivity list.
//! - **Loop control** (clause 10.10, 10.11): `next` and `exit` need an
//!   enclosing loop, and a named one must name an enclosing loop label.
//! - **`return`** (clause 10.12): a function returns a value of its
//!   result subtype, a procedure returns none.
//! - **Choices** (clause 10.9): `case` choices are locally static, cover
//!   the selector's subtype (or end in `others`), and do not overlap.
//! - **Association lists** (clause 6.5.7): port and generic maps are
//!   checked here, including the direction rules an actual must satisfy
//!   for the formal's mode.

use std::collections::HashMap;

use crate::diag::Diagnostic;
use crate::source::Span;
use crate::vhdl::ast::{
    self, Actual, AssociationElement, Choice, ConcurrentKind, ConcurrentStatement, Expr, Mode,
    Name, SequentialKind, SequentialStatement, Target, Waveform,
};

use super::check::{Checker, ProcCtx};
use super::constant::Value;
use super::expr::{Mode as RMode, ObjInfo, Prefix};
use super::types::{TypeClass, TypeId};
use super::{CallTarget, DeclId, DeclKind, LabelKind, ObjectClass, ObjectRole, RegionKind};

impl Checker<'_> {
    // --- concurrent statements ---------------------------------------------

    pub(crate) fn concurrent_statements(&mut self, stmts: &[ConcurrentStatement]) {
        // Labels are visible throughout the statement part, so declare
        // them all first (clause 12.1).
        for s in stmts {
            if let Some(l) = &s.label {
                let sym = self.ident_sym(l);
                let kind = match &s.kind {
                    ConcurrentKind::Process(_) => LabelKind::Process,
                    ConcurrentKind::Block(_) => LabelKind::Block,
                    ConcurrentKind::Instantiation(_) => LabelKind::Instance,
                    ConcurrentKind::ForGenerate(_)
                    | ConcurrentKind::IfGenerate(_)
                    | ConcurrentKind::CaseGenerate(_) => LabelKind::Generate,
                    _ => LabelKind::Other,
                };
                self.declare(sym, &l.name, DeclKind::Label(kind), l.span);
            }
        }
        for s in stmts {
            self.concurrent_statement(s);
        }
    }

    fn concurrent_statement(&mut self, s: &ConcurrentStatement) {
        match &s.kind {
            ConcurrentKind::Process(p) => self.process(p),
            ConcurrentKind::Block(b) => self.block(b),
            ConcurrentKind::SignalAssignment(csa) => {
                // A concurrent signal assignment is a process: the same
                // target and waveform rules apply.
                let saved = self.ctx.process.replace(ProcCtx {
                    has_sensitivity: true,
                    sensitivity_span: None,
                    has_wait: false,
                });
                self.signal_assignment(&csa.assignment);
                self.ctx.process = saved;
            }
            ConcurrentKind::ProcedureCall { call, .. } => self.procedure_call(call),
            ConcurrentKind::Assertion { assertion, .. } => self.assertion(assertion),
            ConcurrentKind::Instantiation(i) => self.instantiation(i, s.label.as_ref()),
            ConcurrentKind::ForGenerate(g) => {
                let prev = self.enter(RegionKind::Generate);
                let info = self.resolve_discrete_range(&g.range);
                let sym = self.ident_sym(&g.param);
                self.declare(
                    sym,
                    &g.param.name,
                    DeclKind::Object {
                        class: ObjectClass::Constant,
                        ty: info.ty,
                        mode: None,
                        role: ObjectRole::GenerateParam,
                        deferred: false,
                    },
                    g.param.span,
                );
                self.a.set_type(g.param.span, info.ty);
                self.generate_body(&g.body);
                self.leave(prev);
            }
            ConcurrentKind::IfGenerate(g) => {
                for arm in &g.arms {
                    self.resolve_condition(&arm.condition);
                    if !self.is_globally_static(&arm.condition) {
                        self.error(
                            "V0403",
                            arm.condition.span(),
                            "the condition of an if-generate must be static (it selects structure, not behaviour)",
                        );
                    }
                    let prev = self.enter(RegionKind::Generate);
                    self.generate_body(&arm.body);
                    self.leave(prev);
                }
                if let Some(e) = &g.else_arm {
                    self.require_2008(e.span, "`else generate` branches");
                    let prev = self.enter(RegionKind::Generate);
                    self.generate_body(e);
                    self.leave(prev);
                }
            }
            ConcurrentKind::CaseGenerate(g) => {
                self.require_2008(g.span, "case-generate statements");
                let ty = self.resolve_free(&g.expr);
                let arms: Vec<&[Choice]> = g.arms.iter().map(|a| a.choices.as_slice()).collect();
                self.check_choices(&arms, ty, g.span, "case-generate");
                for arm in &g.arms {
                    let prev = self.enter(RegionKind::Generate);
                    self.generate_body(&arm.body);
                    self.leave(prev);
                }
            }
        }
    }

    fn generate_body(&mut self, b: &ast::GenerateBody) {
        if let Some(l) = &b.label {
            let sym = self.ident_sym(l);
            self.declare(sym, &l.name, DeclKind::Label(LabelKind::Generate), l.span);
        }
        self.declarations(&b.decls);
        self.concurrent_statements(&b.statements);
    }

    fn block(&mut self, b: &ast::BlockStatement) {
        let prev = self.enter(RegionKind::Block);
        if let Some(g) = &b.guard {
            let ty = self.resolve_condition(g);
            // The implicit GUARD signal (clause 11.2).
            let sym = self.syms.guard;
            let boolean = self.a.builtins.boolean;
            let _ = ty;
            self.declare(
                sym,
                "guard",
                DeclKind::Object {
                    class: ObjectClass::Signal,
                    ty: boolean,
                    mode: None,
                    role: ObjectRole::Plain,
                    deferred: false,
                },
                g.span(),
            );
        }
        let generics = self.interface_list(&b.generics, ObjectRole::Generic);
        if let Some(map) = &b.generic_map {
            self.association_list(map, &generics, b.span, "generic map", false);
        }
        let ports = self.interface_list(&b.ports, ObjectRole::Port);
        if let Some(map) = &b.port_map {
            self.association_list(map, &ports, b.span, "port map", true);
        }
        self.declarations(&b.decls);
        self.concurrent_statements(&b.statements);
        self.leave(prev);
    }

    fn process(&mut self, p: &ast::ProcessStatement) {
        let prev = self.enter(RegionKind::Process);
        let saved = self.ctx.clone();
        self.ctx.subprogram = None;
        self.ctx.loops.clear();
        let mut pc = ProcCtx::default();
        if let Some(s) = &p.sensitivity {
            pc.has_sensitivity = true;
            match s {
                ast::Sensitivity::Names(names) => {
                    pc.sensitivity_span = names.first().map(|n| n.span());
                    for n in names {
                        self.sensitivity_name(n);
                    }
                }
                ast::Sensitivity::All(span) => {
                    self.require_2008(*span, "`process (all)` sensitivity lists");
                    pc.sensitivity_span = Some(*span);
                }
            }
        }
        self.ctx.process = Some(pc);
        self.declarations(&p.decls);
        self.sequential_statements(&p.statements);
        let has_wait = self.ctx.process.map(|p| p.has_wait).unwrap_or(false);
        if !has_wait && p.sensitivity.is_none() && !self.in_stdlib {
            self.warn(
                "V0404",
                p.span,
                "this process has neither a sensitivity list nor a `wait`, so it runs forever without suspending",
            );
        }
        self.ctx = saved;
        self.leave(prev);
    }

    /// A name in a sensitivity list: a static signal name.
    fn sensitivity_name(&mut self, n: &Name) {
        let info = self.commit_name(n, None);
        let Some(obj) = info.obj else {
            // A signal-valued attribute (`s'delayed`) is allowed.
            if !matches!(self.a.call_of(n.span()), Some(CallTarget::Attribute(_)))
                && info.ty.is_some()
            {
                self.error(
                    "V0404",
                    n.span(),
                    "only signals may appear in a sensitivity list",
                );
            }
            return;
        };
        if obj.class != ObjectClass::Signal {
            let dspan = self.a.decl(obj.decl).span;
            let word = obj.class.as_str();
            self.push(
                Diagnostic::error(format!(
                    "only signals may appear in a sensitivity list, but this is a {word}"
                ))
                .with_code("V0404")
                .with_label(n.span(), format!("a {word}, not a signal"))
                .with_secondary(dspan, format!("declared as a {word} here")),
            );
            return;
        }
        if obj.mode == Some(Mode::Out) && obj.role == ObjectRole::Port {
            self.error(
                "V0404",
                n.span(),
                "an `out` port cannot appear in a sensitivity list",
            );
        }
    }

    fn instantiation(&mut self, i: &ast::Instantiation, label: Option<&ast::Ident>) {
        if label.is_none() {
            self.error("V0400", i.span, "an instantiation needs a label");
        }
        let (generics, ports) = match &i.unit {
            ast::InstantiatedUnit::Component(n) => {
                let p = self.classify(n, RMode::Commit, None);
                match p {
                    Prefix::Error => (Vec::new(), Vec::new()),
                    _ => {
                        // A component, or a direct entity by selected name.
                        let decl = self.a.decl_of(n.span());
                        match decl.map(|d| self.a.decl(d).kind.clone()) {
                            Some(DeclKind::Component {
                                generics, ports, ..
                            }) => (generics, ports),
                            Some(DeclKind::Unit { unit, .. }) => self.unit_interfaces(unit),
                            _ => {
                                let text = self.text(n.span()).to_owned();
                                let mut d = Diagnostic::error(format!(
                                    "`{text}` is not a component or entity"
                                ))
                                .with_code("V0206")
                                .with_span(n.span());
                                if decl.is_some() {
                                    d = d.with_note(
                                        "declare a `component` for it, or instantiate it directly with `entity work.name`",
                                    );
                                }
                                self.push(d);
                                (Vec::new(), Vec::new())
                            }
                        }
                    }
                }
            }
            ast::InstantiatedUnit::Entity { name, architecture } => {
                let u = self.resolve_entity_name(name);
                if let (Some(u), Some(arch)) = (u, architecture) {
                    let asym = self.ident_sym(arch);
                    let lib = self.a.units[u.index()].library;
                    let ename = self.a.units[u.index()].name;
                    let found = self.a.units.iter().any(|x| {
                        x.library == lib
                            && x.kind == super::LibraryUnitKind::Architecture
                            && x.name == asym
                            && x.primary == Some(ename)
                    });
                    if !found {
                        let en = self.a.name(ename).to_owned();
                        self.error(
                            "V0102",
                            arch.span,
                            format!("no architecture `{}` of entity `{en}`", arch.name),
                        );
                    }
                }
                u.map(|u| self.unit_interfaces(u)).unwrap_or_default()
            }
            ast::InstantiatedUnit::Configuration(n) => {
                let p = self.classify(n, RMode::Commit, None);
                let _ = p;
                (Vec::new(), Vec::new())
            }
        };
        if let Some(map) = &i.generic_map {
            self.association_list(map, &generics, i.span, "generic map", false);
        } else if let Some(missing) = self.first_without_default(&generics) {
            let name = self.a.decl(missing).spelling.clone();
            let dspan = self.a.decl(missing).span;
            self.push(
                Diagnostic::error(format!("generic `{name}` has no value and no default"))
                    .with_code("V0501")
                    .with_label(i.span, "this instantiation has no generic map")
                    .with_secondary(dspan, "declared here"),
            );
        }
        match &i.port_map {
            Some(map) => self.association_list(map, &ports, i.span, "port map", true),
            None => {
                if let Some(missing) = self.first_required_port(&ports) {
                    let name = self.a.decl(missing).spelling.clone();
                    let dspan = self.a.decl(missing).span;
                    self.push(
                        Diagnostic::error(format!("port `{name}` is not connected"))
                            .with_code("V0501")
                            .with_label(i.span, "this instantiation has no port map")
                            .with_secondary(dspan, "declared here"),
                    );
                }
            }
        }
    }

    /// The generics and ports of an entity unit.
    fn unit_interfaces(&self, unit: super::UnitId) -> (Vec<DeclId>, Vec<DeclId>) {
        let Some(region) = self.a.units[unit.index()].region else {
            return (Vec::new(), Vec::new());
        };
        let mut generics = Vec::new();
        let mut ports = Vec::new();
        for &d in &self.a.region(region).decls {
            if let DeclKind::Object { role, .. } = self.a.decl(d).kind {
                match role {
                    ObjectRole::Generic => generics.push(d),
                    ObjectRole::Port => ports.push(d),
                    _ => {}
                }
            }
        }
        (generics, ports)
    }

    fn first_without_default(&self, decls: &[DeclId]) -> Option<DeclId> {
        decls
            .iter()
            .copied()
            .find(|&d| self.a.decl_value(d).is_none() && !self.has_default(d))
    }

    fn has_default(&self, d: DeclId) -> bool {
        // A generic or port with a default has its value recorded at
        // declaration time; ports of mode `in` without a default must be
        // connected.
        self.a.decl_value(d).is_some()
    }

    fn first_required_port(&self, ports: &[DeclId]) -> Option<DeclId> {
        ports.iter().copied().find(|&d| {
            matches!(
                self.a.decl(d).kind,
                DeclKind::Object {
                    mode: Some(Mode::In),
                    ..
                }
            ) && !self.has_default(d)
        })
    }

    /// Checks a generic or port map against the formals.
    pub(crate) fn association_list(
        &mut self,
        list: &[AssociationElement],
        formals: &[DeclId],
        span: Span,
        what: &str,
        ports: bool,
    ) {
        let mut assoc: HashMap<DeclId, Span> = HashMap::new();
        let mut named = false;
        for (i, el) in list.iter().enumerate() {
            let formal = match &el.formal {
                Some(f) => {
                    named = true;
                    match self.formal_decl(f, formals, what) {
                        Some(d) => Some(d),
                        None => continue,
                    }
                }
                None => {
                    if named {
                        self.error(
                            "V0502",
                            el.span,
                            "a positional association cannot follow a named one",
                        );
                        continue;
                    }
                    match formals.get(i) {
                        Some(&d) => Some(d),
                        None => {
                            self.error(
                                "V0502",
                                el.span,
                                format!(
                                    "this {what} has {} formal{}, but {} associations are given",
                                    formals.len(),
                                    if formals.len() == 1 { "" } else { "s" },
                                    list.len()
                                ),
                            );
                            continue;
                        }
                    }
                }
            };
            let Some(fd) = formal else { continue };
            if let Some(prev) = assoc.insert(fd, el.span) {
                let name = self.a.decl(fd).spelling.clone();
                self.push(
                    Diagnostic::error(format!("`{name}` is associated twice"))
                        .with_code("V0502")
                        .with_label(el.span, "second association")
                        .with_secondary(prev, "first association"),
                );
                continue;
            }
            let (fty, fmode, fclass) = match self.a.decl(fd).kind {
                DeclKind::Object {
                    ty, mode, class, ..
                } => (ty, mode.unwrap_or(Mode::In), class),
                _ => continue,
            };
            match &el.actual {
                Actual::Open(s) => {
                    if ports && fmode == Mode::In && !self.has_default(fd) {
                        let name = self.a.decl(fd).spelling.clone();
                        let dspan = self.a.decl(fd).span;
                        self.push(
                            Diagnostic::error(format!("`in` port `{name}` cannot be left open"))
                                .with_code("V0501")
                                .with_label(*s, "left unconnected")
                                .with_secondary(dspan, "declared `in` without a default here")
                                .with_note("give it a value, or a default in the entity"),
                        );
                    }
                }
                Actual::Expr(e) | Actual::Inertial(e) => {
                    self.resolve(e, fty);
                    self.check_static_range(e, fty);
                    if ports {
                        self.check_port_actual(e, fd, fty, fmode, fclass);
                    } else if fclass == ObjectClass::Constant {
                        // A generic actual must be static at elaboration;
                        // a non-static one is caught by elaboration.
                    }
                }
                Actual::Range(r) => {
                    // A generic type or subtype association.
                    self.resolve_discrete_range(r);
                }
            }
        }
        // Unassociated formals.
        for &f in formals {
            if assoc.contains_key(&f) {
                continue;
            }
            let (mode, _) = match self.a.decl(f).kind {
                DeclKind::Object { mode, ty, .. } => (mode.unwrap_or(Mode::In), ty),
                _ => continue,
            };
            if self.has_default(f) {
                continue;
            }
            if !ports || mode == Mode::In {
                let name = self.a.decl(f).spelling.clone();
                let dspan = self.a.decl(f).span;
                let word = if ports { "port" } else { "generic" };
                self.push(
                    Diagnostic::error(format!("{word} `{name}` has no association"))
                        .with_code("V0501")
                        .with_label(span, format!("this {what} does not associate `{name}`"))
                        .with_secondary(dspan, "declared here"),
                );
            }
        }
    }

    /// Resolves the formal part of an association to one of `formals`.
    fn formal_decl(&mut self, f: &Expr, formals: &[DeclId], what: &str) -> Option<DeclId> {
        // `formal`, `formal(i)`, `formal.field` or `conv(formal)`.
        let name = match f {
            Expr::Name(n) => n,
            other => {
                self.error("V0502", other.span(), "expected a formal name");
                return None;
            }
        };
        let root = name.root();
        let sym = match root {
            Name::Simple(i) => self.ident_sym(i),
            _ => {
                self.error("V0502", f.span(), "expected a formal name");
                return None;
            }
        };
        let found = formals
            .iter()
            .copied()
            .find(|&d| self.a.decl(d).name == sym);
        match found {
            Some(d) => {
                self.a.set_ref(root.span(), d);
                if let DeclKind::Object { ty, .. } = self.a.decl(d).kind {
                    self.a.set_type(root.span(), ty);
                }
                Some(d)
            }
            None => {
                let text = self.text(root.span()).to_owned();
                let names: Vec<String> = formals
                    .iter()
                    .map(|&d| self.a.decl(d).spelling.clone())
                    .collect();
                let mut diag = Diagnostic::error(format!("no formal `{text}` in this {what}"))
                    .with_code("V0502")
                    .with_span(root.span());
                if let Some(s) = super::suggest(&text, names.iter().map(String::as_str)) {
                    diag = diag.with_note(format!("did you mean `{s}`?"));
                } else if !names.is_empty() {
                    let shown: Vec<String> = names.iter().take(8).cloned().collect();
                    diag = diag.with_note(format!("available: {}", shown.join(", ")));
                }
                self.push(diag);
                None
            }
        }
    }

    /// The direction rules for a port association (clause 6.5.7.1).
    fn check_port_actual(
        &mut self,
        e: &Expr,
        formal: DeclId,
        fty: TypeId,
        fmode: Mode,
        _fclass: ObjectClass,
    ) {
        let _ = fty;
        let Expr::Name(n) = e else {
            // An expression actual is allowed only for an `in` port
            // (VHDL-2008 also allows it for others through conversions).
            if fmode != Mode::In {
                let name = self.a.decl(formal).spelling.clone();
                let dspan = self.a.decl(formal).span;
                self.push(
                    Diagnostic::error(format!(
                        "port `{name}` is `{}`, so its actual must be a signal",
                        mode_word(fmode)
                    ))
                    .with_code("V0501")
                    .with_label(e.span(), "this is an expression, not a signal")
                    .with_secondary(dspan, format!("declared `{}` here", mode_word(fmode))),
                );
            }
            return;
        };
        let info = self.name_info(n);
        let Some(obj) = info.obj else {
            if fmode != Mode::In {
                let name = self.a.decl(formal).spelling.clone();
                self.error(
                    "V0501",
                    e.span(),
                    format!(
                        "the actual for `{}` port `{name}` must be a signal",
                        mode_word(fmode)
                    ),
                );
            }
            return;
        };
        let name = self.a.decl(formal).spelling.clone();
        let fspan = self.a.decl(formal).span;
        if obj.class != ObjectClass::Signal {
            let word = obj.class.as_str();
            let dspan = self.a.decl(obj.decl).span;
            if fmode == Mode::In && obj.class == ObjectClass::Constant {
                // A constant actual on an `in` port is a legal
                // expression association.
                return;
            }
            self.push(
                Diagnostic::error(format!(
                    "the actual for port `{name}` must be a signal, but this is a {word}"
                ))
                .with_code("V0501")
                .with_label(e.span(), format!("a {word}"))
                .with_secondary(dspan, format!("declared as a {word} here")),
            );
            return;
        }
        // Mode compatibility (clause 6.5.7.1 table).
        let amode = obj.mode.unwrap_or(Mode::Inout);
        let is_port = obj.role == ObjectRole::Port;
        if !is_port {
            return;
        }
        let ok = match fmode {
            Mode::In => !matches!(amode, Mode::Out) || self.v2008(),
            Mode::Out | Mode::Buffer => !matches!(amode, Mode::In),
            Mode::Inout => matches!(amode, Mode::Inout | Mode::Buffer),
            Mode::Linkage => true,
        };
        if !ok {
            let aname = self.a.decl(obj.decl).spelling.clone();
            let mut d = Diagnostic::error(format!(
                "cannot connect `{}` port `{aname}` to `{}` port `{name}`",
                mode_word(amode),
                mode_word(fmode)
            ))
            .with_code("V0503")
            .with_label(e.span(), format!("`{aname}` is `{}`", mode_word(amode)))
            .with_secondary(fspan, format!("`{name}` is `{}`", mode_word(fmode)));
            d = match (fmode, amode) {
                (Mode::Out | Mode::Inout, Mode::In) => d.with_note(format!(
                    "an `in` port cannot be driven; declare `{aname}` as `out` or `inout`"
                )),
                (Mode::Inout, _) => d.with_note(format!(
                    "an `inout` formal needs an `inout` or `buffer` actual; declare `{aname}` as `inout`"
                )),
                _ => d.with_note("a signal read by the instance must be readable at this level"),
            };
            self.push(d);
        }
    }

    // --- sequential statements ---------------------------------------------

    pub(crate) fn sequential_statements(&mut self, stmts: &[SequentialStatement]) {
        for s in stmts {
            self.sequential_statement(s);
        }
    }

    fn sequential_statement(&mut self, s: &SequentialStatement) {
        if let Some(l) = &s.label
            && !matches!(s.kind, SequentialKind::Loop(_))
        {
            let sym = self.ident_sym(l);
            self.declare(sym, &l.name, DeclKind::Label(LabelKind::Other), l.span);
        }
        match &s.kind {
            SequentialKind::Wait {
                sensitivity,
                condition,
                timeout,
            } => {
                if let Some(sub) = self.ctx.subprogram
                    && sub.kind == ast::SubprogramKind::Function
                {
                    self.error(
                        "V0405",
                        s.span,
                        "a function must not contain a `wait` statement",
                    );
                }
                if let Some(p) = &mut self.ctx.process {
                    p.has_wait = true;
                    if p.has_sensitivity {
                        let sspan = p.sensitivity_span;
                        let mut d = Diagnostic::error(
                            "a process with a sensitivity list must not contain a `wait` statement",
                        )
                        .with_code("V0404")
                        .with_label(s.span, "`wait` here");
                        if let Some(sp) = sspan {
                            d = d.with_secondary(sp, "sensitivity list here");
                        }
                        self.push(d.with_note("remove the sensitivity list, or the `wait`"));
                    }
                }
                if let Some(ast::Sensitivity::Names(names)) = sensitivity {
                    for n in names {
                        self.sensitivity_name(n);
                    }
                }
                if let Some(c) = condition {
                    self.resolve_condition(c);
                }
                if let Some(t) = timeout {
                    let time = self.a.builtins.time;
                    self.resolve(t, time);
                }
            }
            SequentialKind::Assertion(a) => self.assertion(a),
            SequentialKind::Report { message, severity } => {
                let string = self.a.builtins.string;
                self.resolve(message, string);
                if let Some(sev) = severity {
                    let sl = self.a.builtins.severity_level;
                    self.resolve(sev, sl);
                }
            }
            SequentialKind::SignalAssignment(sa) => self.signal_assignment(sa),
            SequentialKind::VariableAssignment(va) => self.variable_assignment(va),
            SequentialKind::ProcedureCall(call) => self.procedure_call(call),
            SequentialKind::If(i) => {
                for arm in &i.arms {
                    self.resolve_condition(&arm.condition);
                    self.sequential_statements(&arm.statements);
                }
                if let Some(e) = &i.else_statements {
                    self.sequential_statements(e);
                }
            }
            SequentialKind::Case(c) => {
                let ty = self.resolve_case_selector(&c.expr);
                let arms: Vec<&[Choice]> = c.arms.iter().map(|a| a.choices.as_slice()).collect();
                if c.matching {
                    self.require_2008(c.span, "matching case statements (`case?`)");
                }
                self.check_choices(&arms, ty, c.span, "case");
                for arm in &c.arms {
                    self.sequential_statements(&arm.statements);
                }
            }
            SequentialKind::Loop(l) => {
                // The label belongs to the enclosing region; the loop
                // parameter to the loop's own region (clause 10.10).
                let label = s.label.as_ref().map(|lab| {
                    let sym = self.ident_sym(lab);
                    self.declare(sym, &lab.name, DeclKind::Label(LabelKind::Loop), lab.span);
                    sym
                });
                let prev = self.enter(RegionKind::Loop);
                match &l.scheme {
                    Some(ast::IterationScheme::While(c)) => {
                        self.resolve_condition(c);
                    }
                    Some(ast::IterationScheme::For { param, range }) => {
                        let info = self.resolve_discrete_range(range);
                        let sym = self.ident_sym(param);
                        self.declare(
                            sym,
                            &param.name,
                            DeclKind::Object {
                                class: ObjectClass::Constant,
                                ty: info.ty,
                                mode: None,
                                role: ObjectRole::LoopParam,
                                deferred: false,
                            },
                            param.span,
                        );
                        self.a.set_type(param.span, info.ty);
                    }
                    None => {}
                }
                self.ctx.loops.push(label);
                self.sequential_statements(&l.statements);
                self.ctx.loops.pop();
                self.leave(prev);
            }
            SequentialKind::Next { label, condition }
            | SequentialKind::Exit { label, condition } => {
                let word = if matches!(s.kind, SequentialKind::Next { .. }) {
                    "next"
                } else {
                    "exit"
                };
                if self.ctx.loops.is_empty() {
                    self.error(
                        "V0407",
                        s.span,
                        format!("`{word}` is only allowed inside a loop"),
                    );
                } else if let Some(l) = label {
                    let sym = self.ident_sym(l);
                    if !self.ctx.loops.contains(&Some(sym)) {
                        self.error(
                            "V0407",
                            l.span,
                            format!("`{}` does not name an enclosing loop", l.name),
                        );
                    } else {
                        let lk = super::scope::lookup(self.a, self.region, sym);
                        if let Some(d) = lk.single() {
                            self.a.set_ref(l.span, d);
                        }
                    }
                }
                if let Some(c) = condition {
                    self.resolve_condition(c);
                }
            }
            SequentialKind::Return(e) => self.return_statement(e.as_ref(), s.span),
            SequentialKind::Null => {}
        }
    }

    fn return_statement(&mut self, e: Option<&Expr>, span: Span) {
        let Some(sub) = self.ctx.subprogram else {
            self.error(
                "V0406",
                span,
                "`return` is only allowed inside a subprogram",
            );
            if let Some(e) = e {
                self.resolve_free(e);
            }
            return;
        };
        match (sub.kind, e) {
            (ast::SubprogramKind::Function, Some(e)) => {
                let ret = sub.ret.unwrap_or(self.a.builtins.error);
                self.resolve(e, ret);
                self.check_static_range(e, ret);
            }
            (ast::SubprogramKind::Function, None) => {
                let name = self.a.decl(sub.decl).spelling.clone();
                let ret = sub
                    .ret
                    .map(|r| self.ty_name(r))
                    .unwrap_or_else(|| "its result type".into());
                self.push(
                    Diagnostic::error(format!("`return` in function `{name}` needs a value"))
                        .with_code("V0406")
                        .with_label(span, format!("expected a value of `{ret}`")),
                );
            }
            (ast::SubprogramKind::Procedure, Some(e)) => {
                let name = self.a.decl(sub.decl).spelling.clone();
                self.push(
                    Diagnostic::error(format!("`return` in procedure `{name}` takes no value"))
                        .with_code("V0406")
                        .with_label(e.span(), "a procedure returns nothing"),
                );
                self.resolve_free(e);
            }
            (ast::SubprogramKind::Procedure, None) => {}
        }
    }

    fn assertion(&mut self, a: &ast::Assertion) {
        self.resolve_condition(&a.condition);
        if let Some(r) = &a.report {
            let string = self.a.builtins.string;
            self.resolve(r, string);
        }
        if let Some(s) = &a.severity {
            let sl = self.a.builtins.severity_level;
            self.resolve(s, sl);
        }
    }

    fn procedure_call(&mut self, call: &Name) {
        // A bare name without arguments is a parameterless call.
        match self.classify(call, RMode::Commit, None) {
            Prefix::Overloaded(ds) => {
                let procs: Vec<DeclId> = ds
                    .iter()
                    .copied()
                    .filter(|&d| {
                        matches!(&self.a.decl(d).kind, DeclKind::Subprogram { sig, .. }
                            if sig.kind == ast::SubprogramKind::Procedure)
                    })
                    .collect();
                let viable: Vec<DeclId> = procs
                    .iter()
                    .copied()
                    .filter(|&d| {
                        matches!(&self.a.decl(d).kind, DeclKind::Subprogram { sig, .. }
                            if sig.params.iter().all(|p| p.has_default))
                    })
                    .collect();
                match viable.as_slice() {
                    [d] => {
                        self.a.set_ref(call.span(), *d);
                        self.a.set_call(call.span(), CallTarget::Subprogram(*d));
                    }
                    [] => {
                        let text = self.text(call.span()).to_owned();
                        let mut diag = Diagnostic::error(format!(
                            "no visible procedure `{text}` takes no arguments"
                        ))
                        .with_code("V0303")
                        .with_span(call.span());
                        for &p in procs.iter().take(5) {
                            diag = diag
                                .with_note(format!("candidate: {}", self.a.describe_subprogram(p)));
                        }
                        if procs.is_empty() && !ds.is_empty() {
                            diag = diag.with_note("this name denotes a function, not a procedure");
                        }
                        self.push(diag);
                    }
                    _ => {
                        self.error("V0302", call.span(), "ambiguous procedure call");
                    }
                }
            }
            Prefix::Value(_) | Prefix::Error => {
                // `p(args)` went through the call path already; check that
                // it named a procedure.
                if let Some(CallTarget::Subprogram(d)) = self.a.call_of(call.span()).cloned()
                    && let DeclKind::Subprogram { sig, .. } = &self.a.decl(d).kind
                    && sig.kind == ast::SubprogramKind::Function
                {
                    let name = self.a.decl(d).spelling.clone();
                    self.push(
                        Diagnostic::error(format!("`{name}` is a function, not a procedure"))
                            .with_code("V0400")
                            .with_label(call.span(), "called as a statement")
                            .with_note("a function call is an expression; use its result"),
                    );
                }
            }
            _ => {
                let text = self.text(call.span()).to_owned();
                self.error("V0400", call.span(), format!("`{text}` is not a procedure"));
            }
        }
    }

    // --- assignments --------------------------------------------------------

    fn signal_assignment(&mut self, sa: &ast::SignalAssignment) {
        let ty = self.assignment_target(&sa.target, true);
        if let Some(d) = &sa.delay {
            match d {
                ast::DelayMechanism::Inertial { reject, .. } => {
                    if let Some(r) = reject {
                        let time = self.a.builtins.time;
                        self.resolve(r, time);
                    }
                }
                ast::DelayMechanism::Transport(_) => {}
            }
        }
        let ty = ty.unwrap_or(self.a.builtins.error);
        match &sa.rhs {
            ast::SignalAssignmentRhs::Simple(w) => self.waveform(w, ty),
            ast::SignalAssignmentRhs::Conditional(arms) => {
                for (i, arm) in arms.iter().enumerate() {
                    self.waveform(&arm.waveform, ty);
                    match &arm.condition {
                        Some(c) => {
                            self.resolve_condition(c);
                        }
                        None => {
                            if i + 1 != arms.len() {
                                self.error(
                                    "V0400",
                                    arm.span,
                                    "only the last alternative may omit its condition",
                                );
                            }
                        }
                    }
                }
            }
            ast::SignalAssignmentRhs::Selected {
                selector,
                matching,
                arms,
            } => {
                if *matching {
                    self.require_2008(sa.span, "matching selected assignments (`select?`)");
                }
                let sty = self.resolve_case_selector(selector);
                let choices: Vec<&[Choice]> = arms.iter().map(|a| a.choices.as_slice()).collect();
                self.check_choices(&choices, sty, sa.span, "selected assignment");
                for arm in arms {
                    self.waveform(&arm.waveform, ty);
                }
            }
            ast::SignalAssignmentRhs::Force { mode, arms } => {
                self.require_2008(sa.span, "`force` assignments");
                let _ = mode;
                for arm in arms {
                    self.resolve(&arm.value, ty);
                    if let Some(c) = &arm.condition {
                        self.resolve_condition(c);
                    }
                }
            }
            ast::SignalAssignmentRhs::Release { .. } => {
                self.require_2008(sa.span, "`release` assignments");
            }
        }
    }

    fn waveform(&mut self, w: &Waveform, ty: TypeId) {
        match w {
            Waveform::Elements(els) => {
                for el in els {
                    if !matches!(&el.value, Expr::Literal(l) if l.kind == ast::LiteralKind::Null) {
                        self.resolve(&el.value, ty);
                        self.check_static_range(&el.value, ty);
                    }
                    if let Some(after) = &el.after {
                        let time = self.a.builtins.time;
                        self.resolve(after, time);
                    }
                }
            }
            Waveform::Unaffected(span) => {
                self.require_2008(*span, "`unaffected` waveforms");
            }
        }
    }

    fn variable_assignment(&mut self, va: &ast::VariableAssignment) {
        let ty = self
            .assignment_target(&va.target, false)
            .unwrap_or(self.a.builtins.error);
        match &va.rhs {
            ast::VariableAssignmentRhs::Simple(e) => {
                self.resolve(e, ty);
                self.check_static_range(e, ty);
            }
            ast::VariableAssignmentRhs::Conditional(arms) => {
                self.require_2008(va.span, "conditional variable assignments");
                for arm in arms {
                    self.resolve(&arm.value, ty);
                    if let Some(c) = &arm.condition {
                        self.resolve_condition(c);
                    }
                }
            }
            ast::VariableAssignmentRhs::Selected {
                selector,
                matching,
                arms,
            } => {
                self.require_2008(va.span, "selected variable assignments");
                let _ = matching;
                let sty = self.resolve_case_selector(selector);
                let choices: Vec<&[Choice]> = arms.iter().map(|a| a.choices.as_slice()).collect();
                self.check_choices(&choices, sty, va.span, "selected assignment");
                for arm in arms {
                    self.resolve(&arm.value, ty);
                }
            }
        }
    }

    /// Resolves an assignment target and checks the object rules.
    /// Returns the target's type.
    fn assignment_target(&mut self, t: &Target, signal: bool) -> Option<TypeId> {
        match t {
            Target::Name(n) => {
                let info = self.commit_name(n, None);
                let ty = info.ty?;
                let Some(obj) = info.obj else {
                    self.error(
                        "V0401",
                        n.span(),
                        "the target of an assignment must be an object",
                    );
                    return Some(ty);
                };
                let what = if signal {
                    "assigned by a signal assignment"
                } else {
                    "assigned by a variable assignment"
                };
                self.check_writable(&obj, n.span(), what);
                self.check_target_class(&obj, n.span(), signal);
                Some(ty)
            }
            Target::Aggregate(agg) => {
                // An aggregate target: every element is an object.
                for el in &agg.elements {
                    if let Expr::Name(n) = &el.value {
                        let info = self.commit_name(n, None);
                        if let Some(obj) = info.obj {
                            self.check_writable(&obj, n.span(), "assigned");
                            self.check_target_class(&obj, n.span(), signal);
                        }
                    } else {
                        self.resolve_free(&el.value);
                    }
                }
                None
            }
        }
    }

    /// A signal assignment needs a signal, a variable assignment a
    /// variable; and a function may not assign signals at all.
    fn check_target_class(&mut self, obj: &ObjInfo, span: Span, signal: bool) {
        let decl = self.a.decl(obj.decl);
        let dspan = decl.span;
        let spelling = decl.spelling.clone();
        if signal {
            if obj.class != ObjectClass::Signal && obj.class != ObjectClass::Constant {
                let word = obj.class.as_str();
                self.push(
                    Diagnostic::error(format!(
                        "`<=` assigns a signal, but `{spelling}` is a {word}"
                    ))
                    .with_code("V0401")
                    .with_label(span, format!("a {word}"))
                    .with_secondary(dspan, format!("declared as a {word} here"))
                    .with_note("use `:=` to assign a variable"),
                );
                return;
            }
            if let Some(sub) = self.ctx.subprogram
                && sub.kind == ast::SubprogramKind::Function
                && obj.role != ObjectRole::Parameter
            {
                let fname = self.a.decl(sub.decl).spelling.clone();
                self.push(
                    Diagnostic::error(format!("function `{fname}` assigns to signal `{spelling}`"))
                        .with_code("V0405")
                        .with_label(span, "a function must not assign a signal")
                        .with_secondary(dspan, "declared here")
                        .with_note("a function computes a value; use a procedure to drive signals"),
                );
            }
        } else if !matches!(
            obj.class,
            ObjectClass::Variable | ObjectClass::SharedVariable | ObjectClass::Constant
        ) {
            let word = obj.class.as_str();
            self.push(
                Diagnostic::error(format!(
                    "`:=` assigns a variable, but `{spelling}` is a {word}"
                ))
                .with_code("V0401")
                .with_label(span, format!("a {word}"))
                .with_secondary(dspan, format!("declared as a {word} here"))
                .with_note("use `<=` to assign a signal"),
            );
        }
    }

    // --- choices ------------------------------------------------------------

    /// The selector of a `case` must be a discrete type or a
    /// one-dimensional array of characters, and locally static enough for
    /// the choices to cover it (clause 10.9).
    fn resolve_case_selector(&mut self, e: &Expr) -> TypeId {
        let ty = self.resolve_free(e);
        if self.a.is_error(ty) {
            return ty;
        }
        let ok = self.a.is_discrete(ty)
            || (self.a.class(ty) == TypeClass::Array && self.a.dimensions(ty) == 1);
        if !ok {
            let tn = self.ty_name(ty);
            self.error(
                "V0408",
                e.span(),
                format!("`{tn}` cannot be a case selector: it is neither discrete nor a one-dimensional array"),
            );
        }
        ty
    }

    /// Checks a choice list: static choices of the right type, no
    /// duplicates, `others` last, and full coverage of a static discrete
    /// subtype.
    fn check_choices(&mut self, arms: &[&[Choice]], ty: TypeId, span: Span, what: &str) {
        if self.a.is_error(ty) {
            return;
        }
        let mut seen: Vec<(i128, Span)> = Vec::new();
        let mut others: Option<Span> = None;
        let mut non_static = false;
        for (ai, arm) in arms.iter().enumerate() {
            for c in arm.iter() {
                if let Some(o) = others {
                    self.push(
                        Diagnostic::error(format!("this choice follows `others` in a {what}"))
                            .with_code("V0408")
                            .with_label(c.span(), "unreachable")
                            .with_secondary(o, "`others` here"),
                    );
                    continue;
                }
                match c {
                    Choice::Others(s) => {
                        if ai + 1 != arms.len() {
                            self.error(
                                "V0408",
                                *s,
                                format!("`others` must be the last alternative of a {what}"),
                            );
                        }
                        others = Some(*s);
                    }
                    Choice::Expr(e) => {
                        self.resolve(e, ty);
                        match self.a.value_of(e.span()).and_then(Value::as_int) {
                            Some(v) => {
                                if let Some((_, prev)) = seen.iter().find(|(x, _)| *x == v) {
                                    let prev = *prev;
                                    let shown = self
                                        .a
                                        .value_of(e.span())
                                        .map(|val| self.a.describe_value(val, ty))
                                        .unwrap_or_default();
                                    self.push(
                                        Diagnostic::error(format!(
                                            "choice `{shown}` appears twice in this {what}"
                                        ))
                                        .with_code("V0408")
                                        .with_label(e.span(), "duplicate choice")
                                        .with_secondary(prev, "first used here"),
                                    );
                                } else {
                                    seen.push((v, e.span()));
                                }
                            }
                            None => {
                                if self.a.value_of(e.span()).is_none() {
                                    non_static = true;
                                    self.error(
                                        "V0408",
                                        e.span(),
                                        format!("the choices of a {what} must be locally static"),
                                    );
                                }
                            }
                        }
                    }
                    Choice::Range(r) => {
                        let info = self.resolve_discrete_range_in(r, Some(ty));
                        match info.bounds.low_high() {
                            Some((lo, hi)) => {
                                for v in lo..=hi {
                                    if let Some((_, prev)) = seen.iter().find(|(x, _)| *x == v) {
                                        let prev = *prev;
                                        self.push(
                                            Diagnostic::error(format!(
                                                "this range overlaps an earlier choice in this {what}"
                                            ))
                                            .with_code("V0408")
                                            .with_label(r.span(), "overlapping choice")
                                            .with_secondary(prev, "first used here"),
                                        );
                                        break;
                                    }
                                    seen.push((v, r.span()));
                                }
                            }
                            None => non_static = true,
                        }
                    }
                }
            }
        }
        if others.is_some() || non_static || !self.a.is_discrete(ty) {
            return;
        }
        // Coverage of a static discrete subtype.
        let Some(range) = self.a.scalar_range(ty) else {
            return;
        };
        let Some((lo, hi)) = range.low_high() else {
            return;
        };
        if hi - lo > 4096 {
            return;
        }
        let missing: Vec<i128> = (lo..=hi)
            .filter(|v| !seen.iter().any(|(x, _)| x == v))
            .collect();
        if missing.is_empty() {
            return;
        }
        let tn = self.ty_name(ty);
        let shown: Vec<String> = missing
            .iter()
            .take(4)
            .map(|&v| {
                let val = if self.a.class(ty) == TypeClass::Enum {
                    Value::Enum(u32::try_from(v).unwrap_or(0))
                } else {
                    Value::Int(v)
                };
                self.a.describe_value(&val, ty)
            })
            .collect();
        let more = if missing.len() > shown.len() {
            format!(" and {} more", missing.len() - shown.len())
        } else {
            String::new()
        };
        self.push(
            Diagnostic::error(format!("this {what} does not cover every value of `{tn}`"))
                .with_code("V0408")
                .with_label(span, format!("missing: {}{more}", shown.join(", ")))
                .with_note("add the missing alternatives, or a `when others =>` alternative"),
        );
    }

    /// True when an expression is static enough for a generate condition:
    /// it has a value, or every name in it is a generic or a constant.
    fn is_globally_static(&mut self, e: &Expr) -> bool {
        if self.a.value_of(e.span()).is_some() {
            return true;
        }
        match e {
            Expr::Name(n) => {
                let info = self.name_info(n);
                info.obj.is_some_and(|o| o.class == ObjectClass::Constant)
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.is_globally_static(lhs) && self.is_globally_static(rhs)
            }
            Expr::Unary { operand, .. } | Expr::Paren { inner: operand, .. } => {
                self.is_globally_static(operand)
            }
            Expr::Literal(_) => true,
            _ => false,
        }
    }
}

fn mode_word(m: Mode) -> &'static str {
    match m {
        Mode::In => "in",
        Mode::Out => "out",
        Mode::Inout => "inout",
        Mode::Buffer => "buffer",
        Mode::Linkage => "linkage",
    }
}
