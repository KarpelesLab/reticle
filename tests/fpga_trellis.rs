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
) -> (reticle::fpga::bitstream::Bitstream, Ecp5Stream, usize) {
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
    // The design's whole point: nothing to route.
    assert!(
        fabric.unroutable(&netlist).is_empty(),
        "this design has something to route, which this fabric cannot do"
    );
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
        LEDS.len(),
        "every constrained pin is held at the site the ball map gives it"
    );
    let (routing, route_report) =
        route(&netlist, &graph, &placement, &RouteOptions::default()).unwrap();
    assert_eq!(route_report.pips, 0, "there is nothing to route");

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
    let stream = fabric.stream(&bits, "8").unwrap();
    (bits, stream, pads)
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
        stats.pads, 56,
        "PIOs of the top edge, which is all that is built"
    );
    assert_eq!(stats.pads_skipped, 141);
    // The text form is what a document quotes, so it has to hold the
    // numbers rather than a summary of them.
    let text = stats.to_text();
    assert!(text.contains("configuration bits: 4476704"), "{text}");
    assert!(text.contains("tiles: 4312"), "{text}");
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
        assert_eq!(pad.pad_bits.len(), 6, "LED{led} {ball}");
        assert_eq!(pad.pic_bits.len(), 2, "LED{led} {ball}");
        for (what, at, bits) in [
            ("the pad tile", (pad.x, 0), &pad.pad_bits),
            ("the tile one row south", (pad.x, 1), &pad.pic_bits),
        ] {
            for bit in bits {
                let (frame, index) = fabric
                    .frames
                    .locate(at, *bit)
                    .unwrap_or_else(|| panic!("LED{led}: {bit:?} is outside {at:?}"));
                assert!(
                    gateware.cram.get(frame, index),
                    "LED{led} ({ball}, side {}, x {}): F{frame}B{index} of {what} is set by \
                     this crate for an output and is clear in the board's own gateware, which \
                     drives this LED. The tile rule is wrong.",
                    pad.side,
                    pad.x
                );
            }
        }
        // The ties are *not* expected to agree: the gateware routes a
        // signal into the data wire where this crate ties a constant, so
        // both of its constant-mux bits are clear there. Asserting that
        // keeps the comparison above honest about what it covers.
        for bit in pad.low_bits.iter().chain(&pad.high_bits) {
            let (frame, index) = fabric.frames.locate((pad.x, 1), *bit).unwrap();
            assert!(
                !gateware.cram.get(frame, index),
                "the gateware drives LED{led} from logic, so it ties nothing"
            );
        }
        // Tying high and tying low are different bits.
        assert_ne!(pad.low_bits, pad.high_bits);
    }
}

/// Every pad of the top edge lands at a distinct site, and side B's tiles
/// are one column east of its ball's.
#[test]
fn a_pads_tiles_are_its_own_side_by_side_with_its_neighbours() {
    let Some(fabric) = open() else { return };
    let mut seen: Vec<(u32, char)> = Vec::new();
    for pad in &fabric.io {
        assert!(
            !seen.contains(&(pad.x, pad.side)),
            "two balls claim X{}Y0/PIO{}",
            pad.x,
            pad.side
        );
        seen.push((pad.x, pad.side));
        // Every pad's own site name round trips through the ball map.
        let site = format!("X{}Y0/PIO{}", pad.x, pad.side);
        assert_eq!(fabric.arch.site_of_pin(&pad.ball), Some(site.as_str()));
        assert_eq!(fabric.pad_of_site(&site).map(|p| &p.ball), Some(&pad.ball));
        // And the three tiles it needs really exist at those positions.
        assert!(fabric.arch.tile_at(pad.x, 0).is_some());
        assert!(fabric.arch.tile_at(pad.x, 1).is_some());
    }
    assert_eq!(seen.len(), 56);

    // A ball on another edge is not in the map at all, rather than being
    // in it and configured nowhere. `T17` is a corner ball of this
    // package and is not a top-edge PIO.
    assert!(fabric.pad("T17").is_none());
    assert!(fabric.arch.site_of_pin("no-such-ball").is_none());
}

/// The whole flow, from Verilog to a `.bit`: the design that lights all
/// six LEDs, which is the one that has been loaded into a part.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_all_on_design_compiles_to_a_bitstream_the_container_reads_back() {
    let Some(fabric) = open() else { return };
    let (bits, stream, pads) = compile(
        &fabric,
        "testdata/fpga/cynthion/leds.v",
        "testdata/fpga/cynthion/leds.rcf",
    );
    assert_eq!(pads, 6, "one pad per LED");
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
            let (f, b) = fabric.frames.locate((pad.x, 1), *bit).unwrap();
            assert!(stream.cram.get(f, b), "LED{led} is not tied low");
        }
        for bit in &pad.high_bits {
            let (f, b) = fabric.frames.locate((pad.x, 1), *bit).unwrap();
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
    let (_, stream, pads) = compile(
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
                let (f, b) = fabric.frames.locate((pad.x, 1), *bit).unwrap();
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
    assert_eq!(
        fabric.bank_bits.keys().copied().collect::<Vec<_>>(),
        vec![0, 1]
    );
    let (at, bank1) = fabric.bank_bits.get(&1).unwrap();
    assert_eq!(bank1.len(), 1, "one bit says the rail");

    let (_, stream, _) = compile(
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

/// A design with anything to route is **refused**, and the refusal names
/// the signals.
///
/// This is the guard that keeps the gap in `src/fpga/trellis/mod.rs`'s
/// header from becoming a lie: that fabric declares no interconnect, so a
/// bitstream for a design that needs some would load, assert `DONE` and do
/// nothing, which is the worst of the available outcomes.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn a_design_with_something_to_route_is_named_rather_than_built() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::Netlist;
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
    let unroutable = fabric.unroutable(&netlist);
    assert!(
        !unroutable.is_empty(),
        "a wire from one pad to another has to be routed, and this fabric cannot"
    );
}
