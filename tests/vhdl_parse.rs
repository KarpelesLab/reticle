//! Golden-file tests for the VHDL parser.
//!
//! Every `testdata/vhdl/parse/<name>.vhd` is lexed and parsed and compared
//! against:
//!
//! - `<name>.ast`: the AST as rendered by `reticle::vhdl::ast_dump::dump`;
//! - `<name>.diag`: the rendered diagnostics, present only when there are
//!   any.
//!
//! A `.vhd` file whose first line is `-- parse: vhdl93` is parsed in
//! VHDL-93 mode. Run with `UPDATE_EXPECT=1` to rewrite the expectation
//! files.

#![cfg(feature = "vhdl")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::vhdl::{Standard, ast_dump, parse_source};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/parse")
}

/// Compares `actual` with the golden file at `path`, where an empty
/// `actual` means the file must not exist. With `UPDATE_EXPECT=1` the file
/// is written (or removed) instead. Returns a description of the mismatch.
fn check_golden(path: &Path, actual: &str) -> Option<String> {
    let update = std::env::var_os("UPDATE_EXPECT").is_some_and(|v| v == "1");
    if update {
        if actual.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, actual).expect("write expectation");
        }
        return None;
    }
    let expected = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => panic!("reading {}: {e}", path.display()),
    };
    if expected == actual {
        return None;
    }
    Some(format!(
        "{} differs from the parser output\n--- expected\n{expected}--- actual\n{actual}",
        path.display()
    ))
}

fn run_one(vhd: &Path) -> Vec<String> {
    let text = fs::read_to_string(vhd).expect("read corpus file");
    let standard = if text.lines().next() == Some("-- parse: vhdl93") {
        Standard::Vhdl93
    } else {
        Standard::Vhdl2008
    };
    let mut map = SourceMap::new();
    let name = vhd.file_name().unwrap().to_string_lossy().into_owned();
    let id = map.add(name, text).unwrap();

    let mut diags = Diagnostics::new();
    let file = parse_source(&map, id, standard, &mut diags);
    let ast = ast_dump::dump(&file, map.file(id));

    diags.sort();
    let rendered = diags.render(&map);

    let mut failures = Vec::new();
    for (ext, actual) in [("ast", ast), ("diag", rendered)] {
        if let Some(f) = check_golden(&vhd.with_extension(ext), &actual) {
            failures.push(f);
        }
    }
    failures
}

#[test]
fn golden_corpus() {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("testdata/vhdl/parse exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    files.sort();
    assert!(files.len() >= 25, "the parser corpus is too small");

    let mut failures = Vec::new();
    for f in &files {
        failures.extend(run_one(f));
    }
    if !failures.is_empty() {
        panic!(
            "{} golden mismatch(es); run with UPDATE_EXPECT=1 to accept:\n\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// Every corpus file that is not an error-recovery case must parse
/// without errors, and every error case must have at least one.
#[test]
fn error_cases_are_the_only_ones_with_errors() {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("testdata/vhdl/parse exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    files.sort();
    for vhd in files {
        let text = fs::read_to_string(&vhd).unwrap();
        let mut map = SourceMap::new();
        let name = vhd.file_name().unwrap().to_string_lossy().into_owned();
        let is_error_case = name.starts_with("err_");
        let standard = if text.lines().next() == Some("-- parse: vhdl93") {
            Standard::Vhdl93
        } else {
            Standard::Vhdl2008
        };
        let id = map.add(name.clone(), text).unwrap();
        let mut diags = Diagnostics::new();
        let _ = parse_source(&map, id, standard, &mut diags);
        assert_eq!(
            diags.has_errors(),
            is_error_case,
            "{name}: unexpected error status\n{}",
            diags.render(&map)
        );
    }
}
