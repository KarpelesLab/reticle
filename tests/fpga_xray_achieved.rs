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
            ["netlist", ..] | ["slices", ..] | ["vnetlist", ..] | ["pnr", ..] => {}
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

/// **The ways through a lookup table cost the corpus no clock.** `throughs` is
/// docs/fpga-vivado-parity.md, "Eight flip-flops a slice", at ten seeds, and the
/// tree without it is `packing` (seeds 1 to 5) and `bypass` (6 to 10): the
/// geometric mean of each clock's median against Vivado's own flow went from
/// 1.256 to 1.248. This holds the mean no higher, ten seeds of each, and no
/// clock's median more than 6 % over its own before: the worst is
/// `srl_delay`, 2.057 to 2.170 ns, 5.5 %, the one clock this holds looser than
/// the sections before. Would catch a regeneration that lost the change or mixed
/// the builds; would not catch the flow losing it since, which needs Vivado.
#[test]
fn the_ways_through_a_table_cost_the_corpus_no_clock() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 2];
    for ((design, clock), (vivado, builds)) in &table {
        let mut before = builds["packing"].clone();
        before.extend(&builds["bypass"]);
        let after = &builds["throughs"];
        assert_eq!((before.len(), after.len()), (10, 10), "{design} {clock}");
        let (before, after) = (median(&before), median(after));
        assert!(
            after <= before * 1.06,
            "{design} {clock}: {after:.3} ns through tables against {before:.3}"
        );
        logs[0] += (before / vivado).ln();
        logs[1] += (after / vivado).ln();
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let (before, after) = ((logs[0] / n).exp(), (logs[1] / n).exp());
    assert!(
        after <= before,
        "geometric mean against Vivado's own flow: {after:.3} through tables, {before:.3} before"
    );
}

/// Per build, its slices by seed, summed over the corpus's designs.
fn slices() -> Option<BTreeMap<String, BTreeMap<u32, u32>>> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/qor/achieved.txt");
    let text = std::fs::read_to_string(&path).ok()?;
    let mut out: BTreeMap<String, BTreeMap<u32, u32>> = BTreeMap::new();
    let mut designs: BTreeMap<(String, u32), u32> = BTreeMap::new();
    for line in text.lines() {
        if let ["slices", build, seed, _design, n] =
            line.split_whitespace().collect::<Vec<_>>().as_slice()
        {
            let seed: u32 = seed.parse().expect("a seed");
            *out.entry((*build).to_owned())
                .or_default()
                .entry(seed)
                .or_default() += n.parse::<u32>().expect("a count");
            *designs.entry(((*build).to_owned(), seed)).or_default() += 1;
        }
    }
    for ((build, seed), n) in &designs {
        assert_eq!(*n, 17, "`{build}` seed {seed}: {n} designs' slices");
    }
    Some(out)
}

