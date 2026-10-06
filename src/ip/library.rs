//! The IP library: placing a dependency by **name**.
//!
//! A package declares its own identity (`name uart`, `version 1.0.0`)
//! and a dependency between packages is already written without a path
//! (`depends usb_device_fs ^1.0.0`). A *project*, until this module
//! existed, had to spell the directory out:
//!
//! ```text
//! depends uart ^1.0.0 path ../../ip/uart
//! ```
//!
//! which put the library's directory layout into every manifest, every
//! lock file, every test that lists sources and every document that
//! quotes a build command. A [`LibraryIndex`] removes the need: the
//! project says once where the library is, and a dependency names only
//! what it wants.
//!
//! ```text
//! # reticle.proj
//! library ../../ip
//!
//! depends uart   ^1.0.0
//! depends mos6502 ^1.0.0
//! ```
//!
//! # Where the root comes from
//!
//! From the `library` lines of `reticle.proj`, each relative to the
//! directory holding that manifest, searched in the order written; the
//! CLI's `--library <dir>` appends more for a build against a library
//! that is not the project's own. There is no default and no environment
//! variable: a path in this project comes from a manifest or from the
//! command line and never from a guess, because a manifest is reviewed
//! and committed and `$RETICLE_IP_PATH` is neither — the same project
//! would then build from different HDL on two machines with nothing in
//! the repository to say so.
//!
//! Order decides nothing, because a name found twice is an error rather
//! than a shadowing; it only decides which root a note lists first.
//!
//! # Sans-I/O
//!
//! Walking a directory is I/O, so the walk is the caller's: the CLI (or
//! a test, or a WebAssembly bundle) finds the `reticle.ip` files under
//! each root and hands over `(path, text)` pairs, and
//! [`LibraryIndex::from_manifests`] turns them into the index. Nothing
//! here opens a file.
//!
//! The paths handed in are in the same space as every other path the
//! resolver sees: **relative to the project manifest's directory**,
//! exactly like a `path` dependency's. That is what lets the index's
//! answer be used by [`super::PathProvider`] without a second notion of
//! where things are.
//!
//! # What is indexed
//!
//! Only a manifest's `name` and `version`, read by a deliberately small
//! scan ([`manifest_identity`]) rather than by
//! [`super::IpManifest::parse`]. Two reasons: a package nobody depends
//! on must not push its own diagnostics into an unrelated build, and the
//! index is built over every manifest in the library while only a few
//! are wanted, so the work per manifest should stay two lines of text.
//! A file with no `name` line is not indexed; [`LibraryIndex::unnamed`]
//! lists those so a caller can say so.
//!
//! # One name, one package
//!
//! Two manifests in the library declaring the same name is
//! [`LibraryProblem::Ambiguous`] ([`AMBIGUOUS`]), naming both paths. It
//! is not "highest version wins": a library is a layout, one directory
//! per package, and the failure this really catches is a package copied
//! where it should have been moved — the exact mistake a reorganisation
//! makes. A library that genuinely wants two versions of one package
//! side by side is what a registry is for ([`super::registry`], which
//! does carry a version per line), or `path`, which still says exactly
//! which directory is meant.
//!
//! The error is raised when the name is *looked up*, not when the index
//! is built, so that it has the `depends` line to point at and so that a
//! duplicate somewhere else in a large library does not fail a build
//! that never wanted it. [`LibraryIndex::duplicates`] reports them all
//! for a caller that wants to check the library itself.

use std::collections::BTreeMap;

use super::manifest::Version;
use super::text::closest;
use crate::diag::Diagnostic;
use crate::source::Span;

/// Diagnostic code for a name the library has no package for.
pub const NOT_IN_LIBRARY: &str = "P0801";
/// Diagnostic code for two packages in the library with one name.
pub const AMBIGUOUS: &str = "P0802";

/// The file a package's identity is read from.
pub const MANIFEST_NAME: &str = "reticle.ip";

