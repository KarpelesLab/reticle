//! Golden and property tests for the VHDL formatter.
//!
//! Every `testdata/vhdl/format/<name>.vhd` is formatted and compared
//! against `<name>.fmt.vhd`; set `UPDATE_EXPECT=1` to rewrite the
//! expectations after an intended change.
//!
//! On top of the goldens, three properties are checked over both that
//! corpus and the parser corpus in `testdata/vhdl/parse`:
//!
//! - **idempotence**: formatting a formatted file changes nothing;
//! - **semantic preservation**: the tree is the same before and after,
//!   ignoring the positions the dump carries;
//! - **comment preservation**: the multiset of comment texts is unchanged.
//!
//! Files the formatter refuses (`err_*` in the parser corpus, and the PSL
//! case, whose directives the parser skips) are left out: the formatter
//! reports them instead of rewriting them.

#![cfg(feature = "vhdl")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::vhdl::format::{FormatOptions, format_source};
use reticle::vhdl::{Lexer, Standard, ast_dump, parse_source};

fn testdata(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(dir)
}

/// The `.vhd` inputs of a corpus directory, in a stable order, leaving out
/// the `.fmt.vhd` expectations and the deliberate error cases.
fn inputs(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .filter(|p| !name(p).ends_with(".fmt.vhd"))
        .filter(|p| !name(p).starts_with("err_"))
        .collect();
    files.sort();
    files
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Reads a corpus file.
///
/// A case whose name starts with `crlf` is fed to the formatter with CRLF
/// line endings whatever the checkout did to it, so the test means the same
/// thing on every platform.
fn read_input(path: &Path) -> String {
    let text = fs::read_to_string(path).expect("read corpus file");
    if name(path).starts_with("crlf") {
        text.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        text
    }
}

/// The parser corpus marks VHDL-93 files with a first-line comment.
fn standard_of(text: &str) -> Standard {
    if text.lines().next().map(str::trim_end) == Some("-- parse: vhdl93") {
        Standard::Vhdl93
    } else {
        Standard::Vhdl2008
    }
}

fn format(text: &str) -> Option<String> {
    format_source(text, standard_of(text), &FormatOptions::default()).ok()
}

/// The AST dump with every `@line:col` replaced, so two trees compare
/// equal when only their positions differ.
fn dump_without_positions(text: &str) -> String {
    let mut map = SourceMap::new();
    let id = map.add("x.vhd", text).expect("source fits");
    let mut diags = Diagnostics::new();
    let file = parse_source(&map, id, standard_of(text), &mut diags);
    strip_positions(&ast_dump::dump(&file, map.file(id)))
}

fn strip_positions(dump: &str) -> String {
    let mut out = String::new();
    let mut chars = dump.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '@' {
            out.push(c);
            continue;
        }
        let line: String = take_digits(&mut chars);
        if line.is_empty() || chars.peek() != Some(&':') {
            out.push('@');
            out.push_str(&line);
            continue;
        }
        chars.next();
        let col: String = take_digits(&mut chars);
        if col.is_empty() {
            out.push('@');
            out.push_str(&line);
            out.push(':');
        } else {
            out.push_str("@_:_");
        }
    }
    out
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::new();
    while let Some(c) = chars.peek().copied() {
        if c.is_ascii_digit() {
            out.push(c);
            chars.next();
        } else {
            break;
        }
    }
    out
}

/// Every comment in the text, sorted, so the two sides compare as
/// multisets rather than in order.
fn comment_texts(text: &str) -> Vec<String> {
    let mut map = SourceMap::new();
    let id = map.add("x.vhd", text).expect("source fits");
    let mut diags = Diagnostics::new();
    let src = map.file(id).text();
    let lexed = Lexer::new(src, id, standard_of(text)).lex(&mut diags);
    let mut out: Vec<String> = lexed
        .comments
        .iter()
        .map(|(span, _)| {
            let start = usize::try_from(span.start).expect("offset fits usize");
            let end = usize::try_from(span.end).expect("offset fits usize");
            src[start..end].trim_end().to_owned()
        })
        .collect();
    out.sort();
    out
}

