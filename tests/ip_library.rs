//! The Reticle IP library: every block elaborated, synthesised, measured
//! and simulated.
//!
//! The library itself is HDL, not Rust: one directory per block under
//! `ip/<category>/`, each a package with a `reticle.ip` manifest and its
//! sources under `rtl/`. These tests reach a block by the **name** its
//! manifest declares and never by its path, so which category a block is
//! filed under is `ip/`'s business. This file is what makes it a *tested*
//! library rather than a folder of Verilog:
//!
//! | Test | What it proves |
//! |------|----------------|
//! | `manifests_parse` | every `reticle.ip` parses, names its own directory and lists files that exist |
//! | `packages_resolve_and_elaborate` | every block builds through `ip::resolve` and `ip::elaborate`, dependencies and all |
//! | `blocks_synthesise_cleanly` | generic synthesis reports nothing — no errors, and no inferred latch |
//! | `usb_descriptors_survive_lookup_table_mapping` | a **mapped** netlist still answers GET_DESCRIPTOR with the right bytes |
//! | `every_block_maps_to_the_logic_it_was_mapped_from` | every block's LUT4 and LUT6 mapping is **proved** equivalent to the logic it came from |
//! | `footprints_match_the_documentation` | the table in `docs/ip-library.md` is the one this run measures |
//! | `crypto_blocks_hold_no_memory_to_index` | neither crypto block holds a memory array, so nothing in one can be addressed by a secret |
//! | `sha256_takes_the_same_cycles_...`, `chacha20_takes_the_same_cycles_...` | the cycle count of both crypto blocks is **measured** to be independent of the message and of the key |
//! | `inflate_decompresses_the_compcol_corpus` | 123 DEFLATE and zlib streams a second, independent compression library produced are decompressed byte for byte, under four different consumers |
//! | `inflate_reports_every_malformed_stream`, `inflate_refuses_or_decodes_every_single_byte_corruption`, `inflate_reports_every_truncation` | every malformed input is **reported** — 21 hand-built streams, 482 corruptions and 235 truncations, and none of the 1196 runs reached its loop bound |
//! | `a_streams_ready_is_a_function_of_registers` | every block with a ready/valid input stream has an `in_ready` the timing graph shows depends on **no input port**, which is the rule `ip/crypto/chacha20`'s header states and nothing used to check |
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
//! The two blocks of `ip/crypto/` are tested against **published vectors
//! and a second implementation**. FIPS 180-4 Appendix B and RFC 8439's
//! §2.1.1, §2.2.1, §2.3.2, §2.4.2, A.1 and A.2 are all here, cited by
//! section, and §2.3.2's sixteen-word intermediate state is read off
//! `chacha20_core`'s working register rather than its port — because a
//! wrong keystream says nothing about *which* quarter round is wrong and
//! that table does. Beside them is a nineteen-length SHA-256 padding table
//! whose digests came from `purecrypto`, the user's from-scratch Rust
//! crypto library, run out of tree: no document publishes nineteen lengths
//! of an arbitrary message, and that is the only way every branch of the
//! padding gets covered. `purecrypto` reproduced every published vector
//! here before it was trusted for the rest.
//!
//! Two of those tests are about *time* rather than values, and they are the
//! point of the category: a hardware block whose cycle count depends on a
//! secret is the direct analogue of a secret-dependent branch. They are
//! measurements — nine maximally different messages per length, eighty-one
//! key, nonce and counter combinations — and each also asserts the outputs
//! were all different, because equal cycle counts from identical runs prove
//! nothing. What they do not establish is anything about power or
//! electromagnetic emission; `docs/ip-library.md` and both blocks' READMEs
//! are careful about that line and so is this file.
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
use std::sync::OnceLock;

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::{self, Constraints, FpgaOptions};
use reticle::ip::{self, IpManifest, LibraryIndex, PathProvider};
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

/// The walk that finds this library's manifests, shared with the example
/// tests that resolve a project against it.
#[path = "library_walk/mod.rs"]
mod library_walk;

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

