//! Project Trellis' Lattice ECP5 database, and the `.bit` container that
//! goes with it.
//!
//! # What needs what, and what skips
//!
//! **A missing database must never fail the build**, and a test here never
//! downloads one. Two things can be missing and each skips separately,
//! with a line saying what to do about it:
//!
//! - the **database**, `reticle fetch prjtrellis-db` or `RETICLE_TRELLISDB`;
//! - the **reference bitstreams**, `RETICLE_ECP5_REF` pointing at a
//!   directory holding `analyzer.bit`, `selftest.bit` and
//!   `facedancer.bit` — the three Great Scott Gadgets build for a Cynthion
//!   r1.4 and ship in their `cynthion` Python package. They are not in
//!   this repository: they are 750 kB of somebody else's build output, and
//!   what they are for here is a cross-check, not an input.
//!
//! A database that is *present and will not open* panics rather than
//! skipping, the same way `tests/fpga_gowin.rs` and `tests/fpga_xray.rs`
//! treat theirs: a broken copy is a bug, not an absence.
//!
//! # What the reference bitstreams settle
//!
//! Two different things, and it is worth keeping them apart.
//!
//! They settle the **container**, by round-tripping: each file is read by
//! `Ecp5Stream::parse` and written back by `to_bytes` identical byte for
//! byte, which covers the metadata header, the preamble, every command,
//! the compression dictionary and the rule that picks it, all 7562
//! per-frame check words, the reversed frame order and the bit packing
//! inside a frame.
//!
//! And they settle **where a pad's bits are**, which no amount of reading
//! a database can. All six of a Cynthion's FPGA LEDs are outputs in its
//! own analyzer gateware, so the bits this crate would set to make one an
//! output can be compared, at absolute frame positions, with the bits
//! Lattice's own packer set for the same ball. That comparison is
//! `the_led_pads_are_where_this_boards_own_gateware_has_them`, and it is
//! what turns nextpnr's tile rule from something read into something
//! checked.

#![cfg(feature = "fpga")]

use std::path::Path;

// Named by the reference-bitstream comparisons as well as by the designs,
// so the gate is `verilog` and not `verilog` plus `synth`.
#[cfg(feature = "verilog")]
use reticle::fpga::arch::ConfigBit;
use reticle::fpga::ecp5::{Ecp5Stream, FrameFormat};
use reticle::fpga::trellis::{self, TrellisOptions};
use reticle::ir::memfile::FileProvider;

/// The part, as Project Trellis names it, and as the device file does.
const PART: &str = "LFE5U-12F";
#[cfg(all(feature = "verilog", feature = "synth"))]
const DEVICE: &str = "ecp5-12f-CABGA256";

/// The identifier a Cynthion's ECP5 answers, read from the part by
/// `tests/program_apollo.rs`.
const IDCODE: u32 = 0x2111_1043;

/// The shape of the LFE5U-12F's configuration memory, as `devices.json`
/// gives it. Written out here so a reader of a `.bit` does not need the
/// database; the test below checks the two agree.
const FRAMES: u32 = 7562;
const BITS_PER_FRAME: u32 = 592;

/// The USER button's ball, from Great Scott Gadgets' own platform file.
/// Unlike the LEDs it is on the **right** edge of the die, in bank 3.
const BUTTON: &str = "M14";

/// The six FPGA LEDs of a Cynthion r1.4, in the order the platform file
/// lists them, so index 0 is the one silkscreened `0`.
const LEDS: [(u32, &str); 6] = [
    (0, "E13"),
    (1, "C13"),
    (2, "B14"),
    (3, "A15"),
    (4, "D12"),
    (5, "C11"),
];

/// A [`FileProvider`] over a directory.
struct Disk(String);

impl FileProvider for Disk {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(Path::new(&self.0).join(path)).ok()
    }
}

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
///
/// The name and version are spelled out because a test cannot see the
/// binary's `datadir` module; a unit test there checks that this file
/// asks for the version it pins.
fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let root = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")))?;
    let dir = format!("{root}/{name}/{version}");
    Path::new(&format!("{dir}/{probe}"))
        .is_file()
        .then_some(dir)
}

/// The database directory, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let probe = "ECP5/LFE5U-12F/tilegrid.json";
    let Ok(root) = std::env::var("RETICLE_TRELLISDB") else {
        let pinned = fetched(
            "prjtrellis-db",
            "015e0330630d7c238c0e4f2cdd9c8157eb78c54a",
            probe,
        );
        if pinned.is_none() {
            eprintln!(
                "skipped: needs a Project Trellis database; run `reticle fetch prjtrellis-db` \
                 or set RETICLE_TRELLISDB to a prjtrellis-db checkout"
            );
        }
        return pinned;
    };
    let full = format!("{root}/{probe}");
    if !Path::new(&full).exists() {
        eprintln!("skipped: RETICLE_TRELLISDB is `{root}` but `{full}` is not there");
        return None;
    }
    Some(root)
}

/// The fabric, loaded, or `None` having said why not.
///
/// A database that is there and will not load panics: an absent database
/// is somebody's machine, a broken one is a bug.
fn open() -> Option<trellis::TrellisFabric> {
    let root = chipdb()?;
    let db = match trellis::open(&Disk(root), "", PART) {
        Ok(db) => db,
        Err(err) => panic!("the database is there and would not open: {err}"),
    };
    match db.load(&TrellisOptions::new()) {
        Ok(fabric) => Some(fabric),
        Err(err) => panic!("the database is there and would not load: {err}"),
    }
}

/// One of Great Scott Gadgets' own bitstreams for a Cynthion r1.4, or
/// `None` having said where to get it.
fn reference(name: &str) -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("RETICLE_ECP5_REF") else {
        eprintln!(
            "skipped: needs Great Scott Gadgets' own bitstreams for a Cynthion r1.4; set \
             RETICLE_ECP5_REF to a directory holding analyzer.bit, selftest.bit and \
             facedancer.bit (the `cynthion` Python package ships them under \
             `cynthion/assets/CynthionPlatformRev1D4/`). See docs/fpga-trellis.md"
        );
        return None;
    };
    let path = Path::new(&dir).join(format!("{name}.bit"));
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            eprintln!("skipped: `{}` would not read: {err}", path.display());
            None
        }
    }
}

/// The frame shape of the one part this file knows, for
/// [`Ecp5Stream::parse`], which reads no database of its own.
fn formats(idcode: u32) -> Option<FrameFormat> {
    (idcode == IDCODE).then(|| FrameFormat::new(FRAMES, BITS_PER_FRAME))
}

/// Compiles a Verilog design against the ECP5 fabric, all the way to a
/// bitmap and a stream, and returns both.
///
/// Behind `verilog` and `synth` because a build with only `fpga` on has no
/// frontend to read a design with. The tests above it need neither and run
/// in that build; the three below it are skipped by the feature and not by
/// a missing database, which is a different kind of absence.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn compile(
    fabric: &trellis::TrellisFabric,
    verilog_path: &str,
    rcf_path: &str,
) -> (
    reticle::fpga::bitstream::Bitstream,
    Ecp5Stream,
    usize,
    reticle::fpga::route::RoutingReport,
    reticle::fpga::bitstream::Bitstream,
    Routed,
) {
    compile_all(fabric, &[verilog_path], rcf_path)
}

/// The same, for a design whose sources are several files: a top level and
/// the library blocks it instantiates. There is no search path, so they are
/// named, exactly as `reticle fpga` wants them on its command line.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn compile_all(
    fabric: &trellis::TrellisFabric,
    verilog_paths: &[&str],
    rcf_path: &str,
) -> (
    reticle::fpga::bitstream::Bitstream,
    Ecp5Stream,
    usize,
    reticle::fpga::route::RoutingReport,
    reticle::fpga::bitstream::Bitstream,
    Routed,
) {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{Netlist, PlaceOptions, place};
    use reticle::fpga::route::{RouteOptions, route};
    use reticle::fpga::{Constraints, FpgaOptions, bitstream, synthesize_for, target};
    use reticle::source::SourceMap;

    let rcf = std::fs::read_to_string(rcf_path).expect("the constraints");
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut asts = Vec::new();
    for path in verilog_paths {
        let text = std::fs::read_to_string(path).expect("the design");
        let source = map.add(*path, &text).unwrap();
        asts.push(reticle::verilog::parse_source(
            &mut map,
            source,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        ));
    }
    let rcf_file = map.add(rcf_path, &rcf).unwrap();
    let refs: Vec<_> = asts.iter().collect();
    let mut design =
        reticle::verilog::elaborate(&refs, &reticle::verilog::ElabOptions::default(), &mut diags)
            .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.unwrap();

    let device = target(DEVICE).expect("the device file describes this part");
    assert_eq!(device.idcode, Some(IDCODE), "the device file states it");
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, report) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    assert_eq!(
        report.fixed,
        netlist.instances.iter().filter(|i| i.pin.is_some()).count(),
        "every constrained pin is held at the site the ball map gives it"
    );
    // The same options `reticle fpga --bitstream` uses, because the one
    // knob this family needs changes where a clock goes: without it the
    // shortest path from a pad to a flip-flop's clock pin is through data
    // wires. See `TrellisFabric::clock_node_costs`.
    let options = RouteOptions {
        node_base: fabric.clock_node_costs(&graph, trellis::CLOCK_PREFERENCE),
        ..RouteOptions::default()
    };
    let (routing, route_report) = route(&netlist, &graph, &placement, &options).unwrap();
    // Every signal that has one, and every sink walked back to its driver
    // rather than taken on the router's word.
    assert_eq!(route_report.signals, netlist.routable().len());
    let problems = routing.verify(&netlist, &graph, &placement);
    assert!(problems.is_empty(), "{problems:?}");

    let mut bits = bitstream::generate(
        &design,
        top,
        &fabric.arch,
        &graph,
        &netlist,
        &placement,
        &routing,
    )
    .unwrap();
    let pads = fabric
        .configure_io(&design, top, &netlist, &placement, &graph, &mut bits)
        .unwrap();
    fabric
        .configure_logic(&design, top, &netlist, &placement, &graph, &mut bits)
        .unwrap();
    let ffs = fabric
        .configure_registers(
            &design, top, &netlist, &placement, &graph, &routing, &mut bits,
        )
        .unwrap();
    let rams = fabric
        .configure_lutram(&design, top, &netlist, &placement, &graph, &mut bits)
        .unwrap();
    let mut ram_only =
        bitstream::Bitstream::empty(bitstream::BitstreamFormat::from_arch(&fabric.arch));
    fabric
        .configure_lutram(&design, top, &netlist, &placement, &graph, &mut ram_only)
        .unwrap();
    let clocks = fabric.clock_network_use(&netlist, &placement, &graph, &routing);
    let dropped = fabric.dropped_clear_bits(&graph, &routing, &bits);
    // The same pass on its own, into an empty bitmap. A `CIB`'s constant
    // mux and its routing mux are the *same* mux, so the bits that tie a
    // wire and the bits that route into it share bit space and "is this pad
    // tied?" cannot be read off a finished bitstream. Asking the pass
    // directly can.
    let mut io_only =
        bitstream::Bitstream::empty(bitstream::BitstreamFormat::from_arch(&fabric.arch));
    fabric
        .configure_io(&design, top, &netlist, &placement, &graph, &mut io_only)
        .unwrap();
    let stream = fabric.stream(&bits, "8").unwrap();
    let wires = routing
        .routes()
        .flat_map(|route| route.pips.iter())
        .filter(|pip| !graph.pip_bits(**pip).is_empty())
        .map(|pip| {
            let pip = graph.pip(*pip);
            let from = graph.wire(pip.from);
            let to = graph.wire(pip.to);
            ((to.name.clone(), to.tile), (from.name.clone(), from.tile))
        })
        .collect();
    (
        bits,
        stream,
        pads,
        route_report,
        io_only,
        Routed {
            wires,
            ffs,
            rams,
            ram_only,
            placement,
            clocks,
            dropped,
            graph,
            routing,
            netlist,
        },
    )
}

/// The arcs a routing used, as `(sink, source)` in the graph's own names and
/// positions, for comparing with a decoding of the bitstream.
///
/// Only the arcs that *cost bits* are here. A pip with an empty bit pattern
/// leaves no trace in a bitstream, so nothing can be said about it this way;
/// `Routing::verify` is what checks those.
#[cfg(all(feature = "verilog", feature = "synth"))]
struct Routed {
    wires: std::collections::BTreeSet<(Wire, Wire)>,
    /// How many flip-flops `configure_registers` wrote settings for.
    ffs: usize,
    /// How many distributed RAMs `configure_lutram` wrote settings for.
    rams: usize,
    /// That pass on its own, into an empty bitmap, for the same reason
    /// `io_only` exists: what a feature costs is a question about the pass,
    /// not about the finished image, where several features share bit space.
    ram_only: reticle::fpga::bitstream::Bitstream,
    /// Where every instance ended up, so a test can ask which tile holds
    /// what. A distributed RAM is exactly the case where that is the
    /// question: it takes six of its tile's eight lookup tables, so nothing
    /// else may be in there.
    placement: reticle::fpga::place::Placement,
    /// Which global clock network each of their clocks arrived on.
    clocks: trellis::ClockUse,
    /// Bits an arc of the design needs **clear** that something else set.
    /// Empty is the only acceptable answer; see
    /// `TrellisFabric::dropped_clear_bits`.
    dropped: Vec<String>,
    /// The graph the routing is over, so a test can ask the routing a
    /// question of its own rather than only read what `compile` measured.
    graph: reticle::fpga::arch::RoutingGraph,
    /// The routing itself, for the same reason.
    routing: reticle::fpga::Routing,
    /// The netlist the placer and the router worked on, so a test can ask
    /// which of a pad's three wires carry a signal rather than only which
    /// bits came out. A bidirectional pad is exactly the case where that
    /// is the question: its tristate has to be *routed*, and the bits that
    /// would tie it are the same bits a route into it sets.
    netlist: reticle::fpga::place::Netlist,
}

/// One wire of the routing graph: its name and the position it starts in.
#[cfg(all(feature = "verilog", feature = "synth"))]
type Wire = (String, (u32, u32));

/// Whether Lattice's own bitstreams for this board contain a distributed
/// RAM at all, asked because the answer decides how strong the claim in
/// `TrellisFabric::configure_lutram` is allowed to be.
///
/// **They do not.** Not one slice of `analyzer.bit`, `selftest.bit` or
/// `facedancer.bit` is in `DPRAM` or `RAMW` mode. All three use block RAM
/// for their memories — nine, none and forty-four `DP16KD` blocks — and
/// their `MODE` fields are `LOGIC` and `CCU2` and nothing else.
///
/// So the distributed RAM this backend writes is measured against the
/// database and against nextpnr's stated intent, and **not** against a
/// vendor artefact, which is a weaker kind of evidence than
/// `what_lattices_own_packer_writes_for_a_slew_rate` or
/// `what_lattices_own_packer_writes_for_a_constant` rest on. This test is
/// the thing that keeps that distinction honest: it is an assertion that
/// the stronger evidence is *absent*, so a future reference bitstream that
/// did contain one would fail here and say so, which is exactly when the
/// claim could be upgraded. `docs/fpga-trellis.md` states the gap in the
/// same words.
/// Everything Lattice's own packer writes for a distributed RAM, asked in
/// full and answered out of their own bitstreams for this very board.
///
/// This was expected to be the weak case. `docs/fpga-trellis.md`
/// distinguishes what a vendor artefact proves from what the database plus
/// nextpnr's intent merely suggest, and a distributed RAM looked like the
/// second kind. It is not: **`analyzer.bit` has 22 of them and
/// `facedancer.bit` 89**, and every detail of what this backend writes is
/// in their files at the absolute frame positions it would write them.
///
/// | | |
/// |---|---|
/// | How many | 22 in `analyzer.bit`, **none** in `selftest.bit`, 89 in `facedancer.bit` |
/// | `SLICEA.MODE`, `SLICEB.MODE`, `SLICEC.MODE` | `DPRAM`, `DPRAM`, `RAMW` — and in all 111 tiles **all three or none**, because they are the same bit |
/// | Which bit | `F50B11` of the `PLC2`, one bit for the whole RAM, set in their files at the frame this flow computes |
/// | `SLICEC.K0.INIT`, `SLICEC.K1.INIT` | sixteen zeros each, in all 111. The `RAMW` slice's two lookup tables hold nothing and are written anyway |
/// | `SLICEA.K0/K1.INIT`, `SLICEB.K0/K1.INIT` | sixteen zeros each, in all 111: **every distributed RAM on this board starts empty**, which is the only case this flow can produce |
/// | `SLICEA.WREMUX` | never written, in any of the three files. `WRE` is the default |
/// | `CLK1.CLKMUX` | never written either. `CLK` is the default, and the write clock's polarity is the only thing it could say |
/// | The write clock | `CLK1` of the tile, and it arrives on a **global clock network** (`G_HPBX<n>00`) in all 111 |
/// | The write enable | `LSR1` of the tile, and it arrives on **general routing** in all 111 |
/// | Slice D | still used, for ordinary logic, in 18 of analyzer's 22 and 82 of facedancer's 89. A RAM costs six lookup tables of eight, not eight |
/// | A flip-flop in a RAM's own slice | used in 8 of analyzer's tiles and 60 of facedancer's. `DPRAM` mode takes a slice's lookup tables, **not** its registers |
///
/// So a distributed RAM costs 97 bits in their bitstreams too: one for the
/// mode, 64 for the four zeroed content words and 32 for the `RAMW`
/// slice's two. That is the number
/// `a_distributed_ram_places_routes_and_every_bit_of_it_decodes` measures
/// on this flow's own output.
///
/// The last two rows are the ones that decided code rather than
/// documentation. Slice D being in use is why
/// [`trellis::DPRAM_BLOCKS`] names six bels and not eight, and a
/// flip-flop sharing a slice with the RAM is why it names no flip-flop at
/// all — a model that blocked the whole tile would have been wrong in a way
/// no test of this flow's own output could have caught.
///
/// # What those flip-flops' control wires are, which decided code too
///
/// A RAM's write enable is on `LSR1` and its write clock on `CLK1`, and a
/// tile has only `LSR0`/`LSR1` and `CLK0`/`CLK1` for its four slices. So
/// the question the placer needed answering was not "does a RAM share a
/// tile with a flip-flop" — it plainly does — but **what those flip-flops
/// do with the two wires the RAM did not leave them**. Their own
/// `MUXLSR<s>` and `MUXCLK<s>` settings say, and the two answers are
/// opposite:
///
/// | | |
/// |---|---|
/// | Slices of a RAM tile driving a reset | 51: one in `analyzer.bit`, fifty in `facedancer.bit` |
/// | …of those taking `LSR0` | **all 51** |
/// | …of those taking `LSR1`, the wire the RAM spent | **none** |
/// | Slices of a RAM tile taking a clock | 166 |
/// | …of those taking `CLK1`, the RAM's own write-clock wire | **158** |
/// | …of those taking `CLK0` | 8 |
/// | Slices of a RAM tile with a clock enable | 114, and `CE0`..`CE3` are four wires for four slices, so a RAM contends for none of them |
///
/// A reset is a signal of its own and so it takes the free wire; a clock
/// is usually the *same* net as the write clock, and a wire that already
/// carries a signal is free to the signal it carries. That is why
/// `SiteRules` in `src/fpga/place.rs` models a control wire as a budget
/// with a matching over signals rather than as an exclusion: the weak rule
/// is what the vendor's own output does, and a rule stricter than that
/// would be a wrong rule.
#[test]
fn what_lattices_own_packer_writes_for_a_distributed_ram() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let expected: [(&str, usize, usize, usize); 3] = [
        // file, distributed RAMs, of those with slice D in use, of those
        // with a flip-flop in slice A, B or C.
        ("analyzer", 22, 18, 8),
        ("selftest", 0, 0, 0),
        ("facedancer", 89, 82, 60),
    ];
    // Across all three: slices of a RAM's tile taking their reset from
    // `LSR0` and from `LSR1`, their clock from `CLK0` and from `CLK1`, and
    // slices with a clock enable at all.
    let mut lsr = [0usize; 2];
    let mut clk = [0usize; 2];
    let mut enables = 0usize;
    let mut total = 0usize;
    for (name, rams, with_slice_d, with_a_flop) in expected {
        let Some(bytes) = reference(name) else {
            return;
        };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        let mut modes: std::collections::BTreeMap<(u32, u32), Vec<String>> = Default::default();
        for (at, field, value) in &decoded.enums {
            if field.ends_with(".MODE") && (value == "DPRAM" || value == "RAMW") {
                modes
                    .entry(*at)
                    .or_default()
                    .push(format!("{field}={value}"));
            }
        }
        assert_eq!(
            modes.len(),
            rams,
            "{name}.bit: distributed RAMs. `selftest.bit` having none is asserted rather than \
             skipped, for the same reason it is in \
             `what_lattices_own_packer_writes_for_a_constant`: a design that needs none is the \
             case this flow must not change"
        );
        let mut slice_d = 0usize;
        let mut flops = 0usize;
        for (at, fields) in &modes {
            // The fixed relationship between the three slices, in a file
            // `ecppack` wrote: all three or none, never one of them.
            let mut sorted = fields.clone();
            sorted.sort();
            assert_eq!(
                sorted,
                ["SLICEA.MODE=DPRAM", "SLICEB.MODE=DPRAM", "SLICEC.MODE=RAMW"],
                "{name}.bit at X{}Y{}: a slice in one of the two modes without the other two",
                at.0,
                at.1
            );
            // And it is one bit, at the absolute frame position this flow
            // would write it.
            let ty = fabric
                .arch
                .tile_index_at(at.0, at.1)
                .expect("a tile the decoder named");
            let dpram = fabric
                .dprams
                .get(&ty)
                .unwrap_or_else(|| panic!("{name}.bit: X{}Y{} holds no `lutram` bel", at.0, at.1));
            assert_eq!(dpram.mode.len(), 1, "one bit for three slices");
            for bit in &dpram.mode {
                let (frame, index) = fabric
                    .frames
                    .locate(*at, *bit)
                    .unwrap_or_else(|| panic!("{bit:?} is outside X{}Y{}", at.0, at.1));
                assert!(
                    stream.cram.get(frame, index),
                    "F{frame}B{index}, which is the DPRAM/RAMW mode bit of X{}Y{}, is clear in \
                     {name}.bit — whose decoding says that tile holds a distributed RAM",
                    at.0,
                    at.1
                );
            }
            let word = |what: &str| -> Option<&str> {
                decoded
                    .words
                    .iter()
                    .find(|(p, w, _)| p == at && w == what)
                    .map(|(_, _, v)| v.as_str())
            };
            // The contents, and the `RAMW` slice's two words that hold
            // nothing. All six are sixteen zeros, in every one of them.
            for bel in trellis::DPRAM_DATA_LUTS
                .iter()
                .chain(trellis::DPRAM_RAMW_LUTS.iter())
            {
                let field = format!("{bel}.INIT");
                assert_eq!(
                    word(&field),
                    Some("0000000000000000"),
                    "{name}.bit at X{}Y{}: {field}",
                    at.0,
                    at.1
                );
            }
            // Two settings the database has and their packer never writes.
            for field in ["SLICEA.WREMUX", "CLK1.CLKMUX"] {
                assert!(
                    !decoded.enums.iter().any(|(p, f, _)| p == at && f == field),
                    "{name}.bit at X{}Y{}: {field} is written after all",
                    at.0,
                    at.1
                );
            }
            // Where the write clock and the write enable come from, which
            // is what `trellis::DPRAM_PINS` bets its `wclk` and `we` on.
            let source = |sink: &str| -> Option<&str> {
                decoded
                    .arcs
                    .iter()
                    .find(|(p, to, _)| p == at && to == sink)
                    .map(|(_, _, from)| from.as_str())
            };
            let wck = source("CLK1").unwrap_or_else(|| {
                panic!(
                    "{name}.bit at X{}Y{}: nothing drives CLK1, which is every DPRAM slice's \
                     write clock",
                    at.0, at.1
                )
            });
            assert!(
                wck.starts_with("G_HPBX"),
                "{name}.bit at X{}Y{}: the write clock came from {wck} rather than a global \
                 clock network",
                at.0,
                at.1
            );
            assert!(
                source("LSR1").is_some(),
                "{name}.bit at X{}Y{}: nothing drives LSR1, which is every DPRAM slice's write \
                 enable",
                at.0,
                at.1
            );
            // And what the tile's four slices do with the two reset wires
            // and the two clock wires the RAM left them one of. This is
            // the ground truth `SiteRules`' control-wire budget is built
            // on: a reset takes the wire the RAM did not spend, a clock
            // takes the RAM's own because it is the same net.
            for slice in 0..4u32 {
                match source(&format!("MUXLSR{slice}")) {
                    Some("LSR0") => lsr[0] += 1,
                    Some("LSR1") => lsr[1] += 1,
                    Some(other) => panic!("{name}.bit: MUXLSR{slice} <- {other}"),
                    None => {}
                }
                match source(&format!("MUXCLK{slice}")) {
                    Some("CLK0") => clk[0] += 1,
                    Some("CLK1") => clk[1] += 1,
                    Some(other) => panic!("{name}.bit: MUXCLK{slice} <- {other}"),
                    None => {}
                }
                if source(&format!("CE{slice}")).is_some() {
                    enables += 1;
                }
            }
            // And what the tile still does besides holding a RAM.
            if decoded
                .words
                .iter()
                .any(|(p, w, _)| p == at && w.starts_with("SLICED."))
                || decoded
                    .enums
                    .iter()
                    .any(|(p, f, _)| p == at && f.starts_with("SLICED."))
            {
                slice_d += 1;
            }
            if decoded.enums.iter().any(|(p, f, _)| {
                p == at
                    && f.contains(".REG")
                    && (f.starts_with("SLICEA")
                        || f.starts_with("SLICEB")
                        || f.starts_with("SLICEC"))
            }) {
                flops += 1;
            }
        }
        assert_eq!(
            slice_d, with_slice_d,
            "{name}.bit: RAM tiles whose slice D is still doing ordinary logic, which is why \
             `DPRAM_BLOCKS` names six bels and not eight"
        );
        assert_eq!(
            flops, with_a_flop,
            "{name}.bit: RAM tiles with a flip-flop in slice A, B or C, which is why \
             `DPRAM_BLOCKS` names no flip-flop"
        );
        total += modes.len();
    }
    assert_eq!(total, 111, "distributed RAMs across the three files");
    assert_eq!(
        lsr,
        [51, 0],
        "slices of a RAM's tile taking their reset from LSR0 and from LSR1. **Not one takes \
         LSR1**, which is the wire the RAM's write enable is joined to, and that is why a \
         distributed RAM must be modelled as spending one of the tile's two reset wires"
    );
    assert_eq!(
        clk,
        [8, 158],
        "slices of a RAM's tile taking their clock from CLK0 and from CLK1. Most take CLK1, the \
         RAM's **own** write-clock wire, because a write clock and a flip-flop's clock are \
         usually the same net — which is why the rule the placer needs is the weak one, `a \
         different signal may not share`, and not `a flip-flop may not share`"
    );
    assert_eq!(
        enables, 114,
        "slices of a RAM's tile with a clock enable. `CE0`..`CE3` are four wires for four slices \
         and a `TRELLIS_DPR16X4` has no enable pin at all, so a RAM contends for none of them and \
         the clock enable needs no rule"
    );
}

