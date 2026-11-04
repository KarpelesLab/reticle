//! Reticle's reading of Vivado's XDC against what Vivado 2026.1 read.
//!
//! `testdata/fpga/vivado/xdc/` holds small constraint files and, beside each, a
//! `.answer`: what Vivado reported holding after reading it, with each query as
//! it resolved it and each line it refused
//! (`tools/vivado-parity/gen_xdc.py`; Vivado is not needed to run this).
//!
//! What this catches: a port bound to another pin than Vivado binds it, a bit
//! of a bus off by its declared range, a glob reaching other objects, a clock
//! at another period, a command Vivado refuses being kept or one it keeps being
//! lost, and a constraint the flow cannot honour going by without a word.
//!
//! What it does not catch: whether the flow then does what the reading says.
//! The pins and clocks of the real designs are held to their `.rcf` below, which
//! is a build's whole output where it has to be, but what the timing engine
//! makes of an exception is `tests/fpga_xray_timing.rs`, and nothing here has
//! been on a board.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use reticle::diag::{Diagnostics, Severity};
use reticle::fpga::xdc::{XDC_NO_EFFECT, XDC_NOT_HONOURED, XDC_REFUSED, XdcDesign, read_xdc};
use reticle::fpga::{Constraints, target};
use reticle::ir::{Design, ModuleId};
use reticle::source::{SourceId, SourceMap};

const DEVICE: &str = "xc7a100t-ftg256";

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> Option<PathBuf> {
    let dir = manifest().join("testdata/fpga/vivado/xdc");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        None
    }
}

fn elaborate(
    map: &mut SourceMap,
    paths: &[PathBuf],
    diags: &mut Diagnostics,
) -> (Design, ModuleId) {
    let mut asts = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).expect("the source is readable");
        let id = map
            .add(path.to_string_lossy().into_owned(), text)
            .expect("the source fits");
        asts.push(reticle::verilog::parse_source(
            map,
            id,
            reticle::verilog::Dialect::Verilog2005,
            &mut reticle::verilog::NoIncludes,
            diags,
        ));
    }
    let refs: Vec<_> = asts.iter().collect();
    let options = reticle::verilog::ElabOptions::default();
    let design =
        reticle::verilog::elaborate(&refs, &options, diags).expect("the design elaborates");
    let top = design.top.expect("a top module");
    (design, top)
}

/// The fixture design, with the one fixture file `case` loaded next to it.
struct Case {
    map: SourceMap,
    design: Design,
    top: ModuleId,
    file: SourceId,
    text: String,
}

fn case(dir: &Path, name: &str) -> Case {
    let mut map = SourceMap::new();
    let mut diags = Diagnostics::new();
    let (design, top) = elaborate(&mut map, &[dir.join("design.v")], &mut diags);
    assert!(!diags.has_errors(), "{}", diags.render(&map));
    let text =
        std::fs::read_to_string(dir.join(format!("{name}.xdc"))).expect("the fixture is there");
    let file = map
        .add(format!("{name}.xdc"), text.clone())
        .expect("it fits");
    Case {
        map,
        design,
        top,
        file,
        text,
    }
}

/// One line of an answer, in the words the reading renders: a clock's `user=`
/// said whether Vivado made it, a place's `bel=` carries the site type Vivado
/// worked out, and a list's order means nothing.
fn normal(line: &str) -> String {
    let words: Vec<String> = line
        .split(' ')
        .filter(|w| !w.starts_with("user="))
        .map(|word| {
            if let Some(bel) = word.strip_prefix("bel=") {
                return format!("bel={}", bel.rsplit('.').next().unwrap_or(bel));
            }
            if word.contains(':') && word.contains(',') {
                let mut items: Vec<&str> = word.split(',').collect();
                items.sort_unstable();
                return items.join(",");
            }
            word.to_owned()
        })
        .collect();
    words.join(" ")
}

/// Vivado's reading of `name`, without what it made by itself (the clocks it
/// derives from a clock manager) and without the lines it refused, which have a
/// test of their own.
fn answer(dir: &Path, name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.answer"))).expect("an answer");
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with("refused ") && !l.starts_with("tile "))
        .filter(|l| !(l.starts_with("clock ") && l.contains("generated=1 user=0")))
        .map(normal)
        .collect();
    lines.sort();
    lines
}

fn cases(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("the fixtures are readable")
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension().is_some_and(|x| x == "xdc"))
                .then(|| path.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
}

