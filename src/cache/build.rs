//! The incremental build: key every module, hit or elaborate, assemble.
//!
//! [`build`] takes the sources of a design, works out the module graph
//! from [`super::scan`], gives every module a [`CacheKey`], and then, in
//! dependency order, either reads the module back from the store or
//! elaborates it and writes it there. What comes back is one [`Design`]
//! and a per-module account of what hit and what missed.
//!
//! # The dependency direction
//!
//! A module's key folds in the keys of the modules it instantiates. So:
//!
//! - Editing a **leaf** changes the leaf's key, which changes the key of
//!   everything that reaches it, so the leaf and all its dependents miss.
//! - Editing a **top-level** file changes only that module's key; its
//!   dependencies still hit.
//! - Editing an **unrelated** module changes nothing else.
//!
//! That relationship is what makes a cache either useful or wrong, so
//! `tests/cache_build.rs` asserts each of the three directly.
//!
//! # What a module's artefact is
//!
//! Exactly one thing: *the design you get by elaborating this module as
//! the top of its own hierarchy*, rendered in the `.rtl` text format. It
//! therefore contains the module and everything below it, pruned of
//! anything the module does not reach. Nothing in an entry depends on the
//! build that produced it, so two builds that want the same module always
//! agree, and an entry can be read by eye or fed straight to `reticle
//! emit`.
//!
//! The cost of that choice is that a cold build elaborates every module
//! separately rather than the whole source set once, which is slower than
//! not caching at all. [`BuildOptions::only_top`] turns it off for a build
//! that only wants the top; `docs/cache.md` has the measurements.
//!
//! # Parameters
//!
//! The frontends apply parameter and generic overrides to the top of the
//! hierarchy, so an artefact for a module that is *not* the build's top is
//! always its default-parameter elaboration, and the override set is
//! folded into the top's key alone. A module instantiated with overrides
//! is specialised inside its parent's artefact, which is a different
//! artefact under a different key; the two never mix.
//!
//! # Diagnostics
//!
//! | Code    | Meaning                                                        |
//! |---------|----------------------------------------------------------------|
//! | `C0501` | No top module could be selected                                |
//! | `C0502` | The module named as the top is not defined                     |
//! | `C0503` | A source's language is not compiled into this build            |
//! | `C0504` | The hierarchy is recursive, so keys fall back to whole-build    |
//!
//! A cache hit reports nothing, so the warnings a frontend emitted when a
//! module was first elaborated do not reappear on a later build. That is
//! inherent to caching a compilation step; `docs/cache.md` says how to get
//! them back.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::{Design, ModuleRef, Name};
use crate::source::{SourceId, SourceMap};

use super::key::{CacheKey, KIND_ELAB, KIND_SCAN, KeyBuilder};
use super::scan::{FileScan, Language, ModuleGraph, SourceUnit, normalise, scan_file};
use super::store::{Cache, Entry, Storage};

/// Diagnostic code for a build with no module that could be the top.
pub const NO_TOP: &str = "C0501";
/// Diagnostic code for a `top` that names no module of the build.
pub const NO_SUCH_TOP: &str = "C0502";
/// Diagnostic code for a source whose frontend is not compiled in.
pub const UNSUPPORTED_LANGUAGE: &str = "C0503";
/// Diagnostic code for a recursive hierarchy.
pub const RECURSIVE: &str = "C0504";

/// The VHDL standard a build reads its VHDL sources under.
///
/// Mirrors [`crate::vhdl::Standard`] so that [`BuildOptions`] compiles
/// without the `vhdl` feature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VhdlStandard {
    /// IEEE 1076-1993.
    Vhdl93,
    /// IEEE 1076-2008.
    #[default]
    Vhdl2008,
}

impl VhdlStandard {
    /// The word folded into a cache key and shown in reports.
    pub fn keyword(self) -> &'static str {
        match self {
            VhdlStandard::Vhdl93 => "vhdl93",
            VhdlStandard::Vhdl2008 => "vhdl2008",
        }
    }

    #[cfg(feature = "vhdl")]
    fn to_frontend(self) -> crate::vhdl::Standard {
        match self {
            VhdlStandard::Vhdl93 => crate::vhdl::Standard::Vhdl93,
            VhdlStandard::Vhdl2008 => crate::vhdl::Standard::Vhdl2008,
        }
    }
}

