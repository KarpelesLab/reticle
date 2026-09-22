//! The Verilog / SystemVerilog source formatter.
//!
//! [`format_source`] parses the text with the ordinary frontend and prints
//! the tree back out through the shared document printer
//! ([`crate::fmt_doc`]), so the output depends on the *structure* of the
//! design and not at all on how the input happened to be laid out. It is
//! built from three parts:
//!
//! - `rules`: one layout rule per construct, producing a
//!   [`Doc`](crate::fmt_doc::Doc);
//! - `comments`: comments and compiler directives, re-attached to the
//!   items they precede or follow so none is ever lost;
//! - [`crate::fmt_doc`]: the width-driven layout algorithm and the diff
//!   behind [`format_check`].
//!
//! # Safety net
//!
//! A formatter that mangles a file is worse than no formatter, so
//! [`format_source`] refuses to touch a file it does not fully understand:
//! if the lexer or parser reports an error, the diagnostics come back and
//! the caller keeps the original text. Two properties are tested over the
//! whole parser corpus: formatting twice changes nothing after the first
//! pass, and the tree (and the set of comments) of the formatted text is
//! the tree of the original.
//!
//! # Macros
//!
//! The preprocessor is deliberately **not** run: expanding a macro would
//! rewrite the user's source into text they never wrote. Directive lines
//! are instead carried across verbatim, as described in `comments`. The
//! practical consequence is that macro-heavy files format conservatively —
//! the directive lines keep their own layout, and a directive that splits a
//! construct in half leaves the file unparseable and therefore unformatted.
//!
//! ```
//! use reticle::verilog::Dialect;
//! use reticle::verilog::format::{FormatOptions, format_source};
//!
//! let src = "module m(input a,output y);assign y=~a;endmodule";
//! let out = format_source(src, Dialect::Verilog2005, &FormatOptions::default()).unwrap();
//! assert_eq!(
//!     out,
//!     "module m (input a, output y);\n  assign y = ~a;\nendmodule\n"
//! );
//! ```

mod comments;
mod rules;

pub use crate::fmt_doc::{FormatCheck, FormatOptions, Indent, KeywordCase, Newline};

use crate::diag::{Diagnostic, Diagnostics};
use crate::source::SourceMap;
use crate::verilog::{Dialect, Lexer, Parser};

use comments::Comments;
use rules::Rules;

/// The name the formatter gives the text it was handed, for diagnostics
/// and diff headers.
const NAME: &str = "<input>";

/// Formats one Verilog or SystemVerilog source text.
///
/// Returns the formatted text, or the lexer and parser diagnostics when the
/// text does not parse; in that case nothing is formatted, so a caller can
/// always fall back to the original.
///
/// The text is not preprocessed: see the [module documentation](self) for
/// what that means for macro-heavy files.
pub fn format_source(
    text: &str,
    dialect: Dialect,
    opts: &FormatOptions,
) -> Result<String, Diagnostics> {
    let regions = comments::directive_regions(text);
    let blanked = comments::blank_directives(text, &regions);

    let mut map = SourceMap::new();
    let id = match map.add(NAME, blanked) {
        Ok(id) => id,
        Err(err) => {
            let mut diags = Diagnostics::new();
            diags.push(Diagnostic::error(err.to_string()));
            return Err(diags);
        }
    };

    let mut diags = Diagnostics::new();
    let lexed = Lexer::new(map.file(id).text(), id, dialect, &mut diags).run();
    let mut parser = Parser::new(&lexed.tokens, dialect);
    let file = parser.parse_source_file();
    diags.append(&mut parser.take_diagnostics());
    if diags.has_errors() {
        diags.sort();
        return Err(diags);
    }

    // The comment table is built over the *original* text: blanking only
    // replaced directive lines, so every other span still spells the same
    // bytes, and the directives are needed verbatim.
    let comments = Comments::new(text, &lexed.comments, &regions);
    let doc = Rules::new(opts, dialect, comments).source_file(&file);
    Ok(doc.render(opts))
}

/// Formats `text` and reports whether that would change it.
///
/// The result carries the formatted text and a unified diff from the
/// original, so a command-line `--check` mode can print exactly what it
/// would do. A change that is only in the line endings shows as `changed`
/// with an empty diff, since the diff compares lines by content.
pub fn format_check(
    text: &str,
    dialect: Dialect,
    opts: &FormatOptions,
) -> Result<FormatCheck, Diagnostics> {
    let formatted = format_source(text, dialect, opts)?;
    Ok(FormatCheck::new(text, formatted, NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format_source(src, Dialect::SystemVerilog, &FormatOptions::default()).expect("formats")
    }

    #[test]
    fn normalises_a_module() {
        let out = fmt("module   m ;\n\n\n wire  a ;\nendmodule");
        assert_eq!(out, "module m;\n  wire a;\nendmodule\n");
    }

    #[test]
    fn is_idempotent() {
        let src = "module m(input clk,output reg q);always @(posedge clk)q<=~q;endmodule";
        let once = fmt(src);
        assert_eq!(fmt(&once), once);
    }

    #[test]
    fn keeps_one_blank_line() {
        let out = fmt("module m;\n  wire a;\n\n\n\n  wire b;\nendmodule");
        assert_eq!(out, "module m;\n  wire a;\n\n  wire b;\nendmodule\n");
    }

    #[test]
    fn keeps_comments_and_directives() {
        let out = fmt("`define W 8\n// lead\nmodule m; // trail\nendmodule\n");
        assert_eq!(out, "`define W 8\n// lead\nmodule m; // trail\nendmodule\n");
    }

    #[test]
    fn refuses_to_format_a_broken_file() {
        let err = format_source("module m(", Dialect::Verilog2005, &FormatOptions::default())
            .expect_err("does not parse");
        assert!(err.has_errors());
    }

    #[test]
    fn check_reports_a_diff() {
        let opts = FormatOptions::default();
        let clean = format_source("module m;\nendmodule\n", Dialect::Verilog2005, &opts).unwrap();
        let unchanged = format_check(&clean, Dialect::Verilog2005, &opts).unwrap();
        assert!(!unchanged.changed);
        assert!(unchanged.diff.is_empty());

        let dirty = format_check("module    m;\nendmodule\n", Dialect::Verilog2005, &opts).unwrap();
        assert!(dirty.changed);
        assert!(dirty.diff.contains("-module    m;"), "{}", dirty.diff);
        assert!(dirty.diff.contains("+module m;"), "{}", dirty.diff);
    }

    #[test]
    fn honours_the_indent_and_newline_options() {
        let opts = FormatOptions {
            indent: Indent::Tabs,
            newline: Newline::CrLf,
            ..FormatOptions::default()
        };
        let out = format_source("module m;wire a;endmodule", Dialect::Verilog2005, &opts).unwrap();
        assert_eq!(out, "module m;\r\n\twire a;\r\nendmodule\r\n");
    }

    #[test]
    fn breaks_a_long_port_list_one_per_line() {
        let opts = FormatOptions {
            line_width: 40,
            ..FormatOptions::default()
        };
        let out = format_source(
            "module m(input wire clk, input wire rst, output reg [7:0] q);endmodule",
            Dialect::Verilog2005,
            &opts,
        )
        .unwrap();
        assert_eq!(
            out,
            "module m (\n  input  wire       clk,\n  input  wire       rst,\n  output reg  [7:0] q\n);\nendmodule\n"
        );
    }
}
