//! Golden tests for the ASIC flow: Liberty to gate library, the whole
//! mapping pipeline, the constraints and the OpenROAD hand-off.
//!
//! Everything runs against `testdata/asic/reticle_sc.lib` and
//! `reticle_sc.lef`, a **synthetic** standard-cell library whose numbers
//! are invented; its own header says so at length. It is shaped like an
//! open PDK (drive variants, 2-D delay tables, flip-flops with `ff`
//! groups, cells the mapper must refuse), so pointing the same code at
//! SKY130 or IHP SG13G2 changes nothing but the file names.
//!
//! Each case under `testdata/asic/` is a design in the IR text format
//! (`<name>.rtl`) taken through `asic::synthesize_asic` and
//! `asic::export_openroad`:
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<name>.diag` | everything the flow reported |
//! | `<name>.map` | the area and timing report |
//! | `<name>.mapped.rtl` | the mapped netlist, in the IR text format |
//! | `<name>.v` | the same netlist as the structural Verilog OpenROAD reads |
//! | `<name>.sdc` | the constraints handed to the back end |
//! | `<name>.tcl` | the generated OpenROAD script |
//! | `reticle_sc.cells` | the mapper's view of the library |
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.
//!
//! Two tests are about behaviour rather than bytes:
//!
//! - `mapping_preserves_behaviour` proves each mapped netlist equivalent
//!   to the design before mapping, through `asic::logic_model` and
//!   `formal::check_equivalent`. A mapper that changes behaviour is the
//!   failure that matters, so every design carries its expected verdict
//!   the way `tests/synth_verify.rs` does.
//! - `exports_are_placeable` runs `asic::check_physical` over each
//!   export: only library cells, all of them in both the Liberty and the
//!   LEF, wired to pins those macros have.
//!
//! One test runs the real `openroad` over the export when one is
//! installed; it says so and returns when none is, the way the other
//! environment-dependent tests in this repository do.

#![cfg(feature = "asic")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::asic::flow::{AsicOptions, AsicReport, synthesize_asic};
use reticle::asic::lef::{self, Lef};
use reticle::asic::liberty::Library;
use reticle::asic::library::{LibraryOptions, StdCells};
use reticle::asic::openroad::{
    Floorplan, OpenRoadInputs, OpenRoadOptions, check_physical, export_openroad,
};
use reticle::asic::sdc::{AsicConstraints, Clock, PortDelay};
use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;

/// The designs, and whether they have a clock.
const CASES: [(&str, bool); 4] = [
    ("alu", false),
    ("toggle", true),
    ("counter", true),
    ("shifter", true),
];

/// The Liberty file every case is mapped against.
const LIBERTY: &str = "reticle_sc.lib";
/// The LEF that describes the same cells physically.
const TECH_LEF: &str = "reticle_sc.lef";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/asic")
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

/// The synthetic library, parsed.
fn library() -> Library {
    let mut sources = SourceMap::new();
    let text = read(LIBERTY);
    let file = sources.add(LIBERTY, text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let library = Library::parse(&text, file, &mut diags)
        .unwrap_or_else(|| panic!("{LIBERTY} has no library group"));
    assert!(
        !diags.has_errors(),
        "{LIBERTY} does not parse cleanly:\n{}",
        diags.render(&sources)
    );
    library
}

/// The synthetic technology, parsed.
fn technology() -> Lef {
    let mut sources = SourceMap::new();
    let text = read(TECH_LEF);
    let file = sources.add(TECH_LEF, text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let lef = lef::parse_lef(&text, file, &mut diags);
    assert!(
        !diags.has_errors(),
        "{TECH_LEF} does not parse cleanly:\n{}",
        diags.render(&sources)
    );
    lef
}

/// The constraints a case is synthesised and exported with.
fn constraints(name: &str, clocked: bool) -> AsicConstraints {
    let mut c = AsicConstraints::new();
    if clocked {
        c = c
            .with_clock(Clock::new("sys", "clk", 4.0))
            .with_input_delay(PortDelay::new("rst", "sys", 0.5))
            .with_false_path(Some("rst"), None);
        if name != "toggle" {
            c = c
                .with_input_delay(PortDelay::new("en", "sys", 0.5))
                .with_output_delay(PortDelay::new("q", "sys", 0.8))
                .with_load("q", 0.01)
                .with_driving_cell("en", "INV_X1", "Y")
                .with_multicycle_path(Some("u_reg"), Some("carry"), 2);
        }
    } else {
        // A combinational block is still constrained: the analysis needs
        // a clock to hang the input and output delays off.
        c = c
            .with_clock(Clock::new("virt", "a", 4.0))
            .with_input_delay(PortDelay::new("a", "virt", 0.4))
            .with_output_delay(PortDelay::new("y", "virt", 0.4))
            .with_load("y", 0.01);
    }
    c.uncertainty = Some(0.05);
    c
}

/// What one case produced.
///
/// `before` and `cells` are the equivalence check's, and that test needs
/// the `formal` feature; without it they are built and not read.
#[cfg_attr(not(feature = "formal"), allow(dead_code))]
struct Run {
    /// The design before mapping, synthesised only.
    before: Design,
    /// The design after the whole flow.
    design: Design,
    /// The mapper's view of the library.
    cells: StdCells,
    /// Everything the flow reported.
    diagnostics: String,
    /// The area and timing report.
    report: AsicReport,
    /// The OpenROAD hand-off.
    inputs: OpenRoadInputs,
}

fn run_case(name: &str, clocked: bool) -> Run {
    let library = library();
    let lef = technology();
    let constraints = constraints(name, clocked);

    let mut sources = SourceMap::new();
    let rtl = read(&format!("{name}.rtl"));
    let file = sources.add(format!("{name}.rtl"), rtl.clone()).unwrap();
    let mut design = Design::parse_text(&rtl, file)
        .unwrap_or_else(|diags| panic!("{name}: parse failed\n{}", diags.render(&sources)));
    let problems = validate(&design);
    assert!(
        !problems.has_errors(),
        "{name}: the input is invalid\n{}",
        problems.render(&sources)
    );
    let top = design
        .top
        .unwrap_or_else(|| panic!("{name}: no top module"));

    // The reference for the equivalence check: the same design with
    // generic synthesis run over it and nothing else.
    let mut before = design.clone();
    let mut synth_diags = Diagnostics::new();
    reticle::synth::run(
        &mut before,
        &reticle::synth::SynthOptions::default(),
        &mut synth_diags,
    );
    assert!(
        !synth_diags.has_errors(),
        "{name}: synthesis of the reference failed\n{}",
        synth_diags.render(&sources)
    );

    let options = AsicOptions {
        constraints: constraints.clone(),
        ..AsicOptions::new()
    };
    let mut diags = Diagnostics::new();
    let report = synthesize_asic(&mut design, top, &library, &options, &mut diags)
        .unwrap_or_else(|e| panic!("{name}: the flow failed: {e}\n{}", diags.render(&sources)));
    let problems = validate(&design);
    assert!(
        !problems.has_errors(),
        "{name}: the mapped design is invalid\n{}",
        problems.render(&sources)
    );
    diags.sort();

    let openroad = OpenRoadOptions {
        liberty_files: vec![LIBERTY.to_owned()],
        lef_files: vec![TECH_LEF.to_owned()],
        floorplan: Some(Floorplan::square(30.0, 2.76)),
        clock_buffers: vec!["BUF_X2".to_owned(), "BUF_X4".to_owned()],
        fillers: vec!["FILL_X1".to_owned()],
        ..OpenRoadOptions::new()
    };
    let inputs = export_openroad(&design, top, &library, &lef, &constraints, &openroad)
        .unwrap_or_else(|e| panic!("{name}: the OpenROAD export failed: {e}"));

    Run {
        before,
        design,
        cells: StdCells::from_library(&library, &LibraryOptions::default()),
        diagnostics: diags.render(&sources),
        report,
        inputs,
    }
}

#[test]
fn golden_asic_flow() {
    let mut failures = Vec::new();
    expect(
        "reticle_sc.cells",
        &StdCells::from_library(&library(), &LibraryOptions::default()).to_text(),
        &mut failures,
    );
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        expect(&format!("{name}.diag"), &run.diagnostics, &mut failures);
        expect(&format!("{name}.map"), &run.report.to_text(), &mut failures);

        let mapped = run.design.to_text();
        expect(&format!("{name}.mapped.rtl"), &mapped, &mut failures);
        // The mapped design must survive the text format unchanged, like
        // any other design in the IR.
        let mut sources = SourceMap::new();
        let file = sources
            .add(format!("{name}.mapped.rtl"), mapped.clone())
            .unwrap();
        match Design::parse_text(&mapped, file) {
            Ok(again) => {
                if again.to_text() != mapped {
                    failures.push(format!("{name}: the mapped design does not round-trip"));
                }
            }
            Err(diags) => failures.push(format!(
                "{name}: the mapped design does not parse\n{}",
                diags.render(&sources)
            )),
        }

        expect(&format!("{name}.v"), &run.inputs.verilog, &mut failures);
        expect(&format!("{name}.sdc"), &run.inputs.sdc, &mut failures);
        expect(&format!("{name}.tcl"), &run.inputs.tcl, &mut failures);
        assert!(
            run.inputs.verilog.contains("module "),
            "{name}: the netlist is empty"
        );
        assert!(run.inputs.def.is_some(), "{name}: no DEF was written");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Every design maps completely: no generic cell survives, and the area
/// and the cell counts agree with each other.
#[test]
fn every_design_maps_completely() {
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        assert!(
            run.report.is_fully_mapped(),
            "{name}: the netlist is not fully mapped: {:?}",
            run.report.cells
        );
        assert!(
            run.report.unmapped.is_empty(),
            "{name}: {:?}",
            run.report.unmapped
        );
        assert!(run.report.area > 0.0, "{name}: no area");
        assert_eq!(
            run.report.total_cells(),
            run.report.cells.iter().map(|(_, n)| n).sum::<usize>(),
            "{name}: the cell counts disagree"
        );
        if clocked {
            assert!(run.report.flop_cells > 0, "{name}: no flip-flops");
        }
        let timing = run
            .report
            .timing
            .as_ref()
            .filter(|_| cfg!(feature = "timing"));
        if let Some(timing) = timing {
            assert!(
                timing.worst_setup.is_some(),
                "{name}: nothing was checked for setup"
            );
        }
    }
}

/// The inverter insertion the FPGA side does not do: `toggle` has an
/// active-high asynchronous reset and the library has only an
/// active-low one, so an inverter must appear and the flip-flop must
/// still map.
#[test]
fn an_active_high_reset_gets_an_inverter() {
    let run = run_case("toggle", true);
    assert_eq!(run.report.inverters_inserted, 1, "{}", run.report.to_text());
    assert_eq!(run.report.count("DFFR_X1"), 1, "{}", run.report.to_text());
    assert!(run.report.is_fully_mapped());
}

/// A synchronous reset has no pin on any flip-flop of the library (no
/// open PDK ships one), so it becomes a multiplexer on `d`. The clock
/// enable does have a cell, so it stays a pin.
#[test]
fn a_sync_reset_becomes_logic_and_an_enable_stays_a_pin() {
    let run = run_case("counter", true);
    assert_eq!(run.report.lowered_resets, 1, "{}", run.report.to_text());
    assert_eq!(run.report.lowered_enables, 0, "{}", run.report.to_text());
    assert_eq!(run.report.count("DFFE_X1"), 4, "{}", run.report.to_text());
}

/// No cell has both an enable and a reset, so the enable is the one that
/// moves into the data path and the reset keeps its pin.
#[test]
fn an_enable_becomes_logic_when_the_reset_needs_the_cell() {
    let run = run_case("shifter", true);
    assert_eq!(run.report.lowered_enables, 1, "{}", run.report.to_text());
    assert_eq!(run.report.lowered_resets, 0, "{}", run.report.to_text());
    assert_eq!(run.report.count("DFFR_X1"), 4, "{}", run.report.to_text());
    // The reset is active low, as the library's pin is, so no inverter.
    assert_eq!(run.report.inverters_inserted, 0, "{}", run.report.to_text());
}

/// Every export is a netlist OpenROAD can place: library cells only,
/// each in the Liberty and the LEF, wired to pins they have.
#[test]
fn exports_are_placeable() {
    let library = library();
    let lef = technology();
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        let top = run.design.top.unwrap();
        let problems = check_physical(&run.design, top, &library, &lef);
        assert!(
            problems.is_empty(),
            "{name}: the export is not placeable:\n  {}",
            problems.join("\n  ")
        );
        assert!(
            run.inputs.problems.is_empty(),
            "{name}: {:?}",
            run.inputs.problems
        );
    }
}

/// Mapping must not change what the design does.
///
/// The mapped netlist is a pile of black boxes, so it is first turned
/// back into logic with `asic::logic_model` — every standard cell
/// becomes the generic IR cell computing the same function — and then
/// compared with the design as generic synthesis left it.
///
/// The expected verdict is written down per design, as
/// `tests/synth_verify.rs` does, so both the proofs and the limits stay
/// visible. Three of the four are proved, `alu` combinationally and
/// `toggle` and `shifter` by induction from reset even though their
/// registers were split into one flip-flop per bit.
///
/// `counter` is inconclusive, and for a reason worth knowing: its reset
/// is synchronous, so the flow moved it into a multiplexer on `d`, and
/// the flip-flops that are left have no reset pin and therefore no reset
/// *value*. Sequential equivalence from reset (`InitMode::Reset`) needs
/// one for every state element (`F0018`), so the check cannot start.
/// The netlist is not wrong — the reset logic is in the data path where
/// the library put it — but proving it needs an initial state the
/// mapped netlist no longer declares.
#[cfg(feature = "formal")]
#[test]
fn mapping_preserves_behaviour() {
    use reticle::formal::{EquivOptions, EquivOutcome, InitMode, check_equivalent};

    /// What the check is expected to say, per design.
    const VERDICTS: [(&str, bool); 4] = [
        ("alu", true),
        ("toggle", true),
        ("counter", false),
        ("shifter", true),
    ];

    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        let top = run.design.top.unwrap();
        let proved = VERDICTS
            .iter()
            .find(|(case, _)| *case == name)
            .map(|(_, proved)| *proved)
            .unwrap_or_else(|| panic!("{name} has no expected verdict"));

        // Both designs have to live in one arena for the miter.
        let mut combined = run.before.clone();
        let reference = combined.top.expect("the reference has a top");
        let mut diags = Diagnostics::new();
        let model = reticle::asic::flow::logic_model(
            run.design.module(top),
            &run.cells,
            &format!("{name}$mapped"),
            &mut diags,
        );
        assert!(
            !diags.has_errors(),
            "{name}: the model of the mapped netlist is incomplete"
        );
        let mapped = combined.add_module(model);
        let problems = validate(&combined);
        assert!(
            !problems.has_errors(),
            "{name}: the model is not a valid design\n{}",
            problems
                .iter()
                .map(|d| format!("  {}", d.message))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let options = EquivOptions {
            depth: 8,
            max_k: 4,
            init: InitMode::Reset,
            ..EquivOptions::default()
        };
        let report = check_equivalent(&combined, reference, mapped, &options);
        match (&report.outcome, proved) {
            (EquivOutcome::Equivalent(_), true) => {}
            (EquivOutcome::Unknown { .. }, false) => {}
            (outcome, _) => panic!(
                "{name}: unexpected verdict {outcome:?}\n{}\n{}",
                report.render(name, "mapped"),
                report
                    .diags
                    .iter()
                    .map(|d| format!("  {:?} {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        }
    }
}

/// Runs the real OpenROAD over the export when one is installed, and
/// says what it said. When none is installed the test prints why and
/// returns, since nothing in this crate may depend on an external tool
/// being present.
#[test]
fn openroad_reads_the_export() {
    use std::process::Command;

    let mut ran = 0;
    let mut failed = Vec::new();
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        let tool = &run.inputs.args[0];
        let found = Command::new("which")
            .arg(tool)
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !found {
            println!("{name}: {tool} is not installed, skipping");
            continue;
        }
        let work = std::env::temp_dir().join(format!("reticle-asic-{name}"));
        fs::create_dir_all(&work).unwrap();
        for (file, contents) in &run.inputs.files {
            fs::write(work.join(file), contents).unwrap();
        }
        // The script reads the PDK files by name from the same directory.
        for file in [LIBERTY, TECH_LEF] {
            fs::write(work.join(file), read(file)).unwrap();
        }
        let output = Command::new(tool)
            .args(&run.inputs.args[1..])
            .current_dir(&work)
            .output()
            .unwrap_or_else(|e| panic!("{name}: cannot run {tool}: {e}"));
        ran += 1;
        println!(
            "{name}: {tool} {} -> {}\n{}\n{}",
            run.inputs.args[1..].join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            failed.push(format!("{name}: {tool} exited with {}", output.status));
        }
    }
    if ran == 0 {
        println!(
            "no openroad is installed, so the export was checked against the Liberty \
             and the LEF only (see `exports_are_placeable`)"
        );
        return;
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// Reads the exported netlist back with `yosys` when one is installed,
/// which is a cheap second opinion on the structural Verilog.
#[test]
fn yosys_reads_the_netlist() {
    use std::process::Command;

    let found = Command::new("which")
        .arg("yosys")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !found {
        println!("yosys is not installed, skipping");
        return;
    }
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        let work = std::env::temp_dir().join(format!("reticle-asic-yosys-{name}"));
        fs::create_dir_all(&work).unwrap();
        let netlist = work.join(format!("{name}.v"));
        fs::write(&netlist, &run.inputs.verilog).unwrap();
        let output = Command::new("yosys")
            .arg("-p")
            .arg(format!(
                "read_verilog {}; hierarchy; check",
                netlist.display()
            ))
            .output()
            .expect("yosys runs");
        assert!(
            output.status.success(),
            "{name}: yosys rejected the netlist:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Reads the netlist, the Liberty and the SDC with OpenSTA when one is
/// installed, which checks the constraints against a second timer.
#[test]
fn sta_reads_the_constraints() {
    use std::process::Command;

    let found = Command::new("which")
        .arg("sta")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !found {
        println!("sta (OpenSTA) is not installed, skipping");
        return;
    }
    for (name, clocked) in CASES {
        let run = run_case(name, clocked);
        let work = std::env::temp_dir().join(format!("reticle-asic-sta-{name}"));
        fs::create_dir_all(&work).unwrap();
        for (file, contents) in &run.inputs.files {
            fs::write(work.join(file), contents).unwrap();
        }
        fs::write(work.join(LIBERTY), read(LIBERTY)).unwrap();
        let script = format!(
            "read_liberty {LIBERTY}\nread_verilog {name}.v\nlink_design {name}\n\
             read_sdc {name}.sdc\nreport_checks\nexit\n"
        );
        fs::write(work.join("sta.tcl"), script).unwrap();
        let output = Command::new("sta")
            .arg("-exit")
            .arg("sta.tcl")
            .current_dir(&work)
            .output()
            .expect("sta runs");
        assert!(
            output.status.success(),
            "{name}: sta rejected the design:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The drive-strength pass, forced to do something: with a wire load
/// heavier than an `X1` cell's `max_capacitance`, every driver is over
/// its rating, so the ones with a stronger variant are swapped for it
/// and the ones without are reported as overloaded rather than quietly
/// left.
#[test]
fn a_heavy_wire_load_upsizes_what_it_can() {
    let library = library();
    let mut sources = SourceMap::new();
    let rtl = read("alu.rtl");
    let file = sources.add("alu.rtl", rtl.clone()).unwrap();
    let mut design = Design::parse_text(&rtl, file).expect("alu.rtl parses");
    let top = design.top.unwrap();
    let mut diags = Diagnostics::new();
    let report = synthesize_asic(
        &mut design,
        top,
        &library,
        &AsicOptions {
            // An X1 cell is rated for 0.06, an X2 for 0.12 and an X4 for
            // 0.24, so this is beyond X1 and within X2.
            wire_load: 0.1,
            constraints: constraints("alu", false),
            ..AsicOptions::new()
        },
        &mut diags,
    )
    .expect("the flow runs");
    assert!(
        report.resized > 0,
        "nothing was upsized\n{}",
        report.to_text()
    );
    assert!(
        !report.overloaded.is_empty(),
        "no net was reported as overloaded\n{}",
        report.to_text()
    );
    // Every cell that has a stronger variant took it.
    for (name, _) in &report.cells {
        assert!(
            !name.ends_with("_X1") || !matches!(&name[..], "INV_X1" | "NAND2_X1" | "NOR2_X1"),
            "`{name}` has a stronger variant and kept its rating\n{}",
            report.to_text()
        );
    }
    assert!(report.to_text().contains("upsized"));
    // The netlist is still valid and still fully mapped.
    assert!(!validate(&design).has_errors());
    assert!(report.is_fully_mapped());
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(reticle::asic::flow::OVERLOADED_NET)),
        "the overloaded nets were not reported"
    );
}
