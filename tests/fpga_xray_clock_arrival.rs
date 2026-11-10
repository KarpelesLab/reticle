//! When a global clock reaches a slice of the `xc7a100t` and of the
//! `xc7a35t`, against what Vivado said: `testdata/fpga/vivado/clock/arrival.txt`
//! and `arrival_xc7a35t.txt`, written by `tools/vivado-parity/clock_arrival.tcl`
//! and `clock_arrival.py fixture`, the clock net's delay to 700 flip-flops a
//! run over the die from global buffers at three sites, and on the `xc7a35t` a
//! fourth run in its other package; and `arrival_grades.txt` and
//! `arrival_xc7a35t_grades.txt`, the same at speed grades 1 and 2. Vivado is
//! not needed: the fixtures carry its answers.

#![cfg(feature = "fpga")]

use std::path::PathBuf;

use reticle::fpga::xray::SpeedGrade;

/// Each run's flip-flops of a fixture, as (run, site, Vivado's figure less
/// the table's), the run being `<part>/<buffer>` or, in the `xc7a100t`'s
/// fixture, which predates that, the buffer alone, of `default_part`; the
/// table the one of the part's die at the grade its name ends in, or at
/// `grade` when one is given. `None` when the fixture is not there (the
/// published crate).
fn differences(
    file: &str,
    default_part: &str,
    grade: Option<SpeedGrade>,
) -> Option<Vec<(String, String, f64)>> {
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
                let named = part
                    .rsplit_once('-')
                    .and_then(|(_, g)| SpeedGrade::from_name(g));
                let grade = grade
                    .or(named)
                    .unwrap_or_else(|| panic!("no grade in `{part}`"));
                let table = reticle::fpga::xray::clock_arrival_ps(part, grade, x, y)
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
    let Some(differences) = differences("arrival.txt", "xc7a100tftg256-3", None) else {
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
/// not catch a speed grade other than 3, which the next test is for. (This
/// said, until the tables of the other grades were measured, that "the
/// grade-1 part is about 1.26 times slower to every pin, which the table, like
/// the rest of the fitted model, does not scale"; it is a table of its own at
/// each grade now.)
#[test]
fn every_flip_flop_vivado_timed_on_the_xc7a35t_is_within_its_tables_reach() {
    let Some(differences) = differences("arrival_xc7a35t.txt", "", None) else {
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

/// **At speed grades 1 and 2, every flip-flop Vivado timed is within 7 ps of
/// its grade's table, on both dies and both of the `xc7a35t`'s packages; and
/// grade 3's table would not do.** Three runs from the same three buffers at
/// each of `xc7a100tftg256-1` and `-2`, and of `xc7a35tftg256-1` and `-2`, with a
/// CPG236 run at each grade on the `xc7a35t` that the tables were not fitted
/// to. Against grade 3's table, the same flip-flops are 21 to 57 ps off at
/// worst, by grade and die: the clock's spread over the die grows with the
/// grade, and not by one factor, the outer region rows by more than the inner.
///
/// Would catch a grade's table read for another grade, a grade's table
/// written into the wrong die's slot, the grade in a part's name taken over
/// the build's, or a table of one grade scaled for another; would not catch a
/// grade nobody measured, of which there is none among Reticle's devices.
#[test]
fn every_flip_flop_vivado_timed_at_the_other_grades_is_within_its_grades_table() {
    for file in ["arrival_grades.txt", "arrival_xc7a35t_grades.txt"] {
        let Some(measured) = differences(file, "", None) else {
            return;
        };
        for grade in ["-1/", "-2/"] {
            let runs: Vec<(String, String, f64)> = measured
                .iter()
                .filter(|d| d.0.contains(grade))
                .cloned()
                .collect();
            let parts: std::collections::BTreeSet<&str> =
                runs.iter().map(|d| d.0.as_str()).collect();
            assert!(parts.len() >= 3, "{file} {grade}: {parts:?}");
            for package in ["ftg256", "cpg236"] {
                let these: Vec<_> = runs
                    .iter()
                    .filter(|d| d.0.contains(package))
                    .cloned()
                    .collect();
                if !these.is_empty() {
                    within(&these, 7.0);
                }
            }
        }
        // Grade 3's table against the same flip-flops.
        let Some(as_three) = differences(file, "", Some(SpeedGrade::Three)) else {
            return;
        };
        for grade in ["-1/", "-2/"] {
            let runs: Vec<(String, String, f64)> = as_three
                .iter()
                .filter(|d| d.0.contains(grade) && d.0.contains("ftg256"))
                .cloned()
                .collect();
            let insertion = median(runs.iter().map(|d| d.2).collect());
            let worst = runs
                .iter()
                .map(|d| (d.2 - insertion).abs())
                .fold(0.0f64, f64::max);
            assert!(
                worst > 14.0,
                "{file} {grade}: grade 3's table within {worst:.1} ps"
            );
        }
    }
}

/// `pessimism.txt` by (part, way the two ends relate): see [`pessimism`].
type Pessimism = std::collections::BTreeMap<(String, String), (u32, f64, f64)>;

/// What `pessimism.txt` says of each part and way a path's two ends relate:
/// (paths, ps kept at the capture, the decomposition's check in ps); and of
/// each part's clock uncertainty, as the way `uncertainty`, (0, its ps, 0).
/// `None` when the fixture is not there (the published crate).
fn pessimism() -> Option<Pessimism> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/clock/pessimism.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipped: `{}` is not in the published crate",
            path.display()
        );
        return None;
    };
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<&str> = line.split_whitespace().collect();
        if let ["uncertainty", part, ps] = w.as_slice() {
            out.insert(
                ((*part).to_owned(), "uncertainty".to_owned()),
                (0, ps.parse().unwrap(), 0.0),
            );
            continue;
        }
        let [
            "pessimism",
            part,
            kind,
            "paths",
            paths,
            "kept",
            kept,
            "sd",
            _,
            "credit",
            _,
            "late_less_early",
            _,
            "late_skew",
            _,
            "check",
            check,
        ] = w.as_slice()
        else {
            panic!("pessimism.txt: `{line}`");
        };
        out.insert(
            ((*part).to_owned(), (*kind).to_owned()),
            (
                paths.parse().unwrap(),
                kept.parse().unwrap(),
                check.parse().unwrap(),
            ),
        );
    }
    Some(out)
}

