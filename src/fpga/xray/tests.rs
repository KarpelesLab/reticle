//! Unit tests for the readers, on text written here rather than on the
//! database.
//!
//! The database is 35 MB that this repository does not vendor, so these
//! feed each reader the shape of the file it reads, taken from the real
//! one. `tests/fpga_xray.rs` runs the whole loader against the real
//! database when `RETICLE_CHIPDB` points at one, and skips when it does
//! not.

use std::collections::HashSet;

use super::*;
use crate::fpga::arch::ConfigEntry;
use crate::ir::memfile::MemoryFiles;

#[test]
fn a_die_finds_its_family_and_its_fabric() {
    assert_eq!(family_directory("xc7a35t"), Some("artix7"));
    assert_eq!(family_directory("xc7k70t"), Some("kintex7"));
    assert_eq!(family_directory("xc7s50"), Some("spartan7"));
    assert_eq!(family_directory("xc7vx485t"), Some("virtex7"));
    assert_eq!(family_directory("xc7z020"), Some("zynq7"));
    assert_eq!(family_directory("xc7q1"), None);
    assert_eq!(family_directory("ice40"), None);

    // The shape `mapping/devices.yaml` really has.
    let text = "# device to fabric mapping\n\
                \"xc7a200t\":\n  fabric: \"xc7a200t\"\n\
                \"xc7a35t\":\n  fabric: \"xc7a50t\"\n";
    assert_eq!(fabric_of(text, "xc7a35t").as_deref(), Some("xc7a50t"));
    assert_eq!(fabric_of(text, "xc7a200t").as_deref(), Some("xc7a200t"));
    assert_eq!(fabric_of(text, "xc7a100t"), None);
}

#[test]
fn a_feature_is_a_pip_when_nothing_makes_its_head_a_site() {
    // `INT_L` has no feature of three parts, so every two-part one is a
    // pip.
    let int = parse::segbits(
        "INT_L.BYP_ALT0.BYP_BOUNCE_N3_3 21_07 !22_07 23_07\n\
         INT_L.BYP_ALT0.FAN_BOUNCE2 21_07 !22_07\n",
        "segbits_int_l.db",
    )
    .unwrap();
    assert!(int.sites().is_empty());
    assert_eq!(int.pip_count(), 2);
    let pips: Vec<_> = int.pips().map(|(to, from, _)| (to, from)).collect();
    assert_eq!(pips[0], ("BYP_ALT0", "BYP_BOUNCE_N3_3"));
    assert_eq!(int.features()[0].ones.len(), 2);
    assert_eq!(int.features()[0].zeros.len(), 1);

    // A `CLBLL_L` has `SLICEL_X0.AFF.ZINI`, which makes `SLICEL_X0` a
    // site, so `SLICEL_X0.CLKINV` is a bel feature and not a pip.
    let clb = parse::segbits(
        "CLBLL_L.SLICEL_X0.AFF.ZINI 31_03\n\
         CLBLL_L.SLICEL_X0.CLKINV 20_00\n\
         CLBLL_L.SLICEL_X0.ALUT.INIT[0] 32_00\n",
        "segbits_clbll_l.db",
    )
    .unwrap();
    assert_eq!(clb.sites().iter().collect::<Vec<_>>(), vec!["SLICEL_X0"]);
    assert_eq!(clb.pip_count(), 0);

    let mut sites = HashSet::new();
    sites.insert("SLICEL_X0".to_owned());
    assert!(!is_pip_feature("SLICEL_X0.CLKINV", &sites));
    assert!(is_pip_feature("BYP_ALT0.FAN_BOUNCE2", &sites));
    // One part is a tile-wide feature, three or more is a bel feature.
    assert!(!is_pip_feature("ENABLE_BUFFER", &sites));
    assert!(!is_pip_feature("A.B.C", &sites));
}

#[test]
fn a_bad_bit_position_is_reported_with_its_line() {
    let err = parse::segbits("INT_L.A.B 12_34\nINT_L.A.C 9_x\n", "segbits_int_l.db").unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
    assert!(err.to_string().contains("not a <frame>_<bit>"), "{err}");
    assert!(matches!(err, XrayError::Malformed { .. }));
    // A word with no underscore is a `ppips`-style keyword, not an error.
    let set = parse::segbits("INT_L.A.B always\n", "x.db").unwrap();
    assert!(set.features()[0].ones.is_empty());
}

