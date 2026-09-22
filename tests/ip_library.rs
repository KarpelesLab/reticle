//! The Reticle IP library: every block elaborated, synthesised, measured
//! and simulated.
//!
//! The library itself is HDL, not Rust: one directory per block under
//! `ip/`, each a package with a `reticle.ip` manifest and its sources
//! under `rtl/`. This file is what makes it a *tested* library rather
//! than a folder of Verilog:
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `manifests_parse` | every `reticle.ip` parses, names its own directory and lists files that exist |
//! | `packages_resolve_and_elaborate` | every block builds through `ip::resolve` and `ip::elaborate`, dependencies and all |
//! | `blocks_synthesise_cleanly` | generic synthesis reports nothing — no errors, and no inferred latch |
//! | `footprints_match_the_documentation` | the table in `docs/ip-library.md` is the one this run measures |
//! | `axil_gpio_matches_the_axi4lite_definition` | `bus::match_ports` recognises the GPIO's bus port |
//! | `cdc_*`, `fifo_async_*` | `timing::analyze_cdc` calls every crossing a synchroniser, never an unsynchronised one |
//! | the rest | behaviour, driven through `sim::Simulator` |
//!
//! The behavioural tests are real testbenches: the UART transmits a byte
//! its own receiver recovers, the SPI master's bits are checked against
//! the `sclk` edges a slave would use, the I²C master is answered by a
//! slave model that acknowledges and stretches the clock, the
//! asynchronous FIFO passes data between two clocks with no common
//! period, and the PWM's duty cycle is counted over a whole period.
//!
//! The three larger blocks are tested the same way and harder.
//! `eth_mac_rmii` has its own transmitter looped into its own receiver,
//! and the frame on the pins is decoded independently in Rust against a
//! check sequence this file computes for itself. `spiflash_xip` answers
//! to a serial flash model on four wires. And `rv32i` runs **machine
//! code**: a small assembler in `mod asm` builds programs from the base
//! ISA's own field layout, a memory model answers both of the core's
//! ports, and the architectural state is read out of the register file
//! after each instruction. The last two of those programs are a loop
//! summing an array and Fibonacci computed recursively on a stack, so
//! the whole datapath is proved together and not only piece by piece.
//!
//! The blocks that need device primitives are held to their protocol by
//! models that enforce it. `sdram_ctrl` answers to an SDRAM model that
//! records every datasheet timing the controller breaks — and a test
//! that gives the model a slower part proves it would notice.
//! `hyperram_ctrl` answers to a HyperRAM model that insists on the
//! initial latency, through the DDR IO registers modelled in quarter
//! cycles. `dvi_tx`'s TMDS encoder is checked against the DVI
//! specification's algorithm for every byte from every running disparity
//! it can reach. `eth_mac_rgmii` loops its double-data-rate pins into
//! itself the way the RMII test does. `usb_device_fs` is enumerated by
//! a USB host model sending real packets, NRZI and bit stuffing and
//! CRCs included, on a clock a little off the device's.
//!
//! Six tests here came from gaps in Reticle rather than in the blocks,
//! found by writing real HDL, which is the argument for a first-party
//! library in the first place:
//! `ice40_flip_flops_take_an_active_low_reset_through_one_inverter`,
//! `small_memories_become_logic_after_the_fpga_flow`,
//! `function_locals_are_not_reported_as_unreset_registers`,
//! `a_two_read_port_register_file_is_duplicated_across_block_rams`,
//! `a_project_top_that_is_also_instantiated_with_an_override_keeps_its_name`
//! and `a_zero_step_io_delay_builds_nothing`. Each once asserted that its
//! gap was *still there*, so fixing it failed the test and named what to
//! change; all six now hold the fix. The paragraph in
//! `docs/ip-library.md` each points at records what was wrong.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the footprint table in
//! `docs/ip-library.md` after an intended change, and read the diff: a
//! block that suddenly costs twice as much is exactly what that table is
//! for.

