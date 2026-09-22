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
//! Four tests here describe gaps in Reticle rather than in the blocks:
//! `ice40_flip_flops_still_refuse_an_active_low_reset`,
//! `small_memories_are_left_generic_after_the_fpga_flow`,
//! `function_locals_are_reported_as_unreset_registers` and
//! `a_two_read_port_register_file_is_declined_with_a_contradictory_reason`.
//! Each asserts that the gap is *still there*, so closing one fails the
//! test that names it and points at the paragraph in
//! `docs/ip-library.md` to delete. Writing real HDL is how they were
//! found, which is the argument for a first-party library in the first
//! place.
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
];

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

/// `F0310`: the device has no flip-flop of the shape a cell needs.
///
/// Every block in this library resets on `negedge rst_n`, which is the
/// convention the rest of this repository's IP uses and the one nearly
/// all real HDL uses. The iCE40 family's `SB_DFF*` primitives all reset
/// *high*, and `fpga::primitives` declines the mismatch instead of
/// inverting the reset net, so those flip-flops stay generic. That is a
/// gap in the primitive mapper, not in the blocks, and it is recorded
/// rather than worked around: `ice40_flip_flops_need_an_active_high_reset`
/// below pins it down, and the footprint table shows what it costs.
const RESET_POLARITY_GAP: &str = "F0310";

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
    let constraints = Constraints::new();
    let options = FpgaOptions::default();
    let mut diags = Diagnostics::new();
    let report = fpga::synthesize_for(&mut design, id, device, &constraints, &options, &mut diags)
        .unwrap_or_else(|e| {
            panic!(
                "{}.{} does not map for {label}: {e:?}",
                variant.package, variant.top
            )
        });
    let unexpected: Vec<String> = diags
        .iter()
        .filter(|d| d.severity >= Severity::Error && d.code != Some(RESET_POLARITY_GAP))
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

/// The iCE40 flip-flop mapping declines an active-low reset.
///
/// Pinned down here rather than described in prose, so that the day
/// `fpga::primitives` learns to invert a reset net this test fails and
/// says which document to update. Every `SB_DFF*` variant resets high;
/// the mapper matches polarity exactly instead of putting an inverter in
/// front, so `always @(posedge clk or negedge rst_n)` — the way nearly
/// all real HDL is written — leaves generic flip-flops behind.
#[test]
fn ice40_flip_flops_still_refuse_an_active_low_reset() {
    let variant = &VARIANTS[2]; // cdc_sync, two flops and nothing else
    let (mut design, id) = flattened(variant.package, variant.top, variant.params);
    let device = fpga::target("ice40-hx1k-tq144").expect("the iCE40 device");
    let mut diags = Diagnostics::new();
    let _ = fpga::synthesize_for(
        &mut design,
        id,
        device,
        &Constraints::new(),
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("the flow runs");
    let refusals: Vec<&str> = diags
        .iter()
        .filter(|d| d.code == Some(RESET_POLARITY_GAP))
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        !refusals.is_empty(),
        "the iCE40 flip-flop mapping now accepts an active-low reset: drop \
         RESET_POLARITY_GAP and refresh docs/ip-library.md"
    );
    for message in &refusals {
        assert!(
            message.contains("active-low"),
            "unexpected {RESET_POLARITY_GAP}: {message}"
        );
    }
}

/// A memory below the block-RAM threshold is reported as falling back to
/// logic, but nothing performs the fallback.
///
/// `fpga::primitives` records the decision and leaves the `$memrd` and
/// `$memwr` cells in place, and `fpga::check_nextpnr_json` — this
/// crate's own netlist checker — then says the netlist is unusable. The
/// two FIFOs are where the library meets it, since a 16 x 8 FIFO is 128
/// bits and the threshold is 256.
#[test]
fn small_memories_are_left_generic_after_the_fpga_flow() {
    let variant = &VARIANTS[0]; // fifo_sync, WIDTH=8 DEPTH=16
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
    assert!(
        report
            .primitives
            .bram_fallbacks
            .iter()
            .any(|f| f.reason.contains("threshold")),
        "the 128-bit memory should be below the block RAM threshold"
    );
    let problems = fpga::check_nextpnr_json(&design, id, device, &Constraints::new());
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("did not become")),
        "the fallback is performed now: drop this test and refresh \
         docs/ip-library.md"
    );
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
/// reports the function's own locals as registers that failed to get one.
///
/// The netlist is right — the call is inlined into pure combinational
/// logic, and `proc_lower` says so itself by turning the locals into
/// wires — but flip-flop inference looks at them first and complains
/// about every argument and every local of the function. A block that
/// wants a function, which is the readable way to write a CRC step or a
/// decode table, then cannot be synthesised without a warning, and
/// `blocks_synthesise_cleanly` above insists on none.
///
/// `eth_mac_rmii` works around it by calling `crc_step` from a continuous
/// assignment and using the wire inside the process. This test holds the
/// statement so that the day the pass stops counting function locals as
/// state, it fails and says which workaround to unwind.
#[test]
fn function_locals_are_reported_as_unreset_registers() {
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
        !complaints.is_empty(),
        "a function call in an asynchronously reset process no longer warns: \
         drop the `crc_step` wires in ip/eth_mac_rmii/rtl/eth_mac_rmii.v and \
         refresh docs/ip-library.md"
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

/// A register file with two read ports and one write port is declined
/// with a reason that reads as though it should have been accepted.
///
/// `rv32i` holds x1..x31 in one array with two asynchronous — or, with
/// REGFILE_BRAM, two clocked — read ports and one write port. On the
/// ECP5 that is turned down with
///
/// ```text
/// `DP16KD` has 2 read and 2 write port(s), the memory needs 2 and 1
/// ```
///
/// Both of those comparisons hold: two reads are wanted and two are
/// available, one write is wanted and two are available. The real
/// constraint is the one the sentence does not say — a `DP16KD` port is
/// *either* a read or a write, so two reads and a write want three ports
/// and the device has two. The mapping decision is right; the sentence
/// explaining it is not, and a user reading it has no way to work out
/// what to change.
///
/// The fix a real flow applies is to duplicate the memory: two block
/// RAMs holding the same contents, each with one read and one write port,
/// both written together. That is not done here either, so the register
/// file stays generic in the footprint table.
#[test]
fn a_two_read_port_register_file_is_declined_with_a_contradictory_reason() {
    let variant = VARIANTS
        .iter()
        .find(|v| v.package == "rv32i")
        .expect("rv32i is in the catalogue");
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

    let reason = report
        .primitives
        .bram_fallbacks
        .iter()
        .find(|f| f.memory.as_str() == "regs")
        .map(|f| f.reason.clone())
        .expect("the register file falls back");
    assert!(
        reason.contains("2 read and 2 write port(s)") && reason.contains("needs 2 and 1"),
        "the reason has changed: {reason}"
    );
    assert!(
        diags.iter().any(|d| d.code == Some(BRAM_SHAPE_GAP)),
        "the fallback should be reported as {BRAM_SHAPE_GAP}"
    );
    // And, as for the FIFOs, nothing performs the fallback it names.
    let problems = fpga::check_nextpnr_json(&design, id, device, &Constraints::new());
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("did not become")),
        "the register file is mapped now: drop this test and refresh \
         docs/ip-library.md"
    );
}
