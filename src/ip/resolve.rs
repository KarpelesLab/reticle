//! Dependency resolution and the lock file.
//!
//! A project names the IP it needs and a version requirement for each;
//! those packages name more. This module walks that graph, picks one
//! version of each package, loads every source it will need, and writes
//! down what it chose so the next build chooses the same.
//!
//! # Sans-I/O
//!
//! The library never opens a file. Everything the graph needs comes
//! through a [`SourceProvider`]:
//!
//! - [`SourceProvider::fetch`] turns a [`Dependency`] into a
//!   [`ResolvedIp`]: where the package came from, and the text of its
//!   `reticle.ip`.
//! - [`SourceProvider::read`] reads one file of a package the resolver
//!   has already fetched, once the manifest has said which files exist.
//! - [`SourceProvider::read_root`] does the same for the project's own
//!   sources.
//!
//! [`PathProvider`] is the one implementation shipped here: it lays
//! packages out as directories and asks a caller-supplied closure for
//! each file's contents. Even it does not touch `std::fs` — the CLI owns
//! the filesystem, passes in a closure that reads it, and is therefore
//! also the only place that has to think about permissions, symlinks and
//! path traversal. A WebAssembly playground hands in a closure over a
//! bundle instead and everything else works unchanged.
//!
//! # The rules
//!
//! 1. **Depth-first from the project.** Each `depends` line is followed,
//!    then each dependency's own `depends` lines, and so on.
//! 2. **A transitive dependency's *location* is the project's business.**
//!    An IP manifest says `depends fifo_sync ^1.0.0` and nothing more,
//!    so if the project has its own `depends fifo_sync ... path ../fifo`
//!    that is where `fifo_sync` comes from; otherwise the provider
//!    decides, and [`PathProvider`] looks for a directory named after
//!    the package. This is what keeps an IP package portable.
//! 3. **Highest version that satisfies everyone.** Every requirement on
//!    a package is collected, then the highest fetched version
//!    satisfying *all* of them is selected. When there is none the
//!    result is a [`ResolveError::Conflict`] that lists each requirement
//!    with the path through the graph that stated it.
//! 4. **A cycle is an error**, reported as the path that closes it.
//! 5. **Order is deterministic.** Packages come back with the leaves
//!    first and ties broken by name, so a build, a report and a lock
//!    file never depend on hash iteration order.
//!
//! # Reproducibility
//!
//! [`Resolver::resolve`] always produces a [`LockFile`]: the exact
//! version of every package, where each came from, and the requirement
//! edges that led there. It is written in the same line-oriented format
//! as the manifests, as `reticle.lock`, and [`LockFile::parse`] reads it
//! back. [`LockFile::differences`] says in words how an old lock file
//! and a new resolution disagree, which is what a `--locked` build
//! reports rather than silently moving.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::manifest::{
    DepSource, Dependency, IpManifest, Language, Project, SourceEntry, Version, VersionReq,
};
use super::text::{Line, tokenize};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, SourceMap, Span};

/// Diagnostic code for a dependency the provider could not fetch.
pub const NOT_FOUND: &str = "P0101";
/// Diagnostic code for requirements no available version satisfies.
pub const CONFLICT: &str = "P0102";
/// Diagnostic code for a cycle in the dependency graph.
pub const CYCLE: &str = "P0103";
/// Diagnostic code for a package whose manifest names a different package.
pub const NAME_MISMATCH: &str = "P0104";
/// Diagnostic code for a dependency source this build cannot follow.
pub const UNSUPPORTED_SOURCE: &str = "P0105";
/// Diagnostic code for a malformed line in a lock file.
pub const LOCK_SYNTAX: &str = "P0301";

/// How deep the graph may go before the resolver gives up.
///
/// A cycle is caught by the stack check, so this only fires on a graph
/// that is genuinely a thousand packages deep, which is a bug in the
/// packages rather than in Reticle.
const MAX_DEPTH: usize = 1000;

// ---------------------------------------------------------------------------
// The provider
// ---------------------------------------------------------------------------

/// A package as the provider found it: its origin and its manifest text.
///
/// The sources are not here. The resolver parses the manifest first — so
/// that every span points into the one [`SourceMap`] a diagnostic will be
/// rendered against — and only then asks [`SourceProvider::read`] for the
/// files the manifest names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedIp {
    /// Where the package came from, recorded in the lock file.
    pub origin: DepSource,
    /// An opaque location the provider can find the package's files
    /// again from; a directory, for [`PathProvider`].
    ///
    /// It is also used as the display prefix of the package's sources in
    /// the source map, so two packages that both ship `rtl/top.v` are
    /// told apart in a diagnostic.
    pub root: String,
    /// The display path of the manifest, used as the source file's name.
    pub manifest_path: String,
    /// The text of `reticle.ip`.
    pub manifest_text: String,
}

/// Where the resolver gets packages from.
///
/// Implementing [`fetch`](SourceProvider::fetch) is enough for a graph of
/// manifests; implement [`read`](SourceProvider::read) as well for the
/// sources to be loaded, and [`read_root`](SourceProvider::read_root) for
/// the project's own.
pub trait SourceProvider {
    /// Finds the package `dep` names.
    ///
    /// `dep.source` is the project's `depends` line when there was one;
    /// a transitive dependency the project says nothing about arrives
    /// with `source: None` and the provider decides where to look.
    fn fetch(&mut self, dep: &Dependency) -> Result<ResolvedIp, ResolveError>;

    /// Reads one file of a fetched package, relative to its root.
    ///
    /// `None` means "not available", which is how an encrypted or
    /// vendor-supplied file arrives; the package becomes a black box
    /// rather than an error. The default reads nothing, so a provider
    /// that only inspects the graph need not implement it.
    fn read(&mut self, package: &ResolvedIp, path: &str) -> Option<String> {
        let _ = (package, path);
        None
    }

    /// Reads one of the project's own source files.
    fn read_root(&mut self, project: &Project, path: &str) -> Option<String> {
        let _ = (project, path);
        None
    }
}

