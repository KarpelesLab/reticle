//! The iCE40 backend against Project IceStorm's database and against
//! bitstreams other tools wrote.
//!
//! Two inputs are optional and every test that needs one skips with a
//! message when it is absent:
//!
//! - **the chip database**: `RETICLE_ICESTORMDB` names the directory
//!   holding `chipdb-5k.txt` and the other dice's; unset, the copy
//!   `reticle fetch icestorm` cached is used. A test that needs a die the
//!   copy lacks (one fetched before the other dice were taken) skips;
//! - **reference bitstreams**: `RETICLE_ICE40_REF` names a directory of
//!   `<name>.asc` / `<name>.bin` pairs that nextpnr and icepack wrote, for
//!   any iCE40 die: each is decoded on the die its `.device` line names.
//!   None ship with Reticle.

#![cfg(feature = "fpga")]

mod ice40_mac16;

use std::path::PathBuf;

use reticle::fpga::icestorm::IceFabric;
use reticle::fpga::icestorm::db::ChipDb;
use reticle::fpga::icestorm::image::Image;

/// The database `reticle fetch icestorm` pins (`src/bin/reticle/datadir.rs`
/// checks these strings against its own).
const DATABASE: &str = "icestorm";
/// Its pinned version, which is the cache directory's name.
const VERSION: &str = "0~20250207git7fbf8c0+dfsg1-1";
/// The variable that names a copy instead.
const ENV: &str = "RETICLE_ICESTORMDB";
/// The file every copy has.
const PROBE: &str = "chipdb-5k.txt";

/// The directory holding `chipdb-5k.txt`, or `None` with the reason
/// printed.
fn database_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(ENV) {
        return Some(PathBuf::from(dir));
    }
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
        .join("reticle")
        .join(DATABASE)
        .join(VERSION);
    if dir.join(PROBE).is_file() {
        return Some(dir);
    }
    eprintln!("skipping: no IceStorm database (`reticle fetch {DATABASE}`, or {ENV})");
    None
}

fn chipdb_5k() -> Option<ChipDb> {
    chipdb("5k")
}

/// The chip database of die `die` (`384`, `1k`, `5k`, `8k`, `u4k`,
/// `lm4k`), or `None` with the reason printed.
fn chipdb(die: &str) -> Option<ChipDb> {
    let dir = database_dir()?;
    let text = match std::fs::read_to_string(dir.join(format!("chipdb-{die}.txt"))) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: {}: {e}", dir.display());
            return None;
        }
    };
    Some(ChipDb::parse(&text).expect("the database parses"))
}

/// The UltraPlus's timing figures, `timings_up5k.txt` beside the chip
/// database, or `None` with the reason printed.
#[cfg(feature = "verilog")]
fn timing_db() -> Option<reticle::fpga::icestorm::timing::TimingDb> {
    let dir = database_dir()?;
    let text = match std::fs::read_to_string(dir.join("timings_up5k.txt")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: {}: {e}", dir.display());
            return None;
        }
    };
    Some(reticle::fpga::icestorm::timing::TimingDb::parse(&text).expect("the timing file parses"))
}

/// The reference `.asc`/`.bin` pairs, or none with the reason printed.
fn reference_pairs() -> Vec<(String, String, Vec<u8>)> {
    let Ok(dir) = std::env::var("RETICLE_ICE40_REF") else {
        eprintln!("skipping: RETICLE_ICE40_REF names no directory of reference bitstreams");
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: cannot read {dir}");
        return out;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for asc in paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "asc"))
    {
        let bin = asc.with_extension("bin");
        if let (Ok(text), Ok(bytes)) = (std::fs::read_to_string(asc), std::fs::read(&bin)) {
            let name = asc
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((name, text, bytes));
        }
    }
    if out.is_empty() {
        eprintln!("skipping: {dir} holds no .asc with a .bin beside it");
    }
    out
}

/// What icepack made of each reference `.asc` is what this packs it into,
/// byte for byte, and unpacking that `.bin` gives the same image.
///
/// This is the check on `image.rs`'s port of icepack's bank arithmetic: a
/// wrong column offset, a missed mirror or a wrong IO permutation moves a
/// bit, and the bytes differ. It would not catch a bit both directions
/// misplace identically, which the round trip through the *vendor's*
/// `.bin` rules out.
#[test]
fn icepacks_bytes_are_reproduced() {
    for (name, asc, bin) in reference_pairs() {
        let image = Image::from_asc(&asc).unwrap_or_else(|e| panic!("{name}.asc: {e}"));
        let ours = image.to_bin();
        // The comment block before the synchronisation word is the
        // `.comment`'s text, which says which tool wrote it.
        let sync = |b: &[u8]| {
            b.windows(4)
                .position(|w| w == [0x7E, 0xAA, 0x99, 0x7E])
                .unwrap()
        };
        assert_eq!(
            &ours[sync(&ours)..],
            &bin[sync(&bin)..],
            "{name}: the stream differs"
        );
        let back = Image::from_bin(&bin).unwrap_or_else(|e| panic!("{name}.bin: {e}"));
        assert_eq!(back.tiles, image.tiles, "{name}: unpacked tiles differ");
        assert_eq!(
            back.extra_bits, image.extra_bits,
            "{name}: unpacked extra bits differ"
        );
        eprintln!(
            "{name}: {} set bits, {} bytes, identical",
            image.ones(),
            bin.len()
        );
    }
}

/// Every bit nextpnr set in a reference bitstream belongs to something the
/// database names: a multiplexer pattern that selects a source, or a named
/// function bit. A decoder that left bits unexplained in a vendor's own
/// file would be no check on this flow's.
#[test]
fn every_bit_of_a_reference_decodes() {
    let pairs = reference_pairs();
    if pairs.is_empty() {
        return;
    }
    let mut fabrics: std::collections::BTreeMap<String, IceFabric> = Default::default();
    for (name, asc, _) in pairs {
        let image = Image::from_asc(&asc).unwrap();
        let die = image.layout.device.clone();
        if !fabrics.contains_key(&die) {
            let Some(db) = chipdb(&die) else { continue };
            let fabric = IceFabric::build(db, default_package(&die)).expect("the fabric builds");
            fabrics.insert(die.clone(), fabric);
        }
        let fabric = &fabrics[&die];
        let decoded = fabric.decode(&image);
        assert!(
            decoded.bad_patterns.is_empty(),
            "{name}: {:?}",
            decoded.bad_patterns
        );
        assert!(
            decoded.unexplained.is_empty(),
            "{name}: {:?}",
            decoded.unexplained
        );
        eprintln!(
            "{name} ({die}): {} set bits, {} arcs, {} function fields, 0 unexplained; {} block \
             RAMs (read and write modes {:?}) holding {} content bits, {} single-port RAMs",
            decoded.bits,
            decoded.arcs.len(),
            decoded.functions.len(),
            decoded.rams.len(),
            decoded
                .rams
                .values()
                .map(|r| (r.read_mode, r.write_mode))
                .collect::<Vec<_>>(),
            decoded.ram_bits,
            decoded.sprams.len()
        );
        // A reference nextpnr placed and routed from Reticle's own export of
        // `ram_selftest.v` holds the same block RAMs, modes and contents
        // that Reticle's backend writes for it, wherever each one is placed.
        #[cfg(feature = "verilog")]
        if name == "ram_selftest" && die == "5k" {
            let graph = fabric.arch.build_graph();
            let built = build_board_design("ram_selftest", fabric, &graph);
            let key = |r: &reticle::fpga::icestorm::RamConfig| {
                (r.read_mode, r.write_mode, r.contents.clone())
            };
            let mut theirs: Vec<_> = decoded.rams.values().map(key).collect();
            let mut ours: Vec<_> = built.decoded.rams.values().map(key).collect();
            theirs.sort();
            ours.sort();
            assert!(
                theirs == ours,
                "nextpnr's block RAMs differ from this backend's"
            );
            assert_eq!(decoded.sprams.len(), built.decoded.sprams.len());
            eprintln!("{name}: the same block RAMs, modes and contents as this backend writes");
        }
        // And one placed and routed from `dsp_selftest.v` holds the same
        // eight DSP configurations but for the two parameters nextpnr's
        // `bitstream.cc` writes under IceStorm's names and this backend
        // swaps (`DspConfig::as_bits`): nextpnr and this backend agree on
        // which bit IceStorm names what.
        #[cfg(feature = "verilog")]
        if name == "dsp_selftest" && die == "5k" {
            let graph = fabric.arch.build_graph();
            let built = build_board_design("dsp_selftest", fabric, &graph);
            let swapped = |d: &reticle::fpga::icestorm::DspConfig| {
                let mut p = d.params.clone();
                let top = p.get("TOP_8x8_MULT_REG").copied().unwrap_or(0);
                let pipe = p.get("PIPELINE_16x16_MULT_REG1").copied().unwrap_or(0);
                p.insert("TOP_8x8_MULT_REG".to_owned(), pipe);
                p.insert("PIPELINE_16x16_MULT_REG1".to_owned(), top);
                p
            };
            let mut theirs: Vec<_> = decoded.dsps.values().map(swapped).collect();
            let mut ours: Vec<_> = built
                .decoded
                .dsps
                .values()
                .map(|d| d.params.clone())
                .collect();
            theirs.sort();
            ours.sort();
            assert_eq!(
                theirs, ours,
                "nextpnr's DSP blocks differ from this backend's"
            );
            eprintln!("{name}: the same eight DSP configurations as this backend writes");
        }
        // And the hard IP designs: the same hard cells, each with the same
        // configuration bits, as this backend writes.
        #[cfg(feature = "verilog")]
        if matches!(name.as_str(), "sbus_probe" | "ledda_breathe") && die == "5k" {
            let graph = fabric.arch.build_graph();
            let built = build_board_design(&name, fabric, &graph);
            assert_eq!(
                decoded.hard_ip, built.decoded.hard_ip,
                "nextpnr's hard cells differ from this backend's"
            );
            eprintln!(
                "{name}: the same hard cells and bits as this backend: {:?}",
                decoded.hard_ip.keys().collect::<Vec<_>>()
            );
        }
        // And the PLL designs: the same PLL parameters and the same global
        // networks driven through a pad's input path.
        #[cfg(feature = "verilog")]
        if (name == "pll_uart" || name == "pll2f_uart") && die == "5k" {
            let graph = fabric.arch.build_graph();
            let built = build_board_design(&name, fabric, &graph);
            assert_eq!(decoded.plls, built.decoded.plls, "nextpnr's PLL differs");
            assert_eq!(
                decoded.padin, built.decoded.padin,
                "nextpnr's global networks"
            );
            eprintln!(
                "{name}: the same PLL parameters and pad-input networks {:?} as this backend",
                decoded.padin
            );
        }
    }
}

/// The package a reference bitstream of die `die` is decoded in: the one
/// in which the die has all its PLLs, which is all a package changes about
/// what the bits can mean.
fn default_package(die: &str) -> &'static str {
    match die {
        "384" => "qn32",
        "1k" => "tq144",
        "8k" => "ct256",
        "lm4k" => "swg25tr",
        _ => "sg48",
    }
}

/// The 5k fabric's shape: the counts a reader can check against the
/// database by hand.
#[test]
fn the_5k_fabric_has_the_parts_shape() {
    let Some(db) = chipdb_5k() else { return };
    let nets = db.nets.len();
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let lcs = graph.sites.iter().filter(|s| s.kind == "lc").count();
    let ios = graph.sites.iter().filter(|s| s.kind == "io").count();
    eprintln!(
        "{} nodes for {nets} nets, {} pips, {lcs} logic cells, {ios} IO blocks, {} dangling",
        graph.nodes.len(),
        graph.pips.len(),
        graph.dangling
    );
    // 5280 logic cells: the data sheet's figure for the UP5K.
    assert_eq!(lcs, 5280);
    assert_eq!(graph.dangling, 0, "every reference resolves");
}

// ---------------------------------------------------------------------------
// Packing computes what the mapped netlist computes
// ---------------------------------------------------------------------------

/// Two proofs, each reading a netlist through a model of its own cells.
///
/// **The mapped netlist computes what the source does.** The netlist
/// Reticle's mapping writes — the one `reticle fpga` hands nextpnr as JSON,
/// and the one packing starts from — read through the iCE Technology
/// Library's own cell semantics (as Yosys's `techlibs/ice40/cells_sim.v`
/// writes them, where an `SB_DFFESR` is `if (E) Q <= R ? 0 : D`, the reset
/// waiting for the enable), is proved equivalent to the source after
/// generic synthesis. The `*_maps_to_the_logic_it_was_written_as` tests do
/// that, for every flip-flop shape with an enable and a set or reset, on
/// both edges, set and reset, active high and low, synchronous and not.
/// They fail with the enable widening of `techcells::FfCtx::widen` removed,
/// because a reset asserted with the enable low is then lost: that is the
/// netlist nextpnr was given for any iCE40 until it was found.
///
/// **The packed netlist computes what the mapped one does.** The packed
/// netlist's model of a logic cell is the *silicon's*, as `icebox_vlog`
/// decodes a configured one: an unconnected input reads zero, an
/// unconnected enable one, a synchronous set/reset acts only on an enabled
/// edge, and the output is the flip-flop when `DFF_ENABLE` is set. So a
/// packing that dropped a carry in, folded a constant the wrong way, or
/// moved a table's inputs without its truth table is a difference found
/// here (`*_packs_to_the_same_logic`). With the library and the silicon
/// agreeing about the reset, packing passes `CEN` and `SR` through as they
/// are, and these tests would fail if it widened the enable a second time
/// in a way that changed the logic — though `E | R | R` is `E | R`, so a
/// second widening would only cost a table, which the report counts.
///
/// What they would not catch: anything after packing — placement, routing,
/// the bits. Those are what `the_board_designs_build_and_every_bit_decodes`
/// and the board check. Nor do they run nextpnr, which CI does not have:
/// the JSON nextpnr reads is the netlist proved here, and that it places,
/// routes and runs on a board was checked by hand (`docs/fpga-icestorm.md`,
/// "A reset that waited for an enable").
#[cfg(all(feature = "formal", feature = "verilog"))]
mod packing {
    use reticle::diag::Diagnostics;
    use reticle::formal::{EquivOptions, EquivOutcome, InitMode, check_equivalent};
    use reticle::fpga::icestorm::pack;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::ir::builder::ModuleBuilder;
    use reticle::ir::{AttrValue, CellId, CellKind, Const, Design, ExprId, Module, Name, Reset};
    use reticle::source::SourceMap;
    use reticle::synth::Pass;
    use reticle::synth::proc::ProcLower;