/// **Every fixture reads as Vivado read it:** each bit's pin, standard, drive,
/// slew and pull, each clock's period, waveform and source, each pblock and
/// location, each configuration property, and each path constraint Vivado kept
/// with its objects as Vivado resolved them. Would catch any of them wrong, and
/// a command Vivado refuses being kept; see the file's docs for what not.
///
/// Three things in `refused.xdc` are excused, none of them a reading: a pin
/// (`ZZ9`) and a standard (`NONSENSE`) the reader passes to
/// `Constraints::check`, which knows the device and whose error
/// `every_line_vivado_refuses_is_reported` holds, and `SLICE_X500Y0`, which only
/// the fabric database can call outside the device, which
/// `a_pblock_outside_the_device_confines_nothing` holds.
#[test]
fn every_fixture_reads_as_vivado_read_it() {
    let Some(dir) = fixtures() else { return };
    let mut wrong = String::new();
    for name in cases(&dir) {
        let c = case(&dir, &name);
        let view = XdcDesign::of(&c.design, c.top);
        let mut diags = Diagnostics::new();
        let reading = read_xdc(&c.text, c.file, &view, &mut diags);
        let mut ours: Vec<String> = reading.render(&view).lines().map(normal).collect();
        if name == "refused" {
            for line in &mut ours {
                if line.starts_with("pblock dup ") {
                    *line = "pblock dup ranges= cells=".to_owned();
                }
                *line = line
                    .replace("pin=ZZ9", "pin=")
                    .replace("iostandard=NONSENSE", "iostandard=LVCMOS18");
            }
        }
        ours.sort();
        let theirs = answer(&dir, &name);
        for line in theirs.iter().filter(|l| !ours.contains(l)) {
            wrong.push_str(&format!("{name}: Vivado has, Reticle does not: {line}\n"));
        }
        for line in ours.iter().filter(|l| !theirs.contains(l)) {
            wrong.push_str(&format!("{name}: Reticle has, Vivado does not: {line}\n"));
        }
    }
    assert!(wrong.is_empty(), "{wrong}");
}

/// The line a byte offset of `text` is on, from one.
fn line_of(text: &str, offset: u32) -> usize {
    text.as_bytes()[..offset as usize]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// **Every line Vivado refuses is reported here too, and a line whose loss
/// could make a wrong bitstream is an error.** The levels are by what the line
/// is about: anything on a pad or a design's configuration, and any command
/// that is not XDC, must stop the build; a timing or placement command may go
/// on with a warning. Would catch a refused pin read as nothing, or as a
/// warning, and a timing command stopping a build that only needed saying so.
/// Would not catch a line being reported for the wrong reason.
#[test]
fn every_line_vivado_refuses_is_reported() {
    let Some(dir) = fixtures() else { return };
    let c = case(&dir, "refused");
    let mut diags = Diagnostics::new();
    let constraints = Constraints::parse_xdc(&c.text, c.file, &c.design, c.top, &mut diags);
    let device = target(DEVICE).expect("the device is built in");
    constraints.check(&c.design, device, &mut diags);

    let mut severity = vec![None; c.text.lines().count() + 1];
    for diag in diags.iter() {
        for label in &diag.labels {
            if label.span.file == c.file {
                let line = line_of(&c.text, label.span.start);
                let worst = severity[line].get_or_insert(diag.severity);
                if diag.severity == Severity::Error {
                    *worst = Severity::Error;
                }
            }
        }
    }
    let refused = std::fs::read_to_string(dir.join("refused.answer")).expect("an answer");
    let source: Vec<&str> = c.text.lines().collect();
    let mut wrong = String::new();
    for line in refused.lines().filter_map(|l| l.strip_prefix("refused ")) {
        let number: usize = line.split(' ').next().unwrap().parse().unwrap();
        let said = source[number - 1];
        // Only the fabric database can say this one's site is not there.
        if said.starts_with("resize_pblock") {
            continue;
        }
        let must_stop = said.contains("set_property")
            && !said.contains("get_cells")
            && !said.contains("get_nets")
            || !["set_", "create_", "add_", "current_design"]
                .iter()
                .any(|p| said.starts_with(p));
        match severity[number] {
            None => wrong.push_str(&format!("{number}: nothing said of `{said}`\n")),
            Some(Severity::Error) => {}
            Some(_) if must_stop => {
                wrong.push_str(&format!("{number}: only a warning of `{said}`\n"));
            }
            Some(_) => {}
        }
    }
    assert!(wrong.is_empty(), "{wrong}{}", diags.render(&c.map));
}

/// The translation of one fixture, with what was said about it.
fn translation(dir: &Path, name: &str) -> String {
    let c = case(dir, name);
    let mut diags = Diagnostics::new();
    let constraints = Constraints::parse_xdc(&c.text, c.file, &c.design, c.top, &mut diags);
    let device = target(DEVICE).expect("the device is built in");
    constraints.check(&c.design, device, &mut diags);
    let mut out = format!("== {name}\n");
    let mut said: Vec<(usize, String)> = Vec::new();
    for diag in diags.iter() {
        let line = diag
            .labels
            .first()
            .map_or(0, |l| line_of(&c.text, l.span.start));
        let level = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            _ => "note",
        };
        said.push((
            line,
            format!("{level} {} {}", diag.code.unwrap_or("-"), diag.message),
        ));
    }
    said.sort();
    for (line, text) in said {
        out.push_str(&format!("{line}: {text}\n"));
    }
    let mut pins: Vec<String> = constraints
        .pins
        .iter()
        .map(|p| {
            format!(
                "pin {} {:?} {:?} {:?} {:?} {:?}",
                p.signal(),
                p.pin,
                p.io.io_standard,
                p.io.drive,
                p.io.slew,
                (p.io.pullup, p.io.pulldown),
            )
        })
        .collect();
    pins.sort();
    for pin in pins {
        out.push_str(&pin);
        out.push('\n');
    }
    for clock in &constraints.clocks {
        out.push_str(&format!(
            "clock {} on {} period {:.3}\n",
            clock.name, clock.net, clock.period_ns
        ));
    }
    for path in &constraints.false_paths {
        out.push_str(&format!("false_path {:?} {:?}\n", path.from, path.to));
    }
    for multi in &constraints.multicycle_paths {
        out.push_str(&format!(
            "multicycle {} hold={} {:?} {:?}\n",
            multi.cycles, multi.hold, multi.path.from, multi.path.to
        ));
    }
    for delay in &constraints.max_delays {
        out.push_str(&format!(
            "max_delay {:.3} {:?} {:?}\n",
            delay.delay_ns, delay.path.from, delay.path.to
        ));
    }
    for delay in &constraints.input_delays {
        out.push_str(&format!(
            "input_delay {} {:.3} {:?}\n",
            delay.clock, delay.delay_ns, delay.ports
        ));
    }
    for region in &constraints.site_regions {
        out.push_str(&format!(
            "site_region {} {} {} {} {}\n",
            region.name, region.x0, region.y0, region.x1, region.y1
        ));
    }
    for assignment in &constraints.region_assignments {
        out.push_str(&format!(
            "assign {} {}\n",
            assignment.pattern, assignment.region
        ));
    }
    for (pattern, _, _) in &constraints.keep_hierarchy {
        out.push_str(&format!("keep_hierarchy {pattern}\n"));
    }
    out
}

