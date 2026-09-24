//! The loader against a **synthetic** database built here in memory.
//!
//! These run in every build with the feature on, with no file and no
//! board, and they are what checks the rules the loader applies: that an
//! inter-tile wire is declared once under its root and referred to at an
//! offset, that a reference whose root falls off the grid is counted and
//! left unconnected rather than joined to the wrong wire, that a LUT's
//! `INIT` reaches the architecture inverted, and that the `.fs` envelope
//! comes out of the database rather than out of this file.
//!
//! What they cannot check is whether the real database says what this
//! loader thinks it says. `tests/fpga_gowin.rs` does that, against a real
//! `GW2A-18.msgpack.xz`, and skips without one.

use super::*;

/// A MessagePack **writer**, for tests only.
///
/// [`crate::msgpack`] deliberately has no serialiser — nothing in Reticle
/// produces MessagePack — but a test needs input, and building it from
/// bytes by hand would be unreadable. This is the smallest encoder that
/// covers the shapes an Apicula database has, in the wide forms so there
/// is no length-dependent branching to get wrong.
mod pack {
    /// One value to encode.
    pub(super) enum V {
        Nil,
        Bool(bool),
        U(u64),
        I(i64),
        S(&'static str),
        Bin(Vec<u8>),
        A(Vec<V>),
        M(Vec<(V, V)>),
    }

    fn encode(value: &V, out: &mut Vec<u8>) {
        match value {
            V::Nil => out.push(0xc0),
            V::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
            V::U(u) => {
                out.push(0xcf);
                out.extend_from_slice(&u.to_be_bytes());
            }
            V::I(i) => {
                out.push(0xd3);
                out.extend_from_slice(&i.to_be_bytes());
            }
            V::S(s) => string(s, out),
            V::Bin(bytes) => {
                out.push(0xc6);
                out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
                out.extend_from_slice(bytes);
            }
            V::A(items) => {
                out.push(0xdd);
                out.extend_from_slice(&u32::try_from(items.len()).unwrap().to_be_bytes());
                for item in items {
                    encode(item, out);
                }
            }
            V::M(pairs) => {
                out.push(0xdf);
                out.extend_from_slice(&u32::try_from(pairs.len()).unwrap().to_be_bytes());
                for (key, value) in pairs {
                    encode(key, out);
                    encode(value, out);
                }
            }
        }
    }

