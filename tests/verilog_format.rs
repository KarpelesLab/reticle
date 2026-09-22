//! Golden and property tests for the Verilog formatter.
//!
//! Every `testdata/verilog/format/<name>.v` (Verilog-2005) or `<name>.sv`
//! (SystemVerilog) is formatted with the default options and must match
//! `<name>.fmt.v` / `<name>.fmt.sv`. Set `UPDATE_EXPECT=1` to rewrite the
//! expectations after an intended change.
//!
//! Three properties are then checked over both that corpus and the whole
//! parser corpus (`testdata/verilog/parse`, minus the files with expected
//! diagnostics):
//!
//! - *idempotence*: formatting formatted text changes nothing;
//! - *semantic preservation*: the AST of the formatted text is the AST of
//!   the original, ignoring positions;
//! - *comment preservation*: the multiset of comment texts is unchanged.

#![cfg(feature = "verilog")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::verilog::format::{FormatOptions, format_check, format_source};
use reticle::verilog::{Dialect, Lexer, NoIncludes, ast_dump, parse_source};

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn dialect_of(path: &Path) -> Dialect {
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    Dialect::for_extension(&ext).expect("known extension")
}

/// Every `.v` / `.sv` file of a directory, sorted.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv")))
        .filter(|p| {
            !p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".fmt"))
        })
        .collect();
    out.sort();
    out
}

/// The AST dump with every `@line:col` removed, so two layouts of the same
/// design compare equal.
fn dump_without_positions(text: &str, dialect: Dialect, label: &str) -> String {
    let mut map = SourceMap::new();
    let id = map
        .add(label.to_owned(), text.to_owned())
        .expect("text fits");
    let mut diags = Diagnostics::new();
    let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut diags);
    assert!(
        !diags.has_errors(),
        "{label}: reparse failed:\n{}",
        diags.render(&map)
    );
    strip_positions(&ast_dump::dump_with_locations(&file, &map))
}

