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
//! | `usb_descriptors_survive_lookup_table_mapping` | a **mapped** netlist still answers GET_DESCRIPTOR with the right bytes |
//! | `every_block_maps_to_the_logic_it_was_mapped_from` | every block's LUT4 and LUT6 mapping is **proved** equivalent to the logic it came from |
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
//! The four larger blocks are tested the same way and harder.
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
//! `mos6502` is the second processor and is deliberately the opposite
//! of the first: an 8-bit accumulator machine with thirteen addressing
//! modes, variable-length instructions and a cycle count per
//! instruction that its programs were written to depend on. `mod m6502`
//! is a second assembler built on the documented opcode matrix — the
//! mnemonic, the mode, the opcode byte and the reference's own cycle
//! count — so `mos6502_counts_the_cycles_of_every_instruction` compares
//! two independent statements of the same table rather than the core
//! with itself. Beside the architectural tests there are tests for each
//! of the quirks a compatible core has to reproduce: the indirect JMP
//! page bug, the page-crossing and branch penalties, the stack's wrap
//! inside page one, zero-page wrap, the read-modify-write double write
//! and the one-instruction delay of CLI and SEI. Its three end-to-end
//! programs are a 16 x 16 shift-and-add multiply, an array summed into
//! sixteen bits through a subroutine, and Fibonacci computed
//! recursively with its frames reached through `TSX` and absolute,X.
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
//! `usb_device_ulpi` is enumerated by **that same host model**, with a
//! **ULPI transceiver model** put between the two: a full-speed receiver
//! that recovers the bit clock off the pair, a transmitter that adds the
//! SYNC field, the stuffing, NRZI and the end of packet, and the ULPI bus
//! above them — both turnaround cycles, transmit and receive commands,
//! and a register file with its reset values. The enumeration is written
//! once, in `enumerate`, and run against both cores, because what a host
//! does to a device does not depend on how the device's bytes reach the
//! pair. The transceiver model checks the Link as well as answering it,
//! and a test drives it with a Link that breaks each of the turnaround's
//! rules, because a model that accepts anything proves nothing. The
//! loopback runs through it too, and through the version of it that
//! reports LineState a clock late, which is the part on the board.
//!
//! That transceiver model is **store and forward**: it takes the Link's bytes
//! at the interface's rate and puts the packet on the pair once `stp` has
//! ended it, where a real one serialises as the bytes arrive. So it adds one
//! interface clock per byte of the answer to the delay a host measures, and
//! `UsbPair::added_delay` takes that off before the inter-packet delay is
//! compared with the two to six and a half bit times USB 2.0 §7.1.18 allows.
//! It needed no such correction when a packet was eight bytes and the
//! overhead was eleven clocks; at 64 it does, and widening the window to 67
//! clocks instead would have made it vacuous, since the host gives up after
//! 18 bit times anyway.
//!
//! Seven tests here came from gaps in Reticle rather than in the blocks,
//! found by writing real HDL, which is the argument for a first-party
//! library in the first place:
//! `ice40_flip_flops_take_an_active_low_reset_through_one_inverter`,
//! `small_memories_become_logic_after_the_fpga_flow`,
//! `function_locals_are_not_reported_as_unreset_registers`,
//! `a_two_read_port_register_file_is_duplicated_across_block_rams`,
//! `a_project_top_that_is_also_instantiated_with_an_override_keeps_its_name`,
//! `a_zero_step_io_delay_builds_nothing` and
//! `a_comment_above_a_parameter_still_moves_into_the_port_list`. Each
//! asserts that its gap is *still there*, so fixing it fails the test and
//! names what to change; the first six now hold their fix and the
//! seventh, which the 6502 found, still holds its gap. The paragraph in
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
use reticle::verilog::format::{FormatOptions, format_source};
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
    // The divider that turns a bit rate into that divisor. Measured on
    // its own as well as inside `uart`, because it is the only block in
    // the library that does arithmetic on a number a host chose.
    Variant {
        package: "uart",
        top: "uart_baud_div",
        params: &[],
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
        package: "mos6502",
        top: "mos6502",
        params: &[("DECIMAL_MODE", "1")],
    },
    Variant {
        package: "mos6502",
        top: "mos6502",
        params: &[("DECIMAL_MODE", "0")],
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
        package: "vga_out",
        top: "vga_out",
        params: &[("MODE", "0"), ("BPC", "4")],
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
    // The same block with the **smallest** packet size USB 2.0 §5.8.3 and
    // §5.5.3 allow, so that the table says what 64 bytes cost rather than
    // leaving a reader to find out by building it. The whole difference
    // between this row and the one above is the two 64-byte buffers.
    Variant {
        package: "usb_device_fs",
        top: "usb_device_fs",
        params: &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("MAXPKT", "7'd8"),
            ("MAXPKT0", "7'd8"),
        ],
    },
    // And both sizes again with the buffers as **shift registers** rather than
    // arrays, which is `usb_bulk_ep`'s `BUF_RAM = 0`. Four rows a size, and
    // they earn the minutes: the array is the default because it is far
    // smaller on a family with a distributed RAM, it is larger on one without,
    // and "which one is my part" is a question the table should answer rather
    // than the prose. The pair of sizes is here because the penalty on the
    // iCE40 does not scale with the buffer the way the saving on the ECP5
    // does.
    Variant {
        package: "usb_device_fs",
        top: "usb_device_fs",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001"), ("BUF_RAM", "0")],
    },
    Variant {
        package: "usb_device_fs",
        top: "usb_device_fs",
        params: &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("MAXPKT", "7'd8"),
            ("MAXPKT0", "7'd8"),
            ("BUF_RAM", "0"),
        ],
    },
    Variant {
        package: "usb_device_fs_pll",
        top: "usb_device_fs_pll",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    Variant {
        package: "usb_device_ulpi",
        top: "usb_device_ulpi",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    Variant {
        package: "usb_cdc_acm",
        top: "usb_cdc_acm_fs",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    Variant {
        package: "usb_cdc_acm",
        top: "usb_cdc_acm_ulpi",
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

/// Every block's lookup-table mapping computes the logic it was mapped
/// from, proved rather than sampled, at LUT4 and LUT6.
///
/// This is the gate that was missing. `blocks_synthesise_cleanly` synthesises
/// every block and `footprints_match_the_documentation` maps every block onto
/// LUT4 and LUT6, and neither of them ever asked the mapped netlist to *do*
/// anything; `usb_descriptors_survive_lookup_table_mapping` asks one mapped
/// block for one thing, which is how the defect below was found, and it only
/// covers the descriptors of two blocks.
///
/// `reticle::synth::techmap::verify` closes it for every block and every
/// width at once. Mapping's input is an AIG and its output is a network over
/// the same inputs, so the two are combinational circuits with one interface:
/// the miter of them is decided by simulation for a counter-example and by
/// [`fraig`](reticle::synth::aig::fraig) for a proof, with no state, no
/// unrolling and no induction to be inconclusive about. Reverting the fix in
/// `src/synth/techmap/cuts.rs` fails this test on `usb_device_fs`,
/// `usb_cdc_acm`, `rv32i` and more, with the input assignment that breaks
/// each one.
///
/// **What it costs.** The work is bounded by the graphs and by
/// `MapVerifyOptions`, never by a clock: sixteen words of random patterns
/// over each miter and one `fraig` pass under its own conflict limit. Measured
/// in a debug build on one core, the sixty-eight mappings and their
/// sixty-eight proofs take **189 s** together; when there were sixty-two of
/// them it was 160 s against 84 s for the mappings alone, so a proof costs
/// about as much as the mapping it checks. The whole test is half of what
/// `footprints_match_the_documentation` already spends, and it maps the same
/// designs that test does. The largest is `rv32i` at 5552 AIG nodes and 2445
/// cells. Set `RETICLE_MAP_VERIFY_REPORT=1` to print the per-block verdicts
/// and node counts.
///
/// **What it covers less of than it did**, and it is worth knowing which way:
/// `usb_bulk_ep`'s packet buffers are arrays now, so they are memory cells that
/// `MapOptions::lut(k)` does not lower, and the network this proves for every
/// USB block is correspondingly smaller — `usb_device_fs` at LUT4 is 1668 AIG
/// nodes and 804 cells where the shift-register shape is 1964 and 902. Both
/// shapes are in `VARIANTS`, so both are proved; what is outside this check is
/// the memory itself, which is the backend's to build and
/// `tests/fpga_flow.rs`'s `the_logic_fallback_answers_like_the_memory_it_replaced`
/// to answer for.
#[test]
fn every_block_maps_to_the_logic_it_was_mapped_from() {
    use reticle::synth::techmap::{MapVerifyOptions, map_module_checked};

    let report = std::env::var_os("RETICLE_MAP_VERIFY_REPORT").is_some();
    let options = MapVerifyOptions::default();
    let mut unproved: Vec<String> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for variant in VARIANTS {
        for k in [4u32, 6] {
            let (mut design, id) = flattened(variant.package, variant.top, variant.params);
            let mut diags = Diagnostics::new();
            synth_run(&mut design, &SynthOptions::default(), &mut diags);
            assert!(
                !diags.has_errors(),
                "{}.{} does not synthesise",
                variant.package,
                variant.top
            );
            let (stats, equivalence) =
                map_module_checked(&mut design.modules[id], &MapOptions::lut(k), Some(&options));
            let equivalence = equivalence.expect("the check was asked for");
            let what = format!("{}.{} at LUT{k}", variant.package, variant.top);
            if report {
                println!(
                    "{what}: {} aig nodes, {} cells: {}",
                    stats.after.nodes,
                    stats.cells,
                    equivalence.render()
                );
            }
            checked += 1;
            if equivalence.wrong() {
                wrong.push(format!("{what}: {}", equivalence.render()));
            } else if !equivalence.proved() {
                unproved.push(format!("{what}: {}", equivalence.render()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the technology mapper emitted wrong logic for {} of {checked} mappings:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
    // An unproved mapping is a weaker answer, not a wrong one: the miter did
    // not collapse and no counter-example was simulated. It is still a
    // failure here, because on this library every one of them does collapse,
    // and a block that stops collapsing is worth a look rather than a shrug.
    assert!(
        unproved.is_empty(),
        "{} of {checked} mappings could not be proved:\n  {}",
        unproved.len(),
        unproved.join("\n  ")
    );
    assert!(checked >= 2 * VARIANTS.len(), "only {checked} mappings");
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
    // The divisor port, tied to zero: "use CLK_DIV", which is what these
    // two tests are about. `uart_baud_div` drives it in the designs that
    // follow a host's rate, and `uart_takes_its_divisor_from_a_port`
    // below is where a non-zero one is checked.
    sim.set(top_net(&sim, "div"), word(16, 0));
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
    sim.set(top_net(&sim, "div"), word(16, 0)); // "use CLK_DIV"
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

/// The gaps between the edges of one frame on `tx`, in clocks.
///
/// Sends `0x55` — which alternates, so start-plus-bit-0 is one gap of two
/// bit periods and the rest are one each — and returns the gaps. What a
/// caller checks is that every gap is a whole number of the period it
/// asked for, which is the only thing readable off a line without knowing
/// where the line began.
fn uart_bit_clocks(div: u64, clk_div: &str) -> Vec<u64> {
    let design = design_of("uart", "uart", &[("CLK_DIV", clk_div)]);
    let mut sim = simulate(&design, "uart");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let tx_data = top_net(&sim, "tx_data");
    let tx_valid = top_net(&sim, "tx_valid");
    let tx_ready = top_net(&sim, "tx_ready");
    let tx = top_net(&sim, "tx");

    sim.set(top_net(&sim, "rx"), bit(true));
    sim.set(tx_valid, bit(false));
    sim.set(top_net(&sim, "div"), word(16, div));
    reset(&mut sim, clk, rst_n);

    sim.set(tx_data, word(8, 0x55));
    sim.set(tx_valid, bit(true));
    let mut edges: Vec<u64> = Vec::new();
    let mut last = high(&sim, tx);
    let mut accepted = false;
    for tick in 0..4000u64 {
        if !accepted && high(&sim, tx_valid) && high(&sim, tx_ready) {
            accepted = true;
        }
        cycle(&mut sim, clk, HALF);
        if accepted {
            sim.set(tx_valid, bit(false));
        }
        let now = high(&sim, tx);
        if now != last {
            edges.push(tick);
            last = now;
        }
    }
    edges.windows(2).map(|w| w[1] - w[0]).collect()
}

/// The divisor is a port, and driving it changes the bit period.
///
/// Built with CLK_DIV 8 and driven with 20, so a period that came from the
/// parameter and one that came from the port cannot be confused: neither
/// number divides the other.
///
/// What it would not catch: whether the *receiver* uses the same number.
/// `uart_receives_a_byte_at_a_divisor_from_its_port` is that, and the two
/// are separate because a transmitter and a receiver reading different
/// divisors is precisely the failure a loopback cannot see — which is the
/// subject of `testdata/fpga/cynthion/usb_cdc_uart.v`'s header.
#[test]
fn uart_takes_its_divisor_from_a_port() {
    let gaps = uart_bit_clocks(20, "8");
    assert!(!gaps.is_empty(), "the transmitter never moved the line");
    for gap in &gaps {
        assert_eq!(
            gap % 20,
            0,
            "a gap of {gap} clocks is not a multiple of 20: {gaps:?}"
        );
    }
    assert!(
        gaps.iter().any(|g| *g == 20),
        "no gap is one bit long at the divisor asked for: {gaps:?}"
    );
}

/// Zero, and anything under four, means "use CLK_DIV".
///
/// A divisor a UART cannot keep time with has to have a defined answer,
/// and this is it: the number the design was built with, so the port goes
/// on working rather than stopping. Four is checked as well, because it is
/// the first divisor the receiver can halve and therefore the boundary.
#[test]
fn uart_falls_back_to_its_parameter_for_a_divisor_it_cannot_use() {
    for div in [0u64, 1, 3] {
        let gaps = uart_bit_clocks(div, "8");
        assert!(!gaps.is_empty(), "div {div}: the line never moved");
        for gap in &gaps {
            assert_eq!(
                gap % 8,
                0,
                "div {div}: a gap of {gap} clocks is not a multiple of CLK_DIV: {gaps:?}"
            );
        }
    }
    let gaps = uart_bit_clocks(4, "8");
    assert!(
        gaps.iter().any(|g| *g == 4),
        "a divisor of four is the smallest usable one and was not used: {gaps:?}"
    );
}

/// The receiver reads the same port, so a byte sent at a divisor from the
/// port comes back at it.
///
/// The loop is the transmitter's own output, one clock at a time, exactly
/// as `uart_receives_the_byte_its_own_transmitter_sends` does it. The only
/// difference is that the period is 20 and CLK_DIV is 8, so a receiver
/// still sampling at 8 would look for a stop bit in the middle of bit two
/// and return nothing or the wrong byte.
#[test]
fn uart_receives_a_byte_at_a_divisor_from_its_port() {
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

    sim.set(rx, bit(true));
    sim.set(tx_valid, bit(false));
    sim.set(top_net(&sim, "div"), word(16, 20));
    reset(&mut sim, clk, rst_n);

    sim.set(tx_data, word(8, 0xC3));
    sim.set(tx_valid, bit(true));
    let mut got: Vec<(u64, bool)> = Vec::new();
    let mut accepted = false;
    for _ in 0..1000 {
        let level = high(&sim, tx);
        sim.set(rx, bit(level));
        if !accepted && high(&sim, tx_valid) && high(&sim, tx_ready) {
            accepted = true;
        }
        cycle(&mut sim, clk, HALF);
        if accepted {
            sim.set(tx_valid, bit(false));
        }
        if high(&sim, rx_valid) {
            got.push((get_u64(&sim, rx_data), high(&sim, rx_error)));
        }
    }
    assert_eq!(got, vec![(0xC3, false)], "one byte, no framing error");
}

/// `uart_baud_div` divides, rounds to nearest, and says when it could not.
///
/// The expected divisor is computed here as `round(60e6 / rate)` rather
/// than listed, so this assertion and the worked table in the block's
/// header cannot drift apart without one of them being wrong about
/// arithmetic. The refusals *are* listed, because each is a different
/// reason and the reason is the point.
///
/// It also checks the thing the block is for: that every rate it accepts
/// lands inside the 2% an 8N1 frame survives.
///
/// What it would not catch: whether `uart` then uses the number — that is
/// `uart_takes_its_divisor_from_a_port` — or whether a real host's
/// `dwDTERate` reaches `rate`, which is
/// `usb_cdc_acm_ulpi_answers_the_line_coding_and_control_line_requests`.
#[test]
fn uart_baud_div_computes_the_divisor_for_the_rates_a_host_asks_for() {
    const CLK_HZ: u64 = 60_000_000;
    let design = design_of("uart", "uart_baud_div", &[]);
    let mut sim = simulate(&design, "uart_baud_div");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let rate = top_net(&sim, "rate");
    let div = top_net(&sim, "div");
    let ok = top_net(&sim, "ok");
    let busy = top_net(&sim, "busy");

    sim.set(rate, word(32, 0));
    reset(&mut sim, clk, rst_n);
    for _ in 0..80 {
        cycle(&mut sim, clk, HALF);
    }
    assert_eq!(get_u64(&sim, div), 521, "a rate of zero falls back");
    assert!(!high(&sim, ok), "and does not claim to be the host's rate");

    for want in [
        1200u64, 2400, 4800, 9600, 19_200, 38_400, 57_600, 115_200, 230_400, 460_800, 921_600,
        1_000_000, 1_500_000, 2_000_000,
    ] {
        sim.set(rate, word(32, want));
        for _ in 0..80 {
            cycle(&mut sim, clk, HALF);
        }
        assert!(
            !high(&sim, busy),
            "rate {want}: still dividing after 80 clocks"
        );
        let expect = (CLK_HZ + want / 2) / want;
        assert_eq!(
            get_u64(&sim, div),
            expect,
            "rate {want}: round(60e6 / {want}) is {expect}"
        );
        assert!(high(&sim, ok), "rate {want}: refused a rate it computed");
        // The divisor it chose is inside the 8N1 error budget. Integer
        // arithmetic, because a test that compares floating point is a
        // test about floating point.
        let slip = (CLK_HZ * 1000).abs_diff(want * expect * 1000) / (want * expect);
        assert!(
            slip * 50 < 1000,
            "rate {want}: divisor {expect} is {slip} parts per thousand off, over the 2% budget"
        );
    }

    // The refusals, each for its own reason.
    for (want, why) in [
        (1u64, "a divisor of 60 000 000 does not fit sixteen bits"),
        (
            915u64,
            "a divisor of 65 574 does not fit sixteen bits, by 39",
        ),
        (
            3_000_000u64,
            "a divisor of 20 is under DIV_MIN: half a clock in 20 is 2.5%",
        ),
        (
            40_000_000u64,
            "a divisor of 2 is under DIV_MIN and under what a shifter can do",
        ),
    ] {
        sim.set(rate, word(32, want));
        for _ in 0..80 {
            cycle(&mut sim, clk, HALF);
        }
        assert_eq!(get_u64(&sim, div), 521, "rate {want}: {why}");
        assert!(!high(&sim, ok), "rate {want}: {why}");
    }

    // And 916 is the other side of that boundary: 65 501 fits, just.
    sim.set(rate, word(32, 916));
    for _ in 0..80 {
        cycle(&mut sim, clk, HALF);
    }
    assert_eq!(get_u64(&sim, div), (CLK_HZ + 458) / 916);
    assert!(
        high(&sim, ok),
        "916 baud is the slowest rate sixteen bits can express at 60 MHz"
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

/// A minimal RV32I assembler, written out from the base ISA's field
/// layout. It lives in `tests/rv32i_asm/mod.rs` so that `tests/soc.rs`
/// assembles its program with the same encoders.
#[path = "rv32i_asm/mod.rs"]
mod asm;

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

/// A comment above a parameter is re-attached to the first *port*.
///
/// `mos6502` and `rv32i` both document their parameters the way every
/// block documents its ports — a `//` line above the declaration — and
/// `reticle fmt` lifts those lines out of the `#(...)` list and stacks
/// them in front of the first entry of the `(...)` list. No comment is
/// lost, which is the property `verilog::format`'s own corpus test
/// checks ("the set of comments" is preserved), but each one then sits
/// above something it does not describe: `DECIMAL_MODE`'s sentence ends
/// up above `clk`. A formatter that moves a sentence onto a different
/// declaration is worse than one that reindents badly, because the file
/// still looks right.
///
/// That is why neither core's source is run through `reticle fmt`, and
/// this test asserts the gap is **still there**: the fix — anchoring a
/// leading comment to the parameter it precedes in
/// `verilog::format::comments`, the same way a port's leading comment is
/// already anchored — makes it fail, and this paragraph is where to look
/// when it does.
#[test]
fn a_comment_above_a_parameter_still_moves_into_the_port_list() {
    const SOURCE: &str = "\
module m #(
    // How wide the data bus is.
    parameter WIDTH = 8
) (
    // The clock.
    input  wire clk,
    output wire q
);
    assign q = clk;
endmodule
";
    let formatted = format_source(SOURCE, Dialect::Verilog2005, &FormatOptions::default())
        .unwrap_or_else(|d| panic!("the sample does not format: {} problem(s)", d.len()));
    let (parameters, ports) = formatted
        .split_once(") (")
        .expect("a parameter list and then a port list");
    assert!(
        !parameters.contains("How wide the data bus is"),
        "the comment above `WIDTH` stayed in the parameter list, which means \
         `verilog::format::comments` now anchors it to the parameter: delete this \
         test and format both cores' sources.\n{formatted}"
    );
    assert!(
        ports.contains("// How wide the data bus is.\n  // The clock.\n  input  wire clk"),
        "the comment moved somewhere new again:\n{formatted}"
    );
}

// ---------------------------------------------------------------------------
// mos6502: a 6502 assembler, 64 KiB of memory, and programs the core has
// to get right
// ---------------------------------------------------------------------------

/// A minimal 6502 assembler, written out from the documented opcode
/// matrix rather than from the core's decoder. It lives in
/// `tests/mos6502_asm/mod.rs`.
#[path = "mos6502_asm/mod.rs"]
mod m6502;

/// The three vectors, where the part has always had them.
const VEC_NMI: u16 = 0xFFFA;
const VEC_RES: u16 = 0xFFFC;
const VEC_IRQ: u16 = 0xFFFE;

/// Where `m6502::assemble` places a program that does not say `.org`.
const M6502_ORIGIN: u16 = 0x0200;

/// Both settings of the decimal parameter. A program that does not use
/// decimal mode has to behave the same on each, so most tests run both.
const DECIMALS: [&str; 2] = ["1", "0"];

fn mos6502_design(decimal: &str) -> Design {
    design_of("mos6502", "mos6502", &[("DECIMAL_MODE", decimal)])
}

/// Assembles a program into 64 KiB of memory.
///
/// The three vectors are filled in where the program did not place them
/// itself: RES from the label `reset` or the origin, NMI from the label
/// `nmi`, IRQ from the label `irq`, each falling back to the origin.
fn mos6502_image(source: &str) -> (m6502::Image, Vec<u8>) {
    let image =
        m6502::assemble(source).unwrap_or_else(|e| panic!("the program does not assemble: {e}"));
    let mut mem = vec![0u8; 0x1_0000];
    for (addr, byte) in &image.bytes {
        mem[usize::from(*addr)] = *byte;
    }
    let start = image.bytes.keys().next().copied().unwrap_or(M6502_ORIGIN);
    for (vector, label) in [(VEC_RES, "reset"), (VEC_NMI, "nmi"), (VEC_IRQ, "irq")] {
        if image.bytes.contains_key(&vector) {
            continue;
        }
        let target = if image.has(label) {
            image.label(label)
        } else {
            start
        };
        mem[usize::from(vector)] = u8::try_from(target & 0xFF).expect("a byte");
        mem[usize::from(vector) + 1] = u8::try_from(target >> 8).expect("a byte");
    }
    (image, mem)
}

/// The core with 64 KiB behind its one bus, driven a cycle at a time.
///
/// The 6502 makes exactly one access per cycle, so [`Mos::step`] is both
/// "answer the bus" and "one clock": the memory presents the byte at
/// `addr`, takes `dout` where `we` is high, and the rising edge is the
/// transfer. With `stalls` wait states an access takes `stalls + 1`
/// clocks and the core must still spend the same number of *cycles*.
struct Mos<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    rst_n: NetHandle,
    addr: NetHandle,
    dout: NetHandle,
    din: NetHandle,
    we: NetHandle,
    ready: NetHandle,
    sync: NetHandle,
    irq: NetHandle,
    nmi: NetHandle,
    dbg_pc: NetHandle,
    dbg_retire: NetHandle,
    dbg_trap: NetHandle,
    a_n: NetHandle,
    x_n: NetHandle,
    y_n: NetHandle,
    s_n: NetHandle,
    pc_n: NetHandle,
    flag_n: [NetHandle; 6],
    /// The whole address space.
    mem: Vec<u8>,
    /// What the assembler said, so a test can name a label.
    image: m6502::Image,
    stalls: u32,
    /// Bus cycles taken since reset was released.
    cycles: u64,
    /// Instructions finished and interrupt sequences finished.
    retired: u64,
    traps: u64,
    /// Every bus cycle, once `trace` is on: address, write, byte.
    trace: bool,
    bus: Vec<(u16, bool, u8)>,
}

impl<'d> Mos<'d> {
    /// A core at its first opcode fetch, the reset sequence behind it.
    fn boot(design: &'d Design, source: &str, stalls: u32) -> Mos<'d> {
        let mut cpu = Mos::new(design, source, stalls);
        // RES is seven cycles, like every other interrupt sequence.
        for _ in 0..7 {
            cpu.cycle();
        }
        let vector = u16::from(cpu.mem[usize::from(VEC_RES)])
            | (u16::from(cpu.mem[usize::from(VEC_RES) + 1]) << 8);
        assert!(cpu.at_fetch(), "the reset sequence ends in an opcode fetch");
        assert_eq!(cpu.bus_addr(), vector, "and it fetches from the RES vector");
        assert_eq!(cpu.s(), 0xFD, "with S three below where it started");
        assert!(cpu.flag(2), "and I set");
        // The reset sequence is an interrupt sequence and counts as one;
        // the counters start at zero from the program's point of view.
        cpu.cycles = 0;
        cpu.retired = 0;
        cpu.traps = 0;
        cpu
    }

    /// A core held in reset and then released, with nothing run yet.
    fn new(design: &'d Design, source: &str, stalls: u32) -> Mos<'d> {
        let (image, mem) = mos6502_image(source);
        let sim = simulate(design, "mos6502");
        let name = |what: &str| net(&sim, &format!("{}.{what}", sim.top_name()));
        let mut cpu = Mos {
            clk: name("clk"),
            rst_n: name("rst_n"),
            addr: name("addr"),
            dout: name("dout"),
            din: name("din"),
            we: name("we"),
            ready: name("ready"),
            sync: name("sync"),
            irq: name("irq"),
            nmi: name("nmi"),
            dbg_pc: name("dbg_pc"),
            dbg_retire: name("dbg_retire"),
            dbg_trap: name("dbg_trap"),
            a_n: name("a_r"),
            x_n: name("x_r"),
            y_n: name("y_r"),
            s_n: name("s_r"),
            pc_n: name("pc"),
            flag_n: [
                name("p_c"),
                name("p_z"),
                name("p_i"),
                name("p_d"),
                name("p_v"),
                name("p_n"),
            ],
            sim,
            mem,
            image,
            stalls,
            cycles: 0,
            retired: 0,
            traps: 0,
            trace: false,
            bus: Vec::new(),
        };
        cpu.start();
        cpu
    }

    fn start(&mut self) {
        for line in [self.irq, self.nmi] {
            self.sim.set(line, bit(false));
        }
        self.sim.set(self.ready, bit(true));
        self.sim.set(self.din, word(8, 0));
        let clk = self.clk;
        let rst_n = self.rst_n;
        reset(&mut self.sim, clk, rst_n);
    }

    /// The address the core is presenting.
    fn bus_addr(&self) -> u16 {
        u16::try_from(loose_u64(&self.sim, self.addr) & 0xFFFF).expect("sixteen bits")
    }

    /// Whether this cycle is an opcode fetch.
    fn at_fetch(&self) -> bool {
        high(&self.sim, self.sync)
    }

    /// One bus cycle: answer the access, take the edge. A cycle takes
    /// `stalls + 1` clocks.
    fn cycle(&mut self) {
        let address = self.bus_addr();
        let writing = high(&self.sim, self.we);
        let byte = self.mem[usize::from(address)];
        self.sim.set(self.din, word(8, u64::from(byte)));
        // The access is held until the memory says it is ready.
        for _ in 0..self.stalls {
            self.sim.set(self.ready, bit(false));
            let clk = self.clk;
            cycle(&mut self.sim, clk, HALF);
            assert_eq!(
                self.bus_addr(),
                address,
                "the access is held while the memory is not ready"
            );
        }
        self.sim.set(self.ready, bit(true));
        let written = octet(loose_u64(&self.sim, self.dout));
        if writing {
            self.mem[usize::from(address)] = written;
        }
        if self.trace {
            self.bus
                .push((address, writing, if writing { written } else { byte }));
        }
        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);
        self.cycles += 1;
        if high(&self.sim, self.dbg_retire) {
            self.retired += 1;
        }
        if high(&self.sim, self.dbg_trap) {
            self.traps += 1;
        }
    }

    /// `n` bus cycles.
    fn run(&mut self, n: u32) {
        for _ in 0..n {
            self.cycle();
        }
    }

    /// Runs one whole instruction, from this opcode fetch to the next,
    /// and answers how many cycles it took.
    fn next(&mut self) -> u32 {
        assert!(self.at_fetch(), "`next` starts at an opcode fetch");
        let mut n = 0;
        loop {
            self.cycle();
            n += 1;
            if self.at_fetch() {
                return n;
            }
            assert!(n < 64, "an instruction that never ends");
        }
    }

    /// Runs `count` instructions.
    fn run_instructions(&mut self, count: usize) {
        for _ in 0..count {
            self.next();
        }
    }

    /// Runs until the opcode at `addr` is fetched, and answers the
    /// number of cycles that took.
    fn run_until_fetch(&mut self, addr: u16, limit: u32) -> u32 {
        for n in 0..limit {
            if self.at_fetch() && self.bus_addr() == addr {
                return n;
            }
            self.cycle();
        }
        panic!("{addr:#06x} was never fetched within {limit} cycles");
    }

    /// Runs until `count` interrupt sequences have been entered since
    /// reset, counting BRK and every taken IRQ and NMI.
    fn run_until_traps(&mut self, count: u64, limit: u32) {
        for _ in 0..limit {
            if self.traps >= count {
                return;
            }
            self.cycle();
        }
        panic!(
            "only {} interrupt(s) in {limit} cycles, wanted {count}",
            self.traps
        );
    }

    /// Runs until the program's `done` label is reached.
    fn run_to_done(&mut self, limit: u32) -> u32 {
        let done = self.image.label("done");
        self.run_until_fetch(done, limit)
    }

    fn a(&self) -> u8 {
        octet(get_u64(&self.sim, self.a_n))
    }
    fn x(&self) -> u8 {
        octet(get_u64(&self.sim, self.x_n))
    }
    fn y(&self) -> u8 {
        octet(get_u64(&self.sim, self.y_n))
    }
    fn s(&self) -> u8 {
        octet(get_u64(&self.sim, self.s_n))
    }
    fn pc(&self) -> u16 {
        u16::try_from(get_u64(&self.sim, self.pc_n) & 0xFFFF).expect("sixteen bits")
    }

    /// One flag by its bit number in the status byte.
    fn flag(&self, bit_number: usize) -> bool {
        let index = match bit_number {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            6 => 4,
            _ => 5,
        };
        high(&self.sim, self.flag_n[index])
    }

    /// The status byte as PHP would push it, without bit 4: bit 5 reads
    /// as one and B is not a flag.
    fn p(&self) -> u8 {
        let mut value = 0x20u8;
        for (bit_number, weight) in [
            (0, 0x01u8),
            (1, 0x02),
            (2, 0x04),
            (3, 0x08),
            (6, 0x40),
            (7, 0x80),
        ] {
            if self.flag(bit_number) {
                value |= weight;
            }
        }
        value
    }

    fn byte_at(&self, addr: u16) -> u8 {
        self.mem[usize::from(addr)]
    }

    fn set_byte(&mut self, addr: u16, value: u8) {
        self.mem[usize::from(addr)] = value;
    }

    /// The little-endian word at `addr`.
    fn word_at(&self, addr: u16) -> u16 {
        u16::from(self.byte_at(addr)) | (u16::from(self.byte_at(addr.wrapping_add(1))) << 8)
    }

    fn label(&self, name: &str) -> u16 {
        self.image.label(name)
    }

    /// Starts recording every bus cycle.
    fn record(&mut self) {
        self.trace = true;
        self.bus.clear();
    }

    /// The addresses recorded so far.
    fn addresses(&self) -> Vec<u16> {
        self.bus.iter().map(|(a, _, _)| *a).collect()
    }

    fn set_irq(&mut self, value: bool) {
        let line = self.irq;
        self.sim.set(line, bit(value));
    }

    fn set_nmi(&mut self, value: bool) {
        let line = self.nmi;
        self.sim.set(line, bit(value));
    }

    /// The address the last finished instruction was fetched from.
    fn last_pc(&self) -> u16 {
        u16::try_from(get_u64(&self.sim, self.dbg_pc) & 0xFFFF).expect("sixteen bits")
    }
}

#[test]
fn mos6502_loads_stores_and_transfers() {
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        lda #$42
        sta $10
        ldx #$80
        stx $0011
        ldy #$00
        sty $12
        lda $10
        tax
        tay
        lda #$ff
        txa
        ldx #$fe
        txs
        tsx
        tya
done:   jmp done
",
            0,
        );

        cpu.next();
        assert_eq!(cpu.a(), 0x42, "DECIMAL_MODE={decimal}: LDA immediate");
        assert_eq!(cpu.p() & 0x82, 0, "a positive, non-zero byte sets neither");
        cpu.next();
        assert_eq!(cpu.byte_at(0x0010), 0x42, "STA zero page");
        cpu.next();
        assert_eq!(cpu.x(), 0x80);
        assert_eq!(cpu.p() & 0x82, 0x80, "LDX of a byte with bit 7 set is N");
        cpu.next();
        assert_eq!(cpu.byte_at(0x0011), 0x80, "STX absolute");
        cpu.next();
        assert_eq!(cpu.y(), 0x00);
        assert_eq!(cpu.p() & 0x82, 0x02, "LDY of zero is Z");
        cpu.next();
        assert_eq!(cpu.byte_at(0x0012), 0x00, "STY zero page");
        cpu.next();
        assert_eq!(cpu.a(), 0x42, "LDA zero page reads back what STA wrote");
        cpu.next();
        assert_eq!(cpu.x(), 0x42, "TAX");
        cpu.next();
        assert_eq!(cpu.y(), 0x42, "TAY");
        cpu.run_instructions(2);
        assert_eq!(cpu.a(), 0x42, "TXA");
        cpu.next();
        assert_eq!(cpu.x(), 0xFE);
        cpu.next();
        assert_eq!(cpu.s(), 0xFE, "TXS moves X into S");
        assert_eq!(
            cpu.p() & 0x82,
            0x80,
            "and is the one transfer that sets no flag"
        );
        cpu.next();
        assert_eq!(cpu.x(), 0xFE, "TSX moves it back");
        cpu.next();
        assert_eq!(cpu.a(), 0x42, "TYA");
        assert_eq!(
            cpu.bus_addr(),
            cpu.label("done"),
            "and the program ran to the end"
        );
    }
}

#[test]
fn mos6502_reaches_every_addressing_mode() {
    // One load per mode, each from an address only that mode computes,
    // so a mode that lands anywhere else reads a zero.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        ldx #$04
        ldy #$06
        lda #$11
        sta $30         ; a byte the (zp,X) pointer is not
        lda $0040       ; absolute, a low address written wide
        lda $41,x       ; zero page,X  -> $45
        lda $42,y       ; there is no LDA $nn,y, so this is absolute,Y
        lda $1000,x     ; absolute,X   -> $1004
        lda $1010,y     ; absolute,Y   -> $1016
        lda ($30,x)     ; the pointer at $30 + X = $34 -> $2000
        lda ($38),y     ; the pointer at $38 -> $3000, plus Y
        lda #$00
done:   jmp done
",
        0,
    );
    // The pointers and the byte each mode must find.
    cpu.set_byte(0x0034, 0x00);
    cpu.set_byte(0x0035, 0x20);
    cpu.set_byte(0x0038, 0x00);
    cpu.set_byte(0x0039, 0x30);
    for (addr, value) in [
        (0x0040u16, 0xA1u8),
        (0x0045, 0xA2),
        (0x0048, 0xA3),
        (0x1004, 0xA4),
        (0x1016, 0xA5),
        (0x2000, 0xA6),
        (0x3006, 0xA7),
    ] {
        cpu.set_byte(addr, value);
    }

    cpu.run_instructions(4);
    for (what, want) in [
        ("absolute", 0xA1u8),
        ("zero page,X", 0xA2),
        ("absolute,Y", 0xA3),
        ("absolute,X", 0xA4),
        ("absolute,Y again", 0xA5),
        ("(zp,X)", 0xA6),
        ("(zp),Y", 0xA7),
    ] {
        cpu.next();
        assert_eq!(cpu.a(), want, "{what} reached the wrong byte");
    }
}

/// Runs one arithmetic instruction with the accumulator, the operand,
/// the carry and the decimal flag the caller names, and answers the
/// accumulator and the status byte it left.
fn mos6502_arith(
    design: &Design,
    mnemonic: &str,
    a: u8,
    m: u8,
    carry: bool,
    decimal: bool,
) -> (u8, u8) {
    let source = format!(
        "        {}\n        {}\n        lda #${a:02x}\n        {mnemonic} #${m:02x}\ndone:   jmp done\n",
        if decimal { "sed" } else { "cld" },
        if carry { "sec" } else { "clc" },
    );
    let mut cpu = Mos::boot(design, &source, 0);
    cpu.run_to_done(40);
    (cpu.a(), cpu.p())
}

/// The status bits the arithmetic tests name, as a mask.
const F_C: u8 = 0x01;
const F_Z: u8 = 0x02;
const F_I: u8 = 0x04;
const F_D: u8 = 0x08;
const F_V: u8 = 0x40;
const F_N: u8 = 0x80;

/// The four flags an ADC or an SBC sets, as a byte, so a case can be
/// written as one expectation.
fn arith_flags(c: bool, z: bool, n: bool, v: bool) -> u8 {
    (if c { F_C } else { 0 })
        | (if z { F_Z } else { 0 })
        | (if n { F_N } else { 0 })
        | (if v { F_V } else { 0 })
}

#[test]
fn mos6502_traces_what_it_retired() {
    // `dbg_retire` pulses at the end of an instruction with `dbg_pc`
    // naming the opcode it came from, which is how a testbench follows a
    // program without decoding the bus for itself.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        lda #$01
        ldx #$02
        jmp there
        nop             ; jumped over
there:  nop
done:   jmp done
",
        0,
    );
    let mut seen = Vec::new();
    for _ in 0..4 {
        let fetched = cpu.bus_addr();
        cpu.next();
        seen.push((fetched, cpu.last_pc()));
    }
    for (fetched, traced) in &seen {
        assert_eq!(
            fetched, traced,
            "`dbg_pc` names the opcode whose instruction just retired"
        );
    }
    assert_eq!(
        cpu.retired, 4,
        "four instructions, the skipped NOP not among them"
    );
    assert_eq!(cpu.traps, 0, "and nothing trapped");
    assert_eq!(
        cpu.pc(),
        cpu.label("done"),
        "the program counter is at the opcode being fetched"
    );
}

#[test]
fn mos6502_counts_its_index_registers() {
    // INX, INY, DEX and DEY over both wrap-arounds, with the flags each
    // one leaves.
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        ldx #$fe
        inx             ; $FF
        inx             ; $00
        inx             ; $01
        dex             ; $00
        dex             ; $FF
        ldy #$7f
        iny             ; $80
        dey             ; $7F
        ldy #$00
        dey             ; $FF
        iny             ; $00
done:   jmp done
",
            0,
        );
        cpu.next();
        for (want, flags) in [
            (0xFFu8, F_N),
            (0x00, F_Z),
            (0x01, 0),
            (0x00, F_Z),
            (0xFF, F_N),
        ] {
            cpu.next();
            assert_eq!(cpu.x(), want, "DECIMAL_MODE={decimal}: the X counters");
            assert_eq!(cpu.p() & (F_N | F_Z), flags, "and their flags");
        }
        cpu.next();
        for (want, flags) in [(0x80u8, F_N), (0x7F, 0)] {
            cpu.next();
            assert_eq!(cpu.y(), want, "the Y counters");
            assert_eq!(cpu.p() & (F_N | F_Z), flags);
        }
        cpu.next();
        for (want, flags) in [(0xFFu8, F_N), (0x00, F_Z)] {
            cpu.next();
            assert_eq!(cpu.y(), want, "and Y wrapping the other way");
            assert_eq!(cpu.p() & (F_N | F_Z), flags);
        }
    }
}

#[test]
fn mos6502_stores_through_every_mode() {
    // Every mode each of the three stores has, each landing on an
    // address only that mode computes.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        ldx #$02
        ldy #$03
        lda #$5a
        sta $10         ; $0010
        sta $20,x       ; $0022
        sta $0030       ; $0030
        sta $0040,x     ; $0042
        sta $0050,y     ; $0053
        sta ($60,x)     ; the pointer at $62 -> $1000
        sta ($70),y     ; the pointer at $70 -> $2000, plus Y
        stx $11
        stx $0012
        stx $13,y       ; STX has a zero page,Y and no zero page,X
        sty $14
        sty $0015
        sty $16,x       ; and STY the other way round
done:   jmp done
",
        0,
    );
    cpu.set_byte(0x0062, 0x00);
    cpu.set_byte(0x0063, 0x10);
    cpu.set_byte(0x0070, 0x00);
    cpu.set_byte(0x0071, 0x20);
    cpu.run_to_done(120);

    for (addr, want, what) in [
        (0x0010u16, 0x5Au8, "STA zero page"),
        (0x0022, 0x5A, "STA zero page,X"),
        (0x0030, 0x5A, "STA absolute"),
        (0x0042, 0x5A, "STA absolute,X"),
        (0x0053, 0x5A, "STA absolute,Y"),
        (0x1000, 0x5A, "STA (zp,X)"),
        (0x2003, 0x5A, "STA (zp),Y"),
        (0x0011, 0x02, "STX zero page"),
        (0x0012, 0x02, "STX absolute"),
        (0x0016, 0x02, "STX zero page,Y"),
        (0x0014, 0x03, "STY zero page"),
        (0x0015, 0x03, "STY absolute"),
        (0x0018, 0x03, "STY zero page,X"),
    ] {
        assert_eq!(cpu.byte_at(addr), want, "{what} should reach {addr:#06x}");
    }
    assert_eq!(cpu.a(), 0x5A, "and none of them changed a register");
    assert_eq!(cpu.x(), 0x02);
    assert_eq!(cpu.y(), 0x03);
}

#[test]
fn mos6502_computes_every_logical_operation() {
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        lda #$f0
        sta $10
        lda #$3c
        ora $10         ; $FC
        and #$0f        ; $0C
        eor #$ff        ; $F3
        eor #$f3        ; $00
        ora #$00        ; $00 again, to leave Z set
done:   jmp done
",
            0,
        );
        cpu.run_instructions(3);
        cpu.next();
        assert_eq!(cpu.a(), 0xFC, "DECIMAL_MODE={decimal}: ORA");
        assert_eq!(cpu.p() & (F_N | F_Z), F_N, "and its flags");
        cpu.next();
        assert_eq!(cpu.a(), 0x0C, "AND");
        assert_eq!(cpu.p() & (F_N | F_Z), 0);
        cpu.next();
        assert_eq!(cpu.a(), 0xF3, "EOR");
        assert_eq!(cpu.p() & (F_N | F_Z), F_N);
        cpu.next();
        assert_eq!(cpu.a(), 0x00, "EOR with itself");
        assert_eq!(cpu.p() & (F_N | F_Z), F_Z);
        cpu.next();
        assert_eq!(cpu.p() & (F_N | F_Z), F_Z, "ORA of zero with zero");
    }
}

#[test]
fn mos6502_sets_overflow_for_every_sign_combination() {
    // The four sign combinations of ADC and of SBC, each with a result
    // that overflows and one that does not: the table every 6502
    // tutorial on the V flag prints.
    let cases: &[(&str, u8, u8, bool, u8, bool, bool)] = &[
        // mnemonic,  A,    M,   carry-in, result, carry-out, overflow
        ("adc", 0x50, 0x10, false, 0x60, false, false), // + +  -> +
        ("adc", 0x50, 0x50, false, 0xA0, false, true),  // + +  -> - !
        ("adc", 0x50, 0x90, false, 0xE0, false, false), // + -  -> -
        ("adc", 0x50, 0xD0, false, 0x20, true, false),  // + -  -> +
        ("adc", 0xD0, 0x10, false, 0xE0, false, false), // - +  -> -
        ("adc", 0xD0, 0x50, false, 0x20, true, false),  // - +  -> +
        ("adc", 0xD0, 0x90, false, 0x60, true, true),   // - -  -> + !
        ("adc", 0xD0, 0xD0, false, 0xA0, true, false),  // - -  -> -
        // the carry in is part of the sum, and of the overflow
        ("adc", 0x7F, 0x00, true, 0x80, false, true),
        ("adc", 0xFF, 0x00, true, 0x00, true, false),
        ("sbc", 0x50, 0xF0, true, 0x60, false, false),
        ("sbc", 0x50, 0xB0, true, 0xA0, false, true),
        ("sbc", 0x50, 0x70, true, 0xE0, false, false),
        ("sbc", 0x50, 0x30, true, 0x20, true, false),
        ("sbc", 0xD0, 0xF0, true, 0xE0, false, false),
        ("sbc", 0xD0, 0xB0, true, 0x20, true, false),
        ("sbc", 0xD0, 0x70, true, 0x60, true, true),
        ("sbc", 0xD0, 0x30, true, 0xA0, true, false),
        // a borrow in is one more taken away
        ("sbc", 0x00, 0x00, false, 0xFF, false, false),
        ("sbc", 0x80, 0x00, false, 0x7F, true, true),
    ];
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        for (mnemonic, a, m, carry, result, carry_out, overflow) in cases.iter().copied() {
            let (got, p) = mos6502_arith(&design, mnemonic, a, m, carry, false);
            let want = arith_flags(carry_out, result == 0, result & 0x80 != 0, overflow);
            assert_eq!(
                got, result,
                "DECIMAL_MODE={decimal}: {mnemonic} #${m:02x} with A=${a:02x} C={carry}"
            );
            assert_eq!(
                p & (F_C | F_Z | F_N | F_V),
                want,
                "DECIMAL_MODE={decimal}: the flags of {mnemonic} #${m:02x} with A=${a:02x} C={carry}"
            );
        }
    }
}

/// One decimal-mode case: the accumulator, the operand and the carry
/// in, then the result and the C, Z, N and V it leaves.
type DecimalCase = (u8, u8, bool, u8, bool, bool, bool, bool);

#[test]
fn mos6502_adds_and_subtracts_in_decimal_mode() {
    // Known results of the NMOS part, including the three places its
    // decimal arithmetic is famously surprising: Z comes from the binary
    // sum, N and V from the intermediate before the high nibble is
    // corrected, and SBC's flags are the binary subtraction's entirely.
    let adc: &[DecimalCase] = &[
        // A,    M,   Cin, result, C,     Z,     N,     V
        (0x00, 0x00, false, 0x00, false, true, false, false),
        (0x00, 0x00, true, 0x01, false, false, false, false),
        (0x09, 0x01, false, 0x10, false, false, false, false),
        (0x12, 0x34, false, 0x46, false, false, false, false),
        (0x50, 0x50, false, 0x00, true, false, true, true),
        (0x99, 0x01, false, 0x00, true, false, true, false),
        (0x99, 0x00, true, 0x00, true, false, true, false),
        (0x58, 0x46, true, 0x05, true, false, true, true),
        (0x79, 0x00, true, 0x80, false, false, true, true),
        (0x24, 0x56, false, 0x80, false, false, true, true),
        (0x93, 0x82, false, 0x75, true, false, false, true),
        (0x89, 0x76, false, 0x65, true, false, false, false),
        // the binary sum is $100, so Z is set although the result is $60
        (0x80, 0x80, false, 0x60, true, true, false, true),
        (0x45, 0x45, false, 0x90, false, false, true, true),
    ];
    let sbc: &[DecimalCase] = &[
        (0x00, 0x00, true, 0x00, true, true, false, false),
        (0x00, 0x01, true, 0x99, false, false, true, false),
        (0x50, 0x25, true, 0x25, true, false, false, false),
        (0x12, 0x34, true, 0x78, false, false, true, false),
        (0x46, 0x12, true, 0x34, true, false, false, false),
        (0x99, 0x99, true, 0x00, true, true, false, false),
        (0x99, 0x00, true, 0x99, true, false, true, false),
        (0x20, 0x10, false, 0x09, true, false, false, false),
        (0x05, 0x21, true, 0x84, false, false, true, false),
    ];

    let design = mos6502_design("1");
    for (mnemonic, cases) in [("adc", adc), ("sbc", sbc)] {
        for (a, m, carry, result, c, z, n, v) in cases.iter().copied() {
            let (got, p) = mos6502_arith(&design, mnemonic, a, m, carry, true);
            assert_eq!(
                got, result,
                "SED {mnemonic} #${m:02x} with A=${a:02x} C={carry}"
            );
            assert_eq!(
                p & (F_C | F_Z | F_N | F_V),
                arith_flags(c, z, n, v),
                "the flags of SED {mnemonic} #${m:02x} with A=${a:02x} C={carry}"
            );
            assert_eq!(p & F_D, F_D, "and D is still set afterwards");
        }
    }

    // Compiled out, D is still a flag and the arithmetic is binary.
    let plain = mos6502_design("0");
    for (mnemonic, cases) in [("adc", adc), ("sbc", sbc)] {
        for (a, m, carry, _, _, _, _, _) in cases.iter().copied() {
            let (got, p) = mos6502_arith(&plain, mnemonic, a, m, carry, true);
            let (binary, _) = mos6502_arith(&plain, mnemonic, a, m, carry, false);
            assert_eq!(
                got, binary,
                "DECIMAL_MODE=0: SED {mnemonic} #${m:02x} with A=${a:02x} is still binary"
            );
            assert_eq!(p & F_D, F_D, "but SED still sets the flag");
        }
    }
}

#[test]
fn mos6502_compares_with_cmp_cpx_and_cpy() {
    // A comparison is a subtraction whose result is thrown away: C is
    // set when the register is at least the operand, Z when they are
    // equal, N from bit 7 of the difference. None of the three touches
    // V, and none of them is affected by decimal mode.
    let cases: &[(u8, u8, bool, bool, bool)] = &[
        // register, operand, C,     Z,     N
        (0x00, 0x00, true, true, false),
        (0x01, 0x00, true, false, false),
        (0x00, 0x01, false, false, true),
        (0x7F, 0x80, false, false, true),
        (0x80, 0x7F, true, false, false),
        (0xFF, 0xFF, true, true, false),
        (0xFF, 0x00, true, false, true),
        (0x00, 0xFF, false, false, false),
        (0x40, 0x20, true, false, false),
    ];
    let design = mos6502_design("1");
    for (mnemonic, load) in [("cmp", "lda"), ("cpx", "ldx"), ("cpy", "ldy")] {
        for (register, operand, c, z, n) in cases.iter().copied() {
            // V is set beforehand so that a comparison touching it is
            // caught, and D is set so that a decimal comparison is too.
            let source = format!(
                "        sed
        clc
        lda #$50
        adc #$50        ; V = 1
        {load} #${register:02x}
        {mnemonic} #${operand:02x}
done:   jmp done
"
            );
            let mut cpu = Mos::boot(&design, &source, 0);
            cpu.run_to_done(60);
            assert_eq!(
                cpu.p() & (F_C | F_Z | F_N),
                arith_flags(c, z, n, false),
                "{mnemonic} #${operand:02x} against ${register:02x}"
            );
            assert_eq!(cpu.p() & F_V, F_V, "{mnemonic} leaves V alone");
        }
    }
}

#[test]
fn mos6502_tests_bits_with_bit() {
    // BIT is the odd one: N and V are bits 7 and 6 of the *memory*
    // byte, whatever the accumulator holds, and only Z comes from the
    // conjunction.
    let design = mos6502_design("1");
    for (memory, accumulator, n, v, z) in [
        (0xC0u8, 0x00u8, true, true, true),
        (0xC0, 0xFF, true, true, false),
        (0x80, 0x80, true, false, false),
        (0x40, 0x40, false, true, false),
        (0x3F, 0xFF, false, false, false),
        (0x00, 0xFF, false, false, true),
        (0xFF, 0x01, true, true, false),
    ] {
        let source = format!(
            "        lda #${memory:02x}
        sta $10
        sta $0300
        lda #${accumulator:02x}
        bit $10
done:   jmp done
"
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        cpu.run_to_done(40);
        assert_eq!(
            cpu.p() & (F_N | F_V | F_Z),
            arith_flags(false, z, n, v),
            "BIT ${memory:02x} with A=${accumulator:02x}"
        );
        assert_eq!(cpu.a(), accumulator, "and BIT does not change A");
    }
}

#[test]
fn mos6502_shifts_and_rotates_through_carry() {
    // Each of the four over the accumulator, with the carry both ways.
    let cases: &[(&str, u8, bool, u8, bool)] = &[
        // mnemonic, in,   Cin,   out,  Cout
        ("asl", 0x40, false, 0x80, false),
        ("asl", 0x80, false, 0x00, true),
        ("asl", 0xFF, true, 0xFE, true),
        ("lsr", 0x01, false, 0x00, true),
        ("lsr", 0x80, true, 0x40, false),
        ("lsr", 0xFF, false, 0x7F, true),
        ("rol", 0x80, false, 0x00, true),
        ("rol", 0x80, true, 0x01, true),
        ("rol", 0x7F, true, 0xFF, false),
        ("ror", 0x01, false, 0x00, true),
        ("ror", 0x01, true, 0x80, true),
        ("ror", 0xFE, false, 0x7F, false),
    ];
    let design = mos6502_design("1");
    for (mnemonic, input, carry_in, output, carry_out) in cases.iter().copied() {
        let setup = if carry_in { "sec" } else { "clc" };
        // The accumulator form and the memory form have to agree.
        let source = format!(
            "        lda #${input:02x}
        sta $10
        {setup}
        {mnemonic} a
        sta $11
        lda #${input:02x}
        sta $12
        {setup}
        {mnemonic} $12
done:   jmp done
"
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        cpu.run_instructions(4);
        assert_eq!(
            cpu.a(),
            output,
            "{mnemonic} a of ${input:02x} with C={carry_in}"
        );
        assert_eq!(
            cpu.p() & (F_C | F_Z | F_N),
            arith_flags(carry_out, output == 0, output & 0x80 != 0, false),
            "the flags of {mnemonic} a of ${input:02x} with C={carry_in}"
        );
        cpu.run_to_done(40);
        assert_eq!(
            cpu.byte_at(0x0012),
            output,
            "{mnemonic} $12 of ${input:02x} with C={carry_in}"
        );
        assert_eq!(
            cpu.p() & (F_C | F_Z | F_N),
            arith_flags(carry_out, output == 0, output & 0x80 != 0, false),
            "and its flags are the same as the accumulator form's"
        );
    }
}

#[test]
fn mos6502_reads_modifies_and_writes_memory() {
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        lda #$7f
        sta $10
        inc $10         ; $80: N
        dec $10         ; $7F
        lda #$81
        sta $11
        asl $11         ; $02, C = 1
        lsr $11         ; $01, C = 0
        sec
        rol $11         ; $03, C = 0
        ror $11         ; $01, C = 1
        ldx #$01
        inc $10,x       ; $11 -> $02, C untouched
        lda #$ff
        sta $0400
        inc $0400       ; $00: Z, C still set
        dec $0400,x     ; $0401 -> $FF
done:   jmp done
",
            0,
        );
        cpu.run_instructions(2);
        cpu.next();
        assert_eq!(cpu.byte_at(0x0010), 0x80, "DECIMAL_MODE={decimal}: INC");
        assert_eq!(cpu.p() & (F_N | F_Z), F_N);
        cpu.next();
        assert_eq!(cpu.byte_at(0x0010), 0x7F, "DEC");
        assert_eq!(cpu.p() & (F_N | F_Z), 0);
        cpu.run_instructions(2);
        cpu.next();
        assert_eq!(cpu.byte_at(0x0011), 0x02, "ASL in memory");
        assert_eq!(cpu.p() & F_C, F_C, "and its carry out");
        cpu.next();
        assert_eq!(cpu.byte_at(0x0011), 0x01, "LSR in memory");
        assert_eq!(cpu.p() & F_C, 0);
        cpu.next();
        cpu.next();
        assert_eq!(
            cpu.byte_at(0x0011),
            0x03,
            "ROL in memory brings the carry in"
        );
        assert_eq!(cpu.p() & F_C, 0);
        cpu.next();
        assert_eq!(cpu.byte_at(0x0011), 0x01, "ROR in memory");
        assert_eq!(cpu.p() & F_C, F_C);
        cpu.run_instructions(2);
        assert_eq!(cpu.byte_at(0x0011), 0x02, "INC zero page,X");
        assert_eq!(cpu.p() & F_C, F_C, "and INC leaves the carry alone");
        cpu.run_instructions(3);
        assert_eq!(cpu.byte_at(0x0400), 0x00, "INC absolute wrapping to zero");
        assert_eq!(cpu.p() & (F_Z | F_C), F_Z | F_C);
        cpu.next();
        assert_eq!(cpu.byte_at(0x0401), 0xFF, "DEC absolute,X wrapping down");
        assert_eq!(cpu.p() & F_N, F_N);
    }
}

#[test]
fn mos6502_writes_a_read_modify_write_byte_back_before_the_result() {
    // A 6502 read-modify-write touches its address three times: a read,
    // a write of what it read, and a write of the result. Hardware
    // registers mapped into memory can see that middle write, so it is
    // part of the contract and not an implementation detail.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        lda #$41
        sta $10
        inc $10
done:   jmp done
",
        0,
    );
    cpu.run_instructions(2);
    cpu.record();
    assert_eq!(cpu.next(), 5, "INC zero page is five cycles");
    assert_eq!(
        cpu.bus,
        vec![
            (M6502_ORIGIN + 4, false, 0xE6),
            (M6502_ORIGIN + 5, false, 0x10),
            (0x0010, false, 0x41),
            (0x0010, true, 0x41),
            (0x0010, true, 0x42),
        ],
        "read, write back, write the result"
    );
}

