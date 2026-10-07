//! Block RAM on the Xilinx 7-series fabric, against the **real** Project
//! X-Ray database.
//!
//! Every test skips without the database, exactly as `tests/fpga_xray.rs`
//! does and for the same reason: it is 45 MB of public-domain data this
//! repository does not vendor and CI does not have. `reticle fetch
//! prjxray-db` puts the pinned copy where these tests find it.
//!
//! What these tests can and cannot reach is said test by test. None of
//! them says a block RAM works on silicon; only a board can, and
//! `examples/basys3/bram_rom.v` is the design for that.

#![cfg(feature = "fpga")]

use std::path::Path;

use reticle::fpga::xray::{GridRegion, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a35t-cpg236";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same search as `tests/fpga_xray.rs`: `RETICLE_CHIPDB`, then the pinned
/// copy in the per-user cache.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{probe}` is not under it");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// The block RAM tile `BRAM_L_X6Y20` (grid 19, 135) and the interconnect
/// around it.
fn bram_region() -> GridRegion {
    GridRegion::new(14, 126, 24, 140)
}

/// A `BRAM_L` tile type gets its two 18 kbit halves as `bram` bels, each
/// with a wire for every pin `xc7.dev`'s `RAMB18E1` can ask for.
///
/// Would catch the two halves going back to being read as wires (which
/// gave a `BRAM_L` no bel at all, the original reason a block RAM did not
/// route), a pin whose wire the database has not got, and a role the
/// netlist would produce that the bel does not offer. Would not catch a
/// role on the wrong wire: the wire *names* are what `ppips_bram_l.db`
/// spells, and which site pin a name is, is quoted.
#[test]
fn block_ram_halves_become_bels_with_pins() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(bram_region()))
        .unwrap();
    assert_eq!(fabric.stats.coverage.pins_unresolved, 0);
    let ty = fabric
        .arch
        .tile_types
        .iter()
        .find(|t| t.name == "BRAM_L")
        .expect("the region holds a BRAM_L");
    // 28 configuration frames on CLB_IO_CLK and 128 contents frames on
    // BLOCK_RAM, stacked; 10 words of each.
    assert_eq!((ty.bit_rows, ty.bit_cols), (28 + 128, 320));

    let device = reticle::fpga::target(DEVICE).unwrap();
    let bram = device
        .block_rams
        .iter()
        .find(|b| b.name == "RAMB18E1")
        .expect("xc7.dev declares RAMB18E1");
    let mut wanted: Vec<String> = Vec::new();
    for (index, port) in bram.port_map.iter().enumerate() {
        for (role, _) in &port.signals {
            let width = match role.as_str() {
                "addr" => 14,
                "din" | "dout" => 16,
                "we" => port.width(role),
                _ => 1,
            };
            for bit in 0..width {
                if width == 1 {
                    wanted.push(format!("p{index}_{role}"));
                } else {
                    wanted.push(format!("p{index}_{role}{bit}"));
                }
            }
        }
    }
    for name in ["RAMB18_Y0", "RAMB18_Y1"] {
        let bel = ty.bel(name).unwrap_or_else(|| panic!("no bel {name}"));
        assert_eq!(bel.kind, "bram");
        assert_eq!(bel.pins.len(), 122, "{name}");
        for role in &wanted {
            assert!(
                bel.pins.iter().any(|(r, _)| r == role),
                "{name} has no pin for `{role}`"
            );
        }
    }
    let graph = fabric.arch.build_graph();
    let brams = graph
        .site_counts()
        .into_iter()
        .find(|(kind, _)| kind == "bram")
        .map_or(0, |(_, n)| n);
    assert!(brams >= 2, "{:?}", graph.site_counts());
}