/// One measured configuration: a package, the module inside it that is
/// measured and simulated, and the parameters it is built with.
struct Variant {
    /// The name the package's own `reticle.ip` declares, which is also
    /// the last component of its directory under `ip/`.
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
    // The one block in this library that is a **host** and not a peripheral,
    // at its own defaults: the waits of an enumeration are milliseconds on a
    // board and the counters that hold them are what most of its registers
    // are, so the row is measured with the numbers a board gets rather than
    // the small ones the simulation uses.
    Variant {
        package: "usb_host_ulpi",
        top: "usb_host_ulpi",
        params: &[],
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
    // The hub, which is the one class in this library with **no bulk
    // endpoint**: `DATA_ENDP = 4'd0` into `usb_dev_core`, a two-byte interrupt
    // IN and a twenty-five byte configuration descriptor. The two rows beside
    // the two CDC ACM rows are what a class costs when none of it is buffers,
    // which is the comparison the table is for.
    Variant {
        package: "usb_hub",
        top: "usb_hub_fs",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    Variant {
        package: "usb_hub",
        top: "usb_hub_ulpi",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    // The proxy: that hub with the forwarding half added, and the biggest
    // block in this library by some way — two ULPI Links, two packet
    // decoders, a device core, a host transaction engine and a 64-byte
    // relay buffer. The row beside `usb_hub_ulpi` is what forwarding costs
    // over reporting a port, and the one beside `usb_host_ulpi` is what the
    // proxy saves by not instantiating `usb_host_enum`: the PC enumerates,
    // so we do not.
    Variant {
        package: "usb_proxy",
        top: "usb_hub_proxy_ulpi",
        params: &[("VID", "16'h1209"), ("PID", "16'h0001")],
    },
    // The `crypto` category: two blocks, each measured at every level it
    // offers, because the levels are the area/throughput argument. The
    // quarter round has a row of its own so that the claim "four of them
    // a cycle costs four quarter rounds and no more depth" is a number
    // and not a sentence.
    Variant {
        package: "sha256",
        top: "sha256_core",
        params: &[],
    },
    Variant {
        package: "sha256",
        top: "sha256",
        params: &[],
    },
    // What the message counter costs. The default spans the whole of
    // FIPS 180-4's length range; 32 bits spans four gigabytes.
    Variant {
        package: "sha256",
        top: "sha256",
        params: &[("LEN_BITS", "32")],
    },
    Variant {
        package: "chacha20",
        top: "chacha20_qr",
        params: &[],
    },
    Variant {
        package: "chacha20",
        top: "chacha20_core",
        params: &[],
    },
    Variant {
        package: "chacha20",
        top: "chacha20",
        params: &[],
    },
    // The `compress` category. `inflate_adler` has a row of its own for
    // the same reason `chacha20_qr` does: RFC 1950 §9 is a testable
    // statement on its own and the row is what prices it. `inflate_window`
    // has one because it is where sixteen of the eighteen block RAMs are,
    // so the two rows together say what the decoder costs and what the
    // window costs.
    Variant {
        package: "inflate",
        top: "inflate_adler",
        params: &[],
    },
    Variant {
        package: "inflate",
        top: "inflate_window",
        params: &[("WINDOW_BITS", "15")],
    },
    Variant {
        package: "inflate",
        top: "inflate",
        params: &[("WINDOW_BITS", "15"), ("WRAPPER", "1")],
    },
    // What RFC 1950's framing costs: the two-byte header check, the
    // four-byte trailer compare and `inflate_adler`.
    Variant {
        package: "inflate",
        top: "inflate",
        params: &[("WINDOW_BITS", "15"), ("WRAPPER", "0")],
    },
    // And what the window costs, which is the question a small part
    // asks: 1 KiB instead of 32, so the table says it rather than
    // leaving a reader to build it.
    Variant {
        package: "inflate",
        top: "inflate",
        params: &[("WINDOW_BITS", "10"), ("WRAPPER", "1")],
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

/// The eight folders `ip/` is grouped into, which
/// `every_block_is_findable_by_the_name_it_declares` holds the layout to.
///
/// They are a filing system and nothing more: no code reads a category,
/// a package's identity is the name its manifest declares, and a block
/// resolves by that name wherever it sits.
const CATEGORIES: [&str; 9] = [
    "bus", "compress", "cpu", "crypto", "memory", "net", "usb", "util", "video",
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
///
/// `rel` is relative to `ip/`, so it starts with a category: the UART's
/// manifest is `bus/uart/reticle.ip`.
fn read(rel: &str) -> Option<String> {
    fs::read_to_string(ip_dir().join(rel))
        .ok()
        .map(|t| t.replace("\r\n", "\n"))
}

/// The real library, indexed by the names its manifests declare, with
/// paths relative to `ip/` — walked once for the whole run.
///
/// Its root is `.` because `read` already starts at `ip/`, which puts
/// the index's paths in the same space as that closure's.
fn library() -> &'static LibraryIndex {
    static INDEX: OnceLock<LibraryIndex> = OnceLock::new();
    INDEX.get_or_init(|| LibraryIndex::from_manifests([".".to_owned()], walk_library(".")))
}

/// Where the package called `name` lives, relative to `ip/`.
///
/// A block sits one level down from `ip/` now — `bus/uart`,
/// `usb/usb_hub` — so a name is no longer a directory, and the index
/// built from the manifests' own `name` lines is what turns one into the
/// other. That is the same answer `reticle build` gets from a project's
/// `library` line, so a block these tests cannot find is a block a build
/// cannot find either.
fn package_dir(name: &str) -> String {
    library()
        .lookup(name)
        .unwrap_or_else(|problem| panic!("`{name}` is not a package under ip/: {problem:?}"))
        .dir
        .clone()
}

/// Parses one package's manifest, failing loudly.
fn manifest(package: &str) -> IpManifest {
    let path = format!("{}/reticle.ip", package_dir(package));
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
/// A dependency is found by its name through [`package_dir`], which is
/// the rule `PathProvider` applies too once it has an index.
fn gather(package: &str, seen: &mut BTreeSet<String>, out: &mut Vec<(String, String)>) {
    if !seen.insert(package.to_owned()) {
        return;
    }
    let dir = package_dir(package);
    let ip = manifest(package);
    for dep in &ip.depends {
        gather(&dep.name, seen, out);
    }
    for source in &ip.sources {
        let path = format!("{dir}/{}", source.path);
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

/// Every package in the library, by the name its manifest declares,
/// sorted.
///
/// Read off the index rather than off `read_dir`, because the top level
/// of `ip/` is eight category folders and not thirty-one packages.
fn packages() -> Vec<String> {
    let mut names: Vec<String> = library()
        .entries()
        .iter()
        .map(|entry| entry.name.clone())
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
        let dir = package_dir(&package);
        // The folder is still named after the package, one category
        // down: `bus/uart` holds `uart`. The redundancy is deliberate —
        // a reader of a path should not have to open a manifest to know
        // which package it is.
        assert_eq!(
            dir.rsplit('/').next().unwrap_or(dir.as_str()),
            package,
            "{dir} holds the package named {package}"
        );
        // And the full parse agrees with the index, which reads `name`
        // by a small scan rather than by parsing.
        let ip = manifest(&package);
        assert_eq!(ip.name, package, "a package's name is its directory");
        assert!(ip.license.is_some(), "{package} has no license");
        assert!(ip.description.is_some(), "{package} has no description");
        assert!(!ip.sources.is_empty(), "{package} lists no source");
        for source in &ip.sources {
            let path = format!("{dir}/{}", source.path);
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
        // which is exactly how a user reaches a library block: one
        // `library` line, and a `depends` that names no path. Its
        // dependencies name no path either, so this is also what proves
        // the search places a *transitive* name — `usb_cdc_acm` asking
        // for `usb_device_fs` across the library, not beside itself.
        let project_text =
            format!("name {package}_check\ntop {top}\n\nlibrary .\n\ndepends {package} *\n");

        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let project = ip::load_project(&mut map, "reticle.proj", &project_text, &mut diags)
            .expect("the generated project parses");
        let mut provider = PathProvider::new(".", read).with_library(library().clone());
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
/// **What it covers less of than it did**, and it is worth knowing which
/// way: `usb_bulk_ep`'s packet buffers are arrays now, so they are memory
/// cells that `MapOptions::lut(k)` does not lower, and the network this
/// proves for every USB block is correspondingly smaller — `usb_device_fs`
/// at LUT4 is 1668 AIG nodes and 804 cells where the shift-register shape
/// is 1964 and 902. Both shapes are in `VARIANTS`, so both are proved; what
/// is outside this check is the memory itself, which is the backend's to
/// build and `tests/fpga_flow.rs`'s
/// `the_logic_fallback_answers_like_the_memory_it_replaced` to answer for.
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
    // Twelve bit periods is a frame and a margin, whatever the period is.
    let budget = 12 * div.max(4) + 200;
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
    for tick in 0..budget {
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
        gaps.contains(&20),
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
        gaps.contains(&4),
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

/// The bit period a host's rate actually produces, in clocks.
///
/// `uart_baud_div` and `uart` are joined by a wire in every design that
/// follows a host's rate — `testdata/fpga/cynthion/usb_cdc_uart.v` and
/// `examples/mos6502_monitor`'s mirror on ball C11 are the two. This
/// joins them by a variable: it runs the divider until it has an answer,
/// takes the number, hands it to `uart`'s `div` port, and measures the
/// gaps between the edges of a frame on `tx`.
///
/// **It is the last link in the claim that a host setting a rate changes
/// a waveform.** `uart_baud_div_computes_the_divisor_for_the_rates_a_host
/// _asks_for` proves the arithmetic and
/// `uart_takes_its_divisor_from_a_port` proves the port is read; this
/// proves that the number one produces is the number the other keeps time
/// with, for the rates a terminal program offers.
///
/// What it would not catch: the wire. Nothing here proves the two blocks
/// are connected *in a design* — that is structural, and
/// `examples/mos6502_monitor/README.md` says which measurement stands in
/// for the oscilloscope nobody here has.
#[test]
fn a_hosts_rate_becomes_a_bit_period() {
    const CLK_HZ: u64 = 60_000_000;
    let div_design = design_of("uart", "uart_baud_div", &[]);

    for rate in [1200u64, 9600, 19_200, 38_400, 115_200, 230_400, 921_600] {
        // The divider, run until it has settled on an answer.
        let mut sim = simulate(&div_design, "uart_baud_div");
        let clk = top_net(&sim, "clk");
        let rst_n = top_net(&sim, "rst_n");
        sim.set(top_net(&sim, "rate"), word(32, rate));
        reset(&mut sim, clk, rst_n);
        for _ in 0..80 {
            cycle(&mut sim, clk, HALF);
        }
        assert!(
            high(&sim, top_net(&sim, "ok")),
            "rate {rate}: the divider refused a rate a terminal offers"
        );
        let div = get_u64(&sim, top_net(&sim, "div"));

        // The transmitter, driven with that number and nothing else. Its
        // CLK_DIV is deliberately *wrong* for this rate — 104 is 115200
        // at 12 MHz — so a period that came from the parameter could not
        // be mistaken for one that came from the port.
        let gaps = uart_bit_clocks(div, "104");
        assert!(!gaps.is_empty(), "rate {rate}: the line never moved");
        for gap in &gaps {
            assert_eq!(
                gap % div,
                0,
                "rate {rate}: a gap of {gap} clocks is not a multiple of {div}: {gaps:?}"
            );
        }
        assert!(
            gaps.contains(&div),
            "rate {rate}: no gap is one bit long at {div} clocks: {gaps:?}"
        );

        // And the rate that period really is, against the one asked for.
        // Integer arithmetic, in parts per thousand, because a test that
        // compares floating point is a test about floating point.
        let slip = (CLK_HZ * 1000).abs_diff(rate * div * 1000) / (rate * div);
        println!(
            "{rate} baud -> {div} clocks a bit -> {} baud, {slip} part(s) per thousand off",
            CLK_HZ / div
        );
        assert!(
            slip * 50 < 1000,
            "rate {rate}: {slip} parts per thousand is over the 2% an 8N1 frame survives"
        );
    }
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
/// Start of frame, which only a **host** sends and which is therefore only
/// used by `usb_host_ulpi`'s tests.
const USB_SOF: u8 = 0b0101;
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

/// A SOF token: the same three bytes any token is, with an 11-bit frame
/// number where a token's address and endpoint number go (USB 2.0 §8.4.3),
/// and the same CRC5 over the same eleven bits.
fn usb_sof(frame: u16) -> Vec<u8> {
    usb_token(
        USB_SOF,
        u8::try_from(frame & 0x7F).expect("seven bits"),
        u8::try_from(frame >> 7 & 0xF).expect("four bits"),
    )
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
/// default: `ip/usb/usb_cdc_acm` says `02h` in the device descriptor as well as
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

    // Requests it does not do are acknowledged and then stalled: a string
    // descriptor, GET_STATUS to an **interface**, GET_STATUS to an
    // **endpoint**, and SET_FEATURE.
    //
    // **GET_STATUS to the device is not in that list any more**, and the next
    // assertion is why: it is a standard request, endpoint 0 implements it now,
    // and `usb_ctrl_ep`'s "THE DEVICE'S OWN STATUS" says why the other two
    // recipients cannot come with it — an interface's and an endpoint's status
    // are answerable only by something that knows which of each exist, which
    // endpoint 0 does not. A device that answered zero for any endpoint number
    // would be claiming endpoints it has not got, where §9.4.5 asks for a
    // STALL. This is the pair of assertions that pins the division.
    for request in [
        [0x80, 0x06, 0x01, 0x03, 0x09, 0x04, 0xFF, 0x00],
        [0x81, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00],
        [0x82, 0x00, 0x00, 0x00, 0x81, 0x00, 0x02, 0x00],
        [0x00, 0x03, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
    ] {
        assert_eq!(host.setup(0, request), UsbReply::Handshake(USB_ACK));
        host.idle(4);
        assert_eq!(host.in_token(0), UsbReply::Handshake(USB_STALL));
        host.idle(10);
    }

    // And GET_STATUS to the **device** is answered, with the two bytes USB 2.0
    // §9.4.5 and Table 9-4 describe: bit 0 Self Powered, which is bit 6 of
    // `CFG_ATTR` and is clear for the `80h` every device in this library
    // declares, and bit 1 Remote Wakeup Enabled, which is clear because nothing
    // here implements SET_FEATURE(DEVICE_REMOTE_WAKEUP) — which is the
    // request stalled two lines above.
    //
    // **A host asking for more than two bytes gets two**, which is the short
    // packet that ends a control read, and asking for one gets one: both are
    // asserted, because the data stage is capped at `min(wLength, 2)` and a
    // cap that was the wrong way round would pass the first and fail the
    // second.
    assert_eq!(
        host.control_read(0, [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00])
            .expect("GET_STATUS to the device"),
        vec![0, 0],
        "bus powered, remote wake-up not enabled"
    );
    host.idle(10);
    assert_eq!(
        host.control_read(0, [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00])
            .expect("GET_STATUS asked for more than it is"),
        vec![0, 0],
        "a wLength of 255 gets the two bytes there are and a short packet"
    );
    host.idle(10);
    assert_eq!(
        host.control_read(0, [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00])
            .expect("GET_STATUS asked for one byte"),
        vec![0],
        "and a wLength of one gets one byte, capped the other way"
    );
    host.idle(10);

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
/// The read-only registers that say what the part is, and the one that
/// reports its interrupt sources. ULPI 1.1 Table 19 for the addresses;
/// `read_reg` says where the values come from.
const ULPI_VENDOR_ID_LOW: u8 = 0x00;
const ULPI_VENDOR_ID_HIGH: u8 = 0x01;
const ULPI_PRODUCT_ID_LOW: u8 = 0x02;
const ULPI_PRODUCT_ID_HIGH: u8 = 0x03;
const ULPI_INT_STATUS: u8 = 0x13;

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
/// `ip/usb/usb_device_ulpi/README.md`.
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
    /// What the pair is doing **this cycle**, whatever the status being
    /// reported says. `line_state` is the field a receive command carries and
    /// is deliberately held still during a packet (and queued, with
    /// `stale_line`); this is the pair itself, and the end of a packet is
    /// timed from it.
    line_now: u8,
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
    /// Whether this transceiver is a **host's** rather than a peripheral's.
    ///
    /// It changes two things and neither is on the ULPI bus, which is why one
    /// model serves both: what an undriven pair is at, and whether the
    /// transceiver can drive SE0 onto it.
    ///
    /// A peripheral's `TermSelect` connects a 1.5 kOhm pull-up to D+, so a
    /// pair nothing is driving goes to **J** once it has charged. A host's
    /// connects nothing: a host has two 15 kOhm pull-downs and an idle
    /// downstream port is **SE0** until a device pulls a line up. And a host
    /// drives SE0 for a bus reset by switching the 45 Ohm high-speed
    /// terminations on with no transmitter running, which is
    /// `XcvrSelect = 00b` with `TermSelect = 0b` — ULPI 1.1 §3.8.5.1 step 2
    /// and the `HSTERM_EN` column of USB334x DS00002646A Table 5-1's "Host
    /// Chirp" row. `drives_se0` is that condition and nothing else.
    host: bool,
    /// The transceiver's own registers above `30h`, which ULPI reserves and
    /// describes not at all (§4.1).
    ///
    /// `39h` is a USB3343's "USB IO & Power Management", whose bit 1 is
    /// `SwapDP/DM` and whose reset value is `04h` (DS00002646A §7.1.3.5). It
    /// is here so that a Link told to write one can be watched writing it and
    /// reading it back; nothing in this model acts on the bit, because what
    /// it does is exchange two pins on the other side of the pair and this
    /// model has no pins.
    vendor: BTreeMap<u8, u8>,
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
            line_now: 0b00,
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
            host: false,
            vendor: BTreeMap::new(),
        };
        phy.power_on();
        phy
    }

    /// The same transceiver on a **host's** port. See `host`.
    fn hosting(mut self) -> UlpiPhy {
        self.host = true;
        self
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
        // The reset value DS00002646A §7.1.3.5 gives `39h`: `SwapDP/DM`
        // clear, the UART-mode regulator at its default of `01b`. A
        // transceiver's own registers survive the core reset of §3.5 — "The
        // RESET bit in the Function Control Register does not reset the bits
        // of the ULPI register array" (§7.1) — but the pin reset this model
        // calls `power_on` for does not, so it is set here and not in `new`.
        self.vendor.insert(0x39, 0x04);
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
    ///
    /// **A host's port has no pull-up at all** and so is at SE0 for ever
    /// until something else pulls a line up, which is what makes an empty
    /// socket and a device that is still charging the pair look the same from
    /// this end and is why `usb_ulpi_host_link` reads LineState **once**
    /// where the peripheral's Link reads it until it is not SE0.
    fn idle_line(&self) -> UsbLine {
        if !self.host && !self.no_pullup && self.pullup >= ULPI_PULLUP_SETTLE {
            UsbLine::J
        } else {
            UsbLine::Se0
        }
    }

    /// Whether this transceiver's 1.5 kOhm pull-up has had time to bring the
    /// pair up, which is what a harness joining two of these needs to know to
    /// decide what an undriven pair is at.
    fn pullup_ready(&self) -> bool {
        !self.host && !self.no_pullup && self.pullup >= ULPI_PULLUP_SETTLE
    }

    /// Whether this transceiver is **driving SE0** onto the pair through its
    /// 45 Ohm high-speed terminations, which is how a ULPI host performs a
    /// bus reset:
    ///
    ///   "If a host detects a full speed peripheral, it resets the peripheral
    ///    by writing to the Function Control register and setting
    ///    XcvrSelect = 00b (HS) and TermSelect = 0b which drives SE0 on the
    ///    bus (D+ and D- connected to ground via 45 Ohm)."
    ///    — ULPI 1.1 §3.8.5.1 step 2
    ///
    /// The same two fields with both pull-downs set are the "Host Chirp" and
    /// "Host High Speed" rows of USB334x DS00002646A Table 5-1, and they are
    /// the only two rows of it with `HSTERM_EN` asserted on a host — so this
    /// is the datasheet's condition and not a reading of it. `OpMode` does not
    /// come into it: §3.8.5.1 asks a host to set `10b` as well, but that is
    /// "for correct chirp transmit and receive" and changes the encoder, not
    /// the resistors.
    fn drives_se0(&self) -> bool {
        let func = self.regs.get(&ULPI_FUNC_CTRL).copied().unwrap_or(0);
        let otg = self.regs.get(&ULPI_OTG_CTRL).copied().unwrap_or(0);
        // XcvrSelect = 00, TermSelect = 0, and a host's two pull-downs.
        func & 0x07 == 0x00 && otg & 0x06 == 0x06
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
            // The four read-only bytes that say what the part is, at the
            // addresses ULPI 1.1 Table 19 gives and with the values a
            // Microchip USB3343 has: Vendor ID `0424h` and Product ID
            // `0009h` (DS00002646A Table 7-1, §7.1.1.1 to §7.1.1.4). The
            // first of those has been **read off a part** on this board;
            // `ip/usb/usb_device_ulpi/README.md` §11 has that measurement and
            // the other three are quoted from the same table.
            ULPI_VENDOR_ID_LOW => 0x24,
            ULPI_VENDOR_ID_HIGH => 0x04,
            ULPI_PRODUCT_ID_LOW => 0x09,
            ULPI_PRODUCT_ID_HIGH => 0x00,
            // USB Interrupt Status, which "dynamically updates to reflect
            // current status of interrupt sources" (DS00002646A §7.1.1.10).
            // Nothing in this model has an interrupt source, so it is zero —
            // and on a part it reads zero for `VbusValid` too whenever that
            // comparator's two interrupt-enable bits are set, which they are
            // by default. The receive command's `VbusState` is the field
            // worth reading and this is here so a probe can be watched
            // reading the other one.
            ULPI_INT_STATUS => 0x00,
            // The transceiver's own registers (§4.1). A read of one this
            // model has never been given is zero rather than a complaint: the
            // addresses are reserved to the part and what they answer is not
            // ULPI's business.
            0x30..=0x3F => self.vendor.get(&addr).copied().unwrap_or(0),
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
            0x30..=0x3F => {
                self.vendor.insert(addr, value);
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
        //
        // **`tx_line` and not only the bus state**, and the difference is
        // what a harness that joins two of these finds out. The ULPI bus
        // leaves `Line` over and over during a packet — every transition of
        // the line is a receive command and each costs a turnaround, a byte
        // and a release — so the state alone says the transceiver is idle for
        // a third of its own transmission. A harness that hands the *pair*
        // to both ends then has this receiver assemble its own packet into
        // bytes and deliver them to its own Link, which is nothing any
        // transceiver does: a full-speed receiver is squelched while its
        // transmitter drives. Measured in exactly that harness: a host read
        // back its own SETUP token and its own DATA0 packet, byte for byte,
        // and then waited for a handshake it had already filed as a packet.
        //
        // `hears_itself` is **not** this and is still a real thing a part
        // does: it is RxActive being reported in the receive command's status
        // during a transmit, which says nothing about bytes. That flag stays
        // exactly as it was.
        //
        // The device tests were never affected, because the harness they run
        // in hands this model `None` while the device transmits and `None`
        // means "the other end has let go" — so the receiver was sampling an
        // idle line rather than the packet. Which is the same lesson: a model
        // is only as good as the question its harness asks it.
        if !matches!(self.state, PhyState::Line | PhyState::Collect) && self.tx_line.is_empty() {
            let seen = host.unwrap_or(self.idle_line());
            self.line_now = line_bits(seen);
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
                // **RxActive is not cleared until the pair is back at idle**,
                // and that is the part's own rule and not a reading of it:
                //
                //   "In Full Speed, the USB334x will not issue a Rxactive
                //    de-assertion in the RXCMD until the DP/DM linestate
                //    transitions to idle. This prevents the Link from
                //    violating the two Full Speed bit times minimum turn
                //    around time."
                //    — USB334x DS00002646A §6.3.1
                //
                // The receiver knows a packet is over at the **first** SE0 of
                // its end-of-packet, two and a half bit times before the pair
                // is anybody's again. A transceiver that reported it there
                // would hand its Link a starting gun for a window that USB
                // measures from the SE0-to-J transition, and a Link answering
                // promptly from the wrong end of it drives the pair while the
                // other end still is. This model used to do exactly that, and
                // nothing noticed until two of them were joined pair to pair:
                // the device's answer and the host's next token each
                // overlapped the other end's end-of-packet by a few cycles.
                //
                // `line_now` and not `line_state`, because `line_state` is the
                // field a receive command carries and is held still during a
                // packet.
                if self.rx_done && self.rx_bytes.is_empty() && self.line_now != 0b00 {
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
// usb_host_ulpi: the other end of the wire, against our own device
// ---------------------------------------------------------------------------

/// The stages `usb_host_enum` reports on `stage`, which is what a test that
/// waits for one has to name.
const E_OFF: u64 = 0;
const E_IDLE: u64 = 1;
const E_LOWSPEED: u64 = 3;
const E_DEV8: u64 = 9;
const E_UP: u64 = 17;
const E_FAIL: u64 = 18;

/// `usb_host_sie`'s `trn_status`, as `fail_status` reports it, plus the two
/// codes `usb_host_enum` adds for a failure that was not a transaction.
const ST_TIMEOUT: u64 = 4;
const FAIL_NOT_J: u64 = 6;

/// `usb_host_ulpi` behind a transceiver model of its own, with the pair
/// brought out so that something can be put on the other end of it.
///
/// This is `UlpiPair` turned round. It is **not** a `UsbPair`: that trait is
/// the far end of a pair as a *host* reaches it, and this is the host.
struct UlpiHost<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    dir: NetHandle,
    nxt: NetHandle,
    data_in: NetHandle,
    data_out: NetHandle,
    data_oe: NetHandle,
    stp: NetHandle,
    rst_out: NetHandle,
    enum_en: NetHandle,
    reg_start: NetHandle,
    reg_write: NetHandle,
    reg_addr: NetHandle,
    sof_sent: NetHandle,
    desc_valid: NetHandle,
    desc_data: NetHandle,
    desc_index: NetHandle,
    desc_tag: NetHandle,
    desc_done: NetHandle,
    desc_len: NetHandle,
    phy: UlpiPhy,
    /// The descriptor bytes the block streamed out, by `desc_tag`, written at
    /// the offset it gave each one — which is the whole of what a design has
    /// to do with the stream, and the reason it has an offset.
    desc: [Vec<u8>; 2],
    /// How long each descriptor was when `desc_done` said it was whole.
    desc_whole: [Option<usize>; 2],
    /// Frames sent.
    sofs: u64,
}

impl<'d> UlpiHost<'d> {
    /// The block out of reset and the transceiver configured, which is the
    /// start-up sequence run to its end with nothing on the pair — an empty
    /// socket, which is what a host's own two pull-downs make of one.
    fn new(design: &'d Design, phy: UlpiPhy) -> UlpiHost<'d> {
        let mut host = UlpiHost::fresh(design, phy);
        for _ in 0..8000 {
            if host.ready() {
                return host;
            }
            host.cycle(Some(UsbLine::Se0));
        }
        panic!(
            "the transceiver was never configured; it saw {:?}",
            host.phy.accesses
        );
    }

    /// Out of reset and **not** stepped at all, for a caller that has
    /// something on the other end of the pair and has to resolve it itself.
    ///
    /// The start-up reads the Debug register, so what the pair is doing while
    /// it runs is what the host's `line_state` starts at: a host brought up
    /// against a device that is already there must not be told the port is
    /// empty.
    fn fresh(design: &'d Design, phy: UlpiPhy) -> UlpiHost<'d> {
        let sim = simulate(design, "usb_host_ulpi");
        let pin = |n: &str| top_net(&sim, n);
        let mut host = UlpiHost {
            clk: pin("clk60"),
            dir: pin("ulpi_dir"),
            nxt: pin("ulpi_nxt"),
            data_in: pin("ulpi_data_i"),
            data_out: pin("ulpi_data_o"),
            data_oe: pin("ulpi_data_oe"),
            stp: pin("ulpi_stp"),
            rst_out: pin("ulpi_rst_n"),
            enum_en: pin("enum_en"),
            reg_start: pin("reg_start"),
            reg_write: pin("reg_write"),
            reg_addr: pin("reg_addr"),
            sof_sent: pin("sof_sent"),
            desc_valid: pin("desc_valid"),
            desc_data: pin("desc_data"),
            desc_index: pin("desc_index"),
            desc_tag: pin("desc_tag"),
            desc_done: pin("desc_done"),
            desc_len: pin("desc_len"),
            phy,
            desc: [Vec::new(), Vec::new()],
            desc_whole: [None, None],
            sofs: 0,
            sim,
        };
        host.sim.set(host.dir, bit(false));
        host.sim.set(host.nxt, bit(false));
        host.sim.set(host.data_in, word(8, 0));
        host.sim.set(host.enum_en, bit(false));
        host.sim.set(host.reg_start, bit(false));
        host.sim.set(host.reg_write, bit(false));
        host.sim.set(host.reg_addr, word(6, 0));
        host.sim.set(top_net(&host.sim, "reg_wdata"), word(8, 0));
        let clk = host.clk;
        let rst_n = top_net(&host.sim, "rst_n");
        reset(&mut host.sim, clk, rst_n);
        host
    }

    fn ready(&self) -> bool {
        high(&self.sim, top_net(&self.sim, "phy_ready"))
    }

    fn port(&self, name: &str) -> u64 {
        get_u64(&self.sim, top_net(&self.sim, name))
    }

    fn flag(&self, name: &str) -> bool {
        high(&self.sim, top_net(&self.sim, name))
    }

    fn driven(&self) -> Option<UsbLine> {
        self.phy.line_out
    }

    /// One clock, with `pair` the state of the D+ / D- pair this cycle.
    fn cycle(&mut self, pair: Option<UsbLine>) {
        // What the transceiver drives for this cycle, presented before the
        // edge that samples it, as every two-domain testbench here does.
        self.sim.set(self.dir, bit(self.phy.dir));
        self.sim.set(self.nxt, bit(self.phy.nxt));
        self.sim
            .set(self.data_in, word(8, u64::from(self.phy.data)));
        self.sim.run_for(HALF);
        let oe = high(&self.sim, self.data_oe);
        let link = LinkOut {
            oe,
            // Only while the Link drives, for the reason `UlpiPair::cycle`
            // gives: what the lines carry otherwise is not a value it claims.
            data: if oe {
                octet(get_u64(&self.sim, self.data_out))
            } else {
                0
            },
            stp: high(&self.sim, self.stp),
            rst_n: high(&self.sim, self.rst_out),
        };
        if high(&self.sim, self.sof_sent) {
            self.sofs += 1;
        }
        if high(&self.sim, self.desc_valid) {
            let tag = usize::try_from(get_u64(&self.sim, self.desc_tag)).expect("a tag");
            let at = usize::try_from(get_u64(&self.sim, self.desc_index)).expect("an index");
            let byte = octet(get_u64(&self.sim, self.desc_data));
            if tag < 2 {
                if self.desc[tag].len() <= at {
                    self.desc[tag].resize(at + 1, 0);
                }
                self.desc[tag][at] = byte;
            }
        }
        if high(&self.sim, self.desc_done) {
            let tag = usize::try_from(get_u64(&self.sim, self.desc_tag)).expect("a tag");
            let len = usize::try_from(get_u64(&self.sim, self.desc_len)).expect("a length");
            if tag < 2 {
                self.desc_whole[tag] = Some(len);
            }
        }
        self.sim.set(self.clk, bit(true));
        self.sim.run_for(HALF);
        self.sim.set(self.clk, bit(false));
        self.phy.step(&link, pair);
    }

    /// Lets the enumeration start, which is also what hands the register port
    /// over to it.
    fn enable(&mut self) {
        self.sim.set(self.enum_en, bit(true));
    }

    /// One register read through the block's own port, the way a design that
    /// probes the transceiver does it: hold the request until the port has
    /// taken it, then wait for `reg_done`. `None` is `reg_ok` low.
    fn read_register(&mut self, addr: u8, line: UsbLine) -> Option<u8> {
        self.sim.set(self.reg_write, bit(false));
        self.sim.set(self.reg_addr, word(6, u64::from(addr)));
        self.sim.set(self.reg_start, bit(true));
        let mut taken = false;
        for _ in 0..4000 {
            self.cycle(Some(line));
            if !taken && self.flag("reg_busy") {
                taken = true;
                self.sim.set(self.reg_start, bit(false));
            }
            if self.flag("reg_done") {
                self.sim.set(self.reg_start, bit(false));
                return if self.flag("reg_ok") {
                    Some(octet(self.port("reg_rdata")))
                } else {
                    None
                };
            }
        }
        self.sim.set(self.reg_start, bit(false));
        panic!("a register read of {addr:#04x} never finished");
    }

    /// Cycles with the pair held where the caller says, which is how a port
    /// with nothing in it and a port with a device that will not answer are
    /// both modelled without a second simulation.
    fn run_alone(&mut self, line: UsbLine, cycles: u64) {
        for _ in 0..cycles {
            self.cycle(Some(line));
        }
    }

    /// Cycles until `want` is the stage, or `limit` cycles, with the pair
    /// held. Says whether it got there.
    fn until_stage_alone(&mut self, line: UsbLine, want: u64, limit: u64) -> bool {
        for _ in 0..limit {
            if self.port("stage") == want {
                return true;
            }
            self.cycle(Some(line));
        }
        self.port("stage") == want
    }
}

/// Our host and our own device, each behind a transceiver model, with the
/// pair between them resolved by the only three things that drive it.
///
/// Both halves are this repository's, and that is the point of the test and
/// also its limit: it establishes that the two agree, not that either
/// agrees with anybody else's. `ip/usb/usb_device_ulpi` has already been
/// enumerated by a real host on a real board, which is what makes the
/// device side of this a fixed point rather than a second guess — so a
/// disagreement here is the host's.
struct HostDevice<'d> {
    host: UlpiHost<'d>,
    dev: UlpiPair<'d>,
    problems: Vec<String>,
}

impl<'d> HostDevice<'d> {
    fn new(host_design: &'d Design, dev_design: &'d Design, stale: bool) -> HostDevice<'d> {
        let dev_phy = if stale {
            UlpiPhy::new(ULPI_CPB)
                .hearing_itself()
                .reporting_stale_line()
        } else {
            UlpiPhy::new(ULPI_CPB)
        };
        // The device first, and on its own: its start-up has to connect its
        // own pull-up and wait for the pair to charge, and until it has there
        // is nothing on the pair for a host to see. That is the order the two
        // ends really come up in on a board, where the device is plugged into
        // a port that has been idle.
        let dev = UlpiPair::with_phy(dev_design, dev_phy);
        let host_phy = if stale {
            UlpiPhy::new(ULPI_CPB)
                .hosting()
                .hearing_itself()
                .reporting_stale_line()
        } else {
            UlpiPhy::new(ULPI_CPB).hosting()
        };
        // The host's own start-up has to run with the device already on the
        // pair, because it reads the Debug register and what that says is
        // where this host thinks the port is. Brought up against SE0 and then
        // joined, it would start out believing the socket is empty.
        let host = UlpiHost::fresh(host_design, host_phy);
        let mut both = HostDevice {
            host,
            dev,
            problems: Vec::new(),
        };
        for _ in 0..8000 {
            if both.host.ready() {
                return both;
            }
            both.cycle();
        }
        panic!(
            "the host's transceiver was never configured; it saw {:?}",
            both.host.phy.accesses
        );
    }

    /// One cycle of both, with the pair worked out between them.
    fn cycle(&mut self) {
        let h = self.host.driven();
        let d = self.dev.driven();
        if h.is_some() && d.is_some() {
            self.problems
                .push("both ends drove the pair in the same cycle".into());
        }
        // Three things decide what the pair is at, in this order:
        //
        //   * the host's 45 Ohm terminations to ground, which are what a bus
        //     reset is and which beat a 1.5 kOhm pull-up by a factor of
        //     thirty (ULPI 1.1 §3.8.5.1);
        //   * whichever end is transmitting;
        //   * the device's own pull-up, once it has charged the pair — and
        //     SE0 before that, because a host's two 15 kOhm pull-downs are
        //     the only other thing on the wire.
        let line = if self.host.phy.drives_se0() {
            UsbLine::Se0
        } else if let Some(state) = h.or(d) {
            state
        } else if self.dev.phy.pullup_ready() {
            UsbLine::J
        } else {
            UsbLine::Se0
        };
        self.host.cycle(Some(line));
        self.dev.cycle(Some(line));
    }

    /// Cycles until the host's stage is `want` or `E_FAIL`, or `limit` cycles
    /// go by. Returns the stage it stopped at, so a test can say what it saw
    /// rather than only that it did not see what it wanted.
    fn until_stage(&mut self, want: u64, limit: u64) -> u64 {
        for _ in 0..limit {
            let at = self.host.port("stage");
            if at == want || at == E_FAIL {
                return at;
            }
            self.cycle();
        }
        self.host.port("stage")
    }

    /// Everything either model complained about, and the pair's own rule.
    fn complaints(&self) -> Vec<String> {
        let mut out = self.problems.clone();
        out.extend(self.host.phy.problems.iter().cloned());
        out.extend(self.dev.phy.problems.iter().cloned());
        out
    }
}

/// The host at the scale a simulation can run: the waits USB measures in
/// milliseconds, in hundreds of clocks.
///
/// Every one of these is a **time** on a board and a count here, and the
/// ratio is about fifteen thousand to one. What that does and does not
/// weaken is worth being exact about, because it is the one thing in this
/// test that is not the design that goes on a part:
///
///   * it **keeps** every ordering — the debounce before the reset, the reset
///     before the first transaction, the status stage of `SET_ADDRESS` before
///     the new address is used — because each is a state and not a duration;
///   * it **keeps** the bus reset long enough to be one: 400 clocks is more
///     than `usb_device_ulpi`'s own `SE0_CYCLES` of 150, so the device really
///     does see a reset and really does forget its address;
///   * it **does not** check that 100 ms of debounce is 100 ms, that 10 ms of
///     SE0 is 10 ms, or that a device given 2 ms after `SET_ADDRESS` has
///     enough. Those are numbers a host must get right against a **device's**
///     patience and no simulation of two of our own blocks can falsify them.
///     They are in `usb_host_enum`'s parameters with the section of USB 2.0
///     that sets each, and the defaults are what a board gets.
///
/// `FRAME_CYCLES` is small for the opposite reason: at the real 60 000 a
/// whole enumeration fits inside two frames and the SOF would hardly appear.
/// At 4 000 it lands between transactions over and over, which is where a
/// frame that interrupted one would be caught.
const HOST_TEST_PARAMS: &[(&str, &str)] = &[
    ("DEBOUNCE_CYCLES", "200"),
    ("RESET_HOLD", "400"),
    ("RESET_RECOVERY", "400"),
    ("ADDR_SETTLE", "200"),
    ("NAK_CYCLES", "40000"),
    ("FRAME_CYCLES", "4000"),
];

fn host_design() -> Design {
    design_of("usb_host_ulpi", "usb_host_ulpi", HOST_TEST_PARAMS)
}

/// The same block with a board's vendor register to write, which is the one
/// thing on a Cynthion that stands between a host and calling every
/// full-speed device low speed.
fn host_design_with_vendor() -> Design {
    let mut params = HOST_TEST_PARAMS.to_vec();
    params.push(("VENDOR_ADDR", "6'h39"));
    params.push(("VENDOR_DATA", "8'h06"));
    design_of("usb_host_ulpi", "usb_host_ulpi", &params)
}

/// The start-up sequence, byte for byte: the reset pin held, the reset ULPI
/// itself asks for, the two registers a full-speed **host** needs, the
/// readback that confirms them, and LineState read **once**.
///
/// The two differences from
/// `usb_device_ulpi_configures_the_transceiver_before_it_answers` are the
/// whole of what makes this a host, and they are worth having a test say
/// rather than a comment:
///
///   * `0Ah` is `06h` and not `00h` — `DpPulldown` and `DmPulldown`, a host's
///     two 15 kOhm pull-downs, where a peripheral clears both
///     (ULPI 1.1 §3.8.5.3.2);
///   * the Debug register is read once and the answer is **SE0**, and that is
///     not a failure. A peripheral reads it again and again while it says SE0
///     because the pull-up it has just connected is still charging the pair;
///     a host has no pull-up, and SE0 is what an empty socket is.
#[test]
fn usb_host_ulpi_configures_the_transceiver_as_a_host() {
    let design = host_design();
    let host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    assert_eq!(
        collapsed(&host.phy.accesses),
        vec![
            // XcvrSelect = 01 (full speed), TermSelect = 1, OpMode = 00,
            // SuspendM = 1, and the reset bit §3.5 asks for.
            wrote(0x04, 0x65),
            // The two 15 kOhm pull-downs, which are a host's.
            wrote(0x0A, 0x06),
            // The same settings without the reset bit.
            wrote(0x04, 0x45),
            got(0x04, 0x45),
            // LineState, once: an idle downstream port is SE0.
            got(0x15, 0x00),
        ]
    );
    assert_eq!(
        times(&host.phy.accesses, &got(0x15, 0x00)),
        1,
        "a host read LineState more than once; it has no pull-up to wait for"
    );
    assert_eq!(host.phy.regs[&0x04], 0x45, "Function Control");
    assert_eq!(host.phy.regs[&0x0A], 0x06, "OTG Control");
    assert!(
        host.phy.reset_cycles >= 300,
        "the reset pin was held for {} cycles, not the 5 us the parameter asks for",
        host.phy.reset_cycles
    );
    assert_eq!(
        host.phy.packets,
        Vec::<Vec<u8>>::new(),
        "nothing on the USB: the port is empty and nothing has been enabled"
    );
    assert!(
        host.phy.problems.is_empty(),
        "the transceiver saw the bus misused:\n  {}",
        host.phy.problems.join("\n  ")
    );
}

/// The board's own register, written before anything else and read back.
#[test]
fn usb_host_ulpi_writes_the_boards_vendor_register_before_it_drives_anything() {
    let design = host_design_with_vendor();
    let host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    assert_eq!(
        collapsed(&host.phy.accesses),
        vec![
            wrote(0x04, 0x65),
            // 39h bit 1 is a USB3343's `SwapDP/DM`, and `06h` is the
            // register's reset value with that bit set. It goes **before**
            // Function Control's second write, so the pair is never read
            // through the wrong pins even for a moment.
            wrote(0x39, 0x06),
            got(0x39, 0x06),
            wrote(0x0A, 0x06),
            wrote(0x04, 0x45),
            got(0x04, 0x45),
            got(0x15, 0x00),
        ]
    );
    assert_eq!(host.phy.vendor[&0x39], 0x06);
    assert!(host.phy.problems.is_empty());
}

/// The probe: what the block is for before it is a host at all.
///
/// With `enum_en` low the register port is the design's, and the three
/// questions a port nobody has measured raises are the three a register read
/// and a receive command answer. Nothing is driven while this happens, and
/// that is asserted rather than assumed.
#[test]
fn usb_host_ulpi_reads_the_transceiver_before_it_drives_anything() {
    let design = host_design();
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());

    // What the part is. A Microchip USB3343 answers `0424h` and `0009h`;
    // `ip/usb/usb_device_ulpi/README.md` §11 has the first of those read off a
    // transceiver on this board.
    assert_eq!(host.read_register(0x00, UsbLine::Se0), Some(0x24));
    assert_eq!(host.read_register(0x01, UsbLine::Se0), Some(0x04));
    assert_eq!(host.read_register(0x02, UsbLine::Se0), Some(0x09));
    assert_eq!(host.read_register(0x03, UsbLine::Se0), Some(0x00));
    // How this host left Function Control and OTG Control.
    assert_eq!(host.read_register(0x04, UsbLine::Se0), Some(0x45));
    assert_eq!(host.read_register(0x0A, UsbLine::Se0), Some(0x06));
    // The vendor register, whose reset value is `04h` on a USB3343 and which
    // this build was not told to write.
    assert_eq!(host.read_register(0x39, UsbLine::Se0), Some(0x04));

    // The receive command: VBUS valid, SE0 on the pair, no packet. The model
    // reports VBUS valid always, so what this checks is that the field
    // reaches the port at all and in the right bits, not what a board's
    // supply is doing.
    //
    // **That there is one at all** is the whole reason
    // `usb_ulpi_host_link` believes the bus from the end of the transceiver's
    // own reset rather than from the end of its start-up. ULPI 1.1 §3.5
    // promises exactly one receive command after that reset, and on a port
    // whose VBUS and whose pair have not changed since, it is the only one
    // that will ever arrive. A Link that waits for `phy_ready` throws it away
    // and then has nothing to report VBUS from, for ever.
    assert!(
        host.flag("rx_cmd_seen"),
        "no receive command was kept, so VbusState can never be read"
    );
    assert_eq!(host.port("line_state"), 0b00, "an empty port is SE0");
    assert_eq!(host.port("vbus_state"), 0b11, "the model's VBUS is valid");

    assert_eq!(
        host.phy.packets,
        Vec::<Vec<u8>>::new(),
        "the probe put something on the USB"
    );
    assert!(host.phy.problems.is_empty());
}

/// An empty socket is not an attachment, and a host that has not been told
/// anything is attached does not send a frame to it.
///
/// This is the mirror of
/// `usb_device_ulpi_does_not_call_an_undriven_pair_a_bus_reset` and it is
/// the same fact from the other end: SE0 on a host's port is the resting
/// state and means nothing has happened.
#[test]
fn usb_host_ulpi_does_not_call_an_empty_port_an_attachment() {
    let design = host_design();
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    host.enable();
    // Far longer than `DEBOUNCE_CYCLES`, so a host that counted SE0 as an
    // attachment would have got there.
    host.run_alone(UsbLine::Se0, 5000);
    assert_eq!(
        host.port("stage"),
        E_IDLE,
        "a host left an empty port for stage {}",
        host.port("stage")
    );
    assert!(!host.flag("attached"));
    assert_eq!(
        host.phy.packets,
        Vec::<Vec<u8>>::new(),
        "a host sent {} packet(s) to an empty port",
        host.phy.packets.len()
    );
    assert_eq!(host.sofs, 0, "a host framed an empty port");
    assert!(host.phy.problems.is_empty());
}

/// A device that pulls **D-** up is a low-speed device, and this host says so
/// and sends it nothing.
#[test]
fn usb_host_ulpi_reports_a_low_speed_device_and_does_not_talk_to_it() {
    let design = host_design();
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    host.enable();
    assert!(
        host.until_stage_alone(UsbLine::K, E_LOWSPEED, 5000),
        "a host at stage {} never called a D- pull-up low speed",
        host.port("stage")
    );
    assert!(host.flag("attached"));
    assert!(host.flag("low_speed"));
    // Nothing is sent, and nothing can be: a low-speed packet needs the
    // transceiver told to prepend a preamble, which this Link does not do.
    host.run_alone(UsbLine::K, 3000);
    assert_eq!(host.phy.packets, Vec::<Vec<u8>>::new());
    assert!(host.phy.problems.is_empty());
}

/// A device that is there and answers nothing: the host resets it, asks for
/// its device descriptor, retries, and **reports** rather than hanging.
///
/// The pair is held at J by the harness and no device is simulated at all, so
/// every token this host sends goes nowhere. What it checks is the one thing
/// a host must do that nothing else in this file does: give up.
#[test]
fn usb_host_ulpi_gives_up_on_a_device_that_never_answers() {
    let design = host_design();
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    host.enable();
    // J is a full-speed device's pull-up, so the host debounces it, resets
    // it, re-reads LineState and starts asking. Every reply times out.
    assert!(
        host.until_stage_alone(UsbLine::J, E_FAIL, 200_000),
        "a host at stage {} neither enumerated a device that is not there nor gave up",
        host.port("stage")
    );
    assert!(host.flag("attached"), "it did see the pull-up");
    assert!(host.flag("failed"));
    assert_eq!(
        host.port("fail_stage"),
        E_DEV8,
        "it should stop at the first thing it asks for"
    );
    assert_eq!(
        host.port("fail_status"),
        ST_TIMEOUT,
        "nothing came back, so it is a timeout and not an error"
    );
    // It really did send the tokens, and really did send them again: a SETUP
    // and its data packet, four times over, which is `RETRIES` of 3 plus the
    // first try. **And the bytes are asserted**, against this file's own
    // packet arithmetic rather than against themselves — the same `usb_token`
    // and `usb_data` the host *model* builds with, whose CRC5 and CRC16 are
    // held to the published catalogue's check values by
    // `usb_crcs_match_the_catalogue_and_the_wire` before anything is held to
    // them. So this is the hardware's CRC5 and CRC16 against a second
    // implementation, and the second one is pinned to a third.
    let want_token = usb_token(USB_SETUP, 0, 0);
    let want_data = usb_data(USB_DATA0, &get_descriptor(0x01, 8));
    assert_eq!(
        host.phy.packets,
        vec![
            want_token.clone(),
            want_data.clone(),
            want_token.clone(),
            want_data.clone(),
            want_token.clone(),
            want_data.clone(),
            want_token,
            want_data
        ],
        "four tries of a SETUP token and its GET_DESCRIPTOR data packet"
    );
    // **No frame went out**, and that is this design and not a defect: the
    // whole of this run is one control transfer being retried, `H_IDLE` is
    // never reached between the tries, and `usb_host_sie` only sends a SOF
    // from `H_IDLE`. On a board the four tries are 273 us and a frame is
    // 1 ms, so a frame is late and not missing; here the transfer is the
    // entire life of the bus. `usb_host_sie`'s header says what it would take
    // to make a SOF interrupt a transaction and why it has not been done.
    assert_eq!(
        host.sofs, 0,
        "a retried transaction leaves no room for a frame at these timings"
    );
    assert!(
        host.phy.problems.is_empty(),
        "the transceiver saw the bus misused:\n  {}",
        host.phy.problems.join("\n  ")
    );
}

/// **Our host enumerates our own device**, through a pair of transceiver
/// models with the D+ / D- pair between them.
///
/// Both halves are in this tree and neither is a model of the other: the
/// device is `ip/usb/usb_device_ulpi`, which a real host has enumerated on
/// a real board, and the host is this package. What the test asserts is
/// bytes — the eighteen of the device descriptor and the thirty-two of the
/// configuration descriptor, each compared with the same `expected_*`
/// function the device's own tests compare a *host model's* reading against
/// — and not that something came back.
fn enumerate_our_device(stale: bool) {
    let host_design = host_design();
    let dev_design = ulpi_design();
    let mut both = HostDevice::new(&host_design, &dev_design, stale);

    // Nothing has been enabled yet, so the host is at `E_OFF` and has
    // touched nothing: the device is up with its pull-up on and the host can
    // see it, and that is all that has happened.
    assert_eq!(both.host.port("stage"), E_OFF);
    assert_eq!(both.host.port("line_state"), 0b01, "the device's pull-up");
    both.host.enable();

    let at = both.until_stage(E_UP, 400_000);
    assert_eq!(
        at,
        E_UP,
        "the host stopped at stage {at} with fail_stage {} and fail_status {};\n\
         the host put {:02x?} on the bus\n\
         and the device answered {:02x?}",
        both.host.port("fail_stage"),
        both.host.port("fail_status"),
        both.host.phy.packets,
        both.dev.phy.packets
    );

    // What the host learnt, and what the device agrees it was told.
    assert!(both.host.flag("attached"));
    assert!(!both.host.flag("low_speed"), "it is a full-speed device");
    assert!(!both.host.flag("failed"));
    assert_eq!(both.host.port("dev_addr"), 1, "the address it gave");
    assert_eq!(
        both.host.port("maxpkt0"),
        64,
        "bMaxPacketSize0, out of byte 7 of the descriptor"
    );
    assert_eq!(
        both.dev.address(),
        1,
        "the device took the address the host gave it"
    );
    assert!(
        both.dev.configured(),
        "the device accepted SET_CONFIGURATION"
    );

    // The bytes. These are the same two functions the device's own tests use,
    // so a disagreement is between this host and a reading already checked
    // against a real one.
    let device_descriptor = expected_device_descriptor(0x1209, 0x0001);
    assert_eq!(
        both.host.desc_whole[0],
        Some(device_descriptor.len()),
        "the device descriptor was never whole"
    );
    assert_eq!(
        both.host.desc[0], device_descriptor,
        "the device descriptor the host read"
    );

    let configuration = expected_configuration_descriptor();
    assert_eq!(
        both.host.port("cfg_total"),
        configuration.len() as u64,
        "wTotalLength, out of bytes 2 and 3 of the first nine"
    );
    assert_eq!(
        both.host.desc_whole[1],
        Some(configuration.len()),
        "the configuration descriptor was never whole"
    );
    assert_eq!(
        both.host.desc[1], configuration,
        "the configuration descriptor the host read"
    );
    // `bConfigurationValue` is byte 5 of it and is what `SET_CONFIGURATION`
    // was given — not a constant 1, which is what a host that assumed would
    // have sent.
    assert_eq!(both.host.port("cfg_value"), u64::from(configuration[5]));

    // The bus reset really was one: the device's own link layer saw SE0 held
    // long enough to call it a reset, which is what makes an address of 1
    // mean anything.
    assert!(
        both.dev.phy.reset_cycles > 0 || both.host.phy.packets.len() > 4,
        "something should have happened on this bus"
    );
    // A frame every millisecond, scaled: the host framed the bus while it
    // enumerated, and **the frame's own three bytes are asserted** against
    // this file's packet arithmetic. The frame number increments before the
    // packet goes out, so the first one carries 1, and its CRC5 is over the
    // eleven bits of the number rather than over an address and an endpoint —
    // which is the one place a token's CRC5 covers something else and is
    // worth having a byte comparison of.
    assert!(
        both.host.sofs > 0,
        "the host sent no SOF, so a real device would have suspended"
    );
    let sofs: Vec<&Vec<u8>> = both
        .host
        .phy
        .packets
        .iter()
        .filter(|p| p.first() == Some(&usb_pid(USB_SOF)))
        .collect();
    assert_eq!(
        sofs.len() as u64,
        both.host.sofs,
        "every SOF the block reported should be a packet the transceiver got"
    );
    assert_eq!(*sofs[0], usb_sof(1), "the first frame");
    assert_eq!(*sofs[1], usb_sof(2), "and the next one");

    // The first two packets of the enumeration proper, byte for byte: the
    // SETUP token to address 0 and the eight-byte GET_DESCRIPTOR that USB
    // 2.0 §9.4.3 and Table 9-5 make `80 06 00 01 00 00 08 00`, with the CRC5
    // and the CRC16 this host computed.
    assert_eq!(
        both.host.phy.packets[0],
        usb_token(USB_SETUP, 0, 0),
        "the first thing a host says to a device it has just reset"
    );
    assert_eq!(
        both.host.phy.packets[1],
        usb_data(USB_DATA0, &get_descriptor(0x01, 8)),
        "and the eight bytes of the request"
    );

    let complaints = both.complaints();
    assert!(
        complaints.is_empty(),
        "the bus was misused:\n  {}",
        complaints.join("\n  ")
    );
}

#[test]
fn usb_host_ulpi_enumerates_usb_device_ulpi() {
    enumerate_our_device(false);
}

/// The same enumeration against a device whose **endpoint 0 takes eight
/// bytes a packet**, which is the smallest USB 2.0 §5.5.3 allows.
///
/// This is the one thing the test above cannot do, and it is worth a design
/// of its own: our device declares `bMaxPacketSize0` of 64, so every
/// descriptor it sends fits in one packet and the host's **multi-packet data
/// stage** — three packets for an eighteen-byte descriptor, four for a
/// thirty-two-byte one — is never entered. A data stage of one packet proves
/// nothing about the toggle, which alternates DATA1, DATA0, DATA1 across a
/// stage (§8.5.3) and is the part of a control transfer most likely to be
/// wrong.
///
/// It also reaches the **other** way a data stage ends. With 64-byte packets
/// every read of this device ends on a short packet; with eight, the
/// thirty-two bytes of the configuration descriptor are exactly four full
/// packets and the stage ends because `wLength` has been read, which is the
/// branch a host that only handled short packets would hang in.
#[test]
fn usb_host_ulpi_reads_a_descriptor_in_packets_of_eight() {
    let host_design = host_design();
    let dev_design = design_of(
        "usb_device_ulpi",
        "usb_device_ulpi",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("MAXPKT0", "7'd8"),
        ],
    );
    let mut both = HostDevice::new(&host_design, &dev_design, false);
    both.host.enable();

    let at = both.until_stage(E_UP, 400_000);
    assert_eq!(
        at,
        E_UP,
        "the host stopped at stage {at} with fail_stage {} and fail_status {}",
        both.host.port("fail_stage"),
        both.host.port("fail_status")
    );

    assert_eq!(
        both.host.port("maxpkt0"),
        8,
        "bMaxPacketSize0, which the host read out of byte 7 and then used"
    );
    let mut device_descriptor = expected_device_descriptor(0x1209, 0x0001);
    device_descriptor[7] = 8;
    assert_eq!(
        both.host.desc[0], device_descriptor,
        "eighteen bytes in three packets"
    );
    assert_eq!(
        both.host.desc[1],
        expected_configuration_descriptor(),
        "thirty-two bytes in four, ending because wLength was reached"
    );
    assert_eq!(both.dev.address(), 1);
    assert!(both.dev.configured());

    // **The toggle really did alternate**, and this is how to see it without
    // counting packets: a data stage of more than one packet is the only way
    // a DATA0 packet with a payload comes back from endpoint 0, since a
    // stage starts at DATA1 and a status stage carries nothing. With 64-byte
    // packets there is not one of these.
    let data0_with_payload = both
        .dev
        .phy
        .packets
        .iter()
        .filter(|p| p.first() == Some(&usb_pid(USB_DATA0)) && p.len() > 3)
        .count();
    assert!(
        data0_with_payload >= 3,
        "a multi-packet data stage should have sent several DATA0 packets \
         with payload; {data0_with_payload} did"
    );

    let complaints = both.complaints();
    assert!(
        complaints.is_empty(),
        "the bus was misused:\n  {}",
        complaints.join("\n  ")
    );
}

/// The same, through two transceivers that behave the way the one on the
/// board does: each hears its own transmission, and each reports LineState
/// **late**, one transition at a time out of a backlog that outlives the
/// packet.
///
/// Both of those were measured on a Microchip USB3343 on a Cynthion r1.4 and
/// both are things ULPI 1.1 either permits or forbids and the part does
/// anyway; `UlpiPhy`'s `hears_itself` and `stale_line` have the measurements.
/// The device has enumerated through them; this is the host doing it.
#[test]
fn usb_host_ulpi_enumerates_through_the_transceiver_that_is_on_the_board() {
    enumerate_our_device(true);
}

/// A host whose idea of which line is which is wrong calls a full-speed
/// device low speed, and `FS_LINE` is the parameter that says which.
///
/// This is the failure that cost `ip/usb/usb_device_ulpi` three attempts
/// from the other end, and the reason the parameter exists rather than a
/// constant: a board that exchanges DP and DM between the transceiver and
/// its connector makes `10` the full-speed idle, and a host built for `01`
/// refuses to talk to everything.
#[test]
fn usb_host_ulpi_takes_which_line_is_full_speed_from_its_parameter() {
    let mut params = HOST_TEST_PARAMS.to_vec();
    params.push(("FS_LINE", "2'b10"));
    let design = design_of("usb_host_ulpi", "usb_host_ulpi", &params);
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    host.enable();
    // J, which this build has been told is **not** full speed.
    assert!(
        host.until_stage_alone(UsbLine::J, E_LOWSPEED, 5000),
        "stage {} with FS_LINE = 10 and the pair at J",
        host.port("stage")
    );
    assert!(host.flag("low_speed"));
    assert_eq!(host.phy.packets, Vec::<Vec<u8>>::new());
}

/// A device that goes away between the reset and the first transaction is
/// reported as that and not as a device that will not answer.
///
/// `FAIL_NOT_J` is the one failure `usb_host_enum` has that is not a
/// transaction's: after the bus reset it reads the Debug register and the pair
/// is supposed to be back at J. Here it is not, because the harness drops the
/// pair to SE0 while the reset is being driven and leaves it there.
#[test]
fn usb_host_ulpi_reports_a_device_that_leaves_during_the_reset() {
    let design = host_design();
    let mut host = UlpiHost::new(&design, UlpiPhy::new(ULPI_CPB).hosting());
    host.enable();
    // Long enough to debounce the attachment and start the reset.
    host.run_alone(UsbLine::J, 400);
    assert!(host.flag("attached"));
    // And now there is nothing there at all.
    assert!(
        host.until_stage_alone(UsbLine::Se0, E_FAIL, 20_000),
        "stage {} after the device went away",
        host.port("stage")
    );
    assert_eq!(host.port("fail_status"), FAIL_NOT_J);
    assert!(host.phy.problems.is_empty());
}

/// Nothing in the host crosses a clock boundary: one 60 MHz clock, as ULPI's
/// own rate makes possible, and no PLL anywhere.
#[test]
fn usb_host_ulpi_is_one_clock_domain() {
    let kinds = crossings("usb_host_ulpi", "usb_host_ulpi", &[]);
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

/// What `ip/usb/usb_cdc_acm`'s own designs drive `serial_state` to:
/// `bRxCarrier` and `bTxCarrier` set, every error bit clear.
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
    // subordinate to interface 0. `ip/usb/usb_cdc_acm/README.md` §2
    // says what is known and what is only understood about a host
    // needing it.
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
/// interfaces are one function, and the interrupt IN endpoint because CDC
/// 1.1 §3.2 puts the notification element on one. Both are also understood
/// to be what `cdc_acm` looks for, and `ip/usb/usb_cdc_acm/README.md` §2
/// and §4 say how sure of that this project is — which is less sure than of
/// the bytes.
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
/// specification as the device. `tests/usb_cdc_acm.rs` reads the ten bytes
/// off the wire and `ip/usb/usb_cdc_acm/README.md` §5 has `TIOCMGET` off a
/// real kernel across three opens, which is the other half.
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
    // up. Bits 7..5 of what comes back are its own -- eight data bits and
    // one stop bit -- and bits 4..0 are the host's, which is 9600 and its
    // code 14, because those five bits are where the bit clock comes from
    // and on this board that is not the processor's to decide.
    assert_eq!(
        host.port("acia_control"),
        0x1E,
        "the monitor's framing bits and the host's rate"
    );

    // Now a host opens the port at 115200 — which is what a terminal
    // program does, and which the 65C51's four baud bits **cannot
    // name**. The class layer writes the whole external configuration:
    // code 0 in the rate field *and* bit 4 clear, because the generator
    // selected with a rate field of `0000` is a state the data sheet has
    // no meaning for. The word length and stop bits the monitor wrote are
    // left alone.
    assert_eq!(
        host.control_write_data(9, CDC_SET_LINE_CODING, &cdc_line_coding(115_200, 0, 0, 8)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_LINE_CODING"
    );
    host.idle(100);
    assert_eq!(
        host.port("acia_control"),
        0x00,
        "the whole external configuration: no generator selected and no rate"
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
        "5003\r\r5003: 00\r",
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

// ---------------------------------------------------------------------------
// The crypto console: both blocks of ip/crypto/ behind that same serial port
// ---------------------------------------------------------------------------

/// `testdata/fpga/cynthion/crypto_console_ulpi.v`, when this copy of the
/// crate has the board designs.
///
/// Built here rather than in `tests/usb_crypto_console.rs` because the
/// transceiver model and its harness live in this file, and a model of a
/// part that misbehaves the way the one on the board misbehaves is worth
/// more than a second copy of it.
fn crypto_console_design() -> Option<Design> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/cynthion");
    if !root.join("crypto_console_ulpi.v").is_file() {
        println!("skipping: testdata/fpga/cynthion is not in this copy of the crate");
        return None;
    }

    let mut sources: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    gather("usb_cdc_acm", &mut seen, &mut sources);
    gather("sha256", &mut seen, &mut sources);
    gather("chacha20", &mut seen, &mut sources);
    for name in ["crypto_console.v", "crypto_console_ulpi.v"] {
        let path = format!("testdata/fpga/cynthion/{name}");
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
        "the crypto console does not parse:\n{}",
        diags.render(&map)
    );

    let options = ElabOptions::new(Dialect::Verilog2005)
        .with_top("crypto_console_ulpi")
        // The board writes a Microchip USB3343 vendor register to undo a
        // crossed DP / DM pair, and `UlpiPhy` models ULPI's own registers
        // and not that part's extras — so it answers zero to a read of one
        // and the Link retries for ever. The board's register is the
        // board's; `a_6502_monitor_answers_through_the_transceiver_that_is_on_the_board`
        // does the same for the same reason.
        .with_param("VENDOR_ADDR", "6'h00")
        .with_param("VENDOR_DATA", "8'h00");
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &options, &mut diags);
    assert!(
        !diags.has_errors(),
        "crypto_console_ulpi does not elaborate:\n{}",
        diags.render(&map)
    );
    Some(design.expect("crypto_console_ulpi produced a design"))
}

/// One answer line, read off endpoint `81h` until the CR LF that ends it.
///
/// The console commits a short packet at the end of every answer, so an
/// answer is there as soon as it is finished — but *when* it is finished is
/// a number of clock cycles that depends on the command: 129 a block for
/// SHA-256 and 39 for a ChaCha20 block, and `Z 400` is sixteen blocks. So
/// this polls rather than waiting a fixed time, and the budget is in polls
/// and never in seconds.
///
/// The CR and the LF are stripped, because what the caller wants to compare
/// is the answer and not the line ending — and the line ending is checked
/// here instead, once, which is the better place for it.
fn console_line<P: UsbPair>(host: &mut UsbHost<P>, pipe: &mut BulkPipe, addr: u8) -> String {
    let mut out: Vec<u8> = Vec::new();
    for _ in 0..4000 {
        let packet = pipe.read_or_nothing(host, addr);
        if packet.is_empty() {
            host.idle(140);
        } else {
            out.extend_from_slice(&packet);
            if out.ends_with(b"\r\n") {
                out.truncate(out.len() - 2);
                return String::from_utf8_lossy(&out).into_owned();
            }
        }
    }
    panic!(
        "no answer line came back: {:?}",
        String::from_utf8_lossy(&out)
    );
}

/// A line typed at the console, as a host writes it: bulk packets of at
/// most `wMaxPacketSize`, which is what a write longer than one packet
/// becomes on the wire.
///
/// A `K` line is 67 bytes with its carriage return, so it **is** two
/// packets, and that is worth having rather than avoiding: a parser that
/// lost a byte where one packet ends and the next begins would pass every
/// other assertion in this test.
fn console_type<P: UsbPair>(host: &mut UsbHost<P>, pipe: &mut BulkPipe, addr: u8, line: &str) {
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\r');
    for packet in bytes.chunks(BULK_MAXPKT) {
        pipe.write(host, addr, packet);
    }
}

/// SHA-256 and ChaCha20 answering a host **through the whole USB stack**,
/// in simulation, before any of it is on a part.
///
/// This is `testdata/fpga/cynthion/usb_crypto_console.v` with nothing left
/// out but the pads: `crypto_console`, `ip/crypto/sha256`,
/// `ip/crypto/chacha20`, `ip/usb/usb_cdc_acm`'s class layer,
/// `ip/usb/usb_device_ulpi`'s link layer, and the model of a Microchip
/// USB3343 that **reports LineState late** — the behaviour ULPI 1.1
/// §3.8.1.3 forbids in so many words and the part on this board has anyway.
/// A host enumerates it, reads the banner the console printed, types at it,
/// and checks every answer against a published vector.
///
/// `testdata/fpga/cynthion/crypto_console_tb.v` is the same session with
/// the USB stack taken out of the way, and it is the one to read when an
/// answer is wrong. This one proves there is a **way through**: that the
/// bytes of a command reach the parser across a bulk OUT endpoint and that
/// the digits of an answer come back across a bulk IN one, with the data
/// toggles and the NAKs a real host would see.
///
/// What it would and would not catch. It catches a wrong digest, a wrong
/// keystream, a parser that loses a byte at a packet boundary, and an
/// answer that is never committed — which would show here as the poll loop
/// in `console_line` running out. It catches the console not coming up at
/// all, because the banner is the first assertion.
///
/// It would **not** catch anything about the board that is not in the
/// model: an unrouted wire, a pad that does not drive, a timing path that
/// does not close. `CLAUDE.md` says why that is not optional, and
/// `testdata/fpga/cynthion/usb_crypto_console.v`'s header has the session
/// that was taken on the part instead. It also says nothing at all about
/// what either core **leaks** — `ip/crypto/sha256/README.md` §5 is the list
/// of what is not defended against, and a cycle count in a zero-delay
/// simulator does not touch any of it.
#[test]
fn a_crypto_console_answers_through_the_transceiver_that_is_on_the_board() {
    let Some(design) = crypto_console_design() else {
        return;
    };
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy_data(&design, phy, false), 0);
    host.set_port("serial_state", CDC_LINES_UP, 7);
    configure_for(
        &mut host,
        11,
        &expected_cdc_device_descriptor(),
        &expected_cdc_configuration(),
    );

    let mut pipe = BulkPipe::new(CDC_DATA_ENDP);

    // The banner. The console is held in reset until `configured`, so it
    // prints this into a live endpoint rather than into one a bus reset is
    // about to clear — which is the argument `crypto_console_ulpi.v`'s
    // header makes, and the reason a person who opens the port with `cat`
    // is told what to type.
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "H t|h x|E x|e n|Z n|X n|K x|N x|C x|k|n|c|?",
        "the help line is the console's reset state"
    );

    // FIPS 180-4 Appendix B.1, typed the way a person would type it.
    console_type(&mut host, &mut pipe, 11, "H abc");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "FIPS 180-4 B.1, off the bulk endpoints"
    );

    // The same three bytes as hex. Two forms of the same message must give
    // one digest, and this is the assertion that says the text form is not
    // quietly adding or dropping a byte.
    console_type(&mut host, &mut pipe, 11, "h 616263");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "the hex form of B.1 is the same message"
    );

    // The empty message, which Appendix B does not give and which no other
    // command here can express. `purecrypto` computes this for the empty
    // input and it is published in a great many places;
    // `sha256_matches_the_fips_180_4_examples` has the same constant with
    // the same provenance.
    console_type(&mut host, &mut pipe, 11, "h");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "the empty message"
    );

    // Appendix B.2 is 56 bytes long, which is the padding edge: the length
    // field needs all eight bytes left in the block, so the `1` bit has
    // nowhere to go and the padding spills into a second block. It is also
    // the one line here longer than a single bulk packet going the other
    // way would be, so it crosses the OUT endpoint in one 57-byte packet
    // and the digest crosses the IN endpoint in a 64-byte one and a short
    // one.
    console_type(
        &mut host,
        &mut pipe,
        11,
        "H abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
    );
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        "FIPS 180-4 B.2, the padding edge"
    );

    // Sixty-four zero bytes **made on the device**, which is the other
    // padding case: a whole block of message and then a whole block of
    // nothing but padding. This one does not cross the OUT endpoint at all,
    // which is what makes `Z` a measurement of the core rather than of the
    // link.
    console_type(&mut host, &mut pipe, 11, "Z 40");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "f5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b",
        "sixty-four zero bytes, hashed without any of them crossing USB"
    );

    // The key, the nonce and the counter as they come out of reset, which
    // are RFC 8439 §2.4.2's so that the keystream below needs no key typed
    // first.
    console_type(&mut host, &mut pipe, 11, "k");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        "RFC 8439 2.4.2's key"
    );
    console_type(&mut host, &mut pipe, 11, "n");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "000000000000004a00000000",
        "RFC 8439 2.4.2's nonce"
    );
    console_type(&mut host, &mut pipe, 11, "c");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "00000001",
        "RFC 8439 2.4.2's block counter"
    );

    // And the keystream RFC 8439 §2.4.2 prints beside its ciphertext. Two
    // blocks of it, so the counter advances from 1 to 2 with nothing
    // re-started — which `chacha20_encrypts_the_text_of_rfc_8439_2_4_2`
    // asserts of the block and this asserts of the whole stack.
    console_type(&mut host, &mut pipe, 11, "e 80");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "224f51f3401bd9e12fde276fb8631ded8c131f823d2c06e27e4fcaec9ef3cf78\
         8a3b0aa372600a92b57974cded2b9334794cba40c63e34cdea212c4cf07d41b7\
         69a6749f3f630f4122cafe28ec4dc47e26d4346d70b98c73f3e9c53ac40c5945\
         398b6eda1a832c89c167eacd901d7e2bf363740373201aa188fbbce83991c4ed",
        "RFC 8439 2.4.2's keystream, 128 bytes over two blocks"
    );

    // The same keystream reached the other way, because exclusive-or with
    // zero is the identity: `E` of zero bytes is `e` of that many. A
    // keystream that came out right one way and wrong the other would be
    // the exclusive-or dropping or swapping bytes.
    console_type(&mut host, &mut pipe, 11, "E 0000000000000000");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "224f51f3401bd9e1",
        "enciphering zeros is the keystream"
    );

    // And the digest of sixty-four bytes of that keystream, which is the
    // command that makes a megabyte of cipher output checkable in
    // sixty-four digits. The expectation is what `sha256sum` says of the
    // first sixty-four bytes of the hexdump above, computed on the host and
    // not by anything in this repository.
    console_type(&mut host, &mut pipe, 11, "X 40");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "690c2d96bf4ce23316caa5bb6949a6f694e8759b7de598964c34db4be4a04720",
        "the hash of the cipher's own output, both cores in series"
    );

    // A key, a nonce and a counter typed at it, and then RFC 8439
    // Appendix A.1 vector #1 — the all-zero key at counter zero.
    console_type(
        &mut host,
        &mut pipe,
        11,
        "K 0000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_eq!(console_line(&mut host, &mut pipe, 11), "OK", "the key");
    console_type(&mut host, &mut pipe, 11, "N 000000000000000000000000");
    assert_eq!(console_line(&mut host, &mut pipe, 11), "OK", "the nonce");
    console_type(&mut host, &mut pipe, 11, "C 00000000");
    assert_eq!(console_line(&mut host, &mut pipe, 11), "OK", "the counter");
    console_type(&mut host, &mut pipe, 11, "e 40");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7\
         da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586",
        "RFC 8439 A.1 vector 1"
    );

    // The five ways a line can be wrong, each a different branch of the
    // parser, and then a digest again — because recovering from an error is
    // the half of error handling that is easy to get wrong, and a console
    // that answers `ERR` for ever afterwards would pass every assertion
    // above.
    for bad in ["Q", "h 6", "h 6g", "C 0000", "Z"] {
        console_type(&mut host, &mut pipe, 11, bad);
        assert_eq!(
            console_line(&mut host, &mut pipe, 11),
            "ERR",
            "`{bad}` is not a line this console can answer"
        );
    }
    console_type(&mut host, &mut pipe, 11, "H abc");
    assert_eq!(
        console_line(&mut host, &mut pipe, 11),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "and it still works after five bad lines"
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
// usb_hub: the class layer a host's own hub driver binds to
// ---------------------------------------------------------------------------

/// The interface number, the endpoint address and the one port `usb_hub`
/// states, written here rather than read from it.
const HUB_IFACE: u8 = 0;
const HUB_STATUS_ENDP: u8 = 1;
const HUB_NBR_PORTS: u8 = 1;
/// `wMaxPacketSize` of the status-change endpoint.
///
/// Two, for a bitmap that is one byte: USB 2.0 §11.12.4 makes the bitmap one
/// bit a port plus bit 0 for the hub, rounded up to a byte, and §5.7.3 allows
/// a full-speed interrupt endpoint anything up to 64 — so one would be legal.
/// `usb_bulk_ep` needs a power of two and one of them is **zero bits** of
/// index, which is not a register; the block's own localparam says so at
/// length.
const HUB_STATUS_MAXPKT: u8 = 2;
/// `bInterval` of that endpoint, in frames, which is this block's choice and
/// not a specification's.
const HUB_STATUS_INTERVAL: u8 = 12;

/// Feature selectors, USB 2.0 §11.24.2. The numbering is the
/// specification's and so are the gaps in it.
const FEAT_C_HUB_LOCAL_POWER: u16 = 0;
const FEAT_C_HUB_OVER_CURRENT: u16 = 1;
const FEAT_PORT_CONNECTION: u16 = 0;
const FEAT_PORT_ENABLE: u16 = 1;
const FEAT_PORT_SUSPEND: u16 = 2;
const FEAT_PORT_RESET: u16 = 4;
const FEAT_PORT_POWER: u16 = 8;
const FEAT_C_PORT_CONNECTION: u16 = 16;
const FEAT_C_PORT_ENABLE: u16 = 17;
const FEAT_C_PORT_SUSPEND: u16 = 18;
const FEAT_C_PORT_OVER_CURRENT: u16 = 19;
const FEAT_C_PORT_RESET: u16 = 20;
const FEAT_PORT_TEST: u16 = 21;
const FEAT_PORT_INDICATOR: u16 = 22;

/// `wPortStatus`, USB 2.0 §11.24.2.7.1, and `wPortChange`, §11.24.2.7.2.
const PORT_STAT_CONNECTION: u16 = 1 << 0;
const PORT_STAT_ENABLE: u16 = 1 << 1;
const PORT_STAT_SUSPEND: u16 = 1 << 2;
const PORT_STAT_RESET: u16 = 1 << 4;
const PORT_STAT_POWER: u16 = 1 << 8;
const PORT_STAT_LOW_SPEED: u16 = 1 << 9;
const PORT_CHG_CONNECTION: u16 = 1 << 0;
const PORT_CHG_SUSPEND: u16 = 1 << 2;
const PORT_CHG_RESET: u16 = 1 << 4;

/// The status-change bitmap of a one-port hub with a change on that port:
/// bit 0 is the hub itself and bit 1 is port 1 (USB 2.0 §11.12.4).
///
/// Bit 0 is never set by this block and the reason is in `usb_hub_req`'s port
/// comment: the only two things §11.24.2.6 puts in `wHubChange` are a local
/// power supply a bus-powered hub does not have and an over-current detector
/// this one does not have either.
const HUB_BITMAP_PORT1: u8 = 1 << 1;

/// GetHubDescriptor, USB 2.0 §11.24.2.5: class, device to host, to the
/// device, with the descriptor type in the **high** byte of `wValue` exactly
/// as a standard GET_DESCRIPTOR has it. `29h` is §11.23.2.1's type.
fn hub_get_hub_descriptor(length: u16) -> [u8; 8] {
    let [lo, hi] = length.to_le_bytes();
    [0xA0, 0x06, 0x00, 0x29, 0x00, 0x00, lo, hi]
}

/// GetHubStatus, §11.24.2.6: four bytes, `wHubStatus` then `wHubChange`.
const HUB_GET_HUB_STATUS: [u8; 8] = [0xA0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00];

/// The **standard** GET_STATUS of USB 2.0 §9.4.5, device recipient, which
/// Linux's hub driver sends during hub probe and treats a failure of as fatal.
///
/// `usb_hub_req` claimed it on the class hook for one round, because
/// `usb_ctrl_ep` did not implement it and a hub that stalls it is not a hub as
/// far as that driver is concerned. **It is endpoint 0's now** — a standard
/// request whose two bytes are the same for every device in this library — so
/// this request goes through the hub without the class seeing it at all, and
/// `usb_hub_stalls_the_class_requests_it_does_not_claim` is where the class
/// not shadowing it is asserted.
const HUB_GET_DEVICE_STATUS: [u8; 8] = [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00];

/// GetPortStatus, §11.24.2.7: class, device to host, recipient "other" —
/// which for a hub is a port — with the port number in `wIndex`, from one.
fn hub_get_port_status(port: u8) -> [u8; 8] {
    [0xA3, 0x00, 0x00, 0x00, port, 0x00, 0x04, 0x00]
}

/// SetPortFeature, §11.24.2.12, and ClearPortFeature, §11.24.2.2: the
/// feature selector in `wValue`, the port in `wIndex`, no data stage.
fn hub_set_port_feature(port: u8, feature: u16) -> [u8; 8] {
    let [lo, hi] = feature.to_le_bytes();
    [0x23, 0x03, lo, hi, port, 0x00, 0x00, 0x00]
}

fn hub_clear_port_feature(port: u8, feature: u16) -> [u8; 8] {
    let [lo, hi] = feature.to_le_bytes();
    [0x23, 0x01, lo, hi, port, 0x00, 0x00, 0x00]
}

/// ClearHubFeature, §11.24.2.1: the same with the device as the recipient.
fn hub_clear_hub_feature(feature: u16) -> [u8; 8] {
    let [lo, hi] = feature.to_le_bytes();
    [0x20, 0x01, lo, hi, 0x00, 0x00, 0x00, 0x00]
}

/// The nine bytes of the hub descriptor, written **forwards** from USB 2.0
/// §11.23.2.1 rather than from the block's `case`.
///
/// `bDescLength` is written as **the arithmetic and not the answer**, because
/// that is the field this descriptor gets wrong if anything does: the two
/// variable-length fields at the end are one bit a port plus one for the
/// reserved bit 0, rounded up to a byte, and a hub whose length disagrees
/// with a host's reading of it is not a hub. Linux requires at least
/// `7 + 2` bytes back and asks for the whole of its own `struct
/// usb_hub_descriptor`, which is sized for its maximum port count, so the short
/// data stage is what makes nine the answer.
fn expected_hub_descriptor() -> Vec<u8> {
    // DeviceRemovable and PortPwrCtrlMask, one bit a port plus the reserved
    // bit 0, **rounded up to a byte** — which is what `div_ceil` is and is the
    // arithmetic `bDescLength` has to agree with.
    let mask_bytes = (usize::from(HUB_NBR_PORTS) + 1).div_ceil(8);
    let mut d = vec![
        // bDescLength: seven fixed bytes and the two masks.
        u8::try_from(7 + 2 * mask_bytes).expect("a short descriptor"),
        // bDescriptorType: 29h, the hub descriptor's own.
        0x29,
        // bNbrPorts.
        HUB_NBR_PORTS,
    ];
    // wHubCharacteristics, little endian:
    //
    //   D1:D0  Logical Power Switching Mode   01 — individual port power
    //   D2     part of a compound device       0
    //   D4:D3  Over-current Protection Mode   10 — none at all
    //   D6:D5  TT Think Time                  00 — no transaction translator
    //   D7     Port Indicators Supported       0
    //
    // Individual power switching rather than none because Linux's
    // `hub_power_on` sends SetPortFeature(PORT_POWER) only to a hub whose
    // mode is 00 or 01; "no over-current protection" because that is the
    // truth and §11.23.2.1 allows it for a bus-powered hub that implements
    // none.
    let characteristics: u16 = 0b0001_0001;
    d.extend_from_slice(&characteristics.to_le_bytes());
    // bPwrOn2PwrGood, in 2 ms units: 100 ms.
    d.push(50);
    // bHubContrCurrent, in mA: the same 100 the configuration's bMaxPower
    // declares.
    d.push(100);
    // DeviceRemovable: bit 0 reserved, bit n port n, 0 removable. Whatever is
    // in the downstream socket can be unplugged.
    d.extend(std::iter::repeat_n(0x00u8, mask_bytes));
    // PortPwrCtrlMask: "All bits in this field should be set to 1B", for
    // compatibility with software written for 1.0 hubs.
    d.extend(std::iter::repeat_n(0xFFu8, mask_bytes));
    d
}

/// The configuration descriptor `usb_hub` should send, written **forwards**
/// from USB 2.0 §11.23.1 and not from the block's `IFACE_DESC` parameter.
///
/// Twenty-five bytes, which is the smallest configuration descriptor anything
/// in this library has: a hub's class-specific descriptor is fetched by a
/// request of its own, so there is nothing between the interface and its one
/// endpoint. The derived fields are the arithmetic and not the answer, as
/// `expected_cdc_configuration` next door makes them.
fn expected_hub_configuration() -> Vec<u8> {
    let mut iface: Vec<u8> = Vec::new();
    // INTERFACE 0: bInterfaceClass 09h, bInterfaceSubClass 00h,
    // bInterfaceProtocol 00h — the full-speed hub of §11.23.1, which has one
    // alternate setting where a high-speed one has two. bNumEndpoints is
    // byte 4 and is counted below.
    iface.extend_from_slice(&[9, 4, HUB_IFACE, 0, 0, 0x09, 0x00, 0x00, 0]);
    // ENDPOINT 81h: the status change endpoint. bmAttributes 03h is interrupt
    // (USB 2.0 Table 9-13 bits 1:0).
    iface.extend_from_slice(&[
        7,
        5,
        0x80 | HUB_STATUS_ENDP,
        0x03,
        HUB_STATUS_MAXPKT,
        0,
        HUB_STATUS_INTERVAL,
    ]);
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

/// The device descriptor a hub sends: `09h 00h 00h`, which §11.23.1 asks for
/// and whose last byte is the whole statement that this is a **full-speed**
/// hub — 1 and 2 are a high-speed hub with one transaction translator and
/// with one per port.
fn expected_hub_device_descriptor() -> Vec<u8> {
    expected_device_descriptor_of(0x1209, 0x0001, [0x09, 0x00, 0x00])
}

fn hub_fs_design() -> Design {
    design_of(
        "usb_hub",
        "usb_hub_fs",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

fn hub_ulpi_design() -> Design {
    design_of(
        "usb_hub",
        "usb_hub_ulpi",
        &[("VID", "16'h1209"), ("PID", "16'h0001")],
    )
}

/// `wPortStatus` and `wPortChange` out of the four bytes GetPortStatus sends.
fn port_words(data: &[u8]) -> (u16, u16) {
    assert_eq!(data.len(), 4, "GetPortStatus is two 16-bit fields");
    (
        u16::from_le_bytes([data[0], data[1]]),
        u16::from_le_bytes([data[2], data[3]]),
    )
}

/// The status-change endpoint as a host polls it: a data toggle and nothing
/// else.
///
/// `BulkPipe` next door repeats a NAKed transaction until it succeeds, which
/// is what a host controller does with a transfer it has been handed. A hub's
/// status-change endpoint is the opposite case — a NAK is the **answer** most
/// of the time, and which polls get one is what the assertions below are
/// about — so this returns the NAK instead of retrying through it.
struct StatusPipe {
    toggle: u8,
}

impl StatusPipe {
    fn new() -> StatusPipe {
        StatusPipe { toggle: USB_DATA0 }
    }

    /// Back to DATA0, which is what SET_CONFIGURATION does to every
    /// endpoint's toggle (USB 2.0 §9.4.5).
    fn reconfigured(&mut self) {
        self.toggle = USB_DATA0;
    }

    /// One poll: the bitmap, or `None` for the NAK of a hub with nothing to
    /// say.
    fn poll<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8) -> Option<u8> {
        match host.bulk_in(addr, HUB_STATUS_ENDP) {
            UsbReply::Data(pid, payload) => {
                assert_eq!(pid, self.toggle, "the status endpoint's data toggle");
                assert_eq!(
                    payload.len(),
                    1,
                    "a one-port hub's bitmap is one byte (§11.12.4)"
                );
                host.ack();
                self.toggle = other_toggle(self.toggle);
                host.idle(6);
                Some(payload[0])
            }
            UsbReply::Handshake(USB_NAK) => {
                host.idle(6);
                None
            }
            other => panic!("a poll of the status-change endpoint was answered {other:?}"),
        }
    }

    /// The one report that follows a host clearing a change, and then quiet.
    ///
    /// `usb_bulk_ep` is a **store-and-forward packet buffer** and a hub's
    /// status-change endpoint is a register a host reads, so the two do not
    /// quite fit: the bitmap armed while the change bit was still set goes out
    /// even though the host has since cleared it. Exactly one, and
    /// deterministically one — the host acknowledged the previous packet, the
    /// bit was still set, so a packet armed; then the ClearPortFeature landed;
    /// then that packet went. A host does one GetPortStatus over it, finds
    /// nothing changed, and carries on.
    ///
    /// **Asserting it rather than tolerating it** is the point. It is a
    /// property of the endpoint below the hub, not of the hub, and anything
    /// that changed it — an endpoint that can be disarmed, an arming rule
    /// with a latch in it — should fail here and be read.
    /// `ip/usb/usb_hub/README.md` §5 is the same thing in prose with what
    /// the alternative would cost.
    fn settles<P: UsbPair>(
        &mut self,
        host: &mut UsbHost<P>,
        addr: u8,
        want: u8,
        count: usize,
        why: &str,
    ) {
        assert_eq!(
            self.poll(host, addr),
            Some(want),
            "the bitmap already armed when the host cleared the change: {why}"
        );
        self.quiet(host, addr, count, why);
    }

    /// `count` polls that must every one be NAKed.
    fn quiet<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8, count: usize, why: &str) {
        for poll in 0..count {
            assert_eq!(self.poll(host, addr), None, "poll {poll} of {count}: {why}");
        }
    }

    /// One poll that must return the bitmap given.
    fn expect<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8, want: u8, why: &str) {
        assert_eq!(self.poll(host, addr), Some(want), "{why}");
    }
}

/// Nothing on the downstream port, which is what every hub test starts from.
///
/// Both are **inputs**, and an input nobody drives is unknown — the same
/// reason every test of `ip/usb/usb_cdc_acm` sets `serial_state`.
fn hub_port_empty<P: UsbPair>(host: &mut UsbHost<P>) {
    host.set_port("port_attached", 0, 1);
    host.set_port("port_low_speed", 0, 1);
    // THE RESET THAT TAKES NO TIME, WHICH IS WHAT THIS BLOCK ON ITS OWN HAS
    //
    // `ip/usb/usb_hub`'s port reset is a handshake: a level out while the
    // port is resetting and a pulse in when whatever drives it has
    // finished. Nothing downstream of the block **in these tests** drives
    // one, so `port_reset_done` is held high and every reset finishes in
    // the cycle it is asked for, which is exactly what this block did
    // before the handshake existed and is what
    // `testdata/fpga/cynthion/usb_hub_target.v` ties it to.
    //
    // It is driven here and not left alone because an undriven input is
    // `x`, and an `x` into `resetting && port_reset_done` is a port that
    // neither finishes its reset nor says so. `ip/usb/usb_proxy`'s own
    // tests are where the handshake takes time.
    host.set_port("port_reset_done", 1, 1);
}

/// The whole enumeration a host does, against the descriptor set a hub
/// declares.
fn hub_enumerate<P: UsbPair>(host: &mut UsbHost<P>) {
    hub_port_empty(host);
    enumerate_descriptors(
        host,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );
}

#[test]
fn usb_hub_enumerates_as_a_hub() {
    let design = hub_fs_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    hub_enumerate(&mut host);
}

#[test]
fn usb_hub_ulpi_enumerates_as_a_hub() {
    let design = hub_ulpi_design();
    let mut host = UsbHost::new(UlpiPair::new(&design), 0);
    hub_enumerate(&mut host);
}

/// The same through the transceiver that is **on the board**: the one that
/// reports LineState a clock late, against ULPI §3.8.1.3.
#[test]
fn usb_hub_ulpi_enumerates_through_the_transceiver_that_is_on_the_board() {
    let design = hub_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    hub_enumerate(&mut host);
}

/// The descriptor set has the parts a host's hub driver looks for, said in
/// the terms the driver looks for them in.
///
/// `usb_hub_enumerates_as_a_hub` already compares all twenty-five bytes, so
/// this adds nothing about the bytes. What it adds is **why those bytes**:
/// Linux's `hub_probe` refuses an interface whose `bInterfaceSubClass` is
/// neither 0 nor 1, refuses one with any number of endpoints but exactly one,
/// and refuses one whose single endpoint is not an interrupt IN. Those three
/// are asserted here one at a time, so that a reader changing the descriptors
/// sees which properties are load-bearing rather than a byte string that must
/// not move.
///
/// **What it would not catch.** Whether the driver binds. Nothing in
/// simulation can, because this expectation and the block are both written
/// from the same specification; §6 of `ip/usb/usb_hub/README.md` has the kernel
/// log.
#[test]
fn usb_hub_descriptors_carry_what_a_host_hub_driver_binds_on() {
    let device = expected_hub_device_descriptor();
    assert_eq!(
        device[4], 0x09,
        "bDeviceClass 09h is what the driver matches"
    );
    assert_eq!(device[5], 0x00, "bDeviceSubClass");
    assert_eq!(
        device[6], 0x00,
        "bDeviceProtocol 00h: a full-speed hub, with no transaction translator"
    );

    let config = expected_hub_configuration();
    assert_eq!(config.len(), 25, "nine bytes of configuration plus sixteen");
    assert_eq!(
        u16::from_le_bytes([config[2], config[3]]),
        25,
        "wTotalLength is the sum and not a number typed twice"
    );
    assert_eq!(config[4], 1, "one interface");

    // Walk it the way a host does.
    let mut at = 9;
    let mut interfaces: Vec<(u8, u8, u8, u8, u8)> = Vec::new();
    let mut endpoints: Vec<(u8, u8, u16, u8)> = Vec::new();
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
                config[at + 7],
            )),
            5 => endpoints.push((
                config[at + 2],
                config[at + 3] & 0x03,
                u16::from_le_bytes([config[at + 4], config[at + 5]]),
                config[at + 6],
            )),
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

    // There is **no** class-specific descriptor in here. A hub's one
    // class-specific descriptor is fetched by GetHubDescriptor, §11.23.2.1,
    // and a hub that put it in the configuration would be describing itself
    // twice.
    assert_eq!(
        interfaces,
        vec![(HUB_IFACE, 1, 0x09, 0x00, 0x00)],
        "(bInterfaceNumber, bNumEndpoints, bInterfaceClass, bInterfaceSubClass, \
         bInterfaceProtocol)"
    );
    assert!(
        matches!(interfaces[0].3, 0 | 1),
        "bInterfaceSubClass must be 0 or 1 or Linux's hub_probe refuses the interface"
    );
    assert_eq!(
        interfaces[0].1, 1,
        "exactly one endpoint, which hub_probe also insists on"
    );

    assert_eq!(
        endpoints,
        vec![(
            0x80 | HUB_STATUS_ENDP,
            0x03,
            u16::from(HUB_STATUS_MAXPKT),
            HUB_STATUS_INTERVAL
        )],
        "(bEndpointAddress, transfer type, wMaxPacketSize, bInterval)"
    );
    assert_eq!(
        endpoints[0].0 & 0x80,
        0x80,
        "an interrupt **IN** endpoint, which is the third thing hub_probe checks"
    );
    assert!(
        endpoints[0].2 >= 1,
        "the bitmap of a {HUB_NBR_PORTS}-port hub is one byte and the endpoint holds it"
    );
    assert!(
        endpoints[0].2 <= 64,
        "USB 2.0 §5.7.3 allows a full-speed interrupt endpoint 64 bytes at most"
    );

    // And the hub descriptor's own length, which is the field a host and a
    // hub can disagree about.
    let hub = expected_hub_descriptor();
    assert_eq!(hub.len(), 9, "a one-port hub descriptor is nine bytes");
    assert_eq!(
        usize::from(hub[0]),
        hub.len(),
        "bDescLength is the length of what is sent"
    );
    assert_eq!(hub[1], 0x29, "bDescriptorType, §11.23.2.1");
    assert_eq!(hub[2], HUB_NBR_PORTS, "bNbrPorts");
    assert!(
        hub[3] & 0x03 < 2,
        "wHubCharacteristics D1:D0 below 2, or a host never powers the port"
    );
}

/// The hub and port class requests, answered, and the port state they move.
///
/// This is the hook working end to end against a class that needs more of it
/// than the first one did: six class requests, one **standard** request
/// endpoint 0 does not implement, a nine-byte data stage and a port whose
/// reported state is a function of five registers and two inputs.
fn hub_class_requests<P: UsbPair>(host: &mut UsbHost<P>) {
    hub_port_empty(host);
    configure_for(
        host,
        3,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );

    // ------------------------------------------------------------------
    // The descriptors a request of their own fetches.
    // ------------------------------------------------------------------
    // Fifteen bytes asked for, which is what Linux asks for — the whole of
    // its own `struct usb_hub_descriptor` — and nine returned, which is a
    // short data stage and is what ends the transfer.
    let desc = host
        .control_read(3, hub_get_hub_descriptor(15))
        .expect("GetHubDescriptor is answered and not stalled");
    assert_eq!(
        desc,
        expected_hub_descriptor(),
        "the hub descriptor, byte for byte"
    );
    // ... and a host that asks for fewer bytes than there are gets what it
    // asked for, which is `wLength` ending a class data stage.
    let seven = host
        .control_read(3, hub_get_hub_descriptor(7))
        .expect("a short GetHubDescriptor");
    assert_eq!(
        seven,
        expected_hub_descriptor()[..7],
        "wLength ends a class data stage too"
    );

    // GetHubStatus: four zeros. This hub has no local power supply and no
    // over-current detector, so neither the status nor the change can ever be
    // anything else (§11.24.2.6), and `usb_hub_req` has no register for them.
    assert_eq!(
        host.control_read(3, HUB_GET_HUB_STATUS)
            .expect("GetHubStatus"),
        vec![0, 0, 0, 0],
        "wHubStatus and wHubChange"
    );

    // The standard device status, which is the request Linux's hub driver
    // sends during probe. It is **endpoint 0's** now rather than this
    // class's, and the two bytes are the same either way: bit 0 Self
    // Powered, out of bit 6 of `CFG_ATTR`, and bit 1 Remote Wakeup Enabled,
    // which nothing here can set. That it still answers is the whole of
    // what moving it had to not break.
    assert_eq!(
        host.control_read(3, HUB_GET_DEVICE_STATUS)
            .expect("the standard GET_STATUS a hub driver sends"),
        vec![0, 0],
        "wStatus: bus powered, remote wake-up not enabled"
    );

    // ------------------------------------------------------------------
    // The port, before and after a host powers it.
    // ------------------------------------------------------------------
    let status = |host: &mut UsbHost<P>| {
        port_words(
            &host
                .control_read(3, hub_get_port_status(HUB_NBR_PORTS))
                .expect("GetPortStatus"),
        )
    };

    assert_eq!(
        status(host),
        (0, 0),
        "a hub the host has only just configured has a powered-off, empty port"
    );

    // A device plugged in while the port is powered off is **not** a
    // connection: §11.5.1.1 makes a port's connection status meaningless
    // then, and `usb_hub_req` gates it so that the first connection is
    // reportable at all.
    host.set_port("port_attached", 1, 1);
    host.idle(20);
    assert_eq!(
        status(host),
        (0, 0),
        "a powered-off port reports nothing on it and no change"
    );

    // Powering it is what makes the connection appear — and a connection
    // change with it, which is the whole reason a host hears about a device
    // that was already plugged in before it ever looked.
    assert_eq!(
        host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_POWER)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SetPortFeature(PORT_POWER) is acknowledged"
    );
    host.idle(20);
    assert_eq!(
        host.port("port_power"),
        1,
        "and the same on the block's port"
    );
    assert_eq!(
        status(host),
        (PORT_STAT_POWER | PORT_STAT_CONNECTION, PORT_CHG_CONNECTION),
        "powered, something on it, and a connection change to say so"
    );

    // The host clears the change. Nothing else moves.
    assert_eq!(
        host.control_write(
            3,
            hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_CONNECTION)
        ),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "ClearPortFeature(C_PORT_CONNECTION) is acknowledged"
    );
    host.idle(20);
    assert_eq!(
        status(host),
        (PORT_STAT_POWER | PORT_STAT_CONNECTION, 0),
        "the change is cleared and the connection is not"
    );

