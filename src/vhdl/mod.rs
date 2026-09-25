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
//! - [`sema`]: the type model, overload resolution, attribute evaluation,
//!   visibility and static evaluation. [`sema::Design`] collects parsed
//!   files into libraries and [`sema::Design::analyze`] returns an
//!   [`sema::Analysis`]: the AST annotated through side tables rather than
//!   rewritten, so the elaboration and lowering passes walk the same tree.
//! - [`stdlib`]: the `std` and `ieee` libraries, shipped as VHDL source and
//!   analysed like user code. `std.standard`, `std.textio`, `std.env`,
//!   `ieee.std_logic_1164`, `ieee.numeric_std`, `ieee.numeric_bit`,
//!   `ieee.math_real`, `ieee.std_logic_textio` and the Synopsys packages are
//!   bundled; `ieee.fixed_pkg`, `ieee.float_pkg` and
//!   `ieee.numeric_std_unsigned` are not yet, and naming one yields one
//!   clear diagnostic.
//! - [`elab`]: elaboration and lowering to [`crate::ir`] — top selection,
//!   architecture binding, generics, generate unrolling, port maps and
//!   configurations, with explicit `std_logic` resolution. [`elaborate`]
//!   turns an [`Analysis`] into a validated [`crate::ir::Design`].
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
pub mod elab;
pub mod format;
pub mod lex;
pub mod parse;
pub mod sema;
pub mod stdlib;
pub mod token;

pub use elab::{ElabOptions, elaborate};
pub use lex::{CommentKind, Lexed, Lexer, lex_source};
pub use parse::Parser;
pub use sema::{Analysis, DeclId, Design, TypeId};
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

/// Parses and analyses one file against the bundled `std` and `ieee`
/// libraries, compiling it into the `work` library.
///
/// This is the one-file convenience entry point; a design of several files
/// (or one that needs more than one library) builds a [`sema::Design`],
/// adds each source to it and calls [`sema::Design::analyze`], so that the
/// units are ordered by their dependencies rather than by file order.
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::source::SourceMap;
/// use reticle::vhdl::{Standard, analyze_source};
///
/// let mut map = SourceMap::new();
/// let src = "entity t is port (clk : in bit); end entity;\n\
///            architecture a of t is signal s : bit; begin s <= clk; end;";
/// let id = map.add("t.vhd", src).unwrap();
/// let mut diags = Diagnostics::new();
/// let analysis = analyze_source(&mut map, id, Standard::Vhdl2008, &mut diags);
/// assert!(!diags.has_errors(), "{}", diags.render(&map));
/// assert!(analysis.unit("work", "t").is_some());
/// ```
pub fn analyze_source(
    map: &mut SourceMap,
    id: SourceId,
    standard: Standard,
    diags: &mut Diagnostics,
) -> Analysis {
    let mut design = Design::with_stdlib(map, standard, diags);
    design.add_source(map, id, "work", diags);
    design.analyze(map, diags)
}
