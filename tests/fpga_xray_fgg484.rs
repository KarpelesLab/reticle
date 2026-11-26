//! The XC7A100T in its FGG484 package, against Project X-Ray's own files.
//!
//! `xc7a100t-fgg484`'s pin list in `src/fpga/devices/xc7.dev` was generated
//! from prjxray-db's `artix7/xc7a100tfgg484-1/package_pins.csv`, and the
//! bitstream flow puts each pin on the site that file names. These tests say
//! that the two agree and that the file is consistent with itself and with
//! the FTG256 package of the same die. `docs/fpga-xray.md`, "The FGG484
//! package", says what else was checked and against what.
//!
//! What they would catch: the device file and the database drifting apart
//! (a ball missing, in the wrong bank, or a clock-capable or `VREF` ball
//! not marked so); a row of the database's file whose site is not in the
//! bank its pad name says, or not where that name puts it in the bank; a
//! pad the two packages share landing on two different sites. What they
//! would not catch: a database row wrong in a way that still fits the
//! naming rule, which is a mistake in Project X-Ray's own extraction, and
//! that is what Xilinx's pinout file was read against instead (by hand, at
//! the time: it is not in this repository).
//!
//! Every test that reads the database skips without it, saying why; a
//! copy fetched before the FGG484 files joined the manifest is completed by
//! the next `reticle fpga` or `reticle fetch prjxray-db`, and until then
//! these tests skip with that reason.

#![cfg(feature = "fpga")]

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use reticle::fpga::device::PinKind;
use reticle::fpga::xray::{SpeedGrade, XrayDatabase, XrayOptions};
use reticle::ir::memfile::FileProvider;

const DEVICE: &str = "xc7a100t-fgg484";
const PACKAGE_PINS: &str = "artix7/xc7a100tfgg484-1/package_pins.csv";
const FTG256_PINS: &str = "artix7/xc7a100tftg256-1/package_pins.csv";

struct DiskFiles;

impl FileProvider for DiskFiles {
    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

/// Where `reticle fetch <name>` puts the pinned copy, if it is there.
fn fetched(name: &str, version: &str, probe: &str) -> Option<String> {
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

/// The database root holding the FGG484 files, or `None` saying why not.
fn chipdb() -> Option<String> {
    let probe = "artix7/xc7a100t/tilegrid.json";
    let root = match std::env::var("RETICLE_CHIPDB") {
        Ok(root) => root,
        Err(_) => {
            let Some(root) = fetched(
                "prjxray-db",
                "0a0addedd73e7e4139d52a6d8db4258763e0f1f3",
                probe,
            ) else {
                eprintln!(
                    "skipped: needs a Project X-Ray database with the XC7A100T; run \
                     `reticle fetch prjxray-db` or set RETICLE_CHIPDB to a prjxray-db checkout"
                );
                return None;
            };
            root
        }
    };
    for wanted in [probe, PACKAGE_PINS, FTG256_PINS] {
        let path = format!("{root}/{wanted}");
        if !Path::new(&path).is_file() {
            eprintln!(
                "skipped: `{path}` is not there (a cache filled before the FGG484 package \
                 joined the manifest is completed by `reticle fetch prjxray-db`)"
            );
            return None;
        }
    }
    Some(root)
}

/// One row of `package_pins.csv`.
#[derive(Clone, Debug)]
struct Row {
    ball: String,
    bank: u32,
    site: String,
    tile: String,
    function: String,
}

fn rows(root: &str, file: &str) -> Vec<Row> {
    let text = std::fs::read_to_string(format!("{root}/{file}")).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("pin,bank,site,tile,pin_function"));
    lines
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            assert_eq!(f.len(), 5, "{line}");
            Row {
                ball: f[0].to_owned(),
                bank: f[1].parse().unwrap(),
                site: f[2].to_owned(),
                tile: f[3].to_owned(),
                function: f[4].to_owned(),
            }
        })
        .collect()
}

/// The user-I/O rows: those whose function is an `IO_` pad.
fn user_io(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter()
        .filter(|r| r.function.starts_with("IO_"))
        .collect()
}

