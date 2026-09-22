//! VHDL frontend (IEEE 1076-2008, with 1993 compatibility).
//!
//! Layout, per phase 2 of `ROADMAP.md`:
//!
//! - [`token`]: [`TokenKind`] (every reserved word is a variant), [`Token`]
//!   and the [`Standard`] switch that gates the VHDL-2008 reserved words.
//! - [`lex`]: [`lex_source`] and [`Lexer`], source text to tokens plus a
//!   side table of comments. Literals are kept as raw text; identifiers keep
//!   their original spelling and are compared case-insensitively later.
//! - `parse`: design units and statements (planned).
//! - `sema`: types, overload resolution, attributes, visibility (planned).
//! - `stdlib`: the `std` and `ieee` libraries, shipped as source (planned).
//! - `elab`: generics, port maps, generate, configurations (planned).
//! - `lower`: to [`crate::ir`], with explicit `std_logic` resolution
//!   (planned).
//!
//! ```
//! use reticle::diag::Diagnostics;
//! use reticle::source::SourceMap;
//! use reticle::vhdl::{Standard, TokenKind, lex_source};
//!
//! let mut map = SourceMap::new();
//! let id = map.add("t.vhd", "entity T is end;").unwrap();
//! let mut diags = Diagnostics::new();
//! let tokens = lex_source(&map, id, Standard::Vhdl2008, &mut diags);
//! assert!(diags.is_empty());
//! let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
//! assert_eq!(
//!     kinds,
//!     [TokenKind::Entity, TokenKind::Ident, TokenKind::Is, TokenKind::End, TokenKind::Semi, TokenKind::Eof]
//! );
//! assert_eq!(tokens[1].text(), "T");
//! ```

pub mod lex;
pub mod token;

pub use lex::{CommentKind, Lexed, Lexer, lex_source};
pub use token::{Standard, Token, TokenKind};
