//! Golden tests for the `.rtl` IR text format.
//!
//! Every `testdata/ir/*.rtl` file must parse, validate without errors, and
//! print back byte-for-byte identical. Run with `UPDATE_EXPECT=1` to rewrite
//! the files with the canonical rendering after a deliberate format change.

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;

fn golden_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ir");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rtl"))
        .collect();
    paths.sort();
    paths
}

/// The first line at which two texts differ, for a readable failure.
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

#[test]
fn golden_rtl_files_round_trip() {
    let paths = golden_files();
    assert!(!paths.is_empty(), "no golden files found");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut failures = Vec::new();

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let design = match Design::parse_text(&text, file) {
            Ok(design) => design,
            Err(diags) => {
                failures.push(format!("{name}: parse failed\n{}", diags.render(&map)));
                continue;
            }
        };

        // The printed form must be a fixed point of parse/print.
        let printed = design.to_text();
        let mut map2 = SourceMap::new();
        let file2 = map2.add(name.clone(), printed.clone()).unwrap();
        match Design::parse_text(&printed, file2) {
            Ok(again) => {
                let twice = again.to_text();
                if twice != printed {
                    failures.push(format!(
                        "{name}: reprint is not a fixed point\n{}",
                        first_difference(&printed, &twice)
                    ));
                }
            }
            Err(diags) => failures.push(format!(
                "{name}: reprint does not parse\n{}",
                diags.render(&map2)
            )),
        }

        if printed != text {
            if update {
                fs::write(&path, &printed).unwrap();
            } else {
                failures.push(format!(
                    "{name}: text is not canonical (set UPDATE_EXPECT=1 to rewrite)\n{}",
                    first_difference(&text, &printed)
                ));
            }
        }

        let diags = validate(&design);
        if diags.has_errors() {
            failures.push(format!("{name}: validation failed\n{}", diags.render(&map)));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
