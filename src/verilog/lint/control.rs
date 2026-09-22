//! Rules about control flow: case coverage, inferred latches, `casex`,
//! unreachable statements and constant conditions.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::Diagnostics;
use crate::source::Span;
use crate::verilog::ast::{Case, CaseKind, Expr, ExprKind, Literal, SourceFile, Stmt, StmtKind};

use super::facts::{DeclId, DeclKind, ModuleFacts, ProcClass, ProcKind, WriteKind, walk_proc};
use super::width::{WidthKind, const_eval, expr_width, parse_number};
use super::{Level, Lint, LintContext, join_names};

/// The largest subject width the coverage check enumerates.
const MAX_CASE_WIDTH: u64 = 8;

/// How much of its subject's value space a `case` covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coverage {
    /// A `default` arm is present.
    Default,
    /// The arms cover every value.
    Full,
    /// The arms cover `covered` of `total` values.
    Partial {
        /// Values the arms match.
        covered: u128,
        /// Values the subject can take.
        total: u128,
    },
    /// The subject's width or an arm's value is not known statically.
    Unknown,
}

/// Classifies a `case` statement's coverage.
fn coverage(ctx: &LintContext<'_>, m: &ModuleFacts<'_>, case: &Case) -> Coverage {
    if case.items.iter().any(|i| i.patterns.is_empty()) {
        return Coverage::Default;
    }
    let Some(width) = expr_width(ctx.facts, m, &case.expr) else {
        return Coverage::Unknown;
    };
    if width.kind != WidthKind::Exact || width.bits == 0 || width.bits > MAX_CASE_WIDTH {
        return Coverage::Unknown;
    }
    let total = 1u128 << width.bits;
    let wildcards_allowed = case.kind != CaseKind::Case;
    let mut covered = 0u128;
    for item in &case.items {
        for p in &item.patterns {
            match &p.kind {
                ExprKind::ValueRange { low, high } => {
                    let (Some(lo), Some(hi)) = (const_eval(m, low), const_eval(m, high)) else {
                        return Coverage::Unknown;
                    };
                    covered += u128::try_from((hi - lo + 1).max(0)).unwrap_or(0);
                }
                ExprKind::Literal(Literal::Number { text, .. }) => {
                    let Some(n) = parse_number(text) else {
                        return Coverage::Unknown;
                    };
                    if n.has_xz {
                        if !wildcards_allowed {
                            return Coverage::Unknown;
                        }
                        let wild = u64::from(n.wild_bits).min(width.bits);
                        covered += 1u128 << wild;
                    } else {
                        covered += 1;
                    }
                }
                _ => {
                    if const_eval(m, p).is_none() {
                        return Coverage::Unknown;
                    }
                    covered += 1;
                }
            }
        }
    }
    if covered >= total {
        Coverage::Full
    } else {
        Coverage::Partial { covered, total }
    }
}

/// `incomplete-case`: a `case` without `default` that leaves values out.
pub(super) struct IncompleteCase;

impl Lint for IncompleteCase {
    fn id(&self) -> &'static str {
        "L0012"
    }

    fn name(&self) -> &'static str {
        "incomplete-case"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for p in &m.procs {
                if !matches!(p.kind, ProcKind::Always(_) | ProcKind::Initial) {
                    continue;
                }
                walk_proc(p, &mut |s| {
                    let StmtKind::Case(case) = &s.kind else {
                        return;
                    };
                    // `unique`, `unique0` and `priority` assert coverage.
                    if case.qualifier.is_some() {
                        return;
                    }
                    let Coverage::Partial { covered, total } = coverage(ctx, m, case) else {
                        return;
                    };
                    ctx.report(
                        case.expr.span,
                        format!(
                            "`{}` covers {covered} of {total} values and has no `default`",
                            case.kind.as_str()
                        ),
                    )
                    .note("add a `default` arm, or cover the remaining values")
                    .emit(diags);
                });
            }
        }
    }
}

