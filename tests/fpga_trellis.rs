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

#[cfg(all(feature = "verilog", feature = "synth"))]
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
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{Netlist, PlaceOptions, place};
    use reticle::fpga::route::{RouteOptions, route};
    use reticle::fpga::{Constraints, FpgaOptions, bitstream, synthesize_for, target};
    use reticle::source::SourceMap;

    let verilog = std::fs::read_to_string(verilog_path).expect("the design");
    let rcf = std::fs::read_to_string(rcf_path).expect("the constraints");
    let mut map = SourceMap::new();
    let source = map.add(verilog_path, &verilog).unwrap();
    let rcf_file = map.add(rcf_path, &rcf).unwrap();
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
        stats.pads, 120,
        "56 PIOs on the top edge and 64 on the right"
    );
    assert_eq!(stats.pads_skipped, 77, "the balls of the other two edges");

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
            ("io".to_owned(), 120),
            ("lut".to_owned(), 24_288)
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
        }
    }
    // 56 balls on the top edge and 64 on the right, which is every ball of
    // the package on either.
    assert_eq!(top, 56);
    assert_eq!(right, 64);
    assert_eq!(seen.len(), 120);

    // A ball on another edge is not in the map at all, rather than being
    // in it and configured nowhere. `T17` is a corner ball of this
    // package and is not a PIO.
    assert!(fabric.pad("T17").is_none());
    assert!(fabric.arch.site_of_pin("no-such-ball").is_none());
    // A left-edge ball is a PIO and is still left out, because that edge's
    // rule has not been checked against a part.
    assert!(fabric.pad("R2").is_none(), "the left edge is not described");
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
    // The two banks of the top edge and the two of the right, because the
    // fabric declares pads on both edges now. Note that 2 and 7 are spelled
    // `BANKREF2A` and `BANKREF7A` by Lattice, which is why the lookup tries
    // both spellings.
    assert_eq!(
        fabric.bank_bits.keys().copied().collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
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

    // AND THE ONE THING THAT CANNOT BE READ BACK, measured on the vendor's
    // own file so that it is a property of the format and not of this crate.
    //
    // F16 and G15 are sides A and B of one right-edge position, so they
    // share the pad tile at (col 72, row 15), and both are ULPI data pins,
    // so `ecppack` made both bidirectional. On the right edge a
    // *pseudo-differential* value of `PIO<s>.BASE_TYPE` reaches across the
    // pair — `PIOA.BASE_TYPE = OUTPUT_LVCMOS33D` is ten bits, four of which
    // are PIOB's — and with both halves bidirectional all ten of them
    // happen to be set. `TrellisDatabase::decode` resolves a field by the
    // longest matching pattern, which is also what `libtrellis`' own
    // `Tile::get_config` does, so ten beats `BIDIR_LVCMOS33`'s eight and
    // side A reads back as a differential output it is not — leaving the
    // two bits only a bidirectional or an input pad wants.
    //
    // This is asserted rather than worked around because the alternative is
    // to change the resolution rule, and a bitstream this crate writes for
    // the same pins has exactly the same property: see "What cannot be read
    // back" in `docs/fpga-trellis.md`, which is also where the candidate fix
    // is. A single bidirectional pad, and a whole bus on the **top** edge
    // where each PIO has a tile of its own, decode with nothing left over.
    let a = fabric.pad("F16").unwrap();
    let b = fabric.pad("G15").unwrap();
    assert_eq!(a.pad_at, b.pad_at, "F16 and G15 share a pad tile");
    assert!(
        decoded.enums.iter().any(|(at, field, value)| {
            *at == a.pad_at && field == "PIOA.BASE_TYPE" && value.ends_with('D')
        }),
        "side A of (col 72, row 15) no longer reads back as a differential output in \
         analyzer.bit, so the ambiguity this documents is gone and the paragraph above is stale"
    );
    let orphans = decoded
        .leftovers
        .iter()
        .filter(|(_, at, _)| *at == a.pad_at)
        .count();
    assert!(
        orphans >= 2,
        "`ecppack`'s own bitstream now decodes that tile completely, so the limitation this \
         documents is not one"
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
    // is the third place on this part where two features share bit space —
    // the others are a `CIB` tie against a route, and a centre mux's six-bit
    // code — and all three are in `docs/fpga-trellis.md`.
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
