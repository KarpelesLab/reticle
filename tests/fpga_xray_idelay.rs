//! An input delay's settings and its controller, held to what Vivado wrote.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/idelay/` has what `tools/vivado-parity/gen_idelay.py`
//! writes: an `IDELAYE2` beside each pad of bank 35, one setting a pad
//! (`index.txt`), delays of the fabric, controllers in one region and two, and
//! the designs Vivado refuses. Vivado 2026.1 built them (`run_all.sh`), and
//! `vivado.txt` is each bitstream decoded through this repository's database, or
//! the rule that refused it. This regenerates it when `RETICLE_IDELAY_BITS`
//! names `run_all.sh`'s output and `UPDATE_EXPECT` is set.
//!
//! An answer has, for each pad of `index.txt`, every `IDELAY` and `ILOGIC`
//! feature of its half, and what each control pin of its delay reads: a ground
//! (`gnd`), a signal, a leaf clock, or nothing at all (`open`, which reads one).
//! And the raw bits of each pad's delay, frames 32 to 35 of its tile, since a
//! bit this flow named decodes to that name whatever the bit is.
//! A delay of the fabric is the same without its place, since Vivado and this
//! flow put it in different tiles, and so is a controller of a design that has
//! one. A controller of a pad's delay is its clock row's tile.
//!
//! # What these would and would not catch
//!
//! [`what_reticle_writes_is_what_vivado_wrote`] builds a few designs in the gate
//! and [`every_design_is_what_vivado_wrote`] all of them: a feature wrong,
//! missing or extra on any delay, a control grounded that Vivado leaves open or
//! the reverse, a controller missing from a region or put in the wrong one. It
//! would not catch which leaf clock a controller's reference takes, or the route
//! of a signal, which are not compared, nor what a delay does on a part, which no
//! test here can. [`what_vivado_refuses_is_refused`] would catch `PLIDC-10`,
//! `PLIDC-7`, `REQP-83`, `REQP-84`, `PDIL-1` or `Route 35-7` let through.
//!
//! The `ddly_*` designs put a delay on an `ISERDESE2`'s `DDLY`, and the
//! `ILOGIC` features compared are then the serialiser's: a mux of `IOBDELAY`
//! missing, or a tie the router took (`CINVCTRL`'s, beside a serialiser),
//! fails. Which of the serialiser's pins are tied is
//! `tests/fpga_xray_ioreg.rs`'s question, not asked again here.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use reticle::fpga::xc7;
use reticle::fpga::xray::{XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a100t-ftg256";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/idelay")
}

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

/// A tile's place, without its type: `X57Y115`.
fn place(tile: &str) -> &str {
    tile.rsplit_once('_').map_or(tile, |(_, at)| at)
}

/// The IO logic half beside each pad of `design` in `index.txt`, as
/// `(place, half)`: `X57Y115`, `'0'` for `G2`. A `_SING` tile's one pad is the
/// half the database names it by (`IO_0_` is `Y1`, `IO_25_` is `Y0`).
fn pads(root: &str, design: &str) -> Vec<(String, char)> {
    let index = std::fs::read_to_string(fixtures().join("index.txt")).unwrap_or_default();
    let balls: Vec<&str> = index
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter(|l| l.split(' ').next() == Some(design))
        .filter_map(|l| l.split(' ').nth(2))
        .collect();
    let package =
        std::fs::read_to_string(format!("{root}/artix7/xc7a100tftg256-1/package_pins.csv"))
            .unwrap();
    let mut out = Vec::new();
    for line in package.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 5 || !balls.contains(&f[0]) {
            continue;
        }
        let first = f[4].contains("IO_25_")
            || f[4]
                .split('_')
                .any(|w| w.starts_with('L') && w.ends_with('P'));
        let half = if !f[4].contains("IO_0_") && first {
            '0'
        } else {
            '1'
        };
        out.push((place(f[3]).to_owned(), half));
    }
    out
}

/// The interconnect wires each control pin of a delay comes in on, as
/// `ppips_lioi3.db` has them.
const PINS: [(&str, &str); 13] = [
    ("C", "CLK1"),
    ("CE", "IMUX32"),
    ("INC", "IMUX26"),
    ("LD", "IMUX30"),
    ("LDPIPEEN", "IMUX33"),
    ("REGRST", "IMUX12"),
    ("CINVCTRL", "BYP_ALT6"),
    ("DATAIN", "IMUX25"),
    ("CNTVALUEIN0", "IMUX41"),
    ("CNTVALUEIN1", "IMUX36"),
    ("CNTVALUEIN2", "IMUX35"),
    ("CNTVALUEIN3", "IMUX38"),
    ("CNTVALUEIN4", "IMUX39"),
];

