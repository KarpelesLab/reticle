//! Reticle's timing estimate of small designs against Vivado's routed
//! timing of the same designs on the same part.
//!
//! Each design in `testdata/fpga/vivado/` comes with Vivado's constraints
//! (`.xdc`), Reticle's (`.rcf`), the script that ran Vivado (`.tcl`) and an
//! excerpt of Vivado's own report (`.txt`), which is what is compared with.
//! The reports are quoted, not regenerated: Vivado is not needed to run
//! these tests, only the Project X-Ray database, and they skip without it.
//!
//! Each design is the shape of something in NewAE's ChipWhisperer-Husky
//! Plus that this flow got wrong, built small enough to read whole.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The database root, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100tftg256-1/package_pins.csv";
    if let Ok(root) = std::env::var("RETICLE_CHIPDB") {
        if Path::new(&format!("{root}/{probe}")).exists() {
            return Some(root);
        }
        eprintln!("skipped: RETICLE_CHIPDB is `{root}` but has no `{probe}`");
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
                "skipped: needs a Project X-Ray database with the XC7A100T's FTG256 package; \
                 run `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

fn fixtures() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        None
    }
}

/// Builds `<name>.v` and `extra` for the Husky Plus's part with
/// `<name>.rcf`, and returns what `--timing-endpoints` wrote.
fn endpoints(root: &str, dir: &Path, name: &str, extra: &[PathBuf]) -> String {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("vivado-{name}"));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let report = out.join("endpoints.txt");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", root])
        .args(["--top", name, "--timing-driven", "--timing"])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join(format!("{name}.bit")))
        .arg("--timing-endpoints")
        .arg(&report)
        .arg(dir.join(format!("{name}.v")))
        .args(extra)
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::read_to_string(&report).unwrap()
}

