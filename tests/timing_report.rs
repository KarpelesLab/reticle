//! Golden tests for static timing analysis and clock domain crossing
//! analysis.
//!
//! Each case under `testdata/timing/` is a design in the IR text format
//! (`<name>.rtl`), optionally with constraints (`<name>.rcf`, parsed by
//! `fpga::Constraints`) and a Liberty library (`<name>.lib`, which
//! switches the run from the unit delay model to the non-linear one).
//! Every case is taken through both analyses:
//!
//! | File            | What it holds                              |
//! |-----------------|--------------------------------------------|
//! | `<name>.timing` | `TimingReport::render`                     |
//! | `<name>.cdc`    | `CdcReport::render`                        |
//!
//! Set `UPDATE_EXPECT=1` to rewrite the expectations after an intended
//! change, and read the diff before committing it.
//!
//! The goldens are not the only check. Where a number can be worked out
//! by hand it is asserted here as well, in [`hand_computed_slacks`], so
//! a regression cannot quietly become a new, wrong, but stable golden
//! file. Timing values are floats, so those comparisons use a tolerance
//! of one femtosecond: that is far below anything a library
//! characterises, so a difference that small can only be floating-point
//! rounding.

#![cfg(feature = "timing")]

// The golden cases read their clocks and path exceptions from `.rcf`
// constraint files, which `fpga::Constraints` parses; with the `timing`
// feature alone there is no parser for them, and this file compiles to
// an empty test binary.
#[cfg(feature = "fpga")]
mod golden {
    use std::fs;
    use std::path::{Path, PathBuf};

    use reticle::diag::Diagnostics;
    use reticle::fpga::Constraints;
    use reticle::ir::Design;
    use reticle::ir::validate::validate;
    use reticle::source::SourceMap;
    use reticle::timing::cdc::analyze_cdc;
    use reticle::timing::delay::{DelayModel, UnitModel};
    use reticle::timing::sta::{Check, TimingOptions, TimingReport, analyze};

