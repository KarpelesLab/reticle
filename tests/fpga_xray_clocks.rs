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
//! - a `road` line for each feature of a clock tile between two wires, with its
//!   numbers taken off. A route's own hops are the router's, so this says only
//!   what kind of wire joined what kind, which says where each end was. Compared
//!   as Vivado's list being the same as this flow's when the design is in
//!   [`SAME_ROADS`].
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
const FAMILIES: &[&str] = &["gctrl", "gmux", "ibuf"];
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
];

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
    {
        let (site, name) = rest.split_once('.')?;
        let kind = feature.split('.').next()?;
        return Some((site.to_owned(), kind.to_owned(), name));
    }
    for kind in ["BUFR_Y", "BUFIO_Y", "BUFMRCE_Y"] {
        if feature.starts_with(kind) {
            let (site, name) = feature.split_once('.')?;
            return Some((
                site.to_owned(),
                kind.trim_end_matches("_Y").to_owned(),
                name,
            ));
        }
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

/// How many clock rows a global clock passes on `IN_USE` and `ZINV_CE` depends
/// on where its loads went, and those are the same in any number: a horizontal
/// buffer of a design asking for one is another line, or else the same one
/// whose count does not matter either.
fn plain_rows_once(mut sites: Vec<String>) -> Vec<String> {
    sites.dedup_by(|a, b| a == b && a == "BUFHCE: IN_USE ZINV_CE");
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
            (Some(rule), true) => wrong.push(format!(
                "{name}: Vivado refuses it ({rule}), and this built it"
            )),
            (Some(_), false) => {}
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
    for (name, one) in built {
        assert!(
            one.log
                .contains("the interconnect arcs those bits select are exactly the ones"),
            "{name}: {}",
            one.log
        );
    }
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
