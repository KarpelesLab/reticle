//! An IO register's settings, held to what Vivado wrote for each.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/iologic/` has what `tools/vivado-parity/gen_iologic.py`
//! writes: an `IDDR`, `IDDR_2CLK` or `ODDR` beside each pad of bank 35, one
//! setting a pad (`index.txt`), and three designs Vivado refuses. Vivado 2026.1
//! built them (`run_all.sh`), and `vivado.txt` is each bitstream decoded through
//! this repository's database: every feature of each IO logic tile's `ILOGIC`
//! and `OLOGIC` halves, or the rule that refused the design. This regenerates it
//! when `RETICLE_IOLOGIC_BITS` names `run_all.sh`'s output and `UPDATE_EXPECT`
//! is set.
//!
//! # What these would and would not catch
//!
//! [`every_register_is_what_vivado_wrote`] builds each design and compares every
//! `ILOGIC` and `OLOGIC` feature of every IO logic tile, so a setting's bit
//! wrong, missing or extra on any of the 58 registers is caught; interconnect is
//! not compared, and Vivado routes an `ODDR`'s `CLKB` through `IOI_OCLKM` where
//! this flow takes `IOI_OCLK`, the same clock to the same pin. It says nothing of
//! what a register does on a part, which no test here can.
//! [`what_vivado_refuses_is_refused`] would catch `REQP-76`, `REQP-77` or
//! `REQP-129` let through.
//!
//! The serialisers since 2026-11-05: 192 `ISERDESE2` and `OSERDESE2` in 16
//! designs, compared the same way, with the grounds and ties high of every
//! control a serialiser has, the clock sources of `CLKDIV` (an `ILOGIC`'s from
//! the interconnect's `CLK0`) and `OCLK`, and every bit of a register's tile
//! that no feature names (`?`), of which there are none now: a bit a setting
//! sets and this does not name is caught as a line on one side. Twelve
//! designs Vivado refuses (`AVAL-69`, `AVAL-74`, `PDRC-26`, `PDRC-37`,
//! `INBB-3`, `THRU-2`, `Route 35-7`). Not caught: anything of a serialiser's
//! timing, or what it does on a part.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/iologic")
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

/// The IO logic beside each register's ball, as `(tile place, half)`, from
/// `index.txt` and the package: `X57Y115`, `0` for `G2`. Only these are
/// compared, so a clock's own pad is left to the clock buffers'.
fn registers(root: &str) -> Vec<(String, char)> {
    let index = std::fs::read_to_string(fixtures().join("index.txt")).unwrap_or_default();
    let balls: Vec<&str> = index
        .lines()
        .filter(|l| !l.starts_with('#'))
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
        let place = f[3].rsplit_once('_').map_or(f[3], |(_, at)| at).to_owned();
        let p = f[4]
            .split('_')
            .any(|w| w.starts_with('L') && w.ends_with('P'));
        out.push((place, if p { '0' } else { '1' }));
    }
    out
}

