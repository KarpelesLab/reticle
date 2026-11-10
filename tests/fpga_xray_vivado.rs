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
    endpoints_in(root, dir, name, extra, name)
}

/// [`endpoints`], built in a directory of `tag`'s, for two tests of one
/// design to run at once.
fn endpoints_in(root: &str, dir: &Path, name: &str, extra: &[PathBuf], tag: &str) -> String {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("vivado-{tag}"));
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

/// **The analyser's FIFO is as fast as Vivado makes it.** `analyser_fifo.v`
/// is the Husky Plus analyser's FIFO stand-in alone, written at 400 MHz and
/// read at 96. Vivado routes the write-side count into `wused_q[14]` in
/// 1.929 ns, putting `wptr - rptr_w` on the carry chain (a `LUT2` and four
/// `CARRY4`s); `stored` in 4.147 ns; and the word read out, folded into
/// `dout`, in 4.251 ns. Checks each of the three is within 1.35 times
/// Vivado's.
///
/// It was a target until 2026-11-06, `#[ignore]`d: on 2026-11-02 Reticle's
/// estimate of the first was 3.9 ns, a ripple of seven lookup tables,
/// because subtractions were kept off the carry chain
/// (`PrimitiveOptions::min_subtract_carry_width`, off for an unexplained
/// failure on the part). The 7 series' default synthesis (`--synthesis
/// vivado`, docs/fpga-vivado-parity.md, "Synthesis against Vivado's") puts
/// them there, and it passes; it catches the gap opening again. `--synthesis
/// legacy` would still fail it.
#[test]
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
#[ignore = "target, on the legacy model's quoted run: at grade 3 the estimate of Vivado's worst endpoint is out by more than 1.35 on crc32 and gray_cdc; the fitted model meets it, tests/fpga_xray_delay.rs; see docs/fpga-vivado-parity.md"]
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