/// What a statement assigns on every path through it.
#[derive(Clone, Debug, Default)]
struct Assigned {
    /// Assigned on every path.
    set: BTreeSet<DeclId>,
    /// Every path leaves the block (`return`, `break`, `$finish`).
    diverges: bool,
}

impl Assigned {
    fn diverging() -> Self {
        Assigned {
            set: BTreeSet::new(),
            diverges: true,
        }
    }

    /// Sequential composition: everything either statement assigns.
    fn then(mut self, other: Assigned) -> Self {
        if self.diverges {
            return self;
        }
        self.set.extend(other.set);
        self.diverges = other.diverges;
        self
    }

    /// Branch composition: only what both branches assign.
    fn merge(self, other: Assigned) -> Self {
        if self.diverges {
            return other;
        }
        if other.diverges {
            return self;
        }
        Assigned {
            set: self.set.intersection(&other.set).copied().collect(),
            diverges: false,
        }
    }
}

/// True when the statement is a `$finish` / `$fatal`, with or without
/// an argument list.
fn is_finish(s: &Stmt) -> bool {
    let StmtKind::Expr(e) = &s.kind else {
        return false;
    };
    let callee = match &e.kind {
        ExprKind::Call { callee, .. } => callee,
        _ => e,
    };
    matches!(&callee.kind, ExprKind::SystemIdent(id) if matches!(id.name.as_str(), "finish" | "fatal"))
}

/// The statements that end a path through their block.
fn diverges(s: &Stmt) -> bool {
    matches!(
        s.kind,
        StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue
    ) || is_finish(s)
}

/// Computes what `s` assigns on every path.
///
/// The analysis is deliberately optimistic where Verilog is ambiguous: a
/// bit or part select counts as assigning the whole variable, and a loop
/// counts as running at least once. Both choices avoid reporting a latch
/// that is not there.
fn assigned(ctx: &LintContext<'_>, m: &ModuleFacts<'_>, s: &Stmt) -> Assigned {
    match &s.kind {
        StmtKind::Assign(a) => {
            let mut out = Assigned::default();
            collect_targets(m, &a.lhs, &mut out.set);
            out
        }
        StmtKind::Block(b) => {
            let mut out = Assigned::default();
            for st in &b.stmts {
                out = out.then(assigned(ctx, m, st));
            }
            out
        }
        StmtKind::If(i) => {
            let then = assigned(ctx, m, &i.then_stmt);
            match &i.else_stmt {
                Some(e) => then.merge(assigned(ctx, m, e)),
                None => Assigned::default(),
            }
        }
        StmtKind::Case(c) => {
            let full = matches!(coverage(ctx, m, c), Coverage::Default | Coverage::Full)
                || c.qualifier.is_some();
            if !full {
                return Assigned::default();
            }
            let mut out: Option<Assigned> = None;
            for item in &c.items {
                let a = assigned(ctx, m, &item.body);
                out = Some(match out {
                    Some(prev) => prev.merge(a),
                    None => a,
                });
            }
            out.unwrap_or_default()
        }
        StmtKind::For(f) => assigned(ctx, m, &f.body),
        StmtKind::While(_, b)
        | StmtKind::DoWhile(b, _)
        | StmtKind::Repeat(_, b)
        | StmtKind::Forever(b)
        | StmtKind::Wait(_, b)
        | StmtKind::Timing(_, b) => assigned(ctx, m, b),
        StmtKind::Foreach(f) => assigned(ctx, m, &f.body),
        StmtKind::Fork(b, _) => {
            let mut out = Assigned::default();
            for st in &b.stmts {
                let a = assigned(ctx, m, st);
                out.set.extend(a.set);
            }
            out
        }
        _ if diverges(s) => Assigned::diverging(),
        _ => Assigned::default(),
    }
}