/// How a build elaborates, and how it uses the store.
///
/// Build one with [`BuildOptions::new`] and the `with_*` methods rather
/// than as a literal: the synthesis field is only present with the `synth`
/// feature.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    /// The language to read a source as when its name says nothing.
    pub language: Language,
    /// The module to build as the top. When absent, the build takes the
    /// only root of the hierarchy, and reports `C0501` if there is not
    /// exactly one.
    pub top: Option<String>,
    /// Parameter or generic overrides, applied to the top module only.
    pub params: Vec<(String, String)>,
    /// The VHDL working library; `work` when absent.
    pub library: Option<String>,
    /// The VHDL standard.
    pub standard: VhdlStandard,
    /// Build only the top module instead of every module.
    ///
    /// This selects work, not content: an artefact is the same either
    /// way, so the two modes share a store. Off by default, because the
    /// per-module entries are what make a later build that changes one
    /// leaf cheap.
    pub only_top: bool,
    /// The Unix timestamp to record on entries this build writes, or `0`
    /// when the caller has no clock. The library never reads one.
    pub created: u64,
    /// Evict least-recently-used entries past this many bytes.
    pub capacity: Option<u64>,
    /// Synthesise each module that is built, and cache the result under a
    /// key that adds these options.
    #[cfg(feature = "synth")]
    pub synth: Option<crate::synth::SynthOptions>,
}

impl BuildOptions {
    /// Options that read unrecognised sources as `language` and take the
    /// design's only root as the top.
    pub fn new(language: Language) -> BuildOptions {
        BuildOptions {
            language,
            top: None,
            params: Vec::new(),
            library: None,
            standard: VhdlStandard::default(),
            only_top: false,
            created: 0,
            capacity: None,
            #[cfg(feature = "synth")]
            synth: None,
        }
    }

    /// The same options with `top` as the top module.
    pub fn with_top(mut self, top: impl Into<String>) -> BuildOptions {
        self.top = Some(top.into());
        self
    }

    /// The same options with one more override for the top module.
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> BuildOptions {
        self.params.push((name.into(), value.into()));
        self
    }

    /// The same options with a VHDL working library other than `work`.
    pub fn with_library(mut self, library: impl Into<String>) -> BuildOptions {
        self.library = Some(library.into());
        self
    }

    /// The same options under another VHDL standard.
    pub fn with_standard(mut self, standard: VhdlStandard) -> BuildOptions {
        self.standard = standard;
        self
    }

    /// The same options, building only the top module.
    pub fn with_only_top(mut self, only_top: bool) -> BuildOptions {
        self.only_top = only_top;
        self
    }

    /// The same options, stamping new entries with `created`.
    pub fn with_created(mut self, created: u64) -> BuildOptions {
        self.created = created;
        self
    }

    /// The same options, with the store capped at `bytes`.
    pub fn with_capacity(mut self, bytes: u64) -> BuildOptions {
        self.capacity = Some(bytes);
        self
    }

    /// The same options, synthesising every module that is built.
    #[cfg(feature = "synth")]
    pub fn with_synth(mut self, synth: crate::synth::SynthOptions) -> BuildOptions {
        self.synth = Some(synth);
        self
    }

    /// The working library's name.
    pub fn library_name(&self) -> &str {
        self.library.as_deref().unwrap_or("work")
    }
}

/// Which stage of the pipeline a [`ModuleBuild`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Elaboration of a module into the IR.
    Elaborate,
    /// Synthesis of an elaborated module into a netlist.
    Synthesise,
}

impl Stage {
    /// The word used in [`BuildResult::report`].
    pub fn keyword(self) -> &'static str {
        match self {
            Stage::Elaborate => "elaborate",
            Stage::Synthesise => "synthesise",
        }
    }
}

/// What happened to one unit of work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The artefact came from the store.
    Hit,
    /// The artefact had to be produced, and was then stored.
    Miss,
    /// The artefact could not be produced; diagnostics say why.
    Failed,
}

impl Outcome {
    /// The word used in [`BuildResult::report`].
    pub fn keyword(self) -> &'static str {
        match self {
            Outcome::Hit => "hit",
            Outcome::Miss => "miss",
            Outcome::Failed => "failed",
        }
    }
}

/// One module's trip through one stage of the build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleBuild {
    /// The module's name, as written in the source.
    pub name: String,
    /// The stage this record is about.
    pub stage: Stage,
    /// The key the artefact is filed under.
    pub key: CacheKey,
    /// Whether it hit, missed or failed.
    pub outcome: Outcome,
}

/// One source file's trip through the dependency scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanBuild {
    /// The file's name, as the caller gave it.
    pub name: String,
    /// The language it was read as.
    pub language: Language,
    /// The key the scan is filed under.
    pub key: CacheKey,
    /// Whether the scan hit, missed or failed.
    pub outcome: Outcome,
}

/// What one incremental build produced.
#[derive(Debug)]
pub struct BuildResult {
    /// The design, when one could be assembled.
    pub design: Option<Design>,
    /// The map holding every source the build read, plus one file per
    /// artefact taken from the store, so every span in `diags` resolves.
    pub sources: SourceMap,
    /// The top module's name, when one was selected.
    pub top: Option<String>,
    /// One record per file scanned, in the order the files were given.
    pub scans: Vec<ScanBuild>,
    /// One record per module and stage, in dependency order.
    pub modules: Vec<ModuleBuild>,
    /// How many lookups hit, scans included.
    pub hits: usize,
    /// How many lookups missed, scans included.
    pub misses: usize,
}

