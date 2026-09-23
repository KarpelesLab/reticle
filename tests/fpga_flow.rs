//! Golden tests for the FPGA flow: constraints, the whole mapping
//! pipeline, the netlist check and the hand-off.
//!
//! Each case under `testdata/fpga/` is a design in the IR text format
//! (`<name>.rtl`) plus its constraints (`<name>.rcf`), and is taken all
//! the way to the files a place-and-route tool would be handed by
//! `fpga::synthesize_for`, which is synthesis, primitive mapping, LUT
//! mapping, clean-up and the rewrite to the device's own primitives:
//!
//! | File | What it holds |
//! |------|----------------|
//! | `<name>.diag` | everything the constraint check and the flow reported |
//! | `<name>.map` | the flow report |
//! | `<name>.<device>.rtl` | the mapped design |
//! | `<name>.pcf` / `<name>.lpf` | the nextpnr constraints for the family |
//! | `<name>.xdc` | the Vivado constraints |
//! | `<name>.json` | the netlist handed to nextpnr |
//!
//! Constraints are checked against the *input* design, before mapping,
//! because that is the design the user wrote the names of.
//!
//! Every exported netlist is also run through `fpga::check_nextpnr_json`,
//! which validates it against the device database: only primitives the
//! device declares, only ports they have, one driver per net, constants
//! the family's packer can tie, and pin constraints the package and the
//! design both know. A golden file that no longer passes that check is a
//! regression even when it is otherwise unchanged.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.
//!
//! One test runs the real `nextpnr-<family>` over the export. It is not
//! ignored: when no such tool is installed it says so and returns, the
//! way the other environment-dependent tests in this repository do.

#![cfg(feature = "fpga")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::fpga::{self, Constraints, Device, FlowReport, FpgaOptions};
use reticle::ir::Design;
use reticle::ir::validate::validate;
use reticle::source::SourceMap;

/// The cases, each with the built-in device it targets.
const CASES: [(&str, &str); 17] = [
    ("blinky_ice40", "ice40-hx1k-tq144"),
    ("ram_ice40", "ice40-hx1k-tq144"),
    ("logicram_ice40", "ice40-hx1k-tq144"),
    ("carry_ice40", "ice40-hx1k-tq144"),
    ("pll_ice40", "ice40-hx1k-tq144"),
    ("ddr_ice40", "ice40-hx1k-tq144"),
    ("blinky_ecp5", "ecp5-45f-CABGA381"),
    ("ram_ecp5", "ecp5-45f-CABGA381"),
    ("logicram_ecp5", "ecp5-45f-CABGA381"),
    ("regfile_ecp5", "ecp5-45f-CABGA381"),
    ("clkbuf_ecp5", "ecp5-45f-CABGA381"),
    ("pll_ecp5", "ecp5-45f-CABGA381"),
    ("ddr_ecp5", "ecp5-45f-CABGA381"),
    ("blinky_xc7", "xc7a35t-cpg236"),
    ("ram_xc7", "xc7a35t-cpg236"),
    ("logicram_xc7", "xc7a35t-cpg236"),
    ("pll_xc7", "xc7a35t-cpg236"),
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
    report: FlowReport,
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

    let report = fpga::synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("{name}: the flow failed: {e}\n{}", diags.render(&sources)));
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
        report,
    }
}