/// Adds the declarations an assignment target names.
fn collect_targets(m: &ModuleFacts<'_>, lhs: &Expr, out: &mut BTreeSet<DeclId>) {
    match &lhs.kind {
        ExprKind::Concat(v) | ExprKind::Streaming { elems: v, .. } => {
            for e in v {
                collect_targets(m, e, out);
            }
        }
        _ => {
            if let Some(id) = m.root_decl(lhs) {
                out.insert(id);
            }
        }
    }
}

/// `latch-inferred`: a combinational block that does not assign a variable
/// on every path.
pub(super) struct LatchInferred;

impl Lint for LatchInferred {
    fn id(&self) -> &'static str {
        "L0013"
    }

    fn name(&self) -> &'static str {
        "latch-inferred"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for (pid, p) in m.procs.iter().enumerate() {
                if !matches!(p.kind, ProcKind::Always(_)) || p.class != ProcClass::Combinational {
                    continue;
                }
                let Some(body) = p.body() else { continue };
                let always = assigned(ctx, m, body);
                // Every variable the block writes must be assigned on
                // every path through it.
                let mut first_write: BTreeMap<DeclId, Span> = BTreeMap::new();
                for w in m.writes.iter().filter(|w| w.proc == pid) {
                    if !matches!(w.kind, WriteKind::Blocking | WriteKind::NonBlocking) {
                        continue;
                    }
                    let d = m.decl(w.decl);
                    if d.local || !matches!(d.kind, DeclKind::Var | DeclKind::Port(_)) {
                        continue;
                    }
                    first_write.entry(w.decl).or_insert(w.span);
                }
                let latched: Vec<(DeclId, Span)> = first_write
                    .into_iter()
                    .filter(|(id, _)| !always.set.contains(id))
                    .collect();
                if latched.is_empty() {
                    continue;
                }
                let names: BTreeSet<String> = latched
                    .iter()
                    .map(|(id, _)| m.decl(*id).name.clone())
                    .collect();
                let span = latched
                    .iter()
                    .map(|(_, s)| *s)
                    .min_by_key(|s| s.start)
                    .unwrap_or(p.span);
                ctx.report(
                    span,
                    format!(
                        "{} {} not assigned on every path, inferring a latch",
                        join_names(&names),
                        if names.len() == 1 { "is" } else { "are" }
                    ),
                )
                .secondary(p.span, "in this combinational block")
                .note("assign a default value first, or complete every `if` and `case`")
                .emit(diags);
            }
        }
    }
}

/// `case-x-z`: `casex`, and `x` in the arms of a `case`.
pub(super) struct CaseXZ;

impl Lint for CaseXZ {
    fn id(&self) -> &'static str {
        "L0014"
    }

    fn name(&self) -> &'static str {
        "case-x-z"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for p in &m.procs {
                walk_proc(p, &mut |s| {
                    let StmtKind::Case(case) = &s.kind else {
                        return;
                    };
                    if case.kind == CaseKind::Casex {
                        ctx.report(
                            case.expr.span,
                            "`casex` treats `x` in the subject as a wildcard",
                        )
                        .note("use `casez`, whose wildcard is `?` / `z` only")
                        .emit(diags);
                    }
                    for item in &case.items {
                        for pat in &item.patterns {
                            let ExprKind::Literal(Literal::Number { text, span }) = &pat.kind
                            else {
                                continue;
                            };
                            let Some(n) = parse_number(text) else {
                                continue;
                            };
                            let has_x = text.contains(['x', 'X']);
                            match case.kind {
                                CaseKind::Case if n.has_xz => {
                                    ctx.report(
                                        *span,
                                        "an `x` or `z` in a `case` arm matches nothing",
                                    )
                                    .note("`case` compares bit for bit; use `casez` for wildcards")
                                    .emit(diags);
                                }
                                CaseKind::Casez if has_x => {
                                    ctx.report(*span, "`x` in a `casez` arm matches nothing")
                                        .note("`casez` treats `?` and `z` as wildcards, not `x`")
                                        .emit(diags);
                                }
                                _ => {}
                            }
                        }
                    }
                });
            }
        }
    }
}

