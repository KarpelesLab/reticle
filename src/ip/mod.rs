//! IP integration: manifests, dependency resolution, bus interfaces,
//! interconnect generation and black boxes.
//!
//! This is phase 8 of `ROADMAP.md`, and the reason the project exists
//! beyond "another synthesiser": third-party and first-party IP should
//! drop into a design the way a crate drops into a Rust program. That
//! needs a way to describe a package, a way to find one, a way to
//! connect one, a way to build one, and a way to use one nobody may
//! read; this module is those five things.
//!
//! | File | Role |
//! |------|------|
//! | [`manifest`] | the `reticle.ip` and `reticle.proj` formats |
//! | [`mod@resolve`] | the dependency graph, version selection, `reticle.lock` |
//! | [`bus`] | bus interfaces as data; port-map generation and checking |
//! | [`interconnect`] | crossbar and arbiter generators over the IR builder |
//! | [`blackbox`] | encrypted and vendor IP as stubs |
//!
//! # The format
//!
//! Everything here is one line-oriented text format: a keyword, then
//! whitespace-separated words, `#` or `//` starting a comment, quotes
//! around a word with spaces in it. It is the format of the `.rcf`
//! constraints and `.dev` device databases in [`crate::fpga`] and of the
//! IR's own `.rtl`.
//!
//! It is **not** TOML, although `ROADMAP.md` says `reticle.toml`. Reticle
//! ships no foreign code and will not grow a TOML or JSON parser to read
//! four kinds of file; a hand-written line grammar is the house style,
//! and line-oriented text diffs, reviews and merges better than a nested
//! document does. Naming the file `reticle.toml` would promise a format
//! it does not implement, so the project manifest is **`reticle.proj`**,
//! next to `reticle.ip` and `reticle.lock`, all read by one tokenizer
//! and reported on with one set of `P00nn` diagnostics.
//!
//! ```text
//! # reticle.proj
//! name        blinky
//! top         top
//! device      ice40-hx1k-tq144
//!
//! source      rtl/top.v
//! constraints board/ice40.rcf
//!
//! depends     uart_lite ^1.2.0 path ../ip/uart_lite
//! depends     fifo_sync >=1.0.0 path ../ip/fifo_sync
//! ```
//!
//! `docs/ip.md` has a complete worked example, with the two IP packages
//! this one depends on and everything the build produces.
//!
//! # Resolution rules
//!
//! 1. The graph is walked depth first from the project's `depends`
//!    lines.
//! 2. An IP manifest says *what* it needs (`depends fifo_sync ^1.0.0`)
//!    and never where that lives. The project places it, or the
//!    [`SourceProvider`] does; that is what keeps a package portable.
//! 3. Of everything fetched for a package, the **highest version that
//!    satisfies every requirement** on it is selected.
//! 4. A cycle, a conflict and a package that cannot be fetched are each
//!    a diagnostic naming the path through the graph, not a panic.
//! 5. The result is a [`LockFile`], so the next build resolves the same
//!    way; [`LockFile::differences`] says in words when it would not.
//!
//! The library performs no I/O. A [`SourceProvider`] hands over manifest
//! and source text, and the [`PathProvider`] shipped here reads through
//! a caller-supplied closure, so the CLI owns the filesystem and a
//! WebAssembly build owns a bundle instead.
//!
//! # The bus model
//!
//! A bus is described once, as data, in a `.bus` file: its parameters,
//! and for each signal its name, the direction **the manager** sees, its
//! width (a number, or a parameter possibly divided, which is how
//! `wstrb` is `DATA_WIDTH/8`) and whether it is required. A
//! [`BusRole`] then reinterprets those directions for whichever side a
//! module is on. AXI4, AXI4-Lite, AXI4-Stream, Wishbone (classic and
//! pipelined), APB and Avalon-MM are built in; another bus is another
//! file and no Rust.
//!
//! From that one description Reticle both **checks** and **generates**.
//! [`bus::match_ports`] recognises the bus on a module's ports by naming
//! convention and reports missing signals, reversed directions and
//! widths that contradict the parameters; [`bus::connect`] pairs two
//! instances up and returns the nets that join them; [`Crossbar`] and
//! [`WishboneArbiter`] build whole interconnects whose ports come from
//! the same definitions, so a generated crossbar passes the checker by
//! construction.
//!
//! # A whole build
//!
//! ```
//! use std::collections::BTreeMap;
//! use reticle::diag::Diagnostics;
//! use reticle::ip;
//! use reticle::source::SourceMap;
//!
//! let files: BTreeMap<&str, &str> = BTreeMap::from([
//!     ("ip/fifo/reticle.ip", "name fifo\nversion 1.0.0\n\ntop fifo\n\nsource fifo.v\n"),
//!     ("ip/fifo/fifo.v", "module fifo(input clk, output q); assign q = clk; endmodule\n"),
//!     ("rtl/top.v", "module top(input clk, output q);\n  fifo u(.clk(clk), .q(q));\nendmodule\n"),
//! ]);
//! let project_text = "\
//! name blinky
//! top top
//!
//! source rtl/top.v
//!
//! depends fifo ^1.0.0 path ip/fifo
//! ";
//!
//! let mut map = SourceMap::new();
//! let mut diags = Diagnostics::new();
//! let project = ip::load_project(&mut map, "reticle.proj", project_text, &mut diags).unwrap();
//!
//! let mut provider =
//!     ip::PathProvider::new(".", |path: &str| files.get(path).map(|s| (*s).to_owned()));
//! let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
//! assert!(resolved.is_complete());
//! assert_eq!(resolved.lock.package("fifo").unwrap().version.to_string(), "1.0.0");
//!
//! let design = ip::elaborate_project(&project, &mut resolved, &mut diags).unwrap();
//! assert!(!diags.has_errors(), "{}", diags.render(resolved.source_map()));
//! assert_eq!(design.top_module().unwrap().name, "top");
//! assert_eq!(design.modules.len(), 2);
//! ```