/// **A carry lane's constant generate is written as its table's `O5`.** This
/// flow takes it from the lower half of the lane's table (`CARRY4.<L>CY0`);
/// written as the constant itself, Vivado tied it to the lane's bypass input
/// instead, which a flip-flop of the lane was using, and stopped at the ground
/// net (`carry_counter`'s equality beside its `depth` register, 2026-11-09).
/// On `carry_counter`, whose equality has such lanes: each `__generate` wire
/// is a `CARRY4`'s `DI` and the output of one constant `LUT1`, `<table>__O5`,
/// on the five-input bel of its table's site, on a pin the table's own cell
/// is locked to. Would catch the constant written again, the half on another
/// site or bel, or on a pin of its own, which Vivado would route a second
/// signal to; would not catch Vivado reading the half otherwise, which only
/// giving it the build asks (`achieved.sh`'s `routes` stage).
#[test]
fn a_carry_lanes_constant_generate_is_written_as_its_tables_lower_half() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vivado-generates");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", &root])
        .args(["--top", "carry_counter", "--constraints"])
        .arg(dir.join("carry_counter.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join("carry_counter.bit"))
        .arg("--vivado-placement")
        .arg(out.join("x.tcl"))
        .arg(dir.join("carry_counter.v"))
        .output()
        .expect("failed to run the reticle binary");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap();
    let (verilog, placed) = (read("x.v"), read("x.tcl"));
    let site_of = |cell: &str| -> String {
        placed
            .lines()
            .find_map(|l| l.strip_prefix(&format!("  {cell} ")))
            .unwrap_or_else(|| panic!("`{cell}` is not placed"))
            .to_owned()
    };
    let locks_of = |cell: &str| -> Vec<String> {
        placed
            .lines()
            .find_map(|l| {
                l.strip_prefix("set_property LOCK_PINS {")
                    .and_then(|r| r.strip_suffix(&format!("}} [get_cells {cell}]")))
            })
            .unwrap_or_else(|| panic!("`{cell}` has no LOCK_PINS"))
            .split_whitespace()
            .map(|pair| pair.split_once(':').unwrap().1.to_owned())
            .collect()
    };
    let mut halves = 0;
    for line in verilog.lines().filter(|l| l.contains("__generate)")) {
        // `  LUT1 #(.INIT(2'h0)) c<n>_<name>__O5 (.I0(<net>), .O(c<n>__generate));`
        let words: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(words[0], "LUT1", "a generate's half is one input: {line}");
        assert!(
            words[1] == "#(.INIT(2'h0))" || words[1] == "#(.INIT(2'h3))",
            "a generate's half is a constant: {line}"
        );
        let half = words[2];
        let table = half
            .strip_suffix("__O5")
            .expect("the half is `<table>__O5`");
        let wire = line
            .rsplit_once(".O(")
            .and_then(|(_, w)| w.strip_suffix("));"))
            .unwrap();
        assert!(
            verilog.contains(&format!("wire {wire};")),
            "{wire} undeclared"
        );
        assert!(
            verilog
                .lines()
                .any(|l| l.trim_start().starts_with("CARRY4") && l.contains(wire)),
            "no carry reads `{wire}`"
        );
        let (here, upper) = (site_of(half), site_of(table));
        let (site, bel) = here.split_once('/').unwrap();
        let (table_site, table_bel) = upper.split_once('/').unwrap();
        assert_eq!(site, table_site, "`{half}` is not on its table's site");
        assert_eq!(
            bel.replace("5LUT", "6LUT"),
            table_bel,
            "`{half}` is not the lower half of its table's bel"
        );
        let pin = &locks_of(half)[0];
        assert!(
            locks_of(table).contains(pin),
            "`{half}` is on {pin}, which its table does not read"
        );
        halves += 1;
    }
    assert!(halves > 0, "no constant generate in\n{verilog}");
}

/// **A carry lane's constant generate is tied on the slice's bypass input as
/// Vivado ties it.** Vivado's routed `probe`s (2026-11-10,
/// `docs/fpga-vivado-parity.md`, "A ground on the bypass input"): a zero on a
/// lane's `DI` is `GND_WIRE -> GFAN<k> -> BYP_ALT<n> -> BYP<n> -> <L>X`, which
/// decodes to the interconnect's `GFAN<k>.GND_WIRE` and `BYP_ALT<n>.GFAN<k>`
/// and nothing else, `<L>CY0` clear. On `carry_counter`, whose equality is
/// compared on the chain in lanes of three bit pairs: the build says how many
/// generates it tied; the bitstream, decoded, has exactly that many
/// `BYP_ALT<n>.GFAN<k>` (the design has no other ground tie) and a
/// `GFAN<k>.GND_WIRE` in each of their tiles; the `CARRY4` that holds them is
/// written for Vivado with the zero on `DI`; and no generate is folded onto
/// its table's `O5`.
///
/// Would catch the tie not made (the generate then reads the `VCC_WIRE` an
/// undriven bypass reads, and every check of the flow's own passes: the router
/// chose no arc there), the generate taken off `O5` again (`<L>CY0` set, a
/// table's lower half, which in a six-input table is no constant), and a
/// ground through a fan into anything but a bypass input; would not catch the
/// tie going to the wrong lane of the slice, which the decoded arcs the flow
/// checks against the router's would.
#[test]
fn a_carry_lanes_constant_generate_is_tied_on_the_bypass_as_vivado_ties_it() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("vivado-bypass-ties");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", &root])
        .args(["--top", "carry_counter", "--report", "--constraints"])
        .arg(dir.join("carry_counter.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join("carry_counter.bit"))
        .arg("--vivado-placement")
        .arg(out.join("x.tcl"))
        .arg(dir.join("carry_counter.v"))
        .output()
        .expect("failed to run the reticle binary");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    let tied: usize = stderr
        .lines()
        .find_map(|l| {
            l.strip_prefix("note: ")?
                .split_once(" carry generate(s) tied to zero")
                .map(|(n, _)| n.parse().unwrap())
        })
        .unwrap_or_else(|| panic!("no generate tied:\n{stderr}"));
    // Eleven lanes of three bit pairs over 32 bits, the `+ 1` on `CYINIT`.
    assert_eq!(tied, 11, "{stderr}");
    assert!(
        stderr.contains(", 11 constant generate(s) tied on the lane's bypass input")
            && !stderr.contains("on their propagate's own table"),
        "the lanes were not left on the bypass, or folded:\n{stderr}"
    );
    let decode = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a100t-ftg256",
            "--chipdb",
            &root,
            "--decode",
        ])
        .arg(out.join("carry_counter.bit"))
        .output()
        .expect("failed to run the reticle binary");
    assert!(decode.status.success());
    let decoded = String::from_utf8_lossy(&decode.stdout);
    let features: Vec<(&str, &str)> = decoded.lines().filter_map(|l| l.split_once(' ')).collect();
    let alts: Vec<&(&str, &str)> = features
        .iter()
        .filter(|(_, f)| f.starts_with("BYP_ALT") && f.contains(".GFAN"))
        .collect();
    assert_eq!(alts.len(), tied, "{alts:?}");
    for (tile, feature) in &alts {
        let fan = feature.split_once('.').unwrap().1;
        assert!(
            features.contains(&(tile, &format!("{fan}.GND_WIRE"))),
            "`{tile}` takes `{feature}` with no ground on `{fan}`"
        );
    }
    // Nothing else of the design reads a ground fan. A fan a signal drives
    // (`GFAN1.BYP_BOUNCE1`, then `IMUX4.GFAN1`) is a hop of its route: with
    // the counter's buffers held on `A6`, the router takes it to lane `A`'s.
    let grounded = |tile: &str, fan: &str| features.contains(&(tile, &format!("{fan}.GND_WIRE")));
    let fanned = features
        .iter()
        .filter(|(t, f)| {
            f.rsplit_once('.')
                .is_some_and(|(_, fan)| matches!(fan, "GFAN0" | "GFAN1") && grounded(t, fan))
        })
        .count();
    assert_eq!(fanned, tied, "a ground fan into something else");
    let verilog = std::fs::read_to_string(out.join("x.v")).unwrap();
    assert!(
        verilog
            .lines()
            .any(|l| l.trim_start().starts_with("CARRY4") && l.contains(".DI({1'b0, 1'b0, 1'b0")),
        "no `CARRY4` written with its generates the zero:\n{verilog}"
    );
}

