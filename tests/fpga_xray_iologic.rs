//! The registers beside a pad — an `IDDR` and two `ODDR`s, instantiated by
//! name with NewAE's ChipWhisperer-Husky's own parameters — built by the
//! command line for the Husky's part and compared with what Vivado wrote
//! beside the same kind of ball in NewAE's image.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test does.
//!
//! # The oracle
//!
//! NewAE's image is their build output and not in this repository, so
//! Vivado's features are quoted below, decoded through this database from
//! `cwhusky_top.bit` (chipwhisperer-husky-fpga 2dfe1bc, Vivado 2022.1):
//! the `ILOGIC` beside B12, which samples `ADC_DP[0]`; the `OLOGIC` beside
//! J15, which forwards the ADC's clock; and the `OLOGIC` of one of the
//! Husky's glitch outputs (`RIOI3_X43Y75`), whose `ODDR` has an `ASYNC`
//! reset type and a routed reset. The test design's third register has
//! that one's parameters on a different ball, so only the features are
//! compared, not the tile.
//!
//! # What it would and would not catch
//!
//! It would catch an `IDDR` or `ODDR` placed anywhere but beside its pad
//! (its features would be in another tile, and the route from the pad
//! would not exist), any feature missing or extra — an `INIT` the wrong way
//! round, the zero on `D2` left without its inverter, `SRTYPE("ASYNC")`
//! written as `SYNC`, a routed reset without `SRUSED` or with the set's
//! value — and the `IDDR`'s clock reaching `CLK` without `CLKB`. It
//! compares no routing beyond that: which leaf clock reaches the register
//! is the router's choice, and Vivado's choice differs. Nothing here has run
//! on a Husky.

#![cfg(all(feature = "fpga", feature = "cli"))]

use std::collections::BTreeSet;
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

/// What Vivado set on the `ILOGIC` of B12, its `IDDR` sampling the ADC.
const IDDR: [&str; 8] = [
    "IDDR.IN_USE",
    "IDDR_OR_ISERDES.IN_USE",
    "IFF.DDR_CLK_EDGE.OPPOSITE_EDGE",
    "IFF.SRTYPE.SYNC",
    "IFF.ZINIT_Q1",
    "IFF.ZINIT_Q2",
    "IFF.ZINV_C",
    "ZINV_D",
];

/// What it set on the `OLOGIC` of J15, its `ODDR` forwarding the ADC's clock.
const ODDR: [&str; 8] = [
    "IS_D2_INVERTED",
    "ODDR_TDDR.IN_USE",
    "OQUSED",
    "OSERDES.DATA_RATE_OQ.DDR",
    "OSERDES.DATA_RATE_TQ.BUF",
    "OSERDES.SRTYPE.SYNC",
    "ZINIT_OQ",
    "ZINV_CLK",
];

/// And on a glitch output's `OLOGIC`: `SRTYPE("ASYNC")`, `R` routed.
const GLITCH: [&str; 9] = [
    "IS_D2_INVERTED",
    "ODDR.SRUSED",
    "ODDR_TDDR.IN_USE",
    "OQUSED",
    "OSERDES.DATA_RATE_OQ.DDR",
    "OSERDES.DATA_RATE_TQ.BUF",
    "ZINIT_OQ",
    "ZINV_CLK",
    "ZSRVAL_OQ",
];

#[test]
fn the_husky_io_registers_are_configured_as_vivado_configured_them() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("io_registers.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-io-registers");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("io_registers.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(dir.join("io_registers.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("io_registers.v"))
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

    let half = |tile: &str, prefix: &str| -> BTreeSet<String> {
        decoded
            .features
            .iter()
            .filter(|(t, _)| t == tile)
            .filter_map(|(_, f)| f.strip_prefix(prefix).map(str::to_owned))
            .collect()
    };
    let want =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|n| (*n).to_owned()).collect() };
    // B12 is `IOB_X0Y90` in `LIOB33_X0Y89`, the upper site, `IOB_Y0`.
    assert_eq!(half("LIOI3_X0Y89", "ILOGIC_Y0."), want(&IDDR));
    // J15 is `IOB_X0Y54` in `LIOB33_X0Y53`, also `IOB_Y0`.
    assert_eq!(half("LIOI3_X0Y53", "OLOGIC_Y0."), want(&ODDR));
    // C6 is `IOB_X1Y89` in `RIOB33_X43Y89`, the lower site, `IOB_Y1`.
    assert_eq!(half("RIOI3_X43Y89", "OLOGIC_Y1."), want(&GLITCH));
    // The `IDDR`'s clock reaches both of the `ILOGIC`'s clock inputs, as
    // Vivado routes it.
    let clocks: BTreeSet<&str> = decoded
        .features
        .iter()
        .filter(|(t, _)| t == "LIOI3_X0Y89")
        .filter_map(|(_, f)| f.split('.').next())
        .filter(|f| f.starts_with("IOI_ILOGIC0_CLK"))
        .collect();
    assert_eq!(
        clocks,
        BTreeSet::from(["IOI_ILOGIC0_CLK", "IOI_ILOGIC0_CLKB"])
    );
}
