//! Golden tests for the IP-XACT import.
//!
//! Each case under `testdata/ip/ipxact/` is one component description,
//! written by hand rather than produced by a vendor tool, taken through
//! [`ip::ipxact::import`] and compared against three files:
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<case>.ip` | the `reticle.ip` the import produced |
//! | `<case>.report` | what was translated, approximated and dropped |
//! | `<case>.diag` | everything the import reported |
//!
//! The three cases are the three shapes a catalogue comes in:
//!
//! - `simple` — a plain component in the 1685-2014 spelling, with one
//!   file set, two parameters and a vector whose bounds are an
//!   expression.
//! - `axil_gpio` — an AXI4-Lite peripheral: a bus interface with port
//!   maps, a memory map, a byte strobe whose width is `DATA_WIDTH/8`,
//!   and a second interface on a bus Reticle has no definition for, so
//!   the import has something to drop.
//! - `legacy_uart` — the older 1685-2009 `spirit` spelling, with a VLNV
//!   version that is not a semantic version, `modelParameters`, a
//!   `vector` straight under `wire`, and a bound with a function call in
//!   it that cannot be evaluated.
//!
//! The last test is the one that matters most: an imported package is
//! resolved and elaborated through `ip::resolve` and `ip::elaborate`,
//! which is the proof that what came out is a package and not just a
//! plausible-looking file.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.

#![cfg(feature = "ip")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ip::ipxact::{self, ImportOptions};
use reticle::ip::{self, IpManifest, PathProvider};
use reticle::source::SourceMap;

/// Every component under `testdata/ip/ipxact/`.
const CASES: [&str; 3] = ["simple", "axil_gpio", "legacy_uart"];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ip/ipxact")
}

/// The one place this test suite touches the filesystem.
fn read(path: &str) -> Option<String> {
    fs::read_to_string(dir().join(path)).ok()
}

/// Compares `actual` with the file `name`, rewriting it under
/// `UPDATE_EXPECT`. An absent file counts as empty.
fn expect(name: &str, actual: &str, failures: &mut Vec<String>) {
    let path = dir().join(name);
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        if actual.is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        }
        return;
    }
    failures.push(format!(
        "{name} differs\n--- expected ---\n{expected}--- actual ---\n{actual}"
    ));
}

/// Imports one case, with its diagnostics rendered.
fn import(case: &str) -> (ipxact::ImportedIp, String) {
    let name = format!("{case}.xml");
    let xml = read(&name).unwrap_or_else(|| panic!("no {name}"));
    let mut map = SourceMap::new();
    let file = map
        .add(name.clone(), &xml)
        .expect("the file fits a source map");
    let mut diags = Diagnostics::new();
    let imported = ipxact::import(&xml, &ImportOptions::new(file), &mut diags)
        .unwrap_or_else(|| panic!("{name} does not import:\n{}", diags.render(&map)));
    (imported, diags.render(&map))
}

#[test]
fn imports_match_the_golden_manifests() {
    let mut failures = Vec::new();
    for case in CASES {
        let (imported, diagnostics) = import(case);
        expect(
            &format!("{case}.ip"),
            &imported.manifest.to_text(),
            &mut failures,
        );
        expect(
            &format!("{case}.report"),
            &imported.report.describe(),
            &mut failures,
        );
        expect(&format!("{case}.diag"), &diagnostics, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn every_imported_manifest_reads_back() {
    // An import that produces a file `IpManifest::parse` cannot read, or
    // that does not render the same way twice, is a bug however good the
    // golden file looks.
    for case in CASES {
        let (imported, _) = import(case);
        let text = imported.manifest.to_text();
        let mut map = SourceMap::new();
        let file = map.add("reticle.ip", &text).unwrap();
        let mut diags = Diagnostics::new();
        let again = IpManifest::parse(&text, file, &mut diags)
            .unwrap_or_else(|| panic!("{case}: the imported manifest does not parse"));
        assert!(!diags.has_errors(), "{case}:\n{}", diags.render(&map));
        assert_eq!(
            again.to_text(),
            text,
            "{case}: the manifest does not round trip"
        );
        // The spans differ — the import's point into the XML, the
        // reparse's into the manifest — so everything else is compared
        // instead.
        assert_eq!(again.name, imported.manifest.name);
        assert_eq!(again.version, imported.manifest.version);
        assert_eq!(again.ports.len(), imported.manifest.ports.len());
        assert_eq!(again.params.len(), imported.manifest.params.len());
        assert_eq!(again.sources.len(), imported.manifest.sources.len());
        assert_eq!(
            again.interfaces.len(),
            imported.manifest.interfaces.len(),
            "{case}"
        );
    }
}

#[test]
fn an_imported_package_resolves_and_elaborates() {
    // The proof that the import is usable: take `simple.xml`, hand the
    // manifest it produced to the resolver as `reticle.ip`, and build
    // the design. The package's directory is `testdata/ip/ipxact`, where
    // the RTL the file set names really is.
    let (imported, _) = import("simple");
    let manifest = imported.manifest.to_text();
    assert_eq!(imported.manifest.name, "counter");
    assert_eq!(imported.manifest.top.as_deref(), Some("counter"));

    let project_text = "\
name demo
top demo_top

source demo_top.v

depends counter ^1.0.0 path .
";
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project = ip::load_project(&mut map, "reticle.proj", project_text, &mut diags)
        .expect("the project parses");

    let mut provider = PathProvider::new("", |path: &str| {
        if path == "reticle.ip" {
            Some(manifest.clone())
        } else {
            read(path)
        }
    });
    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    assert!(
        resolved.is_complete(),
        "{}",
        diags.render(resolved.source_map())
    );
    let package = resolved.package("counter").expect("the package resolved");
    assert_eq!(package.version().to_string(), "1.0.0");
    assert!(package.sources[0].is_readable(), "the RTL was not read");

    let elaboration = ip::elaborate(&project, &mut resolved, &mut diags);
    let design = elaboration.design.expect("the design is built");
    assert!(
        !diags.has_errors(),
        "{}",
        diags.render(resolved.source_map())
    );
    assert_eq!(design.top_module().expect("a top").name, "demo_top");
    assert!(
        design.module_by_name("counter").is_some(),
        "the imported package's module is not in the design"
    );
    assert!(
        elaboration.blackboxes.is_empty(),
        "nothing should have been black boxed: {:?}",
        elaboration.blackboxes
    );
}
