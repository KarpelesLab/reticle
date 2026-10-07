//! A 7-series **distributed RAM** against the real fabric database:
//! `examples/basys3/lutram.v`, sixteen words of four bits in four
//! `RAM64X1D`s, placed, routed and written as a `.bit` for the Basys 3.
//!
//! # What these tests can and cannot reach
//!
//! They establish that each RAM sits on a `SLICEM` and nowhere else, that
//! it takes exactly the two lookup tables `src/fpga/xray/lutram.rs` says
//! and leaves the rest of its tile usable, that its write enable, data
//! and clock arrive on the wires the database gives those pins, that its
//! contents reach the bits the database gives them, and that the whole
//! bitstream decodes back into named features with nothing left over and
//! with exactly the interconnect the router chose.
//!
//! What they cannot reach is **the address permutation**: which lookup
//! table input is which address bit. That is quoted from nextpnr-xilinx
//! and Project X-Ray's fuzzer, not measured, and every test here would
//! pass with it wrong — a permuted address places, routes and decodes just
//! the same. `examples/basys3/lutram.v` on a board is the test for it.
//!
//! # Every test here skips without the database
//!
//! Exactly as `tests/fpga_xray.rs` does, and with the same lookup:
//! `RETICLE_CHIPDB`, then the copy `reticle fetch prjxray-db` caches.

#![cfg(all(feature = "fpga", feature = "verilog", feature = "synth"))]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use reticle::fpga::xray::{Decoded, XrayDatabase, XrayOptions, is_pip_feature};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a35t-cpg236";
const IDCODE: u32 = 0x0362_d093;

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
/// The same lookup as `tests/fpga_xray.rs`, which the binary's own test
/// keeps in step with the pinned version.
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

fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    let Ok(root) = std::env::var("RETICLE_CHIPDB") else {
        let pinned = fetched(
            "prjxray-db",
            "0a0addedd73e7e4139d52a6d8db4258763e0f1f3",
            probe,
        );
        if pinned.is_none() {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
        }
        return pinned;
    };
    if !Path::new(&format!("{root}/{probe}")).exists() {
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but it has no `{probe}`");
        return None;
    }
    Some(root)
}

/// The feature names of one tile type's `segbits` file, and which of them
/// are pips, read here rather than through the loader so the check does
/// not lean on the code it checks.
struct Segbits {
    pips: HashSet<String>,
    /// Feature name to its `one` bits, as written.
    ones: HashMap<String, Vec<String>>,
}

fn segbits(root: &str, tile_type: &str) -> Segbits {
    let path = format!("{root}/artix7/segbits_{}.db", tile_type.to_lowercase());
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut names = Vec::new();
    let mut ones = HashMap::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(full) = words.next() else { continue };
        let Some((_, name)) = full.split_once('.') else {
            continue;
        };
        let bits: Vec<String> = words
            .filter(|w| !w.starts_with('!') && w.contains('_'))
            .map(str::to_owned)
            .collect();
        names.push(name.to_owned());
        ones.insert(name.to_owned(), bits);
    }
    let sites: HashSet<String> = names
        .iter()
        .filter(|n| n.split('.').count() >= 3)
        .filter_map(|n| n.split('.').next().map(str::to_owned))
        .collect();
    let pips = names
        .iter()
        .filter(|n| is_pip_feature(n, &sites))
        .cloned()
        .collect();
    Segbits { pips, ones }
}

/// One `RAM64X1D` as it was placed and what it was wired to.
#[derive(Debug)]
struct PlacedRam {
    /// The tile name, `CLBLM_L_X10Y30`.
    tile: String,
    /// The bel.
    bel: String,
    /// Role to the database wire the router delivered it to, and the
    /// primitive that drives it.
    pins: Vec<(String, String, String)>,
    /// Whether the route to `wclk` ran on a global clock wire.
    wclk_global: bool,
}

/// What building the demo produced.
struct Built {
    decoded: Decoded,
    decoded_pips: BTreeSet<(String, String)>,
    routed_pips: BTreeSet<(String, String)>,
    /// Routed pips with bits that are not a `segbits` pip: a fixed path
    /// through a site, which carries a bel feature instead.
    through_sites: usize,
    rams: Vec<PlacedRam>,
    /// Every placed lookup table and flip-flop, by tile and bel.
    others: Vec<(String, String)>,
    /// The lutram site's blocks, by bel name, for one RAM.
    blocks: Vec<String>,
    signals: (usize, usize),
    lutram_sites: usize,
    lutram_tile_types: BTreeSet<String>,
}

