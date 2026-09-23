//! The Xilinx 7-series flow against a **real** fabric database.
//!
//! # Every test here skips without the database
//!
//! Project X-Ray's chip database is 35 MB of public-domain data that this
//! repository does not vendor and CI does not have. Point
//! `RETICLE_CHIPDB` at a `prjxray-db` checkout to run these; without it
//! each test prints what it wanted and returns, exactly as
//! `tests/sim_fst.rs` does for gtkwave and `tests/asic_formats.rs` does
//! for a PDK Liberty file. **A missing database must never fail the
//! build.**
//!
//! ```text
//! git clone --filter=blob:none --no-checkout --depth 1 \
//!     https://github.com/f4pga/prjxray-db.git
//! cd prjxray-db && git sparse-checkout set --no-cone \
//!     '/artix7/*.db' '/artix7/*.csv' '/artix7/xc7a35tcpg236-1/' \
//!     '/artix7/xc7a50t/' '/artix7/mapping/' '/artix7/harness/'
//! export RETICLE_CHIPDB=$PWD
//! ```
//!
//! # NOTHING CHECKED HERE HAS BEEN LOADED INTO A PART
//!
//! What these tests establish is structural: that the container is the
//! one UG470 describes, that its frame addresses and frame count are the
//! ones `part.json` states, that both CRCs are right by an independent
//! calculation, that the writer's output reads back identically, and
//! that all of that agrees with a bitstream Vivado made for this very
//! part. None of it says a design built this way configures anything.
//! See `docs/fpga-xray.md`.

#![cfg(feature = "fpga")]

use std::path::Path;

use reticle::fpga::xc7::{
    self, BitHeader, Command, FrameAddress, Packet, Register, WORDS_PER_FRAME,
};
use reticle::fpga::xray::{GridRegion, XrayDatabase, XrayError, XrayOptions};
use reticle::ir::memfile::FileProvider;

/// The Basys 3's part, and the IDCODE `part.json` gives it.
const DEVICE: &str = "xc7a35t-cpg236";
const IDCODE: u32 = 0x0362_d093;
/// The frames `part.json` describes, before the pad frames.
const DATA_FRAMES: usize = 5408;
/// The frames the stream carries, pad frames included.
const STREAM_FRAMES: usize = 5420;

