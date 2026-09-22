//! Compiled fast mode against the event simulator.
//!
//! A fast simulator that is subtly wrong is worse than no fast simulator,
//! so the deliverable of `sim::compiled` is as much this file as the
//! engine. Three things happen here:
//!
//! | Test | What it does |
//! |------|--------------|
//! | `the_two_engines_agree_on_every_eligible_design` | runs the *same* design through both engines for thousands of seeded random input vectors, comparing every net and every memory element after every cycle |
//! | `the_eligibility_census_is_the_one_in_the_golden_file` | records, per module of every corpus design, whether it qualifies and if not exactly why |
//! | `cycles_per_second` (`#[ignore]`) | measures both engines on a handful of designs; the numbers in `docs/simulation.md` come from it |
//!
//! The corpora are `testdata/sim/*.rtl`, `testdata/synth/*.rtl` (which
//! includes the `.synth.rtl` and `.cells.rtl` netlists, so the cell form is
//! covered as well as the process form) and, when the Verilog frontend is
//! compiled in, the `ip/` library.
//!
//! # Lining the two engines up
//!
//! Compiled mode is two-state, so before a comparison can mean anything
//! the event simulator has to start from a state the compiled engine can
//! represent: after time zero, every net and every memory element is
//! rewritten with its unknown bits cleared, which is exactly what
//! `zero_init` does on the compiled side. From then on the harness drives
//! the top module's input ports once per cycle and holds them across the
//! clock edge, which is the discipline compiled mode documents for
//! asynchronous resets.
//!
//! Set `UPDATE_EXPECT=1` to rewrite the census golden file.

#![cfg(feature = "sim")]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use reticle::ir::{Design, PortDir};
use reticle::logic::Logic;
use reticle::sim::compiled::{CompileOptions, CompiledSim, Ineligible, ProgramStats, check};
use reticle::sim::{MemHandle, NetHandle, SimOptions, Simulator};
use reticle::source::SourceMap;

// ---------------------------------------------------------------------------
// Support
// ---------------------------------------------------------------------------

/// xorshift64*, so every run uses the same vectors.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A random two-state vector of `width` bits, biased towards the
    /// all-zero and all-one corners a counter or a reset cares about.
    fn bits(&mut self, width: u32) -> Logic {
        match self.next_u64() % 8 {
            0 => Logic::zero(width),
            1 => Logic::ones(width),
            _ => {
                let words = (width as usize).div_ceil(64).max(1);
                let mut out = Logic::zero(width);
                for w in 0..words {
                    let v = self.next_u64();
                    for b in 0..64u32 {
                        let bit = u32::try_from(w).expect("word index fits") * 64 + b;
                        if bit >= width {
                            break;
                        }
                        out.set_bit(bit, reticle::logic::Bit::from_bool((v >> b) & 1 == 1));
                    }
                }
                out
            }
        }
    }
}

