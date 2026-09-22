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

/// Files synthesis reads while it runs (`$readmemh`), which no key can
/// cover: the entry records them and every lookup re-checks them.
///
/// The ROM under `testdata/cache/rom/` loads `prog.hex`. The library does
/// no I/O, so the files are handed in through a [`MemoryFiles`] that each
/// test edits the way a user would edit the file on disk.
#[cfg(all(feature = "verilog", feature = "synth"))]
mod discovered_inputs {
    use super::*;

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use reticle::cache::{BuildResult, Cache, Entry, FileInput, Outcome, Stage, Storage};
    use reticle::ir::memfile::{FileProvider, MemoryFiles};
    use reticle::synth::SynthOptions;

    fn testdata() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/cache/rom")
    }

    fn read(name: &str) -> String {
        fs::read_to_string(testdata().join(name))
            .unwrap_or_else(|e| panic!("cannot read {name}: {e}"))
    }

    fn sources() -> Vec<SourceUnit> {
        vec![SourceUnit::new("rom.v", read("rom.v"))]
    }

    /// The files as they are checked in, plus one the design never reads.
    fn files() -> MemoryFiles {
        let mut files = MemoryFiles::new();
        files.insert("prog.hex", read("prog.hex"));
        files.insert("unrelated.hex", "ff\n");
        files
    }

    fn options(files: Option<MemoryFiles>) -> BuildOptions {
        let synth = SynthOptions {
            files: files.map(|f| Rc::new(f) as Rc<dyn FileProvider>),
            ..SynthOptions::default()
        };
        BuildOptions::new(Language::Verilog)
            .with_top("rom")
            .with_synth(synth)
    }

    /// Builds, returning the result and what was reported.
    fn run(storage: &mut MemoryStorage, files: Option<MemoryFiles>) -> (BuildResult, Diagnostics) {
        let mut diags = Diagnostics::new();
        let result = build(&sources(), &options(files), storage, &mut diags);
        (result, diags)
    }

    /// Builds and asserts nothing went wrong.
    fn run_clean(storage: &mut MemoryStorage, files: MemoryFiles) -> BuildResult {
        let (result, diags) = run(storage, Some(files));
        assert!(!diags.has_errors(), "{}", diags.render(&result.sources));
        result
    }

    fn synth(result: &BuildResult) -> Outcome {
        result
            .outcome("rom", Stage::Synthesise)
            .unwrap_or_else(|| panic!("`rom` was not synthesised:\n{}", result.report()))
    }

    /// The key the synthesised module is filed under.
    fn synth_key(result: &BuildResult) -> reticle::cache::CacheKey {
        result
            .modules
            .iter()
            .find(|m| m.stage == Stage::Synthesise)
            .expect("synthesised")
            .key
    }

    /// The ROM's initial contents, as numbers.
    fn rom(result: &BuildResult) -> Option<Vec<u64>> {
        let design = result.design.as_ref().expect("a design");
        let module = design.top_module().expect("a top");
        let mem = module
            .memories
            .iter()
            .map(|(_, m)| m)
            .find(|m| m.name.as_str() == "mem")
            .expect("the ROM is still a memory");
        mem.init
            .as_ref()
            .map(|init| init.iter().map(|w| w.to_u64().expect("known")).collect())
    }

    const PROGRAM: [u64; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    const EDITED: [u64; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7];

    #[test]
    fn a_rom_loaded_by_readmemh_hits_the_second_time() {
        let mut storage = MemoryStorage::new();
        let cold = run_clean(&mut storage, files());
        assert_eq!(synth(&cold), Outcome::Miss);
        assert_eq!(rom(&cold).as_deref(), Some(&PROGRAM[..]));

        let warm = run_clean(&mut storage, files());
        assert_eq!(synth(&warm), Outcome::Hit, "{}", warm.report());
        assert_eq!(warm.misses, 0, "{}", warm.report());
        assert_eq!(rom(&warm).as_deref(), Some(&PROGRAM[..]));
    }

    #[test]
    fn editing_only_the_hex_file_misses_and_loads_the_new_program() {
        // The bug this guards against: the source text, and so every key,
        // is unchanged, so a cache that only keyed on it would serve the
        // old netlist, with the old program in the ROM.
        let mut storage = MemoryStorage::new();
        run_clean(&mut storage, files());

        let mut edited = files();
        edited.insert("prog.hex", "a0 a1 a2 a3 a4 a5 a6 a7\n");
        let after = run_clean(&mut storage, edited.clone());
        assert_eq!(synth(&after), Outcome::Miss, "{}", after.report());
        assert_eq!(rom(&after).as_deref(), Some(&EDITED[..]));
        // Elaboration read no file, so it is still a hit.
        assert_eq!(
            after.outcome("rom", Stage::Elaborate),
            Some(Outcome::Hit),
            "{}",
            after.report()
        );

        // The new program is now what is stored, and hits.
        let again = run_clean(&mut storage, edited);
        assert_eq!(synth(&again), Outcome::Hit, "{}", again.report());
        assert_eq!(rom(&again).as_deref(), Some(&EDITED[..]));

        // And putting the old file back finds the old program again.
        let back = run_clean(&mut storage, files());
        assert_eq!(synth(&back), Outcome::Miss, "{}", back.report());
        assert_eq!(rom(&back).as_deref(), Some(&PROGRAM[..]));
    }

    #[test]
    fn deleting_the_hex_file_misses() {
        let mut storage = MemoryStorage::new();
        run_clean(&mut storage, files());

        let mut gone = MemoryFiles::new();
        gone.insert("unrelated.hex", "ff\n");
        let (after, diags) = run(&mut storage, Some(gone.clone()));
        assert_eq!(synth(&after), Outcome::Miss, "{}", after.report());
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some("S0018") && d.is_error()),
            "a missing file is reported: {}",
            diags.render(&after.sources)
        );
        assert_eq!(rom(&after), None, "nothing was loaded");

        // A synthesis that failed is not stored, so the error comes back
        // on the next build rather than turning into a silent hit.
        let (again, diags) = run(&mut storage, Some(gone));
        assert_eq!(synth(&again), Outcome::Miss, "{}", again.report());
        assert!(diags.has_errors());

        // The entry from before the deletion is still there, and still
        // right for the file it recorded.
        let back = run_clean(&mut storage, files());
        assert_eq!(synth(&back), Outcome::Hit, "{}", back.report());
        assert_eq!(rom(&back).as_deref(), Some(&PROGRAM[..]));
    }

    #[test]
    fn a_file_that_appears_misses() {
        // Built while the file is missing: an error, and nothing stored.
        let mut storage = MemoryStorage::new();
        let (before, diags) = run(&mut storage, Some(MemoryFiles::new()));
        assert!(diags.has_errors());
        assert_eq!(synth(&before), Outcome::Miss);

        let after = run_clean(&mut storage, files());
        assert_eq!(synth(&after), Outcome::Miss, "{}", after.report());
        assert_eq!(rom(&after).as_deref(), Some(&PROGRAM[..]));
    }

    #[test]
    fn a_recorded_missing_file_that_appears_misses() {
        // `$readmemh` fails without its file, and a failed synthesis is not
        // stored, so a stored "not found" record cannot come from it today.
        // Add one by hand, as a design with an optional file would have,
        // to check that a lookup honours it.
        let mut storage = MemoryStorage::new();
        let cold = run_clean(&mut storage, files());
        let key = synth_key(&cold);
        let entry = Entry::decode(&storage.get(key).expect("stored")).expect("readable");
        let mut inputs = entry.inputs.clone();
        inputs.push(FileInput {
            path: "optional.hex".to_owned(),
            digest: None,
        });
        Cache::new(&mut storage).put(entry.with_inputs(inputs));

        // Still absent: a hit.
        let still = run_clean(&mut storage, files());
        assert_eq!(synth(&still), Outcome::Hit, "{}", still.report());

        // Present, even empty: a miss.
        let mut appeared = files();
        appeared.insert("optional.hex", "");
        let after = run_clean(&mut storage, appeared);
        assert_eq!(synth(&after), Outcome::Miss, "{}", after.report());
    }

    #[test]
    fn an_unrelated_file_changing_keeps_the_hit() {
        let mut storage = MemoryStorage::new();
        run_clean(&mut storage, files());

        let mut other = files();
        other.insert("unrelated.hex", "00\n");
        other.insert("brand_new.hex", "01\n");
        let after = run_clean(&mut storage, other);
        assert_eq!(synth(&after), Outcome::Hit, "{}", after.report());
        assert_eq!(after.misses, 0, "{}", after.report());
    }

    #[test]
    fn being_given_files_misses() {
        // Without files the ROM is left empty with a warning; the key says
        // whether there is a provider, so being given one is a miss.
        let mut storage = MemoryStorage::new();
        let (bare, diags) = run(&mut storage, None);
        assert!(!diags.has_errors(), "{}", diags.render(&bare.sources));
        assert!(diags.iter().any(|d| d.code == Some("S0018")));
        assert_eq!(rom(&bare), None);

        let with = run_clean(&mut storage, files());
        assert_eq!(synth(&with), Outcome::Miss, "{}", with.report());
        assert_eq!(rom(&with).as_deref(), Some(&PROGRAM[..]));
    }

    #[test]
    fn a_cached_rom_and_a_cold_one_agree_byte_for_byte() {
        let mut storage = MemoryStorage::new();
        let cold = run_clean(&mut storage, files());
        let warm = run_clean(&mut storage, files());
        assert_eq!(synth(&warm), Outcome::Hit);
        let fresh = run_clean(&mut MemoryStorage::new(), files());

        let cold = cold.design.expect("a design").to_text();
        let warm = warm.design.expect("a design").to_text();
        let fresh = fresh.design.expect("a design").to_text();
        assert_eq!(cold, warm, "a hit must give the bytes a cold build does");
        assert_eq!(cold, fresh);
        assert!(
            cold.contains("init 8'd17 8'd34 8'd51 8'd68 8'd85 8'd102 8'd119 8'd136"),
            "{cold}"
        );

        let golden = testdata().join("rom.synth.rtl");
        if std::env::var_os("UPDATE_EXPECT").is_some() {
            fs::write(&golden, &cold).expect("writable");
        }
        assert_eq!(
            fs::read_to_string(&golden).expect("run with UPDATE_EXPECT=1"),
            cold
        );
    }

    #[test]
    fn the_entry_lists_the_files_it_read() {
        let mut storage = MemoryStorage::new();
        let cold = run_clean(&mut storage, files());
        let raw = String::from_utf8(storage.get(synth_key(&cold)).expect("stored")).expect("text");
        let header = raw.split("\n\n").next().expect("a header");
        let inputs: Vec<&str> = header.lines().filter(|l| l.starts_with("input ")).collect();
        assert_eq!(inputs.len(), 1, "{header}");
        assert!(inputs[0].ends_with(" prog.hex"), "{header}");
        assert!(!header.contains("unrelated.hex"), "{header}");
    }
}
