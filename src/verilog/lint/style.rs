//! Style and portability rules: port connections, port list style,
//! `` `default_nettype ``, reserved words, deprecated constructs, comment
//! markers and naming conventions.

use crate::diag::Diagnostics;
use crate::source::Span;
use crate::verilog::ast::{
    Edge, EventControlKind, ExprKind, ParamKind, Ports, SourceFile, StmtKind,
};
use crate::verilog::token::Keyword;

use super::facts::{DeclKind, ModuleFacts, ProcClass, ProcKind, walk_proc};
use super::{Level, Lint, LintContext};

/// Instances with more connections than this are hard to read positionally.
const MAX_POSITIONAL_PORTS: usize = 3;

/// `port-connection`: positional connections, `.*` and open ports.
pub(super) struct PortConnection;

impl Lint for PortConnection {
    fn id(&self) -> &'static str {
        "L0019"
    }

    fn name(&self) -> &'static str {
        "port-connection"
    }

    fn default_level(&self) -> Level {
        Level::NOTE
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            for inst in &m.instances {
                let positional = inst.conns.iter().filter(|c| c.positional).count();
                if positional > MAX_POSITIONAL_PORTS {
                    ctx.report(
                        inst.span,
                        format!(
                            "instance of `{}` connects {positional} ports by position",
                            inst.module.name
                        ),
                    )
                    .note("name the connections (`.clk(clk)`) so a port list change is caught")
                    .emit(diags);
                }
                if inst.wildcard {
                    ctx.report(
                        inst.span,
                        format!("instance of `{}` uses `.*`", inst.module.name),
                    )
                    .note("`.*` hides which signals are connected; name them instead")
                    .emit(diags);
                }
                for (pos, c) in inst.conns.iter().enumerate().filter(|(_, c)| c.open) {
                    let what = match c.name {
                        Some(name) => format!("`{}`", name.name),
                        None => format!("port {} of `{}`", pos + 1, inst.module.name),
                    };
                    ctx.report(c.span, format!("{what} is left unconnected"))
                        .note("an unconnected input floats; an unconnected output is fine")
                        .emit(diags);
                }
            }
        }
    }
}

/// `non-ansi-ports`: a Verilog-1995 port list.
pub(super) struct NonAnsiPorts;

impl Lint for NonAnsiPorts {
    fn id(&self) -> &'static str {
        "L0020"
    }

    fn name(&self) -> &'static str {
        "non-ansi-ports"
    }

    fn default_level(&self) -> Level {
        Level::NOTE
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if !matches!(m.module.ports, Ports::NonAnsi(_)) {
                continue;
            }
            ctx.report(
                m.module.name.span,
                format!("`{}` uses a Verilog-1995 port list", m.name()),
            )
            .note("declare the direction and type in the header: `module m(input wire clk, ...)`")
            .emit(diags);
        }
    }
}

/// `missing-default-nettype`: the file never turns implicit nets off.
pub(super) struct MissingDefaultNettype;

impl Lint for MissingDefaultNettype {
    fn id(&self) -> &'static str {
        "L0021"
    }

    fn name(&self) -> &'static str {
        "missing-default-nettype"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        if ctx.facts.modules.is_empty() {
            return;
        }
        if ctx
            .facts
            .directives
            .iter()
            .any(|d| d.name == "default_nettype" && d.args.trim() == "none")
        {
            return;
        }
        let span = ctx
            .facts
            .modules
            .first()
            .map_or_else(|| ctx.file_span(), |m| m.module.name.span);
        ctx.report(span, "the file never sets `` `default_nettype none ``")
            .note("without it a typo in a signal name becomes a silent one-bit wire")
            .emit(diags);
    }
}

/// `keyword-as-identifier`: a name reserved by a later dialect.
pub(super) struct KeywordAsIdentifier;

impl Lint for KeywordAsIdentifier {
    fn id(&self) -> &'static str {
        "L0022"
    }

    fn name(&self) -> &'static str {
        "keyword-as-identifier"
    }

    fn default_level(&self) -> Level {
        Level::WARN
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        let strict = ctx.config.strict_dialect;
        if strict <= ctx.dialect {
            return;
        }
        let mut report = |name: &str, span: Span, what: &str| {
            let Some(kw) = Keyword::lookup(name, strict) else {
                return;
            };
            if kw.since() <= ctx.dialect {
                return;
            }
            ctx.report(
                span,
                format!("{what} `{name}` is a reserved word in {}", kw.since()),
            )
            .note(format!(
                "the file is parsed as {}; rename it so the code stays portable",
                ctx.dialect
            ))
            .emit(diags);
        };
        for m in &ctx.facts.modules {
            report(m.name(), m.module.name.span, m.module.kind.as_str());
            for d in &m.decls {
                if d.local {
                    continue;
                }
                let what = match d.kind {
                    DeclKind::Net(_) | DeclKind::Var | DeclKind::Port(_) => "signal",
                    DeclKind::Param(_) => "parameter",
                    DeclKind::Instance => "instance",
                    DeclKind::Function => "function",
                    DeclKind::Task => "task",
                    _ => continue,
                };
                report(&d.name, d.span, what);
            }
        }
    }
}

