//! The Gowin flow against a **real** Project Apicula chip database.
//!
//! # Every test here skips without the database
//!
//! Apicula's chip database is not in this repository and CI does not have
//! it. It is prebuilt, though — no vendor IDE is needed — so obtaining it
//! is one command:
//!
//! ```text
//! pip download --no-deps --no-binary :all: apycula==0.33 -d /tmp/apycula
//! tar -C /tmp/apycula -xzf /tmp/apycula/apycula-0.33.tar.gz
//! export RETICLE_GOWINDB=/tmp/apycula/apycula-0.33/apycula
//! ```
//!
//! That directory holds one `<device>.msgpack.xz` per die; these tests
//! want `GW2A-18.msgpack.xz`, 375 KB. Without `RETICLE_GOWINDB` each test
//! prints what it wanted and returns, exactly as `tests/fpga_xray.rs` does
//! for `prjxray-db`. **A missing database must never fail the build.**
//!
//! One test wants a second thing, a reference `.fs` bitstream produced by
//! Apicula's own packer, and skips separately without it:
//!
//! ```text
//! pip install apycula==0.33
//! gowin_pack -d GW2A-18 -o /tmp/empty.fs empty.json   # see docs/fpga-gowin.md
//! export RETICLE_GOWIN_FS=/tmp/empty.fs
//! ```
//!
//! # What these tests can and cannot reach
//!
//! They establish that the loader reads the real database and that the
//! container agrees with a file the reference packer wrote. **Nothing
//! here has been loaded into a part**, and no test can reach that. See
//! `docs/fpga-gowin.md`.

#![cfg(feature = "apicula")]

use std::collections::BTreeSet;

use reticle::fpga::apicula::{
    ApiculaDatabase, ApiculaError, ApiculaOptions, InterTileWire, fuses_for,
};
use reticle::fpga::gowin::{CMD_LOAD_CONFIG, FsStream, crc16_arc, written_width};
use reticle::fpga::xray::GridRegion;
use reticle::fpga::{BelRole, PinKind, PllDividerRole, PllFeedback};
use reticle::ir::memfile::FileProvider;

/// The die the Sipeed Tang Primer 20K carries, as *Apicula's own*
/// `examples/Makefile` names it: `gowin_pack -d GW2A-18`.
const DEVICE: &str = "GW2A-18";
/// And the part number its nextpnr invocation passes.
const PART: &str = "GW2A-LV18PG256C8/I7";
/// The JTAG IDCODE `reticle program --probe` reads from that board, which
/// the database's own `cmd_hdr` also carries.
const IDCODE: u32 = 0x0000_081b;

/// Reads files from the filesystem, which is what a caller does and what
/// the library never does. The database is binary, so `read_bytes` is the
/// method that matters.
struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn read_bytes(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }
}

/// The database directory, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let Ok(root) = std::env::var("RETICLE_GOWINDB") else {
        eprintln!(
            "skipped: needs a Project Apicula chip database; set RETICLE_GOWINDB to a \
             directory holding {DEVICE}.msgpack.xz (see this file's header)"
        );
        return None;
    };
    let probe = format!("{root}/{DEVICE}.msgpack.xz");
    if std::fs::metadata(&probe).is_err() {
        eprintln!("skipped: RETICLE_GOWINDB is `{root}` but `{probe}` is not there");
        return None;
    }
    Some(root)
}

/// The database, opened, or `None` having said why not.
fn open() -> Option<ApiculaDatabase> {
    let root = chipdb()?;
    match ApiculaDatabase::open(&root, DEVICE, &DiskFiles) {
        Ok(db) => Some(db),
        Err(err) => panic!("the database is there and would not open: {err}"),
    }
}