/// The steps of the first listed path ending at `end` (`instance.role`):
/// each line's arrival in ns and what it names.
fn path_to(report: &str, end: &str) -> Vec<(f64, String)> {
    let mut lines = report.lines();
    for line in lines.by_ref() {
        if !line.starts_with(' ') && line.ends_with(&format!(" at {end}")) {
            break;
        }
    }
    lines
        .take_while(|l| l.starts_with(' '))
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let ns = words.next()?.parse::<f64>().ok()?;
            (words.next()? == "ns").then(|| (ns, words.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

/// The first `Data Path Delay` after `## <section>` in a Vivado excerpt.
fn vivado_delay(text: &str, section: &str) -> f64 {
    let at = text.find(&format!("## {section}")).expect("the section");
    let line = text[at..]
        .lines()
        .find(|l| l.trim_start().starts_with("Data Path Delay:"))
        .expect("a data path delay");
    line.split_whitespace()
        .nth(3)
        .and_then(|w| w.strip_suffix("ns"))
        .and_then(|w| w.parse().ok())
        .expect("a delay in ns")
}

/// **A carry chain is timed as Vivado times it.** NewAE's analyser counts
/// its samples in a 32-bit register at 400 MHz, `reg_la.capture_count`.
/// `carry_counter.v` is that counter, and Vivado times the path into its
/// top bit at 1.972 ns on the Husky Plus's XC7A100T-3: 0.341 ns out of the
/// register, 0.354 to the chain, 0.509 into the first `CARRY4`, 0.089
/// through each of six more, and 0.234 to the last one's sum, with every
/// hop from one `CARRY4` to the next a net of 0.000.
///
/// This flow puts it at 4.60 ns (2026-11-02): 0.3 ns a cell, and an
/// untimed pip's 0.175 on every hop, which is this flow's own pip from the
/// slice below into the `CARRY4` (`sites::carry_pass_throughs`).
///
/// Checks that the estimate is within 0.8 and 1.35 times Vivado's, and
/// that no hop from a `CARRY4`'s `co3` to the next one's `ci` adds
/// anything. Would catch either part of the model; would not catch a
/// chain mapped worse than Vivado maps it, which is a question for the
/// mapper and not the timing model.
///
/// **A target, not met, and fixing it alone made the board worse.** A
/// model with X-Ray's 0.114 ns along the chain, 0.42 ns more to enter it
/// and nothing on the hop passed this test at 2.40 ns. Built with it, the
/// Husky Plus's seeds 1, 2, 3 and 5 passed 32, 20, 37 and 33 of NewAE's
/// 40 tests against master's 37, 39, 34 and 35, seed 4 did not route, and
/// the ADC clock's routed estimate rose from 6.3-7.4 ns to 7.6-8.2
/// (2026-11-02). So the pessimism stands in for something else the model
/// gets wrong, and the carry is not corrected until that is found.
/// `cargo test -- --ignored` runs it.
#[test]
#[ignore = "target: the model times a carry chain at about twice what Vivado does; correcting it alone cost the Husky Plus on the board"]
fn a_carry_chain_is_timed_as_vivado_times_it() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let vivado = vivado_delay(
        &std::fs::read_to_string(dir.join("carry_counter.txt")).unwrap(),
        "count31.rpt",
    );
    assert!(
        (vivado - 1.972).abs() < 1e-9,
        "the quoted report changed: {vivado}"
    );
    let report = endpoints(&root, &dir, "carry_counter", &[]);
    let path = path_to(&report, "count$ff$ff31.d");
    let Some((ours, _)) = path.last() else {
        panic!("no path to the counter's top bit:\n{report}");
    };
    let ratio = ours / vivado;
    assert!(
        (0.8..=1.35).contains(&ratio),
        "estimated {ours:.2} ns against Vivado's {vivado:.3}:\n{report}"
    );
    // Along the chain: each `ci` arrives when the `co3` before it did.
    for pair in path.windows(2) {
        let ((out, from), (into, to)) = (&pair[0], &pair[1]);
        if from.ends_with(".co3") && to.ends_with(".ci") {
            assert!(
                (into - out).abs() < 0.005,
                "{from} at {out:.2} ns reaches {to} at {into:.2}:\n{report}"
            );
        }
    }
}

/// **Target, not yet met: the analyser's FIFO is as fast as Vivado makes
/// it.** `analyser_fifo.v` is the Husky Plus analyser's FIFO stand-in alone,
/// written at 400 MHz and read at 96. Vivado routes the write-side count
/// into `wused_q[14]` in 1.929 ns, putting `wptr - rptr_w` on the carry
/// chain (a `LUT2` and four `CARRY4`s); `stored` in 4.147 ns; and the word
/// read out, folded into `dout`, in 4.251 ns.
///
/// On 2026-11-02 Reticle's estimate of the first was 3.9 ns, a ripple of
/// seven lookup tables, because subtractions are kept off the carry chain
/// (`PrimitiveOptions::min_subtract_carry_width`, off for an unexplained
/// failure on the part). Checks each of the three is within 1.35 times
/// Vivado's. `cargo test -- --ignored` runs it; it passes when the gap is
/// closed, and would then catch it opening again.
#[test]
#[ignore = "target: subtractions are not on the carry chain, so the FIFO's flags are slower than Vivado's"]
fn the_analyser_fifo_is_as_fast_as_vivado_makes_it() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let quoted = std::fs::read_to_string(dir.join("analyser_fifo.txt")).unwrap();
    let stand_in = dir.join("../../../examples/husky/fifo_generator.v");
    let report = endpoints(&root, &dir, "analyser_fifo", &[stand_in]);
    let mut late = Vec::new();
    for (section, end) in [
        (
            "wused.rpt",
            "u_fifo.u_fifo.async_write_flags.wused_q$ff$ff14.d",
        ),
        ("stored.rpt", "u_fifo.u_fifo.stored$ff.d"),
        ("dout.rpt", "dout$ff.d"),
    ] {
        let vivado = vivado_delay(&quoted, section);
        let Some((ours, _)) = path_to(&report, end).last().cloned() else {
            panic!("no path to {end}:\n{report}");
        };
        if ours / vivado > 1.35 {
            late.push(format!("{end}: {ours:.2} ns against Vivado's {vivado:.3}"));
        }
    }
    assert!(late.is_empty(), "{}", late.join("\n"));
}

/// `testdata/fpga/vivado/placed/vivado.txt`, quoted: what Vivado's timer said of
/// Reticle's implementation of each design, by design and then by line.
fn placed() -> Option<Vec<(String, Vec<String>)>> {
    let dir = fixtures()?;
    let text = std::fs::read_to_string(dir.join("placed/vivado.txt")).unwrap();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            out.push((name.to_owned(), Vec::new()));
        } else if let (Some((_, lines)), true) = (out.last_mut(), line.starts_with("  ")) {
            lines.push(line.trim().to_owned());
        }
    }
    Some(out)
}