/// A [`SourceProvider`] over a directory layout, reading through a
/// closure.
///
/// Every dependency must be a `path` dependency; a `git` or `registry`
/// dependency is reported as [`ResolveError::Unsupported`], because
/// fetching one is network I/O and the library does none. A transitive
/// dependency the project does not place is looked for in a directory
/// named after the package, next to the project.
///
/// ```
/// use std::collections::BTreeMap;
/// use reticle::diag::Diagnostics;
/// use reticle::ip::{PathProvider, Project, Resolver};
/// use reticle::source::SourceMap;
///
/// let mut files = BTreeMap::new();
/// files.insert("ip/fifo/reticle.ip".to_owned(), "name fifo\nversion 1.0.0\n".to_owned());
/// let proj = "name blinky\ndepends fifo ^1.0.0 path ip/fifo\n";
///
/// let mut map = SourceMap::new();
/// let file = map.add("reticle.proj", proj).unwrap();
/// let mut diags = Diagnostics::new();
/// let project = Project::parse(proj, file, &mut diags).unwrap();
///
/// let mut provider = PathProvider::new(".", |path: &str| files.get(path).cloned());
/// let resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
/// assert!(resolved.is_complete());
/// assert_eq!(resolved.packages[0].manifest.name, "fifo");
/// ```
pub struct PathProvider<F> {
    root: String,
    read: F,
}

impl<F: FnMut(&str) -> Option<String>> PathProvider<F> {
    /// A provider rooted at `root`, reading files through `read`.
    ///
    /// `read` is given a path already joined to the root and normalised,
    /// so `.` and `..` have been resolved away before it sees anything.
    pub fn new(root: impl Into<String>, read: F) -> Self {
        PathProvider {
            root: root.into(),
            read,
        }
    }

    /// The directory a dependency lives in.
    fn directory(&self, dep: &Dependency) -> Result<String, ResolveError> {
        match &dep.source {
            Some(DepSource::Path(dir)) => Ok(join(&self.root, dir)),
            // Convention for a transitive dependency the project does not
            // place: a sibling directory named after the package.
            None => Ok(join(&self.root, &dep.name)),
            Some(other) => Err(ResolveError::Unsupported {
                name: dep.name.clone(),
                source: other.clone(),
                span: dep.span,
            }),
        }
    }
}

impl<F: FnMut(&str) -> Option<String>> SourceProvider for PathProvider<F> {
    fn fetch(&mut self, dep: &Dependency) -> Result<ResolvedIp, ResolveError> {
        let dir = self.directory(dep)?;
        let manifest_path = join(&dir, "reticle.ip");
        match (self.read)(&manifest_path) {
            Some(manifest_text) => Ok(ResolvedIp {
                origin: dep
                    .source
                    .clone()
                    .unwrap_or_else(|| DepSource::Path(dir.clone())),
                root: dir,
                manifest_path,
                manifest_text,
            }),
            None => Err(ResolveError::NotFound {
                name: dep.name.clone(),
                where_: format!("`{manifest_path}`"),
                span: dep.span,
            }),
        }
    }

    fn read(&mut self, package: &ResolvedIp, path: &str) -> Option<String> {
        (self.read)(&join(&package.root, path))
    }

    fn read_root(&mut self, _project: &Project, path: &str) -> Option<String> {
        (self.read)(&join(&self.root, path))
    }
}

/// Joins two path components and resolves `.` and `..` lexically.
///
/// Lexical is the right kind of resolution here: the library has no
/// filesystem to ask about symlinks, and a tidy path is what ends up in
/// the lock file and in diagnostics.
fn join(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return normalise(path);
    }
    let trimmed = base.trim_end_matches('/');
    if trimmed.is_empty() {
        // Either an empty base, or the root itself, whose slash must
        // survive the trim.
        return if base.starts_with('/') {
            normalise(&format!("/{path}"))
        } else {
            normalise(path)
        };
    }
    normalise(&format!("{trimmed}/{path}"))
}

fn normalise(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&last) if last != ".." => {
                    parts.pop();
                }
                _ => {
                    if !absolute {
                        parts.push("..");
                    }
                }
            },
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".to_owned(),
        (false, false) => joined,
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why resolution could not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The provider has no such package.
    NotFound {
        /// The package name.
        name: String,
        /// Where the provider looked, for the note.
        where_: String,
        /// The `depends` line that asked for it.
        span: Span,
    },
    /// The dependency names a source this build cannot follow.
    Unsupported {
        /// The package name.
        name: String,
        /// The source that was asked for.
        source: DepSource,
        /// The `depends` line.
        span: Span,
    },
    /// The package's manifest could not be parsed; the parser has
    /// already reported why.
    BadManifest {
        /// The package name as the `depends` line spells it.
        name: String,
        /// The `depends` line.
        span: Span,
    },
    /// The fetched manifest calls itself something else.
    NameMismatch {
        /// The name that was asked for.
        expected: String,
        /// The name the manifest declares.
        found: String,
        /// The `depends` line.
        span: Span,
    },
    /// No available version satisfies every requirement.
    Conflict {
        /// The package name.
        name: String,
        /// Each requirement, with the path through the graph that stated
        /// it, in the order they were found.
        requirements: Vec<Requirement>,
        /// The versions that were available, ascending.
        available: Vec<Version>,
    },
    /// The graph has a cycle.
    Cycle {
        /// The path that closes the cycle, first package repeated last.
        path: Vec<String>,
        /// The `depends` line that closes it.
        span: Span,
    },
    /// The graph is deeper than the resolver's depth limit.
    TooDeep {
        /// The package being visited when the limit was hit.
        name: String,
        /// The `depends` line.
        span: Span,
    },
}

/// One requirement on a package, with who stated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    /// The path through the graph, `blinky > uart_lite`.
    pub path: String,
    /// What that package asked for.
    pub req: VersionReq,
    /// Its `depends` line.
    pub span: Span,
}

