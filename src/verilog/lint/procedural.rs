//! Rules about procedural blocks: assignment styles, sensitivity lists and
//! reset conventions.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::Diagnostics;
use crate::source::Span;
use crate::verilog::ast::{
    AssignOp, Edge, EventControlKind, EventExpr, Expr, ExprKind, SourceFile, Stmt, StmtKind,
    UnaryOp,
};

use super::facts::{DeclId, ModuleFacts, Proc, ProcClass, ProcKind, WriteKind, walk_proc};
use super::{Level, Lint, LintContext, join_names};

/// The `always` blocks of a module, with their index.
fn always_procs<'a, 'b>(
    m: &'b ModuleFacts<'a>,
) -> impl Iterator<Item = (usize, &'b Proc<'a>)> + use<'a, 'b> {
    m.procs
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.kind, ProcKind::Always(_)))
}

/// The keyword that introduced the block, for messages.
fn keyword(p: &Proc<'_>) -> &'static str {
    match p.kind {
        ProcKind::Always(k) => k.as_str(),
        _ => "always",
    }
}

/// The declaration an assignment target names, when it is a module-level
/// signal.
fn target_decl(m: &ModuleFacts<'_>, lhs: &Expr) -> Option<DeclId> {
    let id = m.root_decl(lhs)?;
    let d = m.decl(id);
    (d.is_signal() && !d.local).then_some(id)
}

/// `blocking-in-sequential`: `=` inside an edge-triggered block.
pub(super) struct BlockingInSequential;

impl Lint for BlockingInSequential {
    fn id(&self) -> &'static str {
        "L0007"
    }

    fn name(&self) -> &'static str {
        "blocking-in-sequential"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for (_, p) in always_procs(m) {
                if p.class != ProcClass::Sequential {
                    continue;
                }
                walk_proc(p, &mut |s| {
                    let StmtKind::Assign(a) = &s.kind else { return };
                    if a.op == AssignOp::NonBlocking {
                        return;
                    }
                    let Some(id) = target_decl(m, &a.lhs) else {
                        return;
                    };
                    ctx.report(
                        s.span,
                        format!(
                            "blocking assignment to `{}` in an edge-triggered block",
                            m.decl(id).name
                        ),
                    )
                    .note(format!(
                        "use `<=` in `{}`; blocking assignments to a register race with reads of it",
                        keyword(p)
                    ))
                    .emit(diags);
                });
            }
        }
    }
}

/// `nonblocking-in-comb`: `<=` inside a combinational or latch block.
pub(super) struct NonBlockingInComb;

impl Lint for NonBlockingInComb {
    fn id(&self) -> &'static str {
        "L0008"
    }

    fn name(&self) -> &'static str {
        "nonblocking-in-comb"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for (_, p) in always_procs(m) {
                if !matches!(p.class, ProcClass::Combinational | ProcClass::Latch) {
                    continue;
                }
                walk_proc(p, &mut |s| {
                    let StmtKind::Assign(a) = &s.kind else { return };
                    if a.op != AssignOp::NonBlocking {
                        return;
                    }
                    let Some(id) = target_decl(m, &a.lhs) else {
                        return;
                    };
                    ctx.report(
                        s.span,
                        format!(
                            "non-blocking assignment to `{}` in a combinational block",
                            m.decl(id).name
                        ),
                    )
                    .note(format!("use `=` in `{}`", keyword(p)))
                    .emit(diags);
                });
            }
        }
    }
}

/// `mixed-assignment-styles`: one variable written with both `=` and `<=`.
pub(super) struct MixedAssignmentStyles;

impl Lint for MixedAssignmentStyles {
    fn id(&self) -> &'static str {
        "L0009"
    }

    fn name(&self) -> &'static str {
        "mixed-assignment-styles"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for id in 0..m.decls.len() {
                let d = m.decl(id);
                if !d.is_signal() || d.local {
                    continue;
                }
                let mut blocking: Option<Span> = None;
                let mut nonblocking: Option<Span> = None;
                for &wi in &d.writes {
                    let w = &m.writes[wi];
                    // Only procedural blocks count; an `initial` that
                    // seeds a register is the usual, harmless exception.
                    if !matches!(
                        m.procs[w.proc].kind,
                        ProcKind::Always(_) | ProcKind::Function | ProcKind::Task
                    ) {
                        continue;
                    }
                    match w.kind {
                        WriteKind::Blocking => blocking.get_or_insert(w.span),
                        WriteKind::NonBlocking => nonblocking.get_or_insert(w.span),
                        _ => continue,
                    };
                }
                let (Some(b), Some(nb)) = (blocking, nonblocking) else {
                    continue;
                };
                let (first, second) = if b.start <= nb.start {
                    (b, nb)
                } else {
                    (nb, b)
                };
                ctx.report(
                    second,
                    format!("`{}` is assigned with both `=` and `<=`", d.name),
                )
                .secondary(first, "first assigned here")
                .note("mixing the two styles for one variable is not synthesisable")
                .emit(diags);
            }
        }
    }
}

