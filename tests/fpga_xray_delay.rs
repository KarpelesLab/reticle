//! The 7-series delay model fitted to Vivado's timer, held to Vivado's own
//! numbers: `testdata/fpga/vivado/timing/`, written by
//! `tools/vivado-parity/timing_table.py` from Vivado 2026.1's timer on
//! Reticle's own routes (`run_placed.sh` with `TIMING=1`). Vivado is not
//! needed, nor the database: the fixtures carry what the tests read.

#![cfg(feature = "fpga")]

use std::path::PathBuf;

use reticle::fpga::xray::{SpeedGrade, delay};

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

/// **The table in the source is the table the fit wrote, at each grade.**
/// Every class of `classes.txt` (grade 3), `classes_2.txt` and `classes_1.txt`
/// with its cost rounded to the picosecond in that grade's column, the branch
/// charge and the share of prjxray's delay a wire of no class is charged.
/// Would catch a fit regenerated without the source following it, an entry
/// edited by hand, or two grades' columns swapped; would not catch a fit that
/// is itself wrong, which the next test is for.
#[test]
fn the_source_holds_the_table_the_fit_wrote() {
    for (file, grade) in [
        ("classes.txt", SpeedGrade::Three),
        ("classes_2.txt", SpeedGrade::Two),
        ("classes_1.txt", SpeedGrade::One),
    ] {
        holds_the_fit(file, grade);
    }
}

