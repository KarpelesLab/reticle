//! The stage calls both embedding boundaries make, in one place.
//!
//! [`crate::ffi`] and [`crate::wasm`] expose the same operations in
//! different shapes — opaque handles for C, length-prefixed buffers for
//! JavaScript — but the compiler calls underneath are identical: add each
//! source to a map, parse it, elaborate the lot, optionally synthesise.
//! Getting that sequence subtly different in two places is how the two
//! surfaces would drift, so it lives here and each boundary wraps it.
//!
//! The two features are independent (`wasm` does not imply `ffi`), so
//! there is no shared parent module to hang this off; `src/wasm/mod.rs`
//! pulls this same file in with `#[path]`. Everything here therefore names
//! types by their `crate::` path and never reaches for `super::`.
//!
//! Every function is a no-op returning `None` when the stage it drives was
//! not compiled in, which is what lets both boundaries export one fixed
//! set of entry points whatever the feature set is.

// Each boundary uses a subset of this file, so an item only one of them
// needs is not dead code: it is simply unused in the other's copy.
#![allow(dead_code)]

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::Design;
use crate::source::SourceMap;

/// Which frontend to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    /// Verilog-2005 and the synthesisable SystemVerilog subset.
    Verilog,
    /// VHDL-2008.
    Vhdl,
}

impl Language {
    /// The language named by `name`, case-insensitively: `verilog`, `v`,
    /// `sv` and `systemverilog`, or `vhdl` and `vhd`.
    pub(crate) fn from_name(name: &str) -> Option<Language> {
        match name.to_ascii_lowercase().as_str() {
            "verilog" | "v" | "sv" | "svh" | "vh" | "systemverilog" => Some(Language::Verilog),
            "vhdl" | "vhd" => Some(Language::Vhdl),
            _ => None,
        }
    }

    /// The Cargo feature this frontend needs.
    pub(crate) fn feature(self) -> &'static str {
        match self {
            Language::Verilog => "verilog",
            Language::Vhdl => "vhdl",
        }
    }

    /// Whether this build has the frontend compiled in.
    pub(crate) fn enabled(self) -> bool {
        match self {
            Language::Verilog => cfg!(feature = "verilog"),
            Language::Vhdl => cfg!(feature = "vhdl"),
        }
    }
}

/// Adds one source to `map`, reporting a file too large for 32-bit spans
/// as a diagnostic rather than a panic.
fn add_source(
    map: &mut SourceMap,
    name: &str,
    text: &str,
    diags: &mut Diagnostics,
) -> Option<crate::source::SourceId> {
    match map.add(name, text) {
        Ok(id) => Some(id),
        Err(error) => {
            diags.push(Diagnostic::error(format!("`{name}`: {error}")));
            None
        }
    }
}

/// Parses and elaborates `sources` in `language`.
///
/// `sources` is a list of *(display name, text)* pairs forming one
/// compilation; `top` names the root module or entity, or is `None` to let
/// elaboration pick the unit nothing instantiates. Returns `None` when the
/// frontend reported an error or is not compiled in; `diags` always
/// explains why.
pub(crate) fn elaborate(
    language: Language,
    sources: &[(&str, &str)],
    top: Option<&str>,
    map: &mut SourceMap,
    diags: &mut Diagnostics,
) -> Option<Design> {
    if sources.is_empty() {
        diags.push(Diagnostic::error("no source was given"));
        return None;
    }
    if !language.enabled() {
        diags.push(Diagnostic::error(format!(
            "this build has no `{}` frontend",
            language.feature()
        )));
        return None;
    }
    match language {
        Language::Verilog => elaborate_verilog(sources, top, map, diags),
        Language::Vhdl => elaborate_vhdl(sources, top, map, diags),
    }
}

/// The Verilog half of [`elaborate`].
#[cfg(feature = "verilog")]
fn elaborate_verilog(
    sources: &[(&str, &str)],
    top: Option<&str>,
    map: &mut SourceMap,
    diags: &mut Diagnostics,
) -> Option<Design> {
    use crate::verilog::{Dialect, ElabOptions, NoIncludes, ast, parse_source};

    // The dialect follows the first name's extension, as the CLI does:
    // `.sv` turns on the SystemVerilog rules for the whole compilation.
    let dialect = extension(sources[0].0)
        .as_deref()
        .and_then(Dialect::for_extension)
        .unwrap_or_default();

    let mut files: Vec<ast::SourceFile> = Vec::with_capacity(sources.len());
    for &(name, text) in sources {
        let id = add_source(map, name, text, diags)?;
        // The library never opens a file, so `` `include `` is reported
        // rather than resolved: an embedder that wants includes passes the
        // included files in `sources` itself.
        files.push(parse_source(map, id, dialect, &mut NoIncludes, diags));
    }
    if diags.has_errors() {
        return None;
    }

    let mut options = ElabOptions::new(dialect);
    options.top = top.map(str::to_owned);
    let refs: Vec<&ast::SourceFile> = files.iter().collect();
    crate::verilog::elaborate(&refs, &options, diags)
}