#[test]
fn golden_fpga_flow() {
    let mut failures = Vec::new();
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        let top = run.design.top.unwrap();

        expect(&format!("{name}.diag"), &run.diagnostics, &mut failures);
        expect(&format!("{name}.map"), &run.report.to_text(), &mut failures);

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

        // Only the families with a nextpnr back end have a JSON export;
        // a family whose route is a vendor tool answers with the error
        // that says so, which is checked in `a_vendor_family_says_so`.
        if fpga::pnr_route(&run.device.family) == Some(fpga::PnrRoute::Nextpnr) {
            let inputs = fpga::export_nextpnr(&run.design, top, run.device, &run.constraints)
                .unwrap_or_else(|e| panic!("{name}: nextpnr export failed: {e}"));
            // The golden file is named after the case, not after the
            // module, so two cases of one family cannot collide; the
            // extension is the one the export asks for.
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
        }

        let vendor = fpga::export_vendor(&run.design, top, run.device, &run.constraints)
            .unwrap_or_else(|e| panic!("{name}: vendor export failed: {e}"));
        expect(&format!("{name}.xdc"), &vendor.xdc, &mut failures);
        assert!(vendor.verilog.contains("module "), "{name}: empty Verilog");
        // The Verilog and the script are the files a vendor flow reads,
        // so they are goldens too on the family whose route that is.
        if fpga::pnr_route(&run.device.family) == Some(fpga::PnrRoute::Vendor) {
            expect(&format!("{name}.v"), &vendor.verilog, &mut failures);
            expect(&format!("{name}.tcl"), &vendor.script, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Every exported netlist is one the device can actually hold: only
/// primitives it declares, wired to ports they have, one driver per net.
#[test]
fn exports_are_acceptable_netlists() {
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        let top = run.design.top.unwrap();
        let problems = fpga::check_nextpnr_json(&run.design, top, run.device, &run.constraints);
        assert!(
            problems.is_empty(),
            "{name}: the exported netlist is not acceptable:\n{}",
            problems
                .iter()
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        // Nothing generic survives: every cell type is a device primitive.
        for (cell, count) in &run.report.netlist {
            assert!(
                !cell.starts_with('$'),
                "{name}: {count} generic `{cell}` cell(s) are left"
            );
            assert!(
                run.device.primitive_ports(cell).is_some(),
                "{name}: `{cell}` is not a primitive of `{}`",
                run.device.name
            );
        }
    }
}

/// The flow reaches the primitives each case is about: the carry chain,
/// the block RAM, the clock buffer, the LUTs and the flip-flops.
#[test]
fn every_layer_of_the_flow_is_exercised() {
    let expected: [(&str, &[&str]); 17] = [
        (
            "blinky_ice40",
            &["SB_LUT4", "SB_CARRY", "SB_IO", "SB_GB", "SB_DFFSR"],
        ),
        ("ram_ice40", &["SB_RAM40_4K", "SB_IO"]),
        // The fallback: a memory under the threshold on a family with
        // no distributed RAM is flip-flops and a read multiplexer.
        ("logicram_ice40", &["SB_DFFE", "SB_LUT4", "SB_IO"]),
        ("carry_ice40", &["SB_CARRY", "SB_LUT4", "SB_DFF"]),
        ("blinky_ecp5", &["LUT4", "TRELLIS_FF", "TRELLIS_IO", "DCCA"]),
        ("ram_ecp5", &["DP16KD", "TRELLIS_IO"]),
        // The same memory on a family that has one.
        ("logicram_ecp5", &["TRELLIS_DPR16X4", "TRELLIS_IO"]),
        // Two read ports and one write port: the contents duplicated.
        ("regfile_ecp5", &["DP16KD", "TRELLIS_IO"]),
        ("clkbuf_ecp5", &["DCCA", "TRELLIS_FF", "TRELLIS_IO"]),
        // A clock constraint on a net nothing drives: a PLL.
        ("pll_ice40", &["SB_PLL40_CORE", "SB_GB", "SB_DFF"]),
        ("pll_ecp5", &["EHXPLLL", "DCCA", "TRELLIS_FF"]),
        // A port with a `ddr` attribute: the buffer registers both edges
        // on iCE40, a register beside it does on ECP5, and a delay
        // element sits in front of it where one was asked for.
        ("ddr_ice40", &["SB_IO"]),
        ("ddr_ecp5", &["TRELLIS_IO", "IDDRX1F", "ODDRX1F", "DELAYG"]),
        // The 7 series: a different IO primitive for each direction, a
        // six-input LUT, the four-bit carry element, and one flip-flop
        // per kind of reset. The counter resets to 16, so one of its
        // bits loads a one (FDSE) and the rest load zeros (FDRE); the
        // output register resets asynchronously (FDCE).
        (
            "blinky_xc7",
            &[
                "LUT6", "CARRY4", "FDRE", "FDSE", "FDCE", "IBUF", "OBUF", "BUFG",
            ],
        ),
        ("ram_xc7", &["RAMB18E1", "IBUF", "OBUF"]),
        // The same memory below the block RAM threshold, on the
        // family's own distributed RAM: one bit wide, sixty-four deep.
        ("logicram_xc7", &["RAM64X1D", "IBUF", "OBUF"]),
        // A PLL whose feedback loop is closed outside the block.
        ("pll_xc7", &["PLLE2_BASE", "BUFG", "FDRE"]),
    ];
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        let wanted = expected
            .iter()
            .find(|(case, _)| *case == name)
            .map(|(_, cells)| *cells)
            .unwrap_or(&[]);
        for cell in wanted {
            assert!(
                run.report.count(cell) > 0,
                "{name}: no `{cell}` in {:?}",
                run.report.netlist
            );
        }
    }
}

/// The constraints file names the pins of the device it targets, and the
/// device database knows them: a clean case produces no errors at all.
#[test]
fn constraints_check_cleanly() {
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        assert!(
            !run.diagnostics.contains("error["),
            "{name}: {}",
            run.diagnostics
        );
    }
}

/// Runs the real place-and-route tool over the export when one is
/// installed, and says what it said. When none is installed the test
/// prints why and returns, since nothing in this crate may depend on an
/// external tool being present.
#[test]
fn nextpnr_reads_the_export() {
    use std::process::Command;

    let mut ran = 0;
    let mut failed = Vec::new();
    for (name, device_name) in CASES {
        let run = run_case(name, device_name);
        if fpga::pnr_route(&run.device.family) != Some(fpga::PnrRoute::Nextpnr) {
            continue;
        }
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
        let log = String::from_utf8_lossy(&output.stderr).to_string();
        println!(
            "{name}: {tool} {} -> {}\n{log}",
            inputs.args[1..].join(" "),
            output.status
        );
        if !output.status.success() {
            failed.push(format!("{name}: {tool} exited with {}", output.status));
        }
    }
    if ran == 0 {
        println!(
            "no nextpnr is installed, so the export was checked against the device \
             database only (see `exports_are_acceptable_netlists`)"
        );
        return;
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    println!("{ran} of {} cases were placed and routed", CASES.len());
}

/// The logic fallback really *is* the memory it replaced.
///
/// `logicram_ice40` is sixteen words of eight bits with a clocked write
/// and an asynchronous read. The same stimulus is driven into the design
/// as written and into the same design after `fpga::map` has lowered the
/// memory into flip-flops, and the two have to answer alike — which
/// checks the decoded write enable, the read multiplexer and the
/// behaviour of a read during a write to the same address, all at once.
///
/// Only the memory step runs: IO and clock buffers would put primitives
/// in the netlist, and a primitive is a black box the simulator has no
/// model for.
#[cfg(feature = "sim")]
#[test]
fn the_logic_fallback_answers_like_the_memory_it_replaced() {
    use reticle::fpga::MapOptions;
    use reticle::logic::Logic;
    use reticle::sim::{NetHandle, SimOptions, Simulator};

    fn load(name: &str) -> Design {
        let text = read(&format!("{name}.rtl"));
        let mut sources = SourceMap::new();
        let file = sources.add(format!("{name}.rtl"), text.clone()).unwrap();
        Design::parse_text(&text, file)
            .unwrap_or_else(|d| panic!("{name}.rtl does not parse:\n{}", d.render(&sources)))
    }

    /// Drives the same writes and reads into one design and returns what
    /// it read back each time.
    fn exercise(design: &Design) -> Vec<u64> {
        const HALF: u64 = 500;
        let mut sim = Simulator::new(design, SimOptions::default())
            .unwrap_or_else(|d| panic!("cannot simulate: {} problem(s)", d.len()));
        let handle = |sim: &Simulator<'_>, name: &str| -> NetHandle {
            let path = format!("{}.{name}", sim.top_name());
            sim.net(&path).unwrap_or_else(|| panic!("no net `{path}`"))
        };
        let clk = handle(&sim, "clk");
        let we = handle(&sim, "we");
        let waddr = handle(&sim, "waddr");
        let raddr = handle(&sim, "raddr");
        let wdata = handle(&sim, "wdata");
        let rdata = handle(&sim, "rdata");

        let mut seen = Vec::new();
        let cycle = |sim: &mut Simulator<'_>| {
            sim.run_for(HALF);
            sim.set(clk, Logic::from_bool(true));
            sim.run_for(HALF);
            sim.set(clk, Logic::from_bool(false));
        };
        sim.set(clk, Logic::from_bool(false));
        // Write every word with a value only that word can have.
        for word in 0u64..16 {
            sim.set(we, Logic::from_bool(true));
            sim.set(waddr, Logic::from_u64(word, 4));
            sim.set(wdata, Logic::from_u64(0xA0 ^ (word * 7), 8));
            cycle(&mut sim);
        }
        // Read them all back, then read one while the same address is
        // written, which is where a decoded enable goes wrong.
        sim.set(we, Logic::from_bool(false));
        for word in 0u64..16 {
            sim.set(raddr, Logic::from_u64(word, 4));
            sim.run_for(HALF);
            seen.push(sim.get(rdata).to_u64().unwrap_or(u64::MAX));
        }
        sim.set(we, Logic::from_bool(true));
        sim.set(waddr, Logic::from_u64(5, 4));
        sim.set(raddr, Logic::from_u64(5, 4));
        sim.set(wdata, Logic::from_u64(0x5A, 8));
        sim.run_for(HALF);
        seen.push(sim.get(rdata).to_u64().unwrap_or(u64::MAX));
        cycle(&mut sim);
        sim.set(we, Logic::from_bool(false));
        sim.run_for(HALF);
        seen.push(sim.get(rdata).to_u64().unwrap_or(u64::MAX));
        // A write to one address must not disturb its neighbour.
        sim.set(raddr, Logic::from_u64(6, 4));
        sim.run_for(HALF);
        seen.push(sim.get(rdata).to_u64().unwrap_or(u64::MAX));
        seen
    }

    let original = load("logicram_ice40");
    let wanted = exercise(&original);
    assert_eq!(wanted.len(), 19);
    assert!(
        wanted.iter().all(|v| *v != u64::MAX),
        "the memory itself read x: {wanted:?}"
    );

    let mut lowered = load("logicram_ice40");
    let top = lowered.top.unwrap();
    let device = fpga::target("ice40-hx1k-tq144").unwrap();
    let options = MapOptions {
        insert_io_buffers: false,
        insert_clock_buffers: false,
        ..MapOptions::default()
    };
    let mut diags = Diagnostics::new();
    let report = fpga::map(
        &mut lowered,
        top,
        device,
        &Constraints::new(),
        &options,
        &mut diags,
    );
    assert!(report.bram_fallbacks[0].built);
    assert_eq!(report.bram_fallbacks[0].style, "flip-flops");
    assert!(!validate(&lowered).has_errors());
    assert_eq!(exercise(&lowered), wanted);
}