#[test]
fn mos6502_wraps_zero_page_indexing_and_its_pointers() {
    // Zero-page indexing never leaves page zero, and neither does the
    // pair of addresses a pointer is read from.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        ldx #$02
        ldy #$03
        lda $ff,x       ; $FF + 2 = $01, never $0101
        sta $20
        lda ($fe,x)     ; the pointer is at $00 and $01
        sta $21
        lda ($ff),y     ; the pointer is at $FF and $00, wrapped
        sta $22
done:   jmp done
",
        0,
    );
    cpu.set_byte(0x0000, 0x34);
    cpu.set_byte(0x0001, 0x12);
    cpu.set_byte(0x00FF, 0x00);
    // What a core that did not wrap would read instead.
    cpu.set_byte(0x0100, 0x99);
    cpu.set_byte(0x0101, 0x99);
    cpu.set_byte(0x1234, 0xC3);
    cpu.set_byte(0x3403, 0xD4);
    cpu.set_byte(0x9900, 0xEE);

    cpu.run_to_done(80);
    assert_eq!(
        cpu.byte_at(0x0020),
        0x12,
        "LDA $FF,X wraps inside page zero"
    );
    assert_eq!(
        cpu.byte_at(0x0021),
        0xC3,
        "the (zp,X) pointer is at $00 / $01"
    );
    assert_eq!(
        cpu.byte_at(0x0022),
        0xD4,
        "and the (zp),Y pointer's high byte wraps to $00"
    );
}

#[test]
fn mos6502_wraps_the_stack_inside_page_one() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        ldx #$00
        txs             ; S = $00
        lda #$aa
        pha             ; $0100, S -> $FF
        lda #$bb
        pha             ; $01FF, S -> $FE
        pla             ; S -> $FF, reads $01FF
        sta $10
        pla             ; S -> $00, reads $0100
        sta $11
done:   jmp done
",
        0,
    );
    cpu.run_instructions(2);
    cpu.record();
    cpu.run_to_done(80);
    assert_eq!(cpu.byte_at(0x0100), 0xAA, "the first push landed at $0100");
    assert_eq!(
        cpu.byte_at(0x01FF),
        0xBB,
        "and S wrapped to $FF for the next"
    );
    assert_eq!(cpu.byte_at(0x0010), 0xBB, "the first pull came back");
    assert_eq!(
        cpu.byte_at(0x0011),
        0xAA,
        "and the second wrapped round again"
    );
    assert_eq!(cpu.s(), 0x00, "S is back where it started");
    // Every write was either in page one or one of the program's own
    // two stores: the stack never reached outside its page.
    for (addr, writing, _) in &cpu.bus {
        assert!(
            !writing || (0x0100..=0x01FF).contains(addr) || matches!(addr, 0x0010 | 0x0011),
            "a stack access reached {addr:#06x}"
        );
    }
}

#[test]
fn mos6502_reproduces_the_indirect_jmp_page_bug() {
    // `JMP ($10FF)` takes its high byte from $1000, not from $1100.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  jmp ($10ff)
        .org $0300
good:   lda #$01
        jmp done
        .org $0400
bad:    lda #$02
done:   jmp done
",
        0,
    );
    cpu.set_byte(0x10FF, 0x00);
    cpu.set_byte(0x1000, 0x03); // what the part actually reads
    cpu.set_byte(0x1100, 0x04); // what a core without the bug would read
    cpu.record();
    let took = cpu.next();
    assert_eq!(took, 5, "an indirect JMP is five cycles");
    assert_eq!(
        cpu.addresses(),
        vec![0x0200, 0x0201, 0x0202, 0x10FF, 0x1000],
        "the second pointer byte comes from the start of the same page"
    );
    assert_eq!(cpu.bus_addr(), 0x0300, "so the jump lands at $0300");
    cpu.run_to_done(40);
    assert_eq!(cpu.a(), 0x01, "which is the branch a compatible core takes");
}

#[test]
fn mos6502_counts_the_cycles_of_every_instruction() {
    // Every documented encoding but the branches, which have their own
    // test because their count depends on the flags. Each index is zero
    // after reset, so no indexed access here crosses a page and each
    // count is the base one from the table.
    fn operand_text(mode: m6502::Mode) -> &'static str {
        match mode {
            m6502::Mode::Imp => "",
            m6502::Mode::Acc => "a",
            m6502::Mode::Imm => "#$12",
            m6502::Mode::Zp => "$34",
            m6502::Mode::ZpX => "$34,x",
            m6502::Mode::ZpY => "$34,y",
            m6502::Mode::Abs => "$1234",
            m6502::Mode::AbsX => "$1234,x",
            m6502::Mode::AbsY => "$1234,y",
            m6502::Mode::Ind => "($1234)",
            m6502::Mode::IzX => "($34,x)",
            m6502::Mode::IzY => "($34),y",
            m6502::Mode::Rel => "done",
        }
    }

    let design = mos6502_design("1");
    let mut checked = 0;
    for insn in m6502::TABLE {
        if insn.mode == m6502::Mode::Rel {
            continue;
        }
        let source = format!(
            "        {} {}\ndone:   jmp done\n",
            insn.name,
            operand_text(insn.mode)
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        // The assembler and the table agree on the encoding, which is
        // also a check that the source above says what it means.
        assert_eq!(
            cpu.byte_at(M6502_ORIGIN),
            insn.code,
            "`{} {}` should assemble to {:#04x}",
            insn.name,
            operand_text(insn.mode),
            insn.code
        );
        let took = cpu.next();
        assert_eq!(
            took,
            u32::from(insn.cycles),
            "`{} {}` ({:#04x}) is documented as {} cycles",
            insn.name,
            operand_text(insn.mode),
            insn.code,
            insn.cycles
        );
        checked += 1;
    }
    assert_eq!(
        checked, 143,
        "every documented encoding but the eight branches"
    );
}

