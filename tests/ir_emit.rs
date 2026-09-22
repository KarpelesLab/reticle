//! Golden tests for the IR emitters.
//!
//! Every `testdata/ir/emit/<name>.rtl` is parsed, validated and rendered in
//! each format; the result must match `<name>.<ext>` (`v`, `vhd`, `json`,
//! `blif`, `edif`), or `<name>.<ext>.err` when the format legitimately
//! cannot express the design, in which case the file holds the error's
//! location and message. Run with `UPDATE_EXPECT=1` to rewrite the expected
//! files after a deliberate change.

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::Design;
use reticle::ir::emit::{Format, emit};
use reticle::ir::validate::validate;
use reticle::source::SourceMap;

fn golden_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ir/emit");
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

/// Compares `actual` with the file at `path`, rewriting it under
/// `UPDATE_EXPECT`, and removes the stale counterpart (`.err` next to a
/// success file or vice versa) so a format change cannot leave both.
fn check(path: &Path, stale: &Path, actual: &str, update: bool, failures: &mut Vec<String>) {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    if update {
        fs::write(path, actual).unwrap();
        if stale.exists() {
            fs::remove_file(stale).unwrap();
        }
        return;
    }
    match fs::read_to_string(path) {
        Ok(expected) if expected == actual => {}
        Ok(expected) => failures.push(format!(
            "{name}: output differs (set UPDATE_EXPECT=1 to rewrite)\n{}",
            first_difference(&expected, actual)
        )),
        Err(_) => failures.push(format!(
            "{name}: expected file missing (set UPDATE_EXPECT=1 to create)\n--- actual ---\n{actual}"
        )),
    }
    if stale.exists() {
        failures.push(format!(
            "{}: stale expectation next to {name}",
            stale.file_name().unwrap().to_string_lossy()
        ));
    }
}

#[test]
fn golden_emit_outputs() {
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
        let diags = validate(&design);
        if diags.has_errors() {
            failures.push(format!("{name}: validation failed\n{}", diags.render(&map)));
            continue;
        }
        for format in Format::ALL {
            let ok_path = path.with_extension(format.extension());
            let err_path = path.with_extension(format!("{}.err", format.extension()));
            match emit(&design, format) {
                Ok(output) => check(&ok_path, &err_path, &output, update, &mut failures),
                Err(e) => {
                    let (file, loc) = map.locate(e.span);
                    let rendered = format!("{file}:{loc}: {}\n", e.message);
                    check(&err_path, &ok_path, &rendered, update, &mut failures);
                }
            }
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Every golden `.rtl` also round-trips through the text format, so the
/// corpus stays canonical.
#[test]
fn golden_inputs_are_canonical() {
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut failures = Vec::new();
    for path in golden_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let Ok(design) = Design::parse_text(&text, file) else {
            continue;
        };
        let printed = design.to_text();
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
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Runs the Verilog outputs through `iverilog` and the VHDL outputs through
/// `ghdl` when those tools are installed; ignored by default because CI
/// machines do not have them.
#[test]
#[ignore = "needs iverilog / ghdl on PATH"]
fn external_tools_accept_outputs() {
    use std::process::Command;
    let mut failures = Vec::new();
    let has = |tool: &str| {
        Command::new(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let iverilog = has("iverilog");
    let ghdl = has("ghdl");
    assert!(iverilog || ghdl, "neither iverilog nor ghdl is installed");
    for path in golden_files() {
        let v = path.with_extension("v");
        if iverilog && v.exists() {
            let out = Command::new("iverilog")
                .args(["-g2005", "-t", "null", "-o", "/dev/null"])
                .arg(&v)
                .output()
                .unwrap();
            if !out.status.success() {
                failures.push(format!(
                    "{}: iverilog rejected the output\n{}",
                    v.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
        let vhd = path.with_extension("vhd");
        if ghdl && vhd.exists() {
            let out = Command::new("ghdl")
                .args(["-s", "--std=08"])
                .arg(&vhd)
                .output()
                .unwrap();
            if !out.status.success() {
                failures.push(format!(
                    "{}: ghdl rejected the output\n{}",
                    vhd.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
