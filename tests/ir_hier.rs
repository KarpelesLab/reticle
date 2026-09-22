//! Golden tests for the hierarchy passes.
//!
//! Every `testdata/ir/hier/<name>.rtl` is parsed, validated, and then run
//! through [`reticle::ir::Design::flatten`] and
//! [`reticle::ir::Design::uniquify`] with the default options. The results
//! must match `<name>.flat.rtl` and `<name>.uniq.rtl`, print back
//! identically and validate without errors; a design that cannot be
//! flattened must produce the diagnostics in `<name>.diag` and leave the
//! design untouched. Set `UPDATE_EXPECT=1` to rewrite the expectations
//! after an intended change.

use std::fs;
use std::path::{Path, PathBuf};

use reticle::ir::hier::FlattenOptions;
use reticle::ir::validate::validate;
use reticle::ir::{Design, Id, ModuleId};
use reticle::source::SourceMap;

/// Every input of the corpus, sorted, without the generated files.
fn inputs() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/ir/hier");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.ends_with(".rtl") && !name.ends_with(".flat.rtl") && !name.ends_with(".uniq.rtl")
        })
        .collect();
    paths.sort();
    paths
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

/// Compares `actual` with the file at `path`, rewriting it under
/// `UPDATE_EXPECT`. An absent file counts as empty, and empty output
/// removes the file.
fn check(path: &Path, actual: &str, update: bool, failures: &mut Vec<String>) {
    let expected = fs::read_to_string(path).unwrap_or_default();
    if expected == actual {
        return;
    }
    if update {
        if actual.is_empty() {
            let _ = fs::remove_file(path);
        } else {
            fs::write(path, actual).unwrap();
        }
    } else {
        failures.push(format!(
            "{}: mismatch (set UPDATE_EXPECT=1 to rewrite)\n{}",
            path.file_name().unwrap().to_string_lossy(),
            first_difference(&expected, actual)
        ));
    }
}

/// Parses a design, failing the test rather than panicking.
fn parse(name: &str, text: &str, failures: &mut Vec<String>) -> Option<(Design, SourceMap)> {
    let mut map = SourceMap::new();
    let file = map.add(name.to_owned(), text.to_owned()).unwrap();
    match Design::parse_text(text, file) {
        Ok(design) => Some((design, map)),
        Err(diags) => {
            failures.push(format!("{name}: parse failed\n{}", diags.render(&map)));
            None
        }
    }
}

/// Checks that a design validates and survives a print / parse round trip.
fn check_result(
    name: &str,
    design: &Design,
    map: &SourceMap,
    failures: &mut Vec<String>,
) -> String {
    let diags = validate(design);
    if diags.has_errors() {
        failures.push(format!("{name}: result invalid\n{}", diags.render(map)));
    }
    let printed = design.to_text();
    let mut map2 = SourceMap::new();
    let file = map2.add(name.to_owned(), printed.clone()).unwrap();
    match Design::parse_text(&printed, file) {
        Ok(again) => {
            if again.to_text() != printed {
                failures.push(format!("{name}: result is not a print fixed point"));
            }
        }
        Err(diags) => failures.push(format!(
            "{name}: result does not parse\n{}",
            diags.render(&map2)
        )),
    }
    printed
}

