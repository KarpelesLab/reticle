//! A tristate and a bidirectional pad on the Xilinx 7 series, against
//! the **real** Project X-Ray database.
//!
//! Every test here skips without the database, exactly as
//! `tests/fpga_xray.rs` does and for the same reason: it is public-domain
//! data this repository does not vendor and CI does not have. `reticle
//! fetch prjxray-db` puts the pinned copy where these tests find it.
//!
//! # What these tests establish, and what they cannot
//!
//! For `examples/basys3/pmod_bidir.v` (an `IOBUF`) and a plain tri-stated
//! output (an `OBUFT`), each test builds the design from Verilog to a
//! `.bit`, and then asks the bitstream rather than the flow:
//!
//! - **every set bit decodes**: each one is a bit of a feature
//!   `segbits_<type>.db` names, with nothing left over;
//! - **the arcs it decodes to are exactly the arcs the router chose**: the
//!   set of interconnect pips read back out of the frames equals, as a set,
//!   the set of pips the routes took — so no pip was written that nothing
//!   asked for, and none the router took was lost;
//! - **the pad and its IO logic say what a tristate needs**, feature by
//!   feature, on the half of the tile the package pin is on.
//!
//! What none of it can reach is whether `OLOGIC_Y<n>.ZINV_T1` set really
//! means "`T` not inverted" on silicon. That is quoted from prjxray's
//! `fuzzers/036-iob-ologic` and nextpnr-xilinx, and no Vivado bitstream
//! in `artix7/harness/` has a tristate pin to measure it against.
//! `the_enable_reaches_the_pad_the_right_way_round` checks the whole
//! chain *given* that reading; only a board checks the reading.

#![cfg(all(feature = "fpga", feature = "verilog", feature = "synth"))]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use reticle::fpga::xc7::{self, BitHeader};
use reticle::fpga::xray::{Decoded, XrayDatabase, XrayOptions, is_pip_feature};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a35t-cpg236";
const IDCODE: u32 = 0x0362_d093;
const STREAM_FRAMES: usize = 5420;

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing. The
/// same lookup as `tests/fpga_xray.rs`: `RETICLE_CHIPDB`, then the pinned
/// copy in the per-user cache, and never a download.
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

/// What one build gave back.
struct Built {
    /// The bitstream, decoded through the database.
    decoded: Decoded,
    /// The interconnect pips the routes took, as `(tile, "to.from")`.
    routed_arcs: BTreeSet<(String, String)>,
    /// Pips with bits that are *not* a `segbits` pip: the hops through a
    /// site (`sites::pass_throughs`), as `(tile, "to.from")`.
    site_hops: BTreeSet<(String, String)>,
    /// Signals routed, of signals.
    signals: (usize, usize),
    /// Bits the pull-up pass set.
    pulled: usize,
    /// The synthesised design, for the polarity walk.
    design: reticle::ir::Design,
    top: reticle::ir::ModuleId,
}

