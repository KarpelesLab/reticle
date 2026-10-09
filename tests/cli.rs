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

/// `reticle build` resolves a project's dependencies and elaborates the
/// whole graph. The corpus projects use relative `path` dependencies, so
/// the test copies one into a scratch directory and builds it there.
#[test]
fn build_resolves_and_elaborates_a_project() {
    let dir = scratch("build_project");
    copy_dir(Path::new("testdata/ip"), &dir);
    let project = dir.join("projects/two_deps");

    let out = project.join("design.rtl");
    let (code, _, stderr) = run(&[
        "build",
        "--report",
        "--output",
        out.to_str().unwrap(),
        project.join("reticle.proj").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    // The diamond resolved: the top plus all three dependencies.
    assert!(stderr.contains("4 module(s)"), "{stderr}");

    let rtl = std::fs::read_to_string(&out).unwrap();
    for module in ["top", "uart_lite", "fifo_sync", "cdc_sync"] {
        assert!(
            rtl.contains(&format!("module {module}")),
            "{module} missing"
        );
    }
    // A lock file is written next to the manifest by default.
    assert!(project.join("reticle.lock").exists(), "no lock file");
}

/// `reticle build` places a dependency that names no `path` by walking
/// the project's `library` roots, and records where it found each.
///
/// What this would catch: a walk that did not recurse into category
/// folders, a walk whose paths were not relative to the manifest, an
/// index that never reached the provider, and a lock file that did not
/// say the library answered. What it would not catch: anything about the
/// order two roots are searched in, which only an ambiguous name could
/// show, and that is an error rather than an order.
#[test]
fn build_places_dependencies_from_the_library() {
    let dir = scratch("build_library");
    copy_dir(Path::new("testdata/ip"), &dir);
    let project = dir.join("projects/library");

    let out = project.join("design.rtl");
    let (code, _, stderr) = run(&[
        "build",
        "--report",
        "--output",
        out.to_str().unwrap(),
        project.join("reticle.proj").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("3 module(s)"), "{stderr}");
    assert!(
        stderr.contains("the IP library holds 2 package(s) under `../../library`"),
        "{stderr}"
    );

    let rtl = std::fs::read_to_string(&out).unwrap();
    for module in ["top", "clock_div", "pulse_edge"] {
        assert!(
            rtl.contains(&format!("module {module}")),
            "{module} missing"
        );
    }

    let lock = std::fs::read_to_string(project.join("reticle.lock")).expect("no lock file");
    assert!(
        lock.contains("package clock_div 1.0.0 library ../../library/timing/divider"),
        "{lock}"
    );
    assert!(
        lock.contains("package pulse_edge 1.0.0 library ../../library/glue/edge"),
        "{lock}"
    );

    // --locked against the lock file that was just written is silence.
    let (code, _, stderr) = run(&[
        "build",
        "--locked",
        "--quiet",
        project.join("reticle.proj").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
}

/// A lock file that no longer describes the resolution is an error under
/// `--locked`, naming what moved, and the file is not rewritten.
///
/// This is the answer to "what makes a stale lock file an error rather
/// than a wrong build": nothing resolves *from* the lock file, so the
/// only way it can be wrong is to disagree, and `--locked` is the build
/// that refuses to disagree quietly.
#[test]
fn build_refuses_a_stale_lock_file() {
    let dir = scratch("build_locked");
    copy_dir(Path::new("testdata/ip"), &dir);
    let project = dir.join("projects/library");
    let manifest = project.join("reticle.proj");
    let lock_path = project.join("reticle.lock");

    let (code, _, stderr) = run(&["build", "--quiet", manifest.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");

    // The library moved under the lock file's feet.
    let lock = std::fs::read_to_string(&lock_path).unwrap();
    let stale = lock.replace(
        "library ../../library/timing/divider",
        "library ../../library/divider",
    );
    assert_ne!(stale, lock, "the lock file did not record a directory");
    std::fs::write(&lock_path, &stale).unwrap();

    let (code, _, stderr) = run(&["build", "--locked", manifest.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("error[P0302]"), "{stderr}");
    assert!(
        stderr.contains("the lock file does not describe this resolution"),
        "{stderr}"
    );
    assert!(stderr.contains("`clock_div` moves from"), "{stderr}");
    // And it was left as it was, rather than quietly corrected.
    assert_eq!(std::fs::read_to_string(&lock_path).unwrap(), stale);
}

/// A `library` root that is not a directory is reported, rather than
/// becoming an empty index and a pile of missing packages.
#[test]
fn build_reports_a_library_root_that_is_not_there() {
    let dir = scratch("build_bad_library");
    copy_dir(Path::new("testdata/ip"), &dir);
    let manifest = dir.join("projects/two_deps/reticle.proj");
    let (code, _, stderr) = run(&[
        "build",
        "--library",
        "../../nowhere",
        manifest.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("the IP library root `../../nowhere` is not a directory"),
        "{stderr}"
    );
}

#[test]
fn build_reports_a_version_conflict() {
    let dir = scratch("build_conflict");
    copy_dir(Path::new("testdata/ip"), &dir);
    let manifest = dir.join("projects/conflict/reticle.proj");
    let (code, _, stderr) = run(&["build", manifest.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("satisfies every requirement"), "{stderr}");
}

/// Copies a directory tree, which the build tests need because a project
/// writes its lock file next to its manifest.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn search_finds_packages_in_an_index() {
    let (code, stdout, stderr) = run(&["search", "--index", "testdata/ip/registry/index", "fifo"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("fifo_sync"), "{stdout}");
}

#[test]
fn add_writes_a_dependency_into_the_manifest() {
    let dir = scratch("add_dep");
    copy_dir(Path::new("testdata/ip"), &dir);
    let manifest = dir.join("projects/mixed/reticle.proj");
    let index = dir.join("registry/index");

    // --dry-run prints the result and leaves the file alone.
    let before = std::fs::read_to_string(&manifest).unwrap();
    let (code, stdout, stderr) = run(&[
        "add",
        "--dry-run",
        "--index",
        index.to_str().unwrap(),
        "--project",
        manifest.to_str().unwrap(),
        "uart_lite",
        "^1.2.0",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("depends uart_lite ^1.2.0"), "{stdout}");
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), before);

    // Without it the manifest is rewritten, keeping its comments.
    let (code, _, stderr) = run(&[
        "add",
        "--index",
        index.to_str().unwrap(),
        "--project",
        manifest.to_str().unwrap(),
        "uart_lite",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let after = std::fs::read_to_string(&manifest).unwrap();
    assert!(after.contains("depends uart_lite"), "{after}");
    assert!(
        after.contains("# A project whose top is Verilog"),
        "comments lost"
    );
}

#[test]
fn asic_maps_onto_a_liberty_library() {
    let (code, stdout, stderr) = run(&[
        "asic",
        "--liberty",
        "testdata/asic/reticle_sc.lib",
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("mapped onto"), "{stdout}");
    assert!(stdout.contains("DFF"), "{stdout}");
}

/// A library of only an inverter and a two-input NAND is functionally
/// complete. It used to panic the mapper, which had no way to invert a
/// gate's output and so could not build a plain AND.
#[test]
fn asic_maps_onto_a_minimal_library_without_panicking() {
    let (code, stdout, stderr) = run(&[
        "asic",
        "--liberty",
        "testdata/asic/cells.lib",
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(stdout.contains("nand2"), "{stdout}");
}

#[test]
fn sim_runs_an_interactive_script() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["sim", "--interactive", "testdata/sim/counter.rtl"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn reticle");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"run 20000\nprint q\nquit\n").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("ran to time 20000"), "{stdout}");
    assert!(stdout.contains("counter_tb.q"), "{stdout}");
    // No prompt is printed when standard input is not a terminal, so a
    // scripted transcript stays clean.
    assert!(!stdout.contains("reticle>"), "{stdout}");
}

#[test]
fn lsp_answers_the_initialize_handshake() {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    let message = format!("Content-Length: {}\r\n\r\n{body}", body.len());
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_reticle"))
        .arg("lsp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn reticle");
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(message.as_bytes())
            .unwrap();
    }
    // Closing standard input ends the session, so the server exits.
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("Content-Length:"), "{stdout}");
    assert!(stdout.contains("\"capabilities\""), "{stdout}");
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
fn synth_reads_verilog_directly() {
    let (code, stdout, stderr) = run(&["synth", "--quiet", "testdata/verilog/parse/counter.v"]);
    assert_eq!(code, 0, "{stderr}");
    // The always block became a flip-flop, so the process form is gone.
    assert!(stdout.contains("dff"), "{stdout}");
    assert!(!stdout.contains("process"), "{stdout}");
}

#[test]
fn sim_elaborates_several_verilog_files_together() {
    let (code, stdout, stderr) = run(&[
        "sim",
        "testdata/verilog/elab/testbench.v",
        "testdata/verilog/elab/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    // $monitor output from the testbench, with the DUT actually counting.
    assert!(stdout.contains("t=0 q=0"), "{stdout}");
    assert!(stdout.contains("q=4"), "{stdout}");
}

#[test]
fn design_inputs_must_be_one_kind() {
    let (code, _, stderr) = run(&[
        "synth",
        "testdata/ir/counter.rtl",
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("same kind"), "{stderr}");

    let (code, _, stderr) = run(&[
        "synth",
        "testdata/ir/counter.rtl",
        "testdata/vhdl/elab/counter_sync.vhd",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("same kind"), "{stderr}");
}

#[test]
fn synth_reads_vhdl_directly() {
    let (code, stdout, stderr) = run(&["synth", "--quiet", "testdata/vhdl/elab/counter_sync.vhd"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("dff"), "{stdout}");

    // An `if rst = '1' ... elsif rising_edge(clk)` process is an
    // asynchronous reset, and must not be demoted to a synchronous one.
    let (code, stdout, stderr) = run(&["synth", "--quiet", "testdata/vhdl/elab/counter_async.vhd"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("arst"),
        "expected an async reset:\n{stdout}"
    );
    assert!(
        !stderr.contains("no asynchronous reset inferred"),
        "{stderr}"
    );
}

#[test]
fn fmt_formats_checks_and_writes() {
    let source = "testdata/verilog/parse/counter.v";

    // Default: formatted text on stdout, file untouched.
    let before = std::fs::read_to_string(source).unwrap();
    let (code, stdout, stderr) = run(&["fmt", source]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("module counter"), "{stdout}");
    assert_eq!(std::fs::read_to_string(source).unwrap(), before);

    // A deliberately misformatted copy: --check reports it and fails.
    let dir = scratch("fmt_write");
    let ugly = dir.join("ugly.v");
    std::fs::write(
        &ugly,
        "module m(input a,output y);assign y=a;endmodule
",
    )
    .unwrap();
    let (code, _, stderr) = run(&["fmt", "--check", ugly.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("would reformat"), "{stderr}");

    // --write fixes it, and a second --check then passes.
    let (code, _, stderr) = run(&["fmt", "--write", ugly.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("reformatted 1 file"), "{stderr}");
    let (code, _, _) = run(&["fmt", "--check", ugly.to_str().unwrap()]);
    assert_eq!(code, 0);

    // --write and --check contradict each other.
    let (code, _, stderr) = run(&["fmt", "--write", "--check", source]);
    assert_eq!(code, 2);
    assert!(stderr.contains("pick one"), "{stderr}");
}

#[test]
fn fmt_refuses_an_unparseable_file() {
    let dir = scratch("fmt_broken");
    let broken = dir.join("broken.v");
    std::fs::write(
        &broken,
        "module m(;;; endmodule
",
    )
    .unwrap();
    let (code, stdout, _) = run(&["fmt", broken.to_str().unwrap()]);
    assert_eq!(code, 1);
    // Nothing was emitted, so a bad file can never be truncated by a
    // careless shell redirect.
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(&broken).unwrap(),
        "module m(;;; endmodule\n"
    );
}

#[test]
fn synth_maps_to_luts_and_gates() {
    let (code, stdout, stderr) = run(&[
        "synth",
        "--lut",
        "4",
        "--quiet",
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("lut 4"), "{stdout}");

    let (code, stdout, stderr) = run(&[
        "synth",
        "--gates",
        "--quiet",
        "testdata/verilog/parse/counter.v",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("blackbox"), "{stdout}");

    // Option validation happens before any file is opened, so a bad flag is
    // a usage error even when the design does not exist.
    let (code, _, stderr) = run(&["synth", "--lut", "9", "nowhere.rtl"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("out of range"), "{stderr}");

    let (code, _, stderr) = run(&["synth", "--lut", "4", "--gates", "nowhere.rtl"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("pick one"), "{stderr}");
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
fn fpga_lists_devices() {
    let (code, stdout, stderr) = run(&["fpga", "--list-devices"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("ice40-hx1k-tq144"), "{stdout}");
    assert!(stdout.contains("ecp5-45f-CABGA381"), "{stdout}");
}

#[test]
fn fpga_runs_the_whole_flow() {
    let dir = scratch("fpga_flow");
    let (code, _, stderr) = run(&[
        "fpga",
        "--device",
        "ice40-hx1k-tq144",
        "--constraints",
        "testdata/fpga/blinky_ice40.rcf",
        "--output-dir",
        dir.to_str().unwrap(),
        "--report",
        "testdata/fpga/blinky_ice40.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    // The report names the device primitives the design mapped onto.
    assert!(stderr.contains("SB_LUT4"), "{stderr}");
    assert!(stderr.contains("nextpnr-ice40"), "{stderr}");

    let json = std::fs::read_to_string(dir.join("blinky.json")).unwrap();
    // Every cell must be a device primitive: a generic `$` cell would be
    // rejected by nextpnr.
    assert!(
        !json.contains("\"type\": \"$"),
        "generic cell in the netlist"
    );
    assert!(json.contains("SB_LUT4"), "{json}");

    let pcf = std::fs::read_to_string(dir.join("blinky.pcf")).unwrap();
    assert!(pcf.contains("set_io clk"), "{pcf}");
}

#[test]
fn fpga_needs_a_known_device() {
    let (code, _, stderr) = run(&["fpga", "testdata/fpga/blinky_ice40.rtl"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("no device given"), "{stderr}");

    let (code, _, stderr) = run(&[
        "fpga",
        "--device",
        "nosuchpart",
        "testdata/fpga/blinky_ice40.rtl",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("unknown device"), "{stderr}");
}

#[test]
fn fpga_checks_constraints_against_the_device() {
    let dir = scratch("fpga_wrong_pins");
    // ECP5 pin names on an iCE40 part: the checker must catch it before
    // anything is written.
    let (code, _, stderr) = run(&[
        "fpga",
        "--device",
        "ice40-hx1k-tq144",
        "--constraints",
        "testdata/fpga/blinky_ecp5.rcf",
        "--output-dir",
        dir.to_str().unwrap(),
        "testdata/fpga/blinky_ice40.rtl",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("has no pin named"), "{stderr}");
    assert!(!dir.join("blinky.json").exists(), "wrote a netlist anyway");
}

/// The case that found the defect: the Cynthion ULPI device built without
/// the IP sources it instantiates. It used to print a wall of errors about
/// the *top level*, one per reader of an output of the missing instance,
/// and never say that `usb_device_ulpi` was nowhere to be found. Now the
/// missing module is the only error, and it points at the instantiation.
#[test]
fn fpga_names_the_module_it_cannot_find() {
    let dir = scratch("fpga_missing_module");
    let (code, _, stderr) = run(&[
        "fpga",
        "--device",
        "ecp5-12f-CABGA256",
        "--constraints",
        "testdata/fpga/cynthion/usb_ulpi_device.rcf",
        "--output-dir",
        dir.to_str().unwrap(),
        "testdata/fpga/cynthion/usb_ulpi_device.v",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("error[I0034]: no module named `usb_device_ulpi` is defined"),
        "{stderr}"
    );
    assert!(stderr.contains("`u_dev` instantiates it"), "{stderr}");
    assert!(
        stderr.contains("add the file that defines it to this build"),
        "{stderr}"
    );

    // And the misdirection is gone: nothing about the top level's own
    // signals, which are driven right there in that file.
    assert!(!stderr.contains("which nothing drives"), "{stderr}");
    assert!(!stderr.contains("an instance is left"), "{stderr}");
    // One error, not a hundred.
    assert_eq!(
        stderr.matches("error[I0034]").count(),
        1,
        "the cause should be reported once: {stderr}"
    );
    assert!(
        !dir.join("usb_ulpi_device.json").exists(),
        "wrote a netlist"
    );
}

#[test]
fn sim_writes_an_fst_waveform() {
    let dir = scratch("sim_fst");
    let fst = dir.join("counter.fst");
    let (code, _, stderr) = run(&[
        "sim",
        "--fst",
        fst.to_str().unwrap(),
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");

    // Compared through the crate's own FST reader rather than byte for
    // byte: the writer stamps `Reticle <version>` into the header, so a
    // byte comparison against a golden breaks on every release. What must
    // match is what a waveform viewer would show, so compare the decoded
    // variables and value changes. The golden itself is the one the FST
    // tests check against GTKWave's reference reader, so the command's
    // output is covered by that validation too.
    let written =
        reticle::sim::fst::read(&std::fs::read(&fst).unwrap()).expect("the command's FST decodes");
    let golden = reticle::sim::fst::read(&std::fs::read("testdata/sim/fst/counter.fst").unwrap())
        .expect("the golden decodes");
    assert_eq!(
        written.vars, golden.vars,
        "the command's FST declares different variables"
    );
    assert_eq!(
        written.changes, golden.changes,
        "the command's FST holds different value changes"
    );
    assert_eq!(written.end_time, golden.end_time);
}

#[test]
fn sim_checks_assertions() {
    // A property the counter satisfies.
    let (code, _, stderr) = run(&[
        "sim",
        "--quiet",
        "--assert",
        "assert property (@(posedge clk) rst |=> q == 0);",
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("held over"), "{stderr}");

    // One it does not: the counter passes 8, and a failing assertion
    // must make the run fail so it can gate a build.
    let (code, _, stderr) = run(&[
        "sim",
        "--quiet",
        "--assert",
        "assert property (@(posedge clk) q < 8);",
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("FAILED"), "{stderr}");
    // The report says what the values were, not just that it broke.
    assert!(stderr.contains("sampled values"), "{stderr}");
}

#[test]
fn sim_writes_coverage() {
    let dir = scratch("sim_coverage");

    let text = dir.join("cov.txt");
    let (code, _, stderr) = run(&[
        "sim",
        "--quiet",
        "--coverage",
        text.to_str().unwrap(),
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let report = std::fs::read_to_string(&text).unwrap();
    assert!(report.contains("line coverage"), "{report}");

    // A `.info` name selects LCOV, so existing viewers can read it.
    let lcov = dir.join("cov.info");
    let (code, _, stderr) = run(&[
        "sim",
        "--quiet",
        "--coverage",
        lcov.to_str().unwrap(),
        "testdata/sim/counter.rtl",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let report = std::fs::read_to_string(&lcov).unwrap();
    assert!(report.starts_with("TN:"), "{report}");
    assert!(report.contains("SF:"), "{report}");
}

/// `$readmemh` reads its file through a provider the library is handed,
/// since the library performs no I/O. The command must hand it one that
/// resolves a relative name against the design's own directory, or every
/// `$readmemh` in a design run from the command line fails.
#[test]
fn sim_loads_readmemh_files_next_to_the_design() {
    let dir = scratch("sim_readmem");
    std::fs::copy("testdata/sim/memory.rtl", dir.join("memory.rtl")).unwrap();
    let expect = std::fs::read_to_string("testdata/sim/memory.expect").unwrap();
    let hex: String = expect
        .split("file data.hex\n")
        .nth(1)
        .and_then(|rest| rest.split("end-file").next())
        .expect("the golden carries data.hex")
        .to_string();
    std::fs::write(dir.join("data.hex"), &hex).unwrap();

    let design = dir.join("memory.rtl");
    let (code, stdout, stderr) = run(&["sim", "--quiet", design.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    // `hex1` is the second byte of the file, so it only comes out right if
    // the file was actually read.
    assert!(stdout.contains("hex1=cd"), "{stdout}");

    // Without the file the run says which one it could not read.
    std::fs::remove_file(dir.join("data.hex")).unwrap();
    let (_, _, stderr) = run(&["sim", "--quiet", design.to_str().unwrap()]);
    assert!(stderr.contains("data.hex"), "{stderr}");
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
fn timing_reports_slack_with_a_path() {
    let (code, stdout, stderr) = run(&["timing", "--paths", "1", "testdata/timing/reg2reg.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("timing summary"), "{stdout}");
    assert!(stdout.contains("slack"), "{stdout}");
    // The report walks the path pin by pin, not just a number.
    assert!(stdout.contains("startpoint:"), "{stdout}");
    assert!(stdout.contains("data required time"), "{stdout}");

    let (code, stdout, stderr) = run(&["timing", "--summary", "testdata/timing/reg2reg.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stdout.contains("startpoint:"), "--summary printed a path");
}

#[test]
fn timing_synthesises_source_first() {
    // A .v file still holds processes, which have no timing arcs; the
    // command must synthesise before analysing rather than report nothing.
    let (code, stdout, stderr) = run(&["timing", "testdata/verilog/parse/counter.v"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("timing summary"), "{stdout}");
    assert!(stdout.contains("q$ff/d"), "{stdout}");
}

#[test]
fn cdc_separates_a_bug_from_a_synchroniser() {
    // An unsynchronised crossing is the bug everyone is looking for, so it
    // is an error and the exit code says so.
    let (code, stdout, stderr) = run(&["timing", "--cdc", "testdata/timing/cdc_unsync.rtl"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.contains("unsynchronised"), "{stdout}");

    // A proper two-flop synchroniser is not.
    let (code, stdout, stderr) = run(&["timing", "--cdc", "testdata/timing/cdc_sync2.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("synchroniser"), "{stdout}");
}

#[test]
fn timing_survives_a_combinational_loop() {
    // Reported, not hung: the whole point of detecting the loop.
    let (code, stdout, stderr) = run(&["timing", "testdata/timing/comb_loop.rtl"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("combinational loop"), "{stdout}");
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

/// `--place-*` must reach the placer for a 7-series bitstream, not only for
/// an ECP5 one. Until this test, the 7-series writer placed with the
/// defaults whatever it was given and said nothing, so a table of "nine
/// placement settings" in `docs/fpga-xray.md` was one placement built nine
/// times.
///
/// It catches a writer that ignores `--place-effort`: with the defaults
/// forced, both builds are byte-identical. It would not catch one that
/// honours that flag and drops another, since it changes only one.
#[test]
fn place_effort_changes_a_seven_series_placement() {
    let dir = scratch("place_effort_xc7");
    let build = |name: &str, extra: &[&str]| {
        let bit = dir.join(name);
        let mut args = vec![
            "fpga",
            "examples/basys3/blink.v",
            "--device",
            "xc7a35t-cpg236",
            "--constraints",
            "examples/basys3/blink.rcf",
            "--output-dir",
            dir.to_str().unwrap(),
            "--bitstream",
            bit.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
            .args(&args)
            .env("RETICLE_OFFLINE", "1")
            .output()
            .expect("failed to run the reticle binary");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.code(), stderr, bit)
    };

    let (code, stderr, default_bit) = build("default.bit", &[]);
    if stderr.contains("needs a chip database") {
        eprintln!("skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db`");
        return;
    }
    assert_eq!(code, Some(0), "{stderr}");
    let (code, stderr, quick_bit) = build("effort1.bit", &["--place-effort", "1"]);
    assert_eq!(code, Some(0), "{stderr}");

    let default_bytes = std::fs::read(default_bit).unwrap();
    let quick_bytes = std::fs::read(quick_bit).unwrap();
    assert_ne!(
        default_bytes, quick_bytes,
        "`--place-effort 1` built the same bitstream as the default effort"
    );
}

/// Every clock pin of `examples/basys3/ssd1306_console.v` arrives on the
/// global clock network, at the placement that once put fifteen of them on
/// general interconnect.
///
/// That design, at `--place-effort 3`, is where a clock routed through
/// `FAN_BOUNCE` chains of 80 to 103 hops gave a hold violation on a Basys 3:
/// the status line's shift register captured its neighbour's new value and
/// every character read some bits from the next nibble. With the router's
/// steering removed the build is refused, naming 15 clock pins; this
/// requires it to build and to report every pin on the network.
///
/// What it would catch: the steering lost (the build is refused) or the
/// refusal lost along with it (the count stops being "N of N"). What it
/// would not catch: skew *on* the network, which this flow does not model
/// — only a part shows that.
///
/// Ignored because it places and routes an 1800-cell design: about 80 s
/// from a release build and 450 s from a debug one. Run it with
/// `cargo test --release --all-features --test cli -- --include-ignored
/// ssd1306_console_is_clocked`.
#[test]
#[ignore = "places and routes an 1800-cell design; run with --release --include-ignored"]
fn ssd1306_console_is_clocked_entirely_on_the_global_network() {
    let dir = scratch("ssd1306_console_clocks");
    let bit = dir.join("ssd1306_console.bit");
    let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "examples/basys3/ssd1306_console_pad.v",
            "examples/basys3/ssd1306_console.v",
            "ip/bus/spi_display_rx/rtl/spi_display_rx.v",
            "ip/util/cdc_sync/rtl/cdc_sync.v",
            "ip/video/ssd1306_slave/rtl/ssd1306_slave.v",
            "ip/bus/uart/rtl/uart.v",
            "ip/bus/uart/rtl/uart_baud_div.v",
            "ip/bus/uart/rtl/uart_rx.v",
            "ip/bus/uart/rtl/uart_tx.v",
            "--top",
            "ssd1306_console_pad",
            "--device",
            "xc7a35t-cpg236",
            "--constraints",
            "examples/basys3/ssd1306_console.rcf",
            "--place-effort",
            "3",
            "--output-dir",
            dir.to_str().unwrap(),
            "--bitstream",
            bit.to_str().unwrap(),
        ])
        .env("RETICLE_OFFLINE", "1")
        .output()
        .expect("failed to run the reticle binary");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if stderr.contains("needs a chip database") {
        eprintln!("skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db`");
        return;
    }
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let line = stderr
        .lines()
        .find(|line| line.contains("clock pin(s) a global buffer drives arrived"))
        .unwrap_or_else(|| panic!("no clock-network line in:\n{stderr}"));
    let numbers: Vec<usize> = line
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect();
    assert!(
        numbers.len() >= 2 && numbers[0] == numbers[1] && numbers[0] > 0,
        "not every clock pin was on the network: {line}"
    );
}

/// Runs one of `examples/basys3/`'s testbenches through `reticle sim` and
/// requires its PASS line.
#[cfg(feature = "sim")]
fn basys3_testbench(bench: &str, sources: &[&str]) {
    let mut args = vec!["sim", "--quiet", bench];
    args.extend_from_slice(sources);
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.lines().any(|line| line.starts_with("PASS:")),
        "`{bench}` did not pass:\n{out}\n{err}"
    );
}

#[cfg(feature = "sim")]
const UART: [&str; 4] = [
    "ip/bus/uart/rtl/uart.v",
    "ip/bus/uart/rtl/uart_baud_div.v",
    "ip/bus/uart/rtl/uart_rx.v",
    "ip/bus/uart/rtl/uart_tx.v",
];

/// The display console answers every request a host sends without waiting,
/// among the rest of what its testbench checks. Its burst of `?`, `?`, `g`,
/// `?` got 3 lines of 5 before the request queue, because the status flag
/// was cleared on every cycle the port was sending; that was found on a
/// Basys 3 at 2 Mbaud. About 6 s from a debug build.
///
/// It would not catch a queue that is too shallow for a longer burst — the
/// burst is four — nor anything about the part: the board found this, and
/// the board is where the sixteen-deep limit was measured.
#[cfg(feature = "sim")]
#[test]
fn the_ssd1306_console_testbench_passes() {
    let mut sources = vec![
        "examples/basys3/ssd1306_console.v",
        "ip/bus/spi_display_rx/rtl/spi_display_rx.v",
        "ip/util/cdc_sync/rtl/cdc_sync.v",
        "ip/video/ssd1306_slave/rtl/ssd1306_slave.v",
    ];
    sources.extend(UART);
    basys3_testbench("examples/basys3/ssd1306_console_tb.v", &sources);
}

/// The card terminal answers `k`, `s`, `k` sent back to back, in order,
/// and `W` sends a whole SESSION_START and a ticker after the card's status.
/// Before the request queue only the first came back, because both request
/// flags were cleared at the end of either one's line. About 30 s from a
/// debug build. Same limits as the display console's test.
#[cfg(feature = "sim")]
#[test]
fn the_iso7816_terminal_testbench_passes() {
    let mut sources = vec![
        "examples/basys3/iso7816_terminal.v",
        "ip/bus/iso7816_uart/rtl/iso7816_uart.v",
        "ip/bus/seph_mcu/rtl/seph_mcu.v",
        "ip/util/cdc_sync/rtl/cdc_sync.v",
    ];
    sources.extend(UART);
    basys3_testbench("examples/basys3/iso7816_terminal_tb.v", &sources);
}

/// Both halves on one port, as the board runs them: among the rest, a burst
/// of `k s ? k s ? g` sent without waiting must come back as eight whole
/// lines. It got four before the request queue. About 60 s from a debug
/// build.
#[cfg(feature = "sim")]
#[test]
fn the_iso_display_testbench_passes() {
    let mut sources = vec![
        "examples/basys3/iso_display.v",
        "examples/basys3/iso7816_terminal.v",
        "examples/basys3/ssd1306_console.v",
        "ip/bus/iso7816_uart/rtl/iso7816_uart.v",
        "ip/bus/seph_mcu/rtl/seph_mcu.v",
        "ip/bus/spi_display_rx/rtl/spi_display_rx.v",
        "ip/video/ssd1306_slave/rtl/ssd1306_slave.v",
        "ip/util/cdc_sync/rtl/cdc_sync.v",
    ];
    sources.extend(UART);
    basys3_testbench("examples/basys3/iso_display_tb.v", &sources);
}