/// Reads files from the filesystem, which is what the CLI does and what
/// the library never does.
struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let Ok(root) = std::env::var("RETICLE_CHIPDB") else {
        eprintln!(
            "skipped: needs a Project X-Ray database; set RETICLE_CHIPDB to a prjxray-db checkout"
        );
        return None;
    };
    let probe = format!("{root}/artix7/xc7a50t/tilegrid.json");
    if !Path::new(&probe).exists() {
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{probe}` is not there");
        return None;
    }
    Some(root)
}

/// The reference bitstream Vivado produced for a Basys 3, if the
/// checkout has `harness/`.
fn harness(root: &str) -> Option<Vec<u8>> {
    let path = format!("{root}/artix7/harness/basys3/swbut/design.bit");
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipped: the checkout has no `{path}`");
            None
        }
    }
}

/// A region big enough to hold the tiny design and small enough to load
/// in a second: the interconnect and logic columns next to the left-hand
/// IO, in `tilegrid.json`'s own grid coordinates.
fn small_region() -> GridRegion {
    GridRegion::new(0, 100, 14, 112)
}

#[test]
fn the_part_is_the_one_part_json_describes() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    assert_eq!(db.family(), "artix7");
    assert_eq!(db.part_directory(), "xc7a35tcpg236-1");
    // The one fact that makes this work at all: an xc7a35t is an
    // xc7a50t die.
    assert_eq!(db.fabric(), "xc7a50t");

    let part = db.part(&DiskFiles).unwrap();
    assert_eq!(part.idcode, IDCODE);
    assert_eq!(part.layout.data_frames(), DATA_FRAMES);
    assert_eq!(part.layout.frames(), STREAM_FRAMES);
    assert_eq!(part.layout.words(), STREAM_FRAMES * WORDS_PER_FRAME);

    // The first frame of the stream is the one a bitstream sets FAR to.
    assert_eq!(part.layout.order()[0], Some(FrameAddress::default()));
    // And every row ends in pad frames.
    assert!(part.layout.order()[1532].is_none());
}

#[test]
fn the_flow_refuses_a_database_for_another_part() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(small_region()))
        .unwrap();
    fabric.check_idcode(IDCODE).unwrap();
    let wrong = fabric.check_idcode(0x0362_c093).unwrap_err();
    assert!(wrong.to_string().contains("0x0362d093"), "{wrong}");
    assert!(matches!(wrong, xc7::Xc7Error::IdcodeMismatch { .. }));

    // A device the database has no directory for is refused by name.
    let err =
        XrayDatabase::open(&DiskFiles, &root, "ice40-hx1k-tq144", &XrayOptions::new()).unwrap_err();
    assert!(matches!(err, XrayError::WrongPart { .. }), "{err}");
}

#[test]
fn the_whole_die_is_measured_and_refused_with_numbers() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    // No region is the whole `xc7a50t`, which this crate's routing graph
    // cannot hold. The loader counts before it allocates, so this is a
    // measurement rather than a crash.
    let err = db.load(&DiskFiles, &XrayOptions::new()).unwrap_err();
    let XrayError::TooLarge { pips, tiles, limit } = err else {
        panic!("the whole die loaded, which this crate's graph cannot hold: {err}");
    };
    eprintln!("whole die: {tiles} tile(s), {pips} pip(s), limit {limit}");
    assert_eq!(tiles, 18_055);
    assert!(pips > 20_000_000, "{pips}");
}

#[test]
fn a_region_loads_and_reports_what_it_cost() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(small_region()))
        .unwrap();
    let stats = &fabric.stats;
    eprintln!("{}", stats.to_text());

    // The whole fabric is measured whatever the region.
    assert_eq!(stats.tiles, 18_055);
    assert_eq!(stats.tile_types, 112);
    assert_eq!(stats.frames, STREAM_FRAMES);
    assert!(stats.features_die > 23_000_000, "{}", stats.features_die);
    assert_eq!(stats.nodes_die, Some(7_857_396));
    // The frame map is the whole part, so the bitstream is a whole-part
    // bitstream however small the region.
    assert!(stats.mapped_tiles > 10_000, "{}", stats.mapped_tiles);

    // The region itself is small.
    assert!(stats.tiles_loaded <= small_region().area());
    assert!(stats.pips > 0 && stats.wires > 0);
    assert!(stats.joins > 0, "tileconn produced no joins");

    // The architecture is a real one: the grid is the die's.
    assert_eq!((fabric.arch.width, fabric.arch.height), (115, 157));
    assert_eq!(fabric.arch.family, "xc7");
    assert!(fabric.arch.serves(DEVICE));
    // And the interconnect tiles are there with their real names.
    assert!(
        fabric
            .arch
            .tile_types
            .iter()
            .any(|t| t.name == "INT_L" && t.bit_rows == 28 && t.bit_cols == 64),
        "{:?}",
        fabric
            .arch
            .tile_types
            .iter()
            .map(|t| &t.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn real_slices_become_real_bels() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(small_region()))
        .unwrap();
    let graph = fabric.arch.build_graph();
    let counts = graph.site_counts();
    eprintln!("sites: {counts:?}");
    for kind in ["lut", "ff"] {
        assert!(
            counts.iter().any(|(k, n)| k == kind && *n > 0),
            "no {kind} sites in {counts:?}"
        );
    }

    // A LUT bel carries its truth table as parameter bits, which is how
    // `INIT` reaches the bitstream with no Rust knowing what a LUT is.
    let clb = fabric
        .arch
        .tile_types
        .iter()
        .find(|t| t.name.starts_with("CLBL"))
        .expect("the region holds a CLB column");
    let lut = clb
        .bels
        .iter()
        .find(|b| b.kind == "lut")
        .expect("a CLB has LUTs");
    let init_bits = lut
        .config
        .iter()
        .filter(|entry| {
            matches!(entry, reticle::fpga::ConfigEntry::Param { name, .. } if name == "INIT")
        })
        .count();
    assert_eq!(init_bits, 64, "a 7-series LUT has a 64-bit INIT");

    // And the gap that matters: the database does not say which wire a
    // bel pin reaches, so the bels have no pins. When that stops being
    // true this assertion is what will notice.
    assert!(
        lut.pins.is_empty(),
        "bel pins came from somewhere: {:?}",
        lut.pins
    );
}

#[test]
fn every_tile_frame_address_is_inside_the_part_layout() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let tiles = db.tiles(&DiskFiles).unwrap();
    let mut checked = 0usize;
    for tile in &tiles {
        for (_, bits) in &tile.bits {
            for frame in 0..bits.frames {
                let address = FrameAddress::from_u32(bits.baseaddr + frame);
                assert!(
                    part.layout.contains(address),
                    "tile {} wants {address}, which `part.json` does not describe",
                    tile.name
                );
                checked += 1;
            }
        }
    }
    eprintln!("{checked} tile frame address(es), all inside the part layout");
    assert!(checked > 100_000, "{checked}");
}

#[test]
fn the_container_matches_the_vivado_harness() {
    let Some(root) = chipdb() else { return };
    let Some(bytes) = harness(&root) else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();

    // Reticle's own reader on Vivado's own output. If the CRC is wrong
    // this throws; it is the strongest single check available without a
    // cable.
    let theirs = xc7::read_bit(&bytes).unwrap();
    assert_eq!(theirs.header.part, "7a35tcpg236");
    assert_eq!(theirs.idcode, Some(IDCODE));
    assert_eq!(theirs.start_address, Some(FrameAddress::default()));
    assert_eq!(theirs.frame_count(), STREAM_FRAMES);
    assert_eq!(theirs.crc_checks.len(), 2);
    for (expected, computed) in &theirs.crc_checks {
        assert_eq!(expected, computed, "Vivado's CRC and ours disagree");
    }
    assert_eq!(
        theirs.commands,
        vec![
            Command::Null,
            Command::Rcrc,
            Command::Switch,
            Command::Wcfg,
            Command::Grestore,
            Command::Lfrm,
            Command::Start,
            Command::Desync,
        ]
    );

    // Now ours, with the same layout and an empty design.
    let data = xc7::FrameData::empty(part.layout.clone());
    let ours = xc7::write_bit(&BitHeader::new("empty", "7a35tcpg236"), &part, &data).unwrap();
    let mine = xc7::read_bit(&ours).unwrap();
    assert_eq!(mine.frame_count(), theirs.frame_count());
    assert_eq!(mine.idcode, theirs.idcode);
    assert_eq!(mine.commands, theirs.commands);
    assert_eq!(mine.start_address, theirs.start_address);

    // The packet sequences are the same shape, register for register,
    // once the frame payload is set aside.
    let shape = |packets: &[Packet]| -> Vec<(u32, usize)> {
        packets
            .iter()
            .filter_map(|p| match p {
                Packet::Write { register, words } => Some((*register, words.len())),
                _ => None,
            })
            .collect()
    };
    assert_eq!(shape(&mine.packets), shape(&theirs.packets));

    // Everything before the sync word is the same too.
    let sync = bytes
        .windows(4)
        .position(|w| w == xc7::SYNC_WORD.to_be_bytes())
        .unwrap();
    let ours_sync = ours
        .windows(4)
        .position(|w| w == xc7::SYNC_WORD.to_be_bytes())
        .unwrap();
    assert_eq!(&bytes[sync - 40..sync], &ours[ours_sync - 40..ours_sync]);

    // What differs, and it is only this: our frames are empty and
    // Vivado's are a design. Say so with a number rather than by
    // implication.
    let theirs_ones: u32 = theirs.frames.iter().map(|w| w.count_ones()).sum();
    eprintln!(
        "harness: {} frame word(s), {theirs_ones} bit(s) set; ours: 0 set",
        theirs.frames.len()
    );
    assert!(theirs_ones > 0);
    assert!(mine.frames.iter().all(|w| *w == 0));
    assert_eq!(Register::from_u32(2), Some(Register::Fdri));
}

#[test]
fn a_design_reaches_a_real_bitstream() {
    use reticle::fpga::bitstream::{Bitstream, BitstreamFormat};
    use reticle::fpga::{ConfigBit, arch::ConfigEntry};

    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(small_region()))
        .unwrap();
    fabric.check_idcode(IDCODE).unwrap();

    // The smallest thing that proves the chain: put a LUT truth table
    // into a real `SLICEL`, through the ordinary tile bitmap the
    // architecture hands out, and follow it into the frames.
    let mut bitstream = Bitstream::empty(BitstreamFormat::from_arch(&fabric.arch));
    let (index, clb) = fabric
        .arch
        .tile_types
        .iter()
        .enumerate()
        .find(|(_, t)| t.name.starts_with("CLBL"))
        .expect("a CLB column in the region");
    let lut = clb.bels.iter().find(|b| b.kind == "lut").unwrap();
    // `INIT` bit 0, which is what a LUT holding a constant one sets.
    let init0 = lut
        .config
        .iter()
        .find_map(|entry| match entry {
            ConfigEntry::Param { name, index, at } if name == "INIT" && *index == 0 => Some(*at),
            _ => None,
        })
        .expect("a LUT has an INIT bit 0");
    let tile = (0..fabric.arch.height)
        .flat_map(|y| (0..fabric.arch.width).map(move |x| (x, y)))
        .find(|(x, y)| fabric.arch.tile_index_at(*x, *y) == Some(index))
        .expect("a tile of that type");
    bitstream.set(tile, init0).unwrap();
    assert_eq!(bitstream.ones(), 1);

    // That one tile bit becomes one frame bit, at a frame address this
    // part really has.
    let data = xc7::frames_from_bitstream(&fabric.part, &bitstream, &fabric.frames).unwrap();
    assert_eq!(data.ones(), 1);
    let used = data.used_frames();
    assert_eq!(used.len(), 1);
    assert!(fabric.part.layout.contains(used[0]));
    eprintln!("one LUT INIT bit lands in frame {}", used[0]);

    // And the frames become a `.bit` that reads back to the same frames.
    let header = BitHeader::new("lut_init;UserID=0XFFFFFFFF", "7a35tcpg236");
    let bytes = xc7::write_bit(&header, &fabric.part, &data).unwrap();
    let back = xc7::read_bit(&bytes).unwrap();
    assert_eq!(back.idcode, Some(IDCODE));
    assert_eq!(back.frames, data.words());
    assert_eq!(back.frame_count(), STREAM_FRAMES);
    for (expected, computed) in &back.crc_checks {
        assert_eq!(expected, computed);
    }
    // Deterministic: two runs, the same bytes.
    assert_eq!(xc7::write_bit(&header, &fabric.part, &data).unwrap(), bytes);
    // And the size is the part's, not the design's.
    assert!(
        bytes.len() > STREAM_FRAMES * WORDS_PER_FRAME * 4,
        "{}",
        bytes.len()
    );
    eprintln!("wrote a {} byte .bit for {DEVICE}", bytes.len());

    // A bit that no window claims is an error, not a silent drop.
    let mut stray = Bitstream::empty(BitstreamFormat::from_arch(&fabric.arch));
    if stray.set(tile, ConfigBit::new(0, 0)).is_ok() {
        assert!(
            xc7::frames_from_bitstream(&fabric.part, &stray, &reticle::fpga::FrameMap::new())
                .is_err()
        );
    }
}

/// The milestone design, all the way from Verilog to a `.bit`: two slide
/// switches through one LUT to one LED on a Basys 3.
///
/// # What this proves, and what it does not
///
/// It proves the chain runs: source text, synthesis, mapping onto this
/// part's own primitives, placement onto real sites read from
/// `tilegrid.json`, the LUT's truth table through the architecture's
/// `ConfigEntry::Param` entries into tile bits, those tile bits into
/// frames at addresses `part.json` describes, and those frames into a
/// UG470 container whose CRCs check.
///
/// It does not prove the design works. It is **not routed**: the
/// database gives no bel pins, so no signal has a path (see
/// `docs/fpga-xray.md`). Flipping a switch on a board loaded with this
/// would do nothing at all.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_milestone_design_reaches_a_bit_file() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::{Constraints, FpgaOptions, Netlist, bitstream, synthesize_for, target};
    use reticle::source::SourceMap;

    let Some(root) = chipdb() else { return };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("sw_led.v")),
        std::fs::read_to_string(dir.join("sw_led.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };

    let mut map = SourceMap::new();
    let source = map.add("sw_led.v", &verilog).unwrap();
    let rcf_file = map.add("sw_led.rcf", &rcf).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::SystemVerilog,
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

    let device = target(DEVICE).unwrap();
    assert_eq!(
        device.idcode,
        Some(IDCODE),
        "the device file states the IDCODE"
    );
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

    // The fabric around the pins the constraints name, which is how a
    // flow picks a region without a human choosing coordinates.
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    assert_eq!(pins.len(), 3);
    let region = db
        .region_for_pins(&DiskFiles, &pins, 12)
        .unwrap()
        .expect("V17, V16 and U16 are all sites of this package");
    let mut options = XrayOptions::new();
    options.region = Some(region);
    let fabric = db.load(&DiskFiles, &options).unwrap();

    // The refusal that matters: a database for another die never gets as
    // far as writing a file.
    fabric.check_idcode(IDCODE).unwrap();
    assert!(fabric.check_idcode(0x1234_5678).is_err());

    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    // Two input buffers, an output buffer and the LUT that does the work.
    assert_eq!(netlist.instances.len(), 4);
    let (placement, report) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    // Every constrained pin is held at the site the package map gives
    // it, which is the check that the pinmap really names architecture
    // sites and not `tilegrid.json`'s own site names.
    assert_eq!(report.fixed, 3, "the three pins are not held");
    for index in 0..netlist.instances.len() {
        assert!(
            placement.site_of(index).is_some(),
            "instance {index} unplaced"
        );
    }

    let tiles = bitstream::generate(
        &design,
        top,
        &fabric.arch,
        &graph,
        &netlist,
        &placement,
        &reticle::fpga::Routing::new(netlist.signals.len()),
    )
    .unwrap();
    // The LUT's INIT for `a ^ b` over six inputs is half the table, and
    // every one of those bits came from the database.
    assert_eq!(tiles.ones(), 32, "{}", tiles.to_summary());

    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
    assert_eq!(frames.ones(), 32);
    let used = frames.used_frames();
    assert!(!used.is_empty());
    for address in &used {
        assert!(fabric.part.layout.contains(*address));
    }
    eprintln!("the LUT lands in {} frame(s)", used.len());

    let header = BitHeader::new("sw_led;UserID=0XFFFFFFFF;Version=reticle", "7a35tcpg236");
    let bytes = xc7::write_bit(&header, &fabric.part, &frames).unwrap();
    let back = xc7::read_bit(&bytes).unwrap();
    assert_eq!(back.idcode, Some(IDCODE));
    assert_eq!(back.frame_count(), STREAM_FRAMES);
    assert_eq!(back.frames, frames.words());
    assert_eq!(back.start_address, Some(FrameAddress::default()));
    for (expected, computed) in &back.crc_checks {
        assert_eq!(expected, computed);
    }
    eprintln!(
        "sw_led: {} byte(s), {} frame(s), 32 configuration bit(s); NOT ROUTED, CONFIGURES NOTHING",
        bytes.len(),
        STREAM_FRAMES
    );
}
