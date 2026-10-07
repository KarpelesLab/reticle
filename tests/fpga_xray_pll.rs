//! A 7-series PLL against the **real** fabric database: placed in a clock
//! management tile, routed in and out of it, and its registers written
//! where `prjxray-db` says they live.
//!
//! Every test skips without the database, exactly as `tests/fpga_xray.rs`
//! does, and for the same reason: it is 45 MB of public-domain data this
//! repository does not vendor and CI does not have. `reticle fetch
//! prjxray-db` puts the pinned copy where these tests look.
//!
//! # What none of this reaches
//!
//! **No PLL configured by this flow has run on a part**, and there is no
//! Vivado bitstream with a PLL in the database to compare against: the
//! four harness designs have none. What is checked is that every bit
//! decodes back to a named feature, that the arcs the bits select are the
//! arcs the router chose, that the register fields carry the values
//! XAPP888's arithmetic gives for two different settings, and that the
//! feedback closes where it should. `docs/fpga-xray.md` ("The PLL") says
//! which values are computed and which are quoted.

#![cfg(feature = "fpga")]

use std::path::Path;

use reticle::fpga::Bitstream;
use reticle::fpga::xc7;
use reticle::fpga::xray::{GridRegion, PllSettings, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

// The end-to-end test needs the front end and synthesis; the register test
// does not, and runs with `fpga` alone.
#[cfg(all(feature = "verilog", feature = "synth"))]
use reticle::fpga::xray::{Decoded, is_pip_feature};
#[cfg(all(feature = "verilog", feature = "synth"))]
use std::collections::{BTreeSet, HashMap, HashSet};

const DEVICE: &str = "xc7a35t-cpg236";
#[cfg(all(feature = "verilog", feature = "synth"))]
const IDCODE: u32 = 0x0362_d093;

/// The PLL nearest the Basys 3's oscillator: `PLLE2_ADV_X1Y0`, in the
/// bottom-right clock region, which is the region pin W5's clock-capable
/// input feeds.
const PLL_TILE: &str = "CMT_TOP_L_UPPER_T_X106Y44";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same search as `tests/fpga_xray.rs`: `RETICLE_CHIPDB`, then the
/// pinned copy `reticle fetch` leaves in the cache.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but has no `{probe}`");
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

/// Tile name by grid position, and tile type by tile name.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn tile_names(db: &XrayDatabase) -> (HashMap<(u32, u32), String>, HashMap<String, String>) {
    let tiles = db.tiles(&DiskFiles).unwrap();
    let at = tiles
        .iter()
        .map(|t| ((t.grid_x, t.grid_y), t.name.clone()))
        .collect();
    let kind = tiles
        .iter()
        .map(|t| (t.name.clone(), t.tile_type.clone()))
        .collect();
    (at, kind)
}

/// Every programmable arc of a tile type, `TO.FROM`, read from its
/// `segbits` file with the loader's own rule ([`is_pip_feature`]), so that
/// an arc in a decoding can be told from a bel feature.
///
/// `ENABLE_BUFFER.<wire>` is the one two-part name that rule mistakes for
/// an arc: it is the buffer on a clock wire, charged to whichever pip
/// touches the wire (`docs/fpga-xray.md`), and names no pair of wires. It
/// is left out on both sides of the comparison.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn segbits_arcs(root: &str, tile_type: &str) -> HashSet<String> {
    let path = format!("{root}/artix7/segbits_{}.db", tile_type.to_lowercase());
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let names: Vec<String> = text
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|full| full.split_once('.').map(|(_, rest)| rest.to_owned()))
        .collect();
    let sites: HashSet<String> = names
        .iter()
        .filter(|n| n.split('.').count() >= 3)
        .filter_map(|n| n.split('.').next().map(str::to_owned))
        .collect();
    names
        .into_iter()
        .filter(|n| is_pip_feature(n, &sites) && !n.starts_with("ENABLE_BUFFER."))
        .collect()
}

/// What the flow made of `examples/basys3/pll_blink.v`.
#[cfg(all(feature = "verilog", feature = "synth"))]
struct Built {
    decoded: Decoded,
    /// `(tile, "TO.FROM")` for every routed pip the database names as a
    /// pip feature, which is what the bits *should* say.
    routed_arcs: BTreeSet<(String, String)>,
    /// The same, per signal name.
    arcs_of: HashMap<String, Vec<(String, String)>>,
    signals: (usize, usize),
    pll_bits: usize,
    /// Whether the PLL's `rst` / `pwrdwn` pins carry a routed signal.
    control_routed: bool,
    /// The site the PLL was placed on.
    pll_site: String,
    /// The tile holding it.
    pll_tile: String,
    /// The arcs of the net `LOCKED` drives.
    locked_arcs: Vec<(String, String)>,
}