#[test]
fn mos6502_spends_an_extra_cycle_when_an_indexed_read_crosses_a_page() {
    let design = mos6502_design("1");
    let mut checked = 0;
    for insn in m6502::TABLE.iter().filter(|e| e.page_penalty) {
        for (base, crossing) in [(0x12FFu16, true), (0x1200u16, false)] {
            let (setup, operand) = match insn.mode {
                m6502::Mode::AbsX => ("ldx #$01", format!("${base:04x},x")),
                m6502::Mode::AbsY => ("ldy #$01", format!("${base:04x},y")),
                _ => ("ldy #$01", "($40),y".to_owned()),
            };
            let source = format!(
                "        {setup}\n        {} {operand}\ndone:   jmp done\n",
                insn.name
            );
            let mut cpu = Mos::boot(&design, &source, 0);
            if insn.mode == m6502::Mode::IzY {
                cpu.set_byte(0x0040, u8::try_from(base & 0xFF).expect("a byte"));
                cpu.set_byte(0x0041, u8::try_from(base >> 8).expect("a byte"));
            }
            cpu.next();
            let took = cpu.next();
            assert_eq!(
                took,
                u32::from(insn.cycles) + u32::from(crossing),
                "`{} {operand}` with the index at 1 {}crosses a page",
                insn.name,
                if crossing { "" } else { "does not " }
            );
            checked += 1;
        }
    }
    assert!(checked >= 30, "every indexed read is measured: {checked}");

    // An indexed write, and an indexed read-modify-write, spend that
    // cycle whether they cross or not.
    for (mnemonic, operand, want) in [
        ("sta", "$12ff,x", 5),
        ("sta", "$1200,x", 5),
        ("sta", "$12ff,y", 5),
        ("sta", "$1200,y", 5),
        ("inc", "$12ff,x", 7),
        ("inc", "$1200,x", 7),
        ("asl", "$12ff,x", 7),
        ("asl", "$1200,x", 7),
    ] {
        let source = format!(
            "        ldx #$01\n        ldy #$01\n        {mnemonic} {operand}\ndone:   jmp done\n"
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        cpu.run_instructions(2);
        assert_eq!(
            cpu.next(),
            want,
            "`{mnemonic} {operand}` always spends the index cycle"
        );
    }
    // And so does `sta ($40),y`, at six cycles either way.
    for base in [0x12FFu16, 0x1200] {
        let mut cpu = Mos::boot(
            &design,
            "        ldy #$01\n        sta ($40),y\ndone:   jmp done\n",
            0,
        );
        cpu.set_byte(0x0040, u8::try_from(base & 0xFF).expect("a byte"));
        cpu.set_byte(0x0041, u8::try_from(base >> 8).expect("a byte"));
        cpu.next();
        assert_eq!(cpu.next(), 6, "`sta ($40),y` is six cycles");
    }
}

#[test]
fn mos6502_times_branches_by_whether_they_are_taken_and_cross() {
    let design = mos6502_design("1");
    // Not taken: two cycles.
    let mut cpu = Mos::boot(
        &design,
        "
        sec
        bcc away
        nop
away:   nop
done:   jmp done
",
        0,
    );
    cpu.next();
    assert_eq!(cpu.next(), 2, "a branch not taken is two cycles");

    // Taken inside the page: three.
    let mut cpu = Mos::boot(
        &design,
        "
        clc
        bcc away
        nop
away:   nop
done:   jmp done
",
        0,
    );
    cpu.next();
    assert_eq!(cpu.next(), 3, "a branch taken is three");
    assert_eq!(
        cpu.bus_addr(),
        cpu.label("away"),
        "and it lands on its target"
    );

    // Taken onto the next page: four.
    let mut cpu = Mos::boot(
        &design,
        "
        .org $02fa
reset:  clc
        bcc away
        .org $0302
away:   nop
done:   jmp done
",
        0,
    );
    cpu.next();
    assert_eq!(cpu.next(), 4, "a branch across a page is four");
    assert_eq!(cpu.bus_addr(), 0x0302, "and still lands on its target");

    // And backwards across one.
    let mut cpu = Mos::boot(
        &design,
        "
        .org $02f0
away:   nop
        jmp done
        .org $0300
reset:  clc
        bcc away
done:   jmp done
",
        0,
    );
    cpu.next();
    assert_eq!(cpu.next(), 4, "a backward branch across a page is four too");
    assert_eq!(cpu.bus_addr(), 0x02F0);
}

#[test]
fn mos6502_takes_and_declines_every_branch() {
    // Each of the eight branches given a flag state that makes it jump
    // and one that makes it fall through.
    let sets_v = "clc\n        lda #$50\n        adc #$50";
    let cases: &[(&str, &str, bool)] = &[
        ("bpl", "lda #$00", true),
        ("bpl", "lda #$80", false),
        ("bmi", "lda #$80", true),
        ("bmi", "lda #$00", false),
        ("bvc", "clv", true),
        ("bvc", sets_v, false),
        ("bvs", sets_v, true),
        ("bvs", "clv", false),
        ("bcc", "clc", true),
        ("bcc", "sec", false),
        ("bcs", "sec", true),
        ("bcs", "clc", false),
        ("bne", "lda #$01", true),
        ("bne", "lda #$00", false),
        ("beq", "lda #$00", true),
        ("beq", "lda #$01", false),
    ];
    let design = mos6502_design("1");
    for (mnemonic, setup, taken) in cases.iter().copied() {
        let source = format!(
            "        {setup}
        {mnemonic} there
        ldx #$00
        jmp done
there:  ldx #$01
done:   jmp done
"
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        cpu.run_to_done(60);
        assert_eq!(
            cpu.x() == 1,
            taken,
            "`{mnemonic}` after `{setup}` should {}have been taken",
            if taken { "" } else { "not " }
        );
    }
}

#[test]
fn mos6502_calls_and_returns_through_the_stack() {
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        ldx #$ff
        txs
        jsr outer
        sta $20
done:   jmp done

outer:  lda #$11
        jsr inner
        clc
        adc #$22
        rts

inner:  lda #$44
        rts
",
            0,
        );
        cpu.run_instructions(2);
        assert_eq!(
            cpu.s(),
            0xFF,
            "DECIMAL_MODE={decimal}: the stack starts full"
        );
        let jsr_at = M6502_ORIGIN + 3;
        // Six cycles later the return address is on the stack.
        assert_eq!(cpu.next(), 6, "JSR is six cycles");
        assert_eq!(cpu.s(), 0xFD, "and pushes two bytes");
        let pushed = u16::from(cpu.byte_at(0x01FE)) | (u16::from(cpu.byte_at(0x01FF)) << 8);
        assert_eq!(
            pushed,
            jsr_at + 2,
            "JSR pushes the address of its own last byte"
        );
        assert_eq!(
            cpu.bus_addr(),
            cpu.label("outer"),
            "and jumps to its target"
        );

        cpu.run_to_done(200);
        assert_eq!(cpu.a(), 0x66, "$44 from the inner call plus $22");
        assert_eq!(cpu.byte_at(0x0020), 0x66, "stored after the return");
        assert_eq!(cpu.s(), 0xFF, "and every frame was popped again");
    }
}

#[test]
fn mos6502_pushes_and_pulls_the_status_byte() {
    // B is not a flag: PHP pushes bit 4 set, PLP ignores bits 4 and 5.
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        ldx #$ff
        txs
        sec
        sed
        php
        pla
        sta $10
        cld
        clc
        lda #$00
        pha
        plp             ; every flag from a zero byte, B included
        php
        pla
        sta $11
done:   jmp done
",
        0,
    );
    cpu.run_to_done(80);
    // N from `ldx #$ff`, I from the reset sequence, D and C just set.
    assert_eq!(
        cpu.byte_at(0x0010),
        F_N | 0x20 | 0x10 | F_D | F_I | F_C,
        "PHP pushes bit 5 and bit 4 set"
    );
    assert_eq!(
        cpu.byte_at(0x0011),
        0x20 | 0x10,
        "PLP takes no flag from bits 4 and 5, and PHP sets them anyway"
    );
    assert_eq!(
        cpu.p() & (F_C | F_Z | F_N | F_V | F_D | F_I),
        0x00,
        "PLP cleared the rest"
    );
    assert_eq!(cpu.s(), 0xFF, "and the stack is level again");
}

#[test]
fn mos6502_treats_an_undocumented_opcode_as_a_nop() {
    // Out of scope, but not undefined: two cycles and nothing touched.
    let design = mos6502_design("1");
    for code in [0x02u8, 0x03, 0x1A, 0x80, 0xBB, 0xFF] {
        let source = format!(
            "        lda #$42
        ldx #$07
        .byte ${code:02x}
        iny
done:   jmp done
"
        );
        let mut cpu = Mos::boot(&design, &source, 0);
        cpu.run_instructions(2);
        let p = cpu.p();
        assert_eq!(
            cpu.next(),
            2,
            "the undocumented {code:#04x} is a two-cycle NOP"
        );
        assert_eq!(cpu.a(), 0x42, "with the accumulator untouched");
        assert_eq!(cpu.x(), 0x07, "and X");
        assert_eq!(cpu.p(), p, "and the flags");
        cpu.next();
        assert_eq!(cpu.y(), 0x01, "and the instruction after it runs");
    }
}

/// The program the interrupt tests run: a loop that counts in `$30`,
/// with a handler at `$0300` that counts in `$31`.
const MOS_IRQ_PROGRAM: &str = "
        .org $0200
reset:  ldx #$ff
        txs
        cli
loop:   inc $30
        jmp loop
        .org $0300
irq:    inc $31
        rti
";

#[test]
fn mos6502_takes_an_irq_between_instructions() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(&design, MOS_IRQ_PROGRAM, 0);
    let handler = cpu.label("irq");
    let loop_at = cpu.label("loop");
    cpu.run_instructions(3);
    cpu.run(40);
    assert_eq!(cpu.traps, 0, "an unasserted line does not fire");
    let spun = cpu.byte_at(0x0030);
    assert!(spun > 0, "the loop should have gone round");

    // Raised at the top of a known instruction, so the sequence's own
    // seven cycles can be counted off the end of it.
    cpu.run_until_fetch(loop_at, 40);
    cpu.set_irq(true);
    assert_eq!(
        cpu.next(),
        5 + 7,
        "INC zero page's five cycles, then the sequence's seven"
    );
    assert_eq!(cpu.bus_addr(), handler, "the handler is entered");
    assert_eq!(cpu.traps, 1, "exactly one interrupt was taken");
    assert_eq!(cpu.s(), 0xFC, "three bytes on the stack");
    assert!(cpu.flag(2), "I is set on entry");
    let pushed = u16::from(cpu.byte_at(0x01FE)) | (u16::from(cpu.byte_at(0x01FF)) << 8);
    assert!(
        (loop_at..loop_at + 5).contains(&pushed),
        "the pushed address is an instruction of the loop, not {pushed:#06x}"
    );
    let status = cpu.byte_at(0x01FD);
    assert_eq!(status & 0x10, 0x00, "an interrupt pushes bit 4 clear");
    assert_eq!(status & 0x20, 0x20, "and bit 5 set");
    assert_eq!(status & F_I, 0x00, "with I as it was before the interrupt");

    // The line is still high, so it fires again as soon as RTI puts I
    // back: it is level triggered, not an edge.
    cpu.run_until_traps(2, 120);
    assert_eq!(
        cpu.bus_addr(),
        handler,
        "a level that stays high fires again"
    );
    cpu.set_irq(false);
    let before = cpu.traps;
    cpu.run(200);
    assert_eq!(cpu.traps, before, "and stops once the line drops");
    assert!(
        cpu.byte_at(0x0030) > spun,
        "the interrupted loop carried on"
    );
    assert_eq!(cpu.byte_at(0x0031), 2, "the handler ran twice");
}

#[test]
fn mos6502_masks_an_irq_with_the_interrupt_flag() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        sei
loop:   inc $30
        jmp loop
        .org $0300
irq:    inc $31
        rti
",
        0,
    );
    cpu.run_instructions(3);
    cpu.set_irq(true);
    cpu.run(300);
    assert_eq!(cpu.traps, 0, "I masks the IRQ however long it is held");
    assert_eq!(cpu.byte_at(0x0031), 0, "the handler never ran");
    assert!(cpu.byte_at(0x0030) > 4, "and the loop kept going");
}

#[test]
fn mos6502_takes_an_nmi_on_its_edge_and_through_the_mask() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        sei             ; I is set: an NMI does not care
loop:   inc $30
        jmp loop
        .org $0300
nmi:    inc $31
        rti
",
        0,
    );
    let handler = cpu.label("nmi");
    let loop_at = cpu.label("loop");
    cpu.run_instructions(3);

    // A held-high line is one edge and therefore one interrupt, even
    // though I is set throughout.
    cpu.set_nmi(true);
    cpu.run_until_traps(1, 40);
    assert_eq!(
        cpu.bus_addr(),
        handler,
        "the NMI ignores the interrupt mask"
    );
    assert_eq!(cpu.byte_at(0x01FD) & 0x10, 0, "and pushes bit 4 clear");
    cpu.run(300);
    assert_eq!(cpu.traps, 1, "a line that stays high is still one edge");
    assert_eq!(cpu.byte_at(0x0031), 1, "so the handler ran once");

    // A second edge is a second interrupt.
    cpu.set_nmi(false);
    cpu.run(4);
    cpu.set_nmi(true);
    cpu.run_until_traps(2, 60);
    assert_eq!(cpu.bus_addr(), handler, "a fresh edge fires again");

    // And an edge narrower than an instruction is latched, not lost.
    cpu.set_nmi(false);
    cpu.run(4);
    cpu.run_until_fetch(loop_at, 40);
    cpu.set_nmi(true);
    cpu.run(1);
    cpu.set_nmi(false);
    cpu.run_until_traps(3, 60);
    assert_eq!(
        cpu.bus_addr(),
        handler,
        "a one-cycle pulse is held until it is taken"
    );
}

#[test]
fn mos6502_prefers_an_nmi_to_an_irq() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        cli
loop:   inc $30
        jmp loop
        .org $0300
nmi:    inc $31
        jmp done
        .org $0380
irq:    inc $32
done:   jmp done
",
        0,
    );
    let nmi_at = cpu.label("nmi");
    cpu.run_instructions(3);
    cpu.set_irq(true);
    cpu.set_nmi(true);
    cpu.run_until_traps(1, 40);
    assert_eq!(cpu.bus_addr(), nmi_at, "the NMI vector won");
    cpu.run_instructions(1);
    assert_eq!(cpu.byte_at(0x0031), 1, "and its handler ran");
    assert_eq!(cpu.byte_at(0x0032), 0, "while the IRQ handler did not");
}

#[test]
fn mos6502_breaks_and_returns_from_the_interrupt() {
    let design = mos6502_design("1");
    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        cli
        sec
        brk
        .byte $ee       ; BRK's padding byte, which it skips
back:   lda #$99
        sta $20
done:   jmp done
        .org $0400
irq:    inc $30
        rti
",
        0,
    );
    let handler = cpu.label("irq");
    let back = cpu.label("back");
    cpu.run_instructions(4);
    assert_eq!(cpu.next(), 7, "BRK is seven cycles");
    assert_eq!(cpu.bus_addr(), handler, "and vectors through $FFFE");
    assert_eq!(cpu.traps, 1);
    let pushed = u16::from(cpu.byte_at(0x01FE)) | (u16::from(cpu.byte_at(0x01FF)) << 8);
    assert_eq!(pushed, back, "BRK pushes the address past its padding byte");
    let status = cpu.byte_at(0x01FD);
    assert_eq!(
        status & 0x10,
        0x10,
        "and pushes bit 4 set, which an IRQ does not"
    );
    assert_eq!(status & F_I, 0, "with I as it was");
    assert_eq!(status & F_C, F_C, "and the carry it was left with");
    assert!(cpu.flag(2), "the handler runs with I set");

    cpu.run_to_done(60);
    assert_eq!(cpu.byte_at(0x0030), 1, "the handler ran");
    assert_eq!(cpu.a(), 0x99, "RTI came back past the padding byte");
    assert_eq!(cpu.byte_at(0x0020), 0x99);
    assert!(!cpu.flag(2), "RTI restored I to what it was");
    assert!(cpu.flag(0), "and the carry with it");
    assert_eq!(cpu.s(), 0xFF, "and the frame was popped");
}

#[test]
fn mos6502_delays_the_effect_of_cli_and_sei_by_one_instruction() {
    // The interrupt decision is made with the flags as they were before
    // the instruction, which is the delay the part is documented to
    // have: CLI does not let one through until the instruction after
    // it, and SEI does not shut one out until then either.
    let design = mos6502_design("1");

    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        sei
        nop
clear:  cli
after:  inc $30         ; this runs before the interrupt is taken
        inc $31
done:   jmp done
        .org $0300
irq:    inc $32
        rti
",
        0,
    );
    let cli_at = cpu.label("clear");
    let after = cpu.label("after");
    let handler = cpu.label("irq");
    cpu.set_irq(true);
    cpu.run_until_fetch(cli_at, 40);
    assert_eq!(cpu.traps, 0, "I keeps it out while it is set");
    cpu.next();
    assert_eq!(cpu.bus_addr(), after, "CLI does not let it in at once");
    assert_eq!(cpu.traps, 0);
    cpu.next();
    assert_eq!(cpu.byte_at(0x0030), 1, "the instruction after CLI ran");
    assert_eq!(cpu.bus_addr(), handler, "and only then was it taken");

    let mut cpu = Mos::boot(
        &design,
        "
        .org $0200
reset:  ldx #$ff
        txs
        cli
block:  sei
        inc $30
done:   jmp done
        .org $0300
irq:    inc $32
        rti
",
        0,
    );
    let sei_at = cpu.label("block");
    let handler = cpu.label("irq");
    cpu.run_until_fetch(sei_at, 40);
    // The line goes high during SEI itself, so the decision at the end
    // of SEI is made with I still clear.
    cpu.set_irq(true);
    cpu.next();
    assert!(cpu.flag(2), "SEI set the flag");
    assert_eq!(
        cpu.bus_addr(),
        handler,
        "and the interrupt it was meant to keep out is taken anyway"
    );
}

#[test]
fn mos6502_multiplies_sixteen_bits_by_shift_and_add() {
    // 16 x 16 into 16, the way a 6502 does it: shift the multiplier
    // right a bit at a time, add the shifted multiplicand when the bit
    // was set, all of it in a subroutine reached through the stack.
    const MULTIPLY: &str = "
        .org $0200
reset:  ldx #$ff
        txs
        jsr mul16
        lda $14
        sta $20
        lda $15
        sta $21
done:   jmp done

mul16:  lda #$00
        sta $14
        sta $15
        ldx #$10        ; sixteen bits of multiplier
ml:     lsr $13         ; multiplier >>= 1, its low bit into C
        ror $12
        bcc noadd
        clc
        lda $14         ; product += multiplicand
        adc $10
        sta $14
        lda $15
        adc $11
        sta $15
noadd:  asl $10         ; multiplicand <<= 1
        rol $11
        dex
        bne ml
        rts
";
    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        for (a, b) in [
            (1234u16, 53u16),
            (300, 200),
            (0xFFFF, 3),
            (0, 12345),
            (7, 1),
        ] {
            let mut cpu = Mos::boot(&design, MULTIPLY, 0);
            for (addr, value) in [(0x0010u16, a), (0x0012, b)] {
                cpu.set_byte(addr, u8::try_from(value & 0xFF).expect("a byte"));
                cpu.set_byte(addr + 1, u8::try_from(value >> 8).expect("a byte"));
            }
            cpu.run_to_done(4000);
            assert_eq!(
                cpu.word_at(0x0020),
                a.wrapping_mul(b),
                "DECIMAL_MODE={decimal}: {a} x {b}"
            );
            assert_eq!(cpu.s(), 0xFF, "and the stack came back");
            assert_eq!(cpu.traps, 0, "with nothing trapping on the way");
        }
    }
}

#[test]
fn mos6502_sums_an_array_into_sixteen_bits() {
    // A loop over sixteen bytes, each added through a subroutine that
    // carries into the high byte, so the whole call sequence is on the
    // stack for every one of them.
    let values: [u8; 16] = [
        0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0,
        0xFF,
    ];
    let total: u16 = values.iter().map(|v| u16::from(*v)).sum();

    for decimal in DECIMALS {
        let design = mos6502_design(decimal);
        let mut cpu = Mos::boot(
            &design,
            "
        .org $0200
reset:  ldx #$ff
        txs
        lda #$00
        sta $30
        sta $31
        ldy #$00
loop:   lda $0400,y
        jsr add8
        iny
        cpy #$10
        bne loop
done:   jmp done

add8:   clc
        adc $30
        sta $30
        lda $31
        adc #$00
        sta $31
        rts
",
            0,
        );
        for (index, value) in values.iter().enumerate() {
            cpu.set_byte(0x0400 + u16::try_from(index).expect("sixteen"), *value);
        }
        cpu.run_to_done(4000);
        assert_eq!(
            cpu.word_at(0x0030),
            total,
            "DECIMAL_MODE={decimal}: the sixteen-bit total"
        );
        assert_eq!(cpu.y(), 0x10, "the index walked the whole array");
        assert_eq!(cpu.s(), 0xFF, "and every call returned");
    }
}

#[test]
fn mos6502_runs_a_recursive_function_on_the_stack() {
    // Fibonacci by the definition: two nested calls per frame, with the
    // argument and the first result kept on the stack and read back
    // through `tsx` and absolute,X, which is how a 6502 reaches its own
    // frame. Nothing here fits in a register, so the whole calling
    // convention is proved at once.
    const FIB: &str = "
        .org $0200
reset:  ldx #$ff
        txs
        lda #$0a
        jsr fib
        sta $20
done:   jmp done

fib:    cmp #$02
        bcc base        ; fib(0) = 0 and fib(1) = 1 are the argument
        pha             ; [n]
        sec
        sbc #$01
        jsr fib         ; A = fib(n-1)
        pha             ; [n][fib(n-1)]
        tsx
        lda $0102,x     ; n again, out of this frame
        sec
        sbc #$02
        jsr fib         ; A = fib(n-2)
        tsx
        clc
        adc $0101,x     ; + fib(n-1)
        tay
        pla             ; drop fib(n-1)
        pla             ; drop n
        tya
base:   rts
";
    fn fib(n: u32) -> u32 {
        if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
    }

    let design = mos6502_design("1");
    let mut cpu = Mos::boot(&design, FIB, 0);
    cpu.run_to_done(120_000);
    assert_eq!(
        u32::from(cpu.byte_at(0x0020)),
        fib(10),
        "fib(10) computed recursively"
    );
    assert_eq!(cpu.s(), 0xFF, "every frame was popped again");
    assert_eq!(cpu.traps, 0, "and nothing faulted on the way");
    assert!(
        cpu.retired > 1000,
        "the whole recursion ran: {} instructions",
        cpu.retired
    );
}

#[test]
fn mos6502_survives_a_memory_that_makes_it_wait() {
    // `ready` low holds the access; the core must take the same number
    // of bus *cycles* however many clocks each one costs.
    let design = mos6502_design("1");
    let mut counts = Vec::new();
    for stalls in [0u32, 1, 3] {
        let mut cpu = Mos::boot(
            &design,
            "
        .org $0200
reset:  ldx #$ff
        txs
        ldy #$00
        lda #$00
loop:   clc
        adc $0400,y
        iny
        cpy #$08
        bne loop
        sta $20
done:   jmp done
",
            stalls,
        );
        for index in 0..8u16 {
            cpu.set_byte(0x0400 + index, u8::try_from(index + 1).expect("a byte"));
        }
        let cycles = cpu.run_to_done(2000);
        assert_eq!(
            cpu.byte_at(0x0020),
            36,
            "1 + 2 + … + 8, with {stalls} wait states"
        );
        counts.push(cycles);
    }
    assert_eq!(
        counts[0], counts[1],
        "a wait state costs clocks, not bus cycles"
    );
    assert_eq!(counts[0], counts[2]);
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
// vga_out
// ---------------------------------------------------------------------------

/// `vga_out` at one mode and one colour width, out of reset, with every
/// clock edge a pixel.
///
/// The five output pins are registered, so they run one pixel behind the
/// raster: a `tick` passes the pixel `de`, `x` and `y` were naming, and
/// afterwards the pins carry *that* pixel's colour and syncs. Every test
/// below reads the fetch side before the tick and the pins after it,
/// which is why neither ever has to name the delay again.
struct Vga<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    de: NetHandle,
    x: NetHandle,
    y: NetHandle,
    colour: [NetHandle; 3],
    out: [NetHandle; 3],
    hsync: NetHandle,
    vsync: NetHandle,
}

