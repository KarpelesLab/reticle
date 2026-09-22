//! A language server (LSP) for Verilog and VHDL.
//!
//! Phase 9 of `ROADMAP.md`: diagnostics as you type, go-to-definition,
//! hover with resolved types and widths, document symbols, completion,
//! rename, references and formatting, for both front ends, through one
//! server. See `docs/lsp.md` for the protocol-level view and for what each
//! feature can and cannot answer.
//!
//! # Layout
//!
//! | Module       | Role                                                      |
//! |--------------|-----------------------------------------------------------|
//! | [`json`]     | A JSON parser and serialiser, since the crate has no dependency for one |
//! | [`protocol`] | The `Content-Length` framing, JSON-RPC, the LSP types, and the UTF-16 / byte conversion |
//! | [`analysis`] | The document store: text, parse, VHDL analysis, diagnostics |
//! | [`index`]    | The language-neutral per-document index the features read  |
//! | [`verilog`]  | Building that index from a Verilog syntax tree              |
//! | [`vhdl`]     | Building it from VHDL source and its [`crate::vhdl::sema::Analysis`] |
//! | [`features`] | The features themselves, written once over the index        |
//! | [`server`]   | The sans-I/O message loop                                   |
//! | [`stdio`]    | The transport, and the only part that touches a stream      |
//! | [`text`]     | Small byte-offset helpers the rest share                    |
//!
//! # Shape
//!
//! ```text
//!   bytes ──▶ stdio ──▶ protocol ──▶ Json ──▶ Server::handle ──▶ Vec<Json>
//!                                                  │
//!                                    ┌─────────────┴─────────────┐
//!                                    ▼                           ▼
//!                               DocumentStore                features
//!                          (text, parse, analysis)       (over one Index)
//!                                    │
//!                     ┌──────────────┴──────────────┐
//!                     ▼                             ▼
//!            verilog::index (AST walk)     vhdl::index (AST + sema)
//! ```
//!
//! [`server::Server`] is a value: a message in, messages out, no I/O and
//! no clock. That is what makes a whole session a golden test
//! (`tests/lsp_session.rs`), and it is why the transport is thirty lines.
//!
//! # Two things worth knowing
//!
//! **Positions.** LSP counts UTF-16 code units from the start of a line;
//! Reticle counts UTF-8 bytes from the start of a file. Every crossing
//! goes through [`protocol::LineIndex`], which is tested against text
//! holding two-, three- and four-byte characters in both directions.
//!
//! **Precision over helpfulness.** Every feature would rather answer
//! nothing than guess. A width is reported only when the source settles it
//! — never by assuming a parameter's default — and a rename is refused,
//! with a message saying why, as soon as a use of the name could be in a
//! file the server has not been shown.
//!
//! ```
//! use reticle::lsp::json::Json;
//! use reticle::lsp::protocol::{notification, request};
//! use reticle::lsp::server::Server;
//!
//! let mut server = Server::new();
//! server.handle(request(Json::Int(1), "initialize", Json::object()));
//! server.handle(notification("initialized", Json::object()));
//!
//! let out = server.handle(notification(
//!     "textDocument/didOpen",
//!     Json::obj([(
//!         "textDocument",
//!         Json::obj([
//!             ("uri", Json::str("file:///counter.v")),
//!             ("languageId", Json::str("verilog")),
//!             ("version", Json::Int(1)),
//!             ("text", Json::str("module m;\n  wire [7:0] q;\nendmodule\n")),
//!         ]),
//!     )]),
//! ));
//! assert_eq!(
//!     out[0].get("method").and_then(Json::as_str),
//!     Some("textDocument/publishDiagnostics")
//! );
//! ```

pub mod analysis;
pub mod features;
pub mod index;
pub mod json;
pub mod protocol;
pub mod server;
pub mod stdio;
pub mod text;
pub mod verilog;
pub mod vhdl;

pub use analysis::{Document, DocumentStore, Language};
pub use json::Json;
pub use server::Server;
