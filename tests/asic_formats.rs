//! Golden tests for the ASIC file formats (Liberty, LEF, DEF).
//!
//! Each sample under `testdata/asic/` must parse without diagnostics,
//! summarise to its checked-in `.dump` file, and — for LEF and DEF —
//! survive a write/parse round-trip as a fixed point. Run with
//! `UPDATE_EXPECT=1` to rewrite the `.dump` files after a deliberate
//! change to the summaries.

#![cfg(feature = "asic")]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::asic::def::{self, DefOptions};
use reticle::asic::lef;
use reticle::asic::liberty::Library;
use reticle::diag::Diagnostics;
use reticle::ir::Design;
use reticle::source::SourceMap;

fn testdata(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/asic")
        .join(name)
}

fn read(name: &str) -> String {
    let path = testdata(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Compares `actual` with the contents of `name`, rewriting the file when
/// `UPDATE_EXPECT` is set.
fn expect(name: &str, actual: &str) {
    let path = testdata(name);
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        fs::write(&path, actual).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        return;
    }
    let diff = expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .find(|(_, (e, a))| e != a)
        .map_or_else(
            || {
                format!(
                    "expected {} lines, got {}",
                    expected.lines().count(),
                    actual.lines().count()
                )
            },
            |(i, (e, a))| format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1),
        );
    panic!(
        "{} is out of date ({diff})\nrerun with UPDATE_EXPECT=1 to update",
        path.display()
    );
}