#![cfg(all(
    feature = "ip",
    feature = "sim",
    feature = "synth",
    feature = "fpga",
    feature = "timing"
))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ip::{self, IpManifest, PathProvider};
use reticle::ir::hier::FlattenOptions;
use reticle::ir::{CellKind, Design, ModuleId};
use reticle::logic::Logic;
use reticle::sim::{MemHandle, NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;
use reticle::synth::techmap::{MapOptions, map_module};
use reticle::synth::{SynthOptions, run as synth_run};
use reticle::timing::cdc::{CrossingKind, analyze_cdc_with};
use reticle::timing::graph::flatten_for_timing;
use reticle::timing::sta::TimingSpec;
use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

/// One measured configuration: a package, the module inside it that is
/// measured and simulated, and the parameters it is built with.
struct Variant {
    /// The directory under `ip/`.
    package: &'static str,
    /// The module elaborated as the top.
    top: &'static str,
    /// Parameter overrides, as the HDL spells the values.
    params: &'static [(&'static str, &'static str)],
}

/// Every block of the library, in the order `docs/ip-library.md` lists
/// them. A block measured at two settings appears twice.
const VARIANTS: &[Variant] = &[
    Variant {
        package: "fifo_sync",
        top: "fifo_sync",
        params: &[("WIDTH", "8"), ("DEPTH", "16"), ("FWFT", "0")],
    },
    Variant {
        package: "fifo_sync",
        top: "fifo_sync",
        params: &[("WIDTH", "8"), ("DEPTH", "16"), ("FWFT", "1")],
    },
    Variant {
        package: "cdc_sync",
        top: "cdc_sync",
        params: &[("WIDTH", "1"), ("STAGES", "2")],
    },
    Variant {
        package: "cdc_sync",
        top: "cdc_sync",
        params: &[("WIDTH", "8"), ("STAGES", "3")],
    },
    Variant {
        package: "cdc_pulse",
        top: "cdc_pulse",
        params: &[],
    },
    Variant {
        package: "fifo_async",
        top: "fifo_async",
        params: &[("WIDTH", "8"), ("DEPTH", "16")],
    },
    Variant {
        package: "uart",
        top: "uart",
        params: &[("CLK_DIV", "104")],
    },
    Variant {
        package: "spi_master",
        top: "spi_master",
        params: &[
            ("CPOL", "0"),
            ("CPHA", "0"),
            ("CLK_DIV", "4"),
            ("WIDTH", "8"),
        ],
    },
    Variant {
        package: "i2c_master",
        top: "i2c_master",
        params: &[("CLK_DIV", "30")],
    },
    Variant {
        package: "pwm",
        top: "pwm",
        params: &[("WIDTH", "8")],
    },
    Variant {
        package: "timer",
        top: "timer",
        params: &[("WIDTH", "16"), ("PRESCALE_WIDTH", "8")],
    },
    Variant {
        package: "axil_gpio",
        top: "axil_gpio",
        params: &[("WIDTH", "8")],
    },
    Variant {
        package: "ram_wrapper",
        top: "ram_sp",
        params: &[("WIDTH", "8"), ("DEPTH", "256"), ("OUT_REG", "0")],
    },
    Variant {
        package: "ram_wrapper",
        top: "ram_sdp",
        params: &[("WIDTH", "8"), ("DEPTH", "256"), ("OUT_REG", "0")],
    },
    Variant {
        package: "rv32i",
        top: "rv32i",
        params: &[("REGFILE_BRAM", "0")],
    },
    Variant {
        package: "rv32i",
        top: "rv32i",
        params: &[("REGFILE_BRAM", "1")],
    },
    Variant {
        package: "eth_mac_rmii",
        top: "eth_mac_rmii",
        params: &[("IFG_CYCLES", "48")],
    },
    Variant {
        package: "spiflash_xip",
        top: "spiflash_xip",
        params: &[
            ("CLK_DIV", "2"),
            ("READ_CMD", "8'h03"),
            ("DUMMY_CYCLES", "0"),
        ],
    },
    Variant {
        package: "sdram_ctrl",
        top: "sdram_ctrl",
        params: &[("CLK_MHZ", "50"), ("CAS_LATENCY", "2")],
    },
    Variant {
        package: "hyperram_ctrl",
        top: "hyperram_ctrl",
        params: &[("ADDR_WIDTH", "22"), ("CK_DELAY", "100")],
    },
    Variant {
        package: "dvi_tx",
        top: "dvi_tx",
        params: &[("MODE", "0")],
    },
    Variant {
        package: "dvi_tx_pll",
        top: "dvi_tx_pll",
        params: &[("MODE", "0")],
    },
    Variant {
        package: "eth_mac_rgmii",
        top: "eth_mac_rgmii",
        params: &[("IFG_CYCLES", "12"), ("TX_DELAY", "80"), ("RX_DELAY", "80")],
    },
    Variant {
        package: "usb_device_fs",
        top: "usb_device_fs",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    Variant {
        package: "usb_device_fs_pll",
        top: "usb_device_fs_pll",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
];

/// Board constraints a variant needs to go through the FPGA flow, as
/// `.rcf` text: the clock a PLL is fed from, which only a board can
/// say. Everything else a block needs, its own attributes state.
fn board_constraints(variant: &Variant) -> &'static str {
    match variant.top {
        // A 25 MHz oscillator, which both families' PLLs take.
        "dvi_tx_pll" => "create_clock -name ref -period 40.0 clk_ref\n",
        // The 12 MHz oscillator of most small iCE40 boards.
        "usb_device_fs_pll" => "create_clock -name ref -period 83.333333 clk_ref\n",
        _ => "",
    }
}

/// The devices the footprint table reports, besides the generic LUT
/// mappings.
const DEVICES: [(&str, &str); 2] = [
    ("iCE40 HX1K", "ice40-hx1k-tq144"),
    ("ECP5 45F", "ecp5-45f-CABGA381"),
];

// ---------------------------------------------------------------------------
// Reading the library
// ---------------------------------------------------------------------------

fn ip_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ip")
}

/// The one place these tests touch the filesystem for the library.
fn read(rel: &str) -> Option<String> {
    fs::read_to_string(ip_dir().join(rel))
        .ok()
        .map(|t| t.replace("\r\n", "\n"))
}

/// Parses one package's manifest, failing loudly.
fn manifest(package: &str) -> IpManifest {
    let path = format!("{package}/reticle.ip");
    let text = read(&path).unwrap_or_else(|| panic!("no {path}"));
    let mut map = SourceMap::new();
    let file = map.add(path.clone(), &text).expect("manifest fits");
    let mut diags = Diagnostics::new();
    let ip = IpManifest::parse(&text, file, &mut diags)
        .unwrap_or_else(|| panic!("{path} does not parse:\n{}", diags.render(&map)));
    assert!(!diags.has_errors(), "{path}:\n{}", diags.render(&map));
    ip
}

/// The sources of `package` and everything it depends on, deepest first.
///
/// A dependency lives in the directory named after it, which is the rule
/// `PathProvider` applies too.
fn gather(package: &str, seen: &mut BTreeSet<String>, out: &mut Vec<(String, String)>) {
    if !seen.insert(package.to_owned()) {
        return;
    }
    let ip = manifest(package);
    for dep in &ip.depends {
        gather(&dep.name, seen, out);
    }
    for source in &ip.sources {
        let path = format!("{package}/{}", source.path);
        let text = read(&path).unwrap_or_else(|| panic!("no {path}"));
        out.push((path, text));
    }
}

/// Elaborates one variant into a design whose top is `top`.
///
/// This goes straight to the Verilog frontend rather than through
/// `ip::elaborate` for one reason: parameter overrides. A project build
/// takes a module's own defaults, and the footprint table and the
/// testbenches both need to choose.
fn design_of(package: &str, top: &str, params: &[(&str, &str)]) -> Design {
    let mut sources = Vec::new();
    gather(package, &mut BTreeSet::new(), &mut sources);

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::with_capacity(sources.len());
    for (path, text) in &sources {
        let id = map.add(path.clone(), text).expect("source fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    assert!(
        !diags.has_errors(),
        "{package} does not parse:\n{}",
        diags.render(&map)
    );

    let mut options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
    for (name, value) in params {
        options = options.with_param(*name, *value);
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(
        !diags.has_errors(),
        "{package}.{top} does not elaborate:\n{}",
        diags.render(&map)
    );
    let design = design.unwrap_or_else(|| panic!("{package}.{top} produced no design"));
    let problems = reticle::ir::validate::validate(&design);
    assert!(
        !problems.has_errors(),
        "{package}.{top} does not validate:\n{}",
        problems.render(&map)
    );
    design
}

/// A design flattened onto its top, which is what both the measurements
/// and the crossing analysis want.
fn flattened(package: &str, top: &str, params: &[(&str, &str)]) -> (Design, ModuleId) {
    let mut design = design_of(package, top, params);
    let id = design.top.expect("an elaborated top");
    design
        .flatten(id, &FlattenOptions::default())
        .unwrap_or_else(|d| panic!("{package}.{top} does not flatten: {}", d.len()));
    // Removing modules renumbers them, and the design's own `top` is
    // the reference that is renumbered with them.
    design.remove_unused_modules(id);
    let id = design.top.expect("the top survives");
    (design, id)
}

// ---------------------------------------------------------------------------
// Simulation helpers
// ---------------------------------------------------------------------------

fn bit(value: bool) -> Logic {
    Logic::from_bool(value)
}

fn word(width: u32, value: u64) -> Logic {
    Logic::from_u64(value, width)
}

/// A net of the top instance, whatever the elaboration named it: a
/// module built with parameter overrides is renamed after them.
fn top_net(sim: &Simulator<'_>, name: &str) -> NetHandle {
    let path = format!("{}.{}", sim.top_name(), name);
    net(sim, &path)
}

/// A named net, or a panic naming what was looked for.
fn net(sim: &Simulator<'_>, path: &str) -> NetHandle {
    sim.net(path)
        .unwrap_or_else(|| panic!("no net `{path}` in the simulation"))
}

fn get_u64(sim: &Simulator<'_>, handle: NetHandle) -> u64 {
    sim.get(handle)
        .to_u64()
        .unwrap_or_else(|| panic!("net holds x or z: {}", sim.get(handle)))
}

fn high(sim: &Simulator<'_>, handle: NetHandle) -> bool {
    get_u64(sim, handle) != 0
}

/// The low thirty-two bits of a value a net held.
fn narrow(value: u64) -> u32 {
    u32::try_from(value & 0xFFFF_FFFF).expect("thirty-two bits")
}

/// The low eight bits of one.
fn octet(value: u64) -> u8 {
    u8::try_from(value & 0xFF).expect("eight bits")
}

/// A thirty-two bit net's value.
fn get_u32(sim: &Simulator<'_>, handle: NetHandle) -> u32 {
    narrow(get_u64(sim, handle))
}

/// The two's complement bits of a signed immediate, which is what an
/// instruction encoding holds rather than the number itself.
fn twos(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

/// One clock cycle: the low phase, the rising edge, the high phase.
///
/// Inputs are set before the call and settle during the low phase, so
/// nothing changes in the same instant as the edge that samples it —
/// which is a race in a real simulator as much as in a real circuit.
/// Outputs read after the call are the ones the edge produced.
fn cycle(sim: &mut Simulator<'_>, clk: NetHandle, half: u64) {
    sim.run_for(half);
    sim.set(clk, bit(true));
    sim.run_for(half);
    sim.set(clk, bit(false));
}

// ---------------------------------------------------------------------------
// Manifests
// ---------------------------------------------------------------------------

/// Every package directory under `ip/`, sorted.
fn packages() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(ip_dir())
        .expect("the ip/ directory")
        .map(|e| e.expect("dir entry"))
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(
        !names.is_empty(),
        "no packages under {}",
        ip_dir().display()
    );
    names
}

#[test]
fn manifests_parse() {
    for package in packages() {
        let ip = manifest(&package);
        assert_eq!(ip.name, package, "a package's name is its directory");
        assert!(ip.license.is_some(), "{package} has no license");
        assert!(ip.description.is_some(), "{package} has no description");
        assert!(!ip.sources.is_empty(), "{package} lists no source");
        for source in &ip.sources {
            let path = format!("{package}/{}", source.path);
            assert!(read(&path).is_some(), "{path} is listed but missing");
            assert!(
                source.language().is_some(),
                "{path} has no recognisable language"
            );
        }
        // A `top` must be a module one of the sources defines, which the
        // elaboration tests below check for real; here, only that it is
        // named at all, since a package without one is ambiguous.
        assert!(ip.top.is_some(), "{package} names no top");
        // The manifest round-trips, so `reticle` can rewrite one.
        let mut map = SourceMap::new();
        let text = ip.to_text();
        let file = map.add("round-trip", &text).expect("fits");
        let mut diags = Diagnostics::new();
        let again = IpManifest::parse(&text, file, &mut diags).expect("re-parses");
        assert!(!diags.has_errors(), "{package} round-trip: {text}");
        // Spans differ, so compare what the words say.
        assert_eq!(again.to_text(), text, "{package} does not round-trip");
    }
}

#[test]
fn packages_resolve_and_elaborate() {
    for package in packages() {
        let ip = manifest(&package);
        let top = ip.top.clone().expect("a top");
        // A throwaway project that depends on nothing but this package,
        // which is exactly how a user reaches a library block.
        let project_text =
            format!("name {package}_check\ntop {top}\n\ndepends {package} * path {package}\n");

        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let project = ip::load_project(&mut map, "reticle.proj", &project_text, &mut diags)
            .expect("the generated project parses");
        let mut provider = PathProvider::new(".", read);
        let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
        assert!(
            resolved.is_complete(),
            "{package} does not resolve:\n{}",
            diags.render(resolved.source_map())
        );
        let build = ip::elaborate(&project, &mut resolved, &mut diags);
        assert!(
            !diags.has_errors(),
            "{package} does not build:\n{}",
            diags.render(resolved.source_map())
        );
        let design = build.design.expect("a design");
        let module = design
            .top_module()
            .unwrap_or_else(|| panic!("{package} produced no top"));
        assert_eq!(module.name.as_str(), top);
        assert!(build.blackboxes.is_empty(), "{package} became a black box");
        assert!(build.skipped.is_empty(), "{package} skipped a source");
    }
}

#[test]
fn blocks_synthesise_cleanly() {
    for variant in VARIANTS {
        let (mut design, _) = flattened(variant.package, variant.top, variant.params);
        let mut diags = Diagnostics::new();
        synth_run(&mut design, &SynthOptions::default(), &mut diags);
        let complaints: Vec<String> = diags
            .iter()
            .filter(|d| d.severity >= Severity::Warning)
            .map(|d| format!("{}: {}", d.severity, d.message))
            .collect();
        assert!(
            complaints.is_empty(),
            "{}.{} synthesises with complaints:\n  {}",
            variant.package,
            variant.top,
            complaints.join("\n  ")
        );
        // An inferred latch would be a `latch` cell, whatever it was
        // reported as.
        let module = design.module(design.top.expect("a top"));
        let latches = module
            .cells
            .iter()
            .filter(|(_, c)| matches!(c.kind, CellKind::Dlatch))
            .count();
        assert_eq!(
            latches, 0,
            "{}.{} inferred {latches} latch(es)",
            variant.package, variant.top
        );
    }
}

// ---------------------------------------------------------------------------
// Resource footprints
// ---------------------------------------------------------------------------

/// Every cell type of a module with how many there are, plus its
/// memories, in one deterministic line.
fn contents(design: &Design, id: ModuleId) -> String {
    let module = design.module(id);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, cell) in module.cells.iter() {
        let name = match &cell.kind {
            CellKind::Blackbox(name) => name.as_str().to_owned(),
            other => other.keyword().to_owned(),
        };
        *counts.entry(name).or_default() += 1;
    }
    for (_, memory) in module.memories.iter() {
        let key = format!(
            "memory {}x{}",
            memory.size,
            memory.elem.width().unwrap_or(0)
        );
        *counts.entry(key).or_default() += 1;
    }
    if counts.is_empty() {
        return "nothing".to_owned();
    }
    counts
        .into_iter()
        .map(|(name, n)| format!("{n} x {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One measured line of the table.
struct Measurement {
    target: String,
    cells: String,
    depth: u32,
}

/// Maps one variant onto `k`-input LUTs and everything else onto the
/// IR's own cells.
fn measure_lut(variant: &Variant, k: u32) -> Measurement {
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert!(
        !diags.has_errors(),
        "{}.{} does not synthesise",
        variant.package,
        variant.top
    );
    let stats = map_module(&mut design.modules[id], &MapOptions::lut(k));
    Measurement {
        target: format!("LUT{k}"),
        cells: contents(&design, id),
        depth: stats.depth,
    }
}

/// Runs the whole FPGA flow for one device.
fn measure_device(variant: &Variant, label: &str, device: &str) -> Measurement {
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let device = fpga::target(device).unwrap_or_else(|| panic!("no device `{device}`"));
    // The block's own attributes are constraints too: a `ddr` port is
    // built with its double-data-rate register, as a user's flow would.
    let mut diags = Diagnostics::new();
    let mut map = SourceMap::new();
    let rcf = board_constraints(variant);
    let file = map.add("board.rcf", rcf).expect("the constraints fit");
    let mut constraints = Constraints::parse(rcf, file, &mut diags);
    constraints.merge_attrs(&design, id, &mut diags);
    let options = FpgaOptions::default();
    let report = fpga::synthesize_for(&mut design, id, device, &constraints, &options, &mut diags)
        .unwrap_or_else(|e| {
            panic!(
                "{}.{} does not map for {label}: {e:?}",
                variant.package, variant.top
            )
        });
    let unexpected: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| format!("{}: {}", d.code.unwrap_or("-"), d.message))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{}.{} reports errors mapping for {label}:\n  {}",
        variant.package,
        variant.top,
        unexpected.join("\n  ")
    );
    Measurement {
        target: label.to_owned(),
        cells: contents(&design, id),
        depth: report.lut_depth,
    }
}

/// The whole table, as it appears between the markers in
/// `docs/ip-library.md`.
fn footprint_table() -> String {
    let mut out = String::new();
    out.push_str("| Block | Top | Parameters | Target | Cells | LUT depth |\n");
    out.push_str("|-------|-----|------------|--------|-------|-----------|\n");
    for variant in VARIANTS {
        let params = if variant.params.is_empty() {
            "(defaults)".to_owned()
        } else {
            variant
                .params
                .iter()
                .map(|(n, v)| format!("{n}={v}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut rows = vec![measure_lut(variant, 4), measure_lut(variant, 6)];
        for (label, device) in DEVICES {
            rows.push(measure_device(variant, label, device));
        }
        for row in rows {
            out.push_str(&format!(
                "| `{}` | `{}` | {} | {} | {} | {} |\n",
                variant.package, variant.top, params, row.target, row.cells, row.depth
            ));
        }
    }
    out
}

/// Where the generated table lives in the document.
const TABLE_BEGIN: &str = "<!-- footprints: generated by tests/ip_library.rs -->\n";
const TABLE_END: &str = "<!-- end footprints -->\n";

#[test]
fn footprints_match_the_documentation() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/ip-library.md");
    let doc = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n");
    let (head, rest) = doc
        .split_once(TABLE_BEGIN)
        .unwrap_or_else(|| panic!("{} has no footprint marker", path.display()));
    let (found, tail) = rest
        .split_once(TABLE_END)
        .unwrap_or_else(|| panic!("{} has no end marker", path.display()));

    let table = footprint_table();
    if found == table {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        let updated = format!("{head}{TABLE_BEGIN}{table}{TABLE_END}{tail}");
        fs::write(&path, updated).expect("rewrite the document");
        return;
    }
    let first = found
        .lines()
        .zip(table.lines())
        .find(|(a, b)| a != b)
        .map_or_else(
            || {
                format!(
                    "line count differs: {} documented, {} measured",
                    found.lines().count(),
                    table.lines().count()
                )
            },
            |(a, b)| format!("documented: {a}\nmeasured:   {b}"),
        );
    panic!(
        "the footprint table in docs/ip-library.md is out of date.\n{first}\n\nRun with \
         UPDATE_EXPECT=1 to refresh it."
    );
}

// ---------------------------------------------------------------------------
// The compiler gaps this library ran into
// ---------------------------------------------------------------------------

/// The iCE40 flip-flop mapping takes an active-low reset, through one
/// shared inverter.
///
/// Every block in this library resets on `negedge rst_n`, which is the
/// convention the rest of this repository's IP uses and the one nearly
/// all real HDL uses; every `SB_DFF*` primitive resets *high*. This used
/// to be `F0310` and a netlist full of generic `dff` cells. It is now an
/// inverter on the reset net and the active-high primitive, the way
/// `asic::library` has always handled a polarity a library lacks — and
/// the inverter is shared, so a reset reaching many flip-flops costs one
/// LUT and not one per flop.
#[test]
fn ice40_flip_flops_take_an_active_low_reset_through_one_inverter() {
    let variant = &VARIANTS[2]; // cdc_sync, two flops and nothing else
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let device = fpga::target("ice40-hx1k-tq144").expect("the iCE40 device");
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(
        &mut design,
        id,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");
    let refusals: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error)
        .map(|d| format!("{}: {}", d.code.unwrap_or("-"), d.message))
        .collect();
    assert!(
        refusals.is_empty(),
        "the iCE40 flip-flop mapping refuses something:\n  {}",
        refusals.join("\n  ")
    );
    // Both flops are real primitives now, and nothing generic is left.
    assert_eq!(report.device_cells.count("SB_DFFR"), 2);
    assert!(
        report
            .netlist
            .iter()
            .all(|(name, _)| !name.starts_with('$'))
    );
    // One inverter for the reset net the two flops share.
    assert_eq!(report.device_cells.inverters(), 1);
    let (net, pin) = &report.device_cells.inverted[0];
    assert!(net.contains("rst_n"), "the inverted net is `{net}`");
    assert_eq!(pin, "set/reset");
    assert!(fpga::check_nextpnr_json(&design, id, device, &Constraints::new()).is_empty());
}

/// A memory below the block-RAM threshold is built out of logic.
///
/// `fpga::primitives` used to record the decision — "it will be built
/// from distributed LUT RAM" — and then leave the `$memrd` and `$memwr`
/// cells in place, so `fpga::check_nextpnr_json` said the netlist was
/// unusable. The two FIFOs are where the library met it, since a 16 x 8
/// FIFO is 128 bits and the threshold is 256. The fallback is performed
/// now: the ECP5 has a distributed RAM primitive and uses it, the iCE40
/// has none and builds flip-flops with a decoded write enable and a read
/// multiplexer.
#[test]
fn small_memories_become_logic_after_the_fpga_flow() {
    let variant = &VARIANTS[0]; // fifo_sync, WIDTH=8 DEPTH=16
    for (device_name, style, primitive) in [
        (
            "ecp5-45f-CABGA381",
            "distributed LUT RAM",
            Some("TRELLIS_DPR16X4"),
        ),
        ("ice40-hx1k-tq144", "flip-flops", None),
    ] {
        let (mut design, id) = flattened(variant.package, variant.top, variant.params);
        let device = fpga::target(device_name).expect("a built-in device");
        let mut diags = Diagnostics::new();
        let report = fpga::synthesize_for(
            &mut design,
            id,
            device,
            &Constraints::new(),
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow runs");
        let fallback = report
            .primitives
            .bram_fallbacks
            .first()
            .expect("the 128-bit memory is below the block RAM threshold");
        assert!(fallback.reason.contains("threshold"), "{fallback:?}");
        assert!(fallback.built, "{device_name}: {fallback:?}");
        assert_eq!(fallback.style, style, "{device_name}");
        assert_eq!(fallback.primitive.as_deref(), primitive, "{device_name}");
        assert!(fallback.cells > 0, "{device_name}: nothing was built");

        // Nothing generic is left, so the netlist check is clean.
        let problems = fpga::check_nextpnr_json(&design, id, device, &Constraints::new());
        assert!(
            problems.is_empty(),
            "{device_name}: {}",
            problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

// ---------------------------------------------------------------------------
// Buses
// ---------------------------------------------------------------------------

#[test]
fn axil_gpio_matches_the_axi4lite_definition() {
    use reticle::ip::BusRole;
    use reticle::ip::bus::{builtin, match_ports};

    let ip = manifest("axil_gpio");
    let declared = ip
        .interfaces
        .first()
        .expect("axil_gpio declares a bus interface");
    assert_eq!(declared.bus, "axi4lite");
    assert_eq!(declared.role, BusRole::Subordinate);
    assert_eq!(declared.prefix(), "s_axi_");

    let design = design_of("axil_gpio", "axil_gpio", &[("WIDTH", "8")]);
    let module = design.top_module().expect("a top");
    let interface = builtin(&declared.bus).expect("the built-in axi4lite bus");
    let mapping = match_ports(module, interface, declared.role, &declared.prefix()).unwrap_or_else(
        |problems| {
            let lines: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
            panic!(
                "axil_gpio does not present an AXI4-Lite port:\n  {}",
                lines.join("\n  ")
            )
        },
    );
    // Every signal of the definition is there, optional ones included.
    assert_eq!(mapping.signals.len(), interface.signals.len());
}

// ---------------------------------------------------------------------------
// Clock domain crossings
// ---------------------------------------------------------------------------

/// The crossings of one block, as `timing::analyze_cdc` sees them after
/// synthesis and flattening.
fn crossings(package: &str, top: &str, params: &[(&str, &str)]) -> Vec<CrossingKind> {
    let (mut design, id) = flattened(package, top, params);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert!(!diags.has_errors(), "{package}.{top} does not synthesise");
    let module = flatten_for_timing(&design, id).expect("a flat module");
    let report = analyze_cdc_with(&module, &TimingSpec::default());
    assert!(
        !report.has_errors(),
        "{package}.{top} has a crossing reported as an error:\n{}",
        report.render()
    );
    report.crossings.into_iter().map(|c| c.kind).collect()
}

#[test]
fn cdc_pulse_crossings_are_synchronisers() {
    let kinds = crossings("cdc_pulse", "cdc_pulse", &[]);
    let synchronisers = kinds
        .iter()
        .filter(|k| matches!(k, CrossingKind::Synchroniser { depth: 2 }))
        .count();
    assert_eq!(
        synchronisers, 2,
        "both directions should be two-flop synchronisers, got {kinds:?}"
    );
    assert!(
        !kinds.contains(&CrossingKind::Unsynchronised),
        "an unsynchronised crossing in cdc_pulse: {kinds:?}"
    );
}

#[test]
fn fifo_async_pointers_cross_as_gray_synchronisers() {
    let kinds = crossings(
        "fifo_async",
        "fifo_async",
        &[("WIDTH", "8"), ("DEPTH", "16")],
    );
    assert!(
        !kinds.contains(&CrossingKind::Unsynchronised),
        "an unsynchronised crossing in fifo_async: {kinds:?}"
    );
    let gray = kinds
        .iter()
        .filter(|k| {
            matches!(
                k,
                CrossingKind::GrayBus {
                    generator_found: true,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        gray, 2,
        "both pointers should be recognised as gray coded, got {kinds:?}"
    );
    let synchronisers = kinds
        .iter()
        .filter(|k| matches!(k, CrossingKind::Synchroniser { .. }))
        .count();
    assert_eq!(synchronisers, 2, "got {kinds:?}");
}

// ---------------------------------------------------------------------------
// Behaviour
// ---------------------------------------------------------------------------

/// Half a clock period, in ticks. The blocks have no timescale, so the
/// simulator's default precision applies and any constant will do.
const HALF: u64 = 500;

fn simulate<'d>(design: &'d Design, what: &str) -> Simulator<'d> {
    Simulator::new(design, SimOptions::default())
        .unwrap_or_else(|d| panic!("{what} cannot be simulated: {} problem(s)", d.len()))
}

/// Holds `rst_n` low over two clock edges and releases it.
fn reset(sim: &mut Simulator<'_>, clk: NetHandle, rst_n: NetHandle) {
    sim.set(clk, bit(false));
    sim.set(rst_n, bit(false));
    sim.run_for(HALF);
    cycle(sim, clk, HALF);
    cycle(sim, clk, HALF);
    sim.set(rst_n, bit(true));
}

#[test]
fn derived_parameters_follow_the_depth_they_come_from() {
    // CNT_WIDTH is `$clog2(DEPTH) + 1`, so overriding DEPTH has to
    // re-evaluate it: a FIFO of four words counts 0 to 4 in three bits.
    let design = design_of("fifo_sync", "fifo_sync", &[("DEPTH", "4")]);
    let module = design.top_module().expect("a top");
    let port = module.port("count").expect("a count port");
    assert_eq!(module.nets[port.net].ty.width(), Some(3));

    let design = design_of("fifo_sync", "fifo_sync", &[("DEPTH", "256")]);
    let module = design.top_module().expect("a top");
    let port = module.port("count").expect("a count port");
    assert_eq!(module.nets[port.net].ty.width(), Some(9));
}

#[test]
fn fifo_sync_tracks_its_occupancy() {
    let design = design_of(
        "fifo_sync",
        "fifo_sync",
        &[("WIDTH", "8"), ("DEPTH", "4"), ("FWFT", "0")],
    );
    let mut sim = simulate(&design, "fifo_sync");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let wr_en = top_net(&sim, "wr_en");
    let wr_data = top_net(&sim, "wr_data");
    let rd_en = top_net(&sim, "rd_en");
    let rd_data = top_net(&sim, "rd_data");
    let full = top_net(&sim, "full");
    let empty = top_net(&sim, "empty");
    let count = top_net(&sim, "count");

    reset(&mut sim, clk, rst_n);
    assert!(high(&sim, empty), "a reset FIFO is empty");
    assert!(!high(&sim, full));
    assert_eq!(get_u64(&sim, count), 0);

    // Fill it one word at a time.
    for (i, value) in [0x11u64, 0x22, 0x33, 0x44].into_iter().enumerate() {
        assert!(!high(&sim, full), "full after {i} of 4 words");
        sim.set(wr_en, bit(true));
        sim.set(wr_data, word(8, value));
        cycle(&mut sim, clk, HALF);
        assert_eq!(get_u64(&sim, count), i as u64 + 1);
        assert!(!high(&sim, empty));
    }
    assert!(high(&sim, full), "four words fill a four-word FIFO");

    // A write while full is ignored rather than wrapping the pointer.
    sim.set(wr_data, word(8, 0xFF));
    cycle(&mut sim, clk, HALF);
    assert_eq!(get_u64(&sim, count), 4);
    sim.set(wr_en, bit(false));

    // Read them back in order. The read port is registered, so the word
    // is on `rd_data` in the cycle after the one that popped it.
    for (i, value) in [0x11u64, 0x22, 0x33, 0x44].into_iter().enumerate() {
        assert!(!high(&sim, empty));
        sim.set(rd_en, bit(true));
        cycle(&mut sim, clk, HALF);
        assert_eq!(get_u64(&sim, rd_data), value, "word {i}");
        assert_eq!(get_u64(&sim, count), 3 - i as u64);
    }
    sim.set(rd_en, bit(false));
    assert!(high(&sim, empty), "the FIFO is empty again");
    assert!(!high(&sim, full));

    // A read and a write in the same cycle leave the count alone.
    sim.set(wr_en, bit(true));
    sim.set(wr_data, word(8, 0xAA));
    cycle(&mut sim, clk, HALF);
    sim.set(wr_data, word(8, 0xBB));
    sim.set(rd_en, bit(true));
    cycle(&mut sim, clk, HALF);
    assert_eq!(get_u64(&sim, count), 1);
    assert_eq!(get_u64(&sim, rd_data), 0xAA);
}

#[test]
fn fifo_sync_falls_through_when_asked_to() {
    let design = design_of(
        "fifo_sync",
        "fifo_sync",
        &[("WIDTH", "8"), ("DEPTH", "4"), ("FWFT", "1")],
    );
    let mut sim = simulate(&design, "fifo_sync (FWFT)");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let wr_en = top_net(&sim, "wr_en");
    let wr_data = top_net(&sim, "wr_data");
    let rd_en = top_net(&sim, "rd_en");
    let rd_data = top_net(&sim, "rd_data");
    let empty = top_net(&sim, "empty");

    reset(&mut sim, clk, rst_n);

    // One write, and the word is on the output in the very next cycle
    // with no read strobe at all. That is the whole point of FWFT.
    sim.set(wr_en, bit(true));
    sim.set(wr_data, word(8, 0x5A));
    cycle(&mut sim, clk, HALF);
    sim.set(wr_data, word(8, 0xC3));
    cycle(&mut sim, clk, HALF);
    sim.set(wr_en, bit(false));
    assert!(!high(&sim, empty));
    assert_eq!(get_u64(&sim, rd_data), 0x5A, "the first word falls through");

    // `rd_en` acknowledges it; the next word is there in the next cycle.
    sim.set(rd_en, bit(true));
    cycle(&mut sim, clk, HALF);
    assert_eq!(get_u64(&sim, rd_data), 0xC3);
    cycle(&mut sim, clk, HALF);
    sim.set(rd_en, bit(false));
    assert!(high(&sim, empty));
}

#[test]
fn cdc_sync_delays_by_its_stage_count() {
    let design = design_of("cdc_sync", "cdc_sync", &[("WIDTH", "1"), ("STAGES", "3")]);
    let mut sim = simulate(&design, "cdc_sync");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let d = top_net(&sim, "d");
    let q = top_net(&sim, "q");

    reset(&mut sim, clk, rst_n);
    assert!(!high(&sim, q));

    sim.set(d, bit(true));
    for cycles in 1..=2 {
        cycle(&mut sim, clk, HALF);
        assert!(!high(&sim, q), "three stages arrived after {cycles}");
    }
    cycle(&mut sim, clk, HALF);
    assert!(high(&sim, q), "three stages, three cycles");

    sim.set(d, bit(false));
    cycle(&mut sim, clk, HALF);
    cycle(&mut sim, clk, HALF);
    assert!(high(&sim, q));
    cycle(&mut sim, clk, HALF);
    assert!(!high(&sim, q));
}

/// A clock running on its own period, toggled from absolute time so two
/// of them can be genuinely unrelated: no common edge, no whole ratio,
/// which is the only honest way to test a crossing.
struct FreeClock {
    net: NetHandle,
    half: u64,
    next: u64,
    level: bool,
}

/// How long before an edge a two-domain testbench presents its inputs.
///
/// Shorter than either half period and longer than nothing: a value
/// changed in the same instant as the edge that samples it is a race,
/// and an event-driven simulator is entitled to resolve it either way.
const SETUP: u64 = 10;

impl FreeClock {
    fn new(net: NetHandle, half: u64) -> FreeClock {
        FreeClock {
            net,
            half,
            next: half,
            level: false,
        }
    }

    /// True when `time` is this clock's next rising edge.
    fn rises_at(&self, time: u64) -> bool {
        self.next == time && !self.level
    }

    /// Takes this clock's edge if it falls at `time`.
    fn toggle_at(&mut self, sim: &mut Simulator<'_>, time: u64) {
        if self.next == time {
            self.level = !self.level;
            sim.set(self.net, bit(self.level));
            self.next += self.half;
        }
    }
}

/// The time of the next edge of either clock.
fn next_edge(a: &FreeClock, b: &FreeClock) -> u64 {
    a.next.min(b.next)
}

#[test]
fn cdc_pulse_delivers_one_pulse_per_request() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let design = design_of("cdc_pulse", "cdc_pulse", &[]);
    let mut sim = simulate(&design, "cdc_pulse");
    let src_clk = top_net(&sim, "src_clk");
    let src_rst_n = top_net(&sim, "src_rst_n");
    let src_pulse = top_net(&sim, "src_pulse");
    let src_busy = top_net(&sim, "src_busy");
    let dst_clk = top_net(&sim, "dst_clk");
    let dst_rst_n = top_net(&sim, "dst_rst_n");
    let dst_out = top_net(&sim, "dst_pulse");

    // Every rising edge of `dst_pulse` is one delivered request.
    let seen = Rc::new(RefCell::new(0u32));
    let counter = Rc::clone(&seen);
    let mut was_high = false;
    sim.on_change(dst_out, move |_, value| {
        let now = value.to_u64() == Some(1);
        if now && !was_high {
            *counter.borrow_mut() += 1;
        }
        was_high = now;
    });

    // 30 : 107 shares no factor, and the destination is by far the
    // slower domain, which is the case a toggle handshake exists for.
    let mut src = FreeClock::new(src_clk, 30);
    let mut dst = FreeClock::new(dst_clk, 107);

    sim.set(src_rst_n, bit(false));
    sim.set(dst_rst_n, bit(false));
    sim.set(src_pulse, bit(false));
    while sim.time() < 600 {
        let t = next_edge(&src, &dst);
        sim.run_until(t);
        src.toggle_at(&mut sim, t);
        dst.toggle_at(&mut sim, t);
    }
    sim.set(src_rst_n, bit(true));
    sim.set(dst_rst_n, bit(true));

    // Three requests, each presented for exactly one source clock and
    // only while the block says it is free. A fourth is never offered,
    // so the count at the end is what was asked for and no more.
    let mut requested = 0u32;
    while sim.time() < 12_000 {
        let t = next_edge(&src, &dst);
        // Decide, and present, a setup time before the edge.
        sim.run_until(t - SETUP);
        let rise_src = src.rises_at(t);
        let asserted = rise_src && requested < 3 && !high(&sim, src_busy);
        if rise_src {
            sim.set(src_pulse, bit(asserted));
        }
        sim.run_until(t);
        src.toggle_at(&mut sim, t);
        dst.toggle_at(&mut sim, t);
        if asserted {
            requested += 1;
        }
    }

    assert_eq!(requested, 3, "three requests should have been accepted");
    assert_eq!(*seen.borrow(), 3, "one destination pulse per request");
    assert!(
        !high(&sim, src_busy),
        "the acknowledgement should have come home"
    );
}

#[test]
fn fifo_async_carries_data_between_unrelated_clocks() {
    let design = design_of(
        "fifo_async",
        "fifo_async",
        &[("WIDTH", "8"), ("DEPTH", "4")],
    );
    let mut sim = simulate(&design, "fifo_async");
    let wr_clk = top_net(&sim, "wr_clk");
    let wr_rst_n = top_net(&sim, "wr_rst_n");
    let wr_en = top_net(&sim, "wr_en");
    let wr_data = top_net(&sim, "wr_data");
    let wr_full = top_net(&sim, "wr_full");
    let rd_clk = top_net(&sim, "rd_clk");
    let rd_rst_n = top_net(&sim, "rd_rst_n");
    let rd_en = top_net(&sim, "rd_en");
    let rd_data = top_net(&sim, "rd_data");
    let rd_empty = top_net(&sim, "rd_empty");

    // A four-word FIFO and sixteen words to push through it, so the
    // full and the empty handshake are each exercised several times.
    let sent: Vec<u64> = (0..16).map(|i| 0x10 + i * 7).collect();
    let mut received: Vec<u64> = Vec::new();
    let mut next = 0usize;

    // The reader is slower, and the two periods share no factor.
    let mut wr = FreeClock::new(wr_clk, 50);
    let mut rd = FreeClock::new(rd_clk, 71);

    sim.set(wr_rst_n, bit(false));
    sim.set(rd_rst_n, bit(false));
    sim.set(wr_en, bit(false));
    sim.set(rd_en, bit(false));
    while sim.time() < 600 {
        let t = next_edge(&wr, &rd);
        sim.run_until(t);
        wr.toggle_at(&mut sim, t);
        rd.toggle_at(&mut sim, t);
    }
    sim.set(wr_rst_n, bit(true));
    sim.set(rd_rst_n, bit(true));
    sim.run_for(HALF);
    assert!(high(&sim, rd_empty), "a reset FIFO reads empty");
    assert!(!high(&sim, wr_full));

    while sim.time() < 120_000 && received.len() < sent.len() {
        let t = next_edge(&wr, &rd);
        // Both sides look at the flags and present their strobes a
        // setup time before the edge that will sample them.
        sim.run_until(t - SETUP);
        let rise_wr = wr.rises_at(t);
        let rise_rd = rd.rises_at(t);
        if rise_wr {
            let writing = next < sent.len() && !high(&sim, wr_full);
            sim.set(wr_en, bit(writing));
            if writing {
                sim.set(wr_data, word(8, sent[next]));
                next += 1;
            }
        }
        if rise_rd {
            // First word fall through: the word is already on `rd_data`
            // and `rd_en` only acknowledges it.
            let reading = !high(&sim, rd_empty);
            sim.set(rd_en, bit(reading));
            if reading {
                received.push(get_u64(&sim, rd_data));
            }
        }
        sim.run_until(t);
        wr.toggle_at(&mut sim, t);
        rd.toggle_at(&mut sim, t);
    }

    assert_eq!(received, sent, "every word, in order, across two clocks");
}

#[test]
fn uart_receives_the_byte_its_own_transmitter_sends() {
    let design = design_of("uart", "uart", &[("CLK_DIV", "8")]);
    let mut sim = simulate(&design, "uart");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let tx_data = top_net(&sim, "tx_data");
    let tx_valid = top_net(&sim, "tx_valid");
    let tx_ready = top_net(&sim, "tx_ready");
    let tx = top_net(&sim, "tx");
    let rx = top_net(&sim, "rx");
    let rx_data = top_net(&sim, "rx_data");
    let rx_valid = top_net(&sim, "rx_valid");
    let rx_error = top_net(&sim, "rx_error");

    sim.set(rx, bit(true)); // an idle line is high
    sim.set(tx_valid, bit(false));
    reset(&mut sim, clk, rst_n);
    assert!(high(&sim, tx), "the transmitter idles high");
    assert!(high(&sim, tx_ready), "and is ready straight out of reset");

    // Two bytes back to back, with the transmitter's own output looped
    // into the receiver one cycle at a time. Nothing but the two blocks
    // and one wire between them.
    let bytes = [0xA5u64, 0x3C];
    let mut sending = 0usize;
    let mut got: Vec<(u64, bool)> = Vec::new();

    sim.set(tx_data, word(8, bytes[0]));
    sim.set(tx_valid, bit(true));
    for _ in 0..400 {
        let level = high(&sim, tx);
        sim.set(rx, bit(level));
        let accepted = high(&sim, tx_valid) && high(&sim, tx_ready);
        cycle(&mut sim, clk, HALF);
        if accepted {
            sending += 1;
            if sending < bytes.len() {
                sim.set(tx_data, word(8, bytes[sending]));
            } else {
                sim.set(tx_valid, bit(false));
            }
        }
        if high(&sim, rx_valid) {
            got.push((get_u64(&sim, rx_data), high(&sim, rx_error)));
        }
    }

    assert_eq!(sending, bytes.len(), "both bytes were accepted");
    assert_eq!(
        got,
        vec![(bytes[0], false), (bytes[1], false)],
        "both bytes should come back with no framing error"
    );
}

#[test]
fn uart_reports_a_framing_error_when_the_stop_bit_is_missing() {
    let design = design_of("uart", "uart", &[("CLK_DIV", "8")]);
    let mut sim = simulate(&design, "uart");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let rx = top_net(&sim, "rx");
    let rx_data = top_net(&sim, "rx_data");
    let rx_valid = top_net(&sim, "rx_valid");
    let rx_error = top_net(&sim, "rx_error");

    sim.set(rx, bit(true));
    reset(&mut sim, clk, rst_n);

    // A start bit, eight data bits and a stop bit held *low*, driven by
    // hand at eight clocks a bit.
    let byte = 0x7Eu64;
    let mut line: Vec<bool> = vec![false]; // start
    for i in 0..8 {
        line.push((byte >> i) & 1 == 1);
    }
    line.push(false); // a broken stop bit

    let mut got = None;
    for level in line {
        for _ in 0..8 {
            sim.set(rx, bit(level));
            cycle(&mut sim, clk, HALF);
            if high(&sim, rx_valid) {
                got = Some((get_u64(&sim, rx_data), high(&sim, rx_error)));
            }
        }
    }
    sim.set(rx, bit(true));
    for _ in 0..16 {
        cycle(&mut sim, clk, HALF);
        if high(&sim, rx_valid) {
            got = Some((get_u64(&sim, rx_data), high(&sim, rx_error)));
        }
    }

    assert_eq!(
        got,
        Some((byte, true)),
        "the byte arrives, and the framing error with it"
    );
}

/// One SPI transfer in the given mode: what the master put on `mosi`,
/// gathered at the `sclk` edges a slave would sample on, and what it
/// shifted in from `miso`.
///
/// Modes 0 and 3 both sample on the rising edge — mode 0 because the
/// rising edge is the leading one and CPHA is 0, mode 3 because the
/// rising edge is the trailing one and CPHA is 1 — so one slave model
/// serves both.
fn spi_round_trip(cpol: u64, cpha: u64, master_byte: u64, slave_byte: u64) -> (u64, u64) {
    let cpol_text = cpol.to_string();
    let cpha_text = cpha.to_string();
    let design = design_of(
        "spi_master",
        "spi_master",
        &[
            ("CPOL", cpol_text.as_str()),
            ("CPHA", cpha_text.as_str()),
            ("CLK_DIV", "2"),
            ("WIDTH", "8"),
        ],
    );
    let mut sim = simulate(&design, "spi_master");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let tx_data = top_net(&sim, "tx_data");
    let start = top_net(&sim, "start");
    let done = top_net(&sim, "done");
    let rx_data = top_net(&sim, "rx_data");
    let sclk = top_net(&sim, "sclk");
    let mosi = top_net(&sim, "mosi");
    let miso = top_net(&sim, "miso");
    let cs_n = top_net(&sim, "cs_n");

    sim.set(start, bit(false));
    sim.set(miso, bit(false));
    reset(&mut sim, clk, rst_n);
    assert_eq!(
        high(&sim, sclk),
        cpol == 1,
        "sclk should idle at CPOL in mode {cpol}{cpha}"
    );
    assert!(high(&sim, cs_n), "cs_n is released between transfers");

    // The slave presents its first bit before the first edge, exactly as
    // a real one does when the select falls.
    sim.set(miso, bit((slave_byte >> 7) & 1 == 1));
    sim.set(tx_data, word(8, master_byte));
    sim.set(start, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(start, bit(false));
    assert!(
        !high(&sim, cs_n),
        "cs_n falls when the transfer is accepted"
    );

    let mut previous = high(&sim, sclk);
    let mut seen: Vec<bool> = Vec::new();
    let mut index = 0usize;
    let mut finished = false;
    for _ in 0..200 {
        cycle(&mut sim, clk, HALF);
        let level = high(&sim, sclk);
        if level && !previous {
            assert!(!high(&sim, cs_n), "cs_n stays low for the whole frame");
            seen.push(high(&sim, mosi));
            index += 1;
            if index < 8 {
                sim.set(miso, bit((slave_byte >> (7 - index)) & 1 == 1));
            }
        }
        previous = level;
        if high(&sim, done) {
            finished = true;
            break;
        }
    }

    assert!(finished, "mode {cpol}{cpha} never finished");
    assert_eq!(
        seen.len(),
        8,
        "mode {cpol}{cpha} clocked {} bits",
        seen.len()
    );
    assert!(high(&sim, cs_n), "cs_n rises again at the end");
    assert_eq!(
        high(&sim, sclk),
        cpol == 1,
        "sclk returns to CPOL in mode {cpol}{cpha}"
    );
    let sent = seen.iter().fold(0u64, |acc, b| (acc << 1) | u64::from(*b));
    (sent, get_u64(&sim, rx_data))
}

#[test]
fn spi_master_clocks_mode_0_and_mode_3() {
    // Mode 0: CPOL = 0, CPHA = 0.
    let (sent, received) = spi_round_trip(0, 0, 0xA5, 0x3C);
    assert_eq!(
        sent, 0xA5,
        "mode 0 puts the byte out most significant first"
    );
    assert_eq!(received, 0x3C, "mode 0 shifts the slave's byte in");

    // Mode 3: CPOL = 1, CPHA = 1. The same bits, half a period later.
    let (sent, received) = spi_round_trip(1, 1, 0xA5, 0x3C);
    assert_eq!(
        sent, 0xA5,
        "mode 3 puts the byte out most significant first"
    );
    assert_eq!(received, 0x3C, "mode 3 shifts the slave's byte in");

    // And a second pattern, so a stuck bit cannot pass both.
    assert_eq!(spi_round_trip(0, 0, 0x01, 0x80), (0x01, 0x80));
    assert_eq!(spi_round_trip(1, 1, 0xFE, 0x7F), (0xFE, 0x7F));
}

/// One command for the I²C master: what to put on the bus and what the
/// slave model should do about it.
struct I2cCommand {
    start: bool,
    stop: bool,
    read: bool,
    data: u64,
    /// The acknowledgement the master sends after a read byte.
    ack: bool,
    /// The byte the slave shifts out, for a read.
    slave_byte: u64,
}

#[test]
fn i2c_master_addresses_writes_reads_and_waits_for_a_stretched_clock() {
    let design = design_of("i2c_master", "i2c_master", &[("CLK_DIV", "4")]);
    let mut sim = simulate(&design, "i2c_master");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let start = top_net(&sim, "start");
    let cmd_start = top_net(&sim, "cmd_start");
    let cmd_stop = top_net(&sim, "cmd_stop");
    let cmd_read = top_net(&sim, "cmd_read");
    let wr_data = top_net(&sim, "wr_data");
    let ack_in = top_net(&sim, "ack_in");
    let busy = top_net(&sim, "busy");
    let done = top_net(&sim, "done");
    let rd_data = top_net(&sim, "rd_data");
    let ack_out = top_net(&sim, "ack_out");
    let scl_o = top_net(&sim, "scl_o");
    let scl_i = top_net(&sim, "scl_i");
    let sda_o = top_net(&sim, "sda_o");
    let sda_i = top_net(&sim, "sda_i");

    // A whole seven-bit addressed exchange: address the device for
    // writing, write a byte, repeat the start to turn the bus around,
    // address it for reading and take one byte with a closing NACK.
    let commands = [
        I2cCommand {
            start: true,
            stop: false,
            read: false,
            data: 0xA4,
            ack: false,
            slave_byte: 0,
        },
        I2cCommand {
            start: false,
            stop: false,
            read: false,
            data: 0x5A,
            ack: false,
            slave_byte: 0,
        },
        I2cCommand {
            start: true,
            stop: false,
            read: false,
            data: 0xA5,
            ack: false,
            slave_byte: 0,
        },
        I2cCommand {
            start: false,
            stop: true,
            read: true,
            data: 0,
            ack: false,
            slave_byte: 0x3C,
        },
    ];

    sim.set(start, bit(false));
    sim.set(scl_i, bit(true));
    sim.set(sda_i, bit(true));
    reset(&mut sim, clk, rst_n);

    // The bus: open drain, so each line is the AND of what the two ends
    // drive, and a pull-up holds it high when neither does.
    let mut slave_sda_low = false;
    let mut slave_scl_low = false;
    let mut stretch_left = 0u32;
    let mut stretch_used = false;
    let mut previous_scl = true;
    let mut previous_sda = true;
    let mut bit_index = 0usize;
    let mut bits: Vec<bool> = Vec::new();
    let mut starts = 0u32;
    let mut stops = 0u32;

    let mut issued = 0usize;
    let mut in_flight = false;
    let mut reading = false;
    let mut slave_byte = 0u64;
    let mut results: Vec<(u64, u64, bool)> = Vec::new();

    for _ in 0..6000 {
        // Drive the bus from both ends before the edge.
        let bus_scl = high(&sim, scl_o) && !slave_scl_low;
        let bus_sda = high(&sim, sda_o) && !slave_sda_low;
        sim.set(scl_i, bit(bus_scl));
        sim.set(sda_i, bit(bus_sda));

        if !in_flight && issued < commands.len() && !high(&sim, busy) {
            let command = &commands[issued];
            sim.set(cmd_start, bit(command.start));
            sim.set(cmd_stop, bit(command.stop));
            sim.set(cmd_read, bit(command.read));
            sim.set(wr_data, word(8, command.data));
            sim.set(ack_in, bit(command.ack));
            sim.set(start, bit(true));
            reading = command.read;
            slave_byte = command.slave_byte;
            bit_index = 0;
            if reading {
                // The slave was told to expect a read by the address
                // byte, so its first bit is already on the line when the
                // clock next rises, as a real one's would be.
                slave_sda_low = (slave_byte >> 7) & 1 == 0;
            }
            in_flight = true;
        } else {
            sim.set(start, bit(false));
        }

        cycle(&mut sim, clk, HALF);
        sim.set(start, bit(false));

        if high(&sim, done) {
            results.push((
                get_u64(&sim, rd_data),
                bits.iter().fold(0u64, |acc, b| (acc << 1) | u64::from(*b)),
                high(&sim, ack_out),
            ));
            bits.clear();
            issued += 1;
            in_flight = false;
        }

        // Watch the bus as a logic analyser would.
        let scl_now = high(&sim, scl_o) && !slave_scl_low;
        let sda_now = high(&sim, sda_o) && !slave_sda_low;
        if previous_scl && scl_now {
            if previous_sda && !sda_now {
                starts += 1;
                bit_index = 0;
                bits.clear();
            } else if !previous_sda && sda_now {
                stops += 1;
            }
        }
        if !previous_scl && scl_now {
            // Only the eight data bits: the acknowledgement bit and the
            // clock pulse a stop condition rides on are not data.
            if bit_index < 8 && bits.len() < 8 {
                bits.push(sda_now);
            }
            bit_index += 1;
        }
        if previous_scl && !scl_now {
            if reading {
                if bit_index < 8 {
                    slave_sda_low = (slave_byte >> (7 - bit_index)) & 1 == 0;
                } else {
                    slave_sda_low = false;
                    if bit_index >= 9 {
                        bit_index = 0;
                    }
                }
            } else if bit_index == 8 {
                slave_sda_low = true; // acknowledge the byte
            } else if bit_index >= 9 {
                slave_sda_low = false;
                bit_index = 0;
            }
            // Hold the clock down in the middle of the second byte, the
            // thing a slow slave does and a master must survive.
            if issued == 1 && bit_index == 4 && !stretch_used {
                slave_scl_low = true;
                stretch_left = 40;
                stretch_used = true;
            }
        }
        previous_scl = scl_now;
        previous_sda = sda_now;

        if slave_scl_low {
            stretch_left -= 1;
            if stretch_left == 0 {
                slave_scl_low = false;
            }
        }

        if issued == commands.len() {
            break;
        }
    }

    assert!(stretch_used, "the slave never got to stretch the clock");
    assert_eq!(issued, commands.len(), "not every command finished");
    assert_eq!(starts, 2, "one start and one repeated start");
    assert_eq!(stops, 1, "exactly one stop, at the end");

    assert_eq!(results[0].1, 0xA4, "the address byte went out");
    assert!(results[0].2, "the slave acknowledged the address");
    assert_eq!(
        results[1].1, 0x5A,
        "the data byte went out through the stretch"
    );
    assert!(results[1].2, "the slave acknowledged the data");
    assert_eq!(results[2].1, 0xA5, "the read address went out");
    assert!(results[2].2, "the slave acknowledged the read address");
    assert_eq!(results[3].0, 0x3C, "the byte the slave sent came back");
    assert_eq!(results[3].1, 0x3C, "and it is what was on the wire");
}

#[test]
fn pwm_drives_the_duty_it_is_given() {
    let design = design_of("pwm", "pwm", &[("WIDTH", "4")]);
    let mut sim = simulate(&design, "pwm");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let en = top_net(&sim, "en");
    let duty = top_net(&sim, "duty");
    let pwm_out = top_net(&sim, "pwm_out");
    let duty_active = top_net(&sim, "duty_active");
    let period_tick = top_net(&sim, "period_tick");

    sim.set(en, bit(false));
    sim.set(duty, word(4, 0));
    reset(&mut sim, clk, rst_n);

    // WIDTH = 4, so a period is sixteen cycles and the duty is out of
    // sixteen. Measure each setting over one whole period.
    for requested in [0u64, 1, 5, 11, 15] {
        sim.set(en, bit(false));
        sim.set(duty, word(4, requested));
        cycle(&mut sim, clk, HALF);
        assert_eq!(
            get_u64(&sim, duty_active),
            requested,
            "disabling loads the duty for the next period"
        );
        sim.set(en, bit(true));
        sim.run_for(HALF);

        let mut high_cycles = 0u64;
        let mut ticks = 0u64;
        for _ in 0..16 {
            if high(&sim, pwm_out) {
                high_cycles += 1;
            }
            if high(&sim, period_tick) {
                ticks += 1;
            }
            cycle(&mut sim, clk, HALF);
        }
        assert_eq!(
            high_cycles, requested,
            "duty {requested}/16 should be high for {requested} of 16 cycles"
        );
        assert_eq!(ticks, 1, "one period tick per period");
    }
}

#[test]
fn timer_fires_on_the_period_its_prescaler_and_reload_set() {
    let design = design_of("timer", "timer", &[("WIDTH", "8"), ("PRESCALE_WIDTH", "4")]);
    let mut sim = simulate(&design, "timer");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let en = top_net(&sim, "en");
    let load = top_net(&sim, "load");
    let prescale = top_net(&sim, "prescale");
    let reload = top_net(&sim, "reload");
    let value = top_net(&sim, "value");
    let irq = top_net(&sim, "irq");
    let irq_pending = top_net(&sim, "irq_pending");
    let irq_clear = top_net(&sim, "irq_clear");

    sim.set(en, bit(false));
    sim.set(load, bit(false));
    sim.set(irq_clear, bit(false));
    // Divide by three, count four: a period of twelve clocks.
    sim.set(prescale, word(4, 2));
    sim.set(reload, word(8, 3));
    reset(&mut sim, clk, rst_n);
    cycle(&mut sim, clk, HALF);
    assert_eq!(
        get_u64(&sim, value),
        3,
        "disabled, the timer sits at reload"
    );

    sim.set(en, bit(true));
    let mut gaps: Vec<u64> = Vec::new();
    let mut since = 0u64;
    let mut seen = 0u32;
    for _ in 0..200 {
        cycle(&mut sim, clk, HALF);
        since += 1;
        if high(&sim, irq) {
            seen += 1;
            if seen > 1 {
                gaps.push(since);
            }
            since = 0;
        }
    }
    assert!(seen >= 3, "the timer should have fired several times");
    assert!(
        gaps.iter().all(|g| *g == 12),
        "(reload + 1) * (prescale + 1) = 12 clocks, got {gaps:?}"
    );

    // The sticky flag stays until it is cleared, and only then.
    assert!(high(&sim, irq_pending), "the interrupt latches");
    cycle(&mut sim, clk, HALF);
    assert!(high(&sim, irq_pending), "and stays latched");
    sim.set(irq_clear, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(irq_clear, bit(false));
    assert!(!high(&sim, irq_pending), "until it is cleared");
}

/// The handles of an AXI4-Lite subordinate port, so a Rust testbench can
/// be the manager.
struct Axil {
    clk: NetHandle,
    awaddr: NetHandle,
    awvalid: NetHandle,
    awready: NetHandle,
    wdata: NetHandle,
    wstrb: NetHandle,
    wvalid: NetHandle,
    wready: NetHandle,
    bvalid: NetHandle,
    bready: NetHandle,
    araddr: NetHandle,
    arvalid: NetHandle,
    arready: NetHandle,
    rdata: NetHandle,
    rvalid: NetHandle,
    rready: NetHandle,
}

impl Axil {
    fn new(sim: &Simulator<'_>) -> Axil {
        Axil {
            clk: top_net(sim, "s_axi_aclk"),
            awaddr: top_net(sim, "s_axi_awaddr"),
            awvalid: top_net(sim, "s_axi_awvalid"),
            awready: top_net(sim, "s_axi_awready"),
            wdata: top_net(sim, "s_axi_wdata"),
            wstrb: top_net(sim, "s_axi_wstrb"),
            wvalid: top_net(sim, "s_axi_wvalid"),
            wready: top_net(sim, "s_axi_wready"),
            bvalid: top_net(sim, "s_axi_bvalid"),
            bready: top_net(sim, "s_axi_bready"),
            araddr: top_net(sim, "s_axi_araddr"),
            arvalid: top_net(sim, "s_axi_arvalid"),
            arready: top_net(sim, "s_axi_arready"),
            rdata: top_net(sim, "s_axi_rdata"),
            rvalid: top_net(sim, "s_axi_rvalid"),
            rready: top_net(sim, "s_axi_rready"),
        }
    }

    /// One AXI4-Lite write: address and data offered together, each
    /// retired on its own handshake, then the response.
    fn write(&self, sim: &mut Simulator<'_>, addr: u64, data: u64) {
        sim.set(self.awaddr, word(32, addr));
        sim.set(self.awvalid, bit(true));
        sim.set(self.wdata, word(32, data));
        sim.set(self.wstrb, word(4, 0xF));
        sim.set(self.wvalid, bit(true));
        sim.set(self.bready, bit(true));
        let mut answered = false;
        for _ in 0..50 {
            let aw = high(sim, self.awvalid) && high(sim, self.awready);
            let w = high(sim, self.wvalid) && high(sim, self.wready);
            cycle(sim, self.clk, HALF);
            if aw {
                sim.set(self.awvalid, bit(false));
            }
            if w {
                sim.set(self.wvalid, bit(false));
            }
            if high(sim, self.bvalid) {
                cycle(sim, self.clk, HALF);
                answered = true;
                break;
            }
        }
        sim.set(self.awvalid, bit(false));
        sim.set(self.wvalid, bit(false));
        sim.set(self.bready, bit(false));
        assert!(answered, "the write to {addr:#x} never got a response");
    }

    /// One AXI4-Lite read.
    fn read(&self, sim: &mut Simulator<'_>, addr: u64) -> u64 {
        sim.set(self.araddr, word(32, addr));
        sim.set(self.arvalid, bit(true));
        sim.set(self.rready, bit(true));
        for _ in 0..50 {
            let ar = high(sim, self.arvalid) && high(sim, self.arready);
            cycle(sim, self.clk, HALF);
            if ar {
                sim.set(self.arvalid, bit(false));
            }
            if high(sim, self.rvalid) {
                let value = get_u64(sim, self.rdata);
                cycle(sim, self.clk, HALF);
                sim.set(self.rready, bit(false));
                return value;
            }
        }
        panic!("the read from {addr:#x} never answered");
    }
}

#[test]
fn axil_gpio_answers_reads_and_writes() {
    const DATA_OUT: u64 = 0x0;
    const DATA_IN: u64 = 0x4;
    const DIR: u64 = 0x8;
    const DATA_SET: u64 = 0xC;

    let design = design_of("axil_gpio", "axil_gpio", &[("WIDTH", "8")]);
    let mut sim = simulate(&design, "axil_gpio");
    let bus = Axil::new(&sim);
    let rst_n = top_net(&sim, "s_axi_aresetn");
    let gpio_i = top_net(&sim, "gpio_i");
    let gpio_o = top_net(&sim, "gpio_o");
    let gpio_oe = top_net(&sim, "gpio_oe");

    for net in [bus.awvalid, bus.wvalid, bus.bready, bus.arvalid, bus.rready] {
        sim.set(net, bit(false));
    }
    sim.set(gpio_i, word(8, 0));
    reset(&mut sim, bus.clk, rst_n);

    // Out of reset every pin is an input and nothing is driven.
    assert_eq!(bus.read(&mut sim, DIR), 0);
    assert_eq!(bus.read(&mut sim, DATA_OUT), 0);
    assert_eq!(get_u64(&sim, gpio_oe), 0);

    // Make the low nibble outputs and drive a pattern.
    bus.write(&mut sim, DIR, 0x0F);
    assert_eq!(
        bus.read(&mut sim, DIR),
        0x0F,
        "the direction register reads back"
    );
    assert_eq!(get_u64(&sim, gpio_oe), 0x0F, "and reaches the pins");

    bus.write(&mut sim, DATA_OUT, 0xA5);
    assert_eq!(
        get_u64(&sim, gpio_o),
        0xA5,
        "the output register reaches the pins"
    );
    assert_eq!(bus.read(&mut sim, DATA_OUT), 0xA5, "and reads back");

    // The input register is what the pins say, once synchronised.
    sim.set(gpio_i, word(8, 0x5A));
    for _ in 0..4 {
        cycle(&mut sim, bus.clk, HALF);
    }
    assert_eq!(bus.read(&mut sim, DATA_IN), 0x5A, "the pins read back");
    assert_eq!(
        get_u64(&sim, gpio_o),
        0xA5,
        "reading the pins does not disturb the output register"
    );

    // The set register ORs bits in without a read-modify-write.
    bus.write(&mut sim, DATA_SET, 0x42);
    assert_eq!(get_u64(&sim, gpio_o), 0xE7, "0xA5 | 0x42");
    assert_eq!(
        bus.read(&mut sim, DATA_SET),
        0xE7,
        "and reads as the output"
    );

    // Bits above WIDTH are dropped rather than stored.
    bus.write(&mut sim, DATA_OUT, 0xFFFF_FF00);
    assert_eq!(get_u64(&sim, gpio_o), 0, "only WIDTH bits are kept");
    assert_eq!(bus.read(&mut sim, DATA_OUT), 0);
}

/// One cycle of two clocks driven together, for a dual-port RAM used
/// synchronously.
fn cycle_both(sim: &mut Simulator<'_>, a: NetHandle, b: NetHandle, half: u64) {
    sim.run_for(half);
    sim.set(a, bit(true));
    sim.set(b, bit(true));
    sim.run_for(half);
    sim.set(a, bit(false));
    sim.set(b, bit(false));
}

#[test]
fn ram_sdp_reads_back_what_it_wrote() {
    for out_reg in ["0", "1"] {
        let design = design_of(
            "ram_wrapper",
            "ram_sdp",
            &[("WIDTH", "8"), ("DEPTH", "16"), ("OUT_REG", out_reg)],
        );
        let mut sim = simulate(&design, "ram_sdp");
        let wr_clk = top_net(&sim, "wr_clk");
        let wr_en = top_net(&sim, "wr_en");
        let wr_addr = top_net(&sim, "wr_addr");
        let wr_data = top_net(&sim, "wr_data");
        let rd_clk = top_net(&sim, "rd_clk");
        let rd_en = top_net(&sim, "rd_en");
        let rd_addr = top_net(&sim, "rd_addr");
        let rd_data = top_net(&sim, "rd_data");

        sim.set(wr_clk, bit(false));
        sim.set(rd_clk, bit(false));
        sim.set(wr_en, bit(false));
        sim.set(rd_en, bit(false));

        for address in 0..16u64 {
            sim.set(wr_en, bit(true));
            sim.set(wr_addr, word(4, address));
            sim.set(wr_data, word(8, 0xC0 ^ (address * 9)));
            cycle_both(&mut sim, wr_clk, rd_clk, HALF);
        }
        sim.set(wr_en, bit(false));

        for address in [3u64, 0, 15, 7] {
            sim.set(rd_en, bit(true));
            sim.set(rd_addr, word(4, address));
            cycle_both(&mut sim, wr_clk, rd_clk, HALF);
            if out_reg == "1" {
                cycle_both(&mut sim, wr_clk, rd_clk, HALF);
            }
            assert_eq!(
                get_u64(&sim, rd_data),
                0xC0 ^ (address * 9),
                "OUT_REG={out_reg}, address {address}"
            );
        }
    }
}

#[test]
fn ram_sp_is_read_first_on_one_port() {
    let design = design_of(
        "ram_wrapper",
        "ram_sp",
        &[("WIDTH", "8"), ("DEPTH", "16"), ("OUT_REG", "0")],
    );
    let mut sim = simulate(&design, "ram_sp");
    let clk = top_net(&sim, "clk");
    let en = top_net(&sim, "en");
    let we = top_net(&sim, "we");
    let addr = top_net(&sim, "addr");
    let din = top_net(&sim, "din");
    let dout = top_net(&sim, "dout");

    sim.set(clk, bit(false));
    sim.set(en, bit(true));
    sim.set(we, bit(false));

    for address in 0..16u64 {
        sim.set(we, bit(true));
        sim.set(addr, word(4, address));
        sim.set(din, word(8, 0x31 + address));
        cycle(&mut sim, clk, HALF);
    }
    sim.set(we, bit(false));

    for address in [5u64, 11, 0, 15] {
        sim.set(addr, word(4, address));
        cycle(&mut sim, clk, HALF);
        assert_eq!(get_u64(&sim, dout), 0x31 + address, "address {address}");
    }

    // Writing an address shows what was there before it, which is the
    // read-first behaviour every family can build.
    sim.set(addr, word(4, 5));
    sim.set(din, word(8, 0xEE));
    sim.set(we, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(we, bit(false));
    assert_eq!(
        get_u64(&sim, dout),
        0x31 + 5,
        "the write shows the old word"
    );
    cycle(&mut sim, clk, HALF);
    assert_eq!(get_u64(&sim, dout), 0xEE, "and the new one afterwards");

    // `en` low holds the output rather than reading.
    sim.set(en, bit(false));
    sim.set(addr, word(4, 11));
    cycle(&mut sim, clk, HALF);
    cycle(&mut sim, clk, HALF);
    assert_eq!(
        get_u64(&sim, dout),
        0xEE,
        "a disabled port holds its output"
    );
}

// ---------------------------------------------------------------------------
// rv32i: an assembler, a memory, and programs the core has to get right
// ---------------------------------------------------------------------------

/// A minimal RV32I assembler.
///
/// Every function returns the thirty-two bits of one instruction, so a
/// test program is a `Vec<u32>` that reads like the assembly it is. The
/// encodings are written out from the base ISA's field layout rather than
/// taken from a table, which is the point: a test that assembled its
/// programs with the same decoder the core uses would prove nothing.
mod asm {
    use super::twos;

    /// A CSR number in the twelve-bit immediate field an I-type
    /// instruction carries it in.
    fn csr_number(csr: u32) -> i32 {
        i32::try_from(csr).expect("a twelve-bit CSR number")
    }

    fn r(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, op: u32) -> u32 {
        (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | op
    }

    fn i(imm: i32, rs1: u32, funct3: u32, rd: u32, op: u32) -> u32 {
        ((twos(imm) & 0xFFF) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | op
    }

    fn s(imm: i32, rs2: u32, rs1: u32, funct3: u32, op: u32) -> u32 {
        let v = twos(imm);
        (((v >> 5) & 0x7F) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (funct3 << 12)
            | ((v & 0x1F) << 7)
            | op
    }

    fn b(imm: i32, rs2: u32, rs1: u32, funct3: u32, op: u32) -> u32 {
        let v = twos(imm);
        (((v >> 12) & 1) << 31)
            | (((v >> 5) & 0x3F) << 25)
            | (rs2 << 20)
            | (rs1 << 15)
            | (funct3 << 12)
            | (((v >> 1) & 0xF) << 8)
            | (((v >> 11) & 1) << 7)
            | op
    }

    fn u(imm: u32, rd: u32, op: u32) -> u32 {
        (imm & 0xFFFF_F000) | (rd << 7) | op
    }

    fn j(imm: i32, rd: u32, op: u32) -> u32 {
        let v = twos(imm);
        (((v >> 20) & 1) << 31)
            | (((v >> 1) & 0x3FF) << 21)
            | (((v >> 11) & 1) << 20)
            | (((v >> 12) & 0xFF) << 12)
            | (rd << 7)
            | op
    }

    pub(crate) fn lui(rd: u32, imm: u32) -> u32 {
        u(imm, rd, 0x37)
    }
    pub(crate) fn auipc(rd: u32, imm: u32) -> u32 {
        u(imm, rd, 0x17)
    }
    pub(crate) fn jal(rd: u32, off: i32) -> u32 {
        j(off, rd, 0x6F)
    }
    pub(crate) fn jalr(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0, rd, 0x67)
    }

    pub(crate) fn beq(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b000, 0x63)
    }
    pub(crate) fn bne(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b001, 0x63)
    }
    pub(crate) fn blt(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b100, 0x63)
    }
    pub(crate) fn bge(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b101, 0x63)
    }
    pub(crate) fn bltu(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b110, 0x63)
    }
    pub(crate) fn bgeu(rs1: u32, rs2: u32, off: i32) -> u32 {
        b(off, rs2, rs1, 0b111, 0x63)
    }

    pub(crate) fn lb(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0b000, rd, 0x03)
    }
    pub(crate) fn lh(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0b001, rd, 0x03)
    }
    pub(crate) fn lw(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0b010, rd, 0x03)
    }
    pub(crate) fn lbu(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0b100, rd, 0x03)
    }
    pub(crate) fn lhu(rd: u32, rs1: u32, off: i32) -> u32 {
        i(off, rs1, 0b101, rd, 0x03)
    }

    pub(crate) fn sb(rs2: u32, rs1: u32, off: i32) -> u32 {
        s(off, rs2, rs1, 0b000, 0x23)
    }
    pub(crate) fn sh(rs2: u32, rs1: u32, off: i32) -> u32 {
        s(off, rs2, rs1, 0b001, 0x23)
    }
    pub(crate) fn sw(rs2: u32, rs1: u32, off: i32) -> u32 {
        s(off, rs2, rs1, 0b010, 0x23)
    }

    pub(crate) fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b000, rd, 0x13)
    }
    pub(crate) fn slti(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b010, rd, 0x13)
    }
    pub(crate) fn sltiu(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b011, rd, 0x13)
    }
    pub(crate) fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b100, rd, 0x13)
    }
    pub(crate) fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b110, rd, 0x13)
    }
    pub(crate) fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
        i(imm, rs1, 0b111, rd, 0x13)
    }
    pub(crate) fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
        r(0b0000000, sh, rs1, 0b001, rd, 0x13)
    }
    pub(crate) fn srli(rd: u32, rs1: u32, sh: u32) -> u32 {
        r(0b0000000, sh, rs1, 0b101, rd, 0x13)
    }
    pub(crate) fn srai(rd: u32, rs1: u32, sh: u32) -> u32 {
        r(0b0100000, sh, rs1, 0b101, rd, 0x13)
    }

    pub(crate) fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b000, rd, 0x33)
    }
    pub(crate) fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0100000, rs2, rs1, 0b000, rd, 0x33)
    }
    pub(crate) fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b001, rd, 0x33)
    }
    pub(crate) fn slt(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b010, rd, 0x33)
    }
    pub(crate) fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b011, rd, 0x33)
    }
    pub(crate) fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b100, rd, 0x33)
    }
    pub(crate) fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b101, rd, 0x33)
    }
    pub(crate) fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0100000, rs2, rs1, 0b101, rd, 0x33)
    }
    pub(crate) fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b110, rd, 0x33)
    }
    pub(crate) fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
        r(0b0000000, rs2, rs1, 0b111, rd, 0x33)
    }

    pub(crate) fn fence() -> u32 {
        i(0x0FF, 0, 0b000, 0, 0x0F)
    }
    pub(crate) fn ecall() -> u32 {
        i(0x000, 0, 0b000, 0, 0x73)
    }
    pub(crate) fn ebreak() -> u32 {
        i(0x001, 0, 0b000, 0, 0x73)
    }
    pub(crate) fn mret() -> u32 {
        i(0x302, 0, 0b000, 0, 0x73)
    }
    pub(crate) fn wfi() -> u32 {
        i(0x105, 0, 0b000, 0, 0x73)
    }

    pub(crate) fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(csr_number(csr), rs1, 0b001, rd, 0x73)
    }
    pub(crate) fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(csr_number(csr), rs1, 0b010, rd, 0x73)
    }
    pub(crate) fn csrrc(rd: u32, csr: u32, rs1: u32) -> u32 {
        i(csr_number(csr), rs1, 0b011, rd, 0x73)
    }
    pub(crate) fn csrrwi(rd: u32, csr: u32, imm: u32) -> u32 {
        i(csr_number(csr), imm, 0b101, rd, 0x73)
    }
    pub(crate) fn csrrsi(rd: u32, csr: u32, imm: u32) -> u32 {
        i(csr_number(csr), imm, 0b110, rd, 0x73)
    }

    /// A word that decodes to nothing: opcode 0x0B is one of the four
    /// slots the base ISA leaves for a custom extension, so no future
    /// version of this core can accidentally make it legal.
    pub(crate) fn illegal() -> u32 {
        0x0000_000B
    }
}