#[test]
fn golden() {
    let dir = testdata("testdata/vhdl/format");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let cases = inputs(&dir);
    assert!(
        cases.len() >= 15,
        "the formatter corpus is too small: {} cases",
        cases.len()
    );

    let mut failures = Vec::new();
    for case in &cases {
        let text = read_input(case);
        let Some(actual) = format(&text) else {
            failures.push(format!("{}: refused to format", case.display()));
            continue;
        };
        let expect_path = case.with_extension("fmt.vhd");
        if update {
            fs::write(&expect_path, &actual).expect("write expectation");
            continue;
        }
        match fs::read_to_string(&expect_path) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => failures.push(format!(
                "{} differs\n--- expected\n{expected}--- actual\n{actual}",
                expect_path.display()
            )),
            Err(_) => failures.push(format!(
                "{}: missing expectation (run with UPDATE_EXPECT=1)",
                expect_path.display()
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} golden mismatch(es):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every corpus file, from both the formatter's own corpus and the
/// parser's.
fn all_inputs() -> Vec<PathBuf> {
    let mut files = inputs(&testdata("testdata/vhdl/format"));
    files.extend(inputs(&testdata("testdata/vhdl/parse")));
    files
}

#[test]
fn formatting_is_idempotent() {
    let mut failures = Vec::new();
    for case in all_inputs() {
        let text = read_input(&case);
        let Some(once) = format(&text) else { continue };
        let Some(twice) = format(&once) else {
            failures.push(format!("{}: reformatting failed", name(&case)));
            continue;
        };
        if once != twice {
            let line = once
                .lines()
                .zip(twice.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            failures.push(format!(
                "{}: not idempotent at line {}\n  once:  {:?}\n  twice: {:?}",
                name(&case),
                line + 1,
                once.lines().nth(line).unwrap_or("<end>"),
                twice.lines().nth(line).unwrap_or("<end>"),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn formatting_preserves_the_tree() {
    let mut failures = Vec::new();
    for case in all_inputs() {
        let text = read_input(&case);
        let Some(formatted) = format(&text) else {
            continue;
        };
        let before = dump_without_positions(&text);
        let after = dump_without_positions(&formatted);
        if before != after {
            let line = before
                .lines()
                .zip(after.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            failures.push(format!(
                "{}: tree changed at dump line {}\n  before: {}\n  after:  {}",
                name(&case),
                line + 1,
                before.lines().nth(line).unwrap_or("<end>"),
                after.lines().nth(line).unwrap_or("<end>"),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn formatting_preserves_comments() {
    let mut failures = Vec::new();
    for case in all_inputs() {
        let text = read_input(&case);
        let Some(formatted) = format(&text) else {
            continue;
        };
        let before = comment_texts(&text);
        let after = comment_texts(&formatted);
        if before != after {
            failures.push(format!(
                "{}: comments changed\n  before: {before:?}\n  after:  {after:?}",
                name(&case)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The formatter must never rewrite a file it could not parse, and must
/// never silently drop source it does not model.
#[test]
fn broken_and_lossy_files_are_refused() {
    let dir = testdata("testdata/vhdl/parse");
    let mut errors = 0;
    for entry in fs::read_dir(&dir).expect("parser corpus") {
        let path = entry.expect("dir entry").path();
        if !path.extension().is_some_and(|e| e == "vhd") || !name(&path).starts_with("err_") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read corpus file");
        assert!(
            format_source(&text, standard_of(&text), &FormatOptions::default()).is_err(),
            "{}: should not be formatted",
            name(&path)
        );
        errors += 1;
    }
    assert!(errors > 0, "no error cases in the parser corpus");

    let psl = fs::read_to_string(dir.join("psl.vhd")).expect("read psl.vhd");
    assert!(
        format_source(&psl, Standard::Vhdl2008, &FormatOptions::default()).is_err(),
        "psl.vhd holds directives the parser skips and must not be rewritten"
    );
}

/// Every other parser corpus file must format, so that the three property
/// tests above cannot start skipping files unnoticed.
#[test]
fn the_parser_corpus_formats() {
    // Files the formatter is expected to refuse; `psl.vhd` holds
    // directives the parser skips.
    const REFUSED: &[&str] = &["psl.vhd"];

    let mut failures = Vec::new();
    let mut checked = 0;
    for case in inputs(&testdata("testdata/vhdl/parse")) {
        let text = read_input(&case);
        let formatted = format_source(&text, standard_of(&text), &FormatOptions::default());
        if REFUSED.contains(&name(&case).as_str()) {
            assert!(
                formatted.is_err(),
                "{} now formats; drop it from REFUSED",
                name(&case)
            );
            continue;
        }
        checked += 1;
        if let Err(diags) = formatted {
            failures.push(format!("{}: {} diagnostic(s)", name(&case), diags.len()));
        }
    }
    assert!(checked >= 20, "only {checked} parser corpus files checked");
    assert!(
        failures.is_empty(),
        "unformattable:\n{}",
        failures.join("\n")
    );
}
