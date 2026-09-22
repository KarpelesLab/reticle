//! The documentation half, driven end to end from Verilog source.
//!
//! The golden corpus in `tests/viewer_golden.rs` is IR text, which carries
//! no comments, so it exercises the layout but not the prose. This one
//! elaborates `testdata/viewer/*.v`, collects the comments out of the
//! lexer's side table the way `reticle viewer` does, and compares the
//! resulting site with `testdata/viewer/<name>/`.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations.

#![cfg(all(feature = "viewer", feature = "verilog"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, Lexer, NoIncludes, elaborate, parse_source};
use reticle::viewer::{Comments, Site, ViewerOptions, render};

fn sources() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/viewer");
    let mut out: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v" || e == "sv"))
        .collect();
    out.sort();
    out
}

/// Elaborates one Verilog file and renders its site, comments included.
fn site_of(path: &Path) -> Site {
    let text = fs::read_to_string(path).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let mut map = SourceMap::new();
    let file = map.add(name.clone(), text).unwrap();

    let mut diags = Diagnostics::new();
    let parsed = parse_source(
        &mut map,
        file,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let options = ElabOptions::new(Dialect::Verilog2005);
    let design = elaborate(&[&parsed], &options, &mut diags).expect("elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    // The comments live in the lexer's side table, not in the tree; this
    // is the same second pass the CLI makes.
    let mut comments = Comments::new();
    let mut lex_diags = Diagnostics::new();
    let lexed = Lexer::new(
        map.file(file).text(),
        file,
        Dialect::Verilog2005,
        &mut lex_diags,
    )
    .run();
    comments.add_verilog(&lexed);
    assert!(!comments.is_empty(), "{name} has no comments to show");

    render(
        &design,
        &map,
        &ViewerOptions {
            title: format!("{name} \u{2014} design reference"),
            comments,
            ..ViewerOptions::default()
        },
    )
}

#[test]
fn golden_documentation() {
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/viewer");
    let sources = sources();
    assert!(!sources.is_empty(), "no Verilog fixtures found");

    let mut failures = Vec::new();
    for path in &sources {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let site = site_of(path);
        let dir = root.join(&stem);
        let produced: BTreeSet<&str> = site.paths().collect();
        assert!(
            produced.contains("index.html"),
            "{stem}: no index was rendered"
        );

        if update {
            for (file, contents) in &site.files {
                let target = dir.join(file);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(&target, contents).unwrap();
            }
            continue;
        }
        for (file, contents) in &site.files {
            match fs::read_to_string(dir.join(file)) {
                Ok(text) if text == *contents => {}
                Ok(_) => failures.push(format!("{stem}/{file}: differs")),
                Err(_) => failures.push(format!("{stem}/{file}: missing")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "set UPDATE_EXPECT=1 to rewrite:\n{}",
        failures.join("\n")
    );
}

/// The prose really comes from the comments: the block above a module is
/// its description, and the comment after a port is that port's note.
#[test]
fn comments_reach_the_page() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/viewer/documented.v");
    let site = site_of(&path);
    let page = site.get("documented.doc.html").expect("a page per module");
    assert!(page.contains("A small register file wrapper"));
    assert!(page.contains("rising-edge clock for every port"));
    assert!(page.contains("active-low asynchronous reset"));
    assert!(page.contains("registered read port"));
    assert!(page.contains("inference turns this into a memory"));
    // The adder's own description, not the register file's.
    let adder = site.get("documented_adder.doc.html").unwrap();
    assert!(adder.contains("A plain adder"));
    assert!(!adder.contains("register file wrapper"));
    assert!(adder.contains("sum, truncated to W bits"));
    // Locations link into the listing.
    assert!(page.contains("href=\"source/0-documented.v.html#L"));
}
