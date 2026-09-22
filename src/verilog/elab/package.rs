//! Package and compilation-unit elaboration.
//!
//! A package is elaborated once, on first use, into a scope under `$unit`
//! holding its parameters, types and subroutines. Variables declared in a
//! package have no elaboration-time meaning for a synthesisable design and
//! are skipped. `export` is accepted and ignored: since a package's scope
//! is searched directly, re-exported names are visible either way.
//!
//! The compilation-unit scope (`$unit`) is elaborated once at the start of
//! a run from the items that sit outside any module or package, so a file
//! may declare shared typedefs and functions at the top level.

use crate::verilog::ast::ItemKind;

use super::decls::{self, Overrides};
use super::env::{Context, ScopeEnv};
use super::scope::{ScopeId, Symbol};

/// Elaborates package `name` and returns its scope, reusing the scope when
/// the package has already been elaborated.
pub(crate) fn elaborate_package<'ast>(cx: &mut Context<'ast>, name: &str) -> ScopeId {
    if let Some(&id) = cx.packages.get(name) {
        return id;
    }
    let decl = cx.table.packages[name];
    let scope = cx.scopes.push(Some(cx.unit), "");
    cx.packages.insert(name.to_owned(), scope);
    cx.packages_in_progress.push(name.to_owned());
    let mut env = ScopeEnv::new(cx, scope, decl.dialect);
    items(&mut env, &decl.def.items);
    let cx = env.cx;
    cx.packages_in_progress.pop();
    // The package is also visible by name for `pkg::x` written without an
    // import, which resolve_scoped handles through the table; recording it
    // in `$unit` additionally allows `pkg::x` to resolve as a path member.
    cx.scopes.redeclare(
        cx.unit,
        name.to_owned(),
        Symbol::Package(scope),
        decl.def.name.span,
    );
    scope
}

/// Elaborates the compilation-unit items of a run into `$unit`.
pub(crate) fn elaborate_unit(cx: &mut Context<'_>) {
    let unit_items: Vec<_> = cx.table.unit_items.clone();
    for (item, dialect) in unit_items {
        let unit = cx.unit;
        let mut env = ScopeEnv::new(cx, unit, dialect);
        items(&mut env, std::slice::from_ref(item));
    }
}

/// Declares the constant, type and subroutine items of `items` in the
/// current scope.
pub(crate) fn items<'ast>(env: &mut ScopeEnv<'_, 'ast>, items: &'ast [crate::verilog::ast::Item]) {
    let mut no_overrides = Overrides::new();
    for item in items {
        match &item.kind {
            ItemKind::Import(refs) => decls::import(env, refs),
            ItemKind::Export(_) => {}
            ItemKind::Param(pd) => {
                decls::param(env, pd, &mut no_overrides);
            }
            ItemKind::Typedef(td) => decls::typedef(env, td),
            ItemKind::Function(f) => decls::subroutine(env, f, false),
            ItemKind::Task(t) => decls::subroutine(env, t, true),
            ItemKind::Var(_) | ItemKind::Net(_) | ItemKind::Empty | ItemKind::Directive(_) => {}
            ItemKind::Timeunit { .. } | ItemKind::Timeprecision(_) => {}
            _ => env.unsupported(item.span, "this declaration outside a module"),
        }
    }
}