/// Stand-in for a build without the `verilog` feature; [`elaborate`] has
/// already reported that and never calls this.
#[cfg(not(feature = "verilog"))]
fn elaborate_verilog(
    _sources: &[(&str, &str)],
    _top: Option<&str>,
    _map: &mut SourceMap,
    _diags: &mut Diagnostics,
) -> Option<Design> {
    None
}

/// The VHDL half of [`elaborate`].
#[cfg(feature = "vhdl")]
fn elaborate_vhdl(
    sources: &[(&str, &str)],
    top: Option<&str>,
    map: &mut SourceMap,
    diags: &mut Diagnostics,
) -> Option<Design> {
    use crate::vhdl::ElabOptions;
    use crate::vhdl::sema::Design as VhdlDesign;

    // The bundled std and ieee libraries are analysed first; every source
    // is then compiled into `work`, which is what a plain `.vhd` expects.
    let mut library = VhdlDesign::with_stdlib(map, crate::vhdl::Standard::default(), diags);
    if diags.has_errors() {
        return None;
    }
    for &(name, text) in sources {
        let id = add_source(map, name, text, diags)?;
        library.add_source(map, id, "work", diags);
    }
    if diags.has_errors() {
        return None;
    }

    let analysis = library.analyze(map, diags);
    if diags.has_errors() {
        return None;
    }

    let mut options = ElabOptions::new();
    options.top = top.map(str::to_owned);
    crate::vhdl::elaborate(&analysis, &options, diags)
}

/// Stand-in for a build without the `vhdl` feature; [`elaborate`] has
/// already reported that and never calls this.
#[cfg(not(feature = "vhdl"))]
fn elaborate_vhdl(
    _sources: &[(&str, &str)],
    _top: Option<&str>,
    _map: &mut SourceMap,
    _diags: &mut Diagnostics,
) -> Option<Design> {
    None
}

/// The lowercase extension of a path-like name, without the dot.
#[cfg(feature = "verilog")]
fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.contains('/') || ext.contains('\\') {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// True when this build can run [`synth`].
pub(crate) fn synth_enabled() -> bool {
    cfg!(feature = "synth")
}

/// Synthesises `design` in place and returns the rendered report.
///
/// `lut_inputs` is 0 for a generic netlist, or 2 to 8 to map the logic
/// that is left onto `k`-input LUTs. Returns `None` only when the `synth`
/// feature is off.
#[cfg(feature = "synth")]
pub(crate) fn synth(
    design: &mut Design,
    map: &SourceMap,
    lut_inputs: u32,
    diags: &mut Diagnostics,
) -> Option<String> {
    use std::fmt::Write as _;

    use crate::synth::techmap::{MapOptions, Target, map_module};
    use crate::synth::{SynthOptions, run};

    let stats = run(design, &SynthOptions::default(), diags);
    let mut report = stats.render(Some(map));
    if diags.has_errors() || lut_inputs == 0 {
        return Some(report);
    }

    // Technology mapping is deliberately a second step: generic synthesis
    // runs first so inference still sees arithmetic and memories, and only
    // the logic left over is covered.
    let options = MapOptions {
        target: Target::Lut(lut_inputs),
        ..MapOptions::default()
    };
    let ids: Vec<_> = design.modules.iter().map(|(id, _)| id).collect();
    for id in ids {
        let stats = map_module(&mut design.modules[id], &options);
        let _ = writeln!(
            report,
            "map {}: {} cells, depth {}",
            design.modules[id].name, stats.cells, stats.depth
        );
    }
    Some(report)
}

/// Stand-in for a build without the `synth` feature.
#[cfg(not(feature = "synth"))]
pub(crate) fn synth(
    _design: &mut Design,
    _map: &SourceMap,
    _lut_inputs: u32,
    _diags: &mut Diagnostics,
) -> Option<String> {
    None
}
