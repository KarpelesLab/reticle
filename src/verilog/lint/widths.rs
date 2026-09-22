//! Rules about widths: what the source says a value is, versus what it is
//! assigned to.
//!
//! Only mismatches that follow from the text are reported. When either
//! side's width depends on something elaboration decides, or when the
//! right-hand side is an arithmetic expression whose width the assignment
//! context extends, the rules stay quiet.

use crate::diag::Diagnostics;
use crate::source::Span;
use crate::verilog::ast::{AssignOp, Expr, ExprKind, Item, ItemKind, SourceFile, StmtKind};

use super::facts::{ModuleFacts, ProcKind, stmt_exprs, walk_expr, walk_items, walk_proc};
use super::width::{WidthKind, decl_width, expr_width, literal_of, parse_number};
use super::{Level, Lint, LintContext};

/// `width-mismatch`: an assignment whose two sides have different widths.
pub(super) struct WidthMismatch;

impl Lint for WidthMismatch {
    fn id(&self) -> &'static str {
        "L0017"
    }

    fn name(&self) -> &'static str {
        "width-mismatch"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            // Declaration initialisers: `reg [7:0] a = 9'h1ff`.
            for id in 0..m.decls.len() {
                let d = m.decl(id);
                let (Some(init), Some(width)) = (d.init, decl_width(ctx.facts, m, id)) else {
                    continue;
                };
                if !d.unpacked.is_empty() {
                    continue;
                }
                let Some(n) = literal_of(init) else { continue };
                if n.value.is_none() && !n.has_xz {
                    continue;
                }
                if u64::from(n.min_bits) > width {
                    ctx.report(
                        init.span,
                        format!(
                            "literal needs {} bits but `{}` is {width} bits wide",
                            n.min_bits, d.name
                        ),
                    )
                    .secondary(d.span, "declared here")
                    .note("the extra bits are dropped")
                    .emit(diags);
                }
            }

            // Continuous assignments.
            walk_items(&m.module.items, &mut |item: &Item| {
                let ItemKind::ContAssign(ca) = &item.kind else {
                    return;
                };
                for pair in &ca.assigns {
                    self.compare(ctx, m, &pair.lhs, &pair.rhs, pair.span, diags);
                }
            });

            // Procedural assignments.
            for p in &m.procs {
                if matches!(p.kind, ProcKind::Function | ProcKind::Task) {
                    continue;
                }
                walk_proc(p, &mut |s| {
                    let StmtKind::Assign(a) = &s.kind else { return };
                    if !matches!(a.op, AssignOp::Blocking | AssignOp::NonBlocking) {
                        return;
                    }
                    self.compare(ctx, m, &a.lhs, &a.rhs, s.span, diags);
                });
            }
        }
    }
}

impl WidthMismatch {
    /// Reports when both sides have an exactly known, different width.
    fn compare(
        &self,
        ctx: &LintContext<'_>,
        m: &ModuleFacts<'_>,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        diags: &mut Diagnostics,
    ) {
        let (Some(l), Some(r)) = (expr_width(ctx.facts, m, lhs), expr_width(ctx.facts, m, rhs))
        else {
            return;
        };
        if l.kind != WidthKind::Exact || r.kind != WidthKind::Exact || l.bits == r.bits {
            return;
        }
        let what = if r.bits > l.bits {
            "truncated"
        } else {
            "zero-extended"
        };
        ctx.report(
            span,
            format!(
                "width mismatch: the target is {} bits, the value {} bits",
                l.bits, r.bits
            ),
        )
        .secondary(rhs.span, format!("this value is {what}"))
        .emit(diags);
    }
}

/// `unsized-literal-in-concat`: a literal without a width inside `{}`.
pub(super) struct UnsizedLiteralInConcat;

impl Lint for UnsizedLiteralInConcat {
    fn id(&self) -> &'static str {
        "L0018"
    }

    fn name(&self) -> &'static str {
        "unsized-literal-in-concat"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        let mut report = |e: &Expr| {
            walk_expr(e, &mut |x| {
                let elems = match &x.kind {
                    ExprKind::Concat(v) => v,
                    ExprKind::Replicate { elems, .. } => elems,
                    _ => return,
                };
                for elem in elems {
                    let ExprKind::Literal(crate::verilog::ast::Literal::Number { text, span }) =
                        &elem.kind
                    else {
                        continue;
                    };
                    let Some(n) = parse_number(text) else {
                        continue;
                    };
                    if !n.is_unsized() {
                        continue;
                    }
                    ctx.report(
                        *span,
                        format!("unsized literal `{text}` in a concatenation"),
                    )
                    .note("an unsized literal is 32 bits here; write its width, as in `1'b0`")
                    .emit(diags);
                }
            });
        };
        for m in &ctx.facts.modules {
            walk_items(&m.module.items, &mut |item: &Item| {
                if let ItemKind::ContAssign(ca) = &item.kind {
                    for pair in &ca.assigns {
                        report(&pair.lhs);
                        report(&pair.rhs);
                    }
                }
                if let ItemKind::Net(n) = &item.kind {
                    for d in n.decls.iter().filter_map(|d| d.init.as_ref()) {
                        report(d);
                    }
                }
                if let ItemKind::Var(v) = &item.kind {
                    for d in v.decls.iter().filter_map(|d| d.init.as_ref()) {
                        report(d);
                    }
                }
            });
            for p in &m.procs {
                walk_proc(p, &mut |s| {
                    for e in stmt_exprs(s) {
                        report(e);
                    }
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
    fn declaration_initialisers() {
        let src = "module m;\nreg [7:0] a = 9'h1ff;\nreg [7:0] b = 8'hff;\nreg [7:0] c = 1'b0;\n\
                   endmodule\n";
        let out = only(src, "width-mismatch");
        assert!(
            out.contains("literal needs 9 bits but `a` is 8 bits"),
            "{out}"
        );
        assert!(!out.contains("`b`"), "{out}");
        assert!(!out.contains("`c`"), "{out}");
    }

    #[test]
    fn concatenations_and_selects() {
        let src = "module m(input [3:0] b, input [1:0] c, output [7:0] a, output [3:0] d);\n\
                   assign a = {b, c};\n\
                   assign d = {b[1:0], c};\n\
                   endmodule\n";
        let out = only(src, "width-mismatch");
        assert!(
            out.contains("the target is 8 bits, the value 6 bits"),
            "{out}"
        );
        assert!(!out.contains("4 bits, the value 4"), "{out}");
    }

    #[test]
    fn parameterised_widths_are_left_alone() {
        let src = "module m #(parameter W = 4) (input [W-1:0] a, output [W-1:0] y);\n\
                   assign y = a;\n\
                   endmodule\n";
        assert_eq!(only(src, "width-mismatch"), "");

        let src = "module m(input [7:0] a, input [7:0] b, output [7:0] y);\n\
                   assign y = a + b;\n\
                   endmodule\n";
        assert_eq!(only(src, "width-mismatch"), "");
    }

    #[test]
    fn unsized_literals_in_concatenations() {
        let src = "module m(input a, output [2:0] y);\nassign y = {a, 1, 1'b0};\nendmodule\n";
        let out = only(src, "unsized-literal-in-concat");
        assert!(out.contains("unsized literal `1`"), "{out}");
        assert_eq!(out.matches("L0018").count(), 1, "{out}");
    }
}
