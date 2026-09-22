//! The dependency scan: which modules a file defines, and what they use.
//!
//! An incremental build has to know the shape of the design before it can
//! decide what to rebuild, and it must find that out without doing the
//! expensive work. So each source file is scanned once for two facts: the
//! modules (Verilog) or entities (VHDL) it defines, and the names those
//! instantiate. That is all [`ModuleGraph`] needs.
//!
//! Scanning parses, which is why the summary is itself cached (under
//! [`key::KIND_SCAN`](super::key::KIND_SCAN)): the *AST* is not worth
//! keeping — it is large, it has no stable serialised form and rebuilding
//! it is cheap next to elaboration — but the dozen names extracted from it
//! are, because otherwise a build with nothing to do would still parse
//! every file.
//!
//! # Files that define nothing
//!
//! A file with no module or entity of its own — a Verilog package or a
//! header of compilation-unit items, a VHDL package, context or
//! configuration — is a **global**. It is handed to every elaboration and
//! folded into every key, so editing one invalidates the whole build. That
//! is the conservative direction on purpose: attributing a package to the
//! modules that import it would need name resolution, which is most of
//! elaboration.
//!
//! # Limits
//!
//! - `` `include `` is not followed, exactly as [`crate::ip::elaborate`]
//!   does not follow it: a build lists its files. An included file that is
//!   also listed is a global; one that is not listed is not seen at all.
//! - A name a file instantiates but nothing in the build defines (a vendor
//!   primitive, a black box) is recorded as a dependency and resolves to
//!   nothing. It contributes its name to the key and no sources, which is
//!   right: there are no sources to change.
//! - VHDL identifiers are case-insensitive, so the graph matches VHDL
//!   names case-insensitively and keeps the spelling of the first
//!   definition for display. Extended identifiers (`\Foo\`), which are
//!   case-sensitive, are treated like any other name and so may be
//!   conflated with a basic identifier of the same letters.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::Diagnostics;
use crate::source::{SourceId, SourceMap};

/// The language a source file is read as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// Verilog-2005.
    #[default]
    Verilog,
    /// SystemVerilog (the synthesisable subset the frontend accepts).
    SystemVerilog,
    /// VHDL, in the standard the build selects.
    Vhdl,
    /// A design already in the IR's `.rtl` text format.
    Rtl,
}

impl Language {
    /// The word used in reports, diagnostics and cache keys.
    pub fn keyword(self) -> &'static str {
        match self {
            Language::Verilog => "verilog",
            Language::SystemVerilog => "systemverilog",
            Language::Vhdl => "vhdl",
            Language::Rtl => "rtl",
        }
    }

    /// The language a file name's extension implies, if any.
    pub fn from_path(path: &str) -> Option<Language> {
        let ext = path.rsplit('.').next()?.to_ascii_lowercase();
        match ext.as_str() {
            "v" | "vh" => Some(Language::Verilog),
            "sv" | "svh" => Some(Language::SystemVerilog),
            "vhd" | "vhdl" => Some(Language::Vhdl),
            "rtl" => Some(Language::Rtl),
            _ => None,
        }
    }

    /// True when two languages are elaborated by the same frontend.
    ///
    /// Verilog and SystemVerilog differ only in dialect, so they are one
    /// frontend; VHDL and `.rtl` are each their own.
    pub fn frontend(self) -> &'static str {
        match self {
            Language::Verilog | Language::SystemVerilog => "verilog",
            Language::Vhdl => "vhdl",
            Language::Rtl => "rtl",
        }
    }
}

/// One source file handed to a build.
///
/// The name is what diagnostics show and is folded into cache keys; the
/// text is the file's entire content. Nothing here refers to a path on
/// disk, so a build driven from memory, from a version-control checkout or
/// from a network fetch all behave the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceUnit {
    /// The name to report the file under, usually its path.
    pub name: String,
    /// The file's full text.
    pub text: String,
    /// The language to read it as; `None` takes the build's default, or
    /// the extension when it recognises one.
    pub language: Option<Language>,
}