/// **What the flow does with each fixture, and what it says about the rest.**
/// A golden: `translated.txt` has, for each file, every diagnostic by line and
/// the `Constraints` that came out, in the netlist's names. Vivado has no
/// opinion on this; it is here so that a command falling out of the translation
/// or a level changing shows up as a change someone has to look at. Run with
/// `UPDATE_EXPECT=1` to rewrite it.
#[test]
fn the_translation_says_what_it_does() {
    let Some(dir) = fixtures() else { return };
    let mut out = String::new();
    for name in cases(&dir) {
        out.push_str(&translation(&dir, &name));
    }
    let path = dir.join("translated.txt");
    if std::env::var_os("UPDATE_EXPECT").is_some() {
        std::fs::write(&path, &out).expect("the golden is writable");
    }
    let want = std::fs::read_to_string(&path).expect("the golden is there");
    assert_eq!(out, want, "run with UPDATE_EXPECT=1 and read the change");
}

/// **Every command in a fixture is either in what came out or spoken of.** The
/// golden above would take a silent loss as its new truth when rewritten; this
/// would not. For each command the translation has nothing to show for, there
/// must be a diagnostic on its line. Would catch a dropped command with no
/// word, which is what the reader exists not to do.
#[test]
fn nothing_is_dropped_in_silence() {
    let Some(dir) = fixtures() else { return };
    for (name, kinds) in [
        (
            "exceptions",
            [
                "set_false_path",
                "set_multicycle_path",
                "set_max_delay",
                "set_min_delay",
                "set_clock_groups",
            ]
            .as_slice(),
        ),
        ("delays", ["set_input_delay", "set_output_delay"].as_slice()),
        (
            "clocks",
            ["create_clock", "create_generated_clock"].as_slice(),
        ),
    ] {
        let c = case(&dir, name);
        let mut diags = Diagnostics::new();
        let constraints = Constraints::parse_xdc(&c.text, c.file, &c.design, c.top, &mut diags);
        let spoken: Vec<usize> = diags
            .iter()
            .flat_map(|d| d.labels.iter())
            .map(|l| line_of(&c.text, l.span.start))
            .collect();
        let kept: Vec<usize> = constraints
            .false_paths
            .iter()
            .map(|p| p.span)
            .chain(constraints.multicycle_paths.iter().map(|m| m.path.span))
            .chain(constraints.max_delays.iter().map(|m| m.path.span))
            .chain(constraints.input_delays.iter().map(|d| d.span))
            .chain(constraints.clocks.iter().map(|d| d.span))
            .map(|span| line_of(&c.text, span.start))
            .collect();
        for (index, line) in c.text.lines().enumerate() {
            let number = index + 1;
            let command = line.split(' ').next().unwrap_or("");
            if kinds.contains(&command) && !spoken.contains(&number) && !kept.contains(&number) {
                panic!("{name}:{number}: `{line}` is neither kept nor spoken of");
            }
        }
    }
}

