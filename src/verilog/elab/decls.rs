//! Declarations that every scope kind shares: imports, parameters,
//! typedefs and subroutines.
//!
//! Modules, packages and the compilation-unit scope all declare these, so
//! the code lives here and the module lowering adds only what needs a
//! [`ModuleBuilder`](crate::ir::builder::ModuleBuilder): nets, memories and
//! behaviour.
//!
//! Parameters are evaluated in declaration order, in the scope they belong
//! to, so later ones may use earlier ones (`localparam AW = $clog2(DEPTH)`).
//! An override supplied by an instantiation replaces the default *before*
//! the declaration is evaluated, and is converted to the declared type;
//! an override of a `localparam` is refused, as the standard requires.

use crate::diag::Diagnostic;
use crate::source::Span;
use crate::verilog::ast::{self, ParamKind};

use super::codes;
use super::constant::{Evaluator, Value};
use super::env::ScopeEnv;
use super::scope::{FnRef, Symbol};
use super::types::{self, VType};

/// What an instantiation overrides a parameter with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Override {
    /// A value parameter.
    Value(Value),
    /// A `parameter type` override.
    Type(VType),
}

impl Override {
    /// The text used in a uniquified module name.
    pub(crate) fn key_text(&self) -> String {
        match self {
            Override::Value(v) => v.key_text(),
            Override::Type(t) => t.to_string(),
        }
    }
}

/// The parameter overrides applied to one module instantiation, with a
/// used flag per entry so unknown names can be reported afterwards.
#[derive(Clone, Debug, Default)]
pub(crate) struct Overrides {
    items: Vec<(String, Override, Span, bool)>,
}

impl Overrides {
    /// An empty override set.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds an override.
    pub(crate) fn push(&mut self, name: impl Into<String>, value: Override, span: Span) {
        self.items.push((name.into(), value, span, false));
    }

    /// True when nothing is overridden.
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The override for `name`, marking it used.
    pub(crate) fn take(&mut self, name: &str) -> Option<Override> {
        self.items.iter_mut().find(|(n, ..)| n == name).map(|e| {
            e.3 = true;
            e.1.clone()
        })
    }

    /// The entries in order, with their spans and whether a parameter of
    /// that name was found.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, &Override, Span, bool)> {
        self.items
            .iter()
            .map(|(n, v, s, used)| (n.as_str(), v, *s, *used))
    }

    /// Gives positional overrides, recorded as `$0`, `$1`, ..., the name
    /// of the parameter at that position.
    ///
    /// Returns the span and index of every position that has no parameter.
    pub(crate) fn name_positional(&mut self, names: &[String]) -> Vec<(Span, usize)> {
        let mut bad = Vec::new();
        for (name, _, span, _) in &mut self.items {
            let Some(index) = name.strip_prefix('$').and_then(|i| i.parse::<usize>().ok()) else {
                continue;
            };
            match names.get(index) {
                Some(param) => *name = param.clone(),
                None => bad.push((*span, index)),
            }
        }
        self.items.retain(|(name, ..)| !name.starts_with('$'));
        bad
    }

    /// The `name=value` pairs that were applied, for the uniquified module
    /// name and for the IR's parameter metadata.
    pub(crate) fn applied(&self) -> Vec<(String, Override)> {
        self.items
            .iter()
            .filter(|(.., used)| *used)
            .map(|(n, v, ..)| (n.clone(), v.clone()))
            .collect()
    }
}

/// Applies `import pkg::*` and `import pkg::name` clauses.
pub(crate) fn import(env: &mut ScopeEnv<'_, '_>, refs: &[ast::PackageRef]) {
    for r in refs {
        let Some(scope) = env.package(&r.package) else {
            continue;
        };
        match &r.item {
            None => {
                let current = env.scope;
                env.cx.scopes.import_all(current, scope);
            }
            Some(name) => {
                let sym = env
                    .cx
                    .scopes
                    .get(scope)
                    .get(&name.name)
                    .map(|(s, _)| s.clone());
                match sym {
                    Some(sym) => env.declare(&name.name, sym, name.span),
                    None => env.error(
                        codes::PACKAGE,
                        name.span,
                        format!("package `{}` has no member `{}`", r.package.name, name.name),
                    ),
                }
            }
        }
    }
}

