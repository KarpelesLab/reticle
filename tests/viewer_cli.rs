//! End-to-end tests of `reticle viewer`.
//!
//! The library is sans-I/O and returns the pages as values, so this is the
//! only place that covers the writing half: the output directory, the
//! `source/` sub-directory, the flags that leave a view out, and the exit
//! codes.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs the binary and returns `(exit code, stdout, stderr)`.
fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(args)
        .output()
        .expect("failed to run the reticle binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A scratch directory unique to one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the scratch directory");
    dir
}

/// Every file under `dir`, as paths relative to it, sorted.
fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    out.sort();
    out
}

#[test]
fn writes_a_site_for_a_netlist() {
    let dir = scratch("viewer-netlist");
    let out = dir.to_string_lossy().into_owned();
    let (code, _, stderr) = run(&["viewer", "--output-dir", &out, "testdata/ir/netlist.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("index.html"), "{stderr}");
    assert_eq!(
        tree(&dir),
        [
            "counter_synth.doc.html",
            "counter_synth.schematic.html",
            "index.html",
            "source/0-netlist.rtl.html",
        ]
    );
    let index = std::fs::read_to_string(dir.join("index.html")).unwrap();
    assert!(index.contains("counter_synth"));
    let schematic = std::fs::read_to_string(dir.join("counter_synth.schematic.html")).unwrap();
    assert!(schematic.contains("<svg class=\"schematic\""));
    assert!(!schematic.contains("http"));
}

#[test]
fn synthesises_first_when_asked() {
    let dir = scratch("viewer-synth");
    let out = dir.to_string_lossy().into_owned();
    let (code, _, stderr) = run(&[
        "viewer",
        "--quiet",
        "--synth",
        "--no-source",
        "--output-dir",
        &out,
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let files = tree(&dir);
    assert!(
        files.iter().any(|f| f.ends_with(".schematic.html")),
        "{files:?}"
    );
    assert!(
        !files.iter().any(|f| f.starts_with("source/")),
        "--no-source still wrote listings: {files:?}"
    );
    // Synthesis replaced the processes with cells, so the schematic shows
    // the cell form rather than one box per process.
    let schematic = files
        .iter()
        .find(|f| f.ends_with(".schematic.html"))
        .unwrap();
    let text = std::fs::read_to_string(dir.join(schematic)).unwrap();
    assert!(text.contains("node seq"), "no state was drawn");
    assert!(!text.contains("process seq"), "a process survived --synth");
}

#[test]
fn flags_select_and_reject() {
    let dir = scratch("viewer-flags");
    let out = dir.to_string_lossy().into_owned();

    // One module only, no schematic.
    let (code, _, stderr) = run(&[
        "viewer",
        "--quiet",
        "--no-schematic",
        "--no-source",
        "--module",
        "adder",
        "--title",
        "just the adder",
        "--output-dir",
        &out,
        "testdata/ir/hierarchy.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(tree(&dir), ["adder.doc.html", "index.html"]);
    let index = std::fs::read_to_string(dir.join("index.html")).unwrap();
    assert!(index.contains("just the adder"), "{index}");

    // An unknown top is a usage error, not a panic.
    let (code, _, stderr) = run(&[
        "viewer",
        "--top",
        "nope",
        "--output-dir",
        &out,
        "testdata/ir/hierarchy.rtl",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("no module named `nope`"), "{stderr}");

    // And so is no input at all.
    assert_eq!(run(&["viewer"]).0, 2);
}

#[test]
fn help_mentions_the_command() {
    let (code, stdout, _) = run(&["help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("viewer"), "{stdout}");
    let (code, stdout, _) = run(&["help", "viewer"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--output-dir"), "{stdout}");
    assert!(stdout.contains("--no-schematic"), "{stdout}");
}
