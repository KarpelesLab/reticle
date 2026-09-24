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
//! | [`fmt_doc`] | always    | Document printer and diff behind the formatters |
//! | [`intern`]  | always    | Identifier interning (`Symbol`)                 |
//! | [`logic`]   | always    | Four-state bit vectors and `std_ulogic`         |
//! | [`verilog`] | `verilog` | Verilog / SystemVerilog frontend                |
//! | [`vhdl`]    | `vhdl`    | VHDL frontend                                   |
//! | [`ir`]      | always    | The unified design IR                           |
//! | [`sim`]     | `sim`     | Event-driven simulator                          |
//! | [`synth`]   | `synth`   | Synthesis passes and technology mapping         |
//! | [`formal`]  | `formal`  | SAT solver, CNF encoding, model checking        |
//! | [`asic`]    | `asic`    | Liberty, LEF and DEF readers and writers        |
//! | [`fpga`]    | `fpga`    | FPGA device database, primitive mapping, constraints |
//! | [`timing`]  | `timing`  | Static timing analysis and clock domain crossings |
//! | [`ip`]      | `ip`      | IP and project manifests, bus interfaces, interconnect |
//! | [`lsp`]     | `lsp`     | Language server for Verilog and VHDL             |
//! | [`cache`]   | `cache`   | Incremental builds over a content-addressed store |
//! | [`viewer`]  | `viewer`  | Schematic and documentation HTML pages for a design |
//! | [`program`] | `program` | Configuring a real FPGA over USB: MPSSE, JTAG, UG470 |
//! | [`ffi`]     | `ffi`     | C ABI for embedding the compiler in another tool |
//! | [`wasm`]    | `wasm`    | WebAssembly surface for the browser playground   |
//!
//! # `unsafe`
//!
//! `unsafe_code` is denied crate-wide rather than forbidden. `forbid`
//! cannot be lifted even locally, and [`ffi`] and [`wasm`] have to hand raw
//! pointers across an ABI boundary, which no safe formulation expresses.
//! Those two modules are therefore the only place in the crate that opts
//! back in, each with a file-scoped `#![allow(unsafe_code)]` and a comment
//! on every block naming the invariant it relies on. Every other module
//! stays safe; a compiler has no business touching raw memory.

#![deny(unsafe_code)]

pub mod diag;
pub mod fmt_doc;
pub mod intern;
pub mod ir;
pub mod logic;
pub mod source;

// Hand-written JSON, shared by the language server (JSON-RPC) and the
// FPGA side's Project X-Ray database reader (`tilegrid.json` and its
// neighbours). It is compiled only for the builds that have one of
// those, so a lint-only or simulation-only build does not carry it.
#[cfg(any(feature = "lsp", feature = "fpga"))]
pub mod json;

// Hand-written MessagePack, read-only. Its one caller today is the Gowin
// fabric loader (`fpga::apicula`), whose chip database is a MessagePack
// map inside an xz stream; it sits at the crate root beside `json` for
// the same reason, that a second caller is expected and writing the
// parser twice would be the only alternative. Compiled with the FPGA
// side, which is the only thing that has a use for it.
#[cfg(feature = "fpga")]
pub mod msgpack;

#[cfg(feature = "verilog")]
pub mod verilog;

#[cfg(feature = "vhdl")]
pub mod vhdl;

#[cfg(feature = "sim")]
pub mod sim;

#[cfg(feature = "synth")]
pub mod synth;

#[cfg(feature = "formal")]
pub mod formal;

#[cfg(feature = "asic")]
pub mod asic;

#[cfg(feature = "fpga")]
pub mod fpga;

#[cfg(feature = "timing")]
pub mod timing;

#[cfg(feature = "ip")]
pub mod ip;

#[cfg(feature = "lsp")]
pub mod lsp;

#[cfg(feature = "cache")]
pub mod cache;
#[cfg(feature = "viewer")]
pub mod viewer;

// Programming a board over USB. This is the one module with a
// dependency (`rawusb`, optional, see `Cargo.toml`) and the one that
// reaches outside the process at all; all but its `usb` submodule is
// pure byte-shuffling, in the same sans-I/O shape as everything else.
#[cfg(feature = "program")]
pub mod program;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(feature = "wasm")]
pub mod wasm;

/// The crate version, as recorded in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