/// The words after `key` on the line that starts with it.
fn field<'a>(lines: &'a [String], key: &str) -> Option<Vec<&'a str>> {
    lines
        .iter()
        .find_map(|l| l.strip_prefix(&format!("{key} ")))
        .map(|rest| rest.split_whitespace().collect())
}

/// **Vivado took every route.** In all thirteen designs, every net Reticle
/// routed was accepted as Vivado's `FIXED_ROUTE`, in its own routing graph, and
/// the only designs it could not then finish are the ones named, each for a
/// reason `docs/fpga-vivado-parity.md` gives. Reads the quoted file and runs
/// nothing, so it catches a regeneration that lost something and not a routing
/// that changed since: `tools/vivado-parity/run_placed.sh` is what asks again.
#[test]
fn vivado_took_every_route_reticle_gave_it() {
    let Some(designs) = placed() else { return };
    assert_eq!(designs.len(), 13);
    let mut unfinished = Vec::new();
    for (name, lines) in &designs {
        let count = |key: &str| field(lines, key).map(|w| w[0].parse::<usize>().unwrap());
        assert_eq!(count("routes_refused"), Some(0), "{name}");
        assert!(count("routes_set").unwrap() > 0, "{name}");
        if count("routes_nets_not_routed") != Some(0) {
            unfinished.push(name.as_str());
        }
    }
    assert_eq!(unfinished, ["bram_fifo", "lutram32", "regfile_alu"]);
}

/// Every class of difference in a file `compare_bits.py` wrote that is not in
/// `known`, and how many differences of each side the file counts.
fn unknown_classes(file: &str, known: &[&str]) -> Option<(Vec<String>, usize, usize)> {
    let dir = fixtures()?;
    let text = std::fs::read_to_string(dir.join("placed").join(file)).unwrap();
    let (mut ours, mut theirs) = (0usize, 0usize);
    let mut new = Vec::new();
    for line in text.lines() {
        if line.starts_with("only reticle") {
            ours += line.rsplit(' ').next().unwrap().parse::<usize>().unwrap();
        } else if line.starts_with("only vivado") {
            theirs += line.rsplit(' ').next().unwrap().parse::<usize>().unwrap();
        } else if let Some((count, class)) = line.trim_start().split_once(' ')
            && line.starts_with(' ')
            && count.parse::<usize>().is_ok()
            && !known.iter().any(|k| class.starts_with(k))
        {
            new.push(line.to_owned());
        }
    }
    Some((new, ours, theirs))
}

