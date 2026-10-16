//! The 7-series global clock buffers a design instantiates by name — `BUFG`,
//! `BUFGCE`, `BUFGMUX` and the `IBUFG` in front of them — built by the
//! command line for NewAE's ChipWhisperer-Husky part and decoded back.
//!
//! Skips without the Project X-Ray database, as every `fpga_xray` test
//! does: `reticle fetch prjxray-db` puts the pinned copy where this looks.
//!
//! # The oracle
//!
//! NewAE ships the Husky's bitstream, built by Vivado 2022.1 from source
//! that instantiates three `BUFG`s, two `BUFGCE`s and ten `BUFGMUX`es,
//! eight of them `CLK_SEL_TYPE("ASYNC")`. Decoded through this database,
//! its thirteen `BUFGCTRL`s hold three configurations and only three
//! (`src/fpga/xray/bufg.rs` lists them with their counts). This test
//! asks that the same primitives, built here, come out as exactly those
//! configurations — not as something that merely decodes.
//!
//! # What it would and would not catch
//!
//! It would catch a control inverter written the wrong way round, a
//! `BUFGMUX`'s select not reaching both enables, `CLK_SEL_TYPE` lost on the
//! way from the source (it was: a black box's parameters were dropped by
//! the elaborator until this test), a second `BUFG` put behind a clock the
//! design already buffers (it was: two of them), and a select or enable
//! with no path to the buffer (it had none: the region the command line
//! loads did not reach the interconnect beside the buffer column).
//!
//! It would not catch a wrong reading shared by this flow and the test —
//! which is why the configurations are Vivado's, from its image, and not
//! derived — nor anything about the part. Nothing here has run on a Husky.

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

/// The database root, or `None` with a line saying what is missing. The
/// same search as `tests/fpga_xray.rs`.
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

/// Each in-use `BUFGCTRL`'s features, sorted and joined, keyed by
/// `tile:site`.
fn buffer_configurations(features: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tile, feature) in features {
        let Some(rest) = feature.strip_prefix("BUFGCTRL.") else {
            continue;
        };
        let Some((site, name)) = rest.split_once('.') else {
            continue;
        };
        out.entry(format!("{tile}:{site}"))
            .or_default()
            .push(name.to_owned());
    }
    for names in out.values_mut() {
        names.sort();
    }
    out
}

#[test]
fn the_husky_clock_buffers_are_the_ones_vivado_wrote() {
    let Some(root) = chipdb() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fpga/husky");
    if !dir.join("clock_buffers.v").is_file() {
        eprintln!("skipped: `testdata/` is not in the published crate");
        return;
    }
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("husky-clock-buffers");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let bit = out.join("clock_buffers.bit");
    let run = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args([
            "fpga",
            "--device",
            "xc7a35t-ftg256",
            "--chipdb",
            &root,
            "--constraints",
        ])
        .arg(dir.join("clock_buffers.rcf"))
        .arg("--output-dir")
        .arg(&out)
        .arg("--bitstream")
        .arg(&bit)
        .arg(dir.join("clock_buffers.v"))
        .output()
        .expect("the reticle binary runs");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{stderr}");
    assert!(
        !stderr.contains("NOT FULLY ROUTED"),
        "every signal must route, the buffers' controls included:\n{stderr}"
    );

    let db = XrayDatabase::open(&DiskFiles, &root, "xc7a35t-ftg256", &XrayOptions::new()).unwrap();
    let part = db.part(&DiskFiles).unwrap();
    let read = xc7::read_bit(&std::fs::read(&bit).unwrap()).unwrap();
    let frames = xc7::FrameData::from_stream(part.layout.clone(), read.frames).unwrap();
    let decoded = db.decode(&DiskFiles, &frames).unwrap();
    assert_eq!(decoded.unexplained, 0, "{}", decoded.to_text());

    let configurations = buffer_configurations(&decoded.features);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for names in configurations.values() {
        *counts.entry(names.join(" ")).or_default() += 1;
    }
    // Vivado's three, from NewAE's image, and nothing else.
    let bufg = "IN_USE IS_IGNORE1_INVERTED ZINV_CE0 ZINV_S0";
    let mux_async = "IN_USE IS_IGNORE0_INVERTED IS_IGNORE1_INVERTED ZINV_CE1 ZINV_S0 ZINV_S1";
    let mux_sync = "IN_USE ZINV_CE1 ZINV_S0 ZINV_S1";
    for configuration in counts.keys() {
        assert!(
            [bufg, mux_async, mux_sync].contains(&configuration.as_str()),
            "`{configuration}` is not one of Vivado's:\n{configurations:#?}"
        );
    }
    // One `ASYNC` `BUFGMUX` and one `SYNC`, exactly.
    assert_eq!(counts.get(mux_async), Some(&1), "{configurations:#?}");
    assert_eq!(counts.get(mux_sync), Some(&1), "{configurations:#?}");
    // The `BUFG` and the `BUFGCE` at least. A buffer's output reaches
    // another buffer's input directly only from the sites beside it
    // (`CLK_BUFG_R_FBG_OUT<n±1>`), and a clock feeding three of them cannot
    // be beside all three; the router then passes it through a free
    // `BUFGCTRL`, which it configures as a `BUFG` — a cascade, legal and
    // the same function, costing a site. So more than two is allowed here,
    // and the configuration of every one is still Vivado's.
    assert!(
        counts.get(bufg).copied().unwrap_or(0) >= 2,
        "{configurations:#?}"
    );
}