    fn string(s: &str, out: &mut Vec<u8>) {
        out.push(0xdb);
        out.extend_from_slice(&u32::try_from(s.len()).unwrap().to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    /// The bytes of one value.
    pub(super) fn bytes(value: &V) -> Vec<u8> {
        let mut out = Vec::new();
        encode(value, &mut out);
        out
    }

    /// `[[row, col], ..]`.
    pub(super) fn coords(pairs: &[(u64, u64)]) -> V {
        V::A(
            pairs
                .iter()
                .map(|(r, c)| V::A(vec![V::U(*r), V::U(*c)]))
                .collect(),
        )
    }
}

use pack::V;

/// A GW2A-18's ten header lines and six footer lines, which is what the
/// real database carries and what the loader must take verbatim.
fn header_lines() -> Vec<Vec<u8>> {
    vec![
        vec![0xff; 20],
        vec![0xff, 0xff],
        vec![0xa5, 0xc3],
        vec![0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x1b],
        vec![0x10, 0x00, 0x00, 0x00, 0x00, 0xae, 0x00, 0x00],
        vec![0x51, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        vec![0x0b, 0x00, 0x00, 0x00],
        vec![0xd2, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00],
        vec![0x12, 0x00, 0x00, 0x00],
        vec![0x3b, 0x80, 0x00, 0x00],
    ]
}

fn footer_lines() -> Vec<Vec<u8>> {
    vec![
        {
            let mut line = vec![0xff; 18];
            line.extend_from_slice(&[0x34, 0x73]);
            line
        },
        vec![0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        vec![0xff; 8],
        vec![0x08, 0x00, 0x00, 0x00],
        vec![0xff; 8],
        vec![0xff, 0xff],
    ]
}

/// A two-by-two grid of two tile types, shaped like the real thing in
/// every way this loader cares about.
///
/// ```text
///        col 0        col 1
/// row 0  C1 (logic)   I2 (io)
/// row 1  I2 (io)      C1 (logic)
/// ```
///
/// The logic type holds a LUT4 with two `INIT` bits and drives an
/// inter-tile wire's root, `E210`; the IO type holds an `IOBA` and reads
/// that wire's **segment one**, `E211`, whose root is one column west. At
/// `(row 0, col 1)` that root is column 0 and on the grid; at `(row 1, col
/// 0)` it is column −1 and off it, which is the edge wrap.
fn database() -> ApiculaDatabase {
    let logic = V::M(vec![
        (V::S("width"), V::U(8)),
        (V::S("height"), V::U(4)),
        (V::S("ttyp"), V::U(1)),
        (
            V::S("pips"),
            V::M(vec![
                (
                    V::S("D1"),
                    V::M(vec![(V::S("A0"), pack::coords(&[(0, 0), (1, 1)]))]),
                ),
                (
                    V::S("E210"),
                    V::M(vec![(V::S("D1"), pack::coords(&[(2, 2)]))]),
                ),
            ]),
        ),
        (V::S("alonenode"), V::M(vec![])),
        (V::S("clock_pips"), V::M(vec![])),
        (V::S("alonenode_6"), V::M(vec![])),
        (
            V::S("bels"),
            V::M(vec![(
                V::S("LUT0"),
                V::M(vec![
                    (
                        V::S("flags"),
                        V::M(vec![
                            (V::U(0), pack::coords(&[(3, 0)])),
                            (V::U(1), pack::coords(&[(3, 1)])),
                        ]),
                    ),
                    (V::S("simplified_iob"), V::Bool(false)),
                    (V::S("is_diff"), V::Bool(false)),
                    (V::S("is_true_lvds"), V::Bool(false)),
                    (V::S("is_diff_p"), V::Bool(false)),
                    (V::S("modes"), V::M(vec![])),
                    (
                        V::S("portmap"),
                        V::M(vec![
                            (V::S("F"), V::S("F0")),
                            (V::S("I0"), V::S("A0")),
                            (V::S("I1"), V::S("B0")),
                        ]),
                    ),
                    (V::S("fuse_cell_offset"), V::Nil),
                ]),
            )]),
        ),
    ]);
    let io = V::M(vec![
        (V::S("width"), V::U(8)),
        (V::S("height"), V::U(4)),
        (V::S("ttyp"), V::U(2)),
        (
            V::S("pips"),
            V::M(vec![(
                V::S("A0"),
                V::M(vec![(V::S("E211"), pack::coords(&[(0, 1)]))]),
            )]),
        ),
        (V::S("alonenode"), V::M(vec![])),
        (V::S("clock_pips"), V::M(vec![])),
        (V::S("alonenode_6"), V::M(vec![])),
        (
            V::S("bels"),
            V::M(vec![(
                V::S("IOBA"),
                V::M(vec![
                    (V::S("flags"), V::M(vec![])),
                    (V::S("simplified_iob"), V::Bool(false)),
                    (V::S("is_diff"), V::Bool(true)),
                    (V::S("is_true_lvds"), V::Bool(true)),
                    (V::S("is_diff_p"), V::Bool(true)),
                    (V::S("modes"), V::M(vec![])),
                    (
                        V::S("portmap"),
                        V::M(vec![
                            (V::S("O"), V::S("F6")),
                            (V::S("I"), V::S("A0")),
                            (V::S("OE"), V::S("B0")),
                            // The real database carries these as empty
                            // strings, and a wire with no name is not a
                            // wire.
                            (V::S("BOTTOM_IO_PORT_A"), V::S("")),
                        ]),
                    ),
                    (V::S("fuse_cell_offset"), V::Nil),
                ]),
            )]),
        ),
    ]);

    let root = V::M(vec![
        (
            V::S("grid"),
            V::A(vec![
                V::A(vec![V::U(1), V::U(2)]),
                V::A(vec![V::U(2), V::U(1)]),
            ]),
        ),
        (V::S("center_row"), V::U(1)),
        (V::S("center_col"), V::U(1)),
        (V::S("tiles"), V::M(vec![(V::U(1), logic), (V::U(2), io)])),
        (
            V::S("tile_types"),
            V::M(vec![
                (V::S("C"), V::A(vec![V::U(1)])),
                (V::S("I"), V::A(vec![V::U(2)])),
            ]),
        ),
        (V::S("corner_tiles_io"), V::M(vec![])),
        (
            V::S("packages"),
            V::M(vec![(
                V::S("PART-A"),
                V::A(vec![V::S("PKG"), V::S("DEV"), V::S("C8/I7")]),
            )]),
        ),
        (
            V::S("pinout"),
            V::M(vec![(
                V::S("DEV"),
                V::M(vec![(
                    V::S("PKG"),
                    V::M(vec![
                        (V::S("A1"), V::A(vec![V::S("IOT2A"), V::A(vec![])])),
                        (V::S("B2"), V::A(vec![V::S("IOB1A"), V::A(vec![])])),
                        // An `IOLOC` no tile of this die answers to.
                        (V::S("Z9"), V::A(vec![V::S("IOR5A"), V::A(vec![])])),
                    ]),
                )]),
            )]),
        ),
        (
            V::S("cmd_hdr"),
            V::A(header_lines().into_iter().map(V::Bin).collect()),
        ),
        (
            V::S("cmd_ftr"),
            V::A(footer_lines().into_iter().map(V::Bin).collect()),
        ),
        (
            V::S("const"),
            V::M(vec![(V::U(1), pack::coords(&[(0, 7)]))]),
        ),
        (
            V::S("nodes"),
            V::M(vec![(
                V::S("PCLKL0"),
                V::A(vec![
                    V::S("GLOBAL_CLK"),
                    V::A(vec![
                        V::A(vec![V::U(0), V::U(0), V::S("F0")]),
                        V::A(vec![V::U(1), V::U(1), V::S("F0")]),
                    ]),
                ]),
            )]),
        ),
        (
            V::S("logicinfo"),
            V::M(vec![(
                V::S("CLS0"),
                V::M(vec![(V::A(vec![V::U(0), V::U(1)]), V::U(5))]),
            )]),
        ),
        (
            V::S("shortval"),
            V::M(vec![(
                V::U(1),
                V::M(vec![(
                    V::S("CLS0"),
                    V::M(vec![
                        (V::A(vec![V::I(-7), V::U(0)]), pack::coords(&[(3, 3)])),
                        (V::A(vec![V::U(5), V::U(0)]), pack::coords(&[(3, 4)])),
                    ]),
                )]),
            )]),
        ),
        (V::S("hclk_pips"), V::M(vec![])),
    ]);
    let bytes = pack::bytes(&root);
    ApiculaDatabase::from_msgpack("SYNTH-2", &bytes).unwrap()
}

#[test]
fn something_that_is_not_a_device_database_is_refused_by_name() {
    // A bare integer, a map with no grid, and a grid that is not rows.
    let err = ApiculaDatabase::from_msgpack("X", &pack::bytes(&V::U(7))).unwrap_err();
    assert!(matches!(err, ApiculaError::NotADevice { .. }), "{err}");
    let empty = V::M(vec![(V::S("tiles"), V::M(vec![]))]);
    let err = ApiculaDatabase::from_msgpack("X", &pack::bytes(&empty)).unwrap_err();
    assert_eq!(
        err,
        ApiculaError::NotADevice {
            what: "no `grid` field".to_owned()
        }
    );
    let flat = V::M(vec![(V::S("grid"), V::A(vec![V::U(1)]))]);
    let err = ApiculaDatabase::from_msgpack("X", &pack::bytes(&flat)).unwrap_err();
    assert_eq!(
        err,
        ApiculaError::NotADevice {
            what: "`grid` is not rows of tile types".to_owned()
        }
    );
    // And MessagePack that is not MessagePack at all.
    let err = ApiculaDatabase::from_msgpack("X", &[0xc1]).unwrap_err();
    assert!(matches!(err, ApiculaError::Malformed(_)), "{err}");
}

#[test]
fn the_grid_the_layout_and_the_envelope_come_from_the_database() {
    let db = database();
    assert_eq!(db.device(), "SYNTH-2");
    assert_eq!(db.grid_size(), (2, 2));
    // The layout is the cumulative sums: two rows of four, two columns of
    // eight.
    let layout = db.layout();
    assert_eq!((layout.rows(), layout.cols()), (8, 16));
    assert_eq!(layout.origin(1, 1), Some((4, 8)));
    // The header and footer are the database's bytes, unaltered.
    assert_eq!(db.header(), header_lines());
    assert_eq!(db.footer(), footer_lines());
    assert_eq!(db.idcode(), Some(0x0000_081b));
    assert_eq!(
        db.packages(),
        vec![Package {
            part: "PART-A".to_owned(),
            package: "PKG".to_owned(),
            device: "DEV".to_owned(),
            speed: "C8/I7".to_owned(),
        }]
    );
    // And the top-level keys are reported, which is how the written
    // account of the format is kept honest.
    assert!(db.top_level_keys().contains(&"grid"));
    assert!(db.top_level_keys().contains(&"cmd_ftr"));
}

#[test]
fn an_inter_tile_wire_is_declared_once_under_its_root_and_referred_to_by_offset() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    let arch = &fabric.arch;

    // The logic type declares `E21`, not `E210`, and gives it the span the
    // length digit implies: two tiles east.
    let logic = arch.tile_at(0, 0).unwrap();
    assert_eq!(logic.name, "C1");
    let e21 = logic.wires.iter().find(|w| w.name == "E21").unwrap();
    assert_eq!((e21.dx, e21.dy), (2, 0));
    assert!(!logic.has_wire("E210"));
    // Its pip drives the root in its own tile, so no offset.
    let drive = logic
        .pips
        .iter()
        .find(|p| p.to.name == "E21")
        .expect("the logic tile drives the wire");
    assert_eq!((drive.to.dx, drive.to.dy), (0, 0));
    assert_eq!(drive.from.name, "D1");

    // The IO type reads segment one, so it refers to the root one column
    // west — a negative `dx`, which is what makes this a reference rather
    // than a join.
    let io = arch.tile_at(1, 0).unwrap();
    assert_eq!(io.name, "I2");
    let read = io
        .pips
        .iter()
        .find(|p| p.from.name == "E21")
        .expect("the io tile reads the wire");
    assert_eq!((read.from.dx, read.from.dy), (-1, 0));
    assert_eq!(read.to.name, "A0");

    // And the graph joins them with no extra edge: the pip out of the
    // logic tile and the pip into the io tile touch the same node.
    let graph = arch.build_graph();
    let node_of = |pip: &super::super::arch::Pip| (pip.from, pip.to);
    let mut shared = None;
    for pip in &graph.pips {
        if pip.tile == (0, 0) {
            shared = Some(node_of(pip).1);
        }
    }
    let driven = shared.expect("the logic tile's pip is in the graph");
    assert!(
        graph
            .pips
            .iter()
            .any(|p| p.tile == (1, 0) && p.from == driven),
        "the io tile at (1, 0) should read the node the logic tile at (0, 0) drives"
    );
}

#[test]
fn a_reference_whose_root_falls_off_the_grid_is_counted_and_left_unconnected() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    // The io tile at (row 1, col 0) reads `E211`, whose root is column −1.
    // Exactly one such reference, and it is not joined to anything.
    assert_eq!(fabric.stats.edge_wraps, 1);
    let graph = fabric.arch.build_graph();
    assert!(
        graph.dangling > 0,
        "the wrapped reference should be dropped, not connected"
    );
    // Whatever it is, it is not connected to the *other* `E21` node, which
    // is the error this guards against: two nets shorted by a wrap the
    // loader guessed at.
    let wrapped: Vec<_> = graph.pips.iter().filter(|p| p.tile == (0, 1)).collect();
    assert!(
        wrapped.is_empty() || wrapped.iter().all(|p| p.to != p.from),
        "a wrapped reference must not become a self-loop"
    );
}

#[test]
fn a_lut_carries_its_truth_table_inverted_because_a_fuse_means_a_zero() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    let logic = fabric.arch.tile_at(0, 0).unwrap();
    let lut = logic.bel("LUT0").unwrap();
    assert_eq!(lut.kind, "lut");
    // The four inputs and the output, under Reticle's role names.
    assert_eq!(lut.pin("i0").unwrap().name, "A0");
    assert_eq!(lut.pin("i1").unwrap().name, "B0");
    assert_eq!(lut.pin("o").unwrap().name, "F0");
    // And `ParamZero`, not `Param`: the fuse is set when the bit is zero.
    let mut entries: Vec<_> = lut.config.clone();
    entries.sort_by_key(|e| match e {
        ConfigEntry::ParamZero { index, .. } => *index,
        _ => u32::MAX,
    });
    assert_eq!(
        entries,
        vec![
            ConfigEntry::ParamZero {
                name: "INIT".to_owned(),
                index: 0,
                at: ConfigBit::new(3, 0)
            },
            ConfigEntry::ParamZero {
                name: "INIT".to_owned(),
                index: 1,
                at: ConfigBit::new(3, 1)
            },
        ]
    );
}

#[test]
fn an_io_buffer_gets_its_pins_and_leaves_its_bits_to_the_periphery() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    let io = fabric.arch.tile_at(1, 0).unwrap();
    let buffer = io.bel("IOBA").unwrap();
    assert_eq!(buffer.kind, "io");
    // `O` is into the fabric and `I` out of it.
    assert_eq!(buffer.pin("din").unwrap().name, "F6");
    assert_eq!(buffer.pin("dout").unwrap().name, "A0");
    assert_eq!(buffer.pin("oe").unwrap().name, "B0");
    // No pad pin: a ball is not a wire.
    assert!(buffer.pin("pad").is_none());
    // No bel entries: a buffer's bits depend on its bank, which a tile
    // type does not know, so they are `Periphery`'s. `tests/fpga_gowin.rs`
    // checks them against `gowin_pack` bit for bit.
    assert!(buffer.config.is_empty());
    // Nor is the empty-string port a pin.
    assert!(
        buffer.pins.iter().all(|(_, w)| !w.name.is_empty()),
        "{:?}",
        buffer.pins
    );
}

#[test]
fn the_pin_map_is_built_by_naming_the_ring_and_inverting_it() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new().with_part("PART-A")).unwrap();
    let mut pins = fabric.arch.pinmap.clone();
    pins.sort();
    assert_eq!(
        pins,
        vec![
            ("A1".to_owned(), "X1Y0/IOBA".to_owned()),
            ("B2".to_owned(), "X0Y1/IOBA".to_owned()),
        ]
    );
    // The third ball names an `IOLOC` this die has no tile for, and is
    // counted rather than guessed at.
    assert_eq!(fabric.stats.pins_mapped, 2);
    assert_eq!(fabric.stats.pins_unmapped, 1);
    // A part the database does not have is refused with what it does.
    let err = db
        .load(&ApiculaOptions::new().with_part("GW1N-9-NOPE"))
        .unwrap_err();
    assert!(matches!(err, ApiculaError::NoSuchPart { .. }), "{err}");
}