/// `ip/fifo_sync` on a part, at three depths, which is the thing a
/// distributed RAM was needed for.
///
/// Until the `lutram` bel existed this design could not be placed at all —
/// `the design needs 2 lutram site(s) and the part has 0`, at every depth,
/// while `synthesize_for` succeeded, which is why the library test passed
/// and nobody noticed. What it takes now, measured:
///
/// | Depth | `TRELLIS_DPR16X4` | Logic tiles they take | Bits they cost |
/// |---|---|---|---|
/// | 16 | 2 | 2 | 194 |
/// | 32 | 4 | 4 | 388 |
/// | 64 | 8 | 8 | 776 |
///
/// 97 bits each, and that number is not this flow's invention: it is what
/// `ecppack` writes for all 111 distributed RAMs in this board's own
/// bitstreams, measured by
/// `what_lattices_own_packer_writes_for_a_distributed_ram`. One `MODE` bit
/// for three slices, four zeroed 16-bit content words, and the `RAMW`
/// slice's two zeroed words that nothing ever reads.
///
/// Two assertions carry the weight. The **slice relationship**: a RAM is
/// slices A, B and C of one logic tile, so no lookup table may share a tile
/// with one, and a placement that broke that would produce a bitstream
/// which loads, asserts `DONE` and computes nothing — the lookup table's
/// truth table and the RAM's contents are the same `INIT` words. And
/// **every bit decodes**: every set bit of the finished image resolves
/// through the database into a feature it names, with nothing left over,
/// and the arcs those bits select are exactly the arcs the router chose.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_distributed_ram_places_routes_and_every_bit_of_it_decodes() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    // Two compositions of this die hold a `PLC2` — the plain one, 2860
    // positions of it, and `PLC2+TAP_DRIVE`, 176 — and each declares one
    // distributed RAM, which is the 3036 `lutram` sites
    // `the_database_describes_one_part_of_the_ecp5_family` counts.
    assert_eq!(
        fabric.stats.dprams,
        2,
        "every composition that holds a `PLC2` holds one `{}`",
        trellis::DPRAM_BEL
    );

    // The fixed relationship between the three slices, read out of the
    // database rather than believed: `SLICEA.MODE = DPRAM`,
    // `SLICEB.MODE = DPRAM` and `SLICEC.MODE = RAMW` are the **same single
    // bit** of a `PLC2`, and slice D has no such mode at all. That is why a
    // logic tile holds exactly one distributed RAM and why it costs six of
    // the tile's eight lookup tables.
    let plc2 = db.tile_database("PLC2").expect("the logic tile");
    let a = plc2
        .enum_bits("SLICEA.MODE", "DPRAM")
        .expect("SLICEA DPRAM");
    let b = plc2
        .enum_bits("SLICEB.MODE", "DPRAM")
        .expect("SLICEB DPRAM");
    let c = plc2.enum_bits("SLICEC.MODE", "RAMW").expect("SLICEC RAMW");
    assert_eq!(a, b, "slices A and B enter DPRAM mode together");
    assert_eq!(a, c, "and slice C enters RAMW mode with them");
    assert_eq!(a.len(), 1, "and it is one bit: {a:?}");
    assert_eq!(
        (a[0].frame, a[0].bit, a[0].inverted),
        (50, 11, false),
        "F50B11"
    );
    assert!(
        plc2.enum_bits("SLICED.MODE", "DPRAM").is_none()
            && plc2.enum_bits("SLICED.MODE", "RAMW").is_none(),
        "slice D has neither mode, so it stays available"
    );

    for (depth, rams) in [(16u32, 2usize), (32, 4), (64, 8)] {
        let top = format!("testdata/fpga/ecp5/fifo_sync_{depth}.v");
        let rcf = format!("testdata/fpga/ecp5/fifo_sync_{depth}.rcf");
        let (bits, stream, pads, routing, _, routed) =
            compile_all(&fabric, &[&top, "ip/fifo_sync/rtl/fifo_sync.v"], &rcf);

        // ---- what was placed ----
        let lutrams: Vec<usize> = routed
            .netlist
            .instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == "lutram")
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            lutrams.len(),
            rams,
            "depth {depth}: {rams} distributed RAM(s) of 16 words by 4 bits"
        );
        for index in &lutrams {
            assert_eq!(
                routed.netlist.instances[*index].primitive,
                "TRELLIS_DPR16X4"
            );
        }
        assert_eq!(
            routed.rams, rams,
            "depth {depth}: every one of them got its mode and its contents written"
        );

        // ---- the slice relationship, honoured ----
        let mut ram_tiles = std::collections::BTreeSet::new();
        for index in &lutrams {
            let site = routed.placement.site_of(*index).expect("placed");
            ram_tiles.insert(routed.graph.sites[site].tile);
        }
        assert_eq!(
            ram_tiles.len(),
            rams,
            "depth {depth}: one distributed RAM per logic tile, never two"
        );
        // Six of the eight, and exactly those six: slice D stays available,
        // which is what nextpnr's `pack_dram` leaves free too. A lookup
        // table in a RAM's tile is legal on `SLICED.K0` or `SLICED.K1` and
        // on nothing else.
        for (index, instance) in routed.netlist.instances.iter().enumerate() {
            if instance.kind != "lut" {
                continue;
            }
            let site = routed.placement.site_of(index).expect("placed");
            let site = &routed.graph.sites[site];
            if !ram_tiles.contains(&site.tile) {
                continue;
            }
            assert!(
                !trellis::DPRAM_BLOCKS.contains(&site.bel.as_str()),
                "depth {depth}: `{}` is a lookup table at {}, which a distributed RAM in that \
                 tile *is*",
                instance.name,
                site.name
            );
            assert!(
                site.bel.starts_with("SLICED."),
                "depth {depth}: `{}` at {} — only slice D is left of a tile with a RAM in it",
                instance.name,
                site.name
            );
        }
        // Said the other way round, which is the assertion that would catch
        // a legaliser that simply never looked: every one of the six bels a
        // RAM consumes is empty in every tile that holds one.
        for (index, site) in routed.graph.sites.iter().enumerate() {
            if !ram_tiles.contains(&site.tile)
                || !trellis::DPRAM_BLOCKS.contains(&site.bel.as_str())
            {
                continue;
            }
            assert!(
                routed.placement.instance_at(index).is_none(),
                "depth {depth}: {} holds `{}` and its tile holds a distributed RAM",
                site.name,
                routed.netlist.instances[routed.placement.instance_at(index).unwrap()].name
            );
        }

        // ---- it routed, and the clock went where a clock goes ----
        let io = routed
            .netlist
            .instances
            .iter()
            .filter(|i| i.kind == "io")
            .count();
        assert_eq!(pads, io, "depth {depth}: every pad configured");
        assert!(
            routed.clocks.off_network.is_empty(),
            "depth {depth}: {:?}",
            routed.clocks.off_network
        );
        assert!(
            routed.dropped.is_empty(),
            "depth {depth}: {:?}",
            routed.dropped
        );
        assert_eq!(routing.signals, routed.netlist.routable().len());

        // ---- what the RAMs cost, to the bit ----
        assert_eq!(
            routed.ram_only.ones(),
            97 * rams,
            "depth {depth}: 97 bit(s) per distributed RAM — one MODE bit, four zeroed content \
             words and the RAMW slice's two"
        );
        assert!(
            bits.ones() > routed.ram_only.ones(),
            "depth {depth}: and they are a part of the design's image, not all of it"
        );

        // ---- every bit decodes, and nothing is unexplained ----
        let decoded = db.decode(&stream.cram);
        assert_eq!(
            decoded.bits,
            stream.cram.count_ones(),
            "depth {depth}: the decoder and the writer disagree about how many bits are set"
        );
        assert_eq!(
            decoded.unexplained,
            0,
            "depth {depth}: {} of {} bit(s) belong to no feature the database names:\n{}",
            decoded.unexplained,
            decoded.bits,
            decoded.to_text()
        );
        let (selected, unresolved) = db.resolved_arcs(&decoded);
        assert!(unresolved.is_empty(), "depth {depth}: {unresolved:?}");
        assert_eq!(
            selected,
            fabric.routed_arcs(&routed.graph, &routed.routing),
            "depth {depth}: the bits select connections the router did not choose, or miss ones \
             it did"
        );
        // And the decoding names the RAM itself: three `MODE` fields per
        // tile that holds one, which the database reports as the one bit
        // they share.
        let modes = decoded
            .enums
            .iter()
            .filter(|(at, field, value)| {
                ram_tiles.contains(at) && field.ends_with(".MODE") && value != "LOGIC"
            })
            .count();
        assert_eq!(
            modes,
            3 * rams,
            "depth {depth}: SLICEA.MODE, SLICEB.MODE and SLICEC.MODE in each RAM's tile"
        );
    }
}

/// A design whose distributed RAMs and whose flip-flops want the same
/// tile's set/reset wires, which the router used to refuse.
///
/// `testdata/fpga/ecp5/lutram_reset_64.v` is four 64-word FIFOs — 32
/// `TRELLIS_DPR16X4` and about ninety flip-flops — with **two** reset
/// nets. Before the placer knew what a distributed RAM does to a tile's
/// control wires this design did not build at all:
///
/// ```text
/// routing did not converge: 1 node(s) are still oversubscribed after
/// 40 iteration(s), worst at X24Y3/LSR1 (2 signals)
/// ```
///
/// `LSR1` is one of a logic tile's two set/reset wires and a distributed
/// RAM's write enable is *joined* to it — `WRE0_SLICE` and `WRE1_SLICE`
/// are `.fixed_conn`s off `LSR1`, with no mux to choose with — so a tile
/// holding a RAM has one reset wire left and can carry one reset net. The
/// placer put flip-flops of both domains in one such tile and the router
/// was then asked to do something the fabric cannot.
///
/// What this test pins is the **weak** rule and not the strong one. It
/// asserts that a RAM's tile still holds flip-flops, that they may share
/// the RAM's own write clock, and that what is bounded is the number of
/// *distinct* signals wanting one pool of wires — which is what
/// `what_lattices_own_packer_writes_for_a_distributed_ram` measured in
/// `ecppack`'s own output.
///
/// # What it would not catch
///
/// It would not catch a placer that solved this by simply refusing to put
/// anything in a RAM's tile: that also routes, and it is the wrong answer.
/// The `flops_in_ram_tiles` assertion below is there for exactly that, and
/// it is why the number is asserted to be large rather than merely
/// non-zero — 41 of the design's 88 flip-flops are in a RAM's tile here,
/// and all 41 of them have a reset.
///
/// The strong rule was measured on this very design rather than argued
/// about: with a RAM blocking its tile's eight flip-flops as well as its
/// six lookup tables, the design still builds, on **the same 57 tiles**
/// and with 6698 pips against 6790. So on this design the weak rule buys
/// nothing, and that is worth saying plainly: what it buys is flip-flop
/// *capacity*, eight sites per RAM tile, which a design bound by its
/// lookup tables — as this one is, with 226 of them — never spends.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_distributed_ram_and_two_reset_domains_share_a_die() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, _, routing, _, routed) = compile_all(
        &fabric,
        &[
            "testdata/fpga/ecp5/lutram_reset_64.v",
            "ip/fifo_sync/rtl/fifo_sync.v",
        ],
        "testdata/fpga/ecp5/lutram_reset_64.rcf",
    );
    // That `compile_all` returned at all is the headline: it unwraps
    // `route`, and this is the design that did not converge.
    assert_eq!(routing.signals, routed.netlist.routable().len());

    let site_of =
        |index: usize| &routed.graph.sites[routed.placement.site_of(index).expect("placed")];
    let signal_of = |index: usize, role: &str| -> Option<usize> {
        routed
            .netlist
            .pins
            .iter()
            .find(|pin| pin.instance == index && pin.role == role)
            .and_then(|pin| pin.signal)
    };

    // The RAMs, and the write-enable signal each one puts on its tile's
    // `LSR1`.
    let mut rams: std::collections::BTreeMap<(u32, u32), usize> = Default::default();
    for (index, instance) in routed.netlist.instances.iter().enumerate() {
        if instance.kind != "lutram" {
            continue;
        }
        let we = signal_of(index, "we").expect("a distributed RAM's write enable is a signal");
        assert!(
            rams.insert(site_of(index).tile, we).is_none(),
            "one distributed RAM per logic tile"
        );
    }
    assert_eq!(rams.len(), 32, "four 64-word FIFOs of eight RAMs each");

    // Every flip-flop of a RAM's tile, and what it wants on the two wires
    // the RAM left it one of.
    let mut flops_in_ram_tiles = 0usize;
    let mut with_a_reset = 0usize;
    let mut sharing_the_write_clock = 0usize;
    let mut resets: std::collections::BTreeMap<(u32, u32), std::collections::BTreeSet<usize>> =
        Default::default();
    for (index, instance) in routed.netlist.instances.iter().enumerate() {
        if instance.kind != "ff" {
            continue;
        }
        let tile = site_of(index).tile;
        let Some(we) = rams.get(&tile) else { continue };
        flops_in_ram_tiles += 1;
        if let Some(reset) = signal_of(index, "rst") {
            with_a_reset += 1;
            resets.entry(tile).or_default().insert(reset);
        }
        if signal_of(index, "clk") == Some(*we) {
            // Not expected here — a write enable is not a clock — but the
            // question is asked of the clock the same way below.
            sharing_the_write_clock += 1;
        }
    }
    assert_eq!(sharing_the_write_clock, 0);
    // The point of the weak rule: a RAM's tile keeps its flip-flops.
    assert!(
        flops_in_ram_tiles >= 16,
        "only {flops_in_ram_tiles} flip-flop(s) landed in a RAM's tile. A rule that emptied a \
         RAM's tile would route too, and it would be the wrong rule: `ecppack` puts a flip-flop \
         in 79 of the 111 RAM tiles of this board's own bitstreams"
    );
    assert!(
        with_a_reset > 0,
        "no flip-flop with a reset shares a RAM's tile, so this design no longer exercises the \
         thing it was written for"
    );
    // And the constraint itself, said as the fabric says it: a tile's two
    // set/reset wires carry at most two signals, and a distributed RAM has
    // already spent one of them.
    for (tile, mut wanted) in resets {
        let we = rams[&tile];
        wanted.insert(we);
        assert!(
            wanted.len() <= 2,
            "X{}Y{} wants {} distinct signal(s) on LSR0 and LSR1, which are two wires: {:?}",
            tile.0,
            tile.1,
            wanted.len(),
            wanted
                .iter()
                .map(|s| routed.netlist.signals[*s].name.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            wanted.len(),
            2,
            "X{}Y{} holds a RAM and a resettable flip-flop, so both wires are spoken for",
            tile.0,
            tile.1
        );
    }

    // ---- placement quality ----
    let tiles: std::collections::BTreeSet<(u32, u32)> = (0..routed.netlist.instances.len())
        .filter(|index| routed.netlist.instances[*index].kind != "io")
        .map(|index| site_of(index).tile)
        .collect();
    assert_eq!(
        tiles.len(),
        57,
        "logic tiles the design occupies. The strong rule — a distributed RAM blocks its tile's \
         eight flip-flops as well as its six lookup tables — was measured on this same design and \
         needs **57 tiles too**, and 6698 pips against 6790. So the weak rule buys nothing here, \
         and saying so is the honest answer: this design is bound by its 226 lookup tables and \
         not by its flip-flops, and a RAM's tile keeps two lookup tables either way. What the \
         weak rule buys is **capacity** — the eight flip-flop sites of every tile that holds a \
         RAM, 24288 of them on this part — which is what a design with 530 RAMs and a thousand \
         flip-flops spends and this one does not"
    );

    // ---- every bit decodes, and nothing is unexplained ----
    let decoded = db.decode(&stream.cram);
    assert_eq!(decoded.bits, stream.cram.count_ones());
    assert_eq!(
        decoded.unexplained, 0,
        "{} of {} bit(s) belong to no feature the database names",
        decoded.unexplained, decoded.bits
    );
    let (selected, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    assert_eq!(
        selected,
        fabric.routed_arcs(&routed.graph, &routed.routing),
        "the bits select connections the router did not choose, or miss ones it did"
    );
    assert!(routed.dropped.is_empty(), "{:?}", routed.dropped);
    assert!(
        routed.clocks.off_network.is_empty(),
        "{:?}",
        routed.clocks.off_network
    );
}

/// What the database describes, measured rather than believed. These are
/// the numbers `docs/fpga-trellis.md` quotes.
#[test]
fn the_database_describes_one_part_of_the_ecp5_family() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    assert_eq!(db.device().name, PART);
    assert_eq!(db.device().idcode, IDCODE);
    assert_eq!(db.device().format.frames, FRAMES);
    assert_eq!(db.device().format.bits_per_frame, BITS_PER_FRAME);
    assert_eq!(db.device().format.bytes_per_frame(), 74);
    // 73 columns by 51 rows, from `max_col` and `max_row`.
    assert_eq!((db.device().width(), db.device().height()), (73, 51));
    // Three packages, and the one a Cynthion has.
    assert_eq!(
        db.packages(),
        vec!["CABGA256", "CABGA381", "CSFBGA285", "TQFP144"],
        "the package names are upper case and have no hyphen, unlike `devices.json`'s"
    );
    // 4312 tiles of 134 distinct types. The family ships 185 `bits.db`
    // files; this part's grid uses 134 of them.
    assert_eq!(db.size(), (4312, 134));

    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let stats = fabric.stats;
    assert_eq!(stats.tiles, 4312);
    // Every position of the 73x51 grid holds at least one tile.
    assert_eq!(stats.positions, 73 * 51);
    assert_eq!(stats.shared_positions, 480);
    assert_eq!(
        stats.most_windows, 6,
        "the most tiles Project Trellis puts at one position"
    );
    assert_eq!(stats.tile_types, 129, "one per composition of a position");
    assert_eq!(
        u64::from(stats.frames) * u64::from(stats.bits_per_frame),
        4_476_704,
        "the measured size of the fabric"
    );
    assert_eq!(stats.balls, 197, "balls the caBGA-256 map names");
    assert_eq!(
        stats.pads, 197,
        "56 on the top edge, 64 each on the left and the right, 13 on the bottom"
    );
    // **Nothing is left out any more**, and that is worth an assertion of
    // its own rather than a zero nobody reads: every ball `iodb.json` names
    // for this package is on one of the four edges, every one of them has
    // its tiles where its edge's rule says, and every one has a bank in
    // `pio_metadata`. Before the left and bottom edges were described, 77
    // balls — every pin of the TARGET USB transceiver, the HyperRAM, the
    // three VBUS switches and the SPI flash among them — were in the
    // package's map and in no fabric.
    assert_eq!(stats.pads_skipped, 0, "no ball of this package is left out");

    // ---- the interconnect, measured ----
    //
    // These are the numbers `docs/fpga-trellis.md` quotes for the size of
    // the routing graph, and they are all `bits.db` arithmetic: 171 632
    // `.mux` sources and 14 614 `.fixed_conn`s declared over 129 tile
    // types, which expand to one graph edge per tile of each type.
    assert_eq!(stats.globals, 467, "wires that reach the whole die");
    assert_eq!(stats.wires, 1_095_958);
    assert_eq!(stats.arcs, 171_632);
    assert_eq!(stats.fixed, 14_614);
    // The clock network's own numbers. The joins are the connections
    // `bits.db` states nowhere because the network's wires carry the same
    // name in every tile they cross, so they come out of `globals.json`'s
    // geometry instead; `trellis::ClockNetwork` says which three hops they
    // are and why there is no fourth.
    assert_eq!(stats.clock_networks, 16, "G_HPBX0000 to G_HPBX1500");
    assert_eq!(stats.joins, 58_928);
    assert_eq!(
        stats.buffers, 56,
        "twelve at the top, fourteen on each side, sixteen at the bottom"
    );
    assert_eq!(
        stats.clears, 4425,
        "`.mux` sources of this die's tile types that want a bit clear"
    );
    assert_eq!(
        stats.clear_collisions, 6,
        "six of them share their set bits with another source of the same tile type that wants \
         different bits clear. Those are indistinguishable in a finished bitstream whatever is \
         recorded, so only what the two agree about is kept and the number is pinned here rather \
         than hidden: it is what the check in `dropped_clear_bits` cannot see"
    );
    assert_eq!(stats.luts, 16, "eight per composition that has a PLC2");
    assert_eq!(stats.ffs, 16, "and eight flip-flops beside them");
    assert_eq!(
        stats.references_off_the_grid, 3840,
        "a neighbour's wire beyond the edge of the die, which is not an error"
    );
    // The text form is what a document quotes, so it has to hold the
    // numbers rather than a summary of them.
    let text = stats.to_text();
    assert!(text.contains("configuration bits: 4476704"), "{text}");
    assert!(text.contains("tiles: 4312"), "{text}");
    assert!(text.contains("programmable connections: 171632"), "{text}");
    assert!(text.contains("clock networks: 16"), "{text}");

    // And what the graph costs, which is what decides whether a whole die
    // fits. It does, comfortably, which is the surprise: the 7-series
    // fabric needed a region and a pip limit to be loadable at all and this
    // one is a third of a gigabyte.
    let graph = fabric.arch.build_graph();
    assert_eq!(
        graph.nodes.len(),
        1_096_425,
        "467 globals and the rest tile wires"
    );
    assert_eq!(
        graph.pips.len(),
        8_211_900 + 58_928,
        "the declared connections, plus the clock network's implicit joins"
    );
    assert_eq!(
        graph.dangling, 53_632,
        "edges that leave the grid, which happens at all four edges — and not one of the clock \
         network's joins, every one of which was checked against the grid before it was declared"
    );
    assert_eq!(
        graph.bit_patterns(),
        3536,
        "distinct bit patterns, interned once for the whole die"
    );
    assert_eq!(
        graph.site_counts(),
        vec![
            ("ff".to_owned(), 24_288),
            ("gb".to_owned(), 56),
            // One `io` site per ball the package names, now that all four
            // edges are described.
            ("io".to_owned(), 197),
            ("lut".to_owned(), 24_288),
            // One distributed RAM per logic tile, and 24288 / 8 is 3036 of
            // them: a `TRELLIS_DPR16X4` is three slices of one tile, so a
            // tile can hold exactly one. See `trellis::DPRAM_PINS`.
            ("lutram".to_owned(), 3036)
        ]
    );
    // A ratio between two sizes in one process, not a wall clock: the
    // whole-die graph is under a kilobyte per node, which is what makes it
    // fit at all.
    assert!(
        graph.heap_bytes() / graph.nodes.len() < 1024,
        "{} bytes for {} nodes",
        graph.heap_bytes(),
        graph.nodes.len()
    );
}

/// The identifier check, which on this family is not a nicety: the
/// LFE5U-12F and the LFE5U-25F are the same die, and only the identifier
/// tells them apart.
///
/// `devices.json` is where that is visible: the two entries differ in the
/// `idcode` field and in nothing else. Only the 12F's per-part files are
/// in the pinned download — they would be a second copy of the same bytes
/// — so this reads the shared file rather than opening both parts.
#[test]
fn the_same_die_serves_two_parts_and_only_the_idcode_differs() {
    let Some(root) = chipdb() else { return };
    let text = std::fs::read_to_string(format!("{root}/devices.json")).unwrap();
    let parts = trellis::parse::devices(&text).unwrap();
    let find = |name: &str| {
        parts
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no {name} in devices.json"))
            .clone()
    };
    let twelve = find("LFE5U-12F");
    let twenty_five = find("LFE5U-25F");
    assert_eq!(twelve.idcode, 0x2111_1043);
    assert_eq!(twenty_five.idcode, 0x4111_1043);
    // Everything else about them is the same, which is the point.
    assert_eq!(twelve.format, twenty_five.format);
    assert_eq!(twelve.max_row, twenty_five.max_row);
    assert_eq!(twelve.max_col, twenty_five.max_col);
    assert_eq!(twelve.packages, twenty_five.packages);
    // And the prefix that picks out the wires only some dice have is the
    // same for both, because it is a property of the die.
    assert_eq!(twelve.chip_prefix(), "25K_");
    assert_eq!(twenty_five.chip_prefix(), "25K_");
    // The 45F is a different die and says so.
    assert_eq!(find("LFE5U-45F").chip_prefix(), "45K_");

    let Some(fabric) = open() else { return };
    fabric.check_idcode(0x2111_1043).unwrap();
    let err = fabric.check_idcode(0x4111_1043).unwrap_err();
    assert!(err.to_string().contains("0x41111043"), "{err}");
}

/// Each of Great Scott Gadgets' three bitstreams for this board is read
/// and written back **identical byte for byte**.
///
/// That is the whole check on the container, and it is a strong one: a
/// wrong compression dictionary, a wrong frame order, a wrong bit packing
/// or one wrong check word out of 7562 all fail it.
#[test]
fn the_reference_bitstreams_round_trip_byte_for_byte() {
    let Some(analyzer) = reference("analyzer") else {
        return;
    };
    // The measurements, so the module header's table cannot drift.
    let expected = [
        ("analyzer", 238_282usize, 250_001usize, 9usize),
        ("selftest", 112_303, 27_006, 0),
        ("facedancer", 399_402, 424_160, 44),
    ];
    for (name, bytes, ones, brams) in expected {
        let Some(file) = (if name == "analyzer" {
            Some(analyzer.clone())
        } else {
            reference(name)
        }) else {
            continue;
        };
        assert_eq!(file.len(), bytes, "{name} is not the file this pins");
        let stream =
            Ecp5Stream::parse(&file, &formats).unwrap_or_else(|err| panic!("{name}.bit: {err}"));
        assert_eq!(stream.idcode, IDCODE, "{name} is for another part");
        assert!(stream.was_compressed, "{name} is compressed");
        assert_eq!(stream.cram.count_ones(), ones, "{name}: set bits");
        assert_eq!(stream.bram.len(), brams, "{name}: block RAM blocks");
        assert_eq!(
            stream.metadata,
            vec!["Part: LFE5U-12F-8CABGA256".to_owned()]
        );
        assert_eq!(
            stream.to_bytes(true),
            file,
            "{name}.bit does not come back out the way it went in"
        );
    }
}

/// **The check that turns a rule into a measurement.** All six of a
/// Cynthion's FPGA LEDs are outputs in the board's own analyzer gateware,
/// so every bit this crate would set to make one an output must already be
/// set, at the same absolute frame, in a bitstream Lattice's own packer
/// wrote.
///
/// What that settles is the part nothing else could: which tile at which
/// position holds a pad's configuration. It is nextpnr's rule — side A's
/// tiles at the ball's column, side B's one column east — and it is the
/// kind of rule that is easy to get one column wrong and impossible to
/// notice, because a bitstream with a pad configured next door still
/// loads and still asserts `DONE`.
#[test]
fn the_led_pads_are_where_this_boards_own_gateware_has_them() {
    let Some(fabric) = open() else { return };
    let Some(file) = reference("analyzer") else {
        return;
    };
    let gateware = Ecp5Stream::parse(&file, &formats).unwrap();

    for (led, ball) in LEDS {
        let pad = fabric
            .pad(ball)
            .unwrap_or_else(|| panic!("no pad at {ball}"));
        // Six bits in the pad tile and two in the tile one row south.
        assert_eq!(pad.output_pad_bits.len(), 6, "LED{led} {ball}");
        assert_eq!(pad.output_pic_bits.len(), 2, "LED{led} {ball}");
        for (what, at, bits) in [
            ("the pad tile", pad.pad_at, &pad.output_pad_bits),
            ("the tile one row south", pad.pic_at, &pad.output_pic_bits),
        ] {
            for bit in bits {
                let (frame, index) = fabric
                    .frames
                    .locate(at, *bit)
                    .unwrap_or_else(|| panic!("LED{led}: {bit:?} is outside {at:?}"));
                assert!(
                    gateware.cram.get(frame, index),
                    "LED{led} ({ball}, side {}, bel {:?}): F{frame}B{index} of {what} is set by \
                     this crate for an output and is clear in the board's own gateware, which \
                     drives this LED. The tile rule is wrong.",
                    pad.side,
                    pad.bel
                );
            }
        }
        // The ties are *not* expected to agree: the gateware routes a
        // signal into the data wire where this crate ties a constant, so
        // both of its constant-mux bits are clear there. Asserting that
        // keeps the comparison above honest about what it covers.
        for bit in pad.low_bits.iter().chain(&pad.high_bits) {
            let (frame, index) = fabric.frames.locate(pad.cib_at, *bit).unwrap();
            assert!(
                !gateware.cram.get(frame, index),
                "the gateware drives LED{led} from logic, so it ties nothing"
            );
        }
        // Tying high and tying low are different bits.
        assert_ne!(pad.low_bits, pad.high_bits);
    }
}