    /// The source, elaborated and its processes lowered and nothing more —
    /// the minimal pipeline `synth::verify` trusts, before any pass that
    /// infers a set/reset — so that its synchronous resets are a
    /// multiplexer and a register, and its meaning is the HDL's rather than
    /// any cell's.
    fn rtl(source: &str) -> Module {
        let mut map = SourceMap::new();
        let id = map.add("t.v".to_owned(), source).unwrap();
        let mut diags = Diagnostics::new();
        let ast = reticle::verilog::parse_source(
            &mut map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        );
        let design = reticle::verilog::elaborate_file(
            &ast,
            &reticle::verilog::ElabOptions::default(),
            &mut diags,
        )
        .expect("the design elaborates");
        let top = design.top.expect("a top module");
        let mut module = design.modules[top].clone();
        ProcLower::default().run(&mut module, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        module
    }

    fn mapped(source: &str) -> (Design, reticle::ir::ModuleId) {
        let device = target("ice40-up5k-sg48").expect("the device ships");
        let mut map = SourceMap::new();
        let id = map.add("t.v".to_owned(), source).unwrap();
        let mut diags = Diagnostics::new();
        let ast = reticle::verilog::parse_source(
            &mut map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        );
        let mut design = reticle::verilog::elaborate_file(
            &ast,
            &reticle::verilog::ElabOptions::default(),
            &mut diags,
        )
        .expect("the design elaborates");
        let top = design.top.expect("a top module");
        synthesize_for(
            &mut design,
            top,
            device,
            &Constraints::new(),
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow");
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        (design, top)
    }

    fn param(cell: &reticle::ir::Cell, key: &str) -> u64 {
        match cell.params.get(key) {
            Some(AttrValue::Const(c)) => c.to_u64().unwrap_or(0),
            Some(other) => other.as_int().map_or(0, |v| u64::try_from(v).unwrap_or(0)),
            None => 0,
        }
    }

    /// Every cell of `module` replaced by plain IR: an `SB_*` library cell
    /// by the iCE Technology Library's meaning of it, an `ICESTORM_LC` by
    /// the silicon's as `icebox_vlog` decodes it.
    fn model(module: Module) -> Module {
        let span = module.cells.iter().next().map(|(_, c)| c.span).unwrap();
        let mut b = ModuleBuilder::from_module(module, span);
        let ids: Vec<CellId> = b.module().cells.iter().map(|(id, _)| id).collect();
        let mut replaced = Vec::new();
        for id in ids {
            let cell = b.module().cells[id].clone();
            let CellKind::Blackbox(name) = &cell.kind else {
                continue;
            };
            let input = |b: &mut ModuleBuilder, port: &str| -> ExprId {
                cell.input(port).unwrap_or_else(|| b.const_bit(false))
            };
            let buf = |b: &mut ModuleBuilder, tag: &str, a: ExprId, port: &str| {
                if let Some(out) = cell.output(port) {
                    b.cell(
                        format!("{}${tag}", cell.name),
                        CellKind::Buf,
                        vec![(Name::new("a"), a)],
                        vec![(Name::new("y"), out)],
                    );
                }
            };
            let lut = |b: &mut ModuleBuilder, init: u64, tag: &str| -> ExprId {
                let pins: Vec<ExprId> = ["I3", "I2", "I1", "I0"]
                    .iter()
                    .map(|p| input(b, p))
                    .collect();
                let a = b.concat(pins);
                let net = b.add_net(format!("{}${tag}", cell.name), reticle::ir::Type::bit());
                b.cell(
                    format!("{}${tag}_lut", cell.name),
                    CellKind::Lut {
                        k: 4,
                        init: Const::from_u64(init, 16),
                    },
                    vec![(Name::new("a"), a)],
                    vec![(Name::new("y"), net)],
                );
                b.net(net)
            };
            let carry = |b: &mut ModuleBuilder, i0: ExprId, i1: ExprId, ci: ExprId| -> ExprId {
                let g = b.and(i0, i1);
                let p = b.or(i0, i1);
                let t = b.and(p, ci);
                b.or(g, t)
            };
            let dff = |b: &mut ModuleBuilder,
                       tag: &str,
                       clk_pos: bool,
                       clk: ExprId,
                       d: ExprId,
                       en: Option<ExprId>,
                       reset: Option<(ExprId, bool, u64)>,
                       q: reticle::ir::NetId| {
                let mut inputs = vec![(Name::new("clk"), clk), (Name::new("d"), d)];
                if let Some(en) = en {
                    inputs.push((Name::new("en"), en));
                }
                if let Some((rst, _, _)) = reset {
                    inputs.push((Name::new("rst"), rst));
                }
                b.cell(
                    format!("{}${tag}", cell.name),
                    CellKind::Dff {
                        clk_pos,
                        has_enable: en.is_some(),
                        reset: reset.map(|(_, asynchronous, value)| Reset {
                            asynchronous,
                            active_high: true,
                            value: Const::from_u64(value, 1),
                        }),
                    },
                    inputs,
                    vec![(Name::new("q"), q)],
                );
            };
            match name.as_str() {
                "SB_IO" => {
                    if let (Some(pad), Some(_)) = (cell.input("PACKAGE_PIN"), cell.output("D_IN_0"))
                    {
                        buf(&mut b, "in", pad, "D_IN_0");
                    } else if let Some(d) = cell.input("D_OUT_0") {
                        buf(&mut b, "out", d, "PACKAGE_PIN");
                    }
                }
                "SB_GB" => {
                    let a = input(&mut b, "USER_SIGNAL_TO_GLOBAL_BUFFER");
                    buf(&mut b, "gb", a, "GLOBAL_BUFFER_OUTPUT");
                }
                "SB_LUT4" => {
                    let y = lut(&mut b, param(&cell, "LUT_INIT"), "lut");
                    buf(&mut b, "o", y, "O");
                }
                "SB_CARRY" => {
                    let (i0, i1, ci) = (
                        input(&mut b, "I0"),
                        input(&mut b, "I1"),
                        input(&mut b, "CI"),
                    );
                    let co = carry(&mut b, i0, i1, ci);
                    buf(&mut b, "co", co, "CO");
                }
                ff if ff.starts_with("SB_DFF") => {
                    let rest = &ff["SB_DFF".len()..];
                    let clk_pos = !rest.starts_with('N');
                    let rest = rest.trim_start_matches('N');
                    let has_en = rest.starts_with('E');
                    let rest = rest.trim_start_matches('E');
                    let (asynchronous, set, port) = match rest {
                        "" => (false, false, None),
                        "SR" => (false, false, Some("R")),
                        "R" => (true, false, Some("R")),
                        "SS" => (false, true, Some("S")),
                        "S" => (true, true, Some("S")),
                        other => panic!("SB_DFF{other}"),
                    };
                    let (clk, d) = (input(&mut b, "C"), input(&mut b, "D"));
                    let en = has_en.then(|| input(&mut b, "E"));
                    let q = cell.output("Q").unwrap();
                    match port {
                        Some(p) if !asynchronous => {
                            // The library's synchronous reset: `if (E)
                            // Q <= R ? v : D` when there is an enable, and
                            // `Q <= R ? v : D` when there is not. In the
                            // IR, whose reset wins, that is a reset of
                            // `E & R`. (A register with a reset starts at
                            // its reset value in the formal engine, so the
                            // reset stays a reset, as it is in the source.)
                            let r = input(&mut b, p);
                            let r = match en {
                                Some(e) => b.and(e, r),
                                None => r,
                            };
                            let reset = Some((r, false, u64::from(set)));
                            dff(&mut b, "ff", clk_pos, clk, d, en, reset, q);
                        }
                        Some(p) => {
                            // An asynchronous one acts whatever the enable:
                            // `always @(posedge C, posedge R) if (R) Q <= 0;
                            // else if (E) Q <= D;`.
                            let reset = Some((input(&mut b, p), true, u64::from(set)));
                            dff(&mut b, "ff", clk_pos, clk, d, en, reset, q);
                        }
                        None => dff(&mut b, "ff", clk_pos, clk, d, en, None, q),
                    }
                }
                "ICESTORM_LC" | "ICESTORM_LC_COMB" => {
                    let lout = lut(&mut b, param(&cell, "LUT_INIT"), "lout");
                    if param(&cell, "CARRY_ENABLE") != 0 {
                        let (i1, i2, ci) = (
                            input(&mut b, "I1"),
                            input(&mut b, "I2"),
                            input(&mut b, "CIN"),
                        );
                        let co = carry(&mut b, i1, i2, ci);
                        buf(&mut b, "co", co, "COUT");
                    }
                    if param(&cell, "DFF_ENABLE") == 0 {
                        buf(&mut b, "o", lout, "O");
                    } else {
                        let clk = input(&mut b, "CLK");
                        let en = cell.input("CEN").unwrap_or_else(|| b.const_bit(true));
                        let sr = input(&mut b, "SR");
                        let set = param(&cell, "SET_NORESET");
                        let q = cell.output("O").unwrap();
                        let clk_pos = param(&cell, "NEG_CLK") == 0;
                        if param(&cell, "ASYNC_SR") != 0 {
                            let reset = Some((sr, true, set));
                            dff(&mut b, "ff", clk_pos, clk, lout, Some(en), reset, q);
                        } else {
                            // The silicon: `if (cen) q <= sr ? set : d`,
                            // which in the IR is a reset of `cen & sr`.
                            let r = b.and(en, sr);
                            let reset = Some((r, false, set));
                            dff(&mut b, "ff", clk_pos, clk, lout, Some(en), reset, q);
                        }
                    }
                }
                other => panic!("no model for `{other}`"),
            }
            replaced.push(id);
        }
        b.module_mut().cells.retain(|id, _| !replaced.contains(&id));
        b.finish()
    }

    /// `a` and `b`, added to `design`, proved to compute the same thing
    /// from a zero start, or a panic naming `what` differs.
    fn equivalent(design: &mut Design, a: Module, b: Module, name: &str, what: &str) {
        let ia = design.add_module(a);
        let ib = design.add_module(b);
        let options = EquivOptions {
            init: InitMode::Zero,
            match_state_by_name: false,
            depth: 12,
            max_k: 12,
            ..EquivOptions::default()
        };
        let result = check_equivalent(design, ia, ib, &options);
        match &result.outcome {
            EquivOutcome::Equivalent(_) => {}
            EquivOutcome::Different {
                frame, properties, ..
            } => {
                panic!(
                    "{name}: {what} differs at frame {frame}: {properties:?}\n{}",
                    result.render("source", "mapped")
                )
            }
            EquivOutcome::Unknown { depth } => {
                // No difference within the bound is still the claim this
                // test makes; an inductive proof is a bonus the counters'
                // unreachable states can deny.
                assert!(
                    depth.is_some(),
                    "{name}: the check did not run: {:?}",
                    result.diags.iter().map(|d| &d.message).collect::<Vec<_>>()
                );
            }
        }
    }

    fn prove(name: &str, source: &str) {
        let (mut design, top) = mapped(source);
        let mut packed = design.modules[top].clone();
        let report = pack::pack(&mut packed).unwrap_or_else(|e| panic!("{name}: {e}"));
        let a = model(design.modules[top].clone());
        let b = model(packed);
        equivalent(&mut design, a, b, name, "the packed netlist");
        eprintln!(
            "{name}: {} logic cells packed, no difference found",
            report.cells
        );
    }

    /// The mapped netlist, read through the library's cells, proved
    /// against the source; `cells` are primitives it must contain, so that
    /// the proof is about the flip-flops it claims to be about.
    fn prove_mapping(name: &str, source: &str, cells: &[&str]) {
        let (mut design, top) = mapped(source);
        for want in cells {
            let found = design.modules[top]
                .cells
                .iter()
                .any(|(_, c)| matches!(&c.kind, CellKind::Blackbox(n) if n.as_str() == *want));
            assert!(found, "{name}: the mapping made no {want}");
        }
        let a = rtl(source);
        let b = model(design.modules[top].clone());
        equivalent(&mut design, a, b, name, "the mapped netlist");
        // And packing it computes the same again.
        prove(name, source);
    }

    #[test]
    fn a_synchronous_reset_over_an_enable_maps_to_the_logic_it_was_written_as() {
        // Reset to 0101, so both the reset and the set form are used.
        prove_mapping(
            "sync",
            "module top(input clk, input en, input rst, input [3:0] d, output [3:0] q);\n\
             reg [3:0] r = 0;\n\
             always @(posedge clk) if (rst) r <= 4'b0101; else if (en) r <= d;\n\
             assign q = r;\n\
             endmodule\n",
            &["SB_DFFESR", "SB_DFFESS"],
        );
    }

    #[test]
    fn an_active_low_synchronous_reset_maps_to_the_logic_it_was_written_as() {
        // The reset is inverted for the active-high pin and the enable
        // widened by the reset's *active* level, not its wire.
        prove_mapping(
            "sync_low",
            "module top(input clk, input en, input rst_n, input [2:0] d, output [2:0] q);\n\
             reg [2:0] r = 0;\n\
             always @(posedge clk) if (!rst_n) r <= 3'b100; else if (en) r <= d;\n\
             assign q = r;\n\
             endmodule\n",
            &["SB_DFFESR", "SB_DFFESS"],
        );
    }

    #[test]
    fn a_falling_edge_synchronous_reset_maps_to_the_logic_it_was_written_as() {
        prove_mapping(
            "sync_neg",
            "module top(input clk, input en, input rst, input [1:0] d, output [1:0] q);\n\
             reg [1:0] r = 0;\n\
             always @(negedge clk) if (rst) r <= 2'b10; else if (en) r <= d;\n\
             assign q = r;\n\
             endmodule\n",
            &["SB_DFFNESR", "SB_DFFNESS"],
        );
    }

    #[test]
    fn an_asynchronous_reset_with_an_enable_maps_to_the_logic_it_was_written_as() {
        // Not the same class of fault, and checked to say so: an
        // asynchronous set/reset acts whatever the enable, in the library
        // and in the IR alike, so nothing is widened.
        prove_mapping(
            "async",
            "module top(input clk, input en, input rst, input [1:0] d, output [1:0] q);\n\
             reg [1:0] r = 0;\n\
             always @(posedge clk or posedge rst) if (rst) r <= 2'b01; else if (en) r <= d;\n\
             assign q = r;\n\
             endmodule\n",
            &["SB_DFFER", "SB_DFFES"],
        );
    }

    #[test]
    fn a_synchronous_reset_with_no_enable_maps_to_the_logic_it_was_written_as() {
        prove_mapping(
            "sync_plain",
            "module top(input clk, input rst, input [1:0] d, output [1:0] q);\n\
             reg [1:0] r = 0;\n\
             always @(posedge clk) if (rst) r <= 2'b01; else r <= d;\n\
             assign q = r;\n\
             endmodule\n",
            &["SB_DFFSR", "SB_DFFSS"],
        );
    }

    /// A carry out read by the next carry *and* by other logic, which an
    /// iCE40 carry cannot hand to both: packing puts a tap cell between the
    /// two (`icestorm::pack`, rule 5), passing the carry on and giving the
    /// fabric a copy. This was refused before; `ram_selftest.v` needs it for
    /// its hex digits. The test fails if the tap's carry does not pass its
    /// carry in through (its `I1` left at zero makes `CO` a constant) or its
    /// table reads the wrong input.
    #[test]
    fn a_carry_read_inside_and_outside_its_chain_packs_to_the_same_logic() {
        let source = "module top(input [3:0] v, output [7:0] h);\n\
                      assign h = v < 4'd10 ? 8'h30 + {4'd0, v} : 8'h57 + {4'd0, v};\n\
                      endmodule\n";
        let (design, top) = mapped(source);
        let mut packed = design.modules[top].clone();
        pack::pack(&mut packed).expect("packs");
        assert!(
            packed
                .cells
                .iter()
                .any(|(_, c)| c.name.as_str().contains("$tap")),
            "the design no longer needs a tap, so this test proves nothing about one"
        );
        prove("tap", source);
    }

    #[test]
    fn a_counter_with_enable_and_reset_packs_to_the_same_logic() {
        prove(
            "counter",
            "module top(input clk, input en, input rst, output [5:0] q);\n\
             reg [5:0] c = 0;\n\
             always @(posedge clk) if (rst) c <= 6'd21; else if (en) c <= c + 6'd1;\n\
             assign q = c;\n\
             endmodule\n",
        );
    }

    #[test]
    fn a_down_counter_and_a_carry_out_pack_to_the_same_logic() {
        prove(
            "down",
            "module top(input clk, input load, input [4:0] d, output z, output [4:0] q);\n\
             reg [4:0] c = 0;\n\
             always @(posedge clk) if (load) c <= d; else if (c != 0) c <= c - 5'd1;\n\
             assign z = c == 0;\n\
             assign q = c;\n\
             endmodule\n",
        );
    }

    #[test]
    fn an_adder_with_its_carry_out_packs_to_the_same_logic() {
        prove(
            "adder",
            "module top(input [6:0] a, input [6:0] b, output [7:0] s);\n\
             assign s = a + b;\n\
             endmodule\n",
        );
    }
}

// ---------------------------------------------------------------------------
// The board's two designs, built
// ---------------------------------------------------------------------------

/// The two designs that were loaded into a board build here too, through
/// the whole of `icestorm::implement`: packed, placed, routed, configured,
/// and both checks passed — every set bit decodes back through the
/// database and the connections the bits select are exactly the router's.
/// `implement` returns nothing that failed either, so this test is that a
/// build *succeeds*, plus the counts a reader can check.
///
/// What it would catch: a multiplexer pattern written the wrong way round,
/// a bit set in a tile that does not own it, a pip the bitstream selects
/// that the router never chose (or the reverse), a cell kind the flow
/// cannot place, an IO or hard-cell bit the database lacks. What it would
/// not: a bit that decodes to a *legal but wrong* configuration — an IO's
/// input enable in the wrong block, say. That is what the board said, and
/// what `docs/fpga-icestorm.md` records.
/// Builds `testdata/fpga/ice40up/<name>.v` (with the `.rcf` beside it)
/// through `icestorm::implement`, or `None` with the reason printed when
/// the database is absent.
#[cfg(feature = "verilog")]
fn build_board_design(
    name: &str,
    fabric: &IceFabric,
    graph: &reticle::fpga::arch::RoutingGraph,
) -> reticle::fpga::icestorm::Implemented {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/ice40up");
    let source = std::fs::read_to_string(dir.join(format!("{name}.v"))).unwrap();
    let rcf = std::fs::read_to_string(dir.join(format!("{name}.rcf"))).unwrap();
    implement_source(name, &source, &rcf, fabric, graph).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Builds Verilog `source` constrained by `rcf` through
/// `icestorm::implement`, returning what `implement` refused with.
#[cfg(feature = "verilog")]
fn implement_source(
    name: &str,
    source: &str,
    rcf: &str,
    fabric: &IceFabric,
    graph: &reticle::fpga::arch::RoutingGraph,
) -> Result<reticle::fpga::icestorm::Implemented, String> {
    implement_source_on("ice40-up5k-sg48", name, source, rcf, fabric, graph)
}

/// [`implement_source`] on the part the device file `device` describes.
#[cfg(feature = "verilog")]
fn implement_source_on(
    device: &str,
    name: &str,
    source: &str,
    rcf: &str,
    fabric: &IceFabric,
    graph: &reticle::fpga::arch::RoutingGraph,
) -> Result<reticle::fpga::icestorm::Implemented, String> {
    let device = reticle::fpga::target(device).expect("the device ships");
    let (design, top, constraints) = synthesize_source_on(device, name, source, rcf);
    reticle::fpga::icestorm::implement(
        &design,
        top,
        device,
        &constraints,
        fabric,
        graph,
        &reticle::fpga::place::PlaceOptions::default(),
    )
}

/// Verilog `source` constrained by `rcf`, synthesized for the
/// iCE40UP5K-SG48: the design, its top module and the constraints.
#[cfg(feature = "verilog")]
fn synthesize_source(
    name: &str,
    source: &str,
    rcf: &str,
) -> (
    reticle::ir::Design,
    reticle::ir::ModuleId,
    reticle::fpga::Constraints,
) {
    let device = reticle::fpga::target("ice40-up5k-sg48").expect("the device ships");
    synthesize_source_on(device, name, source, rcf)
}

/// [`synthesize_source`] for `device`.
#[cfg(feature = "verilog")]
fn synthesize_source_on(
    device: &reticle::fpga::Device,
    name: &str,
    source: &str,
    rcf: &str,
) -> (
    reticle::ir::Design,
    reticle::ir::ModuleId,
    reticle::fpga::Constraints,
) {
    use reticle::diag::Diagnostics;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for};
    use reticle::source::SourceMap;

    let (source, rcf) = (source.to_owned(), rcf.to_owned());
    let mut map = SourceMap::new();
    let id = map.add(format!("{name}.v"), source).unwrap();
    let cid = map.add(format!("{name}.rcf"), rcf.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        id,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .expect("the design elaborates");
    let top = design.top.expect("a top module");
    let constraints = Constraints::parse(&rcf, cid, &mut diags);
    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap_or_else(|e| panic!("{name}: the flow: {e:?}\n{}", diags.render(&map)));
    assert!(!diags.has_errors(), "{name}: {}", diags.render(&map));
    (design, top, constraints)
}

/// A read-only memory of `ram_selftest.v`: its name, width, depth and words.
#[cfg(feature = "verilog")]
type Rom = (&'static str, u32, u32, fn(u32) -> u32);

/// The block RAM contents a read-only memory of `depth` words of `width`
/// bits should have in the `.asc`, word `a` being `word(a)`, laid out as
/// `docs/fpga.md` ("Block RAM contents") says: the row is the low eight
/// address bits, the bits above them pick word `s` of the row, and data bit
/// `j` of word `s` is row bit `(16 / width) * j + s`; row `r` is bits
/// `16 * (r % 16) ..` of line `r / 16`.
#[cfg(feature = "verilog")]
fn expected_contents(width: u32, depth: u32, word: impl Fn(u32) -> u32) -> Vec<[bool; 256]> {
    let mut lines = vec![[false; 256]; 16];
    let per = 16 / width;
    for a in 0..depth {
        let (row, s) = (a % 256, a / 256);
        for j in 0..width {
            if word(a) >> j & 1 != 0 {
                let bit = per * j + s;
                lines[(row / 16) as usize][(16 * (row % 16) + bit) as usize] = true;
            }
        }
    }
    lines
}

/// The design that exercised both kinds of RAM on a board,
/// `ram_selftest.v`, builds through `icestorm::implement` with every bit
/// explained, the connections exactly the router's, and — the third check,
/// for RAMs — its eight block RAMs and four single-port RAMs read back from
/// the bitstream exactly as placed. And the four read-only ones hold, in
/// the `.asc`, exactly the contents the RTL's `initial` loops compute, laid
/// out as `docs/fpga.md` says, one in each width.
///
/// What it would catch: a RAM pin on a wire that is not the RAM's (no
/// route, or an arc the bits do not explain), a mode written into the wrong
/// tile or the wrong bits (read back differently), contents dropped or
/// misplaced on the way from the netlist's `INIT_`s into the image (the
/// contents comparison), the stream losing them (the round trip), or the
/// mapping's narrow-mode layout drifting from what the documentation says.
/// What it would not: a layout that is consistently wrong in the
/// documentation, the mapping and this test together — the bit order is
/// what the board is for, and `docs/fpga-icestorm.md` records that it read
/// back every word.
#[cfg(feature = "verilog")]
#[test]
fn both_kinds_of_ram_build_and_read_back() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let built = build_board_design("ram_selftest", &fabric, &graph);
    assert!(built.decoded.unexplained.is_empty());
    assert_eq!(built.decoded.arcs.len(), built.chosen);
    assert_eq!(
        built.rams, 8,
        "a block RAM per width, read-only and written"
    );
    assert_eq!(built.sprams, 4, "every single-port RAM");
    assert_eq!(built.decoded.rams.len(), 8);
    assert_eq!(built.decoded.sprams.len(), 4);
    let modes: Vec<(u32, u32)> = built
        .decoded
        .rams
        .values()
        .map(|r| (r.read_mode, r.write_mode))
        .collect();
    for mode in 0..4 {
        assert_eq!(
            modes.iter().filter(|m| **m == (mode, mode)).count(),
            2,
            "two block RAMs in mode {mode}: {modes:?}"
        );
    }
    let roms: [Rom; 4] = [
        ("rom16", 16, 256, |i| ((i ^ 0x5a) << 8) | (!i & 0xff)),
        ("rom8", 8, 512, |i| {
            (i & 0xff) ^ (((i >> 8) & 1) << 7 | ((i >> 2) & 0x7f)) ^ 0xc3
        }),
        ("rom4", 4, 1024, |i| {
            (i & 15) ^ ((i >> 4) & 15) ^ (((i >> 8) & 3) << 2 | ((i >> 8) & 3))
        }),
        ("rom2", 2, 2048, |i| {
            (i & 3) ^ ((i >> 2) & 3) ^ ((i >> 9) & 3)
        }),
    ];
    for (name, width, depth, word) in &roms {
        let want = expected_contents(*width, *depth, word);
        let mode = match width {
            16 => 0,
            8 => 1,
            4 => 2,
            _ => 3,
        };
        let found = built
            .decoded
            .rams
            .values()
            .any(|r| r.contents == want && r.read_mode == mode);
        assert!(found, "no block RAM holds {name}'s contents as documented");
    }
    // The contents survive the stream.
    let back = Image::from_bin(&built.image.to_bin()).unwrap();
    assert_eq!(back.ram_data, built.image.ram_data);
    assert_eq!(back.tiles, built.image.tiles);
    eprintln!(
        "ram_selftest: {} logic cells, {} set bits, {} arcs, {} block RAMs ({} content bits), {} \
         single-port RAMs, 0 unexplained",
        built.pack.cells,
        built.decoded.bits,
        built.chosen,
        built.rams,
        built.decoded.ram_bits,
        built.sprams
    );
}

#[cfg(feature = "verilog")]
#[test]
fn the_board_designs_build_and_every_bit_decodes() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::source::SourceMap;

    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let device = target("ice40-up5k-sg48").expect("the device ships");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/ice40up");
    for name in ["rgb_blink", "uart_echo"] {
        let source = std::fs::read_to_string(dir.join(format!("{name}.v"))).unwrap();
        let rcf = std::fs::read_to_string(dir.join(format!("{name}.rcf"))).unwrap();
        let mut map = SourceMap::new();
        let id = map.add(format!("{name}.v"), source).unwrap();
        let cid = map.add(format!("{name}.rcf"), rcf.clone()).unwrap();
        let mut diags = Diagnostics::new();
        let ast = reticle::verilog::parse_source(
            &mut map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        );
        let mut design = reticle::verilog::elaborate_file(
            &ast,
            &reticle::verilog::ElabOptions::default(),
            &mut diags,
        )
        .expect("the design elaborates");
        let top = design.top.expect("a top module");
        let constraints = Constraints::parse(&rcf, cid, &mut diags);
        synthesize_for(
            &mut design,
            top,
            device,
            &constraints,
            &FpgaOptions::default(),
            &mut diags,
        )
        .expect("the flow");
        assert!(!diags.has_errors(), "{name}: {}", diags.render(&map));
        let built = reticle::fpga::icestorm::implement(
            &design,
            top,
            device,
            &constraints,
            &fabric,
            &graph,
            &reticle::fpga::place::PlaceOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(built.decoded.unexplained.is_empty());
        assert_eq!(built.decoded.arcs.len(), built.chosen);
        assert!(built.chosen > 0, "{name} routed nothing that costs a bit");
        eprintln!(
            "{name}: {} logic cells, {} set bits, {} arcs, 0 unexplained",
            built.pack.cells, built.decoded.bits, built.chosen
        );
        // The stream unpacks to the same image, so what is loaded is what
        // was checked.
        let back = Image::from_bin(&built.image.to_bin()).unwrap();
        assert_eq!(back.tiles, built.image.tiles, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Every part
// ---------------------------------------------------------------------------

/// Each part's device file states what its chip database does: every pin of
/// the package that IceStorm maps to an IO block, and no other, each marked
/// `clock` exactly when it can drive a global network (`.gbufpin`); the
/// logic cells (eight a logic tile); whether there is block RAM; the PLLs
/// the package has (the database's `LOCKED` lines take some away); the DSP
/// blocks; and the grid.
///
/// What it would catch: a pin list copied from a board file rather than the
/// package (the LP/HX lists were, until the backend reached these parts), a
/// pin in the wrong package of a die, a `pll` line on a part whose package
/// does not bond the PLL — which a design would then be refused for only
/// after synthesis — or a count from a data sheet that disagrees with the
/// silicon IceStorm mapped. What it would not: that IceStorm's map is the
/// silicon's, which no board here could check for these parts.
#[test]
fn every_device_file_is_its_parts_database() {
    use reticle::fpga::icestorm::db::TileKind;
    use std::collections::{BTreeMap, BTreeSet};
    for part in &reticle::fpga::icestorm::PARTS {
        let Some(db) = chipdb(part.die) else { return };
        let device = reticle::fpga::target(part.device).expect("the device ships");
        let who = part.device;
        let gbuf: BTreeSet<(u32, u32, u32)> = db.gbufpin.iter().map(|g| (g.0, g.1, g.2)).collect();
        let want: BTreeMap<String, bool> = db
            .package(part.package)
            .unwrap_or_else(|| panic!("{who}: no package {}", part.package))
            .iter()
            .filter(|(_, at)| db.tile(at.0, at.1) == TileKind::Io)
            .map(|(pin, at)| (pin.clone(), gbuf.contains(at)))
            .collect();
        let have: BTreeMap<String, bool> = device
            .pins
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.kind == reticle::fpga::device::PinKind::Clock,
                )
            })
            .collect();
        assert_eq!(have, want, "{who}: the pins");
        assert!(!device.pins_partial, "{who}: the whole package is listed");
        let tiles = |kind: TileKind| db.tiles.iter().filter(|t| **t == kind).count();
        let lut = device.bel_named("SB_LUT4").expect("a lookup table");
        assert_eq!(
            lut.count,
            Some(u32::try_from(tiles(TileKind::Logic) * 8).unwrap()),
            "{who}: logic cells"
        );
        assert_eq!(
            device.block_rams.is_empty(),
            tiles(TileKind::Ramb) == 0,
            "{who}: block RAM"
        );
        let plls = db
            .extra_cells
            .iter()
            .filter(|c| c.kind == "PLL" && c.available_in(part.package))
            .count();
        for shape in &device.clock_resources.plls {
            assert_eq!(
                shape.count,
                Some(u32::try_from(plls).unwrap()),
                "{who}: {}",
                shape.name
            );
        }
        assert_eq!(
            device.clock_resources.plls.is_empty(),
            plls == 0,
            "{who}: PLL lines"
        );
        let macs = db.extra_cells.iter().filter(|c| c.kind == "MAC16").count();
        assert_eq!(
            device.bel_named("SB_MAC16").and_then(|b| b.count),
            (macs > 0).then(|| u32::try_from(macs).unwrap()),
            "{who}: DSP blocks"
        );
        for (cell, kind) in [
            ("SB_SPRAM256KA", "SPRAM"),
            ("SB_HFOSC", "HFOSC"),
            ("SB_LFOSC", "LFOSC"),
            ("SB_RGBA_DRV", "RGBA_DRV"),
        ] {
            let in_db = db.extra_cells.iter().any(|c| c.kind == kind);
            assert_eq!(device.bel_named(cell).is_some(), in_db, "{who}: {cell}");
        }
        let grid = device.tile_grid.expect("a grid");
        assert_eq!((grid.width, grid.height), (db.width, db.height), "{who}");
        eprintln!(
            "{who}: {} pins ({} clock), {} logic cells, {} block RAMs, {plls} PLL(s), {macs} DSP \
             blocks, {} x {}, as the database says",
            want.len(),
            want.values().filter(|c| **c).count(),
            tiles(TileKind::Logic) * 8,
            tiles(TileKind::Ramb),
            db.width,
            db.height
        );
    }
}

/// The column buffers this backend enables — every one at a `.colbuf`
/// source of the chip database — are exactly the rows nextpnr's
/// `bitstream.cc` enables on each die (all their tiles' `ColBufCtrl` bits),
/// so the two ways of saying it agree. On the 384 nextpnr enables none,
/// and so does this backend, whatever the database lists.
///
/// What it would catch: a die whose `.colbuf` sources are not nextpnr's
/// rows, which would leave this backend's global networks reaching other
/// columns than a nextpnr build's. The rows are QUOTED from nextpnr's
/// source, not measured.
#[test]
fn colbuf_rows_are_nextpnrs() {
    use reticle::fpga::icestorm::db::TileKind;
    use std::collections::BTreeSet;
    for (die, rows) in [
        ("1k", &[4u32, 5, 12, 13][..]),
        ("8k", &[8, 9, 24, 25][..]),
        ("u4k", &[4, 5, 16, 17][..]),
        ("5k", &[4, 5, 14, 15, 26, 27][..]),
    ] {
        let Some(db) = chipdb(die) else { return };
        let sources: BTreeSet<(u32, u32)> = db.colbuf.iter().map(|(s, _)| *s).collect();
        let mut nextpnr = BTreeSet::new();
        for y in 0..db.height {
            for x in 0..db.width {
                let kind = db.tile(x, y);
                if kind == TileKind::Empty {
                    continue;
                }
                // On the 1k a block RAM's tiles are one row lower.
                let in_row = if die == "1k" && matches!(kind, TileKind::Ramb | TileKind::Ramt) {
                    [3, 5, 11, 13].contains(&y)
                } else {
                    rows.contains(&y)
                };
                if in_row {
                    nextpnr.insert((x, y));
                }
            }
        }
        assert_eq!(sources, nextpnr, "{die}");
    }
}

/// The package pins of `part` a test design may use: its first clock pin,
/// and the others that are neither a clock pin nor an IO block a PLL's
/// output passes through.
#[cfg(feature = "verilog")]
fn test_pins(part: &reticle::fpga::icestorm::Part, db: &ChipDb) -> (String, Vec<String>) {
    use std::collections::BTreeSet;
    let gbuf: BTreeSet<(u32, u32, u32)> = db.gbufpin.iter().map(|g| (g.0, g.1, g.2)).collect();
    let mut pll_ios = BTreeSet::new();
    for cell in db.extra_cells.iter().filter(|c| c.kind == "PLL") {
        for (key, x, y, z) in &cell.entries {
            if key == "PLLOUT_A" || key == "PLLOUT_B" {
                pll_ios.insert((*x, *y, z.parse::<u32>().unwrap()));
            }
        }
    }
    let pins = db.package(part.package).unwrap();
    let io =
        |at: &(u32, u32, u32)| db.tile(at.0, at.1) == reticle::fpga::icestorm::db::TileKind::Io;
    let clock = pins
        .iter()
        .find(|(_, at)| io(at) && gbuf.contains(at) && !pll_ios.contains(at))
        .map(|(p, _)| p.clone())
        .expect("a clock pin");
    let others = pins
        .iter()
        .filter(|(_, at)| io(at) && !gbuf.contains(at) && !pll_ios.contains(at))
        .map(|(p, _)| p.clone())
        .collect();
    (clock, others)
}

/// A test design: its name, its file under `testdata/fpga`, and its ports
/// other than the clock with their widths.
#[cfg(feature = "verilog")]
type TestDesign = (&'static str, &'static str, &'static [(&'static str, u32)]);

/// Every part builds the designs written for the parts no board here
/// carries — `testdata/fpga/ice40/ff_zoo.v` (every kind of register, both
/// edges), `uart_echo.v`, `bram_check.v` (block RAM in two widths, where the
/// die has any) and both PLL designs (where the package has a PLL) — with
/// every bit decoding, the arcs exactly the router's, the block RAMs and
/// PLLs read back as placed, and the stream unpacking to the same image.
///
/// What it would catch: a die whose fabric does not build, a bit the
/// configuration writes that the die's database does not have (the 1k's
/// input enables and block RAM power-up bits are the other way round from
/// the UltraPlus's, and both are read back here), a PLL placed where the
/// package does not bond it, or the two clock edges sharing a logic tile.
/// What it would not: that the logic is right, which is what IceStorm's
/// own decode, simulated against the source, was for
/// (`docs/fpga-icestorm.md`, "Every iCE40 part"); nor anything a board
/// would, since none was on one.
#[cfg(feature = "verilog")]
#[test]
fn every_part_builds_its_designs_and_every_bit_decodes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga");
    let designs: [TestDesign; 5] = [
        (
            "ff_zoo",
            "ice40/ff_zoo.v",
            &[("rst", 1), ("en", 1), ("d", 1), ("q", 10)],
        ),
        ("uart_echo", "ice40up/uart_echo.v", &[("rx", 1), ("tx", 1)]),
        (
            "bram_check",
            "ice40/bram_check.v",
            &[("we", 1), ("din", 4), ("wide", 16), ("narrow", 8)],
        ),
        ("pll_uart", "ice40up/pll_uart.v", &[("tx", 1)]),
        ("pll2f_uart", "ice40up/pll2f_uart.v", &[("tx", 1)]),
    ];
    // One package per die is enough for the fabric; the parts sharing a
    // die differ in their pins, which the device file test checks.
    let mut dies_done = std::collections::BTreeSet::new();
    for part in &reticle::fpga::icestorm::PARTS {
        if !dies_done.insert(part.die) {
            continue;
        }
        let Some(db) = chipdb(part.die) else { return };
        let (clock, pins) = test_pins(part, &db);
        let fabric = IceFabric::build(db, part.package).expect("the fabric builds");
        let graph = fabric.arch.build_graph();
        let has_ram = graph.sites.iter().any(|s| s.kind == "bram");
        let has_pll = graph.sites.iter().any(|s| s.kind == "pll");
        for (name, file, ports) in designs {
            if (name == "bram_check" && !has_ram) || (name.starts_with("pll") && !has_pll) {
                continue;
            }
            let source = std::fs::read_to_string(root.join(file)).unwrap();
            let mut rcf = format!("set_io -io_standard LVCMOS33 clk {clock}\n");
            let mut free = pins.iter();
            for (port, width) in ports {
                for bit in 0..*width {
                    let signal = if *width == 1 {
                        (*port).to_owned()
                    } else {
                        format!("{port}[{bit}]")
                    };
                    let pin = free.next().expect("enough pins");
                    rcf.push_str(&format!("set_io -io_standard LVCMOS33 {signal} {pin}\n"));
                }
            }
            rcf.push_str("create_clock -name sys -period 83.333 clk\n");
            let built = implement_source_on(part.device, name, &source, &rcf, &fabric, &graph)
                .unwrap_or_else(|e| panic!("{} {name}: {e}", part.device));
            assert!(built.decoded.unexplained.is_empty());
            assert_eq!(built.decoded.arcs.len(), built.chosen);
            if name == "bram_check" {
                assert_eq!(built.rams, 2, "{} {name}", part.device);
                assert_eq!(built.decoded.rams.len(), 2, "{} {name}", part.device);
            }
            if name.starts_with("pll") {
                assert_eq!(built.plls.len(), 1, "{} {name}", part.device);
            }
            let back = Image::from_bin(&built.image.to_bin()).unwrap();
            assert_eq!(back.tiles, built.image.tiles, "{} {name}", part.device);
            assert_eq!(
                back.ram_data, built.image.ram_data,
                "{} {name}",
                part.device
            );
            eprintln!(
                "{} {name}: {} logic cells, {} set bits, {} arcs, {} block RAM(s), {} PLL(s), 0 \
                 unexplained",
                part.device,
                built.pack.cells,
                built.decoded.bits,
                built.chosen,
                built.rams,
                built.plls.len()
            );
        }
    }
}