    /// Every case, in the order the report files are written. The
    /// Liberty one needs the `asic` feature's library reader, so it is
    /// only in the list when that feature is on.
    fn cases() -> Vec<&'static str> {
        let mut cases = vec![
            "reg2reg",
            "multiclock",
            "false_path",
            "multicycle",
            "comb_loop",
        ];
        #[cfg(feature = "asic")]
        cases.push("liberty");
        cases.extend(["cdc_unsync", "cdc_sync2", "cdc_reconverge", "async_fifo"]);
        cases
    }

    /// One femtosecond. Timing numbers are floats, so the hand-computed
    /// checks compare with a tolerance; anything a library
    /// characterises is orders of magnitude larger than this, so a
    /// difference this small can only come from rounding.
    const EPS: f64 = 1e-9;

    fn dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/timing")
    }

    fn read(name: &str) -> Option<String> {
        let path = dir().join(name);
        match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => panic!("cannot read {}: {e}", path.display()),
        }
    }

    /// Compares `actual` with the file `name`, rewriting it under
    /// `UPDATE_EXPECT`.
    fn expect(name: &str, actual: &str, failures: &mut Vec<String>) {
        let path = dir().join(name);
        let expected = fs::read_to_string(&path).unwrap_or_default();
        if expected == actual {
            return;
        }
        if std::env::var_os("UPDATE_EXPECT").is_some() {
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
            return;
        }
        failures.push(format!(
            "{name} differs (set UPDATE_EXPECT=1 to rewrite)\n--- expected ---\n{expected}\n--- \
             actual ---\n{actual}"
        ));
    }

    /// The options every golden case runs with: a few paths so the
    /// report has something to show, and both checks.
    fn options() -> TimingOptions {
        TimingOptions {
            paths_per_endpoint: 1,
            max_paths: 12,
            ..TimingOptions::default()
        }
    }

    /// Loads a case: its design, its constraints and its delay model.
    struct Case {
        design: Design,
        constraints: Constraints,
        sources: SourceMap,
        #[cfg(feature = "asic")]
        library: Option<reticle::asic::liberty::Library>,
    }

    fn load(name: &str) -> Case {
        let mut sources = SourceMap::new();
        let text = read(&format!("{name}.rtl")).expect("every case has a design");
        let file = sources.add(format!("{name}.rtl"), &text).unwrap();
        let design = Design::parse_text(&text, file)
            .unwrap_or_else(|d| panic!("{name}.rtl does not parse:\n{}", d.render(&sources)));
        let diags = validate(&design);
        assert!(
            !diags.has_errors(),
            "{name}.rtl does not validate:\n{}",
            diags.render(&sources)
        );

        let mut constraints = Constraints::new();
        if let Some(text) = read(&format!("{name}.rcf")) {
            let file = sources.add(format!("{name}.rcf"), &text).unwrap();
            let mut diags = Diagnostics::new();
            constraints = Constraints::parse(&text, file, &mut diags);
            assert!(
                !diags.has_errors(),
                "{name}.rcf does not parse:\n{}",
                diags.render(&sources)
            );
        }

        #[cfg(feature = "asic")]
        let library = read(&format!("{name}.lib")).map(|text| {
            let file = sources.add(format!("{name}.lib"), &text).unwrap();
            let mut diags = Diagnostics::new();
            let lib = reticle::asic::liberty::Library::parse(&text, file, &mut diags)
                .unwrap_or_else(|| panic!("{name}.lib has no library group"));
            assert!(
                !diags.has_errors(),
                "{name}.lib does not parse:\n{}",
                diags.render(&sources)
            );
            lib
        });

        Case {
            design,
            constraints,
            sources,
            #[cfg(feature = "asic")]
            library,
        }
    }

    /// Runs both analyses over a case and returns the two reports.
    fn run(name: &str) -> (TimingReport, String) {
        let case = load(name);
        let top = case
            .design
            .top
            .expect("every case names its top module with `top`");
        // A hierarchical design is flattened first; these cases are
        // already flat, so this only proves the path exists.
        let module = reticle::timing::graph::flatten_for_timing(&case.design, top)
            .unwrap_or_else(|d| panic!("{name} does not flatten:\n{}", d.render(&case.sources)));

        let unit = UnitModel::new().with_setup(0.2).with_hold(0.1);
        #[cfg(feature = "asic")]
        let liberty;
        #[allow(unused_mut)]
        let mut model: &dyn DelayModel = &unit;
        #[cfg(feature = "asic")]
        if let Some(lib) = &case.library {
            liberty = reticle::timing::delay::LibertyModel::new(lib);
            model = &liberty;
        }
        let timing = analyze(&module, &options(), model, &case.constraints);
        let cdc = analyze_cdc(&module, &case.constraints);
        (timing, cdc.render())
    }

    #[test]
    fn golden_reports() {
        let mut failures = Vec::new();
        for name in cases() {
            let (timing, cdc) = run(name);
            expect(&format!("{name}.timing"), &timing.render(), &mut failures);
            expect(&format!("{name}.cdc"), &cdc, &mut failures);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }

    /// Every number below was worked out by hand from the netlist, the
    /// `.rcf` file and the unit delay model (one unit per cell arc,
    /// nothing per net, 0.200 setup, 0.100 hold). Asserting them here as
    /// well as snapshotting the reports is what stops a golden file from
    /// drifting into being wrong but stable.
    #[test]
    fn hand_computed_slacks() {
        let close = |a: f64, b: f64| (a - b).abs() < EPS;

        // reg2reg: ff1 -> AND -> ff2 on a 10 ns clock. The clock-to-Q
        // arc is one unit and the AND another, so the data arrives at
        // 2.000 and the budget is 10.000 - 0.200.
        let (r, _) = run("reg2reg");
        assert!(close(r.slack_at("ff2/d", Check::Setup).unwrap(), 7.8));
        // The earliest data at ff2/d comes from the port `b` through the
        // AND, at 1.000, against a hold requirement of 0.100.
        assert!(close(r.slack_at("ff2/d", Check::Hold).unwrap(), 0.9));
        // ff1 captures the port `d` directly, with no delay at all.
        assert!(close(r.slack_at("ff1/d", Check::Setup).unwrap(), 9.8));
        assert!(close(r.slack_at("ff1/d", Check::Hold).unwrap(), -0.1));
        // The output port sees ff2/q one unit after the edge.
        assert!(close(r.slack_at("q", Check::Setup).unwrap(), 9.0));

        // multiclock: `fast` has edges at 0, 10, 20 and `slow` at 0, 15,
        // 30; the tightest pair with capture after launch is 10 -> 15.
        // The path is one clock-to-Q plus one inverter, so 2.000 of
        // data against 5.000 - 0.200 of budget.
        let (r, _) = run("multiclock");
        assert!(close(r.slack_at("dst/d", Check::Setup).unwrap(), 2.8));
        let path = r
            .paths
            .iter()
            .find(|p| p.end_pin == "dst/d" && p.check == Check::Setup)
            .expect("the crossing path is reported");
        assert_eq!(path.launch_clock, "fast");
        assert_eq!(path.capture_clock, "slow");
        assert!(close(path.launch_edge, 10.0));
        assert!(close(path.capture_edge, 15.0));

        // false_path: the configuration register reaches `out` through
        // an XOR and an AND, which would be 3.000 of data and 6.800 of
        // slack; the constraint cuts it, leaving the data register's own
        // path at the same depth but launched by `datareg`.
        let (r, _) = run("false_path");
        assert!(
            !r.paths
                .iter()
                .any(|p| p.end_pin == "out/d" && p.start == "cfgreg"),
            "the false path is still reported"
        );
        // datareg -> xor -> and -> out is three units of data.
        assert!(close(r.slack_at("out/d", Check::Setup).unwrap(), 6.8));

        // multicycle: the multiplier path is two units (clock-to-Q plus
        // the multiply) and is given two periods, so 10.000 + 10.000 -
        // 0.200 - 2.000.
        let (r, _) = run("multicycle");
        let path = r
            .paths
            .iter()
            .find(|p| p.end_pin == "mul_q/d" && p.check == Check::Setup)
            .expect("the multicycle path is reported");
        assert_eq!(path.multicycle, 2);
        assert!(close(path.slack, 17.8));
        assert!(close(r.slack_at("mul_q/d", Check::Setup).unwrap(), 17.8));

        // comb_loop: the loop is reported, and the run finishes.
        let (r, _) = run("comb_loop");
        assert_eq!(r.loops.len(), 1);
        assert!(r.notes.iter().any(|n| n.contains("combinational loop")));
    }

    /// The crossing classifications, and which of them are claimed as
    /// proved.
    #[test]
    fn crossing_classifications() {
        use reticle::diag::Severity;
        use reticle::timing::cdc::CrossingKind;

        let check = |name: &str| {
            let case = load(name);
            let top = case.design.top.unwrap();
            let module = reticle::timing::graph::flatten_for_timing(&case.design, top).unwrap();
            analyze_cdc(&module, &case.constraints)
        };

        // An unsynchronised crossing is an error, and a structural fact.
        let r = check("cdc_unsync");
        assert_eq!(r.crossings.len(), 1);
        assert_eq!(r.crossings[0].kind, CrossingKind::Unsynchronised);
        assert_eq!(r.crossings[0].severity, Severity::Error);
        assert!(r.crossings[0].proven);
        assert_eq!(r.crossings[0].from_domain, "aclk");
        assert_eq!(r.crossings[0].to_domain, "bclk");

        // A two-flop synchroniser is recognised, also structurally.
        let r = check("cdc_sync2");
        assert_eq!(r.crossings.len(), 1);
        assert_eq!(r.crossings[0].kind, CrossingKind::Synchroniser { depth: 2 });
        assert!(r.crossings[0].proven);
        assert!(!r.has_errors());

        // Two correct synchronisers recombined: each crossing is fine,
        // the pair is the bug.
        let r = check("cdc_reconverge");
        assert!(
            r.crossings
                .iter()
                .all(|c| !matches!(c.kind, CrossingKind::Unsynchronised))
        );
        assert_eq!(r.reconvergences.len(), 1);
        assert_eq!(r.reconvergences[0].at_cell, "recombine");
        assert_eq!(r.reconvergences[0].synchronisers, ["sync0b", "sync1b"]);

        // The FIFO's shape is proved; its gray pointers are not.
        let r = check("async_fifo");
        let fifo = r
            .crossings
            .iter()
            .find(|c| matches!(c.kind, CrossingKind::AsyncFifo { .. }))
            .expect("the memory crossing is found");
        assert_eq!(
            fifo.kind,
            CrossingKind::AsyncFifo {
                memory: "ram".to_owned(),
                pointers_synchronised: true,
            }
        );
        assert!(!fifo.proven, "a FIFO's pointer encoding is not provable");
        // The pointer buses are recognised, and the gray generator is
        // found, but the finding stays unverified.
        let bus = r
            .crossings
            .iter()
            .find(|c| matches!(c.kind, CrossingKind::GrayBus { .. }))
            .expect("the pointer bus is found");
        assert_eq!(
            bus.kind,
            CrossingKind::GrayBus {
                bits: 2,
                generator_found: true,
            }
        );
        assert!(!bus.proven);
    }

    /// Rendering the same report twice gives the same bytes, and the
    /// summary is a prefix of the whole report.
    #[test]
    fn reports_are_deterministic() {
        for name in cases() {
            let (timing, cdc) = run(name);
            assert_eq!(timing.render(), timing.render(), "{name}");
            assert!(
                timing.render().starts_with(&timing.render_summary()),
                "{name}"
            );
            let (again, cdc_again) = run(name);
            assert_eq!(timing.render(), again.render(), "{name}");
            assert_eq!(cdc, cdc_again, "{name}");
        }
    }
}
