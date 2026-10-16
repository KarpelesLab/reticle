//! A pad per IO standard, drive and pull NewAE's ChipWhisperer-Husky uses,
//! built by the command line for the Husky's own part and compared, half by
//! half, with what Vivado wrote on the same ball of NewAE's image.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.
//!
//! # The oracle, and why it is written out here
//!
//! NewAE's `cwhusky_top.bit` is their build output and not something this
//! repository can carry, so the features Vivado set on each of these ten
//! balls are quoted below, as decoded through this database from that
//! image (chipwhisperer-husky-fpga 2dfe1bc, Vivado 2022.1). The pins,
//! standards, drive and pulls of the test design are the Husky's own
//! (`testdata/fpga/husky/io_standards.rcf` says where each came from), so
//! the two images configure the same ball for the same request.
//!
//! # What it would and would not catch
//!
//! It would catch any pad given a feature Vivado did not set or missing
//! one it did: a standard's input or drive feature confused with
//! another's, a drive of 8 mA written as 12, a pull added on top of `NONE`
//! instead of replacing it, a pull-down written as anything but the empty
//! field, a differential pair configured on one half only or without its
//! tile's `DIFF.ZIBUF_LOW_PWR`, an `OBUFDS`'s second ball driven as an
//! output of its own, and an `IBUFDS`'s `IBUF_LOW_PWR` lost on the way
//! from the source. It would not catch a pad that is right in its bits and
//! wrong in its routing — the decode's `0 bit(s) unexplained` and the
//! routing checks the command line runs are what cover that — nor
//! anything about the board: nothing here has run on a Husky.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use reticle::fpga::xc7;
use reticle::fpga::xray::{XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The database root, or `None` with a line saying what is missing.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a35tftg256-1/package_pins.csv";
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

const IN_ONLY: &str =
    "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVDS_25_LVTTL_SSTL135_SSTL15_TMDS_33.IN_ONLY";
const SLOW: &str = "LVCMOS12_LVCMOS15_LVCMOS18_LVCMOS25_LVCMOS33_LVTTL_SSTL135_SSTL15.SLEW.SLOW";
const RECEIVER: &str = "LVCMOS25_LVCMOS33_LVTTL.IN";

/// Each ball, its tile, its half, and the features Vivado set on that half
/// in NewAE's image — quoted, as the module documentation says.
const VIVADO: [(&str, &str, &str, &[&str]); 10] = [
    // PLLFPGAP / PLLFPGAN: an `IBUFDS`, `IBUF_LOW_PWR("FALSE")`.
    (
        "D13",
        "LIOB33_X0Y75",
        "IOB_Y0",
        &[IN_ONLY, "LVDS_25_SSTL135_SSTL15.IN_DIFF", "PULLTYPE.NONE"],
    ),
    (
        "C13",
        "LIOB33_X0Y75",
        "IOB_Y1",
        &[IN_ONLY, "LVDS_25_SSTL135_SSTL15.IN_DIFF", "PULLTYPE.NONE"],
    ),
    // ADC_CLKP / ADC_CLKN: an `OBUFDS`.
    (
        "J15",
        "LIOB33_X0Y53",
        "IOB_Y0",
        &["LVDS_25.DRIVE.I_FIXED", "LVDS_25.OUT", "PULLTYPE.NONE"],
    ),
    ("J16", "LIOB33_X0Y53", "IOB_Y1", &[IN_ONLY, "PULLTYPE.NONE"]),
    // ADC_DFS, an LVCMOS25 output at its default 12 mA.
    (
        "G12",
        "LIOB33_X0Y61",
        "IOB_Y1",
        &[SLOW, "LVCMOS25.DRIVE.I12", "PULLTYPE.NONE"],
    ),
    // PLL_STATUS, an LVCMOS25 input.
    (
        "C11",
        "LIOB33_X0Y77",
        "IOB_Y0",
        &[IN_ONLY, RECEIVER, "PULLTYPE.NONE"],
    ),
    // LED_ADC, LVCMOS33 at `DRIVE 8`.
    (
        "C7",
        "RIOB33_X43Y89",
        "IOB_Y0",
        &[SLOW, "LVCMOS33_LVTTL.DRIVE.I12_I8", "PULLTYPE.NONE"],
    ),
    // SAM_MOSI, an LVCMOS33 input.
    (
        "T7",
        "LIOB33_X0Y7",
        "IOB_Y0",
        &[IN_ONLY, RECEIVER, "PULLTYPE.NONE"],
    ),
    // target_io4, bidirectional, `PULLTYPE PULLDOWN`: no `PULLTYPE` feature.
    (
        "T4",
        "RIOB33_X43Y31",
        "IOB_Y0",
        &[SLOW, RECEIVER, "LVCMOS33_LVTTL.DRIVE.I12_I16"],
    ),
    // USERIO_CLK, bidirectional, pulled up.
    (
        "N12",
        "LIOB33_X0Y23",
        "IOB_Y1",
        &[
            SLOW,
            RECEIVER,
            "LVCMOS33_LVTTL.DRIVE.I12_I16",
            "PULLTYPE.PULLUP",
        ],
    ),
];

#[test]
fn every_husky_pad_is_configured_as_vivado_configured_it() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("io_standards.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-io-standards");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("io_standards.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(dir.join("io_standards.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("io_standards.v"))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(!stderr.contains("NOT FULLY ROUTED"), "{stderr}");

    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a35t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(&bit).unwrap()).unwrap();
    let frames = xc7::FrameData::from_stream(part.layout.clone(), read.frames).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    assert_eq!(decoded.unexplained, 0, "{}", decoded.to_text());

    let mut by_tile: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (tile, feature) in &decoded.features {
        by_tile.entry(tile).or_default().insert(feature);
    }
    let mut wrong = Vec::new();
    for (pin, tile, half, want) in VIVADO {
        let ours: BTreeSet<&str> = by_tile
            .get(tile)
            .into_iter()
            .flatten()
            .filter_map(|f| f.strip_prefix(half).and_then(|f| f.strip_prefix('.')))
            .collect();
        let want: BTreeSet<&str> = want.iter().copied().collect();
        if ours != want {
            wrong.push(format!(
                "{pin} ({tile} {half}): ours {ours:?}, Vivado's {want:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));

    // The input pair's tile has the one feature of the pair, because the
    // `IBUFDS` said `IBUF_LOW_PWR("FALSE")`; the output pair's has none.
    let pair = |tile: &str| -> BTreeSet<&str> {
        by_tile
            .get(tile)
            .into_iter()
            .flatten()
            .copied()
            .filter(|f| !f.starts_with("IOB_Y"))
            .collect()
    };
    assert_eq!(pair("LIOB33_X0Y75"), BTreeSet::from(["DIFF.ZIBUF_LOW_PWR"]));
    assert!(pair("LIOB33_X0Y53").is_empty());
}