impl ResolveError {
    /// The rustc-style diagnostic for this error.
    pub fn diagnostic(&self) -> Diagnostic {
        match self {
            ResolveError::NotFound { name, where_, span } => {
                Diagnostic::error(format!("cannot find the IP package `{name}`"))
                    .with_code(NOT_FOUND)
                    .with_span(*span)
                    .with_note(format!("looked for {where_}"))
            }
            ResolveError::Unsupported { name, source, span } => Diagnostic::error(format!(
                "cannot fetch `{name}` from {}",
                source.describe()
            ))
            .with_code(UNSUPPORTED_SOURCE)
            .with_span(*span)
            .with_note("this build resolves `path` dependencies only; vendor the package or use a checkout"),
            ResolveError::BadManifest { name, span } => {
                Diagnostic::error(format!("the manifest of `{name}` is not usable"))
                    .with_code(NOT_FOUND)
                    .with_span(*span)
                    .with_note("the errors above are in that manifest")
            }
            ResolveError::NameMismatch {
                expected,
                found,
                span,
            } => Diagnostic::error(format!(
                "expected the package `{expected}`, but its manifest is named `{found}`"
            ))
            .with_code(NAME_MISMATCH)
            .with_span(*span),
            ResolveError::Conflict {
                name,
                requirements,
                available,
            } => {
                let mut d = Diagnostic::error(format!(
                    "no version of `{name}` satisfies every requirement"
                ))
                .with_code(CONFLICT);
                for requirement in requirements {
                    d = d.with_label(
                        requirement.span,
                        format!("`{}` requires {}", requirement.path, requirement.req),
                    );
                }
                let versions = if available.is_empty() {
                    "none were available".to_owned()
                } else {
                    format!(
                        "available: {}",
                        available
                            .iter()
                            .map(Version::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                d.with_note(versions)
            }
            ResolveError::Cycle { path, span } => {
                Diagnostic::error(format!("`{}` depends on itself", path[0]))
                    .with_code(CYCLE)
                    .with_span(*span)
                    .with_note(format!("the cycle is {}", path.join(" > ")))
            }
            ResolveError::TooDeep { name, span } => Diagnostic::error(format!(
                "the dependency graph is more than {MAX_DEPTH} deep at `{name}`"
            ))
            .with_code(CYCLE)
            .with_span(*span),
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic().message)
    }
}

impl std::error::Error for ResolveError {}

// ---------------------------------------------------------------------------
// The result
// ---------------------------------------------------------------------------

/// One source file of a package or of the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSource {
    /// The path as the manifest wrote it.
    pub path: String,
    /// The language, from the `language` word or the extension.
    pub language: Option<Language>,
    /// The text, once it is in the resolution's [`SourceMap`]; `None`
    /// when the file is encrypted or the provider had nothing.
    pub file: Option<SourceId>,
    /// True when the manifest marked the file `encrypted`.
    pub encrypted: bool,
}

impl LoadedSource {
    /// True when the file's text is available to a frontend.
    pub fn is_readable(&self) -> bool {
        self.file.is_some()
    }
}

/// One selected package, with its manifest and its loaded sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The package's manifest.
    pub manifest: IpManifest,
    /// Where it came from.
    pub origin: DepSource,
    /// Its sources, in manifest order.
    pub sources: Vec<LoadedSource>,
    /// Its behavioural model, if it declared one.
    pub model: Option<LoadedSource>,
}

impl Package {
    /// The package's name.
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// The package's version.
    pub fn version(&self) -> &Version {
        &self.manifest.version
    }

    /// True when nothing of the package can be read: it must be a black
    /// box.
    pub fn is_blackbox(&self) -> bool {
        self.manifest.is_blackbox() || !self.sources.iter().any(LoadedSource::is_readable)
    }
}

/// Everything one resolution produced.
///
/// The [`SourceMap`] inside holds the text of every manifest and every
/// source that was read, which is what makes [`super::elaborate_project`]
/// possible without any further I/O and what a caller renders
/// diagnostics against.
#[derive(Debug)]
pub struct Resolved {
    map: SourceMap,
    /// The project's own sources, in manifest order.
    pub root: Vec<LoadedSource>,
    /// The selected packages, dependencies before dependents.
    pub packages: Vec<Package>,
    /// The lock file this resolution would write.
    pub lock: LockFile,
    /// Everything that went wrong; empty on a clean resolution.
    pub errors: Vec<ResolveError>,
}

impl Resolved {
    /// The map holding every text this resolution read.
    pub fn source_map(&self) -> &SourceMap {
        &self.map
    }

    /// The same map, for a frontend that adds to it.
    ///
    /// The Verilog preprocessor puts the files it includes into the map
    /// it is given, and the VHDL analyser does the same for the bundled
    /// `std` and `ieee` sources, so elaboration needs write access to
    /// the one map the whole build's spans point into.
    pub fn source_map_mut(&mut self) -> &mut SourceMap {
        &mut self.map
    }

    /// The text of a loaded source, if it was readable.
    pub fn text(&self, source: &LoadedSource) -> Option<&str> {
        Some(self.map.file(source.file?).text())
    }

    /// The display name of a loaded source in the map.
    pub fn name(&self, source: &LoadedSource) -> Option<&str> {
        Some(self.map.file(source.file?).name())
    }

    /// True when nothing went wrong.
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty()
    }

    /// The selected package with the given name.
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name() == name)
    }
}

// ---------------------------------------------------------------------------
// The lock file
// ---------------------------------------------------------------------------

/// One line of a lock file: a package pinned to a version and an origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockedPackage {
    /// The package name.
    pub name: String,
    /// The version that was selected.
    pub version: Version,
    /// Where it came from.
    pub origin: DepSource,
}

/// `reticle.lock`: what a resolution chose, so the next one chooses the
/// same.
///
/// The format is the manifests' format:
///
/// ```text
/// version 1
/// package cdc_sync 0.3.1 path ../ip/cdc_sync
/// package fifo_sync 1.0.4 path ../ip/fifo_sync
/// requires fifo_sync cdc_sync ^0.3.0
/// ```
///
/// `package` lines are sorted by name and `requires` lines by dependent
/// then dependency, so the file only changes when the resolution does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockFile {
    /// The format version; `1` today.
    pub version: u32,
    /// The pinned packages, sorted by name.
    pub packages: Vec<LockedPackage>,
    /// The requirement edges, as `(dependent, dependency, requirement)`.
    /// The dependent is the project's name for a top-level dependency.
    pub requires: Vec<(String, String, VersionReq)>,
}

/// The two comment lines every lock file starts with.
const LOCK_HEADER: &str = "\
# reticle.lock: the exact IP versions this project resolved to.
# Generated by Reticle. Edit reticle.proj and resolve again.
";

impl LockFile {
    /// An empty lock file of the current format version.
    pub fn new() -> Self {
        LockFile {
            version: 1,
            packages: Vec::new(),
            requires: Vec::new(),
        }
    }

