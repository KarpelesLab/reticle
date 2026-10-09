//! Shared support for the ZCU104 tests: finding the databases and
//! Vivado's reference bitstream, and loading [`ZcuDatabase`] from disk.
//! The designs themselves are built with `reticle::fpga::uray::zcu104`.
//!
//! Each input skips cleanly when absent, saying why: `reticle fetch
//! prjuray-db prjuray-db-2020` for the databases (or `RETICLE_URAYDB` and
//! `RETICLE_URAYWIRING`), and `RETICLE_ZCU104_REF` for a directory holding
//! the PYNQ 3.1 base overlay's `base.bit`.

// Each test binary uses a different part of this module.
#![allow(dead_code)]
#![allow(unreachable_pub)]
#![allow(unused_imports)]

use std::path::Path;

use reticle::fpga::uray::TileGrid;
pub use reticle::fpga::uray::zcu104::{Board, Written, ZcuDatabase as Inputs};
use reticle::ir::memfile::FileProvider;

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
pub fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
    let var = |v| std::env::var(v).ok().filter(|s: &String| !s.is_empty());
    let root = var("XDG_CACHE_HOME")
        .or_else(|| var("LOCALAPPDATA").filter(|_| cfg!(windows)))
        .map(|d| format!("{d}/reticle"))
        .or_else(|| var("HOME").map(|h| format!("{h}/.cache/reticle")))?;
    let dir = format!("{root}/{name}/{version}");
    Path::new(&format!("{dir}/{probe}"))
        .is_file()
        .then_some(dir)
}

/// A database root from its variable or the cache, or `None` having
/// said why.
pub fn database(env: &str, name: &str, version: &str, probe: &str) -> Option<String> {
    match std::env::var(env) {
        Ok(root) if Path::new(&format!("{root}/{probe}")).is_file() => Some(root),
        Ok(root) => {
            eprintln!("skipped: {env} is `{root}` but `{probe}` is not in it");
            None
        }
        Err(_) => {
            let found = fetched(name, version, probe);
            if found.is_none() {
                eprintln!("skipped: needs `reticle fetch {name}` or {env}");
            }
            found
        }
    }
}

pub fn uraydb() -> Option<String> {
    database(
        "RETICLE_URAYDB",
        "prjuray-db",
        "9e7d3e7965240fd260d6549a1883a01a152ae490",
        "xazu7ev/tilegrid.json",
    )
}

pub fn wiringdb() -> Option<String> {
    database(
        "RETICLE_URAYWIRING",
        "prjuray-db-2020",
        "affbc5e555ebae16475f32e8fb2d6565d4204f3f",
        "zynqusp/xczu3eg-sfvc784-1-e/tileconn.json",
    )
}

pub fn reference() -> Option<Vec<u8>> {
    let Ok(dir) = std::env::var("RETICLE_ZCU104_REF") else {
        eprintln!(
            "skipped: needs Vivado's ZCU104 base overlay; set RETICLE_ZCU104_REF to a \
             directory holding base.bit. See docs/fpga-uray.md"
        );
        return None;
    };
    std::fs::read(Path::new(&dir).join("base.bit")).ok()
}

/// Reads files from the filesystem, which is what the CLI does and what
/// the library never does.
pub struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// The databases, loaded.
pub fn inputs(bits_root: &str, wiring_root: &str) -> Inputs {
    Inputs::load(&DiskFiles, bits_root, wiring_root).expect("the databases load")
}

/// The grid position of the tile called `name`.
pub fn position(grid: &TileGrid, name: &str) -> (u32, u32) {
    grid.tiles()
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("the grid has no `{name}`"))
        .grid
}

/// Writes a finished design as `<name>.bit` and `<name>.bin` into `dir`,
/// printing its report.
pub fn write(written: &Written, dir: &Path, name: &str) {
    for line in &written.report {
        eprintln!("{line}");
    }
    std::fs::write(dir.join(format!("{name}.bit")), &written.bit).unwrap();
    std::fs::write(dir.join(format!("{name}.bin")), &written.bin).unwrap();
}
