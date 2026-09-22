//! Integration tests for incremental builds (`reticle::cache`).
//!
//! The design under `testdata/cache/` is `top` -> `mid` -> `leaf`, plus
//! `top` -> `other`, which is the smallest shape that can tell the three
//! invalidation directions apart: a leaf edit must reach `top`, a top edit
//! must not reach `leaf`, and neither must touch `other`.
//!
//! Run with `UPDATE_EXPECT=1` to rewrite the golden files after a
//! deliberate change.
#![cfg(feature = "cache")]

use reticle::cache::{BuildOptions, Language, MemoryStorage, SourceUnit, build};
use reticle::diag::Diagnostics;

/// A design in the IR text format, which needs no frontend at all.
const RTL: &str = "\
top t

module leaf
  net %o u1 wire
  port o out %o
  assign %o = 1'b0
end

module t
  net %o u1 wire
  port o out %o
  instance u of leaf (o=%o)
end
";

#[test]
fn an_rtl_build_hits_the_second_time() {
    let sources = vec![SourceUnit::new("d.rtl", RTL)];
    let options = BuildOptions::new(Language::Rtl).with_top("t");
    let mut storage = MemoryStorage::new();
    let mut diags = Diagnostics::new();

    let cold = build(&sources, &options, &mut storage, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&cold.sources));
    assert_eq!(cold.hits, 0);
    assert!(cold.misses > 0);

    let warm = build(&sources, &options, &mut storage, &mut diags);
    assert_eq!(warm.misses, 0, "{}", warm.report());
    assert_eq!(
        cold.design.unwrap().to_text(),
        warm.design.unwrap().to_text()
    );
}

#[test]
fn a_store_survives_being_reopened() {
    // The cache holds nothing in memory between builds: everything needed
    // to hit again is in the backend.
    let sources = vec![SourceUnit::new("d.rtl", RTL)];
    let options = BuildOptions::new(Language::Rtl).with_top("t");
    let mut storage = MemoryStorage::new();
    let mut diags = Diagnostics::new();
    build(&sources, &options, &mut storage, &mut diags);

    let copy = storage.clone();
    let mut copy = copy;
    let warm = build(&sources, &options, &mut copy, &mut diags);
    assert_eq!(warm.misses, 0, "{}", warm.report());
}

#[cfg(feature = "verilog")]
mod verilog {
    use super::*;

    use std::fs;
    use std::path::{Path, PathBuf};

    use reticle::cache::{BuildResult, Outcome, Stage};

