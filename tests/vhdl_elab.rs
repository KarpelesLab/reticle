//! Golden tests for VHDL elaboration and lowering to the IR.
//!
//! Every `testdata/vhdl/elab/<name>.vhd` is analysed against the bundled
//! `std` and `ieee` libraries and elaborated; the `.rtl` text of the
//! resulting design must match `<name>.rtl`, and the rendered diagnostics
//! must match `<name>.diag` (which exists only when something was
//! reported). A case whose name starts with `error_` must report at least
//! one error and produces no `.rtl`.
//!
//! Leading comments select the options: `-- top: name` picks the top
//! entity and `-- generic: NAME=VALUE` overrides one of its generics.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change.

#![cfg(feature = "vhdl")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;
use reticle::vhdl::sema::Design as VhdlDesign;
use reticle::vhdl::{ElabOptions, Standard, elaborate};

/// Line endings are normalised so a CRLF checkout compares equal.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The options a corpus file asks for in its leading comments.
fn options(text: &str) -> ElabOptions {
    let mut opts = ElabOptions::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("-- ") else {
            if line.starts_with("--") {
                continue;
            }
            break;
        };
        if let Some(top) = rest.strip_prefix("top:") {
            opts.top = Some(top.trim().to_owned());
        } else if let Some(generic) = rest.strip_prefix("generic:")
            && let Some((name, value)) = generic.split_once('=')
        {
            opts.generics
                .push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    opts
}

/// Analyses and elaborates one source, returning its `.rtl` and the
/// diagnostics of both phases.
fn run_source(name: &str, text: &str, opts: &ElabOptions) -> (Option<String>, String) {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut design = VhdlDesign::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    assert!(
        !diags.has_errors(),
        "the bundled libraries failed to parse:\n{}",
        diags.render(&map)
    );
    let id = map.add(name.to_owned(), text.to_owned()).unwrap();
    design.add_source(&map, id, "work", &mut diags);
    let analysis = design.analyze(&map, &mut diags);
    if diags.has_errors() {
        diags.sort();
        return (None, diags.render(&map));
    }
    let elaborated = elaborate(&analysis, opts, &mut diags);
    diags.sort();
    let rendered = diags.render(&map);
    let rtl = elaborated.map(|d| {
        let problems = validate(&d);
        assert!(
            problems.is_empty(),
            "{name}: the lowered design does not validate:\n{}",
            problems.render(&map)
        );
        let text = d.to_text();
        round_trip(name, &text);
        text
    });
    (rtl, rendered)
}

/// The `.rtl` text must parse back to a design that prints identically.
fn round_trip(name: &str, text: &str) {
    let mut map = SourceMap::new();
    let id = map.add("out.rtl", text.to_owned()).unwrap();
    let back = Design::parse_text(text, id)
        .unwrap_or_else(|d| panic!("{name}: the IR text does not parse:\n{}", d.render(&map)));
    assert_eq!(
        back.to_text(),
        text,
        "{name}: the IR text does not round-trip"
    );
}

fn run(path: &Path) -> (Option<String>, String) {
    let text = normalise(&fs::read_to_string(path).expect("read corpus file"));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let opts = options(&text);
    run_source(&name, &text, &opts)
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/elab");
    let mut cases: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    cases.sort();
    assert!(cases.len() >= 25, "the elaboration corpus is too small");
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
/// case must produce one with no errors at all.
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

/// Every clean design in the semantic-analysis corpus must either
/// elaborate to a valid, round-tripping design or report a diagnostic;
/// none may panic. The `err_*` cases have analysis errors and are skipped,
/// as are the VHDL-93 ones.
#[test]
fn survives_the_sema_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/sema");
    let mut count = 0;
    let mut elaborated = 0;
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("sema corpus directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
        .collect();
    files.sort();
    for path in files {
        let text = normalise(&fs::read_to_string(&path).expect("read corpus file"));
        // The corpus is analysed as VHDL-2008 unless its first line says
        // otherwise; a VHDL-93 file has nothing extra to elaborate.
        if text.lines().next() == Some("-- sema: vhdl93") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("err_") {
            continue;
        }
        let (rtl, _) = run_source(&name, &text, &ElabOptions::new());
        if rtl.is_some() {
            elaborated += 1;
        }
        count += 1;
    }
    assert!(count > 0, "no sema corpus files found");
    assert!(
        elaborated >= count / 2,
        "only {elaborated} of {count} sema corpus designs elaborated"
    );
}

/// The `ieee.numeric_std` testbench simulates and every one of its
/// hand-computed checks holds. This is the strongest evidence that the
/// native builtins are right: the arithmetic is lowered to IR operators
/// and then actually run.
#[cfg(feature = "sim")]
#[test]
fn numeric_std_arithmetic_simulates() {
    use reticle::sim::{SimOptions, Simulator};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/elab/numeric_std_tb.vhd");
    let text = normalise(&fs::read_to_string(&path).expect("read the testbench"));

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut vhdl = VhdlDesign::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    let id = map.add("numeric_std_tb.vhd".to_owned(), text).unwrap();
    vhdl.add_source(&map, id, "work", &mut diags);
    let analysis = vhdl.analyze(&map, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let design = elaborate(&analysis, &ElabOptions::new().with_top("tb"), &mut diags)
        .unwrap_or_else(|| panic!("the testbench should elaborate:\n{}", diags.render(&map)));

    let mut sim = Simulator::new(&design, SimOptions::default())
        .unwrap_or_else(|d| panic!("the testbench should load:\n{}", d.render(&map)));
    sim.run();
    let messages = sim.messages().render(&map);
    assert!(
        messages.contains("start") && messages.contains("done"),
        "the testbench did not run to the end:\n{messages}"
    );
    // Every assertion in the file reports on failure, so any `wrong` in
    // the output names an operator whose result did not match.
    assert!(
        !messages.contains("wrong"),
        "an arithmetic check failed:\n{messages}"
    );
}

/// The lowered testbench simulates, and reports what it was written to
/// report: the strongest evidence that the lowering is right.
#[cfg(feature = "sim")]
#[test]
fn testbench_simulates() {
    use reticle::sim::{SimOptions, Simulator};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vhdl/elab/testbench.vhd");
    let text = normalise(&fs::read_to_string(&path).expect("read the testbench"));

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut vhdl = VhdlDesign::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    let id = map.add("testbench.vhd".to_owned(), text).unwrap();
    vhdl.add_source(&map, id, "work", &mut diags);
    let analysis = vhdl.analyze(&map, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let design = elaborate(&analysis, &ElabOptions::new().with_top("tb"), &mut diags)
        .unwrap_or_else(|| panic!("the testbench should elaborate:\n{}", diags.render(&map)));

    let mut sim = Simulator::new(&design, SimOptions::default())
        .unwrap_or_else(|d| panic!("the testbench should load:\n{}", d.render(&map)));
    sim.run();
    let messages = sim.messages().render(&map);
    assert!(
        messages.contains("start"),
        "the `report \"start\"` did not fire:\n{messages}"
    );
    assert!(
        messages.contains("done"),
        "the `report \"done\"` did not fire:\n{messages}"
    );
    assert!(
        !messages.contains("q did not follow d"),
        "the flip-flop did not capture `d`:\n{messages}"
    );
}
