//! Golden tests for the formal engines.
//!
//! Every `testdata/formal/*.rtl` design is parsed and checked; the report
//! text (verdict, cover witnesses, counter-example traces) followed by the
//! rendered diagnostics must match `<name>.formal`. Run with
//! `UPDATE_EXPECT=1` to rewrite the expected files after a deliberate
//! change.
//!
//! The top module's attributes select the check:
//!
//! | Attribute            | Meaning                                              |
//! |----------------------|------------------------------------------------------|
//! | `formal_mode`        | `verify` (default), `equiv` or `reach`               |
//! | `formal_other`       | the second module of an equivalence check            |
//! | `formal_depth`       | BMC / lint depth (default 20)                        |
//! | `formal_k`           | largest induction depth (default 10)                 |
//! | `formal_init`        | `reset` (default), `zero` or `free`                  |
//! | `formal_match_state` | match state by name in equivalence (default 1)       |

#![cfg(feature = "formal")]

use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::Diagnostics;
use reticle::formal::{
    EquivOptions, InitMode, ReachOptions, VerifyOptions, check_equivalent, reach_lint, verify,
};
use reticle::ir::{AttrValue, Design, Module};
use reticle::source::SourceMap;

fn golden_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/formal");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rtl"))
        .collect();
    paths.sort();
    paths
}

fn attr_u32(m: &Module, key: &str, default: u32) -> u32 {
    m.attrs
        .get(key)
        .and_then(AttrValue::as_int)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(default)
}

fn attr_str<'a>(m: &'a Module, key: &str) -> Option<&'a str> {
    m.attrs.get(key).and_then(AttrValue::as_str)
}

fn init_mode(m: &Module) -> InitMode {
    match attr_str(m, "formal_init") {
        Some("zero") => InitMode::Zero,
        Some("free") => InitMode::Free,
        _ => InitMode::Reset,
    }
}

/// Runs the check the top module asks for; returns the report text and
/// the diagnostics.
fn run(design: &Design) -> (String, Diagnostics) {
    let top = design.top.expect("golden design names a top");
    let m = design.module(top);
    let depth = attr_u32(m, "formal_depth", 20);
    let max_k = attr_u32(m, "formal_k", 10);
    let init = init_mode(m);
    match attr_str(m, "formal_mode").unwrap_or("verify") {
        "equiv" => {
            let other_name = attr_str(m, "formal_other").expect("formal_other names module b");
            let other = design
                .module_by_name(other_name)
                .unwrap_or_else(|| panic!("no module `{other_name}`"));
            let opts = EquivOptions {
                depth,
                max_k,
                init,
                match_state_by_name: attr_u32(m, "formal_match_state", 1) != 0,
                ..EquivOptions::default()
            };
            let r = check_equivalent(design, top, other, &opts);
            (r.render(m.name.as_str(), other_name), r.diags)
        }
        "reach" => {
            let opts = ReachOptions {
                depth,
                init,
                ..ReachOptions::default()
            };
            let r = reach_lint(design, top, &opts);
            (r.render(m.name.as_str()), r.diags)
        }
        _ => {
            let opts = VerifyOptions {
                depth,
                max_k,
                init,
                ..VerifyOptions::default()
            };
            let r = verify(design, top, &opts);
            (r.render(), r.diags)
        }
    }
}

/// The first line at which two texts differ, for a readable failure.
fn first_difference(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!("line {}:\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!(
        "expected {} lines, got {}",
        expected.lines().count(),
        actual.lines().count()
    )
}

#[test]
fn golden_formal_designs() {
    let paths = golden_files();
    assert!(!paths.is_empty(), "no golden files found");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut failures = Vec::new();

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let design = match Design::parse_text(&text, file) {
            Ok(design) => design,
            Err(diags) => {
                failures.push(format!("{name}: parse failed\n{}", diags.render(&map)));
                continue;
            }
        };
        let (report, mut diags) = run(&design);
        diags.sort();
        let mut actual = report;
        if !diags.is_empty() {
            actual.push('\n');
            actual.push_str(&diags.render(&map));
        }
        let expected_path = path.with_extension("formal");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();
        if actual != expected {
            if update {
                fs::write(&expected_path, &actual).unwrap();
            } else {
                failures.push(format!(
                    "{name}: report differs from {} (set UPDATE_EXPECT=1 to rewrite)\n{}",
                    expected_path.display(),
                    first_difference(&expected, &actual)
                ));
            }
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The verdict of an equivalence check, without its evidence.
#[cfg(feature = "synth")]
fn verdict(outcome: &reticle::formal::EquivOutcome) -> String {
    use reticle::formal::EquivOutcome;
    match outcome {
        EquivOutcome::Equivalent(proof) => format!("equivalent {proof:?}"),
        EquivOutcome::Different { frame, .. } => format!("different at {frame}"),
        EquivOutcome::Unknown { depth } => format!("unknown {depth:?}"),
    }
}

/// Every equivalence golden, decided by the monolithic engine and by
/// SAT sweeping alone (no direct attempt, no exhaustive simulation):
/// the verdicts must match, and a difference the sweep reports must come
/// with a trace that shows it.
#[cfg(feature = "synth")]
#[test]
fn the_sweep_agrees_with_the_monolithic_engine() {
    use reticle::formal::{EquivEngine, EquivOutcome, SweepOptions};

    let sweep = EquivEngine::Sweep(SweepOptions {
        quick_conflicts: 0,
        exhaustive_budget: 0,
        ..SweepOptions::default()
    });
    let mut checked = 0;
    for path in golden_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        let mut map = SourceMap::new();
        let file = map.add(name.clone(), text.clone()).unwrap();
        let design = Design::parse_text(&text, file).unwrap();
        let top = design.top.expect("a top");
        let m = design.module(top);
        if attr_str(m, "formal_mode") != Some("equiv") {
            continue;
        }
        let other = design
            .module_by_name(attr_str(m, "formal_other").unwrap())
            .unwrap();
        let options = |engine: EquivEngine| EquivOptions {
            depth: attr_u32(m, "formal_depth", 20),
            max_k: attr_u32(m, "formal_k", 10),
            init: init_mode(m),
            match_state_by_name: attr_u32(m, "formal_match_state", 1) != 0,
            engine,
            ..EquivOptions::default()
        };
        let old = check_equivalent(&design, top, other, &options(EquivEngine::Monolithic));
        let new = check_equivalent(&design, top, other, &options(sweep.clone()));
        assert_eq!(verdict(&old.outcome), verdict(&new.outcome), "{name}");
        if let EquivOutcome::Different {
            properties, trace, ..
        } = &new.outcome
        {
            // The miter's outputs are the two modules' outputs side by
            // side, so a real difference shows in the trace itself.
            let last = trace.frames.last().expect("a frame");
            let half = last.outputs.len() / 2;
            assert!(!properties.is_empty(), "{name}");
            assert!(
                (0..half).any(|i| last.outputs[i].1 != last.outputs[half + i].1),
                "{name}: the trace shows no difference\n{}",
                trace.render()
            );
        }
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} equivalence goldens");
}
