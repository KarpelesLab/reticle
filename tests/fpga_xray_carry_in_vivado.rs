//! A carry chain whose carry in is a signal, held to what Vivado wrote for it.
//!
//! # The oracle
//!
//! `testdata/fpga/vivado/carry_in/` has `s ? c - d : a + b` at eight bits
//! (`shared8`), whose carry in is a signal: `primitives` writes the two adders
//! as one chain with its operands chosen, and the subtraction's `+ 1` as a low
//! bit both operands share, which is `s`. Vivado 2026.1 built it and
//! `vivado.txt` is the bitstream decoded through this repository's database:
//! of each slice holding a `CARRY4`, the carry's own features. Vivado puts `s`
//! on the first `CARRY4`'s `CYINIT`, which a slice reaches from `AX` alone
//! (`PRECYINIT.AX`), and bit 0's generate on the lane's `O5` (`CARRY4.ACY0`),
//! a `LUT3` on `A5LUT` beside the propagate's `LUT5` on `A6LUT`: two `CARRY4`s
//! for eight bits.
//!
//! This flow refused that until 2026-11-07 (`xray::legalise_carries`: both
//! wanted `AX`), so the carry in was a lane of its own and the chain three
//! `CARRY4`s. It now does what Vivado does (`SharedAx::Pin`).
//!
//! # What this would and would not catch
//!
//! [`the_carry_in_is_on_cyinit_and_bit_0s_generate_on_o5_as_vivado_has_them`]
//! builds `shared8` and compares every carry slice's features with Vivado's: a
//! carry in back on a lane of its own (three slices), on `CYINIT` with the
//! generate on `AX` as well (no `ACY0`, or a route the router could not finish),
//! or `PRECYINIT` set from anything but `AX`, fails. It would not catch the
//! function the `LUT6_2` computes, which `tests/fpga_carry.rs` simulates on
//! every input, nor anything on a part. Nor, on this design,
//! `legalise_carries` leaving bit 0's generate for the bypass input: its
//! generate is a table, and `pack_lut_pairs` pairs a lane's two tables into
//! the same `LUT6_2` anyway (a mutation that did that passed here, and failed
//! `tests/fpga_carry.rs`'s `carry_ins_have_ax_to_themselves`, which asks it
//! of the legalised chain before any pairing).
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEVICE: &str = "xc7a100t-ftg256";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/vivado/carry_in")
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

/// Vivado's answer for each design: its `slice` lines.
fn answers() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(fixtures().join("vivado.txt")).unwrap();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut design = String::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("design ") {
            design = name.to_owned();
            out.entry(design.clone()).or_default();
        } else if let Some(slice) = line.trim().strip_prefix("slice ") {
            out.entry(design.clone())
                .or_default()
                .push(slice.to_owned());
        }
    }
    out
}

/// Each carry slice's `PRECYINIT.*` and `CARRY4.*` features, one sorted line a
/// slice, from `reticle fpga --decode`'s `TILE SITE.FEATURE` lines.
fn carry_slices(decoded: &str) -> Vec<String> {
    let mut slices: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for line in decoded.lines() {
        let Some((tile, feature)) = line.split_once(' ') else {
            continue;
        };
        let Some((site, rest)) = feature.split_once('.') else {
            continue;
        };
        if rest.starts_with("PRECYINIT.") || rest.starts_with("CARRY4.") {
            slices
                .entry((tile.to_owned(), site.to_owned()))
                .or_default()
                .push(rest.to_owned());
        }
    }
    let mut out: Vec<String> = slices
        .into_values()
        .map(|mut features| {
            features.sort();
            features.join(" ")
        })
        .collect();
    out.sort();
    out
}

/// Builds `name` to a bitstream and answers its carry slices, or the log.
fn build(root: &str, name: &str) -> Result<Vec<String>, String> {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("carry_in")
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
    let decoded = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(["fpga", "--device", DEVICE, "--offline"])
        .arg("--chipdb")
        .arg(root)
        .arg("--decode")
        .arg(&bit)
        .output()
        .expect("failed to run the reticle binary");
    let text = String::from_utf8_lossy(&decoded.stdout).into_owned();
    let summary = String::from_utf8_lossy(&decoded.stderr).into_owned() + &text;
    if !summary.contains(" 0 bit(s) unexplained") {
        return Err(format!("not every bit decodes: {summary}"));
    }
    Ok(carry_slices(&text))
}

/// What this flow writes and Vivado does not, known and why: the chain's top
/// lane, whose generate nothing can see (its sum is `S ^ CO` of the lane below
/// and its carry out goes nowhere), takes it from `O5` here, the lane's two
/// tables paired as every lane's are, and from the bypass input in Vivado's.
/// `(design, Vivado's line, this flow's)`.
const KNOWN: [(&str, &str, &str); 1] = [(
    "shared8",
    "CARRY4.ACY0 CARRY4.BCY0 CARRY4.CCY0 PRECYINIT.CIN",
    "CARRY4.ACY0 CARRY4.BCY0 CARRY4.CCY0 CARRY4.DCY0 PRECYINIT.CIN",
)];

/// **The carry in is on `CYINIT` and bit 0's generate on `O5`, as Vivado has
/// them.** See the module docs for what it would catch.
#[test]
fn the_carry_in_is_on_cyinit_and_bit_0s_generate_on_o5_as_vivado_has_them() {
    let Some(root) = chipdb() else { return };
    let mut wrong = Vec::new();
    for (name, theirs) in answers() {
        let ours = match build(&root, &name) {
            Ok(ours) => ours,
            Err(log) => {
                wrong.push(format!("{name}: Vivado built it, and this says {log}"));
                continue;
            }
        };
        let mut theirs: Vec<String> = theirs
            .iter()
            .map(|line| {
                KNOWN
                    .iter()
                    .find(|(d, v, _)| *d == name && v == line)
                    .map_or(line.clone(), |(_, _, o)| (*o).to_owned())
            })
            .collect();
        theirs.sort();
        if ours != theirs {
            wrong.push(format!("{name}:\n  ours   {ours:?}\n  Vivado {theirs:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