/// One cell of an exported netlist: `(primitive, parameters, name,
/// connections)`, the connections as `(port, what it is wired to)`.
type NetlistCell = (String, String, String, Vec<(String, String)>);

/// What `--vivado-placement` wrote for one build: the build's messages, the
/// netlist, the placement script and the routes.
struct Export {
    stderr: String,
    verilog: String,
    placed: String,
    routes: String,
}

/// Builds `source` (`top`, constrained by `rcf`) for the Husky Plus's part
/// with `extra` arguments, writing what Vivado is given into `out`.
fn export(root: &str, top: &str, rcf: &Path, source: &Path, extra: &[&str], out: &str) -> Export {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(out);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", "xc7a100t-ftg256", "--chipdb", root])
        .args(["--top", top, "--report", "--constraints"])
        .arg(rcf)
        .args(extra)
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(out.join(format!("{top}.bit")))
        .arg("--vivado-placement")
        .arg(out.join("x.tcl"))
        .arg(source)
        .output()
        .expect("failed to run the reticle binary");
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(run.status.success(), "{stderr}");
    let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap();
    Export {
        stderr,
        verilog: read("x.v"),
        placed: read("x.tcl"),
        routes: read("x.routes.tcl"),
    }
}

impl Export {
    /// Each cell of the netlist: `(primitive, parameters, name, connections)`,
    /// the connections as `(port, what it is wired to)`.
    fn cells(&self) -> Vec<NetlistCell> {
        let mut out = Vec::new();
        for line in self.verilog.lines() {
            let line = line.trim();
            if !line.starts_with(|c: char| c.is_ascii_uppercase()) {
                continue;
            }
            let (head, body) = line.split_once(" (").unwrap();
            let (kind, rest) = head.split_once(' ').unwrap();
            let (params, name) = match rest.rsplit_once(' ') {
                Some((p, n)) => (p.to_owned(), n.to_owned()),
                None => (String::new(), rest.to_owned()),
            };
            let mut ports = Vec::new();
            let mut rest = body;
            while let Some(at) = rest.find('.') {
                rest = &rest[at + 1..];
                let (port, after) = rest.split_once('(').unwrap();
                // A concatenation's braces hold no parentheses.
                let (value, after) = after.split_once(')').unwrap();
                ports.push((port.to_owned(), value.to_owned()));
                rest = after;
            }
            out.push((kind.to_owned(), params, name, ports));
        }
        out
    }

    /// The `<site>/<bel>` a cell is placed on.
    fn site_of(&self, cell: &str) -> Option<&str> {
        self.placed
            .lines()
            .find_map(|l| l.strip_prefix(&format!("  {cell} ")))
    }

    /// A lookup table's `LOCK_PINS`, as `(input, pin)` pairs.
    fn locks_of(&self, cell: &str) -> Vec<(String, String)> {
        self.placed
            .lines()
            .find_map(|l| {
                l.strip_prefix("set_property LOCK_PINS {")
                    .and_then(|r| r.strip_suffix(&format!("}} [get_cells {cell}]")))
            })
            .unwrap_or_default()
            .split_whitespace()
            .map(|pair| {
                let (i, a) = pair.split_once(':').unwrap();
                (i.to_owned(), a.to_owned())
            })
            .collect()
    }