/// `unreachable-statement`: a statement after one that always leaves.
pub(super) struct UnreachableStatement;

impl Lint for UnreachableStatement {
    fn id(&self) -> &'static str {
        "L0015"
    }

    fn name(&self) -> &'static str {
        "unreachable-statement"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for p in &m.procs {
                walk_proc(p, &mut |s| {
                    let (StmtKind::Block(b) | StmtKind::Fork(b, _)) = &s.kind else {
                        return;
                    };
                    let Some(pos) = b
                        .stmts
                        .iter()
                        .position(|st| diverges(st) && !matches!(st.kind, StmtKind::Decl(_)))
                    else {
                        return;
                    };
                    let Some(next) = b.stmts.get(pos + 1) else {
                        return;
                    };
                    let what = match &b.stmts[pos].kind {
                        StmtKind::Return(_) => "`return`",
                        StmtKind::Break => "`break`",
                        StmtKind::Continue => "`continue`",
                        _ => "`$finish`",
                    };
                    ctx.report(
                        next.span,
                        format!("this statement is unreachable after {what}"),
                    )
                    .secondary(b.stmts[pos].span, "control leaves the block here")
                    .emit(diags);
                });
            }
        }
    }
}

/// The constant value of a literal condition.
fn literal_condition(e: &Expr) -> Option<u128> {
    let ExprKind::Literal(Literal::Number { text, .. }) = &e.kind else {
        return None;
    };
    parse_number(text)?.value
}

/// `constant-condition`: `if (1)`, `while (0)` and friends.
pub(super) struct ConstantCondition;