// ---------------------------------------------------------------------------
// Xilinx 7 series
//
// Nothing below has been run on a part. What these check is what the
// files say: that every cell is a primitive the device database
// declares, that the block RAM carries the contents the design gave it,
// that the constraints name pins Digilent's board file names, and that
// the family's unsupported corners are reported rather than silently
// mis-built.
// ---------------------------------------------------------------------------

/// The 7 series has no nextpnr back end in Reticle, and the export says
/// so by name instead of failing obscurely or writing half a file.
#[test]
fn a_vendor_family_declines_the_nextpnr_export() {
    let run = run_case("blinky_xc7", "xc7a35t-cpg236");
    let top = run.design.top.unwrap();
    assert_eq!(
        fpga::pnr_route(&run.device.family),
        Some(fpga::PnrRoute::Vendor)
    );
    let err = fpga::export_nextpnr(&run.design, top, run.device, &run.constraints)
        .expect_err("the 7 series has no nextpnr flow");
    let message = err.to_string();
    for wanted in ["xc7", "xc7a35t-cpg236", "Vivado", "export_vendor"] {
        assert!(
            message.contains(wanted),
            "`{message}` does not name {wanted}"
        );
    }
}

/// What a Basys 3 owner is actually handed: three files, named the way
/// the script names them, holding a netlist of real primitives and a
/// part string Vivado understands.
#[test]
fn the_basys3_export_is_the_three_files_vivado_reads() {
    let run = run_case("blinky_xc7", "xc7a35t-cpg236");
    let top = run.design.top.unwrap();
    let inputs = fpga::export_vendor(&run.design, top, run.device, &run.constraints)
        .expect("the vendor export");

    // The part string Vivado wants is the die, the package and the
    // speed grade run together.
    assert!(
        inputs.script.contains("-part xc7a35tcpg236-1"),
        "{}",
        inputs.script
    );
    assert_eq!(
        inputs.args,
        vec!["vivado", "-mode", "batch", "-source", "blinky.tcl"]
    );
    for step in [
        "read_verilog blinky.v",
        "read_xdc blinky.xdc",
        "synth_design -top blinky",
        "place_design",
        "route_design",
        "write_bitstream -force blinky.bit",
    ] {
        assert!(inputs.script.contains(step), "the script lacks `{step}`");
    }

    // The netlist instantiates 7-series primitives, with the pins those
    // primitives have. `IBUF` takes the pad on `I` and `OBUF` on `O`,
    // which is the reason the family needs one buffer per direction.
    for instance in [
        "IBUF clk$io0",
        "OBUF led$io0",
        "BUFG clk$gbuf",
        "FDSE #(.INIT(1'b1))",
        "FDCE #(.INIT(1'b0))",
    ] {
        assert!(
            inputs.verilog.contains(instance),
            "the Verilog lacks `{instance}`"
        );
    }
    assert!(inputs.verilog.contains("LUT6 #(.INIT(64'h"));

    // The XDC places the board's pins, and they are pins the part has.
    for line in [
        "set_property PACKAGE_PIN W5 [get_ports {clk}]",
        "set_property IOSTANDARD LVCMOS33 [get_ports {sw[0]}]",
        "set_property PACKAGE_PIN U16 [get_ports {led[0]}]",
        "create_clock -name sys -period 10.000 [get_ports {clk}]",
    ] {
        assert!(inputs.xdc.contains(line), "the XDC lacks `{line}`");
    }
    for pin in ["W5", "U18", "V17", "U16"] {
        let found = run.device.pin(pin).expect("a pin the database knows");
        assert!(found.kind.is_io(), "{pin} cannot carry a signal");
    }
    assert_eq!(
        run.device.pin("W5").unwrap().kind,
        reticle::fpga::PinKind::Clock
    );
}

