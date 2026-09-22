//! The VHDL source formatter.
//!
//! [`format_source`] lexes and parses a file and lays the tree out again
//! from scratch, so the result depends on the *structure* of the design and
//! not on how it happened to be typed. Everything the tree does not record
//! — whitespace, line breaks, the choice between `end;` and
//! `end entity foo;` — is decided by [`FormatOptions`]; everything it does
//! record comes back unchanged.
//!
//! Two things that a parser normally discards are recovered from the
//! source: comments, which are re-attached to the tree by span, and blank
//! lines, of which at most one is kept wherever the source had any. Names
//! and literals are copied out of the source text rather than re-spelled,
//! so `16#FF#`, `x"F_F"` and `\Extended Name\` survive exactly.
//!
//! The module is in two halves: `comments` holds the cursor over the
//! lexer's comment table, and `rules` holds one layout rule per syntactic
//! construct, building the [`crate::fmt_doc::Doc`] that the printer then
//! lays out.
//!
//! # Refusing to format
//!
//! A formatter that mangles a file is worse than no formatter, so
//! [`format_source`] returns the diagnostics instead of a result when:
//!
//! - the parser reported an **error** — the tree is then a partial
//!   recovery and would not be written back faithfully; or
//! - the parser reported that it **skipped** part of the source (PSL
//!   directives are the only case today) — that text is in no tree and
//!   would be lost.
//!
//! Warnings that leave the tree complete (a VHDL-2008 construct in
//! VHDL-93 mode, say) do not stop formatting.
//!
//! ```
//! use reticle::vhdl::Standard;
//! use reticle::vhdl::format::{FormatOptions, format_source};
//!
//! let src = "entity e is port(a:in bit;b:out bit); end;";
//! let out = format_source(src, Standard::Vhdl2008, &FormatOptions::default()).unwrap();
//! assert_eq!(out, "entity e is\n  port (a : in bit; b : out bit);\nend entity e;\n");
//! ```

mod comments;
mod rules;

pub use crate::fmt_doc::{FormatCheck, FormatOptions, Indent, KeywordCase, Newline};

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::SourceMap;
use crate::vhdl::{Lexer, Parser, Standard};

/// The name the formatter gives the text it is handed, used in diagnostics
/// and in the header of a check-mode diff.
const FILE_NAME: &str = "<format>";

/// True for a diagnostic that means the tree is not a faithful image of
/// the source, so laying it out again would lose or invent something.
fn blocks_formatting(diag: &Diagnostic) -> bool {
    diag.is_error() || diag.message.contains("skipped")
}

/// Formats one VHDL source text.
///
/// Returns the diagnostics unchanged, and no result, when the file cannot
/// be formatted safely (see the module documentation).
pub fn format_source(
    text: &str,
    standard: Standard,
    opts: &FormatOptions,
) -> Result<String, Diagnostics> {
    let mut map = SourceMap::new();
    let id = match map.add(FILE_NAME, text) {
        Ok(id) => id,
        Err(err) => {
            let mut diags = Diagnostics::new();
            diags.push(Diagnostic::error(err.to_string()));
            return Err(diags);
        }
    };
    let src = map.file(id).text();

    let mut diags = Diagnostics::new();
    let lexed = Lexer::new(src, id, standard).lex(&mut diags);
    let file = Parser::new(&lexed.tokens, standard).parse_design_file(&mut diags);
    if diags.iter().any(blocks_formatting) {
        if !diags.has_errors() {
            diags.push(Diagnostic::error(
                "not formatted: the parser skipped source it does not model",
            ));
        }
        return Err(diags);
    }

    let mut fmt = rules::Fmt::new(src, &lexed.comments, &lexed.tokens, opts);
    Ok(fmt.design_file(&file).render(opts))
}

/// Formats one VHDL source text and reports what would change.
///
/// The [`FormatCheck`] says whether the file is already formatted, carries
/// the formatted text, and holds a unified diff from one to the other for a
/// command-line tool to print.
pub fn format_check(
    text: &str,
    standard: Standard,
    opts: &FormatOptions,
) -> Result<FormatCheck, Diagnostics> {
    let formatted = format_source(text, standard, opts)?;
    Ok(FormatCheck::new(text, formatted, FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTITY: &str = "\
entity e is
  port (a : in bit; b : out bit);
end entity e;
";

    fn format(text: &str, opts: &FormatOptions) -> String {
        format_source(text, Standard::Vhdl2008, opts).expect("formats")
    }

    #[test]
    fn already_formatted_is_unchanged() {
        let check =
            format_check(ENTITY, Standard::Vhdl2008, &FormatOptions::default()).expect("formats");
        assert!(!check.changed, "{}", check.diff);
        assert!(check.diff.is_empty());
    }

    #[test]
    fn check_reports_a_diff() {
        let check = format_check(
            "entity   e is port(a:in bit;b:out bit); end;",
            Standard::Vhdl2008,
            &FormatOptions::default(),
        )
        .expect("formats");
        assert!(check.changed);
        assert!(check.diff.starts_with("--- <format>\n+++ <format>\n"));
        assert!(check.diff.contains("+entity e is"));
        assert_eq!(check.formatted, ENTITY);
    }

    #[test]
    fn tabs_indent() {
        let opts = FormatOptions {
            indent: Indent::Tabs,
            ..FormatOptions::default()
        };
        let out = format("entity e is port(a:in bit); end;", &opts);
        assert_eq!(out, "entity e is\n\tport (a : in bit);\nend entity e;\n");
    }

    #[test]
    fn upper_case_keywords() {
        let opts = FormatOptions {
            keyword_case: KeywordCase::Upper,
            ..FormatOptions::default()
        };
        let out = format("entity e is port(a:in bit); end;", &opts);
        assert_eq!(out, "ENTITY e IS\n  PORT (a : IN bit);\nEND ENTITY e;\n");
    }

    #[test]
    fn crlf_line_endings() {
        let opts = FormatOptions {
            newline: Newline::CrLf,
            ..FormatOptions::default()
        };
        let out = format("entity e is end;", &opts);
        assert_eq!(out, "entity e is\r\nend entity e;\r\n");
    }

    #[test]
    fn end_labels_can_be_left_alone() {
        let opts = FormatOptions {
            complete_end_labels: false,
            ..FormatOptions::default()
        };
        let out = format("entity e is end;", &opts);
        assert_eq!(out, "entity e is\nend entity;\n");
    }

    #[test]
    fn a_parse_error_is_returned_not_formatted() {
        let diags = format_source("entity is", Standard::Vhdl2008, &FormatOptions::default())
            .expect_err("does not format");
        assert!(diags.has_errors());
    }

    #[test]
    fn skipped_source_is_never_reformatted() {
        let src = "\
entity e is
end entity e;

architecture a of e is
begin
  assert always (a -> next b);
end architecture a;
";
        let diags = format_source(src, Standard::Vhdl2008, &FormatOptions::default())
            .expect_err("refuses to format");
        assert!(diags.iter().any(|d| d.message.contains("PSL")));
    }

    #[test]
    fn comments_and_blank_lines_survive() {
        let src = "\
-- header


entity e is
  -- a port
  port (a : in bit); -- trailing
end entity e;
";
        let out = format(src, &FormatOptions::default());
        assert_eq!(
            out,
            "\
-- header

entity e is
  -- a port
  port (a : in bit); -- trailing
end entity e;
"
        );
    }
}
