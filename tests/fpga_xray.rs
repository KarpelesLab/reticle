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

    // And the gap phase one could not close: which wire a bel pin
    // reaches. The pin *names* are UG474's and the wires are read off
    // `ppips_<type>.db`; `xray::sites` says which is which.
    let pins: Vec<(&str, &str)> = lut
        .pins
        .iter()
        .map(|(role, wire)| (role.as_str(), wire.name.as_str()))
        .collect();
    assert_eq!(pins.len(), 7, "six inputs and an output: {pins:?}");
    assert_eq!(pins[0].0, "i0");
    assert_eq!(pins[6].0, "o");
    // Every one of them names a wire the tile type really declares, or
    // the graph would grow a dangling edge.
    for (_, wire) in &pins {
        assert!(clb.has_wire(wire), "{wire} is not a wire of {}", clb.name);
    }
    // An IO bel reaches the fabric and deliberately does not reach the
    // pad: a package ball is not a wire a router can get to.
    let iob = fabric
        .arch
        .tile_types
        .iter()
        .find(|t| t.name.ends_with("IOB33"))
        .expect("the region holds an IO column");
    let buffer = iob
        .bels
        .iter()
        .find(|b| b.kind == "io")
        .expect("an IO tile has buffers");
    assert!(buffer.pin("din").is_some() && buffer.pin("dout").is_some());
    assert!(buffer.pin("pad").is_none());
    // And it carries a whole IO standard under the name of the
    // primitive that wants it.
    for primitive in ["IBUF", "OBUF"] {
        assert!(
            buffer.config.iter().any(|entry| matches!(
                entry,
                reticle::fpga::ConfigEntry::Cell { primitive: p, bits } if p == primitive && !bits.is_empty()
            )),
            "no bits for {primitive}"
        );
    }
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
/// It does **not** prove the design works. Nothing here has been sent
/// down a JTAG cable. What it adds to that is the comparison in
/// [`the_io_path_is_the_one_vivado_built`], which is a different
/// argument: not "this looks structurally right" but "Vivado's own
/// bitstream for this board sets these very features in these very
/// tiles".
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_milestone_design_reaches_a_bit_file() {
    let Some(root) = chipdb() else { return };
    let Some(features) = milestone(&root) else {
        return;
    };
    for (tile, feature) in &features {
        eprintln!("  {tile}.{feature}");
    }
    let has = |needle: &str| {
        features
            .iter()
            .any(|(t, f)| format!("{t}.{f}").contains(needle))
    };
    // The two switches and the LED, on the sites the package map names:
    // V17 is `IOB_X0Y11`, the lower half of `LIOB33_X0Y11`, which is the
    // half prjxray calls `IOB_Y1`.
    assert!(has("LIOB33_X0Y11.IOB_Y1.LVCMOS25_LVCMOS33_LVTTL.IN"), "sw0");
    assert!(has("LIOB33_X0Y11.IOB_Y0.LVCMOS25_LVCMOS33_LVTTL.IN"), "sw1");
    assert!(has("LIOB33_X0Y3.IOB_Y1.LVCMOS33_LVTTL.DRIVE"), "led");
    // The path through the IO logic, which is where a pad reaches the
    // interconnect.
    assert!(
        has("LIOI3_X0Y11.ILOGIC_Y1.ZINV_D"),
        "sw0 through the ilogic"
    );
    assert!(
        has("LIOI3_X0Y3.OLOGIC_Y1.OMUX.D1"),
        "led through the ologic"
    );
    // The interconnect ends of that path.
    assert!(
        has("INT_L_X0Y11.") && has(".LOGIC_OUTS_L18"),
        "sw0 into the fabric"
    );
    assert!(has("INT_L_X0Y3.IMUX_L34."), "the led out of the fabric");
    // And a lookup table with a truth table in it.
    assert!(
        features.iter().any(|(_, f)| f.contains("LUT.INIT[")),
        "the lut"
    );
}

