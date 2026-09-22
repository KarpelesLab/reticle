//! Verilog / SystemVerilog frontend (IEEE 1364-2005 and the synthesisable and
//! testbench subset of IEEE 1800).
//!
//! Layout, per phase 1 of `ROADMAP.md`:
//!
//! - [`preprocess`]: compiler directives, macros, includes, producing
//!   expanded text plus a [`SpanMap`] back to the original files.
//! - [`token`]: the token set, keywords gated by [`Dialect`].
//! - [`lex`]: tokens with spans, from raw or preprocessed text.
//! - [`ast`]: the syntax tree, purely syntactic (names are strings, numbers
//!   are their source text).
//! - [`parse`]: recursive descent with error recovery.
//! - [`ast_dump`]: a deterministic text rendering of the tree, for golden
//!   tests and debugging.
//! - `elab`: name resolution, parameters, generate, width inference (not
//!   yet written).
//! - `lower`: AST to [`crate::ir`].
//! - [`lint`]: AST-level checks that need no synthesis, run over the tree
//!   plus the lexer's comments (see `docs/lints.md`).
//!
//! [`lex_source`] runs the preprocessor and lexer; [`parse_source`] runs
//! all three stages and is the entry point elaboration will consume.

pub mod ast;
pub mod ast_dump;
pub mod lex;
pub mod lint;
pub mod parse;
pub mod preprocess;
pub mod token;

pub use lex::{CommentKind, Lexed, Lexer};
pub use parse::{ParseError, Parser};
pub use preprocess::{IncludeResolver, NoIncludes, Preprocessed, Preprocessor, SpanMap};
pub use token::{Dialect, Keyword, Punct, Token, TokenKind};

use crate::diag::Diagnostics;
use crate::source::{SourceId, SourceMap};

/// Preprocesses and lexes the file `id` of `map`.
///
/// Included files are added to `map` through `resolver`; every token's span
/// points into the original files (a token from a macro expansion reports
/// the use site). Problems go to `diags`; the token stream always ends with
/// [`TokenKind::Eof`] so a parser can proceed after errors.
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::source::SourceMap;
/// use reticle::verilog::{Dialect, Keyword, NoIncludes, TokenKind, lex_source};
///
/// let mut map = SourceMap::new();
/// let id = map.add("t.sv", "`define T logic\n`T x;").unwrap();
/// let mut diags = Diagnostics::new();
/// let tokens = lex_source(&mut map, id, Dialect::SystemVerilog, &mut NoIncludes, &mut diags);
/// assert!(diags.is_empty());
/// assert_eq!(tokens[0].kind, TokenKind::Keyword(Keyword::Logic));
/// assert_eq!(map.file(id).loc(tokens[0].span.start).line, 2);
/// ```
pub fn lex_source(
    map: &mut SourceMap,
    id: SourceId,
    dialect: Dialect,
    resolver: &mut dyn IncludeResolver,
    diags: &mut Diagnostics,
) -> Vec<Token> {
    lex_source_full(map, id, dialect, resolver, diags).tokens
}

/// Like [`lex_source`] but also returns the comments, for tooling that
/// wants them.
pub fn lex_source_full(
    map: &mut SourceMap,
    id: SourceId,
    dialect: Dialect,
    resolver: &mut dyn IncludeResolver,
    diags: &mut Diagnostics,
) -> Lexed {
    let pre = Preprocessor::new().run(map, id, resolver, diags);
    let mut lexed = Lexer::new(&pre.text, id, dialect, diags)
        .with_span_map(&pre.spans)
        .run();
    // The expansion may end early (an inactive `ifdef` reaching the end of
    // the file, or an include as the last thing), which would put the end
    // of input somewhere in the middle of the root file. Pin it to the end.
    if let Some(eof) = lexed.tokens.last_mut()
        && eof.kind == TokenKind::Eof
    {
        let len = u32::try_from(map.file(id).text().len()).expect("source length fits u32");
        eof.span = crate::source::Span::new(id, len, len);
    }
    lexed
}

/// Preprocesses, lexes and parses the file `id` of `map`.
///
/// Lexer and parser diagnostics are appended to `diags`; the tree is
/// always returned, with unparseable regions skipped, so callers can keep
/// going (a linter or language server wants the partial tree).
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::source::SourceMap;
/// use reticle::verilog::ast::ItemKind;
/// use reticle::verilog::{Dialect, NoIncludes, parse_source};
///
/// let mut map = SourceMap::new();
/// let id = map.add("t.sv", "module m(input logic a, output logic y);\n  assign y = ~a;\nendmodule").unwrap();
/// let mut diags = Diagnostics::new();
/// let file = parse_source(&mut map, id, Dialect::SystemVerilog, &mut NoIncludes, &mut diags);
/// assert!(diags.is_empty());
/// let ItemKind::Module(m) = &file.items[0].kind else { panic!() };
/// assert_eq!(m.name.name, "m");
/// ```
pub fn parse_source(
    map: &mut SourceMap,
    id: SourceId,
    dialect: Dialect,
    resolver: &mut dyn IncludeResolver,
    diags: &mut Diagnostics,
) -> ast::SourceFile {
    let tokens = lex_source(map, id, dialect, resolver, diags);
    let mut parser = Parser::new(&tokens, dialect);
    let file = parser.parse_source_file();
    diags.append(&mut parser.take_diagnostics());
    file
}