/// The reference `.fs`, or `None` having said why not.
fn reference_fs() -> Option<String> {
    let Ok(path) = std::env::var("RETICLE_GOWIN_FS") else {
        eprintln!(
            "skipped: needs a reference bitstream; set RETICLE_GOWIN_FS to a `.fs` that \
             `gowin_pack -d {DEVICE}` produced (see docs/fpga-gowin.md)"
        );
        return None;
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) => {
            eprintln!("skipped: RETICLE_GOWIN_FS is `{path}` and would not read: {err}");
            None
        }
    }
}

#[test]
fn the_database_is_one_messagepack_map_of_thirty_eight_fields() {
    let Some(db) = open() else { return };
    let keys = db.top_level_keys();
    eprintln!("{} top-level key(s):", keys.len());
    for chunk in keys.chunks(6) {
        eprintln!("  {}", chunk.join(", "));
    }
    // The shape `docs/fpga-gowin.md` documents: a map whose first field is
    // the grid, because that is the order `apycula/chipdb.py`'s `Device`
    // declares its fields in.
    assert_eq!(keys.first(), Some(&"grid"));
    assert_eq!(keys.len(), 38, "{keys:?}");
    // The ten fields the loader reads, plus `pin_bank`, which is where
    // `devices/gowin.dev`'s bank clauses came from.
    for wanted in [
        "grid",
        "tiles",
        "tile_types",
        "corner_tiles_io",
        "packages",
        "pinout",
        "pin_bank",
        "cmd_hdr",
        "cmd_ftr",
        "const",
        "nodes",
    ] {
        assert!(keys.contains(&wanted), "no `{wanted}` field");
    }
    // And the ones it deliberately does not: the attribute encoding, whose
    // names are not in the file, and the timing model.
    for present in ["logicinfo", "shortval", "longval", "longfuses", "timing"] {
        assert!(keys.contains(&present), "no `{present}` field");
    }
}

#[test]
fn the_part_this_board_carries_is_one_of_the_databases_own() {
    let Some(db) = open() else { return };
    let packages = db.packages();
    eprintln!("{} part number(s)", packages.len());
    let mine = packages
        .iter()
        .find(|p| p.part == PART)
        .unwrap_or_else(|| panic!("no `{PART}`; the database has {packages:#?}"));
    eprintln!("{PART} -> {mine:?}");
    assert_eq!(mine.device, DEVICE);
    assert_eq!(mine.package, "PBGA256");
    assert_eq!(mine.speed, "C8/I7");
    // The IDCODE the board answers with. It does **not** identify the
    // database: a GW2A-18, a GW2A-18C and a GW2AR-18C all carry this one,
    // which is why the device is named and not guessed.
    assert_eq!(db.idcode(), Some(IDCODE));
}