/// Runs the milestone design and gives back what its bitstream says, in
/// the database's own feature names.
///
/// `None` when the crate was published without `examples/`.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn milestone(root: &str) -> Option<Vec<(String, String)>> {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::{Constraints, FpgaOptions, Netlist, bitstream, synthesize_for, target};
    use reticle::source::SourceMap;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("sw_led.v")),
        std::fs::read_to_string(dir.join("sw_led.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return None;
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
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
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

    // And it routes. Three signals: a switch each into the lookup table
    // and the lookup table out to the LED.
    let (routing, routing_report) = reticle::fpga::route(
        &netlist,
        &graph,
        &placement,
        &reticle::fpga::RouteOptions::default(),
    )
    .unwrap();
    assert_eq!(netlist.signals.len(), 3);
    assert_eq!(routing.routed(), 3, "{}", routing.to_text(&netlist, &graph));
    assert!(
        routing.verify(&netlist, &graph, &placement).is_empty(),
        "{:?}",
        routing.verify(&netlist, &graph, &placement)
    );
    eprintln!("{}", routing_report.to_text());

    let tiles = bitstream::generate(
        &design,
        top,
        &fabric.arch,
        &graph,
        &netlist,
        &placement,
        &routing,
    )
    .unwrap();
    // The LUT's INIT for `a ^ b` over six inputs is half the table, and
    // every one of those bits came from the database; the rest is the
    // route and the three buffers.
    assert!(tiles.ones() > 32, "{}", tiles.to_summary());

    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
    assert_eq!(frames.ones(), tiles.ones());
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
        "sw_led: {} byte(s), {} frame(s), {} configuration bit(s) — \
         PLACED AND ROUTED, NEVER LOADED INTO A PART",
        bytes.len(),
        STREAM_FRAMES,
        frames.ones()
    );

    // And now the part that is worth anything: what does it actually
    // say, in the database's own vocabulary?
    Some(db.decode(&DiskFiles, &frames).unwrap())
}

/// Decodes the Vivado bitstream `prjxray-db` ships for this very board.
///
/// `artix7/harness/basys3/swbut/design.bit` is a real, working
/// 2 192 111-byte bitstream, made by Vivado 2017.2 for a
/// `7a35tcpg236`, and it wires the Basys 3's sixteen switches to its
/// sixteen LEDs. Two of those switches are `V17` and `V16` and one of
/// those LEDs is `U16` — the very three pins the milestone design uses.
/// Reading it back through the same decoder gives an oracle: not a
/// story about what a bitstream ought to contain, but what one that
/// works does contain.
fn vivado(root: &str) -> Option<Vec<(String, String)>> {
    let bytes = harness(root)?;
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let bit = xc7::read_bit(&bytes).unwrap();
    // Every CRC in Vivado's own file checks against our calculation,
    // which is the phase-one invariant this leans on.
    for (expected, computed) in &bit.crc_checks {
        assert_eq!(expected, computed);
    }
    let frames = xc7::FrameData::from_stream(part.layout.clone(), bit.frames.clone()).unwrap();
    Some(db.decode(&DiskFiles, &frames).unwrap())
}

/// Every feature a decoding sets in one tile.
fn at<'a>(features: &'a [(String, String)], tile: &str) -> Vec<&'a str> {
    features
        .iter()
        .filter(|(t, _)| t == tile)
        .map(|(_, f)| f.as_str())
        .collect()
}

