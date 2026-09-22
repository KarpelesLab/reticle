//! Golden tests for the Verilog parser.
//!
//! Every `testdata/verilog/parse/<name>.v` (Verilog-2005) or `<name>.sv`
//! (SystemVerilog) is preprocessed, lexed and parsed; the AST dump (see
//! `reticle::verilog::ast_dump`) plus rendered diagnostics must match
//! `<name>.ast`. Set `UPDATE_EXPECT=1` to rewrite the expectations after an
//! intended change.
//!
//! Dump format: one node per line, children indented, each item and
//! statement suffixed with `@line:col`; a `--- diagnostics` section follows
//! when the parser reported anything.

#![cfg(feature = "verilog")]
#![cfg(feature = "verilog")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, NoIncludes, ast_dump, parse_source};

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn dump(path: &Path) -> String {
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let ext = path.extension().unwrap().to_string_lossy();
    let dialect = Dialect::for_extension(&ext).expect("known extension");

    let mut map = SourceMap::new();
    let id = map.add(name, text).unwrap();
    let mut diags = Diagnostics::new();
    let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut diags);

    let mut out = ast_dump::dump_with_locations(&file, &map);
    if !diags.is_empty() {
        diags.sort();
        out.push_str("--- diagnostics\n");
        out.push_str(&diags.render(&map));
    }
    out
}

#[test]
fn golden() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/parse");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut cases: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv")))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no corpus files in {}", dir.display());

    let mut failures = Vec::new();
    for case in &cases {
        let actual = dump(case);
        let expect_path = case.with_extension("ast");
        let expected = fs::read_to_string(&expect_path).map(|t| normalise(&t));
        match expected {
            Ok(expected) if expected == actual => {}
            _ if update => {
                fs::write(&expect_path, &actual).expect("write expectation");
                eprintln!("updated {}", expect_path.display());
            }
            Ok(expected) => {
                let line = expected
                    .lines()
                    .zip(actual.lines())
                    .position(|(a, b)| a != b)
                    .map_or(
                        expected.lines().count().min(actual.lines().count()) + 1,
                        |i| i + 1,
                    );
                let exp_line = expected.lines().nth(line - 1).unwrap_or("<end>");
                let act_line = actual.lines().nth(line - 1).unwrap_or("<end>");
                failures.push(format!(
                    "{}: mismatch at line {line}\n  expected: {exp_line}\n  actual:   {act_line}",
                    case.display()
                ));
            }
            Err(_) => failures.push(format!(
                "{}: missing expectation {} (run with UPDATE_EXPECT=1)",
                case.display(),
                expect_path.display()
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} golden test(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The lexer corpus is full of deliberately broken input (bad literals,
/// stray characters, unterminated strings); the parser must get through
/// all of it without panicking or looping.
#[test]
fn survives_lexer_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/lex");
    let mut count = 0;
    for entry in fs::read_dir(&dir).expect("lexer corpus directory") {
        let path = entry.expect("dir entry").path();
        if !matches!(path.extension().and_then(|e| e.to_str()), Some("v" | "sv")) {
            continue;
        }
        let text = normalise(&fs::read_to_string(&path).expect("read corpus file"));
        let ext = path.extension().unwrap().to_string_lossy();
        let dialect = Dialect::for_extension(&ext).expect("known extension");
        let mut map = SourceMap::new();
        let id = map.add(path.to_string_lossy().into_owned(), text).unwrap();
        let mut diags = Diagnostics::new();
        let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut diags);
        // The dump must not panic either.
        let _ = ast_dump::dump_with_locations(&file, &map);
        count += 1;
    }
    assert!(count > 0, "no lexer corpus files found");
}

/// Error-recovery cases must report something, and clean cases nothing;
/// the file name prefix `error_` says which is which.
#[test]
fn error_cases_have_diagnostics() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/parse");
    let mut seen_error_case = false;
    for entry in fs::read_dir(&dir).expect("corpus directory") {
        let path = entry.expect("dir entry").path();
        if !matches!(path.extension().and_then(|e| e.to_str()), Some("v" | "sv")) {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let has_diags = dump(&path).contains("--- diagnostics");
        if name.starts_with("error_") {
            seen_error_case = true;
            assert!(has_diags, "{name} should report diagnostics");
        } else {
            assert!(!has_diags, "{name} should parse cleanly");
        }
    }
    assert!(seen_error_case, "no error-recovery cases in the corpus");
}
