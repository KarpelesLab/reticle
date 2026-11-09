//! The same 7-series build, run twice at once in two processes, writes the
//! same bytes.
//!
//! # Why two processes
//!
//! std's `HashMap` and `HashSet` draw their hash keys at random for each
//! process (and step them for each map within one), so a flow that let one's
//! iteration order decide anything would place, route or name differently from
//! one run to the next. Reticle's builds are meant to be byte for byte
//! reproducible: the ChipWhisperer-Husky's byte-identity checks compare
//! bitstreams across builds, and a golden file is a statement about every run.
//! Two processes are two draws of every key; a build in one process would be
//! one.
//!
//! # What this would and would not catch
//!
//! It would catch a hash order, or anything else that differs between two
//! processes (an address, the time, the environment), that reaches the
//! bitstream, the netlist and placement the Vivado export writes, the routes,
//! the timing endpoints or the report, **when it does so on most runs**: two
//! runs that differ that way differ here.
//!
//! Checked by making legalisation seat the cells in an order drawn from a
//! `RandomState`: both tests failed, the two reports differing.
//!
//! It would not catch one that matters on a rare run. One build in 1,360 of
//! `regfile_alu` panicked in the placer on 2026-11-10 and none of the
//! thousands of reruns did; a pair of runs here is two draws, and would have
//! had about one chance in 700 of seeing it. What looked for that was a scratch
//! build whose hash maps all take their keys from a variable, run over every
//! key a thousand times (`docs/fpga-placement.md`, "Reproducible builds"); this
//! test is the part of that which is cheap enough to run on every gate.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DEVICE: &str = "xc7a100t-ftg256";

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The database root, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100tftg256-1/package_pins.csv";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but has no `{probe}`");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database with the FTG256 package; run \
                 `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// One build of `name` from `testdata/fpga/vivado/<corpus>/`, as
/// `tools/vivado-parity/achieved.sh` asks for it, into `out`, started and
/// not waited for.
fn start(root: &str, corpus: &str, name: &str, seed: &str, out: &Path) -> Child {
    let dir = manifest().join("testdata/fpga/vivado").join(corpus);
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap();
    Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .args(["--timing-driven", "--report", "--place-seed", seed])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(out)
        .arg("--bitstream")
        .arg(out.join(format!("{name}.bit")))
        .arg("--timing-endpoints")
        .arg(out.join("endpoints.txt"))
        .arg("--vivado-placement")
        .arg(out.join("x.tcl"))
        .arg(dir.join(format!("{name}.v")))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the reticle binary runs")
}

/// Every file a build wrote, by name, and its report with the output
/// directory taken out of it, which is the one thing the two runs were told
/// differently.
fn outputs(child: Child, out: &Path) -> (Vec<(String, Vec<u8>)>, String) {
    let done = child.wait_with_output().expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&done.stderr).replace(&out.display().to_string(), "<out>");
    assert!(done.status.success(), "{log}");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(out)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    (files, log)
}

/// Builds `name` twice at once and holds the second to the first's bytes.
fn twice(corpus: &str, name: &str, seed: &str) {
    let Some(root) = chipdb() else { return };
    if !manifest().join("testdata/fpga/vivado").is_dir() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let base: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reproducible");
    let (a, b) = (
        base.join(format!("{name}.a")),
        base.join(format!("{name}.b")),
    );
    let first = start(&root, corpus, name, seed, &a);
    let second = start(&root, corpus, name, seed, &b);
    let (files_a, log_a) = outputs(first, &a);
    let (files_b, log_b) = outputs(second, &b);
    if let Some((x, y)) = log_a.lines().zip(log_b.lines()).find(|(x, y)| x != y) {
        panic!("`{name}`: the two runs reported differently:\n  {x}\n  {y}");
    }
    assert_eq!(
        log_a.lines().count(),
        log_b.lines().count(),
        "`{name}`: one report is longer"
    );
    let names = |files: &[(String, Vec<u8>)]| files.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&files_a),
        names(&files_b),
        "`{name}`: different files"
    );
    for ((file, bytes_a), (_, bytes_b)) in files_a.iter().zip(&files_b) {
        assert!(
            bytes_a == bytes_b,
            "`{name}`: `{file}` differs between two runs of the same build"
        );
    }
    // The bitstream, the export and the routes are what this is about, and a
    // build that wrote none of them would pass for nothing.
    for wanted in [
        format!("{name}.bit"),
        "x.v".to_owned(),
        "x.routes.tcl".to_owned(),
    ] {
        assert!(
            names(&files_a).contains(&wanted),
            "`{name}`: no `{wanted}` was written"
        );
    }
}

/// The design whose placement panicked once (see the module's docs): lookup
/// table memories, three carry chains with generates tied on the bypass
/// input, and a clock buffer, at the seed it panicked at.
#[test]
fn a_build_with_tied_carries_and_lut_memories_writes_the_same_bytes_twice() {
    twice("qor", "regfile_alu", "16");
}

/// A block RAM, whose idle inputs are tied after routing from sets of
/// wires, and the FIFO's counters on the carry chain.
#[test]
fn a_build_with_a_block_ram_writes_the_same_bytes_twice() {
    twice("qor", "bram_fifo", "1");
}
