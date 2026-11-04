//! Clock buffers, held to what Vivado wrote for the same designs.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/clocks/` has one small design per question, written
//! here, each asking one parameter of a clock buffer or a clock manager or one
//! wiring of it, and `vivado.txt` has what Vivado 2026.1 made of each, from
//! `tools/vivado-parity/clocks.sh` and `clocks_fixture.py`:
//!
//! - a `site` line for each clock site the bitstream uses, with the features set
//!   on it, read back through this repository's database. Which site of a kind
//!   holds which cell is the placer's, so these are compared as a list without
//!   the sites' names.
//! - a `refused` line in place of them, with the rule Vivado gave.
//!
//! # What these would and would not catch
//!
//! [`each_site_holds_what_vivados_does`] would catch an inverter read the wrong
//! way round, a parameter dropped, a tied pin given the other value and the
//! `ZPRESELECT_I0` a `BUFGCTRL` has and a `BUFG` does not. It would not catch a
//! bit Vivado never set in any design here, or one that means something other
//! than its name: nothing here has been on a board, so all it says is that the
//! bits are Vivado's.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;

use reticle::fpga::xray::{
    Compensation, CounterShape, ManagerShapes, MmcmSettings, PllSettings, check_manager,
    mmcm_registers, pll_registers,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const DEVICE: &str = "xc7a100t-ftg256";

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures() -> PathBuf {
    manifest().join("testdata/fpga/vivado/clocks")
}

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
                "skipped: needs a Project X-Ray database with the FTG256 package; run \
                 `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
            );
            None
        }
    }
}

/// What `vivado.txt` says of one design.
#[derive(Default)]
struct Answer {
    sites: Vec<String>,
    refused: Option<String>,
}

fn answers() -> Option<BTreeMap<String, Answer>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Answer> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            at = name.to_owned();
            out.entry(at.clone()).or_default();
        } else if let Some(site) = line.strip_prefix("  site ") {
            out.get_mut(&at).unwrap().sites.push(site.to_owned());
        } else if let Some(rule) = line.strip_prefix("  refused ") {
            out.get_mut(&at).unwrap().refused = Some(rule.to_owned());
        }
    }
    Some(out)
}

/// The designs of a family, by their names' beginnings.
fn family(answers: &BTreeMap<String, Answer>, prefixes: &[&str]) -> Vec<String> {
    answers
        .keys()
        .filter(|name| prefixes.iter().any(|p| name.starts_with(p)))
        .cloned()
        .collect()
}

/// The families this flow builds, and a few of each that every run of the gate
/// asks about; `every_design_of_a_built_family` asks about the rest.
const FAMILIES: &[&str] = &[
    "gctrl",
    "gmux",
    "ibuf",
    "bufh",
    "bufr_",
    "bufio_",
    "bufmr",
    "no_bufio_",
    "no_bufmr_",
    "mmcm_",
    "pll_",
];
const IN_THE_GATE: &[&str] = &[
    "gctrl_all",
    "gctrl_as_bufg",
    "gctrl_ce0_0",
    "gctrl_ignore1_1",
    "gctrl_init_out",
    "gctrl_inv_s1",
    "gctrl_inv_tied",
    "gctrl_inv_tied0",
    "gctrl_preselect_i0",
    "gctrl_preselect_i1",
    "gmux_plain",
    "gmuxctrl",
    "gmuxctrl_s0",
    "gmuxctrl_s1",
    "ibufds_bufg",
    "ibufgds_bufg",
    "bufh_bufg",
    "bufh_pad",
    "bufh_data",
    "bufhce_ce",
    "bufhce_ce0",
    "bufhce_async",
    "bufhce_init1",
    "bufhce_inv",
    "bufr_plain",
    "bufr_div1",
    "bufr_div8",
    "bufr_both",
    "bufr_fabric",
    "bufr_bufg",
    "bufr_right3",
    "bufio_oddr",
    "bufio_iddr",
    "bufio_loc0",
    "no_bufio_ff",
    "bufmr_bufr",
    "bufmr_bufio",
    "bufmr_fabric",
    "bufmrce_async",
    "bufmrce_s0_inv",
    "no_bufmr_ff",
    "mmcm_base",
    "mmcm_comp_zhold_bufg",
    "mmcm_right_zhold",
    "mmcm_frac_both",
    "mmcm_phase_11p25",
    "mmcm_fine_signal",
    "mmcm_casc4",
    "mmcm_inv_rst",
    "pll_base",
    "pll_comp_zhold_bufg",
    "pll_right_zhold",
    "pll_duty_odd",
    "pll_inv_clkinsel",
];