/// `deprecated-construct`: `defparam`, `` `include `` of a `.v` file and
/// `wait` in a synthesisable module.
pub(super) struct DeprecatedConstruct;

impl Lint for DeprecatedConstruct {
    fn id(&self) -> &'static str {
        "L0023"
    }

    fn name(&self) -> &'static str {
        "deprecated-construct"
    }

    fn default_level(&self) -> Level {
        Level::NOTE
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for &span in &ctx.facts.defparams {
            ctx.report(span, "`defparam` is deprecated")
                .note("override the parameter at the instance: `m #(.W(8)) u0 (...)`")
                .emit(diags);
        }

        for m in &ctx.facts.modules {
            if !m.looks_synthesisable() {
                continue;
            }
            for p in &m.procs {
                walk_proc(p, &mut |s| {
                    if matches!(s.kind, StmtKind::Wait(_, _) | StmtKind::WaitFork) {
                        ctx.report(s.span, "`wait` is not synthesisable")
                            .note("use an `always` block sensitive to the condition's signals")
                            .emit(diags);
                    }
                });
            }
        }

        // The preprocessor consumes `` `include ``, so the source text is
        // where the directive can still be seen.
        let text = ctx.text();
        let mut offset = 0usize;
        for line in text.split_inclusive('\n') {
            if let Some(col) = line.find("`include")
                && let Some(open) = line[col..].find('"')
                && let Some(close) = line[col + open + 1..].find('"')
            {
                let path = &line[col + open + 1..col + open + 1 + close];
                if path.ends_with(".v") || path.ends_with(".sv") {
                    let start = u32::try_from(offset + col).unwrap_or(0);
                    let end = u32::try_from(offset + col + open + close + 2).unwrap_or(start);
                    ctx.report(
                        Span::new(ctx.source, start, end),
                        format!("`` `include `` of the design file `{path}`"),
                    )
                    .note("include headers (`.vh`, `.svh`) only; list design files in the build")
                    .emit(diags);
                }
            }
            offset += line.len();
        }
    }
}

/// Comment markers `todo-comment` reports.
const TODO_MARKERS: &[&str] = &["TODO", "FIXME", "XXX", "HACK"];

/// `todo-comment`: a comment left as a reminder.
pub(super) struct TodoComment;

impl Lint for TodoComment {
    fn id(&self) -> &'static str {
        "L0024"
    }

    fn name(&self) -> &'static str {
        "todo-comment"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        let text = ctx.text();
        for (span, _) in ctx.comments {
            if span.file != ctx.source {
                continue;
            }
            let start = span.start as usize;
            let end = (span.end as usize).min(text.len());
            if start >= end {
                continue;
            }
            let body = &text[start..end];
            let Some(marker) = TODO_MARKERS.iter().find(|m| body.contains(**m)) else {
                continue;
            };
            ctx.report(*span, format!("{marker} comment")).emit(diags);
        }
    }
}

/// True for `snake_case` (lower-case letters, digits and underscores).
fn is_snake_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// True for `UPPER_CASE`.
fn is_upper_case(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// True when the name reads as a clock.
fn is_clock_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("clk") || lower.contains("clock")
}

/// True when the name marks an active-low signal.
fn is_active_low_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with("_n") || lower.ends_with("_b") || lower.ends_with("_neg")
}

/// `naming`: the project conventions this linter can check without
/// regular expressions.
pub(super) struct Naming;

impl Lint for Naming {
    fn id(&self) -> &'static str {
        "L0025"
    }

    fn name(&self) -> &'static str {
        "naming"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for m in &ctx.facts.modules {
            if !is_snake_case(m.name()) {
                ctx.report(
                    m.module.name.span,
                    format!(
                        "{} `{}` is not snake_case",
                        m.module.kind.as_str(),
                        m.name()
                    ),
                )
                .emit(diags);
            }
            for d in &m.decls {
                if d.local {
                    continue;
                }
                match d.kind {
                    DeclKind::Param(ParamKind::Parameter | ParamKind::Localparam)
                        if !is_upper_case(&d.name) =>
                    {
                        ctx.report(d.span, format!("parameter `{}` is not UPPER_CASE", d.name))
                            .emit(diags);
                    }
                    DeclKind::Net(_) | DeclKind::Var | DeclKind::Port(_)
                        if !is_snake_case(&d.name) =>
                    {
                        ctx.report(d.span, format!("signal `{}` is not snake_case", d.name))
                            .emit(diags);
                    }
                    _ => {}
                }
            }
            self.check_edges(ctx, m, diags);
        }
    }
}

