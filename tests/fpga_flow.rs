//! Golden tests for the FPGA flow: constraints, mapping and hand-off.
//!
//! Each case under `testdata/fpga/` is a design in the IR text format
//! (`<name>.rtl`) plus its constraints (`<name>.rcf`), and is taken all
//! the way to the files a place-and-route tool would be handed:
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<name>.diag` | everything the constraint check and the mapper reported |
//! | `<name>.map` | the mapping report |
//! | `<name>.<device>.rtl` | the mapped design |
//! | `<name>.pcf` / `<name>.lpf` | the nextpnr constraints for the family |
//! | `<name>.xdc` | the Vivado constraints |
//! | `<name>.json` | the netlist handed to nextpnr |
//!
//! Constraints are checked against the *input* design, before mapping,
//! because that is the design the user wrote the names of.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.
//!
//! One test is `#[ignore]`d: it runs `nextpnr-<family>` over the exported
//! JSON when one is installed, and reports what it said. It is not part
//! of the normal run because the export is not complete until technology
//! mapping lands (see `fpga::flow`), and because nothing in this crate
//! may depend on an external tool.

#![cfg(feature = "fpga")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::fpga::{self, Constraints, Device, MapOptions};
use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;

/// The cases, each with the built-in device it targets.
const CASES: [(&str, &str); 4] = [
    ("blinky_ice40", "ice40-hx1k-tq144"),
    ("ram_ice40", "ice40-hx1k-tq144"),
    ("blinky_ecp5", "ecp5-45f-CABGA381"),
    ("ram_ecp5", "ecp5-45f-CABGA381"),
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga")
}

fn read(name: &str) -> String {
    let path = dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Compares `actual` with the file `name`, rewriting it under
/// `UPDATE_EXPECT`. An absent file counts as empty.
fn expect(name: &str, actual: &str, failures: &mut Vec<String>) {
    let path = dir().join(name);
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        if actual.is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        }
        return;
    }
    let diff = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (e, a))| e != a)
        .map_or_else(
            || {
                format!(
                    "expected {} lines, got {}",
                    expected.lines().count(),
                    actual.lines().count()
                )
            },
            |(i, (e, a))| format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1),
        );
    failures.push(format!(
        "{name}: mismatch (set UPDATE_EXPECT=1 to rewrite)\n{diff}"
    ));
}

/// Runs one case and returns what it produced.
struct Run {
    design: Design,
    constraints: Constraints,
    device: &'static Device,
    diagnostics: String,
    report: String,
}