impl<'d> Vga<'d> {
    fn new(design: &'d Design) -> Vga<'d> {
        let mut sim = simulate(design, "vga_out");
        let clk = top_net(&sim, "clk");
        let rst_n = top_net(&sim, "rst_n");
        let pix_en = top_net(&sim, "pix_en");
        let de = top_net(&sim, "de");
        let x = top_net(&sim, "x");
        let y = top_net(&sim, "y");
        let colour = [top_net(&sim, "r"), top_net(&sim, "g"), top_net(&sim, "b")];
        let out = [
            top_net(&sim, "vga_r"),
            top_net(&sim, "vga_g"),
            top_net(&sim, "vga_b"),
        ];
        let hsync = top_net(&sim, "vga_hsync");
        let vsync = top_net(&sim, "vga_vsync");
        // One pixel per clock: the block takes its pixel rate as an
        // enable, so a test that wants every edge to be a pixel ties it
        // high, exactly as a design on a true pixel clock would.
        sim.set(pix_en, bit(true));
        reset(&mut sim, clk, rst_n);
        Vga {
            sim,
            clk,
            de,
            x,
            y,
            colour,
            out,
            hsync,
            vsync,
        }
    }

    /// The colour offered for the pixel the raster is naming now.
    fn drive(&mut self, rgb: [u64; 3]) {
        for (net, value) in self.colour.iter().zip(rgb) {
            self.sim.set(*net, word(8, value));
        }
    }

    /// Passes one pixel.
    fn tick(&mut self) {
        cycle(&mut self.sim, self.clk, HALF);
    }

    /// What the three colour pins carry.
    fn pins(&self) -> [u64; 3] {
        self.out.map(|net| get_u64(&self.sim, net))
    }
}

/// `vga_out` built for one mode, with the Basys 3's four bits a channel.
fn vga_design(mode: &str, bpc: &str) -> Design {
    design_of("vga_out", "vga_out", &[("MODE", mode), ("BPC", bpc)])
}

/// The raster on the pins is the mode's raster.
///
/// The numbers come from `VIDEO_MODES` above, which is VESA's table
/// typed out, and not from the RTL: 640 active pixels, 16 front porch,
/// 96 sync and 48 back porch, and 480 lines with 10, 2 and 33. Every one
/// of the 420 000 pixel slots of a frame is checked — `de`, `x` and `y`
/// on the fetch side, hsync and vsync on the pins a monitor would see —
/// and the frame is followed round to the top left pixel of the next.
#[test]
fn vga_out_draws_the_raster_of_its_mode() {
    let m = &VIDEO_MODES[0];
    let design = vga_design(m.mode, "4");
    let mut vga = Vga::new(&design);
    let h_total: u64 = m.h.iter().sum();
    let v_total: u64 = m.v.iter().sum();
    let active = |level: bool| level == m.sync_high;

    let mut visible = 0u64;
    let mut hsync_pixels = 0u64;
    let mut vsync_lines = BTreeSet::new();
    for line in 0..v_total {
        for pixel in 0..h_total {
            let on = high(&vga.sim, vga.de);
            let want = pixel < m.h[0] && line < m.v[0];
            assert_eq!(on, want, "de at ({pixel}, {line})");
            if on {
                visible += 1;
                assert_eq!(get_u64(&vga.sim, vga.x), pixel, "x at ({pixel}, {line})");
                assert_eq!(get_u64(&vga.sim, vga.y), line, "y at ({pixel}, {line})");
            }
            vga.tick();

            // The pins now carry the pixel that just went by.
            let h_sync = pixel >= m.h[0] + m.h[1] && pixel < m.h[0] + m.h[1] + m.h[2];
            assert_eq!(
                active(high(&vga.sim, vga.hsync)),
                h_sync,
                "hsync at ({pixel}, {line})"
            );
            if h_sync && line == 0 {
                hsync_pixels += 1;
            }
            let v_sync = line >= m.v[0] + m.v[1] && line < m.v[0] + m.v[1] + m.v[2];
            assert_eq!(
                active(high(&vga.sim, vga.vsync)),
                v_sync,
                "vsync at ({pixel}, {line})"
            );
            if v_sync {
                vsync_lines.insert(line);
            }
        }
    }
    assert_eq!(visible, m.h[0] * m.v[0], "visible pixels");
    assert_eq!(hsync_pixels, m.h[2], "hsync width");
    assert_eq!(vsync_lines.len() as u64, m.v[2], "vsync lines");
    assert_eq!(
        get_u64(&vga.sim, vga.x),
        0,
        "the next frame starts at x = 0"
    );
    assert_eq!(
        get_u64(&vga.sim, vga.y),
        0,
        "the next frame starts at y = 0"
    );
}

/// The colour pins are black everywhere outside the active area.
///
/// This is the one behaviour a VGA block has that a DVI block does not
/// need: a monitor takes its black level from the back porch, so colour
/// driven during blanking makes it lose sync or drag that level until
/// the picture rolls. The test drives white into *every* pixel slot of a
/// whole frame, blanking and all, so nothing but the block's own gate
/// can keep it off the pins, and counts the two intervals separately —
/// 76 800 pixels of horizontal blanking on the visible lines and 36 000
/// on the lines of the vertical blanking — so a gate that caught only
/// one of them would fail here.
#[test]
fn vga_out_forces_black_through_both_blanking_intervals() {
    let m = &VIDEO_MODES[0];
    let design = vga_design(m.mode, "4");
    let mut vga = Vga::new(&design);
    let h_total: u64 = m.h.iter().sum();
    let v_total: u64 = m.v.iter().sum();
    // Four bits a channel, so full scale on the ladder is 15.
    let white = [15u64; 3];
    let black = [0u64; 3];
    vga.drive([0xFF; 3]);

    let mut lit = 0u64;
    let mut blanked_across = 0u64;
    let mut blanked_below = 0u64;
    for line in 0..v_total {
        for pixel in 0..h_total {
            let visible = pixel < m.h[0] && line < m.v[0];
            vga.tick();
            assert_eq!(
                vga.pins(),
                if visible { white } else { black },
                "the colour at ({pixel}, {line})"
            );
            if visible {
                lit += 1;
            } else if line < m.v[0] {
                blanked_across += 1;
            } else {
                blanked_below += 1;
            }
        }
    }
    assert_eq!(lit, m.h[0] * m.v[0], "the picture");
    assert_eq!(
        blanked_across,
        m.v[0] * (h_total - m.h[0]),
        "the horizontal blanking of every visible line"
    );
    assert_eq!(
        blanked_below,
        (v_total - m.v[0]) * h_total,
        "every line of the vertical blanking"
    );
}

/// Each mode's syncs pulse the way that mode says, at the pins.
///
/// Mode 0's are negative and modes 1 and 2's are positive, which is
/// `video_timing`'s rule and is not restated in `vga_out` except as the
/// level its output register resets to. This test pins both statements
/// to the same value: the pin's level out of reset, before any pixel has
/// gone by, must be the level it holds through the back porch of a real
/// line, and the pulse in between must be the other one, for exactly as
/// many pixels as the mode's sync is wide.
///
/// It costs one line per mode rather than one frame, so vsync is checked
/// here only for staying idle through the picture; `vga_out` passes it
/// through the same register as hsync, and
/// `vga_out_draws_the_raster_of_its_mode` follows it through a whole
/// 640x480 frame.
#[test]
fn vga_out_syncs_idle_at_the_polarity_of_the_mode() {
    for m in &VIDEO_MODES {
        let design = vga_design(m.mode, "4");
        let mut vga = Vga::new(&design);
        let idle = !m.sync_high;
        assert_eq!(
            high(&vga.sim, vga.hsync),
            idle,
            "mode {}: hsync out of reset",
            m.mode
        );
        assert_eq!(
            high(&vga.sim, vga.vsync),
            idle,
            "mode {}: vsync out of reset",
            m.mode
        );

        let h_total: u64 = m.h.iter().sum();
        let mut pulse = 0u64;
        for pixel in 0..h_total {
            vga.tick();
            let pulsing = pixel >= m.h[0] + m.h[1] && pixel < m.h[0] + m.h[1] + m.h[2];
            assert_eq!(
                high(&vga.sim, vga.hsync),
                if pulsing { m.sync_high } else { idle },
                "mode {}: hsync at pixel {pixel}",
                m.mode
            );
            if pulsing {
                pulse += 1;
            }
            assert_eq!(
                high(&vga.sim, vga.vsync),
                idle,
                "mode {}: vsync moved on line 0",
                m.mode
            );
        }
        assert_eq!(pulse, m.h[2], "mode {}: hsync width", m.mode);
    }
}

/// A known pattern in, the pixels out, read off the pins.
///
/// Sixty-four consecutive pixels of the first visible line, each a
/// different colour, with a different ramp on each channel so that two
/// channels swapped anywhere between the port and the pin would show.
/// The same pattern is put through three colour widths: the Basys 3's
/// four bits a channel, the eight of a board with a real DAC, and one,
/// which is a board with no ladder at all.
///
/// What the pins must carry is the **top** `BPC` bits of each byte:
/// `vga_out` truncates rather than rounds, so `8'h00` is 0 and `8'hFF`
/// is full scale at every width, and both of those appear in the ramps
/// below at pixel 0.
#[test]
fn vga_out_truncates_the_colour_to_the_boards_width() {
    for (bpc, width) in [(4u32, "4"), (8, "8"), (1, "1")] {
        let design = vga_design("0", width);
        let mut vga = Vga::new(&design);
        for pixel in 0..64u64 {
            assert!(high(&vga.sim, vga.de), "pixel {pixel} of line 0 is visible");
            assert_eq!(get_u64(&vga.sim, vga.x), pixel);
            assert_eq!(get_u64(&vga.sim, vga.y), 0);
            let rgb = [pixel * 4, 255 - pixel * 4, pixel * 3 + 7];
            vga.drive(rgb);
            vga.tick();
            assert_eq!(
                vga.pins(),
                rgb.map(|v| v >> (8 - bpc)),
                "pixel {pixel} at {bpc} bit(s) a channel"
            );
        }
    }
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

/// The far end of the D+ / D- pair, as a host reaches it.
///
/// `usb_device_fs` drives the pair itself, so for it this is the device's
/// own pins. `usb_device_ulpi` cannot: its lines belong to a transceiver,
/// and what sits between the host and the device is a model of that
/// transceiver. The host model above is the same either way, which is
/// the point of the trait — the ULPI core is enumerated by the host that
/// enumerates the full-speed one, packets, CRCs, drift and all.
trait UsbPair {
    /// Cycles of the device's clock in one full-speed bit time.
    fn cycles_per_bit(&self) -> u64;
    /// One device clock cycle. `host` is the state the host drives onto
    /// the pair, or `None` while the host has let go and is listening.
    fn cycle(&mut self, host: Option<UsbLine>);
    /// What the far end drives onto the pair, if it drives at all.
    fn driven(&mut self) -> Option<UsbLine>;
    /// The address the device has taken.
    fn address(&self) -> u64;
    fn configured(&self) -> bool;
    /// Whether the device is reporting a bus reset this cycle.
    fn usb_reset(&self) -> bool;
    /// Anything the far end did that its own protocol forbids.
    fn problems(&self) -> &[String];
    /// The cycles an answer may take, which is a property of what is
    /// between the host and the device.
    fn answer_window(&self) -> (u64, u64);

    /// Cycles the layer between the host and the device added to each answer,
    /// oldest first, which the **device** must not be blamed for.
    ///
    /// Nothing sits between the host and `usb_device_fs`, so the default is
    /// nothing. A ULPI transceiver does, and the model of one in this file is
    /// **store and forward**: it takes the Link's bytes at the interface's
    /// rate and only puts the packet on the pair once `stp` has ended it, so
    /// the delay the host measures is the device's turnaround **plus one
    /// interface clock per byte of the answer**. A real transceiver serialises
    /// as the bytes arrive and adds a fixed latency instead.
    ///
    /// Subtracting it is what keeps the inter-packet delay USB 2.0 §7.1.18
    /// allows — 2 to 6.5 bit times — a tight bound on the **device** through
    /// both link layers. It used to need no subtracting because a packet was
    /// eight bytes and eleven cycles of overhead still fitted inside 6.5 bit
    /// times of a 60 MHz clock; at 64 bytes it does not, and widening the
    /// window to 67 cycles instead would have made it vacuous, since the host
    /// gives up after 18 bit times anyway.
    fn added_delay(&self) -> Vec<u64> {
        Vec::new()
    }
    /// The data endpoint's byte interface, and the logic above it.
    fn data(&mut self) -> &mut DataEp;
    /// One of the block's own top-level nets, by name.
    ///
    /// The bus is what most of these tests read, but a class layer reports
    /// things that never reach the bus — what line coding the host asked
    /// for, whether it raised DTR — and those are ports. A named lookup
    /// keeps the trait from growing a method per signal of every future
    /// class.
    fn port(&self, name: &str) -> u64;

    /// Drive one of the block's own top-level inputs, by name.
    ///
    /// The mirror of `port`, and it exists for `serial_state`: what a CDC ACM
    /// device reports as its line state is an **input** of the class layer, so
    /// a test that wants a notification to say something has to say it. A
    /// named lookup for the same reason `port` is one, and it panics on a name
    /// the block does not have, which is what a test driving the wrong port
    /// deserves.
    fn set_port(&mut self, name: &str, value: u64, bits: u32);

    /// Fills one of the design's memories with zeros, by hierarchical
    /// name, and says whether it found one.
    ///
    /// A distributed RAM cannot be given initial contents —
    /// `fpga::primitives` declines to lower a memory that has any — so in
    /// simulation every word of one is `x` until something writes it,
    /// while a real part comes up holding whatever its cells held, which
    /// is not `x`. A test whose device *reads* its own RAM has to say
    /// which of those two it is modelling, and this is how it says zero.
    fn clear_memory(&mut self, _path: &str) -> bool {
        false
    }
}

/// The data endpoint's byte interface, and whatever stands in for the logic
/// above it.
///
/// `usb_bulk_ep` hands bytes over with `out_valid` / `out_ready` / `out_last`
/// and takes them with `in_valid` / `in_ready` / `in_commit`, and every one
/// of its outputs is a function of registers alone — no input of this
/// interface reaches any of them combinationally — so a testbench may read
/// the outputs and drive the inputs in the same cycle without a loop. That
/// is a property of the endpoint and not a convenience: a top level wires
/// `out_ready` to `in_ready` on the board, and if either side were
/// combinational in the other that would be one.
///
/// Two modes, because they prove different things. With `loopback` the OUT
/// stream is wired into the IN stream, which is the design that goes on the
/// part. Without it, this struct **is** the logic above the endpoint: it
/// takes every byte the host sends and hands back packets from a queue, so
/// the interface is exercised as a user of the block would, and a byte
/// arriving at the wrong index or a packet boundary in the wrong place is
/// visible here and not only in what the host reads back.
struct DataEp {
    out_data: NetHandle,
    out_valid: NetHandle,
    out_last: NetHandle,
    out_ready: NetHandle,
    in_data: NetHandle,
    in_valid: NetHandle,
    in_ready: NetHandle,
    in_commit: NetHandle,
    /// Whether the design brings this interface out to its top at all.
    ///
    /// False for a design whose logic above the endpoint is inside the
    /// module — a whole computer, say — and then nothing here drives or
    /// reads anything. The bus is still the bus.
    present: bool,
    loopback: bool,
    /// Whether the logic above the endpoint is taking the bytes at all.
    /// False is a consumer that has stopped, which is what makes the
    /// endpoint NAK: `loopback` has no use for it, since there the IN
    /// side's room is the flow control.
    take: bool,
    /// Packets the OUT endpoint delivered, cut where `out_last` fell.
    got: Vec<Vec<u8>>,
    /// The packet being collected.
    partial: Vec<u8>,
    /// Packets to give the IN endpoint, front first.
    give: Vec<Vec<u8>>,
    /// Bytes of `give[0]` handed over so far.
    at: usize,
}

impl DataEp {
    /// The byte interface of a design that brings it out to its top.
    ///
    /// `present` is false when the design does not — when the thing above
    /// the endpoint is **inside** the module rather than in the test, as
    /// it is for a design that contains a whole computer. Then nothing
    /// here drives or reads anything, every handle is a stand-in, and the
    /// bus is still the bus: `UsbHost` reaches the device through the
    /// pair and not through this.
    fn new(sim: &Simulator<'_>, loopback: bool) -> DataEp {
        let path = format!("{}.out_data", sim.top_name());
        let Some(out_data) = sim.net(&path) else {
            let stand_in = top_net(sim, "rst_n");
            return DataEp {
                out_data: stand_in,
                out_valid: stand_in,
                out_last: stand_in,
                out_ready: stand_in,
                in_data: stand_in,
                in_valid: stand_in,
                in_ready: stand_in,
                in_commit: stand_in,
                present: false,
                loopback,
                take: true,
                got: Vec::new(),
                partial: Vec::new(),
                give: Vec::new(),
                at: 0,
            };
        };
        let pin = |n: &str| top_net(sim, n);
        DataEp {
            out_data,
            out_valid: pin("out_valid"),
            out_last: pin("out_last"),
            out_ready: pin("out_ready"),
            in_data: pin("in_data"),
            in_valid: pin("in_valid"),
            in_ready: pin("in_ready"),
            in_commit: pin("in_commit"),
            present: true,
            loopback,
            take: true,
            got: Vec::new(),
            partial: Vec::new(),
            give: Vec::new(),
            at: 0,
        }
    }

    /// Every input of the interface low, which is what a design that only
    /// enumerates leaves them at.
    fn quiet(&self, sim: &mut Simulator<'_>) {
        if !self.present {
            return;
        }
        sim.set(self.out_ready, bit(false));
        sim.set(self.in_valid, bit(false));
        sim.set(self.in_data, word(8, 0));
        sim.set(self.in_commit, bit(false));
    }
}

/// One cycle of whatever is above the data endpoint, driven before the clock
/// edge the handshakes complete on.
fn step_data(sim: &mut Simulator<'_>, ep: &mut DataEp) {
    if !ep.present {
        return;
    }
    let out_valid = high(sim, ep.out_valid);
    let out_last = high(sim, ep.out_last);
    // Only while `out_valid`. A ready/valid data bus says nothing about its
    // bytes when its valid is low, and this one now comes out of a memory
    // whose contents before the first write are whatever the fabric came up
    // holding — X in simulation, and on an ECP5 whatever `dpram_init_word`
    // put there. A consumer that latched it anyway would be reading a byte
    // the endpoint never claimed to have.
    let out_byte = if out_valid {
        octet(get_u64(sim, ep.out_data))
    } else {
        0
    };
    let in_ready = high(sim, ep.in_ready);

    // In loopback the byte only moves when the IN side has room for it,
    // which is the whole of the flow control a top level does.
    let out_ready = if ep.loopback { in_ready } else { ep.take };
    sim.set(ep.out_ready, bit(out_ready));

    let moved = out_valid && out_ready;
    if moved {
        ep.partial.push(out_byte);
        if out_last {
            let packet = std::mem::take(&mut ep.partial);
            ep.got.push(packet);
        }
    }

    if ep.loopback {
        sim.set(ep.in_valid, bit(out_valid));
        sim.set(ep.in_data, word(8, u64::from(out_byte)));
        sim.set(ep.in_commit, bit(moved && out_last));
        return;
    }

    let mut valid = false;
    let mut commit = false;
    let mut byte = 0u8;
    if in_ready && !ep.give.is_empty() {
        let packet = &ep.give[0];
        if ep.at < packet.len() {
            byte = packet[ep.at];
            valid = true;
            commit = ep.at + 1 == packet.len();
        } else {
            // Nothing to give: a zero-length packet is the commit alone.
            commit = true;
        }
        ep.at += 1;
        if commit {
            ep.give.remove(0);
            ep.at = 0;
        }
    }
    sim.set(ep.in_valid, bit(valid));
    sim.set(ep.in_data, word(8, u64::from(byte)));
    sim.set(ep.in_commit, bit(commit));
}

/// A USB host on the other end of the pair: it sends real packets —
/// NRZI, stuffed, with their CRCs — a bit every few cycles of the
/// device's clock, stretched or shortened now and then as a host
/// clock a little off the device's would, and it decodes what comes
/// back the way a host does, checking the SYNC field, the stuffing, the
/// EOP, the PID check nibble and the CRC16, and measuring how long the
/// device took to answer.
struct UsbHost<P> {
    pair: P,
    /// Every how many bits the host's bit is a cycle long or short: a
    /// positive number stretches, a negative one shortens, zero never.
    drift: i32,
    bits_sent: u64,
    /// Turnaround gaps the device took before answering, in cycles.
    gaps: Vec<u64>,
    problems: Vec<String>,
}

impl<P: UsbPair> UsbHost<P> {
    fn new(pair: P, drift: i32) -> UsbHost<P> {
        UsbHost {
            pair,
            drift,
            bits_sent: 0,
            gaps: Vec::new(),
            problems: Vec::new(),
        }
    }

    fn address(&self) -> u64 {
        self.pair.address()
    }

    fn configured(&self) -> bool {
        self.pair.configured()
    }

    /// One cycle with the host driving `line`; the device must not
    /// drive at the same time.
    fn host_cycle(&mut self, line: UsbLine) {
        self.pair.cycle(Some(line));
        if self.pair.driven().is_some() {
            self.problems
                .push("the device drives the pair while the host does".into());
        }
    }

    /// The cycles the host's next bit lasts.
    fn bit_cycles(&mut self) -> u64 {
        self.bits_sent += 1;
        let nominal = self.pair.cycles_per_bit();
        let every = u64::from(self.drift.unsigned_abs());
        if every != 0 && self.bits_sent.is_multiple_of(every) {
            if self.drift > 0 {
                nominal + 1
            } else {
                nominal - 1
            }
        } else {
            nominal
        }
    }

    fn send_line(&mut self, line: &[UsbLine]) {
        for state in line {
            for _ in 0..self.bit_cycles() {
                self.host_cycle(*state);
            }
        }
    }

    fn send(&mut self, packet: &[u8]) {
        let line = usb_line(packet, true);
        self.send_line(&line);
    }

    /// Idles the bus for `bits` bit times, the gap a host leaves between
    /// its own packets.
    fn idle(&mut self, bits: u64) {
        for _ in 0..self.pair.cycles_per_bit() * bits {
            self.host_cycle(UsbLine::J);
        }
    }

    /// Waits up to eighteen bit times — the host's turnaround timeout —
    /// for the device to answer, and decodes what it sends.
    fn receive(&mut self) -> UsbReply {
        let cpb = self.pair.cycles_per_bit();
        let mut waited = 0u64;
        while self.pair.driven().is_none() {
            if waited > 18 * cpb {
                return UsbReply::Nothing;
            }
            self.pair.cycle(None);
            waited += 1;
        }
        self.gaps.push(waited);

        // Record the pair for as long as the far end drives it.
        let mut states = Vec::new();
        while let Some(state) = self.pair.driven() {
            states.push(state);
            self.pair.cycle(None);
            // A 64-byte data packet is 67 bytes on the wire — SYNC, PID,
            // payload, CRC16 and the EOP — so 550 bit times, and more with
            // stuffing. This is a runaway guard and not a limit on a packet.
            if states.len() as u64 > cpb * 700 {
                self.problems
                    .push("the device never let go of the bus".into());
                break;
            }
        }
        match self.decode(&states) {
            Ok(reply) => reply,
            Err(why) => {
                self.problems.push(why);
                UsbReply::Nothing
            }
        }
    }

    /// A packet from the line states the far end drove, one per cycle,
    /// sampled in the middle of each bit.
    fn decode(&self, states: &[UsbLine]) -> Result<UsbReply, String> {
        let cpb = usize::try_from(self.pair.cycles_per_bit()).expect("a small number");
        if !states.len().is_multiple_of(cpb) {
            return Err(format!(
                "the device drove {} cycles, not whole bits",
                states.len()
            ));
        }
        let symbols: Vec<UsbLine> = states.iter().skip(cpb / 2).step_by(cpb).copied().collect();
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
        for _ in 0..120 * self.pair.cycles_per_bit() {
            self.host_cycle(UsbLine::Se0);
            seen |= self.pair.usb_reset();
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
                    let short = payload.len() < EP0_MAXPKT;
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

    /// One bulk OUT transaction: the token, the data packet with the PID
    /// given, and the handshake the device answers with.
    fn bulk_out(&mut self, addr: u8, endp: u8, pid: u8, payload: &[u8]) -> UsbReply {
        self.send(&usb_token(USB_OUT, addr, endp));
        self.idle(3);
        self.send(&usb_data(pid, payload));
        let reply = self.receive();
        self.idle(4);
        reply
    }

    /// One bulk IN transaction, **not** acknowledged: a caller that means to
    /// keep the byte calls `ack`, and a caller proving the device sends it
    /// again does not.
    fn bulk_in(&mut self, addr: u8, endp: u8) -> UsbReply {
        self.send(&usb_token(USB_IN, addr, endp));
        self.receive()
    }

    /// The data endpoint's byte interface, and the logic above it.
    fn data(&mut self) -> &mut DataEp {
        self.pair.data()
    }

    /// One of the block's own top-level nets, by name.
    fn port(&self, name: &str) -> u64 {
        self.pair.port(name)
    }

    /// Drive one of the block's own top-level inputs, by name.
    fn set_port(&mut self, name: &str, value: u64, bits: u32) {
        self.pair.set_port(name, value, bits);
    }

    fn clear_memory(&mut self, path: &str) -> bool {
        self.pair.clear_memory(path)
    }

    /// A control transfer whose data stage goes **host to device**: SETUP,
    /// one DATA1 packet of `payload`, then the zero-length IN of the status
    /// stage.
    ///
    /// The toggle is DATA1 because a control transfer's data stage starts
    /// at DATA1 whichever way it points (USB 2.0 §8.5.3), and `payload` is
    /// one packet because that is all `usb_ctrl_ep`'s class hook takes.
    /// The reply is the status stage's, so a request the device stalled
    /// somewhere is visible as a STALL here.
    fn control_write_data(&mut self, addr: u8, request: [u8; 8], payload: &[u8]) -> UsbReply {
        let reply = self.setup(addr, request);
        if reply != UsbReply::Handshake(USB_ACK) {
            return reply;
        }
        self.idle(4);
        self.send(&usb_token(USB_OUT, addr, 0));
        self.idle(3);
        self.send(&usb_data(USB_DATA1, payload));
        let data_stage = self.receive();
        if data_stage != UsbReply::Handshake(USB_ACK) {
            return data_stage;
        }
        self.idle(4);
        let status = self.in_token(addr);
        if status == UsbReply::Data(USB_DATA1, Vec::new()) {
            self.ack();
        }
        status
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
        let mut problems = self.problems.clone();
        problems.extend(self.pair.problems().iter().cloned());
        assert!(
            problems.is_empty(),
            "the host saw the protocol broken:\n  {}",
            problems.join("\n  ")
        );
        // Every answer came inside the window the layer between the host
        // and the device allows, once what that layer added to it is taken
        // off; `added_delay` says what that is and why there is any.
        let (low, high) = self.pair.answer_window();
        let added = self.pair.added_delay();
        assert!(
            added.is_empty() || added.len() >= self.gaps.len(),
            "{} answers and {} of them accounted for: the correction has drifted \
             out of step with the answers",
            self.gaps.len(),
            added.len()
        );
        for (i, gap) in self.gaps.iter().enumerate() {
            let extra = added.get(i).copied().unwrap_or(0);
            let took = gap.saturating_sub(extra);
            assert!(
                (low..=high).contains(&took),
                "the device answered after {took} cycles, outside {low} to {high} \
                 (the layer between added {extra} of the {gap} measured); every gap of \
                 this run was {:?} and every correction {added:?}",
                self.gaps
            );
        }
    }
}

/// `usb_device_fs` on the other end of the pair: the device's own pins,
/// four cycles of its 48 MHz clock to a bit.
struct FsPair<'d> {
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
    problems: Vec<String>,
    data: DataEp,
}

impl<'d> FsPair<'d> {
    fn new(design: &'d Design) -> FsPair<'d> {
        FsPair::with_loopback(design, true)
    }

    fn with_loopback(design: &'d Design, loopback: bool) -> FsPair<'d> {
        let sim = simulate(design, "usb_device_fs");
        let pin = |n: &str| top_net(&sim, n);
        let mut pair = FsPair {
            clk: pin("clk48"),
            dp_i: pin("usb_dp_i"),
            dn_i: pin("usb_dn_i"),
            dp_o: pin("usb_dp_o"),
            dn_o: pin("usb_dn_o"),
            oe: pin("usb_oe"),
            address: pin("address"),
            configured: pin("configured"),
            usb_reset: pin("usb_reset"),
            problems: Vec::new(),
            data: DataEp::new(&sim, loopback),
            sim,
        };
        let rst_n = top_net(&pair.sim, "rst_n");
        pair.data.quiet(&mut pair.sim);
        pair.set_line(UsbLine::J);
        let clk = pair.clk;
        reset(&mut pair.sim, clk, rst_n);
        assert!(
            high(&pair.sim, top_net(&pair.sim, "usb_dp_pu")),
            "the device asks for its D+ pull-up"
        );
        pair
    }

    fn set_line(&mut self, line: UsbLine) {
        let (dp, dn) = line.pins();
        self.sim.set(self.dp_i, bit(dp));
        self.sim.set(self.dn_i, bit(dn));
    }
}

impl UsbPair for FsPair<'_> {
    fn cycles_per_bit(&self) -> u64 {
        4
    }

    fn cycle(&mut self, host: Option<UsbLine>) {
        step_data(&mut self.sim, &mut self.data);
        match host {
            Some(line) => self.set_line(line),
            None => {
                // The bus is the device's while it drives it, so the
                // device's own input sees what it puts there.
                if high(&self.sim, self.oe) {
                    let dp = high(&self.sim, self.dp_o);
                    let dn = high(&self.sim, self.dn_o);
                    self.sim.set(self.dp_i, bit(dp));
                    self.sim.set(self.dn_i, bit(dn));
                } else {
                    self.set_line(UsbLine::J);
                }
            }
        }
        let clk = self.clk;
        cycle(&mut self.sim, clk, HALF);
    }

    fn driven(&mut self) -> Option<UsbLine> {
        if !high(&self.sim, self.oe) {
            return None;
        }
        match (high(&self.sim, self.dp_o), high(&self.sim, self.dn_o)) {
            (true, false) => Some(UsbLine::J),
            (false, true) => Some(UsbLine::K),
            (false, false) => Some(UsbLine::Se0),
            (true, true) => {
                self.problems.push("the device drove SE1".into());
                Some(UsbLine::Se0)
            }
        }
    }

    fn address(&self) -> u64 {
        get_u64(&self.sim, self.address)
    }

    fn configured(&self) -> bool {
        high(&self.sim, self.configured)
    }

    fn usb_reset(&self) -> bool {
        high(&self.sim, self.usb_reset)
    }

    fn problems(&self) -> &[String] {
        &self.problems
    }

    fn answer_window(&self) -> (u64, u64) {
        // Two to six and a half bit times after the host's EOP.
        (8, 26)
    }

    fn data(&mut self) -> &mut DataEp {
        &mut self.data
    }

    fn port(&self, name: &str) -> u64 {
        get_u64(&self.sim, top_net(&self.sim, name))
    }

    fn set_port(&mut self, name: &str, value: u64, bits: u32) {
        let net = top_net(&self.sim, name);
        self.sim.set(net, word(bits, value));
    }
}

/// `bMaxPacketSize0` and `wMaxPacketSize`, which every block in this
/// library now declares as 64.
///
/// Both are written here rather than read out of the blocks, because what
/// these tests are for is holding the blocks to the specification's own
/// numbers: USB 2.0 §5.5.3 allows endpoint 0 exactly 8, 16, 32 or 64 and
/// §5.8.3 the same four for a bulk endpoint, so 64 is the largest legal value
/// of each and the one that costs a host the fewest transactions.
///
/// `EP0_MAXPKT` is also **what ends a control read**: a data stage stops on a
/// packet shorter than the endpoint's maximum or on `wLength`, so a host model
/// with the wrong number here would either stop early or ask for ever. That
/// is the same mistake `tests/usb_loopback.rs` records making against a real
/// host, and this is the simulated half of it.
const EP0_MAXPKT: usize = 64;
const BULK_MAXPKT: usize = 64;

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
    expected_device_descriptor_of(vid, pid, [0xFF, 0x00, 0x00])
}

/// The same, for a device whose class triple is not the vendor-specific
/// default: `ip/usb_cdc_acm` says `02h` in the device descriptor as well as
/// in its communications interface, which is what CDC 1.1 Table 14 asks
/// for.
fn expected_device_descriptor_of(vid: u16, pid: u16, class: [u8; 3]) -> Vec<u8> {
    let mut d = vec![
        18,
        1,
        0x00,
        0x02,
        class[0],
        class[1],
        class[2],
        u8::try_from(EP0_MAXPKT).expect("a legal bMaxPacketSize0"),
    ];
    d.extend_from_slice(&vid.to_le_bytes());
    d.extend_from_slice(&pid.to_le_bytes());
    d.extend_from_slice(&[0x00, 0x01, 0, 0, 0, 1]);
    d
}

/// The configuration descriptor the block should send, written **forwards**
/// from the specification's layout — the way `lsusb -v` prints one — rather
/// than from the block's `IFACE_DESC` parameter.
///
/// This is the one assertion that would catch the parameter's byte order
/// coming out reversed, which is the failure mode a concatenation invites:
/// Verilog puts a concatenation's first element in its most significant
/// bits and every index in `usb_ctrl_ep` counts from byte 0 up, so there is
/// a reversal at elaboration and either it happens or this test says so.
/// The three derived fields are written here as **the arithmetic and not the
/// answer**: `wTotalLength` is a sum, `bNumInterfaces` and `bNumEndpoints`
/// are counts of what follows, so a descriptor changed in one place changes
/// this expectation with it.
fn expected_configuration_descriptor() -> Vec<u8> {
    // The interface and its two endpoints, which is what `IFACE_DESC`
    // holds: vendor specific, and a bulk OUT and a bulk IN on endpoint 1
    // with 64-byte packets.
    let mut iface: Vec<u8> = Vec::new();
    let pkt = u8::try_from(BULK_MAXPKT).expect("a legal wMaxPacketSize");
    iface.extend_from_slice(&[9, 4, 0, 0, 0, 0xFF, 0x00, 0x00, 0]);
    iface.extend_from_slice(&[7, 5, 0x01, 2, pkt, 0, 0]);
    iface.extend_from_slice(&[7, 5, 0x81, 2, pkt, 0, 0]);
    // bNumEndpoints is byte 4 of the interface descriptor and is the
    // ENDPOINT descriptors that follow it, counted here rather than typed.
    iface[4] = u8::try_from(count_descriptors(&iface, 5)).expect("a small number");
    let total = u16::try_from(9 + iface.len()).expect("a short descriptor");
    let [lo, hi] = total.to_le_bytes();
    let mut d = vec![
        9,
        2,
        lo,
        hi,
        u8::try_from(count_descriptors(&iface, 4)).expect("a small number"),
        1,
        0,
        0x80,
        50,
    ];
    d.extend(iface);
    d
}

/// Descriptors of one `bDescriptorType` in a run of descriptors, walked
/// along the chain of `bLength` fields the way a host does.
fn count_descriptors(blob: &[u8], kind: u8) -> usize {
    let mut at = 0;
    let mut n = 0;
    while at + 1 < blob.len() && blob[at] != 0 {
        if blob[at + 1] == kind {
            n += 1;
        }
        at += usize::from(blob[at]);
    }
    n
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
    let mut host = UsbHost::new(FsPair::new(&design), drift);
    enumerate(&mut host);
}

/// A whole enumeration, and the only statement of one: both cores are
/// held to it, since what a host does to a device does not depend on how
/// the device's bytes reach the pair.
fn enumerate<P: UsbPair>(host: &mut UsbHost<P>) {
    enumerate_descriptors(
        host,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
    );
}

/// The same enumeration against a stated pair of descriptors, so that a
/// class layer's own descriptor set goes through **the whole of it** —
/// every short read, the read that ends on wLength, both directions of
/// SET_CONFIGURATION and a second enumeration after a bus reset — rather
/// than through a test of its own that would check less.
fn enumerate_descriptors<P: UsbPair>(
    host: &mut UsbHost<P>,
    device_want: &[u8],
    config_want: &[u8],
) {
    host.bus_reset();
    assert_eq!(host.address(), 0);

    // What a host does first: the device descriptor at address 0, with a
    // wLength of 64 whatever the descriptor's size.
    let device = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("the device descriptor");
    assert_eq!(device, device_want);

    // SET_ADDRESS takes effect after its status stage, not before.
    let status = host.control_write(0, set_address(9));
    assert_eq!(status, UsbReply::Data(USB_DATA1, Vec::new()));
    assert_eq!(host.address(), 9, "the new address");

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
    assert_eq!(first8, device_want[..8]);
    let config9 = host
        .control_read(9, get_descriptor(2, 9))
        .expect("the configuration header");
    assert_eq!(config9, config_want[..9]);
    let config = host
        .control_read(9, [0x80, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF])
        .expect("the whole configuration");
    assert_eq!(
        config, config_want,
        "the configuration descriptor and everything under it, in packets of eight"
    );
    // A read of exactly sixteen bytes of an eighteen-byte descriptor: the
    // data stage stops because `wLength` is reached and **not** because a
    // packet was short, since sixteen is less than `EP0_MAXPKT`. That is the
    // other of the two ways a control read ends, and with 64-byte packets it
    // is the only one this particular read exercises — it was two whole
    // packets when a packet was eight bytes.
    let sixteen = host
        .control_read(9, get_descriptor(1, 16))
        .expect("sixteen bytes of the device descriptor");
    assert_eq!(sixteen.len(), 16, "wLength ends the data stage");

    // SET_CONFIGURATION 1, then 0.
    assert!(!host.configured());
    let status = host.control_write(9, [0x00, 0x09, 0x01, 0, 0, 0, 0, 0]);
    assert_eq!(status, UsbReply::Data(USB_DATA1, Vec::new()));
    assert!(host.configured(), "configured");
    host.control_write(9, [0x00, 0x09, 0x00, 0, 0, 0, 0, 0]);
    assert!(!host.configured(), "and back");

    // A bus reset forgets the address.
    host.bus_reset();
    assert_eq!(host.address(), 0);
    let again = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("enumerable again");
    assert_eq!(again, device_want);

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
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    ignore_what_it_cannot_do(&mut host);
}

/// The packets a device must not answer and the requests it must stall,
/// which both cores are held to. A bad CRC or PID is the device's to
/// notice in either arrangement; broken bit stuffing is the transceiver's
/// on a ULPI bus and the receiver's own on a full-speed one, and either
/// way the device must stay quiet.
fn ignore_what_it_cannot_do<P: UsbPair>(host: &mut UsbHost<P>) {
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

    // A token for another endpoint is not for the control endpoint. That
    // endpoint exists now — it is the bulk pair — and it has no control
    // pipe, so a SETUP to it goes unanswered by both of them: the control
    // endpoint because the token is not its, the bulk endpoint because a
    // bulk endpoint has nothing to say to a SETUP.
    host.send(&usb_token(USB_SETUP, 0, 1));
    host.idle(3);
    host.send(&usb_data(USB_DATA0, &GET_DEVICE_DESCRIPTOR));
    assert_eq!(
        host.receive(),
        UsbReply::Nothing,
        "a SETUP to the data endpoint is nobody's"
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
    let mut host = UsbHost::new(FsPair::new(&design), 0);
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
    // The whole eighteen-byte descriptor, because a packet is 64 bytes: this
    // was the first eight of it when it was not.
    assert_eq!(payload[..], expected_device_descriptor(0x1209, 0x0001)[..]);
    host.ack();
    // Acknowledged, and there is nothing left of the data stage, so an IN the
    // host sends anyway is answered with a zero-length DATA0 — the packet that
    // ends a transfer whose length is a multiple of the packet size, sent here
    // from the same arm.
    assert_eq!(
        host.in_token(0),
        UsbReply::Data(USB_DATA0, Vec::new()),
        "a data stage that is over answers an IN with nothing"
    );
    host.assert_clean();
}

/// A data stage cut into packets of the size the descriptor declared, with the
/// toggle alternating, proved at a packet size that is **not** the default.
///
/// `MAXPKT0` is eight here and 64 everywhere else, which is what makes this
/// test worth having twice over: it is the only place a data stage longer than
/// one packet of endpoint 0 goes through the plain device — no descriptor of it
/// reaches 64 bytes — and it is the only place the parameter is exercised at a
/// second one of the four values USB 2.0 §5.5.3 allows. A packet size that had
/// been hard-wired at 64 somewhere, or a `bMaxPacketSize0` that did not follow
/// the parameter, fails here and nowhere else.
#[test]
fn usb_device_fs_cuts_a_data_stage_into_packets_of_the_size_it_declared() {
    let design = design_of(
        "usb_device_fs",
        "usb_device_fs",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("MAXPKT0", "7'd8"),
        ],
    );
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    host.bus_reset();
    assert_eq!(
        host.setup(0, GET_DEVICE_DESCRIPTOR),
        UsbReply::Handshake(USB_ACK)
    );
    host.idle(4);

    // Byte 7 of the device descriptor is `bMaxPacketSize0` and says eight,
    // because the parameter does.
    let mut want = expected_device_descriptor(0x1209, 0x0001);
    want[7] = 8;

    // Eighteen bytes in packets of eight: 8, 8, 2, DATA1 then DATA0 then
    // DATA1, and the short last one is what ends the transfer.
    let mut got: Vec<u8> = Vec::new();
    for (chunk, pid) in [(8usize, USB_DATA1), (8, USB_DATA0), (2, USB_DATA1)] {
        let reply = host.in_token(0);
        let UsbReply::Data(saw, payload) = &reply else {
            panic!("data expected, got {reply:?}");
        };
        assert_eq!(*saw, pid, "the data stage alternates DATA1, DATA0, ...");
        assert_eq!(payload.len(), chunk, "a packet of eight until the last");
        got.extend(payload);
        host.ack();
    }
    assert_eq!(got, want, "eighteen bytes, three packets, byte for byte");
    host.assert_clean();
}

// ---------------------------------------------------------------------------
// The data endpoint: bytes, once the device is enumerated
// ---------------------------------------------------------------------------

/// The other data toggle.
fn other_toggle(pid: u8) -> u8 {
    if pid == USB_DATA0 {
        USB_DATA1
    } else {
        USB_DATA0
    }
}

/// A bulk pipe as a host controller keeps one: an endpoint number and the
/// two data toggles, one per direction.
///
/// The NAK retries are **counted, never timed**. A host controller repeats a
/// NAKed transaction until it succeeds or the transfer times out, and what a
/// test can assert about that is how many repeats it took, which is the same
/// number on a fast machine and a slow one.
struct BulkPipe {
    endp: u8,
    out_pid: u8,
    in_pid: u8,
    naks: usize,
}

impl BulkPipe {
    fn new(endp: u8) -> BulkPipe {
        BulkPipe {
            endp,
            out_pid: USB_DATA0,
            in_pid: USB_DATA0,
            naks: 0,
        }
    }

    /// One packet to the device, repeated while it NAKs.
    fn write<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8, payload: &[u8]) {
        for _ in 0..64 {
            match host.bulk_out(addr, self.endp, self.out_pid, payload) {
                UsbReply::Handshake(USB_ACK) => {
                    self.out_pid = other_toggle(self.out_pid);
                    return;
                }
                UsbReply::Handshake(USB_NAK) => self.naks += 1,
                other => panic!("an OUT of {} bytes was answered {other:?}", payload.len()),
            }
        }
        panic!("the device NAKed all sixty-four attempts at an OUT");
    }

    /// One packet from the device, or nothing if it has nothing to say.
    ///
    /// [`BulkPipe::read`] is for an endpoint that owes an answer and
    /// whose answer is late; this is for one that may have nothing, which
    /// is what a serial port waiting for a keystroke is.
    fn read_or_nothing<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8) -> Vec<u8> {
        match host.bulk_in(addr, self.endp) {
            UsbReply::Data(pid, payload) => {
                assert_eq!(pid, self.in_pid, "the IN endpoint's data toggle");
                host.ack();
                self.in_pid = other_toggle(self.in_pid);
                payload
            }
            UsbReply::Handshake(USB_NAK) => {
                self.naks += 1;
                Vec::new()
            }
            other => panic!("an IN was answered {other:?}"),
        }
    }

    /// One packet from the device, repeated while it NAKs, acknowledged.
    fn read<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8) -> Vec<u8> {
        for _ in 0..64 {
            match host.bulk_in(addr, self.endp) {
                UsbReply::Data(pid, payload) => {
                    assert_eq!(pid, self.in_pid, "the IN endpoint's data toggle");
                    host.ack();
                    self.in_pid = other_toggle(self.in_pid);
                    return payload;
                }
                UsbReply::Handshake(USB_NAK) => {
                    self.naks += 1;
                    host.idle(4);
                }
                other => panic!("an IN was answered {other:?}"),
            }
        }
        panic!("the device NAKed all sixty-four attempts at an IN");
    }
}

