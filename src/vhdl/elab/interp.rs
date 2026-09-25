//! Static interpretation of a function body at elaboration.
//!
//! [`super::eval`] folds expressions once the generics are bound, but it
//! stops at a call to a user-defined function: those are *inlined* into the
//! IR by [`super::stmt`], which is the right answer for a call in a process
//! and no answer at all for one in a place that has to be a number before
//! any hardware exists —
//!
//! ```vhdl
//! generic (g_MODULO : natural := 16;
//!          g_WIDTH  : natural := log2ceil(g_MODULO));
//! ...
//! signal count : unsigned(g_WIDTH - 1 downto 0);
//! ```
//!
//! A library that computes its widths with its own helper functions (which
//! is the usual way to write portable VHDL, `log2ceil` being the canonical
//! example) cannot be elaborated at all without evaluating them. This
//! module is that evaluator: given a function whose arguments are all
//! static, it interprets the body over [`Value`]s.
//!
//! # What it interprets
//!
//! Variable and constant declarations, variable assignment (to a whole
//! object, an element, a slice or a record field), `if`, `case`, `for`,
//! `while` and bare `loop` with `exit` and `next`, `return`, `null`, and
//! nested calls to other interpretable functions. Expressions are
//! [`Lowerer::eval`]'s job, so anything it folds is available here,
//! including the bundled `numeric_std` and `std_logic_1164` operations.
//!
//! Anything else — a signal read, a file, an access value, a procedure
//! call, an `assert` that fails — yields `None`, and the caller reports its
//! usual "not static after elaboration" error at the right span. The
//! interpreter never reports a diagnostic of its own: it is a fast path,
//! and a `None` from it is not by itself a defect in the source.
//!
//! # Termination
//!
//! A VHDL function can loop for ever (`while true loop`), and elaboration
//! must not. Every statement executed costs one step out of
//! [`STEP_BUDGET`], and the interpretation gives up when the budget runs
//! out, as it does past [`DEPTH_LIMIT`] nested calls. The budget is shared
//! by one outermost call and everything it calls, so a pathological library
//! cannot make elaboration take longer than a bounded amount of work per
//! expression in the source.

use std::collections::HashMap;

use crate::vhdl::ast;
use crate::vhdl::ast::Mode;
use crate::vhdl::sema::constant::ArrayValue;
use crate::vhdl::sema::{CallTarget, DeclId, DeclKind, SubprogramBody, TypeId, Value};

use super::lower::{Binding, Lowerer};

/// Statements one interpretation may execute.
const STEP_BUDGET: u32 = 500_000;

/// Nested interpreted calls allowed.
const DEPTH_LIMIT: usize = 16;

/// Why a statement list stopped.
enum Flow {
    /// It ran to the end.
    Normal,
    /// `return`, with the value of a function's return expression.
    Return(Option<Value>),
    /// `exit`, for the innermost loop or the labelled one.
    Exit(Option<String>),
    /// `next`, for the innermost loop or the labelled one.
    Next(Option<String>),
}

