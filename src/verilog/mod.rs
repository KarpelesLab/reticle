//! Verilog / SystemVerilog frontend (IEEE 1364-2005 and the synthesisable and
//! testbench subset of IEEE 1800).
//!
//! Planned layout, per phase 1 of `ROADMAP.md`:
//!
//! - `preprocess`: compiler directives, macros, includes.
//! - `lex`: tokens with spans.
//! - `parse`: AST with error recovery.
//! - `elab`: name resolution, parameters, generate, width inference.
//! - `lower`: AST to [`crate::ir`].
//! - `lint`: AST-level checks that need no synthesis.
