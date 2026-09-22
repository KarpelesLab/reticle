//! The elaboration context and the scope-bound environment.
//!
//! [`Context`] holds everything that outlives one module: the design
//! being built, the declaration table, the scope arena, the elaborated
//! packages and the module cache keyed by parameter set. [`ScopeEnv`] is a
//! cursor into it: the context plus the *current* scope, which is what
//! constant evaluation, width inference and lowering all take. It answers
//! "what does this name mean here", resolves hierarchical paths and
//! package references, reports diagnostics (or swallows them while a
//! caller is probing whether an expression is constant), and manages the
//! scope stack when a function body is evaluated in its home scope.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::diag::{Diagnostic, Diagnostics, Severity};
use crate::ir::{Design, ModuleId};
use crate::source::Span;
use crate::verilog::ast::{self, Expr, ExprKind};
use crate::verilog::token::Dialect;

use super::ElabOptions;
use super::codes;
use super::constant::Evaluator;
use super::hier::Table;
use super::scope::{FnRef, ScopeId, Scopes, Symbol};
use super::types::VType;

/// State shared by every module elaboration of one run.
pub(crate) struct Context<'ast> {
    /// The caller's options.
    #[allow(dead_code, reason = "kept for options that later passes will read")]
    pub opts: ElabOptions,
    /// Every diagnostic produced so far.
    pub diags: Diagnostics,
    /// The declarations found in the source files.
    pub table: Table<'ast>,
    /// Every scope of the run.
    pub scopes: Scopes<'ast>,
    /// The compilation-unit scope (`$unit`).
    pub unit: ScopeId,
    /// Elaborated packages by name.
    pub packages: BTreeMap<String, ScopeId>,
    /// Packages whose elaboration has started, to detect import cycles.
    pub packages_in_progress: Vec<String>,
    /// The design under construction.
    pub design: Design,
    /// Elaborated module variants by their uniquified name.
    pub cache: HashMap<String, ModuleId>,
    /// Module names currently being elaborated, outermost first.
    pub in_progress: Vec<String>,
    /// Every module name used in the design, for uniqueness of variants.
    pub module_names: BTreeSet<String>,
    /// The interface ports of each elaborated module, for instantiations.
    pub iface_ports: HashMap<ModuleId, Vec<super::lower::IfacePort>>,
}

impl<'ast> Context<'ast> {
    /// Creates a context over `table` with an empty `$unit` scope.
    pub(crate) fn new(table: Table<'ast>, opts: ElabOptions) -> Self {
        let mut scopes = Scopes::new();
        let unit = scopes.push(None, "");
        Context {
            opts,
            diags: Diagnostics::new(),
            table,
            scopes,
            unit,
            packages: BTreeMap::new(),
            packages_in_progress: Vec::new(),
            design: Design::new(),
            cache: HashMap::new(),
            in_progress: Vec::new(),
            module_names: BTreeSet::new(),
            iface_ports: HashMap::new(),
        }
    }
}

/// A context plus the scope names currently resolve in.
pub(crate) struct ScopeEnv<'cx, 'ast> {
    /// The shared state.
    pub cx: &'cx mut Context<'ast>,
    /// The current scope.
    pub scope: ScopeId,
    /// Scopes to return to; see [`ScopeEnv::enter`].
    stack: Vec<ScopeId>,
    /// While positive, diagnostics are dropped: a caller is probing.
    quiet: u32,
    /// The dialect the current module was written in.
    pub dialect: Dialect,
}

impl<'cx, 'ast> ScopeEnv<'cx, 'ast> {
    /// Creates an environment positioned at `scope`.
    pub(crate) fn new(cx: &'cx mut Context<'ast>, scope: ScopeId, dialect: Dialect) -> Self {
        ScopeEnv {
            cx,
            scope,
            stack: Vec::new(),
            quiet: 0,
            dialect,
        }
    }

    // --- diagnostics -------------------------------------------------------

    /// Records a diagnostic unless probing.
    pub(crate) fn report(&mut self, d: Diagnostic) {
        if self.quiet == 0 {
            self.cx.diags.push(d);
        }
    }