/// Every pad lands at a distinct site, its bits are where its edge's rule
/// says, and the tile it sits in owns the three wires its buffer presents.
///
/// That last clause is what a routed design added. A pad's *bits* and a
/// pad's *bel* are at different positions on both of these edges, and while
/// nothing routed only the bits mattered.
#[test]
fn a_pads_bel_is_at_its_ball_and_its_bits_are_where_the_edge_rule_says() {
    let Some(fabric) = open() else { return };
    let graph = fabric.arch.build_graph();
    let mut seen: Vec<(u32, u32, char)> = Vec::new();
    let mut top = 0usize;
    let mut right = 0usize;
    let mut left = 0usize;
    let mut bottom = 0usize;
    for pad in &fabric.io {
        assert!(
            !seen.contains(&(pad.bel.0, pad.bel.1, pad.side)),
            "two balls claim {}",
            pad.site_name()
        );
        seen.push((pad.bel.0, pad.bel.1, pad.side));
        // Every pad's own site name round trips through the ball map.
        let name = pad.site_name();
        assert_eq!(fabric.arch.site_of_pin(&pad.ball), Some(name.as_str()));
        assert_eq!(fabric.pad_of_site(&name).map(|p| &p.ball), Some(&pad.ball));
        // The tiles it needs really exist at those positions.
        for at in [pad.bel, pad.pad_at, pad.pic_at, pad.cib_at] {
            assert!(
                fabric.arch.tile_at(at.0, at.1).is_some(),
                "{} wants a tile at {at:?}",
                pad.ball
            );
        }
        // And the bel really is a site, with all three of its pins wired.
        let site = graph.site(&name).expect("the bel becomes a site");
        for role in ["dout", "oe", "din"] {
            assert!(site.pin(role).is_some(), "{} has no {role}", pad.ball);
        }
        assert!(site.pin("pad").is_none(), "a ball is not a wire");
        match pad.edge {
            trellis::Edge::Top => {
                top += 1;
                assert_eq!(pad.bel.1, 0);
                assert_eq!(pad.pad_at, (pad.bel.0 + u32::from(pad.side == 'B'), 0));
                assert_eq!(pad.pic_at, (pad.pad_at.0, 1));
            }
            trellis::Edge::Right => {
                right += 1;
                assert_eq!(pad.bel.0, fabric.arch.width - 1);
                assert_eq!(pad.pad_at, (pad.bel.0, pad.bel.1 + 1));
                let south = if matches!(pad.side, 'A' | 'B') { 0 } else { 2 };
                assert_eq!(pad.pic_at, (pad.bel.0, pad.bel.1 + south));
            }
            // The left edge takes the right edge's rows unchanged — not
            // mirrored, which is the thing a reader will expect and which
            // would have been wrong. Only the column differs.
            trellis::Edge::Left => {
                left += 1;
                assert_eq!(pad.bel.0, 0);
                assert_eq!(pad.pad_at, (0, pad.bel.1 + 1));
                let south = if matches!(pad.side, 'A' | 'B') { 0 } else { 2 };
                assert_eq!(pad.pic_at, (0, pad.bel.1 + south));
                // And its `CIB` is one column *east*, from the buffer's own
                // `JPADDO<s> <- E1_JA0`, where the right edge's is west.
                assert_eq!(pad.cib_at.0, 1, "{}: the CIB is at column 1", pad.ball);
            }
            // The bottom edge is the top edge's column rule with the two
            // tiles collapsed into one, so there is no second copy.
            trellis::Edge::Bottom => {
                bottom += 1;
                assert_eq!(pad.bel.1, fabric.arch.height - 1);
                assert_eq!(
                    pad.pad_at,
                    (pad.bel.0 + u32::from(pad.side == 'B'), pad.bel.1)
                );
                assert_eq!(
                    pad.pic_at, pad.pad_at,
                    "{}: `PICB<n>` is the only tile of this edge",
                    pad.ball
                );
                assert_eq!(pad.output_pic_bits, pad.output_pad_bits);
                // The `CIB` is one row *north*, from `JPADDO<s> <- N1_JA0`.
                assert_eq!(pad.cib_at, (pad.pad_at.0, pad.bel.1 - 1), "{}", pad.ball);
            }
        }
    }
    // Every ball of the package that is a PIO, on all four edges: 56 on the
    // top, 64 each on the left and the right, and 13 on the bottom, which
    // on a caBGA-256 is bank 8's configuration pins and nothing else.
    assert_eq!(top, 56);
    assert_eq!(right, 64);
    assert_eq!(left, 64);
    assert_eq!(bottom, 13);
    assert_eq!(seen.len(), 197);

    // A ball that is not a PIO is not in the map at all, rather than being
    // in it and configured nowhere. `T17` is a corner ball of this package.
    assert!(fabric.pad("T17").is_none());
    assert!(fabric.arch.site_of_pin("no-such-ball").is_none());
    // And the left-edge ball this whole edge was described for is in it:
    // `R2` is `target_phy.data[0]` in Great Scott Gadgets' own platform
    // file, PIO C of (col 0, row 38), bank 6.
    let r2 = fabric.pad("R2").expect("the left edge is described now");
    assert_eq!(
        (r2.bel, r2.side, r2.edge),
        ((0, 38), 'C', trellis::Edge::Left)
    );
    assert_eq!((r2.pad_at, r2.pic_at, r2.bank), ((0, 39), (0, 40), 6));
}

/// The whole flow, from Verilog to a `.bit`: the design that lights all
/// six LEDs, which is the one that has been loaded into a part.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_all_on_design_compiles_to_a_bitstream_the_container_reads_back() {
    let Some(fabric) = open() else { return };
    let (bits, stream, pads, routing, _, _) = compile(
        &fabric,
        "testdata/fpga/cynthion/leds.v",
        "testdata/fpga/cynthion/leds.rcf",
    );
    assert_eq!(pads, 6, "one pad per LED");
    assert_eq!(routing.pips, 0, "this design still has nothing to route");
    // Six pads, each: six pad-tile bits, two in the tile south, one
    // output enable tied low, one data wire tied low. Plus **one** for
    // the bank: all six LEDs are in bank 1, so `BANK.VCCIO` is written
    // once, in a tile none of the pads owns.
    assert_eq!(bits.ones(), 6 * 10 + 1);
    assert_eq!(stream.cram.count_ones(), 6 * 10 + 1);
    assert_eq!(stream.idcode, IDCODE);
    assert_eq!(
        stream.metadata,
        vec!["Part: LFE5U-12F-8CABGA256".to_owned()]
    );

    // Every LED's data wire is tied **low**, which is what lights an
    // active-low LED, and none is tied high.
    for (led, ball) in LEDS {
        let pad = fabric.pad(ball).unwrap();
        for bit in &pad.low_bits {
            let (f, b) = fabric.frames.locate(pad.cib_at, *bit).unwrap();
            assert!(stream.cram.get(f, b), "LED{led} is not tied low");
        }
        for bit in &pad.high_bits {
            let (f, b) = fabric.frames.locate(pad.cib_at, *bit).unwrap();
            assert!(!stream.cram.get(f, b), "LED{led} is also tied high");
        }
    }

    // The file comes back out the way it went in, which is the only check
    // that the writer and the frame map agree.
    let bytes = stream.to_bytes(true);
    let back = Ecp5Stream::parse(&bytes, &formats).unwrap();
    assert_eq!(back.cram, stream.cram);
    assert_eq!(back.idcode, stream.idcode);
    assert_eq!(back.metadata, stream.metadata);
    // And compression is worth having: 4.5 million configuration bits in
    // under 150 kB.
    assert!(bytes.len() < 150_000, "{} bytes", bytes.len());
}

/// The same flow with a different constant: **each pad carries the value
/// its own pin was given**, which is the property the all-on design
/// cannot demonstrate on its own.
///
/// This is how a real bug was found. Before the pad bel declared its
/// pins, `Netlist::build` recorded no pin for an output's data input at
/// all, so the constant never reached the bitstream and both designs
/// produced the same 60 bits. Now they differ, and they differ in exactly
/// the right places.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn each_pad_carries_the_constant_its_own_pin_was_given() {
    let Some(fabric) = open() else { return };
    let (_, stream, pads, _, _, _) = compile(
        &fabric,
        "testdata/fpga/cynthion/leds_alternate.v",
        "testdata/fpga/cynthion/leds.rcf",
    );
    assert_eq!(pads, 6);
    // Three pads tied low (one bit each) and three tied high (two bits
    // each), on top of the 54 that do not depend on the value and the one
    // that is the bank's.
    assert_eq!(stream.cram.count_ones(), 54 + 1 + 3 + 6);

    for (led, ball) in LEDS {
        // `6'b101010`: bit 0 is zero, so LED 0 is lit, and so on.
        let lit = led % 2 == 0;
        let pad = fabric.pad(ball).unwrap();
        let set = |bits: &[ConfigBit]| {
            bits.iter().all(|bit| {
                let (f, b) = fabric.frames.locate(pad.cib_at, *bit).unwrap();
                stream.cram.get(f, b)
            })
        };
        assert_eq!(
            set(&pad.low_bits),
            lit,
            "LED{led} ({ball}) should be {}",
            if lit {
                "lit (tied low)"
            } else {
                "dark (tied high)"
            }
        );
        assert_eq!(set(&pad.high_bits), !lit, "LED{led} ({ball})");
    }
}

/// **The bank setting no pad's tiles hold.** All six LEDs are in bank 1,
/// whose reference tile sits at the far end of the top edge, and the bit
/// that says the rail is 3.3 V is the one thing `configure_io` was missing
/// when the first `.bit` this project built went into a part.
///
/// It is checked the only way it can be: against what Lattice's own packer
/// wrote for the same board. All three of Great Scott Gadgets' bitstreams
/// set it, and what this crate sets for `leds.v` must be among what each
/// of them sets — among, and not equal to, because their designs use IOs
/// on all four edges and this one uses six pads on one.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_bank_rail_is_the_bit_lattices_own_packer_sets_for_this_board() {
    let Some(fabric) = open() else { return };
    // Every LED is in bank 1, from `iodb.json`'s own `pio_metadata`.
    for (led, ball) in LEDS {
        assert_eq!(fabric.pad(ball).unwrap().bank, 1, "LED{led} ({ball})");
    }
    // The top edge is two banks, and the value is the one the RCF's
    // `LVCMOS33` implies.
    assert_eq!(fabric.voltage, "3V3");
    // **Seven banks, which is every bank this die has a `BANKREF` for**,
    // because the fabric declares pads on all four edges now: 0 and 1 are
    // the top edge, 2 and 3 the right, 6 and 7 the left, and 8 the bottom —
    // which on a caBGA-256 is bank 8's thirteen configuration pins and is
    // also where the part's sysconfig settings live. Note that 2 and 7 are
    // spelled `BANKREF2A` and `BANKREF7A` by Lattice, which is why the
    // lookup tries both spellings. There is no bank 4 or 5 on this die.
    assert_eq!(
        fabric.bank_bits.keys().copied().collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 6, 7, 8]
    );
    let (at, bank1) = fabric.bank_bits.get(&1).unwrap();
    assert_eq!(bank1.len(), 1, "one bit says the rail");

    let (_, stream, _, _, _, _) = compile(
        &fabric,
        "testdata/fpga/cynthion/leds.v",
        "testdata/fpga/cynthion/leds.rcf",
    );
    for bit in bank1 {
        let (frame, index) = fabric.frames.locate(*at, *bit).unwrap();
        assert!(
            stream.cram.get(frame, index),
            "the bitstream does not set bank 1's rail at F{frame}B{index}"
        );
        for name in ["analyzer", "selftest", "facedancer"] {
            let Some(file) = reference(name) else { return };
            let theirs = Ecp5Stream::parse(&file, &formats).unwrap();
            assert!(
                theirs.cram.get(frame, index),
                "{name}.bit does not set F{frame}B{index}, so this is not BANK.VCCIO"
            );
        }
    }
    // Bank 0 is the other half of the top edge and has no pad in this
    // design, so its rail is **not** written. That is what Lattice's own
    // packer does: the banks a design uses, and no others.
    let (at0, bank0) = fabric.bank_bits.get(&0).unwrap();
    assert_ne!(at0, at, "the two banks' tiles are different positions");
    for bit in bank0 {
        let (frame, index) = fabric.frames.locate(*at0, *bit).unwrap();
        assert!(
            !stream.cram.get(frame, index),
            "bank 0 has no pad in this design and should not be written"
        );
    }
}

/// A wire from one pad to another **routes**, where it used to be refused.
///
/// This test used to be the guard: the fabric declared no interconnect, so
/// a design that needed some was named and rejected before a file was
/// written. It is rewritten rather than deleted because what it pins is the
/// same question with the opposite answer, and the answer is the milestone.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_wire_from_one_pad_to_another_now_routes() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{Netlist, PlaceOptions, place};
    use reticle::fpga::route::{RouteOptions, route};
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::source::SourceMap;

    let Some(fabric) = open() else { return };
    let verilog = "module wire_through(input a, output y); assign y = a; endmodule\n";
    let rcf = "set_io -io_standard LVCMOS33 a B14\nset_io -io_standard LVCMOS33 y E13\n";
    let mut map = SourceMap::new();
    let source = map.add("wire_through.v", verilog).unwrap();
    let rcf_file = map.add("wire_through.rcf", rcf).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .unwrap();
    let top = design.top.unwrap();
    let device = target(DEVICE).unwrap();
    let mut constraints = Constraints::parse(rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    // One signal: the input buffer's `O` to the output buffer's `I`. Two
    // pads on the top edge, four columns apart.
    assert_eq!(netlist.routable().len(), 1);
    let (placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let (routing, report) = route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
    assert_eq!(report.signals, 1);
    assert!(report.pips > 0, "a route with no pips is not a route");
    assert!(
        routing.verify(&netlist, &graph, &placement).is_empty(),
        "the route does not join the driver to the sink"
    );
    // And every pip it took has a home: a bit pattern, empty or not, in a
    // tile the bitstream has.
    for route in routing.routes() {
        for pip in &route.pips {
            let at = graph.pip(*pip).tile;
            assert!(fabric.arch.tile_at(at.0, at.1).is_some());
        }
    }
}

/// **Where the USER button's bits are, checked against Lattice's own
/// packer.** This is the right edge's version of the LED comparison, and
/// the reason the right edge could be described at all.
///
/// Of Great Scott Gadgets' three bitstreams for this board exactly one
/// reads the button — `facedancer.bit`, whose gateware asks for
/// `button_user` through its `ButtonProvider` — so it is the one that says
/// where an input on the right edge is configured. Every bit this crate
/// sets to make M14 an input must already be set, at the same absolute
/// frame, in that file; and the two that must be *clear* must be clear.
///
/// What it settles is the part nothing else could: that a right-edge PIO's
/// `BASE_TYPE` is one row south of its ball, that a `PIOD`'s second copy is
/// two rows south rather than at the ball's own row, and that an input
/// costs `HYSTERESIS` and `PULLMODE` on top of the base type. Getting any
/// of those wrong gives a bitstream that loads, asserts `DONE`, and reads
/// a pin nobody is pressing.
#[test]
fn the_user_buttons_pad_is_where_this_boards_own_gateware_has_it() {
    let Some(fabric) = open() else { return };
    let Some(file) = reference("facedancer") else {
        return;
    };
    let gateware = Ecp5Stream::parse(&file, &formats).unwrap();
    let pad = fabric.pad(BUTTON).expect("the USER button is a pad");

    // The right edge, PIO D of the last column, in bank 3.
    assert_eq!(pad.edge, trellis::Edge::Right);
    assert_eq!((pad.side, pad.bel), ('D', (72, 32)));
    assert_eq!(pad.pad_at, (72, 33));
    assert_eq!(
        pad.pic_at,
        (72, 34),
        "a PIOD's second copy is two rows south"
    );
    assert_eq!(pad.bank, 3);
    // Five bits say the base type and one of them is also the hysteresis;
    // the second copy costs nothing at all, which is why `PICR2` shows no
    // bits for this pad in either bitstream.
    assert_eq!(pad.input_pad_bits.len(), 5);
    assert_eq!(pad.input_pic_bits.len(), 0);
    assert_eq!(pad.hysteresis_bits.len(), 1);
    assert_eq!(pad.pull_bits(trellis::PULL_NONE).len(), 1);

    for (what, bits) in [
        ("the base type", &pad.input_pad_bits[..]),
        ("hysteresis", &pad.hysteresis_bits[..]),
        ("the pull mode", pad.pull_bits(trellis::PULL_NONE)),
    ] {
        for bit in bits {
            let (frame, index) = fabric
                .frames
                .locate(pad.pad_at, *bit)
                .unwrap_or_else(|| panic!("{bit:?} is outside {:?}", pad.pad_at));
            assert!(
                gateware.cram.get(frame, index),
                "F{frame}B{index}, which this crate sets for {what} of an input on {BUTTON}, is \
                 clear in facedancer.bit, whose gateware reads that very button. The right \
                 edge's tile rule is wrong."
            );
        }
    }
    // An input ties nothing: no tristate and no constant. Lattice's own
    // packer does not write those for an input either, and the bits are in
    // a tile a route would use, so writing them would be worse than
    // useless.
    for bit in pad
        .enable_bits
        .iter()
        .chain(&pad.low_bits)
        .chain(&pad.high_bits)
    {
        let (frame, index) = fabric.frames.locate(pad.cib_at, *bit).unwrap();
        assert!(
            !gateware.cram.get(frame, index),
            "facedancer.bit ties {BUTTON}'s CIB wires, which an input should not"
        );
    }
}

/// The milestone design: the USER button through the fabric and a lookup
/// table to two LEDs.
///
/// What it pins is every claim `button_led.v`'s header makes about the
/// bitstream, at absolute frame positions: the button configured as an
/// input with the three settings an input needs, two signals routed, one
/// lookup table holding the inverting truth table, LED 0 driven by the
/// route rather than tied, and LEDs 2 to 5 tied high.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_button_design_routes_and_configures_what_its_header_promises() {
    let Some(fabric) = open() else { return };
    let (bits, stream, pads, routing, io_only, _) = compile(
        &fabric,
        "testdata/fpga/cynthion/button_led.v",
        "testdata/fpga/cynthion/button_led.rcf",
    );
    assert_eq!(pads, 7, "six LEDs and the button");
    // Two signals: the button's, which fans out to LED 0 and to the lookup
    // table, and the table's output.
    assert_eq!(routing.signals, 2);
    assert!(routing.pips >= 10, "{} pips", routing.pips);

    let set = |at: (u32, u32), bit: &ConfigBit| {
        let (frame, index) = fabric
            .frames
            .locate(at, *bit)
            .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
        stream.cram.get(frame, index)
    };

    // The button: an input, with hysteresis and no pull.
    let button = fabric.pad(BUTTON).unwrap();
    for bit in button
        .input_pad_bits
        .iter()
        .chain(&button.hysteresis_bits)
        .chain(button.pull_bits(trellis::PULL_NONE))
    {
        assert!(set(button.pad_at, bit), "the button is not an input");
    }
    // And it is not also an output: the one bit `OUTPUT_LVCMOS33` needs
    // that `INPUT_LVCMOS33` does not is clear.
    let only_output: Vec<_> = button
        .output_pad_bits
        .iter()
        .filter(|bit| !button.input_pad_bits.contains(bit))
        .collect();
    assert!(!only_output.is_empty());
    assert!(
        only_output.iter().any(|bit| !set(button.pad_at, bit)),
        "the button is configured as an output as well"
    );

    // LED 0 and LED 1 are driven by the routing, so `configure_io` ties
    // neither of them. It has to be asked directly: the constant mux and
    // the routing mux of a `CIB` wire are one mux, so a tie's bits and a
    // route's bits overlap and the finished bitstream cannot tell them
    // apart. That overlap is the whole reason this pass has to know.
    for ball in ["E13", "C13"] {
        let pad = fabric.pad(ball).unwrap();
        for bit in pad.low_bits.iter().chain(&pad.high_bits) {
            assert_eq!(
                io_only.get(pad.cib_at, *bit),
                Some(false),
                "{ball} is driven by a route and tied as well"
            );
        }
        // Its tristate is still tied low, which is what makes it drive.
        for bit in &pad.enable_bits {
            assert!(set(pad.cib_at, bit), "{ball} does not drive");
            assert_eq!(io_only.get(pad.cib_at, *bit), Some(true));
        }
    }
    // LEDs 2 to 5 are tied **high**, which is dark.
    for ball in ["B14", "A15", "D12", "C11"] {
        let pad = fabric.pad(ball).unwrap();
        assert!(!pad.high_bits.is_empty());
        for bit in &pad.high_bits {
            assert!(set(pad.cib_at, bit), "{ball} is not tied high");
            assert_eq!(io_only.get(pad.cib_at, *bit), Some(true));
        }
        for bit in &pad.low_bits {
            assert_eq!(
                io_only.get(pad.cib_at, *bit),
                Some(false),
                "{ball} is also tied low"
            );
        }
    }
    // And nothing at all was written into the button's `CIB`: an input ties
    // neither its data nor its tristate.
    for bit in button
        .enable_bits
        .iter()
        .chain(&button.low_bits)
        .chain(&button.high_bits)
    {
        assert_eq!(io_only.get(button.cib_at, *bit), Some(false));
    }

    // Two banks are used — 1 for the LEDs and 3 for the button — and both
    // rails are written. Banks 0 and 2 have no pad in this design and are
    // left alone, which is what Lattice's own packer does.
    for (bank, wanted) in [(1u32, true), (3, true), (0, false), (2, false)] {
        let (at, rail) = fabric.bank_bits.get(&bank).unwrap();
        for bit in rail {
            assert_eq!(set(*at, bit), wanted, "bank {bank}'s rail");
        }
    }

    // Exactly one lookup table, holding `~A`: `INIT` is `0x5555`, and the
    // word is stored inverted, so the bits that are *set* are the ones
    // where the truth table is zero — the odd addresses.
    let luts: Vec<_> = bits
        .used_tiles()
        .into_iter()
        .filter(|(format, _)| {
            fabric
                .arch
                .tile_at(format.tile.0, format.tile.1)
                .is_some_and(|ty| ty.name.split('+').any(|t| t == trellis::LOGIC_TILE))
        })
        .collect();
    let holding_a_table: Vec<_> = luts
        .iter()
        .filter(|(format, set)| {
            let index = fabric
                .arch
                .tile_index_at(format.tile.0, format.tile.1)
                .unwrap();
            fabric.luts.iter().any(|((ti, _), lut)| {
                *ti == index && lut.init_zero.iter().flatten().any(|b| set.contains(b))
            })
        })
        .collect();
    assert_eq!(holding_a_table.len(), 1, "one lookup table");
    let (format, _) = holding_a_table[0];
    let index = fabric
        .arch
        .tile_index_at(format.tile.0, format.tile.1)
        .unwrap();
    let (_, lut) = fabric
        .luts
        .iter()
        .find(|((ti, _), lut)| {
            *ti == index
                && lut
                    .init_zero
                    .iter()
                    .flatten()
                    .any(|b| bits.get(format.tile, *b) == Some(true))
        })
        .unwrap();
    for (bit, groups) in lut.init_zero.iter().enumerate() {
        // `~A` with A the lowest address bit: the table is one at every
        // even address and zero at every odd one, and a zero is a set bit.
        let odd = bit % 2 == 1;
        for at in groups {
            assert_eq!(
                bits.get(format.tile, *at),
                Some(odd),
                "bit {bit} of the truth table"
            );
        }
    }
    // Three of its four inputs are tied high, and the routed one is not.
    let tied = lut
        .tie_high
        .iter()
        .filter(|group| {
            group
                .iter()
                .all(|at| bits.get(format.tile, *at) == Some(true))
        })
        .count();
    assert_eq!(tied, 3, "B, C and D are tied and A is routed");

    // The file round trips, which is the only check that the writer and the
    // frame map agree about a bitstream this dense.
    let bytes = stream.to_bytes(true);
    let back = Ecp5Stream::parse(&bytes, &formats).unwrap();
    assert_eq!(back.cram, stream.cram);
}

/// **The one place a `.mux` source wants a bit *clear*, and why it does not
/// reach the interconnect.**
///
/// `locate_field` and the pip writer both drop the bits a feature wants
/// clear, because a bitstream is assembled from zero. That is only sound
/// while no two features written into one tile disagree about a bit, and a
/// `.mux` source with an inverted bit is exactly a feature that could
/// disagree: taking such an arc leaves a bit set that some other arc of the
/// same mux wanted clear.
///
/// So it is worth knowing where those are, and the answer is a clean one:
/// **every one of them is in the clock network's own tiles**, and this
/// asserts it from the database rather than believing it. The general
/// interconnect — the `CIB*` tiles, the `PLC2`s and the `PIC*`s a pad or a
/// lookup table routes through — has none, so a combinational design cannot
/// reach one. A clocked design could, and that is one of the things
/// `docs/fpga-trellis.md` lists as remaining.
#[test]
fn an_inverted_mux_bit_only_happens_in_the_clock_network() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let mut sources = 0usize;
    let mut inverted = 0usize;
    let mut culprits: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for tile in [
        "PLC2",
        "CIB",
        "CIB_LR",
        "CIB_LR_S",
        "PIOT0",
        "PIOT1",
        "PICT0",
        "PICT1",
        "PICR0",
        "PICR0_DQS2",
        "PICR1",
        "PICR1_DQS3",
        "PICR2",
        "TAP_DRIVE",
    ] {
        let Some(data) = db.tile_database(tile) else {
            continue;
        };
        for (_, _, bits) in data.arcs() {
            sources += 1;
            if bits.iter().any(|bit| bit.inverted) {
                inverted += 1;
                culprits.insert(tile.to_owned());
            }
        }
    }
    assert!(sources > 8_000, "{sources} sources looked at");
    assert_eq!(
        inverted, 0,
        "the general interconnect has {inverted} mux source(s) wanting a bit clear, in {culprits:?}"
    );

    // And the clock network does have them, so the check above is not
    // vacuous: `CMUX_*` is where the quadrant clock muxes live.
    let clock = db
        .tile_database("CMUX_UL_0")
        .expect("the database has the clock multiplexers");
    let clocked = clock
        .arcs()
        .filter(|(_, _, bits)| bits.iter().any(|bit| bit.inverted))
        .count();
    assert!(clocked > 100, "{clocked} inverted sources in CMUX_UL_0");
}