    // Which line the device pulled up. Only meaningful while something is
    // connected, which is what the specification calls the speed of the
    // **attached** device.
    host.set_port("port_low_speed", 1, 1);
    host.idle(20);
    assert_eq!(
        status(host),
        (
            PORT_STAT_POWER | PORT_STAT_CONNECTION | PORT_STAT_LOW_SPEED,
            0
        ),
        "a low-speed device pulled D- up"
    );
    host.set_port("port_low_speed", 0, 1);
    host.idle(20);

    // ------------------------------------------------------------------
    // The reset, which takes no time because nothing is driven downstream.
    // ------------------------------------------------------------------
    assert_eq!(
        host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_RESET)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SetPortFeature(PORT_RESET) is acknowledged"
    );
    host.idle(20);
    let (stat, chg) = status(host);
    assert_eq!(
        stat,
        PORT_STAT_POWER | PORT_STAT_CONNECTION | PORT_STAT_ENABLE,
        "the port is enabled by the reset completing"
    );
    assert_eq!(stat & PORT_STAT_RESET, 0, "and the reset is already over");
    assert_eq!(chg, PORT_CHG_RESET, "C_PORT_RESET says it completed");
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_RESET));
    host.idle(20);
    assert_eq!(status(host).1, 0, "and the change clears");

    // ------------------------------------------------------------------
    // Suspend, and the resume that follows it.
    // ------------------------------------------------------------------
    host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_SUSPEND));
    host.idle(20);
    assert_eq!(
        status(host).0 & PORT_STAT_SUSPEND,
        PORT_STAT_SUSPEND,
        "the port is suspended"
    );
    assert_eq!(host.port("port_suspended"), 1);
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_PORT_SUSPEND));
    host.idle(20);
    let (stat, chg) = status(host);
    assert_eq!(stat & PORT_STAT_SUSPEND, 0, "and resumed");
    assert_eq!(
        chg, PORT_CHG_SUSPEND,
        "C_PORT_SUSPEND is the resume having completed (§11.5.1.8)"
    );
    host.control_write(
        3,
        hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_SUSPEND),
    );
    host.idle(20);

    // ------------------------------------------------------------------
    // The features claimed and ignored, and the port disabled by hand.
    // ------------------------------------------------------------------
    // Two change bits nothing in this hub can set: claimed so that a host
    // clearing them is answered rather than stalled, and a no-op because the
    // bit was already zero.
    for (what, feature) in [
        ("C_PORT_ENABLE", FEAT_C_PORT_ENABLE),
        ("C_PORT_OVER_CURRENT", FEAT_C_PORT_OVER_CURRENT),
    ] {
        assert_eq!(
            host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, feature)),
            UsbReply::Data(USB_DATA1, Vec::new()),
            "ClearPortFeature({what}) is accepted even though the bit is a constant zero"
        );
        host.idle(10);
    }
    for (what, feature) in [
        ("C_HUB_LOCAL_POWER", FEAT_C_HUB_LOCAL_POWER),
        ("C_HUB_OVER_CURRENT", FEAT_C_HUB_OVER_CURRENT),
    ] {
        assert_eq!(
            host.control_write(3, hub_clear_hub_feature(feature)),
            UsbReply::Data(USB_DATA1, Vec::new()),
            "ClearHubFeature({what}) is accepted"
        );
        host.idle(10);
    }
    assert_eq!(
        status(host),
        (PORT_STAT_POWER | PORT_STAT_CONNECTION | PORT_STAT_ENABLE, 0),
        "and none of those four changed anything"
    );

    // ClearPortFeature(PORT_ENABLE), which is a host taking the port out of
    // service without powering it off.
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_PORT_ENABLE));
    host.idle(20);
    assert_eq!(
        status(host),
        (PORT_STAT_POWER | PORT_STAT_CONNECTION, 0),
        "disabled, still powered, still connected"
    );
    assert_eq!(host.port("port_enabled"), 0);

    // ------------------------------------------------------------------
    // Unplugging, and powering the port off.
    // ------------------------------------------------------------------
    host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_RESET));
    host.idle(20);
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_RESET));
    host.idle(20);
    assert_eq!(
        status(host).0 & PORT_STAT_ENABLE,
        PORT_STAT_ENABLE,
        "enabled again"
    );
    host.set_port("port_attached", 0, 1);
    host.idle(20);
    assert_eq!(
        status(host),
        (PORT_STAT_POWER, PORT_CHG_CONNECTION),
        "the device left: no connection, no enable — §11.5.1.4 — and a change"
    );
    host.control_write(
        3,
        hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_CONNECTION),
    );
    host.idle(20);

    // And the port powered off reports nothing at all.
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_PORT_POWER));
    host.idle(20);
    assert_eq!(status(host), (0, 0), "a powered-off port");
    assert_eq!(host.port("port_power"), 0);

    host.assert_clean();
}

