//! Constant expression evaluation.
//!
//! Parameters, dimensions, generate conditions, replication counts and
//! `case` items must be known at elaboration time. `Evaluator` computes
//! them from the AST with `Logic` arithmetic, following the sizing and
//! signing rules of IEEE 1364-2005 §5.4 and §5.5 exactly as the lowering
//! does for run-time expressions, so a constant folded here has the same
//! bits as the simulator would compute:
//!
//! - Every expression has a *self-determined* width from
//!   `super::width::info`. When an expression is evaluated in a context
//!   (the right-hand side of an assignment to a wider target, an operand of
//!   a wider operator), its arithmetic, bitwise and conditional operands are
//!   evaluated at the larger of the two widths *before* the operator is
//!   applied (§5.4.1 step 2), so `a + b` keeps its carry when assigned to a
//!   wider variable.
//! - Operands of shifts' right sides, concatenations, replications,
//!   reductions, logical connectives and the condition of `?:` are
//!   self-determined (§5.4.1 Table 5-22).
//! - Comparison operands are sized to each other but not to the outer
//!   context (§5.4.1).
//! - The expression is signed only when every context-determined operand
//!   is signed (§5.5.1); an unsigned expression zero-extends its operands,
//!   even signed ones, which is why `$signed` and `$unsigned` matter.
//! - Unsized literals are 32 bits (signed for plain decimals); `'0`, `'1`,
//!   `'x`, `'z` fill the context width (IEEE 1800-2017 §5.7.1).
//! - A `real` operand makes an arithmetic operator real (§5.5.1 step 1);
//!   reals are converted to integers by rounding (§4.3.1).
//!
//! Constant functions are interpreted over the same evaluator: formal
//! arguments and locals live in a frame, statements run with a step budget
//! so a runaway loop is reported rather than hung, and a function that
//! reaches itself on the call stack is rejected (`V0012`).
//!
//! `Logic`: crate::logic::Logic

use std::fmt;

use crate::diag::Diagnostic;
use crate::logic::{Bit, Logic};
use crate::source::Span;
use crate::verilog::ast::{
    self, Arg, AssignOp, BinaryOp, CastTarget, DataTypeKind, Direction, Expr, ExprKind, ForInit,
    Literal, Stmt, StmtKind, UnaryOp,
};

use super::codes;
use super::env::ScopeEnv;
use super::scope::{FnRef, Symbol};
use super::types::{self, Packed, Range, VType};
use super::width::{self, Info};

/// The result of evaluating a constant expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// An integral value with a width and signedness.
    Logic(Logic),
    /// A real number.
    Real(f64),
    /// A string.
    Str(String),
    /// An unpacked aggregate from an assignment pattern `'{...}`, used for
    /// array-typed parameters and memory initialisers.
    Array(Vec<Value>),
}

impl Value {
    /// A signed 32-bit integer.
    pub(crate) fn int(v: i64) -> Self {
        Value::Logic(Logic::from_i64(v, 32))
    }

    /// The value as `i64` when it is a fully known integral that fits (or
    /// a real, rounded).
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Logic(l) => {
                if l.is_signed() {
                    l.to_i64()
                } else {
                    l.to_u64().and_then(|u| i64::try_from(u).ok())
                }
            }
            Value::Real(r) => real_to_i64(*r),
            Value::Str(_) | Value::Array(_) => None,
        }
    }

    /// The value as a bit vector: strings become their 8-bit-per-character
    /// packing, reals are rounded (§4.3.1).
    pub(crate) fn to_logic(&self) -> Option<Logic> {
        match self {
            Value::Logic(l) => Some(l.clone()),
            Value::Real(r) => Some(Logic::from_i64(real_to_i64(*r)?, 64).with_signed(true)),
            Value::Str(s) => Some(string_bits(s)),
            Value::Array(_) => None,
        }
    }

    /// The value as a real number.
    pub(crate) fn to_real(&self) -> Option<f64> {
        match self {
            Value::Logic(l) => {
                if l.is_signed() {
                    l.to_i64().map(|i| i as f64)
                } else {
                    l.to_u64().map(|u| u as f64)
                }
            }
            Value::Real(r) => Some(*r),
            Value::Str(_) | Value::Array(_) => None,
        }
    }

    /// The truth value used by `if` and the logical operators.
    pub(crate) fn truth(&self) -> Bit {
        match self {
            Value::Logic(l) => l.truth(),
            Value::Real(r) => Bit::from_bool(*r != 0.0),
            Value::Str(s) => Bit::from_bool(!s.is_empty()),
            Value::Array(_) => Bit::X,
        }
    }

    /// The type a declaration without a type takes from this value: the
    /// bit vector's own width and signedness (flagged unsized), `real` or
    /// `string`.
    pub(crate) fn natural_type(&self) -> VType {
        match self {
            Value::Logic(l) => {
                let mut p = if l.is_signed() {
                    Packed::sbits(l.width())
                } else {
                    Packed::bits(l.width())
                };
                p.is_unsized = true;
                VType::Packed(p)
            }
            Value::Real(_) => VType::Real,
            Value::Str(_) => VType::String,
            Value::Array(items) => VType::Unpacked {
                elem: Box::new(items.first().map_or_else(VType::bit, Value::natural_type)),
                range: Range::unpacked_size(i64::try_from(items.len()).unwrap_or(i64::MAX)),
            },
        }
    }

    /// The value rendered for a module variant name: an integer when it
    /// fits, else a sized literal; strings quoted; reals as written by
    /// Rust.
    pub(crate) fn key_text(&self) -> String {
        match self {
            Value::Logic(l) => match self.as_i64() {
                Some(i) if l.is_fully_known() => i.to_string(),
                _ => l.to_string(),
            },
            Value::Real(r) => format!("{r}"),
            Value::Str(s) => format!("{s:?}"),
            Value::Array(items) => {
                let parts: Vec<String> = items.iter().map(Value::key_text).collect();
                format!("{{{}}}", parts.join(","))
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key_text())
    }
}

/// Rounds a real to the nearest integer (§4.3.1), `None` for values that
/// do not fit or are not finite.
fn real_to_i64(r: f64) -> Option<i64> {
    if !r.is_finite() {
        return None;
    }
    real_to_int(r.round())
}

/// Converts a whole real to an `i64`, or `None` when it does not fit.
///
/// The bounds are the exact `f64` values of `i64::MIN` and `2^63`, so the
/// conversion inside is lossless; it is written out rather than done with
/// `as` on an unchecked value.
fn real_to_int(whole: f64) -> Option<i64> {
    const MIN: f64 = -9_223_372_036_854_775_808.0;
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if !(MIN..LIMIT).contains(&whole) {
        return None;
    }
    // The range check above makes the conversion exact.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is whole and within the i64 range"
    )]
    Some(whole as i64)
}

/// The bit-vector form of a string literal: 8 bits per byte, first
/// character most significant (§3.6.2); an empty string is 8 zero bits.
pub fn string_bits(s: &str) -> Logic {
    if s.is_empty() {
        return Logic::zero(8);
    }
    let parts: Vec<Logic> = s
        .bytes()
        .map(|b| Logic::from_u64(u64::from(b), 8))
        .collect();
    Logic::concat_all(&parts)
}

/// Why an evaluation produced no value. Either way a diagnostic has been
/// reported unless the evaluator is probing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// The expression refers to something that is not a constant (a net,
    /// a system function with run-time meaning).
    NotConstant,
    /// The expression is malformed or undefined.
    Failed,
}

/// The result of an evaluation.
pub type EResult<T> = Result<T, EvalError>;

/// A local variable of an interpreted function.
#[derive(Clone, Debug)]
struct Var {
    name: String,
    ty: VType,
    value: Value,
}

/// The locals of one function activation.
#[derive(Clone, Debug, Default)]
struct Frame {
    vars: Vec<Var>,
}

/// Control flow out of an interpreted statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

/// The maximum number of statements one constant function call may run.
const STEP_LIMIT: u64 = 5_000_000;

/// Evaluates constant expressions in a [`ScopeEnv`].
pub(crate) struct Evaluator<'e, 'cx, 'ast> {
    env: &'e mut ScopeEnv<'cx, 'ast>,
    frames: Vec<Frame>,
    call_stack: Vec<String>,
    steps: u64,
}

/// One formal argument of a function or task.
#[derive(Clone, Copy, Debug)]
pub struct Formal<'ast> {
    /// The argument name.
    pub name: &'ast ast::Ident,
    /// The direction.
    pub dir: Direction,
    /// The declared type.
    pub data_type: &'ast ast::DataType,
    /// Unpacked dimensions.
    pub dims: &'ast [ast::Dim],
    /// The default value.
    pub default: Option<&'ast Expr>,
}

/// The formal arguments of a subroutine, from its ANSI list or from the
/// port declarations in its body, in order.
pub fn formals(def: &ast::Subroutine) -> Vec<Formal<'_>> {
    let mut out = Vec::new();
    if let Some(ports) = &def.ports {
        let mut last_dir = Direction::Input;
        let mut last_type: Option<&ast::DataType> = None;
        for p in ports {
            let dir = p.direction.unwrap_or(last_dir);
            last_dir = dir;
            let data_type = if p.data_type.is_empty() && p.direction.is_none() {
                last_type.unwrap_or(&p.data_type)
            } else {
                &p.data_type
            };
            last_type = Some(data_type);
            out.push(Formal {
                name: &p.name,
                dir,
                data_type,
                dims: &p.dims,
                default: p.default.as_ref(),
            });
        }
    }
    for s in &def.body {
        if let StmtKind::Decl(item) = &s.kind
            && let ast::ItemKind::Port(pd) = &item.kind
        {
            for d in &pd.decls {
                out.push(Formal {
                    name: &d.name,
                    dir: pd.direction,
                    data_type: &pd.data_type,
                    dims: &d.dims,
                    default: d.init.as_ref(),
                });
            }
        }
    }
    out
}

