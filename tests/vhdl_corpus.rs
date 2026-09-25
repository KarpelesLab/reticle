//! Reticle's VHDL frontend and simulator against an independent corpus:
//! CERN's **Colibri** common VHDL library.
//!
//! Colibri is production VHDL nobody here wrote, deliberately vendor
//! independent, with self-checking testbenches of its own. It is the
//! strongest test available of the frontend, and every defect it found is
//! listed in `docs/vhdl-corpus.md` with the regression test that pins the
//! fix. Those regression tests live in `testdata/vhdl/` and need no corpus;
//! this file needs one.
//!
//! # Getting the corpus
//!
//! ```sh
//! git clone https://gitlab.com/colibri-cern/colibri.git /path/to/colibri
//! export RETICLE_COLIBRI=/path/to/colibri
//! cargo test --all-features --test vhdl_corpus -- --nocapture
//! ```
//!
//! It is never vendored and CI never has it: without `RETICLE_COLIBRI`
//! every test here prints one line and passes, so the gate is green without
//! it.
//!
//! # What is measured
//!
//! `CORPUS_DUMP=1` additionally prints every diagnostic, which is where a
//! fix starts.
//!
//! Each test prints what it measured and asserts a floor, so that a change
//! which makes less of the corpus work fails here. The floors are the
//! numbers `docs/vhdl-corpus.md` quotes; raising them is the point, and
//! whoever raises one updates that page in the same commit.

#![cfg(feature = "vhdl")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use reticle::diag::{Diagnostic, Diagnostics, Severity};
use reticle::source::SourceMap;
use reticle::vhdl::sema::{Design, LibraryUnitKind};
use reticle::vhdl::{Analysis, ElabOptions, Standard, elaborate, parse_source};

/// The library Colibri is compiled into; its own readme says so.
const LIBRARY: &str = "colibri";

/// The checkout, or `None` with a line saying what is missing.
fn root() -> Option<PathBuf> {
    let Some(var) = std::env::var_os("RETICLE_COLIBRI") else {
        eprintln!(
            "skipped: needs the Colibri corpus; \
             `git clone https://gitlab.com/colibri-cern/colibri.git` and point \
             RETICLE_COLIBRI at it (see docs/vhdl-corpus.md)"
        );
        return None;
    };
    let root = PathBuf::from(var);
    let probe = root.join("src/common/counter.vhdl");
    if !probe.is_file() {
        eprintln!(
            "skipped: RETICLE_COLIBRI is set but `{}` is not there",
            probe.display()
        );
        return None;
    }
    Some(root)
}

/// Every `.vhdl` file under `dir`, sorted.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "vhdl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// A diagnostic's headline with the quoted names blanked out, so that
/// errors of one cause count together however they are spelled.
fn cause(d: &Diagnostic) -> String {
    let mut out = String::new();
    let mut quoting = false;
    for c in d.message.chars() {
        match c {
            '`' if quoting => {
                quoting = false;
                out.push_str("X`");
            }
            '`' => {
                quoting = true;
                out.push('`');
            }
            _ if quoting => {}
            _ => out.push(c),
        }
    }
    match d.code {
        Some(code) => format!("[{code}] {out}"),
        None => out,
    }
}

/// The file a diagnostic points into.
fn file_of(map: &SourceMap, d: &Diagnostic) -> String {
    d.labels.first().map_or_else(
        || "<no file>".to_owned(),
        |l| map.file(l.span.file).name().to_owned(),
    )
}

/// The library's own sources, analysed together as one library.
struct Corpus {
    map: SourceMap,
    analysis: Analysis,
    diags: Diagnostics,
    files: Vec<String>,
}

impl Corpus {
    /// The corpus's own design units of one kind.
    fn units(&self, kind: LibraryUnitKind) -> Vec<String> {
        let library = self.analysis.interner.get_ci(LIBRARY);
        self.analysis
            .units
            .iter()
            .filter(|u| Some(u.library) == library && u.kind == kind)
            .map(|u| self.analysis.name(u.name).to_owned())
            .collect()
    }