fn run_case(name: &str, device_name: &str) -> Run {
    let device = fpga::target(device_name)
        .unwrap_or_else(|| panic!("{name}: no built-in device `{device_name}`"));

    let mut sources = SourceMap::new();
    let rtl = read(&format!("{name}.rtl"));
    let rtl_file = sources.add(format!("{name}.rtl"), rtl.clone()).unwrap();
    let mut design = Design::parse_text(&rtl, rtl_file)
        .unwrap_or_else(|diags| panic!("{name}: parse failed\n{}", diags.render(&sources)));
    let problems = validate(&design);
    assert!(
        !problems.has_errors(),
        "{name}: input invalid\n{}",
        problems.render(&sources)
    );
    let top = design
        .top
        .unwrap_or_else(|| panic!("{name}: no top module"));

    let rcf = read(&format!("{name}.rcf"));
    let rcf_file = sources.add(format!("{name}.rcf"), rcf.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    constraints.check(&design, device, &mut diags);

    let report = fpga::map(
        &mut design,
        top,
        device,
        &constraints,
        &MapOptions::default(),
        &mut diags,
    );
    let problems = validate(&design);
    assert!(
        !problems.has_errors(),
        "{name}: mapped design invalid\n{}",
        problems.render(&sources)
    );
    diags.sort();

    Run {
        design,
        constraints,
        device,
        diagnostics: diags.render(&sources),
        report: report.to_text(),
    }
}

#[test]
fn golden_fpga_flow() {
    let mut failures = Vec::new();
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        let top = run.design.top.unwrap();

        expect(&format!("{name}.diag"), &run.diagnostics, &mut failures);
        expect(&format!("{name}.map"), &run.report, &mut failures);

        let mapped = run.design.to_text();
        expect(
            &format!("{name}.{}.rtl", run.device.name),
            &mapped,
            &mut failures,
        );

        // The mapped design must survive the text format unchanged, like
        // any other design in the IR.
        let mut sources = SourceMap::new();
        let file = sources
            .add(format!("{name}.mapped.rtl"), mapped.clone())
            .unwrap();
        match Design::parse_text(&mapped, file) {
            Ok(again) => {
                if again.to_text() != mapped {
                    failures.push(format!("{name}: mapped design does not round-trip"));
                }
            }
            Err(diags) => failures.push(format!(
                "{name}: mapped design does not parse\n{}",
                diags.render(&sources)
            )),
        }

        let inputs = fpga::export_nextpnr(&run.design, top, run.device, &run.constraints)
            .unwrap_or_else(|e| panic!("{name}: nextpnr export failed: {e}"));
        // The golden file is named after the case, not after the module,
        // so two cases of one family cannot collide; the extension is the
        // one the export asks for.
        let extension = Path::new(&inputs.constraints_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("pcf");
        expect(
            &format!("{name}.{extension}"),
            &inputs.pcf_or_lpf,
            &mut failures,
        );
        expect(&format!("{name}.json"), &inputs.json, &mut failures);
        assert_eq!(inputs.args[0], format!("nextpnr-{}", run.device.family));

        let vendor = fpga::export_vendor(&run.design, top, run.device, &run.constraints)
            .unwrap_or_else(|e| panic!("{name}: vendor export failed: {e}"));
        expect(&format!("{name}.xdc"), &vendor.xdc, &mut failures);
        assert!(vendor.verilog.contains("module "), "{name}: empty Verilog");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The constraints file names the pins of the device it targets, and the
/// device database knows them: a clean case produces no diagnostics at
/// all.
#[test]
fn constraints_check_cleanly() {
    for (name, device_name) in CASES {
        if !name.starts_with("blinky") {
            continue;
        }
        let run = run_case(name, device_name);
        assert!(
            !run.diagnostics.contains("error["),
            "{name}: {}",
            run.diagnostics
        );
    }
}

/// Runs the real place-and-route tool over the export, when one is
/// installed. Ignored by default; run with
/// `cargo test --all-features --test fpga_flow -- --ignored --nocapture`.
#[test]
#[ignore = "needs nextpnr installed; the export is not complete before technology mapping"]
fn nextpnr_reads_the_export() {
    use std::process::Command;

    let mut ran = 0;
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        let top = run.design.top.unwrap();
        let inputs = fpga::export_nextpnr(&run.design, top, run.device, &run.constraints).unwrap();
        let tool = &inputs.args[0];
        let found = Command::new("which")
            .arg(tool)
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !found {
            println!("{name}: {tool} is not installed, skipping");
            continue;
        }
        let dir = std::env::temp_dir().join(format!("reticle-fpga-{name}"));
        fs::create_dir_all(&dir).unwrap();
        let module = run.design.module(top).name.as_str().to_owned();
        fs::write(dir.join(format!("{module}.json")), &inputs.json).unwrap();
        fs::write(dir.join(&inputs.constraints_name), &inputs.pcf_or_lpf).unwrap();
        let output = Command::new(tool)
            .args(&inputs.args[1..])
            .current_dir(&dir)
            .output()
            .unwrap_or_else(|e| panic!("{name}: cannot run {tool}: {e}"));
        ran += 1;
        println!(
            "{name}: {} {} -> {}\n{}",
            tool,
            inputs.args[1..].join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    println!("{ran} of {} cases were run", CASES.len());
}
