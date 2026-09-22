//! VHDL frontend (IEEE 1076-2008, with 1993 compatibility).
//!
//! Planned layout, per phase 2 of `ROADMAP.md`:
//!
//! - `lex`: case-insensitive tokens, literals, spans.
//! - `parse`: design units and statements.
//! - `sema`: types, overload resolution, attributes, visibility.
//! - `stdlib`: the `std` and `ieee` libraries, shipped as source.
//! - `elab`: generics, port maps, generate, configurations.
//! - `lower`: to [`crate::ir`], with explicit `std_logic` resolution.