    /// The wires of the route of `net`, as `<tile>/<wire>`.
    fn route_of(&self, net: &str) -> Vec<&str> {
        self.routes
            .lines()
            .find_map(|l| l.strip_prefix(&format!("reticle_route {{{net}}} ")))
            .map(|tree| {
                tree.split_whitespace()
                    .filter(|w| w.contains('/'))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Each `CARRY4`'s propagate bits, lane 0 first.
fn propagates(cells: &[NetlistCell]) -> Vec<Vec<String>> {
    cells
        .iter()
        .filter(|(kind, ..)| kind == "CARRY4")
        .map(|(_, _, _, ports)| {
            let s = &ports.iter().find(|(p, _)| p == "S").unwrap().1;
            let mut bits: Vec<String> = s
                .trim_matches(|c| c == '{' || c == '}')
                .split(',')
                .map(|b| b.trim().to_owned())
                .collect();
            bits.reverse();
            bits
        })
        .collect()
}

/// **A way through a lane's table to its carry propagate is no cell to
/// Vivado.** Vivado's own `carry_counter` (2026.1, routed) takes each counter
/// propagate on the slice's `<L>6` pin, through `<L>6LUT/A6` to `CARRY4/S<k>`,
/// with no cell, the table's `INIT` the upper half (`docs/fpga-vivado-parity.md`,
/// "A way through to the propagate"); this flow's buffer, written as a `LUT1`,
/// was a table more to Vivado, 45 against its 12. On `carry_counter`: no
/// `CARRY4` propagate is read off a buffer table, the 31 of its counter read a
/// flip-flop's output straight, and each of those routes ends on its lane's
/// `<L>6`. The flow's own check of the bitstream (every table cell decodes to
/// what was placed) holds the buffer's truth table, a way through on `A6`, in
/// Reticle's bitstream. With `--synthesis legacy`, whose buffers the router
/// puts on any input, a buffer off `A6` stays a cell: Vivado, given an empty
/// table, goes through it on `A6` and leaves a route to another input hanging
/// (`ANTENNAS` on 9 nets of `carry_counter` seed 1).
///
/// Would catch a buffer written as a cell again, a `CARRY4` reading the
/// buffer's output where no cell drives it, the router moving a buffer off
/// `A6` under the default synthesis, and a buffer off `A6` written as a way
/// through; would not catch Vivado taking the way otherwise, which only giving
/// it the build asks (`achieved.sh`).
#[test]
fn a_way_through_to_a_carry_propagate_is_no_cell_to_vivado() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let (rcf, source) = (dir.join("carry_counter.rcf"), dir.join("carry_counter.v"));
    let built = export(
        &root,
        "carry_counter",
        &rcf,
        &source,
        &[],
        "vivado-way-through",
    );
    assert!(
        built
            .stderr
            .contains("decodes to exactly what was placed and routed"),
        "{}",
        built.stderr
    );
    let cells = built.cells();
    let driver = |net: &str| {
        cells.iter().find(|(_, _, _, ports)| {
            ports
                .iter()
                .any(|(p, v)| (p == "O" || p == "Q") && v == net)
        })
    };
    let mut straight = 0;
    for (carry, bits) in propagates(&cells).iter().enumerate() {
        for (lane, bit) in bits.iter().enumerate() {
            let Some((kind, params, name, _)) = driver(bit) else {
                panic!("CARRY4 {carry} lane {lane} reads `{bit}`, which nothing drives");
            };
            assert!(
                !(kind == "LUT1" && params == "#(.INIT(2'h2))"),
                "`{name}` is a buffer in front of a propagate"
            );
            if kind.starts_with("FD") {
                straight += 1;
                let letter = char::from(b'A' + u8::try_from(lane).unwrap());
                assert!(
                    built
                        .route_of(bit)
                        .iter()
                        .any(|w| w.ends_with(&format!("_{letter}6"))),
                    "`{bit}`, lane {letter}'s propagate, is not routed to `{letter}6`"
                );
            }
        }
    }
    assert_eq!(straight, 31, "the counter's propagates");
    let legacy = export(
        &root,
        "carry_counter",
        &rcf,
        &source,
        &["--synthesis", "legacy"],
        "vivado-way-through-legacy",
    );
    let cells = legacy.cells();
    let mut kept = 0;
    for bits in propagates(&cells) {
        for bit in bits {
            if let Some((kind, params, name, _)) = cells
                .iter()
                .find(|(_, _, _, ports)| ports.iter().any(|(p, v)| p == "O" && *v == bit))
                && kind == "LUT1"
                && params == "#(.INIT(2'h2))"
            {
                let locks = legacy.locks_of(name);
                assert_ne!(locks, [("I0".to_owned(), "A6".to_owned())], "`{name}`");
                kept += 1;
            }
        }
    }
    assert!(kept > 0, "no buffer off `A6` in the legacy build");
}

/// **A flip-flop's way through a table's lower half leaves Vivado the input it
/// came in on and no other.** Vivado picks that input itself as it places the
/// flip-flop, whatever route it is given after, and on `barrel` seed 3 took
/// `B2` where the route ended on `B1` (`ANTENNAS`; 40 builds of 170 had such a
/// net); given one free input, it takes that one. So the placement script
/// rebuilds the table, before anything is placed, with every other free input
/// of `A1` to `A5` tied high, which it ignores (`reticle_hold`, `xray::vivado`'s
/// `hold_the_way`); the netlist keeps the table as it is, since Vivado's own
/// placement of the netlist reads the ties too. On `barrel`, which has such
/// ways: each held table is in the netlist untied with fewer inputs, its held
/// `INIT` the netlist's repeated along each tie, its locked pins leave exactly
/// one of `A1` to `A5` free, and a flip-flop of its lane, on `<L>FF`, has its
/// data routed to that input.
///
/// Would catch the ties left out (nothing held), written into the netlist, one
/// too many or too few (two inputs free, or none), a held table that reads a
/// tie, and the input left free not the route's; would not catch Vivado
/// choosing otherwise, which `achieved.sh` asks.
#[test]
fn a_flip_flops_way_through_a_tables_lower_half_is_held_to_its_input() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let qor = dir.join("qor");
    let built = export(
        &root,
        "barrel",
        &qor.join("barrel.rcf"),
        &qor.join("barrel.v"),
        &[],
        "vivado-held-way",
    );
    let cells = built.cells();
    let mut held = 0;
    for line in built.placed.lines() {
        // `reticle_hold <cell> LUT<k> <2^k>'h<init> {I<n> ...}`
        let Some(rest) = line.strip_prefix("reticle_hold ") else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        let (name, kind, init) = (words[0], words[1], words[2]);
        let ties = rest
            .split_once('{')
            .unwrap()
            .1
            .trim_end_matches('}')
            .split_whitespace()
            .count();
        let k: u32 = kind[3..].parse().unwrap();
        let init = u64::from_str_radix(init.split_once("'h").unwrap().1, 16).unwrap();
        let (written, params, _, ports) = cells
            .iter()
            .find(|(_, _, n, _)| n == name)
            .unwrap_or_else(|| panic!("`{name}` is not in the netlist"));
        let w: u32 = written[3..].parse().unwrap();
        assert_eq!(w + u32::try_from(ties).unwrap(), k, "`{name}`: {line}");
        assert!(
            !ports.iter().any(|(_, v)| v == "1'b1"),
            "`{name}` is tied in the netlist"
        );
        let netlist = params.split_once("'h").unwrap().1.trim_end_matches("))");
        let netlist = u64::from_str_radix(netlist, 16).unwrap();
        assert!(
            (0..1u64 << k).all(|m| (init >> m) & 1 == (netlist >> (m & ((1 << w) - 1))) & 1),
            "`{name}` held reads a tie, or is not the netlist's table: {line}"
        );
        let locks = built.locks_of(name);
        assert_eq!(locks.len(), k as usize, "`{name}`: {locks:?}");
        let used: Vec<&str> = locks.iter().map(|(_, a)| a.as_str()).collect();
        let free: Vec<String> = (1..=5)
            .map(|a| format!("A{a}"))
            .filter(|a| !used.contains(&a.as_str()))
            .collect();
        assert_eq!(free.len(), 1, "`{name}` leaves {free:?} free: {locks:?}");
        let at = built.site_of(name).unwrap();
        let (site, bel) = at.split_once('/').unwrap();
        let letter = &bel[..1];
        let ff = cells
            .iter()
            .find(|(kind, _, ff, _)| {
                kind.starts_with("FD")
                    && built.site_of(ff) == Some(format!("{site}/{letter}FF").as_str())
            })
            .unwrap_or_else(|| panic!("no flip-flop on `{site}/{letter}FF`"));
        let d = &ff.3.iter().find(|(p, _)| p == "D").unwrap().1;
        let pin = format!("_{letter}{}", &free[0][1..]);
        assert!(
            built.route_of(d).iter().any(|w| w.ends_with(&pin)),
            "`{}`'s data `{d}` does not reach `{name}` on {}",
            ff.2,
            free[0]
        );
        held += 1;
    }
    assert!(held > 0, "no way through a table's lower half on `barrel`");
}

/// **A frame's Hamming code names nothing, and is counted apart.** Would catch
/// the range being wrong at either end: bit 12 of word 50 is the code, and bit
/// 13 is not. A lone bit of code on an empty frame is the wrong code, so it is
/// also a fault: being counted apart does not excuse a code from being right.
/// That the code is what Vivado fills in is
/// `the_frame_ecc_is_exact_on_vivados_frames`.
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
    assert_eq!((code.ecc_bits, code.unexplained), (1, 0));
    assert_eq!(code.ecc_faults.len(), 1);
    let other = image(13);
    assert_eq!((other.ecc_bits, other.unexplained), (0, 1));
}

/// **The endpoints file says what the placement estimated, beside what the
/// route made of it** (`estimate` and `estimate_endpoint`, which
/// `tools/vivado-parity/placement_fit.py` reads). On `carry_counter`: an
/// `estimate` line for each `connection` line, the same routed delay, fanout
/// and wire; each placement figure the line `xray::timing::VIVADO_WIRE` gives
/// its distance and fanout, or nothing on a carry's own cascade or a table's
/// way into its lane, which the placement charges nothing; each criticality
/// between nothing and one; and an `estimate_endpoint` line for each
/// `endpoint` line, its routed arrival the same.
///
/// Would catch the placement's estimate written from another model than the
/// one it placed by, the routed column taken from elsewhere, a connection or
/// an endpoint left out, or the distances wrong in sum. Would not catch the
/// columns and rows swapped, which the line, costing a tile alike either way,
/// cannot tell apart.
#[test]
fn the_endpoints_file_says_what_the_placement_estimated() {
    let Some(root) = chipdb() else { return };
    let Some(dir) = fixtures() else { return };
    let report = endpoints_in(&root, &dir, "carry_counter", &[], "estimates");
    let mut connections: Vec<(String, String, String)> = report
        .lines()
        .filter_map(|l| l.strip_prefix("connection "))
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            (w[0].to_owned(), w[1].to_owned(), w[3].to_owned())
        })
        .collect();
    let mut estimated = Vec::new();
    for l in report.lines().filter_map(|l| l.strip_prefix("estimate ")) {
        let w: Vec<&str> = l.split(' ').collect();
        assert_eq!(w.len(), 12, "{l}");
        estimated.push((w[1].to_owned(), w[4].to_owned(), w[11].to_owned()));
        let placed: f64 = w[0].parse().unwrap();
        let tiles = f64::from(w[2].parse::<i32>().unwrap().unsigned_abs())
            + f64::from(w[3].parse::<i32>().unwrap().unsigned_abs());
        let fanout: usize = w[4].parse().unwrap();
        let line = if w[5] == "1" {
            0.0
        } else {
            0.414 + 0.0358 * tiles + if fanout > 4 { 0.0244 * tiles } else { 0.0 }
        };
        assert!(
            (placed - line).abs() < 1.5e-3,
            "{l}: the line says {line:.3}"
        );
        for c in [w[6], w[7]] {
            assert!((0.0..=1.0).contains(&c.parse::<f64>().unwrap()), "{l}");
        }
    }
    assert!(connections.len() > 50, "{} connections", connections.len());
    connections.sort();
    estimated.sort();
    assert_eq!(connections, estimated);
    let mut ends: Vec<(String, String, String)> = report
        .lines()
        .filter_map(|l| l.strip_prefix("endpoint "))
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            (w[0].to_owned(), w[1].to_owned(), w[3].to_owned())
        })
        .collect();
    let mut placed_ends: Vec<(String, String, String)> = report
        .lines()
        .filter_map(|l| l.strip_prefix("estimate_endpoint "))
        .map(|l| {
            let w: Vec<&str> = l.split(' ').collect();
            (w[0].to_owned(), w[3].to_owned(), w[5].to_owned())
        })
        .collect();
    assert!(!ends.is_empty());
    ends.sort();
    placed_ends.sort();
    assert_eq!(ends, placed_ends);
}