fn holds_the_fit(file: &str, grade: SpeedGrade) {
    let Some(text) = fixture(file) else {
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
                    (ps - f64::from(delay::branch_ps(grade))).abs() <= 0.5,
                    "{file}: branch {ps}"
                );
            }
            ["other", share] => {
                let share: f64 = share.parse().unwrap();
                let ours = f64::from(delay::OTHER_PER_MILLE[grade.index()]) / 1000.0;
                assert!((share - ours).abs() <= 0.0005, "{file}: other {share}");
            }
            _ => {}
        }
    }
    // The fit's tenths rounded to the picosecond, a half either way.
    let names: Vec<&str> = classes.iter().map(|(n, _)| n.as_str()).collect();
    let ours: Vec<&str> = delay::CLASSES.iter().map(|(n, _)| *n).collect();
    assert_eq!(ours, names);
    for ((name, ps), (_, fitted)) in delay::CLASSES.iter().zip(&classes) {
        let ps = ps[grade.index()];
        assert!(
            (f64::from(ps) - fitted).abs() <= 0.5,
            "{file} {name}: {ps} against {fitted}"
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
        let (mut ours, mut theirs) = (branches * delay::branch_ps(SpeedGrade::Three), 0u32);
        for pip in &words[3..] {
            let parts: Vec<&str> = pip.split('.').collect();
            let [tile_type, to, ps] = parts.as_slice() else {
                panic!("`{pip}` is not <tile type>.<wire>.<ps or ->");
            };
            let ps: Option<u32> = ps.parse().ok();
            ours += delay::wire_ps(tile_type, to, ps, SpeedGrade::Three);
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

/// What the model costs a connection of `sample.txt` at `grade`: its branches
/// and the wire of every pip, as `xray::delay` costs a route.
fn costed(line: &str, grade: SpeedGrade) -> u32 {
    let words: Vec<&str> = line.split_whitespace().collect();
    let branches: u32 = words[2].parse().unwrap();
    let mut ours = branches * delay::branch_ps(grade);
    for pip in &words[3..] {
        let parts: Vec<&str> = pip.split('.').collect();
        let [tile_type, to, ps] = parts.as_slice() else {
            panic!("`{pip}` is not <tile type>.<wire>.<ps or ->");
        };
        ours += delay::wire_ps(tile_type, to, ps.parse().ok(), grade);
    }
    ours
}

/// **At speed grades 1 and 2 too, a connection the model times comes within a
/// few percent of Vivado's figure for it at that grade; and grade 3's figures
/// do not.** `sample_grades.txt` is `sample.txt`'s connections, the same
/// routes, with Vivado's interconnect delay to each at `xc7a100tftg256-1`, `-2`
/// and `-3`. Each grade's column of the table puts the median within 3 % of
/// Vivado's at that grade and the 5th and 95th percentiles inside 0.9 and 1.1;
/// grade 3's column against grade 1's figures is a quarter under at the median,
/// which is what every build of a grade-1 part was timed with before.
///
/// Would catch a grade's column holding another grade's fit, the branch charge
/// or the share of prjxray's figure taken from the wrong grade, and the grade
/// not reaching the lookup at all; would not catch a grade's fit that is
/// itself wrong in the same way as Vivado's sample, which only Vivado's timer
/// on new routes says (`docs/fpga-vivado-parity.md`, "Speed grades").
#[test]
fn at_each_grade_a_connection_is_within_a_few_percent_of_vivados_figure() {
    let (Some(sample), Some(grades)) = (fixture("sample.txt"), fixture("sample_grades.txt")) else {
        return;
    };
    let sample: Vec<&str> = sample.lines().filter(|l| !l.starts_with('#')).collect();
    let grades: Vec<&str> = grades.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(
        sample.len(),
        grades.len(),
        "one line of figures a connection"
    );
    let mut ratios = [Vec::new(), Vec::new(), Vec::new()];
    let mut three_at_one = Vec::new();
    for (line, figures) in sample.iter().zip(&grades) {
        let words: Vec<&str> = figures.split_whitespace().collect();
        let [design, one, two, three] = words.as_slice() else {
            panic!("`{figures}` is not <design> <ps> <ps> <ps>");
        };
        // The same connection: its design and its grade-3 figure are sample.txt's.
        let mine: Vec<&str> = line.split_whitespace().take(2).collect();
        assert_eq!(mine, [*design, *three], "out of step at `{line}`");
        let vivado: [f64; 3] = [one, two, three].map(|w| w.parse().unwrap());
        if vivado[2] < 20.0 {
            continue;
        }
        for (grade, ratio) in [SpeedGrade::One, SpeedGrade::Two, SpeedGrade::Three]
            .into_iter()
            .zip(&mut ratios)
        {
            ratio.push(f64::from(costed(line, grade)) / vivado[grade.index()]);
        }
        three_at_one.push(f64::from(costed(line, SpeedGrade::Three)) / vivado[0]);
    }
    for (grade, ratio) in ratios.iter().enumerate() {
        assert!(ratio.len() > 900, "{} connections", ratio.len());
        let (median, low, high) = (
            quantile(ratio, 0.5),
            quantile(ratio, 0.05),
            quantile(ratio, 0.95),
        );
        assert!(
            (0.97..=1.03).contains(&median),
            "grade {}: median {median}",
            grade + 1
        );
        assert!(
            low > 0.9 && high < 1.1,
            "grade {}: 5th {low}, 95th {high}",
            grade + 1
        );
    }
    let median = quantile(&three_at_one, 0.5);
    assert!(
        median < 0.8,
        "grade 3's figures against grade 1's: median {median}"
    );
}

/// Each `design` line of `summary.txt` for `model`, with its lines under it.
fn summary(model: &str) -> Option<Vec<(String, Vec<String>)>> {
    summary_in("summary.txt", model)
}

/// Each `design` line of the fixture `file` for `model`, with its lines under
/// it.
fn summary_in(file: &str, model: &str) -> Option<Vec<(String, Vec<String>)>> {
    let text = fixture(file)?;
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

/// **At each speed grade the build was made for, Reticle's estimate is
/// Vivado's at that grade, connection by connection and at each clock's worst
/// endpoint; and a grade-1 part timed with grade 3's figures was a quarter
/// under.** `summary_grades.txt` is `timing_table.py compare` of four builds of
/// the corpus on the XC7A35T: at the device's grade 1, at `--speed-grade 2`
/// and `3`, each timed by Vivado at that grade, and the tree before grades
/// timed at grade 1. At each grade every design's median connection is within
/// 3 % of Vivado's and every clock's worst endpoint within 15 %; before, every
/// design's median connection was under 0.8 and every clock's worst endpoint
/// under 0.8 too.
///
/// Reads the quoted run: it would catch a regeneration that lost the
/// agreement at a grade, and not a change to the flow since, which only
/// `run_placed.sh` asks again.
#[test]
fn at_each_grade_every_design_agrees_with_vivado_at_that_grade() {
    let worst_ratios = |lines: &[String]| -> Vec<f64> {
        lines
            .iter()
            .filter(|l| l.starts_with("clock "))
            .map(|line| {
                let words: Vec<&str> = line.split_whitespace().collect();
                let field = |key: &str| -> f64 {
                    let at = words.iter().position(|w| *w == key).unwrap();
                    words[at + 1].parse().unwrap()
                };
                field("reticle_there") / field("vivado")
            })
            .collect()
    };
    for model in ["grade1", "grade2", "grade3"] {
        let Some(designs) = summary_in("summary_grades.txt", model) else {
            return;
        };
        assert!(designs.len() >= 17, "{model}: {} designs", designs.len());
        for (name, lines) in &designs {
            let [median, ..] = spread(lines, "connections").unwrap();
            assert!(
                (0.97..=1.03).contains(&median),
                "{model} {name}: median {median}"
            );
            for ratio in worst_ratios(lines) {
                assert!(
                    (0.85..=1.15).contains(&ratio),
                    "{model} {name}: worst {ratio:.3}"
                );
            }
        }
    }
    let Some(before) = summary_in("summary_grades.txt", "before") else {
        return;
    };
    assert!(before.len() >= 17, "{} designs", before.len());
    for (name, lines) in &before {
        let [median, ..] = spread(lines, "connections").unwrap();
        assert!(median < 0.8, "before {name}: median {median}");
        for ratio in worst_ratios(lines) {
            assert!(ratio < 0.8, "before {name}: worst {ratio:.3}");
        }
    }
}

/// What `placement.txt`'s fit (`placement_fit.py fit`) predicts for one of its
/// sampled connections: `c <dx> <dy> <fanout> <driver> <sink> <inputs used>`.
fn placement_fit_predicts(terms: &std::collections::BTreeMap<String, f64>, words: &[&str]) -> f64 {
    let term = |name: &str| terms.get(name).copied().unwrap_or(0.0);
    let mut ns = term("base");
    for (axis, word) in [("dx", words[0]), ("dy", words[1])] {
        let n: i64 = word.parse::<i64>().unwrap().abs();
        for k in 1..=n.min(8) {
            ns += term(&format!("{axis}>={k}"));
        }
        #[allow(clippy::cast_precision_loss, reason = "a few hundred tiles")]
        let over = |at: i64| (n - at).max(0) as f64;
        ns += term(&format!("{axis}-8")) * over(8) + term(&format!("{axis}-16")) * over(16);
    }
    let fanout: usize = words[2].parse().unwrap();
    let band = [
        (4, "2-4"),
        (8, "5-8"),
        (16, "9-16"),
        (32, "17-32"),
        (64, "33-64"),
    ]
    .iter()
    .find(|(most, _)| fanout <= *most)
    .map_or("65-", |(_, name)| name);
    if fanout > 1 {
        ns += term(&format!("fanout_{band}"));
    }
    let sink = if words[4] == "lut.i" {
        format!("lut.i{}", words[5].parse::<usize>().unwrap().min(6))
    } else {
        words[4].to_owned()
    };
    ns + term(&format!("driver_{}", words[3])) + term(&format!("sink_{sink}"))
}

/// **The placement's refitted estimate predicts the routed delay where its
/// line did not** (`placement.txt`; `docs/fpga-vivado-parity.md`, "The
/// placement's estimate, refitted"). Over the sample of the corpus's routed
/// connections, the fit's terms put a connection within 5 % of its routed
/// delay at the median and under 1.9 times it at the 95th percentile, where
/// the line (`xray::timing::VIVADO_WIRE`, the placement's estimate in every
/// build) is over 1.06 and 2.0; at twenty tiles and more within 10 % where the
/// line is a fifth over; and with a root mean square error under two thirds of
/// the line's.
///
/// What the document claims of the fit, held to the numbers it was read
/// from. Would catch the fit's file regenerated with a fit that lost that, or
/// a sample that no longer shows the line's overestimate; would not catch
/// anything the flow does since, as the fit is not the placement's estimate
/// (the section says why) and this reads a quoted run.
#[test]
fn the_placements_refit_predicts_the_routed_delay_better_than_its_line() {
    let Some(text) = fixture("placement.txt") else {
        return;
    };
    let terms: std::collections::BTreeMap<String, f64> = text
        .lines()
        .filter_map(|l| l.strip_prefix("fit "))
        .map(|l| {
            let (term, ns) = l.split_once(' ').unwrap();
            (term.to_owned(), ns.parse().unwrap())
        })
        .collect();
    assert!(terms.len() > 60, "{} terms", terms.len());
    let (mut fit, mut line, mut far, mut far_line) = (vec![], vec![], vec![], vec![]);
    let (mut err, mut line_err) = (0.0f64, 0.0f64);
    for l in text.lines().filter_map(|l| l.strip_prefix("c ")) {
        let w: Vec<&str> = l.split(' ').collect();
        let (routed, was): (f64, f64) = (w[6].parse().unwrap(), w[7].parse().unwrap());
        let ns = placement_fit_predicts(&terms, &w);
        fit.push(ns / routed);
        line.push(was / routed);
        err += (ns - routed).powi(2);
        line_err += (was - routed).powi(2);
        let tiles = w[0].parse::<i64>().unwrap().abs() + w[1].parse::<i64>().unwrap().abs();
        if tiles >= 20 {
            far.push(ns / routed);
            far_line.push(was / routed);
        }
    }
    assert!(fit.len() > 5000, "{} connections", fit.len());
    let (median, p95) = (quantile(&fit, 0.5), quantile(&fit, 0.95));
    assert!((0.95..1.05).contains(&median), "fit median {median}");
    assert!(p95 < 1.9, "fit 95th {p95}");
    let (line_median, line_p95) = (quantile(&line, 0.5), quantile(&line, 0.95));
    assert!(
        line_median > 1.06 && line_p95 > 2.0,
        "line {line_median} {line_p95}"
    );
    let (far_median, far_line_median) = (quantile(&far, 0.5), quantile(&far_line, 0.5));
    assert!((0.9..1.1).contains(&far_median), "fit far {far_median}");
    assert!(far_line_median > 1.2, "line far {far_line_median}");
    assert!(err < line_err * 2.0 / 3.0, "{err} {line_err}");
}
