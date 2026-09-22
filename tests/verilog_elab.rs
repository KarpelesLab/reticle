//! Golden tests for Verilog elaboration and lowering to the IR.
//!
//! Every `testdata/verilog/elab/<name>.v` (Verilog-2005) or `<name>.sv`
//! (SystemVerilog) is parsed, elaborated and lowered; the `.rtl` text of
//! the resulting design must match `<name>.rtl`, and the rendered
//! diagnostics must match `<name>.diag` (which exists only when
//! elaboration reports something). A case whose name starts with `error_`
//! must report at least one error and produces no `.rtl`.
//!
//! A first line of the form `// top: name` in a corpus file selects the
//! top module, and `// param: NAME=VALUE` adds a parameter override.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change.

#![cfg(feature = "verilog")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The options a corpus file asks for in its leading comments.
fn options(text: &str, dialect: Dialect) -> ElabOptions {
    let mut opts = ElabOptions::new(dialect);
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("// ") else {
            if line.starts_with("//") {
                continue;
            }
            break;
        };
        if let Some(top) = rest.strip_prefix("top:") {
            opts.top = Some(top.trim().to_owned());
        } else if let Some(param) = rest.strip_prefix("param:")
            && let Some((name, value)) = param.split_once('=')
        {
            opts.params
                .push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    opts
}

/// Elaborates one corpus file, returning its `.rtl` and its diagnostics.
fn run(path: &Path) -> (Option<String>, String) {
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let ext = path.extension().unwrap().to_string_lossy();
    let dialect = Dialect::for_extension(&ext).expect("known extension");
    let opts = options(&text, dialect);

    let mut map = SourceMap::new();
    let id = map.add(name.clone(), text).unwrap();
    let mut diags = Diagnostics::new();
    let file = parse_source(&mut map, id, dialect, &mut NoIncludes, &mut diags);
    assert!(
        !diags.has_errors(),
        "{name} must parse:\n{}",
        diags.render(&map)
    );
    let design = elaborate(&[&file], &opts, &mut diags);
    diags.sort();
    let rendered = diags.render(&map);
    if let Some(design) = &design {
        let problems = validate(design);
        assert!(
            problems.is_empty(),
            "{name}: the lowered design does not validate:\n{}",
            problems.render(&map)
        );
    }
    (design.map(|d| d.to_text()), rendered)
}

/// Compares `actual` with the file at `path`, honouring `UPDATE_EXPECT`.
fn check(path: &Path, actual: &str, failures: &mut Vec<String>) {
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let expected = fs::read_to_string(path).map(|t| normalise(&t));
    match expected {
        Ok(expected) if expected == actual => {}
        _ if update => {
            if actual.is_empty() {
                let _ = fs::remove_file(path);
            } else {
                fs::write(path, actual).expect("write expectation");
                eprintln!("updated {}", path.display());
            }
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
            failures.push(format!(
                "{}: mismatch at line {line}\n  expected: {}\n  actual:   {}",
                path.display(),
                expected.lines().nth(line - 1).unwrap_or("<end>"),
                actual.lines().nth(line - 1).unwrap_or("<end>"),
            ));
        }
        Err(_) if actual.is_empty() => {}
        Err(_) => failures.push(format!(
            "{}: missing expectation (run with UPDATE_EXPECT=1)",
            path.display()
        )),
    }
}

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/elab");
    let mut cases: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv")))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no corpus files in {}", dir.display());
    cases
}

#[test]
fn golden() {
    let mut failures = Vec::new();
    for case in corpus() {
        let (rtl, diags) = run(&case);
        check(&case.with_extension("diag"), &diags, &mut failures);
        check(
            &case.with_extension("rtl"),
            rtl.as_deref().unwrap_or(""),
            &mut failures,
        );
    }
    assert!(
        failures.is_empty(),
        "{} golden test(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `error_*` cases must report an error and produce no design; every other
/// case must produce one.
#[test]
fn error_cases_fail_and_others_do_not() {
    for case in corpus() {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let (rtl, diags) = run(&case);
        if name.starts_with("error_") {
            assert!(rtl.is_none(), "{name} should not elaborate");
            assert!(diags.contains("error["), "{name} should report an error");
        } else {
            assert!(rtl.is_some(), "{name} should elaborate:\n{diags}");
            assert!(
                !diags.contains("error["),
                "{name} should elaborate cleanly:\n{diags}"
            );
        }
    }
}

/// Every module in the parser corpus that is a complete design should
/// either elaborate or report a diagnostic; none may panic.
#[test]
fn survives_the_parser_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/verilog/parse");
    let mut count = 0;
    for entry in fs::read_dir(&dir).expect("parser corpus directory") {
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
        let design = elaborate(&[&file], &ElabOptions::new(dialect), &mut diags);
        if let Some(design) = design {
            let problems = validate(&design);
            assert!(
                problems.is_empty(),
                "{}: invalid IR:\n{}",
                path.display(),
                problems.render(&map)
            );
            // The text format must round-trip what we produced.
            let text = design.to_text();
            let mut map2 = SourceMap::new();
            let id2 = map2.add("out.rtl", text.clone()).unwrap();
            let back = reticle::ir::Design::parse_text(&text, id2)
                .unwrap_or_else(|d| panic!("{}: {}", path.display(), d.render(&map2)));
            assert_eq!(
                back.to_text(),
                text,
                "{} does not round-trip",
                path.display()
            );
        }
        count += 1;
    }
    assert!(count > 0, "no parser corpus files found");
}

/// Elaborates an open core if one is installed on this machine. Ignored by
/// default: the file is not part of the repository.
///
/// Run with `cargo test --features verilog -- --ignored open_core`.
#[test]
#[ignore = "needs picorv32.v or another open core on the machine"]
fn open_core() {
    let candidates = [
        "/usr/share/picorv32/picorv32.v",
        "/usr/local/share/picorv32/picorv32.v",
        "picorv32.v",
    ];
    let Some(path) = candidates.iter().map(Path::new).find(|p| p.exists()) else {
        eprintln!("no open core found; nothing to do");
        return;
    };
    let text = normalise(&fs::read_to_string(path).expect("read core"));
    let mut map = SourceMap::new();
    let id = map.add(path.to_string_lossy().into_owned(), text).unwrap();
    let mut diags = Diagnostics::new();
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    let design = elaborate(
        &[&file],
        &ElabOptions::new(Dialect::Verilog2005),
        &mut diags,
    );
    eprintln!("{}", diags.render(&map));
    let design = design.expect("the core should elaborate");
    let problems = validate(&design);
    assert!(problems.is_empty(), "{}", problems.render(&map));
    eprintln!("elaborated {} modules", design.modules.len());
}