/// Enough of an enumeration to reach a data endpoint: an address, the
/// configuration descriptor checked, and SET_CONFIGURATION.
fn configure<P: UsbPair>(host: &mut UsbHost<P>, addr: u8) {
    configure_for(
        host,
        addr,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
    );
}

/// The same, against a stated pair of descriptors.
fn configure_for<P: UsbPair>(
    host: &mut UsbHost<P>,
    addr: u8,
    device_want: &[u8],
    config_want: &[u8],
) {
    host.bus_reset();
    let device = host
        .control_read(0, GET_DEVICE_DESCRIPTOR)
        .expect("the device descriptor");
    assert_eq!(device, device_want);
    assert_eq!(
        host.control_write(0, set_address(addr)),
        UsbReply::Data(USB_DATA1, Vec::new())
    );
    let config = host
        .control_read(addr, [0x80, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF])
        .expect("the configuration");
    assert_eq!(
        config, config_want,
        "the endpoints are in the descriptor the host reads"
    );
    host.control_write(addr, [0x00, 0x09, 0x01, 0, 0, 0, 0, 0]);
    assert!(host.configured(), "configured");
    host.idle(10);
}

/// Bytes out to endpoint 1 and the same bytes back from it, and the **only
/// statement of it**: both cores are put through this, as both are put
/// through `enumerate`, because what a host does to a data endpoint does not
/// depend on how the device's bytes reach the pair.
///
/// The pair is in loopback — `out_*` wired into `in_*` — which is the design
/// that goes on the part, so what this proves is the same wiring the board
/// has and not a testbench's private arrangement.
fn bulk_loopback<P: UsbPair>(host: &mut UsbHost<P>) {
    configure(host, 7);

    // A full packet, one a byte short of full, a middling one, and one byte.
    // `FF` and `07` between them put six ones in a row on the wire in both
    // directions, so the device has to stuff what it sends and unstuff what it
    // receives inside a data endpoint's payload and not only inside a
    // descriptor's.
    //
    // **The full one is 64 bytes and that is the point of it.** Every byte of
    // it is different from its neighbours and from its own position modulo
    // eight, so a packet that came back with the first eight bytes repeated,
    // or shifted by the two bytes of a CRC, or drawn from a buffer whose base
    // was computed for a different length, is visible here and not only as a
    // length. Sixty-three bytes beside it is the case a base counter off by
    // one gets wrong, since that is the only length at which the packet fills
    // the buffer but does not start at its bottom.
    let full: Vec<u8> = (0..BULK_MAXPKT)
        .map(|i| {
            u8::try_from(i)
                .expect("a byte")
                .wrapping_mul(73)
                .wrapping_add(5)
        })
        .collect();
    let packets: Vec<Vec<u8>> = vec![
        full.clone(),
        full[..BULK_MAXPKT - 1].to_vec(),
        vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07],
        vec![0xDE, 0xAD, 0xBE, 0xEF, 0xFF],
        vec![0x5A],
    ];

    let mut pipe = BulkPipe::new(1);
    for payload in &packets {
        pipe.write(host, 7, payload);
        let back = pipe.read(host, 7);
        assert_eq!(&back, payload, "what went out came back");
    }

    // And it went through the byte interface on its way, packet by packet,
    // with `out_last` where the host put the end of each one.
    assert_eq!(
        host.data().got,
        packets,
        "the bytes reached the interface above the endpoint"
    );

    // Nothing is left: the next IN is NAKed rather than answered with a
    // packet the device has already sent.
    assert_eq!(
        host.bulk_in(7, 1),
        UsbReply::Handshake(USB_NAK),
        "an empty IN endpoint NAKs"
    );
    host.idle(10);
    host.assert_clean();
}

#[test]
fn usb_device_fs_loops_bytes_through_endpoint_one() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    bulk_loopback(&mut host);
}

#[test]
fn usb_device_fs_loops_bytes_through_endpoint_one_off_clock() {
    // The same bytes with the host's clock 0.4 % slow and 0.4 % fast, which
    // is what found the receiver sampling a bit in its last cycle instead of
    // its middle. A data endpoint's payload is arbitrary bytes rather than a
    // descriptor's, so it is where a sampling fault shows first.
    let design = usb_design();
    bulk_loopback(&mut UsbHost::new(FsPair::new(&design), 64));
    bulk_loopback(&mut UsbHost::new(FsPair::new(&design), -64));
}

/// The same bytes with the two packet buffers as **shift registers**, which is
/// `usb_bulk_ep`'s `BUF_RAM = 0`.
///
/// The shape that ships is the array, and every other behavioural test above
/// runs on it. This one exists because the two shapes are not two independent
/// pieces of logic that happen to agree: they share `owp`, `ilen`, `ordx` and
/// `tx_index` and differ only in the storage and in the two read expressions,
/// and the shift register's read is `index - written` at the index's own width
/// where the array's is `index`. That subtraction is arithmetic, it is the
/// place a base counter used to be, and an off-by-one in it is exactly the
/// defect `bulk_loopback`'s 63-byte packet was put there for: 63 bytes is the
/// only length that fills the buffer and does not start at the bottom of it.
///
/// **What it would and would not catch.** It catches a shift-register buffer
/// that reads from the wrong position, at five packet lengths in both
/// directions, with the bytes checked at the byte interface and at the host. It
/// does not re-run the endpoint's other rules — the toggle, the NAKs, the
/// zero-length packet — because none of those touches a buffer's addressing,
/// and it says nothing about either shape as a *device* builds it; the
/// footprint table carries both shapes for that, and a board for the array.
#[test]
fn usb_device_fs_loops_bytes_with_the_buffers_as_shift_registers() {
    let design = design_of(
        "usb_device_fs",
        "usb_device_fs",
        &[("VID", "16'h1209"), ("PID", "16'h0001"), ("BUF_RAM", "0")],
    );
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    bulk_loopback(&mut host);
}

#[test]
fn usb_device_ulpi_loops_bytes_through_endpoint_one() {
    let design = ulpi_design();
    let mut host = UsbHost::new(UlpiPair::new(&design), 0);
    bulk_loopback(&mut host);
}

/// The same bytes through the transceiver that is **on the board**: the one
/// that reports LineState a clock late, against ULPI §3.8.1.3.
///
/// `usb_device_ulpi_enumerates_through_a_transceiver_that_reports_linestate_late`
/// says what that part does and what believing it cost. A data endpoint
/// answers from a second turnaround counter, in a second module, so it is
/// worth putting through the same transceiver rather than assuming that
/// endpoint 0 having survived it covers both.
#[test]
fn usb_device_ulpi_loops_bytes_through_the_transceiver_that_is_on_the_board() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    bulk_loopback(&mut host);
}

/// The byte interface driven as a user of the block would drive it, rather
/// than looped back: every byte the host sends is taken, and the packets the
/// host reads are handed over from a queue.
///
/// This is where the **zero-length packet** is, in both directions, because a
/// loopback cannot carry one: an OUT of no bytes hands nothing to the
/// interface, so there is nothing for the interface to give back, and
/// `in_commit` on its own is the only way to send one. A host uses a
/// zero-length IN packet to end a transfer whose length is a multiple of the
/// packet size, so an endpoint that cannot send one is an endpoint a class
/// layer would have to work around.
#[test]
fn usb_bytes_reach_the_byte_interface_and_come_back_from_it() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 3);

    let sent: Vec<Vec<u8>> = vec![
        vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        vec![0x99],
        Vec::new(),
    ];
    let mut pipe = BulkPipe::new(1);
    for payload in &sent {
        pipe.write(&mut host, 3, payload);
    }
    // A zero-length OUT packet delivers no bytes, so the interface saw the
    // two that had any.
    assert_eq!(
        host.data().got,
        sent[..2].to_vec(),
        "the packets with bytes"
    );

    // Three packets the other way, one of them empty.
    let give: Vec<Vec<u8>> = vec![
        vec![0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7],
        Vec::new(),
        vec![0xB0, 0xB1],
    ];
    host.data().give = give.clone();
    for payload in &give {
        assert_eq!(
            &pipe.read(&mut host, 3),
            payload,
            "the packet the interface gave"
        );
    }
    host.idle(10);
    host.assert_clean();
}

/// The OUT endpoint NAKs while the last packet has not been taken, and
/// nothing is lost when it is.
///
/// This is the flow control a bulk endpoint has instead of a FIFO, and it is
/// the reason the block can be one packet deep. The count is the assertion:
/// exactly one NAK for the one attempt made while the buffer was full.
#[test]
fn usb_bulk_endpoint_naks_an_out_until_the_bytes_are_taken() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 5);

    // Nothing above the endpoint is taking bytes.
    host.data().take = false;
    let mut pipe = BulkPipe::new(1);
    let first = [0x01, 0x02, 0x03, 0x04];
    pipe.write(&mut host, 5, &first);
    assert_eq!(pipe.naks, 0, "the first packet fits");
    assert!(host.data().got.is_empty(), "and nothing has taken it");

    // The second is NAKed: one attempt, one NAK.
    let second = [0x05, 0x06];
    assert_eq!(
        host.bulk_out(5, 1, pipe.out_pid, &second),
        UsbReply::Handshake(USB_NAK),
        "a full OUT buffer NAKs"
    );

    // Taken, and then the same packet goes in.
    host.data().take = true;
    host.idle(20);
    pipe.write(&mut host, 5, &second);
    host.idle(20);
    assert_eq!(
        host.data().got,
        vec![first.to_vec(), second.to_vec()],
        "both packets, in order, once each"
    );
    host.assert_clean();
}

/// A data packet longer than 64 bytes is refused whole, and the 64-byte one
/// beside it is taken.
///
/// This is the boundary of `usb_pkt_rx`'s `too_long`, which withdraws `dat_ok`
/// from an over-long packet instead of cutting it to fit, and it is arithmetic
/// this round rewrote: the count of bytes in a packet used to be four bits and
/// saturate at fifteen, and it is seven bits saturating at `64 + 3` now. Off by
/// one in either direction and this test fails — one way a legal 64-byte packet
/// is refused, the other way a 65-byte one is taken and the endpoint stores a
/// length its buffer does not have.
///
/// A refused packet is answered with **nothing at all**, which is right: USB
/// 2.0 §8.7.1 has a device return no handshake for a packet whose CRC failed,
/// and a device that cannot tell a corrupt packet from an over-long one should
/// treat both the same. The host then retries, which is what the second half
/// here is.
///
/// **What it would not catch**: anything about a host that obeys
/// `wMaxPacketSize`, since no host sends this. It is a test of the device's
/// arithmetic, not of an interaction.
#[test]
fn usb_bulk_endpoint_refuses_a_packet_longer_than_it_promised() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 9);

    // Sixty-five bytes, one more than the descriptor promised.
    let over: Vec<u8> = (0..=BULK_MAXPKT)
        .map(|i| u8::try_from(i).expect("a byte").wrapping_mul(11))
        .collect();
    assert_eq!(over.len(), BULK_MAXPKT + 1);
    assert_eq!(
        host.bulk_out(9, 1, USB_DATA0, &over),
        UsbReply::Nothing,
        "a packet longer than wMaxPacketSize is not answered"
    );
    host.idle(20);
    assert!(
        host.data().got.is_empty(),
        "and nothing of it reached the interface: {:?}",
        host.data().got
    );

    // The toggle never moved, so the host's retry is still DATA0 — and 64
    // bytes, which is what it should have sent, is taken.
    let legal: Vec<u8> = (0..BULK_MAXPKT)
        .map(|i| u8::try_from(i).expect("a byte").wrapping_mul(11))
        .collect();
    assert_eq!(
        host.bulk_out(9, 1, USB_DATA0, &legal),
        UsbReply::Handshake(USB_ACK),
        "a full packet is taken"
    );
    host.idle(20);
    assert_eq!(
        host.data().got,
        vec![legal],
        "all 64 bytes of it, and only those"
    );
    host.assert_clean();
}

/// A packet the host sends twice with the same toggle is acknowledged twice
/// and delivered once.
///
/// That is what a data toggle is *for*: the host repeats a packet when it
/// does not hear the ACK, and a device that cannot tell a repeat from a new
/// packet duplicates data. The repeat is sent while the first packet is
/// still in the buffer, which is the case a device that checked its buffer
/// before its toggle would answer with a NAK — and then the host would
/// repeat for ever.
#[test]
fn usb_bulk_endpoint_delivers_a_repeated_packet_once() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 5);

    host.data().take = false;
    let payload = [0xAA, 0xBB, 0xCC];
    assert_eq!(
        host.bulk_out(5, 1, USB_DATA0, &payload),
        UsbReply::Handshake(USB_ACK),
        "the packet"
    );
    assert_eq!(
        host.bulk_out(5, 1, USB_DATA0, &payload),
        UsbReply::Handshake(USB_ACK),
        "the same packet again, acknowledged again"
    );
    host.data().take = true;
    host.idle(40);
    assert_eq!(host.data().got, vec![payload.to_vec()], "delivered once");
    host.assert_clean();
}

/// An IN packet the host does not acknowledge is sent again with the same
/// toggle, and released only when the ACK arrives.
#[test]
fn usb_bulk_endpoint_sends_again_what_the_host_did_not_acknowledge() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 5);

    host.data().give = vec![vec![0x31, 0x41, 0x59]];
    host.idle(20);
    let first = host.bulk_in(5, 1);
    assert_eq!(first, UsbReply::Data(USB_DATA0, vec![0x31, 0x41, 0x59]));
    host.idle(20);
    assert_eq!(
        host.bulk_in(5, 1),
        first,
        "the same packet with the same toggle"
    );
    host.ack();
    host.idle(20);
    assert_eq!(
        host.bulk_in(5, 1),
        UsbReply::Handshake(USB_NAK),
        "and once acknowledged it is gone"
    );
    host.idle(10);
    host.assert_clean();
}

/// CLEAR_FEATURE(ENDPOINT_HALT) puts one direction's data toggle back to
/// DATA0, which is how a host and a device agree on a toggle again without a
/// bus reset.
///
/// Without it, a host program that starts with the device's toggles anywhere
/// but where its own are sends a packet the device calls a repeat and
/// discards — and the host sees an ACK and believes the bytes arrived. The
/// host-side loopback calls `clear_halt` on both endpoints for exactly this
/// reason, so this is the device half of that.
#[test]
fn usb_clear_feature_puts_a_bulk_endpoints_toggle_back() {
    let design = usb_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    configure(&mut host, 5);

    // One packet moves the OUT toggle on to DATA1.
    let mut pipe = BulkPipe::new(1);
    pipe.write(&mut host, 5, &[0x01]);
    host.idle(20);
    assert_eq!(pipe.out_pid, USB_DATA1, "the host's toggle moved too");

    // CLEAR_FEATURE(ENDPOINT_HALT) on endpoint 1 OUT: bmRequestType 02h,
    // bRequest 01h, wValue 0000h, wIndex 0001h.
    assert_eq!(
        host.control_write(5, [0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "the request is accepted, not stalled"
    );
    host.idle(20);

    // So a DATA0 packet is new again rather than a repeat of the last one.
    assert_eq!(
        host.bulk_out(5, 1, USB_DATA0, &[0x02]),
        UsbReply::Handshake(USB_ACK)
    );
    host.idle(40);
    assert_eq!(
        host.data().got,
        vec![vec![0x01], vec![0x02]],
        "both packets arrived, so the toggle really was reset"
    );
    host.assert_clean();
}

/// `bNumEndpoints` is counted from the descriptors, so a class that states it
/// wrongly is corrected rather than believed.
///
/// The parameter here is the default interface with `bNumEndpoints` set to
/// **5** and two endpoint descriptors after it. A block that copied the byte
/// out of the parameter would report 5, and a host would look for three
/// endpoints that are not there.
#[test]
fn usb_configuration_descriptor_counts_the_endpoints_the_class_miscounted() {
    let design = design_of(
        "usb_device_fs",
        "usb_device_fs",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("IFACE_BYTES", "23"),
            // The default descriptors with `bNumEndpoints` set to 5, and
            // `wMaxPacketSize` 40h on both endpoints, which is what `MAXPKT`
            // puts there by default: a parameter given by hand has to state
            // every byte, including the ones the default derives.
            (
                "IFACE_DESC",
                "184'h0904000005FF0000000705010240000007058102400000",
            ),
        ],
    );
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    host.bus_reset();
    let config = host
        .control_read(0, get_descriptor(2, 64))
        .expect("the configuration");
    assert_eq!(
        config,
        expected_configuration_descriptor(),
        "the block's own count, not the parameter's"
    );
    host.assert_clean();
}

/// Two interfaces with one endpoint each: `bNumInterfaces` is 2,
/// `wTotalLength` is 41, and each interface's `bNumEndpoints` is 1.
///
/// The parameter states **9** endpoints in both interface descriptors, so
/// every number in the nine bytes this block writes is being computed here
/// and none is being copied. Two interfaces also proves the walk along the
/// chain of `bLength` fields does not stop at the first one, which counting
/// a single interface cannot.
#[test]
fn usb_configuration_descriptor_counts_two_interfaces_separately() {
    let design = design_of(
        "usb_device_fs",
        "usb_device_fs",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("IFACE_BYTES", "32"),
            (
                "IFACE_DESC",
                "256'h0904000009FF000000070501020800000904010009FF00000007058102080000",
            ),
        ],
    );
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    host.bus_reset();
    let config = host
        .control_read(0, get_descriptor(2, 64))
        .expect("the configuration");
    let mut want: Vec<u8> = vec![9, 2, 41, 0, 2, 1, 0, 0x80, 50];
    want.extend_from_slice(&[9, 4, 0, 0, 1, 0xFF, 0x00, 0x00, 0]);
    want.extend_from_slice(&[7, 5, 0x01, 2, 8, 0, 0]);
    want.extend_from_slice(&[9, 4, 1, 0, 1, 0xFF, 0x00, 0x00, 0]);
    want.extend_from_slice(&[7, 5, 0x81, 2, 8, 0, 0]);
    assert_eq!(config, want, "two interfaces, one endpoint each");
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
// usb_device_ulpi: a ULPI transceiver model between the host and the device
// ---------------------------------------------------------------------------

/// Cycles of the 60 MHz ULPI clock in one full-speed bit time.
const ULPI_CPB: u64 = 5;

/// The registers of a ULPI transceiver this model has.
/// Interface clocks the pair takes to reach J after `TermSelect` connects
/// the 1.5 kOhm pull-up.
///
/// The model used to have an undriven pair sitting at **J**, which is
/// backwards: a full-speed bus idles at J *because a device pulls D+ up*,
/// and before that pull-up is connected and has charged the pair against a
/// host's two 15 kOhm pull-downs there is nothing on it but SE0. Measured on
/// a Great Scott Gadgets Cynthion r1.4 against a Microchip transceiver, five
/// microseconds after the write that sets `TermSelect` the Debug register
/// still reads `00h` and a few milliseconds later it reads `01h`. Ten
/// microseconds here is that shape at simulation's scale: the start-up's
/// first LineState read lands in the SE0 window, so a Link that believes one
/// read of it is caught.
const ULPI_PULLUP_SETTLE: u64 = 600;

const ULPI_FUNC_CTRL: u8 = 0x04;
const ULPI_OTG_CTRL: u8 = 0x0A;
const ULPI_DEBUG: u8 = 0x15;

/// LineState(1:0) of a receive command: bit 0 is D+, bit 1 is D-.
fn line_bits(line: UsbLine) -> u8 {
    match line {
        UsbLine::J => 0b01,
        UsbLine::K => 0b10,
        UsbLine::Se0 => 0b00,
    }
}

/// What one cycle of the pair told the transceiver's receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEvent {
    Nothing,
    Byte(u8),
    /// The packet ended on a byte boundary.
    Eop,
    /// It ended off one, or the bit stuffing was broken.
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RxPhase {
    Idle,
    Sync,
    Data,
    Eop,
}

/// The receive half of a full-speed transceiver: it recovers the bit
/// clock from the transitions, decodes NRZI, drops the stuffed zeros and
/// assembles bytes least significant bit first. Written from the USB 2.0
/// line rules, and it samples in the middle of each bit, since a
/// transceiver is not counting a 48 MHz clock the way `usb_fs_rx` is.
struct LineRx {
    div: u64,
    phase: u64,
    last: UsbLine,
    state: RxPhase,
    prev: UsbLine,
    zeros: u32,
    ones: u32,
    bits: u32,
    shift: u8,
}

impl LineRx {
    fn new(div: u64) -> LineRx {
        LineRx {
            div,
            phase: 0,
            last: UsbLine::J,
            state: RxPhase::Idle,
            prev: UsbLine::J,
            zeros: 0,
            ones: 0,
            bits: 0,
            shift: 0,
        }
    }

    /// Whether a packet is on the pair, which is UTMI's RxActive.
    fn active(&self) -> bool {
        matches!(self.state, RxPhase::Sync | RxPhase::Data)
    }

    fn step(&mut self, line: UsbLine) -> LineEvent {
        if line == self.last {
            // The counter wraps, so a run of bits with no transition in
            // it is still sampled once a bit.
            self.phase = (self.phase + 1) % self.div;
        } else {
            self.phase = 0;
        }
        self.last = line;
        if self.phase != self.div / 2 {
            return LineEvent::Nothing;
        }
        // NRZI: no change of state is a one.
        let one = line == self.prev;
        match self.state {
            RxPhase::Idle => {
                if line == UsbLine::K {
                    // The first K of the SYNC field, which is a zero.
                    self.state = RxPhase::Sync;
                    self.prev = UsbLine::K;
                    self.zeros = 1;
                }
                LineEvent::Nothing
            }
            RxPhase::Sync => {
                self.prev = line;
                if line == UsbLine::Se0 {
                    self.state = RxPhase::Eop;
                } else if !one {
                    self.zeros += 1;
                } else if self.zeros >= 3 {
                    self.state = RxPhase::Data;
                    self.ones = 1;
                    self.bits = 0;
                    self.shift = 0;
                } else {
                    self.state = RxPhase::Eop;
                }
                LineEvent::Nothing
            }
            RxPhase::Data => {
                self.prev = line;
                if line == UsbLine::Se0 {
                    self.state = RxPhase::Eop;
                    if self.bits == 0 {
                        LineEvent::Eop
                    } else {
                        LineEvent::Error
                    }
                } else if self.ones == 6 {
                    // The stuffed zero, which carries no data.
                    self.ones = 0;
                    if one {
                        self.state = RxPhase::Eop;
                        LineEvent::Error
                    } else {
                        LineEvent::Nothing
                    }
                } else {
                    self.shift = (u8::from(one) << 7) | (self.shift >> 1);
                    self.ones = if one { self.ones + 1 } else { 0 };
                    self.bits += 1;
                    if self.bits == 8 {
                        self.bits = 0;
                        LineEvent::Byte(self.shift)
                    } else {
                        LineEvent::Nothing
                    }
                }
            }
            RxPhase::Eop => {
                if line == UsbLine::J {
                    self.state = RxPhase::Idle;
                }
                LineEvent::Nothing
            }
        }
    }
}

/// One register access the transceiver was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UlpiAccess {
    write: bool,
    addr: u8,
    value: u8,
}

fn wrote(addr: u8, value: u8) -> UlpiAccess {
    UlpiAccess {
        write: true,
        addr,
        value,
    }
}

/// A register the transceiver was asked to read, and what it answered.
fn got(addr: u8, value: u8) -> UlpiAccess {
    UlpiAccess {
        write: false,
        addr,
        value,
    }
}

/// What the Link drove for one cycle.
struct LinkOut {
    oe: bool,
    data: u8,
    stp: bool,
    rst_n: bool,
}

