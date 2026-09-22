//! The registry: a static index of packages, and `reticle add`.
//!
//! A registry here is what crates.io's index is: a **git repository of
//! manifests**, one file per package, each line one released version.
//! There is no server, no API and nothing to run — cloning or pulling the
//! repository is the whole protocol, `git log` is the audit trail, and a
//! mirror is a clone.
//!
//! # The layout
//!
//! [`index_path`] puts a package where its name says, so a directory
//! listing stays a few hundred entries long however large the index
//! grows:
//!
//! | Name | Path |
//! |------|------|
//! | `a` | `1/a` |
//! | `ab` | `2/ab` |
//! | `abc` | `3/a/abc` |
//! | `fifo_sync` | `fi/fo/fifo_sync` |
//!
//! # The file
//!
//! One `package` line per released version, in the line-oriented format
//! the rest of `src/ip` uses:
//!
//! ```text
//! package fifo_sync 1.0.0 checksum 6f1e…  description "A synchronous FIFO"
//! package fifo_sync 1.0.4 checksum 91a2…  depends cdc_sync ^0.3.0
//! package fifo_sync 1.1.0 checksum 0c77…  yanked
//! ```
//!
//! After the name and version come the words in a fixed order:
//! `checksum <word>`, then `yanked` if it is, then
//! `description "<text>"` if there is one, then one `depends <name>
//! <requirement>` per dependency. A line is a *summary* of a manifest:
//! enough to resolve a graph without fetching anything, and nothing
//! else.
//!
//! What the checksum is over, and with which algorithm, is the index
//! publisher's business: this crate ships no hash function, and
//! verification belongs next to the bytes, in the fetcher. It is carried
//! here so that a lock file can be checked against the index the next
//! time round.
//!
//! # Resolving through it
//!
//! [`RegistryProvider`] is a [`SourceProvider`], so a registry
//! dependency resolves exactly as a path one does. The library still
//! performs no I/O: the provider takes a closure, is handed a path like
//! `fifo_sync-1.0.4/reticle.ip`, and whatever the caller has — a
//! checkout, a cache directory, a bundle compiled into a WebAssembly
//! module — answers it.
//!
//! ```
//! use std::collections::BTreeMap;
//! use reticle::diag::Diagnostics;
//! use reticle::ip::registry::{Index, RegistryProvider};
//! use reticle::ip::{Project, Resolver};
//! use reticle::source::SourceMap;
//!
//! let index_text = "package fifo 1.0.0 checksum abc123\n";
//! let files = BTreeMap::from([(
//!     "fifo-1.0.0/reticle.ip".to_owned(),
//!     "name fifo\nversion 1.0.0\n".to_owned(),
//! )]);
//!
//! let mut map = SourceMap::new();
//! let mut diags = Diagnostics::new();
//! let id = map.add("index", index_text).unwrap();
//! let index = Index::parse(index_text, id, &mut diags);
//!
//! let text = "name blinky\ndepends fifo ^1.0.0 registry\n";
//! let id = map.add("reticle.proj", text).unwrap();
//! let project = Project::parse(text, id, &mut diags).unwrap();
//!
//! let mut provider = RegistryProvider::new(&index, |path: &str| files.get(path).cloned());
//! let resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
//! assert!(resolved.is_complete());
//! assert_eq!(resolved.packages[0].version().to_string(), "1.0.0");
//! ```
//!
//! # `reticle add`
//!
//! [`add`] is the library half of the command: it picks the best version
//! the index offers, writes the `depends` line into the project manifest
//! **in place**, and hands back the new text. The rewrite keeps every
//! comment, every blank line and the existing alignment, and puts the new
//! line where it belongs — in name order when the file is already in name
//! order, and after the last `depends` line otherwise. A tool that
//! reformats a file it was asked to add one line to is a tool nobody lets
//! near their repository twice.

use std::collections::BTreeMap;
use std::fmt;

use super::manifest::{DepSource, Dependency, IpManifest, Project, Version, VersionReq};
use super::resolve::{ResolveError, ResolvedIp, SourceProvider, join};
use super::text::{Token, closest, quote, tokenize};
use crate::diag::{Diagnostic, Diagnostics};
use crate::source::{SourceId, Span};

/// Diagnostic code for a malformed line in an index file.
pub const INDEX_SYNTAX: &str = "P0701";
/// Diagnostic code for a package the index does not have.
pub const NO_SUCH_PACKAGE: &str = "P0702";
/// Diagnostic code for a package the index has no usable version of.
pub const NO_SUCH_VERSION: &str = "P0703";
/// Diagnostic code for a dependency the project already has.
pub const ALREADY_A_DEPENDENCY: &str = "P0704";

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Where a package's file lives inside the index.
///
/// The prefix comes from the name, so no directory ever holds more than
/// the packages sharing four leading characters. Names are compared
/// lower-cased, as they are everywhere else here.
///
/// ```
/// use reticle::ip::registry::index_path;
/// assert_eq!(index_path("a"), "1/a");
/// assert_eq!(index_path("ab"), "2/ab");
/// assert_eq!(index_path("abc"), "3/a/abc");
/// assert_eq!(index_path("fifo_sync"), "fi/fo/fifo_sync");
/// ```
pub fn index_path(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    match chars.len() {
        0 => "0/_".to_owned(),
        1 => format!("1/{lower}"),
        2 => format!("2/{lower}"),
        3 => format!("3/{}/{lower}", chars[0]),
        _ => {
            let first: String = chars[..2].iter().collect();
            let second: String = chars[2..4].iter().collect();
            format!("{first}/{second}/{lower}")
        }
    }
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// One released version of one package, as the index summarises it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    /// The package name.
    pub name: String,
    /// The released version.
    pub version: Version,
    /// The publisher's checksum over the package's bytes, opaque here.
    pub checksum: String,
    /// One line about what the package is, for [`Index::search`].
    pub description: Option<String>,
    /// True when the release was withdrawn: it still resolves for a lock
    /// file that names it, but nothing new selects it.
    pub yanked: bool,
    /// What this version depends on, with no source: where a dependency
    /// comes from is the project's business, as everywhere else here.
    pub depends: Vec<Dependency>,
    /// The line this came from.
    pub span: Span,
}