/// Builds `examples/basys3/lutram.v` exactly as `reticle fpga --bitstream`
/// does, with `init` (if any) given to the first RAM as its `INIT`.
fn build(root: &str, init: Option<i64>) -> Option<Built> {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::xc7::{self, BitHeader};
    use reticle::fpga::{
        Constraints, FpgaOptions, Netlist, RouteOptions, bitstream, route, synthesize_for, target,
    };
    use reticle::ir::AttrValue;
    use reticle::source::SourceMap;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("lutram.v")),
        std::fs::read_to_string(dir.join("lutram.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return None;
    };

    let mut map = SourceMap::new();
    let source = map.add("lutram.v", &verilog).unwrap();
    let rcf_file = map.add("lutram.rcf", &rcf).unwrap();
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

    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    let region = db.region_for_pins(&DiskFiles, &pins, 12).unwrap().unwrap();
    let region = db
        .region_with_site_type(&DiskFiles, region, "BUFGCTRL")
        .unwrap()
        .unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(region))
        .unwrap();
    fabric.check_idcode(IDCODE).unwrap();
    let graph = fabric.arch.build_graph();

    let mut lutram_tile_types = BTreeSet::new();
    for tile_type in &fabric.arch.tile_types {
        if tile_type.bels.iter().any(|b| b.kind == "lutram") {
            lutram_tile_types.insert(tile_type.name.clone());
        }
    }
    let lutram_sites = graph.sites.iter().filter(|s| s.kind == "lutram").count();

    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let (routing, _) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .expect("the distributed RAM design does not route");
    let problems = routing.verify(&netlist, &graph, &placement);
    assert!(problems.is_empty(), "{problems:?}");

    // Tile names by grid position, from the database itself.
    let tiles = db.tiles(&DiskFiles).unwrap();
    let mut name_at: HashMap<(u32, u32), (String, String)> = HashMap::new();
    for tile in &tiles {
        name_at.insert(
            (tile.grid_x, tile.grid_y),
            (tile.name.clone(), tile.tile_type.clone()),
        );
    }

    if let Some(value) = init {
        let first = netlist
            .instances
            .iter()
            .find(|i| i.primitive == "RAM64X1D")
            .expect("the demo has a RAM");
        design.modules[top].cells[first.cell]
            .params
            .set("INIT", AttrValue::Int(value));
    }

    let mut rams = Vec::new();
    let mut others = Vec::new();
    let mut blocks = Vec::new();
    for (index, instance) in netlist.instances.iter().enumerate() {
        let Some(site) = placement.site_of(index) else {
            panic!("{} unplaced", instance.name);
        };
        let site = &graph.sites[site];
        let (tile, _) = name_at[&site.tile].clone();
        if instance.kind != "lutram" {
            if instance.kind == "lut" || instance.kind == "ff" {
                others.push((tile, site.bel.clone()));
            }
            continue;
        }
        if blocks.is_empty() {
            blocks = site
                .blocks
                .iter()
                .map(|b| {
                    let other = &graph.sites[*b];
                    assert_eq!(other.tile, site.tile, "a block outside the tile");
                    other.bel.clone()
                })
                .collect();
        }
        let mut wired = Vec::new();
        let mut wclk_global = false;
        for &pin in &instance.pins {
            let pin = &netlist.pins[pin];
            if !matches!(pin.role.as_str(), "we" | "din" | "wclk") {
                continue;
            }
            let signal = pin.signal.expect("a RAM control pin is connected");
            let driver = netlist.signals[signal].driver.expect("and driven");
            let driver = netlist.instances[netlist.pins[driver].instance]
                .primitive
                .clone();
            let node = site.pin(&pin.role).expect("the pin has a wire");
            let route = routing.route(signal).expect("and is routed");
            assert!(route.nodes.binary_search(&node).is_ok());
            if pin.role == "wclk" {
                wclk_global = route.nodes.iter().any(|n| {
                    let name = &graph.wire(*n).name;
                    name.contains("GCLK") || name.starts_with("HCLK_CK_BUFHCLK")
                });
            }
            wired.push((pin.role.clone(), graph.wire(node).name.clone(), driver));
        }
        wired.sort();
        rams.push(PlacedRam {
            tile,
            bel: site.bel.clone(),
            pins: wired,
            wclk_global,
        });
    }

    let mut tiles_out = bitstream::generate(
        &design,
        top,
        &fabric.arch,
        &graph,
        &netlist,
        &placement,
        &routing,
    )
    .unwrap();
    fabric
        .enable_global_clocks(&graph, &routing, &mut tiles_out)
        .unwrap();
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles_out, &fabric.frames).unwrap();
    let header = BitHeader::new("lutram;UserID=0XFFFFFFFF;Version=reticle", "7a35tcpg236");
    let bytes = xc7::write_bit(&header, &fabric.part, &frames).unwrap();
    let back = xc7::read_bit(&bytes).unwrap();
    assert_eq!(back.idcode, Some(IDCODE));
    for (expected, computed) in &back.crc_checks {
        assert_eq!(expected, computed);
    }
    let decoded = db.decode(&DiskFiles, &frames).unwrap();

    // The interconnect, both ways: what the decoding names as pips, and
    // what the router took that the database names as pips.
    let mut cache: HashMap<String, Segbits> = HashMap::new();
    let mut type_of: HashMap<String, String> = HashMap::new();
    for (name, tile_type) in name_at.values() {
        type_of.insert(name.clone(), tile_type.clone());
    }
    let mut decoded_pips = BTreeSet::new();
    let mut enabled_wires = BTreeSet::new();
    for (tile, feature) in &decoded.features {
        let tile_type = &type_of[tile];
        let seg = cache
            .entry(tile_type.clone())
            .or_insert_with(|| segbits(root, tile_type));
        // `ENABLE_BUFFER.<wire>` is two components with no site prefix,
        // so the pip rule calls it a pip; it is the buffer *on* a clock
        // wire, charged to whichever pip touches that wire (see
        // `sites::wire_enable_features`). It is checked below against the
        // wires the routes touched rather than against the pips.
        if let Some(wire) = feature.strip_prefix("ENABLE_BUFFER.") {
            enabled_wires.insert((tile.clone(), wire.to_owned()));
        } else if seg.pips.contains(feature) {
            decoded_pips.insert((tile.clone(), feature.clone()));
        }
    }
    let mut routed_pips = BTreeSet::new();
    let mut touched_wires = BTreeSet::new();
    let mut through_sites = 0;
    for route in routing.routes() {
        for &id in &route.pips {
            if graph.pip_bits(id).is_empty() {
                continue;
            }
            let pip = graph.pip(id);
            let (tile, tile_type) = name_at[&pip.tile].clone();
            for node in [pip.from, pip.to] {
                touched_wires.insert((tile.clone(), graph.wire(node).name.clone()));
            }
            let feature = format!("{}.{}", graph.wire(pip.to).name, graph.wire(pip.from).name);
            let seg = cache
                .entry(tile_type.clone())
                .or_insert_with(|| segbits(root, &tile_type));
            if seg.pips.contains(&feature) && !seg.ones[&feature].is_empty() {
                routed_pips.insert((tile, feature));
            } else {
                through_sites += 1;
            }
        }
    }

    let unrouted_enables: Vec<_> = enabled_wires.difference(&touched_wires).collect();
    assert!(
        unrouted_enables.is_empty(),
        "a clock buffer enabled on a wire no route touches: {unrouted_enables:?}"
    );

    Some(Built {
        decoded,
        decoded_pips,
        routed_pips,
        through_sites,
        rams,
        others,
        blocks,
        signals: (routing.routed(), netlist.signals.len()),
        lutram_sites,
        lutram_tile_types,
    })
}

