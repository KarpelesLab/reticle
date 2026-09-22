//! End-to-end tests of the `reticle` binary.
//!
//! These run the real executable over the checked-in corpus, so they cover
//! the argument parsing, the file I/O and the exit codes that the library
//! tests cannot reach. Everything is driven through a temporary directory
//! under the target directory, so the tests leave no trace elsewhere.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Runs the binary with the given arguments.
fn reticle(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(args)
        .output()
        .expect("failed to run the reticle binary")
}

/// The exit code, stdout and stderr of a run.
fn run(args: &[&str]) -> (i32, String, String) {
    let out = reticle(args);
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

#[test]
fn version_and_help() {
    let (code, stdout, _) = run(&["version"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("reticle "), "{stdout}");

    let (code, stdout, _) = run(&["help"]);
    assert_eq!(code, 0);
    for command in ["check", "synth", "emit", "sim", "verify"] {
        assert!(stdout.contains(command), "`{command}` missing from help");
    }

    // Per-command help is reachable and specific.
    let (code, stdout, _) = run(&["help", "verify"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--depth"), "{stdout}");
}

#[test]
fn usage_errors_exit_two() {
    assert_eq!(run(&[]).0, 2);
    assert_eq!(run(&["frobnicate"]).0, 2);
    assert_eq!(run(&["check"]).0, 2);

    let (code, _, stderr) = run(&["synth", "--dpeth", "3", "x.rtl"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown option `--dpeth`"), "{stderr}");

    let (code, _, stderr) = run(&["emit", "--format", "nope", "x.rtl"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("is not one of verilog"), "{stderr}");
}

#[test]
fn missing_file_is_an_error() {
    let (code, _, stderr) = run(&["check", "no/such/file.rtl"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("cannot read"), "{stderr}");
}

#[test]
fn check_accepts_every_frontend() {
    let (code, _, stderr) = run(&[
        "check",
        "testdata/ir/counter.rtl",
        "testdata/verilog/parse/counter.v",
        "testdata/vhdl/parse/counter.vhd",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("checked 3 file(s)"), "{stderr}");
}

#[test]
fn check_reports_a_broken_design() {
    let dir = scratch("check_broken");
    let path = dir.join("broken.rtl");
    std::fs::write(
        &path,
        "module m\n  net %a u8 wire\n  assign %a = frob(%a)\nend\n",
    )
    .unwrap();
    let (code, _, stderr) = run(&["check", path.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(stderr.contains("frob"), "{stderr}");
}

#[test]
fn synth_lowers_a_process_to_a_flip_flop() {
    let dir = scratch("synth_counter");
    let out = dir.join("counter.synth.rtl");
    let (code, _, stderr) = run(&[
        "synth",
        "--report",
        "--output",
        out.to_str().unwrap(),
        "testdata/ir/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");

    let netlist = std::fs::read_to_string(&out).unwrap();
    assert!(netlist.contains("dff"), "{netlist}");
    // The process form is gone: everything is cells now.
    assert!(!netlist.contains("process"), "{netlist}");
    // The report names the inferred storage.
    assert!(stderr.contains("flip-flop"), "{stderr}");
}

#[test]
fn synth_output_defaults_to_stdout() {
    let (code, stdout, stderr) = run(&["synth", "--quiet", "testdata/ir/counter.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("dff"), "{stdout}");
}

#[test]
fn emit_writes_verilog_and_vhdl() {
    let dir = scratch("emit_formats");
    let netlist = dir.join("counter.synth.rtl");
    let (code, _, stderr) = run(&[
        "synth",
        "--quiet",
        "--output",
        netlist.to_str().unwrap(),
        "testdata/ir/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");

    let (code, stdout, stderr) = run(&["emit", "--format", "verilog", netlist.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("module counter"), "{stdout}");
    assert!(stdout.contains("always @(posedge clk)"), "{stdout}");

    let (code, stdout, stderr) = run(&["emit", "--format", "vhdl", netlist.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("entity counter is"), "{stdout}");

    // `--output` puts it in a file instead.
    let v = dir.join("counter.v");
    let (code, stdout, stderr) = run(&[
        "emit",
        "--output",
        v.to_str().unwrap(),
        netlist.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.is_empty());
    assert!(std::fs::read_to_string(&v).unwrap().contains("endmodule"));
}

#[test]
fn sim_runs_a_testbench_and_dumps_a_waveform() {
    let dir = scratch("sim_counter");
    let vcd = dir.join("counter.vcd");
    let (code, stdout, stderr) = run(&[
        "sim",
        "--vcd",
        vcd.to_str().unwrap(),
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("q=10"), "{stdout}");
    assert!(stderr.contains("finished at time"), "{stderr}");

    let waves = std::fs::read_to_string(&vcd).unwrap();
    assert!(waves.starts_with("$timescale"), "{waves}");
    assert!(waves.contains("$scope module counter_tb $end"), "{waves}");
}

#[test]
fn sim_honours_the_time_limit() {
    let (code, stdout, stderr) = run(&["sim", "--until", "50", "testdata/sim/counter.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    // $finish happens well after time 50, so the testbench never prints.
    assert!(!stdout.contains("q="), "{stdout}");
    assert!(stderr.contains("time limit"), "{stderr}");
}

#[test]
fn verify_proves_a_good_design() {
    let (code, stdout, stderr) = run(&["verify", "--depth", "12", "testdata/formal/fifo_good.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("PROVED"), "{stdout}");
}

#[test]
fn verify_refutes_a_bad_design_and_writes_a_trace() {
    let dir = scratch("verify_bad");
    let trace = dir.join("cex.vcd");
    let (code, stdout, stderr) = run(&[
        "verify",
        "--depth",
        "8",
        "--trace",
        trace.to_str().unwrap(),
        "testdata/formal/fifo_bad.rtl",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.contains("FAIL"), "{stdout}");

    let waves = std::fs::read_to_string(&trace).unwrap();
    assert!(waves.contains("$scope module fifo $end"), "{waves}");
}
