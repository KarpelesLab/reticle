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
    let (routing, route_report) =
        route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
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
        .configure_io(&netlist, &placement, &graph, &mut bits)
        .unwrap();
    fabric
        .configure_logic(&design, top, &netlist, &placement, &graph, &mut bits)
        .unwrap();
    // The same pass on its own, into an empty bitmap. A `CIB`'s constant
    // mux and its routing mux are the *same* mux, so the bits that tie a
    // wire and the bits that route into it share bit space and "is this pad
    // tied?" cannot be read off a finished bitstream. Asking the pass
    // directly can.
    let mut io_only =
        bitstream::Bitstream::empty(bitstream::BitstreamFormat::from_arch(&fabric.arch));
    fabric
        .configure_io(&netlist, &placement, &graph, &mut io_only)
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
    (bits, stream, pads, route_report, io_only, Routed { wires })
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
    assert_eq!(stats.luts, 16, "eight per composition that has a PLC2");
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
    assert_eq!(graph.pips.len(), 8_211_900);
    assert_eq!(
        graph.dangling, 53_632,
        "edges that leave the grid, which happens at all four edges"
    );
    assert_eq!(
        graph.bit_patterns(),
        3536,
        "distinct bit patterns, interned once for the whole die"
    );
    assert_eq!(
        graph.site_counts(),
        vec![("io".to_owned(), 120), ("lut".to_owned(), 24_288)]
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
    assert_eq!(pad.pull_none_bits.len(), 1);

    for (what, bits) in [
        ("the base type", &pad.input_pad_bits),
        ("hysteresis", &pad.hysteresis_bits),
        ("the pull mode", &pad.pull_none_bits),
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
        .chain(&button.pull_none_bits)
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