#[test]
fn the_measurements_are_the_loaders_own_and_cover_what_is_not_loaded() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    let stats = &fabric.stats;
    assert_eq!((stats.rows, stats.cols, stats.tiles), (2, 2, 4));
    assert_eq!(stats.tile_types, 2);
    assert_eq!((stats.bitmap_rows, stats.bitmap_cols), (8, 16));
    // Two pips in the logic type, one in the io type, two tiles of each.
    assert_eq!(stats.pips_die, 2 * 2 + 2);
    assert_eq!(stats.clock_pips_die, 0);
    // Three distinct bit patterns: [(0,0),(1,1)], [(2,2)], [(0,1)].
    assert_eq!(stats.bit_patterns, 3);
    assert_eq!(stats.bels_die["LUT0"], 2);
    assert_eq!(stats.bels_die["IOBA"], 2);
    // The `nodes` table is counted and not loaded, which is the honest
    // half of this loader's account of itself.
    assert_eq!((stats.nodes, stats.node_members), (1, 2));
    assert_eq!(stats.const_bits, 2);
    assert_eq!(stats.tiles_loaded, 4);
    // Two LUT bits in each of two logic tiles.
    assert_eq!(stats.config_entries, 4);
    assert_eq!(stats.idcode, Some(0x0000_081b));
    // And the report says all of it.
    let text = stats.to_text();
    assert!(text.contains("SYNTH-2"), "{text}");
    assert!(
        text.contains("not loaded: 1 node(s) with 2 member(s)"),
        "{text}"
    );
    assert!(text.contains("edge-wrapped"), "{text}");
}