/// **The bitstream read back through the database, and it says what the
/// router said.**
///
/// This is the only check there is on an arc, and it is a real one. The
/// reference bitstreams route other designs, so there is nothing to compare
/// a route against; what can be done is to decode this flow's own output
/// through the same `.mux` records the router read and ask two questions.
///
/// **Does every set bit belong to something?** All of them, in this
/// bitstream: no bit is left over once every feature the database names has
/// claimed the bits it needs. A leftover bit would be a bit this crate set
/// for a reason the database does not know, which is how a wrong tile rule
/// looks from the inside.
///
/// **Do the arcs come back the same?** The bits of one tile are shared
/// between features — `CIB.JA0MUX` and the `.mux JA0` that routes into the
/// same wire are literally one mux, and the bits a feature wants *clear* are
/// dropped rather than written — so a second feature written into a tile can
/// change what a first one selects. Nothing structural would notice. This
/// notices: the set of connections the bits select must be exactly the set
/// the router chose, in the same tiles, under the same names.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_bitstream_decodes_back_to_the_arcs_the_router_chose() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, _, routing, _, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/button_led.v",
        "testdata/fpga/cynthion/button_led.rcf",
    );

    let decoded = db.decode(&stream.cram);
    assert_eq!(
        decoded.bits,
        stream.cram.count_ones(),
        "the decoder and the writer disagree about how many bits are set"
    );
    assert_eq!(
        decoded.unexplained,
        0,
        "{} of {} bit(s) belong to no feature the database names:\n{}",
        decoded.unexplained,
        decoded.bits,
        decoded.to_text()
    );

    // Every decoded arc, resolved from the `bits.db` spelling into the same
    // `(name, tile)` pairs the graph uses. `S1E1_JA0` at (62, 0) is the
    // `JA0` of (63, 1), and a wire of another die does not exist here.
    let prefix = db.device().chip_prefix();
    let resolve = |at: (u32, u32), name: &str| -> Option<(String, (u32, u32))> {
        match trellis::parse::globalise(name, prefix)? {
            trellis::parse::WireTarget::Global { name } => Some((name, (0, 0))),
            trellis::parse::WireTarget::Tile { dx, dy, name } => {
                let x = u32::try_from(i64::from(at.0) + i64::from(dx)).ok()?;
                let y = u32::try_from(i64::from(at.1) + i64::from(dy)).ok()?;
                Some((name, (x, y)))
            }
        }
    };
    let mut selected = std::collections::BTreeSet::new();
    for (at, sink, source) in &decoded.arcs {
        let (Some(to), Some(from)) = (resolve(*at, sink), resolve(*at, source)) else {
            panic!("`{sink} <- {source}` at {at:?} names a wire of another die");
        };
        selected.insert((to, from));
    }

    assert_eq!(
        selected.len(),
        decoded.arcs.len(),
        "two arcs of the bitstream resolve to the same pair of wires"
    );
    assert_eq!(
        selected,
        routed.wires,
        "the bits select connections the router did not choose, or fail to \
         select ones it did.\nthe router took {} arc(s) that cost bits, the \
         bitstream says {}:\n{}",
        routed.wires.len(),
        selected.len(),
        decoded.to_text()
    );
    // And the milestone's shape, so this test fails loudly if the design
    // changes rather than quietly comparing an empty set with an empty set.
    assert_eq!(routing.signals, 2);
    assert!(
        selected.len() >= 10,
        "{} arcs cost bits out of {} pips",
        selected.len(),
        routing.pips
    );

    // The rest of the decoding, for the record: the button as an input, the
    // pads as outputs, two bank rails, and one truth table.
    let fields: Vec<&str> = decoded
        .enums
        .iter()
        .map(|(_, field, _)| field.as_str())
        .collect();
    for wanted in [
        "PIOD.BASE_TYPE",
        "PIOD.HYSTERESIS",
        "PIOD.PULLMODE",
        "BANK.VCCIO",
        "SLICED.B0MUX",
    ] {
        assert!(fields.contains(&wanted), "{wanted} is not in {fields:?}");
    }
    assert_eq!(
        decoded
            .words
            .iter()
            .map(|(_, field, value)| (field.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        vec![("SLICED.K0.INIT", "1010101010101010")],
        "one truth table, `~A` read bit 0 first"
    );
    assert_eq!(
        decoded
            .enums
            .iter()
            .filter(|(_, field, _)| field == "BANK.VCCIO")
            .map(|(_, _, value)| value.as_str())
            .collect::<Vec<_>>(),
        vec!["3V3", "3V3"],
        "bank 1 for the LEDs and bank 3 for the button"
    );
}

/// The clock network's geometry, as `globals.json` states it, and what a
/// position resolves to through it.
///
/// This is the one file of the database whose contents cannot be checked
/// against anything else, because it says the thing `bits.db` leaves out:
/// which tile a clock's wires belong to. So the numbers are written down
/// here, and `what_lattices_own_packer_writes_for_a_clock` checks them
/// against a bitstream Lattice's own packer wrote, which is the only
/// independent evidence there is.
#[test]
fn the_clock_network_is_the_geometry_globals_json_states() {
    let Some(fabric) = open() else { return };
    let clocks = &fabric.clocks;

    // Sixteen networks, read from the database rather than assumed: the set
    // of `n` for which every quadrant offers a `G_<quadrant>PCLK<n>`.
    assert_eq!(clocks.indices, (0..16).collect::<Vec<u32>>());

    // Four quadrants, splitting the 73 x 51 grid at column 32 and row 26.
    assert_eq!(
        clocks.quadrants,
        vec![
            ("LL".to_owned(), (0, 26, 31, 50)),
            ("LR".to_owned(), (32, 26, 72, 50)),
            ("UL".to_owned(), (0, 0, 31, 25)),
            ("UR".to_owned(), (32, 0, 72, 25)),
        ]
    );

    // Four tap columns, each driving a run of columns to its left and a run
    // to its right. The two runs of one tap are adjacent and no tap crosses
    // a quadrant boundary, which is what lets a position resolve to exactly
    // one tap and one quadrant.
    assert_eq!(
        clocks.taps,
        vec![
            (4, (0, 3, 4, 12)),
            (22, (13, 21, 22, 31)),
            (42, (32, 41, 42, 50)),
            (60, (51, 59, 60, 72)),
        ]
    );

    // Eight spines, one per (quadrant, tap column), each one column west of
    // its tap and on the quadrant's middle row.
    assert_eq!(
        clocks.spines,
        vec![
            ("LL".to_owned(), 4, (3, 37)),
            ("LL".to_owned(), 22, (21, 37)),
            ("LR".to_owned(), 42, (41, 37)),
            ("LR".to_owned(), 60, (59, 37)),
            ("UL".to_owned(), 4, (3, 13)),
            ("UL".to_owned(), 22, (21, 13)),
            ("UR".to_owned(), 42, (41, 13)),
            ("UR".to_owned(), 60, (59, 13)),
        ]
    );

    // Every column of the die resolves to one tap and one side, and every
    // position to one quadrant. That is not a property the file states —
    // two runs could overlap, or a column could fall outside all four — so
    // it is checked rather than read.
    for x in 0..73 {
        assert!(clocks.tap_of(x).is_some(), "column {x} has no tap");
        for y in 0..51 {
            assert!(
                clocks.quadrant_of(x, y).is_some(),
                "(col {x}, row {y}) is in no quadrant"
            );
        }
    }
    // And the answers for the columns the clocked design's flip-flops are
    // in, which is where a wrong tap would put the branch bits.
    assert_eq!(clocks.tap_of(50), Some((42, 'R')));
    assert_eq!(clocks.tap_of(51), Some((60, 'L')));
    assert_eq!(clocks.tap_of(4), Some((4, 'R')));
    assert_eq!(clocks.tap_of(3), Some((4, 'L')));
    assert_eq!(clocks.quadrant_of(50, 5), Some("UR"));
    assert_eq!(clocks.quadrant_of(3, 40), Some("LL"));

    // The network index a branch wire names, which is what says which of
    // the sixteen a clock ended up on.
    assert_eq!(trellis::ClockNetwork::branch_index("G_HPBX0000"), Some(0));
    assert_eq!(trellis::ClockNetwork::branch_index("R_HPBX1500"), Some(15));
    assert_eq!(trellis::ClockNetwork::branch_index("V02N0701"), None);
    assert_eq!(trellis::ClockNetwork::branch_index("G_HPRX0000"), None);
}

/// **What Lattice's own packer writes for a clock, asked in full.**
///
/// This is the question that found `BANK.VCCIO` and `PULLMODE`, asked again
/// for the clock network: not "what differs between our bitstream and the
/// reference" but "what does `ecppack` write, in full, for a clock arriving
/// on a pad, reaching a global network and clocking a flip-flop, and do we
/// write all of it?" A diff only finds disagreements about things already
/// emitted; it is silent about things never emitted at all, and both
/// previous misses were of the second kind.
///
/// `analyzer.bit` is the one of Great Scott Gadgets' three bitstreams that
/// is clocked, and it uses **two** globals. Per global it writes:
///
/// | | |
/// |---|---|
/// | the buffer's input mux | `G_LDCC<n>CLKI <- …`, one arc in `LMID_0` |
/// | the centre mux | `G_<quadrant>PCLK<g> <- G_HPFE<n>00`, in **all four** quadrants |
/// | the spine | `G_VPTX<g>00 <- G_HPRX<g>00`, per spine it reaches |
/// | the tap | `L_`/`R_HPBX<g>00 <- G_VPTX<g>00`, per row with a sink |
/// | the tile | `CLK<c> <- G_HPBX<g>00`, per logic tile |
///
/// and — the part a diff could never have produced — **nothing for the
/// buffer itself.** There is no `DCC_*.MODE` anywhere in the file. nextpnr's
/// `write_dcc` writes `DCC_<x><n>.MODE = DCCA` only when the cell has a
/// clock enable; `NONE` is the field's default and costs no bits, so an
/// ungated buffer is a wire. That is why `trellis` declares one as a bel
/// with no configuration rather than hunting for a bit to set.
#[test]
fn what_lattices_own_packer_writes_for_a_clock() {
    let Some(root) = chipdb() else { return };
    let Some(bytes) = reference("analyzer") else {
        return;
    };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
    let decoded = db.decode(&stream.cram);
    let fabric = db.load(&TrellisOptions::new()).unwrap();

    // Two globals, and each one reaches all four quadrants' centre muxes.
    // nextpnr's `route_onto_global` loops over the quadrants deliberately,
    // whether the design needs them or not.
    let centres: Vec<((u32, u32), &str, &str)> = decoded
        .arcs
        .iter()
        .filter(|(_, to, _)| to.contains("PCLK") && !to.contains("CIB"))
        .map(|(at, to, from)| (*at, to.as_str(), from.as_str()))
        .collect();
    assert_eq!(
        centres,
        vec![
            ((31, 13), "G_ULPCLK0", "G_HPFE0600"),
            ((31, 13), "G_ULPCLK1", "G_HPFE0000"),
            ((31, 37), "G_LLPCLK0", "G_HPFE0600"),
            ((31, 37), "G_LLPCLK1", "G_HPFE0000"),
            ((32, 13), "G_URPCLK0", "G_HPFE0600"),
            ((32, 13), "G_URPCLK1", "G_HPFE0000"),
            ((32, 37), "G_LRPCLK0", "G_HPFE0600"),
            ((32, 37), "G_LRPCLK1", "G_HPFE0000"),
        ],
        "one centre mux per quadrant, at the four positions the grid puts them"
    );

    // The spine arcs land on exactly the positions `globals.json` names, and
    // nowhere else. This is what turns the spine table from something read
    // into something measured.
    let spines: std::collections::BTreeSet<(u32, u32)> = decoded
        .arcs
        .iter()
        .filter(|(_, to, from)| to.starts_with("G_VPTX") && from.starts_with("G_HPRX"))
        .map(|(at, _, _)| *at)
        .collect();
    let named: std::collections::BTreeSet<(u32, u32)> =
        fabric.clocks.spines.iter().map(|(_, _, at)| *at).collect();
    assert!(
        spines.is_subset(&named),
        "a spine arc at a position `globals.json` does not name: {:?}",
        spines.difference(&named).collect::<Vec<_>>()
    );
    assert_eq!(spines.len(), 8, "both globals reach all four quadrants");

    // Every branch driver is in a tap column. One in the wrong column would
    // mean the tap table is wrong and every branch bit of every design this
    // flow builds is in the wrong place.
    let mut taps = 0usize;
    for (at, to, from) in &decoded.arcs {
        if !from.starts_with("G_VPTX") || !to.contains("HPBX") {
            continue;
        }
        taps += 1;
        assert!(
            fabric.clocks.taps.iter().any(|(col, _)| *col == at.0),
            "a branch driver at column {}, which is not a tap column",
            at.0
        );
    }
    assert!(taps > 100, "{taps} branch drivers");

    // And the finding: the buffer costs nothing. Not one `DCC_*.MODE` in the
    // whole file, although two of its buffers are carrying a global.
    let modes: Vec<&str> = decoded
        .enums
        .iter()
        .filter(|(_, field, _)| field.starts_with("DCC_"))
        .map(|(_, field, _)| field.as_str())
        .collect();
    assert!(
        modes.is_empty(),
        "`ecppack` wrote a DCC mode after all: {modes:?}"
    );
    // The two buffers it does use, named by their input mux, so the claim
    // above is about a file that really does have a clock in it.
    let inputs: Vec<&str> = decoded
        .arcs
        .iter()
        .filter(|(_, to, _)| to.contains("DCC") && to.ends_with("CLKI"))
        .map(|(_, to, _)| to.as_str())
        .collect();
    assert_eq!(inputs, vec!["G_LDCC0CLKI", "G_LDCC6CLKI"]);
}

/// The clocked milestone: `clock_blink.v`, and every claim its header makes
/// about what reaches the part.
///
/// The header says a person should see two LEDs trading places at 0.89 Hz.
/// Nothing here can check that — that is what a person is for — so what is
/// checked is everything between the Verilog and the bits:
///
/// 1. the design places and routes **completely**, and every sink walks back
///    to its driver;
/// 2. every flip-flop's clock arrives on a **global network** and not through
///    interconnect that happens to reach a clock mux;
/// 3. the whole path is the one `ClockNetwork` describes — buffer, centre
///    mux, spine, tap, branch — at the positions `globals.json` gives;
/// 4. no bit an arc needs **clear** has been set by anything else;
/// 5. and every bit of the finished image decodes back, through the same
///    records the router read, into exactly the arcs the router chose.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_clocked_design_routes_and_configures_what_its_header_promises() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (bits, stream, pads, report, _, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/clock_blink.v",
        "testdata/fpga/cynthion/clock_blink.rcf",
    );

    // ---- 1. it fits and it routes ----
    assert_eq!(pads, 7, "one clock in and six LEDs out");
    assert_eq!(
        routed.ffs, 26,
        "a 26-bit counter, one flip-flop per bit, each configured by its own parameters"
    );
    assert_eq!(
        report.signals, 100,
        "every signal of the design, and `compile` has already walked each sink back to its driver"
    );

    // ---- 2. every clock on the network ----
    assert!(
        routed.clocks.off_network.is_empty(),
        "these flip-flops' clocks came through general interconnect: {:?}",
        routed.clocks.off_network
    );
    assert_eq!(
        routed.clocks.networks.values().sum::<usize>(),
        26,
        "26 clock pins accounted for"
    );
    assert_eq!(
        routed.clocks.networks.len(),
        1,
        "one clock, so one network: {:?}",
        routed.clocks.networks
    );

    // ---- 3. the path is the one the network describes ----
    //
    // Read off the bits rather than off the routing, so this is a statement
    // about the file and not about the router's bookkeeping.
    let decoded = db.decode(&stream.cram);
    let index = *routed.clocks.networks.keys().next().unwrap();
    let branch = format!("G_HPBX{index:02}00");
    let arcs: Vec<(&(u32, u32), &str, &str)> = decoded
        .arcs
        .iter()
        .map(|(at, to, from)| (at, to.as_str(), from.as_str()))
        .collect();
    let has = |sink: &str, source: &str| -> bool {
        arcs.iter()
            .any(|(_, to, from)| to.contains(sink) && from.contains(source))
    };
    // The buffer's input, fed from a `PCLKCIB` wire — which is how a clock
    // on a pad that is *not* a dedicated clock pad reaches the centre. A8 is
    // `PCLKC0_0`, the complement half of bank 0's pair, and only the `PCLKT`
    // half has the dedicated `JINCK` path.
    assert!(
        has("DCCCLKI", "PCLKCIB") || has("DCC", "PCLKCIB"),
        "no buffer input fed from a PCLKCIB wire:\n{}",
        decoded.to_text()
    );
    // The centre mux, the spine, the tap and the branch, each at a position
    // the network's own tables name.
    let centre: Vec<&(u32, u32)> = arcs
        .iter()
        .filter(|(_, to, _)| to.contains("PCLK") && !to.contains("CIB"))
        .map(|(at, _, _)| *at)
        .collect();
    assert_eq!(
        centre.len(),
        1,
        "one quadrant's centre mux, since every flip-flop is in one quadrant"
    );
    let spines: Vec<&(u32, u32)> = arcs
        .iter()
        .filter(|(_, to, from)| to.starts_with("G_VPTX") && from.starts_with("G_HPRX"))
        .map(|(at, _, _)| *at)
        .collect();
    assert!(!spines.is_empty(), "no spine arc");
    for at in &spines {
        assert!(
            fabric
                .clocks
                .spines
                .iter()
                .any(|(_, _, position)| position == *at),
            "a spine arc at {at:?}, which `globals.json` does not name"
        );
    }
    let mut branch_tiles = 0usize;
    for (at, to, from) in &arcs {
        if !from.starts_with("G_VPTX") || !to.contains("HPBX") {
            continue;
        }
        branch_tiles += 1;
        let side = to.chars().next().unwrap();
        // The tap column and the side both come out of the table, and the
        // arc has to agree with both.
        assert!(
            fabric.clocks.taps.iter().any(|(col, _)| *col == at.0),
            "a branch driver at column {}, which is not a tap column",
            at.0
        );
        assert!(side == 'L' || side == 'R', "{to}");
    }
    assert!(branch_tiles > 0, "no branch driver");
    // And the last hop into each logic tile that holds a flip-flop.
    let into_tiles: Vec<&(u32, u32)> = arcs
        .iter()
        .filter(|(_, to, from)| (*to == "CLK0" || *to == "CLK1") && *from == branch.as_str())
        .map(|(at, _, _)| *at)
        .collect();
    assert!(
        into_tiles.len() >= 4,
        "the counter is spread over more tiles than that: {into_tiles:?}"
    );

    // ---- 4. nothing has stolen a bit an arc needs clear ----
    assert!(
        routed.dropped.is_empty(),
        "a bit an arc of this design needs clear was set by something else: {:?}",
        routed.dropped
    );

    // ---- 5. and the bits say what the router chose ----
    assert_eq!(
        decoded.bits,
        stream.cram.count_ones(),
        "the decoder and the writer disagree about how many bits are set"
    );
    assert_eq!(
        decoded.unexplained, 0,
        "{} of {} bit(s) belong to no feature the database names",
        decoded.unexplained, decoded.bits
    );
    let (selected, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    assert_eq!(
        selected,
        routed.wires,
        "the bits select connections the router did not choose, or fail to select ones it did. \
         The router took {} arc(s) that cost bits and the bitstream says {}",
        routed.wires.len(),
        selected.len()
    );
    assert!(
        selected.len() > 600,
        "{} arcs cost bits, which is fewer than a routed counter takes",
        selected.len()
    );
    let _ = bits;
}

/// The flip-flop settings a `TRELLIS_FF` costs, and the one of them that
/// would leave a whole design frozen if it were missed.
///
/// `SLICE<l>.CEMUX` defaults to `CE` — take the clock enable from the
/// fabric — so a bitstream that leaves the field alone has every flip-flop
/// gated by a wire nothing drives. It is the same shape of omission as the
/// bank rail and the pull mode: a database default that is wrong for the
/// design, in a field the design never mentions, with no symptom any
/// structural check could see.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_flip_flops_settings_are_the_ones_lattices_own_packer_writes() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, _, _, _, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/clock_blink.v",
        "testdata/fpga/cynthion/clock_blink.rcf",
    );
    assert_eq!(routed.ffs, 26);
    let decoded = db.decode(&stream.cram);
    let fields: std::collections::BTreeSet<(&str, &str)> = decoded
        .enums
        .iter()
        .map(|(_, field, value)| (field.as_str(), value.as_str()))
        .collect();
    // The four that cost bits, for at least one slice. `LSRMODE`, `GSR`'s
    // `ENABLED`, `CLK<n>.CLKMUX = CLK`, `LSR<n>.LSRMUX = LSR` and
    // `LSR<n>.SRMODE = LSR_OVER_CE` are each a field's own default and cost
    // nothing, which is why they are not here: the list is what a bitstream
    // *pays* for, and a default that cost a bit would show up as a
    // regression here.
    for wanted in [
        ("SLICEA.CEMUX", "1"),
        ("SLICEA.REG0.SD", "0"),
        ("SLICEA.REG0.REGSET", "RESET"),
        ("SLICEA.GSR", "DISABLED"),
    ] {
        assert!(
            fields.contains(&wanted),
            "{wanted:?} is not in the decoding"
        );
    }
    // And nothing asked for an inverted clock or an asynchronous reset,
    // which are the two settings `configure_registers` refuses rather than
    // writing into a mux the routing does not identify.
    assert!(
        !fields
            .iter()
            .any(|(field, value)| field.ends_with("CLKMUX") && *value == "INV"),
        "{fields:?}"
    );
}

/// The check that makes a dropped clear bit sound, shown failing.
///
/// A `.mux` source that wants a bit clear leaves nothing to write, so the
/// loader does not record it on the pip — and the only thing "honouring" it
/// can mean is noticing when another feature of the same tile has set it.
/// This sets one such bit by hand and asserts that the check says so, which
/// is the only way to know the check would fire: the designs this flow
/// builds do not collide, so a green run of them proves nothing about
/// whether anything is looking.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_bit_an_arc_needs_clear_is_noticed_when_something_else_sets_it() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (mut bits, _, _, _, _, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/clock_blink.v",
        "testdata/fpga/cynthion/clock_blink.rcf",
    );
    assert!(
        routed.dropped.is_empty(),
        "the design as built already collides: {:?}",
        routed.dropped
    );

    // The arcs of this design that have a bit they want clear at all. On a
    // clocked design there are some — a centre mux encodes its source as a
    // six-bit code, five bits of which it wants clear — and on a
    // combinational one there are none, which is exactly what
    // `docs/fpga-trellis.md` says and why this could not be tested before.
    let mut coded: Vec<((u32, u32), ConfigBit)> = Vec::new();
    for route in routed.routing.routes() {
        for id in &route.pips {
            let pip = routed.graph.pip(*id);
            let mut key = routed.graph.pip_bits(*id).to_vec();
            key.sort_unstable();
            let Some(ty) = fabric.arch.tile_index_at(pip.tile.0, pip.tile.1) else {
                continue;
            };
            if let Some(clear) = fabric.clears.get(&(ty, key))
                && let Some(bit) = clear.first()
            {
                coded.push((pip.tile, *bit));
            }
        }
    }
    assert!(
        !coded.is_empty(),
        "this design takes no arc with a bit it wants clear, so there would be nothing to check"
    );

    // Set one of them, which is what a second feature written into the same
    // tile would do, and the arc silently becomes a different one.
    let (at, bit) = coded[0];
    bits.set(at, bit).unwrap();
    let problems = fabric.dropped_clear_bits(&routed.graph, &routed.routing, &bits);
    assert_eq!(
        problems.len(),
        1,
        "one stolen bit should be one complaint: {problems:?}"
    );
    assert!(
        problems[0].contains(&format!("X{}Y{}", at.0, at.1))
            && problems[0].contains(&format!("{}.{}", bit.row, bit.col)),
        "{}",
        problems[0]
    );
}

/// The eight data balls of the Cynthion's **auxiliary** ULPI transceiver,
/// in `ulpi_data[0]` to `[7]` order.
///
/// From Great Scott Gadgets' own platform file, `cynthion_r1_4.py`:
///
/// ```python
/// ULPIResource("aux_phy", 0,
///     data="F16 G15 G16 H15 J15 J16 K15 K16", clk="D16", clk_dir='o',
///     dir="E16", nxt="F15", stp="E15", rst="J13", rst_invert=True,
///     attrs=Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")),
/// ```
///
/// Amaranth's `ULPIResource` makes `data` a `Subsignal(..., dir="io")`, so
/// these eight are the board's bidirectional pins: the transceiver drives
/// them for received data and the FPGA for transmitted data, arbitrated by
/// `dir`. All eight are on the **right** edge of the die, which is an edge
/// this backend describes.
#[cfg(feature = "verilog")]
const AUX_ULPI_DATA: [&str; 8] = ["F16", "G15", "G16", "H15", "J15", "J16", "K15", "K16"];

/// The "what does `ecppack` write, **in full**, for a bidirectional pad?"
/// question, asked of a file that has eight of them.
///
/// This is the third time that question has been asked this way round and it
/// is the reason to keep asking: a diff against a reference only disagrees
/// about settings already emitted and is silent about settings never emitted
/// at all. `BANK.VCCIO` and an input's `PULLMODE` were both of the second
/// kind and `DONE` was high without either.
///
/// `analyzer.bit` is Great Scott Gadgets' own build for this very board and
/// it instantiates the auxiliary ULPI transceiver, whose eight-bit data bus
/// turns around. So it is a direct oracle, and what it has for each of those
/// eight balls is:
///
/// | Setting | Tile | Bits beyond the base type | Written here |
/// |---|---|---|---|
/// | `PIO<s>.BASE_TYPE = BIDIR_LVCMOS33` | the pad tile | — | yes |
/// | `PIO<s>.BASE_TYPE = BIDIR_LVCMOS33` | the second-copy tile | — | yes |
/// | `PIO<s>.PULLMODE = NONE` | the pad tile | **one, `F7B0`** | yes |
/// | `PIO<s>.HYSTERESIS = ON` | the pad tile | none: the base type's own bits already contain it | yes |
/// | `PIO<s>.SLEWRATE = FAST` | the pad tile | one | only when a constraint asks |
/// | `BANK.VCCIO = 3V3` | `BANKREF<n>` | — | yes |
/// | anything governing the **tristate** | — | **nothing at all** | nothing to write |
///
/// The last row is the finding, and it is the one a diff could not have
/// produced. The field that governs where a pad's tristate comes from is
/// `PIO<s>.TRIMUX_TSREG`, in the second-copy tile, and its values are
/// `PADDT` — the wire the fabric drives — and `IOLTO`, the `IOLOGIC`
/// tristate register. `PADDT` is the **default**, so it costs no bits, and
/// it is what a fabric-driven tristate means. nextpnr writes the field only
/// when its packer moved a tristate flip-flop into `IOLOGIC`
/// (`pack.cc`'s `pio->params[id_TRIMUX_TSREG] = "IOLTO"`), and this asserts
/// that **no `TRIMUX_TSREG` appears anywhere in the whole file** although
/// eight of its pads are bidirectional. So a bidirectional pad differs from
/// an output by its base type and nothing else, and the tristate is a
/// routed wire rather than a setting.
///
/// The other half is what is *not* there: nextpnr ties the tristate wire in
/// the `CIB` only when `T` is unconnected (`dir != "INPUT" && T == nullptr`
/// in `write_io`), which is exactly the complement of a real bidirectional
/// pad. That half cannot be read off a finished bitstream — a `CIB`'s
/// constant mux and its routing mux are one mux — so it is asserted against
/// this crate's own pass instead, in
/// `the_bidirectional_design_routes_and_configures_what_its_header_promises`.
#[test]
#[cfg(feature = "verilog")]
fn what_lattices_own_packer_writes_for_a_bidirectional_pad() {
    let Some(root) = chipdb() else { return };
    let Some(bytes) = reference("analyzer") else {
        return;
    };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
    let decoded = db.decode(&stream.cram);

    // The finding first, because it is about the whole file and not about
    // one ball: nothing anywhere governs a tristate.
    let tri: Vec<String> = decoded
        .enums
        .iter()
        .filter(|(_, field, _)| field.contains("TRIMUX"))
        .map(|(at, field, value)| format!("{field}={value} at {at:?}"))
        .collect();
    assert!(
        tri.is_empty(),
        "`ecppack` wrote a tristate mux after all, so `PADDT` is not simply the default: {tri:?}"
    );
    // The `DATAMUX_*` fields are the same shape and the same story, with one
    // difference that makes the story checkable: this file *does* write
    // `DATAMUX_ODDR = IOLDO`, on the thirteen HyperRAM pins, which are
    // registered and are all on the left edge at column 0. So the field is
    // one the decoding reports when it is set, and its absence at a ULPI pin
    // means what it looks like rather than meaning the decoder is blind to
    // it. Asserted both ways round.
    let data_muxes: Vec<(u32, u32)> = decoded
        .enums
        .iter()
        .filter(|(_, field, _)| field.contains("DATAMUX"))
        .map(|(at, _, _)| *at)
        .collect();
    assert!(
        !data_muxes.is_empty(),
        "no `DATAMUX_*` anywhere, so its absence at a ULPI pin says nothing"
    );
    assert!(
        data_muxes.iter().all(|(col, _)| *col == 0),
        "a `DATAMUX_*` off the left edge, where this board's registered pins are: {data_muxes:?}"
    );

    // And the eight balls, bit by bit at absolute frame positions, which is
    // the same standard the LEDs and the button were held to.
    let mut checked = 0usize;
    for ball in AUX_ULPI_DATA {
        let pad = fabric
            .pad(ball)
            .unwrap_or_else(|| panic!("{ball} is not in the ball map"));
        assert_eq!(pad.edge, trellis::Edge::Right, "{ball}");
        assert_eq!(pad.bidir_pad_bits.len(), 8, "{ball}: BIDIR_LVCMOS33");
        assert_eq!(pad.bidir_pic_bits.len(), 2, "{ball}: the second copy");
        assert_eq!(pad.pull_bits(trellis::PULL_NONE).len(), 1, "{ball}");
        for (what, at, bits) in [
            ("the base type", pad.pad_at, &pad.bidir_pad_bits[..]),
            ("the second copy of it", pad.pic_at, &pad.bidir_pic_bits[..]),
            ("hysteresis", pad.pad_at, &pad.hysteresis_bits[..]),
            (
                "the pull mode",
                pad.pad_at,
                pad.pull_bits(trellis::PULL_NONE),
            ),
        ] {
            for bit in bits {
                let (frame, index) = fabric
                    .frames
                    .locate(at, *bit)
                    .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
                assert!(
                    stream.cram.get(frame, index),
                    "F{frame}B{index}, which this crate sets for {what} of a bidirectional pad on \
                     {ball}, is clear in analyzer.bit, whose own gateware has that very ball on a \
                     ULPI data bus"
                );
                checked += 1;
            }
        }
        // A bidirectional pad is not an input with an output bolted on: the
        // input's pattern has a bit the bidirectional one does not want, so
        // writing both would not have produced this.
        assert!(
            pad.input_pad_bits
                .iter()
                .all(|bit| pad.bidir_pad_bits.contains(bit)),
            "{ball}: this assertion only documents the relation; correct it if it changes"
        );
        assert!(
            pad.output_pad_bits
                .iter()
                .any(|bit| !pad.bidir_pad_bits.contains(bit)),
            "{ball}: an output has a bit a bidirectional pad wants clear"
        );
        // Hysteresis costs nothing on top of the base type, which is why
        // writing it is free and why its absence would not have shown up
        // here. Said out loud so the row of the table above is honest.
        assert!(
            pad.hysteresis_bits
                .iter()
                .all(|bit| pad.bidir_pad_bits.contains(bit)),
            "{ball}: hysteresis is no longer implied by the base type"
        );
        // The pull is the opposite: a bit of its own, outside the base
        // type's, and the field's default is a pull-*down*.
        assert!(
            pad.pull_bits(trellis::PULL_NONE)
                .iter()
                .all(|bit| !pad.bidir_pad_bits.contains(bit)),
            "{ball}: the pull mode is no longer a setting of its own"
        );
        assert!(pad.pull_bits("DOWN").is_empty(), "{ball}: the default");
    }
    assert_eq!(checked, 8 * (8 + 2 + 1 + 1), "every bit of every ball");

    // AND BOTH HALVES OF EVERY PAIR READ BACK BIDIRECTIONAL, which is the
    // check that proves the resolution rule and is measured on the vendor's
    // own file so that it is a property of the format and not of this crate.
    //
    // F16 and G15 are sides A and B of one right-edge position, so they
    // share the pad tile at (col 72, row 15), and both are ULPI data pins,
    // so `ecppack` made both bidirectional. The eight balls come in four
    // such pairs. On the right edge a *pseudo-differential* value of
    // `PIO<s>.BASE_TYPE` reaches across the pair — `PIOA.BASE_TYPE =
    // OUTPUT_LVCMOS33D` is ten bits, four of which are PIOB's — and with
    // both halves bidirectional all ten of them happen to be set, so the
    // longest match alone read side A back as a differential output it is
    // not and left the two bits only a bidirectional or an input pad wants
    // belonging to nothing. `TrellisDatabase::decode` now resolves a field
    // by the reading that leaves fewest of the tile's bits unexplained,
    // with the longest match as the tie-break; "What could not be read back"
    // in `docs/fpga-trellis.md` is the account, and this is the assertion
    // it says would prove it right.
    let mut pairs: Vec<((u32, u32), char)> = Vec::new();
    for ball in AUX_ULPI_DATA {
        let pad = fabric.pad(ball).unwrap();
        pairs.push((pad.pad_at, pad.side));
    }
    pairs.sort_unstable();
    pairs.dedup();
    assert_eq!(pairs.len(), 8, "eight balls, eight (tile, side) pairs");
    for (at, side) in &pairs {
        let field = format!("PIO{side}.BASE_TYPE");
        let value = decoded
            .enums
            .iter()
            .find(|(where_, what, _)| where_ == at && what == &field)
            .map(|(_, _, value)| value.as_str());
        assert_eq!(
            value,
            Some("BIDIR_LVCMOS33"),
            "{field} at {at:?} — a ULPI data ball of `analyzer.bit`'s own aux transceiver"
        );
    }
    // And not one bit of any of those four tiles is left over, which is the
    // other half: a reading can always be made to look right by leaving the
    // bits that disagree with it unaccounted for.
    let orphans: Vec<&(String, (u32, u32), String)> = decoded
        .leftovers
        .iter()
        .filter(|(_, at, _)| pairs.iter().any(|(pad_at, _)| pad_at == at))
        .collect();
    assert!(
        orphans.is_empty(),
        "bits of a ULPI pad tile belong to no feature: {orphans:?}"
    );
}