/// Runs the demo exactly as `reticle fpga --bitstream` does.
#[cfg(all(feature = "verilog", feature = "synth"))]
fn build(root: &str) -> Option<Built> {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::{
        Constraints, FpgaOptions, Netlist, RouteOptions, bitstream, route, synthesize_for, target,
    };
    use reticle::source::SourceMap;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("pll_blink.v")),
        std::fs::read_to_string(dir.join("pll_blink.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return None;
    };
    let mut map = SourceMap::new();
    let source = map.add("pll_blink.v", &verilog).unwrap();
    let rcf_file = map.add("pll_blink.rcf", &rcf).unwrap();
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
    let region = db
        .region_with_site_type(&DiskFiles, region, "PLLE2_ADV")
        .unwrap()
        .unwrap();
    let fabric = db
        .load(&DiskFiles, &XrayOptions::new().with_region(region))
        .unwrap();
    fabric.check_idcode(IDCODE).unwrap();

    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let (routing, report) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .expect("the PLL design does not route");
    eprintln!("{}", report.to_text());
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
    let pll_bits = fabric
        .configure_clock_managers(&design, top, &graph, &netlist, &placement, &mut tiles)
        .unwrap();
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();

    let (at, kind) = tile_names(&db);
    let mut prefixes: HashMap<String, HashSet<String>> = HashMap::new();
    let mut routed_arcs = BTreeSet::new();
    let mut arcs_of: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for route in routing.routes() {
        for id in &route.pips {
            if graph.pip_bits(*id).is_empty() {
                continue;
            }
            let pip = graph.pip(*id);
            let tile = at[&pip.tile].clone();
            let tile_type = kind[&tile].clone();
            let arcs = prefixes
                .entry(tile_type.clone())
                .or_insert_with(|| segbits_arcs(root, &tile_type));
            // A pip with bits that is not a `segbits` arc is a path through
            // a site (a flip-flop's input mux, a clock buffer) or a fixed
            // connection charged for the buffer on its wire; its bits
            // decode as bel or wire features, which `unexplained == 0`
            // already accounts for.
            let name = format!("{}.{}", graph.wire(pip.to).name, graph.wire(pip.from).name);
            if !arcs.contains(&name) {
                continue;
            }
            routed_arcs.insert((tile.clone(), name.clone()));
            arcs_of
                .entry(netlist.signals[route.signal].name.clone())
                .or_default()
                .push((tile, name));
        }
    }

    let pll = netlist
        .instances
        .iter()
        .position(|i| i.primitive == "PLLE2_BASE")
        .expect("the design has a PLL");
    let control_routed = netlist.instances[pll].pins.iter().any(|p| {
        let pin = &netlist.pins[*p];
        (pin.role == "rst" || pin.role == "pwrdwn") && pin.signal.is_some()
    });
    // The lock net carries whatever name synthesis left it — the LED's,
    // once the assignment is folded — so it is found by its driver.
    let locked = netlist.instances[pll]
        .pins
        .iter()
        .map(|p| &netlist.pins[*p])
        .find(|p| p.role == "lock")
        .and_then(|p| p.signal)
        .map(|s| netlist.signals[s].name.clone());
    let locked_arcs = locked
        .and_then(|name| arcs_of.get(&name).cloned())
        .unwrap_or_default();
    let site = &graph.sites[placement.site_of(pll).unwrap()];
    let pll_site = site.name.clone();
    let pll_tile = at[&site.tile].clone();

    Some(Built {
        decoded: db.decode(&DiskFiles, &frames).unwrap(),
        routed_arcs,
        arcs_of,
        signals: (routing.routed(), netlist.signals.len()),
        pll_bits,
        control_routed,
        pll_site,
        pll_tile,
        locked_arcs,
    })
}