/// `X` and `Y` of a `..._X<x>Y<y>` name.
fn xy(name: &str) -> (u32, u32) {
    let (_, at) = name.rsplit_once("_X").unwrap();
    let (x, y) = at.split_once('Y').unwrap();
    (x.parse().unwrap(), y.parse().unwrap())
}

/// Where a pad's name puts it in its bank, counted down from the bank's
/// top row of fifty: `IO_0` the top, `IO_25` the bottom, and pair `n`'s
/// `N` side `2n` rows below the top with its `P` side one row above that.
/// A reading of the names fitted on every `IO_` row of the three other
/// packages this database's cache holds, which it fits with no exception;
/// it is a structure Xilinx's pad numbering has, not a rule anyone wrote
/// down for us.
fn rows_below_top(function: &str) -> u32 {
    let rest = function.strip_prefix("IO_").unwrap();
    let head = rest.split('_').next().unwrap();
    match head {
        "0" => 0,
        "25" => 49,
        pair => {
            let pair = pair.strip_prefix('L').unwrap();
            let (n, side) = pair.split_at(pair.len() - 1);
            let n: u32 = n.parse().unwrap();
            match side {
                "N" => 2 * n,
                "P" => 2 * n - 1,
                _ => panic!("`{function}` is not a pad name this reads"),
            }
        }
    }
}

/// The device file needs no database, and says what grade it is.
#[test]
fn the_fgg484_device_is_a_grade_2_xc7a100t() {
    let device = reticle::fpga::target(DEVICE).expect("the FGG484 part ships");
    assert_eq!(device.family, "xc7");
    // The board's part is an XC7A100T-2FGG484I.
    assert_eq!(device.speed_grade, "2");
    assert_eq!(
        SpeedGrade::from_name(&device.speed_grade),
        Some(SpeedGrade::Two)
    );
    let ftg256 = reticle::fpga::target("xc7a100t-ftg256").unwrap();
    assert_eq!(device.idcode, ftg256.idcode);
    assert_eq!(device.pins.len(), 285);
    let mut per_bank: BTreeMap<&str, usize> = BTreeMap::new();
    for pin in &device.pins {
        *per_bank.entry(pin.bank.as_deref().unwrap()).or_default() += 1;
    }
    assert_eq!(
        per_bank,
        BTreeMap::from([
            ("b13", 35),
            ("b14", 50),
            ("b15", 50),
            ("b16", 50),
            ("b34", 50),
            ("b35", 50),
        ])
    );
}

/// Every ball the device file lists is a user-I/O row of the database's
/// file with the same bank, `clock` exactly when the pad is clock-capable
/// and `vref` exactly when it can be its bank's reference, and the other
/// way round.
#[test]
fn the_fgg484_pin_list_is_the_package_files_user_io() {
    let Some(root) = chipdb() else { return };
    let device = reticle::fpga::target(DEVICE).unwrap();
    let io = user_io(rows(&root, PACKAGE_PINS));
    assert_eq!(io.len(), 285);
    let listed: HashMap<&str, _> = device.pins.iter().map(|p| (p.name.as_str(), p)).collect();
    assert_eq!(listed.len(), device.pins.len(), "a ball listed twice");
    for row in &io {
        let pin = listed
            .get(row.ball.as_str())
            .unwrap_or_else(|| panic!("{} ({}) is not listed", row.ball, row.function));
        assert_eq!(pin.bank.as_deref(), Some(format!("b{}", row.bank).as_str()));
        let clock = row.function.contains("MRCC") || row.function.contains("SRCC");
        let kind = if clock { PinKind::Clock } else { PinKind::Io };
        assert_eq!(pin.kind, kind, "{}", row.function);
        assert_eq!(pin.vref, row.function.contains("VREF"), "{}", row.function);
    }
    assert_eq!(io.len(), device.pins.len());
}