#[test]
fn a_region_loads_only_its_tiles_and_a_limit_refuses_before_allocating() {
    let db = database();
    // One tile.
    let one = db
        .load(&ApiculaOptions::new().with_region(GridRegion::new(0, 0, 0, 0)))
        .unwrap();
    assert_eq!(one.stats.tiles_loaded, 1);
    assert_eq!(one.stats.tile_types_loaded, 1);
    // The die-wide numbers do not shrink with the region, which is the
    // point of reporting both.
    assert_eq!(one.stats.tiles, 4);
    assert_eq!(one.stats.pips_die, 6);
    // And a limit is a refusal with the numbers, not a dead machine.
    let mut options = ApiculaOptions::new();
    options.max_pips = 3;
    let err = db.load(&options).unwrap_err();
    assert_eq!(err, ApiculaError::TooLarge { pips: 6, limit: 3 });
}

#[test]
fn a_blank_stream_is_a_whole_fs_file_that_configures_nothing() {
    let db = database();
    let fabric = db.load(&ApiculaOptions::new()).unwrap();
    let stream = fabric.blank_stream().unwrap();
    // The bitmap is the whole die, whatever the region, and carries the
    // `const` bits and nothing else: bit (0, 7) of each of the two logic
    // tiles, at (0, 0) and at (1, 1).
    assert_eq!(stream.bitmap.count_ones(), 2);
    assert!(stream.bitmap.get(0, 7));
    assert!(stream.bitmap.get(4, 15));
    // The row count reached the load-configuration command.
    assert_eq!(stream.header[9], vec![0x3b, 0x80, 0x00, 0x08]);
    // And it reads back, check words and all.
    let text = stream.to_text();
    let again = FsStream::parse(&text, 16).unwrap();
    assert_eq!(again.bitmap, stream.bitmap);
    assert!(fabric.check_idcode(0x0000_081b).is_ok());
    assert!(fabric.check_idcode(0x0900_281b).is_err());
}

#[test]
fn the_code_tables_are_reachable_and_the_negative_rule_holds_on_a_real_shape() {
    let db = database();
    let table = db.code_table("shortval", 1, "CLS0");
    assert_eq!(table.len(), 2);
    // Nothing set: the negative row applies, which is why a Gowin
    // bitstream is not a blank sheet.
    let none = fuses_for(&table, &BTreeSet::new());
    assert_eq!(none, BTreeSet::from([(3, 3)]));
    // Code 5 set: the positive row applies and the negative one still
    // does, since 5 is not 7.
    let five = fuses_for(&table, &BTreeSet::from([5]));
    assert_eq!(five, BTreeSet::from([(3, 3), (3, 4)]));
    // And the `logicinfo` that turns an attribute pair into that code is
    // there too — with numbers for names, which is the gap.
    assert_eq!(db.logicinfo("CLS0"), vec![((0, 1), 5)]);
    assert!(db.code_table("shortval", 2, "CLS0").is_empty());
}