#[test]
fn usb_hub_answers_the_hub_and_port_class_requests() {
    let design = hub_fs_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    hub_class_requests(&mut host);
}

#[test]
fn usb_hub_ulpi_answers_the_hub_and_port_class_requests() {
    let design = hub_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    hub_class_requests(&mut host);
}

/// A class request the hub does not claim is stalled, and a standard request
/// is still endpoint 0's.
///
/// Both halves matter, and the first is what keeps a hub honest about what it
/// is: `wHubCharacteristics` says no port indicators, so
/// SetPortFeature(PORT_INDICATOR) is stalled rather than silently accepted;
/// there is one port, so a request about port 2 is stalled rather than
/// answered about port 1; and the optional requests of a high-speed hub are
/// stalled because there is no transaction translator to ask about.
#[test]
fn usb_hub_stalls_the_class_requests_it_does_not_claim() {
    let design = hub_fs_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    hub_port_empty(&mut host);
    configure_for(
        &mut host,
        4,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );

    // Power the port and connect something, so that a stalled request has
    // state it could have disturbed.
    host.set_port("port_attached", 1, 1);
    host.control_write(4, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_POWER));
    host.idle(20);
    host.control_write(
        4,
        hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_CONNECTION),
    );
    host.idle(20);
    let before = port_words(
        &host
            .control_read(4, hub_get_port_status(HUB_NBR_PORTS))
            .expect("GetPortStatus"),
    );
    assert_eq!(before, (PORT_STAT_POWER | PORT_STAT_CONNECTION, 0));

    for (what, request) in [
        // SetPortFeature(PORT_TEST), §11.24.2.12. There are no test modes.
        (
            "SetPortFeature(PORT_TEST)",
            hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_TEST),
        ),
        // SetPortFeature(PORT_INDICATOR). D7 of wHubCharacteristics is clear,
        // so this hub never offered indicators.
        (
            "SetPortFeature(PORT_INDICATOR)",
            hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_INDICATOR),
        ),
        // PORT_CONNECTION is a status bit and not a feature a host may set.
        (
            "SetPortFeature(PORT_CONNECTION)",
            hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_CONNECTION),
        ),
        // SetPortFeature(PORT_ENABLE): §11.24.2.12 gives a host no way to
        // enable a port but by resetting it.
        (
            "SetPortFeature(PORT_ENABLE)",
            hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_ENABLE),
        ),
        // A port this hub does not have.
        (
            "SetPortFeature on port 2",
            hub_set_port_feature(2, FEAT_PORT_POWER),
        ),
        ("GetPortStatus of port 2", hub_get_port_status(2)),
        ("GetPortStatus of port 0", hub_get_port_status(0)),
        // SetHubFeature, §11.24.2.11 — a hub need not implement it.
        (
            "SetHubFeature",
            [0x20, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        // SetHubDescriptor, §11.24.2.10 — likewise.
        (
            "SetHubDescriptor",
            [0x20, 0x07, 0x00, 0x29, 0x00, 0x00, 0x09, 0x00],
        ),
        // ClearTTBuffer, §11.24.2.3, and the rest of the transaction
        // translator's requests, which a full-speed hub has none of.
        (
            "ClearTTBuffer",
            [0x23, 0x08, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
        ),
        ("ResetTT", [0x23, 0x09, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]),
        (
            "GetTTState",
            [0xA3, 0x0A, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00],
        ),
        ("StopTT", [0x23, 0x0B, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]),
        // GetBusState, §11.24.2.4, which is optional and for debugging.
        (
            "GetBusState",
            [0xA3, 0x02, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00],
        ),
        // A descriptor type that is not the hub's.
        (
            "GetHubDescriptor of type 2Ah",
            [0xA0, 0x06, 0x00, 0x2A, 0x00, 0x00, 0x0C, 0x00],
        ),
        // A feature request that arrived with a data stage, which no feature
        // request has.
        (
            "SetPortFeature with a payload",
            [0x23, 0x03, 0x08, 0x00, 0x01, 0x00, 0x02, 0x00],
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

    // None of that reached a register.
    let after = port_words(
        &host
            .control_read(4, hub_get_port_status(HUB_NBR_PORTS))
            .expect("GetPortStatus still works"),
    );
    assert_eq!(after, before, "a stalled request changed nothing");

    // And the standard requests still belong to endpoint 0 with a class on the
    // hook — including GET_STATUS, which this class claimed for one round and
    // which `usb_ctrl_ep` implements now, and SET_ADDRESS, which shares the
    // first byte of its `bmRequestType` with nothing but a direction bit.
    assert_eq!(
        host.control_write(4, set_address(11)),
        UsbReply::Data(USB_DATA1, Vec::new()),
        "SET_ADDRESS is endpoint 0's and not the class's"
    );
    assert_eq!(host.address(), 11);
    host.idle(10);
    assert_eq!(
        host.control_read(11, get_descriptor(1, 18))
            .expect("GET_DESCRIPTOR is endpoint 0's too"),
        expected_hub_device_descriptor()
    );
    host.idle(10);
    host.assert_clean();
}

/// The status-change endpoint reports **every** change and NAKs in between,
/// and the second report is what this test exists for.
///
/// Six things:
///
///   * a configured hub with a powered-off, empty port says nothing, twenty
///     polls in a row;
///   * a device appearing while the port is **off** is still nothing, because
///     §11.5.1.1 makes a powered-off port's connection meaningless;
///   * the host powering the port produces the first report, which is how a
///     device plugged in before the host ever looked is noticed;
///   * **the same bitmap again, because the host has not cleared it** — the
///     endpoint reports sticky state and holds no record of having reported;
///   * the host clearing the change quiets it; and then
///   * **a second change — the device leaving — is reported too**, and then a
///     third of a different kind, a reset completing.
///
/// One more thing is asserted along the way and it is not the hub's: a bitmap
/// armed before the host's ClearPortFeature arrived goes out anyway, because
/// `usb_bulk_ep` cannot unsay a packet it has been given. `StatusPipe::settles`
/// is where that is written down.
///
/// **Why the fourth and sixth are the assertions that matter.** The first
/// version of `ip/usb/usb_cdc_acm`'s notification endpoint sent one
/// notification per configuration and never another, which is a functional
/// defect: the host that consumed it need not be the host that acts on it,
/// and a driver bound a second time started with nothing. §4 of that
/// block's README.md has the measurement that condemned it. The defect
/// survives any test that only asks "is anything ever reported", so this
/// one asks for a report **after** the state has gone back and come again,
/// and for the same report twice with the state deliberately unchanged, so
/// that nothing but the absence of a one-shot can produce them.
///
/// **What it would not catch.** Whether a host's hub driver acts on the
/// bitmap, or whether twelve frames is often enough for one. Nothing in
/// simulation can: this host model is written from the same specification as
/// the device.
fn hub_status_changes<P: UsbPair>(host: &mut UsbHost<P>) {
    let port = HUB_NBR_PORTS;
    hub_port_empty(host);
    configure_for(
        host,
        6,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );
    let mut pipe = StatusPipe::new();

    // A hub whose port is powered off and empty has nothing to say, and says
    // it for as long as it is asked.
    pipe.quiet(
        host,
        6,
        20,
        "a configured hub with a powered-off, empty port",
    );

    // A device appears while the port is off. Still nothing.
    host.set_port("port_attached", 1, 1);
    host.idle(20);
    pipe.quiet(
        host,
        6,
        4,
        "a powered-off port has no connection, so nothing changed",
    );

    // The host powers the port, and **that** is when the connection appears.
    host.control_write(6, hub_set_port_feature(port, FEAT_PORT_POWER));
    host.idle(20);
    pipe.expect(
        host,
        6,
        HUB_BITMAP_PORT1,
        "the port the host has just powered has something on it",
    );

    // THE SAME BITMAP AGAIN, because the host has not cleared the change.
    //
    // This is the property that makes a one-shot impossible: there is no
    // latch anywhere saying the host has been told. `usb_hub`'s header says
    // what the extra packet costs and why the alternative is refused.
    pipe.expect(
        host,
        6,
        HUB_BITMAP_PORT1,
        "a change the host has not cleared is still a change",
    );

    // The host clears it, and the endpoint goes quiet.
    host.control_write(6, hub_clear_port_feature(port, FEAT_C_PORT_CONNECTION));
    host.idle(20);
    pipe.settles(
        host,
        6,
        HUB_BITMAP_PORT1,
        20,
        "the host has cleared the only change there was",
    );

    // ------------------------------------------------------------------
    // THE SECOND CHANGE
    // ------------------------------------------------------------------
    // The device leaves. A hub that reported once and then held a latch would
    // be silent here for ever, and the host would go on believing a device is
    // on the port.
    host.set_port("port_attached", 0, 1);
    host.idle(20);
    pipe.expect(host, 6, HUB_BITMAP_PORT1, "the device left, and it is news");
    host.control_write(6, hub_clear_port_feature(port, FEAT_C_PORT_CONNECTION));
    host.idle(20);
    pipe.settles(host, 6, HUB_BITMAP_PORT1, 4, "and quiet again");

    // ------------------------------------------------------------------
    // A THIRD, of a different kind: a reset completing.
    // ------------------------------------------------------------------
    host.set_port("port_attached", 1, 1);
    host.idle(20);
    pipe.expect(host, 6, HUB_BITMAP_PORT1, "plugged in again");
    host.control_write(6, hub_clear_port_feature(port, FEAT_C_PORT_CONNECTION));
    host.idle(20);
    pipe.settles(host, 6, HUB_BITMAP_PORT1, 4, "and cleared");

    host.control_write(6, hub_set_port_feature(port, FEAT_PORT_RESET));
    host.idle(20);
    pipe.expect(
        host,
        6,
        HUB_BITMAP_PORT1,
        "a reset completing is a change of the same port",
    );
    host.control_write(6, hub_clear_port_feature(port, FEAT_C_PORT_RESET));
    host.idle(20);
    pipe.settles(host, 6, HUB_BITMAP_PORT1, 4, "and cleared");

    // The endpoint has one direction. An OUT to it is nobody's: the status
    // endpoint because it has no OUT side, endpoint 0 because the token is
    // not its, and there is no data endpoint at all in this device.
    assert_eq!(
        host.bulk_out(6, HUB_STATUS_ENDP, USB_DATA0, &[0x55]),
        UsbReply::Nothing,
        "an OUT to an IN-only endpoint is not answered"
    );
    host.idle(20);
    // A token for the endpoint number a device with a bulk pair would have
    // had is nobody's either, which is what `DATA_ENDP = 4'd0` means.
    assert_eq!(
        host.bulk_in(6, 4),
        UsbReply::Nothing,
        "an endpoint this device does not have answers nothing"
    );
    host.idle(20);

    // ------------------------------------------------------------------
    // A bus reset, and a host that enumerates the hub again.
    // ------------------------------------------------------------------
    // The port goes back to powered off, so there is nothing to report
    // until the host powers it — and then there is, which is the same path
    // the first connection took. `ip/usb/usb_cdc_acm` needed a trigger of
    // its own for exactly the case this covers for nothing.
    configure_for(
        host,
        6,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );
    pipe.reconfigured();
    pipe.quiet(
        host,
        6,
        4,
        "a re-enumerated hub has powered-off ports and nothing outstanding",
    );
    host.control_write(6, hub_set_port_feature(port, FEAT_PORT_POWER));
    host.idle(20);
    pipe.expect(
        host,
        6,
        HUB_BITMAP_PORT1,
        "and the device on the port is reported to the host all over again",
    );
    host.idle(10);
    host.assert_clean();
}

#[test]
fn usb_hub_status_change_endpoint_reports_every_change() {
    let design = hub_fs_design();
    let mut host = UsbHost::new(FsPair::new(&design), 0);
    hub_status_changes(&mut host);
}

/// The same, through the **ULPI** link layer and the transceiver that reports
/// LineState a clock late, which is the part on the board.
///
/// The bitmap is a **one-byte** packet out of a second endpoint, so it goes
/// through the ULPI transmitter's length field, that endpoint's own
/// turnaround counter and `usb_dev_core`'s arbitration — with the data
/// endpoint that arbitration was written for absent, which no other test in
/// this file arranges.
#[test]
fn usb_hub_ulpi_status_change_endpoint_reports_every_change() {
    let design = hub_ulpi_design();
    let phy = UlpiPhy::new(ULPI_CPB).reporting_stale_line();
    let mut host = UsbHost::new(UlpiPair::with_phy(&design, phy), 0);
    hub_status_changes(&mut host);
}

/// Everything runs on the one clock, in both wrappers.
#[test]
fn usb_hub_is_one_clock_domain() {
    for top in ["usb_hub_fs", "usb_hub_ulpi"] {
        let kinds = crossings("usb_hub", top, &[("VID", "16'h1209"), ("PID", "16'h0001")]);
        assert!(kinds.is_empty(), "{top}: nothing should cross: {kinds:?}");
    }
}

// ---------------------------------------------------------------------------
// usb_proxy: the half of a hub that forwards
// ---------------------------------------------------------------------------

/// `ip/usb/usb_proxy` at the scale a simulation can run, which is the same
/// trade `HOST_TEST_PARAMS` makes and for the same reasons.
///
/// Every one of these is a **time** on a board and a count here:
///
///   * it **keeps** every ordering — the attach debounce before the port is a
///     port, the PC's port reset before anything is forwarded, the SE0 before
///     the recovery — because each is a state and not a duration;
/// * it **keeps** the downstream bus reset long enough to be one: 20 000
///   clocks is far more than `usb_device_ulpi`'s own `SE0_CYCLES` of 150,
///   so the device behind the port really does see a reset and really does
///   forget the address the PC gave it;
///   * it **does not** check that 100 ms of debounce is 100 ms or that 15 ms of
///     SE0 is 15 ms. Those are `usb_proxy_dn`'s defaults with the section of
///     USB 2.0 that sets each, and only a board exercises them.
///
/// The reset is the one that is **not** shrunk as far as it will go, and the
/// reason is an assertion rather than a device: a whole GetPortStatus is a
/// couple of thousand clocks, so a reset of 800 would be over before the PC
/// could ever read PORT_RESET set, and "the hub reports §11.5.1's **Resetting**
/// state" would be untestable. At 20 000 each the PC polls through it a dozen
/// times.
///
/// `FRAME_CYCLES` is small for the opposite reason: at the real 60 000 a
/// whole enumeration through the proxy fits inside a handful of frames and
/// a SOF would hardly appear. At 4 000 it lands between forwarded
/// transactions over and over, which is where a frame that broke one would
/// be caught.
const PROXY_TEST_PARAMS: &[(&str, &str)] = &[
    ("DEBOUNCE_CYCLES", "200"),
    ("RESET_HOLD", "20000"),
    ("RESET_RECOVERY", "20000"),
    ("FRAME_CYCLES", "4000"),
];

fn proxy_design() -> Design {
    design_of("usb_proxy", "usb_hub_proxy_ulpi", PROXY_TEST_PARAMS)
}

/// What a Link drove this cycle, read off one of the two ULPI buses.
///
/// `data` is read **only while `oe`**, for `UlpiPair::cycle`'s reason: ULPI's
/// data lines are the Link's for exactly as long as `ulpi_data_oe` is high, and
/// on a design whose transmit buffer is a distributed RAM they are `x` until
/// something has filled one.
fn link_out_of(
    sim: &Simulator<'_>,
    oe_net: NetHandle,
    data_net: NetHandle,
    stp_net: NetHandle,
    rst_net: NetHandle,
) -> LinkOut {
    let oe = high(sim, oe_net);
    LinkOut {
        oe,
        data: if oe { octet(get_u64(sim, data_net)) } else { 0 },
        stp: high(sim, stp_net),
        rst_n: high(sim, rst_net),
    }
}

/// Three ends on two wires: the PC, the proxy, and a device behind its port.
///
/// `HostDevice` next door resolves **two** ends of one pair. A proxy needs
/// three, on two pairs, and they are not symmetrical:
///
/// * the **upstream** pair has the PC on it — which is `UsbHost` driving
///   through this as a `UsbPair` — and the proxy's upstream transceiver,
///   which is a *peripheral's*: it has the 1.5 kOhm pull-up that tells the
///   PC something is attached;
/// * the **downstream** pair has the proxy's own transceiver on it, which
///   is a *host's* — two 15 kOhm pull-downs, and the 45 Ohm terminations a
///   bus reset is made of — and a whole second design, `usb_device_ulpi`,
///   behind its own third transceiver.
///
/// So the downstream pair is resolved exactly the way `HostDevice::cycle`
/// resolves its one, and the upstream pair is resolved by `UsbHost`, which
/// drives it while it is sending and reads it when it is not.
struct ProxyRig<'d> {
    sim: Simulator<'d>,
    clk: NetHandle,
    up_dir: NetHandle,
    up_nxt: NetHandle,
    up_data_in: NetHandle,
    up_data_out: NetHandle,
    up_data_oe: NetHandle,
    up_stp: NetHandle,
    up_rst_out: NetHandle,
    dn_dir: NetHandle,
    dn_nxt: NetHandle,
    dn_data_in: NetHandle,
    dn_data_out: NetHandle,
    dn_data_oe: NetHandle,
    dn_stp: NetHandle,
    dn_rst_out: NetHandle,
    address: NetHandle,
    configured: NetHandle,
    reset_net: NetHandle,
    ready_net: NetHandle,
    dn_ready_net: NetHandle,
    setup_seen_net: NetHandle,
    data_fwd_net: NetHandle,
    up_phy: UlpiPhy,
    dn_phy: UlpiPhy,
    dev: UlpiPair<'d>,
    data: DataEp,
    problems: Vec<String>,
    /// SETUPs the relay took and forwarded, and transactions it forwarded that
    /// were **not** part of a control transfer.
    ///
    /// Sampled every cycle, the way `UlpiHost` samples `sof_sent`, because a
    /// one-cycle pulse is not something a caller that steps whole packets can
    /// catch. Counting them is what turns "the PC got an answer" into "the
    /// answer came from the other bus": a transfer that the relay satisfied out
    /// of something of its own would move neither counter.
    setups: u64,
    data_fwds: u64,
}

impl<'d> ProxyRig<'d> {
    fn new(proxy: &'d Design, dev_design: &'d Design, stale: bool) -> ProxyRig<'d> {
        let flaws = |phy: UlpiPhy| {
            if stale {
                phy.hearing_itself().reporting_stale_line()
            } else {
                phy
            }
        };
        // The device first and on its own, which is the order the two really
        // come up in: its start-up connects its own pull-up and waits for the
        // pair to charge, and until it has there is nothing for a host to see.
        let dev = UlpiPair::with_phy(dev_design, flaws(UlpiPhy::new(ULPI_CPB)));
        let sim = simulate(proxy, "usb_hub_proxy_ulpi");
        let pin = |n: &str| top_net(&sim, n);
        let mut rig = ProxyRig {
            clk: pin("clk60"),
            up_dir: pin("up_dir"),
            up_nxt: pin("up_nxt"),
            up_data_in: pin("up_data_i"),
            up_data_out: pin("up_data_o"),
            up_data_oe: pin("up_data_oe"),
            up_stp: pin("up_stp"),
            up_rst_out: pin("up_rst_n"),
            dn_dir: pin("dn_dir"),
            dn_nxt: pin("dn_nxt"),
            dn_data_in: pin("dn_data_i"),
            dn_data_out: pin("dn_data_o"),
            dn_data_oe: pin("dn_data_oe"),
            dn_stp: pin("dn_stp"),
            dn_rst_out: pin("dn_rst_n"),
            address: pin("address"),
            configured: pin("configured"),
            reset_net: pin("usb_reset"),
            ready_net: pin("phy_ready"),
            dn_ready_net: pin("dn_phy_ready"),
            setup_seen_net: pin("setup_seen"),
            data_fwd_net: pin("data_fwd"),
            up_phy: flaws(UlpiPhy::new(ULPI_CPB)),
            dn_phy: flaws(UlpiPhy::new(ULPI_CPB).hosting()),
            dev,
            data: DataEp::new(&sim, true),
            problems: Vec::new(),
            setups: 0,
            data_fwds: 0,
            sim,
        };
        rig.data.quiet(&mut rig.sim);
        rig.sim.set(rig.up_dir, bit(false));
        rig.sim.set(rig.up_nxt, bit(false));
        rig.sim.set(rig.up_data_in, word(8, 0));
        rig.sim.set(rig.dn_dir, bit(false));
        rig.sim.set(rig.dn_nxt, bit(false));
        rig.sim.set(rig.dn_data_in, word(8, 0));
        let clk = rig.clk;
        let rst_n = top_net(&rig.sim, "rst_n");
        reset(&mut rig.sim, clk, rst_n);
        // Both start-ups, with nothing on the upstream pair: the PC has not
        // reset the bus yet, and the proxy's upstream Link has to connect its
        // own pull-up and wait for the pair to charge the way any peripheral
        // does.
        for _ in 0..40_000 {
            if rig.ready() && high(&rig.sim, rig.dn_ready_net) {
                return rig;
            }
            rig.cycle(None);
        }
        panic!(
            "a transceiver was never configured; the upstream one saw {:?} and the \
             downstream one {:?}",
            rig.up_phy.accesses, rig.dn_phy.accesses
        );
    }

    fn ready(&self) -> bool {
        high(&self.sim, self.ready_net)
    }

    /// One of the **device's** own top-level nets, by name: its address, its
    /// `configured`, whatever else it reports. The proxy's own are `port`.
    fn dev_port(&self, name: &str) -> u64 {
        self.dev.port(name)
    }

    /// Cycles until the downstream port has a debounced device on it, or
    /// `limit` cycles go by.
    fn until_attached(&mut self, limit: u64) -> bool {
        for _ in 0..limit {
            if self.port("dn_attached") == 1 {
                return true;
            }
            self.cycle(None);
        }
        false
    }
}

impl UsbPair for ProxyRig<'_> {
    fn cycles_per_bit(&self) -> u64 {
        ULPI_CPB
    }

    fn cycle(&mut self, host: Option<UsbLine>) {
        // The downstream pair, from the outputs of the cycle before. Three
        // things decide what it is at, in the order `HostDevice::cycle` states:
        // the proxy's 45 Ohm terminations, which are what its port reset is and
        // which beat a 1.5 kOhm pull-up thirtyfold; then whichever end is
        // transmitting; then the device's own pull-up once it has charged.
        let h = self.dn_phy.line_out;
        let d = self.dev.phy.line_out;
        if h.is_some() && d.is_some() {
            self.problems
                .push("both ends drove the downstream pair in the same cycle".into());
        }
        let dn_line = if self.dn_phy.drives_se0() {
            UsbLine::Se0
        } else if let Some(state) = h.or(d) {
            state
        } else if self.dev.phy.pullup_ready() {
            UsbLine::J
        } else {
            UsbLine::Se0
        };

        step_data(&mut self.sim, &mut self.data);
        // Both transceivers' outputs, presented before the edge that samples
        // them: `ulpi_data_oe` is combinational in `dir`, as ULPI means it to
        // be, so the low phase has to settle before it is read.
        self.sim.set(self.up_dir, bit(self.up_phy.dir));
        self.sim.set(self.up_nxt, bit(self.up_phy.nxt));
        self.sim
            .set(self.up_data_in, word(8, u64::from(self.up_phy.data)));
        self.sim.set(self.dn_dir, bit(self.dn_phy.dir));
        self.sim.set(self.dn_nxt, bit(self.dn_phy.nxt));
        self.sim
            .set(self.dn_data_in, word(8, u64::from(self.dn_phy.data)));
        self.sim.run_for(HALF);
        let up_link = link_out_of(
            &self.sim,
            self.up_data_oe,
            self.up_data_out,
            self.up_stp,
            self.up_rst_out,
        );
        let dn_link = link_out_of(
            &self.sim,
            self.dn_data_oe,
            self.dn_data_out,
            self.dn_stp,
            self.dn_rst_out,
        );
        self.sim.set(self.clk, bit(true));
        self.sim.run_for(HALF);
        self.sim.set(self.clk, bit(false));
        if high(&self.sim, self.setup_seen_net) {
            self.setups += 1;
        }
        if high(&self.sim, self.data_fwd_net) {
            self.data_fwds += 1;
        }
        self.up_phy.step(&up_link, host);
        self.dn_phy.step(&dn_link, Some(dn_line));
        self.dev.cycle(Some(dn_line));
        // Moved rather than read in place, because `UsbPair::problems`
        // returns one slice and there are three models here.
        self.problems.append(&mut self.up_phy.problems);
        self.problems.append(&mut self.dn_phy.problems);
        self.problems.append(&mut self.dev.phy.problems);
    }

    fn driven(&mut self) -> Option<UsbLine> {
        self.up_phy.line_out
    }

    /// The **hub's** address, which is what a `UsbPair` means by one. The
    /// device behind the port has its own and `dev_port` is how to read
    /// it.
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
        &self.problems
    }

    fn answer_window(&self) -> (u64, u64) {
        (2 * ULPI_CPB, 13 * ULPI_CPB / 2)
    }

    fn added_delay(&self) -> Vec<u64> {
        self.up_phy.held.clone()
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

/// How many times a transaction is offered again while the proxy NAKs it.
///
/// **Counted and never timed.** A NAK means the answer is not here yet and
/// a host's answer to one is to ask again; what a test can assert about
/// that is how many times it had to ask, which is the same number on a fast
/// machine and a slow one. Forty is far more than the two or three a
/// working proxy takes and far less than for ever.
const PROXY_TRIES: usize = 40;

/// Bit times of idle bus between one attempt and the next, which is the
/// breathing room the downstream transaction runs in.
const PROXY_GAP: u64 = 60;

/// A SETUP through the proxy, offered again while it is not acknowledged.
///
/// A device must acknowledge a SETUP and the proxy does so at once — it has the
/// eight bytes — so the retry here is for the bus and not for the proxy.
fn proxy_setup<P: UsbPair>(host: &mut UsbHost<P>, addr: u8, request: [u8; 8]) -> UsbReply {
    let mut reply = UsbReply::Nothing;
    for _ in 0..PROXY_TRIES {
        reply = host.setup(addr, request);
        if reply == UsbReply::Handshake(USB_ACK) {
            return reply;
        }
        host.idle(PROXY_GAP);
    }
    reply
}

/// A control read through the proxy: `UsbHost::control_read`'s transfer with
/// every stage offered again while it is NAKed, and its failures **returned**
/// rather than asserted so that a test can say which stage went wrong.
///
/// `maxpkt` is the **device's** `bMaxPacketSize0` and not the hub's,
/// because what ends the data stage is a packet shorter than what the
/// device behind the port declared. That is "a short packet ends a
/// transfer" from this side of it.
fn proxy_control_read<P: UsbPair>(
    host: &mut UsbHost<P>,
    addr: u8,
    request: [u8; 8],
    maxpkt: usize,
) -> Result<Vec<u8>, String> {
    let reply = proxy_setup(host, addr, request);
    if reply != UsbReply::Handshake(USB_ACK) {
        return Err(format!("the SETUP was answered {reply:?}"));
    }
    host.idle(4);
    let length = usize::from(u16::from_le_bytes([request[6], request[7]]));
    let mut got = Vec::new();
    let mut toggle = USB_DATA1;
    loop {
        let mut data = None;
        for _ in 0..PROXY_TRIES {
            match host.bulk_in(addr, 0) {
                UsbReply::Data(pid, payload) => {
                    data = Some((pid, payload));
                    break;
                }
                UsbReply::Handshake(USB_NAK) => host.idle(PROXY_GAP),
                other => return Err(format!("the data stage was answered {other:?}")),
            }
        }
        let Some((pid, payload)) = data else {
            return Err(format!(
                "the data stage was NAKed {PROXY_TRIES} times running after {} bytes",
                got.len()
            ));
        };
        if pid != toggle {
            return Err(format!(
                "the data stage carried PID {pid:#x} where the toggle said {toggle:#x}, \
                 after {} bytes",
                got.len()
            ));
        }
        let short = payload.len() < maxpkt;
        got.extend(payload);
        host.ack();
        toggle = other_toggle(toggle);
        if short || got.len() >= length {
            break;
        }
    }
    // The status stage of a read: a zero-length OUT carrying DATA1.
    for _ in 0..PROXY_TRIES {
        match host.bulk_out(addr, 0, USB_DATA1, &[]) {
            UsbReply::Handshake(USB_ACK) => {
                host.idle(4);
                return Ok(got);
            }
            UsbReply::Handshake(USB_NAK) => host.idle(PROXY_GAP),
            other => return Err(format!("the status stage was answered {other:?}")),
        }
    }
    Err("the status stage of a read was NAKed to exhaustion".into())
}

/// A control transfer with no data stage, through the proxy: the SETUP and then
/// the zero-length IN of the status stage, acknowledged.
fn proxy_control_write<P: UsbPair>(
    host: &mut UsbHost<P>,
    addr: u8,
    request: [u8; 8],
) -> Result<(), String> {
    let reply = proxy_setup(host, addr, request);
    if reply != UsbReply::Handshake(USB_ACK) {
        return Err(format!("the SETUP was answered {reply:?}"));
    }
    host.idle(4);
    for _ in 0..PROXY_TRIES {
        match host.bulk_in(addr, 0) {
            UsbReply::Data(USB_DATA1, payload) if payload.is_empty() => {
                host.ack();
                host.idle(4);
                return Ok(());
            }
            UsbReply::Handshake(USB_NAK) => host.idle(PROXY_GAP),
            other => return Err(format!("the status stage was answered {other:?}")),
        }
    }
    Err("the status stage of a write was NAKed to exhaustion".into())
}

/// A bulk pipe through the proxy, with its two toggles — which are the **PC's**
/// toggles and not the proxy's, so that the two sides staying in step is
/// something this asserts rather than something it assumes.
struct ProxyPipe {
    endp: u8,
    out_pid: u8,
    in_pid: u8,
    naks: usize,
}

impl ProxyPipe {
    fn new(endp: u8) -> ProxyPipe {
        ProxyPipe {
            endp,
            out_pid: USB_DATA0,
            in_pid: USB_DATA0,
            naks: 0,
        }
    }

    fn write<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8, payload: &[u8]) {
        for _ in 0..PROXY_TRIES {
            match host.bulk_out(addr, self.endp, self.out_pid, payload) {
                UsbReply::Handshake(USB_ACK) => {
                    self.out_pid = other_toggle(self.out_pid);
                    return;
                }
                UsbReply::Handshake(USB_NAK) => {
                    self.naks += 1;
                    host.idle(PROXY_GAP);
                }
                other => panic!("an OUT of {} bytes was answered {other:?}", payload.len()),
            }
        }
        panic!("the proxy NAKed all {PROXY_TRIES} attempts at an OUT");
    }

    fn read<P: UsbPair>(&mut self, host: &mut UsbHost<P>, addr: u8) -> Vec<u8> {
        for _ in 0..PROXY_TRIES {
            match host.bulk_in(addr, self.endp) {
                UsbReply::Data(pid, payload) => {
                    assert_eq!(
                        pid, self.in_pid,
                        "the IN endpoint's data toggle, as the PC keeps it"
                    );
                    host.ack();
                    self.in_pid = other_toggle(self.in_pid);
                    return payload;
                }
                UsbReply::Handshake(USB_NAK) => {
                    self.naks += 1;
                    host.idle(PROXY_GAP);
                }
                other => panic!("an IN was answered {other:?}"),
            }
        }
        panic!("the proxy NAKed all {PROXY_TRIES} attempts at an IN");
    }
}

/// The four bytes of a GetPortStatus, as words.
fn proxy_port_status(host: &mut UsbHost<ProxyRig<'_>>, hub_addr: u8) -> (u16, u16) {
    port_words(
        &host
            .control_read(hub_addr, hub_get_port_status(HUB_NBR_PORTS))
            .expect("GetPortStatus"),
    )
}