/// What a part lacks is refused by name: a PLL on a package that does not
/// bond one (the HX1K's VQ100), a PLL on a die with none (the 384), and the
/// UltraPlus's hard blocks on a part without them, which the device file
/// does not declare, so synthesis names the primitive and the device.
///
/// What it would catch: a design built as if the block were there — the
/// PLL placed on a site the package leaves out, or a DSP block's ports left
/// as a black box whose outputs then read as nothing.
#[cfg(feature = "verilog")]
#[test]
fn what_a_part_lacks_is_refused_by_name() {
    use reticle::diag::Diagnostics;
    use reticle::fpga::{Constraints, FpgaOptions, synthesize_for, target};
    use reticle::source::SourceMap;

    let pll = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/ice40up/pll_uart.v"),
    )
    .unwrap();
    for (device, die, package, clock, tx) in [
        ("ice40-hx1k-vq100", "1k", "vq100", "13", "2"),
        ("ice40-lp384-qn32", "384", "qn32", "6", "2"),
    ] {
        let Some(db) = chipdb(die) else { return };
        let fabric = IceFabric::build(db, package).expect("the fabric builds");
        let graph = fabric.arch.build_graph();
        let rcf = format!(
            "set_io clk {clock}\nset_io tx {tx}\ncreate_clock -name ref -period 83.333 clk\n"
        );
        // The device file has no `pll` line for these parts, so the
        // primitive is not even declared for them; synthesized for the
        // UltraPlus, which declares it, the backend still refuses it by
        // name on these fabrics. (Both packages have pins of those names.)
        let device_file = target(device).unwrap();
        assert!(device_file.clock_resources.plls.is_empty(), "{device}");
        let refused =
            implement_source_on("ice40-up5k-sg48", "pll_uart", &pll, &rcf, &fabric, &graph);
        assert!(
            matches!(&refused, Err(e) if e.contains("has no PLL in the") && e.contains("`pll`")),
            "{device}: {refused:?}",
            refused = refused.as_ref().err()
        );
    }
    for (device, primitive, ports) in [
        (
            "ice40-hx8k-ct256",
            "SB_MAC16",
            ".CLK(clk), .A(16'd3), .B(16'd5), .O(o)",
        ),
        (
            "ice40-u4k-sg48",
            "SB_SPRAM256KA",
            ".CLOCK(clk), .DATAOUT(o)",
        ),
        (
            "ice40-lp1k-tq144",
            "SB_HFOSC",
            ".CLKHFEN(1'b1), .CLKHFPU(1'b1), .CLKHF(o[0])",
        ),
    ] {
        let source = format!(
            "module top(input clk, output [31:0] o);\n{primitive} u ({ports});\nendmodule\n"
        );
        let mut map = SourceMap::new();
        let id = map.add("top.v".to_owned(), source).unwrap();
        let mut diags = Diagnostics::new();
        let ast = reticle::verilog::parse_source(
            &mut map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            &mut diags,
        );
        let mut design = reticle::verilog::elaborate_file(
            &ast,
            &reticle::verilog::ElabOptions::default(),
            &mut diags,
        )
        .expect("the design elaborates");
        let top = design.top.expect("a top module");
        let result = synthesize_for(
            &mut design,
            top,
            target(device).unwrap(),
            &Constraints::default(),
            &FpgaOptions::default(),
            &mut diags,
        );
        let rendered = diags.render(&map);
        assert!(result.is_err(), "{device}: {primitive} was accepted");
        assert!(
            rendered.contains(&format!("no module named `{primitive}`"))
                && rendered.contains(&format!("the device `{device}` declares no primitive")),
            "{device}: {rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// The static timing analysis (`icestorm::timing::analyse`) of bitstreams
/// nextpnr placed and routed, against nextpnr's own report of the same
/// bitstream and against IceStorm's `icetime` run on it.
///
/// `RETICLE_ICE40_TIMING_REF` names a directory of `<name>.asc` files (a
/// name `<device>--<design>` is timed as that part, with its own figures
/// and its family's setup; any other as the UltraPlus) with,
/// beside each, `<name>.npr.log` (nextpnr's log: the last `Max frequency
/// for clock` line of each clock) and `<name>.icetime` (`icetime -i -t`'s
/// report: its `Total path delay`), either or both — Reticle's own builds
/// have only the second. None ship; without them this says so and passes.
/// icetime is compared on a design of one clock only: it times every path
/// as one, across clocks too. Each of nextpnr's clocks is matched to the analysis's clock
/// nearest it in frequency.
///
/// What it would catch: a delay model or a graph that disagrees with the
/// tools the iCE40 world times with — a connection's cell, a logic cell's
/// arc, a setup, a register not found or a path through one. nextpnr's
/// model is the one ported (`pip_delay`), so its figures should agree
/// closely; icetime's chooses cells differently and is the independent
/// check. What it would not: whether any of the three is the silicon.
#[cfg(feature = "verilog")]
#[test]
fn the_timing_analysis_agrees_with_nextpnr_and_icetime() {
    let Ok(dir) = std::env::var("RETICLE_ICE40_TIMING_REF") else {
        eprintln!("skipping: RETICLE_ICE40_TIMING_REF names no directory of nextpnr builds");
        return;
    };
    // A file named `<device>--<name>.asc` is of that part, timed with its
    // own figures and its family's setup; any other is the UltraPlus's.
    let mut fabrics: std::collections::BTreeMap<
        &str,
        (IceFabric, reticle::fpga::icestorm::timing::TimingDb),
    > = Default::default();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the directory reads")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "asc"))
        .collect();
    names.sort();
    let mut worst_npr: f64 = 0.0;
    let mut worst_icetime: f64 = 0.0;
    for asc in names {
        let name = asc.file_stem().unwrap().to_string_lossy().into_owned();
        let part = name
            .split_once("--")
            .and_then(|(device, _)| reticle::fpga::icestorm::part(device))
            .or_else(|| reticle::fpga::icestorm::part("ice40-up5k-sg48"))
            .expect("the UltraPlus is a part");
        if !fabrics.contains_key(part.device) {
            let Some(db) = chipdb(part.die) else { return };
            let Some(dir) = database_dir() else { return };
            let Ok(text) = std::fs::read_to_string(dir.join(part.timing)) else {
                eprintln!("skipping: no {}", part.timing);
                return;
            };
            let mut tdb = reticle::fpga::icestorm::timing::TimingDb::parse(&text).unwrap();
            let device = reticle::fpga::target(part.device).expect("the device ships");
            tdb.lut_setup =
                reticle::fpga::icestorm::timing::TimingDb::lut_setup_for(&device.speed_grade);
            let fabric = IceFabric::build(db, part.package).expect("the fabric builds");
            fabrics.insert(part.device, (fabric, tdb));
        }
        let (fabric, tdb) = &fabrics[part.device];
        let image = Image::from_asc(&std::fs::read_to_string(&asc).unwrap()).unwrap();
        let report = reticle::fpga::icestorm::timing::analyse(fabric, &image, tdb);
        let ours: Vec<f64> = report.clocks.iter().map(|c| c.fmax_mhz).collect();
        let dsp = fabric
            .decode(&image)
            .dsps
            .values()
            .any(|d| d.params.values().any(|v| *v != 0));
        // Whether any logic tile's flip-flops are on a falling edge.
        let falling = fabric.decode(&image).functions.keys().any(|((x, y), f)| {
            f == "NegClk" && fabric.db.tile(*x, *y) == reticle::fpga::icestorm::db::TileKind::Logic
        });
        if let Ok(log) = std::fs::read_to_string(asc.with_extension("npr.log")) {
            let mut last: std::collections::BTreeMap<String, f64> =
                std::collections::BTreeMap::new();
            for line in log
                .lines()
                .filter(|l| l.contains("Max frequency for clock"))
            {
                let Some(rest) = line.split("clock").nth(1) else {
                    continue;
                };
                let clock = rest.split('\'').nth(1).unwrap_or("").to_owned();
                let mhz = rest
                    .split(':')
                    .nth(1)
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|f| f.parse::<f64>().ok());
                if let Some(mhz) = mhz {
                    last.insert(clock, mhz);
                }
            }
            for (clock, theirs) in &last {
                let mine = ours
                    .iter()
                    .copied()
                    .min_by(|a, b| (a - theirs).abs().total_cmp(&(b - theirs).abs()));
                let Some(mine) = mine else {
                    panic!("{name}: nextpnr times `{clock}` and the analysis no clock");
                };
                let error = (mine - theirs) / theirs;
                // nextpnr times a path from one edge of a clock to the
                // other as half a period; this analysis does not time it,
                // so a design on both edges is shown and not held to the
                // bound.
                let both_edges = falling;
                if !both_edges && !dsp {
                    worst_npr = worst_npr.max(error.abs());
                }
                eprintln!(
                    "{name}: `{clock}` nextpnr {theirs:.2} MHz, the analysis {mine:.2} MHz \
                     ({:+.1} %){}",
                    100.0 * error,
                    if both_edges {
                        ", both edges: not held"
                    } else {
                        ""
                    }
                );
            }
        }
        // icetime times every path as one, whatever its clocks, so only a
        // design of one clock compares. A design with a DSP block is shown
        // and not held to either bound: nextpnr takes every one of the
        // block's ports for a register, and icetime picks a cell for the
        // block by a few of its bits, where this analysis times an
        // unregistered multiply as logic and anything else as registers
        // (`timing::dsp_arcs`).
        if report.clocks.len() == 1
            && let Ok(text) = std::fs::read_to_string(asc.with_extension("icetime"))
            && let Some(ns) = text
                .lines()
                .find_map(|l| l.strip_prefix("Total path delay: "))
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse::<f64>().ok())
        {
            let mine = report.worst().map_or(0.0, |c| c.delay_ps / 1000.0);
            let error = (mine - ns) / ns;
            if !dsp {
                worst_icetime = worst_icetime.max(error.abs());
            }
            eprintln!(
                "{name}: icetime's longest path {ns:.2} ns, the analysis's {mine:.2} ns ({:+.1} %)",
                100.0 * error
            );
        }
    }
    eprintln!(
        "the largest disagreement: {:.1} % with nextpnr, {:.1} % with icetime (no DSP block)",
        100.0 * worst_npr,
        100.0 * worst_icetime
    );
    assert!(worst_npr <= 0.05, "{:.1} % from nextpnr", 100.0 * worst_npr);
    assert!(
        worst_icetime <= 0.05,
        "{:.1} % from icetime",
        100.0 * worst_icetime
    );
}