/// The two pieces of evidence for which half is `RAMB18_Y0`, re-read from
/// the database so they cannot drift from `sites::block_ram_half`: every
/// `BRAM_L` tile's even-`Y` `RAMB18` site is the lower one (typed
/// `FIFO18E1`), and every contents bit of `RAMB18_Y0` lies in the low 176
/// bits of the tile's frame window, `RAMB18_Y1`'s in the high ones.
///
/// Would catch a database in which those two disagree. It is not a
/// measurement against Vivado: the parity rule itself is prjxray's
/// `segmaker.py`, quoted.
#[test]
fn the_lower_half_is_ramb18_y0_by_both_readings() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let tiles = db.tiles(&DiskFiles).unwrap();
    let mut seen = 0;
    for tile in tiles.iter().filter(|t| t.tile_type.starts_with("BRAM_")) {
        if tile.tile_type != "BRAM_L" && tile.tile_type != "BRAM_R" {
            continue;
        }
        let halves: Vec<&(String, String)> = tile
            .sites
            .iter()
            .filter(|(name, _)| name.starts_with("RAMB18_"))
            .collect();
        assert_eq!(halves.len(), 2, "{}", tile.name);
        let y = |name: &str| -> u32 { name.rsplit_once('Y').unwrap().1.parse().unwrap() };
        let (even, odd) = if y(&halves[0].0) % 2 == 0 {
            (halves[0], halves[1])
        } else {
            (halves[1], halves[0])
        };
        assert_eq!(y(&odd.0), y(&even.0) + 1, "{}", tile.name);
        assert_eq!(even.1, "FIFO18E1", "{}", tile.name);
        assert_eq!(odd.1, "RAMB18E1", "{}", tile.name);
        // CLB_IO_CLK first and BLOCK_RAM second: the stacking order.
        let buses: Vec<&str> = tile.bits.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(buses, ["CLB_IO_CLK", "BLOCK_RAM"], "{}", tile.name);
        seen += 1;
    }
    assert_eq!(seen, 75, "the xc7a50t fabric has 55 BRAM_L and 20 BRAM_R");

    let path = format!("{root}/artix7/segbits_bram_l.block_ram.db");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut count = [0usize; 2];
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(name), Some(bit)) = (words.next(), words.next()) else {
            continue;
        };
        let column: u32 = bit.split_once('_').unwrap().1.parse().unwrap();
        if name.starts_with("BRAM_L.RAMB18_Y0.") {
            assert!(column < 176, "{line}");
            count[0] += 1;
        } else if name.starts_with("BRAM_L.RAMB18_Y1.") {
            assert!(column >= 176, "{line}");
            count[1] += 1;
        }
    }
    // 64 INIT and 8 INITP parameters of 256 bits, per half.
    assert_eq!(count, [72 * 256, 72 * 256]);
}

/// A cell's `INIT_xx` parameters, name and 256 bits low bit first.
#[cfg(all(feature = "verilog", feature = "synth"))]
type Inits = Vec<(String, Vec<bool>)>;

/// What building a design all the way to a `.bit` gave back.
#[cfg(all(feature = "verilog", feature = "synth"))]
struct Built {
    /// The bitstream decoded into the database's feature names.
    decoded: reticle::fpga::xray::Decoded,
    /// Every pip the router or a tie switched on, as `(tile, "DEST.SRC")`.
    arcs: std::collections::BTreeSet<(String, String)>,
    /// Every name of a wire the loaded graph has, per tile name.
    wires: std::collections::HashMap<String, std::collections::HashSet<String>>,
    /// The tile and the bel each `RAMB18E1` was placed on, with the cell's
    /// `INIT_xx` parameters as 256 bits each, low bit first.
    rams: Vec<(String, String, Inits)>,
    /// What configuring the block RAMs did.
    report: reticle::fpga::xray::BlockRamReport,
    /// Signals with a driver and a sink, and how many were routed.
    signals: (usize, usize),
    /// Pips taken through a site (an IO logic hop), whose bits are site
    /// features rather than an arc of their own.
    site_hops: usize,
}

