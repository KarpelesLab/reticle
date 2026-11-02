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