fn testdata(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Every `.rtl` file of a corpus directory, sorted.
fn rtl_files(dir: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(testdata(dir))
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rtl"))
        .collect();
    out.sort();
    out
}

fn parse_rtl(path: &Path) -> Option<Design> {
    let text = fs::read_to_string(path).ok()?;
    let mut map = SourceMap::new();
    let file = map.add(path.display().to_string(), &text).ok()?;
    Design::parse_text(&text, file).ok()
}

/// A value with every `x` and `z` bit read as zero.
fn two_state(value: &Logic) -> Logic {
    let words: Vec<u64> = value
        .value_words()
        .iter()
        .zip(value.unknown_words())
        .map(|(v, u)| v & !u)
        .collect();
    let unknown = vec![0u64; words.len()];
    Logic::from_planes(value.width(), value.is_signed(), words, unknown)
}

/// The compiled engine's view of two values being the same: same width,
/// same bits, and nothing unknown on either side. An `x` in the event
/// simulator is a divergence, not an excuse: compiled mode promised there
/// would not be one.
fn same(a: &Logic, b: &Logic) -> bool {
    a.width() == b.width()
        && !a.has_unknown()
        && !b.has_unknown()
        && a.value_words() == b.value_words()
}

/// One design measured or compared.
struct Subject {
    label: String,
    design: Design,
    top: String,
}

fn corpus() -> Vec<Subject> {
    let mut out = Vec::new();
    for dir in ["testdata/sim", "testdata/synth"] {
        for path in rtl_files(dir) {
            let Some(design) = parse_rtl(&path) else {
                continue;
            };
            let file = path
                .file_name()
                .expect("a file")
                .to_string_lossy()
                .into_owned();
            let tops: Vec<String> = design
                .modules
                .iter()
                .map(|(_, m)| m.name.to_string())
                .collect();
            for top in tops {
                out.push(Subject {
                    label: format!("{dir}/{file}:{top}"),
                    design: design.clone(),
                    top,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The equivalence run
// ---------------------------------------------------------------------------

/// What one equivalence run covered.
struct Covered {
    cycles: u64,
    nets: usize,
    mem_elems: usize,
}

/// Runs `top` of `design` through both engines for `cycles` cycles and
/// compares every net and every memory element after each one.
///
/// Returns `Err` with the obstacles when the design does not qualify.
fn compare(subject: &Subject, cycles: u64, seed: u64) -> Result<Covered, Vec<Ineligible>> {
    let options = CompileOptions {
        top: Some(subject.top.clone()),
        zero_init: true,
        ..CompileOptions::default()
    };
    let plan = check(&subject.design, options)?;
    let clock_rising = plan.clock_rising();
    let mut csim = plan.compile();
    let clock = csim.clock();

    let mut sim = Simulator::new(
        &subject.design,
        SimOptions {
            top: Some(subject.top.clone()),
            ..SimOptions::default()
        },
    )
    .expect("the compiled check already elaborated this design");
    sim.run_until(0);

    // Start the event simulator from a state two states can represent,
    // then copy that state into the compiled engine, so both begin the
    // first cycle identical. Copying this way round matters for an
    // asynchronous reset: the event simulator applies one the moment the
    // reset input settles, before any edge, and compiled mode samples it
    // at the edge.
    let nets = sim.nets();
    for (_, h) in &nets {
        let value = two_state(&sim.get(*h));
        sim.set(*h, value);
    }
    let mems: Vec<(String, MemHandle)> = sim.memories();
    let mut mem_elems = 0;
    for (_, m) in &mems {
        for i in 0..sim.mem_len(*m) {
            let index = u64::try_from(i).expect("memory index fits");
            if let Some(v) = sim.get_mem(*m, index) {
                sim.set_mem(*m, index, two_state(&v));
            }
            mem_elems += 1;
        }
    }
    sim.run_for(1);
    for (_, h) in &nets {
        if csim.is_settable(*h) {
            csim.set(*h, two_state(&sim.get(*h)));
        }
    }
    for (i, (_, m)) in mems.iter().enumerate() {
        let handle = csim.memories()[i].1;
        for e in 0..sim.mem_len(*m) {
            let index = u64::try_from(e).expect("memory index fits");
            if let Some(v) = sim.get_mem(*m, index) {
                csim.set_mem(handle, index, two_state(&v));
            }
        }
    }

    // The top module's input ports, minus the clock, are what the
    // testbench drives.
    let top_module = subject
        .design
        .module_by_name(&subject.top)
        .expect("the top exists");
    let m = &subject.design.modules[top_module];
    let inputs: Vec<(NetHandle, u32)> = m
        .ports
        .iter()
        .filter(|p| p.dir == PortDir::In)
        .filter_map(|p| {
            let name = m.nets.get(p.net)?.name.to_string();
            let handle = sim.net(&format!("{}.{name}", subject.top))?;
            Some(handle)
        })
        .filter(|h| Some(*h) != clock)
        .map(|h| {
            let width = sim.get(h).width();
            (h, width)
        })
        .collect();

    let (idle, active) = match clock_rising {
        Some(false) => (Logic::ones(1), Logic::zero(1)),
        _ => (Logic::zero(1), Logic::ones(1)),
    };
    let mut rng = Rng::new(seed);
    let mut ran = 0;
    for cycle in 0..cycles {
        for (h, width) in &inputs {
            let value = rng.bits(*width);
            sim.set(*h, value.clone());
            csim.set(*h, value);
        }
        // Before the edge: the settle has to agree, which is what decides
        // what the edge captures.
        if let Some(clk) = clock {
            sim.set(clk, idle.clone());
            csim.set(clk, idle.clone());
            sim.run_for(1);
        } else {
            sim.run_for(1);
        }
        csim.settle();
        for (name, h) in &nets {
            let want = sim.get(*h);
            if want.width() == 0 {
                continue;
            }
            let got = csim.get(*h);
            assert!(
                same(&want, &got),
                "{}: net {name} diverged before the edge of cycle {cycle}\n  event    = {want}\n  compiled = {got}",
                subject.label
            );
        }
        if let Some(clk) = clock {
            sim.set(clk, active.clone());
            csim.set(clk, active.clone());
        }
        sim.run_for(1);
        csim.step();
        ran = cycle + 1;

        for (name, h) in &nets {
            let want = sim.get(*h);
            if want.width() == 0 {
                continue;
            }
            let got = csim.get(*h);
            assert!(
                same(&want, &got),
                "{}: net {name} diverged at cycle {cycle}\n  event    = {want}\n  compiled = {got}",
                subject.label
            );
        }
        for (name, mh) in &mems {
            for i in 0..sim.mem_len(*mh) {
                let index = u64::try_from(i).expect("memory index fits");
                let want = sim.get_mem(*mh, index).expect("in range");
                let got = csim.get_mem(*mh, index).expect("in range");
                assert!(
                    same(&want, &got),
                    "{}: {name}[{i}] diverged at cycle {cycle}\n  event    = {want}\n  compiled = {got}",
                    subject.label
                );
            }
        }
        if sim.finished() || csim.finished() {
            break;
        }
    }
    Ok(Covered {
        cycles: ran,
        nets: nets.len(),
        mem_elems,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn the_two_engines_agree_on_every_eligible_design() {
    let mut checked = 0;
    let mut cycles = 0u64;
    let mut comparisons = 0u64;
    for (i, subject) in corpus().iter().enumerate() {
        let seed = 0x5EED_0000 + u64::try_from(i).expect("corpus fits");
        match compare(subject, 2_000, seed) {
            Ok(covered) => {
                checked += 1;
                cycles += covered.cycles;
                comparisons += covered.cycles
                    * u64::try_from(covered.nets + covered.mem_elems).expect("counts fit");
            }
            Err(_) => continue,
        }
    }
    assert!(
        checked >= 15,
        "only {checked} corpus designs qualified; the eligibility check has become too strict"
    );
    println!("{checked} designs, {cycles} cycles, {comparisons} value comparisons");
}

#[test]
fn a_long_run_of_random_vectors_stays_identical() {
    // The corpus sweep is broad but shallow; this one is deep. Five
    // thousand cycles each on the designs with the most state.
    for (file, top) in [
        ("testdata/synth/counter_en.rtl", "counter_en"),
        ("testdata/synth/fsm.rtl", "fsm"),
        ("testdata/synth/ram_regread.rtl", "ram_regread"),
        ("testdata/synth/adder_tree.rtl", "adder_tree"),
        ("testdata/synth/casez_priority.rtl", "casez_priority"),
        ("testdata/synth/blocking_ssa.rtl", "blocking_ssa"),
    ] {
        let design = parse_rtl(&testdata(file)).unwrap_or_else(|| panic!("{file} parses"));
        let subject = Subject {
            label: format!("{file}:{top}"),
            design,
            top: top.to_owned(),
        };
        let covered = compare(&subject, 5_000, 0xC0FF_EE01)
            .unwrap_or_else(|p| panic!("{file}:{top} is not eligible: {p:?}"));
        assert!(covered.cycles >= 1);
    }
}

/// Every `.v` file of the in-crate IP library, elaborated together so
/// cross-block instances resolve.
#[cfg(feature = "verilog")]
fn library(top: &str) -> Option<Design> {
    use reticle::diag::Diagnostics;
    use reticle::verilog::{Dialect, ElabOptions, NoIncludes, elaborate, parse_source};
    {
        let root = testdata("ip");
        let mut sources: Vec<(String, String)> = Vec::new();
        let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let rtl = dir.join("rtl");
            let Ok(entries) = fs::read_dir(&rtl) else {
                continue;
            };
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "v"))
                .collect();
            files.sort();
            for f in files {
                let text = fs::read_to_string(&f).ok()?;
                sources.push((f.display().to_string(), text));
            }
        }
        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let mut parsed = Vec::with_capacity(sources.len());
        for (path, text) in &sources {
            let id = map.add(path.clone(), text).ok()?;
            parsed.push(parse_source(
                &mut map,
                id,
                Dialect::Verilog2005,
                &mut NoIncludes,
                &mut diags,
            ));
        }
        if diags.has_errors() {
            return None;
        }
        let options = ElabOptions::new(Dialect::Verilog2005).with_top(top);
        let refs: Vec<_> = parsed.iter().collect();
        let design = elaborate(&refs, &options, &mut diags);
        if diags.has_errors() { None } else { design }
    }
}

/// The blocks worth trying: everything in `ip/` that is not a multi-clock
/// design by construction.
#[cfg(feature = "verilog")]
const BLOCKS: &[&str] = &[
    "uart_tx",
    "uart_rx",
    "uart",
    "fifo_sync",
    "pwm",
    "timer",
    "cdc_sync",
    "ram_sp",
    "ram_sdp",
    "spi_master",
    "i2c_master",
    "axil_gpio",
    "cdc_pulse",
    "fifo_async",
];

#[cfg(feature = "verilog")]
#[test]
fn the_two_engines_agree_on_the_eligible_ip_blocks() {
    let mut qualified = Vec::new();
    for (i, top) in BLOCKS.iter().enumerate() {
        let Some(design) = library(top) else {
            println!("ip {top}: does not elaborate on its own");
            continue;
        };
        let subject = Subject {
            label: format!("ip:{top}"),
            design,
            top: (*top).to_owned(),
        };
        let seed = 0xABCD_0000_u64 + u64::try_from(i).expect("fits");
        match compare(&subject, 2_000, seed) {
            Ok(_) => qualified.push(*top),
            Err(problems) => {
                let mut why: Vec<String> = problems
                    .iter()
                    .map(|p| format!("{}: {}", p.object, p.reason))
                    .collect();
                why.sort();
                why.dedup();
                println!("ip {top}: refused - {}", why.join("; "));
            }
        }
    }
    assert!(
        qualified.len() >= 5,
        "only {} IP blocks qualified for compiled mode",
        qualified.len()
    );
    println!("IP blocks in compiled mode: {}", qualified.join(", "));
}

#[test]
fn the_eligibility_census_is_the_one_in_the_golden_file() {
    let mut report = String::new();
    report.push_str(
        "# Which corpus modules run in compiled fast mode, and why the rest do not.\n\
         # Rewritten by `UPDATE_EXPECT=1 cargo test --features sim eligibility`.\n",
    );
    let mut last_file = String::new();
    for subject in corpus() {
        let (file, top) = subject
            .label
            .rsplit_once(':')
            .expect("the label has a module");
        if file != last_file {
            let _ = writeln!(report, "\n{file}");
            last_file = file.to_owned();
        }
        let options = CompileOptions {
            top: Some(subject.top.clone()),
            zero_init: true,
            ..CompileOptions::default()
        };
        match check(&subject.design, options) {
            Ok(plan) => {
                let s = plan.stats();
                let _ = writeln!(
                    report,
                    "  {top}: eligible ({} comb ops, {} edge ops, {} registers, {} bits)",
                    s.comb_ops, s.seq_ops, s.registers, s.state_bits
                );
            }
            Err(problems) => {
                let mut lines: BTreeSet<String> = problems
                    .iter()
                    .map(|p| format!("{}: {}", p.object, p.reason))
                    .collect();
                let first = lines.pop_first().unwrap_or_default();
                let _ = writeln!(report, "  {top}: refused - {first}");
                for extra in lines {
                    let _ = writeln!(report, "      also {extra}");
                }
            }
        }
    }
    golden("testdata/sim/compiled_eligibility.txt", &report);
}

/// Compares `text` with a golden file, rewriting it under `UPDATE_EXPECT`.
fn golden(rel: &str, text: &str) {
    let path = testdata(rel);
    if std::env::var("UPDATE_EXPECT").is_ok() {
        fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        return;
    }
    let want = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        want,
        text,
        "{} is out of date; rerun with UPDATE_EXPECT=1",
        path.display()
    );
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// The input ports a benchmark drives, with their widths.
fn drivable(subject: &Subject, csim: &CompiledSim<'_>) -> Vec<(NetHandle, u32)> {
    let clock = csim.clock();
    let Some(id) = subject.design.module_by_name(&subject.top) else {
        return Vec::new();
    };
    let m = &subject.design.modules[id];
    let top = &subject.top;
    m.ports
        .iter()
        .filter(|p| p.dir == PortDir::In)
        .filter_map(|p| {
            let net = m.nets.get(p.net)?;
            let handle = csim.net(&format!("{top}.{}", net.name))?;
            Some((handle, net.ty.width().unwrap_or(1)))
        })
        .filter(|(h, _)| Some(*h) != clock)
        .collect()
}

/// Cycles per second for both engines on one design.
fn measure(subject: &Subject, cycles: u64) -> Option<(f64, f64, ProgramStats)> {
    let options = CompileOptions {
        top: Some(subject.top.clone()),
        zero_init: true,
        ..CompileOptions::default()
    };
    let plan = check(&subject.design, options).ok()?;
    let stats = plan.stats();
    let clock_rising = plan.clock_rising();
    let mut csim = plan.compile();
    let clock = csim.clock();
    let inputs = drivable(subject, &csim);

    // One fixed pool of vectors, so both engines see the same stimulus and
    // neither pays for generating it inside the timed loop.
    let mut rng = Rng::new(7);
    let vectors: Vec<Vec<Logic>> = (0..256)
        .map(|_| inputs.iter().map(|(_, w)| rng.bits(*w)).collect())
        .collect();
    let (idle, active) = match clock_rising {
        Some(false) => (Logic::ones(1), Logic::zero(1)),
        _ => (Logic::zero(1), Logic::ones(1)),
    };

    let start = Instant::now();
    for c in 0..cycles {
        let v = &vectors[usize::try_from(c).unwrap_or(0) % vectors.len()];
        for ((h, _), value) in inputs.iter().zip(v) {
            csim.set(*h, value.clone());
        }
        if let Some(clk) = clock {
            csim.set(clk, active.clone());
        }
        csim.step();
    }
    let compiled = start.elapsed().as_secs_f64();

    let mut sim = Simulator::new(
        &subject.design,
        SimOptions {
            top: Some(subject.top.clone()),
            ..SimOptions::default()
        },
    )
    .ok()?;
    sim.run_until(0);
    let handles: Vec<NetHandle> = inputs
        .iter()
        .filter_map(|(h, _)| sim.net(csim.net_name(*h)))
        .collect();
    let sim_clock = clock.and_then(|c| sim.net(csim.net_name(c)));
    let start = Instant::now();
    for c in 0..cycles {
        let v = &vectors[usize::try_from(c).unwrap_or(0) % vectors.len()];
        for (h, value) in handles.iter().zip(v) {
            sim.set(*h, value.clone());
        }
        if let Some(clk) = sim_clock {
            sim.set(clk, idle.clone());
            sim.run_for(1);
            sim.set(clk, active.clone());
        }
        sim.run_for(1);
    }
    let event = start.elapsed().as_secs_f64();
    Some((event, compiled, stats))
}

/// Cycles per second for both engines, on the designs named in
/// `docs/simulation.md`.
///
/// Ignored because it is slow and because a timing number in CI is a
/// flake. Run it with
/// `cargo test --release --features sim,verilog --test sim_compiled --
/// --ignored --nocapture` and paste the table into the document.
#[test]
#[ignore = "a benchmark, not a check"]
fn cycles_per_second() {
    let mut subjects: Vec<(Subject, u64)> = Vec::new();
    for (file, top, cycles) in [
        ("testdata/synth/counter_en.rtl", "counter_en", 1_000_000u64),
        ("testdata/synth/fsm.rtl", "fsm", 1_000_000),
        ("testdata/synth/adder_tree.rtl", "adder_tree", 500_000),
        ("testdata/synth/ram_regread.rtl", "ram_regread", 500_000),
        ("testdata/synth/mux_tree.rtl", "mux_tree", 500_000),
        (
            "testdata/synth/counter_en.cells.rtl",
            "counter_en",
            1_000_000,
        ),
    ] {
        let Some(design) = parse_rtl(&testdata(file)) else {
            continue;
        };
        subjects.push((
            Subject {
                label: file.rsplit('/').next().unwrap_or(file).to_owned(),
                design,
                top: top.to_owned(),
            },
            cycles,
        ));
    }
    #[cfg(feature = "verilog")]
    for (top, cycles) in [
        ("uart_tx", 500_000u64),
        ("uart", 200_000),
        ("fifo_sync", 200_000),
        ("spi_master", 200_000),
        ("i2c_master", 200_000),
        ("axil_gpio", 200_000),
    ] {
        let Some(design) = library(top) else { continue };
        subjects.push((
            Subject {
                label: format!("ip/{top}"),
                design,
                top: top.to_owned(),
            },
            cycles,
        ));
    }

    println!(
        "\n| Design | Program | Event sim | Compiled | Speed-up |\n\
         |--------|---------|-----------|----------|----------|"
    );
    for (subject, cycles) in &subjects {
        let Some((event, compiled, stats)) = measure(subject, *cycles) else {
            println!("| `{}` | not eligible | | | |", subject.label);
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        let n = *cycles as f64;
        println!(
            "| `{}` | {} comb + {} edge ops, {} regs / {} bits | {:.2} M/s | {:.2} M/s | {:.1}x |",
            subject.label,
            stats.comb_ops,
            stats.seq_ops,
            stats.registers,
            stats.state_bits,
            n / event / 1e6,
            n / compiled / 1e6,
            event / compiled
        );
    }
}
