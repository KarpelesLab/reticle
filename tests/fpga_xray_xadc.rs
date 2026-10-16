//! The XADC, configured as NewAE's ChipWhisperer-Husky configures it, built
//! by the command line for the Husky's part and compared with NewAE's image.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.
//!
//! # What makes this a check and not a tautology
//!
//! prjxray-db describes none of the XADC tile's bits; `src/fpga/xray/xadc.rs`
//! derived the register layout from NewAE's image and the wizard's recorded
//! values, and says how. This test builds an `XADC` with those values through
//! the whole flow — mapping, placement on the one site, routing of its DRP and
//! alarms over the fixed connections transcribed from prjxray's tile JSON,
//! the register bits from the cell's `INIT_*` parameters — and asks that
//! the tile's register file read back as the one Vivado wrote. The values
//! below are Vivado's, decoded from its image through the same layout.
//!
//! It would catch the register bits not reaching the bitstream (they did
//! not, at first: a two-component feature name was not taken for a site),
//! the XADC's pins unroutable, and `IN_USE` missing. It would not catch a
//! wrong layout for a bit Vivado never sets, which only a second image with
//! other values can settle; and nothing here has run on a Husky.

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

/// The XADC's initial registers in NewAE's image, nonzero ones only:
/// `xadc_wiz_0`'s values, with `INIT_5C` as Vivado wrote it.
const VIVADO: [(&str, u64); 14] = [
    ("INIT_41", 0x20a0),
    ("INIT_42", 0x0400),
    ("INIT_48", 0x4700),
    ("INIT_4A", 0x4700),
    ("INIT_50", 0xb363),
    ("INIT_51", 0x5999),
    ("INIT_52", 0xa147),
    ("INIT_53", 0xb5e3),
    ("INIT_54", 0xa93a),
    ("INIT_55", 0x5111),
    ("INIT_56", 0x91eb),
    ("INIT_57", 0xa93a),
    ("INIT_58", 0x5999),
    ("INIT_5C", 0x5111),
];

#[test]
fn the_husky_xadc_is_configured_as_vivado_configured_it() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("xadc.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-xadc");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("xadc.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(dir.join("xadc.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("xadc.v"))
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
    // The tile used to have no `segbits` file; it is described now.
    assert_eq!(
        decoded.tiles_without_a_segbits_file,
        0,
        "{}",
        decoded.to_text()
    );

    let mut registers: BTreeMap<String, u64> = BTreeMap::new();
    let mut in_use = false;
    for (tile, feature) in &decoded.features {
        if tile != "MONITOR_BOT_X46Y79" {
            continue;
        }
        if feature == "XADC.IN_USE" {
            in_use = true;
            continue;
        }
        let Some((name, rest)) = feature
            .strip_prefix("XADC.")
            .and_then(|f| f.split_once('['))
        else {
            panic!("an XADC feature this test does not know: {feature}");
        };
        let bit: u32 = rest.trim_end_matches(']').parse().unwrap();
        *registers.entry(name.to_owned()).or_default() |= 1 << bit;
    }
    assert!(in_use, "the XADC is not marked in use");
    let want: BTreeMap<String, u64> = VIVADO.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
    assert_eq!(registers, want);
}