/// Every pin of the Cynthion's **TARGET** USB transceiver, in the platform
/// file's own order, with the direction Amaranth's `ULPIResource` gives it.
///
/// From Great Scott Gadgets' own `cynthion_r1_4.py`:
///
/// ```python
/// ULPIResource("target_phy", 0,
///     data="R2 R1 P2 P1 N3 N1 M2 M1", clk="T4", clk_dir='o',
///     dir="R3", nxt="T2", stp="T3", rst="R4", rst_invert=True,
///     attrs=Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")),
/// ```
///
/// `data` is `dir="io"`, `clk_dir='o'` makes the FPGA drive the 60 MHz
/// clock, `stp` and `rst` are outputs, and `dir` and `nxt` are what the
/// transceiver drives. **All thirteen are on the left edge, at column 0**,
/// in bank 6 — which is why this port could not be built at all until that
/// edge was described, and why these thirteen are the oracle for it.
#[cfg(feature = "verilog")]
const TARGET_ULPI: [(&str, &str); 13] = [
    ("R2", "BIDIR"),
    ("R1", "BIDIR"),
    ("P2", "BIDIR"),
    ("P1", "BIDIR"),
    ("N3", "BIDIR"),
    ("N1", "BIDIR"),
    ("M2", "BIDIR"),
    ("M1", "BIDIR"),
    ("T4", "OUTPUT"),
    ("R3", "INPUT"),
    ("T2", "INPUT"),
    ("T3", "OUTPUT"),
    ("R4", "OUTPUT"),
];

/// The three VBUS switches, which are also on the left edge and which no
/// design could drive before it was described.
///
/// ```python
/// Resource("target_c_vbus_en", 0, Pins("K5", dir="o"), Attrs(IO_TYPE="LVCMOS33")),
/// Resource("control_vbus_en",  0, Pins("L1", dir="o"), Attrs(IO_TYPE="LVCMOS33")),
/// Resource("aux_vbus_en",      0, Pins("L2", dir="o"), Attrs(IO_TYPE="LVCMOS33")),
/// ```
///
/// Plain `Pins`, so active **high**: a zero keeps the switch off. And **no
/// `SLEWRATE`**, unlike the ULPI resource, which makes them the negative
/// control in the test below — `ecppack` leaves the slew field alone on
/// exactly the pads whose attributes do not ask for it.
///
/// Only `analyzer.bit` configures them; `selftest.bit` and `facedancer.bit`
/// leave all three alone, which the test asserts rather than skipping over.
#[cfg(feature = "verilog")]
const VBUS_SWITCHES: [&str; 3] = ["K5", "L1", "L2"];

/// The "what does `ecppack` write, **in full**, for a pad on the **left**
/// edge?" question, asked of the thirteen balls of the TARGET transceiver
/// and the three VBUS switches.
///
/// This is the measurement the left edge's tile rule rests on, and it is
/// asked this way round for the reason the other three
/// `what_lattices_own_packer_writes_for_a_...` tests are: the left edge
/// *looks* like the right edge mirrored, and a mirror is exactly the kind of
/// guess that decodes perfectly against itself and drives the wrong ball.
/// What the database says, and what all three of Great Scott Gadgets'
/// bitstreams confirm at absolute frame positions:
///
/// | | |
/// |---|---|
/// | The pad tile | one row **south** of the ball, a `PICL1*` or (at rows 13, 25, 37 and 49 of this die) a `MIB_CIB_LR` |
/// | The second `BASE_TYPE` | the ball's own row (`PICL0*`) for sides A and B, two rows south (`PICL2*`) for C and D |
/// | The `CIB` that ties data and enable | one column **east**, at column 1, from the buffer's own `JPADDO<s> <- E1_JA0` — where the right edge's says `W1_JA0` |
/// | Sides | **four**, A B C and D, and all four are used by these sixteen balls |
/// | What it costs | **exactly what the right edge costs**: 8 bits for `BIDIR_LVCMOS33`, 6 for `OUTPUT_LVCMOS33`, 5 for `INPUT_LVCMOS33`, plus 2 in the second copy for a bidirectional pad or an output and **none at all** for an input |
/// | Hysteresis, pull mode, slew rate | one bit each, in the pad tile, exactly as on the right edge |
///
/// **The rows do not mirror, and that is the finding.** Had the left edge
/// been written as the right edge reflected — pad tile one row *north* —
/// every one of these sixteen balls would have been configured in the tile
/// of a different ball, and the only thing that says otherwise is this
/// comparison.
///
/// Three negative controls are asserted as well, because a test that only
/// looks for bits that are set cannot tell a rule from a coincidence:
///
/// * **no hysteresis on an output.** `ecppack` writes `HYSTERESIS = ON` for
///   the inputs and the bidirectional pads and leaves it clear on `T4`,
///   `T3`, `R4` and the three switches, which is what `configure_io` does;
/// * **no slew rate on the switches.** All thirteen ULPI pins ask for
///   `SLEWRATE="FAST"` and have the bit; the three VBUS pins ask for nothing
///   and have it clear. So the field is written on request and not by
///   direction;
/// * **no `CIB` tie on any of them.** Every one of these pads has its data
///   wire *routed*, and the tie and the route are one mux, so a tie would
///   have been a second driver.
///
/// One known reading ambiguity shows up here and is pinned rather than
/// hidden: `R4` is side C of a C/D pair whose other half (`T3`) is also an
/// output, so the fewest-leftovers reading of its `BASE_TYPE` is the
/// *pseudo-differential* spelling `OUTPUT_LVCMOS33D`, whose pattern spans
/// the pair. All six bits of the plain `OUTPUT_LVCMOS33` are set inside it,
/// which is what matters for what this backend writes; see "What could not
/// be read back" in `docs/fpga-trellis.md`.
#[test]
#[cfg(feature = "verilog")]
fn what_lattices_own_packer_writes_for_a_left_edge_pad() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let mut checked = 0usize;
    for name in ["analyzer", "selftest", "facedancer"] {
        let Some(bytes) = reference(name) else {
            return;
        };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        // `analyzer.bit` is the only one of the three whose design drives the
        // VBUS switches, so the other two are asked the opposite question
        // about them.
        let switches = name == "analyzer";

        let set = |at: (u32, u32), bit: &ConfigBit| {
            let (frame, index) = fabric
                .frames
                .locate(at, *bit)
                .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
            (stream.cram.get(frame, index), frame, index)
        };

        for (ball, dir) in TARGET_ULPI
            .iter()
            .copied()
            .chain(VBUS_SWITCHES.iter().map(|b| (*b, "OUTPUT")))
        {
            let switch = VBUS_SWITCHES.contains(&ball);
            let pad = fabric
                .pad(ball)
                .unwrap_or_else(|| panic!("{ball} is not in the ball map"));
            // Where it is, which is the whole question.
            assert_eq!(pad.edge, trellis::Edge::Left, "{ball}");
            assert_eq!(pad.bel.0, 0, "{ball}: the buffer is at column 0");
            assert_eq!(pad.pad_at, (0, pad.bel.1 + 1), "{ball}: one row south");
            let south = if matches!(pad.side, 'A' | 'B') { 0 } else { 2 };
            assert_eq!(pad.pic_at, (0, pad.bel.1 + south), "{ball}");
            assert_eq!(pad.cib_at.0, 1, "{ball}: the CIB is one column east");
            assert_eq!(pad.bank, 6, "{ball}: bank 6, from `pio_metadata`");

            // What it costs, which is bit for bit what the right edge costs.
            assert_eq!(pad.bidir_pad_bits.len(), 8, "{ball}: BIDIR_LVCMOS33");
            assert_eq!(pad.bidir_pic_bits.len(), 2, "{ball}");
            assert_eq!(pad.output_pad_bits.len(), 6, "{ball}: OUTPUT_LVCMOS33");
            assert_eq!(pad.output_pic_bits.len(), 2, "{ball}");
            assert_eq!(pad.input_pad_bits.len(), 5, "{ball}: INPUT_LVCMOS33");
            assert!(
                pad.input_pic_bits.is_empty(),
                "{ball}: an input's second copy is empty on this edge too"
            );
            assert_eq!(pad.hysteresis_bits.len(), 1, "{ball}");
            assert_eq!(pad.pull_bits(trellis::PULL_NONE).len(), 1, "{ball}");
            assert!(pad.pull_bits("DOWN").is_empty(), "{ball}: the default");
            assert_eq!(pad.slew_bits("FAST").len(), 1, "{ball}");
            assert!(pad.slew_bits("SLOW").is_empty(), "{ball}: the default");

            // What the file reads back for this pad, by name, through the
            // same database. Looked up before the branch because the two
            // files that do *not* drive the switches have to be asked about
            // it too.
            let field = format!("PIO{}.BASE_TYPE", pad.side);
            let value = decoded
                .enums
                .iter()
                .find(|(at, what, _)| *at == pad.pad_at && *what == field)
                .map(|(_, _, v)| v.as_str());

            if switch && !switches {
                // `selftest.bit` and `facedancer.bit` do not drive the
                // switches, and what that looks like is worth being exact
                // about rather than asserting the obvious thing and finding
                // it false. **The pull mode is clear** — a field of its own,
                // one bit, that nothing else wants — and **the base type
                // does not read back at all**. What is *not* true is that
                // every bit of the base type's pattern is clear: `K5` is
                // side B of a pair whose side A (`K4`,
                // `target_a_discharge`) both files do drive, and a
                // pseudo-differential value of side A's `BASE_TYPE` reaches
                // across the pair, so one of side B's six bits is set by
                // side A's setting. Hence "the pattern is not complete"
                // rather than "no bit of it is set".
                for bit in pad.pull_bits(trellis::PULL_NONE) {
                    let (on, frame, index) = set(pad.pad_at, bit);
                    assert!(
                        !on,
                        "F{frame}B{index} is `PULLMODE = NONE` for {ball} in {name}.bit, whose \
                         design does not drive that switch"
                    );
                }
                assert!(
                    !pad.output_pad_bits.iter().all(|bit| set(pad.pad_at, bit).0),
                    "{ball} has every bit of an output's base type set in {name}.bit, whose \
                     design does not drive that switch"
                );
                assert_eq!(
                    value, None,
                    "{name}.bit reads back a base type for {ball} and does not drive it"
                );
                continue;
            }

            // And every bit of it, at an absolute frame position, in their
            // file.
            let (pad_bits, pic_bits) = match dir {
                "BIDIR" => (&pad.bidir_pad_bits, &pad.bidir_pic_bits),
                "INPUT" => (&pad.input_pad_bits, &pad.input_pic_bits),
                _ => (&pad.output_pad_bits, &pad.output_pic_bits),
            };
            let mut wanted: Vec<(&str, (u32, u32), &[ConfigBit])> = vec![
                ("the base type", pad.pad_at, &pad_bits[..]),
                ("the second copy of it", pad.pic_at, &pic_bits[..]),
                (
                    "the pull mode",
                    pad.pad_at,
                    pad.pull_bits(trellis::PULL_NONE),
                ),
            ];
            if dir != "OUTPUT" {
                wanted.push(("hysteresis", pad.pad_at, &pad.hysteresis_bits[..]));
            }
            if !switch {
                wanted.push(("the slew rate", pad.pad_at, pad.slew_bits("FAST")));
            }
            for (what, at, bits) in wanted {
                for bit in bits {
                    let (on, frame, index) = set(at, bit);
                    assert!(
                        on,
                        "F{frame}B{index}, which this crate sets for {what} of a {dir} pad on \
                         {ball}, is clear in {name}.bit — whose own gateware has that ball on the \
                         TARGET port"
                    );
                    checked += 1;
                }
            }

            // THE NEGATIVE CONTROLS.
            if dir == "OUTPUT" {
                for bit in &pad.hysteresis_bits {
                    let (on, frame, index) = set(pad.pad_at, bit);
                    assert!(
                        !on,
                        "F{frame}B{index} is `HYSTERESIS = ON` for {ball}, an output, and \
                         {name}.bit has it set — so hysteresis is not input-only after all"
                    );
                }
            }
            if switch {
                for bit in pad.slew_bits("FAST") {
                    let (on, frame, index) = set(pad.pad_at, bit);
                    assert!(
                        !on,
                        "F{frame}B{index} is `SLEWRATE = FAST` for {ball}, whose resource asks \
                         for no slew rate, and {name}.bit has it set"
                    );
                }
            }
            // Nothing ties either of this pad's `CIB` wires: all sixteen have
            // a signal routed into the data wire, and the tie and the route
            // are one mux.
            let wire = if matches!(pad.side, 'A' | 'C') { 0 } else { 3 };
            for field in [format!("CIB.JA{wire}MUX"), format!("CIB.JB{wire}MUX")] {
                let tie = decoded
                    .enums
                    .iter()
                    .find(|(at, what, _)| *at == pad.cib_at && *what == field)
                    .map(|(_, _, v)| v.as_str());
                assert!(
                    tie.is_none(),
                    "{name}.bit ties {field} at {:?} to {tie:?} for {ball}, which is a second \
                     driver on a wire its own router drives",
                    pad.cib_at
                );
            }

            // And the reading taken above, so this is not only an assertion
            // about bit positions.
            let expected: &[&str] = match dir {
                "BIDIR" => &["BIDIR_LVCMOS33"],
                "INPUT" => &["INPUT_LVCMOS33"],
                // A pseudo-differential spelling reaches across the pair, so
                // an output whose partner is also an output reads back as the
                // `D` form. `R4` is the one here, and its six plain bits are
                // asserted set above either way.
                _ => &["OUTPUT_LVCMOS33", "OUTPUT_LVCMOS33D"],
            };
            assert!(
                value.is_some_and(|v| expected.contains(&v)),
                "{name}.bit: {field} at {:?} ({ball}) reads {value:?}, not one of {expected:?}",
                pad.pad_at
            );
            if ball == "R4" {
                assert_eq!(
                    value,
                    Some("OUTPUT_LVCMOS33D"),
                    "R4 is side C of a pair whose side D is also an output, so the \
                     fewest-leftovers reading is the pseudo-differential one. Correct this \
                     assertion if the resolution rule changes; it documents a reading, not a \
                     requirement."
                );
            }
        }

        // AND NOTHING ON THE WHOLE EDGE BELONGS TO NO PAD, which is the
        // honest direction and the one that would catch a tile rule that is
        // right for sixteen balls and wrong for the rest. Every
        // `PIO<s>.BASE_TYPE` these files set anywhere in column 0 is either a
        // pad tile or a second-copy tile of a ball this backend maps.
        let mut pads = 0usize;
        let mut copies = 0usize;
        let mut orphans: Vec<String> = Vec::new();
        for (at, field, value) in &decoded.enums {
            let Some(side) = field
                .strip_prefix("PIO")
                .and_then(|rest| rest.strip_suffix(".BASE_TYPE"))
                .and_then(|s| s.chars().next())
            else {
                continue;
            };
            if at.0 != 0 {
                continue;
            }
            let on = |pick: fn(&trellis::IoSite) -> (u32, u32)| {
                fabric
                    .io
                    .iter()
                    .any(|p| p.edge == trellis::Edge::Left && pick(p) == *at && p.side == side)
            };
            if on(|p| p.pad_at) {
                pads += 1;
            } else if on(|p| p.pic_at) {
                copies += 1;
            } else {
                orphans.push(format!("{field} = {value} at {at:?}"));
            }
        }
        assert!(
            orphans.is_empty(),
            "{name}.bit configures a left-edge PIO that belongs to no ball of the map: {orphans:?}"
        );
        // How many left-edge pads each file configures, which is the size of
        // the oracle and is asserted exactly so that a file that stopped
        // being one would be noticed.
        let expected = match name {
            "analyzer" => (58, 57),
            "selftest" => (52, 53),
            _ => (40, 40),
        };
        assert_eq!((pads, copies), expected, "{name}.bit: left-edge pads");
    }
    // 13 ULPI pins in three files and 3 switches in one. A bidirectional
    // ball is 8 bits of base type, 2 of second copy, and one each of pull
    // mode, hysteresis and slew rate; an input is 5 and **no** second copy
    // with the same three; an output is 6 and 2 with no hysteresis; a switch
    // is an output with no slew rate either.
    let per_file = 8 * (8 + 2 + 1 + 1 + 1) + 2 * (5 + 1 + 1 + 1) + 3 * (6 + 2 + 1 + 1);
    assert_eq!(
        checked,
        3 * per_file + 3 * (6 + 2 + 1),
        "every bit of every ball"
    );
}

/// The same question for the **bottom** edge, which on a caBGA-256 is
/// thirteen balls of bank 8 and nothing else.
///
/// Every one of those thirteen is a configuration pin — `D0`..`D7`, `CSN`,
/// `CS1N`, `HOLDN`, `DOUT` and `WRITEN` — and Great Scott Gadgets' own
/// designs use four of them: `int` on `T6` and the SPI flash on `T8`, `T7`
/// and `N8`, with `facedancer.bit` taking `T8` and `T7` as a quad-mode
/// bidirectional pair. So the edge has an oracle, and it says something the
/// other three edges do not:
///
/// | | |
/// |---|---|
/// | The pad tile | the ball's own position for side A, **one column east** for side B — the top edge's column rule |
/// | The second `BASE_TYPE` | **there is no second tile.** `PICB0` and `PICB1` hold the pad's own fields *and* the `DATAMUX_*` and `TRIMUX_TSREG` the top edge keeps in a `PICT<n>` a row away, and no position of row 50 or row 49 declares another `PIO<side>.BASE_TYPE`. So `pic_at == pad_at` |
/// | The `CIB` | one row **north**, at row 49, from `JPADDO<s> <- N1_JA0` (side A) and `N1E1_JA0` (side B) |
/// | Sides | **two**, A and B, which is what `PICB0` and `PICB1` declare and what the ball map uses |
/// | What it costs | **not what the other edges cost**: 10 bits for `BIDIR_LVCMOS33` and 7 for `OUTPUT_LVCMOS33` where the top and the two long edges spend 8 and 6. An input is 5, the same as everywhere |
///
/// The extra bits are this edge's own and not a second copy counted twice:
/// the pattern is one tile's. Hysteresis, the pull mode and the slew rate are
/// one bit each here as well, so the only things that differ are the base
/// type's width and the absence of a second tile to repeat it in.
///
/// **Nothing is loaded onto a board from this edge and nothing should be**:
/// `docs/fpga-trellis.md` says why — a design driving `D0`..`D7` is driving
/// the pins the part configures itself through.
#[test]
#[cfg(feature = "verilog")]
fn what_lattices_own_packer_writes_for_a_bottom_edge_pad() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    // Which ball each file configures, and as what. `T6` is `int` and `R6` a
    // pseudo-supply pin; `T8` and `T7` are the SPI flash's `sdi` and `sdo` in
    // `analyzer.bit` and four of them — `T8 T7 M7 N7` — are the quad-mode
    // `dq` bus, bidirectional, in `facedancer.bit`; `N8` is the flash's chip
    // select. **`selftest.bit` configures exactly one**, `R6`, and that is
    // asserted rather than skipped: a file that is not an oracle must be
    // known not to be one.
    const BOTTOM: [(&str, &[(&str, &str)]); 3] = [
        (
            "analyzer",
            &[
                ("T6", "OUTPUT_LVCMOS33"),
                ("R6", "OUTPUT_LVCMOS33"),
                ("T8", "OUTPUT_LVCMOS33"),
                ("T7", "INPUT_LVCMOS33"),
                ("N8", "OUTPUT_LVCMOS33"),
            ],
        ),
        ("selftest", &[("R6", "OUTPUT_LVCMOS33")]),
        (
            "facedancer",
            &[
                ("T6", "OUTPUT_LVCMOS33"),
                ("R6", "OUTPUT_LVCMOS33"),
                ("T8", "BIDIR_LVCMOS33"),
                ("T7", "BIDIR_LVCMOS33"),
                ("M7", "BIDIR_LVCMOS33"),
                ("N7", "BIDIR_LVCMOS33"),
                ("N8", "OUTPUT_LVCMOS33"),
            ],
        ),
    ];
    let mut checked = 0usize;
    for (name, wanted) in BOTTOM {
        let Some(bytes) = reference(name) else {
            return;
        };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        for (ball, standard) in wanted {
            let pad = fabric
                .pad(ball)
                .unwrap_or_else(|| panic!("{ball} is not in the ball map"));
            assert_eq!(pad.edge, trellis::Edge::Bottom, "{ball}");
            assert_eq!(pad.bel.1, 50, "{ball}: the buffer is on row 50");
            assert_eq!(
                pad.pad_at,
                (pad.bel.0 + u32::from(pad.side == 'B'), 50),
                "{ball}: side B is one column east"
            );
            assert_eq!(pad.pic_at, pad.pad_at, "{ball}: one tile, not two");
            assert_eq!(pad.cib_at, (pad.pad_at.0, 49), "{ball}: the CIB is north");
            assert_eq!(pad.bank, 8, "{ball}: bank 8, the configuration bank");
            // This edge's own widths, which are not the other edges'.
            assert_eq!(pad.bidir_pad_bits.len(), 10, "{ball}: BIDIR_LVCMOS33");
            assert_eq!(pad.output_pad_bits.len(), 7, "{ball}: OUTPUT_LVCMOS33");
            assert_eq!(pad.input_pad_bits.len(), 5, "{ball}: INPUT_LVCMOS33");
            assert_eq!(pad.output_pic_bits, pad.output_pad_bits, "{ball}");
            assert_eq!(pad.input_pic_bits, pad.input_pad_bits, "{ball}");
            assert_eq!(pad.hysteresis_bits.len(), 1, "{ball}");
            assert_eq!(pad.pull_bits(trellis::PULL_NONE).len(), 1, "{ball}");
            assert_eq!(pad.slew_bits("FAST").len(), 1, "{ball}");

            let field = format!("PIO{}.BASE_TYPE", pad.side);
            let value = decoded
                .enums
                .iter()
                .find(|(at, what, _)| *at == pad.pad_at && *what == field)
                .map(|(_, _, v)| v.as_str());
            assert_eq!(
                value,
                Some(*standard),
                "{name}.bit: {field} at {:?} ({ball})",
                pad.pad_at
            );
            let value = *standard;
            let bits = match value {
                "BIDIR_LVCMOS33" => &pad.bidir_pad_bits,
                "INPUT_LVCMOS33" => &pad.input_pad_bits,
                "OUTPUT_LVCMOS33" => &pad.output_pad_bits,
                other => panic!("{name}.bit has {field} = {other} on {ball}"),
            };
            for (what, bits) in [
                ("the base type", &bits[..]),
                ("the pull mode", pad.pull_bits(trellis::PULL_NONE)),
            ] {
                for bit in bits {
                    let (frame, index) = fabric.frames.locate(pad.pad_at, *bit).unwrap();
                    assert!(
                        stream.cram.get(frame, index),
                        "F{frame}B{index}, which this crate sets for {what} of a {value} pad on \
                         {ball}, is clear in {name}.bit"
                    );
                    checked += 1;
                }
            }
            // Hysteresis follows the direction here exactly as it does on the
            // other edges: on for an input or a bidirectional pad, clear for
            // an output.
            let hyst = pad.hysteresis_bits.iter().all(|bit| {
                let (frame, index) = fabric.frames.locate(pad.pad_at, *bit).unwrap();
                stream.cram.get(frame, index)
            });
            assert_eq!(
                hyst,
                value != "OUTPUT_LVCMOS33",
                "{name}.bit: hysteresis on {ball}, a {value} pad"
            );
            // And no slew rate on any of them: none of these resources asks
            // for one, unlike the ULPI pins of either transceiver.
            for bit in pad.slew_bits("FAST") {
                let (frame, index) = fabric.frames.locate(pad.pad_at, *bit).unwrap();
                assert!(
                    !stream.cram.get(frame, index),
                    "F{frame}B{index} is `SLEWRATE = FAST` on {ball}, which asks for none"
                );
            }
        }
        // Nothing on the whole edge belongs to no pad, the same honest
        // direction the left edge gets. There is no second-copy count here,
        // since the only tile is the pad tile.
        let mut pads = 0usize;
        let mut orphans: Vec<String> = Vec::new();
        for (at, field, value) in &decoded.enums {
            let Some(side) = field
                .strip_prefix("PIO")
                .and_then(|rest| rest.strip_suffix(".BASE_TYPE"))
                .and_then(|s| s.chars().next())
            else {
                continue;
            };
            if at.1 != 50 {
                continue;
            }
            if fabric
                .io
                .iter()
                .any(|p| p.edge == trellis::Edge::Bottom && p.pad_at == *at && p.side == side)
            {
                pads += 1;
            } else {
                orphans.push(format!("{field} = {value} at {at:?}"));
            }
        }
        assert!(
            orphans.is_empty(),
            "{name}.bit configures a bottom-edge PIO that belongs to no ball: {orphans:?}"
        );
        let expected = match name {
            "analyzer" => 5,
            "selftest" => 1,
            _ => 7,
        };
        assert_eq!(pads, expected, "{name}.bit: bottom-edge pads");
    }
    // An output is seven bits of base type and one of pull mode, an input
    // five and one, a bidirectional pad ten and one.
    assert_eq!(
        checked,
        // analyzer: T6, R6, T8 and N8 outputs, T7 an input.
        (7 + 1) * 4 + (5 + 1)
            // selftest: R6 alone.
            + (7 + 1)
            // facedancer: T6, R6 and N8 outputs, four bidirectional dq balls.
            + (7 + 1) * 3
            + (10 + 1) * 4,
        "every bit of every bottom-edge ball these files configure"
    );
}