/// Builds `verilog` with `rcf` the way `reticle fpga --bitstream` does,
/// and reads the result back.
fn build(root: &str, name: &str, verilog: &str, rcf: &str) -> Built {
    use reticle::diag::Diagnostics;
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::{
        Constraints, FpgaOptions, Netlist, RouteOptions, bitstream, route, synthesize_for, target,
    };
    use reticle::source::SourceMap;

    let mut map = SourceMap::new();
    let source = map.add(format!("{name}.v"), verilog).unwrap();
    let rcf_file = map.add(format!("{name}.rcf"), rcf).unwrap();
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
    let region = db
        .region_for_pins(&DiskFiles, &pins, 12)
        .unwrap()
        .expect("every pin is a site of this package");
    let mut options = XrayOptions::new();
    options.region = Some(region);
    let fabric = db.load(&DiskFiles, &options).unwrap();
    fabric.check_idcode(IDCODE).unwrap();
    eprint!("{}", fabric.stats.to_text());

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
    assert_eq!(report.fixed, pins.len(), "a constrained pin is not held");
    let (routing, routing_report) = route(&netlist, &graph, &placement, &RouteOptions::default())
        .unwrap_or_else(|e| panic!("{name} does not route: {e}"));
    eprintln!("{}", routing_report.to_text());
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
    let pulled = fabric
        .apply_pullups(&design, top, &graph, &netlist, &placement, &mut tiles)
        .unwrap();
    let frames = xc7::frames_from_bitstream(&fabric.part, &tiles, &fabric.frames).unwrap();
    let header = BitHeader::new(
        format!("{name};UserID=0XFFFFFFFF;Version=reticle"),
        "7a35tcpg236",
    );
    let bytes = xc7::write_bit(&header, &fabric.part, &frames).unwrap();
    let back = xc7::read_bit(&bytes).unwrap();
    assert_eq!(back.idcode, Some(IDCODE));
    assert_eq!(back.frame_count(), STREAM_FRAMES);
    for (expected, computed) in &back.crc_checks {
        assert_eq!(expected, computed);
    }
    let decoded = db.decode(&DiskFiles, &frames).unwrap();

    // Every pip the routes took, named the way `segbits` names it. A pip
    // with no bits is metal (a `tileconn` join or an `always` ppip) and
    // writes nothing, so it can never be read back and is not counted.
    let names: HashMap<(u32, u32), (String, String)> = db
        .tiles(&DiskFiles)
        .unwrap()
        .into_iter()
        .map(|t| ((t.grid_x, t.grid_y), (t.name, t.tile_type)))
        .collect();
    let mut segbits = Segbits::new(root);
    let mut routed_arcs = BTreeSet::new();
    let mut site_hops = BTreeSet::new();
    for route in routing.routes() {
        for id in &route.pips {
            if graph.pip_bits(*id).is_empty() {
                continue;
            }
            let pip = graph.pip(*id);
            let (tile, tile_type) = &names[&pip.tile];
            let arc = format!("{}.{}", graph.wire(pip.to).name, graph.wire(pip.from).name);
            if segbits.is_pip(tile_type, &arc) {
                routed_arcs.insert((tile.clone(), arc));
            } else {
                site_hops.insert((tile.clone(), arc));
            }
        }
    }

    Built {
        decoded,
        routed_arcs,
        site_hops,
        signals: (routing.routed(), netlist.signals.len()),
        pulled,
        design,
        top,
    }
}

/// The feature names of each tile type's `segbits` file, read once, so
/// a decoded feature can be told apart as a pip or a site setting by the
/// loader's own rule ([`is_pip_feature`]).
struct Segbits {
    root: String,
    by_type: HashMap<String, (HashSet<String>, HashSet<String>)>,
}

impl Segbits {
    fn new(root: &str) -> Segbits {
        Segbits {
            root: root.to_owned(),
            by_type: HashMap::new(),
        }
    }

    fn load(&mut self, tile_type: &str) -> &(HashSet<String>, HashSet<String>) {
        let root = self.root.clone();
        self.by_type.entry(tile_type.to_owned()).or_insert_with(|| {
            let path = format!("{root}/artix7/segbits_{}.db", tile_type.to_lowercase());
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let mut names = HashSet::new();
            let mut heads = HashSet::new();
            for line in text.lines() {
                let Some(full) = line.split_whitespace().next() else {
                    continue;
                };
                let Some((_, name)) = full.split_once('.') else {
                    continue;
                };
                if name.split('.').count() > 2 {
                    heads.insert(name.split('.').next().unwrap_or("").to_owned());
                }
                names.insert(name.to_owned());
            }
            (names, heads)
        })
    }

    fn is_pip(&mut self, tile_type: &str, name: &str) -> bool {
        let (names, heads) = self.load(tile_type);
        names.contains(name) && is_pip_feature(name, heads)
    }
}

/// The type of a tile, from its name: `INT_L_X0Y25` is `INT_L`.
fn type_of(tile: &str) -> &str {
    tile.rsplit_once('_').map_or(tile, |(head, _)| head)
}