/// `part.json`'s shape, with one row of one bus.
const PART_JSON: &str = r#"{
  "global_clock_regions": {
    "top": {"rows": {"0": {"configuration_buses": {
      "CLB_IO_CLK": {"configuration_columns": {"0": {"frame_count": 2}, "1": {"frame_count": 3}}}
    }}}},
    "bottom": {"rows": {"0": {"configuration_buses": {
      "BLOCK_RAM": {"configuration_columns": {"0": {"frame_count": 4}}}
    }}}}
  },
  "idcode": 56807571,
  "iobanks": {}
}"#;

#[test]
fn the_part_layout_comes_out_of_part_json() {
    let json = Json::parse(PART_JSON).unwrap();
    let part = parse::part_from_json(&json, "part.json", "7atest").unwrap();
    assert_eq!(part.idcode, 0x0362_d093);
    assert_eq!(part.layout.data_frames(), 2 + 3 + 4);
    // Two rows, so two pads each.
    assert_eq!(part.layout.frames(), 9 + 4);
    assert_eq!(part.layout.rows().len(), 2);
    // `CLB_IO_CLK` is block 0 and sorts before `BLOCK_RAM`'s block 1,
    // whichever order the file listed them in.
    assert_eq!(part.layout.rows()[0].block, 0);
    assert_eq!(part.layout.rows()[0].half, 0);
    assert_eq!(part.layout.rows()[1].block, 1);
    assert_eq!(part.layout.rows()[1].half, 1);

    let bad = Json::parse(r#"{"idcode": 1}"#).unwrap();
    let err = parse::part_from_json(&bad, "part.json", "x").unwrap_err();
    assert!(err.to_string().contains("global_clock_regions"), "{err}");
}

/// `tilegrid.json`'s shape, with two tiles.
const TILEGRID_JSON: &str = r#"{
  "CLBLL_L_X2Y0": {
    "bits": {"CLB_IO_CLK": {"baseaddr": "0x00400100", "frames": 36, "offset": 0, "words": 2}},
    "grid_x": 10, "grid_y": 155,
    "sites": {"SLICE_X1Y0": "SLICEL", "SLICE_X0Y0": "SLICEL"},
    "type": "CLBLL_L"
  },
  "VBRK_X0Y0": {"bits": {}, "grid_x": 0, "grid_y": 0, "sites": {}, "type": "VBRK"}
}"#;

#[test]
fn the_tile_grid_comes_out_of_tilegrid_json() {
    let json = Json::parse(TILEGRID_JSON).unwrap();
    let tiles = parse::tilegrid(&json, "tilegrid.json").unwrap();
    assert_eq!(tiles.len(), 2);
    let clb = &tiles[0];
    assert_eq!(clb.name, "CLBLL_L_X2Y0");
    assert_eq!(clb.tile_type, "CLBLL_L");
    assert_eq!((clb.grid_x, clb.grid_y), (10, 155));
    // Sites come back in name order, which is what makes a tile type's
    // site list uniform across tiles.
    assert_eq!(clb.sites[0].0, "SLICE_X0Y0");
    assert_eq!(clb.bits.len(), 1);
    assert_eq!(clb.bits[0].1.baseaddr, 0x0040_0100);
    assert_eq!(clb.bits[0].1.frames, 36);
    assert_eq!(clb.bits[0].1.words, 2);
    assert_eq!(clb.bits[0].1.bit_count(), 36 * 64);
    assert!(tiles[1].bits.is_empty());

    let err = parse::tilegrid(&Json::Array(Vec::new()), "tilegrid.json").unwrap_err();
    assert!(err.to_string().contains("not an object"), "{err}");
}

#[test]
fn tile_joins_come_out_of_tileconn_json() {
    let text = r#"[{"grid_deltas": [1, 0], "tile_types": ["INT_L", "INT_R"],
                    "wire_pairs": [["EE2BEG0", "EE2A0"], ["EE2BEG1", "EE2A1"]]}]"#;
    let json = Json::parse(text).unwrap();
    let conns = parse::tileconn(&json, "tileconn.json").unwrap();
    assert_eq!(conns.len(), 1);
    assert_eq!(conns[0].types, ("INT_L".to_owned(), "INT_R".to_owned()));
    assert_eq!(conns[0].delta, (1, 0));
    assert_eq!(conns[0].pairs.len(), 2);

    let err = parse::tileconn(&Json::object(), "tileconn.json").unwrap_err();
    assert!(err.to_string().contains("not an array"), "{err}");
}