    /// The directory the fixtures live in.
    fn testdata() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/cache")
    }

    /// Compares against a golden file, rewriting it under `UPDATE_EXPECT=1`.
    fn expect(name: &str, actual: &str) {
        let path = testdata().join(name);
        if std::env::var_os("UPDATE_EXPECT").is_some() {
            fs::write(&path, actual)
                .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
            return;
        }
        let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {} ({e}); run with UPDATE_EXPECT=1",
                path.display()
            )
        });
        assert_eq!(expected, actual, "{} is out of date", path.display());
    }

    /// The four fixture files, in a deterministic order.
    fn sources() -> Vec<SourceUnit> {
        ["leaf.v", "mid.v", "other.v", "top.v"]
            .iter()
            .map(|name| {
                let text = fs::read_to_string(testdata().join(name))
                    .unwrap_or_else(|e| panic!("cannot read {name}: {e}"));
                SourceUnit::new(*name, text)
            })
            .collect()
    }

    fn options() -> BuildOptions {
        BuildOptions::new(Language::Verilog).with_top("top")
    }

    /// Builds and asserts nothing was reported.
    fn run(sources: &[SourceUnit], storage: &mut MemoryStorage) -> BuildResult {
        let mut diags = Diagnostics::new();
        let result = build(sources, &options(), storage, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&result.sources));
        result
    }

    /// Replaces one file's text.
    fn edit(sources: &[SourceUnit], name: &str, text: &str) -> Vec<SourceUnit> {
        sources
            .iter()
            .map(|unit| {
                if unit.name == name {
                    SourceUnit::new(name, text)
                } else {
                    unit.clone()
                }
            })
            .collect()
    }

    fn outcome(result: &BuildResult, module: &str) -> Outcome {
        result
            .outcome(module, Stage::Elaborate)
            .unwrap_or_else(|| panic!("`{module}` was not built:\n{}", result.report()))
    }

    #[test]
    fn a_second_build_is_all_hits() {
        let sources = sources();
        let mut storage = MemoryStorage::new();

        let cold = run(&sources, &mut storage);
        assert_eq!(cold.hits, 0, "a cold build cannot hit:\n{}", cold.report());
        for module in ["leaf", "mid", "other", "top"] {
            assert_eq!(outcome(&cold, module), Outcome::Miss);
        }

        let warm = run(&sources, &mut storage);
        assert_eq!(warm.misses, 0, "{}", warm.report());
        for module in ["leaf", "mid", "other", "top"] {
            assert_eq!(outcome(&warm, module), Outcome::Hit);
        }
        // The scans hit too, so the warm build never parses a file.
        assert!(warm.scans.iter().all(|s| s.outcome == Outcome::Hit));
    }

    #[test]
    fn editing_a_leaf_misses_the_leaf_and_everything_above_it() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        run(&sources, &mut storage);

        let edited = edit(
            &sources,
            "leaf.v",
            "module leaf (input wire a, output wire y);\n    assign y = a;\nendmodule\n",
        );
        let result = run(&edited, &mut storage);

        assert_eq!(outcome(&result, "leaf"), Outcome::Miss, "the edited module");
        assert_eq!(outcome(&result, "mid"), Outcome::Miss, "reaches `leaf`");
        assert_eq!(outcome(&result, "top"), Outcome::Miss, "reaches `leaf`");
        assert_eq!(outcome(&result, "other"), Outcome::Hit, "unrelated");
    }

    #[test]
    fn editing_the_top_misses_only_the_top() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        run(&sources, &mut storage);

        let edited = edit(
            &sources,
            "top.v",
            "module top (input wire a, output wire y, output wire z);\n\
             \x20   mid u0 (.a(a), .y(y));\n\
             \x20   other u1 (.a(~a), .y(z));\n\
             endmodule\n",
        );
        let result = run(&edited, &mut storage);

        assert_eq!(outcome(&result, "top"), Outcome::Miss, "the edited module");
        assert_eq!(outcome(&result, "leaf"), Outcome::Hit, "below the edit");
        assert_eq!(outcome(&result, "mid"), Outcome::Hit, "below the edit");
        assert_eq!(outcome(&result, "other"), Outcome::Hit, "below the edit");
    }

    #[test]
    fn editing_an_unrelated_module_leaves_the_rest_alone() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        run(&sources, &mut storage);

        let edited = edit(
            &sources,
            "other.v",
            "module other (input wire a, output wire y);\n    assign y = ~a;\nendmodule\n",
        );
        let result = run(&edited, &mut storage);

        assert_eq!(outcome(&result, "other"), Outcome::Miss);
        assert_eq!(outcome(&result, "leaf"), Outcome::Hit);
        assert_eq!(outcome(&result, "mid"), Outcome::Hit);
        // `top` instantiates `other`, so it misses too.
        assert_eq!(outcome(&result, "top"), Outcome::Miss);
    }

    #[test]
    fn editing_only_a_comment_still_misses() {
        // Keys are taken over the source text, comments included: nothing
        // normalises them away, because doing so would make a key depend
        // on a second parse of the file. A comment-only edit is therefore
        // a miss for that module and everything above it, and a hit for
        // everything else. `docs/cache.md` explains the trade.
        let sources = sources();
        let mut storage = MemoryStorage::new();
        let cold = run(&sources, &mut storage);

        let original = fs::read_to_string(testdata().join("leaf.v")).unwrap();
        let edited = edit(
            &sources,
            "leaf.v",
            &format!("// one more comment, nothing else\n{original}"),
        );
        let result = run(&edited, &mut storage);

        assert_eq!(outcome(&result, "leaf"), Outcome::Miss);
        assert_eq!(outcome(&result, "mid"), Outcome::Miss);
        assert_eq!(outcome(&result, "top"), Outcome::Miss);
        assert_eq!(outcome(&result, "other"), Outcome::Hit);

        // The design is unchanged, though: only the key moved.
        assert_eq!(
            cold.design.unwrap().to_text(),
            result.design.unwrap().to_text()
        );
    }

    #[test]
    fn a_cached_build_and_a_cold_build_agree_byte_for_byte() {
        // The property that matters more than any speed-up.
        let sources = sources();
        let mut warm_storage = MemoryStorage::new();
        let cold = run(&sources, &mut warm_storage);
        let warm = run(&sources, &mut warm_storage);

        let mut fresh = MemoryStorage::new();
        let independent = run(&sources, &mut fresh);

        let cold = cold.design.expect("a design").to_text();
        let warm = warm.design.expect("a design").to_text();
        let independent = independent.design.expect("a design").to_text();
        assert_eq!(cold, warm, "a hit must give what the miss gave");
        assert_eq!(cold, independent, "a cold build must be reproducible");
        expect("design.rtl", &cold);
    }

    #[test]
    fn a_partly_cached_build_still_agrees_with_a_cold_one() {
        // The interesting case: some modules from the store, some fresh.
        let sources = sources();
        let mut storage = MemoryStorage::new();
        run(&sources, &mut storage);

        let edited = edit(
            &sources,
            "leaf.v",
            "module leaf (input wire a, output wire y);\n    assign y = a;\nendmodule\n",
        );
        let mixed = run(&edited, &mut storage);
        let mut fresh = MemoryStorage::new();
        let cold = run(&edited, &mut fresh);

        assert!(mixed.hits > 0 && mixed.misses > 0, "{}", mixed.report());
        assert_eq!(
            cold.design.expect("a design").to_text(),
            mixed.design.expect("a design").to_text()
        );
    }

    #[test]
    fn the_report_is_a_stable_golden() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        let cold = run(&sources, &mut storage);
        expect("cold.expect", &cold.report());
        let warm = run(&sources, &mut storage);
        expect("warm.expect", &warm.report());
    }

    #[test]
    fn only_top_builds_one_module_and_shares_the_store() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();

        let full = build(&sources, &options(), &mut storage, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&full.sources));

        let narrow_options = options().with_only_top(true);
        let narrow = build(&sources, &narrow_options, &mut storage, &mut diags);
        assert_eq!(narrow.modules.len(), 1, "{}", narrow.report());
        assert_eq!(narrow.modules[0].name, "top");
        // An artefact does not depend on which modules were asked for, so
        // the entry the full build wrote is the entry this build reads.
        assert_eq!(narrow.modules[0].outcome, Outcome::Hit);
        assert_eq!(
            full.design.expect("a design").to_text(),
            narrow.design.expect("a design").to_text()
        );
    }

    #[test]
    fn parameters_are_part_of_the_top_key() {
        let sources = vec![SourceUnit::new(
            "p.v",
            "module p #(parameter W = 8) (output wire [W-1:0] y);\n\
             \x20   assign y = {W{1'b0}};\nendmodule\n",
        )];
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let base = BuildOptions::new(Language::Verilog).with_top("p");

        let first = build(&sources, &base, &mut storage, &mut diags);
        let wider = base.clone().with_param("W", "16");
        let second = build(&sources, &wider, &mut storage, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&second.sources));

        assert_ne!(first.modules[0].key, second.modules[0].key);
        assert_eq!(second.modules[0].outcome, Outcome::Miss);
        assert_ne!(
            first.design.expect("a design").to_text(),
            second.design.expect("a design").to_text()
        );

        // And the same override hits.
        let third = build(&sources, &wider, &mut storage, &mut diags);
        assert_eq!(third.modules[0].outcome, Outcome::Hit);
    }

    #[test]
    fn a_missing_top_is_reported() {
        let sources = sources();
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let options = BuildOptions::new(Language::Verilog).with_top("nope");
        let result = build(&sources, &options, &mut storage, &mut diags);
        assert!(result.design.is_none());
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(reticle::cache::build::NO_SUCH_TOP))
        );
    }

    #[cfg(feature = "synth")]
    #[test]
    fn synthesised_modules_are_cached_under_their_own_key() {
        use reticle::synth::SynthOptions;

        let sources = sources();
        let mut storage = MemoryStorage::new();
        let mut diags = Diagnostics::new();
        let synth_options = options().with_synth(SynthOptions::default());

        let cold = build(&sources, &synth_options, &mut storage, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&cold.sources));
        assert_eq!(cold.outcome("top", Stage::Synthesise), Some(Outcome::Miss));

        let warm = build(&sources, &synth_options, &mut storage, &mut diags);
        // A synthesis hit answers on its own: the elaborated entry is not
        // even looked at.
        assert_eq!(warm.outcome("top", Stage::Synthesise), Some(Outcome::Hit));
        assert_eq!(warm.outcome("top", Stage::Elaborate), None);
        assert_eq!(warm.misses, 0, "{}", warm.report());
        assert_eq!(
            cold.design.expect("a design").to_text(),
            warm.design.expect("a design").to_text()
        );

        // Changing a synthesis option invalidates the netlist but not the
        // elaboration underneath it.
        let other = SynthOptions {
            max_iterations: 3,
            ..SynthOptions::default()
        };
        let changed = build(
            &sources,
            &options().with_synth(other),
            &mut storage,
            &mut diags,
        );
        assert_eq!(
            changed.outcome("top", Stage::Synthesise),
            Some(Outcome::Miss)
        );
        assert_eq!(changed.outcome("top", Stage::Elaborate), Some(Outcome::Hit));
    }
}