impl BuildResult {
    /// A deterministic, multi-line summary of the build.
    ///
    /// Keys are deliberately left out: they move with the compiler
    /// version, and this is what a golden test compares. Read them off
    /// [`BuildResult::modules`] instead.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("sources\n");
        for scan in &self.scans {
            out.push_str(&format!(
                "  {}: {}, scan {}\n",
                scan.name,
                scan.language.keyword(),
                scan.outcome.keyword()
            ));
        }
        out.push_str("modules\n");
        for module in &self.modules {
            out.push_str(&format!(
                "  {}: {}, {}\n",
                module.name,
                module.stage.keyword(),
                module.outcome.keyword()
            ));
        }
        out.push_str("summary\n");
        out.push_str(&format!(
            "  top: {}\n",
            self.top.as_deref().unwrap_or("<none>")
        ));
        out.push_str(&format!("  hits: {}\n", self.hits));
        out.push_str(&format!("  misses: {}\n", self.misses));
        out
    }

    /// The outcome recorded for `module` at `stage`, if any.
    pub fn outcome(&self, module: &str, stage: Stage) -> Option<Outcome> {
        self.modules
            .iter()
            .find(|m| m.name == module && m.stage == stage)
            .map(|m| m.outcome)
    }
}

/// Builds `sources` incrementally against `storage`.
///
/// Every module gets a key from its sources, its options and its
/// dependencies' keys; one that is already in the store is read back, one
/// that is not is elaborated and written there. The result holds the
/// assembled design and a record of every hit and miss.
///
/// Diagnostics go to `diags` with spans into [`BuildResult::sources`],
/// which is returned rather than taken because the artefacts read back
/// from the store are added to it as files of their own.
pub fn build(
    sources: &[SourceUnit],
    options: &BuildOptions,
    storage: &mut dyn Storage,
    diags: &mut Diagnostics,
) -> BuildResult {
    let mut cache = match options.capacity {
        Some(bytes) => Cache::with_capacity(storage, bytes),
        None => Cache::new(storage),
    };
    let mut builder = Builder::new(sources, options);
    builder.run(&mut cache, diags);
    builder.finish(&cache)
}

/// The state of one build.
struct Builder<'a> {
    options: &'a BuildOptions,
    units: Vec<Unit<'a>>,
    map: SourceMap,
    graph: ModuleGraph,
    cyclic: Vec<String>,
    keys: BTreeMap<String, CacheKey>,
    artefacts: BTreeMap<String, Design>,
    scans: Vec<ScanBuild>,
    modules: Vec<ModuleBuild>,
    top: Option<String>,
    design: Option<Design>,
    /// Lazily parsed Verilog files, one slot per unit.
    #[cfg(feature = "verilog")]
    asts: Vec<Option<crate::verilog::ast::SourceFile>>,
    /// The VHDL analysis, built on the first VHDL miss.
    #[cfg(feature = "vhdl")]
    analysis: Option<crate::vhdl::sema::Analysis>,
}

/// One source unit, resolved to a language and a place in the map.
struct Unit<'a> {
    source: &'a SourceUnit,
    language: Language,
    id: SourceId,
}

