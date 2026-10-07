//! A 7-series carry chain, placed and routed on the **real** XC7A35T
//! fabric and read back out of the bitstream.
//!
//! `examples/basys3/blink_carry.v` is a 32-bit `count <= count + 1`. It
//! maps to eight `CARRY4`; these tests build it exactly as `reticle fpga
//! --bitstream` does and check, against Project X-Ray's database, the
//! four things a carry chain needs that nothing else in the flow does:
//!
//! 1. the chain runs up **one column of slices, one half of the tile**,
//!    each `CARRY4` in the slice directly above the one before it;
//! 2. every propagate bit is the output of the lookup table **at the same
//!    position of the same slice**;
//! 3. the first carry in is a constant and the rest arrive on `CIN`, and
//!    the bits for both are the ones the database names;
//! 4. every set bit decodes back into a feature, and the interconnect arcs
//!    the bits select are exactly the ones the router chose.
//!
//! None of it says the counter counts on silicon. That is a board's to
//! say; `docs/fpga-xray.md` records what has been tried.
//!
//! Every test here skips, saying why, without the database — `reticle
//! fetch prjxray-db` puts the pinned copy where they look, and
//! `RETICLE_CHIPDB` names another.

#![cfg(all(feature = "fpga", feature = "verilog", feature = "synth"))]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use reticle::fpga::arch::RoutingGraph;
use reticle::fpga::place::{PlaceOptions, Placement, place};
use reticle::fpga::xray::{Decoded, XrayDatabase, XrayOptions, is_pip_feature, legalise_carries};
use reticle::fpga::{
    Constraints, FpgaOptions, Netlist, RouteOptions, Routing, bitstream, route, synthesize_for,
    target, xc7,
};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a35t-cpg236";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same search `tests/fpga_xray.rs` makes: `RETICLE_CHIPDB`, then the
/// pinned copy in the per-user cache.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a50t/tilegrid.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{root}/{probe}` is not there");
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

/// Everything one build produced, for the tests to look at.
struct Built {
    netlist: Netlist,
    graph: RoutingGraph,
    placement: Placement,
    routing: Routing,
    decoded: Decoded,
    /// Grid position to tile name.
    names: BTreeMap<(u32, u32), String>,
    /// Grid position to tile type.
    types: BTreeMap<(u32, u32), String>,
    root: String,
}

/// Builds `examples/basys3/blink_carry.v` exactly as `reticle fpga
/// --bitstream` does: synthesise, legalise the carries, load the fabric
/// around the pins grown to the nearest `BUFGCTRL`, place, route, write,
/// read back.
fn blink_carry(root: &str) -> Option<Built> {
    built_from(root)
}

/// The one build every test here looks at, made once: placing and
/// routing the counter is most of what these tests cost, and four tests
/// asking four questions of the same bitstream should not pay it four
/// times.
fn built() -> Option<&'static Built> {
    static BUILT: OnceLock<Option<Built>> = OnceLock::new();
    BUILT
        .get_or_init(|| chipdb().and_then(|root| blink_carry(&root)))
        .as_ref()
}

fn built_from(root: &str) -> Option<Built> {
    use reticle::diag::Diagnostics;
    use reticle::source::SourceMap;

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("blink_carry.v")),
        std::fs::read_to_string(dir.join("blink_carry.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return None;
    };
    let mut map = SourceMap::new();
    let source = map.add("blink_carry.v", &verilog).unwrap();
    let rcf_file = map.add("blink_carry.rcf", &rcf).unwrap();
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
    let packing = legalise_carries(&mut design, top, device).unwrap();
    eprint!("{}", packing.to_text());
    assert_eq!(packing.carries, 8, "32 bits are eight CARRY4");
    assert_eq!(packing.dead_lanes, 0, "32 bits fill eight CARRY4 exactly");

    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    let region = db.region_for_pins(&DiskFiles, &pins, 12).unwrap().unwrap();
    let region = db
        .region_with_site_type(&DiskFiles, region, "BUFGCTRL")
        .unwrap()
        .unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(region);
    let fabric = db.load(&DiskFiles, &options).unwrap();
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
    eprint!("{}", report.to_text());
    let (routing, routing_report) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .expect("the carry counter does not route");
    eprint!("{}", routing_report.to_text());
    assert!(
        routing.verify(&netlist, &graph, &placement).is_empty(),
        "{:?}",
        routing.verify(&netlist, &graph, &placement)
    );
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
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();

    let mut names = BTreeMap::new();
    let mut types = BTreeMap::new();
    for tile in db.tiles(&DiskFiles).unwrap() {
        names.insert((tile.grid_x, tile.grid_y), tile.name.clone());
        types.insert((tile.grid_x, tile.grid_y), tile.tile_type.clone());
    }
    Some(Built {
        netlist,
        graph,
        placement,
        routing,
        decoded,
        names,
        types,
        root: root.to_owned(),
    })
}