/// CSR numbers the tests name.
const CSR_MSTATUS: u32 = 0x300;
const CSR_MIE: u32 = 0x304;
const CSR_MTVEC: u32 = 0x305;
const CSR_MEPC: u32 = 0x341;
const CSR_MCAUSE: u32 = 0x342;
const CSR_MIP: u32 = 0x344;
const CSR_MCYCLE: u32 = 0xB00;
const CSR_MINSTRET: u32 = 0xB02;
const CSR_MHARTID: u32 = 0xF14;

/// Words of memory behind both of the core's ports. The programs below
/// keep their code under `DATA_BASE` and their data above it.
const MEM_WORDS: usize = 1024;
const DATA_BASE: u32 = 0x400;

/// The core with a memory on each of its ports, driven a cycle at a time
/// from Rust.
///
/// One `Cpu` is one running simulation. `step` answers whatever the core
/// asked for in this cycle and then takes the clock edge, which is the
/// whole of the memory model: a word array that answers after `stalls`
/// wait states, writes through `dmem_be`, and reads zero — an illegal
/// instruction — outside the program.
struct Cpu<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    rst_n: NetHandle,
    imem_addr: NetHandle,
    imem_req: NetHandle,
    imem_ready: NetHandle,
    imem_rdata: NetHandle,
    dmem_addr: NetHandle,
    dmem_req: NetHandle,
    dmem_we: NetHandle,
    dmem_be: NetHandle,
    dmem_wdata: NetHandle,
    dmem_ready: NetHandle,
    dmem_rdata: NetHandle,
    irq_timer: NetHandle,
    irq_software: NetHandle,
    irq_external: NetHandle,
    dbg_pc: NetHandle,
    dbg_retire: NetHandle,
    dbg_trap: NetHandle,
    regs: MemHandle,
    mem: Vec<u32>,
    stalls: u32,
    i_wait: u32,
    d_wait: u32,
    /// Instructions retired since reset.
    retired: u64,
    /// Traps entered since reset.
    traps: u64,
    /// `dbg_pc` at the last trap.
    trap_pc: u32,
}

/// A net's value, or zero while it is still undriven. The memory model
/// reads the request lines every cycle, including the ones before reset
/// is released, where a port is legitimately `x`.
fn loose_u64(sim: &Simulator<'_>, handle: NetHandle) -> u64 {
    sim.get(handle).to_u64().unwrap_or(0)
}

impl<'d> Cpu<'d> {
    fn new(design: &'d Design, program: &[u32], stalls: u32) -> Cpu<'d> {
        let sim = simulate(design, "rv32i");
        let regs = sim
            .memory(&format!("{}.regs", sim.top_name()))
            .expect("the register file is a memory");
        let mut mem = vec![0u32; MEM_WORDS];
        mem[..program.len()].copy_from_slice(program);
        let mut cpu = Cpu {
            clk: top_net(&sim, "clk"),
            rst_n: top_net(&sim, "rst_n"),
            imem_addr: top_net(&sim, "imem_addr"),
            imem_req: top_net(&sim, "imem_req"),
            imem_ready: top_net(&sim, "imem_ready"),
            imem_rdata: top_net(&sim, "imem_rdata"),
            dmem_addr: top_net(&sim, "dmem_addr"),
            dmem_req: top_net(&sim, "dmem_req"),
            dmem_we: top_net(&sim, "dmem_we"),
            dmem_be: top_net(&sim, "dmem_be"),
            dmem_wdata: top_net(&sim, "dmem_wdata"),
            dmem_ready: top_net(&sim, "dmem_ready"),
            dmem_rdata: top_net(&sim, "dmem_rdata"),
            irq_timer: top_net(&sim, "irq_timer"),
            irq_software: top_net(&sim, "irq_software"),
            irq_external: top_net(&sim, "irq_external"),
            dbg_pc: top_net(&sim, "dbg_pc"),
            dbg_retire: top_net(&sim, "dbg_retire"),
            dbg_trap: top_net(&sim, "dbg_trap"),
            sim,
            regs,
            mem,
            stalls,
            i_wait: stalls,
            d_wait: stalls,
            retired: 0,
            traps: 0,
            trap_pc: 0,
        };
        cpu.start();
        cpu
    }

    fn start(&mut self) {
        for net in [
            self.imem_ready,
            self.dmem_ready,
            self.irq_timer,
            self.irq_software,
            self.irq_external,
        ] {
            self.sim.set(net, bit(false));
        }
        self.sim.set(self.imem_rdata, word(32, 0));
        self.sim.set(self.dmem_rdata, word(32, 0));
        let clk = self.clk;
        let rst_n = self.rst_n;
        reset(&mut self.sim, clk, rst_n);
        // The ISA leaves x1..x31 undefined after reset and the core does
        // not clear them, so the testbench does — the same service a boot
        // ROM or a debugger performs on a real machine.
        for index in 0..32 {
            self.sim.set_mem(self.regs, index, word(32, 0));
        }
    }

    /// The architectural value of `x{index}`.
    fn reg(&self, index: u64) -> u32 {
        if index == 0 {
            return 0;
        }
        self.sim
            .get_mem(self.regs, index)
            .and_then(|v| v.to_u64())
            .map_or_else(|| panic!("x{index} holds x"), narrow)
    }

    /// The word at a byte address.
    fn word_at(&self, addr: u32) -> u32 {
        self.mem[(addr >> 2) as usize]
    }

    /// Answers both ports and takes one clock edge.
    fn step(&mut self) {
        let ia = narrow(loose_u64(&self.sim, self.imem_addr)) >> 2;
        let iw = self.mem.get(ia as usize).copied().unwrap_or(0);
        self.sim.set(self.imem_rdata, word(32, u64::from(iw)));
        let asked = high(&self.sim, self.imem_req);
        let i_ok = Cpu::handshake(asked, &mut self.i_wait, self.stalls);
        self.sim.set(self.imem_ready, bit(i_ok));

        let da = narrow(loose_u64(&self.sim, self.dmem_addr));
        let index = (da >> 2) as usize;
        let dw = self.mem.get(index).copied().unwrap_or(0);
        self.sim.set(self.dmem_rdata, word(32, u64::from(dw)));
        let asked = high(&self.sim, self.dmem_req);
        let d_ok = Cpu::handshake(asked, &mut self.d_wait, self.stalls);
        self.sim.set(self.dmem_ready, bit(d_ok));
        if d_ok && high(&self.sim, self.dmem_we) {
            let be = narrow(loose_u64(&self.sim, self.dmem_be));
            let value = narrow(loose_u64(&self.sim, self.dmem_wdata));
            let mut held = dw;
            for lane in 0..4 {
                if (be >> lane) & 1 == 1 {
                    let mask = 0xFFu32 << (lane * 8);
                    held = (held & !mask) | (value & mask);
                }
            }
            if index < self.mem.len() {
                self.mem[index] = held;
            }
        }

        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);

        if high(&self.sim, self.dbg_retire) {
            self.retired += 1;
        }
        if high(&self.sim, self.dbg_trap) {
            self.traps += 1;
            self.trap_pc = get_u32(&self.sim, self.dbg_pc);
        }
    }

    /// One port's `ready`, `stalls` cycles after the request appears.
    fn handshake(asked: bool, wait: &mut u32, stalls: u32) -> bool {
        if !asked {
            *wait = stalls;
            return false;
        }
        if *wait == 0 {
            *wait = stalls;
            true
        } else {
            *wait -= 1;
            false
        }
    }

    fn run(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.step();
        }
    }

    /// Runs until one more instruction retires or a trap is entered.
    fn next(&mut self) {
        let retired = self.retired;
        let traps = self.traps;
        for _ in 0..64 {
            self.step();
            if self.retired != retired || self.traps != traps {
                return;
            }
        }
        panic!("nothing retired and nothing trapped in 64 cycles");
    }

    /// Runs `count` instructions.
    fn run_instructions(&mut self, count: usize) {
        for _ in 0..count {
            self.next();
        }
    }

    /// Runs until the instruction at `addr` retires.
    fn run_to(&mut self, addr: u32, limit: u32) {
        for _ in 0..limit {
            self.step();
            if high(&self.sim, self.dbg_retire) && get_u32(&self.sim, self.dbg_pc) == addr {
                return;
            }
        }
        panic!("the instruction at {addr:#x} never retired within {limit} cycles");
    }
}

/// A core with the register file of the caller's choosing.
fn rv32i_design(bram: &str) -> Design {
    design_of("rv32i", "rv32i", &[("REGFILE_BRAM", bram)])
}

/// The shape every conditional branch encoder has, so a table of them
/// can be written down.
type Branch = fn(u32, u32, i32) -> u32;

/// Both register-file flavours, so every program below runs on each.
const REGFILES: [&str; 2] = ["0", "1"];

#[test]
fn rv32i_builds_constants_and_pc_relative_addresses() {
    for bram in REGFILES {
        let program = vec![
            asm::lui(1, 0xABCD_E000),   // 0x00
            asm::addi(2, 1, 0x123),     // 0x04
            asm::auipc(3, 0x0000_1000), // 0x08
            asm::addi(4, 0, -1),        // 0x0C
            asm::lui(5, 0x8000_0000),   // 0x10
            asm::addi(6, 5, 1),         // 0x14
            asm::addi(0, 4, 7),         // 0x18: a write to x0 is dropped
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);

        cpu.next();
        assert_eq!(cpu.reg(1), 0xABCD_E000, "REGFILE_BRAM={bram}: LUI");
        cpu.next();
        assert_eq!(cpu.reg(2), 0xABCD_E123, "ADDI with a signed immediate");
        cpu.next();
        assert_eq!(cpu.reg(3), 0x0000_1008, "AUIPC adds to its own address");
        cpu.next();
        assert_eq!(cpu.reg(4), 0xFFFF_FFFF, "ADDI sign extends the immediate");
        cpu.next();
        assert_eq!(cpu.reg(5), 0x8000_0000, "LUI reaches the top bit");
        cpu.next();
        assert_eq!(cpu.reg(6), 0x8000_0001, "and the value is a whole word");
        cpu.next();
        assert_eq!(cpu.reg(0), 0, "x0 stays zero however it is written");
        assert_eq!(cpu.retired, 7, "seven instructions");
        assert_eq!(cpu.traps, 0, "and no traps");
    }
}

