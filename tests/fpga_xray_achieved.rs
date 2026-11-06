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
/// build**, `before`, `after`, `synthesis`, `area` and `packing`. Would catch a design, a clock or a seed lost from a regeneration,
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
        for build in ["before", "after", "synthesis", "area", "packing"] {
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

/// **The default synthesis achieves a shorter period than the legacy one over
/// the corpus, and no clock is much worse.** `synthesis` is `after` with
/// `--synthesis vivado` (docs/fpga-vivado-parity.md, "Synthesis against
/// Vivado's"): the geometric mean against Vivado's own flow went from 1.437
/// to 1.274. This holds it at least 5 % under, and no clock's median more
/// than 10 % over its own `after`: the worst were `srl_delay`, 1.871 to
/// 2.027 ns on a netlist of Vivado's own seven tables two deep, a placement's
/// wires, and `sad`'s `clk_usb`, 2.588 to 2.787, one seed of five at 3.79.
/// Would catch a regeneration in which the improvement was lost or a clock
/// went far wrong; would not catch the flow losing it since, which needs
/// Vivado.
#[test]
fn the_default_synthesis_achieves_a_shorter_period_than_the_legacy_one() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 2];
    for ((design, clock), (vivado, builds)) in &table {
        let legacy = median(&builds["after"]);
        let synthesis = median(&builds["synthesis"]);
        assert!(
            synthesis <= legacy * 1.10,
            "{design} {clock}: {synthesis:.3} ns with the default synthesis against {legacy:.3}"
        );
        logs[0] += (legacy / vivado).ln();
        logs[1] += (synthesis / vivado).ln();
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let (legacy, synthesis) = ((logs[0] / n).exp(), (logs[1] / n).exp());
    assert!(
        synthesis < legacy * 0.95,
        "geometric mean against Vivado's own flow: {synthesis:.3}, legacy {legacy:.3}"
    );
}

/// **The default synthesis with "Area against Vivado's" is no slower than
/// without, over the corpus, and no clock is much worse.** `area` is
/// `synthesis` with a register's loads read as Vivado does and each `case` a
/// tree: the geometric mean against Vivado's own flow went from 1.274 to 1.249.
/// This holds it no higher, and no clock's median more than 5 % over its own
/// `synthesis`: the worst was `analyser_fifo`'s read clock, 5.248 to 5.352 ns,
/// inside its seeds' spread. Would catch a regeneration in which the area cost
/// the period; would not catch the flow losing it since, which needs Vivado.
#[test]
fn the_area_synthesis_is_no_slower_than_the_one_before() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 2];
    for ((design, clock), (vivado, builds)) in &table {
        let synthesis = median(&builds["synthesis"]);
        let area = median(&builds["area"]);
        assert!(
            area <= synthesis * 1.05,
            "{design} {clock}: {area:.3} ns with the area changes against {synthesis:.3}"
        );
        logs[0] += (synthesis / vivado).ln();
        logs[1] += (area / vivado).ln();
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let (synthesis, area) = ((logs[0] / n).exp(), (logs[1] / n).exp());
    assert!(
        area <= synthesis,
        "geometric mean against Vivado's own flow: {area:.3}, before {synthesis:.3}"
    );
}

/// **The packing of a RAM's registers, a carry in on `CYINIT` and a delay beside
/// its serialiser costs the corpus no clock, and reaches only the designs it
/// should.** `packing` is `area` with docs/fpga-vivado-parity.md, "Packing
/// against Vivado's": the geometric mean against Vivado's own flow went from
/// 1.249 to 1.248, `lutram32` 2.205 to 2.164 ns, `esad`'s and `sad`'s clocks each
/// within its seeds' spread either way, and every design with neither a
/// registered distributed RAM nor a carry in that is a signal built the same,
/// seed for seed. This holds the mean no higher, no clock's median more than 5 %
/// over its own `area` (the worst is `esad`'s `adc_sampleclk`, 3.033 to 3.084),
/// and the other designs' figures the same. Would catch a regeneration that lost
/// the change or mixed the builds, and a design the change should not reach that
/// moved; would not catch the flow losing it since, which needs Vivado.
#[test]
fn the_packing_costs_no_clock_and_reaches_only_its_designs() {
    let Some(table) = achieved() else { return };
    // Those with a registered distributed RAM or a carry in that is a signal;
    // `regfile_alu`'s RAMs are read by tables alone, and it built the same.
    let reached = ["esad", "lutram32", "sad"];
    let mut logs = [0.0f64; 2];
    for ((design, clock), (vivado, builds)) in &table {
        let (area, packing) = (&builds["area"], &builds["packing"]);
        if !reached.contains(&design.as_str()) {
            assert_eq!(packing, area, "{design} {clock}: the packing reached it");
        }
        let (area, packing) = (median(area), median(packing));
        assert!(
            packing <= area * 1.05,
            "{design} {clock}: {packing:.3} ns packed against {area:.3}"
        );
        logs[0] += (area / vivado).ln();
        logs[1] += (packing / vivado).ln();
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let (area, packing) = ((logs[0] / n).exp(), (logs[1] / n).exp());
    assert!(
        packing <= area,
        "geometric mean against Vivado's own flow: {packing:.3} packed, {area:.3} before"
    );
}
