//! Synthesis against Vivado's, on the timing corpus: what the 7 series' default
//! synthesis (`FpgaOptions::vivado_synthesis`, `--synthesis vivado`) makes of
//! each design, proved, and what `--synthesis legacy` makes of it.
//!
//! `docs/fpga-vivado-parity.md`, "Synthesis against Vivado's", has the account;
//! `testdata/fpga/vivado/qor/netlist.txt` holds what Vivado's own synthesis
//! made of each design and what it counted in Reticle's netlists, both by
//! Vivado's counting (`tools/vivado-parity/netlist_qor.sh`).
//!
//! Every figure pinned here is Reticle's own report of its own mapping, so
//! the tests need no fabric database and no Vivado, and none skips.

#![cfg(all(feature = "fpga", feature = "synth", feature = "verilog"))]

use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::fpga::{Constraints, FlowReport, FpgaOptions, synthesize_for, target};
use reticle::source::SourceMap;
use reticle::synth::techmap::{MapEquivalence, MapVerifyOptions};

const DEVICE: &str = "xc7a100t-ftg256";

/// The corpus's designs in this repository: the thirteen of `qor/`, and two
/// beside it. (`esad` and `sad`, the Husky Plus's pieces, are NewAE's sources
/// and not here; only Vivado's figures about them are, in the fixture.)
const DESIGNS: [&str; 15] = [
    "adders",
    "analyser_fifo",
    "barrel",
    "bram_fifo",
    "carry_counter",
    "counter64",
    "crc32",
    "fir8",
    "fsm",
    "gray_cdc",
    "lutram32",
    "mul18",
    "regfile_alu",
    "srl_delay",
    "widemux",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The sources and the constraints of one design.
fn sources(name: &str) -> (Vec<PathBuf>, PathBuf) {
    let vivado = root().join("testdata/fpga/vivado");
    let qor = vivado.join("qor");
    if qor.join(format!("{name}.v")).exists() {
        return (
            vec![qor.join(format!("{name}.v"))],
            qor.join(format!("{name}.rcf")),
        );
    }
    let mut files = vec![vivado.join(format!("{name}.v"))];
    if name == "analyser_fifo" {
        files.push(root().join("examples/husky/fifo_generator.v"));
    }
    (files, vivado.join(format!("{name}.rcf")))
}

/// One design through the flow, the mapping proved.
fn synthesise(name: &str, vivado: bool) -> FlowReport {
    let (files, rcf) = sources(name);
    let text: String = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{}: {e}", f.display())))
        .collect::<Vec<_>>()
        .join("\n");
    let constraints_text = std::fs::read_to_string(&rcf).unwrap();
    let mut map = SourceMap::new();
    let source = map.add(format!("{name}.v"), text).unwrap();
    let rcf = map
        .add(format!("{name}.rcf"), constraints_text.clone())
        .unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::SystemVerilog,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let options = reticle::verilog::ElabOptions {
        top: Some(name.to_owned()),
        ..reticle::verilog::ElabOptions::default()
    };
    let mut design = reticle::verilog::elaborate_file(&ast, &options, &mut diags)
        .expect("the design elaborates");
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let top = design.top.expect("a top module");
    let device = target(DEVICE).expect("a built-in device");
    let mut constraints = Constraints::parse(&constraints_text, rcf, &mut diags);
    constraints.merge_attrs(&design, top, &mut diags);
    // As `reticle fpga` maps for the 7 series: memories as Vivado does too.
    let mut options = FpgaOptions {
        verify_mapping: Some(MapVerifyOptions::default()),
        vivado_synthesis: vivado,
        ..FpgaOptions::default()
    };
    options.map.vivado_memories = true;
    let report = synthesize_for(&mut design, top, device, &constraints, &options, &mut diags)
        .expect("the design maps");
    assert!(!diags.has_errors(), "{name}: {}", diags.render(&map));
    report
}

/// What a report says of a netlist, as pinned: lookup tables, their depth,
/// and `CARRY4`s.
type Figures = (usize, u32, usize);

/// The [`Figures`] of a report.
fn figures(report: &FlowReport) -> Figures {
    (report.luts, report.lut_depth, report.count("CARRY4"))
}

/// `(design, legacy, vivado)`: the lookup tables, their depth and the
/// `CARRY4`s of each design, under `--synthesis legacy` and under the default.
const PINNED: [(&str, Figures, Figures); 15] = [
    ("adders", (289, 15, 40), (231, 2, 56)),
    ("analyser_fifo", (194, 10, 12), (180, 3, 26)),
    ("barrel", (105, 3, 0), (87, 2, 0)),
    ("bram_fifo", (32, 4, 9), (27, 3, 12)),
    ("carry_counter", (18, 4, 8), (13, 1, 11)),
    ("counter64", (35, 5, 16), (24, 1, 22)),
    ("crc32", (204, 7, 0), (191, 2, 0)),
    ("fir8", (9, 3, 0), (7, 2, 0)),
    ("fsm", (72, 4, 0), (38, 3, 0)),
    ("gray_cdc", (15, 2, 3), (15, 2, 3)),
    ("lutram32", (9, 3, 0), (7, 2, 0)),
    ("mul18", (9, 3, 0), (7, 2, 0)),
    ("regfile_alu", (421, 18, 8), (469, 6, 20)),
    ("srl_delay", (9, 3, 0), (7, 2, 0)),
    ("widemux", (276, 3, 0), (276, 2, 0)),
];

/// Runs `f` over every design at once, one thread a design; the results in
/// [`DESIGNS`]' order.
fn every_design<R: Send>(f: impl Fn(&str) -> R + Sync) -> Vec<R> {
    std::thread::scope(|scope| {
        let f = &f;
        let runs: Vec<_> = DESIGNS
            .iter()
            .map(|name| scope.spawn(move || f(name)))
            .collect();
        runs.into_iter()
            .map(|r| r.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

/// **Every design's mapping under the default synthesis is proved, and is
/// the netlist pinned; `--synthesis legacy` is the netlist before.**
///
/// The proof is `techmap::verify` of the lookup tables against the graph
/// before the XOR rebuilding (`aig::xor`), so it covers that pass and the
/// mapping together; the carry chains are `tests/fpga_carry.rs`'s to prove.
/// The pinned figures catch any change to either synthesis, which is then a
/// change to measure (`tools/vivado-parity/achieved.sh`) and to pin again
/// with `RETICLE_SYNTH_TABLE=1`, which prints the table. The legacy ones
/// are the figures before this synthesis existed, so they catch the flag
/// leaking a new mapping into the old netlist, which the Husky's builds
/// depend on; they would not catch a change that keeps all three counts,
/// which `examples/husky`'s byte-identical bitstreams do.
#[test]
fn every_design_maps_as_pinned_and_is_proved() {
    let print = std::env::var_os("RETICLE_SYNTH_TABLE").is_some();
    let results = every_design(|name| (synthesise(name, false), synthesise(name, true)));
    let mut wrong = Vec::new();
    for ((name, (legacy, vivado)), pinned) in DESIGNS.iter().zip(&results).zip(PINNED) {
        assert_eq!(pinned.0, *name, "PINNED is in DESIGNS' order");
        for (which, report) in [("legacy", legacy), ("vivado", vivado)] {
            match &report.mapping {
                Some(MapEquivalence::Equivalent) => {}
                other => wrong.push(format!("{name} ({which}): the mapping is {other:?}")),
            }
        }
        let got = (figures(legacy), figures(vivado));
        if print {
            println!("    (\"{name}\", {:?}, {:?}),", got.0, got.1);
        }
        if got != (pinned.1, pinned.2) {
            wrong.push(format!(
                "{name}: legacy {:?} vivado {:?}, pinned {:?} {:?}",
                got.0, got.1, pinned.1, pinned.2
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The designs where the default synthesis was asked to close a gap, and
/// what it closed it to: each no deeper in tables than Vivado's own netlist
/// was in levels on its worst path (`netlist.txt`, `vivado` lines). Would
/// catch the subtractions back off the chain (`adders` fifteen deep), the
/// XOR rebuilding gone (`crc32` five) or the depth priority gone (`fsm`
/// four); says nothing about a design pinned above that is not here.
#[test]
fn the_gaps_asked_about_are_closed() {
    for (name, deepest) in [("adders", 3), ("crc32", 3), ("fsm", 3), ("regfile_alu", 7)] {
        let report = synthesise(name, true);
        assert!(
            report.lut_depth <= deepest,
            "{name} is {} tables deep, Vivado's {deepest}",
            report.lut_depth
        );
    }
}

/// `testdata/fpga/vivado/qor/netlist.txt` has every design of the corpus, as
/// Vivado's synthesis made it and as Reticle's builds before and after made
/// it, each by Vivado's counting; and after, no clock's worst path has more
/// tables than before. (Its levels can be more: a subtraction on the chain is
/// a `CARRY4` a level where it was none, and `bram_fifo`'s most levels went
/// from five to seven, Vivado's own count.) Catches a regeneration that lost
/// a design or a build; not the flow moving on since, which only
/// `netlist_qor.sh` asks.
#[test]
fn the_fixture_has_every_design_and_after_is_no_deeper() {
    let text =
        std::fs::read_to_string(root().join("testdata/fpga/vivado/qor/netlist.txt")).unwrap();
    let mut levels: std::collections::BTreeMap<(String, String, String), u32> = Default::default();
    let mut cells: std::collections::BTreeSet<(String, String)> = Default::default();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first() {
            Some(&"cells") => {
                cells.insert((w[1].to_owned(), w[2].to_owned()));
            }
            Some(&"levels") => {
                // The tables on the worst path.
                levels.insert(
                    (w[1].to_owned(), w[2].to_owned(), w[3].to_owned()),
                    w[6].parse().unwrap(),
                );
            }
            _ => {}
        }
    }
    let mut corpus: Vec<&str> = DESIGNS.to_vec();
    corpus.extend(["esad", "sad"]);
    for name in corpus {
        for build in ["vivado", "before", "after"] {
            assert!(
                cells.contains(&(build.to_owned(), name.to_owned())),
                "no `{build}` cells of {name}"
            );
        }
    }
    for ((build, name, clock), after) in &levels {
        if build != "after" {
            continue;
        }
        let before = levels[&("before".to_owned(), name.clone(), clock.clone())];
        assert!(
            *after <= before,
            "{name} {clock}: {after} tables on the worst path after, {before} before"
        );
    }
}