/// **The PLL design, end to end.** The oscillator into a `PLLE2_BASE`,
/// its output through a `BUFG` to twenty-four flip-flops, its `LOCKED`
/// out to an LED.
///
/// What it would catch: a PLL pin on a wire that does not reach the
/// fabric (the design would not route); a register field the database
/// does not have (`configure_clock_managers` refuses); a bit set that no
/// feature names; a route whose bits select a different arc from the one
/// the router chose; a feedback loop closed anywhere but inside the tile;
/// a reset left reading the fabric's default one.
///
/// What it would not catch: a register *value* that is wrong but legal —
/// `the_pll_registers_land_where_the_database_says` checks those — or
/// anything about whether the part locks. Nothing here has run on one.
#[test]
#[cfg(all(feature = "verilog", feature = "synth"))]
fn the_pll_design_routes_and_every_bit_decodes() {
    let Some(root) = chipdb() else { return };
    let Some(built) = build(&root) else { return };
    let decoded = &built.decoded;
    eprintln!("{}", decoded.to_text());
    eprintln!("the PLL is on {}", built.pll_site);
    for (tile, feature) in &decoded.features {
        if tile.starts_with("CMT_TOP") || tile.starts_with("HCLK_CMT") {
            eprintln!("  {tile}.{feature}");
        }
    }

    // ---- All of it routes, and every bit has a name. ----
    let (routed, signals) = built.signals;
    assert_eq!(routed, signals, "{routed} of {signals} signal(s) routed");
    assert_eq!(decoded.unexplained, 0, "a bit this flow set has no name");
    assert!(built.pll_bits > 100, "{} PLL bits", built.pll_bits);

    // ---- The arcs the bits select are the arcs the router chose. ----
    let mut decoded_arcs = BTreeSet::new();
    let (_, kind) =
        tile_names(&XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap());
    let mut prefixes: HashMap<String, HashSet<String>> = HashMap::new();
    for (tile, feature) in &decoded.features {
        let tile_type = &kind[tile];
        let arcs = prefixes
            .entry(tile_type.clone())
            .or_insert_with(|| segbits_arcs(&root, tile_type));
        if arcs.contains(feature) {
            decoded_arcs.insert((tile.clone(), feature.clone()));
        }
    }
    let extra: Vec<_> = decoded_arcs.difference(&built.routed_arcs).collect();
    let missing: Vec<_> = built.routed_arcs.difference(&decoded_arcs).collect();
    assert!(
        extra.is_empty() && missing.is_empty(),
        "decoded but not routed: {extra:?}\nrouted but not decoded: {missing:?}"
    );
    eprintln!(
        "{} arc(s) decoded, the same {} the router chose",
        decoded_arcs.len(),
        built.routed_arcs.len()
    );

    // ---- The PLL is on a clock management tile, configured. ----
    //
    // Which of the die's PLLs is the placer's choice. Either side works:
    // the reference reaches a PLL in its own clock region straight from
    // the pad's clock-capable input, or any PLL from a global buffer
    // through the clock row, and both are the `CLKIN1` input mux below.
    assert!(built.pll_site.ends_with("/PLLE2_ADV"), "{}", built.pll_site);
    let tile = built.pll_tile.as_str();
    let side = if tile.starts_with("CMT_TOP_L_") {
        "L"
    } else {
        "R"
    };
    let pll = decoded.at(tile);
    assert!(pll.contains(&"PLLE2_ADV.IN_USE"), "{pll:?}");

    // ---- The reference arrives on the dedicated clock input. ----
    //
    // Not through `CLK_IN1_INT`, which is the fabric's general routing
    // and would put the interconnect's jitter into the loop.
    let clkin = format!("CMT_TOP_R_UPPER_T_PLLE2_CLKIN1.CMT_TOP_{side}_UPPER_T_CLKIN1");
    assert!(pll.contains(&clkin.as_str()), "{pll:?}");
    assert!(
        decoded
            .features
            .iter()
            .any(|(t, f)| t.starts_with("HCLK_CMT") && f.starts_with("HCLK_CMT_MUX_PLLE2_CLKIN1.")),
        "the reference goes through the clock row's PLL input mux"
    );

    // ---- The feedback closes inside the tile. ----
    //
    // `CLKFBOUT` reaches `CMT_TOP_L_CLKFBOUT2IN` unconditionally (a
    // `ppips` line) and the `CLKFBIN` mux selects it, which is the whole
    // of `COMPENSATION = INTERNAL`'s routing. A loop through a global
    // buffer would be `ZHOLD`, whose bits this flow does not set.
    let feedback = built
        .arcs_of
        .iter()
        .find(|(name, _)| name.ends_with("$fb"))
        .map(|(_, arcs)| arcs.clone())
        .expect("the feedback net is routed");
    assert_eq!(
        feedback,
        vec![(
            tile.to_owned(),
            format!("CMT_TOP_R_UPPER_T_PLLE2_CLKFBIN.CMT_TOP_{side}_CLKFBOUT2IN")
        )],
        "the feedback is one arc, inside the clock management tile"
    );
    assert!(pll.contains(&"PLLE2_ADV.COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF"));

    // ---- Reset and power-down: tied low, so unrouted and inverted. ----
    assert!(!built.control_routed);
    assert!(pll.contains(&"PLLE2_ADV.ZINV_RST"), "{pll:?}");
    assert!(pll.contains(&"PLLE2_ADV.ZINV_PWRDWN"), "{pll:?}");

    // ---- LOCKED is a fabric signal: it leaves through the interconnect. ----
    let locked = &built.locked_arcs;
    assert!(
        locked.iter().any(|(t, _)| t.starts_with("INT_")),
        "LOCKED reaches no interconnect: {locked:?}"
    );

    // ---- Both clocks reach a global buffer. ----
    assert!(
        decoded
            .features
            .iter()
            .filter(|(t, f)| t.starts_with("CLK_BUFG_") && f.ends_with(".IN_USE"))
            .count()
            >= 2,
        "two BUFGCTRLs in use"
    );
}