/// Removes `@<line>:<col>` markers from a dump.
fn strip_positions(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    let mut rest = dump;
    while let Some(at) = rest.find('@') {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let mut chars = tail[1..].char_indices();
        let mut end = 1;
        let mut seen_colon = false;
        let mut digits = 0;
        for (i, c) in chars.by_ref() {
            if c.is_ascii_digit() {
                digits += 1;
                end = i + 2;
            } else if c == ':' && !seen_colon && digits > 0 {
                seen_colon = true;
                digits = 0;
                end = i + 2;
            } else {
                break;
            }
        }
        if seen_colon && digits > 0 {
            rest = &tail[end..];
        } else {
            out.push('@');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The texts of every comment in a file, sorted, for comparing multisets.
fn comment_texts(text: &str, dialect: Dialect) -> Vec<String> {
    let mut map = SourceMap::new();
    let id = map.add("case.v", text.to_owned()).expect("text fits");
    let mut diags = Diagnostics::new();
    let lexed = Lexer::new(map.file(id).text(), id, dialect, &mut diags).run();
    let mut out: Vec<String> = lexed
        .comments
        .iter()
        .map(|(span, _)| {
            map.file(id).text()[span.start as usize..span.end as usize]
                .trim_end()
                .to_owned()
        })
        .collect();
    out.sort();
    out
}

/// Runs the three properties over one file.
fn check_properties(path: &Path, failures: &mut Vec<String>) {
    let dialect = dialect_of(path);
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let opts = FormatOptions::default();
    let Ok(formatted) = format_source(&text, dialect, &opts) else {
        return;
    };

    match format_source(&formatted, dialect, &opts) {
        Ok(twice) if twice == formatted => {}
        Ok(twice) => failures.push(format!(
            "{}: not idempotent\n--- once\n{formatted}--- twice\n{twice}",
            path.display()
        )),
        Err(diags) => failures.push(format!(
            "{}: formatted text no longer parses ({} diagnostics)",
            path.display(),
            diags.len()
        )),
    }

    let before = dump_without_positions(&text, dialect, &format!("{} (original)", path.display()));
    let after = dump_without_positions(
        &formatted,
        dialect,
        &format!("{} (formatted)", path.display()),
    );
    if before != after {
        let line = before
            .lines()
            .zip(after.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        failures.push(format!(
            "{}: the tree changed at dump line {}\n  before: {}\n  after:  {}",
            path.display(),
            line + 1,
            before.lines().nth(line).unwrap_or("<end>"),
            after.lines().nth(line).unwrap_or("<end>"),
        ));
    }

    let before = comment_texts(&text, dialect);
    let after = comment_texts(&formatted, dialect);
    if before != after {
        failures.push(format!(
            "{}: comments changed\n  before: {before:?}\n  after:  {after:?}",
            path.display()
        ));
    }
}

#[test]
fn golden() {
    let dir = manifest("testdata/verilog/format");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let cases = sources(&dir);
    assert!(cases.len() >= 15, "the formatter corpus is too small");

    let mut failures = Vec::new();
    for case in &cases {
        let text = fs::read_to_string(case).expect("read corpus file");
        let dialect = dialect_of(case);
        let formatted = match format_source(&text, dialect, &FormatOptions::default()) {
            Ok(formatted) => formatted,
            Err(diags) => {
                failures.push(format!(
                    "{}: does not parse ({})",
                    case.display(),
                    diags.len()
                ));
                continue;
            }
        };
        let ext = case.extension().unwrap().to_string_lossy().into_owned();
        let expect_path = case.with_extension(format!("fmt.{ext}"));
        match fs::read_to_string(&expect_path) {
            Ok(expected) if expected == formatted => {}
            _ if update => {
                fs::write(&expect_path, &formatted).expect("write expectation");
                eprintln!("updated {}", expect_path.display());
            }
            Ok(expected) => {
                let line = expected
                    .lines()
                    .zip(formatted.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| expected.lines().count().min(formatted.lines().count()));
                failures.push(format!(
                    "{}: mismatch at line {}\n  expected: {}\n  actual:   {}",
                    case.display(),
                    line + 1,
                    expected.lines().nth(line).unwrap_or("<end>"),
                    formatted.lines().nth(line).unwrap_or("<end>"),
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

/// The formatted text of the golden corpus must already be formatted.
#[test]
fn goldens_are_fixpoints() {
    let dir = manifest("testdata/verilog/format");
    let mut failures = Vec::new();
    for case in sources(&dir) {
        let ext = case.extension().unwrap().to_string_lossy().into_owned();
        let expect_path = case.with_extension(format!("fmt.{ext}"));
        let Ok(expected) = fs::read_to_string(&expect_path) else {
            continue;
        };
        let check = format_check(&expected, dialect_of(&case), &FormatOptions::default())
            .expect("expectation parses");
        if check.changed {
            failures.push(format!("{}\n{}", expect_path.display(), check.diff));
        }
    }
    assert!(
        failures.is_empty(),
        "not fixpoints:\n{}",
        failures.join("\n")
    );
}

#[test]
fn properties_over_the_formatter_corpus() {
    let mut failures = Vec::new();
    for case in sources(&manifest("testdata/verilog/format")) {
        check_properties(&case, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn properties_over_the_parser_corpus() {
    let dir = manifest("testdata/verilog/parse");
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in sources(&dir) {
        // The runner records expected diagnostics in the `.ast` file; those
        // cases are not expected to format.
        let expectation = fs::read_to_string(case.with_extension("ast")).unwrap_or_default();
        if expectation.contains("--- diagnostics") {
            continue;
        }
        checked += 1;
        check_properties(&case, &mut failures);
    }
    assert!(checked >= 20, "only {checked} parser corpus files checked");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Parser corpus files that use a macro where a token is expected, so they
/// cannot be parsed without the preprocessor and are left alone. See the
/// formatter's module documentation.
const MACRO_HEAVY: &[&str] = &["directives.v"];

/// Every parser corpus file without expected diagnostics must format,
/// except the macro-heavy ones, which must not (so the list stays honest).
#[test]
fn the_parser_corpus_formats() {
    let dir = manifest("testdata/verilog/parse");
    let mut failures = Vec::new();
    for case in sources(&dir) {
        let expectation = fs::read_to_string(case.with_extension("ast")).unwrap_or_default();
        if expectation.contains("--- diagnostics") {
            continue;
        }
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        if MACRO_HEAVY.contains(&name.as_str()) {
            let text = normalise(&fs::read_to_string(&case).expect("read corpus file"));
            assert!(
                format_source(&text, dialect_of(&case), &FormatOptions::default()).is_err(),
                "{name} now formats; drop it from MACRO_HEAVY"
            );
            continue;
        }
        let text = normalise(&fs::read_to_string(&case).expect("read corpus file"));
        if let Err(diags) = format_source(&text, dialect_of(&case), &FormatOptions::default()) {
            // Offsets survive the formatter's directive blanking, so the
            // original text renders the diagnostics faithfully.
            let mut map = SourceMap::new();
            let _ = map.add("<input>", text);
            failures.push(format!("{}:\n{}", case.display(), diags.render(&map)));
        }
    }
    assert!(
        failures.is_empty(),
        "unformattable:\n{}",
        failures.join("\n")
    );
}
