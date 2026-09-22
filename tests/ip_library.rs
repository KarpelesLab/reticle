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
//! Two tests here describe gaps in Reticle rather than in the blocks:
//! `ice40_flip_flops_still_refuse_an_active_low_reset` and
//! `small_memories_are_left_generic_after_the_fpga_flow`. Each asserts
//! that the gap is *still there*, so closing one fails the test that
//! names it and points at the paragraph in `docs/ip-library.md` to
//! delete. Writing real HDL is how they were found, which is the argument
//! for a first-party library in the first place.
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
use reticle::sim::{NetHandle, SimOptions, Simulator};
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
