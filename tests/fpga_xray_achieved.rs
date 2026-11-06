//! The clock period Reticle's 7-series implementations achieve under Vivado's
//! timer, against Vivado's own flow on the same designs:
//! `testdata/fpga/vivado/qor/achieved.txt`, written by
//! `tools/vivado-parity/achieved.sh` and `achieved.py table --fixture`. Vivado
//! is not needed: the fixture carries its answers.
//!
//! **What these hold is a quoted run, not the flow.** They catch a fixture
//! regenerated into one that lost what the run measured, and a run that left a
//! design or a seed out; they do not catch the flow moving on since, which only
//! running `achieved.sh` again asks.

#![cfg(feature = "fpga")]

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Per (design, clock): Vivado's figure, and each build's figures by seed.
type Table = BTreeMap<(String, String), (f64, BTreeMap<String, Vec<f64>>)>;

fn achieved() -> Option<Table> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/qor/achieved.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipped: `{}` is not in the published crate",
            path.display()
        );
        return None;
    };
    let mut table = Table::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["vivado", design, clock, ns] => {
                let entry = table
                    .entry(((*design).to_owned(), (*clock).to_owned()))
                    .or_default();
                entry.0 = ns.parse().expect("a figure");
            }
            ["reticle", build, _seed, design, clock, ns] => {
                table
                    .entry(((*design).to_owned(), (*clock).to_owned()))
                    .or_default()
                    .1
                    .entry((*build).to_owned())
                    .or_default()
                    .push(ns.parse().expect("a figure"));
            }
            ["netlist", ..] => {}
            other => panic!("a line of achieved.txt this does not know: {other:?}"),
        }
    }
    Some(table)
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// **Every clock of the corpus has Vivado's figure and five seeds of each
/// build.** Would catch a design, a clock or a seed lost from a regeneration,
/// which would make the table below a median of fewer placements than it says.
#[test]
fn every_clock_has_vivados_figure_and_five_seeds_of_each_build() {
    let Some(table) = achieved() else { return };
    assert_eq!(
        table.len(),
        21,
        "seventeen designs, four of them on two clocks"
    );
    for ((design, clock), (vivado, builds)) in &table {
        assert!(*vivado > 0.0, "{design} {clock}: no Vivado figure");
        for build in ["before", "after"] {
            let seeds = builds.get(build).map_or(0, Vec::len);
            assert_eq!(seeds, 5, "{design} {clock}: {seeds} seeds of `{build}`");
        }
    }
}

/// **This change's build achieves a shorter period than the one before it,
/// over the corpus, and no clock is much worse.** The geometric mean of each
/// clock's median over its seeds, against Vivado's own figure, was 1.576
/// before and 1.437 after; this holds the after at least 5 % under the before,
/// and no clock's median more than 5 % over its own before (the worst was
/// `gray_cdc`'s `clk2`, 1.670 to 1.736 ns, within its seeds' spread). Would
/// catch a regeneration in which the improvement was lost or a clock went far
/// wrong; would not catch the flow losing it since, which needs Vivado.
#[test]
fn the_build_after_achieves_a_shorter_period_than_the_one_before() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 2];
    for ((design, clock), (vivado, builds)) in &table {
        let before = median(&builds["before"]);
        let after = median(&builds["after"]);
        assert!(
            after <= before * 1.05,
            "{design} {clock}: {after:.3} ns after against {before:.3} before"
        );
        logs[0] += (before / vivado).ln();
        logs[1] += (after / vivado).ln();
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let (before, after) = ((logs[0] / n).exp(), (logs[1] / n).exp());
    assert!(
        after < before * 0.95,
        "geometric mean against Vivado's own flow: {after:.3} after, {before:.3} before"
    );
}