/// Every `ILOGIC_Y<n>`, `OLOGIC_Y<n>` and `IDELAY_Y<n>` feature of a `.bit` beside
/// a register, as `<tile place> <feature>` lines, sorted.
fn logic_of(root: &str, bit: &Path) -> Vec<String> {
    let keep = registers(root);
    let db = XrayDatabase::open(&DiskFiles, root, DEVICE, &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(bit).unwrap()).unwrap();
    let frames = read.image(part.layout.clone()).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    let mut out: Vec<String> = decoded
        .features
        .iter()
        .filter(|(tile, _)| tile.contains("IOI3"))
        .filter(|(_, f)| {
            f.starts_with("ILOGIC_Y") || f.starts_with("OLOGIC_Y") || f.starts_with("IDELAY_Y")
        })
        .filter(|(tile, f)| {
            let place = tile.rsplit_once('_').map_or(tile.as_str(), |(_, at)| at);
            let half = f.chars().nth(f.find("_Y").unwrap_or(0) + 2).unwrap_or('?');
            keep.iter().any(|(p, h)| p == place && *h == half)
        })
        .map(|(tile, f)| {
            let place = tile.rsplit_once('_').map_or(tile.as_str(), |(_, at)| at);
            format!("{place} {f}")
        })
        .collect();
    // And a bit of an IO logic tile beside a register that no feature names,
    // which a serialiser's settings set (`xray::iologic::measured_features`
    // names the ones measured, so a line here is one that was not).
    for (tile, bit) in &decoded.leftovers {
        let place = tile.rsplit_once('_').map_or(tile.as_str(), |(_, at)| at);
        if tile.contains("IOI3") && keep.iter().any(|(p, _)| p == place) {
            out.push(format!("{place} ? {:02}_{:02}", bit.row, bit.col));
        }
    }
    // And which of a register's controls is grounded, which no site feature
    // says: a clock enable tied low reads a one unrouted. The IO logic tile's
    // `IOI_IMUX5_<n>` (`ILOGIC` `CE1`), `IOI_IMUX29_<n>` (`OLOGIC` `OCE`),
    // `IOI_CTRL1_<n>` and `IOI_CTRL0_<n>` (`SR`) are the interconnect's `IMUX5`
    // and so on at its row for `_0`, the row above for `_1`, and half 0 takes
    // `_1` (`ppips_rioi3.db`).
    //
    // A ground fan carries a ground only when it is fed from `GND_WIRE`: Vivado
    // routes a signal through one too (`oserdes_tristate`'s `T4`,
    // `IMUX21.GFAN1` with `GFAN1.NR1END1`), which the first version of this
    // called a ground.
    let fans: std::collections::BTreeSet<(&str, &str)> = decoded
        .features
        .iter()
        .filter(|(tile, f)| tile.starts_with("INT_") && f.ends_with(".GND_WIRE"))
        .map(|(tile, f)| (tile.as_str(), f.split('.').next().unwrap_or("")))
        .collect();
    let grounded: std::collections::BTreeSet<(&str, &str)> = decoded
        .features
        .iter()
        .filter(|(tile, f)| tile.starts_with("INT_") && f.contains(".GFAN"))
        .filter(|(tile, f)| {
            f.split_once('.')
                .is_some_and(|(_, fan)| fans.contains(&(tile.as_str(), fan)))
        })
        .map(|(tile, f)| (tile.as_str(), f.split('.').next().unwrap_or("")))
        .collect();
    // A serialiser's other controls, the same way, a pin tied high over
    // `VCC_WIRE`, and `CLKDIV`'s clock, which reaches an `ILOGIC` from the
    // interconnect's own `CLK0` at that row (`IOI_CLK0_<n>`).
    let tied_high: std::collections::BTreeSet<(&str, &str)> = decoded
        .features
        .iter()
        .filter(|(tile, f)| tile.starts_with("INT_") && f.ends_with(".VCC_WIRE"))
        .map(|(tile, f)| (tile.as_str(), f.split('.').next().unwrap_or("")))
        .collect();
    let clock_in: BTreeMap<&str, &str> = decoded
        .features
        .iter()
        .filter(|(tile, f)| tile.starts_with("INT_") && f.starts_with("CLK0."))
        .map(|(tile, f)| (tile.as_str(), &f[5..]))
        .collect();
    for (place, half) in &keep {
        let Some((x, y)) = place
            .strip_prefix('X')
            .and_then(|r| r.split_once('Y'))
            .and_then(|(x, y)| Some((x.to_owned(), y.parse::<u32>().ok()?)))
        else {
            continue;
        };
        let row = if *half == '0' { y + 1 } else { y };
        for (wire, pin) in FIRST_CONTROLS {
            let at = |side: &str| format!("INT_{side}_X{x}Y{row}");
            if grounded.contains(&(at("R").as_str(), wire))
                || grounded.contains(&(at("L").as_str(), wire))
            {
                out.push(format!("{place} ground {half} {pin}"));
            }
        }
        let at = |side: &str| format!("INT_{side}_X{x}Y{row}");
        let either = |set: &std::collections::BTreeSet<(&str, &str)>, wire: &str| {
            set.contains(&(at("R").as_str(), wire)) || set.contains(&(at("L").as_str(), wire))
        };
        for (wire, pin) in SERDES_CONTROLS {
            if either(&grounded, wire) {
                out.push(format!("{place} ground {half} {pin}"));
            }
        }
        for (wire, pin) in SERDES_CONTROLS.iter().chain(&FIRST_CONTROLS) {
            if either(&tied_high, wire) {
                out.push(format!("{place} high {half} {pin}"));
            }
        }
        for side in ["R", "L"] {
            if let Some(from) = clock_in.get(at(side).as_str()) {
                let kind = if from.starts_with("GCLK") {
                    "leaf"
                } else {
                    "fabric"
                };
                out.push(format!("{place} clock {half} ICLKDIV {kind}"));
            }
        }
    }
    // And where each clock pin's clock comes from: a leaf clock, or the
    // interconnect. An `IDDR_2CLK`'s `CB` of `~C` is a leaf clock in Vivado's,
    // and a lookup table's output through the interconnect if it is taken for
    // a signal. `ODDR`'s `CLKB` is left out: Vivado takes `IOI_OCLKM`, this
    // flow `IOI_OCLK`, both from the same leaf clock.
    for (tile, f) in &decoded.features {
        if !tile.contains("IOI3") {
            continue;
        }
        let place = tile.rsplit_once('_').map_or(tile.as_str(), |(_, at)| at);
        let Some((to, from)) = f.split_once('.') else {
            continue;
        };
        let pin = match to {
            "IOI_ILOGIC0_CLK" => ("0", "ICLK"),
            "IOI_ILOGIC0_CLKB" => ("0", "ICLKB"),
            "IOI_ILOGIC1_CLK" => ("1", "ICLK"),
            "IOI_ILOGIC1_CLKB" => ("1", "ICLKB"),
            "IOI_OCLK_0" => ("0", "OCLK"),
            "IOI_OCLK_1" => ("1", "OCLK"),
            "IOI_OLOGIC0_CLKDIV" => ("0", "OCLKDIV"),
            "IOI_OLOGIC1_CLKDIV" => ("1", "OCLKDIV"),
            _ => continue,
        };
        if !keep
            .iter()
            .any(|(p, h)| p == place && h.to_string() == pin.0)
        {
            continue;
        }
        let kind = if from.contains("LEAF_GCLK") {
            "leaf"
        } else {
            "fabric"
        };
        out.push(format!("{place} clock {} {} {kind}", pin.0, pin.1));
    }
    out.sort();
    out
}

