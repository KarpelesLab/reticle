//! The 7-series delay model fitted to Vivado's timer, held to Vivado's own
//! numbers: `testdata/fpga/vivado/timing/`, written by
//! `tools/vivado-parity/timing_table.py` from Vivado 2026.1's timer on
//! Reticle's own routes (`run_placed.sh` with `TIMING=1`). Vivado is not
//! needed, nor the database: the fixtures carry what the tests read.

#![cfg(feature = "fpga")]

use std::path::PathBuf;

use reticle::fpga::xray::delay;

fn fixture(name: &str) -> Option<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/fpga/vivado/timing")
        .join(name);
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(_) => {
            eprintln!(
                "skipped: `{}` is not in the published crate",
                path.display()
            );
            None
        }
    }
}

/// The `q`th quantile of `values`, interpolated, as `timing_table.py` takes it.
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let (lo, frac) = {
        let k = (v.len() - 1) as f64 * q;
        (k.floor() as usize, k - k.floor())
    };
    let hi = (lo + 1).min(v.len() - 1);
    v[lo] + (v[hi] - v[lo]) * frac
}

/// **The table in the source is the table the fit wrote.** Every class of
/// `classes.txt` with its cost rounded to the picosecond, the branch charge and
/// the share of prjxray's delay a wire of no class is charged. Would catch a
/// fit regenerated without the source following it, or an entry edited by hand;
/// would not catch a fit that is itself wrong, which the next test is for.
#[test]
fn the_source_holds_the_table_the_fit_wrote() {
    let Some(text) = fixture("classes.txt") else {
        return;
    };
    let mut classes = Vec::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["class", name, _, ps] => {
                classes.push(((*name).to_owned(), ps.parse::<f64>().unwrap()));
            }
            ["branch", ps] => {
                let ps: f64 = ps.parse().unwrap();
                assert!(
                    (ps - f64::from(delay::BRANCH_PS)).abs() <= 0.5,
                    "branch {ps}"
                );
            }
            ["other", share] => {
                let share: f64 = share.parse().unwrap();
                let ours = f64::from(delay::OTHER_PER_MILLE) / 1000.0;
                assert!((share - ours).abs() <= 0.0005, "other {share}");
            }
            _ => {}
        }
    }
    // The fit's tenths rounded to the picosecond, a half either way.
    let names: Vec<&str> = classes.iter().map(|(n, _)| n.as_str()).collect();
    let ours: Vec<&str> = delay::CLASSES.iter().map(|(n, _)| *n).collect();
    assert_eq!(ours, names);
    for ((name, ps), (_, fitted)) in delay::CLASSES.iter().zip(&classes) {
        assert!(
            (f64::from(*ps) - fitted).abs() <= 0.5,
            "{name}: {ps} against {fitted}"
        );
    }
}

/// **A connection the model times comes within a few percent of Vivado's
/// figure for it.** Over the sample of `sample.txt`, one connection in
/// twenty-seven of the 27,000 the table was fitted to (the 979 of them Vivado
/// puts at 20 ps or more), each costed as `xray::delay` costs a route: the
/// class of every wire it enters, prjxray's delay times
/// [`delay::OTHER_PER_MILLE`] for a wire of no class, and [`delay::BRANCH_PS`]
/// a branch. The median of the model over Vivado is within 3 % of one, the 5th
/// percentile above 0.9 and the 95th below 1.1. prjxray's pip delays alone,
/// what the legacy model is made of, are far wider on the same connections, and
/// that is held too, so that the comparison is known to see a model that is
/// wrong.
///
/// Would catch the table out of step with its classes, a class that lost a
/// side or a number in [`delay::wire_class`], the branch charge dropped, or
/// the fallback misapplied; would not catch a route Reticle walks differently
/// from Vivado's, which `summary.txt` and the database's own test of
/// `node_delays` are for.
#[test]
fn a_connection_is_within_a_few_percent_of_vivados_figure() {
    let Some(text) = fixture("sample.txt") else {
        return;
    };
    let (mut fitted, mut prjxray) = (Vec::new(), Vec::new());
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let vivado: f64 = words[1].parse().unwrap();
        let branches: u32 = words[2].parse().unwrap();
        if vivado < 20.0 {
            continue;
        }
        let (mut ours, mut theirs) = (branches * delay::BRANCH_PS, 0u32);
        for pip in &words[3..] {
            let parts: Vec<&str> = pip.split('.').collect();
            let [tile_type, to, ps] = parts.as_slice() else {
                panic!("`{pip}` is not <tile type>.<wire>.<ps or ->");
            };
            let ps: Option<u32> = ps.parse().ok();
            ours += delay::wire_ps(tile_type, to, ps);
            theirs += ps.unwrap_or(0);
        }
        fitted.push(f64::from(ours) / vivado);
        prjxray.push(f64::from(theirs) / vivado);
    }
    assert!(fitted.len() > 900, "{} connections", fitted.len());
    let (median, low, high) = (
        quantile(&fitted, 0.5),
        quantile(&fitted, 0.05),
        quantile(&fitted, 0.95),
    );
    assert!((0.97..=1.03).contains(&median), "median {median}");
    assert!(low > 0.9 && high < 1.1, "5th {low}, 95th {high}");
    let (low, high) = (quantile(&prjxray, 0.05), quantile(&prjxray, 0.95));
    assert!(low < 0.5 && high > 1.3, "prjxray's 5th {low}, 95th {high}");
}