impl Built {
    /// Every `CARRY4`, in chain order: the one whose carry in is a
    /// constant first, then each one whose `CI` the previous one's `CO[3]`
    /// drives.
    fn chain(&self) -> Vec<usize> {
        let carries: Vec<usize> = (0..self.netlist.instances.len())
            .filter(|i| self.netlist.instances[*i].primitive == "CARRY4")
            .collect();
        let ci_driver = |i: usize| -> Option<usize> {
            let pin = self.netlist.instances[i]
                .pins
                .iter()
                .map(|p| &self.netlist.pins[*p])
                .find(|p| p.role == "ci")?;
            let signal = pin.signal?;
            let driver = self.netlist.signals[signal].driver?;
            Some(self.netlist.pins[driver].instance)
        };
        let mut order: Vec<usize> = carries
            .iter()
            .copied()
            .filter(|i| ci_driver(*i).is_none())
            .collect();
        assert_eq!(order.len(), 1, "one chain, so one carry with no carry in");
        while let Some(next) = carries
            .iter()
            .copied()
            .find(|i| ci_driver(*i) == order.last().copied())
        {
            order.push(next);
        }
        assert_eq!(
            order.len(),
            carries.len(),
            "every CARRY4 is on the one chain"
        );
        order
    }

    /// The tile and bel an instance was placed on.
    fn site(&self, instance: usize) -> ((u32, u32), &str) {
        let site = &self.graph.sites[self.placement.site_of(instance).unwrap()];
        (site.tile, site.bel.as_str())
    }

    /// The pin of `instance` playing `role`.
    fn pin(&self, instance: usize, role: &str) -> &reticle::fpga::place::NetPin {
        self.netlist.instances[instance]
            .pins
            .iter()
            .map(|p| &self.netlist.pins[*p])
            .find(|p| p.role == role)
            .unwrap_or_else(|| panic!("no `{role}` pin"))
    }
}

/// **The chain runs up one column, in one half of the tile.**
///
/// A `CARRY4`'s `CIN` is joined by `tileconn.json` to the `COUT_N` of the
/// tile one row down the grid — two rows where a clock row's `HCLK_CLB`
/// sits between — and to nothing else, and `CLBLL_LL_CIN` pairs with
/// `CLBLL_LL_COUT_N`, never with `CLBLL_L_COUT_N`. So each link must sit in
/// the same grid column, one tile (or two, across a clock row) up, on the
/// bel of the same name.
///
/// Would catch: a placer that ignores the chain (the router then refuses
/// it, before this runs), one that stacks the chain downwards, or one that
/// changes slice half. Would not catch: a wrong `tileconn` reading shared
/// by the loader and this test — the next test's arcs are that check.
#[test]
fn the_chain_runs_up_one_column_of_one_slice_half() {
    let Some(b) = built() else { return };
    let chain = b.chain();
    assert_eq!(chain.len(), 8);
    for pair in chain.windows(2) {
        let ((x0, y0), bel0) = b.site(pair[0]);
        let ((x1, y1), bel1) = b.site(pair[1]);
        eprintln!(
            "{} {bel0} -> {} {bel1}",
            b.names[&(x0, y0)],
            b.names[&(x1, y1)]
        );
        assert_eq!(x0, x1, "the chain leaves its column");
        assert_eq!(bel0, bel1, "the chain changes slice half");
        assert!(bel0.ends_with("_CARRY4"), "{bel0}");
        let step = y0 - y1;
        let crossing =
            step == 2 && b.types.get(&(x0, y0 - 1)).map(String::as_str) == Some("HCLK_CLB");
        assert!(step == 1 || crossing, "the next CARRY4 is {step} row(s) up");
    }
}