impl IndexEntry {
    /// The summary of a manifest, with a checksum the caller computed.
    pub fn from_manifest(manifest: &IpManifest, checksum: impl Into<String>) -> IndexEntry {
        IndexEntry {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            checksum: checksum.into(),
            description: manifest.description.clone(),
            yanked: false,
            depends: manifest
                .depends
                .iter()
                .map(|dep| Dependency {
                    source: None,
                    ..dep.clone()
                })
                .collect(),
            span: manifest.span,
        }
    }

    /// The entry as one index line, newline included.
    pub fn to_line(&self) -> String {
        let mut words = vec![
            quote(&self.name),
            self.version.to_string(),
            "checksum".to_owned(),
            quote(&self.checksum),
        ];
        if self.yanked {
            words.push("yanked".to_owned());
        }
        if let Some(description) = &self.description {
            words.push("description".to_owned());
            words.push(quote(description));
        }
        for dep in &self.depends {
            words.push("depends".to_owned());
            words.push(quote(&dep.name));
            words.push(dep.req.to_string());
        }
        format!("package {}\n", words.join(" "))
    }
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// A whole index, or as much of one as has been read.
///
/// Entries are kept sorted by name and then version, so every listing,
/// search and rendering is deterministic whatever order they arrived in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    entries: Vec<IndexEntry>,
}

impl Index {
    /// An empty index.
    pub fn new() -> Index {
        Index {
            entries: Vec::new(),
        }
    }

    /// Parses index text: any number of `package` lines, from one
    /// package's file or from a whole index concatenated.
    ///
    /// A malformed line is reported and skipped, so one bad line does
    /// not lose the file — an index is other people's data and is very
    /// likely to have one.
    pub fn parse(text: &str, file: SourceId, diags: &mut Diagnostics) -> Index {
        let mut out = Index::new();
        for line in tokenize(text, file) {
            if line.keyword() != "package" {
                diags.push(
                    Diagnostic::error(format!("unknown index directive `{}`", line.keyword()))
                        .with_code(INDEX_SYNTAX)
                        .with_span(line.keyword_span())
                        .with_note("every line of an index file is a `package` line"),
                );
                continue;
            }
            let args = line.args();
            if args.len() < 4 {
                syntax(
                    diags,
                    line.span,
                    "`package <name> <version> checksum <sum>`",
                );
                continue;
            }
            let Some(version) = Version::parse(args[1].as_str()) else {
                syntax(diags, args[1].span, "a `major.minor.patch` version");
                continue;
            };
            if !args[2].is("checksum") {
                syntax(diags, args[2].span, "`checksum <sum>` after the version");
                continue;
            }
            let mut entry = IndexEntry {
                name: args[0].as_str().to_owned(),
                version,
                checksum: args[3].as_str().to_owned(),
                description: None,
                yanked: false,
                depends: Vec::new(),
                span: line.span,
            };
            if !read_tail(&args[4..], &mut entry, diags, line.span) {
                continue;
            }
            out.insert(entry);
        }
        out
    }

    /// Renders every entry, sorted, as the concatenation of the index's
    /// package files.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.to_line());
        }
        out
    }

    /// Adds an entry, replacing any entry for the same name and version.
    pub fn insert(&mut self, entry: IndexEntry) {
        let wanted = (entry.name.as_str(), &entry.version);
        match self
            .entries
            .binary_search_by(|e| (e.name.as_str(), &e.version).cmp(&wanted))
        {
            Ok(at) => self.entries[at] = entry,
            Err(at) => self.entries.insert(at, entry),
        }
    }

    /// Every entry, sorted by name and then version.
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    /// How many entries the index holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the index holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every package name, once each, in order.
    pub fn names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for entry in &self.entries {
            if out.last() != Some(&entry.name.as_str()) {
                out.push(&entry.name);
            }
        }
        out
    }

    /// Every released version of a package, oldest first, yanked ones
    /// included.
    pub fn versions(&self, name: &str) -> Vec<&IndexEntry> {
        self.entries.iter().filter(|e| e.name == name).collect()
    }

    /// The entry for one exact version.
    pub fn entry(&self, name: &str, version: &Version) -> Option<&IndexEntry> {
        self.entries
            .iter()
            .find(|e| e.name == name && e.version == *version)
    }

    /// The highest version that satisfies `req` and is not yanked.
    ///
    /// A yanked version is only ever selected by naming it exactly, and
    /// then only because a lock file already did.
    ///
    /// ```
    /// use reticle::diag::Diagnostics;
    /// use reticle::ip::registry::Index;
    /// use reticle::ip::VersionReq;
    /// use reticle::source::SourceMap;
    ///
    /// let text = "package a 1.0.0 checksum x\npackage a 1.4.0 checksum y\n\
    ///             package a 2.0.0 checksum z yanked\n";
    /// let mut map = SourceMap::new();
    /// let file = map.add("a", text).unwrap();
    /// let index = Index::parse(text, file, &mut Diagnostics::new());
    /// let req = VersionReq::parse("*").unwrap();
    /// assert_eq!(index.best("a", &req).unwrap().version.to_string(), "1.4.0");
    /// ```
    pub fn best(&self, name: &str, req: &VersionReq) -> Option<&IndexEntry> {
        let exact = matches!(req, VersionReq::Exact(_));
        self.entries
            .iter()
            .rev()
            .filter(|e| e.name == name)
            .find(|e| req.matches(&e.version) && (!e.yanked || exact))
    }

    /// The package names and descriptions matching `query`, best first.
    ///
    /// The ranking is the four kinds of match in [`MatchKind`] order, and
    /// then the name, so the same query over the same index always
    /// returns the same list. An empty query matches everything, since
    /// every name starts with it.
    pub fn search(&self, query: &str) -> Vec<Match> {
        let needle = query.trim().to_ascii_lowercase();
        let mut best: BTreeMap<&str, (MatchKind, &IndexEntry)> = BTreeMap::new();
        for entry in &self.entries {
            let lower = entry.name.to_ascii_lowercase();
            let kind = if lower == needle {
                MatchKind::ExactName
            } else if lower.starts_with(&needle) {
                MatchKind::NamePrefix
            } else if lower.contains(&needle) {
                MatchKind::NameSubstring
            } else if entry
                .description
                .as_ref()
                .is_some_and(|d| d.to_ascii_lowercase().contains(&needle))
            {
                MatchKind::Description
            } else {
                continue;
            };
            // One row per package, showing the version somebody would
            // actually get: the newest that is neither yanked nor a
            // pre-release, falling back through pre-releases to yanked
            // ones when that is all there is.
            let rank = |e: &IndexEntry| (!e.yanked, !e.version.is_prerelease(), e.version.clone());
            let better = best
                .get(entry.name.as_str())
                .is_none_or(|(_, held)| rank(entry) > rank(held));
            if better {
                best.insert(entry.name.as_str(), (kind, entry));
            }
        }
        let mut out: Vec<Match> = best
            .into_iter()
            .map(|(name, (kind, entry))| Match {
                name: name.to_owned(),
                version: entry.version.clone(),
                description: entry.description.clone(),
                yanked: entry.yanked,
                kind,
            })
            .collect();
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// The text of one package's index file, as [`index_path`] names it.
    pub fn file_text(&self, name: &str) -> Option<String> {
        let mut out = String::new();
        for entry in self.entries.iter().filter(|e| e.name == name) {
            out.push_str(&entry.to_line());
        }
        (!out.is_empty()).then_some(out)
    }

    /// Every package's `(path, contents)`, for writing a whole index out.
    pub fn files(&self) -> Vec<(String, String)> {
        self.names()
            .into_iter()
            .map(|name| {
                (
                    index_path(name),
                    self.file_text(name).expect("the name came from the index"),
                )
            })
            .collect()
    }
}