pub mod blackbox;
pub mod bus;
pub mod interconnect;
pub mod manifest;
pub mod resolve;
mod text;

pub use blackbox::{BlackBoxReport, BlackBoxSource};
pub use bus::{
    BusEndpoint, BusInterface, BusProblem, BusRole, BusSignal, Connection, PortMapping, Width,
};
pub use interconnect::{AddressRange, Crossbar, WishboneArbiter};
pub use manifest::{
    DepSource, Dependency, InterfaceDecl, IpManifest, Language, ParamDecl, ParamType, PortDecl,
    Project, SourceEntry, Version, VersionReq,
};
pub use resolve::{
    LoadedSource, LockFile, LockedPackage, Package, PathProvider, ResolveError, Resolved,
    ResolvedIp, Resolver, SourceProvider,
};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::Design;
use crate::source::{SourceId, SourceMap};

/// Diagnostic code for a project whose top module is nowhere to be found.
pub const NO_SUCH_TOP: &str = "P0401";
/// Diagnostic code for a source whose language nothing recognises.
pub const UNKNOWN_LANGUAGE: &str = "P0402";
/// Diagnostic code once used for a VHDL source that could not be lowered.
///
/// VHDL now reaches the IR like Verilog does, so nothing emits this. It is
/// kept so a stored report referring to it still resolves.
#[deprecated(note = "VHDL is lowered like Verilog; nothing emits this code")]
pub const VHDL_NOT_LOWERED: &str = "P0403";
/// Diagnostic code for an elaborated design that fails IR validation.
pub const INVALID_DESIGN: &str = "P0404";