/// **A path between two clock regions is charged what Vivado's timer keeps
/// of the clock's pessimism there over what it keeps inside one region, at
/// each grade, one figure for both dies.** `pessimism.txt`
/// (`tools/vivado-parity/clock_pessimism.py`): over each clock's 400 worst
/// setup paths of Reticle's routes of the QoR corpus at `xc7a100tftg256` and
/// `xc7a35tftg256`, every path's skew is its capture's early clock delay less
/// its launch's late one plus the pessimism removal's credit on the clock net,
/// to a picosecond; a path within one site keeps nothing; and what the capture
/// keeps in the other half of its region row, and in another row, less what it
/// keeps within one region, is `xray::clock_margins`' second figure, the two
/// dies' mean within a picosecond and each die within four; and what every
/// capture is charged, its first figure, is the uncertainty and what the
/// capture keeps within one region, the dies' mean within a picosecond. The
/// legacy model charges nothing.
///
/// Would catch a grade's figures given another grade (or the old 0.062 ns at
/// every grade, 6 to 9 ps under), the two figures swapped, either taken as
/// the whole of what the capture keeps rather than
/// what it keeps over a region's own, or one die's figure where the two differ
/// by more than four picoseconds. Would not catch the routes or the probe
/// being wrong, which only Vivado's timer asks, nor whether the analysis
/// charges the figure to the right paths, which `place`'s
/// `a_path_between_clock_regions_keeps_the_clocks_pessimism` holds.
#[test]
fn a_path_between_regions_is_charged_the_pessimism_vivado_keeps() {
    use reticle::fpga::xray::{DelayModel, clock_margins};
    let Some(fixture) = pessimism() else {
        return;
    };
    for ((_, kind), (paths, _, check)) in &fixture {
        if kind == "uncertainty" {
            continue;
        }
        assert!(*paths > 50, "{fixture:?}");
        assert!(
            *check <= 1.0,
            "the skew is the decomposition's: {fixture:?}"
        );
    }
    for grade in [SpeedGrade::One, SpeedGrade::Two, SpeedGrade::Three] {
        let (margin, (half, row)) = clock_margins(DelayModel::Vivado, grade);
        let mut over = Vec::new();
        let mut inside_mean = 0.0;
        for die in ["xc7a100tftg256", "xc7a35tftg256"] {
            let part = format!("{die}-{}", grade.name());
            let kept = |kind: &str| fixture[&(part.clone(), kind.to_owned())].1;
            assert!(kept("same_site").abs() <= 1.0, "{part}");
            let inside = kept("one_region");
            inside_mean += (kept("uncertainty") + inside) / 2.0;
            let these = (kept("other_half") - inside, kept("other_row") - inside);
            assert!(
                (these.0 - half * 1000.0).abs() <= 4.0 && (these.1 - row * 1000.0).abs() <= 4.0,
                "{part}: {these:?} against {half} and {row}"
            );
            over.push(these);
        }
        let mean = ((over[0].0 + over[1].0) / 2.0, (over[0].1 + over[1].1) / 2.0);
        assert!(
            (mean.0 - half * 1000.0).abs() <= 1.0 && (mean.1 - row * 1000.0).abs() <= 1.0,
            "grade {}: {mean:?} against {half} and {row}",
            grade.name()
        );
        assert!(
            (inside_mean - margin * 1000.0).abs() <= 1.0,
            "grade {}: the margin {margin} against {inside_mean:.1} ps",
            grade.name()
        );
        assert_eq!(clock_margins(DelayModel::Legacy, grade), (0.0, (0.0, 0.0)));
    }
}
