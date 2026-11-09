//! When a global clock reaches a slice of the `xc7a100t` and of the
//! `xc7a35t`, against what Vivado said: `testdata/fpga/vivado/clock/arrival.txt`
//! and `arrival_xc7a35t.txt`, written by `tools/vivado-parity/clock_arrival.tcl`
//! and `clock_arrival.py fixture`, the clock net's delay to 700 flip-flops a
//! run over the die from global buffers at three sites, and on the `xc7a35t` a
//! fourth run in its other package. Vivado is not needed: the fixtures carry
//! its answers.

#![cfg(feature = "fpga")]

use std::path::PathBuf;

/// Each run's flip-flops of a fixture, as (run, site, Vivado's figure less
/// the table's), the run being `<part>/<buffer>` or, in the `xc7a100t`'s
/// fixture, which predates that, the buffer alone, of `default_part`. `None`
/// when the fixture is not there (the published crate).
fn differences(file: &str, default_part: &str) -> Option<Vec<(String, String, f64)>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/fpga/vivado/clock")
        .join(file);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipped: `{}` is not in the published crate",
            path.display()
        );
        return None;
    };
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["hclk", _] => {}
            ["arrival", run, site, x, y, ps] => {
                let part = run.split_once('/').map_or(default_part, |(part, _)| part);
                let (x, y): (u32, u32) = (x.parse().unwrap(), y.parse().unwrap());
                let ps: f64 = ps.parse().unwrap();
                let table = reticle::fpga::xray::clock_arrival_ps(part, x, y)
                    .unwrap_or_else(|| panic!("no table for `{part}`"));
                out.push(((*run).to_owned(), (*site).to_owned(), ps - table));
            }
            other => panic!("a line of {file} this does not know: {other:?}"),
        }
    }
    Some(out)
}

/// The median of `values`, the upper of the middle two.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Every flip-flop of `differences` within `ps` of the table, once the
/// table's zero, the earliest arrival, is set against Vivado's insertion
/// delay (the median difference).
fn within(differences: &[(String, String, f64)], ps: f64) {
    let insertion = median(differences.iter().map(|d| d.2).collect());
    for (run, site, difference) in differences {
        assert!(
            (difference - insertion).abs() <= ps,
            "{run} {site}: {:.1} ps from the table",
            difference - insertion
        );
    }
}

/// **Every flip-flop Vivado timed is within 6 ps of the table**, once the
/// table's zero, the earliest arrival, is set against Vivado's insertion delay
/// (the median difference). The table is the fixture's medians by clock region
/// row and column, so this holds that the growth along a column, the region
/// rows and the columns between those measured are what Vivado's are: a row
/// read from the wrong region row is off by 60 to 230 ps, a column by up to 80,
/// the growth left out by 19 at a region's edge. Would not catch the table
/// being applied to a part it was not measured on, which
/// `the_clock_reaches_a_site_when_the_table_says` (`xray::timing`) holds.
#[test]
fn every_flip_flop_vivado_timed_is_within_six_picoseconds_of_the_table() {
    let Some(differences) = differences("arrival.txt", "xc7a100tftg256-3") else {
        return;
    };
    let runs: std::collections::BTreeSet<&str> = differences.iter().map(|d| d.0.as_str()).collect();
    assert_eq!(runs.len(), 3, "three global buffers: {runs:?}");
    assert_eq!(differences.len(), 2100);
    within(&differences, 6.0);
}

/// **On the `xc7a35t` too, every flip-flop Vivado timed is within 7 ps of
/// its die's table, in both packages, and the package moves nothing.** Three
/// runs at `xc7a35tftg256-3`, which the table is the medians of, and one at
/// `xc7a35tcpg236-3` that it was not fitted to: the CPG236's insertion delay
/// is the FTG256's to 2 ps, so one table serves both. Would catch the
/// `xc7a100t`'s table used for the `xc7a35t` (876 of the 2,800 more than 7 ps
/// off it, up to 77), a row or column looked up wrong, the growth
/// left out, and a package whose clock reaches its slices otherwise. Would
/// not catch a speed grade other than 3: the grade-1 part is about 1.26 times
/// slower to every pin, which the table, like the rest of the fitted model,
/// does not scale.
#[test]
fn every_flip_flop_vivado_timed_on_the_xc7a35t_is_within_its_tables_reach() {
    let Some(differences) = differences("arrival_xc7a35t.txt", "") else {
        return;
    };
    let runs: std::collections::BTreeSet<&str> = differences.iter().map(|d| d.0.as_str()).collect();
    assert_eq!(
        runs.iter()
            .filter(|r| r.starts_with("xc7a35tftg256-3/"))
            .count(),
        3,
        "three global buffers on the FTG256: {runs:?}"
    );
    assert_eq!(
        runs.iter()
            .filter(|r| r.starts_with("xc7a35tcpg236-3/"))
            .count(),
        1,
        "one on the CPG236: {runs:?}"
    );
    assert_eq!(differences.len(), 2800);
    within(&differences, 7.0);
    let package = |prefix: &str| {
        median(
            differences
                .iter()
                .filter(|d| d.0.starts_with(prefix))
                .map(|d| d.2)
                .collect(),
        )
    };
    let (ftg, cpg) = (package("xc7a35tftg256"), package("xc7a35tcpg236"));
    assert!(
        (ftg - cpg).abs() <= 2.0,
        "the FTG256's insertion {ftg:.1} ps, the CPG236's {cpg:.1}"
    );
}