/// The two invariants every bitstream from this flow is held to: every
/// set bit is named, and the pips the names say are on are exactly the
/// pips the router took.
fn assert_decodes_to_the_routing(root: &str, name: &str, built: &Built) {
    let decoded = &built.decoded;
    eprintln!("{name}: {}", decoded.to_text().trim_end());
    for (tile, feature) in &decoded.features {
        eprintln!("  {tile}.{feature}");
    }
    assert_eq!(
        decoded.unexplained, 0,
        "{name}: a bit this flow set has no name in the database"
    );
    assert_eq!(decoded.tiles_without_a_segbits_file, 0, "{name}");

    let mut segbits = Segbits::new(root);
    let decoded_arcs: BTreeSet<(String, String)> = decoded
        .features
        .iter()
        .filter(|(tile, feature)| segbits.is_pip(type_of(tile), feature))
        .cloned()
        .collect();
    assert_eq!(
        decoded_arcs, built.routed_arcs,
        "{name}: the arcs the bits select are not the arcs the router chose"
    );
    eprintln!(
        "{name}: all {} set bit(s) decode, 0 unexplained; the {} arc(s) they select are \
         exactly the {} the router chose; {} hop(s) through a site; {} of {} signal(s) routed",
        decoded.bits,
        decoded_arcs.len(),
        built.routed_arcs.len(),
        built.site_hops.len(),
        built.signals.0,
        built.signals.1
    );
    assert_eq!(built.signals.0, built.signals.1, "{name}: not fully routed");
}

/// The features one tile decodes to.
fn at<'a>(built: &'a Built, tile: &str) -> Vec<&'a str> {
    built.decoded.at(tile)
}

