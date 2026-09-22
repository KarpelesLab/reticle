//! Post-synthesis equivalence checking on the golden designs.
//!
//! [`reticle::synth::verify::check_synthesis`] proves the fully optimised
//! and cellified result equivalent to the same source lowered with process
//! lowering alone; see that module's docs for what the proof does and does
//! not cover.
//!
//! Every golden design gets an expected verdict here, so both the proofs
//! and the documented limitations stay visible:
//!
//! - `Proved`: the optimiser and cellify preserved behaviour, and the
//!   miter is inductive.
//! - `Inconclusive`: the check could not run or could not close. The
//!   causes in this corpus are a register with no reset value (`F0018`,
//!   which sequential equivalence from reset needs), a latch (`F0004`)
//!   and a process the bit-blaster cannot read (`F0001`).
//! - `Different`: the optimiser resolved a don't-care the reference still
//!   holds as `x`. Honest disagreement, explained in the module docs.

#![cfg(all(feature = "synth", feature = "formal"))]

use std::fs;
use std::path::Path;

use reticle::diag::Diagnostics;
use reticle::formal::{EquivOptions, EquivOutcome, InitMode};
use reticle::ir::Design;
use reticle::source::SourceMap;
use reticle::synth::verify::{SynthVerifyReport, VerifyOptions, check_synthesis_by_name};
use reticle::synth::{SynthOptions, run};

/// What the check is expected to say about a design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// Proved equivalent.
    Proved,
    /// Neither proved nor refuted; see the module docs.
    Inconclusive,
    /// A difference was found.
    Different,
}

fn verdict(report: &SynthVerifyReport) -> Verdict {
    match report.outcome {
        EquivOutcome::Equivalent(_) => Verdict::Proved,
        EquivOutcome::Unknown { .. } => Verdict::Inconclusive,
        EquivOutcome::Different { .. } => Verdict::Different,
    }
}