/// A transmit command the Link can send.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PhyCmd {
    /// A USB packet with this PID.
    Tx(u8),
    Write(u8),
    Read(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PhyState {
    /// `dir` low, waiting for a transmit command.
    Idle,
    /// The command byte is being consumed; this is what it said.
    Accept(PhyCmd),
    /// A register write: the byte, then the stop that commits it.
    WriteData(u8),
    WriteStop(u8, u8),
    /// A register read: the turnaround cycle, the byte, and letting go.
    ReadTurn(u8),
    ReadData(u8),
    Release,
    /// Taking a USB packet from the Link, then putting it on the pair.
    Collect,
    Line,
    /// Handing a USB packet to the Link.
    RxTurn,
    Rx,
    /// One unsolicited receive command.
    CmdTurn,
    Cmd,
    /// `dir` held while the core resets, or while a receive overrides a
    /// register read.
    Resetting(u32),
    Override(u32),
}

/// A ULPI transceiver: a full-speed line on one side and the ULPI bus on
/// the other, written from the specification in
/// `ip/usb_device_ulpi/README.md`.
///
/// Every cycle it presents `dir`, `nxt` and the data bus, reads what the
/// Link drove, and complains if the Link drove a bus that was not its.
/// It holds LineState still while a packet is on the pair, since a
/// transceiver's LineState during a packet says nothing a Link can use,
/// and reports the SE0-to-J transition of every end of packet, which is
/// what ULPI 1.1 Table 10 has the Link time its answer from.
struct UlpiPhy {
    div: u64,
    /// Registered outputs, held for the cycle about to run.
    dir: bool,
    nxt: bool,
    data: u8,
    was_dir: bool,
    /// What the transceiver drives onto the pair, if anything.
    line_out: Option<UsbLine>,
    state: PhyState,
    regs: BTreeMap<u8, u8>,
    /// Every register access, in order.
    accesses: Vec<UlpiAccess>,
    /// Every transmit command byte the Link drove.
    commands: Vec<u8>,
    /// Every USB packet the Link handed over, PID byte and CRC included.
    packets: Vec<Vec<u8>>,
    /// Interface clocks between each packet's transmit command and the `stp`
    /// that ended it, which is what this model's store-and-forward adds to the
    /// delay a host measures. `UsbPair::added_delay` says why it matters.
    held: Vec<u64>,
    /// Interface clocks since the transmit command being collected.
    tx_ticks: u64,
    /// Cycles the Link held the reset pin low.
    reset_cycles: u64,
    /// Turnaround cycles the transceiver has taken the bus for.
    turnarounds: u64,
    /// Unsolicited receive commands sent out of an idle bus.
    unsolicited: u64,
    /// Transmits abandoned because the transceiver took the bus.
    aborts: u64,
    rx: LineRx,
    rx_bytes: Vec<u8>,
    rx_active: bool,
    rx_error: bool,
    rx_done: bool,
    line_state: u8,
    /// Interface clocks `TermSelect` has been set for, so the pair can be
    /// charging rather than already at J. See `ULPI_PULLUP_SETTLE`.
    pullup: u64,
    /// The last receive command the Link was given.
    sent: u8,
    /// The status has changed since the Link was last told, and the Link
    /// has still not been told.
    ///
    /// This used to be computed as `status() != sent`, which loses a change
    /// that comes and goes while the transceiver is busy with a register
    /// access: the Link is never told, `sent` never moves, and the way back
    /// looks like no change at all. ULPI 1.1 §3.8.1.3 queues a receive
    /// command instead and says the queued one "always carries the current
    /// values, never an old snapshot" — so what is sticky is the *fact* that
    /// the Link is owed one, and what is sent is whatever is true by then.
    owed: bool,
    forced: bool,
    tx: Vec<u8>,
    tx_line: Vec<UsbLine>,
    tx_pos: usize,
    tx_hold: u64,
    ticks: u64,
    problems: Vec<String>,
    // What this transceiver does to make the Link's life harder.
    /// What a turnaround cycle carries. The Link must ignore it.
    garbage: u8,
    /// One cycle in this many has `nxt` low, throttling the Link.
    throttle: u64,
    /// Take the bus away after this many bytes of a USB transmit.
    steal_after: Option<usize>,
    /// End a receive by dropping `dir` with no closing receive command.
    terse_eop: bool,
    /// Answer the first Function Control read with the reset value.
    lie_once: bool,
    lied: bool,
    /// Let a receive override the first register read.
    override_read: bool,
    overrode: bool,
    /// The pull-up never reaches the pair, so it stays at SE0.
    no_pullup: bool,
    /// The Link's own packet is still going out on the pair, so the bus
    /// state machine goes back to `Line` when it has finished whatever it
    /// interrupted with.
    resume_line: bool,
    /// The transceiver's full-speed receiver is **not squelched while it
    /// transmits**, so every receive command it sends during the Link's own
    /// packet reports RxActive. Measured on a Microchip USB3343: `rx_active`
    /// in the Link stuck high for over 1024 clocks, twenty times the longest
    /// packet a host sends, with nothing on the pair but the Link's own
    /// answer. ULPI 1.1 does not forbid it — §3.8.1.3 only says the receive
    /// command at the end of a transmit "contains the status that is current
    /// at the time" — so a Link must not read RxActive in that window as
    /// news about an incoming packet.
    hears_itself: bool,
    /// **Receive commands report LineState late**: what the pair did, in
    /// order, rather than what it is doing now.
    ///
    /// Measured on a Microchip USB3343 on a Cynthion, with a ULPI trace taken
    /// through the board's own console: between a host's SETUP token and its
    /// DATA0 packet — with the pair idle at J the whole time — the transceiver
    /// sent seven receive commands three clocks apart reporting J, K, J, K, J,
    /// K and finally **K**. Those are the bit transitions of the packet that
    /// had just finished, arriving after it: the part reports one per
    /// transition and the ULPI bus can carry one at a time, so the reports
    /// queue and the queue outlives the packet.
    ///
    /// ULPI 1.1 §3.8.1.3 forbids it in so many words — a queued receive
    /// command "must always convey the current RX CMD values, not a previous
    /// or old value" — and this part does it anyway, which is the whole reason
    /// this flag exists. A Link that waits for `LineState == J` before
    /// answering waits on the parity of a backlog, and when the backlog ends
    /// on K nothing changes it back, because a transceiver sends a receive
    /// command only when something *changes*.
    stale_line: bool,
    /// The LineState values still to be reported, oldest first; the last of
    /// them is what the pair is doing now. Only used when `stale_line` is set.
    line_queue: std::collections::VecDeque<u8>,
}

impl UlpiPhy {
    fn new(div: u64) -> UlpiPhy {
        let mut phy = UlpiPhy {
            div,
            dir: false,
            nxt: false,
            data: 0,
            was_dir: false,
            line_out: None,
            state: PhyState::Idle,
            regs: BTreeMap::new(),
            accesses: Vec::new(),
            commands: Vec::new(),
            packets: Vec::new(),
            held: Vec::new(),
            tx_ticks: 0,
            reset_cycles: 0,
            turnarounds: 0,
            unsolicited: 0,
            aborts: 0,
            rx: LineRx::new(div),
            rx_bytes: Vec::new(),
            rx_active: false,
            rx_error: false,
            rx_done: false,
            line_state: 0b00,
            pullup: 0,
            sent: 0,
            owed: false,
            forced: true,
            tx: Vec::new(),
            tx_line: Vec::new(),
            tx_pos: 0,
            tx_hold: 0,
            ticks: 0,
            problems: Vec::new(),
            garbage: 0xAA,
            throttle: 0,
            steal_after: None,
            terse_eop: false,
            lie_once: false,
            lied: false,
            override_read: false,
            overrode: false,
            no_pullup: false,
            resume_line: false,
            hears_itself: false,
            stale_line: false,
            line_queue: std::collections::VecDeque::new(),
        };
        phy.power_on();
        phy
    }

    /// One cycle in `every` has `nxt` low.
    fn throttling(mut self, every: u64) -> UlpiPhy {
        self.throttle = every;
        self
    }

    /// Take the bus away `after` bytes into a USB transmit.
    fn stealing(mut self, after: usize) -> UlpiPhy {
        self.steal_after = Some(after);
        self
    }

    /// End a receive by dropping `dir`, with no closing receive command.
    fn terse(mut self) -> UlpiPhy {
        self.terse_eop = true;
        self
    }

    /// Answer the first Function Control read with the reset value, as a
    /// transceiver that did not take the settings would.
    fn lying(mut self) -> UlpiPhy {
        self.lie_once = true;
        self
    }

    /// Let a receive override the first register read, which ULPI 1.1
    /// §3.8.3.2 says may happen in any cycle of one.
    fn overriding(mut self) -> UlpiPhy {
        self.override_read = true;
        self
    }

    /// A transceiver whose full-speed receiver hears its own transmission.
    /// See `hears_itself`.
    fn hearing_itself(mut self) -> UlpiPhy {
        self.hears_itself = true;
        self
    }

    /// A transceiver that reports LineState **late**, one transition at a
    /// time, so what a receive command carries is the pair's history and not
    /// its present. See `stale_line`.
    fn reporting_stale_line(mut self) -> UlpiPhy {
        self.stale_line = true;
        self
    }

    /// A transceiver whose 1.5 kOhm pull-up never reaches the pair, however
    /// often `TermSelect` is written: a peripheral with no VBUS, which must
    /// not connect one. The pair then reads SE0 for ever, and that is not a
    /// host holding a reset.
    fn unpowered(mut self) -> UlpiPhy {
        self.no_pullup = true;
        self
    }

    /// The power-on state: the reset values of the registers, and no
    /// memory of anything on either side.
    fn power_on(&mut self) {
        self.regs.insert(ULPI_FUNC_CTRL, 0x41);
        self.regs.insert(ULPI_OTG_CTRL, 0x06);
        self.state = PhyState::Idle;
        self.dir = false;
        self.nxt = false;
        self.data = 0;
        self.line_out = None;
        self.rx = LineRx::new(self.div);
        self.rx_bytes.clear();
        self.rx_active = false;
        self.rx_error = false;
        self.rx_done = false;
        self.tx.clear();
        self.tx_line.clear();
        self.forced = true;
        self.resume_line = false;
        // A reset disconnects the pull-up with the register that held it,
        // so the pair falls back to SE0 and has to charge again.
        self.pullup = 0;
        self.line_state = 0b00;
    }

    /// What an undriven pair is at: SE0 until this transceiver's own
    /// `TermSelect` pull-up has been connected long enough to charge it.
    fn idle_line(&self) -> UsbLine {
        if !self.no_pullup && self.pullup >= ULPI_PULLUP_SETTLE {
            UsbLine::J
        } else {
            UsbLine::Se0
        }
    }

    fn problem(&mut self, why: String) {
        self.problems.push(why);
    }

    /// The receive command byte: LineState, VBUS valid, and the receive
    /// event.
    fn status(&self) -> u8 {
        let hearing = self.hears_itself && !self.tx_line.is_empty();
        let event = if !self.rx_active && !hearing {
            0b00
        } else if self.rx_error {
            0b11
        } else {
            0b01
        };
        let line = if self.stale_line {
            self.line_queue.front().copied().unwrap_or(self.line_state)
        } else {
            self.line_state
        };
        line | 0b11 << 2 | event << 4
    }

    /// Whether the Link is owed a receive command.
    fn owed(&self) -> bool {
        self.forced || self.owed || (self.stale_line && self.line_queue.len() > 1)
    }

    fn send_status(&mut self) {
        self.data = self.status();
        self.sent = self.data;
        self.owed = false;
        self.forced = false;
        self.nxt = false;
        self.dir = true;
        // One transition of the backlog has now been reported. The last entry
        // stays, because it is what the pair is doing.
        if self.stale_line && self.line_queue.len() > 1 {
            self.line_queue.pop_front();
        }
    }

    /// What the pair is doing this cycle, which in `stale_line` mode joins the
    /// queue of transitions still to be reported rather than being reported at
    /// once.
    fn observe_line(&mut self, bits: u8) {
        self.line_state = bits;
        if self.stale_line && self.line_queue.back().copied() != Some(bits) {
            self.line_queue.push_back(bits);
        }
    }

    fn idle_out(&mut self) {
        self.dir = false;
        self.nxt = false;
        self.data = 0;
    }

    /// Whether `nxt` is asserted for the next cycle, which is where the
    /// throttle lives.
    fn accept(&mut self) -> bool {
        self.ticks += 1;
        self.throttle == 0 || !self.ticks.is_multiple_of(self.throttle)
    }

    /// A byte to the Link if there is one, a receive command if not,
    /// which is what ULPI does with the cycles of a receive that carry no
    /// data.
    fn rx_out(&mut self) {
        if self.rx_bytes.is_empty() {
            self.send_status();
        } else {
            self.data = self.rx_bytes.remove(0);
            self.nxt = true;
            self.dir = true;
        }
    }

    fn read_reg(&mut self, addr: u8) -> u8 {
        match addr {
            ULPI_DEBUG => self.line_state,
            ULPI_FUNC_CTRL if self.lie_once && !self.lied => {
                self.lied = true;
                0x41
            }
            ULPI_FUNC_CTRL | ULPI_OTG_CTRL => self.regs[&addr],
            other => {
                self.problem(format!("a read of register {other:#04x}, which is not one"));
                0
            }
        }
    }

    fn write_reg(&mut self, addr: u8, value: u8) {
        match addr {
            ULPI_FUNC_CTRL => {
                // The reset bit is not stored: the transceiver performs
                // the reset and clears it (ULPI 1.1 §4.2.2).
                self.regs.insert(addr, value & !0x20);
            }
            ULPI_OTG_CTRL => {
                self.regs.insert(addr, value);
            }
            other => self.problem(format!(
                "a write of {value:#04x} to register {other:#04x}, which is not writable"
            )),
        }
    }

    /// One clock: what the Link drove this cycle, and what the host has
    /// on the pair.
    fn step(&mut self, link: &LinkOut, host: Option<UsbLine>) {
        if link.oe && self.dir {
            self.problem("the link drove the data bus while dir was high".into());
        }
        // The **falling** turnaround is the Link's, and it is not optional.
        // ULPI 1.1 §2.3.1 wants `dir` "wired straight to the output
        // buffers" of both ends, and the Microchip transceiver this model
        // stands for says what happens to a Link that waits a cycle
        // instead: "When the USB334x sends a RXCMD the Link is required to
        // drive the data bus back to idle at the end of the turn around
        // cycle. If the Link does not drive the databus to idle the USB334x
        // may take the information on the data bus as a TXCMD and transmit
        // data on DP and DM until the Link asserts stop" — and its weak
        // pull-downs "are not strong enough to pull the data bus low after
        // a ULPI RXCMD" (DS00002646A §6.5.4.1). A receive command with its
        // ID or `alt_int` bit set is a byte with bit 6 or bit 7 high, which
        // is a transmit command or a register command; left on a floating
        // bus for one cycle it is read back as one.
        //
        // So this model requires what that part requires: in the cycle
        // `dir` falls, the Link drives, and it drives 00h.
        if self.was_dir && !self.dir {
            if !link.oe {
                self.problem(
                    "the link left the bus floating in the cycle dir fell, where a \
                     transceiver reads its own last receive command back as a command"
                        .into(),
                );
            } else if link.data != 0 {
                self.problem(format!(
                    "the link drove {:#04x} and not idle in the cycle dir fell",
                    link.data
                ));
            }
        }
        if link.stp && self.dir {
            self.problem("the link asserted stp while the transceiver had the bus".into());
        }
        if self.dir != self.was_dir {
            self.turnarounds += 1;
        }
        self.was_dir = self.dir;

        if !link.rst_n {
            self.reset_cycles += 1;
            self.power_on();
            return;
        }

        // How long the 1.5 kOhm pull-up has been connected, which is what
        // decides whether an undriven pair is at SE0 or has reached J.
        if self
            .regs
            .get(&ULPI_FUNC_CTRL)
            .is_some_and(|v| v & 0x04 != 0)
        {
            self.pullup += 1;
        } else {
            self.pullup = 0;
        }

        // The line, except while the transceiver owns it: during a
        // transmit the receive path is blocked (ULPI 1.1 §3.8.2.2).
        if !matches!(self.state, PhyState::Line | PhyState::Collect) {
            let seen = host.unwrap_or(self.idle_line());
            match self.rx.step(seen) {
                LineEvent::Byte(b) => self.rx_bytes.push(b),
                LineEvent::Eop => self.rx_done = true,
                LineEvent::Error => {
                    self.rx_error = true;
                    self.rx_done = true;
                }
                LineEvent::Nothing => {}
            }
            if self.rx.active() && !self.rx_active {
                self.rx_active = true;
                self.rx_error = false;
                self.rx_done = false;
            }
            if self.stale_line {
                // Every transition, packet or no packet: that is what makes
                // the backlog.
                self.observe_line(line_bits(seen));
            } else if !self.rx_active {
                // With no packet on it, LineState is the pair.
                self.line_state = line_bits(seen);
            }
        }

        // The pair, if the transceiver is driving it. This runs whatever the
        // bus is doing, because a USB packet on a wire does not pause while
        // a receive command goes out on the ULPI bus.
        if !self.tx_line.is_empty() {
            if self.tx_hold > 1 {
                self.tx_hold -= 1;
            } else {
                self.tx_pos += 1;
                if self.tx_pos >= self.tx_line.len() {
                    self.line_out = None;
                    self.tx_line.clear();
                    self.tx_pos = 0;
                } else {
                    self.tx_hold = self.div;
                    self.line_out = Some(self.tx_line[self.tx_pos]);
                    let bits = line_bits(self.tx_line[self.tx_pos]);
                    self.observe_line(bits);
                }
            }
        }

        // Anything the Link has not been told yet leaves it owed a receive
        // command, and that outlives the change going away again.
        if self.status() != self.sent {
            self.owed = true;
        }

        let want_rx = self.rx_active || !self.rx_bytes.is_empty();
        match self.state {
            PhyState::Idle => {
                if want_rx {
                    // A packet: `dir` and `nxt` together tell the Link
                    // immediately what this is (§3.8.2.4).
                    self.state = PhyState::RxTurn;
                    self.dir = true;
                    self.nxt = true;
                    self.data = self.garbage;
                } else if self.owed() {
                    // A receive command outranks register access (ULPI 1.1
                    // §3.8.1.3), so it goes out even though the Link is
                    // driving a command this cycle — which aborts that
                    // command and has the Link retry it (§3.8.3.1). Taking
                    // the Link's command first instead, as this used to,
                    // means a status change that happens while the Link is
                    // reading a register in a loop is never reported at all:
                    // `sent` moves on without the Link ever having been
                    // told, and the next change back looks like no change.
                    self.state = PhyState::CmdTurn;
                    self.unsolicited += 1;
                    self.dir = true;
                    self.nxt = false;
                    self.data = self.garbage;
                } else if link.oe && link.data != 0 {
                    self.take_command(link.data);
                } else {
                    self.idle_out();
                }
            }
            PhyState::Accept(kind) => match kind {
                PhyCmd::Tx(pid) => {
                    if pid == 0 {
                        self.problem("a transmit command with no PID".into());
                    }
                    self.tx = vec![usb_pid(pid)];
                    self.ticks = 0;
                    self.tx_ticks = 0;
                    self.state = PhyState::Collect;
                    self.nxt = self.accept();
                }
                PhyCmd::Write(addr) => {
                    self.ticks = 0;
                    self.state = PhyState::WriteData(addr);
                    self.nxt = self.accept();
                }
                PhyCmd::Read(addr) => {
                    if self.override_read && !self.overrode {
                        self.overrode = true;
                        self.state = PhyState::Override(4);
                        self.dir = true;
                        self.nxt = true;
                        self.data = self.garbage;
                    } else {
                        self.state = PhyState::ReadTurn(addr);
                        self.dir = true;
                        self.nxt = false;
                        self.data = self.garbage;
                    }
                }
            },
            PhyState::WriteData(addr) => {
                if link.stp {
                    self.problem("a register write ended before its byte".into());
                    self.state = PhyState::Idle;
                    self.idle_out();
                } else if self.nxt {
                    self.state = PhyState::WriteStop(addr, link.data);
                    self.nxt = false;
                } else {
                    self.nxt = self.accept();
                }
            }
            PhyState::WriteStop(addr, value) => {
                if !link.stp {
                    self.problem("a register write was not ended by stp".into());
                    self.state = PhyState::Idle;
                    self.idle_out();
                } else {
                    self.write_reg(addr, value);
                    self.accesses.push(wrote(addr, value));
                    self.idle_out();
                    if addr == ULPI_FUNC_CTRL && value & 0x20 != 0 {
                        // §3.5: the transceiver asserts `dir` and resets
                        // its core, and sends a receive command after it.
                        self.state = PhyState::Resetting(12);
                        self.dir = true;
                        self.data = self.garbage;
                        self.forced = true;
                    } else {
                        self.state = PhyState::Idle;
                    }
                }
            }
            PhyState::ReadTurn(addr) => {
                self.data = self.read_reg(addr);
                self.dir = true;
                self.nxt = false;
                self.state = PhyState::ReadData(addr);
            }
            PhyState::ReadData(addr) => {
                let value = self.data;
                self.accesses.push(got(addr, value));
                self.state = PhyState::Release;
                self.dir = false;
                self.nxt = false;
                self.data = self.garbage;
            }
            PhyState::Release => {
                if self.resume_line && !self.tx_line.is_empty() {
                    self.state = PhyState::Line;
                } else {
                    self.resume_line = false;
                    self.state = PhyState::Idle;
                }
                self.idle_out();
            }
            PhyState::Collect => {
                self.tx_ticks += 1;
                if link.stp {
                    if link.data != 0 {
                        let byte = link.data;
                        self.problem(format!("stp with {byte:#04x} on the bus, not 00h"));
                    }
                    self.send_packet();
                } else if self.nxt {
                    self.tx.push(link.data);
                    // A 64-byte data packet is the PID, 64 bytes and the
                    // CRC16: 67. Anything past that is a Link that never
                    // asserted `stp`, which is a fault and not a long packet.
                    if self.tx.len() > 67 {
                        self.problem("a USB transmit that never ended".into());
                        self.state = PhyState::Idle;
                        self.idle_out();
                    } else if Some(self.tx.len()) == self.steal_after {
                        // §3.8.4.1: the transceiver may take the bus at
                        // any time, and the Link must give up the packet.
                        // Once is enough to make the point.
                        self.steal_after = None;
                        self.aborts += 1;
                        self.tx.clear();
                        self.state = PhyState::CmdTurn;
                        self.dir = true;
                        self.nxt = false;
                        self.data = self.garbage;
                        self.forced = true;
                    } else {
                        self.nxt = self.accept();
                    }
                } else {
                    self.nxt = self.accept();
                }
            }
            PhyState::Line => {
                // The Link's own packet is going out on the pair. The bus is
                // idle for it, and every transition of the line is reported
                // to the Link as a receive command, which is what the part
                // does: "after STP is asserted each FS/LS bit transition will
                // generate a RXCMD since the bit times are relatively slow"
                // (USB334x DS00002646A §6.3.1). The last of them carries the
                // SE0-to-J transition that ULPI 1.1 §3.8.1.3 makes the end of
                // the packet, and that is the one a Link can time from.
                //
                // This used to keep the bus silent for the whole packet and
                // then send **one** receive command, already at J, which is
                // less than ULPI promises and nothing like what the part
                // sends. A Link cannot find its own end of packet in it,
                // which is why this model could not falsify a Link that read
                // every receive command as news about a host.
                if self.tx_line.is_empty() {
                    self.state = PhyState::Idle;
                    self.idle_out();
                } else if self.owed() {
                    self.resume_line = true;
                    self.state = PhyState::CmdTurn;
                    self.unsolicited += 1;
                    self.dir = true;
                    self.nxt = false;
                    self.data = self.garbage;
                } else {
                    self.idle_out();
                }
            }
            PhyState::RxTurn => {
                self.state = PhyState::Rx;
                self.rx_out();
            }
            PhyState::Rx => {
                if self.rx_done && self.rx_bytes.is_empty() {
                    self.rx_active = false;
                }
                if !self.rx_active && (self.terse_eop || self.sent == self.status()) {
                    self.state = PhyState::Release;
                    self.dir = false;
                    self.nxt = false;
                    self.data = self.garbage;
                } else {
                    self.rx_out();
                }
            }
            PhyState::CmdTurn => {
                self.state = PhyState::Cmd;
                self.send_status();
            }
            PhyState::Cmd => {
                if want_rx {
                    self.state = PhyState::Rx;
                    self.rx_out();
                } else if self.owed() {
                    // Back-to-back receive commands, which the Link must
                    // accept any number of (§3.8.1.3).
                    self.send_status();
                } else {
                    self.state = PhyState::Release;
                    self.dir = false;
                    self.nxt = false;
                    self.data = self.garbage;
                }
            }
            PhyState::Resetting(left) => {
                if left == 0 {
                    self.state = PhyState::Idle;
                    self.idle_out();
                } else {
                    self.state = PhyState::Resetting(left - 1);
                    self.dir = true;
                    self.nxt = false;
                    self.data = self.garbage;
                }
            }
            PhyState::Override(left) => {
                if left == 0 {
                    self.state = PhyState::Idle;
                    self.idle_out();
                } else {
                    self.state = PhyState::Override(left - 1);
                    self.dir = true;
                    self.nxt = false;
                    self.data = self.status();
                }
            }
        }
    }

    /// A transmit command byte from the Link.
    fn take_command(&mut self, cmd: u8) {
        let kind = match cmd >> 6 {
            0b01 if cmd & 0b0011_0000 == 0 => Some(PhyCmd::Tx(cmd & 0x0F)),
            0b10 => Some(PhyCmd::Write(cmd & 0x3F)),
            0b11 => Some(PhyCmd::Read(cmd & 0x3F)),
            _ => None,
        };
        match kind {
            Some(kind) => {
                if let PhyCmd::Tx(_) = kind {
                    self.commands.push(cmd);
                }
                self.state = PhyState::Accept(kind);
                // §3.2: never in the first cycle of the transmit command.
                self.nxt = true;
            }
            None => self.problem(format!("a reserved transmit command {cmd:#04x}")),
        }
    }

    /// The Link's packet, onto the pair: the SYNC field, NRZI with the
    /// zeros stuffed, and the end of packet, all of which are the
    /// transceiver's and none of which the Link ever sees.
    fn send_packet(&mut self) {
        let bytes = std::mem::take(&mut self.tx);
        self.tx_line = usb_line(&bytes, true);
        self.packets.push(bytes);
        self.held.push(self.tx_ticks);
        self.tx_pos = 0;
        self.tx_hold = self.div;
        self.line_out = Some(self.tx_line[0]);
        self.line_state = line_bits(self.tx_line[0]);
        self.resume_line = false;
        self.state = PhyState::Line;
        self.idle_out();
    }
}

/// `usb_device_ulpi` behind that transceiver: the host's pair reaches the
/// device through the model, and the device sees a ULPI bus.
struct UlpiPair<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    dir: NetHandle,
    nxt: NetHandle,
    data_in: NetHandle,
    data_out: NetHandle,
    data_oe: NetHandle,
    stp: NetHandle,
    rst_out: NetHandle,
    address: NetHandle,
    configured: NetHandle,
    reset_net: NetHandle,
    ready_net: NetHandle,
    phy: UlpiPhy,
    data: DataEp,
}

impl<'d> UlpiPair<'d> {
    fn new(design: &'d Design) -> UlpiPair<'d> {
        UlpiPair::with_phy(design, UlpiPhy::new(ULPI_CPB))
    }

    /// The device out of reset and the transceiver configured, which is
    /// the start-up sequence run to its end before a host looks.
    fn with_phy(design: &'d Design, phy: UlpiPhy) -> UlpiPair<'d> {
        UlpiPair::with_phy_data(design, phy, true)
    }

    fn with_phy_data(design: &'d Design, phy: UlpiPhy, loopback: bool) -> UlpiPair<'d> {
        let sim = simulate(design, "usb_device_ulpi");
        let pin = |n: &str| top_net(&sim, n);
        let mut pair = UlpiPair {
            clk: pin("clk60"),
            dir: pin("ulpi_dir"),
            nxt: pin("ulpi_nxt"),
            data_in: pin("ulpi_data_i"),
            data_out: pin("ulpi_data_o"),
            data_oe: pin("ulpi_data_oe"),
            stp: pin("ulpi_stp"),
            rst_out: pin("ulpi_rst_n"),
            address: pin("address"),
            configured: pin("configured"),
            reset_net: pin("usb_reset"),
            ready_net: pin("phy_ready"),
            phy,
            data: DataEp::new(&sim, loopback),
            sim,
        };
        pair.data.quiet(&mut pair.sim);
        pair.sim.set(pair.dir, bit(false));
        pair.sim.set(pair.nxt, bit(false));
        pair.sim.set(pair.data_in, word(8, 0));
        let clk = pair.clk;
        let rst_n = top_net(&pair.sim, "rst_n");
        reset(&mut pair.sim, clk, rst_n);
        // Nobody drives the pair while the device starts up, which is the
        // point: a host's downstream port does not drive an idle bus, it
        // holds both lines down through two 15 kOhm resistors, and the bus
        // reaches J only because the device connects its own 1.5 kOhm
        // pull-up and that charges the pair. This used to be
        // `Some(UsbLine::J)` — a pair already at J before the device had
        // asked for it — and that is what hid the Link believing one read of
        // the Debug register. See `ULPI_PULLUP_SETTLE`.
        for _ in 0..8000 {
            if pair.ready() {
                return pair;
            }
            pair.cycle(None);
        }
        panic!(
            "the transceiver was never configured; it saw {:?}",
            pair.phy.accesses
        );
    }

    fn ready(&self) -> bool {
        high(&self.sim, self.ready_net)
    }
}

impl UsbPair for UlpiPair<'_> {
    fn cycles_per_bit(&self) -> u64 {
        ULPI_CPB
    }

    fn cycle(&mut self, host: Option<UsbLine>) {
        step_data(&mut self.sim, &mut self.data);
        // What the transceiver drives for this cycle, presented before
        // the edge that samples it, as every two-domain testbench here
        // does. `ulpi_data_oe` is combinational in `dir` — ULPI means it
        // to be, since `dir` is what a Link's output buffers hang off —
        // so the low phase has to settle before it is read.
        self.sim.set(self.dir, bit(self.phy.dir));
        self.sim.set(self.nxt, bit(self.phy.nxt));
        self.sim
            .set(self.data_in, word(8, u64::from(self.phy.data)));
        self.sim.run_for(HALF);
        let oe = high(&self.sim, self.data_oe);
        let link = LinkOut {
            oe,
            // Only while the Link drives. ULPI's data lines are the
            // Link's for exactly as long as `ulpi_data_oe` is high, and
            // what they carry otherwise is not a value it claims — the
            // same argument `step_data` makes about `out_data` one
            // interface up. On a design whose transmit buffer is a
            // distributed RAM they are `x` until something has filled
            // one, because a distributed RAM cannot be given initial
            // contents; reading them anyway stopped a run that the part
            // itself completes, since a real RAM comes up holding
            // something rather than `x`. Every use of `data` below is
            // inside a state the Link only reaches while driving.
            data: if oe {
                octet(get_u64(&self.sim, self.data_out))
            } else {
                0
            },
            stp: high(&self.sim, self.stp),
            rst_n: high(&self.sim, self.rst_out),
        };
        self.sim.set(self.clk, bit(true));
        self.sim.run_for(HALF);
        self.sim.set(self.clk, bit(false));
        self.phy.step(&link, host);
    }

    fn driven(&mut self) -> Option<UsbLine> {
        self.phy.line_out
    }

    fn address(&self) -> u64 {
        get_u64(&self.sim, self.address)
    }

    fn configured(&self) -> bool {
        high(&self.sim, self.configured)
    }

    fn usb_reset(&self) -> bool {
        high(&self.sim, self.reset_net)
    }

    fn problems(&self) -> &[String] {
        &self.phy.problems
    }

    fn answer_window(&self) -> (u64, u64) {
        // The same two to six and a half bit times, in cycles of a
        // 60 MHz clock instead of a 48 MHz one.
        (2 * ULPI_CPB, 13 * ULPI_CPB / 2)
    }

    fn added_delay(&self) -> Vec<u64> {
        // One interface clock per byte the Link handed over, because this
        // model is store and forward; `UsbPair::added_delay` says the rest.
        self.phy.held.clone()
    }

    fn data(&mut self) -> &mut DataEp {
        &mut self.data
    }

    fn port(&self, name: &str) -> u64 {
        get_u64(&self.sim, top_net(&self.sim, name))
    }

    fn set_port(&mut self, name: &str, value: u64, bits: u32) {
        let net = top_net(&self.sim, name);
        self.sim.set(net, word(bits, value));
    }

    fn clear_memory(&mut self, path: &str) -> bool {
        let full = format!("{}.{path}", self.sim.top_name());
        let Some(mem) = self.sim.memory(&full) else {
            return false;
        };
        for index in 0..self.sim.mem_len(mem) as u64 {
            self.sim.set_mem(mem, index, word(8, 0));
        }
        true
    }
}

fn ulpi_design() -> Design {
    design_of(
        "usb_device_ulpi",
        "usb_device_ulpi",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

/// Runs of the same access collapsed to one, with how long the longest run
/// of each was.
///
/// The start-up's LineState read repeats while the pull-up charges the pair,
/// and how many times is a property of a capacitance rather than of the
/// Link, so the sequence is asserted with each run standing for itself and
/// the repetition asserted separately.
fn collapsed(accesses: &[UlpiAccess]) -> Vec<UlpiAccess> {
    let mut out: Vec<UlpiAccess> = Vec::new();
    for access in accesses {
        if out.last() != Some(access) {
            out.push(*access);
        }
    }
    out
}

/// How many times `accesses` holds exactly this one.
fn times(accesses: &[UlpiAccess], which: &UlpiAccess) -> usize {
    accesses.iter().filter(|a| *a == which).count()
}

/// The start-up sequence, byte for byte: the reset pin held, the reset
/// ULPI itself asks for, the two registers a full-speed peripheral needs,
/// the readback that confirms them and the LineState the device starts
/// from.
#[test]
fn usb_device_ulpi_configures_the_transceiver_before_it_answers() {
    let design = ulpi_design();
    let pair = UlpiPair::new(&design);
    assert_eq!(
        collapsed(&pair.phy.accesses),
        vec![
            // XcvrSelect = 01 (full speed), TermSelect = 1 (the pull-up
            // on D+), SuspendM = 1, and the reset bit.
            wrote(0x04, 0x65),
            // No 15 kOhm pull-downs: they are a host's.
            wrote(0x0A, 0x00),
            // The same settings without the reset bit.
            wrote(0x04, 0x45),
            got(0x04, 0x45),
            // The Debug register, whose low two bits are LineState. The
            // pull-up the write above connected has not charged the pair
            // yet, so the first answers are **SE0** — the line on its way
            // up — and the Link reads again until they are not.
            got(0x15, 0x00),
            // And then J, which is where a full-speed bus idles.
            got(0x15, 0x01),
        ]
    );
    assert!(
        times(&pair.phy.accesses, &got(0x15, 0x00)) > 1,
        "the LineState read was not repeated while the pair charged: {:?}",
        pair.phy.accesses
    );
    assert_eq!(
        times(&pair.phy.accesses, &got(0x15, 0x01)),
        1,
        "the Link went on reading LineState after it had it"
    );
    assert_eq!(pair.phy.regs[&0x04], 0x45, "Function Control");
    assert_eq!(pair.phy.regs[&0x0A], 0x00, "OTG Control");
    assert!(
        pair.phy.reset_cycles >= 300,
        "the reset pin was held for {} cycles, not the 5 us the parameter asks for",
        pair.phy.reset_cycles
    );
    assert_eq!(
        pair.phy.packets,
        Vec::<Vec<u8>>::new(),
        "nothing on the USB"
    );
    assert!(
        pair.phy.problems.is_empty(),
        "the transceiver saw the bus misused:\n  {}",
        pair.phy.problems.join("\n  ")
    );
}

/// The whole enumeration, through the transceiver, by the host model that
/// enumerates `usb_device_fs`.
#[test]
fn usb_device_ulpi_enumerates_through_a_transceiver_model() {
    let design = ulpi_design();
    let mut host = UsbHost::new(UlpiPair::new(&design), 0);
    enumerate(&mut host);

    // The bytes the device put on the ULPI bus, rather than only that
    // something came back: the transmit command and then the packet, with
    // the PID's check nibble the transceiver's own work and the CRC16 the
    // device's.
    let phy = &host.pair.phy;
    assert_eq!(
        phy.commands[..3],
        // The command code 0100 and then the PID: the SETUP's ACK, the whole
        // data stage in one DATA1, and the ACK of the status stage. It was an
        // ACK, a DATA1 and a **DATA0** when a packet held eight bytes and an
        // eighteen-byte descriptor took three of them.
        [0x42, 0x4B, 0x42],
        "the transmit commands of an ACK, a DATA1 and the status stage's ACK"
    );
    let descriptor = expected_device_descriptor(0x1209, 0x0001);
    let mut first = vec![usb_pid(USB_DATA1)];
    first.extend_from_slice(&descriptor);
    first.extend_from_slice(&usb_crc16(&descriptor).to_le_bytes());
    assert_eq!(phy.packets[0], vec![usb_pid(USB_ACK)], "the SETUP's ACK");
    assert_eq!(
        phy.packets[1], first,
        "the whole eighteen-byte descriptor, in one packet"
    );
    // A zero-length data packet is a PID and the CRC16 of nothing.
    assert!(
        phy.packets
            .iter()
            .any(|p| p == &vec![usb_pid(USB_DATA1), 0x00, 0x00]),
        "the status stage of a control write"
    );
    // Every packet cost the bus at least two turnarounds, and the ends of
    // packets were reported out of an idle bus.
    assert!(
        phy.unsolicited > 10,
        "only {} unsolicited receive commands",
        phy.unsolicited
    );
    assert_eq!(phy.aborts, 0, "nothing aborted");
}

#[test]
fn usb_device_ulpi_tracks_a_host_clock_that_is_slow_or_fast() {
    // One bit in sixty-four a cycle long or a cycle short: a host clock
    // 0.4 % off the device's, which is more than the 0.25 % the
    // specification allows, and the transceiver model has to stay locked
    // to it as a transceiver would.
    for drift in [64, -64] {
        let design = ulpi_design();
        let mut host = UsbHost::new(UlpiPair::new(&design), drift);
        enumerate(&mut host);
    }
}

/// The bus turnaround is what ULPI adds, so it is exercised on purpose:
/// every turnaround cycle of every test carries 0xAA, which the Link must
/// ignore, and here the transceiver also throttles the Link with `nxt`.
#[test]
fn usb_device_ulpi_ignores_turnaround_cycles_and_a_throttled_bus() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).throttling(2);
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    enumerate(&mut host);
    assert!(
        host.pair.phy.turnarounds > 40,
        "only {} turnaround cycles",
        host.pair.phy.turnarounds
    );
}

/// `dir` falling ends a received packet as surely as a receive command
/// saying RxActive is 0, and ULPI 1.1 §3.8.2.4 allows either.
#[test]
fn usb_device_ulpi_ends_a_packet_on_dir_alone() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).terse();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    enumerate(&mut host);
}

