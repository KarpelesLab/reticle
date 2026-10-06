//! The walk that finds an IP library's manifests, shared by the tests
//! that resolve a project against `ip/`.
//!
//! Walking a directory is I/O and `reticle::ip` does none, so the walk
//! belongs to whoever has a filesystem: the CLI (`library_manifests` in
//! `src/bin/reticle/main.rs`) or a test. This is that walk for the tests,
//! written once rather than once per test binary — the copy that was in
//! `tests/mos6502_computer.rs` was the only one, and three more example
//! tests needed it the moment their manifests dropped their `path` lines.
//!
//! It matches the CLI's in every rule that can change an answer:
//!
//! - a directory holding a `reticle.ip` **is** a package and is not
//!   descended into, so the walk never enters an `rtl/` or a `tb/`;
//! - a directory whose name starts with `.` is not entered;
//! - `file_type` does not follow a link, so a linked directory is skipped
//!   rather than walked and a loop cannot hang it;
//! - every level is sorted, because `read_dir` returns filesystem order
//!   and this reaches a lock file;
//! - eight levels deep and no further.
//!
//! What it does *not* do is read a manifest's contents: that is
//! `LibraryIndex::from_manifests`, which scans for `name` and `version`
//! and is the part under test.

// Each test binary uses a different half of this.
#![allow(dead_code)]
#![allow(unreachable_pub)]

use std::fs;
use std::path::Path;

use reticle::ip::library::{MANIFEST_NAME, entry_path};
use reticle::ip::{LibraryIndex, Project};

/// As deep as the walk goes, the same limit the CLI applies.
const DEPTH: usize = 8;

/// Every `reticle.ip` at or under `dir`, as the `(path, text)` pairs
/// `LibraryIndex::from_manifests` wants, sorted.
///
/// `prefix` is written in front of every path, so it is the root
/// *as the project manifest spells it* — `../../ip`, or `ip` for a test
/// that works from the repository root. `dir` is where that resolves to
/// on this machine.
pub fn manifests(dir: &Path, prefix: &str) -> Vec<(String, String)> {
    fn descend(at: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, String)>) {
        let manifest = at.join(MANIFEST_NAME);
        if manifest.is_file() {
            let text = fs::read_to_string(&manifest)
                .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
            out.push((entry_path(prefix, MANIFEST_NAME), text));
            return;
        }
        if depth >= DEPTH {
            return;
        }
        let mut names: Vec<String> = fs::read_dir(at)
            .unwrap_or_else(|e| panic!("{}: {e}", at.display()))
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                (entry.file_type().ok()?.is_dir() && !name.starts_with('.')).then_some(name)
            })
            .collect();
        names.sort();
        for name in names {
            descend(&at.join(&name), &entry_path(prefix, &name), depth + 1, out);
        }
    }

    let mut out = Vec::new();
    descend(dir, prefix, 0, &mut out);
    out.sort();
    out
}

/// The index a project's `library` lines describe.
///
/// `dir` holds the project manifest, which is what each root is relative
/// to. A project with no `library` line gets an empty index, which is a
/// provider that places every dependency by its `path` — that is
/// `examples/soc`, and it is deliberate.
pub fn index(dir: &Path, project: &Project) -> LibraryIndex {
    let mut found = Vec::new();
    for root in &project.libraries {
        found.extend(manifests(&dir.join(root), root));
    }
    found.sort();
    LibraryIndex::from_manifests(project.libraries.clone(), found)
}