#[test]
fn the_fabric_measures_itself_and_the_numbers_are_the_documented_ones() {
    let Some(db) = open() else { return };
    let fabric = db
        .load(&ApiculaOptions::new().with_part(PART))
        .expect("the whole die loads");
    eprintln!("{}", fabric.stats.to_text());
    let stats = &fabric.stats;

    // The grid and the bitmap.
    assert_eq!((stats.rows, stats.cols), (55, 56));
    assert_eq!(stats.tiles, 3080);
    assert_eq!(stats.tile_types, 74);
    assert_eq!((stats.bitmap_rows, stats.bitmap_cols), (1342, 3376));
    // 3376 bits is 422 whole bytes, which is why a GW2A-18 row needs no
    // padding and a GW1N-9's does.
    assert_eq!(stats.bitmap_cols % 8, 0);

    // The interconnect, over the die.
    assert_eq!(stats.pips_die, 8_095_135);
    assert_eq!(stats.clock_pips_die, 5198);

    // The logic, over the die. These are the datasheet's numbers for a
    // GW2A-18, arrived at by counting the database's bels rather than by
    // being told: 20 736 LUT4, 15 552 registers, 46 x 18 kbit of block RAM
    // (828 kbit), 48 18x18 multipliers in 12 DSP blocks, 4 PLLs.
    let bel = |name: &str| stats.bels_die.get(name).copied().unwrap_or(0);
    let luts: usize = (0..8).map(|i| bel(&format!("LUT{i}"))).sum();
    let ffs: usize = (0..6).map(|i| bel(&format!("DFF{i}"))).sum();
    let alus: usize = (0..6).map(|i| bel(&format!("ALU{i}"))).sum();
    eprintln!("LUT4 {luts}, DFF {ffs}, ALU {alus}, RAM16 {}", bel("RAM16"));
    assert_eq!(luts, 20_736);
    assert_eq!(ffs, 15_552);
    // Six per logic tile, one for each of the six slices that has a
    // flip-flop; the `ALU54D` bels of the DSP columns are a different
    // thing with a similar name and are not counted here.
    assert_eq!(alus, 15_552);
    assert_eq!(bel("ALU54D0") + bel("ALU54D1"), 24);
    assert_eq!(bel("RAM16"), 648);
    assert_eq!(bel("IOBA") + bel("IOBB"), 384);
    assert_eq!(bel("BSRAM"), 46);
    assert_eq!(bel("DSP"), 12);
    assert_eq!(bel("MULT18X1800") * 4, 48);
    assert_eq!(bel("RPLLA"), 4);

    // What is counted and not loaded, which is the honest half.
    assert_eq!(stats.nodes, 14_748);
    assert_eq!(stats.node_members, 89_583);
    assert!(stats.hclk_pips > 0);
    assert_eq!(stats.const_bits, 462);
    assert_eq!(stats.idcode, Some(IDCODE));

    // The edge wrap this loader leaves unconnected rather than guessing.
    eprintln!(
        "{} inter-tile reference(s) left unconnected at the die edge",
        stats.edge_wraps
    );
    assert_eq!(stats.edge_wraps, 16_872);

    // And every configuration entry is a lookup table bit, because the
    // attribute names nothing else needs are not in the database.
    assert_eq!(stats.config_entries, luts * 16);
}

#[test]
fn the_whole_die_is_a_graph_this_crate_can_hold() {
    let Some(db) = open() else { return };
    let fabric = db
        .load(&ApiculaOptions::new())
        .expect("the whole die loads");
    let graph = fabric.arch.build_graph();
    let heap = graph.heap_bytes();
    eprintln!(
        "whole die: {} node(s), {} edge(s) kept, {} dropped, {} distinct bit pattern(s), \
         {} MiB on the heap",
        graph.nodes.len(),
        graph.pips.len(),
        graph.dangling,
        graph.bit_patterns(),
        heap / (1024 * 1024)
    );
    // The point of interning: the die's eight million pips share a few
    // thousand distinct bit patterns, so a pip costs bytes and not a
    // vector. Bounded as a *ratio* rather than as a time or a byte count,
    // since both of those depend on the machine.
    assert!(
        graph.bit_patterns() * 100 < graph.pips.len(),
        "{} patterns for {} pips is not the saving interning is for",
        graph.bit_patterns(),
        graph.pips.len()
    );
    // Every pip the architecture declares is either kept or dropped.
    // `dangling` counts dropped bel *pins* too, so this equality also says
    // that over the whole die not one bel pin named a wire its tile has
    // not got — which is worth knowing, since the bel pin names come from
    // the database's own portmaps and nothing checks them otherwise.
    assert_eq!(graph.pips.len() + graph.dangling, fabric.stats.pips);
    assert!(
        graph.dangling >= fabric.stats.edge_wraps,
        "{} dropped, {} wrapped",
        graph.dangling,
        fabric.stats.edge_wraps
    );
    // A pip a wrap leaves unconnected is dropped, so a die with the whole
    // grid loaded keeps almost all of them: the losses are the edge.
    assert!(
        graph.pips.len() * 100 > fabric.stats.pips * 90,
        "{} of {} edges kept",
        graph.pips.len(),
        fabric.stats.pips
    );
}