/// A register read the transceiver never answers, because it asserted
/// `dir` and `nxt` for a USB receive instead. §3.8.3.2 says that may
/// happen in any cycle of a read, and §3.8.3.1 says the Link must retry.
#[test]
fn usb_device_ulpi_retries_a_register_read_a_receive_overrode() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).overriding();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    let reads = host
        .pair
        .phy
        .accesses
        .iter()
        .filter(|a| !a.write && a.addr == 0x04)
        .count();
    assert_eq!(reads, 1, "the read was tried again and then answered");
    enumerate(&mut host);
}

/// A transceiver that **hears its own transmission**, which is the one that
/// is soldered to the board.
///
/// A full-speed receiver has no squelch, so a transceiver that does not gate
/// its own can report RxActive while it is putting the Link's own packet on
/// the pair — and this one does. Measured on a Microchip USB3343 on a Great
/// Scott Gadgets Cynthion: `rx_active` inside the Link high for more than
/// 1024 clocks, twenty times the longest packet this host sends, with
/// nothing on the pair but the device's own answer; `dir` held high across
/// the join into the host's handshake; and `D2h` — an ACK's PID, check
/// nibble and all — arriving on the data bus and being filed as the *k*th
/// byte of a phantom packet instead of the first byte of a handshake.
///
/// What that cost: the data stage never advanced, because the ACK that
/// advances it was never recognised; the same packet went out again with the
/// same toggle for ever; and the host's transfer died of a five second
/// timeout. ULPI 1.1 permits all of it — §3.8.1.3 says a receive command
/// "contains the status that is current at the time the RX CMD is sent" and
/// says nothing about what RxEvent must be during a transmit — so the Link
/// is what has to be right: **a packet is bytes**, and `rx_active` reaches
/// the endpoint only once one has arrived.
///
/// The enumeration asserted here is the same one the straight transceiver
/// gets, byte for byte.
#[test]
fn usb_device_ulpi_enumerates_through_a_transceiver_that_hears_itself() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).hearing_itself();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    enumerate(&mut host);
    assert!(
        host.pair.phy.packets.len() > 4,
        "the device answered {} packet(s)",
        host.pair.phy.packets.len()
    );
    assert_eq!(host.pair.problems(), &[] as &[String]);
}

/// A transceiver that reports LineState **late**, which is the one that is
/// soldered to the board — and the case that kept this device from being
/// enumerated.
///
/// Measured on a Microchip USB3343 on a Great Scott Gadgets Cynthion, with a
/// ULPI trace taken over the board's own console: between a host's SETUP token
/// and its DATA0 packet, with the pair idle at J the whole way, the
/// transceiver sent seven single receive commands three to five clocks apart
/// reporting J, K, J, K, J, K and finally **K**. Those are the bit transitions
/// of the packet that had already finished, arriving after it: one receive
/// command per transition, a bus that carries one at a time, and a backlog
/// that outlives the packet. ULPI 1.1 §3.8.1.3 says a queued receive command
/// "must always convey the current RX CMD values, not a previous or old
/// value"; this part does not.
///
/// What it cost. `usb_ulpi_link` used to report `line_idle` only while
/// `LineState == J`, and after a three-byte token the backlog ends on K — so
/// `line_idle` was false, and *nothing was going to change it*, because a
/// transceiver sends a receive command only when something changes. The answer
/// `usb_ctrl_ep` had ready for the host's IN token was never sent and the
/// transfer died of a five second timeout: `device descriptor read/64,
/// error -110`.
///
/// The Link is what has to be right, twice over: `line_idle` no longer looks
/// at LineState at all — the receive command that clears RxActive already
/// **is** the SE0-to-J transition ULPI 1.1 Table 10 times an answer from, and
/// DS00002646A §6.3.2 says this part does not send it until the pair is idle —
/// and the turnaround counts cycles the bus was quiet for **in a row**, so a
/// backlog draining three clocks at a time cannot let the count creep up to
/// the point where a transmit starts one clock before `dir` rises again.
#[test]
fn usb_device_ulpi_enumerates_through_a_transceiver_that_reports_linestate_late() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    enumerate(&mut host);
    assert!(
        host.pair.phy.packets.len() > 4,
        "the device answered {} packet(s)",
        host.pair.phy.packets.len()
    );
    assert_eq!(host.pair.problems(), &[] as &[String]);
}

/// Both of the part's two surprises at once: it hears its own transmission
/// **and** it reports LineState late. That is what is on the board.
#[test]
fn usb_device_ulpi_enumerates_through_the_transceiver_that_is_on_the_board() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB)
        .hearing_itself()
        .reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    enumerate(&mut host);
    assert_eq!(host.pair.problems(), &[] as &[String]);
}

/// A pair nothing is driving is not a host holding a bus reset.
///
/// This is the other half of what a charging pull-up taught. The Link's
/// start-up reads LineState and gets SE0, because the pull-up it has just
/// connected has not brought the pair up yet — and with a transceiver that
/// has no VBUS and so connects no pull-up at all, it never will. SE0 held
/// for 2.5 us is a bus reset *on a bus*, and a device that has never seen
/// the pair anywhere else has no bus: a transceiver reports LineState only
/// when it **changes**, so nothing would ever arrive to end a reset called
/// from that, and `usb_reset` would be stuck high for as long as the part
/// was configured. `phy_ready` still has to come up, because the LEDs and
/// the readback say something true about the transceiver either way.
#[test]
fn usb_device_ulpi_does_not_call_an_undriven_pair_a_bus_reset() {
    // Twenty attempts rather than the board's forty thousand, so the
    // simulation is short; the Link gives up and goes on, which is the case
    // under test.
    let design = design_of(
        "usb_device_ulpi",
        "usb_device_ulpi",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("LINE_TRIES", "20"),
        ],
    );
    let mut pair = UlpiPair::with_phy(&design, UlpiPhy::new(ULPI_CPB).unpowered());
    assert!(
        pair.ready(),
        "`phy_ready` never came up on a pair that stayed at SE0"
    );
    assert_eq!(
        pair.phy.line_state, 0b00,
        "the model let the pair leave SE0"
    );
    for _ in 0..4000 {
        pair.cycle(None);
        assert!(
            !pair.usb_reset(),
            "an SE0 pair the device had never seen anywhere else was called a bus reset"
        );
    }
    assert!(pair.phy.problems.is_empty(), "{:?}", pair.phy.problems);
}

/// A transceiver that does not take the settings is written to again
/// rather than believed.
#[test]
fn usb_device_ulpi_writes_the_registers_again_when_the_readback_is_wrong() {
    let design = ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).lying();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    assert_eq!(
        collapsed(&host.pair.phy.accesses),
        vec![
            wrote(0x04, 0x65),
            wrote(0x0A, 0x00),
            wrote(0x04, 0x45),
            // The lie, which is the register's reset value.
            got(0x04, 0x41),
            // So the settings go again, and this time they read back.
            wrote(0x0A, 0x00),
            wrote(0x04, 0x45),
            got(0x04, 0x45),
            // SE0 while the pair charges, then J.
            got(0x15, 0x00),
            got(0x15, 0x01),
        ]
    );
    enumerate(&mut host);
}

/// The transceiver takes the bus in the middle of the device's data
/// packet, which §3.8.4.1 allows it to do for reasons ULPI does not
/// specify. The packet is lost, the host hears nothing, and asking again
/// gets the same packet with the same toggle.
#[test]
fn usb_device_ulpi_gives_up_a_packet_the_transceiver_aborts() {
    let design = ulpi_design();
    // Two bytes in: the PID has gone and part of the payload.
    let phy = UlpiPhy::new(ULPI_CPB).stealing(3);
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    host.bus_reset();
    assert_eq!(
        host.setup(0, GET_DEVICE_DESCRIPTOR),
        UsbReply::Handshake(USB_ACK)
    );
    host.idle(4);
    // The data stage: the transceiver aborts this one.
    assert_eq!(
        host.in_token(0),
        UsbReply::Nothing,
        "the aborted packet reached nobody"
    );
    assert_eq!(host.pair.phy.aborts, 1, "one packet abandoned");
    host.idle(20);
    // Asked again, and this time the transceiver lets it through.
    let again = host.in_token(0);
    let descriptor = expected_device_descriptor(0x1209, 0x0001);
    assert_eq!(
        again,
        UsbReply::Data(USB_DATA1, descriptor.clone()),
        "the same packet with the same toggle"
    );
    host.ack();
    // And there is nothing left of the data stage, since a 64-byte packet
    // holds the whole descriptor: the toggle has moved and the packet is
    // empty. It was the next eight bytes when a packet held eight.
    assert_eq!(
        host.in_token(0),
        UsbReply::Data(USB_DATA0, Vec::new()),
        "and then nothing, with the toggle moved on"
    );
    host.assert_clean();
}

#[test]
fn usb_device_ulpi_ignores_bad_packets_and_stalls_what_it_cannot_do() {
    let design = ulpi_design();
    let mut host = UsbHost::new(UlpiPair::new(&design), 0);
    ignore_what_it_cannot_do(&mut host);
}

/// Everything runs on the one 60 MHz clock the board's oscillator gives
/// it and the design forwards to the transceiver. No PLL, and nothing
/// crossing.
#[test]
fn usb_device_ulpi_is_one_clock_domain() {
    let kinds = crossings(
        "usb_device_ulpi",
        "usb_device_ulpi",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    );
    assert!(kinds.is_empty(), "nothing should cross: {kinds:?}");
}

// ---------------------------------------------------------------------------
// usb_cdc_acm: the class layer a host's own serial driver binds to
// ---------------------------------------------------------------------------

/// The interface numbers, the endpoint addresses and the class codes
/// `usb_cdc_acm` states, written here rather than read from it.
const CDC_COMM_IFACE: u8 = 0;
const CDC_DATA_IFACE: u8 = 1;
const CDC_NOTIF_ENDP: u8 = 2;
const CDC_DATA_ENDP: u8 = 1;

/// `wMaxPacketSize` of the notification endpoint, and the ten bytes of a
/// SERIAL_STATE notification it has to hold.
///
/// Sixteen and not ten: USB 2.0 §5.7.3 allows a full-speed **interrupt**
/// endpoint any size up to 64 — unlike a bulk one, which §5.8.3 restricts to
/// 8, 16, 32 and 64 — and `usb_bulk_ep` masks its buffer's byte index to the
/// index's own width, so the size has to be a power of two. Sixteen is the
/// smallest power of two that holds ten bytes.
const CDC_NOTIF_MAXPKT: u8 = 16;
const CDC_SERIAL_STATE_BYTES: usize = 10;

/// What `ip/usb_cdc_acm`'s own designs drive `serial_state` to: `bRxCarrier`
/// and `bTxCarrier` set, every error bit clear.
///
/// PSTN 1.2 §6.5.4 makes bit 0 DCD and bit 1 DSR, and a serial port whose
/// far end is inside the same die has both from the moment it exists. The
/// block's own port comment has the argument at length.
const CDC_LINES_UP: u64 = 0b000_0011;

/// The configuration descriptor `usb_cdc_acm` should send, written
/// **forwards** from the specifications' tables — the way `lsusb -v` prints
/// one — and not from the block's `IFACE_DESC` parameter.
///
/// Every byte here comes from a table named in the comment beside it, which
/// is the whole point of writing it twice: the block states the descriptors
/// as a Verilog concatenation, reversed at elaboration, with three fields
/// counted out of the blob, and this is the same descriptor set arrived at
/// from the other direction. The three derived fields are written as **the
/// arithmetic and not the answer**, so a descriptor added to the block and
/// added here changes both sides consistently and a descriptor added to only
/// one of them fails.
fn expected_cdc_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE 0: the communications interface. bInterfaceClass 02h is
    // CDC 1.1 Table 15, bInterfaceSubClass 02h (Abstract Control Model) is
    // Table 16, and bInterfaceProtocol 00h — "no class specific protocol
    // required" — is Table 17. bNumEndpoints is byte 4 and is counted below.
    iface.extend_from_slice(&[9, 4, CDC_COMM_IFACE, 0, 0, 0x02, 0x02, 0x00, 0]);
    // Header functional descriptor, CDC 1.1 Table 26: bDescriptorType 24h
    // (CS_INTERFACE), bDescriptorSubtype 00h, bcdCDC 0110h.
    iface.extend_from_slice(&[5, 0x24, 0x00, 0x10, 0x01]);
    // Call Management functional descriptor, PSTN 1.2 Table 3:
    // bmCapabilities 00h — no call management — and bDataInterface 1.
    iface.extend_from_slice(&[5, 0x24, 0x01, 0x00, CDC_DATA_IFACE]);
    // Abstract Control Management functional descriptor, PSTN 1.2 Table 4:
    // bmCapabilities 02h, which is D1 — Set_Line_Coding,
    // Set_Control_Line_State, Get_Line_Coding — and nothing else.
    iface.extend_from_slice(&[4, 0x24, 0x02, 0x02]);
    // Union functional descriptor, CDC 1.1 Table 33: interface 1 is
    // subordinate to interface 0. `ip/usb_cdc_acm/README.md` §2 says what is
    // known and what is only understood about a host needing it.
    iface.extend_from_slice(&[5, 0x24, 0x06, CDC_COMM_IFACE, CDC_DATA_IFACE]);
    // ENDPOINT 82h: interrupt IN, sixteen bytes — room for the ten of a
    // SERIAL_STATE — bInterval 16 frames.
    iface.extend_from_slice(&[7, 5, 0x80 | CDC_NOTIF_ENDP, 0x03, CDC_NOTIF_MAXPKT, 0, 16]);
    // INTERFACE 1: the data interface, bInterfaceClass 0Ah (CDC 1.1
    // Table 18).
    iface.extend_from_slice(&[9, 4, CDC_DATA_IFACE, 0, 0, 0x0A, 0x00, 0x00, 0]);
    // ENDPOINT 01h and ENDPOINT 81h: bulk, 64 bytes, which USB 2.0 §5.8.3
    // makes the largest of the four sizes a full-speed bulk endpoint may have.
    let pkt = u8::try_from(BULK_MAXPKT).expect("a legal wMaxPacketSize");
    iface.extend_from_slice(&[7, 5, CDC_DATA_ENDP, 0x02, pkt, 0, 0]);
    iface.extend_from_slice(&[7, 5, 0x80 | CDC_DATA_ENDP, 0x02, pkt, 0, 0]);

    // bNumEndpoints of each interface descriptor: the ENDPOINT descriptors
    // that follow it before the next INTERFACE, counted along the chain of
    // bLength fields. The functional descriptors are walked over rather
    // than counted, because their bDescriptorType is 24h and not 5.
    let mut at = 0;
    while at + 1 < iface.len() {
        let len = usize::from(iface[at]);
        if iface[at + 1] == 4 {
            let mut n = 0;
            let mut k = at + len;
            while k + 1 < iface.len() && iface[k + 1] != 4 {
                if iface[k + 1] == 5 {
                    n += 1;
                }
                k += usize::from(iface[k]);
            }
            iface[at + 4] = u8::try_from(n).expect("a small number");
        }
        at += len;
    }

    let total = u16::try_from(9 + iface.len()).expect("a short descriptor");
    let [lo, hi] = total.to_le_bytes();
    let mut d = vec![
        9,
        2,
        lo,
        hi,
        u8::try_from(count_descriptors(&iface, 4)).expect("a small number"),
        1,
        0,
        0x80,
        50,
    ];
    d.extend(iface);
    d
}

/// The device descriptor a CDC device sends: the class triple in it is
/// `02h 00h 00h`, which CDC 1.1 Table 14 asks for and which is what tells a
/// host the two interfaces are one function before it has read the union
/// descriptor.
fn expected_cdc_device_descriptor() -> Vec<u8> {
    expected_device_descriptor_of(0x1209, 0x0001, [0x02, 0x00, 0x00])
}

fn cdc_fs_design() -> Design {
    design_of(
        "usb_cdc_acm",
        "usb_cdc_acm_fs",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

fn cdc_ulpi_design() -> Design {
    design_of(
        "usb_cdc_acm",
        "usb_cdc_acm_ulpi",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

/// SET_LINE_CODING: bmRequestType 21h, bRequest 20h, wIndex the
/// communications interface, seven bytes of data (PSTN 1.2 §6.3.10).
const CDC_SET_LINE_CODING: [u8; 8] = [0x21, 0x20, 0x00, 0x00, CDC_COMM_IFACE, 0x00, 0x07, 0x00];
/// GET_LINE_CODING: the same the other way (PSTN 1.2 §6.3.11).
const CDC_GET_LINE_CODING: [u8; 8] = [0xA1, 0x21, 0x00, 0x00, CDC_COMM_IFACE, 0x00, 0x07, 0x00];

/// SET_CONTROL_LINE_STATE with DTR and RTS as given: wValue D0 is DTR and
/// D1 is RTS, and there is no data stage (PSTN 1.2 §6.3.12).
fn cdc_set_control_line_state(dtr: bool, rts: bool) -> [u8; 8] {
    let value = u8::from(dtr) | (u8::from(rts) << 1);
    [0x21, 0x22, value, 0x00, CDC_COMM_IFACE, 0x00, 0x00, 0x00]
}

/// A line coding structure, in the order PSTN 1.2 Table 17 gives:
/// dwDTERate little endian, bCharFormat, bParityType, bDataBits.
fn cdc_line_coding(rate: u32, format: u8, parity: u8, bits: u8) -> Vec<u8> {
    let mut d = rate.to_le_bytes().to_vec();
    d.extend_from_slice(&[format, parity, bits]);
    d
}

/// The whole enumeration a host does, against the descriptor set a serial
/// port declares, through both link layers.
///
/// Sixty-seven bytes of configuration descriptor go out in nine packets, so
/// this also exercises a data stage four times longer than any the plain
/// device has — which is where a seven-bit offset that should have been
/// eight, or a `DESC_MAX` too small for the blob, would show.
fn cdc_enumerate<P: UsbPair>(host: &mut UsbHost<P>) {
    // The line state this port reports, driven before anything is configured
    // so that the notification the device owes says something definite. Every
    // test of this block sets it: it is an input, and an input nobody drives
    // is unknown.
    host.set_port("serial_state", CDC_LINES_UP, 7);
    enumerate_descriptors(
        host,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );
}

#[test]
fn usb_cdc_acm_enumerates_as_a_serial_port() {
    let design = cdc_fs_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    cdc_enumerate(&mut host);
}

#[test]
fn usb_cdc_acm_ulpi_enumerates_as_a_serial_port() {
    let design = cdc_ulpi_design();
    let mut host = UsbHost::new(UlpiPair::new(&design), 0);
    cdc_enumerate(&mut host);
}

/// The same through the transceiver that is **on the board**: the one that
/// reports LineState a clock late, against ULPI §3.8.1.3.
#[test]
fn usb_cdc_acm_ulpi_enumerates_through_the_transceiver_that_is_on_the_board() {
    let design = cdc_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    cdc_enumerate(&mut host);
}

/// The descriptor set has the parts a host's driver looks for, said in the
/// terms the driver looks for them in.
///
/// `usb_cdc_acm_enumerates_as_a_serial_port` already compares all
/// sixty-seven bytes, so this adds nothing about the bytes. What it adds is
/// **why those bytes**: a reader changing the descriptors sees which
/// properties are the ones a host binds on, rather than a byte string that
/// must not move: the union descriptor because it is what says the two
/// interfaces are one function, and the interrupt IN endpoint because CDC 1.1
/// §3.2 puts the notification element on one. Both are also understood to be
/// what `cdc_acm` looks for, and `ip/usb_cdc_acm/README.md` §2 and §4 say how
/// sure of that this project is — which is less sure than of the bytes.
#[test]
fn usb_cdc_acm_descriptors_carry_what_a_host_driver_binds_on() {
    let config = expected_cdc_configuration();
    assert_eq!(
        config.len(),
        67,
        "nine bytes of configuration plus fifty-eight"
    );
    assert_eq!(
        u16::from_le_bytes([config[2], config[3]]),
        67,
        "wTotalLength is the sum and not a number typed twice"
    );
    assert_eq!(config[4], 2, "two interfaces");

    // Walk it the way a host does and collect what is there.
    let mut at = 9;
    let mut interfaces: Vec<(u8, u8, u8, u8)> = Vec::new();
    let mut endpoints: Vec<(u8, u8, u16, u8)> = Vec::new();
    let mut functional: Vec<u8> = Vec::new();
    let mut union_fd: Option<Vec<u8>> = None;
    let mut bits = 9 * 8;
    while at + 1 < config.len() {
        let len = usize::from(config[at]);
        assert!(len >= 2 && at + len <= config.len(), "a descriptor at {at}");
        match config[at + 1] {
            4 => interfaces.push((
                config[at + 2],
                config[at + 4],
                config[at + 5],
                config[at + 6],
            )),
            5 => endpoints.push((
                config[at + 2],
                config[at + 3] & 0x03,
                u16::from_le_bytes([config[at + 4], config[at + 5]]),
                config[at + 6],
            )),
            0x24 => {
                functional.push(config[at + 2]);
                if config[at + 2] == 0x06 {
                    union_fd = Some(config[at..at + len].to_vec());
                }
            }
            other => panic!("an unexpected bDescriptorType {other:#04x} at {at}"),
        }
        bits += len * 8;
        at += len;
    }
    assert_eq!(at, config.len(), "the chain of bLength fields ends exactly");
    assert_eq!(
        bits,
        config.len() * 8,
        "every bit of the blob is accounted for"
    );

    // Interface 0 is the communications interface, class 02h subclass 02h,
    // and interface 1 is the data interface, class 0Ah.
    assert_eq!(
        interfaces,
        vec![
            (CDC_COMM_IFACE, 1, 0x02, 0x02),
            (CDC_DATA_IFACE, 2, 0x0A, 0x00),
        ],
        "(bInterfaceNumber, bNumEndpoints, bInterfaceClass, bInterfaceSubClass)"
    );

    // The functional descriptors, in the order CDC 1.1 §5.2.3 gives: the
    // header first, then the rest.
    assert_eq!(
        functional,
        vec![0x00, 0x01, 0x02, 0x06],
        "header, call management, abstract control management, union"
    );
    assert_eq!(
        union_fd,
        Some(vec![5, 0x24, 0x06, CDC_COMM_IFACE, CDC_DATA_IFACE]),
        "the union functional descriptor, which says the two interfaces are one function"
    );

    // Three endpoints: an interrupt IN on the communications interface and a
    // bulk pair on the data interface. bmAttributes 03h is interrupt and
    // 02h is bulk (USB 2.0 Table 9-13).
    let pkt = u16::try_from(BULK_MAXPKT).expect("a legal wMaxPacketSize");
    let notif = u16::from(CDC_NOTIF_MAXPKT);
    assert_eq!(
        endpoints,
        vec![
            (0x80 | CDC_NOTIF_ENDP, 0x03, notif, 16),
            (CDC_DATA_ENDP, 0x02, pkt, 0),
            (0x80 | CDC_DATA_ENDP, 0x02, pkt, 0),
        ],
        "(bEndpointAddress, transfer type, wMaxPacketSize, bInterval)"
    );
    // A SERIAL_STATE notification is ten bytes and this endpoint has to hold
    // them, which is the whole reason the size is not eight any more. The
    // descriptor says what the endpoint does, so the assertion is that it is
    // big enough and legal — not that it is any particular number.
    assert!(
        usize::from(endpoints[0].2) >= CDC_SERIAL_STATE_BYTES,
        "the notification endpoint holds {} bytes and a SERIAL_STATE is {}",
        endpoints[0].2,
        CDC_SERIAL_STATE_BYTES
    );
    assert!(
        endpoints[0].2 <= 64,
        "USB 2.0 §5.7.3 allows a full-speed interrupt endpoint 64 bytes at most"
    );
    // A bulk endpoint may be 8, 16, 32 or 64 and **nothing else** (§5.8.3),
    // which is the one rule about these two that arithmetic can check.
    for (addr, kind, size, _) in &endpoints {
        if *kind == 0x02 {
            assert!(
                matches!(size, 8 | 16 | 32 | 64),
                "endpoint {addr:#04x} declares {size} bytes, which §5.8.3 does not list"
            );
        }
    }

    // bmCapabilities of the Abstract Control Management descriptor is 02h:
    // D1 alone, so no Send_Break and no Comm_Feature, which is exactly the
    // set `usb_cdc_req` claims.
    let acm = config
        .windows(4)
        .find(|w| w[0] == 4 && w[1] == 0x24 && w[2] == 0x02)
        .expect("the abstract control management descriptor");
    assert_eq!(acm[3], 0x02, "D1 set, D0 and D2 clear");
}

/// The three class requests, answered.
///
/// This is the hook working end to end: a SETUP endpoint 0 does not
/// understand, offered to a class, claimed, with its data stage in either
/// direction and its status stage. Without the hook every one of these is a
/// STALL and `cdc_acm` fails to open the port.
fn cdc_class_requests<P: UsbPair>(host: &mut UsbHost<P>) {
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        host,
        3,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    // What it says before a host has asked: 9600 8N1, which is the
    // parameter's reset value.
    let coding = host
        .control_read(3, CDC_GET_LINE_CODING)
        .expect("GET_LINE_CODING is answered and not stalled");
    assert_eq!(
        coding,
        cdc_line_coding(9600, 0, 0, 8),
        "the line coding before a host has set one"
    );
    assert_eq!(host.port("baud"), 9600, "and the same on the block's port");

    // SET_LINE_CODING: 115200, two stop bits, odd parity, seven data bits —
    // deliberately not 8N1, so that a block reporting a constant rather
    // than what it stored is caught, and deliberately a rate whose four
    // bytes are all different from each other.
    let want = cdc_line_coding(115_200, 2, 1, 7);
    assert_eq!(
        host.control_write_data(3, CDC_SET_LINE_CODING, &want),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_LINE_CODING is acknowledged through its status stage"
    );
    host.idle(10);
    assert_eq!(host.port("baud"), 115_200, "dwDTERate reached the port");
    assert_eq!(host.port("char_format"), 2, "bCharFormat");
    assert_eq!(host.port("parity"), 1, "bParityType");
    assert_eq!(host.port("data_bits"), 7, "bDataBits");

    // GET_LINE_CODING gives back what was set, which is the data stage of a
    // class request going the other way.
    let back = host
        .control_read(3, CDC_GET_LINE_CODING)
        .expect("GET_LINE_CODING");
    assert_eq!(back, want, "what was set comes back, byte for byte");

    // A short GET_LINE_CODING: endpoint 0 caps a class data stage at the
    // host's wLength the same way it caps a descriptor's.
    let mut short = CDC_GET_LINE_CODING;
    short[6] = 4;
    let four = host.control_read(3, short).expect("four bytes");
    assert_eq!(four, want[..4], "wLength ends a class data stage too");

    // SET_CONTROL_LINE_STATE: no data stage at all, and the two bits reach
    // the ports. DTR is what a host raises when a program opens the port.
    assert!(
        host.port("dtr") == 0 && host.port("rts") == 0,
        "both low to start"
    );
    assert_eq!(
        host.control_write(3, cdc_set_control_line_state(true, false)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_CONTROL_LINE_STATE is acknowledged"
    );
    host.idle(10);
    assert_eq!(host.port("dtr"), 1, "DTR raised");
    assert_eq!(host.port("rts"), 0, "and RTS not");
    host.control_write(3, cdc_set_control_line_state(true, true));
    host.idle(10);
    assert_eq!(host.port("rts"), 1, "RTS raised");
    host.control_write(3, cdc_set_control_line_state(false, false));
    host.idle(10);
    assert_eq!(host.port("dtr"), 0, "and both dropped again");
    assert_eq!(host.port("rts"), 0);

    host.assert_clean();
}

#[test]
fn usb_cdc_acm_answers_the_line_coding_and_control_line_requests() {
    let design = cdc_fs_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    cdc_class_requests(&mut host);
}

#[test]
fn usb_cdc_acm_ulpi_answers_the_line_coding_and_control_line_requests() {
    let design = cdc_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy_data(&design, phy, false), 0);
    cdc_class_requests(&mut host);
}

/// A class request the block does not claim is stalled, and a standard
/// request is **never** offered to the class.
///
/// Both halves matter. The first is what keeps a device honest about what it
/// implements: bmCapabilities says no SEND_BREAK, so SEND_BREAK is stalled
/// rather than silently accepted, and a class request aimed at the wrong
/// interface is stalled rather than acted on. The second is the hook's
/// safety property — `class_req` is not raised for the requests endpoint 0
/// implements — and it is checked by asking the device to do something a
/// class request could have shadowed and seeing that endpoint 0 still did
/// it.
#[test]
fn usb_cdc_acm_stalls_the_class_requests_it_does_not_claim() {
    let design = cdc_fs_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        &mut host,
        4,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    // Set a line coding that a stalled request must not disturb.
    let kept = cdc_line_coding(19_200, 0, 0, 8);
    assert_eq!(
        host.control_write_data(4, CDC_SET_LINE_CODING, &kept),
        UsbReply::Data(USB_DATA1, Vec::new())
    );
    host.idle(10);

    for (what, request) in [
        // SEND_BREAK, PSTN 1.2 §6.3.13. bmCapabilities D2 is clear, so the
        // device never offered it.
        (
            "SEND_BREAK",
            [0x21, 0x23, 0x10, 0x00, CDC_COMM_IFACE, 0x00, 0x00, 0x00],
        ),
        // GET_COMM_FEATURE, D0 of the same byte and also clear.
        (
            "GET_COMM_FEATURE",
            [0xA1, 0x03, 0x01, 0x00, CDC_COMM_IFACE, 0x00, 0x02, 0x00],
        ),
        // SET_LINE_CODING to the **data** interface, which is not where the
        // requests live.
        (
            "SET_LINE_CODING to the wrong interface",
            [0x21, 0x20, 0x00, 0x00, CDC_DATA_IFACE, 0x00, 0x07, 0x00],
        ),
        // A vendor request, which no class in this device claims.
        (
            "a vendor request",
            [0xC0, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00],
        ),
    ] {
        assert_eq!(
            host.setup(4, request),
            UsbReply::Handshake(USB_ACK),
            "{what}: a SETUP is always acknowledged"
        );
        host.idle(4);
        assert_eq!(
            host.in_token(4),
            UsbReply::Handshake(USB_STALL),
            "{what} is stalled"
        );
        host.idle(10);
    }

    // The line coding is untouched by all of that, which is what says a
    // stalled request reached no register.
    let back = host
        .control_read(4, CDC_GET_LINE_CODING)
        .expect("GET_LINE_CODING still works");
    assert_eq!(back, kept, "a stalled request changed nothing");

    // And the standard requests still belong to endpoint 0: SET_ADDRESS
    // moves the address, and CLEAR_FEATURE(ENDPOINT_HALT) is accepted, both
    // with a class sitting on the hook. A hook that offered the standard
    // requests to the class would have had them stalled, since
    // `usb_cdc_req` claims neither.
    assert_eq!(
        host.control_write(4, set_address(11)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_ADDRESS is endpoint 0's and not the class's"
    );
    assert_eq!(host.address(), 11);
    host.idle(10);
    assert_eq!(
        host.control_write(
            11,
            [0x02, 0x01, 0x00, 0x00, CDC_DATA_ENDP, 0x00, 0x00, 0x00]
        ),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "CLEAR_FEATURE(ENDPOINT_HALT) too"
    );
    host.idle(10);
    host.assert_clean();
}

/// The ten bytes of a SERIAL_STATE notification, as PSTN 1.2 §6.5.4 gives
/// them, for a device reporting the line state `state`.
///
/// Written forwards from the specification — §6.5 for the eight-byte
/// notification header, which has a SETUP packet's shape (USB 2.0 Table 9-2),
/// and §6.5.4 Table 31 for `wSerialState` — rather than from the block's
/// `case`, so that a field in the wrong place fails here.
fn cdc_serial_state(state: u8) -> Vec<u8> {
    vec![
        // bmRequestType: device to host, class, to an interface.
        0xA1,
        // bNotification: SERIAL_STATE.
        0x20,
        // wValue, which this notification does not use.
        0x00,
        0x00,
        // wIndex: the communications interface.
        CDC_COMM_IFACE,
        0x00,
        // wLength: the two bytes that follow.
        0x02,
        0x00,
        // wSerialState, low byte, then the nine bits reserved.
        state,
        0x00,
    ]
}

/// The notification endpoint sends **SERIAL_STATE** whenever the host's idea of
/// the line state could be stale, and NAKs every poll in between.
///
/// Five things, and the **third is the one this test exists for**:
///
///   * the first poll after SET_CONFIGURATION returns the ten bytes, because a
///     host's idea of the line state starts empty and nothing else would fill
///     it — Linux's `cdc_acm` answers `TIOCMGET` out of the last bitmap it was
///     sent;
///   * every poll after that is NAKed, twenty of them, because a state that
///     has not changed is not news;
///   * **SET_CONTROL_LINE_STATE, and then SET_LINE_CODING, each produce one
///     more even though `serial_state` has not moved**, because those requests
///     are a host opening or reconfiguring the port and its idea of the state
///     is then not this device's doing;
///   * a **change** of `serial_state` produces one more, with the new bitmap;
///   * and a bus reset makes the device owe one again, because the host has
///     forgotten what it was told.
///
/// **Why the third one is the assertion that matters.** The first version of
/// this test had the other four and passed against a device that sent
/// **one notification per configuration and never another**, which is a
/// functional defect: `cdc_acm` submits its interrupt URB at `open` and
/// consumes that one, and a `cdc_acm` bound a second time without a bus reset
/// starts with `acm->ctrlin` at zero and nothing left to fill it. On the part
/// that read as `TIOCMGET = 0x026` — no DCD, no DSR — on three consecutive
/// opens. A test that only asks "is a notification ever sent" is satisfied by a
/// one-shot; this one asks for a **second** one after a simulated open, with
/// the state deliberately **unchanged** so that nothing but the request can
/// have caused it.
///
/// It also asserts the thing that would be a real fault: the endpoint has
/// **no OUT direction**, so an OUT token for it is answered with nothing at
/// all rather than with an ACK that would tell a host an endpoint is there
/// which the descriptors do not declare.
///
/// **What it would not catch.** Whether a host's driver acts on the bytes.
/// Nothing in simulation can: this host model is written from the same
/// specification as the device. `tests/usb_cdc_acm.rs` reads the ten bytes off
/// the wire and `ip/usb_cdc_acm/README.md` §5 has `TIOCMGET` off a real kernel
/// across three opens, which is the other half.
fn cdc_notification<P: UsbPair>(host: &mut UsbHost<P>) {
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        host,
        6,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    // The notification the host has been owed since SET_CONFIGURATION. DATA0
    // because SET_CONFIGURATION put every endpoint's toggle back (USB 2.0
    // §9.4.5) and this is the first packet of this one since.
    let up = u8::try_from(CDC_LINES_UP).expect("seven bits");
    let first = host.bulk_in(6, CDC_NOTIF_ENDP);
    assert_eq!(
        first,
        UsbReply::Data(USB_DATA0, cdc_serial_state(up)),
        "the first poll after the host configured the device"
    );
    assert_eq!(
        cdc_serial_state(up).len(),
        CDC_SERIAL_STATE_BYTES,
        "ten bytes, which is what did not fit in an eight-byte endpoint"
    );
    host.ack();
    host.idle(6);

    // And then nothing, for ever, because nothing has changed.
    for poll in 0..20 {
        assert_eq!(
            host.bulk_in(6, CDC_NOTIF_ENDP),
            UsbReply::Handshake(USB_NAK),
            "poll {poll} of an endpoint with nothing new to say"
        );
        host.idle(6);
    }

    // ------------------------------------------------------------------
    // A host opening the port: SET_CONTROL_LINE_STATE, and one more
    // notification although the line state has not moved.
    // ------------------------------------------------------------------
    // This is what `cdc_acm` does at every `open` — `acm_port_dtr_rts` issues
    // it with no comparison against what it last sent — and it is the only
    // sight this class gets of a host arriving. `serial_state` is deliberately
    // left alone, so a notification here can only be the request's doing.
    assert_eq!(
        host.control_write(6, cdc_set_control_line_state(true, true)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_CONTROL_LINE_STATE is acknowledged"
    );
    host.idle(20);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Data(USB_DATA1, cdc_serial_state(up)),
        "a host that has just opened the port is told the state again"
    );
    host.ack();
    host.idle(6);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Handshake(USB_NAK),
        "and then nothing again, because it has now been told"
    );
    host.idle(6);

    // And SET_LINE_CODING does it too, which is the defensive half:
    // `acm_tty_set_termios` only sends it when the coding actually differs, so
    // it is not the one to rely on, but a host that reconfigures the line has
    // as much claim to a fresh state as one that opens the port.
    assert_eq!(
        host.control_write_data(6, CDC_SET_LINE_CODING, &cdc_line_coding(19_200, 0, 0, 8)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_LINE_CODING is acknowledged through its status stage"
    );
    host.idle(20);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Data(USB_DATA0, cdc_serial_state(up)),
        "and a host that has reconfigured the line is told as well"
    );
    host.ack();
    host.idle(6);

    // GET_LINE_CODING is a host **reading**, and says nothing about what the
    // host believes, so it must not produce one.
    let coding = host
        .control_read(6, CDC_GET_LINE_CODING)
        .expect("GET_LINE_CODING");
    assert_eq!(coding, cdc_line_coding(19_200, 0, 0, 8));
    host.idle(20);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Handshake(USB_NAK),
        "a request that only reads is not a host telling this port anything"
    );
    host.idle(6);

    // A change: the carrier drops and a framing error is reported instead. One
    // more notification, with the new bitmap.
    let dropped = 0b001_0000u64;
    host.set_port("serial_state", dropped, 7);
    host.idle(20);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Data(
            USB_DATA1,
            cdc_serial_state(u8::try_from(dropped).expect("seven bits"))
        ),
        "a change of the line state is a notification of its own"
    );
    host.ack();
    host.idle(6);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Handshake(USB_NAK),
        "and then nothing again"
    );
    host.idle(6);

    // Back up again, so the rest of this test is about a port with a carrier.
    host.set_port("serial_state", CDC_LINES_UP, 7);
    host.idle(20);
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Data(USB_DATA0, cdc_serial_state(up)),
        "and back, which is the other direction of the same change"
    );
    host.ack();
    host.idle(6);

    // The endpoint has one direction. An OUT to it is nobody's: the
    // notification endpoint because it has no OUT side, the data endpoint
    // because the number is not its, endpoint 0 because the token is not
    // its either.
    assert_eq!(
        host.bulk_out(6, CDC_NOTIF_ENDP, USB_DATA0, &[0x55]),
        UsbReply::Nothing,
        "an OUT to an IN-only endpoint is not answered"
    );
    host.idle(20);

    // And the data endpoint still works after all of that, which is what
    // says the arbitration between three endpoints did not get stuck on the
    // one that answers nothing.
    let mut pipe = BulkPipe::new(CDC_DATA_ENDP);
    host.data().give = vec![vec![0x2A]];
    host.idle(20);
    assert_eq!(
        pipe.read(host, 6),
        vec![0x2A],
        "the serial port still moves"
    );
    host.idle(10);

    // A bus reset, and the host is owed one again: it has forgotten the state
    // along with the address, so a device that only ever sent one would leave
    // a re-enumerated host with no carrier for ever.
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        host,
        6,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );
    assert_eq!(
        host.bulk_in(6, CDC_NOTIF_ENDP),
        UsbReply::Data(USB_DATA0, cdc_serial_state(up)),
        "a host that has enumerated the device again is told again"
    );
    host.ack();
    host.idle(10);
    host.assert_clean();
}