/// Adds `text` to `map` under `name` and parses it as a project manifest.
///
/// The file is kept in `map` so that every diagnostic about it, and
/// about everything resolution pulls in afterwards, renders against one
/// source map.
pub fn load_project(
    map: &mut SourceMap,
    name: impl Into<String>,
    text: &str,
    diags: &mut Diagnostics,
) -> Option<Project> {
    let file: SourceId = match map.add(name, text) {
        Ok(file) => file,
        Err(error) => {
            diags.push(Diagnostic::error(format!(
                "cannot read the project manifest: {error}"
            )));
            return None;
        }
    };
    Project::parse(text, file, diags)
}

/// Resolves `project`'s dependencies through `provider`.
///
/// `map` must be the map `project` was parsed from; it is moved into the
/// [`Resolved`], which ends up holding every manifest and source the
/// build read. See [`Resolver`] for the rules.
pub fn resolve(
    map: SourceMap,
    project: &Project,
    provider: &mut dyn SourceProvider,
    diags: &mut Diagnostics,
) -> Resolved {
    Resolver::new(map).resolve(project, provider, diags)
}

/// What one project elaboration produced.
#[derive(Debug)]
pub struct Elaboration {
    /// The design, when one could be built.
    pub design: Option<Design>,
    /// One report per package that became a black box or ran from a
    /// behavioural model, in dependency order.
    pub blackboxes: Vec<BlackBoxReport>,
    /// Every source that was elaborated, as `(owner, path, language)`,
    /// in the order they were given to the frontends.
    pub sources: Vec<(String, String, Language)>,
    /// Sources that contributed nothing, with why, in the order found.
    pub skipped: Vec<(String, String)>,
}

impl Elaboration {
    /// A deterministic, multi-line summary of the build.
    ///
    /// This is what a golden test compares and what `reticle build`
    /// prints: which sources were used, which packages were black boxed
    /// and, crucially, which of them ran from a model instead of the
    /// real thing.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("sources\n");
        for (owner, path, language) in &self.sources {
            out.push_str(&format!("  {owner}: {path} ({language})\n"));
        }
        if !self.skipped.is_empty() {
            out.push_str("skipped\n");
            for (what, why) in &self.skipped {
                out.push_str(&format!("  {what}: {why}\n"));
            }
        }
        if !self.blackboxes.is_empty() {
            out.push_str("black boxes\n");
            for report in &self.blackboxes {
                for line in report.describe().lines() {
                    out.push_str(&format!("  {line}\n"));
                }
            }
        }
        match &self.design {
            Some(design) => {
                let mut names: Vec<&str> = design
                    .modules
                    .iter()
                    .map(|(_, m)| m.name.as_str())
                    .collect();
                names.sort_unstable();
                out.push_str(&format!(
                    "design\n  top: {}\n  modules: {}\n",
                    design.top_module().map_or("<none>", |m| m.name.as_str()),
                    names.join(", ")
                ));
            }
            None => out.push_str("design\n  not built\n"),
        }
        out
    }
}

