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
    synthesise_text(name, text, &constraints_text, vivado)
}

/// One design, given as its text and its constraints', through the flow, the
/// mapping proved.
fn synthesise_text(name: &str, text: String, constraints_text: &str, vivado: bool) -> FlowReport {
    let mut map = SourceMap::new();
    let source = map.add(format!("{name}.v"), text).unwrap();
    let rcf = map
        .add(format!("{name}.rcf"), constraints_text.to_owned())
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
    let mut constraints = Constraints::parse(constraints_text, rcf, &mut diags);
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
/// Both map memories as Vivado does, so `analyser_fifo`'s memory being nine
/// `RAMB36E1` side by side with no read multiplexer took 29 and 26 tables off
/// both (`docs/fpga-vivado-parity.md`, "Memories as Vivado arranges them"):
/// 194 and 180 before. "The netlist against Vivado's, placed by Vivado"
/// moved four of the default's: equalities on the chain in lanes of two
/// (`carry_counter` 13 tables and 11 `CARRY4`s to 18 and 13, `counter64` 24
/// and 22 to 34 and 25, `analyser_fifo` 154 and 26 to 162 and 29, every one
/// a table fewer deep in front of its chain once legalised), and the adders
/// Vivado keeps in tables kept there (`bram_fifo` 27 and 12 to 47 and 6,
/// Vivado's own 49 and 3). "Three terms on one chain" moved two:
/// `analyser_fifo`'s `(wptr_r_next - rptr_next) + out_next` one chain (162
/// tables and 29 `CARRY4`s to 165 and 25), and `bram_fifo`'s `used + wr - rd`
/// (47 and 6 to 48 and 3, Vivado's count of `CARRY4`s). "A ground on the
/// bypass input" moved three: equalities in lanes of three again, their
/// generates tied on the bypass input and their `+ 1` on `CYINIT`
/// (`carry_counter` 18 tables and 13 `CARRY4`s to 12 and 11, `counter64` 34 and
/// 25 to 23 and 22, both Vivado's own counts, `analyser_fifo` 165 and 25 to
/// 156 and 22).
const PINNED: [(&str, Figures, Figures); 15] = [
    ("adders", (289, 15, 40), (231, 2, 56)),
    ("analyser_fifo", (165, 10, 12), (156, 3, 22)),
    ("barrel", (105, 3, 0), (87, 2, 0)),
    ("bram_fifo", (32, 4, 9), (48, 3, 3)),
    ("carry_counter", (18, 4, 8), (12, 1, 11)),
    ("counter64", (35, 5, 16), (23, 1, 22)),
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
/// Vivado's synthesis made it and as Reticle's builds before, after, with
/// "Area against Vivado's" (`area`), on 91802be8 (`refined`) and with "The
/// netlist against Vivado's, placed by Vivado" (`closing`) and "Three terms
/// on one chain" (`fused`) and "A way through to the propagate" (`through`)
/// made it, each by Vivado's counting; and in each build, no clock's worst path
/// has more tables than in the one before it (`through` against `tied`, whose
/// buffers in front of a counter's propagates went, `fused` against
/// `closing`, and `closing` against `refined`, whose
/// equality lanes lost their buffer tables and `esad`'s and `sad`'s worst
/// paths a table each). (Its levels can be more: a subtraction on the chain is
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
        for build in [
            "vivado", "before", "after", "area", "refined", "closing", "fused", "copied", "through",
        ] {
            assert!(
                cells.contains(&(build.to_owned(), name.to_owned())),
                "no `{build}` cells of {name}"
            );
        }
    }
    for ((build, name, clock), after) in &levels {
        let earlier = match build.as_str() {
            "after" => "before",
            "area" => "after",
            "closing" => "refined",
            "fused" => "closing",
            "copied" => "fused",
            "through" => "tied",
            _ => continue,
        };
        let before = levels[&(earlier.to_owned(), name.clone(), clock.clone())];
        assert!(
            *after <= before,
            "{name} {clock}: {after} tables on the worst path in `{build}`, {before} in `{earlier}`"
        );
    }
}

/// **`widemux`'s select bits that each drive 256 tables are copied, as in
/// Vivado's netlist** (`xray::replicate_wide_fanout`): two flip-flops each,
/// half the tables moved to the copy, under the default synthesis; none under
/// `--synthesis legacy`, which the Husky's builds use; and the mapping proved
/// either way (the copies come after it, and `replicate`'s unit tests hold
/// each copy to its original's inputs and parameters). Would catch the pass
/// not run, run under legacy, or its threshold or share moved.
#[test]
fn a_wide_fanout_is_copied_as_vivado_copies_it() {
    let vivado = synthesise("widemux", true);
    let legacy = synthesise("widemux", false);
    let r = &vivado.replicated;
    assert_eq!((r.registers, r.copies, r.moved), (2, 2, 256), "{r:?}");
    assert_eq!(vivado.count("FDRE"), 1049, "Vivado's 1049");
    assert_eq!(legacy.replicated, Default::default());
    assert_eq!(legacy.count("FDRE"), 1047);
}