/// One package the index found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryEntry {
    /// The name the package's manifest declares.
    pub name: String,
    /// The version it declares, when it declares a well-formed one.
    ///
    /// A missing or malformed `version` does not keep a package out of
    /// the index: resolution reads the real manifest afterwards and
    /// reports what is wrong with it there, with spans.
    pub version: Option<Version>,
    /// The package's directory, relative to the project manifest.
    pub dir: String,
    /// The path of its `reticle.ip`, relative to the project manifest.
    pub manifest: String,
}

/// An index of the packages under a project's `library` roots.
///
/// Built by the caller's walk (see the module documentation), queried by
/// [`LibraryIndex::lookup`], and used by [`super::PathProvider`] to
/// place a dependency that names no source.
#[derive(Clone, Debug, Default)]
pub struct LibraryIndex {
    roots: Vec<String>,
    entries: Vec<LibraryEntry>,
    unnamed: Vec<String>,
}

impl LibraryIndex {
    /// An index over nothing, which places no dependency.
    ///
    /// This is what [`super::PathProvider`] has unless a caller gives it
    /// one, so a project with no `library` line resolves exactly as it
    /// did before this module existed.
    pub fn new() -> Self {
        LibraryIndex::default()
    }

    /// Indexes the manifests a walk of `roots` found.
    ///
    /// Each manifest arrives as `(path, text)` with the path relative to
    /// the project manifest's directory. Entries come out sorted by name
    /// then path, so every list, note and lock file this produces is in
    /// the same order whatever order the filesystem walked in.
    ///
    /// ```
    /// use reticle::ip::LibraryIndex;
    ///
    /// let index = LibraryIndex::from_manifests(
    ///     ["../../ip".to_owned()],
    ///     [(
    ///         "../../ip/usb/uart/reticle.ip".to_owned(),
    ///         "name uart\nversion 1.0.0\n".to_owned(),
    ///     )],
    /// );
    /// let found = index.lookup("uart").expect("the library has it");
    /// assert_eq!(found.dir, "../../ip/usb/uart");
    /// assert_eq!(found.version.as_ref().unwrap().to_string(), "1.0.0");
    /// ```
    pub fn from_manifests(
        roots: impl IntoIterator<Item = String>,
        manifests: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut out = LibraryIndex {
            roots: roots.into_iter().collect(),
            entries: Vec::new(),
            unnamed: Vec::new(),
        };
        for (path, text) in manifests {
            let (name, version) = manifest_identity(&text);
            match name {
                Some(name) => out.entries.push(LibraryEntry {
                    name,
                    version,
                    dir: parent(&path),
                    manifest: path,
                }),
                None => out.unnamed.push(path),
            }
        }
        out.entries
            .sort_by(|a, b| (&a.name, &a.manifest).cmp(&(&b.name, &b.manifest)));
        out.unnamed.sort();
        out
    }

    /// The roots that were searched, in the order they were given.
    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    /// Every indexed package, sorted by name then path.
    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    /// The manifests that declared no name, sorted; none normally.
    pub fn unnamed(&self) -> &[String] {
        &self.unnamed
    }