/// Builds a design the way `reticle fpga --bitstream` does: synthesis,
/// the region around the pins grown to a block RAM column, load, place,
/// route, generate, the block RAM pass, frames, `.bit`, read back, decode.
///
/// `half`, when given, moves every placed `RAMB18E1` to that half of the
/// tile it was placed in before routing — the placer has no reason to
/// prefer either and in practice takes `RAMB18_Y0`, so this is how the
/// other half gets built at all.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn build(root: &str, verilog: &str, rcf: &str, half: Option<&str>) -> Built {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::xc7::{self, BitHeader};
    use reticle::fpga::{
        Constraints, FpgaOptions, Netlist, RouteOptions, bitstream, route, synthesize_for, target,
    };
    use reticle::ir::AttrValue;
    use reticle::source::SourceMap;
    use std::collections::{BTreeSet, HashMap, HashSet};

    let mut map = SourceMap::new();
    let source = map.add("design.v", verilog).unwrap();
    let rcf_file = map.add("design.rcf", rcf).unwrap();
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

    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    let region = db.region_for_pins(&DiskFiles, &pins, 12).unwrap().unwrap();
    let region = db
        .region_with_site_type(&DiskFiles, region, "RAMB18E1")
        .unwrap()
        .map(|r| GridRegion::new(r.x0, r.y0.saturating_sub(6), r.x1 + 2, r.y1 + 6))
        .unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(region))
        .unwrap();
    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (mut placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    if let Some(half) = half {
        for (index, instance) in netlist.instances.iter().enumerate() {
            if instance.primitive != "RAMB18E1" {
                continue;
            }
            let here = &graph.sites[placement.site_of(index).unwrap()];
            if here.bel == half {
                continue;
            }
            let there = graph
                .sites
                .iter()
                .position(|s| s.tile == here.tile && s.bel == half)
                .expect("the tile has that half");
            assert!(placement.instance_at(there).is_none(), "that half is taken");
            placement.unplace(index);
            placement.place(index, there);
        }
    }
    let (routing, _) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .expect("the design does not route");
    assert!(routing.verify(&netlist, &graph, &placement).is_empty());

    let mut tiles = bitstream::generate(
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
        .enable_global_clocks(&graph, &routing, &mut tiles)
        .unwrap();
    let report = fabric
        .configure_block_rams(
            &design, top, &graph, &netlist, &placement, &routing, &mut tiles,
        )
        .unwrap();
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
    assert_eq!(frames.ones(), tiles.ones(), "a bit landed twice or nowhere");
    let header = BitHeader::new("bram;UserID=0XFFFFFFFF;Version=reticle", "7a35tcpg236");
    let bytes = xc7::write_bit(&header, &fabric.part, &frames).unwrap();
    let back = xc7::read_bit(&bytes).unwrap();
    assert_eq!(back.frames, frames.words());
    for (expected, computed) in &back.crc_checks {
        assert_eq!(expected, computed);
    }
    let decoded = db.decode(&DiskFiles, &frames).unwrap();

    let all_tiles = db.tiles(&DiskFiles).unwrap();
    let names: HashMap<(u32, u32), String> = all_tiles
        .iter()
        .map(|t| ((t.grid_x, t.grid_y), t.name.clone()))
        .collect();
    let types: HashMap<(u32, u32), String> = all_tiles
        .iter()
        .map(|t| ((t.grid_x, t.grid_y), t.tile_type.clone()))
        .collect();
    // The names a tile type's `segbits` file gives a feature, so that a
    // hop through a site — which costs site features, not an arc of its
    // own — is told apart from an arc.
    let mut segbits: HashMap<String, HashSet<String>> = HashMap::new();
    let mut is_arc = |tile_type: &str, name: &str| -> bool {
        let known = segbits.entry(tile_type.to_owned()).or_insert_with(|| {
            let path = format!("{root}/artix7/segbits_{}.db", tile_type.to_lowercase());
            std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.split_whitespace().next())
                .filter_map(|f| f.split_once('.').map(|(_, rest)| rest.to_owned()))
                .collect()
        });
        known.contains(name)
    };
    let mut site_hops = 0usize;
    let mut arcs = BTreeSet::new();
    let pips = routing
        .routes()
        .flat_map(|r| r.pips.iter().copied())
        .chain(report.tie_pips.iter().copied());
    for id in pips {
        if graph.pip_bits(id).is_empty() {
            continue;
        }
        let pip = graph.pip(id);
        let name = format!("{}.{}", graph.wire(pip.to).name, graph.wire(pip.from).name);
        if !is_arc(&types[&pip.tile], &name) {
            site_hops += 1;
            continue;
        }
        arcs.insert((names[&pip.tile].clone(), name));
    }
    let mut wires: HashMap<String, HashSet<String>> = HashMap::new();
    for wire in &graph.nodes {
        if let Some(name) = names.get(&wire.tile) {
            wires
                .entry(name.clone())
                .or_default()
                .insert(wire.name.clone());
        }
    }

    let mut rams = Vec::new();
    for (index, instance) in netlist.instances.iter().enumerate() {
        if instance.primitive != "RAMB18E1" {
            continue;
        }
        let site = &graph.sites[placement.site_of(index).unwrap()];
        let cell = &design.modules[top].cells[instance.cell];
        let mut inits = Vec::new();
        for (name, value) in cell.params.iter() {
            let name = name.as_str();
            if !name.starts_with("INIT_") || name.len() != 7 {
                continue;
            }
            let AttrValue::Const(value) = value else {
                panic!("{name} is not a constant");
            };
            let bits = (0..256)
                .map(|i| value.get(i) == Some(reticle::logic::Bit::One))
                .collect();
            inits.push((name.to_owned(), bits));
        }
        rams.push((names[&site.tile].clone(), site.bel.clone(), inits));
    }
    let routable = netlist.signals.iter().filter(|s| s.is_routable()).count();
    Built {
        decoded,
        arcs,
        wires,
        rams,
        report,
        signals: (routable, routing.routed()),
        site_hops,
    }
}