impl SourceUnit {
    /// A unit whose language is taken from its name's extension.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceUnit {
        let name = name.into();
        let language = Language::from_path(&name);
        SourceUnit {
            name,
            text: text.into(),
            language,
        }
    }

    /// The same unit, read as `language` whatever its name says.
    pub fn with_language(mut self, language: Language) -> SourceUnit {
        self.language = Some(language);
        self
    }

    /// The language to read this unit as, falling back to `default`.
    pub fn language_or(&self, default: Language) -> Language {
        self.language.unwrap_or(default)
    }
}

/// One module a file defines, and what it instantiates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleScan {
    /// The name as written in the source.
    pub name: String,
    /// The names it instantiates, sorted and deduplicated.
    pub deps: Vec<String>,
}

/// What one file contributes to the design.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileScan {
    /// The modules defined in it, in source order.
    pub modules: Vec<ModuleScan>,
}

impl FileScan {
    /// True when the file defines no module of its own; see the module
    /// docs on globals.
    pub fn is_global(&self) -> bool {
        self.modules.is_empty()
    }

    /// The scan as the text a cache entry holds.
    ///
    /// One record per line, so an entry stays readable:
    ///
    /// ```text
    /// defines top
    /// uses mid
    /// uses other
    /// defines helper
    /// ```
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for module in &self.modules {
            out.push_str("defines ");
            out.push_str(&escape(&module.name));
            out.push('\n');
            for dep in &module.deps {
                out.push_str("uses ");
                out.push_str(&escape(dep));
                out.push('\n');
            }
        }
        out
    }

    /// Reads back what [`FileScan::encode`] wrote.
    ///
    /// Returns `None` for anything else, so a damaged entry is a miss
    /// rather than a wrong graph.
    pub fn decode(text: &str) -> Option<FileScan> {
        let mut scan = FileScan::default();
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("defines ") {
                scan.modules.push(ModuleScan {
                    name: unescape(name)?,
                    deps: Vec::new(),
                });
            } else {
                let name = line.strip_prefix("uses ")?;
                scan.modules.last_mut()?.deps.push(unescape(name)?);
            }
        }
        Some(scan)
    }
}

/// Escapes the characters that would break the one-record-per-line form.
///
/// An escaped Verilog or VHDL identifier may hold anything, a newline
/// included, so the encoding cannot assume otherwise.
fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// The inverse of [`escape`]; `None` on a trailing or unknown escape.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

/// One module of the graph.
#[derive(Clone, Debug, Default)]
struct Node {
    /// The name as first written, for reports and for the frontends.
    display: String,
    /// Indices into the build's source units, in the order given.
    files: Vec<usize>,
    /// Normalised names this module instantiates, sorted and deduplicated.
    deps: Vec<String>,
}

/// Which modules a build holds, which files define them, and what they use.
///
/// Names are normalised (lower-cased for VHDL, kept as written otherwise)
/// so lookups match the language's rules; [`ModuleGraph::display_name`]
/// gives the spelling to show and to hand a frontend.
#[derive(Clone, Debug, Default)]
pub struct ModuleGraph {
    nodes: BTreeMap<String, Node>,
    globals: Vec<usize>,
}

impl ModuleGraph {
    /// An empty graph.
    pub fn new() -> ModuleGraph {
        ModuleGraph::default()
    }

    /// Adds one file's scan, under the language's name rules.
    pub fn add(&mut self, file: usize, language: Language, scan: &FileScan) {
        if scan.is_global() {
            self.globals.push(file);
            return;
        }
        for module in &scan.modules {
            let key = normalise(&module.name, language);
            let node = self.nodes.entry(key).or_insert_with(|| Node {
                display: module.name.clone(),
                ..Node::default()
            });
            if !node.files.contains(&file) {
                node.files.push(file);
            }
            for dep in &module.deps {
                let dep = normalise(dep, language);
                if !node.deps.contains(&dep) {
                    node.deps.push(dep);
                }
            }
            node.deps.sort_unstable();
        }
    }

    /// Every module, in ascending normalised-name order.
    pub fn modules(&self) -> Vec<&str> {
        self.nodes.keys().map(String::as_str).collect()
    }