/// The settings both register tests write, with what each one's fields
/// must decode to, worked by hand from XAPP888 rather than by the code
/// under test.
struct Case {
    settings: PllSettings,
    /// Every `PLLE2_ADV.*` feature with bits that must decode, and no
    /// other.
    expected: Vec<String>,
}

fn bits_of(field: &str, value: u64, width: u32) -> Vec<String> {
    (0..width)
        .filter(|i| (value >> i) & 1 == 1)
        .map(|i| format!("PLLE2_ADV.{field}[{i}]"))
        .collect()
}

fn case_a() -> Case {
    // 100 MHz, DIVCLK 1, MULT 8, CLKOUT0 32: the demo's.
    let mut expected: Vec<String> = [
        "IN_USE",
        "COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF",
        "ZINV_RST",
        "ZINV_PWRDWN",
        // A division by one is a bypassed counter, high 1 and low 1.
        "DIVCLK_DIVCLK_HIGH_TIME[0]",
        "DIVCLK_DIVCLK_LOW_TIME[0]",
        "DIVCLK_DIVCLK_NO_COUNT[0]",
        // 8 = 4 high + 4 low.
        "CLKFBOUT_CLKOUT1_HIGH_TIME[2]",
        "CLKFBOUT_CLKOUT1_LOW_TIME[2]",
        "CLKFBOUT_CLKOUT1_OUTPUT_ENABLE[0]",
        // 32 = 16 high + 16 low.
        "CLKOUT0_CLKOUT1_HIGH_TIME[4]",
        "CLKOUT0_CLKOUT1_LOW_TIME[4]",
        "CLKOUT0_CLKOUT1_OUTPUT_ENABLE[0]",
        "FILTREG1_RESERVED[3]",
        "LOCKREG3_RESERVED[0]",
    ]
    .iter()
    .map(|f| format!("PLLE2_ADV.{f}"))
    .collect();
    for n in 1..=5 {
        for f in [
            "CLKOUT1_HIGH_TIME[0]",
            "CLKOUT1_LOW_TIME[0]",
            "CLKOUT2_NO_COUNT[0]",
        ] {
            expected.push(format!("PLLE2_ADV.CLKOUT{n}_{f}"));
        }
    }
    // nextpnr-xilinx's constants for every PLL it writes, which are the
    // table entries for a multiplier of 8.
    expected.extend(bits_of("LKTABLE", 0xB5_BE8F_A401, 40));
    expected.extend(bits_of("TABLE", 0x3B4, 10));
    expected.sort();
    Case {
        settings: PllSettings {
            divclk_divide: 1,
            clkfbout_mult: 8,
            clkout0_divide: 32,
            bandwidth: "OPTIMIZED".to_owned(),
            startup_wait: false,
            clkout0_used: true,
            clkfbout_used: true,
            invert_rst: true,
            invert_pwrdwn: true,
        },
        expected,
    }
}