#[test]
fn package_pins_map_to_sites() {
    let text = "pin,bank,site,tile,pin_function\n\
                V17,14,IOB_X0Y26,LIOB33_X0Y26,IO_L18N_T2_A23_15\n\
                U16,14,IOB_X0Y24,LIOB33_X0Y24,IO_L18P_T2_A24_15\n\
                ,,,,\n";
    let pins = parse::package_pins(text);
    assert_eq!(
        pins,
        vec![
            ("V17".to_owned(), "IOB_X0Y26".to_owned()),
            ("U16".to_owned(), "IOB_X0Y24".to_owned()),
        ]
    );
    // A file with no `site` column gives nothing rather than guessing.
    assert!(parse::package_pins("pin,bank\nV17,14\n").is_empty());
    assert!(parse::package_pins("").is_empty());
}

#[test]
fn a_slice_becomes_luts_flip_flops_and_a_carry() {
    let json = Json::parse(TILEGRID_JSON).unwrap();
    let tiles = parse::tilegrid(&json, "tilegrid.json").unwrap();
    let features = parse::segbits(
        "CLBLL_L.SLICEL_X0.ALUT.INIT[0] 32_00\n\
         CLBLL_L.SLICEL_X0.ALUT.INIT[1] 32_01\n\
         CLBLL_L.SLICEL_X0.AFF.ZINI 31_03\n\
         CLBLL_L.SLICEL_X0.PRECYINIT.AX 20_00\n\
         CLBLL_L.SLICEL_X0.CLKINV 20_01\n",
        "segbits_clbll_l.db",
    )
    .unwrap();
    let wires: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut coverage = super::SiteCoverage::default();
    let bels = parse::bels_of(&tiles[0], &features, &wires, None, &mut coverage);
    let names: Vec<&str> = bels.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"SLICEL_X0_ALUT"), "{names:?}");
    assert!(names.contains(&"SLICEL_X0_AFF"), "{names:?}");
    assert!(names.contains(&"SLICEL_X0_PRECYINIT"), "{names:?}");
    assert!(names.contains(&"SLICEL_X0"), "{names:?}");

    let lut = bels.iter().find(|b| b.name == "SLICEL_X0_ALUT").unwrap();
    assert_eq!(lut.kind, "lut");
    assert_eq!(
        lut.config,
        vec![
            ConfigEntry::Param {
                name: "INIT".to_owned(),
                index: 0,
                at: ConfigBit::new(32, 0),
            },
            ConfigEntry::Param {
                name: "INIT".to_owned(),
                index: 1,
                at: ConfigBit::new(32, 1),
            },
        ]
    );
    // A mode feature keeps the database's own name for itself, because
    // nothing here knows what Reticle would call it.
    let ff = bels.iter().find(|b| b.name == "SLICEL_X0_AFF").unwrap();
    assert_eq!(ff.kind, "ff");
    assert_eq!(
        ff.config,
        vec![ConfigEntry::Cell {
            primitive: "ZINI".to_owned(),
            bits: vec![ConfigBit::new(31, 3)],
        }]
    );
    // The gap that stops a signal reaching any of them.
    assert!(bels.iter().all(|b| b.pins.is_empty()));
}

#[test]
fn a_missing_database_is_reported_by_path() {
    let files = MemoryFiles::new();
    let err =
        XrayDatabase::open(&files, "/nowhere", "xc7a35t-cpg236", &XrayOptions::new()).unwrap_err();
    assert!(err.to_string().contains("part.json"), "{err}");
    assert!(matches!(err, XrayError::WrongPart { .. }));

    // A part directory with a `part.json` but no `devices.yaml`.
    let mut files = MemoryFiles::new();
    files.insert("/db/artix7/xc7a35tcpg236-1/part.json", PART_JSON);
    let err = XrayDatabase::open(&files, "/db", "xc7a35t-cpg236", &XrayOptions::new()).unwrap_err();
    assert!(err.to_string().contains("devices.yaml"), "{err}");
    assert!(matches!(err, XrayError::Missing { .. }));

    // A device name that is not `<die>-<package>`.
    let err = XrayDatabase::open(&files, "/db", "nonsense", &XrayOptions::new()).unwrap_err();
    assert!(err.to_string().contains("`<die>-<package>`"), "{err}");
}