/// Reads the optional words after `checksum <sum>`.
///
/// Returns false when the line was malformed, which is reported.
fn read_tail(words: &[Token], entry: &mut IndexEntry, diags: &mut Diagnostics, span: Span) -> bool {
    let mut i = 0usize;
    while i < words.len() {
        if words[i].is("yanked") {
            entry.yanked = true;
            i += 1;
        } else if words[i].is("description") {
            let Some(text) = words.get(i + 1) else {
                syntax(diags, span, "a line after `description`");
                return false;
            };
            entry.description = Some(text.as_str().to_owned());
            i += 2;
        } else if words[i].is("depends") {
            let (Some(name), Some(req)) = (words.get(i + 1), words.get(i + 2)) else {
                syntax(diags, span, "`depends <name> <requirement>`");
                return false;
            };
            let Some(parsed) = VersionReq::parse(req.as_str()) else {
                syntax(diags, req.span, "a version requirement after the name");
                return false;
            };
            entry
                .depends
                .push(Dependency::new(name.as_str(), parsed, span));
            i += 3;
        } else {
            let mut d = Diagnostic::error(format!("`{}` is not an index word", words[i]))
                .with_code(INDEX_SYNTAX)
                .with_span(words[i].span);
            let known = ["yanked", "description", "depends"];
            d = match closest(words[i].as_str(), &known) {
                Some(s) => d.with_note(format!("did you mean `{s}`?")),
                None => d.with_note(format!("a line may go on with: {}", known.join(", "))),
            };
            diags.push(d);
            return false;
        }
    }
    true
}

fn syntax(diags: &mut Diagnostics, span: Span, expected: &str) {
    diags.push(
        Diagnostic::error(format!("an index line takes {expected}"))
            .with_code(INDEX_SYNTAX)
            .with_span(span),
    );
}

/// Why a package matched a search, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchKind {
    /// The name is the query.
    ExactName,
    /// The name starts with the query.
    NamePrefix,
    /// The name contains the query.
    NameSubstring,
    /// Only the description contains the query.
    Description,
}

impl MatchKind {
    /// How a listing explains the match.
    pub fn describe(self) -> &'static str {
        match self {
            MatchKind::ExactName => "exact name",
            MatchKind::NamePrefix => "name prefix",
            MatchKind::NameSubstring => "name",
            MatchKind::Description => "description",
        }
    }
}

impl fmt::Display for MatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.describe())
    }
}

/// One package a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The package name.
    pub name: String,
    /// The version this row describes: the newest release that is
    /// neither yanked nor a pre-release, when there is one.
    pub version: Version,
    /// Its description, when it has one.
    pub description: Option<String>,
    /// True when even the newest version is yanked.
    pub yanked: bool,
    /// Why it matched.
    pub kind: MatchKind,
}