    /// Elaborates one entity as the top of a design of its own.
    fn elaborate(&self, top: &str) -> (Option<reticle::ir::Design>, Diagnostics) {
        let mut opts = ElabOptions::new().with_top(top);
        opts.library = Some(LIBRARY.to_owned());
        opts.only_top = true;
        let mut diags = Diagnostics::new();
        let design = elaborate(&self.analysis, &opts, &mut diags);
        (design, diags)
    }
}

fn analyse(root: &Path) -> Corpus {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let mut design = Design::with_stdlib(&mut map, Standard::Vhdl2008, &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let mut files = Vec::new();
    for f in sources(&root.join("src")) {
        let text = fs::read_to_string(&f).expect("read source");
        let name = f
            .strip_prefix(root)
            .unwrap_or(&f)
            .to_string_lossy()
            .into_owned();
        let id = map.add(&name, &text).expect("source fits");
        design.add_source(&map, id, LIBRARY, &mut diags);
        files.push(name);
    }
    let mut diags = Diagnostics::new();
    let analysis = design.analyze(&map, &mut diags);
    Corpus {
        map,
        analysis,
        diags,
        files,
    }
}

#[test]
fn every_source_file_parses() {
    let Some(root) = root() else { return };
    let mut files = sources(&root.join("src"));
    files.extend(sources(&root.join("sim")));
    files.extend(sources(&root.join("fv")));
    assert!(files.len() > 200, "found only {} files", files.len());

    let mut map = SourceMap::new();
    let mut failed = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).expect("read source");
        let name = f
            .strip_prefix(&root)
            .unwrap_or(f)
            .to_string_lossy()
            .into_owned();
        let id = map.add(&name, &text).expect("source fits");
        let mut diags = Diagnostics::new();
        let _ = parse_source(&map, id, Standard::Vhdl2008, &mut diags);
        if diags.has_errors() {
            failed.push((name, diags.render(&map)));
        }
    }
    println!(
        "parsed {} of {} files (designs, testbenches and formal benches)",
        files.len() - failed.len(),
        files.len()
    );
    for (name, rendered) in &failed {
        println!("---- {name}\n{rendered}");
    }
    assert!(
        failed.is_empty(),
        "{} of {} files do not parse",
        failed.len(),
        files.len()
    );
}

#[test]
fn the_library_analyses() {
    let Some(root) = root() else { return };
    let c = analyse(&root);

    let mut per_file: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_cause: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = 0;
    for d in c.diags.iter().filter(|d| d.severity == Severity::Error) {
        errors += 1;
        *per_file.entry(file_of(&c.map, d)).or_default() += 1;
        *per_cause.entry(cause(d)).or_default() += 1;
    }
    let clean = c.files.len() - per_file.len();

    let entities = c.units(LibraryUnitKind::Entity);
    println!(
        "{} files: {} entities, {} architectures, {} packages",
        c.files.len(),
        entities.len(),
        c.units(LibraryUnitKind::Architecture).len(),
        c.units(LibraryUnitKind::Package).len()
    );
    println!(
        "{clean} of {} files analyse with no error; {errors} errors in the other {}",
        c.files.len(),
        per_file.len()
    );
    let mut causes: Vec<_> = per_cause.iter().collect();
    causes.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    println!("errors by cause:");
    for (cause, n) in causes {
        println!("  {n:5}  {cause}");
    }
    let mut worst: Vec<_> = per_file.iter().collect();
    worst.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    println!("errors by file:");
    for (f, n) in worst {
        println!("  {n:5}  {f}");
    }
    // The diagnostics themselves are what a fix starts from, and there are
    // hundreds of them, so they are printed only when asked for.
    if std::env::var_os("CORPUS_DUMP").is_some() {
        println!("{}", c.diags.render(&c.map));
    }

    assert!(entities.len() >= 90, "{} entities", entities.len());
    assert!(
        clean >= 75,
        "only {clean} of {} files analyse",
        c.files.len()
    );
}

