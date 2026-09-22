//! Golden tests for the synthesis pipeline.
//!
//! Every `testdata/synth/<name>.rtl` (process form) is parsed, validated
//! and run through [`reticle::synth::run`] twice: once with
//! `cellify` off, whose result must match `<name>.synth.rtl` (the
//! expression form the optimisation loop leaves), and once with the
//! default options, whose result must match `<name>.cells.rtl` (the
//! netlist of cells the default pipeline produces). Both must validate
//! and print back identically. Diagnostics are rendered and compared with
//! `<name>.diag` when that file exists (or when any diagnostic was
//! produced). Set `UPDATE_EXPECT=1` to rewrite the expectations after an
//! intended change.

#![cfg(feature = "synth")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;
use reticle::synth::{SynthOptions, run};

fn inputs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/synth");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.ends_with(".rtl") && !name.ends_with(".synth.rtl") && !name.ends_with(".cells.rtl")
        })
        .collect();
    paths.sort();
    paths
}

fn first_difference(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!(
        "expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

/// Compares `actual` with the file at `path`, rewriting it under
/// `UPDATE_EXPECT`. An absent file counts as empty.
fn check(path: &Path, actual: &str, update: bool, failures: &mut Vec<String>) {
    let expected = fs::read_to_string(path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if update {
        if actual.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, actual).unwrap();
        }
    } else {
        failures.push(format!(
            "{}: mismatch (set UPDATE_EXPECT=1 to rewrite)\n{}",
            path.file_name().unwrap().to_string_lossy(),
            first_difference(&expected, actual)
        ));
    }
}

#[test]
fn golden_synthesis() {
    let paths = inputs();
    assert!(!paths.is_empty(), "no golden files found");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut failures = Vec::new();

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".rtl").to_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let mut design = match Design::parse_text(&text, file) {
            Ok(design) => design,
            Err(diags) => {
                failures.push(format!("{name}: parse failed\n{}", diags.render(&map)));
                continue;
            }
        };
        let problems = validate(&design);
        if problems.has_errors() {
            failures.push(format!("{name}: input invalid\n{}", problems.render(&map)));
            continue;
        }

        // Two expectations per design: the expression form the
        // optimisation loop leaves (`.synth.rtl`), and what the default
        // pipeline produces once `cellify` has turned every expression
        // into cells (`.cells.rtl`).
        let mut expressions = design.clone();
        let mut ignored = Diagnostics::new();
        let stats = run(
            &mut expressions,
            &SynthOptions {
                validate: true,
                cellify: false,
                ..SynthOptions::default()
            },
            &mut ignored,
        );
        assert!(stats.iterations >= 1, "{name}: pipeline did not run");

        let mut diags = Diagnostics::new();
        let options = SynthOptions {
            validate: true,
            ..SynthOptions::default()
        };
        run(&mut design, &options, &mut diags);
        diags.sort();

        for (design, suffix) in [(&expressions, "synth"), (&design, "cells")] {
            let problems = validate(design);
            if problems.has_errors() {
                failures.push(format!(
                    "{name}: {suffix} output invalid\n{}",
                    problems.render(&map)
                ));
            }
            let printed = design.to_text();
            check(
                &path.with_file_name(format!("{stem}.{suffix}.rtl")),
                &printed,
                update,
                &mut failures,
            );

            // The printed result must load back and print identically.
            let mut map2 = SourceMap::new();
            let file2 = map2.add(name.clone(), printed.clone()).unwrap();
            match Design::parse_text(&printed, file2) {
                Ok(again) => {
                    if again.to_text() != printed {
                        failures.push(format!(
                            "{name}: {suffix} output is not a print fixed point"
                        ));
                    }
                }
                Err(diags) => failures.push(format!(
                    "{name}: {suffix} output does not parse\n{}",
                    diags.render(&map2)
                )),
            }
        }

        let diag_path = path.with_file_name(format!("{stem}.diag"));
        check(&diag_path, &diags.render(&map), update, &mut failures);
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
