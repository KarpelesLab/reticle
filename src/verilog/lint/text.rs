//! Rules over the raw text: line length, trailing whitespace and tabs.
//!
//! These belong to the formatter that phase 9 will bring, but they are
//! cheap to compute here and useful before it exists, so they ship off by
//! default and are enabled by a project that wants them.

use crate::diag::Diagnostics;
use crate::source::Span;
use crate::verilog::ast::SourceFile;

use super::{Level, Lint, LintContext};

/// One line of the file: its text without the newline and the byte offset
/// it starts at.
struct Line<'a> {
    text: &'a str,
    start: usize,
}

/// The lines of the linted file.
fn lines<'a>(ctx: &LintContext<'a>) -> Vec<Line<'a>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for raw in ctx.text().split_inclusive('\n') {
        out.push(Line {
            text: raw.trim_end_matches(['\n', '\r']),
            start,
        });
        start += raw.len();
    }
    out
}

/// A span inside the linted file from byte offsets relative to its start.
fn span_of(ctx: &LintContext<'_>, start: usize, end: usize) -> Span {
    let start = u32::try_from(start).unwrap_or(u32::MAX);
    let end = u32::try_from(end).unwrap_or(u32::MAX);
    Span::new(ctx.source, start, end)
}

/// `line-length`: a line past the configured column.
pub(super) struct LineLength;

impl Lint for LineLength {
    fn id(&self) -> &'static str {
        "L0026"
    }

    fn name(&self) -> &'static str {
        "line-length"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        let max = ctx.config.max_line_length;
        if max == 0 {
            return;
        }
        for line in lines(ctx) {
            let count = line.text.chars().count();
            if count <= max {
                continue;
            }
            // The byte offset of the first character past the limit.
            let over = line
                .text
                .char_indices()
                .nth(max)
                .map_or(line.text.len(), |(i, _)| i);
            ctx.report(
                span_of(ctx, line.start + over, line.start + line.text.len()),
                format!("line is {count} characters, the limit is {max}"),
            )
            .emit(diags);
        }
    }
}

/// `trailing-whitespace`: spaces or tabs before the line break.
pub(super) struct TrailingWhitespace;

impl Lint for TrailingWhitespace {
    fn id(&self) -> &'static str {
        "L0027"
    }

    fn name(&self) -> &'static str {
        "trailing-whitespace"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for line in lines(ctx) {
            let trimmed = line.text.trim_end_matches([' ', '\t']);
            if trimmed.len() == line.text.len() {
                continue;
            }
            ctx.report(
                span_of(
                    ctx,
                    line.start + trimmed.len(),
                    line.start + line.text.len(),
                ),
                "trailing whitespace",
            )
            .emit(diags);
        }
    }
}

/// `tabs`: a tab character anywhere in a line.
pub(super) struct Tabs;

impl Lint for Tabs {
    fn id(&self) -> &'static str {
        "L0028"
    }

    fn name(&self) -> &'static str {
        "tabs"
    }

    fn default_level(&self) -> Level {
        Level::Off
    }

    fn check(&self, ctx: &LintContext<'_>, _file: &SourceFile, diags: &mut Diagnostics) {
        for line in lines(ctx) {
            let Some(first) = line.text.find('\t') else {
                continue;
            };
            let end = line.text[first..]
                .find(|c| c != '\t')
                .map_or(line.text.len(), |n| first + n);
            ctx.report(
                span_of(ctx, line.start + first, line.start + end),
                "tab character",
            )
            .note("indent with spaces; a tab is displayed differently everywhere")
            .emit(diags);
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
    fn text_rules() {
        let src = "module m;   \n\twire a;\nendmodule\n";
        let out = only(src, "trailing-whitespace");
        assert_eq!(out.matches("trailing whitespace").count(), 1, "{out}");
        assert!(only(src, "tabs").contains("tab character"));

        let mut config = LintConfig::parse("off:all warn:line-length").unwrap();
        config.max_line_length = 12;
        let out = lint("module m;\nwire this_is_long;\nendmodule\n", &config);
        assert!(
            out.contains("line is 18 characters, the limit is 12"),
            "{out}"
        );
    }

    #[test]
    fn text_rules_are_off_by_default() {
        let mut config = LintConfig::new();
        config.set_level("unused-signal", Level::Off).unwrap();
        config.set_level("undriven-signal", Level::Off).unwrap();
        assert_eq!(lint("module m;\t \n  wire a;\nendmodule\n", &config), "");
    }
}