/// **What Reticle's bitstream and Vivado's disagree on, with the same placement
/// and routes, is a short list of known classes.** A class not named here is a
/// finding nobody has looked at, and so fails. Each named one is answered, or
/// said to be open, in `docs/fpga-vivado-parity.md`. Reads the quoted file: it
/// would catch a regeneration that found something new, and not a Reticle bit
/// that went wrong since the file was written.
#[test]
fn only_known_classes_of_bit_differ_from_vivado() {
    let known = [
        "clock tile: CLK_BUFG_REBUF",
        "clock tile: HCLK_CMT",
        "pad: ",
        "interconnect: a pip",
        "interconnect: a tie to GND_WIRE",
    ];
    let Some((new, ours, theirs)) = unknown_classes("bits.txt", &known) else {
        return;
    };
    assert!(new.is_empty(), "{}", new.join("\n"));
    assert_eq!((ours, theirs), (18, 70), "the file's totals changed");
}

/// **And with only the placement in common, the cells agree too:** the same
/// question with routing left out, which reaches the three designs Vivado would
/// write no bitstream for on Reticle's own routes. As the test above, it reads
/// the quoted file. `#OUTMUX.O6` belongs to Vivado's routes, not to a cell.
#[test]
fn only_known_classes_of_cell_differ_from_vivado() {
    let known = [
        "block RAM: CASCOUT_ARD_ACTIVE",
        "block RAM: ZALMOST_",
        "block RAM: ZINV_",
        "pad: ",
        "slice: #OUTMUX.O6",
        "slice: WEMUX.CE",
    ];
    let Some((new, ours, theirs)) = unknown_classes("cells.txt", &known) else {
        return;
    };
    assert!(new.is_empty(), "{}", new.join("\n"));
    assert_eq!((ours, theirs), (24, 369), "the file's totals changed");
}