impl fmt::Display for Match {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name, self.version)?;
        if self.yanked {
            f.write_str(" (yanked)")?;
        }
        if let Some(description) = &self.description {
            write!(f, " — {description}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Resolving through the index
// ---------------------------------------------------------------------------

/// A [`SourceProvider`] over an [`Index`], reading through a closure.
///
/// The closure is given `<name>-<version>/<path>` and answers with the
/// bytes or with `None`; verifying them against
/// [`IndexEntry::checksum`] is its job, since the crate ships no hash
/// function and the bytes never pass through here.
///
/// A `path` or `git` dependency is declined with
/// [`ResolveError::Unsupported`]: this provider knows one place to look.
/// Compose it with another provider to mix the two in one project.
pub struct RegistryProvider<'a, F> {
    index: &'a Index,
    read: F,
    root: String,
}

impl<'a, F: FnMut(&str) -> Option<String>> RegistryProvider<'a, F> {
    /// A provider over `index`, reading files through `read`.
    ///
    /// The project's own sources are read relative to the directory the
    /// project is in, which starts as `.`; [`with_root`] moves it.
    ///
    /// [`with_root`]: RegistryProvider::with_root
    pub fn new(index: &'a Index, read: F) -> Self {
        RegistryProvider {
            index,
            read,
            root: ".".to_owned(),
        }
    }

    /// Sets the directory the project's own sources are read from.
    pub fn with_root(mut self, root: impl Into<String>) -> Self {
        self.root = root.into();
        self
    }

    /// The directory a version's files are laid out under.
    pub fn root_for(name: &str, version: &Version) -> String {
        format!("{name}-{version}")
    }
}

impl<F: FnMut(&str) -> Option<String>> SourceProvider for RegistryProvider<'_, F> {
    fn fetch(&mut self, dep: &Dependency) -> Result<ResolvedIp, ResolveError> {
        if let Some(other) = &dep.source
            && *other != DepSource::Registry
        {
            return Err(ResolveError::Unsupported {
                name: dep.name.clone(),
                source: other.clone(),
                span: dep.span,
            });
        }
        let Some(entry) = self.index.best(&dep.name, &dep.req) else {
            let available = self.index.versions(&dep.name);
            let where_ = if available.is_empty() {
                "the registry index, which has no such package".to_owned()
            } else {
                format!(
                    "the registry index, which has {}",
                    available
                        .iter()
                        .map(|e| format!(
                            "{}{}",
                            e.version,
                            if e.yanked { " (yanked)" } else { "" }
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(ResolveError::NotFound {
                name: dep.name.clone(),
                where_,
                span: dep.span,
            });
        };
        let root = Self::root_for(&entry.name, &entry.version);
        let manifest_path = join(&root, "reticle.ip");
        match (self.read)(&manifest_path) {
            Some(manifest_text) => Ok(ResolvedIp {
                origin: DepSource::Registry,
                root,
                manifest_path,
                manifest_text,
            }),
            None => Err(ResolveError::NotFound {
                name: dep.name.clone(),
                where_: format!("`{manifest_path}`, which the registry index lists"),
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

// ---------------------------------------------------------------------------
// `reticle add`
// ---------------------------------------------------------------------------

/// A project manifest and the exact text it was parsed from.
///
/// [`add`] needs both: the parsed form to decide what to do, and the
/// text to rewrite, because the text holds the comments and the layout
/// that parsing threw away.
#[derive(Clone, Copy, Debug)]
pub struct ProjectFile<'a> {
    /// The parsed manifest.
    pub project: &'a Project,
    /// The text it was parsed from.
    pub text: &'a str,
}

impl<'a> ProjectFile<'a> {
    /// Pairs a project with its text.
    pub fn new(project: &'a Project, text: &'a str) -> Self {
        ProjectFile { project, text }
    }
}

/// Why a dependency could not be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddError {
    /// The index has no package of that name.
    NoSuchPackage {
        /// The name that was asked for.
        name: String,
        /// The closest name the index does have, when one is close.
        close: Option<String>,
        /// Where to point the diagnostic.
        span: Span,
    },
    /// The index has the package, but no version satisfying the
    /// requirement that is not yanked.
    NoSuchVersion {
        /// The package name.
        name: String,
        /// What was asked for.
        req: VersionReq,
        /// What the index offers, ascending, yanked ones marked.
        available: Vec<(Version, bool)>,
        /// Where to point the diagnostic.
        span: Span,
    },
    /// The project already depends on that package.
    Already {
        /// The package name.
        name: String,
        /// The requirement already written.
        req: VersionReq,
        /// The `depends` line that has it.
        span: Span,
    },
}

impl AddError {
    /// The rustc-style diagnostic for this error.
    pub fn diagnostic(&self) -> Diagnostic {
        match self {
            AddError::NoSuchPackage { name, close, span } => {
                let d = Diagnostic::error(format!("the registry has no package `{name}`"))
                    .with_code(NO_SUCH_PACKAGE)
                    .with_span(*span);
                match close {
                    Some(close) => d.with_note(format!("did you mean `{close}`?")),
                    None => d.with_note("`reticle search` lists what the index does have"),
                }
            }
            AddError::NoSuchVersion {
                name,
                req,
                available,
                span,
            } => Diagnostic::error(format!("no release of `{name}` satisfies {req}"))
                .with_code(NO_SUCH_VERSION)
                .with_span(*span)
                .with_note(if available.is_empty() {
                    "the index lists no release of it at all".to_owned()
                } else {
                    format!(
                        "the index has {}",
                        available
                            .iter()
                            .map(|(v, yanked)| format!(
                                "{v}{}",
                                if *yanked { " (yanked)" } else { "" }
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }),
            AddError::Already { name, req, span } => {
                Diagnostic::error(format!("the project already depends on `{name}`"))
                    .with_code(ALREADY_A_DEPENDENCY)
                    .with_span(*span)
                    .with_note(format!("it requires {req}; edit that line to change it"))
            }
        }
    }
}

impl fmt::Display for AddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic().message)
    }
}

impl std::error::Error for AddError {}

/// A project with one more dependency in it.
///
/// This is what [`add`] returns rather than a bare [`Project`], because
/// the caller needs the rewritten **text** to write back; a `Project`
/// alone would have to be rendered, and rendering would throw away the
/// user's comments and layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedProject {
    /// The project as it now reads.
    ///
    /// The new dependency carries the span of the whole manifest until
    /// the file is read back, since it has no line of its own yet.
    pub project: Project,
    /// The rewritten manifest text, comments and layout preserved.
    pub text: String,
    /// The version the index offered.
    pub version: Version,
    /// The line that was inserted, without its line ending.
    pub line: String,
    /// Which line of the new text it went on, counting from one.
    pub at: usize,
}

/// Adds a registry dependency to a project manifest.
///
/// This is what `reticle add <name>` calls: it picks the best version of
/// `name` the index offers for `req`, writes a `depends` line into the
/// manifest text and returns the result. Nothing is written to disk —
/// the caller does that with [`AddedProject::text`].
///
/// ```
/// use reticle::diag::Diagnostics;
/// use reticle::ip::registry::{self, Index, ProjectFile};
/// use reticle::ip::{Project, VersionReq};
/// use reticle::source::SourceMap;
///
/// let index_text = "package fifo 1.2.0 checksum abc\n";
/// let text = "# my design\nname blinky\n\ndepends uart ^1.0.0 registry\n";
///
/// let mut map = SourceMap::new();
/// let mut diags = Diagnostics::new();
/// let id = map.add("index", index_text).unwrap();
/// let index = Index::parse(index_text, id, &mut diags);
/// let id = map.add("reticle.proj", text).unwrap();
/// let project = Project::parse(text, id, &mut diags).unwrap();
///
/// let req = VersionReq::parse("^1.2.0").unwrap();
/// let added = registry::add(&ProjectFile::new(&project, text), "fifo", &req, &index).unwrap();
/// assert_eq!(added.line, "depends fifo ^1.2.0 registry");
/// assert!(added.text.starts_with("# my design\n"));
/// ```
pub fn add(
    project: &ProjectFile<'_>,
    name: &str,
    req: &VersionReq,
    index: &Index,
) -> Result<AddedProject, AddError> {
    let manifest = project.project;
    if let Some(existing) = manifest.dependency(name) {
        return Err(AddError::Already {
            name: name.to_owned(),
            req: existing.req.clone(),
            span: existing.span,
        });
    }
    let versions = index.versions(name);
    if versions.is_empty() {
        return Err(AddError::NoSuchPackage {
            name: name.to_owned(),
            close: closest(name, &index.names()).map(str::to_owned),
            span: manifest.span,
        });
    }
    let Some(entry) = index.best(name, req) else {
        return Err(AddError::NoSuchVersion {
            name: name.to_owned(),
            req: req.clone(),
            available: versions
                .iter()
                .map(|e| (e.version.clone(), e.yanked))
                .collect(),
            span: manifest.span,
        });
    };

    let dependency = Dependency {
        name: entry.name.clone(),
        req: req.clone(),
        source: Some(DepSource::Registry),
        span: manifest.span,
    };
    let (text, line, at) = insert_line(project.text, &dependency);
    let mut rewritten = manifest.clone();
    rewritten.depends.push(dependency);
    Ok(AddedProject {
        project: rewritten,
        text,
        version: entry.version.clone(),
        line,
        at,
    })
}

/// Writes one `depends` line into a manifest's text.
///
/// Returns the new text, the line as it was written and which line of
/// the result it is. The rules are in [`add`]'s documentation: keep the
/// line ending, keep the alignment, insert in name order when the file
/// is already in name order, and otherwise after the last `depends`.
fn insert_line(text: &str, dep: &Dependency) -> (String, String, usize) {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text
        .split_inclusive('\n')
        .map(std::string::ToString::to_string)
        .collect();

    // Every `depends` line, as (index, the name it names, the spacing
    // after the keyword).
    let mut existing: Vec<(usize, String, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\n', '\r']);
        let Some(rest) = body.strip_prefix("depends") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let spacing: String = rest.chars().take_while(|c| c.is_whitespace()).collect();
        let name = rest.split_whitespace().next().unwrap_or("").to_owned();
        existing.push((i, name, spacing));
    }

    let spacing = existing
        .last()
        .map_or_else(|| " ".to_owned(), |(_, _, s)| s.clone());
    let mut words = vec![quote(&dep.name), dep.req.to_string()];
    if let Some(source) = &dep.source {
        words.push(source.to_string());
    }
    let written = format!("depends{spacing}{}", words.join(" "));

    let sorted = existing.windows(2).all(|w| w[0].1 <= w[1].1);
    let at = match existing.last() {
        // No `depends` line yet: the file grows at the end, after a
        // blank line if the last line is not one already.
        None => {
            if let Some(last) = lines.last_mut()
                && !last.ends_with('\n')
            {
                last.push_str(ending);
            }
            if lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.len()
            } else {
                lines.push(ending.to_owned());
                lines.len()
            }
        }
        Some((last, _, _)) => {
            if sorted {
                existing
                    .iter()
                    .find(|(_, name, _)| *name > dep.name)
                    .map_or(last + 1, |(i, _, _)| *i)
            } else {
                last + 1
            }
        }
    };
    lines.insert(at, format!("{written}{ending}"));
    // A file that did not end in a newline before must not start doing
    // so because of a line inserted above the last one.
    let inserted_last = at + 1 == lines.len();
    if !text.ends_with('\n')
        && !inserted_last
        && let Some(last) = lines.last_mut()
    {
        let trimmed = last.trim_end_matches(['\n', '\r']).to_owned();
        *last = trimmed;
    }
    (lines.concat(), written, at + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostics;
    use crate::ip::{self, Resolver};
    use crate::source::SourceMap;

    const INDEX: &str = "\
package cdc_sync 0.3.1 checksum c1 description \"Two-flop synchroniser\"
package fifo_sync 1.0.0 checksum f1 description \"A synchronous FIFO\"
package fifo_sync 1.0.4 checksum f2 description \"A synchronous FIFO\" depends cdc_sync ^0.3.0
package fifo_sync 1.1.0 checksum f3 yanked description \"A synchronous FIFO\"
package uart_lite 1.2.0 checksum u1 description \"An AXI4-Lite UART with a 16-byte FIFO\" depends fifo_sync ^1.0.0
package uart_lite 2.0.0-rc1 checksum u2 description \"An AXI4-Lite UART with a 16-byte FIFO\"
";

    /// Parses index text, returning it and everything reported.
    fn parse_index(text: &str) -> (Index, String) {
        let mut map = SourceMap::new();
        let file = map.add("index", text).unwrap();
        let mut diags = Diagnostics::new();
        let index = Index::parse(text, file, &mut diags);
        (index, diags.render(&map))
    }

    /// [`INDEX`], which has to parse cleanly.
    fn parsed() -> Index {
        let (index, reported) = parse_index(INDEX);
        assert_eq!(reported, "");
        index
    }

    /// Parses a project manifest into `map`.
    fn parse_project_in(map: &mut SourceMap, text: &str) -> Project {
        let file = map.add("reticle.proj", text).unwrap();
        let mut diags = Diagnostics::new();
        let project = Project::parse(text, file, &mut diags).expect("parses");
        assert!(!diags.has_errors(), "{}", diags.render(map));
        project
    }

    /// Parses a project manifest into a map of its own.
    fn parse_project(text: &str) -> Project {
        parse_project_in(&mut SourceMap::new(), text)
    }

    #[test]
    fn parses_and_round_trips() {
        let index = parsed();
        assert_eq!(index.len(), 6);
        assert_eq!(index.names(), vec!["cdc_sync", "fifo_sync", "uart_lite"]);
        assert_eq!(index.to_text(), INDEX);
        // And parsing what it rendered gives the same index again.
        let (again, reported) = parse_index(&index.to_text());
        assert_eq!(reported, "");
        assert_eq!(again, index);

        let entry = index.entry("fifo_sync", &Version::new(1, 0, 4)).unwrap();
        assert_eq!(entry.checksum, "f2");
        assert_eq!(entry.depends.len(), 1);
        assert_eq!(entry.depends[0].name, "cdc_sync");
        assert_eq!(entry.depends[0].req.to_string(), "^0.3.0");
        assert!(entry.depends[0].source.is_none());
        assert!(!entry.yanked);
        assert!(index.entry("fifo_sync", &Version::new(9, 0, 0)).is_none());
    }

    #[test]
    fn entries_are_sorted_however_they_arrive() {
        let text = "\
package b 1.0.0 checksum x
package a 2.0.0 checksum x
package a 1.0.0 checksum x
";
        let (index, reported) = parse_index(text);
        assert_eq!(reported, "");
        assert_eq!(
            index.to_text(),
            "package a 1.0.0 checksum x\npackage a 2.0.0 checksum x\npackage b 1.0.0 checksum x\n"
        );
        // A second line for the same release replaces the first.
        let mut index = index;
        let mut entry = index.entry("a", &Version::new(1, 0, 0)).unwrap().clone();
        entry.checksum = "y".to_owned();
        index.insert(entry);
        assert_eq!(index.len(), 3);
        assert_eq!(
            index.entry("a", &Version::new(1, 0, 0)).unwrap().checksum,
            "y"
        );
    }

    #[test]
    fn malformed_lines_are_reported_and_skipped() {
        let text = "\
package good 1.0.0 checksum x
package
package bad notaversion checksum x
package bad 1.0.0 sum x
package bad 1.0.0 checksum x dependss other ^1.0.0
package bad 1.0.0 checksum x depends other notareq
package bad 1.0.0 checksum x description
pkg bad 1.0.0 checksum x
";
        let mut map = SourceMap::new();
        let file = map.add("index", text).unwrap();
        let mut diags = Diagnostics::new();
        let index = Index::parse(text, file, &mut diags);
        let reported = diags.render(&map);
        assert_eq!(index.len(), 1);
        assert_eq!(index.names(), vec!["good"]);
        // One diagnostic per bad line, and the good line still read.
        assert_eq!(diags.error_count(), 7, "{reported}");
        assert!(reported.contains("P0701"), "{reported}");
        assert!(reported.contains("did you mean `depends`?"), "{reported}");
    }

    #[test]
    fn selects_a_version_for_every_requirement_form() {
        let index = parsed();
        let best = |name: &str, req: &str| {
            index
                .best(name, &VersionReq::parse(req).unwrap())
                .map(|e| e.version.to_string())
        };
        // `*` takes the newest release that is not yanked.
        assert_eq!(best("fifo_sync", "*"), Some("1.0.4".to_owned()));
        // `^` stays inside the compatible range.
        assert_eq!(best("fifo_sync", "^1.0.0"), Some("1.0.4".to_owned()));
        assert_eq!(best("fifo_sync", "^1.1.0"), None);
        // `>=` takes everything above, still skipping the yanked one.
        assert_eq!(best("fifo_sync", ">=1.0.0"), Some("1.0.4".to_owned()));
        // An exact requirement is the one way to reach a yanked release,
        // which is what a lock file naming it needs.
        assert_eq!(best("fifo_sync", "1.1.0"), Some("1.1.0".to_owned()));
        assert_eq!(best("fifo_sync", "1.0.0"), Some("1.0.0".to_owned()));
        // A pre-release is only ever selected by name.
        assert_eq!(best("uart_lite", "*"), Some("1.2.0".to_owned()));
        assert_eq!(best("uart_lite", "2.0.0-rc1"), Some("2.0.0-rc1".to_owned()));
        assert_eq!(best("nothing", "*"), None);

        let versions: Vec<String> = index
            .versions("fifo_sync")
            .iter()
            .map(|e| e.version.to_string())
            .collect();
        assert_eq!(versions, vec!["1.0.0", "1.0.4", "1.1.0"]);
        assert!(index.versions("nothing").is_empty());
    }

    #[test]
    fn search_ranks_deterministically() {
        let index = parsed();
        let names = |query: &str| -> Vec<String> {
            index
                .search(query)
                .into_iter()
                .map(|m| format!("{} {} {}", m.name, m.version, m.kind))
                .collect()
        };
        // An exact name first, then a prefix, then a substring, then a
        // description; ties are broken by name.
        assert_eq!(
            names("sync"),
            vec![
                "cdc_sync 0.3.1 name".to_owned(),
                "fifo_sync 1.0.4 name".to_owned(),
            ]
        );
        assert_eq!(names("fifo_sync"), vec!["fifo_sync 1.0.4 exact name"]);
        assert_eq!(
            names("fifo"),
            vec![
                "fifo_sync 1.0.4 name prefix".to_owned(),
                "uart_lite 1.2.0 description".to_owned(),
            ]
        );
        // The query is case-insensitive and trimmed.
        assert_eq!(names("  UART "), vec!["uart_lite 1.2.0 name prefix"]);
        // An empty query is every package, since every name starts with
        // it.
        assert_eq!(index.search("").len(), 3);
        assert!(index.search("nothing at all").is_empty());
        // The row shows the newest version that is not yanked.
        let row = &index.search("fifo_sync")[0];
        assert_eq!(row.version, Version::new(1, 0, 4));
        assert!(!row.yanked);
        assert_eq!(row.to_string(), "fifo_sync 1.0.4 — A synchronous FIFO");

        // A package whose every release is yanked still lists, marked.
        let (index, _) = parse_index("package gone 1.0.0 checksum x yanked\n");
        let row = &index.search("gone")[0];
        assert!(row.yanked);
        assert_eq!(row.to_string(), "gone 1.0.0 (yanked)");
    }

    #[test]
    fn paths_spread_the_index_out() {
        let index = parsed();
        let files = index.files();
        assert_eq!(
            files.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["cd/c_/cdc_sync", "fi/fo/fifo_sync", "ua/rt/uart_lite"]
        );
        // Each file holds exactly that package's lines, and the whole
        // index is their concatenation in name order.
        assert_eq!(
            files.iter().map(|(_, t)| t.as_str()).collect::<String>(),
            index.to_text()
        );
        assert_eq!(index.file_text("fifo_sync").unwrap().lines().count(), 3);
        assert!(index.file_text("nothing").is_none());
        assert_eq!(index_path(""), "0/_");
        assert_eq!(index_path("AbCd"), "ab/cd/abcd");
    }

    #[test]
    fn builds_an_entry_from_a_manifest() {
        let text = "name fifo\nversion 1.0.0\ndescription \"A FIFO\"\n\ndepends cdc ^0.3.0\n";
        let mut map = SourceMap::new();
        let file = map.add("reticle.ip", text).unwrap();
        let mut diags = Diagnostics::new();
        let manifest = IpManifest::parse(text, file, &mut diags).unwrap();
        let entry = IndexEntry::from_manifest(&manifest, "deadbeef");
        assert_eq!(
            entry.to_line(),
            "package fifo 1.0.0 checksum deadbeef description \"A FIFO\" depends cdc ^0.3.0\n"
        );
        let mut index = Index::new();
        assert!(index.is_empty());
        index.insert(entry);
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn resolves_a_registry_dependency() {
        let index = parsed();
        let files: BTreeMap<&str, &str> = BTreeMap::from([
            (
                "uart_lite-1.2.0/reticle.ip",
                "name uart_lite\nversion 1.2.0\n\nsource uart.v\n\ndepends fifo_sync ^1.0.0\n",
            ),
            ("uart_lite-1.2.0/uart.v", "module uart_lite(); endmodule\n"),
            (
                "fifo_sync-1.0.4/reticle.ip",
                "name fifo_sync\nversion 1.0.4\n\nsource fifo.v\n",
            ),
            ("fifo_sync-1.0.4/fifo.v", "module fifo_sync(); endmodule\n"),
        ]);
        let text = "name blinky\n\ndepends uart_lite ^1.2.0 registry\n";
        let mut map = SourceMap::new();
        let project = parse_project_in(&mut map, text);
        let mut diags = Diagnostics::new();
        let mut provider = RegistryProvider::new(&index, |path: &str| {
            files.get(path).map(|s| (*s).to_owned())
        });
        let resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
        assert!(
            resolved.is_complete(),
            "{}",
            diags.render(resolved.source_map())
        );
        // The transitive dependency came from the index too, at the
        // version the index's own summary asked for.
        let names: Vec<String> = resolved
            .packages
            .iter()
            .map(|p| format!("{} {}", p.name(), p.version()))
            .collect();
        assert_eq!(names, vec!["fifo_sync 1.0.4", "uart_lite 1.2.0"]);
        assert_eq!(
            resolved.lock.package("uart_lite").unwrap().origin,
            DepSource::Registry
        );
        // The sources were read through the same closure.
        assert!(resolved.packages[0].sources[0].is_readable());
    }

    #[test]
    fn declines_what_the_registry_cannot_answer() {
        let index = parsed();
        let mut provider = RegistryProvider::new(&index, |_: &str| None);
        let span = parsed_span();

        // A package the index does not list.
        let dep = Dependency::new("nothing", VersionReq::Any, span);
        let error = provider.fetch(&dep).expect_err("no such package");
        assert!(error.to_string().contains("cannot find the IP package"));
        assert!(error.diagnostic().notes[0].contains("no such package"));

        // One it lists at a version nothing satisfies.
        let dep = Dependency::new("fifo_sync", VersionReq::parse("^2.0.0").unwrap(), span);
        let error = provider.fetch(&dep).expect_err("no such version");
        assert!(
            error.diagnostic().notes[0].contains("1.1.0 (yanked)"),
            "{error}"
        );

        // One it lists but whose bytes the fetcher has not got.
        let dep = Dependency::new("fifo_sync", VersionReq::Any, span);
        let error = provider.fetch(&dep).expect_err("no bytes");
        assert!(
            error
                .diagnostic()
                .notes
                .iter()
                .any(|n| n.contains("fifo_sync-1.0.4/reticle.ip")),
            "{error}"
        );

        // And a dependency that names somewhere else entirely.
        let dep = Dependency {
            source: Some(DepSource::Path("../fifo".to_owned())),
            ..Dependency::new("fifo_sync", VersionReq::Any, span)
        };
        let error = provider.fetch(&dep).expect_err("not a registry dependency");
        assert!(matches!(error, ResolveError::Unsupported { .. }));
        // A dependency that says `registry` explicitly is fine.
        let dep = Dependency {
            source: Some(DepSource::Registry),
            ..Dependency::new("fifo_sync", VersionReq::Any, span)
        };
        assert!(matches!(
            provider.fetch(&dep),
            Err(ResolveError::NotFound { .. })
        ));
    }

    /// A span in a throwaway file, for building dependencies by hand.
    fn parsed_span() -> Span {
        let mut map = SourceMap::new();
        let file = map.add("t", "x").unwrap();
        Span::new(file, 0, 1)
    }

    #[test]
    fn add_keeps_comments_and_layout() {
        let text = "\
# The blinky design.
#
# Everything here is hand written except the IP.
name        blinky
top         top
device      ice40-hx1k-tq144

source      rtl/top.v   # the only source

# The IP this needs, newest first in the changelog.
depends     cdc_sync ^0.3.0 registry
depends     uart_lite ^1.2.0 registry
";
        let project = parse_project(text);
        let index = parsed();
        let added = add(
            &ProjectFile::new(&project, text),
            "fifo_sync",
            &VersionReq::parse("^1.0.0").unwrap(),
            &index,
        )
        .expect("adds");
        assert_eq!(added.version, Version::new(1, 0, 4));
        // The alignment of the existing `depends` lines is copied, and
        // the line goes in name order because the file is in name order.
        assert_eq!(added.line, "depends     fifo_sync ^1.0.0 registry");
        assert_eq!(added.at, 12);
        assert_eq!(
            added.text,
            "\
# The blinky design.
#
# Everything here is hand written except the IP.
name        blinky
top         top
device      ice40-hx1k-tq144

source      rtl/top.v   # the only source

# The IP this needs, newest first in the changelog.
depends     cdc_sync ^0.3.0 registry
depends     fifo_sync ^1.0.0 registry
depends     uart_lite ^1.2.0 registry
"
        );
        // Every comment survived, and so did the trailing comment on the
        // source line.
        assert_eq!(added.text.matches('#').count(), 5);
        // The returned project is the old one plus the dependency.
        assert_eq!(added.project.depends.len(), 3);
        let dep = added.project.dependency("fifo_sync").expect("added");
        assert_eq!(dep.source, Some(DepSource::Registry));
        // And the new text parses back to the same set of dependencies.
        let reparsed = parse_project(&added.text);
        assert_eq!(
            reparsed
                .depends
                .iter()
                .map(|d| d.name.clone())
                .collect::<Vec<_>>(),
            vec!["cdc_sync", "fifo_sync", "uart_lite"]
        );
    }

    #[test]
    fn add_puts_the_line_where_it_belongs() {
        let index = parsed();
        let req = VersionReq::parse("^0.3.0").unwrap();
        let go = |text: &str| {
            let project = parse_project(text);
            add(&ProjectFile::new(&project, text), "cdc_sync", &req, &index)
                .expect("adds")
                .text
        };

        // No `depends` line yet: a blank line, then the new one.
        assert_eq!(
            go("name blinky\nsource rtl/top.v\n"),
            "name blinky\nsource rtl/top.v\n\ndepends cdc_sync ^0.3.0 registry\n"
        );
        // A file already ending in a blank line does not grow another.
        assert_eq!(
            go("name blinky\n\n"),
            "name blinky\n\ndepends cdc_sync ^0.3.0 registry\n"
        );
        // A file with no final newline gets one.
        assert_eq!(
            go("name blinky"),
            "name blinky\n\ndepends cdc_sync ^0.3.0 registry\n"
        );
        // Lines that are not in name order are left alone: the new one
        // goes after the last of them.
        assert_eq!(
            go(
                "name blinky\ndepends uart_lite ^1.2.0 registry\ndepends fifo_sync ^1.0.0 registry\n"
            ),
            "name blinky\ndepends uart_lite ^1.2.0 registry\ndepends fifo_sync ^1.0.0 registry\n\
             depends cdc_sync ^0.3.0 registry\n"
        );
        // A sorted file takes it in order, before everything.
        assert_eq!(
            go("name blinky\ndepends fifo_sync ^1.0.0 registry\n"),
            "name blinky\ndepends cdc_sync ^0.3.0 registry\ndepends fifo_sync ^1.0.0 registry\n"
        );
        // Windows line endings stay Windows line endings.
        assert_eq!(
            go("name blinky\r\ndepends fifo_sync ^1.0.0 registry\r\n"),
            "name blinky\r\ndepends cdc_sync ^0.3.0 registry\r\ndepends fifo_sync ^1.0.0 registry\r\n"
        );
        // A commented-out `depends` is a comment, not a dependency.
        assert_eq!(
            go("name blinky\n# depends fifo_sync ^1.0.0 registry\n"),
            "name blinky\n# depends fifo_sync ^1.0.0 registry\n\ndepends cdc_sync ^0.3.0 registry\n"
        );
    }

    #[test]
    fn add_says_why_it_cannot() {
        let index = parsed();
        let text = "name blinky\ndepends fifo_sync ^1.0.0 registry\n";
        let project = parse_project(text);
        let file = ProjectFile::new(&project, text);

        let error = add(&file, "fifo_sync", &VersionReq::Any, &index).expect_err("already there");
        assert!(matches!(error, AddError::Already { .. }));
        assert!(error.to_string().contains("already depends on `fifo_sync`"));
        assert_eq!(error.diagnostic().code, Some(ALREADY_A_DEPENDENCY));

        let error = add(&file, "fifo_snyc", &VersionReq::Any, &index).expect_err("no such package");
        assert_eq!(error.diagnostic().code, Some(NO_SUCH_PACKAGE));
        assert!(
            error.diagnostic().notes[0].contains("did you mean `fifo_sync`?"),
            "{:?}",
            error.diagnostic().notes
        );

        let error =
            add(&file, "wobbleflange", &VersionReq::Any, &index).expect_err("no such package");
        assert!(error.diagnostic().notes[0].contains("reticle search"));

        let error = add(
            &file,
            "cdc_sync",
            &VersionReq::parse("^9.0.0").unwrap(),
            &index,
        )
        .expect_err("no such version");
        assert_eq!(error.diagnostic().code, Some(NO_SUCH_VERSION));
        assert!(error.diagnostic().notes[0].contains("0.3.1"));
    }

    #[test]
    fn a_registry_project_elaborates() {
        // The proof that a registry dependency is a dependency like any
        // other: resolve one, then build the design from it.
        let index_text = "package blink 1.0.0 checksum b1 description \"An LED blinker\"\n";
        let (index, _) = parse_index(index_text);
        let files: BTreeMap<&str, &str> = BTreeMap::from([
            (
                "blink-1.0.0/reticle.ip",
                "name blink\nversion 1.0.0\n\ntop blink\n\nsource blink.v\n",
            ),
            (
                "blink-1.0.0/blink.v",
                "module blink(input clk, output q); assign q = clk; endmodule\n",
            ),
            (
                "top.v",
                "module top(input clk, output q);\n  blink u(.clk(clk), .q(q));\nendmodule\n",
            ),
        ]);
        let text = "name demo\ntop top\n\nsource top.v\n\ndepends blink ^1.0.0 registry\n";

        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let project = ip::load_project(&mut map, "reticle.proj", text, &mut diags).unwrap();
        let mut provider = RegistryProvider::new(&index, |path: &str| {
            files.get(path).map(|s| (*s).to_owned())
        });
        let mut resolved = Resolver::new(map).resolve(&project, &mut provider, &mut diags);
        assert!(resolved.is_complete());
        let design = ip::elaborate_project(&project, &mut resolved, &mut diags).unwrap();
        assert!(
            !diags.has_errors(),
            "{}",
            diags.render(resolved.source_map())
        );
        assert_eq!(design.modules.len(), 2);
        assert_eq!(design.top_module().unwrap().name, "top");
    }
}