/// What is left of Lattice's own bitstreams that this database cannot name,
/// on all three of them, and where it is.
///
/// This is the counterpart of `what_lattices_own_packer_writes_for_a_...`:
/// that one asks whether eight balls read back right, this one asks whether
/// anything anywhere does not. Both matter, and this one is the honest
/// direction — a decoder is only as good as the bits it cannot explain, and
/// naming them is how the pseudo-differential case was found in the first
/// place.
///
/// The answer, for `analyzer.bit`, `selftest.bit` and `facedancer.bit`
/// alike, is **five bits of one `DSP_SPINE_UL1` tile** and nothing else:
/// 250 001, 27 006 and 424 160 set bits and the same five left over in all
/// three, at the same position. The same five in three unrelated designs is
/// what says it is a property of the tile and not of a design — most likely
/// a feature the fuzzers never named, since the position is fixed and every
/// build of every design sets it.
///
/// Before the resolution rule changed these were 34, 33 and 25, and all the
/// difference was pad tiles: two bits for each pair of bidirectional pads
/// sharing one, on the left edge (the HyperRAM's bus) as well as the right
/// (the ULPI ones). So this test is the measurement that rule is judged by,
/// and it is asserted **exactly** rather than as "few", because a rule that
/// explains more bits than it should would pass a bound and fail this.
#[test]
fn the_only_bits_of_lattices_own_bitstreams_this_database_cannot_name() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    for (name, bits) in [
        ("analyzer", 250_001usize),
        ("selftest", 27_006),
        ("facedancer", 424_160),
    ] {
        let Some(bytes) = reference(name) else {
            return;
        };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        assert_eq!(decoded.bits, bits, "{name}: how big it is");
        let spelled: Vec<String> = decoded
            .leftovers
            .iter()
            .map(|(ty, at, bit)| format!("{ty} {bit} at {at:?}"))
            .collect();
        assert_eq!(
            spelled,
            [
                "DSP_SPINE_UL1 F11B0 at (3, 13)",
                "DSP_SPINE_UL1 F13B0 at (3, 13)",
                "DSP_SPINE_UL1 F2B0 at (3, 13)",
                "DSP_SPINE_UL1 F3B0 at (3, 13)",
                "DSP_SPINE_UL1 F5B0 at (3, 13)",
            ],
            "{name}"
        );
    }
}

/// Every pin of the auxiliary ULPI transceiver, in the platform file's own
/// order: the eight data balls, then `dir`, `nxt`, `stp`, `rst` and `clk`.
///
/// All thirteen are one `ULPIResource` and so all thirteen carry the same
/// `attrs=Attrs(IO_TYPE="LVCMOS33", SLEWRATE="FAST")`, whatever direction
/// Amaranth gives each of them — which is the point of asking about them
/// together, since nextpnr's condition for writing a slew rate is the
/// attribute and not the direction.
#[cfg(feature = "verilog")]
const AUX_ULPI_ALL: [&str; 13] = [
    "F16", "G15", "G16", "H15", "J15", "J16", "K15", "K16", "E16", "F15", "E15", "J13", "D16",
];

/// The "what does `ecppack` write, **in full**, for a slew rate?" question,
/// asked of the three files whose every ULPI pin asks for one.
///
/// This is the same question that found [`trellis::BANK_VCCIO`] and an
/// input's `PULLMODE` missing, asked about the last attribute of Great Scott
/// Gadgets' `ULPIResource` this backend did not write. The answer, read out
/// of their own bitstreams at absolute frame positions:
///
/// | | |
/// |---|---|
/// | Where | the **pad** tile, the one `HYSTERESIS` and `PULLMODE` are in |
/// | How much | **one bit** for `FAST`; `SLOW` is the field's default and is that bit clear |
/// | For which pads | all thirteen pins of the resource — inputs (`dir`, `nxt`), outputs (`stp`, `rst`, `clk`) and the eight bidirectional data balls alike |
/// | Of the base type's bits | **none**: it is a setting of its own, like the pull mode and unlike hysteresis |
/// | When this writes it | only when a constraint asks, which is nextpnr's own condition |
///
/// The last row is why this is a test about *their* files and not about this
/// crate's output: a design that does not ask for a slew rate must come out
/// of this flow bit for bit as it did before the field existed here, and
/// `the_bidirectional_design_routes_and_configures_what_its_header_promises`
/// and the rest are the assertions that say so.
#[test]
#[cfg(feature = "verilog")]
fn what_lattices_own_packer_writes_for_a_slew_rate() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let mut checked = 0usize;
    for name in ["analyzer", "selftest", "facedancer"] {
        let Some(bytes) = reference(name) else {
            return;
        };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        for ball in AUX_ULPI_ALL {
            let pad = fabric
                .pad(ball)
                .unwrap_or_else(|| panic!("{ball} is not in the ball map"));
            // One bit, and the default is that bit clear.
            assert_eq!(
                pad.slew_bits("FAST").len(),
                1,
                "{ball}: PIO{}.SLEWRATE = FAST",
                pad.side
            );
            assert!(
                pad.slew_bits("SLOW").is_empty(),
                "{ball}: SLOW is the field's default and costs nothing"
            );
            // A setting of its own: the base type does not contain it, so a
            // pad written without it is written without it.
            for bits in [
                &pad.bidir_pad_bits,
                &pad.input_pad_bits,
                &pad.output_pad_bits,
            ] {
                assert!(
                    !bits.iter().any(|bit| pad.slew_bits("FAST").contains(bit)),
                    "{ball}: the slew rate is no longer a setting of its own"
                );
            }
            // And it is set, in their file, at the absolute frame position
            // this crate would write.
            for bit in pad.slew_bits("FAST") {
                let (frame, index) = fabric
                    .frames
                    .locate(pad.pad_at, *bit)
                    .unwrap_or_else(|| panic!("{bit:?} is outside {:?}", pad.pad_at));
                assert!(
                    stream.cram.get(frame, index),
                    "F{frame}B{index}, which is PIO{}.SLEWRATE = FAST on {ball}, is clear in \
                     {name}.bit — whose platform file asks for SLEWRATE=FAST on that very pin",
                    pad.side
                );
                checked += 1;
            }
            // Read back through the database by name, too, so this is not
            // only an assertion about a bit position.
            let field = format!("PIO{}.SLEWRATE", pad.side);
            let value = decoded
                .enums
                .iter()
                .find(|(at, what, _)| *at == pad.pad_at && what == &field)
                .map(|(_, _, value)| value.as_str());
            assert_eq!(value, Some("FAST"), "{name}.bit: {field} on {ball}");
        }
    }
    assert_eq!(checked, 3 * 13, "one bit per ULPI pin per file");
}

/// Where every flip-flop's data comes from, in Lattice's own bitstreams for
/// this very board, and what they do about one whose data is a constant.
///
/// This is the "what does the vendor write, **in full**?" question asked
/// about the defect that cost eight rounds of looking somewhere else: a
/// flip-flop whose data input is the constant zero. The answer, read out of
/// `analyzer.bit`, `selftest.bit` and `facedancer.bit` by walking their own
/// arcs backwards from every `M` wire:
///
/// | | |
/// |---|---|
/// | Undriven data wires | **none.** Every one of 1135 (analyzer), 215 (selftest) and 3132 (facedancer) flip-flops with `REG<n>.SD = 0` has something routed to its `M` wire. The vendor never leaves that wire floating |
/// | What a constant is made of | a **lookup table**, `SLICEA.K0` in all four cases, with `INIT` all zeros for a zero and all ones for a one, every one of its four inputs tied high (`A0MUX`..`D0MUX = 1`), `MODE` left at `LOGIC` |
/// | How many | **one per constant per design**, shared: analyzer has one of each feeding 12 and 18 flip-flops, facedancer one of each feeding 8 and 32, and its output fans out to nine hundred-odd sinks across the whole die |
/// | What it costs | sixteen `INIT` bits for a zero, **none** for a one — all ones is the field's default — plus the four tie bits either way |
/// | What they never write | `SLICE<l>.M<n>MUX`. The database declares a `1` value for it, which would tie the `M` wire high for two bits and no LUT at all, and not one of the three files sets it anywhere |
///
/// That is nextpnr's `pack_constants` verbatim — a `$PACKER_GND` and a
/// `$PACKER_VCC` LUT4 with `INIT` 0 and 0xFFFF — and it is what
/// `techcells::drive_constant_data` now builds, which is why this test is
/// about *their* files: it is the statement of intent the flow is measured
/// against.
///
/// `selftest.bit` has no constant on a flip-flop's data pin at all, and that
/// is asserted rather than skipped: a design that needs no constant is the
/// case the flow must not change.
#[test]
fn what_lattices_own_packer_writes_for_a_constant() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    // Per file: how many flip-flops take their data from the fabric at all,
    // then the lookup tables that make a zero and the ones that make a one.
    let expected: [(&str, usize, Vec<Constant>, Vec<Constant>); 3] = [
        (
            "analyzer",
            1135,
            vec![((38, 27), 'A', 0, 12)],
            vec![((5, 27), 'A', 0, 18)],
        ),
        ("selftest", 215, vec![], vec![]),
        (
            "facedancer",
            3132,
            vec![((11, 6), 'A', 0, 8)],
            vec![((64, 8), 'A', 0, 32)],
        ),
    ];
    for (name, fabric_fed, zeros, ones) in expected {
        let Some(bytes) = reference(name) else { return };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        let (arcs, unresolved) = db.resolved_arcs(&decoded);
        assert!(unresolved.is_empty(), "{name}: {unresolved:?}");
        let mut driver = std::collections::BTreeMap::new();
        for (to, from) in &arcs {
            driver.insert(to.clone(), from.clone());
        }
        // Every flip-flop whose data comes from the fabric, and where that
        // data ultimately comes from.
        let mut undriven: Vec<String> = Vec::new();
        let mut fed_by: std::collections::BTreeMap<(String, (u32, u32)), usize> =
            std::collections::BTreeMap::new();
        let mut flops = 0usize;
        for (at, field, value) in &decoded.enums {
            let Some(z) = fabric_data_index(field, value) else {
                continue;
            };
            flops += 1;
            let mut wire = (format!("M{z}"), *at);
            if !driver.contains_key(&wire) {
                undriven.push(format!("M{z} at X{}Y{}", at.0, at.1));
                continue;
            }
            let mut seen = std::collections::BTreeSet::new();
            while let Some(next) = driver.get(&wire) {
                if !seen.insert(next.clone()) {
                    break;
                }
                wire = next.clone();
            }
            *fed_by.entry(wire).or_default() += 1;
        }
        assert_eq!(flops, fabric_fed, "{name}: flip-flops with REG<n>.SD = 0");
        // The headline: the vendor never leaves a flip-flop's data wire
        // floating. That is the check this flow did not have for eight
        // rounds of looking somewhere else.
        assert!(
            undriven.is_empty(),
            "{name}.bit has {} flip-flop(s) with nothing routed to their data wire: {undriven:?}",
            undriven.len()
        );
        // Of those sources, the ones that are a constant lookup table.
        let mut found_zero: Vec<Constant> = Vec::new();
        let mut found_one: Vec<Constant> = Vec::new();
        for ((wire, at), fed) in &fed_by {
            let Some(index) = wire.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let letter = ['A', 'B', 'C', 'D'][(index / 2) as usize];
            let half = index % 2;
            let field = format!("SLICE{letter}.K{half}.INIT");
            let init = decoded
                .words
                .iter()
                .find(|(pos, what, _)| pos == at && *what == field)
                .map(|(_, _, value)| value.as_str())
                // A word at its default is not reported, and this field's
                // default is all ones.
                .unwrap_or("1111111111111111");
            let here = (*at, letter, half, *fed);
            if init.chars().all(|c| c == '0') {
                found_zero.push(here);
            } else if init.chars().all(|c| c == '1') {
                found_one.push(here);
            }
        }
        assert_eq!(found_zero, zeros, "{name}: the LUTs that make a zero");
        assert_eq!(found_one, ones, "{name}: the LUTs that make a one");

        // And what those cost, at absolute frame positions, read as the bits
        // this crate's own `LutBits` would write.
        for (at, letter, half, _) in zeros.iter().chain(ones.iter()) {
            let ty = fabric
                .arch
                .tile_index_at(at.0, at.1)
                .expect("a position of the grid");
            let lut = fabric
                .luts
                .get(&(ty, format!("SLICE{letter}.K{half}")))
                .expect("a logic tile declares eight lookup tables");
            let zero = zeros.iter().any(|(pos, _, _, _)| pos == at);
            for (bit, groups) in lut.init_zero.iter().enumerate() {
                for bit_at in groups {
                    let (frame, index) = fabric
                        .frames
                        .locate(*at, *bit_at)
                        .expect("a bit of the position it belongs to");
                    assert_eq!(
                        stream.cram.get(frame, index),
                        zero,
                        "{name}.bit: F{frame}B{index} is bit {bit} of SLICE{letter}.K{half}.INIT \
                         at X{}Y{}, the LUT that makes the constant {}",
                        at.0,
                        at.1,
                        u32::from(!zero)
                    );
                }
            }
            // Every input tied high, whichever constant it makes, which is
            // what makes the value independent of what an unrouted input
            // reads as.
            for (input, groups) in lut.tie_high.iter().enumerate() {
                assert!(!groups.is_empty(), "input {input} has no tie");
                for bit_at in groups {
                    let (frame, index) = fabric
                        .frames
                        .locate(*at, *bit_at)
                        .expect("a bit of the position it belongs to");
                    assert!(
                        stream.cram.get(frame, index),
                        "{name}.bit: F{frame}B{index}, which ties input {input} of \
                         SLICE{letter}.K{half} at X{}Y{} high, is clear",
                        at.0,
                        at.1
                    );
                }
            }
        }

        // The slice's own tie for an `M` wire, which the database declares
        // and the vendor does not use anywhere.
        let ties: Vec<&String> = decoded
            .enums
            .iter()
            .filter(|(_, field, _)| {
                field.contains(".M") && field.ends_with("MUX") && !field.contains("MODE")
            })
            .map(|(_, field, _)| field)
            .collect();
        assert!(
            ties.is_empty(),
            "{name}.bit ties an M wire in the slice: {ties:?}"
        );
    }
}

/// The bits one setting of a mux needs, as `(frame, bit, wants it clear)`
/// inside the tile, sorted so two spellings of the same pattern compare equal.
type Pattern = Vec<(u32, u32, bool)>;

/// One source of a mux: where it comes from, and the pattern that selects it.
type Source = (String, Pattern);

/// **What an unused reset wire is, in the database and in Lattice's own
/// files** — the question left open beside the constant, and it is settled
/// here without a board.
///
/// `configure_registers` writes nothing for a flip-flop that does not use its
/// reset, and the note beside the constant said why that was uncomfortable:
/// "an unrouted `LSR` evidently does not hold a flop in reset the way an
/// unrouted `M` held its data at one — but *why* is unmeasured, and a register
/// that resets itself every clock would look exactly like the `stage` fault
/// did". Three things, and the third is the answer.
///
/// **1. The reset wire has a constant zero and the data wire has no constant
/// at all.** `CIB.JLSR0MUX = 0` is not a field of its own: it is a twentieth
/// code point of the *same nineteen-source mux* that `PLC2`'s `.mux LSR0`
/// describes — the same two bits per source, at the same position, because a
/// logic position's `CIB` and `PLC2` tiles overlap. So "tie the reset low" and
/// "route something to the reset" are one mux with one bit pattern, and one of
/// its settings is a zero. A flip-flop's data wire has **no** such setting:
/// there is no `CIB.JM<n>MUX` in the database, and the only constant the slice
/// offers on `M` at all is `SLICE<l>.M<n>MUX = 1`, a **one**. The hardware can
/// hold a reset low and cannot hold a data pin low, which is the whole reason
/// one of them needed `techcells::drive_constant_data` and the other did not.
///
/// | Pin | Constants the database offers |
/// |---|---|
/// | `JCE<n>` (enable) | `1` only — an unused enable must be high or the flop never clocks |
/// | `JCLK<n>` (clock) | `0` only |
/// | `JLSR<n>` (reset) | `0` only |
/// | `JD<n>` (a lookup table's inputs) | `0` **and** `1` |
/// | `JM<n>` (a flip-flop's data) | **none**, and the slice's own tie is a `1` |
///
/// **2. No flip-flop of the three reference files is in `PRLD` mode**, so
/// `LSRMODE` is not what takes their resets out of the question and the wire
/// is.
///
/// **3. And their own working bitstreams leave hundreds of flip-flops on a
/// reset wire that nothing drives and nothing ties.** Of the logic tiles that
/// hold flip-flops, the ones with no arc *and* no tie on either of the tile's
/// two `LSR` wires: **84 of 438** in `analyzer.bit`, 3 of 91 in
/// `selftest.bit`, **551 of 1159** in `facedancer.bit`. If an unrouted `LSR`
/// held a flop in reset, `analyzer.bit` would be a logic analyser with 84
/// tiles of dead registers, and it is Great Scott Gadgets' shipped gateware.
///
/// And the ties they *do* write — 30 in `analyzer.bit`, none in
/// `selftest.bit`, 153 in `facedancer.bit` — are **never at a position that
/// holds a flip-flop**, not one of the 183. So the vendor does not tie a reset
/// wire for a flip-flop's sake either; those ties belong to something else in
/// the `CIB`, and the note's guess that they were the flops' is wrong.
///
/// That retires the question. What it does **not** settle is a voltage:
/// nothing here measures what an unselected mux output reads as, on `LSR` or
/// on `M`. The `M` case was measured the expensive way, by a register coming
/// up set on a part; the `LSR` case is settled the cheap way instead, by three
/// of the vendor's own files not doing it in a thousand places.
#[test]
fn what_lattices_own_packer_writes_for_an_unused_reset() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();

    // ---- 1. which control pins the CIB can tie, and to what ----
    let cib = db
        .tile_database("CIB")
        .expect("every logic position has a CIB tile");
    let ties = |stem: &str| -> Vec<String> {
        let mut values: Vec<String> = cib
            .enums
            .iter()
            .filter(|(field, _, _)| field.starts_with(&format!("CIB.{stem}")))
            .flat_map(|(_, _, values)| values.iter())
            .map(|(value, _)| value.clone())
            .filter(|value| value == "0" || value == "1")
            .collect();
        values.sort();
        values.dedup();
        values
    };
    assert_eq!(ties("JCE"), ["1"], "an unused enable is tied high, only");
    assert_eq!(ties("JCLK"), ["0"], "an unused clock is tied low, only");
    assert_eq!(ties("JLSR"), ["0"], "an unused reset is tied low, only");
    assert_eq!(
        ties("JD"),
        ["0", "1"],
        "a lookup table's input can be tied either way"
    );
    assert_eq!(
        ties("JM"),
        [] as [String; 0],
        "a flip-flop's data pin has no tie at all, which is the whole reason \
         `techcells::drive_constant_data` exists"
    );

    // The one constant the *slice* offers on a data pin, and it is a one:
    // exactly what an unrouted wire already reads as, which is why it buys
    // nothing and why a zero has to be built.
    let plc2 = db
        .tile_database("PLC2")
        .expect("the logic tile of this family");
    let slice_m: Vec<&str> = plc2
        .enums
        .iter()
        .filter(|(field, _, _)| field.starts_with("SLICEA.M") && field.ends_with("MUX"))
        .flat_map(|(_, _, values)| values.iter())
        .map(|(value, _)| value.as_str())
        .filter(|value| *value == "0" || *value == "1")
        .collect();
    assert!(
        !slice_m.is_empty() && slice_m.iter().all(|value| *value == "1"),
        "the slice's tie for a data pin is a one and only a one: {slice_m:?}"
    );

    // And the shape that makes the reset's zero a zero: `CIB.JLSR0MUX` and
    // `PLC2`'s `.mux LSR0` are the **same mux**, so the tie is one more source
    // of it — a setting no route can collide with, and one the data wire's mux
    // has no equivalent of.
    let pattern = |bits: &[reticle::fpga::trellis::parse::DbBit]| -> Pattern {
        let mut out: Pattern = bits.iter().map(|b| (b.frame, b.bit, b.inverted)).collect();
        out.sort_unstable();
        out
    };
    let sources = |db: &reticle::fpga::trellis::parse::TileDatabase, sink: &str| -> Vec<Source> {
        let mut out: Vec<Source> = db
            .muxes
            .iter()
            .filter(|(to, _, _)| to == sink)
            .map(|(_, from, bits)| (from.clone(), pattern(bits)))
            .collect();
        out.sort();
        out
    };
    let in_the_cib = sources(cib, "JLSR0");
    let in_the_plc = sources(plc2, "LSR0");
    assert!(!in_the_plc.is_empty(), "the logic tile routes a reset wire");
    assert_eq!(
        in_the_cib, in_the_plc,
        "`CIB.JLSR0`'s mux and `PLC2.LSR0`'s mux are the same bits, so the tie is a source of \
         the routing mux and not a field beside it"
    );
    let tie = cib
        .enums
        .iter()
        .find(|(field, _, _)| field == "CIB.JLSR0MUX")
        .and_then(|(_, _, values)| values.iter().find(|(value, _)| value == "0"))
        .map(|(_, bits)| pattern(bits))
        .expect("the reset wire can be tied low");
    assert!(
        in_the_plc.iter().all(|(_, bits)| *bits != tie),
        "the tie's pattern is one of the mux's routing sources, so it is not a distinct setting"
    );
    assert_eq!(tie.len(), 2, "one code point of a two-bits-per-source mux");

    // ---- 2. and what their own files do with it ----
    //
    // Per file: logic tiles holding at least one flip-flop; of those, the ones
    // with **nothing at all** on either of the tile's two reset wires — not
    // routed and not tied; and the ties the packer did write, with how many of
    // them sit in a tile whose *other* reset wire carries a signal.
    let expected: [(&str, usize, usize, usize, usize); 3] = [
        ("analyzer", 438, 84, 30, 0),
        ("selftest", 91, 3, 0, 0),
        ("facedancer", 1159, 551, 153, 0),
    ];
    for (name, with_flops, bare, written, at_a_flop_tile) in expected {
        let Some(bytes) = reference(name) else { return };
        let stream = Ecp5Stream::parse(&bytes, &formats).unwrap();
        let decoded = db.decode(&stream.cram);
        let (arcs, unresolved) = db.resolved_arcs(&decoded);
        assert!(unresolved.is_empty(), "{name}: {unresolved:?}");
        // Where a reset actually arrives, as a fact about the file's own arcs,
        // kept per mux: a tile has two and they are independent.
        let mut routed: std::collections::BTreeSet<((u32, u32), u32)> =
            std::collections::BTreeSet::new();
        for (to, _) in &arcs {
            let (wire, at) = to;
            if let Some(c) = wire.strip_prefix("LSR").and_then(|n| n.parse::<u32>().ok()) {
                routed.insert((*at, c));
            }
        }
        // Where the flip-flops are, where a tie was written, and whether any
        // flop is in `PRLD` mode — which would take the reset out of the
        // question a different way and has to be ruled out, not assumed.
        let mut flops: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
        let mut ties: std::collections::BTreeSet<((u32, u32), u32)> =
            std::collections::BTreeSet::new();
        let mut preload = 0usize;
        for (at, field, value) in &decoded.enums {
            if fabric_data_index(field, value).is_some() {
                flops.insert(*at);
            }
            if field.ends_with(".LSRMODE") && value == "PRLD" {
                preload += 1;
            }
            if let Some(c) = field
                .strip_prefix("CIB.JLSR")
                .and_then(|rest| rest.strip_suffix("MUX"))
                .and_then(|n| n.parse::<u32>().ok())
                && value == "0"
            {
                ties.insert((*at, c));
            }
        }
        assert_eq!(
            preload, 0,
            "{name}.bit puts a flip-flop in PRLD mode, so `LSRMODE` and not the wire is what \
             takes its reset out of the question and this measurement means something else"
        );
        // The tiles that hold flip-flops and have **nothing** on either reset
        // wire: no arc, no tie. If an unrouted `LSR` reset a flop every clock,
        // a working bitstream could not contain one of these.
        let bare_tiles: Vec<(u32, u32)> = flops
            .iter()
            .copied()
            .filter(|at| (0..2).all(|c| !routed.contains(&(*at, c)) && !ties.contains(&(*at, c))))
            .collect();
        // And where the ties they *did* write are, which is the other half of
        // the answer: not in the tiles that hold the flip-flops.
        let beside: usize = ties.iter().filter(|(at, _)| flops.contains(at)).count();
        assert_eq!(flops.len(), with_flops, "{name}: tiles holding flip-flops");
        assert_eq!(
            bare_tiles.len(),
            bare,
            "{name}: tiles with flip-flops and nothing at all on either reset wire"
        );
        assert_eq!(ties.len(), written, "{name}: `CIB.JLSR<n>MUX = 0` written");
        assert_eq!(
            beside, at_a_flop_tile,
            "{name}: of those, the ones at a position that holds a flip-flop"
        );
        // The headline, and the reason this flow's silence about an unused
        // `LSR` is a tidiness gap and not a correctness one: their own working
        // bitstream leaves hundreds of flip-flops sitting on a reset wire that
        // nothing drives and nothing ties.
        assert!(
            bare_tiles.len() > 2 * ties.len(),
            "{name}.bit ties a serious fraction of its unused reset wires ({} tied against {} \
             left bare), which would make this flow's silence a real gap",
            ties.len(),
            bare_tiles.len()
        );
    }
}

/// One constant driver of a reference bitstream: where it is, which slice
/// and which half of it, and how many flip-flops take their data from it.
type Constant = ((u32, u32), char, u32, usize);

/// The flip-flop index a `SLICE<l>.REG<n>.SD` field names, for the value
/// `0` — the one that says the data comes from the fabric's `M` wire — and
/// `None` for any other field or value.
fn fabric_data_index(field: &str, value: &str) -> Option<u32> {
    if value != "0" {
        return None;
    }
    let rest = field.strip_prefix("SLICE")?;
    let letter = *rest.as_bytes().first()?;
    let half = rest.strip_prefix(&format!("{}.REG", char::from(letter)))?;
    let half = half.strip_suffix(".SD")?.parse::<u32>().ok()?;
    let letter = u32::from(letter.checked_sub(b'A')?);
    (letter < 4 && half < 2).then_some(2 * letter + half)
}