/// Reticle's estimate of each connection against Vivado's timer of that same
/// connection, on Reticle's own routes at speed grade 3: the median of all of
/// them, per design. Pins the state of the model as measured, within a stated
/// band and on the quoted file. It would catch a regeneration in which the
/// model's typical wire had moved by a third; it would not catch a wrong path,
/// which is `the_estimate_of_the_worst_endpoint_is_near_vivados`.
#[test]
fn a_typical_connection_is_estimated_within_a_stated_band_of_vivados_timer() {
    let Some(designs) = placed() else { return };
    let mut wrong = Vec::new();
    for (name, lines) in &designs {
        let Some(words) = field(lines, "connections 3") else {
            continue;
        };
        let median: f64 = words[1].parse().unwrap();
        if !(CONNECTION_BAND.0..=CONNECTION_BAND.1).contains(&median) {
            wrong.push(format!("{name}: {median}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join(", "));
}

/// What [`a_typical_connection_is_estimated_within_a_stated_band_of_vivados_timer`]
/// allows: the ratio of Reticle's estimate to Vivado's slow corner.
const CONNECTION_BAND: (f64, f64) = (0.7, 2.2);

/// **Target, not met: the estimate of the path Vivado says is worst is near
/// Vivado's figure for it,** at grade 3 on Reticle's routes, within 1.35 either
/// way as this file's other targets ask. Reads the quoted file. It passes once
/// the file is regenerated from a model that has closed the gap.
#[test]
#[ignore = "target: at grade 3 on Reticle's own routes, the estimate of Vivado's worst endpoint is out by more than 1.35 on crc32 and gray_cdc; see docs/fpga-vivado-parity.md"]
fn the_estimate_of_the_worst_endpoint_is_near_vivados() {
    let Some(designs) = placed() else { return };
    let mut wrong = Vec::new();
    for (name, lines) in &designs {
        let Some(words) = field(lines, "same_endpoint 3") else {
            continue;
        };
        let (ours, theirs): (f64, f64) = (words[0].parse().unwrap(), words[2].parse().unwrap());
        if !(1.0 / 1.35..=1.35).contains(&(ours / theirs)) {
            wrong.push(format!("{name}: {ours} ns against {theirs}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join(", "));
}

/// **`--vivado-placement` writes something `link_design` could read,** on a
/// design with a carry chain, two clocks and paired lookup tables. Every cell
/// has a place to go, with its bel as Vivado spells it; nothing is left of the
/// constructs `link_design` refuses; every route is on a declared net and names
/// no wire Reticle invented inside a slice. Vivado is not run: this catches the
/// writer losing a cell or a net, and not a spelling Vivado would refuse, which
/// is what `run_placed.sh` asks and `placed/vivado.txt` quotes.
#[test]
fn the_vivado_placement_covers_every_cell_and_net() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let qor = dir.join("qor");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vivado-placement");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", &root])
        .args(["--top", "gray_cdc", "--constraints"])
        .arg(qor.join("gray_cdc.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join("gray_cdc.bit"))
        .arg("--vivado-placement")
        .arg(out.join("x.tcl"))
        .arg(qor.join("gray_cdc.v"))
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap();
    let (verilog, placed, routes) = (read("x.v"), read("x.tcl"), read("x.routes.tcl"));
    assert!(
        !verilog.contains("function") && !verilog.contains("assign"),
        "{verilog}"
    );
    assert!(!verilog.contains('$'), "{verilog}");
    let mut cells = 0;
    let mut kinds = std::collections::BTreeSet::new();
    for line in verilog.lines() {
        let mut words = line.split_whitespace();
        let Some(kind) = words.next() else { continue };
        if !kind.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        kinds.insert(kind.to_owned());
        let name = line
            .split(" (")
            .next()
            .and_then(|head| head.rsplit(' ').next())
            .unwrap();
        cells += 1;
        assert!(
            placed.contains(&format!("\n  {name} ")),
            "`{name}` has no place to go:\n{placed}"
        );
    }
    assert!(cells > 40, "{cells} cells in\n{verilog}");
    for kind in ["CARRY4", "FDRE", "BUFG", "IBUF", "OBUF"] {
        assert!(kinds.contains(kind), "no {kind} in {kinds:?}");
    }
    // A pair is two bels of one site, and the upper is the six-input one.
    for bel in ["A6LUT", "CARRY4", "BUFG", "INBUF_EN", "OUTBUF"] {
        assert!(
            placed.contains(&format!("/{bel}\n")),
            "no {bel} in\n{placed}"
        );
    }
    let mut nets = 0;
    for line in routes.lines().filter(|l| l.starts_with("reticle_route")) {
        let net = line.split(['{', '}']).nth(1).unwrap();
        assert!(
            verilog.contains(&format!("wire {net};")) || net.contains('['),
            "the route's net `{net}` is not in the netlist"
        );
        assert!(
            !line.contains("_MUX_") && !line.contains("_CARRY_"),
            "a wire Vivado has no name for: {line}"
        );
        nets += 1;
    }
    assert!(nets > 40, "{nets} routes in\n{routes}");
}

/// **A frame's Hamming code names nothing, and is counted apart.** Would catch
/// the range being wrong at either end: bit 12 of word 50 is the code, and bit
/// 13 is not. That the range is what Vivado fills in is
/// `tools/vivado-parity/frame_ecc.py`'s check, which needs a Vivado bitstream.
#[test]
fn a_frames_own_code_is_not_an_unexplained_bit() {
    use reticle::fpga::xc7::FrameData;
    use reticle::fpga::xray::{XrayDatabase, XrayOptions};
    use reticle::ir::memfile::FileProvider;
    struct DiskFiles;
    impl FileProvider for DiskFiles {
        fn read_file(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(path).ok()
        }
    }
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a100t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let address = part
        .layout
        .order()
        .iter()
        .find_map(|a| *a)
        .expect("a frame");
    let image = |bit: u32| {
        let mut frames =
            FrameData::from_stream(part.layout.clone(), vec![0; part.layout.words()]).unwrap();
        frames.set(address, 50, bit).unwrap();
        db.decode(&DiskFiles, &frames).unwrap()
    };
    let code = image(12);
    assert_eq!((code.frame_ecc, code.unexplained), (1, 0));
    let other = image(13);
    assert_eq!((other.frame_ecc, other.unexplained), (0, 1));
}
