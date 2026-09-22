//! Golden tests for the Verilog linter.
//!
//! Every `testdata/verilog/lint/<name>.v` (Verilog-2005) or `<name>.sv`
//! (SystemVerilog) is preprocessed, lexed, parsed and linted; the rendered
//! diagnostics must match `<name>.diag`. Set `UPDATE_EXPECT=1` to rewrite
//! the expectations after an intended change.
//!
//! A case selects the rules it exercises with a `// lint-config:` comment
//! on one of its first lines, in the syntax of
//! [`reticle::verilog::lint::LintConfig::parse`]; without one the default
//! configuration runs. Cases named `clean_*` must produce no diagnostics
//! at all.

#![cfg(feature = "verilog")]
#![cfg(feature = "verilog")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::source::SourceMap;
use reticle::verilog::lint::{LintConfig, run};
use reticle::verilog::{Dialect, NoIncludes, lex_source_full, parse_source};

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The configuration a case asks for, from its `// lint-config:` header.
fn config_of(text: &str) -> LintConfig {
    let mut config = LintConfig::new();
    for line in text.lines().take(8) {
        if let Some(spec) = line.split("// lint-config:").nth(1) {
            config = LintConfig::parse(spec).expect("valid lint-config header");
        }
        if let Some(value) = line.split("// max-line-length:").nth(1) {
            config.max_line_length = value.trim().parse().expect("a number");
        }
    }
    config
}

/// Lints one corpus file and renders what it reported.
fn lint_file(path: &Path) -> String {
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let ext = path.extension().unwrap().to_string_lossy();
    let dialect = Dialect::for_extension(&ext).expect("known extension");
    let config = config_of(&text);

    let mut map = SourceMap::new();
    let id = map.add(name.clone(), text).unwrap();
    let mut parse_diags = Diagnostics::new();
    let lexed = lex_source_full(&mut map, id, dialect, &mut NoIncludes, &mut parse_diags);
    let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut parse_diags);
    assert!(
        !parse_diags.has_errors(),
        "{name} must parse cleanly:\n{}",
        parse_diags.render(&map)
    );

    let mut diags = Diagnostics::new();
    run(
        &config,
        &map,
        id,
        &file,
        &lexed.comments,
        dialect,
        &mut diags,
    );
    diags.render(&map)
}

/// Every `.v` / `.sv` file of a directory, sorted.
fn cases(dir: &Path) -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(dir)
        .expect("corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv")))
        .collect();
    cases.sort();
    cases
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/lint")
}

#[test]
fn golden() {
    let dir = corpus();
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let cases = cases(&dir);
    assert!(!cases.is_empty(), "no corpus files in {}", dir.display());

    let mut failures = Vec::new();
    for case in &cases {
        let actual = lint_file(case);
        let expect_path = case.with_extension("diag");
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

/// The `clean_*` cases are real designs that must lint silently, and the
/// others must report something: a rule with no case is not tested.
#[test]
fn clean_designs_are_silent_and_cases_report() {
    let mut seen_clean = false;
    for case in cases(&corpus()) {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let out = lint_file(&case);
        if name.starts_with("clean_") {
            seen_clean = true;
            assert!(out.is_empty(), "{name} should lint cleanly, got:\n{out}");
        } else {
            assert!(!out.is_empty(), "{name} should report something");
        }
    }
    assert!(seen_clean, "no clean designs in the corpus");
}

/// Every rule must have at least one case that reports it, so a rule
/// cannot rot unnoticed.
#[test]
fn every_rule_is_covered() {
    use reticle::verilog::lint::LintSet;

    let mut reported = std::collections::BTreeSet::new();
    for case in cases(&corpus()) {
        for line in lint_file(&case).lines() {
            if let Some(rest) = line.trim_start().strip_prefix("= note: lint: ")
                && let Some(name) = rest.split(',').next()
            {
                reported.insert(name.to_string());
            }
        }
    }
    let missing: Vec<&str> = LintSet::all()
        .names()
        .filter(|n| !reported.contains(*n))
        .collect();
    assert!(missing.is_empty(), "rules with no corpus case: {missing:?}");
}

/// The linter must survive every file of the parser corpus, including the
/// deliberately broken ones, with every rule enabled.
#[test]
fn survives_the_parser_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/parse");
    let mut config = LintConfig::new();
    for name in reticle::verilog::lint::LintSet::all().names() {
        config
            .set_level(name, reticle::verilog::lint::Level::WARN)
            .unwrap();
    }
    let mut count = 0;
    for case in cases(&dir) {
        let text = normalise(&fs::read_to_string(&case).expect("read corpus file"));
        let ext = case.extension().unwrap().to_string_lossy();
        let dialect = Dialect::for_extension(&ext).expect("known extension");
        let mut map = SourceMap::new();
        let id = map.add(case.to_string_lossy().into_owned(), text).unwrap();
        let mut diags = Diagnostics::new();
        let lexed = lex_source_full(&mut map, id, dialect, &mut NoIncludes, &mut diags);
        let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut diags);
        let mut lints = Diagnostics::new();
        run(
            &config,
            &map,
            id,
            &file,
            &lexed.comments,
            dialect,
            &mut lints,
        );
        // Rendering must not panic either.
        let _ = lints.render(&map);
        count += 1;
    }
    assert!(count > 0, "no parser corpus files found");
}
