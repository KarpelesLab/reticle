//! Reticle: a VHDL / Verilog compiler written from scratch in Rust.
//!
//! Reticle takes hardware designs from source text through a unified
//! intermediate representation (IR) to simulation, synthesis and physical
//! targets. Both languages lower to the same IR, so mixed-language designs
//! are handled uniformly by every later stage.
//!
//! The crate is a library first; the `reticle` binary is a thin wrapper over
//! it. The core never performs I/O: sources are handed in through a
//! [`source::SourceMap`] and results are returned as values, so the compiler
//! embeds cleanly in tests, build scripts and a WebAssembly playground.
//!
//! Every stage is behind a Cargo feature (see `Cargo.toml`), and the plan for
//! each is laid out in `ROADMAP.md` at the repository root.
//!
//! # Layout
//!
//! | Module      | Feature   | Role                                            |
//! |-------------|-----------|-------------------------------------------------|
//! | [`source`]  | always    | Source files, byte spans, line/column lookup    |
//! | [`diag`]    | always    | Diagnostics and their text rendering            |
//! | [`verilog`] | `verilog` | Verilog / SystemVerilog frontend                |
//! | [`vhdl`]    | `vhdl`    | VHDL frontend                                   |
//! | [`ir`]      | always    | The unified design IR                           |
//! | [`sim`]     | `sim`     | Event-driven simulator                          |
//! | [`synth`]   | `synth`   | Synthesis passes and technology mapping         |

#![forbid(unsafe_code)]

pub mod diag;
pub mod ir;
pub mod source;

#[cfg(feature = "verilog")]
pub mod verilog;

#[cfg(feature = "vhdl")]
pub mod vhdl;

#[cfg(feature = "sim")]
pub mod sim;

#[cfg(feature = "synth")]
pub mod synth;

/// The crate version, as recorded in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