#[test]
fn rv32i_computes_every_register_immediate_operation() {
    for bram in REGFILES {
        let program = vec![
            asm::lui(1, 0xF0F0_F000),
            asm::addi(1, 1, 0x0F0), // x1 = 0xF0F0F0F0
            asm::addi(2, 0, -1),    // x2 = 0xFFFFFFFF
            asm::slli(3, 1, 4),
            asm::srli(4, 1, 4),
            asm::srai(5, 1, 4),
            asm::srai(6, 2, 31),
            asm::xori(7, 1, 0x0FF),
            asm::ori(8, 0, 0x7FF),
            asm::andi(9, 1, -1),
            asm::slti(10, 2, 1),
            asm::sltiu(11, 2, 1),
            asm::sltiu(12, 0, -1),
            asm::slti(13, 1, 0),
            asm::addi(14, 2, 1),
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(program.len());
        assert_eq!(cpu.traps, 0, "REGFILE_BRAM={bram}");

        assert_eq!(cpu.reg(1), 0xF0F0_F0F0);
        assert_eq!(cpu.reg(3), 0x0F0F_0F00, "SLLI");
        assert_eq!(cpu.reg(4), 0x0F0F_0F0F, "SRLI shifts zeros in");
        assert_eq!(cpu.reg(5), 0xFF0F_0F0F, "SRAI shifts the sign in");
        assert_eq!(cpu.reg(6), 0xFFFF_FFFF, "SRAI of -1 by 31");
        assert_eq!(cpu.reg(7), 0xF0F0_F00F, "XORI");
        assert_eq!(cpu.reg(8), 0x0000_07FF, "ORI");
        assert_eq!(cpu.reg(9), 0xF0F0_F0F0, "ANDI with -1 is a copy");
        assert_eq!(cpu.reg(10), 1, "SLTI is signed");
        assert_eq!(cpu.reg(11), 0, "SLTIU compares the same bits unsigned");
        assert_eq!(cpu.reg(12), 1, "SLTIU sign extends the immediate first");
        assert_eq!(cpu.reg(13), 1, "SLTI sees the top bit as a sign");
        assert_eq!(cpu.reg(14), 0, "ADDI wraps");
    }
}

#[test]
fn rv32i_computes_every_register_register_operation() {
    for bram in REGFILES {
        let program = vec![
            asm::lui(1, 0xF0F0_F000),
            asm::addi(1, 1, 0x0F0), // x1 = 0xF0F0F0F0
            asm::addi(2, 0, -1),    // x2 = 0xFFFFFFFF
            asm::addi(3, 0, 4),
            asm::addi(4, 0, 7),
            asm::add(5, 1, 3),
            asm::sub(6, 3, 4),
            asm::sll(7, 1, 3),
            asm::srl(8, 1, 3),
            asm::sra(9, 1, 3),
            asm::xor(10, 1, 2),
            asm::or(11, 3, 4),
            asm::and(12, 3, 4),
            asm::slt(13, 2, 3),
            asm::sltu(14, 2, 3),
            asm::slt(15, 3, 2),
            asm::sltu(16, 3, 2),
            asm::addi(17, 0, 33),
            asm::sll(18, 3, 17),
            asm::sub(19, 3, 3),
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(program.len());
        assert_eq!(cpu.traps, 0, "REGFILE_BRAM={bram}");

        assert_eq!(cpu.reg(5), 0xF0F0_F0F4, "ADD");
        assert_eq!(cpu.reg(6), 0xFFFF_FFFD, "SUB");
        assert_eq!(cpu.reg(7), 0x0F0F_0F00, "SLL");
        assert_eq!(cpu.reg(8), 0x0F0F_0F0F, "SRL");
        assert_eq!(cpu.reg(9), 0xFF0F_0F0F, "SRA");
        assert_eq!(cpu.reg(10), 0x0F0F_0F0F, "XOR");
        assert_eq!(cpu.reg(11), 7, "OR");
        assert_eq!(cpu.reg(12), 4, "AND");
        assert_eq!(cpu.reg(13), 1, "SLT");
        assert_eq!(cpu.reg(14), 0, "SLTU");
        assert_eq!(cpu.reg(15), 0, "SLT the other way round");
        assert_eq!(cpu.reg(16), 1, "SLTU the other way round");
        assert_eq!(cpu.reg(18), 8, "a shift takes only rs2[4:0]");
        assert_eq!(cpu.reg(19), 0, "SUB of a register from itself");
    }
}

#[test]
fn rv32i_takes_and_declines_every_branch() {
    // Each branch is given a pair of values that makes it jump and then a
    // pair that makes it fall through, and what happened is counted: a
    // taken branch skips the `addi` that adds one to x20. Counting rather
    // than checking one landing catches a branch that jumped to the right
    // place for the wrong reason.
    for bram in REGFILES {
        let mut program: Vec<u32> = vec![
            asm::addi(1, 0, 5),
            asm::addi(2, 0, 5),
            asm::addi(3, 0, -1),
            asm::addi(4, 0, 1),
            asm::addi(20, 0, 0),
        ];
        let cases: [(Branch, u32, u32, bool); 12] = [
            (asm::beq, 1, 2, true),
            (asm::beq, 1, 4, false),
            (asm::bne, 1, 4, true),
            (asm::bne, 1, 2, false),
            (asm::blt, 3, 4, true),   // -1 < 1
            (asm::blt, 4, 3, false),  // 1 < -1 is false
            (asm::bge, 4, 3, true),   // 1 >= -1
            (asm::bge, 3, 4, false),  // -1 >= 1 is false
            (asm::bltu, 4, 3, true),  // 1 < 0xFFFFFFFF unsigned
            (asm::bltu, 3, 4, false), // and not the other way round
            (asm::bgeu, 3, 4, true),  // 0xFFFFFFFF >= 1 unsigned
            (asm::bgeu, 4, 3, false),
        ];
        let mut taken = 0u32;
        for (make, a, b, jumps) in cases {
            program.push(make(a, b, 8)); // over the `addi` that follows
            program.push(asm::addi(20, 20, 1));
            if jumps {
                taken += 1;
            }
        }
        let fell_through = u32::try_from(cases.len()).unwrap() - taken;

        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(5 + cases.len() + fell_through as usize);
        assert_eq!(cpu.traps, 0, "REGFILE_BRAM={bram}");
        assert_eq!(
            cpu.reg(20),
            fell_through,
            "REGFILE_BRAM={bram}: {fell_through} of the twelve should not jump"
        );
    }
}

#[test]
fn rv32i_jumps_and_links() {
    for bram in REGFILES {
        //   0x00  jal  x1, +0x10      -> x1 = 0x04, pc = 0x10
        //   0x04  addi x5, x0, 0x5A   (never runs)
        //   0x08  addi x6, x0, 2      (the JALR target)
        //   0x0C  jal  x0, +0x0C      -> pc = 0x18, nothing linked
        //   0x10  jalr x2, x1, +4     -> x2 = 0x14, pc = 0x08
        //   0x14  addi x5, x0, 0x5A   (never runs)
        //   0x18  addi x7, x0, 3
        let program = vec![
            asm::jal(1, 0x10),
            asm::addi(5, 0, 0x5A),
            asm::addi(6, 0, 2),
            asm::jal(0, 0x0C),
            asm::jalr(2, 1, 4),
            asm::addi(5, 0, 0x5A),
            asm::addi(7, 0, 3),
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);

        cpu.next();
        assert_eq!(cpu.reg(1), 0x04, "JAL links the address after itself");
        cpu.next();
        assert_eq!(cpu.reg(2), 0x14, "JALR links the address after itself");
        cpu.next();
        assert_eq!(cpu.reg(6), 2, "JALR jumped to rs1 + imm");
        cpu.next(); // the jal at 0x0C
        cpu.next();
        assert_eq!(cpu.reg(7), 3, "JAL with rd = x0 jumped without linking");
        assert_eq!(cpu.reg(5), 0, "neither skipped instruction ran");
        assert_eq!(cpu.traps, 0);

        // The low bit of a JALR target is cleared rather than faulting.
        let program = vec![
            asm::addi(1, 0, 9), // an odd address
            asm::jalr(0, 1, 0), // jumps to 8, not 9
            asm::addi(3, 0, 7),
        ];
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(3);
        assert_eq!(cpu.traps, 0, "JALR clears bit 0 instead of faulting");
        assert_eq!(cpu.reg(3), 7, "and lands on the aligned word below");
    }
}

#[test]
fn rv32i_loads_and_stores_every_width() {
    for bram in REGFILES {
        let base = DATA_BASE as i32;
        let program = vec![
            asm::lui(1, 0x89AB_C000),
            asm::addi(1, 1, 0x0DE), // x1 = 0x89ABC0DE
            asm::addi(2, 0, 0),     // the base register
            asm::sw(1, 2, base),
            asm::lw(3, 2, base),
            asm::lb(4, 2, base),
            asm::lbu(5, 2, base),
            asm::lb(6, 2, base + 3),
            asm::lbu(7, 2, base + 2),
            asm::lh(8, 2, base),
            asm::lhu(9, 2, base),
            asm::lh(10, 2, base + 2),
            asm::lhu(11, 2, base + 2),
            // A sub-word store lands in the lane the address names and
            // leaves the rest of the word alone.
            asm::addi(12, 0, 0x55),
            asm::sb(12, 2, base + 1),
            asm::lw(13, 2, base),
            asm::lui(14, 0x0000_1000),
            asm::addi(14, 14, 0x234), // x14 = 0x1234
            asm::sh(14, 2, base + 2),
            asm::lw(15, 2, base),
            // A negative offset reaches back down.
            asm::addi(16, 0, base + 8),
            asm::sw(1, 16, -8),
            asm::lw(17, 2, base),
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(program.len());
        assert_eq!(cpu.traps, 0, "REGFILE_BRAM={bram}");

        assert_eq!(cpu.reg(3), 0x89AB_C0DE, "SW then LW");
        assert_eq!(cpu.reg(4), 0xFFFF_FFDE, "LB sign extends");
        assert_eq!(cpu.reg(5), 0x0000_00DE, "LBU does not");
        assert_eq!(cpu.reg(6), 0xFFFF_FF89, "LB of the top byte");
        assert_eq!(cpu.reg(7), 0x0000_00AB, "LBU of byte 2");
        assert_eq!(cpu.reg(8), 0xFFFF_C0DE, "LH sign extends");
        assert_eq!(cpu.reg(9), 0x0000_C0DE, "LHU does not");
        assert_eq!(cpu.reg(10), 0xFFFF_89AB, "LH of the top half");
        assert_eq!(cpu.reg(11), 0x0000_89AB, "LHU of the top half");
        assert_eq!(cpu.reg(13), 0x89AB_55DE, "SB writes one lane");
        assert_eq!(cpu.reg(15), 0x1234_55DE, "SH writes two");
        assert_eq!(cpu.reg(17), 0x89AB_C0DE, "a negative offset reaches back");
        assert_eq!(
            cpu.word_at(DATA_BASE),
            0x89AB_C0DE,
            "and the memory itself agrees"
        );
    }
}

#[test]
fn rv32i_survives_memories_that_make_it_wait() {
    // The same program with wait states on every access: the handshake is
    // the only thing that changes, so the results must not.
    for stalls in [0u32, 1, 3] {
        let base = DATA_BASE as i32;
        let program = vec![
            asm::addi(1, 0, 0x2A),
            asm::addi(2, 0, 0),
            asm::sw(1, 2, base),
            asm::lw(3, 2, base),
            asm::add(4, 3, 3),
        ];
        let design = rv32i_design("0");
        let mut cpu = Cpu::new(&design, &program, stalls);
        cpu.run_instructions(program.len());
        assert_eq!(cpu.reg(3), 0x2A, "{stalls} wait states: the load");
        assert_eq!(cpu.reg(4), 0x54, "{stalls} wait states: and what used it");
        assert_eq!(cpu.traps, 0);
    }
}

#[test]
fn rv32i_traps_on_a_misaligned_access_and_on_nonsense() {
    let base = DATA_BASE as i32;
    let design = rv32i_design("0");

    // The handler adds up the causes it saw and returns past the
    // instruction that faulted, so one program covers five of them.
    let mut program = vec![0u32; 0x120 / 4];
    let text = [
        asm::addi(2, 0, 0),          // 0x00
        asm::addi(3, 0, 0x100),      // 0x04
        asm::csrrw(0, CSR_MTVEC, 3), // 0x08
        asm::lw(4, 2, base + 1),     // 0x0C  cause 4
        asm::sh(4, 2, base + 1),     // 0x10  cause 6
        asm::illegal(),              // 0x14  cause 2
        asm::ebreak(),               // 0x18  cause 3
        asm::ecall(),                // 0x1C  cause 11
        asm::addi(9, 0, 0x5A),       // 0x20
        asm::jal(0, 0),              // 0x24
    ];
    program[..text.len()].copy_from_slice(&text);
    let handler = [
        asm::csrrs(5, CSR_MCAUSE, 0),
        asm::add(6, 6, 5),
        asm::csrrs(7, CSR_MEPC, 0),
        asm::addi(7, 7, 4),
        asm::csrrw(0, CSR_MEPC, 7),
        asm::mret(),
    ];
    program[0x100 / 4..0x100 / 4 + handler.len()].copy_from_slice(&handler);

    let mut cpu = Cpu::new(&design, &program, 0);
    cpu.run_to(0x24, 2000);
    assert_eq!(cpu.traps, 5, "five faults, five trap entries");
    assert_eq!(cpu.trap_pc, 0x1C, "the last one was the ECALL");
    assert_eq!(
        cpu.reg(6),
        4 + 6 + 2 + 3 + 11,
        "load misaligned, store misaligned, illegal, breakpoint, ecall"
    );
    assert_eq!(cpu.reg(9), 0x5A, "and the program carried on afterwards");
    assert_eq!(cpu.reg(4), 0, "the misaligned load wrote nothing");

    // A jump to an address that is not a multiple of four is cause 0,
    // reported against the jump rather than against the target.
    let mut program = vec![0u32; 0x120 / 4];
    program[0] = asm::addi(3, 0, 0x100);
    program[1] = asm::csrrw(0, CSR_MTVEC, 3);
    program[2] = asm::addi(1, 0, 2);
    program[3] = asm::jalr(0, 1, 0);
    program[0x100 / 4] = asm::csrrs(5, CSR_MCAUSE, 0);
    program[0x104 / 4] = asm::csrrs(6, CSR_MEPC, 0);
    program[0x108 / 4] = asm::jal(0, 0);

    let mut cpu = Cpu::new(&design, &program, 0);
    cpu.run_to(0x108, 2000);
    assert_eq!(cpu.reg(5), 0, "cause 0: instruction address misaligned");
    assert_eq!(cpu.reg(6), 0x0C, "blamed on the jump, not on the target");
}

#[test]
fn rv32i_reads_and_writes_its_machine_csrs() {
    let design = rv32i_design("0");
    let program = vec![
        asm::addi(1, 0, 0x100),
        asm::csrrw(0, CSR_MTVEC, 1),
        asm::csrrs(2, CSR_MTVEC, 0),
        asm::addi(3, 0, 0x102), // the low two bits are dropped
        asm::csrrw(4, CSR_MTVEC, 3),
        asm::csrrs(5, CSR_MTVEC, 0),
        asm::csrrwi(0, CSR_MSTATUS, 8), // set MIE
        asm::csrrs(6, CSR_MSTATUS, 0),
        asm::csrrc(0, CSR_MSTATUS, 1), // x1 is 0x100: nothing in MIE
        asm::csrrs(7, CSR_MSTATUS, 0),
        asm::addi(8, 0, 8),
        asm::csrrc(0, CSR_MSTATUS, 8), // clear MIE
        asm::csrrs(9, CSR_MSTATUS, 0),
        asm::csrrs(10, CSR_MHARTID, 0),
        asm::csrrs(11, CSR_MIP, 0),
        asm::csrrsi(12, CSR_MIE, 8),
        asm::csrrs(13, CSR_MIE, 0),
        asm::fence(),
        asm::wfi(),
        asm::jal(0, 0),
    ];
    let mut cpu = Cpu::new(&design, &program, 0);
    let halt = 4 * (u32::try_from(program.len()).unwrap() - 1);
    cpu.run_to(halt, 400);
    assert_eq!(cpu.traps, 0, "none of this should have faulted");

    assert_eq!(cpu.reg(2), 0x100, "mtvec reads back");
    assert_eq!(cpu.reg(4), 0x100, "CSRRW returns the old value");
    assert_eq!(
        cpu.reg(5),
        0x100,
        "and mtvec keeps its low two bits at zero"
    );
    assert_eq!(cpu.reg(6), 0x1808, "MIE set, MPP reading 2'b11");
    assert_eq!(cpu.reg(7), 0x1808, "clearing bits that were not set");
    assert_eq!(cpu.reg(9), 0x1800, "MIE cleared again");
    assert_eq!(cpu.reg(10), 0, "mhartid is zero");
    assert_eq!(cpu.reg(11), 0, "mip with nothing asserted");
    assert_eq!(cpu.reg(12), 0, "mie started at zero");
    assert_eq!(cpu.reg(13), 8, "and MSIE stuck");

    // The counters run, at the rates the core's timing says they should.
    let program = vec![
        asm::csrrs(1, CSR_MCYCLE, 0),
        asm::csrrs(2, CSR_MINSTRET, 0),
        asm::addi(0, 0, 0),
        asm::addi(0, 0, 0),
        asm::addi(0, 0, 0),
        asm::csrrs(3, CSR_MCYCLE, 0),
        asm::csrrs(4, CSR_MINSTRET, 0),
        asm::jal(0, 0),
    ];
    let mut cpu = Cpu::new(&design, &program, 0);
    cpu.run_to(4 * 7, 200);
    assert_eq!(
        cpu.reg(4) - cpu.reg(2),
        5,
        "five instructions retired between the two reads of minstret"
    );
    assert_eq!(
        cpu.reg(3) - cpu.reg(1),
        10,
        "and with a memory that never waits each of them took two cycles"
    );

    // An unknown CSR, and a write to one whose number says it is read
    // only, are both illegal instructions.
    for encoding in [asm::csrrs(1, 0x3FF, 0), asm::csrrw(0, CSR_MHARTID, 1)] {
        let mut program = vec![0u32; 0x120 / 4];
        program[0] = asm::addi(3, 0, 0x100);
        program[1] = asm::csrrw(0, CSR_MTVEC, 3);
        program[2] = asm::addi(1, 0, 1);
        program[3] = encoding;
        program[4] = asm::jal(0, 0);
        program[0x100 / 4] = asm::csrrs(2, CSR_MCAUSE, 0);
        program[0x104 / 4] = asm::jal(0, 0);

        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_to(0x104, 400);
        assert_eq!(cpu.traps, 1, "one illegal instruction");
        assert_eq!(cpu.reg(2), 2, "cause 2");
    }
}

#[test]
fn rv32i_takes_a_timer_interrupt_and_returns_from_it() {
    let design = rv32i_design("0");
    //   0x00  addi x3, x0, 0x100
    //   0x04  csrrw x0, mtvec, x3
    //   0x08  addi x4, x0, 0x80      (MTIE)
    //   0x0C  csrrw x0, mie, x4
    //   0x10  csrrwi x0, mstatus, 8  (MIE)
    //   0x14  addi x1, x1, 1         <- the loop the interrupt lands in
    //   0x18  jal x0, -4
    //   0x100 addi x20, x20, 1
    //   0x104 csrrs x21, mcause, x0
    //   0x108 csrrs x22, mepc, x0
    //   0x10C csrrwi x0, mie, 0      (stop asking)
    //   0x110 mret
    let mut program = vec![0u32; 0x120 / 4];
    let text = [
        asm::addi(3, 0, 0x100),
        asm::csrrw(0, CSR_MTVEC, 3),
        asm::addi(4, 0, 0x80),
        asm::csrrw(0, CSR_MIE, 4),
        asm::csrrwi(0, CSR_MSTATUS, 8),
        asm::addi(1, 1, 1),
        asm::jal(0, -4),
    ];
    program[..text.len()].copy_from_slice(&text);
    let handler = [
        asm::addi(20, 20, 1),
        asm::csrrs(21, CSR_MCAUSE, 0),
        asm::csrrs(22, CSR_MEPC, 0),
        asm::csrrwi(0, CSR_MIE, 0),
        asm::mret(),
    ];
    program[0x100 / 4..0x100 / 4 + handler.len()].copy_from_slice(&handler);

    let mut cpu = Cpu::new(&design, &program, 0);
    cpu.run_instructions(5);
    assert_eq!(cpu.traps, 0, "nothing is asserted yet");
    cpu.run(20);
    assert_eq!(cpu.traps, 0, "and an unasserted line does not fire");
    let spun = cpu.reg(1);
    assert!(spun > 0, "the loop should have gone round");

    let timer = cpu.irq_timer;
    cpu.sim.set(timer, bit(true));
    cpu.run_to(0x110, 400);
    assert_eq!(cpu.traps, 1, "exactly one interrupt was taken");
    assert_eq!(cpu.reg(20), 1, "the handler ran once");
    assert_eq!(cpu.reg(21), 0x8000_0007, "cause: machine timer interrupt");
    let resumed = cpu.reg(22);
    assert!(
        resumed == 0x14 || resumed == 0x18,
        "mepc is an instruction of the loop, got {resumed:#x}"
    );

    // MRET put MIE back, but the handler turned MTIE off, so the line —
    // which is still asserted — does not fire again.
    cpu.run(60);
    assert_eq!(cpu.traps, 1, "and it does not fire twice");
    assert!(cpu.reg(1) > spun, "the interrupted loop carried on");

    // The other two lines have their own bits and their own causes.
    for (line, shift, cause) in [(0u32, 11u32, 0x8000_000Bu32), (1, 3, 0x8000_0003)] {
        let mut program = vec![0u32; 0x120 / 4];
        let text = [
            asm::addi(3, 0, 0x100),
            asm::csrrw(0, CSR_MTVEC, 3),
            asm::addi(4, 0, 1),
            asm::slli(4, 4, shift),
            asm::csrrw(0, CSR_MIE, 4),
            asm::csrrwi(0, CSR_MSTATUS, 8),
            asm::jal(0, 0),
        ];
        program[..text.len()].copy_from_slice(&text);
        program[0x100 / 4] = asm::csrrs(21, CSR_MCAUSE, 0);
        program[0x104 / 4] = asm::jal(0, 0);

        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_instructions(6);
        let net = if line == 0 {
            cpu.irq_external
        } else {
            cpu.irq_software
        };
        cpu.sim.set(net, bit(true));
        cpu.run_to(0x100, 200);
        assert_eq!(cpu.reg(21), cause, "the cause of interrupt line {line}");
    }
}

#[test]
fn rv32i_sums_an_array_in_a_loop() {
    let values: [u32; 8] = [3, 1, 4, 1, 5, 9, 2, 6];
    let total: u32 = values.iter().sum();

    for bram in REGFILES {
        //       addi x1, x0, DATA_BASE
        //       addi x2, x0, 8
        //       addi x3, x0, 0
        // loop: lw   x4, 0(x1)
        //       add  x3, x3, x4
        //       addi x1, x1, 4
        //       addi x2, x2, -1
        //       bne  x2, x0, loop
        //       sw   x3, 0(x1)
        //       jal  x0, 0
        let program = vec![
            asm::addi(1, 0, DATA_BASE as i32),
            asm::addi(2, 0, i32::try_from(values.len()).unwrap()),
            asm::addi(3, 0, 0),
            asm::lw(4, 1, 0),
            asm::add(3, 3, 4),
            asm::addi(1, 1, 4),
            asm::addi(2, 2, -1),
            asm::bne(2, 0, -16),
            asm::sw(3, 1, 0),
            asm::jal(0, 0),
        ];
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        for (i, v) in values.iter().enumerate() {
            cpu.mem[(DATA_BASE as usize >> 2) + i] = *v;
        }
        cpu.run_to(4 * 9, 4000);
        assert_eq!(cpu.reg(3), total, "REGFILE_BRAM={bram}: the sum");
        assert_eq!(cpu.reg(2), 0, "the counter ran out");
        assert_eq!(
            cpu.reg(1),
            DATA_BASE + 4 * u32::try_from(values.len()).unwrap(),
            "the pointer walked the whole array"
        );
        assert_eq!(
            cpu.word_at(DATA_BASE + 4 * u32::try_from(values.len()).unwrap()),
            total,
            "and the answer was stored past the end"
        );
        assert_eq!(cpu.traps, 0);
    }
}

#[test]
fn rv32i_runs_a_recursive_function_on_the_stack() {
    // Fibonacci by the definition: two nested calls, a frame per call and
    // a saved return address, so the whole calling convention is proved
    // at once rather than one instruction at a time. It needs no
    // multiply, which matters — this core has no M extension.
    //
    //  0x00  addi x2, x0, 0x7F0     ; sp
    //  0x04  addi x10, x0, N
    //  0x08  jal  x1, fib
    //  0x0C  jal  x0, halt
    //  0x10  addi x2, x2, -12       ; fib:
    //  0x14  sw   x1, 8(x2)
    //  0x18  sw   x10, 4(x2)
    //  0x1C  addi x5, x0, 2
    //  0x20  blt  x10, x5, base
    //  0x24  addi x10, x10, -1
    //  0x28  jal  x1, fib
    //  0x2C  sw   x10, 0(x2)        ; fib(n-1)
    //  0x30  lw   x10, 4(x2)
    //  0x34  addi x10, x10, -2
    //  0x38  jal  x1, fib           ; fib(n-2)
    //  0x3C  lw   x6, 0(x2)
    //  0x40  add  x10, x10, x6
    //  0x44  jal  x0, base
    //  0x48  lw   x1, 8(x2)         ; base:
    //  0x4C  addi x2, x2, 12
    //  0x50  jalr x0, x1, 0
    //  0x54  jal  x0, halt          ; halt:
    const N: u32 = 10;
    let fib_at = 0x10i32;
    let base_at = 0x48i32;
    let halt_at = 0x54u32;
    let program = vec![
        asm::addi(2, 0, 0x7F0),
        asm::addi(10, 0, N as i32),
        asm::jal(1, fib_at - 0x08),
        asm::jal(0, halt_at as i32 - 0x0C),
        asm::addi(2, 2, -12),
        asm::sw(1, 2, 8),
        asm::sw(10, 2, 4),
        asm::addi(5, 0, 2),
        asm::blt(10, 5, base_at - 0x20),
        asm::addi(10, 10, -1),
        asm::jal(1, fib_at - 0x28),
        asm::sw(10, 2, 0),
        asm::lw(10, 2, 4),
        asm::addi(10, 10, -2),
        asm::jal(1, fib_at - 0x38),
        asm::lw(6, 2, 0),
        asm::add(10, 10, 6),
        asm::jal(0, base_at - 0x44),
        asm::lw(1, 2, 8),
        asm::addi(2, 2, 12),
        asm::jalr(0, 1, 0),
        asm::jal(0, 0),
    ];

    fn fib(n: u32) -> u32 {
        if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
    }

    for bram in REGFILES {
        let design = rv32i_design(bram);
        let mut cpu = Cpu::new(&design, &program, 0);
        cpu.run_to(halt_at, 60_000);
        assert_eq!(
            cpu.reg(10),
            fib(N),
            "REGFILE_BRAM={bram}: fib({N}) computed recursively"
        );
        assert_eq!(cpu.reg(2), 0x7F0, "every frame was popped again");
        assert_eq!(cpu.traps, 0, "and nothing faulted on the way");
        assert!(
            cpu.retired > 1000,
            "the whole recursion ran: {} instructions",
            cpu.retired
        );
    }
}

// ---------------------------------------------------------------------------
// eth_mac_rmii
// ---------------------------------------------------------------------------

/// The Ethernet frame check sequence, written out here rather than taken
/// from the block: a testbench that borrowed the implementation's own
/// arithmetic would agree with it however wrong both were.
fn eth_crc(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// The four octets of check sequence a frame is transmitted with.
fn eth_fcs(bytes: &[u8]) -> [u8; 4] {
    (!eth_crc(bytes)).to_le_bytes()
}

/// The pins of the MAC, so a testbench can be both the PHY and the user.
struct Mac<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    rst_n: NetHandle,
    tx_en: NetHandle,
    txd: NetHandle,
    crs_dv: NetHandle,
    rxd: NetHandle,
    rx_er: NetHandle,
    tx_data: NetHandle,
    tx_valid: NetHandle,
    tx_ready: NetHandle,
    tx_last: NetHandle,
    tx_underrun: NetHandle,
    rx_data: NetHandle,
    rx_valid: NetHandle,
    rx_last: NetHandle,
    rx_crc_ok: NetHandle,
    rx_error: NetHandle,
}

impl<'d> Mac<'d> {
    fn new(design: &'d Design) -> Mac<'d> {
        let sim = simulate(design, "eth_mac_rmii");
        let mut mac = Mac {
            clk: top_net(&sim, "ref_clk"),
            rst_n: top_net(&sim, "rst_n"),
            tx_en: top_net(&sim, "tx_en"),
            txd: top_net(&sim, "txd"),
            crs_dv: top_net(&sim, "crs_dv"),
            rxd: top_net(&sim, "rxd"),
            rx_er: top_net(&sim, "rx_er"),
            tx_data: top_net(&sim, "tx_data"),
            tx_valid: top_net(&sim, "tx_valid"),
            tx_ready: top_net(&sim, "tx_ready"),
            tx_last: top_net(&sim, "tx_last"),
            tx_underrun: top_net(&sim, "tx_underrun"),
            rx_data: top_net(&sim, "rx_data"),
            rx_valid: top_net(&sim, "rx_valid"),
            rx_last: top_net(&sim, "rx_last"),
            rx_crc_ok: top_net(&sim, "rx_crc_ok"),
            rx_error: top_net(&sim, "rx_error"),
            sim,
        };
        for net in [mac.crs_dv, mac.rx_er, mac.tx_valid, mac.tx_last] {
            mac.sim.set(net, bit(false));
        }
        mac.sim.set(mac.rxd, word(2, 0));
        mac.sim.set(mac.tx_data, word(8, 0));
        let clk = mac.clk;
        let rst_n = mac.rst_n;
        reset(&mut mac.sim, clk, rst_n);
        mac
    }

    fn tick(&mut self) {
        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);
    }
}

/// Sends `frame` through the transmitter with the block's own output
/// looped into its receiver, returning what came out and whether the
/// check sequence held. `corrupt_at` flips one dibit that many cycles
/// into the transmission, which is how a damaged frame is made.
fn rmii_loopback(frame: &[u8], corrupt_at: Option<u32>) -> (Vec<u8>, bool, bool) {
    let design = design_of("eth_mac_rmii", "eth_mac_rmii", &[("IFG_CYCLES", "12")]);
    let mut mac = Mac::new(&design);

    mac.sim.set(mac.tx_data, word(8, u64::from(frame[0])));
    mac.sim.set(mac.tx_valid, bit(true));
    mac.sim.set(mac.tx_last, bit(frame.len() == 1));

    let mut sent = 0usize;
    let mut got: Vec<u8> = Vec::new();
    let mut last_seen = false;
    let mut crc_ok = false;
    let mut error = false;
    let mut driven = 0u32;

    for _ in 0..4000 {
        // The wire: what the transmitter drives is what the receiver
        // sees, in the same cycle, with one dibit optionally damaged.
        let enabled = high(&mac.sim, mac.tx_en);
        let mut dibit = get_u64(&mac.sim, mac.txd);
        if enabled {
            if corrupt_at == Some(driven) {
                dibit ^= 1;
            }
            driven += 1;
        }
        mac.sim.set(mac.crs_dv, bit(enabled));
        mac.sim.set(mac.rxd, word(2, dibit));

        let taken = high(&mac.sim, mac.tx_valid) && high(&mac.sim, mac.tx_ready);
        mac.tick();

        if taken {
            sent += 1;
            if sent < frame.len() {
                mac.sim.set(mac.tx_data, word(8, u64::from(frame[sent])));
                mac.sim.set(mac.tx_last, bit(sent == frame.len() - 1));
            } else {
                mac.sim.set(mac.tx_valid, bit(false));
                mac.sim.set(mac.tx_last, bit(false));
            }
        }
        if high(&mac.sim, mac.rx_error) {
            error = true;
        }
        if high(&mac.sim, mac.rx_valid) {
            got.push(octet(get_u64(&mac.sim, mac.rx_data)));
            if high(&mac.sim, mac.rx_last) {
                last_seen = true;
                crc_ok = high(&mac.sim, mac.rx_crc_ok);
                break;
            }
        }
    }

    assert_eq!(sent, frame.len(), "the transmitter took every octet");
    assert!(last_seen, "no end of frame arrived");
    assert!(!error, "the frame should be well formed, whatever its CRC");
    assert!(
        !high(&mac.sim, mac.tx_underrun),
        "the testbench never let the transmitter starve"
    );
    (got, crc_ok, error)
}

#[test]
fn eth_mac_rmii_loops_a_frame_from_its_transmitter_into_its_receiver() {
    // Long enough that the five-octet hold-back pipeline fills and empties
    // several times, short enough to stay quick.
    let frame: Vec<u8> = (0..24u8)
        .map(|i| i.wrapping_mul(37).wrapping_add(9))
        .collect();
    let (got, crc_ok, _) = rmii_loopback(&frame, None);
    assert_eq!(got, frame, "every octet, in order");
    assert!(crc_ok, "and the check sequence held");

    // The shortest frame the hold-back pipeline can deliver at all is one
    // octet of data behind four of check sequence.
    let (got, crc_ok, _) = rmii_loopback(&[0xA5], None);
    assert_eq!(got, vec![0xA5], "a one-octet frame still comes back");
    assert!(crc_ok);
}

#[test]
fn eth_mac_rmii_rejects_a_frame_with_a_damaged_octet() {
    let frame: Vec<u8> = (0..24u8)
        .map(|i| i.wrapping_mul(37).wrapping_add(9))
        .collect();
    // Dibit 48 is well past the preamble and the delimiter, which take
    // thirty-two, so this lands in the payload.
    let (got, crc_ok, _) = rmii_loopback(&frame, Some(48));
    assert_eq!(got.len(), frame.len(), "the octets still arrive");
    assert_ne!(got, frame, "one of them is not what was sent");
    assert!(
        !crc_ok,
        "and the frame check sequence is what says the frame is bad"
    );
}

#[test]
fn eth_mac_rmii_puts_a_standard_frame_on_the_wire() {
    // Watch the pins and rebuild what a PHY would see: preamble,
    // delimiter, payload, check sequence, then the gap.
    let design = design_of("eth_mac_rmii", "eth_mac_rmii", &[("IFG_CYCLES", "48")]);
    let mut mac = Mac::new(&design);
    let frame: Vec<u8> = vec![
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x00, 0x5E, 0x11, 0x22, 0x33,
    ];

    mac.sim.set(mac.tx_data, word(8, u64::from(frame[0])));
    mac.sim.set(mac.tx_valid, bit(true));
    mac.sim.set(mac.tx_last, bit(false));

    let mut sent = 0usize;
    let mut dibits: Vec<u8> = Vec::new();
    let mut gap = 0u32;
    let mut frames = 0u32;
    let mut was_on = false;

    for _ in 0..3000 {
        let on = high(&mac.sim, mac.tx_en);
        if on {
            dibits.push(octet(get_u64(&mac.sim, mac.txd)));
        } else if was_on {
            frames += 1;
        } else if frames == 1 {
            gap += 1;
        }
        was_on = on;

        let taken = high(&mac.sim, mac.tx_valid) && high(&mac.sim, mac.tx_ready);
        mac.tick();
        if taken {
            sent += 1;
            if sent < frame.len() {
                mac.sim.set(mac.tx_data, word(8, u64::from(frame[sent])));
                mac.sim.set(mac.tx_last, bit(sent == frame.len() - 1));
            } else {
                mac.sim.set(mac.tx_valid, bit(false));
                mac.sim.set(mac.tx_last, bit(false));
            }
        }
        if frames == 1 && gap > 60 {
            break;
        }
    }

    assert_eq!(sent, frame.len(), "the whole frame went in");
    assert_eq!(frames, 1, "and exactly one frame came out");
    assert_eq!(dibits.len() % 4, 0, "a whole number of octets on the wire");

    // Bits go out least significant first, two at a time.
    let octets: Vec<u8> = dibits
        .chunks(4)
        .map(|c| c[0] | (c[1] << 2) | (c[2] << 4) | (c[3] << 6))
        .collect();
    let mut expected: Vec<u8> = vec![0x55; 7];
    expected.push(0xD5);
    expected.extend_from_slice(&frame);
    expected.extend_from_slice(&eth_fcs(&frame));
    assert_eq!(octets, expected, "preamble, delimiter, payload, FCS");

    assert!(
        gap >= 48,
        "the inter-frame gap should be at least IFG_CYCLES, got {gap}"
    );
}

#[test]
fn eth_mac_rmii_reports_a_frame_that_ends_in_the_middle_of_an_octet() {
    // Drive the receive pins by hand: a preamble, a delimiter, two and a
    // half octets, and then the carrier goes away.
    let design = design_of("eth_mac_rmii", "eth_mac_rmii", &[("IFG_CYCLES", "12")]);
    let mut mac = Mac::new(&design);

    let mut dibits: Vec<u8> = Vec::new();
    for byte in [0x55u8, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0xD5, 0x12, 0x34] {
        for i in 0..4 {
            dibits.push((byte >> (2 * i)) & 3);
        }
    }
    dibits.push(1); // half an octet, and then nothing
    dibits.push(2);

    let mut errors = 0u32;
    let mut valids = 0u32;
    for dibit in dibits {
        mac.sim.set(mac.crs_dv, bit(true));
        mac.sim.set(mac.rxd, word(2, u64::from(dibit)));
        mac.tick();
        if high(&mac.sim, mac.rx_valid) {
            valids += 1;
        }
    }
    mac.sim.set(mac.crs_dv, bit(false));
    mac.sim.set(mac.rxd, word(2, 0));
    for _ in 0..8 {
        mac.tick();
        if high(&mac.sim, mac.rx_error) {
            errors += 1;
        }
        if high(&mac.sim, mac.rx_valid) {
            valids += 1;
        }
    }

    assert_eq!(errors, 1, "one error pulse for the malformed frame");
    assert_eq!(valids, 0, "and nothing was delivered from it");
}

// ---------------------------------------------------------------------------
// spiflash_xip
// ---------------------------------------------------------------------------

/// The block with a serial flash model on the other end of the four
/// wires, in the style of the `spi_master` tests: the model samples
/// `mosi` where a real device does, on the rising edge, and presents the
/// next bit of `miso` on the falling one.
struct Xip<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    rst_n: NetHandle,
    mem_addr: NetHandle,
    mem_req: NetHandle,
    mem_ready: NetHandle,
    mem_rdata: NetHandle,
    cfg_addr: NetHandle,
    cfg_we: NetHandle,
    cfg_wdata: NetHandle,
    cfg_rdata: NetHandle,
    busy: NetHandle,
    sclk: NetHandle,
    cs_n: NetHandle,
    mosi: NetHandle,
    miso: NetHandle,
    /// The contents of the device.
    flash: Vec<u8>,
    /// Dummy cycles the model expects between address and data.
    dummy: usize,
    prev_sclk: bool,
    prev_cs: bool,
    shift_in: u64,
    count: usize,
    miso_bit: bool,
    /// The last command and address the model decoded.
    last_cmd: u8,
    last_addr: u32,
    /// Cycles `cs_n` was low before the first `sclk` rise of a frame.
    setup: u32,
    /// Cycles `cs_n` stayed low after the last `sclk` fall.
    hold: u32,
    saw_edge: bool,
}

impl<'d> Xip<'d> {
    fn new(design: &'d Design, flash: Vec<u8>) -> Xip<'d> {
        let sim = simulate(design, "spiflash_xip");
        let mut xip = Xip {
            clk: top_net(&sim, "clk"),
            rst_n: top_net(&sim, "rst_n"),
            mem_addr: top_net(&sim, "mem_addr"),
            mem_req: top_net(&sim, "mem_req"),
            mem_ready: top_net(&sim, "mem_ready"),
            mem_rdata: top_net(&sim, "mem_rdata"),
            cfg_addr: top_net(&sim, "cfg_addr"),
            cfg_we: top_net(&sim, "cfg_we"),
            cfg_wdata: top_net(&sim, "cfg_wdata"),
            cfg_rdata: top_net(&sim, "cfg_rdata"),
            busy: top_net(&sim, "busy"),
            sclk: top_net(&sim, "sclk"),
            cs_n: top_net(&sim, "cs_n"),
            mosi: top_net(&sim, "mosi"),
            miso: top_net(&sim, "miso"),
            sim,
            flash,
            dummy: 0,
            prev_sclk: false,
            prev_cs: true,
            shift_in: 0,
            count: 0,
            miso_bit: false,
            last_cmd: 0,
            last_addr: 0,
            setup: 0,
            hold: 0,
            saw_edge: false,
        };
        for net in [xip.mem_req, xip.cfg_we, xip.miso] {
            xip.sim.set(net, bit(false));
        }
        xip.sim.set(xip.mem_addr, word(32, 0));
        xip.sim.set(xip.cfg_addr, word(2, 0));
        xip.sim.set(xip.cfg_wdata, word(32, 0));
        let clk = xip.clk;
        let rst_n = xip.rst_n;
        reset(&mut xip.sim, clk, rst_n);
        xip
    }

    /// One clock cycle with the flash model on the wires.
    fn tick(&mut self) {
        self.sim.set(self.miso, bit(self.miso_bit));
        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);

        let cs = high(&self.sim, self.cs_n);
        let sclk = high(&self.sim, self.sclk);
        if self.prev_cs && !cs {
            self.count = 0;
            self.shift_in = 0;
            self.setup = 0;
            self.saw_edge = false;
        }
        if !cs {
            if !self.saw_edge && !sclk {
                self.setup += 1;
            }
            if sclk && !self.prev_sclk {
                self.saw_edge = true;
                let bit_in = u64::from(high(&self.sim, self.mosi));
                self.shift_in = (self.shift_in << 1) | bit_in;
                self.count += 1;
                if self.count == 32 {
                    self.last_cmd = octet(self.shift_in >> 24);
                    self.last_addr = narrow(self.shift_in & 0x00FF_FFFF);
                }
            }
            if !sclk && self.prev_sclk {
                self.hold = 0;
                if self.count >= 32 + self.dummy {
                    let index = self.count - 32 - self.dummy;
                    let addr = (self.last_addr as usize + index / 8) % self.flash.len();
                    let byte = self.flash[addr];
                    self.miso_bit = (byte >> (7 - index % 8)) & 1 == 1;
                }
            }
            if !sclk && !self.prev_sclk && self.saw_edge {
                self.hold += 1;
            }
        }
        self.prev_cs = cs;
        self.prev_sclk = sclk;
    }

    /// One read through the memory port.
    fn read(&mut self, addr: u32) -> u32 {
        self.sim.set(self.mem_addr, word(32, u64::from(addr)));
        self.sim.set(self.mem_req, bit(true));
        for _ in 0..4000 {
            self.tick();
            if high(&self.sim, self.mem_ready) {
                let value = get_u32(&self.sim, self.mem_rdata);
                self.sim.set(self.mem_req, bit(false));
                self.tick();
                return value;
            }
        }
        panic!("the read from {addr:#x} never answered");
    }

    /// One configuration write.
    fn configure(&mut self, addr: u64, value: u64) {
        assert!(!high(&self.sim, self.busy), "configured while busy");
        self.sim.set(self.cfg_addr, word(2, addr));
        self.sim.set(self.cfg_wdata, word(32, value));
        self.sim.set(self.cfg_we, bit(true));
        self.tick();
        self.sim.set(self.cfg_we, bit(false));
        self.tick();
    }

    /// What the configuration register at `addr` reads back as.
    fn config(&mut self, addr: u64) -> u32 {
        self.sim.set(self.cfg_addr, word(2, addr));
        self.sim.run_for(HALF);
        get_u32(&self.sim, self.cfg_rdata)
    }
}

/// A flash image with no two words alike.
fn flash_image() -> Vec<u8> {
    (0..256u32)
        .map(|i| (i.wrapping_mul(97).wrapping_add(11) & 0xFF) as u8)
        .collect()
}

/// The word at `addr`, assembled little-endian the way the block does.
fn flash_word(image: &[u8], addr: u32) -> u32 {
    let base = (addr & !3) as usize;
    u32::from_le_bytes([
        image[base],
        image[base + 1],
        image[base + 2],
        image[base + 3],
    ])
}

#[test]
fn spiflash_xip_turns_a_word_read_into_a_flash_read_command() {
    let image = flash_image();
    let design = design_of(
        "spiflash_xip",
        "spiflash_xip",
        &[
            ("CLK_DIV", "2"),
            ("READ_CMD", "8'h03"),
            ("DUMMY_CYCLES", "0"),
        ],
    );
    let mut xip = Xip::new(&design, image.clone());

    for addr in [0x00u32, 0x04, 0x40, 0xFC, 0x06] {
        let got = xip.read(addr);
        assert_eq!(
            got,
            flash_word(&image, addr),
            "the word at {addr:#x}, little-endian"
        );
        assert_eq!(xip.last_cmd, 0x03, "the command the flash saw");
        assert_eq!(xip.last_addr, addr & !3, "the address it saw, word aligned");
        assert!(
            xip.setup >= 2,
            "cs_n falls a divisor before the first sclk edge, got {}",
            xip.setup
        );
        assert!(xip.hold >= 1, "and stays low past the last one");
        assert!(
            high(&xip.sim, xip.cs_n),
            "cs_n is released between transactions"
        );
        assert!(!high(&xip.sim, xip.sclk), "sclk idles low, as mode 0 says");
    }
}

#[test]
fn spiflash_xip_takes_a_new_command_divisor_and_dummy_count() {
    let image = flash_image();
    let design = design_of(
        "spiflash_xip",
        "spiflash_xip",
        &[
            ("CLK_DIV", "2"),
            ("READ_CMD", "8'h03"),
            ("DUMMY_CYCLES", "0"),
        ],
    );
    let mut xip = Xip::new(&design, image.clone());

    assert_eq!(xip.config(0), 2, "the divisor resets to CLK_DIV");
    assert_eq!(xip.config(1), 0x0003, "and the command to READ_CMD");

    // A fast read: a different command octet and eight dummy cycles, at
    // half the clock rate.
    xip.configure(0, 1);
    xip.configure(1, 0x0800 | 0x0B);
    assert_eq!(xip.config(0), 1);
    assert_eq!(xip.config(1), 0x080B, "command and dummy count together");
    xip.dummy = 8;

    for addr in [0x10u32, 0x2C, 0x80] {
        let got = xip.read(addr);
        assert_eq!(
            got,
            flash_word(&image, addr),
            "at {addr:#x} after the dummy cycles"
        );
        assert_eq!(xip.last_cmd, 0x0B, "the reconfigured command");
        assert_eq!(xip.last_addr, addr & !3);
    }

    // And back again, so nothing is one-way, at the fastest divisor the
    // block has: `sclk` is then the clock divided by two.
    xip.configure(1, 0x03);
    xip.configure(0, 0);
    xip.dummy = 0;
    let got = xip.read(0x20);
    assert_eq!(got, flash_word(&image, 0x20), "at sclk = clk / 2");
    assert_eq!(xip.last_cmd, 0x03);
    assert_eq!(xip.last_addr, 0x20);
}

// ---------------------------------------------------------------------------
// sdram_ctrl
// ---------------------------------------------------------------------------

/// A datasheet's timings, in nanoseconds, and the clock they are read
/// at. The test turns them into cycles itself, with its own rounding,
/// rather than asking the block what it derived.
#[derive(Clone, Copy)]
struct SdramPart {
    mhz: u64,
    cas_latency: u64,
    row_bits: u32,
    col_bits: u32,
    init_refreshes: u64,
    init_us: u64,
    rcd_ns: u64,
    rp_ns: u64,
    ras_ns: u64,
    rc_ns: u64,
    rfc_ns: u64,
    wr_ns: u64,
    rrd_ns: u64,
    refi_ns: u64,
    mrd: u64,
}

/// Micron's MT48LC16M16A2 at speed grade -75, run at 75 MHz: every
/// timing but tRRD rounds up to a cycle count that is not a whole
/// number of nanoseconds, which is where a rounding mistake would show.
const MT48LC16M16A2: SdramPart = SdramPart {
    mhz: 75,
    cas_latency: 2,
    row_bits: 13,
    col_bits: 9,
    init_refreshes: 2,
    init_us: 20,
    rcd_ns: 20,
    rp_ns: 20,
    ras_ns: 44,
    rc_ns: 66,
    rfc_ns: 66,
    wr_ns: 15,
    rrd_ns: 15,
    refi_ns: 7812,
    mrd: 2,
};

/// ISSI's IS42S16400 (8 MiB, twelve row bits and eight column bits) at
/// CAS latency 3, on a 48 MHz clock.
const IS42S16400: SdramPart = SdramPart {
    mhz: 48,
    cas_latency: 3,
    row_bits: 12,
    col_bits: 8,
    init_refreshes: 8,
    init_us: 20,
    rcd_ns: 20,
    rp_ns: 20,
    ras_ns: 45,
    rc_ns: 67,
    rfc_ns: 67,
    wr_ns: 14,
    rrd_ns: 14,
    refi_ns: 15625,
    mrd: 2,
};

impl SdramPart {
    /// Nanoseconds to cycles, rounded up: a timing is a minimum.
    fn cycles(&self, ns: u64) -> u64 {
        (ns * self.mhz).div_ceil(1000)
    }

    /// The refresh interval, rounded down: it is a maximum.
    fn refi(&self) -> u64 {
        self.refi_ns * self.mhz / 1000
    }

    fn params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("CLK_MHZ", self.mhz.to_string()),
            ("CAS_LATENCY", self.cas_latency.to_string()),
            ("ROW_BITS", self.row_bits.to_string()),
            ("COL_BITS", self.col_bits.to_string()),
            ("INIT_REFRESHES", self.init_refreshes.to_string()),
            ("T_INIT_US", self.init_us.to_string()),
            ("T_RCD_NS", self.rcd_ns.to_string()),
            ("T_RP_NS", self.rp_ns.to_string()),
            ("T_RAS_NS", self.ras_ns.to_string()),
            ("T_RC_NS", self.rc_ns.to_string()),
            ("T_RFC_NS", self.rfc_ns.to_string()),
            ("T_WR_NS", self.wr_ns.to_string()),
            ("T_RRD_NS", self.rrd_ns.to_string()),
            ("T_REFI_NS", self.refi_ns.to_string()),
            ("T_MRD", self.mrd.to_string()),
        ]
    }
}