    /// True when there is nothing to search: no root and no entry.
    ///
    /// A provider asks this to decide whether a dependency with no
    /// source should be looked up by name at all, or left to the older
    /// convention of a sibling directory named after the package.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty() && self.entries.is_empty()
    }

    /// The one package named `name`.
    ///
    /// Fails with [`LibraryProblem::NotInLibrary`] when nothing declares
    /// that name and [`LibraryProblem::Ambiguous`] when two packages do.
    pub fn lookup(&self, name: &str) -> Result<&LibraryEntry, LibraryProblem> {
        let matched: Vec<&LibraryEntry> = self.entries.iter().filter(|e| e.name == name).collect();
        match matched.as_slice() {
            [one] => Ok(one),
            [] => {
                let names: Vec<&str> = self.entries.iter().map(|e| e.name.as_str()).collect();
                Err(LibraryProblem::NotInLibrary {
                    name: name.to_owned(),
                    roots: self.roots.clone(),
                    count: self.entries.len(),
                    suggestion: closest(name, &names).map(str::to_owned),
                })
            }
            several => Err(LibraryProblem::Ambiguous {
                name: name.to_owned(),
                manifests: several.iter().map(|e| e.manifest.clone()).collect(),
            }),
        }
    }

    /// Every name more than one package declares, with their manifests.
    ///
    /// Sorted by name, and each list sorted by path. Empty for a library
    /// that is what it claims to be.
    pub fn duplicates(&self) -> Vec<(String, Vec<String>)> {
        let mut by_name: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for entry in &self.entries {
            by_name
                .entry(entry.name.as_str())
                .or_default()
                .push(entry.manifest.clone());
        }
        by_name
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(name, paths)| (name.to_owned(), paths))
            .collect()
    }
}

/// Why a name could not be placed in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryProblem {
    /// No package under any root declares the name.
    NotInLibrary {
        /// The name that was wanted.
        name: String,
        /// The roots that were searched.
        roots: Vec<String>,
        /// How many packages the index holds, for the note.
        count: usize,
        /// The nearest name the library does have, if one is near.
        suggestion: Option<String>,
    },
    /// Two or more packages declare the name.
    Ambiguous {
        /// The name that was wanted.
        name: String,
        /// Every manifest declaring it, sorted by path.
        manifests: Vec<String>,
    },
}