/// **The demo design**: `examples/basys3/pmod_bidir.v`, an `IOBUF` on
/// Pmod JC pin 1 (K17) with its pull-up on, read back onto LD0.
///
/// What this catches, beyond "it routes": a bidirectional pad given the
/// input recipe (`IN_ONLY` would decode, and the drive would not), given
/// no receiver (`LVCMOS25_LVCMOS33_LVTTL.IN` would be missing), its `T`
/// left unrouted (no `ZINV_T1`, and no `TQ` site hop), the polarity bit put
/// on the wrong half of the tile, the pull-up missing or decoding as
/// `NONE`, and any pip written that the router did not take. What it does
/// not catch: that `ZINV_T1` means what prjxray says on silicon.
#[test]
fn the_bidirectional_demo_decodes_to_exactly_what_it_routed() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("pmod_bidir.v")),
        std::fs::read_to_string(dir.join("pmod_bidir.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };
    let built = build(&root, "pmod_bidir", &verilog, &rcf);
    assert_decodes_to_the_routing(&root, "pmod_bidir", &built);
    assert_eq!(built.pulled, 1, "exactly the one pad asked for a pull-up");

    // K17 is `IOB_X0Y25` in `LIOB33_X0Y25` (`package_pins.csv`), the
    // lower of the tile's two sites, which prjxray calls `IOB_Y1`.
    let pad = at(&built, "LIOB33_X0Y25");
    for wanted in [
        "IOB_Y1.LVCMOS25_LVCMOS33_LVTTL.IN",
        "IOB_Y1.LVCMOS33_LVTTL.DRIVE.I12_I16",
        "IOB_Y1.LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL_SSTL135_SSTL15.SLEW.SLOW",
        "IOB_Y1.PULLTYPE.PULLUP",
    ] {
        assert!(pad.contains(&wanted), "K17 lacks `{wanted}`: {pad:?}");
    }
    assert!(
        !pad.iter().any(|f| f.ends_with("IN_ONLY")),
        "a bidirectional pad configured input-only: {pad:?}"
    );
    assert!(
        !pad.contains(&"IOB_Y1.PULLTYPE.NONE"),
        "the pull-up did not displace NONE: {pad:?}"
    );
    assert!(
        pad.iter().all(|f| f.starts_with("IOB_Y1.")),
        "the other half of K17's tile was touched: {pad:?}"
    );

    let logic = at(&built, "LIOI3_X0Y25");
    for wanted in [
        "ILOGIC_Y1.ZINV_D",
        "OLOGIC_Y1.OMUX.D1",
        "OLOGIC_Y1.OQUSED",
        "OLOGIC_Y1.OSERDES.DATA_RATE_TQ.BUF",
        "OLOGIC_Y1.ZINV_T1",
    ] {
        assert!(
            logic.contains(&wanted),
            "K17's IO logic lacks `{wanted}`: {logic:?}"
        );
    }
    assert!(
        !logic
            .iter()
            .any(|f| f.starts_with("OLOGIC_Y0") || f.starts_with("ILOGIC_Y0")),
        "the other half of K17's IO logic was touched: {logic:?}"
    );
    assert!(
        built.site_hops.contains(&(
            "LIOI3_X0Y25".to_owned(),
            "LIOI_OLOGIC1_TQ.IOI_OLOGIC1_T1".to_owned()
        )),
        "the tristate did not go through the T1 -> TQ hop: {:?}",
        built.site_hops
    );

    // The two LEDs are plain outputs and must stay exactly the recipe that
    // has lit an LED: no polarity bit, no receiver, no pull-up.
    for (tile, half) in [("LIOI3_X0Y3", "Y1"), ("LIOI3_TBYTESRC_X0Y43", "Y1")] {
        let logic = at(&built, tile);
        assert!(
            !logic.iter().any(|f| f.contains("ZINV_T1")),
            "an LED's output logic got a tristate polarity bit: {tile} {logic:?}"
        );
        assert!(
            logic.contains(&format!("OLOGIC_{half}.OQUSED").as_str()),
            "{tile}: {logic:?}"
        );
    }
}

/// An `output` port a tri-state drives builds an `OBUFT`, and it costs at
/// the pad exactly what an `OBUF` does.
///
/// What this catches: the mapper falling back to `OBUF` and dropping the
/// enable (no `ZINV_T1`, no `TQ` hop), or giving the pad a receiver it
/// does not have. It also pins that this used to be an error — `$tristate`
/// "is a generic cell, not a primitive" — and is not any more.
#[test]
fn a_tristated_output_is_an_obuft_and_decodes_to_exactly_what_it_routed() {
    let Some(root) = chipdb() else { return };
    let verilog = "module obuft_out (input wire sw0, input wire sw1, output wire jc1);\n\
                   assign jc1 = sw0 ? sw1 : 1'bz;\n\
                   endmodule\n";
    let rcf = "set_io -io_standard LVCMOS33 sw0 V17\n\
               set_io -io_standard LVCMOS33 sw1 V16\n\
               set_io -io_standard LVCMOS33 jc1 K17\n";
    let built = build(&root, "obuft_out", verilog, rcf);
    assert_decodes_to_the_routing(&root, "obuft_out", &built);
    assert_eq!(built.pulled, 0);

    let module = &built.design.modules[built.top];
    let obuft = module
        .cells
        .iter()
        .filter(
            |(_, c)| matches!(&c.kind, reticle::ir::CellKind::Blackbox(n) if n.as_str() == "OBUFT"),
        )
        .count();
    assert_eq!(obuft, 1, "the tristated output is not an OBUFT");

    let pad = at(&built, "LIOB33_X0Y25");
    assert!(
        pad.contains(&"IOB_Y1.LVCMOS33_LVTTL.DRIVE.I12_I16"),
        "{pad:?}"
    );
    assert!(pad.contains(&"IOB_Y1.PULLTYPE.NONE"), "{pad:?}");
    assert!(
        !pad.iter()
            .any(|f| f.ends_with(".IN") || f.ends_with("IN_ONLY")),
        "an output-only pad got an input setting: {pad:?}"
    );
    let logic = at(&built, "LIOI3_X0Y25");
    assert!(logic.contains(&"OLOGIC_Y1.ZINV_T1"), "{logic:?}");
    assert!(
        !logic.iter().any(|f| f.starts_with("ILOGIC")),
        "an output-only pad got an input path: {logic:?}"
    );
}

/// **The polarity, end to end, given prjxray's reading of `ZINV_T1`.**
///
/// The chain from the switch to the pad's `T` has two places an inversion
/// can live: the lookup table the mapper builds (because `xc7.dev` says
/// `oen=T`, a one releases) and `ZINV_T1` in the `OLOGIC`. This walks the
/// synthesised netlist from the `IOBUF`'s `T` back to the lookup table that
/// drives it and from there to `sw0`'s buffer, evaluates the table, and
/// applies `ZINV_T1` as prjxray tags it (`ZINV_T1 = 1 ^ IS_T1_INVERTED`):
/// with `sw0` high the pad's `T` must be **zero** (driving), and with `sw0`
/// low it must be **one** (released).
///
/// What this catches: the mapper's inversion lost or doubled, the enable
/// wired to the wrong switch, or `ZINV_T1` missing from the bitstream. What
/// it cannot: that prjxray's reading is right. If it were the other way
/// round, every number here would still pass and the pad would drive
/// exactly when it was told to let go — which is the table in
/// `pmod_bidir.v`'s header under "the enable inverted".
#[test]
fn the_enable_reaches_the_pad_the_right_way_round() {
    use reticle::ir::{AttrValue, CellKind, ExprKind};

    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("pmod_bidir.v")),
        std::fs::read_to_string(dir.join("pmod_bidir.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };
    let built = build(&root, "pmod_bidir", &verilog, &rcf);
    let module = &built.design.modules[built.top];

    // A net an expression reads, looking through one-bit slices.
    let net_of = |mut expr: reticle::ir::ExprId| loop {
        match &module.exprs[expr].kind {
            ExprKind::Net(net) => break Some(*net),
            ExprKind::Slice { base, .. } => expr = *base,
            _ => break None,
        }
    };
    let blackbox = |name: &str| {
        module
            .cells
            .iter()
            .filter(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == name))
            .map(|(_, c)| c)
            .collect::<Vec<_>>()
    };
    let iobuf = blackbox("IOBUF");
    assert_eq!(iobuf.len(), 1);
    let t_net = net_of(iobuf[0].input("T").expect("the IOBUF's T is connected"))
        .expect("the IOBUF's T is a net");
    let lut = module
        .cells
        .iter()
        .map(|(_, c)| c)
        .find(|c| c.outputs.iter().any(|(_, n)| *n == t_net))
        .expect("something drives T");
    assert!(
        matches!(&lut.kind, CellKind::Blackbox(n) if n.as_str().starts_with("LUT")),
        "T is driven by `{:?}`, not a lookup table",
        lut.kind
    );
    // sw0's input buffer, and the net it drives into the fabric.
    let sw0 = blackbox("IBUF")
        .into_iter()
        .find(|c| c.attrs.get("port").and_then(AttrValue::as_str) == Some("sw0"))
        .expect("sw0 has an input buffer");
    let sw0_net = sw0
        .outputs
        .iter()
        .find(|(p, _)| p.as_str() == "O")
        .unwrap()
        .1;

    let init = match lut.params.get("INIT") {
        Some(AttrValue::Const(c)) => c.clone(),
        other => panic!("the lookup table's INIT is {other:?}"),
    };
    let t1 = |switch: bool| -> bool {
        let mut index = 0u32;
        let mut reads_sw0 = false;
        for (port, expr) in &lut.inputs {
            let Some(i) = port
                .as_str()
                .strip_prefix('I')
                .and_then(|i| i.parse::<u32>().ok())
            else {
                continue;
            };
            let bit = match &module.exprs[*expr].kind {
                ExprKind::Const(c) => c.get(0) == Some(reticle::ir::Bit::One),
                _ if net_of(*expr) == Some(sw0_net) => {
                    reads_sw0 = true;
                    switch
                }
                _ => panic!("the enable's lookup table reads something other than sw0"),
            };
            if bit {
                index |= 1 << i;
            }
        }
        assert!(reads_sw0, "the enable does not come from sw0");
        init.get(index) == Some(reticle::ir::Bit::One)
    };

    // `ZINV_T1` set is `IS_T1_INVERTED = 0`: the pad's T is T1.
    let zinv_t1 = at(&built, "LIOI3_X0Y25").contains(&"OLOGIC_Y1.ZINV_T1");
    assert!(zinv_t1, "the polarity bit is not in the bitstream");
    let pad_t = |switch: bool| if zinv_t1 { t1(switch) } else { !t1(switch) };
    assert!(!pad_t(true), "sw0 up must DRIVE the pad (T = 0)");
    assert!(pad_t(false), "sw0 down must RELEASE the pad (T = 1)");
}