impl<'a> Lowerer<'a, '_> {
    /// Evaluates a call to `d` by interpreting its body, or `None` when it
    /// cannot be interpreted.
    ///
    /// `args` are the call's association elements, matched to the formals
    /// positionally and by name. Every formal must be of mode `in` and
    /// every actual must be static.
    pub(crate) fn eval_static_call(
        &mut self,
        d: DeclId,
        args: &'a [ast::AssociationElement],
    ) -> Option<Value> {
        if self.interp_depth >= DEPTH_LIMIT {
            return None;
        }
        let d = self.subprogram_with_body(super::lower::resolve_alias(self.a(), d));
        let DeclKind::Subprogram { body, .. } = self.a().decl(d).kind.clone() else {
            return None;
        };
        let SubprogramBody::Vhdl(bspan) = body else {
            return None;
        };
        let def = self.cx.ast.body(bspan)?;
        if def.spec.kind != ast::SubprogramKind::Function {
            return None;
        }

        // Bind the formals in a scope of their own. Every write during the
        // interpretation goes into this one scope, whatever nesting the
        // body has, so that an assignment in a loop is not lost when the
        // loop ends.
        let formals = super::lower::interface_objects(&def.spec.params);
        let mut frame: HashMap<DeclId, Binding> = HashMap::new();
        let mut taken = vec![false; args.len()];
        for (i, (ident, obj)) in formals.iter().enumerate() {
            let fd = self.cx.object_decl_at(ident.span)?;
            let DeclKind::Object { ty, mode, .. } = self.a().decl(fd).kind else {
                return None;
            };
            // A parameter of any other mode would have to be copied back
            // out, which a static call has nowhere to put.
            if !matches!(mode, None | Some(Mode::In)) {
                return None;
            }
            let actual = args
                .iter()
                .position(|el| match &el.formal {
                    Some(f) => self.formal_names(f, fd),
                    None => false,
                })
                .or_else(|| (i < args.len() && args[i].formal.is_none()).then_some(i));
            let value = match actual {
                Some(k) => {
                    taken[k] = true;
                    let ast::Actual::Expr(e) = &args[k].actual else {
                        return None;
                    };
                    self.eval(e)?
                }
                // Left out: its default, which must be static too.
                None => self.eval(obj.default.as_ref()?)?,
            };
            frame.insert(fd, Binding::Value { value, ty });
        }
        // An actual nothing claimed means the match is not understood.
        if taken.iter().any(|t| !t) {
            return None;
        }

        self.interp_depth += 1;
        self.steps = self.steps.saturating_add(1);
        let base = self.scopes.len();
        self.scopes.push(frame);
        let result = self.run_body(base, def);
        self.scopes.truncate(base);
        self.interp_depth -= 1;
        if self.interp_depth == 0 {
            self.steps = 0;
        }
        result
    }

    /// True when the formal part `f` of an association names `fd` and
    /// nothing else (a whole formal, not `p(0)` or `p.field`).
    fn formal_names(&self, f: &ast::Expr, fd: DeclId) -> bool {
        matches!(f, ast::Expr::Name(ast::Name::Simple(i))
            if self.a().decl_of(i.span) == Some(fd))
    }

    /// Runs the declarations and statements of one call.
    fn run_body(&mut self, base: usize, def: &'a ast::SubprogramBody) -> Option<Value> {
        for decl in &def.decls {
            self.interp_declaration(base, decl)?;
        }
        match self.interp_statements(base, &def.statements)? {
            Flow::Return(v) => v,
            // Falling off the end of a function is an error at run time,
            // not a value; leave it to the inlining path to report.
            _ => None,
        }
    }

    /// Declares one local of an interpreted body.
    fn interp_declaration(&mut self, base: usize, decl: &'a ast::Declaration) -> Option<()> {
        match decl {
            ast::Declaration::Object(o) => {
                if !matches!(
                    o.kind,
                    ast::ObjectKind::Variable | ast::ObjectKind::Constant
                ) {
                    return None;
                }
                for name in &o.names {
                    let d = self.cx.object_decl_at(name.span)?;
                    let ty = match self.a().decl(d).kind {
                        DeclKind::Object { ty, .. } => ty,
                        _ => return None,
                    };
                    let value = match &o.init {
                        Some(e) => self.eval(e)?,
                        None => self.default_value(ty)?,
                    };
                    self.set_local(base, d, value, ty);
                }
                Some(())
            }
            // Types, subtypes, aliases and use clauses were resolved by the
            // analyser and need nothing here; a nested body is only reached
            // if it is called, and then through `eval_static_call`.
            ast::Declaration::Type(_)
            | ast::Declaration::Subtype(_)
            | ast::Declaration::Use(_)
            | ast::Declaration::Attribute(_)
            | ast::Declaration::AttributeSpec(_)
            | ast::Declaration::Subprogram(_)
            | ast::Declaration::SubprogramBody(_) => Some(()),
            _ => None,
        }
    }

    /// The initial value of an uninitialised object: `T'left` for a scalar,
    /// and every element's for a constrained array of scalars (clause 6.4.2.3).
    fn default_value(&mut self, ty: TypeId) -> Option<Value> {
        let a = self.a();
        if a.is_scalar(ty) {
            let bounds = a.scalar_range(ty)?;
            return match bounds.left.value() {
                Some(v) => Some(v.clone()),
                // A bound that depends on a generic or a parameter, which
                // is the usual case for a local of a function that sizes
                // itself from its arguments.
                None => Some(Value::Int(i128::from(self.bound_value(&bounds.left)?))),
            };
        }
        let bounds = a.index_constraint(ty)?;
        let [b] = bounds.as_slice() else { return None };
        let (l, r) = match b.ints() {
            Some(pair) => pair,
            None => (
                i128::from(self.bound_value(&b.left)?),
                i128::from(self.bound_value(&b.right)?),
            ),
        };
        let b = b.clone();
        let elem = self.default_value(self.a().element_type(ty)?)?;
        let len = match b.dir {
            ast::Direction::To => r - l + 1,
            ast::Direction::Downto => l - r + 1,
        };
        let len = usize::try_from(len.max(0)).ok()?;
        Some(Value::Array(ArrayValue {
            left: l,
            dir: b.dir,
            elems: vec![elem; len],
        }))
    }

    /// Writes a local's value into the frame at `base`.
    fn set_local(&mut self, base: usize, d: DeclId, value: Value, ty: TypeId) {
        if let Some(scope) = self.scopes.get_mut(base) {
            scope.insert(d, Binding::Value { value, ty });
        }
    }

    /// The current value of a local, if it has one.
    fn local(&self, base: usize, d: DeclId) -> Option<(Value, TypeId)> {
        match self.scopes.get(base)?.get(&d)? {
            Binding::Value { value, ty } => Some((value.clone(), *ty)),
            _ => None,
        }
    }

    fn interp_statements(
        &mut self,
        base: usize,
        stmts: &'a [ast::SequentialStatement],
    ) -> Option<Flow> {
        for s in stmts {
            match self.interp_statement(base, s)? {
                Flow::Normal => {}
                other => return Some(other),
            }
        }
        Some(Flow::Normal)
    }

    #[allow(clippy::too_many_lines, reason = "one arm per statement kind")]
    fn interp_statement(&mut self, base: usize, s: &'a ast::SequentialStatement) -> Option<Flow> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > STEP_BUDGET {
            return None;
        }

        match &s.kind {
            ast::SequentialKind::Null => Some(Flow::Normal),
            ast::SequentialKind::Return(e) => {
                let v = match e {
                    Some(e) => Some(self.eval(e)?),
                    None => None,
                };
                Some(Flow::Return(v))
            }
            ast::SequentialKind::VariableAssignment(va) => {
                let ast::VariableAssignmentRhs::Simple(rhs) = &va.rhs else {
                    return None;
                };
                let value = self.eval(rhs)?;
                let ast::Target::Name(name) = &va.target else {
                    return None;
                };
                self.assign_target(base, name, value)?;
                Some(Flow::Normal)
            }
            ast::SequentialKind::If(stmt) => {
                for arm in &stmt.arms {
                    if self.condition(&arm.condition)? {
                        return self.interp_statements(base, &arm.statements);
                    }
                }
                match &stmt.else_statements {
                    Some(stmts) => self.interp_statements(base, stmts),
                    None => Some(Flow::Normal),
                }
            }
            ast::SequentialKind::Case(stmt) => {
                if stmt.matching {
                    return None;
                }
                let selector = self.eval(&stmt.expr)?;
                let mut fallback = None;
                for arm in &stmt.arms {
                    for c in &arm.choices {
                        match c {
                            ast::Choice::Others(_) => fallback = Some(&arm.statements),
                            ast::Choice::Expr(e) => {
                                if self.eval(e)? == selector {
                                    return self.interp_statements(base, &arm.statements);
                                }
                            }
                            ast::Choice::Range(r) => {
                                let (l, hi) = self.static_range(r)?;
                                let v = i64::try_from(selector.as_int()?).ok()?;
                                let (lo, hi) = if l <= hi { (l, hi) } else { (hi, l) };
                                if v >= lo && v <= hi {
                                    return self.interp_statements(base, &arm.statements);
                                }
                            }
                        }
                    }
                }
                match fallback {
                    Some(stmts) => self.interp_statements(base, stmts),
                    None => Some(Flow::Normal),
                }
            }
            ast::SequentialKind::Loop(stmt) => self.interp_loop(base, s.label.as_ref(), stmt),
            ast::SequentialKind::Exit { label, condition } => {
                if !self.when(condition)? {
                    return Some(Flow::Normal);
                }
                Some(Flow::Exit(label.as_ref().map(|l| l.name.clone())))
            }
            ast::SequentialKind::Next { label, condition } => {
                if !self.when(condition)? {
                    return Some(Flow::Normal);
                }
                Some(Flow::Next(label.as_ref().map(|l| l.name.clone())))
            }
            ast::SequentialKind::Assertion(assertion) => {
                // A failing assertion in a function evaluated at
                // elaboration is a real message the inlining path reports
                // with its span, so give up rather than swallow it.
                if self.condition(&assertion.condition)? {
                    Some(Flow::Normal)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// One `while`, `for` or bare `loop`.
    fn interp_loop(
        &mut self,
        base: usize,
        label: Option<&ast::Ident>,
        stmt: &'a ast::LoopStatement,
    ) -> Option<Flow> {
        let mine = |l: &Option<String>| match (l, label) {
            (None, _) => true,
            (Some(want), Some(have)) => want.eq_ignore_ascii_case(&have.name),
            (Some(_), None) => false,
        };
        // A `for` binds its parameter in the same frame as everything else;
        // the analyser gave it a declaration of its own.
        let mut param = None;
        let mut values: Vec<i128> = Vec::new();
        if let Some(ast::IterationScheme::For { param: p, range }) = &stmt.scheme {
            let (l, r) = self.static_range(range)?;
            let dir = self.range_direction(range);
            let d = self.cx.object_decl_at(p.span)?;
            let ty = self
                .a()
                .type_of(p.span)
                .or_else(|| self.a().range_of(range.span()).map(|info| info.ty))?;
            param = Some((d, ty));
            let (l, r) = (i128::from(l), i128::from(r));
            match dir {
                ast::Direction::To => values.extend(l..=r),
                ast::Direction::Downto => values.extend((r..=l).rev()),
            }
        }
        let mut iteration = 0usize;
        loop {
            self.steps = self.steps.saturating_add(1);
            if self.steps > STEP_BUDGET {
                return None;
            }
            match &stmt.scheme {
                Some(ast::IterationScheme::While(cond)) => {
                    if !self.condition(cond)? {
                        return Some(Flow::Normal);
                    }
                }
                Some(ast::IterationScheme::For { .. }) => {
                    let Some(&v) = values.get(iteration) else {
                        return Some(Flow::Normal);
                    };
                    let (d, ty) = param?;
                    let v = match self.a().class(ty) {
                        crate::vhdl::sema::TypeClass::Enum
                        | crate::vhdl::sema::TypeClass::Physical => {
                            Value::Enum(u32::try_from(v).ok()?)
                        }
                        _ => Value::Int(v),
                    };
                    self.set_local(base, d, v, ty);
                }
                None => {}
            }
            iteration += 1;
            match self.interp_statements(base, &stmt.statements)? {
                Flow::Normal => {}
                Flow::Next(l) if mine(&l) => {}
                Flow::Exit(l) if mine(&l) => return Some(Flow::Normal),
                other => return Some(other),
            }
        }
    }

    /// A condition. Only a `boolean` one is interpreted: the VHDL-2008
    /// `??` conversion of a `std_ulogic` condition is the lowering pass's
    /// business, and getting it subtly wrong here would silently change a
    /// computed width.
    fn condition(&mut self, e: &'a ast::Expr) -> Option<bool> {
        let ty = self.a().type_of(e.span())?;
        if !self.a().is_boolean(ty) {
            return None;
        }
        match self.eval(e)? {
            Value::Enum(p) => Some(p == 1),
            _ => None,
        }
    }

    /// An optional `when` condition: absent means unconditional.
    fn when(&mut self, condition: &'a Option<ast::Expr>) -> Option<bool> {
        match condition {
            Some(c) => self.condition(c),
            None => Some(true),
        }
    }

    /// Assigns `value` to a variable, an element of one, a slice of one or
    /// a field of one.
    fn assign_target(&mut self, base: usize, name: &'a ast::Name, value: Value) -> Option<()> {
        match name {
            ast::Name::Simple(_) => {
                let d = self.a().decl_of(name.span())?;
                let (_, ty) = self.local(base, d)?;
                self.set_local(base, d, value, ty);
                Some(())
            }
            ast::Name::Call { prefix, args, span } => {
                // An index; a call would not be a target.
                if !matches!(self.a().call_of(*span), Some(CallTarget::Index)) {
                    return None;
                }
                let d = self.a().decl_of(prefix.span())?;
                let (current, ty) = self.local(base, d)?;
                let mut arr = current.as_array()?.clone();
                let [arg] = args.as_slice() else { return None };
                let ast::Actual::Expr(ie) = &arg.actual else {
                    return None;
                };
                let i = self.eval_int(ie)?;
                let slot = slot_of(&arr, i)?;
                *arr.elems.get_mut(slot)? = value;
                self.set_local(base, d, Value::Array(arr), ty);
                Some(())
            }
            ast::Name::Slice { prefix, range, .. } => {
                let d = self.a().decl_of(prefix.span())?;
                let (current, ty) = self.local(base, d)?;
                let mut arr = current.as_array()?.clone();
                let (l, r) = self.static_range(range)?;
                let new = value.as_array()?.clone();
                let mut k = 0usize;
                let (mut i, step) = if l <= r { (l, 1) } else { (l, -1) };
                loop {
                    let slot = slot_of(&arr, i128::from(i))?;
                    *arr.elems.get_mut(slot)? = new.elems.get(k)?.clone();
                    if i == r {
                        break;
                    }
                    i += step;
                    k += 1;
                }
                self.set_local(base, d, Value::Array(arr), ty);
                Some(())
            }
            ast::Name::Selected { prefix, suffix, .. } => {
                let ast::Suffix::Designator(ast::Designator::Ident(field)) = suffix else {
                    return None;
                };
                let d = self.a().decl_of(prefix.span())?;
                let (current, ty) = self.local(base, d)?;
                let Value::Record(mut fields) = current else {
                    return None;
                };
                let sym = self.a().interner.get_ci(&field.name)?;
                let index = self
                    .a()
                    .record_fields(ty)?
                    .iter()
                    .position(|f| f.name == sym)?;
                *fields.get_mut(index)? = value;
                self.set_local(base, d, Value::Record(fields), ty);
                Some(())
            }
            _ => None,
        }
    }
}

/// The offset of logical index `i` in an array value.
fn slot_of(arr: &ArrayValue, i: i128) -> Option<usize> {
    let off = match arr.dir {
        ast::Direction::To => i - arr.left,
        ast::Direction::Downto => arr.left - i,
    };
    usize::try_from(off).ok().filter(|&o| o < arr.elems.len())
}