impl<'a> Builder<'a> {
    fn new(sources: &'a [SourceUnit], options: &'a BuildOptions) -> Builder<'a> {
        let mut map = SourceMap::new();
        let mut units = Vec::with_capacity(sources.len());
        for source in sources {
            let language = source.language_or(options.language);
            // A file too large for the map is reported when it is scanned;
            // an empty stand-in keeps the indices lined up.
            let id = map
                .add(source.name.clone(), source.text.clone())
                .or_else(|_| map.add(source.name.clone(), ""))
                .expect("an empty file always fits");
            units.push(Unit {
                source,
                language,
                id,
            });
        }
        Builder {
            options,
            #[cfg(feature = "verilog")]
            asts: (0..units.len()).map(|_| None).collect(),
            units,
            map,
            graph: ModuleGraph::new(),
            cyclic: Vec::new(),
            keys: BTreeMap::new(),
            artefacts: BTreeMap::new(),
            scans: Vec::new(),
            modules: Vec::new(),
            top: None,
            design: None,
            #[cfg(feature = "vhdl")]
            analysis: None,
        }
    }

    fn run(&mut self, cache: &mut Cache<'_>, diags: &mut Diagnostics) {
        self.scan_all(cache, diags);
        self.cyclic = self.graph.cyclic();
        if !self.cyclic.is_empty() {
            diags.push(
                Diagnostic::error(format!(
                    "the hierarchy is recursive: {} instantiate each other",
                    self.cyclic.join(", ")
                ))
                .with_code(RECURSIVE)
                .with_note(
                    "their keys fall back to the whole source set, so any edit rebuilds them"
                        .to_owned(),
                ),
            );
        }
        self.compute_keys();
        let Some(top) = self.select_top(diags) else {
            return;
        };
        self.top = self.graph.display_name(&top).map(str::to_owned);

        let wanted: Vec<String> = if self.options.only_top {
            vec![top.clone()]
        } else {
            self.graph.topological()
        };
        for name in &wanted {
            self.build_module(name, name == &top, cache, diags);
        }
        self.design = self.assemble(&top, &wanted);
    }

    /// Scans every file, through the cache.
    fn scan_all(&mut self, cache: &mut Cache<'_>, diags: &mut Diagnostics) {
        for index in 0..self.units.len() {
            let (name, language, key) = {
                let unit = &self.units[index];
                let mut builder = KeyBuilder::new(KIND_SCAN);
                builder
                    .subject(&unit.source.name)
                    .option("language", unit.language.keyword())
                    .option("standard", self.options.standard.keyword())
                    .source(&unit.source.name, &unit.source.text);
                (unit.source.name.clone(), unit.language, builder.finish())
            };

            let cached = cache
                .get(key)
                .and_then(|entry| entry.text().and_then(FileScan::decode));
            let (scan, outcome) = match cached {
                Some(scan) => (scan, Outcome::Hit),
                None => {
                    if !self.language_available(language) {
                        diags.push(
                            Diagnostic::error(format!(
                                "`{name}` is {}, which this build of reticle does not include",
                                language.keyword()
                            ))
                            .with_code(UNSUPPORTED_LANGUAGE)
                            .with_note(format!(
                                "rebuild with the `{}` feature",
                                language.frontend()
                            )),
                        );
                        self.scans.push(ScanBuild {
                            name,
                            language,
                            key,
                            outcome: Outcome::Failed,
                        });
                        continue;
                    }
                    let id = self.units[index].id;
                    let scan = scan_file(&mut self.map, id, language, diags);
                    cache.insert(
                        key,
                        &format!("scan:{}", language.keyword()),
                        self.options.created,
                        scan.encode().into_bytes(),
                    );
                    (scan, Outcome::Miss)
                }
            };
            self.graph.add(index, language, &scan);
            self.scans.push(ScanBuild {
                name,
                language,
                key,
                outcome,
            });
        }
    }

    /// True when the frontend for `language` is compiled in.
    fn language_available(&self, language: Language) -> bool {
        match language {
            Language::Rtl => true,
            Language::Verilog | Language::SystemVerilog => cfg!(feature = "verilog"),
            Language::Vhdl => cfg!(feature = "vhdl"),
        }
    }

    /// Gives every module a key, dependencies first.
    fn compute_keys(&mut self) {
        for name in self.graph.topological() {
            let key = self.elaboration_key(&name);
            self.keys.insert(name, key);
        }
    }

    /// The language a module is elaborated by, from the first file that
    /// defines it.
    fn language_of(&self, module: &str) -> Language {
        self.graph
            .files_of(module)
            .first()
            .map_or(self.options.language, |&i| self.units[i].language)
    }

    /// The Verilog dialect the whole build elaborates under.
    ///
    /// SystemVerilog wins, so a build with one `.sv` file among `.v` files
    /// elaborates as SystemVerilog. It is one decision for the build
    /// rather than one per module so that a module's dialect cannot change
    /// when an unrelated file is added or removed — which would be a key
    /// input that nothing folded in.
    fn dialect_word(&self) -> &'static str {
        if self
            .units
            .iter()
            .any(|unit| unit.language == Language::SystemVerilog)
        {
            Language::SystemVerilog.keyword()
        } else {
            Language::Verilog.keyword()
        }
    }

    /// The key of one module; see [`super::key`] for the full list of
    /// inputs.
    fn elaboration_key(&self, module: &str) -> CacheKey {
        let display = self.graph.display_name(module).unwrap_or(module);
        let is_top = self.is_designated_top(module);
        let language = self.language_of(module);

        let mut builder = KeyBuilder::new(KIND_ELAB);
        builder
            .subject(display)
            .option("language", language.keyword())
            .option("dialect", self.dialect_word())
            .option("standard", self.options.standard.keyword())
            .option("library", self.options.library_name())
            .flag("is-top", is_top);
        builder.params(if is_top { &self.options.params } else { &[] });

        if self.cyclic.contains(&module.to_owned()) {
            // A recursive hierarchy has no well-founded key, so fall back
            // to hashing the whole source set. Conservative: every edit
            // rebuilds it, and nothing stale is ever served.
            builder.flag("recursive", true);
            for unit in &self.units {
                builder.source(&unit.source.name, &unit.source.text);
            }
            return builder.finish();
        }

        for &index in self.graph.files_of(module) {
            let unit = &self.units[index];
            builder.source(&unit.source.name, &unit.source.text);
        }
        for &index in self.graph.globals() {
            let unit = &self.units[index];
            builder.source(&unit.source.name, &unit.source.text);
        }
        for dep in self.graph.deps_of(module) {
            // A dependency nothing defines (a vendor primitive, a black
            // box) contributes its name and the zero key: there are no
            // sources of its own to change.
            let key = self.keys.get(dep).copied().unwrap_or_default();
            builder.dependency(dep, key);
        }
        builder.finish()
    }

    /// True when `module` is the module the build's options name as top.
    fn is_designated_top(&self, module: &str) -> bool {
        match &self.options.top {
            Some(top) => normalise(top, self.language_of(module)) == module,
            None => false,
        }
    }

    /// The files that have to be handed to the frontend to elaborate
    /// `module`: its own, the globals, and the same-frontend closure of
    /// its dependencies.
    ///
    /// Indices come back sorted, so the frontend sees the files in the
    /// order the caller gave them.
    ///
    /// Only the Verilog frontend needs this: VHDL analyses every source
    /// together (the bundled libraries make a per-module analysis far more
    /// expensive than one shared one) and an `.rtl` module is already whole.
    #[cfg(feature = "verilog")]
    fn closure(&self, module: &str) -> Vec<usize> {
        let frontend = self.language_of(module).frontend();
        let mut files: BTreeSet<usize> = BTreeSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![module.to_owned()];
        while let Some(name) = stack.pop() {
            if !self.graph.contains(&name) || !seen.insert(name.clone()) {
                continue;
            }
            if self.language_of(&name).frontend() != frontend {
                continue;
            }
            files.extend(self.graph.files_of(&name).iter().copied());
            stack.extend(self.graph.deps_of(&name).iter().cloned());
        }
        for &index in self.graph.globals() {
            if self.units[index].language.frontend() == frontend {
                files.insert(index);
            }
        }
        files.into_iter().collect()
    }

    /// Picks the top module, reporting when there is no single answer.
    fn select_top(&self, diags: &mut Diagnostics) -> Option<String> {
        if let Some(top) = &self.options.top {
            for module in self.graph.modules() {
                if normalise(top, self.language_of(module)) == module {
                    return Some(module.to_owned());
                }
            }
            let mut names: Vec<&str> = self.graph.modules();
            names.sort_unstable();
            diags.push(
                Diagnostic::error(format!("no module named `{top}` to use as the top"))
                    .with_code(NO_SUCH_TOP)
                    .with_note(if names.is_empty() {
                        "the sources define no modules".to_owned()
                    } else {
                        format!("the sources define {}", names.join(", "))
                    }),
            );
            return None;
        }
        let roots = self.graph.roots();
        match roots.len() {
            1 => Some(roots[0].to_owned()),
            0 => {
                diags.push(
                    Diagnostic::error("the sources define no module that could be the top")
                        .with_code(NO_TOP),
                );
                None
            }
            _ => {
                diags.push(
                    Diagnostic::error(format!(
                        "the sources have {} possible tops: {}",
                        roots.len(),
                        roots.join(", ")
                    ))
                    .with_code(NO_TOP)
                    .with_note("name one with the build's `top` option".to_owned()),
                );
                None
            }
        }
    }

    /// Builds one module: hit, or elaborate and store.
    fn build_module(
        &mut self,
        module: &str,
        is_top: bool,
        cache: &mut Cache<'_>,
        diags: &mut Diagnostics,
    ) {
        let display = self.graph.display_name(module).unwrap_or(module).to_owned();
        let key = self.keys.get(module).copied().unwrap_or_default();

        #[cfg(feature = "synth")]
        let synth_key = self.options.synth.as_ref().map(|s| synth_key(key, s));
        #[cfg(feature = "synth")]
        if let Some(synth_key) = synth_key
            && let Some(design) = self.take_cached(synth_key, &display, cache)
        {
            // A synthesised hit makes elaborating pointless: the netlist is
            // the artefact the build wants.
            self.artefacts.insert(module.to_owned(), design);
            self.modules.push(ModuleBuild {
                name: display,
                stage: Stage::Synthesise,
                key: synth_key,
                outcome: Outcome::Hit,
            });
            return;
        }

        let design = match self.take_cached(key, &display, cache) {
            Some(design) => {
                self.modules.push(ModuleBuild {
                    name: display.clone(),
                    stage: Stage::Elaborate,
                    key,
                    outcome: Outcome::Hit,
                });
                design
            }
            None => match self.elaborate(module, &display, is_top, diags) {
                Some(design) => {
                    cache.insert(
                        key,
                        &format!("elab:{}", self.language_of(module).keyword()),
                        self.options.created,
                        design.to_text().into_bytes(),
                    );
                    self.modules.push(ModuleBuild {
                        name: display.clone(),
                        stage: Stage::Elaborate,
                        key,
                        outcome: Outcome::Miss,
                    });
                    design
                }
                None => {
                    self.modules.push(ModuleBuild {
                        name: display,
                        stage: Stage::Elaborate,
                        key,
                        outcome: Outcome::Failed,
                    });
                    return;
                }
            },
        };

        #[cfg(feature = "synth")]
        let mut design = design;
        #[cfg(feature = "synth")]
        if let (Some(synth_key), Some(options)) = (synth_key, self.options.synth.as_ref()) {
            crate::synth::run(&mut design, options, diags);
            cache.insert(
                synth_key,
                "synth",
                self.options.created,
                design.to_text().into_bytes(),
            );
            self.modules.push(ModuleBuild {
                name: display,
                stage: Stage::Synthesise,
                key: synth_key,
                outcome: Outcome::Miss,
            });
        }

        self.artefacts.insert(module.to_owned(), design);
    }

    /// Reads an artefact back, adding its text to the source map so that
    /// the spans of the parsed design resolve.
    fn take_cached(
        &mut self,
        key: CacheKey,
        display: &str,
        cache: &mut Cache<'_>,
    ) -> Option<Design> {
        let entry: Entry = cache.get(key)?;
        let text = entry.text()?.to_owned();
        let id = self
            .map
            .add(format!("<cache:{display}>"), text.clone())
            .ok()?;
        // A stored artefact that no longer parses is a corrupt store, not
        // a user error: drop it and rebuild rather than report it.
        match Design::parse_text(&text, id) {
            Ok(design) => Some(design),
            Err(_) => {
                cache.remove(key);
                None
            }
        }
    }

    /// Elaborates one module as the top of its own hierarchy.
    fn elaborate(
        &mut self,
        module: &str,
        display: &str,
        is_top: bool,
        diags: &mut Diagnostics,
    ) -> Option<Design> {
        // `is_top` and `display` are read only by the frontends.
        #[cfg(not(any(feature = "verilog", feature = "vhdl")))]
        let _ = is_top;
        let language = self.language_of(module);
        let mut design = match language {
            Language::Rtl => self.elaborate_rtl(module, diags)?,
            #[cfg(feature = "verilog")]
            Language::Verilog | Language::SystemVerilog => {
                self.elaborate_verilog(module, display, is_top, diags)?
            }
            #[cfg(not(feature = "verilog"))]
            Language::Verilog | Language::SystemVerilog => return None,
            #[cfg(feature = "vhdl")]
            Language::Vhdl => self.elaborate_vhdl(display, is_top, diags)?,
            #[cfg(not(feature = "vhdl"))]
            Language::Vhdl => return None,
        };
        // A parameterised top is renamed by the Verilog frontend (`p`
        // with `W=16` becomes `p$W_16`), so the frontend's own choice of
        // top is authoritative; an `.rtl` source has a `top` line of its
        // own, which is not the module asked for.
        let top = match language {
            Language::Rtl => design.module_by_name(display)?,
            _ => design.top.or_else(|| design.module_by_name(display))?,
        };
        design.top = Some(top);
        design.remove_unused_modules(top);
        Some(design)
    }

    /// Takes one module's hierarchy out of an `.rtl` source.
    fn elaborate_rtl(&mut self, module: &str, diags: &mut Diagnostics) -> Option<Design> {
        let &index = self.graph.files_of(module).first()?;
        let id = self.units[index].id;
        let text = self.map.file(id).text().to_owned();
        match Design::parse_text(&text, id) {
            Ok(design) => Some(design),
            Err(mut errors) => {
                diags.append(&mut errors);
                None
            }
        }
    }

    #[cfg(feature = "verilog")]
    fn elaborate_verilog(
        &mut self,
        module: &str,
        display: &str,
        is_top: bool,
        diags: &mut Diagnostics,
    ) -> Option<Design> {
        use crate::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

        let closure = self.closure(module);
        let dialect = if self.dialect_word() == Language::SystemVerilog.keyword() {
            Dialect::SystemVerilog
        } else {
            Dialect::Verilog2005
        };
        for &index in &closure {
            if self.asts[index].is_none() {
                let id = self.units[index].id;
                let file_dialect = match self.units[index].language {
                    Language::SystemVerilog => Dialect::SystemVerilog,
                    _ => Dialect::Verilog2005,
                };
                let mut sink = Diagnostics::new();
                let file =
                    parse_source(&mut self.map, id, file_dialect, &mut NoIncludes, &mut sink);
                // The scan already reported this file's parse problems.
                self.asts[index] = Some(file);
            }
        }
        let files: Vec<&crate::verilog::ast::SourceFile> = closure
            .iter()
            .filter_map(|&i| self.asts[i].as_ref())
            .collect();

        let mut options = ElabOptions::new(dialect);
        options.top = Some(display.to_owned());
        if is_top {
            options.params = self.options.params.clone();
        }
        elaborate(&files, &options, diags)
    }

    #[cfg(feature = "vhdl")]
    fn elaborate_vhdl(
        &mut self,
        display: &str,
        is_top: bool,
        diags: &mut Diagnostics,
    ) -> Option<Design> {
        use crate::vhdl::elab::ElabOptions;
        use crate::vhdl::sema::Design as VhdlDesign;

        if self.analysis.is_none() {
            // One analysis serves the whole build: it carries the bundled
            // std and ieee libraries, which are far more expensive than any
            // single entity's elaboration.
            let standard = self.options.standard.to_frontend();
            let mut sink = Diagnostics::new();
            let mut library = VhdlDesign::with_stdlib(&mut self.map, standard, &mut sink);
            let ids: Vec<SourceId> = self
                .units
                .iter()
                .filter(|u| u.language == Language::Vhdl)
                .map(|u| u.id)
                .collect();
            let work = self.options.library_name().to_owned();
            for id in ids {
                library.add_source(&self.map, id, &work, &mut sink);
            }
            self.analysis = Some(library.analyze(&self.map, &mut sink));
            diags.append(&mut sink);
        }
        let analysis = self.analysis.as_ref().expect("just built");

        let mut options = ElabOptions::new();
        options.top = Some(display.to_owned());
        options.library = self.options.library.clone();
        if is_top {
            options.generics = self.options.params.clone();
        }
        crate::vhdl::elaborate(analysis, &options, diags)
    }

    /// Merges the artefacts into one design, the top's first.
    ///
    /// Each artefact is a whole hierarchy, so merging means adding the
    /// modules a previous artefact did not already define. The first
    /// definition of a repeated name wins, which is the rule
    /// [`crate::ip::elaborate`] uses for a mixed-language project; two
    /// roots that specialise a shared module differently therefore keep
    /// the first root's copy.
    fn assemble(&self, top: &str, wanted: &[String]) -> Option<Design> {
        let mut out = Design::new();
        let roots: BTreeSet<&str> = self.graph.roots().into_iter().collect();
        let mut order: Vec<&String> = Vec::new();
        if let Some(name) = wanted.iter().find(|n| *n == top) {
            order.push(name);
        }
        for name in wanted {
            if name != top && roots.contains(name.as_str()) {
                order.push(name);
            }
        }
        for name in order {
            if let Some(design) = self.artefacts.get(name) {
                merge_into(&mut out, design);
            }
        }
        if out.modules.is_empty() {
            return None;
        }
        out.resolve_instances();
        // The artefact knows which of its modules is the top, which is the
        // only way to find a parameterised variant by name.
        out.top = self
            .artefacts
            .get(top)
            .and_then(|design| design.top_module())
            .map(|module| module.name.as_str().to_owned())
            .and_then(|name| out.module_by_name(&name));
        Some(out)
    }

    fn finish(self, cache: &Cache<'_>) -> BuildResult {
        let hits = usize::try_from(cache.hits()).unwrap_or(usize::MAX);
        let misses = usize::try_from(cache.misses()).unwrap_or(usize::MAX);
        BuildResult {
            design: self.design,
            sources: self.map,
            top: self.top,
            scans: self.scans,
            modules: self.modules,
            hits,
            misses,
        }
    }
}