/// The hub enumerated, its port powered and reset, and the reset waited out —
/// which is everything the PC does before it can reach the device behind the
/// port, and the point at which forwarding begins.
///
/// The device's attach is **not poked**: `port_attached` is not an input of
/// this design at all, the way it is of `ip/usb/usb_hub` on its own. It is
/// `usb_proxy_dn`'s own debounced sight of the downstream pair leaving SE0,
/// so this runs the clock until it has seen it. That difference is why this
/// harness exists.
fn proxy_open_the_port(host: &mut UsbHost<ProxyRig<'_>>, hub_addr: u8) {
    assert!(
        host.pair.until_attached(8_000),
        "the downstream device attached; dn_stage is {}",
        host.port("dn_stage")
    );

    configure_for(
        host,
        hub_addr,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );

    let port = HUB_NBR_PORTS;
    assert_eq!(
        proxy_port_status(host, hub_addr),
        (0, 0),
        "a configured hub's port starts powered off, whatever is plugged into it"
    );
    host.control_write(hub_addr, hub_set_port_feature(port, FEAT_PORT_POWER));
    host.idle(20);
    assert_eq!(
        proxy_port_status(host, hub_addr),
        (PORT_STAT_POWER | PORT_STAT_CONNECTION, PORT_CHG_CONNECTION),
        "the port the host has just powered reports the device on it"
    );
    host.control_write(
        hub_addr,
        hub_clear_port_feature(port, FEAT_C_PORT_CONNECTION),
    );
    host.idle(20);

    // THE RESET THAT TAKES TIME
    //
    // SetPortFeature(PORT_RESET) reaches the real device now:
    // `usb_proxy_dn` writes `50h` into the downstream transceiver's
    // Function Control register, holds SE0 for `RESET_HOLD`, writes `45h`
    // back and waits `RESET_RECOVERY`. So PORT_RESET reads **set** while
    // that is happening — the bit `ip/usb/usb_hub` used to report as a
    // constant zero because its reset was over in the cycle it was asked
    // for — and the port is not enabled until it is over.
    host.control_write(hub_addr, hub_set_port_feature(port, FEAT_PORT_RESET));
    let (stat, _) = proxy_port_status(host, hub_addr);
    assert_ne!(
        stat & PORT_STAT_RESET,
        0,
        "PORT_RESET is set while the reset is being driven at the device"
    );
    assert_eq!(
        stat & PORT_STAT_ENABLE,
        0,
        "and the port is not enabled until it is over"
    );

    for _ in 0..200 {
        let (stat, chg) = proxy_port_status(host, hub_addr);
        if stat & PORT_STAT_RESET == 0 && chg & PORT_CHG_RESET != 0 {
            assert_ne!(
                stat & PORT_STAT_ENABLE,
                0,
                "a reset that completed enables the port"
            );
            host.control_write(hub_addr, hub_clear_port_feature(port, FEAT_C_PORT_RESET));
            host.idle(20);
            return;
        }
        host.idle(40);
    }
    panic!(
        "the port reset never completed; wPortStatus {:#06x}, dn_stage {}",
        proxy_port_status(host, hub_addr).0,
        host.port("dn_stage")
    );
}

/// The PC's own enumeration of the device behind the port, which is the
/// whole of what a proxy is for: `dev_addr` is assigned by **the PC**,
/// through the proxy, and the device's own `address` output is what says
/// it was taken.
fn proxy_enumerate_the_device(
    host: &mut UsbHost<ProxyRig<'_>>,
    dev_addr: u8,
    device_want: &[u8],
    config_want: &[u8],
    maxpkt0: usize,
) {
    // A reset put the device at address 0, which is where the PC starts.
    assert_eq!(
        host.pair.dev_port("address"),
        0,
        "the port reset reached the device and it forgot its address"
    );

    let device = proxy_control_read(host, 0, GET_DEVICE_DESCRIPTOR, maxpkt0)
        .expect("the device descriptor, through the proxy");
    assert_eq!(
        device, device_want,
        "the descriptor the PC read is the device's own and not the proxy's"
    );

    proxy_control_write(host, 0, set_address(dev_addr)).expect("SET_ADDRESS, through the proxy");
    assert_eq!(
        host.pair.dev_port("address"),
        u64::from(dev_addr),
        "pass-through addressing: the device took the address the PC chose"
    );
    host.idle(20);

    // The device descriptor again at the new address, which is the one
    // transaction that tells "the address was accepted" from "the device is
    // still answering at 0" — the two look identical until something addresses
    // it.
    let again = proxy_control_read(host, dev_addr, get_descriptor(1, 18), maxpkt0)
        .expect("the device descriptor at the address the PC assigned");
    assert_eq!(again, device_want);

    // The first nine bytes of the configuration descriptor and then the
    // whole of it: the same two reads a host does, and the second is where
    // a multi-packet data stage and its toggle live.
    let nine = proxy_control_read(host, dev_addr, get_descriptor(2, 9), maxpkt0)
        .expect("the configuration header");
    assert_eq!(nine, config_want[..9]);
    let config = proxy_control_read(
        host,
        dev_addr,
        [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00],
        maxpkt0,
    )
    .expect("the whole configuration descriptor");
    assert_eq!(
        config, config_want,
        "the configuration descriptor and everything under it"
    );

    assert!(
        !host.pair.dev.configured(),
        "the device is not configured until the PC says so"
    );
    proxy_control_write(host, dev_addr, [0x00, 0x09, 0x01, 0, 0, 0, 0, 0])
        .expect("SET_CONFIGURATION, through the proxy");
    assert!(
        host.pair.dev.configured(),
        "the device is configured, by the PC, through the proxy"
    );
    host.idle(20);
}

/// The whole of it: a PC enumerates a device it can only reach through our hub.
///
/// What this would catch, and it is the round's whole claim: a control transfer
/// not forwarded in either direction, a SETUP the device never saw, a data
/// toggle that drifts between the two buses, a data stage cut in the wrong
/// place, an address the device did not take, a port reset that did not reach
/// it, and a descriptor byte that is the proxy's rather than the device's.
///
/// What it would **not** catch: that a *kernel* enumerates it. A host model
/// written from the same specification as the proxy can agree with it about
/// something they are both wrong about, which is the sentence
/// `ip/usb/usb_device_ulpi/README.md` §11 wrote before that block had a
/// board; `ip/usb/usb_proxy/README.md` §8 is the other half. Nor any of the
/// times, which `PROXY_TEST_PARAMS` says.
fn proxy_enumerate_our_device(stale: bool) {
    let proxy = proxy_design();
    let dev_design = ulpi_design();
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, stale), 0);

    proxy_open_the_port(&mut host, 3);
    assert_eq!(
        host.port("proxied"),
        0,
        "nothing has been forwarded yet: the PC has only talked to the hub"
    );
    assert_eq!(
        host.pair.setups, 0,
        "and no SETUP has gone downstream: every control transfer so far was the hub's own"
    );

    proxy_enumerate_the_device(
        &mut host,
        9,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
        EP0_MAXPKT,
    );

    assert_eq!(
        host.port("proxied"),
        1,
        "the PC addressed something behind the port"
    );
    assert_eq!(
        host.port("job"),
        0,
        "and the relay is idle again, holding no answer nobody asked for"
    );
    assert_ne!(
        host.port("dn_frame"),
        0,
        "frames went out on the downstream bus while this was happening"
    );
    assert_eq!(
        host.port("dn_reg_failed"),
        0,
        "and the transceiver took every register write the port reset needed"
    );
    // SIX CONTROL TRANSFERS, AND EVERY ONE OF THEM REACHED THE DEVICE
    //
    // `proxy_enumerate_the_device` does exactly six: the device descriptor,
    // the address, the device descriptor again, the configuration header,
    // the whole configuration, and the configuration value. One SETUP each,
    // forwarded once each — and that last part is what this number is for.
    // A relay that answered a repeated SETUP out of something of its own,
    // or that forwarded one twice because the PC retried a later stage,
    // would not read six.
    assert_eq!(
        host.pair.setups, 6,
        "one SETUP forwarded for each of the six control transfers the PC did, and not \
         one more"
    );
    assert_eq!(
        host.pair.data_fwds, 0,
        "and nothing outside a control transfer: the PC has not touched an endpoint of its \
         own yet"
    );
    host.assert_clean();
    assert!(
        host.pair.problems.is_empty(),
        "the three transceiver models saw nothing wrong:\n  {}",
        host.pair.problems.join("\n  ")
    );
}

/// A PC enumerating, through our hub, a device our hub never enumerated.
#[test]
fn usb_proxy_enumerates_the_device_behind_the_port() {
    proxy_enumerate_our_device(false);
}

/// The same, with all three transceiver models behaving the way the part on
/// the board does: each hears its own transmission and each reports
/// `LineState` **late**, one transition at a time out of a backlog that
/// outlives the packet. Both were measured on a Microchip USB3343 on a
/// Cynthion r1.4 and both are things ULPI either permits or forbids and the
/// part does anyway.
///
/// It matters more here than anywhere else in this file, because the thing
/// that decides when the downstream port is a port at all is `LineState`: a
/// proxy whose attach detection believed a stale reading would reset a
/// device that had not arrived, or never reset one that had.
#[test]
fn usb_proxy_enumerates_through_the_transceivers_that_are_on_the_board() {
    proxy_enumerate_our_device(true);
}

/// The same enumeration against a device whose `bMaxPacketSize0` is **eight**,
/// which is the smallest USB 2.0 §5.5.3 allows.
///
/// This is the test that reaches what the other two cannot. Our device declares
/// 64, so every descriptor it sends fits in one packet, and a data stage of one
/// packet proves nothing about a toggle or about where a transfer ends. With
/// eight, the eighteen-byte device descriptor arrives in three packets and the
/// thirty-two-byte configuration descriptor in four — and the second of those
/// also reaches the *other* way a data stage ends, because four full packets is
/// exactly the length asked for and there is no short packet to stop on.
///
/// What it would catch: a toggle this proxy did not forward or did not track,
/// which is invisible at one packet a stage; a short packet treated as the
/// middle of a transfer; and a transfer that needed a short packet to end and
/// hung without one.
#[test]
fn usb_proxy_forwards_a_data_stage_of_more_than_one_packet() {
    let proxy = proxy_design();
    let dev_design = design_of(
        "usb_device_ulpi",
        "usb_device_ulpi",
        &[
            ("VID", "16'h1209"),
            ("PID", "16'h0001"),
            ("MAXPKT0", "7'd8"),
        ],
    );
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, false), 0);

    proxy_open_the_port(&mut host, 3);

    // The device descriptor with `bMaxPacketSize0` of eight in byte 7, which is
    // the one byte of it this parameter changes.
    let mut device_want = expected_device_descriptor(0x1209, 0x0001);
    device_want[7] = 8;
    proxy_enumerate_the_device(
        &mut host,
        9,
        &device_want,
        &expected_configuration_descriptor(),
        8,
    );

    // A read of exactly thirty-two bytes of a thirty-two-byte descriptor: four
    // whole packets of eight and **no short packet**, so the data stage ends
    // because `wLength` was reached. That is the branch a proxy that only
    // handled short packets would hang in.
    let exact = proxy_control_read(&mut host, 9, get_descriptor(2, 32), 8)
        .expect("four whole packets and no short one");
    assert_eq!(exact, expected_configuration_descriptor());
    host.assert_clean();
}

/// Bytes through the port, in both directions, on the bulk pair.
///
/// `ip/usb/usb_device_ulpi`'s own data endpoint is wired in **loopback** by
/// this file's harness — `out_*` into `in_*`, which is the arrangement the
/// board has — so a packet written to endpoint 1 OUT comes back from
/// endpoint 1 IN. Four of them, of four different lengths, so that:
///
///   * the OUT toggle alternates DATA0, DATA1, DATA0, DATA1 across four packets
///     and the IN toggle does the same independently, which is what two
///     endpoints' worth of toggle tracking means;
/// * a **short** packet goes both ways, which on a bulk endpoint is the
///   end of a transfer and must not be padded or merged;
/// * and a **full** 64-byte packet goes both ways, which is the one that
///   fills the relay's buffer exactly.
///
/// What it would catch: a toggle the proxy got wrong in either direction, a
/// packet forwarded twice, a packet dropped, a length lost, and a buffer whose
/// bytes come out in the wrong order. What it would **not** catch: throughput,
/// which is not asserted anywhere and is a count of transactions rather than a
/// number of seconds.
#[test]
fn usb_proxy_moves_bytes_through_the_port() {
    let proxy = proxy_design();
    let dev_design = ulpi_design();
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, false), 0);

    proxy_open_the_port(&mut host, 3);
    proxy_enumerate_the_device(
        &mut host,
        9,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
        EP0_MAXPKT,
    );

    let mut pipe = ProxyPipe::new(1);
    let payloads: [Vec<u8>; 4] = [
        vec![0xA5],
        (0..8u8).collect(),
        // A full packet, each byte different from its index so that a
        // buffer read at the wrong offset comes back wrong rather than
        // plausible.
        (0..u8::try_from(BULK_MAXPKT).expect("a legal wMaxPacketSize"))
            .map(|i| i ^ 0x5A)
            .collect(),
        vec![0xDE, 0xAD, 0xBE, 0xEF],
    ];
    for payload in &payloads {
        pipe.write(&mut host, 9, payload);
        let back = pipe.read(&mut host, 9);
        assert_eq!(
            &back, payload,
            "the bytes the PC wrote came back out of the device's loopback"
        );
    }
    assert_eq!(
        pipe.out_pid, USB_DATA0,
        "four OUT packets put the toggle back where it started"
    );
    assert_eq!(pipe.in_pid, USB_DATA0, "and four IN packets likewise");
    // EIGHT TRANSACTIONS WENT DOWN THE OTHER BUS, AND NOT ONE MORE
    //
    // Four OUT packets and four IN packets, each forwarded exactly once.
    // This is the number that distinguishes bytes the device really sent
    // from bytes a relay held on to and handed over twice, and it is the
    // assertion a board cannot make: a host turns a NAK for ever into a
    // timeout, so from the PC's side a bulk endpoint the proxy never
    // reached and one the device NAKed are the same thing. `data_fwd` is
    // that difference, and `testdata/fpga/cynthion/usb_proxy_target.v` puts
    // it on its console for exactly that reason.
    assert_eq!(
        host.pair.data_fwds, 8,
        "four OUTs and four INs forwarded, one downstream transaction each"
    );
    host.assert_clean();
}

/// A device that refuses a request, and a PC that is told so.
///
/// A string descriptor is the request to use: `ip/usb/usb_device_ulpi` has no
/// strings, so endpoint 0 there offers the request to a class that is not there
/// and stalls it — which is a **real** STALL from the real device and not a
/// condition this test manufactures.
///
/// What it would catch: a proxy that swallowed the refusal, which is the
/// failure mode that matters, because a device that refuses a request and a
/// proxy that never answers look completely different to a host: the first
/// is a driver finding out that what it asked for does not exist, the
/// second is a transfer that hangs for five seconds and then a device the
/// kernel gives up on. It also catches a STALL that is not sticky — USB 2.0
/// §8.5.3 has a stalled control transfer stay stalled until the next SETUP
/// — and a STALL that outlives the transfer it belonged to, which would
/// refuse everything after it.
#[test]
fn usb_proxy_propagates_a_stall_from_the_device() {
    let proxy = proxy_design();
    let dev_design = ulpi_design();
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, false), 0);

    proxy_open_the_port(&mut host, 3);
    proxy_enumerate_the_device(
        &mut host,
        9,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
        EP0_MAXPKT,
    );

    // GET_DESCRIPTOR of a string, which the device behind the port does not
    // have. The SETUP is acknowledged — the proxy holds the eight bytes — and
    // the data stage is the stage that carries the refusal.
    let request = [0x80, 0x06, 0x01, 0x03, 0x09, 0x04, 0xFF, 0x00];
    assert_eq!(
        proxy_setup(&mut host, 9, request),
        UsbReply::Handshake(USB_ACK),
        "a SETUP is always acknowledged, whatever the request turns out to be"
    );
    host.idle(4);
    let mut stalled = false;
    for _ in 0..PROXY_TRIES {
        match host.bulk_in(9, 0) {
            UsbReply::Handshake(USB_STALL) => {
                stalled = true;
                break;
            }
            UsbReply::Handshake(USB_NAK) => host.idle(PROXY_GAP),
            other => panic!("a request the device stalls was answered {other:?}"),
        }
    }
    assert!(
        stalled,
        "the device's STALL reached the PC rather than being swallowed"
    );

    // §8.5.3: it stays stalled until the next SETUP.
    host.idle(10);
    assert_eq!(
        host.bulk_in(9, 0),
        UsbReply::Handshake(USB_STALL),
        "a stalled control transfer stays stalled"
    );
    host.idle(10);

    // And the next transfer is served as if none of it had happened, which is
    // the other half: a STALL that outlived its transfer would refuse this.
    let device = proxy_control_read(&mut host, 9, GET_DEVICE_DESCRIPTOR, EP0_MAXPKT)
        .expect("the device recovers, through the proxy");
    assert_eq!(device, expected_device_descriptor(0x1209, 0x0001));
    host.assert_clean();
}

/// A port the host has not enabled forwards nothing, and a port reset forgets
/// everything about what is behind it.
///
/// Two properties with one setup, and both are about the gate rather than about
/// the forwarding:
///
///   * before the PC resets the port, a token for an address that is not the
///     hub's gets **nothing at all** — not a NAK, which would claim an endpoint
///     — because `enabled` is low and the relay has not claimed the token;
/// * resetting the port a second time, after the device has an address,
///   puts the device back at 0. That is what pass-through addressing needs
///   most: without it the device would keep an address from a previous
///   session while the PC talked to address 0, and nothing would answer.
///
/// What it would catch: a relay that claimed tokens before the PC had enabled
/// the port, which would answer for a device the PC has not reset and whose
/// address is therefore anybody's guess; and a port reset that moved a bit in
/// the hub without reaching the transceiver, which is the whole of what
/// `usb_proxy_dn` adds.
#[test]
fn usb_proxy_forwards_nothing_until_the_host_has_reset_the_port() {
    let proxy = proxy_design();
    let dev_design = ulpi_design();
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, false), 0);

    assert!(
        host.pair.until_attached(8_000),
        "the downstream device attached; dn_stage is {}",
        host.port("dn_stage")
    );
    configure_for(
        &mut host,
        3,
        &expected_hub_device_descriptor(),
        &expected_hub_configuration(),
    );

    // The hub is configured and its port is not even powered. A token for
    // another address is nobody's.
    for addr in [0, 9] {
        assert_eq!(
            host.bulk_in(addr, 0),
            UsbReply::Nothing,
            "an IN to address {addr} with the port unpowered is answered by nothing"
        );
        host.idle(10);
    }

    host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_POWER));
    host.idle(20);
    // Powered, connected, and still not **enabled**: §11.5.1 only leaves a port
    // Enabled after a reset, and the relay is gated on that and not on power.
    let (stat, _) = proxy_port_status(&mut host, 3);
    assert_ne!(stat & PORT_STAT_CONNECTION, 0, "the device is reported");
    assert_eq!(stat & PORT_STAT_ENABLE, 0, "and the port is not enabled");
    assert_eq!(
        host.bulk_in(9, 0),
        UsbReply::Nothing,
        "a powered port that has not been reset still forwards nothing"
    );
    assert_eq!(host.port("proxied"), 0, "and nothing was ever forwarded");
    host.idle(20);

    // Now open it properly and give the device an address, so that the second
    // reset has something to forget.
    host.control_write(
        3,
        hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_CONNECTION),
    );
    host.idle(20);
    host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_RESET));
    let mut enabled = false;
    for _ in 0..200 {
        let (stat, chg) = proxy_port_status(&mut host, 3);
        if stat & PORT_STAT_RESET == 0 && chg & PORT_CHG_RESET != 0 {
            enabled = stat & PORT_STAT_ENABLE != 0;
            break;
        }
        host.idle(40);
    }
    assert!(enabled, "the first port reset enabled the port");
    host.control_write(3, hub_clear_port_feature(HUB_NBR_PORTS, FEAT_C_PORT_RESET));
    host.idle(20);

    proxy_control_write(&mut host, 0, set_address(9)).expect("SET_ADDRESS through the proxy");
    assert_eq!(host.pair.dev_port("address"), 9);
    host.idle(20);

    // A second port reset, which is what a PC does when an enumeration goes
    // wrong and what it does every time it re-enumerates the hub.
    host.control_write(3, hub_set_port_feature(HUB_NBR_PORTS, FEAT_PORT_RESET));
    for _ in 0..200 {
        let (stat, chg) = proxy_port_status(&mut host, 3);
        if stat & PORT_STAT_RESET == 0 && chg & PORT_CHG_RESET != 0 {
            break;
        }
        host.idle(40);
    }
    assert_eq!(
        host.pair.dev_port("address"),
        0,
        "the second port reset reached the device, which forgot the address the PC gave it"
    );
    host.assert_clean();
}

/// A SETUP that arrives while a downstream transaction is still running.
///
/// **HIGH** (USB 2.0 §8.5.3) and it is not optional: a SETUP starts a new
/// control transfer whatever the last one was doing, because the host has moved
/// on. So the relay's one job can be replaced while `usb_host_sie` is still
/// running the job it replaced, and the two have to not be confused for each
/// other.
///
/// The transaction in flight here is an IN to **endpoint 5**, which
/// `usb_device_ulpi` has not got: `usb_bulk_ep` answers a token for another
/// endpoint number with nothing at all, which is what a host must see from an
/// endpoint that is not in the descriptors, so the engine spends its whole
/// timeout and its retries on it. That is long enough for a SETUP to land well
/// inside it, which `job` not being idle asserts rather than assumes.
///
/// What this would catch: a relay that hangs, loses the transfer, or
/// answers the PC out of the abandoned job's buffer.
///
/// What it would **not** catch, and this is the honest part: the defect the
/// fix in `usb_proxy_relay`'s "What the engine is asked for" is about.
/// There, a new job read the **old** transaction's `trn_busy` as its own
/// and then the old transaction's status as its own answer — and the
/// harmful case is the old status being an acknowledgement, which would set
/// `ct_setup_ok` for a SETUP the device never saw. Reaching that needs the
/// preempted transaction to be one that gets acknowledged *and* to complete
/// inside the few hundred clocks between the PC's SETUP token and its next
/// token, and both of those are the host model's own packet timing rather
/// than anything a test can ask for. With a transaction that times out
/// instead — which is what this one does — the defect produced a transfer
/// that recovered on the host's next retry, so this test passes against the
/// broken implementation too. It is here for the hang and the corruption,
/// and that file's comment is where the rest of it is written down.
#[test]
fn usb_proxy_takes_a_setup_that_preempts_a_transaction_in_flight() {
    let proxy = proxy_design();
    let dev_design = ulpi_design();
    let mut host = UsbHost::new(ProxyRig::new(&proxy, &dev_design, false), 0);

    proxy_open_the_port(&mut host, 3);
    proxy_enumerate_the_device(
        &mut host,
        9,
        &expected_device_descriptor(0x1209, 0x0001),
        &expected_configuration_descriptor(),
        EP0_MAXPKT,
    );

    let before = host.pair.data_fwds;
    assert_eq!(
        host.bulk_in(9, 5),
        UsbReply::Handshake(USB_NAK),
        "an IN to an endpoint the device has not got is NAKed while the relay asks it"
    );
    assert_eq!(
        host.pair.data_fwds,
        before + 1,
        "and the IN was forwarded, which is what makes a transaction be in flight"
    );
    assert_ne!(
        host.port("job"),
        0,
        "the job is still running: the device behind the port answers an endpoint it \
         has not got with silence, so the engine is spending its timeout"
    );

    // And now a whole control transfer, with that transaction still in flight.
    let device = proxy_control_read(&mut host, 9, GET_DEVICE_DESCRIPTOR, EP0_MAXPKT)
        .expect("a control transfer whose SETUP preempted a transaction in flight");
    assert_eq!(device, expected_device_descriptor(0x1209, 0x0001));
    host.assert_clean();
}