/// Parses `testdata/synth/<name>.rtl`.
fn load(name: &str) -> Design {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/synth")
        .join(format!("{name}.rtl"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut map = SourceMap::new();
    let file = map.add(format!("{name}.rtl"), text.clone()).unwrap();
    Design::parse_text(&text, file)
        .unwrap_or_else(|d| panic!("{name}: parse failed\n{}", d.render(&map)))
}

/// A shallow bound: these designs are tiny, and a proof is inductive.
fn options() -> VerifyOptions {
    VerifyOptions {
        equiv: EquivOptions {
            depth: 6,
            max_k: 3,
            init: InitMode::Reset,
            ..EquivOptions::default()
        },
        ..VerifyOptions::default()
    }
}

/// Synthesises `name` with the default pipeline and checks its top module.
fn check(name: &str, options: &VerifyOptions) -> SynthVerifyReport {
    let original = load(name);
    let mut synthesised = original.clone();
    let mut diags = Diagnostics::new();
    run(&mut synthesised, &SynthOptions::default(), &mut diags);
    check_synthesis_by_name(&original, &synthesised, name, options)
        .unwrap_or_else(|| panic!("{name}: the module vanished"))
}

/// Every golden design with the verdict it must receive.
const EXPECTED: &[(&str, Verdict)] = &[
    ("adder_tree", Verdict::Proved),
    ("async_reg", Verdict::Proved),
    ("blocking_ssa", Verdict::Proved),
    ("casez_priority", Verdict::Proved),
    ("counter_en", Verdict::Proved),
    ("fsm", Verdict::Proved),
    ("mux_tree", Verdict::Proved),
    // The optimiser resolves `mux(s, a, 'x)` to `a`; the reference keeps
    // the `x`, which the blaster reads as a free value.
    ("const_chain", Verdict::Different),
    // Registers without a reset value: sequential equivalence from reset
    // has no initial state to start from (`F0018`).
    ("dead_logic", Verdict::Inconclusive),
    ("dup_logic", Verdict::Inconclusive),
    ("ram_regread", Verdict::Inconclusive),
    // A latch, which the bit-blaster does not model (`F0004`).
    ("case_latch", Verdict::Inconclusive),
    // Keeps a `wait` process on purpose, which the blaster refuses.
    ("unsynth", Verdict::Inconclusive),
];

#[test]
fn golden_designs_get_the_expected_verdict() {
    let options = options();
    let mut failures = Vec::new();
    for (name, want) in EXPECTED {
        let report = check(name, &options);
        let got = verdict(&report);
        if got != *want {
            failures.push(format!(
                "{name}: expected {want:?}, got {got:?}\n{}",
                report.render()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The inconclusive verdicts name their cause, rather than something
/// silently going wrong.
#[test]
fn inconclusive_verdicts_say_why() {
    let options = options();
    for name in ["dead_logic", "dup_logic"] {
        let report = check(name, &options);
        assert!(
            report.diags.iter().any(|d| d.code == Some("F0018")),
            "{name}: no reset-value diagnostic\n{}",
            report.render()
        );
    }
    let report = check("unsynth", &options);
    assert!(
        report.diags.iter().any(|d| d.code == Some("F0001")),
        "unsynth: no process diagnostic\n{}",
        report.render()
    );
}

/// The pipeline's own switch reaches the same verdict, and records it.
#[test]
fn the_pipeline_records_the_verdict() {
    let mut design = load("counter_en");
    let mut diags = Diagnostics::new();
    let stats = run(
        &mut design,
        &SynthOptions {
            verify_equivalence: true,
            ..SynthOptions::default()
        },
        &mut diags,
    );
    assert_eq!(stats.verification.len(), 1);
    assert!(stats.verification[0].passed(), "{}", stats.render(None));
    assert!(!diags.has_errors());
    assert!(
        stats
            .render(None)
            .contains("synthesis of counter_en: EQUIVALENT")
    );
}

/// A failing check is an error with the counter-example attached, so the
/// pipeline does not quietly hand back a netlist it could not confirm.
#[test]
fn a_difference_is_reported_as_an_error() {
    let mut design = load("const_chain");
    let mut diags = Diagnostics::new();
    let stats = run(
        &mut design,
        &SynthOptions {
            verify_equivalence: true,
            ..SynthOptions::default()
        },
        &mut diags,
    );
    assert_eq!(stats.verification.len(), 1);
    assert_eq!(verdict(&stats.verification[0]), Verdict::Different);
    let problem = diags
        .iter()
        .find(|d| d.code == Some("S0031"))
        .expect("the failure is reported");
    assert!(problem.is_error());
    assert!(problem.notes.iter().any(|n| n.contains("DIFFERENT")));
}

/// The same table at the default bound, which is much deeper.
#[test]
#[ignore = "slow: deeper sequential equivalence on every golden"]
fn golden_designs_at_a_deeper_bound() {
    let options = VerifyOptions::default();
    for (name, want) in EXPECTED {
        let report = check(name, &options);
        assert_eq!(verdict(&report), *want, "{name}: {}", report.render());
    }
}

/// The combinational goldens decided by SAT sweeping alone (no direct
/// attempt, no exhaustive simulation) and by the monolithic engine: the
/// verdicts must match the table and each other. The sequential goldens
/// do not sweep, so the engines cannot differ on them.
#[test]
fn the_sweep_agrees_with_the_monolithic_engine() {
    use reticle::formal::{EquivEngine, SweepOptions};

    let with = |engine: EquivEngine| {
        let mut options = options();
        options.equiv.engine = engine;
        options
    };
    let monolithic = with(EquivEngine::Monolithic);
    let sweep = with(EquivEngine::Sweep(SweepOptions {
        quick_conflicts: 0,
        exhaustive_budget: 0,
        ..SweepOptions::default()
    }));
    for (name, want) in EXPECTED {
        let old = verdict(&check(name, &monolithic));
        let new = check(name, &sweep);
        assert_eq!(old, *want, "{name}, monolithic");
        assert_eq!(verdict(&new), *want, "{name}, sweep: {}", new.render());
    }
}