/// Copies the modules of `src` that `dest` does not already have.
///
/// Instance references are rewritten to names on the way in and resolved
/// once at the end, so no arena id crosses between designs.
fn merge_into(dest: &mut Design, src: &Design) {
    for (_, module) in src.modules.iter() {
        if dest.module_by_name(module.name.as_str()).is_some() {
            continue;
        }
        let mut module = module.clone();
        for (_, instance) in module.instances.iter_mut() {
            if let ModuleRef::Resolved(id) = instance.module {
                let name = src
                    .modules
                    .get(id)
                    .map_or_else(|| Name::new(format!("?{id}")), |m| m.name.clone());
                instance.module = ModuleRef::Unresolved(name);
            }
        }
        dest.add_module(module);
    }
}

/// The key of a synthesised module, from its elaborated key.
///
/// Every option that can change the netlist is folded in.
/// [`SynthOptions::validate`](crate::synth::SynthOptions) is not: it only
/// decides whether the passes check their own invariants, so folding it in
/// would give a debug build and a release build different keys for
/// identical output. `verify_equivalence` *is* folded in, because it
/// changes what the build reports even though it leaves the netlist alone,
/// and a hit that skipped the proof would be a silent loss of checking.
///
/// The encoding is folded in through its `Debug` spelling, which is stable
/// for a fieldless enum; renaming a variant would retire the entries that
/// used it, which is a spurious rebuild rather than a wrong one.
#[cfg(feature = "synth")]
fn synth_key(elaboration: CacheKey, options: &crate::synth::SynthOptions) -> CacheKey {
    use super::key::KIND_SYNTH;

    let mut builder = KeyBuilder::new(KIND_SYNTH);
    builder
        .derived_from(elaboration)
        .flag("cellify", options.cellify)
        .option("fsm-encoding", &format!("{:?}", options.fsm_encoding))
        .flag("keep-hierarchy", options.keep_hierarchy)
        .number("max-iterations", u64::from(options.max_iterations))
        .number("max-unroll", u64::from(options.max_unroll))
        .flag("verify-equivalence", options.verify_equivalence);
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MemoryStorage;

    /// A two-module design in the IR text format, which builds whatever
    /// frontends are compiled in.
    const RTL: &str = "top t\n\nmodule leaf\n  net %o u1 wire\n  port o out %o\n  \
                       assign %o = 1'b0\nend\n\nmodule t\n  net %o u1 wire\n  \
                       port o out %o\n  instance u of leaf (o=%o)\nend\n";

    /// A one-module design in the same format.
    const ONE: &str = "module m\n  net %o u1 wire\n  port o out %o\n  \
                       assign %o = 1'b0\nend\n";

    fn units(files: &[(&str, &str)]) -> Vec<SourceUnit> {
        files
            .iter()
            .map(|(name, text)| SourceUnit::new(*name, *text))
            .collect()
    }

    #[test]
    fn options_are_built_by_the_helpers() {
        let options = BuildOptions::new(Language::Verilog)
            .with_top("top")
            .with_param("W", "8")
            .with_library("lib")
            .with_standard(VhdlStandard::Vhdl93)
            .with_only_top(true)
            .with_created(42)
            .with_capacity(1024);
        assert_eq!(options.top.as_deref(), Some("top"));
        assert_eq!(options.params, vec![("W".to_owned(), "8".to_owned())]);
        assert_eq!(options.library_name(), "lib");
        assert_eq!(options.standard.keyword(), "vhdl93");
        assert!(options.only_top);
        assert_eq!(options.created, 42);
        assert_eq!(options.capacity, Some(1024));
        assert_eq!(BuildOptions::new(Language::Vhdl).library_name(), "work");
    }

    #[test]
    fn an_empty_build_reports_no_top() {
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let result = build(
            &[],
            &BuildOptions::new(Language::Verilog),
            &mut storage,
            &mut diags,
        );
        assert!(result.design.is_none());
        assert!(diags.iter().any(|d| d.code == Some(NO_TOP)));
        assert_eq!(result.report().lines().next(), Some("sources"));
    }

    #[test]
    fn an_rtl_design_builds_without_a_frontend() {
        let sources = units(&[("d.rtl", RTL)]);
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let options = BuildOptions::new(Language::Rtl).with_top("t");

        let cold = build(&sources, &options, &mut storage, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&cold.sources));
        assert_eq!(cold.top.as_deref(), Some("t"));
        assert_eq!(cold.outcome("t", Stage::Elaborate), Some(Outcome::Miss));
        assert_eq!(cold.outcome("leaf", Stage::Elaborate), Some(Outcome::Miss));

        let warm = build(&sources, &options, &mut storage, &mut diags);
        assert_eq!(warm.misses, 0, "{}", warm.report());
        assert_eq!(
            cold.design.unwrap().to_text(),
            warm.design.unwrap().to_text()
        );
    }

    #[test]
    fn a_language_that_is_not_compiled_in_is_reported() {
        // `.rtl` always works, so use whichever frontend is absent.
        let missing = if cfg!(feature = "verilog") {
            if cfg!(feature = "vhdl") {
                return;
            }
            ("t.vhd", "entity t is end entity;\n")
        } else {
            ("t.v", "module t; endmodule\n")
        };
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let result = build(
            &units(&[missing]),
            &BuildOptions::new(Language::Verilog),
            &mut storage,
            &mut diags,
        );
        assert!(diags.iter().any(|d| d.code == Some(UNSUPPORTED_LANGUAGE)));
        assert!(result.design.is_none());
    }

    #[test]
    fn a_report_is_deterministic() {
        let sources = units(&[("d.rtl", ONE)]);
        let options = BuildOptions::new(Language::Rtl);
        let mut a = MemoryStorage::new();
        let mut b = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let first = build(&sources, &options, &mut a, &mut diags);
        let second = build(&sources, &options, &mut b, &mut diags);
        assert_eq!(first.report(), second.report());
        assert_eq!(first.modules[0].key, second.modules[0].key);
        assert_eq!(
            first.report(),
            "sources\n  d.rtl: rtl, scan miss\nmodules\n  m: elaborate, miss\n\
             summary\n  top: m\n  hits: 0\n  misses: 2\n"
        );
    }

    #[cfg(feature = "synth")]
    #[test]
    fn synthesis_options_change_the_synthesis_key() {
        use crate::synth::SynthOptions;
        let base = CacheKey::default();
        let mut options = SynthOptions::default();
        let first = synth_key(base, &options);
        options.max_iterations += 1;
        assert_ne!(first, synth_key(base, &options));

        // `validate` is deliberately not an input: it changes no output.
        let a = SynthOptions {
            validate: true,
            ..SynthOptions::default()
        };
        let b = SynthOptions {
            validate: false,
            ..SynthOptions::default()
        };
        assert_eq!(synth_key(base, &a), synth_key(base, &b));

        // `verify_equivalence` is, because it changes what is reported.
        let c = SynthOptions {
            verify_equivalence: !SynthOptions::default().verify_equivalence,
            ..SynthOptions::default()
        };
        assert_ne!(
            synth_key(base, &SynthOptions::default()),
            synth_key(base, &c)
        );
    }
}