#[test]
fn usb_proxy_is_one_clock_domain() {
    let kinds = crossings("usb_proxy", "usb_hub_proxy_ulpi", PROXY_TEST_PARAMS);
    assert!(kinds.is_empty(), "nothing should cross: {kinds:?}");
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
/// It was found by one. `ip/usb/usb_cdc_acm`'s fifty-eight byte descriptor set,
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
    // The CDC ACM descriptor set, as `ip/usb/usb_cdc_acm` states it, for the
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

    // Four of them and two LUT widths. The first is the descriptor set that
    // has always worked on a board, the second is the one that did not, the
    // third is the block that states it for itself, and the fourth is a
    // **fourth blob** — `ip/usb/usb_hub`'s twenty-five bytes — because what the
    // defect below depended on was the ROM's contents and not its length.
    //
    // What the fourth case does **not** cover is the hub descriptor of USB 2.0
    // §11.23.2.1, which is a `case` in `usb_hub_req` and not a part-select of
    // a constant, so it is not a ROM and is not what this test is about.
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
        Case {
            variant: Variant {
                package: "usb_hub",
                top: "usb_hub_fs",
                params: &[("VID", "16'h1209"), ("PID", "16'h0001"), ("BUF_RAM", "0")],
            },
            want: expected_hub_configuration,
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
            if package == "usb_hub" {
                hub_port_empty(&mut host);
            }
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

// ---------------------------------------------------------------------------
// Crypto: SHA-256 and ChaCha20
// ---------------------------------------------------------------------------

/// A byte string as a `Logic`, **first byte at the most significant end**.
///
/// That is the convention both crypto blocks use for every flat
/// multi-byte port — `key`, `nonce`, `digest`, `block` — and it is the
/// order a specification prints a key in, so a vector copied out of
/// RFC 8439 goes straight in.
fn byte_string(bytes: &[u8]) -> Logic {
    let parts: Vec<Logic> = bytes
        .iter()
        .map(|b| Logic::from_u64(u64::from(*b), 8))
        .collect();
    Logic::concat_all(parts.iter())
}

/// The `n` bytes a net holds, read back the same way round.
fn net_bytes(sim: &Simulator<'_>, handle: NetHandle, n: usize) -> Vec<u8> {
    let value = sim.get(handle);
    assert_eq!(
        value.width(),
        u32::try_from(n * 8).expect("a width that fits"),
        "net is {} bits, {n} bytes asked for",
        value.width()
    );
    (0..n)
        .map(|i| {
            let hi = u32::try_from((n - i) * 8 - 1).expect("a bit index that fits");
            octet(
                value
                    .slice(hi, hi - 7)
                    .to_u64()
                    .unwrap_or_else(|| panic!("byte {i} holds x or z: {value}")),
            )
        })
        .collect()
}

/// A hexadecimal string, lower case and unseparated, which is how every
/// expectation in this section is written: a digest or a keystream is
/// copied out of its document as one string and compared as one string,
/// so a mismatch prints both in full and the eye finds where they part.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The message body the padding table uses: byte `i` is
/// `(i * 7 + 13) & 0xff`.
///
/// Neither all zeros nor text, cheap to restate, and — the point — the
/// same rule the `purecrypto` run that produced the expected digests
/// used, so the two sides of that comparison were written once.
fn crypto_pattern(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| octet(((i * 7 + 13) & 0xff) as u64))
        .collect()
}

/// Everything a testbench needs to hold of `sha256`.
struct Sha256Bench {
    clk: NetHandle,
    rst_n: NetHandle,
    start: NetHandle,
    in_byte: NetHandle,
    in_valid: NetHandle,
    in_last: NetHandle,
    in_ready: NetHandle,
    digest: NetHandle,
    digest_valid: NetHandle,
}

impl Sha256Bench {
    fn attach(sim: &mut Simulator<'_>) -> Self {
        let bench = Sha256Bench {
            clk: top_net(sim, "clk"),
            rst_n: top_net(sim, "rst_n"),
            start: top_net(sim, "start"),
            in_byte: top_net(sim, "in_byte"),
            in_valid: top_net(sim, "in_valid"),
            in_last: top_net(sim, "in_last"),
            in_ready: top_net(sim, "in_ready"),
            digest: top_net(sim, "digest"),
            digest_valid: top_net(sim, "digest_valid"),
        };
        sim.set(bench.start, bit(false));
        sim.set(bench.in_valid, bit(false));
        sim.set(bench.in_last, bit(false));
        sim.set(bench.in_byte, word(8, 0));
        reset(sim, bench.clk, bench.rst_n);
        bench
    }

    /// Hashes one message and returns its digest and the number of clock
    /// cycles from the `start` pulse to `digest_valid`.
    ///
    /// `in_valid` is held high for every byte, with no gaps, so the cycle
    /// count is a property of the block and not of this driver. That is
    /// what makes it worth comparing two of them.
    fn message(&self, sim: &mut Simulator<'_>, msg: &[u8]) -> ([u8; 32], u64) {
        // A new message from a known state, whatever the last one left.
        sim.set(self.start, bit(true));
        cycle(sim, self.clk, HALF);
        sim.set(self.start, bit(false));

        let mut index = 0usize;
        let mut ended = false;
        let mut cycles = 0u64;
        // Two blocks of padding is the worst case, and a block is 129
        // cycles; this is that with room, and it is a loop bound rather
        // than a time-out.
        let limit = 200 + 140 * (msg.len() as u64 / 64 + 2);
        loop {
            if ended {
                sim.set(self.in_valid, bit(false));
                sim.set(self.in_last, bit(false));
            } else {
                let more = index < msg.len();
                sim.set(self.in_valid, bit(more));
                sim.set(
                    self.in_byte,
                    word(8, if more { u64::from(msg[index]) } else { 0 }),
                );
                // `in_last` ends the message at this point in the stream;
                // with `in_valid` low it ends it with no byte, which is
                // how the empty message is spelled.
                sim.set(self.in_last, bit(index + 1 >= msg.len()));
            }
            // `in_ready` is driven by registers only, so its value here
            // is the one the last edge produced and setting the inputs
            // above cannot have disturbed it.
            let accepted = !ended && high(sim, self.in_ready);
            cycle(sim, self.clk, HALF);
            cycles += 1;
            if accepted {
                if index + 1 >= msg.len() {
                    ended = true;
                } else {
                    index += 1;
                }
            }
            if high(sim, self.digest_valid) {
                let bytes = net_bytes(sim, self.digest, 32);
                let mut digest = [0u8; 32];
                digest.copy_from_slice(&bytes);
                return (digest, cycles);
            }
            assert!(
                cycles < limit,
                "sha256 did not finish a {}-byte message in {limit} cycles",
                msg.len()
            );
        }
    }
}

/// The three messages FIPS 180-4 Appendix B works through, and the empty
/// one it does not.
///
/// B.1 is `"abc"`, one block. B.2 is the 56-byte string, which is **the
/// padding edge**: 56 bytes leave no room for the eight-byte length
/// field, so the padding spills into a second block. FIPS 180-4's own
/// example being exactly 56 bytes long is not a coincidence, and it is
/// the case a byte-stream hasher gets wrong.
///
/// The empty message is **not** in FIPS 180-4 Appendix B. Its digest is
/// published widely and is what `purecrypto` computes for `b""` on this
/// machine, and the padding rule derives it by hand; this test treats it
/// as a vector from a running implementation and §3 of
/// `ip/crypto/sha256/README.md` says so.
///
/// What this test would catch: any wrong K constant, any wrong initial
/// value, a wrong sigma rotation, a wrong round order, a wrong byte
/// order on the way in or out, padding that omits or misplaces the `1`
/// bit or the length, and — this is B.2's job — padding that does not
/// spill into a second block when it must.
///
/// What it would not catch: a message longer than two blocks (B.3, below,
/// and the length table after it), a length that is not one of these
/// four, and anything at all about timing.
#[test]
fn sha256_matches_the_fips_180_4_examples() {
    let design = design_of("sha256", "sha256", &[]);
    let mut sim = simulate(&design, "sha256");
    let bench = Sha256Bench::attach(&mut sim);

    let cases: [(&str, &[u8], &str); 3] = [
        (
            "FIPS 180-4 B.1",
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "FIPS 180-4 B.2",
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            "the empty message",
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
    ];

    for (what, message, expected) in cases {
        let (digest, cycles) = bench.message(&mut sim, message);
        assert_eq!(
            hex(&digest),
            expected,
            "{what}: {} bytes, {cycles} cycles",
            message.len()
        );
    }

    // B.2 is two blocks and B.1 is one, which is the whole point of
    // having both: the chaining value has to carry from one to the next.
    let (_, one) = bench.message(&mut sim, b"abc");
    let (_, two) = bench.message(
        &mut sim,
        b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
    );
    assert!(
        two > one,
        "the 56-byte message must cost more than the 3-byte one: {two} against {one}"
    );
}

/// Every padding branch there is, at nineteen message lengths.
///
/// The expected digests come from **`purecrypto`**, the user's
/// from-scratch Rust crypto library, run over the same `crypto_pattern`
/// rule this test uses. That makes them a second, independent oracle
/// rather than a reading: `purecrypto` reproduces all four of the
/// published values in the test above and all nine ChaCha20 vectors
/// below, so by the time it is trusted here it has agreed with the
/// authorities everywhere both of them speak.
///
/// The lengths are chosen around every boundary the padding has:
///
/// | Length mod 64 | What it exercises |
/// |---|---|
/// | 55 | the `1` bit lands on byte 55, the length field fills 56..63, **no zeros at all** |
/// | 56 | no room for the length field: the padding spills into a whole extra block |
/// | 57..63 | the spill, with the zeros wrapping a block boundary |
/// | 0 | a whole block of message and then a whole block of nothing but padding |
/// | 1, 3 | the ordinary case, short |
///
/// What it would catch: an off-by-one in where the length field starts,
/// a zero run that stops at the wrong byte, a byte counter that counts
/// padding bytes as message bytes, a length field written little-endian,
/// and a chaining value that does not survive three blocks.
///
/// What it would not catch: a length above 192 bytes, a counter that
/// overflows LEN_BITS (nothing simulated here comes near 2^61 bytes),
/// and a block that is wrong in a way the compression function and the
/// padding cancel out — which is why the published vectors above are a
/// separate test and not folded into this one.
#[test]
fn sha256_pads_every_length_purecrypto_was_asked_about() {
    let design = design_of("sha256", "sha256", &[]);
    let mut sim = simulate(&design, "sha256");
    let bench = Sha256Bench::attach(&mut sim);

    // Produced by `purecrypto::hash::sha256` over `crypto_pattern(n)`.
    const TABLE: [(usize, &str); 19] = [
        (
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            1,
            "9d1e0e2d9459d06523ad13e28a4093c2316baafe7aec5b25f30eba2e113599c4",
        ),
        (
            3,
            "10c38ff6aa77aaf0a08345dea78ac620f90429576e826996518aba266f14b46f",
        ),
        (
            54,
            "a181b5442ceb430fd1c2b2245a953f17ca4058881c1282024b973b8562a805be",
        ),
        (
            55,
            "764c574722e6e2ccaa5422f8ec731111ac72ff7039793148623e56b75a32c11f",
        ),
        (
            56,
            "43fbbe48a6796cb7414a92cd785d9f4a976c2f70fc59c60a309f95e3022db77a",
        ),
        (
            57,
            "e038a2370dbd74c3c8b89b95e7c351fec4821e3415f7aef3a0925215bc6ff953",
        ),
        (
            63,
            "c309180feace42e90107301813aef6f309cac604e831b3fd9692a3298aa6da54",
        ),
        (
            64,
            "3a38aed112131d75fc0e636437f5b675c83c01ade88d99f6b6c54b0d6129174f",
        ),
        (
            65,
            "2ee4bedec261c1561dafa7ba28e4e3ece281bc0f51afca40b83b3a2a7c41a050",
        ),
        (
            118,
            "4a6edb5613289eccf0568a8091fc9ae750bbf82f5352528e4688d65480d6cc38",
        ),
        (
            119,
            "0a70cbf85ea376617e4bfad11040a9559638f8ceb57844a901573674578af539",
        ),
        (
            120,
            "7d3fd765bd3d4a0587f5bab94200b1d38b23398b94544ff5257f695b2227918f",
        ),
        (
            121,
            "ce828eed6582389e10c153a818088fd9f2b48d8478325f72d0bd67f979b8536d",
        ),
        (
            127,
            "ff998a2ad3412188b7ba531324bf977b22e77aa3b1befb11c699bf2a14959ee7",
        ),
        (
            128,
            "8b94fd8b7db8b1ef29c089c16389697a057310b7c739c1ad844e9be970f5cfd6",
        ),
        (
            129,
            "22afcb610b1282b24536c87a33acc00a80c720c9d3509960ae11a9bd87501330",
        ),
        (
            192,
            "6b2a097aff28b485a0c701b5da8724a4cb7d4d97c6f2178ba43fe295beca2232",
        ),
        // 108 is the middle of a block rather than an edge of one, as a
        // control: one full block of message, then 44 bytes of message
        // with the 1 bit, the zeros and the length behind them.
        (
            108,
            "13f6a38d129ef9870df728f2d3364ae2ab9acbb8d1de245b1f0ecbe1050f43c6",
        ),
    ];

    for (length, expected) in TABLE {
        let message = crypto_pattern(length);
        let (digest, cycles) = bench.message(&mut sim, &message);
        assert_eq!(
            hex(&digest),
            expected,
            "a {length}-byte message, {cycles} cycles"
        );
    }
}

/// `sha256_core` on its own, fed a block this test padded itself.
///
/// The point is that the two halves separate: a caller with padded blocks
/// uses the core and gets the same answer the wrapper would have given,
/// which is what makes "padding is the wrapper's job" a decomposition and
/// not a claim. The padded block here is `"abc"` padded by hand from
/// FIPS 180-4 §5.1.1 — three message bytes, `0x80`, fifty-two zeros, and
/// `0x0000000000000018` — so this test states the padding rule
/// independently of the hardware that implements it.
///
/// What it would catch: a compression function that only works behind its
/// own padder (a shared register the wrapper happened to initialise, say),
/// and a `block_valid` that fires at the wrong time.
///
/// What it would not catch: anything about the padding logic, which is the
/// point — this test is the one that does not use it.
#[test]
fn sha256_core_compresses_a_block_somebody_else_padded() {
    let design = design_of("sha256", "sha256_core", &[]);
    let mut sim = simulate(&design, "sha256_core");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let start = top_net(&sim, "start");
    let in_byte = top_net(&sim, "in_byte");
    let in_valid = top_net(&sim, "in_valid");
    let in_ready = top_net(&sim, "in_ready");
    let block_valid = top_net(&sim, "block_valid");
    let state = top_net(&sim, "state");

    sim.set(start, bit(false));
    sim.set(in_valid, bit(false));
    sim.set(in_byte, word(8, 0));
    reset(&mut sim, clk, rst_n);

    // FIPS 180-4 §5.1.1 applied to "abc" by hand: the message, the single
    // 1 bit as 0x80, zeros, and the length 24 as sixty-four big-endian
    // bits.
    let mut padded = Vec::from(*b"abc");
    padded.push(0x80);
    padded.resize(56, 0x00);
    padded.extend_from_slice(&(24u64).to_be_bytes());
    assert_eq!(padded.len(), 64, "one block");

    let mut index = 0usize;
    sim.set(in_valid, bit(true));
    sim.set(in_byte, word(8, u64::from(padded[0])));
    for _ in 0..400 {
        let accepted = high(&sim, in_ready) && index < padded.len();
        cycle(&mut sim, clk, HALF);
        if accepted {
            index += 1;
            if index < padded.len() {
                sim.set(in_byte, word(8, u64::from(padded[index])));
            } else {
                sim.set(in_valid, bit(false));
            }
        }
        if high(&sim, block_valid) {
            assert_eq!(index, padded.len(), "the whole block went in first");
            assert_eq!(
                hex(&net_bytes(&sim, state, 32)),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "the chaining value after one padded block is the digest of \"abc\""
            );
            return;
        }
    }
    panic!("sha256_core never finished a block");
}

/// **The fixed-latency measurement for SHA-256.**
///
/// A hardware block whose cycle count depends on a secret is the direct
/// analogue of a secret-dependent branch, which is the thing
/// `purecrypto`'s `ct` module exists to avoid. So: nine messages of the
/// same length whose bytes are as different as they can be made — all
/// zeros, all ones, one bit set at each end, a counting pattern, its
/// complement — and the assertion is that every one of them takes the
/// **same number of cycles** from `start` to `digest_valid`. Then the
/// same again at six lengths, including both sides of the padding edge.
///
/// What it establishes: the cycle count is a function of the message
/// *length* and of nothing in the message's *content*. That is the
/// property that matters, because a length is not a secret — a caller
/// streams the bytes in, so the byte count is visible on the interface
/// whatever this block does, and no block could hide it.
///
/// What it does **not** establish, and the README says this again at more
/// length: anything about power or electromagnetic emission. Differential
/// power analysis works on a block with a perfectly fixed cycle count,
/// and nothing here has been near an oscilloscope. It also says nothing
/// about the *synthesised* netlist's gate delays — this is a cycle count
/// in a zero-delay simulation, so it proves the control path is
/// data-independent and not that the data path's timing is.
///
/// What it would not catch: a data-dependent *stall* that the driver's
/// own back-pressure hid, which is why the driver holds `in_valid` high
/// for every byte with no gaps; and a cycle count that depends on the
/// *previous* message, which is why `start` is pulsed before each one.
#[test]
fn sha256_takes_the_same_cycles_whatever_the_message_says() {
    let design = design_of("sha256", "sha256", &[]);
    let mut sim = simulate(&design, "sha256");
    let bench = Sha256Bench::attach(&mut sim);

    // Every length is measured at every one of these bodies. 55 and 56
    // are the two sides of the padding edge, 64 is a whole block, and 0
    // is the message with no bytes at all.
    for length in [0usize, 1, 55, 56, 64, 130] {
        let bodies: Vec<Vec<u8>> = vec![
            vec![0x00; length],
            vec![0xff; length],
            vec![0xaa; length],
            crypto_pattern(length),
            crypto_pattern(length).iter().map(|b| !b).collect(),
            (0..length).map(|i| if i == 0 { 1 } else { 0 }).collect(),
            (0..length)
                .map(|i| if i + 1 == length { 0x80 } else { 0 })
                .collect(),
            (0..length)
                .map(|i| octet((i as u64).wrapping_mul(31)))
                .collect(),
            (0..length).map(|i| octet(!(i as u64) & 0xff)).collect(),
        ];

        let mut seen: Vec<(u64, String)> = Vec::new();
        for body in &bodies {
            assert_eq!(body.len(), length, "a body of the stated length");
            let (digest, cycles) = bench.message(&mut sim, body);
            seen.push((cycles, hex(&digest)));
        }

        let first = seen[0].0;
        for (cycles, digest) in &seen {
            assert_eq!(
                *cycles, first,
                "a {length}-byte message took {cycles} cycles where another took \
                 {first}; the digest was {digest}"
            );
        }
        // And the nine bodies really were different messages, so the
        // equality above is not the equality of nine identical runs. The
        // empty message is the one exception: there is only one of it.
        let digests: BTreeSet<&String> = seen.iter().map(|(_, d)| d).collect();
        if length == 0 {
            assert_eq!(digests.len(), 1, "there is one empty message");
        } else {
            assert!(
                digests.len() > 1,
                "a {length}-byte message gave one digest for nine different bodies, so \
                 this test proved nothing about it"
            );
        }
    }
}

/// FIPS 180-4 Appendix B.3: one million `'a'` characters.
///
/// Ignored, because it is 15 625 blocks at 129 cycles each — about two
/// million clock edges — and that is minutes in a debug build rather
/// than the two seconds the rest of this section takes. It is here
/// because it is the only vector in the standard that runs the chaining
/// value through four figures of blocks, and because the length field it
/// produces, 8 000 000, is the only published one with bits above the
/// low sixteen set.
///
/// Run it with
/// `cargo test --all-features --test ip_library --release -- --ignored
/// --nocapture sha256_hashes_the_million`.
#[test]
#[ignore = "two million clock edges; run it on purpose"]
fn sha256_hashes_the_million_characters_of_appendix_b_3() {
    let design = design_of("sha256", "sha256", &[]);
    let mut sim = simulate(&design, "sha256");
    let bench = Sha256Bench::attach(&mut sim);
    let message = vec![b'a'; 1_000_000];
    let (digest, cycles) = bench.message(&mut sim, &message);
    println!("FIPS 180-4 B.3: {cycles} cycles for 1 000 000 bytes");
    assert_eq!(
        hex(&digest),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

/// RFC 8439 §2.1.1 and §2.2.1: the quarter round, on its own ports.
///
/// Two vectors from two different sections. §2.1.1 is four numbers chosen
/// to make the arithmetic visible; §2.2.1 applies QUARTERROUND(2, 7, 8,
/// 13) to a sample state, which is a *diagonal* round's quarter and
/// therefore reaches the same logic through a different set of values.
///
/// This is why `chacha20_qr` is a module. A keystream that comes out
/// wrong tells you nothing about which of the four additions, the four
/// exclusive-ors or the four rotations is at fault; these two vectors
/// tell you it is none of them.
///
/// What it would catch: a wrong rotation amount, a rotation written as a
/// shift, the wrong operand order in any of the four steps, a rotation
/// applied before the exclusive-or rather than after.
///
/// What it would not catch: anything about how `chacha20_core` wires
/// four of these together, which is §2.3.2's job below, or the state
/// setup, or the feed-forward addition.
#[test]
fn chacha20_qr_matches_rfc_8439_2_1_1_and_2_2_1() {
    let design = design_of("chacha20", "chacha20_qr", &[]);
    let mut sim = simulate(&design, "chacha20_qr");
    let a_in = top_net(&sim, "a_in");
    let b_in = top_net(&sim, "b_in");
    let c_in = top_net(&sim, "c_in");
    let d_in = top_net(&sim, "d_in");
    let a_out = top_net(&sim, "a_out");
    let b_out = top_net(&sim, "b_out");
    let c_out = top_net(&sim, "c_out");
    let d_out = top_net(&sim, "d_out");

    let cases: [(&str, [u32; 4], [u32; 4]); 2] = [
        (
            "RFC 8439 2.1.1",
            [0x1111_1111, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567],
            [0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb],
        ),
        (
            // The sample state of §2.2.1 at indices 2, 7, 8 and 13.
            "RFC 8439 2.2.1, QUARTERROUND(2, 7, 8, 13)",
            [0x5164_61b1, 0x2a5f_714c, 0x5337_2767, 0x3d63_1689],
            [0xbdb8_86dc, 0xcfac_afd2, 0xe46b_ea80, 0xccc0_7c79],
        ),
    ];

    for (what, input, expected) in cases {
        sim.set(a_in, word(32, u64::from(input[0])));
        sim.set(b_in, word(32, u64::from(input[1])));
        sim.set(c_in, word(32, u64::from(input[2])));
        sim.set(d_in, word(32, u64::from(input[3])));
        sim.run_for(HALF);
        let got = [
            get_u32(&sim, a_out),
            get_u32(&sim, b_out),
            get_u32(&sim, c_out),
            get_u32(&sim, d_out),
        ];
        assert_eq!(
            got.map(|v| format!("{v:08x}")),
            expected.map(|v| format!("{v:08x}")),
            "{what}"
        );
    }
}

/// One Appendix A.1 case: a label, a key, a nonce, a block counter and the
/// sixty-four keystream bytes RFC 8439 prints for them.
type BlockVector = (&'static str, [u8; 32], [u8; 12], u32, &'static str);

/// Everything `chacha20_core` needs from a testbench.
struct ChaChaCore {
    clk: NetHandle,
    rst_n: NetHandle,
    start: NetHandle,
    advance: NetHandle,
    key: NetHandle,
    nonce: NetHandle,
    counter: NetHandle,
    valid: NetHandle,
    block: NetHandle,
    /// The working state, read for RFC 8439 §2.3.2's intermediate value.
    st_q: NetHandle,
}

impl ChaChaCore {
    fn attach(sim: &mut Simulator<'_>) -> Self {
        let bench = ChaChaCore {
            clk: top_net(sim, "clk"),
            rst_n: top_net(sim, "rst_n"),
            start: top_net(sim, "start"),
            advance: top_net(sim, "advance"),
            key: top_net(sim, "key"),
            nonce: top_net(sim, "nonce"),
            counter: top_net(sim, "counter"),
            valid: top_net(sim, "valid"),
            block: top_net(sim, "block"),
            st_q: top_net(sim, "st_q"),
        };
        sim.set(bench.start, bit(false));
        sim.set(bench.advance, bit(false));
        sim.set(bench.key, byte_string(&[0u8; 32]));
        sim.set(bench.nonce, byte_string(&[0u8; 12]));
        sim.set(bench.counter, word(32, 0));
        reset(sim, bench.clk, bench.rst_n);
        bench
    }

    /// One block. Returns its 64 serialised bytes, the sixteen state
    /// words as they stood **after the twenty rounds and before the
    /// final addition**, and the cycles from `start` to `valid`.
    fn block_of(
        &self,
        sim: &mut Simulator<'_>,
        key: &[u8; 32],
        nonce: &[u8; 12],
        counter: u32,
    ) -> (Vec<u8>, [u32; 16], u64) {
        sim.set(self.key, byte_string(key));
        sim.set(self.nonce, byte_string(nonce));
        sim.set(self.counter, word(32, u64::from(counter)));
        sim.set(self.start, bit(true));
        cycle(sim, self.clk, HALF);
        sim.set(self.start, bit(false));

        let mut cycles = 1u64;
        loop {
            // The state as the last edge left it. When `valid` comes up
            // on the next edge, this is the value the addition was
            // applied to — which is the table RFC 8439 §2.3.2 prints
            // under "After running 20 rounds".
            let before = self.state_words(sim);
            cycle(sim, self.clk, HALF);
            cycles += 1;
            if high(sim, self.valid) {
                return (net_bytes(sim, self.block, 64), before, cycles);
            }
            assert!(cycles < 200, "chacha20_core never raised valid");
        }
    }

    fn state_words(&self, sim: &Simulator<'_>) -> [u32; 16] {
        let value = sim.get(self.st_q);
        let mut out = [0u32; 16];
        for (i, word) in out.iter_mut().enumerate() {
            let hi = u32::try_from(511 - 32 * i).expect("a bit index that fits");
            *word = narrow(
                value
                    .slice(hi, hi - 31)
                    .to_u64()
                    .unwrap_or_else(|| panic!("state word {i} holds x or z")),
            );
        }
        out
    }
}

/// RFC 8439 §2.3.2, including **the intermediate state**.
///
/// The RFC prints three sixteen-word tables for one block: the state as
/// set up, the state after twenty rounds, and the state after the
/// original is added back. The middle one is the valuable one, and it is
/// the reason this test reads an internal register and not only a port:
/// if the keystream is wrong but the after-twenty-rounds state is right,
/// the fault is in the feed-forward addition or the serialisation; if the
/// intermediate state is wrong, the fault is in a quarter round or in how
/// the four of them are wired into a column or a diagonal round, and
/// `chacha20_qr_matches_rfc_8439_2_1_1_and_2_2_1` says which.
///
/// What it would catch: the state setup (the constants, the little-endian
/// key load, the counter in word 12 and not 13), the column/diagonal
/// alternation and which words each kind touches, the round count, the
/// feed-forward addition, and the serialisation's byte order.
///
/// What it would not catch: a counter that is byte-swapped — this vector's
/// counter is 1, which is the same number either way round. Appendix A.1
/// test vector #4 uses counter 2 and #5 counter 0 with a nonce that is
/// not symmetric, and those are the next test.
#[test]
fn chacha20_core_reaches_the_state_rfc_8439_2_3_2_prints() {
    let design = design_of("chacha20", "chacha20_core", &[]);
    let mut sim = simulate(&design, "chacha20_core");
    let bench = ChaChaCore::attach(&mut sim);

    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = octet(i as u64);
    }
    let nonce: [u8; 12] = [0, 0, 0, 0x09, 0, 0, 0, 0x4a, 0, 0, 0, 0];

    let (block, after_rounds, cycles) = bench.block_of(&mut sim, &key, &nonce, 1);

    // "After running 20 rounds (10 column rounds interleaved with 10
    // diagonal rounds), the ChaCha state looks like this".
    const AFTER_20: [u32; 16] = [
        0x8377_78ab,
        0xe238_d763,
        0xa67a_e21e,
        0x5950_bb2f,
        0xc4f2_d0c7,
        0xfc62_bb2f,
        0x8fa0_18fc,
        0x3f5e_c7b7,
        0x3352_71c2,
        0xf294_89f3,
        0xeabd_a8fc,
        0x82e4_6ebd,
        0xd19c_12b4,
        0xb04e_16de,
        0x9e83_d0cb,
        0x4e3c_50a2,
    ];
    assert_eq!(
        after_rounds.map(|w| format!("{w:08x}")),
        AFTER_20.map(|w| format!("{w:08x}")),
        "the state after twenty rounds, RFC 8439 2.3.2"
    );

    // "Serialized Block".
    assert_eq!(
        hex(&block),
        "10f1e7e4d13b5915500fdd1fa32071c4\
         c7d1f4c733c068030422aa9ac3d46c4e\
         d2826446079faa0914c2d705d98b02a2\
         b5129cd1de164eb9cbd083e8a2503c4e",
        "the serialised block, RFC 8439 2.3.2"
    );
    // 20 rounds, one cycle to add the initial state back, one to start.
    assert_eq!(cycles, 22, "a block is 22 cycles");
}

/// RFC 8439 Appendix A.1: five more blocks of the block function.
///
/// These are the vectors that pin down what §2.3.2's cannot. Between them
/// they use counter 0, 1 and 2, a key of all zeros, a key with its last
/// byte set, a key with `0xff` in its *second* byte, and a nonce with a
/// 2 in its last byte — so a byte-swapped counter, a reversed key load
/// and a nonce written into the wrong words are all visible here and
/// none of them is visible in §2.3.2.
///
/// All five were also run through `purecrypto` on this machine and agree,
/// which is what earns `purecrypto` the right to be the oracle for the
/// SHA-256 length table above.
///
/// What it would not catch: anything about the stream — the counter
/// advancing between blocks, the exclusive-or, the 32-bit port — which is
/// §2.4.2 and A.2 below.
#[test]
fn chacha20_core_matches_rfc_8439_appendix_a_1() {
    let design = design_of("chacha20", "chacha20_core", &[]);
    let mut sim = simulate(&design, "chacha20_core");
    let bench = ChaChaCore::attach(&mut sim);

    let zero = [0u8; 32];
    let mut one_at_end = [0u8; 32];
    one_at_end[31] = 1;
    let mut ff_second = [0u8; 32];
    ff_second[1] = 0xff;
    let n_zero = [0u8; 12];
    let mut n_two = [0u8; 12];
    n_two[11] = 2;

    let cases: [BlockVector; 5] = [
        (
            "#1",
            zero,
            n_zero,
            0,
            "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7\
             da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586",
        ),
        (
            "#2",
            zero,
            n_zero,
            1,
            "9f07e7be5551387a98ba977c732d080dcb0f29a048e3656912c6533e32ee7aed\
             29b721769ce64e43d57133b074d839d531ed1f28510afb45ace10a1f4b794d6f",
        ),
        (
            "#3",
            one_at_end,
            n_zero,
            1,
            "3aeb5224ecf849929b9d828db1ced4dd832025e8018b8160b82284f3c949aa5a\
             8eca00bbb4a73bdad192b5c42f73f2fd4e273644c8b36125a64addeb006c13a0",
        ),
        (
            "#4",
            ff_second,
            n_zero,
            2,
            "72d54dfbf12ec44b362692df94137f328fea8da73990265ec1bbbea1ae9af0ca\
             13b25aa26cb4a648cb9b9d1be65b2c0924a66c54d545ec1b7374f4872e99f096",
        ),
        (
            "#5",
            zero,
            n_two,
            0,
            "c2c64d378cd536374ae204b9ef933fcd1a8b2288b3dfa49672ab765b54ee27c7\
             8a970e0e955c14f3a88e741b97c286f75f8fc299e8148362fa198a39531bed6d",
        ),
    ];

    for (what, key, nonce, counter, expected) in cases {
        let (block, _, cycles) = bench.block_of(&mut sim, &key, &nonce, counter);
        assert_eq!(hex(&block), expected, "RFC 8439 A.1 {what}");
        assert_eq!(cycles, 22, "RFC 8439 A.1 {what} took {cycles} cycles");
    }
}

/// Everything `chacha20` needs from a testbench.
struct ChaChaStream {
    clk: NetHandle,
    rst_n: NetHandle,
    start: NetHandle,
    key: NetHandle,
    nonce: NetHandle,
    counter: NetHandle,
    in_data: NetHandle,
    in_valid: NetHandle,
    in_ready: NetHandle,
    out_data: NetHandle,
    out_valid: NetHandle,
    exhausted: NetHandle,
}

impl ChaChaStream {
    fn attach(sim: &mut Simulator<'_>) -> Self {
        let bench = ChaChaStream {
            clk: top_net(sim, "clk"),
            rst_n: top_net(sim, "rst_n"),
            start: top_net(sim, "start"),
            key: top_net(sim, "key"),
            nonce: top_net(sim, "nonce"),
            counter: top_net(sim, "counter"),
            in_data: top_net(sim, "in_data"),
            in_valid: top_net(sim, "in_valid"),
            in_ready: top_net(sim, "in_ready"),
            out_data: top_net(sim, "out_data"),
            out_valid: top_net(sim, "out_valid"),
            exhausted: top_net(sim, "exhausted"),
        };
        sim.set(bench.start, bit(false));
        sim.set(bench.in_valid, bit(false));
        sim.set(bench.in_data, word(32, 0));
        sim.set(bench.key, byte_string(&[0u8; 32]));
        sim.set(bench.nonce, byte_string(&[0u8; 12]));
        sim.set(bench.counter, word(32, 0));
        reset(sim, bench.clk, bench.rst_n);
        bench
    }

    /// Encrypts (or decrypts — it is the same thing) `data`, returning
    /// the result and the cycles from `start` to the last `out_valid`.
    ///
    /// A final partial word is padded with zeros and the extra output
    /// bytes dropped, which is the block's documented answer to a length
    /// that is not a multiple of four: the keystream depends on the key,
    /// the nonce and the position and on nothing in the data, so the
    /// discarded bytes change nothing.
    fn apply(
        &self,
        sim: &mut Simulator<'_>,
        key: &[u8; 32],
        nonce: &[u8; 12],
        counter: u32,
        data: &[u8],
    ) -> (Vec<u8>, u64) {
        sim.set(self.key, byte_string(key));
        sim.set(self.nonce, byte_string(nonce));
        sim.set(self.counter, word(32, u64::from(counter)));
        sim.set(self.start, bit(true));
        cycle(sim, self.clk, HALF);
        sim.set(self.start, bit(false));

        let mut words: Vec<u32> = Vec::new();
        for chunk in data.chunks(4) {
            let mut four = [0u8; 4];
            four[..chunk.len()].copy_from_slice(chunk);
            words.push(u32::from_be_bytes(four));
        }

        let mut sent = 0usize;
        let mut out: Vec<u8> = Vec::new();
        let mut cycles = 1u64;
        let limit = 100 + 60 * (words.len() as u64 + 2);
        while out.len() < words.len() * 4 {
            let more = sent < words.len();
            sim.set(self.in_valid, bit(more));
            sim.set(
                self.in_data,
                word(32, if more { u64::from(words[sent]) } else { 0 }),
            );
            let moved = more && high(sim, self.in_ready);
            cycle(sim, self.clk, HALF);
            cycles += 1;
            if moved {
                sent += 1;
            }
            if high(sim, self.out_valid) {
                out.extend_from_slice(&get_u32(sim, self.out_data).to_be_bytes());
            }
            assert!(
                cycles < limit,
                "chacha20 produced {} of {} bytes in {limit} cycles",
                out.len(),
                words.len() * 4
            );
        }
        sim.set(self.in_valid, bit(false));
        out.truncate(data.len());
        (out, cycles)
    }
}

/// RFC 8439 §2.4.2: the sunscreen text, 114 bytes over two blocks.
///
/// This is the first vector here that needs the *stream*: 114 bytes is
/// one block and most of another, so the counter has to advance from 1 to
/// 2 between them with nothing re-started, and the 29th word is a partial
/// one. The RFC prints the keystream as well as the ciphertext, so both
/// are checked — the keystream by encrypting zeros, which is the same
/// thing the block function's output is and a separate path through this
/// module.
///
/// What it would catch: a counter that does not advance between blocks, a
/// counter that advances by the wrong amount, a block boundary at the
/// wrong word, the exclusive-or dropping or swapping bytes, and the
/// registered output being off by a word.
///
/// What it would not catch: a counter that does not advance *correctly
/// past a carry*, which needs a counter near a byte boundary — A.2 #3
/// below starts at 42 and A.2 #2 at 1, so neither does that either. A
/// block starting at counter 0xFFFFFFFE would, and
/// `chacha20_stops_rather_than_repeat_its_keystream` is the test that
/// goes there.
#[test]
fn chacha20_encrypts_the_text_of_rfc_8439_2_4_2() {
    let design = design_of("chacha20", "chacha20", &[]);
    let mut sim = simulate(&design, "chacha20");
    let bench = ChaChaStream::attach(&mut sim);

    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = octet(i as u64);
    }
    let nonce: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    let plain: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    assert_eq!(plain.len(), 114, "the RFC's own length");

    let (cipher, cycles) = bench.apply(&mut sim, &key, &nonce, 1, plain);
    assert_eq!(
        hex(&cipher),
        "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0b\
         f91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d8\
         07ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab7793736\
         5af90bbf74a35be6b40b8eedf2785e42874d",
        "RFC 8439 2.4.2 ciphertext, in {cycles} cycles"
    );

    // The keystream the RFC prints beside it, which is what encrypting
    // zeros gives. 128 bytes, because the RFC prints both blocks.
    let (keystream, _) = bench.apply(&mut sim, &key, &nonce, 1, &[0u8; 128]);
    assert_eq!(
        hex(&keystream),
        "224f51f3401bd9e12fde276fb8631ded8c131f823d2c06e27e4fcaec9ef3cf78\
         8a3b0aa372600a92b57974cded2b9334794cba40c63e34cdea212c4cf07d41b7\
         69a6749f3f630f4122cafe28ec4dc47e26d4346d70b98c73f3e9c53ac40c5945\
         398b6eda1a832c89c167eacd901d7e2bf363740373201aa188fbbce83991c4ed",
        "RFC 8439 2.4.2 keystream"
    );

    // Encryption and decryption are the same operation, so the
    // ciphertext put back through gives the plaintext. The RFC does not
    // say this in a vector and it does not have to; it is what a
    // keystream cipher is, and a block that got it wrong would be wrong
    // in a way the vectors above cannot see.
    let (back, _) = bench.apply(&mut sim, &key, &nonce, 1, &cipher);
    assert_eq!(back, plain, "decryption is the same operation");
}

/// RFC 8439 Appendix A.2: two more encryptions, one of 375 bytes.
///
/// #2 is the IETF Note Well text, 375 bytes — six blocks — at counter 1
/// with a key and nonce that are almost but not entirely zero. #3 is 127
/// bytes at counter **42** with a key that is nothing but entropy, which
/// is the only vector anywhere here whose starting counter is neither 0,
/// 1 nor 2.
///
/// Both agree with `purecrypto` as well, which is #3's second reason for
/// being here: it is the longest thing in this section and the oracle and
/// the document both say the same of it.
///
/// What it would not catch: #1 of A.2, which is 64 zero bytes under a
/// zero key — already covered exactly by A.1 #1, since encrypting zeros
/// is the keystream.
#[test]
fn chacha20_matches_rfc_8439_appendix_a_2() {
    let design = design_of("chacha20", "chacha20", &[]);
    let mut sim = simulate(&design, "chacha20");
    let bench = ChaChaStream::attach(&mut sim);

    // A.2 #2.
    let mut key = [0u8; 32];
    key[31] = 1;
    let mut nonce = [0u8; 12];
    nonce[11] = 2;
    let plain: &[u8] = b"Any submission to the IETF intended by the Contributor for publication as all or part of an IETF Internet-Draft or RFC and any statement made within the context of an IETF activity is considered an \"IETF Contribution\". Such statements include oral statements in IETF sessions, as well as written and electronic communications made at any time or place, which are addressed to";
    assert_eq!(plain.len(), 375, "the RFC's own length");
    let (cipher, cycles) = bench.apply(&mut sim, &key, &nonce, 1, plain);
    assert_eq!(
        hex(&cipher),
        "a3fbf07df3fa2fde4f376ca23e82737041605d9f4f4f57bd8cff2c1d4b7955ec\
         2a97948bd3722915c8f3d337f7d370050e9e96d647b7c39f56e031ca5eb6250d\
         4042e02785ececfa4b4bb5e8ead0440e20b6e8db09d881a7c6132f420e527950\
         42bdfa7773d8a9051447b3291ce1411c680465552aa6c405b7764d5e87bea85a\
         d00f8449ed8f72d0d662ab052691ca66424bc86d2df80ea41f43abf937d3259d\
         c4b2d0dfb48a6c9139ddd7f76966e928e635553ba76c5c879d7b35d49eb2e62b\
         0871cdac638939e25e8a1e0ef9d5280fa8ca328b351c3c765989cbcf3daa8b6c\
         cc3aaf9f3979c92b3720fc88dc95ed84a1be059c6499b9fda236e7e818b04b0b\
         c39c1e876b193bfe5569753f88128cc08aaa9b63d1a16f80ef2554d7189c411f\
         5869ca52c5b83fa36ff216b9c1d30062bebcfd2dc5bce0911934fda79a86f6e6\
         98ced759c3ff9b6477338f3da4f9cd8514ea9982ccafb341b2384dd902f3d1ab\
         7ac61dd29c6f21ba5b862f3730e37cfdc4fd806c22f221",
        "RFC 8439 A.2 #2, in {cycles} cycles"
    );

    // A.2 #3.
    let key3: [u8; 32] = [
        0x1c, 0x92, 0x40, 0xa5, 0xeb, 0x55, 0xd3, 0x8a, 0xf3, 0x33, 0x88, 0x86, 0x04, 0xf6, 0xb5,
        0xf0, 0x47, 0x39, 0x17, 0xc1, 0x40, 0x2b, 0x80, 0x09, 0x9d, 0xca, 0x5c, 0xbc, 0x20, 0x70,
        0x75, 0xc0,
    ];
    let plain3: &[u8] = b"'Twas brillig, and the slithy toves\nDid gyre and gimble in the wabe:\nAll mimsy were the borogoves,\nAnd the mome raths outgrabe.";
    assert_eq!(plain3.len(), 127, "the RFC's own length");
    let (cipher3, _) = bench.apply(&mut sim, &key3, &nonce, 42, plain3);
    assert_eq!(
        hex(&cipher3),
        "62e6347f95ed87a45ffae7426f27a1df5fb69110044c0d73118effa95b01e5cf\
         166d3df2d721caf9b21e5fb14c616871fd84c54f9d65b283196c7fe4f60553eb\
         f39c6402c42234e32a356b3e764312a61a5532055716ead6962568f87d3f3f77\
         04c6a8d1bcd1bf4d50d6154b6da731b187b58dfd728afa36757a797ac188d1",
        "RFC 8439 A.2 #3"
    );
}

/// **The fixed-latency measurement for ChaCha20.**
///
/// Nine keys as different from each other as thirty-two bytes can be,
/// each with three nonces and three counters, and the assertion is that
/// `chacha20_core` takes the **same 22 cycles** for every one of the
/// eighty-one combinations. Then the same for the stream: nine keys over
/// a 200-byte message, same cycle count every time.
///
/// Why this is the test that matters here: ChaCha20's cycle count has no
/// honest reason to depend on the key, so an implementation where it does
/// has a data-dependent branch in it — which is exactly the thing an
/// AES implementation with a table-driven S-box has, and exactly the
/// reason AES is not in this round.
///
/// What it establishes: twenty rounds happen whatever the key, the nonce
/// and the counter are, and nothing in this block short-circuits on a
/// zero word or an equal pair.
///
/// What it does **not** establish: anything about power or
/// electromagnetic emission, which is where a real attack on this
/// primitive would go, and nothing about gate delay in the synthesised
/// netlist. `ip/crypto/chacha20/README.md` §5 is the whole of that.
///
/// What it would not catch: a *data*-dependent stall in the stream, since
/// the keystream does not depend on the data at all — the plaintext in
/// the stream half of this test is the same every time on purpose, and
/// the keys are what vary.
#[test]
fn chacha20_takes_the_same_cycles_whatever_the_key_is() {
    let keys: Vec<[u8; 32]> = {
        let mut out: Vec<[u8; 32]> = vec![[0x00; 32], [0xff; 32], [0xaa; 32], [0x55; 32]];
        let mut counting = [0u8; 32];
        for (i, byte) in counting.iter_mut().enumerate() {
            *byte = octet(i as u64);
        }
        out.push(counting);
        out.push(counting.map(|b| !b));
        let mut first = [0u8; 32];
        first[0] = 0x80;
        out.push(first);
        let mut last = [0u8; 32];
        last[31] = 0x01;
        out.push(last);
        let mut spread = [0u8; 32];
        for (i, byte) in spread.iter_mut().enumerate() {
            *byte = octet((i as u64).wrapping_mul(37).wrapping_add(11));
        }
        out.push(spread);
        out
    };

    // The block function.
    let design = design_of("chacha20", "chacha20_core", &[]);
    let mut sim = simulate(&design, "chacha20_core");
    let bench = ChaChaCore::attach(&mut sim);

    let nonces: [[u8; 12]; 3] = [
        [0; 12],
        [0xff; 12],
        [0, 0, 0, 0x09, 0, 0, 0, 0x4a, 0, 0, 0, 0],
    ];
    let counters = [0u32, 1, 0xFFFF_FFFF];

    let mut blocks: BTreeSet<String> = BTreeSet::new();
    let mut measured = 0usize;
    for key in &keys {
        for nonce in &nonces {
            for counter in counters {
                let (block, _, cycles) = bench.block_of(&mut sim, key, nonce, counter);
                assert_eq!(
                    cycles,
                    22,
                    "chacha20_core took {cycles} cycles for key {}",
                    hex(key)
                );
                blocks.insert(hex(&block));
                measured += 1;
            }
        }
    }
    assert_eq!(measured, keys.len() * nonces.len() * counters.len());
    assert_eq!(
        blocks.len(),
        measured,
        "the {measured} inputs should give {measured} different blocks, so the equal \
         cycle counts above are not the cycle counts of one repeated run"
    );

    // And the stream, over a message that is exactly four blocks, so the
    // cycle count printed below divides by the block.
    let design = design_of("chacha20", "chacha20", &[]);
    let mut sim = simulate(&design, "chacha20");
    let bench = ChaChaStream::attach(&mut sim);
    let plain = crypto_pattern(256);
    let mut counts: BTreeSet<u64> = BTreeSet::new();
    let mut ciphers: BTreeSet<String> = BTreeSet::new();
    for key in &keys {
        let (cipher, cycles) = bench.apply(&mut sim, key, &nonces[2], 7, &plain);
        counts.insert(cycles);
        ciphers.insert(hex(&cipher));
    }
    assert_eq!(
        counts.len(),
        1,
        "the stream took {counts:?} cycles over nine different keys"
    );
    assert_eq!(ciphers.len(), keys.len(), "nine keys, nine ciphertexts");
    // Printed and not asserted, because it is the throughput figure the
    // README quotes and a number of cycles is not a number of seconds.
    // 256 bytes is 64 words, which is four whole blocks.
    println!(
        "chacha20: {} cycles for {} bytes",
        counts.iter().next().expect("one count"),
        plain.len()
    );
}

/// The counter runs out and the block stops instead of repeating itself.
///
/// RFC 8439 §2.3's block counter is 32 bits, so a (key, nonce) pair is
/// good for 2^32 blocks and no more. Wrapping it would hand out the same
/// keystream twice, which for a stream cipher is the end of the
/// confidentiality of both messages — so this block latches `exhausted`,
/// stops accepting data, and makes the caller say `start` again with a
/// new nonce.
///
/// The test starts the stream at 0xFFFFFFFE, which gives it exactly two
/// blocks, and checks that the first 128 bytes come out, that the 129th
/// word is **not** accepted, and that `exhausted` is high. Starting two
/// below the top is also the only place in this file where the counter
/// carries from 0xFF to 0x00 in its low byte, so it is the test that
/// would catch a counter incremented a byte at a time.
///
/// What it would not catch: an `exhausted` that latches one block early
/// or late by more than one — the two-block window here pins it to
/// within one block, and a wider window would take 2^32 blocks to check.
#[test]
fn chacha20_stops_rather_than_repeat_its_keystream() {
    let design = design_of("chacha20", "chacha20", &[]);
    let mut sim = simulate(&design, "chacha20");
    let bench = ChaChaStream::attach(&mut sim);

    let key = [0x42u8; 32];
    let nonce = [0x17u8; 12];

    // Two blocks' worth, which is all there is from 0xFFFFFFFE. The
    // sixteenth word of the second block is the last the counter can pay
    // for, so `exhausted` latches on the edge that word moves on — which
    // is the edge this keystream's last four bytes were registered on.
    let (two, _) = bench.apply(&mut sim, &key, &nonce, 0xFFFF_FFFE, &[0u8; 128]);
    assert_eq!(two.len(), 128);

    // One word more. It is never accepted.
    sim.set(bench.in_valid, bit(true));
    sim.set(bench.in_data, word(32, 0));
    for _ in 0..200 {
        assert!(
            !high(&sim, bench.in_ready),
            "chacha20 accepted a word past block 2^32"
        );
        cycle(&mut sim, bench.clk, HALF);
    }
    sim.set(bench.in_valid, bit(false));
    assert!(high(&sim, bench.exhausted), "and it says why");

    // The two blocks really were blocks 0xFFFFFFFE and 0xFFFFFFFF and
    // not the same block twice, which is the thing being prevented.
    assert_ne!(two[..64], two[64..], "two different keystream blocks");

    // `start` clears it.
    let (again, _) = bench.apply(&mut sim, &key, &nonce, 0, &[0u8; 64]);
    assert_eq!(again.len(), 64);
    assert!(!high(&sim, bench.exhausted), "`start` clears it");
}

/// **No secret-dependent addressing: there is nothing to address.**
///
/// Neither crypto block contains a memory array at all. SHA-256's one
/// table, the sixty-four K constants, is a `case` over the round counter
/// and becomes logic; ChaCha20 has no table of any kind, which is one of
/// the reasons it is in this round and AES is not.
///
/// What this establishes: no lookup table in either block can be indexed
/// by a secret byte, because there is no indexable storage for one to
/// index. That is a structural fact about the synthesised design rather
/// than a reading of the source.
///
/// What it does **not** establish: that no *multiplexer* is selected by a
/// secret. A one-hot mux over sixteen words whose select comes from key
/// material would pass this test and would be a timing side channel in a
/// netlist even though it is not a memory. Nothing in either block does
/// that — every select here is a counter — but this test does not prove
/// it, and no test in this file does. It would take a taint analysis from
/// the key ports forward, which Reticle does not have; §5 of both READMEs
/// records that as the gap it is.
#[test]
fn crypto_blocks_hold_no_memory_to_index() {
    for variant in VARIANTS {
        if !matches!(variant.package, "sha256" | "chacha20") {
            continue;
        }
        let (mut design, id) = flattened(variant.package, variant.top, variant.params);
        let mut diags = Diagnostics::new();
        synth_run(&mut design, &SynthOptions::default(), &mut diags);
        assert!(
            !diags.has_errors(),
            "{}.{} does not synthesise",
            variant.package,
            variant.top
        );
        let module = design.module(id);
        let memories: Vec<String> = module
            .memories
            .iter()
            .map(|(_, m)| format!("{} x {}", m.size, m.elem.width().unwrap_or(0)))
            .collect();
        assert!(
            memories.is_empty(),
            "{}.{} holds {} memory array(s): {}",
            variant.package,
            variant.top,
            memories.len(),
            memories.join(", ")
        );
    }
}

// ---------------------------------------------------------------------------
// The library as an index
// ---------------------------------------------------------------------------

/// Every `reticle.ip` under `ip/`, as `ip::library` wants them: paths
/// relative to a project that declares `library <root>`, text as read.
///
/// The walk itself is `tests/library_walk`, shared with the example
/// tests, and it is the walk `reticle build` performs
/// (`library_manifests` in `src/bin/reticle/main.rs`): a directory
/// holding a manifest is a package and is not descended into, every
/// level is sorted, and nothing hidden is entered. It is in a test and
/// not in the library because walking a directory is I/O.
fn walk_library(root: &str) -> Vec<(String, String)> {
    library_walk::manifests(&ip_dir(), root)
}
/// The real library, indexed by name.
///
/// This is the test that fails if a reorganisation of `ip/` ever copies
/// a package instead of moving it, or renames a directory without its
/// manifest: the index is built from the declared names, so two `uart`s
/// or a package nobody can name shows up here and nowhere else. It was
/// written one round before the move into category folders and it is
/// what that move was checked with.
///
/// What it would catch: a duplicate name anywhere under `ip/`, a manifest
/// with no `name`, a `depends` naming a package the library does not
/// have, and a package whose name does not resolve back to its own
/// directory. What it would not catch: a package whose *contents* are
/// wrong, which is what the rest of this file is for, and anything about
/// `path` dependencies, which no block uses.
#[test]
fn every_block_is_findable_by_the_name_it_declares() {
    let manifests = walk_library("ip");
    let index = reticle::ip::LibraryIndex::from_manifests(["ip".to_owned()], manifests.clone());

    assert!(
        index.duplicates().is_empty(),
        "two packages in ip/ claim one name: {:?}",
        index.duplicates()
    );
    assert!(
        index.unnamed().is_empty(),
        "a manifest under ip/ declares no name: {:?}",
        index.unnamed()
    );
    assert_eq!(
        index.entries().len(),
        manifests.len(),
        "a manifest was walked but not indexed"
    );

    // Every name resolves, and resolves to the directory its own
    // manifest sits in.
    for entry in index.entries() {
        let found = index
            .lookup(&entry.name)
            .unwrap_or_else(|e| panic!("`{}` does not resolve: {e:?}", entry.name));
        assert_eq!(found.manifest, entry.manifest);
        assert!(
            found.manifest.starts_with(&format!("{}/", found.dir)),
            "{} is not under {}",
            found.manifest,
            found.dir
        );
        // The layout: `ip/<category>/<package>`, with the folder named
        // after the package it holds. Nothing in the resolver requires
        // either — a block is found by its declared name at any depth —
        // so this is the test that says the library is laid out the way
        // `docs/ip-library.md` draws it.
        let parts: Vec<&str> = found.dir.split('/').collect();
        assert_eq!(
            parts.len(),
            3,
            "{} is not ip/<category>/<package>",
            found.dir
        );
        assert_eq!(parts[0], "ip");
        assert!(
            CATEGORIES.contains(&parts[1]),
            "`{}` is not one of the eight categories {CATEGORIES:?}",
            parts[1]
        );
        assert_eq!(
            parts[2], entry.name,
            "{} is not named for {}",
            found.dir, entry.name
        );
    }

    // And every dependency any block states is a name the library has,
    // which is what makes a project able to drop every `path`.
    for variant in VARIANTS {
        for dep in &manifest(variant.package).depends {
            index
                .lookup(&dep.name)
                .unwrap_or_else(|e| panic!("`{}` needs `{}`: {e:?}", variant.package, dep.name));
        }
    }
}

/// What building that index costs, printed and never asserted.
///
/// Run with `cargo test --all-features --test ip_library -- --ignored
/// --nocapture the_library_index_cost`. It exists because the ECP5
/// database load was thirteen seconds of a fourteen-second build before
/// anyone measured it, and the honest way to know whether an index needs
/// caching is to time the thing rather than to reason about it.
///
/// No assertion: a number of seconds is a property of the machine, and
/// `CLAUDE.md` says so.
#[test]
#[ignore = "prints a measurement rather than asserting one"]
fn the_library_index_cost() {
    use std::time::Instant;

    const ROUNDS: u32 = 20;

    let started = Instant::now();
    let mut walked = 0;
    for _ in 0..ROUNDS {
        walked = walk_library("ip").len();
    }
    let walk = started.elapsed() / ROUNDS;

    let manifests = walk_library("ip");
    let bytes: usize = manifests.iter().map(|(_, text)| text.len()).sum();
    let started = Instant::now();
    for _ in 0..ROUNDS {
        let index = reticle::ip::LibraryIndex::from_manifests(["ip".to_owned()], manifests.clone());
        assert!(!index.is_empty());
    }
    let build = started.elapsed() / ROUNDS;

    println!(
        "ip/: {walked} packages, {bytes} bytes of manifest\n  \
         walk and read: {walk:?} per run\n  \
         index (clone, scan, sort): {build:?} per run"
    );
}

// ---------------------------------------------------------------------------
// ip/compress/inflate — RFC 1951 DEFLATE and RFC 1950 zlib
// ---------------------------------------------------------------------------

/// The plaintext rules the corpus names, implemented the same way the
/// out-of-tree generator implemented them.
///
/// The corpus stores a stream and a *rule*, not a plaintext, which is the
/// trick that lets one case be a hundred thousand bytes of highly
/// compressible text for a kilobyte of committed hex. The cost is that
/// this function and the generator's have to agree, and the `adler` field
/// of each record is what catches it when they do not:
/// `inflate_plaintext_rules_match_the_corpus` checks every one of them
/// before any simulation happens, so a rule that has drifted is reported
/// as a rule that has drifted rather than as a broken decompressor.
fn plaintext(rule: &str, len: usize) -> Vec<u8> {
    const WORDS: [&str; 16] = [
        "the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "and", "then", "it",
        "runs", "away", "into", "deep", "forest",
    ];
    let mut out = Vec::with_capacity(len);
    let (kind, arg) = match rule.split_once(':') {
        Some((k, a)) => (k, a.parse::<u32>().expect("a rule argument")),
        None => (rule, 0),
    };
    match kind {
        "zeros" => out.resize(len, 0u8),
        "ones" => out.resize(len, 0xffu8),
        // The same rule `sha256`'s padding table uses, done in `u8` so
        // that the modulo 256 is the arithmetic rather than a cast.
        "pattern" => {
            let mut b = 13u8;
            for _ in 0..len {
                out.push(b);
                b = b.wrapping_add(7);
            }
        }
        "counter" => {
            let mut b = 0u8;
            for _ in 0..len {
                out.push(b);
                b = b.wrapping_add(1);
            }
        }
        // Incompressible, so the encoder reaches for a stored block. The
        // byte is bits 16 to 23 of the state, which is the low byte of
        // `x >> 16`.
        "lcg" => {
            let mut x = arg | 1;
            for _ in 0..len {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                out.push((x >> 16).to_le_bytes()[0]);
            }
        }
        // `pattern` modulo a period, so every match is at that distance.
        "rep" => {
            let period: Vec<u8> = plaintext("pattern", usize::try_from(arg).expect("a period"));
            out.extend((0..len).map(|i| period[i % period.len()]));
        }
        // English-shaped text: what a dynamic Huffman block is for.
        "text" => {
            let mut x = arg | 1;
            while out.len() < len {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let w = usize::try_from((x >> 20) % 16).expect("a word index");
                out.extend_from_slice(WORDS[w].as_bytes());
                out.push(b' ');
            }
            out.truncate(len);
        }
        // Runs of one byte, 1 to 64 long: many short distances.
        "runs" => {
            let mut x = arg | 1;
            while out.len() < len {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let b = (x >> 16).to_le_bytes()[0];
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let n = (x >> 20) % 64 + 1;
                for _ in 0..n {
                    out.push(b);
                }
            }
            out.truncate(len);
        }
        other => panic!("unknown plaintext rule `{other}`"),
    }
    assert_eq!(out.len(), len, "rule `{rule}` produced the wrong length");
    out
}

/// RFC 1950 §9, in Rust, for the corpus's own bookkeeping.
fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// One record of `testdata/ip/inflate_corpus.txt`.
struct Corpus {
    name: String,
    /// `fast` for the cases the gate runs, `bulk` for the big ones.
    tier: String,
    /// `zlib` or `deflate`, which is what `WRAPPER` has to be set to.
    wrapper: String,
    rule: String,
    len: usize,
    adler: u32,
    /// BFINAL and BTYPE of the first block, which is as much as can be
    /// read off a stream without an inflater.
    first: (bool, u8),
    note: String,
    stream: Vec<u8>,
}

fn corpus() -> &'static [Corpus] {
    static CORPUS: OnceLock<Vec<Corpus>> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ip/inflate_corpus.txt");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
            .replace("\r\n", "\n");
        let mut out: Vec<Corpus> = Vec::new();
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (tag, rest) = line.split_once(' ').expect("a tagged line");
            let mut field = rest.split_whitespace();
            match tag {
                "case" => {
                    let name = field.next().expect("a name").to_owned();
                    let tier = field.next().expect("a tier").to_owned();
                    let wrapper = field.next().expect("a wrapper").to_owned();
                    out.push(Corpus {
                        name,
                        tier,
                        wrapper,
                        rule: String::new(),
                        len: 0,
                        adler: 0,
                        first: (false, 0),
                        note: String::new(),
                        stream: Vec::new(),
                    });
                }
                "gen" => {
                    let case = out.last_mut().expect("a case first");
                    case.rule = field.next().expect("a rule").to_owned();
                    case.len = field.next().expect("a length").parse().expect("a number");
                    case.adler =
                        u32::from_str_radix(field.next().expect("an adler"), 16).expect("hex");
                }
                "first" => {
                    let case = out.last_mut().expect("a case first");
                    let bfinal = field.next().expect("bfinal") == "1";
                    let btype: u8 = field.next().expect("btype").parse().expect("a number");
                    case.first = (bfinal, btype);
                }
                "note" => out.last_mut().expect("a case first").note = rest.to_owned(),
                "hex" => {
                    let case = out.last_mut().expect("a case first");
                    let digits = field.next().expect("hex digits").as_bytes();
                    assert!(digits.len() % 2 == 0, "{}: odd hex run", case.name);
                    for pair in digits.chunks(2) {
                        let s = std::str::from_utf8(pair).expect("ascii");
                        case.stream
                            .push(u8::from_str_radix(s, 16).expect("a hex byte"));
                    }
                }
                other => panic!("unknown corpus tag `{other}`"),
            }
        }
        assert!(out.len() > 100, "only {} corpus cases", out.len());
        out
    })
}