/// What Vivado builds and this does not yet, which the docs say. Listed so that
/// it is not lost among what is checked.
const NOT_YET: &[&str] = &["mmcm_ss"];

/// What Vivado refuses and this builds: a manager's `CLKIN1_PERIOD` alone out of
/// range, with no clock on its input, which `cmt::configure_clock_managers` says
/// why it does not check.
const LOOSER: &[&str] = &["mmcm_period_alone", "pll_period_alone"];

/// One design through the flow.
struct Built {
    ok: bool,
    log: String,
    /// Each clock site, as a `site` line.
    sites: Vec<String>,
    /// Set bits the database has no name for.
    unnamed: Vec<String>,
}

fn build(root: &str, name: &str) -> Built {
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("clocks")
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--top", name])
        .arg("--constraints")
        .arg(dir.join(format!("{name}.rcf")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .args(["--regional-clocks", "on", "--max-pips", "120000000"])
        .arg(dir.join(format!("{name}.v")))
        .output()
        .expect("the reticle binary runs");
    let log = String::from_utf8_lossy(&run.stderr).into_owned();
    if !run.status.success() {
        return Built {
            ok: false,
            log,
            sites: Vec::new(),
            unnamed: Vec::new(),
        };
    }
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", root, "--decode"])
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    let text = String::from_utf8_lossy(&decoded.stdout).into_owned();
    let (sites, unnamed) = read_sites(&text);
    Built {
        ok: true,
        log,
        sites,
        unnamed,
    }
}

/// The site of a feature, and what it is called on that site.
fn site_of(feature: &str) -> Option<(String, String, &str)> {
    if let Some(rest) = feature
        .strip_prefix("BUFGCTRL.")
        .or_else(|| feature.strip_prefix("BUFHCE."))
        .or_else(|| feature.strip_prefix("BUFIO."))
        .or_else(|| feature.strip_prefix("BUFMRCE."))
    {
        let (site, name) = rest.split_once('.')?;
        let kind = feature.split('.').next()?;
        return Some((site.to_owned(), kind.to_owned(), name));
    }
    if feature.starts_with("BUFR_Y") {
        let (site, name) = feature.split_once('.')?;
        return Some((site.to_owned(), "BUFR".to_owned(), name));
    }
    for kind in ["MMCME2_ADV", "PLLE2_ADV"] {
        if let Some(name) = feature.strip_prefix(kind).and_then(|f| f.strip_prefix('.')) {
            return Some((kind.to_owned(), kind.to_owned(), name));
        }
    }
    None
}

/// A line for each clock site `--decode`'s text sets, and each unnamed bit.
fn read_sites(text: &str) -> (Vec<String>, Vec<String>) {
    let mut sites: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    let mut unnamed = Vec::new();
    for line in text.lines() {
        if let Some(bit) = line.strip_prefix("? ") {
            unnamed.push(bit.to_owned());
            continue;
        }
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        if let Some((site, kind, name)) = site_of(feature) {
            sites
                .entry((tile.to_owned(), site, kind))
                .or_default()
                .push(name.to_owned());
        }
    }
    let mut out: Vec<String> = sites
        .into_iter()
        .map(|((_, _, kind), mut names)| {
            names.sort();
            format!("{kind}: {}", names.join(" "))
        })
        .collect();
    out.sort();
    (plain_rows_once(out), unnamed)
}

/// How many clock rows a global clock passes on `IN_USE` and `ZINV_CE` depends on
/// where its loads went, so that line is the same in any number. So is a plain
/// `BUFG`'s: this flow's router may take a clock through a spare `BUFGCTRL` on its
/// way, which writes that line, and Vivado's does not (`docs/fpga-vivado-parity.md`
/// has it). Each of those is then a line and not a count, so a `BUFG` missing
/// outright is seen only if it was the design's last.
fn plain_rows_once(mut sites: Vec<String>) -> Vec<String> {
    const PLAIN: [&str; 2] = [
        "BUFHCE: IN_USE ZINV_CE",
        "BUFGCTRL: IN_USE IS_IGNORE1_INVERTED ZINV_CE0 ZINV_S0",
    ];
    sites.sort();
    sites.dedup_by(|a, b| a == b && PLAIN.contains(&a.as_str()));
    sites
}

fn build_all(names: Vec<String>) -> Vec<(String, Built)> {
    let Some(root) = chipdb() else {
        return Vec::new();
    };
    std::thread::scope(|scope| {
        let root = &root;
        let handles: Vec<_> = names
            .chunks(names.len().div_ceil(4).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|name| (name.clone(), build(root, name)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

fn compare(built: &[(String, Built)], answers: &BTreeMap<String, Answer>) {
    let mut wrong = Vec::new();
    for (name, one) in built {
        let theirs = &answers[name];
        match (&theirs.refused, one.ok) {
            (Some(_), true) if LOOSER.contains(&name.as_str()) => {}
            (Some(rule), true) => wrong.push(format!(
                "{name}: Vivado refuses it ({rule}), and this built it"
            )),
            (Some(_), false) => {}
            (None, false) if NOT_YET.contains(&name.as_str()) => {}
            (None, false) => wrong.push(format!("{name}: Vivado built it, and:\n{}", one.log)),
            (None, true) => {
                let plain = plain_rows_once(theirs.sites.clone());
                if one.sites != plain {
                    wrong.push(format!(
                        "{name}:\n  ours   {:?}\n  Vivado {:?}",
                        one.sites, plain
                    ));
                }
                if !one.unnamed.is_empty() {
                    wrong.push(format!("{name}: unnamed bits {:?}", one.unnamed));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The gate's designs, built once for what asks about them.
fn gate() -> Option<&'static Vec<(String, Built)>> {
    static BUILT: OnceLock<Option<Vec<(String, Built)>>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            chipdb()?;
            answers()?;
            Some(build_all(
                IN_THE_GATE.iter().map(|s| (*s).to_owned()).collect(),
            ))
        })
        .as_ref()
}

/// **The sites are Vivado's**, on the designs each run of the gate builds. See
/// the module docs for what a line says and what this would catch.
#[test]
fn each_site_holds_what_vivados_does() {
    let Some(built) = gate() else { return };
    compare(built, &answers().unwrap());
}

/// **The same on every design of the families this flow builds.** Takes a
/// bitstream per design, which is why it is not in the gate.
#[test]
#[ignore = "builds every design here to a bitstream, and needs the Project X-Ray database"]
fn every_design_of_a_built_family() {
    let Some(answers) = answers() else { return };
    let built = build_all(family(&answers, FAMILIES));
    compare(&built, &answers);
}

/// **Each design in the gate says it was checked.** `reticle fpga` refuses to
/// finish when the arcs its bits select are not the ones its router chose, and
/// [`each_site_holds_what_vivados_does`] already fails on an unnamed bit; this
/// reads the verdict, so that the check being dropped from the flow is noticed
/// here. Would not catch the check being wrong.
#[test]
fn the_arcs_selected_are_the_routers() {
    let Some(built) = gate() else { return };
    let mut seen = 0;
    // What is refused is the test above's.
    for (name, one) in built.iter().filter(|(_, one)| one.ok) {
        seen += 1;
        assert!(
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{name}: {}",
            one.log
        );
    }
    assert!(seen > 40, "only {seen} designs built");
}

/// **What Vivado refuses of a `BUFGCTRL`, this refuses, and says why.** Would
/// catch either rule being dropped, each of which otherwise builds a clock that
/// never runs, or an inverter Reticle has no bit for.
#[test]
fn a_bufgctrl_vivado_refuses_is_refused() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    for (name, rule, says) in [
        ("gctrl_open", "PDCN-1567", "PDCN-1567"),
        ("gctrl_inv_i0", "PDRC-159", "PDRC-159"),
        ("gctrl_inv_i1", "PDRC-159", "PDRC-159"),
    ] {
        let theirs = answers[name].refused.as_deref().unwrap_or("not refused");
        assert!(theirs.contains(rule), "{name}: the fixture says {theirs}");
        let one = build(&root, name);
        assert!(!one.ok, "{name} built");
        assert!(one.log.contains(says), "{name}: {}", one.log);
    }
}

/// **What Vivado refuses of a `BUFIO`, this refuses, with the rule.** Would catch
/// a rule dropped, or the placer no longer holding one to its bank: each of those
/// otherwise builds a clock that never arrives, whose bitstream decodes perfectly.
#[test]
fn a_bufio_vivado_refuses_is_refused() {
    let Some(root) = chipdb() else { return };
    let Some(answers) = answers() else { return };
    for (name, rule, says) in [
        ("no_bufio_ff", "Route 35-4445", "clocks a bank's IO logic"),
        ("no_bufio_data", "Route 35-4445", "clocks a bank's IO logic"),
        ("no_bufio_bufg", "Route 35-4445", "takes its clock from"),
        ("no_bufio_fabric", "Route 35-4445", "takes its clock from"),
        ("no_bufio_far", "Place 30-126", "Place 30-126"),
    ] {
        let theirs = answers[name].refused.as_deref().unwrap_or("not refused");
        assert!(theirs.contains(rule), "{name}: the fixture says {theirs}");
        let one = build(&root, name);
        assert!(!one.ok, "{name} built");
        assert!(one.log.contains(says), "{name}: {}", one.log);
    }
}

/// **A `BUFR` the global network can stand in for stays on it unless asked**, and
/// the option is what moves it. Would catch `auto` moving the one case the
/// ChipWhisperer-Husky builds with, whose only judge is a board, and `on` not
/// moving it. The first half is a bitstream with no regional site in it.
#[test]
fn a_plain_bufr_stays_on_a_global_buffer_by_default() {
    let Some(root) = chipdb() else { return };
    if answers().is_none() {
        return;
    }
    let dir = fixtures();
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("clocks-auto");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("bufr_plain.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", &root])
        .args(["--top", "bufr_plain", "--constraints"])
        .arg(dir.join("bufr_plain.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("bufr_plain.v"))
        .output()
        .expect("the reticle binary runs");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--chipdb", &root, "--decode"])
        .arg(&bit)
        .output()
        .expect("the reticle binary runs");
    let text = String::from_utf8_lossy(&decoded.stdout);
    let (sites, _) = read_sites(&text);
    assert!(sites.iter().all(|s| !s.starts_with("BUFR")), "{sites:?}");
    let buffers = text
        .lines()
        .filter(|l| l.contains("BUFGCTRL.") && l.ends_with(".IN_USE"))
        .count();
    assert_eq!(buffers, 2, "{text}");
    let on = build(&root, "bufr_plain");
    assert!(
        on.sites.iter().any(|s| s.starts_with("BUFR")),
        "{:?}",
        on.sites
    );
}

/// [`units`], of what is never negative.
fn uunits(text: &str, scale: f64) -> u64 {
    u64::try_from(units(text, scale)).unwrap()
}

/// A real number of a parameter in units of `1 / scale`, as the flow reads it.
#[allow(clippy::cast_possible_truncation)]
fn units(text: &str, scale: f64) -> i64 {
    (text.parse::<f64>().unwrap() * scale).round() as i64
}

/// What one line of `registers.txt` asks: its parameters over the base's.
fn asked(line: &str) -> (bool, BTreeMap<String, String>) {
    let mut words = line.split_whitespace();
    let mmcm = words.next() == Some("mmcm");
    (
        mmcm,
        words
            .filter_map(|w| w.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

/// The registers `mmcm_all_outs` or `pll_all_outs` gets under some parameters.
fn registers_of(mmcm: bool, p: &BTreeMap<String, String>) -> BTreeMap<String, u64> {
    let bases = [8u64, 8, 9, 10, 11, 12, 13, 14];
    let divide = |n: usize| -> u64 {
        p.get(&format!("CLKOUT{n}_DIVIDE"))
            .map_or(bases[n + 1], |v| v.parse().unwrap())
    };
    let shape = |name: &str| CounterShape {
        duty: p
            .get(&format!("{name}_DUTY_CYCLE"))
            .map_or(500_000, |v| uunits(v, 1e6)),
        phase: p.get(&format!("{name}_PHASE")).map_or(0, |v| units(v, 1e3)),
    };
    let mut shapes = ManagerShapes {
        feedback: shape("CLKFBOUT"),
        compensation: Compensation::NotZhold,
        ..ManagerShapes::default()
    };
    for (n, out) in shapes.outputs.iter_mut().enumerate() {
        *out = shape(&format!("CLKOUT{n}"));
    }
    let divclk = p.get("DIVCLK_DIVIDE").map_or(1, |v| v.parse().unwrap());
    let band = p
        .get("BANDWIDTH")
        .map_or("OPTIMIZED", String::as_str)
        .to_owned();
    if mmcm {
        let eighths =
            |name: &str, default: u64| p.get(name).map_or(default * 8, |v| uunits(v, 8.0));
        let feedback = eighths("CLKFBOUT_MULT_F", 8);
        let clkout0 = eighths("CLKOUT0_DIVIDE_F", 8);
        shapes.feedback_eighths = feedback % 8;
        shapes.clkout0_eighths = clkout0 % 8;
        shapes.cascade = p.get("CLKOUT4_CASCADE").is_some();
        let fine = |n: &str| p.get(&format!("{n}_USE_FINE_PS")).is_some();
        let mut outputs = [(1u64, true, false); 7];
        for (n, out) in outputs.iter_mut().enumerate() {
            let name = format!("CLKOUT{n}");
            *out = (
                if n == 0 { clkout0 / 8 } else { divide(n) },
                true,
                fine(&name),
            );
        }
        mmcm_registers(&MmcmSettings {
            divclk_divide: divclk,
            clkfbout_mult: feedback / 8,
            clkfbout_fine_ps: fine("CLKFBOUT"),
            clkfbout_used: true,
            outputs,
            bandwidth: band,
            startup_wait: false,
            invert: [true; 4],
            invert_clkinsel: false,
            shapes,
        })
    } else {
        let more = [1, 2, 3, 4, 5].map(|n| (divide(n), true));
        pll_registers(&PllSettings {
            divclk_divide: divclk,
            clkfbout_mult: p.get("CLKFBOUT_MULT").map_or(8, |v| v.parse().unwrap()),
            clkout0_divide: p.get("CLKOUT0_DIVIDE").map_or(8, |v| v.parse().unwrap()),
            bandwidth: band,
            startup_wait: false,
            clkout0_used: true,
            clkfbout_used: true,
            invert_rst: true,
            invert_pwrdwn: true,
            more_outputs: more,
            invert_clkinsel: false,
            shapes,
        })
    }
}

/// `registers.txt`, or `None` with why not.
fn sweep() -> Option<String> {
    let text = std::fs::read_to_string(fixtures().join("registers.txt")).ok();
    if text.is_none() {
        eprintln!("skipped: `testdata/` is not in the published crate");
    }
    text
}

/// **Every register field is what Vivado wrote, on every run of the sweep**: each
/// `CLKFBOUT_MULT` under each `BANDWIDTH`, each `DIVCLK_DIVIDE`, each output's
/// division, duty cycle and phase, fractional counters, fine phase shift and the
/// cascade. Needs no database: it asks the arithmetic alone, and the designs above
/// ask whether the flow gets here from a design's parameters. Would catch a table
/// entry wrong, a half rounded the wrong way, `EDGE` or `NO_COUNT` misread, a
/// fractional counter's other half in the wrong output's registers and `POWER_REG`
/// wrong for a phase. Skips spread spectrum, which this does not do.
#[test]
fn every_register_is_what_vivado_wrote() {
    let Some(text) = sweep() else { return };
    let mut bases: BTreeMap<bool, BTreeMap<String, u64>> = BTreeMap::new();
    let mut checked = 0usize;
    let mut wrong = Vec::new();
    let read_fields = |text: &str| -> BTreeMap<String, u64> {
        text.split_whitespace()
            .filter_map(|w| w.split_once('='))
            .filter(|(k, _)| *k != "unnamed")
            .map(|(k, v)| {
                let hex = v.trim_start_matches("0x");
                (k.to_owned(), u64::from_str_radix(hex, 16).unwrap())
            })
            .collect()
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("base ") {
            let (kind, fields) = rest.split_once(' ').unwrap();
            bases.insert(kind == "mmcm", read_fields(fields));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (left, right) = line.split_once(" => ").unwrap_or((line, ""));
        if right.starts_with("refused") {
            continue;
        }
        let (mmcm, params) = asked(left);
        // What this flow refuses, with bits Vivado set that nothing names.
        if params.contains_key("SS_EN") || right.contains("unnamed=") {
            continue;
        }
        let mut theirs = bases[&mmcm].clone();
        theirs.extend(read_fields(right));
        theirs.retain(|_, v| *v != 0);
        let mut ours = registers_of(mmcm, &params);
        ours.retain(|_, v| *v != 0);
        checked += 1;
        if ours != theirs {
            let differs: Vec<String> = ours
                .keys()
                .chain(theirs.keys())
                .filter(|k| ours.get(*k) != theirs.get(*k))
                .map(|k| format!("{k}: {:?} / {:?}", ours.get(k), theirs.get(k)))
                .collect();
            wrong.push(format!("{left}: {differs:?}"));
        }
    }
    assert!(checked > 1200, "only {checked} lines");
    assert!(
        wrong.is_empty(),
        "{} of {checked} differ, ours / Vivado:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// **What Vivado refuses of a manager's frequencies and phases, this refuses, and
/// nothing else.** Would catch a window wrong either way: a `VCO` below its range
/// builds a clock that does not lock, and one refused too eagerly a design that
/// Vivado builds. Two lines of the sweep have a refusal it does not give the reason
/// of, which is what `unknown` says.
#[test]
fn what_vivado_refuses_of_a_managers_numbers_is_refused() {
    let Some(text) = sweep() else { return };
    let mut wrong = Vec::new();
    let mut refused = 0usize;
    let mut built = 0usize;
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("base ") {
            continue;
        }
        let (left, right) = line.split_once(" => ").unwrap_or((line, ""));
        let (mmcm, p) = asked(left);
        if p.contains_key("SS_EN") {
            continue;
        }
        let mult = if mmcm {
            p.get("CLKFBOUT_MULT_F").map_or(64, |v| uunits(v, 8.0))
        } else {
            p.get("CLKFBOUT_MULT").map_or(64, |v| uunits(v, 8.0))
        };
        let divide = |n: usize| -> u64 {
            let base = [8u64, 9, 10, 11, 12, 13, 14][n];
            let key = if n == 0 {
                if mmcm {
                    "CLKOUT0_DIVIDE_F"
                } else {
                    "CLKOUT0_DIVIDE"
                }
                .to_owned()
            } else {
                format!("CLKOUT{n}_DIVIDE")
            };
            p.get(&key).map_or(base * 8, |v| uunits(v, 8.0))
        };
        let fine = |n: &str| p.contains_key(&format!("{n}_USE_FINE_PS"));
        let names = [
            "CLKFBOUT", "CLKOUT0", "CLKOUT1", "CLKOUT2", "CLKOUT3", "CLKOUT4", "CLKOUT5", "CLKOUT6",
        ];
        let mut phases: Vec<(&str, u64, i64, bool)> = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let eighths = if index == 0 {
                if p.contains_key(if mmcm {
                    "CLKFBOUT_MULT_F"
                } else {
                    "CLKFBOUT_MULT"
                }) {
                    mult
                } else {
                    64
                }
            } else {
                divide(index - 1)
            };
            let phase = p.get(&format!("{name}_PHASE")).map_or(0, |v| units(v, 1e3));
            phases.push((name, eighths, phase, fine(name)));
        }
        let period = p.get("CLKIN1_PERIOD").map_or(10_000, |v| uunits(v, 1e3));
        let divclk = p.get("DIVCLK_DIVIDE").map_or(1, |v| v.parse().unwrap());
        let has_mult = p.contains_key(if mmcm {
            "CLKFBOUT_MULT_F"
        } else {
            "CLKFBOUT_MULT"
        });
        let ours = check_manager(
            mmcm,
            period,
            divclk,
            if has_mult { mult } else { 64 },
            &phases,
        );
        match (right.starts_with("refused"), ours.is_ok()) {
            (true, false) => refused += 1,
            (false, true) => built += 1,
            (true, true) => wrong.push(format!("{left}: Vivado {right}, this takes it")),
            (false, false) => wrong.push(format!("{left}: Vivado takes it, this {ours:?}")),
        }
    }
    assert!(
        refused > 50 && built > 1200,
        "{refused} refused, {built} built"
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