/// **The levels are the ones the module documentation promises.** A
/// configuration option that changes what the part does stops the build, one
/// measured to change only how it loads is a warning, and one measured to change
/// no byte is a note. Would catch `UNUSEDPIN` or `STARTUPCLK` passing as a
/// warning, on a bitstream Vivado would make differently; would not catch the
/// measurement going stale, which is `docs/fpga-vivado-parity.md`'s to say.
#[test]
fn configuration_goes_by_what_it_changes() {
    let Some(dir) = fixtures() else { return };
    let c = case(&dir, "config");
    let view = XdcDesign::of(&c.design, c.top);
    let mut diags = Diagnostics::new();
    let _ = read_xdc(&c.text, c.file, &view, &mut diags);
    let mut levels = std::collections::BTreeMap::new();
    for diag in diags.iter() {
        let line = line_of(&c.text, diag.labels[0].span.start);
        let property = c
            .text
            .lines()
            .nth(line - 1)
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap();
        levels.insert(property.to_owned(), diag.code);
    }
    let of = |p: &str| levels.get(p).copied().flatten();
    assert_eq!(of("CFGBVS"), Some(XDC_NO_EFFECT));
    assert_eq!(of("CONFIG_VOLTAGE"), Some(XDC_NO_EFFECT));
    assert_eq!(of("CONFIG_MODE"), Some(XDC_NOT_HONOURED));
    assert_eq!(of("BITSTREAM.GENERAL.COMPRESS"), Some(XDC_NOT_HONOURED));
    assert_eq!(of("BITSTREAM.CONFIG.UNUSEDPIN"), Some(XDC_REFUSED));
    assert_eq!(of("BITSTREAM.STARTUP.STARTUPCLK"), Some(XDC_REFUSED));
}

/// **A real file binds what its `.rcf` binds.** Every design under
/// `testdata/fpga/vivado/` (the two at its top, `qor/` and the 146 of
/// `primitives/`) has both, written apart: each port and bit gets the
/// same pin and the same standard from either, and nothing comes of reading one
/// and the two probes that set a bank's `INTERNAL_VREF` have it read.
/// Would catch a glob such as `[get_ports *]` missing a port, or a port
/// getting a pin its `.rcf` does not give it. The `.rcf`s of `analyser_fifo` and
/// `carry_counter` differ on purpose in their periods and false paths, for
/// another test, so only pins are compared here.
///
/// One difference is the languages': an XDC names only the `P` ball of a
/// differential pair, as Vivado infers the other, and the `.rcf`s here name
/// both. So the second half of a pair goes uncompared; the flow takes the pair
/// from its `P` ball either way, and `an_xdc_makes_the_same_bitstream` holds the
/// result.
#[test]
fn a_real_file_binds_what_its_rcf_binds() {
    if fixtures().is_none() {
        return;
    }
    let root = manifest().join("testdata/fpga/vivado");
    let mut designs: Vec<(PathBuf, Vec<PathBuf>)> = vec![
        (
            root.join("carry_counter"),
            vec![root.join("carry_counter.v")],
        ),
        (
            root.join("analyser_fifo"),
            vec![
                root.join("analyser_fifo.v"),
                manifest().join("examples/husky/fifo_generator.v"),
            ],
        ),
    ];
    for directory in ["qor", "primitives"] {
        let mut names: Vec<PathBuf> = std::fs::read_dir(root.join(directory))
            .expect("the designs are readable")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "xdc"))
            .collect();
        names.sort();
        for xdc in names {
            let stem = xdc.with_extension("");
            let source = stem.with_extension("v");
            designs.push((stem, vec![source]));
        }
    }
    let mut seen = 0usize;
    let mut vref = 0usize;
    for (stem, sources) in designs {
        if !sources.iter().all(|s| s.is_file()) {
            eprintln!("skipped {}: a source is not here", stem.display());
            continue;
        }
        seen += 1;
        let mut map = SourceMap::new();
        let mut diags = Diagnostics::new();
        let (design, top) = elaborate(&mut map, &sources, &mut diags);
        assert!(!diags.has_errors(), "{}", diags.render(&map));
        let xdc_text = std::fs::read_to_string(stem.with_extension("xdc")).unwrap();
        let rcf_text = std::fs::read_to_string(stem.with_extension("rcf")).unwrap();
        let xdc_file = map.add("x.xdc", xdc_text.clone()).unwrap();
        let rcf_file = map.add("x.rcf", rcf_text.clone()).unwrap();
        let mut diags = Diagnostics::new();
        let from_xdc = Constraints::parse_xdc(&xdc_text, xdc_file, &design, top, &mut diags);
        let from_rcf = Constraints::parse(&rcf_text, rcf_file, &mut diags);
        // The two SSTL probes set a bank's `INTERNAL_VREF`, which this refused
        // until a pad's bank rules were measured. `.rcf`s do not say it, in
        // these, so it is not compared.
        let other: Vec<&str> = diags
            .iter()
            .filter(|d| d.severity != Severity::Note)
            .map(|d| d.message.as_str())
            .collect();
        assert!(other.is_empty(), "{}: {other:?}", stem.display());
        vref += from_xdc.internal_vref.len();
        let pins = |c: &Constraints| {
            let mut out: Vec<(String, Option<u32>, String, Option<String>)> = c
                .pins
                .iter()
                .map(|p| {
                    (
                        p.port.clone(),
                        p.bit,
                        p.pin.clone(),
                        p.io.io_standard.clone(),
                    )
                })
                .collect();
            out.sort();
            out
        };
        let (a, mut b) = (pins(&from_xdc), pins(&from_rcf));
        let differential = |standard: &Option<String>| {
            standard.as_deref().is_some_and(|s| {
                [
                    "LVDS",
                    "DIFF_",
                    "TMDS",
                    "BLVDS",
                    "MINI_LVDS",
                    "RSDS",
                    "PPDS",
                ]
                .iter()
                .any(|d| s.starts_with(d))
            })
        };
        for pin in &mut b {
            let unnamed = a
                .iter()
                .any(|x| x.0 == pin.0 && x.1 == pin.1 && x.2.is_empty());
            if unnamed && differential(&pin.3) {
                pin.2.clear();
            }
        }
        assert_eq!(a, b, "{}", stem.display());
    }
    assert!(seen > 2, "the designs were not found");
    assert_eq!(vref, 2, "the SSTL probes' `INTERNAL_VREF` is read");
}