/// The value the first RAM is given as its `INIT`: bits 0, 5 and 62.
const INIT: i64 = 0x4000_0000_0000_0021;

/// One build shared by every test here, because a build loads a band of
/// the die and takes a while in a debug build. The first RAM carries
/// [`INIT`]; nothing else differs from what `reticle fpga` writes.
fn built() -> Option<&'static Built> {
    static BUILT: std::sync::OnceLock<Option<Built>> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| chipdb().and_then(|root| build(&root, Some(INIT))))
        .as_ref()
}

/// **The milestone for this track**: a design with a distributed RAM
/// reaches a `.bit` for the XC7A35T, every set bit decodes back into a
/// feature the database names, and the interconnect the decoding finds is
/// exactly the interconnect the router chose.
///
/// It would catch a RAM that does not place (the original failure:
/// "the design needs 4 lutram site(s) and the part has 0"), one that
/// places but cannot route, a bit the RAM's bel sets that the database
/// cannot name, and a pip set or missed. It would **not** catch a wrong
/// address permutation, which decodes just the same.
#[test]
fn a_distributed_ram_reaches_a_bit_file_and_every_bit_decodes() {
    let Some(built) = built() else { return };
    eprint!("{}", built.decoded.to_text());
    eprintln!(
        "{} of {} signal(s) routed; {} interconnect pip(s) routed, {} decoded; \
         {} fixed path(s) through a site",
        built.signals.0,
        built.signals.1,
        built.routed_pips.len(),
        built.decoded_pips.len(),
        built.through_sites
    );
    assert_eq!(built.signals.0, built.signals.1, "not every signal routed");
    assert_eq!(
        built.decoded.unexplained, 0,
        "a bit this flow set has no name"
    );
    assert_eq!(built.decoded.tiles_without_a_segbits_file, 0);
    let missing: Vec<_> = built.routed_pips.difference(&built.decoded_pips).collect();
    let extra: Vec<_> = built.decoded_pips.difference(&built.routed_pips).collect();
    assert!(missing.is_empty(), "routed and not decoded: {missing:?}");
    assert!(extra.is_empty(), "decoded and not routed: {extra:?}");
    assert!(!built.routed_pips.is_empty());

    // And the RAM's own configuration, in the database's vocabulary: the
    // two lookup tables in RAM mode, and nothing that would make it a
    // 32-word, shifting, cascaded or CE-enabled one.
    assert_eq!(built.rams.len(), 4, "four bits wide, one RAM64X1D a bit");
    for ram in &built.rams {
        let here = built.decoded.at(&ram.tile);
        eprintln!("{} {}: {here:?}", ram.tile, ram.bel);
        for wanted in ["SLICEM_X0.DLUT.RAM", "SLICEM_X0.CLUT.RAM"] {
            assert!(here.contains(&wanted), "{}: no {wanted}", ram.tile);
        }
        for unwanted in [
            "SLICEM_X0.ALUT.RAM",
            "SLICEM_X0.BLUT.RAM",
            "SLICEM_X0.DLUT.SMALL",
            "SLICEM_X0.CLUT.SMALL",
            "SLICEM_X0.DLUT.SRL",
            "SLICEM_X0.CLUT.SRL",
            "SLICEM_X0.WA7USED",
            "SLICEM_X0.WA8USED",
            "SLICEM_X0.WEMUX.CE",
            "SLICEM_X0.CLKINV",
            "SLICEM_X0.CLUT.DI1MUX.CI",
        ] {
            assert!(!here.contains(&unwanted), "{}: {unwanted}", ram.tile);
        }
    }
}

