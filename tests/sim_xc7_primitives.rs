//! The 7-series hard blocks a simulation takes from
//! `src/fpga/devices/xc7_sim.v`, against what Vivado's simulator printed for
//! the same testbenches.
//!
//! Each `testdata/sim/xc7/<name>.v` is a testbench written here around one
//! kind of block, and `<name>.xsim` is what it printed under `xsim` with
//! Vivado 2026.1's `unisims_ver`, `secureip` and `glbl` (`tools/vivado-parity/
//! gen_sim_xc7.sh` writes them, and Vivado is not needed to run this). The
//! testbench runs under `reticle sim --device xc7a35t-cpg236`, which is how a
//! user would run it, so the library comes in by name and nothing passes its
//! source. Its output must be the same, byte for byte.
//!
//! It would catch a model written from a wrong reading of the documentation,
//! which the first runs did: a `RAMB18E1`'s narrow parity bit is bit 8 of
//! `INIT_A`, not 16; an `ODDR` in `SAME_EDGE` loads its falling-edge output only
//! if CE was high at the rising edge before; the global reset holds a clock
//! buffer low whatever its `INIT_OUT`. It would catch a lookup table that
//! reads `x` for an unknown input its function ignores, which left the routed
//! netlist of the language corpus's `v_alu` all `x` (`lut_unknown_inputs`),
//! and a block RAM's byte enables or a DSP's pre-adder modelled wrong in the
//! configurations Vivado chose for that corpus (`ramb18e1_byte_write`,
//! `dsp48e1_preadd`; mutated, each fails). It would not catch a behaviour no
//! testbench exercises, and each model's comment says which those are.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The testbenches, or `None` with a line saying they are not there.
fn testbenches() -> Option<Vec<PathBuf>> {
    let Ok(dir) = std::fs::read_dir(manifest().join("testdata/sim/xc7")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut files: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .collect();
    files.sort();
    Some(files)
}

/// **Every testbench prints what it printed under `xsim`.**
#[test]
fn every_testbench_prints_what_xsim_printed() {
    let Some(files) = testbenches() else { return };
    assert!(files.len() >= 10, "{} testbenches", files.len());
    let mut wrong = Vec::new();
    for file in &files {
        let expected = std::fs::read_to_string(file.with_extension("xsim"))
            .unwrap_or_else(|e| panic!("{}: no answer beside it: {e}", file.display()));
        let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args([
                "sim",
                "--quiet",
                "--top",
                "tb",
                "--device",
                "xc7a35t-cpg236",
            ])
            .arg(file)
            .output()
            .expect("failed to run the reticle binary");
        let out = String::from_utf8_lossy(&run.stdout);
        let err = String::from_utf8_lossy(&run.stderr);
        if !run.status.success() || !err.is_empty() {
            wrong.push(format!("{}: {err}", file.display()));
            continue;
        }
        if out != expected {
            let (n, (ours, theirs)) = out
                .lines()
                .zip(expected.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .unwrap_or((0, ("(a line count)", "(differs)")));
            wrong.push(format!(
                "{}: line {}:\n  reticle {ours}\n  xsim    {theirs}",
                file.display(),
                n + 1
            ));
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

/// **Every block the library models is in some testbench.** A model added
/// without asking `xsim` fails this; the helpers the models share are the
/// exceptions, named here.
#[test]
fn every_model_has_a_testbench() {
    let Some(files) = testbenches() else { return };
    let library = std::fs::read_to_string(manifest().join("src/fpga/devices/xc7_sim.v")).unwrap();
    let benches: Vec<String> = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    let mut missing = Vec::new();
    for line in library.lines() {
        let Some(rest) = line.strip_prefix("module ") else {
            continue;
        };
        let name = rest.split([' ', '(']).next().unwrap_or("");
        // KEEPER is there so that a design naming it elaborates; it does
        // nothing, since holding a weak level needs strengths the simulator
        // has not got, and no testbench could agree with xsim on it.
        if name.starts_with("xc7_sim_") || name == "KEEPER" {
            continue;
        }
        let used = benches.iter().any(|b| {
            b.lines()
                .any(|l| l.trim_start().starts_with(&format!("{name} ")))
        });
        if !used {
            missing.push(name.to_owned());
        }
    }
    assert!(missing.is_empty(), "no testbench: {}", missing.join(" "));
}

/// **A build never takes a hard block's model.** The library is for a
/// simulator; a build that read it would map a `RAMB18E1` onto logic.
#[test]
fn a_build_does_not_read_the_simulation_library() {
    for name in ["RAMB18E1", "IDDR", "BUFGCTRL", "PLLE2_ADV"] {
        let cell = reticle::fpga::cell_source("xc7", &[name.to_owned()]);
        assert!(cell.is_none(), "{name} is in the build's cell library");
        let sim = reticle::fpga::simulation_source("xc7", &[name.to_owned()]);
        assert!(sim.is_some(), "{name} is not in the simulation library");
    }
}

/// **A FIFO on two clocks is refused, by name.** Its model knows one clock
/// (`EN_SYN TRUE`); given `EN_SYN FALSE`, the default, it must not hand out
/// one clock's answers, and the refusal names why. Would catch the guard
/// removed; it would not catch a two-clock model added, which should replace
/// this with a testbench.
#[test]
fn a_fifo_on_two_clocks_is_refused() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sim_xc7_fifo_two_clocks");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("tb.v");
    std::fs::write(
        &file,
        "module tb;\n\
         \x20 reg c = 0;\n\
         \x20 FIFO18E1 #(.DATA_WIDTH(18)) u (.DI(32'h0), .DIP(4'h0), .RDCLK(c), .WRCLK(c),\n\
         \x20   .RDEN(1'b0), .WREN(1'b0), .REGCE(1'b1), .RST(1'b0), .RSTREG(1'b0));\n\
         endmodule\n",
    )
    .unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "sim",
            "--quiet",
            "--top",
            "tb",
            "--device",
            "xc7a35t-cpg236",
        ])
        .arg(&file)
        .output()
        .expect("failed to run the reticle binary");
    assert!(!run.status.success());
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(err.contains("EN_SYN_FALSE_is_not_modelled"), "{err}");
}

/// **Without `--device` a hard block is an undeclared module, and the
/// simulator says so** rather than running a black box.
#[test]
fn a_hard_block_needs_the_device() {
    let Some(files) = testbenches() else { return };
    let iddr = files
        .iter()
        .find(|f| f.file_stem().is_some_and(|s| s == "iddr"))
        .expect("the IDDR testbench");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--quiet", "--top", "tb"])
        .arg(iddr)
        .output()
        .expect("failed to run the reticle binary");
    assert!(!run.status.success());
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(err.contains("IDDR"), "{err}");
}