    /// True when `name` is a module of this graph.
    pub fn contains(&self, name: &str) -> bool {
        self.nodes.contains_key(name)
    }

    /// The spelling to show for `name`, which is how it was first written.
    pub fn display_name(&self, name: &str) -> Option<&str> {
        self.nodes.get(name).map(|n| n.display.as_str())
    }

    /// The files that define `name`, in the order they were given.
    pub fn files_of(&self, name: &str) -> &[usize] {
        self.nodes.get(name).map_or(&[], |n| &n.files)
    }

    /// The names `name` instantiates, sorted; some may not be modules of
    /// this graph.
    pub fn deps_of(&self, name: &str) -> &[String] {
        self.nodes.get(name).map_or(&[], |n| &n.deps)
    }

    /// The files that define no module and so reach every elaboration.
    pub fn globals(&self) -> &[usize] {
        &self.globals
    }

    /// The modules nothing else in the graph instantiates, sorted.
    ///
    /// These are the roots of the hierarchy, and the candidates for a top
    /// when the build does not name one.
    pub fn roots(&self) -> Vec<&str> {
        let used: BTreeSet<&str> = self
            .nodes
            .values()
            .flat_map(|node| node.deps.iter().map(String::as_str))
            .collect();
        self.nodes
            .keys()
            .map(String::as_str)
            .filter(|name| !used.contains(name))
            .collect()
    }

    /// Every module, dependencies before dependents.
    ///
    /// Modules caught in a cycle come last, in name order, and are listed
    /// by [`ModuleGraph::cyclic`].
    pub fn topological(&self) -> Vec<String> {
        let mut out = self.settled();
        let placed: BTreeSet<&str> = out.iter().map(String::as_str).collect();
        let rest: Vec<String> = self
            .nodes
            .keys()
            .filter(|name| !placed.contains(name.as_str()))
            .cloned()
            .collect();
        out.extend(rest);
        out
    }

    /// The modules whose dependencies cannot be ordered, sorted.
    ///
    /// That is every module in an instantiation cycle, and every module
    /// that reaches one: neither can be given a key built from settled
    /// dependency keys. Neither language allows a cycle, so this is a
    /// malformed design; the build reports it and falls back to a
    /// conservative key over the whole source set (see [`mod@super::build`]).
    pub fn cyclic(&self) -> Vec<String> {
        let settled: BTreeSet<String> = self.settled().into_iter().collect();
        self.nodes
            .keys()
            .filter(|name| !settled.contains(*name))
            .cloned()
            .collect()
    }

    /// The modules that can be ordered, dependencies before dependents.
    ///
    /// Kahn's algorithm with a worklist kept in name order, so the result
    /// is the same on every run and every platform. A module left out is
    /// one in a cycle, or one that reaches a cycle.
    fn settled(&self) -> Vec<String> {
        // Count only the dependencies this graph actually defines; a name
        // nothing defines can never be placed and so cannot block anything.
        let mut waiting: BTreeMap<&str, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, node) in &self.nodes {
            let mut count = 0;
            for dep in &node.deps {
                if let Some((dep, _)) = self.nodes.get_key_value(dep) {
                    count += 1;
                    dependents.entry(dep.as_str()).or_default().push(name);
                }
            }
            waiting.insert(name.as_str(), count);
        }

        let mut ready: BTreeSet<&str> = waiting
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(name, _)| *name)
            .collect();
        let mut out = Vec::with_capacity(self.nodes.len());
        while let Some(name) = ready.iter().next().copied() {
            ready.remove(name);
            out.push(name.to_owned());
            for dependent in dependents.get(name).map(Vec::as_slice).unwrap_or(&[]) {
                let count = waiting.get_mut(dependent).expect("a known module");
                *count -= 1;
                if *count == 0 {
                    ready.insert(dependent);
                }
            }
        }
        out
    }
}

/// Normalises a name for lookup under a language's identifier rules.
pub fn normalise(name: &str, language: Language) -> String {
    match language {
        Language::Vhdl => name.to_ascii_lowercase(),
        _ => name.to_owned(),
    }
}