/// An unregistered DSP multiply is timed as logic: a fabric register,
/// the 16 x 16 multiplier, and a fabric register again is a path of the
/// multiplier's own arcs and more, longer than the 9.05 ns icetime's slow
/// corner gives the multiplier alone; with every register of the block on,
/// the longest path is a short one.
///
/// What it would catch: the analysis taking a DSP block's ports for
/// registers whatever its configuration, as nextpnr does and as it did,
/// which the board showed wrong (`docs/fpga-icestorm.md`, "How fast the DSP
/// multiplier is"): the unregistered build would then time at the
/// registered one's frequency. What it would not: whether the figures are
/// the silicon's, which they are not (a slow corner); the board measured
/// the path at about 10 ns.
#[cfg(feature = "verilog")]
#[test]
fn an_unregistered_dsp_multiply_is_timed_as_logic() {
    let Some(db) = chipdb_5k() else { return };
    let Some(tdb) = timing_db() else { return };
    let mut fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    fabric.timing = Some(tdb);
    let graph = fabric.arch.build_graph();
    let source = |pipe: u32| {
        format!(
            "module t(input clk, output reg y);\n\
             reg [31:0] l = 32'd0; reg [15:0] a = 16'd0, b = 16'd0;\n\
             always @(posedge clk) begin l <= {{l[30:0], ~(l[31] ^ l[21] ^ l[1] ^ l[0])}}; \
             a <= l[15:0]; b <= l[31:16]; end\n\
             wire [31:0] o;\n\
             SB_MAC16 #(.A_REG({pipe}), .B_REG({pipe}), .TOP_8x8_MULT_REG({pipe}), \
             .BOT_8x8_MULT_REG({pipe}), .PIPELINE_16x16_MULT_REG1({pipe}), \
             .PIPELINE_16x16_MULT_REG2({pipe}), .TOPOUTPUT_SELECT(2'b11), \
             .BOTOUTPUT_SELECT(2'b11)) m (.CLK(clk), .CE(1'b1), .A(a), .B(b), .O(o));\n\
             reg [31:0] p = 32'd0; reg [7:0] f = 8'd0;\n\
             always @(posedge clk) begin p <= o; \
             f <= {{^p[31:28], ^p[27:24], ^p[23:20], ^p[19:16], ^p[15:12], ^p[11:8], ^p[7:4], \
             ^p[3:0]}}; y <= ^f; end\nendmodule\n"
        )
    };
    let worst = |pipe: u32| {
        let built = implement_source(
            "dsp",
            &source(pipe),
            "set_io clk 20\nset_io y 10\n",
            &fabric,
            &graph,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let report = built.timing.expect("timed");
        report.worst().expect("a path").delay_ps / 1000.0
    };
    let (comb, piped) = (worst(0), worst(1));
    eprintln!("longest path: {comb:.2} ns unregistered, {piped:.2} ns with every register on");
    assert!(
        comb > 10.0,
        "the unregistered multiply is a path: {comb:.2} ns"
    );
    assert!(piped < comb - 5.0, "registered: {piped:.2} ns");
}

/// A board design built with the timing figures is timed: its one clock
/// has a longest path, well inside the 83 ns its constraint asks for, and
/// the path starts at a register and ends at one. Built timing-driven, it
/// is still legal (every bit explained, the arcs the router's) and timed.
///
/// What it would catch: the analysis finding no register or no path, the
/// timing-driven flow failing to place or route, its build breaking the
/// two checks. What it would not: whether the figures are right, which
/// `the_timing_analysis_agrees_with_nextpnr_and_icetime` is for.
#[cfg(feature = "verilog")]
#[test]
fn a_board_design_is_timed() {
    let Some(db) = chipdb_5k() else { return };
    let Some(tdb) = timing_db() else { return };
    let mut fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    fabric.timing = Some(tdb);
    let graph = fabric.arch.build_graph();
    for driven in [false, true] {
        fabric.timing_driven = driven;
        let built = build_board_design("uart_echo", &fabric, &graph);
        assert!(built.decoded.unexplained.is_empty());
        assert_eq!(built.decoded.arcs.len(), built.chosen);
        let report = built.timing.expect("timed");
        assert_eq!(report.clocks.len(), 1, "{report:?}");
        let clock = &report.clocks[0];
        assert!(clock.fmax_mhz > 12.0 && clock.fmax_mhz < 500.0, "{clock:?}");
        assert!(clock.nets.len() > 2, "{clock:?}");
        eprintln!(
            "uart_echo{}: {:.2} ns, {:.1} MHz over {} nets",
            if driven { ", timing-driven" } else { "" },
            clock.delay_ps / 1000.0,
            clock.fmax_mhz,
            clock.nets.len()
        );
    }
}

/// What timing-driven placement and routing buy on the iCE40UP5K: each
/// board design built five ways (five seeds) without and with them, the
/// slowest clock's frequency by `timing::analyse` printed for each, and
/// the least, the median and the most of the five. The seeds are the
/// noise: one build against another of the same flow differs by as much
/// as the flows might. Ignored, because it builds sixty designs; it
/// asserts nothing about a speed-up, only that every build is legal and
/// timed. `docs/fpga-icestorm.md` has its figures.
#[cfg(feature = "verilog")]
#[test]
#[ignore = "builds sixty designs; prints what timing-driven placement and routing buy"]
fn the_timing_driven_flow_is_faster() {
    let Some(db) = chipdb_5k() else { return };
    let Some(tdb) = timing_db() else { return };
    let mut fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    fabric.timing = Some(tdb);
    let graph = fabric.arch.build_graph();
    let device = reticle::fpga::target("ice40-up5k-sg48").expect("the device ships");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/ice40up");
    let designs = std::env::var("RETICLE_ICE40_TIMING_DESIGNS").unwrap_or_else(|_| {
        "uart_echo ram_selftest dsp_selftest pll_uart pll2f_uart osc_count".to_owned()
    });
    let seeds: Vec<u64> = (1..=5).collect();
    for name in designs.split_whitespace() {
        let source = std::fs::read_to_string(dir.join(format!("{name}.v"))).unwrap();
        let rcf = std::fs::read_to_string(dir.join(format!("{name}.rcf"))).unwrap();
        let (design, top, mut constraints) = synthesize_source(name, &source, &rcf);
        // `RETICLE_ICE40_TIMING_PERIOD` asks every clock the constraints
        // name for that period instead, so that a design whose own clock
        // leaves every path slack still has something to be driven by.
        if let Some(period) = std::env::var("RETICLE_ICE40_TIMING_PERIOD")
            .ok()
            .and_then(|p| p.parse::<f64>().ok())
        {
            for clock in &mut constraints.clocks {
                clock.period_ns = period;
            }
        }
        let mut line = format!("{name}:");
        for driven in [false, true] {
            fabric.timing_driven = driven;
            let mut fmax: Vec<f64> = Vec::new();
            for &seed in &seeds {
                let options = reticle::fpga::place::PlaceOptions {
                    seed,
                    ..reticle::fpga::place::PlaceOptions::default()
                };
                let built = reticle::fpga::icestorm::implement(
                    &design,
                    top,
                    device,
                    &constraints,
                    &fabric,
                    &graph,
                    &options,
                )
                .unwrap_or_else(|e| panic!("{name} seed {seed}: {e}"));
                let report = built.timing.expect("timed");
                let worst = report.worst().expect("a path");
                fmax.push(worst.fmax_mhz);
            }
            let mut sorted = fmax.clone();
            sorted.sort_by(f64::total_cmp);
            line.push_str(&format!(
                " {} {:.1}/{:.1}/{:.1} MHz ({})",
                if driven {
                    "timing-driven"
                } else {
                    "wirelength"
                },
                sorted[0],
                sorted[sorted.len() / 2],
                sorted[sorted.len() - 1],
                fmax.iter()
                    .map(|f| format!("{f:.1}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        eprintln!("{line}");
    }
}

// ---------------------------------------------------------------------------
// The PLL
// ---------------------------------------------------------------------------

/// Reticle's PLL solver (`fpga::pll::solve`, on the UltraPlus's
/// `SB_PLL40_CORE` line) against Project IceStorm's own `icepll`, for
/// every output frequency from 16 to 275 MHz in steps of 0.1 MHz from a 12
/// MHz reference and of 0.5 MHz from four others: the same DIVR, DIVF,
/// DIVQ and FILTER_RANGE every time.
///
/// `RETICLE_ICEPLL` names an `icepll` binary (built from IceStorm's
/// `icepll/`); without it this says so and passes. What it would catch: a
/// range, a field's encoding, a filter band edge or the tie-break between
/// equally good settings differing from the tool every iCE40 design gets its
/// settings from. What it would not: whether either is what the silicon
/// does, which the board test says for one setting.
#[test]
fn the_pll_solver_gives_what_icepll_gives() {
    let Ok(icepll) = std::env::var("RETICLE_ICEPLL") else {
        eprintln!("skipping: RETICLE_ICEPLL names no icepll binary");
        return;
    };
    let device = reticle::fpga::target("ice40-up5k-sg48").expect("the device ships");
    let shape = device
        .clock_resources
        .plls
        .iter()
        .find(|p| p.name == "SB_PLL40_CORE")
        .expect("the UltraPlus has an SB_PLL40_CORE");
    let mut compared = 0usize;
    let mut differ = Vec::new();
    for (input, step) in [(12.0, 1), (10.0, 5), (25.0, 5), (30.0, 5), (100.0, 5)] {
        for tenths in (160..=2750).step_by(step) {
            let out = f64::from(tenths) / 10.0;
            let run = std::process::Command::new(&icepll)
                .args(["-i", &format!("{input}"), "-o", &format!("{out}")])
                .output()
                .expect("running icepll");
            let text = String::from_utf8_lossy(&run.stdout).into_owned();
            let field = |key: &str| -> Option<i64> {
                text.lines()
                    .find(|l| l.starts_with(key))?
                    .split_whitespace()
                    .nth(1)?
                    .parse()
                    .ok()
            };
            let theirs = ["DIVR:", "DIVF:", "DIVQ:", "FILTER_RANGE:"].map(field);
            let ours = reticle::fpga::pll::solve(shape, input, out)
                .ok()
                .map(|s| ["DIVR", "DIVF", "DIVQ", "FILTER_RANGE"].map(|k| s.param(k)));
            let theirs = theirs.iter().all(Option::is_some).then_some(theirs);
            compared += 1;
            if theirs != ours {
                differ.push(format!(
                    "{input} -> {out}: icepll {theirs:?}, Reticle {ours:?}"
                ));
            }
        }
    }
    assert!(
        differ.is_empty(),
        "{} of {compared} differ: {:?}",
        differ.len(),
        &differ[..differ.len().min(20)]
    );
    eprintln!("{compared} frequencies, every setting the same as icepll's");
}

/// The two PLL designs build with the PLL placed, every bit explained, and
/// its parameters read back from the bits as the instance asks for them:
/// `PLLTYPE` 3 for `SB_PLL40_CORE` and 7 for `SB_PLL40_2F_CORE`, `SIMPLE`
/// feedback (1), DIVR 0, DIVF 63, DIVQ 4, FILTER_RANGE 1, port B
/// `GENCLK_HALF` (1) in the second; and the global networks the PLL drives
/// through its outputs' pads, 7 and, for the second output, 2.
///
/// What it would catch: a parameter lost or mistranslated between the
/// RTL's `#(...)` and the bits (a name nextpnr's packer translates, such
/// as `FEEDBACK_PATH`, read as a number), a pad-input bit missing or
/// extra, the `LOCK` cell not where its wire reaches (no route). What it
/// would not: a parameter consistently in the wrong bit both ways, which
/// the board and nextpnr's build are for.
#[cfg(feature = "verilog")]
#[test]
fn the_pll_designs_build_and_read_back() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    for (name, plltype, select_b, networks) in [
        ("pll_uart", 3, 0, vec![7]),
        ("pll2f_uart", 7, 1, vec![2, 7]),
    ] {
        let built = build_board_design(name, &fabric, &graph);
        assert!(built.decoded.unexplained.is_empty());
        assert_eq!(built.decoded.arcs.len(), built.chosen);
        assert_eq!(built.plls.len(), 1, "{name}: one PLL was placed");
        let (tile, pll) = built.plls.iter().next().expect("a PLL was placed");
        assert_eq!(built.decoded.plls.get(tile), Some(pll));
        let want: Vec<(&str, u64)> = vec![
            ("PLLTYPE", plltype),
            ("FEEDBACK_PATH", 1),
            ("DIVR", 0),
            ("DIVF", 63),
            ("DIVQ", 4),
            ("FILTER_RANGE", 1),
            ("PLLOUT_SELECT_A", 0),
            ("PLLOUT_SELECT_B", select_b),
        ];
        for (key, value) in want {
            assert_eq!(pll.params.get(key), Some(&value), "{name}: {key}");
        }
        assert_eq!(
            built.decoded.padin.iter().copied().collect::<Vec<u32>>(),
            networks,
            "{name}: the networks the PLL drives"
        );
        eprintln!(
            "{name}: {} logic cells, {} set bits, {} arcs, the PLL read back, 0 unexplained",
            built.pack.cells, built.decoded.bits, built.chosen
        );
    }
}

/// `osc_count.v` builds with both oscillators placed and each one's clock
/// on its global network through that network's pad-input bit, 4 for the
/// high-frequency one and 5 for the low: the bits nextpnr sets for them
/// (`create_padin_gbuf`), without which neither clock reached anything on
/// the board.
///
/// What it would catch: a pad-input bit dropped or on the wrong network,
/// or a global buffer put behind an oscillator, which cannot be routed.
/// What it would not: whether the oscillators run, which the board says.
#[cfg(feature = "verilog")]
#[test]
fn the_oscillators_drive_their_networks() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let built = build_board_design("osc_count", &fabric, &graph);
    assert!(built.decoded.unexplained.is_empty());
    assert_eq!(built.decoded.arcs.len(), built.chosen);
    assert_eq!(
        built.decoded.padin.iter().copied().collect::<Vec<u32>>(),
        vec![4, 5]
    );
    eprintln!(
        "osc_count: {} logic cells, {} set bits, {} arcs, networks 4 and 5 from their pads' \
         input path, 0 unexplained",
        built.pack.cells, built.decoded.bits, built.chosen
    );
}

/// What the PLL refuses, each by name: a pad form on a pin that is not the
/// PLL's own pad (until 2026-11-19 every pad form was refused, pin 35
/// included), and a design
/// that also wants pin 35 as a pin while the PLL's first output leaves
/// through that pin's IO block. The second used to be refused by the
/// placer, because the PLL's `out` and the IO block's `din` are one wire
/// of one tile; since a die may have two PLLs, each PLL is given a site
/// before placement, and one whose output would take a pin the design uses
/// is refused there, saying so.
#[cfg(feature = "verilog")]
#[test]
fn a_pll_the_flow_cannot_place_is_refused() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let pad = "module t(input p, output o);\n\
               wire c;\n\
               SB_PLL40_PAD #(.DIVF(7'd63), .DIVQ(3'd4), .FILTER_RANGE(3'd1)) pll \
               (.PACKAGEPIN(p), .PLLOUTCORE(c), .RESETB(1'b1), .BYPASS(1'b0));\n\
               reg q = 1'b0; always @(posedge c) q <= ~q; assign o = q;\n\
               endmodule\n";
    // Pin 35 is the PLL's own pad, so the pad form builds there now
    // (`the_pll_pad_forms_build_on_every_die`); on pin 20 it is refused.
    let built = implement_source("pad", pad, "set_io p 35\nset_io o 4\n", &fabric, &graph);
    assert!(built.is_ok(), "{:?}", built.as_ref().err());
    let refused = implement_source("pad", pad, "set_io p 20\nset_io o 4\n", &fabric, &graph);
    assert!(
        matches!(&refused, Err(e) if e.contains("SB_PLL40_PAD") && e.contains("`pll`")
            && e.contains("pin(s) 35")),
        "{:?}",
        refused.as_ref().err()
    );
    let both = "module t(input clk, input p, output o);\n\
                wire c;\n\
                SB_PLL40_CORE #(.DIVF(7'd63), .DIVQ(3'd4), .FILTER_RANGE(3'd1)) pll \
                (.REFERENCECLK(clk), .PLLOUTCORE(c), .RESETB(1'b1), .BYPASS(1'b0));\n\
                reg q = 1'b0; always @(posedge c) q <= p; assign o = q;\n\
                endmodule\n";
    let refused = implement_source(
        "both",
        both,
        "set_io clk 20\nset_io p 35\nset_io o 4\n",
        &fabric,
        &graph,
    );
    assert!(
        matches!(&refused, Err(e) if e.contains("`pll`")
            && e.contains("through an IO block the design uses as a pin")),
        "{:?}",
        refused.as_ref().err()
    );
}

// ---------------------------------------------------------------------------
// The UltraPlus's other hard IP, and warm boot
// ---------------------------------------------------------------------------

/// The board designs of the I2C and SPI blocks, the LED dimmer, the
/// open-drain pads and warm boot build with every bit decoding and the
/// arcs exactly the router's, and the hard cells' configuration bits read
/// back as placed: the SPI block on bus address `0b0000` is the one at
/// (0, 0) with its four enable bits, the I2C block on `0b0001` the one at
/// (0, 31) with both enables and both SDA delays (on unless an attribute
/// says otherwise), the RGB driver with the dimmer's currents.
///
/// What it would catch: a block on the wrong site for its bus address
/// (another block's enables set, which the read-back refuses), an enable
/// written into another tile's bit, a port on a wire that is not the
/// block's (no route), or an `SB_IO_OD` not on a pad (refused). What it
/// would not: whether the blocks work, which is the board's
/// (`docs/fpga-icestorm.md`, "The UltraPlus's other hard IP").
#[cfg(feature = "verilog")]
#[test]
fn the_hard_ip_designs_build_and_read_back() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let key = |kind: &str, x: u32, y: u32| (kind.to_owned(), (x, y));
    let built = build_board_design("sbus_probe", &fabric, &graph);
    let spi = built
        .decoded
        .hard_ip
        .get(&key("SPI", 0, 0))
        .expect("the SPI block");
    assert_eq!(spi.get("SPI_ENABLE"), Some(&15));
    let i2c = built
        .decoded
        .hard_ip
        .get(&key("I2C", 0, 31))
        .expect("the I2C block");
    assert_eq!(i2c.get("I2C_ENABLE"), Some(&3));
    assert_eq!(i2c.get("SDA_INPUT_DELAYED"), Some(&1));
    assert_eq!(i2c.get("SDA_OUTPUT_DELAYED"), Some(&1));
    assert_eq!(
        built.decoded.hard_ip.len(),
        2,
        "{:?}",
        built.decoded.hard_ip
    );
    let built = build_board_design("ledda_breathe", &fabric, &graph);
    let rgb = built
        .decoded
        .hard_ip
        .get(&key("RGBA_DRV", 0, 30))
        .expect("the RGB driver");
    assert_eq!(rgb.get("RGB0_CURRENT"), Some(&3));
    assert_eq!(rgb.get("CURRENT_MODE"), Some(&1));
    assert_eq!(built.hard_cells, 2, "the dimmer and the driver");
    for name in ["io_od_blink", "warmboot_demo"] {
        let built = build_board_design(name, &fabric, &graph);
        assert!(built.decoded.unexplained.is_empty());
        assert_eq!(built.decoded.arcs.len(), built.chosen);
        eprintln!(
            "{name}: {} set bits, {} arcs",
            built.decoded.bits, built.chosen
        );
    }
}

/// What the hard IP refuses, by name: an `SB_IO_OD` on a pad that is not
/// one of the three open-drain ones, an `SB_IO_I3C` on a pad that is not
/// an I3C one, and an I2C block whose bus address is an SPI block's. And an
/// `SB_IO_I3C` on pin 23, one of the two I3C pads, builds, its pull-up
/// enables routed to where the `.extra_cell IO_I3C` record says.
///
/// What it would catch: a special pad's primitive placed on any IO block,
/// which would build and, on the silicon, not be open-drain or have no
/// switchable pull-ups; a bus address mapped to the wrong block.
#[cfg(feature = "verilog")]
#[test]
fn what_the_hard_ip_refuses() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let od = "module t(input clk, inout p);\n\
              SB_IO_OD #(.PIN_TYPE(6'b011001)) o (.PACKAGEPIN(p), .DOUT0(clk));\nendmodule\n";
    let refused = implement_source("od", od, "set_io clk 20\nset_io p 2\n", &fabric, &graph);
    assert!(
        matches!(&refused, Err(e) if e.contains("SB_IO_OD") && e.contains("open-drain")),
        "{:?}",
        refused.as_ref().err()
    );
    let built = implement_source("od", od, "set_io clk 20\nset_io p 40\n", &fabric, &graph);
    assert!(built.is_ok(), "{:?}", built.as_ref().err());
    let i3c = "module t(input clk, input a, input b, inout p);\n\
               SB_IO_I3C #(.PIN_TYPE(6'b101001)) o (.PACKAGE_PIN(p), .D_OUT_0(clk), \
               .OUTPUT_ENABLE(a), .PU_ENB(a), .WEAK_PU_ENB(b));\nendmodule\n";
    let pins = |p: u32| format!("set_io clk 20\nset_io a 2\nset_io b 3\nset_io p {p}\n");
    let built =
        implement_source("i3c", i3c, &pins(23), &fabric, &graph).unwrap_or_else(|e| panic!("{e}"));
    assert!(built.decoded.unexplained.is_empty());
    let refused = implement_source("i3c", i3c, &pins(4), &fabric, &graph);
    assert!(
        matches!(&refused, Err(e) if e.contains("SB_IO_I3C") && e.contains("I3C pad")),
        "{:?}",
        refused.as_ref().err()
    );
    let bus = "module t(input clk, output o);\n\
               SB_I2C #(.BUS_ADDR74(\"0b0010\")) i (.SBCLKI(clk), .SBACKO(o));\nendmodule\n";
    let refused = implement_source("bus", bus, "set_io clk 20\nset_io o 2\n", &fabric, &graph);
    assert!(
        matches!(&refused, Err(e) if e.contains("BUS_ADDR74") && e.contains("SB_I2C")),
        "{:?}",
        refused.as_ref().err()
    );
}

/// The PLL's pad forms on every die with a PLL: an `SB_PLL40_PAD` whose
/// pad is the pin of its first output's IO block — the PLL's own pad, pin
/// 35 on the UltraPlus's SG48 — builds with every bit decoding, reads back
/// as `PLLTYPE` 2, its pad's input enabled, and the two-output forms as 4
/// and 6; a pad on any other pin is refused, naming the pads there are.
///
/// What it would catch: a pad form placed on a PLL whose pad is not the
/// design's pin (its reference would be a pin nothing drives), the pad's
/// input left disabled, or the wrong `PLLTYPE`. What it would not: whether
/// the PLL locks to its pad, which no board here could show — the
/// oscillator of the one attached is on pin 20, not the PLL's pad.
#[cfg(feature = "verilog")]
#[test]
fn the_pll_pad_forms_build_on_every_die() {
    let pll = |primitive: &str, second: &str| {
        format!(
            "module t(input p, output o, output q);\n\
             wire c, d;\n\
             {primitive} #(.FEEDBACK_PATH(\"SIMPLE\"), .DIVR(4'd0), .DIVF(7'd63), .DIVQ(3'd4), \
             .FILTER_RANGE(3'd1)) pll (.PACKAGEPIN(p), .PLLOUTGLOBAL{second}(c), .RESETB(1'b1), \
             .BYPASS(1'b0));\n\
             reg [3:0] n = 4'd0; always @(posedge c) n <= n + 4'd1; assign o = n[3];\n\
             assign q = 1'b0;\nendmodule\n"
        )
    };
    for (device, die, package) in [
        ("ice40-up5k-sg48", "5k", "sg48"),
        ("ice40-hx1k-tq144", "1k", "tq144"),
        ("ice40-hx8k-ct256", "8k", "ct256"),
        ("ice40-u4k-sg48", "u4k", "sg48"),
    ] {
        let Some(db) = chipdb(die) else { return };
        let fabric = IceFabric::build(db, package).expect("the fabric builds");
        let graph = fabric.arch.build_graph();
        let part = reticle::fpga::icestorm::part(device).unwrap();
        let (_, others) = test_pins(part, &fabric.db);
        let pad_pin = |cell: &reticle::fpga::icestorm::db::ExtraCell| {
            let (_, x, y, z) = cell.entries.iter().find(|(k, ..)| k == "PLLOUT_A")?;
            let z: u32 = z.parse().ok()?;
            fabric
                .arch
                .pinmap
                .iter()
                .find(|(_, s)| *s == format!("X{x}Y{y}/io{z}"))
                .map(|(p, _)| p.clone())
        };
        let pad = fabric.plls().find_map(pad_pin).expect("a PLL pad");
        let rcf = format!(
            "set_io p {pad}\nset_io o {}\nset_io q {}\n",
            others[0], others[1]
        );
        for (primitive, second, plltype) in [
            ("SB_PLL40_PAD", "", 2u64),
            ("SB_PLL40_2_PAD", "B", 4),
            ("SB_PLL40_2F_PAD", "A", 6),
        ] {
            let source = pll(primitive, second);
            let built = implement_source_on(device, primitive, &source, &rcf, &fabric, &graph)
                .unwrap_or_else(|e| panic!("{device} {primitive}: {e}"));
            assert!(built.decoded.unexplained.is_empty());
            assert_eq!(built.decoded.arcs.len(), built.chosen);
            let (_, config) = built.plls.iter().next().expect("a PLL");
            assert_eq!(
                config.params.get("PLLTYPE"),
                Some(&plltype),
                "{device} {primitive}"
            );
            eprintln!(
                "{device} {primitive} on pin {pad}: {} set bits, {} arcs, PLLTYPE {plltype}",
                built.decoded.bits, built.chosen
            );
        }
        let rcf = format!(
            "set_io p {}\nset_io o {}\nset_io q {}\n",
            others[2], others[0], others[1]
        );
        let refused = implement_source_on(
            device,
            "pad",
            &pll("SB_PLL40_PAD", ""),
            &rcf,
            &fabric,
            &graph,
        );
        assert!(
            matches!(&refused, Err(e) if e.contains("no free PLL's own pad") && e.contains(&pad)),
            "{device}: {:?}",
            refused.as_ref().err()
        );
    }
}

// ---------------------------------------------------------------------------
// The DSP blocks
// ---------------------------------------------------------------------------

/// A DSP block asking for its top partial product registered and its
/// middle ones not, while its 16 x 16 product reaches an output, is refused
/// by name: on the silicon one bit registers the top product and one middle
/// product together, so no bitstream holds that. The same parameters with
/// the 16 x 16 product unused build, and so do both registers asked for
/// together.
///
/// What it would catch: the backend quietly writing a configuration the
/// silicon computes differently from the library cell, as nextpnr does for
/// this one. What it would not: whether the refusal's reasoning is right,
/// which is the board's (`docs/fpga-icestorm.md`).
#[cfg(feature = "verilog")]
#[test]
fn a_dsp_block_the_silicon_cannot_hold_is_refused() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let rcf = "set_io a 2\nset_io b 3\nset_io clk 20\nset_io o 4\n";
    let design = |params: &str| {
        format!(
            "module t(input clk, input [15:0] a, input [15:0] b, output o);\n\
             wire [31:0] p;\n\
             SB_MAC16 #({params}) m (.CLK(clk), .CE(1'b1), .A(a), .B(b), .O(p));\n\
             assign o = ^p;\n\
             endmodule\n"
        )
    };
    // Every pin but the clock is one wide here: four of the operands' bits
    // are on pins, the rest tied, which is enough to reach the cell.
    let source = |params: &str| {
        design(params)
            .replace("input [15:0] a, input [15:0] b", "input a, input b")
            .replace(".A(a), .B(b)", ".A({15'd0, a}), .B({15'd0, b})")
    };
    let refused = implement_source(
        "split",
        &source(".TOP_8x8_MULT_REG(1'b1), .TOPOUTPUT_SELECT(2'd3), .BOTOUTPUT_SELECT(2'd3)"),
        rcf,
        &fabric,
        &graph,
    );
    let Err(message) = refused else {
        panic!("a top product registered alone beside a used 16 x 16 product was built")
    };
    assert!(
        message.contains("TOP_8x8_MULT_REG") && message.contains("`m`"),
        "{message}"
    );
    for params in [
        ".TOP_8x8_MULT_REG(1'b1), .MODE_8x8(1'b1), .TOPOUTPUT_SELECT(2'd2)",
        ".TOP_8x8_MULT_REG(1'b1), .PIPELINE_16x16_MULT_REG1(1'b1), .TOPOUTPUT_SELECT(2'd3)",
    ] {
        if let Err(e) = implement_source("split", &source(params), rcf, &fabric, &graph) {
            panic!("{params}: {e}");
        }
    }
}

