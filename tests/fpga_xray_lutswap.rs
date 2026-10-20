//! A lookup table's inputs as the router chose them, on the real fabric.
//!
//! `xray::lutswap` lets the router end a lookup table's input on any of
//! the table's free pins and rewrites the truth table to follow. Its own
//! test checks the permutation on one function; this one routes a real
//! design on Project X-Ray's database with the classes it builds, applies
//! the moves, and then re-derives, from nothing but the netlist's pins
//! before and after, what each moved table must compute — and compares it
//! with the truth table the rewrite wrote, at every one of the 64 indices.
//! It also asks the route's own check, which walks every sink back to its
//! driver through the pins the rewritten netlist names.

#![cfg(all(feature = "fpga", feature = "verilog", feature = "synth"))]

use std::path::Path;

use reticle::fpga::place::{PlaceOptions, place};
use reticle::fpga::xray::{XrayDatabase, XrayOptions, apply_lut_moves, lut_input_classes};
use reticle::fpga::{
    Constraints, FpgaOptions, Netlist, RouteOptions, route, synthesize_for, target,
};
use reticle::ir::memfile::FileProvider;
use reticle::ir::{AttrValue, Const};

const DEVICE: &str = "xc7a35t-cpg236";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing; as
/// `tests/fpga_xray.rs` finds it.
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
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")))?;
    let dir = format!("{cache}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3");
    if Path::new(&format!("{dir}/{probe}")).is_file() {
        Some(dir)
    } else {
        eprintln!("skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db`");
        None
    }
}

/// **Every table the router moved computes what it was mapped to.** The
/// Basys 3 `blink` design, placed and routed with each lookup table's
/// free inputs one class, then rewritten. For every table with a moved
/// input: wherever a pin's signal now arrives, the new truth table read
/// with that input set must equal the old one read with the old input
/// set, for all 64 combinations — the inputs that carry no signal held
/// where the rewrite left them, which a table either does not read or, if
/// it does, never had moved. Would catch the truth table permuted the wrong
/// way round, a pin given a role it was not routed to (the route check
/// fails), and a constant input the table reads moved off its pin; would
/// not catch a class that lets two tables trade pins, which the router's
/// classes are per site to prevent and which the route check would also
/// refuse.
#[test]
fn every_table_the_router_moved_computes_what_it_was_mapped_to() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/basys3");
    let (Ok(verilog), Ok(rcf)) = (
        std::fs::read_to_string(dir.join("blink.v")),
        std::fs::read_to_string(dir.join("blink.rcf")),
    ) else {
        eprintln!("skipped: `examples/` is not in the published crate");
        return;
    };
    use reticle::diag::Diagnostics;
    use reticle::source::SourceMap;
    let mut map = SourceMap::new();
    let source = map.add("blink.v", &verilog).unwrap();
    let rcf_file = map.add("blink.rcf", &rcf).unwrap();
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
    let top = design.top.unwrap();
    let device = target(DEVICE).unwrap();
    let mut constraints = Constraints::parse(&rcf, rcf_file, &mut diags);
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

    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
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
    let before = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, _) = place(
        &before,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let options = RouteOptions {
        equivalent: lut_input_classes(&design, top, &before, &graph, &placement),
        ..RouteOptions::default()
    };
    let (routing, _) = route(&before, &graph, &placement, &options).expect("blink routes");

    let mut after_design = design.clone();
    let mut after = before.clone();
    let tables = apply_lut_moves(
        &mut after_design,
        top,
        &mut after,
        &graph,
        &placement,
        &routing,
    )
    .unwrap();
    assert!(tables > 0, "no table was moved, so nothing here was tested");
    let problems = routing.verify(&after, &graph, &placement);
    assert!(problems.is_empty(), "{problems:?}");

    let init = |design: &reticle::ir::Design, cell| -> Const {
        match design.modules[top].cells[cell].params.get("INIT") {
            Some(AttrValue::Const(c)) => c.clone(),
            other => panic!("no truth table: {other:?}"),
        }
    };
    let index = |role: &str| -> Option<u32> { role.strip_prefix('i')?.parse().ok() };
    let mut checked = 0;
    for (instance, inst) in before.instances.iter().enumerate() {
        if inst.kind != "lut" {
            continue;
        }
        // Each input pin's position, as mapped and as routed.
        let moves: Vec<(u32, u32)> = inst
            .pins
            .iter()
            .filter(|p| !before.pins[**p].output)
            .filter_map(|p| Some((index(&before.pins[*p].role)?, index(&after.pins[*p].role)?)))
            .collect();
        if moves.iter().all(|(from, to)| from == to) {
            continue;
        }
        let (old, new) = (init(&design, inst.cell), init(&after_design, inst.cell));
        for v in 0..64u32 {
            // The old table's index: each mapped input reads what its pin
            // now reads; an input with no pin reads nothing it cares about.
            let mut u = 0u32;
            for (from, to) in &moves {
                if v & (1 << to) != 0 {
                    u |= 1 << from;
                }
            }
            assert_eq!(
                new.bit(v),
                old.bit(u),
                "`{}` at index {v}: moves {moves:?}",
                after.instances[instance].name
            );
        }
        checked += 1;
    }
    assert_eq!(checked, tables);
}