/// **Only a `SLICEM` holds one**, and holding one costs exactly its `D`
/// and `C` lookup tables.
///
/// Every `lutram` site of the loaded region is on a `CLBLM` tile type and
/// named after its `SLICEM`; no `CLBLL` offers one; the region offers one
/// per `CLBLM` tile, not zero (the original failure) and not two. The
/// RAM's blocks are its own slice's `DLUT` and `CLUT` and nothing else, so
/// the `A` and `B` lookup tables, all eight flip-flops and the whole
/// `SLICEL` of the same tile stay usable; and no lookup table this
/// placement put down sits on a blocked bel of a RAM's tile.
///
/// It would catch a `SLICEL` offered as a RAM, a RAM that blocks too much
/// (which no decoding would notice — the design would just use more
/// tiles) or too little, and the placer ignoring the blocks. It would not
/// catch the two lookup tables being the wrong two: that is the quoted
/// part, pinned in `src/fpga/xray/lutram.rs`'s own test.
#[test]
fn a_ram_takes_two_lookup_tables_of_a_slicem_and_nothing_else() {
    let Some(built) = built() else { return };
    assert!(built.lutram_sites > 0, "the region offers no lutram site");
    assert_eq!(
        built.lutram_tile_types.iter().collect::<Vec<_>>(),
        vec!["CLBLM_L", "CLBLM_R"],
        "only a CLBLM has a SLICEM"
    );
    for ram in &built.rams {
        assert!(ram.tile.starts_with("CLBLM_"), "{ram:?}");
        assert_eq!(ram.bel, "SLICEM_X0_RAM64X1D");
    }
    assert_eq!(built.blocks, vec!["SLICEM_X0_DLUT", "SLICEM_X0_CLUT"]);
    let ram_tiles: HashSet<&str> = built.rams.iter().map(|r| r.tile.as_str()).collect();
    for (tile, bel) in &built.others {
        if ram_tiles.contains(tile.as_str()) {
            assert!(
                !built.blocks.contains(bel),
                "{bel} of {tile} holds a lookup table and a RAM"
            );
        }
    }
    let shared = built
        .others
        .iter()
        .filter(|(tile, _)| ram_tiles.contains(tile.as_str()))
        .count();
    eprintln!(
        "{} lutram site(s) in the region; {shared} lookup table(s) or flip-flop(s) \
         share a tile with a RAM",
        built.lutram_sites
    );
}