/// Every bit of every parameter an `SB_MAC16` holds changes something
/// `dsp_selftest.v` prints, in at least one of its eight blocks, under the
/// model of the cell: so a bit the silicon holds differently from what the
/// backend meant — written into the wrong `CBIT`, or two swapped — makes
/// the board's report differ from the model's, and the board test fails.
///
/// This is the mutation check on the board test, run without a board: each
/// mutant is one block with one parameter bit flipped. What it would not
/// catch: a fault the model shares with the bits, since the model is the
/// reference; and it says nothing about the routing, which the board does.
#[test]
fn every_dsp_parameter_bit_shows_in_the_report() {
    use ice40_mac16::{BLOCKS, params, replay, vectors};
    use reticle::fpga::icestorm::MAC16_PARAMS;
    let vs = vectors();
    let mut missed = Vec::new();
    for (name, width) in MAC16_PARAMS {
        for bit in 0..width {
            let seen_by: Vec<&str> = BLOCKS
                .iter()
                .filter(|block| {
                    let p = params(block);
                    let mut q = p.clone();
                    *q.entry(name).or_insert(0) ^= 1 << bit;
                    replay(&p, &block.2, &vs) != replay(&q, &block.2, &vs)
                })
                .map(|block| block.0)
                .collect();
            if seen_by.is_empty() {
                missed.push(format!("{name}[{bit}]"));
            }
        }
    }
    assert!(
        missed.is_empty(),
        "no block of dsp_selftest.v shows these parameter bits: {missed:?}"
    );
}