/// A register bit nothing in the design ever sets is **built**, out of a
/// constant this flow makes, and the bit comes up **clear**.
///
/// This test used to pin a refusal. The refusal was honest — a flip-flop
/// whose data input nothing drives comes up as a *one* on this family, so
/// writing one would have been writing a device that does something else —
/// but refusing is a capability regression: a register bit that is genuinely
/// constant is legal Verilog and legal silicon, and this backend now builds
/// it the way Lattice's own packer does. See
/// `what_lattices_own_packer_writes_for_a_constant` for what that is, read
/// out of their own bitstreams.
///
/// `testdata/fpga/cynthion/wide_state.v` is the reproducer: three bits, four
/// values, so `state[2]` is a flip-flop whose data input is the constant
/// zero. `ip/usb_device_fs/rtl/usb_ctrl_ep.v` had exactly that in its
/// `stage` register, and read back off a real ECP5 through a debug port
/// `stage` was **5**: every `case (stage)` label missed, every IN token the
/// host sent was answered from the `default` arm with a NAK, and the
/// transfer died of the five-second timeout the kernel prints as `device
/// descriptor read/64, error -110`. Nothing about that bitstream was
/// unexplained and 26 916 of 26 916 bits decoded; the only thing wrong was a
/// flip-flop loading a one where the design said zero.
///
/// What is pinned here is the whole of the fix:
///
/// 1. the design **compiles**, places, routes and configures;
/// 2. the netlist holds the constant driver, by name, and only the one the
///    design needs;
/// 3. the finished bitstream holds exactly one lookup table whose `INIT` is
///    all zeros — the constant — and its four inputs are tied high, so its
///    output does not depend on what an unrouted input reads as;
/// 4. the flip-flop that wanted the zero takes its data, through the
///    bitstream's **own** arcs walked backwards, from that lookup table:
///    nothing is floating any more;
/// 5. and every one of the image's set bits still decodes.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_register_bit_nothing_drives_is_built_from_a_constant() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, pads, _, _, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/wide_state.v",
        "testdata/fpga/cynthion/wide_state.rcf",
    );

    // ---- 1. it builds ----
    assert_eq!(pads, 2, "a clock in and one LED out");
    assert_eq!(routed.ffs, 3, "three bits, and the third is the constant");
    assert!(routed.dropped.is_empty(), "{:?}", routed.dropped);

    // ---- 2. the constant driver is in the netlist, and only the one ----
    let luts: Vec<&str> = routed
        .netlist
        .instances
        .iter()
        .filter(|i| i.kind == "lut")
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(
        luts.iter().filter(|n| n.starts_with("const0")).count(),
        1,
        "one driver for the one constant the design needs: {luts:?}"
    );
    assert!(
        !luts.iter().any(|n| n.starts_with("const1")),
        "nothing in this design wants a one: {luts:?}"
    );

    // ---- 3. and one in the bitstream, with its inputs tied ----
    let decoded = db.decode(&stream.cram);
    let zero_luts: Vec<((u32, u32), String)> = decoded
        .words
        .iter()
        .filter(|(_, field, value)| field.ends_with(".INIT") && value.chars().all(|c| c == '0'))
        .map(|(at, field, _)| (*at, field.clone()))
        .collect();
    assert_eq!(
        zero_luts.len(),
        1,
        "one lookup table holding the constant zero: {zero_luts:?}"
    );
    let (at, field) = &zero_luts[0];
    let bel = field.trim_end_matches(".INIT");
    let letter = bel.as_bytes()[5];
    let half: u32 = bel[bel.len() - 1..].parse().unwrap();
    let ty = fabric.arch.tile_index_at(at.0, at.1).unwrap();
    let lut = fabric.luts.get(&(ty, bel.to_owned())).unwrap();
    for (input, groups) in lut.tie_high.iter().enumerate() {
        for bit_at in groups {
            let (frame, index) = fabric.frames.locate(*at, *bit_at).unwrap();
            assert!(
                stream.cram.get(frame, index),
                "F{frame}B{index} ties input {input} of {bel} at X{}Y{} high, and it is clear: a \
                 constant whose inputs are not tied depends on what an unrouted input reads as",
                at.0,
                at.1
            );
        }
    }

    // ---- 4. the flip-flop's data comes from it ----
    //
    // Walked backwards through the bitstream's own arcs, so this is a
    // statement about the file and not about the router's bookkeeping.
    let (arcs, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    let mut driver = std::collections::BTreeMap::new();
    for (to, from) in &arcs {
        driver.insert(to.clone(), from.clone());
    }
    let index = 2 * u32::from(letter - b'A') + half;
    let output = (format!("F{index}"), *at);
    let mut fed = 0usize;
    let mut floating: Vec<String> = Vec::new();
    for (pos, sd, value) in &decoded.enums {
        let Some(z) = fabric_data_index(sd, value) else {
            continue;
        };
        let start = (format!("M{z}"), *pos);
        let mut wire = start.clone();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(next) = driver.get(&wire) {
            if !seen.insert(next.clone()) {
                break;
            }
            wire = next.clone();
        }
        if wire == start {
            floating.push(format!("M{z} at X{}Y{}", pos.0, pos.1));
        } else if wire == output {
            fed += 1;
        }
    }
    assert!(
        floating.is_empty(),
        "a flip-flop's data wire has nothing routed to it: {floating:?}"
    );
    assert_eq!(
        fed, 1,
        "exactly one flip-flop of this design takes its data from the constant"
    );

    // ---- 5. and every bit still decodes ----
    assert_eq!(
        decoded.bits,
        stream.cram.count_ones(),
        "the decoder and the writer disagree about how many bits are set"
    );
    assert_eq!(
        decoded.unexplained, 0,
        "{} of {} bit(s) belong to no feature the database names: {:?}",
        decoded.unexplained, decoded.bits, decoded.leftovers
    );
    let (selected, _) = db.resolved_arcs(&decoded);
    assert_eq!(
        selected, routed.wires,
        "the bits select connections the router did not choose, or fail to select ones it did"
    );
    // The size of the finished image, and what the constant cost in it.
    // Without a constant driver the same design is 171 bits, 45 arcs that
    // cost bits, 4 `.config` words and 34 enumerated fields; with one it is
    // 199, 47, 5 and 38. The 28 new bits are all accounted for: **16** for
    // the `INIT` of the lookup table, one bit per entry of a truth table
    // that is all zeros, **8** for the four two-bit ties that hold its
    // inputs high, and **4** for the two arcs that carry its output to the
    // flip-flop's `M` wire, two bits each. Nothing else in the image moved.
    assert_eq!(
        decoded.bits, 199,
        "the size of the finished image, of which 28 bits are the constant"
    );
    assert_eq!(decoded.words.len(), 5, "four mapped LUTs and the constant");
}

/// The sources of the ULPI USB device, top level first, the way
/// `reticle fpga` wants them: there is no search path.
#[cfg(all(feature = "verilog", feature = "synth"))]
const USB_SOURCES: [&str; 4] = [
    "testdata/fpga/cynthion/usb_ulpi_device.v",
    "ip/usb_device_ulpi/rtl/usb_ulpi_link.v",
    "ip/usb_device_ulpi/rtl/usb_device_ulpi.v",
    "ip/usb_device_fs/rtl/usb_ctrl_ep.v",
];

/// The constraints that go with them.
#[cfg(all(feature = "verilog", feature = "synth"))]
const USB_RCF: &str = "testdata/fpga/cynthion/usb_ulpi_device.rcf";

/// **The flip-flop that carries the constant zero out of the part survives
/// synthesis**, and so does the lookup table that drives it.
///
/// This is the guard on an experiment, and the experiment is the last
/// unmeasured half of the constant driver: a flip-flop whose data input is
/// the constant *one* is confirmed in silicon by `usb_ulpi_link`'s `rst_q`,
/// and a constant *zero* never had a design to be confirmed in, because the
/// registers that used to supply one **were** the defect that cost eight
/// rounds and narrowing them removed them.
///
/// `usb_ulpi_device.v` puts one back, in the one place on this board a
/// program can read a register bit rather than a person looking at a lamp:
/// `zero_probe` is XORed into the byte endpoint 1 hands back, so
/// `tests/usb_loopback.rs` passes untouched when the bit is zero and fails
/// with every byte complemented when it is not.
///
/// **Why this test has to exist.** A compiler may delete a flip-flop whose
/// value is a known constant, and `synth::opt::FfOpt` does — so an
/// experiment built out of one can quietly stop being an experiment and pass
/// whatever the backend writes, which is worse than no test at all. What
/// keeps this one alive is not an attribute and not luck: `zero_probe` is
/// initialised to **one** and clocked to **zero**, so its value before the
/// first edge differs from its data and no optimiser may fold it away
/// without changing what the design means. `FfOpt`'s rule says exactly that
/// — a constant `d` collapses only when the initial value agrees with it.
/// This test is what says that argument still holds, and it needs no
/// database and no board: it is synthesis and the netlist, nothing more.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_usb_devices_constant_zero_probe_survives_synthesis() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::source::SourceMap;

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut asts = Vec::new();
    for name in USB_SOURCES {
        let Ok(text) = std::fs::read_to_string(name) else {
            eprintln!("skipped: `{name}` is not in this copy of the crate");
            return;
        };
        let id = map.add(name, &text).expect("fits");
        asts.push(reticle::verilog::parse_source(
            &mut map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        ));
    }
    let rcf = std::fs::read_to_string(USB_RCF).expect("the constraints");
    let rcf_file = map.add(USB_RCF, &rcf).expect("fits");
    let refs: Vec<_> = asts.iter().collect();
    let mut design =
        reticle::verilog::elaborate(&refs, &reticle::verilog::ElabOptions::default(), &mut diags)
            .expect("the design elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.expect("a top level");
    let device = target(DEVICE).expect("the device file describes this part");
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .expect("it maps onto the device");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    // The mapped netlist, in the `.rtl` text form `reticle fpga --netlist`
    // writes, which is what a person would look at by hand.
    let text = design.to_text();
    let flop = text
        .lines()
        .find(|line| line.trim_start().starts_with("cell zero_probe$ff "))
        .unwrap_or_else(|| {
            panic!(
                "`zero_probe` has been optimised out of the netlist, so the loopback's \
                 constant-zero check on the part cannot fail and proves nothing. See this \
                 test's docs: the register is initialised to one and clocked to zero for \
                 exactly this reason"
            )
        });
    assert!(
        flop.contains("TRELLIS_FF"),
        "`zero_probe` is not a device flip-flop: {flop}"
    );
    assert!(
        flop.contains("DI=%const0"),
        "`zero_probe`'s data does not come from the constant-zero driver: {flop}"
    );
    // And the driver, which is the thing being measured on the part: a
    // lookup table whose truth table is all zeros, so its output does not
    // depend on what an unrouted input reads as.
    let driver = text
        .lines()
        .find(|line| line.trim_start().starts_with("cell const0$lut "))
        .expect("`techcells::drive_constant_data` builds a driver for the constant");
    assert!(
        driver.contains("LUT4") && driver.contains("INIT=16'd0"),
        "the constant-zero driver is not an all-zeros lookup table: {driver}"
    );
    // One driver, shared, the way nextpnr's `pack_constants` does it.
    assert_eq!(
        text.lines()
            .filter(|line| line.trim_start().starts_with("cell const0$lut"))
            .count(),
        1,
        "one constant-zero driver for the whole module"
    );
}

/// The same flip-flop, followed into the `.bit`: the `INIT` word, the input
/// ties and the arcs that carry the constant to its data wire.
///
/// `a_register_bit_nothing_drives_is_built_from_a_constant` does this on a
/// three-line design, which is where the mechanism is pinned. What this adds
/// is that it holds in **the design that is loaded into the part** — 1028
/// lookup tables, 408 flip-flops and sixteen distributed RAMs — because a
/// result read off a board is worth nothing until the bitstream that produced
/// it has been read.
///
/// It is `#[ignore]`d because it places and routes the whole USB device,
/// which is minutes rather than seconds; the rest of this file is seconds.
/// Run it with:
///
/// ```console
/// cargo test --release --all-features --test fpga_trellis -- --ignored \
///     the_usb_devices_constant_zero_probe_reaches_the_bitstream --nocapture
/// ```
#[test]
#[ignore = "places and routes the whole USB device: minutes, not seconds"]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_usb_devices_constant_zero_probe_reaches_the_bitstream() {
    let Some(root) = chipdb() else { return };
    for name in USB_SOURCES {
        if !Path::new(name).exists() {
            eprintln!("skipped: `{name}` is not in this copy of the crate");
            return;
        }
    }
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, pads, _, _, routed) = compile_all(&fabric, &USB_SOURCES, USB_RCF);
    assert_eq!(pads, 20, "the ULPI bus, the clock, the reset and six LEDs");

    // ---- the netlist ----
    let flops: Vec<&str> = routed
        .netlist
        .instances
        .iter()
        .filter(|i| i.kind == "ff")
        .map(|i| i.name.as_str())
        .collect();
    assert!(
        flops.iter().any(|n| n.starts_with("zero_probe")),
        "the constant-zero probe is not in the netlist"
    );
    let zeros = routed
        .netlist
        .instances
        .iter()
        .filter(|i| i.kind == "lut" && i.name.starts_with("const0"))
        .count();
    assert_eq!(zeros, 1, "one constant-zero driver, shared");

    // ---- the `INIT` word, and the ties that make it a constant ----
    let decoded = db.decode(&stream.cram);

    // A **distributed RAM's storage is an `INIT` word too**, and this design
    // has sixteen of them: `usb_bulk_ep`'s two 64-byte buffers are eight
    // `TRELLIS_DPR16X4` each. Their contents at configuration are empty —
    // neither buffer is initialised, because a distributed RAM has no reset
    // pin and nothing needs the clear — so every one of them puts six all-zero
    // `INIT` words in the image, the four of `DPRAM_DATA_LUTS` and the two of
    // `DPRAM_RAMW_LUTS`. Ninety-six of those against one real constant is not
    // a probe this test can find by looking for zeros alone.
    //
    // A slice in that mode says so, so the RAMs are separated by the mode
    // rather than by their position or their count: `SLICEA.MODE = DPRAM`
    // marks the tile, and the six words of a marked tile are the RAM's bits.
    // Asserting the total is what keeps this honest — a seventeenth RAM, or a
    // RAM whose contents were *not* empty, changes that number and is worth
    // being told about.
    let ram_tiles: std::collections::BTreeSet<(u32, u32)> = decoded
        .enums
        .iter()
        .filter(|(_, field, value)| field == "SLICEA.MODE" && value == "DPRAM")
        .map(|(at, _, _)| *at)
        .collect();
    assert_eq!(
        ram_tiles.len(),
        16,
        "the two 64-byte endpoint buffers are eight distributed RAMs each"
    );
    let all_zero_inits: Vec<((u32, u32), String)> = decoded
        .words
        .iter()
        .filter(|(_, field, value)| field.ends_with(".INIT") && value.chars().all(|c| c == '0'))
        .map(|(at, field, _)| (*at, field.clone()))
        .collect();
    let in_a_ram = all_zero_inits
        .iter()
        .filter(|(at, _)| ram_tiles.contains(at))
        .count();
    assert_eq!(
        in_a_ram,
        6 * 16,
        "six all-zero `INIT` words per distributed RAM: four of data and the \
         `RAMW` slice's two, all sixteen RAMs empty"
    );
    let zero_luts: Vec<((u32, u32), String)> = all_zero_inits
        .into_iter()
        .filter(|(at, _)| !ram_tiles.contains(at))
        .collect();
    assert_eq!(
        zero_luts.len(),
        1,
        "one lookup table of this design holds the constant zero: {zero_luts:?}"
    );
    let (at, field) = &zero_luts[0];
    let bel = field.trim_end_matches(".INIT");
    let letter = bel.as_bytes()[5];
    let half: u32 = bel[bel.len() - 1..].parse().unwrap();
    let ty = fabric.arch.tile_index_at(at.0, at.1).unwrap();
    let lut = fabric.luts.get(&(ty, bel.to_owned())).unwrap();
    for (input, groups) in lut.tie_high.iter().enumerate() {
        for bit_at in groups {
            let (frame, index) = fabric.frames.locate(*at, *bit_at).unwrap();
            assert!(
                stream.cram.get(frame, index),
                "F{frame}B{index} ties input {input} of {bel} at X{}Y{} high, and it is clear",
                at.0,
                at.1
            );
        }
    }

    // ---- and the arcs, walked backwards out of the file itself ----
    let (arcs, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    let mut driver = std::collections::BTreeMap::new();
    for (to, from) in &arcs {
        driver.insert(to.clone(), from.clone());
    }
    let output = (format!("F{}", 2 * u32::from(letter - b'A') + half), *at);
    let mut fed = 0usize;
    let mut floating: Vec<String> = Vec::new();
    let mut flops_from_fabric = 0usize;
    for (pos, sd, value) in &decoded.enums {
        let Some(z) = fabric_data_index(sd, value) else {
            continue;
        };
        flops_from_fabric += 1;
        let start = (format!("M{z}"), *pos);
        let mut wire = start.clone();
        let mut seen = std::collections::BTreeSet::new();
        while let Some(next) = driver.get(&wire) {
            if !seen.insert(next.clone()) {
                break;
            }
            wire = next.clone();
        }
        if wire == start {
            floating.push(format!("M{z} at X{}Y{}", pos.0, pos.1));
        } else if wire == output {
            fed += 1;
        }
    }
    // 408. It was 1440 while `usb_bulk_ep`'s two 64-byte buffers were shift
    // registers and 1024 of them were the buffers; those bits are in the
    // sixteen distributed RAMs above now. It was 491 before that, when a
    // packet was eight bytes. What matters here is not the number but that
    // **every one** of them takes its data from the fabric and none is
    // floating, which is the defect this test exists for.
    assert_eq!(
        flops_from_fabric, 408,
        "every flip-flop of this design takes its data from the fabric"
    );
    assert!(
        floating.is_empty(),
        "a flip-flop's data wire has nothing routed to it, which is the defect itself: \
         {floating:?}"
    );
    assert_eq!(
        fed, 1,
        "exactly one flip-flop takes its data from the constant-zero lookup table, and it is \
         the probe"
    );

    // ---- every bit still decodes ----
    assert_eq!(
        decoded.bits,
        stream.cram.count_ones(),
        "the decoder and the writer disagree about how many bits are set"
    );
    assert_eq!(
        decoded.unexplained, 0,
        "{} of {} bit(s) belong to no feature the database names: {:?}",
        decoded.unexplained, decoded.bits, decoded.leftovers
    );
}

/// The ball the bidirectional pad is on: `led_n[0]`, the LED at the end of
/// the row away from the USER button. `bidir_loopback.rcf` says why it is
/// safe to drive and to release.
#[cfg(all(feature = "verilog", feature = "synth"))]
const PROBE: &str = "E13";

/// The bidirectional milestone: `bidir_loopback.v`, and every claim its
/// header makes about what reaches the part.
///
/// The header says a person should see one LED blinking alone until the
/// `USER` button is held and four blinking together while it is. Nothing
/// here can check that — that is what a person is for — so what is checked
/// is everything between the Verilog and the bits:
///
/// 1. the design places and routes **completely**, and every sink walks back
///    to its driver;
/// 2. the pad on E13 is configured `BIDIR_LVCMOS33` — not `INPUT`, not
///    `OUTPUT`, and not the two written together — in both of its tiles;
/// 3. its pull mode is `UP`, which is what the released level depends on,
///    and it is `UP` rather than merely "not the default": `NONE`'s bits are
///    a subset of `UP`'s, so the claim has to be about a bit only `UP` has;
/// 4. **its tristate is not tied.** `configure_io` writes `CIB.JB0MUX = 0`
///    for every ordinary output, which holds the tristate wire low so the
///    buffer always drives. Here a signal drives that wire, and the tie and
///    the route are *one mux*, so the tie must not be written. This is the
///    half of the question no finished bitstream can be asked, so it is
///    asked of the pass itself;
/// 5. all three of the pad's wires — data in, data out and tristate — carry
///    a **routed signal**, and the tristate's comes from the button's own
///    pad on the other edge of the die;
/// 6. no bit an arc needs **clear** has been set by anything else;
/// 7. and every bit of the finished image decodes back, through the same
///    records the router read, into exactly the arcs the router chose — with
///    E13's base type and pull mode read back out of the image rather than
///    out of the pass that wrote them.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_bidirectional_design_routes_and_configures_what_its_header_promises() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, pads, report, io_only, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/bidir_loopback.v",
        "testdata/fpga/cynthion/bidir_loopback.rcf",
    );
    assert_eq!(pads, 8, "the clock, the button and the six LED balls");
    assert_eq!(routed.ffs, 26, "the counter");
    assert_eq!(report.signals, routed.netlist.routable().len());
    assert!(
        routed.clocks.off_network.is_empty(),
        "a clock off the global network: {:?}",
        routed.clocks.off_network
    );
    assert!(routed.dropped.is_empty(), "{:?}", routed.dropped);

    // ---- the pad itself, as `configure_io` alone wrote it ----------------
    let pad = fabric.pad(PROBE).expect("E13 is in the ball map");
    assert_eq!(pad.side, 'B');
    assert_eq!(pad.bel, (62, 0));
    assert_eq!(pad.pad_at, (63, 0));
    // The second-copy tile and the `CIB` are the same *position*, holding
    // two windows that the loader addresses end to end.
    assert_eq!(pad.pic_at, (63, 1));
    assert_eq!(pad.cib_at, (63, 1));

    // What `configure_io` wrote on its own. A `CIB`'s constant mux and its
    // routing mux are one mux, so "is this pad tied?" cannot be asked of the
    // finished bitstream; it can be asked of this.
    let alone = fabric.stream(&io_only, "8").unwrap();
    let set = |at: (u32, u32), bit: &reticle::fpga::arch::ConfigBit| {
        let (frame, index) = fabric
            .frames
            .locate(at, *bit)
            .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
        alone.cram.get(frame, index)
    };

    for bit in &pad.bidir_pad_bits {
        assert!(
            set(pad.pad_at, bit),
            "{bit:?} of BIDIR_LVCMOS33 is clear in what `configure_io` wrote for E13"
        );
    }
    for bit in &pad.bidir_pic_bits {
        assert!(set(pad.pic_at, bit), "{bit:?} of the second copy");
    }
    // Neither an input nor an output. All three patterns share `F2B0` and
    // `F9B0` and then diverge, so what is decisive is that the bits
    // `BIDIR_LVCMOS33` has and the other two lack are the ones that are set.
    for (other, name) in [
        (&pad.input_pad_bits, "INPUT_LVCMOS33"),
        (&pad.output_pad_bits, "OUTPUT_LVCMOS33"),
    ] {
        let only: Vec<_> = pad
            .bidir_pad_bits
            .iter()
            .filter(|bit| !other.contains(bit))
            .collect();
        assert!(!only.is_empty(), "BIDIR_LVCMOS33 differs from {name}");
        for bit in only {
            assert!(
                set(pad.pad_at, bit),
                "{bit:?} is in BIDIR_LVCMOS33 and not in {name}, and it is clear"
            );
        }
    }
    // AND A THING WORTH KNOWING, because it is why this cannot be asserted
    // the other way round. The one bit `OUTPUT_LVCMOS33` has that
    // `BIDIR_LVCMOS33` does not is **also** `PULLMODE`'s low bit, so on this
    // family "an output" and "a pull that is not a pull-down" are one bit. A
    // finished image therefore cannot be asked whether a pad is an output:
    // `BIDIR` plus a pull is a superset of `OUTPUT`'s pattern, and the
    // decoding at the end of this test resolves it by the longer match. This
    // is the third of four places on this part where two features share bit
    // space — the others are a `CIB` tie against a route, a centre mux's
    // six-bit code, and a right-edge pseudo-differential base type reaching
    // into its neighbour's bits — and all four are in one table in
    // `docs/fpga-trellis.md`.
    let output_only: Vec<_> = pad
        .output_pad_bits
        .iter()
        .filter(|bit| !pad.bidir_pad_bits.contains(bit))
        .collect();
    assert_eq!(output_only.len(), 1, "one bit apart: {output_only:?}");
    assert!(
        pad.pull_bits(trellis::PULL_NONE).contains(output_only[0]),
        "{output_only:?} is no longer shared with PULLMODE, so the paragraph above is stale and \
         the stronger assertion it explains away is now available"
    );
    // The pull, and `UP` rather than `NONE`.
    let up_only: Vec<_> = pad
        .pull_bits(trellis::PULL_UP)
        .iter()
        .filter(|bit| !pad.pull_bits(trellis::PULL_NONE).contains(bit))
        .collect();
    assert_eq!(up_only.len(), 1, "`UP` is `NONE` plus one bit");
    for bit in pad.pull_bits(trellis::PULL_UP) {
        assert!(
            set(pad.pad_at, bit),
            "{bit:?} of PULLMODE=UP is clear, so a released E13 has no pull-up and the design \
             reads whatever the LED leaves on the pin"
        );
    }
    // Hysteresis, which costs nothing here because the base type's own bits
    // already contain it — asserted anyway so that "written" stays true.
    for bit in &pad.hysteresis_bits {
        assert!(set(pad.pad_at, bit), "{bit:?} of HYSTERESIS=ON");
    }
    // AND THE TRISTATE IS NOT TIED, which is the assertion the milestone is
    // about.
    assert!(!pad.enable_bits.is_empty(), "there is a tie to not write");
    assert!(
        pad.enable_bits.iter().any(|bit| !set(pad.cib_at, bit)),
        "`configure_io` tied E13's tristate although the router drives it: the pad would drive \
         at all times and the button would do nothing"
    );
    // While a LED next door, an ordinary output, *is* tied — so the
    // assertion above is about this pad and not about a pass that stopped
    // tying anything.
    let led = fabric.pad("C13").expect("C13 is in the ball map");
    for bit in &led.enable_bits {
        assert!(
            set(led.cib_at, bit),
            "{bit:?}: C13 is a plain output and its tristate is not tied"
        );
    }

    // ---- and all three of the pad's wires carry a routed signal ---------
    let netlist = &routed.netlist;
    let probe = netlist
        .instances
        .iter()
        .position(|i| i.pin.as_deref() == Some(PROBE))
        .expect("the pad is constrained to E13");
    let button = netlist
        .instances
        .iter()
        .position(|i| i.pin.as_deref() == Some(BUTTON))
        .expect("the button is constrained to M14");
    let pin_of = |instance: usize, role: &str| {
        netlist
            .pins
            .iter()
            .find(|pin| pin.instance == instance && pin.role == role)
    };
    for role in ["din", "dout", "oe"] {
        let pin = pin_of(probe, role)
            .unwrap_or_else(|| panic!("E13's `{role}` pin is not in the netlist"));
        assert!(
            pin.signal.is_some(),
            "E13's `{role}` carries no signal, so it is tied and not routed"
        );
    }
    // The tristate comes from the button's own pad, which is on the other
    // edge of the die: M14's buffer is at (72, 32) and E13's at (62, 0).
    let enable = pin_of(probe, "oe").unwrap().signal.unwrap();
    let driver = netlist.signals[enable]
        .driver
        .expect("the tristate has a driver");
    assert_eq!(
        netlist.pins[driver].instance, button,
        "E13's tristate is driven by something other than the USER button's pad"
    );

    // ---- and the whole image says what the router said -------------------
    let decoded = db.decode(&stream.cram);
    assert_eq!(decoded.unexplained, 0, "bit(s) left over");
    assert!(decoded.bits > 2000, "{} set bit(s)", decoded.bits);
    let (selected, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    assert_eq!(selected, fabric.routed_arcs(&routed.graph, &routed.routing));
    // E13's own settings, read back out of the finished image through the
    // database rather than out of the pass that wrote them.
    let mine: Vec<(&str, &str)> = decoded
        .enums
        .iter()
        .filter(|(at, _, _)| *at == pad.pad_at)
        .map(|(_, field, value)| (field.as_str(), value.as_str()))
        .collect();
    assert!(
        mine.contains(&("PIOB.BASE_TYPE", "BIDIR_LVCMOS33")),
        "E13's base type reads back as {mine:?}"
    );
    assert!(
        mine.contains(&("PIOB.PULLMODE", "UP")),
        "E13's pull mode reads back as {mine:?}"
    );
}