/// Every user-I/O row's site is an `IOB` of its tile in the die's grid, the
/// tile an IO tile of the bank the pad name ends in, every bank one column
/// and one block of fifty rows of its own, and each pad where its name puts
/// it in that block.
#[test]
fn every_fgg484_pad_sits_where_its_name_puts_it() {
    let Some(root) = chipdb() else { return };
    let db = XrayDatabase::open(&DiskFiles, &root, DEVICE, &XrayOptions::new()).unwrap();
    assert_eq!(db.part_directory(), "xc7a100tfgg484-1");
    assert_eq!(db.fabric(), "xc7a100t");
    let part = db.part(&DiskFiles).unwrap();
    assert_eq!(part.idcode, 0x0363_1093);
    let tiles = db.tiles(&DiskFiles).unwrap();
    let by_name: HashMap<&str, _> = tiles.iter().map(|t| (t.name.as_str(), t)).collect();

    let mut bank_at: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for row in user_io(rows(&root, PACKAGE_PINS)) {
        let tile = by_name
            .get(row.tile.as_str())
            .unwrap_or_else(|| panic!("{}: no tile `{}`", row.ball, row.tile));
        assert!(
            tile.tile_type.starts_with("LIOB33") || tile.tile_type.starts_with("RIOB33"),
            "{}: `{}` is a {}",
            row.ball,
            row.tile,
            tile.tile_type
        );
        assert!(
            tile.sites.iter().any(|(name, _)| *name == row.site),
            "{}: `{}` has no site `{}`",
            row.ball,
            row.tile,
            row.site
        );
        assert!(
            row.function.ends_with(&format!("_{}", row.bank)),
            "{}",
            row.function
        );
        let (column, y) = xy(&row.site);
        let (_, tile_y) = xy(&row.tile);
        // A pair's two pads share the tile, which is named for the lower.
        assert!(tile_y == y || tile_y + 1 == y, "{}: {row:?}", row.ball);
        let block = y / 50;
        let seen = *bank_at.entry(row.bank).or_insert((column, block));
        assert_eq!(seen, (column, block), "bank {} spans two blocks", row.bank);
        assert_eq!(
            block * 50 + 49 - y,
            rows_below_top(&row.function),
            "{} {}",
            row.ball,
            row.function
        );
    }
    // Six banks, no two in one place.
    assert_eq!(bank_at.len(), 6);
    let mut places: Vec<_> = bank_at.values().collect();
    places.sort();
    places.dedup();
    assert_eq!(places.len(), 6);
}

/// The naming rule above, on the packages it was read off: had it been
/// wrong for any of them, the test before would be checking nothing.
#[test]
fn the_pad_naming_rule_holds_on_the_other_packages() {
    let Some(root) = chipdb() else { return };
    let mut checked = 0;
    for file in [
        FTG256_PINS,
        "artix7/xc7a35tftg256-1/package_pins.csv",
        "artix7/xc7a35tcpg236-1/package_pins.csv",
    ] {
        if !Path::new(&format!("{root}/{file}")).is_file() {
            eprintln!("skipped `{file}`: not in this copy");
            continue;
        }
        for row in user_io(rows(&root, file)) {
            let (_, y) = xy(&row.site);
            assert_eq!(
                (y / 50) * 50 + 49 - y,
                rows_below_top(&row.function),
                "{file}: {row:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 170, "{checked}");
}

/// A pad is a property of the die: every pad the FTG256 package bonds is
/// bonded by the FGG484 too (on another ball), on the same site and tile.
#[test]
fn the_pads_fgg484_shares_with_ftg256_are_on_the_same_sites() {
    let Some(root) = chipdb() else { return };
    let small = user_io(rows(&root, FTG256_PINS));
    assert_eq!(small.len(), 170);
    let large: HashMap<String, Row> = user_io(rows(&root, PACKAGE_PINS))
        .into_iter()
        .map(|r| (r.function.clone(), r))
        .collect();
    for row in &small {
        let other = large
            .get(&row.function)
            .unwrap_or_else(|| panic!("FGG484 does not bond `{}`", row.function));
        assert_eq!(
            (&other.site, &other.tile, other.bank),
            (&row.site, &row.tile, row.bank),
            "{}",
            row.function
        );
    }
}