// The two binary literals below keep f4pga's own field grouping — 5, 5,
// 10, 10, 10 bits and 4, 4, 2 — so that they can be compared with that
// file line for line; regrouping them in fours would hide that.
#[allow(clippy::unusual_byte_groupings)]
fn case_b() -> Case {
    // 100 MHz, DIVCLK 2, MULT 20 (VCO 1000 MHz), CLKOUT0 7, LOW
    // bandwidth, STARTUP_WAIT, a routed reset: every field moves.
    let mut expected: Vec<String> = [
        "IN_USE",
        "COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF",
        "STARTUP_WAIT",
        "ZINV_PWRDWN",
        // 2 = 1 high + 1 low, counting.
        "DIVCLK_DIVCLK_HIGH_TIME[0]",
        "DIVCLK_DIVCLK_LOW_TIME[0]",
        // 20 = 10 high + 10 low; 10 is 0b001010.
        "CLKFBOUT_CLKOUT1_HIGH_TIME[1]",
        "CLKFBOUT_CLKOUT1_HIGH_TIME[3]",
        "CLKFBOUT_CLKOUT1_LOW_TIME[1]",
        "CLKFBOUT_CLKOUT1_LOW_TIME[3]",
        "CLKFBOUT_CLKOUT1_OUTPUT_ENABLE[0]",
        // 7 = 3 high + 4 low, and the odd half cycle is EDGE.
        "CLKOUT0_CLKOUT1_HIGH_TIME[0]",
        "CLKOUT0_CLKOUT1_HIGH_TIME[1]",
        "CLKOUT0_CLKOUT1_LOW_TIME[2]",
        "CLKOUT0_CLKOUT2_EDGE[0]",
        "CLKOUT0_CLKOUT1_OUTPUT_ENABLE[0]",
        "FILTREG1_RESERVED[3]",
        "LOCKREG3_RESERVED[0]",
    ]
    .iter()
    .map(|f| format!("PLLE2_ADV.{f}"))
    .collect();
    for n in 1..=5 {
        for f in [
            "CLKOUT1_HIGH_TIME[0]",
            "CLKOUT1_LOW_TIME[0]",
            "CLKOUT2_NO_COUNT[0]",
        ] {
            expected.push(format!("PLLE2_ADV.CLKOUT{n}_{f}"));
        }
    }
    // The twentieth lines of f4pga's `pll_lktable_lookup` and of the
    // `lookup_low` table in `pll_table_lookup`, copied as written there.
    expected.extend(bits_of(
        "LKTABLE",
        0b11111_11111_0111110100_1111101001_0000000001,
        40,
    ));
    expected.extend(bits_of("TABLE", 0b0010_1100_00, 10));
    expected.sort();
    Case {
        settings: PllSettings {
            divclk_divide: 2,
            clkfbout_mult: 20,
            clkout0_divide: 7,
            bandwidth: "LOW".to_owned(),
            startup_wait: true,
            clkout0_used: true,
            clkfbout_used: true,
            invert_rst: false,
            invert_pwrdwn: true,
        },
        expected,
    }
}

/// **The DRP-derived bits, for two settings.** Writes each into the same
/// clock management tile, reads the bitstream back through the
/// database's own feature names, and compares with a list worked by hand.
///
/// What it would catch: a field at the wrong bit (the database's names
/// carry XAPP888's register layout, so a value in the wrong field decodes
/// as the wrong name); high and low time swapped, or `EDGE` lost, for an
/// odd division; the lock or filter table indexed off by one (multipliers
/// 8 and 20 differ from their neighbours); the `LOW` table read for
/// `OPTIMIZED` or the other way; a stray bit, since the comparison is the
/// whole tile and every bit must decode.
///
/// What it would not catch: XAPP888's tables themselves being wrong, or
/// a reserved field Vivado sets that f4pga and nextpnr-xilinx both leave
/// clear. Those are quoted and only a part or a Vivado bitstream with a
/// PLL in it can check them.
#[test]
fn the_pll_registers_land_where_the_database_says() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let fabric = db
        .load(
            &DiskFiles,
            &XrayOptions::new().with_region(GridRegion::around(106, 112, 3)),
        )
        .unwrap();
    for (label, case) in [("a", case_a()), ("b", case_b())] {
        let mut tiles = Bitstream::empty(reticle::fpga::BitstreamFormat::from_arch(&fabric.arch));
        let set = fabric
            .write_pll((106, 112), &case.settings, &mut tiles)
            .unwrap();
        let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
        let decoded = db.decode(&DiskFiles, &frames).unwrap();
        assert_eq!(decoded.unexplained, 0, "case {label}");
        assert_eq!(decoded.bits, set, "case {label}");
        let mut got: Vec<String> = decoded
            .at(PLL_TILE)
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        got.sort();
        let extra: Vec<_> = got.iter().filter(|f| !case.expected.contains(f)).collect();
        let missing: Vec<_> = case.expected.iter().filter(|f| !got.contains(f)).collect();
        assert!(
            extra.is_empty() && missing.is_empty(),
            "case {label}: decoded but not expected {extra:?}; expected but not decoded {missing:?}"
        );
        assert_eq!(decoded.tiles, 1, "case {label}: one tile and no other");
        eprintln!("case {label}: {set} bit(s), {} feature(s)", got.len());
    }
}