/// **The write port's three control wires.** `we`, `din` and `wclk` reach
/// `CLBLM_M_WE`, `CLBLM_M_DI` and `CLBLM_M_CLK` — the wires
/// `ppips_clblm_*.db` feeds from `FAN4`, `FAN3` and `CLK1` — driven by the
/// button's synchroniser, a switch's input buffer and the global clock
/// buffer; and the clock arrives on a global clock wire, never through
/// data routing.
///
/// It would catch a pin table naming the wrong wire (`CI` instead of `DI`,
/// `CE` instead of `WE`), a clock reaching a RAM off general routing (the
/// fault the ECP5's distributed RAM had), and the mapper wiring a control
/// pin to the wrong net. It would not catch the slice reading `WE` from
/// the `CE` pin instead: that is `WEMUX.CE`, which the first test holds
/// clear, and whose meaning is the fuzzer's.
#[test]
fn the_write_enable_data_and_clock_arrive_where_the_database_says() {
    let Some(built) = built() else { return };
    for ram in &built.rams {
        let wired: Vec<(&str, &str, &str)> = ram
            .pins
            .iter()
            .map(|(r, w, d)| (r.as_str(), w.as_str(), d.as_str()))
            .collect();
        assert_eq!(
            wired,
            vec![
                ("din", "CLBLM_M_DI", "IBUF"),
                ("wclk", "CLBLM_M_CLK", "BUFG"),
                ("we", "CLBLM_M_WE", "FDRE"),
            ],
            "{ram:?}"
        );
        assert!(
            ram.wclk_global,
            "{}: the write clock is off data wires",
            ram.tile
        );
    }
}

/// **The contents.** An `INIT` given to one RAM lands, bit for bit and
/// unpermuted, in **both** of its lookup tables — the write port's copy
/// and the read port's — and nowhere else.
///
/// Today the flow never emits an `INIT` (the mapper declines a memory with
/// initial contents rather than load it), so this is the path a future
/// one takes, set by hand on the netlist. It would catch the contents
/// going to one copy only (a RAM that reads back its initial value only on
/// the port that is not used), going to the wrong letter, or being
/// permuted. It would not catch the identity being the wrong order in the
/// first place: that is the same quoted reading as the address order, and
/// the same board test.
#[test]
fn an_init_lands_unpermuted_in_both_copies() {
    let Some(built) = built() else { return };
    assert_eq!(built.decoded.unexplained, 0);
    let mut seen = 0;
    for ram in &built.rams {
        let inits: Vec<&str> = built
            .decoded
            .at(&ram.tile)
            .into_iter()
            .filter(|f| {
                f.starts_with("SLICEM_X0.DLUT.INIT[") || f.starts_with("SLICEM_X0.CLUT.INIT[")
            })
            .collect();
        if inits.is_empty() {
            continue;
        }
        seen += 1;
        assert_eq!(
            inits,
            vec![
                "SLICEM_X0.CLUT.INIT[00]",
                "SLICEM_X0.CLUT.INIT[05]",
                "SLICEM_X0.CLUT.INIT[62]",
                "SLICEM_X0.DLUT.INIT[00]",
                "SLICEM_X0.DLUT.INIT[05]",
                "SLICEM_X0.DLUT.INIT[62]",
            ],
            "{}",
            ram.tile
        );
    }
    assert_eq!(seen, 1, "exactly the one RAM given an INIT has one");
}
