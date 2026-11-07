//! The language corpus against what Vivado's simulator printed.
//!
//! `testdata/sim/lang/` holds testbenches written here, one language feature
//! area each: Verilog-2005 under `v/`, SystemVerilog under `sv/`, VHDL-2008
//! under `vhd/`. The `v_`, `sv_` and `vh_` files exercise a feature area
//! case by case and print what they compute; the `min_` files are the
//! smallest testbench that showed one difference. Beside each is
//! `<name>.xsim`, what xsim 2026.1 printed for it
//! (`tools/vivado-parity/xsim_lang.sh` writes them; Vivado is not needed to
//! run this). Each testbench runs under `reticle sim --top tb`, and what it
//! prints must be xsim's, byte for byte, but for xsim's own run-time
//! warnings (`WARNING: ...`, and the tab-indented lines that go on with
//! one), which go to its log where Reticle's go to standard error.
//!
//! `known.txt` lists the testbenches that still print something else, each
//! with the reason, and the test holds them to differing: one that starts
//! to agree must leave the list, so that the list says what is left. An
//! entry marked `skip` is not run (it does not end).
//!
//! It would catch a construct xsim accepts and Reticle refuses or computes
//! differently, in any area the corpus covers, and a fix that undoes
//! another. It would not catch a difference in a construct no testbench
//! uses, nor one in what a testbench does not print.

#![cfg(all(feature = "sim", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/sim/lang")
}

/// Every testbench with an answer beside it, or `None` with a line saying
/// the corpus is not there.
fn testbenches() -> Option<Vec<PathBuf>> {
    let root = corpus();
    if !root.is_dir() {
        eprintln!("skipped: `testdata/sim/lang` is not in the published crate");
        return None;
    }
    let mut files = Vec::new();
    for dir in ["v", "sv", "vhd"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let is_tb = p
                .extension()
                .is_some_and(|x| x == "v" || x == "sv" || x == "vhd");
            if is_tb && p.with_extension("xsim").is_file() {
                files.push(p);
            }
        }
    }
    files.sort();
    Some(files)
}

/// `known.txt`: testbench path (relative to the corpus) to its reason.
fn known() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(corpus().join("known.txt")).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(' ') {
            Some((name, why)) => (name.to_owned(), why.trim().to_owned()),
            None => (l.to_owned(), String::new()),
        })
        .collect()
}

/// What a testbench printed under Reticle; `None` when it had not ended
/// after a minute, which no testbench here takes when it runs as it should
/// (they end in well under a second), so that a hang is a failure rather
/// than a test that never ends.
fn reticle(file: &Path) -> Option<String> {
    use std::io::Read;
    let mut child = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb"])
        .arg(file)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to run the reticle binary");
    let mut out = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = out.read_to_end(&mut text);
        text
    });
    for _ in 0..6000 {
        if child.try_wait().ok().flatten().is_some() {
            let text = reader.join().unwrap_or_default();
            return Some(String::from_utf8_lossy(&text).into_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// xsim's answer, less its own run-time warnings.
fn xsim(file: &Path) -> String {
    let text = std::fs::read_to_string(file.with_extension("xsim")).unwrap();
    text.lines()
        .filter(|l| !l.starts_with("WARNING: ") && !l.starts_with("\t "))
        .map(|l| format!("{l}\n"))
        .collect()
}

/// **Every testbench prints what xsim printed**, but those `known.txt`
/// lists, which still do not and are held to that.
#[test]
fn every_testbench_prints_what_xsim_printed() {
    let Some(files) = testbenches() else { return };
    assert!(files.len() >= 100, "{} testbenches", files.len());
    let known = known();
    let root = corpus();
    let mut wrong = Vec::new();
    let mut fixed = Vec::new();
    let mut matched = 0usize;
    for file in &files {
        let name = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let entry = known.iter().find(|(n, _)| *n == name);
        if entry.is_some_and(|(_, why)| why.starts_with("skip")) {
            continue;
        }
        let theirs = xsim(file);
        let Some(ours) = reticle(file) else {
            if entry.is_none() {
                wrong.push(format!("{name}: did not end"));
            }
            continue;
        };
        let same = ours == theirs;
        match (same, entry) {
            (true, None) => matched += 1,
            (true, Some(_)) => fixed.push(name),
            (false, Some(_)) => {}
            (false, None) => {
                let (n, (a, b)) = ours
                    .lines()
                    .chain(std::iter::repeat("(nothing)"))
                    .zip(theirs.lines())
                    .enumerate()
                    .find(|(_, (a, b))| a != b)
                    .unwrap_or((0, ("(more lines)", "(fewer lines)")));
                wrong.push(format!(
                    "{name}: line {}:\n  reticle {a}\n  xsim    {b}",
                    n + 1
                ));
            }
        }
    }
    for (name, _) in &known {
        assert!(
            files.iter().any(|f| f.ends_with(name)),
            "known.txt names `{name}`, which is not in the corpus"
        );
    }
    eprintln!(
        "{matched} of {} testbenches print what xsim printed; {} are known not to",
        files.len(),
        known.len()
    );
    assert!(
        fixed.is_empty(),
        "these now print what xsim printed; take them out of known.txt:\n{}",
        fixed.join("\n")
    );
    assert!(
        wrong.is_empty(),
        "{} testbench(es) differ from xsim:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