/// The model itself, on the cases a 16 x 16 multiplier is defined by:
/// unsigned and signed products of the corners, the 8 x 8 split's two
/// products, and an accumulator adding two products. Without this the
/// board test would compare the silicon with a model nobody checked.
#[test]
fn the_dsp_model_multiplies_and_accumulates() {
    use ice40_mac16::{Inputs, Params, State, step};
    let p = |pairs: &[(&'static str, u64)]| -> Params { pairs.iter().copied().collect() };
    let mul = p(&[("TOPOUTPUT_SELECT", 3), ("BOTOUTPUT_SELECT", 3)]);
    let smul = p(&[
        ("TOPOUTPUT_SELECT", 3),
        ("BOTOUTPUT_SELECT", 3),
        ("A_SIGNED", 1),
        ("B_SIGNED", 1),
    ]);
    let split = p(&[
        ("MODE_8x8", 1),
        ("TOPOUTPUT_SELECT", 2),
        ("BOTOUTPUT_SELECT", 2),
    ]);
    let at = |p: &Params, a: u16, b: u16| {
        step(
            p,
            &State::default(),
            &Inputs {
                a,
                b,
                ..Inputs::default()
            },
        )
        .o
    };
    for (a, b) in [
        (0u16, 0u16),
        (0xffff, 0xffff),
        (0x8000, 0x8000),
        (0xffff, 0x8000),
        (0x7fff, 0x8000),
        (0x1234, 0xfedc),
        (0x00ff, 0xff00),
    ] {
        assert_eq!(
            at(&mul, a, b),
            u32::from(a) * u32::from(b),
            "{a:04x} * {b:04x}"
        );
        let signed = i32::from(a.cast_signed()) * i32::from(b.cast_signed());
        assert_eq!(
            at(&smul, a, b),
            signed.cast_unsigned(),
            "{a:04x} * {b:04x} signed"
        );
        let hi = u32::from(a >> 8) * u32::from(b >> 8);
        let lo = u32::from(a & 0xff) * u32::from(b & 0xff);
        assert_eq!(at(&split, a, b), hi << 16 | lo, "{a:04x} x {b:04x} split");
    }
    // An accumulator: H's halves into Q and S, the top adding the bottom's
    // carry.
    let mac = p(&[
        ("TOPOUTPUT_SELECT", 1),
        ("BOTOUTPUT_SELECT", 1),
        ("TOPADDSUB_LOWERINPUT", 2),
        ("BOTADDSUB_LOWERINPUT", 2),
        ("TOPADDSUB_CARRYSELECT", 2),
    ]);
    let mut st = State::default();
    let mut sum = 0u32;
    for (a, b) in [(0xffffu16, 0xffffu16), (0x8000, 0x0003), (0x1234, 0x5678)] {
        let s = step(
            &mac,
            &st,
            &Inputs {
                a,
                b,
                ..Inputs::default()
            },
        );
        assert_eq!(s.o, sum, "the accumulator before adding {a:04x} * {b:04x}");
        sum = sum.wrapping_add(u32::from(a) * u32::from(b));
        st = s.next;
    }
}

/// `dsp_selftest.v` builds with all eight DSP blocks placed, every bit
/// explained, and each block's parameters read back from the bits as its
/// instance asks for them: the eight configurations read back are the
/// eight [`ice40_mac16::BLOCKS`] says the RTL has.
///
/// What it would catch: a parameter lost between the RTL's `#(...)` and
/// the bits (every block would read back as zero, or as another's), a
/// `CBIT` the database names that the image does not hold, a block that
/// cannot be placed or routed to. What it would not: a parameter written
/// into the wrong `CBIT` consistently both ways, which is what the board
/// test and `every_dsp_parameter_bit_shows_in_the_report` are for.
#[cfg(feature = "verilog")]
#[test]
fn the_dsp_design_builds_and_reads_back() {
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    let graph = fabric.arch.build_graph();
    let built = build_board_design("dsp_selftest", &fabric, &graph);
    assert!(built.decoded.unexplained.is_empty());
    assert_eq!(built.decoded.arcs.len(), built.chosen);
    assert_eq!(built.dsps, 8, "every DSP block");
    let mut read: Vec<Vec<(String, u64)>> = built
        .decoded
        .dsps
        .values()
        .map(|d| {
            d.params
                .iter()
                .filter(|(_, v)| **v != 0)
                .map(|(k, v)| (k.clone(), *v))
                .collect()
        })
        .collect();
    // The bits IceStorm names `TOP_8x8_MULT_REG` and
    // `PIPELINE_16x16_MULT_REG1` hold the library's parameters of the other
    // name (`DspConfig::as_bits`), which only `mul_8` sets differently.
    let swap = |k: &'static str| match k {
        "TOP_8x8_MULT_REG" => "PIPELINE_16x16_MULT_REG1",
        "PIPELINE_16x16_MULT_REG1" => "TOP_8x8_MULT_REG",
        other => other,
    };
    let mut want: Vec<Vec<(String, u64)>> = ice40_mac16::BLOCKS
        .iter()
        .map(|b| {
            let mut v: Vec<(String, u64)> =
                b.1.iter().map(|(k, v)| (swap(k).to_owned(), *v)).collect();
            v.sort();
            v
        })
        .collect();
    read.sort();
    want.sort();
    assert_eq!(read, want);
    eprintln!(
        "dsp_selftest: {} logic cells, {} set bits, {} arcs, 8 DSP blocks read back, 0 \
         unexplained",
        built.pack.cells, built.decoded.bits, built.chosen
    );
}

// ---------------------------------------------------------------------------
// On the board
// ---------------------------------------------------------------------------

/// The board checks, built with Reticle's own flow, loaded into the
/// configuration memory and checked over the FT2232H's second channel.
///
/// Every test here is `#[ignore]`d, because it needs an iCE40UP5K-SG48
/// board with an FT2232H whose first channel is wired as `iceprog` expects
/// and whose second is a serial line to pins 9 (into the FPGA) and 10 (out
/// of it), with a 12 MHz clock on pin 20:
///
/// ```sh
/// cargo test --all-features --test fpga_icestorm board -- --ignored --test-threads 1
/// ```
///
/// Each skips, saying why, when the database, the FT2232H or its serial
/// port is absent. The serial port is found through sysfs as interface 1
/// of the `0403:6010` device, so another adapter's `/dev/ttyUSB1` is never
/// opened by mistake; `RETICLE_ICE40_TTY` names one instead. The line is
/// set up with `stty`, which is why these run on Linux only.
///
/// Nothing here writes the flash: each test leaves its design running in
/// the configuration memory, and a power cycle or `reticle program --ice40
/// --boot` brings the flash's image back.
#[cfg(all(feature = "verilog", feature = "program"))]
mod board {
    use super::{IceFabric, build_board_design, chipdb_5k};
    use reticle::program::{ProgramError, ftdi, ice40, usb};
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// The FT2232H's second channel as a tty, found through sysfs.
    fn find_tty() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("RETICLE_ICE40_TTY") {
            return Some(PathBuf::from(path));
        }
        let entries = std::fs::read_dir("/sys/class/tty").ok()?;
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("ttyUSB"))
            .collect();
        names.sort();
        for name in names {
            let Ok(dev) = std::fs::canonicalize(format!("/sys/class/tty/{name}/device")) else {
                continue;
            };
            let Some(iface) = dev.parent() else { continue };
            let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
            let number = read(iface.join("bInterfaceNumber"));
            let Some(usb) = iface.parent() else { continue };
            if number.trim() == "01"
                && read(usb.join("idVendor")).trim() == "0403"
                && read(usb.join("idProduct")).trim() == "6010"
            {
                return Some(PathBuf::from(format!("/dev/{name}")));
            }
        }
        None
    }

    /// What came over the line, with the breaks and framing errors the
    /// tty marked kept apart from the bytes.
    #[derive(Debug, Default)]
    pub(super) struct Received {
        pub(super) bytes: Vec<u8>,
        /// Breaks: the line held low for longer than a frame.
        pub(super) breaks: usize,
        /// Bytes received with a framing error, not in `bytes`.
        pub(super) bad_frames: Vec<u8>,
    }

    /// A serial line at 115200 baud, 8N1, raw, with reads that time out
    /// after a tenth of a second and breaks marked (`PARMRK`), so a break
    /// is told apart from a received `0x00`.
    pub(super) struct Serial {
        file: std::fs::File,
        /// Undecoded bytes: a `0xFF` marker split across two reads.
        pending: Vec<u8>,
    }

    impl Serial {
        fn open(path: &PathBuf) -> Option<Serial> {
            let status = std::process::Command::new("stty")
                .arg("-F")
                .arg(path)
                .args([
                    "115200", "cs8", "-cstopb", "-parenb", "raw", "-echo", "clocal", "-crtscts",
                    "-ixon", "-ixoff", "-ignbrk", "-brkint", "-ignpar", "parmrk", "-istrip",
                    "-inpck", "min", "0", "time", "1",
                ])
                .status();
            match status {
                Ok(s) if s.success() => {}
                other => {
                    eprintln!(
                        "skipping: `stty` could not set up {}: {other:?}",
                        path.display()
                    );
                    return None;
                }
            }
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                Ok(file) => Some(Serial {
                    file,
                    pending: Vec::new(),
                }),
                Err(e) => {
                    eprintln!("skipping: cannot open {}: {e}", path.display());
                    None
                }
            }
        }

        pub(super) fn send(&mut self, bytes: &[u8]) {
            self.file
                .write_all(bytes)
                .expect("writing to the serial line");
        }

        /// Reads into `into` until `done` says so or nothing arrives for
        /// `quiet`, never past `limit`.
        pub(super) fn read(
            &mut self,
            into: &mut Received,
            quiet: Duration,
            limit: Duration,
            done: impl Fn(&Received) -> bool,
        ) {
            let start = Instant::now();
            let mut last = Instant::now();
            let mut buf = [0u8; 4096];
            while start.elapsed() < limit && !done(into) {
                let n = self.file.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    if last.elapsed() >= quiet {
                        break;
                    }
                    continue;
                }
                last = Instant::now();
                self.pending.extend_from_slice(&buf[..n]);
                self.decode(into);
            }
            // A marker left half-read at the end is a byte after all.
            if self.pending == [0xFF] {
                into.bytes.push(0xFF);
                self.pending.clear();
            }
        }

        /// `PARMRK` with `ISTRIP` off: `FF FF` is a received `FF`, `FF 00
        /// 00` a break, `FF 00 x` the byte `x` with a framing error.
        fn decode(&mut self, into: &mut Received) {
            let p = std::mem::take(&mut self.pending);
            let mut i = 0;
            while i < p.len() {
                if p[i] != 0xFF {
                    into.bytes.push(p[i]);
                    i += 1;
                    continue;
                }
                match (p.get(i + 1), p.get(i + 2)) {
                    (Some(0xFF), _) => {
                        into.bytes.push(0xFF);
                        i += 2;
                    }
                    (Some(0), Some(0)) => {
                        into.breaks += 1;
                        i += 3;
                    }
                    (Some(0), Some(&x)) => {
                        into.bad_frames.push(x);
                        i += 3;
                    }
                    _ => {
                        self.pending = p[i..].to_vec();
                        return;
                    }
                }
            }
        }

        /// Everything waiting, read and returned.
        pub(super) fn drain(&mut self) -> Received {
            let mut got = Received::default();
            self.read(
                &mut got,
                Duration::from_millis(200),
                Duration::from_secs(5),
                |_| false,
            );
            got
        }
    }

    /// Builds `testdata/fpga/ice40up/<name>.v` with Reticle's flow, or
    /// `None` with the reason printed.
    ///
    /// `RETICLE_ICE40_LOAD` names a `.bin` to load instead — another
    /// flow's build of the same design, nextpnr's say, so that the same
    /// host check judges both. Run one test at a time with it set.
    pub(super) fn build(name: &str) -> Option<Vec<u8>> {
        if let Ok(path) = std::env::var("RETICLE_ICE40_LOAD") {
            eprintln!("{name}: loading {path} instead of building it");
            return Some(std::fs::read(&path).expect("reading RETICLE_ICE40_LOAD"));
        }
        let db = chipdb_5k()?;
        let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
        let graph = fabric.arch.build_graph();
        let built = build_board_design(name, &fabric, &graph);
        assert!(built.decoded.unexplained.is_empty());
        assert_eq!(built.decoded.arcs.len(), built.chosen);
        eprintln!(
            "{name}: {} logic cells, {} set bits, {} arcs, 0 unexplained",
            built.pack.cells, built.decoded.bits, built.chosen
        );
        Some(built.image.to_bin())
    }

    /// The board, ready: the serial line open and drained, then `bin`
    /// loaded into the configuration memory. `None`, with the reason
    /// printed, when either is absent.
    ///
    /// The line is opened *before* the load and left open, so whatever the
    /// FPGA's transmit pin does while it is held in reset and configured is
    /// read by the first read after this returns.
    pub(super) fn load(bin: &[u8]) -> Option<Serial> {
        let Some(tty) = find_tty() else {
            eprintln!(
                "skipping: no FT2232H serial port (interface 1 of 0403:6010) in sysfs, and \
                 RETICLE_ICE40_TTY is not set"
            );
            return None;
        };
        let cable = match usb::Cable::open(None) {
            Ok(cable) => cable,
            Err(ProgramError::NoDevice { .. }) => {
                eprintln!("skipping: no FT2232H is attached");
                return None;
            }
            Err(err) => panic!("opening the FT2232H: {err}"),
        };
        let mut serial = Serial::open(&tty)?;
        let stale = serial.drain();
        if !stale.bytes.is_empty() || stale.breaks != 0 {
            eprintln!("before the load: {stale:?}");
        }
        let chip = cable.chip();
        let divisor = ftdi::divisor_for_chip(chip, ice40::DEFAULT_SPI_HZ);
        let mut p = ice40::Programmer::new(cable, chip, divisor, ice40::ICEPROG_WIRING)
            .expect("configuring the MPSSE");
        let done = p
            .load_sram(bin, ice40::SramPath::FlashOutput)
            .expect("loading the configuration memory");
        assert!(done, "CDONE did not rise after the load");
        Some(serial)
    }

    /// `uart_echo.v`: every byte value sent comes back plus one, one byte at
    /// a time, and nothing else comes back.
    ///
    /// What it catches: an IO block's input or output configured wrong
    /// (nothing comes back), a carry chain, enable or reset that is not the
    /// silicon's (wrong or no bytes: this is the design that found the reset
    /// that waited for an enable), and a baud counter off by more than a
    /// UART tolerates. And it holds the line to saying nothing, not even a
    /// break, from before the reset until the first byte is sent: the stray
    /// `0x00` an earlier board showed (`docs/fpga-icestorm.md`) would fail
    /// it. What it does not: anything the design does not use — RAMs, the
    /// RGB driver.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn the_echo_answers_every_byte_on_the_board() {
        let Some(bin) = build("uart_echo") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        // The line was open through the reset and the configuration, so a
        // transmit pin that floated low then would show as a break here;
        // the earlier scripts' termios read a break as one `0x00`.
        let after = serial.drain();
        eprintln!("after the load: {after:?}");
        assert!(
            after.bytes.is_empty() && after.breaks == 0 && after.bad_frames.is_empty(),
            "the line said something before anything was sent: {after:?}"
        );
        let mut wrong = Vec::new();
        for b in 0..=255u8 {
            serial.send(&[b]);
            let mut got = Received::default();
            serial.read(
                &mut got,
                Duration::from_millis(300),
                Duration::from_secs(1),
                |r| !r.bytes.is_empty(),
            );
            if got.bytes != [b.wrapping_add(1)] || got.breaks != 0 || !got.bad_frames.is_empty() {
                wrong.push(format!("sent {b:02x}, got {got:?}"));
            }
        }
        serial.send(b"HAL hello");
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(300),
            Duration::from_secs(2),
            |r| r.bytes.len() >= 9,
        );
        assert!(
            wrong.is_empty(),
            "{} of 256 byte(s) wrong: {wrong:?}",
            wrong.len()
        );
        assert_eq!(got.bytes, b"IBM!ifmmp", "{got:?}");
        eprintln!("all 256 byte values came back plus one, and `HAL hello` as `IBM!ifmmp`");
    }

    /// The four dumps `ram_selftest.v` prints, as its formulas say.
    fn expected_dumps() -> [(&'static str, String); 4] {
        let r16: String = (0..256u32)
            .map(|i| format!("{:02x}{:02x}", (i ^ 0x5a) & 0xff, !i & 0xff))
            .collect();
        let r8: String = (0..512u32)
            .map(|i| {
                format!(
                    "{:02x}",
                    ((i & 0xff) ^ (((i >> 8) & 1) << 7 | ((i >> 2) & 0x7f)) ^ 0xc3) & 0xff
                )
            })
            .collect();
        let r4: String = (0..1024u32)
            .map(|i| {
                format!(
                    "{:x}",
                    ((i & 15) ^ ((i >> 4) & 15) ^ (((i >> 8) & 3) << 2 | ((i >> 8) & 3))) & 15
                )
            })
            .collect();
        let r2: String = (0..2048u32)
            .map(|i| format!("{:x}", ((i & 3) ^ ((i >> 2) & 3) ^ ((i >> 9) & 3)) & 3))
            .collect();
        [("R16", r16), ("R08", r8), ("R04", r4), ("R02", r2)]
    }

    /// Checks one report of `ram_selftest.v`, returning what was wrong.
    fn check_report(text: &str) -> Vec<String> {
        let mut problems = Vec::new();
        let want = expected_dumps();
        let mut seen = Vec::new();
        for line in text.split(['\r', '\n']).filter(|l| !l.is_empty()) {
            if let Some(rest) = line.strip_prefix("SP:") {
                seen.push("SP");
                if line != "SP:PPPP BR:PPPP" {
                    problems.push(format!("the verdicts read `SP:{rest}`"));
                }
            } else if line == "END" {
                seen.push("END");
            } else if let Some((key, expect)) = want.iter().find(|(k, _)| line.starts_with(k)) {
                seen.push(key);
                let got = line.get(4..).unwrap_or("");
                if got != expect {
                    let differ = got
                        .chars()
                        .zip(expect.chars())
                        .filter(|(a, b)| a != b)
                        .count();
                    problems.push(format!(
                        "{key}: {} digit(s) where {} were expected, {differ} differ",
                        got.len(),
                        expect.len()
                    ));
                }
            } else {
                problems.push(format!(
                    "an unexpected line: {:?}",
                    line.chars().take(60).collect::<String>()
                ));
            }
        }
        for key in ["SP", "R16", "R08", "R04", "R02", "END"] {
            if !seen.contains(&key) {
                problems.push(format!("no {key} line"));
            }
        }
        problems
    }

    /// `ram_selftest.v`: all four single-port RAMs and a block RAM of each
    /// width pass, and every digit of the four read-only block RAMs'
    /// dumps — contents from `initial` — equals the formula in the RTL,
    /// on the report printed after power-up and on one asked for by a
    /// byte.
    ///
    /// What it catches: a RAM pin on the wrong wire, a mode in the wrong
    /// bits, a mask bit swapped, contents in the wrong order or the wrong
    /// layout for a narrow mode — the two faults `docs/fpga-icestorm.md`
    /// planted and the board saw (`SP:FFFF`, and 2 560 digits wrong) were
    /// both invisible to `implement`'s checks. What it does not: the 2-bit
    /// dump under a reversal of the contents' lines, which its formula
    /// happens to be symmetric under; the other three dumps are not.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn both_kinds_of_ram_pass_and_dump_their_contents_on_the_board() {
        let Some(bin) = build("ram_selftest") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let ended = |r: &Received| r.bytes.windows(3).any(|w| w == b"END");
        // The report it prints on its own, about 87 ms after configuring.
        let mut first = Received::default();
        serial.read(
            &mut first,
            Duration::from_millis(1500),
            Duration::from_secs(5),
            ended,
        );
        // A byte asks for another run.
        serial.send(b"?");
        let mut second = Received::default();
        serial.read(
            &mut second,
            Duration::from_millis(1500),
            Duration::from_secs(5),
            ended,
        );
        eprintln!(
            "after the load: {} byte(s), {} break(s), {} bad frame(s), starting {:02x?}",
            first.bytes.len(),
            first.breaks,
            first.bad_frames.len(),
            &first.bytes[..first.bytes.len().min(4)]
        );
        let mut problems: Vec<String> = check_report(&String::from_utf8_lossy(&first.bytes))
            .into_iter()
            .map(|p| format!("after the load: {p}"))
            .collect();
        problems.extend(
            check_report(&String::from_utf8_lossy(&second.bytes))
                .into_iter()
                .map(|p| format!("when asked: {p}")),
        );
        for (when, got) in [("after the load", &first), ("when asked", &second)] {
            if got.breaks != 0 || !got.bad_frames.is_empty() {
                problems.push(format!(
                    "{when}: {} break(s) and bad frames {:02x?}",
                    got.breaks, got.bad_frames
                ));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
        eprintln!("SP:PPPP BR:PPPP and all 4 608 dumped words as the RTL's formulas say, twice");
    }

    /// `dsp_selftest.v`: all eight DSP blocks, each configured another way,
    /// fed 64 vectors, and every block's 32-bit output and carry out on
    /// every step equal to what the model of the cell
    /// (`tests/ice40_mac16`) gives for that block's parameters and wiring —
    /// 16 x 16 unsigned and signed multiplies, a fully pipelined one, the
    /// 8 x 8 split, multiply-add, two accumulators and a 32-bit adder —
    /// after configuration and again when asked.
    ///
    /// What it catches: a DSP pin on the wrong wire, a parameter in the
    /// wrong bit (every bit of every parameter changes the report, which
    /// `every_dsp_parameter_bit_shows_in_the_report` checks), an unrouted
    /// input that does not read zero, a register that is not there. What
    /// it does not: the cascade between blocks, which nothing here uses.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn every_dsp_block_computes_what_the_model_says_on_the_board() {
        let Some(bin) = build("dsp_selftest") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let ended = |r: &Received| r.bytes.windows(3).any(|w| w == b"END");
        let mut first = Received::default();
        serial.read(
            &mut first,
            Duration::from_millis(1500),
            Duration::from_secs(5),
            ended,
        );
        serial.send(b"?");
        let mut second = Received::default();
        serial.read(
            &mut second,
            Duration::from_millis(1500),
            Duration::from_secs(5),
            ended,
        );
        let mut problems = Vec::new();
        for (when, got) in [("after the load", &first), ("when asked", &second)] {
            if got.breaks != 0 || !got.bad_frames.is_empty() {
                problems.push(format!("{when}: line errors, {got:?}"));
            }
            problems.extend(
                super::ice40_mac16::check_report(&String::from_utf8_lossy(&got.bytes))
                    .into_iter()
                    .map(|p| format!("{when}: {p}")),
            );
        }
        assert!(
            problems.is_empty(),
            "{} problem(s): {:#?}",
            problems.len(),
            &problems[..problems.len().min(40)]
        );
        eprintln!(
            "all 8 DSP blocks, 64 steps each, every output and carry out as the model says, twice"
        );
    }

    /// Checks the lines a PLL design prints, `L<lock> N<count>` and, for
    /// the two-output one, ` H<count>`: after the first two (the first
    /// window starts at configuration, and B's count is a window late), at
    /// least five, every one locked, `N` within two cycles of 0x400000 and
    /// `H` of 0x200000.
    fn check_pll_lines(text: &str, second: bool) -> Vec<String> {
        let lines: Vec<&str> = text.split(['\r', '\n']).filter(|l| !l.is_empty()).collect();
        let mut problems = Vec::new();
        if lines.len() < 7 {
            problems.push(format!("{} line(s), not seven or more", lines.len()));
        }
        let near = |field: Option<&str>, prefix: char, want: i64| {
            field
                .and_then(|f| f.strip_prefix(prefix))
                .and_then(|h| i64::from_str_radix(h, 16).ok())
                .is_some_and(|v| (v - want).abs() <= 2)
        };
        for line in lines.iter().skip(2) {
            let fields: Vec<&str> = line.split(' ').collect();
            let ok = fields.first() == Some(&"L1")
                && near(fields.get(1).copied(), 'N', 0x40_0000)
                && (!second || near(fields.get(2).copied(), 'H', 0x20_0000))
                && fields.len() == if second { 3 } else { 2 };
            if !ok {
                problems.push(format!("the line {line:?}"));
            }
        }
        problems
    }

    /// `pll_uart.v`: the PLL multiplies the board's 12 MHz by four. Its
    /// transmitter runs on the PLL's global output at 417 clocks a bit,
    /// which is 115200 baud at 48 MHz and nothing readable at any other
    /// frequency, and each line counts the PLL's cycles in 2^20 of the
    /// reference's: 0x400000, to within resynchronisation, says 48.000 MHz
    /// against the board's own oscillator. And LOCK, through the cell the
    /// backend confines to the one tile its wire reaches, reads one.
    ///
    /// What it catches: a divider, the filter range or the feedback path in
    /// the wrong bits (another frequency, or no lock), the global output
    /// not driving its network (no clock at all), the output's IO block or
    /// pad-input bit not set, `LOCK` not reaching the fabric, and `BYPASS`
    /// reading one undriven (the reference itself, 12 MHz). What it does
    /// not: the other output modes, the phase shifts and delays.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn the_pll_runs_at_48_mhz_and_locks_on_the_board() {
        let Some(bin) = build("pll_uart") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(500),
            Duration::from_secs(2),
            |r| r.bytes.iter().filter(|b| **b == b'\n').count() >= 12,
        );
        let text = String::from_utf8_lossy(&got.bytes).into_owned();
        let problems = check_pll_lines(&text, false);
        assert!(problems.is_empty(), "{problems:?} in {text:?}");
        eprintln!(
            "{} lines, every one L1 and 0x400000 +- 2 cycles",
            text.lines().count()
        );
    }

    /// `pll2f_uart.v`: the two-output PLL, port A at 48 MHz clocking the
    /// transmitter and port B at half that, `GENCLK_HALF`, counted in a
    /// domain of its own: 0x400000 and 0x200000 cycles per window.
    ///
    /// What it catches, beyond `the_pll_runs_at_48_mhz_and_locks_on_the_board`:
    /// port B's output mode, its IO block and its global network's
    /// pad-input bit, `PLLTYPE` for the dual form.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn both_pll_outputs_run_at_their_frequencies_on_the_board() {
        let Some(bin) = build("pll2f_uart") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(500),
            Duration::from_secs(2),
            |r| r.bytes.iter().filter(|b| **b == b'\n').count() >= 12,
        );
        let text = String::from_utf8_lossy(&got.bytes).into_owned();
        let problems = check_pll_lines(&text, true);
        assert!(problems.is_empty(), "{problems:?} in {text:?}");
        eprintln!(
            "{} lines, every one L1, A 0x400000 and B 0x200000 +- 2 cycles",
            text.lines().count()
        );
    }

    /// `osc_count.v`: both internal oscillators run, each counted in 2^20
    /// cycles of the board's 12 MHz crystal oscillator: the high-frequency
    /// one, divided to a nominal 12 MHz, within the data sheet's 10 % of
    /// 0x100000, and the low-frequency one within 25 % of its nominal 10
    /// kHz, 874 cycles. It prints what it measured.
    ///
    /// What it catches: an oscillator's clock not reaching its network (a
    /// count of zero, which is what the first build gave: the pad-input
    /// bit was missing), its enable or power-up not reaching it, the
    /// divider in the wrong bits (48, 24 or 6 MHz instead of 12). What it
    /// does not: the trim inputs, which nothing here drives.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn both_oscillators_run_on_the_board() {
        let Some(bin) = build("osc_count") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(500),
            Duration::from_secs(2),
            |r| r.bytes.iter().filter(|b| **b == b'\n').count() >= 10,
        );
        let text = String::from_utf8_lossy(&got.bytes).into_owned();
        let lines: Vec<&str> = text.split(['\r', '\n']).filter(|l| !l.is_empty()).collect();
        assert!(lines.len() >= 7, "{text:?}");
        let mut hf = Vec::new();
        let mut lf = Vec::new();
        for line in lines.iter().skip(2) {
            let fields: Vec<&str> = line.split(' ').collect();
            let value = |i: usize, p: char| {
                fields
                    .get(i)
                    .and_then(|f| f.strip_prefix(p))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
            };
            let (Some(h), Some(l)) = (value(0, 'H'), value(1, 'L')) else {
                panic!("the line {line:?}");
            };
            hf.push(f64::from(h));
            lf.push(f64::from(l));
        }
        let window = f64::from(1u32 << 20) / 12e6;
        for (what, counts, nominal, tolerance) in
            [("SB_HFOSC", &hf, 12e6, 0.10), ("SB_LFOSC", &lf, 10e3, 0.25)]
        {
            for c in counts {
                let hz = c / window;
                assert!(
                    (hz / nominal - 1.0).abs() <= tolerance,
                    "{what} at {hz:.0} Hz, not within {tolerance} of {nominal}: {text:?}"
                );
            }
            let mean = counts.iter().sum::<f64>() / counts.len() as f64 / window;
            eprintln!("{what}: {mean:.1} Hz over {} windows", counts.len());
        }
    }

    /// The report check itself, with no board: the report the formulas
    /// predict passes, and one wrong digit, one failed verdict, a missing
    /// line or a stray byte before the first line each fail it. Without
    /// this, a check that accepted anything would pass on the board too.
    /// The lines `sbus_probe.v` prints, after the first, as `(registers, acks)`:
    /// thirty-two register values (SPI 0..15, then I2C 0..15) and their
    /// acknowledge bits, index `i` at bit `i`.
    fn sbus_lines(text: &str) -> Vec<(Vec<u8>, u32)> {
        let mut out = Vec::new();
        for line in text.lines().skip(1) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let (Some(s), Some(i), Some(a)) = (
                w.first().and_then(|x| x.strip_prefix('S')),
                w.get(1).and_then(|x| x.strip_prefix('I')),
                w.get(2).and_then(|x| x.strip_prefix('A')),
            ) else {
                continue;
            };
            let hex = |t: &str| -> Option<Vec<u8>> {
                (0..t.len() / 2)
                    .map(|k| u8::from_str_radix(&t[2 * k..2 * k + 2], 16).ok())
                    .collect()
            };
            let (Some(mut regs), Some(more), Ok(acks)) =
                (hex(s), hex(i), u32::from_str_radix(a, 16))
            else {
                continue;
            };
            regs.extend(more);
            if regs.len() == 32 {
                out.push((regs, acks));
            }
        }
        out
    }

    /// `sbus_probe.v`: the hard SPI block on bus address `0b0000` and the
    /// hard I2C block on `0b0001` answer their system buses. Each of the
    /// registers that, on 2026-11-19, acknowledged and read back what was
    /// written — the SPI block's 7 to 11, 13 and 15, the I2C block's 3, 7
    /// to 11 and 13 — must again, every line, and no register of the
    /// SPI block's 0 to 5 or the I2C block's 0 to 2 and 4 and 5 may
    /// acknowledge.
    ///
    /// What it catches: a block on the wrong site for its bus address (the
    /// other block's acknowledges, or none), a bus port on the wrong wire
    /// (a register reads back another value), an enable bit missing (the
    /// block silent). What it does not: the blocks' SPI and I2C sides,
    /// which reach no pin here, nor what each register means.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn the_hard_spi_and_i2c_blocks_answer_their_buses_on_the_board() {
        let Some(bin) = build("sbus_probe") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(800),
            Duration::from_secs(4),
            |r| r.bytes.iter().filter(|b| **b == b'\n').count() >= 6,
        );
        let text = String::from_utf8_lossy(&got.bytes).into_owned();
        let lines = sbus_lines(&text);
        assert!(lines.len() >= 3, "{text:?}");
        let written = |i: usize| {
            let a = u8::try_from(i % 16).unwrap();
            (if i < 16 { 0x5A } else { 0xC3 }) ^ (a << 4 | a)
        };
        let back: Vec<usize> = [7, 8, 9, 10, 11, 13, 15, 19, 23, 24, 25, 26, 27, 29].to_vec();
        let silent: Vec<usize> = [0, 1, 2, 3, 4, 5, 16, 17, 18, 20, 21].to_vec();
        for (regs, acks) in &lines {
            for &i in &back {
                assert!(
                    acks >> i & 1 == 1,
                    "register {i} did not acknowledge: {text:?}"
                );
                assert_eq!(regs[i], written(i), "register {i}: {text:?}");
            }
            for &i in &silent {
                assert!(acks >> i & 1 == 0, "register {i} acknowledged: {text:?}");
            }
        }
        eprintln!(
            "{} lines, every one as written: {}",
            lines.len(),
            text.lines().nth(1).unwrap_or("")
        );
    }

    /// `ledda_breathe.v`: the LED dimmer breathes. Over five seconds the red
    /// output's duty, measured in the fabric each 22 ms, rises from below a
    /// tenth of its peak to above nine tenths and falls back, more than once,
    /// with `LEDDON` low only while the duty is zero.
    ///
    /// What it catches: the dimmer not running (a constant duty, or none), a
    /// control-bus port on the wrong wire (registers not written: no
    /// breathing), its outputs not reaching the RGB driver. What it does
    /// not: the light itself, which is the driver's and the eye's.
    #[test]
    #[ignore = "needs an iCE40UP5K-SG48 board with an FT2232H"]
    fn the_led_dimmer_breathes_on_the_board() {
        let Some(bin) = build("ledda_breathe") else {
            return;
        };
        let Some(mut serial) = load(&bin) else {
            return;
        };
        let mut got = Received::default();
        serial.read(
            &mut got,
            Duration::from_millis(800),
            Duration::from_secs(6),
            |r| r.bytes.iter().filter(|b| **b == b'\n').count() >= 230,
        );
        let text = String::from_utf8_lossy(&got.bytes).into_owned();
        let samples: Vec<(u32, bool)> = text
            .lines()
            .skip(1)
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                let d = u32::from_str_radix(w.first()?.strip_prefix('P')?, 16).ok()?;
                let on = w.get(1)?.strip_prefix('O')? == "1";
                Some((d, on))
            })
            .collect();
        assert!(samples.len() > 150, "{text:?}");
        let peak = samples.iter().map(|s| s.0).max().unwrap_or(0);
        assert!(peak > 0x1000, "no light: {text:?}");
        // Count the swings from below a tenth to above nine tenths.
        let (mut swings, mut low) = (0, false);
        for &(d, _) in &samples {
            if d * 10 < peak {
                low = true;
            } else if d * 10 > peak * 9 && low {
                swings += 1;
                low = false;
            }
        }
        assert!(swings >= 1, "no breath: {text:?}");
        for &(d, on) in &samples {
            assert!(on || d == 0, "LEDDON low with the LED lit: {text:?}");
        }
        eprintln!(
            "{} samples, peak {peak:#x}, {swings} breath(s)",
            samples.len()
        );
    }

    #[test]
    fn the_report_check_rejects_a_wrong_report() {
        let mut good = String::from("SP:PPPP BR:PPPP\r\n");
        for (key, digits) in expected_dumps() {
            good.push_str(&format!("{key}:{digits}\r\n"));
        }
        good.push_str("END\r\n");
        assert_eq!(check_report(&good), Vec::<String>::new());
        let digit = good.find("R04:").unwrap() + 4 + 100;
        let mut wrong = good.clone().into_bytes();
        wrong[digit] = if wrong[digit] == b'0' { b'1' } else { b'0' };
        let pll = "L1 N3fffff\r\nL1 N400001\r\n".to_owned() + &"L1 N400000\r\n".repeat(6);
        assert!(check_pll_lines(&pll, false).is_empty());
        for bad in ["L0 N400000", "L1 N400003", "L1 N3ffffd", "L1 N200000"] {
            let text = pll.clone() + bad + "\r\n";
            assert!(!check_pll_lines(&text, false).is_empty(), "accepted {bad}");
        }
        assert!(!check_pll_lines("L1 N400000\r\n", false).is_empty());
        let two = "L1 N400000 H000000\r\n".repeat(2) + &"L1 N400000 H200001\r\n".repeat(6);
        assert!(check_pll_lines(&two, true).is_empty());
        assert!(!check_pll_lines(&(two.clone() + "L1 N400000 H400000\r\n"), true).is_empty());
        for bad in [
            String::from_utf8(wrong).unwrap(),
            good.replacen("BR:PPPP", "BR:PPFP", 1),
            good.replacen("END\r\n", "", 1),
            format!("\0{good}"),
        ] {
            assert!(!check_report(&bad).is_empty(), "accepted: {:?}", &bad[..20]);
        }
    }
}