/// The root identifier of an event expression.
fn event_root(e: &EventExpr) -> Option<&crate::verilog::ast::Ident> {
    fn root(e: &Expr) -> Option<&crate::verilog::ast::Ident> {
        match &e.kind {
            ExprKind::Ident(id) => Some(id),
            ExprKind::Index { base, .. }
            | ExprKind::Range { base, .. }
            | ExprKind::Member { base, .. } => root(base),
            _ => None,
        }
    }
    root(&e.expr)
}

/// `sensitivity-list`: incomplete lists, mixed edges and empty `@*`.
pub(super) struct SensitivityList;

impl Lint for SensitivityList {
    fn id(&self) -> &'static str {
        "L0010"
    }

    fn name(&self) -> &'static str {
        "sensitivity-list"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for (pid, p) in always_procs(m) {
                let Some(sens) = p.sens else { continue };
                match &sens.kind {
                    EventControlKind::Any => {
                        if !m.reads.iter().any(|r| r.proc == pid) {
                            ctx.report(sens.span, "`always @*` reads nothing")
                                .note("the block will never run again after time zero")
                                .emit(diags);
                        }
                    }
                    EventControlKind::List(list) => {
                        let edges = list.iter().filter(|e| e.edge.is_some()).count();
                        if edges > 0 && edges < list.len() {
                            let level = list.iter().find(|e| e.edge.is_none()).unwrap();
                            ctx.report(
                                level.span,
                                "sensitivity list mixes edge and level sensitivity",
                            )
                            .secondary(sens.span, "in this list")
                            .note("an edge-triggered block must list only edges")
                            .emit(diags);
                        }
                        if edges == 0 {
                            self.check_complete(ctx, m, pid, p, list, diags);
                        }
                    }
                }
            }
        }
    }
}

impl SensitivityList {
    /// Reports the signals a level-sensitive list leaves out.
    fn check_complete(
        &self,
        ctx: &LintContext<'_>,
        m: &ModuleFacts<'_>,
        pid: usize,
        p: &Proc<'_>,
        list: &[EventExpr],
        diags: &mut Diagnostics,
    ) {
        let sens_span = p.sens.map(|s| s.span).unwrap_or(p.span);
        let listed: BTreeSet<DeclId> = list
            .iter()
            .filter_map(event_root)
            .filter_map(|i| m.lookup(&i.name))
            .collect();
        // Everything written in the block is either a temporary or a
        // target; neither has to be listed.
        let written: BTreeSet<DeclId> = m
            .writes
            .iter()
            .filter(|w| w.proc == pid)
            .map(|w| w.decl)
            .collect();
        let mut missing: BTreeMap<String, Span> = BTreeMap::new();
        for r in m.reads.iter().filter(|r| r.proc == pid) {
            if listed.contains(&r.decl) || written.contains(&r.decl) {
                continue;
            }
            // Reads inside the sensitivity list itself are the list.
            if r.span.start >= sens_span.start && r.span.end <= sens_span.end {
                continue;
            }
            let d = m.decl(r.decl);
            if !d.is_signal() || d.local {
                continue;
            }
            missing.entry(d.name.clone()).or_insert(r.span);
        }
        if missing.is_empty() {
            return;
        }
        let names: BTreeSet<String> = missing.keys().cloned().collect();
        let first = missing.values().min_by_key(|s| s.start).copied();
        let mut report = ctx
            .report(
                sens_span,
                format!("sensitivity list leaves out {}", join_names(&names)),
            )
            .note("use `always @*` so the list cannot go stale");
        if let Some(first) = first {
            report = report.secondary(first, "read here");
        }
        report.emit(diags);
    }
}

/// True when the name looks like a reset.
fn looks_like_reset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("rst") || lower.contains("reset")
}