/// Scans one file of `map`, reporting parse problems into `diags`.
///
/// The file must already be in `map`; the build adds every source before
/// scanning so that one map covers the whole run.
pub fn scan_file(
    map: &mut SourceMap,
    id: SourceId,
    language: Language,
    diags: &mut Diagnostics,
) -> FileScan {
    match language {
        Language::Rtl => scan_rtl(map, id, diags),
        #[cfg(feature = "verilog")]
        Language::Verilog | Language::SystemVerilog => verilog::scan(map, id, language, diags),
        #[cfg(not(feature = "verilog"))]
        Language::Verilog | Language::SystemVerilog => FileScan::default(),
        #[cfg(feature = "vhdl")]
        Language::Vhdl => vhdl::scan(map, id, diags),
        #[cfg(not(feature = "vhdl"))]
        Language::Vhdl => FileScan::default(),
    }
}

/// Scans a design already in the `.rtl` text format.
fn scan_rtl(map: &SourceMap, id: SourceId, diags: &mut Diagnostics) -> FileScan {
    let text = map.file(id).text();
    let design = match crate::ir::Design::parse_text(text, id) {
        Ok(design) => design,
        Err(mut errors) => {
            diags.append(&mut errors);
            return FileScan::default();
        }
    };
    let mut scan = FileScan::default();
    for (_, module) in design.modules.iter() {
        let mut deps: Vec<String> = Vec::new();
        for (_, instance) in module.instances.iter() {
            let target = match &instance.module {
                crate::ir::ModuleRef::Resolved(id) => {
                    design.modules.get(*id).map(|m| m.name.as_str().to_owned())
                }
                crate::ir::ModuleRef::Unresolved(name) => Some(name.as_str().to_owned()),
            };
            if let Some(target) = target
                && !deps.contains(&target)
            {
                deps.push(target);
            }
        }
        deps.sort();
        scan.modules.push(ModuleScan {
            name: module.name.as_str().to_owned(),
            deps,
        });
    }
    scan
}

#[cfg(feature = "verilog")]
mod verilog {
    use super::{FileScan, Language, ModuleScan};
    use crate::diag::Diagnostics;
    use crate::source::{SourceId, SourceMap};
    use crate::verilog::ast::{Item, ItemKind};
    use crate::verilog::{Dialect, NoIncludes, parse_source};

    /// Parses one Verilog file and reads its definitions off the AST.
    pub(super) fn scan(
        map: &mut SourceMap,
        id: SourceId,
        language: Language,
        diags: &mut Diagnostics,
    ) -> FileScan {
        let dialect = match language {
            Language::SystemVerilog => Dialect::SystemVerilog,
            _ => Dialect::Verilog2005,
        };
        let file = parse_source(map, id, dialect, &mut NoIncludes, diags);
        let mut scan = FileScan::default();
        for item in &file.items {
            if let ItemKind::Module(module) = &item.kind {
                let mut deps = Vec::new();
                instances(&module.items, &mut deps);
                deps.sort();
                deps.dedup();
                scan.modules.push(ModuleScan {
                    name: module.name.name.clone(),
                    deps,
                });
            }
        }
        scan
    }

    /// Collects every instantiated name, following generate constructs.
    ///
    /// A generate block is not evaluated — that is elaboration's job — so
    /// every arm contributes, which can only add dependencies and so only
    /// over-invalidate.
    fn instances(items: &[Item], out: &mut Vec<String>) {
        for item in items {
            match &item.kind {
                ItemKind::Instance(inst) => out.push(inst.module.name.clone()),
                ItemKind::Generate(items) => instances(items, out),
                ItemKind::GenBlock(block) => instances(&block.items, out),
                ItemKind::GenFor(region) => instances(&region.body.items, out),
                ItemKind::GenIf(region) => {
                    instances(&region.then_block.items, out);
                    if let Some(block) = &region.else_block {
                        instances(&block.items, out);
                    }
                }
                ItemKind::GenCase(region) => {
                    for arm in &region.items {
                        instances(&arm.block.items, out);
                    }
                }
                // A nested module is a definition of its own; the outer
                // module does not depend on it unless it instantiates it,
                // which the `Instance` arm above catches.
                _ => {}
            }
        }
    }
}