#[test]
fn an_inter_tile_wire_is_one_node_the_whole_length_of_its_span() {
    let Some(db) = open() else { return };
    // A region in the middle of the die, well away from any edge, so no
    // wrap can muddy the result.
    let fabric = db
        .load(&ApiculaOptions::new().with_region(GridRegion::new(20, 20, 30, 30)))
        .expect("an interior region loads");
    let arch = &fabric.arch;
    eprintln!("{}", fabric.stats.to_text());

    // No tile type declares a wire whose name carries a segment: they are
    // all declared under their root, which is the whole point.
    for tile_type in &arch.tile_types {
        for wire in &tile_type.wires {
            if let Some(inter) = InterTileWire::parse(&wire.name) {
                panic!(
                    "{} declares `{}`, which is segment {} of {}",
                    tile_type.name,
                    wire.name,
                    inter.segment,
                    inter.root_name()
                );
            }
            // A three-character inter-tile root has the span its length
            // digit gives it.
            if wire.name.len() == 3 {
                let with_segment = format!("{}0", wire.name);
                if let Some(inter) = InterTileWire::parse(&with_segment) {
                    assert_eq!((wire.dx, wire.dy), inter.reach(), "{}'s span", wire.name);
                }
            }
        }
    }

    // And a pip that names a later segment refers to the root at an
    // offset, so the graph joins the two tiles with no extra edge.
    let mut offsets = BTreeSet::new();
    for tile_type in &arch.tile_types {
        for pip in &tile_type.pips {
            for end in [&pip.from, &pip.to] {
                if (end.dx, end.dy) != (0, 0) {
                    offsets.insert((end.dx, end.dy));
                }
            }
        }
    }
    eprintln!(
        "{} distinct wire-reference offset(s): {offsets:?}",
        offsets.len()
    );
    // One, two, four and eight tiles in each of the four directions.
    assert!(offsets.contains(&(-1, 0)));
    assert!(offsets.contains(&(1, 0)));
    assert!(offsets.contains(&(0, -1)));
    assert!(offsets.contains(&(0, 1)));
    assert!(offsets.contains(&(-8, 0)) || offsets.contains(&(8, 0)));

    let graph = arch.build_graph();
    eprintln!(
        "interior region: {} node(s), {} edge(s), {} dropped at the region's own border",
        graph.nodes.len(),
        graph.pips.len(),
        graph.dangling
    );
    assert!(graph.pips.len() > 100_000);
}