/// Every pin constraint the board file states survives into the XDC,
/// and every pin it names is one the device database lists, so a typo
/// is an error here rather than a surprise in Vivado.
#[test]
fn the_board_constraints_round_trip_through_the_xdc() {
    for case in ["blinky_xc7", "ram_xc7", "pll_xc7"] {
        let run = run_case(case, "xc7a35t-cpg236");
        let top = run.design.top.unwrap();
        let xdc = fpga::export_vendor(&run.design, top, run.device, &run.constraints)
            .expect("the vendor export")
            .xdc;
        assert!(
            !run.diagnostics.contains("error["),
            "{case}: {}",
            run.diagnostics
        );
        for pin in &run.constraints.pins {
            if pin.pin.is_empty() {
                continue;
            }
            assert!(
                run.device.pin(&pin.pin).is_some(),
                "{case}: `{}` is not a pin of the part",
                pin.pin
            );
            let line = format!(
                "set_property PACKAGE_PIN {} [get_ports {{{}}}]",
                pin.pin,
                pin.signal()
            );
            assert!(xdc.contains(&line), "{case}: the XDC lacks `{line}`");
        }
    }
}

/// A memory with initial contents keeps them: the block RAM's
/// `INIT_00` holds the words the design named, least significant word
/// in the low bits, and every one of the sixty-four parameters is
/// written so that the block's contents are stated in full.
#[test]
fn a_block_ram_carries_its_contents_into_the_netlist() {
    let run = run_case("ram_xc7", "xc7a35t-cpg236");
    assert_eq!(run.report.count("RAMB18E1"), 1);
    let text = run.design.to_text();
    // cafe, babe, dead, beef, 0001, 8000 as sixteen-bit words, word 0
    // lowest: the six the design gave, then zeros.
    assert!(
        text.contains(
            "INIT_00=256'h000000000000000000000000000000000000000080000001beefdeadbabecafe"
        ),
        "the contents are not in INIT_00"
    );
    for index in 0..64u32 {
        assert!(
            text.contains(&format!("INIT_{index:02X}=")),
            "INIT_{index:02X} is missing"
        );
    }
    // The 16-bit mode's word address sits on the top ten address pins,
    // with the four below it tied low, which is how the 7-series block
    // addresses in units of its narrowest mode.
    assert!(text.contains("ADDRARDADDR={%raddr, 4'd0}"), "{text}");
    // A whole-word write drives every byte enable, not just the first.
    assert!(text.contains("WEBWE={%we, %we, %we, %we}"), "{text}");
}