    /// Records an error with a code and one primary span.
    pub(crate) fn error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.report(Diagnostic::error(message).with_code(code).with_span(span));
    }

    /// Records a warning with a code and one primary span.
    pub(crate) fn warning(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.report(Diagnostic::warning(message).with_code(code).with_span(span));
    }

    /// Reports `what` as unsupported.
    pub(crate) fn unsupported(&mut self, span: Span, what: &str) {
        self.report(
            Diagnostic::error(format!("unsupported construct: {what}"))
                .with_code(codes::UNSUPPORTED)
                .with_label(span, "not supported by this version of reticle"),
        );
    }

    /// Runs `f` with diagnostics suppressed.
    pub(crate) fn probe<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.quiet += 1;
        let r = f(self);
        self.quiet -= 1;
        r
    }

    /// True while diagnostics are suppressed.
    pub(crate) fn is_quiet(&self) -> bool {
        self.quiet > 0
    }

    /// Records an error at severity `sev`, for rules whose severity
    /// depends on the dialect.
    pub(crate) fn report_at(
        &mut self,
        sev: Severity,
        code: &'static str,
        span: Span,
        message: impl Into<String>,
    ) {
        self.report(
            Diagnostic::new(sev, message)
                .with_code(code)
                .with_span(span),
        );
    }

    // --- scopes ------------------------------------------------------------

    /// Creates a child of the current scope and makes it current.
    pub(crate) fn enter_new(&mut self, prefix: impl Into<String>) -> ScopeId {
        let id = self.cx.scopes.push(Some(self.scope), prefix);
        self.enter(id);
        id
    }

    /// Makes `scope` current, remembering the previous one.
    pub(crate) fn enter(&mut self, scope: ScopeId) {
        self.stack.push(self.scope);
        self.scope = scope;
    }

    /// Returns to the scope that was current before the last
    /// [`enter`](Self::enter) or [`enter_new`](Self::enter_new).
    pub(crate) fn leave(&mut self) {
        if let Some(prev) = self.stack.pop() {
            self.scope = prev;
        }
    }

    /// Declares `name` in the current scope, reporting a duplicate.
    pub(crate) fn declare(&mut self, name: &str, sym: Symbol<'ast>, span: Span) {
        if let Some(first) = self.cx.scopes.declare(self.scope, name, sym, span) {
            self.report(
                Diagnostic::error(format!("`{name}` is declared twice in this scope"))
                    .with_code(codes::DUPLICATE)
                    .with_label(span, "second declaration")
                    .with_secondary(first, "first declaration"),
            );
        }
    }

    /// The IR name for `name` declared in the current scope.
    pub(crate) fn qualified(&self, name: &str) -> String {
        self.cx.scopes.qualified(self.scope, name)
    }

    // --- name resolution ---------------------------------------------------

    /// Looks `name` up from the current scope without reporting.
    pub(crate) fn lookup(&self, name: &str) -> Option<(&Symbol<'ast>, Span, ScopeId)> {
        self.cx.scopes.lookup(self.scope, name)
    }

    /// Resolves a simple name, reporting an undefined identifier with a
    /// suggestion when it is unknown.
    pub(crate) fn resolve_name(&mut self, id: &ast::Ident) -> Option<Symbol<'ast>> {
        if let Some((sym, _, _)) = self.lookup(&id.name) {
            return Some(sym.clone());
        }
        self.undefined(id);
        None
    }

    /// Reports `V0001` for `id`, with a "did you mean" note when a visible
    /// name is close.
    pub(crate) fn undefined(&mut self, id: &ast::Ident) {
        if self.is_quiet() {
            return;
        }
        let names = self.cx.scopes.visible_names(self.scope);
        let mut d = Diagnostic::error(format!("cannot find `{}` in this scope", id.name))
            .with_code(codes::UNDEFINED)
            .with_label(id.span, "not declared here");
        if let Some(s) = Scopes::suggest(&id.name, names.iter().map(String::as_str)) {
            d = d.with_note(format!("did you mean `{s}`?"));
        }
        self.report(d);
    }

    /// The scope of package `name`, elaborating it on first use.
    pub(crate) fn package(&mut self, name: &ast::Ident) -> Option<ScopeId> {
        if let Some(&id) = self.cx.packages.get(&name.name) {
            return Some(id);
        }
        if !self.cx.table.packages.contains_key(&name.name) {
            self.report(
                Diagnostic::error(format!("package `{}` is not defined", name.name))
                    .with_code(codes::PACKAGE)
                    .with_span(name.span),
            );
            return None;
        }
        if self.cx.packages_in_progress.contains(&name.name) {
            self.report(
                Diagnostic::error(format!(
                    "package `{}` imports itself, directly or through another package",
                    name.name
                ))
                .with_code(codes::PACKAGE)
                .with_span(name.span),
            );
            return None;
        }
        Some(super::package::elaborate_package(self.cx, &name.name))
    }

    /// Resolves a `pkg::name` reference.
    pub(crate) fn resolve_scoped(
        &mut self,
        pkg: &ast::Ident,
        name: &ast::Ident,
    ) -> Option<Symbol<'ast>> {
        if pkg.name == "$unit" {
            if let Some((sym, _)) = self.cx.scopes.get(self.cx.unit).get(&name.name) {
                return Some(sym.clone());
            }
            self.error(
                codes::UNDEFINED,
                name.span,
                format!("`{}` is not declared in the compilation unit", name.name),
            );
            return None;
        }
        let scope = self.package(pkg)?;
        if let Some((sym, _)) = self.cx.scopes.get(scope).get(&name.name) {
            return Some(sym.clone());
        }
        self.error(
            codes::UNDEFINED,
            name.span,
            format!("package `{}` has no member `{}`", pkg.name, name.name),
        );
        None
    }

    /// Resolves a name path: a simple name, `pkg::name`, a member of a
    /// generate block, generate loop iteration or interface instance, or
    /// a nested combination. Returns the symbol, or `None` after reporting.
    pub(crate) fn resolve_path(&mut self, e: &Expr) -> Option<Symbol<'ast>> {
        match &e.kind {
            ExprKind::Ident(id) => self.resolve_name(id),
            ExprKind::Scoped { scope, name } => {
                let ExprKind::Ident(pkg) = &scope.kind else {
                    self.unsupported(e.span, "nested scope resolution");
                    return None;
                };
                self.resolve_scoped(pkg, name)
            }
            ExprKind::Member { base, name } => {
                let base_sym = self.resolve_path(base)?;
                self.member_of(base_sym, base.span, name)
            }
            ExprKind::Index { base, index } => {
                let base_sym = self.resolve_path(base)?;
                match base_sym {
                    Symbol::GenArray(iters) => {
                        let idx = Evaluator::new(self).eval_i64(index).ok()?;
                        match iters.iter().find(|(i, _)| *i == idx) {
                            Some((_, scope)) => Some(Symbol::GenBlock(*scope)),
                            None => {
                                self.error(
                                    codes::HIERARCHY,
                                    index.span,
                                    format!("generate loop has no iteration {idx}"),
                                );
                                None
                            }
                        }
                    }
                    Symbol::Instance { module } => {
                        self.error(
                            codes::HIERARCHY,
                            e.span,
                            format!(
                                "hierarchical reference into instance array of `{module}` is not supported"
                            ),
                        );
                        None
                    }
                    other => Some(other),
                }
            }
            ExprKind::SystemIdent(id) if id.name == "root" => {
                self.unsupported(e.span, "`$root` hierarchical reference");
                None
            }
            _ => None,
        }
    }

    /// The member `name` of the scope-like symbol `base`.
    pub(crate) fn member_of(
        &mut self,
        base: Symbol<'ast>,
        base_span: Span,
        name: &ast::Ident,
    ) -> Option<Symbol<'ast>> {
        let scope = match base {
            Symbol::GenBlock(s) | Symbol::Iface { scope: s, .. } | Symbol::Package(s) => s,
            Symbol::GenArray(_) => {
                self.error(
                    codes::HIERARCHY,
                    base_span,
                    "a generate loop must be indexed before selecting a member",
                );
                return None;
            }
            Symbol::Instance { module } => {
                self.report(
                    Diagnostic::error(format!(
                        "hierarchical reference into instance of `{module}` is not supported"
                    ))
                    .with_code(codes::HIERARCHY)
                    .with_label(name.span, "member of another module")
                    .with_note("only references within the current module and its generate blocks and interfaces are resolved"),
                );
                return None;
            }
            _ => return None,
        };
        if let Some((sym, _)) = self.cx.scopes.get(scope).get(&name.name) {
            return Some(sym.clone());
        }
        let names: Vec<String> = self
            .cx
            .scopes
            .get(scope)
            .names()
            .map(|(n, _)| n.to_owned())
            .collect();
        let mut d = Diagnostic::error(format!("no member `{}` here", name.name))
            .with_code(codes::UNDEFINED)
            .with_span(name.span);
        if let Some(s) = Scopes::suggest(&name.name, names.iter().map(String::as_str)) {
            d = d.with_note(format!("did you mean `{s}`?"));
        }
        self.report(d);
        None
    }

    /// The declared type of a name path, for any kind of object that has
    /// one (net, memory, constant, type name).
    pub(crate) fn type_of_path(&mut self, e: &Expr) -> Option<VType> {
        match self.probe(|env| env.resolve_path(e))? {
            Symbol::Net { ty, .. } | Symbol::Memory { ty, .. } | Symbol::Type(ty) => Some(ty),
            Symbol::Const { ty, value } => Some(ty.unwrap_or_else(|| value.natural_type())),
            _ => None,
        }
    }

    /// Resolves a function or task name path.
    pub(crate) fn resolve_callee(&mut self, e: &Expr) -> Option<(FnRef<'ast>, bool)> {
        let sym = self.probe(|env| env.resolve_path(e));
        match sym {
            Some(Symbol::Function(f)) => Some((f, false)),
            Some(Symbol::Task(f)) => Some((f, true)),
            Some(_) => {
                self.error(
                    codes::UNDEFINED,
                    e.span,
                    "this name is not a function or task",
                );
                None
            }
            None => {
                self.resolve_path(e);
                None
            }
        }
    }
}