/// The same, through the **ULPI** link layer and the transceiver that reports
/// LineState a clock late, which is the part on the board.
///
/// The notification is a ten-byte packet out of a *third* endpoint, so it goes
/// through the ULPI transmitter's length field, that endpoint's own turnaround
/// counter and `usb_dev_core`'s arbitration, none of which the full-speed run
/// above exercises. Ten bytes is also the only packet in this file that is
/// neither a handshake, a control transfer nor a bulk payload.
#[test]
fn usb_cdc_acm_ulpi_notification_endpoint_sends_the_serial_state() {
    let design = cdc_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy_data(&design, phy, false), 0);
    cdc_notification(&mut host);
}

#[test]
fn usb_cdc_acm_notification_endpoint_sends_the_serial_state() {
    let design = cdc_fs_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    cdc_notification(&mut host);
}

/// Bytes through the serial port's bulk pair, in both directions, with the
/// class layer above it.
fn cdc_bytes<P: UsbPair>(host: &mut UsbHost<P>) {
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        host,
        7,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    // What a terminal does: a line of text out, a line of text back. `FF`
    // and `07` put six ones in a row on the wire in both directions, and the
    // first one is a **full 64-byte packet** — what a paste into a terminal
    // looks like, and the length the endpoint now declares.
    let out: Vec<Vec<u8>> = vec![
        b"the quick brown fox jumps over the lazy dog, 0123456789 abcdefg!".to_vec(),
        b"hello, w".to_vec(),
        b"orld\n".to_vec(),
        vec![0xFF, 0x07, 0x5A],
    ];
    let back: Vec<Vec<u8>> = vec![
        b"> ".to_vec(),
        Vec::new(),
        b"ok\r\n".to_vec(),
        (0..BULK_MAXPKT)
            .map(|i| {
                u8::try_from(i)
                    .expect("a byte")
                    .wrapping_mul(29)
                    .wrapping_add(3)
            })
            .collect(),
    ];

    let mut pipe = BulkPipe::new(CDC_DATA_ENDP);
    for payload in &out {
        pipe.write(host, 7, payload);
    }
    host.idle(20);
    assert_eq!(
        host.data().got,
        out,
        "every packet reached the interface above the endpoint"
    );

    host.data().give = back.clone();
    for payload in &back {
        assert_eq!(&pipe.read(host, 7), payload, "the packet the port gave");
    }
    host.idle(10);
    host.assert_clean();
}

#[test]
fn usb_cdc_acm_moves_bytes_through_the_serial_port() {
    let design = cdc_fs_design();
    let mut host = UsbHost::new(FsPair::with_loopback(&design, false), 0);
    cdc_bytes(&mut host);
}

#[test]
fn usb_cdc_acm_ulpi_moves_bytes_through_the_serial_port() {
    let design = cdc_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy_data(&design, phy, false), 0);
    cdc_bytes(&mut host);
}

/// The loopback the board's design has, through the class layer: `out_*`
/// wired into `in_*`, so what the host writes to the port it reads back.
///
/// `bulk_loopback` is the plain device's version of this and asserts the
/// vendor descriptors, so this is the same wiring with the serial port's.
#[test]
fn usb_cdc_acm_ulpi_loops_the_serial_port_back_on_itself() {
    let design = cdc_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        &mut host,
        8,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );
    let packets: Vec<Vec<u8>> = vec![
        b"12345678".to_vec(),
        vec![0xDE, 0xAD, 0xBE, 0xEF, 0xFF],
        vec![0x5A],
    ];
    let mut pipe = BulkPipe::new(CDC_DATA_ENDP);
    for payload in &packets {
        pipe.write(&mut host, 8, payload);
        assert_eq!(&pipe.read(&mut host, 8), payload, "what went out came back");
    }
    assert_eq!(
        host.data().got,
        packets,
        "and it went through the interface"
    );
    host.idle(10);
    host.assert_clean();
}

/// `examples/mos6502_monitor`'s USB-facing top, when this copy of the
/// crate has the example.
///
/// Built here rather than in `tests/mos6502_monitor.rs` because the
/// transceiver model and its harness live in this file, and a model of a
/// part that misbehaves the way the one on the board misbehaves is worth
/// more than a second copy of it.
///
/// The parameters make the run tractable: one clock per bus cycle
/// instead of fifty-nine, and a flush timeout of 512 clocks instead of
/// 16384. Neither changes what the machine *does* — the bus is the same
/// bus at either divisor, and the timeout only decides when a partly
/// filled packet goes.
fn monitor_ulpi_design() -> Option<Design> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/mos6502_monitor/rtl");
    if !root.join("monitor_ulpi.v").is_file() {
        println!("skipping: examples/mos6502_monitor is not in this copy of the crate");
        return None;
    }

    let mut sources: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    gather("usb_cdc_acm", &mut seen, &mut sources);
    gather("mos6502", &mut seen, &mut sources);
    for name in [
        "monitor_rom.v",
        "monitor_acia.v",
        "monitor_machine.v",
        "monitor_ulpi.v",
    ] {
        let path = format!("examples/mos6502_monitor/rtl/{name}");
        let text =
            std::fs::read_to_string(root.join(name)).unwrap_or_else(|e| panic!("{path}: {e}"));
        sources.push((path, text));
    }

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
        "the monitor does not parse:\n{}",
        diags.render(&map)
    );

    let options = ElabOptions::new(Dialect::Verilog2005)
        .with_top("monitor_ulpi")
        .with_param("CPU_DIV", "1")
        .with_param("FLUSH_CLKS", "512")
        // The board writes a Microchip USB3343 vendor register to undo a
        // crossed DP/DM pair, and `UlpiPhy` models ULPI's own registers
        // and not that part's extras — so it answers zero to a read of
        // one and the Link retries for ever. The board's register is the
        // board's; every other `usb_cdc_acm_ulpi` test leaves these at
        // their defaults for the same reason.
        .with_param("VENDOR_ADDR", "6'h00")
        .with_param("VENDOR_DATA", "8'h00");
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(
        !diags.has_errors(),
        "monitor_ulpi does not elaborate:\n{}",
        diags.render(&map)
    );
    Some(design.expect("monitor_ulpi produced a design"))
}

/// Reads whatever the device has, giving it time to produce it.
///
/// A serial port that has nothing to say NAKs, and a 6502 that has been
/// asked a question takes thousands of clocks to answer, so this idles
/// first and then reads until two consecutive polls come back empty.
/// The budget is in bit times and never in seconds.
fn monitor_answer<P: UsbPair>(host: &mut UsbHost<P>, pipe: &mut BulkPipe, addr: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut quiet = 0;
    // The first wait is the long one: the monitor takes about four
    // thousand clocks to answer anything, and `FLUSH_CLKS` is five
    // hundred more before a partly filled packet goes.
    host.idle(1400);
    for _ in 0..40 {
        host.idle(400);
        let packet = pipe.read_or_nothing(host, addr);
        if packet.is_empty() {
            quiet += 1;
            if quiet == 2 && !out.is_empty() {
                break;
            }
        } else {
            quiet = 0;
            out.extend_from_slice(&packet);
        }
    }
    out
}

/// The whole machine, through the transceiver that is on the board.
///
/// This is `examples/mos6502_monitor` with nothing left out: a 6502, an
/// ACIA, `usb_cdc_acm`'s class layer, `usb_device_ulpi`'s link layer, and
/// the model of a Microchip USB3343 that **reports LineState late** — the
/// behaviour ULPI 1.1 §3.8.1.3 forbids in so many words and the part on
/// this board has anyway. A host enumerates it, reads the prompt the
/// processor printed, types at it, and reads the answer.
///
/// `tests/mos6502_monitor.rs` proves the monitor, a character at a time,
/// with the USB stack taken out of the way. This proves there is a way
/// through: that a byte a host writes to endpoint 1 reaches `$5000`, and
/// that a byte the 6502 stores at `$5000` comes back on endpoint `81h`.
///
/// **Reading the IN endpoint is not optional here, and that is a fact
/// about the machine.** `usb_bulk_ep` holds `in_ready` low while a
/// packet is armed; the ACIA's TDRE is that signal; and the monitor
/// polls TDRE before every character. So a host that never polls IN
/// stops the 6502 inside `echo` after one byte, and the OUT endpoint
/// then NAKs for ever because nothing is draining it. That is correct
/// behaviour in both blocks and it is what a serial port is — but it
/// means this test cannot check one direction at a time.
///
/// What it would not catch: anything about the board that is not in the
/// model — an unrouted wire, a pad that does not drive, a timing path
/// that does not close. `CLAUDE.md` says why that is not optional, and
/// `examples/mos6502_monitor/README.md` says what was done on the part
/// instead, with the session it was done from.
#[test]
fn a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board() {
    let Some(design) = monitor_ulpi_design() else {
        return;
    };
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy_data(&design, phy, false), 0);
    host.set_port("serial_state", CDC_LINES_UP, 7);
    // The machine's RAM, which it reads as well as writes: a deposit
    // prints the byte that *was* at the address, and this says what was
    // there. `clear_memory`'s own comment has the argument; the short
    // form is that `x` is not what a part comes up holding.
    assert!(
        host.clear_memory("u_machine.ram"),
        "the machine has a RAM and this is where it is"
    );

    // A whole enumeration. The descriptors are `usb_cdc_acm`'s own, built
    // forwards from CDC 1.1 and PSTN 1.2 by `expected_cdc_configuration`,
    // so a machine wrapped around that block still looks to a host
    // exactly like a serial port.
    configure_for(
        &mut host,
        9,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    let mut pipe = BulkPipe::new(CDC_DATA_ENDP);

    // The prompt. The 6502 is held in reset until `configured`, so it
    // starts when the enumeration finishes and prints this into a live
    // endpoint rather than into one a bus reset is about to clear.
    assert_eq!(
        String::from_utf8_lossy(&monitor_answer(&mut host, &mut pipe, 9)),
        "\\\r",
        "the backslash and the carriage return, out of a USB endpoint"
    );

    // Type at it. `FE00` is the ROM's own first byte, the `LDX #$FF` the
    // reset entry begins with.
    pipe.write(&mut host, 9, b"FE00\r");
    assert_eq!(
        String::from_utf8_lossy(&monitor_answer(&mut host, &mut pipe, 9)),
        "FE00\r\rFE00: A2\r",
        "the echo, the monitor's own return, the byte, and the next line"
    );

    // A deposit and a read-back: the byte going into the machine's memory
    // through the same pipe it came out of.
    pipe.write(&mut host, 9, b"0300: 5A A5\r");
    let deposit = monitor_answer(&mut host, &mut pipe, 9);
    assert!(
        deposit.starts_with(b"0300: 5A A5\r"),
        "the deposit was not echoed: {:?}",
        String::from_utf8_lossy(&deposit)
    );
    pipe.write(&mut host, 9, b"0300.0301\r");
    assert_eq!(
        String::from_utf8_lossy(&monitor_answer(&mut host, &mut pipe, 9)),
        "0300.0301\r\r0300: 5A A5\r",
        "what was deposited came back"
    );

    // The monitor wrote $1F into the ACIA's CONTROL register on its way
    // up, and nothing has contradicted it: the host has not changed the
    // rate since the machine started.
    assert_eq!(
        host.port("acia_control"),
        0x1F,
        "what the monitor programmed, untouched"
    );

    // Now a host opens the port at 115200 — which is what a terminal
    // program does, and which the 65C51's four baud bits **cannot
    // name**. The class layer writes code 0 into them, which is the data
    // sheet's own "clocked from outside this part" and the literal truth
    // for a USB pipe, and leaves the four bits the monitor wrote alone.
    assert_eq!(
        host.control_write_data(9, CDC_SET_LINE_CODING, &cdc_line_coding(115_200, 0, 0, 8)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_LINE_CODING"
    );
    host.idle(100);
    assert_eq!(
        host.port("acia_control"),
        0x10,
        "code 0, and $1's top nibble"
    );
    assert_eq!(
        host.port("acia_rate"),
        115_200,
        "and the rate the part is programmed to is the host's own"
    );

    // And the 6502 reads it back over its own bus, which is the whole
    // point of putting a host's rate in a register rather than in a wire
    // nothing inside the machine can see.
    pipe.write(&mut host, 9, b"5003\r");
    assert_eq!(
        String::from_utf8_lossy(&monitor_answer(&mut host, &mut pipe, 9)),
        "5003\r\r5003: 10\r",
        "the processor read the host's rate out of its own ACIA"
    );

    // A rate the table does name puts its code back.
    assert_eq!(
        host.control_write_data(9, CDC_SET_LINE_CODING, &cdc_line_coding(1200, 0, 0, 8)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_LINE_CODING again"
    );
    host.idle(100);
    pipe.write(&mut host, 9, b"5003\r");
    assert_eq!(
        String::from_utf8_lossy(&monitor_answer(&mut host, &mut pipe, 9)),
        "5003\r\r5003: 18\r",
        "1200 baud is code 8 on a 65C51"
    );

    host.idle(10);
}

/// Everything runs on the one clock, in both wrappers.
#[test]
fn usb_cdc_acm_is_one_clock_domain() {
    for top in ["usb_cdc_acm_fs", "usb_cdc_acm_ulpi"] {
        let kinds = crossings(
            "usb_cdc_acm",
            top,
            &[("VID", "16'h1209"), ("PID", "16'h0001")],
        );
        assert!(kinds.is_empty(), "{top}: nothing should cross: {kinds:?}");
    }
}

/// A transceiver model that accepts anything proves nothing, so this
/// drives it with a Link that breaks each of the rules the bus turnaround is
/// made of — including the one that used to be stated backwards. The
/// **rising** turnaround is the transceiver's and the Link must stay off it;
/// the **falling** one is the Link's and it must drive `00h` into it, or the
/// part reads its own last receive command back as a command (USB334x
/// DS00002646A §6.5.4.1).
#[test]
fn the_transceiver_model_catches_a_link_that_drives_a_bus_that_is_not_its() {
    let driving = LinkOut {
        oe: true,
        data: 0x42,
        stp: false,
        rst_n: true,
    };
    // Driving while the transceiver has the bus.
    let mut phy = UlpiPhy::new(ULPI_CPB);
    phy.dir = true;
    phy.was_dir = true;
    phy.step(&driving, Some(UsbLine::J));
    assert!(
        phy.problems
            .iter()
            .any(|p| p.contains("while dir was high")),
        "the model let the link drive the transceiver's bus: {:?}",
        phy.problems
    );

    // Driving something other than idle into the falling turnaround, which
    // the part reads as a command.
    let mut phy = UlpiPhy::new(ULPI_CPB);
    phy.dir = false;
    phy.was_dir = true;
    phy.step(&driving, Some(UsbLine::J));
    assert!(
        phy.problems.iter().any(|p| p.contains("not idle")),
        "the model let the link drive {:#04x} into the falling turnaround: {:?}",
        driving.data,
        phy.problems
    );

    // And leaving that same cycle floating, which is what this Link used to
    // do and what the part warns about: its weak pull-downs cannot pull the
    // bus low after a receive command, so the byte stays there to be read
    // back as a transmit command.
    let mut phy = UlpiPhy::new(ULPI_CPB);
    phy.dir = false;
    phy.was_dir = true;
    phy.step(
        &LinkOut {
            oe: false,
            data: 0,
            stp: false,
            rst_n: true,
        },
        Some(UsbLine::J),
    );
    assert!(
        phy.problems.iter().any(|p| p.contains("floating")),
        "the model let the link leave the falling turnaround floating: {:?}",
        phy.problems
    );

    // And a stop while the transceiver owns the bus, which means
    // something else entirely.
    let mut phy = UlpiPhy::new(ULPI_CPB);
    phy.dir = true;
    phy.was_dir = true;
    phy.step(
        &LinkOut {
            oe: false,
            data: 0,
            stp: true,
            rst_n: true,
        },
        Some(UsbLine::J),
    );
    assert!(
        phy.problems.iter().any(|p| p.contains("stp")),
        "the model let the link abort the transceiver: {:?}",
        phy.problems
    );
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

/// The descriptors a host reads are still the right bytes **after the logic
/// has been mapped onto lookup tables**.
///
/// This is the test that was missing, and it was missing in a way worth
/// spelling out. `blocks_synthesise_cleanly` synthesises every block and
/// `footprints_match_the_documentation` maps every block onto LUT4 and LUT6 —
/// and neither of them ever asks the mapped netlist to *do* anything. Every
/// other behavioural test in this file runs on the design as elaborated. So
/// there was no test anywhere between "the block behaves" and "the bitstream
/// is loaded on a board", and a technology mapper that covered one cone
/// wrongly would be found by a person with an oscilloscope.
///
/// `every_block_maps_to_the_logic_it_was_mapped_from` is the general answer
/// now: it proves every block's mapping equivalent to what it was mapped from,
/// at both widths, which is stronger than reading one ROM. This test stays for
/// what it uniquely does — drive a **host** at a **mapped** netlist and check
/// the bytes that came off a board wrong — and because a byte-level regression
/// in the block itself would show here and nowhere else.
///
/// It was found by one. `ip/usb_cdc_acm`'s fifty-eight byte descriptor set,
/// read off a Cynthion, had `bInterfaceNumber` of its data interface as **0**
/// where the sources say 1 — one byte in sixty-seven — and `cdc_acm` refused
/// the device with `config 1 has 1 interface, different from the descriptor's
/// value: 2`. In simulation the same design was byte-perfect.
///
/// **What was wrong was the LUT4 cover of the descriptor ROM**, and it was a
/// gap in `src/synth/techmap` rather than in these blocks:
///
///   * `usb_ctrl_ep`'s class descriptors are a 512-bit constant read by a
///     variable-indexed part-select, which is a 64-entry ROM — eight
///     independent six-input Boolean functions;
///   * mapped onto LUT4, three of the sixty-seven bytes came out wrong. Onto
///     LUT2, LUT3, LUT5, LUT6, LUT7 or LUT8, none did;
///   * the plain vendor descriptor set, twenty-three bytes in the same
///     512-bit constant, was and is correct at LUT4, so what the fault
///     depended on was the ROM's **contents**;
///   * and it had nothing to do with the class hook: the failing
///     configuration below is `usb_device_fs` with no class layer at all and
///     the longer blob in its parameter.
///
/// **It is fixed.** A cut is reduced to the leaves its function depends on,
/// which leaves a set that is no longer a *cut*, and a parent that merged one
/// had its function computed by simulating a cone over leaves that did not
/// separate it — reading the input they missed as constant zero, and writing
/// the function of a different ROM into a LUT `init`. Contents mattered
/// because which cuts a blob produces decides whether such a merge happens at
/// all. `src/synth/techmap/cuts.rs` has it, `every_cut_computes_its_node` holds
/// the cause and `the_descriptor_rom_cone_maps_to_its_own_function` the effect
/// on the cone this was shrunk to. `usb_ctrl_ep`'s `desc()` reads the blob
/// with one part-select again; the page split that worked around it is gone,
/// and taking it out came out smaller in eleven of the sixteen footprint rows
/// for these blocks and shallower in fourteen, because selecting a page and
/// then a byte out of it is a level of muxing the direct part-select does not
/// need. `docs/ip-library.md` has the table.
///
/// `reticle synth --lut 4 --verify` did **not** catch it, because that option
/// proves the optimised netlist against the unoptimised lowering and the
/// lookup-table mapping happens outside what it compares. That was a second
/// gap and it is closed too: `synth::techmap::verify` proves a mapped network
/// against the AIG it was mapped from, `--verify` runs it for `reticle synth`
/// and `reticle fpga`, and
/// `every_block_maps_to_the_logic_it_was_mapped_from` runs it over the whole
/// library in the gate.
///
/// **What this would and would not catch.** It catches a mapped netlist that
/// answers GET_DESCRIPTOR with the wrong bytes, at two LUT widths, for both
/// descriptor sets in the library — which is the whole of what took the CDC
/// device from working in simulation to refused by a kernel. It does not
/// catch anything the ECP5 or iCE40 flows do *after* mapping: placement,
/// routing and bitstream generation are not here. The Cynthion's own count of
/// wrong bytes was one rather than three because the FPGA flow's mapping is
/// not bit-for-bit `MapOptions::lut(4)`: `fpga::synthesize_for` infers block
/// RAM, carry, IO and clock primitives *between* synthesis and covering, so
/// the mapper sees a different AIG, and one defect covering different cones
/// spoils a different number of bytes. A board is still the last word. This
/// also says nothing about the rest of either block after mapping; the
/// descriptors are what it reads because the descriptors are what a ROM is.
/// And it says nothing about `usb_bulk_ep`'s packet buffers as **arrays**: the
/// cases below take `BUF_RAM = 0` and the comment on them says why.
#[test]
fn usb_descriptors_survive_lookup_table_mapping() {
    // The CDC ACM descriptor set, as `ip/usb_cdc_acm` states it, for the
    // configuration that has no class layer: the same fifty-eight bytes in
    // `usb_device_fs`'s parameter. Written here in descriptor order, which is
    // the order the parameter's concatenation is in.
    const CDC_BLOB: &str = concat!(
        "464'h",
        "090400000002020000", // INTERFACE 0: communications
        "0524001001",         // header functional
        "0524010001",         // call management functional
        "04240202",           // abstract control management functional
        "0524060001",         // union functional
        "07058203100010",     // ENDPOINT 82h: interrupt IN, sixteen bytes
        "09040100000A000000", // INTERFACE 1: data
        "07050102400000",     // ENDPOINT 01h: bulk OUT, 64 bytes
        "07058102400000",     // ENDPOINT 81h: bulk IN, 64 bytes
    );

    /// One configuration to map and read the descriptors off: a variant to
    /// build and the descriptor set it should answer with.
    struct Case {
        variant: Variant,
        want: fn() -> Vec<u8>,
    }

    // Three of them and two LUT widths. The first is the descriptor set that
    // has always worked on a board, the second is the one that did not, and
    // the third is the block that states it for itself.
    //
    // **All three take `BUF_RAM = 0`, and that is a limit of this simulator
    // rather than a preference.** `usb_bulk_ep`'s buffers are arrays by
    // default, so they are memories, and a memory nothing has written yet
    // reads as X. An X out of them is harmless in the design as elaborated —
    // `CellKind::Mux` answers from the input its select chose, so the byte
    // multiplexer `usb_dev_core` shares between the endpoints hands endpoint
    // 0's byte on untouched, which is why every other test in this file
    // passes with the default. After **covering**, that multiplexer is `lut`
    // cells, and `CellKind::Lut` in `src/sim/sched.rs` answers X as soon as
    // any input bit is X, whether its function depends on that input or not.
    // One byte of an unwritten buffer therefore silences the whole device
    // here and GET_DESCRIPTOR returns nothing at all.
    //
    // The shape has nothing to do with what this test is for — the descriptor
    // ROM is a constant in `usb_ctrl_ep` and `BUF_RAM` does not reach it — so
    // the cases take the shape whose netlist a four-state simulator can
    // decide, and the coverage this test had before the buffers became arrays
    // is exactly the coverage it has now. What is **not** covered anywhere is
    // the array shape through a mapped netlist; an X-optimal `lut` evaluation
    // would close that, and it belongs in `src/sim`.
    let cases = [
        Case {
            variant: Variant {
                package: "usb_device_fs",
                top: "usb_device_fs",
                params: &[("VID", "16'h1209"), ("PID", "16'h0001"), ("BUF_RAM", "0")],
            },
            want: expected_configuration_descriptor,
        },
        Case {
            variant: Variant {
                package: "usb_device_fs",
                top: "usb_device_fs",
                params: &[
                    ("VID", "16'h1209"),
                    ("PID", "16'h0001"),
                    ("IFACE_BYTES", "58"),
                    ("IFACE_DESC", CDC_BLOB),
                    ("BUF_RAM", "0"),
                ],
            },
            want: expected_cdc_configuration,
        },
        Case {
            variant: Variant {
                package: "usb_cdc_acm",
                top: "usb_cdc_acm_fs",
                params: &[("VID", "16'h1209"), ("PID", "16'h0001"), ("BUF_RAM", "0")],
            },
            want: expected_cdc_configuration,
        },
    ];

    for Case { variant, want } in cases {
        let (package, top, params) = (variant.package, variant.top, variant.params);
        for k in [4u32, 6] {
            let (mut design, id) = flattened(package, top, params);
            let mut diags = Diagnostics::new();
            synth_run(&mut design, &SynthOptions::default(), &mut diags);
            assert!(
                !diags.has_errors(),
                "{package}.{top} does not synthesise:\n{}",
                diags.len()
            );
            map_module(&mut design.modules[id], &MapOptions::lut(k));

            // The simulator borrows the design for its lifetime and the design
            // is built inside this loop, so it is leaked for the test's sake,
            // the way `tests/sim_cosim.rs` does.
            let mapped: &'static Design = Box::leak(Box::new(design));
            let mut host = UsbHost::new(FsPair::with_loopback(mapped, false), 0);
            host.bus_reset();
            let config = host
                .control_read(0, [0x80, 0x06, 0x00, 0x02, 0xFF, 0xFF, 0xFF, 0xFF])
                .expect("the configuration descriptor");
            let expected = want();
            if config != expected {
                let mut wrong: Vec<String> = Vec::new();
                for (at, (got, wanted)) in config.iter().zip(&expected).enumerate() {
                    if got != wanted {
                        wrong.push(format!("offset {at}: {got:#04x} not {wanted:#04x}"));
                    }
                }
                panic!(
                    "{package}.{top} mapped onto LUT{k} answers GET_DESCRIPTOR with \
                     {} wrong byte(s) out of {}:\n  {}\n\
                     That is the technology mapper covering the descriptor ROM wrongly; this \
                     test's own comment has the whole of what is known about it.",
                    wrong.len(),
                    expected.len(),
                    wrong.join("\n  ")
                );
            }
            host.assert_clean();
        }
    }
}