/// The arcs a decoding names — every feature `DEST.SRC` whose two halves
/// are wires of its tile — against the ones the build switched on.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn assert_arcs_are_the_routers(built: &Built) -> usize {
    let decoded: std::collections::BTreeSet<(String, String)> = built
        .decoded
        .features
        .iter()
        .filter(|(tile, feature)| {
            let Some((to, from)) = feature.split_once('.') else {
                return false;
            };
            // A leaf clock buffer's enable is spelled like a pip and is
            // charged to the pip that touches its wire; it is not an arc.
            if to == "ENABLE_BUFFER" {
                return false;
            }
            built
                .wires
                .get(tile)
                .is_some_and(|w| w.contains(to) && w.contains(from))
        })
        .cloned()
        .collect();
    let missing: Vec<_> = built.arcs.difference(&decoded).collect();
    let extra: Vec<_> = decoded.difference(&built.arcs).collect();
    assert!(missing.is_empty(), "switched on, not decoded: {missing:?}");
    assert!(extra.is_empty(), "decoded, not switched on: {extra:?}");
    decoded.len()
}

/// The contents a decoding gives one block RAM half, as
/// `"<bel>.INIT_xx[kkk]"`.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn contents(built: &Built, tile: &str, bel: &str) -> std::collections::BTreeSet<String> {
    built
        .decoded
        .at(tile)
        .into_iter()
        .filter(|f| f.starts_with(bel) && f.contains(".INIT"))
        .map(str::to_owned)
        .collect()
}