impl Lint for ConstantCondition {
    fn id(&self) -> &'static str {
        "L0016"
    }

    fn name(&self) -> &'static str {
        "constant-condition"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for p in &m.procs {
                walk_proc(p, &mut |s| match &s.kind {
                    StmtKind::If(i) => {
                        let Some(v) = literal_condition(&i.cond) else {
                            return;
                        };
                        let what = if v == 0 { "false" } else { "true" };
                        ctx.report(i.cond.span, format!("this condition is always {what}"))
                            .note(if v == 0 {
                                "the branch is dead code"
                            } else {
                                "the `else` branch, if any, is dead code"
                            })
                            .emit(diags);
                    }
                    StmtKind::While(c, _) | StmtKind::DoWhile(_, c) => {
                        // `while (1)` is the usual way to write a forever
                        // loop, so only a never-taken loop is reported.
                        if literal_condition(c) == Some(0) {
                            ctx.report(c.span, "this loop condition is always false")
                                .note("the body never runs")
                                .emit(diags);
                        }
                    }
                    StmtKind::Repeat(c, _) if literal_condition(c) == Some(0) => {
                        ctx.report(c.span, "`repeat (0)` never runs its body")
                            .emit(diags);
                    }
                    _ => {}
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::LintConfig;
    use super::super::tests::lint;

    fn only(src: &str, rule: &str) -> String {
        let config = LintConfig::parse(&format!("off:all warn:{rule}")).unwrap();
        lint(src, &config)
    }

    #[test]
    fn incomplete_case_counts_values() {
        let src = "module m(input [1:0] s, output reg [1:0] y);\n\
                   always @* case (s) 2'd0: y = 0; 2'd1: y = 1; endcase\n\
                   endmodule\n";
        assert!(only(src, "incomplete-case").contains("covers 2 of 4 values"));

        let full = "module m(input [1:0] s, output reg [1:0] y);\n\
                    always @* case (s) 2'd0: y = 0; 2'd1: y = 1; 2'd2: y = 2; 2'd3: y = 3; endcase\n\
                    endmodule\n";
        assert_eq!(only(full, "incomplete-case"), "");

        let wide = "module m(input [15:0] s, output reg y);\n\
                    always @* case (s) 16'd0: y = 0; endcase\n\
                    endmodule\n";
        assert_eq!(only(wide, "incomplete-case"), "");

        let casez = "module m(input [1:0] s, output reg y);\n\
                     always @* casez (s) 2'b0?: y = 0; 2'b1?: y = 1; endcase\n\
                     endmodule\n";
        assert_eq!(only(casez, "incomplete-case"), "");
    }

    #[test]
    fn latches_are_inferred_from_incomplete_paths() {
        let src = "module m(input a, input b, output reg y);\n\
                   always @* if (a) y = b;\n\
                   endmodule\n";
        assert!(only(src, "latch-inferred").contains("`y` is not assigned on every path"));

        let guarded = "module m(input a, input b, output reg y);\n\
                       always @* begin y = 1'b0; if (a) y = b; end\n\
                       endmodule\n";
        assert_eq!(only(guarded, "latch-inferred"), "");

        let complete = "module m(input a, input b, output reg y);\n\
                        always @* if (a) y = b; else y = 1'b0;\n\
                        endmodule\n";
        assert_eq!(only(complete, "latch-inferred"), "");

        let case_latch = "module m(input [1:0] s, output reg y);\n\
                          always @* case (s) 2'd0: y = 1'b1; 2'd1: y = 1'b0; endcase\n\
                          endmodule\n";
        assert!(only(case_latch, "latch-inferred").contains("inferring a latch"));

        // A sequential block infers a flip-flop, not a latch.
        let ff = "module m(input clk, input a, output reg y);\n\
                  always @(posedge clk) if (a) y <= 1'b1;\n\
                  endmodule\n";
        assert_eq!(only(ff, "latch-inferred"), "");
    }

    #[test]
    fn case_x_and_z() {
        let src = "module m(input [3:0] s, output reg y);\n\
                   always @* casex (s) 4'b1xxx: y = 1; default: y = 0; endcase\n\
                   endmodule\n";
        assert!(only(src, "case-x-z").contains("`casex` treats"));

        let plain = "module m(input [3:0] s, output reg y);\n\
                     always @* case (s) 4'b1x0z: y = 1; default: y = 0; endcase\n\
                     endmodule\n";
        assert!(only(plain, "case-x-z").contains("matches nothing"));

        let casez = "module m(input [3:0] s, output reg y);\n\
                     always @* casez (s) 4'b1??x: y = 1; default: y = 0; endcase\n\
                     endmodule\n";
        assert!(only(casez, "case-x-z").contains("`x` in a `casez` arm"));

        let ok = "module m(input [3:0] s, output reg y);\n\
                  always @* casez (s) 4'b1???: y = 1; default: y = 0; endcase\n\
                  endmodule\n";
        assert_eq!(only(ok, "case-x-z"), "");
    }

    #[test]
    fn unreachable_and_constant() {
        let src = "module m;\n\
                   function int f(input int x);\n\
                   begin\n return x;\n f = 0;\n end\n endfunction\n\
                   initial begin\n $finish;\n $display(\"no\");\n end\n\
                   endmodule\n";
        let out = only(src, "unreachable-statement");
        assert!(out.contains("unreachable after `return`"), "{out}");
        assert!(out.contains("unreachable after `$finish`"), "{out}");

        let src = "module m(output reg y);\n\
                   initial begin\n if (1) y = 0;\n while (0) y = 1;\n end\n\
                   endmodule\n";
        let out = only(src, "constant-condition");
        assert!(out.contains("always true"), "{out}");
        assert!(out.contains("always false"), "{out}");

        let forever = "module m(output reg y);\ninitial while (1) y = ~y;\nendmodule\n";
        assert_eq!(only(forever, "constant-condition"), "");
    }
}