/// What feeds `wire` of the interconnect tile at `(x, y)`: `gnd` for a ground
/// fan, or a bypass that comes from one, `leaf` for a global clock, `signal`
/// for anything else and `open` for nothing.
fn class(
    int: &BTreeMap<(String, u32), BTreeMap<String, String>>,
    x: &str,
    y: u32,
    wire: &str,
) -> &'static str {
    let at = |y: u32| int.get(&(x.to_owned(), y));
    let Some(source) = at(y).and_then(|t| t.get(wire)) else {
        return "open";
    };
    // A ground fan carries a ground, or a signal it was taken for as a hop.
    let grounded = |y: u32, fan: &str| {
        at(y)
            .and_then(|t| t.get(fan))
            .is_some_and(|s| s == "GND_WIRE")
    };
    if source.starts_with("GFAN") {
        return if grounded(y, source) { "gnd" } else { "signal" };
    }
    if source.contains("GCLK") {
        return "leaf";
    }
    // A bypass's bounce: from this row's `BYP_ALT<k>`, or the row below's.
    if let Some(rest) = source.strip_prefix("BYP_BOUNCE") {
        let (row, k) = match rest.strip_prefix("_N3_") {
            Some(k) => (y.wrapping_sub(1), k),
            None => (y, rest),
        };
        let fed = at(row).and_then(|t| t.get(&format!("BYP_ALT{k}")));
        if fed.is_some_and(|s| s.starts_with("GFAN") && grounded(row, s)) {
            return "gnd";
        }
    }
    "signal"
}

/// A bitstream's answer for `design`; see the module documentation.
fn answer_of(root: &str, design: &str, bit: &Path) -> Vec<String> {
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(bit).unwrap()).unwrap();
    let frames = read.image(part.layout.clone()).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    let keep = pads(root, design);
    // Every interconnect tile's pips, by `(X, Y)` and wire, `_L` spellings
    // made the same.
    let mut int: BTreeMap<(String, u32), BTreeMap<String, String>> = BTreeMap::new();
    for (tile, f) in &decoded.features {
        if !tile.starts_with("INT_") {
            continue;
        }
        let Some((x, y)) = place(tile)
            .strip_prefix('X')
            .and_then(|r| r.split_once('Y'))
            .and_then(|(x, y)| Some((x.to_owned(), y.parse::<u32>().ok()?)))
        else {
            continue;
        };
        let Some((to, from)) = f.split_once('.') else {
            continue;
        };
        let to = to.replace("IMUX_L", "IMUX").replace("CLK_L", "CLK");
        int.entry((x, y)).or_default().insert(to, from.to_owned());
    }
    let mut out = Vec::new();
    // Each delay in use, at a pad or of the fabric.
    let delays: BTreeSet<(String, String, char)> = decoded
        .features
        .iter()
        .filter(|(tile, f)| {
            tile.contains("IOI3") && f.starts_with("IDELAY_Y") && f.ends_with(".IN_USE")
        })
        .map(|(tile, f)| {
            (
                tile.clone(),
                place(tile).to_owned(),
                f.as_bytes()[8] as char,
            )
        })
        .collect();
    // Of the fabric: its source is `DATAIN`.
    let fabric = |tile: &str, h: char| {
        decoded
            .features
            .iter()
            .any(|(t, f)| t == tile && *f == format!("IDELAY_Y{h}.DELAY_SRC_DATAIN"))
    };
    for (tile, at, half) in &delays {
        let single = tile.contains("SING");
        let name = if fabric(tile, *half) {
            "fabric".to_owned()
        } else {
            format!("{at}/{half}")
        };
        for (t, f) in &decoded.features {
            if t != tile {
                continue;
            }
            for prefix in ["IDELAY_Y", "ILOGIC_Y"] {
                if let Some(rest) = f.strip_prefix(prefix)
                    && rest.starts_with(*half)
                {
                    out.push(format!("{name} {}.{}", &prefix[..6], &rest[2..]));
                }
            }
        }
        let (x, y) = at[1..].split_once('Y').unwrap();
        let y: u32 = y.parse().unwrap();
        // Half 0's controls come from the row above, except in a `_SING` tile.
        let row = if *half == '0' && !single { y + 1 } else { y };
        for (pin, wire) in PINS {
            out.push(format!("{name} pin {pin} {}", class(&int, x, row, wire)));
        }
    }
    // And the raw bits of each pad's delay, frames 32 to 35 of its IO logic
    // tile, where the delays' features are: a name this flow gave a bit
    // (`IS_C_INVERTED`) decodes to itself whatever the bit, so without these a
    // wrong one would agree. Not for a delay of the fabric, whose tile differs.
    let tiles = db.tiles(&DiskFiles).unwrap();
    for (tile, at, half) in &delays {
        if fabric(tile, *half) {
            continue;
        }
        let Some((_, window)) = tiles
            .iter()
            .find(|t| t.name == *tile)
            .and_then(|t| t.bits.first())
        else {
            continue;
        };
        let mut set = Vec::new();
        for row in 32..36 {
            for col in 0..window.words * 32 {
                let bit = reticle::fpga::arch::ConfigBit::new(row, col);
                let Some((address, word, index)) = window.locate(bit) else {
                    continue;
                };
                if frames.get(address, word, index).unwrap_or(false) {
                    set.push(format!("{row}_{col:02}"));
                }
            }
        }
        out.push(format!("{at}/{half} raw {}", set.join(" ")));
    }
    // A pad of the index with no delay in use: nothing, said.
    for (at, half) in &keep {
        if !delays.iter().any(|(_, p, h)| p == at && h == half) {
            out.push(format!("{at}/{half} no delay"));
        }
    }
    // The controllers: a clock row's tile with its reference clock routed, and
    // its reset, which comes from the IO logic tile above it.
    let controllers: Vec<&String> = decoded
        .features
        .iter()
        .filter(|(_, f)| f.starts_with("HCLK_IOI_IDELAYCTRL_REFCLK."))
        .map(|(tile, _)| tile)
        .collect();
    let floating = delays.iter().any(|(tile, _, half)| fabric(tile, *half));
    if floating {
        out.push(format!("controllers {}", controllers.len()));
    } else {
        for tile in controllers {
            out.push(format!("controller {}", place(tile)));
        }
    }
    // A `DCIRESET`: its feature, and what its reset reads, which on this part
    // comes from `IMUX35` of `INT_L_X18Y78`.
    for (tile, f) in &decoded.features {
        if tile.starts_with("CFG_CENTER") && f.starts_with("DCIRESET.") {
            out.push(format!("dci {f}"));
            out.push(format!("dci pin RST {}", class(&int, "18", 78, "IMUX35")));
        }
    }
    out.sort();
    out
}