/// A behavioural SDRAM that **enforces** the datasheet rather than just
/// storing data.
///
/// It is clocked by what the part actually receives, `sdram_clk`, which
/// the block drives as `clk` inverted: so the model acts on each falling
/// edge of `clk`, sampling the command pins the block launched on the
/// rising edge before. Every command is checked against the state the
/// part is in and the time since the commands that constrain it, and a
/// violation is *recorded*, not ignored — the test then fails on the
/// list. A READ puts its data on `dq` CAS latency edges later for
/// exactly one cycle and garbage either side of it, so a controller that
/// captures a cycle early or late reads garbage.
struct SdramModel {
    part: SdramPart,
    trcd: u64,
    trp: u64,
    tras: u64,
    trc: u64,
    trfc: u64,
    twr: u64,
    trrd: u64,
    tinit: u64,
    trefi: u64,
    /// Edges of the part's clock since reset was released.
    now: u64,
    /// Sparse contents, by (bank, row, column).
    cells: BTreeMap<(u64, u64, u64), u16>,
    open: [Option<u64>; 4],
    last_act: [Option<u64>; 4],
    last_pre: [Option<u64>; 4],
    last_write: [Option<u64>; 4],
    last_act_any: Option<u64>,
    last_ref: Option<u64>,
    last_mrs: Option<u64>,
    /// Where the current refresh interval started, for the overdue check.
    refresh_window: Option<u64>,
    precharged: bool,
    init_refreshes: u64,
    cas_programmed: Option<u64>,
    /// READs in flight: the edge their data appears, and where from.
    reads: Vec<(u64, u64, u64, u64)>,
    /// DQM as sampled at each edge, for the read mask's latency of two.
    dqm_history: Vec<u64>,
    /// Whether the model drove `dq` for the cycle just started.
    driving: bool,
    violations: Vec<String>,
    /// Commands seen, by name, for the tests that count them.
    log: Vec<(u64, &'static str)>,
}

/// The pins as the part saw them at one edge.
struct SdramPins {
    cke: bool,
    cmd: u64,
    ba: u64,
    a: u64,
    dqm: u64,
    dq: u16,
    dq_oe: bool,
}

impl SdramModel {
    fn new(part: SdramPart) -> SdramModel {
        SdramModel {
            trcd: part.cycles(part.rcd_ns),
            trp: part.cycles(part.rp_ns),
            tras: part.cycles(part.ras_ns),
            trc: part.cycles(part.rc_ns),
            trfc: part.cycles(part.rfc_ns),
            twr: part.cycles(part.wr_ns),
            trrd: part.cycles(part.rrd_ns),
            tinit: part.init_us * part.mhz,
            trefi: part.refi(),
            part,
            now: 0,
            cells: BTreeMap::new(),
            open: [None; 4],
            last_act: [None; 4],
            last_pre: [None; 4],
            last_write: [None; 4],
            last_act_any: None,
            last_ref: None,
            last_mrs: None,
            refresh_window: None,
            precharged: false,
            init_refreshes: 0,
            cas_programmed: None,
            reads: Vec::new(),
            dqm_history: Vec::new(),
            driving: false,
            violations: Vec::new(),
            log: Vec::new(),
        }
    }

    fn violation(&mut self, what: String) {
        // One line per problem is enough to act on; a controller that is
        // wrong once is usually wrong every cycle after.
        if self.violations.len() < 20 {
            self.violations.push(format!("edge {}: {what}", self.now));
        }
    }

    /// `since` edges must have passed since `then`, or it is a violation
    /// named `what`.
    fn spacing(&mut self, then: Option<u64>, since: u64, what: &str) {
        if let Some(then) = then {
            let gap = self.now - then;
            if gap < since {
                self.violation(format!("{what}: {gap} cycle(s), the part needs {since}"));
            }
        }
    }

    fn initialised(&self) -> bool {
        self.precharged
            && self.init_refreshes >= self.part.init_refreshes
            && self.cas_programmed.is_some()
    }

    fn count(&self, what: &str) -> usize {
        self.log.iter().filter(|(_, c)| *c == what).count()
    }

    /// One edge of the part's clock. Returns what the part drives on `dq`
    /// until the next one.
    fn edge(&mut self, pins: &SdramPins) -> u64 {
        self.now += 1;
        self.dqm_history.push(pins.dqm);

        // Refresh overdue: once the part is running, no gap between two
        // AUTO REFRESH commands may exceed tREFI.
        if self.initialised()
            && let Some(start) = self.refresh_window
            && self.now - start > self.trefi
        {
            let late = self.now - start;
            self.violation(format!(
                "refresh overdue: {late} cycles since the last, tREFI is {}",
                self.trefi
            ));
            // Once per lapse rather than on every edge after it.
            self.refresh_window = Some(self.now);
        }

        // The data bus: the controller must not drive while the part does.
        if pins.dq_oe && self.driving {
            self.violation("bus contention: the controller drives dq during read data".into());
        }

        let name = match pins.cmd {
            0b1111 | 0b0111 => None,
            0b0011 => Some("ACTIVE"),
            0b0101 => Some("READ"),
            0b0100 => Some("WRITE"),
            0b0010 => Some("PRECHARGE"),
            0b0001 => Some("REFRESH"),
            0b0000 => Some("MODE"),
            _ => Some("other"),
        };
        if let Some(name) = name {
            self.log.push((self.now, name));
            if !pins.cke {
                self.violation(format!("{name} with CKE low"));
            }
            if self.now <= self.tinit {
                self.violation(format!(
                    "{name} before the {}-cycle power-up wait is over",
                    self.tinit
                ));
            }
            let (refresh, trfc) = (self.last_ref, self.trfc);
            self.spacing(refresh, trfc, &format!("{name} after REFRESH (tRFC)"));
            let (mode, tmrd) = (self.last_mrs, self.part.mrd);
            self.spacing(mode, tmrd, &format!("{name} after MODE (tMRD)"));
        }
        let bank = usize::try_from(pins.ba).expect("two bits");
        let a10 = pins.a & (1 << 10) != 0;
        match name {
            Some("ACTIVE") => {
                if !self.initialised() {
                    self.violation("ACTIVE before the initialisation sequence is complete".into());
                }
                if self.open[bank].is_some() {
                    self.violation(format!("ACTIVE to bank {bank}, which is already open"));
                }
                let (p, trp) = (self.last_pre[bank], self.trp);
                self.spacing(p, trp, "ACTIVE after PRECHARGE (tRP)");
                let (a, trc) = (self.last_act[bank], self.trc);
                self.spacing(a, trc, "ACTIVE after ACTIVE to one bank (tRC)");
                let (any, trrd) = (self.last_act_any, self.trrd);
                self.spacing(any, trrd, "ACTIVE after ACTIVE to another bank (tRRD)");
                if pins.a >> self.part.row_bits != 0 {
                    self.violation(format!("row {:#x} is out of range", pins.a));
                }
                self.open[bank] = Some(pins.a);
                self.last_act[bank] = Some(self.now);
                self.last_act_any = Some(self.now);
            }
            Some(op @ ("READ" | "WRITE")) => {
                let col = pins.a & ((1 << 10) - 1);
                if a10 {
                    self.violation(format!(
                        "{op} with auto-precharge, which this model does not do"
                    ));
                }
                if col >> self.part.col_bits != 0 {
                    self.violation(format!("column {col:#x} is out of range"));
                }
                match self.open[bank] {
                    None => self.violation(format!("{op} to bank {bank}, which has no open row")),
                    Some(row) => {
                        let (a, trcd) = (self.last_act[bank], self.trcd);
                        self.spacing(a, trcd, &format!("{op} after ACTIVE (tRCD)"));
                        let key = (bank as u64, row, col);
                        if op == "WRITE" {
                            if !pins.dq_oe {
                                self.violation("WRITE with the data bus not driven".into());
                            }
                            let old = self.cells.get(&key).copied().unwrap_or(0);
                            let mut new = old;
                            if pins.dqm & 1 == 0 {
                                new = (new & 0xFF00) | (pins.dq & 0x00FF);
                            }
                            if pins.dqm & 2 == 0 {
                                new = (new & 0x00FF) | (pins.dq & 0xFF00);
                            }
                            self.cells.insert(key, new);
                            self.last_write[bank] = Some(self.now);
                        } else {
                            let cl = self.cas_programmed.unwrap_or(self.part.cas_latency);
                            self.reads.push((self.now + cl, key.0, key.1, key.2));
                        }
                    }
                }
            }
            Some("PRECHARGE") => {
                let banks: Vec<usize> = if a10 { (0..4).collect() } else { vec![bank] };
                for b in banks {
                    if self.open[b].is_some() {
                        let (a, tras) = (self.last_act[b], self.tras);
                        self.spacing(a, tras, "PRECHARGE after ACTIVE (tRAS)");
                        let (w, twr) = (self.last_write[b], self.twr);
                        self.spacing(w, twr, "PRECHARGE after WRITE (tWR)");
                    }
                    self.open[b] = None;
                    self.last_pre[b] = Some(self.now);
                }
                if a10 {
                    self.precharged = true;
                }
            }
            Some("REFRESH") => {
                if !self.precharged {
                    self.violation("REFRESH before the first PRECHARGE ALL".into());
                }
                if self.open.iter().any(Option::is_some) {
                    self.violation("REFRESH with a row open".into());
                }
                for b in 0..4 {
                    let (p, trp) = (self.last_pre[b], self.trp);
                    self.spacing(p, trp, "REFRESH after PRECHARGE (tRP)");
                }
                if self.cas_programmed.is_none() {
                    self.init_refreshes += 1;
                }
                self.last_ref = Some(self.now);
                self.refresh_window = Some(self.now);
            }
            Some("MODE") => {
                if !self.precharged || self.open.iter().any(Option::is_some) {
                    self.violation("MODE with the banks not all precharged".into());
                }
                if self.init_refreshes < self.part.init_refreshes {
                    self.violation(format!(
                        "MODE after {} refresh(es), the part asks for {}",
                        self.init_refreshes, self.part.init_refreshes
                    ));
                }
                for b in 0..4 {
                    let (p, trp) = (self.last_pre[b], self.trp);
                    self.spacing(p, trp, "MODE after PRECHARGE (tRP)");
                }
                // Burst length 1, sequential, standard operation.
                let cl = (pins.a >> 4) & 7;
                if pins.a & !0x70 != 0 || pins.ba != 0 {
                    self.violation(format!("mode word {:#x} is not burst length 1", pins.a));
                }
                if cl != self.part.cas_latency {
                    self.violation(format!(
                        "CAS latency {cl} programmed, the test runs {}",
                        self.part.cas_latency
                    ));
                }
                self.cas_programmed = Some(cl);
                self.last_mrs = Some(self.now);
            }
            Some("other") => {
                self.violation(format!("unexpected command {:04b}", pins.cmd));
            }
            _ => {}
        }

        // What the part drives from this edge to the next: the read due
        // now, masked by DQM as it was two edges ago, or garbage that
        // changes every cycle.
        let garbage = (self.now.wrapping_mul(0x9E37) ^ 0x5A5A) & 0xFFFF;
        let due: Vec<(u64, u64, u64, u64)> = self
            .reads
            .iter()
            .copied()
            .filter(|r| r.0 == self.now)
            .collect();
        self.reads.retain(|r| r.0 != self.now);
        self.driving = !due.is_empty();
        match due.first() {
            Some(&(_, b, row, col)) => {
                let value = u64::from(self.cells.get(&(b, row, col)).copied().unwrap_or(0));
                let mask = self
                    .dqm_history
                    .len()
                    .checked_sub(3)
                    .map_or(0, |i| self.dqm_history[i]);
                let mut out = value;
                if mask & 1 != 0 {
                    out = (out & 0xFF00) | (garbage & 0xFF);
                }
                if mask & 2 != 0 {
                    out = (out & 0x00FF) | (garbage & 0xFF00);
                }
                out
            }
            None => garbage,
        }
    }
}

/// The controller, its user port and the model on its pins.
struct Sdram<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    req_valid: NetHandle,
    req_ready: NetHandle,
    req_we: NetHandle,
    req_addr: NetHandle,
    req_wdata: NetHandle,
    req_be: NetHandle,
    rd_data: NetHandle,
    rd_valid: NetHandle,
    init_done: NetHandle,
    pins: [NetHandle; 10],
    dq_i: NetHandle,
    addr_width: u32,
    model: SdramModel,
    /// Cycles of `clk` since reset.
    cycles: u64,
}

impl<'d> Sdram<'d> {
    fn new(design: &'d Design, part: SdramPart) -> Sdram<'d> {
        let sim = simulate(design, "sdram_ctrl");
        let pin = |n: &str| top_net(&sim, n);
        let pins = [
            pin("sdram_cke"),
            pin("sdram_cs_n"),
            pin("sdram_ras_n"),
            pin("sdram_cas_n"),
            pin("sdram_we_n"),
            pin("sdram_ba"),
            pin("sdram_a"),
            pin("sdram_dqm"),
            pin("sdram_dq_o"),
            pin("sdram_dq_oe"),
        ];
        let mut s = Sdram {
            clk: pin("clk"),
            req_valid: pin("req_valid"),
            req_ready: pin("req_ready"),
            req_we: pin("req_we"),
            req_addr: pin("req_addr"),
            req_wdata: pin("req_wdata"),
            req_be: pin("req_be"),
            rd_data: pin("rd_data"),
            rd_valid: pin("rd_valid"),
            init_done: pin("init_done"),
            dq_i: pin("sdram_dq_i"),
            pins,
            addr_width: part.row_bits + 2 + part.col_bits,
            model: SdramModel::new(part),
            cycles: 0,
            sim,
        };
        let rst_n = top_net(&s.sim, "rst_n");
        s.sim.set(s.req_valid, bit(false));
        s.sim.set(s.req_we, bit(false));
        s.sim.set(s.req_addr, word(s.addr_width, 0));
        s.sim.set(s.req_wdata, word(16, 0));
        s.sim.set(s.req_be, word(2, 0));
        s.sim.set(s.dq_i, word(16, 0));
        let clk = s.clk;
        reset(&mut s.sim, clk, rst_n);
        // The forwarded clock is `clk` inverted, for as long as it runs.
        assert_eq!(get_u64(&s.sim, top_net(&s.sim, "sdram_clk")), 0b10);
        s
    }

    /// One cycle of `clk`: the rising edge the controller acts on, then
    /// the falling one the part acts on.
    fn tick(&mut self) {
        self.sim.run_for(HALF);
        self.sim.set(self.clk, bit(true));
        self.sim.run_for(HALF);
        self.sim.set(self.clk, bit(false));
        self.sim.run_for(0);
        let [cke, cs, ras, cas, we, ba, a, dqm, dq, oe] = self.pins;
        let pins = SdramPins {
            cke: high(&self.sim, cke),
            cmd: (get_u64(&self.sim, cs) << 3)
                | (get_u64(&self.sim, ras) << 2)
                | (get_u64(&self.sim, cas) << 1)
                | get_u64(&self.sim, we),
            ba: get_u64(&self.sim, ba),
            a: get_u64(&self.sim, a),
            dqm: get_u64(&self.sim, dqm),
            dq: u16::try_from(get_u64(&self.sim, dq)).expect("sixteen bits"),
            dq_oe: high(&self.sim, oe),
        };
        let out = self.model.edge(&pins);
        self.sim.set(self.dq_i, word(16, out));
        self.cycles += 1;
    }

    fn wait_for_init(&mut self) {
        for _ in 0..(self.model.tinit + 2000) {
            if high(&self.sim, self.init_done) {
                return;
            }
            self.tick();
        }
        panic!("init_done never rose");
    }

    /// Offers one request and waits for it to be taken. A read then
    /// waits for its data too, and returns it.
    fn request(&mut self, we: bool, addr: u64, data: u16, be: u64) -> Option<u16> {
        self.sim.set(self.req_we, bit(we));
        self.sim.set(self.req_addr, word(self.addr_width, addr));
        self.sim.set(self.req_wdata, word(16, u64::from(data)));
        self.sim.set(self.req_be, word(2, be));
        self.sim.set(self.req_valid, bit(true));
        let mut taken = false;
        for _ in 0..400 {
            let ready = high(&self.sim, self.req_ready);
            self.tick();
            if ready {
                taken = true;
                break;
            }
        }
        assert!(taken, "the request for {addr:#x} was never taken");
        self.sim.set(self.req_valid, bit(false));
        if we {
            return None;
        }
        for _ in 0..400 {
            self.tick();
            if high(&self.sim, self.rd_valid) {
                return Some(u16::try_from(get_u64(&self.sim, self.rd_data)).expect("16 bits"));
            }
        }
        panic!("the read of {addr:#x} never answered");
    }

    fn idle(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.tick();
        }
    }

    fn assert_clean(&self) {
        assert!(
            self.model.violations.is_empty(),
            "the SDRAM model caught the controller breaking the datasheet:\n  {}",
            self.model.violations.join("\n  ")
        );
    }
}

fn sdram_design(part: SdramPart) -> Design {
    let params = part.params();
    let refs: Vec<(&str, &str)> = params.iter().map(|(n, v)| (*n, v.as_str())).collect();
    design_of("sdram_ctrl", "sdram_ctrl", &refs)
}

/// The word address of (row, bank, column), as the block maps it.
fn sdram_addr(part: SdramPart, row: u64, bank: u64, col: u64) -> u64 {
    (row << (part.col_bits + 2)) | (bank << part.col_bits) | col
}

