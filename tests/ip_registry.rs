//! The registry index over the packages in `testdata/ip/packages`.
//!
//! The unit tests in `src/ip/registry.rs` cover the format, the search
//! and the rewrite in detail; this file is the other half of the claim:
//! that an index built from real manifests resolves a real project, and
//! that what it writes out is a directory tree somebody could commit.
//!
//! | File | What it holds |
//! |------|----------------|
//! | `testdata/ip/registry/index` | every package line, as one file |
//! | `testdata/ip/registry/layout` | the paths those lines are split into |
//!
//! Set `UPDATE_EXPECT=1` to rewrite them after an intended change.

#![cfg(feature = "ip")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ip::registry::{Index, IndexEntry, ProjectFile, RegistryProvider};
use reticle::ip::{self, IpManifest, VersionReq};
use reticle::source::SourceMap;

/// The packages the index is built from, in name order.
const PACKAGES: [&str; 5] = [
    "axi_regs",
    "cdc_sync",
    "fifo_sync",
    "gray_counter",
    "uart_lite",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ip")
}

/// The one place this test suite touches the filesystem.
fn read(path: &str) -> Option<String> {
    fs::read_to_string(dir().join(path)).ok()
}

/// Compares `actual` with the file `name`, rewriting it under
/// `UPDATE_EXPECT`.
fn expect(name: &str, actual: &str, failures: &mut Vec<String>) {
    let path = dir().join(name);
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&path, actual).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        return;
    }
    failures.push(format!(
        "{name} differs\n--- expected ---\n{expected}--- actual ---\n{actual}"
    ));
}

/// A stand-in for a real checksum.
///
/// FNV-1a, written here rather than in the crate: what an index's
/// checksums are computed with is the publisher's business, and Reticle
/// ships no hash function (see `src/ip/registry.rs`). It only has to be
/// deterministic, which is what makes the golden file stable.
fn checksum(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// An index summarising every package under `testdata/ip/packages`.
fn build() -> Index {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut index = Index::new();
    for name in PACKAGES {
        let path = format!("packages/{name}/reticle.ip");
        let text = read(&path).unwrap_or_else(|| panic!("no {path}"));
        let file = map.add(path.clone(), &text).expect("it fits a source map");
        let manifest = IpManifest::parse(&text, file, &mut diags)
            .unwrap_or_else(|| panic!("{path} does not parse"));
        index.insert(IndexEntry::from_manifest(&manifest, checksum(&text)));
    }
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    index
}

#[test]
fn the_index_matches_the_golden_files() {
    let index = build();
    let mut failures = Vec::new();
    expect("registry/index", &index.to_text(), &mut failures);

    let mut layout = String::new();
    for (path, text) in index.files() {
        layout.push_str(&format!("{path}: {} line(s)\n", text.lines().count()));
    }
    expect("registry/layout", &layout, &mut failures);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));

    // What was written out reads back as the same index.
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut again = Index::new();
    for (path, text) in index.files() {
        let file = map.add(path, &text).unwrap();
        for entry in Index::parse(&text, file, &mut diags).entries() {
            again.insert(entry.clone());
        }
    }
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    assert_eq!(again.to_text(), index.to_text());
}

#[test]
fn a_project_resolves_and_elaborates_through_the_index() {
    let index = build();
    // `uart_lite` needs `fifo_sync`, which needs `cdc_sync`, and the
    // project names none of that: the index's own summaries carry the
    // graph, and the provider lays each package out under
    // `<name>-<version>`.
    let text = "\
name blinky
top top

source rtl/top.v

depends uart_lite ^1.2.0 registry
";
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project =
        ip::load_project(&mut map, "reticle.proj", text, &mut diags).expect("the project parses");

    let mut provider = RegistryProvider::new(&index, |path: &str| {
        // `<name>-<version>/<file>` is where the registry says a
        // package's files are; here they are still in the packages
        // directory, which is exactly the translation a real fetcher
        // does between a cache layout and its own. Anything else is the
        // project's own source, under the root below.
        if let Some((package, rest)) = path.split_once('/')
            && let Some((name, _version)) = package.rsplit_once('-')
            && !name.is_empty()
        {
            return read(&format!("packages/{name}/{rest}"));
        }
        read(path)
    })
    .with_root("projects/two_deps");

    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    assert!(
        resolved.is_complete(),
        "{}",
        diags.render(resolved.source_map())
    );
    let names: Vec<String> = resolved
        .packages
        .iter()
        .map(|p| format!("{} {}", p.name(), p.version()))
        .collect();
    assert_eq!(
        names,
        vec!["cdc_sync 0.3.1", "fifo_sync 1.0.4", "uart_lite 1.2.0"]
    );
    // Every one of them came from the registry, and the lock file says
    // so, which is what makes the build reproducible.
    for package in &resolved.packages {
        assert_eq!(
            resolved.lock.package(package.name()).unwrap().origin,
            reticle::ip::DepSource::Registry
        );
    }

    let design = ip::elaborate_project(&project, &mut resolved, &mut diags).expect("it builds");
    assert!(
        !diags.has_errors(),
        "{}",
        diags.render(resolved.source_map())
    );
    for module in ["top", "uart_lite", "fifo_sync", "cdc_sync"] {
        assert!(
            design.module_by_name(module).is_some(),
            "`{module}` is not in the design"
        );
    }
}

#[test]
fn add_writes_a_line_into_a_real_project_manifest() {
    let index = build();
    let path = "projects/two_deps/reticle.proj";
    let text = read(path).unwrap_or_else(|| panic!("no {path}"));
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project = ip::load_project(&mut map, path, &text, &mut diags).expect("it parses");

    let req = VersionReq::parse("^0.2.0").expect("a requirement");
    let added = reticle::ip::registry::add(
        &ProjectFile::new(&project, &text),
        "gray_counter",
        &req,
        &index,
    )
    .expect("it adds");

    // The file keeps its comment block and its spacing, and gains one
    // line. The `depends` lines there are not in name order, so the new
    // one goes after the last of them rather than being sorted in.
    assert!(
        added.text.starts_with("# A complete project"),
        "{}",
        added.text
    );
    assert_eq!(added.text.lines().count(), text.lines().count() + 1);
    assert_eq!(added.line, "depends gray_counter ^0.2.0 registry");
    assert_eq!(added.at, text.lines().count() + 1);
    assert_eq!(
        added.text.lines().nth(added.at - 1),
        Some(added.line.as_str())
    );
    for line in text.lines() {
        assert!(added.text.contains(line), "`{line}` was lost");
    }

    // And the rewritten text parses, with the new dependency in it.
    let mut diags = Diagnostics::new();
    let reparsed = ip::load_project(&mut map, "rewritten.proj", &added.text, &mut diags)
        .expect("the rewrite parses");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let dep = reparsed.dependency("gray_counter").expect("it is there");
    assert_eq!(dep.req, req);
    assert_eq!(dep.source, Some(reticle::ip::DepSource::Registry));
}