/// `vivado.txt`: per design, `refused <rule>` or its lines.
fn answers() -> Option<BTreeMap<String, Result<Vec<String>, String>>> {
    let Ok(text) = std::fs::read_to_string(fixtures().join("vivado.txt")) else {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return None;
    };
    let mut out: BTreeMap<String, Result<Vec<String>, String>> = BTreeMap::new();
    let mut at = String::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if let Some(rest) = line.strip_prefix("design ") {
            let mut w = rest.splitn(3, ' ');
            at = w.next().unwrap_or("").to_owned();
            let answer = match w.next() {
                Some("refused") => Err(w.next().unwrap_or("").to_owned()),
                _ => Ok(Vec::new()),
            };
            out.insert(at.clone(), answer);
        } else if let Some(line) = line.strip_prefix("  ")
            && let Some(Ok(lines)) = out.get_mut(&at)
        {
            lines.push(line.to_owned());
        }
    }
    Some(out)
}

fn names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .into_string()
                .ok()?
                .strip_suffix(".v")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// See the module's documentation.
#[test]
fn regenerate_when_asked() {
    let (Some(out), true, Some(root)) = (
        std::env::var_os("RETICLE_IDELAY_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
        chipdb(),
    ) else {
        return;
    };
    let out = PathBuf::from(out);
    let mut text = String::from(
        "# Vivado 2026.1 on testdata/fpga/vivado/idelay, xc7a100tftg256-1, by\n\
         # tools/vivado-parity/run_all.sh, and decoded by tests/fpga_xray_idelay.rs.\n",
    );
    for name in names() {
        let bit = out.join(&name).join(format!("{name}.bit"));
        if !bit.is_file() {
            let log = std::fs::read_to_string(out.join(format!("{name}.log"))).unwrap_or_default();
            let rule = log
                .lines()
                .filter_map(|l| l.strip_prefix("ERROR: ["))
                .filter_map(|l| l.split(']').next())
                .map(|l| l.strip_prefix("DRC ").unwrap_or(l))
                .find(|l| !l.starts_with("Common") && !l.starts_with("Vivado"))
                .unwrap_or("?");
            text.push_str(&format!("design {name} refused {rule}\n"));
            continue;
        }
        text.push_str(&format!("design {name} built\n"));
        for line in answer_of(&root, &name, &bit) {
            text.push_str(&format!("  {line}\n"));
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), text).unwrap();
}

/// One design through the flow, from its `.xdc`: its lines, or its log.
fn build(name: &str, root: &str) -> Result<Vec<String>, String> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("idelay")
        .join(name);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join(format!("{name}.bit"));
    let built = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--quiet", "--offline"])
        .arg("--chipdb")
        .arg(root)
        .args(["--top", name, "--constraints"])
        .arg(fixtures().join(format!("{name}.xdc")))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(fixtures().join(format!("{name}.v")))
        .output()
        .expect("failed to run the reticle binary");
    let log = String::from_utf8_lossy(&built.stderr).into_owned();
    if !bit.is_file() || log.contains("NOT FULLY ROUTED") {
        return Err(log);
    }
    Ok(answer_of(root, name, &bit))
}

