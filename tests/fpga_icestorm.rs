//! The iCE40 backend against Project IceStorm's database and against
//! bitstreams other tools wrote.
//!
//! Two inputs are optional and every test that needs one skips with a
//! message when it is absent:
//!
//! - **the chip database**: `RETICLE_ICESTORMDB` names the directory
//!   holding `chipdb-5k.txt`; unset, the copy `reticle fetch icestorm`
//!   cached is used;
//! - **reference bitstreams**: `RETICLE_ICE40_REF` names a directory of
//!   `<name>.asc` / `<name>.bin` pairs that nextpnr and icepack wrote for an
//!   iCE40UP5K. None ship with Reticle.

#![cfg(feature = "fpga")]

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
    let dir = database_dir()?;
    let text = match std::fs::read_to_string(dir.join(PROBE)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("skipping: {}: {e}", dir.display());
            return None;
        }
    };
    Some(ChipDb::parse(&text).expect("the database parses"))
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
    let Some(db) = chipdb_5k() else { return };
    let fabric = IceFabric::build(db, "sg48").expect("the fabric builds");
    for (name, asc, _) in pairs {
        let image = Image::from_asc(&asc).unwrap();
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
            "{name}: {} set bits, {} arcs, {} function fields, 0 unexplained",
            decoded.bits,
            decoded.arcs.len(),
            decoded.functions.len()
        );
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