/// **The oracle.** What Reticle writes for the milestone's IO path,
/// against what Vivado wrote for the same three pins of the same board.
///
/// This is the strongest thing that can be said without a JTAG cable,
/// and it is not the same as "it works". It says: decode both
/// bitstreams into the database's own feature names, look at the tiles
/// the milestone's signals pass through, and the site configuration is
/// **identical** — the same IO standard features on the same halves of
/// the same IO blocks, the same input-logic and output-logic features
/// on the same halves of the same IO logic tiles. The routing differs,
/// because two routers chose two different legal paths across the same
/// interconnect; where it differs, the *ends* still agree, and the
/// assertions below say which is which.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_io_path_is_the_one_vivado_built() {
    let Some(root) = chipdb() else { return };
    let Some(theirs) = vivado(&root) else { return };
    let Some(ours) = milestone(&root) else { return };

    // ---- The IO blocks. Identical, feature for feature. ----
    //
    // `LIOB33_X0Y11` holds V17 and V16, and both designs drive both of
    // them as inputs, so the whole tile has to agree.
    assert_eq!(
        at(&ours, "LIOB33_X0Y11"),
        at(&theirs, "LIOB33_X0Y11"),
        "the two switches' input buffers"
    );
    // `LIOI3_X0Y11` is the input logic behind them, likewise both
    // halves in both designs.
    assert_eq!(
        at(&ours, "LIOI3_X0Y11"),
        at(&theirs, "LIOI3_X0Y11"),
        "the two switches' input logic"
    );
    // `LIOB33_X0Y3` holds U16 (LED 0) and U15 (LED 5). The harness
    // drives both; the milestone drives only LED 0, so ours is the
    // half of theirs that belongs to `IOB_Y1`, which is `IOB_X0Y3`,
    // which is U16.
    let theirs_led: Vec<&str> = at(&theirs, "LIOB33_X0Y3")
        .into_iter()
        .filter(|f| f.starts_with("IOB_Y1."))
        .collect();
    assert_eq!(
        at(&ours, "LIOB33_X0Y3"),
        theirs_led,
        "LED 0's output buffer"
    );
    let theirs_ologic: Vec<&str> = at(&theirs, "LIOI3_X0Y3")
        .into_iter()
        .filter(|f| f.starts_with("OLOGIC_Y1."))
        .collect();
    assert_eq!(
        at(&ours, "LIOI3_X0Y3"),
        theirs_ologic,
        "LED 0's output logic"
    );
    // And it is not vacuous: these are real features with real bits.
    assert_eq!(at(&ours, "LIOI3_X0Y3").len(), 3);
    assert_eq!(at(&ours, "LIOB33_X0Y3").len(), 3);

    // ---- The interconnect. A different legal route, same ends. ----
    //
    // A pad reaches the fabric on `LOGIC_OUTS_L18` of its own
    // interconnect row, and leaves it on `IMUX_L34`; which long line
    // carries it from there is the router's business and the two
    // routers disagree. So the *source* of the first pip and the
    // *destination* of the last are what must match, not the pip.
    let source_at = |features: &[(String, String)], tile: &str| -> Vec<String> {
        at(features, tile)
            .iter()
            .filter_map(|f| f.split_once('.').map(|(_, from)| from.to_owned()))
            .filter(|from| from.starts_with("LOGIC_OUTS"))
            .collect()
    };
    assert_eq!(
        source_at(&ours, "INT_L_X0Y11"),
        source_at(&theirs, "INT_L_X0Y11"),
        "V17 leaves the input logic on the same wire in both"
    );
    assert_eq!(source_at(&ours, "INT_L_X0Y11"), vec!["LOGIC_OUTS_L18"]);

    let dest_at = |features: &[(String, String)], tile: &str| -> Vec<String> {
        at(features, tile)
            .iter()
            .filter_map(|f| f.split_once('.').map(|(to, _)| to.to_owned()))
            .filter(|to| to.starts_with("IMUX"))
            .collect()
    };
    assert_eq!(
        dest_at(&ours, "INT_L_X0Y3"),
        dest_at(&theirs, "INT_L_X0Y3"),
        "U16 enters the output logic on the same wire in both"
    );
    assert_eq!(dest_at(&ours, "INT_L_X0Y3"), vec!["IMUX_L34"]);

    // Where they do differ, say so out loud rather than hiding it.
    for tile in ["INT_L_X0Y11", "INT_L_X0Y3"] {
        eprintln!("{tile}:");
        eprintln!("  vivado:  {:?}", at(&theirs, tile));
        eprintln!("  reticle: {:?}", at(&ours, tile));
    }
}