#[test]
fn golden_hierarchy_passes() {
    let paths = inputs();
    assert!(!paths.is_empty(), "no golden files found");
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let mut failures = Vec::new();

    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".rtl").to_owned();
        let text = fs::read_to_string(&path).unwrap();
        let Some((design, map)) = parse(&name, &text, &mut failures) else {
            continue;
        };
        let diags = validate(&design);
        if diags.has_errors() {
            failures.push(format!("{name}: input invalid\n{}", diags.render(&map)));
            continue;
        }
        // The inputs keep their comments, so only the results are compared
        // with a canonical rendering.
        let canonical = design.to_text();
        let top = design.top.expect("the corpus declares a top module");

        // Flattening, or the diagnostics explaining why it is impossible.
        let mut flat = design.clone();
        let (flat_text, diag_text) = match flat.flatten(top, &FlattenOptions::default()) {
            Ok(report) => {
                if report.inlined_total + report.kept == 0 {
                    failures.push(format!("{name}: nothing to flatten"));
                }
                (
                    check_result(&name, &flat, &map, &mut failures),
                    String::new(),
                )
            }
            Err(mut diags) => {
                diags.sort();
                if flat.to_text() != canonical {
                    failures.push(format!("{name}: a failed flatten changed the design"));
                }
                (String::new(), diags.render(&map))
            }
        };
        check(
            &path.with_file_name(format!("{stem}.flat.rtl")),
            &flat_text,
            update,
            &mut failures,
        );
        check(
            &path.with_file_name(format!("{stem}.diag")),
            &diag_text,
            update,
            &mut failures,
        );

        // Unique-ification, and flattening on top of it.
        let mut uniq = design.clone();
        uniq.uniquify();
        let uniq_text = check_result(&name, &uniq, &map, &mut failures);
        check(
            &path.with_file_name(format!("{stem}.uniq.rtl")),
            &uniq_text,
            update,
            &mut failures,
        );
        let mut both = uniq.clone();
        if both.flatten(top, &FlattenOptions::default()).is_ok() {
            check_result(&name, &both, &map, &mut failures);
        }

        // `dedup` undoes `uniquify`.
        uniq.dedup();
        if uniq.to_text() != canonical {
            failures.push(format!(
                "{name}: dedup did not undo uniquify\n{}",
                first_difference(&canonical, &uniq.to_text())
            ));
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Loads one corpus file.
fn load(name: &str) -> Design {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/ir/hier")
        .join(name);
    let text = fs::read_to_string(&path).unwrap();
    let mut map = SourceMap::new();
    let file = map.add(name.to_owned(), text.clone()).unwrap();
    match Design::parse_text(&text, file) {
        Ok(design) => design,
        Err(diags) => panic!("{}", diags.render(&map)),
    }
}

#[test]
fn hierarchy_queries_on_the_corpus() {
    let design = load("three_level.rtl");
    let top = design.top.unwrap();
    let paths: Vec<String> = design.hier_paths(top).into_iter().map(|(p, _)| p).collect();
    assert_eq!(paths, ["m0", "m0.u_leaf", "m1", "m1.u_leaf"]);
    assert_eq!(design.instance_count(top), 4);

    let (instances, net) = design.resolve_path(top, "m1.u_leaf.b").unwrap();
    assert_eq!(instances.len(), 2);
    let leaf = design.module_by_name("leaf").unwrap();
    assert_eq!(design.module(leaf).nets[net].name, "b");
    assert!(design.resolve_path(top, "m1.u_leaf.nope").is_none());

    // Unresolved instances have no module and are left out of the paths.
    let bb = load("blackbox.rtl");
    let bb_top = bb.top.unwrap();
    let paths: Vec<String> = bb.hier_paths(bb_top).into_iter().map(|(p, _)| p).collect();
    assert_eq!(paths, ["u_wrap", "u_wrap.u_bb"]);
    assert_eq!(bb.instance_count(bb_top), 3);
}

#[test]
fn flattening_removes_the_modules_it_inlined() {
    let mut design = load("three_level.rtl");
    let top = design.top.unwrap();
    let report = design.flatten(top, &FlattenOptions::default()).unwrap();
    assert_eq!(report.inlined_total, 4);
    assert_eq!(report.inlined_count("mid"), 2);
    assert_eq!(report.inlined_count("leaf"), 2);
    assert_eq!(report.depth, 2);
    assert_eq!(report.processes, 2);
    // `leaf` and `mid` are now unreachable.
    assert_eq!(design.remove_unused_modules(top), 2);
    assert_eq!(design.modules.len(), 1);
    assert!(!validate(&design).has_errors());
}

#[test]
fn keep_hierarchy_survives_and_max_depth_stops_early() {
    let mut design = load("keep_hier.rtl");
    let top = design.top.unwrap();
    let report = design.flatten(top, &FlattenOptions::default()).unwrap();
    assert_eq!(report.kept, 1);
    assert_eq!(report.inlined_total, 1);
    assert_eq!(
        design.module(top).instances.values().next().unwrap().name,
        "u_keep"
    );

    let mut design = load("three_level.rtl");
    let top = design.top.unwrap();
    let nothing = FlattenOptions {
        max_depth: Some(0),
        ..FlattenOptions::default()
    };
    let report = design.flatten(top, &nothing).unwrap();
    assert_eq!(report.inlined_total, 0);
    assert_eq!(report.kept, 2);
    assert!(!validate(&design).has_errors());
}

/// A missing module id is reported rather than panicking.
#[test]
fn flattening_an_unknown_module_fails() {
    let mut design = load("three_level.rtl");
    let diags = design
        .flatten(ModuleId::from_index(42), &FlattenOptions::default())
        .unwrap_err();
    assert!(diags.has_errors());
}

/// The flattened design must behave exactly like the hierarchical one.
#[cfg(feature = "sim")]
#[test]
fn flattened_design_simulates_identically() {
    use reticle::logic::Logic;
    use reticle::sim::{SimOptions, Simulator};

    let hier = load("three_level.rtl");
    let mut flat = hier.clone();
    let top = flat.top.unwrap();
    flat.flatten(top, &FlattenOptions::default()).unwrap();
    flat.remove_unused_modules(top);

    let mut a = Simulator::new(&hier, SimOptions::default()).unwrap();
    let mut b = Simulator::new(&flat, SimOptions::default()).unwrap();
    let ports = ["clk", "x", "o0", "o1"];
    let handles: Vec<_> = ports
        .iter()
        .map(|p| {
            let path = format!("top.{p}");
            (
                a.net(&path).unwrap_or_else(|| panic!("{path} in hier")),
                b.net(&path).unwrap_or_else(|| panic!("{path} in flat")),
            )
        })
        .collect();
    let (clk_a, clk_b) = handles[0];
    let (x_a, x_b) = handles[1];

    // Drive both the way a testbench would: change the input while the
    // clock is low, let the combinational logic settle, then clock. Half a
    // period is five ticks.
    let half = 5;
    let low = Logic::from_u64(0, 1);
    let high = Logic::from_u64(1, 1);
    a.set(clk_a, low.clone());
    b.set(clk_b, low.clone());
    for cycle in 0..300u64 {
        let x = Logic::from_u64(cycle.wrapping_mul(7) & 0xff, 8);
        a.set(x_a, x.clone());
        b.set(x_b, x);
        a.run_for(half);
        b.run_for(half);
        a.set(clk_a, high.clone());
        b.set(clk_b, high.clone());
        a.run_for(half);
        b.run_for(half);
        for (i, name) in ports.iter().enumerate().skip(2) {
            let (ha, hb) = handles[i];
            assert_eq!(
                a.get(ha).to_string(),
                b.get(hb).to_string(),
                "cycle {cycle}: `{name}` differs"
            );
        }
        a.set(clk_a, low.clone());
        b.set(clk_b, low.clone());
        a.run_for(half);
        b.run_for(half);
    }
    assert_eq!(a.time(), b.time());
    assert!(a.messages().is_empty());
    assert!(b.messages().is_empty());
}