/// **A reset the 7 series' flip-flops have only active high, computed by a
/// table, takes its inverter into that table under `--synthesis vivado`.**
/// `if (!(a && b && c)) q <= 0` was a table and an inverter table after it
/// before the eight reset pins; flipping the reset's polarity before the
/// mapping (`techcells::flip_reset_polarities`) leaves the one table. A reset
/// a flip-flop drives keeps its inverter, as Vivado's netlist does: `p`'s
/// here. Both mappings proved. Would catch the flip gone (two inverters, a
/// level more); would not catch the flip's own `not` read the wrong way
/// round, which the proof of the mapping does not reach (it starts after
/// the flip) and `techcells`' unit test does.
#[test]
fn a_computed_reset_takes_its_inverter_into_its_table() {
    let text = "module resets (input clk, input a, input b, input c, input [7:0] d,
                   output reg [7:0] q, output reg [7:0] p);
      reg ra, rb, rc, rz;
      always @(posedge clk) begin
        ra <= a; rb <= b; rc <= c; rz <= a;
        if (!(ra && rb && rc)) q <= 8'd0; else q <= d;
        if (!rz) p <= 8'd0; else p <= d;
      end
    endmodule
    "
    .to_owned();
    let vivado = synthesise_text("resets", text.clone(), "", true);
    let legacy = synthesise_text("resets", text, "", false);
    for (which, report) in [("vivado", &vivado), ("legacy", &legacy)] {
        assert!(
            matches!(report.mapping, Some(MapEquivalence::Equivalent)),
            "{which}: the mapping is {:?}",
            report.mapping
        );
    }
    // Every table: the reset's and `rz`'s inverter, both the mapper's now;
    // legacy the reset's and an inverter after the mapping for each.
    let tables = |r: &FlowReport| r.luts + r.device_cells.inverters();
    assert_eq!(
        (vivado.luts, vivado.device_cells.inverters()),
        (2, 0),
        "{vivado:?}"
    );
    assert_eq!(
        (legacy.luts, legacy.device_cells.inverters()),
        (1, 2),
        "{legacy:?}"
    );
    assert_eq!(tables(&vivado) + 1, tables(&legacy));
}

/// **The adders the design wrote stay in tables where Vivado leaves them**
/// under `--synthesis vivado` (`MapOptions::vivado_adder_widths`): `a + 1` to
/// eleven bits, `a + c` of a one-bit `c` to ten, two signals to five, each
/// on the chain one bit wider; and the legacy synthesis's chain from four
/// bits whatever the operands. Every mapping proved. Would catch a
/// threshold off by one either way, an operand class read as another (a
/// one-bit operand taken for two signals puts `a + c` at six bits on the
/// chain), and the rule leaking into the legacy flow; would not catch an
/// adder this mapping makes itself (a subtraction's, an equality's) left in
/// tables, which the pinned figures of the corpus do.
#[test]
fn the_adders_vivado_leaves_in_tables_stay_there() {
    let cases = [
        ("one", "r + 1'b1", 11),
        ("bit", "r + cr", 10),
        ("two", "r + s", 5),
    ];
    for (kind, sum, widest) in cases {
        for width in [widest, widest + 1] {
            let name = format!("add_{kind}_{width}");
            let text = format!(
                "module {name} (input clk, input [{h}:0] a, input [{h}:0] b, input c,
                               output reg [{h}:0] y);
                  reg [{h}:0] r, s; reg cr;
                  always @(posedge clk) begin r <= a; s <= b; cr <= c; y <= {sum}; end
                endmodule
                ",
                h = width - 1
            );
            for vivado in [true, false] {
                let report = synthesise_text(&name, text.clone(), "", vivado);
                assert!(
                    matches!(report.mapping, Some(MapEquivalence::Equivalent)),
                    "{name} (vivado {vivado}): the mapping is {:?}",
                    report.mapping
                );
                let chained = report.count("CARRY4") > 0;
                let want = !vivado || width > widest;
                assert_eq!(
                    chained, want,
                    "{name}, vivado {vivado}: {:?}",
                    report.netlist
                );
            }
        }
    }
}

/// **With a way through the lane's table in front of a counter's propagate,
/// Vivado counts the counters' tables as its own**
/// (`docs/fpga-vivado-parity.md`, "A way through to the propagate"). Vivado's
/// counting of seed 1's netlist: `carry_counter` 45 tables in `tied`, 14 in
/// `through`, against Vivado's 12; `counter64` 88, then 25, against 23;
/// `gray_cdc` 26, then 15, against 15, with the same `CARRY4`s. This holds each
/// within two tables of Vivado's in `through`, at least ten over in `tied`, and
/// no `CARRY4` gained or lost. Catches a regeneration that lost the change or
/// swapped the builds; not the flow losing it since, which only
/// `netlist_qor.sh` asks.
#[test]
fn a_counters_propagates_through_their_tables_are_no_tables_to_vivado() {
    let text =
        std::fs::read_to_string(root().join("testdata/fpga/vivado/qor/netlist.txt")).unwrap();
    let cells = |build: &str, design: &str| -> (u32, u32) {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("cells {build} {design} ")))
            .map(|rest| {
                let w: Vec<u32> = rest
                    .split_whitespace()
                    .map(|n| n.parse().unwrap())
                    .collect();
                (w[0], w[1])
            })
            .unwrap_or_else(|| panic!("no `{build}` cells of {design}"))
    };
    for design in ["carry_counter", "counter64", "gray_cdc"] {
        let (vivado, through, tied) = (
            cells("vivado", design),
            cells("through", design),
            cells("tied", design),
        );
        assert!(
            through.0 <= vivado.0 + 2 && tied.0 >= vivado.0 + 10,
            "{design}: {} tables through, {} tied, Vivado's {}",
            through.0,
            tied.0,
            vivado.0
        );
        assert_eq!(through.1, tied.1, "{design}: CARRY4s");
    }
}