/// The things this family cannot do are said out loud, and the blocks
/// Reticle cannot wire correctly are simply not declared.
#[test]
fn the_unsupported_corners_are_reported() {
    use reticle::fpga::BelRole;
    let run = run_case("blinky_xc7", "xc7a35t-cpg236");
    let notes = run.report.to_text();
    // The carry chain is no longer one of them: the counter's adder is
    // on CARRY4, four bits an instance, and nothing declines it.
    assert!(
        !notes.contains("stay generic"),
        "the carry chain is declined again:\n{notes}"
    );
    assert_eq!(run.report.count("CARRY4"), 2, "an 8-bit adder is two CARRY4");
    assert!(run.report.count("LUT6") > 0);
    let top = run.design.top.unwrap();
    assert!(fpga::check_nextpnr_json(&run.design, top, run.device, &run.constraints).is_empty());

    let device = fpga::target("xc7a35t-cpg236").expect("the part");
    assert!(
        device.bel(BelRole::DdrIn).is_none() && device.bel(BelRole::DdrOut).is_none(),
        "the 7-series file declares a DDR register it cannot wire"
    );
    assert!(device.bel(BelRole::IoDelay).is_none());
    assert!(device.dsps.is_empty(), "DSP48E1 is deliberately undeclared");
}

/// The part's own figures, so that a change to the device file that
/// contradicts the datasheet fails here.
#[test]
fn the_artix7_matches_its_datasheet_figures() {
    use reticle::fpga::BelRole;
    let device = fpga::target("xc7a35t-cpg236").expect("the part");
    assert_eq!(device.family, "xc7");
    assert_eq!(device.lut_size, 6);
    assert_eq!(device.bel(BelRole::Lut).unwrap().name, "LUT6");
    assert_eq!(device.bel(BelRole::Lut).unwrap().count, Some(20_800));
    assert_eq!(device.bel(BelRole::Ff).unwrap().count, Some(41_600));
    assert_eq!(device.clock_resources.global_buffers, 32);
    // 100 RAMB18E1 of 18 kbit; Reticle uses the 16 kbit of data and
    // leaves the parity bits, which is what the device file says.
    assert_eq!(device.block_rams.len(), 1);
    assert_eq!(device.block_rams[0].name, "RAMB18E1");
    assert_eq!(device.block_rams[0].bits(), 16_384);
    assert!(device.block_rams[0].has_byte_enable);
    // One clock generator of each kind in each of the five tiles.
    let plls: Vec<&str> = device
        .clock_resources
        .plls
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(plls, ["PLLE2_BASE", "MMCME2_BASE"]);
    for pll in &device.clock_resources.plls {
        assert_eq!(pll.count, Some(5));
        assert!(pll.is_configurable());
        // The loop is closed outside the block, so both ends are named.
        assert_eq!(pll.port("fbout"), Some("CLKFBOUT"));
        assert_eq!(pll.port("fb"), Some("CLKFBIN"));
    }
    // The carry element is the four-bit kind: a propagate, a generate
    // source, its own sums, and two ways in.
    let carry = device.bel(BelRole::Carry).unwrap();
    assert_eq!(carry.carry_width, Some(4));
    assert_eq!(carry.all_port_names(), ["CI", "CYINIT", "S", "DI", "O", "CO"]);
    let wide = carry.wide_carry().expect("a wide carry");
    assert_eq!((wide.propagate, wide.data), ("S", "DI"));
    assert_eq!((wide.sum, wide.carry_out), ("O", "CO"));
    // No tile grid: Reticle cannot state the SLICE array of this part.
    assert!(device.tile_grid.is_none());
    // The pin list is the Basys 3's, and it is not the whole package.
    assert!(device.pins_partial);
    assert!(device.pin("B18").is_some(), "the UART receive pin");
    assert!(device.pin("A18").is_some(), "the UART transmit pin");
    assert!(device.pin("G19").is_some(), "the first VGA red bit");
}