/// Declares the parameters of one `parameter` / `localparam` declaration.
///
/// `overrides` is consulted for `parameter` declarations only. Returns the
/// names and values that were declared, in order, for the IR's parameter
/// metadata.
pub(crate) fn param(
    env: &mut ScopeEnv<'_, '_>,
    pd: &ast::ParamDecl,
    overrides: &mut Overrides,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let overridable = pd.kind == ParamKind::Parameter;
    for d in &pd.decls {
        if pd.is_type {
            let over = overridable.then(|| overrides.take(&d.name.name)).flatten();
            let ty = match over {
                Some(Override::Type(t)) => Some(t),
                Some(Override::Value(v)) => {
                    env.error(
                        codes::TYPE,
                        d.span,
                        format!("`{}` is a type parameter; `{v}` is not a type", d.name.name),
                    );
                    None
                }
                None => match &d.init {
                    Some(init) => type_of_type_expr(env, init),
                    None => {
                        env.error(
                            codes::CONST_EVAL,
                            d.span,
                            format!("type parameter `{}` has no default", d.name.name),
                        );
                        None
                    }
                },
            };
            if let Some(ty) = ty {
                env.declare(&d.name.name, Symbol::Type(ty), d.name.span);
            }
            continue;
        }

        // The declared type, if one was written.
        let declared = if pd.data_type.is_empty() && d.dims.is_empty() {
            None
        } else {
            types::resolve(env, &pd.data_type, &d.dims, true)
        };
        let over = overridable.then(|| overrides.take(&d.name.name)).flatten();
        let value = match over {
            Some(Override::Value(v)) => match &declared {
                Some(ty) => {
                    let mut ev = Evaluator::new(env);
                    ev.fit(v.clone(), ty, d.span).unwrap_or(v)
                }
                None => v,
            },
            Some(Override::Type(t)) => {
                env.error(
                    codes::TYPE,
                    d.span,
                    format!("`{}` is a value parameter; `{t}` is a type", d.name.name),
                );
                continue;
            }
            None => {
                let Some(init) = &d.init else {
                    env.error(
                        codes::CONST_EVAL,
                        d.span,
                        format!("parameter `{}` has no default value", d.name.name),
                    );
                    continue;
                };
                let mut ev = Evaluator::new(env);
                let r = match &declared {
                    Some(ty) => ev.eval_as(init, ty),
                    None => ev.eval_self(init),
                };
                match r {
                    Ok(v) => v,
                    // The value could not be evaluated and the reason has
                    // been reported; the parameter is still declared, as
                    // an unknown, so every later use of it does not
                    // produce a second cascade of errors.
                    Err(_) => Value::Logic(crate::logic::Logic::x(32).with_signed(true)),
                }
            }
        };
        env.declare(
            &d.name.name,
            Symbol::Const {
                value: value.clone(),
                ty: declared,
            },
            d.name.span,
        );
        out.push((d.name.name.clone(), value));
    }
    out
}

/// The type an expression in type-parameter position denotes.
fn type_of_type_expr(env: &mut ScopeEnv<'_, '_>, e: &ast::Expr) -> Option<VType> {
    match &e.kind {
        ast::ExprKind::Type(dt) => types::resolve(env, dt, &[], true),
        ast::ExprKind::Ident(_) | ast::ExprKind::Scoped { .. } => match env.resolve_path(e) {
            Some(Symbol::Type(t)) => Some(t),
            Some(_) => {
                env.error(codes::TYPE, e.span, "this name is not a type");
                None
            }
            None => None,
        },
        _ => {
            env.error(codes::TYPE, e.span, "a type is required here");
            None
        }
    }
}

/// Declares a `typedef`.
pub(crate) fn typedef(env: &mut ScopeEnv<'_, '_>, td: &ast::Typedef) {
    let Some(dt) = &td.data_type else {
        // A forward declaration (`typedef enum name;`) introduces nothing;
        // the real declaration follows.
        return;
    };
    if let Some(ty) = types::resolve(env, dt, &td.dims, true) {
        env.declare(&td.name.name, Symbol::Type(ty), td.name.span);
    }
}

/// Declares a function or task in the current scope.
pub(crate) fn subroutine<'ast>(
    env: &mut ScopeEnv<'_, 'ast>,
    def: &'ast ast::Subroutine,
    is_task: bool,
) {
    let home = env.scope;
    let r = FnRef { def, home };
    let sym = if is_task {
        Symbol::Task(r)
    } else {
        Symbol::Function(r)
    };
    env.declare(&def.name.name, sym, def.name.span);
}

/// Reports every override that no parameter of the instantiated module
/// accepted (`V0006`).
pub(crate) fn report_unused_overrides(
    env: &mut ScopeEnv<'_, '_>,
    module: &str,
    overrides: &Overrides,
) {
    for (name, _, span, used) in overrides.entries() {
        if !used {
            env.report(
                Diagnostic::error(format!(
                    "module `{module}` has no parameter `{name}` to override"
                ))
                .with_code(codes::PARAM_NOT_FOUND)
                .with_span(span),
            );
        }
    }
}