/// A DEFLATE bit writer, for the streams no compressor will produce.
///
/// RFC 1951 §3.1.1 packs everything but Huffman codes least significant
/// bit first, and Huffman codes most significant bit first. Both are
/// here, because a hand-built vector needs both and getting one of them
/// backwards is the classic way to waste an afternoon.
struct BitWriter {
    bytes: Vec<u8>,
    bit: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            bytes: Vec::new(),
            bit: 0,
        }
    }

    /// `n` bits of `value`, least significant first: §3.1.1's order for
    /// block headers, lengths and extra bits.
    fn bits(&mut self, value: u32, n: u32) {
        for i in 0..n {
            if self.bit == 0 {
                self.bytes.push(0);
            }
            if (value >> i) & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << self.bit;
            }
            self.bit = (self.bit + 1) & 7;
        }
    }

    /// `n` bits of `code`, most significant first: §3.1.1's order for a
    /// Huffman code, "packed starting with the most significant bit of
    /// the code".
    fn code(&mut self, code: u32, n: u32) {
        for i in (0..n).rev() {
            self.bits((code >> i) & 1, 1);
        }
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
        }
    }

    fn byte(&mut self, value: u8) {
        assert_eq!(self.bit, 0, "a byte must be written aligned");
        self.bytes.push(value);
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// §3.2.6's fixed literal/length code, as (code, bits).
fn fixed_lit_code(symbol: u32) -> (u32, u32) {
    match symbol {
        0..=143 => (0x30 + symbol, 8),
        144..=255 => (0x190 + symbol - 144, 9),
        256..=279 => (symbol - 256, 7),
        280..=287 => (0xc0 + symbol - 280, 8),
        _ => panic!("no fixed code for {symbol}"),
    }
}

/// Everything a testbench needs to hold of `inflate`.
struct InflateBench {
    clk: NetHandle,
    rst_n: NetHandle,
    start: NetHandle,
    in_byte: NetHandle,
    in_valid: NetHandle,
    in_last: NetHandle,
    in_ready: NetHandle,
    out_byte: NetHandle,
    out_valid: NetHandle,
    out_ready: NetHandle,
    done: NetHandle,
    error: NetHandle,
    error_code: NetHandle,
}

/// How the consumer behaves, which is the whole of the back-pressure
/// story and so the whole of the stall-mid-copy story.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sink {
    /// `out_ready` always high: the block runs at its own rate.
    Greedy,
    /// High on every other cycle, which guarantees a stall inside every
    /// copy longer than one byte.
    Alternate,
    /// High when a counter says so, from a fixed sequence: an uneven
    /// pattern that stalls at positions no round number would reach.
    Ragged(u32),
    /// Low for `n` cycles after each byte taken.
    Slow(u32),
}

/// What one run produced.
#[derive(Debug)]
struct Run {
    /// The bytes the block handed over, however the run ended.
    out: Vec<u8>,
    /// `done` came up with everything delivered.
    done: bool,
    /// `error` came up, with its code.
    error: Option<u8>,
    cycles: u64,
}

impl InflateBench {
    fn attach(sim: &mut Simulator<'_>) -> Self {
        let bench = InflateBench {
            clk: top_net(sim, "clk"),
            rst_n: top_net(sim, "rst_n"),
            start: top_net(sim, "start"),
            in_byte: top_net(sim, "in_byte"),
            in_valid: top_net(sim, "in_valid"),
            in_last: top_net(sim, "in_last"),
            in_ready: top_net(sim, "in_ready"),
            out_byte: top_net(sim, "out_byte"),
            out_valid: top_net(sim, "out_valid"),
            out_ready: top_net(sim, "out_ready"),
            done: top_net(sim, "done"),
            error: top_net(sim, "error"),
            error_code: top_net(sim, "error_code"),
        };
        sim.set(bench.start, bit(false));
        sim.set(bench.in_valid, bit(false));
        sim.set(bench.in_last, bit(false));
        sim.set(bench.in_byte, word(8, 0));
        sim.set(bench.out_ready, bit(false));
        reset(sim, bench.clk, bench.rst_n);
        bench
    }

    /// Decompresses one stream.
    ///
    /// `limit` is a **loop bound**, not a time-out: it is a count of
    /// simulated clock edges, so it is the same number on every machine,
    /// and reaching it is a failure of the design rather than of the
    /// host. Every state of this block either consumes an input bit,
    /// produces an output byte, or ends on a counter, so a stream that
    /// reaches the bound is one that has found a loop that does none of
    /// those — which is the defect this bound exists to catch.
    fn run(&self, sim: &mut Simulator<'_>, stream: &[u8], sink: Sink, limit: u64) -> Run {
        sim.set(self.start, bit(true));
        sim.set(self.in_valid, bit(false));
        sim.set(self.in_last, bit(false));
        sim.set(self.out_ready, bit(false));
        cycle(sim, self.clk, HALF);
        sim.set(self.start, bit(false));

        let mut index = 0usize;
        let mut ended = false;
        let mut out: Vec<u8> = Vec::new();
        let mut cycles = 0u64;
        let mut hold = 0u32;
        loop {
            if ended {
                sim.set(self.in_valid, bit(false));
                sim.set(self.in_last, bit(false));
            } else {
                let more = index < stream.len();
                sim.set(self.in_valid, bit(more));
                sim.set(
                    self.in_byte,
                    word(8, if more { u64::from(stream[index]) } else { 0 }),
                );
                // Like `sha256`'s: `in_last` marks the point the stream
                // ends at, and with `in_valid` low it ends it with no
                // byte here — which is how an empty stream is spelled.
                sim.set(self.in_last, bit(index + 1 >= stream.len()));
            }

            let want = match sink {
                Sink::Greedy => true,
                Sink::Alternate => cycles & 1 == 0,
                // A 23-step cycle, so it does not line up with anything
                // in the block: 23 is prime and longer than the longest
                // run of states between two output bytes.
                Sink::Ragged(seed) => ((cycles + u64::from(seed)) * 7 % 23) < 9,
                Sink::Slow(_) => hold == 0,
            };
            sim.set(self.out_ready, bit(want));

            // Both handshakes are driven by registers, so what they read
            // here is what the last edge produced and nothing set above
            // can have disturbed them.
            let accepted = !ended && high(sim, self.in_ready);
            let handed = want && high(sim, self.out_valid);
            let byte = if handed {
                Some(octet(get_u64(sim, self.out_byte)))
            } else {
                None
            };

            cycle(sim, self.clk, HALF);
            cycles += 1;

            if accepted {
                if index + 1 >= stream.len() {
                    ended = true;
                } else {
                    index += 1;
                }
            }
            if let Some(b) = byte {
                out.push(b);
                if let Sink::Slow(n) = sink {
                    hold = n;
                }
            } else {
                hold = hold.saturating_sub(1);
            }

            if high(sim, self.error) {
                return Run {
                    out,
                    done: false,
                    error: Some(octet(get_u64(sim, self.error_code))),
                    cycles,
                };
            }
            if high(sim, self.done) {
                return Run {
                    out,
                    done: true,
                    error: None,
                    cycles,
                };
            }
            assert!(
                cycles < limit,
                "inflate neither finished nor failed in {limit} cycles: \
                 {} of {} input bytes taken, {} output bytes",
                index,
                stream.len(),
                out.len()
            );
        }
    }
}

/// A loop bound for a stream of `inlen` bytes expected to produce
/// `outlen`.
///
/// The worst case per input byte is a dynamic block header: three
/// counting sorts, each at most 16 + 290 + 15 + 290 cycles, over the
/// fifteen-odd bytes the smallest dynamic header takes — call it 200 a
/// byte. The worst case per output byte is fifteen bits of Huffman code
/// plus a cycle, so twenty. Neither is tight; both are numbers rather
/// than seconds.
fn inflate_limit(inlen: usize, outlen: usize) -> u64 {
    2000 + 200 * inlen as u64 + 20 * outlen as u64
}

fn inflate_design(window_bits: u32, wrapper: u32) -> Design {
    design_of(
        "inflate",
        "inflate",
        &[
            ("WINDOW_BITS", &window_bits.to_string()),
            ("WRAPPER", &wrapper.to_string()),
        ],
    )
}

/// The plaintext rules in this file are the generator's rules.
///
/// What it would catch: a rule here that has drifted from the one the
/// corpus was built against — which would otherwise show up as every
/// case of that rule failing, and look like a decompressor fault.
///
/// What it would not catch: anything about the decompressor at all. It
/// does not simulate. It is the check that makes every failure below
/// attributable.
#[test]
fn inflate_plaintext_rules_match_the_corpus() {
    let mut rules: BTreeSet<&str> = BTreeSet::new();
    for case in corpus() {
        let plain = plaintext(&case.rule, case.len);
        assert_eq!(
            adler32(&plain),
            case.adler,
            "{}: the `{}` rule at {} bytes no longer makes what the corpus was built from",
            case.name,
            case.rule,
            case.len
        );
        rules.insert(case.rule.split(':').next().expect("a rule name"));
    }
    // Every rule the generator has is used, so none of them is dead
    // code pretending to be coverage.
    let expected: BTreeSet<&str> = [
        "counter", "lcg", "ones", "pattern", "rep", "runs", "text", "zeros",
    ]
    .into_iter()
    .collect();
    assert_eq!(rules, expected, "the corpus no longer uses every rule");
}

/// RFC 1950 §9's checksum, on its own.
///
/// §9 gives the algorithm and no vectors, so the expectations are what
/// the Rust `adler32` above computes — a second implementation of the
/// same six lines, which is weak as oracles go. What makes it worth
/// having is the *cases*: an input long enough to need the reduction on
/// `a` and one long enough to need it on `b`, which is where a
/// hand-written modular sum goes wrong.
///
/// What it would catch: a wrong initial value (1 and 0, not 0 and 0), a
/// wrong BASE, a reduction that subtracts when it should not or not when
/// it should, and the two sums swapped in `sum`.
///
/// What it would not catch: a reduction that is wrong only for inputs
/// longer than 8192 bytes of 0xff — the bound where `b` could in
/// principle need two subtractions if `a` were not reduced first. The
/// module header argues it cannot, and `inflate_decompresses_the_compcol_corpus`
/// checks the whole 100 000-byte case through the real trailer.
#[test]
fn inflate_adler_matches_rfc_1950_9() {
    let design = design_of("inflate", "inflate_adler", &[]);
    let mut sim = simulate(&design, "inflate_adler");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let start = top_net(&sim, "start");
    let in_byte = top_net(&sim, "in_byte");
    let in_valid = top_net(&sim, "in_valid");
    let sum = top_net(&sim, "sum");
    sim.set(start, bit(false));
    sim.set(in_valid, bit(false));
    sim.set(in_byte, word(8, 0));
    reset(&mut sim, clk, rst_n);

    // The empty string is the reset value, which §9's "s1 = 1" makes 1.
    assert_eq!(get_u64(&sim, sum), 1, "the empty string is not 1");

    let cases: [(&str, Vec<u8>); 6] = [
        ("one byte", vec![0x61]),
        ("\"abc\"", b"abc".to_vec()),
        ("256 bytes of 0xff", vec![0xff; 256]),
        // 258 bytes of 0xff puts `a` over 65521 and so needs its
        // reduction; `b` passes 65521 long before.
        ("1024 bytes of 0xff", vec![0xff; 1024]),
        ("a counting pattern", plaintext("counter", 1000)),
        ("text", plaintext("text:1", 777)),
    ];
    for (what, data) in cases {
        sim.set(start, bit(true));
        cycle(&mut sim, clk, HALF);
        sim.set(start, bit(false));
        for byte in &data {
            sim.set(in_byte, word(8, u64::from(*byte)));
            sim.set(in_valid, bit(true));
            cycle(&mut sim, clk, HALF);
        }
        sim.set(in_valid, bit(false));
        cycle(&mut sim, clk, HALF);
        assert_eq!(
            narrow(get_u64(&sim, sum)),
            adler32(&data),
            "{what}: {} bytes",
            data.len()
        );
    }
}