#[test]
fn liberty_parses_to_a_gate_summary() {
    let text = read("cells.lib");
    let mut map = SourceMap::new();
    let file = map.add("cells.lib", text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let lib = Library::parse(&text, file, &mut diags).expect("library");
    assert!(diags.is_empty(), "{}", diags.render(&map));

    // The generic tree keeps everything, including groups and attributes
    // the typed view does not model.
    assert_eq!(
        lib.group.attr("technology").map(ToString::to_string),
        Some("(cmos)".to_string())
    );
    assert_eq!(lib.group.attr_str("bus_naming_style"), Some("%s[%d]"));
    assert!(
        lib.cell("demo_sc_hd__inv_1")
            .is_some_and(|c| c.is_combinational())
    );

    let mut out = lib.to_gate_summary();
    out.push('\n');
    // Timing detail for the cells the mapper and the timing analyser care
    // about, exercised through the table interpolation.
    let inv_y = lib.pin("demo_sc_hd__inv_1", "Y").expect("inv Y");
    let arc = &inv_y.timing[0];
    writeln!(
        out,
        "inv A->Y {} {}: rise(0.04,0.002)={:.4} fall(0.04,0.002)={:.4} slew(0.04,0.002)={:.4}",
        arc.timing_type,
        arc.timing_sense.expect("sense").as_str(),
        arc.cell_rise.as_ref().unwrap().lookup(0.04, 0.002).unwrap(),
        arc.cell_fall.as_ref().unwrap().lookup(0.04, 0.002).unwrap(),
        arc.rise_transition
            .as_ref()
            .unwrap()
            .lookup(0.04, 0.002)
            .unwrap(),
    )
    .unwrap();
    // A point between grid lines, to pin the bilinear interpolation down.
    writeln!(
        out,
        "inv A->Y interpolated rise(0.10,0.005)={:.6}",
        arc.cell_rise.as_ref().unwrap().lookup(0.10, 0.005).unwrap()
    )
    .unwrap();

    let d = lib.pin("demo_sc_hd__dfrtp_1", "D").expect("ff D");
    for t in &d.timing {
        writeln!(
            out,
            "dfrtp D {} from {}: rise={:.4} fall={}",
            t.timing_type,
            t.related_pin,
            t.rise_constraint
                .as_ref()
                .and_then(|t| t.lookup(0.04, 0.04))
                .unwrap_or(f64::NAN),
            t.fall_constraint
                .as_ref()
                .and_then(|t| t.lookup(0.04, 0.04))
                .map_or("-".to_string(), |v| format!("{v:.4}"))
        )
        .unwrap();
    }

    // Truth tables, the form the standard-cell mapper matches against.
    for (cell, pin, inputs) in [
        ("demo_sc_hd__inv_1", "Y", &["A"][..]),
        ("demo_sc_hd__nand2_1", "Y", &["A", "B"][..]),
    ] {
        let f = lib.pin(cell, pin).unwrap().function.as_ref().unwrap();
        writeln!(
            out,
            "{cell}.{pin} = {f} truth_table({}) = 0x{:x}",
            inputs.join(","),
            f.truth_table(inputs).unwrap()
        )
        .unwrap();
    }

    expect("cells.lib.dump", &out);
}

#[test]
fn lef_round_trips_and_summarises() {
    let text = read("cells.lef");
    let mut map = SourceMap::new();
    let file = map.add("cells.lef", text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let parsed = lef::parse_lef(&text, file, &mut diags);
    assert!(diags.is_empty(), "{}", diags.render(&map));

    // write -> parse -> write is a fixed point, and the second parse
    // yields an equal value.
    let written = lef::write_lef(&parsed);
    let mut map2 = SourceMap::new();
    let file2 = map2.add("cells.lef", written.clone()).unwrap();
    let mut diags2 = Diagnostics::new();
    let again = lef::parse_lef(&written, file2, &mut diags2);
    assert!(diags2.is_empty(), "{}\n{written}", diags2.render(&map2));
    assert_eq!(again, parsed, "LEF round-trip changed the typed view");
    assert_eq!(lef::write_lef(&again), written, "LEF writer is not stable");

    expect("cells.lef.dump", &written);
}

#[test]
fn def_round_trips_and_summarises() {
    let text = read("counter.def");
    let mut map = SourceMap::new();
    let file = map.add("counter.def", text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let parsed = def::parse_def(&text, file, &mut diags);
    assert!(diags.is_empty(), "{}", diags.render(&map));

    let written = def::write_def(&parsed);
    let mut map2 = SourceMap::new();
    let file2 = map2.add("counter.def", written.clone()).unwrap();
    let mut diags2 = Diagnostics::new();
    let again = def::parse_def(&written, file2, &mut diags2);
    assert!(diags2.is_empty(), "{}\n{written}", diags2.render(&map2));
    assert_eq!(again, parsed, "DEF round-trip changed the typed view");
    assert_eq!(def::write_def(&again), written, "DEF writer is not stable");

    expect("counter.def.dump", &written);
}

#[test]
fn netlist_becomes_an_unplaced_def() {
    let text = read("mapped.rtl");
    let mut map = SourceMap::new();
    let file = map.add("mapped.rtl", text.clone()).unwrap();
    let design =
        Design::parse_text(&text, file).unwrap_or_else(|diags| panic!("{}", diags.render(&map)));
    let top = design.top.expect("top module");

    let mut diags = Diagnostics::new();
    let opts = DefOptions {
        die_area: Some(def::Rect {
            x1: 0,
            y1: 0,
            x2: 9200,
            y2: 10880,
        }),
        ..DefOptions::default()
    };
    let out = def::from_netlist(&design, top, &opts, &mut diags);
    assert!(diags.is_empty(), "{}", diags.render(&map));

    // Every component is unplaced and every net carries its drivers and
    // loads, which is what OpenROAD needs to start placement.
    assert_eq!(out.components.len(), 3);
    assert!(
        out.components
            .iter()
            .all(|c| c.placement.status == def::PlacementStatus::Unplaced)
    );
    let written = def::write_def(&out);
    let mut map2 = SourceMap::new();
    let file2 = map2.add("out.def", written.clone()).unwrap();
    let mut diags2 = Diagnostics::new();
    let again = def::parse_def(&written, file2, &mut diags2);
    assert!(diags2.is_empty(), "{}\n{written}", diags2.render(&map2));
    assert_eq!(again, out);

    expect("mapped.def.dump", &written);
}

/// Parses a real PDK Liberty file when one is available, for a smoke test
/// against something larger than the hand-written corpus. Ignored by
/// default: it needs a file this repository deliberately does not vendor.
/// Run with `RETICLE_LIBERTY=/path/to/lib cargo test --all-features
/// --test asic_formats -- --ignored --nocapture`.
#[test]
#[ignore = "needs a PDK Liberty file; set RETICLE_LIBERTY"]
fn parses_a_real_pdk_liberty_file() {
    let Some(path) = std::env::var_os("RETICLE_LIBERTY") else {
        panic!("set RETICLE_LIBERTY to a .lib file");
    };
    let path = PathBuf::from(path);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut map = SourceMap::new();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let file = map.add(name.into_owned(), text.clone()).unwrap();
    let mut diags = Diagnostics::new();
    let lib = Library::parse(&text, file, &mut diags).expect("library");
    println!(
        "{}: {} cells, {} templates, {} bytes",
        lib.name,
        lib.cells.len(),
        lib.templates.len(),
        text.len()
    );
    let combinational = lib.cells.iter().filter(|c| c.is_combinational()).count();
    let sequential = lib.cells.iter().filter(|c| c.is_sequential()).count();
    println!("  {combinational} combinational, {sequential} sequential");
    println!(
        "  {} errors, {} warnings",
        diags.error_count(),
        diags.warning_count()
    );
    for d in diags.iter().take(20) {
        println!("  {}", d.message);
    }
    assert_eq!(diags.error_count(), 0, "{}", diags.render(&map));
    assert!(!lib.cells.is_empty());
}