/// The two orientations this loader had to measure rather than assume,
/// re-derived from the database so the tables in `xray::sites` cannot
/// drift away from it.
///
/// 1. Which half of an IO tile prjxray's `_Y0` means. The harness
///    designs say: `design.json`'s `required_features` names the tiles
///    and `design.txt` names the pins, and every tile that configures
///    exactly one half agrees that `_Y0` is the *higher* site `Y`.
///    Here that is checked from the bitstream instead, on the one tile
///    of the Basys 3 harness that holds one input and one output.
/// 2. Which tile wires belong to which slice of a `CLBLL`. The
///    `ppips` files say: the interconnect index a slice pin uses is the
///    same in a `CLBLM`, where the site types settle which slice is
///    which.
#[test]
fn the_orientations_the_site_tables_assume_are_the_databases() {
    let Some(root) = chipdb() else { return };
    let Some(theirs) = vivado(&root) else { return };

    // `LIOB33_X0Y111` holds A18 (LED 16, an output, `IOB_X0Y111`) and
    // B18 (switch 16, an input, `IOB_X0Y112`). `IOB_X0Y112` is the
    // higher site Y, so if `_Y0` is the higher half then `IOB_Y0` is
    // the input and `IOB_Y1` the output.
    let tile = at(&theirs, "LIOB33_X0Y111");
    assert!(
        tile.iter()
            .any(|f| f.starts_with("IOB_Y0.") && f.ends_with(".IN")),
        "IOB_Y0 should be the input half: {tile:?}"
    );
    assert!(
        tile.iter()
            .any(|f| f.starts_with("IOB_Y1.") && f.contains(".DRIVE.")),
        "IOB_Y1 should be the output half: {tile:?}"
    );
    let ioi = at(&theirs, "LIOI3_X0Y111");
    assert!(ioi.contains(&"ILOGIC_Y0.ZINV_D"), "{ioi:?}");
    assert!(ioi.contains(&"OLOGIC_Y1.OMUX.D1"), "{ioi:?}");

    // And the slice wires, from `ppips_*.db` directly.
    let read = |name: &str| {
        std::fs::read_to_string(format!("{root}/artix7/ppips_{name}.db")).unwrap_or_default()
    };
    let feeds = |text: &str, wire: &str| -> Option<String> {
        text.lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                let full = w.next()?;
                let mut parts = full.split('.');
                let _ = parts.next()?;
                let to = parts.next()?;
                let from = parts.next()?;
                (to == wire).then(|| from.to_owned())
            })
            .next()
    };
    let clbll = read("clbll_l");
    let clblm = read("clblm_l");
    if clbll.is_empty() || clblm.is_empty() {
        eprintln!("skipped: the checkout has no `ppips_clb*.db`");
        return;
    }
    // In a `CLBLM` the `M` wires are the `SLICEM`'s, and the `SLICEM`
    // is the tile's X-index-0 site. Whatever interconnect index feeds
    // its `A1` must feed the X-index-0 slice of a `CLBLL` too.
    let m = feeds(&clblm, "CLBLM_M_A1").expect("CLBLM_M_A1");
    let ll = feeds(&clbll, "CLBLL_LL_A1").expect("CLBLL_LL_A1");
    assert_eq!(
        m.trim_start_matches("CLBLM"),
        ll.trim_start_matches("CLBLL"),
        "`CLBLL_LL` is the X-index-0 slice because it shares the \
         interconnect index of a CLBLM's SLICEM"
    );
    let l_m = feeds(&clblm, "CLBLM_L_A1").expect("CLBLM_L_A1");
    let l_l = feeds(&clbll, "CLBLL_L_A1").expect("CLBLL_L_A1");
    assert_eq!(
        l_m.trim_start_matches("CLBLM"),
        l_l.trim_start_matches("CLBLL")
    );
    assert_ne!(m, l_m);
}