/// What Vivado writes and this flow does not, known and why: Vivado gives a
/// delay of the fabric a controller of its own where it puts it, in the top
/// left corner of the die, and this flow moves it into the region its pad's
/// delay already has a controller in. `(design, Vivado's line, this flow's)`.
const KNOWN: [(&str, &str, &str); 1] = [("datain_with_pad", "controllers 2", "controllers 1")];

/// What Vivado takes and this flow refuses, by the message it refuses with:
/// an `IDELAY_VALUE` past 31, of which Vivado warns, critically, and which it
/// takes as 0, on purpose; and a delay of the fabric into a serialiser's
/// `DDLY`, which this flow's placer would put in another tile than the
/// serialiser, and which is refused rather than built unrouted.
const STRICTER: [(&str, &str); 2] = [
    ("value_32", "is not 0 to 31"),
    ("ddly_datain", "nothing places the two in one tile"),
];

/// One design against Vivado's answer.
fn compare(name: &str, theirs: &[String], root: &str) -> Result<usize, String> {
    let ours =
        build(name, root).map_err(|log| format!("{name}: Vivado built it, and this says {log}"))?;
    let theirs: Vec<String> = theirs
        .iter()
        .map(|line| {
            KNOWN
                .iter()
                .find(|(d, v, _)| *d == name && v == line)
                .map_or(line.clone(), |(_, _, o)| (*o).to_owned())
        })
        .collect();
    if ours == theirs {
        Ok(theirs.len())
    } else {
        Err(format!(
            "{name}: Vivado only {:?}\nhere only {:?}",
            theirs
                .iter()
                .filter(|l| !ours.contains(l))
                .collect::<Vec<_>>(),
            ours.iter()
                .filter(|l| !theirs.contains(l))
                .collect::<Vec<_>>()
        ))
    }
}

/// A few designs in the gate: the sweep of settings, a delay into an `IDDR`,
/// a `_SING` tile's delay and the bit the database does not name, two regions'
/// controllers, a delay of the fabric, and delays into an `ISERDESE2`'s
/// `DDLY`: a setting a pad, a master and slave, and `IOBDELAY("NONE")`.
#[test]
fn what_reticle_writes_is_what_vivado_wrote() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for name in [
        "idelay_sweep",
        "idelay_iddr",
        "c_inverted",
        "ctrl_two_banks",
        "datain_variable",
        "ddly_sweep",
        "ddly_wide",
        "ddly_none",
    ] {
        let Some(Ok(theirs)) = answers.get(name) else {
            wrong.push(format!("{name}: no answer"));
            continue;
        };
        match compare(name, theirs, &root) {
            Ok(n) => checked += n,
            Err(e) => wrong.push(e),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked > 500, "only {checked} lines");
}

/// Every design Vivado built. Takes some time, so it is ignored.
#[test]
#[ignore = "builds every design to a bitstream, and needs the Project X-Ray database"]
fn every_design_is_what_vivado_wrote() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (name, answer) in &answers {
        let Ok(theirs) = answer else { continue };
        if STRICTER.iter().any(|(d, _)| d == name) {
            continue;
        }
        match compare(name, theirs, &root) {
            Ok(n) => checked += n,
            Err(e) => wrong.push(e),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    eprintln!("{checked} lines agree");
}

/// See the module's documentation; and what this flow refuses that Vivado
/// takes, by its message.
#[test]
fn what_vivado_refuses_is_refused() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let mut wrong = Vec::new();
    for (name, answer) in &answers {
        let Err(rule) = answer else { continue };
        match build(name, &root) {
            Ok(_) => wrong.push(format!(
                "{name}: Vivado refuses it ({rule}), and this built"
            )),
            Err(log) if !log.contains(rule.as_str()) => {
                wrong.push(format!("{name}: Vivado says {rule}, and this says {log}"));
            }
            Err(_) => {}
        }
    }
    for (name, message) in STRICTER {
        match build(name, &root) {
            Ok(_) => wrong.push(format!("{name}: this should refuse it")),
            Err(log) if !log.contains(message) => {
                wrong.push(format!("{name}: expected `{message}`, and this says {log}"));
            }
            Err(_) => {}
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