/// Each `design` line of `summary.txt` for `model`, with its lines under it.
fn summary(model: &str) -> Option<Vec<(String, Vec<String>)>> {
    let text = fixture("summary.txt")?;
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut keep = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("design ") {
            let (name, of) = rest.split_once(' ').unwrap();
            keep = of == model;
            if keep {
                out.push((name.to_owned(), Vec::new()));
            }
        } else if keep && line.starts_with("  ") {
            out.last_mut().unwrap().1.push(line.trim().to_owned());
        } else {
            keep = false;
        }
    }
    Some(out)
}

/// The five numbers of a `connections` or `endpoints` line: median, 5th and
/// 95th percentile, lowest and highest.
fn spread(lines: &[String], key: &str) -> Option<[f64; 5]> {
    let words: Vec<f64> = lines
        .iter()
        .find(|l| l.starts_with(key))?
        .split_whitespace()
        .skip(2)
        .map(|w| w.parse().unwrap())
        .collect();
    words.try_into().ok()
}

/// **Reticle's own estimate of each connection of its own routes is Vivado's,
/// within a few percent, on every design of the corpus; and the legacy model's
/// was not.** `summary.txt` is `timing_table.py compare` of two runs of the
/// corpus through Vivado's timer, one built with `--delay-model legacy` and
/// one with the default, each connection matched by the wire its route reached
/// the sink on. Under the fitted model every design's median is within 3 % of
/// Vivado's and its 5th percentile above 0.85; under the legacy model every
/// design's 5th percentile is under 0.85, and 13 of the 17 under 0.5. Reads
/// the quoted file: it
/// would catch a regeneration that lost the agreement, and not a change to the
/// flow since, which only `run_placed.sh` asks again.
#[test]
fn every_design_agrees_with_vivado_a_connection_at_a_time() {
    let (Some(fitted), Some(legacy)) = (summary("vivado"), summary("legacy")) else {
        return;
    };
    assert!(fitted.len() >= 17, "{} designs", fitted.len());
    for (name, lines) in &fitted {
        let [median, low, ..] = spread(lines, "connections").unwrap();
        assert!((0.97..=1.03).contains(&median), "{name}: median {median}");
        assert!(low > 0.85, "{name}: 5th percentile {low}");
    }
    let lows: Vec<f64> = legacy
        .iter()
        .map(|(_, lines)| spread(lines, "connections").unwrap()[1])
        .collect();
    assert!(lows.iter().all(|low| *low < 0.85), "{lows:?}");
    assert!(
        lows.iter().filter(|low| **low < 0.5).count() >= 13,
        "{lows:?}"
    );
}

/// **The estimate of the path Vivado says is a clock's worst is near Vivado's,
/// within 1.35 either way, on every clock of every design; and a typical
/// endpoint's within 10 %.** From `summary.txt`'s fitted run: Vivado's data
/// path, setup and clock skew to the endpoint it puts last in each clock,
/// against Reticle's arrival at that endpoint with its capture. Reads the
/// quoted file, as the test above does.
#[test]
fn the_worst_endpoint_of_each_clock_is_near_vivados() {
    let Some(fitted) = summary("vivado") else {
        return;
    };
    let mut wrong = Vec::new();
    for (name, lines) in &fitted {
        let [median, ..] = spread(lines, "endpoints").unwrap();
        if !(0.9..=1.1).contains(&median) {
            wrong.push(format!("{name}: endpoints' median {median}"));
        }
        for line in lines.iter().filter(|l| l.starts_with("clock ")) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let field = |key: &str| -> f64 {
                let at = words.iter().position(|w| *w == key).unwrap();
                words[at + 1].parse().unwrap()
            };
            let ratio = field("reticle_there") / field("vivado");
            if !(1.0 / 1.35..=1.35).contains(&ratio) {
                wrong.push(format!("{name} {}: {ratio:.3}", words[1]));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join(", "));
}