#[test]
fn the_entities_elaborate() {
    let Some(root) = root() else { return };
    let c = analyse(&root);
    let entities = c.units(LibraryUnitKind::Entity);

    let mut ok = Vec::new();
    let mut needs_generics = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    for e in &entities {
        let (design, diags) = c.elaborate(e);
        match design {
            Some(_) if !diags.has_errors() => ok.push(e.clone()),
            _ => {
                let first = diags
                    .iter()
                    .find(|d| d.severity == Severity::Error)
                    .map(cause)
                    .unwrap_or_default();
                // An entity whose generics have no defaults cannot be
                // elaborated on its own at all: the width it waits for is
                // the instantiating design's to give. That is not a failure
                // of the frontend, so it is counted apart.
                if first.contains("generic `X` has no value") {
                    needs_generics.push(e.clone());
                } else {
                    if std::env::var_os("CORPUS_DUMP").is_some() {
                        println!("======== {e}\n{}", diags.render(&c.map));
                    }
                    failed.push((e.clone(), first));
                }
            }
        }
    }

    println!(
        "{} of {} entities elaborate from their own defaults; \
         {} need generic values first; {} fail",
        ok.len(),
        entities.len(),
        needs_generics.len(),
        failed.len()
    );
    println!("elaborated: {}", ok.join(", "));
    println!("need generic values: {}", needs_generics.join(", "));
    let mut per_cause: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (name, first) in &failed {
        per_cause.entry(first.clone()).or_default().push(name);
    }
    println!("the first error of each failing entity:");
    for (cause, names) in &per_cause {
        println!("  {:3}  {cause} ({})", names.len(), names.join(", "));
    }

    assert!(ok.len() >= 18, "only {} entities elaborate", ok.len());
}