/// Binds call arguments to formals: positional first, then named; missing
/// ones take their default. Returns one slot per formal.
pub fn bind_args<'a, 'ast>(
    formals: &[Formal<'ast>],
    args: &'a [Arg],
) -> Result<Vec<Option<&'a Expr>>, String>
where
    'ast: 'a,
{
    let mut slots: Vec<Option<&'a Expr>> = vec![None; formals.len()];
    let mut pos = 0usize;
    for a in args {
        match &a.name {
            None => {
                if pos >= formals.len() {
                    return Err(format!(
                        "too many arguments: expected {}, found {}",
                        formals.len(),
                        args.len()
                    ));
                }
                slots[pos] = a.value.as_ref();
                pos += 1;
            }
            Some(n) => {
                let Some(i) = formals.iter().position(|f| f.name.name == n.name) else {
                    return Err(format!("no argument named `{}`", n.name));
                };
                slots[i] = a.value.as_ref();
            }
        }
    }
    for (slot, f) in slots.iter_mut().zip(formals) {
        if slot.is_none() {
            *slot = f.default;
        }
    }
    Ok(slots)
}

/// Text form of a numeric literal after classification.
enum Lit {
    /// `'0`, `'1`, `'x`, `'z`: fills the context width.
    Unbased(Bit),
    /// A real number.
    Real(f64),
    /// A time literal with its unit text.
    Time(f64, &'static str),
    /// An integer literal.
    Int(Logic),
}

const TIME_UNITS: [&str; 7] = ["step", "ms", "us", "ns", "ps", "fs", "s"];

/// Classifies a numeric literal's text.
fn classify_literal(text: &str) -> Result<Lit, String> {
    let t = text.trim();
    if t.len() == 2
        && t.starts_with('\'')
        && let Some(b) = t.chars().nth(1).and_then(Bit::from_char)
    {
        return Ok(Lit::Unbased(b));
    }
    if t.contains('\'') {
        return Logic::parse_verilog(t)
            .map(Lit::Int)
            .map_err(|e| e.to_string());
    }
    for unit in TIME_UNITS {
        if let Some(num) = t.strip_suffix(unit)
            && !num.is_empty()
            && num
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'e' | b'E' | b'-' | b'+'))
        {
            let v: f64 = num
                .replace('_', "")
                .parse()
                .map_err(|_| format!("malformed time literal `{t}`"))?;
            return Ok(Lit::Time(v, unit));
        }
    }
    if t.contains(['.', 'e', 'E']) {
        let v: f64 = t
            .replace('_', "")
            .parse()
            .map_err(|_| format!("malformed real literal `{t}`"))?;
        return Ok(Lit::Real(v));
    }
    Logic::parse_verilog(t)
        .map(Lit::Int)
        .map_err(|e| e.to_string())
}

/// Parses a time literal such as `10ns` into its number and unit text.
pub fn time_literal(text: &str) -> Option<(f64, &'static str)> {
    match classify_literal(text) {
        Ok(Lit::Time(v, u)) => Some((v, u)),
        _ => None,
    }
}

impl<'e, 'cx, 'ast> Evaluator<'e, 'cx, 'ast> {
    /// Creates an evaluator over `env`.
    pub(crate) fn new(env: &'e mut ScopeEnv<'cx, 'ast>) -> Self {
        Evaluator {
            env,
            frames: Vec::new(),
            call_stack: Vec::new(),
            steps: 0,
        }
    }

    fn fail(&mut self, span: Span, msg: impl Into<String>) -> EvalError {
        self.env.error(codes::CONST_EVAL, span, msg);
        EvalError::Failed
    }

    fn not_constant(&mut self, span: Span, what: &str) -> EvalError {
        self.env.report(
            Diagnostic::error(format!("{what} is not a constant"))
                .with_code(codes::NOT_CONSTANT)
                .with_label(span, "must be known at elaboration time"),
        );
        EvalError::NotConstant
    }

    // --- public entry points ---------------------------------------------

    /// Evaluates `e` with no outer context (self-determined).
    pub(crate) fn eval_self(&mut self, e: &Expr) -> EResult<Value> {
        self.eval(e, None)
    }

    /// Evaluates `e` as an integer.
    pub(crate) fn eval_i64(&mut self, e: &Expr) -> EResult<i64> {
        let v = self.eval(e, None)?;
        match v.as_i64() {
            Some(i) => Ok(i),
            None => Err(self.fail(
                e.span,
                match &v {
                    Value::Logic(l) if l.has_unknown() => {
                        "value has x or z bits where an integer is required".to_owned()
                    }
                    Value::Logic(_) => "value does not fit in 64 bits".to_owned(),
                    other => format!("`{other}` is not an integer"),
                },
            )),
        }
    }

    /// Evaluates `e` as a non-negative integer that fits `u32`.
    pub(crate) fn eval_u32(&mut self, e: &Expr) -> EResult<u32> {
        let i = self.eval_i64(e)?;
        u32::try_from(i).map_err(|_| self.fail(e.span, format!("`{i}` is out of range here")))
    }

    /// Evaluates `e` as a bit vector in the given context width.
    pub(crate) fn eval_logic(&mut self, e: &Expr, ctx: Option<u32>) -> EResult<Logic> {
        let v = self.eval(e, ctx)?;
        self.as_logic(v, e.span)
    }

    /// Evaluates `e` and converts it to `ty`, as an initialiser or
    /// assignment would.
    pub(crate) fn eval_as(&mut self, e: &Expr, ty: &VType) -> EResult<Value> {
        let ctx = ty.packed().map(Packed::width);
        let v = match (&e.kind, ty) {
            (ExprKind::Pattern(items), VType::Unpacked { .. }) => {
                self.pattern_array(items, ty, e.span)?
            }
            (ExprKind::Pattern(items), VType::Packed(p)) if p.fields.is_some() => {
                self.pattern_struct(items, p, e.span)?
            }
            _ => self.eval(e, ctx)?,
        };
        self.fit(v, ty, e.span)
    }