/// Runs a mixed workload against one part and checks every read.
fn sdram_workload(part: SdramPart) {
    let design = sdram_design(part);
    let mut ram = Sdram::new(&design, part);
    ram.wait_for_init();
    assert!(
        ram.model.initialised(),
        "init_done rose before the part was initialised"
    );
    assert!(
        ram.cycles > ram.model.tinit,
        "init_done before the power-up wait"
    );
    assert_eq!(ram.model.count("REFRESH") as u64, part.init_refreshes);

    let mut reference: BTreeMap<u64, u16> = BTreeMap::new();
    let write =
        |ram: &mut Sdram<'_>, reference: &mut BTreeMap<u64, u16>, addr: u64, data: u16, be: u64| {
            ram.request(true, addr, data, be);
            let mut new = reference.get(&addr).copied().unwrap_or(0);
            if be & 1 != 0 {
                new = (new & 0xFF00) | (data & 0x00FF);
            }
            if be & 2 != 0 {
                new = (new & 0x00FF) | (data & 0xFF00);
            }
            reference.insert(addr, new);
        };

    // A row filled and read back: one ACTIVE for the lot.
    let row_a = sdram_addr(part, 5, 1, 0);
    let refs_before = ram.model.count("REFRESH");
    let acts_before = ram.model.count("ACTIVE");
    for col in 0..16u16 {
        write(
            &mut ram,
            &mut reference,
            row_a + u64::from(col),
            0x1000 + col * 0x0101,
            3,
        );
    }
    for col in 0..16u16 {
        let got = ram
            .request(false, row_a + u64::from(col), 0, 0)
            .expect("a read");
        assert_eq!(got, 0x1000 + col * 0x0101, "row hit, column {col}");
    }
    let acts = ram.model.count("ACTIVE") - acts_before;
    let refs = ram.model.count("REFRESH") - refs_before;
    assert!(
        acts <= 1 + refs,
        "32 accesses to one row took {acts} ACTIVE commands with {refs} refresh(es) between"
    );
    assert!(acts >= 1);

    // Byte enables: each octet alone, then neither.
    let addr = sdram_addr(part, 5, 1, 3);
    write(&mut ram, &mut reference, addr, 0xAB00, 2);
    write(&mut ram, &mut reference, addr, 0x00CD, 1);
    write(&mut ram, &mut reference, addr, 0xFFFF, 0);
    let expect = reference[&addr];
    assert_eq!(expect, 0xABCD);
    assert_eq!(ram.request(false, addr, 0, 0), Some(0xABCD), "byte enables");

    // A row miss in the same bank, and back: precharge, activate, both
    // ways, with the first row's data intact.
    let row_b = sdram_addr(part, 9, 1, 0);
    write(&mut ram, &mut reference, row_b + 7, 0xBEEF, 3);
    assert_eq!(
        ram.request(false, row_a + 7, 0, 0),
        Some(reference[&(row_a + 7)])
    );
    assert_eq!(ram.request(false, row_b + 7, 0, 0), Some(0xBEEF));

    // Every bank, and the top of the address space.
    let top_row = (1u64 << part.row_bits) - 1;
    let top_col = (1u64 << part.col_bits) - 1;
    for bank in 0..4u16 {
        let a = sdram_addr(part, top_row, u64::from(bank), top_col);
        write(&mut ram, &mut reference, a, 0xC000 | bank, 3);
        let n = u64::from(bank);
        let b = sdram_addr(part, n, n, n);
        write(&mut ram, &mut reference, b, 0xD000 | bank, 3);
    }
    for bank in 0..4u16 {
        let a = sdram_addr(part, top_row, u64::from(bank), top_col);
        assert_eq!(ram.request(false, a, 0, 0), Some(0xC000 | bank));
        let n = u64::from(bank);
        let b = sdram_addr(part, n, n, n);
        assert_eq!(ram.request(false, b, 0, 0), Some(0xD000 | bank));
    }

    // A pseudo-random stream over a handful of rows in every bank,
    // mixing reads, writes and partial writes, long enough to cross
    // several refreshes in the middle of traffic.
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        u64::from(seed >> 8)
    };
    let space = 1u64 << ram.addr_width;
    let mut reads = 0;
    for _ in 0..400 {
        let r = next();
        let row = (r >> 4) % 4;
        let bank = (r >> 8) & 3;
        let col = (r >> 10) % 6;
        let addr = sdram_addr(part, row * 97 % (1 << part.row_bits), bank, col) % space;
        if r & 3 == 0 || !reference.contains_key(&addr) {
            let be = match r & 0x30 {
                0x00 => 1,
                0x10 => 2,
                _ => 3,
            };
            write(
                &mut ram,
                &mut reference,
                addr,
                u16::try_from(next() & 0xFFFF).expect("16 bits"),
                be,
            );
        } else {
            let got = ram.request(false, addr, 0, 0).expect("a read");
            assert_eq!(got, reference[&addr], "read of {addr:#x}");
            reads += 1;
        }
    }
    assert!(
        reads > 100,
        "the stream should be mostly reads, got {reads}"
    );

    // Left alone, it keeps refreshing on time, and the data survives.
    let before = ram.model.count("REFRESH");
    let idle = ram.model.trefi * 4;
    ram.idle(idle);
    let during = ram.model.count("REFRESH") - before;
    assert!(
        during as u64 >= 4,
        "{during} refresh(es) in {idle} idle cycles with tREFI {}",
        ram.model.trefi
    );
    for (addr, value) in reference.iter().take(40) {
        assert_eq!(
            ram.request(false, *addr, 0, 0),
            Some(*value),
            "after the idle spell"
        );
    }

    let total = ram.model.count("REFRESH");
    assert!(total > 8, "the run crossed {total} refreshes");
    ram.assert_clean();
}

#[test]
fn sdram_ctrl_keeps_the_datasheet_at_cas_latency_2() {
    sdram_workload(MT48LC16M16A2);
}

#[test]
fn sdram_ctrl_keeps_the_datasheet_at_cas_latency_3() {
    sdram_workload(IS42S16400);
}

#[test]
fn sdram_ctrl_derives_its_cycle_counts_from_nanoseconds() {
    // tRAS 44 ns at 75 MHz is 3.3 cycles, so 4; tWR 15 ns is 1.125, so 2;
    // the refresh interval, a maximum, rounds down: 585.9 to 585.
    let design = sdram_design(MT48LC16M16A2);
    let module = design.top_module().expect("a top");
    let port = module.port("req_addr").expect("req_addr");
    assert_eq!(module.nets[port.net].ty.width(), Some(24), "13 + 2 + 9");
    let part = MT48LC16M16A2;
    assert_eq!(part.cycles(part.ras_ns), 4);
    assert_eq!(part.cycles(part.wr_ns), 2);
    assert_eq!(part.refi(), 585);

    let design = sdram_design(IS42S16400);
    let module = design.top_module().expect("a top");
    let port = module.port("req_addr").expect("req_addr");
    assert_eq!(module.nets[port.net].ty.width(), Some(22), "12 + 2 + 8");
    let port = module.port("sdram_a").expect("sdram_a");
    assert_eq!(module.nets[port.net].ty.width(), Some(12));
}

/// The model is not a pushover: a part stricter than the one the
/// controller was built for is caught, each time by the rule the
/// difference breaks. Without this, the clean runs above would prove
/// nothing about the model.
#[test]
fn the_sdram_model_catches_a_controller_that_breaks_the_datasheet() {
    type Stricter = fn(&mut SdramPart);
    let cases: [(Stricter, &str); 6] = [
        (|p| p.rcd_ns = 30, "tRCD"),
        (|p| p.rp_ns = 30, "tRP"),
        (|p| p.ras_ns = 80, "tRAS"),
        (|p| p.rc_ns = 100, "tRC"),
        (|p| p.wr_ns = 40, "tWR"),
        (|p| p.refi_ns = 3000, "refresh overdue"),
    ];
    let design = sdram_design(MT48LC16M16A2);
    for (stricter, rule) in cases {
        let mut part = MT48LC16M16A2;
        stricter(&mut part);
        let mut ram = Sdram::new(&design, part);
        ram.wait_for_init();
        // A write to one row of a bank after another: every one a row
        // miss straight after a write, which is where the row timings
        // bite; then a read of each, and a long wait for the refresh.
        for i in 0..6u64 {
            ram.request(true, sdram_addr(part, i, 0, 1), 0x1111, 3);
        }
        for i in 0..6u64 {
            ram.request(false, sdram_addr(part, i, 0, 1), 0, 0);
        }
        ram.idle(MT48LC16M16A2.refi() * 2);
        assert!(
            ram.model.violations.iter().any(|v| v.contains(rule)),
            "a part needing more should break {rule}, the model saw: {:?}",
            ram.model.violations
        );
    }
}

// ---------------------------------------------------------------------------
// hyperram_ctrl
// ---------------------------------------------------------------------------

/// The latency, in clocks, a configuration register 0 value selects.
fn hyper_latency(cr0: u16) -> u32 {
    match (cr0 >> 4) & 0xF {
        0 => 5,
        1 => 6,
        2 => 7,
        14 => 3,
        15 => 4,
        other => panic!("reserved latency code {other}"),
    }
}

/// The word address of a HyperRAM register, as CA[44:16] and CA[2:0]
/// spell it: configuration register 0 is CA 0x0000_0100_0000 with the
/// read and address-space bits aside.
const HYPER_CR0: u32 = 0x800;
const HYPER_CR1: u32 = 0x801;
const HYPER_ID0: u32 = 0x000;
/// What the model answers for them, and CR0's reset value: six clocks,
/// fixed latency, legacy wrapped bursts of 32 bytes.
const HYPER_CR0_RESET: u16 = 0x8F1F;
const HYPER_CR1_VALUE: u16 = 0xFFC1;
const HYPER_ID0_VALUE: u16 = 0x0C81;

/// A HyperRAM that **enforces** the initial latency.
///
/// It counts CK edges from the fall of CS#, reads the command-address
/// from the first six, and then expects the data exactly where its
/// latency — single or doubled, fixed or chosen per transaction — puts
/// it: a write whose data is on the bus a clock early drives DQ during
/// the latency count, one a clock late has no data at the first beat,
/// and both are violations. A read's data is driven with RWDS toggling
/// from the first beat and nothing before. In variable-latency mode it
/// doubles every third transaction, as a refresh collision would, and
/// says so on RWDS during CA the way the part does.
struct HyperModel {
    mem: BTreeMap<u32, u16>,
    cr0: u16,
    t_rwr: u64,
    /// CK edges since CS# fell in this transaction.
    edge: u32,
    in_tx: bool,
    ca: u64,
    doubled: bool,
    transactions: u64,
    /// Bytes written in this transaction, with their RWDS masks.
    beats: Vec<(u8, bool)>,
    /// The word address the next data beat belongs to.
    next_addr: u32,
    /// What the part drives, when it drives.
    dq: Option<u8>,
    rwds: Option<bool>,
    /// The clock cycle CS# last rose in.
    cs_rose: Option<u64>,
    violations: Vec<String>,
    /// How many transactions ran with a doubled latency.
    doubled_count: u64,
}

/// The HyperBus as it is at one CK edge: what the controller drives.
#[derive(Clone, Copy)]
struct HyperBus {
    dq: Option<u8>,
    rwds: Option<bool>,
}

impl HyperModel {
    fn new(t_rwr: u64) -> HyperModel {
        HyperModel {
            mem: BTreeMap::new(),
            cr0: HYPER_CR0_RESET,
            t_rwr,
            edge: 0,
            in_tx: false,
            ca: 0,
            doubled: false,
            transactions: 0,
            beats: Vec::new(),
            next_addr: 0,
            dq: None,
            rwds: None,
            cs_rose: None,
            violations: Vec::new(),
            doubled_count: 0,
        }
    }

    fn violation(&mut self, what: String) {
        if self.violations.len() < 20 {
            self.violations.push(format!(
                "transaction {}, CK edge {}: {what}",
                self.transactions, self.edge
            ));
        }
    }

    fn fixed(&self) -> bool {
        self.cr0 & 0x8 != 0
    }

    fn is_read(&self) -> bool {
        self.ca >> 47 & 1 == 1
    }

    fn is_reg(&self) -> bool {
        self.ca >> 46 & 1 == 1
    }

    fn address(&self) -> u32 {
        let upper = u32::try_from((self.ca >> 16) & 0x1FFF_FFFF).expect("29 bits");
        let lower = u32::try_from(self.ca & 7).expect("3 bits");
        (upper << 3) | lower
    }

    /// The CK edge at which the first data byte is transferred: the
    /// rising edge that begins clock 3 + latency, counting the CA's
    /// first clock as clock 1, so edge 4 + 2 * latency counting from 0.
    fn first_data_edge(&self) -> u32 {
        if self.is_reg() && !self.is_read() {
            return 6;
        }
        let count = hyper_latency(self.cr0);
        let clocks = if self.doubled { 2 * count } else { count };
        4 + 2 * clocks
    }

    /// CS# fell in clock cycle `cycle`.
    fn select(&mut self, cycle: u64) {
        if let Some(rose) = self.cs_rose {
            let high = cycle - rose;
            if high < self.t_rwr {
                self.violation(format!(
                    "CS# high for {high} cycle(s), tRWR needs {}",
                    self.t_rwr
                ));
            }
        }
        self.transactions += 1;
        self.in_tx = true;
        self.edge = 0;
        self.ca = 0;
        self.beats.clear();
        self.doubled = self.fixed() || self.transactions.is_multiple_of(3);
        if self.doubled {
            self.doubled_count += 1;
        }
        // RWDS during CA says whether the latency is doubled.
        self.rwds = Some(self.doubled);
        self.dq = None;
    }

    /// CS# rose in clock cycle `cycle`.
    fn deselect(&mut self, cycle: u64) {
        if self.in_tx && !self.is_read() && self.edge >= 6 {
            if !self.beats.len().is_multiple_of(2) {
                self.violation("CS# rose in the middle of a word".into());
            }
            if self.beats.is_empty() {
                self.violation("a write ended with no data".into());
            }
        }
        if self.in_tx && self.edge < 6 {
            self.violation(format!("CS# rose after {} CA edge(s)", self.edge));
        }
        self.in_tx = false;
        self.dq = None;
        self.rwds = None;
        self.cs_rose = Some(cycle);
    }

    /// One CK edge while CS# is low, with what the controller drives.
    fn ck_edge(&mut self, bus: HyperBus) {
        let e = self.edge;
        self.edge += 1;
        if bus.rwds.is_some() && (e < 6 || self.is_read()) {
            self.violation("the controller drives RWDS while the part does".into());
        }
        if e < 6 {
            match bus.dq {
                Some(byte) => self.ca = (self.ca << 8) | u64::from(byte),
                None => self.violation("no command-address byte on DQ".into()),
            }
            if e == 5 {
                self.next_addr = self.address();
                // After CA the part stops signalling latency; a read's
                // RWDS is its preamble, low, until the data.
                self.rwds = if self.is_read() { Some(false) } else { None };
            }
            return;
        }
        let first = self.first_data_edge();
        if self.is_read() {
            if bus.dq.is_some() {
                self.violation("the controller drives DQ during a read".into());
            }
            if e >= first {
                // Beat `e - first` goes out from this edge to the next,
                // with RWDS high over a word's first byte and low over
                // its second.
                let beat = e - first;
                let [hi, lo] = self.read_word(self.next_addr).to_be_bytes();
                self.dq = Some(if beat.is_multiple_of(2) { hi } else { lo });
                self.rwds = Some(beat.is_multiple_of(2));
                if beat % 2 == 1 {
                    self.next_addr += 1;
                }
            }
            return;
        }
        // A write.
        if e < first {
            if bus.dq.is_some() {
                self.violation(format!(
                    "DQ driven during the latency count, {} edge(s) early",
                    first - e
                ));
            }
            return;
        }
        let Some(byte) = bus.dq else {
            self.violation(format!("no write data at beat {}", e - first));
            return;
        };
        let masked = if self.is_reg() {
            false
        } else {
            match bus.rwds {
                Some(mask) => mask,
                None => {
                    self.violation("a memory write with RWDS not driven".into());
                    true
                }
            }
        };
        self.beats.push((byte, masked));
        if self.beats.len().is_multiple_of(2) {
            let n = self.beats.len();
            let (hi, hi_masked) = self.beats[n - 2];
            let (lo, lo_masked) = self.beats[n - 1];
            let addr = self.next_addr;
            self.next_addr += 1;
            if self.is_reg() {
                let value = (u16::from(hi) << 8) | u16::from(lo);
                match addr {
                    HYPER_CR0 => self.cr0 = value,
                    HYPER_CR1 => {}
                    other => self.violation(format!("a write to register {other:#x}")),
                }
            } else {
                let mut value = self.mem.get(&addr).copied().unwrap_or(0);
                if !hi_masked {
                    value = (value & 0x00FF) | (u16::from(hi) << 8);
                }
                if !lo_masked {
                    value = (value & 0xFF00) | u16::from(lo);
                }
                self.mem.insert(addr, value);
            }
        }
    }

    fn read_word(&self, addr: u32) -> u16 {
        if self.is_reg() {
            match addr {
                HYPER_CR0 => self.cr0,
                HYPER_CR1 => HYPER_CR1_VALUE,
                HYPER_ID0 => HYPER_ID0_VALUE,
                _ => 0,
            }
        } else {
            self.mem.get(&addr).copied().unwrap_or(0)
        }
    }
}

/// The controller with the model on its pins, and the IO registers
/// between them modelled the way the FPGA backend builds them.
///
/// A double-data-rate output port is registered on the rising edge of
/// `clk` and appears on the pin for the whole next cycle, its low half
/// first and its high half after the falling edge; `hram_ck` then goes
/// through a quarter-cycle IO delay, which is what `CK_DELAY` is for. A
/// double-data-rate input samples the pin on the rising edge and on the
/// falling edge, and the fabric sees the pair at the next rising edge.
/// Chip select and the output enables are ordinary outputs. The
/// testbench steps in quarter cycles to put each of those events where
/// it belongs.
struct Hyper<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    req_valid: NetHandle,
    req_ready: NetHandle,
    req_we: NetHandle,
    req_reg: NetHandle,
    req_addr: NetHandle,
    req_wdata: NetHandle,
    req_be: NetHandle,
    rd_data: NetHandle,
    rd_valid: NetHandle,
    rd_error: NetHandle,
    cur_latency: NetHandle,
    cur_fixed: NetHandle,
    cs_n: NetHandle,
    ck: NetHandle,
    dq_o: NetHandle,
    dq_oe: NetHandle,
    dq_i: NetHandle,
    rwds_o: NetHandle,
    rwds_oe: NetHandle,
    rwds_i: NetHandle,
    model: HyperModel,
    cycle: u64,
    ck_level: bool,
    cs_level: bool,
}

impl<'d> Hyper<'d> {
    fn new(design: &'d Design, t_rwr: u64) -> Hyper<'d> {
        let sim = simulate(design, "hyperram_ctrl");
        let pin = |n: &str| top_net(&sim, n);
        let mut h = Hyper {
            clk: pin("clk"),
            req_valid: pin("req_valid"),
            req_ready: pin("req_ready"),
            req_we: pin("req_we"),
            req_reg: pin("req_reg"),
            req_addr: pin("req_addr"),
            req_wdata: pin("req_wdata"),
            req_be: pin("req_be"),
            rd_data: pin("rd_data"),
            rd_valid: pin("rd_valid"),
            rd_error: pin("rd_error"),
            cur_latency: pin("cur_latency"),
            cur_fixed: pin("cur_fixed"),
            cs_n: pin("hram_cs_n"),
            ck: pin("hram_ck"),
            dq_o: pin("hram_dq_o"),
            dq_oe: pin("hram_dq_oe"),
            dq_i: pin("hram_dq_i"),
            rwds_o: pin("hram_rwds_o"),
            rwds_oe: pin("hram_rwds_oe"),
            rwds_i: pin("hram_rwds_i"),
            model: HyperModel::new(t_rwr),
            cycle: 0,
            ck_level: false,
            cs_level: true,
            sim,
        };
        let rst_n = top_net(&h.sim, "rst_n");
        for net in [h.req_valid, h.req_we, h.req_reg] {
            h.sim.set(net, bit(false));
        }
        h.sim.set(h.req_addr, word(22, 0));
        h.sim.set(h.req_wdata, word(16, 0));
        h.sim.set(h.req_be, word(2, 0));
        h.sim.set(h.dq_i, word(16, 0));
        h.sim.set(h.rwds_i, word(2, 0));
        let clk = h.clk;
        reset(&mut h.sim, clk, rst_n);
        h
    }

    /// The byte on DQ and the level on RWDS, as the IO sees them: the
    /// controller's where it drives, the part's where it does, a
    /// changing pattern where nobody does.
    fn bus(&mut self, ctrl: HyperBus) -> (u8, bool) {
        if ctrl.dq.is_some() && self.model.dq.is_some() {
            self.model.violation("both ends drive DQ".into());
        }
        if ctrl.rwds.is_some() && self.model.rwds.is_some() {
            self.model.violation("both ends drive RWDS".into());
        }
        let floating = self.cycle.wrapping_mul(0x5B).to_le_bytes()[0];
        (
            ctrl.dq.or(self.model.dq).unwrap_or(floating),
            ctrl.rwds.or(self.model.rwds).unwrap_or(self.cycle & 1 == 1),
        )
    }

    /// The controller's drive for one half of the cycle.
    fn drive(dq_oe: bool, rwds_oe: bool, dq: u64, rwds: u64, half: u32) -> HyperBus {
        HyperBus {
            dq: dq_oe.then(|| (dq >> (8 * half)).to_le_bytes()[0]),
            rwds: rwds_oe.then(|| (rwds >> half) & 1 == 1),
        }
    }

    fn tick(&mut self) {
        let quarter = HALF / 2;
        // What the output registers take at this rising edge, for the
        // cycle that follows it.
        let ck = get_u64(&self.sim, self.ck);
        let dq = get_u64(&self.sim, self.dq_o);
        let rwds = get_u64(&self.sim, self.rwds_o);

        // The rising edge: the input registers sample the low half.
        self.sim.set(self.clk, bit(true));
        self.sim.run_for(0);
        self.cycle += 1;
        let cs = high(&self.sim, self.cs_n);
        if cs != self.cs_level {
            if cs {
                self.model.deselect(self.cycle);
            } else {
                self.model.select(self.cycle);
            }
            self.cs_level = cs;
        }
        let dq_oe = high(&self.sim, self.dq_oe);
        let rwds_oe = high(&self.sim, self.rwds_oe);
        let first = Self::drive(dq_oe, rwds_oe, dq, rwds, 0);
        let second = Self::drive(dq_oe, rwds_oe, dq, rwds, 1);
        let (lo_dq, lo_rwds) = self.bus(first);

        // A quarter later, CK takes the level of the low half.
        self.sim.run_for(quarter);
        self.ck_step(ck & 1 == 1, first);

        // The falling edge: the input registers sample the high half,
        // and the fabric sees both at the next rising edge.
        self.sim.run_for(quarter);
        let (hi_dq, hi_rwds) = self.bus(second);
        self.sim.set(
            self.dq_i,
            word(16, (u64::from(hi_dq) << 8) | u64::from(lo_dq)),
        );
        self.sim.set(
            self.rwds_i,
            word(2, (u64::from(hi_rwds) << 1) | u64::from(lo_rwds)),
        );
        self.sim.set(self.clk, bit(false));

        // And a quarter after that, the high half of CK.
        self.sim.run_for(quarter);
        self.ck_step(ck & 2 == 2, second);
        self.sim.run_for(quarter);
    }

    fn ck_step(&mut self, level: bool, bus: HyperBus) {
        if level != self.ck_level {
            self.ck_level = level;
            if self.cs_level {
                // CK may run with CS# high; the part ignores it.
            } else {
                self.model.ck_edge(bus);
            }
        }
    }

    /// One request; a read returns its word and whether it timed out.
    fn request(
        &mut self,
        we: bool,
        reg: bool,
        addr: u32,
        data: u16,
        be: u64,
    ) -> Option<(u16, bool)> {
        self.sim.set(self.req_we, bit(we));
        self.sim.set(self.req_reg, bit(reg));
        self.sim.set(self.req_addr, word(22, u64::from(addr)));
        self.sim.set(self.req_wdata, word(16, u64::from(data)));
        self.sim.set(self.req_be, word(2, be));
        self.sim.set(self.req_valid, bit(true));
        let mut taken = false;
        for _ in 0..100 {
            let ready = high(&self.sim, self.req_ready);
            self.tick();
            if ready {
                taken = true;
                break;
            }
        }
        assert!(taken, "the request for {addr:#x} was never taken");
        self.sim.set(self.req_valid, bit(false));
        for _ in 0..100 {
            self.tick();
            if !we && high(&self.sim, self.rd_valid) {
                let value = u16::try_from(get_u64(&self.sim, self.rd_data)).expect("16 bits");
                let error = high(&self.sim, self.rd_error);
                return Some((value, error));
            }
            if we && high(&self.sim, self.req_ready) {
                return None;
            }
        }
        panic!("the request for {addr:#x} never finished");
    }

    fn read(&mut self, reg: bool, addr: u32) -> u16 {
        let (value, error) = self.request(false, reg, addr, 0, 0).expect("a read");
        assert!(!error, "the read of {addr:#x} timed out");
        value
    }

    fn assert_clean(&self) {
        assert!(
            self.model.violations.is_empty(),
            "the HyperRAM model caught the controller breaking the protocol:\n  {}",
            self.model.violations.join("\n  ")
        );
    }
}

fn hyper_design(latency: &str, fixed: &str) -> Design {
    design_of(
        "hyperram_ctrl",
        "hyperram_ctrl",
        &[
            ("ADDR_WIDTH", "22"),
            ("LATENCY", latency),
            ("FIXED_LATENCY", fixed),
            ("T_RWR", "4"),
        ],
    )
}

/// Writes, partial writes and reads of memory, checked against a
/// reference, on whatever latency the part is set to now.
fn hyper_traffic(h: &mut Hyper<'_>, seed: u32) {
    let mut reference: BTreeMap<u32, u16> = BTreeMap::new();
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
        state >> 8
    };
    for i in 0..24u32 {
        let addr = (next() & 0x3F_FFFF) | (i & 1);
        let value = u16::try_from(next() & 0xFFFF).expect("16 bits");
        h.request(true, false, addr, value, 3);
        reference.insert(addr, value);
        // Every third one gets a byte masked off on a second write.
        if i % 3 == 0 {
            let be = if i % 2 == 0 { 1 } else { 2 };
            let patch = 0xA55A;
            h.request(true, false, addr, patch, be);
            let old = reference[&addr];
            let new = if be == 1 {
                (old & 0xFF00) | (patch & 0x00FF)
            } else {
                (old & 0x00FF) | (patch & 0xFF00)
            };
            reference.insert(addr, new);
        }
    }
    for (addr, value) in &reference {
        assert_eq!(h.read(false, *addr), *value, "memory word {addr:#x}");
    }
    // The model agrees with the reference about what it holds, so the
    // writes landed where they were meant to and not only somewhere the
    // reads could find them again.
    for (addr, value) in &reference {
        assert_eq!(
            h.model.mem.get(addr),
            Some(value),
            "the part's word {addr:#x}"
        );
    }
}

#[test]
fn hyperram_ctrl_reads_and_writes_at_the_fixed_reset_latency() {
    let design = hyper_design("6", "1");
    let mut h = Hyper::new(&design, 4);
    assert_eq!(get_u64(&h.sim, h.cur_latency), 6);
    assert!(high(&h.sim, h.cur_fixed));

    // The registers first: the identification and the configuration.
    assert_eq!(h.read(true, HYPER_ID0), HYPER_ID0_VALUE, "ID register 0");
    assert_eq!(h.read(true, HYPER_CR0), HYPER_CR0_RESET, "CR0 at reset");
    assert_eq!(h.read(true, HYPER_CR1), HYPER_CR1_VALUE, "CR1");

    hyper_traffic(&mut h, 7);
    // Fixed latency doubles every transaction.
    assert_eq!(h.model.doubled_count, h.model.transactions);
    h.assert_clean();
}

#[test]
fn hyperram_ctrl_follows_the_latency_it_configures() {
    let design = hyper_design("6", "1");
    let mut h = Hyper::new(&design, 4);

    // Every latency the part has, variable and fixed, set through CR0
    // and then used: the controller must time its writes the way the
    // part now expects, and its reads must find the data wherever the
    // part's RWDS puts it.
    for (code, clocks) in [(14u16, 3u64), (15, 4), (0, 5), (2, 7), (1, 6)] {
        for fixed in [false, true] {
            let cr0 = (HYPER_CR0_RESET & !0x00F8) | (code << 4) | if fixed { 0x8 } else { 0 };
            h.request(true, true, HYPER_CR0, cr0, 3);
            assert_eq!(h.model.cr0, cr0, "the part took the new CR0");
            assert_eq!(get_u64(&h.sim, h.cur_latency), clocks);
            assert_eq!(high(&h.sim, h.cur_fixed), fixed);
            assert_eq!(h.read(true, HYPER_CR0), cr0, "and reads it back");
            let before = (h.model.transactions, h.model.doubled_count);
            hyper_traffic(&mut h, u32::from(code) * 2 + u32::from(fixed));
            let ran = h.model.transactions - before.0;
            let doubled = h.model.doubled_count - before.1;
            if fixed {
                assert_eq!(doubled, ran, "fixed latency doubles every transaction");
            } else {
                // A third of them collided with a refresh, and the
                // controller had to see RWDS say so.
                assert!(doubled > 0 && doubled < ran, "{doubled} of {ran} doubled");
            }
        }
    }
    h.assert_clean();
}

/// The model is not a pushover: a controller that believes the part is
/// set to a different latency than it is gets caught, early or late.
#[test]
fn the_hyperram_model_catches_a_controller_with_the_wrong_latency() {
    // Too short a latency drives the data while the part is still
    // counting; too long leaves the part's first beat with nothing on it.
    for (latency, symptom) in [("5", "during the latency count"), ("7", "no write data")] {
        let design = hyper_design(latency, "1");
        let mut h = Hyper::new(&design, 4);
        h.request(true, false, 0x1234, 0xBEEF, 3);
        let _ = h.request(false, false, 0x1234, 0, 0);
        assert!(
            h.model.violations.iter().any(|v| v.contains(symptom)),
            "LATENCY = {latency} against a part at six: {:?}",
            h.model.violations
        );
    }
    // A controller that waits too little between transactions breaks
    // the part's read-write recovery time.
    let design = design_of(
        "hyperram_ctrl",
        "hyperram_ctrl",
        &[("LATENCY", "6"), ("FIXED_LATENCY", "1"), ("T_RWR", "1")],
    );
    let mut h = Hyper::new(&design, 4);
    h.request(true, false, 1, 1, 3);
    h.request(true, false, 2, 2, 3);
    assert!(
        h.model.violations.iter().any(|v| v.contains("tRWR")),
        "{:?}",
        h.model.violations
    );
}

// ---------------------------------------------------------------------------
// dvi_tx
// ---------------------------------------------------------------------------

/// The TMDS encoder exactly as the DVI 1.0 specification writes it, in
/// section 3.2.2's flow chart, with the running disparity an unbounded
/// integer. Returns the symbol, bit 0 first on the wire, and the new
/// disparity.
fn tmds_reference(d: u8, cnt: i32) -> (u16, i32) {
    let n1_d = d.count_ones();
    let bit = |v: u8, i: u32| (v >> i) & 1;
    let mut q_m: u16 = u16::from(bit(d, 0));
    let xnor = n1_d > 4 || (n1_d == 4 && bit(d, 0) == 0);
    for i in 1..8 {
        let prev = (q_m >> (i - 1)) & 1;
        let b = u16::from(bit(d, i));
        let next = if xnor { !(prev ^ b) & 1 } else { prev ^ b };
        q_m |= next << i;
    }
    if !xnor {
        q_m |= 1 << 8;
    }
    let q_m8 = (q_m >> 8) & 1;
    let low = q_m & 0xFF;
    let n1 = i32::try_from(low.count_ones()).expect("eight bits");
    let n0 = 8 - n1;
    if cnt == 0 || n1 == n0 {
        let q9 = 1 - q_m8;
        let body = if q_m8 == 1 { low } else { !low & 0xFF };
        let q = (q9 << 9) | (q_m8 << 8) | body;
        let cnt = if q_m8 == 0 {
            cnt + (n0 - n1)
        } else {
            cnt + (n1 - n0)
        };
        (q, cnt)
    } else if (cnt > 0 && n1 > n0) || (cnt < 0 && n0 > n1) {
        let q = (1 << 9) | (q_m8 << 8) | (!low & 0xFF);
        (q, cnt + 2 * i32::from(q_m8) + (n0 - n1))
    } else {
        let q = (q_m8 << 8) | low;
        (q, cnt - 2 * (1 - i32::from(q_m8)) + (n1 - n0))
    }
}

/// The four control-period symbols, by {C1, C0}.
const TMDS_CONTROL: [u16; 4] = [
    0b11_0101_0100,
    0b00_1010_1011,
    0b01_0101_0100,
    0b10_1010_1011,
];

/// What a TMDS receiver makes of a symbol: a control pair, or a byte.
/// Written from the decoder half of the specification, which undoes the
/// encoder without knowing the disparity.
fn tmds_decode(q: u16) -> Result<u8, u8> {
    if let Some(c) = TMDS_CONTROL.iter().position(|s| *s == q) {
        return Err(u8::try_from(c).expect("two bits"));
    }
    let mut body = q & 0xFF;
    if q >> 9 & 1 == 1 {
        body = !body & 0xFF;
    }
    let xor = q >> 8 & 1 == 1;
    let mut d = body & 1;
    for i in 1..8 {
        let b = (body >> i) & 1;
        let prev = (body >> (i - 1)) & 1;
        let bit = if xor { b ^ prev } else { !(b ^ prev) & 1 };
        d |= bit << i;
    }
    Ok(u8::try_from(d).expect("eight bits"))
}

/// Every running disparity the algorithm can reach from zero, each with
/// the shortest run of bytes that reaches it.
fn tmds_reachable() -> BTreeMap<i32, Vec<u8>> {
    let mut paths: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
    paths.insert(0, Vec::new());
    let mut frontier = vec![0i32];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for cnt in frontier {
            let path = paths[&cnt].clone();
            for d in 0..=255u8 {
                let (_, after) = tmds_reference(d, cnt);
                if let std::collections::btree_map::Entry::Vacant(e) = paths.entry(after) {
                    let mut p = path.clone();
                    p.push(d);
                    e.insert(p);
                    next.push(after);
                }
            }
        }
        frontier = next;
    }
    paths
}