/// The identifier an `if` condition tests, and whether it is inverted.
fn tested_signal(cond: &Expr) -> Option<(&crate::verilog::ast::Ident, bool)> {
    match &cond.kind {
        ExprKind::Ident(id) => Some((id, false)),
        ExprKind::Unary {
            op: UnaryOp::LogicNot | UnaryOp::BitNot,
            operand,
        } => tested_signal(operand).map(|(id, inv)| (id, !inv)),
        ExprKind::Binary { op, lhs, rhs } => {
            use crate::verilog::ast::BinaryOp as B;
            let zero = |e: &Expr| matches!(&e.kind, ExprKind::Literal(l) if is_zero(l));
            let one = |e: &Expr| matches!(&e.kind, ExprKind::Literal(l) if is_one(l));
            match op {
                B::Eq | B::CaseEq => {
                    if zero(rhs) {
                        tested_signal(lhs).map(|(id, inv)| (id, !inv))
                    } else if one(rhs) {
                        tested_signal(lhs)
                    } else {
                        None
                    }
                }
                B::Ne | B::CaseNe => {
                    if zero(rhs) {
                        tested_signal(lhs)
                    } else if one(rhs) {
                        tested_signal(lhs).map(|(id, inv)| (id, !inv))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// True for a literal whose value is zero.
fn is_zero(l: &crate::verilog::ast::Literal) -> bool {
    matches!(l, crate::verilog::ast::Literal::Number { text, .. }
        if super::width::parse_number(text).is_some_and(|n| n.value == Some(0)))
}

/// True for a literal whose value is one.
fn is_one(l: &crate::verilog::ast::Literal) -> bool {
    matches!(l, crate::verilog::ast::Literal::Number { text, .. }
        if super::width::parse_number(text).is_some_and(|n| n.value == Some(1)))
}

/// The `if` statements of a block's top-level chain, in order.
fn if_chain<'a>(body: &'a Stmt, out: &mut Vec<&'a crate::verilog::ast::If>) {
    match &body.kind {
        StmtKind::If(i) => {
            out.push(i);
            if let Some(e) = &i.else_stmt {
                if_chain(e, out);
            }
        }
        StmtKind::Block(b) => {
            if let Some(first) = b.stmts.first()
                && b.stmts.len() == 1
            {
                if_chain(first, out);
            } else if let Some(first) = b.stmts.iter().find(|s| matches!(s.kind, StmtKind::If(_))) {
                if_chain(first, out);
            }
        }
        StmtKind::Timing(_, inner) => if_chain(inner, out),
        _ => {}
    }
}

/// `reset-style`: async reset polarity, ordering and mixing.
pub(super) struct ResetStyle;

impl Lint for ResetStyle {
    fn id(&self) -> &'static str {
        "L0011"
    }

    fn name(&self) -> &'static str {
        "reset-style"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for (_, p) in always_procs(m) {
                if p.class != ProcClass::Sequential {
                    continue;
                }
                let Some(sens) = p.sens else { continue };
                let EventControlKind::List(list) = &sens.kind else {
                    continue;
                };
                let edges: Vec<&EventExpr> = list.iter().filter(|e| e.edge.is_some()).collect();
                // The first edge is the clock; any further edge is an
                // asynchronous reset (or set).
                let asyncs = &edges[1.min(edges.len())..];
                let Some(body) = p.body() else { continue };
                let mut chain = Vec::new();
                if_chain(body, &mut chain);

                for (i, ev) in asyncs.iter().enumerate() {
                    let Some(name) = event_root(ev) else { continue };
                    let want_active_low = ev.edge == Some(Edge::Negedge);
                    let position = chain.iter().position(|c| {
                        tested_signal(&c.cond).is_some_and(|(id, _)| id.name == name.name)
                    });
                    match position {
                        None => {
                            ctx.report(
                                ev.span,
                                format!(
                                    "asynchronous `{} {}` is not tested in the block",
                                    ev.edge.map_or("", Edge::as_str),
                                    name.name
                                ),
                            )
                            .secondary(body.span, "this body never checks it")
                            .note("an asynchronous reset must be the first condition of the block")
                            .emit(diags);
                        }
                        Some(pos) => {
                            let cond = &chain[pos].cond;
                            let (_, inverted) = tested_signal(cond).expect("tested above");
                            if inverted != want_active_low {
                                let edge = ev.edge.map_or("", Edge::as_str);
                                let want = if want_active_low { "`!" } else { "`" };
                                ctx.report(
                                    cond.span,
                                    format!(
                                        "reset `{}` is tested with the wrong polarity",
                                        name.name
                                    ),
                                )
                                .secondary(ev.span, format!("declared as `{edge}`"))
                                .note(format!(
                                    "`{edge} {0}` resets when the level is {1}, so test {want}{0}`",
                                    name.name,
                                    if want_active_low { "low" } else { "high" }
                                ))
                                .emit(diags);
                            }
                            if pos > i {
                                ctx.report(
                                    cond.span,
                                    format!("reset `{}` is not tested first", name.name),
                                )
                                .secondary(chain[0].cond.span, "this condition comes before it")
                                .note(
                                    "an asynchronous reset must take priority over everything else",
                                )
                                .emit(diags);
                            }
                        }
                    }
                }

                // A synchronous reset next to an asynchronous one.
                if !asyncs.is_empty() {
                    let async_names: BTreeSet<&str> = asyncs
                        .iter()
                        .filter_map(|e| event_root(e))
                        .map(|i| i.name.as_str())
                        .collect();
                    for c in &chain {
                        let Some((id, _)) = tested_signal(&c.cond) else {
                            continue;
                        };
                        if looks_like_reset(&id.name) && !async_names.contains(id.name.as_str()) {
                            ctx.report(
                                c.cond.span,
                                format!(
                                    "synchronous reset `{}` in a block with an asynchronous reset",
                                    id.name
                                ),
                            )
                            .secondary(sens.span, "asynchronous reset declared here")
                            .note("mixing the two reset styles in one block confuses synthesis")
                            .emit(diags);
                        }
                    }
                }
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
    fn assignment_styles() {
        let src = "module m(input clk, input a, output reg q, output reg y);\n\
                   always @(posedge clk) q = a;\n\
                   always @* y <= a;\n\
                   initial q = 1'b0;\n\
                   endmodule\n";
        assert!(only(src, "blocking-in-sequential").contains("blocking assignment to `q`"));
        assert!(only(src, "nonblocking-in-comb").contains("non-blocking assignment to `y`"));
        // The `initial` is exempt, so only the always block is mixed.
        assert_eq!(only(src, "mixed-assignment-styles"), "");

        let mixed = "module m(input clk, input a, output reg q);\n\
                     always @(posedge clk) q <= a;\n\
                     always @* q = a;\n\
                     endmodule\n";
        assert!(only(mixed, "mixed-assignment-styles").contains("both `=` and `<=`"));
    }

    #[test]
    fn sensitivity_lists() {
        let src = "module m(input a, input b, output reg y);\n\
                   always @(a) y = a & b;\n\
                   endmodule\n";
        let out = only(src, "sensitivity-list");
        assert!(out.contains("leaves out `b`"), "{out}");

        let src = "module m(input clk, input a, output reg y);\n\
                   always @(posedge clk or a) y <= a;\n\
                   endmodule\n";
        assert!(only(src, "sensitivity-list").contains("mixes edge and level"));

        let src = "module m(output reg y);\nalways @* y = 1'b0;\nendmodule\n";
        assert!(only(src, "sensitivity-list").contains("reads nothing"));

        // A temporary assigned before it is read need not be listed.
        let src = "module m(input a, output reg y);\n\
                   reg t;\n\
                   always @(a) begin t = a; y = t; end\n\
                   endmodule\n";
        assert_eq!(only(src, "sensitivity-list"), "");
    }

    #[test]
    fn reset_style() {
        let good = "module m(input clk, input rst_n, input d, output reg q);\n\
                    always @(posedge clk or negedge rst_n)\n\
                    if (!rst_n) q <= 1'b0; else q <= d;\n\
                    endmodule\n";
        assert_eq!(only(good, "reset-style"), "");

        let bad = "module m(input clk, input rst_n, input d, output reg q);\n\
                   always @(posedge clk or negedge rst_n)\n\
                   if (rst_n) q <= 1'b0; else q <= d;\n\
                   endmodule\n";
        assert!(only(bad, "reset-style").contains("wrong polarity"));

        let late = "module m(input clk, input rst, input en, input d, output reg q);\n\
                    always @(posedge clk or posedge rst)\n\
                    if (en) q <= d; else if (rst) q <= 1'b0;\n\
                    endmodule\n";
        let out = only(late, "reset-style");
        assert!(out.contains("not tested first"), "{out}");

        let missing = "module m(input clk, input rst, input d, output reg q);\n\
                       always @(posedge clk or posedge rst) q <= d;\n\
                       endmodule\n";
        assert!(only(missing, "reset-style").contains("is not tested in the block"));

        let mixed = "module m(input clk, input arst_n, input srst, input d, output reg q);\n\
                     always @(posedge clk or negedge arst_n)\n\
                     if (!arst_n) q <= 1'b0; else if (srst) q <= 1'b0; else q <= d;\n\
                     endmodule\n";
        assert!(only(mixed, "reset-style").contains("synchronous reset `srst`"));
    }
}