impl LibraryProblem {
    /// This problem's diagnostic code.
    pub fn code(&self) -> &'static str {
        match self {
            LibraryProblem::NotInLibrary { .. } => NOT_IN_LIBRARY,
            LibraryProblem::Ambiguous { .. } => AMBIGUOUS,
        }
    }

    /// The rustc-style diagnostic, pointing at `span`.
    ///
    /// `span` is the `depends` line that wanted the name: the index
    /// itself has no spans, because the manifests it read are not in the
    /// build's source map and must not be — only the packages a build
    /// actually uses belong there.
    pub fn diagnostic(&self, span: Span) -> Diagnostic {
        match self {
            LibraryProblem::NotInLibrary {
                name,
                roots,
                count,
                suggestion,
            } => {
                let where_ = if roots.is_empty() {
                    "no library root was given".to_owned()
                } else {
                    format!(
                        "searched {} package{} under {}",
                        count,
                        if *count == 1 { "" } else { "s" },
                        roots
                            .iter()
                            .map(|r| format!("`{r}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let mut d =
                    Diagnostic::error(format!("the IP library has no package named `{name}`"))
                        .with_code(NOT_IN_LIBRARY)
                        .with_span(span)
                        .with_note(where_);
                if let Some(near) = suggestion {
                    d = d.with_note(format!("there is a package named `{near}`"));
                }
                d.with_note(
                    "a package outside the library is named with `path <dir>` on the `depends` line",
                )
            }
            LibraryProblem::Ambiguous { name, manifests } => {
                let mut d = Diagnostic::error(format!(
                    "the IP library has {} packages named `{name}`",
                    manifests.len()
                ))
                .with_code(AMBIGUOUS)
                .with_span(span);
                for manifest in manifests {
                    d = d.with_note(format!("`{manifest}` declares it"));
                }
                d.with_note(
                    "one name is one package: move one of them, or say which with `path <dir>`",
                )
            }
        }
    }
}

/// The `name` and `version` a `reticle.ip` declares, or what of them it
/// does.
///
/// A deliberately small reader of the manifest grammar — a keyword, then
/// words, `#` or `//` starting a comment — because this runs over every
/// manifest in a library and reports nothing: whatever is wrong with a
/// manifest is reported with spans by [`super::IpManifest::parse`] if
/// and when the package is used. A second `name` line loses to the
/// first, which is the opposite of the parser's "duplicate" error, and
/// is why a package the index places is parsed properly afterwards
/// rather than trusted from here.
///
/// ```
/// use reticle::ip::library::manifest_identity;
///
/// let (name, version) = manifest_identity("# a uart\nname uart\nversion 1.2.0\n");
/// assert_eq!(name.as_deref(), Some("uart"));
/// assert_eq!(version.unwrap().to_string(), "1.2.0");
/// ```
pub fn manifest_identity(text: &str) -> (Option<String>, Option<Version>) {
    let mut name = None;
    let mut version = None;
    for line in text.lines() {
        let code = line
            .split('#')
            .next()
            .unwrap_or("")
            .split("//")
            .next()
            .unwrap_or("");
        let Some((keyword, rest)) = code.trim_start().split_once(char::is_whitespace) else {
            continue;
        };
        match keyword {
            "name" if name.is_none() => name = first_word(rest),
            "version" if version.is_none() => {
                version = first_word(rest).as_deref().and_then(Version::parse);
            }
            _ => {}
        }
    }
    (name, version)
}

/// The first word of `text`: a quoted run, or a run of non-whitespace.
///
/// The one piece of the tokenizer this scan needs. Escapes are not
/// resolved, because a package name or a version containing a backslash
/// is not a thing that resolves anyway, and whatever is wrong with it
/// will be reported with a span when the manifest is really parsed.
fn first_word(text: &str) -> Option<String> {
    let text = text.trim_start();
    let word = match text.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or(quoted),
        None => text.split_whitespace().next()?,
    };
    (!word.is_empty()).then(|| word.to_owned())
}

/// The path to record for a manifest found at `relative` under the
/// `library` root `root`.
///
/// The caller's walk uses this so that the index, a diagnostic and a
/// lock file all spell one package's path the one way: lexically
/// normalised, relative to the project manifest, exactly as a `path`
/// dependency is written.
///
/// ```
/// use reticle::ip::library::entry_path;
/// assert_eq!(entry_path("../../ip", "usb/uart/reticle.ip"), "../../ip/usb/uart/reticle.ip");
/// assert_eq!(entry_path(".", "uart/reticle.ip"), "uart/reticle.ip");
/// ```
pub fn entry_path(root: &str, relative: &str) -> String {
    super::resolve::join(root, relative)
}

/// The directory part of a path, as the index records it.
fn parent(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir.to_owned(),
        Some(_) => "/".to_owned(),
        None => ".".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(entries: &[(&str, &str)]) -> LibraryIndex {
        LibraryIndex::from_manifests(
            ["ip".to_owned()],
            entries
                .iter()
                .map(|(p, t)| ((*p).to_owned(), (*t).to_owned())),
        )
    }

    #[test]
    fn a_package_nested_two_levels_deep_is_found_by_name() {
        let index = index(&[
            (
                "ip/usb/device/reticle.ip",
                "name usb_device_fs\nversion 1.0.0\n",
            ),
            ("ip/serial/uart/reticle.ip", "name uart\nversion 2.1.0\n"),
        ]);
        let found = index.lookup("uart").expect("found");
        assert_eq!(found.dir, "ip/serial/uart");
        assert_eq!(found.manifest, "ip/serial/uart/reticle.ip");
        assert_eq!(found.version, Some(Version::new(2, 1, 0)));
        // The directory name is not the package name, and the lookup is
        // by the declared name, not by the directory.
        let found = index.lookup("usb_device_fs").expect("found");
        assert_eq!(found.dir, "ip/usb/device");
        assert!(index.lookup("device").is_err());
    }

    #[test]
    fn entries_are_sorted_whatever_order_the_walk_returned() {
        let forwards = index(&[
            ("ip/a/reticle.ip", "name aaa\nversion 1.0.0\n"),
            ("ip/b/reticle.ip", "name bbb\nversion 1.0.0\n"),
        ]);
        let backwards = index(&[
            ("ip/b/reticle.ip", "name bbb\nversion 1.0.0\n"),
            ("ip/a/reticle.ip", "name aaa\nversion 1.0.0\n"),
        ]);
        let names = |i: &LibraryIndex| -> Vec<String> {
            i.entries().iter().map(|e| e.name.clone()).collect()
        };
        assert_eq!(names(&forwards), ["aaa", "bbb"]);
        assert_eq!(names(&forwards), names(&backwards));
    }

    #[test]
    fn a_missing_name_suggests_the_nearest_and_says_where_it_looked() {
        let index = index(&[("ip/uart/reticle.ip", "name uart\nversion 1.0.0\n")]);
        let problem = index.lookup("uarts").expect_err("not in the library");
        assert_eq!(problem.code(), NOT_IN_LIBRARY);
        let rendered = format!("{:?}", problem);
        assert!(rendered.contains("uarts"), "{rendered}");
        match problem {
            LibraryProblem::NotInLibrary {
                suggestion,
                count,
                roots,
                ..
            } => {
                assert_eq!(suggestion.as_deref(), Some("uart"));
                assert_eq!(count, 1);
                assert_eq!(roots, ["ip"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn one_name_from_two_packages_names_both_paths() {
        let index = index(&[
            ("ip/new/uart/reticle.ip", "name uart\nversion 2.0.0\n"),
            ("ip/uart/reticle.ip", "name uart\nversion 1.0.0\n"),
        ]);
        let problem = index.lookup("uart").expect_err("ambiguous");
        assert_eq!(problem.code(), AMBIGUOUS);
        match &problem {
            LibraryProblem::Ambiguous { manifests, .. } => {
                assert_eq!(manifests, &["ip/new/uart/reticle.ip", "ip/uart/reticle.ip"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            index.duplicates(),
            vec![(
                "uart".to_owned(),
                vec![
                    "ip/new/uart/reticle.ip".to_owned(),
                    "ip/uart/reticle.ip".to_owned()
                ]
            )]
        );
    }

    #[test]
    fn a_manifest_with_no_name_is_listed_rather_than_indexed() {
        let index = index(&[
            (
                "ip/broken/reticle.ip",
                "# nothing here yet\nversion 1.0.0\n",
            ),
            ("ip/uart/reticle.ip", "name uart\n"),
        ]);
        assert_eq!(index.unnamed(), ["ip/broken/reticle.ip"]);
        // And a package with no version is still placeable: the real
        // parse reports the missing `version` with a span.
        assert_eq!(index.lookup("uart").unwrap().version, None);
    }

    #[test]
    fn the_identity_scan_follows_the_manifest_grammar() {
        assert_eq!(
            manifest_identity("name \"odd name\"\n"),
            (Some("odd name".to_owned()), None)
        );
        assert_eq!(manifest_identity("# name commented\n"), (None, None));
        assert_eq!(manifest_identity("// name uart\n"), (None, None));
        assert_eq!(manifest_identity("name\n"), (None, None));
        assert_eq!(manifest_identity("name \"\"\n"), (None, None));
        assert_eq!(
            manifest_identity("version 1.0\nname a\n").1,
            None,
            "a malformed version is dropped, not guessed at"
        );
        // The first wins, and the parser reports the duplicate later.
        assert_eq!(
            manifest_identity("name first\nname second\n").0,
            Some("first".to_owned())
        );
    }

    #[test]
    fn an_empty_index_places_nothing() {
        let empty = LibraryIndex::new();
        assert!(empty.is_empty());
        assert!(empty.lookup("uart").is_err());
        assert!(!index(&[("ip/uart/reticle.ip", "name uart\n")]).is_empty());
    }

    #[test]
    fn a_manifest_at_the_top_of_the_walk_has_a_directory() {
        assert_eq!(parent("reticle.ip"), ".");
        assert_eq!(parent("ip/reticle.ip"), "ip");
        assert_eq!(parent("/reticle.ip"), "/");
    }
}