/// The ULPI USB device's top level, simulated against the smallest
/// transceiver that gets it through its start-up.
///
/// The device itself is checked in `tests/ip_library.rs`, against a
/// transceiver model and a host model written in Rust; what is checked here
/// is the part of it that only a **top level** has — the eight-bit
/// turnaround built out of pads, the input side of the same pads reaching
/// the core so that its register readback can work, the power-on reset, and
/// the interface clock leaving the part.
///
/// It skips when `ip/` is not in this copy of the crate, the way
/// `tests/soc.rs` skips without `examples/soc`: the IP library is published
/// as part of the repository and excluded from the `.crate`, so a consumer
/// building from crates.io has the design and not the blocks it
/// instantiates.
#[test]
#[cfg(all(feature = "verilog", feature = "sim"))]
fn the_usb_devices_top_level_configures_a_transceiver_through_its_pads() {
    use reticle::diag::Diagnostics;
    use reticle::sim::{SimOptions, Simulator};
    use reticle::source::SourceMap;
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    let sources = [
        "testdata/fpga/cynthion/usb_ulpi_device_tb.v",
        "testdata/fpga/cynthion/usb_ulpi_device.v",
        "ip/usb_device_ulpi/rtl/usb_ulpi_link.v",
        "ip/usb_device_ulpi/rtl/usb_device_ulpi.v",
        "ip/usb_device_fs/rtl/usb_ctrl_ep.v",
    ];
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for name in sources {
        let Ok(text) = std::fs::read_to_string(name) else {
            eprintln!("skipped: `{name}` is not in this copy of the crate");
            return;
        };
        let id = map.add(name, &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &ElabOptions::new(Dialect::Verilog2005), &mut diags)
        .expect("the testbench elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let mut sim = Simulator::new(&design, SimOptions::default()).expect("it simulates");
    sim.run();
    assert!(sim.finished(), "the testbench did not reach `$finish`");
    assert_eq!(
        sim.output(),
        "PASS: the transceiver was configured and read back through the pads\n",
        "`usb_ulpi_device_tb.v` disagrees with `usb_ulpi_device.v`"
    );
}

/// The ULPI trace's console, simulated: the instrument that found the fault.
///
/// `usb_ulpi_trace.v` is `usb_ulpi_device.v` with a logic analyser on the pin
/// Apollo bridges to `/dev/ttyACM0`, and it is how a device that had given one
/// bit of information per bitstream for eight rounds gave three hundred in one.
/// Two of its own bugs are the reason this test exists, and the testbench's
/// header names both: a trigger that matched the cycle after a PID instead of
/// the next byte of the packet, which a stub handing bytes over back to back
/// could not catch, and a dump that rotated the trace's shift register partway
/// through filling it and left every later dump four entries out of step.
///
/// So the assertion is the whole line, character for character, of the last
/// complete dump. It needs `sim` and not `fpga`, so it runs in builds with no
/// device database at all, and it skips without `ip/` the way the top level's
/// own test does.
#[test]
#[cfg(all(feature = "verilog", feature = "sim"))]
fn the_ulpi_traces_console_prints_the_bus_in_order() {
    use reticle::diag::Diagnostics;
    use reticle::sim::{SimOptions, Simulator};
    use reticle::source::SourceMap;
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    let sources = [
        "testdata/fpga/cynthion/usb_ulpi_trace_tb.v",
        "testdata/fpga/cynthion/usb_ulpi_trace.v",
        "ip/usb_device_ulpi/rtl/usb_ulpi_link.v",
        "ip/usb_device_ulpi/rtl/usb_device_ulpi.v",
        "ip/usb_device_fs/rtl/usb_ctrl_ep.v",
    ];
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for name in sources {
        let Ok(text) = std::fs::read_to_string(name) else {
            eprintln!("skipped: `{name}` is not in this copy of the crate");
            return;
        };
        let id = map.add(name, &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &ElabOptions::new(Dialect::Verilog2005), &mut diags)
        .expect("the testbench elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let mut sim = Simulator::new(&design, SimOptions::default()).expect("it simulates");
    sim.run();
    assert!(sim.finished(), "the testbench did not reach `$finish`");
    assert_eq!(
        sim.output(),
        "PASS: the trace is the bus, in order, from its oldest entry\n",
        "`usb_ulpi_trace_tb.v` disagrees with `usb_ulpi_trace.v`"
    );
}

/// What `bidir_bus.v`'s header tells a person to look for, simulated.
///
/// Everything else in this file is about the bits. This is about the design,
/// and it is here for the reason the headers in `testdata/fpga/cynthion/`
/// are written before anybody presses anything: an observable nobody has
/// checked is an observable that can be talked into agreeing with whatever
/// happens. `bidir_bus_tb.v` drives the design with the bus left to it,
/// which is what a loopback through the pads is, and prints one line per
/// way the table could be wrong.
///
/// It needs `sim` and not `fpga`, so it runs in builds that have no device
/// database at all and skips in builds without a simulator.
#[test]
#[cfg(all(feature = "verilog", feature = "sim"))]
fn the_bus_designs_leds_say_what_its_header_says_they_will() {
    use reticle::diag::Diagnostics;
    use reticle::sim::{SimOptions, Simulator};
    use reticle::source::SourceMap;
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for name in [
        "testdata/fpga/cynthion/bidir_bus_tb.v",
        "testdata/fpga/cynthion/bidir_bus.v",
    ] {
        let text = std::fs::read_to_string(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let id = map.add(name, &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &ElabOptions::new(Dialect::Verilog2005), &mut diags)
        .expect("the testbench elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let mut sim = Simulator::new(&design, SimOptions::default()).expect("it simulates");
    sim.run();
    assert!(sim.finished(), "the testbench did not reach `$finish`");
    // The testbench prints nothing but its verdict, so any other line is a
    // way the header is wrong — and the message says which.
    assert_eq!(
        sim.output(),
        "PASS: eight pads drove a one and a zero and read back all sixteen\n",
        "`bidir_bus_tb.v` disagrees with `bidir_bus.v`'s header"
    );
}

/// The eight-bit bus milestone: `bidir_bus.v`, on the die's **right** edge,
/// and every claim its header makes about what reaches the part.
///
/// The difference from `bidir_loopback.v` is the edge, and on the right edge
/// four PIOs share one pad tile. Three of the four tiles these eight balls
/// land in hold **two** bidirectional pads at once, which is the case the
/// bitstream could not be read back through until `decode` stopped resolving
/// a field by the longest match; see `docs/fpga-trellis.md`. So what is
/// checked here is:
///
/// 1. the design places and routes **completely**, and every sink walks back
///    to its driver;
/// 2. the eight balls are **three** pad tiles, every one of them holding
///    more than one of the eight — two, two and all four sides at once — so
///    the test is about the case that was refused and not about eight
///    independent pads;
/// 3. every one of the eight is `BIDIR_LVCMOS33` in the finished image, read
///    back through the database, on **both** halves of every pair;
/// 4. every one of the eight has a routed signal on all three of its wires,
///    and none of the eight has its tristate tied;
/// 5. the pull is `NONE` on all eight — the far end of these nets is a
///    transceiver's pin and an internal pull would be fighting the board;
/// 6. the 60 MHz reaches D16's pad as well as the global clock network,
///    which is what `clk_dir='o'` in the board's platform file asks for;
/// 7. and every bit of the image decodes back into exactly the arcs the
///    router chose, with nothing left over.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_bidirectional_bus_routes_and_configures_what_its_header_promises() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, pads, report, io_only, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/bidir_bus.v",
        "testdata/fpga/cynthion/bidir_bus.rcf",
    );
    assert_eq!(
        pads, 20,
        "the clock in, the button, the eight-bit bus, `dir`, the clock out, `stp`, the \
         transceiver's reset and six LEDs"
    );
    assert_eq!(report.signals, routed.netlist.routable().len());
    assert!(
        routed.clocks.off_network.is_empty(),
        "a clock off the global network: {:?}",
        routed.clocks.off_network
    );
    assert!(routed.dropped.is_empty(), "{:?}", routed.dropped);

    // ---- the eight balls are four tiles, and three hold a pair ----------
    let bus: Vec<&trellis::IoSite> = AUX_ULPI_DATA
        .iter()
        .map(|ball| {
            fabric
                .pad(ball)
                .unwrap_or_else(|| panic!("{ball} is not in the ball map"))
        })
        .collect();
    for (ball, pad) in AUX_ULPI_DATA.iter().zip(&bus) {
        assert_eq!(pad.edge, trellis::Edge::Right, "{ball}");
    }
    let mut tiles: Vec<(u32, u32)> = bus.iter().map(|pad| pad.pad_at).collect();
    tiles.sort_unstable();
    tiles.dedup();
    assert_eq!(
        tiles.len(),
        3,
        "eight balls over three pad tiles: {tiles:?}"
    );
    let sides: Vec<usize> = tiles
        .iter()
        .map(|at| bus.iter().filter(|pad| pad.pad_at == *at).count())
        .collect();
    assert_eq!(
        sides,
        vec![2, 2, 4],
        "every one of the three tiles has to hold more than one of the eight — that is what \
         makes this the case that used to be refused, and one of them holds all four sides: \
         {tiles:?}"
    );

    // ---- what `configure_io` wrote for them, on its own -----------------
    let alone = fabric.stream(&io_only, "8").unwrap();
    let set = |at: (u32, u32), bit: &ConfigBit| {
        let (frame, index) = fabric
            .frames
            .locate(at, *bit)
            .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
        alone.cram.get(frame, index)
    };
    for (ball, pad) in AUX_ULPI_DATA.iter().zip(&bus) {
        for bit in &pad.bidir_pad_bits {
            assert!(set(pad.pad_at, bit), "{ball}: {bit:?} of BIDIR_LVCMOS33");
        }
        for bit in &pad.bidir_pic_bits {
            assert!(set(pad.pic_at, bit), "{ball}: {bit:?} of the second copy");
        }
        // The pull is `NONE`, not `UP` and not the database's default of
        // `DOWN`. `NONE`'s bits are a subset of `UP`'s, so the claim has to
        // be about the bit only `UP` has: it must be **clear**.
        for bit in pad.pull_bits(trellis::PULL_NONE) {
            assert!(set(pad.pad_at, bit), "{ball}: {bit:?} of PULLMODE=NONE");
        }
        for bit in pad.pull_bits(trellis::PULL_UP) {
            if !pad.pull_bits(trellis::PULL_NONE).contains(bit) {
                assert!(
                    !set(pad.pad_at, bit),
                    "{ball}: {bit:?} is the bit only PULLMODE=UP has, and it is set — this pad \
                     is pulling against a transceiver's output"
                );
            }
        }
        // And the tristate is not tied: the `CIB` mux that holds an ordinary
        // output's tristate low is the same mux the router drives, so tying
        // it would be a second driver on a wire a signal already drives.
        assert!(
            !pad.enable_bits.is_empty(),
            "{ball}: there is a tie to skip"
        );
        assert!(
            pad.enable_bits.iter().any(|bit| !set(pad.cib_at, bit)),
            "{ball}: `configure_io` tied the tristate although the router drives it, so this pad \
             would drive at all times and the transceiver would be fighting it"
        );
    }
    // While a LED, an ordinary output, *is* tied — so the assertion above is
    // about these eight pads and not about a pass that stopped tying.
    let led = fabric.pad("C13").expect("C13 is in the ball map");
    for bit in &led.enable_bits {
        assert!(set(led.cib_at, bit), "{bit:?}: C13's tristate is not tied");
    }

    // ---- all three wires of all eight carry a routed signal -------------
    let netlist = &routed.netlist;
    let pin_of = |instance: usize, role: &str| {
        netlist
            .pins
            .iter()
            .find(|pin| pin.instance == instance && pin.role == role)
    };
    for ball in AUX_ULPI_DATA {
        let index = netlist
            .instances
            .iter()
            .position(|i| i.pin.as_deref() == Some(ball))
            .unwrap_or_else(|| panic!("no pad is constrained to {ball}"));
        for role in ["din", "dout", "oe"] {
            let pin = pin_of(index, role)
                .unwrap_or_else(|| panic!("{ball}'s `{role}` pin is not in the netlist"));
            assert!(
                pin.signal.is_some(),
                "{ball}'s `{role}` carries no signal, so it is tied and not routed"
            );
        }
    }
    // The clock leaves the part as well as driving the fabric: D16's pad is
    // an output whose data comes from the same net the flip-flops clock on.
    let clock_out = netlist
        .instances
        .iter()
        .position(|i| i.pin.as_deref() == Some("D16"))
        .expect("no pad is constrained to D16");
    assert!(
        pin_of(clock_out, "dout").is_some_and(|pin| pin.signal.is_some()),
        "D16 drives nothing, so the transceiver has no clock and will never let go of the bus"
    );

    // ---- and the whole image says what the router said -------------------
    let decoded = db.decode(&stream.cram);
    assert_eq!(
        decoded.unexplained, 0,
        "{} bit(s) left over: {:?}",
        decoded.unexplained, decoded.leftovers
    );
    let (selected, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    assert_eq!(selected, fabric.routed_arcs(&routed.graph, &routed.routing));
    // Every one of the eight, read back out of the finished image through
    // the database rather than out of the pass that wrote it — which for
    // every one of the three tiles means both halves of a pair at once, and is
    // the assertion this milestone is about.
    for (ball, pad) in AUX_ULPI_DATA.iter().zip(&bus) {
        let field = format!("PIO{}.BASE_TYPE", pad.side);
        let value = decoded
            .enums
            .iter()
            .find(|(at, what, _)| *at == pad.pad_at && *what == field)
            .map(|(_, _, value)| value.as_str());
        assert_eq!(
            value,
            Some("BIDIR_LVCMOS33"),
            "{ball} is {field} at {:?} and reads back wrong",
            pad.pad_at
        );
    }
}

/// The left-edge milestone: `target_ulpi_loopback.v`, sixteen balls of
/// column 0, and every claim its header makes about what would reach a part.
///
/// This is the design the whole exercise is for. Before the left edge was
/// described, every one of these sixteen balls was refused —
/// "`target_clk$io0` is constrained to package pin `T4`, which the
/// architecture maps to no usable site" — so no USB host on the TARGET port
/// and no control of the board's power switches could be built at all.
///
/// **Nothing is loaded onto a board**, and that is not caution for its own
/// sake: a Cynthion r1.4 has no on-board observable on this edge. Its six
/// LEDs are on the top edge and its USER button on the right, which is
/// exactly why this design touches neither — a left-edge rule that was wrong
/// would otherwise be hidden behind a top-edge one that works. So what is
/// checked is everything that can be checked off the part:
///
/// 1. the design places and routes **completely**, every sink walked back to
///    its driver, with the one clock on a global network;
/// 2. all sixteen balls are on `Edge::Left` and in bank 6, over **seven**
///    pad tiles, one of which holds all four PIO sides at once — so this is
///    the shared-tile case and not sixteen independent pads;
/// 3. each of them is the base type its direction calls for, read back out
///    of the finished image through the database;
/// 4. the eight data pads have a **routed** tristate and the four constant
///    outputs have a **tied** one, which is the distinction that would be
///    invisible if the `CIB` were a column out;
/// 5. the slew rate is `FAST` on the thirteen ULPI pins and **clear** on the
///    three switches, which is the constraints file reaching the bitstream
///    per pin;
/// 6. the pull is `UP` on the two inputs this design asks for a pull-up on
///    and `NONE` everywhere else — never the database's default of `DOWN`;
/// 7. bank 6's rail is written and bank 1's, 2's, 3's, 7's and 8's are not,
///    because a design writes the banks it uses;
/// 8. and every bit of the image decodes back into exactly the arcs the
///    router chose, with nothing left over. That was 2729 bits over 751 arcs
///    when this was written, which is reported rather than asserted: the
///    router is free to find a different route of the same design.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_target_ulpi_design_routes_and_configures_what_its_header_promises() {
    let Some(root) = chipdb() else { return };
    let db = trellis::open(&Disk(root), "", PART).unwrap();
    let fabric = db.load(&TrellisOptions::new()).unwrap();
    let (_, stream, pads, report, io_only, routed) = compile(
        &fabric,
        "testdata/fpga/cynthion/target_ulpi_loopback.v",
        "testdata/fpga/cynthion/target_ulpi_loopback.rcf",
    );
    assert_eq!(
        pads, 17,
        "the clock in, eight data balls, `clk` `dir` `nxt` `stp` `rst` and three switches"
    );
    assert_eq!(report.signals, routed.netlist.routable().len());
    assert!(
        routed.clocks.off_network.is_empty(),
        "a clock off the global network: {:?}",
        routed.clocks.off_network
    );
    assert!(routed.dropped.is_empty(), "{:?}", routed.dropped);

    // ---- what each ball is, and what it costs ---------------------------
    //
    // The four constant outputs are the ones whose data is a tie rather than
    // a route: `target_rst_n` holds the transceiver in reset and the three
    // switches are off.
    const TIED: [&str; 4] = ["R4", "K5", "L1", "L2"];
    let want = |ball: &str| -> &'static str {
        if TARGET_ULPI.iter().any(|(b, d)| *b == ball && *d == "BIDIR") {
            "BIDIR"
        } else if matches!(ball, "R3" | "T2") {
            "INPUT"
        } else {
            "OUTPUT"
        }
    };
    let all: Vec<&str> = TARGET_ULPI
        .iter()
        .map(|(b, _)| *b)
        .chain(VBUS_SWITCHES.iter().copied())
        .collect();

    // Seven pad tiles for sixteen balls, and one of them holds all four
    // sides: `L1` `L2` `M1` `M2` are sides A B C D of (col 0, row 26), so
    // their bits are four sides of the one `PICL1` at (col 0, row 27).
    let mut tiles: Vec<(u32, u32)> = Vec::new();
    for ball in &all {
        let pad = fabric
            .pad(ball)
            .unwrap_or_else(|| panic!("{ball} is not in the ball map"));
        assert_eq!(pad.edge, trellis::Edge::Left, "{ball}");
        assert_eq!(pad.bank, 6, "{ball}");
        tiles.push(pad.pad_at);
    }
    tiles.sort_unstable();
    tiles.dedup();
    assert_eq!(
        tiles.len(),
        7,
        "sixteen balls over seven pad tiles: {tiles:?}"
    );
    let busiest = tiles
        .iter()
        .map(|at| {
            all.iter()
                .filter(|ball| fabric.pad(ball).unwrap().pad_at == *at)
                .count()
        })
        .max();
    assert_eq!(
        busiest,
        Some(4),
        "one tile has to hold all four PIO sides, or this is not the shared-tile case"
    );

    // ---- what `configure_io` wrote, on its own --------------------------
    let alone = fabric.stream(&io_only, "8").unwrap();
    let set = |at: (u32, u32), bit: &ConfigBit| {
        let (frame, index) = fabric
            .frames
            .locate(at, *bit)
            .unwrap_or_else(|| panic!("{bit:?} is outside {at:?}"));
        alone.cram.get(frame, index)
    };
    for ball in &all {
        let pad = fabric.pad(ball).unwrap();
        let (pad_bits, pic_bits) = match want(ball) {
            "BIDIR" => (&pad.bidir_pad_bits, &pad.bidir_pic_bits),
            "INPUT" => (&pad.input_pad_bits, &pad.input_pic_bits),
            _ => (&pad.output_pad_bits, &pad.output_pic_bits),
        };
        for bit in pad_bits {
            assert!(set(pad.pad_at, bit), "{ball}: {bit:?} of its base type");
        }
        for bit in pic_bits {
            assert!(set(pad.pic_at, bit), "{ball}: {bit:?} of the second copy");
        }
        // The slew rate, per pin: the thirteen ULPI pins ask for `FAST` and
        // the three switches ask for nothing, exactly as the platform file
        // does. This is the assertion that says the constraint reaches the
        // bitstream one pin at a time rather than per design.
        let fast = VBUS_SWITCHES.iter().all(|b| b != ball);
        for bit in pad.slew_bits("FAST") {
            assert_eq!(
                set(pad.pad_at, bit),
                fast,
                "{ball}: SLEWRATE=FAST at {bit:?} should be {fast}"
            );
        }
        // The pull. `-pullup yes` on the two inputs, because this design
        // holds the transceiver in reset and an undriven `dir` must read as
        // "the bus is not mine". `NONE`'s bits are a subset of `UP`'s, so
        // each claim is made about the bit the other does not have.
        let pulled_up = matches!(*ball, "R3" | "T2");
        let up_only: Vec<&ConfigBit> = pad
            .pull_bits(trellis::PULL_UP)
            .iter()
            .filter(|bit| !pad.pull_bits(trellis::PULL_NONE).contains(bit))
            .collect();
        assert!(!up_only.is_empty(), "{ball}: UP and NONE differ somewhere");
        for bit in up_only {
            assert_eq!(
                set(pad.pad_at, bit),
                pulled_up,
                "{ball}: the bit only PULLMODE=UP has"
            );
        }
        for bit in pad.pull_bits(trellis::PULL_NONE) {
            assert!(
                set(pad.pad_at, bit),
                "{ball}: {bit:?} is in both NONE and UP and is clear, so this pad has the \
                 database's default of PULLMODE=DOWN"
            );
        }
        // The tie, which is the `CIB` one column east of the buffer. A
        // constant output has its data tied low and its tristate tied low; a
        // routed one and a bidirectional pad have neither.
        let tied = TIED.contains(ball);
        assert!(
            !pad.low_bits.is_empty() && !pad.enable_bits.is_empty(),
            "{ball}"
        );
        assert_eq!(
            pad.low_bits.iter().all(|bit| set(pad.cib_at, bit)),
            tied,
            "{ball}: the data wire tied to zero should be {tied}"
        );
        assert_eq!(
            pad.enable_bits.iter().all(|bit| set(pad.cib_at, bit)),
            tied || want(ball) == "OUTPUT",
            "{ball}: an output drives and so has its tristate tied; a bidirectional pad and an \
             input do not"
        );
        // And never the other tie: nothing here asks for a constant one or
        // for a pad released for good.
        assert!(
            !pad.high_bits.iter().all(|bit| set(pad.cib_at, bit)),
            "{ball}: the data wire is tied to a one and nothing asked for that"
        );
        assert!(
            !pad.tristate_bits.iter().all(|bit| set(pad.cib_at, bit)),
            "{ball}: the pad is released for good and nothing asked for that"
        );
    }

    // ---- the banks, which are the setting no pad's tiles hold ------------
    //
    // Bank 6 is the left edge's southern half and bank 0 is the top edge's,
    // where the oscillator is. Nothing else is used, so nothing else is
    // written.
    for (bank, used) in [
        (0u32, true),
        (1, false),
        (2, false),
        (3, false),
        (6, true),
        (7, false),
        (8, false),
    ] {
        let (at, bits) = fabric
            .bank_bits
            .get(&bank)
            .unwrap_or_else(|| panic!("bank {bank} has no rail"));
        assert!(!bits.is_empty(), "bank {bank}");
        assert_eq!(
            bits.iter().all(|bit| set(*at, bit)),
            used,
            "bank {bank}'s rail at {at:?} should be written: {used}"
        );
    }

    // ---- and the whole image says what the router said -------------------
    let decoded = db.decode(&stream.cram);
    assert_eq!(
        decoded.unexplained, 0,
        "{} bit(s) left over: {:?}",
        decoded.unexplained, decoded.leftovers
    );
    let (selected, unresolved) = db.resolved_arcs(&decoded);
    assert!(unresolved.is_empty(), "{unresolved:?}");
    assert_eq!(selected, fabric.routed_arcs(&routed.graph, &routed.routing));

    // Every ball read back out of the finished image rather than out of the
    // pass that wrote it. An **output** is allowed either spelling, because a
    // pseudo-differential value of one side's `BASE_TYPE` reaches across the
    // pair: `L1` and `L2` are sides A and B of one tile and both are
    // outputs, as are `R4` and `T3`, so the fewest-leftovers reading of the
    // odd side is the `D` form. The bits this backend wrote are asserted
    // above, where there is no ambiguity.
    for ball in &all {
        let pad = fabric.pad(ball).unwrap();
        let field = format!("PIO{}.BASE_TYPE", pad.side);
        let value = decoded
            .enums
            .iter()
            .find(|(at, what, _)| *at == pad.pad_at && *what == field)
            .map(|(_, _, value)| value.as_str());
        let expected: &[&str] = match want(ball) {
            "BIDIR" => &["BIDIR_LVCMOS33"],
            "INPUT" => &["INPUT_LVCMOS33"],
            _ => &["OUTPUT_LVCMOS33", "OUTPUT_LVCMOS33D"],
        };
        assert!(
            value.is_some_and(|v| expected.contains(&v)),
            "{ball} is {field} at {:?} and reads back {value:?}, not one of {expected:?}",
            pad.pad_at
        );
    }
}

/// Every label of the TARGET host's report, beside the value it names.
///
/// This exists because that pairing was **wrong on a part for a whole round
/// of work**, and the way it was wrong is the way a report can lie without
/// looking like one: `LABELS` is a table of four-byte elements written back
/// to front, the index into it reversed the elements *and* the characters
/// inside them, and so item 0's value was printed under item 29's name. The
/// console read `VIDL=00 VIDH=00 PHYR=00 RFAL=01` and a reader concluded
/// that the transceiver on the left edge of the die had never answered.
/// What it had actually answered was `0424` — the console was the whole
/// report end for end. `docs/fpga-trellis.md` has both readings.
///
/// How it is checked: the printer is driven on its own. The nine probe
/// slots are written through the simulator's memory handle and the host's
/// outputs are **forced** to distinct values, `probe_done` and `n_items` are
/// forced so that all thirty items print, and `con_in_ready` is forced high
/// because nothing here enumerates the console the bytes would otherwise go
/// to. Then the byte stream the design hands the console is collected and
/// compared character for character against the report those values must
/// produce.
///
/// What it would catch: any shift, reversal or swap between a label and its
/// value, the two multiplexers disagreeing about how many items there are,
/// and a change to `LABELS` or to `item_val` that is not made in both.
///
/// What it would **not** catch: anything about the ULPI bus, the register
/// probe or the enumeration — every one of those is forced here, so a host
/// that never read a register would print this same report. It does not
/// cover the descriptor dump either: `dev_len` and `cfg_len` stay at their
/// reset zero, so the report ends with the items. And it says nothing about
/// which ball a signal reaches; only a part can say that.
#[test]
#[cfg(all(feature = "verilog", feature = "sim"))]
fn every_label_of_the_target_hosts_report_names_the_value_beside_it() {
    use reticle::diag::Diagnostics;
    use reticle::logic::Logic;
    use reticle::sim::{SimOptions, Simulator};
    use reticle::source::SourceMap;
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};

    let sources = [
        "testdata/fpga/cynthion/usb_host_target.v",
        "ip/usb_host_ulpi/rtl/usb_host_ulpi.v",
        "ip/usb_host_ulpi/rtl/usb_ulpi_host_link.v",
        "ip/usb_host_ulpi/rtl/usb_host_sie.v",
        "ip/usb_host_ulpi/rtl/usb_host_enum.v",
        "ip/usb_cdc_acm/rtl/usb_cdc_acm_ulpi.v",
        "ip/usb_cdc_acm/rtl/usb_cdc_acm.v",
        "ip/usb_cdc_acm/rtl/usb_cdc_req.v",
        "ip/usb_device_ulpi/rtl/usb_ulpi_link.v",
        "ip/usb_device_fs/rtl/usb_ctrl_ep.v",
    ];
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut files = Vec::new();
    for name in sources {
        let Ok(text) = std::fs::read_to_string(name) else {
            eprintln!("skipped: `{name}` is not in this copy of the crate");
            return;
        };
        let id = map.add(name, &text).expect("fits");
        files.push(parse_source(
            &mut map,
            id,
            Dialect::Verilog2005,
            &mut NoIncludes,
            &mut diags,
        ));
    }
    let refs: Vec<_> = files.iter().collect();
    let design = elaborate(&refs, &ElabOptions::new(Dialect::Verilog2005), &mut diags)
        .expect("the design elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let options = SimOptions {
        top: Some("usb_host_target".into()),
        ..SimOptions::default()
    };
    let mut sim = Simulator::new(&design, options).expect("it simulates");

    // The nine register answers, written straight into the probe's own
    // array: distinct bytes, and the first four are what a USB3343 really
    // answers to `00h`-`03h`.
    let probe = [0x24u64, 0x04, 0x09, 0x13, 0x45, 0x06, 0x18, 0x1F, 0x39];
    let slots = sim
        .memory("usb_host_target.probe_val")
        .expect("`probe_val` is an array of this design");
    for (index, value) in probe.iter().enumerate() {
        assert!(
            sim.set_mem(slots, index as u64, Logic::from_u64(*value, 8)),
            "probe_val[{index}] is writable"
        );
    }

    // Everything the report reads that this test is not exercising. `forced`
    // is `(net, width, value)`; a one-bit signal is still spelled out, so
    // that the expected text below can be read against this list.
    let forced: &[(&str, u32, u64)] = &[
        // The console takes a byte every clock: nothing here enumerates it.
        ("con_in_ready", 1, 1),
        // The probe is over and the enumeration is running, so all thirty
        // items print rather than the seventeen the probe alone fills in.
        ("probe_done", 1, 1),
        ("n_items", 6, 30),
        // The last receive command, and the two fields the report slices
        // out of it: LineState is its bits 1:0 and VbusState its bits 3:2,
        // so those three items cannot be chosen independently.
        ("h_rx_cmd", 8, 0x5B),
        ("h_rx_cmd_seen", 1, 1),
        ("h_id_pin", 1, 1),
        ("h_phy_ready", 1, 1),
        ("h_stage", 5, 0x0C),
        ("h_attached", 1, 1),
        ("h_low_speed", 1, 0),
        ("h_up", 1, 1),
        ("h_failed", 1, 1),
        ("h_fail_stage", 5, 0x11),
        ("h_fail_status", 3, 0x5),
        ("h_dev_addr", 7, 0x23),
        ("h_maxpkt0", 7, 0x40),
        ("h_cfg_total", 16, 0x0132),
        ("h_cfg_value", 8, 0x77),
        ("h_line_state", 2, 0x1),
        ("h_frame", 11, 0x1A5),
    ];
    for (name, width, value) in forced {
        let path = format!("usb_host_target.{name}");
        let handle = sim
            .net(&path)
            .unwrap_or_else(|| panic!("`{path}` is a net of this design"));
        sim.force(handle, Logic::from_u64(*value, *width));
    }

    let look = |sim: &Simulator, name: &str| {
        let path = format!("usb_host_target.{name}");
        sim.net(&path)
            .unwrap_or_else(|| panic!("`{path}` is a net of this design"))
    };
    let clk = look(&sim, "clk");
    let valid = look(&sim, "con_in_valid");
    let data = look(&sim, "con_in_data");

    // Drive it the way a testbench would: settle with the clock low, read
    // the byte the design is offering, then clock it in. 1200 cycles is the
    // sixteen of the power-on reset, the 27-character banner and thirty
    // nine-character items with room to spare.
    let half = 5;
    let (low, high) = (Logic::from_u64(0, 1), Logic::from_u64(1, 1));
    let mut text = String::new();
    sim.set(clk, low.clone());
    for _ in 0..1200u32 {
        sim.run_for(half);
        if sim.get(valid).to_u64() == Some(1) {
            if let Some(byte) = sim.get(data).to_u64() {
                text.push(byte as u8 as char);
            }
        }
        sim.set(clk, high.clone());
        sim.run_for(half);
        sim.set(clk, low.clone());
    }

    let want = "\r\n== CYNTHION TARGET HOST\r\n\
                VIDL=24\r\nVIDH=04\r\nPIDL=09\r\nPIDH=13\r\n\
                FUNC=45\r\nOTGC=06\r\nINTS=18\r\nDBUG=1f\r\nIOPM=39\r\n\
                RXCM=5b\r\nLINE=03\r\nVBUS=02\r\nIDPN=01\r\nSEEN=01\r\n\
                VBEN=00\r\nPHYR=01\r\nRFAL=00\r\nSTGE=0c\r\nFLAG=0d\r\n\
                FSTG=11\r\nFSTA=05\r\nADDR=23\r\nMPS0=40\r\nCTLO=32\r\n\
                CTHI=01\r\nCFGV=77\r\nDLEN=00\r\nCLEN=00\r\nLIN2=01\r\n\
                FRML=a5\r\n";
    assert_eq!(
        text, want,
        "the report pairs a label with another item's value"
    );
}