    /// The entry for a package.
    pub fn package(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Renders the lock file.
    pub fn to_text(&self) -> String {
        let mut out = String::from(LOCK_HEADER);
        out.push_str(&format!("version {}\n", self.version));
        if !self.packages.is_empty() {
            out.push('\n');
        }
        for package in &self.packages {
            out.push_str(&format!(
                "package {} {} {}\n",
                super::text::quote(&package.name),
                package.version,
                package.origin
            ));
        }
        if !self.requires.is_empty() {
            out.push('\n');
        }
        for (from, to, req) in &self.requires {
            out.push_str(&format!(
                "requires {} {} {req}\n",
                super::text::quote(from),
                super::text::quote(to)
            ));
        }
        out
    }

    /// Parses a lock file.
    ///
    /// Bad lines are reported and skipped; the result is whatever was
    /// readable, so a truncated lock file still says what it can.
    ///
    /// ```
    /// use reticle::diag::Diagnostics;
    /// use reticle::ip::LockFile;
    /// use reticle::source::SourceMap;
    ///
    /// let text = "version 1\n\npackage fifo 1.0.0 path ../fifo\n";
    /// let mut map = SourceMap::new();
    /// let file = map.add("reticle.lock", text).unwrap();
    /// let mut diags = Diagnostics::new();
    /// let lock = LockFile::parse(text, file, &mut diags);
    /// assert!(!diags.has_errors());
    /// assert_eq!(lock.package("fifo").unwrap().version.to_string(), "1.0.0");
    /// ```
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> LockFile {
        let mut out = LockFile::new();
        for line in tokenize(text, file) {
            match line.keyword() {
                "version" => match line.args() {
                    [v] => match v.as_str().parse::<u32>() {
                        Ok(v) => out.version = v,
                        Err(_) => lock_error(diags, &line, "`version <number>`"),
                    },
                    _ => lock_error(diags, &line, "`version <number>`"),
                },
                "package" => {
                    let args = line.args();
                    if args.len() < 3 {
                        lock_error(diags, &line, "`package <name> <version> <origin>`");
                        continue;
                    }
                    let Some(version) = Version::parse(args[1].as_str()) else {
                        lock_error(diags, &line, "a `major.minor.patch` version");
                        continue;
                    };
                    let Some(origin) = parse_origin(&args[2..]) else {
                        lock_error(
                            diags,
                            &line,
                            "an origin of `path <dir>`, `git <url> [rev <r>]` or `registry`",
                        );
                        continue;
                    };
                    out.packages.push(LockedPackage {
                        name: args[0].as_str().to_owned(),
                        version,
                        origin,
                    });
                }
                "requires" => match line.args() {
                    [from, to, req] => match VersionReq::parse(req.as_str()) {
                        Some(req) => out.requires.push((
                            from.as_str().to_owned(),
                            to.as_str().to_owned(),
                            req,
                        )),
                        None => lock_error(diags, &line, "a version requirement as its last word"),
                    },
                    _ => lock_error(diags, &line, "`requires <dependent> <dependency> <req>`"),
                },
                other => diags.push(
                    Diagnostic::error(format!("unknown lock file directive `{other}`"))
                        .with_code(LOCK_SYNTAX)
                        .with_span(line.keyword_span())
                        .with_note("a lock file has `version`, `package` and `requires` lines"),
                ),
            }
        }
        out
    }

    /// How `self` (an old lock file) and `new` (a fresh resolution)
    /// disagree, in words, sorted and deterministic.
    ///
    /// An empty result means the build is reproducible.
    pub fn differences(&self, new: &LockFile) -> Vec<String> {
        let mut out = Vec::new();
        let old_names: BTreeSet<&str> = self.packages.iter().map(|p| p.name.as_str()).collect();
        let new_names: BTreeSet<&str> = new.packages.iter().map(|p| p.name.as_str()).collect();
        for name in old_names.union(&new_names) {
            match (self.package(name), new.package(name)) {
                (Some(old), Some(new)) => {
                    if old.version != new.version {
                        out.push(format!(
                            "`{name}` moves from {} to {}",
                            old.version, new.version
                        ));
                    }
                    if old.origin != new.origin {
                        out.push(format!(
                            "`{name}` moves from {} to {}",
                            old.origin.describe(),
                            new.origin.describe()
                        ));
                    }
                }
                (Some(old), None) => {
                    out.push(format!("`{name}` {} is no longer used", old.version))
                }
                (None, Some(new)) => out.push(format!("`{name}` {} is new", new.version)),
                (None, None) => unreachable!("the name came from one of the two"),
            }
        }
        out
    }
}

fn lock_error(diags: &mut Diagnostics, line: &Line, expected: &str) {
    diags.push(
        Diagnostic::error(format!("`{}` takes {expected}", line.keyword()))
            .with_code(LOCK_SYNTAX)
            .with_span(line.span),
    );
}

fn parse_origin(words: &[super::text::Token]) -> Option<DepSource> {
    match words {
        [w] if w.is("registry") => Some(DepSource::Registry),
        [w, dir] if w.is("path") => Some(DepSource::Path(dir.as_str().to_owned())),
        [w, url] if w.is("git") => Some(DepSource::Git {
            url: url.as_str().to_owned(),
            rev: None,
        }),
        [w, url, r, rev] if w.is("git") && r.is("rev") => Some(DepSource::Git {
            url: url.as_str().to_owned(),
            rev: Some(rev.as_str().to_owned()),
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The resolver
// ---------------------------------------------------------------------------

/// One fetched candidate: a manifest at a version, from somewhere.
struct Candidate {
    manifest: IpManifest,
    ip: ResolvedIp,
}

/// Resolves a project's dependency graph.
///
/// The resolver owns the [`SourceMap`] for the whole build: the caller
/// parses `reticle.proj` into a map, hands that map over, and gets it
/// back inside the [`Resolved`], holding every manifest and source that
/// was read as well. One map means one set of spans, which means
/// diagnostics from the project file, from a dependency's manifest and
/// from elaboration all render together.
pub struct Resolver {
    map: SourceMap,
}

impl Resolver {
    /// A resolver over `map`, which already holds the project manifest.
    pub fn new(map: SourceMap) -> Self {
        Resolver { map }
    }

    /// Walks the graph, selects versions and loads sources.
    ///
    /// Every problem is recorded in [`Resolved::errors`] *and* pushed as
    /// a diagnostic into `diags`; resolution continues past one so a
    /// user sees every missing package at once rather than one per run.
    pub fn resolve(
        mut self,
        project: &Project,
        provider: &mut dyn SourceProvider,
        diags: &mut Diagnostics,
    ) -> Resolved {
        let mut walk = Walk {
            map: &mut self.map,
            provider,
            diags,
            project,
            candidates: BTreeMap::new(),
            requirements: BTreeMap::new(),
            errors: Vec::new(),
            fetched: BTreeSet::new(),
        };
        walk.visit_all(&project.depends, &mut vec![project.name.clone()]);

        let selected = walk.select();
        let errors = std::mem::take(&mut walk.errors);
        let mut candidates = std::mem::take(&mut walk.candidates);
        let requirements = std::mem::take(&mut walk.requirements);

        // Load the project's own sources, then each selected package's,
        // in dependency order.
        let root = load_sources(&mut self.map, "", &project.sources, &mut |path| {
            provider.read_root(project, path)
        });
        let order = dependency_order(&selected, &candidates, project);
        let mut packages = Vec::new();
        for name in &order {
            let Some(version) = selected.get(name) else {
                continue;
            };
            let Some(candidate) = candidates
                .get_mut(name)
                .and_then(|by_version| by_version.remove(version))
            else {
                continue;
            };
            let ip = candidate.ip;
            let manifest = candidate.manifest;
            let sources = load_sources(&mut self.map, &ip.root, &manifest.sources, &mut |path| {
                provider.read(&ip, path)
            });
            let model = manifest.model.as_ref().and_then(|entry| {
                load_sources(
                    &mut self.map,
                    &ip.root,
                    std::slice::from_ref(entry),
                    &mut |path| provider.read(&ip, path),
                )
                .pop()
            });
            packages.push(Package {
                manifest,
                origin: ip.origin,
                sources,
                model,
            });
        }

        let lock = build_lock(&packages, &requirements, project);
        Resolved {
            map: self.map,
            root,
            packages,
            lock,
            errors,
        }
    }
}

/// Reads the text of each entry into `map`, leaving encrypted and
/// unavailable files with no [`SourceId`].
fn load_sources(
    map: &mut SourceMap,
    prefix: &str,
    entries: &[SourceEntry],
    read: &mut dyn FnMut(&str) -> Option<String>,
) -> Vec<LoadedSource> {
    let mut out = Vec::new();
    for entry in entries {
        // The name in the map is the package's root plus the manifest's
        // own path, so two packages that both ship `rtl/top.v` are told
        // apart in a diagnostic.
        let name = if prefix.is_empty() {
            entry.path.clone()
        } else {
            format!("{prefix}/{}", entry.path)
        };
        let file = if entry.encrypted {
            None
        } else {
            read(&entry.path).and_then(|text| map.add(name, text).ok())
        };
        out.push(LoadedSource {
            path: entry.path.clone(),
            language: entry.language(),
            file,
            encrypted: entry.encrypted,
        });
    }
    out
}

/// The depth-first walk over the graph.
struct Walk<'a> {
    map: &'a mut SourceMap,
    provider: &'a mut dyn SourceProvider,
    diags: &'a mut Diagnostics,
    project: &'a Project,
    /// Every fetched manifest, by name then version.
    candidates: BTreeMap<String, BTreeMap<Version, Candidate>>,
    /// Every requirement stated on a package, in discovery order.
    requirements: BTreeMap<String, Vec<Requirement>>,
    errors: Vec<ResolveError>,
    /// The `(name, source)` pairs already fetched, so one package
    /// reached twice is fetched once.
    fetched: BTreeSet<(String, String)>,
}

impl Walk<'_> {
    fn fail(&mut self, error: ResolveError) {
        self.diags.push(error.diagnostic());
        self.errors.push(error);
    }

    fn visit_all(&mut self, deps: &[Dependency], stack: &mut Vec<String>) {
        for dep in deps {
            self.visit(dep, stack);
        }
    }

    fn visit(&mut self, dep: &Dependency, stack: &mut Vec<String>) {
        self.requirements
            .entry(dep.name.clone())
            .or_default()
            .push(Requirement {
                path: stack.join(" > "),
                req: dep.req.clone(),
                span: dep.span,
            });

        if let Some(at) = stack.iter().position(|n| *n == dep.name) {
            let mut path = stack[at..].to_vec();
            path.push(dep.name.clone());
            self.fail(ResolveError::Cycle {
                path,
                span: dep.span,
            });
            return;
        }
        if stack.len() > MAX_DEPTH {
            self.fail(ResolveError::TooDeep {
                name: dep.name.clone(),
                span: dep.span,
            });
            return;
        }

        // A transitive dependency takes its location from the project's
        // own `depends` line when there is one, which is rule 2 in the
        // module documentation.
        let placed = match dep.source {
            Some(_) => dep.clone(),
            None => match self.project.dependency(&dep.name) {
                Some(from_project) => Dependency {
                    source: from_project.source.clone(),
                    ..dep.clone()
                },
                None => dep.clone(),
            },
        };
        let key = (
            placed.name.clone(),
            placed
                .source
                .as_ref()
                .map_or_else(String::new, DepSource::to_string),
        );
        if !self.fetched.insert(key) {
            // Already fetched from this exact place; its own
            // dependencies were walked then. The requirement recorded
            // above still counts towards selection.
            return;
        }

        let ip = match self.provider.fetch(&placed) {
            Ok(ip) => ip,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let Ok(file) = self
            .map
            .add(ip.manifest_path.clone(), ip.manifest_text.clone())
        else {
            self.fail(ResolveError::NotFound {
                name: dep.name.clone(),
                where_: format!("`{}`, which is too large to read", ip.manifest_path),
                span: dep.span,
            });
            return;
        };
        let text = self.map.file(file).text().to_owned();
        let Some(manifest) = IpManifest::parse(&text, file, self.diags) else {
            self.fail(ResolveError::BadManifest {
                name: dep.name.clone(),
                span: dep.span,
            });
            return;
        };
        if manifest.name != dep.name {
            self.fail(ResolveError::NameMismatch {
                expected: dep.name.clone(),
                found: manifest.name.clone(),
                span: dep.span,
            });
            return;
        }

        let deps = manifest.depends.clone();
        let version = manifest.version.clone();
        self.candidates
            .entry(manifest.name.clone())
            .or_default()
            .insert(version, Candidate { manifest, ip });

        stack.push(dep.name.clone());
        self.visit_all(&deps, stack);
        stack.pop();
    }

    /// Picks the highest fetched version of each package that satisfies
    /// every requirement recorded on it.
    fn select(&mut self) -> BTreeMap<String, Version> {
        let mut out = BTreeMap::new();
        let names: Vec<String> = self.candidates.keys().cloned().collect();
        for name in names {
            let reqs = self.requirements.get(&name).cloned().unwrap_or_default();
            let available: Vec<Version> = self.candidates[&name].keys().cloned().collect();
            let chosen = available
                .iter()
                .rev()
                .find(|v| reqs.iter().all(|r| r.req.matches(v)))
                .cloned();
            match chosen {
                Some(version) => {
                    out.insert(name, version);
                }
                None => self.fail(ResolveError::Conflict {
                    name,
                    requirements: reqs,
                    available,
                }),
            }
        }
        out
    }
}

/// Orders the selected packages with dependencies before dependents.
///
/// A depth-first post-order over the graph, entered in the project's
/// `depends` order and then alphabetically, so the result is stable.
fn dependency_order(
    selected: &BTreeMap<String, Version>,
    candidates: &BTreeMap<String, BTreeMap<Version, Candidate>>,
    project: &Project,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut done = BTreeSet::new();
    let mut roots: Vec<String> = project.depends.iter().map(|d| d.name.clone()).collect();
    roots.extend(selected.keys().cloned());
    let mut stack_guard = BTreeSet::new();
    for root in roots {
        visit_order(
            &root,
            selected,
            candidates,
            &mut done,
            &mut stack_guard,
            &mut out,
        );
    }
    out
}

fn visit_order(
    name: &str,
    selected: &BTreeMap<String, Version>,
    candidates: &BTreeMap<String, BTreeMap<Version, Candidate>>,
    done: &mut BTreeSet<String>,
    on_stack: &mut BTreeSet<String>,
    out: &mut Vec<String>,
) {
    if done.contains(name) || !on_stack.insert(name.to_owned()) {
        return;
    }
    if let Some(version) = selected.get(name)
        && let Some(candidate) = candidates.get(name).and_then(|m| m.get(version))
    {
        let mut children: Vec<String> = candidate
            .manifest
            .depends
            .iter()
            .map(|d| d.name.clone())
            .collect();
        children.sort();
        for child in children {
            visit_order(&child, selected, candidates, done, on_stack, out);
        }
    }
    on_stack.remove(name);
    if done.insert(name.to_owned()) && selected.contains_key(name) {
        out.push(name.to_owned());
    }
}

fn build_lock(
    packages: &[Package],
    requirements: &BTreeMap<String, Vec<Requirement>>,
    project: &Project,
) -> LockFile {
    let mut lock = LockFile::new();
    let mut sorted: Vec<&Package> = packages.iter().collect();
    sorted.sort_by(|a, b| a.name().cmp(b.name()));
    for package in sorted {
        lock.packages.push(LockedPackage {
            name: package.name().to_owned(),
            version: package.version().clone(),
            origin: package.origin.clone(),
        });
    }
    let selected: BTreeSet<&str> = packages.iter().map(Package::name).collect();
    let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();
    for dep in &project.depends {
        if selected.contains(dep.name.as_str()) {
            edges.insert((project.name.clone(), dep.name.clone(), dep.req.to_string()));
        }
    }
    for package in packages {
        for dep in &package.manifest.depends {
            if selected.contains(dep.name.as_str()) {
                edges.insert((
                    package.name().to_owned(),
                    dep.name.clone(),
                    dep.req.to_string(),
                ));
            }
        }
    }
    // Anything required but never selected is a failed resolution; the
    // errors say so, and the lock file stays silent about it.
    let _ = requirements;
    for (from, to, req) in edges {
        if let Some(req) = VersionReq::parse(&req) {
            lock.requires.push((from, to, req));
        }
    }
    lock
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    /// A tiny in-memory filesystem, which is all a `PathProvider` needs.
    fn files(entries: &[(&str, &str)]) -> Map<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn run(project_text: &str, fs: Map<String, String>) -> (Resolved, String) {
        let mut map = SourceMap::new();
        let file = map.add("reticle.proj", project_text).unwrap();
        let mut diags = Diagnostics::new();
        let project = Project::parse(project_text, file, &mut diags).expect("project parses");
        let mut provider = PathProvider::new(".", |path: &str| fs.get(path).cloned());
        let resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
        let rendered = diags.render(resolved.source_map());
        (resolved, rendered)
    }

    #[test]
    fn paths_join_and_normalise() {
        assert_eq!(join(".", "ip/fifo"), "ip/fifo");
        assert_eq!(join("ip/fifo", "../cdc"), "ip/cdc");
        assert_eq!(join("ip/fifo", "rtl/f.v"), "ip/fifo/rtl/f.v");
        assert_eq!(join("/a/b", "../c"), "/a/c");
        assert_eq!(join("a", "/abs"), "/abs");
        assert_eq!(join("a/b", "../../.."), "..");
        assert_eq!(join(".", "."), ".");
        assert_eq!(join("/", ".."), "/");
        assert_eq!(join("a", "b//c/./d"), "a/b/c/d");
    }

    #[test]
    fn a_simple_graph_resolves_leaves_first() {
        let fs = files(&[
            (
                "ip/uart/reticle.ip",
                "name uart\nversion 1.2.0\n\nsource rtl/uart.v\n\ndepends fifo ^1.0.0\n",
            ),
            ("ip/uart/rtl/uart.v", "module uart; endmodule\n"),
            (
                "ip/fifo/reticle.ip",
                "name fifo\nversion 1.0.4\n\nsource rtl/fifo.v\n",
            ),
            ("ip/fifo/rtl/fifo.v", "module fifo; endmodule\n"),
            ("rtl/top.v", "module top; endmodule\n"),
        ]);
        let (resolved, rendered) = run(
            "name blinky\ntop top\n\nsource rtl/top.v\n\ndepends uart ^1.2.0 path ip/uart\ndepends fifo ^1.0.0 path ip/fifo\n",
            fs,
        );
        assert_eq!(rendered, "");
        assert!(resolved.is_complete());
        let names: Vec<&str> = resolved.packages.iter().map(Package::name).collect();
        assert_eq!(names, ["fifo", "uart"]);
        assert_eq!(resolved.root.len(), 1);
        assert_eq!(
            resolved.text(&resolved.root[0]),
            Some("module top; endmodule\n")
        );
        assert!(resolved.package("uart").unwrap().sources[0].is_readable());
        assert!(!resolved.package("uart").unwrap().is_blackbox());
        assert_eq!(
            resolved.lock.to_text(),
            format!(
                "{LOCK_HEADER}version 1\n\npackage fifo 1.0.4 path ip/fifo\npackage uart 1.2.0 path ip/uart\n\nrequires blinky fifo ^1.0.0\nrequires blinky uart ^1.2.0\nrequires uart fifo ^1.0.0\n"
            )
        );
    }

    #[test]
    fn a_transitive_dependency_is_placed_by_the_project() {
        // `uart` asks for `fifo` without saying where; the project's own
        // line places it.
        let fs = files(&[
            (
                "ip/uart/reticle.ip",
                "name uart\nversion 1.0.0\n\ndepends fifo ^1.0.0\n",
            ),
            ("vendor/f/reticle.ip", "name fifo\nversion 1.3.0\n"),
        ]);
        let (resolved, rendered) = run(
            "name p\n\ndepends uart ^1.0.0 path ip/uart\ndepends fifo ^1.0.0 path vendor/f\n",
            fs,
        );
        assert_eq!(rendered, "");
        assert_eq!(
            resolved.package("fifo").unwrap().origin,
            DepSource::Path("vendor/f".to_owned())
        );
    }

    #[test]
    fn an_unplaced_transitive_dependency_falls_back_to_a_sibling() {
        let fs = files(&[
            (
                "ip/uart/reticle.ip",
                "name uart\nversion 1.0.0\n\ndepends fifo ^1.0.0\n",
            ),
            ("fifo/reticle.ip", "name fifo\nversion 1.0.0\n"),
        ]);
        let (resolved, rendered) = run("name p\n\ndepends uart ^1.0.0 path ip/uart\n", fs);
        assert_eq!(rendered, "");
        assert!(resolved.package("fifo").is_some());
    }

    #[test]
    fn the_highest_satisfying_version_wins() {
        // Two places offer `fifo`; only the older satisfies both edges.
        let fs = files(&[
            (
                "ip/a/reticle.ip",
                "name a\nversion 1.0.0\n\ndepends fifo 1.0.0\n",
            ),
            ("old/reticle.ip", "name fifo\nversion 1.0.0\n"),
            ("new/reticle.ip", "name fifo\nversion 1.5.0\n"),
        ]);
        let (resolved, rendered) = run(
            "name p\n\ndepends a ^1.0.0 path ip/a\ndepends fifo * path new\ndepends fifo * path old\n",
            fs,
        );
        assert_eq!(rendered, "");
        assert_eq!(
            resolved.package("fifo").unwrap().version(),
            &Version::new(1, 0, 0)
        );
    }

    #[test]
    fn a_version_conflict_names_the_path_through_the_graph() {
        let fs = files(&[
            (
                "ip/a/reticle.ip",
                "name a\nversion 1.0.0\n\ndepends fifo ^2.0.0\n",
            ),
            ("ip/fifo/reticle.ip", "name fifo\nversion 1.0.0\n"),
        ]);
        let (resolved, rendered) = run(
            "name p\n\ndepends a ^1.0.0 path ip/a\ndepends fifo ^1.0.0 path ip/fifo\n",
            fs,
        );
        assert!(!resolved.is_complete());
        assert!(
            rendered.contains("error[P0102]: no version of `fifo` satisfies every requirement"),
            "{rendered}"
        );
        assert!(rendered.contains("`p` requires ^1.0.0"), "{rendered}");
        assert!(rendered.contains("`p > a` requires ^2.0.0"), "{rendered}");
        assert!(rendered.contains("available: 1.0.0"), "{rendered}");
        assert!(resolved.package("fifo").is_none());
    }

    #[test]
    fn a_cycle_is_reported_as_the_path_that_closes_it() {
        let fs = files(&[
            (
                "ip/a/reticle.ip",
                "name a\nversion 1.0.0\n\ndepends b ^1.0.0\n",
            ),
            (
                "ip/b/reticle.ip",
                "name b\nversion 1.0.0\n\ndepends a ^1.0.0\n",
            ),
        ]);
        let (resolved, rendered) = run(
            "name p\n\ndepends a ^1.0.0 path ip/a\ndepends b ^1.0.0 path ip/b\n",
            fs,
        );
        assert!(!resolved.is_complete());
        assert!(
            rendered.contains("error[P0103]: `a` depends on itself"),
            "{rendered}"
        );
        assert!(rendered.contains("the cycle is a > b > a"), "{rendered}");
    }

    #[test]
    fn a_self_dependency_is_a_cycle_too() {
        let fs = files(&[(
            "ip/a/reticle.ip",
            "name a\nversion 1.0.0\n\ndepends a ^1.0.0\n",
        )]);
        let (_, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", fs);
        assert!(rendered.contains("the cycle is a > a"), "{rendered}");
    }

    #[test]
    fn a_missing_package_says_where_it_looked() {
        let (resolved, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", files(&[]));
        assert!(!resolved.is_complete());
        assert!(
            rendered.contains("error[P0101]: cannot find the IP package `a`"),
            "{rendered}"
        );
        assert!(
            rendered.contains("looked for `ip/a/reticle.ip`"),
            "{rendered}"
        );
    }

    #[test]
    fn git_and_registry_dependencies_are_declined_clearly() {
        let (_, rendered) = run(
            "name p\n\ndepends a ^1.0.0 git https://x.invalid/a.git\n",
            files(&[]),
        );
        assert!(
            rendered.contains(
                "error[P0105]: cannot fetch `a` from the repository `https://x.invalid/a.git`"
            ),
            "{rendered}"
        );
        let (_, rendered) = run("name p\n\ndepends a ^1.0.0 registry\n", files(&[]));
        assert!(
            rendered.contains("cannot fetch `a` from the registry"),
            "{rendered}"
        );
    }

    #[test]
    fn a_manifest_under_the_wrong_name_is_reported() {
        let fs = files(&[("ip/a/reticle.ip", "name b\nversion 1.0.0\n")]);
        let (_, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", fs);
        assert!(
            rendered
                .contains("error[P0104]: expected the package `a`, but its manifest is named `b`"),
            "{rendered}"
        );
    }

    #[test]
    fn a_broken_manifest_reports_both_the_cause_and_the_effect() {
        let fs = files(&[("ip/a/reticle.ip", "version 1.0.0\n")]);
        let (_, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", fs);
        assert!(
            rendered.contains("an IP manifest has no `name`"),
            "{rendered}"
        );
        assert!(
            rendered.contains("the manifest of `a` is not usable"),
            "{rendered}"
        );
    }

    #[test]
    fn encrypted_sources_load_as_black_boxes() {
        let fs = files(&[(
            "ip/a/reticle.ip",
            "name a\nversion 1.0.0\n\nsource rtl/a.vp language verilog encrypted\n",
        )]);
        let (resolved, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", fs);
        assert_eq!(rendered, "");
        let package = resolved.package("a").unwrap();
        assert!(package.is_blackbox());
        assert!(!package.sources[0].is_readable());
        assert!(package.sources[0].encrypted);
        assert_eq!(package.sources[0].language, Some(Language::Verilog));
        assert_eq!(resolved.text(&package.sources[0]), None);
        assert_eq!(resolved.name(&package.sources[0]), None);
    }

    #[test]
    fn a_source_the_provider_cannot_read_is_not_fatal() {
        let fs = files(&[(
            "ip/a/reticle.ip",
            "name a\nversion 1.0.0\n\nsource rtl/a.v\n",
        )]);
        let (resolved, rendered) = run("name p\n\ndepends a ^1.0.0 path ip/a\n", fs);
        assert_eq!(rendered, "");
        assert!(resolved.is_complete());
        assert!(resolved.package("a").unwrap().is_blackbox());
    }

    #[test]
    fn a_diamond_fetches_each_package_once() {
        let fs = files(&[
            (
                "ip/a/reticle.ip",
                "name a\nversion 1.0.0\n\ndepends d ^1.0.0\n",
            ),
            (
                "ip/b/reticle.ip",
                "name b\nversion 1.0.0\n\ndepends d ^1.0.0\n",
            ),
            ("d/reticle.ip", "name d\nversion 1.1.0\n"),
        ]);
        let (resolved, rendered) = run(
            "name p\n\ndepends a ^1.0.0 path ip/a\ndepends b ^1.0.0 path ip/b\n",
            fs,
        );
        assert_eq!(rendered, "");
        let names: Vec<&str> = resolved.packages.iter().map(Package::name).collect();
        assert_eq!(names, ["d", "a", "b"]);
        assert_eq!(resolved.lock.requires.len(), 4);
    }

    #[test]
    fn lock_files_round_trip() {
        let lock = LockFile {
            version: 1,
            packages: vec![
                LockedPackage {
                    name: "cdc".to_owned(),
                    version: Version::new(0, 3, 1),
                    origin: DepSource::Path("../ip/cdc".to_owned()),
                },
                LockedPackage {
                    name: "fifo".to_owned(),
                    version: Version::parse("1.0.4-rc1").unwrap(),
                    origin: DepSource::Git {
                        url: "https://x.invalid/f.git".to_owned(),
                        rev: Some("v1".to_owned()),
                    },
                },
                LockedPackage {
                    name: "uart".to_owned(),
                    version: Version::new(2, 0, 0),
                    origin: DepSource::Registry,
                },
            ],
            requires: vec![("fifo".to_owned(), "cdc".to_owned(), VersionReq::Any)],
        };
        let text = lock.to_text();
        let mut map = SourceMap::new();
        let file = map.add("reticle.lock", &text).unwrap();
        let mut diags = Diagnostics::new();
        let again = LockFile::parse(&text, file, &mut diags);
        assert_eq!(diags.render(&map), "");
        assert_eq!(again, lock);
        assert_eq!(again.to_text(), text);
        assert_eq!(
            LockFile::new().to_text(),
            format!("{LOCK_HEADER}version 1\n")
        );
    }

    #[test]
    fn lock_file_errors_are_reported() {
        let cases: [(&str, &str); 6] = [
            ("version\n", "`version` takes `version <number>`"),
            ("version x\n", "`version` takes `version <number>`"),
            (
                "package a\n",
                "`package` takes `package <name> <version> <origin>`",
            ),
            ("package a 1.2 path x\n", "a `major.minor.patch` version"),
            ("package a 1.2.3 nfs x\n", "an origin of `path <dir>`"),
            ("requires a b\n", "`requires` takes `requires <dependent>"),
        ];
        for (text, expected) in cases {
            let mut map = SourceMap::new();
            let file = map.add("reticle.lock", text).unwrap();
            let mut diags = Diagnostics::new();
            LockFile::parse(text, file, &mut diags);
            let rendered = diags.render(&map);
            assert!(rendered.contains(expected), "{text:?} gave {rendered}");
        }
        let text = "wobble 1\nrequires a b ~1\n";
        let mut map = SourceMap::new();
        let file = map.add("reticle.lock", text).unwrap();
        let mut diags = Diagnostics::new();
        LockFile::parse(text, file, &mut diags);
        let rendered = diags.render(&map);
        assert!(
            rendered.contains("unknown lock file directive `wobble`"),
            "{rendered}"
        );
        assert!(
            rendered.contains("a version requirement as its last word"),
            "{rendered}"
        );
    }

    #[test]
    fn differences_say_what_moved() {
        let old = LockFile {
            version: 1,
            packages: vec![
                LockedPackage {
                    name: "a".to_owned(),
                    version: Version::new(1, 0, 0),
                    origin: DepSource::Path("x".to_owned()),
                },
                LockedPackage {
                    name: "gone".to_owned(),
                    version: Version::new(0, 1, 0),
                    origin: DepSource::Registry,
                },
            ],
            requires: Vec::new(),
        };
        let new = LockFile {
            version: 1,
            packages: vec![
                LockedPackage {
                    name: "a".to_owned(),
                    version: Version::new(1, 1, 0),
                    origin: DepSource::Path("y".to_owned()),
                },
                LockedPackage {
                    name: "fresh".to_owned(),
                    version: Version::new(2, 0, 0),
                    origin: DepSource::Registry,
                },
            ],
            requires: Vec::new(),
        };
        assert_eq!(
            old.differences(&new),
            [
                "`a` moves from 1.0.0 to 1.1.0",
                "`a` moves from the directory `x` to the directory `y`",
                "`fresh` 2.0.0 is new",
                "`gone` 0.1.0 is no longer used",
            ]
        );
        assert!(new.differences(&new).is_empty());
    }

    #[test]
    fn errors_display_as_their_headline() {
        let span = {
            let mut map = SourceMap::new();
            Span::new(map.add("x", "").unwrap(), 0, 0)
        };
        let error = ResolveError::NotFound {
            name: "a".to_owned(),
            where_: "`x`".to_owned(),
            span,
        };
        assert_eq!(error.to_string(), "cannot find the IP package `a`");
        let deep = ResolveError::TooDeep {
            name: "a".to_owned(),
            span,
        };
        assert!(deep.to_string().contains("more than 1000 deep"));
        let empty = ResolveError::Conflict {
            name: "a".to_owned(),
            requirements: Vec::new(),
            available: Vec::new(),
        };
        assert_eq!(empty.diagnostic().notes[0], "none were available");
    }
}