/// The controls whose ground the first account compared, as `(interconnect
/// wire, pin)`: an `IDDR`'s `CE` and `SR`, an `ODDR`'s.
const FIRST_CONTROLS: [(&str, &str); 4] = [
    ("IMUX5", "CE1"),
    ("IMUX29", "OCE"),
    ("CTRL1", "ISR"),
    ("CTRL0", "OSR"),
];

/// A serialiser's other controls, from `ppips_rioi3.db`: `IOI_IMUX14_<n>` is
/// the interconnect's `IMUX14` as `CE1`'s `IMUX5` is.
const SERDES_CONTROLS: [(&str, &str); 18] = [
    ("IMUX14", "CE2"),
    ("IMUX0", "BITSLIP"),
    ("IMUX37", "DYNCLKSEL"),
    ("IMUX4", "DYNCLKDIVSEL"),
    ("IMUX10", "DYNCLKDIVPSEL"),
    ("IMUX1", "TCE"),
    ("IMUX34", "D1"),
    ("IMUX40", "D2"),
    ("IMUX44", "D3"),
    ("IMUX42", "D4"),
    ("IMUX43", "D5"),
    ("IMUX45", "D6"),
    ("IMUX46", "D7"),
    ("IMUX47", "D8"),
    ("IMUX15", "T1"),
    ("IMUX7", "T2"),
    ("IMUX13", "T3"),
    ("IMUX21", "T4"),
];

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
        std::env::var_os("RETICLE_IOLOGIC_BITS"),
        std::env::var_os("UPDATE_EXPECT").is_some(),
        chipdb(),
    ) else {
        return;
    };
    let out = PathBuf::from(out);
    let mut text = String::from(
        "# Vivado 2026.1 on testdata/fpga/vivado/iologic, xc7a100tftg256-1, by\n\
         # tools/vivado-parity/run_all.sh, and decoded by tests/fpga_xray_ioreg.rs.\n",
    );
    for name in names() {
        let bit = out.join(&name).join(format!("{name}.bit"));
        if !bit.is_file() {
            let log = std::fs::read_to_string(out.join(format!("{name}.log"))).unwrap_or_default();
            // A design rule's name, or else the first error that is not the
            // step's own "failed due to earlier errors": `Route 35-7`, a
            // `CLKDIVP` that cannot be routed.
            let rule = log
                .lines()
                .filter_map(|l| l.strip_prefix("ERROR: [DRC "))
                .find_map(|l| l.split(']').next())
                .or_else(|| {
                    log.lines()
                        .filter_map(|l| l.strip_prefix("ERROR: ["))
                        .filter_map(|l| l.split(']').next())
                        .find(|id| !id.starts_with("Common ") && !id.starts_with("Vivado"))
                })
                .unwrap_or("?");
            text.push_str(&format!("design {name} refused {rule}\n"));
            continue;
        }
        text.push_str(&format!("design {name} built\n"));
        for line in logic_of(&root, &bit) {
            text.push_str(&format!("  {line}\n"));
        }
    }
    std::fs::write(fixtures().join("vivado.txt"), text).unwrap();
}

/// One design through the flow, from its `.xdc`: its lines, or its log.
fn build(name: &str, root: &str) -> Result<Vec<String>, String> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("iologic")
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
    if !bit.is_file() {
        return Err(String::from_utf8_lossy(&built.stderr).into_owned());
    }
    Ok(logic_of(root, &bit))
}

/// See the module's documentation.
#[test]
fn every_register_is_what_vivado_wrote() {
    let (Some(root), Some(answers)) = (chipdb(), answers()) else {
        return;
    };
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (name, answer) in &answers {
        let Ok(theirs) = answer else { continue };
        match build(name, &root) {
            Err(log) => wrong.push(format!("{name}: Vivado built it, and this says {log}")),
            Ok(ours) if ours == *theirs => checked += theirs.len(),
            Ok(ours) => wrong.push(format!(
                "{name}: Vivado only {:?}\nhere only {:?}",
                theirs
                    .iter()
                    .filter(|l| !ours.contains(l))
                    .collect::<Vec<_>>(),
                ours.iter()
                    .filter(|l| !theirs.contains(l))
                    .collect::<Vec<_>>()
            )),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked > 300, "only {checked} features");
}

/// See the module's documentation.
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
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