/// The window refuses a distance it cannot answer, and forwards a byte
/// it is reading in the cycle it writes it.
///
/// This is the only test in the library that drives `dist_bad` directly,
/// and it exists because no *stream* can ask for distance 0: RFC 1951
/// §3.2.5's smallest distance code is 1, so the arm of the check that
/// refuses zero is unreachable from `inflate.v`. A module boundary is
/// what makes it testable at all, and that is most of why
/// `inflate_window` is a module.
///
/// What it would catch: a distance check that compares against the
/// window size rather than against the bytes actually written, so that a
/// fresh stream could read whatever the last one left in the block RAM;
/// an off-by-one at the two ends of the range (`dist == n` legal,
/// `dist == n + 1` not, and `dist == 1 << WINDOW_BITS` legal once full);
/// a missing write-forward, which shows up as the second byte of a
/// distance-1 run being stale.
///
/// What it would not catch: anything about *where* `inflate.v` asserts
/// `copy_open`, which is the other half of the contract — that is
/// `inflate_refuses_a_distance_behind_the_start_of_the_stream`'s job.
#[test]
fn inflate_window_refuses_a_distance_it_does_not_hold() {
    // A 256-byte window, so "full" is reachable in a readable number of
    // cycles and the wrap is a real wrap.
    let design = design_of("inflate", "inflate_window", &[("WINDOW_BITS", "8")]);
    let mut sim = simulate(&design, "inflate_window");
    let clk = top_net(&sim, "clk");
    let rst_n = top_net(&sim, "rst_n");
    let start = top_net(&sim, "start");
    let wr_data = top_net(&sim, "wr_data");
    let wr_en = top_net(&sim, "wr_en");
    let dist = top_net(&sim, "dist");
    let dist_bad = top_net(&sim, "dist_bad");
    let copy_open = top_net(&sim, "copy_open");
    let rd_en = top_net(&sim, "rd_en");
    let rd_take = top_net(&sim, "rd_take");
    let rd_data = top_net(&sim, "rd_data");
    let rd_valid = top_net(&sim, "rd_valid");

    let idle = |sim: &mut Simulator<'_>| {
        sim.set(wr_en, bit(false));
        sim.set(copy_open, bit(false));
        sim.set(rd_en, bit(false));
        sim.set(rd_take, bit(false));
    };
    idle(&mut sim);
    sim.set(start, bit(false));
    sim.set(dist, word(16, 0));
    sim.set(wr_data, word(8, 0));
    reset(&mut sim, clk, rst_n);

    // Nothing written: every distance is out of range, zero included.
    for d in [0u64, 1, 2, 255, 256, 257, 32768] {
        sim.set(dist, word(16, d));
        sim.run_for(1);
        assert!(
            high(&sim, dist_bad),
            "an empty window accepted distance {d}"
        );
    }

    // Write n bytes; distances 1..=n are in range and n+1 is not.
    let mut written = 0u64;
    let push = |sim: &mut Simulator<'_>, byte: u8| {
        sim.set(wr_data, word(8, u64::from(byte)));
        sim.set(wr_en, bit(true));
        cycle(sim, clk, HALF);
        sim.set(wr_en, bit(false));
    };
    for step in [1u64, 1, 1, 7, 50, 196] {
        for _ in 0..step {
            push(&mut sim, octet(written & 0xff));
            written += 1;
        }
        let have = written.min(256);
        for (d, bad) in [(0u64, true), (1, false), (have, false), (have + 1, true)] {
            sim.set(dist, word(16, d));
            sim.run_for(1);
            assert_eq!(
                high(&sim, dist_bad),
                bad,
                "after {written} bytes, distance {d} should be {}",
                if bad { "refused" } else { "accepted" }
            );
        }
    }
    assert_eq!(written, 256, "the window should be exactly full");
    // Full: the whole window is in range and one more is not. A distance
    // of exactly the window length is the oldest byte, which is the byte
    // about to be overwritten, and is legal.
    for (d, bad) in [(256u64, false), (257, true), (32768, true)] {
        sim.set(dist, word(16, d));
        sim.run_for(1);
        assert_eq!(high(&sim, dist_bad), bad, "when full, distance {d}");
    }
    // And after a wrap it stays full rather than going back to counting.
    push(&mut sim, 0xaa);
    sim.set(dist, word(16, 256));
    sim.run_for(1);
    assert!(!high(&sim, dist_bad), "a wrapped window forgot it was full");

    // Reading a run at distance 1, with a write to the same address in
    // the same cycle. Without the bypass the second byte of the run is
    // whatever the memory held before.
    sim.set(dist, word(16, 1));
    sim.set(copy_open, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(copy_open, bit(false));
    let mut run: Vec<u8> = Vec::new();
    // Prime, then read and write together for as long as the copy lasts.
    sim.set(rd_en, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(rd_en, bit(false));
    for _ in 0..6 {
        assert!(high(&sim, rd_valid), "the window stopped answering");
        let byte = octet(get_u64(&sim, rd_data));
        run.push(byte);
        sim.set(wr_data, word(8, u64::from(byte)));
        sim.set(wr_en, bit(true));
        sim.set(rd_en, bit(true));
        sim.set(rd_take, bit(true));
        cycle(&mut sim, clk, HALF);
        sim.set(wr_en, bit(false));
        sim.set(rd_en, bit(false));
        sim.set(rd_take, bit(false));
    }
    idle(&mut sim);
    assert_eq!(
        run,
        vec![0xaa; 6],
        "a distance-1 run read stale bytes: the write bypass is not working"
    );

    // And a byte nobody has taken survives being left there.
    //
    // `Sink::Alternate` found this as a wrong fourth byte on a four-byte
    // stream: `rd_valid` used to be `rd_en` delayed by a cycle, so a copy
    // that stalled for one cycle lost the byte it had already fetched and
    // the stream came out one byte short with a checksum failure. The
    // same arithmetic that makes the forwarded byte right also has to
    // make it *keep* being right.
    sim.set(dist, word(16, 3));
    sim.set(copy_open, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(copy_open, bit(false));
    sim.set(rd_en, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(rd_en, bit(false));
    assert!(high(&sim, rd_valid), "the primed read never arrived");
    let held = octet(get_u64(&sim, rd_data));
    for n in 0..5 {
        cycle(&mut sim, clk, HALF);
        assert!(
            high(&sim, rd_valid),
            "the window dropped an untaken byte after {} idle cycles",
            n + 1
        );
        assert_eq!(
            octet(get_u64(&sim, rd_data)),
            held,
            "the window changed an untaken byte after {} idle cycles",
            n + 1
        );
    }
    // Taking it, with no new read, clears the valid and nothing else.
    sim.set(rd_take, bit(true));
    cycle(&mut sim, clk, HALF);
    sim.set(rd_take, bit(false));
    assert!(!high(&sim, rd_valid), "a taken byte stayed valid");
    idle(&mut sim);
}

/// The corpus, decompressed byte for byte.
///
/// This is the measurement the whole round rests on. 123 streams that
/// `compcol` — the user's own from-scratch Rust compression library,
/// driven out of tree and read only — produced from eight documented
/// plaintext rules at lengths from 0 to 4096, each in both framings
/// (RFC 1950 and raw RFC 1951) and at compression levels 1, 6 and 9.
/// `ip/compress/inflate/README.md` §3 says how the cases were chosen and
/// what each group reaches.
///
/// Each case is run four times with four different consumers, because
/// **the consumer is the only thing that can stall a copy** and a copy
/// that resumes wrongly is the defect this block is most likely to
/// have. `Sink::Greedy` never stalls; `Sink::Alternate` stalls inside
/// every copy of more than one byte; `Sink::Ragged` stalls on a 23-step
/// pattern that lines up with nothing; `Sink::Slow` stalls for three
/// cycles after every byte, which is longer than any single state.
///
/// What it would catch: every wrong length base, distance base and extra
/// bit count of §3.2.5; a wrong code-length permutation or run-length
/// code in §3.2.7; a counting sort that places a symbol at the wrong
/// index; a bit reader that drops or repeats a bit at a byte boundary; a
/// copy that restarts, overshoots or forgets where it was after a stall;
/// a window that reads the wrong byte for a distance; and — because 61
/// of these are zlib-framed — an Adler-32 that disagrees with a real
/// compressor's over tens of thousands of bytes.
///
/// What it would not catch: a distance past 32768, since nothing here is
/// long enough to use one (`inflate_crosses_the_32_kib_window` is the
/// `#[ignore]`d test that is); a malformed stream of any kind, since
/// every one of these is well formed (the three tests after this one);
/// and anything about the clock, since this is a four-state zero-delay
/// simulation.
#[test]
fn inflate_decompresses_the_compcol_corpus() {
    let zlib = inflate_design(15, 1);
    let raw = inflate_design(15, 0);
    let mut zsim = simulate(&zlib, "inflate (zlib)");
    let mut rsim = simulate(&raw, "inflate (raw)");
    let zbench = InflateBench::attach(&mut zsim);
    let rbench = InflateBench::attach(&mut rsim);

    // Four consumers for the small cases and two for the large ones. Every
    // case is stalled by `Alternate`, which is the pattern that found the
    // window's dropped byte; the other two are there to stall at positions
    // a regular pattern cannot reach, and running them over the handful of
    // four-thousand-byte cases as well would double this test's cycles for
    // no new shape of stall.
    const ALL: [Sink; 4] = [
        Sink::Greedy,
        Sink::Alternate,
        Sink::Ragged(5),
        Sink::Slow(3),
    ];
    const BIG: [Sink; 2] = [Sink::Greedy, Sink::Alternate];
    let mut cases = 0usize;
    let mut runs = 0usize;
    let mut bytes = 0u64;
    let mut cycles = 0u64;
    for case in corpus() {
        if case.tier != "fast" {
            continue;
        }
        let plain = plaintext(&case.rule, case.len);
        let (sim, bench) = if case.wrapper == "zlib" {
            (&mut zsim, &zbench)
        } else {
            (&mut rsim, &rbench)
        };
        cases += 1;
        let sinks: &[Sink] = if case.len <= 1100 { &ALL } else { &BIG };
        for sink in sinks.iter().copied() {
            let run = bench.run(
                sim,
                &case.stream,
                sink,
                inflate_limit(case.stream.len(), case.len),
            );
            assert_eq!(
                run.error,
                None,
                "{} with {sink:?}: reported error {:?} after {} of {} bytes",
                case.name,
                run.error,
                run.out.len(),
                case.len
            );
            assert!(run.done, "{} with {sink:?}: never finished", case.name);
            assert_eq!(
                run.out.len(),
                plain.len(),
                "{} with {sink:?}: {} bytes out, {} expected",
                case.name,
                run.out.len(),
                plain.len()
            );
            if run.out != plain {
                let at = run
                    .out
                    .iter()
                    .zip(&plain)
                    .position(|(a, b)| a != b)
                    .expect("a difference");
                panic!(
                    "{} with {sink:?}: first difference at byte {at}: got {:02x}, want {:02x}",
                    case.name, run.out[at], plain[at]
                );
            }
            runs += 1;
            bytes += case.len as u64;
            cycles += run.cycles;
        }
    }
    assert!(cases >= 120, "only {cases} fast corpus cases");
    // All three of §3.2.3's block types, by the first block of some
    // case: a corpus that lost its stored or its dynamic cases would
    // otherwise pass quietly.
    for btype in [0u8, 1, 2] {
        assert!(
            corpus()
                .iter()
                .any(|c| c.tier == "fast" && c.first == (true, btype)),
            "no fast corpus case is a single block of BTYPE {btype}"
        );
    }
    println!(
        "inflate: {cases} corpus cases, {runs} runs, \
         {bytes} output bytes in {cycles} cycles ({:.3} bytes/cycle)",
        bytes as f64 / cycles as f64
    );
}

/// Every malformed stream this block knows how to refuse, refused with
/// the code it is supposed to refuse it with.
///
/// Each case is hand built with `BitWriter` from the sections named,
/// because no compressor will produce any of them. That makes them the
/// one group of vectors here that is *derived from the specification*
/// rather than agreed with a second implementation, and the bit order of
/// §3.1.1 — least significant first for everything but a Huffman code —
/// is what a reader should check them against.
///
/// What it would catch: a missing check, a check that reports the wrong
/// code, and — this is the point — any of these inputs hanging, since
/// `run`'s loop bound fails rather than waiting.
///
/// What it would not catch: a check that fires when it should not, which
/// is `inflate_decompresses_the_compcol_corpus`'s job from the other
/// side; and the one arm of the distance check no stream can reach,
/// which `inflate_window_refuses_a_distance_it_does_not_hold` drives
/// directly.
#[test]
fn inflate_reports_every_malformed_stream() {
    // RFC 1950 §2.2: FCHECK is chosen so that CMF*256 + FLG is a
    // multiple of 31, which is how a header with a *deliberate* fault in
    // one field can keep every other field legal.
    fn zlib_header(cmf: u8, fdict: bool) -> [u8; 2] {
        for low in 0u8..32 {
            let flg = low | if fdict { 0x20 } else { 0 };
            if (u16::from(cmf) * 256 + u16::from(flg)) % 31 == 0 {
                return [cmf, flg];
            }
        }
        panic!("no FCHECK for CMF {cmf:#04x}");
    }

    // Error codes, as `inflate.v` names them.
    const E_HEADER: u8 = 1;
    const E_BTYPE: u8 = 2;
    const E_NLEN: u8 = 3;
    const E_CODE: u8 = 4;
    const E_DIST: u8 = 5;
    const E_TRUNC: u8 = 6;
    const E_ADLER: u8 = 7;

    // A legal one-byte zlib stream, to corrupt.
    let good: Vec<u8> = corpus()
        .iter()
        .find(|c| c.name == "pattern_9_l6_zlib")
        .expect("a short zlib case")
        .stream
        .clone();

    // --- the RFC 1950 §2.2 header ---------------------------------------
    let mut zlib_cases: Vec<(&str, Vec<u8>, u8)> = Vec::new();
    {
        // A check that is not a multiple of 31. 0x78 0x9c is the usual
        // header; xor 1 into FLG and it is not.
        let mut bad = good.clone();
        bad[1] ^= 1;
        zlib_cases.push(("§2.2 FCHECK is wrong", bad, E_HEADER));
    }
    for (what, cmf, fdict) in [
        ("§2.2 CM is 9, not deflate", 0x79u8, false),
        ("§2.2 CINFO is 8, over the 32K maximum", 0x88, false),
        ("§2.2 FDICT asks for a preset dictionary", 0x78, true),
    ] {
        let head = zlib_header(cmf, fdict);
        let mut bad = good.clone();
        bad[0] = head[0];
        bad[1] = head[1];
        zlib_cases.push((what, bad, E_HEADER));
    }
    // --- RFC 1950 §9's trailer ------------------------------------------
    for i in 0..4 {
        let mut bad = good.clone();
        let n = bad.len();
        bad[n - 4 + i] ^= 0x01;
        zlib_cases.push(("§9 the checksum does not match", bad, E_ADLER));
    }
    // A stream with no trailer at all, which is the truncation that
    // would otherwise pass as a complete decode.
    {
        let mut bad = good.clone();
        bad.truncate(bad.len() - 4);
        zlib_cases.push(("§9 the trailer is missing", bad, E_TRUNC));
    }

    // --- raw RFC 1951 ---------------------------------------------------
    let mut raw_cases: Vec<(&str, Vec<u8>, u8)> = Vec::new();

    // §3.2.3: BTYPE 11 is "reserved (error)".
    {
        let mut w = BitWriter::new();
        w.bits(1, 1); // BFINAL
        w.bits(3, 2); // BTYPE 11
        raw_cases.push(("§3.2.3 BTYPE is the reserved 11", w.finish(), E_BTYPE));
    }
    // §3.2.4: NLEN must be the one's complement of LEN.
    {
        let mut w = BitWriter::new();
        w.bits(0, 1);
        w.bits(0, 2); // stored, not final
        w.align();
        w.byte(5);
        w.byte(0); // LEN = 5
        w.byte(0);
        w.byte(0); // NLEN = 0, which is not ~5
        for b in b"hello" {
            w.byte(*b);
        }
        raw_cases.push(("§3.2.4 NLEN is not ~LEN", w.finish(), E_NLEN));
    }
    // §3.2.5: 286 and 287 "will never actually occur", and the fixed
    // code of §3.2.6 nevertheless has codes for them.
    for symbol in [286u32, 287] {
        let mut w = BitWriter::new();
        w.bits(1, 1);
        w.bits(1, 2); // fixed
        let (code, bits) = fixed_lit_code(symbol);
        w.code(code, bits);
        raw_cases.push(("§3.2.5 a length symbol of 286 or 287", w.finish(), E_CODE));
    }
    // §3.2.6: distance symbols 30 and 31 likewise.
    for symbol in [30u32, 31] {
        let mut w = BitWriter::new();
        w.bits(1, 1);
        w.bits(1, 2);
        let (code, bits) = fixed_lit_code(b'A'.into());
        w.code(code, bits); // one literal, so the window is not empty
        let (code, bits) = fixed_lit_code(257); // length 3
        w.code(code, bits);
        w.code(symbol, 5); // the fixed distance code is 5 bits flat
        raw_cases.push(("§3.2.6 a distance symbol of 30 or 31", w.finish(), E_CODE));
    }
    // §3.2.5: a distance reaching behind the start of the stream. One
    // literal has been produced, so the history is one byte long.
    {
        let mut w = BitWriter::new();
        w.bits(1, 1);
        w.bits(1, 2);
        let (code, bits) = fixed_lit_code(b'A'.into());
        w.code(code, bits);
        let (code, bits) = fixed_lit_code(257); // length 3
        w.code(code, bits);
        w.code(13, 5); // distance symbol 13: base 97, five extra bits
        w.bits(3, 5); // distance 100, and one byte of history
        raw_cases.push((
            "§3.2.5 a distance behind the start of the stream",
            w.finish(),
            E_DIST,
        ));
    }
    // §3.2.7: an over-subscribed code-length code. Four codes of one bit
    // each, in a space that holds two.
    {
        let mut w = BitWriter::new();
        w.bits(1, 1);
        w.bits(2, 2); // dynamic
        w.bits(0, 5); // HLIT: 257 literal/length codes
        w.bits(0, 5); // HDIST: 1 distance code
        w.bits(0, 4); // HCLEN: 4 code-length codes
        for _ in 0..4 {
            w.bits(1, 3); // all one bit long
        }
        raw_cases.push((
            "§3.2.7 an over-subscribed code-length code",
            w.finish(),
            E_CODE,
        ));
    }
    // §3.2.7: an incomplete code-length code. One code of one bit, in a
    // space that holds two.
    {
        let mut w = BitWriter::new();
        w.bits(1, 1);
        w.bits(2, 2);
        w.bits(0, 5);
        w.bits(0, 5);
        w.bits(0, 4);
        w.bits(1, 3);
        for _ in 0..3 {
            w.bits(0, 3);
        }
        raw_cases.push(("§3.2.7 an incomplete code-length code", w.finish(), E_CODE));
    }
    // §3.2.7's code 16 copies the previous code length, and there is no
    // previous one at the start.
    //
    // The code-length code here is four two-bit codes, which is complete:
    // in §3.2.7's permutation the first four entries are symbols 16, 17,
    // 18 and 0, so sorted by (length, symbol) the codes are 0 -> 00,
    // 16 -> 01, 17 -> 10, 18 -> 11.
    let cl_four = |w: &mut BitWriter| {
        w.bits(1, 1);
        w.bits(2, 2);
        w.bits(0, 5);
        w.bits(0, 5);
        w.bits(0, 4);
        for _ in 0..4 {
            w.bits(2, 3); // every one of the four is two bits long
        }
    };
    {
        let mut w = BitWriter::new();
        cl_four(&mut w);
        w.code(1, 2); // symbol 16, with nothing to repeat
        w.bits(0, 2); // its two extra bits
        raw_cases.push((
            "§3.2.7 code 16 with no previous code length",
            w.finish(),
            E_CODE,
        ));
    }
    // §3.2.7: a run that writes past the last code length declared.
    // HLIT 0 and HDIST 0 declare 258 of them; two runs of 138 zeros is
    // 276.
    {
        let mut w = BitWriter::new();
        cl_four(&mut w);
        for _ in 0..2 {
            w.code(3, 2); // symbol 18: repeat zero 11 to 138 times
            w.bits(127, 7); // 11 + 127 = 138
        }
        raw_cases.push((
            "§3.2.7 a zero run past the last code length",
            w.finish(),
            E_CODE,
        ));
    }
    // The empty stream: `in_last` with no bytes at all.
    raw_cases.push(("an empty stream", Vec::new(), E_TRUNC));

    // --- run them -------------------------------------------------------
    let zlib = inflate_design(15, 1);
    let raw = inflate_design(15, 0);
    let mut zsim = simulate(&zlib, "inflate (zlib)");
    let mut rsim = simulate(&raw, "inflate (raw)");
    let zbench = InflateBench::attach(&mut zsim);
    let rbench = InflateBench::attach(&mut rsim);

    for (what, stream, code) in &zlib_cases {
        let run = zbench.run(
            &mut zsim,
            stream,
            Sink::Greedy,
            inflate_limit(stream.len(), 4096),
        );
        assert_eq!(
            run.error,
            Some(*code),
            "{what}: {} -> error {:?}, done {}, {} bytes out",
            hex(stream),
            run.error,
            run.done,
            run.out.len()
        );
    }
    for (what, stream, code) in &raw_cases {
        let run = rbench.run(
            &mut rsim,
            stream,
            Sink::Greedy,
            inflate_limit(stream.len(), 4096),
        );
        assert_eq!(
            run.error,
            Some(*code),
            "{what}: {} -> error {:?}, done {}, {} bytes out",
            hex(stream),
            run.error,
            run.done,
            run.out.len()
        );
    }
    // Every code but `E_NONE` has a case. A code with no case is a check
    // nothing drives.
    let mut seen: BTreeSet<u8> = BTreeSet::new();
    for (_, _, code) in zlib_cases.iter().chain(raw_cases.iter()) {
        seen.insert(*code);
    }
    assert_eq!(
        seen,
        [E_HEADER, E_BTYPE, E_NLEN, E_CODE, E_DIST, E_TRUNC, E_ADLER]
            .into_iter()
            .collect::<BTreeSet<u8>>(),
        "not every error code has a case"
    );
    println!(
        "inflate: {} malformed streams reported, {} distinct codes",
        zlib_cases.len() + raw_cases.len(),
        seen.len()
    );
}

/// A legal stream run against a window too small for it is **reported**,
/// not decoded wrongly.
///
/// A design built with `WINDOW_BITS` below 15 cannot decode every legal
/// stream, because RFC 1951 §3.2.5 lets a compressor name a distance of
/// up to 32768. The choice `inflate_window.v` makes is to let that fail
/// at the distance rather than up front on RFC 1950 §2.2's CINFO, so
/// that a short file compressed by an ordinary compressor still decodes
/// on a small window — which is the common case and the useful one.
///
/// This test is the pair that shows the difference is about the window
/// and not about the stream: the *same* stream decodes at 15 and is
/// refused at 8, and a stream whose distances were capped to 256 decodes
/// at both.
///
/// What it would catch: a window whose fullness is tracked wrongly, so
/// that a distance it does hold is refused or one it does not is
/// accepted; and a `WINDOW_BITS` that is not actually plumbed through.
///
/// What it would not catch: whether the *first* out-of-range distance is
/// the one reported, since nothing here counts how far the decode got.
#[test]
fn inflate_reports_a_distance_its_window_cannot_reach() {
    const E_DIST: u8 = 5;
    // A stream with distances far past 256: 4000 bytes of text at level 9
    // with no distance cap.
    let big = corpus()
        .iter()
        .find(|c| c.name == "text1_4000_l6_deflate")
        .expect("the 4000-byte text case");
    // And the same shape of data compressed so that nothing reaches past
    // 256, which is what `max_distance` is for.
    let capped = corpus()
        .iter()
        .find(|c| c.name == "w256_text19_4000_deflate")
        .expect("the capped 4000-byte text case");

    let wide = inflate_design(15, 0);
    let narrow_win = inflate_design(8, 0);
    let mut wsim = simulate(&wide, "inflate (window 32768)");
    let mut nsim = simulate(&narrow_win, "inflate (window 256)");
    let wbench = InflateBench::attach(&mut wsim);
    let nbench = InflateBench::attach(&mut nsim);

    for case in [big, capped] {
        let plain = plaintext(&case.rule, case.len);
        let limit = inflate_limit(case.stream.len(), case.len);
        let run = wbench.run(&mut wsim, &case.stream, Sink::Greedy, limit);
        assert_eq!(run.error, None, "{}: refused by a full window", case.name);
        assert_eq!(run.out, plain, "{}: wrong at WINDOW_BITS=15", case.name);
    }

    let run = nbench.run(
        &mut nsim,
        &capped.stream,
        Sink::Greedy,
        inflate_limit(capped.stream.len(), capped.len),
    );
    assert_eq!(
        run.error, None,
        "{}: a capped stream was refused by a 256-byte window",
        capped.name
    );
    assert_eq!(
        run.out,
        plaintext(&capped.rule, capped.len),
        "{}: wrong at WINDOW_BITS=8",
        capped.name
    );

    let run = nbench.run(
        &mut nsim,
        &big.stream,
        Sink::Greedy,
        inflate_limit(big.stream.len(), big.len),
    );
    assert_eq!(
        run.error,
        Some(E_DIST),
        "{}: a 256-byte window accepted a 32 KiB-window stream and produced {} bytes",
        big.name,
        run.out.len()
    );
    // And what it did produce before refusing is a prefix of the truth,
    // not rubbish.
    let plain = plaintext(&big.rule, big.len);
    assert!(
        run.out.len() <= plain.len() && plain.starts_with(&run.out),
        "{}: the bytes before the refusal are not a prefix of the plaintext",
        big.name
    );
}

/// A corrupted stream is refused or decoded correctly, and never
/// anything else.
///
/// This is the property that matters for a block that reads bytes
/// somebody else chose: for **every** single-byte corruption of a real
/// stream, the run must end with `error`, or with `done` and the exact
/// plaintext — and it must end. It must never claim `done` with bytes
/// that are not the plaintext, and it must never fail to end, which is
/// what `run`'s loop bound asserts.
///
/// Three streams, one of each block type of §3.2.3, two masks per byte
/// position: 0x01, which usually lands inside a Huffman code, and 0x80,
/// which usually lands in a different field of the same byte.
///
/// **A refused run's partial output is not checked, and must not be.** A
/// corrupted stream is a *different* stream: a flipped bit in a code
/// length changes the whole code, so the bytes a decoder produces before
/// it notices are legitimately different bytes rather than a prefix of
/// the truth. That is what the checksum of RFC 1950 §9 is for, and
/// `error` is the decoder saying so. Asserting a prefix here was the
/// first version of this test and it failed on byte 35 — correctly.
///
/// Some corruptions decode *correctly*, and that is not a fault either:
/// a flipped bit in the padding after the last block changes nothing a
/// decoder reads. The assertion allows exactly that and nothing else.
///
/// What it would catch: a decoder that can be made to loop (a repeat
/// count that does not terminate, a copy that never drains, a bit reader
/// that stops asking for bytes); one that can be made to read a window
/// position it never wrote, which `E_DIST` is the answer to; and one
/// that produces wrong output and still says `done`.
///
/// What it would not catch: a multi-byte corruption; a corruption of a
/// **raw** stream, which has no checksum, so wrong-but-plausible output
/// is possible there by design and the container above it is what
/// catches it; and a stream crafted rather than corrupted, which is
/// `inflate_reports_every_malformed_stream`'s half of the job.
#[test]
fn inflate_refuses_or_decodes_every_single_byte_corruption() {
    let design = inflate_design(15, 1);
    let mut sim = simulate(&design, "inflate (zlib)");
    let bench = InflateBench::attach(&mut sim);

    let mut refused = 0usize;
    let mut unharmed = 0usize;
    let mut codes: BTreeMap<u8, usize> = BTreeMap::new();
    // One stream of each of §3.2.3's three block types, so that a
    // corruption lands in a dynamic header, a fixed block's codes and a
    // stored block's length field.
    for name in ["text1_100_l6_zlib", "runs2_500_l6_zlib", "lcg1_100_l6_zlib"] {
        let case = corpus()
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no case {name}"));
        let plain = plaintext(&case.rule, case.len);
        for i in 0..case.stream.len() {
            for mask in [0x01u8, 0x80] {
                let mut bad = case.stream.clone();
                bad[i] ^= mask;
                let run = bench.run(
                    &mut sim,
                    &bad,
                    Sink::Greedy,
                    inflate_limit(bad.len(), case.len + 1024),
                );
                match run.error {
                    Some(code) => {
                        refused += 1;
                        *codes.entry(code).or_default() += 1;
                        assert!(!run.done, "{name} byte {i} xor {mask:#04x}: done and error");
                    }
                    None => {
                        assert!(
                            run.done,
                            "{name} byte {i} xor {mask:#04x}: neither done nor error"
                        );
                        assert_eq!(
                            run.out, plain,
                            "{name} byte {i} xor {mask:#04x}: said `done` with the wrong bytes"
                        );
                        unharmed += 1;
                    }
                }
            }
        }
    }
    assert!(
        refused > 100 && unharmed < refused,
        "{refused} refused and {unharmed} decoded correctly: that is not a corruption sweep"
    );
    // Every corruption that gets past the structure checks is caught by
    // §9's checksum, so a sweep with no `E_ADLER` in it would mean the
    // trailer was not being checked at all.
    assert!(
        codes.contains_key(&7),
        "no corruption was caught by the Adler-32: {codes:?}"
    );
    println!(
        "inflate: {} single-byte corruptions of three streams: {refused} reported {codes:?}, \
         {unharmed} harmless",
        refused + unharmed
    );
}

/// Every prefix of a stream is reported, and never finished.
///
/// A truncated stream is the malformation an attacker gets for free, and
/// the one a decompressor is most likely to wait forever on. For every
/// proper prefix of three real streams — one of each block type, and one
/// of them raw, so that there is no checksum to fall back on — the run
/// must end with an error and must not say `done`.
///
/// What it would catch: a bit reader that waits for a byte that is never
/// coming instead of noticing `in_last`; any state that needs input and
/// does not check for the end of it; and a zlib stream whose missing
/// trailer is not noticed, which would turn a truncation into a silent
/// short read. The raw case is the one that matters most: with no
/// checksum, `E_TRUNC` is the only thing that can catch it.
///
/// What it would not catch: *which* error a given prefix gets, which is
/// not a property worth pinning — a prefix whose last partial byte
/// happens to spell a reserved BTYPE legitimately reports `E_BTYPE`
/// rather than `E_TRUNC`. The bound on the output length is there
/// instead: the padding bits of the last byte can spell at most one more
/// symbol, so at most one more match of at most 258 bytes.
#[test]
fn inflate_reports_every_truncation() {
    let zlib = inflate_design(15, 1);
    let raw = inflate_design(15, 0);
    let mut zsim = simulate(&zlib, "inflate (zlib)");
    let mut rsim = simulate(&raw, "inflate (raw)");
    let zbench = InflateBench::attach(&mut zsim);
    let rbench = InflateBench::attach(&mut rsim);

    let mut codes: BTreeMap<u8, usize> = BTreeMap::new();
    let mut prefixes = 0usize;
    let mut exact = 0usize;
    for name in [
        "text1_100_l6_zlib",
        "lcg1_100_l6_zlib",
        "runs2_500_l6_deflate",
    ] {
        let case = corpus()
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no case {name}"));
        let plain = plaintext(&case.rule, case.len);
        let (sim, bench) = if case.wrapper == "zlib" {
            (&mut zsim, &zbench)
        } else {
            (&mut rsim, &rbench)
        };
        for n in 0..case.stream.len() {
            let run = bench.run(
                sim,
                &case.stream[..n],
                Sink::Greedy,
                inflate_limit(n, case.len + 1024),
            );
            let code = run
                .error
                .unwrap_or_else(|| panic!("{name}: a {n}-byte prefix finished: done {}", run.done));
            assert!(!run.done, "{name}: a {n}-byte prefix is done and in error");
            assert!(
                run.out.len() <= plain.len() + 258,
                "{name}: a {n}-byte prefix produced {} bytes of a {}-byte plaintext",
                run.out.len(),
                plain.len()
            );
            if plain.starts_with(&run.out) {
                exact += 1;
            }
            *codes.entry(code).or_default() += 1;
            prefixes += 1;
        }
    }
    assert!(prefixes > 200, "only {prefixes} prefixes");
    println!(
        "inflate: {prefixes} prefixes of three streams, all reported: {codes:?}; \
         {exact} produced an exact prefix of the plaintext"
    );
}

/// The big cases: a stream long enough that the 32 KiB window wraps and
/// a distance reaches past it.
///
/// `#[ignore]`d because it is the only part of this round measured in
/// millions of simulated clock edges: 680 000 output bytes across nine
/// streams, including 120 000 bytes of text whose matches do reach back
/// tens of thousands of bytes. Nothing in the fast corpus can: a
/// distance past 32768 needs a plaintext longer than that, and a
/// plaintext longer than that costs more cycles than a gate should
/// spend.
///
/// What it would catch: a window pointer that wraps wrongly, a
/// `full_q` that never latches so that an old distance is refused after
/// the wrap, and an Adler-32 whose reduction drifts over a hundred
/// thousand bytes.
///
/// What it would not catch: anything the fast corpus already covers,
/// and nothing about malformed input.
#[test]
#[ignore = "hundreds of thousands of simulated cycles; run it deliberately"]
fn inflate_crosses_the_32_kib_window() {
    let zlib = inflate_design(15, 1);
    let raw = inflate_design(15, 0);
    let mut zsim = simulate(&zlib, "inflate (zlib)");
    let mut rsim = simulate(&raw, "inflate (raw)");
    let zbench = InflateBench::attach(&mut zsim);
    let rbench = InflateBench::attach(&mut rsim);

    let mut bytes = 0u64;
    let mut cycles = 0u64;
    let mut cases = 0usize;
    for case in corpus() {
        if case.tier != "bulk" {
            continue;
        }
        let plain = plaintext(&case.rule, case.len);
        let (sim, bench) = if case.wrapper == "zlib" {
            (&mut zsim, &zbench)
        } else {
            (&mut rsim, &rbench)
        };
        let run = bench.run(
            sim,
            &case.stream,
            Sink::Greedy,
            inflate_limit(case.stream.len(), case.len),
        );
        assert_eq!(run.error, None, "{}: reported an error", case.name);
        assert!(run.done, "{}: never finished", case.name);
        assert_eq!(run.out.len(), plain.len(), "{}: wrong length", case.name);
        assert!(run.out == plain, "{}: wrong bytes", case.name);
        println!(
            "  {}: {} stream bytes -> {} output bytes in {} cycles ({:.3} bytes/cycle)",
            case.name,
            case.stream.len(),
            case.len,
            run.cycles,
            case.len as f64 / run.cycles as f64
        );
        bytes += case.len as u64;
        cycles += run.cycles;
        cases += 1;
    }
    assert!(cases >= 8, "only {cases} bulk cases");
    println!(
        "inflate: {cases} large cases, {bytes} output bytes in {cycles} cycles \
         ({:.3} bytes/cycle)",
        bytes as f64 / cycles as f64
    );
}

/// What this block actually costs per byte, by block type.
///
/// `#[ignore]`d and printed, never asserted: a throughput figure belongs
/// in a README and a cycle count asserted against a constant is a test
/// that fails the day somebody makes the block faster.
///
/// The three numbers are different enough to be worth separating. A
/// stored block is a byte a cycle. A copy is a byte a cycle once it
/// starts. A literal costs a cycle per bit of its Huffman code and one
/// more, so it is the slow case and it is what ordinary text is mostly
/// made of.
#[test]
#[ignore = "prints a throughput figure for ip/compress/inflate/README.md §8"]
fn inflate_throughput_by_block_type() {
    let raw = inflate_design(15, 0);
    let mut sim = simulate(&raw, "inflate (raw)");
    let bench = InflateBench::attach(&mut sim);
    for name in [
        "lcg1_1000_l6_deflate",
        "zeros_1024_l6_deflate",
        "rep64_4096_l6_deflate",
        "text1_4000_l6_deflate",
        "counter_1000_l6_deflate",
        "runs4_2000_l6_deflate",
    ] {
        let case = corpus()
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no case {name}"));
        let run = bench.run(
            &mut sim,
            &case.stream,
            Sink::Greedy,
            inflate_limit(case.stream.len(), case.len),
        );
        assert_eq!(run.error, None, "{name}");
        println!(
            "{name}: first block BTYPE {}, {} stream bytes -> {} output bytes in {} cycles, \
             {:.3} bytes/cycle",
            case.first.1,
            case.stream.len(),
            case.len,
            run.cycles,
            case.len as f64 / run.cycles as f64
        );
    }
}

/// The input ports a given output port depends on combinationally.
///
/// Builds the timing graph of the flattened, synthesised block and walks
/// *backwards* from the named output port, stopping at every start
/// point — which is either a sequential cell's output or an input port.
/// What comes back is the names of the input ports the walk reached, so
/// an empty answer means the output is a function of registers and
/// constants and nothing else.
fn combinational_inputs_of(
    package: &str,
    top: &str,
    params: &[(&str, &str)],
    port: &str,
) -> BTreeSet<String> {
    use reticle::timing::graph::{PinDirection, PointKind, TimingGraph};

    let (mut design, id) = flattened(package, top, params);
    let mut diags = Diagnostics::new();
    synth_run(&mut design, &SynthOptions::default(), &mut diags);
    assert!(!diags.has_errors(), "{package}.{top} does not synthesise");
    let module = flatten_for_timing(&design, id).expect("a flat module");
    let graph = TimingGraph::build(&module);

    // The pin of the output port is the one that *loads* the net the
    // port drives, which is the one with `Input` direction.
    let start = graph
        .pins
        .iter()
        .position(|p| p.port == port && p.cell_type == "port" && p.direction == PinDirection::Input)
        .map(|i| graph.pins[i].clone())
        .unwrap_or_else(|| panic!("{package}.{top} has no output port pin `{port}`"));
    let start_id = graph
        .pin_by_name(&start.name)
        .expect("the pin names itself");

    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut queue = vec![start_id];
    let mut inputs: BTreeSet<String> = BTreeSet::new();
    while let Some(pin) = queue.pop() {
        if !seen.insert(pin.raw()) {
            continue;
        }
        // A start point ends the walk. Which kind it is, is the answer.
        if pin != start_id && graph.is_start(pin) {
            let kind = graph
                .start_points
                .iter()
                .find(|s| s.pin == pin)
                .map(|s| s.kind)
                .expect("a start point");
            if kind == PointKind::Port {
                inputs.insert(graph.pin(pin).owner.clone());
            }
            continue;
        }
        for arc in graph.fanin(pin) {
            queue.push(arc.from);
        }
    }
    inputs
}

/// `in_ready` is a function of registers, on every block in this library
/// that has one.
///
/// `ip/crypto/chacha20` established the rule and wrote it in its header:
/// **`in_ready` must not depend combinationally on anything a producer
/// drives.** If it does, two such blocks back to back build a path from
/// one's `in_valid` to the other's `in_ready`, and a row of them takes
/// the clock with them. Until this test there was nothing that checked
/// it; `ip/compress/inflate/README.md` §4.3 named that as a gap in the
/// test suite, and this is it closed.
///
/// The method is the timing graph the static timing analysis already
/// builds: walk backwards from the `in_ready` port and see which start
/// points the walk reaches. A start point is either a sequential cell's
/// output or an input port, so the answer is exactly the set of input
/// ports `in_ready` is a combinational function of, and the assertion is
/// that the set is **empty**.
///
/// What it would catch: `assign in_ready = state_ok && in_valid;`, which
/// is the natural way to write a block that only accepts a byte when it
/// can use it, and the single mistake this rule exists to forbid. It
/// would also catch it through any depth of logic and through a module
/// boundary, because the walk is over the flattened design. **It was
/// checked by breaking it**: adding `&& in_valid` to `inflate`'s
/// `in_ready` fails this test with
/// ``inflate.inflate: `in_ready` depends combinationally on {"in_valid"}``,
/// which is what makes it a measurement rather than an empty pass.
///
/// What it would not catch: the same fault on `out_valid`, which is the
/// mirror rule — an `out_valid` that depends on `out_ready` is just as
/// bad — and which is checked here too for the one block that has both
/// handshakes. And it says nothing about *delay*: a registered
/// `in_ready` with forty levels of logic behind it is a slow block, not
/// a broken contract, and `lut_depth` in the footprint table is where
/// that shows up.
#[test]
fn a_streams_ready_is_a_function_of_registers() {
    // Every block in the library with a `valid`/`ready` input stream.
    type Case = (
        &'static str,
        &'static str,
        &'static [(&'static str, &'static str)],
    );
    let cases: &[Case] = &[
        ("sha256", "sha256", &[]),
        ("chacha20", "chacha20", &[]),
        (
            "inflate",
            "inflate",
            &[("WINDOW_BITS", "10"), ("WRAPPER", "1")],
        ),
        (
            "inflate",
            "inflate",
            &[("WINDOW_BITS", "10"), ("WRAPPER", "0")],
        ),
    ];
    for (package, top, params) in cases {
        let inputs = combinational_inputs_of(package, top, params, "in_ready");
        assert!(
            inputs.is_empty(),
            "{package}.{top}: `in_ready` depends combinationally on {inputs:?}"
        );
    }
    // The mirror rule, for the one block with a back-pressured output.
    let inputs = combinational_inputs_of(
        "inflate",
        "inflate",
        &[("WINDOW_BITS", "10"), ("WRAPPER", "1")],
        "out_valid",
    );
    assert!(
        inputs.is_empty(),
        "inflate: `out_valid` depends combinationally on {inputs:?}"
    );
    // And the control-flag outputs a consumer samples, for the same
    // reason: a `done` that depended on `out_ready` would make the
    // consumer's own logic part of this block's path.
    for port in ["done", "error", "busy"] {
        let inputs = combinational_inputs_of(
            "inflate",
            "inflate",
            &[("WINDOW_BITS", "10"), ("WRAPPER", "1")],
            port,
        );
        assert!(
            inputs.is_empty(),
            "inflate: `{port}` depends combinationally on {inputs:?}"
        );
    }
}