/// **The demo design**, `examples/basys3/bram_rom.v`, to a `.bit`, and
/// that `.bit` read back through the database.
///
/// What it establishes:
///
/// - every set bit, contents included, is named by a database feature;
/// - the arcs the bits select are exactly the router's plus the ties';
/// - the block RAM half carries the modes nextpnr-xilinx writes for a
///   16-bit true-dual-port block, and no others, and the other half is
///   untouched;
/// - **the contents**: the `INIT_xx[k]` features the bitstream decodes to
///   are exactly the set bits of the cell's `INIT_xx`, and those are the
///   memory's words in the order UG473 gives (word `a` in bits
///   `16 * (a % 16) ..` of `INIT_<a / 16>`), each word `a + 1`.
///
/// What it cannot establish: that the database's position for
/// `INIT_xx[k]` is where silicon reads it (quoted from prjxray's fuzzer),
/// that `RAMB18_Y0` is the even site (quoted from prjxray's segmaker), or
/// that the mode features are all Vivado would set (quoted from
/// nextpnr). The board is the check for those.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_rom_demo_decodes_bit_for_bit() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("bram_rom.v")),
        std::fs::read_to_string(dir.join("bram_rom.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };
    let built = build(&root, &verilog, &rcf, None);
    let decoded = &built.decoded;
    eprint!("{}", decoded.to_text());
    eprint!("{}", built.report.to_text());
    assert_eq!(built.signals.0, built.signals.1, "not every signal routed");
    assert_eq!(decoded.unexplained, 0, "a set bit no feature names");
    assert_eq!(decoded.tiles_without_a_segbits_file, 0);
    let arcs = assert_arcs_are_the_routers(&built);
    eprintln!(
        "{arcs} arc(s) decoded, equal to the {} the router and the ties chose \
         ({} hop(s) through an IO site carry site features instead)",
        built.arcs.len(),
        built.site_hops
    );

    assert_eq!(built.rams.len(), 1);
    let (tile, bel, inits) = &built.rams[0];
    eprintln!("the block RAM is {tile}.{bel}");
    // No route through another block's address cascade: the first build
    // of this design took four address bits up through the four unused
    // blocks below this one. See `bram::is_cascade_input`.
    for (t, f) in &decoded.features {
        assert!(!f.contains("CASC"), "{t}.{f}");
        if t.starts_with("BRAM_") {
            assert_eq!(t, tile, "a block RAM tile nothing was placed in: {t}.{f}");
        }
    }
    let here: Vec<&str> = decoded.at(tile);
    let modes: Vec<&str> = here
        .iter()
        .copied()
        .filter(|f| f.starts_with(bel.as_str()) && !f.contains(".INIT"))
        .collect();
    for mode in [
        "IN_USE",
        "READ_WIDTH_A_18",
        "READ_WIDTH_B_18",
        "WRITE_WIDTH_A_18",
        "WRITE_WIDTH_B_18",
        "ZINV_CLKARDCLK",
        "ZINV_ENBWREN",
        "ZINV_RSTRAMARSTRAM",
        "ZINIT_A[0]",
        "ZSRVAL_B[17]",
    ] {
        let full = format!("{bel}.{mode}");
        assert!(modes.contains(&full.as_str()), "{full} not decoded");
    }
    assert!(!modes.iter().any(|f| f.ends_with("DOA_REG")));
    assert!(!modes.iter().any(|f| f.contains("WRITE_MODE")));
    let other = if bel == "RAMB18_Y0" {
        "RAMB18_Y1"
    } else {
        "RAMB18_Y0"
    };
    assert!(!here.iter().any(|f| f.starts_with(other)), "{here:?}");

    // The contents, both ways round. First what the cell asks for is the
    // ROM: word a is a + 1, in the 16-bit layout.
    let mut expected = std::collections::BTreeSet::new();
    for a in 0..1024u32 {
        let word = if a < 256 { (a + 1) & 0xFF } else { 0 };
        for j in 0..16 {
            if (word >> j) & 1 == 1 {
                let k = 16 * (a % 16) + j;
                expected.insert(format!("{bel}.INIT_{:02X}[{k:03}]", a / 16));
            }
        }
    }
    let mut asked = std::collections::BTreeSet::new();
    for (name, bits) in inits {
        for (k, set) in bits.iter().enumerate() {
            if *set {
                asked.insert(format!("{bel}.{name}[{k:03}]"));
            }
        }
    }
    assert_eq!(asked, expected, "the mapper's INIT is not the ROM");
    // Then what the bitstream says is exactly that.
    let got = contents(&built, tile, bel);
    assert_eq!(got, expected, "the contents bits are not the cell's INIT");
    eprintln!("{} contents bit(s) decode to exactly the ROM", got.len());
}

/// Four block RAMs with different contents, so that both halves of a tile
/// are in use and a contents bit written into the wrong half, or two
/// cells' contents swapped, would show.
///
/// The memories are 4096 by 4 bits, which the mapper puts in the 4-bit
/// mode: four words share each 16-bit row of `INIT_xx`, so the layout is
/// not the 16-bit one the demo checks. For each block the decoded
/// contents must be exactly that cell's `INIT_xx`, and every block RAM
/// tile that decodes anything must hold a placed block. Would not catch
/// the database's bit positions being wrong for both halves alike.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn four_blocks_keep_their_own_contents() {
    let Some(root) = chipdb() else { return };
    let verilog = "module four (input wire clk, input wire [11:0] a, output reg [3:0] w, \
                   output reg [3:0] x, output reg [3:0] y, output reg [3:0] z);
        reg [3:0] m0 [0:4095];
        reg [3:0] m1 [0:4095];
        reg [3:0] m2 [0:4095];
        reg [3:0] m3 [0:4095];
        integer i;
        initial begin
            for (i = 0; i < 4096; i = i + 1) begin
                m0[i] = i[3:0] ^ i[11:8];
                m1[i] = ~i[7:4];
                m2[i] = i[11:8];
                m3[i] = i[5:2] ^ 4'd9;
            end
        end
        always @(posedge clk) begin
            w <= m0[a];
            x <= m1[a];
            y <= m2[a];
            z <= m3[a];
        end
    endmodule";
    let mut rcf = String::from("set_io -io_standard LVCMOS33 clk W5\n");
    let pins = [
        "V17", "V16", "W16", "W17", "W15", "V15", "W14", "W13", "U16", "E19", "U19", "V19",
    ];
    for (i, pin) in pins.iter().enumerate() {
        rcf.push_str(&format!("set_io -io_standard LVCMOS33 a[{i}] {pin}\n"));
    }
    let outputs = [
        ("w", ["W18", "U15", "V14", "V13"]),
        ("x", ["V3", "W3", "U3", "P3"]),
        ("y", ["N3", "P1", "L1", "J1"]),
        ("z", ["L2", "J2", "G2", "H1"]),
    ];
    for (port, pins) in outputs {
        for (i, pin) in pins.iter().enumerate() {
            rcf.push_str(&format!("set_io -io_standard LVCMOS33 {port}[{i}] {pin}\n"));
        }
    }
    let built = build(&root, verilog, &rcf, None);
    eprint!("{}", built.decoded.to_text());
    eprint!("{}", built.report.to_text());
    assert_eq!(built.signals.0, built.signals.1, "not every signal routed");
    assert_eq!(built.decoded.unexplained, 0);
    assert_arcs_are_the_routers(&built);
    assert_eq!(built.rams.len(), 4);

    let mut placed = std::collections::BTreeSet::new();
    for (tile, bel, inits) in &built.rams {
        eprintln!("a block RAM is {tile}.{bel}");
        placed.insert((tile.clone(), bel.clone()));
        let mut asked = std::collections::BTreeSet::new();
        for (name, bits) in inits {
            for (k, set) in bits.iter().enumerate() {
                if *set {
                    asked.insert(format!("{bel}.{name}[{k:03}]"));
                }
            }
        }
        assert!(!asked.is_empty());
        assert_eq!(contents(&built, tile, bel), asked, "{tile}.{bel}");
    }
    for a in 0..4 {
        for b in a + 1..4 {
            assert_ne!(
                built.rams[a].2, built.rams[b].2,
                "two contents are the same"
            );
        }
    }
    let halves: std::collections::BTreeSet<&str> =
        built.rams.iter().map(|(_, bel, _)| bel.as_str()).collect();
    eprintln!("halves used: {halves:?}");
    for (tile, feature) in &built.decoded.features {
        if let Some(half) = feature.split('.').next()
            && half.starts_with("RAMB18_Y")
        {
            assert!(
                placed.contains(&(tile.clone(), half.to_owned())),
                "{tile}.{feature} is in a half nothing was placed in"
            );
        }
    }
}

