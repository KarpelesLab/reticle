//! Golden-file tests for the VHDL lexer.
//!
//! Every `testdata/vhdl/lex/<name>.vhd` is lexed and compared against:
//!
//! - `<name>.tokens`: one token per line, `kind@line:col text`;
//! - `<name>.diag`: the rendered diagnostics, present only when there are
//!   any;
//! - `<name>.comments`: one side-table entry per line, `kind@line:col text`
//!   with line breaks inside a comment written as `\n`, present only when
//!   the file has comments or tool directives.
//!
//! A `.vhd` file whose first line is `-- lex: vhdl93` is lexed in VHDL-93
//! mode. Run with `UPDATE_EXPECT=1` to rewrite the expectation files.

#![cfg(feature = "vhdl")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::vhdl::{CommentKind, Lexer, Standard};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/lex")
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
        "{} differs from the lexer output\n--- expected\n{expected}--- actual\n{actual}",
        path.display()
    ))
}

fn run_one(vhd: &Path) -> Vec<String> {
    let text = fs::read_to_string(vhd).expect("read corpus file");
    let standard = if text.lines().next() == Some("-- lex: vhdl93") {
        Standard::Vhdl93
    } else {
        Standard::Vhdl2008
    };
    let mut map = SourceMap::new();
    let name = vhd.file_name().unwrap().to_string_lossy().into_owned();
    let id = map.add(name, text).unwrap();
    let file = map.file(id);

    let mut diags = Diagnostics::new();
    let lexed = Lexer::new(file.text(), id, standard).lex(&mut diags);

    let mut tokens = String::new();
    for t in &lexed.tokens {
        let loc = file.loc(t.span.start);
        tokens.push_str(&format!("{}@{loc} {}\n", t.kind.name(), t.text()));
    }

    let mut comments = String::new();
    for (span, kind) in &lexed.comments {
        let loc = file.loc(span.start);
        let kind = match kind {
            CommentKind::Line => "Line",
            CommentKind::Delimited => "Delimited",
            CommentKind::ToolDirective => "ToolDirective",
        };
        let text = &file.text()[span.start as usize..span.end as usize];
        comments.push_str(&format!("{kind}@{loc} {}\n", text.replace('\n', "\\n")));
    }

    diags.sort();
    let rendered = diags.render(&map);

    let mut failures = Vec::new();
    for (ext, actual) in [
        ("tokens", tokens),
        ("diag", rendered),
        ("comments", comments),
    ] {
        if let Some(f) = check_golden(&vhd.with_extension(ext), &actual) {
            failures.push(f);
        }
    }
    failures
}

#[test]
fn golden_corpus() {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("testdata/vhdl/lex exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    files.sort();
    assert!(files.len() >= 10, "the lexer corpus is too small");

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
