//! The four clock managers NewAE's ChipWhisperer-Husky instantiates — three
//! `MMCME2_ADV`s and a `PLLE2_ADV`, each with the Husky's own parameters —
//! built by the command line for the Husky's part, and every register field
//! of every one compared with what Vivado wrote for the same instance.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.
//! It loads the whole die, because three MMCMs need three of the part's
//! five clock management tiles, and a pad's neighbourhood holds one.
//!
//! # The oracle
//!
//! NewAE's image is their build output and not in this repository, so the
//! fields Vivado set are quoted below, decoded through this database from
//! `cwhusky_top.bit` (chipwhisperer-husky-fpga 2dfe1bc, Vivado 2022.1),
//! field by field with each one's value assembled from its bits. A field
//! not listed is zero in Vivado's image; the comparison is of the whole
//! register file, zeros included.
//!
//! **This is the first comparison of this flow's PLL registers with a
//! Vivado bitstream holding a PLL**, which `docs/fpga-xray.md` had said
//! nothing had done: the lock and filter tables at a multiplier of 10, and
//! every divider field, agree with Vivado's.
//!
//! # What it would and would not catch
//!
//! It would catch any field computed or quoted wrong: an MMCM filter table
//! taken from the PLL's (they differ at a multiplier of 6), the MMCM's
//! `POWER_REG` written in f4pga's bit order, the phase mux `MX` missed on a
//! fine-phase-shift output, a phase-shift pin's inverter set when the pin is
//! routed or left clear when it is not, an output enable on an output that
//! drives nothing, and an odd division's `EDGE`. It would not catch a field
//! both this flow and Vivado leave at zero and the silicon needs set, nor
//! anything about locking: nothing here has run on a Husky.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeMap;
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