#[cfg(feature = "vhdl")]
mod vhdl {
    use super::{FileScan, ModuleScan};
    use crate::diag::Diagnostics;
    use crate::source::{SourceId, SourceMap};
    use crate::vhdl::ast::{
        ConcurrentKind, ConcurrentStatement, Designator, InstantiatedUnit, LibraryUnit, Name,
        Suffix,
    };
    use crate::vhdl::{Standard, parse_source};

    /// Parses one VHDL file and reads its definitions off the AST.
    ///
    /// An entity is a module; an architecture contributes its
    /// instantiations to the entity it implements. Packages, package
    /// bodies, contexts and configurations define no module, so a file of
    /// them alone is a global.
    pub(super) fn scan(map: &mut SourceMap, id: SourceId, diags: &mut Diagnostics) -> FileScan {
        // The standard only changes which words are reserved; scanning
        // wants the more permissive one so a 2008 file in a 93 build still
        // yields its entity names.
        let file = parse_source(map, id, Standard::Vhdl2008, diags);
        let mut scan = FileScan::default();
        for unit in &file.units {
            match &unit.unit {
                LibraryUnit::Entity(entity) => {
                    let mut deps = Vec::new();
                    statements(&entity.statements, &mut deps);
                    add(&mut scan, &entity.name.name, deps);
                }
                LibraryUnit::Architecture(arch) => {
                    let Some(entity) = simple_name(&arch.entity) else {
                        continue;
                    };
                    let mut deps = Vec::new();
                    statements(&arch.statements, &mut deps);
                    add(&mut scan, &entity, deps);
                }
                _ => {}
            }
        }
        scan
    }

    /// Records `name` as defined here, merging with an earlier record of
    /// it (an entity and its architecture in one file).
    fn add(scan: &mut FileScan, name: &str, deps: Vec<String>) {
        let slot = scan
            .modules
            .iter_mut()
            .find(|m| m.name.eq_ignore_ascii_case(name));
        let module = match slot {
            Some(module) => module,
            None => {
                scan.modules.push(ModuleScan {
                    name: name.to_owned(),
                    deps: Vec::new(),
                });
                scan.modules.last_mut().expect("just pushed")
            }
        };
        module.deps.extend(deps);
        module.deps.sort();
        module.deps.dedup();
    }

    /// Collects every instantiated unit, following blocks and generates.
    fn statements(list: &[ConcurrentStatement], out: &mut Vec<String>) {
        for statement in list {
            match &statement.kind {
                ConcurrentKind::Instantiation(inst) => {
                    let name = match &inst.unit {
                        InstantiatedUnit::Component(name)
                        | InstantiatedUnit::Configuration(name)
                        | InstantiatedUnit::Entity { name, .. } => simple_name(name),
                    };
                    if let Some(name) = name {
                        out.push(name);
                    }
                }
                ConcurrentKind::Block(block) => statements(&block.statements, out),
                ConcurrentKind::ForGenerate(region) => {
                    statements(&region.body.statements, out);
                }
                ConcurrentKind::IfGenerate(region) => {
                    for arm in &region.arms {
                        statements(&arm.body.statements, out);
                    }
                    if let Some(body) = &region.else_arm {
                        statements(&body.statements, out);
                    }
                }
                ConcurrentKind::CaseGenerate(region) => {
                    for arm in &region.arms {
                        statements(&arm.body.statements, out);
                    }
                }
                _ => {}
            }
        }
    }