fn run_fpga(constraints: &Path, source: &Path, top: &str, out: &Path, bitstream: bool) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_reticle"));
    command
        .args(["fpga", "--device", DEVICE, "--top", top, "--constraints"])
        .arg(constraints)
        .arg("--output-dir")
        .arg(out)
        .arg(source);
    if bitstream {
        command.arg("--bitstream").arg(out.join("top.bit"));
    }
    let run = command.output().expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{}: {}",
        constraints.display(),
        String::from_utf8_lossy(&run.stderr)
    );
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reticle_fpga_xdc_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// **The command line takes an `.xdc`, and a build from one is a build from the
/// `.rcf`.** For a design whose two files say the same, the netlist `reticle
/// fpga` writes is identical byte for byte, and so is the constraints file it
/// writes for Vivado apart from order. Would catch the option not choosing this
/// reader by the name of the file, or anything in the reading that the flow
/// sees and the `.rcf` does not have. Would not catch a difference only the
/// placer or the bitstream would see; `an_xdc_makes_the_same_bitstream` is that.
#[test]
fn a_build_from_an_xdc_is_a_build_from_the_rcf() {
    if fixtures().is_none() {
        return;
    }
    let qor = manifest().join("testdata/fpga/vivado/qor");
    for name in ["fsm", "crc32"] {
        let source = qor.join(format!("{name}.v"));
        if !source.is_file() {
            eprintln!("skipped: {name} is not here");
            return;
        }
        let (a, b) = (scratch(&format!("{name}_x")), scratch(&format!("{name}_r")));
        run_fpga(&qor.join(format!("{name}.xdc")), &source, name, &a, false);
        run_fpga(&qor.join(format!("{name}.rcf")), &source, name, &b, false);
        let read = |dir: &Path, file: String| std::fs::read_to_string(dir.join(file)).unwrap();
        assert_eq!(
            read(&a, format!("{name}.v")),
            read(&b, format!("{name}.v")),
            "{name}: the netlists differ"
        );
        let sorted = |text: String| {
            let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
            lines.sort();
            lines
        };
        assert_eq!(
            sorted(read(&a, format!("{name}.xdc"))),
            sorted(read(&b, format!("{name}.xdc"))),
            "{name}: the constraints written differ"
        );
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }
}

