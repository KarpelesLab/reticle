//! VHDL frontend (IEEE 1076-2008, with 1993 compatibility).
//!
//! Layout, per phase 2 of `ROADMAP.md`:
//!
//! - [`token`]: [`TokenKind`] (every reserved word is a variant), [`Token`]
//!   and the [`Standard`] switch that gates the VHDL-2008 reserved words.
//! - [`lex`]: [`lex_source`] and [`Lexer`], source text to tokens plus a
//!   side table of comments. Literals are kept as raw text; identifiers keep
//!   their original spelling and are compared case-insensitively later.
//! - [`ast`]: the syntax tree, an unresolved image of the source with a
//!   span on every node.
//! - [`parse`]: [`parse_source`] and [`Parser`], tokens to [`ast::DesignFile`]
//!   by recursive descent, with recovery at statement and declaration
//!   boundaries so one mistake yields one diagnostic.
//! - [`ast_dump`]: a deterministic text rendering of the tree for golden
//!   tests and debugging.
//! - [`format`](mod@format): the source formatter, laying the tree back
//!   out with the comments and blank lines put back in place.
//! - `sema`: types, overload resolution, attributes, visibility (planned).
//! - `stdlib`: the `std` and `ieee` libraries, shipped as source (planned).
//! - `elab`: generics, port maps, generate, configurations (planned).
//! - `lower`: to [`crate::ir`], with explicit `std_logic` resolution
//!   (planned).
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::source::SourceMap;
//! use reticle::vhdl::{Standard, ast, parse_source};
//!
//! let mut map = SourceMap::new();
//! let id = map.add("t.vhd", "entity T is port (clk : in bit); end;").unwrap();
//! let mut diags = Diagnostics::new();
//! let file = parse_source(&map, id, Standard::Vhdl2008, &mut diags);
//! assert!(diags.is_empty());
//! let ast::LibraryUnit::Entity(e) = &file.units[0].unit else { panic!() };
//! assert_eq!(e.name.name, "T");
//! assert_eq!(e.ports.len(), 1);
//! ```

pub mod ast;
pub mod ast_dump;
pub mod format;
pub mod lex;
pub mod parse;
pub mod token;

pub use lex::{CommentKind, Lexed, Lexer, lex_source};
pub use parse::Parser;
pub use token::{Standard, Token, TokenKind};

use crate::diag::Diagnostics;
use crate::source::{SourceId, SourceMap};

/// Lexes and parses one file of `map` under `standard`, reporting into
/// `diags`.
///
/// Lexer and parser diagnostics are appended in that order; sort `diags`
/// before rendering if position order is wanted.
pub fn parse_source(
    map: &SourceMap,
    id: SourceId,
    standard: Standard,
    diags: &mut Diagnostics,
) -> ast::DesignFile {
    let tokens = lex_source(map, id, standard, diags);
    Parser::new(&tokens, standard).parse_design_file(diags)
}