#[test]
fn the_pins_apiculas_own_board_file_names_resolve_to_sites() {
    let Some(db) = open() else { return };
    let fabric = db
        .load(&ApiculaOptions::new().with_part(PART))
        .expect("the whole die loads");
    eprintln!(
        "{} of {} PBGA256 ball(s) mapped",
        fabric.stats.pins_mapped,
        fabric.stats.pins_mapped + fabric.stats.pins_unmapped
    );
    assert_eq!(fabric.stats.pins_mapped + fabric.stats.pins_unmapped, 207);
    // Project Apicula's own `examples/primer20k.cst`, which its `make
    // primer20k` target builds every one of its example designs against.
    // These are the dock's six user LEDs, its button, and the pin its
    // `blinky` takes a clock from.
    for (pin, what) in [
        ("C13", "led[0]"),
        ("A13", "led[1]"),
        ("N16", "led[2]"),
        ("N14", "led[3]"),
        ("L14", "led[4]"),
        ("L16", "led[5]"),
        ("T3", "key_i"),
        ("H11", "clk"),
        ("A15", "TXD"),
        ("D14", "RXD"),
    ] {
        let site = fabric
            .arch
            .site_of_pin(pin)
            .unwrap_or_else(|| panic!("ball {pin} ({what}) reaches no site"));
        eprintln!("  {pin:4} {what:8} -> {site}");
        assert!(site.contains("/IOB"), "{pin} -> {site}");
    }
    // Every mapped ball reaches a site the architecture really has.
    let graph = fabric.arch.build_graph();
    let mut missing = Vec::new();
    for (pin, site) in &fabric.arch.pinmap {
        if !graph.sites.iter().any(|s| &s.name == site) {
            missing.push((pin.clone(), site.clone()));
        }
    }
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn the_blank_stream_is_the_reference_files_envelope_byte_for_byte() {
    let Some(db) = open() else { return };
    let Some(reference) = reference_fs() else {
        return;
    };
    let fabric = db
        .load(&ApiculaOptions::new())
        .expect("the whole die loads");
    let ours = fabric.blank_stream().expect("a blank stream");
    let cols = fabric.stats.bitmap_cols;

    // What the reference file's own lines say about its geometry, read
    // without the database's help.
    assert_eq!(written_width(&reference).unwrap(), cols as usize);
    let theirs = FsStream::parse(&reference, cols).expect("the reference file reads");
    eprintln!(
        "reference: {} header line(s), {} row(s) of {} bit(s), {} footer line(s), {} bit(s) set",
        theirs.header.len(),
        theirs.bitmap.rows(),
        theirs.bitmap.cols(),
        theirs.footer.len(),
        theirs.bitmap.count_ones()
    );

    // The commands. Every header line is identical, and so is every footer
    // line but the USERCODE, which `gowin_pack` fills in and this does not.
    assert_eq!(ours.header, theirs.header);
    assert_eq!(ours.header.len(), 10);
    assert_eq!(
        ours.header.last().map(|l| l[0]),
        Some(CMD_LOAD_CONFIG),
        "the last header line is the load-configuration command"
    );
    // Which carries the row count, and it is the die bitmap's height.
    let rows = u16::from_be_bytes([ours.header[9][2], ours.header[9][3]]);
    assert_eq!(u32::from(rows), fabric.stats.bitmap_rows);
    assert_eq!(theirs.bitmap.rows(), fabric.stats.bitmap_rows);
    assert_eq!(theirs.bitmap.cols(), cols);
    for (index, (mine, theirs)) in ours.footer.iter().zip(&theirs.footer).enumerate() {
        if mine.first() == Some(&0x0a) {
            // The USERCODE. `gowin_pack` writes one; this writes zero.
            assert_eq!(mine[..4], theirs[..4]);
            continue;
        }
        assert_eq!(mine, theirs, "footer line {index}");
    }

    // Our own writer's output reads back with every check word verified,
    // which is what `FsStream::parse` does, and the geometry matches.
    let text = ours.to_text();
    let again = FsStream::parse(&text, cols).expect("our own output reads back");
    assert_eq!(again.bitmap, ours.bitmap);
    assert_eq!(text.lines().count(), reference.lines().count());

    // The one check word that is a constant of the format: twenty-four
    // 0xff bytes, six from the last row and eighteen of the footer's own.
    assert_eq!(crc16_arc(&[0xff; 24], 0), 0x7334);

    // And the bits. Every bit this sets, the reference sets too — the
    // `const` tables are a subset of what a real packer writes — and the
    // ones it does not are the unused IO ring, which needs the attribute
    // names the database does not carry.
    let mut ours_only = 0usize;
    let mut theirs_only = 0usize;
    let mut both = 0usize;
    for row in 0..ours.bitmap.rows() {
        for col in 0..ours.bitmap.cols() {
            match (ours.bitmap.get(row, col), theirs.bitmap.get(row, col)) {
                (true, true) => both += 1,
                (true, false) => ours_only += 1,
                (false, true) => theirs_only += 1,
                (false, false) => {}
            }
        }
    }
    eprintln!("bits: {both} in both, {ours_only} only ours, {theirs_only} only the reference's");
    assert_eq!(both, fabric.stats.const_bits);
    assert_eq!(
        ours_only, 0,
        "every bit this sets is one the reference packer also sets"
    );
    assert!(
        theirs_only > 0,
        "the reference sets bits this does not, and that is the documented gap"
    );
}

#[test]
fn the_code_tables_are_there_and_their_names_are_not() {
    let Some(db) = open() else { return };
    // A logic tile type: `CLS0`, `CLS1`, `CLS2` for the three slices and
    // `LUT` for the lookup tables.
    let cls0 = db.code_table("shortval", 15, "CLS0");
    assert!(!cls0.is_empty(), "ttyp 15 has no `CLS0` table");
    eprintln!("shortval[15][CLS0]: {} row(s)", cls0.len());
    // With nothing set, the negative rows apply — which is why a Gowin
    // bitstream is not a blank sheet.
    let default = fuses_for(&cls0, &BTreeSet::new());
    eprintln!(
        "  with no attribute set: {} bit(s) {default:?}",
        default.len()
    );
    assert!(
        !default.is_empty(),
        "a Gowin slice has bits that are on until something turns them off"
    );
    // And the `logicinfo` that turns an attribute pair into a code. The
    // pairs are numbers: the names live in `apycula/attrids.py` and not in
    // this file, which is the gap `docs/fpga-gowin.md` names.
    let slice = db.logicinfo("SLICE");
    eprintln!("logicinfo[SLICE]: {} entr(ies)", slice.len());
    assert!(!slice.is_empty());
    assert!(
        slice.iter().all(|((a, v), _)| *a >= 0 && *v >= 0),
        "an attribute id is a number and nothing more"
    );
    // An IO buffer's table is a `longval`: sixteen codes to a key.
    let ioba = db.code_table("longval", 52, "IOBA");
    assert!(!ioba.is_empty(), "ttyp 52 has no `IOBA` table");
    assert!(
        ioba.iter().all(|(key, _)| key.len() == 16),
        "a `longval` key is sixteen codes"
    );
    eprintln!("longval[52][IOBA]: {} row(s) of 16 code(s)", ioba.len());
}

#[test]
fn a_region_is_what_makes_a_small_design_cheap() {
    let Some(db) = open() else { return };
    // Eleven tiles by eleven around the middle, which is the shape
    // `reticle fpga` would pick from a design's pins.
    let small = db
        .load(&ApiculaOptions::new().with_region(GridRegion::new(25, 25, 35, 35)))
        .expect("a small region loads");
    let whole = db
        .load(&ApiculaOptions::new())
        .expect("the whole die loads");
    eprintln!(
        "121 tiles: {} pip(s); 3080 tiles: {} pip(s)",
        small.stats.pips, whole.stats.pips
    );
    // A ratio in one process, which is a statement about the loader rather
    // than about the machine it runs on: a region of 121 of 3080 tiles
    // costs far less than a twentieth of the whole die's edges.
    assert!(small.stats.tiles_loaded == 121);
    assert!(
        small.stats.pips * 20 < whole.stats.pips,
        "{} vs {}",
        small.stats.pips,
        whole.stats.pips
    );
    // The die-wide measurements do not shrink with the region, and neither
    // does the bitmap: a bitstream is always a whole-part bitstream.
    assert_eq!(small.stats.pips_die, whole.stats.pips_die);
    assert_eq!(small.stats.bitmap_rows, whole.stats.bitmap_rows);
    assert_eq!(small.layout, whole.layout);
    // And a limit refuses with the numbers rather than with a dead
    // machine, having counted first.
    let mut options = ApiculaOptions::new();
    options.max_pips = 1000;
    match db.load(&options) {
        Err(ApiculaError::TooLarge { pips, limit }) => {
            eprintln!("refused: {pips} edge(s), limit {limit}");
            assert_eq!(limit, 1000);
            assert!(pips > 1000);
        }
        other => panic!("expected a refusal, got {:?}", other.map(|f| f.stats.pips)),
    }
}

#[test]
fn the_c_variant_is_the_same_fabric_with_different_errata() {
    let Some(root) = chipdb() else { return };
    let plain = ApiculaDatabase::open(&root, DEVICE, &DiskFiles).expect("GW2A-18 opens");
    let Ok(c) = ApiculaDatabase::open(&root, "GW2A-18C", &DiskFiles) else {
        eprintln!("skipped: the checkout has no GW2A-18C.msgpack.xz");
        return;
    };
    // The same grid, the same tile types and the same IDCODE, which is
    // exactly why the IDCODE cannot choose between them.
    assert_eq!(plain.grid_size(), c.grid_size());
    assert_eq!(plain.idcode(), c.idcode());
    assert_eq!(plain.layout(), c.layout());
    assert_eq!(plain.header(), c.header());
    // And both list the part this board carries, so the marking does not
    // choose either. What differs is the part *list* — the `GW2AR` parts,
    // the ones with SDRAM in the package, are only in the C database — and
    // the block RAM errata, which this loader does not read.
    let parts = |db: &ApiculaDatabase| -> BTreeSet<String> {
        db.packages().into_iter().map(|p| p.part).collect()
    };
    let (a, b) = (parts(&plain), parts(&c));
    assert!(a.contains(PART), "GW2A-18 does not list {PART}");
    assert!(b.contains(PART), "GW2A-18C does not list {PART}");
    let only_c: Vec<_> = b.difference(&a).cloned().collect();
    eprintln!(
        "GW2A-18 has {} part(s), GW2A-18C {}; {} only in the C database, such as {:?}",
        a.len(),
        b.len(),
        only_c.len(),
        only_c.first()
    );
    assert!(
        only_c.iter().any(|p| p.starts_with("GW2AR")),
        "the GW2AR parts should be the C database's"
    );
}

#[test]
fn the_device_file_declares_only_what_this_flow_can_actually_build() {
    // No database needed: this is `src/fpga/devices/gowin.dev` against its
    // own account of itself.
    let device = reticle::fpga::target("gw2a-18-pg256").expect("the device file is compiled in");
    assert_eq!(device.family, "gowin");
    assert_eq!(device.lut_size, 4);
    assert_eq!(device.idcode, Some(IDCODE));
    assert_eq!(device.package, "PBGA256");

    // A LUT4 and an input and an output buffer are mapped.
    let input = device.io_bel("in").expect("an input buffer");
    let output = device.io_bel("out").expect("an output buffer");
    assert_eq!(input.name, "IBUF");
    assert_eq!(output.name, "OBUF");
    assert!(input.has_ports(&["pad", "din"]));
    assert!(output.has_ports(&["pad", "dout"]));

    // And these are deliberately NOT mapped, each for a reason the file
    // states. A test is the only place such a decision stays decided.
    assert!(
        device.io_bel("inout").is_none(),
        "a Gowin IOBUF's OEN is active low and the model cannot say so"
    );
    assert!(
        device.bel(BelRole::GlobalBuffer).is_none(),
        "a Gowin clock is routed onto the global network, not buffered onto it"
    );
    assert!(
        device.bel(BelRole::Carry).is_none(),
        "a Gowin ALU produces the sum as well as the carry and fits neither carry shape"
    );
    assert!(
        device.block_rams.is_empty(),
        "a Gowin BSRAM needs its BLKSEL tied and its contents live outside the die bitmap"
    );
    assert!(
        device.dsps.is_empty(),
        "a Gowin multiplier needs tied mode inputs"
    );
    assert!(
        device.bel(BelRole::LutRam).is_none(),
        "RAM16SDP4's ports are vectors and the `lutram` line names individual pins"
    );

    // Twenty flip-flops, ten edges each way, and the ones without an
    // enable really have no CE pin — on this family that is a different
    // primitive and not the same one with CE tied high.
    let ffs: Vec<&str> = device
        .bels
        .iter()
        .filter(|b| b.role == BelRole::Ff)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(ffs.len(), 20, "{ffs:?}");
    let plain = device
        .bels
        .iter()
        .find(|b| b.name == "DFF")
        .expect("a plain DFF");
    assert!(plain.port("en").is_none(), "DFF has no CE port");
    assert!(plain.port("d").is_some() && plain.port("q").is_some());

    // The PLL is described and, on purpose, not configurable: its output
    // divider's legal values are not a range.
    let pll = device
        .clock_resources
        .plls
        .first()
        .expect("the rPLL is described");
    assert_eq!(pll.name, "rPLL");
    assert_eq!(pll.vco_mhz, Some((500, 1250)));
    assert_eq!(pll.feedback, PllFeedback::Output);
    assert!(
        pll.divider(PllDividerRole::Output).is_none(),
        "ODIV_SEL takes 2, 4, 8, 16, 32, 48, 64, 80, 96, 112 or 128 and nothing between"
    );
    assert!(!pll.is_configurable());
    let refused = reticle::fpga::pll::solve(pll, 27.0, 108.0).unwrap_err();
    eprintln!("asking for a PLL: {refused}");
    assert!(refused.contains("rPLL"), "{refused}");
}

#[test]
fn the_device_file_and_the_database_agree_pin_for_pin() {
    let Some(db) = open() else { return };
    let device = reticle::fpga::target("gw2a-18-pg256").expect("the device file is compiled in");
    let fabric = db
        .load(&ApiculaOptions::new().with_part(PART))
        .expect("the whole die loads");

    // The IDCODE, which is the check that stops a bitstream built from one
    // die's database being labelled with another's.
    assert_eq!(device.idcode, db.idcode());
    fabric
        .check_idcode(device.idcode.unwrap())
        .expect("the device file and the database agree");

    // The grid bounding box.
    let grid = device.tile_grid.as_ref().expect("a grid");
    assert_eq!(
        (grid.width, grid.height),
        (fabric.stats.cols, fabric.stats.rows)
    );

    // The resource counts, re-derived from the database so the file's
    // numbers cannot drift away from the part.
    let bel = |name: &str| fabric.stats.bels_die.get(name).copied().unwrap_or(0);
    let luts: usize = (0..8).map(|i| bel(&format!("LUT{i}"))).sum();
    let ffs: usize = (0..6).map(|i| bel(&format!("DFF{i}"))).sum();
    let declared = |name: &str| {
        device
            .bels
            .iter()
            .find(|b| b.name == name)
            .and_then(|b| b.count)
            .map(usize::try_from)
            .and_then(Result::ok)
    };
    assert_eq!(declared("LUT4"), Some(luts));
    assert_eq!(declared("DFF"), Some(ffs));
    assert_eq!(
        device
            .clock_resources
            .plls
            .first()
            .and_then(|p| p.count)
            .and_then(|c| usize::try_from(c).ok()),
        Some(bel("RPLLA"))
    );

    // Every ball the file lists is a ball the database's pinout has, in
    // the bank the database's `pin_bank` puts it in, and every one of them
    // reaches a site the fabric really has.
    assert_eq!(device.pins.len(), fabric.stats.pins_mapped);
    assert!(!device.pins_partial);
    let banks: BTreeSet<&str> = device.io_banks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(banks.len(), 8, "{banks:?}");
    for pin in &device.pins {
        let site = fabric.arch.site_of_pin(&pin.name).unwrap_or_else(|| {
            panic!(
                "the file lists ball {} and the fabric has no site",
                pin.name
            )
        });
        assert!(site.contains("/IOB"), "{} -> {site}", pin.name);
        let bank = pin.bank.as_deref().expect("every ball names a bank");
        assert!(banks.contains(bank), "{} is in `{bank}`", pin.name);
    }
    eprintln!(
        "{} ball(s) across {} bank(s), all of them sites of the loaded fabric",
        device.pins.len(),
        banks.len()
    );
    // The sixteen clock-capable balls are the eight differential global
    // clock inputs the package's own pinout names.
    let clocks = device
        .pins
        .iter()
        .filter(|p| p.kind == PinKind::Clock)
        .count();
    assert_eq!(clocks, 16);
    assert_eq!(device.clock_resources.global_buffers, 8);
}