/// Every register field Vivado set nonzero in the clock-glitch MMCM (`U_mmcm1_offset`; `U_mmcm2_width` at `CMT_TOP_R_LOWER_B_X8Y113` is identical), `CMT_TOP_R_LOWER_B_X8Y61`.
const GLITCH: &[(&str, u64)] = &[
    ("CLKFBOUT_CLKOUT1_HIGH_TIME", 0x3),
    ("CLKFBOUT_CLKOUT1_LOW_TIME", 0x3),
    ("CLKFBOUT_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT1_HIGH_TIME", 0x3),
    ("CLKOUT0_CLKOUT1_LOW_TIME", 0x3),
    ("CLKOUT0_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT2_MX", 0x1),
    ("CLKOUT1_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT1_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT1_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT2_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT2_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT2_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT3_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT3_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT3_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT4_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT4_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT4_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT5_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT5_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT5_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("CLKOUT6_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT6_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT6_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("COMP.Z_ZHOLD", 0x1),
    ("DIVCLK_DIVCLK_HIGH_TIME", 0x1),
    ("DIVCLK_DIVCLK_LOW_TIME", 0x1),
    ("DIVCLK_DIVCLK_NO_COUNT", 0x1),
    ("FILTREG1_RESERVED", 0x8),
    ("IN_USE", 0x1),
    ("LKTABLE", 0x8c7e8fa401),
    ("LOCKREG3_RESERVED", 0x1),
    ("POWER_REG_POWER_REG_POWER_REG", 0x199),
    ("TABLE", 0x3ac),
];

/// Every register field Vivado set nonzero in the logic analyser's MMCM, `CMT_TOP_L_LOWER_B_X106Y61`.
const LOGIC_ANALYSER: &[(&str, u64)] = &[
    ("CLKFBOUT_CLKOUT1_HIGH_TIME", 0x3),
    ("CLKFBOUT_CLKOUT1_LOW_TIME", 0x3),
    ("CLKFBOUT_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT1_HIGH_TIME", 0x2),
    ("CLKOUT0_CLKOUT1_LOW_TIME", 0x3),
    ("CLKOUT0_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT2_EDGE", 0x1),
    ("CLKOUT1_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT1_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT1_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT2_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT2_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT2_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT3_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT3_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT3_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT4_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT4_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT4_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT5_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT5_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT5_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("CLKOUT6_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT6_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT6_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("COMP.Z_ZHOLD", 0x1),
    ("DIVCLK_DIVCLK_HIGH_TIME", 0x1),
    ("DIVCLK_DIVCLK_LOW_TIME", 0x1),
    ("DIVCLK_DIVCLK_NO_COUNT", 0x1),
    ("FILTREG1_RESERVED", 0x8),
    ("IN_USE", 0x1),
    ("LKTABLE", 0x8c7e8fa401),
    ("LOCKREG3_RESERVED", 0x1),
    ("POWER_REG_POWER_REG_POWER_REG", 0x100),
    ("TABLE", 0x3ac),
    ("ZINV_PSEN", 0x1),
    ("ZINV_PSINCDEC", 0x1),
];

/// Every register field Vivado set nonzero in the trace block's MMCM, `CMT_TOP_R_LOWER_B_X8Y9`.
const TRACE: &[(&str, u64)] = &[
    ("CLKFBOUT_CLKOUT1_HIGH_TIME", 0x10),
    ("CLKFBOUT_CLKOUT1_LOW_TIME", 0x10),
    ("CLKFBOUT_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT1_HIGH_TIME", 0x10),
    ("CLKOUT0_CLKOUT1_LOW_TIME", 0x10),
    ("CLKOUT0_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT2_MX", 0x1),
    ("CLKOUT1_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT1_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT1_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT2_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT2_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT2_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT3_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT3_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT3_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT4_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT4_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT4_CLKOUT2_NO_COUNT", 0x1),
    ("CLKOUT5_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT5_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT5_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("CLKOUT6_CLKOUT1_HIGH_TIME", 0x1),
    ("CLKOUT6_CLKOUT1_LOW_TIME", 0x1),
    ("CLKOUT6_CLKOUT2_FRACTIONAL_NO_COUNT", 0x1),
    ("COMP.Z_ZHOLD", 0x1),
    ("DIVCLK_DIVCLK_HIGH_TIME", 0x1),
    ("DIVCLK_DIVCLK_LOW_TIME", 0x1),
    ("DIVCLK_DIVCLK_NO_COUNT", 0x1),
    ("FILTREG1_RESERVED", 0x8),
    ("IN_USE", 0x1),
    ("LKTABLE", 0xffd2cfa401),
    ("LOCKREG3_RESERVED", 0x1),
    ("POWER_REG_POWER_REG_POWER_REG", 0x199),
    ("TABLE", 0xd0),
];

/// Every register field Vivado set nonzero in the user IO `PLLE2_ADV`, `CMT_TOP_L_UPPER_T_X106Y96`.
const USERIO_PLL: &[(&str, u64)] = &[
    ("CLKFBOUT_CLKOUT1_HIGH_TIME", 0x5),
    ("CLKFBOUT_CLKOUT1_LOW_TIME", 0x5),
    ("CLKFBOUT_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT0_CLKOUT1_HIGH_TIME", 0x5),
    ("CLKOUT0_CLKOUT1_LOW_TIME", 0x5),
    ("CLKOUT0_CLKOUT1_OUTPUT_ENABLE", 0x1),
    ("CLKOUT1_CLKOUT1_HIGH_TIME", 0xa),
    ("CLKOUT1_CLKOUT1_LOW_TIME", 0xa),
    ("CLKOUT2_CLKOUT1_HIGH_TIME", 0x14),
    ("CLKOUT2_CLKOUT1_LOW_TIME", 0x14),
    ("CLKOUT3_CLKOUT1_HIGH_TIME", 0x1e),
    ("CLKOUT3_CLKOUT1_LOW_TIME", 0x1e),
    ("CLKOUT4_CLKOUT1_HIGH_TIME", 0x3c),
    ("CLKOUT4_CLKOUT1_LOW_TIME", 0x3c),
    ("CLKOUT5_CLKOUT1_HIGH_TIME", 0x3c),
    ("CLKOUT5_CLKOUT1_LOW_TIME", 0x3c),
    ("COMPENSATION.Z_ZHOLD_OR_CLKIN_BUF", 0x1),
    ("DIVCLK_DIVCLK_HIGH_TIME", 0x1),
    ("DIVCLK_DIVCLK_LOW_TIME", 0x1),
    ("DIVCLK_DIVCLK_NO_COUNT", 0x1),
    ("FILTREG1_RESERVED", 0x8),
    ("IN_USE", 0x1),
    ("LKTABLE", 0xe73e8fa401),
    ("LOCKREG3_RESERVED", 0x1),
    ("TABLE", 0x3dc),
];

/// The register fields of every clock manager in a decoded image, by tile,
/// each assembled from its bits, nonzero ones only.
fn registers(features: &[(String, String)]) -> BTreeMap<String, (String, BTreeMap<String, u64>)> {
    let mut out: BTreeMap<String, (String, BTreeMap<String, u64>)> = BTreeMap::new();
    for (tile, feature) in features {
        for kind in ["MMCME2_ADV", "PLLE2_ADV"] {
            let Some(field) = feature.strip_prefix(kind).and_then(|f| f.strip_prefix('.')) else {
                continue;
            };
            let (name, bit) = match field.split_once('[') {
                Some((name, rest)) => {
                    (name, rest.trim_end_matches(']').parse::<u32>().unwrap_or(0))
                }
                None => (field, 0),
            };
            let entry = out
                .entry(tile.clone())
                .or_insert_with(|| (kind.to_owned(), BTreeMap::new()));
            *entry.1.entry(name.to_owned()).or_default() |= 1 << bit;
        }
    }
    out
}

#[test]
fn the_husky_clock_managers_are_the_ones_vivado_wrote() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("clock_managers.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-clock-managers");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("clock_managers.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--region",
            "0,0,114,156",
            "--constraints",
        ])
        .arg(dir.join("clock_managers.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("clock_managers.v"))
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

    let ours = registers(&decoded.features);
    assert_eq!(ours.len(), 4, "four clock managers: {:#?}", ours.keys());
    let mut matched: Vec<&str> = Vec::new();
    for (name, kind, want) in [
        ("the glitch MMCM", "MMCME2_ADV", GLITCH),
        ("the logic analyser's MMCM", "MMCME2_ADV", LOGIC_ANALYSER),
        ("the trace MMCM", "MMCME2_ADV", TRACE),
        ("the user IO PLL", "PLLE2_ADV", USERIO_PLL),
    ] {
        let want: BTreeMap<String, u64> = want.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
        let found = ours.iter().find(|(tile, (k, fields))| {
            k == kind && *fields == want && !matched.contains(&tile.as_str())
        });
        let Some((tile, _)) = found else {
            panic!(
                "no {kind} in the image has {name}'s registers as Vivado wrote them:\n\
                 want {want:#x?}\nhave {ours:#x?}"
            );
        };
        matched.push(tile);
    }
}
