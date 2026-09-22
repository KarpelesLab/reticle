//! Golden tests for the Verilog preprocessor and lexer.
//!
//! Every `testdata/verilog/lex/<name>.v` (Verilog-2005) or `<name>.sv`
//! (SystemVerilog) is preprocessed and lexed; the token dump plus rendered
//! diagnostics must match `<name>.tokens`. Set `UPDATE_EXPECT=1` to rewrite
//! the expectations after an intended change.
//!
//! Dump format, one token per line:
//!
//! ```text
//! kind@line:col text            token from the root file
//! kind@file:line:col text       token from an included file
//! --- diagnostics               followed by the rustc-style rendering
//! ```
//!
//! `` `include `` paths are looked up in `testdata/verilog/lex/include/`.

#![cfg(feature = "verilog")]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::{SourceId, SourceMap};
use reticle::verilog::{Dialect, IncludeResolver, TokenKind, lex_source};

/// Resolves includes from one directory, naming them by their relative path.
struct DirResolver {
    dir: PathBuf,
}

impl IncludeResolver for DirResolver {
    fn resolve(&mut self, path: &str, _from: SourceId) -> Option<(String, String)> {
        let full = self.dir.join(path);
        let text = fs::read_to_string(&full).ok()?;
        Some((format!("include/{path}"), normalise(&text)))
    }
}

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn dump(path: &Path, dir: &Path) -> String {
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let ext = path.extension().unwrap().to_string_lossy();
    let dialect = Dialect::for_extension(&ext).expect("known extension");

    let mut map = SourceMap::new();
    let id = map.add(name, text).unwrap();
    let mut diags = Diagnostics::new();
    let mut resolver = DirResolver {
        dir: dir.join("include"),
    };
    let tokens = lex_source(&mut map, id, dialect, &mut resolver, &mut diags);

    let mut out = String::new();
    for tok in &tokens {
        let (file, loc) = map.locate(tok.span);
        let _ = write!(out, "{}@", tok.kind.kind_name());
        if tok.span.file != id {
            let _ = write!(out, "{file}:");
        }
        let _ = write!(out, "{loc}");
        match &tok.kind {
            TokenKind::Eof => {}
            TokenKind::Str { value } => {
                let _ = write!(out, " {value:?}");
            }
            kind => {
                let _ = write!(out, " {}", kind.text());
            }
        }
        out.push('\n');
    }
    if !diags.is_empty() {
        diags.sort();
        out.push_str("--- diagnostics\n");
        out.push_str(&diags.render(&map));
    }
    out
}

#[test]
fn golden() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/lex");
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
        let actual = dump(case, &dir);
        let expect_path = case.with_extension("tokens");
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
