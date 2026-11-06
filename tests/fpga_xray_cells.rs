//! The 7-series cells a design may instantiate by name
//! (`src/fpga/devices/xc7_cells.v`) against what Vivado's own simulator says
//! each one does.
//!
//! Each file in `testdata/fpga/vivado/cells/` is a testbench holding inputs
//! and the outputs the vendor's model answered with under `xsim`
//! (`tools/vivado-parity/gen_cell_vectors.py` made them, and Vivado is not
//! needed to run them). It would catch a cell written from a wrong reading: the
//! first run found every `RAM16X2S`-shaped memory with the wrong pins and an
//! `IS_D_INVERTED` the latches do not have. It would not catch a mistake in
//! Reticle's simulator and its synthesis that agree, and it says nothing of
//! what the part does: that is `examples/` and a board.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The vector files, or `None` with a line saying they are not there.
fn vectors() -> Option<Vec<PathBuf>> {
    let Ok(dir) = std::fs::read_dir(manifest().join("testdata/fpga/vivado/cells")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut files: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
    files.sort();
    Some(files)
}

/// **Every vector file gets the answers Vivado's simulator gave.**
#[test]
fn every_cell_answers_as_the_vendors_model_does() {
    let Some(files) = vectors() else { return };
    let library = manifest().join("src/fpga/devices/xc7_cells.v");
    let mut wrong = Vec::new();
    for file in &files {
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(["sim", "--top", "tb"])
            .arg(&library)
            .arg(file)
            .output()
            .expect("failed to run the reticle binary");
        let out = String::from_utf8_lossy(&run.stdout);
        if !out.lines().any(|l| l == "PASS") {
            let first = out.lines().next().unwrap_or("no output").to_owned();
            let err = String::from_utf8_lossy(&run.stderr);
            wrong.push(format!("{}: {first} {err}", file.display()));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {}:\n{}",
        wrong.len(),
        files.len(),
        wrong.join("\n")
    );
}

/// **No cell gets into the library unasked.** Every module has vectors but
/// those that are only other cells, and which those are is listed here, so a
/// cell added without a vector run fails this.
#[test]
fn every_cell_in_the_library_has_vectors() {
    let Some(files) = vectors() else { return };
    let library = std::fs::read_to_string(manifest().join("src/fpga/devices/xc7_cells.v")).unwrap();
    let have: Vec<String> = files
        .iter()
        .filter_map(|f| f.file_stem()?.to_str().map(str::to_owned))
        .collect();
    let mut missing = Vec::new();
    for line in library.lines() {
        let Some(rest) = line.strip_prefix("module ") else {
            continue;
        };
        let name = rest.split([' ', '(']).next().unwrap_or("");
        if [
            "BUFGP",
            "BUFGCE_1",
            "BUFGMUX_1",
            "BUFGMUX_CTRL",
            "BUFH",
            "BUFMR",
            "PLLE2_BASE",
            "MMCME2_BASE",
            "IBUFGDS",
            "IOBUF",
            "RAM32M_1",
            "RAM64M_1",
            "RAM128X1D_1",
        ]
        .contains(&name)
        {
            continue;
        }
        if !have.iter().any(|h| h == &name.to_ascii_lowercase()) {
            missing.push(name.to_owned());
        }
    }
    assert!(missing.is_empty(), "no vectors: {}", missing.join(" "));
}

/// Every part the probes were measured on, and the directory of its ledger:
/// the XC7A100T's beside the designs, each other part's in a directory of its
/// own name there, with its own constraints when its package is not the
/// FTG256 (`tools/vivado-parity/gen_primitives.py`).
const PARTS: [(&str, &str); 3] = [
    ("xc7a100t-ftg256", ""),
    ("xc7a35t-ftg256", "xc7a35t-ftg256"),
    ("xc7a35t-cpg236", "xc7a35t-cpg236"),
];

/// The ledger of probes on one part: `(primitive, status)`, or `None`
/// without fixtures.
fn ledger(part: &str) -> Option<Vec<(String, String)>> {
    let dir = manifest()
        .join("testdata/fpga/vivado/primitives")
        .join(part);
    let Ok(text) = std::fs::read_to_string(dir.join("index.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    Some(
        text.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut words = l.split_whitespace().skip(1);
                Some((words.next()?.to_owned(), words.next()?.to_owned()))
            })
            .collect(),
    )
}

/// How a probe's build went.
#[derive(PartialEq, Eq)]
enum Built {
    /// It finished, and with a bitstream when one was asked for.
    Done,
    /// It finished, and said the design is not fully routed: a bitstream
    /// that configures less than the design asks for.
    Unrouted,
    /// It did not finish.
    Failed,
}

/// Runs `reticle fpga` on one probe for one part, with or without a
/// bitstream. A part whose package is not the FTG256 has constraints of its
/// own; the designs are the same.
fn build_probe(device: &str, part: &str, primitive: &str, bitstream: bool) -> Built {
    let dir = manifest().join("testdata/fpga/vivado/primitives");
    let top = format!("{}_probe", primitive.to_ascii_lowercase());
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("probe-{device}-{top}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let own = dir.join(part).join(format!("{top}.rcf"));
    let rcf = if own.is_file() {
        own
    } else {
        dir.join(format!("{top}.rcf"))
    };
    let mut run = Command::new(env!("CARGO_BIN_EXE_reticle"));
    run.args(["fpga", "--device", device, "--quiet", "--top", &top])
        .arg("--constraints")
        .arg(rcf)
        .arg("--output-dir")
        .arg(&out)
        .arg(dir.join(format!("{top}.v")));
    if bitstream {
        run.arg("--bitstream").arg(out.join(format!("{top}.bit")));
    }
    let run = run.output().expect("failed to run the reticle binary");
    if !run.status.success() {
        Built::Failed
    } else if String::from_utf8_lossy(&run.stderr).contains("NOT FULLY ROUTED") {
        Built::Unrouted
    } else {
        Built::Done
    }
}

/// **What each part's ledger calls `ok` maps**, on that part. Vivado takes
/// every one of these probes; this keeps a primitive Reticle takes from being
/// lost, and needs no fabric database because it stops short of placing.
/// It would catch a part's device file refusing what Vivado builds there:
/// `xc7a35t-cpg236` refused sixteen probes until 2026-11-06, every `LVDS_25`
/// pair and both `SSTL18_I` inputs, because its file listed the Basys 3's
/// five standards. It would not catch a probe that maps and then fails to
/// route: the ignored test below builds them.
#[test]
fn the_primitives_the_ledger_calls_ok_map() {
    for (device, part) in PARTS {
        let Some(ledger) = ledger(part) else { return };
        let lost: Vec<&str> = ledger
            .iter()
            .filter(|(primitive, status)| {
                status == "ok" && build_probe(device, part, primitive, false) != Built::Done
            })
            .map(|(primitive, _)| primitive.as_str())
            .collect();
        assert!(
            lost.is_empty(),
            "{device}: no longer map: {}",
            lost.join(" ")
        );
    }
}

/// **Each part's ledger is exact, to a fully routed bitstream**: a primitive
/// it calls `ok` builds, with every signal routed, and one it calls `gap`
/// does not, so closing a gap has to say so here. The routing was not asked
/// before 2026-11-06, and `XADC` was `ok` on every part with its `CONVST`
/// unrouted: the fabric loaded around the XADC stopped at its tile's own
/// row, and that pin reaches the interconnect a row above. Tens of minutes on
/// one core, which is why it is ignored; `cargo test -- --ignored`.
#[test]
#[ignore = "builds every probe of three parts to a bitstream, and needs the Project X-Ray database"]
fn the_ledger_says_what_builds() {
    let mut wrong: Vec<String> = Vec::new();
    for (device, part) in PARTS {
        let Some(ledger) = ledger(part) else { return };
        wrong.extend(
            ledger
                .iter()
                .filter(|(primitive, status)| {
                    (status == "ok") != (build_probe(device, part, primitive, true) == Built::Done)
                })
                .map(|(primitive, status)| {
                    format!("{device} {primitive}: the ledger says {status}")
                }),
        );
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// **`--decode` reads what `--bitstream` wrote**: a global buffer's feature
/// among the named lines and nothing unnamed. Would catch the option losing
/// the database's names, or a frame read in the wrong order. Whether it reads
/// Vivado's `.bit` as well is `xc7::read_bit`'s question, which
/// `tests/fpga_xray.rs` asks of one; the unnamed lines are not reached here.
#[test]
fn decode_reads_a_bitstream_back() {
    let Some(_) = ledger("") else { return };
    let dir = manifest().join("testdata/fpga/vivado/primitives");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("probe-decode");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("bufg.bit");
    let built = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--quiet",
            "--offline",
        ])
        .args(["--top", "bufg_probe", "--constraints"])
        .arg(dir.join("bufg_probe.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("bufg_probe.v"))
        .output()
        .expect("failed to run the reticle binary");
    if !bit.is_file() {
        eprintln!(
            "skipped: no bitstream, probably no database: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        return;
    }
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--offline",
            "--decode",
        ])
        .arg(&bit)
        .output()
        .expect("failed to run the reticle binary");
    assert!(decoded.status.success());
    let text = String::from_utf8_lossy(&decoded.stdout);
    assert!(
        text.lines()
            .any(|l| l.contains("BUFGCTRL") && l.ends_with(".IN_USE")),
        "{text}"
    );
    assert!(!text.lines().any(|l| l.starts_with("? ")), "{text}");
}