/// The database root, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let probe = "artix7/tile_type_INT_L.json";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but `{root}/{probe}` is not there");
        return None;
    }
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let cache = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")));
    let dir = cache.map(|c| format!("{c}/prjxray-db/0a0addedd73e7e4139d52a6d8db4258763e0f1f3"));
    match dir {
        Some(dir) if Path::new(&format!("{dir}/{probe}")).is_file() => Some(dir),
        _ => {
            eprintln!(
                "skipped: needs a Project X-Ray database; run `reticle fetch prjxray-db` \
                 or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// **An XDC makes the same bitstream its `.rcf` makes**, byte for byte, on the
/// real thing: placement, routing and the frames. Ignored for the seconds it
/// takes, and it skips without a database.
#[test]
#[ignore = "builds a bitstream twice"]
fn an_xdc_makes_the_same_bitstream() {
    let Some(_) = chipdb() else { return };
    let root = manifest().join("testdata/fpga/vivado");
    // An ordinary design, and a differential pair, whose second ball only the
    // `.rcf` names.
    for (directory, name) in [("qor", "fsm"), ("primitives", "ibufds_probe")] {
        let directory = root.join(directory);
        let source = directory.join(format!("{name}.v"));
        let (a, b) = (
            scratch(&format!("bit_x_{name}")),
            scratch(&format!("bit_r_{name}")),
        );
        run_fpga(
            &directory.join(format!("{name}.xdc")),
            &source,
            name,
            &a,
            true,
        );
        run_fpga(
            &directory.join(format!("{name}.rcf")),
            &source,
            name,
            &b,
            true,
        );
        let read = |dir: &Path| {
            let bytes = std::fs::read(dir.join("top.bit")).unwrap();
            // The file's header says when it was written.
            let at = bytes
                .windows(4)
                .position(|w| w == [0xff, 0xff, 0xff, 0xff])
                .expect("a sync pattern");
            bytes[at..].to_vec()
        };
        assert!(read(&a) == read(&b), "{name}: the bitstreams differ");
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }
}

/// **A pblock outside the device confines nothing, and says so.** Would catch
/// its cells being kept assigned to a region nobody declared, which would stop
/// the build for a reason that is not the one, or being confined to nothing.
#[test]
fn a_pblock_outside_the_device_confines_nothing() {
    let Some(dir) = fixtures() else { return };
    let c = case(&dir, "placement");
    let mut diags = Diagnostics::new();
    let mut constraints = Constraints::parse_xdc(&c.text, c.file, &c.design, c.top, &mut diags);
    let before = constraints.site_regions.len();
    assert!(before > 0);
    let assignments = constraints.region_assignments.len();
    let mut diags = Diagnostics::new();
    // Slices below X10 are in tile (x * 2, y); the rest are not on this device.
    constraints.resolve_site_regions(
        |site| {
            let rest = site.strip_prefix("SLICE_X")?;
            let (x, y) = rest.split_once('Y')?;
            let (x, y): (u32, u32) = (x.parse().ok()?, y.parse().ok()?);
            (x < 10).then_some((x * 2, 100 - y))
        },
        &mut diags,
    );
    assert_eq!(constraints.site_regions.len(), 0);
    let region = constraints
        .regions
        .iter()
        .find(|r| r.name == "pb_sub")
        .expect("pb_sub is inside");
    // Rows run the other way in the grid: the rectangle still has its corners
    // the right way round.
    assert_eq!(
        (region.x0, region.y0, region.x1, region.y1),
        (0, 91, 18, 100)
    );
    assert_eq!(constraints.regions.len(), 1);
    assert_eq!(diags.warning_count(), before - 1);
    assert!(constraints.region_assignments.len() < assignments);
    assert!(
        constraints
            .region_assignments
            .iter()
            .all(|a| a.region == "pb_sub"),
        "{:?}",
        constraints.region_assignments
    );
}

/// **The design's view names what Vivado names.** The two namespaces meet here:
/// a register `t` of `[7:4]` is `t_reg[4]` to Vivado, whose answers above hold
/// that, and bit 0 to the netlist; a port bit of an ascending bus lands on the
/// mirrored bit. Would catch an offset taken from the wrong end, the mistake
/// that gives a wrong bitstream with every line read.
#[test]
fn a_declared_range_reaches_the_netlist() {
    let Some(dir) = fixtures() else { return };
    let c = case(&dir, "ports");
    let mut diags = Diagnostics::new();
    let constraints = Constraints::parse_xdc(&c.text, c.file, &c.design, c.top, &mut diags);
    let pin = |port: &str, bit: u32| {
        constraints
            .pins
            .iter()
            .find(|p| p.port == port && p.bit == Some(bit))
            .map(|p| p.pin.as_str())
    };
    // `input [7:4] hi`: hi[4] is its least significant bit.
    assert_eq!(pin("hi", 0), Some("J4"));
    assert_eq!(pin("hi", 3), Some("J5"));
    // `input [0:3] be`: be[0] is its most significant bit.
    assert_eq!(pin("be", 3), Some("H4"));
    assert_eq!(pin("be", 0), Some("H5"));
    let view = XdcDesign::of(&c.design, c.top);
    let register = |name: &str| {
        view.registers
            .iter()
            .find(|r| r.vivado == name)
            .map(|r| r.netlist.as_str())
    };
    assert_eq!(register("t_reg[4]"), Some("t$ff$ff0"));
    assert_eq!(register("t_reg[7]"), Some("t$ff$ff3"));
    assert_eq!(register("m_reg"), Some("m$ff"));
    assert_eq!(register("u_sub/q_reg"), Some("u_sub.q$ff"));
}

/// **A VHDL port's range is the one it declares**, by the same index a Verilog
/// one would be. Would catch the VHDL front end not recording it, in which case
/// `be(0)` of a `0 to 3` port would land on the bit Verilog calls `be[3]`.
#[test]
fn a_vhdl_range_is_kept() {
    let dir = scratch("vhdl");
    let vhdl = dir.join("t.vhd");
    std::fs::write(
        &vhdl,
        "library ieee; use ieee.std_logic_1164.all;\n\
         entity t is port(\n\
           be : in std_logic_vector(0 to 3);\n\
           hi : in std_logic_vector(7 downto 4);\n\
           y : out std_logic);\n\
         end;\n\
         architecture a of t is begin y <= be(0) and hi(4); end;\n",
    )
    .unwrap();
    let xdc = dir.join("t.xdc");
    std::fs::write(
        &xdc,
        "set_property PACKAGE_PIN H4 [get_ports {be[0]}]\n\
         set_property PACKAGE_PIN J4 [get_ports {hi[4]}]\n\
         set_property PACKAGE_PIN L5 [get_ports y]\n",
    )
    .unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--top", "t", "--constraints"])
        .arg(&xdc)
        .arg("--output-dir")
        .arg(&dir)
        .arg(&vhdl)
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let written = std::fs::read_to_string(dir.join("t.xdc")).unwrap();
    // What it writes for Vivado numbers from zero, as its netlist does.
    assert!(written.contains("H4 [get_ports {be[3]}]"), "{written}");
    assert!(written.contains("J4 [get_ports {hi[0]}]"), "{written}");
    let _ = std::fs::remove_dir_all(dir);
}

/// **A pblock confines the cells it names, on the real fabric, to the tiles
/// Vivado says its corners are in.** The tile of each corner is held to
/// `placement.answer`, and every flip-flop of the register named is placed
/// inside that rectangle, where without the constraint they sit by their pins.
/// Would catch slices taken for tile coordinates, as `write_xdc` writes regions
/// the other way, and a pattern that matches no instance in the netlist, which
/// `check` cannot tell because it runs before mapping. Would not catch a pblock
/// that holds cells of other kinds wrongly, which it has not been tried on.
#[test]
fn a_pblock_confines_what_it_names() {
    use reticle::fpga::place::{PlaceOptions, place};
    use reticle::fpga::xray::{XrayDatabase, XrayOptions};
    use reticle::fpga::{FpgaOptions, Netlist, synthesize_for};
    use reticle::ir::memfile::FileProvider;

    struct DiskFiles;
    impl FileProvider for DiskFiles {
        fn read_file(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(path).ok()
        }
    }

    let Some(dir) = fixtures() else { return };
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    let tiles = db.tiles(&DiskFiles).unwrap();
    let tile_of = |site: &str| {
        tiles
            .iter()
            .find(|t| t.sites.iter().any(|(name, _)| name == site))
    };
    let answers = std::fs::read_to_string(dir.join("placement.answer")).unwrap();
    let mut held = 0;
    for line in answers.lines().filter_map(|l| l.strip_prefix("tile ")) {
        let (site, tile) = line.split_once(' ').unwrap();
        let ours = tile_of(site).unwrap_or_else(|| panic!("{site} is not in the database"));
        assert_eq!(ours.name, tile, "{site}");
        held += 1;
    }
    assert!(held >= 8, "only {held} corners in the answer");

    let verilog = "module top(input clk, input d, output q);\n\
                   reg [7:0] sr;\n\
                   always @(posedge clk) sr <= {sr[6:0], d};\n\
                   assign q = sr[7];\n\
                   endmodule\n";
    let xdc = "set_property -dict {PACKAGE_PIN N11 IOSTANDARD LVCMOS33} [get_ports clk]\n\
               set_property -dict {PACKAGE_PIN L4 IOSTANDARD LVCMOS33} [get_ports d]\n\
               set_property -dict {PACKAGE_PIN L5 IOSTANDARD LVCMOS33} [get_ports q]\n\
               create_clock -period 10 [get_ports clk]\n\
               create_pblock pb\n\
               add_cells_to_pblock [get_pblocks pb] [get_cells {sr_reg[*]}]\n\
               resize_pblock [get_pblocks pb] -add {SLICE_X30Y60:SLICE_X33Y63}\n";
    let mut map = SourceMap::new();
    let source = map.add("top.v", verilog).unwrap();
    let file = map.add("top.xdc", xdc).unwrap();
    let mut diags = Diagnostics::new();
    let ast = reticle::verilog::parse_source(
        &mut map,
        source,
        reticle::verilog::Dialect::Verilog2005,
        &mut reticle::verilog::NoIncludes,
        &mut diags,
    );
    let mut design = reticle::verilog::elaborate_file(
        &ast,
        &reticle::verilog::ElabOptions::default(),
        &mut diags,
    )
    .unwrap();
    let top = design.top.unwrap();
    let mut constraints = Constraints::parse_xdc(xdc, file, &design, top, &mut diags);
    let device = target(DEVICE).unwrap();
    constraints.check(&design, device, &mut diags);
    assert_eq!(
        diags.warning_count() + diags.error_count(),
        0,
        "{}",
        diags.render(&map)
    );
    constraints.resolve_site_regions(
        |site| tile_of(site).map(|t| (t.grid_x, t.grid_y)),
        &mut diags,
    );
    assert_eq!(constraints.regions.len(), 1, "{}", diags.render(&map));
    let rect = constraints.regions[0].clone();

    synthesize_for(
        &mut design,
        top,
        device,
        &constraints,
        &FpgaOptions::default(),
        &mut diags,
    )
    .unwrap();
    assert!(!diags.has_errors(), "{}", diags.render(&map));

    let pins: Vec<String> = constraints.pins.iter().map(|p| p.pin.clone()).collect();
    let around = db.region_for_pins(&DiskFiles, &pins, 12).unwrap().unwrap();
    let mut options = XrayOptions::new();
    options.region = Some(reticle::fpga::xray::GridRegion::new(
        around.x0.min(rect.x0),
        around.y0.min(rect.y0),
        around.x1.max(rect.x1),
        around.y1.max(rect.y1),
    ));
    let fabric = db.load(&DiskFiles, &options).unwrap();
    let graph = fabric.arch.build_graph();
    let netlist = Netlist::build(&design, top, device, &graph).unwrap();
    let (placement, _) = place(
        &netlist,
        &fabric.arch,
        &graph,
        &constraints,
        &PlaceOptions::default(),
    )
    .unwrap();
    let mut inside = 0;
    for (index, instance) in netlist.instances.iter().enumerate() {
        if !instance.name.starts_with("sr$ff") || instance.kind != "ff" {
            continue;
        }
        let site = &graph.sites[placement.site_of(index).expect("it is placed")];
        assert!(
            (rect.x0..=rect.x1).contains(&site.tile.0)
                && (rect.y0..=rect.y1).contains(&site.tile.1),
            "{} is at ({}, {}), outside {rect:?}",
            instance.name,
            site.tile.0,
            site.tile.1
        );
        inside += 1;
    }
    assert_eq!(inside, 8, "the register's flip-flops were not all found");
}

/// **A generated clock on a register is a domain of its own at its divided
/// period.** The placement's estimate names it and its 20 ns, so what it clocks
/// is timed and against the right period. Would catch the clock kept on a net
/// the netlist spells otherwise, in which case its flip-flops are quietly
/// untimed; `the_translation_says_what_it_does` has only the `Constraints`.
#[test]
fn a_generated_clock_on_a_register_is_a_domain() {
    if fixtures().is_none() {
        return;
    }
    let Some(_) = chipdb() else { return };
    let dir = scratch("generated");
    std::fs::write(
        dir.join("g.v"),
        "module g(input clk, input d, output q);\n\
         reg clk_div;\n\
         reg [3:0] on_div;\n\
         always @(posedge clk) clk_div <= ~clk_div;\n\
         always @(posedge clk_div) on_div <= {on_div[2:0], d};\n\
         assign q = on_div[3];\n\
         endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("g.xdc"),
        "set_property -dict {PACKAGE_PIN N11 IOSTANDARD LVCMOS33} [get_ports clk]\n\
         set_property -dict {PACKAGE_PIN L4 IOSTANDARD LVCMOS33} [get_ports d]\n\
         set_property -dict {PACKAGE_PIN L5 IOSTANDARD LVCMOS33} [get_ports q]\n\
         create_clock -period 10 -name sys [get_ports clk]\n\
         create_generated_clock -name g_div -source [get_ports clk] -divide_by 2 \\\n\
             [get_pins clk_div_reg/Q]\n",
    )
    .unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            DEVICE,
            "--top",
            "g",
            "--timing-driven",
            "--constraints",
        ])
        .arg(dir.join("g.xdc"))
        .arg("--output-dir")
        .arg(&dir)
        .arg("--bitstream")
        .arg(dir.join("g.bit"))
        .arg(dir.join("g.v"))
        .output()
        .expect("failed to run the reticle binary");
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{err}");
    assert!(err.contains("of 10 ns"), "{err}");
    assert!(err.contains("g_div") && err.contains("of 20 ns"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}

/// **No text makes the reader panic, or a diagnostic point outside its file.**
/// Every prefix of every fixture, so that each bracket, brace, quote, backslash
/// and `expr` is cut off at each place it could be, is read and rendered. Would
/// catch a slice past the end of the text; would not catch a wrong answer, since
/// a prefix has no answer to be held to.
#[test]
fn no_text_makes_the_reader_panic() {
    let Some(dir) = fixtures() else { return };
    for name in cases(&dir) {
        let c = case(&dir, &name);
        let view = XdcDesign::of(&c.design, c.top);
        for end in (0..=c.text.len()).filter(|e| c.text.is_char_boundary(*e)) {
            let text = &c.text[..end];
            let mut map = SourceMap::new();
            let file = map.add("t.xdc", text).unwrap();
            let mut diags = Diagnostics::new();
            let reading = read_xdc(text, file, &view, &mut diags);
            let _ = reading.constraints(&view, &mut diags);
            let _ = diags.render(&map);
        }
    }
    for text in [
        "\\",
        "set x \\",
        "expr {\"",
        "expr {\"}",
        "nonsense \\",
        "{\\",
        "expr {1 ?",
        "expr {1/0}",
        "$",
        "${",
        "[",
        "]",
        "{",
        "}",
        "\"",
        "set_property -dict {A",
        "get_ports a[",
    ] {
        let mut map = SourceMap::new();
        let file = map.add("t.xdc", text).unwrap();
        let mut diags = Diagnostics::new();
        let _ = read_xdc(text, file, &XdcDesign::default(), &mut diags);
        let _ = diags.render(&map);
    }
}