#[test]
fn tmds_encoder_matches_the_specification_for_every_byte_in_every_disparity() {
    // The disparity the algorithm can reach is bounded, and the block's
    // six-bit register holds all of it.
    let reachable = tmds_reachable();
    let lowest = *reachable.keys().next().expect("zero at least");
    let highest = *reachable.keys().last().expect("zero at least");
    assert!(
        lowest >= -32 && highest <= 31,
        "the disparity reaches {lowest}..{highest}, beyond six bits"
    );
    assert!(lowest < 0 && highest > 0, "both signs are reachable");

    let design = design_of("dvi_tx", "tmds_encoder", &[]);
    let mut sim = simulate(&design, "tmds_encoder");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let en = top_net(&sim, "en");
    let de = top_net(&sim, "de");
    let c = top_net(&sim, "c");
    let d = top_net(&sim, "d");
    let q = top_net(&sim, "q");
    let cnt = top_net(&sim, "cnt");
    sim.set(en, bit(true));
    sim.set(de, bit(false));
    sim.set(c, word(2, 0));
    sim.set(d, word(8, 0));
    reset(&mut sim, clk, rst_n);

    let hw_cnt = |sim: &Simulator<'_>| -> i32 {
        let raw = i32::try_from(get_u64(sim, cnt)).expect("six bits");
        if raw >= 32 { raw - 64 } else { raw }
    };

    // The control period first: each pair its symbol, and the disparity
    // back to zero.
    for (pair, symbol) in TMDS_CONTROL.iter().enumerate() {
        sim.set(de, bit(false));
        sim.set(c, word(2, pair as u64));
        cycle(&mut sim, clk, HALF);
        assert_eq!(get_u64(&sim, q), u64::from(*symbol), "control {pair:02b}");
        assert_eq!(hw_cnt(&sim), 0);
    }

    // Then every byte from every disparity the encoder can be in: a
    // control symbol to zero it, the shortest run of bytes to the
    // disparity wanted, checked on the way, and the byte under test.
    let mut checked = 0;
    for (start, path) in &reachable {
        for value in 0..=255u8 {
            sim.set(de, bit(false));
            sim.set(c, word(2, 0));
            cycle(&mut sim, clk, HALF);
            sim.set(de, bit(true));
            let mut model = 0;
            for byte in path.iter().copied().chain(std::iter::once(value)) {
                sim.set(d, word(8, u64::from(byte)));
                cycle(&mut sim, clk, HALF);
                let (symbol, after) = tmds_reference(byte, model);
                assert_eq!(
                    get_u64(&sim, q),
                    u64::from(symbol),
                    "byte {byte:#04x} at disparity {model} (testing {value:#04x} at {start})"
                );
                assert_eq!(
                    hw_cnt(&sim),
                    after,
                    "disparity after {byte:#04x} at {model}"
                );
                assert_eq!(tmds_decode(symbol), Ok(byte), "the symbol decodes back");
                model = after;
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 256 * reachable.len());

    // `en` low holds everything.
    sim.set(en, bit(false));
    let before = get_u64(&sim, q);
    sim.set(d, word(8, 0x5A));
    cycle(&mut sim, clk, HALF);
    assert_eq!(get_u64(&sim, q), before, "no enable, no new symbol");
}

/// One standard mode's numbers, as VESA and CEA-861 publish them.
struct VideoMode {
    mode: &'static str,
    h: [u64; 4],
    v: [u64; 4],
    sync_high: bool,
}

const VIDEO_MODES: [VideoMode; 3] = [
    VideoMode {
        mode: "0",
        h: [640, 16, 96, 48],
        v: [480, 10, 2, 33],
        sync_high: false,
    },
    VideoMode {
        mode: "1",
        h: [800, 40, 128, 88],
        v: [600, 1, 4, 23],
        sync_high: true,
    },
    VideoMode {
        mode: "2",
        h: [1280, 110, 40, 220],
        v: [720, 5, 5, 20],
        sync_high: true,
    },
];

/// The raster for one mode, counted: a whole frame and the start of the
/// next, every pixel enabled.
fn check_video_mode(m: &VideoMode) {
    let design = design_of("dvi_tx", "video_timing", &[("MODE", m.mode)]);
    let mut sim = simulate(&design, "video_timing");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let en = top_net(&sim, "en");
    let de = top_net(&sim, "de");
    let hsync = top_net(&sim, "hsync");
    let vsync = top_net(&sim, "vsync");
    let frame = top_net(&sim, "frame");
    let x = top_net(&sim, "x");
    let y = top_net(&sim, "y");
    sim.set(en, bit(true));
    reset(&mut sim, clk, rst_n);

    let h_total: u64 = m.h.iter().sum();
    let v_total: u64 = m.v.iter().sum();
    let active = |s: bool| s == m.sync_high;

    // Walk one frame pixel by pixel and compare with where each pixel
    // must be.
    let mut visible = 0u64;
    let mut hsync_pixels = 0u64;
    let mut vsync_lines = BTreeSet::new();
    for line in 0..v_total {
        for pixel in 0..h_total {
            let on = high(&sim, de);
            let want = pixel < m.h[0] && line < m.v[0];
            assert_eq!(on, want, "mode {}: de at ({pixel}, {line})", m.mode);
            if on {
                visible += 1;
                assert_eq!(get_u64(&sim, x), pixel);
                assert_eq!(get_u64(&sim, y), line);
            }
            let h_sync = pixel >= m.h[0] + m.h[1] && pixel < m.h[0] + m.h[1] + m.h[2];
            assert_eq!(
                active(high(&sim, hsync)),
                h_sync,
                "mode {}: hsync at pixel {pixel}",
                m.mode
            );
            if h_sync && line == 0 {
                hsync_pixels += 1;
            }
            let v_sync = line >= m.v[0] + m.v[1] && line < m.v[0] + m.v[1] + m.v[2];
            assert_eq!(
                active(high(&sim, vsync)),
                v_sync,
                "mode {}: vsync on line {line}",
                m.mode
            );
            if v_sync {
                vsync_lines.insert(line);
            }
            assert_eq!(high(&sim, frame), line == 0 && pixel == 0);
            cycle(&mut sim, clk, HALF);
        }
    }
    assert_eq!(visible, m.h[0] * m.v[0], "mode {}: visible pixels", m.mode);
    assert_eq!(hsync_pixels, m.h[2], "mode {}: hsync width", m.mode);
    assert_eq!(
        vsync_lines.len() as u64,
        m.v[2],
        "mode {}: vsync lines",
        m.mode
    );
    // And the frame wraps to the top left.
    assert!(high(&sim, frame), "mode {}: the next frame starts", m.mode);
    assert_eq!(get_u64(&sim, x), 0);
    assert_eq!(get_u64(&sim, y), 0);
}

#[test]
fn video_timing_counts_640x480() {
    check_video_mode(&VIDEO_MODES[0]);
}

#[test]
fn video_timing_counts_800x600() {
    check_video_mode(&VIDEO_MODES[1]);
}

#[test]
fn video_timing_counts_1280x720() {
    check_video_mode(&VIDEO_MODES[2]);
}

#[test]
fn dvi_tx_serialises_what_it_encodes() {
    let design = design_of("dvi_tx", "dvi_tx", &[("MODE", "0")]);
    let mut sim = simulate(&design, "dvi_tx");
    let clk = top_net(&sim, "clk_x5");
    let rst_n = top_net(&sim, "rst_n");
    let pix_en = top_net(&sim, "pix_en");
    let x = top_net(&sim, "x");
    let y = top_net(&sim, "y");
    let colour = [top_net(&sim, "b"), top_net(&sim, "g"), top_net(&sim, "r")];
    let lanes = [
        top_net(&sim, "tmds_d0"),
        top_net(&sim, "tmds_d1"),
        top_net(&sim, "tmds_d2"),
        top_net(&sim, "tmds_clk"),
    ];
    // A pattern that differs on every lane: blue x ^ y, green y, red x.
    let pattern = |px: u64, py: u64| -> [u8; 3] {
        let [x0, ..] = px.to_le_bytes();
        let [y0, ..] = py.to_le_bytes();
        [x0 ^ y0, y0, x0]
    };
    for net in colour {
        sim.set(net, word(8, 0));
    }
    reset(&mut sim, clk, rst_n);

    // Two whole lines and a little more, as bits on each lane.
    let mode = &VIDEO_MODES[0];
    let h_total: u64 = mode.h.iter().sum();
    let cycles = 5 * (2 * h_total + 4);
    let mut bits: [Vec<u8>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut pix_ens = 0u64;
    for _ in 0..cycles {
        // The colour for the pixel `x`, `y` name, ready before the next
        // pixel edge.
        let values = pattern(get_u64(&sim, x), get_u64(&sim, y));
        for (net, value) in colour.iter().zip(values) {
            sim.set(*net, word(8, u64::from(value)));
        }
        if high(&sim, pix_en) {
            pix_ens += 1;
        }
        cycle(&mut sim, clk, HALF);
        for (lane, net) in lanes.iter().enumerate() {
            let pair = get_u64(&sim, *net);
            // The low bit leaves on the rising edge, first.
            bits[lane].push((pair & 1) as u8);
            bits[lane].push((pair >> 1 & 1) as u8);
        }
    }
    assert!(pix_ens >= 2 * h_total, "one pixel enable in five cycles");

    // The clock lane is five ones and five zeros, and its rising edge is
    // where every symbol starts.
    let start = bits[3]
        .windows(10)
        .position(|w| w == [1, 1, 1, 1, 1, 0, 0, 0, 0, 0])
        .expect("the clock pattern appears");
    let symbols = |lane: usize| -> Vec<u16> {
        bits[lane][start..]
            .as_chunks::<10>()
            .0
            .iter()
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u16, |acc, (i, b)| acc | u16::from(*b) << i)
            })
            .collect()
    };
    for s in symbols(3) {
        assert_eq!(s, 0b00_0001_1111, "the clock lane never slips");
    }

    // Find the first symbol of pixel (0, 0): the first data symbol after
    // reset, which is where the raster starts.
    let lane0 = symbols(0);
    let first = lane0
        .iter()
        .position(|s| tmds_decode(*s).is_ok())
        .expect("a data symbol");
    let mut disparities = [0i32; 3];
    let mut checked = 0;
    let two_lines = usize::try_from(2 * h_total).expect("fits");
    for (lane, disparity) in disparities.iter_mut().enumerate() {
        let stream = symbols(lane);
        for (i, symbol) in stream[first..].iter().enumerate().take(two_lines) {
            let i = i as u64;
            let (px, py) = (i % h_total, i / h_total);
            let visible = px < mode.h[0] && py < mode.v[0];
            if visible {
                let byte = pattern(px, py)[lane];
                let (expect, after) = tmds_reference(byte, *disparity);
                assert_eq!(*symbol, expect, "lane {lane}, pixel ({px}, {py})");
                *disparity = after;
                checked += 1;
            } else {
                // Blanking: hsync on lane 0 (active low in this mode),
                // vsync inactive, and nothing on the other two.
                let h_sync = px >= mode.h[0] + mode.h[1] && px < mode.h[0] + mode.h[1] + mode.h[2];
                let c = if lane == 0 {
                    // {vsync, hsync}, both active low here.
                    0b10 | u16::from(!h_sync)
                } else {
                    0
                };
                assert_eq!(
                    tmds_decode(*symbol),
                    Err(u8::try_from(c).expect("two bits")),
                    "lane {lane}, blanking pixel ({px}, {py})"
                );
                *disparity = 0;
            }
        }
    }
    assert_eq!(
        checked,
        3 * 2 * mode.h[0],
        "every visible pixel of two lines, on three lanes"
    );
}

// ---------------------------------------------------------------------------
// eth_mac_rgmii
// ---------------------------------------------------------------------------

/// The RGMII MAC with its transmit pins looped into its receive pins,
/// through the IO registers as the FPGA backend builds them.
///
/// A double-data-rate output register takes the port at a rising edge
/// and drives its low half for the first half of the next cycle and its
/// high half for the second; a double-data-rate input register samples
/// the pin on both edges and gives the fabric the pair at the next
/// rising edge. RGMII sends the clock with the data, edge aligned, and
/// the receiving end — the PHY's internal delay, or RX_DELAY here —
/// moves the sampling point into the middle of each half. So what the
/// transmit registers took at one edge is what the receive registers
/// hand over at the next, which is what this loop does, a cycle at a
/// time. Transmit and receive share one clock here, as they do in a
/// PHY's loopback mode.
struct Rgmii<'d> {
    sim: Simulator<'d>,
    tx_clk: NetHandle,
    rxc: NetHandle,
    txc: NetHandle,
    txd: NetHandle,
    tx_ctl: NetHandle,
    rxd: NetHandle,
    rx_ctl: NetHandle,
    tx_data: NetHandle,
    tx_valid: NetHandle,
    tx_ready: NetHandle,
    tx_last: NetHandle,
    tx_underrun: NetHandle,
    rx_data: NetHandle,
    rx_valid: NetHandle,
    rx_last: NetHandle,
    rx_crc_ok: NetHandle,
    rx_error: NetHandle,
    /// What the transmit pins carried, cycle by cycle: TX_CTL's two
    /// halves and the octet the two nibbles make.
    wire: Vec<(u64, u8)>,
}

impl<'d> Rgmii<'d> {
    fn new(design: &'d Design) -> Rgmii<'d> {
        let sim = simulate(design, "eth_mac_rgmii");
        let pin = |n: &str| top_net(&sim, n);
        let mut m = Rgmii {
            tx_clk: pin("tx_clk"),
            rxc: pin("rgmii_rxc"),
            txc: pin("rgmii_txc"),
            txd: pin("rgmii_txd"),
            tx_ctl: pin("rgmii_tx_ctl"),
            rxd: pin("rgmii_rxd"),
            rx_ctl: pin("rgmii_rx_ctl"),
            tx_data: pin("tx_data"),
            tx_valid: pin("tx_valid"),
            tx_ready: pin("tx_ready"),
            tx_last: pin("tx_last"),
            tx_underrun: pin("tx_underrun"),
            rx_data: pin("rx_data"),
            rx_valid: pin("rx_valid"),
            rx_last: pin("rx_last"),
            rx_crc_ok: pin("rx_crc_ok"),
            rx_error: pin("rx_error"),
            wire: Vec::new(),
            sim,
        };
        let rst_n = top_net(&m.sim, "rst_n");
        m.sim.set(m.tx_valid, bit(false));
        m.sim.set(m.tx_last, bit(false));
        m.sim.set(m.tx_data, word(8, 0));
        m.sim.set(m.rxd, word(8, 0));
        m.sim.set(m.rx_ctl, word(2, 0));
        m.sim.set(m.rxc, bit(false));
        m.sim.set(rst_n, bit(false));
        m.sim.run_for(HALF);
        m.tick(|_, _| None);
        m.tick(|_, _| None);
        m.sim.set(rst_n, bit(true));
        // The reset synchronisers release each half two edges later.
        for _ in 0..3 {
            m.tick(|_, _| None);
        }
        m.wire.clear();
        m
    }

    /// One cycle. `damage` may replace what the receive pins see, given
    /// the cycle number on the wire and the (ctl, octet) the transmitter
    /// drove.
    fn tick(&mut self, damage: impl Fn(usize, (u64, u8)) -> Option<(u64, u8)>) {
        // The output registers take the ports at this rising edge.
        let ctl = get_u64(&self.sim, self.tx_ctl);
        let octet = u8::try_from(get_u64(&self.sim, self.txd)).expect("eight bits");
        assert_eq!(
            get_u64(&self.sim, self.txc),
            0b01,
            "TXC rises with each low nibble and falls with each high one"
        );
        let n = self.wire.len();
        self.wire.push((ctl, octet));
        let (ctl, octet) = damage(n, (ctl, octet)).unwrap_or((ctl, octet));

        self.sim.run_for(HALF);
        self.sim.set(self.tx_clk, bit(true));
        self.sim.set(self.rxc, bit(true));
        self.sim.run_for(HALF);
        self.sim.set(self.tx_clk, bit(false));
        self.sim.set(self.rxc, bit(false));
        // Both halves have been on the pins and sampled; the fabric sees
        // them at the next rising edge.
        self.sim.set(self.rxd, word(8, u64::from(octet)));
        self.sim.set(self.rx_ctl, word(2, ctl));
    }
}

/// Sends `frame` with the transmitter looped into the receiver, returning
/// what came out, whether the check sequence held and whether the
/// receiver flagged an error.
fn rgmii_loopback(
    frame: &[u8],
    damage: impl Fn(usize, (u64, u8)) -> Option<(u64, u8)> + Copy,
) -> (Vec<u8>, bool, bool, Vec<(u64, u8)>) {
    let design = design_of("eth_mac_rgmii", "eth_mac_rgmii", &[("IFG_CYCLES", "12")]);
    let mut mac = Rgmii::new(&design);
    mac.sim.set(mac.tx_data, word(8, u64::from(frame[0])));
    mac.sim.set(mac.tx_valid, bit(true));
    mac.sim.set(mac.tx_last, bit(frame.len() == 1));

    let mut sent = 0usize;
    let mut got = Vec::new();
    let mut crc_ok = false;
    let mut error = false;
    let mut done = false;
    let mut taken_in_a_row = 0usize;
    let mut longest_run = 0usize;
    for _ in 0..400 {
        let taken = high(&mac.sim, mac.tx_valid) && high(&mac.sim, mac.tx_ready);
        mac.tick(damage);
        if taken {
            taken_in_a_row += 1;
            longest_run = longest_run.max(taken_in_a_row);
            sent += 1;
            if sent < frame.len() {
                mac.sim.set(mac.tx_data, word(8, u64::from(frame[sent])));
                mac.sim.set(mac.tx_last, bit(sent == frame.len() - 1));
            } else {
                mac.sim.set(mac.tx_valid, bit(false));
                mac.sim.set(mac.tx_last, bit(false));
            }
        } else {
            taken_in_a_row = 0;
        }
        if high(&mac.sim, mac.rx_error) {
            error = true;
            done = true;
        }
        if high(&mac.sim, mac.rx_valid) {
            got.push(octet(get_u64(&mac.sim, mac.rx_data)));
            if high(&mac.sim, mac.rx_last) {
                crc_ok = high(&mac.sim, mac.rx_crc_ok);
                done = true;
            }
        }
        if done && sent == frame.len() {
            // Let the gap go out too, for the tests that count it.
            for _ in 0..20 {
                mac.tick(damage);
            }
            break;
        }
    }
    assert_eq!(sent, frame.len(), "the transmitter took every octet");
    assert!(done, "the frame never finished arriving");
    assert!(!high(&mac.sim, mac.tx_underrun), "no underrun");
    assert_eq!(
        longest_run,
        frame.len(),
        "gigabit: one octet taken every cycle, the whole frame in one run"
    );
    (got, crc_ok, error, mac.wire)
}

fn rgmii_frame() -> Vec<u8> {
    (0..64u8)
        .map(|i| i.wrapping_mul(29).wrapping_add(3))
        .collect()
}

#[test]
fn eth_mac_rgmii_loops_a_frame_from_its_transmitter_into_its_receiver() {
    let frame = rgmii_frame();
    let (got, crc_ok, error, _) = rgmii_loopback(&frame, |_, _| None);
    assert!(!error);
    assert_eq!(got, frame, "every octet, in order");
    assert!(crc_ok, "and the check sequence held");

    let (got, crc_ok, _, _) = rgmii_loopback(&[0x5A], |_, _| None);
    assert_eq!(got, vec![0x5A], "a one-octet frame still comes back");
    assert!(crc_ok);
}

#[test]
fn eth_mac_rgmii_puts_a_standard_frame_on_the_wire() {
    let frame = rgmii_frame();
    let (_, _, _, wire) = rgmii_loopback(&frame, |_, _| None);
    // TX_CTL is the enable on both edges — enable, and enable XOR an
    // error that is never sent — so it is 00 or 11 and nothing else.
    assert!(
        wire.iter().all(|(ctl, _)| *ctl == 0 || *ctl == 3),
        "TX_CTL's halves disagree: {wire:?}"
    );
    let first = wire.iter().position(|(ctl, _)| *ctl == 3).expect("a frame");
    let len = wire[first..]
        .iter()
        .position(|(ctl, _)| *ctl == 0)
        .expect("and its end");
    let octets: Vec<u8> = wire[first..first + len].iter().map(|(_, o)| *o).collect();
    let mut expected = vec![0x55; 7];
    expected.push(0xD5);
    expected.extend_from_slice(&frame);
    expected.extend_from_slice(&eth_fcs(&frame));
    assert_eq!(octets, expected, "preamble, delimiter, payload, FCS");
    let gap = wire[first + len..]
        .iter()
        .take_while(|(ctl, _)| *ctl == 0)
        .count();
    assert!(
        gap >= 12,
        "the inter-frame gap is 96 bit times, got {gap} cycles"
    );
}

#[test]
fn eth_mac_rgmii_rejects_a_damaged_frame_and_one_the_phy_flags() {
    let frame = rgmii_frame();
    // One bit of one octet in the payload: past eight of preamble and
    // delimiter, and two cycles of IO registers and synchroniser.
    let (got, crc_ok, error, _) =
        rgmii_loopback(&frame, |n, (ctl, o)| (n == 30).then_some((ctl, o ^ 0x10)));
    assert!(!error, "a damaged frame is still well formed");
    assert_eq!(got.len(), frame.len());
    assert_ne!(got, frame);
    assert!(!crc_ok, "the check sequence is what says it is bad");

    // RX_CTL's falling half disagreeing with its rising half is the
    // PHY's receive error: the frame is dropped and reported.
    let (got, _, error, _) = rgmii_loopback(&frame, |n, (ctl, o)| {
        (n == 30 && ctl == 3).then_some((1, o))
    });
    assert!(error, "rx_error for a frame the PHY flagged");
    assert!(
        got.len() < frame.len(),
        "and the last octet is not delivered for it"
    );
}

/// Transmit and receive are two clock domains, and nothing crosses
/// between them: each half is released from reset by its own
/// synchroniser and runs on its own clock.
#[test]
fn eth_mac_rgmii_keeps_its_two_clock_domains_apart() {
    let (mut design, id) = flattened("eth_mac_rgmii", "eth_mac_rgmii", &[]);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    let module = flatten_for_timing(&design, id).expect("a flat module");
    let report = analyze_cdc_with(&module, &TimingSpec::default());
    let mut clocks: Vec<&str> = report.domains.iter().map(|d| d.net.as_str()).collect();
    clocks.sort_unstable();
    assert_eq!(clocks, ["rgmii_rxc", "tx_clk"], "one domain per direction");

    let kinds = crossings("eth_mac_rgmii", "eth_mac_rgmii", &[]);
    assert!(
        !kinds.contains(&CrossingKind::Unsynchronised),
        "an unsynchronised crossing in eth_mac_rgmii: {kinds:?}"
    );
    assert!(kinds.is_empty(), "nothing should cross: {kinds:?}");
}

// ---------------------------------------------------------------------------
// usb_device_fs
// ---------------------------------------------------------------------------

/// USB's CRC5 over a bit sequence, least significant bit first, from the
/// catalogue definition: polynomial 0x05 reflected, seeded and
/// complemented with all ones.
fn usb_crc5(bits: &[u8]) -> u8 {
    let mut c = 0x1Fu8;
    for b in bits {
        c = if (c ^ b) & 1 == 1 {
            (c >> 1) ^ 0x14
        } else {
            c >> 1
        };
    }
    c ^ 0x1F
}

/// USB's CRC16 over bytes, the same way: polynomial 0x8005 reflected.
fn usb_crc16(bytes: &[u8]) -> u16 {
    let mut c = 0xFFFFu16;
    for byte in bytes {
        for i in 0..8 {
            let b = u16::from(byte >> i & 1);
            c = if (c ^ b) & 1 == 1 {
                (c >> 1) ^ 0xA001
            } else {
                c >> 1
            };
        }
    }
    c ^ 0xFFFF
}

fn lsb_bits(value: u64, n: usize) -> Vec<u8> {
    (0..n).map(|i| u8::from(value >> i & 1 == 1)).collect()
}

const USB_OUT: u8 = 0b0001;
const USB_IN: u8 = 0b1001;
const USB_SETUP: u8 = 0b1101;
const USB_DATA0: u8 = 0b0011;
const USB_DATA1: u8 = 0b1011;
const USB_ACK: u8 = 0b0010;
const USB_NAK: u8 = 0b1010;
const USB_STALL: u8 = 0b1110;

/// A PID and its check nibble.
fn usb_pid(pid: u8) -> u8 {
    pid | (!pid & 0xF) << 4
}

fn usb_token(pid: u8, addr: u8, endp: u8) -> Vec<u8> {
    let field = u64::from(addr & 0x7F) | u64::from(endp & 0xF) << 7;
    let crc = usb_crc5(&lsb_bits(field, 11));
    let word = field | u64::from(crc) << 11;
    let [lo, hi, ..] = word.to_le_bytes();
    vec![usb_pid(pid), lo, hi]
}

fn usb_data(pid: u8, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![usb_pid(pid)];
    packet.extend_from_slice(payload);
    packet.extend_from_slice(&usb_crc16(payload).to_le_bytes());
    packet
}

/// A line state on the D+ / D- pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsbLine {
    J,
    K,
    Se0,
}

impl UsbLine {
    fn pins(self) -> (bool, bool) {
        match self {
            UsbLine::J => (true, false),
            UsbLine::K => (false, true),
            UsbLine::Se0 => (false, false),
        }
    }
}

/// A packet as the line carries it, one state per bit time: SYNC, the
/// bytes least significant bit first, a zero stuffed after every six
/// ones counted from SYNC on, NRZI from idle J, and the EOP. `stuff`
/// false leaves the stuffing out, which is how a broken packet is made.
fn usb_line(packet: &[u8], stuff: bool) -> Vec<UsbLine> {
    let mut bits = lsb_bits(0x80, 8);
    for byte in packet {
        bits.extend(lsb_bits(u64::from(*byte), 8));
    }
    let mut stuffed = Vec::new();
    let mut ones = 0;
    for b in bits {
        stuffed.push(b);
        ones = if b == 1 { ones + 1 } else { 0 };
        if stuff && ones == 6 {
            stuffed.push(0);
            ones = 0;
        }
    }
    let mut level = UsbLine::J;
    let mut line = Vec::new();
    for b in stuffed {
        if b == 0 {
            level = if level == UsbLine::J {
                UsbLine::K
            } else {
                UsbLine::J
            };
        }
        line.push(level);
    }
    line.extend([UsbLine::Se0, UsbLine::Se0, UsbLine::J]);
    line
}

/// What the host heard back.
#[derive(Debug, PartialEq, Eq)]
enum UsbReply {
    Handshake(u8),
    Data(u8, Vec<u8>),
    Nothing,
}

/// A USB host on the other end of the pair: it sends real packets —
/// NRZI, stuffed, with their CRCs — a bit every four cycles of the
/// device's 48 MHz, stretched or shortened now and then as a host
/// clock a little off the device's would, and it decodes what the
/// device sends back the way a host does, checking the SYNC field, the
/// stuffing, the EOP, the PID check nibble and the CRC16, and measuring
/// how long the device took to answer.
struct UsbHost<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    dp_i: NetHandle,
    dn_i: NetHandle,
    dp_o: NetHandle,
    dn_o: NetHandle,
    oe: NetHandle,
    address: NetHandle,
    configured: NetHandle,
    usb_reset: NetHandle,
    /// Every how many bits the host's bit is a cycle long or short: a
    /// positive number stretches, a negative one shortens, zero never.
    drift: i32,
    bits_sent: u64,
    /// Turnaround gaps the device took before answering, in cycles.
    gaps: Vec<u64>,
    problems: Vec<String>,
}

impl<'d> UsbHost<'d> {
    fn new(design: &'d Design, drift: i32) -> UsbHost<'d> {
        let sim = simulate(design, "usb_device_fs");
        let pin = |n: &str| top_net(&sim, n);
        let mut host = UsbHost {
            clk: pin("clk48"),
            dp_i: pin("usb_dp_i"),
            dn_i: pin("usb_dn_i"),
            dp_o: pin("usb_dp_o"),
            dn_o: pin("usb_dn_o"),
            oe: pin("usb_oe"),
            address: pin("address"),
            configured: pin("configured"),
            usb_reset: pin("usb_reset"),
            drift,
            bits_sent: 0,
            gaps: Vec::new(),
            problems: Vec::new(),
            sim,
        };
        let rst_n = top_net(&host.sim, "rst_n");
        host.set_line(UsbLine::J);
        let clk = host.clk;
        reset(&mut host.sim, clk, rst_n);
        assert!(
            high(&host.sim, top_net(&host.sim, "usb_dp_pu")),
            "the device asks for its D+ pull-up"
        );
        host
    }

    fn set_line(&mut self, line: UsbLine) {
        let (dp, dn) = line.pins();
        self.sim.set(self.dp_i, bit(dp));
        self.sim.set(self.dn_i, bit(dn));
    }

    /// One cycle with the host driving `line`; the device must not
    /// drive at the same time.
    fn host_cycle(&mut self, line: UsbLine) {
        self.set_line(line);
        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);
        if high(&self.sim, self.oe) {
            self.problems
                .push("the device drives the pair while the host does".into());
        }
    }

    /// The cycles the host's next bit lasts.
    fn bit_cycles(&mut self) -> u32 {
        self.bits_sent += 1;
        let every = u64::from(self.drift.unsigned_abs());
        if every != 0 && self.bits_sent.is_multiple_of(every) {
            if self.drift > 0 { 5 } else { 3 }
        } else {
            4
        }
    }

    fn send_line(&mut self, line: &[UsbLine]) {
        for state in line {
            for _ in 0..self.bit_cycles() {
                self.host_cycle(*state);
            }
        }
        self.set_line(UsbLine::J);
    }

    fn send(&mut self, packet: &[u8]) {
        let line = usb_line(packet, true);
        self.send_line(&line);
    }

    /// Idles the bus for `bits` bit times, the gap a host leaves between
    /// its own packets.
    fn idle(&mut self, bits: u32) {
        for _ in 0..4 * bits {
            self.host_cycle(UsbLine::J);
        }
    }

    /// Waits up to eighteen bit times — the host's turnaround timeout —
    /// for the device to answer, and decodes what it sends.
    fn receive(&mut self) -> UsbReply {
        let clk = self.clk;
        let mut waited = 0u64;
        self.set_line(UsbLine::J);
        while !high(&self.sim, self.oe) {
            if waited > 18 * 4 {
                return UsbReply::Nothing;
            }
            cycle(&mut self.sim, clk, HALF);
            waited += 1;
        }
        self.gaps.push(waited);

        // Record the pair for as long as the device drives it; the bus
        // is the device's while it does, so the device's own input sees
        // it too.
        let mut states = Vec::new();
        while high(&self.sim, self.oe) {
            let dp = high(&self.sim, self.dp_o);
            let dn = high(&self.sim, self.dn_o);
            let state = match (dp, dn) {
                (true, false) => UsbLine::J,
                (false, true) => UsbLine::K,
                (false, false) => UsbLine::Se0,
                _ => {
                    self.problems.push("the device drove SE1".into());
                    UsbLine::Se0
                }
            };
            states.push(state);
            self.sim.set(self.dp_i, bit(dp));
            self.sim.set(self.dn_i, bit(dn));
            cycle(&mut self.sim, clk, HALF);
            if states.len() > 4 * 200 {
                self.problems
                    .push("the device never let go of the bus".into());
                break;
            }
        }
        self.set_line(UsbLine::J);
        match self.decode(&states) {
            Ok(reply) => reply,
            Err(why) => {
                self.problems.push(why);
                UsbReply::Nothing
            }
        }
    }