/// **Every propagate bit is the lookup table beside it.**
///
/// The `CARRY4`'s `S[n]` pin is declared on the very wire the lookup
/// table at position `n` drives, so the driver of each propagate signal
/// must sit on `<slice>_<A..D>LUT` of the carry's own tile.
///
/// Would catch: a propagate driven from a flip-flop or from a lookup table
/// elsewhere (the `+ 1` folds 31 of the 32 propagate bits into flip-flop
/// outputs, so without `legalise_carries` there is no lookup table at
/// all), and a lookup table placed at the wrong letter. Would not catch: a
/// truth table that is the wrong function — `the_carry_counter_decodes`
/// checks the tables are buffers and an inverter, and
/// `tests/fpga_carry.rs` checks the logic.
#[test]
fn every_propagate_bit_comes_from_the_lookup_table_beside_it() {
    let Some(b) = built() else { return };
    let mut checked = 0;
    for carry in b.chain() {
        let (tile, bel) = b.site(carry);
        let slice = bel.strip_suffix("_CARRY4").unwrap();
        for (n, letter) in ["A", "B", "C", "D"].iter().enumerate() {
            let pin = b.pin(carry, &format!("p{n}"));
            let signal = pin.signal.expect("every lane of a 32-bit counter is live");
            let driver = b.netlist.pins[b.netlist.signals[signal].driver.unwrap()].instance;
            assert_eq!(b.netlist.instances[driver].primitive, "LUT6");
            let (lut_tile, lut_bel) = b.site(driver);
            assert_eq!(lut_tile, tile, "S[{n}] comes from another tile");
            assert_eq!(lut_bel, format!("{slice}_{letter}LUT"), "S[{n}]");
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}

/// **The first carry in is a constant zero; the rest arrive on `CIN`.**
///
/// The mapper ties the first `CARRY4`'s `CYINIT` and `CI` to zero, which
/// is `PRECYINIT.C0` — every bit of the field clear — so its slice must
/// carry **no** `PRECYINIT` feature at all. Each later one is reached
/// through `PRECYINIT.CIN`, and its carry is routed over the
/// database's own wires: `COUT` → `COUT_N` → (the tile join) → `CIN`.
///
/// Would catch: `C1` or `AX` set on the first slice (a counter that adds
/// two, or one that adds a floating input), a chained slice that does not
/// select `CIN`, and a carry that left the dedicated path for the
/// interconnect. Would not catch: `C1` being wrong when it *is* wanted —
/// nothing the mapper emits ties a carry in to one; the unit tests of
/// `ConfigEntry::Tied` and `xray::sites`' check that `PRECYINIT.C1` is
/// what a tied `cyinit` costs are what pin that.
#[test]
fn the_chain_starts_from_a_constant_and_continues_on_cin() {
    let Some(b) = built() else { return };
    let chain = b.chain();
    for (index, carry) in chain.iter().enumerate() {
        let (tile, bel) = b.site(*carry);
        let slice = bel.strip_suffix("_CARRY4").unwrap();
        let name = &b.names[&tile];
        let precyinit: Vec<&str> = b
            .decoded
            .at(name)
            .into_iter()
            .filter(|f| f.starts_with(&format!("{slice}.PRECYINIT.")))
            .collect();
        if index == 0 {
            assert!(precyinit.is_empty(), "the first carry in: {precyinit:?}");
            assert_eq!(b.pin(*carry, "cyinit").signal, None);
            continue;
        }
        assert_eq!(
            precyinit,
            vec![format!("{slice}.PRECYINIT.CIN").as_str()],
            "{name}"
        );
        // The route of the carry into this one, wire by wire.
        let signal = b.pin(*carry, "ci").signal.unwrap();
        let route = b.routing.route(signal).expect("the carry is routed");
        let wires: Vec<&str> = route
            .nodes
            .iter()
            .map(|n| b.graph.wire(*n).name.as_str())
            .collect();
        for needle in ["_COUT", "_COUT_N", "_CIN", "_CARRY_CI"] {
            assert!(
                wires.iter().any(|w| w.ends_with(needle)),
                "{needle} missing from {wires:?}"
            );
        }
        assert!(
            wires.len() <= 6,
            "the carry left the dedicated path: {wires:?}"
        );
        assert!(
            !wires
                .iter()
                .any(|w| w.contains("IMUX") || w.contains("LOGIC_OUTS")),
            "the carry went through the interconnect: {wires:?}"
        );
    }
}

/// **Every set bit decodes, and the arcs are the router's.**
///
/// The decoder reads the bitstream back through `segbits_<type>.db` with
/// no knowledge of the placement or the routing. Two things are asserted:
/// that every set bit is one of the `one` bits of a feature that matched,
/// and that the interconnect arcs those features name — the two-part
/// `<to>.<from>` features — are **exactly** the bit-costing pips the
/// router took, in the same tiles, under the same names. The
/// `ENABLE_BUFFER.*` features are two-part as well and are not arcs; they
/// are the clock wires' buffer enables, which ride on a routed pip.
///
/// Would catch: a carry hop charging a feature the database does not
/// have, two features whose bits overlap so that one decodes as another,
/// and a routed pip whose bits select some other arc. Would not catch: a
/// feature that is in the database and means something other than what
/// `xray::sites` says — the quoted fuzzer readings in `docs/fpga-xray.md`
/// are that, and only a board settles it.
#[test]
fn the_carry_counter_decodes_into_exactly_what_was_placed_and_routed() {
    let Some(b) = built() else { return };
    eprint!("{}", b.decoded.to_text());
    assert!(b.decoded.bits > 0);
    assert_eq!(
        b.decoded.unexplained, 0,
        "{} of {} bit(s) belong to no feature the database names",
        b.decoded.unexplained, b.decoded.bits
    );
    let routable = b.netlist.routable().len();
    assert_eq!(
        b.routing.routed(),
        routable,
        "every signal with a sink routes"
    );

    let mut pips_of: BTreeMap<String, HashSet<String>> = BTreeMap::new();

    let mut routed: BTreeSet<(String, String)> = BTreeSet::new();
    for route in b.routing.routes() {
        for id in &route.pips {
            if b.graph.pip_bits(*id).is_empty() {
                continue;
            }
            let pip = b.graph.pip(*id);
            let name = format!(
                "{}.{}",
                b.graph.wire(pip.to).name,
                b.graph.wire(pip.from).name
            );
            if is_pip(&mut pips_of, &b.root, &b.types[&pip.tile], &name) {
                routed.insert((b.names[&pip.tile].clone(), name));
            }
        }
    }
    let mut selected: BTreeSet<(String, String)> = BTreeSet::new();
    let by_name: BTreeMap<&String, &(u32, u32)> = b.names.iter().map(|(k, v)| (v, k)).collect();
    for (tile, feature) in &b.decoded.features {
        if feature.starts_with("ENABLE_BUFFER.") {
            continue;
        }
        if is_pip(&mut pips_of, &b.root, &b.types[by_name[tile]], feature) {
            selected.insert((tile.clone(), feature.clone()));
        }
    }
    let missing: Vec<_> = routed.difference(&selected).collect();
    let extra: Vec<_> = selected.difference(&routed).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "routed but not in the bits: {missing:?}\nin the bits but not routed: {extra:?}"
    );
    assert!(
        routed.len() > 200,
        "{} arcs is not a routed counter",
        routed.len()
    );

    // The carry's own features, by kind, for the record and as a floor.
    let count = |needle: &str| {
        b.decoded
            .features
            .iter()
            .filter(|(_, f)| f.contains(needle))
            .count()
    };
    assert_eq!(
        count(".PRECYINIT.CIN"),
        7,
        "seven links in a chain of eight"
    );
    assert_eq!(count(".PRECYINIT.C1"), 0);
    assert_eq!(count(".PRECYINIT.AX"), 0);
    // A generate from a flip-flop takes the bypass input, which is the
    // cleared state of `<L>CY0`: none is set.
    assert_eq!(
        count(".CARRY4."),
        0,
        "a generate from O5 where none is constant"
    );
    eprintln!(
        "{} bit(s) set, {} unexplained; {} interconnect arc(s), all routed and all decoded; \
         {} flip-flop(s) fed from the sum beside them, {} sum(s) out through the output mux",
        b.decoded.bits,
        b.decoded.unexplained,
        routed.len(),
        count("FFMUX.XOR"),
        count("OUTMUX.XOR")
    );
    assert_eq!(
        count("FFMUX.XOR") + count("OUTMUX.XOR"),
        32,
        "each sum reaches its flip-flop one way or the other"
    );
    // And the lookup tables: thirty-one buffers of `I0` and the inverter
    // that is `count[0] ^ 1`. A buffer sets INIT bits 1, 3, 5, ... and an
    // inverter 0, 2, 4, ...: 32 bits each.
    let inits = b
        .decoded
        .features
        .iter()
        .filter(|(_, f)| f.contains("LUT.INIT["))
        .count();
    assert!(inits >= 32 * 32, "{inits} INIT bit(s)");
}

/// Whether `name` is a pip of `tile_type`, read straight off its
/// `segbits` file with the loader's own rule, and remembered.
fn is_pip(
    cache: &mut BTreeMap<String, HashSet<String>>,
    root: &str,
    tile_type: &str,
    name: &str,
) -> bool {
    cache
        .entry(tile_type.to_owned())
        .or_insert_with(|| {
            let path = format!("{root}/artix7/segbits_{}.db", tile_type.to_lowercase());
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let names: Vec<String> = text
                .lines()
                .filter_map(|l| l.split_whitespace().next())
                .filter_map(|f| f.split_once('.').map(|(_, rest)| rest.to_owned()))
                .collect();
            let sites: HashSet<String> = names
                .iter()
                .filter(|n| n.split('.').count() >= 3)
                .filter_map(|n| n.split('.').next())
                .map(str::to_owned)
                .collect();
            names
                .into_iter()
                .filter(|n| is_pip_feature(n, &sites))
                .collect()
        })
        .contains(name)
}