#[test]
fn a_whole_database_loads_from_memory() {
    let mut files = MemoryFiles::new();
    files.insert("/db/artix7/xc7a35tcpg236-1/part.json", PART_JSON);
    files.insert(
        "/db/artix7/xc7a35tcpg236-1/package_pins.csv",
        "pin,site\nV17,SLICE_X0Y0\nZZ9,NOWHERE\n",
    );
    files.insert(
        "/db/artix7/mapping/devices.yaml",
        "\"xc7a35t\":\n  fabric: \"xc7a50t\"\n",
    );
    files.insert("/db/artix7/xc7a50t/tilegrid.json", TILEGRID_JSON);
    files.insert(
        "/db/artix7/xc7a50t/tileconn.json",
        r#"[{"grid_deltas": [1, 0], "tile_types": ["CLBLL_L", "VBRK"],
             "wire_pairs": [["OUT0", "IN0"]]}]"#,
    );
    files.insert(
        "/db/artix7/segbits_clbll_l.db",
        "CLBLL_L.SLICEL_X0.ALUT.INIT[0] 32_00\nCLBLL_L.OUT0.IN1 30_00\n",
    );
    files.insert("/db/artix7/element_counts.csv", "type,count\nnodes,42\n");

    let db = XrayDatabase::open(&files, "/db", "xc7a35t-cpg236", &XrayOptions::new()).unwrap();
    assert_eq!(db.root(), "/db");
    assert_eq!(db.fabric(), "xc7a50t");
    let fabric = db.load(&files, &XrayOptions::new()).unwrap();

    assert_eq!(fabric.part.idcode, 0x0362_d093);
    assert_eq!(fabric.stats.tiles, 2);
    assert_eq!(fabric.stats.tile_types, 2);
    assert_eq!(fabric.stats.nodes_die, Some(42));
    // One tile has bits, the other does not.
    assert_eq!(fabric.stats.mapped_tiles, 1);
    // The grid is the extent of the tiles, so the CLB at grid (10, 155)
    // makes it 11 x 156.
    assert_eq!((fabric.arch.width, fabric.arch.height), (11, 156));
    // The pip, plus the two directions of the one join.
    assert_eq!(fabric.stats.pips, 3);
    assert_eq!(fabric.stats.joins, 2);
    // Only the pin whose site the grid really has, and named the way
    // the architecture names bels rather than the way `tilegrid.json`
    // names sites: `SLICE_X0Y0` is the first site of the tile, so it is
    // the first site prefix the features use.
    assert_eq!(
        fabric.arch.pinmap,
        vec![("V17".to_owned(), "X10Y155/SLICEL_X0".to_owned())]
    );
    fabric.check_idcode(0x0362_d093).unwrap();

    // The tile bitmap is the frame window: 36 frames of two words.
    let clb = fabric.arch.tile_at(10, 155).unwrap();
    assert_eq!((clb.bit_rows, clb.bit_cols), (36, 64));
    assert_eq!(fabric.frames.windows((10, 155)).len(), 1);
}

#[test]
fn a_region_that_will_not_fit_is_refused_before_it_is_built() {
    let mut files = MemoryFiles::new();
    files.insert("/db/artix7/xc7a35tcpg236-1/part.json", PART_JSON);
    files.insert("/db/artix7/xc7a35tcpg236-1/package_pins.csv", "pin,site\n");
    files.insert(
        "/db/artix7/mapping/devices.yaml",
        "\"xc7a35t\":\n  fabric: \"xc7a50t\"\n",
    );
    files.insert("/db/artix7/xc7a50t/tilegrid.json", TILEGRID_JSON);
    files.insert("/db/artix7/xc7a50t/tileconn.json", "[]");
    files.insert(
        "/db/artix7/segbits_clbll_l.db",
        "CLBLL_L.OUT0.IN1 30_00\nCLBLL_L.OUT1.IN2 30_01\n",
    );
    let db = XrayDatabase::open(&files, "/db", "xc7a35t-cpg236", &XrayOptions::new()).unwrap();

    let mut options = XrayOptions::new();
    options.max_pips = 1;
    let err = db.load(&files, &options).unwrap_err();
    let XrayError::TooLarge { pips, limit, tiles } = err else {
        panic!("expected a refusal, got {err}");
    };
    assert_eq!((pips, limit, tiles), (2, 1, 2));

    // And a region small enough to hold it loads.
    let options = XrayOptions::new().with_region(GridRegion::around(10, 155, 1));
    let fabric = db.load(&files, &options).unwrap();
    assert_eq!(fabric.stats.tiles_loaded, 1);
    assert!(!fabric.stats.to_text().contains("18055"));
    assert!(fabric.stats.to_text().contains("frame map"));
}

#[test]
fn a_grid_region_is_a_rectangle_either_way_round() {
    let r = GridRegion::new(5, 9, 1, 3);
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (1, 3, 5, 9));
    assert_eq!(r.area(), 5 * 7);
    assert!(r.contains(1, 3) && r.contains(5, 9) && r.contains(3, 5));
    assert!(!r.contains(0, 3) && !r.contains(3, 10));
    // A radius around the origin clamps rather than wrapping.
    let around = GridRegion::around(1, 1, 4);
    assert_eq!((around.x0, around.y0), (0, 0));
}