#[test]
fn every_testbench_needs_a_framework_that_is_not_bundled() {
    let Some(root) = root() else { return };
    let benches = sources(&root.join("sim"));
    assert!(!benches.is_empty(), "no testbenches found");
    let frameworks = [
        "vunit_lib",
        "uvvm_util",
        "uvvm_vvc_framework",
        "osvvm",
        "bitvis_vip",
    ];
    let mut standalone = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &benches {
        let text = fs::read_to_string(f)
            .expect("read source")
            .to_ascii_lowercase();
        let mut any = false;
        for fw in frameworks {
            if text.contains(fw) {
                *counts.entry(fw).or_default() += 1;
                any = true;
            }
        }
        if !any {
            standalone.push(
                f.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    println!("{} files under sim/, by framework named:", benches.len());
    for (fw, n) in &counts {
        println!("  {n:4}  {fw}");
    }
    println!("naming none of them: {}", standalone.join(", "));
    // Every bench in the corpus is written against VUnit, UVVM or OSVVM,
    // none of which Reticle bundles, so not one of them can run here. If
    // this ever stops holding, `docs/vhdl-corpus.md` is out of date and
    // there is a self-checking bench to run.
    assert!(
        standalone.len() <= 1,
        "{} files under sim/ need no framework: {}",
        standalone.len(),
        standalone.join(", ")
    );
}

/// The 8b/10b codec, run in Reticle's simulator.
///
/// `encode_8b10b` and `decode_8b10b` are a matched pair of table-driven
/// codecs: the encoder holds a 1024 x 11-bit ROM as a package constant,
/// picks a code word by data byte, control flag and running disparity, and
/// flips the disparity when the table says to. Nothing in it checks itself,
/// so the check is the code's own defining property: 256 bytes are encoded,
/// every code word must carry four, five or six one bits (the bounded
/// disparity 8b/10b exists for), and feeding those words through the decoder
/// must return the bytes that produced them.
///
/// The stimulus is Rust rather than Colibri's own testbench because every
/// bench in the corpus needs VUnit, UVVM or OSVVM. The *design* is theirs.
#[cfg(feature = "sim")]
#[test]
fn the_8b10b_codec_round_trips_in_the_simulator() {
    use reticle::ir::{Delay, TimeUnit};
    use reticle::logic::Logic;
    use reticle::sim::{SimOptions, Simulator};

    let Some(root) = root() else { return };
    let c = analyse(&root);
    let build = |top: &str| {
        let (design, diags) = c.elaborate(top);
        assert!(!diags.has_errors(), "{}", diags.render(&c.map));
        design.expect("a design")
    };
    let encoder = build("encode_8b10b");
    let decoder = build("decode_8b10b");

    let half = Delay::new(5, TimeUnit::Ns);
    let high = Logic::parse_verilog("1'b1").unwrap();
    let low = Logic::parse_verilog("1'b0").unwrap();
    let bytes: Vec<u32> = (0..256).collect();

    // Encode every byte, back to back. The pipeline is two stages deep, so
    // one more cycle is clocked than there are bytes and the first cycle
    // produces nothing.
    let mut codes = Vec::new();
    {
        let mut sim = Simulator::new(&encoder, SimOptions::default()).unwrap();
        let clk = sim.net("encode_8b10b.clk_i").unwrap();
        let data = sim.net("encode_8b10b.snk_data_i").unwrap();
        let valid = sim.net("encode_8b10b.snk_valid_i").unwrap();
        let control = sim.net("encode_8b10b.snk_control_i").unwrap();
        let out = sim.net("encode_8b10b.src_data_o").unwrap();
        let out_valid = sim.net("encode_8b10b.src_valid_o").unwrap();
        sim.set(clk, low.clone());
        sim.set(valid, high.clone());
        sim.set(control, low.clone());
        for byte in bytes.iter().chain(std::iter::once(&0)) {
            sim.set(data, Logic::parse_verilog(&format!("8'd{byte}")).unwrap());
            sim.set(clk, high.clone());
            sim.run_for_delay(half);
            sim.set(clk, low.clone());
            sim.run_for_delay(half);
            if sim.get(out_valid) == high {
                codes.push(sim.get(out).to_u64().expect("a code word"));
            }
        }
        assert!(
            sim.messages().is_empty(),
            "{}",
            sim.messages().render(&c.map)
        );
    }
    assert_eq!(codes.len(), bytes.len(), "one code word per byte");
    for (byte, code) in bytes.iter().zip(&codes) {
        let ones = code.count_ones();
        assert!(
            (4..=6).contains(&ones),
            "the code for {byte:#04x} is {code:#012b}, which has {ones} one bit(s)"
        );
    }

    // Decode them again; the bytes must come back in order.
    {
        let mut sim = Simulator::new(&decoder, SimOptions::default()).unwrap();
        let clk = sim.net("decode_8b10b.clk_i").unwrap();
        let data = sim.net("decode_8b10b.snk_data_i").unwrap();
        let valid = sim.net("decode_8b10b.snk_valid_i").unwrap();
        let out = sim.net("decode_8b10b.src_data_o").unwrap();
        let out_valid = sim.net("decode_8b10b.src_valid_o").unwrap();
        let control = sim.net("decode_8b10b.src_control_o").unwrap();
        sim.set(clk, low.clone());
        sim.set(valid, high.clone());
        let mut decoded = Vec::new();
        for code in codes.iter().chain(std::iter::once(&0)) {
            sim.set(data, Logic::parse_verilog(&format!("10'd{code}")).unwrap());
            sim.set(clk, high.clone());
            sim.run_for_delay(half);
            sim.set(clk, low.clone());
            sim.run_for_delay(half);
            if sim.get(out_valid) == high {
                assert_eq!(sim.get(control), low, "no code word here is a K code");
                decoded.push(sim.get(out).to_u64().expect("a byte"));
            }
        }
        assert!(
            sim.messages().is_empty(),
            "{}",
            sim.messages().render(&c.map)
        );
        let want: Vec<u64> = bytes.iter().map(|&b| u64::from(b)).collect();
        assert_eq!(decoded, want, "the bytes did not survive the round trip");
    }
    println!("256 bytes encoded and decoded through Colibri's 8b/10b ROMs");
}