impl Naming {
    /// Clocks are named `clk*`; a signal used on `negedge` is active low.
    fn check_edges(&self, ctx: &LintContext<'_>, m: &ModuleFacts<'_>, diags: &mut Diagnostics) {
        for p in &m.procs {
            if !matches!(p.kind, ProcKind::Always(_)) || p.class != ProcClass::Sequential {
                continue;
            }
            let Some(sens) = p.sens else { continue };
            let EventControlKind::List(list) = &sens.kind else {
                continue;
            };
            let mut edges = list.iter().filter(|e| e.edge.is_some());
            if let Some(clock) = edges.next()
                && let ExprKind::Ident(id) = &clock.expr.kind
                && !is_clock_name(&id.name)
            {
                ctx.report(
                    clock.span,
                    format!("clock `{}` is not named `clk...`", id.name),
                )
                .emit(diags);
            }
            for ev in list.iter().filter(|e| e.edge == Some(Edge::Negedge)) {
                let ExprKind::Ident(id) = &ev.expr.kind else {
                    continue;
                };
                if is_clock_name(&id.name) || is_active_low_name(&id.name) {
                    continue;
                }
                ctx.report(
                    ev.span,
                    format!("`{}` is active low but not named `{}_n`", id.name, id.name),
                )
                .note("suffix an active-low signal with `_n`")
                .emit(diags);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::lint;
    use super::super::{Level, LintConfig};

    fn only(src: &str, rule: &str) -> String {
        let config = LintConfig::parse(&format!("off:all warn:{rule}")).unwrap();
        lint(src, &config)
    }

    #[test]
    fn port_connection_style() {
        let src = "module leaf(input a, input b, input c, input d, output y);\n\
                   assign y = a & b & c & d;\nendmodule\n\
                   module top(input w, output y);\n\
                   leaf u0 (w, w, w, w, y);\n\
                   leaf u1 (.a(w), .b(w), .c(w), .d(w), .y());\n\
                   leaf u2 (.*);\n\
                   endmodule\n";
        let out = only(src, "port-connection");
        assert!(out.contains("connects 5 ports by position"), "{out}");
        assert!(out.contains("`y` is left unconnected"), "{out}");
        assert!(out.contains("uses `.*`"), "{out}");
    }

    #[test]
    fn non_ansi_and_default_nettype() {
        let src = "module m(a, y);\ninput a;\noutput y;\nassign y = a;\nendmodule\n";
        assert!(only(src, "non-ansi-ports").contains("Verilog-1995 port list"));

        let mut config = LintConfig::new();
        config
            .set_level("missing-default-nettype", Level::WARN)
            .unwrap();
        config.set_level("non-ansi-ports", Level::Off).unwrap();
        assert!(lint(src, &config).contains("never sets"));

        let with =
            "`default_nettype none\nmodule m(input a, output y);\nassign y = a;\nendmodule\n";
        assert!(!lint(with, &config).contains("never sets"));
    }

    #[test]
    fn deprecated_constructs() {
        let src = "module m(input clk, input a, output reg y);\n\
                   defparam u0.W = 4;\n\
                   always @(posedge clk) begin wait (a) y <= a; end\n\
                   endmodule\n";
        let out = only(src, "deprecated-construct");
        assert!(out.contains("`defparam` is deprecated"), "{out}");
        assert!(out.contains("`wait` is not synthesisable"), "{out}");

        let inc = "`ifdef USE_EXTERNAL\n`include \"design.v\"\n`endif\n\
                   module m(input a, output y);\nassign y = a;\nendmodule\n";
        assert!(only(inc, "deprecated-construct").contains("design.v"));
    }

    #[test]
    fn naming_conventions() {
        let src = "module BadName #(parameter width = 4) (input CK, input rst, output reg q);\n\
                   always @(posedge CK or negedge rst) q <= 1'b0;\n\
                   endmodule\n";
        let out = only(src, "naming");
        assert!(out.contains("`BadName` is not snake_case"), "{out}");
        assert!(out.contains("parameter `width` is not UPPER_CASE"), "{out}");
        assert!(out.contains("signal `CK` is not snake_case"), "{out}");
        assert!(out.contains("clock `CK` is not named"), "{out}");
        assert!(out.contains("`rst` is active low"), "{out}");

        let good = "module good_name #(parameter WIDTH = 4) (input clk, input rst_n, output reg q);\n\
                    always @(posedge clk or negedge rst_n) q <= 1'b0;\n\
                    endmodule\n";
        assert_eq!(only(good, "naming"), "");
    }

    #[test]
    fn todo_comments() {
        let src = "// TODO: finish this\nmodule m(input a, output y);\nassign y = a;\nendmodule\n";
        assert!(only(src, "todo-comment").contains("TODO comment"));
    }

    #[test]
    fn keyword_as_identifier() {
        // `bit` is a SystemVerilog keyword; in a Verilog-2005 file it is
        // an ordinary name, and the rule says so.
        let src = "module m(input clk, output reg q);\nreg bit_;\nalways @(posedge clk) q <= bit_;\nendmodule\n";
        assert_eq!(only(src, "keyword-as-identifier"), "");
    }
}