    /// Converts `v` to type `ty`: resizes integrals, converts between
    /// integral and real, keeps strings and arrays.
    pub(crate) fn fit(&mut self, v: Value, ty: &VType, span: Span) -> EResult<Value> {
        match ty {
            VType::Packed(p) => {
                let l = self.as_logic(v, span)?;
                Ok(Value::Logic(l.resize(p.width()).with_signed(p.signed)))
            }
            VType::Real => match v.to_real() {
                Some(r) => Ok(Value::Real(r)),
                None => Err(self.fail(span, "value cannot be converted to real")),
            },
            VType::String => match v {
                Value::Str(_) => Ok(v),
                Value::Logic(l) => Ok(Value::Str(logic_to_string(&l))),
                other => Err(self.fail(span, format!("`{other}` is not a string"))),
            },
            VType::Unpacked { elem, range } => match v {
                Value::Array(items) => {
                    if items.len() as u64 != range.len() {
                        return Err(self.fail(
                            span,
                            format!(
                                "array initialiser has {} elements but the type has {}",
                                items.len(),
                                range.len()
                            ),
                        ));
                    }
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        out.push(self.fit(item, elem, span)?);
                    }
                    Ok(Value::Array(out))
                }
                other => Err(self.fail(span, format!("`{other}` is not an array value"))),
            },
            VType::Event | VType::Chandle | VType::Void | VType::Interface { .. } => {
                Err(self.fail(span, format!("a value cannot have type `{ty}`")))
            }
        }
    }

    // --- evaluation --------------------------------------------------------

    fn as_logic(&mut self, v: Value, span: Span) -> EResult<Logic> {
        match v.to_logic() {
            Some(l) => Ok(l),
            None => Err(self.fail(span, "an integral value is required here")),
        }
    }

    fn as_real(&mut self, v: Value, span: Span) -> EResult<f64> {
        match v.to_real() {
            Some(r) => Ok(r),
            None => Err(self.fail(span, "a numeric value is required here")),
        }
    }

    /// The self-determined width and signedness of `e`.
    fn info(&mut self, e: &Expr) -> Option<Info> {
        width::info(e, self.env, &self.frames_types())
    }

    /// The types of the locals in the innermost frame, for width
    /// inference inside function bodies.
    fn frames_types(&self) -> Vec<(String, VType)> {
        self.frames
            .last()
            .map(|f| {
                f.vars
                    .iter()
                    .map(|v| (v.name.clone(), v.ty.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Coerces an operand to the expression's width and signedness
    /// (§5.4.1 step 2, §5.5.1 step 3).
    fn coerce(&mut self, v: Value, w: u32, signed: bool, span: Span) -> EResult<Logic> {
        let l = self.as_logic(v, span)?;
        Ok(l.with_signed(signed).resize(w).with_signed(signed))
    }

    /// Evaluates `e` in a context of `ctx` bits (`None`: self-determined).
    pub(crate) fn eval(&mut self, e: &Expr, ctx: Option<u32>) -> EResult<Value> {
        match &e.kind {
            ExprKind::Literal(lit) => self.literal(lit, ctx, e.span),
            ExprKind::Ident(id) => self.ident(id),
            ExprKind::SystemIdent(id) => match id.name.as_str() {
                "time" | "stime" | "realtime" | "random" | "urandom" => {
                    Err(self.not_constant(e.span, &format!("`${}`", id.name)))
                }
                _ => Err(self.fail(e.span, format!("unknown system function `${}`", id.name))),
            },
            ExprKind::Scoped { .. } | ExprKind::Member { .. } => self.path_value(e),
            ExprKind::Index { base, index } => self.index(e, base, index),
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => self.range(e, base, *kind, left, right),
            ExprKind::Unary { op, operand } => self.unary(e, *op, operand, ctx),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs, ctx),
            ExprKind::Ternary {
                cond,
                then_expr,
                else_expr,
            } => self.ternary(e, cond, then_expr, else_expr, ctx),
            ExprKind::Concat(parts) => {
                let mut vals = Vec::with_capacity(parts.len());
                for p in parts {
                    let v = self.eval(p, None)?;
                    vals.push(self.as_logic(v, p.span)?);
                }
                Ok(Value::Logic(Logic::concat_all(&vals)))
            }
            ExprKind::Replicate { count, elems } => {
                let n = self.eval_u32(count)?;
                let mut vals = Vec::with_capacity(elems.len());
                for p in elems {
                    let v = self.eval(p, None)?;
                    vals.push(self.as_logic(v, p.span)?);
                }
                Ok(Value::Logic(Logic::concat_all(&vals).replicate(n)))
            }
            ExprKind::Call { callee, args } => self.call(e, callee, args, ctx),
            ExprKind::Cast { target, expr } => self.cast(e, target, expr, ctx),
            ExprKind::Inside { expr, set } => self.inside(expr, set),
            ExprKind::MinTypMax { typ, .. } => self.eval(typ, ctx),
            ExprKind::Pattern(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if item.key.is_some() {
                        self.env.unsupported(
                            item.span,
                            "keyed assignment pattern without a target type",
                        );
                        return Err(EvalError::Failed);
                    }
                    out.push(self.eval(&item.value, None)?);
                }
                Ok(Value::Array(out))
            }
            ExprKind::Type(_) => Err(self.fail(e.span, "a type cannot be used as a value")),
            ExprKind::Assign { lhs, op, rhs } => {
                let v = self.assign_expr(lhs, *op, rhs, e.span)?;
                Ok(v)
            }
            ExprKind::IncDec {
                increment, target, ..
            } => {
                let op = if *increment {
                    AssignOp::Add
                } else {
                    AssignOp::Sub
                };
                let one = Expr::new(
                    ExprKind::Literal(Literal::Number {
                        text: "1".into(),
                        span: e.span,
                    }),
                    e.span,
                );
                self.assign_expr(target, op, &one, e.span)
            }
            ExprKind::ValueRange { .. } => {
                Err(self.fail(e.span, "a value range is only valid inside `inside`"))
            }
            ExprKind::Streaming { .. } => {
                self.env.unsupported(e.span, "streaming operator");
                Err(EvalError::Failed)
            }
            ExprKind::New(_) => {
                self.env.unsupported(e.span, "`new`");
                Err(EvalError::Failed)
            }
            ExprKind::Default => Err(self.fail(e.span, "`default` is only valid in a pattern")),
        }
    }

    fn literal(&mut self, lit: &Literal, ctx: Option<u32>, span: Span) -> EResult<Value> {
        match lit {
            Literal::Number { text, .. } => match classify_literal(text) {
                Ok(Lit::Unbased(b)) => Ok(Value::Logic(Logic::filled(ctx.unwrap_or(1), b))),
                Ok(Lit::Int(l)) => Ok(Value::Logic(l)),
                Ok(Lit::Real(r)) => Ok(Value::Real(r)),
                Ok(Lit::Time(..)) => Err(self.fail(
                    span,
                    "a time literal is only valid in a delay or a timeunit declaration",
                )),
                Err(msg) => Err(self.fail(span, format!("malformed literal `{text}`: {msg}"))),
            },
            Literal::Str { value, .. } => Ok(Value::Str(value.clone())),
            Literal::Null(_) => Err(self.fail(span, "`null` is not a constant value")),
            Literal::Unbounded(_) => Err(self.fail(span, "`$` is not a value here")),
        }
    }

    fn ident(&mut self, id: &ast::Ident) -> EResult<Value> {
        if let Some(v) = self.local(&id.name) {
            return Ok(v.value.clone());
        }
        let Some((sym, _, _)) = self.env.lookup(&id.name) else {
            self.env.undefined(id);
            return Err(EvalError::Failed);
        };
        match sym {
            Symbol::Const { value, .. } => Ok(value.clone()),
            Symbol::Genvar => Err(self.fail(id.span, "genvar is used outside a generate loop")),
            Symbol::Net { .. } | Symbol::Memory { .. } => {
                Err(self.not_constant(id.span, &format!("`{}`", id.name)))
            }
            Symbol::Type(_) => {
                Err(self.fail(id.span, format!("`{}` is a type, not a value", id.name)))
            }
            Symbol::Function(_) | Symbol::Task(_) => Err(self.fail(
                id.span,
                format!("`{}` is a subroutine, not a value", id.name),
            )),
            _ => Err(self.fail(id.span, format!("`{}` is not a value", id.name))),
        }
    }

    fn local(&self, name: &str) -> Option<&Var> {
        self.frames
            .last()
            .and_then(|f| f.vars.iter().rev().find(|v| v.name == name))
    }

    fn local_mut(&mut self, name: &str) -> Option<&mut Var> {
        self.frames
            .last_mut()
            .and_then(|f| f.vars.iter_mut().rev().find(|v| v.name == name))
    }

    fn path_value(&mut self, e: &Expr) -> EResult<Value> {
        match self.env.resolve_path(e) {
            Some(Symbol::Const { value, .. }) => Ok(value),
            Some(Symbol::Net { .. } | Symbol::Memory { .. }) => {
                Err(self.not_constant(e.span, "this signal"))
            }
            Some(_) => Err(self.fail(e.span, "this name is not a value")),
            None => Err(EvalError::Failed),
        }
    }

    /// The declared packed type of a value-producing path, if known.
    fn packed_type_of(&mut self, e: &Expr) -> Option<Packed> {
        if let ExprKind::Ident(id) = &e.kind
            && let Some(v) = self.local(&id.name)
        {
            return v.ty.packed().cloned();
        }
        self.env.type_of_path(e).and_then(|t| t.packed().cloned())
    }

    fn index(&mut self, e: &Expr, base: &Expr, index: &Expr) -> EResult<Value> {
        // A generate-array element or an interface member path.
        if matches!(base.kind, ExprKind::Ident(_) | ExprKind::Member { .. })
            && let Some(Symbol::GenArray(_)) = self.env.probe(|env| env.resolve_path(base))
        {
            return self.path_value(e);
        }
        let bv = self.eval(base, None)?;
        let i = self.eval_i64(index)?;
        match bv {
            Value::Array(items) => {
                let ty = self.env.type_of_path(base);
                let range = match &ty {
                    Some(VType::Unpacked { range, .. }) => *range,
                    _ => Range::unpacked_size(i64::try_from(items.len()).unwrap_or(i64::MAX)),
                };
                match range
                    .element_offset(i)
                    .and_then(|o| usize::try_from(o).ok())
                    .filter(|o| *o < items.len())
                {
                    Some(o) => Ok(items[o].clone()),
                    None => Err(self.fail(index.span, format!("index {i} is outside {range}"))),
                }
            }
            other => {
                let l = self.as_logic(other, base.span)?;
                let ty = self
                    .packed_type_of(base)
                    .unwrap_or_else(|| Packed::bits(l.width()));
                let elem = ty.element();
                let ew = elem.width();
                match ty.outer().offset(i) {
                    Some(o) => {
                        let lo = u32::try_from(o.saturating_mul(u64::from(ew))).unwrap_or(u32::MAX);
                        Ok(Value::Logic(slice_bits(&l, lo, ew)))
                    }
                    None => {
                        self.env.warning(
                            codes::OUT_OF_RANGE,
                            index.span,
                            format!("index {i} is outside {}", ty.outer()),
                        );
                        Ok(Value::Logic(Logic::x(ew)))
                    }
                }
            }
        }
    }

    fn range(
        &mut self,
        e: &Expr,
        base: &Expr,
        kind: ast::RangeKind,
        left: &Expr,
        right: &Expr,
    ) -> EResult<Value> {
        let bv = self.eval(base, None)?;
        let l = self.as_logic(bv, base.span)?;
        let ty = self
            .packed_type_of(base)
            .unwrap_or_else(|| Packed::bits(l.width()));
        let outer = ty.outer();
        let ew = ty.element().width();
        let a = self.eval_i64(left)?;
        let b = self.eval_i64(right)?;
        let (hi_idx, lo_idx) = match kind {
            ast::RangeKind::Fixed => {
                if outer.is_descending() != (a >= b) {
                    return Err(self.fail(
                        e.span,
                        format!("part-select [{a}:{b}] runs against the declared {outer}"),
                    ));
                }
                (a.max(b), a.min(b))
            }
            ast::RangeKind::IndexedUp => {
                if b <= 0 {
                    return Err(self.fail(right.span, "part-select width must be positive"));
                }
                if outer.is_descending() {
                    (a + b - 1, a)
                } else {
                    (a, a + b - 1)
                }
            }
            ast::RangeKind::IndexedDown => {
                if b <= 0 {
                    return Err(self.fail(right.span, "part-select width must be positive"));
                }
                if outer.is_descending() {
                    (a, a - b + 1)
                } else {
                    (a - b + 1, a)
                }
            }
        };
        let count = u32::try_from(hi_idx.abs_diff(lo_idx) + 1).unwrap_or(u32::MAX);
        let width = count.saturating_mul(ew);
        let low_end = if outer.is_descending() {
            lo_idx
        } else {
            hi_idx
        };
        match outer
            .offset(low_end)
            .zip(outer.offset(if outer.is_descending() {
                hi_idx
            } else {
                lo_idx
            })) {
            Some((o, _)) => {
                let lo = u32::try_from(o.saturating_mul(u64::from(ew))).unwrap_or(u32::MAX);
                Ok(Value::Logic(slice_bits(&l, lo, width)))
            }
            None => {
                self.env.warning(
                    codes::OUT_OF_RANGE,
                    e.span,
                    format!("part-select [{hi_idx}:{lo_idx}] is outside {outer}"),
                );
                Ok(Value::Logic(Logic::x(width)))
            }
        }
    }

    fn unary(&mut self, e: &Expr, op: UnaryOp, operand: &Expr, ctx: Option<u32>) -> EResult<Value> {
        match op {
            UnaryOp::LogicNot => {
                let v = self.eval(operand, None)?;
                Ok(Value::Logic(Logic::from_bit(!v.truth())))
            }
            UnaryOp::ReduceAnd
            | UnaryOp::ReduceNand
            | UnaryOp::ReduceOr
            | UnaryOp::ReduceNor
            | UnaryOp::ReduceXor
            | UnaryOp::ReduceXnor => {
                let v = self.eval(operand, None)?;
                let l = self.as_logic(v, operand.span)?;
                Ok(Value::Logic(match op {
                    UnaryOp::ReduceAnd => l.reduce_and(),
                    UnaryOp::ReduceNand => l.reduce_nand(),
                    UnaryOp::ReduceOr => l.reduce_or(),
                    UnaryOp::ReduceNor => l.reduce_nor(),
                    UnaryOp::ReduceXor => l.reduce_xor(),
                    _ => l.reduce_xnor(),
                }))
            }
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => {
                let Some(info) = self.info(e) else {
                    return Err(EvalError::Failed);
                };
                match info {
                    Info::Real => {
                        let v = self.eval(operand, None)?;
                        let r = self.as_real(v, operand.span)?;
                        Ok(Value::Real(match op {
                            UnaryOp::Minus => -r,
                            UnaryOp::Plus => r,
                            _ => return Err(self.fail(e.span, "`~` is not defined on real values")),
                        }))
                    }
                    Info::Bits { width, signed, .. } => {
                        let w = width.max(ctx.unwrap_or(0));
                        let v = self.eval(operand, Some(w))?;
                        let l = self.coerce(v, w, signed, operand.span)?;
                        Ok(Value::Logic(match op {
                            UnaryOp::Minus => l.neg(),
                            UnaryOp::Plus => l,
                            _ => l.not(),
                        }))
                    }
                    _ => Err(self.fail(e.span, "operand is not numeric")),
                }
            }
        }
    }

    fn binary(
        &mut self,
        e: &Expr,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        ctx: Option<u32>,
    ) -> EResult<Value> {
        use BinaryOp as B;
        match op {
            B::LogicAnd | B::LogicOr | B::Implies | B::Equiv => {
                let a = self.eval(lhs, None)?.truth();
                let b = self.eval(rhs, None)?.truth();
                let r = match op {
                    B::LogicAnd => a.and(b),
                    B::LogicOr => a.or(b),
                    B::Implies => (!a).or(b),
                    _ => !a.xor(b),
                };
                return Ok(Value::Logic(Logic::from_bit(r)));
            }
            _ => {}
        }
        let Some(info) = self.info(e) else {
            return Err(EvalError::Failed);
        };
        // Comparisons size their operands to each other (§5.4.1).
        let is_cmp = matches!(
            op,
            B::Lt
                | B::Le
                | B::Gt
                | B::Ge
                | B::Eq
                | B::Ne
                | B::CaseEq
                | B::CaseNe
                | B::WildEq
                | B::WildNe
        );
        let operand_info = if is_cmp {
            // Comparison operands are sized to each other, not to the
            // context (§5.4.1).
            let li = self.info(lhs).ok_or(EvalError::Failed)?;
            let ri = self.info(rhs).ok_or(EvalError::Failed)?;
            li.combine(&ri)
        } else {
            info.clone()
        };
        match operand_info {
            Info::Str(_) => {
                let a = self.eval(lhs, None)?;
                let b = self.eval(rhs, None)?;
                let (Value::Str(a), Value::Str(b)) = (a, b) else {
                    return Err(self.fail(e.span, "string operands expected"));
                };
                let r = match op {
                    B::Eq | B::CaseEq => a == b,
                    B::Ne | B::CaseNe => a != b,
                    B::Lt => a < b,
                    B::Le => a <= b,
                    B::Gt => a > b,
                    B::Ge => a >= b,
                    _ => return Err(self.fail(e.span, "operator is not defined on strings")),
                };
                Ok(Value::Logic(Logic::from_bool(r)))
            }
            Info::Real => {
                let a = self.eval(lhs, None)?;
                let b = self.eval(rhs, None)?;
                let a = self.as_real(a, lhs.span)?;
                let b = self.as_real(b, rhs.span)?;
                let bit = |c: bool| Value::Logic(Logic::from_bool(c));
                Ok(match op {
                    B::Add => Value::Real(a + b),
                    B::Sub => Value::Real(a - b),
                    B::Mul => Value::Real(a * b),
                    B::Div => Value::Real(a / b),
                    B::Pow => Value::Real(a.powf(b)),
                    B::Lt => bit(a < b),
                    B::Le => bit(a <= b),
                    B::Gt => bit(a > b),
                    B::Ge => bit(a >= b),
                    B::Eq | B::CaseEq => bit(a == b),
                    B::Ne | B::CaseNe => bit(a != b),
                    _ => return Err(self.fail(e.span, "operator is not defined on real values")),
                })
            }
            Info::Bits { width, signed, .. } => {
                let w = if is_cmp {
                    width
                } else {
                    width.max(ctx.unwrap_or(0))
                };
                let self_det_rhs = matches!(op, B::Shl | B::Shr | B::Ashl | B::Ashr | B::Pow);
                let a = self.eval(lhs, Some(w))?;
                let a = self.coerce(a, w, signed, lhs.span)?;
                let b = if self_det_rhs {
                    let v = self.eval(rhs, None)?;
                    self.as_logic(v, rhs.span)?
                } else {
                    let v = self.eval(rhs, Some(w))?;
                    self.coerce(v, w, signed, rhs.span)?
                };
                Ok(Value::Logic(match op {
                    B::Add => a.add(&b),
                    B::Sub => a.sub(&b),
                    B::Mul => a.mul(&b),
                    B::Div => a.div(&b),
                    B::Mod => a.rem(&b),
                    B::Pow => a.pow(&b),
                    B::Shl | B::Ashl => a.shl_by(&b),
                    B::Shr => a.shr_by(&b),
                    B::Ashr => a.sshr_by(&b),
                    B::BitAnd => a.and(&b),
                    B::BitOr => a.or(&b),
                    B::BitXor => a.xor(&b),
                    B::BitXnor => a.xnor(&b),
                    B::Lt => a.lt(&b),
                    B::Le => a.le(&b),
                    B::Gt => a.gt(&b),
                    B::Ge => a.ge(&b),
                    B::Eq => a.eq(&b),
                    B::Ne => a.ne(&b),
                    B::CaseEq => a.case_eq(&b),
                    B::CaseNe => a.case_ne(&b),
                    B::WildEq => a.wildcard_eq(&b),
                    B::WildNe => a.wildcard_eq(&b).logical_not(),
                    B::LogicAnd | B::LogicOr | B::Implies | B::Equiv => {
                        unreachable!("handled above")
                    }
                }))
            }
            Info::Other => Err(self.fail(e.span, "operands are not numeric")),
        }
    }

    fn ternary(
        &mut self,
        e: &Expr,
        cond: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        ctx: Option<u32>,
    ) -> EResult<Value> {
        let c = self.eval(cond, None)?.truth();
        let Some(info) = self.info(e) else {
            return Err(EvalError::Failed);
        };
        match info {
            Info::Bits { width, signed, .. } => {
                let w = width.max(ctx.unwrap_or(0));
                match c {
                    Bit::One => {
                        let v = self.eval(then_expr, Some(w))?;
                        Ok(Value::Logic(self.coerce(v, w, signed, then_expr.span)?))
                    }
                    Bit::Zero => {
                        let v = self.eval(else_expr, Some(w))?;
                        Ok(Value::Logic(self.coerce(v, w, signed, else_expr.span)?))
                    }
                    _ => {
                        let a = self.eval(then_expr, Some(w))?;
                        let a = self.coerce(a, w, signed, then_expr.span)?;
                        let b = self.eval(else_expr, Some(w))?;
                        let b = self.coerce(b, w, signed, else_expr.span)?;
                        Ok(Value::Logic(merge_unknown(&a, &b)))
                    }
                }
            }
            _ => match c {
                Bit::Zero => self.eval(else_expr, ctx),
                _ => self.eval(then_expr, ctx),
            },
        }
    }

    fn cast(
        &mut self,
        e: &Expr,
        target: &CastTarget,
        expr: &Expr,
        ctx: Option<u32>,
    ) -> EResult<Value> {
        match target {
            CastTarget::Const => self.eval(expr, ctx),
            CastTarget::Signing(s) => {
                let v = self.eval(expr, None)?;
                let l = self.as_logic(v, expr.span)?;
                Ok(Value::Logic(l.with_signed(*s == ast::Signing::Signed)))
            }
            CastTarget::Size(n) => {
                let w = self.eval_u32(n)?;
                let v = self.eval(expr, Some(w))?;
                let l = self.as_logic(v, expr.span)?;
                Ok(Value::Logic(l.resize(w)))
            }
            CastTarget::Type(dt) => {
                let Some(ty) = types::resolve(self.env, dt, &[], true) else {
                    return Err(EvalError::Failed);
                };
                let v = self.eval(expr, ty.packed().map(Packed::width))?;
                let v = self.fit(v, &ty, e.span)?;
                Ok(v)
            }
        }
    }

    fn inside(&mut self, expr: &Expr, set: &[Expr]) -> EResult<Value> {
        let mut w = self.info(expr).map_or(1, |i| i.width());
        for s in set {
            match &s.kind {
                ExprKind::ValueRange { low, high } => {
                    w = w.max(self.info(low).map_or(1, |i| i.width()));
                    w = w.max(self.info(high).map_or(1, |i| i.width()));
                }
                _ => w = w.max(self.info(s).map_or(1, |i| i.width())),
            }
        }
        let x = self.eval_logic(expr, Some(w))?.resize(w);
        let mut result = Bit::Zero;
        for s in set {
            let hit = match &s.kind {
                ExprKind::ValueRange { low, high } => {
                    let lo = self.eval_logic(low, Some(w))?.resize(w);
                    let hi = self.eval_logic(high, Some(w))?.resize(w);
                    x.ge(&lo).and(&x.le(&hi)).truth()
                }
                _ => {
                    let v = self.eval_logic(s, Some(w))?.resize(w);
                    x.wildcard_eq(&v).truth()
                }
            };
            result = result.or(hit);
            if result == Bit::One {
                break;
            }
        }
        Ok(Value::Logic(Logic::from_bit(result)))
    }

    fn pattern_array(
        &mut self,
        items: &[ast::PatternItem],
        ty: &VType,
        span: Span,
    ) -> EResult<Value> {
        let VType::Unpacked { elem, range } = ty else {
            return Err(EvalError::Failed);
        };
        let n = usize::try_from(range.len()).unwrap_or(usize::MAX);
        let mut default: Option<Value> = None;
        let mut positional = Vec::new();
        for item in items {
            match &item.key {
                None => positional.push(self.eval_as(&item.value, elem)?),
                Some(k) if matches!(k.kind, ExprKind::Default) => {
                    default = Some(self.eval_as(&item.value, elem)?);
                }
                Some(_) => {
                    self.env
                        .unsupported(item.span, "keyed array assignment pattern");
                    return Err(EvalError::Failed);
                }
            }
        }
        if positional.is_empty() {
            let Some(d) = default else {
                return Err(self.fail(span, "empty assignment pattern"));
            };
            return Ok(Value::Array(vec![d; n]));
        }
        if positional.len() != n {
            return Err(self.fail(
                span,
                format!(
                    "assignment pattern has {} elements but the array has {n}",
                    positional.len()
                ),
            ));
        }
        // A pattern lists elements in declared order; the array holds them
        // in element order, lowest index first.
        if range.is_descending() {
            positional.reverse();
        }
        Ok(Value::Array(positional))
    }

    fn pattern_struct(
        &mut self,
        items: &[ast::PatternItem],
        p: &Packed,
        span: Span,
    ) -> EResult<Value> {
        let fields = p.fields.clone().expect("packed struct");
        let mut result = Logic::x(p.width());
        let mut default: Option<&Expr> = None;
        let mut positional: Vec<&Expr> = Vec::new();
        let mut named: Vec<(&str, &Expr)> = Vec::new();
        for item in items {
            match &item.key {
                None => positional.push(&item.value),
                Some(k) => match &k.kind {
                    ExprKind::Default => default = Some(&item.value),
                    ExprKind::Ident(id) => named.push((&id.name, &item.value)),
                    _ => {
                        self.env
                            .unsupported(item.span, "typed assignment pattern key");
                        return Err(EvalError::Failed);
                    }
                },
            }
        }
        if !positional.is_empty() && positional.len() != fields.len() {
            return Err(self.fail(
                span,
                "positional pattern does not match the number of struct members",
            ));
        }
        for (i, f) in fields.iter().enumerate() {
            let src = positional
                .get(i)
                .copied()
                .or_else(|| named.iter().find(|(n, _)| *n == f.name).map(|(_, e)| *e))
                .or(default);
            let Some(src) = src else {
                return Err(self.fail(span, format!("struct member `{}` is not assigned", f.name)));
            };
            let v = self.eval_as(src, &f.ty)?;
            let l = self.as_logic(v, src.span)?;
            let fw = f.ty.packed().map_or(0, Packed::width);
            for b in 0..fw {
                result.set_bit(f.lsb + b, l.bit(b));
            }
        }
        Ok(Value::Logic(result.with_signed(p.signed)))
    }

    // --- calls -------------------------------------------------------------

    fn call(&mut self, e: &Expr, callee: &Expr, args: &[Arg], ctx: Option<u32>) -> EResult<Value> {
        if let ExprKind::SystemIdent(id) = &callee.kind {
            return self.system_call(e, &id.name, args, ctx);
        }
        let Some((f, is_task)) = self.env.resolve_callee(callee) else {
            return Err(EvalError::Failed);
        };
        if is_task {
            return Err(self.fail(e.span, "a task cannot be called in a constant expression"));
        }
        self.call_user(f, args, e.span)
    }

    fn arg_values<'a>(
        &mut self,
        args: &'a [Arg],
        span: Span,
        expected: usize,
    ) -> EResult<Vec<&'a Expr>> {
        let mut out = Vec::new();
        for a in args {
            match &a.value {
                Some(v) => out.push(v),
                None => return Err(self.fail(a.span, "missing argument")),
            }
        }
        if out.len() != expected {
            return Err(self.fail(
                span,
                format!("expected {expected} argument(s), found {}", out.len()),
            ));
        }
        Ok(out)
    }

    fn system_call(
        &mut self,
        e: &Expr,
        name: &str,
        args: &[Arg],
        ctx: Option<u32>,
    ) -> EResult<Value> {
        let _ = ctx;
        match name {
            "clog2" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let l = self.as_logic(v, a[0].span)?;
                if l.has_unknown() {
                    return Ok(Value::Logic(Logic::x(32).with_signed(true)));
                }
                Ok(Value::int(i64::from(clog2(&l))))
            }
            "bits" => {
                let a = self.arg_values(args, e.span, 1)?;
                let bits = self.bits_of(a[0])?;
                Ok(Value::int(i64::try_from(bits).unwrap_or(i64::MAX)))
            }
            "size"
            | "high"
            | "low"
            | "left"
            | "right"
            | "increment"
            | "dimensions"
            | "unpacked_dimensions" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(self.fail(e.span, format!("`${name}` takes one or two arguments")));
                }
                let target = args[0]
                    .value
                    .as_ref()
                    .ok_or_else(|| self.fail(args[0].span, "missing argument"))?;
                let dim = match args.get(1).and_then(|a| a.value.as_ref()) {
                    Some(d) => self.eval_i64(d)?,
                    None => 1,
                };
                let ty = self.type_of_expr(target)?;
                let unpacked = ty.unpacked_dims();
                let packed_dims: Vec<Range> = match ty.base() {
                    VType::Packed(p) => {
                        if p.dims.is_empty() {
                            vec![Range::new(0, 0)]
                        } else {
                            p.dims.clone()
                        }
                    }
                    _ => Vec::new(),
                };
                if name == "dimensions" {
                    return Ok(Value::int(
                        i64::try_from(unpacked.len() + packed_dims.len()).unwrap_or(0),
                    ));
                }
                if name == "unpacked_dimensions" {
                    return Ok(Value::int(i64::try_from(unpacked.len()).unwrap_or(0)));
                }
                let all: Vec<Range> = unpacked.into_iter().chain(packed_dims).collect();
                let Some(r) = usize::try_from(dim - 1).ok().and_then(|i| all.get(i)) else {
                    return Err(self.fail(e.span, format!("dimension {dim} does not exist")));
                };
                Ok(Value::int(match name {
                    "size" => i64::try_from(r.len()).unwrap_or(i64::MAX),
                    "high" => r.high(),
                    "low" => r.low(),
                    "left" => r.left,
                    "right" => r.right,
                    _ => {
                        if r.left >= r.right {
                            1
                        } else {
                            -1
                        }
                    }
                }))
            }
            "signed" | "unsigned" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let l = self.as_logic(v, a[0].span)?;
                Ok(Value::Logic(l.with_signed(name == "signed")))
            }
            "itor" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let r = self.as_real(v, a[0].span)?;
                Ok(Value::Real(r))
            }
            "rtoi" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let r = self.as_real(v, a[0].span)?;
                let truncated = real_to_int(r.trunc()).unwrap_or(0);
                Ok(Value::Logic(
                    Logic::from_i64(truncated, 32).with_signed(true),
                ))
            }
            "sqrt" | "ln" | "log10" | "exp" | "floor" | "ceil" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let r = self.as_real(v, a[0].span)?;
                Ok(Value::Real(match name {
                    "sqrt" => r.sqrt(),
                    "ln" => r.ln(),
                    "log10" => r.log10(),
                    "exp" => r.exp(),
                    "floor" => r.floor(),
                    _ => r.ceil(),
                }))
            }
            "pow" => {
                let a = self.arg_values(args, e.span, 2)?;
                let x = self.eval(a[0], None)?;
                let y = self.eval(a[1], None)?;
                let x = self.as_real(x, a[0].span)?;
                let y = self.as_real(y, a[1].span)?;
                Ok(Value::Real(x.powf(y)))
            }
            "countones" | "onehot" | "onehot0" | "isunknown" => {
                let a = self.arg_values(args, e.span, 1)?;
                let v = self.eval(a[0], None)?;
                let l = self.as_logic(v, a[0].span)?;
                let ones = l.bits().iter().filter(|b| **b == Bit::One).count();
                Ok(match name {
                    "countones" => Value::int(i64::try_from(ones).unwrap_or(i64::MAX)),
                    "onehot" => Value::Logic(Logic::from_bool(ones == 1)),
                    "onehot0" => Value::Logic(Logic::from_bool(ones <= 1)),
                    _ => Value::Logic(Logic::from_bool(l.has_unknown())),
                })
            }
            "time" | "stime" | "realtime" | "random" | "urandom" | "urandom_range" | "fopen"
            | "fgets" | "fscanf" | "sscanf" | "test$plusargs" | "value$plusargs" => {
                Err(self.not_constant(e.span, &format!("`${name}`")))
            }
            _ => Err(self.fail(
                e.span,
                format!("unknown or non-constant system function `${name}`"),
            )),
        }
    }

    /// The type of an expression for the array query functions: a name
    /// path's declared type, a type in expression position, or the packed
    /// type implied by the expression's width.
    fn type_of_expr(&mut self, e: &Expr) -> EResult<VType> {
        if let ExprKind::Type(dt) = &e.kind {
            return types::resolve(self.env, dt, &[], true).ok_or(EvalError::Failed);
        }
        if let ExprKind::Ident(id) = &e.kind
            && let Some(v) = self.local(&id.name)
        {
            return Ok(v.ty.clone());
        }
        if let Some(t) = self.env.type_of_path(e) {
            return Ok(t);
        }
        match self.info(e) {
            Some(Info::Bits { width, signed, .. }) => Ok(VType::Packed(if signed {
                Packed::sbits(width)
            } else {
                Packed::bits(width)
            })),
            Some(Info::Real) => Ok(VType::Real),
            Some(Info::Str(_)) => Ok(VType::String),
            _ => {
                self.env.resolve_path(e);
                Err(EvalError::Failed)
            }
        }
    }

    /// `$bits` of an expression or type.
    pub(crate) fn bits_of(&mut self, e: &Expr) -> EResult<u64> {
        let ty = self.type_of_expr(e)?;
        ty.bit_size()
            .ok_or_else(|| self.fail(e.span, format!("`{ty}` has no bit representation")))
    }

    /// Interprets a call of the user function `f`.
    fn call_user(&mut self, f: FnRef<'ast>, args: &[Arg], span: Span) -> EResult<Value> {
        let name = f.def.name.name.clone();
        if self.call_stack.contains(&name) {
            self.env.report(
                Diagnostic::error(format!("constant function `{name}` calls itself"))
                    .with_code(codes::RECURSION)
                    .with_label(span, "recursive call")
                    .with_secondary(f.def.name.span, "function declared here")
                    .with_note("recursion is not supported in constant functions"),
            );
            return Err(EvalError::Failed);
        }
        let formals = formals(f.def);
        let slots = match bind_args(&formals, args) {
            Ok(s) => s,
            Err(msg) => {
                self.env.error(
                    codes::ARGUMENTS,
                    span,
                    format!("in call of `{name}`: {msg}"),
                );
                return Err(EvalError::Failed);
            }
        };
        // Actuals are evaluated in the caller's scope.
        let mut actuals: Vec<Option<Value>> = Vec::with_capacity(slots.len());
        for (slot, formal) in slots.iter().zip(&formals) {
            match slot {
                Some(e) => actuals.push(Some(self.eval(e, None)?)),
                None => {
                    if formal.dir == Direction::Input {
                        self.env.error(
                            codes::ARGUMENTS,
                            span,
                            format!(
                                "missing argument `{}` in call of `{name}`",
                                formal.name.name
                            ),
                        );
                        return Err(EvalError::Failed);
                    }
                    actuals.push(None);
                }
            }
        }
        self.env.enter(f.home);
        let result = self.run_function(&f, &formals, actuals, span);
        self.env.leave();
        result
    }

    fn run_function(
        &mut self,
        f: &FnRef<'ast>,
        formals: &[Formal<'ast>],
        actuals: Vec<Option<Value>>,
        span: Span,
    ) -> EResult<Value> {
        let name = f.def.name.name.clone();
        let ret_ty = match &f.def.ret {
            None => return Err(self.fail(span, format!("`{name}` is a task"))),
            Some(dt) if dt.kind == DataTypeKind::Void => VType::Void,
            Some(dt) if dt.is_empty() => VType::bit(),
            Some(dt) => types::resolve(self.env, dt, &[], true).ok_or(EvalError::Failed)?,
        };
        let mut frame = Frame::default();
        for (formal, actual) in formals.iter().zip(actuals) {
            let ty = if formal.data_type.is_empty() && formal.dims.is_empty() {
                VType::bit()
            } else {
                types::resolve(self.env, formal.data_type, formal.dims, true)
                    .ok_or(EvalError::Failed)?
            };
            let value = match actual {
                Some(v) => self.fit(v, &ty, formal.name.span)?,
                None => default_value(&ty),
            };
            frame.vars.push(Var {
                name: formal.name.name.clone(),
                ty,
                value,
            });
        }
        if ret_ty != VType::Void {
            frame.vars.push(Var {
                name: name.clone(),
                ty: ret_ty.clone(),
                value: default_value(&ret_ty),
            });
        }
        self.frames.push(frame);
        self.call_stack.push(name.clone());
        let flow = self.exec_stmts(&f.def.body);
        self.call_stack.pop();
        let frame = self.frames.pop().expect("frame pushed above");
        flow?;
        if ret_ty == VType::Void {
            return Ok(Value::Logic(Logic::zero(1)));
        }
        Ok(frame
            .vars
            .iter()
            .rev()
            .find(|v| v.name == name)
            .map(|v| v.value.clone())
            .expect("return variable declared above"))
    }

    // --- statements --------------------------------------------------------

    fn step(&mut self, span: Span) -> EResult<()> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            self.env.report(
                Diagnostic::error("constant function evaluation exceeded its step limit")
                    .with_code(codes::LIMIT)
                    .with_label(span, "still running here")
                    .with_note("a loop in a constant function may not terminate"),
            );
            return Err(EvalError::Failed);
        }
        Ok(())
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> EResult<Flow> {
        for s in stmts {
            let flow = self.exec(s)?;
            if flow != Flow::Next {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }

    fn exec(&mut self, s: &Stmt) -> EResult<Flow> {
        self.step(s.span)?;
        match &s.kind {
            StmtKind::Null => Ok(Flow::Next),
            StmtKind::Block(b) => self.exec_stmts(&b.stmts),
            StmtKind::Assign(a) => {
                if a.op == AssignOp::NonBlocking {
                    return Err(self.fail(s.span, "non-blocking assignment in a constant function"));
                }
                if a.timing.is_some() {
                    return Err(self.fail(s.span, "timing control in a constant function"));
                }
                self.assign_expr(&a.lhs, a.op, &a.rhs, s.span)?;
                Ok(Flow::Next)
            }
            StmtKind::Expr(e) => {
                match &e.kind {
                    ExprKind::Call { callee, args } => {
                        if let ExprKind::SystemIdent(id) = &callee.kind
                            && matches!(
                                id.name.as_str(),
                                "display" | "write" | "info" | "warning" | "error"
                            )
                        {
                            return Ok(Flow::Next);
                        }
                        self.call(e, callee, args, None)?;
                    }
                    ExprKind::Cast { .. } => {
                        self.eval(e, None)?;
                    }
                    _ => {
                        self.eval(e, None)?;
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::If(i) => {
                let c = self.eval(&i.cond, None)?.truth();
                if c == Bit::One {
                    self.exec(&i.then_stmt)
                } else if let Some(e) = &i.else_stmt {
                    self.exec(e)
                } else {
                    Ok(Flow::Next)
                }
            }
            StmtKind::Case(c) => self.exec_case(c),
            StmtKind::For(f) => {
                for init in &f.init {
                    match init {
                        ForInit::Decl(vd) => self.declare_vars(vd)?,
                        ForInit::Assign(e) => {
                            self.eval(e, None)?;
                        }
                    }
                }
                loop {
                    self.step(s.span)?;
                    if let Some(c) = &f.cond
                        && self.eval(c, None)?.truth() != Bit::One
                    {
                        break;
                    }
                    match self.exec(&f.body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next | Flow::Continue => {}
                    }
                    for st in &f.step {
                        self.eval(st, None)?;
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::While(c, body) => {
                loop {
                    self.step(s.span)?;
                    if self.eval(c, None)?.truth() != Bit::One {
                        break;
                    }
                    match self.exec(body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::DoWhile(body, c) => {
                loop {
                    self.step(s.span)?;
                    match self.exec(body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next | Flow::Continue => {}
                    }
                    if self.eval(c, None)?.truth() != Bit::One {
                        break;
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::Repeat(n, body) => {
                let n = self.eval_i64(n)?;
                for _ in 0..n.max(0) {
                    self.step(s.span)?;
                    match self.exec(body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next | Flow::Continue => {}
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::Forever(body) => loop {
                self.step(s.span)?;
                match self.exec(body)? {
                    Flow::Break => return Ok(Flow::Next),
                    Flow::Return => return Ok(Flow::Return),
                    Flow::Next | Flow::Continue => {}
                }
            },
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
            StmtKind::Return(v) => {
                if let Some(v) = v {
                    let name = self.call_stack.last().cloned().unwrap_or_default();
                    let ty = self.local(&name).map(|v| v.ty.clone());
                    let Some(ty) = ty else {
                        return Err(self.fail(s.span, "`return` with a value in a void function"));
                    };
                    let val = self.eval_as(v, &ty)?;
                    if let Some(var) = self.local_mut(&name) {
                        var.value = val;
                    }
                }
                Ok(Flow::Return)
            }
            StmtKind::Decl(item) => {
                match &item.kind {
                    ast::ItemKind::Var(vd) => self.declare_vars(vd)?,
                    ast::ItemKind::Param(pd) => {
                        for d in &pd.decls {
                            let ty = if pd.data_type.is_empty() {
                                None
                            } else {
                                Some(
                                    types::resolve(self.env, &pd.data_type, &d.dims, true)
                                        .ok_or(EvalError::Failed)?,
                                )
                            };
                            let Some(init) = &d.init else {
                                return Err(self.fail(d.span, "parameter without a value"));
                            };
                            let value = match &ty {
                                Some(t) => self.eval_as(init, t)?,
                                None => self.eval(init, None)?,
                            };
                            let ty = ty.unwrap_or_else(|| value.natural_type());
                            self.frames
                                .last_mut()
                                .expect("in a function")
                                .vars
                                .push(Var {
                                    name: d.name.name.clone(),
                                    ty,
                                    value,
                                });
                        }
                    }
                    ast::ItemKind::Port(_) => {}
                    _ => {
                        return Err(self.fail(
                            s.span,
                            "this declaration is not allowed in a constant function",
                        ));
                    }
                }
                Ok(Flow::Next)
            }
            StmtKind::Fork(..)
            | StmtKind::Timing(..)
            | StmtKind::Wait(..)
            | StmtKind::WaitFork
            | StmtKind::Disable(_)
            | StmtKind::DisableFork
            | StmtKind::ProcAssign(..)
            | StmtKind::Deassign(_)
            | StmtKind::Force(..)
            | StmtKind::Release(_)
            | StmtKind::Trigger { .. }
            | StmtKind::Foreach(_)
            | StmtKind::Assert(_) => Err(self.fail(
                s.span,
                "this statement is not allowed in a constant function",
            )),
        }
    }

    fn exec_case(&mut self, c: &ast::Case) -> EResult<Flow> {
        if c.inside {
            for item in &c.items {
                if item.patterns.is_empty() {
                    continue;
                }
                let hit = self.inside(&c.expr, &item.patterns)?;
                if hit.truth() == Bit::One {
                    return self.exec(&item.body);
                }
            }
            if let Some(d) = c.items.iter().find(|i| i.patterns.is_empty()) {
                return self.exec(&d.body);
            }
            return Ok(Flow::Next);
        }
        let mut w = self.info(&c.expr).map_or(1, |i| i.width());
        for item in &c.items {
            for p in &item.patterns {
                w = w.max(self.info(p).map_or(1, |i| i.width()));
            }
        }
        let subject = self.eval_logic(&c.expr, Some(w))?.resize(w);
        for item in &c.items {
            for p in &item.patterns {
                let v = self.eval_logic(p, Some(w))?.resize(w);
                let hit = match c.kind {
                    ast::CaseKind::Case => subject.case_eq(&v).truth() == Bit::One,
                    ast::CaseKind::Casez => subject.casez_match(&v),
                    ast::CaseKind::Casex => subject.casex_match(&v),
                };
                if hit {
                    return self.exec(&item.body);
                }
            }
        }
        if let Some(d) = c.items.iter().find(|i| i.patterns.is_empty()) {
            return self.exec(&d.body);
        }
        Ok(Flow::Next)
    }

    fn declare_vars(&mut self, vd: &ast::VarDecl) -> EResult<()> {
        for d in &vd.decls {
            let ty =
                types::resolve(self.env, &vd.data_type, &d.dims, true).ok_or(EvalError::Failed)?;
            let value = match &d.init {
                Some(init) => self.eval_as(init, &ty)?,
                None => default_value(&ty),
            };
            self.frames
                .last_mut()
                .expect("in a function")
                .vars
                .push(Var {
                    name: d.name.name.clone(),
                    ty,
                    value,
                });
        }
        Ok(())
    }

    /// Runs `lhs op= rhs` on a local variable and returns the new value.
    fn assign_expr(&mut self, lhs: &Expr, op: AssignOp, rhs: &Expr, span: Span) -> EResult<Value> {
        let target_ty = self.lvalue_type(lhs)?;
        let value = match op {
            AssignOp::Blocking | AssignOp::NonBlocking => self.eval_as(rhs, &target_ty)?,
            _ => {
                let bin = match op {
                    AssignOp::Add => BinaryOp::Add,
                    AssignOp::Sub => BinaryOp::Sub,
                    AssignOp::Mul => BinaryOp::Mul,
                    AssignOp::Div => BinaryOp::Div,
                    AssignOp::Mod => BinaryOp::Mod,
                    AssignOp::And => BinaryOp::BitAnd,
                    AssignOp::Or => BinaryOp::BitOr,
                    AssignOp::Xor => BinaryOp::BitXor,
                    AssignOp::Shl => BinaryOp::Shl,
                    AssignOp::Shr => BinaryOp::Shr,
                    AssignOp::Ashl => BinaryOp::Ashl,
                    _ => BinaryOp::Ashr,
                };
                let combined = Expr::new(
                    ExprKind::Binary {
                        op: bin,
                        lhs: Box::new(lhs.clone()),
                        rhs: Box::new(rhs.clone()),
                    },
                    span,
                );
                self.eval_as(&combined, &target_ty)?
            }
        };
        self.store(lhs, value.clone())?;
        Ok(value)
    }

    /// The type of an assignment target inside a function.
    fn lvalue_type(&mut self, lhs: &Expr) -> EResult<VType> {
        match &lhs.kind {
            ExprKind::Ident(id) => match self.local(&id.name) {
                Some(v) => Ok(v.ty.clone()),
                None => Err(self.fail(
                    lhs.span,
                    format!("`{}` is not a local variable of this function", id.name),
                )),
            },
            ExprKind::Index { base, .. } => {
                let bt = self.lvalue_type(base)?;
                Ok(match bt {
                    VType::Unpacked { elem, .. } => *elem,
                    VType::Packed(p) => VType::Packed(p.element()),
                    other => {
                        return Err(self.fail(lhs.span, format!("`{other}` cannot be indexed")));
                    }
                })
            }
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => {
                let bt = self.lvalue_type(base)?;
                let Some(p) = bt.packed().cloned() else {
                    return Err(self.fail(lhs.span, "part-select of a non-integral variable"));
                };
                let n = match kind {
                    ast::RangeKind::Fixed => {
                        let a = self.eval_i64(left)?;
                        let b = self.eval_i64(right)?;
                        a.abs_diff(b) + 1
                    }
                    _ => u64::try_from(self.eval_i64(right)?).unwrap_or(0),
                };
                let w = u32::try_from(n)
                    .unwrap_or(u32::MAX)
                    .saturating_mul(p.element().width());
                Ok(VType::bits(w))
            }
            _ => Err(self.fail(lhs.span, "invalid assignment target in a constant function")),
        }
    }

    /// Writes `value` (already converted to the target's type) to `lhs`.
    fn store(&mut self, lhs: &Expr, value: Value) -> EResult<()> {
        match &lhs.kind {
            ExprKind::Ident(id) => {
                let Some(var) = self.local_mut(&id.name) else {
                    return Err(self.fail(lhs.span, "assignment to a non-local name"));
                };
                var.value = value;
                Ok(())
            }
            ExprKind::Index { base, index } => {
                let i = self.eval_i64(index)?;
                let bt = self.lvalue_type(base)?;
                let cur = self.eval(base, None)?;
                let updated = match (bt, cur) {
                    (VType::Unpacked { range, .. }, Value::Array(mut items)) => {
                        let Some(o) = range.offset(i).and_then(|o| usize::try_from(o).ok()) else {
                            return Err(
                                self.fail(index.span, format!("index {i} is outside {range}"))
                            );
                        };
                        let idx = items.len() - 1 - o.min(items.len() - 1);
                        items[idx] = value;
                        Value::Array(items)
                    }
                    (VType::Packed(p), Value::Logic(mut l)) => {
                        let ew = p.element().width();
                        let Some(o) = p.outer().offset(i) else {
                            self.env.warning(
                                codes::OUT_OF_RANGE,
                                index.span,
                                format!("index {i} is outside {}", p.outer()),
                            );
                            return Ok(());
                        };
                        let lo = u32::try_from(o.saturating_mul(u64::from(ew))).unwrap_or(u32::MAX);
                        let v = self.as_logic(value, lhs.span)?.resize(ew);
                        for b in 0..ew {
                            l.set_bit(lo + b, v.bit(b));
                        }
                        Value::Logic(l)
                    }
                    _ => return Err(self.fail(lhs.span, "invalid indexed assignment")),
                };
                self.store(base, updated)
            }
            ExprKind::Range {
                base,
                kind,
                left,
                right,
            } => {
                let bt = self.lvalue_type(base)?;
                let Some(p) = bt.packed().cloned() else {
                    return Err(self.fail(lhs.span, "part-select of a non-integral variable"));
                };
                let cur = self.eval(base, None)?;
                let mut l = self.as_logic(cur, base.span)?;
                let a = self.eval_i64(left)?;
                let b = self.eval_i64(right)?;
                let (hi, lo) = match kind {
                    ast::RangeKind::Fixed => (a.max(b), a.min(b)),
                    ast::RangeKind::IndexedUp => (a + b - 1, a),
                    ast::RangeKind::IndexedDown => (a, a - b + 1),
                };
                let ew = p.element().width();
                let outer = p.outer();
                let (Some(olo), Some(ohi)) = (outer.offset(lo), outer.offset(hi)) else {
                    self.env.warning(
                        codes::OUT_OF_RANGE,
                        lhs.span,
                        format!("part-select [{hi}:{lo}] is outside {outer}"),
                    );
                    return Ok(());
                };
                let start =
                    u32::try_from(olo.min(ohi).saturating_mul(u64::from(ew))).unwrap_or(u32::MAX);
                let width = u32::try_from((hi.abs_diff(lo) + 1).saturating_mul(u64::from(ew)))
                    .unwrap_or(u32::MAX);
                let v = self.as_logic(value, lhs.span)?.resize(width);
                for i in 0..width {
                    l.set_bit(start + i, v.bit(i));
                }
                self.store(base, Value::Logic(l))
            }
            _ => Err(self.fail(lhs.span, "invalid assignment target in a constant function")),
        }
    }
}

/// `width` bits of `l` starting at bit `lo`, zero-filled past the end.
pub fn slice_bits(l: &Logic, lo: u32, width: u32) -> Logic {
    let bits: Vec<Bit> = (0..width)
        .map(|i| l.get(lo.saturating_add(i)).unwrap_or(Bit::Zero))
        .collect();
    Logic::from_bits(&bits)
}

/// The initial value of a variable of type `ty`: all `x` for four-state
/// integrals, zero for two-state ones, `0.0`, `""`.
pub fn default_value(ty: &VType) -> Value {
    match ty {
        VType::Packed(p) => Value::Logic(
            if p.four_state {
                Logic::x(p.width())
            } else {
                Logic::zero(p.width())
            }
            .with_signed(p.signed),
        ),
        VType::Real => Value::Real(0.0),
        VType::String => Value::Str(String::new()),
        VType::Unpacked { elem, range } => {
            let n = usize::try_from(range.len()).unwrap_or(0);
            Value::Array(vec![default_value(elem); n])
        }
        VType::Event | VType::Chandle | VType::Void | VType::Interface { .. } => {
            Value::Logic(Logic::zero(1))
        }
    }
}

/// Merges two equal-width values for an `x` condition in `?:`: bits that
/// agree are kept, the rest become `x` (§5.1.13).
pub fn merge_unknown(a: &Logic, b: &Logic) -> Logic {
    let bits: Vec<Bit> = a
        .bits()
        .iter()
        .zip(b.bits())
        .map(|(x, y)| if *x == y && x.is_known() { *x } else { Bit::X })
        .collect();
    Logic::from_bits(&bits).with_signed(a.is_signed() && b.is_signed())
}

/// `$clog2`: the number of bits needed to address `v` items (0 for 0 and
/// 1).
pub fn clog2(v: &Logic) -> u32 {
    if v.width() <= 64 {
        let n = v.to_u64().unwrap_or(0);
        if n <= 1 {
            return 0;
        }
        return 64 - (n - 1).leading_zeros();
    }
    let one = Logic::from_u64(1, v.width());
    let m = v.sub(&one);
    if v.is_zero() || m.is_zero() {
        return 0;
    }
    (0..v.width())
        .rev()
        .find(|&i| m.bit(i) == Bit::One)
        .map_or(0, |i| i + 1)
}

/// The string a bit vector represents when read as text (8 bits per
/// character, leading NULs dropped), for `string` conversions.
fn logic_to_string(l: &Logic) -> String {
    let mut bytes = Vec::new();
    let w = l.width();
    let mut i = 0;
    while i < w {
        let n = (w - i).min(8);
        let v = slice_bits(l, i, n).to_u64().unwrap_or(0);
        bytes.push(u8::try_from(v).unwrap_or(0));
        i += n;
    }
    bytes.reverse();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_conversions() {
        assert_eq!(Value::int(5).as_i64(), Some(5));
        assert_eq!(Value::Real(2.5).as_i64(), Some(3));
        assert_eq!(Value::Real(-2.5).as_i64(), Some(-3));
        assert_eq!(
            Value::Str("ab".into()).to_logic(),
            Some(Logic::from_u64(0x6162, 16))
        );
        assert_eq!(string_bits(""), Logic::zero(8));
        assert_eq!(Value::Logic(Logic::from_u64(3, 4)).to_real(), Some(3.0));
        assert_eq!(Value::Logic(Logic::from_i64(-3, 4)).to_real(), Some(-3.0));
        assert_eq!(Value::Str("".into()).truth(), Bit::Zero);
        assert_eq!(Value::Array(vec![]).truth(), Bit::X);
        assert_eq!(Value::int(-1).key_text(), "-1");
        assert_eq!(Value::Logic(Logic::x(4)).key_text(), "4'hx");
        assert_eq!(Value::Str("a".into()).key_text(), "\"a\"");
        assert_eq!(Value::Real(1.5).to_string(), "1.5");
        assert_eq!(
            Value::Array(vec![Value::int(1), Value::int(2)]).key_text(),
            "{1,2}"
        );
        let nat = Value::int(7).natural_type();
        assert_eq!(
            nat.packed().map(|p| (p.width(), p.signed, p.is_unsized)),
            Some((32, true, true))
        );
        assert_eq!(Value::Real(1.0).natural_type(), VType::Real);
        assert_eq!(logic_to_string(&Logic::from_u64(0x6869, 24)), "hi");
        assert_eq!(real_to_i64(f64::NAN), None);
        assert_eq!(real_to_i64(1e30), None);
    }

    #[test]
    fn literal_classification() {
        assert!(matches!(
            classify_literal("'0"),
            Ok(Lit::Unbased(Bit::Zero))
        ));
        assert!(matches!(classify_literal("'Z"), Ok(Lit::Unbased(Bit::Z))));
        assert!(matches!(classify_literal("8'hff"), Ok(Lit::Int(_))));
        assert!(matches!(classify_literal("1.5e3"), Ok(Lit::Real(v)) if v == 1500.0));
        assert!(matches!(classify_literal("1e1"), Ok(Lit::Real(v)) if v == 10.0));
        assert_eq!(time_literal("10ns"), Some((10.0, "ns")));
        assert_eq!(time_literal("1step"), Some((1.0, "step")));
        assert_eq!(time_literal("1.5us"), Some((1.5, "us")));
        assert_eq!(time_literal("42"), None);
        assert!(classify_literal("8'hzz_").is_ok());
        assert!(classify_literal("'").is_err());
    }

    #[test]
    fn clog2_values() {
        assert_eq!(clog2(&Logic::from_u64(0, 32)), 0);
        assert_eq!(clog2(&Logic::from_u64(1, 32)), 0);
        assert_eq!(clog2(&Logic::from_u64(2, 32)), 1);
        assert_eq!(clog2(&Logic::from_u64(16, 32)), 4);
        assert_eq!(clog2(&Logic::from_u64(17, 32)), 5);
        assert_eq!(clog2(&Logic::from_u64(256, 100)), 8);
        assert_eq!(clog2(&Logic::from_u64(1, 100)), 0);
    }

    #[test]
    fn merge_and_defaults() {
        let a = Logic::from_u64(0b1100, 4);
        let b = Logic::from_u64(0b1010, 4);
        assert_eq!(merge_unknown(&a, &b).to_binary_string(), "1xx0");
        assert_eq!(default_value(&VType::bits(3)), Value::Logic(Logic::x(3)));
        let mut two = Packed::bits(3);
        two.four_state = false;
        assert_eq!(
            default_value(&VType::Packed(two)),
            Value::Logic(Logic::zero(3))
        );
        assert_eq!(default_value(&VType::Real), Value::Real(0.0));
        assert_eq!(default_value(&VType::String), Value::Str(String::new()));
        let arr = VType::Unpacked {
            elem: Box::new(VType::bit()),
            range: Range::new(0, 1),
        };
        assert_eq!(
            default_value(&arr),
            Value::Array(vec![Value::Logic(Logic::x(1)); 2])
        );
    }

    /// The sizing examples of IEEE 1364-2005 §5.4, evaluated end to end.
    mod sizing {
        use crate::verilog::elab::tests::param_text;

        /// `expr` evaluates to the sized literal `want`.
        #[track_caller]
        fn eval(expr: &str, want: &str) {
            assert_eq!(param_text("", expr), want, "for `{expr}`");
        }

        /// Same, with declarations (parameters) in scope.
        #[track_caller]
        fn eval_in(decls: &str, expr: &str, want: &str) {
            assert_eq!(param_text(decls, expr), want, "for `{expr}`");
        }

        #[test]
        fn literals_keep_their_written_size() {
            eval("4'hf", "4'd15");
            eval("8'hff", "8'd255");
            eval("12", "32'sd12");
            eval("-1", "32'sd-1");
            eval("'d5", "32'd5");
            eval("4'sd7", "4'sd7");
            eval("4'bx", "4'hx");
            eval("8'b1010_zzzz", "8'haz");
        }

        /// §5.4.1: an operator is evaluated at the width of its widest
        /// operand, and the result wraps at that width.
        #[test]
        fn arithmetic_uses_the_widest_operand() {
            eval("4'hf + 4'h1", "4'd0");
            eval("4'hf + 8'h1", "8'd16");
            eval("8'hff + 1", "32'd256");
            eval("4'h8 * 4'h2", "4'd0");
            eval("4'h8 * 8'h2", "8'd16");
            eval("8'd200 + 8'd100", "8'd44");
        }

        /// §5.5.1: the expression is unsigned as soon as one
        /// context-determined operand is, and a signed operand is then
        /// read as unsigned rather than sign-extended.
        #[test]
        fn mixing_signs_makes_the_expression_unsigned() {
            eval("4'sd1 - 4'd2", "4'd15");
            eval("4'sd1 - 4'sd2", "4'sd-1");
            eval("-4'sd1 + 8'sd0", "8'sd-1");
            // The operand is widened before the minus is applied.
            eval("-4'd1 + 8'd0", "8'd255");
            eval("$signed(4'hf)", "4'sd-1");
            eval("$unsigned(-4'sd1)", "4'd15");
            eval("$signed(4'hf) + 8'sd0", "8'sd-1");
        }

        /// §5.4.1 Table 5-22: concatenations and replications are
        /// self-determined and unsigned.
        #[test]
        fn concatenation_is_self_determined() {
            eval("{4'hf, 4'h1}", "8'd241");
            eval("{4{1'b1}}", "4'd15");
            eval("{2{2'b10}}", "4'd10");
            eval("{1'b1, {3{1'b0}}}", "4'd8");
            // The operands of a concatenation do not take its width.
            eval("{8'hff + 8'h1, 1'b1}", "9'd1");
            // A concatenation of signed values is unsigned.
            eval("{4'sd15} + 8'sd0", "8'd15");
        }

        /// §5.4.1: `?:` sizes its branches to the context, its condition
        /// is self-determined.
        #[test]
        fn conditional_sizes_both_branches() {
            eval("1'b1 ? 4'hf : 8'h0", "8'd15");
            eval("1'b0 ? 4'hf : 8'h1", "8'd1");
            eval_in("localparam S = 1'b1;", "S ? 8'hff + 8'h1 : 8'h0", "8'd0");
            // An x condition merges the branches bit by bit.
            eval("1'bx ? 4'b1100 : 4'b1010", "4'b1xx0");
        }

        /// §5.4.1: the right operand of a shift is self-determined, the
        /// left one takes the context.
        #[test]
        fn shifts_keep_the_left_operand_width() {
            eval("4'h1 << 2", "4'd4");
            eval("4'h1 << 4", "4'd0");
            eval("1'b1 << 3", "1'd0");
            eval("8'h80 >> 3", "8'd16");
            eval("8'sh80 >>> 3", "8'sd-16");
            eval("8'h80 >>> 3", "8'd16");
            eval("4'sd8 >>> 1", "4'sd-4");
        }

        /// Comparisons yield one unsigned bit but size their operands to
        /// each other first.
        #[test]
        fn comparisons_are_one_bit() {
            eval("4'hf > 4'h1", "1'd1");
            eval("-4'sd1 < 4'sd1", "1'd1");
            // One unsigned operand makes the comparison unsigned, so -1
            // reads as 15.
            eval("-4'sd1 < 4'd1", "1'd0");
            eval("8'hff == 8'hff", "1'd1");
            eval("4'bxx01 == 4'b0001", "1'hx");
            eval("4'bxx01 === 4'b0001", "1'd0");
            eval("4'b1z01 ==? 4'b1z01", "1'd1");
        }

        /// Reductions and the logical connectives are one bit.
        #[test]
        fn reductions_and_logic() {
            eval("&4'hf", "1'd1");
            eval("&4'h7", "1'd0");
            eval("|4'h0", "1'd0");
            eval("^4'b1011", "1'd1");
            eval("~&4'hf", "1'd0");
            eval("!4'h0", "1'd1");
            eval("4'h1 && 4'h2", "1'd1");
            eval("4'h0 || 4'h0", "1'd0");
        }

        /// Division, modulo and the power operator.
        #[test]
        fn division_and_power() {
            eval("8'd200 / 8'd3", "8'd66");
            eval("-8'sd7 / 8'sd2", "8'sd-3");
            eval("-8'sd7 % 8'sd2", "8'sd-1");
            eval("8'd5 / 8'd0", "8'hxx");
            eval("4'h2 ** 3", "4'd8");
            eval("8'd2 ** 8'd9", "8'd0");
        }

        /// Reals convert to integers by rounding (§4.3.1), and an integer
        /// operand of a real operator becomes real (§5.5.1).
        #[test]
        fn real_arithmetic() {
            eval("1.5 + 1.5", "\"3\"");
            eval_in("localparam real RV = 2.5;", "RV * 2.0", "\"5\"");
            eval_in("localparam int I = 3;", "I / 2", "32'sd1");
            eval_in("localparam real RV = 3.0;", "RV / 2.0", "\"1.5\"");
            eval("3.7 > 3.6", "1'd1");
            eval_in("localparam int I = 2.6;", "I", "32'sd3");
            eval("$rtoi(3.9)", "32'sd3");
            eval("$sqrt(16.0)", "\"4\"");
        }

        /// Strings are bit vectors of eight bits per character (§3.6.2).
        #[test]
        fn strings() {
            eval_in("localparam string S = \"hi\";", "S", "\"hi\"");
            eval("\"A\" + 8'd1", "8'd66");
            eval("\"AB\" == \"AB\"", "1'd1");
        }

        /// The constant system functions.
        #[test]
        fn system_functions() {
            eval("$clog2(1)", "32'sd0");
            eval("$clog2(2)", "32'sd1");
            eval("$clog2(100)", "32'sd7");
            eval("$clog2(1024)", "32'sd10");
            eval("$bits(8'h0)", "32'sd8");
            eval_in("logic [3:0][7:0] v;", "$bits(v)", "32'sd32");
            eval_in("logic [7:0] v [0:3];", "$size(v)", "32'sd4");
            eval_in("logic [7:2] v;", "$high(v)", "32'sd7");
            eval_in("logic [7:2] v;", "$low(v)", "32'sd2");
            eval_in("logic [0:7] v;", "$left(v)", "32'sd0");
            eval_in("logic [0:7] v;", "$right(v)", "32'sd7");
            eval("$countones(8'b1011_0000)", "32'sd3");
            eval("$isunknown(4'b10x1)", "1'd1");
        }

        /// Parameters, enum members and constant functions all fold.
        #[test]
        fn names_fold() {
            eval_in("localparam W = 4;", "W + 1", "32'sd5");
            eval_in("localparam logic [7:0] M = 8'hf0;", "M | 8'h0f", "8'd255");
            eval_in("typedef enum logic [1:0] { A, B, C } e_t;", "C", "2'd2");
            eval_in(
                "function automatic int twice(input int x); return x * 2; endfunction",
                "twice(21)",
                "32'sd42",
            );
            eval_in(
                "function automatic int total(input int n);\n                 int acc = 0;\n                 for (int i = 1; i <= n; i++) acc += i;\n                 return acc;\n                 endfunction",
                "total(10)",
                "32'sd55",
            );
        }

        /// Selects of constants.
        #[test]
        fn selects() {
            eval_in("localparam logic [7:0] V = 8'hab;", "V[3:0]", "4'd11");
            eval_in("localparam logic [7:0] V = 8'hab;", "V[7]", "1'd1");
            eval_in("localparam logic [7:0] V = 8'hab;", "V[3 +: 4]", "4'd5");
            eval_in("localparam logic [7:0] V = 8'hab;", "V[7 -: 4]", "4'd10");
            eval_in("localparam logic [0:7] V = 8'hab;", "V[0]", "1'd1");
        }
    }
}