    /// The last simple identifier of a name, so `work.fifo` is `fifo`.
    fn simple_name(name: &Name) -> Option<String> {
        match name {
            Name::Simple(ident) => Some(ident.name.clone()),
            Name::Selected {
                suffix: Suffix::Designator(Designator::Ident(ident)),
                ..
            } => Some(ident.name.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-module design in the IR text format, which every build
    /// understands whatever frontends are compiled in.
    const RTL: &str = "top t\n\nmodule leaf\n  net %o u1 wire\n  port o out %o\n  \
                       assign %o = 1'b0\nend\n\nmodule t\n  net %o u1 wire\n  \
                       port o out %o\n  instance u of leaf (o=%o)\nend\n";

    fn scan_text(name: &str, text: &str) -> FileScan {
        let mut map = SourceMap::new();
        let id = map.add(name, text).unwrap();
        let mut diags = Diagnostics::new();
        let language = Language::from_path(name).expect("a known extension");
        let scan = scan_file(&mut map, id, language, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        scan
    }

    #[test]
    fn a_language_is_read_off_the_extension() {
        assert_eq!(Language::from_path("a.v"), Some(Language::Verilog));
        assert_eq!(Language::from_path("a.SV"), Some(Language::SystemVerilog));
        assert_eq!(Language::from_path("a.vhdl"), Some(Language::Vhdl));
        assert_eq!(Language::from_path("a.rtl"), Some(Language::Rtl));
        assert_eq!(Language::from_path("Makefile"), None);
        assert_eq!(Language::Vhdl.keyword(), "vhdl");
        assert_eq!(Language::SystemVerilog.frontend(), "verilog");
    }

    #[test]
    fn a_source_unit_takes_its_language_from_its_name() {
        let unit = SourceUnit::new("top.sv", "");
        assert_eq!(unit.language, Some(Language::SystemVerilog));
        assert_eq!(unit.language_or(Language::Vhdl), Language::SystemVerilog);
        let unit = SourceUnit::new("top.inc", "").with_language(Language::Verilog);
        assert_eq!(unit.language_or(Language::Vhdl), Language::Verilog);
        assert_eq!(
            SourceUnit::new("top.inc", "").language_or(Language::Vhdl),
            Language::Vhdl
        );
    }

    #[test]
    fn a_scan_round_trips_through_its_text_form() {
        let scan = FileScan {
            modules: vec![
                ModuleScan {
                    name: "top".to_owned(),
                    deps: vec!["a".to_owned(), "b".to_owned()],
                },
                ModuleScan {
                    name: "helper".to_owned(),
                    deps: Vec::new(),
                },
            ],
        };
        let text = scan.encode();
        assert_eq!(text, "defines top\nuses a\nuses b\ndefines helper\n");
        assert_eq!(FileScan::decode(&text), Some(scan));
        assert_eq!(FileScan::decode(""), Some(FileScan::default()));
        assert_eq!(FileScan::decode("nonsense\n"), None);
        assert_eq!(FileScan::decode("uses orphan\n"), None);
    }

    #[test]
    fn an_awkward_name_round_trips() {
        let scan = FileScan {
            modules: vec![ModuleScan {
                name: "\\odd\nname ".to_owned(),
                deps: vec!["back\\slash".to_owned()],
            }],
        };
        assert_eq!(FileScan::decode(&scan.encode()), Some(scan));
        assert_eq!(FileScan::decode("defines a\\q\n"), None);
        assert_eq!(FileScan::decode("defines a\\\n"), None);
    }

    #[test]
    fn a_graph_orders_dependencies_first() {
        let mut graph = ModuleGraph::new();
        graph.add(
            0,
            Language::Verilog,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "top".to_owned(),
                    deps: vec!["mid".to_owned()],
                }],
            },
        );
        graph.add(
            1,
            Language::Verilog,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "mid".to_owned(),
                    deps: vec!["leaf".to_owned()],
                }],
            },
        );
        graph.add(
            2,
            Language::Verilog,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "leaf".to_owned(),
                    deps: Vec::new(),
                }],
            },
        );
        assert_eq!(graph.topological(), vec!["leaf", "mid", "top"]);
        assert_eq!(graph.roots(), vec!["top"]);
        assert_eq!(graph.files_of("mid"), &[1]);
        assert_eq!(graph.deps_of("top"), &["mid".to_owned()]);
        assert!(graph.cyclic().is_empty());
        assert!(graph.globals().is_empty());
    }

    #[test]
    fn a_file_with_no_module_is_global() {
        let mut graph = ModuleGraph::new();
        graph.add(0, Language::Verilog, &FileScan::default());
        assert_eq!(graph.globals(), &[0]);
        assert!(graph.modules().is_empty());
    }

    #[test]
    fn a_cycle_is_reported_and_still_ordered() {
        let mut graph = ModuleGraph::new();
        for (i, (name, dep)) in [("a", "b"), ("b", "a"), ("c", "a")].iter().enumerate() {
            graph.add(
                i,
                Language::Verilog,
                &FileScan {
                    modules: vec![ModuleScan {
                        name: (*name).to_owned(),
                        deps: vec![(*dep).to_owned()],
                    }],
                },
            );
        }
        // `c` is not itself in the cycle, but it reaches one, so it too
        // has no well-founded key.
        assert_eq!(
            graph.cyclic(),
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
        let order = graph.topological();
        assert_eq!(order.len(), 3);
        assert!(order.contains(&"c".to_owned()));
    }

    #[test]
    fn a_missing_dependency_is_kept_as_a_name() {
        let mut graph = ModuleGraph::new();
        graph.add(
            0,
            Language::Verilog,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "top".to_owned(),
                    deps: vec!["vendor_prim".to_owned()],
                }],
            },
        );
        assert_eq!(graph.deps_of("top"), &["vendor_prim".to_owned()]);
        assert!(!graph.contains("vendor_prim"));
        assert_eq!(graph.topological(), vec!["top"]);
    }

    #[test]
    fn vhdl_names_match_case_insensitively() {
        let mut graph = ModuleGraph::new();
        graph.add(
            0,
            Language::Vhdl,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "Fifo".to_owned(),
                    deps: Vec::new(),
                }],
            },
        );
        graph.add(
            1,
            Language::Vhdl,
            &FileScan {
                modules: vec![ModuleScan {
                    name: "top".to_owned(),
                    deps: vec!["FIFO".to_owned()],
                }],
            },
        );
        assert!(graph.contains("fifo"));
        assert_eq!(graph.display_name("fifo"), Some("Fifo"));
        assert_eq!(graph.topological(), vec!["fifo", "top"]);
    }

    #[test]
    fn rtl_is_scanned_without_a_frontend() {
        let scan = scan_text("d.rtl", RTL);
        let names: Vec<&str> = scan.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["leaf", "t"]);
        assert!(scan.modules[0].deps.is_empty());
        assert_eq!(scan.modules[1].deps, vec!["leaf".to_owned()]);
    }

    #[cfg(feature = "verilog")]
    #[test]
    fn verilog_definitions_and_instances_are_found() {
        let scan = scan_text(
            "t.v",
            "module top(output o);\n  wire a, b;\n  mid u0(a);\n  \
             generate if (1) begin leaf u1(b); end else begin other u2(b); end endgenerate\n\
             endmodule\nmodule mid(output o); endmodule\n",
        );
        let names: Vec<&str> = scan.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["top", "mid"]);
        // Both arms of the generate count: the scan does not evaluate it.
        assert_eq!(
            scan.modules[0].deps,
            vec!["leaf".to_owned(), "mid".to_owned(), "other".to_owned()]
        );
    }

    #[cfg(feature = "verilog")]
    #[test]
    fn a_verilog_package_only_file_is_global() {
        let scan = scan_text("p.sv", "package p;\n  localparam W = 8;\nendpackage\n");
        assert!(scan.is_global());
    }

    #[cfg(feature = "vhdl")]
    #[test]
    fn vhdl_entities_and_architectures_are_found() {
        let scan = scan_text(
            "t.vhd",
            "entity top is port (o : out bit); end entity;\n\
             architecture rtl of top is begin\n  \
             u0 : entity work.leaf port map (o => o);\n  \
             g : for i in 0 to 1 generate u1 : mid port map (o => o); end generate;\n\
             end architecture;\n",
        );
        assert_eq!(scan.modules.len(), 1);
        assert_eq!(scan.modules[0].name, "top");
        assert_eq!(
            scan.modules[0].deps,
            vec!["leaf".to_owned(), "mid".to_owned()]
        );
    }

    #[cfg(feature = "vhdl")]
    #[test]
    fn a_vhdl_package_only_file_is_global() {
        let scan = scan_text(
            "p.vhd",
            "package p is\n  constant w : integer := 8;\nend package;\n",
        );
        assert!(scan.is_global());
    }
}