/// The demo again, with its block RAM moved to the **upper** half,
/// `RAMB18_Y1`, whose wires are `BRAM_RAMB18_*` and whose contents bits
/// are the high words of the frames. The placer takes the lower half on
/// its own, so without this the upper half's pin table and contents
/// bits would never be built.
///
/// Would catch a wrong wire prefix for the upper half (it would not
/// route, or route to the lower half's pins), contents landing in the
/// lower half, and a tie that cannot reach the upper half's write
/// enables (two of which come from `FAN` wires rather than `IMUX`). Would
/// not catch the two halves' names being swapped in the database itself.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_upper_half_holds_the_same_rom() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("bram_rom.v")),
        std::fs::read_to_string(dir.join("bram_rom.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };
    let built = build(&root, &verilog, &rcf, Some("RAMB18_Y1"));
    eprint!("{}", built.decoded.to_text());
    eprint!("{}", built.report.to_text());
    assert_eq!(built.signals.0, built.signals.1, "not every signal routed");
    assert_eq!(built.decoded.unexplained, 0);
    assert_arcs_are_the_routers(&built);
    let (tile, bel, inits) = &built.rams[0];
    assert_eq!(bel, "RAMB18_Y1");
    let mut asked = std::collections::BTreeSet::new();
    for (name, bits) in inits {
        for (k, set) in bits.iter().enumerate() {
            if *set {
                asked.insert(format!("{bel}.{name}[{k:03}]"));
            }
        }
    }
    assert_eq!(asked.len(), 1024);
    assert_eq!(contents(&built, tile, bel), asked);
    assert!(
        !built
            .decoded
            .at(tile)
            .iter()
            .any(|f| f.starts_with("RAMB18_Y0")),
        "the lower half is touched"
    );
}