/// **Filling slices costs the corpus no clock and a tenth of its slices.**
/// docs/fpga-vivado-parity.md, "Filling slices": `tree` is the tree before it,
/// `fill` it with the section's four changes and `fillall` those with
/// `--route-throughs all`, the default now, ten seeds each. The geometric mean of
/// each clock's median against Vivado's own flow was 1.247, 1.239 and 1.231; this
/// holds the default's and the four changes' no higher than the tree's, and no
/// clock's median more than 6 % over its own before (the worst, `lutram32` and
/// `esad`'s `clk_usb` by default, 3.8 %); and the slices, summed over the corpus
/// and the median over the seeds, 1,822 before and 1,629 and 1,600 after, at least
/// a tenth fewer. Would catch a regeneration that lost the change or mixed the
/// builds, and a seed or a design lost from the slices; would not catch the flow
/// losing it since, which needs Vivado.
#[test]
fn filling_slices_costs_the_corpus_no_clock_and_a_tenth_of_its_slices() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 3];
    for ((design, clock), (vivado, builds)) in &table {
        let runs: Vec<&Vec<f64>> = ["tree", "fill", "fillall"]
            .iter()
            .map(|b| &builds[*b])
            .collect();
        for (run, name) in runs.iter().zip(["tree", "fill", "fillall"]) {
            assert_eq!(run.len(), 10, "{design} {clock}: seeds of `{name}`");
        }
        let medians: Vec<f64> = runs.iter().map(|r| median(r)).collect();
        for (after, name) in medians[1..].iter().zip(["fill", "fillall"]) {
            assert!(
                *after <= medians[0] * 1.06,
                "{design} {clock}: {after:.3} ns `{name}` against {:.3}",
                medians[0]
            );
        }
        for (log, m) in logs.iter_mut().zip(&medians) {
            *log += (m / vivado).ln();
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let [tree, fill, fillall] = logs.map(|l| (l / n).exp());
    assert!(
        fill <= tree && fillall <= tree,
        "geometric mean against Vivado's own flow: {fill:.3} and {fillall:.3} against {tree:.3}"
    );
    let Some(slices) = slices() else { return };
    let total = |build: &str| {
        let mut v: Vec<f64> = slices[build].values().map(|n| f64::from(*n)).collect();
        assert_eq!(v.len(), 10, "ten seeds of `{build}`'s slices");
        v.sort_by(f64::total_cmp);
        median(&v)
    };
    let (before, fill, fillall) = (total("tree"), total("fill"), total("fillall"));
    assert!(
        fill <= before * 0.9 && fillall <= before * 0.9,
        "slices: {fill} and {fillall} against {before}"
    );
}

/// **The routes of "Routing against Vivado's routes" make the corpus faster and
/// close most of the distance to Vivado's own routes of the same placements.**
/// `routed` is `fillall` with the change, ten seeds each, and `vroutes` is
/// `fillall`'s placements at seeds 1 to 5 routed by Vivado with the tables' pins
/// free. The geometric mean against Vivado's own flow went from 1.231 to 1.195;
/// this holds it at least 2 % under, no clock's median more than 4 % over its own
/// `fillall` (the worst, `analyser_fifo`'s `wr_clk`, 2.6 %), and over seeds 1 to
/// 5, where `vroutes` is 1.194, `routed` within 1 % of it (1.198). Would catch a
/// regeneration that lost the change, swapped `routed` and `fillall` or lost a
/// seed (each made and caught); would not catch `vroutes` replaced by slower
/// figures, `routed`'s own among them (made, and passed), nor the flow losing
/// the change since, which needs Vivado.
#[test]
fn the_routes_close_most_of_the_distance_to_vivados_on_the_same_placements() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 4];
    for ((design, clock), (vivado, builds)) in &table {
        let (fill, routed, vroutes) = (&builds["fillall"], &builds["routed"], &builds["vroutes"]);
        assert_eq!(routed.len(), 10, "{design} {clock}: seeds of `routed`");
        assert_eq!(vroutes.len(), 5, "{design} {clock}: seeds of `vroutes`");
        let (before, after) = (median(fill), median(routed));
        assert!(
            after <= before * 1.04,
            "{design} {clock}: {after:.3} ns routed against {before:.3}"
        );
        for (log, ns) in logs
            .iter_mut()
            .zip([before, after, median(&routed[..5]), median(vroutes)])
        {
            *log += (ns / vivado).ln();
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let [before, after, first, vivados] = logs.map(|l| (l / n).exp());
    assert!(
        after <= before * 0.98,
        "geometric mean against Vivado's own flow: {after:.3} routed, {before:.3} before"
    );
    assert!(
        first <= vivados * 1.01,
        "seeds 1 to 5: {first:.3} routed against {vivados:.3} for Vivado's routes"
    );
}

/// **Giving each late lookup table's inputs as Vivado does makes the corpus
/// faster, and faster than Vivado's own routes of the same placements.**
/// `refined` is `routed` with docs/fpga-vivado-parity.md, "A table's inputs as
/// Vivado gives them", ten seeds each, the placements the same (the tree it was
/// measured on built `routed` again seed for seed); `vroutes` is Vivado's
/// routes of seeds 1 to 5. The geometric mean against Vivado's own flow went
/// from 1.195 to 1.165; this holds it at least 2 % under, no clock's median
/// more than 2 % over its own `routed` (the worst, `esad`'s `clk_usb`, 0.6 %),
/// and over seeds 1 to 5, where `vroutes` is 1.194, `refined` (1.163) no slower
/// than it. Would catch a regeneration that lost the change, swapped `refined`
/// and `routed`, or lost a seed; would not catch the flow losing the change
/// since, which needs Vivado.
#[test]
fn a_tables_inputs_given_as_vivado_gives_them_make_the_corpus_faster() {
    let Some(table) = achieved() else { return };
    let mut logs = [0.0f64; 4];
    for ((design, clock), (vivado, builds)) in &table {
        let (routed, refined, vroutes) =
            (&builds["routed"], &builds["refined"], &builds["vroutes"]);
        assert_eq!(refined.len(), 10, "{design} {clock}: seeds of `refined`");
        let (before, after) = (median(routed), median(refined));
        assert!(
            after <= before * 1.02,
            "{design} {clock}: {after:.3} ns refined against {before:.3}"
        );
        for (log, ns) in
            logs.iter_mut()
                .zip([before, after, median(&refined[..5]), median(vroutes)])
        {
            *log += (ns / vivado).ln();
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let [before, after, first, vivados] = logs.map(|l| (l / n).exp());
    assert!(
        after <= before * 0.98,
        "geometric mean against Vivado's own flow: {after:.3} refined, {before:.3} before"
    );
    assert!(
        first <= vivados,
        "seeds 1 to 5: {first:.3} refined against {vivados:.3} for Vivado's routes"
    );
}

/// Per (design, clock): Vivado's netlist placed and routed by Vivado
/// (`vnetlist`), and each build's netlist placed and routed the same way by
/// seed (`pnr`).
fn netlists() -> Option<Table> {
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
            ["vnetlist", design, clock, ns] => {
                let entry = table
                    .entry(((*design).to_owned(), (*clock).to_owned()))
                    .or_default();
                entry.0 = ns.parse().expect("a figure");
            }
            ["pnr", build, _seed, design, clock, ns] => {
                table
                    .entry(((*design).to_owned(), (*clock).to_owned()))
                    .or_default()
                    .1
                    .entry((*build).to_owned())
                    .or_default()
                    .push(ns.parse().expect("a figure"));
            }
            _ => {}
        }
    }
    Some(table)
}

/// **The netlist of "The netlist against Vivado's, placed by Vivado" closes
/// most of the distance to Vivado's netlist, and Reticle's flow is faster for
/// it** (`docs/fpga-vivado-parity.md`). Vivado placing and routing both, the
/// geometric mean of Reticle's netlist against Vivado's went from 1.092
/// (`refined`) to 1.033 (`closing`), seeds 1 to 5; Reticle's own flow from
/// 1.165 to 1.119 against Vivado's own, ten seeds, the worst clock `sad`'s ADC
/// clock 2.8 % over its own. This holds the netlist at least 4 % nearer
/// Vivado's and within 5 % of it, the flow at least 3 % faster, no clock's
/// median more than 4 % over its own, and every clock with Vivado's netlist's
/// figure and five seeds of each netlist. Would catch a regeneration that lost
/// the change, swapped the builds or lost a seed or a clock; would not catch
/// the flow losing the change since, which needs Vivado.
#[test]
fn the_netlist_closes_on_vivados_own_placement() {
    let (Some(table), Some(nets)) = (achieved(), netlists()) else {
        return;
    };
    assert_eq!(nets.len(), table.len(), "every clock has its netlists");
    let mut logs = [0.0f64; 4];
    for ((design, clock), (vivado, builds)) in &table {
        let (refined, closing) = (&builds["refined"], &builds["closing"]);
        assert_eq!(closing.len(), 10, "{design} {clock}: seeds of `closing`");
        let (before, after) = (median(refined), median(closing));
        assert!(
            after <= before * 1.04,
            "{design} {clock}: {after:.3} ns closing against {before:.3}"
        );
        let (own, placed) = &nets[&(design.clone(), clock.clone())];
        assert!(*own > 0.0, "{design} {clock}: no `vnetlist`");
        let (net_before, net_after) = (&placed["refined"], &placed["closing"]);
        assert_eq!(
            (net_before.len(), net_after.len()),
            (5, 5),
            "{design} {clock}: seeds of `pnr`"
        );
        for (log, ratio) in logs.iter_mut().zip([
            before / vivado,
            after / vivado,
            median(net_before) / own,
            median(net_after) / own,
        ]) {
            *log += ratio.ln();
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let [before, after, net_before, net_after] = logs.map(|l| (l / n).exp());
    assert!(
        after <= before * 0.97,
        "Reticle's flow against Vivado's: {after:.3} closing, {before:.3} refined"
    );
    assert!(
        net_after <= net_before * 0.96 && net_after <= 1.05,
        "the netlist against Vivado's: {net_after:.3} closing, {net_before:.3} refined"
    );
}

/// **Three terms on one chain make `analyser_fifo`'s read clock faster, and
/// cost the corpus nothing** (`docs/fpga-vivado-parity.md`, "Three terms on
/// one chain"). `(wptr_r_next - rptr_next) + out_next` was two chains in a row,
/// and is one: Vivado placing and routing both netlists, the read clock went
/// from 5.097 ns (`closing`) to 4.371 (`fused`), seeds 1 to 5, against
/// Vivado's netlist's 4.087; in Reticle's own flow from 5.213 to 4.426, ten
/// seeds. The geometric means went from 1.033 to 1.027 (the netlist against
/// Vivado's) and from 1.119 to 1.111 (the flow against Vivado's own). This
/// holds the read clock at least 10 % faster at both levels, no clock's median
/// more than 4 % over `closing`'s at either, neither mean worse, and every
/// clock with ten seeds of `fused` and five of its netlist. Would catch a
/// regeneration that lost the change, swapped the builds or lost a seed or a
/// clock; would not catch the flow losing the change since, which needs
/// Vivado.
#[test]
fn three_terms_on_one_chain_close_the_fifos_read_clock() {
    let (Some(table), Some(nets)) = (achieved(), netlists()) else {
        return;
    };
    let mut logs = [0.0f64; 4];
    for ((design, clock), (vivado, builds)) in &table {
        let (closing, fused) = (&builds["closing"], &builds["fused"]);
        assert_eq!(fused.len(), 10, "{design} {clock}: seeds of `fused`");
        let (own, placed) = &nets[&(design.clone(), clock.clone())];
        let (net_closing, net_fused) = (&placed["closing"], &placed["fused"]);
        assert_eq!(net_fused.len(), 5, "{design} {clock}: seeds of `pnr fused`");
        let medians = [
            median(closing),
            median(fused),
            median(net_closing),
            median(net_fused),
        ];
        for (before, after, what) in [
            (medians[0], medians[1], "the flow"),
            (medians[2], medians[3], "the netlist"),
        ] {
            assert!(
                after <= before * 1.04,
                "{design} {clock}, {what}: {after:.3} ns fused against {before:.3}"
            );
            if design == "analyser_fifo" && clock == "rd_clk" {
                assert!(
                    after <= before * 0.90,
                    "the read clock, {what}: {after:.3} ns fused against {before:.3}"
                );
            }
        }
        for (log, ratio) in logs.iter_mut().zip([
            medians[0] / vivado,
            medians[1] / vivado,
            medians[2] / own,
            medians[3] / own,
        ]) {
            *log += ratio.ln();
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "twenty-one clocks")]
    let n = table.len() as f64;
    let [before, after, net_before, net_after] = logs.map(|l| (l / n).exp());
    assert!(
        after <= before && net_after <= net_before,
        "fused {after:.3} and {net_after:.3}, closing {before:.3} and {net_before:.3}"
    );
}

/// **A flip-flop read by more tables than Vivado leaves on one is copied, and
/// `widemux` is faster for it** (`docs/fpga-vivado-parity.md`, "A wide fanout
/// copied"). Its select's two bits that each drive 256 tables are two
/// flip-flops each, as in Vivado's netlist: Vivado placing and routing both
/// netlists, the clock went from 2.556 ns (`fused`) to 2.495 (`copied`), seeds
/// 1 to 5, against Vivado's netlist's 2.453; in Reticle's own flow from 2.769
/// to 2.691, ten seeds. No other design has such a flip-flop, and every other
/// clock is the same netlist and the same figures. This holds `widemux` at
/// least 2 % faster at both levels, every other clock's seeds exactly
/// `fused`'s, and every clock with ten seeds of `copied` and five of its
/// netlist. Would catch a regeneration that lost the change, swapped the
/// builds or lost a seed; would not catch the flow losing it since.
#[test]
fn a_wide_fanout_copied_makes_widemux_faster() {
    let (Some(table), Some(nets)) = (achieved(), netlists()) else {
        return;
    };
    for ((design, clock), (_, builds)) in &table {
        let (fused, copied) = (&builds["fused"], &builds["copied"]);
        assert_eq!(copied.len(), 10, "{design} {clock}: seeds of `copied`");
        let (_, placed) = &nets[&(design.clone(), clock.clone())];
        let (net_fused, net_copied) = (&placed["fused"], &placed["copied"]);
        assert_eq!(
            net_copied.len(),
            5,
            "{design} {clock}: seeds of `pnr copied`"
        );
        if design == "widemux" {
            for (before, after, what) in [
                (median(fused), median(copied), "the flow"),
                (median(net_fused), median(net_copied), "the netlist"),
            ] {
                assert!(
                    after <= before * 0.98,
                    "widemux, {what}: {after:.3} ns copied against {before:.3}"
                );
            }
        } else {
            assert_eq!(fused, copied, "{design} {clock}: the flow");
            assert_eq!(net_fused, net_copied, "{design} {clock}: the netlist");
        }
    }
}