/// Elaborates a project and everything it depends on into one design.
///
/// Sources are taken in dependency order — the deepest package first,
/// the project's own last — and dispatched to a frontend by extension,
/// or by the `language` word on the `source` line when the extension
/// lies. A package with no readable source becomes a black box (see
/// [`blackbox`]); one with a behavioural model is elaborated from that
/// model instead, and [`Elaboration::report`] says which.
///
/// `resolved` is taken by `&mut` because the Verilog preprocessor adds
/// the files it includes to the source map, and the map lives inside the
/// resolution so that one set of spans covers the whole build.
///
/// # What is not here yet
///
/// Sources of each language are elaborated together, so an entity or
/// module defined in one file and instantiated from another resolves. A
/// mixed-language project merges both halves into one design, keeping the
/// first definition of a repeated name. `` `include `` is not followed: a
/// package lists its files in its manifest, which is what the provider
/// reads.
pub fn elaborate(
    project: &Project,
    resolved: &mut Resolved,
    diags: &mut Diagnostics,
) -> Elaboration {
    let mut out = Elaboration {
        design: None,
        blackboxes: Vec::new(),
        sources: Vec::new(),
        skipped: Vec::new(),
    };

    // 1. Decide, per package, what represents it.
    let mut chosen: Vec<(String, Vec<LoadedSource>)> = Vec::new();
    let mut stubs: Vec<crate::ir::Module> = Vec::new();
    for package in &resolved.packages {
        let name = package.name().to_owned();
        let source = blackbox::source_for(package);
        match &source {
            BlackBoxSource::Sources => {
                let mut files = Vec::new();
                for file in &package.sources {
                    if file.is_readable() {
                        files.push(file.clone());
                    } else {
                        out.skipped.push((
                            format!("{name}: {}", file.path),
                            if file.encrypted {
                                "encrypted".to_owned()
                            } else {
                                "not available".to_owned()
                            },
                        ));
                    }
                }
                chosen.push((name, files));
            }
            BlackBoxSource::Model(_) => {
                let model = package.model.clone().expect("a model was chosen");
                chosen.push((name, vec![model]));
                let (_, report) = blackbox::stub(
                    &package.manifest,
                    source,
                    package.manifest.span,
                    &mut Diagnostics::new(),
                );
                out.blackboxes.push(report);
            }
            BlackBoxSource::Encrypted | BlackBoxSource::Missing => {
                let (module, report) =
                    blackbox::stub(&package.manifest, source, package.manifest.span, diags);
                out.blackboxes.push(report);
                chosen.push((name, Vec::new()));
                // Stubs are held aside and added once a design exists.
                stubs.push(module);
            }
        }
    }
    chosen.push((project.name.clone(), resolved.root.clone()));

    // 2. Sort the readable sources by frontend.
    let mut verilog = Vec::new();
    let mut vhdl = Vec::new();
    let mut rtl = Vec::new();
    for (owner, files) in &chosen {
        for file in files {
            let Some(id) = file.file else { continue };
            let Some(language) = file.language else {
                diags.push(
                    Diagnostic::warning(format!(
                        "`{}` has no recognised language and was skipped",
                        file.path
                    ))
                    .with_code(UNKNOWN_LANGUAGE)
                    .with_note("expected .v, .sv, .vhd, .vhdl or .rtl, or a `language` word"),
                );
                out.skipped.push((
                    format!("{owner}: {}", file.path),
                    "unknown language".to_owned(),
                ));
                continue;
            };
            out.sources
                .push((owner.clone(), file.path.clone(), language));
            match language {
                Language::Verilog | Language::SystemVerilog => verilog.push((id, language)),
                Language::Vhdl => vhdl.push(id),
                Language::Rtl => rtl.push(id),
            }
        }
    }

    // 3. Verilog, elaborated together so instances resolve across files.
    let mut design = if verilog.is_empty() {
        Design::new()
    } else {
        let dialect = if verilog.iter().any(|(_, l)| *l == Language::SystemVerilog) {
            crate::verilog::Dialect::SystemVerilog
        } else {
            crate::verilog::Dialect::Verilog2005
        };
        // The frontend takes the diagnostics sink over and declines to
        // produce a design when it already holds an error, so give it a
        // fresh one: a dependency that failed to resolve must not stop
        // the sources that did from being elaborated and reported on.
        let mut front = Diagnostics::new();
        let mut files = Vec::with_capacity(verilog.len());
        for (id, language) in &verilog {
            let per_file = match language {
                Language::SystemVerilog => crate::verilog::Dialect::SystemVerilog,
                _ => crate::verilog::Dialect::Verilog2005,
            };
            files.push(crate::verilog::parse_source(
                resolved.source_map_mut(),
                *id,
                per_file,
                &mut crate::verilog::NoIncludes,
                &mut front,
            ));
        }
        let refs: Vec<&crate::verilog::ast::SourceFile> = files.iter().collect();
        let options = crate::verilog::ElabOptions::new(dialect);
        let design = crate::verilog::elaborate(&refs, &options, &mut front);
        diags.append(&mut front);
        match design {
            Some(design) => design,
            None => return out,
        }
    };

    // 4. VHDL, analysed against the bundled libraries and elaborated
    //    together, for the same reason the Verilog sources are: an entity
    //    in one file is instantiated from another.
    if !vhdl.is_empty() {
        let mut front = Diagnostics::new();
        let mut library = crate::vhdl::sema::Design::with_stdlib(
            resolved.source_map_mut(),
            crate::vhdl::Standard::Vhdl2008,
            &mut front,
        );
        for id in &vhdl {
            library.add_source(resolved.source_map_mut(), *id, "work", &mut front);
        }
        let analysis = library.analyze(resolved.source_map(), &mut front);
        let analysed_cleanly = !front.has_errors();
        diags.append(&mut front);

        if analysed_cleanly {
            let mut front = Diagnostics::new();
            let options = crate::vhdl::ElabOptions::new();
            match crate::vhdl::elaborate(&analysis, &options, &mut front) {
                Some(vhdl_design) => {
                    // Merge rather than replace: the Verilog half of a
                    // mixed-language project is already in `design`, and a
                    // name defined twice keeps the first, which is what the
                    // `.rtl` merge below does too.
                    for (_, module) in vhdl_design.modules.iter() {
                        if design.module_by_name(module.name.as_str()).is_none() {
                            design.add_module(module.clone());
                        }
                    }
                }
                None => {
                    for id in &vhdl {
                        let name = resolved.source_map().file(*id).name().to_owned();
                        out.skipped
                            .push((name, "VHDL elaboration failed".to_owned()));
                    }
                }
            }
            diags.append(&mut front);
        } else {
            for id in &vhdl {
                let name = resolved.source_map().file(*id).name().to_owned();
                out.skipped.push((name, "VHDL analysis failed".to_owned()));
            }
        }
    }

    // 5. Designs already in the IR text format.
    for id in &rtl {
        let text = resolved.source_map().file(*id).text().to_owned();
        match Design::parse_text(&text, *id) {
            Ok(parsed) => {
                for (_, module) in parsed.modules.iter() {
                    if design.module_by_name(module.name.as_str()).is_none() {
                        design.add_module(module.clone());
                    }
                }
            }
            Err(mut errors) => diags.append(&mut errors),
        }
    }

    // 6. The black boxes, then bind every instance that names one.
    for module in stubs {
        if design.module_by_name(module.name.as_str()).is_none() {
            design.add_module(module);
        }
    }
    design.resolve_instances();

    // 7. The top.
    if let Some(top) = &project.top {
        match design.module_by_name(top) {
            Some(id) => design.top = Some(id),
            None => {
                // Leave no top at all rather than the root a frontend
                // happened to pick: a report naming a module the project
                // never asked for is worse than one naming none.
                design.top = None;
                diags.push(
                    Diagnostic::error(format!("the project's top `{top}` is not in the design"))
                        .with_code(NO_SUCH_TOP)
                        .with_span(project.top_span.unwrap_or(project.span))
                        .with_note("no elaborated module has that name"),
                );
            }
        }
    }

    let mut problems = crate::ir::validate::validate(&design);
    if problems.has_errors() {
        diags.push(
            Diagnostic::error("the elaborated project does not validate")
                .with_code(INVALID_DESIGN)
                .with_span(project.span),
        );
        diags.append(&mut problems);
        return out;
    }

    out.design = Some(design);
    out
}

/// Elaborates a project into one design, or `None` when it could not be
/// built.
///
/// The thin form of [`elaborate`], for a caller that wants only the
/// design; everything else it would have reported goes to `diags`.
pub fn elaborate_project(
    project: &Project,
    resolved: &mut Resolved,
    diags: &mut Diagnostics,
) -> Option<Design> {
    elaborate(project, resolved, diags).design
}