    /// A packet from the line states the device drove, one per cycle,
    /// sampled in the middle of each four-cycle bit.
    fn decode(&self, states: &[UsbLine]) -> Result<UsbReply, String> {
        if !states.len().is_multiple_of(4) {
            return Err(format!(
                "the device drove {} cycles, not whole bits",
                states.len()
            ));
        }
        let symbols: Vec<UsbLine> = states.iter().skip(2).step_by(4).copied().collect();
        let n = symbols.len();
        if n < 8 + 8 + 3 {
            return Err(format!("{n} bit times is too short for a packet"));
        }
        if symbols[n - 3..] != [UsbLine::Se0, UsbLine::Se0, UsbLine::J] {
            return Err(format!(
                "the packet does not end SE0 SE0 J: {:?}",
                &symbols[n - 3..]
            ));
        }
        let mut level = UsbLine::J;
        let mut raw = Vec::new();
        for s in &symbols[..n - 3] {
            if *s == UsbLine::Se0 {
                return Err("SE0 in the middle of a packet".into());
            }
            raw.push(u8::from(*s == level));
            level = *s;
        }
        if raw[..8] != [0, 0, 0, 0, 0, 0, 0, 1] {
            return Err(format!("the SYNC field is {:?}", &raw[..8]));
        }
        // Unstuff, counting ones from SYNC on.
        let mut bits = Vec::new();
        let mut ones = 0;
        let mut skip = false;
        for (i, b) in raw.iter().enumerate() {
            if skip {
                if *b != 0 {
                    return Err(format!("bit {i} should be a stuffed zero"));
                }
                skip = false;
                ones = 0;
                continue;
            }
            if i >= 8 {
                bits.push(*b);
            }
            ones = if *b == 1 { ones + 1 } else { 0 };
            if ones == 6 {
                skip = true;
            }
        }
        if skip {
            return Err("the stuffed zero after the last six ones is missing".into());
        }
        if !bits.len().is_multiple_of(8) {
            return Err(format!("{} bits is not whole bytes", bits.len()));
        }
        let bytes: Vec<u8> = bits
            .chunks(8)
            .map(|c| c.iter().enumerate().fold(0u8, |a, (i, b)| a | b << i))
            .collect();
        let pid = bytes[0] & 0xF;
        if bytes[0] >> 4 != !pid & 0xF {
            return Err(format!("PID {:#04x} fails its check nibble", bytes[0]));
        }
        match pid {
            USB_ACK | USB_NAK | USB_STALL if bytes.len() == 1 => Ok(UsbReply::Handshake(pid)),
            USB_DATA0 | USB_DATA1 if bytes.len() >= 3 => {
                let payload = &bytes[1..bytes.len() - 2];
                let crc = u16::from_le_bytes([bytes[bytes.len() - 2], bytes[bytes.len() - 1]]);
                if crc != usb_crc16(payload) {
                    return Err(format!("CRC16 {crc:#06x} over {payload:02x?}"));
                }
                Ok(UsbReply::Data(pid, payload.to_vec()))
            }
            _ => Err(format!("an unexpected packet {bytes:02x?}")),
        }
    }

    /// SE0 for ten microseconds: a bus reset.
    fn bus_reset(&mut self) {
        let mut seen = false;
        for _ in 0..480 {
            self.host_cycle(UsbLine::Se0);
            seen |= high(&self.sim, self.usb_reset);
        }
        assert!(seen, "the device saw the bus reset");
        self.idle(50);
    }

    /// SETUP and its eight bytes; the device must acknowledge.
    fn setup(&mut self, addr: u8, request: [u8; 8]) -> UsbReply {
        self.send(&usb_token(USB_SETUP, addr, 0));
        self.idle(3);
        self.send(&usb_data(USB_DATA0, &request));
        self.receive()
    }

    fn in_token(&mut self, addr: u8) -> UsbReply {
        self.send(&usb_token(USB_IN, addr, 0));
        self.receive()
    }

    fn ack(&mut self) {
        self.idle(2);
        self.send(&[usb_pid(USB_ACK)]);
        self.idle(4);
    }

    /// A whole control read: SETUP, IN until a short packet or wLength,
    /// each acknowledged, and the zero-length OUT of the status stage.
    fn control_read(&mut self, addr: u8, request: [u8; 8]) -> Result<Vec<u8>, UsbReply> {
        let reply = self.setup(addr, request);
        if reply != UsbReply::Handshake(USB_ACK) {
            return Err(reply);
        }
        self.idle(4);
        let length = usize::from(u16::from_le_bytes([request[6], request[7]]));
        let mut got = Vec::new();
        let mut toggle = USB_DATA1;
        loop {
            match self.in_token(addr) {
                UsbReply::Data(pid, payload) => {
                    assert_eq!(pid, toggle, "the data stage alternates DATA1, DATA0, ...");
                    let short = payload.len() < 8;
                    got.extend(payload);
                    self.ack();
                    toggle = if toggle == USB_DATA1 {
                        USB_DATA0
                    } else {
                        USB_DATA1
                    };
                    if short || got.len() >= length {
                        break;
                    }
                }
                other => return Err(other),
            }
        }
        self.send(&usb_token(USB_OUT, addr, 0));
        self.idle(3);
        self.send(&usb_data(USB_DATA1, &[]));
        let status = self.receive();
        assert_eq!(
            status,
            UsbReply::Handshake(USB_ACK),
            "the status stage of a read"
        );
        self.idle(4);
        Ok(got)
    }

    /// A control transfer with no data stage: SETUP, then a zero-length
    /// IN for the status, acknowledged.
    fn control_write(&mut self, addr: u8, request: [u8; 8]) -> UsbReply {
        let reply = self.setup(addr, request);
        if reply != UsbReply::Handshake(USB_ACK) {
            return reply;
        }
        self.idle(4);
        let status = self.in_token(addr);
        if status == UsbReply::Data(USB_DATA1, Vec::new()) {
            self.ack();
        }
        status
    }

    fn assert_clean(&self) {
        assert!(
            self.problems.is_empty(),
            "the host saw the device break the protocol:\n  {}",
            self.problems.join("\n  ")
        );
        // Every answer came between two and six and a half bit times
        // after the host's EOP.
        for gap in &self.gaps {
            assert!(
                (8..=26).contains(gap),
                "the device answered after {gap} cycles, outside 2 to 6.5 bit times"
            );
        }
    }
}

const GET_DEVICE_DESCRIPTOR: [u8; 8] = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00];

fn get_descriptor(kind: u8, length: u16) -> [u8; 8] {
    let [lo, hi] = length.to_le_bytes();
    [0x80, 0x06, 0x00, kind, 0x00, 0x00, lo, hi]
}

fn set_address(addr: u8) -> [u8; 8] {
    [0x00, 0x05, addr, 0x00, 0x00, 0x00, 0x00, 0x00]
}

/// The device descriptor the block should send, written from the
/// specification's layout rather than from the block's table.
fn expected_device_descriptor(vid: u16, pid: u16) -> Vec<u8> {
    let mut d = vec![18, 1, 0x00, 0x02, 0xFF, 0x00, 0x00, 8];
    d.extend_from_slice(&vid.to_le_bytes());
    d.extend_from_slice(&pid.to_le_bytes());
    d.extend_from_slice(&[0x00, 0x01, 0, 0, 0, 1]);
    d
}

fn usb_design() -> Design {
    design_of(
        "usb_device_fs",
        "usb_device_fs",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

#[test]
fn usb_crcs_match_the_catalogue_and_the_wire() {
    // The host model's own arithmetic, held to published values before
    // anything is held to it: the CRC catalogue's check values over
    // "123456789", and the bytes every USB analyser shows for a SETUP to
    // address 0 and for the first GET_DESCRIPTOR of an enumeration.
    let check: Vec<u8> = b"123456789"
        .iter()
        .flat_map(|b| lsb_bits(u64::from(*b), 8))
        .collect();
    assert_eq!(usb_crc5(&check), 0x19, "CRC-5/USB check value");
    assert_eq!(usb_crc16(b"123456789"), 0xB4C8, "CRC-16/USB check value");
    assert_eq!(usb_token(USB_SETUP, 0, 0), vec![0x2D, 0x00, 0x10]);
    assert_eq!(usb_token(USB_IN, 0, 0), vec![0x69, 0x00, 0x10]);
    assert_eq!(
        usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR)[9..],
        [0xDD, 0x94]
    );
}

fn usb_enumerate(drift: i32) {
    let design = usb_design();
    let mut host = UsbHost::new(&design, drift);
    host.bus_reset();
    assert_eq!(get_u64(&host.sim, host.address), 0);

    // What a host does first: the device descriptor at address 0, with a
    // wLength of 64 whatever the descriptor's size.
    let device = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("the device descriptor");
    assert_eq!(device, expected_device_descriptor(0x1209, 0x0001));

    // SET_ADDRESS takes effect after its status stage, not before.
    let status = host.control_write(0, set_address(9));
    assert_eq!(status, UsbReply::Data(USB_DATA1, Vec::new()));
    assert_eq!(get_u64(&host.sim, host.address), 9, "the new address");

    // Address 0 is not the device's any more.
    host.idle(10);
    assert_eq!(host.in_token(0), UsbReply::Nothing, "IN to the old address");
    host.idle(10);

    // The descriptors again at the new address: short reads, a read of
    // exactly two packets, and a read longer than the descriptor with
    // runs of ones in the request to exercise the device's unstuffing.
    let first8 = host
        .control_read(9, get_descriptor(1, 8))
        .expect("eight bytes");
    assert_eq!(first8, expected_device_descriptor(0x1209, 0x0001)[..8]);
    let config9 = host
        .control_read(9, get_descriptor(2, 9))
        .expect("the configuration header");
    assert_eq!(config9, [9, 2, 18, 0, 1, 1, 0, 0x80, 50]);
    let config = host
        .control_read(9, [0x80, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF])
        .expect("the whole configuration");
    assert_eq!(
        config,
        [9, 2, 18, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 0, 0xFF, 0, 0, 0],
        "configuration and interface, 18 bytes in 8 + 8 + 2"
    );
    let sixteen = host
        .control_read(9, get_descriptor(1, 16))
        .expect("two whole packets");
    assert_eq!(sixteen.len(), 16, "wLength ends the data stage");

    // SET_CONFIGURATION 1, then 0.
    assert!(!high(&host.sim, host.configured));
    let status = host.control_write(9, [0x00, 0x09, 0x01, 0, 0, 0, 0, 0]);
    assert_eq!(status, UsbReply::Data(USB_DATA1, Vec::new()));
    assert!(high(&host.sim, host.configured), "configured");
    host.control_write(9, [0x00, 0x09, 0x00, 0, 0, 0, 0, 0]);
    assert!(!high(&host.sim, host.configured), "and back");

    // A bus reset forgets the address.
    host.bus_reset();
    assert_eq!(get_u64(&host.sim, host.address), 0);
    let again = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("enumerable again");
    assert_eq!(again.len(), 18);

    host.assert_clean();
}

#[test]
fn usb_device_fs_enumerates_with_a_host_on_its_own_clock() {
    usb_enumerate(0);
}

#[test]
fn usb_device_fs_tracks_a_host_clock_that_is_slow_or_fast() {
    // One bit in sixty-four a cycle long or a cycle short: a host clock
    // 0.4 % off the device's, more than the 0.25 % the specification
    // allows between the two, and the device must stay locked.
    usb_enumerate(64);
    usb_enumerate(-64);
}

#[test]
fn usb_device_fs_ignores_bad_packets_and_stalls_what_it_cannot_do() {
    let design = usb_design();
    let mut host = UsbHost::new(&design, 0);
    host.bus_reset();

    // A SETUP whose data has a wrong CRC16 is not acknowledged.
    host.send(&usb_token(USB_SETUP, 0, 0));
    host.idle(3);
    let mut bad = usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR);
    bad[9] ^= 0x01;
    host.send(&bad);
    assert_eq!(host.receive(), UsbReply::Nothing, "a bad CRC16 gets no ACK");
    host.idle(20);

    // Nor is one whose token has a wrong CRC5: the data that follows
    // belongs to nothing.
    let mut token = usb_token(USB_SETUP, 0, 0);
    token[2] ^= 0x80;
    host.send(&token);
    host.idle(3);
    host.send(&usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR));
    assert_eq!(host.receive(), UsbReply::Nothing, "a bad CRC5 gets no ACK");
    host.idle(20);

    // A PID whose check nibble is wrong is ignored.
    let mut token = usb_token(USB_SETUP, 0, 0);
    token[0] ^= 0x10;
    host.send(&token);
    host.idle(3);
    host.send(&usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR));
    assert_eq!(host.receive(), UsbReply::Nothing, "a bad PID gets no ACK");
    host.idle(20);

    // A packet with its stuffed zeros left out breaks the stuffing
    // rule, and is ignored too. 0xFF payload bytes make sure it has runs
    // of six ones to break.
    host.send(&usb_token(USB_SETUP, 0, 0));
    host.idle(3);
    let line = usb_line(
        &usb_data(USB_DATA0, &[0x80, 0x06, 0x00, 0x01, 0xFF, 0xFF, 0x40, 0x00]),
        false,
    );
    host.send_line(&line);
    assert_eq!(
        host.receive(),
        UsbReply::Nothing,
        "a stuffing error gets no ACK"
    );
    host.idle(20);

    // A token for another endpoint is not for the control endpoint.
    host.send(&usb_token(USB_SETUP, 0, 1));
    host.idle(3);
    host.send(&usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR));
    assert_eq!(
        host.receive(),
        UsbReply::Nothing,
        "endpoint 1 does not exist"
    );
    host.idle(20);

    // Requests it does not do are acknowledged and then stalled:
    // GET_STATUS, and a string descriptor.
    for request in [
        [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00],
        [0x80, 0x06, 0x01, 0x03, 0x09, 0x04, 0xFF, 0x00],
    ] {
        assert_eq!(host.setup(0, request), UsbReply::Handshake(USB_ACK));
        host.idle(4);
        assert_eq!(host.in_token(0), UsbReply::Handshake(USB_STALL));
        host.idle(10);
    }

    // And after all of that, the next SETUP is served as if none of it
    // had happened.
    let device = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("the device recovers");
    assert_eq!(device, expected_device_descriptor(0x1209, 0x0001));
    host.assert_clean();
}

#[test]
fn usb_device_fs_sends_again_what_the_host_did_not_acknowledge() {
    let design = usb_design();
    let mut host = UsbHost::new(&design, 0);
    host.bus_reset();
    assert_eq!(
        host.setup(0, GET_DEVICE_DESCRIPTOR),
        UsbReply::Handshake(USB_ACK)
    );
    host.idle(4);
    // The first packet, and the host says nothing: as if it were lost.
    let first = host.in_token(0);
    host.idle(20);
    // Asked again, the device sends the same packet with the same toggle.
    let again = host.in_token(0);
    assert_eq!(first, again, "the same packet again");
    let UsbReply::Data(pid, payload) = &again else {
        panic!("data expected, got {again:?}");
    };
    assert_eq!(*pid, USB_DATA1);
    assert_eq!(payload[..], expected_device_descriptor(0x1209, 0x0001)[..8]);
    host.ack();
    // Acknowledged, it moves on to the next eight bytes and DATA0.
    let next = host.in_token(0);
    assert_eq!(
        next,
        UsbReply::Data(
            USB_DATA0,
            expected_device_descriptor(0x1209, 0x0001)[8..16].to_vec()
        )
    );
    host.assert_clean();
}

/// Everything runs on the one 48 MHz clock; the pins come in through
/// two flip-flops each, which are not a crossing between clocks.
#[test]
fn usb_device_fs_is_one_clock_domain() {
    let kinds = crossings(
        "usb_device_fs",
        "usb_device_fs",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    );
    assert!(kinds.is_empty(), "nothing should cross: {kinds:?}");
}

/// From a 12 MHz board clock both families' PLLs make 48 MHz exactly.
#[test]
fn usb_device_fs_pll_takes_48_mhz_from_the_pll() {
    let variant = VARIANTS
        .iter()
        .find(|v| v.top == "usb_device_fs_pll")
        .expect("usb_device_fs_pll is measured");
    for (device_name, primitive) in [
        ("ice40-hx1k-tq144", "SB_PLL40_CORE"),
        ("ecp5-45f-CABGA381", "EHXPLLL"),
    ] {
        let (mut design, id) = flattened(variant.package, variant.top, variant.params);
        let device = fpga::target(device_name).expect("a built-in device");
        let mut diags = Diagnostics::new();
        let mut map = SourceMap::new();
        let rcf = board_constraints(variant);
        let file = map.add("board.rcf", rcf).expect("fits");
        let mut constraints = Constraints::parse(rcf, file, &mut diags);
        constraints.merge_attrs(&design, id, &mut diags);
        let report = fpga::synthesize_for(
            &mut design,
            id,
            device,
            &constraints,
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow runs");
        assert!(
            !diags.has_errors(),
            "{device_name}:\n{}",
            diags.render(&map)
        );
        let pll = report
            .primitives
            .plls
            .first()
            .unwrap_or_else(|| panic!("{device_name}: no PLL"));
        assert_eq!(pll.primitive, primitive);
        assert_eq!(pll.net, "clk48");
        assert_eq!(pll.source, "clk_ref");
        assert_eq!(pll.input_hz, 12_000_000);
        assert_eq!(pll.achieved_hz, 48_000_000, "{device_name}: exactly 48 MHz");
    }
}

// ---------------------------------------------------------------------------
// Two more compiler gaps the larger blocks ran into
// ---------------------------------------------------------------------------

/// Elaborates one module written out here rather than shipped in `ip/`,
/// which is what a minimal reproduction wants: the whole of it visible
/// next to the assertion about it.
fn design_of_text(name: &str, text: &str) -> Design {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let id = map.add(name.to_owned(), text).expect("the source fits");
    let file = parse_source(
        &mut map,
        id,
        Dialect::Verilog2005,
        &mut NoIncludes,
        &mut diags,
    );
    assert!(!diags.has_errors(), "{name}:\n{}", diags.render(&map));
    let options = ElabOptions::new(Dialect::Verilog2005);
    let design = elaborate(&[&file], &options, &mut diags).expect("a design");
    assert!(!diags.has_errors(), "{name}:\n{}", diags.render(&map));
    design
}

/// `S0013`: a register with an asynchronous reset that is not held
/// constant in the reset branch.
const FUNCTION_LOCAL_GAP: &str = "S0013";

/// Calling a Verilog function from a process with an asynchronous reset
/// must not report the function's own locals as registers that failed to
/// get one.
///
/// It used to. The netlist was always right — the call is inlined into
/// pure combinational logic — but flip-flop inference looked at every
/// argument and every local of the function and complained about each,
/// so a block that wanted a function, which is the readable way to write
/// a CRC step or a decode table, could not be synthesised without a
/// warning, and `blocks_synthesise_cleanly` above insists on none.
///
/// The fix was to warn only about nets the process assigns
/// non-blockingly, since a blocking assignment inside a clocked block is
/// a temporary and needs no reset. This test holds that fix.
#[test]
fn function_locals_are_not_reported_as_unreset_registers() {
    const REPRO: &str = "\
module fnwarn (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] d,
    output reg  [7:0] q
);
    function [7:0] inc;
        input [7:0] v;
        begin
            inc = v + 8'd1;
        end
    endfunction

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 8'd0;
        else        q <= inc(d);
    end
endmodule
";
    let mut design = design_of_text("fnwarn.v", REPRO);
    let id = design.top.expect("a top");
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    let complaints: Vec<&str> = diags
        .iter()
        .filter(|d| d.code == Some(FUNCTION_LOCAL_GAP))
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        complaints.is_empty(),
        "a function call in an asynchronously reset process warns again: {complaints:?}"
    );
    for message in &complaints {
        assert!(
            message.contains("inc$"),
            "{FUNCTION_LOCAL_GAP} should be about the function's own names: {message}"
        );
    }

    // The netlist itself is correct, which is what makes this a
    // diagnostic problem rather than a synthesis one: one adder, one
    // flip-flop, and no storage for the function at all.
    let module = design.module(id);
    let adders = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Add))
        .count();
    let flops = module
        .cells
        .iter()
        .filter(|(_, c)| matches!(c.kind, CellKind::Dff { .. }))
        .count();
    assert_eq!(
        (adders, flops),
        (1, 1),
        "the function was inlined correctly"
    );

    // The same function, called from a continuous assignment, is silent.
    const WORKAROUND: &str = "\
module fnok (
    input  wire       clk,
    input  wire       rst_n,
    input  wire [7:0] d,
    output reg  [7:0] q
);
    function [7:0] inc;
        input [7:0] v;
        begin
            inc = v + 8'd1;
        end
    endfunction

    wire [7:0] inc_d = inc(d);

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) q <= 8'd0;
        else        q <= inc_d;
    end
endmodule
";
    let mut design = design_of_text("fnok.v", WORKAROUND);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert_eq!(
        diags.iter().count(),
        0,
        "the wire form should be clean:\n{}",
        diags.render(&SourceMap::new())
    );
}

/// `F0300`: a memory that does not fit the device's block RAM.
const BRAM_SHAPE_GAP: &str = "F0300";

/// A register file with two read ports and one write port is duplicated
/// across block RAMs.
///
/// `rv32i` holds x1..x31 in one array with two read ports and one write
/// port. A `DP16KD` has two physical ports and each of them is *either*
/// a read or a write, so one block cannot serve three accesses; the
/// mapper used to say so with
///
/// ```text
/// `DP16KD` has 2 read and 2 write port(s), the memory needs 2 and 1
/// ```
///
/// where every comparison held and it was still a refusal, because the
/// constraint it left out was the one that applied. That wording was
/// fixed first; what the mapper does about it is fixed here.
///
/// The answer a real flow gives is duplication: one copy of the contents
/// per read port, each with one read and one write port of its own, all
/// written together from the one writer. With `REGFILE_BRAM = 1` the two
/// reads are clocked, which is the shape a block RAM has, and the file
/// now maps onto four `DP16KD` — two copies, two blocks wide each, since
/// thirty-two bits do not fit one block's eighteen.
#[test]
fn a_two_read_port_register_file_is_duplicated_across_block_rams() {
    let variant = VARIANTS
        .iter()
        .find(|v| v.package == "rv32i" && v.params == [("REGFILE_BRAM", "1")])
        .expect("rv32i with a clocked register file is in the catalogue");
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let device = fpga::target("ecp5-45f-CABGA381").expect("the ECP5 device");
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(
        &mut design,
        id,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");

    let mapping = report
        .primitives
        .block_rams
        .iter()
        .find(|m| m.memory.as_str() == "regs")
        .expect("the register file is a block RAM now");
    assert_eq!(mapping.primitive, "DP16KD");
    assert_eq!(mapping.copies, 2, "one copy per read port");
    assert_eq!(mapping.blocks(), 4);
    assert_eq!(report.count("DP16KD"), 4);
    assert!(
        report
            .primitives
            .bram_fallbacks
            .iter()
            .all(|f| f.memory.as_str() != "regs"),
        "the register file should not fall back at all"
    );
    assert!(
        !diags.iter().any(|d| d.code == Some(BRAM_SHAPE_GAP)),
        "nothing about the register file is refused now"
    );
    assert!(fpga::check_nextpnr_json(&design, id, device, &Constraints::new()).is_empty());
}

/// The asynchronous register file is *not* given a block RAM, and the
/// reason says so.
///
/// With `REGFILE_BRAM = 0` the same array is read combinationally, which
/// no block RAM does — its own header says that variant wants
/// distributed RAM or flip-flops. Duplication does not change that, so
/// the memory takes the logic fallback and the refusal names the
/// property that decided it.
#[test]
fn an_asynchronous_register_file_takes_the_logic_fallback() {
    let variant = VARIANTS
        .iter()
        .find(|v| v.package == "rv32i" && v.params == [("REGFILE_BRAM", "0")])
        .expect("rv32i with an asynchronous register file is in the catalogue");
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let device = fpga::target("ecp5-45f-CABGA381").expect("the ECP5 device");
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(
        &mut design,
        id,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");

    let fallback = report
        .primitives
        .bram_fallbacks
        .iter()
        .find(|f| f.memory.as_str() == "regs")
        .expect("the register file falls back");
    assert!(
        fallback.reason.contains("asynchronous read port"),
        "the reason has changed: {}",
        fallback.reason
    );
    // It is not a *warning*: reading a small array combinationally is a
    // design decision, not a mistake, and `ram_style = "block"` is what
    // turns it into one (`F0300`). The report is where it is recorded.
    assert!(!diags.iter().any(|d| d.code == Some(BRAM_SHAPE_GAP)));
    // And the fallback it names is performed: the ECP5's distributed RAM.
    assert!(fallback.built);
    assert_eq!(fallback.style, "distributed LUT RAM");
    assert_eq!(fallback.primitive.as_deref(), Some("TRELLIS_DPR16X4"));
    assert!(fpga::check_nextpnr_json(&design, id, device, &Constraints::new()).is_empty());
}

/// Builds a one-package project entirely from text, the package in
/// `pkg/`, and returns the module names the build produced and the
/// diagnostics it rendered.
fn build_project_from_text(manifest: &str, source: &str, top: &str) -> (Vec<String>, String) {
    let files: BTreeMap<String, String> = [
        ("pkg/reticle.ip".to_owned(), manifest.to_owned()),
        ("pkg/rtl/pkg.v".to_owned(), source.to_owned()),
    ]
    .into_iter()
    .collect();
    let project_text = format!("name gap_check\ntop {top}\n\ndepends pkg * path pkg\n");
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let project = ip::load_project(&mut map, "reticle.proj", &project_text, &mut diags)
        .expect("the project parses");
    let mut provider = PathProvider::new(".", |path: &str| files.get(path).cloned());
    let mut resolved = ip::resolve(map, &project, &mut provider, &mut diags);
    assert!(
        resolved.is_complete(),
        "{}",
        diags.render(resolved.source_map())
    );
    let build = ip::elaborate(&project, &mut resolved, &mut diags);
    let names = build
        .design
        .map(|d| {
            d.modules
                .iter()
                .map(|(_, m)| m.name.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    (names, diags.render(resolved.source_map()))
}

/// The code `ip::elaborate` reports for a project top it cannot find.
const PROJECT_TOP_GAP: &str = "P0401";

/// A project's top that its own sources also instantiate with a
/// parameter override must keep its plain name.
///
/// `ip::elaborate` used to leave the Verilog elaborator to pick its own
/// root, which is the module nothing instantiates. When the project's top
/// was also instantiated elsewhere with an override, it then existed only
/// as the renamed variant (`leaf$W_1`) and the build failed with P0401.
/// It now passes the project's top to the frontend that defines it. This
/// test holds the fix, with and without the override.
#[test]
fn a_project_top_that_is_also_instantiated_with_an_override_keeps_its_name() {
    let manifest =
        "name pkg\nversion 1.0.0\nlicense MIT\ndescription \"gap\"\ntop leaf\nsource rtl/pkg.v\n";
    let source = "\
module leaf #(parameter W = 1) (input wire [W-1:0] a, output wire [W-1:0] y);
    assign y = ~a;
endmodule
module wrap #(parameter W = 1) (input wire [W-1:0] a, output wire [W-1:0] y);
    leaf #(.W(W)) u (.a(a), .y(y));
endmodule
";
    for source in [
        source.to_string(),
        source.replace("leaf #(.W(W)) u", "leaf u"),
    ] {
        let (names, rendered) = build_project_from_text(manifest, &source, "leaf");
        assert!(
            !rendered.contains(PROJECT_TOP_GAP),
            "the project's top was lost:\n{rendered}"
        );
        assert!(
            names.iter().any(|n| n == "leaf"),
            "`leaf` should keep its plain name: {names:?}"
        );
    }
}

/// The pixel rate is an enable, not a clock, so the whole transmitter is
/// one domain and nothing crosses: the serialisers are loaded by the
/// clock that shifts them.
#[test]
fn dvi_tx_is_one_clock_domain() {
    let kinds = crossings("dvi_tx", "dvi_tx", &[("MODE", "0")]);
    assert!(
        kinds.is_empty(),
        "dvi_tx should have no crossing: {kinds:?}"
    );
}

/// The wrapper's five-times clock comes out of each family's PLL, fed
/// from the board clock, and every TMDS lane leaves through a
/// double-data-rate output register on it.
#[test]
fn dvi_tx_pll_takes_its_clock_from_the_pll_and_its_lanes_through_ddr() {
    let variant = VARIANTS
        .iter()
        .find(|v| v.top == "dvi_tx_pll")
        .expect("dvi_tx_pll is measured");
    for (device_name, primitive, ddr) in [
        ("ice40-hx1k-tq144", "SB_PLL40_CORE", "SB_IO"),
        ("ecp5-45f-CABGA381", "EHXPLLL", "ODDRX1F"),
    ] {
        let (mut design, id) = flattened(variant.package, variant.top, variant.params);
        let device = fpga::target(device_name).expect("a built-in device");
        let mut diags = Diagnostics::new();
        let mut map = SourceMap::new();
        let rcf = board_constraints(variant);
        let file = map.add("board.rcf", rcf).expect("fits");
        let mut constraints = Constraints::parse(rcf, file, &mut diags);
        constraints.merge_attrs(&design, id, &mut diags);
        let report = fpga::synthesize_for(
            &mut design,
            id,
            device,
            &constraints,
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow runs");
        assert!(
            !diags.has_errors(),
            "{device_name}:\n{}",
            diags.render(&map)
        );

        let pll = report
            .primitives
            .plls
            .first()
            .unwrap_or_else(|| panic!("{device_name}: no PLL was built"));
        assert_eq!(pll.primitive, primitive);
        assert_eq!(pll.net, "clk_x5");
        assert_eq!(pll.source, "clk_ref");
        assert_eq!(pll.input_hz, 25_000_000);
        assert_eq!(pll.requested_hz, 126_000_000);
        assert!(
            pll.error_ppm().abs() < 10_000.0,
            "{device_name}: {} Hz is more than 1 % off",
            pll.achieved_hz
        );

        for lane in ["tmds_d0", "tmds_d1", "tmds_d2", "tmds_clk"] {
            let io = report
                .primitives
                .io_buffers
                .iter()
                .find(|b| b.port == lane)
                .unwrap_or_else(|| panic!("{device_name}: no buffer for {lane}"));
            assert_eq!(io.bits, 1, "{device_name}: {lane} is one pin");
            let (clock, register) = io.ddr.clone().expect("a DDR lane");
            assert_eq!(clock, "clk_x5", "{device_name}: {lane}");
            assert_eq!(register, ddr, "{device_name}: {lane}");
        }
    }
}

/// The code the FPGA flow warns with about an IO register or delay it
/// cannot build as asked.
const ZERO_DELAY_GAP: &str = "F0304";

/// A zero-step IO delay must build nothing and warn about nothing.
///
/// A parameterised block can only spell its delay as
/// `(* io_delay = TX_DELAY *)`, since an attribute cannot be made
/// conditional in Verilog, and zero is how it says "none". The mapper
/// used to take the zero literally: the ECP5 got a `DELAYG` set to
/// nothing, one primitive per pin for no effect, and the iCE40, which has
/// no delay element, reported `F0304` for a delay nobody asked for. The
/// netlist was correct either way; the cost and the warning were not.
/// This test holds the fix.
#[test]
fn a_zero_step_io_delay_builds_nothing() {
    let text = "\
module zerodelay (input wire clk, (* io_delay = 0 *) input wire d, output reg q);
    always @(posedge clk) q <= d;
endmodule
";
    for device_name in ["ecp5-45f-CABGA381", "ice40-hx1k-tq144"] {
        let mut design = design_of_text("zerodelay.v", text);
        let id = design.top.expect("a top");
        let device = fpga::target(device_name).expect("a built-in device");
        let mut diags = Diagnostics::new();
        let mut constraints = Constraints::new();
        constraints.merge_attrs(&design, id, &mut diags);
        let report = fpga::synthesize_for(
            &mut design,
            id,
            device,
            &constraints,
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow runs");
        let io = report
            .primitives
            .io_buffers
            .iter()
            .find(|b| b.port == "d")
            .expect("a buffer for d");
        assert!(
            io.delay.is_none(),
            "{device_name}: a zero-step delay built an element: {:?}",
            io.delay
        );
        let warning = diags
            .iter()
            .any(|d| d.code == Some(ZERO_DELAY_GAP) && d.message.contains("0-step"));
        assert!(
            !warning,
            "{device_name}: warned about a delay nobody asked for"
        );
    }
}
