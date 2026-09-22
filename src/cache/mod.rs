//! Incremental compilation over a content-addressed cache.
//!
//! A large design that is re-simulated after a one-line edit should not be
//! re-elaborated from scratch. This module stores what each module
//! elaborated to, filed under a key that describes everything that went
//! into producing it, so a later build can tell — without looking at
//! timestamps, and without trusting that nobody edited a file behind its
//! back — which modules can be taken from the store and which have to be
//! built again.
//!
//! ```no_run
//! # #[cfg(feature = "verilog")] {
//! use reticle::cache::{BuildOptions, Language, MemoryStorage, SourceUnit, build};
//! use reticle::diag::Diagnostics;
//!
//! let sources = vec![
//!     SourceUnit::new("leaf.v", "module leaf(output o); assign o = 1'b0; endmodule\n"),
//!     SourceUnit::new("top.v", "module top(output o); leaf u(o); endmodule\n"),
//! ];
//! let mut storage = MemoryStorage::new();
//! let mut diags = Diagnostics::new();
//! let options = BuildOptions::new(Language::Verilog).with_top("top");
//!
//! let first = build(&sources, &options, &mut storage, &mut diags);
//! let second = build(&sources, &options, &mut storage, &mut diags);
//! assert_eq!(second.misses, 0); // everything came from the store
//! assert_eq!(first.design.unwrap().to_text(), second.design.unwrap().to_text());
//! # }
//! ```
//!
//! # The shape
//!
//! | Piece | What it is |
//! |-------|------------|
//! | [`hash`] | A 128-bit hash written in-crate (two xxHash64 lanes) |
//! | [`key`]  | [`CacheKey`], and the rule for what goes into one |
//! | [`inputs`] | Files a unit of work read while it ran, recorded and re-checked on every hit |
//! | [`store`] | [`Cache`] over the [`Storage`] trait, with metadata, eviction and [`Cache::verify`] |
//! | [`scan`] | The per-file dependency scan that builds the module graph |
//! | [`mod@build`] | The incremental build itself |
//!
//! Nothing here performs I/O. [`MemoryStorage`] ships with the library;
//! a directory-backed [`Storage`] is about forty lines and lives in the
//! `reticle` binary (`src/bin/reticle/cache_store.rs`), which is the only
//! part of the project allowed to touch the filesystem.
//!
//! # Content addressing, not timestamps
//!
//! A key is a digest of the *inputs*: the source text of every file that
//! contributed, the options, the parameter set, the compiler version, the
//! compiled-in feature set, and the keys of the module's dependencies.
//! [`key`] enumerates them and tests each one. Nothing about the
//! filesystem enters a key, which is what makes the cache stay correct
//! when a file is edited, restored, touched or moved while the build is
//! not looking — and what makes a store shareable between machines.
//!
//! # What is cached
//!
//! - **Elaborated modules**, serialised as the IR's `.rtl` text format.
//!   That format already round-trips exactly, so there is no new
//!   serialiser to get wrong and an entry can be read with `cat`.
//! - **Synthesised modules** (with the `synth` feature), the same way,
//!   under a key that adds the synthesis options. Synthesis also reads the
//!   files `$readmemh` names, which no key computed beforehand can cover,
//!   so the entry records each one it read and a digest of its contents,
//!   and a lookup re-reads them and misses on any difference ([`inputs`]).
//! - **Dependency scans**: the handful of names a file defines and
//!   instantiates. Parsed ASTs are *not* worth caching — parsing is a
//!   small fraction of elaboration and an AST has no stable serialised
//!   form — but the two-line summary extracted from one is, because
//!   without it every build would have to parse every file just to find
//!   out what depends on what.
//!
//! See `docs/cache.md` for the measured effect, including the cases where
//! the cache does not help.

pub mod build;
pub mod hash;
pub mod inputs;
pub mod key;
pub mod scan;
pub mod store;

pub use build::{
    BuildOptions, BuildResult, ModuleBuild, Outcome, ScanBuild, Stage, VhdlStandard, build,
};
pub use hash::{Hash128, Hasher128, hash128};
pub use inputs::{FileInput, Recorder, still_valid};
pub use key::{CacheKey, KeyBuilder, feature_set};
pub use scan::{FileScan, Language, ModuleGraph, ModuleScan, SourceUnit};
pub use store::{Cache, Corruption, Entry, EntryError, MemoryStorage, Problem, Storage};
